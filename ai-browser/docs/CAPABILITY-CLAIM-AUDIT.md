# Capability claim audit — 79 fork-era claims, re-filed against the kernel we actually chose

**What this file is.** Every capability sentence in `ai-native-browser-design-and-engineering-backlog.md`
that was written while the plan was "our own thin Chromium fork", re-filed one row per claim against the
kernel the user actually chose on 2026-11-13: **the existing WebView2 Evergreen runtime, driven by our own
Rust embedder shell** — no fork, no checkout, no `depot_tools`, no CEF, no Electron. Computed line numbers
below are the backlog's own; `bindings.rs` numbers are **vtable-slot** lines in
`webview2-com-sys-0.39.1/src/bindings.rs` (44,394 lines), grep-derived 2026-11-13 — not the generated safe
wrapper a few lines earlier. Companions: `FORK-OR-CDP.md` (capability map + CDP ruling),
`RELEASE-0.md` (work order + budgets), `BACKLOG-DELTAS.md` (the spec edits this audit implies),
`../tools/baseline.md` + `../tools/wv2-probe/RESULTS.md` (**the measured numbers**).

## Provenance, and the one thing this file cannot recover

The audit began life **report-only**: it was delivered in conversation and never written to disk. When the
accessibility scope decision landed the same day, it moved claims by id, and two of those ids — **C-05** and
**C-45** — had no copy anywhere on disk. **Their verbatim wording is gone.** They are filed here at the end
with the literal marker `verbatim text not captured` rather than reconstructed into plausible sentences, and
`FORK-OR-CDP.md` §1b.2 and `BACKLOG-DELTAS.md` §1b carry the same caveat. Nothing else here is a guess:
every other row's claim is re-derived from a backlog line, a `bindings.rs` slot, a grep absence, or a type
that already compiles in `../crates/contract/src/contract.rs`.

**Row count and id policy.** 79 rows, the count the 2026-11-13 instruction names. Twelve labels
(**C-05, C-42, C-45, C-49, C-58, C-58a, C-62, C-74, C-80, C-90, C-91, C-92**) already appear in committed
docs; they **keep their original numbers and meanings** so every cross-reference still resolves. All other
ids are assigned fresh, in backlog order, and no id is above C-92. The sequence is therefore not strictly
ascending (a retained label sits between new ones), and fourteen numbers are **not** used — **C-48, C-60,
C-77, C-78, C-79, C-81 to C-89**. Those are gaps left by rows folded into a neighbour during the sweep (for
example viewport plus selection, redirect plus navigation-start, structured-data plus page-type), **not
lost rows**; nothing was deleted to hit a number, and the 79 rows on disk are verified by count rather than
by assertion.

## The class scale

| Class | Meaning |
|:-:|---|
| **a** | Inherited for free. The sentence holds as written on this kernel. |
| **b** | Not inherited — **we build it in the shell.** Real work, no blocker. The inheritance claim is false; the capability is real. |
| **b\*** | Reachable **only through the CDP bridge** (`CallDevToolsProtocolMethod`, 1897). Removable by an Evergreen update; off by default; nothing may depend on it. |
| **c** | Reachable, but with a **named permanent degradation** from what the sentence implies. |
| **d** | Unreachable on this kernel by any documented route. |
| **OOS** | **Out of scope by user decision.** The capability question is moot — a thing we do not want has no cost. Distinct from **d**, which is something we cannot reach *and want*. |

**Two 2026-11-13 decisions are folded into the Out-of-scope column**, not left as open questions:
accessibility features are **deleted** ("the browser is not for everyone, but focus on 60 to 70% people"),
and §4.2's edge extractors — PDF text, video transcripts, OCR — are **cut** as *wanted-not* scope decisions
with the same 60-70 % rationale (tier "transcripts" excepted: it was **d** anyway). **Carve-out,
user-confirmed the same day:** keyboard navigation, focus handling and IME for **our own** chrome are input
quality, not assistive technology, and stay in scope in every release (C-42).

---

