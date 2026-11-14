//! V2 — what the overlay controller costs.
//!
//! The shape being measured is taken from the shipping code, not invented:
//! `native-browser` builds its overlay with a second `build_as_child` call in the
//! *same* top-level window (`app.rs:2388`, `with_transparent(true)`, shared
//! environment) and repositions it with `WebView::set_bounds` (`app.rs:649`). The
//! questions with numbers in them are therefore:
//!
//! 1. how many MiB does adding that controller cost — a delta over the whole process
//!   tree against a live page, not an absolute;
//! 2. how long between a rect the renderer reports and the card being visibly
//!    repainted on screen, idle and during a fling, for the two ways a card can
//!    follow: `set_bounds` on its own controller, versus a fixed full-client overlay
//!    that translates the card inside itself;
//! 3. how far behind the anchor the card sits during a fling;
//! 4. what `SetWindowPos` on a second top-level window costs — the alternative this
//!    architecture is compared against.
//!
//! ## Why these latencies need no clock calibration
//!
//! A sample starts when the page's rect message lands on the host (host QPC) and ends
//! when the pixel probe first sees the card's leading edge at the place it was told to
//! occupy (host QPC). One clock, so there is no `performance.now()`-to-QPC alignment
//! step and no uncertainty carried in from it. The renderer clock is used for one thing
//! only — the page↔host message round trip in V2.4, which bounds the "renderer reported
//! a rect" leg — and is reported separately.
//!
//! ## What `rect→painted` includes
//!
//! WebView2's page→host delivery, the host's own call, controller layout, the
//! compositor, the DWM present, and the poll granularity of the readback. V2.3 measures
//! that last component, so a total can be read as "upper bound with this resolution"
//! rather than as a measurement of the true present.
//!
//! ## Why the probe looks at a leading edge and not the card centre
//!
//! The card is 300×140; during a fling it moves a dozen pixels a frame, so its centre
//! pixel is red for ten frames running and cannot time a paint. The pixel just inside
//! the edge the card is moving towards was not red before the move, so it can. Each
//! sample verifies that the probe pixel was clear *before* the move and counts itself
//! unattributable if it was not, rather than reporting a latency it did not observe.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::harness::{action, expect_record, mark, sleep_ms, step, Harness, Step, Tick};
use crate::win;

/// The card the overlay draws. Same size under both follow strategies so both are
/// measured with one instrument looking for the same colour.
const CARD_W: i32 = 300;
const CARD_H: i32 = 140;

/// How far successive idle samples move the card, logical px. A sample that moves
/// nothing measures nothing. Kept small enough that both positions of the pair stay
/// inside a 660-logical-px-tall client: a card moved off the window cannot repaint
/// where the probe is looking, and that showed up as timeouts, not as a latency.
const IDLE_JUMP: i32 = 150;

/// Scroll-driven follow speeds, CSS px per renderer frame: a slow drag, a wheel notch, a
/// hard fling, and one fast enough that 100 px of error would need a dropped frame. The
/// follow error turned out to be a property of scroll velocity rather than of the overlay
/// mechanism, so one speed is one point on the line the claim is about; four bracket it.
/// The last is short by construction — 480 px of travel at 120 px a frame is four frames,
/// which the run says out loud.
const FLING_SPEEDS: [i32; 4] = [4, 12, 40, 120];

const PAGE_VIEW: usize = 0;
const OVERLAY_VIEW: usize = 1;
/// Hidden borderless top-level, used only to price `SetWindowPos`.
const SCRATCH_WINDOW: usize = 1;

pub struct Cfg {
    pub samples: usize,
    pub site: String,
    /// Scroll distance of one fling, CSS px. It has to fit in the viewport: the anchor
    /// starts at the viewport top and travels down as the page scrolls up, and past
    /// `client height − card height` it leaves the screen, where Chromium is free to skip
    /// the paint work being measured. 480 of a 660 px client with a 140 px card fits.
    pub fling: i32,
}

impl Default for Cfg {
    fn default() -> Self {
        Self {
            samples: 120,
            site: "https://example.com/".to_string(),
            fling: 480,
        }
    }
}

/// Shared across sections. `Rc<RefCell>` and not a mutex because steps, harness and the
/// COM handlers all run on the one UI thread, and taking a lock inside a sub-millisecond
/// measured interval would put the lock in the number.
#[derive(Default)]
struct Acc {
    rss: Vec<(String, f64)>,
    idle_bounds_ms: Vec<f64>,
    idle_css_ms: Vec<f64>,
    /// Same two idle runs, timed instead to the overlay's own paint ack: a lower bound
    /// on rect→painted that needs no screen readback, so it survives a machine where
    /// the readback resolves 85 ms. Empty for `set_bounds`, whose move at constant size
    /// is not observable from inside the overlay document at all.
    ///
    /// `idle_css_ack_ms` is the frame whose computed style is the new one;
    /// `idle_css_commit_ms` is one renderer frame later, after that frame was committed.
    /// The difference between them is the ack instrument's own delay, printed rather
    /// than hidden inside the latency.
    idle_css_ack_ms: Vec<f64>,
    idle_css_commit_ms: Vec<f64>,
    setbounds_us: Vec<f64>,
    csspost_us: Vec<f64>,
    setwindowpos_us: Vec<f64>,
    rtt_ms: Vec<f64>,
    /// Every follow-error series this run produced, labelled with the strategy and the
    /// scroll speed it was measured at. The claim under test is a range, and a range is
    /// only tested by more than one speed.
    err_series: Vec<(String, Vec<f64>)>,
    /// Same series, but the pixel-derived rect→painted latency instead of the follow
    /// error, so the fling results print in one loop.
    paint_series: Vec<(String, Vec<f64>)>,
    probe_transitions: usize,
    probe_expected: usize,
    probe_gaps: Vec<f64>,
    probe_ok: bool,
    /// Whether one pixel read is cheap enough to sit inside a 16 ms frame. The idle
    /// runs tolerate a coarse read (they just quantise the answer); the fling runs
    /// cannot, because a read slower than the frame makes the instrument the slowest
    /// thing in the loop and manufactures the very follow error being measured.
    probe_fits_frame: bool,
    /// Cost of one read in ms, carried to the verdict so every pixel number states it.
    probe_read_ms: f64,
    /// Samples the probe could not attribute (probe pixel already red).
    unattributable: usize,
    /// Rects whose paint was never observed before the run ended.
    dropped: usize,
    /// CSS px moved per second during the fling, from the page's own report.
    fling_velocity: f64,
    dpr: f64,
    scale: f64,
    /// Which readback served the pixel timings, and what one read costs. Printed in
    /// the verdict so a window-surface latency is not read as an on-screen one.
    instrument: String,
}

type Shared = Rc<RefCell<Acc>>;

// ---------------------------------------------------------------------------
// Page-side scripts
// ---------------------------------------------------------------------------

/// Installed into the live page: a tall filler so the document can be flung, two
/// absolutely positioned anchors, and the rect reporter. The document stays a real
/// site; the probe adds three nodes, which the report states rather than hides.
///
/// The two anchors are at different document heights on purpose. `#probe-anchor` is
/// inside the *first* viewport because the idle samples aim the card at its rect, and a
/// card aimed at a rect below the fold is a card aimed off the window — which the first
/// runs of this measurement reported as timeouts, not as a latency.
/// `#probe-fling-anchor` is one viewport below the fold and the fling is driven by
/// scrolling *up* to it, so its viewport rect travels down the screen and stays inside
/// the viewport for the whole run. An anchor that scrolls off the top would let the
/// renderer skip the card's paint work entirely, and the latency measured there is not
/// the latency of a card the user can see.
fn page_setup_js() -> String {
    r#"(function(){
  if (window.__probeReady) return 'already';
  var f = document.createElement('div');
  f.id = 'probe-filler';
  f.style.cssText = 'height:6000px;width:10px';
  document.body.appendChild(f);
  var a = document.createElement('div');
  a.id = 'probe-anchor';
  a.style.cssText = 'position:absolute;left:120px;top:200px;width:120px;height:40px;background:rgb(0,128,255)';
  document.body.appendChild(a);
  var b = document.createElement('div');
  b.id = 'probe-fling-anchor';
  b.style.cssText = 'position:absolute;left:120px;top:' + Math.round(window.innerHeight *
    1.5) + 'px;width:120px;height:40px;background:rgb(0,200,0)';
  document.body.appendChild(b);
  window.__rectOf = function(id){
    var r = document.getElementById(id).getBoundingClientRect();
    return {x: r.left, y: r.top, w: r.width, h: r.height};
  };
  window.__anchorRect = function(){ return window.__rectOf('probe-anchor'); };
  window.__flingRect = function(){ return window.__rectOf('probe-fling-anchor'); };
  window.__flingStart = function(){
    return document.getElementById('probe-fling-anchor').offsetTop;
  };
  window.__postRect = function(seq, dy){
    var r = window.__anchorRect();
    window.chrome.webview.postMessage(JSON.stringify({
      kind: 'anchor', seq: seq, x: r.x, y: r.y + (dy || 0), w: r.w, h: r.h,
      page: performance.now(), dpr: window.devicePixelRatio, scroll: window.scrollY
    }));
    return 'posted';
  };
  window.__probeReady = true;
  return 'ready';
})()"#.to_string()
}

