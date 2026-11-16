# Capability map — WebView2 kernel, we are the embedder host

bindings.rs line numbers refer to webview2-com-sys-0.39.1, grep-derived 2026-11-13. **Convention: the
vtable-slot line** — `pub <Name>:` inside `pub struct I<Interface>_Vtbl` — not the generated safe wrapper
(`pub unsafe fn <Name>`) on its earlier line. Slots whose declaration wraps its type onto the next line
(`pub FrameId:`) were resolved by reading the enclosing `_Vtbl`; numbers not reproducible by grep were deleted.
Kernel fixed (user decision, 2026-11-13): **the existing WebView2 runtime**, hosted by our own Rust process —
no Chromium source, no fork, no checkout, no CEF, no Electron. This is a capability map, not a justification.
Companions: `RELEASE-0.md` (work order), `tools/baseline.md` + `tools/wv2-probe/RESULTS.md` (**the measured
numbers this file now quotes**), `BACKLOG-DELTAS.md` (proposed spec edits).

## 0. Correction notice — my earlier verdict tested the wrong alternative

The previous version concluded that backlog §2.1 (a privileged, selection-anchored inline UI the page cannot
restyle, remove, or impersonate) "requires a fork", reasoning that content scripts share the page DOM and
`openOrClosedShadowRoot` leaves the host element removable and restyleable. **That reasoning is correct and
fatal — to in-document surfaces only.** Extensions and CDP are not the alternatives the user chose; the
chosen alternative is the **embedder host process**, a third surface I had not evaluated.

Revised verdict: **§2.1 is achievable on the fixed kernel — and not merely achievable, it already
ships** (code path in §1). So §2.1 is not an unavailable capability we must engineer but an existing
mechanism, and what was the program's strongest architectural risk is now a narrow measurement
question — largely answered since (§7 V2/V3, `tools/wv2-probe`). §2.5 remains a genuine, confirmed
loss. A fork is **not** required for Release 0.

## 1. Why the host HWND has the property §2.1 demands

| Surface | In the page's document tree? | Page CSS authority | Page JS authority |
|---|---|---|---|
| Content-script node / closed shadow root | yes | cascade, inheritance, `z-index`, ancestor `transform`/`filter`/`clip-path` | query, mutate, `remove()`, occlude, forge an identical node |
| **Host-owned controller HWND** (`CreateCoreWebView2ControllerWithOptions`) | **no** | **none** — the page's style sheet cannot select a node that does not exist in its document | **none** — no DOM handle exists; `window.open`/`showModalDialog` do not reach our window |

The boundary is not a privilege level inside Blink; it is a **window boundary**. Page CSS and page JS are
authorities over a document; our overlay lives in another window, created by our process, composited by the OS.
A page cannot out-z-index a window, cannot place a `pointer-events` capture layer over one, and cannot delete a
window it cannot name — exactly §2.1's invariant, obtained structurally rather than by policy.

**This is not hypothetical in our codebase — it already ships.** `crates/shell/src/app.rs`
keeps one `overlay: Option<WebView>` (app.rs:97) built from the shared environment (app.rs:103), positioned by
`layout::Overlay` = `Closed | Palette | Dropdown{x,y,w}` (layout.rs:29-38, `Closed` → 0×0), moved with
`set_bounds` (app.rs:627/649), focus-restored on close (app.rs:659), served from the path-traversal-hardened
`asset://` handler (`ui.rs`, registered at app.rs:2358), and talking to the host over
`window.ipc.postMessage` → `window.chrome.webview.postMessage` (wry `webview2/mod.rs:968`) →
`add_WebMessageReceived`. **What was unproven was narrower than §2.1's whole claim** — the surface is an opaque
palette covering content today, so the open items were per-pixel transparency with correct hit-testing over
live content (V3) and re-anchor latency (V2), never whether the page can touch our window. Both are measured
now: the card-sized shape passes clicks **by geometry**, a full-bleed one never does (`wv2-probe` Headline 5;
§3.1, §3.2, §7).

Verified present in the SDK we link — `webview2-com-sys-0.39.1/src/bindings.rs` (presence verified by
grep; behaviour confirmed on a running webview for the items `tools/wv2-probe` covered, still
**verify** for the rest — see §7):

Interface owner is stated because three rows changed owner when the numbers were re-derived:

| API | Line | Use |
|---|---:|---|
| `CreateCoreWebView2EnvironmentWithOptions` (free function, no vtable) | 29 | one environment per process |
| `ICoreWebView2Environment10::CreateCoreWebView2ControllerWithOptions` | 14762 | second controller = second host window |
| `ICoreWebView2ControllerOptions::{SetProfileName, SetIsInPrivateModeEnabled}` | 10458 / 10466 | overlay profile isolation |
| `ICoreWebView2Controller::{SetBounds, SetBoundsAndZoomFactor}` | 9321 / 9336 | position the window on a DOM rect, honouring page zoom |
| `ICoreWebView2Controller::{MoveFocus, add_MoveFocusRequested}` | 9341 / 9345 | host-mediated traversal between the two input surfaces |
| `ICoreWebView2Controller2::{DefaultBackgroundColor, SetDefaultBackgroundColor}` | 9906 / 9910 | transparent overlay backing (`put_DefaultBackgroundColor` in the safe wrapper) |
| `ICoreWebView2::AddScriptToExecuteOnDocumentCreated` | 1859 | probe injection (main world — see §3.4) |
| `ICoreWebView2::ExecuteScript` / `ICoreWebView2_21::ExecuteScriptWithResult` | 1870 / 41822 | snapshot + rect probes |
| `ICoreWebView2::PostWebMessageAsJson` | 1882 | probe → host channel |
| `ICoreWebView2::{CallDevToolsProtocolMethod, GetDevToolsProtocolEventReceiver}` | **1897 / 1915** | the CDP bridge, **on the base interface** — ruling 2026-11-13: read-only extraction only, off by default; see §1b |
| `ICoreWebView2DevToolsProtocolEventReceiver::add_DevToolsProtocolEventReceived` | 12937 | streaming CDP events back to the host |
| `ICoreWebView2::BrowserProcessId` | 1903 | browser-process id on the base interface — no `ICoreWebView2_10` QI needed |
| `ICoreWebView2_4::add_FrameCreated` / `ICoreWebView2Frame7::add_FrameCreated` | 43310 / 20475 | per-frame handles: webview-level and nested-frame-level (**owners were reversed in the previous draft**) |
| `ICoreWebView2Frame2::{add_NavigationStarting, add_ContentLoading, ExecuteScript, PostWebMessageAsJson}` | 19668 / 19675 / 19696 / 19701 | per-frame events and per-frame script |
| `ICoreWebView2Frame5::FrameId` | 20265 | per-frame key for `DocumentKey` |
| `ICoreWebView2Frame::{add_Destroyed, IsDestroyed}` | 19286 / 19293 | frame teardown → `FrameDetached` |
| `ICoreWebView2Frame6::add_ScreenCaptureStarting` | 20352 | (not used in Release 0) |
| `ICoreWebView2_23::PostWebMessageAsJsonWithAdditionalObjects` / `ICoreWebView2_17::PostSharedBufferToScript` | 42136 / 41009 | large citation payloads into **our** overlay only |
| `ICoreWebView2::AddWebResourceRequestedFilter` / `ICoreWebView2_22::AddWebResourceRequestedFilterWithRequestSourceKinds` | 1969 / 41968 | egress + header control (semantics verify) |
| `ICoreWebView2_3::SetVirtualHostNameToFolderMapping` | 43109 | **fallback only** — the shipped UI already comes from an `asset://` custom protocol (`ui.rs`, app.rs:2359/2398) |
| `ICoreWebView2::add_NewWindowRequested` | 1921 | keep overlay z-order/bounds correct when the page opens windows (the previous `~24596` was a guess and is deleted) |

