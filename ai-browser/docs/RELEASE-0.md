# R0a — work order (build this, nothing more)

bindings.rs line numbers refer to webview2-com-sys-0.39.1 and were grep-derived on 2026-11-13 (the **vtable-slot** line inside `I…_Vtbl`, not the safe-wrapper line).

Kernel fixed: the existing **WebView2 Evergreen runtime** driven by a Rust shell, bound through
`webview2-com-sys 0.39.1` (+ `wry 0.57.2`). No checkout, no fork, no depot_tools. Companions:
`FORK-OR-CDP.md` (capability map), `PROCESS-AND-LANGUAGE.md` (AI executable + IPC boundary),
`../foundation/README.md` (contract crate, 45 tests green), `../tools/baseline.md` (**measured**).

**R0a is a 12-engineer-week plumbing proof. Nothing ships.** It proves one chain end to end:

```
real page → host-minted document identity → one bounded extraction pass → selection
  → out-of-process provider call → answer in a host-owned overlay
  → navigation → stale label, grant revoked, mutation denied
```

Every line item below is machine-checkable by one command (R0a-8); if an assertion cannot run against a live webview, it is not done.

**Module paths (ruled by Carol, 2026-11-13).** R0a lands **in place** in the existing shell crate —
`native-browser/crates/browser/src/ai/…`, package `browser` — which already ships the tab strip,
history, omnibox, downloads and **150** green tests (`cargo test --offline -p browser`, 150 passed /
0 failed, re-verified 2026-11-13) that R0a needs; a parallel `ai-browser/shell` crate would duplicate
them or add a worse build graph. `ai/` is a new module directory, not a new crate. Adopt
`create_shared_environment` from `src/webview2.rs` and the single-`Connection` discipline from
`src/store.rs`. **Exception:** the AI utility stays its own crate (`ai-browser/utility`) because it
must never link `webview2-com`.

## Corrections applied to the previous version of this plan

