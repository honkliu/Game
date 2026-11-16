native-browser — audit and performance pass (2026-11-12)
=========================================================

Everything below was measured on this machine with the release binary
(target\release\browser.exe), not inferred from source. Where a figure is a
single run, it says so. Profile at %LOCALAPPDATA%\native-browser; the clean-
profile figures were taken after deleting it.

Cargo state
-----------
  cargo test                     115 passed, 0 failed
  cargo clippy --all-targets -D warnings   clean
  (split: unittests src\lib.rs)

WHAT CHANGED IN THIS PASS
-------------------------
1. Shared WebView2 environment (webview2::create_shared_environment).
   wry 0.57.2's `WebContext` is an empty struct on Windows
   (web_context.rs:104 `pub(crate) struct WebContextImpl;`), so every webview
   was creating its own browser process. The app now creates one
   ICoreWebView2Environment and passes it to all three builders via
   `WebViewBuilderExtWindows::with_environment`.
2. Tab sleeping (AppState::sleep_background_tabs). Only the active tab keeps a
   live webview; deactivating a tab drops it and activating rebuilds it.
   Measured cost of a live page webview: ~120 MB.

MEASUREMENTS
------------
Baseline, 1 tab, clean profile:
  processes 9   RSS 522.7 MB   shell_ready 656 ms   (before fixes: 14 / 973 MB)
  breakdown: main:2, crashpad:1, gpu:1, utility:2, renderer:3

Live app, 1 tab:            9 procs / 576.7 MB
31 tabs open (navloop):     9 procs / 723.5 MB peak   live_webviews = 1
  (before tab sleeping, same scenario: 31 webviews / 4504 MB)

Sleep/restore, 6 tabs (driver "restore"): 1 live webview throughout, every
activation logged a rebuild and a re-navigation. Scroll/form state is lost on
sleep by design (same trade-off as Chrome tab discarding).

Leak: 120 tab open/close cycles (driver "churn"): tabs 1->1, live 1->1,
RSS 513.2 -> 489.9 MB. No growth; teardown leaves no surviving process
(`survivors: gone` in the idle soak).

Idle soak, 60 s on a heavy Wikipedia page: 14->14 procs, 984.6->994.1 MB
(+1.0%). That run predates the shared-environment change.

Tab-switch latency (driver "restore", per-command wall time across dispatch):
  already-live tab switch :   3.3 ms   (1 sample; W6 target is p95 <= 8 ms)
  slept-tab restore       : 105.4, 109.2, 114.7, 120.3, 122.8, 125.7, 137.3 ms
                            (7 samples, mean ~119 ms). This includes
                            re-creating the webview and starting navigation;
                            first content is later still, and not measured.
  So sleeping clears the memory problem and misses the ~8 ms tab-switch target
  for slept tabs by ~15x. That is the trade-off, stated plainly: without
  sleeping, the same 6 tabs hold 6 x ~120 MB live.

BUGS FOUND AND FIXED
--------------------
1. session::load ignored the `.bak` file when session.json was *missing*
   (load_one returned Ok((None, None)) for NotFound, so recovery was skipped).
   That is exactly the crash state the backup exists for.
   Fix: recover_backup() is tried for both Ok(None) and Err.
   Test: session::tests::load_recovers_a_session_stranded_in_the_backup.
2. downloads::sanitize_file_name over-blocked legal names. It treated
   `CON.txt`, `LPT1.log`, `COM10.log`, `CONIN$.log` as reserved device names and
   renamed them `_CON.txt` etc. Verified with CreateFileW +
   GetFileAttributesEx (build/resv/resv2.cpp on this machine): `CON`, `NUL`,
   `COM1` create NO file (they open the console/device), while `CON.txt`,
   `COM10.log`, `CONIN$.log` create ordinary files. Reserved-name handling now
   applies only to the extensionless form.
   Test: downloads::tests::handles_reserved_device_names.
3. clippy::manual_contains regression in downloads.rs (introduced by the audit
   agent) — fixed, clippy is clean again.

FEATURE SUPPORT — reachable vs implemented
------------------------------------------
Rust implements 52 IPC commands; the HTML shell invokes 29 of them.
These 23 are implemented and tested in Rust but have no control in this UI
build, so a user cannot reach them:

  bookmark.remove, bookmark.toggle, group.create, group.delete, group.rename,
  group.set_color, nav.open, nav.reload_ignoring_cache, nav.stop,
  page.copy_url, settings.open_folder, settings.update, shell.focus_omnibox,
  shell.input_focus, shell.omnibox_dropdown, tab.bookmark_all, tab.move,
  tab.reopen_closed, tab.zoom_reset, tab.zoom_set, window.close,
  window.minimize, window.toggle_maximize

Practical effect: no zoom control, no "reopen closed tab", no way to add a
bookmark from the UI, no tab pinning button, no minimize/maximize/close
buttons drawn by the shell, and the settings page writes via a navigation
hack rather than settings.update.

Not implemented at all (wry 0.57 API), unchanged by this pass:
  - find-in-page match counts / highlight (no find API on the WebView2 side)
  - download pause/resume/cancel (start/completed callbacks only)
  - Ctrl+F accelerators proven end-to-end (key map unit-tested, not injected)

STILL OPEN (not attempted here)
-------------------------------
  - Cold start is 640-672 ms shell-ready against a 400 ms target. WebView2
    environment creation dominates. Not attacked.
  - Find-in-page, downloads progress, zoom UI, reopen-closed UI.
  - Zero-egress (W7) never verified with a packet capture.

ARTIFACTS (build/, scratch)
---------------------------
  cleanup: build/resv/*.cpp are tiny repros; keep resv2.cpp as the evidence for
  the reserved-name behaviour, the rest can go.
  soak.json / clean-measure.json / ipc-audit.json / driver.json hold the raw
  numbers quoted above.