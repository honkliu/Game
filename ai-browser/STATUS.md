# AI-Native Browser — program status

Spec: `../ai-native-browser-design-and-engineering-backlog.md` (878 lines, Epics
A–I, 45 stories). This file is the live status list; a story moves to `done` only
after its verification actually ran.

## Kernel decision (locked 2026-11-13)

**Use the existing WebView2 kernel.** No Chromium source checkout, no fork, no
`depot_tools`, no build. The shell to build on is `../native-browser/`
(wry 0.57.2 + tao 0.37.2 over the system WebView2 runtime, already rendering real
sites). The from-scratch Rust engine at `../engine/` is parked, not deleted.

### Voided by that decision

- **FND-05** upstream-sync pipeline, OWNERS rules, patch series — there is no
  upstream to sync. Dropped.
- **All `.mojom` contracts** — Mojo is Chromium-internal and unavailable to an
  embedder. Contracts are now plain IPC over a chosen serialization.
- **"C++ in-tree `services/ai/`"** and its `rust_static_library` re-decision gate —
  there is no tree. The AI utility process is a separate executable beside the
  Rust shell.
- GATE-0a/0b/0c (install depot_tools, 25 GB checkout, baseline build) — not needed.

## Capability surface measured 2026-11-13

Grepped `webview2-com-sys-0.39.1/src/bindings.rs`. **Symbol presence only — none of
these semantics have been confirmed on a running webview yet.**

| API | Line | What it unlocks |
|---|---|---|
| `NavigationId() -> u64` | 7432, 12499, 23717, 24037 | Browser-assigned per-navigation id, on four event-args interfaces. **Audit-only in our identity model — not the authority** (see "Identity authority" below). V1 in the spike decides only whether it can ever be promoted |
| `CreateCoreWebView2ControllerWithOptions` | 14709 | A second host-owned controller = its own HWND → the §2.1 privileged overlay |
| `ICoreWebView2Frame` / `Frame5` / `Frame6` | 19152, 20250, 20320 | Per-iframe handles → §CTX-05 coverage gaps can be per-frame |
| `PostWebMessageAsJson` / `...WithAdditionalObjects` | 1402, 19610, 42113 | Typed host↔page messages, frames included |
| `ICoreWebView2ControllerOptions` | 10406 | Named controller options (popup / user-data isolation) |
| `Environment11` / `Environment12` | 14912, 15003 | Environment + user-data-folder control |
| `Profile5` | 32231 | `ClearBrowsingData` for §8's clear-workspace-memory |
| `AddBrowserExtension` | 32492 | Extension loading (not currently planned) |

**Correction of record:** I initially told the user that §2.3 browser-process
document identity was *not* reachable without a fork. That was wrong —
`NavigationId` is exactly that. Corrected to the user and to Alice.

## Where the browser-process role now lives

The Rust process running `native-browser` **is** the browser process for our
purposes. It owns tab identity, navigation events, workspace membership, policy,
approval, and final action dispatch. The AI utility process owns provider calls,
prompt construction, routing and planning, and has no tab authority.

## Identity authority — my earlier wording was wrong

I wrote "identity from `NavigationId`". Wrong, and both agents caught it before I did.

`foundation/src/identity.rs` (lines 15-35) makes `navigation_sequence` **our own per-tab
counter** the authority and records `upstream_navigation_id` as `Option<..>`, **audit-only**.
The decisive reason needs no measurement: a `history.pushState` route change never reaches a
WebView2 navigation event at all — it is visible only from injected script — and
`SourceChanged`/history navigations carry no `NavigationId`. So `NavigationId` cannot be the
authority for same-document changes whatever its monotonicity. Two further reasons in the
crate: the bindings don't state whether the counter is per-webview or per-browser or whether
back/forward reuses an id, and it has no concept of profile or workspace.

**Consequence for the spike:** V1 no longer blocks identity. It decides only whether the
audit field can ever be promoted. **V2 and V3 are now the real critical path** — they decide
whether the anchored-overlay design is viable at all.