| # | Correction | Effect on this work order |
|---|---|---|
| C1 | **Accessibility is out of scope** (user decision, 2026-11-13), and separately **there is no embedder accessibility API on the COM surface**: `accessibilit`, `IAccessible`, `uiautomation`, `UiaRaise` → **0 matches** in `webview2-com-sys-0.39.1/src/bindings.rs` (44,394 lines). The CDP ruling that same day permits read-only extraction via `DOMSnapshot.captureSnapshot` only (`CallDevToolsProtocolMethod`, 1897) — the `Accessibility` domain is **not** called, and R0a does not use CDP at all (`FORK-OR-CDP.md` §1b). | R0a-4 has one tier: DOM + rendered visibility + our own `semanticBlockId`. `accessibilityPath` is not in the anchor contract and backlog §4.2's tier-2 step is **deleted** (`BACKLOG-DELTAS.md` D-01), not deferred. Nothing in R0a-4 may be re-wired to CDP; DOMSnapshot is an R0b+ question gated on V16/V18. |
| C2 | **One overlay controller per browser window**, hosting toolbar + all cards + composer + action indicator — never one per annotation. | **Measured** 2026-11-13 (`../tools/baseline.md` §Pricing the overlay, six runs): **+1 renderer, +0 browser processes, ≈ 66 MB** (64.2–68.5) against the 80 MB ceiling — the ≈ 60 MB estimate was ~11 % low. The alternative reading (the overlay shares the shell's `asset://` renderer and costs 0 renderers) is **dead**: it is its own `with_id("overlay")` controller and took its own renderer on every flip. The process-count half of the prediction was exact. A per-card controller would be ≈ +66 MB **each** (ten cards ≈ **+660 MB**, ten extra renderers) — rejected, not optimised, and C5 would not catch it, because it watches browser processes rather than controllers. R0a-5 creates the controller at window creation, not at card creation. |
| C3 | Scroll is **decided** (2026-11-13): host-driven scroll is `ActionLevel::Navigate` and is allowed in Release 0 by `ActionLevel::allowed_in_release_zero()` in the shipped crate. | R0a-7 does not re-open it and does not duplicate the authority; it asserts the shipped gate is the only authority. |
| C4 | Identity is **registry-minted**, not `NavigationId`. `navigation_sequence` is the authority; `upstream_navigation_id` is audit correlation only (`../foundation/src/identity.rs`, `../tools/README.md`). | **Confirmed by measurement**, not assumed: `wv2-probe` Headline 1 (runtime 157.0.4341.57) found `NavigationId` is one counter per *browser process*, shared with subframes, absent for `pushState`/fragment, and non-dense (one failed navigation consumed 3 ids, 7 ids spent with no event delivered). Promoting it would have produced a per-process sequence counting invisible retries. R0a-2's assertion is about the registry's token and adds no identity code. |
| C5 | **The host-owned overlay already ships.** | See §1 of `FORK-OR-CDP.md` and R0a-5 below: `app.rs:97` `overlay: Option<WebView>` from the shared environment, `layout::Overlay::Dropdown { x, y, w }` (layout.rs:32) repositioned at `app.rs:649`, `asset://` handler at `app.rs:2359`. **R0a-5 creates no new surface**, and §2.1's risk is **measured** (`wv2-probe` Headline 5): the card-sized overlay (`app.rs:2388`, `build_as_child` + `.with_transparent(true)`) passes clicks in its transparent region **by geometry**, while a full-bleed transparent overlay never does — the window rect is the hit region, alpha is irrelevant, `WS_EX_TRANSPARENT` does not change it, and `WS_EX_LAYERED` is refused on a child window; only `SetWindowRgn` clipped to the card is a workable full-bleed shape. Delivered `mousedown` is still **NOT MEASURED** (no synthetic input on this desktop), so this is a `WindowFromPoint` result plus a geometry argument, not a click-through test. |
| C6 | **No new `frame::Method` for selection or answer artifacts** (ruled 2026-11-13): they belong in **params**. Question + target `DocumentToken`s + optional selection → `model.complete` params; answer + citations → `task.finish` params; tokens → `task.progress`. The closed 7-method list and its `Peer` side-gates are untouched. | R0a-6 is **not blocked on a method decision.** What is open is **two payload types in `../foundation/src/contract.rs`** — a *task request* and an *answer-with-citations* — which Bob must add (gap 1). Verified absent: `contract.rs` ships `ContextSnapshot`, `ContextRef`, `ConversationBinding`, `TaskProgress`/`TaskStage`, `ActionRequest`, and nothing naming a model request or an answer. |

Format per item, exactly four fields: **Module / WebView2 call / The one assertion / Defers to R0b.**

## R0a-1 — Shell boots one page from one shared environment

1. **Module** — **no new surface.** Existing `native-browser/crates/browser/src/main.rs`, `app.rs`
   (window + message loop), `tabs.rs`, and `webview2.rs::create_shared_environment`, which **already**
   owns the single `ICoreWebView2Environment`. R0a-1 adds only `src/ai/mod.rs`, which re-exports it and
   counts constructions.
2. **WebView2 call** — `CreateCoreWebView2EnvironmentWithOptions` (free function, line 29) called
   **exactly once per process**; controllers created through `wry`'s
   `WebViewBuilderExtWindows::with_environment` so every webview reuses it
   (`WebViewExtWindows::{controller(), environment()}`, `wry-0.57.2/src/lib.rs:2345/2348`). Bounds via
   `ICoreWebView2Controller::SetBounds` (9321).
3. **The one assertion** — **measured green 2026-11-13** (`../tools/baseline.md` mode D, `RESULT 0 of 8
   checks failed`): +4 tabs added **exactly +4 renderers and +0 `msedgewebview2` browser processes**
   (C2/C3) and the overlay added **exactly +1 renderer and +0 browser processes** (C5) — +1 browser
   process ≈ 150 MB would mean the overlay silently got its own environment. One shell-spawned utility
   PID; exactly **one** `ICoreWebView2Environment` constructed for the process lifetime (C1). The gate
   command is the mode-D form in §R0a exit gate, **never** bare `--proof`: in shipping mode C3/C5/C6 are
   structurally `NOT MEASURED`, so the counts this item asserts have nothing to vary.
4. **Defers to R0b** — tab strip changes, restore, profiles/`SetProfileName` (10458) beyond the default, workspace partitioning, `--inprivate` (`SetIsInPrivateModeEnabled`, 10466).

## R0a-2 — Document identity is minted by the host, and provably not forgeable

1. **Module** — `native-browser/crates/browser/src/ai/identity.rs` (wraps
   `ai_platform_contract::identity::DocumentIdentityRegistry`, owns `navigation_sequence`, owns the
   grant epoch), `identity_audit.rs` (records `upstream_navigation_id` per event for correlation).
2. **WebView2 call** — subscribe to `add_NavigationStarting` (1789) / `add_ContentLoading` (1796) /
   `add_DOMContentLoaded` (`ICoreWebView2_2`, 41482) / `add_NavigationCompleted` (1817) and read `NavigationId` off the **base** args
   interfaces: `ICoreWebView2NavigationStartingEventArgs` (24075), `…ContentLoadingEventArgs` (7450),
   `…DOMContentLoadedEventArgs` (12513), `…NavigationCompletedEventArgs` (23739); frames via
   `ICoreWebView2_4::add_FrameCreated` (43310) + `ICoreWebView2Frame5::FrameId` (20265).
3. **The one assertion** — the scripted walk in `../tools/README.md` (100 navigations: cross-origin
   redirect chain, reload, back/forward, `target=_blank`, 20 × `history.pushState`, 20 × fragment
   change): **one and the same `DocumentKey` is resolved from all four events of every committed
   navigation, the registry's token changes on document replacement only, and never on
   pushState/fragment** — plus `grep` proves no `DocumentKey` is constructed outside the registry
   (the crate's `compile_fail` doctest already blocks deserialisation). **The four live-webview
   questions are answered, so they are no longer part of this item** (`wv2-probe` Headline 1, runtime
   157.0.4341.57): `NavigationId` is one counter per *browser process*, never reused, absent for
   `pushState`/fragment, and **non-dense** — one failed navigation consumed 3 ids, 7 went out with no
   event delivered — therefore `navigation_sequence` stays the authority and this item adds no code.
4. **Defers to R0b** — element-level handles (R0a mints none — the invariant is that no such type
   exists in the binary), per-frame identity nesting beyond `frameId` recording, multi-document
   context, and any change to the crate's identity model — ruled out by the measurement above.

## R0a-3 — Navigation invalidates everything downstream

1. **Module** — `native-browser/crates/browser/src/ai/policy.rs` (grant + epoch store), `context.rs` (snapshot
   invalidation), `utility.rs` (in-flight cancellation).
2. **WebView2 call** — the epoch bumps on `add_ContentLoading` (the earliest commit signal we have);
   in-flight work is cancelled with `task.cancel` over stdio and, if wedged, `supervise::abort()`. Page-side
   listeners die with the document, so this item adds **no new** WebView2 surface.
3. **The one assertion** — script "navigate while the provider is mid-stream": the open card reaches
   the `stale` state within **100 ms** of `add_ContentLoading`, exactly **one** epoch bump is
   recorded, **zero** `task.progress` frames carrying the old `DocumentToken` reach the overlay, and
   after 50 × that cycle the process working set returns to within 5 % of its pre-cycle value (no
   leaked grant, task, or listener).
4. **Defers to R0b** — back/forward-cache subtleties (`pageshow`/persisted), stale-while-revalidate
   of cached snapshots, multi-tab invalidation fan-out, and the "outdated card is still readable"
   polish pass.

## R0a-4 — Exactly one extraction pass, bounded and redacted

1. **Module** — `native-browser/crates/browser/src/ai/context.rs` (coordinator), `probe.rs` (install/invoke/verify),
   `assets/probe.js`, `redact.rs` (host-side pass), `snapshot_store.rs` (copy the
   single-`rusqlite::Connection` discipline from `native-browser/crates/browser/src/store.rs`).
2. **WebView2 call** — install with `ICoreWebView2::AddScriptToExecuteOnDocumentCreated` (1859);
   invoke with `ExecuteScript` (1870) or `ExecuteScriptWithResult` (`ICoreWebView2_21`, 41822); return data with
   `PostWebMessageAsJson` (1882) and read it in `add_WebMessageReceived` (1890); per-frame with
   `ICoreWebView2Frame2::{add_NavigationStarting 19668, add_ContentLoading 19675, ExecuteScript 19696,
   PostWebMessageAsJson 19701}`. **Main world only — wry/WebView2 0.39.1 exposes no isolated world**,
   so probe output is untrusted input, not truth.
3. **The one assertion** — over 6 golden pages + 1 adversarial + 1 authenticated: every snapshot is
   ≤ 2 MiB and ≤ 150 ms p95 (article) / ≤ 400 ms p95 (5 MB SPA), **zero** keys matching
   `password|cookie|token|authorization|secret` in the serialised payload after the host redaction
   pass, and a page that deletes/shadows the probe yields typed `probe-blocked` coverage — never an
   empty success.
4. **Defers to R0b** — tables/images/structured-data extractors, snapshot deltas (recapture-on-ask is
   the R0a behaviour), below-fold chunking, `PostSharedBufferToScript` (`ICoreWebView2_17`, 41009) if the message-size
   ceiling (V14) forces it, and any second extraction tier (there is no AX tier — C1).

## R0a-5 — One privileged overlay controller per window

1. **Module** — **reuse the existing overlay controller** (C5), not a new one. New files:
   `native-browser/crates/browser/src/ai/overlay.rs` (transparent backing + card slots inside that one
   surface) and `ai/anchor.rs` (quote + prefix/suffix + rect reattachment, confidence score).
2. **WebView2 call** — reuse what the palette already drives: `ICoreWebView2Controller::SetBounds`
   (9321), `MoveFocus` (9341) / `add_MoveFocusRequested` (9345), and the existing message bridge
   (`window.ipc.postMessage` → `window.chrome.webview.postMessage`, wry `webview2/mod.rs:968` →
   `add_WebMessageReceived` 1890) — so **`AddHostObjectToScript` (1939) stays unused**; UI assets come
   from the hardened `asset://` handler in `ui.rs`, not a new origin. New in R0a:
   `ICoreWebView2Controller2::put_DefaultBackgroundColor` (slot `SetDefaultBackgroundColor`, 9910) at **A = 0** (MS docs: alpha may be
   only `0` or `255`; at A = 0 "WebView will render hosting app content as the background"),
   `SetBoundsAndZoomFactor` (9336) to keep card pixels stable under page zoom, and
   `SetAreDefaultContextMenusEnabled` (35551) + `SetAreDevToolsEnabled` (35543) on the UI surface.
3. **The one assertion** — against a hostile page (`z-index: 2^31`, ancestor `transform`,
   full-viewport capture layer, `MutationObserver` node-deleter, forged look-alike card, focus-steal
   loop) the card is provably untouchable **and** the cost is bounded: opening the **10th** card adds
   **0** processes and **< 5 MB**, a transparent-region click reaches the page (C5 — measured for the
   card-sized shape; delivered `mousedown` still needs an interactive desktop), and re-anchoring after
   a scripted scroll settles is ≤ 200 ms p95 with **zero** `SetBounds` calls during the fling (the
   card pins — C2/`FORK-OR-CDP.md` §3.2). **Kill criterion, restated after that measurement:** the
   question is not whether A = 0 hit-tests (full-bleed never will) but whether the card can stay
   card-sized; if it cannot, drop the model for an assistant pane + in-page highlight, designed first.
4. **Defers to R0b** — connector/leader line art (decorative art may live in the page; the card may
   not), composer UX, toolbar, action indicator states, DPI/per-monitor edge cases,
   `add_NewWindowRequested` (1921) z-order juggling, and `EnableWindow(controllerHwnd, FALSE)` modality
   — the substitute for `ICoreWebView2Controller::put_IsEnabled`, **which does not exist** (`IsVisible`
   9309 / `SetIsVisible` 9313; `SetIsEnabled` 7748 is `ICoreWebView2ContextMenuItem`'s).

## R0a-6 — One provider in a separate executable, answering one selection question inline

1. **Module** — utility: `ai-browser/utility/src/main.rs`, `providers/mod.rs`,
   `providers/replay.rs`, `providers/winhttp.rs`, `router.rs`; shell: `native-browser/crates/browser/src/ai/utility.rs`
   (spawn via `foundation::supervise::spawn`), `selection.rs`, `thread.rs`.
2. **WebView2 call** — **none, by design**: the utility must not link `webview2-com` at all (asserted
   by the dependency check). Its only webview-adjacent touch is that the shell stamps the request with
   the host's `DocumentToken`. Selection reaches the shell through the probe's
   `PostWebMessageAsJson` (1882); the answer streams to the overlay through
   `PostWebMessageAsJson` (1882) / `PostWebMessageAsJsonWithAdditionalObjects` (`ICoreWebView2_23`, 42136).
   Transport is **decided**: NDJSON over piped stdin/stdout (`foundation/README.md`). Wire shape is
   **ruled** (C6): the question, the target `DocumentToken`s and the optional selection ride as the params
   of `model.complete`; the answer and its citations ride as `task.finish` params, tokens on `task.progress`.
3. **The one assertion** — select a passage, ask one question, answer streams into the card, and the
   turn's stored record carries `{navigation_sequence, frameId, snapshot_version, exact_quote}` with
   the quote byte-matching the selection — **while `Get-ExtendedTcpTable`/handle inspection proves the
   PID holding the provider socket is not the shell PID**, and stop-button → last token is ≤ 100 ms.
   Determinism: `providers/replay.rs` (fixture stream) is the default in CI so the assertion never
   depends on the network.
4. **Defers to R0b** — a second live provider, routing/quotas, prompt versioning, the second
   executable instance for local inference, approval-gated `Interact`, and answer formatting beyond
   plain text + links. **Waiting on payload types, not on a method (C6):** R0a-6 needs Bob's *task request*
   and *answer-with-citations* types in `contract.rs` before it type-checks; no method decision is pending.

## R0a-7 — Deny-all on page mutation

1. **Module** — `native-browser/crates/browser/src/ai/policy.rs` (the allowlist is data) and `audit.rs`. The authority
   **is** `ActionLevel::allowed_in_release_zero()` in `ai-platform-contract`; the shell must not
   duplicate the classification (C3).
2. **WebView2 call** — the capability exists and is deliberately fenced: `ExecuteScript` (1870) and
   `ICoreWebView2::Navigate` (1781) may be called from **one** module (`dispatch.rs`, compiled with only
   allowlisted read paths in R0a). `AddHostObjectToScript` (1939) is **not** used at all.
   Defence-in-depth for page traffic (not the provider socket — see `PROCESS-AND-LANGUAGE.md` §2b):
   `AddWebResourceRequestedFilter` (1969) / `ICoreWebView2_22::…WithRequestSourceKinds` (41968).
3. **The one assertion** — a test fires every `ActionLevel` variant and every denied method name:
   only `Observe` and the allowlisted `Navigate`-class scroll return `allowed`; `Interact` and
   `Commit` return `blocked` **plus** an audit record, and a source-level test proves no synthetic
   input primitive (`SendInput`, `mouse_event`, `keybd_event`, `SendMouseInput`-style shims) appears
   anywhere in the R0a tree.
4. **Defers to R0b** — the whole of `Interact`/`Commit`, the approval modal + its `EnableWindow`
   modality, action batching, per-site action allowlists, and post-stale action revocation (nothing
   mutable exists in R0a to revoke).

## R0a-8 — The harness *is* the definition of done

1. **Module** — **shipped** as `native-browser/crates/browser/src/ai/proof.rs` + `ai/mode.rs` + `ai/processes.rs`, **not** the `harness.rs` / `harness/golden/*.html` / `harness/budgets.json` named here originally: no golden corpus and no budgets file exist on disk (gap 7).
2. **WebView2 call** — the harness drives the same calls as items 1–7 and adds no API of its own. It is a real `--proof` mode on the existing `browser` binary, not a doc.
3. **The one assertion** — **met for the checks that exist**: `cargo run -p browser -- --proof=+no_tab_sleep+no_overlay` prints one `PASS` / `FAIL` / `NOT MEASURED` line per check **C0…C7** with the measured number beside it, exits **1** on any FAIL and **2** on a mistyped flag, twice back-to-back with identical verdicts. `NOT MEASURED` is never green (`ai/proof.rs`), which is exactly why the gate command carries two flags (§exit gate).
4. **Defers to R0b** — CI scheduling, dashboards, per-runtime-version regression tracking across Evergreen updates, corpus growth beyond 8 pages, and the instruments for the six timing budgets + utility RSS in §Budgets, none of which exist yet (gap 7).

## R0a exit gate

**The gate command is `cargo run -p browser -- --proof=+no_tab_sleep+no_overlay` — not bare `--proof`.** In shipping behaviour (mode A) the shell sleeps background tabs and builds the overlay at startup, so checks **C3/C5/C6** have nothing to vary and print `NOT MEASURED`; a gate written as "the default command goes green" could therefore never be met. That was a doc bug in this file, not a code bug, and it is fixed here. **Achieved 2026-11-13:** the mode-D command ran green (`RESULT 0 of 8 checks failed`, exit 0) on a live webview, and so did every other mode — 14 harness runs across the four modes, 0 failures (`../tools/baseline.md`). R0b does not start while any of R0a-1…R0a-5 is red. **Coverage caveat:** those eight checks are the R0a-1 process/identity half plus the two cost deltas — the six timing budgets have no instrument (gap 7), so "the gate is green" does not yet mean "R0a-3…R0a-7 were measured".

## What R0a explicitly does NOT do

Multi-tab context · split panes · tab strip UX · citations-opening-panes · tables/images/structured data · snapshot deltas · model routing · a second live provider · local inference · any page mutation · any approval UI · element handles · any accessibility tier (out of scope by decision + no COM API, C1) · schema publication · the overlay's composer/toolbar art · dashboards · remote config/rollout.

## Budgets (the two memory rows are **measured** — `../tools/baseline.md`, 2026-11-13, dev profile; the other seven have no instrument yet, gap 7. A release build is a different baseline and is **not** measured)

| Budget | Target | Status | Measured by |
|---|---|---|--:|
| Selection → answer starts streaming | ≤ 50 ms p95 shell-side (replay provider) | no instrument yet | R0a-6 |
| Card re-anchor after settle | ≤ 200 ms p95, **0** `SetBounds` during fling | no instrument yet; and `set_bounds` is invisible to the overlay document (`wv2-probe` Headline 3), so the instrument must be screen-side | R0a-5 |
| Snapshot capture | ≤ 150 ms p95 article / ≤ 400 ms p95 SPA, ≤ 2 MiB | no instrument yet | R0a-4 |
| Cancel → last token | ≤ 100 ms p95 | no instrument yet | R0a-6 |
| Navigate → card `stale` | ≤ 100 ms | no instrument yet | R0a-3 |
| Shell UI-thread stall in the AI path | none > 50 ms | no instrument yet (no watchdog) | R0a-8 |
| Overlay, per window | +1 renderer, +0 browser processes, **≈ 66 MB** (ceiling 80 MB), **+0 per card** | **measured**: 64.2 / 65.2 / 65.7 / 66.9 / 67.0 / 68.5 MB over six runs; the old ≈ 60 MB estimate was ~11 % low and the "shares the shell's `asset://` renderer, 0 renderers" reading is **dead** — the overlay is its own controller and took its own renderer every time | R0a-1, R0a-5 (C5) |
| Background tab kept alive (`sleep_background_tabs` off) | **no budget — this row is the ask.** Measured **floor**: +1 renderer, **+97.0–98.2 MB** per tab (σ ≈ 0.4) on example.com/.org/.net and iana.org, i.e. some of the cheapest pages on the internet; the shell's own note in `app.rs` says ≈ 120 MB on real content ⇒ 8 tabs ≈ +780 MB, 20 tabs ≈ +1.95 GB on top of a ~590 MB five-tab tree | **measured** (C6). Shipping R0a sleeps background tabs, so this is the price of *not* sleeping them — set the ceiling before any keep-alive feature is proposed | R0a-1 (C6) |
| Utility RSS | ≤ 150 MB (breach = kill + `unavailable`) | no instrument yet | R0a-6 |

## Known gaps that block or weaken R0a (not hidden anywhere else)

1. **Two payload types are missing from `contract.rs`** (ruled in C6 — this is *not* a missing method): a
   *task request* (question + target `DocumentToken`s + optional selection + `snapshot_version`) and an
   *answer-with-citations*. R0a-6 cannot type-check until Bob adds them; the closed 7-method list stays.
2. **No orphan-proofing.** A shell crash leaves a utility alive with a live provider session. Needs a
   Job Object (`CreateJobObjectW` + `AssignProcessToJobObject` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`)
   at spawn; `windows = "=0.62.2"` is already pinned. Must land before R0a-6 is run by hand twice.
3. **`--offline` vs provider HTTP.** `reqwest`/`hyper`/`rustls` need crates.io; `winhttp` (via the already-pinned `windows` crate) does not — **that is the only reason to prefer it, and it stands.** It buys no proxy inheritance: WinHTTP keeps its **own** store, which may be "direct access" while pages are proxied (§2b). Undecided; forced to `winhttp` unless the extra deps are vendored.
4. **Provider egress is outside the browser's network stack** — enterprise proxy (WinHTTP's own store, *not* the browser's), system TLS policy and per-origin rules are all **not** inherited (`PROCESS-AND-LANGUAGE.md` §2b). Corporate proxy is a deployment requirement with its own failure cause; the kill switch is process-level (`supervise::abort()`), not navigation-layer.
5. **Baseline recorded; the release-build baseline is not.** `../tools/baseline.md` (2026-11-13: four modes, 14 runs, machine record, repro commands) is the table every memory number above is a delta against, and it also fixes the comparability rule that the earlier draft did not state — absolute MB are comparable only from a **wiped profile** *and* the **same mode**, because one session-restored run read +231 MB (+39 %) on identical verdicts; check **C7** now fails the run instead of letting that number through. Two follow-ons: every figure there is `cargo`'s **dev** profile, so a release baseline needs `--release` and its own file; and two strings in `ai/proof.rs` are now stale — C5 prints "against the ≈ 60 MB budget" and C6 "nothing in the plan sets a ceiling on it yet", both superseded by the measured rows in §Budgets. **Disposition (Carol, 2026-11-13): those two strings are not ours to fix.** `ai/proof.rs` has another engineer working in it right now on **C8**, and one file does not get two writers; the exact strings have been routed to them. This doc stays the record of what the strings should say, and §R0a-8's **C0…C7** inventory must be re-checked once C8 lands — the exit-gate line "0 of 8 checks failed" is a count that C8 will change.
6. **`accessibilityPath` still exists in backlog §2.3's `AnnotationAnchor`.** That file is the user's, so the deletion is a **proposed delta** (`BACKLOG-DELTAS.md` D-01) awaiting approval — and it **blocks nothing**, because the code side already matches the decision: `AnnotationAnchor` (`contract.rs:242`) carries `document`, `snapshot_version`, `block_id`, `quote`, `prefix`, `suffix`, `text_position` — **no AX field** — `AnchorReattachmentTier` is `SemanticBlockId | QuoteWithAffixes | NormalizedTextPosition | FuzzyQuoteMatch | Detached`, and `tests/action_levels.rs:43` fails if an AX variant is ever added.
7. **The harness covers one work-order item, not eight.** Checks C0…C7 measure R0a-1 (attribution, environment count, renderer/browser-process counts) and the two cost deltas (C5 overlay, C6 kept-alive tab). Nothing measures snapshot ms, cancel→last-token, re-anchor ms, navigate→`stale` ms, UI-thread stalls or utility RSS, so seven §Budgets rows are aspiration; there is no 6+1+1 golden corpus, no `budgets.json`, and no watchdog. "The gate is green" therefore means *the plumbing is proven*, not *the budgets are met* — R0a-4/5/6 must land their own instruments before §Budgets can be quoted as passed.
8. **Most module paths in this work order are not on disk yet.** `src/ai/` holds `mod.rs`, `policy.rs`, `mode.rs`, `processes.rs`, `proof.rs`; `identity.rs`, `context.rs`, `probe.rs`, `redact.rs`, `snapshot_store.rs`, `overlay.rs`, `anchor.rs`, `utility.rs`, `selection.rs`, `thread.rs`, `audit.rs` and `dispatch.rs` still have to be created. Expected in a work order, but it means the "reuse what ships" instruction currently applies to the action gate (`policy.rs`) and the proof harness only — and `ai-browser/utility` does not exist at all yet.
