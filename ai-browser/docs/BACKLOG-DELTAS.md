# Proposed deltas to the backlog — approve or reject one at a time

**This file does not change the backlog.** `ai-native-browser-design-and-engineering-backlog.md` is the
user's file; every change below is a proposal with paste-ready replacement text. Nothing here has been
applied.

Kernel these deltas are written against (user decision, 2026-11-13): the **existing WebView2 Evergreen
runtime** driven by our Rust shell as the embedder host — no fork, no Chromium checkout, no Mojo, no
in-Blink enforcement point. Evidence is grep-derived from
`webview2-com-sys-0.39.1/src/bindings.rs` (44,394 lines) and from `crates/shell/src/`,
`ai-browser/crates/contract/src/`, `crates/shell/ui/`. bindings.rs line numbers refer to
webview2-com-sys-0.39.1 and were grep-derived on 2026-11-13 (the **vtable-slot** line inside
`I…_Vtbl`, not the safe-wrapper line). Absence claims list the exact grep pattern.

**Five user decisions and rulings are already folded in, so no delta here is written against an open question:**

| Date | Decision | Effect on this file |
|---|---|---|
| 2026-11-13 | Kernel = the existing WebView2 runtime, driven by our Rust shell | the whole audit's reason for existing |
| 2026-11-13 | **CDP permitted for read-only extraction only** — now one method, `DOMSnapshot.captureSnapshot`; never in the Interact/Commit path; never a product API; pinned, feature-detected, off by default | §4, D-11's addendum, D-15 closed, **D-21** |
| 2026-11-13 | **Accessibility features are out of scope** ("not for everyone… focus on 60 to 70% people") | **D-01** becomes a deletion; **§1b** reclassification; SHL-01 newly flagged; **D-20**; the AX half of the CDP ruling withdrawn |
| 2026-11-13 | **Keyboard navigation, focus handling and IME for our own chrome stay in scope** — confirmed by the user when asked directly, not inferred | C-42 stays in column **b**; **no** delta touches SHL-02's keyboard/IME acceptance, ANN-05's keyboard path, or D-05's L269 wording; D-20's Non-goals block names them as still in scope |
| 2026-11-13 | **Edge extractors cut: PDF text, video transcripts/captions, OCR** (§4.2 tiers 7, 8, 9) — required behaviour becomes the coverage report | **D-22** (final 6-rung ladder, CTX-06 shrink, QAL-01 clause); D-18 superseded |

**In scope by the fourth decision above:** keyboard navigation and IME for our own chrome (C-42) — input
quality for the majority user, not an accessibility feature. See §5.

## 1. Read this first — 7 P0 stories are blocked as written, and accessibility is now out of scope

Severity uses the backlog's own convention: **P0** = must be settled before Release-1 planning,
**P1** = settle while building, **P2** = editorial.

