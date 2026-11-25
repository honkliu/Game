//! native-browser entry point.
//!
//! The first thing we do is capture a monotonic anchor: every W1/W2 number in
//! the diagnostics report is measured from here, not from "whenever the event
//! loop happened to start".
//!
//! Usage:
//!   browser.exe                       — normal launch
//!   browser.exe --self-test[=URL]     — load URL (default: the new tab page),
//!                                       write `<data>\self-test.json`, exit
//!   browser.exe --driver=NAME         — run the scripted scenario NAME through
//!                                       the real IPC dispatch path, write
//!                                       `<data>\driver.json`, exit
//!   browser.exe --proof               — Release 0 harness: open distinct-origin
//!                                       tabs (4 by default) on one event loop, print
//!                                       one PASS/FAIL line per measured budget to
//!                                       stdout and `<data>\proof.txt`, and exit 1
//!                                       on any FAIL. Run it from a console build
//!                                       (`cargo run -p browser -- --proof`); a
//!                                       `windows_subsystem` release build has no
//!                                       console, so only the file gets written.
//!                                       It takes a *private* profile (see
//!                                       [`private_proof_profile`]), wipes it before
//!                                       sampling (see [`wipe_proof_profile`]), and
//!                                       exits 2 on a malformed flag rather than
//!                                       quietly running the default mode.
//!   browser.exe --proof=N             — open N tabs (capped at the harness's list of
//!                                       distinct origins)
//!   browser.exe --proof=+no_tab_sleep — additionally do not destroy background tabs'
//!                                       webviews during the run, so "+N tabs" means
//!                                       "+N live pages" and C3 / C6 become
//!                                       measurable. Off by default; never reachable
//!                                       outside `--proof`.
//!   browser.exe --proof=+no_overlay   — start without the AI overlay webview and
//!                                       build it as a measured step, so C5 prices the
//!                                       overlay as a delta instead of having it
//!                                       inside every sample. Off by default.
//!   browser.exe --proof=+self_refresh — open one tab on a page that reloads itself in
//!                                       JavaScript and watch that same page for two
//!                                       300-second windows with no command in flight,
//!                                       the first with it backgrounded and the second
//!                                       with it in the foreground, so C8 can tell "the
//!                                       page refreshes too rarely for the window" apart
//!                                       from "a background page is never allowed to
//!                                       refresh" and C9 can price idle drift. Needs
//!                                       `+no_tab_sleep` to measure anything: a slept
//!                                       background tab has no webview left to refresh.
//!                                       Adds ~11 minutes to the run. Off by default.
//!                                       Flags combine:
//!                                       `--proof=4+no_tab_sleep+no_overlay`.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;

use browser::{ai, app, platform, sys};

fn main() {
    let start_tick = sys::process_uptime_ms();

    // Test harness: drive a scripted IPC scenario on the real app, then exit.
    // Parsed before anything else runs, because `--proof` has to relocate the
    // profile before the first path is resolved (see `private_proof_profile`).
    let driver = std::env::args()
        .find_map(|a| a.strip_prefix("--driver=").map(str::to_string))
        // The proof harness is a driver scenario with a verdict attached, so it
        // enters through the same door. `--proof=6` becomes the driver name
        // `proof:6`, which is what `ai::proof` parses the tab count out of.
        .or_else(|| {
            std::env::args().find_map(|a| {
                a.strip_prefix("--proof=")
                    .map(|n| format!("proof:{n}"))
                    .or_else(|| (a == "--proof").then(|| "proof".to_string()))
            })
        });
    let proof = driver.as_deref().is_some_and(|d| d.starts_with("proof"));
    if proof && !cfg!(windows) {
        eprintln!("--proof measures Windows WebView2 budgets; use --self-test on macOS.");
        std::process::exit(2);
    }
    if proof {
        // Redirect the whole data directory before anything resolves a path. See
        // `private_proof_profile`.
        private_proof_profile();
        // Then the measurement mode and the wipe, both before the event loop exists.
        prepare_proof_run(driver.as_deref().unwrap_or("proof"));
    }

    // A5: hard gate on the WebView2 Evergreen runtime, with a dialog and a
    // download hint rather than a panic deep inside tao.
    if let Err(e) = platform::ensure_runtime() {
        eprintln!("River: {} unavailable: {e}", platform::ENGINE_NAME);
        std::process::exit(1);
    }

    let self_test_url = std::env::args().find_map(|a| {
        a.strip_prefix("--self-test=")
            .map(str::to_string)
            .or_else(|| (a == "--self-test").then(|| "native://newtab".to_string()))
    });

    if proof {
        println!(
            "proof: {} environment(s) constructed before the app started",
            ai::environments_constructed()
        );
    }
    app::run_with(start_tick, self_test_url, driver);
}

