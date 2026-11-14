# wv2-probe — measured answers

Raw logs: `results/v1-release.txt`, `results/v2-release.txt`, `results/v3-release.txt`,
`results/v3-recheck.txt`, `results/v3-recheck2.txt`, `results/v4-release.txt`.
Machine: Windows 11, 144 DPI (scale 1.5, page DPR 1.5), screen 1664x1109 physical,
WebView2 runtime **157.0.4341.57**, wry 0.57.2 / tao 0.37.2 / webview2-com 0.39.1 /
windows 0.62.2 (pinned from `native-browser`'s `Cargo.lock`, which is copied here so
`--offline` resolves).

```bat
cd /d Q:\gitroot\Game\ai-browser\tools\wv2-probe
cargo run --offline --release --quiet -- v1                        > results\v1-release.txt
cargo run --offline --release --quiet -- v2 --samples=120          > results\v2-release.txt
cargo run --offline --release --quiet -- v3                        > results\v3-release.txt
cargo run --offline --release --quiet -- v4                        > results\v4-release.txt
```

`v3` is written to send real mouse clicks with `SendInput`; on the desktop this was run
on it could not (below), so no clicks were injected during the recorded run.

---

## Headline 1 — `NavigationId` is environment-wide, monotone, never reused, and *not* usable as the shell's identity

Answers to the four questions the old `tools/README.md` said had never been measured:

| Q | Answer | Raw evidence |
|---|---|---|
| 1. Scope | **One counter per browser process (environment-wide), not per webview.** Four webviews in one process interleave on one sequence. | ids seen per webview: A `[2,3,4,5,6,7,8,9,16,17,22,23]`, B `[19,20,21,30,32,34,36]`, C `[25,26,31,33,35,37]`, D `[28,29]` — B's first navigation took 19 while A's next took 22. Merged stream: 27 ids, 0 steps backwards. |
| 2. Monotonicity / reuse | **Strictly increasing within each webview; no reuse across reload, back or forward.** | reload-kind ids `[6,17,22]`, back/forward-kind ids `[7,8,9]`, **0 repeats over 27** `NavigationStarting`. |
| 3. `pushState` / `replaceState` / fragment | **Emits no `NavigationId`-bearing event at all.** `SourceChanged` + `HistoryChanged` fire; neither interface carries a `NavigationId`. | four probes: `NavigationStarting+0 ContentLoading+0 DOMContentLoaded+0 SourceChanged+1 HistoryChanged+1 new_ids=[]`. |
| 4. Subframes | **Draw from the same counter**, and report on `FrameNavigationStarting`. | iframe ids `[14,15]`, both strictly between the top-level ids 9 and 16. |