## C5 — the privileged overlay already ships in our own crate

Alice found it while applying the path decision; I verified every line in the source before
accepting it. This is the most consequential finding so far.

| Fact | Where |
|---|---|
| `overlay: Option<WebView>` — **one per app**, not per card | `app.rs:97` |
| Built from the **shared** environment | `app.rs:103` |
| `enum Overlay { Closed, Palette, Dropdown { x, y, w } }`, `Closed` → `0×0` | `layout.rs:29-38` |
| Repositioned to an **arbitrary host-supplied rect** via `set_bounds` — the omnibox dropdown | `app.rs:649` |
| Focus returned to the page on close | `app.rs:659` |
| UI assets served by an audited `asset://` handler (traversal, percent-escape, ADS, 8.3-short-name each tested) | `app.rs:2359`, `2398` |
| Process-tree + working-set instrumentation already exists — `process_tree()`, `snapshot_processes()`, `working_set_bytes()`, `mb()` | `sys.rs:53`, `75`, `114`, `213` |

**Why this matters more than the saved work:** backlog §2.1's host-owned overlay was the single
biggest architecture risk in the program, and it is not merely *available* on this kernel — it
runs today and already anchors to an arbitrary rect. The unknown shrinks to one thing: **the
shipping overlay is opaque and covers content, so per-pixel transparency with correct
hit-testing in transparent regions has never been exercised.** That is V3, and it is now the
whole of the §2.1 risk. `SetVirtualHostNameToFolderMapping` drops out of the plan in favour of
the existing audited `asset://` handler.

I also had to retract part of my own brief: I told an engineer to write Toolhelp32 enumeration,
which `sys.rs` has shipped for a while. Redirected to reuse it. The genuine remaining gap there
is the **browser-vs-renderer split** — whether `ProcessEntry` carries a command line so
`--type=renderer` can be discriminated. Counting every `msedgewebview2.exe` as a browser process
is the bug this check exists to catch.

## Params, not methods — ruling on the "missing selection/answer method" gap

R0a-6's shape is settled: the question, target `DocumentKey`s and optional selection ride as
**params of `model.complete`**; the answer and its citations ride as **params of `task.finish`**,
with streamed tokens on `task.progress`. No `frame::Method` is added. The open work is two
payload **types** in `contract.rs` — a task request and an answer-with-citations.

## Gap list for the contract crate — corrected after checking the code

Alice reported "no `frame::Method` carries selection or answer artifacts". The symptom is
real, the diagnosis is off by one layer, and acting on it as written would churn 45 green
tests:

- `frame.rs` `Method` has 7 variants and, correctly, **none needs adding**. In a
  request/response protocol the selection and the answer ride in **params**, not in method
  names. The shell→utility direction already has `model.complete`, which is exactly where a
  task belongs.
- The actual gap is in `contract.rs`: there is no **task request** payload (question +
  target `DocumentKey`s + optional selection) for `model.complete` to carry, and no
  **answer-with-citations** payload for `FinishTask`/`TaskProgress` to carry. `ContextRef`,
  `ConversationBinding`, `TaskProgress`, `AnnotationAnchor` and `ActionRequest` all exist;
  those two do not.

**`accessibilityPath` does not exist in our code.** Verified directly: `AnnotationAnchor`
(`contract.rs:242`) is `{document, snapshot_version, block_id, quote, prefix, suffix,
text_position}` — no AX path, so nothing to remove. The only exposure is backlog §2.3's spec
text. Code side is clean; the spec file is the open item.

## WinHTTP proxy — third fork-era inheritance claim, same direction