/// One driven scroll for the fling measurement: *and the scroll itself*.
///
/// The scroll is driven from the main thread one step per frame rather than with
/// `scrollBy({behavior:'smooth'})`: a smooth scroll is animated on the compositor
/// thread, and while it runs the main thread's `getBoundingClientRect()` keeps
/// returning the pre-scroll position — measured on the first run, where 22 frames of
/// a 1800 px fling all reported the same anchor rect. A main-thread scroll makes the
/// reported rect the position this frame actually shows, and gives the measurement a
/// known per-frame velocity, which is the quantity the follow error is about.
///
/// It scrolls *upward* from the fling anchor's own document offset, i.e. the anchor
/// travels down the screen. Scrolling downward instead walks the anchor off the top of
/// the viewport within a few frames, and a card the renderer has scrolled out of the
/// viewport is a card whose paint work Chromium is free to skip — the latency measured
/// there belongs to a card the user cannot see.
fn page_follow_js(distance: i32, step_px: i32) -> String {
    format!(
        r#"(function(){{
  var y0 = window.scrollY, cur = y0, n = 0, frames = 0;
  var t0 = performance.now();
  window.__fl = {{on: true}};
  var stepFn = function(){{
    if (!window.__fl.on) return;
    frames++;
    var r = window.__flingRect();
    window.chrome.webview.postMessage(JSON.stringify({{
      kind: 'anchor', seq: n++, x: r.x, y: r.y, w: r.w, h: r.h,
      page: performance.now(), scroll: window.scrollY, moved: y0 - window.scrollY
    }}));
    if (y0 - cur >= {distance}) {{
      window.__fl.on = false;
      window.chrome.webview.postMessage(JSON.stringify({{
        kind: 'fling_end', scroll: y0 - cur, frames: frames,
        ms: performance.now() - t0
      }}));
      return;
    }}
    cur = Math.max(0, cur - {step_px});
    window.scrollTo(0, cur);
    requestAnimationFrame(stepFn);
  }};
  requestAnimationFrame(stepFn);
  return 'driving';
}})()"#,
        distance = distance,
        step_px = step_px
    )
}

/// Host→page→host channel cost, plus the page-side click logger that V3 relies on.
fn page_echo_js() -> String {
    r#"(function(){
  if (window.__echoHook) return 'already';
  window.chrome.webview.addEventListener('message', function(e){
    var m = e.data;
    if (m && m.kind === 'ping') {
      window.chrome.webview.postMessage(JSON.stringify({kind: 'pong', seq: m.seq, page: performance.now()}));
    }
  });
  document.addEventListener('mousedown', function(e){
    window.chrome.webview.postMessage(JSON.stringify({
      kind: 'page_mousedown', x: e.clientX, y: e.clientY,
      target: (e.target && (e.target.id || e.target.tagName)) || '?',
      page: performance.now()
    }));
  }, true);
  window.__echoHook = true;
  return 'echo-hooked';
})()"#.to_string()
}

/// The overlay document: transparent root, one opaque card filling the viewport (so a
/// `set_bounds` move moves the card), the transform path for strategy B, and the flash
/// mode the probe calibration uses.
fn overlay_setup_js() -> String {
    r#"(function(){
  document.documentElement.style.cssText = 'background:transparent;margin:0';
  document.body.style.cssText = 'margin:0;padding:0;background:transparent;overflow:hidden;width:100%;height:100%';
  var card = document.createElement('div');
  card.id = 'probe-card';
  card.style.cssText = 'position:absolute;left:0;top:0;width:100%;height:100%;background:rgb(255,0,0)';
  document.body.appendChild(card);
  window.__card = card;
  window.__flash = function(on, every){
    window.__flashOn = on;
    window.__flashEvery = every || 2;
    var n = 0;
    var tick = function(){
      if (!window.__flashOn) return;
      card.style.background = ((n++ % window.__flashEvery) === 0) ? 'rgb(255,0,0)' : 'rgb(255,255,255)';
      if (n % 30 === 0) {
        window.chrome.webview.postMessage(JSON.stringify({kind: 'flash_frames', frames: n, page: performance.now()}));
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return 'flashing';
  };
  window.__place = function(x, y){
    card.style.width = '300px';
    card.style.height = '140px';
    card.style.transform = 'translate(' + x + 'px,' + y + 'px)';
    return 'placed';
  };
  // Paint ack: the overlay's own evidence that a new state reached a composited frame.
  // Two messages, one frame apart, because the instrument's own delay must be visible
  // in the numbers rather than baked into them:
  //   state-seen    — the first frame whose computed style is the new one. That frame is
  //                   the one that draws the new state, so its arrival is the closest
  //                   host-clock reading of "the renderer has produced the frame".
  //   state-committed — the next frame's callback, which by definition runs after the
  //                   previous frame was committed. One frame later by construction.
  // Neither is "on screen": the present and the DWM flip come after both, and are
  // invisible from inside the renderer. Printed as bounds for exactly that reason.
  //
  // They also only exist for states the document can observe — a CSS transform. A
  // `set_bounds` move that keeps the size identical changes nothing this page can read,
  // which is itself the finding: the shipping strategy cannot be instrumented from
  // inside the renderer at all.
  window.__ackSeq = 0;
  var ackOn = false, seen = '', waiting = false;
  var signature = function(){
    return getComputedStyle(card).transform + '|' + window.innerWidth + 'x' + window.innerHeight;
  };
  var ackTick = function(){
    if (!ackOn) return;
    var s = signature();
    if (s !== seen) {
      seen = s;
      waiting = true;
      window.chrome.webview.postMessage(JSON.stringify({
        kind: 'painted', step: 'seen', seq: ++window.__ackSeq, sig: s,
        page: performance.now()
      }));
    } else if (waiting) {
      waiting = false;
      window.chrome.webview.postMessage(JSON.stringify({
        kind: 'painted', step: 'committed', seq: ++window.__ackSeq, sig: s,
        page: performance.now()
      }));
    }
    requestAnimationFrame(ackTick);
  };
  window.__ack = function(on){
    if (on && !ackOn) {
      ackOn = true;
      seen = signature();
      waiting = false;
      requestAnimationFrame(ackTick);
    } else if (!on) {
      ackOn = false;
    }
    return ackOn ? 'acking' : 'ack-off';
  };
  document.addEventListener('mousedown', function(e){
    window.chrome.webview.postMessage(JSON.stringify({
      kind: 'overlay_mousedown', x: e.clientX, y: e.clientY,
      target: (e.target && (e.target.id || e.target.tagName)) || '?',
      page: performance.now()
    }));
  }, true);
  window.chrome.webview.postMessage(JSON.stringify({kind: 'overlay_ready', dpr: window.devicePixelRatio}));
  return 'overlay-ready';
})()"#.to_string()
}

/// Host-driven CSS placement through a message, so the message path a real host would
/// use is inside the measured interval rather than outside it.
fn overlay_relay_js() -> String {
    r#"(function(){
  if (window.__relay) return 'already';
  window.chrome.webview.addEventListener('message', function(e){
    var m = e.data;
    if (m && m.kind === 'place') { window.__place(m.x, m.y); }
  });
  window.__relay = true;
  return 'relay-on';
})()"#.to_string()
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Red-card test on an `(r,g,b)` readback: loose enough for the compositor's colour
/// management, tight enough that the blue anchor and the page's white fail it.
fn is_red(c: (u8, u8, u8)) -> bool {
    c.0 > 150 && c.1 < 110 && c.2 < 110
}

fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s
}

fn median(v: &[f64]) -> Option<f64> {
    let s = sorted(v);
    if s.is_empty() {
        None
    } else {
        Some(s[s.len() / 2])
    }
}

