//! Event loop and webview orchestration.
//!
//! Layout of this module, top to bottom:
//!   1. `AppState` + snapshots the shell consumes
//!   2. tab lifecycle (create/activate/close/navigate)
//!   3. panels & overlay sizing
//!   4. command dispatch (the IPC surface, §6.2)
//!   5. webview construction
//!   6. `run()`: startup order, event loop, event handlers
//!
//! Threading: everything is UI-thread-only except the debounce timer thread,
//! which only sends an `AppMessage`. WebView2 callbacks (title, download,
//! navigation) also arrive on the UI thread; they forward through the
//! `EventLoopProxy` so no handler re-enters `AppState` while it is borrowed.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::http::Request as HttpRequest;
use wry::{WebContext, WebView, WebViewBuilder};
#[cfg(windows)]
use wry::WebViewExtWindows;

use crate::downloads::{self, Download, DownloadState};
use crate::history::{self, HistoryQuery};
use crate::host;
use crate::ipc::{self, Reply, Request, Surface};
use crate::keys::Shortcut;
#[cfg(windows)]
use crate::keys::{self, Mods, Outcome};
use crate::layout::{self, Overlay, Panel};
use crate::omnibox::{self, Intent, Mode};
use crate::platform;
use crate::session::{self, ClosedRecord, GroupRecord, Session, TabRecord, WindowRect};
use crate::settings::{self, Settings};
use crate::store::Store;
use crate::sys;
use crate::tabs::{self, Group, GroupColor, GroupId, Tab, TabId, TabSet};
use crate::ui;

mod agent_glue;
mod ai_glue;
mod canvas;
mod lifecycle;
mod find;
mod omnibox_worker;
mod settings_page;
mod providers;
mod popups;
const SESSION_DEBOUNCE: Duration = Duration::from_millis(500);
const TICK: Duration = Duration::from_millis(250);
const NAV_COALESCE: Duration = Duration::from_millis(100);

/// Retained download records. The panel is a recent-activity list; without a
/// cap, `self.downloads` and every `state.downloads` payload grow for the life
/// of the session (and the session file with them).
const MAX_DOWNLOAD_ROWS: usize = 200;

/// Drop the oldest *completed* download records beyond `MAX_DOWNLOAD_ROWS`.
/// In-progress rows are never dropped: losing one would leave its
/// `DownloadFinished` event unable to find its record.
fn trim_downloads(list: &mut Vec<Download>) {
    while list.len() > MAX_DOWNLOAD_ROWS {
        match list.iter().position(|d| d.state.is_terminal()) {
            Some(i) => {
                list.remove(i);
            }
            None => break,
        }
    }
}

// ---------------------------------------------------------------------------
// 1. State
// ---------------------------------------------------------------------------

struct LiveTab {
    webview: WebView,
    pdf: crate::pdf::PdfSession,
    #[cfg(windows)]
    native_pdf: Option<crate::native_pdf::Session>,
    last_used: Instant,
    lifecycle_known: bool,
    editing: bool,
    playing: bool,
    /// Scroll offset to apply on the next committed navigation (§6.4).
    pending_scroll: Option<f64>,
    /// Last bounds and corner radius applied to the page window; skips redundant re-shaping.
    shaped: std::cell::Cell<Option<(i32, i32, i32, i32, i32)>>,
}

impl Drop for LiveTab {
    fn drop(&mut self) { self.pdf.cancel(); }
}

impl LiveTab {
    fn read_content(&self, script: &str, reply: impl Fn(String) + Send + 'static) -> Result<(), String> {
        #[cfg(windows)]
        if let Some(pdf) = &self.native_pdf {
            return pdf.read(reply);
        }
        self.webview.evaluate_script_with_callback(script, reply).map_err(|error| error.to_string())
    }