`PROCESS-AND-LANGUAGE.md` §2b said provider egress "inherits the system proxy for free" via
`winhttp`. It does not. **WinHTTP keeps its own proxy configuration**, a separate store from
the user's WinINet settings at `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet
Settings`; a fresh machine defaults WinHTTP to **direct access**, and
`netsh winhttp import proxy source=ie` exists precisely because the two do not track each
other. Enterprises usually push WinHTTP's setting by GPO — not automatically, and not the
browser's value.

Verification honesty: `netsh winhttp show proxy` here reports `Direct access (no proxy
server)` and `ProxyEnable` is `0x0`, so **both are direct on this box and I could not
demonstrate divergence empirically** — no proxy to test against. The claim rests on API
design plus that `netsh` import command. Fix the doc; keep the `winhttp`-over-`reqwest`
recommendation for the `--offline` reason.

This is the third time a fork-era "we inherit it from Chromium" claim survived into the
WebView2 plan (network service, accessibility tier, proxy). **Audit the docs for every
remaining "inherit" before anyone builds against them** — grepped 2026-11-13, clean apart from
this one.

## R0a module paths — decided (Carol)

`native-browser/crates/browser/src/ai/…`, **not** a new `ai-browser/shell` crate. That crate
already owns the tab strip, history, omnibox, downloads and 118 green tests, all of which R0a
needs; a parallel shell would duplicate them or grow a worse build graph. The AI utility stays
a separate executable crate because it must never link `webview2-com`.

**Verified while writing that brief:** the single-shared-environment plumbing R0a-1 asks for
**already exists** — `src/webview2.rs` calls `CreateCoreWebView2EnvironmentWithOptions` once
and every builder shares it via `WebViewBuilderExtWindows::with_environment`
(`app.rs:1753`, `2366`, `2405`); `webview2.rs:21` documents that as the motivation. So R0a-1's
remaining work is the **assertion and the baseline**, not the plumbing, and no `baseline.md`
exists yet — every memory budget in the plan is currently an absolute target with nothing to
subtract from.

## Current work

| ID | Pri | Story | Status | Owner | Progress |
|---|---|---|---|---|---|
| FND-01 | P0 | Identity model (`DocumentKey` carrying `NavigationId`, `navigationSequence`, `WorkspaceId`, `TabId`, `SnapshotVersion`, `TaskId`) + ownership rules | **done — verified** | Bob | `foundation/src/identity.rs`; 15 tests I ran green |
| FND-02 | P0 | Versioned context/action/progress/cancel/error schemas + incompatible-major rejection test | **done — verified** | Bob | `contract.rs` + `version.rs`; 5 version tests green |
| FND-03 | P0 | Process boundary: shell ↔ AI utility | **done at contract level** | Bob+Alice | `frame.rs` codec + `Peer` side-gate, `supervise.rs` spawn/abort. Not yet wired to a live provider child |
| FND-04 | P0 | Typed failure taxonomy (7 variants) | **done — verified** | Bob | `failure.rs`: 7 wire variants + `Unknown` as a deserialisation sink, so a new variant is a minor bump, not a parse failure |
| — | P0 | Choose the one IPC transport, incl. cancellation + kill of a wedged utility process | **done** | Bob | NDJSON over the child's stdin/stdout; no loopback listener. `abort()` = drop write half → EOF → 500 ms grace → kill + reap, tested against a real process |
| SPK | P0 | **V4 CDP bridge** — is `CallDevToolsProtocolMethod` callable from an embedder | **done — measured** | Bob | `DOMSnapshot.captureSnapshot` → S_OK, 2.2 ms, 5 809 B, 35 nodes, 23 layout rects, **DevTools never opened, no extra browser arg**. V16 passes for callability |
| SPK | P0 | **V2/V3** — painted-rect latency, transparent-region hit-testing | **BLOCKED — instrument limit, not code** | Bob | Cannot be measured on this box: GDI error 5 blocks synthetic input, GDI error 6 blocks screen BitBlt. Needs a physical console session. See "V2/V3 are unmeasurable here" |
| SHL | P0 | R0a-1 `--proof` harness + `tools/baseline.md` | **done — 8/8 green, run by me** | Eng. 2 | `--proof=+no_tab_sleep+no_overlay` → `RESULT 0 of 8 checks failed`, exit 0. Profile wipe now enforced by `wipe_proof_profile()` + check C7 |
| SHL | P0 | R0a-7 deny-all gate in `crates/browser/src/ai/policy.rs` | **done — verified** | Eng. 2 | `ai-platform-contract` is a path dep of the shell. Unknown action kinds deny via `AuditRecord::Unclassified` because `ActionDecision.level` is not optional |
| — | P0 | Fix `--driver=NAME`, which never ran | **done — verified in source** | Eng. 2 | Off-by-one wake-up skipped step 0 and wedged the process (`app.rs:759-764`). **Every driver-derived number predating this is suspect** |
| DOC | P0 | `docs/FORK-OR-CDP.md` | **done** | Alice | 408 lines; §2.1 reframed as "it already ships"; CDP narrowed to one method; AX removed from the loss column |
| DOC | P0 | `docs/PROCESS-AND-LANGUAGE.md` | **done** | Alice | 294 lines; §2b records what is *not* inherited; proxy divergence = verify L6 |
| DOC | P0 | `docs/RELEASE-0.md` | **done** | Alice | 236 lines; measured numbers folded in. **Doc bug open: the exit-gate command must be `--proof=+no_tab_sleep+no_overlay`, not bare `--proof`** |
| DOC | P0 | `docs/BACKLOG-DELTAS.md` — 21 deltas, 7 blocked P0 stories | **awaiting user approval** | Alice | 749 lines. Nothing applied to the backlog until the user approves delta ids |
| DOC | P0 | `docs/CAPABILITY-CLAIM-AUDIT.md` — the 79-claim audit | **failed once, re-tasked** | Alice | Agent closed mid-write with the file absent. Re-tasked with "write only this file". Its absence already cost precision twice |
| JOB | P0 | Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` | open — queued for Bob | Bob | Crash path orphans an AI child holding a live provider session |
| CTX | P0 | Two payload **types** in `contract.rs`: *task request* and *answer-with-citations* | open — queued for Bob | Bob | R0a-6's only remaining prerequisite |
| ISO | P0 | **Workspace is not an isolation boundary** | **open decision for the user** | — | One browser process per user-data-folder, confirmed by two-host probe. §8 must not be frozen as stronger than application-layer |
| SPEC | P0 | Backlog `accessibilityPath` + §4.2 tier 2 | **resolved by scope decision** | Alice | Accessibility dropped entirely; covered by deltas D-01/D-20/D-21 rather than a separate edit |

