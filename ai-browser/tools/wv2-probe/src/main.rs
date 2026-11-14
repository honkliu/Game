//! `wv2-probe` — measure the three WebView2 behaviours the AI-browser design rests
//! on, on a running webview, and print raw numbers.
//!
//! ```text
//! cargo run --offline --release -- v1
//! cargo run --offline --release -- v2 --samples 200
//! cargo run --offline --release -- v3 --site https://example.com/
//! ```
//!
//! Each mode owns its windows and its script. Nothing here writes into the
//! `native-browser` tree, and the shared WebView2 user-data folder lives under
//! `%TEMP%`, so a probe run cannot touch a real profile.

mod env;
mod harness;
mod hooks;
mod v1;
mod v2;
mod v3;
mod v4;
mod win;

use std::path::PathBuf;

use tao::event_loop::EventLoop;
use tao::window::Window;

use crate::harness::{make_windows, Harness, WindowSpec};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    V1,
    V2,
    V3,
    V4,
}

struct Args {
    mode: Mode,
    sites: v1::Sites,
    /// Samples per measurement loop in V2.
    samples: usize,
    /// The live site V2 and V3 put under the overlay.
    site: String,
    /// Fling distance in CSS px.
    fling: i32,
}

impl Args {
    fn parse() -> Result<Args, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut mode = None;
        let mut sites = v1::Sites::default();
        let mut samples = 120usize;
        let mut site = "https://example.com/".to_string();
        let mut fling = 480i32;
        for a in &raw {
            match a.as_str() {
                "v1" => mode = Some(Mode::V1),
                "v2" => mode = Some(Mode::V2),
                "v3" => mode = Some(Mode::V3),
                "v4" => mode = Some(Mode::V4),
                "--help" | "-h" => return Err(usage()),
                s if s.starts_with("--site-a=") => sites.a = s["--site-a=".len()..].to_string(),
                s if s.starts_with("--site-b=") => sites.b = s["--site-b=".len()..].to_string(),
                s if s.starts_with("--site-c=") => sites.c = s["--site-c=".len()..].to_string(),
                s if s.starts_with("--site=") => site = s["--site=".len()..].to_string(),
                s if s.starts_with("--samples=") => {
                    samples = s["--samples=".len()..]
                        .parse::<usize>()
                        .map_err(|e| format!("--samples: {e}"))?
                        .clamp(5, 5000);
                }
                s if s.starts_with("--fling=") => {
                    fling = s["--fling=".len()..]
                        .parse::<i32>()
                        .map_err(|e| format!("--fling: {e}"))?
                        .clamp(200, 5000);
                }
                other => return Err(format!("unrecognised argument: {other}\n\n{}", usage())),
            }
        }
        Ok(Args {
            mode: mode.ok_or_else(|| format!("no mode given\n\n{}", usage()))?,
            sites,
            samples,
            site,
            fling,
        })
    }
}

fn usage() -> String {
    "usage: wv2-probe <v1|v2|v3|v4> [options]\n\
     \n\
     modes:\n\
     \x20 v1  NavigationId semantics: monotonicity, cross-webview uniqueness,\n\
     \x20     pushState/fragment silence, reload and back/forward id reuse\n\
     \x20 v2  overlay controller cost: process-tree RSS delta, rect-to-repaint\n\
     \x20     latency idle and during a fling, follow error, SetWindowPos cost\n\
     \x20 v3  transparent-region hit-testing over live content: who gets the\n\
     \x20     click, the page underneath or the overlay on top of it\n\
     \x20 v4  the DevTools-Protocol bridge: does CallDevToolsProtocolMethod answer\n\
     \x20     at runtime, with what payload, and does the AX tree carry geometry\n\
     \n\
     options:\n\
     \x20 --site=URL     live page for v2/v3/v4 (default https://example.com/)\n\
     \x20 --samples=N    loop iterations for v2 (default 120)\n\
     \x20 --fling=PX     fling distance for v2 (default 480; it must fit between\n\
     \x20     the viewport top and the fold, or the card leaves the screen)\n\
     \x20 --site-a/-b/-c URLs for v1's navigations\n\
     \n\
     a run needs a desktop session and outbound network for the test sites.\n\
     v3 sends real mouse clicks to the screen under the probe's cursor.\n\
     exit code 2 means an argument, the WebView2 runtime, or the user-data folder failed."
        .to_string()
}

