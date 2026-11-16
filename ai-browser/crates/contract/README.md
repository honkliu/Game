# `ai-platform-contract` — Epic A foundations (FND-01, FND-02, FND-04)

Rust crate holding the contracts the rest of the AI-native browser is written
against: the document identity model, the versioned message envelope, and the one
typed failure taxonomy. Spec: `../../ai-native-browser-design-and-engineering-backlog.md`
§2.3, §4.1, §5, §7; story list in §10 Epic A.

Kernel decision this was rewritten for (2026-11-13): **the existing WebView2 runtime
driven by a Rust shell. No Chromium checkout, no fork, no depot_tools, no Mojo.**

## Layout

| Path | Backlog | Contents |
|---|---|---|
| `src/identity.rs` | FND-01 | `ProfileId`, `WorkspaceId`, `TabId`, `DocumentToken`, `DocumentKey`, `SnapshotVersion`, `TaskId`, `DocumentIdentityRegistry`, ownership rules |
| `src/version.rs` | FND-02 | `SchemaVersion`, the major-version gate |
| `src/contract.rs` | FND-02 | context / action / progress / cancellation / anchor payloads, action-level classification, the assistant round trip (`TaskRequest` → `TaskOutcome::Answered` / `NoAnswer`) |
| `src/failure.rs` | FND-04 | `FailureKind` (8), `Failure`, `OutputLimit` |
| `src/frame.rs` | FND-02/03 | frame envelope, closed method list, size + version + **side** gates, codec |
| `src/supervise.rs` | §7, ACT-06 | spawn, cooperative cancel, forced kill, reap, abandoned-work report |
| `src/bin/stub_utility.rs` | — | test double of the AI utility process (not a provider) |
| `tests/*.rs` | all | 68 tests; plus 3 `compile_fail` doctests (1 in `src/identity.rs`, 2 in `src/contract.rs`) |
| `tests/fixtures/*.json` | FND-02 | 3 golden wire lines: the request, an answer whose second citation is stale, a no-answer |

## Run the tests

```bat
cd /d Q:\gitroot\Game\ai-browser\foundation
set CARGO_INCREMENTAL=0
cargo test --offline
```

`--offline` is deliberate and load-bearing: the only dependencies are `serde` and
`serde_json`, both already in this machine's cargo registry cache and both already
pinned by `crates/shell`. Nothing here downloads or installs.

Last run: **71 passed, 0 failed** (68 integration across 6 files + 3 `compile_fail` doctests), via
`cargo test --offline`. **`cargo clippy --offline --all-targets -- -D warnings` is clean.** It was
not: clippy 1.99 surfaced four hits, all mechanical and none in code this task wrote —
`src/supervise.rs:224` plus `tests/identity_contract.rs:75` and `:387`
(`needless_borrows_for_generic_args`: `to_value(&x)` → `to_value(x)`; `CancellationRequest` and
`DocumentKey` are both `Copy`, so dropping the borrow is free) and `tests/identity_contract.rs:392`
(`bool_assert_comparison`, now `assert!(!…, msg)`). Two of the four were named in the work order; the
other two were in the same file and the same lint, and `-D warnings` cannot go green with them left,
so they went too. The crate's own lint gate (`unwrap_used = deny`) is clean.

## Contract format and transport decision

**Decision: hand-written Rust types are the source of truth; the wire encoding is one
JSON object per line (NDJSON) over the AI utility process's stdin and stdout.** Both
endpoints compile against the *same* `struct` definitions, so the two ends of the
boundary cannot hold different ideas about a message — a mismatch becomes a compile
error in whichever crate is wrong, not a runtime parse surprise. The alternatives and
why they lost: *protobuf/capnp* buy a codegen toolchain (`protoc`/`capnp`) and two
generated types that merely agree, in exchange for a compactness this channel does not
need — the payloads are page text and small JSON, kilobytes, not tensors; and
`schemars`-style schema-first generation would put a contract in a file that the
compiler does not read. *Loopback TCP or a localhost WebSocket* is **ruled out by
policy, not preference**: pages rendered inside our own shell can reach local ports, so
a loopback listener would place the model channel inside the attack surface of every
site we open (`supervise::spawn` opens only `Stdio::piped()` — no socket exists to
connect to). *Named pipes with a per-instance ACL* are the upgrade path if the utility
process ever needs to outlive a shell restart or be attached by a second window
process; the frame codec above is transport-agnostic and does not change when that
happens. The cost of the choice is honesty: JSON is runtime-typed, so the authority
boundary is **not** carried by the format. It is carried by two things the format cannot
weaken — `DocumentKey` has a private constructor and no `Deserialize` impl (see the
`compile_fail` doctest), so a peer cannot put one in a message at all; and
`frame::decode` takes the `Peer` established when the pipe was brokered and refuses any
method that side may not issue, before the payload is interpreted. Mojo gave the second
property for free through interface binding; over a byte pipe it is one explicit gate
at the single chokepoint every inbound frame passes, which is a smaller surface to
audit than a generator and a validator chain.

## Where the safety actually lives

Four properties, each enforced rather than documented:

1. **Identity is minted, never asserted.** `DocumentKey` is unconstructible outside the
   registry (`compile_fail` doctest) and not deserialisable; a peer sends a
   `DocumentToken`, which is a forgeable integer that resolves to nothing.
   `tests/identity_contract.rs` covers substitution across workspace, tab, and
   navigation, plus the URL-as-identity criterion.
2. **A frame cannot name a method its side does not own.** `Method::issuer()` is one
   table; `tests/frame_contract.rs` proves the utility cannot issue `model.complete` or
   `task.cancel`, and the shell cannot issue `action.dispatch`.