## Measured — estimates the plan was wrong about

Run by me on 2026-11-13: `cargo test --offline -p browser` → **150 passed, 0 failed**, no
warnings; `--proof=+no_tab_sleep+no_overlay` → **`RESULT 0 of 8 checks failed`, exit 0**.
Foundation still **45/45**, untouched.

| Quantity | Plan's estimate | Measured | Note |
|---|---|---|---|
| Overlay controller | ≈60 MB, maybe shares the shell's renderer | **≈66 MB, own renderer** (+1 renderer, +0 browser procs) | 64.2–68.5 MB over six runs. The "shares the renderer, costs 0" reading is **dead** |
| Per renderer | ≈60 MB | **≈73.6 MB** | ~22% low |
| Kept-alive background tab | **no budget line existed** | **≈97.9 MB** — a *floor* | Measured on example.com/iana.org; shell's own notes say ~120 MB on real content. 8 tabs ≈ +780 MB, 20 ≈ +1.95 GB |
| `+N tabs → +N renderers` | asserted | **holds, but only with sleep off** | `sleep_background_tabs` destroys non-active webviews, so shipping mode has 1 live page for N tabs |

**Gate-command bug — CLOSED, and the stale note was mine.** In shipping mode C3/C5/C6 are
*structurally* NOT MEASURED, so a gate written as "the default `--proof` goes green" can never be
met; the gate command is `--proof=+no_tab_sleep+no_overlay`. I had this logged as still open in
`RELEASE-0.md`, and re-checking the file says otherwise: §exit gate line 203 states the mode-D form
explicitly, line 58 warns "**never** bare `--proof`", and item 2 of R0a-8 line 198 gives the same
command. All three are correct; `RELEASE-0.md` needed no edit. The stale claim was in this file —
the usual failure mode of a status list that records a finding and never re-reads it.