fn pct(v: &[f64], p: f64) -> Option<f64> {
    let s = sorted(v);
    if s.is_empty() {
        None
    } else {
        Some(s[((p * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)])
    }
}

/// Print a number only if it was measured. `None` prints `not measured`; a zero would be
/// a claim about something the probe never saw.
fn shown(v: Option<f64>, unit: &str) -> String {
    match v {
        Some(x) => format!("{x:.1} {unit}"),
        None => "not measured".to_string(),
    }
}

fn f64_field(json: &serde_json::Value, key: &str) -> f64 {
    json.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Strategy {
    /// Move the overlay's own controller — what `native-browser` does today.
    Bounds,
    /// Full-client overlay, card translated inside it by a host message.
    Css,
}

/// The screen pixel that goes red when the card reaches `(x, y)`: just inside the edge
/// it is travelling towards, in that edge's middle.
fn leading_edge(h: &Harness, x: i32, y: i32, dy: i32) -> (i32, i32) {
    let ly = if dy >= 0 {
        y + CARD_H - 3
    } else {
        y + 3
    };
    h.view_point_on_screen(OVERLAY_VIEW, x + CARD_W / 2, ly)
}

/// Apply one rect with one strategy, returning the microseconds the issuing call took.
fn apply(h: &mut Harness, strategy: Strategy, x: i32, y: i32) -> Result<f64, String> {
    match strategy {
        Strategy::Bounds => h.move_view(OVERLAY_VIEW, x, y, CARD_W, CARD_H),
        Strategy::Css => {
            let t = win::qpc_ticks();
            h.post(OVERLAY_VIEW, &format!("{{\"kind\":\"place\",\"x\":{x},\"y\":{y}}}"))?;
            Ok(win::us_since(t))
        }
    }
}

// ---------------------------------------------------------------------------
// Program
// ---------------------------------------------------------------------------

pub fn program(cfg: Cfg) -> Vec<Step> {
    let acc: Shared = Rc::new(RefCell::new(Acc::default()));
    let mut s: Vec<Step> = Vec::new();

    s.push(mark("V2.0 live page in the browser window (the surface an overlay must sit on)"));
    s.push(build_step(PAGE_VIEW, 0, "PAGE", Some(cfg.site.clone()), None, false));
    s.push(expect_record("PAGE", "NavigationCompleted", 20_000, "live page"));
    s.push(sleep_ms(2000));
    s.push(action("install the page probe script", |h| {
        h.run(PAGE_VIEW, &page_setup_js())?;
        h.run(PAGE_VIEW, &page_echo_js())?;
        Ok(())
    }));
    s.push(sleep_ms(400));

    s.push(mark("V2.1 process-tree memory: page live, no overlay yet (the baseline)"));
    s.push(rss_samples("baseline-no-overlay", 5, acc.clone()));

    s.push(mark("V2.2 add the overlay controller: same window, same environment, transparent"));
    s.push(build_step(
        OVERLAY_VIEW,
        0,
        "OVLY",
        Some("about:blank".to_string()),
        Some((40, 40, CARD_W, CARD_H)),
        true,
    ));
    s.push(action("install the overlay document", |h| {
        h.run(OVERLAY_VIEW, &overlay_setup_js())?;
        h.run(OVERLAY_VIEW, &overlay_relay_js())
    }));
    s.push(sleep_ms(1500));
    s.push(action("report the geometry both strategies share", {
        let acc = acc.clone();
        move |h| {
            let c = h.client_of(OVERLAY_VIEW);
            let scale = h.scale_of(OVERLAY_VIEW);
            acc.borrow_mut().scale = scale;
            println!(
                "GEOM window client = {}x{} at +{},{}  window scale = {scale}",
                c.2, c.3, c.0, c.1
            );
            println!(
                "GEOM card {CARD_W}x{CARD_H} logical = {}x{} physical",
                (CARD_W as f64 * scale).round() as i32,
                (CARD_H as f64 * scale).round() as i32
            );
            println!(
                "GEOM child windows of the browser window (creation order): {:?}",
                win::child_windows(h.hwnd(0))
            );
            Ok(())
        }
    }));
    s.push(rss_samples("overlay-added", 5, acc.clone()));
    s.push(step({
        let acc = acc.clone();
        move |h| {
            report_delta(&acc.borrow());
            h.rss_detail("attribution after the overlay was added");
            println!(
                "INFO browser process id — page view {:?}, overlay view {:?}. Same pid means \
                 the delta is one controller's worth, not a second engine's.",
                h.browser_process_id(PAGE_VIEW),
                h.browser_process_id(OVERLAY_VIEW)
            );
            Tick::Done
        }
    }));

    s.push(mark("V2.3 pixel-probe calibration: can this machine see a repaint at all, and at what resolution"));
    s.push(probe_calibration(acc.clone()));

    s.push(mark("V2.4 page↔host message round trip: the floor under every rect-originated latency below"));
    s.push(rtt_run(40, acc.clone()));

    s.push(mark("V2.5 idle follow, strategy A — WebView::set_bounds on the overlay's own controller (what ships)"));
    s.push(size_overlay("size the overlay to the card rect", false));
    s.push(sleep_ms(600));
    s.push(follow_run("set_bounds", Strategy::Bounds, cfg.samples, acc.clone()));

    s.push(mark("V2.6 idle follow, strategy B — full-client overlay, card translated by CSS transform"));
    s.push(size_overlay("grow the overlay to the full client area once", true));
    s.push(sleep_ms(600));
    s.push(follow_run("css-transform", Strategy::Css, cfg.samples, acc.clone()));

    s.push(mark(
        "V2.7 fling: does the card hold position over scrolling content, at four scroll \
         speeds, each strategy at its own real shape",
    ));
    // Each speed is run twice: the `set_bounds` run needs the card-sized overlay it
    // actually ships with, the CSS run needs the full-client one its transform lives in.
    // Running both against whatever size the last section left behind measured a shape
    // neither strategy would use.
    for speed in FLING_SPEEDS {
        s.push(size_overlay("card-sized overlay for the set_bounds fling", false));
        s.push(sleep_ms(300));
        s.push(fling_run(
            &format!("set_bounds @{speed}px/frame"),
            Strategy::Bounds,
            cfg.fling,
            speed,
            acc.clone(),
        ));
        s.push(size_overlay("full-client overlay for the css-transform fling", true));
        s.push(sleep_ms(300));
        s.push(fling_run(
            &format!("css-transform @{speed}px/frame"),
            Strategy::Css,
            cfg.fling,
            speed,
            acc.clone(),
        ));
        s.push(sleep_ms(400));
    }
    s.push(action("scroll the page back to the top", |h| {
        h.run(PAGE_VIEW, "window.scrollTo(0, 0)")
    }));
    s.push(sleep_ms(400));

    s.push(mark("V2.8 the alternative architecture: SetWindowPos on a second top-level window"));
    s.push(setwindowpos_run(cfg.samples, acc.clone()));

    s.push(step({
        let acc = acc.clone();
        move |_h| {
            verdict(&acc.borrow());
            Tick::Done
        }
    }));
    s
}

/// Put the overlay in the shape one strategy actually uses: the card's own rect, or the
/// whole client area. Needed between runs because the two strategies are measured on
/// overlays of different sizes, and a full-client overlay moved by `set_bounds` is not
/// the thing that ships.
fn size_overlay(tag: &str, full: bool) -> Step {
    let tag = tag.to_string();
    step(move |h| {
        let r = if full {
            let c = h.client_of(OVERLAY_VIEW);
            let scale = h.scale_of(OVERLAY_VIEW);
            (
                0,
                0,
                (c.2 as f64 / scale).round() as i32,
                (c.3 as f64 / scale).round() as i32,
            )
        } else {
            (40, 40, CARD_W, CARD_H)
        };
        match h.move_view(OVERLAY_VIEW, r.0, r.1, r.2, r.3) {
            Ok(us) => {
                println!("INFO {tag}: overlay → x{} y{} w{} h{} in {us:.0} µs", r.0, r.1, r.2, r.3);
                if full {
                    // Without this the grown card fills the client with red and there is
                    // no anchor left to compare against.
                    let _ = h.run(OVERLAY_VIEW, "window.__place(40, 40)");
                }
                Tick::Done
            }
            Err(e) => {
                println!("ERROR sizing the overlay: {e}");
                Tick::Done
            }
        }
    })
}

/// A build step that reports failure instead of truncating the run.
fn build_step(
    view: usize,
    window: usize,
    label: &str,
    url: Option<String>,
    bounds: Option<(i32, i32, i32, i32)>,
    transparent: bool,
) -> Step {
    let label = label.to_string();
    step(move |h| match h.build_in(view, window, &label, url.as_deref(), bounds, transparent) {
        Ok(()) => Tick::Done,
        Err(e) => {
            println!("ERROR build {label}: {e}");
            Tick::Done
        }
    })
}

/// `n` whole-process-tree working-set samples, 200 ms apart.
fn rss_samples(tag: &str, n: usize, acc: Shared) -> Step {
    let tag = tag.to_string();
    let mut i = 0usize;
    let mut got: Vec<f64> = Vec::new();
    let mut t = 0.0f64;
    step(move |h| {
        if i > 0 && win::ms_since(t) < 200.0 {
            return Tick::Running;
        }
        let m = h.rss(&format!("{tag}#{i}"));
        got.push(m.ws_mib());
        acc.borrow_mut().rss.push((format!("{tag}#{i}"), m.ws_mib()));
        i += 1;
        t = win::qpc_ms();
        if i < n {
            return Tick::Running;
        }
        println!(
            "RSS {tag}: median={:.1} MiB min={:.1} max={:.1} over {n} samples",
            median(&got).unwrap_or(0.0),
            got.iter().fold(f64::INFINITY, |a, b| a.min(*b)),
            got.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b))
        );
        Tick::Done
    })
}