Also verified by absence — the first item below no longer constrains the plan (scope decision), the
rest do:
- **No embedder accessibility-tree API on the COM surface: `accessibilit`, `IAccessible`,
  `uiautomation`, `UiaRaise` each return 0 matches** (case-insensitive, 44,394 lines of `bindings.rs`).
  **The absence stopped being a constraint when the user ruled accessibility features out of scope on
  2026-11-13** ("the browser is not for everyone, but focus on 60 to 70% people") — and the tier we
  deleted was worth far less than the plan assumed: the only time the AX tree was ever read here
  (`Accessibility.getFullAXTree`, `wv2-probe` Headline 6) it returned **38 nodes, 0 carrying bounds**,
  and just 18 of the 38 could be joined onto DOM geometry by `backendDOMNodeId` (all 20 `InlineTextBox`
  line fragments lack the key). Names with no addressable position. Backlog §4.2's tier-2 and
  `AnnotationAnchor`'s `accessibilityPath` are **deleted from the spec** (`BACKLOG-DELTAS.md` **D-01**) —
  not deferred, not reserved, not reached through CDP — and shipped code already matches:
  `TargetResolution::DomLocator` (`contract.rs:211`) keeps the DOM half, the AX half is **absent rather
  than reserved**, guarded by `tests/action_levels.rs:43`. **Carve-out (user-confirmed the same day):
  the decision does not reach input quality.** Keyboard navigation, focus handling and IME for **our
  own** chrome (C-42 — `ui/app.js`, `MoveFocus` 9341 / `add_MoveFocusRequested` 9345, the `EnableWindow`
  approval modality) stay in scope in every release; §3.3 stands unchanged.
- **`Tabbable` appears 0 times.** No `IsTabbable` in this binding; see cost 3.
- `wry` 0.57.2 does hand us raw COM: `WebViewExtWindows::{controller(), environment(), webview()}`
  (`wry-0.57.2/src/lib.rs` 2345/2348/2351, the trait at 2343), `WebViewBuilderExtWindows::with_environment`
  (1862), `with_additional_browser_args` (1794), and on the Windows impl `hwnd()` (`webview2/mod.rs` 1528),
  `bounds()`/`set_bounds()`/`set_visible()`/`reparent()` (1591/1640/1654/1866). wry itself casts
  (`webview.cast::<ICoreWebView2_10>()`, `webview2/mod.rs` 1976), so the QI pattern works on this exact
  binding version. **No wry patch is needed** for §2.1/§2.3/§8.

## 1b. The CDP bridge — the one route that reaches past the COM surface

Verified on the **base** `ICoreWebView2` vtable (`ICoreWebView2_Vtbl` begins at bindings.rs:1771),
so no QI, no `--remote-debugging-port`, and no devtools-enabled setting is a documented prerequisite:

| Slot | Line | What it gives |
|---|---:|---|
| `ICoreWebView2::CallDevToolsProtocolMethod` | 1897 | invoke any `{domain}.{method}` against that webview |
| `ICoreWebView2::GetDevToolsProtocolEventReceiver` | 1915 | subscribe to a CDP event domain for this webview |
| `ICoreWebView2DevToolsProtocolEventReceiver::add_DevToolsProtocolEventReceived` | 12937 | stream the events into the host |

This is why the 2026-11-13 inheritance audit introduced a separate bucket, **b\*** — "reachable only
through the CDP bridge" — instead of folding those claims into **(d)**.

### 1b.1 The ruling (user, 2026-11-13) — read-only extraction, nothing else

| Rule | As ruled |
|---|---|
| **Allowed** | exactly one method: **`DOMSnapshot.captureSnapshot`**. The ruling was issued with `Accessibility.getFullAXTree` alongside it, and that half is **withdrawn by the 2026-11-13 scope decision** (accessibility out of scope) — we do not call the `Accessibility` domain at all |
| **Forbidden** | `Input.dispatchMouseEvent` and **every** other method that mutates page state. **No CDP in the Interact or Commit path at any release** without a new ruling |
| **Exposure** | never a public or product-facing API — CDP is an internal implementation detail of extraction. No story, no acceptance criterion, and nothing in the extension or tool ecosystem may depend on it |
| **Stability** | pin the exact domain versions we call; treat the surface as **unstable across Evergreen runtime updates**. Feature-detect at startup and degrade to the main-world probe with the existing typed `unsupported` failure. **A CDP domain disappearing must never break a page load** |
| **Default** | **off**, behind a runtime-read flag honouring the same fail-to-disabled rule as C-80 (§8). Because the flag is read at runtime, it cannot live in `AdditionalBrowserArguments` (fixed at environment creation, `webview2.rs`) |
| **Still buys (1)** — the case does not borrow from AX | a **non-forgeable read**: structure, geometry and **renderer-computed** layout/visibility serialised by Blink, on a channel the page cannot speak into, flood, shadow or schema-forge. Kills the forged-visibility and forged-rect failure modes in the highest-volume case, the whole-page snapshot, and removes the dependency on a probe the page can delete or hook (V8) |
| **Still buys (2)** | trustworthy rects for citation placement — §2.1's card-on-a-passage is geometry-critical and §3.2 shows it is lag-prone, so renderer rects are the best input that path can get — plus a **smaller** unstable surface than the ruling asked for: one method, one feature-detect, one degrade path, fewer fields to pin per runtime version (V17) |
| **Costs** | nothing material. AX's marginal value over DOM extraction was roles and names on custom-widget pages; canvas text, which AX would not have given us either, is §4.2's cut OCR/vision tier regardless. The 60-70 %-of-users target is DOM-plus-layout shaped, which is what we keep |