**Reproducibility is now enforced, not documented:** `wipe_proof_profile()` deletes the harness
profile before sampling, and check **C7** fails if step 0 isn't 1 tab / 1 live webview. My own
earlier run skipped the wipe and got `9 → 13` tabs and 822.6 MB instead of `1 → 5` / 591.6 MB
with *identical verdicts* — which is why absolute whole-tree MB are comparable only from a wiped
profile, while per-role working set and intra-run deltas are portable.

## V2/V3 are unmeasurable on this machine — and the risk is smaller than I said

Painted-rect latency and transparent-region hit-testing **cannot be measured here**: GDI error 5
blocks synthetic input, GDI error 6 blocks screen BitBlt. Not a code problem — an instrument
limit, needing a physical console session rather than remote desktop.

I had been calling V3 "the whole §2.1 risk". Having read the shipped overlay, **I overstated
it.** The overlay is already positioned to a rect (`Overlay::Dropdown { x, y, w }`), not a
full-window transparent sheet, so transparent-region hit-testing only arises when **two cards are
open with a gap between them** — one controller cannot be non-rectangular. That reduces a
viability question to a layout question with cheap answers (union box, stack the cards, or a
second controller for the rare second card). **Recommendation: design the risk away rather than
build a measurement rig for it.**

## Scope decisions (user, 2026-11-13)

1. **Accessibility is out of scope — dropped, not deferred.** AX-tree extraction, the
   `accessibilityPath` anchor tier, and the AX target locator are all deleted.
   **The call cost almost nothing, and now we have the number:** the AX tree returned 38 nodes
   with **zero carrying bounds** — names only. Positional use needs a join onto `DOMSnapshot`
   `backendNodeId`, which covers only 18 of 38 because `InlineTextBox` fragments have no key,
   while `DOMSnapshot` gives text *and* geometry in one call.
2. **Keyboard navigation, focus handling and IME for our own chrome stay in scope** — the user
   chose this explicitly. Majority-user input quality, not an accessibility feature.
3. **§4.2 tiers 7 (PDF), 8 (transcripts/captions), 9 (OCR) cut**, leaving 6 tiers. Required
   behaviour is the existing `CoverageGap`/`CoverageStatus`: say plainly that the PDF/video/canvas
   can't be read. Tier 8 had no embedder path anyway; 7 and 9 are wanted-not, not impossible.

## CDP bridge — measured, and it works

`DOMSnapshot.captureSnapshot` → S_OK in the completion, **2.2 ms**, 5 809 B, 35 DOM nodes, 23
layout rects with text and geometry, on a normal https origin with **DevTools never opened**
(`AreDevToolsEnabled` reads back `false`) and **no additional browser argument**. V16 passes for
callability. Three traps for whoever writes the analyzer:
1. **Sync `S_OK` is not an ack.** An invented method returns S_OK synchronously and fails only in
   the completion (`0x80070057`). Branch on the completion result only.
2. **This runtime returns the legacy columnar payload** — parallel arrays under
   `documents[0].nodes`, strings in a shared table. An analyzer written against the modern
   `nodeTreeSnapshot` docs prints **0 nodes against a payload holding 35**.
3. **`textBoxes.start`/`length` are UTF-16 code-unit offsets** — never count in `char`s.

## Release 0 sizing — corrected 2026-11-13

Alice's arithmetic on the backlog's own size midpoints: total P0 ≈ 73 engineer-weeks,
and the Release-0 set **as I originally scoped it ≈ 71 — i.e. 97% of all P0** — while
backlog §9 calls Release 0 a "technical proof". Those cannot both be true. My earlier
scoping was wrong.

Adopting the split:

- **R0a — proof of plumbing, ≈ 12 eng-weeks.** Host-minted identity (`navigation_sequence`
  authoritative, `NavigationId` audit-only), one
  extraction pass, one provider, one selection question answered inline, navigation
  invalidation, deny-all on mutations. Nothing ships; it proves the loop closes.