/// The delta, which is the number that matters — not the absolute.
fn report_delta(a: &Acc) {
    let med = |tag: &str| -> Option<f64> {
        median(
            &a.rss
                .iter()
                .filter(|(k, _)| k.starts_with(tag))
                .map(|(_, m)| *m)
                .collect::<Vec<f64>>(),
        )
    };
    let (Some(base), Some(after)) = (med("baseline-no-overlay"), med("overlay-added")) else {
        println!("V2-1 RSS cost of one overlay controller: NOT MEASURED (missing samples)");
        return;
    };
    println!(
        "V2-1 RSS delta of adding one overlay controller over a live page: {:+.1} MiB \
         ({base:.1} → {after:.1} MiB, whole process tree, median of 5 samples each). Per-pid \
         attribution follows: a second browser process would appear as a new pid, so check the \
         process counts on the two RSS lines before believing the delta is one controller's.",
        after - base
    );
}

/// Page → host → page → host round trip through `postMessage` /
/// `PostWebMessageAsJson`: both legs that a rect report and a placement command each pay.
fn rtt_run(n: usize, acc: Shared) -> Step {
    let mut sent = 0.0f64;
    let mut waiting = false;
    let mut i = 0usize;
    let mut timeouts = 0usize;
    step(move |h| {
        if !waiting {
            h.clear_inbox();
            sent = win::qpc_ms();
            if let Err(e) = h.post(PAGE_VIEW, &format!("{{\"kind\":\"ping\",\"seq\":{i}}}")) {
                println!("ERROR post: {e}");
                return Tick::Done;
            }
            waiting = true;
            return Tick::Running;
        }
        if h.take_msgs("PAGE", "pong").is_empty() {
            if win::ms_since(sent) > 1500.0 {
                timeouts += 1;
                println!("TIMEOUT: no pong for sample {i} ({timeouts} in a row)");
                waiting = false;
                if timeouts >= 3 {
                    // Bounded: an unresponsive message channel is a finding, not a
                    // reason to keep the run spinning.
                    println!(
                        "ABORT round trip: the page channel stopped answering after {i} of \
                         {n} samples; the round trip is NOT MEASURED"
                    );
                    return Tick::Done;
                }
                return Tick::Running;
            }
            return Tick::Running;
        }
        acc.borrow_mut().rtt_ms.push(win::qpc_ms() - sent);
        i += 1;
        timeouts = 0;
        waiting = false;
        if i >= n {
            // Cloned, not taken: the verdict prints the same distribution again with
            // the instrument named next to it.
            let v = acc.borrow().rtt_ms.clone();
            Harness::percentiles("page↔host round trip", "ms", v);
            return Tick::Done;
        }
        Tick::Running
    })
}

/// The two pixels this machine must be able to read before any pixel timing is
/// believed: the page's white background and the card's red. Two known colours,
/// because a single read cannot distinguish "this region is transparent" from "this
/// instrument is blind".
fn references(
    h: &mut Harness,
    page: (i32, i32),
    card: (i32, i32),
) -> (Option<(u8, u8, u8)>, Option<(u8, u8, u8)>) {
    let white = h.probe.pixel(page.0, page.1);
    let red = h.probe.pixel(card.0, card.1);
    (white, red)
}

fn white_ok(c: Option<(u8, u8, u8)>) -> bool {
    c.map(|c| c.0 > 200 && c.1 > 200 && c.2 > 200)
        .unwrap_or(false)
}

/// A pixel read has to be cheaper than this to be used inside a per-frame loop. Half a
/// 60 Hz frame, chosen so the instrument cannot be the thing that makes a frame late.
const FRAME_BUDGET_MS: f64 = 8.0;
/// And cheaper than this to be used as the answer to a latency question at all: the
/// lower edge of the band under test. A coarser instrument can bound a latency, it
/// cannot resolve one, and saying so is the result rather than a failure.
const RESOLVE_BUDGET_MS: f64 = 25.0;

/// Flashes the card every other renderer frame while the host polls one screen pixel, so
/// the readback's detection ratio and resolution are known before any latency printed
/// from it is believed.
///
/// The calibration also *disqualifies* the instrument. A read that costs more than the
/// band under test cannot measure it, so the step says that in as many words and turns
/// pixel attribution off, instead of printing a 95 ms number as though it were a
/// latency. Attribution stays on when the read is coarse but the run is idle-paced,
/// because there it quantises the answer rather than distorting it.
fn probe_calibration(acc: Shared) -> Step {
    let mut started = false;
    let mut start = 0.0f64;
    let mut last_state = true;
    let mut last_flip = 0.0f64;
    let mut page_frames = 0usize;
    step(move |h| {
        if !h.probe.available() {
            println!("PROBE unavailable: neither the screen DC nor PrintWindow returns a pixel here.");
            println!("Every pixel-derived latency below is reported as NOT MEASURED instead of guessed.");
            acc.borrow_mut().probe_ok = false;
            return Tick::Done;
        }
        // The card is at (40,40) logical in the window's client space at this point.
        let card_pt = h.view_point_on_screen(OVERLAY_VIEW, 40 + CARD_W / 2, 40 + CARD_H - 3);
        let page_pt = h.view_point_on_screen(PAGE_VIEW, 20, 20);
        let (px, py) = card_pt;
        if !started {
            // Which instrument is live decides what every pixel number below means,
            // so it is printed once, before any of them.
            println!("PROBE instruments: {}", h.probe.diagnostic());
            // The cheap target — the window under the point — is tried first and only
            // kept if it answers both reference pixels.
            let (white, red) = references(h, page_pt, card_pt);
            let cheap_ok = white_ok(white) && red.map(is_red).unwrap_or(false);
            println!(
                "PROBE reference read ({}): page background {white:?} at {page_pt:?}, card \
                 {red:?} at ({px},{py})",
                h.probe.render_target()
            );
            if !cheap_ok {
                println!("PROBE the card's own window does not answer both references; rendering the whole top-level window instead");
                h.probe.render_into_root_window();
                let (white, red) = references(h, page_pt, card_pt);
                println!(
                    "PROBE reference read ({}): page background {white:?}, card {red:?}",
                    h.probe.render_target()
                );
                if !white_ok(white) || !red.map(is_red).unwrap_or(false) {
                    println!("PROBE neither target sees this machine's rendering; pixel timings are NOT MEASURED");
                    acc.borrow_mut().probe_ok = false;
                    return Tick::Done;
                }
            }
            // The resolution that matters is the cost of the read that answers, which on
            // this machine includes a failed screen attempt before the PrintWindow.
            let read_ms = h.probe.read_cost_us() / 1000.0;
            let best_ms = h.probe.read_best_us() / 1000.0;
            println!(
                "PROBE reading through the {} via the {}; one read costs {read_ms:.1} ms \
                 (fastest {best_ms:.1} ms), which is the resolution of every pixel number \
                 below",
                h.probe.source(),
                h.probe.render_target(),
            );
            {
                let mut a = acc.borrow_mut();
                a.probe_read_ms = read_ms;
                a.probe_fits_frame = read_ms <= FRAME_BUDGET_MS;
                a.instrument = format!(
                    "{} instrument, one read costs {:.0} µs",
                    h.probe.source(),
                    read_ms * 1000.0
                );
            }
            if read_ms > RESOLVE_BUDGET_MS {
                // The headline this step can produce on its own: the question is
                // answerable here or it is not, and that is known before any sample.
                println!(
                    "PROBE verdict: DECLINED. One read costs {read_ms:.1} ms, which is \
                     {read_ms:.0}× the {RESOLVE_BUDGET_MS:.0} ms lower edge of the band under \
                     test, so this machine cannot resolve a rect→painted latency — it can \
                     only bound one. Pixel attribution is off; the numbers that do not need \
                     a readback (issuing calls, message round trip, follow error read off \
                     the message stream, the overlay's own paint ack) are still measured."
                );
                acc.borrow_mut().probe_ok = false;
                return Tick::Done;
            }
            if let Err(e) = h.run(OVERLAY_VIEW, "window.__flash(true, 2)") {
                println!("ERROR flash: {e}");
                acc.borrow_mut().probe_ok = false;
                return Tick::Done;
            }
            started = true;
            start = win::qpc_ms();
            last_flip = start;
            return Tick::Running;
        }
        // A failed read is not a colour transition: skip the tick instead of
        // recording it as one.
        let Some(c) = h.probe.pixel(px, py) else {
            return Tick::Running;
        };
        let red = is_red(c);
        if red != last_state {
            let now = win::qpc_ms();
            let mut a = acc.borrow_mut();
            a.probe_transitions += 1;
            a.probe_gaps.push(now - last_flip);
            last_flip = now;
            last_state = red;
        }
        for m in h.take_msgs("OVLY", "flash_frames") {
            page_frames = f64_field(&m.json, "frames") as usize;
        }
        if win::ms_since(start) < 1500.0 {
            return Tick::Running;
        }
        let _ = h.run(OVERLAY_VIEW, "window.__flash(false, 2)");
        let expected = page_frames / 2;
        let gaps = std::mem::take(&mut acc.borrow_mut().probe_gaps);
        let transitions = acc.borrow().probe_transitions;
        {
            let mut a = acc.borrow_mut();
            a.probe_expected = expected;
            a.probe_ok = true;
        }
        println!(
            "PROBE calibration: the page ran {page_frames} frames and asked for {expected} \
             colour flips; the readback saw {transitions} transitions in 1500 ms"
        );
        Harness::percentiles("probe interval between detected flips", "ms", gaps);
        let ratio = if expected > 0 { transitions as f64 / expected as f64 } else { 0.0 };
        println!(
            "PROBE verdict: detection ratio {ratio:.2}. The readback {} follow a per-frame \
             repaint; every rect→painted number below is an upper bound with a resolution of \
             the median poll interval above plus the compositor present delay.",
            if ratio > 0.9 {
                "does"
            } else if ratio > 0.4 {
                "only partly can"
            } else {
                "cannot"
            }
        );
        Tick::Done
    })
}

