//! V3 — the one thing nobody has observed: per-pixel transparency with correct
//! hit-testing in the transparent parts, over live web content.
//!
//! Not in question any more, because the shipping code already proves it: a second
//! controller in the same window works, shares the environment, and anchors to an
//! arbitrary host rect (`app.rs:2388` builds the overlay, `app.rs:649` moves it to an
//! x/y/w the host chose). What has never been tested is the transparent case, because
//! today's overlay is sized to the card and painted edge to edge, so no click ever has
//! to fall through it.
//!
//! So each variant answers two questions, and both matter:
//!
//! * does a click in a **fully transparent** region reach the page underneath, or does
//!   the overlay eat it? — a click-through-only overlay cannot show anything;
//! * does the **opaque card** region still get its own clicks? — an overlay that passes
//!   everything through cannot host a composer.
//!
//! Clicks are injected with `SendInput`, which goes through the real hit-tester;
//! `PostMessage` would let the probe ask a window nicely instead of asking the window
//! manager where the click belongs, which is the exact question here. Both webviews log
//! `mousedown` on themselves, so "who got it" is what the pages report, not what the host
//! assumed. `WindowFromPoint` is printed alongside as the window-manager's own answer, and
//! the card's pixel is read back so a variant that achieves click-through by making the
//! card invisible cannot pass as a success.

use std::cell::RefCell;
use std::rc::Rc;

use crate::harness::{action, expect_record, mark, sleep_ms, step, Step, Tick};
use crate::win;

const PAGE_VIEW: usize = 0;
const OVERLAY_VIEW: usize = 1;

/// The card's rect inside the overlay, window-client logical px.
const CARD: (i32, i32, i32, i32) = (60, 60, 300, 140);

/// The shapes to test. Each is a complete state the overlay window can be in; the
/// setup of one variant does not depend on the previous one having left anything set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Variant {
    /// Control: overlay sized to the card. No transparent region exists, so clicks
    /// outside the card reach the page because the window is not there. This is what
    /// ships today; it is the baseline every other variant has to beat.
    CardSized,
    /// Full-client transparent overlay, nothing else — the shape an anchored card needs
    /// if the card floats inside a larger surface (shadow, backdrop, resize handles).
    FullClient,
    /// Full client + `WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE`, the classic
    /// click-through recipe.
    TransparentLayered,
    /// Full client + `WS_EX_TRANSPARENT` alone.
    TransparentOnly,
    /// Full client + `WS_EX_NOACTIVATE` alone, to separate "does not steal focus" from
    /// "does not take clicks".
    NoActivateOnly,
    /// Full client, but the window region is the card rect: the rest of the surface is
    /// absent to the hit-tester rather than transparent.
    RegionCard,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Variant::CardSized => "overlay sized to the card (what ships)",
            Variant::FullClient => "full-client transparent overlay",
            Variant::TransparentLayered => "full client + WS_EX_TRANSPARENT|LAYERED|NOACTIVATE",
            Variant::TransparentOnly => "full client + WS_EX_TRANSPARENT",
            Variant::NoActivateOnly => "full client + WS_EX_NOACTIVATE",
            Variant::RegionCard => "full client + window region = card rect",
        }
    }

    /// Whether the overlay should cover the whole client area in this variant.
    fn full_client(self) -> bool {
        !matches!(self, Variant::CardSized)
    }
}

#[derive(Clone, Debug)]
struct Row {
    variant: &'static str,
    /// Readback of the card's centre pixel: is it even visible in this variant?
    card_rgb: Option<(u8, u8, u8)>,
    /// Readback at the transparent point: must not be the card's red.
    clear_rgb: Option<(u8, u8, u8)>,
    /// Which probe window the hit-test says owns each point — the direct answer to
    /// "does the click land on the page or on the overlay". `None` is no probe
    /// window at all (the desktop, or a foreign window above us).
    owner_card: Option<usize>,
    owner_clear: Option<usize>,
    /// Whether each of the two clicks for this variant actually reached the input
    /// queue. When one did not, the two counts it produced are zeros that mean
    /// nothing, and the verdict has to say so instead of reporting a failure of the
    /// design.
    clicked_clear: bool,
    clicked_card: bool,
    /// Whether the probe's own top-level window was the window the OS put the two
    /// sample points in. When it was not, the hit-test rows for this variant are
    /// about somebody else's window and cannot be graded at all.
    occluded: bool,
    page_at_clear: usize,
    overlay_at_clear: usize,
    page_at_card: usize,
    overlay_at_card: usize,
}