The two findings that decide the design (they are the reason `NavigationId` cannot
become the shell's counter):

* **One failed navigation to a dead host consumed 3 ids** — `[16,17,22]`, the runtime
  retried it itself — and the next successful navigation used 23. A sequence derived
  from `NavigationId` counts navigations the user never performed.
* **The counter is not dense**: it advanced by 36 (2 → 37) while only 29 ids ever
  reached a handler, so **7 ids were spent with no event delivered**. Ids also exist
  before `add_NavigationStarting` can be attached (wry builds the controller first).

**Decision this forces:** keep `navigation_sequence` as the shell's own counter and keep
`NavigationId` for audit correlation only. Promoting it would give a per-*process*
sequence that includes iframe loads and invisible retries and has holes. The four
hedges in the contract were all load-bearing.

---

## Headline 2 — an overlay controller costs ~70 MiB, not a second engine

`RSS` is the whole process tree (`K32GetProcessMemoryInfo` summed per pid), median of 5
samples each side, page already live in the browser window:

| | procs | working set | private |
|---|---|---|---|
| no overlay | 7 | 350.3 MiB | 107.1 MiB |
| + one transparent overlay controller | 8 | 418.4 MiB | 143.3 MiB |
| **delta** | **+1** | **+68.1 MiB** | **+36.2 MiB** |

Five runs of that measurement gave working-set deltas of **+68.1, +68.2, +68.1, +79.4,
+79.9, +82.5 MiB** — the spread is run-to-run, the order of magnitude is not.
`BrowserProcessId` is the **same pid** for the page and the overlay view (printed on both
RSS rows), so the cost is one controller + one renderer, not a second engine. Report it
as a delta: the absolute number depends on what the page pulled in.

## Headline 3 — the rect→painted latency cannot be measured on this box, and why

`PrintWindow(PW_RENDERFULLCONTENT)` is the only pixel instrument that works here:
whole-screen GDI `BitBlt` fails (2/2 reads, GDI error 6, `SM_REMOTESESSION=1` —
non-interactive desktop). One `pixel()` read through it costs **86–113 ms**, which is
~100× the 25 ms lower edge of the band under test, and PrintWindow renders *one window's
own* content — it cannot see an overlay composited on top of the window under the point.
The probe therefore prints `PROBE verdict: DECLINED` and reports **no** on-screen
rect→painted number rather than a bound read from an instrument 100× coarser than the
quantity. The 25–50 ms p50 / 60–120 ms p95 estimates stay **untested on this machine**.

What could be measured, and does bear on the claim:

* `css-transform rect→style observed` (the overlay's renderer sees the new computed
  style in a rAF callback): **p50 4.0 ms, p95 4.9 ms, n=120**. Lower bound only — commit,
  present and DWM flip come after it.
* `rect→frame committed` (one renderer frame later, by construction): p50 35.4 ms.
* The **gap between those two beats, 31.3 ms, is the overlay document's own frame
  period** (~32 fps here). A rect→screen latency is that frame period plus the present;
  the 3.9 ms host pipeline is not the term that puts the answer in the 25–50 ms band.
* Structural result worth more than any number: **the shipping strategy cannot be
  instrumented from inside the overlay at all.** `set_bounds` with unchanged width/height
  changes nothing the overlay document can observe — no style, no size, no resize event.
  Only the screen can tell when a `set_bounds` card got there, and this machine's screen
  read costs 100 ms. `SetWindowPos` on a top-level costs 408 µs, `set_bounds` 977 µs, a
  CSS `postMessage` 40 µs; page↔host round trip 0.33 ms.

## Headline 4 — follow error is exactly one reported frame of scroll, both strategies

Fling: the page scrolls a fixed step per its own rAF and reports the anchor rect every
frame; the host follows immediately. Error = how far the anchor had already moved from
the rect the card was last given. No screen readback involved.

| speed | velocity the page got | follow error, both strategies |
|---|---|---|
| 4 css px/frame | 129 css px/s | **4.0 px** (= 6.0 physical) |
| 12 | 387 css px/s | **12.0** (18.0) |
| 40 | 1315 css px/s | **40.0** (60.0) |
| 120 | 4178 css px/s | **120.0** (180.0) |

The error equals the per-frame scroll step to the last decimal at every speed — the
follower is exactly one reported frame behind and never drops a second frame — and
`set_bounds` and CSS transform are indistinguishable. Alice's **100–500 px** claim is
reached only at the 4th speed, ~4200 css px/s (the page itself only manages ~30 fps here,
21–31 ms per reported frame), i.e. the claim is really a statement about scroll velocity ×
frame time, not about overlay technology. `set_bounds`' issuing call does have a host-side
tail under load: p50 914 µs but p99 1408 µs / max 2190 µs during the 121-rect fling.

---

## Headline 5 — a full-bleed transparent overlay eats clicks; only two shapes work

`SendInput` and `SetCursorPos` both fail on this desktop (**GDI error 5**, accepted
0/3 events), so the page-side `mousedown` counters are **NOT MEASURED**. What *is*
measured is `WindowFromPoint` — the same hit-test the window manager runs before it
delivers a click — resolved through the ancestor chain (it answers with each webview's
`Chrome_RenderWidgetHostHWND`, three levels below the HWND wry hands back).

| overlay shape | transparent point belongs to | card point belongs to | both properties? |
|---|---|---|---|
| sized to the card (**what ships**) | the page | the overlay | **yes** |
| full-client, transparent controller | **the overlay** | the overlay | no — transparent region eats the click |
| full-client + `WS_EX_TRANSPARENT` (verified applied: mask reads `0x20`) | **the overlay** | the overlay | no |
| full-client + `WS_EX_TRANSPARENT|LAYERED|NOACTIVATE` | **the overlay** | the overlay | no — and the mask was **refused** (`0x0` read back): `WS_EX_LAYERED` is top-level-only and this overlay is a child window |
| full-client + `WS_EX_NOACTIVATE` | **the overlay** | the overlay | no |
| **full-client + window region = card rect** (`SetWindowRgn`) | the page | the overlay | **yes** |

Answer to the question as posed: a click in a fully transparent region of a transparent
WebView2 **does not reach the page** — the window's rectangle is the hit region, alpha is
irrelevant — and the opaque card keeps its clicks in every shape. The only workable
full-bleed shape is a **window region** clipped to the card (plus whatever opaque chrome
is genuinely needed), which is what native-browser could adopt if the card ever needs to
float inside a larger surface. Today's card-sized overlay is already correct by geometry,
so §2.1's question is not "does transparency pass clicks" but "what does the card's
shadow/band get drawn on".

### Reproduced — after fixing an instrument that was grading somebody else's windows

A re-run of `v3` printed `owner → no probe window` and card pixel `(25,26,27)` for
**every** variant and graded them `FAILS`. Cause: fourteen other
`msedgewebview2.exe` hosts are live on this desktop, and a foreign `Chrome_WidgetWin_1`
(ancestor chain ending at `#32769`, the desktop window) was stacked above the probe, so
`WindowFromPoint` was answering for a window this process does not own. `v3` now raises
its own window before each sample point (`SetWindowPos(HWND_TOP,
SWP_NOACTIVATE|NOMOVE|NOSIZE)`) and checks that `GetAncestor(GA_ROOT)` of the hit window
is the probe's own root; when it is not, the row prints `GEOMETRY OCCLUDED … NOT
MEASURED` and, if every row was occluded, the verdict is `V3 NOT MEASURED` instead of a
false `FAILS`. `results/v3-recheck.txt` is an occluded run showing that path;
`results/v3-recheck2.txt` is a clean run whose table is **identical to the one above**.
Anyone re-running a hit-test measurement on a shared or RDP desktop has to raise first —
otherwise the numbers describe another process's windows.

---

## Headline 6 — the DevTools-Protocol bridge answers today: tier-2 extraction is real work, and the AX tree carries no geometry

`CallDevToolsProtocolMethod` is on the base `ICoreWebView2` in webview2-com-sys 0.39.1
(`ICoreWebView2_Vtbl` at bindings.rs:1771, the method at 1897,
`GetDevToolsProtocolEventReceiver` at 1915). Symbol presence is not capability, so it was
called. Conditions of the run: a normal **https** origin, **DevTools never opened**
(`ICoreWebView2Settings::AreDevToolsEnabled` reads back **false**), no remote-debugging
port, and the environment built with only wry's own browser arguments
(`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`) — i.e. **no additional
browser argument was needed**.

| call | sync return | completion | ms | bytes | nodes |
|---|---|---|---|---|---|
| `DOMSnapshot.captureSnapshot {"computedStyles":[]}` | S_OK | **S_OK** | 2.2 | 5 809 | **35** DOM nodes |
| `Accessibility.getFullAXTree {}` | S_OK | **S_OK** | 7.3 | 15 481 | **38** AX nodes (3 ignored, 29 named) |
| `NotARealDomain.NotAMethod {}` (control) | S_OK | **HRESULT 0x80070057** | 0.6 | 85 | — |

**Text and geometry come back for the same content in one snapshot call.** Of the 23
layout objects carrying a rect, 8 carry text; the paragraph's own run came back as

```text
laid-out text #0: layout object 9, node 20 <#text>, x=354.390625 y=252 w=618.203125
                  h=108.78125, text "This domain is for use in documentation examples…"
text box #0: layout object 9, x=354.390625 y=252 w=618.203125 h=32 — "This domain is for
             use in documentation examples without"
```

plus **21 `textBoxes`** — per-line rects, each locating its run inside the node's string
by `(layoutIndex, start, length)`. That is ordinary paragraph content with a rect, so a
tier that needs positions has a real rect source and does not need `DOM.getBoxModel`
per node.

**The AX tree carries names and zero geometry.** 38 nodes, and the line
`AX nodes carrying a bounds property: 0 of 38` — the payload text contains no `bounds` or
`boundingBox` key at
all. Paragraph words are present (`StaticText` × 7 with full sentences; the six
`paragraph` nodes themselves have empty names, and 20 `InlineTextBox` nodes hold the
line fragments). The only way to give an AX node a position is to join it to the DOM
snapshot on `backendDOMNodeId` ↔ `backendNodeId` — and that join covers **18 of 38**
AX nodes: exactly the 20 `InlineTextBox` fragments have no key, so per-line AX content
has no addressable position through this pair of calls. Line geometry therefore comes
from `textBoxes`, not from the AX tree.

**The control is the second-most-useful number.** The invented method returned **S_OK
synchronously** and failed only in the completion callback with `0x80070057` and
`{"error":{"code":-32601,"message":"'NotARealDomain.NotAMethod' wasn't found"},"id":2}`.
A sync `S_OK` therefore means "the bridge accepted the call", **not** "this method
exists", so shipped code must not treat it as an acknowledgement and must branch on the
completion result.

Two payload-shape warnings for whoever implements the tier, both learned by being wrong
first:

* This runtime returns the **legacy columnar** `DOMSnapshot` shape — `documents[0].nodes`
  is an object of parallel arrays (`nodeName`, `nodeType`, `parentIndex`,
  `backendNodeId`, …), geometry is `documents[0].layout.{bounds,nodeIndex,text}` with
  `bounds` as `[x,y,w,h]` arrays, and every string is an index into the top-level
  `strings` table. It is **not** the modern `nodeTreeSnapshot` / `layoutTreeSnapshot`
  shape the current CDP docs describe; an analyzer written against those docs reports
  **0 nodes** against a payload containing 35. Read the shape from the answer.
* The page's copy came back in four languages (English, Arabic, Chinese, Spanish), so
  text offsets must be counted in UTF-16 code units (`textBoxes.start`/`length` are) and
  nothing may hard-code a sentence — the probe's first version keyed on an example.com
  sentence that no longer exists.

**What this decides.** Both extraction methods work, so §4.2's tier-2 has a real,
non-forgeable read path and the "no out-of-band content read" loss is reversed *today*.
That is a runtime-conditional fact, not a platform one: the bridge is Chromium's and can
vanish or change shape on a runtime update regardless. The docs must therefore assume
disappearance, and the tier must degrade to the main-world probe with a typed
`unsupported`/`failed` result on a sync failure, a completion failure, a timeout, or an
unparseable payload — and it must never sit on the navigation path, where a method that
vanished last week could fail a page load.

---

## Not done, and why

1. **On-screen rect→painted latency (V2-2, and the fling's paint-attributed error)** —
   no instrument on this box resolves below ~86 ms; the band under test is 25–50 ms.
   Needs a run on an interactive desktop with a working screen DC (then
   `--samples=120` gives the percentiles directly; the sections self-enable when the
   calibration reads ≤ 25 ms a pixel).
2. **Delivered-click confirmation of the hit-test (V3's mousedown columns)** —
   `SendInput`/`SetCursorPos` are refused on this non-interactive desktop.
3. **`set_bounds` renderer-side ack** — impossible in principle, not just unmeasured: a
   constant-size bounds move is invisible to the document. Stated in the run output.
4. **Overlay frame period on a 60 Hz interactive display** — the 31.3 ms measured here is
   a remote-desktop artefact; on a real display the frame term is expected near 16.7 ms,
   which would halve both the latency and the follow error. Not measured here.
5. **V2's paint-ack split (`seen` vs `committed`) for `set_bounds`** — see 3.
6. **V4 beyond one small page** — the snapshot was taken on a one-paragraph https page
   with one document (`documents: 1`). Bytes and milliseconds per call on a real page
   (megabytes of DOM, many frames, `computedStyles` requested, `includeShadowTree` for
   closed shadow roots) are unmeasured, as is whether a subframe appears as a second
   entry in `documents` or needs its own session.
7. **V4's CDP event path** — `GetDevToolsProtocolEventReceiver` was verified present in
   the vtable but never called; nothing here measures `DOM.documentUpdated`-style
   invalidation, which is what a real extraction cache would need instead of re-snapshotting.
8. **V4's degradation path** — the probe reports what happens when a method is *unknown*;
   what happens when a method that exists is *removed* by a runtime update is necessarily
   unmeasured (this runtime still has both). The required behaviour is stated in the
   headline above, not verified.