/// Idle follow samples: the page reports the anchor rect, the host applies it with
/// `strategy`, and the run times two different endings of the same interval.
///
/// 1. *pixel*: how long until the card's leading edge is visibly there. This is the
///    question as asked, and it needs a screen readback that resolves the band.
/// 2. *ack*: how long until the overlay's own renderer reports a committed frame
///    carrying the new state. It is a lower bound — the present and the DWM flip come
///    after it — and it needs no readback, so it is measurable on a machine where 1 is
///    not. It is only available to the strategy that changes the document.
///
/// The previous position is watched too — a card that appears in the new place before
/// it has left the old one is a visible artefact, not a latency win.
fn follow_run(name: &str, strategy: Strategy, n: usize, acc: Shared) -> Step {
    let name = name.to_string();
    // Read when the step first runs, not when the program is built: the calibration
    // step decides this flag, and at build time it has not run yet.
    let mut probe_usable = false;
    let mut initialised = false;
    // The ack instrument is the CSS strategy's only: it observes the document, and a
    // `set_bounds` move at constant size does not change the document.
    let mut acks = false;
    let mut latencies: Vec<f64> = Vec::new();
    // The two paint-ack beats, timed separately so the ack's own one-frame delay is a
    // printed number and not a hidden term in the latency.
    let mut seen_ms: Vec<f64> = Vec::new();
    let mut commit_ms: Vec<f64> = Vec::new();
    let mut calls_us: Vec<f64> = Vec::new();
    let mut clears_ms: Vec<f64> = Vec::new();
    let mut seq = 0usize;
    let mut unattributable = 0usize;
    let mut timeouts = 0usize;
    // Sample state. All of it lives in this closure: no statics, no globals.
    enum Ph {
        Fire,
        WaitRect,
        WaitPaint,
        Gap,
    }
    let mut ph = Ph::Fire;
    let mut t_mark = 0.0f64;
    let mut recv = 0.0f64;
    let mut target = (0i32, 0i32);
    let mut probe_pt = (0i32, 0i32);
    let mut old_center = (0i32, 0i32);
    let mut erase_from: Option<f64> = None;
    let mut pix_done = true;
    let mut seen_done = true;
    let mut commit_done = true;
    let mut last_read = (0u8, 0u8, 0u8);

    step(move |h| {
        if !initialised {
            probe_usable = acc.borrow().probe_ok;
            acks = matches!(strategy, Strategy::Css);
            initialised = true;
            println!(
                "{name}: pixel attribution is {} (calibration's call); reading through the {}; \
                 one read costs {:.1} ms",
                if probe_usable { "on" } else { "OFF — pixel latencies will be NOT MEASURED" },
                h.probe.render_target(),
                acc.borrow().probe_read_ms
            );
            if acks {
                let _ = h.run(OVERLAY_VIEW, "window.__ack(true)");
                println!(
                    "{name}: the overlay's paint ack is on — rect→ack below is a lower bound \
                     on rect→painted (it stops at the frame commit, before the present)"
                );
            } else {
                println!(
                    "{name}: no renderer-side ack is possible for this strategy — a \
                     set_bounds move that keeps the size identical changes nothing the \
                     overlay document can observe, so only the screen can answer when it \
                     got there"
                );
            }
        }
        loop {
            match ph {
                Ph::Fire => {
                    h.clear_inbox();
                    let dy = if seq % 2 == 0 { 0 } else { IDLE_JUMP };
                    if let Err(e) = h.run(PAGE_VIEW, &format!("window.__postRect({seq},{dy})")) {
                        println!("ERROR postRect: {e}");
                        finish_follow(
                            &acc, &name, strategy, &latencies, &seen_ms, &commit_ms, &calls_us,
                            &clears_ms, unattributable, seq, timeouts,
                        );
                        return Tick::Done;
                    }
                    t_mark = win::qpc_ms();
                    ph = Ph::WaitRect;
                }
                Ph::WaitRect => {
                    let got = h.take_msgs("PAGE", "anchor");
                    let Some(msg) = got.into_iter().last() else {
                        if win::ms_since(t_mark) > 1500.0 {
                            println!("TIMEOUT: no anchor rect for sample {seq}");
                            timeouts += 1;
                            seq += 1;
                            ph = if seq >= n { Ph::Gap } else { Ph::Fire };
                            t_mark = win::qpc_ms();
                            continue;
                        }
                        return Tick::Running;
                    };
                    recv = msg.ms;
                    acc.borrow_mut().dpr = f64_field(&msg.json, "dpr");
                    let x = f64_field(&msg.json, "x").round() as i32;
                    let y = f64_field(&msg.json, "y").round() as i32;
                    let dy = y - target.1;
                    old_center = h.view_point_on_screen(OVERLAY_VIEW, target.0 + CARD_W / 2, target.1 + CARD_H / 2);
                    target = (x, y);
                    probe_pt = leading_edge(h, x, y, dy);
                    // A sample counts only if the pixel it is about to watch was clear.
                    // Otherwise "it was already red" would be printed as 0 ms latency.
                    let attributable = probe_usable
                        && h.probe.pixel(probe_pt.0, probe_pt.1)
                            .map(|c| !is_red(c))
                            .unwrap_or(false);
                    if probe_usable && !attributable {
                        unattributable += 1;
                    }
                    let call_us = match apply(h, strategy, x, y) {
                        Ok(us) => us,
                        Err(e) => {
                            println!("ERROR applying the rect: {e}");
                            finish_follow(
                                &acc, &name, strategy, &latencies, &seen_ms, &commit_ms,
                                &calls_us, &clears_ms, unattributable, seq, timeouts,
                            );
                            return Tick::Done;
                        }
                    };
                    calls_us.push(call_us);
                    erase_from = None;
                    pix_done = !attributable;
                    seen_done = !acks;
                    commit_done = !acks;
                    ph = if attributable || acks { Ph::WaitPaint } else { Ph::Gap };
                    t_mark = win::qpc_ms();
                }
                Ph::WaitPaint => {
                    let now = win::qpc_ms();
                    if !pix_done {
                        let c = h.probe.pixel(probe_pt.0, probe_pt.1).unwrap_or((0, 0, 0));
                        last_read = c;
                        if is_red(c) {
                            latencies.push(now - recv);
                            erase_from = Some(now);
                            pix_done = true;
                        }
                    }
                    // Both ack beats are collected before the sample closes, so a beat
                    // can never be attributed to the following sample.
                    if !seen_done || !commit_done {
                        for m in h.take_msgs("OVLY", "painted") {
                            let dt = m.ms - recv;
                            let beat = m.json.get("step").and_then(|s| s.as_str()).unwrap_or("seen");
                            if beat == "committed" {
                                if !commit_done {
                                    commit_ms.push(dt);
                                    commit_done = true;
                                }
                            } else if !seen_done {
                                seen_ms.push(dt);
                                seen_done = true;
                            }
                        }
                    }
                    if pix_done && seen_done && commit_done {
                        ph = Ph::Gap;
                        t_mark = win::qpc_ms();
                        continue;
                    }
                    if now - recv > 1000.0 {
                        let mut waited = Vec::new();
                        if !pix_done {
                            waited.push(format!("the leading edge never turned red (last read rgb{last_read:?})"));
                        }
                        if !seen_done {
                            waited.push("no paint ack from the overlay".to_string());
                        }
                        println!("TIMEOUT: {name} sample {seq}: {waited:?}");
                        timeouts += 1;
                        ph = Ph::Gap;
                        t_mark = win::qpc_ms();
                        continue;
                    }
                    return Tick::Running;
                }
                Ph::Gap => {
                    // A late ack still belongs to this sample; a late one is itself the
                    // answer, just a bad one.
                    if !seen_done || !commit_done {
                        for m in h.take_msgs("OVLY", "painted") {
                            let dt = m.ms - recv;
                            let beat = m.json.get("step").and_then(|s| s.as_str()).unwrap_or("seen");
                            if beat == "committed" {
                                if !commit_done {
                                    commit_ms.push(dt);
                                    commit_done = true;
                                }
                            } else if !seen_done {
                                seen_ms.push(dt);
                                seen_done = true;
                            }
                        }
                    }
                    // Did the card leave the place it was? Measured from the paint, so a
                    // long value here is a visible smear, not a scheduling accident.
                    if let Some(from) = erase_from {
                        let c = h.probe.pixel(old_center.0, old_center.1).unwrap_or((0, 0, 0));
                        if !is_red(c) {
                            clears_ms.push(win::qpc_ms() - from);
                            erase_from = None;
                        }
                    }
                    if win::ms_since(t_mark) < 120.0 {
                        return Tick::Running;
                    }
                    seq += 1;
                    if seq >= n {
                        if acks {
                            let _ = h.run(OVERLAY_VIEW, "window.__ack(false)");
                        }
                        finish_follow(
                            &acc, &name, strategy, &latencies, &seen_ms, &commit_ms, &calls_us,
                            &clears_ms, unattributable, seq, timeouts,
                        );
                        return Tick::Done;
                    }
                    ph = Ph::Fire;
                }
            }
        }
    })
}