## A. Root premise, process model, chrome surface

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-01 | "Build a **thin Chromium fork** … Do not develop a new rendering engine" — the means by which every claim below was assumed available | L5 | **d** | Kernel decision 2026-11-13 removes the fork; the second half of L5 survives untouched. Every **a** row in this file is now an **a** obtained from a shipped runtime rather than from our own tree | — | §3 contradiction 1 |
| C-02 | Existing browser tooling may be a strong actuator but must never be the authority for identity, permissions, policy or task lifecycle | L28 | **a** | Holds *a fortiori*: our only authority is the host process, and the runtime exposes nothing above a webview that could become the authority | — | none |
| C-03 | The browser / AI / renderer boundary is plumbed with **typed Mojo** interfaces | L282, L735 | **d** | No Mojo or field-trial surface exists (0 matches in `bindings.rs`). Our boundary is our own process over our own IPC; the page channel is `PostWebMessageAsJson` **1882** / `WebMessageReceived` **1890**, per frame **19701** | — | D-03 |
| C-04 | The platform box in §3's diagram hands the coordinator an **AX tree** | L296-297 | **d → OOS** | `accessibilit`, `IAccessible`, `uiautomation`, `UiaRaise` each **0 matches**. When the AX tree was finally read (CDP, `wv2-probe` Headline 6) it returned **38 nodes, 0 carrying bounds**, only 18 joinable onto DOM geometry — names with no addressable position | **OOS by decision** | D-01 |
| C-06 | Multi-process architecture keeps site renderers away from privileged browser functionality and isolates faults | L308 | **a** *(measured)* | `--proof` C1/C2/C3: one `ICoreWebView2Environment`; **+N tabs → +N renderers, +0 browser processes**; per-renderer added **73.1–73.8 MB**. `../tools/baseline.md` | — | none |
| C-07 | The browser process owns tab/document identity, navigation lifecycle, policy, action validation and final dispatch | L317-325 | **a** *(measured)* | `--proof` C0 attributes the process tree to **our** browser process; `app.rs` owns the controllers, `webview2.rs` the environment. `BrowserProcessId` sits on the **base** interface, **1903** | — | none |
| C-08 | The AI utility process can be given **no** authority over tabs or pages | L345 | **b** | Nothing on the COM surface hands page access to a process the host did not give a webview to. It is our own executable over our own IPC, holding no webview handle | — | none |
| C-09 | Each pane/region of the workspace is a live document we create, position and stack | §1 L40-48, §2.4 | **b** *(measured)* | `ICoreWebView2Environment10::CreateCoreWebView2ControllerWithOptions` **14762**; `SetBounds` **9321** / `SetBoundsAndZoomFactor` **9336**. Measured per extra controller: **+1 renderer, ≈ 66 MB** (64.2–68.5 across six runs) | — | RELEASE-0 C2 |
| C-10 | "Tabs" are a runtime object we can enumerate and drive | implicit in §2.2, §2.3, SHL-03 | **d → b** | **`Tab` matches 0 slots** in `bindings.rs` — there is no tab concept above a webview. Structurally good news: every tab is a controller we made, so workspace ownership cannot leak | — | none |

## B. §2.1 — inline selection, overlay, anchors

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-11 | A privileged overlay / isolated surface that page JS and CSS cannot interfere with or impersonate | L92 | **a** *(ships)* | It is a **window**, not a node: page CSS cannot select it, page JS holds no handle, no `z-index` war exists. Live in `app.rs:97` + `build_as_child` (`app.rs:2388`, `.with_transparent(true)`) + `layout::Overlay` (`layout.rs:29-38`) | — | none |
| C-12 | The host can read the user's selection without page cooperation | L71 | **c** | The only host-side read is the **context-menu** path: `HasSelection` **8767** / `SelectionText` **8771**. No selection event exists (0 matches for `add_Selection*`), so outside that path selection is observer-derived and page-authored | — | D-17 |
| C-13 | An answer card stays attached to its passage while the page scrolls | L92 | **c** *(measured)* | Follow error is **exactly one reported frame of scroll** at 4 / 12 / 40 / 120 css px per frame, both tracking strategies; css-transform rect→style **p50 4.0 ms / p95 4.9** (n=120); `SetWindowPos` 408 µs, `set_bounds` 977 µs (p99 1.41). On-screen rect→painted **NOT MEASURED** (GDI 6 on this desktop) | — | D-07 |
| C-14 | An annotation reattaches after reload, and dates itself when the source text changes | L89, L90 | **b** | Shipped types: `AnnotationAnchor` (`contract.rs:242`), `AnchorReattachmentTier` (**226**). Nothing in the runtime participates. The *change* trigger rides the observer, so the trigger is untrusted even though the state machine is ours | — | D-14, D-11 |
| C-58a | `accessibilityPath` as reattachment tier 4 | L104, L116 | **OOS** | Deleted from the spec, not reserved. It could never have worked as a *persisted* anchor either: CDP node identity does not survive a document change, and the tree carries no bounds to fall back on | **OOS by decision** | D-01 |
| C-15 | Quote + prefix/suffix + fingerprint (+ `domPathHint`) is enough to re-find a passage after ordinary edits | L105-107, L113-120 | **b** | Own code plus a golden corpus we do **not** have. `domPathHint` is a tie-breaker, not a tier — the backlog concedes at L120 that DOM paths break on common updates. ANN-03's "at least the target reattachment rate" names no number; **the backlog contains no numeric target anywhere** | — | D-13, D-19 |

