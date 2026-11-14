//! The driver: a sequential program of steps run inside tao's event loop.
//!
//! Each experiment is a `Vec<Step>`. A step is a closure that is called on every
//! event-loop iteration until it reports `Done`, which is what lets a step wait
//! for a COM callback or a page message without blocking the thread those
//! callbacks need. Nothing in here spawns a thread or blocks on a channel; a
//! blocked UI thread would freeze WebView2 and turn every latency number into
//! garbage.

use std::collections::VecDeque;
use std::path::PathBuf;

use tao::dpi::LogicalPosition;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment;
use windows::Win32::Foundation::HWND;
use wry::{WebView, WebViewExtWindows, WebContext};

use crate::{env, hooks, win};

/// What a step returns each time it is polled.
pub enum Tick {
    /// Not finished; call me again next iteration.
    Running,
    /// Finished; move to the next step.
    Done,
}

pub type Step = Box<dyn FnMut(&mut Harness) -> Tick>;

/// Box a closure into a [`Step`].
pub fn step<F: FnMut(&mut Harness) -> Tick + 'static>(f: F) -> Step {
    Box::new(f)
}

/// Geometry and style for one top-level window, created before the loop starts.
pub struct WindowSpec {
    pub title: &'static str,
    /// Physical pixels; converted through the primary monitor's scale factor.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub decorations: bool,
    pub visible: bool,
}

impl WindowSpec {
    pub fn new(title: &'static str, x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            title,
            x,
            y,
            w,
            h,
            decorations: true,
            visible: true,
        }
    }

    /// Borderless and hidden until a step shows it — the overlay shape.
    pub fn overlay(title: &'static str, x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            title,
            x,
            y,
            w,
            h,
            decorations: false,
            visible: false,
        }
    }
}