    fn capture(&self, reply: impl FnOnce(Result<String, String>) + Send + 'static) -> Result<(), ai_platform_contract::failure::Failure> {
        #[cfg(windows)]
        if let Some(pdf) = &self.native_pdf {
            return pdf.read(move |raw| reply(crate::native_pdf::snapshot(&raw)))
                .map_err(|error| ai_platform_contract::failure::Failure::unavailable("context.native_pdf_unavailable").with_detail(error));
        }
        crate::ai::context::capture(&self.webview, reply)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Egress {
    pub tab_id: u64,
    pub url: String,
    pub started_ms: u64,
}

pub struct AppState {
    data_dir: PathBuf,
    store: Store,
    settings: Settings,
    settings_page: settings_page::State,
    tabs: TabSet,
    groups: Vec<Group>,
    active: Option<TabId>,
    live: HashMap<TabId, LiveTab>,
    /// The canvas tab on screen, if any. While it is set, `active` is that canvas's
    /// focused cell (see `app::canvas`).
    canvas_shown: Option<TabId>,
    downloads: Vec<Download>,
    next_download_id: u64,
    download_owners: HashMap<u64, TabId>,
    last_tab_prune: Instant,
    #[cfg(target_os = "macos")]
    download_destination: Rc<RefCell<PathBuf>>,
    shutting_down: bool,

    shell: Option<WebView>,
    overlay: Option<WebView>,
    /// The docked AI side pane (its own webview, so the omnibox dropdown and the
    /// palette, which share the overlay webview, never replace it).
    sidebar: Option<WebView>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    /// Shared WebView2 environment for every page webview. Taken during a
    /// build and put back immediately after (wry needs `&mut`).
    context: Option<WebContext>,
    /// One WebView2 environment shared by shell, overlay and every page
    /// webview. Without it each webview creates its own browser process.
    environment: Option<platform::Environment>,

    panel: Panel,
    overlay_rect: Overlay,
    // A shell menu taller than the toolbar needs the page windows out of its way.
    menu_open: bool,

    find: Option<FindResult>,
    find_seq: u64,
    omnibox_worker: Option<std::sync::mpsc::Sender<omnibox_worker::Query>>,

    proxy: Option<EventLoopProxy<AppMessage>>,
    start_tick: u64,
    shell_ready_ms: Option<u64>,
    dpi_scale: f64,
    system_dark: bool,
    /// True while the shell reports a text field focused (Alt-nav suppression,
    /// `Ctrl+W` → word delete).
    #[cfg(windows)]
    shell_focus_in_input: bool,
    /// HWND of the shell webview's child window; keystrokes for it are the
    /// shell's own and must not be intercepted twice.
    #[cfg(windows)]
    shell_hwnd: isize,

    session_dirty_at: Option<Instant>,
    last_nav_emit: HashMap<TabId, Instant>,
    navigation_seq: u64,
    /// The page a tab is navigating to *because the user asked for it* (omnibox,
    /// new-tab tile, history row). Consumed by `on_navigation` so the committed
    /// visit is recorded with `typed = true`. Cleared on the next navigation, so
    /// a redirect or link click afterwards is recorded as an ordinary visit.
    typed_target: Option<(TabId, String)>,

    log: VecDeque<String>,
    egress: VecDeque<Egress>,
    /// Self-test mode: report after the first real (non-asset) navigation.
    self_test_url_pending: bool,
    /// `--driver=NAME`: scenario name; empty means the driver is off.
    driver_name: String,
    driver_step: u32,
    driver_total: u32,
    /// Per-step snapshots, collected while the driver runs and drained on exit.
    driver_snaps: Option<Vec<Value>>,
    /// Everything about AI work in flight; see `ai::session` and `app::ai_glue`.
    ai: crate::ai::session::AiSession,
}

#[derive(Debug, Clone, Serialize)]
struct FindResult {
    tab_id: u64,
    query: String,
    total: u64,
    active: u64,
    /// §7 R3: the UI renders its controls from this list, so it never offers a
    /// mode that silently does nothing.
    supported_flags: Vec<&'static str>,
}

pub enum AppMessage {
    /// A raw IPC envelope from the shell or overlay.
    Ipc(Surface, String),
    /// A page webview changed its document title.
    TabTitle(TabId, String),
    /// A page webview started a navigation to `url`.
    TabNavigated(TabId, String),
    OpenPdf(TabId, String),
    #[cfg(windows)]
    DroppedPdf(TabId, String),
    #[cfg(windows)]
    NativePdfSelection(TabId, u64, String, Option<[f64; 4]>),
    #[cfg(windows)]
    RuntimeZoom(TabId, f64),
    #[cfg(windows)]
    PdfDetected(TabId, String),
    PdfRequest(TabId, HttpRequest<Vec<u8>>, wry::RequestAsyncResponder),
    PageLoaded(TabId),
    /// A page asked for a new window (`target=_blank`, `window.open`): it becomes a tab.
    NewWindow(TabId, String),
    /// A download started; `path` is the OS-suggested destination.
    DownloadStarted(TabId, String, PathBuf),
    /// A download finished (or failed).
    DownloadFinished(String, Option<PathBuf>, bool),
    Flush,
    Quit,
    /// Self-test: fired shortly after the shell reports ready.
    SelfTestStart(String),
    /// Self-test: navigation for the URL committed (or the timeout expired).
    SelfTestNavigated,
    /// Driver: run one scenario step (test harness, `--driver=NAME`).
    DriverStep(u32),
    /// Driver: finish, write the report, exit.
    DriverDone,
    /// A frame from the AI utility.
    AiFrame(u64, ai_platform_contract::frame::Frame),
    /// The utility's pipe closed or it exited.
    AiLinkClosed(u64, String),
    /// A DOM snapshot finished for `tab`; `what` says who asked.
    AiCaptured(TabId, ai_glue::CaptureFor, Result<String, String>),
    /// A page script reported its selection. Untrusted: validated on receipt.
    PageProbe(TabId, String),
    /// The left mouse button went down at these screen coordinates (physical px).
    /// Page webviews swallow mouse input, so this is how the canvas learns which
    /// cell was clicked whatever the page is.
    MouseDown(i32, i32),
    /// AppKit supplies window-local logical coordinates, including native child views.
    CanvasPointer(i32, i32),
    NativeShortcut(Shortcut),
    /// Time to read an extra tab for this task (it was woken and needed a moment to load).
    AiReadExtra(u64, TabId),
    /// Run (or retry) the page script of a pending agent tool call, by utility request id.
    AgentEval(u64),
    /// A page script of an agent tool call returned (raw JSON text of its result).
    AgentEvalDone(u64, String),
    /// The active page's content was read for an agent job (None: it could not be, or
    /// took too long, and the job goes ahead without it).
    AgentJob(u64, Option<String>),
    /// A tab's live-context re-read finished (None: the page could not be read).
    PageRead(TabId, Option<ai_platform_contract::identity::DocumentKey>, u64, Option<String>),
    FindDone(TabId, u64, String),
    OmniboxReady(Surface, Option<u64>, Value),
}

type Shared = Rc<RefCell<AppState>>;

fn elapsed_ms(start_tick: u64) -> u64 {
    sys::elapsed_since(start_tick)
}

impl AppState {
    fn note(&mut self, msg: impl Into<String>) {
        let line = format!("[{:>5} ms] {}", elapsed_ms(self.start_tick), msg.into());
        eprintln!("{line}");
        // Durable evidence: append to <data>/browser.log so a run can be audited
        // after the process exits. Opening/closing per line is fine — this is
        // human-rate logging, not a hot path.
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.data_dir.join("browser.log"))
        {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
        self.log.push_back(line);
        while self.log.len() > 500 {
            self.log.pop_front();
        }
    }

    // ---- snapshots ------------------------------------------------------

    fn tab_json(&self, t: &Tab) -> Value {
        json!({
            "id": t.id.0,
            "url": t.url,
            "title": t.display_title(),
            "raw_title": t.title,
            "pinned": t.pinned,
            "group_id": t.group_id.map(|g| g.0),
            "favicon": t.favicon,
            "zoom": t.zoom,
            "loading": t.loading,
            "sleeping": t.sleeping,
            "zoom": t.zoom,
            "can_go_back": t.can_go_back,
            "can_go_forward": t.can_go_forward,
            "active": Some(t.id) == self.active,
            // The canvas on screen is "selected" in the rail even though its focused
            // cell, not the canvas, is `active` (the omnibox follows the cell).
            "shown": Some(t.id) == self.canvas_shown,
            "member_of": t.member_of.map(|c| c.0),
            "canvas": t.canvas.as_ref().map(|c| json!({
                "cols": c.cols, "rows": c.rows, "pages": c.occupied(), "capacity": c.capacity(),
            })),
        })
    }

    fn groups_json(&self) -> Value {
        Value::Array(
            self.groups
                .iter()
                .map(|g| {
                    json!({
                        "id": g.id.0,
                        "name": g.name,
                        "color": g.color,
                        "collapsed": g.collapsed,
                    })
                })
                .collect(),
        )
    }

    fn tabs_payload(&self) -> Value {
        json!({
            "tabs": self.tabs.tabs().iter().map(|t| self.tab_json(t)).collect::<Vec<_>>(),
            "groups": self.groups_json(),
            "active_tab_id": self.active.map(|a| a.0),
            "closed_count": self.tabs.closed_len(),
            "canvas": self.canvas_payload(),
        })
    }

    fn settings_payload(&self) -> Value {
        json!({
            "theme": self.settings.theme,
            "system_dark": self.system_dark,
            "rail_collapsed": self.settings.rail_collapsed,
            "vertical_tabs": self.settings.vertical_tabs,
            "compact_tab_rows": self.settings.compact_tab_rows,
            "density": self.settings.density,
            "search_template": self.settings.search_template,
            "clear_history_on_exit": self.settings.clear_history_on_exit,
            "clear_cookies_on_exit": self.settings.clear_cookies_on_exit,
            "clear_cache_on_exit": self.settings.clear_cache_on_exit,
            "always_show_full_url": self.settings.always_show_full_url,
            "history_cap": self.settings.history_cap,
            "sleep_after_minutes": self.settings.sleep_after_minutes,
            "download_dir": self.download_dir(),
            "history_count": self.store.history_count().unwrap_or(-1),
            "webview2_version": webview2_version(),
            "engine_name": platform::ENGINE_NAME,
            "engine_version": webview_version(),
            "data_dir": self.data_dir,
        })
    }

    fn download_dir(&self) -> String {
        if self.settings.download_dir.trim().is_empty() {
            settings::default_download_dir()
        } else {
            self.settings.download_dir.clone()
        }
    }

    fn viewport(&self) -> (tao::dpi::PhysicalSize<u32>, f64) {
        match &self.window {
            Some(w) => (w.inner_size(), w.scale_factor()),
            None => (tao::dpi::PhysicalSize::new(1280, 820), 1.0),
        }
    }

    fn layout(&self) -> layout::Layout {
        let (size, scale) = self.viewport();
        layout::compute(size, scale, self.settings.tabs(), self.panel, self.overlay_rect)
    }

    // ---- emitting -------------------------------------------------------

    fn emit_raw(&mut self, target: &'static str, event: &str, payload: Value) {
        let env = match serde_json::to_string(&ipc::Event::new(event, payload)) {
            Ok(s) => s,
            Err(e) => {
                self.note(format!("serialize {event}: {e}"));
                return;
            }
        };
        let script = format!("window.__native&&window.__native._dispatch({env})");
        let wv = match target {
            "overlay" => self.overlay.as_ref(),
            "sidebar" => self.sidebar.as_ref(),
            _ => self.shell.as_ref(),
        };
        if let Some(wv) = wv {
            if let Err(e) = wv.evaluate_script(&script) {
                self.note(format!("emit {event} -> {target}: {e}"));
            }
        }
    }

    fn emit(&mut self, event: &str, mut payload: Value) {
        self.ai.ui.record(event, &mut payload);
        self.emit_settings(event, payload.clone());
        self.emit_raw("shell", event, payload.clone());
        if self.sidebar.is_some() {
            self.emit_raw("sidebar", event, payload.clone());
        }
        if self.overlay.is_some() {
            self.emit_raw("overlay", event, payload);
        }
    }

    fn emit_tabs(&mut self) {
        let payload = self.tabs_payload();
        self.emit("state.tabs", payload);
        self.mark_dirty();
    }

    fn reply(&mut self, source: Surface, r: Reply) {
        let Ok(body) = serde_json::to_string(&r) else { return };
        let script = format!("window.__native&&window.__native._reply({body})");
        let view = match source {
            Surface::Shell => self.shell.as_ref(),
            Surface::Overlay => self.overlay.as_ref(),
            Surface::Sidebar => self.sidebar.as_ref(),
            Surface::Settings(id) => self.settings_view(id),
        };
        if let Some(view) = view {
            let _ = view.evaluate_script(&script);
        }
    }

    fn mark_dirty(&mut self) {
        self.session_dirty_at = Some(Instant::now() + SESSION_DEBOUNCE);
    }

    // ---- persistence ----------------------------------------------------

    fn session_path(&self) -> PathBuf {
        self.data_dir.join("session.json")
    }

    fn settings_path(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    fn save_settings(&mut self) {
        if let Err(e) = settings::save(&self.settings_path(), &self.settings) {
            self.note(format!("settings save failed: {e}"));
        }
    }

    fn flush_session(&mut self) {
        let now = ipc::now_secs();
        let records: Vec<TabRecord> = self
            .tabs
            .tabs()
            .iter()
            .map(|t| session::tab_to_record(t, t.scroll_y, now))
            .collect();
        let groups: Vec<GroupRecord> = self.groups.iter().map(session::group_to_record).collect();
        let rect = match &self.window {
            Some(w) => WindowRect {
                x: w.outer_position().map(|p| p.x).unwrap_or(80),
                y: w.outer_position().map(|p| p.y).unwrap_or(60),
                width: w.outer_size().width.max(320),
                height: w.outer_size().height.max(240),
                maximized: w.is_maximized(),
            },
            None => WindowRect::default(),
        };
        let closed: Vec<ClosedRecord> = self
            .tabs
            .closed_snapshot()
            .into_iter()
            .map(|c| ClosedRecord {
                url: c.url,
                title: c.title,
                pinned: c.pinned,
                group_id: c.group_id.map(|g| g.0),
                index: c.index,
            })
            .collect();
        let mut s = Session::capture(session::Captured {
            window: rect,
            rail_collapsed: self.settings.rail_collapsed,
            tabs: records,
            groups,
            active_tab_id: self.active.map(|a| a.0),
            closed,
            downloads: self.downloads.clone(),
        });
        s.theme_override = match self.settings.theme {
            settings::Theme::System => None,
            settings::Theme::Light => Some("light".into()),
            settings::Theme::Dark => Some("dark".into()),
        };
        match session::save(&self.session_path(), &s) {
            Ok(()) => self.session_dirty_at = None,
            Err(e) => {
                self.session_dirty_at = Some(Instant::now() + Duration::from_secs(2));
                self.note(format!("session save failed: {e}"));
            }
        }
    }

    // ---- 2. tabs --------------------------------------------------------

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tabs.get(id))
    }

    /// The URL to load for a tab: internal aliases expand to asset URLs.
    fn resolve_url(&self, url: &str) -> String {
        #[cfg(target_os = "macos")]
        if crate::pdf::is_pdf_url(url) {
            return crate::pdf::viewer_url(url);
        }
        let resolved = if url == tabs::NEWTAB_URL {
            host::newtab_url(&self.store)
        } else if url == "native://settings" {
            host::settings_url(
                &self.settings,
                self.system_dark,
                self.store.history_count().unwrap_or(-1),
                webview2_version(),
            )
        } else {
            host::expand_internal(url, &host::newtab_url(&self.store))
        };
        platform::page_asset_url(&resolved)
    }

    /// A page asked to open a link in a new window. Inside the canvas on screen it
    /// fills the next empty cell (the canvas stays whole); otherwise it is a tab.
    /// Steps (or resets) one tab's page zoom; the chrome scales with it (§8, clamped 0.8–2.0).
    pub(super) fn zoom_tab(&mut self, id: TabId, name: &str) {
        let cur = self.tabs.get(id).map(|t| t.zoom).unwrap_or(1.0);
        let factor = match name {
            "zoom_in" => cur * 1.1,
            "zoom_out" => cur / 1.1,
            _ => 1.0,
        }
        .clamp(0.25, 5.0);
            self.apply_zoom(id, factor);
            if let Some(t) = self.tabs.get_mut(id) {
                t.zoom = factor;
            }
            self.emit("tab.zoom_changed", json!({ "tab_id": id.0, "factor": factor }));
            self.emit("shell.ui_scale", json!({ "scale": factor.clamp(0.8, 2.0) }));
        }

        /// The PDF viewer fits its pages to the window width, so the runtime's zoom would
        /// be undone by the re-fit; its pages are scaled by the viewer itself instead.
        pub(super) fn is_viewer_tab(&self, id: TabId) -> bool {
            self.live
                .get(&id)
                .and_then(|l| l.webview.url().ok())
                .is_some_and(|u| crate::pdf::is_viewer_url(&u))
        }

        pub(super) fn has_native_pdf(&self, id: TabId) -> bool {
            #[cfg(windows)]
            { self.live.get(&id).is_some_and(|live| live.native_pdf.is_some()) }
            #[cfg(not(windows))]
            { let _ = id; false }
        }

        #[cfg(windows)]
        pub(super) fn start_native_pdf(&mut self, id: TabId, source: &str) {
            let Some(tab) = self.tabs.get(id).filter(|tab| tab.url == source) else { return };
            let Some(live) = self.live.get(&id).filter(|live| live.native_pdf.is_none()
                && live.webview.url().is_ok_and(|url| url == source)) else { return };
            let proxy = self.proxy();
            match crate::native_pdf::Session::new(&live.webview, source.to_string(), tab.display_title(),
                move |generation, text, rect| {
                    let _ = proxy.send_event(AppMessage::NativePdfSelection(id, generation, text, rect));
                }) {
                Ok(pdf) => {
                    if let Some(live) = self.live.get_mut(&id) { live.native_pdf = Some(pdf); }
                    self.note(format!("Native PDF text bridge ready for tab {}", id.0));
                    self.sync_native_pdf_zoom(id);
                    self.page_read_start(id);
                }
                Err(error) => self.note(format!("Native PDF extraction: {error}")),
            }
        }

        #[cfg(windows)]
        pub(super) fn sync_native_pdf_zoom(&mut self, id: TabId) {
            if !self.has_native_pdf(id) { return; }
            let Some(factor) = self.live.get(&id).and_then(|live| crate::webview2::zoom_factor(&live.webview).ok())
                .filter(|factor| factor.is_finite() && *factor > 0.0) else { return };
            if let Some(tab) = self.tabs.get_mut(id) { tab.zoom = factor; }
            self.emit("tab.zoom_changed", json!({ "tab_id": id.0, "factor": factor }));
        }

        fn apply_zoom(&self, id: TabId, factor: f64) {
            let Some(live) = self.live.get(&id) else { return };
            if self.is_viewer_tab(id) {
                let _ = live.webview.zoom(1.0);
                let _ = live.webview.evaluate_script(&format!("window.__pdfZoom && window.__pdfZoom({factor})"));
            } else {
                let _ = live.webview.zoom(factor);
            }
        }

    pub(super) fn open_from_page(&mut self, opener: TabId, url: &str) {
        if let Some(canvas) = self.shown_canvas_of(opener) {
            if self.canvas_of(canvas).and_then(|c| c.first_empty()).is_some() {
                let _ = self.canvas_set(&json!({ "canvas_id": canvas.0, "url": url }));
                return;
            }
        }
        self.create_tab(url, Some(opener), true);
    }

    fn create_tab(&mut self, url: &str, after: Option<TabId>, activate: bool) -> TabId {
        let id = self.tabs.alloc_id();
        self.tabs.insert_after(after, Tab::new(id, url));
        if let Some(t) = self.tabs.get(id) {
            let payload = self.tab_json(t);
            self.emit("tab.created", json!({ "tab": payload }));
        }
        // Only the tab that will be shown gets a webview; a background tab stays a
        // row until it is activated (`activate` builds it). Building one here
        // would cost a renderer immediately and then be slept on the next
        // activation anyway.
        if activate {
            self.activate(id);
        }
        self.emit_tabs();
        id
    }

    fn activate(&mut self, id: TabId) {
        let Some(tab) = self.tabs.get(id) else { return };
        // A canvas, or a page inside one, is shown as the whole grid.
        if let Some(parent) = tab.member_of {
            self.show_canvas(parent, Some(id));
            return;
        }
        if tab.canvas.is_some() {
            self.show_canvas(id, None);
            return;
        }
        self.canvas_shown = None;
        // Discarded tabs are rebuilt; warm tabs keep their live document and history.
        self.build_webview(id);
        if let Some(live) = self.live.get_mut(&id) {
            live.last_used = Instant::now();
        }
        self.active = Some(id);
        let ids: Vec<TabId> = self.tabs.tabs().iter().map(|t| t.id).collect();
        for other in ids {
            let visible = other == id;
            // Take the results before touching `self` again: `note` needs
            // `&mut self` while `live` holds a borrow of `self.live`.
            let result = match self.live.get(&other) {
                Some(live) => {
                    let vis = live.webview.set_visible(visible);
                    if visible {
                        let _ = live.webview.focus();
                    }
                    Some(vis)
                }
                None => None,
            };
            if let Some(Err(e)) = result {
                self.note(format!("set_visible({}): {e}", other.0));
            }
        }
        self.sleep_background_tabs(&[id]);
        let title = self.tabs.get(id).map(|t| t.display_title()).unwrap_or_default();
        self.emit(
            "tab.activated",
            json!({ "tab_id": id.0, "title": title }),
        );
        self.emit_tabs();
        self.ai_on_activated(id);
    }

    /// Keep a small warm pool; editing, media, downloads and AI work take precedence.
    fn sleep_background_tabs(&mut self, keep: &[TabId]) {
        if !crate::ai::mode::tabs_may_sleep() {
            return;
        }
        let candidates = self.live.iter().filter(|(id, _)| !keep.contains(id)).map(|(id, live)| {
            let tab = self.tabs.get(*id);
            crate::tab_lifecycle::Candidate {
                id: id.0,
                idle: live.last_used.elapsed(),
                protected: !live.lifecycle_known || live.editing || live.playing
                    || tab.is_some_and(|t| t.pinned || t.loading)
                    || self.download_owners.values().any(|owner| owner == id)
                    || self.ai.agent_run != 0
                    || self.ai.tasks.values().any(|task| task.tab == id.0 || task.extra_tabs.contains(&id.0))
                    || self.ai.agent_evals.values().any(|eval| eval.tab == id.0),
            }
        }).collect();
        let victims = crate::tab_lifecycle::eviction_candidates(candidates,
            Duration::from_secs(u64::from(self.settings.sleep_after_minutes) * 60));
        if victims.is_empty() {
            return;
        }
        let mut slept = 0usize;
        for id in victims.into_iter().map(TabId) {
            if self.live.remove(&id).is_some() {
                slept += 1;
            }
            if let Some(t) = self.tabs.get_mut(id) {
                t.sleeping = true;
            }
        }
        if slept > 0 {
            self.note(format!("slept {slept} background webview(s)"));
        }
    }

    fn close_tab(&mut self, id: TabId) {
        // Closing a page inside a canvas clears its cell; the canvas itself closes
        // its pages first.
        if let Some(parent) = self.tabs.get(id).and_then(|t| t.member_of) {
            self.canvas_close_member(parent, id);
            return;
        }
        let Some(pos) = self.tabs.position(id) else { return };
        let is_canvas = self.canvas_of(id).is_some();
        let was_active = if is_canvas { self.canvas_shown == Some(id) } else { self.active == Some(id) };
        if is_canvas {
            self.canvas_close(id);
            if was_active {
                self.canvas_shown = None;
            }
        }
        let pos = self.tabs.rail().position(|t| t.id == id).unwrap_or(pos);
        self.live.remove(&id);
        self.ai_on_tab_closed(id);
        self.last_nav_emit.remove(&id);
        if self.tabs.close(id).is_none() {
            return;
        }
        self.emit("tab.closed", json!({ "tab_id": id.0 }));
        if was_active {
            let next = self
                .tabs
                .at_index(pos)
                .or_else(|| pos.checked_sub(1).and_then(|p| self.tabs.at_index(p)))
                .or_else(|| self.tabs.last());
            self.active = None;
            if let Some(n) = next {
                self.activate(n);
            }
        }
        if self.tabs.is_empty() {
            self.create_tab(tabs::NEWTAB_URL, None, true);
        }
        self.emit_tabs();
    }

    fn step_tab(&mut self, delta: isize) {
        if let Some(cur) = self.active {
            if let Some(next) = self.tabs.step(cur, delta) {
                self.activate(next);
            }
        }
    }

    fn refresh_history_state(&mut self, id: TabId) {
        let (back, forward) = self.live.get(&id).map(|live| (
            live.webview.can_go_back().unwrap_or(false),
            live.webview.can_go_forward().unwrap_or(false),
        )).unwrap_or_default();
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.can_go_back = back;
            tab.can_go_forward = forward;
        }
    }

    /// Start a navigation in a tab. `typed` marks an omnibox-initiated visit so
    /// history ranking can favour it.
    fn navigate(&mut self, tab: TabId, url: &str, typed: bool) {
        let resolved = self.resolve_url(url);
        let Some(t) = self.tabs.get_mut(tab) else { return };
        t.url = url.to_string();
        t.loading = true;
        let Some(live) = self.live.get(&tab) else { return };
        if let Err(e) = live.webview.load_url(&resolved) {
            self.note(format!("load_url({resolved}): {e}"));
            self.emit(
                "nav.failed",
                json!({ "code": "LOAD_ERROR", "url": url, "navigation_id": self.navigation_seq, "tab_id": tab.0 }),
            );
        }
        if typed {
            // Recorded on commit (`on_navigation`), which is the only place that
            // sees redirects and link clicks too. Marking the target here keeps
            // the "user asked for this URL" fact without double-counting.
            self.typed_target = Some((tab, url.to_string()));
        }
        self.egress.push_back(Egress {
            tab_id: tab.0,
            url: resolved,
            started_ms: elapsed_ms(self.start_tick),
        });
        while self.egress.len() > 200 {
            self.egress.pop_front();
        }
        self.emit(
            "tab.loading",
            json!({ "tab_id": tab.0, "url": url, "is_loading": true }),
        );
        self.emit_tabs();
    }

    // ---- 3. panels ------------------------------------------------------

    fn set_panel(&mut self, panel: Panel) {
        self.panel = panel;
        if panel != Panel::History {
            self.overlay_rect = if self.overlay_rect == Overlay::Palette {
                self.overlay_rect
            } else {
                Overlay::Closed
            };
        }
        // History is a full-page overlay surface.
        if panel == Panel::History {
            self.overlay_rect = Overlay::Closed;
        }
        self.apply_layout();
        self.emit("shell.panel", json!({ "panel": panel_str(panel) }));
        if panel == Panel::History {
            self.emit_raw(
                "overlay",
                "overlay.state",
                json!({ "open": true, "kind": "full_page" }),
            );
        }
    }

    fn open_palette(&mut self) {
        let l = layout::compute(
            self.viewport().0,
            self.viewport().1,
            self.settings.tabs(),
            self.panel,
            Overlay::Palette,
        );
        self.overlay_rect = Overlay::Palette;
        if let Some(ov) = self.overlay.as_ref() {
            let _ = ov.set_bounds(layout::rect(l.overlay_x, l.overlay_y, l.overlay_w, l.overlay_h));
            let _ = ov.set_visible(true);
            let _ = ov.focus();
        }
        self.emit_raw(
            "overlay",
            "overlay.state",
            json!({ "open": true, "kind": "palette" }),
        );
        self.emit("shell.overlay", json!({ "kind": "palette" }));
    }

    fn open_dropdown(&mut self, x: i32, y: i32, w: i32) {
        let l = layout::compute(
            self.viewport().0,
            self.viewport().1,
            self.settings.tabs(),
            self.panel,
            Overlay::Dropdown { x, y, w },
        );
        self.overlay_rect = Overlay::Dropdown { x, y, w };
        if let Some(ov) = self.overlay.as_ref() {
            let _ = ov.set_bounds(layout::rect(l.overlay_x, l.overlay_y, l.overlay_w, l.overlay_h));
            let _ = ov.set_visible(true);
        }
        self.emit_raw(
            "overlay",
            "overlay.state",
            json!({ "open": true, "kind": "dropdown" }),
        );
    }