## C. §2.2 — the context header

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-16 | The header can state exactly which documents, tabs and selection the model was given | L126-134 | **b** | Shell-owned bookkeeping; the runtime has no concept of a workspace, a conversation, or a context set. Automatic-context-must-be-visible (L150) is a product rule riding on this | — | none |
| C-17 | "△ Embedded video has no transcript" is a reportable coverage line | L133 | **d** | No caption / text-track / transcript surface exists at all — the example header in the spec is unimplementable as drawn, independent of whether we wanted the feature | Cut by **D-22** (the one cut that was also **d**) | D-22 |
| C-18 | "Visible viewport" and "current selection" are usable context sources | L143-144 | **c** | Both observer-derived — no scroll, viewport or selection slot exists (0 matches) — so both arrive at a rate the page chooses, which is why D-11's cap is not optional | — | D-17, D-11 |

## D. §2.3 — document identity and the navigation lifecycle

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-19 | The browser process can mint a document identity that is not a URL | L159-169 | **b** *(measured)* | True, but **not** by adopting `NavigationId`: it is one counter per browser process, **non-dense** (one failed navigation consumed 3 ids; 7 spent with no event delivered), and emits **no id at all** for pushState / replaceState / fragment. `navigation_sequence` stays the authority; `upstream_navigation_id` stays audit-only (`../tools/README.md`) | — | D-17 |
| C-20 | `DocumentKey.frameTreeNodeId` has a direct runtime counterpart | L163 | **c** | `ICoreWebView2Frame5::FrameId` **20265** is the substitute. Renaming the field is cosmetic and is Bob's call with the contract tests | — | §5 (not proposed) |
| C-21 | `NavigationStarted`, including redirects | L175-176 | **a** | `add_NavigationStarting` **1789**, args carry `NavigationId` (slot **7450**); the redirect chain terminates in `NavigationCompleted` **1817**. Whether redirect is a flag rather than its own event: **verify** | — | D-17 |
| C-22 | `DocumentCommitted` | L177 | **a** | `add_ContentLoading` **1796** (+ `NavigationId` slot **12513**) | — | none |
| C-23 | `HistoryStateChanged` | L178 | **c** *(measured)* | `SourceChanged` **1803** and `HistoryChanged` **1810** exist and are the **only** signal for same-document navigation — and **neither carries a navigation id** | — | D-17 |
| C-24 | `FrameAttached` / `FrameDetached` / `DocumentDestroyed` | L179-180, L184 | **a** for frames, **c** for documents | `_4::add_FrameCreated` **43310**, `Frame7::add_FrameCreated` **20475**, `Frame::add_Destroyed` **19286**, `IsDestroyed` **19293**. No top-level *document*-destroyed slot was found — **verify**; use navigation-away plus frame teardown as the proxy | — | D-17 |
| C-25 | `TabActivated` | L174 | **b** | There is no tab object to activate (C-10); the event is ours by construction, which is what makes SHL-03's cross-workspace guarantee testable | — | none |
| C-26 | `MeaningfulContentChanged` | L181 | **c** | No embedder route. Observer-derived, therefore page-rate-limited and page-shapeable | — | D-11, D-17 |
| C-27 | `SelectionChanged` and `ViewportChanged` | L182-183 | **c** | 0 matches for either as an event; both come from the probe. They may change *content* classification but must never change *identity* | — | D-17 |
| C-28 | "This lifecycle must use browser-process navigation events rather than renderer URL polling" holds for the list it sits under | L205 | **c** | True for **7 of the 11** events; the other four must come from the observer. Keep L205 as a rule about identity-bearing events only | — | D-17 |
| C-29 | On navigation we "invalidate element handles and action targets" | L199 | **d** | We cannot mint an element handle the runtime will honour — no such surface exists. What we invalidate is our own: anchors, snapshot versions, resolved target descriptors | — | D-14 |