/// Give a `--proof` run a data directory of its own, and therefore a WebView2
/// user-data-folder of its own.
///
/// Why: the WebView2 runtime gives one browser process per *user-data-folder*, not
/// per host process. Measured on this machine — a `browser.exe` launched while
/// another instance held `%LOCALAPPDATA%\native-browser\webview2` got **no**
/// `msedgewebview2.exe` children at all; the new renderer processes were parented
/// to the *older* instance's browser process. Any harness that roots its process
/// walk at its own pid would then measure an empty tree and report zero cost,
/// which is a wrong answer rather than a missing one.
///
/// The redirect is done by moving `LOCALAPPDATA`, because `app::data_dir()` reads
/// it on every call: one write here relocates the WebView2 user-data-folder, the
/// SQLite store, the log and the harness's own `proof.txt` together, leaving the
/// real profile untouched. Called before `ensure_runtime` and before the event
/// loop exists, so no thread can observe a half-redirected environment.
fn private_proof_profile() {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let private = base.join("native-browser-proof");
    std::env::set_var("LOCALAPPDATA", &private);
    std::env::set_var("AI_BROWSER_DATA_DIR", private.join("native-browser"));
    println!("proof: data directory redirected to {}", private.display());
}

/// Arm the measurement mode for this run and start it from a known-empty profile.
///
/// Both halves exist so a report describes the run that produced it:
///
/// * an unrecognised flag exits non-zero instead of falling back to the default
///   mode, because the report header states which mode ran, and a typo would make
///   that statement false;
/// * the profile is wiped first, because session restore decides how much was
///   already loaded when sample 0 is taken. Measured on this machine: the same
///   scenario measured 591.6 MB of process tree from a wiped profile and 822.6 MB
///   from one that restored 5 tabs — every verdict identical, every absolute number
///   different. See [`wipe_proof_profile`].
fn prepare_proof_run(driver_name: &str) {
    let config = ai::mode::Config::parse(driver_name, ai::proof::MAX_TABS);
    if !config.unknown.is_empty() {
        eprintln!(
            "proof: unknown flag(s) {} in `{driver_name}`; expected \
             --proof[=N][+{}][+{}][+{}] — refusing to run the default mode instead",
            config.unknown.join(", "),
            ai::mode::NO_TAB_SLEEP,
            ai::mode::NO_OVERLAY,
            ai::mode::SELF_REFRESH,
        );
        // 2, not 1: 1 is reserved for "a check failed", so a script can tell a
        // mis-typed command from a red gate.
        std::process::exit(2);
    }
    wipe_proof_profile();
    println!(
        "proof: mode tabs={} tab_sleep={} overlay_at_startup={} self_refresh={}",
        config.tabs,
        if config.keep_background_tabs_alive { "off" } else { "on" },
        config.overlay_at_startup,
        config.self_refresh_tab,
    );
    ai::mode::install(config);
}

/// Delete the harness's private profile before it samples anything.
///
/// This is not housekeeping. WebView2 persists session state in the user-data-folder
/// and the shell restores it, so a leftover profile makes sample 0 "however many tabs
/// the last run left" rather than "one new tab page". The harness's verdicts are
/// deltas and survive that (measured: identical verdicts both ways), but its absolute
/// working-set numbers do not, and those are the figures `tools/baseline.md` quotes
/// and later budgets are set against. So the harness does the one thing that makes
/// them comparable, and check C7 fails loudly if it did not happen.
///
/// A failure to delete is reported, not swallowed: it usually means another proof
/// instance still holds this user-data-folder, which is also the condition that makes
/// the process tree we count belong to that instance (check C0).
fn wipe_proof_profile() {
    let dir = app::data_dir();
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => println!("proof: profile wiped ({})", dir.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("proof: profile already absent ({})", dir.display())
        }
        Err(e) => println!(
            "proof: WARNING could not wipe {} ({e}); restored session state will shift \
             every absolute MB in this report and C7 will fail",
            dir.display()
        ),
    }
}