    fn close_overlay(&mut self) {
        self.overlay_rect = Overlay::Closed;
        if let Some(ov) = self.overlay.as_ref() {
            let _ = ov.set_visible(false);
            let _ = ov.set_bounds(layout::rect(0, 0, 0, 0));
        }
        self.emit_raw("overlay", "overlay.state", json!({ "open": false }));
        // §3.6: focus returns to the page.
        if let Some(id) = self.active {
            if let Some(live) = self.live.get(&id) {
                let _ = live.webview.focus();
            }
        }
    }

    /// Corner radius of a split-view pane, in logical pixels.
    const PANE_RADIUS: f64 = 6.0;

    /// A page in a canvas cell gets rounded corners (like Edge's split view); any other
    /// page is square.
    fn shape_tab(&self, id: TabId) {
        let Some(window) = self.window.as_ref() else { return };
        let Some(live) = self.live.get(&id) else { return };
        let (r, shown) = self.tab_geom(id);
        let in_grid = shown == Some(true) && r != self.canvas_area_rect();
        let scale = window.scale_factor();
        let phys = |v: i32| (f64::from(v) * scale).round() as i32;
        let radius = if in_grid { (Self::PANE_RADIUS * scale).round() as i32 } else { 0 };
        let key = (phys(r.x), phys(r.y), phys(r.w), phys(r.h), radius);
        if live.shaped.get() == Some(key) {
            return;
        }
        live.shaped.set(Some(key));
        platform::shape_webview(&live.webview, window, (key.0, key.1, key.2, key.3), radius);
    }

    fn apply_layout(&mut self) {
        let l = self.layout();
        if let Some(wv) = self.shell.as_ref() {
            if let Err(e) = wv.set_bounds(layout::rect(0, 0, l.shell_w, l.shell_h)) {
                self.note(format!("shell bounds: {e}"));
            }
        }
        let ids: Vec<TabId> = self.live.keys().copied().collect();
        for id in ids {
            let (r, shown) = self.tab_geom(id);
            if let Some(live) = self.live.get(&id) {
                let _ = live.webview.set_bounds(layout::rect(r.x, r.y, r.w, r.h));
                // Cells of a zoomed canvas keep their page alive but out of sight.
                if self.menu_open {
                    let _ = live.webview.set_visible(false);
                } else if let Some(visible) = shown {
                    let _ = live.webview.set_visible(visible);
                } else {
                    let _ = live.webview.set_visible(self.active == Some(id));
                }
            }
            self.shape_tab(id);
        }
        if let Some(ov) = self.overlay.as_ref() {
            if self.overlay_rect.is_open() {
                let _ = ov.set_bounds(layout::rect(l.overlay_x, l.overlay_y, l.overlay_w, l.overlay_h));
            }
        }
        self.ai_layout_sidebar();
        self.emit_raw(
            "shell",
            "shell.layout",
            json!({
                "rail_w": self.settings.tabs().rail_w(),
                "header_h": l.header_h,
                "vertical_tabs": self.settings.vertical_tabs,
                "rail_collapsed": self.settings.rail_collapsed,
                "panel": panel_str(self.panel),
                "page_x": l.page_x,
                "page_y": l.page_y,
                "page_w": l.page_w,
                "page_h": l.page_h,
                "canvas": self.canvas_payload(),
            }),
        );
    }

    // ---- 4. command dispatch -------------------------------------------

    fn dispatch(&mut self, source: Surface, raw: &str) -> Option<Reply> {
        let req: Request = match serde_json::from_str(raw) {
            Ok(r) => r,
            Err(e) => return Some(Reply::err(None, format!("bad envelope: {e}"))),
        };
        let id = req.id;
        match self.run_from(&req, source) {
            Ok(data) => match data {
                Some(d) => Some(Reply::ok(id, d)),
                None if req.cmd == "omnibox.query" => None,
                None => Some(Reply::empty(id)),
            },
            Err(e) => {
                self.note(format!("{} failed: {e}", req.cmd));
                Some(Reply::err(id, e))
            }
        }
    }

    fn run(&mut self, req: &Request) -> Result<Option<Value>, String> {
        self.run_from(req, Surface::Shell)
    }