/// Create every window up front.
///
/// tao 0.37 has no `create_window` on the running event loop, so windows that a
/// step needs later still have to be built here. They start hidden where the
/// experiment wants them hidden, which is also what keeps the memory baseline
/// honest: a hidden window with no webview costs one empty HWND and nothing else.
pub fn make_windows(event_loop: &EventLoop<()>, specs: &[WindowSpec]) -> Vec<Window> {
    specs
        .iter()
        .map(|s| {
            WindowBuilder::new()
                .with_title(s.title)
                .with_position(LogicalPosition::new(s.x as f64, s.y as f64))
                .with_inner_size(tao::dpi::LogicalSize::new(s.w as f64, s.h as f64))
                .with_decorations(s.decorations)
                .with_visible(s.visible)
                .with_skip_taskbar(!s.decorations)
                .build(event_loop)
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("create probe windows")
}

/// Everything a step can act on.
pub struct Harness {
    /// `wry` needs `&mut WebContext` for the whole build call, so it is taken out
    /// of the harness for the duration and put back afterwards.
    context: Option<WebContext>,
    env: ICoreWebView2Environment,
    windows: Vec<Window>,
    /// Every webview, indexed independently of windows: V2 and V3 put a page and
    /// an overlay in the *same* window, which is how `native-browser` does it
    /// (`build_as_child` for shell, overlay and page alike).
    views: Vec<Option<WebView>>,
    /// Which window each view was built into.
    view_window: Vec<usize>,
    pub sink: hooks::Sink,
    msg_tx: std::sync::mpsc::Sender<hooks::PageMsg>,
    msg_rx: std::sync::mpsc::Receiver<hooks::PageMsg>,
    /// Messages received since the last `drain_messages`.
    pub inbox: Vec<hooks::PageMsg>,
    pub probe: win::PixelProbe,
    steps: VecDeque<Step>,
}

/// Views beyond the window count, for the page-plus-overlay shape.
const EXTRA_VIEWS: usize = 3;

impl Harness {
    pub fn new(env: ICoreWebView2Environment, windows: Vec<Window>, data_dir: PathBuf) -> Self {
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        let n = windows.len();
        let views = (0..n + EXTRA_VIEWS)
            .map(|_| None)
            .collect::<Vec<Option<WebView>>>();
        let view_window = (0..n + EXTRA_VIEWS)
            .map(|i| i.min(n - 1))
            .collect::<Vec<usize>>();
        Self {
            context: Some(WebContext::new(Some(data_dir.clone()))),
            env,
            windows,
            views,
            view_window,
            sink: hooks::new_sink(),
            msg_tx,
            msg_rx,
            inbox: Vec::new(),
            probe: win::PixelProbe::new(),
            steps: VecDeque::new(),
        }
    }

    pub fn load(&mut self, steps: Vec<Step>) {
        self.steps = steps.into();
    }

    /// Run one iteration. Returns true when the program is finished.
    pub fn tick(&mut self) -> bool {
        self.drain_messages();
        loop {
            // The step is taken out of the queue before it runs so it can call
            // back into the harness without aliasing it, and put back at the
            // front by `Tick::Running` to be retried on the next iteration.
            let Some(mut head) = self.steps.pop_front() else {
                break;
            };
            match head(self) {
                Tick::Done => {}
                Tick::Running => {
                    self.steps.push_front(head);
                    return false;
                }
            }
        }
        true
    }

    fn drain_messages(&mut self) {
        while let Ok(m) = self.msg_rx.try_recv() {
            self.inbox.push(m);
        }
    }

    /// Take every inbox message of `kind` from `label`, oldest first.
    pub fn take_msgs(&mut self, label: &str, kind: &str) -> Vec<hooks::PageMsg> {
        let mut taken = Vec::new();
        let mut keep = Vec::new();
        for m in self.inbox.drain(..) {
            if m.label == label && m.kind == kind {
                taken.push(m);
            } else {
                keep.push(m);
            }
        }
        self.inbox = keep;
        taken
    }

    /// Drop everything received so far, so a later wait cannot observe a stale
    /// message from a previous step.
    pub fn clear_inbox(&mut self) {
        self.inbox.clear();
    }

    // -- webview lifecycle ---------------------------------------------------

    /// Build a webview filling `slot`'s client area, opaque, and return once the
    /// build call has returned.
    pub fn build(&mut self, slot: usize, label: &str, url: Option<&str>) -> Result<(), String> {
        self.build_in(slot, slot, label, url, None, false)
    }

    /// Build a webview into `view`, parented to window `window`.
    ///
    /// `bounds` is in window-client logical pixels, `None` meaning the whole
    /// client area. This is the shape `native-browser` uses for its overlay: a
    /// second `build_as_child` webview in the same top-level window, moved with
    /// `set_bounds` rather than a second host-owned top-level window.
    pub fn build_in(
        &mut self,
        view: usize,
        window: usize,
        label: &str,
        url: Option<&str>,
        bounds: Option<(i32, i32, i32, i32)>,
        transparent: bool,
    ) -> Result<(), String> {
        let mut context = self
            .context
            .take()
            .expect("web context must be present between builds");
        let client = win::client_rect_on_screen(self.hwnd(window));
        // A webview with no bounds fills the client, and `env::logical_rect` below
        // takes logical px while `client_rect_on_screen` returns physical. Feeding one
        // to the other made a full-window webview `scale`× too large — 1590×990 logical
        // in a 1060×660 logical client — which showed up as the page reporting
        // `innerHeight` 990 while only 660 of it was on screen, and as a page surface
        // 2.25× the pixels the shipping shell would composite.
        let scale = win::dpi_of(self.hwnd(window)).max(96) as f64 / 96.0;
        let rect = bounds.unwrap_or((
            0,
            0,
            (client.2 as f64 / scale).round().max(1.0) as i32,
            (client.3 as f64 / scale).round().max(1.0) as i32,
        ));
        let built = env::build_webview(
            &mut context,
            &self.env,
            &self.windows[window],
            label,
            env::logical_rect(rect.0, rect.1, rect.2, rect.3),
            url,
            transparent,
        );
        self.context = Some(context);
        let wv = built?;
        let com = env::com_of(&wv);
        // SAFETY: the webview is alive, created on this thread, and this thread
        // owns the COM apartment WebView2 was created in.
        unsafe {
            hooks::attach_nav_events(label, &com, &self.sink)?;
            hooks::attach_page_messages(label, &com, self.msg_tx.clone())?;
        }
        println!(
            "BUILD {label} view={view} window={window} bounds={:?} transparent={transparent} \
             controller_bg={} client={}x{}+{},{} webview_hwnd={}",
            rect,
            env::default_background_alpha(&wv),
            client.2,
            client.3,
            client.0,
            client.1,
            win::hwnd_value(wv.hwnd())
        );
        self.views[view] = Some(wv);
        self.view_window[view] = window;
        Ok(())
    }

    /// Close a webview. Dropping wry's `WebView` releases the controller, which
    /// is the "webview went away" half of the id-reuse question.
    pub fn close(&mut self, slot: usize) {
        let dropped = self.views[slot].take();
        println!("CLOSE slot={slot} dropped={}", dropped.is_some());
    }

    /// Number of window slots the harness was built with.
    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// The raw `ICoreWebView2` of a slot, if it is built.
    pub fn com(&self, slot: usize) -> Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2> {
        self.views[slot].as_ref().map(env::com_of)
    }

    /// Run a navigation on a slot's webview.
    pub fn nav(&mut self, slot: usize, url: &str) -> Result<(), String> {
        let Some(wv) = self.com(slot) else {
            return Err(format!("nav: slot {slot} has no webview"));
        };
        let h = windows::core::HSTRING::from(url);
        // SAFETY: live webview on this thread's apartment.
        unsafe { wv.Navigate(&h) }.map_err(|e| format!("Navigate({url}): {e}"))?;
        Ok(())
    }

    pub fn reload(&mut self, slot: usize) -> Result<(), String> {
        let Some(wv) = self.com(slot) else {
            return Err(format!("reload: slot {slot} has no webview"));
        };
        // SAFETY: live webview.
        unsafe { wv.Reload() }.map_err(|e| format!("Reload: {e}"))
    }

    pub fn go_back(&mut self, slot: usize) -> Result<(), String> {
        let Some(wv) = self.com(slot) else {
            return Err(format!("back: slot {slot} has no webview"));
        };
        // SAFETY: live webview.
        unsafe { wv.GoBack() }.map_err(|e| format!("GoBack: {e}"))
    }

    pub fn go_forward(&mut self, slot: usize) -> Result<(), String> {
        let Some(wv) = self.com(slot) else {
            return Err(format!("forward: slot {slot} has no webview"));
        };
        // SAFETY: live webview.
        unsafe { wv.GoForward() }.map_err(|e| format!("GoForward: {e}"))
    }

    /// Process id of the shared browser process, for attributing memory lines.
    pub fn browser_process_id(&self, slot: usize) -> Option<u32> {
        let wv = self.com(slot)?;
        let mut pid = 0u32;
        // SAFETY: live webview, out-param to a local.
        unsafe { wv.BrowserProcessId(&mut pid).ok()? };
        Some(pid)
    }

    // -- overlay-shaped views ------------------------------------------------

    /// The child HWND a view's webview owns. This is what the ex-style, region
    /// and hit-test measurements act on: the overlay is a *child* window of the
    /// browser window, not a top-level of its own.
    pub fn view_hwnd(&self, view: usize) -> Option<HWND> {
        self.views[view].as_ref().map(|wv| wv.hwnd())
    }

    pub fn view_hwnd_value(&self, view: usize) -> Option<isize> {
        self.view_hwnd(view).map(win::hwnd_value)
    }

    /// `WebView::set_bounds`, timed. This is the per-frame follow call the
    /// shipping overlay makes (`app.rs:649`), so its cost is the cost the
    /// anchored-card design pays on every frame it repositions.
    pub fn move_view(&mut self, view: usize, x: i32, y: i32, w: i32, h: i32) -> Result<f64, String> {
        let Some(wv) = self.views[view].as_ref() else {
            return Err(format!("move_view: view {view} has no webview"));
        };
        let t = win::qpc_ticks();
        wv.set_bounds(env::logical_rect(x, y, w, h))
            .map_err(|e| format!("set_bounds: {e}"))?;
        Ok(win::us_since(t))
    }

    pub fn set_view_visible(&self, view: usize, visible: bool) -> Result<(), String> {
        let Some(wv) = self.views[view].as_ref() else {
            return Err(format!("set_view_visible: view {view} has no webview"));
        };
        wv.set_visible(visible).map_err(|e| format!("set_visible: {e}"))
    }

    /// Device scale factor of the window a view lives in — the CSS px to physical
    /// px factor every geometry comparison must go through.
    pub fn scale_of(&self, view: usize) -> f64 {
        self.windows[self.view_window[view]].scale_factor()
    }

    /// Client area of a view's window in physical screen coordinates.
    pub fn client_of(&self, view: usize) -> (i32, i32, i32, i32) {
        win::client_rect_on_screen(self.hwnd(self.view_window[view]))
    }

    /// Screen position of a point in a view's client coordinate space, where
    /// WebView2's own bounds are expressed. Logical in, physical out.
    pub fn view_point_on_screen(&self, view: usize, x: i32, y: i32) -> (i32, i32) {
        let c = self.client_of(view);
        let s = self.scale_of(view);
        (c.0 + (x as f64 * s).round() as i32, c.1 + (y as f64 * s).round() as i32)
    }

    /// Apply the ex-style set to a view's child window.
    ///
    /// Returns the mask *after* the write, read back from the window. The Win32
    /// call hands back the previous mask, and printing that made every variant's
    /// style row describe the variant before it — which matters here because a
    /// style the OS refused to apply has to be distinguishable from a style that
    /// was applied and made no difference.
    pub fn view_ex_style(
        &mut self,
        view: usize,
        add: windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE,
        remove: windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE,
    ) -> Option<isize> {
        let h = self.view_hwnd(view)?;
        win::modify_ex_style(h, add, remove);
        Some(win::ex_style(h))
    }

    /// Punch a rectangular hole in a view's window region, in that window's
    /// client coordinates: the hole is outside the window for hit-testing
    /// purposes, so the OS sends clicks there to whatever is underneath.
    pub fn set_view_region(&mut self, view: usize, hole: Option<(i32, i32, i32, i32)>) -> Option<bool> {
        let h = self.view_hwnd(view)?;
        let outer = {
            let r = win::client_rect_on_screen(h);
            (0, 0, r.2, r.3)
        };
        Some(match hole {
            Some(hole) => win::set_region_with_hole(h, outer, Some(hole)),
            None => win::clear_region(h),
        })
    }

    /// Which window the OS says owns a screen point — the hit-test answer before
    /// any JavaScript is involved.
    pub fn window_at(&self, x: i32, y: i32) -> isize {
        win::window_from_point(x, y)
    }

    /// Restrict a view's window to exactly one client rectangle, so everything
    /// outside it is absent to the hit-tester rather than merely transparent.
    pub fn set_view_region_rect(&mut self, view: usize, rect: (i32, i32, i32, i32)) -> Option<bool> {
        let h = self.view_hwnd(view)?;
        Some(win::set_region_rect(h, rect))
    }

    /// Name whatever `window_at` returned: the handle, its window class, and which
    /// of the probe's own views it is, so a hit-test row reads as an answer and not
    /// as a hex number.
    ///
    /// The direct match is never enough. `WindowFromPoint` returns the innermost
    /// window at the point, which for a WebView2 is its own `Chrome_...HWND`
    /// surface — a *child* of the HWND wry hands back — so every hit-test in this
    /// probe would have printed "not a probe view" and answered nothing. The
    /// ancestor chain is walked instead, and the first ancestor that is one of the
    /// probe's views names the owner of the point.
    pub fn describe_window(&self, value: isize) -> String {
        let chain = win::ancestor_chain(value);
        let class = chain
            .first()
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| "?".to_string());
        match chain.iter().find(|(h, _)| {
            (0..self.views.len()).any(|v| self.view_hwnd_value(v) == Some(*h))
        }) {
            Some((h, _)) => {
                let v = (0..self.views.len())
                    .find(|v| self.view_hwnd_value(*v) == Some(*h))
                    .unwrap_or(usize::MAX);
                let depth = chain.iter().position(|(c, _)| *c == *h).unwrap_or(0);
                format!(
                    "hwnd={value} class={class} → view={v}{}",
                    if depth == 0 {
                        " (the view's own window)".to_string()
                    } else {
                        format!(" (its descendant {depth} levels down)")
                    }
                )
            }
            None => {
                // Nothing in the chain is a probe window: a foreign top-level, or
                // the browser window itself. Say what the chain actually holds.
                format!(
                    "hwnd={value} class={class} (no probe view in its ancestor chain {:?})",
                    chain.iter().map(|(h, c)| (h, c.as_str())).collect::<Vec<_>>()
                )
            }
        }
    }

    /// Which view owns a screen point, or `None` when no probe window does. This
    /// is the hit-test answer V3's verdict is built on, so the click counts are a
    /// cross-check on it rather than the only evidence.
    pub fn view_at(&self, x: i32, y: i32) -> Option<usize> {
        let chain = win::ancestor_chain(self.window_at(x, y));
        chain.iter().find_map(|(h, _)| {
            (0..self.views.len()).find(|v| self.view_hwnd_value(*v) == Some(*h))
        })
    }

    /// Put one of the probe's windows on top without activating it.
    ///
    /// Every hit-test number in V3 describes whichever window is *topmost* at the
    /// sample point, so this is a precondition of the measurement, not a nicety.
    pub fn raise(&self, window: usize) {
        win::bring_to_top(self.hwnd(window));
    }

    /// The top-level window the OS says owns a screen point. Compared against
    /// [`Harness::window_root`] this is the occlusion check: when they differ,
    /// some other process's window is under the point and the probe's own
    /// Z-order was never what got measured.
    pub fn root_at(&self, x: i32, y: i32) -> isize {
        win::root_of(self.window_at(x, y))
    }

    /// The top-level handle of one of the probe's own windows.
    pub fn window_root(&self, window: usize) -> isize {
        win::hwnd_value(self.hwnd(window))
    }

    /// Send a JSON payload into a view's page.
    pub fn post(&self, view: usize, json: &str) -> Result<(), String> {
        let Some(wv) = self.views[view].as_ref() else {
            return Err(format!("post: view {view} has no webview"));
        };
        let com = env::com_of(wv);
        // SAFETY: live webview on this thread's apartment.
        unsafe { hooks::post_to_page(&com, json) }
    }

    /// Fire-and-forget script on a view (same as `eval`, named for the way V2 and
    /// V3 use it, where `view` is not a window slot).
    pub fn run(&mut self, view: usize, js: &str) -> Result<(), String> {
        self.eval(view, js)
    }

    /// Fire-and-forget script.
    pub fn eval(&mut self, slot: usize, js: &str) -> Result<(), String> {
        self.views[slot]
            .as_ref()
            .ok_or_else(|| format!("eval: slot {slot} has no webview"))?
            .evaluate_script(js)
            .map_err(|e| format!("evaluate_script: {e}"))
    }

    // -- windows -------------------------------------------------------------

    pub fn hwnd(&self, slot: usize) -> HWND {
        win::hwnd_of(self.windows[slot].hwnd())
    }

    /// Move/resize a slot's top-level window; returns the microseconds the call
    /// took.
    pub fn move_window(&mut self, slot: usize, x: i32, y: i32, w: i32, h: i32, show: bool) -> f64 {
        win::set_window_pos(self.hwnd(slot), x, y, w, h, show)
    }

    pub fn show(&mut self, slot: usize) {
        win::show_no_activate(self.hwnd(slot));
    }

    /// Grow a window's inner size by a logical-pixel delta, which is what a user resizing
    /// the browser window does to an overlay: the client area changes, the overlay's
    /// bounds do not, and whatever the anchored card is pointing at moves.
    pub fn grow_window(&self, slot: usize, dx_logical: i32, dy_logical: i32) {
        let size = self.windows[slot].inner_size();
        let w = (size.width as f64 + dx_logical as f64).max(200.0);
        let h = (size.height as f64 + dy_logical as f64).max(200.0);
        self.windows[slot].set_inner_size(tao::dpi::LogicalSize::new(w, h));
    }

    // -- observation ---------------------------------------------------------

    /// Print and return the whole-process-tree memory for a labelled sample.
    pub fn rss(&mut self, tag: &str) -> win::TreeMemory {
        let m = win::tree_memory();
        println!(
            "RSS {tag:<28} procs={:<3} working_set={:>9.1} MiB private={:>9.1} MiB",
            m.processes,
            m.ws_mib(),
            m.priv_mib()
        );
        m
    }

    pub fn rss_detail(&mut self, tag: &str) {
        println!("TREE {tag}");
        for (pid, name, ws, priv_bytes) in win::tree_detail() {
            println!(
                "  pid={pid:<8} {:<28} ws={:>9.1} MiB private={:>9.1} MiB",
                name,
                ws as f64 / (1024.0 * 1024.0),
                priv_bytes as f64 / (1024.0 * 1024.0)
            );
        }
    }

    /// Percentiles of a sample set, printed in one line. Empty input prints
    /// `no samples` rather than a zero, which would be a lie.
    pub fn percentiles(tag: &str, unit: &str, mut samples: Vec<f64>) {
        if samples.is_empty() {
            println!("STATS {tag:<28} no samples");
            return;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = samples.len();
        let pct = |p: f64| samples[((p * (n - 1) as f64).round() as usize).min(n - 1)];
        let mean = samples.iter().sum::<f64>() / n as f64;
        println!(
            "STATS {tag:<28} n={n:<5} min={:<8.2} p50={:<8.2} p95={:<8.2} p99={:<8.2} max={:<8.2} mean={:<8.2} {unit}",
            samples[0],
            pct(0.50),
            pct(0.95),
            pct(0.99),
            samples[n - 1],
            mean
        );
    }
}

/// Drive the harness to completion. This call does not return: tao's `run`
/// consumes the loop and exits the process.
pub fn spin(event_loop: EventLoop<()>, mut harness: Harness) -> ! {
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Poll;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested { .. },
                ..
            } => {
                // The probe owns shutdown; a close request from the window
                // manager must not truncate a run halfway through.
            }
            Event::MainEventsCleared => {
                if harness.tick() {
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    })
}

// ---------------------------------------------------------------------------
// Reusable steps
// ---------------------------------------------------------------------------

/// Print a section header. Every raw line that follows belongs to it.
pub fn mark(title: &str) -> Step {
    let title = title.to_string();
    step(move |_h| {
        println!("\n=== {title} ===");
        Tick::Done
    })
}

/// Do nothing for `ms` of wall clock.
pub fn sleep_ms(ms: u64) -> Step {
    let mut start = 0.0f64;
    step(move |_h| {
        if start == 0.0 {
            start = win::qpc_ms();
        }
        if win::ms_since(start) >= ms as f64 {
            Tick::Done
        } else {
            Tick::Running
        }
    })
}

/// Wait for one new record of `event` on `label`. On timeout the run continues
/// and the timeout is printed — an absent event is a result, not a crash.
pub fn expect_record(label: &str, event: &str, timeout_ms: u64, note: &str) -> Step {
    let (label, event, note) = (label.to_string(), event.to_string(), note.to_string());
    let mut baseline = None::<usize>;
    let mut start = 0.0f64;
    step(move |h| {
        let base = *baseline.get_or_insert_with(|| {
            start = win::qpc_ms();
            hooks::count(&h.sink, &label, &event)
        });
        if hooks::count(&h.sink, &label, &event) > base {
            return Tick::Done;
        }
        if win::ms_since(start) > timeout_ms as f64 {
            println!("TIMEOUT after {timeout_ms} ms: expected {label} {event} ({note})");
            Tick::Done
        } else {
            Tick::Running
        }
    })
}

/// A step that builds a webview and prints the failure instead of aborting the
/// run, so one bad slot does not silently erase the rest of the report.
pub fn build_step(slot: usize, label: &str, url: Option<&str>) -> Step {
    let label = label.to_string();
    let url = url.map(|s| s.to_string());
    step(move |h| match h.build(slot, &label, url.as_deref()) {
        Ok(()) => Tick::Done,
        Err(e) => {
            println!("ERROR {e}");
            Tick::Done
        }
    })
}

/// A step that runs an action which can fail, printing the failure.
pub fn action(name: &str, f: impl FnOnce(&mut Harness) -> Result<(), String> + 'static) -> Step {
    let name = name.to_string();
    let mut f = Some(f);
    step(move |h| {
        if let Some(f) = f.take() {
            if let Err(e) = f(h) {
                println!("ERROR {name}: {e}");
            }
        }
        Tick::Done
    })
}
