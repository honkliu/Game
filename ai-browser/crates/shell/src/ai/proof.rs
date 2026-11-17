//! `--proof`: the Release 0 measurement harness (R0a-8's "the harness is the
//! definition of done", carrying R0a-1's assertions).
//!
//! It is a mode of the shipping binary, not a test: it runs the real app on a real
//! webview, opens real tabs through the same IPC dispatch path a user click uses,
//! and prints one `PASS`/`FAIL` line per budget with the measured number next to
//! it. Any `FAIL` exits non-zero, so `cargo run -p browser -- --proof` is a gate
//! rather than a report.
//!
//! ## Why it rides the driver
//!
//! `--driver=NAME` already scripts commands through `AppState::dispatch` with a
//! settle window per step, which is the only way to measure anything on a webview:
//! navigations commit asynchronously, so driving them in one event-loop iteration
//! measures zero. The proof scenario is therefore a driver script, and the harness
//! takes one measurement per driver step (before that step's command runs) via
//! [`drive`]. That gives a baseline sample before any tab is opened, one settled
//! sample after the last, and the intermediate samples as evidence.
//!
//! ## What it refuses to guess
//!
//! A process whose command line cannot be read is counted as UNCLASSIFIED and makes
//! the browser-process check fail ([`crate::ai::processes::Role::Unclassified`]). A
//! budget that cannot be measured is printed `NOT MEASURED`, never green. And a run
//! that sees no WebView2 browser process at all fails the attribution check (C0)
//! before any count is believed: the runtime gives one browser process per
//! user-data-folder rather than per host process, so an empty tree means the tree
//! belongs to another instance, not that the process cost is zero.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::ai::mode;
use crate::ai::processes;
use crate::sys;
use crate::platform as webview2;
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};

/// The dispatch command the scenario uses to bring the overlay up one step after
/// the baseline, so C5 measures a delta. Handled in `app.rs` only when
/// [`mode::active`], so it is not a command a normal session can invoke.
pub const OVERLAY_STEP: &str = "proof.overlay";

/// Prefix of every harness-internal dispatch command, so `app.rs` can give the
/// whole family a settle window long enough for a webview to be built (the default
/// 30 ms window would sample a half-initialised overlay and C5 would read its cost
/// as roughly zero).
pub const OVERLAY_STEP_PREFIX: &str = "proof.";

/// A harness-internal step that does nothing but spend its settle window, so the
/// run can *wait* without driving anything. Used by `+self_refresh` to watch a
/// background tab for a fixed interval with no commands in flight, and handled in
/// `app.rs` only when [`mode::active`].
pub const WAIT_STEP: &str = "proof.wait";

/// Ceiling on a [`WAIT_STEP`]'s `ms`. The value is chosen by [`scenario`], not by a
/// caller, and `app.rs` caps it on the way through as well — a scenario bug should
/// stall a measurement, not wedge the binary in an un-wakeable wait. It must stay at
/// or above [`WATCH_MS`] or the watch window the report describes would not be the
/// window the run got; a test asserts it.
pub const MAX_WAIT_MS: u64 = 300_000;

/// A page that reloads itself in JavaScript. Found in a real 2.2-hour user session
/// on 2026-11-13: `tab 2 navigated: https://news.qq.com.cn/` was logged twelve times
/// with the tab never activated, at ~1.4 s, 1.9 s, 5 s, 30 s, 60 s, 300 s, 600 s and
/// then 1800 s intervals — capped exponential backoff, and none of it ours (the crate
/// has no retry or backoff logic; reload fires only from an explicit command,
/// F5/Ctrl+R or IPC).
///
/// Shipping behaviour hides whatever this costs, because `sleep_background_tabs`
/// destroys the background webview and its refresh timer with it. `+self_refresh`
/// measures it, and measures it in *both* visibility states — see [`WATCH_MS`] for
/// why one state is not enough.
pub const SELF_REFRESH_PAGE: &str = "https://news.qq.com.cn/";

/// Settle window after the self-refreshing page is opened, so C8's "cost of the
/// loaded tab" delta is taken on a loaded page rather than a spinner.
const BUSY_LOAD_SETTLE_MS: u64 = 12_000;

/// Quiet window between a command and a watch window, so a page still loading from
/// that command cannot be charged to the thing under observation.
const BUSY_QUIET_MS: u64 = 5_000;

/// Length of one command-free watch window. `+self_refresh` runs two of them, the
/// same page for the same length of time, once with the page in the foreground and
/// once with it backgrounded, and reports the committed-navigation count of each.
///
/// That A/B pair is what distinguishes the two explanations for a silent window:
/// either the page's own interval exceeded the window, or Chromium throttles the
/// timers of a page it considers hidden (`AppState::activate` calls
/// `set_visible(false)` on a deactivated tab's webview, which is the state that
/// throttling keys on). The first version of this check watched only the background
/// tab for 180 s, saw zero navigations, and could not say which of the two it had
/// measured.
///
/// 300 s because the observed intervals in the real session run 1.4 s, 1.9 s, 5 s,
/// 30 s, 60 s, 300 s, 600 s, 1800 s: a 300 s window covers everything up to and
/// including the 300 s step. It still cannot see the 600 s and 1800 s tail, and the
/// report says that rather than extrapolating it.
const WATCH_MS: u64 = 300_000;

/// The `tab_id` the self-refreshing tab gets, i.e. the first tab the scenario opens.
/// Deterministic because check C7 fails any run that did not start from a wiped
/// profile, and a wiped profile means exactly one start tab (`id 1`) before the
/// scenario's first `nav.open`.
const BUSY_TAB_ID: u64 = 2;

/// The overlay's cost as measured across an overlay flip, in MB of process-tree
/// working set: the six runs recorded in `ai-browser/tools/baseline.md`
/// (§Pricing the overlay) measured 64.2 / 65.2 / 65.7 / 66.9 / 67.0 / 68.5 from a
/// wiped profile in one mode, i.e. ~66 MB against the plan's 80 MB ceiling. The plan
/// estimated ≈60 MB, which is ~11 % low, so C5 prints this range instead of the
/// estimate; the process-count half of the budget (+1 renderer, +0 browser processes)
/// was exact and is still asserted rather than quoted.
const OVERLAY_MEASURED_MB: [f64; 6] = [64.2, 65.2, 65.7, 66.9, 67.0, 68.5];

/// The measured floor for one background tab kept alive, in MB of WebView2 child
/// working set per tab: the same file's six `+no_tab_sleep` runs came to 97.0–98.2
/// (σ ≈ 0.4) on example.com/.org/.net and iana.org — some of the cheapest pages on the
/// internet. It is a *floor*: a page that refreshes itself costs more (C8), and the
/// shell's own note in `app.rs` puts a real content page at ≈120 MB.
/// `RELEASE-0.md` §Budgets still sets no ceiling for this row, and that gap is the ask;
/// C6 prints both the floor and the fact that no ceiling exists.
const KEPT_TAB_FLOOR_MB: (f64, f64) = (97.0, 98.2);

/// Distinct origins, one per tab the scenario may open.
///
/// Site isolation keys a renderer on the *site instance*, not on the tab, so tabs
/// that share an origin legitimately share a renderer and would measure a renderer
/// delta of zero for any tab count. Every entry is a different registrable host,
/// which is what makes "+N tabs" a statement about processes at all. The list caps
/// `--proof=N` so the caller cannot silently ask for tabs the harness has no
/// distinct origins for.
const PROOF_PAGES: [&str; 8] = [
    "https://example.com/",
    "https://example.org/",
    "https://example.net/",
    "https://www.iana.org/",
    "https://www.mozilla.org/",
    "https://www.chromium.org/",
    "https://www.wikipedia.org/",
    "https://www.rust-lang.org/",
];

/// Tabs the scenario can open, i.e. the length of [`PROOF_PAGES`]. `main.rs` caps
/// `--proof=N` with it so the cap and the page list cannot drift apart.
pub const MAX_TABS: usize = PROOF_PAGES.len();

/// The app-owned numbers a sample records alongside the process counts. Passed by
/// the driver hook because these are the app's own figures — deliberately *not*
/// derived from the process counts, since whether the two agree is under test.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed {
    /// Tabs open in the window.
    pub tabs: usize,
    /// Tabs with a live page webview (`AppState::live`).
    pub live_webviews: usize,
    /// Whether the AI overlay webview exists.
    pub overlay_live: bool,
    /// `AppState::navigation_seq`: committed navigations since startup, including
    /// the ones the page asked for itself. This is the counter behind the log line
    /// `tab N navigated: …`, so it is how C8 proves a background tab really
    /// reloaded instead of sitting idle.
    pub navigations: u64,
}

/// Which part of the run a sample belongs to. `+self_refresh` measures the *same*
/// page in two visibility states, and a count only means something if the sample it
/// came from knows which state it was taken in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    /// Ordinary scripted steps: a command follows every sample.
    #[default]
    Scripted,
    /// Command-free window with the self-refreshing page in the foreground.
    Visible,
    /// Command-free window with the same page backgrounded — `set_visible(false)`,
    /// which is the state Chromium's timer throttling keys on.
    Hidden,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Scripted => "-",
            Phase::Visible => "vis",
            Phase::Hidden => "hid",
        }
    }
}

/// What the *scenario* says about a sample, as distinct from what the app says about
/// itself ([`Observed`]). Built from [`Shape`] rather than from caller-supplied
/// booleans, so two adjacent flags cannot be transposed and a step inserted at the
/// front of the script cannot mislabel the samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Mark {
    phase: Phase,
    /// The self-refreshing tab is already open in this sample.
    busy_tab: bool,
}

/// One measurement, taken immediately before a driver step's command runs.
#[derive(Debug, Clone)]
struct Sample {
    /// Tabs the scenario was asked to open, recorded per sample so the report
    /// describes the run that happened rather than the default.
    requested_tabs: usize,
    tabs: usize,
    live_webviews: usize,
    /// Whether the AI overlay webview existed when this sample was taken. The
    /// overlay is not in `AppState::live` (it is not a tab), so C5's delta needs
    /// its own marker: without it "renderers 3 -> 4" is ambiguous between "the
    /// overlay arrived" and "a page got a second renderer".
    overlay_live: bool,
    /// Committed navigations at sample time, for C8's reload accounting.
    navigations: u64,
    /// Monotonic milliseconds since process start, so a watch window's real length
    /// is reported rather than the length the scenario asked for.
    at_ms: u64,
    /// Which part of the run this sample belongs to. A `Visible` or `Hidden` sample
    /// was taken inside a command-free window: nothing the harness did can explain
    /// what moved between two of them.
    phase: Phase,
    /// The self-refreshing tab had been opened by the time this sample was taken.
    /// C8's "what does that tab cost on its own" delta is this sample against the
    /// last one without it, which is the only pairing that has no other tab in it.
    busy_tab: bool,
    environments: u64,
    browser_processes: usize,
    renderers: usize,
    other_children: usize,
    unclassified: usize,
    processes_enumerated: usize,
    /// `msedgewebview2.exe` processes machine-wide, ours or not. Attribution
    /// evidence: it separates "WebView2 did not start" from "WebView2 started, but
    /// under a different host process".
    webviews_machine_wide: usize,
    /// Other processes running this image. A non-zero count means another host
    /// holds a WebView2 user-data-folder, which is how the tree under our pid can
    /// come up empty while the browser process is alive elsewhere.
    other_hosts: usize,
    tree_mb: f64,
    shell_mb: f64,
    webview_mb: f64,
    renderer_mb: f64,
    browser_mb: f64,
    /// `pid:role` per WebView2 member, for the evidence lines. Captured at sample
    /// time: by the time the report prints, shutdown has already released the
    /// children, so a re-enumeration then would read "no WebView2 processes".
    roles: String,
}