    fn run_from(&mut self, req: &Request, source: Surface) -> Result<Option<Value>, String> {
        let p = &req.payload;
        let id_arg = || p.get("tab_id").and_then(Value::as_u64).map(TabId).or(self.active);
        match req.cmd.as_str() {
            "shell.ready" => {
                self.shell_ready_ms = Some(elapsed_ms(self.start_tick));
                self.note("shell ready");
                self.emit_tabs();
                self.emit("state.settings", self.settings_payload());
                self.emit("state.downloads", json!({ "downloads": self.downloads }));
                self.emit("app.ready", json!({}));
                self.apply_layout();
                // The shell is painted, so the rail is already on screen (W1).
                // Now create the active tab's webview and load its URL: this is
                // the ordering §6.4 prescribes, and it is why navigation is not
                // done during startup.
                if let Some(canvas) = self.canvas_shown {
                    self.show_canvas(canvas, self.active);
                } else if let Some(id) = self.active {
                    self.build_webview(id);
                    let url = self.tabs.get(id).map(|t| t.url.clone()).unwrap_or_default();
                    self.navigate(id, &url, false);
                }
                // The driver runs after the first navigation has had a moment
                // to commit, on the event loop, so state matches a live session.
                if !self.driver_name.is_empty() && self.driver_step == 0 && self.driver_snaps.is_none() {
                    self.driver_snaps = Some(Vec::new());
                    let proxy = self.proxy();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(2_000));
                        // `driver_step` is the 0-based index of the step to run next
                        // and `DriverStep(i)` is dropped unless `i` equals it, so
                        // waking with 1 skipped every scenario's first step and then
                        // wedged the driver: no step ever ran and the process never
                        // exited.
                        let _ = proxy.send_event(AppMessage::DriverStep(0));
                    });
                }
                Ok(None)
            }
            "ai.ask" => self.ai_ask(p),
            "ai.stop" => self.ai_stop(p),
            "ai.status" => Ok(Some(self.ai_status())),
            "ai.session" => Ok(Some(self.ai.ui.snapshot())),
            "ai.models" => self.ai_models(),
            "ai.config.set" => self.ai_config_set(p),
            "ai.provider.set" => self.ai_provider_set(p),
            "ai.provider.delete" => self.ai_provider_delete(p),
            "ai.conversation.open" => {
                if self.ai.agent_run != 0 || !self.ai.tasks.is_empty() || !self.ai.agent_jobs.is_empty() {
                    return Err("stop the current task before opening a conversation".into());
                }
                let id = p.get("id").and_then(Value::as_str).ok_or("conversation ID required")?;
                self.settings_page.conversation = Some(id.to_string());
                self.ai_show_pane();
                self.emit_raw("sidebar", "conversation.open", json!({}));
                Ok(None)
            }
            "ai.conversation.pending" => Ok(Some(json!({ "id": self.settings_page.conversation.take() }))),
            "ai.history.clear" => self.ai_history_clear(),
            "agent.run" => self.agent_run(p),
            "agent.stop" => self.agent_stop(),
            "agent.approve" => self.agent_approve(p),
            "agent.call" => self.agent_call(p, source),
            "agent.context" => Ok(Some(self.agent_context())),
            "ai.pane.resize" => {
                let wanted = p.get("width").and_then(Value::as_f64).ok_or("ai.pane.resize: missing width")?.round() as i32;
                let l = self.layout();
                let rail = self.settings.tabs().rail_w();
                let w = layout::set_ai_width(wanted, l.shell_w, rail);
                self.apply_layout();
                Ok(Some(json!({ "width": w })))
            }
            "ai.citation.scroll" => self.ai_citation_scroll(p),
            "ai.selection.act" => self.ai_selection_act(p),
            "nav.open" => {
                let url = p.get("url").and_then(Value::as_str).ok_or("nav.open: missing url")?.to_string();
                match p.get("disposition").and_then(Value::as_str).unwrap_or("current") {
                    "new_tab" => {
                        self.create_tab(&url, self.active, true);
                    }
                    "new_tab_background" => {
                        self.create_tab(&url, self.active, false);
                    }
                    _ => {
                        if let Some(id) = id_arg() {
                            // A `nav.open` from the shell is a user choice: a
                            // new-tab tile, a recent/history row, a menu item.
                            // It outranks a followed link in `history::score`.
                            self.navigate(id, &url, true);
                        }
                    }
                };
                Ok(None)
            }
            "nav.back" | "nav.forward" | "nav.reload" | "nav.reload_ignoring_cache" | "nav.stop" => {
                let Some(id) = id_arg() else { return Ok(None) };
                let Some(live) = self.live.get(&id) else { return Ok(None) };
                let r = match req.cmd.as_str() {
                    "nav.back" => live.webview.go_back(),
                    "nav.forward" => live.webview.go_forward(),
                    "nav.reload" | "nav.reload_ignoring_cache" => live.webview.reload(),
                    _ => live.webview.evaluate_script("window.stop()"),
                };
                if let Err(e) = r {
                    return Err(format!("{}: {e}", req.cmd));
                }
                Ok(None)
            }
            "canvas.create" => self.canvas_create(p),
            "canvas.set" => self.canvas_set(p),
            "canvas.focus" => self.canvas_focus(p),
            "canvas.zoom" => self.canvas_zoom(p),
            "canvas.layout" => self.canvas_layout(p),
            "canvas.resize" => self.canvas_resize(p),
            "page.dismiss_popups" => {
                let targets = self.dismiss_targets(p)?;
                self.ai.agent_job_seq += 1;
                self.dismiss_start((1 << 62) + self.ai.agent_job_seq, targets, false);
                Ok(None)
            }
            "shell.menu" => {
                self.menu_open = p.get("open").and_then(Value::as_bool).unwrap_or(false);
                self.apply_layout();
                Ok(None)
            }
            "canvas.clear" => self.canvas_clear(p),
            "canvas.state" => Ok(Some(self.canvas_payload())),
            "tab.select" => {
                if let Some(id) = id_arg() {
                    self.activate(id);
                }
                Ok(None)
            }
            "tab.close" => {
                if let Some(id) = id_arg() {
                    self.close_tab(id);
                }
                Ok(None)
            }
            "tab.new" => {
                self.create_tab(tabs::NEWTAB_URL, self.active, true);
                Ok(None)
            }
            "tab.duplicate" => {
                if let Some(url) = self.active_tab().map(|t| t.url.clone()) {
                    self.create_tab(&url, self.active, true);
                }
                Ok(None)
            }
            "tab.reopen_closed" => {
                if let Some(c) = self.tabs.take_closed() {
                    let id = self.tabs.alloc_id();
                    let mut tab = Tab::new(id, &c.url);
                    tab.pinned = c.pinned;
                    tab.group_id = c.group_id;
                    self.tabs.insert_at(c.index, tab);
                    self.build_webview(id);
                    self.activate(id);
                    self.emit_tabs();
                }
                Ok(None)
            }
            "tab.pin" | "tab.unpin" => {
                if let Some(id) = id_arg() {
                    if let Some(t) = self.tabs.get_mut(id) {
                        t.pinned = req.cmd == "tab.pin";
                    }
                    self.emit_tabs();
                }
                Ok(None)
            }
            "tab.move" => {
                let Some(id) = id_arg() else { return Err("tab.move: missing tab_id".into()) };
                let before = p.get("before_tab_id").and_then(Value::as_u64).map(TabId);
                let after = p.get("after_tab_id").and_then(Value::as_u64).map(TabId);
                let group = p.get("group_id").map(|v| v.as_u64().map(GroupId));
                let moved = self.tabs.move_tab(id, before, after, Some(group.flatten()));
                if moved {
                    self.emit_tabs();
                }
                Ok(Some(json!({ "moved": moved })))
            }
            "tab.bookmark" => {
                let Some(t) = self.active_tab().cloned() else { return Ok(None) };
                let on = self
                    .store
                    .toggle_bookmark(&t.url, &t.display_title(), ipc::now_secs())
                    .map_err(|e| e.to_string())?;
                self.emit("state.tabs", self.tabs_payload());
                Ok(Some(json!({ "bookmarked": on })))
            }
            "tab.bookmark_all" => {
                let pairs: Vec<(String, String)> = self
                    .tabs
                    .tabs()
                    .iter()
                    .map(|t| (t.url.clone(), t.display_title()))
                    .collect();
                let now = ipc::now_secs();
                for (url, title) in pairs {
                    self.store.add_bookmark(&url, &title, now).map_err(|e| e.to_string())?;
                }
                Ok(None)
            }
            "tab.zoom_set" | "tab.zoom_reset" => {
                let Some(id) = id_arg() else { return Ok(None) };
                let factor = if req.cmd == "tab.zoom_reset" {
                    1.0
                } else {
                    p.get("factor").and_then(Value::as_f64).unwrap_or(1.0)
                }
                .clamp(0.25, 5.0);
                if self.live.contains_key(&id) {
                    self.apply_zoom(id, factor);
                }
                if let Some(t) = self.tabs.get_mut(id) {
                    t.zoom = factor;
                }
                self.emit("tab.zoom_changed", json!({ "tab_id": id.0, "factor": factor }));
                self.mark_dirty();
                Ok(Some(json!({ "factor": factor })))
            }
            "group.create" => {
                let gid = self.tabs.alloc_group_id();
                let color = self.groups.last().map(|g| g.color.next()).unwrap_or(GroupColor::Blue);
                self.groups.push(Group {
                    id: gid,
                    name: p.get("name").and_then(Value::as_str).unwrap_or("Group").to_string(),
                    color,
                    collapsed: false,
                });
                if let Some(ids) = p.get("tab_ids").and_then(Value::as_array) {
                    for v in ids.iter().filter_map(Value::as_u64) {
                        if let Some(t) = self.tabs.get_mut(TabId(v)) {
                            t.group_id = Some(gid);
                        }
                    }
                }
                self.emit("state.groups", self.groups_json());
                self.emit_tabs();
                Ok(Some(json!({ "group_id": gid.0 })))
            }
            "group.rename" | "group.set_color" | "group.collapse" | "group.delete" => {
                let gid = p.get("group_id").and_then(Value::as_u64).map(GroupId);
                let Some(pos) = gid.and_then(|g| self.groups.iter().position(|x| x.id == g)) else {
                    return Err("unknown group".into());
                };
                match req.cmd.as_str() {
                    "group.rename" => {
                        self.groups[pos].name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string()
                    }
                    "group.collapse" => {
                        self.groups[pos].collapsed = p.get("collapsed").and_then(Value::as_bool).unwrap_or(false)
                    }
                    "group.set_color" => {
                        self.groups[pos].color = match p.get("color").and_then(Value::as_str) {
                            Some("green") => GroupColor::Green,
                            Some("amber") => GroupColor::Amber,
                            Some("red") => GroupColor::Red,
                            Some("violet") => GroupColor::Violet,
                            Some("slate") => GroupColor::Slate,
                            _ => GroupColor::Blue,
                        }
                    }
                    _ => {
                        let g = self.groups.remove(pos);
                        for t in self.tabs.tabs_mut() {
                            if t.group_id == Some(g.id) {
                                t.group_id = None;
                            }
                        }
                    }
                }
                self.emit("state.groups", self.groups_json());
                self.emit_tabs();
                Ok(None)
            }
            "find.start" | "find.query" | "find.next" | "find.prev" | "find.stop" => {
                self.find_query(&req.cmd, p)
            }
            "omnibox.query" => {
                let text = p.get("text").and_then(Value::as_str).unwrap_or("");
                let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(8).clamp(1, 40) as usize;
                let res = self.omnibox_results(text, limit);
                if let Some(worker) = &self.omnibox_worker {
                    if worker.send(omnibox_worker::Query { source, id: req.id, text: text.chars().take(2_000).collect(),
                        limit, base: res.clone() }).is_ok() {
                        return Ok(None);
                    }
                }
                Ok(Some(res))
            }
            "omnibox.activate" => {
                let text = p.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                let row_url = p.get("url").and_then(Value::as_str).map(str::to_string);
                let verb = p.get("verb").and_then(Value::as_str).map(str::to_string);
                self.close_overlay();
                if let Some(verb) = verb {
                    self.run_verb(&verb);
                    return Ok(None);
                }
                let disposition = p.get("disposition").and_then(Value::as_str).unwrap_or("current");
                if let Some(url) = row_url {
                    self.open_url_with(url, disposition);
                    return Ok(None);
                }
                match omnibox::classify(&text) {
                    Intent::Command { verb, .. } => self.run_verb(&verb),
                    Intent::Find(q) => {
                        self.emit("shell.focus_find", json!({ "query": q }));
                    }
                    Intent::Ask { question, intent, all_tabs } => {
                        self.ai_show_pane();
                        if let Err(e) = self.ai_ask(&json!({ "question": question, "intent": intent, "tabs": if all_tabs { "all" } else { "" } })) {
                            self.emit("ai.notice", json!({ "text": e }));
                        }
                    }
                    Intent::Url(u) => self.open_url_with(u, disposition),
                    Intent::Search(q) => {
                        let url = omnibox::search_url(&self.settings.search_template, &q);
                        self.open_url_with(url, disposition);
                    }
                }
                Ok(None)
            }
            "history.query" => {
                let tokens = p.get("tokens").and_then(Value::as_str).unwrap_or("");
                let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(300) as usize;
                let q: HistoryQuery = history::parse_query(tokens);
                let entries = self
                    .store
                    .search_history(&q, limit, ipc::now_secs())
                    .map_err(|e| e.to_string())?;
                let rows: Vec<Value> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "id": e.id,
                            "url": e.url,
                            "title": if e.title.is_empty() { history::host_of(&e.url) } else { e.title.as_str() },
                            "host": history::host_of(&e.url),
                            "visited_at": e.visited_at,
                            "visit_count": e.visit_count,
                        })
                    })
                    .collect();
                let out = json!({ "rows": rows });
                self.emit("history.results", out.clone());
                Ok(Some(out))
            }
            "history.forget" => {
                let removed = if let Some(site) = p.get("site").and_then(Value::as_str) {
                    self.store.forget_site(site).map_err(|e| e.to_string())?
                } else {
                    let ids: Vec<i64> = p
                        .get("entry_ids")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_i64).collect())
                        .unwrap_or_default();
                    self.store.forget_ids(&ids).map_err(|e| e.to_string())?
                };
                self.emit(
                    "state.history_meta",
                    json!({ "removed": removed, "count": self.store.history_count().ok() }),
                );
                Ok(Some(json!({ "removed": removed })))
            }
            "history.clear" => {
                let removed = if p.get("scope").and_then(Value::as_str) == Some("range") {
                    let from = p.get("from").and_then(Value::as_i64).unwrap_or(0);
                    let to = p.get("to").and_then(Value::as_i64).unwrap_or(i64::MAX);
                    self.store.clear_history_range(from, to).map_err(|e| e.to_string())?
                } else {
                    self.store.clear_history().map_err(|e| e.to_string())?
                };
                self.emit(
                    "state.history_meta",
                    json!({ "removed": removed, "count": self.store.history_count().ok() }),
                );
                Ok(Some(json!({ "removed": removed })))
            }
            "bookmark.toggle" => {
                let url = p.get("url").and_then(Value::as_str).ok_or("missing url")?;
                let title = p.get("title").and_then(Value::as_str).unwrap_or("");
                let on = self
                    .store
                    .toggle_bookmark(url, title, ipc::now_secs())
                    .map_err(|e| e.to_string())?;
                Ok(Some(json!({ "bookmarked": on })))
            }
            "bookmark.remove" => {
                let id = p.get("id").and_then(Value::as_i64).ok_or("missing id")?;
                self.store.remove_bookmark(id).map_err(|e| e.to_string())?;
                Ok(None)
            }
            "download.action" => {
                let did = p.get("download_id").and_then(Value::as_u64).ok_or("missing download_id")?;
                let action = p.get("action").and_then(Value::as_str).unwrap_or("");
                let Some(idx) = self.downloads.iter().position(|d| d.id == did) else {
                    return Err("unknown download".into());
                };
                let path = self.downloads[idx].path.clone();
                match action {
                    "open" if !sys::shell_open(&path) => return Err(format!("cannot open {path}")),
                    "reveal" if !sys::shell_reveal(&path) => return Err(format!("cannot reveal {path}")),
                    "remove" => {
                        self.downloads.remove(idx);
                    }
                    "open" | "reveal" => {}
                    other => return Err(format!("unsupported download action: {other}")),
                }
                self.emit("state.downloads", json!({ "downloads": self.downloads }));
                self.mark_dirty();
                Ok(None)
            }
            "download.clear_list" => {
                self.downloads.retain(|d| d.state == DownloadState::InProgress);
                self.emit("state.downloads", json!({ "downloads": self.downloads }));
                self.mark_dirty();
                Ok(None)
            }
            "settings.update" => {
                let key = p.get("key").and_then(Value::as_str).ok_or("missing key")?;
                let value = p.get("value").cloned().unwrap_or(Value::Null);
                if matches!(source, Surface::Settings(_)) {
                    self.update_preferences(key, &value)?;
                    return Ok(Some(self.settings_payload()));
                }
                self.settings.apply(key, &value).map_err(|e| e.to_string())?;
                #[cfg(target_os = "macos")]
                if key == "download_dir" {
                    *self.download_destination.borrow_mut() = PathBuf::from(self.download_dir());
                }
                if key == "history_cap" {
                    self.store
                        .vacuum_after_cap_change(self.settings.history_cap)
                        .map_err(|e| e.to_string())?;
                }
                self.save_settings();
                if key == "rail_collapsed" || key == "vertical_tabs" {
                    self.apply_layout();
                }
                let payload = self.settings_payload();
                self.emit("state.settings", payload.clone());
                self.emit_tabs();
                Ok(Some(payload))
            }
            "settings.open_folder" => {
                let dir = self.data_dir.display().to_string();
                if !sys::shell_open_folder(&dir) {
                    return Err(format!("cannot open {dir}"));
                }
                Ok(None)
            }
            "settings.open" => {
                let section = p.get("section").and_then(Value::as_str).unwrap_or("general");
                self.open_settings_section(section, p.get("skill_draft").cloned());
                Ok(None)
            }
            "settings.state" => Ok(Some(json!({
                "section": self.settings_page.section,
                "skill_draft": self.settings_page.skill_draft.take(),
                "settings": self.settings_payload(), "downloads": self.downloads,
            }))),
            "settings.section" => {
                let section = p.get("section").and_then(Value::as_str).unwrap_or("general");
                if !settings_page::valid_section(section) { return Err("unknown settings section".into()); }
                self.settings_page.section = section.to_string();
                Ok(None)
            }
            "shell.focus_omnibox" => {
                self.close_overlay();
                self.emit("shell.focus_changed", json!({ "scope": "omnibox", "select_all": true }));
                Ok(None)
            }
            "shell.focus_page" => {
                self.close_overlay();
                Ok(None)
            }
            "shell.open_palette" => {
                self.open_palette();
                Ok(None)
            }
            "shell.toggle_rail" => {
                self.settings.rail_collapsed = !self.settings.rail_collapsed;
                self.save_settings();
                self.apply_layout();
                let payload = self.settings_payload();
                self.emit("state.settings", payload);
                Ok(None)
            }
            crate::ai::proof::OVERLAY_STEP => {
                // Measurement-only: building or destroying the overlay webview moves
                // the process count, so this is reachable only inside a run started
                // with `--proof` (`mode::active()` is false otherwise, and nothing in
                // shipping code sends the command). Page webviews install only
                // the selection probe handler, which feeds `PageProbe` and never this
                // dispatcher, so this check guards a surface a page cannot reach.
                if !crate::ai::mode::active() {
                    return Err(format!(
                        "{} is a measurement command; start the binary with --proof",
                        crate::ai::proof::OVERLAY_STEP
                    ));
                }
                let enable = p.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                let changed = if enable {
                    // Idempotent by refusal: a second live overlay would double the
                    // exact cost the harness is trying to measure.
                    let build = self.overlay.is_none();
                    if build {
                        self.build_overlay_webview();
                    }
                    build
                } else {
                    // Dropping the `WebView` is the same teardown
                    // `sleep_background_tabs` relies on: wry destroys the controller
                    // and the browser process retires the renderer.
                    let dropped = self.overlay.take().is_some();
                    if dropped {
                        self.note("overlay webview dropped (measurement mode)");
                    }
                    dropped
                };
                Ok(Some(json!({
                    "overlay": self.overlay.is_some(),
                    "changed": changed,
                })))
            }
            crate::ai::proof::WAIT_STEP => {
                // Measurement-only, same gate as the overlay step: this command
                // mutates no shell state — its only effect is that the driver
                // spends its settle window on it, which is what lets the harness
                // watch a background tab with nothing in flight.
                if !crate::ai::mode::active() {
                    return Err(format!(
                        "{} is a measurement command; start the binary with --proof",
                        crate::ai::proof::WAIT_STEP
                    ));
                }
                let ms = p
                    .get("ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(crate::ai::proof::MAX_WAIT_MS);
                // One log line per window, so `browser.log` shows the page's own
                // `tab N navigated:` lines landing *between* two of these — the
                // evidence that a reload happened with no command in flight. It is
                // the only thing this step does, and it is stderr plus one append.
                self.note(format!(
                    "proof: waiting {ms} ms with no command in flight"
                ));
                Ok(Some(json!({ "waited_ms": ms })))
            }
            "shell.close_top_overlay" => {
                if self.overlay_rect.is_open() {
                    self.close_overlay();
                } else if self.panel != Panel::None {
                    self.set_panel(Panel::None);
                    self.emit("shell.focus_changed", json!({ "scope": "page" }));
                } else if let Some(id) = self.active {
                    if let Some(live) = self.live.get(&id) {
                        let _ = live.webview.evaluate_script("window.stop()");
                    }
                }
                Ok(None)
            }
            "shell.omnibox_dropdown" => {
                let open = p.get("open").and_then(Value::as_bool).unwrap_or(false);
                if open {
                    let x = p.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                    let y = p.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                    let w = p.get("w").and_then(Value::as_i64).unwrap_or(480) as i32;
                    self.open_dropdown(x, y, w);
                } else {
                    self.close_overlay();
                }
                Ok(None)
            }
            "shell.input_focus" => {
                // No host-side flag needed: accelerator resolution reads the
                // focused HWND, which is the shell while an input has focus.
                Ok(None)
            }
            "shortcut" => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("");
                let args = p.get("args").cloned().unwrap_or(Value::Null);
                self.perform_shortcut(name, args);
                Ok(None)
            }
            "window.minimize" => {
                if let Some(w) = self.window.as_ref() {
                    w.set_minimized(true);
                }
                Ok(None)
            }
            "window.toggle_maximize" => {
                if let Some(w) = self.window.as_ref() {
                    w.set_maximized(!w.is_maximized());
                }
                Ok(None)
            }
            "window.drag" => {
                if let Some(w) = self.window.as_ref() {
                    let _ = w.drag_window();
                }
                Ok(None)
            }
            "window.resize" => {
                use tao::window::ResizeDirection as D;
                let dir = match p.get("dir").and_then(Value::as_str).unwrap_or("") {
                    "n" => D::North,
                    "s" => D::South,
                    "e" => D::East,
                    "w" => D::West,
                    "ne" => D::NorthEast,
                    "nw" => D::NorthWest,
                    "se" => D::SouthEast,
                    "sw" => D::SouthWest,
                    _ => return Ok(None),
                };
                if let Some(w) = self.window.as_ref() {
                    if !w.is_maximized() {
                        let _ = w.drag_resize_window(dir);
                    }
                }
                Ok(None)
            }
            "window.close" => {
                if let Some(p) = self.proxy.as_ref() {
                    let _ = p.send_event(AppMessage::Quit);
                }
                Ok(None)
            }
            "page.copy_url" => {
                let url = self.active_tab().map(|t| t.url.clone()).unwrap_or_default();
                self.emit("shell.copy", json!({ "text": url }));
                Ok(None)
            }
            "diagnostics.self_test" => {
                let report = self.self_test();
                Ok(Some(report))
            }
            other => Err(format!("unknown command: {other}")),
        }
    }

    fn open_url_with(&mut self, url: String, disposition: &str) {
        match disposition {
            "new_tab" => {
                self.create_tab(&url, self.active, true);
            }
            "new_tab_background" => {
                self.create_tab(&url, self.active, false);
            }
            _ => {
                if let Some(id) = self.active {
                    self.navigate(id, &url, true);
                }
            }
        }
    }

    fn run_verb(&mut self, verb: &str) {
        let cmd = match verb {
            "new-tab" => Some("tab.new"),
            "new-canvas" => Some("canvas.create"),
            "canvas-zoom" => Some("canvas.zoom"),
            "close-tab" => Some("tab.close"),
            "reopen-tab" => Some("tab.reopen_closed"),
            "bookmark" => Some("tab.bookmark"),
            "reload" => Some("nav.reload"),
            "zoom-reset" => Some("tab.zoom_reset"),
            _ => None,
        };
        if let Some(cmd) = cmd {
            let req = Request {
                id: None,
                cmd: cmd.to_string(),
                payload: json!({}),
            };
            if let Err(e) = self.run(&req) {
                self.note(format!("verb {verb}: {e}"));
            }
            return;
        }
        match verb {
            "pin-tab" => self.perform_shortcut("toggle_pin", Value::Null),
            "downloads" => self.open_settings_section("downloads", None),
            "history" => self.open_settings_section("history", None),
            "settings" => {
                self.open_settings();
            }
            "toggle-rail" => {
                self.settings.rail_collapsed = !self.settings.rail_collapsed;
                self.save_settings();
                self.apply_layout();
            }
            "close-window" => {
                if let Some(p) = self.proxy.as_ref() {
                    let _ = p.send_event(AppMessage::Quit);
                }
            }
            other => self.note(format!("unknown command verb: {other}")),
        }
        self.emit("shell.command_done", json!({ "verb": verb }));
    }

    fn perform_shortcut(&mut self, name: &str, args: Value) {
        match name {
            "new_tab" => {
                self.create_tab(tabs::NEWTAB_URL, self.active, true);
            }
            "close_tab" => {
                if let Some(id) = self.active {
                    self.close_tab(id);
                }
            }
            "close_window" => {
                if let Some(p) = self.proxy.as_ref() {
                    let _ = p.send_event(AppMessage::Quit);
                }
            }
            "next_tab" | "rapid_next_tab" => self.step_tab(1),
            "prev_tab" | "rapid_prev_tab" => self.step_tab(-1),
            "tab_index" => {
                let n = args.as_u64().unwrap_or(0);
                let target = if n >= 9 {
                    self.tabs.last()
                } else {
                    self.tabs.at_index(n.saturating_sub(1) as usize)
                };
                if let Some(t) = target {
                    self.activate(t);
                }
            }
            "reopen_closed" => {
                let req = Request { id: None, cmd: "tab.reopen_closed".into(), payload: json!({}) };
                let _ = self.run(&req);
            }
            "focus_omnibox" => {
                self.close_overlay();
                self.emit("shell.focus_changed", json!({ "scope": "omnibox", "select_all": true }));
            }
            "cycle_focus" => self.emit("shell.focus_changed", json!({ "scope": "cycle" })),
            "open_palette" => self.open_palette(),
            "ask_ai" => self.ai_open_ask(),
            "close_palette" => self.close_overlay(),
            "find" => {
                self.close_overlay();
                if let Some(shell) = self.shell.as_ref() { let _ = shell.focus(); }
                self.emit("shell.focus_changed", json!({ "scope": "find" }));
            }
            "find_next" | "find_prev" => {
                self.emit("shell.find_action", json!({ "action": name }));
            }
            "open_downloads" => self.open_settings_section("downloads", None),
            "open_history" | "open_history_today" => self.open_settings_section("history", None),
            "page_back" | "page_forward" => {
                if let Some(id) = self.active {
                    if let Some(live) = self.live.get(&id) {
                        let r = if name == "page_back" {
                            live.webview.go_back()
                        } else {
                            live.webview.go_forward()
                        };
                        if let Err(e) = r {
                            self.note(format!("{name}: {e}"));
                        }
                    }
                }
            }
            "home" => {
                if let Some(id) = self.active {
                    self.navigate(id, tabs::NEWTAB_URL, false);
                }
            }
            "reload" | "reload_ignoring_cache" => {
                if let Some(id) = self.active {
                    if let Some(live) = self.live.get(&id) {
                        if let Err(e) = live.webview.reload() {
                            self.note(format!("reload: {e}"));
                        }
                    }
                }
            }
            "bookmark" => {
                let req = Request { id: None, cmd: "tab.bookmark".into(), payload: json!({}) };
                let _ = self.run(&req);
            }
            "bookmark_all_tabs" => {
                let req = Request { id: None, cmd: "tab.bookmark_all".into(), payload: json!({}) };
                let _ = self.run(&req);
            }
            "view_source" => {
                if let Some(id) = self.active {
                    self.navigate(id, ui::internal_page(ui::InternalPage::ViewSourceUnsupported), false);
                }
            }
            "zoom_in" | "zoom_out" | "zoom_reset" => {
                let Some(id) = self.active else { return };
                self.zoom_tab(id, name);
            }
            "open_settings" => {
                self.open_settings();
            }
            "escape" => {
                self.emit("shell.escape", json!({}));
            }
            "item_menu" => self.emit("shell.item_menu", json!({})),
            "toggle_rail" => {
                self.settings.rail_collapsed = !self.settings.rail_collapsed;
                self.save_settings();
                self.apply_layout();
            }
            "toggle_pin" => {
                if let Some(id) = self.active {
                    if let Some(t) = self.tabs.get_mut(id) {
                        t.pinned = !t.pinned;
                    }
                    self.emit_tabs();
                }
            }
            "close_tab_or_word" => self.emit("shell.ctrl_w", json!({})),
            other => self.note(format!("unhandled shortcut: {other}")),
        }
    }

    fn self_test(&mut self) -> Value {
        let pid = sys::current_pid();
        let tree = sys::process_tree(pid);
        let total: u64 = tree.iter().map(|p| p.working_set).sum();
        let report = json!({
            "startup_ms": elapsed_ms(self.start_tick),
            "shell_ready_ms": self.shell_ready_ms,
            "tabs": self.tabs.len(),
            "process_count": tree.len(),
            "process_list": tree,
            "rss_mb": (sys::mb(total) * 10.0).round() / 10.0,
            "egress_log": self.egress,
            "log": self.log,
            "webview2_version": webview2_version(),
            "engine_name": platform::ENGINE_NAME,
            "engine_version": webview_version(),
            "find_supported": true,
        });
        self.emit("diagnostics.report", report.clone());
        report
    }

    // ---- driver: scripted scenarios against the real app -----------------
    //
    // `--driver=NAME` runs a fixed command script through the same `dispatch`
    // path the shell uses, sampling state between steps. It exists so leak and
    // feature behaviour can be measured on the shipping binary instead of being
    // argued from the source. It is inert unless the flag is passed.

    fn driver_snapshot(&self) -> Value {
        json!({
            "step": self.driver_step,
            "tabs": self.tabs.len(),
            "live_webviews": self.live.len(),
            "groups": self.groups.len(),
            "active": self.active.map(|a| a.0),
            "downloads": self.downloads.len(),
            "egress": self.egress.len(),
            "log_lines": self.log.len(),
            "rss_mb": (sys::mb(sys::process_tree(sys::current_pid()).iter().map(|p| p.working_set).sum()) * 10.0).round() / 10.0,
        })
    }

    /// One dispatch of a driver command, returning the reply's ok flag.
    fn driver_call(&mut self, cmd: &str, payload: Value) -> bool {
        let raw = json!({ "id": 0, "cmd": cmd, "payload": payload }).to_string();
        let reply = self.dispatch(Surface::Shell, &raw);
        let Some(reply) = reply else { return true };
        let text = serde_json::to_string(&reply).unwrap_or_default();
        let ok = text.contains("\"ok\":true");
        if !ok {
            self.note(format!("DRIVER {cmd} -> {text}"));
        }
        ok
    }

    fn driver_script(&self) -> Vec<(&'static str, Value)> {
        let u = |s: &str| json!({ "url": s, "disposition": "new_tab_background" });
        let page = "https://example.com/";
        let w = "https://en.wikipedia.org/wiki/Rust_(programming_language)";
        match self.driver_name.as_str() {
            // R0a's proof harness (`--proof[=N][+flags]`): samples the process tree
            // once per step and opens N distinct-origin tabs. All logic is in
            // `ai::proof`; this arm only routes the scenario name to it and hands over
            // the numbers the app owns — tabs, live page webviews, whether the overlay
            // exists, and the committed-navigation counter, none of which may be
            // inferred from the process list.
            name if name.starts_with("proof") => crate::ai::proof::drive(
                name,
                &crate::ai::proof::Observed {
                    tabs: self.tabs.len(),
                    live_webviews: self.live.len(),
                    overlay_live: self.overlay.is_some(),
                    navigations: self.navigation_seq,
                },
            ),
            // 40 tabs open/close rounds: the leak test that matters.
            "churn" => {
                let mut v = Vec::new();
                for i in 0..40 {
                    let url = if i % 2 == 0 { page } else { w };
                    // "new_tab" activates, so a webview is really created and
                    // then destroyed by the close: this is the leak test.
                    v.push(("nav.open", json!({ "url": url, "disposition": "new_tab" })));
                    // Omit tab_id: both commands fall back to the active tab.
                    v.push(("tab.close", json!({})));
                    v.push(("tab.select", json!({})));
                }
                v
            }
            // Open N tabs *and show each one*, so every page really loads. A
            // background tab has no webview (tab sleeping), so opening without
            // activating would measure nothing at all.
            "navloop" => {
                let mut v = Vec::new();
                for i in 0..30u64 {
                    v.push(("nav.open", json!({ "url": if i % 2 == 0 { page } else { w }, "disposition": "new_tab" })));
                }
                v
            }
            // Sleep/restore: open tabs, then activate a slept one. Proves a
            // slept tab rebuilds its webview and re-navigates. Also the source
            // of the tab-switch latency figures (per-command `us`).
            "restore" => {
                let mut v = Vec::new();
                // Tab 1 exists; ids are allocated 1,2,3,... so the nth opened
                // tab has id n+1. Open 5, activating each (which sleeps the
                // previous one), then go back and forth to force rebuilds.
                for i in 0..5 {
                    v.push(("nav.open", u(if i % 2 == 0 { page } else { w })));
                    v.push(("tab.select", json!({ "tab_id": i + 2 })));
                }
                // Ping-pong between the last two: each hop is a slept-tab
                // restore (webview destroyed on the way out).
                for _ in 0..4 {
                    v.push(("tab.select", json!({ "tab_id": 6 })));
                    v.push(("tab.select", json!({ "tab_id": 5 })));
                }
                v
            }
            // Every omnibox query shape through the real classifier + ranker.
            "omnibox" => { vec![
                ("omnibox.query", json!({ "text": "" })),
                ("omnibox.query", json!({ "text": "exa" })),
                ("omnibox.query", json!({ "text": "example.com" })),
                ("omnibox.query", json!({ "text": "localhost:8080" })),
                ("omnibox.query", json!({ "text": "[::1]:8080" })),
                ("omnibox.query", json!({ "text": "HTTP://EXAMPLE.COM/Path?q=1#f" })),
                ("omnibox.query", json!({ "text": "rust programming" })),
                ("omnibox.query", json!({ "text": ">" })),
                ("omnibox.query", json!({ "text": "> new" })),
                ("omnibox.query", json!({ "text": ">set zoom 120" })),
                ("omnibox.query", json!({ "text": "  spaces  " })),
                ("omnibox.query", json!({ "text": "!!!???" })),
                ("omnibox.query", json!({ "text": "a" })),
                ("omnibox.query", json!({ "text": "file:///C:/Windows/win.ini" })),
                ("omnibox.query", json!({ "text": "about:blank" })),
            ]}
            // History/bookmark/download/settings operations, then a reload.
            "state" => { vec![
                ("bookmark.toggle", json!({})),
                ("bookmark.toggle", json!({})),
                ("history.query", json!({ "text": "" })),
                ("history.query", json!({ "text": "example" })),
                ("history.query", json!({ "text": "%_%" })),
                ("settings.update", json!({ "key": "zoom", "value": 125 })),
                ("settings.update", json!({ "key": "rail_collapsed", "value": true })),
                ("find.start", json!({})),
                ("find.query", json!({ "text": "example" })),
                ("find.next", json!({})),
                ("find.stop", json!({})),
            ]}
            _ => Vec::new(),
        }
    }

    /// Run ONE script step, recording a snapshot. Returns the settle window
    /// before the next step, and whether the script is finished.
    ///
    /// Driving the whole script in a single event-loop iteration measured
    /// nothing: navigations commit asynchronously, so a tight loop finished 30
    /// "steps" in 9 ms with no page ever loading. Each step gets a settle window.
    fn driver_step(&mut self) -> (Duration, bool) {
        let script = self.driver_script();
        self.driver_total = script.len() as u32;
        let i = self.driver_step as usize;
        let Some((cmd, payload)) = script.get(i).cloned() else {
            return (Duration::ZERO, true);
        };
        let t0 = Instant::now();
        // The harness's wait step carries its own duration, and `driver_call`
        // consumes the payload, so read it here: for that step the settle window
        // *is* the wait, and it is the only thing that happens.
        let wait_ms = if cmd == crate::ai::proof::WAIT_STEP {
            payload
                .get("ms")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(crate::ai::proof::MAX_WAIT_MS)
        } else {
            0
        };
        let ok = self.driver_call(cmd, payload);
        let dur_us = t0.elapsed().as_micros() as u64;
        let mut s = self.driver_snapshot();
        s["command"] = json!(cmd);
        s["ok"] = json!(ok);
        s["us"] = json!(dur_us);
        if let Some(f) = self.driver_snaps.as_mut() {
            f.push(s);
        }
        // A navigation needs time to commit and reach the history write; a pure
        // command needs almost none. The latency figures quoted for tab restore
        // come from `us`, which excludes this window.
        let settle = match cmd {
            // Before the `proof.` prefix arm: this step's payload says how long the
            // observation window is, and it is minutes rather than milliseconds.
            c if c == crate::ai::proof::WAIT_STEP => Duration::from_millis(wait_ms),
            c if c.starts_with("nav.") || c == "tab.select" => Duration::from_millis(900),
            // A measurement step: the overlay arm creates a whole WebView2 controller
            // and its first navigation, where the startup log shows roughly 400 ms for
            // the webview plus whatever the page takes. The 900 ms navigation window is
            // not enough for a fresh controller, and sampling too early would attribute
            // a late-arriving renderer to the next step.
            c if c.starts_with(crate::ai::proof::OVERLAY_STEP_PREFIX) => Duration::from_millis(2_500),
            _ => Duration::from_millis(30),
        };
        self.driver_step += 1;
        (settle, false)
    }

    fn driver_report(&mut self) -> Value {
        json!({
            "scenario": self.driver_name,
            "steps": self.driver_total,
            "shell_ready_ms": self.shell_ready_ms,
            "final_tabs": self.tabs.len(),
            "final_live_webviews": self.live.len(),
            "snapshots": self.driver_snaps.take().unwrap_or_default(),
            "log": self.log,
        })
    }

    fn omnibox_results(&mut self, text: &str, limit: usize) -> Value {
        let intent = omnibox::classify(text);
        let mode = intent.mode();
        let trimmed = text.trim();
        let mut rows: Vec<Value> = Vec::new();
        let tpl = self.settings.search_template.clone();

        match &intent {
            Intent::Command { .. } => {
                let needle = trimmed.trim_start_matches("> ").trim().to_ascii_lowercase();
                for c in commands_catalog() {
                    if needle.is_empty()
                        || format!("{} {}", c.verb, c.label).to_ascii_lowercase().contains(&needle)
                    {
                        rows.push(json!({
                            "kind": "command",
                            "verb": c.verb,
                            // `app.js` `completeSelection` writes `r.url || r.title`
                            // into the omnibox and `activateRow` hands `r.verb`
                            // to `omnibox.activate`. A display string like
                            // `verb — description` is not a command the host
                            // recognises, so Tab-complete would turn it into a
                            // web search. `title` has to be typeable.
                            "title": if c.args.is_empty() {
                                c.verb.to_string()
                            } else {
                                format!("{} {}", c.verb, c.args)
                            },
                            "hint": c.label,
                        }));
                    }
                }
                for t in self.tabs.tabs() {
                    let title = t.display_title();
                    if needle.is_empty() || title.to_ascii_lowercase().contains(&needle) {
                        rows.push(json!({ "kind": "tab", "tab_id": t.id.0, "title": title, "hint": t.url }));
                    }
                }
            }
            Intent::Find(q) => rows.push(json!({ "kind": "find", "title": format!("Find “{q}” in page") })),
            Intent::Ask { question, .. } => {
                let label = if question.is_empty() { "River".to_string() } else { format!("River: {question}") };
                rows.push(json!({ "kind": "ask", "title": trimmed, "hint": label }));
            }
            Intent::Url(url) => {
                rows.push(json!({ "kind": "url", "url": url, "title": format!("Go to {url}") }));
                rows.push(json!({
                    "kind": "search",
                    "url": omnibox::search_url(&tpl, trimmed),
                    "title": format!("Search for “{trimmed}”"),
                }));
            }
            Intent::Search(q) => {
                if !q.is_empty() {
                    rows.push(json!({
                        "kind": "search",
                        "url": omnibox::search_url(&tpl, q),
                        "title": format!("Search for “{q}”"),
                    }));
                }
            }
        }

        json!({ "mode": mode, "chip": mode.chip_label(), "rows": rows.into_iter().take(limit).collect::<Vec<_>>() })
    }

    // ---- 5. webviews ----------------------------------------------------

    pub(crate) fn build_webview(&mut self, id: TabId) {
        if self.live.contains_key(&id) {
            return;
        }
        let Some(tab) = self.tabs.get(id).cloned() else { return };
        let Some(window) = self.window.as_ref() else {
            self.note("build_webview: no window");
            return;
        };
        let (geom, canvas_visible) = self.tab_geom(id);
        let visible = canvas_visible.unwrap_or(self.active == Some(id));
        let initial = self.resolve_url(&tab.url);
        let pdf = crate::pdf::PdfSession::default();
        let proxy_title = self.proxy();
        let proxy_nav = self.proxy();
        let proxy_dl = self.proxy();
        let proxy_probe = self.proxy();

        // wry needs `&mut WebContext` for the duration of the build; take it out
        // of the state so no other borrow of `self` is live across the call.
        let mut context = match self.context.take() {
            Some(c) => c,
            None => {
                let dir = platform::context_dir(&self.data_dir);
                WebContext::new(Some(dir))
            }
        };
        // `WebViewId<'a>` borrows the id string for the life of the build.
        let webview_id = format!("tab-{}", id.0);
        let builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_id(&webview_id)
            .with_bounds(layout::rect(geom.x, geom.y, geom.w, geom.h))
            .with_visible(visible)
            .with_focused(visible)
            .with_background_color((255, 255, 255, 255))
            // Browser accelerators are off so Ctrl+F/Ctrl+P/Ctrl+R never fire a
            // WebView2 action behind our back; the process message hook in
            // `main` delivers ours. Ctrl+W/Ctrl+T are not WebView2 accelerators.
            .with_url(&initial)
            // Page reports are untrusted and cannot reach the command dispatcher.
            // Only the bundled Settings document may send settings changes.
            .with_initialization_script(include_str!("../ui/probe.js"))
            .with_initialization_script(include_str!("../ui/pdf-drop.js"))
            .with_custom_protocol("asset".into(), asset_handler())
            .with_ipc_handler(move |req: HttpRequest<String>| {
                let _ = proxy_probe.send_event(AppMessage::PageProbe(id, req.body().clone()));
            })
            .with_asynchronous_custom_protocol("pdf".into(), {
                let proxy = self.proxy();
                move |_id, req, responder| {
                    if req.uri().path() == "/data" {
                        let _ = proxy.send_event(AppMessage::PdfRequest(id, req, responder));
                    } else {
                        crate::pdf::serve(req, responder);
                    }
                }
            })
            .with_on_page_load_handler({
                let proxy = self.proxy();
                move |event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        let _ = proxy.send_event(AppMessage::PageLoaded(id));
                    }
                }
            })
            .with_navigation_handler({
                let proxy_pdf = self.proxy();
                move |url| {
                    if cfg!(target_os = "macos") && crate::pdf::is_pdf_url(&url) {
                        let _ = proxy_pdf.send_event(AppMessage::OpenPdf(id, url));
                        return false;
                    }
                    let _ = proxy_nav.send_event(AppMessage::TabNavigated(id, url.clone()));
                    !url.starts_with("view-source:")
                }
            })
            .with_new_window_req_handler({
                let proxy = proxy_dl.clone();
                move |url, _features| {
                    let _ = proxy.send_event(AppMessage::NewWindow(id, url));
                    wry::NewWindowResponse::Deny
                }
            })
            .with_document_title_changed_handler(move |title| {
                let _ = proxy_title.send_event(AppMessage::TabTitle(id, title));
            })
            .with_download_started_handler({
                let proxy = proxy_dl.clone();
                #[cfg(target_os = "macos")]
                let destination = self.download_destination.clone();
                move |url, path| {
                    // WKWebView's completion callback omits its path. Reserve and pass the
                    // actual destination at start so both the native download and UI agree.
                    #[cfg(target_os = "macos")]
                    {
                        match platform::macos::download_destination(path, &destination.borrow()) {
                            Ok(chosen) => *path = chosen,
                            Err(_) => return false,
                        }
                    }
                    let _ = proxy.send_event(AppMessage::DownloadStarted(id, url, path.clone()));
                    true
                }
            })
            .with_download_completed_handler({
                let proxy = proxy_dl.clone();
                move |url, path, ok| {
                    let _ = proxy.send_event(AppMessage::DownloadFinished(url, path, ok));
                }
            });

        let builder = platform::configure(builder, self.environment.as_ref(), false);
        let built = builder.build_as_child(window);
        // Put the shared environment back regardless of the outcome so the next
        // tab joins the same WebView2 browser process (W10).
        self.context = Some(context);

        match built {
            Ok(wv) => {
                #[cfg(windows)]
                {
                    let proxy = self.proxy();
                    if let Err(e) = crate::webview2::install_pdf_drop(&wv, move |url| {
                        let _ = proxy.send_event(AppMessage::DroppedPdf(id, url));
                    }) {
                        self.note(format!("PDF drop handler: {e}"));
                    }
                    let proxy = self.proxy();
                    if let Err(e) = crate::webview2::install_zoom_changed(&wv, move |factor| {
                        let _ = proxy.send_event(AppMessage::RuntimeZoom(id, factor));
                    }) {
                        self.note(format!("Native zoom handler: {e}"));
                    }
                    let proxy = self.proxy();
                    if let Err(e) = crate::webview2::install_pdf_detection(&wv, move |url| {
                        let _ = proxy.send_event(AppMessage::PdfDetected(id, url));
                    }) {
                        self.note(format!("Native PDF detection: {e}"));
                    }
                }
                if let Err(e) = wv.set_bounds(layout::rect(geom.x, geom.y, geom.w, geom.h)) {
                    self.note(format!("tab bounds: {e}"));
                }
                self.live.insert(id, LiveTab {
                    webview: wv, pdf, pending_scroll: (tab.scroll_y > 0.0).then_some(tab.scroll_y),
                    #[cfg(windows)]
                    native_pdf: None,
                    last_used: Instant::now(), lifecycle_known: false, editing: false, playing: false,
                    shaped: Default::default(),
                });
                self.shape_tab(id);
                if let Some(t) = self.tabs.get_mut(id) {
                    t.sleeping = false;
                }
                self.note(format!("webview for tab {} -> {initial}", id.0));
            }
            Err(e) => {
                self.note(format!("webview create failed for tab {}: {e}", id.0));
                self.emit(
                    "nav.failed",
                    json!({ "tab_id": id.0, "code": "WEBVIEW_CREATE", "url": initial, "navigation_id": self.navigation_seq }),
                );
            }
        }
    }
}

