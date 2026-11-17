# `tools/`

Tooling for the contract crate in `../crates/contract/`, plus the measurement spike that was
required before `NavigationId` could be trusted.

## Native PDF extraction probe

```powershell
$env:BROWSER_EXE='Q:\gitroot\Game\ai-browser\target\debug\browser.exe'
node ai-browser\tools\probe-native-pdf.mjs
```

Run from the repository root with the existing `e2e` dependencies installed. This
Windows-only probe launches an isolated browser profile and serves synthetic PDFs
locally. Embedded documents exercise WebView2's native PDF viewer without changing
the application's PDF routing. It neither uses the clipboard nor moves the desktop
pointer.

On November 17, 2026, WebView2 157.0.4341.60 exposed all three pages of a PDF,
including offscreen pages, through Windows UI Automation `TextPattern.DocumentRange`.
Separate canvas cells returned separate documents, without PDF toolbar text.
`TextPattern.GetSelection` returned the selected text and its screen rectangles.
Accessibility initialized lazily; no special runtime flags were needed. Selection
updates were asynchronous and had to be awaited.

This is the native viewer's accessibility provider, not a WebView2 SDK
`ExtractPdfText` method or ordinary page DOM access. The Windows browser now uses
the native viewer and connects its accessibility provider to River, selection,
Find and citation reveal. `e2e/suite5.mjs` covers the integrated behavior.
macOS retains PDF.js pending a separate native PDFKit adapter.

## `wv2-probe/` — built, run, answered

A standalone crate (its own `Cargo.toml`/`Cargo.lock`, nothing in `native-browser/`
touched) that answers the three WebView2 behaviours the design rested on, plus the
DevTools-Protocol question the read-only-extraction ruling raised. Full numbers, the
exact repro commands and the list of what could not be measured on that machine:
**[`wv2-probe/RESULTS.md`](wv2-probe/RESULTS.md)**.

```bat
cd /d Q:\gitroot\Game\ai-browser\tools\wv2-probe
cargo run --offline --release --quiet -- v1                > results\v1-release.txt
cargo run --offline --release --quiet -- v2 --samples=120  > results\v2-release.txt
cargo run --offline --release --quiet -- v3                > results\v3-release.txt
cargo run --offline --release --quiet -- v4                > results\v4-release.txt
```

Raw COM is reached without patching wry: `WebViewExtWindows::controller()`
(`wry-0.57.2/src/lib.rs:2345`) hands back the `ICoreWebView2Controller`, and the
`webview2-com` 0.39.1 event-arg interfaces expose `NavigationId(*mut u64)` on
`NavigationStarting`, `ContentLoading`, `NavigationCompleted` and
`DOMContentLoadedEventArgs` only — `SourceChanged` and `HistoryChanged` have no such
property.

### The `NavigationId` experiment: result and decision

All four questions the previous version of this file listed as open came back, on
WebView2 runtime 157.0.4341.57:

1. **Scope: one counter per browser process, not per webview.** Four webviews in one
   environment interleave on a single sequence (webview B's first navigation took id 19
   while webview A's next took 22). Monotone overall and within each webview.
2. **No reuse.** Reload ids `[6,17,22]`, back/forward ids `[7,8,9]`, 0 repeats over 27
   `NavigationStarting` events.
3. **Same-document navigation emits no `NavigationId`-bearing event at all** —
   `pushState`, `replaceState` and `location.hash` each produced 0
   `NavigationStarting`/`ContentLoading`/`DOMContentLoaded` and only
   `SourceChanged`+`HistoryChanged`, which carry no id.
4. **Subframes draw from the same counter** and report on `FrameNavigationStarting`
   (ids `[14,15]`, between top-level ids 9 and 16).

Two further measurements decide it. **A single failed navigation consumed 3 ids** (the
runtime retried a dead host on its own) and the next success used the id after them; and
**the counter is not dense** — it advanced 36 (2 → 37) while only 29 ids ever reached a
handler, so 7 were spent with no event delivered, plus any spent before wry lets handlers
be attached (it navigates during `build`).

**Therefore `navigation_sequence` stays the authority and `upstream_navigation_id` stays
audit-only.** The hedge was correct, and is now documented rather than an open question:
promoting `NavigationId` would yield a per-*process* sequence that counts iframe loads
and invisible retries and has holes in it.

### The two design corrections the measurements forced

* **`native-browser`'s overlay is already transparent and already card-sized**
  (`crates/browser/src/app.rs:2388` builds it with `build_as_child` +
  `.with_transparent(true)`), so click-through works today by geometry, not by alpha. A
  full-bleed transparent overlay **does not** pass clicks through: `WindowFromPoint` keeps
  handing the transparent point to the overlay's own window, and `WS_EX_TRANSPARENT` does
  not change that (`WS_EX_LAYERED` cannot even be set on a child window — the style write
  is refused outright). The one workable full-bleed shape is `SetWindowRgn` clipped to the
  card rect.
* **An overlay controller costs ~70 MiB of process-tree working set (+36 MiB private) in
  the same browser process** — measured as a delta over a live page, six runs, +68 to
  +83 MiB. And a `set_bounds` reposition is invisible to the overlay document (constant
  size, nothing observable changes), so an anchored card cannot be instrumented from
  inside the renderer at all — only `SetWindowPos` on a top-level or a CSS-transform
  strategy can be.

`v3` also carries an occlusion guard, because the first re-run of it graded **foreign**
windows: fourteen other `msedgewebview2.exe` hosts live on this desktop meant
`WindowFromPoint` answered with a window the probe does not own. It now raises its own
window and verifies `GetAncestor(GA_ROOT)` of the hit before grading a row, printing
`NOT MEASURED` instead of a false `FAILS` when something else is on top.

### The CDP probe (V4): the bridge answers, so tier-2 extraction is real work

`CallDevToolsProtocolMethod` is on the **base** `ICoreWebView2` (webview2-com-sys 0.39.1,
`ICoreWebView2_Vtbl` at bindings.rs:1771; the method at 1897,
`GetDevToolsProtocolEventReceiver` at 1915), so no cast to an extension interface is
needed. Symbol presence is not capability, so `v4` calls it on a live https page with
DevTools never opened. On runtime 157.0.4341.57:

| call | completion | ms | bytes | what came back |
|---|---|---|---|---|
| `DOMSnapshot.captureSnapshot` | **S_OK** | 2.2 | 5 809 | 35 DOM nodes, 23 layout rects, **8 of them carrying text**, 21 per-line `textBoxes` |
| `Accessibility.getFullAXTree` | **S_OK** | 7.3 | 15 481 | 38 AX nodes, 29 named, paragraph sentences present, **0 with bounds** |
| `NotARealDomain.NotAMethod` | HRESULT **0x80070057** | 0.6 | 85 | `-32601 … wasn't found` — **asynchronously**; the sync return was `S_OK` |

* No extra browser argument, no debugging port, no setting: `AreDevToolsEnabled` reads
  back `false` and both calls still worked, on a normal https origin, with only wry's own
  browser args.
* Text **and** geometry come back for the same paragraph content in one snapshot call
  (`layout.bounds[i]` + `layout.text[i]` → the shared `strings` table), so a
  rect-carrying extraction tier does not need a `DOM.getBoxModel` call per node.
* The AX tree is names-only. Position has to be joined back on
  `backendDOMNodeId` ↔ `backendNodeId`, and that covers 18 of 38 nodes — the 20
  `InlineTextBox` line fragments have no key, so line-level geometry comes from
  `textBoxes`, not from the AX tree.
* Two traps for the implementation: this runtime returns the **legacy columnar**
  `DOMSnapshot` shape (parallel arrays + a shared `strings` table), not the
  `nodeTreeSnapshot`/`layoutTreeSnapshot` shape current CDP docs describe, and
  `textBoxes.start`/`length` count **UTF-16 code units** (the test page returned the same
  paragraph in four languages).

**What it decides and what it does not.** The ruling that permits CDP for read-only
extraction is not moot: tier-2 gets a read path a page script cannot forge, which
reverses one of the listed permanent losses. The docs must still treat the bridge as
**removable by a runtime update** regardless of this measurement — every call is optional,
must degrade to the main-world probe with a typed `unsupported`/`failed` result on a sync
failure, a completion failure, a timeout or an unparseable payload, and must never sit on
the navigation path where a vanished method could break a page load. Note also that a
sync `S_OK` is not an acknowledgement that the method exists (the control above proves
it), so the completion result is the only thing worth branching on.

## Not built (do not treat as available)

- `emit_schema.py` / schema publication for the WebView2 UI — see "Known gaps" in
  `../crates/contract/README.md`. Deriving `schema/v1/*.json` from the Rust types is the
  intended route, so the schema cannot disagree with the compiled contract.
- `check_conformance.py` over wire fixtures — blocked on the above.
- Nothing in Epic A needs `gn`, `ninja`, `gclient`, or `depot_tools`; the earlier
  `sync_upstream.py` plan for FND-05 was dropped with the fork decision and was never
  written.

## Test command

The whole Epic A suite is one command and it needs no network:

```bat
cd /d Q:\gitroot\Game\ai-browser\foundation
set CARGO_INCREMENTAL=0
cargo test --offline
```

wv2-probe needs outbound network for its test sites and a desktop session; `v3` is
written to send real mouse clicks.