- **R0b — the rest of §9, ≈ 24 eng-weeks.** Overlay polish, provider abstraction
  across two providers, model selection, coverage reporting.
- **Backlog §12 is Release 1's definition of done, not Release 0's.** Four of its 14
  conditions map to §9 Release-1 bullets or P1 stories (`SHL-04`, `ANN-05`, `WRK-01`,
  `QAL-04`), and two more are undemonstrable in R0 because R0 forbids mutations.
  Scoring R0 against §12 would fail it on day one for unactionable reasons.

Cut list under review: `SHL-02` omnibox 4.5→0 (reusing our own `omnibox.rs` instead),
`CTX-04` snapshot deltas 4.5→0, `SAF-02` approval-class matrix 4.5→0 (R0 has no
mutations to gate), `SHL-01` 4.5→3.

## Language decision — resolved 2026-11-13 (ratified by Carol)

**Rust.** Alice's original "C++ in-tree `services/ai/`" recommendation was void (no
tree). Bob then decided it by construction, and the reasoning holds: the shell and the
AI utility process are both Rust, so the IPC boundary compiles against **one set of
`struct`s** — a disagreement about a message shape becomes a compile error, not a
parse surprise. protobuf/capnp were rejected for adding a codegen toolchain and two
generated types that merely *agree*, to buy compactness this channel does not need.

**Honest cost, recorded by Bob and accepted:** JSON is runtime-typed, so the wire
format carries **no** authority guarantee. Authority rests instead on two things the
format cannot weaken — `DocumentKey` has a private constructor and no `Deserialize`
(proved by a `compile_fail` doctest), and `frame::decode` takes the `Peer` fixed when
the pipe was brokered and refuses any method that side may not issue **before** the
payload is interpreted. Mojo gave this free via interface binding; here it is one gate
at the single chokepoint every inbound frame crosses. Any transport change must keep it.

## QA evidence — run by me, not reported to me

`cargo test --offline` in `ai-browser/foundation`, 2026-11-13: **45 passed, 0 failed**,
exit 0. Breakdown: `identity_contract` 15 · `frame_contract` 14 · `action_levels` 5 ·
`supervise_contract` 5 · `version_contract` 5 · 1 `compile_fail` doctest. Only deps are
`serde` + `serde_json`, already in the local registry cache — nothing downloaded.

Tests carrying real weight, all observed passing:
`same_url_reload_yields_a_different_document_key` ·
`inbound_frames_cannot_carry_a_document_key` ·
`the_shell_cannot_issue_a_utility_only_method` ·
`a_newer_major_is_rejected_before_the_payload_is_interpreted` ·
`no_target_resolution_tier_promises_the_accessibility_tree` ·
`host_driven_scroll_is_navigate_class_and_passes_the_release_zero_gate` ·
`a_utility_that_stopped_reading_stdin_is_killed_and_reaped` (against a real OS process).

The suite earned its keep by catching **dead defensive code Bob had written himself** —
an unreachable `identity.document_not_current` arm in `resolve_in_tab`, since a document
stops being current in exactly the step that makes `resolve` return `token_retired`.

## Required before unattended tasks