fn panel_str(p: Panel) -> &'static str {
    match p {
        Panel::None => "none",
        Panel::Downloads => "downloads",
        Panel::History => "history",
        Panel::Ai => "ai",
    }
}

/// Should a committed navigation become a history row?
///
/// Only real web content: `http(s)` and `ftp`. Internal `asset://`/`native://`
/// pages, `about:blank` (WebView2's initial document), `data:` and
/// `view-source:` would otherwise fill history with pages that cannot be
/// revisited, and an empty URL (the title-only update path) must never write a
/// row.
fn should_record_history(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    ["http://", "https://", "ftp://"].iter().any(|p| lower.starts_with(p))
}

/// Do two URLs refer to the same page for the purpose of consuming a pending
/// "the user asked for this" mark? Redirects rewrite the URL, so an exact
/// compare would mis-attribute them; the fragment is never sent to a server, so
/// it is not a difference here either.
fn same_page(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.split('#').next().unwrap_or(s);
        s.trim_end_matches('/').to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Alias the rest of the module uses for the runtime version.
fn webview2_version() -> Option<String> {
    webview_version()
}

fn webview_version() -> Option<String> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| wry::webview_version().ok()).clone()
}

struct CommandSpec {
    verb: &'static str,
    /// Arguments appended after the verb, so `title` is a typeable omnibox
    /// string. Empty for verbs that take none.
    args: &'static str,
    label: &'static str,
}

