native-browser — measured results (this build)
================================================

Machine: Windows, WebView2 Evergreen runtime 157.0.4341.46
Build:   cargo build --release (opt-level=2, thin LTO)
Command: target\release\browser.exe --self-test=https://example.com
Data:    %LOCALAPPDATA%\native-browser\self-test.json (written by the app)

W1 — cold start → first shell paint
-----------------------------------
Measured as ms from process start (GetTickCount64 anchor taken in main())
to the shell's `shell.ready` IPC.
  run 1: 703 ms
  run 2: 672 ms
  run 3: 672 ms
TARGET: ≤ 400 ms p95. RESULT: MISS. The cost is WebView2 environment
startup for the shell webview; the shell's own HTML paint is not on the
critical path (the rail skeleton is static markup).

W10 — process footprint at 1 tab
--------------------------------
TARGET: ≤ 3 OS processes (browser.exe + 1 WebView2 browser + 1 renderer),
with the design explicitly saying the real count must be measured.
MEASURED: 14 processes, 746–759 MB RSS.

Breakdown from the measured tree (one run):
  browser.exe                 26–29 MB    (1)
  msedgewebview2.exe         129 MB      browser process
  msedgewebview2.exe         150 MB      renderer (shell or page)
  msedgewebview2.exe         182 MB      renderer (page)
  msedgewebview2.exe          88 MB
  msedgewebview2.exe          66 MB
  msedgewebview2.exe          50 MB
  msedgewebview2.exe          38 MB      GPU
  msedgewebview2.exe          22 MB      utility
  msedgewebview2.exe          22 MB      utility
  msedgewebview2.exe          22 MB      utility
  msedgewebview2.exe          23 MB      utility
  msedgewebview2.exe          46 MB
  msedgewebview2.exe         129 MB

Why more than 3: the app creates three webviews (shell, overlay, page
webview per tab) plus WebView2's own GPU/utility/audio processes. One
shared WebContext is used for page webviews so a single browser process
serves them, but Chromium's multi-process model still yields helpers.
The design's R4 predicted this; the number above is the honest result.
The overlay webview costs a renderer even at 0x0 and is the clearest
candidate to remove if W10 matters more than floating surfaces.

Functional evidence (same runs)
-------------------------------
- Shell webview created and reported `shell.ready`.
- Overlay webview created at 0x0.
- Page webview created; `tab 1 navigated: https://example.com/` logged.
- Session restore after restart: a saved example.com tab was restored and
  re-navigated (`webview for tab 1 -> https://example.com/`,
  `tab 1 navigated: https://example.com/`).
- Real-world navigation observed during interactive use:
  `tab 1 navigated: https://news.sina.com.cn/`.
- The shell rendered (screenshot build\shell-screenshot.png): toolbar with
  rail toggle, back/forward/reload, mode chip + omnibox, find-bar row
  ("Find in page"), rail row "New tab".

NOT VERIFIED
------------
- Visual correctness beyond the one screenshot above. Screen capture failed
  on later attempts ("The handle is invalid", RDP session), and this model
  cannot inspect image detail, so layout/typography/spacing claims are not
  made.
- Find-in-page counts: wry 0.57 exposes no WebView2 find API, so
  `supported_flags` is empty by design and the UI shows "n/a".
- Download pause/resume/cancel: wry exposes start/completed only.