**No Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` exists yet.** Today a shell
crash orphans an AI child that still holds a live provider session. `abort()` covers the
cooperative path; the crash path is unhandled. That is a privacy hole, not a leak
cleanup, and it must land before anything runs unattended.

## Corrections to a subagent's report

Bob reported that `ai-browser/STATUS.md` "still lists GATE-0a/0b/0c, FND-05, and the
C++ utility-process decision". **Checked and false** — lines 14-24 of this file void all
three. A subagent's claim about another artifact is not evidence.

## Verdict: is a fork required for Release 0? **No.**

Alice's first pass claimed §2.1's overlay required a fork. She tested extension and
CDP surfaces, not the embedder host, and retracted it after I sent her back. Revised
verdict: **Release 0 is fully buildable on the WebView2 kernel** — nothing in
backlog §2, §3, §7 or §8 blocks it.

Independently verified by me on 2026-11-13, not forwarded:

| Claim | Check | Result |
|---|---|---|
| `wry` 0.57.2 needs no patch to reach raw COM | grep `wry-0.57.2/src/lib.rs` | **Confirmed** — `trait WebViewExtWindows` at 2343, `fn controller()` 2345, `fn environment()` 2348, `impl` 2381-2386 |
| No embedder accessibility-tree API exists | grep `webview2-com-sys-0.39.1/src/bindings.rs` for `[Aa]ccessibilit\|IAccessible\|uiautomation\|UiaRaise` | **Confirmed — 0 matches** |
| Size of that bindings file | `find /c /v ""` | **44,394** lines (Alice said 44,395; final-line counting convention) |

**Architecture rule, from Alice, adopted:** **one overlay controller per browser
window**, hosting toolbar + all cards + composer + indicator. One controller per
annotation would be ~+600 MB at ten comments and breaks the memory ceiling. Guard:
assert the `msedgewebview2` process count at startup — an accidentally separate
environment silently pays another browser process (~150 MB).

### Lost by not forking (confirmed list)

1. **The native omnibox.** `AutocompleteController` / `TemplateURLService` /
   `OmniboxEditModel` are not exposed to embedders. We write our own Rust widget fed
   by our own history and bookmarks — honours §2.5's intent, not its letter. The one
   clean unrecoverable loss.
2. **The accessibility tree — PARTLY REVERSED 2026-11-13, pending a runtime check.** There is
   still zero embedder AX API (`accessibilit`/`IAccessible`/`uiautomation`/`UiaRaise` → 0
   matches). But the CDP bridge is on the **base** `ICoreWebView2` (`CallDevToolsProtocolMethod`
   at bindings.rs:1897 inside `ICoreWebView2_Vtbl` starting 1771), so
   `Accessibility.getFullAXTree` is reachable without a fork. The user has ruled CDP **in for
   read-only extraction only**. So this item moves from "permanently gone" to "available via a
   policy-scoped, unstable-by-contract, feature-detected path" — and **nobody has made the call
   yet**. `AnnotationAnchor.accessibilityPath` remains unresolvable regardless: an AX *tree read*
   is not a resolvable *anchor tier*. Frame-rate privileged anchoring inside the page compositor
   is still traded away for authority over the surface.
3. **The runtime is not ours.** No engine patch, no in-Blink enforcement, no field
   trials, Evergreen updates underneath us. §3/§8's "renderer never returns
   credentials" degrades from structural guarantee to allowlist-plus-redaction-plus-
   tests. MS docs warn *"For performance reasons, `GET` HTTP requests may happen,
   while the host is responding"*, so nav-layer egress blocking is best-effort; the
   control that does work is the AI executable owning the program's only model socket.

### Weakened, not lost

Backlog §7.2 ranks 1-2 ("browser-native semantic command", "page-advertised
structured tool") collapse into our own `ExecuteScript` semantic action. That is a
real weakening of the action-safety model and argues for keeping Interact and Commit
out until Release 2.

## Decisions made 2026-11-13 (Carol)

1. **Scroll-to-passage ships in Release 0, classified Navigate-class read-only.**
   Backlog §9 says "no page mutations" while §12 requires citations that scroll to
   the passage; those conflict, and Alice flagged it. A host-driven scroll changes no
   page state, is trivially reversible, and citations you cannot verify in place are
   half the value. Recorded rationale; revisit if it ever covaries with anything
   consequential.
2. **No loopback TCP listener in the IPC transport.** Confirmed as a hard invariant.
   Pages rendered in our own shell can reach local ports, so a loopback socket would
   put the model channel inside the attack surface of every site we open. Named pipes
   with a per-instance ACL, or stdio, over the wire — and the AI executable must be
   the only holder of a model socket.

## Capability audit — 79 claims, 45 stories (2026-11-13)

Alice audited every capability claim in the 878-line backlog against the embedder surface.
Counts I have **not** independently reproduced and will not present as verified: **26** in our
code · **22** embedder COM · **4** CDP-only · **16** page-content-only (forgeable) · **13 not
reachable at all** · **11** unbuilt-but-reachable. **19 of 45 stories flagged, 8 of them P0.**

Two claims I **did** verify:

| Claim | Check | Result |
|---|---|---|
| CDP bridge is on the **base** `ICoreWebView2` | walked back from `CallDevToolsProtocolMethod` (1897) to the enclosing vtable | **Confirmed** — `ICoreWebView2_Vtbl` starts at 1771. No fork, no version `QueryInterface` |
| `ICoreWebView2Find` exists with `FindNext` | grep bindings.rs | **Confirmed** — `Find` at 42949, `FindNext` 18255. wry 0.57 doesn't expose it, which is why we hand-rolled find |

**Her line citations were wrong and I sent her back to re-derive all of them** — `SelectionText`
cited 8706, actually 8771; `HasSelection` 8697 → 8767; `Find` 42934 → 42949; CDP 1455 → 1897.
Offsets of 70–440, and only one `webview2-com-sys` is extracted here, so it was imprecision, not
a version mixup. Wrong line numbers in a document people cite in review are worse than none.

### CDP ruling (user, 2026-11-13)

**Read-only extraction only.** `DOMSnapshot.captureSnapshot` and `Accessibility.getFullAXTree`
are permitted. No CDP method that mutates page state — **never in the Interact or Commit path at
any release**. Never public or product-facing; no story or acceptance criterion may depend on it.
Pin the domain versions, feature-detect at startup, degrade to the main-world probe with a typed
`unsupported`, off by default behind a runtime-read flag that fails to disabled.

That makes `DOMSnapshot.captureSnapshot` our **only non-forgeable extraction path**, softening
C-49/C-62/ANN-02 from "all extraction is forgeable" to "most of it is". It also reverses one
listed permanent loss — see the loss list. **Unverified: whether the call works at all from an
embedder at runtime.** Queued for Bob after V2/V3.

### The backlog has 8 broken P0 stories — deltas pending user approval

`ai-browser/docs/BACKLOG-DELTAS.md` is being written as a line-by-line proposal against the
user's own text. **The user chose "deltas doc first, then apply"**, so nobody touches
`ai-native-browser-design-and-engineering-backlog.md` until they approve it. Worst entries:
**FND-05** is an impossible P0 (upstream sync against a checkout that doesn't exist), **SHL-02**'s
acceptance is "behave like Chromium" so it can never close, **FND-03**'s text still says Mojo,
**SAF-02** needs an `EnableWindow` substitute for a controller-level freeze that doesn't exist,
and **SAF-03**/**QAL-03** claim enterprise-policy and remote-kill-switch capabilities we can
neither read nor provide.

## Known degraded vs. the backlog as written

1. **§2.5 native omnibox** — not exposed to embedders. Own `omnibox.rs`. Clearest loss.
2. **§2.1 anchored card** — achievable as a host-owned controller window, but
   positioning comes from renderer-reported rects, so the card **pins during scroll
   and re-anchors ~120-150 ms after settle** rather than chasing the text. Alice
   estimates 25-50 ms p50 / 60-120 ms p95 reposition latency and 100-500 px error
   during a fling; **these are estimates, not measurements** — V2 in
   `FORK-OR-CDP.md` §7.
3. **§4.2 tier-2 extraction (accessibility tree)** — absent, permanently, at this SDK.
4. **No Blink patches** — canvas-only text, exotic shadow DOM, engine bugs.

## Not verified

- Nothing has been compiled in `ai-browser/` yet — no code exists there beyond docs
  and foundation work in flight.
- Every WebView2 capability above is grep-verified symbol presence, not observed
  behaviour. Each needs a running-webview check before it is load-bearing.
- `native-browser`'s baseline is sound: `cargo test` on 2026-11-13 gave
  **118 passed, 0 failed** in 0.39 s. This is the foundation we build on.
- The backlog's 45 stories have not been re-scored against the kernel decision —
  some P1/P2 items assume fork-only capability.