fn commands_catalog() -> &'static [CommandSpec] {
    &[
        CommandSpec { verb: "new-tab", args: "", label: "open a new tab" },
        CommandSpec { verb: "new-canvas", args: "", label: "open a canvas: a grid of up to 6 pages" },
        CommandSpec { verb: "canvas-zoom", args: "", label: "zoom the focused canvas page, or back to the grid" },
        CommandSpec { verb: "close-tab", args: "", label: "close the current tab" },
        CommandSpec { verb: "reopen-tab", args: "", label: "reopen the last closed tab" },
        CommandSpec { verb: "pin-tab", args: "", label: "pin or unpin the current tab" },
        CommandSpec { verb: "bookmark", args: "", label: "bookmark the current page" },
        CommandSpec { verb: "downloads", args: "", label: "show downloads" },
        CommandSpec { verb: "history", args: "", label: "show history" },
        CommandSpec { verb: "settings", args: "", label: "open settings" },
        CommandSpec { verb: "toggle-rail", args: "", label: "collapse or expand the tab rail" },
        CommandSpec { verb: "reload", args: "", label: "reload the page" },
        CommandSpec { verb: "zoom-reset", args: "", label: "reset page zoom" },
        CommandSpec { verb: "close-window", args: "", label: "close the window" },
    ]
}

// ---------------------------------------------------------------------------
// 6. Startup + event loop
// ---------------------------------------------------------------------------

/// `%LOCALAPPDATA%\native-browser`.
pub fn data_dir() -> PathBuf {
    platform::data_dir()
}

fn asset_handler() -> impl Fn(&str, HttpRequest<Vec<u8>>) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> + Send + Sync + 'static {
    move |_id: &str, req| ui::serve_embedded(&req)
}

fn is_asset_origin(url: &str) -> bool {
    url.starts_with("asset://")
        // wry maps `asset://host/path` to `https://asset.host/path` on Windows
        // when `with_https_scheme(true)` is set.
        || url.starts_with("https://asset.")
        || url == "about:blank"
}

/// Entry point called from `main`. Returns `!` because tao owns the thread.
///
/// Ordering constraint: `with_msg_hook` is a builder-time setting on the
/// `EventLoopBuilder`, and the hook needs the `AppState` `Rc`. So the state is
/// built first (it needs no window), then the loop is built with the hook
/// closing over that `Rc`, then the window is created against the loop target.
pub fn run(start_tick: u64) -> ! {
    run_with(start_tick, None, None)
}