## E. §2.4 — candidate tabs, split panes, automatic opening

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-30 | Candidate pages can be opened in **background** managed tabs and extracted without the user visiting them | L228-236 | **b** *(measured, unbudgeted)* | A kept-alive background tab costs **+1 renderer, +97.0–98.2 MB** (σ ≈ 0.4) **as a floor** — measured on `example.*` and `iana.org`; `app.rs`'s own note says ≈ 120 MB on real content, so 8 tabs ≈ **+780 MB** and 20 ≈ **+1.95 GB** on top of a ~590 MB five-tab tree. No budget line existed for this; `RELEASE-0.md` now carries the ask | — | RELEASE-0 Budgets (C6) |
| C-31 | Split view: two live documents side by side plus the assistant | SHL-04 L746 | **b** | One controller per pane, each measured ≈ **+66 MB, +1 renderer**. `SetBounds` **9321**; `IsVisible`/`SetIsVisible` **9309/9313** for hide-without-destroy | — | RELEASE-0 C2 |
| C-32 | Automatic opening is confined to safe navigation and previews; consequential acts need stronger involvement | L252 | **b** | Approval policy is ours end to end — no runtime assist and no runtime obstacle. Already enforced in code: `ActionLevel::allowed_in_release_zero()` with guard test `crates/contract/tests/action_levels.rs:43` | — | D-12 |

## F. §2.5 — the omnibox

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-33 | "Do not build a parallel JavaScript omnibox" — satisfiable by reusing Chromium's classification | L269 | **d → b** | The classifier we would reuse is not reachable; ours is `crates/shell/src/omnibox.rs` with the UI in app-owned JS (`ui/app.js`). The sentence's **prohibition** is the part we unavoidably violate | — | D-05 |
| C-34 | Reuse Chromium **autocomplete, ranking and providers** | L269, SHL-02 L744 | **d** | 0 slots for autocomplete / omnibox. What *does* exist is a **toggle on the runtime's own autofill** — `ICoreWebView2Settings4::{IsGeneralAutofillEnabled, SetIsGeneralAutofillEnabled}` **36260/36264**, `ICoreWebView2Profile6` **32366/32370**. Precise shape of the loss: **we can switch Chromium's behaviour off, we cannot borrow its suggestions** | — | D-04, D-05 |
| C-35 | Reuse Chromium **policy and search-provider behaviour** | L269 | **d** | No policy source reaches an embedder (see C-63); the search provider is our own config | — | D-05, D-10 |
| C-36 | "a native Chromium control owns editing, suggestions, acceptance, and navigation" while WebUI owns geometry | L271 | **d** | There is no native control to host. Our omnibox is one document inside a controller we created, so the split the sentence recommends is not available | — | D-05 |
| C-42 | Keyboard navigation, focus traversal and IME for **our own** chrome | L269 (keyboard, IME) | **b** — **in scope** | `Controller::MoveFocus` **9341** / `add_MoveFocusRequested` **9345**; wry hands us the controller (`wry-0.57.2/src/lib.rs:2345`, trait at 2343). **User-confirmed 2026-11-13** as majority-user input quality — the accessibility decision does not reach it, and `FORK-OR-CDP.md` §1 carries the carve-out | — (explicitly **not** OOS) | D-05, D-20 carve-out |
| C-37 | "…keyboard acceptance behave like Chromium" as a closeable acceptance criterion | SHL-02 L744 | **d** | The comparator no longer exists on our surface, so the criterion is unfalsifiable as written. Replace it with a behaviour list we can actually test | — | D-04 |