fn finish_follow(
    acc: &Shared,
    name: &str,
    strategy: Strategy,
    latencies: &[f64],
    seen_ms: &[f64],
    commit_ms: &[f64],
    calls_us: &[f64],
    clears_ms: &[f64],
    unattributable: usize,
    samples: usize,
    timeouts: usize,
) {
    println!(
        "{name}: {samples} samples, {} attributable, {unattributable} unattributable, \
         {timeouts} timeouts",
        latencies.len()
    );
    // These clones are on the report path, after the last sample was timed, so they
    // cannot perturb a number above them.
    Harness::percentiles(&format!("{name} rect→painted (on screen)"), "ms", latencies.to_vec());
    Harness::percentiles(
        &format!("{name} rect→style observed (renderer 'seen' ack, a lower bound)"),
        "ms",
        seen_ms.to_vec(),
    );
    Harness::percentiles(
        &format!("{name} rect→frame committed (renderer 'committed' ack, +1 frame)"),
        "ms",
        commit_ms.to_vec(),
    );
    Harness::percentiles(&format!("{name} issuing call"), "µs", calls_us.to_vec());
    Harness::percentiles(&format!("{name} old position cleared"), "ms", clears_ms.to_vec());
    let mut a = acc.borrow_mut();
    a.unattributable += unattributable;
    match strategy {
        Strategy::Bounds => {
            a.idle_bounds_ms = latencies.to_vec();
            a.setbounds_us = calls_us.to_vec();
        }
        Strategy::Css => {
            a.idle_css_ms = latencies.to_vec();
            a.csspost_us = calls_us.to_vec();
            a.idle_css_ack_ms = seen_ms.to_vec();
            a.idle_css_commit_ms = commit_ms.to_vec();
        }
    }
}

/// One rect the host has applied and is waiting to see painted.
struct Pending {
    seq: u64,
    /// Host clock at the moment the rect arrived.
    recv: f64,
    /// The rect that was applied, so the error can be computed against where the anchor
    /// actually was when the card painted.
    x: i32,
    y: i32,
    probe: (i32, i32),
}