/// `self_test_url`: when `Some`, load this URL in the one tab, wait for the
/// navigation to commit, write the diagnostics report, and exit. Used for the
/// reproducible startup/process measurements reported to the delivery lead.
pub fn run_with(start_tick: u64, self_test_url: Option<String>, driver: Option<String>) -> ! {
    let dir = data_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        platform::show_fatal(&format!("Cannot create {}\n\n{e}", dir.display()));
        std::process::exit(2);
    }
    let (settings, settings_warn) = settings::load(&dir.join("settings.json"));
    let store = match Store::open(&dir.join("data.db"), settings.history_cap) {
        Ok(s) => s,
        Err(e) => {
            platform::show_fatal(&format!(
                "Cannot open the history database in {}\n\n{e}",
                dir.display()
            ));
            std::process::exit(3);
        }
    };
    let (saved, session_warn) = session::load(&dir.join("session.json"));

    let mut state = AppState {
        data_dir: dir.clone(),
        store,
        #[cfg(target_os = "macos")]
        download_destination: Rc::new(RefCell::new(PathBuf::from(
            if settings.download_dir.trim().is_empty() {
                settings::default_download_dir()
            } else {
                settings.download_dir.clone()
            }
        ))),
        settings,
        settings_page: settings_page::State::default(),
        tabs: TabSet::new(),
        groups: Vec::new(),
        active: None,
        canvas_shown: None,
        live: HashMap::new(),
        downloads: Vec::new(),
        next_download_id: 0,
        download_owners: HashMap::new(),
        last_tab_prune: Instant::now(),
        shutting_down: false,
        shell: None,
        overlay: None,
        sidebar: None,
        window: None,
        window_id: None,
        environment: None,
        context: None,
        panel: Panel::None,
        overlay_rect: Overlay::Closed,
        menu_open: false,
        find: None,
        find_seq: 0,
        omnibox_worker: None,
        proxy: None,
        start_tick,
        shell_ready_ms: None,
        dpi_scale: 1.0,
        system_dark: false,
        #[cfg(windows)]
        shell_focus_in_input: false,
        #[cfg(windows)]
        shell_hwnd: 0,
        session_dirty_at: None,
        last_nav_emit: HashMap::new(),
        typed_target: None,
        navigation_seq: 0,
        log: VecDeque::new(),
        egress: VecDeque::new(),
        self_test_url_pending: false,
        driver_name: driver.clone().unwrap_or_default(),
        driver_step: 0,
        driver_total: 0,
        driver_snaps: None,
        ai: crate::ai::session::AiSession::new(),
    };
    if let Some(w) = settings_warn {
        state.note(w);
    }
    if let Some(w) = session_warn {
        state.note(w);
    }

    let mut restored_window = WindowRect::default();
    if let Some(sess) = saved {
        state.settings.rail_collapsed = sess.rail_collapsed;
        restored_window = sess.window.clone();
        for g in &sess.groups {
            state.tabs.observe_group_id(GroupId(g.id));
            state.groups.push(Group {
                id: GroupId(g.id),
                name: g.name.clone(),
                color: g.color,
                collapsed: g.collapsed,
            });
        }
        for t in &sess.tabs {
            state.tabs.observe_id(TabId(t.id));
            let mut tab = Tab::new(TabId(t.id), &t.url);
            tab.title = t.title.clone();
            tab.pinned = t.pinned;
            tab.group_id = t.group_id.map(GroupId);
            tab.zoom = t.zoom;
            tab.scroll_y = t.scroll_y;
            tab.canvas = t.canvas.clone();
            tab.member_of = t.member_of.map(TabId);
            state.tabs.insert_at(state.tabs.len(), tab);
        }
        state.active = sess.active_tab_id.map(TabId);
        state.canvas_sanitize();
        state.downloads = sess.downloads;
        // A restored record still marked `in_progress` is not downloading — the
        // process that owned it is gone (§6.4: status is non-authoritative).
        for d in &mut state.downloads {
            if d.state == DownloadState::InProgress {
                d.state = DownloadState::Failed;
            }
        }
        trim_downloads(&mut state.downloads);
        state.next_download_id = state.downloads.iter().map(|d| d.id).max().unwrap_or(0);
        state.note(format!(
            "session: {} tabs, {} groups, active {:?}",
            state.tabs.len(),
            state.groups.len(),
            state.active.map(|a| a.0)
        ));
    }
    if state.tabs.is_empty() {
        let id = state.tabs.alloc_id();
        state.tabs.insert_after(None, Tab::new(id, tabs::NEWTAB_URL));
        state.active = Some(id);
    }

    let shared: Shared = Rc::new(RefCell::new(state));

    // ---- event loop with the process message hook ----------------------
    #[cfg(windows)]
    let hook_state = shared.clone();
    let mut builder = EventLoopBuilder::<AppMessage>::with_user_event();
    #[cfg(windows)]
    {
        use tao::platform::windows::EventLoopBuilderExtWindows;
        builder.with_msg_hook(move |msg| dispatch_accelerator(&hook_state, msg));
    }
    let event_loop = builder.build();
    let proxy = event_loop.create_proxy();
    shared.borrow_mut().proxy = Some(proxy.clone());
    shared.borrow_mut().omnibox_worker = Some(omnibox_worker::start(dir.join("data.db"), proxy.clone()));

    // ---- window ---------------------------------------------------------
    let window = {
        let mut b = WindowBuilder::new()
            .with_title("native-browser")
            .with_inner_size(tao::dpi::LogicalSize::new(
                restored_window.width.max(640) as f64,
                restored_window.height.max(480) as f64,
            ))
            .with_min_inner_size(tao::dpi::LogicalSize::new(640.0, 480.0))
            .with_decorations(false)
            .with_visible(false);
        #[cfg(windows)]
        {
            use tao::platform::windows::{IconExtWindows, WindowBuilderExtWindows};
            let icon = |size| tao::window::Icon::from_resource(
                1, Some(tao::dpi::PhysicalSize::new(size, size)),
            ).expect("cannot load the browser application icon");
            b = b.with_window_icon(Some(icon(32))).with_taskbar_icon(Some(icon(256)));
        }
        if restored_window.x != 0 || restored_window.y != 0 {
            b = b.with_position(tao::dpi::PhysicalPosition::new(restored_window.x, restored_window.y));
        }
        b.build(&event_loop).unwrap_or_else(|e| {
            platform::show_fatal(&format!("Cannot create the window: {e}"));
            std::process::exit(4);
        })
    };
    if restored_window.maximized {
        window.set_maximized(true);
    }
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        window.set_undecorated_shadow(true);
    }

    {
        let mut s = shared.borrow_mut();
        s.dpi_scale = window.scale_factor();
        s.window_id = Some(window.id());
        s.window = Some(window);
        s.context = Some(WebContext::new(Some(platform::context_dir(&s.data_dir))));
    }

    // One WebView2 environment for the whole app. Created after the event loop
    // exists (COM is initialised) and before any webview is built. Failure is
    // non-fatal: wry then falls back to one environment per webview, which
    // costs processes but still works.
    {
        let dir = platform::context_dir(&shared.borrow().data_dir);
        match platform::create_shared_environment(&dir) {
            Ok(env) => shared.borrow_mut().environment = Some(env),
            Err(e) => {
                #[cfg(target_os = "macos")]
                {
                    platform::show_fatal(&format!("Cannot open the WebKit profile: {e}"));
                    std::process::exit(4);
                }
                #[cfg(windows)]
                shared.borrow_mut().note(format!("shared WebView2 environment unavailable: {e}"));
            }
        }
    }

    build_shell(&shared);
    if crate::ai::mode::overlay_at_startup() {
        build_overlay(&shared);
    } else {
        // `--proof=…+no_overlay`: the harness measures the overlay as a delta, so it
        // must not be present in the baseline sample. The command that builds it is
        // gated on `mode::active()`, which is what makes this branch unreachable in
        // ordinary browsing.
        shared
            .borrow_mut()
            .note("overlay deferred (measurement mode); built by the harness step");
    }

    {
        let mut s = shared.borrow_mut();
        let ready_ms = elapsed_ms(s.start_tick);
        s.note(format!("window + shell created at {ready_ms} ms"));
        s.apply_layout();
        if let Some(w) = s.window.as_ref() {
            w.set_visible(true);
        }
    }

    // Mouse watcher: reports left-button presses with their screen position.
    #[cfg(target_os = "macos")]
    let native_events = {
        let s = shared.borrow();
        platform::macos::install_events(s.window.as_ref().expect("window installed"), proxy.clone())
            .unwrap_or_else(|e| {
                platform::show_fatal(&e);
                std::process::exit(4);
            })
    };
    #[cfg(windows)]
    let mouse_proxy = proxy.clone();
    #[cfg(windows)]
    std::thread::spawn(move || {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut was_down = false;
        loop {
            std::thread::sleep(Duration::from_millis(10));
            // Bit 0 ("pressed since the last call") is not reliable on its own, so a
            // press is also detected as an up-to-down edge of the current state.
            let s = unsafe { GetAsyncKeyState(0x01) } as u16;
            let down = s & 0x8000 != 0;
            let pressed = (down && !was_down) || s & 1 != 0;
            was_down = down;
            if pressed {
                let mut p = POINT::default();
                if unsafe { GetCursorPos(&mut p) }.is_ok() && mouse_proxy.send_event(AppMessage::MouseDown(p.x, p.y)).is_err() {
                    break;
                }
            }
        }
    });

    // Debounce thread: only wakes the loop.
    let tick_proxy = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        if tick_proxy.send_event(AppMessage::Flush).is_err() {
            break;
        }
    });

    // Driver: scheduled from the `shell.ready` handler (see `run`), which is the
    // first point at which a scripted command behaves exactly as in a session.

    // Self-test driver: navigate once the shell is up, then report and exit.
    if let Some(url) = self_test_url.clone() {
        // `native://newtab` is already the startup page; report on it directly.
        let report_now = url == tabs::NEWTAB_URL;
        let st = proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2_500));
            if !report_now {
                let _ = st.send_event(AppMessage::SelfTestStart(url));
            }
            // Hard deadline: report even if the page never commits.
            std::thread::sleep(Duration::from_secs(18));
            let _ = st.send_event(AppMessage::SelfTestNavigated);
        });
    }

    let window_id = shared.borrow().window_id;

    event_loop.run(move |event, _target, control_flow| {
        #[cfg(target_os = "macos")]
        let _keep_native_events = &native_events;
        *control_flow = ControlFlow::WaitUntil(Instant::now() + TICK);
        match event {
            Event::WindowEvent { window_id: id, event, .. } if Some(id) == window_id => {
                handle_window_event(&shared, event, control_flow)
            }
            Event::UserEvent(AppMessage::Ipc(source, raw)) => {
                let reply = {
                    let mut s = shared.borrow_mut();
                    s.dispatch(source, &raw)
                };
                if let Some(reply) = reply {
                    shared.borrow_mut().reply(source, reply);
                }
            }
            Event::UserEvent(AppMessage::TabTitle(id, title)) => {
                let mut s = shared.borrow_mut();
                s.on_title(id, title);
            }
            Event::UserEvent(AppMessage::NewWindow(opener, url)) => {
                let mut s = shared.borrow_mut();
                if url.starts_with("http://") || url.starts_with("https://") {
                    s.open_from_page(opener, &url);
                }
            }
            Event::UserEvent(AppMessage::OpenPdf(id, url)) => {
                shared.borrow_mut().navigate(id, &url, false);
            }
            #[cfg(windows)]
            Event::UserEvent(AppMessage::DroppedPdf(id, url)) => {
                let mut s = shared.borrow_mut();
                if s.live.contains_key(&id) {
                    s.activate(id);
                    s.navigate(id, &url, false);
                }
            }
            #[cfg(windows)]
            Event::UserEvent(AppMessage::NativePdfSelection(tab, generation, text, rect)) => {
                let mut s = shared.borrow_mut();
                if s.live.get(&tab).and_then(|live| live.native_pdf.as_ref()).is_some_and(|pdf| pdf.id == generation) {
                    s.ai_on_selection(tab, crate::ai::session::Selection { text, rect });
                }
            }
            #[cfg(windows)]
            Event::UserEvent(AppMessage::RuntimeZoom(id, factor)) => {
                let mut s = shared.borrow_mut();
                if s.has_native_pdf(id) {
                    let clamped = factor.clamp(0.25, 5.0);
                    if clamped != factor { s.apply_zoom(id, clamped); }
                    let factor = clamped;
                    s.canvas_focus_tab(id);
                    if let Some(tab) = s.tabs.get_mut(id) { tab.zoom = factor; }
                    s.emit("tab.zoom_changed", json!({ "tab_id": id.0, "factor": factor }));
                }
            }
            #[cfg(windows)]
            Event::UserEvent(AppMessage::PdfDetected(id, url)) => {
                shared.borrow_mut().start_native_pdf(id, &url);
            }
            Event::UserEvent(AppMessage::TabNavigated(id, url)) => {
                let mut s = shared.borrow_mut();
                s.on_navigation(id, &url);
            }
            Event::UserEvent(AppMessage::AiFrame(generation, frame)) => {
                let mut s = shared.borrow_mut();
                if s.ai.link_generation == generation { s.ai_on_frame(frame); }
            }
            Event::UserEvent(AppMessage::AiLinkClosed(generation, reason)) => {
                let mut s = shared.borrow_mut();
                if s.ai.link_generation == generation { s.ai_on_link_closed(reason); }
            }
            Event::UserEvent(AppMessage::AiCaptured(tab, what, result)) => {
                shared.borrow_mut().ai_on_captured(tab, what, result);
            }
            Event::UserEvent(AppMessage::AiReadExtra(task, tab)) => {
                shared.borrow_mut().ai_read_extra_now(task, tab);
            }
            Event::UserEvent(AppMessage::AgentEval(id)) => {
                shared.borrow_mut().agent_eval_run(id);
            }
            Event::UserEvent(AppMessage::AgentEvalDone(id, raw)) => {
                shared.borrow_mut().agent_eval_done(id, raw);
            }
            Event::UserEvent(AppMessage::AgentJob(id, raw)) => {
                shared.borrow_mut().agent_job_done(id, raw);
            }
            Event::UserEvent(AppMessage::PageRead(tab, document, sequence, raw)) => {
                shared.borrow_mut().page_read_done(tab, document, sequence, raw);
            }
            Event::UserEvent(AppMessage::FindDone(tab, sequence, raw)) => {
                shared.borrow_mut().find_done(tab, sequence, raw);
            }
            Event::UserEvent(AppMessage::OmniboxReady(source, id, result)) => {
                shared.borrow_mut().reply(source, Reply::ok(id, result));
            }
            Event::UserEvent(AppMessage::MouseDown(x, y)) => {
                shared.borrow_mut().canvas_mouse_down(x, y);
            }
            Event::UserEvent(AppMessage::CanvasPointer(x, y)) => {
                shared.borrow_mut().canvas_local_mouse_down(x, y);
            }
            Event::UserEvent(AppMessage::NativeShortcut(shortcut)) => {
                let name = shortcut_name(&shortcut);
                let args = shortcut_args(&shortcut);
                AppState::emit_public(&shared, "shell.shortcut", json!({ "name": name, "args": args }));
                shared.borrow_mut().perform_shortcut(name, args);
            }
            Event::UserEvent(AppMessage::PageProbe(tab, raw)) => {
                let mut s = shared.borrow_mut();
                if !s.lifecycle_probe(tab, &raw) && !s.find_probe(tab, &raw) && !s.settings_probe(tab, &raw) {
                    s.ai_on_probe(tab, &raw);
                }
            }
            Event::UserEvent(AppMessage::PageLoaded(tab)) => {
                let mut s = shared.borrow_mut();
                #[cfg(windows)]
                if let Some(live) = s.live.get(&tab) {
                    let proxy = s.proxy();
                    let _ = live.webview.evaluate_script_with_callback(
                        "document.contentType==='application/pdf'?location.href:null",
                        move |raw| {
                            if let Ok(url) = serde_json::from_str::<String>(&raw) {
                                let _ = proxy.send_event(AppMessage::PdfDetected(tab, url));
                            }
                        });
                }
                s.refresh_history_state(tab);
                if let Some(page) = s.tabs.get_mut(tab) { page.loading = false; }
                s.emit_tabs();
            }
            Event::UserEvent(AppMessage::PdfRequest(tab, req, responder)) => {
                let s = shared.borrow();
                let source = crate::pdf::requested_source(&req);
                let Some(live) = s.live.get(&tab).filter(|_| s.tabs.get(tab).is_some_and(|page| page.url == source)) else {
                    responder.respond(wry::http::Response::builder().status(403).body(Vec::new()).unwrap());
                    return;
                };
                let cookies = if source.starts_with("http") { live.webview.cookies_for_url(&source) } else { Ok(Vec::new()) };
                match cookies {
                    Ok(cookies) => crate::pdf::serve_authenticated(req, responder, cookies, &live.pdf),
                    Err(_) => responder.respond(wry::http::Response::builder().status(503)
                        .body(b"Could not access the browser session; retry shortly.".to_vec()).unwrap()),
                }
            }
            Event::UserEvent(AppMessage::DownloadStarted(tab, url, path)) => {
                let mut s = shared.borrow_mut();
                s.on_download_started(tab, url, &path);
            }
            Event::UserEvent(AppMessage::DownloadFinished(url, path, ok)) => {
                let mut s = shared.borrow_mut();
                s.on_download_finished(&url, path, ok);
            }
            Event::UserEvent(AppMessage::Flush) => {
                let mut s = shared.borrow_mut();
                s.ai_tick();
                s.lifecycle_tick();
                if matches!(s.session_dirty_at, Some(t) if Instant::now() >= t) {
                    s.flush_session();
                }
            }
            Event::UserEvent(AppMessage::SelfTestStart(url)) => {
                let mut s = shared.borrow_mut();
                s.self_test_url_pending = true;
                if let Some(id) = s.active {
                    s.navigate(id, &url, true);
                }
            }
            Event::UserEvent(AppMessage::SelfTestNavigated) => {
                let report = {
                    let mut s = shared.borrow_mut();
                    s.self_test()
                };
                let path = shared.borrow().data_dir.join("self-test.json");
                if let Ok(text) = serde_json::to_string_pretty(&report) {
                    let _ = std::fs::write(&path, text);
                }
                eprintln!(
                    "SELF-TEST process_count={} rss_mb={} shell_ready_ms={} tabs={}",
                    report["process_count"],
                    report["rss_mb"],
                    report["shell_ready_ms"],
                    report["tabs"]
                );
                shut_down(&shared);
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(AppMessage::DriverStep(i)) => {
                let mut s = shared.borrow_mut();
                if i != s.driver_step {
                    return; // a stale wake-up
                }
                let (settle, done) = s.driver_step();
                if done {
                    // Re-enter the loop via a message so `DriverDone` handling
                    // (and its ControlFlow::Exit) runs normally.
                    let proxy = s.proxy();
                    drop(s);
                    let _ = proxy.send_event(AppMessage::DriverDone);
                    return;
                }
                let proxy = s.proxy();
                let next = s.driver_step;
                drop(s);
                std::thread::spawn(move || {
                    std::thread::sleep(settle);
                    let _ = proxy.send_event(AppMessage::DriverStep(next));
                });
            }
            Event::UserEvent(AppMessage::DriverDone) => {
                let report = {
                    let mut s = shared.borrow_mut();
                    s.driver_report()
                };
                let path = shared.borrow().data_dir.join("driver.json");
                if let Ok(text) = serde_json::to_string_pretty(&report) {
                    let _ = std::fs::write(&path, text);
                }
                let snaps = report["snapshots"].as_array().cloned().unwrap_or_default();
                let first = snaps.first().cloned().unwrap_or_default();
                let last = snaps.last().cloned().unwrap_or_default();
                eprintln!(
                    "DRIVER scenario={} steps={} tabs {}->{} live_webviews {}->{} rss_mb {}->{}",
                    report["scenario"],
                    report["steps"],
                    first["tabs"],
                    last["tabs"],
                    first["live_webviews"],
                    last["live_webviews"],
                    first["rss_mb"],
                    last["rss_mb"]
                );
                shut_down(&shared);
                *control_flow = ControlFlow::Exit;
                // R0a's measured verdicts. Inert unless this run was `--proof`; it
                // exits 1 on any FAIL, because the event loop's exit path carries no
                // status code. Called after `shut_down` so the WebView2 children are
                // already released — every number in the report was captured during
                // the run, not here.
                crate::ai::proof::report_and_exit_if_failed();
            }
            Event::UserEvent(AppMessage::Quit) => {
                shut_down(&shared);
                *control_flow = ControlFlow::Exit;
            }
            #[cfg(target_os = "macos")]
            Event::LoopDestroyed => shut_down(&shared),
            _ => {}
        }
    });
}

impl AppState {
    fn proxy(&self) -> EventLoopProxy<AppMessage> {
        // `proxy` is installed before any webview exists, so this is always
        // Some in practice; a cloned proxy is cheap.
        self.proxy
            .clone()
            .expect("event loop proxy installed before webview creation")
    }

    fn on_title(&mut self, id: TabId, title: String) {
        let url = match self.tabs.get_mut(id) {
            Some(t) => {
                if t.title == title {
                    return;
                }
                t.title = title.clone();
                t.url.clone()
            }
            None => return,
        };
        let _ = self.store.update_title(&url, &title);
        if let Some(p) = self.tabs.get(id).map(|t| self.tab_json(t)) {
            self.emit("tab.updated", p);
        }
        self.mark_dirty();
    }

    fn on_download_started(&mut self, tab: TabId, url: String, suggested: &std::path::Path) {
        #[cfg(windows)]
        let dir = PathBuf::from(self.download_dir());
        #[cfg(target_os = "macos")]
        let dir = suggested.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(self.download_dir()));
        let (dir, name) = downloads::resolve_destination(suggested, &dir);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.note(format!("download dir: {e}"));
        }
        #[cfg(windows)]
        let name = downloads::unique_name(&dir, &name);
        // Ids must come from the surviving records, not a private counter: the
        // counter is restored from the session, but `download.action` matches
        // `download_id` against `self.downloads`, so two records can end up
        // with the same id (a cleared list then a new download) and the wrong
        // row would be opened or removed.
        self.next_download_id = self
            .next_download_id
            .max(self.downloads.iter().map(|d| d.id).max().unwrap_or(0))
            + 1;
        let path = dir.join(&name).display().to_string();
        self.download_owners.insert(self.next_download_id, tab);
        self.downloads.push(Download {
            id: self.next_download_id,
            url,
            file: name,
            path,
            state: DownloadState::InProgress,
            total_bytes: None,
            received_bytes: None,
        });
        // Bounded list: only the newest `MAX_DOWNLOAD_ROWS` records are kept
        // (in-progress rows are never evicted, so this is a retention cap on
        // completed history, not a cancellation).
        trim_downloads(&mut self.downloads);
        let payload = json!({ "downloads": self.downloads });
        self.emit("state.downloads", payload);
        self.mark_dirty();
    }

    fn on_download_finished(&mut self, url: &str, path: Option<PathBuf>, ok: bool) {
        if let Some(d) = self
            .downloads
            .iter_mut()
            .rev()
            .find(|d| d.url == url && d.state == DownloadState::InProgress)
        {
            self.download_owners.remove(&d.id);
            d.state = if ok { DownloadState::Completed } else { DownloadState::Failed };
            if let Some(p) = path {
                d.path = p.display().to_string();
            }
        }
        let payload = json!({ "downloads": self.downloads });
        self.emit("state.downloads", payload);
        self.mark_dirty();
    }
}

fn handle_window_event(shared: &Shared, event: WindowEvent, control_flow: &mut ControlFlow) {
    match event {
        WindowEvent::CloseRequested | WindowEvent::Destroyed => {
            shut_down(shared);
            *control_flow = ControlFlow::Exit;
        }
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            let mut s = shared.borrow_mut();
            s.dpi_scale = s.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
            s.apply_layout();
        }
        WindowEvent::Focused(true) => {
            let mut s = shared.borrow_mut();
            s.note("window focused");
        }
        _ => {}
    }
}