## G. §4 — the context engine and the extraction ladder

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-38 | "Do not claim AI has read the whole page" — and coverage is something we can serialise | L414-425 | **a** *(shipped)* | The duty is kernel-independent, and the type already compiles: `CoverageGapKind` **44** (`InaccessibleFrame` 45 / `UnloadedRegion` 46 / `UnsupportedContent` 47), `CoverageGap` **51**, `CoverageStatus::Partial` **63**, `ContextCoverage` **77**. Cite these as the mechanism rather than inventing one — this is what D-22's surviving tier amounts to | — | D-22 |
| C-58 | Extraction tier 2 = the **accessibility tree** | L432 | **OOS** | Deleted, not deferred. Filed with its cost on the record: the only time the tree was read here it yielded **38 nodes with 0 bounds**, 18 joinable — so the tier we removed was worth materially less than the plan assumed | **OOS by decision** | D-01 |
| C-39 | Tier "semantic DOM" (+ tier 1 metadata's non-trivial fields) | L431, L433 | **b** | url / origin / title arrive on the navigation events; `canonicalUrl`, `language`, `pageType` need the probe: `ExecuteScript` **1870** / `_21::ExecuteScriptWithResult` **41822**, per frame via `Frame2::ExecuteScript` **19696**, injected by `AddScriptToExecuteOnDocumentCreated` **1859** | — | D-01 (renumber) |
| C-49 | Our page-side probe output is page-authored: the page can shadow our globals, forge fields, inflate the payload, or claim hidden text is visible | §3 L349-356, CTX-01 L753 | **c → improved by b\*** | `DOMSnapshot.captureSnapshot` is Blink's own serialisation on a channel the page cannot speak into. **Measured callable**: completion **S_OK, 2.2 ms, 5 809 B, 35 nodes, 23 layout rects, 21 `textBoxes`**, live https page, DevTools never opened, `AreDevToolsEnabled=false`, no extra browser arg. **Extracted content stays page-authored — only channel trust changes.** Off by default, feature-detected, typed degrade | — | D-11, D-21 |
| C-40 | Tier "rendered visibility/layout" | L434 | **b** and **b\*** | Probe `getBoundingClientRect` (rect→style p50 4.0 ms) or the snapshot's layout arrays. Renderer-computed visibility exists **only** on the CDP path; the exact field list is **verify** (V17), as is a large-DOM payload size (V15) | — | D-11 |
| C-41 | Tiers "structured data" and "page-type extractors" | L435-436 | **b** | Both read out of the same DOM pass; no kernel surface involved. QAL-01's golden pages do not exist yet, so "each extractor has golden-page tests" is currently unevidenced | — | D-22 |
| C-43 | Tier "PDF/document extractors" | L437 | **d → OOS** | The built-in viewer exposes **no** content API to an embedder. `ICoreWebView2_7::PrintToPdf` **43663** is page→PDF — an *output*, not a PDF text read — so it is not a workaround. Cut as **wanted-not**; the required behaviour is the coverage report (C-38) | **OOS by decision** (D-22) | D-22 |
| C-44 | Tier "transcripts" | L438 | **d** | No caption / text-track / transcript surface. The single cut with genuinely **no path** rather than no appetite — recorded so the 60-70 % rationale is not over-claimed | Cut (D-22); class stays **d** | D-22 |
| C-46 | Tier "OCR / vision fallback" | L439 | **d → OOS** | `CapturePreview` **1875** gives pixels; there is no pixels→text surface reachable from an embedder. Cut as **wanted-not** | **OOS by decision** (D-22) | D-22 |
| C-92 | Host-side screenshot of a region, for a visual path | (implied by tier 9) | **a** | `CapturePreview` **1875**, a COM call we already own — so this stays reachable *even though* CDP `Page.captureScreenshot` is not on the permitted-method allowlist. The tier that consumed it is cut | Consumer cut (D-22); capability **a** | D-22 |
| C-47 | "the browser cannot access third-party frames" | L425 | **c** | Per-frame script and identity exist (**19696**, **20265**), but **OOPIF coverage through the runtime is unmeasured** (V17). Until measured, report `CoverageGapKind::InaccessibleFrame` rather than assuming either way | — | D-22 |
| C-76 | "…or canvas text" | L425 | **d** | True as written and stays true. The tier that could have worked around it (OCR) is cut, so this is now a permanent, disclosed gap rather than a to-do | Consumer cut (D-22) | D-22 |
| C-50 | §4.2's CDP sentence: CDP "demonstrates useful structured inspection surfaces for DOM, accessibility, page, target, and browser state" | L441 | **a** *(as a fact)* / **d** *(as an invitation)* | Every domain named is callable in principle, but only **`DOMSnapshot.captureSnapshot`** is permitted, and `Accessibility` is withdrawn by the scope decision. Three traps measured: a sync **S_OK is not an acknowledgement**; an invented method returns sync S_OK then fails at completion with **0x80070057** (= the documented `E_INVALIDARG`, arriving asynchronously); and this runtime answers in the **legacy columnar** shape, so an analyzer written against the `nodeTreeSnapshot` docs reads **0 nodes against a payload holding 35**. `textBoxes.start/length` are **UTF-16** code-unit offsets | `Accessibility` half **OOS** | D-21 |
| C-62 | A page-side observer feed tells us about mutations (the mechanism under §4.3's classification table) | L443-470 | **c** — **unchanged by CDP** | `captureSnapshot` is a **pull**; no CDP event domain is in scope (`GetDevToolsProtocolEventReceiver` 1915 / `add_DevToolsProtocolEventReceived` **12937** stay unused). A live change feed still arrives at a rate the page chooses, which is why D-11's cap stays mandatory | — | D-11 |

## H. §5 / §6 — stale context and models

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-51 | An action from a stale answer cannot execute without replanning | L509-510 | **b** | Ours, and it already has teeth: `ActionLevel::allowed_in_release_zero()` plus the guard test `action_levels.rs:43`. The "content changed" trigger is the untrusted half (C-14) | — | D-14 |
| C-52 | "A navigation action must target a stable tab/document handle, not 'the active tab' resolved earlier" | L511 | **b** | Handles are our `TabId`/`DocumentKey`; the runtime's notion of a target stops at a webview we created. §6 (model platform) carries **no** kernel-dependent claim at all, which is why it gets no rows | — | D-14 |

## I. §7 — the action runtime

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-53 | Four action levels — Observe / Navigate / Interact / Commit | L588-593 | **a** *(shipped)* | A policy model, not a platform feature; already encoded as `contract.rs` `ActionLevel` with the Release-0 restriction and its guard test | — | D-12 |
| C-54 | §7.2 rank 1: "browser-native semantic command" | L596 | **d** | No embedder semantic-command surface (no autofill-command / `ExecCommand`-class API on the COM surface). **Closed, not open:** CDP `Input.dispatch*` was ruled out on 2026-11-13 — no CDP in the Interact or Commit path at any release — so nothing re-opens this rank | — | D-15 |
| C-90 | The same rank read as "the browser performs the act" (synthetic semantic input) | L596 | **d** | Reachable only through CDP `Input.dispatchMouseEvent` / `dispatchKeyEvent`, **forbidden** by the ruling; no COM equivalent exists. Still wanted, explicitly not permitted | — | D-15 |
| C-91 | The same for rank 2 (invoking a page-advertised tool at semantic level) | L597 | **d** | As C-90. Rank 2's **read** half survives — a page's declared tools are visible in the DOM pass (C-41) — only the trusted-execution half dies here | — | D-15 |
| C-74 | Rank 3 / ACT-02: a "stable **accessibility**/DOM locator format with just-in-time resolution" | L598, L785 | AX half **OOS**; DOM half **b** | `TargetResolution::DomLocator` (`contract.rs:211`) is **the design, not a fallback**, and `action_levels.rs:43` fails if anyone adds an AX variant. The CDP act path is forbidden regardless, so this row could not have been rescued through the bridge even if AX were wanted | AX half **OOS by decision** | D-13 |
| C-55 | Rank 4 "visual target" and rank 5 "coordinate action only as fallback" | L597, L600-601 | **c** | Dispatch happens from the probe, so the events are page-trusted (`isTrusted: false`) and there is **no host input-injection API** in this binding; per-site behaviour is **verify**. Ranks 4-5 are the only ranks left, so "last resort" describes the whole ladder | — | D-15 |
| C-56 | Pre-action: resolve target again, verify document identity, origin, snapshot assumptions, evaluate policy, obtain approval | L621-626 | **b** | All of it is host-side and ours; `PermissionRequested` **1845** is available where the runtime brokers a permission, `ContainsFullScreenElementChanged` **1950** for a common UI-mode surprise. The approval modality itself is an `EnableWindow` dance, not a Chromium sheet (D-12) | — | D-12 |
| C-57 | Post-action: "stop on unexpected navigation or **dialog**" | L633 | **d** *(dialog half)* | **No page-dialog event exists** in this SDK: `DialogStarting`, `DialogClosing`, `HasDialog`, `FrameDialog` → **0 matches**. Navigation half is fine (**1789**); a wedged renderer surfaces as `ProcessFailed` **1852** | — | D-16 |

## J. §8 — data, privacy, controls

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-59 | A remote page may receive safe metadata but never the contents of other tabs | L641 | **b** | Enforced only by our own egress path. `AddWebResourceRequestedFilter` **1969** and `_22::…WithRequestSourceKinds` **41968** are present but their exact semantics are **verify** | — | D-08 |
| C-61 | The renderer probe "must not return cookies, password values, authentication headers, browser storage, hidden credential fields" | L358 | **c** | Not structural on our route: a main-world probe can read whatever the page can. The guarantee is **policy plus a test we must be able to fail** — which is also how §12 L870's absolute should be read | — | D-08 |
| C-63 | SAF-03's "**enterprise-policy**" checks | L807 | **d** | Nothing to read: no enterprise-policy surface reaches an embedder on this kernel, and there is no domain-joined policy source | — | D-10 |
| C-80 | "Kill switch for every **remotely rolled-out** capability" | L664, L816 | **c** | There is **no remote rollout service** on this kernel. What survives is a runtime-read flag that fails to disabled — a requirement on our own flag surface, not on a vendor one. Same limit as C-66 | — | D-09 |

## K. §9 / §12 — delivery lines and the definition of done

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-64 | Release 0's lines: "Chromium shell integration", "inline overlay", "navigation invalidation", "no page mutations" | L674-680 | **a** *(measured)* | All four hold on this kernel; only the word "Chromium" is a casualty. The overlay ships and costs ≈ 66 MB; invalidation rides **1789/1796/1817**; "no page mutations" is a **test**, not a comment (`action_levels.rs:43`) | — | §3 contradiction 1 |
| C-65 | "Citations scroll to the supporting passage" | L865 | **c** | Scrolling is a page-side act through the probe, so it carries C-13's one-frame follow error and C-55's trust caveat. The backlog also classes scroll as *Navigate* (L591) while listing scroll-to-source as a *read-only* tool (L784) — same defect, two places | — | D-07 |
| C-66 | "Feature flags and kill switches work **without requiring a new binary**" | L872 | **c** | Achievable only if the flag is read at runtime — it cannot live in `AdditionalBrowserArguments`, which is frozen at environment creation. This is the mechanism C-80 depends on | — | D-09 |

## L. Story-level claims (Epic A-J rows that assert an inherited capability)

| id | Claim, as the design inherited it | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| C-67 | FND-05 (P0): "Establish **Chromium upstream-sync** pipeline … a clean upstream merge can be built and smoke-tested automatically" | L737 | **d** | Nothing to sync and no checkout. The residual truth: we *are* hostage to the runtime's release cadence — we just cannot pin, preview or patch it. This is why V16's second half (survives an Evergreen update) matters more than it looks | — | D-02 |
| C-68 | SHL-01 (P0) acceptance: "…focus, keyboard, **accessibility**, localization, and **forced colors** work" | L743 | **d → OOS** | Forced colors = Windows high-contrast conformance, an assistive-technology requirement. As written this is a P0 that cannot be signed off; focus + keyboard + localization survive unchanged | **OOS by decision** (the accessibility/forced-colors clauses only) | D-20 |
| C-69 | CTX-01 (P0) acceptance: "metadata, semantic-DOM, **accessibility**, link, and table extraction" | L753 | **a** minus the OOS clause | Delete the one word and the story is ordinary work on **b** paths. This is why CTX-01 left the "cannot be completed" list when accessibility went out of scope | Accessibility clause **OOS by decision** | D-01 |
| C-70 | ANN-05 (P1): "All operations are **keyboard accessible**" | L768 | **b** | Read as *keyboard operability of our own chrome* it is in scope and stays in scope — user-confirmed 2026-11-13. Read as screen-reader conformance it would be OOS. The backlog's wording does not distinguish; **D-20's Non-goals block** is what does | — (in scope) | none — no delta touches it |
| C-71 | CTX-06 (P1): "Add **PDF**, **transcript**, structured-data, product, search-result, and article extractors. Each extractor has golden-page tests" | L758 | mixed | PDF and transcript clauses are cut (transcript was **d**, PDF is **d**-for-an-embedder); the other three are ordinary **b** work. "Each has golden-page tests" is unevidenced while QAL-01's corpus does not exist | two clauses **OOS by decision** | D-22 |
| C-72 | QAL-03 (P0): "feature flags, kill switches, **staged rollout**, and fail-to-disabled … every external provider and action level independently disableable" | L816 | **c** | "Independently disableable" and "fail to disabled" are ours to build and testable today; **staged rollout** presumes the rollout service C-80 says does not exist | — | D-09 |
| C-73 | §12's DoD as a whole is the definition of done for "the first useful release" | L858-873 | **c** | It is Release 1's list (annotations, multi-tab, model selection, dashboards) wearing Release 0's label. Three bullets are unreachable **as written**: L863 (element references we cannot mint), L870 (an absolute no one can prove without the test), L873 (dashboards "operating") | — | D-06 |
| C-75 | QAL-01 (P0): a deterministic corpus "covering articles, shopping, SPAs, frames, **PDFs**, lazy loading, authenticated pages, adversarial content" | L814 | **b**, with one dead item | The corpus is ours to write and is the thing every **b** row above secretly depends on. Its **PDF** item now contradicts D-22's cut — an internal inconsistency in the backlog after the decision, not before it | PDF item inconsistent with the cut | D-22 |

## The two rows this file cannot recover

| id | Claim | Src | Class | Evidence on the chosen kernel | Out of scope? | Δ |
|---|---|---|:-:|---|---|---|
| **C-05** | An accessibility feature claim, identified by id in Carol's 2026-11-13 reclassification instruction | — | **d → OOS** | **verbatim text not captured** — the audit was report-only when the decision landed, so no copy of this row exists on disk. Nothing is being withheld; the wording is gone | **OOS by decision** | none needed beyond D-20's non-goal line |
| **C-45** | An accessibility feature claim (same provenance caveat) | — | **d → OOS** | **verbatim text not captured** | **OOS by decision** | as above |

If either wording exists in a chat log or a note, send it and these two rows become real in one edit. Until
then they are counted, flagged, and **not reconstructed** — inventing two accessibility claims and then
deleting them would make the tally above look checked when it is not.

## Counts

**79 rows** (verified by counting the table rows, not by hand). Tally is by **headline class** — a row with a
second leg is counted once, where it is filed, and the leg is named below.

| Headline class | Rows | Notes |
|---|---:|---|
| **a** — inherited for free, holds as written | **13** | 6 of these are already shipped in our code or measured on a running runtime: C-06, C-07, C-11, C-38, C-53, C-64 |
| **b** — not inherited; we build it in the shell | **20** | C-40 carries a second **b\*** leg; C-70 is in scope by the user's confirmation, not by accident |
| **c** — reachable with a named permanent loss | **18** | C-49 is filed here and is the **only** headline carrying a **b\*** improvement (**c → improved by b\***) |
| **d** — unreachable on this kernel | **16** | the hard floor: no dialog event, no element handle, no semantic-command surface, no rollout service, no canvas text |
| **OOS** — out of scope by decision (2 **OOS** + 6 **d → OOS**) | **8** | **all 8 were also d** — see point 5 |
| **d → b** — the inheritance is dead, the capability is ours | **2** | C-10 (tabs), C-33 (omnibox classification) |
| partial / mixed | **2** | C-74 (AX half OOS, DOM half **b**), C-71 (two clauses cut, three ordinary) |
| **Total** | **79** | |

**Rows touched by the two 2026-11-13 decisions: 12** — eight filed out-of-scope outright (C-04, C-05, C-43,
C-45, C-46, C-58, C-58a, C-68) and four cut **by clause only** (C-50's `Accessibility` half, C-69's
accessibility clause, C-71's PDF + transcript clauses, C-74's AX half). One further row, **C-44
(transcripts), is cut while keeping class d** — it is not OOS-marked because there was nothing to want.
**Three of the twelve were the sole reason a P0 could not close** (C-58's tier via CTX-01, C-68, C-69).

**Rows whose status changed because of measurement rather than decision: 9** — C-06, C-07, C-09, C-13, C-19,
C-23, C-30, C-31, C-49. Each now carries a number from `../tools/baseline.md` or
`../tools/wv2-probe/RESULTS.md` instead of an inference.

## Where this audit is internally inconsistent — stated rather than smoothed

1. **The 79 is the instructed count, and the claim set behind it was re-derived.** Only the twelve retained
   labels are known to be *the same claims* the original report carried. The other 67 rows are re-derived
   from backlog lines, `bindings.rs` slots, greps and shipped types on 2026-11-13; they are a complete
   sweep of the sentences that assert inherited capability, not a recovered document.
2. **C-92 and C-46 measure different things and could be conflated.** A reachable screenshot call
   (`CapturePreview`, **a**) is not a reachable OCR tier (**d**, cut). They are separate rows on purpose.
3. **L425 is two claims wearing one sentence.** "Cannot access third-party frames" is **c** (script and
   identity exist per frame; OOPIF coverage is *unmeasured*, V17) while "or canvas text" is flat **d**
   (C-47 and C-76). Filed separately on purpose: a reader who collapses them over-claims in one direction
   and under-claims in the other.
4. **D-22's arithmetic is still wrong in the source ruling.** "9 to 6" after deleting tiers 2, 7, 8 and 9
   leaves **5**; D-22 adds coverage reporting as the sixth rung to match the instruction. If that is not
   what was meant, the only candidate to restore is PDF-with-our-own-parser.
5. **"Wanted-not" must not be read as "impossible" — but here the two coincide, and that is worth saying
   exactly.** All eight rows filed out-of-scope were **also d** on this kernel: there is no embedder AX API,
   no PDF content API, no caption API and no OCR surface. So the 60-70 % decision did not give up a
   capability we could have bought with engineering effort; what it gave up is the *right to keep trying* —
   the workarounds (a bundled PDF parser, a caption scraper, a vision model) that a fork-era plan would
   have scheduled. Recording the cuts as scope decisions rather than as kernel findings is still correct,
   because the next contributor with a weekend and a crate *can* rebuild PDF extraction; they cannot rebuild
   the AX tree. **C-44 (transcripts) is the one cut with no path at all** and is labelled as such.
6. **Two rows have no text at all** (C-05, C-45) and are counted anyway. The count is therefore honest about
   shape and dishonest about nothing: 79 rows exist, 79 rows have claims, 2 of those claims are described
   rather than quoted.