#[derive(Default)]
struct Acc {
    rows: Vec<Row>,
    client_logical: (i32, i32),
    scale: f64,
    /// Which readback served the two colour columns.
    instrument: String,
}

type Shared = Rc<RefCell<Acc>>;

/// Where each view points its mousedown logger at the host, and the click logger the
/// page needs. Kept here so V3 does not depend on V2 having run.
fn page_hook_js() -> String {
    r#"(function(){
  if (window.__v3Hook) return 'already';
  document.addEventListener('mousedown', function(e){
    window.chrome.webview.postMessage(JSON.stringify({
      kind: 'page_mousedown', x: e.clientX, y: e.clientY,
      target: (e.target && (e.target.id || e.target.tagName)) || '?',
      page: performance.now()
    }));
  }, true);
  document.addEventListener('click', function(e){
    window.chrome.webview.postMessage(JSON.stringify({kind: 'page_click', x: e.clientX, y: e.clientY}));
  }, true);
  window.__v3Hook = true;
  return 'page-hooked';
})()"#.to_string()
}

fn overlay_doc_js() -> String {
    r#"(function(){
  document.documentElement.style.cssText = 'background:transparent;margin:0';
  document.body.style.cssText = 'margin:0;padding:0;background:transparent;overflow:hidden;width:100%;height:100%';
  var card = document.createElement('div');
  card.id = 'probe-card';
  card.style.cssText = 'position:absolute;left:0;top:0;width:300px;height:140px;background:rgb(255,0,0)';
  document.body.appendChild(card);
  window.__card = card;
  window.__place = function(x, y){
    card.style.transform = 'translate(' + x + 'px,' + y + 'px)';
    return 'placed';
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

pub fn program(site: String) -> Vec<Step> {
    let acc: Shared = Rc::new(RefCell::new(Acc::default()));
    let mut s: Vec<Step> = Vec::new();

    s.push(mark("V3.0 live page under a full-client transparent overlay"));
    s.push(step(move |h| match h.build_in(PAGE_VIEW, 0, "PAGE", Some(&site), None, false) {
        Ok(()) => Tick::Done,
        Err(e) => {
            println!("ERROR building the page view: {e}");
            Tick::Done
        }
    }));
    s.push(expect_record("PAGE", "NavigationCompleted", 20_000, "live page under the overlay"));
    s.push(sleep_ms(1500));
    s.push(action("hook the page's own mousedown logger", |h| {
        h.run(PAGE_VIEW, &page_hook_js())
    }));

    s.push(build_overlay(OVERLAY_VIEW, 0, "OVLY", CARD));
    s.push(action("install the overlay document", |h| {
        h.run(OVERLAY_VIEW, &overlay_doc_js())?;
        h.run(OVERLAY_VIEW, &format!("window.__place({}, {})", CARD.0, CARD.1))
    }));
    s.push(sleep_ms(1200));
    s.push(action("record the coordinate space once", {
        let acc = acc.clone();
        move |h| {
            let c = h.client_of(OVERLAY_VIEW);
            let scale = h.scale_of(OVERLAY_VIEW);
            let mut a = acc.borrow_mut();
            a.client_logical = (
                (c.2 as f64 / scale).round() as i32,
                (c.3 as f64 / scale).round() as i32,
            );
            a.scale = scale;
            println!(
                "GEOM window client = {}x{} physical = {}x{} logical, scale {scale}",
                c.2, c.3, a.client_logical.0, a.client_logical.1
            );
            println!(
                "GEOM card rect = {CARD:?} logical; the transparent probe point is the bottom \
                 middle of the client, which is inside the overlay and outside the card"
            );
            println!(
                "GEOM page view hwnd={}, overlay view hwnd={}",
                h.view_hwnd_value(PAGE_VIEW).unwrap_or(0),
                h.view_hwnd_value(OVERLAY_VIEW).unwrap_or(0)
            );
            Ok(())
        }
    }));

    for v in [
        Variant::CardSized,
        Variant::FullClient,
        Variant::TransparentLayered,
        Variant::TransparentOnly,
        Variant::NoActivateOnly,
        Variant::RegionCard,
    ] {
        s.push(mark(&format!("V3.x variant: {}", v.name())));
        s.push(apply_variant(v));
        s.push(sleep_ms(500));
        s.push(observe_geometry(v, acc.clone()));
        s.push(click_and_collect(v, Point::Clear, acc.clone()));
        s.push(click_and_collect(v, Point::Card, acc.clone()));
        s.push(sleep_ms(300));
    }

    s.push(mark("V3.9 resize under the overlay: does the anchored card hold position"));
    s.push(apply_variant(Variant::RegionCard));
    s.push(sleep_ms(400));
    s.push(resize_hold(acc.clone()));

    s.push(step({
        let acc = acc.clone();
        move |_h| {
            verdict(&acc.borrow());
            Tick::Done
        }
    }));
    s
}

/// The card's centre and the transparent point, in window-client logical px.
#[derive(Clone, Copy)]
enum Point {
    Card,
    Clear,
}

fn point_logical(acc: &Acc, p: Point) -> (i32, i32) {
    match p {
        Point::Card => (CARD.0 + CARD.2 / 2, CARD.1 + CARD.3 / 2),
        // Bottom middle of the client area: inside a full-client overlay, far outside the
        // card, and over the page in every case.
        Point::Clear => (
            (acc.client_logical.0 / 2).max(40),
            (acc.client_logical.1 - 60).max(CARD.1 + CARD.3 + 40),
        ),
    }
}

fn build_overlay(
    view: usize,
    window: usize,
    label: &'static str,
    bounds: (i32, i32, i32, i32),
) -> Step {
    step(move |h| match h.build_in(view, window, label, Some("about:blank"), Some(bounds), true) {
        Ok(()) => Tick::Done,
        Err(e) => {
            println!("ERROR building the overlay view: {e}");
            Tick::Done
        }
    })
}

/// Put the overlay window into one variant's state from scratch: geometry first, then
/// ex-styles, then region, each reset before being set so a variant cannot inherit a
/// previous variant's setting and look like a pass.
fn apply_variant(v: Variant) -> Step {
    step(move |h| {
        let c = h.client_of(OVERLAY_VIEW);
        let scale = h.scale_of(OVERLAY_VIEW);
        let cw = (c.2 as f64 / scale).round() as i32;
        let ch = (c.3 as f64 / scale).round() as i32;
        use windows::Win32::UI::WindowsAndMessaging::{
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
        };
        let none = windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0);
        let geo = if v.full_client() {
            h.move_view(OVERLAY_VIEW, 0, 0, cw, ch)
        } else {
            h.move_view(OVERLAY_VIEW, CARD.0, CARD.1, CARD.2, CARD.3)
        };
        match geo {
            Ok(us) => println!("VARIANT geometry set in {us:.0} µs → {:?}", if v.full_client() { "full client" } else { "card rect" }),
            Err(e) => println!("ERROR setting the geometry: {e}"),
        }
        // Reset first: styles are cumulative across variants otherwise, and the whole
        // point is to attribute an effect to one style.
        let added = match v {
            Variant::TransparentLayered => {
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE
            }
            Variant::TransparentOnly => WS_EX_TRANSPARENT,
            Variant::NoActivateOnly => WS_EX_NOACTIVATE,
            _ => none,
        };
        let style = h.view_ex_style(OVERLAY_VIEW, added, WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE);
        let region = match v {
            Variant::RegionCard => h.set_view_region_rect(
                OVERLAY_VIEW,
                // The card rect in the overlay window's own client coordinates. For a
                // full-client overlay that is the same origin as the browser window's
                // client area; for the card-sized control the whole window is the card.
                (CARD.0, CARD.1, CARD.2, CARD.3),
            ),
            _ => h.set_view_region(OVERLAY_VIEW, None),
        };
        println!(
            "VARIANT {} → asked ex_style={:#010x}, window now holds {:#010x}, region {} \
             (SetWindowRgn {region:?}), overlay_hwnd={}",
            v.name(),
            added.0,
            style.unwrap_or(0),
            // `Some(true)` is "SetWindowRgn succeeded", not "a region is set": the
            // reset call reports success too. Print the intent alongside it.
            match v {
                Variant::RegionCard => "= card rect (set)",
                _ => "= none (cleared)",
            },
            h.view_hwnd_value(OVERLAY_VIEW).unwrap_or(0)
        );
        if style.unwrap_or(0) as u32 & added.0 != added.0 {
            println!(
                "VARIANT note: the styles asked for were NOT applied — SetWindowLongPtrW \
                 refused the whole mask. WS_EX_LAYERED is a top-level-only style and this \
                 overlay is a child window, so the classic transparent+layered click-through \
                 recipe cannot even be installed here; the hit-test rows below for this \
                 variant describe a window with no such style."
            );
        }
        // Make sure the overlay is actually on top of the page in the same window, or a
        // click could reach the page simply because something else is above it.
        h.set_view_visible(OVERLAY_VIEW, true).ok();
        Tick::Done
    })
}

/// What the screen looks like at the two probe points, and what the window manager says
/// owns them, before any click is sent.
fn observe_geometry(v: Variant, acc: Shared) -> Step {
    step(move |h| {
        // Raise first, then measure: `WindowFromPoint` reports whichever window is
        // on top, and 14 other `msedgewebview2.exe` hosts are live on this desktop.
        h.raise(0);
        let (clx, cly) = point_logical(&acc.borrow(), Point::Clear);
        let (cax, cay) = point_logical(&acc.borrow(), Point::Card);
        let (csx, csy) = h.view_point_on_screen(OVERLAY_VIEW, cax, cay);
        let (xs, ys) = h.view_point_on_screen(OVERLAY_VIEW, clx, cly);
        let card_rgb = h.probe.pixel(csx, csy);
        let clear_rgb = h.probe.pixel(xs, ys);
        let wfp_card = h.window_at(csx, csy);
        let wfp_clear = h.window_at(xs, ys);
        // Resolved through the ancestor chain: `WindowFromPoint` answers with
        // WebView2's own surface window, which is a child of the handle wry gives
        // back, so comparing the raw number against the view's HWND never matches
        // and the answer was thrown away.
        let owner_card = h.view_at(csx, csy);
        let owner_clear = h.view_at(xs, ys);
        // Occlusion: both points have to be inside the probe's own top-level, or
        // the hit-test below belongs to a window this probe does not own.
        let root = h.window_root(0);
        let occluded = h.root_at(csx, csy) != root && h.root_at(xs, ys) != root;
        println!(
            "GEOMETRY card pixel ({csx},{csy}) = {card_rgb:?}   transparent pixel ({xs},{ys}) = {clear_rgb:?}",
        );
        println!("GEOMETRY window_from_point(card) = {}", h.describe_window(wfp_card));
        println!("GEOMETRY window_from_point(transparent) = {}", h.describe_window(wfp_clear));
        println!(
            "GEOMETRY hit-test owner: card → {}, transparent → {}",
            owner_name(owner_card),
            owner_name(owner_clear)
        );
        if occluded {
            println!(
                "GEOMETRY OCCLUDED: neither sample point is inside this probe's own top-level \
                 window (root hwnd={root}); the window the OS put there is {}. A foreign \
                 window is above the overlay, so the owners above are that window's, not this \
                 overlay's, and the variant below is graded NOT MEASURED.",
                h.describe_window(wfp_clear)
            );
        }
        let mut a = acc.borrow_mut();
        if a.rows.is_empty() {
            println!("PROBE instruments: {}", h.probe.diagnostic());
        }
        a.instrument = h.probe.source().to_string();
        a.rows.push(Row {
            variant: v.name(),
            card_rgb,
            clear_rgb,
            owner_card,
            owner_clear,
            clicked_clear: false,
            clicked_card: false,
            occluded,
            page_at_clear: 0,
            overlay_at_clear: 0,
            page_at_card: 0,
            overlay_at_card: 0,
        });
        Tick::Done
    })
}

/// The transparent point is measured against the card's red, so a variant that
/// "passes clicks" by not painting the card cannot pass as a success.
fn is_card_red(rgb: Option<(u8, u8, u8)>) -> bool {
    rgb.map(|c| c.0 > 150 && c.1 < 110 && c.2 < 110).unwrap_or(false)
}

fn owner_name(v: Option<usize>) -> String {
    match v {
        Some(v) if v == OVERLAY_VIEW => "THE OVERLAY".to_string(),
        Some(v) if v == PAGE_VIEW => "the page".to_string(),
        Some(v) => format!("probe view {v}"),
        None => "no probe window".to_string(),
    }
}

/// One real click at one point, then what each page says it received.
fn click_and_collect(v: Variant, p: Point, acc: Shared) -> Step {
    let label = match p {
        Point::Card => "card",
        Point::Clear => "transparent",
    };
    let mut phase = 0u8;
    let mut start = 0.0f64;
    let mut screen = (0i32, 0i32);
    let mut delivered = false;
    step(move |h| {
        match phase {
            0 => {
                h.clear_inbox();
                // Same reason as in `observe_geometry`: the click must be aimed at
                // this probe's window, not at whatever happens to be on top.
                h.raise(0);
                let (lx, ly) = point_logical(&acc.borrow(), p);
                screen = h.view_point_on_screen(OVERLAY_VIEW, lx, ly);
                // The hit-test answer for this exact instant, taken immediately before the
                // click, in case a style change above has not landed yet.
                println!(
                    "CLICK {label} at screen ({},{}) — window_from_point says {}, owned by {}",
                    screen.0,
                    screen.1,
                    h.describe_window(h.window_at(screen.0, screen.1)),
                    owner_name(h.view_at(screen.0, screen.1))
                );
                // The cursor has to be where the click is aimed before the queue sees
                // the button, and both halves of that have to be reported: an
                // undelivered click looks exactly like a click the overlay ate.
                let moved = win::move_cursor(screen.0, screen.1);
                let (injected, err, cursor) = win::click_at(screen.0, screen.1);
                delivered = injected == 3 && cursor == screen;
                println!(
                    "CLICK {label}: SetCursorPos {}, SendInput accepted {injected}/3 events \
                     (GDI error {err}), cursor now at {cursor:?} of the {screen:?} aimed at",
                    if moved { "ok" } else { "FAILED" }
                );
                if !delivered {
                    println!(
                        "CLICK {label}: NOT DELIVERED — the counts below are zero because no \
                         click reached the queue, not because a window refused it. The \
                         hit-test line above is still the window manager's own answer."
                    );
                }
                start = win::qpc_ms();
                phase = 1;
                Tick::Running
            }
            _ => {
                if win::ms_since(start) < 700.0 {
                    return Tick::Running;
                }
                let page = h.take_msgs("PAGE", "page_mousedown");
                let ovl = h.take_msgs("OVLY", "overlay_mousedown");
                println!(
                    "RESULT {label}: page logged {} mousedown(s) {:?}, overlay logged {} {:?}",
                    page.len(),
                    page.iter().map(|m| (m.json.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32, m.json.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32, m.json.get("target").and_then(|v| v.as_str()).unwrap_or("?").to_string())).collect::<Vec<_>>(),
                    ovl.len(),
                    ovl.iter().map(|m| (m.json.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32, m.json.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32, m.json.get("target").and_then(|v| v.as_str()).unwrap_or("?").to_string())).collect::<Vec<_>>()
                );
                // Move the cursor away so the pointer itself is not in the next sample's
                // readback.
                win::move_cursor(screen.0, screen.1 + 400);
                let mut a = acc.borrow_mut();
                if let Some(row) = a.rows.iter_mut().rev().find(|r| r.variant == v.name()) {
                    match p {
                        Point::Clear => {
                            row.clicked_clear = delivered;
                            row.page_at_clear = page.len();
                            row.overlay_at_clear = ovl.len();
                        }
                        Point::Card => {
                            row.clicked_card = delivered;
                            row.page_at_card = page.len();
                            row.overlay_at_card = ovl.len();
                        }
                    }
                }
                Tick::Done
            }
        }
    })
}

/// Resize the browser window with the overlay up and see whether the card is still where
/// its anchor is. The overlay does not know the window moved unless the host tells it,
/// which is the point of the measurement.
fn resize_hold(acc: Shared) -> Step {
    let mut phase = 0u8;
    let mut start = 0.0f64;
    let mut before = (0i32, 0i32);
    step(move |h| {
        match phase {
            0 => {
                let (cax, cay) = point_logical(&acc.borrow(), Point::Card);
                before = h.view_point_on_screen(OVERLAY_VIEW, cax, cay);
                println!("RESIZE card's screen pixel before the resize = {before:?}");
                h.grow_window(0, 80, 60);
                start = win::qpc_ms();
                phase = 1;
                Tick::Running
            }
            _ => {
                if win::ms_since(start) < 800.0 {
                    return Tick::Running;
                }
                let (cax, cay) = point_logical(&acc.borrow(), Point::Card);
                let now = h.view_point_on_screen(OVERLAY_VIEW, cax, cay);
                let rgb = h.probe.pixel(now.0, now.1);
                println!(
                    "RESIZE after growing the browser window by 120 physical px: the overlay's \
                     card rect now maps to screen {now:?} (was {before:?}); card pixel there = \
                     {rgb:?}"
                );
                println!(
                    "RESIZE note: this is where the overlay drew the card after the window \
                     moved. Whether the page's own anchor moved with it is the difference \
                     between 'holds position' and 'moves with the window' — V2.7 measures the \
                     same quantity against a scrolling anchor."
                );
                Tick::Done
            }
        }
    })
}

fn verdict(a: &Acc) {
    println!("\n=== V3 VERDICTS: transparent-region hit-testing over live content ===");
    if a.rows.is_empty() {
        println!("NOT MEASURED (no variant ran)");
        return;
    }
    println!(
        "The two colour columns were read through the {} instrument by rendering one window \
         with PrintWindow. That instrument has no alpha compositing: it can confirm the card \
         was painted, but a transparent region of the overlay reads as whatever the overlay's \
         own surface holds, never as the page underneath — so the hit-test owners are the \
         answers here, not the colours.",
        if a.instrument.is_empty() {
            "unused"
        } else {
            a.instrument.as_str()
        }
    );
    let any_clicks = a.rows.iter().any(|r| r.clicked_clear || r.clicked_card);
    if !any_clicks {
        println!(
            "The injected-click instrument delivered nothing on this desktop (see the CLICK \
             lines: SendInput accepted 0 events), so the mousedown columns below are NOT \
             MEASURED and are printed as '-' rather than as 0. What *is* measured is \
             WindowFromPoint plus each result's ancestor chain: that is the same hit-test the \
             window manager runs before it delivers a click, so who owns the point is a real \
             answer even on a machine with no pointer."
        );
    }
    println!(
        "{:<52} {:>13} {:>13} {:>7} {:>7}",
        "variant", "clear owner", "card owner", "card dp", "clear dp"
    );
    for r in &a.rows {
        println!(
            "{:<52} {:>13} {:>13} {:>7} {:>7}",
            r.variant,
            owner_name(r.owner_clear),
            owner_name(r.owner_card),
            if is_card_red(r.card_rgb) { "red" } else { "not red" },
            if is_card_red(r.clear_rgb) { "RED" } else { "not red" },
        );
    }
    if any_clicks {
        println!();
        println!(
            "{:<52} {:>9} {:>9} {:>9} {:>9}",
            "variant (delivered clicks only)", "clear→pg", "clear→ov", "card→pg", "card→ov"
        );
        for r in a.rows.iter().filter(|r| r.clicked_clear || r.clicked_card) {
            println!(
                "{:<52} {:>9} {:>9} {:>9} {:>9}",
                r.variant,
                r.page_at_clear,
                r.overlay_at_clear,
                r.page_at_card,
                r.overlay_at_card
            );
        }
    }
    println!();
    if a.rows.iter().all(|r| r.occluded) {
        println!(
            "V3 NOT MEASURED: every variant was sampled with a foreign window above the \
             probe's, so no hit-test row describes this overlay. Close or raise the other \
             window and re-run; the printed OCCLUDED lines name what was there."
        );
        return;
    }
    let mut usable: Vec<&str> = Vec::new();
    for r in &a.rows {
        if r.occluded {
            println!(
                "{}: NOT MEASURED — a window this probe does not own was above both sample \
                 points, so its hit-test rows are that window's. Re-run with the probe on \
                 top (see the GEOMETRY OCCLUDED line above).",
                r.variant
            );
            continue;
        }
        // The shipping question has exactly two halves, and the hit-test answers both
        // without needing a pointer: the transparent region must belong to the page
        // underneath, and the card's own rect must belong to the overlay.
        let passes = r.owner_clear == Some(PAGE_VIEW);
        let captures = r.owner_card == Some(OVERLAY_VIEW);
        let card_visible = is_card_red(r.card_rgb);
        let verdict = match (passes, captures, card_visible) {
            (true, true, true) => "USABLE — transparent parts pass through, the card keeps its clicks",
            (true, true, false) => "SUSPECT — hit-testing is right but the card is not visibly there",
            (true, false, _) => "NOT USABLE FOR A COMPOSER — passes clicks but the card gets none",
            (false, true, _) => "NOT USABLE OVER LIVE CONTENT — the transparent region eats clicks",
            (false, false, _) => "NOT USABLE — neither the page nor the card gets the clicks",
        };
        if passes && captures && card_visible {
            usable.push(r.variant);
        }
        let click_note = match (r.clicked_clear, r.clicked_card) {
            (true, true) => format!(
                ", clicks delivered confirm it (transparent: page {} / overlay {}; card: \
                 overlay {} / page {})",
                r.page_at_clear, r.overlay_at_clear, r.overlay_at_card, r.page_at_card
            ),
            _ => ", click delivery NOT MEASURED on this desktop".to_string(),
        };
        println!(
            "{}: transparent→page {} (hit-test {}), card→overlay {} (hit-test {}), card \
             visible {}{}{click_note} → {verdict}",
            r.variant,
            if passes { "HOLDS" } else { "FAILS" },
            owner_name(r.owner_clear),
            if captures { "HOLDS" } else { "FAILS" },
            owner_name(r.owner_card),
            if card_visible { "yes" } else { "NO" },
            if is_card_red(r.clear_rgb) {
                ", transparent pixel shows the card's red"
            } else {
                ""
            }
        );
    }
    println!(
        "\nV3 ANSWER: shapes that keep both properties: {}.",
        if usable.is_empty() {
            "none measured — the transparent-card design needs a different mechanism".to_string()
        } else {
            usable.join("; ")
        }
    );
}