fn shut_down(shared: &Shared) {
    let mut s = shared.borrow_mut();
    if s.shutting_down {
        return;
    }
    s.shutting_down = true;
    s.ai_shutdown();
    s.flush_session();
    if s.settings.clear_history_on_exit {
        if let Err(e) = s.store.clear_history() {
            s.note(format!("clear history on exit: {e}"));
        }
    }
    if s.settings.clear_cookies_on_exit || s.settings.clear_cache_on_exit {
        if let Some(wv) = s.shell.as_ref() {
            if let Err(e) = wv.clear_all_browsing_data() {
                s.note(format!("clear browsing data: {e}"));
            }
        }
    }
    s.note("shut down");
}

fn build_shell(shared: &Shared) {
    let mut s = shared.borrow_mut();
    let proxy = s.proxy();
    let builder = WebViewBuilder::new()
        .with_id("shell")
        .with_bounds(layout::rect(0, 0, 1280, 820))
        .with_visible(true)
        .with_focused(false)
        .with_custom_protocol("asset".into(), asset_handler())
        .with_ipc_handler(move |req: HttpRequest<String>| {
            let _ = proxy.send_event(AppMessage::Ipc(Surface::Shell, req.body().clone()));
        })
        .with_navigation_handler(|url| is_asset_origin(&url))
        .with_url(ui::SHELL_URL);
    let builder = platform::configure(builder, s.environment.as_ref(), true);
    let Some(window) = s.window.as_ref() else { return };
    match builder.build_as_child(window) {
        Ok(wv) => {
            // Remember the shell's child HWND: keystrokes going to it are the
            // shell's own, so the accelerator hook must not intercept them.
            #[cfg(windows)]
            { s.shell_hwnd = wv.hwnd().0 as isize; }
            s.shell = Some(wv);
            s.note("shell webview ready");
        }
        Err(e) => {
            drop(s);
            platform::show_fatal(&format!(
                "Cannot create the shell webview.\n\n{e}\n\n{} runtime: {}",
                platform::ENGINE_NAME,
                webview_version().unwrap_or_else(|| "not detected".into())
            ));
            std::process::exit(5);
        }
    }
}

/// Build the overlay webview at startup. Deferred when the proof harness asked to
/// start without it, and rebuilt on demand by the `proof.overlay` command so its
/// cost lands between two samples instead of inside every sample.
fn build_overlay(shared: &Shared) {
    shared.borrow_mut().build_overlay_webview();
}

impl AppState {
    /// Create the AI overlay webview, or leave the state untouched if the window
    /// does not exist yet.
    ///
    /// Called from startup and from the harness's `proof.overlay` command. It is
    /// idempotent in the sense that matters to the harness: a second call while the
    /// overlay is live is refused by the caller, not attempted here, because a
    /// second webview would double the very cost being measured.
    fn build_overlay_webview(&mut self) {
        let proxy = self.proxy();
        let builder = WebViewBuilder::new()
            .with_id("overlay")
            .with_bounds(layout::rect(0, 0, 0, 0))
            .with_visible(false)
            .with_transparent(true)
            .with_custom_protocol("asset".into(), asset_handler())
            .with_ipc_handler(move |req: HttpRequest<String>| {
                let _ = proxy.send_event(AppMessage::Ipc(Surface::Overlay, req.body().clone()));
            })
            .with_navigation_handler(|url| is_asset_origin(&url))
            .with_url("asset://shell/overlay.html");
        let builder = platform::configure(builder, self.environment.as_ref(), true);
        let Some(window) = self.window.as_ref() else { return };
        match builder.build_as_child(window) {
            Ok(wv) => {
                self.overlay = Some(wv);
                self.note("overlay webview ready (0x0)");
            }
            Err(e) => self.note(format!("overlay webview failed: {e}")),
        }
    }
}

impl AppState {
    /// A page webview committed a navigation.
    fn on_navigation(&mut self, id: TabId, url: &str) {
        if let Some(live) = self.live.get_mut(&id) {
            live.pdf.cancel();
            #[cfg(windows)]
            { live.native_pdf = None; }
            live.lifecycle_known = false;
            live.editing = false;
            live.playing = false;
        }
        if self.active == Some(id) {
            self.find_seq += 1;
            self.find = None;
        }
        // The PDF viewer page stands for the document it shows.
        let shown = crate::pdf::source_of(url);
        let url = shown.as_deref().unwrap_or(url);
        self.navigation_seq += 1;
        self.ai_on_navigation(id, url);

        let display = if host::is_internal_page(url, "newtab.html") {
            tabs::NEWTAB_URL.to_string()
        } else if host::is_internal_page(url, "settings.html") {
            "native://settings".to_string()
        } else {
            url.to_string()
        };

        if let Some(t) = self.tabs.get_mut(id) {
            t.url = display;
            t.loading = true;
        }
        self.refresh_history_state(id);

        self.emit_tabs();

        // History is written on *commit*, not on request. The command path can
        // only see navigations the host initiated; a link click, a form POST or
        // a redirect never goes through it. Recording here is the only place
        // that sees every committed page — without it, history contained only
        // omnibox submissions.
        if let Some(t) = self.tabs.get(id) {
            let url = t.url.clone();
            let title = t.display_title();
            if should_record_history(&url) {
                let typed = self.typed_target.as_ref().is_some_and(|(tid, u)| {
                    *tid == id && same_page(u, &url)
                });
                if typed {
                    self.typed_target = None;
                }
                if let Err(e) = self.store.record_visit(&url, &title, ipc::now_secs(), typed) {
                    self.note(format!("history: {e}"));
                }
            }
        }

        let coalesced = match self.last_nav_emit.get(&id) {
            Some(t) => t.elapsed() < NAV_COALESCE,
            None => false,
        };
        if !coalesced {
            self.last_nav_emit.insert(id, Instant::now());
            if let Some(t) = self.tabs.get(id) {
                let payload = self.tab_json(t);
                self.emit("tab.navigated", payload);
            }
            self.mark_dirty();
            if !url.starts_with("asset://") {
                self.note(format!("tab {} navigated: {url}", id.0));
            }
        }

        // A committed real page ends the self-test wait.
        if self.self_test_url_pending && !url.starts_with("asset://") {
            self.self_test_url_pending = false;
            // Give the renderer a moment to paint before sampling.
            let proxy = self.proxy();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(2_000));
                let _ = proxy.send_event(AppMessage::SelfTestNavigated);
            });
        }
    }
}

/// Decode a `MSG` into (vk, mods) and consult `keys::resolve`.
#[cfg(windows)]
fn dispatch_accelerator(shared: &Shared, msg: *const std::ffi::c_void) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_SYSKEYDOWN};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    // SAFETY: tao passes a valid `*mut MSG` that lives for the call.
    let msg = unsafe { &*(msg as *const windows::Win32::UI::WindowsAndMessaging::MSG) };
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return false;
    }
    let vk = msg.wParam.0 as u32;
    let mods = Mods {
        // SAFETY: GetKeyState has no preconditions and reports the thread's
        // current keyboard state.
        ctrl: unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) < 0 },
        alt: unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x12) < 0 },
        shift: unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x10) < 0 },
    };
    if mods == (Mods { ctrl: false, alt: false, shift: false }) && vk == 0 {
        return false;
    }

    let (focused_is_input, shell_hwnd) = {
        let s = match shared.try_borrow() {
            Ok(s) => s,
            // A re-entrant call while a command handler mutates state: let the
            // key through rather than panic.
            Err(_) => return false,
        };
        (s.shell_focus_in_input, s.shell_hwnd)
    };

    // If focus is inside the shell webview, its JS handles the key. Focus in a
    // page webview is a different HWND owned by msedgewebview2.exe.
    // SAFETY: GetFocus returns the thread's focus HWND or null.
    let focus = unsafe { GetFocus() };
    if !focus.is_invalid() && focus.0 as isize == shell_hwnd {
        return false;
    }

    match keys::resolve(vk, mods, focused_is_input) {
        Outcome::Pass => false,
        Outcome::Consume(sc) => {
            let name = shortcut_name(&sc);
            let args = shortcut_args(&sc);
            // `emit` calls `evaluate_script` on the shell. That is a synchronous
            // COM call into WebView2, which can pump messages and re-enter this
            // very hook; a `RefCell` borrow held across it would turn into a
            // `BorrowMutError` panic inside a Win32 callback. Borrow, act, and
            // release before any script runs.
            {
                let mut s = match shared.try_borrow_mut() {
                    Ok(s) => s,
                    Err(_) => return false,
                };
                s.note(format!("accelerator: {name}"));
            }
            // The shell owns focus/UI effects; ask it first, then apply the
            // host-side effect so the action never depends on shell focus.
            AppState::emit_public(shared, "shell.shortcut", json!({ "name": name, "args": args }));
            let mut s = match shared.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return false,
            };
            s.perform_shortcut(name, args);
            true
        }
    }
}

impl AppState {
    /// Emit a host→UI event on a *fresh* borrow. Used where the caller must not
    /// already hold one (the Win32 message hook).
    fn emit_public(shared: &Shared, event: &str, payload: Value) {
        if let Ok(mut s) = shared.try_borrow_mut() {
            s.emit(event, payload);
        }
    }
}

fn shortcut_name(s: &Shortcut) -> &'static str {
    use Shortcut as S;
    match s {
        S::NewTab => "new_tab",
        S::CloseTab => "close_tab",
        S::CloseWindow => "close_window",
        S::NextTab => "next_tab",
        S::PrevTab => "prev_tab",
        S::RapidNextTab => "rapid_next_tab",
        S::RapidPrevTab => "rapid_prev_tab",
        S::TabIndex(_) => "tab_index",
        S::ReopenClosed => "reopen_closed",
        S::FocusOmnibox => "focus_omnibox",
        S::CycleFocus => "cycle_focus",
        S::OpenPalette => "open_palette",
        S::ClosePalette => "close_palette",
        S::Find => "find",
        S::AskAi => "ask_ai",
        S::FindNext => "find_next",
        S::FindPrev => "find_prev",
        S::OpenDownloads => "open_downloads",
        S::OpenHistory { today: false } => "open_history",
        S::OpenHistory { today: true } => "open_history_today",
        S::PageBack => "page_back",
        S::PageForward => "page_forward",
        S::Home => "home",
        S::Reload => "reload",
        S::ReloadIgnoringCache => "reload_ignoring_cache",
        S::Bookmark => "bookmark",
        S::BookmarkAllTabs => "bookmark_all_tabs",
        S::ViewSource => "view_source",
        S::ZoomIn => "zoom_in",
        S::ZoomOut => "zoom_out",
        S::ZoomReset => "zoom_reset",
        S::OpenSettings => "open_settings",
        S::Escape => "escape",
        S::ItemMenu => "item_menu",
        S::ToggleRail => "toggle_rail",
        S::TogglePin => "toggle_pin",
        S::CloseTabOrWord => "close_tab_or_word",
    }
}

fn shortcut_args(s: &Shortcut) -> Value {
    match s {
        Shortcut::TabIndex(n) => json!(n),
        Shortcut::OpenHistory { today } => json!(today),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_records_only_real_web_pages() {
        for ok in [
            "https://example.com/",
            "http://127.0.0.1:3000/",
            "HTTPS://Example.COM/x",
            "ftp://files.example/",
        ] {
            assert!(should_record_history(ok), "{ok} must be recorded");
        }
        for no in [
            "",
            " ",
            "asset://shell/pages/newtab.html",
            "native://newtab",
            "about:blank",
            "data:text/html,hi",
            "view-source:https://a.b",
            "chrome-extension://abc/x",
        ] {
            assert!(!should_record_history(no), "{no} must not be recorded");
        }
    }

    #[test]
    fn typed_mark_matches_the_committed_page_through_redirects() {
        assert!(same_page("https://a.com", "https://a.com/"));
        assert!(same_page("https://a.com/x", "https://A.COM/x#frag"));
        assert!(!same_page("https://a.com/x", "https://a.com/y"));
        assert!(!same_page("https://a.com/", "https://evil.com/"));
    }

    #[test]
    fn trim_downloads_keeps_in_progress_and_caps_history() {
        let mk = |id: u64, state| Download {
            id,
            url: format!("https://a/{id}"),
            file: format!("{id}.bin"),
            path: format!(r"C:\dl\{id}.bin"),
            state,
            total_bytes: None,
            received_bytes: None,
        };
        let mut list: Vec<Download> = (1..=(MAX_DOWNLOAD_ROWS as u64 + 50))
            .map(|i| mk(i, DownloadState::Completed))
            .collect();
        // One in-progress row in the middle must survive even though it is old.
        list.insert(0, mk(0, DownloadState::InProgress));
        trim_downloads(&mut list);
        assert_eq!(list.len(), MAX_DOWNLOAD_ROWS);
        assert_eq!(list[0].id, 0, "in-progress rows are never evicted");
        assert_eq!(list.last().unwrap().id, MAX_DOWNLOAD_ROWS as u64 + 50);
        // Only in-progress rows: nothing can be evicted, and nothing is lost.
        let mut all_live: Vec<Download> = (0..300).map(|i| mk(i, DownloadState::InProgress)).collect();
        trim_downloads(&mut all_live);
        assert_eq!(all_live.len(), 300);
    }

    #[test]
    fn panel_str_is_stable_for_the_wire() {
        assert_eq!(panel_str(Panel::None), "none");
        assert_eq!(panel_str(Panel::Downloads), "downloads");
        assert_eq!(panel_str(Panel::History), "history");
    }

    #[test]
    fn command_catalog_has_unique_verbs() {
        let mut seen = std::collections::HashSet::new();
        for c in commands_catalog() {
            assert!(seen.insert(c.verb), "duplicate verb {}", c.verb);
        }
    }

    #[test]
    fn command_row_titles_round_trip_through_classify() {
        // `app.js` `completeSelection` puts `r.url || r.title` back into the
        // omnibox, and the submit path calls `omnibox::classify`. A row title
        // that does not classify as the same command means Tab-complete turns
        // a command into a web search.
        for c in commands_catalog() {
            let title = if c.args.is_empty() {
                c.verb.to_string()
            } else {
                format!("{} {}", c.verb, c.args)
            };
            let typed = format!("> {title}");
            match omnibox::classify(&typed) {
                Intent::Command { verb, rest } => {
                    assert_eq!(verb, c.verb, "row {title:?} reclassifies to {verb:?}");
                    assert_eq!(rest, c.args, "row {title:?} lost its args");
                }
                other => panic!("row {title:?} classifies as {other:?}, not a command"),
            }
            // And it must not read as a URL/search by itself either.
            assert!(!title.contains('—'), "row title must not carry display text: {title:?}");
        }
    }

    #[test]
    fn shortcut_names_are_unique_across_the_map() {
        let all = [
            Shortcut::NewTab,
            Shortcut::CloseTab,
            Shortcut::CloseWindow,
            Shortcut::NextTab,
            Shortcut::PrevTab,
            Shortcut::RapidNextTab,
            Shortcut::RapidPrevTab,
            Shortcut::TabIndex(1),
            Shortcut::ReopenClosed,
            Shortcut::FocusOmnibox,
            Shortcut::CycleFocus,
            Shortcut::OpenPalette,
            Shortcut::ClosePalette,
            Shortcut::Find,
            Shortcut::AskAi,
            Shortcut::FindNext,
            Shortcut::FindPrev,
            Shortcut::OpenDownloads,
            Shortcut::OpenHistory { today: false },
            Shortcut::OpenHistory { today: true },
            Shortcut::PageBack,
            Shortcut::PageForward,
            Shortcut::Home,
            Shortcut::Reload,
            Shortcut::ReloadIgnoringCache,
            Shortcut::Bookmark,
            Shortcut::BookmarkAllTabs,
            Shortcut::ViewSource,
            Shortcut::ZoomIn,
            Shortcut::ZoomOut,
            Shortcut::ZoomReset,
            Shortcut::OpenSettings,
            Shortcut::Escape,
            Shortcut::ItemMenu,
            Shortcut::ToggleRail,
            Shortcut::TogglePin,
            Shortcut::CloseTabOrWord,
        ];
        let mut seen = std::collections::HashSet::new();
        for s in &all {
            assert!(seen.insert(shortcut_name(s)), "duplicate {}", shortcut_name(s));
        }
    }
}