3. **Rejection happens before interpretation.** Size → syntax → version → side → typed
   payload, in that order. `tests/version_contract.rs` pins the order with a frame that
   would fail typed parsing under the *supported* major and asserts the wrong-major case
   reports `schema.major_unsupported`, not a payload code.
4. **A citation cannot become evidence without being checked.**
   `AnswerWithCitations::verify` is the only source of `TrustedCitation`, and that type
   has no visible constructor (second `compile_fail` doctest) and no `Deserialize`. A
   citation whose `navigation_sequence` does not match the live document is a
   `Failure::stale` — which cannot be built without `replan_required = true` — rather
   than a bool a caller may not read. `tests/answer_contract.rs` drives it from the
   golden fixture, where one of the two citations quotes the document a `pushState` route
   change replaced.

## Cancellation and a stuck utility process

Cooperative first, forced second, and the forced path is not allowed to lose
information. `supervise::UtilityProcess::cancel` sends `task.cancel` and awaits a
`CancellationResult` that states `cancelled_at` and `actions_completed`; a reply of
`InFlightAction` sets `effect_unverified`, because the page may already have changed.
If the utility is wedged in a synchronous provider call it cannot read stdin, so
`abort()` closes our write half (EOF), waits `DEFAULT_KILL_GRACE` (500 ms), then
`kill()`s and reaps. Every unanswered call comes back as `AbandonedWork`:
`unavailable` / `utility.process_lost`, retryable, and `effect_unverified` for anything
that could have reached a page. Both paths run against a real OS process in
`tests/supervise_contract.rs`, including the uninterruptible one.

## Deliberate deviations from the backlog

These are places the contract is *not* what §2.3/§7.3 sketch, each with the reason in
the code:

- `DocumentKey` is never serialised inbound. The backlog's `DocumentKey` carries
  `profileId`/`workspaceId` as wire fields; as payload those are a substitution hole, so
  peers send a token and the scope comes from the brokered pipe (`identity::RequestScope`).
- `BrowserActionRequest` drops `approvalClass`. A requester that grades its own approval
  requirement can downgrade a `Commit` to an `Observe`; the shell decides, and reports it
  in `ActionDecision`.
- `upstream_navigation_id` records WebView2's `NavigationId` for audit but is not the
  sequence authority. See the module docs in `src/identity.rs` for the three reasons, and
  `tools/README.md` for the live-webview measurement that must happen before it is trusted.
- `ActionLevel::Navigate` includes host-driven **scroll** (backlog §9 vs §12 conflict,
  decided 2026-11-13), gated by `ActionLevel::allowed_in_release_zero()`.
- No accessibility tier anywhere. `IAccessible`/`Accessibility`/`uiautomation`/`UiaRaise`
  have zero matches in `webview2-com-sys-0.39.1/src/bindings.rs` (44,394 lines), so
  backlog §4.2 tier 2 and any `accessibilityPath` anchor field are absent rather than
  reserved; `TargetResolution::DomLocator` is DOM-only via `ExecuteScript`.
- The assistant round trip rides existing methods — `model.complete` params carry
  `TaskRequest`, `task.finish` params carry `TaskOutcome` — because `frame::Method` is a
  closed list and adding a method means touching the side gate for one payload's sake.
  `TaskRequest` carries a resolved `DocumentKey` and so is `Serialize` only; a derived
  `Deserialize` would be a back door that mints a key, since serde fills private fields
  from inside the defining crate. The utility reads its inbound copy as
  `TaskRequestView`, whose `document` is a `DocumentClaim`: same seven fields, same bytes,
  worthless to forge.
- `FailureKind` is **eight**, not the seven FND-04 lists, added after the provider
  endpoint was measured (2026-11-13): an 8-token probe of the served reasoning
  model returned `"content": ""`, a non-empty `"reasoning_content"`,
  `"finish_reason": "length"`, `usage.reasoning_tokens: 9`. `no_answer` is neither
  `output_limited` (an answer exists, truncated) nor `unavailable` (re-issuing could
  help). `TaskOutcome::NoAnswer` carries the budget and the reasoning token **count**, and
  there is deliberately no field anywhere that could hold the reasoning *text*.
- The golden fixtures are alphabetically keyed. `params` is a `serde_json::Value`, whose
  map is a `BTreeMap`, so `frame::encode` emits every payload's keys in sorted order —
  true of all frames, not just these. Anything hand-editing a fixture to declaration
  order will fail the byte-exact assertion.
- Architecture rule kept: no type here implies a per-annotation process or surface —
  one overlay controller per browser window.

## Known gaps (not implemented in this crate)

- **Orphan-proofing.** If the shell crashes, a spawned utility survives with a live
  provider session. Needs a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
  (`CreateJobObjectW` + `AssignProcessToJobObject`) at spawn; `windows = "=0.62.2"` is
  already pinned by `native-browser`, so no new dependency. Must land before unattended
  tasks. Not done here because a contract crate must not link `windows` to be spawnable
  in a test.
- **No JSON Schema publication.** A non-Rust consumer (the WebView2 UI, which speaks
  `postMessage`) needs `schema/v1/*.json`. Derivable from the types; deferred, and
  because it is deferred, the versioned *behaviour* is pinned by tests rather than by a
  published document.
- **The utility process itself does not exist.** `stub_utility` is a test double; there
  is no provider code anywhere in this repo.

## Superseded artifacts

`archive/fork-contracts-superseded-mojom/` (5 `.mojom` files) and
`archive/fork-contracts-superseded-cpp/` (identity + failure in C++) were written for
the Chromium-fork direction cancelled on 2026-11-13. They are **dead code**, kept only
because deletion was out of scope, and are not referenced by the crate or the build.
Mojo is unavailable to a WebView2 embedder, so the `.mojom` files cannot be compiled
here at all. The identity *design* survived; the transport and language did not.