Two properties cut *for* the ruling: calling through `CallDevToolsProtocolMethod` opens **no listening
port** (unlike `--remote-debugging-port`, which any local process could attach to), and the one allowed
method is not an event — so `GetDevToolsProtocolEventReceiver` (1915) /
`add_DevToolsProtocolEventReceived` (**12937**) stay unused on the permitted path.

Documented constraints that survive the ruling, from the Microsoft `CallDevToolsProtocolMethod` page:
the method name is `{domain}.{method}` and an unknown name fails with `E_INVALIDARG`; **"CDP method
calls may be processed out of order"** → any use needs a serialising queue in our process plus
per-call correlation, and no ordering assumption may be encoded in the action runtime. The required
parameter shape and OOPIF coverage remain **verify** (§7 V17).

**V16 is measured: the bridge answers.** `tools/wv2-probe v4` (Headline 6, `results/v4-release.txt`, runtime
**157.0.4341.57**) called `DOMSnapshot.captureSnapshot {"computedStyles":[]}` on a live https page with
**DevTools never opened** (`AreDevToolsEnabled` reads back **false**), no debugging port and **no extra
browser argument**: completion **S_OK, 2.2 ms, 5,809 B, 35 DOM nodes, 23 layout rects, 8 of them carrying
text, 21 per-line `textBoxes`** — structure, text and geometry in one call, so rect-carrying extraction is
real work rather than a dead end and does not need `DOM.getBoxModel` per node. Two things still hold: this is
a **runtime** fact that an Evergreen update can revoke (V16's second half is open, and the probe is untracked
in git), and the payload has traps — **a synchronous `S_OK` is not an acknowledgement** (the bogus-method
control returned `S_OK` synchronously and failed only on completion), and **this runtime answers in the legacy
columnar shape**, not the `nodeTreeSnapshot` current CDP docs describe, so an analyzer written from those docs
reads **0 nodes against a payload holding 35**; `textBoxes.start`/`length` are **UTF-16** code-unit offsets.
Rows V16/V17 carry all three in full, and V18 is the gate that makes a vanished domain survivable.

### 1b.2 Affected claims after the ruling **and** the accessibility scope decision

| Audit id | Capability | COM surface | via CDP | Status |
|---|---|:-:|---|---|
| C-58 | Accessibility tree for extraction tier 2 (backlog §4.2 L432) | **d** | would be `Accessibility.getFullAXTree` | **OUT OF SCOPE BY DECISION** (2026-11-13). Tier 2 is deleted from the spec (D-01); no AX call, no AX row in the contract. Not a (d) any more — a (d) is something we cannot reach **and want** |
| C-58a | `accessibilityPath` as an anchor tier | **d** | — | **OUT OF SCOPE BY DECISION.** Deleted (D-01). Even before the decision it was unusable as a *persisted* anchor: CDP node identity does not survive a document change |
| C-74 | Element-by-locator resolution **for actions** | **d** | `Accessibility.queryAXTree`, `DOM.getDocument`, `DOM.focus` | **OUT OF SCOPE BY DECISION for its AX half.** The *need* is not dropped: action targets are `TargetResolution::DomLocator` (`contract.rs:211`), resolved just-in-time via `ExecuteScript` (1870) — the design, not a fallback. The CDP **act** path stays forbidden regardless, so this row could not have been rescued by CDP anyway |
| C-90 / C-91 | Synthetic input at semantic level (§7.2 rank 1-2) | **d** | `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent` | **d — unchanged.** Still wanted, explicitly not permitted |
| C-92 | Host-side screenshot of a region | **a/b** | `Page.captureScreenshot` | **not permitted** — not on the one-method allowlist. We keep `CapturePreview` (**1875**), a COM call we already own |
| — | Non-forgeable structural read | **d** | **`DOMSnapshot.captureSnapshot`** | **b → permitted** (read-only, off by default, feature-detected), and **measured callable** (§1b.1). Unaffected by the accessibility decision — see the **Still buys** rows above |

C-05 and C-45 (the two accessibility claims Carol identified in the 2026-11-13 audit) move to **OUT OF
SCOPE BY DECISION** with C-58; their verbatim wording lived only in the report-only audit and is not
recoverable from disk — see `CAPABILITY-CLAIM-AUDIT.md` (now committed, with the reclassification and the
rows flagged as unrecoverable) and `BACKLOG-DELTAS.md` §1b.

### 1b.3 What stops being true: not all extraction is forgeable any more

The docs previously implied that *every* byte we read out of a page is page-supplied JSON. On the CDP path
that is no longer accurate, and the correction is narrow:

| Row | Before | After | Net |
|---|---|---|---|
| **C-49** — main-world probe output is page-controlled and forgeable | the page can shadow our globals, forge fields, inflate the payload, or claim hidden text is visible | `DOMSnapshot.captureSnapshot` is produced by the renderer's own serialisation: structure, geometry, and **renderer-computed layout/visibility** arrive without asking page script. The page cannot speak into this channel, cannot rate-limit it, and cannot fake a bounding box for an unlayouted node (verify the exact fields, V17). First measurement: **2.2 ms, 5,809 B, 35 nodes** on a one-paragraph page — a 5 MB DOM is unmeasured (V15) | **Materially improved** for the one-shot snapshot path — the *channel* trust problem disappears there. Independent of the accessibility decision |
| **C-62** — a page-side observer feed is a page-controlled message rate | the page chooses how often we are told about mutations | **unchanged.** `captureSnapshot` is a **pull**; no CDP event domain is in scope, so a live change feed still comes from page script at a rate the page chooses | **No improvement.** The D-11 rate cap stays mandatory |
| **ANN-02** — citation path | quote + rect both self-reported by the probe | rect and visibility can carry renderer provenance; the quoted **text** is still whatever the page authored | **Fidelity improved, trust unchanged** |