fn main() {
    let args = match Args::parse() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };

    println!("wv2-probe — WebView2 measurement spike");
    let runtime = env::runtime_version();
    if runtime.starts_with("unavailable") {
        eprintln!("webview2 runtime : {runtime}");
        std::process::exit(2);
    }
    println!("webview2 runtime : {runtime}");
    println!("pid              : {}", std::process::id());

    // The event loop is created first: it owns the thread that every window,
    // environment and controller is then created on.
    let event_loop: EventLoop<()> = EventLoop::new();

    let data_dir = match data_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("user data folder: {e}");
            std::process::exit(2);
        }
    };
    println!("user data        : {}", data_dir.display());

    // Window counts are per mode, and every window a step needs later is built
    // here because tao 0.37 cannot create one on a running loop. V2 and V3 need
    // one *large* window: the overlay they measure is a child of it, full-bleed,
    // so the window has to be big enough for a 300x140 card to travel in and for
    // a page to be worth scrolling. 1060x660 logical keeps the whole window on a
    // 1664x1109 physical / 144 DPI screen.
    let specs = match args.mode {
        Mode::V1 => vec![
            WindowSpec::new("wv2-probe A", 20, 20, 820, 600),
            WindowSpec::new("wv2-probe B", 860, 20, 780, 600),
            WindowSpec::new("wv2-probe C", 20, 640, 820, 440),
            WindowSpec::new("wv2-probe D", 860, 640, 780, 440),
        ],
        Mode::V2 => vec![
            WindowSpec::new("wv2-probe page+overlay", 20, 20, 1060, 660),
            // Hidden until V2.8: a borderless top-level whose only job is to be
            // moved, so `SetWindowPos` is priced on a real window rather than on
            // the one under measurement.
            WindowSpec::overlay("wv2-probe scratch", 620, 320, 420, 300),
        ],
        Mode::V3 => vec![WindowSpec::new("wv2-probe page+overlay", 20, 20, 1060, 660)],
        // V4 needs no overlay and no particular size: it asks the runtime for data
        // about a page, and never reads the screen.
        Mode::V4 => vec![WindowSpec::new("wv2-probe page", 20, 20, 900, 600)],
    };
    let windows: Vec<Window> = make_windows(&event_loop, &specs);
    for (i, w) in windows.iter().enumerate() {
        let inner = w.inner_size();
        println!(
            "window {i}: inner={}x{} scale={}",
            inner.width,
            inner.height,
            w.scale_factor()
        );
    }

    // The environment comes *after* the first window on purpose: tao calls
    // `OleInitialize` (which is what puts the thread in an single-threaded
    // apartment) while creating a window, and
    // `CreateCoreWebView2EnvironmentWithOptions` fails with CO_E_NOTINITIALIZED
    // (0x800401F0) before that has happened.
    let shared_env = match env::shared_environment(&data_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("environment: {e}");
            std::process::exit(2);
        }
    };

    let mut harness = Harness::new(shared_env, windows, data_dir);
    let steps = match args.mode {
        Mode::V1 => v1::program(args.sites, 4),
        Mode::V2 => v2::program(v2::Cfg {
            samples: args.samples,
            site: args.site.clone(),
            fling: args.fling,
        }),
        Mode::V3 => v3::program(args.site.clone()),
        Mode::V4 => v4::program(args.site.clone()),
    };
    harness.load(steps);
    harness::spin(event_loop, harness)
}

/// WebView2's user-data folder for the probe, kept out of the repository.
fn data_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("wv2-probe-udata");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Ok(dir)
}