impl Sample {
    fn measure(requested_tabs: usize, observed: &Observed, mark: Mark) -> Sample {
        let tree = processes::measure(sys::current_pid());
        let attribution = processes::attribution(sys::current_pid());
        Sample {
            requested_tabs,
            tabs: observed.tabs,
            live_webviews: observed.live_webviews,
            overlay_live: observed.overlay_live,
            navigations: observed.navigations,
            at_ms: sys::process_uptime_ms(),
            phase: mark.phase,
            busy_tab: mark.busy_tab,
            environments: webview2::environments_constructed(),
            browser_processes: tree.browser_processes(),
            renderers: tree.renderers(),
            other_children: tree.other_children(),
            unclassified: tree.unclassified(),
            processes_enumerated: tree.total_processes,
            webviews_machine_wide: attribution.webviews_machine_wide,
            other_hosts: attribution.other_host_processes,
            tree_mb: processes::round_mb(sys::mb(tree.total_working_set)),
            shell_mb: processes::round_mb(sys::mb(tree.shell_working_set)),
            webview_mb: processes::round_mb(sys::mb(tree.webview_working_set)),
            renderer_mb: tree.working_set_mb_of("renderer"),
            browser_mb: tree.working_set_mb_of("browser"),
            roles: if tree.members.is_empty() {
                "no msedgewebview2.exe in our tree".to_string()
            } else {
                tree.members
                    .iter()
                    .map(|m| format!("{}:{}(p{})", m.pid, m.role.label(), m.parent_pid))
                    .collect::<Vec<_>>()
                    .join(" ")
            },
        }
    }
}

/// Samples in step order. Non-empty only while a proof run is in progress, which is
/// what [`report_and_exit_if_failed`] keys off to stay inert on every other driver
/// scenario.
static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());

/// Set by the last report so the caller can decide an exit code.
static FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    /// The harness could not measure this budget. Reported, never folded into a
    /// pass: an unmeasured budget that prints green is how a budget comes to be
    /// "met".
    NotMeasured,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::NotMeasured => "NOT MEASURED",
        }
    }
}

struct Verdict {
    status: Status,
    check: &'static str,
    detail: String,
}

/// Where the interesting samples of a `+self_refresh` scenario fall. Derived from
/// the same construction that builds the script, so a step added in front of the
/// scenario cannot silently mislabel the samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    /// Index of the step that opens the self-refreshing tab. The sample *at* that
    /// index is taken before the tab exists, so C8's "what does that tab cost on its
    /// own" delta is `samples[busy_before + 2] - samples[busy_before]`: the sample
    /// after the tab's load window minus the sample with no such tab at all, with no
    /// other tab in either one.
    busy_before: usize,
    /// Inclusive sample-index range of each command-free window. The hidden window's
    /// first sample is taken after the static tabs have settled with the busy tab
    /// backgrounded; the visible window's first sample is taken after `tab.select`
    /// made it the foreground tab again and a quiet window let that flip settle.
    hidden: (usize, usize),
    visible: (usize, usize),
}

impl Shape {
    /// Which window a sample index falls in. Anything between the two windows — the
    /// `tab.select` step and its settle — is an ordinary scripted step: the harness
    /// did something there, so movement in it is not the page's doing.
    fn phase_at(&self, idx: usize) -> Phase {
        if self.visible.0 <= idx && idx <= self.visible.1 {
            Phase::Visible
        } else if self.hidden.0 <= idx && idx <= self.hidden.1 {
            Phase::Hidden
        } else {
            Phase::Scripted
        }
    }
}