**What does not improve, stated plainly.** (1) A faithful DOM read is not a truthful page — a page can author
any content we will faithfully report, so extracted text stays untrusted input and the allowlist + host-side
redaction layers (§2.2, loss #3) are unchanged. (2) The permitted path is **off by default**, so the probe is
still the default extractor and every requirement written against it (D-11's cap and schema rule,
`probe-blocked` as a typed coverage gap) still ships. (3) Coverage of third-party/OOPIF frames by either
method is unverified (V17), so no story may claim whole-page or all-frame reach on this ruling's strength.

## 2. Capability map — backlog §2, §3, §7, §8

Verdicts: **A** achievable as written · **D** achievable with degradation (substitute named) ·
**X** not achievable (substitute named).

### 2.1 §2 — user experience

| Ref | Requirement | V | Mechanism / substitute |
|---|---|:-:|---|
| 2.1 | Browser-owned floating selection toolbar, not restylable/interceptable by the page | **A** | persistent host overlay controller, transparent backing (`SetDefaultBackgroundColor` A=0), positioned by us |
| 2.1 | Streamed answer as a Word-style comment attached to the passage | **D** | the card is host-owned and always correct; **it cannot track text per frame during scroll**. Substitute: pinned state during scroll + re-anchor on scroll-settle + short glide. Quantified in §3.2 |
| 2.1 | Page cannot impersonate the AI card | **A** | a forged card is page DOM and the real card is a separate window; we additionally mark our window with a host-drawn 1 px accent (our own paint, not page-visible CSS) so a user can tell them apart. Verify what is actually distinguishable at A=0 |
| 2.1 | Multi-selector anchor + reattachment (`AnnotationAnchor`) | **A** | host-side reattachment using quote+prefix/suffix+`navigationId`+frame id; all matching logic in Rust |
| 2.1 | `accessibilityPath` anchor tier | **— removed** | **Out of scope by decision (2026-11-13)**, so this is no longer a capability question. The field is deleted from the spec (D-01) and the tier ladder becomes 5 steps. Independently it never would have worked as a *persisted* anchor — no embedder AX API on the COM surface (grep above) and CDP node identity does not survive a document change. The anchor ships as: `semanticBlockId` (our own id, minted by our extraction pass per snapshot) + quote + prefix/suffix + rect + frame id + `navigationId`; DOMSnapshot may improve **which** block we pick, never **identify** one across sessions |
| 2.2 | Context header, add/remove context without rewriting the prompt | **A** | our own chrome (Rust/overlay UI), driven by host events. No dependency on the kernel |
| 2.3 | `NavigationId`-based document identity; URL is metadata | **A** | `NavigationId` is `*mut u64` (read at 24075) and appears on NavigationStarting (24075), ContentLoading (7450), DOMContentLoaded (12513), NavigationCompleted (23739) → one id correlates the whole lifecycle. MS docs state only *"The ID of the navigation."* — monotonicity and same-document behaviour are **verify** (§7 V1) |
| 2.3 | `FrameAttached` / `FrameDetached` / per-frame coverage | **A** | `add_FrameCreated` on the webview (`ICoreWebView2_4`, 43310) and on a frame (`ICoreWebView2Frame7`, 20475) + `ICoreWebView2Frame::{add_Destroyed 19286, IsDestroyed 19293}` + `ICoreWebView2Frame5::FrameId` (20265) |
| 2.3 | `HistoryStateChanged` (pushState / fragment / back-forward) | **D** | WebView2 has no per-pushState event equivalent to `onHistoryStateUpdated`. Substitute: `add_HistoryChanged` (**1810** — presence confirmed; coverage is V11) + a probe-reported `location.href` epoch, treated as *content changed*, never as *identity changed*. Identity moves only on `NavigationStarting/Completed` |
| 2.3 | `MeaningfulContentChanged`, `SelectionChanged`, `ViewportChanged` | **D** | probe over `PostWebMessageAsJson`, throttled by us; classification is ours. Main-world caveat §3.4. **No selection event exists on the embedder surface** — the only selection APIs are `ICoreWebView2ContextMenuTarget::{HasSelection 8767, SelectionText 8771}`, which exist only while a context menu is being built, so selection must come from the probe (rate-capped: see the CTX-04 requirement in `BACKLOG-DELTAS.md` D-11) |
| 2.4 | Typed artifacts, open candidate beside, background tabs, result strip | **A** | our tab model + `SetBounds` split panes. Nothing kernel-specific |
| 2.4 | Auto-open only for safe navigation; block cart/purchase | **A** | policy in **our** process, which is the only thing that can `Navigate`/`ExecuteScript`. Stronger than the extension path, where blocking was the polite option |
| 2.5 | Native omnibox reusing Chromium classification, autocomplete, IME, policy, search providers | **X** | `AutocompleteController`/`TemplateURLService`/`OmniboxEditModel` are not exposed to embedders; we cannot reuse Chromium's classification, autocomplete ranking, keyword handling or search-provider policy. **Loss #1, already being paid:** the shipped shell has its own omnibox — a JS `<input id="omnibox">` + ARIA combobox in `ui/app.js` (40 ms debounce → `omnibox.query`), with decision authority in Rust (`src/omnibox.rs`: pure, deterministic `Mode::{Url,Search,Command,Find}`, zero-egress) fed by our own history/bookmarks stores. We own ranking, IME polish and keyword handling |
| 2.5 | "Do not build a parallel JavaScript omnibox" | **Already violated, deliberately** | Correction to my earlier draft, which called ours "a native widget, not a JS box" — it is not; it is app-owned JS in the `asset://` chrome document. What does hold is the *intent* behind the sentence: never page DOM, classifier and ranking in Rust, JS only mirrors the mode chip. §2.5 needs restating in those terms → `BACKLOG-DELTAS.md` **D-05** |

### 2.2 §3 — process ownership

| Ref | Requirement | V | Mechanism |
|---|---|:-:|---|
| §3 | Model calls, routing, compaction run outside the browser process | **A**, and **easier than a fork** | a separate executable beside the shell (`PROCESS-AND-LANGUAGE.md`). A fork would have needed a Mojo utility service; we get process isolation for free from the OS |
| §3 | Browser process owns identity, navigation, policy, action dispatch, annotation placement | **A** | all of it is our process — this is the whole point of being the embedder |
| §3 | Renderer runs narrowly scoped probes only | **D** | probes run in the frame's **main world**; WebView2 exposes no isolated-world equivalent (verify absence). Therefore probe output is **untrusted input**: size-capped, schema-validated in Rust, never an authority, and no host capability is reachable from it. **One exception the ruling opens:** a one-shot structural snapshot may come from `DOMSnapshot.captureSnapshot` instead of a probe, and that path is *not* page-authored (§1b.3) — but it is a pull, so the change-feed half of §2.3 stays probe-only, and the probe stays the default extractor because the CDP flag is off by default |
| §3 | Renderer "must not return cookies, password values, auth headers, browser storage, hidden credential fields" | **D** | not enforceable at the extraction boundary because we cannot patch Blink and the probe is main-world. Enforce as three host-side layers: (1) allowlist-shaped extraction (we ask for blocks/links/controls, never for values), (2) host-side redaction pass before the payload leaves the shell, (3) `WebResourceRequested` header stripping for egress (verify). This is policy + tests, not a structural guarantee. See loss #3 |
| §3 | Isolated IPC between browser and AI process | **A** | our own framed transport; transport choice is Bob's |

### 2.3 §7 — action runtime

| Ref | Requirement | V | Mechanism |
|---|---|:-:|---|
| 7.1 | Four levels with different capability grants | **A** | Release 0 compiles with zero mutation capabilities in the AI process |
| 7.2 | Prefer browser-native semantic command → coordinate fallback | **D** | there is no browser-native semantic command surface on the COM surface (no `autofill::`-equivalent). Ranks 1-2 collapse into "our own semantic action over `ExecuteScript` (1870)"; rank 5 coordinate action needs synthetic input into a specific controller HWND (verify: `SendInput` targeting + WebView2's own input handling). **Closed by ruling, not left open:** `Input.dispatchMouseEvent`/`dispatchKeyEvent` — the one route that would have restored real rank-1 semantics — are **not permitted** (2026-11-13: no CDP in the Interact or Commit path at any release without a new ruling, §1b.1), and independently MS docs' "may be processed out of order" made them unusable for an ordered action runtime. Coordinate clicks on web content are also the least auditable thing we could ship — keep them out until Release 2 review |
| 7.3 | Pre-action re-resolve of document identity, origin, snapshot assumptions | **A** | `NavigationId` + `Source` + frame id re-read from host state at dispatch time, never cached from a probe |
| 7.3 | Post-action observation, stop on unexpected navigation/dialog | **D** | navigation half is fine (`add_NavigationStarting` 1789, `add_NavigationCompleted` 1817, `add_NewWindowRequested` 1921) plus a post-action snapshot diff. **The dialog half is not available: `DialogStarting`, `DialogClosing`, `HasDialog` and `FrameDialog` each return 0 matches in `bindings.rs`** — this SDK exposes no page-dialog event to the embedder, so "stop on unexpected dialog" has nothing to subscribe to. Substitute: treat dialog as a *page-content* signal (probe reports `document.activeElement`/modal-host changes), rely on `add_ProcessFailed` (1852) for the wedged-renderer case, and keep the Release-0 allowlist free of commands that can trigger a script dialog. Unavailable at this SDK, not deferred |
| 7.x | Approval broker that page script cannot bypass | **A** | revised verdict, §4 |

### 2.4 §8 — data and privacy

| Ref | Requirement | V | Mechanism |
|---|---|:-:|---|
| §8 | A remote page never receives another tab's contents | **A** | snapshots are built into the shell and streamed to the AI executable; page-visible text is only what we deliberately paint into the overlay window, which is a different origin and document |
| §8 | Password/cookie/token never collected | **D** | see §2.2 row above: host-side allowlist + redaction + tests, not a Blink-level guarantee |
| §8 | Raw screenshots not persisted | **A** | we never capture them; the AI process has no file handles |
| §8 | Action audit is metadata only | **A** | log schema has no payload field; test asserts corpus text never appears in logs |
| §8 | Disable cloud models by domain; kill switch per capability without a new binary | **D** | ours: config/feature flags read at start + a signed config reload if we build it. There is no field-trial service. Runtime flip of *some* flags is fine; anything baked into `AdditionalBrowserArguments` needs a restart because environment options are fixed at creation (`create_shared_environment`, `crates/browser/src/webview2.rs`) |
| §8 | Kill switch that stops egress | **D** | `AddWebResourceRequestedFilter` + `WebResourceRequested` cancel is the lever, **but** MS docs on navigation cancellation state: *"For performance reasons, `GET` HTTP requests may happen, while the host is responding."* → interception is best-effort for navigations. Substitute for a true kill switch: disable the AI executable's own egress (it has its own socket) **and** disable the automation surface; treat WebResourceRequested as defence-in-depth, not as the guarantee |

## 3. §2.1 on the embedder-host surface — the four costs, quantified

Architecture rule that keeps cost 1 bounded: **one overlay controller per browser window, created
once, hosting the selection toolbar, every comment card, the composer and the action indicator.**
Never one controller per card.

### 3.1 Per-overlay cost — **measured 2026-11-13**, `../tools/baseline.md` §Pricing the overlay

| Item | Count | Measured / budget | Basis |
|---|:-:|--:|---|
| Extra browser process | **0** | 0 B | created from the **same** `ICoreWebView2Environment` (`create_shared_environment`); check C5 confirms +0 on every flip |
| Extra GPU / network utility | **0** | 0 B | shared per-environment (4 non-renderer children, unchanged) |
| Extra renderer | **1** | **≈ 73.6 MB** of renderer working set per renderer added (73.1–73.8 measured) | the overlay document is its own site instance because it is an app-owned origin — the shipped `asset://` handler (`ui.rs`, app.rs:2359/2398) |
| Shell-side cost | — | ≈ 2 MB (shell 28.4–38.6 MB, mode-invariant) | our HWND + rect state |
| **Total, 1 tab, dev profile** | 8 → **9 processes**; 456–462 → 524–528 MB | **+1 process, ≈ 66 MB (+14 %), once per window** | six before/after deltas: 64.2 / 65.2 / 65.7 / 66.9 / 67.0 / 68.5 MB. The ≈ 60 MB estimate this table carried was ~11 % low; the 80 MB ceiling holds. The independent gap between an overlay-less and overlay-present mode-0 sample (~65 MB) agrees with the delta method |

- **The "shares the shell's `asset://` renderer, costs 0 renderers" reading is dead**: the overlay is a
  separate `WebViewBuilder::with_id("overlay")` controller and took its own renderer on every flip. Its
  renderer is the lighter kind — a page renderer added ≈73.1–73.8 MB where the overlay's whole-tree delta was
  ≈66 MB — which is why the two numbers in the table above are not the same measurement.
- Per-annotation cost is **zero**, which is why the single-controller rule is not a style point: one controller
  per card would be ≈ +66 MB per comment, so ten cards ≈ **+660 MB** and ten renderers. Check C5 would **not**
  catch that — it counts browser processes, not controllers.
- Failure mode that C5 does guard: the overlay accidentally getting its own environment pays another browser
  process (**140.5–163.8 MB** measured) plus its own GPU chatter; the assert-the-process-count gate Carol asked
  for now ships as `--proof` checks **C0/C1/C2/C5** (attribution, one environment, process counts).
- The overlay must be excluded from tab-sleeping (it is not a tab) and must not be destroyed on navigation —
  only on window close. Absolute MB compare only from a wiped profile **and** the same mode (check **C7**), and
  every figure here is **dev profile**: a release build needs its own baseline.

### 3.2 Scroll and repositioning lag — measured, and weaker than this section claimed

Path: probe reads `Range.getBoundingClientRect()` in `requestAnimationFrame` → `PostWebMessageAsJson` →
shell → `SetBoundsAndZoomFactor` → DWM present: **two frame boundaries minimum** plus one window move.

| Claim (earlier draft) | Measured on this box (`wv2-probe` Headlines 3-4, runtime 157.0.4341.57) |
|---|---|
| "25–50 ms p50, 60–120 ms p95 reposition" | **NOT MEASURED, still an estimate.** The only working pixel instrument here (`PrintWindow(PW_RENDERFULLCONTENT)`) costs 86–113 ms per read, ~100× coarser than the band under test, and cannot see an overlay composited on top. Needs an interactive desktop (V2) |
| The host pipeline is the cost | It is not: `set_bounds` 977 µs (p99 1.41 ms, max 2.19 ms during a 121-rect fling), `SetWindowPos` on a top-level 408 µs, `postMessage` 40 µs, page↔host round trip 0.33 ms, overlay renderer sees the new computed style at p50 **4.0 ms** (p95 4.9, n=120) |
| CSS transform may beat `set_bounds` | **Follow error is exactly one reported frame of scroll, for both strategies** — 4.0/12.0/40.0/120.0 css px of error at 4/12/40/120 px per frame (6/18/60/180 physical at DPR 1.5). The follower is one frame behind and never drops a second |
| "During a fling the card sits **100–500 px** from its passage" | True only at ~4,200 css px/s of scroll — i.e. it was a claim about **velocity × frame time**, not about overlay technology. The real dominant term is the overlay document's own frame period: **31.3 ms** here (a remote-desktop artefact; ~16.7 ms expected on a 60 Hz interactive display), which alone puts the answer in the band |

Design consequence — unchanged by the measurement, because one frame of error at fling speed is ~180
physical px and no animation makes that legible:
1. **Do not chase during scroll.** While `scrolling` is asserted the card is **pinned**: frozen at the
   last position with a dimmed leader, or docked to the viewport edge. Zero `SetBounds` calls while
   scrolling (cap: at most one per rAF, none while flinging).
2. **Re-anchor on settle** — 120–150 ms after the last scroll signal — with one 80–120 ms glide. Motion
   communicates state, never decorates: exactly one animation per settle, none on focus loss.
3. **The leader line may live in the page; the card may not.** A decorative connector in the page's own
   layer may be fake — a fake line harms nothing — while the card, composer and approval surface stay
   host-owned: most of the Word-margin look, nothing authoritative in the DOM.
4. Acceptance gate: reposition ≤ 200 ms p95 after settle, **0** visible per-frame lag during a scripted
   fling (because we are pinned), and no latency number quoted again until V2 runs where the screen can
   actually be read.

**One shared round-trip with §2.3: yes.** One probe message carries
`{probeSeq, reportedNavigationId, reportedFrameHint, rect, scrollY, viewport, docEpoch}`; the shell uses
**its own** `NavigationId`/`FrameId` for identity and treats the page-reported copies as correlation
hints only, since they are attacker-controlled. Convert CSS px → physical px with the controller's zoom
factor (`SetBoundsAndZoomFactor` exists precisely because those two disagree).

### 3.3 Focus and input

| Question | Answer | Basis |
|---|---|---|
| How many input surfaces | 2 controller HWNDs under our frame window; Win32 focus is exclusive between them | `hwnd()` (`wry` webview2/mod.rs:1528) + standard Win32 |
| Can the page steal focus back | **Yes** — `element.focus()`, `autofocus`, click-through-to-page, and any navigation we trigger | web platform behaviour |
| Must the card die on focus loss | **No.** Rule: focus loss removes the active ring and keeps the draft (held in the shell, never in page storage). Only `NavigationId` change, tab close, or explicit dismiss detaches | our invariant, matching §5's stale rules |
| Host-mediated traversal | `MoveFocus(reason)` (**9341**) and `add_MoveFocusRequested` (**9345**) — card → page and page → card on our keybinds, so keyboard-only operation works | verified present |
| Keeping the overlay out of the page's Tab order | **No `IsTabbable` in this binding (0 occurrences).** Substitute: keep the overlay document fully `tabindex="-1"`/`inert` until invoked, so it cannot enter a cycle it was not put in. Verify whether an unfocused controller can ever receive Tab in the frame's cycle | verified absent; substitute unverified |
| Stranding scenarios | page opens a window (`add_NewWindowRequested`, **1921**), page enters fullscreen (`add_ContainsFullScreenElementChanged`, **1950**, base `ICoreWebView2`), or our window is minimised → overlay can end up mispositioned or above the wrong thing. Rule: re-assert overlay bounds + z-order on those events, and hide the overlay when the frame window is not active | presence verified; behaviour per event still **verify** (§7 V12) |
| Hostile focus-flicker | a page that spams focus changes must not make the UI strobe → no motion, no sound on focus transitions | design rule |

### 3.4 Frames, coverage, and large payloads

**What the frame surface gives**: `add_FrameCreated` on the webview (`ICoreWebView2_4`, **43310**) and on a
frame (`ICoreWebView2Frame7`, **20475**, so nested frames are reachable); `ICoreWebView2Frame5::FrameId`
(**20265**, `*mut u32`) as the key; `add_Destroyed`/`IsDestroyed` (**19286** / **19293**) as teardown →
**CTX-05's per-frame coverage gaps, including `inaccessibleFrames`, are representable**; per-frame script and
messaging on `ICoreWebView2Frame2` (**19696** / **19701**), so a frame is addressed rather than broadcast to;
and `PostWebMessageAsJsonWithAdditionalObjects` (`ICoreWebView2_23`, **42136**) /
`PostSharedBufferToScript` (`ICoreWebView2_17`, **41009**) for large payloads **into the overlay only**.

**What it does not give: no isolated world.** `AddScriptToExecuteOnDocumentCreated` (**1859**) runs in the
frame's main world (verify — no `Frame::` method checked above names a world concept). Three consequences we
design for: the page can see, shadow, or delete our probe globals, so absence/tampering is a **typed coverage
gap** (`probe-blocked`) surfaced in the context header, never a silent empty answer; the probe holds no
capability and never receives AI content (probe → shell → overlay is one-directional); payloads are
schema-validated and size-capped in Rust before they count as data. **Never** `PostSharedBufferToScript` into
a page frame — that hands our context bytes into a page-controlled document, a direct §8 violation.

## 4. §7 revised verdict — the approval gate

Previous verdict: "unforgeable dialog yes, un-bypassable no, fork required." Revised:

| Property | Achievable? | Mechanism |
|---|:-:|---|
| Dialog cannot be restyled, removed, occluded or spoofed by the page | **A** | host-owned window, outside the document (§1) |
| Dialog geometry cannot be controlled by the page | **A** | we centre it on the frame window and never at a renderer-supplied rect. **Invariant: nothing security-relevant is ever positioned from renderer-reported geometry** — cost 3.2 shows that geometry is renderer-influenced and laggy |
| The action provably cannot execute while pending | **A** | execution lives behind one dispatcher in our process; the AI executable holds no capability object to call. This is structurally stronger than the extension path, where "cannot bypass" meant "the extension politely doesn't" |
| The page cannot keep mutating while the user decides | **D** | `ICoreWebView2Controller::put_IsEnabled` **does not exist in this binding** (`Controller` has `IsVisible` 9309 / `SetIsVisible` 9313 only; the `SetIsEnabled` at **7748** belongs to `ICoreWebView2ContextMenuItem`, and `IsEnabled`/`Enable` at **4256 / 4260** belong to `ICoreWebView2BrowserExtension`). There is also no `IsTabbable`. Substitute that certainly works: Win32 `EnableWindow(tabControllerHwnd, FALSE)` on the tab's own controller HWND (obtained via `WebViewExtWindows::controller()` → `hwnd()`), re-enable on decision. **Verify** it blocks input without visual artefacts and that WebView2 does not re-enable itself on navigation (§7 V4) |
| Cannot be dismissed by page-synthesised input | **A** | the dialog is a separate window; page-generated events are confined to the page's HWND |

**§7 needs no fork.** In Release 0 it needs no broker either: the allowlist has zero mutation
entries, so the broker is an empty-policy shell that returns typed `blocked`.

## 5. The three things genuinely lost by not forking

1. **The native omnibox.** `AutocompleteController`, `TemplateURLService`, `OmniboxEditModel` and
   `LocationBarView` are not exposed to embedders, so Chromium's classification, autocomplete ranking,
   keyword handling and search-provider policy are all unwritable from here. We write our own (Rust
   classifier + ranking, app-owned JS editing surface, our own stores). The one clear loss.
2. **Per-frame privileged anchoring inside the page's own compositor.** The *only* thing a window loses:
   authority and arbitrary-rect positioning are **solved in existing code** (§0, §1), but a window cannot
   be a compositor layer, so frame-rate attachment is gone — no Word-style margin note that tracks text
   every frame, no compositing with page scroll without a round trip (§3.2).
3. **The runtime is not ours.** No engine patches, no in-Blink enforcement point, no field-trial rollout,
   no pinned engine version unless we distribute fixed-version. So §3/§8's "the renderer must never return
   credentials" degrades from a *structural guarantee* to *our allowlist + host-side redaction + tests*,
   and navigation-layer egress control is best-effort — MS docs: *"For performance reasons, `GET` HTTP
   requests may happen, while the host is responding."* We compensate by making the AI executable the only
   thing holding a model socket, so the kill switch we control is the one that matters.

**Removed from the loss column, not recovered (2026-11-13).** Earlier drafts counted **the embedder
accessibility tree** as a loss (0 matches for `accessibilit` / `IAccessible` / `uiautomation` / `UiaRaise`,
while backlog §4.2 ranked it as extraction tier 2). The user then ruled accessibility **out of scope**, so it is
not something we want and cannot be a loss; it is recorded here rather than quietly deleted so the change is
attributable — tier 2 and `accessibilityPath` deleted from the spec (D-01), the CDP `Accessibility` domain never
called (§1b.1), the AX half of the locator ladder absent rather than reserved in code (`contract.rs:211`,
`tests/action_levels.rs:43`), and the tree itself worth far less than the plan assumed once it was finally read
(§1's absence bullet: **38 nodes, 0 carrying bounds**, only 18 joinable to DOM geometry). **Not dropped:**
keyboard navigation, focus handling and IME for our own chrome (C-42) — majority-user input quality, every release.

## 6. Answer

**Is a fork required for Release 0? No — R0 is fully buildable on the WebView2 kernel and nothing in
§2/§3/§7/§8 blocks it.** A fork would still buy only §5's three items, of which the omnibox is the sole
user-visible difference. §2.1 is not merely reachable here, it is already implemented and already anchors to a
host-supplied rect; the last structural objection reduced to one measurement, which has since been made (§3.1,
§3.2, V3).

## 7. `verify` — behaviour, not existence. Status added 2026-11-13; raw logs in `tools/wv2-probe/results/`

| # | Claim to test | Test | Status |
|---|---|---|---|
| V1 | `NavigationId` semantics: monotonic? unique across tabs? stable across pushState/fragment/back-forward? does a prerender→activate reuse it? | log id on all four events across a scripted 100-navigation walk (redirect chains, pushState, reload, back/forward, target=_blank); assert which transitions change it | **ANSWERED** (runtime 157.0.4341.57): one counter per **browser process**, strictly increasing, never reused, **no id at all** for pushState/replaceState/fragment, shared with subframes, and **non-dense** (one failed navigation consumed 3 ids; 7 spent with no event delivered). `navigation_sequence` stays the authority. Prerender→activate not exercised. `tools/README.md` §"The `NavigationId` experiment" |
| V2 | Overlay reposition latency and the true cost of the **existing** overlay controller (not a new one) | instrument timestamps at probe `rAF` → shell receive → `SetBounds` return → present; count `msedgewebview2` processes and private working set with 0 tabs, 1 tab, 1 tab + overlay open | **PARTLY** (§3.1, §3.2): cost measured — **+1 renderer, +0 browser processes, ≈ 66 MB** (six deltas 64.2-68.5, `../tools/baseline.md`); follow error = exactly one reported frame at every speed; `set_bounds` 977 µs, style-observed p50 4.0 ms. **On-screen rect→painted NOT MEASURED**: the only working pixel instrument costs 86-113 ms/read on this box. Needs an interactive desktop |
| V3 | The shipped overlay's transparency/hit-testing over live content: A=0 per-pixel alpha correct? flicker? pass-through where the card is not drawn? | overlay a checkerboard test page, diff pixels, confirm clicks land on the page in transparent regions | **PARTLY** (`wv2-probe` Headline 5): `WindowFromPoint` says the **card-sized** overlay shape passes transparent-region clicks **by geometry**, and a **full-bleed** transparent overlay never does (window rect is the hit region; `WS_EX_TRANSPARENT` does not help; `WS_EX_LAYERED` is refused on a child). Delivered `mousedown` and pixel diffing **NOT MEASURED** (no synthetic input, no screen DC here) |
| V4 | `EnableWindow(controllerHwnd, FALSE)` as approval modality | confirm keyboard+mouse blocked, no repaint artefacts, behaviour across navigation and after the dialog closes | open. **In scope** — keyboard/focus/IME for our own chrome is user-confirmed |
| V5 | Whether an unfocused overlay controller can enter the frame window's Tab cycle | press Tab 50× in a page with the overlay present but unfocused; assert focus never enters it | open (in scope — keyboard path) |
| V6 | Focus-steal behaviour: page `element.focus()` while composer open | assert card survives, draft intact, no strobe | open |
| V7 | Whether `AddScriptToExecuteOnDocumentCreated` runs in OOPIF frames, and whether `ICoreWebView2Frame` is handed to us for cross-origin frames | 3-level page: cross-origin iframe with a nested same-origin iframe; assert frame events + per-frame `ExecuteScript` return values | open |
| V8 | Probe tamperability in the main world | page defines getters on our probe's globals, deletes them, and overrides `PostWebMessageAsJson`; assert we emit `probe-blocked` rather than an empty success | open |
| V9 | `WebResourceRequested` cancel semantics + whether request headers are visible/modifiable there | block a known tracker; verify no socket write; verify cookie header stripped | open |
| V10 | Per-frame `add_NavigationStarting` args carry the same `NavigationId` family as top-level | correlate ids across top and frame events | **ANSWERED** as a side effect of V1: same counter, reported on `FrameNavigationStarting` (iframe ids 14, 15 between top-level 9 and 16) |
| V11 | `add_HistoryChanged` (**1810**) coverage of pushState — name confirmed present, coverage unknown | pushState 20×; count events | **ANSWERED**: `SourceChanged` + `HistoryChanged` each fired once per `pushState`/`replaceState`/hash change, and neither interface carries a `NavigationId` — so it is a *content changed* signal only, exactly as §2.1's row assumed |
| V12 | Overlay behaviour under fullscreen / new-window / minimise / DPI change | scripted walkthrough, assert no stranded card | open |
| V13 | Origin treatment of the **shipped** UI origin — the `asset://` custom protocol registered with `with_https_scheme(true)` (app.rs:2359/2398): can page script reach it, does the overlay get its own site instance, and does `ui.rs`'s traversal/ADS/8.3 gate hold at runtime as it does in unit tests? (`SetVirtualHostNameToFolderMapping` is now the fallback, not the plan) | page attempts `fetch` and `window.open` against the asset origin; assert denial and process separation | open, but the **site-instance half is now answered by §3.1's measurement**: the overlay reliably gets its **own** renderer, so it is its own site instance in practice |
| V14 | Message size ceiling for `PostWebMessageAsJson` (drives whether shared buffers are mandatory) | post 1/8/32 MiB JSON payloads; find the failure point | open |
| V15 | Cost of the permitted CDP method — now unblocked by V16: per-call latency, `DOMSnapshot.captureSnapshot` payload size on a 5 MB DOM, whether the documented out-of-order behaviour is observable with a serialising queue, and the main-thread cost on the page | measure against the same page used for V2 so CDP cost is comparable to the DOM-probe cost, and set the snapshot cadence cap from the result. Still no product-facing surface (§1b.1), so a bad number means "stay on the probe path", not "ship slower" | **PARTLY**: 2.2 ms / 5,809 B / 35 nodes on a one-paragraph single-document page. A real DOM, multiple frames, `computedStyles` requested, and the ordering question are unmeasured |
| V16 | **Gate for everything above: is `CallDevToolsProtocolMethod` (1897) callable from an embedder at all?** Second half: does it still work after an Evergreen runtime update? | flag on, minimal params: assert a JSON result and a zero error for `DOMSnapshot.captureSnapshot`; call a bogus `{domain}.{method}` and assert the failure arrives as a **completed call**, not a hang or page fault; assert the page still loads and stays interactive in every case; then update the Evergreen runtime and re-run | **PASS on callability** (runtime 157.0.4341.57, DevTools never opened, `AreDevToolsEnabled` false, no extra browser args): completion `S_OK`, and the bogus method failed **asynchronously** with `0x80070057` / CDP `-32601` rather than hanging — but a sync `S_OK` is **not** an ack, so branch on the completion result. **The second half (survives an update) is still open**, and the probe is untracked in git. §1b.1 carries the payload-shape traps |
| V17 | Coverage and exact parameter shape of `DOMSnapshot.captureSnapshot` across **OOPIF / third-party frames**; which fields carry renderer-computed visibility and geometry; whether the field names match what we intend to pin | 3-level page (top + cross-origin iframe + nested same-origin iframe) and assert whether the snapshot covers the child document or only the root; pin the accepted parameter object and response field names per runtime version in `tools/baseline.md`, treating any field we read that is not in the pinned shape as absent data, never as zero | **PARTLY, and the answer changed the plan**: this runtime returns the **legacy columnar** shape (`documents[0].nodes` parallel arrays + `layout.{bounds,nodeIndex,text}` + a shared `strings` table), **not** `nodeTreeSnapshot`, so an analyzer written against current docs reads 0 nodes; `textBoxes` carry per-line rects with **UTF-16** offsets. OOPIF coverage and multi-document `documents[]` unmeasured (1 document observed) |
| V18 | The degrade path, which is the part the ruling actually depends on | force failure three ways — flag off, bogus/removed domain name, and a method that errors mid-session — and assert in all three: the typed `unsupported` failure surfaces in the context header, the main-world probe takes over, `probe-blocked`/coverage accounting stays honest, **and the page load is unaffected**. This is the gate that lets a CDP domain vanish in an Evergreen update without breaking browsing | open. Only the "unknown method" leg is characterised (the V16 control): the *removed-by-update* case cannot be produced on a runtime that still has the method |