| Δ | Sev | Backlog location | What is wrong, in one line | P0 story touched |
|---|:-:|---|---|---|
| D-01 | **P0** | §2.1 L104, L116 · §4.2 L432 · CTX-01 L753 | `accessibilityPath` and extraction tier 2 are **out of scope by decision** (2026-11-13): delete both, renumber both ladders, drop the word from CTX-01 | CTX-01 (wording only now) |
| D-02 | **P0** | FND-05 L737 | "Chromium upstream-sync pipeline" is impossible — there is no fork to sync | **FND-05** |
| D-03 | **P0** | FND-03 L735 | "Mojo interfaces" does not exist for embedders; the real contract already ships | **FND-03** |
| D-04 | **P0** | SHL-02 L744 | "using Chromium providers and ranking … behave like Chromium" can never be closed | **SHL-02** |
| D-05 | **P0** | §2.5 L269 | "Do not build a parallel JavaScript omnibox" contradicts both the kernel and our shipped shell | SHL-02 |
| D-06 | **P0** | §12 L866, L872, L873 | Release-1 DoD demands split pane, remote kill switch and live dashboards — none reachable, none built | — |
| D-07 | **P0** | §7.1 L588 · §9 L680/L687 · ACT-01 L784 · §12 L865 | Scroll is simultaneously a `Navigate` mutation, banned in Release 0, and required by the DoD | **ACT-01** |
| D-08 | **P0** | §3 L358 · §12 L870 | "must not return cookies…" is written as a structural guarantee; it can only be policy + test | CTX-01 |
| D-09 | **P0** | §8 L664 · QAL-03 L816 · §12 L872 | "remotely rolled-out capability" / "staged rollout" assume a field-trial service this kernel does not expose | **QAL-03** |
| D-10 | **P0** | SAF-03 L807 | "enterprise-policy" check has no embedder policy API to read from | **SAF-03** |
| D-11 | **P0** | §2.3 L181-182 · CTX-04 L756 · ANN-02 L765 | **Missing requirement:** probe output is forgeable page content and a mutation feed is a self-inflicted DoS — no cap or trust rule is written down | CTX-04, ANN-02 |
| D-12 | **P0** | SAF-02 L806 | "cannot bypass the approval broker" is true of the dispatcher but the *modality* it implies has no API | **SAF-02** |
| D-13 | P1 | ACT-02 L785 · §7.2 L598 | the locator's accessibility half is **gone by decision**, so the story is no longer half-impossible — `DomLocator` becomes the design. Wording change only | — (no longer flagged) |
| D-14 | P1 | §2.3 L199 · §12 L863 | "invalidate element handles" presumes a handle type we cannot mint | — |
| D-15 | P1 | §7.2 L596 | rank 1 "browser-native semantic command" has no surface; ranks 1-2 collapse | — |
| D-16 | P1 | ACT-07 L790 | "Unexpected origin, **dialog**, permission … stops the task" — this SDK fires no dialog event | — |
| D-17 | P1 | §2.3 L174-184, L205 | 4 of the 11 context events can only come from page content, contradicting L205's "browser-process events rather than renderer polling" | CTX-02 |
| D-18 | P1 | CTX-06 L758 · §4.2 L437-438 (tiers 7-8 today, **6-7 after D-01's renumber**) | tiers 7-8 (PDF, transcripts) have no embedder source to read from | **SUPERSEDED by D-22** — those tiers are now deliberate cuts, not workarounds |
| D-19 | **P0** | CTX-03 L755 · ANN-03 L766 · (whole doc) | "meets the target" / "the target reattachment rate" — the backlog defines **no** numeric target anywhere, so two P0 acceptance criteria are unclosable | **CTX-03**, **ANN-03** |
| D-20 | **P0** | SHL-01 L743 · (L269 via D-05) · no non-goals list exists | an acceptance list still names accessibility / forced colors as a ship requirement, and nothing in the doc stops a contributor re-adding them | SHL-01 |
| D-21 | P1 | §4.2 L441 | the CDP sentence predates the 2026-11-13 ruling: it does not say read-only, one method, off by default, or that nothing may depend on it | — |
| D-22 | **P0** | §4.2 L431-439 · §4.1 L425 · CTX-06 L758 · QAL-01 L814 | the ladder still lists four tiers we have decided not to build (2, 7, 8, 9), and the spec never says what the assistant does instead | QAL-01 (wording: corpus asks for a PDF answer today) |

## 1b. Reclassification after the accessibility scope decision (2026-11-13)

The user ruled: *"I do not care the accessibility features as well, the browser is not for everyone,
but focus on 60 to 70% people."* Accessibility is **out of scope** — not deferred, not shipped later.
That moves four audit claims out of **(d) "cannot be reached"** and into a new column, which is the
better outcome: a thing we do not want has no cost.

| Audit id | Claim | Was | Now | What changes in the backlog |
|---|---|:-:|:-:|---|
| **C-05** | accessibility feature claim — **id referenced by the 2026-11-13 instruction; verbatim wording not recoverable from disk** (`CAPABILITY-CLAIM-AUDIT.md` carries the same caveat in its row) | **d** | **OUT OF SCOPE** | nothing to build; no delta needed beyond D-20's non-goal line |
| **C-45** | accessibility feature claim (same caveat, same reason) | **d** | **OUT OF SCOPE** | as above |
| **C-58** | accessibility-tree extraction (backlog §4.2 tier 2 L432) | **d** | **OUT OF SCOPE** | **D-01 / D-22**: delete L432, delete `accessibilityPath` L104/L116, delete the word from CTX-01 L753; §4.2's ladder is re-cut to **6** rungs by **D-22**, not renumbered to 8 as D-01 originally said |
| **C-74** | element-by-locator resolution for actions | **d** (half-impossible) | **OUT OF SCOPE** for the AX half | **D-13**: `TargetResolution::DomLocator` (`crates/contract/src/contract.rs:211`) is **the design, not a fallback** — the enum already ships without an AX variant and `crates/contract/tests/action_levels.rs:43` fails if anyone adds one. **ACT-02's half-impossible flag is gone** |
| C-90 / C-91 | synthetic semantic input | **d** | **d, unchanged** | still wanted, still forbidden (CDP ruling, act path). D-15 stands |
| C-49 | probe output is page-authored | improved by CDP | unchanged by this decision | D-11 stands (**`DOMSnapshot.captureSnapshot` is unaffected by the accessibility drop** — it returns text *and* layout bounds in one call; `FORK-OR-CDP.md` §1b.1's **Still buys** rows make that case on its own terms, and it is now measured callable) |

**Not dropped, deliberately — and confirmed:** keyboard navigation and IME for **our own** chrome —
C-42, the `ui/app.js` omnibox, focus handling, `SHL-02`'s "IME … and keyboard acceptance", and
`ANN-05`'s "All operations are keyboard accessible" (L768). Those are majority-user input quality, not
accessibility features, so **no delta touches them** and C-42 stays in column **b**. The user was asked
this as a category question and **confirmed it directly on 2026-11-13**, so D-20 no longer needs
re-cutting on this point.

**The process gap this exposed, and its limit.** The 79-claim inheritance audit is now committed as
`CAPABILITY-CLAIM-AUDIT.md` with the reclassification and the out-of-scope column, so reclassifications
like this one are checkable rather than remembered. It does **not** recover everything: the audit was
report-only when the accessibility ruling landed, so **C-05 and C-45 survive only as ids** — their
verbatim wording is gone, and the audit says so in their rows rather than reconstructing plausible
sentences. Treat those two ids as *named by the ruling, wording unrecoverable*; every other row is
re-derived from a backlog line, a `bindings.rs` slot, or a shipped contract type.

## 1c. Counts after both decisions

**7 P0 stories cannot be completed as written** — FND-03, FND-05, SHL-02, SAF-02, SAF-03, QAL-03,
ACT-01. *(Was 8: CTX-01 left this list when accessibility went out of scope, because the clause that
made it impossible is now deleted rather than implemented.)*
**7 further P0 stories need a wording or requirement change before they are closable** — CTX-01 (D-01),
SHL-01 (D-20, newly flagged), CTX-02, CTX-03, CTX-04, ANN-02, ANN-03.
**13 of the 27 P0 stories need no kernel-driven change** — FND-01, FND-02, FND-04, SHL-03, ANN-01,
MOD-01, MOD-02, MOD-03, ACT-06, SAF-01, SAF-04, QAL-01, QAL-02. *(SHL-01 moved out: its acceptance list
names accessibility and forced colors.)*
Three §12 DoD bullets (D-06) describe capabilities no one can build on this kernel.
Of the 18 non-P0 stories, **ACT-07** is the only one still kernel-blocked (D-16). **CTX-06 is no longer
kernel-blocked — it shrinks by decision** (D-22 cuts its PDF and transcript extractors and keeps its
structured-data/product/search/article ones). **ACT-02 is a wording change, not a defect** (D-13).
Among the 13 clean P0 stories, **QAL-01 needs one clause rewritten** (D-22): its corpus currently asks
for a PDF *answer*, and after the cuts the expected result for PDF, video and canvas is a coverage gap.

**Audited and clean:** the 13-story list in §1c above is current and supersedes every earlier count in
this file. The other 14 P0 stories are covered by D-01…D-22.

**To approve:** reply with the delta ids you accept (e.g. "D-02, D-03, D-07, D-11 accepted; D-04 rejected
because …"). Nothing is applied to the backlog until you say so. **Two decisions have since landed and
are already reflected here:** the CDP ruling (§4 — read-only, one method) and **accessibility out of
scope** (§1b). No delta in this file is conditional on an open question any more, except where a row
says `verify`.

## 2. Format

Each delta: **Location** (section, line) → **Current text** (verbatim) → **Wrong because** (evidence) →
**Replacement text** (paste-ready) → **If you reject it**.

---

## D-01 · §2.1 + §4.2 + CTX-01 — accessibility is out of scope: delete the tier, no hedge

**Decision this rests on (2026-11-13, user):** *"I do not care the accessibility features as well, the
browser is not for everyone, but focus on 60 to 70% people."* Accessibility features are **out of scope
— not deferred, not a later release.** This delta is a deletion, not a workaround, and it has no
"unless CDP" clause: the CDP question is **moot for accessibility** (§4).

**Location:** §2.1 `AnnotationAnchor` L104 and reattachment step 4 L116; §4.2 tier 2, L432; Epic C
`CTX-01` L753.

**Current text (L104):**
```ts
  accessibilityPath?: string[];
```
**Current text (L116):** `4. accessibility path;`
**Current text (L432):** `2. accessibility tree;`
**Current text (L753):**
`| CTX-01 | P0 | L | Implement metadata, semantic-DOM, accessibility, link, and table extraction. Authenticated test pages return bounded structured snapshots. | FND-02 |`

**Wrong because.** Two independent reasons; the second is now the operative one.
1. **Capability.** Grep `accessibilit`, `IAccessible`, `uiautomation`, `UiaRaise` → **0 matches** across
   all 44,394 lines of `bindings.rs`. No embedder accessibility-tree API exists on the COM surface, so
   even if we wanted it there was nothing to call.
2. **Scope.** The user has decided we do not want it. That settles the field, the tier, and the CDP
   temptation in one stroke: `Accessibility.*` is **never called**, even though the same user permitted
   CDP for read-only extraction (§4). `FORK-OR-CDP.md` §1b.1's **Still buys** rows keep
   `DOMSnapshot.captureSnapshot` on its own merits — non-forgeable structure, geometry and
   renderer-computed visibility — and **borrow nothing from the accessibility argument.**

**Already true in shipped code, so this delta aligns the spec with reality:** `AnnotationAnchor`
(`crates/contract/src/contract.rs:242`) carries `document`, `snapshot_version`, `block_id`, `quote`,
`prefix`, `suffix`, `text_position` — **no AX field**; `AnchorReattachmentTier`
(`contract.rs:226`) is exactly the five tiers below; `TargetResolution` has no accessibility variant
(`contract.rs:207-214`) and `crates/contract/tests/action_levels.rs:43` fails the build if anyone adds one.

**Replacement text (L104):** delete the line, and add above the interface:
> Anchor tiers are limited to what our own extraction pass can re-resolve: semantic block id, quote with
> affixes, normalised text position, fuzzy quote match. Accessibility paths are not a tier — accessibility
> features are out of scope for this browser (decision, 2026-11-13).

**Replacement text (L113-118):**
```text
1. semantic block ID (minted by the browser's own extraction pass, per snapshot);
2. quote plus prefix/suffix;
3. normalized text position;
4. fuzzy quote match;
5. detach and ask the user to re-anchor.
```
**Replacement text (L431-439)** — 8 tiers, accessibility removed and the list renumbered:
```text
1. browser and document metadata;
2. semantic DOM;
3. rendered visibility/layout;
4. structured data;
5. page-type extractors;
6. PDF/document extractors (own parser — see D-18);
7. transcripts (own asset lookup — see D-18);
8. OCR/vision fallback.
```
Add after the list: *"No accessibility-tree tier: accessibility features are out of scope for this
browser (decision, 2026-11-13), and the embedder surface exposes no accessibility tree in any case."*

**Replacement text (L753):**
`| CTX-01 | P0 | L | Implement metadata, semantic-DOM, link, and table extraction. Authenticated test pages return bounded structured snapshots. | FND-02 |`

**If you reject it:** the spec keeps a field and a tier nobody may implement, and CTX-01 stays a P0 whose
acceptance text asks for something the product has deliberately decided not to want — which is worse than
an impossible requirement, because reattachment code will silently treat an always-empty field as a signal.

---

## D-02 · FND-05 — an impossible P0

**Location:** Epic A, L737.

**Current text:**
`| FND-05 | P0 | M | Establish Chromium upstream-sync pipeline and OWNERS rules. A clean upstream merge can be built and smoke-tested automatically. | — |`

**Wrong because.** There is no fork (user decision 2026-11-13), so there is nothing to merge and no
`OWNERS` file to gate. This story is not "descoped", it is **void**: it can never be completed and it
sits in the P0 set that gates private alpha.

**Replacement text:**
`| FND-05 | P0 | S | Pin and verify the runtime boundary: record the WebView2 Evergreen runtime version used for every gate run, assert the feature set the shell depends on exists at that version, and define the fail-closed behaviour when an Evergreen update removes or changes a used API. Compatibility smoke suite runs automatically. | FND-01 |`

**If you reject it:** a P0 story blocks Release 1 with no possible "done", and the real risk it was
standing in for — *Evergreen moves under us* — stays unowned.

---

## D-03 · FND-03 — the mechanism is not Mojo, and it already ships

**Location:** Epic A, L735. Same error in the §3 diagram, L282 (`│ typed Mojo interfaces`) and the
Chromium Platform box, L297 (`AX tree | Blink`).

**Current text:**
`| FND-03 | P0 | M | Establish browser/AI/renderer process boundaries and Mojo interfaces. No model/network work blocks the browser UI thread. | FND-01 |`

**Wrong because.** Mojo is Chromium-internal; an embedder gets no Mojo interfaces and cannot register a
Mojo service. Our process boundary is an OS process over framed stdio, and it **already ships**:
`ai-platform-contract` (45 tests green) defines a closed 7-method `Method` enum with `Peer` side-gating,
and `supervise::{spawn,cancel,abort}` implements EOF → 500 ms → kill → reap.

**Replacement text:**
`| FND-03 | P0 | M | Establish the browser-process / AI-utility / page-probe boundary as three separate trust domains over a framed transport (NDJSON over piped stdio). Contracts are versioned in one shared crate; the utility links no browser-automation library at all. No model or network work runs on the browser UI thread. | FND-01 |`

**Also replace L282** with `│ typed contract (framed stdio)` and, in the L297 box, replace
`AX tree | Blink` with `Blink` — the AX tree is not reachable (D-01).

**If you reject it:** the first P0 story on the critical path names a mechanism that does not exist, and
whoever picks it up spends the week discovering it.

---

## D-04 · SHL-02 — acceptance criterion that can never be closed

**Location:** Epic B, L744.

**Current text:**
`| SHL-02 | P0 | L | Implement native omnibox overlay using Chromium providers and ranking. URLs, searches, IME, policy, and keyboard acceptance behave like Chromium. | SHL-01 |`

**Wrong because.** `AutocompleteController`, `TemplateURLService`, `OmniboxEditModel` and
`LocationBarView` are not exposed to embedders — there is no API to "use Chromium providers and ranking",
and "behave like Chromium" names no observable pass/fail. The shipped shell already owns the substitute:
JS `<input id="omnibox">` + ARIA combobox in `ui/app.js` (40 ms debounce → `omnibox.query`), with the
decision authority in Rust (`src/omnibox.rs`: pure, deterministic `Mode::{Url,Search,Command,Find}`,
zero-egress) over our own history/bookmarks stores.

**Replacement text:**
`| SHL-02 | P0 | L | Implement the browser-owned omnibox: browser-process classifier and ranker over local history, bookmarks, and open tabs, with a keyboard-first text field in the app-owned chrome surface. Acceptance is measured, not comparative: a fixed 200-query corpus classifies with ≥ 99% expected-mode agreement; zero network requests are issued while typing unless the user explicitly accepts a web suggestion; Up/Down/Tab/Enter/Escape and IME composition pass the scripted keyboard suite; screen-reader announcement of the suggestion list is verified. | SHL-01 |`

**If you reject it:** keep it as written and the story is uncloseable — "behave like Chromium" is not a
test, so the only way to finish it is to argue.

---

## D-05 · §2.5 — the omnibox sentence contradicts the kernel *and* our own shell

**Location:** §2.5, L269 (and the pattern note at L271).

**Current text:**
`Do not build a parallel JavaScript omnibox. Reuse Chromium classification, autocomplete, keyboard, accessibility, IME, policy, and search-provider behavior.`

**Wrong because.** Second sentence unsatisfiable (D-04). The first is **already false of our product**, by
necessity and by choice: the chrome — including the omnibox — is a JS document served from the app-owned
`asset://` origin. That is deliberately *not* the thing the sentence was guarding against: it is never
page DOM, page scripts cannot reach it, and classification/ranking live in Rust, not JS. Note also that
L271's "New Tab for Copilot In-App Browsing" pattern (native Chromium control owns editing) is exactly the
part we cannot have. My own earlier draft of `FORK-OR-CDP.md` claimed ours was "a native widget, not a JS
box"; that was wrong and has been corrected.

**Replacement text:**
`The omnibox's decision logic is browser-process code: mode classification, ranking, and acceptance are computed in the browser process over local stores, never in page content and never by a model. The editing surface may live in the app's own UI document, but it must be an app-owned surface that page scripts cannot read, restyle, or reach, and it must not depend on Chromium-internal components the embedder cannot access. Reusing Chromium's classification, autocomplete, keyword, or search-provider policy is not available to an embedder and is explicitly out of scope: we own ranking, keyboard behaviour, IME polish, and provider configuration.`

**If you reject it:** the document forbids the only design available on this kernel, and a reader will
"fix" the shell to satisfy a sentence that cannot be satisfied.

---

## D-06 · §12 — the DoD is Release 1's, and three bullets are unreachable

**Location:** §12, L866, L872, L873 (and §9's Release-1 list, L682-692).

**Current text (L866):** `- Returned links can open in a split browsing pane.`
**Current text (L872):** `- Feature flags and kill switches work without requiring a new binary.`
**Current text (L873):** `- Crash, latency, coverage, cancellation, and correctness dashboards are operating.`

**Wrong because.** §12 says "the first useful release" while §9 splits that scope across Release 0
(proof) and Release 1 (alpha); the DoD is therefore Release 1's definition, and should say so. Then:
split panes are unbuilt (and `SHL-04` is P1); a *remote* kill switch presumes a rollout service (D-09);
dashboards are an unbuilt P1 story (`QAL-04`) with no ingestion pipeline named anywhere. A DoD that
requires unbuilt platform is not a gate, it is a wish.

**Replacement text:** change L858 to
`Release 1 ("Read with me" private alpha) is complete only when:`
then replace the three bullets with:
```text
- Returned links open in a new tab or a background tab, and can be pinned beside the assistant.
  (Split-pane viewing is a Release-2 capability and is not part of this definition.)
- Every AI capability is disableable by configuration that the running process reloads without a new
  binary; egress is stopped by terminating the AI utility process (see the kill-switch note in §8).
- Performance, crash, coverage, and cancellation telemetry is emitted locally and reviewed manually
  before each release gate. Hosted dashboards and automated release gates are Release-2 scope.
```

**If you reject it:** Release 1 cannot be declared done, because three of its bullets depend on other
releases.

---

## D-07 · §7.1 vs §9 vs ACT-01 vs §12 — one sentence resolves the scroll contradiction

**Location:** §7.1 L588; §9 Release 0 L680 (`- no page mutations`); §9 Release 1 L687
(`- citations and scroll-to-source`); ACT-01 L784; §12 L865.

**Current texts:**
`| Navigate | Reversible | Open, close, switch, preview, scroll |`
`| ACT-01 | P0 | M | Implement read-only tools: active context, blocks, links, tabs, selection, find, and scroll-to-source. … |`
`- Citations scroll to the supporting passage.`

**Wrong because.** §7.1 classes scroll as `Navigate`, i.e. a mutation class. §9 Release 0 bans page
mutations. ACT-01 (P0) then files scroll-to-source under **read-only** tools, and §12 demands it for the
first release. Four statements, one capability, mutually inconsistent.

**Replacement text (one sentence, inserted after §9's Release-0 bullet list and echoed in ACT-01):**
`Release 0's "no page mutations" forbids changes to page DOM and page state; host-driven scrolling is a Navigate-class capability, is explicitly permitted in Release 0, and is the mechanism scroll-to-source uses.`

**Also in ACT-01 (L784),** replace `Implement read-only tools:` with
`Implement read-only and Navigate-class tools (scroll is Navigate, not read-only):`.

**If you reject it:** either Release 0 ships a citation feature that violates its own rule, or Release 1
loses a DoD bullet — and whichever it is will be decided by whoever reads the doc first.

---

## D-08 · §3 + §12 — credential exclusion is policy plus a test, not a structural guarantee

**Location:** §3 "Renderer process", L358; §12 L870.

**Current text (L358):**
`It must not return cookies, password values, authentication headers, browser storage, or hidden credential fields.`

**Wrong because.** Under the fork plan this sentence described an *enforcement point in Blink*. On this
kernel the extraction probe runs as **page-content JavaScript in the frame's main world** — WebView2
exposes no isolated-world API — so the probe is forgeable and cannot be the boundary. The guarantee has to
be re-expressed as three host-side layers plus a test that fails when they leak.

**Replacement text (L358):**
`The probe is untrusted page content and is never the credential boundary. Credential exclusion is enforced in the browser process: (a) allowlist-shaped extraction — the host requests semantic blocks, links, and control descriptors, never values; (b) a host-side redaction pass over the serialised payload before it leaves the browser process; (c) egress header stripping for anything leaving the machine. A release-blocking test asserts no key matching password|cookie|token|authorization|secret and no hidden-field value appears in any snapshot. "Cannot" in this design means "is tested for", not "is architecturally impossible".`

**Replacement text (L870):** `- No password, cookie, token, authentication header, or browser-storage content enters snapshots — verified by the redaction test suite, which blocks the release on any hit.`

**If you reject it:** the document asserts a structural guarantee we do not have, and the next auditor
finds it before the next user does.

---

## D-09 · §8 + QAL-03 — there is no remote rollout service on this kernel

**Location:** §8 required controls, L664; QAL-03 L816; §12 L872 (see D-06).

**Current text (L664):** `- kill switch for every remotely rolled-out capability.`
**Current text (L816):**
`| QAL-03 | P0 | M | Implement feature flags, kill switches, staged rollout, and fail-to-disabled behavior. Every external provider and action level is independently disableable. | SAF-03 |`

**Wrong because.** "Remotely rolled-out" and "staged rollout" assume a field-trial/configuration service.
An embedder gets none: there is no field-trial client, no remote-config API and no rollout surface in the
bindings; anything passed as additional browser arguments is fixed at environment creation
(`webview2.rs::create_shared_environment`) and needs a restart. We can ship local flags and a signed
config file we fetch ourselves — that is *our* mechanism and it must be named as ours.

**Replacement text (L664):** `- kill switch for every AI capability, and a named owner for the distribution channel that delivers it (see the flag-configuration note in this section).`
**Replacement text (L816):**
`| QAL-03 | P0 | M | Implement feature flags, kill switches, and fail-to-disabled behaviour, with rollout staged by our own configuration distribution (signed config file or service we build — not an engine field-trial). Every external provider and action level is independently disableable without shipping a new binary; anything fixed at runtime-environment creation is documented as requiring a restart. | SAF-03 |`

**Add to §8 after L664:**
`Flag distribution is product-owned. The kernel provides no field-trial, remote-configuration, or staged-rollout service, and browser-environment options are frozen at creation; a restart-free kill switch therefore requires configuration the product reads and reloads itself.`

**If you reject it:** QAL-03 (P0) waits forever on infrastructure that the kernel will never provide.

---

## D-10 · SAF-03 — "enterprise-policy" has no source

**Location:** Epic H, L807.

**Current text:**
`| SAF-03 | P0 | M | Implement URL, scheme, origin, profile, enterprise-policy, and content-sharing checks. Policy failures are typed and fail closed. | SAF-02 |`

**Wrong because.** There is no embedder policy/management API in the bindings (no policy-provider, no
managed-settings surface) — an embedder cannot read Chromium's enterprise policy store. Either we build
our own admin-configurable policy file, or the word is removed. Leaving it means the story can be marked
done while checking nothing an enterprise actually set.

**Replacement text:**
`| SAF-03 | P0 | M | Implement URL, scheme, origin, profile, and content-sharing checks against a product-owned policy source (a signed local policy file; group-policy delivery to that file is deployment work, not engine integration). Policy failures are typed and fail closed. | SAF-02 |`

**If you reject it:** a P0 safety story is satisfied by an absent system.

---

## D-11 · CTX-04 + ANN-02 — write the rate cap and the untrusted-data rule into the contract

**Location:** §2.3 events L181-182 (`MeaningfulContentChanged`, `SelectionChanged`); CTX-04 L756;
ANN-02 L765.

**Current text (L756):**
`| CTX-04 | P0 | L | Implement incremental snapshot deltas and meaningful-mutation classification. Decorative mutation storms do not trigger full extraction. | CTX-01 |`
**Current text (L765):**
`| ANN-02 | P0 | L | Implement streamed inline answer card with follow-up composer and citations. Page navigation cancels or detaches the card correctly. | ANN-01, MOD-02 |`

**Wrong because — this is a missing requirement, not a wrong claim.** Audit ids **C-49** (main-world probe
output is page-controlled and forgeable) and **C-62** (a page-side observer feed is a page-controlled
message rate) plus **ANN-02**'s citation path. Three of the context events
(`SelectionChanged`, `MeaningfulContentChanged`, `ViewportChanged`) have no embedder event
(counter-check: no selection event exists — the only selection APIs are
`ICoreWebView2ContextMenuTarget::{HasSelection 8767, SelectionText 8771}`, which fire only while a context
menu is being built). They can therefore only come from a page-side observer. That makes two failure modes
ours to own, and nothing in the backlog mentions either:
1. **Self-inflicted DoS.** A `MutationObserver`/`selectionchange` feed pumped through
   `PostWebMessageAsJson` into a delta classifier is a page-controlled message rate on the browser UI
   thread. A hostile page does not need to beat our overlay — it just needs to make our own observer
   expensive. CTX-04's "decorative mutation storms do not trigger full extraction" bounds the *response*
   but not the *ingress rate*.
2. **Forgeable data.** A main-world probe reports identity-adjacent values (`location.href`, rects,
   selection text) that page script can fake. Treat any probe-reported identity or geometry as a
   correlation hint, never as authority — the shipped design already does this by minting its own
   `navigation_sequence`.

**Effect of the CDP ruling (2026-11-13) — read this before approving.** `DOMSnapshot.captureSnapshot` is
now permitted for read-only extraction, so failure mode 2 is no longer total: a one-shot snapshot can be
renderer-serialised, which page script can neither forge nor flood. What does **not** change, and why this
delta stays P0 exactly as written:
- a snapshot is a **pull**. `captureSnapshot` is not a change feed, so **C-62 is untouched** and the cap is
  still the only control on ingress;
- the CDP path is **off by default** and unstable-by-contract, so the main-world probe remains the default
  extractor and must be safe on its own (`FORK-OR-CDP.md` §1b.1, **Still buys** rows);
- extracted **content** is page-authored however we obtain it — the untrusted-data rule is about content
  trust, not channel trust.

Net: **C-49 materially improved** (structure, geometry, renderer-computed visibility), **C-62 unchanged**,
**ANN-02 fidelity improved / trust unchanged**. D-11 is **not** relaxed by the ruling.

**Replacement text (L756):**
`| CTX-04 | P0 | L | Implement incremental snapshot deltas and meaningful-mutation classification **inside a bounded page-observer contract**: the page-side observer coalesces to at most one message per animation frame and at most N messages per second (budget in the budgets file); the browser process drops-and-flags beyond the cap rather than queueing; a page exceeding the cap for T seconds is marked hostile-observer and downgraded to pull-based extraction. Decorative mutation storms do not trigger full extraction. Every field arriving from the observer is validated against a schema, size-capped, and treated as untrusted data: page-reported document identity, URL, and geometry are correlation hints only, and nothing security-relevant is positioned or authorised from them. | CTX-01 |`

**Add to ANN-02 (L765), after the existing sentence:**
`The card's content, citations, and anchor data are produced by the browser process; selection text arriving from the page is validated, size-capped, and attributed as page-supplied.`

**If you reject it:** the cheapest remote-code-execution-adjacent failure we could ship is a page that
starves our own UI thread with our own observer — and we would have specified it.

---

## D-12 · SAF-02 — the "cannot bypass" claim is fine; the modality it implies is not

**Location:** Epic H, L806.

**Current text:**
`| SAF-02 | P0 | L | Implement approval classes for observe, navigate, interact, and commit. High-impact actions cannot bypass the approval broker. | FND-02 |`

**Why it needs rewording.** "Cannot bypass" is *true and stronger* than the extension version — execution
lives behind one dispatcher in our process and the AI utility holds no capability object. What is false is
the assumption readers add: that the page can be frozen while the user decides.
`ICoreWebView2Controller::put_IsEnabled` does not exist (the Controller exposes `IsVisible` 9309 /
`SetIsVisible` 9313; `SetIsEnabled` 7748 belongs to `ICoreWebView2ContextMenuItem`, `IsEnabled`/`Enable`
4256/4260 to `ICoreWebView2BrowserExtension`). The working substitute is Win32
`EnableWindow(controllerHwnd, FALSE)` on the tab's own controller HWND.

**Replacement text:**
`| SAF-02 | P0 | L | Implement approval classes for observe, navigate, interact, and commit, enforced at the single action dispatcher in the browser process; the AI utility holds no capability handle, so a pending action has no path to execution. While a decision is pending the tab's input surface is suspended by host window disablement (the embedder exposes no input-enable API on the controller); the page may continue changing, so approval re-validates against a fresh snapshot at dispatch time. | FND-02 |`

**If you reject it:** the reader assumes a modal page freeze that the kernel will not give them, and the
first real approval test finds a page mutating under the dialog.

---

## D-19 · CTX-03 + ANN-03 — acceptance criteria that point at a number nobody wrote

**Location:** Epic C L755, Epic D L766; and the absence of any budget section in §8-§12.

**Current text (L755):**
`| CTX-03 | P0 | M | Implement active-tab context switching. P95 context-header update meets the target and no previous-tab response can act on the new tab. | CTX-01, CTX-02 |`
**Current text (L766):**
`| ANN-03 | P0 | M | Implement multi-selector annotation anchoring and confidence scoring. Ordinary article edits preserve at least the target reattachment rate. | CTX-01 |`

**Wrong because.** There is no "target" to meet. Grep of all 878 lines for
`p95 | P95 | ≤ | budget | within N` returns **exactly three lines** — these two plus MOD-05's "token
budgeting" (a feature, not a target) — and **none of the three contains a number**. Meanwhile the document
promises exactly the discipline it never quantifies: L25 `- Cancellation, stale-context handling, bounded
output, and telemetry are first-class.` and L670 `The initial milestone should be a useful **read-only
vertical slice**, not a broad but unreliable agent.` Two P0 stories are therefore unclosable by
construction: any implementation passes, and so does none.

**Replacement text (new subsection, insert as "## 9.5 Performance and quality budgets (normative)" before §10):**

```text
## 9.5 Performance and quality budgets (normative)

No story may reference a target that is not in this table. Every value is a proposal pending measurement
on the pinned runtime; measured values replace proposed ones at each release gate and the delta is recorded.

| Budget | Target (p95 unless stated) |
|---|---|
| Context-header update after tab switch | ≤ 100 ms |
| Semantic snapshot, article page | ≤ 150 ms and ≤ 2 MiB |
| Semantic snapshot, 5 MB SPA | ≤ 400 ms and ≤ 2 MiB |
| Selection → first streamed token (local replay provider, shell-side) | ≤ 50 ms |
| Stop pressed → last token delivered | ≤ 100 ms |
| Navigation → open answers marked stale | ≤ 100 ms |
| Re-anchor after scroll settles | ≤ 200 ms, and zero reposition calls during a fling |
| Anchor outcome on the golden edit corpus | ≥ 90 % attach at tier 1-2; 100 % either attached or explicitly detached |
| Inline overlay surface | +1 renderer and ≈ 60 MB per window (ceiling 80 MB), +0 per open answer |
| AI utility resident set | ≤ 150 MB, breach terminates the utility with a typed `unavailable` |
| Browser UI-thread stall attributable to AI work | none > 50 ms |
```

**Replacement text (L755):**
`| CTX-03 | P0 | M | Implement active-tab context switching. Context-header update meets the §9.5 budget, and no previous-tab response can act on the new tab. | CTX-01, CTX-02 |`
**Replacement text (L766):**
`| ANN-03 | P0 | M | Implement multi-selector annotation anchoring and confidence scoring. On the golden edit corpus the §9.5 anchor budget is met, and an anchor that cannot be resolved is explicitly detached rather than silently misplaced. | CTX-01 |`

**If you reject it:** the two P0 stories remain unfalsifiable, and "we met the target" stays an opinion
that no test can contradict — which is exactly how a read-only slice turns into an unshippable one.

---

## D-20 · SHL-01 + §2.5 + a missing Non-goals line — finish the accessibility cut

**Location:** Epic B `SHL-01` L743; §2.5 L269; and the doc-wide absence of any non-goals statement.

**Current text (L743):**
`| SHL-01 | P0 | L | Implement native AI workspace container with browser and assistant panes. Resize, focus, keyboard, accessibility, localization, and forced colors work. | FND-03 |`
**Current text (L269, tail):**
`Do not build a parallel JavaScript omnibox. Reuse Chromium classification, autocomplete, keyboard, accessibility, IME, policy, and search-provider behavior.`

**Wrong because.** Both lines still make accessibility a **ship requirement**: SHL-01 is a P0 whose
acceptance includes "accessibility … and forced colors work" (forced colors = Windows high-contrast
mode, an assistive-tech requirement), and §2.5 asks us to "reuse Chromium's … accessibility" behaviour
we cannot reach and no longer want. Separately, the backlog has **no non-goals section at all**, so
nothing stops a contributor (or an agent) from re-adding an accessibility story after this decision.

**Replacement text (L743):**
`| SHL-01 | P0 | L | Implement native AI workspace container with browser and assistant panes. Resize, focus, keyboard navigation, and localization work. | FND-03 |`

**§2.5 L269 — no second replacement proposed.** That line is already governed by **D-05**, whose
replacement text drops the word "accessibility" and keeps "keyboard behaviour, IME polish" as things we
own. Approving D-05 alone fully discharges the L269 part of this delta; do not apply two rewrites to one
line. What D-20 adds beyond D-05 is **L743** and the **Non-goals** block.

**New subsection, suggest after §1's direction paragraph:**
> **Non-goals.** Accessibility features (screen-reader support, forced-colors/high-contrast
> conformance, accessibility-tree extraction, ARIA-driven anchoring) are **out of scope by decision**
> (2026-11-13): this browser targets the 60-70% majority use case. This is a scope decision, not a
> deferral — do not add accessibility stories to a later release. **Explicitly still in scope, because
> they are input quality and not assistive technology** — and confirmed as in scope by the user on
> 2026-11-13 when the category boundary was put to them directly: full keyboard navigation of our own
> chrome, IME composition in the omnibox and composer, and visible focus indication.

**If you reject it:** SHL-01 stays a P0 that cannot be signed off, and the decision exists only in chat
logs — so the next person to read §4.2 will helpfully reimplement the thing we just dropped.

## D-21 · §4.2 L441 — the CDP sentence predates the ruling

**Location:** §4.2, L441.

**Current text (L441):**
`The Chrome DevTools Protocol demonstrates useful structured inspection surfaces for DOM, accessibility, page, target, and browser state, although the product should expose stable internal contracts rather than making CDP its permanent product API.`

**Wrong because.** The sentence is a hedge written before there was a policy. The user has now ruled
(2026-11-13): CDP is **permitted for read-only extraction only**, and the accessibility half of the
sentence is moot (§1b). "Demonstrates useful … surfaces" also invites someone to reach for the next
domain they find. Evidence that the constraint is real, not theoretical: the bridge is `ICoreWebView2::`
`CallDevToolsProtocolMethod` (bindings.rs:1897, **base** interface, so nothing about it is gated behind a
version `QueryInterface`), MS docs state an unknown `{domain}.{method}` fails with `E_INVALIDARG` and
that **"CDP method calls may be processed out of order"**. Both are measured rather than quoted as of
2026-11-13: on runtime 157.0.4341.57 the bridge **answers** — `DOMSnapshot.captureSnapshot` returned
completion `S_OK` in 2.2 ms carrying 35 DOM nodes and 23 layout rects, with DevTools never opened and
`AreDevToolsEnabled` reading back `false` (`FORK-OR-CDP.md` §1b.1, `tools/wv2-probe/RESULTS.md`) — while
an **invented** method returns `S_OK` synchronously and only fails at completion with `0x80070057`, which
*is* the promised `E_INVALIDARG`; the docs simply never said it arrives asynchronously. That asymmetry is
the strongest possible argument for "nothing may depend on it, and the flag fails to disabled".

**Replacement text (L441):**
> The Chrome DevTools Protocol is reachable from the embedder host and is **permitted for read-only
> extraction only**: `DOMSnapshot.captureSnapshot`, as an internal implementation detail of the
> extraction layer. It is never a product API, never an extension or tool surface, and no story or
> acceptance criterion may depend on it. No CDP method that mutates page state may be used in the
> Interact or Commit path at any release without a new decision. The surface is treated as unstable:
> the called domain and its parameter shape are pinned per runtime version, the capability is
> feature-detected at startup and **off by default** behind a runtime-read flag that fails to disabled,
> and a missing or failing domain degrades to the DOM-probe path with the typed `unsupported` failure —
> never a broken page load.

**If you reject it:** the one sentence in the spec that governs the most unstable dependency we have
still reads as an invitation, and "off by default, nothing may depend on it" stays tribal knowledge.

## D-22 · §4.2 — the final extraction ladder, and three deliberate cuts

**Authority:** ruled by Carol on 2026-11-13 rather than put to the user again — the earlier question
conflated PDF, captions and OCR with accessibility, which they are not. **These are scope decisions with
the 60-70% rationale, not kernel limitations**, except tier 8, which genuinely has no path.

**Location:** §4.2 L431-439; §4.1 L425 (already correct, cited as the requirement we keep); Epic C
CTX-06 L758; Epic J QAL-01 L814.

**Supersedes two fragments in this file.** The §4.2 replacement inside **D-01** (the 8-tier list) and the
tier-7/8 wording inside **D-18** are both withdrawn. **Approving D-22 means applying exactly one
replacement to L431-439 — the one below.**

**Current text (L431-439, verbatim):**
```text
1. browser and document metadata;
2. accessibility tree;
3. semantic DOM;
4. rendered visibility/layout;
5. structured data;
6. page-type extractors;
7. PDF/document extractors;
8. transcripts;
9. OCR/vision fallback.
```

**Replacement text (L431-439) — this is what we are building, all of it, no ambiguity:**
```text
1. browser and document metadata;
2. semantic DOM;
3. rendered visibility/layout;
4. structured data;
5. page-type extractors (article, product, search-result);
6. coverage reporting — what the assistant could not read, stated to the user and to the model.
```

**⚠ One arithmetic note.** The ruling said "shrinks from 9 to **6**". Deleting tiers 2, 7, 8 and 9 from a
9-item list leaves **5**. The list above therefore adds tier 6 = **coverage reporting** as an explicit
rung, which is the honest reading: after four cuts, *saying what you could not read is the sixth thing
the ladder does*, and it is already a requirement at L425 and already a type in the shipped contract. If
you meant something else by 6, the only candidate to restore is PDF-with-our-own-parser — say so and I
will re-cut this delta.

**The three cuts, each recorded with its real reason:**

| Old tier | Cut | Reason — wanted-not unless marked otherwise | What the product does instead |
|---|---|---|---|
| 7 PDF text | **CUT** | It is *not* "no embedder API" shorthand for "too expensive": the runtime's PDF viewer exposes nothing, so the work is intercepting bytes + writing our own PDF text extractor. Not now, majority-case reasoning | Coverage gap on the PDF; the user can still download and open it in the runtime's viewer |
| 8 transcripts / captions | **CUT** | **Genuinely no path.** No embedder media, caption, or text-track API exists at all; DRM text tracks are unreachable. Page-exposed `<track>` or on-page transcript text is ordinary DOM content and stays in **tier 2** | `has_untranscribed_media = true` + a coverage gap; never an invented transcript |
| 9 OCR / vision | **CUT** | It existed to rescue canvas-rendered text. With the accessibility tier already gone we have decided not to chase that class of page, so a heavy OCR fallback buys a case we do not target | Coverage gap naming the unreadable region |

**The mechanism is already in the contract — do not invent one.** Cite these in the spec text:
`CoverageGapKind::UnsupportedContent` (`crates/contract/src/contract.rs:47`), carried as
`ContextCoverage::unsupported_content: Vec<CoverageGap>` (`contract.rs:82`) with
`status: CoverageStatus::Partial` (`contract.rs:63`, field at `:78`), and
`has_untranscribed_media: bool` (`contract.rs:83`) for the media case. The requirement sentence already
exists at **L425**: *"Do not claim 'AI has read the whole page' when the browser cannot access
third-party frames, unloaded content, canvas text, or media."* — so the cuts add no new contract type.

**Replacement text (L758, CTX-06):**
`| CTX-06 | P1 | M | Add structured-data, product, search-result, and article extractors. Each extractor has golden-page tests. PDF, transcript, and OCR extractors are out of scope (decision, 2026-11-13); the corresponding content surfaces as a typed coverage gap, and a golden test asserts the assistant says it cannot read it rather than guessing. | CTX-01, CTX-05 |`

**Replacement text (L814, QAL-01 — one clause, so the corpus stops asking for a PDF *answer*):**
`| QAL-01 | P0 | L | Create deterministic test corpus covering articles, shopping, SPAs, frames, lazy loading, authenticated pages, and adversarial content, plus non-answer cases (PDF, video, canvas) whose expected result is an explicit coverage gap. | CTX-01 |`

**If you reject it:** either the four cuts get relitigated as "unsupported by the kernel" — which is
false for three of them and misstates a product decision as an engineering defeat — or the ladder keeps
tiers nobody will build, and the assistant's only honest output for a PDF, a video, and a canvas stays
undefined in the spec.

---

## D-13..D-18 — P1 wording (same evidence base, shorter treatment)

| Δ | Location | Current text (excerpt) | Problem | Replacement (gist — say so and I'll expand any one) |
|---|---|---|---|---|
| D-13 | ACT-02 L785, §7.2 L598 | "stable accessibility/DOM locator format"; rank "3. accessibility/DOM locator;" | **Not a defect any more** — the accessibility half is out of scope (D-01), so the tier is simply DOM. What remains is the *handle* problem (D-14). `TargetResolution::DomLocator` (`contract.rs:211`) is the design, and `tests/action_levels.rs:43` already forbids an AX variant. | "Locator = (frame identity, snapshot version, DOM selector from D-01's tier list) resolved just-in-time by the shell via `ExecuteScript`; a locator that does not resolve uniquely fails typed." Same wording for §7.2 L598: "3. DOM locator;". |
| D-14 | §2.3 L199, §12 L863 | "invalidate element handles and action targets" / "stale element and document references" | We cannot mint an element handle the runtime will honour; only our own anchors. | "invalidate document-scoped references: anchors, snapshot versions, and any resolved target descriptor". |
| D-15 | §7.2 L596 | "1. browser-native semantic command;" | No embedder semantic-command surface (no autofill/`ExecCommand`-class API). **Closed, not open:** CDP `Input.dispatch*` was ruled out on 2026-11-13 — no CDP in the Interact or Commit path at any release — so nothing re-opens this rank. | Fold ranks 1-2 into "browser-process semantic action executed through the probe, with the page's own declared structured tools preferred when present". |
| D-16 | ACT-07 L790 | "Unexpected origin, dialog, permission, or state transition stops the task" | **No page-dialog event exists in this SDK**: `DialogStarting`, `DialogClosing`, `HasDialog`, `FrameDialog` → 0 matches in bindings.rs. | Replace "dialog" with "a page-reported modal change or a wedged renderer (process-failed)", and forbid Release-0 commands that can raise a script dialog. |
| D-17 | §2.3 L174-184 + L205 | the 11-event list, and "this lifecycle must use browser-process navigation events rather than renderer URL polling" | True for 7 of 11. `HistoryStateChanged`, `SelectionChanged`, `MeaningfulContentChanged`, `ViewportChanged` have no embedder equivalent and must come from the observer. | Tag each event **browser-process** or **observer-derived (untrusted)**; keep L205 for identity-bearing events only, and state that observer-derived events can change *content* classification but never *identity*. |
| D-18 | CTX-06 L758, §4.2 L437-438 (tiers 7-8 today, **6-7 after D-01's renumber**) | "Add PDF, transcript, structured-data … extractors" | The runtime's built-in PDF viewer exposes no content API to the embedder, and there is no caption/transcript API — those tiers have no source. | "PDF: detect via content-type and either parse a downloaded copy in the browser process or hand off to a viewer (no in-viewer text extraction available to an embedder). Transcripts: only from page-visible transcript text or a media element's own `<track>` data; no closed-caption or DRM text-track access." |

## 3. One delta per contradiction — where the backlog disagrees with itself

| # | Sections | The contradiction | Fix |
|---|---|---|---|
| 1 | Exec direction L5 vs the kernel decision | "Build a **thin Chromium fork**… Do not develop a new rendering engine." The kernel decision keeps the second half and drops the first. | L5 → "Build on an existing production rendering engine (the WebView2 runtime, driven by our own embedder shell). Do not develop a new rendering engine." |
| 2 | §3 L282 / L297 vs reality | diagram asserts Mojo plumbing and an embedder-visible AX tree | D-03; the AX box is deleted outright by D-01 (out of scope), not relabelled |
| 3 | §4.2 L441 vs the ruling | L441 hedges ("demonstrates useful … surfaces") where there is now a hard policy: one read-only method, off by default, nothing may depend on it | **D-21** replaces the sentence; §4 below records the decision it encodes |
| 4 | §7.1 vs §9 vs ACT-01 vs §12 on scroll | D-07 | one sentence |
| 5 | §2.5 L269 vs the shipped shell | D-05 | restate the requirement in authority terms (also discharges the accessibility word — see D-20) |
| 6 | SHL-01 L743 vs the scope decision | a P0 acceptance list demands accessibility + forced-colors conformance after accessibility was ruled out | **D-20** |

## 4. Not a delta: the CDP ruling, as it now stands (2026-11-13)

**Ruled.** CDP is reachable from the embedder host — `ICoreWebView2::CallDevToolsProtocolMethod`
(bindings.rs:**1897**, on the **base** `ICoreWebView2` vtable at 1771, so no QI and no debugging port is
a documented prerequisite) — and is **permitted for read-only extraction only**.

| Term | As ruled |
|---|---|
| Permitted | **`DOMSnapshot.captureSnapshot`** — one method |
| Withdrawn | `Accessibility.getFullAXTree` — accessibility is out of scope (§1b), so we do not call the `Accessibility` domain at all |
| Forbidden | every state-mutating method; **no CDP in the Interact or Commit path at any release** without a new ruling (this is what keeps C-74, C-90, C-91 at **d**) |
| Never | a public or product-facing API; nothing in a story, acceptance criterion, extension, or tool surface may depend on it |
| Stability | pin the domain + parameter shape per runtime version, feature-detect at startup, degrade to the main-world probe with the typed `unsupported` failure, **never break a page load** |
| Default | **off**, runtime-read flag, fails to disabled (same rule as C-80's kill switch) |

**Why the one remaining method still earns its keep without AX:** `DOMSnapshot.captureSnapshot` is the
program's **only non-page-authored read** — structure, geometry, and renderer-computed visibility come
from Blink's own serialisation instead of from a probe the page can shadow or flood. That is the entire
argument, and it never needed accessibility. Full case: `FORK-OR-CDP.md` §1b.1, **"Still buys"** rows;
effect on the forgeability rows: D-11's addendum.

**Measured 2026-11-13, and the method is worth what the ruling assumed** — but only for an analyzer that
knows three things the docs do not say. The bridge **answers**: completion `S_OK`, 2.2 ms, 5,809 B,
**35 DOM nodes with 23 layout rects and 21 per-line `textBoxes`**, on a normal https origin with DevTools
never opened, `AreDevToolsEnabled=false`, and none of wry's browser arguments changed. The traps:
1. **A synchronous `S_OK` is not an acknowledgement.** Branch on the completion result only.
2. An **invented** method returns `S_OK` synchronously, then fails at completion with `0x80070057` /
   CDP `-32601` — so a silent implementation that never checks completion believes it succeeded.
3. This runtime returns the **legacy columnar** payload (`documents[0].nodes` as parallel arrays plus a
   shared `strings` table), **not** the `nodeTreeSnapshot` shape current CDP docs describe. An analyzer
   written against the docs **prints 0 nodes against a payload holding 35** — a false "this page is empty"
   straight into the assistant's context. `textBoxes.start`/`length` are **UTF-16 code-unit** offsets.

**Still open:** V16's *second* half — the call is proven on runtime 157.0.4341.57 but nothing yet shows
it survives an Evergreen update; V18 (the degrade path with a typed `unsupported`) is untested; and the
probe is **untracked in git**, so this result is not reproducible by a third party today. Also open, and
unchanged by the measurement: MS docs state CDP calls **"may be processed out of order"**, so even a
permitted read needs a serialising queue and per-call correlation in our process.

**Net effect on this document:** D-01 and D-13 no longer have a CDP branch; D-15's rank 1 is closed out
rather than left open; D-11 stands unchanged; D-21 is the only delta the ruling creates.

## 5. Deliberately not proposed here

- Any change to §1, §2.2, §2.4, §4.1, §5, §6, or Epic E/G/I beyond the rows above — nothing in them is
  kernel-dependent enough to warrant editing. (§1's only edit here is D-20's **Non-goals** block, which
  exists so the accessibility decision cannot be quietly re-litigated by the next contributor.)
- Dropping **keyboard navigation or IME** for our own chrome. The accessibility decision does **not**
  authorise that; it is majority-user input quality (C-42), and the user **confirmed it directly when
  asked** (2026-11-13). No delta in this file touches it, and D-20's Non-goals block names it as
  explicitly in scope so no one infers the opposite from the accessibility cut.
- The `DocumentKey` shape (§2.3 L159-166): `frameTreeNodeId: number` is Chromium-specific naming, but the
  shipped contract's equivalent works; renaming is cosmetic and is Bob's call with the contract tests.
- Story-size re-estimates. Several P0s change *kind* here (FND-05 L→S, SHL-02 loses an impossible
  obligation and gains a test corpus), but I have not re-scored effort — that needs the people who own
  the stories.
- Anything in `STATUS.md`, the backlog itself, or code. Write scope for this work is `ai-browser/docs/**`.