/// The scenario's step script, and where its samples land. A pure function of the
/// mode so the shell's behaviour and the steps that measure it cannot disagree.
///
/// Under `+self_refresh` the order *is* the experiment: the page is opened, the other
/// tabs take the foreground, one command-free window is watched with the page
/// backgrounded, then `tab.select` puts it back in the foreground — a pure
/// `set_visible` change, since `build_webview` returns early for a tab that already
/// has a webview, so nothing reloads and the page's own backoff clock is untouched —
/// and a second, equally long window is watched. Same page, same tab set, same window
/// length, two visibility states.
///
/// The hidden window goes first on purpose: the page's refresh interval grows with its
/// age, so a silent *hidden* window followed by a *visible* window that does refresh
/// cannot be explained by the interval having outgrown the window. The reverse order
/// would leave that ambiguity open.
fn scenario(
    cfg: &mode::Config,
    pages: &[&'static str],
) -> (Vec<(&'static str, Value)>, Option<Shape>) {
    let mut steps: Vec<(&'static str, Value)> = Vec::new();
    if !cfg.overlay_at_startup {
        // The run started without the overlay (`+no_overlay`), so bringing it up here
        // gives C5 a before/after pair. The command is idempotent in `app.rs`, so a
        // later step re-sending it is harmless and keeps the script a pure function
        // of the scenario name.
        steps.push((OVERLAY_STEP, json!({ "enabled": true })));
    }
    let busy_before = if cfg.self_refresh_tab {
        let at = steps.len();
        steps.push((
            "nav.open",
            json!({ "url": SELF_REFRESH_PAGE, "disposition": "new_tab" }),
        ));
        // Spend a load window before the tabs that follow, so the busy tab's own cost
        // is sampled on a loaded page rather than a spinner.
        steps.push((WAIT_STEP, json!({ "ms": BUSY_LOAD_SETTLE_MS })));
        Some(at)
    } else {
        None
    };
    steps.extend((0..cfg.tabs.min(pages.len())).map(|i| {
        (
            "nav.open",
            json!({ "url": pages[i], "disposition": "new_tab" }),
        )
    }));
    // Nothing to watch without `+self_refresh`: trailing waits would only stall a
    // run that is measuring something else.
    let shape = busy_before.map(|busy_before| {
        // The static tabs above each activated themselves, so the busy tab is
        // backgrounded by now; let them finish loading before anything is timed.
        steps.push((WAIT_STEP, json!({ "ms": BUSY_QUIET_MS })));
        let hidden_start = steps.len();
        steps.push((WAIT_STEP, json!({ "ms": WATCH_MS })));
        // The sample taken at *this* index is the one that closes the window: it is
        // taken after the wait above has been spent, before `tab.select` runs.
        let hidden_end = steps.len();
        // Foreground again. `AppState::activate` only flips visibility on a live tab
        // (`build_webview` returns early when one exists), so the page is not
        // reloaded and its own backoff clock is untouched.
        steps.push(("tab.select", json!({ "tab_id": BUSY_TAB_ID })));
        steps.push((WAIT_STEP, json!({ "ms": BUSY_QUIET_MS })));
        let visible_start = steps.len();
        steps.push((WAIT_STEP, json!({ "ms": WATCH_MS })));
        let visible_end = steps.len();
        Shape {
            busy_before,
            hidden: (hidden_start, hidden_end),
            visible: (visible_start, visible_end),
        }
    });
    (steps, shape)
}

/// Driver hook: record one measurement of the tree as it stands now, then return the
/// scenario's command script.
///
/// Called once per driver step by `AppState::driver_script`, which makes the first
/// call the pre-open baseline and the call after the last command the settled
/// measurement. `observed` comes from the app because those are the app's own
/// numbers, deliberately *not* derived from the process counts — whether the two
/// agree is one of the things under test.
pub fn drive(driver_name: &str, observed: &Observed) -> Vec<(&'static str, Value)> {
    let requested = mode::Config::parse(driver_name, PROOF_PAGES.len());
    let (steps, shape) = scenario(&requested, &PROOF_PAGES);
    // Sample index == step index, because this function is called once per step
    // before that step's command runs (and once more after the last one).
    let mut samples = SAMPLES
        .lock()
        .expect("proof sample lock is not held across a panic boundary");
    let idx = samples.len();
    let mark = shape
        .map(|s| Mark {
            phase: s.phase_at(idx),
            // The sample at `busy_before` is taken before that step's `nav.open`
            // runs, so the tab is only in this sample strictly after it.
            busy_tab: idx > s.busy_before,
        })
        .unwrap_or_default();
    samples.push(Sample::measure(requested.tabs, observed, mark));
    steps
}

/// Print the report, write it to `<data>\proof.txt`, and exit `1` if any check
/// failed. Returns without effect when no proof run is in progress, so it is inert
/// on every other driver path.
///
/// It exits the process instead of returning a code because the app's own exit path
/// ends in `ControlFlow::Exit` inside the event loop, which carries no status code.
/// Call it after `shut_down`, so the WebView2 children have already been released.
pub fn report_and_exit_if_failed() {
    let samples = SAMPLES
        .lock()
        .expect("proof sample lock is not held across a panic boundary")
        .clone();
    if samples.len() < 2 {
        return; // not a proof run, or it never got past its first sample
    }
    let (report, failures) = render(&samples);
    FAILED.store(failures > 0, Ordering::Relaxed);
    println!("{report}");
    let path = crate::app::data_dir().join("proof.txt");
    // The file is not a convenience: a `windows_subsystem = "windows"` build has no
    // attached console, and a gate nobody can read is not a gate.
    let text = format!(
        "{report}\nwritten by: native-browser --proof (the exact command is in the \
         `command` line above)\n"
    );
    if std::fs::write(&path, &text).is_ok() {
        println!("proof: report written to {}", path.display());
    } else {
        println!("proof: could not write {}", path.display());
    }
    if FAILED.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}

/// The report body and the number of failing checks.
fn render(samples: &[Sample]) -> (String, usize) {
    let first = &samples[0];
    let last = samples.last().expect("checked by the caller");
    let peak_browser = samples.iter().map(|s| s.browser_processes).max().unwrap_or(0);
    let max_unclassified = samples.iter().map(|s| s.unclassified).max().unwrap_or(0);
    let n = last.requested_tabs;
    // Tabs the run actually opened, which is `n` plus the self-refreshing page under
    // `+self_refresh`. Any sentence in the report that says "+N tabs" must use this,
    // not the requested count, or the header and the numbers disagree.
    let tabs_opened = last.tabs.saturating_sub(first.tabs);

    // Baseline for anything attributed to *tabs*: the first sample that was already
    // in the same overlay state as the last one. Under `+no_overlay` the overlay is
    // built by step 0, so comparing the tab counts against sample 0 would charge the
    // overlay's renderer and working set to the tabs.
    let base_idx = samples
        .iter()
        .position(|s| s.overlay_live == last.overlay_live)
        .unwrap_or(0);
    let base = &samples[base_idx];
    let after_base = &samples[base_idx..];
    let peak_renderer = after_base.iter().map(|s| s.renderers).max().unwrap_or(0);
    let peak_live = after_base.iter().map(|s| s.live_webviews).max().unwrap_or(0);

    let mut out = String::new();
    out.push_str("PROOF  native-browser Release 0 harness (R0a-1)\n");
    out.push_str(&format!("  machine    {}\n", machine_line()));
    // Which profile produced these numbers. The WebView2 browser process is keyed
    // on this path, so a report without it cannot be reproduced or attributed.
    out.push_str(&format!(
        "  profile    {}\n",
        crate::app::data_dir().join("webview2").display()
    ));
    let cfg = mode::config();
    out.push_str(&format!(
        "  mode       tabs={tabs} tab_sleep={sleep} overlay_at_startup={overlay} \
         self_refresh={refresh} (absolute MB figures from this run are comparable only \
         against a run with the same mode)\n",
        tabs = cfg.tabs,
        sleep = if cfg.keep_background_tabs_alive { "off" } else { "on" },
        overlay = cfg.overlay_at_startup,
        refresh = if cfg.self_refresh_tab { "on" } else { "off" },
    ));
    out.push_str(&format!(
        "  command    cargo run -p browser -- --proof{}   (open {} distinct-origin \
         tabs)\n",
        mode_suffix(&cfg),
        cfg.tabs
    ));
    out.push_str(&format!(
        "  samples    {} (one per driver step)\n",
        samples.len()
    ));
    out.push_str(&format!(
        "  context    {} other host process(es), {} msedgewebview2.exe in total (ours \
         and everybody else's)\n",
        last.other_hosts, last.webviews_machine_wide
    ));
    // Absolute working set depends on how much was already loaded when the run
    // started, so the starting state is printed rather than assumed. Measured:
    // a run that restored 5 tabs read 822.6 MB where a wiped-profile run read
    // 591.6 MB for the same scenario — verdicts identical, absolutes ~40% apart.
    out.push_str(&format!(
        "  start      {} tab(s) at step 0, {} live webview(s), {} overlay; {}\n",
        first.tabs,
        first.live_webviews,
        if first.overlay_live { "built" } else { "absent" },
        if first.tabs == 1 {
            "profile was wiped, so absolute MB are comparable"
        } else {
            "PROFILE NOT WIPED — session state was restored, see C7"
        }
    ));
    out.push_str("  step  tabs  live_wv  ovl phase  navs  env  browser  renderer  other  unclass   tree_MB  shell_MB  webview_MB\n");
    for (i, s) in samples.iter().enumerate() {
        out.push_str(&format!(
            "  {:>4} {:>5} {:>8} {:>4} {:>5} {:>5} {:>4} {:>7} {:>8} {:>5} {:>7} {:>9} {:>9} {:>10}\n",
            i,
            s.tabs,
            s.live_webviews,
            if s.overlay_live { "yes" } else { "no" },
            // Which command-free window this sample belongs to, so a reader can see
            // that the counts C8 quotes were taken with the harness doing nothing.
            s.phase.label(),
            s.navigations,
            s.environments,
            s.browser_processes,
            s.renderers,
            s.other_children,
            s.unclassified,
            s.tree_mb,
            s.shell_mb,
            s.webview_mb
        ));
    }
    out.push_str(&format!(
        "  per-role working set at the last sample: browser {:.1} MB, renderer {:.1} MB\n",
        last.browser_mb, last.renderer_mb
    ));
    out.push_str(&format!("  step 0 WebView2 members  {}\n", first.roles));
    out.push_str(&format!("  last WebView2 members  {}\n", last.roles));

    let mut verdicts: Vec<Verdict> = Vec::new();

    // C0 — attribution, checked before any budget can mean anything. The runtime
    // keys its browser process on the *user-data-folder*, not on the host process,
    // so a native-browser instance launched on a folder another instance already
    // holds ends up with no `msedgewebview2.exe` children at all: the children
    // belong to the older host. Letting "0 browser processes" flow into C2 and C3
    // would report an attribution failure as a measurement. `--proof` therefore
    // takes a private profile (`main.rs::private_proof_profile`) and asserts here
    // that the tree it counted is the tree it caused.
    let min_browser = samples.iter().map(|s| s.browser_processes).min().unwrap_or(0);
    let widest_tree = samples
        .iter()
        .map(|s| s.processes_enumerated)
        .max()
        .unwrap_or(0);
    verdicts.push(Verdict {
        status: if min_browser >= 1 {
            Status::Pass
        } else {
            Status::Fail
        },
        check: "C0 the WebView2 tree counted is the one this run started",
        detail: format!(
            "{} browser process(es) in our tree across {} sample(s); widest tree {} \
             process(es); {} msedgewebview2.exe machine-wide and {} other host \
             process(es) running. An empty tree here means the browser process for \
             this user-data-folder belongs to somebody else{}, not that the cost is \
             zero.",
            min_browser,
            samples.len(),
            widest_tree,
            last.webviews_machine_wide,
            last.other_hosts,
            if widest_tree <= 1 {
                " and our process had no children at all"
            } else {
                ""
            }
        ),
    });

    // C1 — the environment count, read at its construction site.
    verdicts.push(Verdict {
        status: if first.environments == 1 && last.environments == 1 {
            Status::Pass
        } else {
            Status::Fail
        },
        check: "C1 exactly one ICoreWebView2Environment per process lifetime",
        detail: format!(
            "constructed={} sampled before and after opening {tabs_opened} tabs (a second \
             one means a webview was built without with_environment)",
            last.environments
        ),
    });

    // C2 — the check the harness exists for: a WebView2 child is not a browser
    // process, and a second browser process is exactly what an unshared environment
    // costs. Unreadable command lines fail it rather than defaulting either way.
    let browser_delta = peak_browser as isize - first.browser_processes as isize;
    verdicts.push(Verdict {
        status: if max_unclassified > 0 {
            Status::Fail
        } else if peak_browser == 1 && first.browser_processes == 1 && browser_delta == 0 {
            Status::Pass
        } else {
            Status::Fail
        },
        check: "C2 zero extra WebView2 browser processes for +N tabs",
        detail: format!(
            "browser processes {} -> {} (peak {}, delta {:+}) while tabs went {} -> {}; \
             {} process(es) with an unreadable --type= [{}]",
            first.browser_processes,
            last.browser_processes,
            peak_browser,
            browser_delta,
            first.tabs,
            last.tabs,
            max_unclassified,
            last.roles,
        ),
    });

    // C3 — the renderer half of the assertion.
    //
    // Both readings of "+N tabs, +N renderers" are measured here, and which one is
    // possible depends on a shell behaviour the plan did not account for:
    // `AppState::sleep_background_tabs` destroys every non-active tab's webview, so
    // in shipping mode N tabs never means N live pages and the budget has no
    // referent. `--proof=…+no_tab_sleep` (`mode::NO_TAB_SLEEP`) turns that off for
    // the run, which is what makes the plan's sentence measurable. Measured both
    // ways in `tools/baseline.md`; nothing here silently prefers one.
    //
    //   +N live pages and +N renderers              → pass, plan satisfied.
    //   tab sleeping on, so never more than one
    //   live page and no renderer growth            → not measurable, with the flag
    //                                                 that would measure it.
    //   fewer renderers than live pages             → fail: two live pages shared a
    //                                                 renderer, the real defect.
    //   anything else                               → fail, with the numbers.
    let renderer_delta = peak_renderer as isize - base.renderers as isize;
    let live_delta = peak_live as isize - base.live_webviews as isize;
    // How many live pages the run was *supposed* to end with. `+self_refresh` opens
    // one page beyond the requested tab count — that extra tab is the thing under
    // measurement — so "+N tabs, +N renderers" has to be read as "+N pages, +N
    // renderers", or the check would fail a run for the page it was told to add.
    let expected_pages = n as isize + if cfg.self_refresh_tab { 1 } else { 0 };
    // The per-sample invariant behind "no two live pages shared a renderer": at no
    // instant may there be fewer renderer processes than live page webviews.
    let shared_renderer = samples.iter().any(|s| s.renderers < s.live_webviews);
    let sleep_on = mode::tabs_may_sleep();
    let c3_status = if max_unclassified > 0 || shared_renderer {
        Status::Fail
    } else if live_delta == 0 && sleep_on {
        Status::NotMeasured
    } else if renderer_delta == live_delta && live_delta == expected_pages {
        Status::Pass
    } else {
        Status::Fail
    };
    // The interpretation has to track the case the numbers are actually in, or the
    // report says "the shell never had N live pages" on a run where it did.
    let c3_reading = if shared_renderer {
        "FEWER renderer processes than live page webviews in at least one sample: two live \
         pages shared a renderer process, which is the defect this check exists to catch"
            .to_string()
    } else if max_unclassified > 0 {
        "the renderer split is unusable while any command line is unreadable".to_string()
    } else if c3_status == Status::Pass {
        let mut reading = format!(
            "+{pages} live page webview(s) and +{pages} renderer processes: the plan's \
             assertion holds as written",
            pages = expected_pages,
        );
        if cfg.self_refresh_tab {
            reading.push_str(&format!(
                " — {n} requested tab(s) plus the one self-refreshing page `+{}` opens",
                mode::SELF_REFRESH
            ));
        }
        reading
    } else if sleep_on && live_delta == 0 {
        format!(
            "tab sleeping is on (shipping behaviour): the shell held at most {peak_live} \
             live page webview(s) for {tabs} tabs, so the runtime was never handed {n} \
             live pages and \"+{n} renderers\" has no referent in this mode. Measure it \
             with `--proof={want}+{flag}`; what this mode does establish is that no \
             sample ever held fewer renderers than live page webviews, i.e. no sharing.",
            tabs = last.tabs,
            want = cfg.tabs,
            flag = mode::NO_TAB_SLEEP,
        )
    } else if sleep_on {
        format!(
            "the shell presented {peak_live} live page webview(s) (delta {live_delta:+}) \
             while tab sleeping was still on, and renderers moved by {renderer_delta:+}, \
             so live pages and renderers did not track each other. Re-run with \
             `--proof={want}+{flag}` so the tab count and the live-page count are one \
             number before drawing a conclusion.",
            want = cfg.tabs,
            flag = mode::NO_TAB_SLEEP,
        )
    } else {
        format!(
            "the shell presented {peak_live} live page webview(s) (delta {live_delta:+}) \
             but renderers moved by {renderer_delta:+} with tab sleeping already off — \
             this is not the sleep-behaviour caveat, it is a mismatch to investigate"
        )
    };
    verdicts.push(Verdict {
        status: c3_status,
        check: "C3 +N tabs add exactly +N renderer processes",
        detail: format!(
            "renderers {} -> {} (peak {}, peak delta {:+}, final delta {:+}) while tabs \
             went {} -> {} (+{tabs_opened}) and live page webviews went {} -> {} (peak {}, delta \
             {:+}), measured from step {base_idx}; {}",
            base.renderers,
            last.renderers,
            peak_renderer,
            renderer_delta,
            last.renderers as isize - base.renderers as isize,
            base.tabs,
            last.tabs,
            base.live_webviews,
            last.live_webviews,
            peak_live,
            live_delta,
            c3_reading,
        ),
    });

    // C4 — the memory number every later budget is a delta against.
    let other_mb = last.tree_mb - last.shell_mb - last.webview_mb;
    verdicts.push(Verdict {
        status: if last.tree_mb > 0.0 && last.shell_mb > 0.0 && last.processes_enumerated > 0 {
            Status::Pass
        } else {
            Status::Fail
        },
        check: "C4 process-tree working set is measurable",
        detail: format!(
            "tree {:.1} MB = shell {:.1} + WebView2 children {:.1} + other {:.1}, over \
             {} processes ({} live webviews, {} tabs)",
            last.tree_mb,
            last.shell_mb,
            last.webview_mb,
            // The three terms are each rounded from bytes, so their sum can sit a
            // hair over the tree total; a "−0.0 MB" would read as a leak.
            processes::round_mb(other_mb.max(0.0)),
            last.processes_enumerated,
            last.live_webviews,
            last.tabs,
        ),
    });

    // C5 — the overlay's own cost, as a delta.
    //
    // The plan budgets the overlay at +1 renderer and +0 browser processes, a
    // prediction that came out exact; its ≈60 MB working-set estimate did not — the
    // six runs in [`OVERLAY_MEASURED_MB`] measured ~66 MB against an 80 MB ceiling, so
    // the detail prints the measured range and this run's own delta. That is only
    // measurable if the overlay appears between two samples; with `mode::NO_OVERLAY`
    // the run starts without it and step 0 builds it, so the before/after pair exists.
    // Without the flag the overlay is in every sample and the check says so instead of
    // guessing.
    let overlay_flip = samples
        .windows(2)
        .enumerate()
        .find(|(_, pair)| pair[0].overlay_live != pair[1].overlay_live);
    let (c5_status, c5_detail) = match overlay_flip {
        None => (
            Status::NotMeasured,
            format!(
                "no sample pair differs in overlay presence, so the overlay was built at \
                 startup and is inside every sample ({} live webview(s) at step 0 = \
                 shell + overlay + one page). Run `--proof={}+{}` to measure its delta.",
                first.live_webviews, cfg.tabs, mode::NO_OVERLAY
            ),
        ),
        Some((i, _)) => {
            let before = &samples[i];
            let after = &samples[i + 1];
            let built = after.overlay_live;
            // Sign-normalised so the report always reads as "what the overlay cost".
            let sign = if built { 1 } else { -1 };
            let renderer_delta = sign * (after.renderers as isize - before.renderers as isize);
            let browser_delta = sign * (after.browser_processes as isize - before.browser_processes as isize);
            let tree_delta = sign as f64 * (after.tree_mb - before.tree_mb);
            let status = if max_unclassified > 0 {
                Status::Fail
            } else if browser_delta != 0 {
                // The failure the budget exists to catch: the overlay silently got its
                // own ICoreWebView2Environment, which is a whole browser process.
                Status::Fail
            } else if renderer_delta == 0 || renderer_delta == 1 {
                Status::Pass
            } else {
                Status::Fail
            };
            let reading = match renderer_delta {
                1 => "exactly +1 renderer, as budgeted".to_string(),
                0 => "0 renderers: the overlay shared a renderer process with the shell's \
                      own asset:// UI instead of getting one, so the plan's '+1 renderer' \
                      overstates it — cheaper, and nothing got a second browser process"
                    .to_string(),
                other => format!("{other} renderers for one overlay webview — more than the \
                                 budget allows, investigate"),
            };
            let (measured_lo, measured_hi) = OVERLAY_MEASURED_MB.iter().fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(lo, hi), mb| (lo.min(*mb), hi.max(*mb)),
            );
            let measured_mean =
                OVERLAY_MEASURED_MB.iter().sum::<f64>() / OVERLAY_MEASURED_MB.len() as f64;
            (
                status,
                format!(
                    "overlay {} between steps {i} and {} ({} -> {} live webview(s) in the \
                     shell): renderers {} -> {} (delta {:+}), browser processes {} -> {} \
                     (delta {:+}), tree {:.1} -> {:.1} MB, a delta of {:+.1} MB on this \
                     run. The same delta has measured {lo:.1}-{hi:.1} MB over {runs} \
                     wiped-profile runs (mean {mean:.1} MB, so ~66 MB) against the 80 MB \
                     ceiling (`ai-browser/tools/baseline.md` §Pricing the overlay); the \
                     plan's ≈60 MB estimate was ~11 % low, its process counts were exact. \
                     {reading}. Working-set deltas carry page-load noise; the process \
                     counts do not.",
                    if built { "built" } else { "destroyed" },
                    i + 1,
                    before.live_webviews,
                    after.live_webviews,
                    before.renderers,
                    after.renderers,
                    renderer_delta,
                    before.browser_processes,
                    after.browser_processes,
                    browser_delta,
                    before.tree_mb,
                    after.tree_mb,
                    tree_delta,
                    lo = measured_lo,
                    hi = measured_hi,
                    runs = OVERLAY_MEASURED_MB.len(),
                    mean = measured_mean,
                ),
            )
        }
    };
    verdicts.push(Verdict {
        status: c5_status,
        check: "C5 overlay adds no browser process and a bounded renderer cost",
        detail: c5_detail,
    });

    // C6 — what keeping background tabs alive costs, which is the number the
    // product decision needs and which shipping behaviour hides by design: tab
    // sleeping keeps one live page, so the delta per extra live webview only exists
    // under `mode::NO_TAB_SLEEP`. No budget for this exists in RELEASE-0.md, so the
    // verdict is "did we get a number", and the number is the deliverable.
    let per_live_webview = if live_delta > 0 {
        Some(processes::round_mb(
            (last.webview_mb - base.webview_mb) / live_delta as f64,
        ))
    } else {
        None
    };
    // The same delta split by renderer rather than by webview, because the plan's
    // per-renderer estimate is written per *process*. Both readings are printed
    // instead of one being derived by the reader from numbers the report omits.
    let renderer_added = last.renderers as isize - base.renderers as isize;
    let per_renderer = if renderer_added > 0 {
        Some((last.renderer_mb - base.renderer_mb) / renderer_added as f64)
    } else {
        None
    };
    verdicts.push(Verdict {
        status: match per_live_webview {
            Some(_) => Status::Pass,
            None => Status::NotMeasured,
        },
        check: "C6 cost of a background tab kept alive (measured floor; no ceiling set)",
        detail: match per_live_webview {
            Some(per) => format!(
                "live page webviews {live_delta:+} ({base_live} -> {last_live}) while \
                 WebView2 children went {base_mb:.1} -> {last_mb:.1} MB: {per:.1} MB per \
                 tab kept alive, {run_mb:+.1} MB for the run; renderer processes \
                 {per_renderers}. This is the price of not sleeping background tabs. What \
                 is still unset is the ceiling — `RELEASE-0.md` §Budgets carries this \
                 row as an explicit ask rather than a limit — but the floor is measured: \
                 {floor_lo:.1}-{floor_hi:.1} MB per kept-alive tab on some of the cheapest \
                 pages on the internet (example.com/.org/.net, iana.org), i.e. \
                 {eight_lo:.0}-{eight_hi:.0} MB for 8 tabs and {twenty_lo:.0}-{twenty_hi:.0} \
                 MB ({gb:.1} GB) for 20 tabs of WebView2 children, against the ≈120 MB \
                 `app.rs` itself notes for a real content page. Quote the floor as the best \
                 case, not the price.{floor}",
                floor_lo = KEPT_TAB_FLOOR_MB.0,
                floor_hi = KEPT_TAB_FLOOR_MB.1,
                eight_lo = KEPT_TAB_FLOOR_MB.0 * 8.0,
                eight_hi = KEPT_TAB_FLOOR_MB.1 * 8.0,
                twenty_lo = KEPT_TAB_FLOOR_MB.0 * 20.0,
                twenty_hi = KEPT_TAB_FLOOR_MB.1 * 20.0,
                gb = (KEPT_TAB_FLOOR_MB.0 + KEPT_TAB_FLOOR_MB.1) * 10.0 / 1024.0,
                base_live = base.live_webviews,
                last_live = last.live_webviews,
                base_mb = base.webview_mb,
                last_mb = last.webview_mb,
                run_mb = last.webview_mb - base.webview_mb,
                floor = if cfg.self_refresh_tab {
                    format!(
                        " The divisor includes the self-refreshing tab C8 exists to watch, \
                         so this is a floor: quote the plain `+{}` run for the price of an \
                         idle kept-alive tab.",
                        mode::NO_TAB_SLEEP
                    )
                } else {
                    String::new()
                },
                per_renderers = match per_renderer {
                    Some(pr) => format!(
                        "{base_renderers} -> {last_renderers} holding {base_rmb:.1} -> \
                         {last_rmb:.1} MB, {pr:.1} MB per renderer added",
                        base_renderers = base.renderers,
                        last_renderers = last.renderers,
                        base_rmb = base.renderer_mb,
                        last_rmb = last.renderer_mb,
                    ),
                    None => format!(
                        "{base_renderers} -> {last_renderers} holding {base_rmb:.1} -> \
                         {last_rmb:.1} MB",
                        base_renderers = base.renderers,
                        last_renderers = last.renderers,
                        base_rmb = base.renderer_mb,
                        last_rmb = last.renderer_mb,
                    ),
                },
            ),
            None => format!(
                "no live-webview growth to divide by: tab sleeping kept the run at {} \
                 live page webview(s). Run `--proof={}+{}` for the number.",
                last.live_webviews, cfg.tabs, mode::NO_TAB_SLEEP
            ),
        },
    });

    // C7 — comparability. A run that restored a previous session measured 822.6 MB
    // where the same scenario from a wiped profile measured 591.6 MB: same verdicts,
    // different absolutes. The harness wipes its own private profile at startup
    // (`main.rs`), so anything other than one tab at step 0 means the wipe did not
    // happen or something wrote session state — either way the absolute MB in this
    // report must not be quoted.
    verdicts.push(Verdict {
        status: if first.tabs == 1 && first.live_webviews <= 1 {
            Status::Pass
        } else {
            Status::Fail
        },
        check: "C7 the run started from a wiped profile (one tab, no restored state)",
        detail: format!(
            "{} tab(s) and {} live webview(s) at step 0 (expected 1 and 1). If this fails, \
             delete the harness profile before the next run — \
             `rmdir /s /q \"%LOCALAPPDATA%\\native-browser-proof\"` — and treat every \
             absolute MB in this report as incomparable; per-role working set and deltas \
             across samples of one run stay valid.",
            first.tabs, first.live_webviews
        ),
    });

    // One command-free window's measured contents. `None` when the run has no such
    // window (or only its opening sample), which is what makes the checks below say
    // "not measured" instead of dividing by nothing.
    struct Window {
        secs: f64,
        navs: u64,
        mb: f64,
        live: usize,
        renderer_delta: isize,
        browser_delta: isize,
    }
    let window_of = |phase: Phase| -> Option<Window> {
        let first = samples.iter().position(|s| s.phase == phase)?;
        let last = samples.iter().rposition(|s| s.phase == phase)?;
        if last == first {
            return None;
        }
        let (a, b) = (&samples[first], &samples[last]);
        Some(Window {
            secs: b.at_ms.saturating_sub(a.at_ms) as f64 / 1000.0,
            navs: b.navigations.saturating_sub(a.navigations),
            mb: b.webview_mb - a.webview_mb,
            live: b.live_webviews,
            renderer_delta: b.renderers as isize - a.renderers as isize,
            browser_delta: b.browser_processes as isize - a.browser_processes as isize,
        })
    };
    let hidden = window_of(Phase::Hidden);
    let visible = window_of(Phase::Visible);

    // (a) of C8: what the self-refreshing tab costs by itself. `busy_tab` marks the
    // samples that already contain it, so the first of them minus the sample before it
    // is the only pairing in the run where that tab is the only thing the scenario
    // opened, and `+ 1` is the sample taken after its load window — a loaded page
    // rather than a spinner.
    let busy_line = samples
        .iter()
        .position(|s| s.busy_tab)
        .and_then(|first| {
            // `checked_sub` covers the "it was the very first sample" case, in which
            // there is no earlier sample to diff against.
            let before = samples.get(first.checked_sub(1)?)?;
            let loaded = samples.get(first + 1)?;
            Some(format!(
                "(a) the page on its own, while it was the only tab the scenario had \
                 opened: {mb:+.1} MB of WebView2 children ({from:.1} -> {to:.1}) and \
                 {renderers:+} renderer process(es), between steps {before_step} and \
                 {loaded_step}.",
                mb = loaded.webview_mb - before.webview_mb,
                from = before.webview_mb,
                to = loaded.webview_mb,
                renderers = loaded.renderers as isize - before.renderers as isize,
                before_step = first - 1,
                loaded_step = first + 1,
            ))
        });

    // C8 — does a self-refreshing page refresh at all while it is in the background,
    // and what does it cost when it does? Shipping behaviour hides the answer:
    // `sleep_background_tabs` destroys the background webview and its refresh timer
    // with it, so C6's per-tab figure is a floor rather than the price of a real tab.
    //
    // The scenario watches the *same* page for the *same* length of time in both
    // visibility states, because a single silent background window has two opposite
    // explanations and only the pair separates them:
    //
    //   * the page's own interval simply exceeded the window (its backoff in the real
    //     session reaches 1800 s), or
    //   * the page never gets to run, because a deactivated tab's webview is
    //     `set_visible(false)` and Chromium throttles a page it considers hidden.
    //
    // Background-silent plus foreground-refreshing is the second; silence in both is
    // the first. Either way the refresh cost is not measurable from that run, so the
    // verdict is NOT MEASURED with the two counts, never a pass: the number the
    // product decision needs only exists when a background navigation is actually
    // observed, and no page was substituted or driven to make one appear.
    let (c8_status, c8_detail) = match (&hidden, &visible, &busy_line) {
        (None, _, _) => (
            Status::NotMeasured,
            format!(
                "no command-free window was taken, so nothing here prices a \
                 self-refreshing page and C6's per-tab number stays a floor. Run \
                 `--proof={tabs}+{sleep}+{refresh}` (adds ~2x{watch}s); the tab-sleep flag \
                 is not optional, because a slept background tab has no webview left to \
                 refresh, which is precisely what shipping behaviour throws away.",
                tabs = cfg.tabs,
                sleep = mode::NO_TAB_SLEEP,
                refresh = mode::SELF_REFRESH,
                watch = WATCH_MS / 1000,
            ),
        ),
        // The scenario asked to be watched but the shell slept its background tabs, so
        // there was no webview left for a page to refresh in.
        _ if last.live_webviews < 2 => (
            Status::NotMeasured,
            format!(
                "the watch windows ran with {live} live page webview(s) for {tabs} tab(s), \
                 so the background tab had no webview left for a page to refresh in: tab \
                 sleeping was on. Run `--proof={want}+{sleep}+{refresh}`.",
                live = last.live_webviews,
                tabs = last.tabs,
                want = cfg.tabs,
                sleep = mode::NO_TAB_SLEEP,
                refresh = mode::SELF_REFRESH,
            ),
        ),
        (Some(_), _, None) => (
            Status::Fail,
            "watch windows were taken but no sample is labelled as containing the \
             self-refreshing tab, so the scenario builder and the reporter disagree about \
             what ran and nothing in those windows can be attributed"
                .to_string(),
        ),
        (Some(h), v, Some(busy)) => {
            let shown = |name: &str, w: Option<&Window>| match w {
                None => format!("{name} window: not sampled"),
                Some(w) => format!(
                    "{name} window: {navs} self-navigation(s) in {secs:.0} s, WebView2 \
                     children {mb:+.1} MB ({mb_per_min:+.1} MB/min across {live} live \
                     page webview(s)), renderers {renderers:+}, browser processes \
                     {browser:+}",
                    navs = w.navs,
                    secs = w.secs,
                    mb = w.mb,
                    mb_per_min = if w.secs > 0.0 { w.mb / (w.secs / 60.0) } else { 0.0 },
                    live = w.live,
                    renderers = w.renderer_delta,
                    browser = w.browser_delta,
                ),
            };
            let both = format!(
                "{busy} (b) {hidden}. (c) {visible}",
                hidden = shown("background", Some(h)),
                visible = shown("foreground", v.as_ref()),
            );
            match (h.navs, v.as_ref().map(|w| w.navs)) {
                // A background navigation was observed: this is the price.
                (n, _) if n > 0 => (
                    Status::Pass,
                    format!(
                        "{both} (d) the page reloaded itself {n} time(s) in the background \
                         window, so that window is the price: {per_nav:.1} MB of WebView2 \
                         working set per self-initiated reload and {per_min:+.1} MB/min \
                         while it sits there. The page's own intervals in the real session \
                         run 1.4 s to 1800 s, so a long-lived background tab pays this \
                         repeatedly; no budget for it exists in RELEASE-0.md.",
                        both = both,
                        n = h.navs,
                        per_nav = h.mb / h.navs as f64,
                        per_min = if h.secs > 0.0 { h.mb / (h.secs / 60.0) } else { 0.0 },
                    ),
                ),
                // Only the background arm was sampled, so the pair that separates the
                // two explanations does not exist and neither can be claimed.
                (_, None) => (
                    Status::NotMeasured,
                    format!(
                        "{both} (d) only the background window was sampled, so there is no \
                         visible-state control to compare it against: a silent background \
                         window is then equally consistent with the page's interval \
                         exceeding {secs:.0} s and with hidden-tab timer throttling, and \
                         this check refuses to pick one. Re-run \
                         `--proof={tabs}+{sleep}+{refresh}`.",
                        both = both,
                        secs = h.secs,
                        tabs = cfg.tabs,
                        sleep = mode::NO_TAB_SLEEP,
                        refresh = mode::SELF_REFRESH,
                    ),
                ),
                // Foreground refreshed, background did not: throttling, not timing.
                (0, Some(vn)) if vn > 0 => (
                    Status::NotMeasured,
                    format!(
                        "{both} (d) VERDICT: throttling (hypothesis B) — the same page \
                         committed {vn} navigation(s) while it was the visible tab and 0 \
                         while it was backgrounded for the same {secs:.0} s, so a background \
                         tab is not refreshing today: its page's timers never get to \
                         schedule the reload (`AppState::activate` hides a deactivated \
                         tab's webview with `set_visible(false)`, which is the state \
                         Chromium's timer throttling keys on). There is therefore no \
                         self-refresh cost to price in background tabs, and this check \
                         reports none rather than converting the idle window's drift into \
                         one; the number that survives is C9's idle drift. If shipping \
                         behaviour ever lets a hidden tab keep refreshing, background \
                         navigations appearing in this line's (b) count are the trigger to \
                         re-price it.",
                        both = both,
                        vn = vn,
                        secs = h.secs,
                    ),
                ),
                // Silent in both states: the page's interval exceeded the window.
                _ => (
                    Status::NotMeasured,
                    format!(
                        "{both} (d) VERDICT: silent in both states (hypothesis A) — 0 \
                         navigations while visible and 0 while hidden for the same \
                         {secs:.0} s each, so the page's own interval exceeded the window, \
                         which is a timing limit of the harness rather than a property of \
                         the shell (the real session's backoff steps 1.4 s, 1.9 s, 5 s, \
                         30 s, 60 s, 300 s, 600 s, 1800 s). To price this check the \
                         scenario needs either a page that refreshes faster than the window \
                         or a longer window; the window was lengthened to {watch} s per arm \
                         and no page was substituted, because pointing C8 at a page this \
                         repo controls, or driving a refresh from the harness, is the \
                         manufactured pass the check exists to refuse. {watch} s covers \
                         every documented interval up to 300 s and stops there: covering \
                         the 600 s and 1800 s steps would make the run tens of minutes for \
                         a number a browser restart already bounds. NOT MEASURED, not a \
                         price.",
                        both = both,
                        secs = h.secs,
                        watch = WATCH_MS / 1000,
                    ),
                ),
            }
        }
    };
    verdicts.push(Verdict {
        status: c8_status,
        check: "C8 cost of a self-refreshing background tab kept alive (no budget set; \
                number only)",
        detail: c8_detail,
    });

    // C9 — what live tabs cost by existing, with the harness doing nothing at all.
    // Separate from C8 on purpose: the per-reload number only exists if a page
    // refreshes, while this one is paid by every kept-alive tab whether or not any of
    // them moves, which is the quantity that matters to someone with twenty tabs open.
    // Each arm reports its own navigation count so a reader can see which of the two
    // drifts is pure idle.
    let (c9_status, c9_detail) = match (&hidden, &visible) {
        (None, None) => (
            Status::NotMeasured,
            format!(
                "no command-free window in this run, so nothing was left idle long enough \
                 to measure. Run `--proof={tabs}+{sleep}+{refresh}`.",
                tabs = cfg.tabs,
                sleep = mode::NO_TAB_SLEEP,
                refresh = mode::SELF_REFRESH,
            ),
        ),
        _ => {
            let arm = |name: &str, w: Option<&Window>| -> Option<(String, f64)> {
                let w = w?;
                let per_min = if w.secs > 0.0 { w.mb / (w.secs / 60.0) } else { 0.0 };
                let per_webview = if w.live > 0 { per_min / w.live as f64 } else { 0.0 };
                Some((
                    format!(
                        "{name} {secs:.0} s with {live} live page webview(s) and {navs} \
                         navigation(s): {mb:+.1} MB = {per_min:+.1} MB/min \
                         ({per_webview:+.2} MB/min per webview)",
                        secs = w.secs,
                        live = w.live,
                        navs = w.navs,
                        mb = w.mb,
                        per_min = per_min,
                        per_webview = per_webview,
                    ),
                    per_webview,
                ))
            };
            let arms: Vec<(String, f64)> =
                [arm("background", hidden.as_ref()), arm("foreground", visible.as_ref())]
                    .into_iter()
                    .flatten()
                    .collect();
            let quiet = hidden.iter().chain(visible.iter()).map(|w| w.navs).sum::<u64>();
            // One quotable figure rather than one per arm: every arm watches the same tab
            // count, so the per-live-webview rate is averaged over whichever arms ran. The
            // match arm above guarantees at least one, so the divisor is never zero.
            let idle_per_webview =
                arms.iter().map(|(_, rate)| rate).sum::<f64>() / arms.len() as f64;
            (
                Status::Pass,
                format!(
                    "{arms}. {quiet} navigation(s) were committed across {n_arms} \
                     command-free window(s), so the drift is not navigation work{suffix}. \
                     Combined: {idle:+.2} MB/min per live page webview. This is what \
                     kept-alive tabs cost by existing rather than by reloading — the figure \
                     a twenty-tab user pays on every tab they are not looking at, and \
                     nothing in RELEASE-0.md budgets for it.",
                    arms = arms
                        .iter()
                        .map(|(text, _)| text.as_str())
                        .collect::<Vec<_>>()
                        .join("; "),
                    n_arms = arms.len(),
                    quiet = quiet,
                    idle = idle_per_webview,
                    suffix = if quiet == 0 {
                        " at all — the number is pure idle drift, the one nobody has".to_string()
                    } else {
                        String::new()
                    },
                ),
            )
        }
    };
    verdicts.push(Verdict {
        status: c9_status,
        check: "C9 idle drift of background tabs kept alive, no navigation required \
                (no budget set; number only)",
        detail: c9_detail,
    });

    let failures = verdicts.iter().filter(|v| v.status == Status::Fail).count();
    out.push('\n');
    for v in &verdicts {
        out.push_str(&format!("{}  {} — {}\n", v.status.label(), v.check, v.detail));
    }
    out.push_str(&format!(
        "\nRESULT {} of {} checks failed{}\n",
        failures,
        verdicts.len(),
        if failures == 0 { "" } else { " (exit 1)" }
    ));
    (out, failures)
}

/// The command line that produced this run, echoed so a report can be replayed.
fn mode_suffix(cfg: &mode::Config) -> String {
    let mut suffix = String::new();
    if cfg.tabs != mode::DEFAULT_TABS {
        suffix.push_str(&format!("={}", cfg.tabs));
    }
    let mut flags = Vec::new();
    if cfg.keep_background_tabs_alive {
        flags.push(mode::NO_TAB_SLEEP);
    }
    if !cfg.overlay_at_startup {
        flags.push(mode::NO_OVERLAY);
    }
    if cfg.self_refresh_tab {
        flags.push(mode::SELF_REFRESH);
    }
    for flag in flags {
        suffix.push('+');
        suffix.push_str(flag);
    }
    // `--proof=x` and `--proof+x` both parse; prefer the bare form when the tab
    // count is the default so the echoed command is the shortest true one.
    if suffix.starts_with('+') {
        suffix.insert(0, '=');
    }
    suffix
}

/// OS-reported machine capacity. The CPU brand string has no dependency-free Win32
/// call, so `tools/baseline.md` records it from the shell command noted there rather
/// than having the harness guess at it.
#[cfg(windows)]
fn machine_line() -> String {
    // SAFETY: both calls write through an out-parameter the caller owns;
    // `MEMORYSTATUSEX` requires `dwLength` set first, done below.
    unsafe {
        let mut info = SYSTEM_INFO::default();
        GetSystemInfo(&mut info);
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        let ram = if GlobalMemoryStatusEx(&mut status).is_ok() {
            format!("{:.0} GiB RAM", sys::mb(status.ullTotalPhys as u64) / 1024.0)
        } else {
            "RAM unreadable".to_string()
        };
        format!("{} logical CPUs, {}", info.dwNumberOfProcessors, ram)
    }
}

#[cfg(target_os = "macos")]
fn machine_line() -> String {
    format!("{} logical CPUs (WebView2 budgets are Windows-only)",
        std::thread::available_parallelism().map_or(0, |n| n.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the app would report at a given moment. `Sample::measure` takes the
    /// app's own figures rather than deriving them, so the tests hand it over the
    /// same way `app.rs` does.
    fn observed(tabs: usize, live: usize, overlay_live: bool, navigations: u64) -> Observed {
        Observed {
            tabs,
            live_webviews: live,
            overlay_live,
            navigations,
        }
    }

    /// A baseline sample with a WebView2 tree in it. `Sample::measure` reads the
    /// real tree, which in a unit test is empty, so the counts the checks reason
    /// about are written explicitly: what is under test is the verdict, not the
    /// enumeration (that has its own tests in `ai::processes`).
    fn baseline() -> Sample {
        baseline_with(true)
    }

    /// Same, with the overlay state chosen, because C5 measures a transition and
    /// C3's baseline is "the first sample already in the last sample's overlay
    /// state".
    fn baseline_with(overlay_live: bool) -> Sample {
        let mut s = Sample::measure(2, &observed(1, 1, overlay_live, 0), Mark::default());
        s.environments = 1;
        s.browser_processes = 1;
        s.renderers = 3;
        s.other_children = 4;
        s.unclassified = 0;
        s.processes_enumerated = 9;
        s.webviews_machine_wide = 37;
        s.other_hosts = 0;
        s
    }

    /// The branch a shipping-behaviour run falls into: the shell slept every
    /// background tab, so it never had `N` live pages and the renderer count could
    /// not grow by `N`. Reported as not measurable, not as a pass and not as a
    /// regression, and with the flag that would make it measurable.
    #[test]
    fn c3_is_not_measured_when_the_shell_never_held_the_tabs_live() {
        let a = baseline();
        let mut b = a.clone();
        b.tabs = 3; // +2 tabs, but live webviews stay at 1 and renderers stay flat
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C3 ")).expect("C3 line");
        assert!(line.starts_with("NOT MEASURED"), "{line}");
        assert!(line.contains("has no referent"), "{line}");
        assert!(line.contains(mode::NO_TAB_SLEEP), "{line}");
        // Not measurable is not a failure: nothing got worse on this path.
        assert_eq!(failures, 0, "{report}");
    }

    /// The case C3 exists to catch, kept testable even though this build's shipping
    /// behaviour cannot reach it: live pages grew and renderers did not follow. That
    /// must be a FAIL, not the "not measurable" answer above.
    #[test]
    fn c3_fails_when_live_pages_grew_but_renderers_did_not_follow() {
        let a = baseline();
        let mut b = a.clone();
        b.tabs = 3;
        b.live_webviews = 3; // the shell really kept all three pages live
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C3 ")).expect("C3 line");
        assert!(line.starts_with("FAIL"), "{line}");
        assert!(line.contains("did not track each other"), "{line}");
        assert!(failures > 0);
    }

    /// The per-sample invariant, spelled out: if a sample ever holds fewer
    /// renderer processes than live page webviews, two pages shared one.
    #[test]
    fn c3_fails_when_a_sample_held_fewer_renderers_than_live_pages() {
        let a = baseline();
        let mut b = a.clone();
        b.tabs = 4;
        b.live_webviews = 4;
        b.renderers = 3; // four live pages, three renderer processes
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C3 ")).expect("C3 line");
        assert!(line.starts_with("FAIL"), "{line}");
        assert!(line.contains("shared a renderer process"), "{line}");
        assert!(failures > 0);
    }

    /// And the plan's own happy path, so the check is demonstrably capable of
    /// passing rather than being structurally lenient: every live page that arrived
    /// brought its own renderer process.
    #[test]
    fn c3_passes_when_each_live_page_brought_its_own_renderer() {
        let a = baseline();
        let mut b = a.clone();
        b.tabs = 3;
        b.live_webviews = 3;
        b.renderers = a.renderers + 2;
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C3 ")).expect("C3 line");
        assert!(line.starts_with("PASS"), "{line}");
        assert!(line.contains("holds as written"), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// Attribution comes first: a run that saw no WebView2 browser process of its
    /// own must not be able to report a green C2, and must say whose process it is
    /// instead of printing a bare zero. This is the shared-user-data-folder case
    /// measured on this machine, reproduced without a browser.
    #[test]
    fn an_empty_tree_fails_attribution_rather_than_reading_as_zero_cost() {
        let a = Sample::measure(4, &observed(1, 1, true, 0), Mark::default());
        let mut empty = a.clone();
        empty.browser_processes = 0;
        empty.renderers = 0;
        empty.other_children = 0;
        empty.processes_enumerated = 1;
        empty.webviews_machine_wide = 37;
        empty.other_hosts = 1;
        let mut after = empty.clone();
        after.tabs = 5;
        let (report, failures) = render(&[empty, after]);
        let line = report.lines().find(|l| l.contains("C0 ")).expect("C0 line");
        assert!(line.starts_with("FAIL"), "{line}");
        assert!(line.contains("37 msedgewebview2.exe machine-wide"), "{line}");
        assert!(line.contains("1 other host"), "{line}");
        assert!(failures > 0);
    }

    /// The scenario asks for at most one tab per distinct origin, never more: a tab
    /// that shares an origin with another cannot add a renderer, so an uncapped `N`
    /// would fail the renderer assertion for a reason that has nothing to do with the
    /// browser. The cap is [`MAX_TABS`], the length of the page list, so a caller
    /// cannot ask for tabs the harness has no distinct origins for.
    #[test]
    fn tab_count_defaults_to_four_and_is_capped_at_the_distinct_origins() {
        let parse = |name: &str| mode::Config::parse(name, MAX_TABS).tabs;
        assert_eq!(parse("proof"), mode::DEFAULT_TABS);
        assert_eq!(parse("proof=2"), 2);
        assert_eq!(parse("proof:6"), 6);
        assert_eq!(parse("proof:999"), MAX_TABS);
        assert_eq!(parse("proof:0"), 1);
        assert_eq!(parse("proof=2+no_tab_sleep"), 2, "flags do not eat the count");
        // A non-numeric token is reported rather than silently degraded into the
        // default run (`mode`'s own tests cover the flag side).
        assert_eq!(
            mode::Config::parse("proof:notanumber", MAX_TABS).unknown,
            vec!["notanumber".to_string()]
        );
    }

    #[test]
    fn every_proof_page_is_on_its_own_host() {
        let mut hosts: Vec<&str> = PROOF_PAGES
            .iter()
            .map(|url| url.split('/').nth(2).unwrap_or_default())
            .collect();
        let total = hosts.len();
        hosts.sort_unstable();
        hosts.dedup();
        assert_eq!(hosts.len(), total, "two proof pages share a host and a renderer");
    }

    /// Two samples of the live test process: the harness must produce a report and
    /// must not claim a WebView2 tree exists in a unit test. This also pins the
    /// report's shape without needing a browser.
    #[test]
    fn report_renders_every_check_from_two_samples() {
        let a = Sample::measure(4, &observed(1, 3, true, 0), Mark::default());
        let mut b = a.clone();
        b.tabs = 5;
        let (report, failures) = render(&[a, b]);
        for check in ["C0 ", "C1 ", "C2 ", "C3 ", "C4 ", "C5 ", "C6 ", "C7 ", "C8 ", "C9 "] {
            assert!(report.contains(check), "report is missing the {check} line");
        }
        assert!(report.contains("RESULT"), "{report}");
        // C2/C3 cannot pass here: there is no WebView2 tree in a unit test, so the
        // harness must not print them green.
        assert!(report.contains("NOT MEASURED"), "{report}");
        assert!(failures > 0, "a run with no browser process must not report 0 failures");
        // The empty tree has to be *named* as an attribution failure, not just
        // counted as a low number: this is the case that would otherwise read as
        // "0 browser processes, cheap!".
        assert!(report.contains("belongs to somebody else"), "{report}");
    }

    /// C5 with the flag it needs: a sample pair that differs only in the overlay
    /// webview's existence prices the overlay, and the pair must be reported as
    /// "+1 renderer, +0 browser process" rather than as an absolute count.
    #[test]
    fn c5_prices_the_overlay_as_a_delta_between_two_samples() {
        let mut a = baseline_with(false);
        a.tree_mb = 500.0;
        a.webview_mb = 400.0;
        a.renderers = 3;
        let mut b = a.clone();
        b.overlay_live = true;
        b.renderers = 4;
        b.tree_mb = 560.0;
        b.webview_mb = 460.0;
        // Tabs only move after the overlay exists, so C3 is measured from sample 1
        // and is not charged for the overlay.
        let mut c = b.clone();
        c.tabs = 3;
        c.live_webviews = 3;
        c.renderers = 6;
        c.tree_mb = 700.0;
        c.webview_mb = 600.0;
        let (report, failures) = render(&[a, b, c]);
        let line = report.lines().find(|l| l.contains("C5 ")).expect("C5 line");
        assert!(line.starts_with("PASS"), "{line}");
        assert!(line.contains("exactly +1 renderer"), "{line}");
        assert!(line.contains("browser processes 1 -> 1 (delta +0)"), "{line}");
        assert!(line.contains("+60.0 MB"), "{line}");
        // And C3 measured from the post-overlay sample, not from sample 0: the
        // overlay's renderer must not be read as a tab's renderer.
        let c3 = report.lines().find(|l| l.contains("C3 ")).expect("C3 line");
        assert!(c3.starts_with("PASS"), "{c3}");
        assert!(c3.contains("renderers 4 -> 6"), "{c3}");
        assert_eq!(failures, 0, "{report}");
    }

    /// The failure C5 exists to catch: the overlay got its own
    /// `ICoreWebView2Environment`, which shows up as a second browser process.
    #[test]
    fn c5_fails_when_the_overlay_brought_a_second_browser_process() {
        let a = baseline_with(false);
        let mut b = a.clone();
        b.overlay_live = true;
        b.renderers = 4;
        b.browser_processes = 2;
        let mut c = b.clone();
        c.tabs = 3;
        c.live_webviews = 3;
        c.renderers = 6;
        let (report, failures) = render(&[a, b, c]);
        let line = report.lines().find(|l| l.contains("C5 ")).expect("C5 line");
        assert!(line.starts_with("FAIL"), "{line}");
        assert!(failures > 0);
    }

    /// C6 is the number the product decision needs: megabytes per background tab
    /// kept alive, which only exists when tab sleeping was turned off for the run,
    /// plus the same delta read per renderer process.
    #[test]
    fn c6_reports_megabytes_per_background_tab_kept_alive() {
        let mut a = baseline();
        a.webview_mb = 400.0;
        a.renderer_mb = 200.0;
        let mut b = a.clone();
        b.tabs = 3;
        b.live_webviews = 3;
        b.renderers = 5;
        b.webview_mb = 640.0;
        b.renderer_mb = 440.0;
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C6 ")).expect("C6 line");
        assert!(line.starts_with("PASS"), "{line}");
        assert!(line.contains("120.0 MB per tab kept alive"), "{line}");
        assert!(line.contains("120.0 MB per renderer added"), "{line}");
        assert!(line.contains("+240.0 MB for the run"), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// The two figures a reader is most likely to misjudge are the plan's estimates, so
    /// both cost lines print what those estimates have since been measured at, beside
    /// this run's own delta: the overlay's ~66 MB against its 80 MB ceiling, and the
    /// kept-alive tab's measured floor against the ceiling that still does not exist.
    /// Pinned here so the ≈60 MB estimate cannot quietly come back looking like a
    /// measurement, and so the extrapolation stays derived from the recorded floor
    /// rather than retyped per run.
    #[test]
    fn the_cost_lines_quote_measured_figures_rather_than_the_plans_estimates() {
        let mut without = baseline_with(false);
        without.tree_mb = 500.0;
        without.webview_mb = 400.0;
        without.renderers = 3;
        let mut with_overlay = without.clone();
        with_overlay.overlay_live = true;
        with_overlay.renderers = 4;
        with_overlay.tree_mb = 560.0;
        with_overlay.webview_mb = 460.0;
        let mut two_tabs = with_overlay.clone();
        two_tabs.tabs = 3;
        two_tabs.live_webviews = 3;
        two_tabs.renderers = 6;
        two_tabs.webview_mb = 700.0;
        let (report, failures) = render(&[without, with_overlay, two_tabs]);
        assert_eq!(failures, 0, "{report}");

        let c5 = report.lines().find(|l| l.contains("C5 ")).expect("C5 line");
        assert!(c5.contains("a delta of +60.0 MB on this run"), "{c5}");
        assert!(c5.contains("64.2-68.5 MB over 6 wiped-profile runs"), "{c5}");
        assert!(c5.contains("the 80 MB"), "{c5}");
        assert!(!c5.contains("against the ≈60 MB budget"), "{c5}");

        let c6 = report.lines().find(|l| l.contains("C6 ")).expect("C6 line");
        assert!(c6.contains("97.0-98.2 MB per kept-alive tab"), "{c6}");
        assert!(c6.contains("776-786 MB for 8 tabs"), "{c6}");
        // The unset ceiling stays the ask, and stays worded as an ask rather than as a
        // limit the run could be checked against.
        assert!(c6.contains("still unset is the ceiling"), "{c6}");
        assert!(c6.contains("explicit ask"), "{c6}");
    }

    /// C7 is the reproducibility gate. A run that restored a previous session
    /// reported the same verdicts and different absolutes (measured: 822.6 MB vs
    /// 591.6 MB of tree), so it must fail rather than silently publish numbers that
    /// cannot be compared with the baseline.
    #[test]
    fn a_restored_session_fails_the_comparability_check() {
        let mut a = baseline();
        a.tabs = 9;
        a.live_webviews = 5;
        let mut b = a.clone();
        b.tabs = 13;
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C7 ")).expect("C7 line");
        assert!(line.starts_with("FAIL"), "{line}");
        assert!(line.contains("native-browser-proof"), "{line}");
        assert!(report.contains("PROFILE NOT WIPED"), "{report}");
        assert!(failures > 0);
    }

    /// The scenario script *is* the toggles: with the overlay already live there is
    /// no overlay step, and with `+no_overlay` the first step brings it up so C5 has
    /// a before/after pair. Measured through the same entry point `app.rs` calls.
    #[test]
    fn the_scenario_script_is_the_toggles_it_was_asked_for() {
        let plain = drive("proof:2", &observed(1, 1, true, 0));
        assert_eq!(plain.len(), 2, "one step per tab, no overlay step");
        assert!(plain.iter().all(|(cmd, _)| *cmd == "nav.open"));

        let overlay = drive("proof:2+no_overlay", &observed(1, 1, false, 0));
        assert_eq!(overlay.len(), 3, "the overlay step plus one per tab");
        assert_eq!(overlay[0].0, OVERLAY_STEP);
        assert_eq!(overlay[0].1, json!({ "enabled": true }));
        assert_eq!(overlay[1].0, "nav.open");
    }

    /// The `+self_refresh` layout *is* the A/B experiment: the busy tab is opened
    /// before the static ones so it ends up in the background without a "deactivate"
    /// command, it gets its own load window, then the run stops issuing commands for a
    /// window with it backgrounded, brings it to the foreground with one `tab.select`,
    /// and stops again for an equally long window. `Shape` comes out of the same
    /// construction that built the steps, so the sample labels the report reads cannot
    /// drift from what actually ran.
    #[test]
    fn the_self_refresh_scenario_watches_one_page_in_both_visibility_states() {
        let cfg = mode::Config::parse("proof:2+no_tab_sleep+self_refresh", MAX_TABS);
        let (steps, shape) = scenario(&cfg, &PROOF_PAGES);
        let shape = shape.expect("+self_refresh names a shape");

        // Busy tab first, and it is the self-refreshing page, opened so it takes the
        // foreground and hands it straight back to the tabs that follow.
        let busy = shape.busy_before;
        assert_eq!(steps[busy].0, "nav.open");
        assert_eq!(steps[busy].1["url"], json!(SELF_REFRESH_PAGE));
        assert_eq!(steps[busy].1["disposition"], json!("new_tab"));
        // Then a load window, so its own cost is sampled on a loaded page.
        assert_eq!(steps[busy + 1].0, WAIT_STEP);
        assert_eq!(steps[busy + 1].1, json!({ "ms": BUSY_LOAD_SETTLE_MS }));
        // Then the static tabs, whose activation is what leaves the busy tab
        // backgrounded for everything that follows.
        assert_eq!(steps[busy + 2].1["url"], json!(PROOF_PAGES[0]));
        assert_eq!(steps[busy + 3].1["url"], json!(PROOF_PAGES[1]));

        // The windows run in order, do not overlap, and the foreground one is last.
        assert!(shape.hidden.0 < shape.hidden.1);
        assert!(shape.hidden.1 < shape.visible.0);
        assert!(shape.visible.0 < shape.visible.1);
        // The sample at a window's end index is the *next* step's pre-command sample:
        // the hidden window is closed by the `tab.select` that flips visibility, and
        // the visible one by the sample after the run's last step.
        assert_eq!(steps[shape.hidden.1].0, "tab.select");
        assert_eq!(steps[shape.hidden.1].1, json!({ "tab_id": BUSY_TAB_ID }));
        assert_eq!(shape.visible.1, steps.len());
        // Neither window includes the steps that move the page between them: the
        // settle before it and the `tab.select` itself are scripted steps, so anything
        // the harness did there is not counted as the page's own doing.
        assert_eq!(steps[shape.hidden.0 - 1].0, WAIT_STEP);
        assert_eq!(steps[shape.hidden.0 - 1].1, json!({ "ms": BUSY_QUIET_MS }));
        assert_eq!(steps[shape.hidden.1 + 1].1, json!({ "ms": BUSY_QUIET_MS }));

        // The control of the A/B test: the same wait, twice. If these ever differ the
        // experiment no longer compares like with like.
        assert_eq!(steps[shape.hidden.0], steps[shape.visible.0]);
        assert_eq!(steps[shape.hidden.0].1, json!({ "ms": WATCH_MS }));
        // Sample labelling follows the ranges rather than a hand-maintained index.
        assert_eq!(shape.phase_at(shape.hidden.0), Phase::Hidden);
        assert_eq!(shape.phase_at(shape.hidden.1), Phase::Hidden);
        assert_eq!(shape.phase_at(shape.hidden.1 + 1), Phase::Scripted);
        assert_eq!(shape.phase_at(shape.visible.0), Phase::Visible);
        assert_eq!(shape.phase_at(busy), Phase::Scripted);
        // Every wait the scenario asks for is inside the cap `app.rs` applies, or the
        // window it reports would not be the window it got.
        assert!(steps
            .iter()
            .filter(|(cmd, _)| *cmd == WAIT_STEP)
            .all(|(_, p)| p["ms"].as_u64().unwrap_or(u64::MAX) <= MAX_WAIT_MS));
        assert!(WATCH_MS <= MAX_WAIT_MS);
    }

    /// Without the flag there is nothing to watch and no reason to stall a run that
    /// is measuring something else.
    #[test]
    fn without_the_flag_the_scenario_has_no_waits_and_no_shape() {
        let (steps, shape) = scenario(
            &mode::Config::parse("proof:2+no_overlay", MAX_TABS),
            &PROOF_PAGES,
        );
        assert!(shape.is_none());
        assert_eq!(steps.len(), 3);
        assert!(!steps.iter().any(|(cmd, _)| *cmd == WAIT_STEP));
    }

    /// C8 without the mode that produces it: say so and name the command, never
    /// green and never the static-page number by default.
    #[test]
    fn c8_is_not_measured_without_a_watch_window() {
        let a = baseline();
        let mut b = a.clone();
        b.tabs = 3;
        b.live_webviews = 3;
        b.renderers = 5;
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C8 ")).expect("C8 line");
        assert!(line.starts_with("NOT MEASURED"), "{line}");
        assert!(line.contains(mode::SELF_REFRESH), "{line}");
        assert!(line.contains(mode::NO_TAB_SLEEP), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// Both states silent means the page's own interval exceeded the window, which is
    /// a limit of the harness rather than of the shell. Its growth must not be
    /// published as the cost of a self-refreshing tab — that is the flattering number
    /// this check exists to refuse — and the reason chosen (a longer window, never a
    /// substituted page) has to be in the line.
    #[test]
    fn c8_reports_both_states_silent_as_a_window_shorter_than_the_interval() {
        let (report, failures) = render(&self_refresh_samples(0, 0));
        let line = report.lines().find(|l| l.contains("C8 ")).expect("C8 line");
        assert!(line.starts_with("NOT MEASURED"), "{line}");
        assert!(line.contains("silent in both states"), "{line}");
        assert!(line.contains("0 self-navigation(s) in 180 s"), "{line}");
        assert!(line.contains("no page was substituted"), "{line}");
        // Not a price: no megabyte-per-reload figure may appear.
        assert!(!line.contains("per self-initiated reload"), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// The foreground state refreshed and the background one did not: that is hidden
    /// tab timer throttling, not the interval being too long, and it inverts the
    /// product conclusion — so it gets its own verdict instead of a green tick, and
    /// names the mechanism rather than asserting a cause.
    #[test]
    fn c8_reports_throttling_when_only_the_visible_state_refreshed() {
        let (report, failures) = render(&self_refresh_samples(0, 3));
        let line = report.lines().find(|l| l.contains("C8 ")).expect("C8 line");
        assert!(line.starts_with("NOT MEASURED"), "{line}");
        assert!(line.contains("throttling"), "{line}");
        assert!(line.contains("3 navigation(s) while it was the visible tab"), "{line}");
        assert!(line.contains("set_visible(false)"), "{line}");
        // Still not a price, and still not a failure of the shell.
        assert!(!line.contains("per self-initiated reload"), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// The number Carol asked for: megabytes per minute and per reload of a
    /// background tab that refreshes itself, next to what the same tab costs loaded
    /// and idle. Rate arithmetic checked against hand-computed values (120 MB over
    /// 180 s = 40 MB/min; 120 MB over 4 reloads = 30 MB each).
    #[test]
    fn c8_prices_a_self_refreshing_background_tab_per_minute_and_per_reload() {
        let (report, failures) = render(&self_refresh_samples(4, 4));
        let line = report.lines().find(|l| l.contains("C8 ")).expect("C8 line");
        assert!(line.starts_with("PASS"), "{line}");
        assert!(line.contains("4 self-navigation(s) in 180 s"), "{line}");
        assert!(line.contains("+120.0 MB"), "{line}");
        assert!(line.contains("+40.0 MB/min"), "{line}");
        assert!(line.contains("30.0 MB of WebView2 working set per self-initiated reload"), "{line}");
        // (a) is the tab on its own: +100 MB between the sample with no such tab and
        // the sample after its load window, while it is the only tab open.
        assert!(line.contains("+100.0 MB of WebView2 children"), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// Idle drift is reported whether or not a page ever refreshed, which is the whole
    /// reason it is a separate check: the per-reload number above only exists in the
    /// PASS case, while this one is paid by every kept-alive tab.
    #[test]
    fn c9_reports_the_idle_drift_even_when_nothing_refreshed() {
        let (report, failures) = render(&self_refresh_samples(0, 0));
        let line = report.lines().find(|l| l.contains("C9 ")).expect("C9 line");
        assert!(line.starts_with("PASS"), "{line}");
        assert!(line.contains("0 navigation(s) were committed across 2 command-free window(s)"), "{line}");
        assert!(line.contains("pure idle drift"), "{line}");
        // 120 MB over 180 s is 40 MB/min across 4 live page webviews = 10 MB/min each.
        assert!(line.contains("+40.0 MB/min"), "{line}");
        assert!(line.contains("+10.00 MB/min per live page webview"), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// Without command-free windows there is no idle window to measure, and saying so
    /// beats printing the scripted steps' growth as drift.
    #[test]
    fn c9_is_not_measured_without_a_command_free_window() {
        let a = baseline();
        let mut b = a.clone();
        b.tabs = 3;
        b.live_webviews = 3;
        b.renderers = 5;
        let (report, failures) = render(&[a, b]);
        let line = report.lines().find(|l| l.contains("C9 ")).expect("C9 line");
        assert!(line.starts_with("NOT MEASURED"), "{line}");
        assert!(line.contains(mode::SELF_REFRESH), "{line}");
        assert_eq!(failures, 0, "{report}");
    }

    /// Samples of a `+self_refresh` run laid out the way [`scenario`] builds it: the
    /// busy tab opened (webview created, page still loading), its load window spent
    /// (page loaded, still the only tab, so its own cost is isolated), then two
    /// command-free windows — `Phase::Hidden` first, `Phase::Visible` second — each
    /// 180 s and +120 MB of WebView2 children, `hidden_navs` / `visible_navs` being
    /// the navigations the page committed inside each. Those figures make every rate
    /// in the report checkable by hand: 120 MB / 3 min = 40 MB/min, / 4 live webviews
    /// = 10 MB/min each, 120 MB / 4 reloads = 30 MB.
    fn self_refresh_samples(hidden_navs: u64, visible_navs: u64) -> Vec<Sample> {
        let mut opened = baseline();
        opened.requested_tabs = 3;
        opened.webview_mb = 400.0;
        opened.tree_mb = 500.0;
        opened.navigations = 1;
        // Sample taken right after the busy tab's `nav.open`: the webview exists, the
        // page has not committed yet.
        let mut created = opened.clone();
        created.busy_tab = true;
        created.tabs = 2;
        created.live_webviews = 2;
        created.renderers = 4;
        created.webview_mb = 420.0;
        created.tree_mb = 520.0;
        // Sample taken after its load window: still the scenario's only tab, so the
        // delta against `opened` is that tab's own cost (+100 MB here).
        let mut loaded = created.clone();
        loaded.webview_mb = 500.0;
        loaded.tree_mb = 600.0;
        loaded.navigations = 2;
        let mut samples = vec![opened, created, loaded.clone()];
        for (phase, navs, at_ms) in [
            (Phase::Hidden, hidden_navs, 20_000_u64),
            (Phase::Visible, visible_navs, 400_000_u64),
        ] {
            let mut before = loaded.clone();
            before.phase = phase;
            before.tabs = 4;
            before.live_webviews = 4;
            before.renderers = 6;
            before.at_ms = at_ms;
            before.webview_mb = 600.0;
            before.tree_mb = 700.0;
            let mut after = before.clone();
            after.at_ms = at_ms + 180_000;
            after.webview_mb = 720.0;
            after.tree_mb = 820.0;
            after.navigations = before.navigations + navs;
            samples.extend([before, after]);
        }
        samples
    }

    /// The scenario asked to be watched but the shell slept the background tabs, so
    /// there was no webview left for a page to refresh in. Not a price, and not a
    /// pass: a number for something that never happened.
    #[test]
    fn c8_is_not_measured_when_tab_sleeping_ate_the_page_it_was_watching() {
        let mut samples = self_refresh_samples(4, 4);
        for s in &mut samples {
            if s.phase != Phase::Scripted {
                s.live_webviews = 1; // only the foreground tab kept its webview
            }
        }
        let (report, _) = render(&samples);
        let line = report.lines().find(|l| l.contains("C8 ")).expect("C8 line");
        assert!(line.starts_with("NOT MEASURED"), "{line}");
        assert!(line.contains("no webview left for a page to refresh in"), "{line}");
        assert!(line.contains(mode::NO_TAB_SLEEP), "{line}");
        // Not a pass, and not a price: the growth in that window belongs to a page
        // that was no longer there.
        assert!(!line.contains("MB/min"), "{line}");
    }

    /// The report echoes the command that produced it, so a number in
    /// `tools/baseline.md` can be replayed without guessing the flags.
    #[test]
    fn the_echoed_command_names_the_mode_that_ran() {
        assert_eq!(mode_suffix(&mode::Config::default()), "");
        let counted = mode::Config::parse("proof:6", MAX_TABS);
        assert_eq!(mode_suffix(&counted), "=6");
        let both = mode::Config::parse("proof:6+no_tab_sleep+no_overlay", MAX_TABS);
        assert_eq!(mode_suffix(&both), "=6+no_tab_sleep+no_overlay");
        let flags_only = mode::Config::parse("proof+no_tab_sleep", MAX_TABS);
        assert_eq!(
            mode_suffix(&flags_only), "=+no_tab_sleep",
            "a flag needs the `=` separator after a bare `--proof`"
        );
    }
}