/// One fling at one speed. The page drives a fixed scroll step per renderer frame and
/// reports its anchor rect every frame; the host follows every report immediately, which
/// is what an anchored card would have to do; the error is how far the anchor had already
/// moved from the rect the card was showing at the moment that card painted. Both
/// quantities are on the host clock, so no renderer-clock alignment enters the number.
fn fling_run(name: &str, strategy: Strategy, distance: i32, step_px: i32, acc: Shared) -> Step {
    let name = name.to_string();
    // Decided by the calibration step, which runs before this one — so it is read on
    // the first tick, not when the program was built.
    let mut probe_usable = false;
    let mut initialised = false;
    enum Ph {
        Prep,
        Flinging,
        Settle,
    }
    let mut ph = Ph::Prep;
    let mut t_mark = 0.0f64;
    let mut latencies: Vec<f64> = Vec::new();
    let mut errs: Vec<f64> = Vec::new();
    // Follow error read straight off the message stream: how far the anchor had moved
    // from the rect the card was last given, sampled at every frame the renderer
    // reports. Needs no pixel instrument, so it survives a machine where the readback
    // cannot see the screen.
    let mut frame_err: Vec<f64> = Vec::new();
    // Movement between consecutive anchor reports: the velocity the follower has to
    // overcome, and therefore the follow-error floor for a host with zero latency. It
    // is per run, because each run is one speed and the two numbers are read as a pair.
    let mut deltas: Vec<f64> = Vec::new();
    let mut last_applied: Option<(i32, i32)> = None;
    let mut raw_logged = 0usize;
    let mut calls_us: Vec<f64> = Vec::new();
    // Page sequence numbers of the rects whose paint the probe actually saw. The range
    // is printed at the end so a reader can tell "the probe missed a few frames" from
    // "the probe never saw anything".
    let mut resolved: Vec<u64> = Vec::new();
    let mut applied = 0usize;
    let mut unattributable = 0usize;
    let mut dropped = 0usize;
    // Anchor rects as reported, stamped with host arrival time — the table the error
    // is read out of.
    let mut table: Vec<(f64, i32, i32)> = Vec::new();
    let mut prev: Option<(i32, i32)> = None;
    let mut pending: VecDeque<Pending> = VecDeque::new();
    // Prep has two beats: jump to the fling anchor, let the page settle there, then
    // start the driven scroll. Starting on the same tick as the jump would measure a
    // rect that had not reached layout yet.
    let mut jumped = false;

    step(move |h| {
        if !initialised {
            let a = acc.borrow();
            probe_usable = a.probe_ok && a.probe_fits_frame;
            initialised = true;
            if !a.probe_ok {
                println!("FLING {name}: pixel attribution OFF, the readback was declined in V2.3.");
            } else if !a.probe_fits_frame {
                println!(
                    "FLING {name}: pixel attribution OFF on purpose — one read costs \
                     {:.1} ms, which is more than the {:.0} ms frame the page is reporting \
                     into, so attributing with it would make the instrument the slowest \
                     thing in the loop and manufacture the follow error this section is \
                     supposed to measure. The follow error below is read off the message \
                     stream instead.",
                    a.probe_read_ms,
                    FRAME_BUDGET_MS
                );
            } else {
                println!(
                    "FLING {name}: pixel attribution on ({:.1} ms a read, inside the frame \
                     budget); reading through the {}",
                    a.probe_read_ms,
                    h.probe.render_target()
                );
            }
        }
        match ph {
            Ph::Prep => {
                h.clear_inbox();
                if !jumped {
                    // The anchor travels down the screen for the whole run only if the
                    // run starts with it at the viewport top.
                    if let Err(e) = h.run(PAGE_VIEW, "window.scrollTo(0, window.__flingStart())") {
                        println!("ERROR positioning the fling: {e}");
                        return Tick::Done;
                    }
                    jumped = true;
                    t_mark = win::qpc_ms();
                    return Tick::Running;
                }
                if win::ms_since(t_mark) < 500.0 {
                    return Tick::Running;
                }
                let _ = h.take_msgs("PAGE", "anchor");
                if let Err(e) = h.run(PAGE_VIEW, &page_follow_js(distance, step_px)) {
                    println!("ERROR follow script: {e}");
                    return Tick::Done;
                }
                t_mark = win::qpc_ms();
                ph = Ph::Flinging;
                println!(
                    "FLING {name}: scrolling {distance} CSS px in {step_px} px steps (upward, \
                     so the anchor stays in the viewport) from the page's own frames, \
                     following every report with {strategy:?}"
                );
                Tick::Running
            }
            Ph::Flinging => {
                // 1. Follow every anchor report the renderer has produced, in order.
                for m in h.take_msgs("PAGE", "anchor") {
                    let seq = f64_field(&m.json, "seq") as u64;
                    let x = f64_field(&m.json, "x").round() as i32;
                    let y = f64_field(&m.json, "y").round() as i32;
                    if raw_logged < 3 {
                        raw_logged += 1;
                        // The rect and the scroll offset together: if the first frames
                        // report a still rect beside a moving scroll, the page-side
                        // observer is lying and nothing downstream can be trusted.
                        println!(
                            "FRAME {seq}: anchor x={} y={} scrollY={:.1} moved={:.1} css px",
                            f64_field(&m.json, "x"),
                            f64_field(&m.json, "y"),
                            f64_field(&m.json, "scroll"),
                            f64_field(&m.json, "moved"),
                        );
                    }
                    // Before the new rect is applied: this is the gap the card showed
                    // for the whole duration of this frame.
                    if let Some((ax, ay)) = last_applied {
                        let dx = (x - ax) as f64;
                        let dyf = (y - ay) as f64;
                        frame_err.push((dx * dx + dyf * dyf).sqrt());
                    }
                    last_applied = Some((x, y));
                    let dy = prev.map(|(_, py)| y - py).unwrap_or(1);
                    if let Some((px, py)) = prev {
                        let d = (((x - px) * (x - px) + (y - py) * (y - py)) as f64).sqrt();
                        deltas.push(d);
                    }
                    prev = Some((x, y));
                    table.push((m.ms, x, y));
                    let probe_pt = leading_edge(h, x, y, dy);
                    let ok = probe_usable
                        && h.probe.pixel(probe_pt.0, probe_pt.1)
                            .map(|c| !is_red(c))
                            .unwrap_or(false);
                    if probe_usable && !ok {
                        unattributable += 1;
                    }
                    match apply(h, strategy, x, y) {
                        Ok(us) => calls_us.push(us),
                        Err(e) => {
                            println!("ERROR applying the rect mid-fling: {e}");
                            ph = Ph::Settle;
                            t_mark = win::qpc_ms();
                            break;
                        }
                    }
                    applied += 1;
                    if ok {
                        if pending.len() >= 16 {
                            // The probe cannot attribute that far back; drop the oldest
                            // rather than let the queue grow without bound.
                            pending.pop_front();
                            dropped += 1;
                        }
                        pending.push_back(Pending { seq, recv: m.ms, x, y, probe: probe_pt });
                    }
                }
                // 2. Resolve the ones whose pixels have turned. Paints land in order, so
                // scanning front to back and stopping at the first miss is correct.
                while let Some(p) = pending.front() {
                    let c = h.probe.pixel(p.probe.0, p.probe.1).unwrap_or((0, 0, 0));
                    if !is_red(c) {
                        break;
                    }
                    let p = pending.pop_front().expect("front checked above");
                    let now = win::qpc_ms();
                    latencies.push(now - p.recv);
                    resolved.push(p.seq);
                    // Where was the anchor when that card actually painted? The last
                    // report to arrive by then is the closest observation there is.
                    if let Some(&(_, ax, ay)) = table.iter().filter(|(t, _, _)| *t <= now).last() {
                        let dx = (ax - p.x) as f64;
                        let dyf = (ay - p.y) as f64;
                        errs.push((dx * dx + dyf * dyf).sqrt());
                    }
                }
                // 3. End of the fling: the page says the scroll settled, and tells us the
                // distance and the time it took, which is the velocity the follower was up
                // against.
                let ends = h.take_msgs("PAGE", "fling_end");
                let ended = !ends.is_empty();
                if ended {
                    dropped += pending.len();
                    pending.clear();
                    // The velocity and the frame pacing the page actually got. Both are
                    // needed to read the follow error: the error is one reported frame of
                    // scroll, so it is velocity × frame time and nothing about the overlay.
                    let mut label = name.clone();
                    if let Some(v) = ends.first().map(|m| m.json.clone()) {
                        let scroll = f64_field(&v, "scroll");
                        let ms = f64_field(&v, "ms");
                        let frames = f64_field(&v, "frames");
                        if ms > 0.0 {
                            let per_s = scroll.max(0.0) / (ms / 1000.0);
                            acc.borrow_mut().fling_velocity = per_s;
                            println!(
                                "FLING {name}: the page reports {scroll:.0} CSS px in {ms:.0} ms \
                                 = {per_s:.0} CSS px/s over {frames:.0} of its own frames \
                                 ({:.1} ms each)",
                                if frames > 0.0 { ms / frames } else { 0.0 }
                            );
                            label = format!("{name}, {per_s:.0} css px/s");
                        }
                    }
                    let (lo, hi) = resolved
                        .iter()
                        .fold((u64::MAX, 0u64), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
                    let range = if resolved.is_empty() {
                        "none".to_string()
                    } else {
                        format!("{lo}..={hi}")
                    };
                    println!(
                        "FLING {name}: {applied} rects followed, {} painted-resolved \
                         (page seqs {range}), {unattributable} unattributable, {dropped} not \
                         attributable before the fling ended",
                        latencies.len()
                    );
                    Harness::percentiles(
                        &format!("{name} fling rect→painted (on screen)"),
                        "ms",
                        latencies.clone(),
                    );
                    Harness::percentiles(
                        &format!("{name} fling follow error (paint-attributed)"),
                        "css px",
                        errs.clone(),
                    );
                    Harness::percentiles(&format!("{name} fling apply call"), "µs", calls_us.clone());
                    Harness::percentiles(
                        &format!("{name} follow error per reported frame"),
                        "css px",
                        frame_err.clone(),
                    );
                    Harness::percentiles(
                        &format!("{name} scroll per reported frame"),
                        "css px",
                        deltas.clone(),
                    );
                    let dpr = acc.borrow().dpr.max(1.0);
                    let phys_frame: Vec<f64> = frame_err.iter().map(|e| e * dpr).collect();
                    Harness::percentiles(
                        &format!("{name} follow error per reported frame"),
                        "physical px",
                        phys_frame,
                    );
                    // One row per (strategy, speed): the verdict prints follow error beside
                    // the scroll per frame beside the velocity, which is the point of
                    // running more than one speed.
                    let mut a = acc.borrow_mut();
                    a.err_series.push((label.clone(), frame_err.clone()));
                    a.paint_series.push((label, latencies.clone()));
                    a.dropped += dropped;
                    a.unattributable += unattributable;
                    ph = Ph::Settle;
                    t_mark = win::qpc_ms();
                    return Tick::Running;
                }
                if win::ms_since(t_mark) > 8000.0 {
                    println!("TIMEOUT: the fling never reported an end after 8 s ({applied} rects followed)");
                    let mut a = acc.borrow_mut();
                    a.dropped += dropped + pending.len();
                    a.err_series.push((name.clone(), frame_err.clone()));
                    a.paint_series.push((name.clone(), latencies.clone()));
                    return Tick::Done;
                }
                Tick::Running
            }
            Ph::Settle => {
                if win::ms_since(t_mark) < 400.0 {
                    return Tick::Running;
                }
                let _ = h.run(PAGE_VIEW, "if (window.__fl) window.__fl.on = false;");
                Tick::Done
            }
        }
    })
}

/// `SetWindowPos` on a second top-level window: what the "host owns a window per
/// overlay" architecture pays per reposition, measured with the same instrument.
fn setwindowpos_run(n: usize, acc: Shared) -> Step {
    let mut i = 0usize;
    let mut samples: Vec<f64> = Vec::new();
    let mut t = 0.0f64;
    step(move |h| {
        if i == 0 {
            h.show(SCRATCH_WINDOW);
            t = win::qpc_ms();
        }
        if i >= n {
            Harness::percentiles("SetWindowPos (top-level, no activate)", "µs", samples.clone());
            acc.borrow_mut().setwindowpos_us = std::mem::take(&mut samples);
            return Tick::Done;
        }
        let x = 200 + ((i % 2) as i32 * 120);
        let y = 200 + ((i % 3) as i32 * 90);
        samples.push(h.move_window(SCRATCH_WINDOW, x, y, 420, 300, true));
        i += 1;
        if i % 40 == 0 {
            println!("INFO {i}/{n} SetWindowPos samples, {} ms elapsed", win::ms_since(t) as i64);
        }
        Tick::Running
    })
}

/// The comparison the design was built on: 25–50 ms p50, 60–120 ms p95, and 100–500 px of
/// follow error during a fling.
fn verdict(a: &Acc) {
    println!("\n=== V2 VERDICTS ===");
    println!(
        "Pixel timings below were read through the {}.",
        if a.instrument.is_empty() {
            "readback that never became usable — see V2.3"
        } else {
            a.instrument.as_str()
        }
    );
    if !a.probe_ok {
        println!(
            "The readback was DECLINED in V2.3 (one read costs {:.1} ms, the band under test \
             starts at {RESOLVE_BUDGET_MS:.0} ms), so no on-screen latency is claimed below. \
             Where a number is marked as a lower bound, it was measured by the overlay's own \
             paint ack and stops before the compositor present.",
            a.probe_read_ms
        );
    }
    let band = |label: &str, v: &[f64], p: f64, lo: f64, hi: f64, claim: &str| {
        let Some(x) = pct(v, p) else {
            println!("{label}: NOT MEASURED — the {claim} estimate stays untested");
            return;
        };
        let call = if x < lo {
            "below the band, the estimate is pessimistic"
        } else if x <= hi {
            "inside the band, the estimate holds"
        } else {
            "ABOVE the band, the estimate is optimistic and the design pays more than planned"
        };
        println!("{label} = {x:.1} ms against a claim of {claim} → {call}");
    };
    band("V2-2 idle rect→painted p50, set_bounds", &a.idle_bounds_ms, 0.50, 25.0, 50.0, "25-50 ms");
    band("V2-2 idle rect→painted p95, set_bounds", &a.idle_bounds_ms, 0.95, 60.0, 120.0, "60-120 ms");
    band("V2-2 idle rect→painted p50, css-transform", &a.idle_css_ms, 0.50, 25.0, 50.0, "25-50 ms");
    band("V2-2 idle rect→painted p95, css-transform", &a.idle_css_ms, 0.95, 60.0, 120.0, "60-120 ms");
    // The same interval from the other end. The overlay's renderer reports the frame
    // whose style is the new one, and again one frame later once that frame has been
    // committed; both are inside the chain the 25-50 ms claim is about, and both stop
    // before the present and the flip, which nothing on this machine can see.
    if let Some(p50) = pct(&a.idle_css_ack_ms, 0.50) {
        let p95 = pct(&a.idle_css_ack_ms, 0.95).unwrap_or(p50);
        println!(
            "V2-2 idle rect→style-observed, css-transform: p50={p50:.1} ms p95={p95:.1} ms \
             (n={}) — every stage from the page reporting its rect to the overlay's renderer \
             beginning the frame that carries the new style. It is a lower bound on \
             rect→painted: the commit, the present and the DWM flip all come after it, and \
             nothing on this machine can see them.",
            a.idle_css_ack_ms.len()
        );
        if let Some(cp50) = pct(&a.idle_css_commit_ms, 0.50) {
            let cp95 = pct(&a.idle_css_commit_ms, 0.95).unwrap_or(cp50);
            let gap = cp50 - p50;
            println!(
                "V2-2 idle rect→frame-committed, css-transform: p50={cp50:.1} ms p95={cp95:.1} \
                 ms; the {gap:+.1} ms between the two ack beats is one renderer frame, which is \
                 the ack instrument's own granularity and the reason the beat above is a \
                 bound rather than the present",
            );
            // The gap is not just noise: it is the overlay document's own frame
            // period, measured. Everything the user sees arrives a whole number of
            // those after the host acted, so it, not the host pipeline, sets the
            // latency floor the 25-50 ms claim is about.
            if gap > 0.5 {
                println!(
                    "V2-2 the overlay document's own frame period, measured off that gap: \
                     {gap:.1} ms = {:.0} fps. The host pipeline above it is {p50:.1} ms, so a \
                     rect→screen latency is this frame period plus whatever the present costs, \
                     never the pipeline alone — which is the mechanism the 25-50 ms claim is \
                     really estimating.",
                    1000.0 / gap,
                );
            }
        }
        let call = if p95 < 25.0 {
            "the measured chain is under the claim's lower edge, so the present plus at least \
             one renderer frame would have to be most of the 25 ms for the claim to hold"
        } else if p50 <= 50.0 {
            "inside the claim's band already, before the present is added"
        } else {
            "past the claim's band before the present is added — the design pays more than planned"
        };
        println!("V2-2 that against the 25-50 ms claim: {call}");
    } else {
        println!(
            "V2-2 idle rect→renderer-ack: NOT MEASURED. Note that this instrument cannot \
             exist for set_bounds either — a bounds move at constant size changes nothing \
             the overlay document can read, so for the shipping strategy the screen is the \
             only instrument there is."
        );
    }

    let err_line = |label: &str, v: &[f64]| {
        let Some(p50) = pct(v, 0.50) else {
            println!("{label}: NO SAMPLES");
            return;
        };
        let p95 = pct(v, 0.95).unwrap_or(p50);
        let max = v.iter().fold(p50, |a, &b| a.max(b));
        let call = if max < 100.0 {
            "far below the 100-500 px estimate: the anchored card keeps its anchor with room to spare"
        } else if p50 <= 500.0 {
            "overlapping the 100-500 px estimate: the estimate is about right here"
        } else {
            "ABOVE the estimate: the card visibly detaches from its anchor"
        };
        println!("{label}: p50={p50:.1} p95={p95:.1} max={max:.1} css px → {call}");
    };
    println!(
        "V2-3 follow error, read off the message stream — how far the anchor had already \
         moved from the rect the card was showing, sampled at every frame the page reported. \
         Measured without a screen readback, so the declined instrument does not touch it:"
    );
    for (label, v) in &a.err_series {
        err_line(&format!("     {label}"), v);
    }
    if !a.paint_series.iter().any(|(_, v)| !v.is_empty()) {
        println!(
            "V2-3 fling rect→painted: NO SAMPLES — the fling runs attribute follow error from \
             the message stream instead, which is the number the 100-500 px claim is about."
        );
    } else {
        for (label, v) in &a.paint_series {
            if !v.is_empty() {
                println!("     {label} rect→painted: {} samples, see the STATS line above", v.len());
            }
        }
    }

    println!(
        "V2-4 cost of one reposition: set_bounds {}, CSS post {}, SetWindowPos on a \
         top-level {}. The first two are the two follow strategies; the third is the other \
         architecture's per-frame cost.",
        shown(median(&a.setbounds_us), "µs"),
        shown(median(&a.csspost_us), "µs"),
        shown(median(&a.setwindowpos_us), "µs")
    );
    println!(
        "V2-5 page↔host message round trip: {} — the floor under every rect→painted number \
         above, since a rect report pays one leg before the host can even start",
        shown(median(&a.rtt_ms), "ms")
    );
    println!(
        "V2-5 device pixel ratio reported by the page: {}, window scale factor: {}. These are \
         the CSS-px to physical-px factors; if they differ, css-px follow error and the \
         physical follow error are not the same number, which is why both are printed.",
        a.dpr, a.scale
    );
    println!(
        "V2-6 samples the probe could not attribute: {u}; rects whose paint was never seen \
         before the run ended: {d}. A missing sample is a missing sample, not a fast frame.",
        u = a.unattributable,
        d = a.dropped
    );
}
