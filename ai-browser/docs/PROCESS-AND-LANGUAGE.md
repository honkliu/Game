# The AI utility process — a separate executable beside the shell

bindings.rs line numbers refer to webview2-com-sys-0.39.1 and were grep-derived on 2026-11-13 (the **vtable-slot** line inside `I…_Vtbl`, not the safe-wrapper line).

Backlog §3 requires a process that owns model calls and never touches tabs. Under the fixed kernel
(WebView2 runtime, our Rust shell is the embedder host) this is **simpler than it was under the
fork plan**: we do not need Chromium's child-process infrastructure, we need an OS process we spawn.

Void from the previous version of this document: `services/ai/`, `content::UtilityProcessWorker`,
Mojo, the `network::` service, and the "does in-tree Rust build on MSVC" gate. **There is no tree.**

## 1. Shape

```
native-browser shell (Rust, tao/wry, owns WebView2 environment)
  ├─ tab controller HWNDs ── page content (untrusted)
  ├─ overlay controller HWND (host-owned, transparent, §2.1 of FORK-OR-CDP.md)
  ├─ ai::utility_client  ── framed IPC over a private channel ──┐
  └─ ai::policy, ai::context, ai::store                        │
                                                               ▼
                                   ai-utility (separate executable, own heap, own socket)
                                     provider adapters · router · compaction
                                     structured-output validation · (local inference)
```

Two instances behind one client interface, because their resource profiles are incompatible:

| Instance | Contents | Lifecycle | Ceiling |
|---|---|---|---|
| `ai-utility` | providers, routing, prompt assembly, output validation, compaction | started with the profile, restartable, stateless | RSS ≤ 150 MB |
| `ai-inference` | local runtime + weights + tokenizer | lazy, on demand, evicted under memory pressure, never started unless a route needs it | user-set, default 4 GB; killed on idle > 5 min |

One instance would let a multi-GB model load stall a selection-level question — the §3 "nothing long
blocks the browser" violation, merely relocated out of the UI process. The shell enforces the
ceilings: it owns the process handle, so it can watch RSS and kill on breach.

## 2. Language — Rust vs C++ for a standalone executable

The old recommendation (C++) rested on three things that no longer exist: living inside the same
source tree as the browser's network service, native Mojo, and "one toolchain with the fork". With no
tree, **C++ reuses nothing from this program and adds a toolchain the program does not otherwise
need**; Rust is already mandatory for the shell. The argument inverted, so the recommendation
inverts with it. (Read §2b before repeating any fork-era claim about inheriting the browser's
network stack — **we do not inherit it.**)

Measured facts behind the reuse claim, re-checked today so nobody re-litigates it:

| Check | Result |
|---|---|
| `#[test]`/`#[tokio::test]` in `native-browser/crates/browser/src` | **118** (grep, 2026-11-13) |
| Model-provider code in `native-browser/**.rs` | **none** — grep for `struct .*Provider\|trait .*Provider\|fn chat\|completion\|llm\|api_key\|Authorization` → 1 hit, a COM comment in `webview2.rs` |
| Reusable as code | `store.rs` (one `rusqlite::Connection`, `busy_timeout(2 s)`, row cap, in-memory ctor for tests), `ipc.rs` (typed envelope), `settings.rs`, `downloads.rs` (Windows filename sanitiser), `omnibox.rs` (search URL template). Design + test vectors, **not** provider plumbing |

| Dimension | Rust | C++ |
|---|---|---|
| Toolchains to install and keep in sync beyond the shell's | **0 extra** | +1 full C++ build for this exe (MSVC is present, but a second project system, second third-party story, second CI job) |
| Contract sharing with the shell | **one `ai-contracts` crate**, types compiled on both sides; a malformed message is a type error, not a runtime surprise | shared **schema file + generated code** in two languages; drift is possible and shows up at runtime |
| Existing code that compiles into it today | the leaf modules above, unchanged | none |
| Parsing untrusted model output + untrusted page text (§8's whole risk surface) | **memory-safe by construction**; `serde` + a schema gives bounded, typed deserialisation | hand-managed bounds in the exact process that eats hostile text |
| HTTPS/SSE streaming | `reqwest`/`hyper`/`rustls`, or `winhttp` via the `windows` crate already pinned at `=0.62.2` | WinHTTP + hand-rolled SSE, or libcurl; own cert handling either way |
| SQLite | `rusqlite` already vendored + `bundled`, already building on this box | SQLite amalgamation in the build |
| Local model runtime (ONNX Runtime / llama.cpp, both C APIs) | `*-sys` bindings over the same C API; the `windows` crate is already a dependency at the version the WebView2 bindings need | direct FFI, marginal edge |
| Crash handling / diagnostics | must be added to this exe; **the shell must treat its exit as a typed `unavailable`** — and the same requirement lands on C++ | same |
| Hiring/fit | the shell is Rust; the person on this is a C++ systems engineer who will read either — noted, not decisive | — |
| **Net** | **wins**: no added toolchain, contract compiled once, hostile-input parser memory-safe, real reuse | no remaining advantage now that there is no tree to live in |

**Recommendation: Rust**, as a second binary in the existing workspace, with the message types in a
shared `ai-contracts` crate so the shell and the utility cannot disagree by accident.

Mandatory regardless of language:
- The utility process gets **no** file handles into the profile directory, **no** listening socket,
  **no** inherited WebView2 handles.
- Its egress is **only** to origins the shell's policy granted for that task. It owns the only model
  socket in the program, which makes our kill switch the one that actually works (§8).
- Its crash is a typed failure surfaced in the context header, never a silent spinner.
- Every parser over model output or page text is fuzz-tested with a corpus and runs under the
  address sanitizer in CI.

## 2b. Provider egress is OUTSIDE the browser's network stack

Any fork-era claim of the form "provider calls go through the browser's network service, so we
inherit one TLS stack plus enterprise proxy and policy" is **void and impossible on this kernel**.
The stack that loads pages belongs to the **WebView2 runtime — Microsoft's process**, not ours. Our
AI utility process is a separate executable that opens **its own socket, from our code**. Provider
traffic never traverses the stack that serves the page, and we cannot route it there: no embedder API
hands us the runtime's network service.

Therefore we do **not** inherit:

| Not inherited | What that costs us |
|---|---|
| Enterprise proxy configuration — and **WinHTTP does not read the browser's** | Only what the AI process reads for itself. **WinHTTP keeps its own proxy configuration store**, separate from the user's WinINet settings under `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` that the page's traffic uses. A fresh machine leaves WinHTTP at **direct access**; `netsh winhttp import proxy source=ie` exists precisely because the two stores do not track each other. Enterprises commonly push WinHTTP's value by GPO for services, but that is neither automatic nor the browser's setting. So `winhttp` via the pinned `windows = "=0.62.2"` reads *WinHTTP's* config (`WinHttpGetProxyForUrl` for PAC/WPAD); `WinHttpGetIEProxyConfigForCurrentUser` is the **deliberate opt-in copy** of the browser's setting, and it exists because that copy is not automatic. A pure `hyper`/`rustls` client reads nothing at all. Net: provider egress may sit on "direct access" while pages are happily proxied |
| System-level TLS / certificate policy the runtime applies for pages (enterprise-issued roots as the runtime presents them, TLS version and CT policy, cert pinning behaviour) | We validate certificates ourselves or via the OS store through our own code path. A corporate TLS-intercepting proxy with an enterprise root must be trusted **by our process**, deliberately, and its absence must produce a typed failure rather than a hang |
| Per-origin rules the runtime enforces on page traffic (CORS, SameSite, mixed-content blocking, storage partitioning, ORB/content-type sniffing) | Those guard pages against each other; they do not guard us. Our only equivalent control is our own origin allowlist per grant — which is therefore load-bearing in a way it was not under the fork assumption |
| Shared network telemetry | No HAR, no "inspect the page's traffic" view covers a provider call. Provider egress gets its own accounting in `ReportTelemetry` |

### Consequences, as requirements

1. **Corporate proxy needs its own detection, its own override, and its own failure cause. This is a
   known deployment requirement, not a TODO.** Provider egress honours **WinHTTP's own** per-machine /
   per-user proxy configuration, which is *not* the setting the page's traffic uses and may read
   "direct access" while the browser is happily proxied. Therefore: (a) read WinHTTP's config at request
   time (`WinHttpGetProxyForUrl`, PAC/WPAD); (b) expose an explicit override in AI settings plus a
   deliberate "copy the browser's (WinINet) configuration" switch, since that copy is not automatic;
   (c) return typed `unavailable` naming **`winhttp proxy not configured`** as a distinct cause, never a
   silent timeout, which users read as "the model is slow". Detection must be able to *report* "direct"
   as a finding rather than assume no proxy is needed.
   *Verification honesty:* the divergence is stated from WinHTTP's API design and from the existence of
   `netsh winhttp import proxy source=ie` — **not from a measurement.** On the machine these docs were
   written on (Carol's run) `netsh winhttp show proxy` reports `Direct access (no proxy server)` and
   `ProxyEnable` is `0x0`, so both stores happen to be direct and the divergence cannot be demonstrated
   there; it needs a proxied network (L6).
2. **The network kill switch is process-level.** Terminate the AI executable: it holds the only model
   socket in the program (`supervise::abort()` → EOF → 500 ms grace → `kill()` → reap, already
   implemented and tested in `foundation/src/supervise.rs`). Backlog §7/§8 wording about blocking AI
   egress at the navigation layer **does not hold here**. Navigation-layer interception is best-effort
   by Microsoft's own documentation — *"For performance reasons, `GET` HTTP requests may happen,
   while the host is responding."* — so `AddWebResourceRequestedFilter` (1969) is defence-in-depth
   for *page* traffic and must never be described as the AI kill switch.
3. **`--offline` build viability is a real constraint.** Everything in Epic A builds offline with
   `serde` + `serde_json` only. Adding `reqwest`/`hyper`/`rustls` means new crates fetched from
   crates.io. If the R0a proof must stay offline-buildable on this machine, provider HTTP goes
   through `winhttp` via the already-pinned `windows` crate — **no new dependency, and that is the
   whole argument for it.** It buys no proxy inheritance (requirement 1), so it does not remove the
   configuration work; it only removes a fetch. Decide this before the first provider lands, not after.

## 3. Transport — decided by Bob; these are the conformance requirements

**Decided** (see `../foundation/README.md`, "Contract format and transport decision"): hand-written
Rust types in `ai-platform-contract` are the source of truth; the wire is **one JSON object per line
(NDJSON) over the utility's stdin/stdout**, spawned with `Stdio::piped()`. Loopback TCP and localhost
WebSockets are ruled out **by policy** — pages we render can reach local ports — which is the same
invariant this document asserted independently; both endpoints share `struct` definitions, and
`frame::decode` gates by `Peer` before any payload is interpreted. Named pipes with a per-instance
ACL are the recorded upgrade path if a utility must ever outlive a shell restart; the frame codec is
transport-agnostic and will not change when that happens.

What follows is therefore a **conformance checklist** for the chosen transport, not an open choice.
R1-R10 are unaffected by §2b.

> **No loopback TCP listener.** A port on `127.0.0.1` is reachable from web content our shell is
> rendering — any page can attempt a connection to a local port, and DNS-rebinding/CSRF patterns
> against local helper servers are a known class. The AI executable is spawned by us, so we never
> need it. If a candidate transport is a local HTTP server, it fails this requirement.

| Requirement | Why |
|---|---|
| R1 Private channel established at spawn (stdin/stdout pipes, or a named pipe with a per-user DACL) | the only process that may talk to it is our shell |
| R2 Authenticated hello: per-spawn random token + protocol major/minor, both sides fail closed on mismatch | prevents a same-user process impersonating the shell and gaining capabilities |
| R3 Length-prefixed frames, 4- or 8-byte header, hard max frame size, and chunking above it | §8 bounded output; oversized frames are a hostile-input surface |
| R4 Sustained streaming for minutes, ≥ 200 msg/s, with backpressure that cannot deadlock the shell UI thread | token streams; the shell must stay at 60 fps while tokens arrive |
| R5 Out-of-order responses correlated by `requestId`; **unsolicited** utility→shell messages allowed (progress, tokens) | streaming + concurrent tasks |
| R6 Cancellation is a frame, and **channel EOF is cancellation**: if the shell dies, the utility must stop spending tokens within 100 ms | §11 cancellation; a runaway agent is the worst failure mode we can ship |
| R7 Deadline propagation in the envelope, non-extendable by the utility | §7.3 |
| R8 One process per instance, shell-owned handle, RSS watchdog, restart-at-most-once then fail closed | §1 ceilings |
| R9 No ambient authority in the transport: no inherited env vars carrying profile paths, no inherited handles beyond the channel | least privilege |
| R10 Locally benchmarkable: a 2 MiB blob round trip and a 10k-message stream are both in the transport conformance test, with numbers recorded | so "fast enough" is a measurement, not an opinion |

Candidates, unranked, with the property each one has to prove:

| Candidate | Must prove |
|---|---|
| stdio (`CreateProcess` anonymous pipes) | R4 without pipe-buffer stalls; R6 half-close detection is prompt |
| Named pipe (`\\.\pipe\…`) | DACL really blocks other same-user processes; overlapped I/O throughput; disconnect detection |
| Local TCP on 127.0.0.1 | **fails R1/R2 unless the port is unguessable *and* the listener binds after drop-privileges *and* web content is proven unable to reach it. Recommend he rule it out early** |
| Anything shared-memory (`FileMapping` + events) | still needs a control channel underneath; complexity only if R3/R10 numbers demand it |

Alice's boundary duty stands whatever he picks: the **semantics** below are enforced above the
transport, so a transport swap must not change them.

## 4. The IPC boundary

Envelope on every message:

```
{ taskId, requestId(seq), grantEpoch, documentToken?{tabId,frameId,navigationId},
  snapshotVersion?, deadlineMono, schemaVersion }
```

No request is served without `taskId` + `grantEpoch`. Every reply is `ok | error`. Errors are the
FND-04 taxonomy and nothing else: `unavailable | blocked | stale | cancelled | timed_out |
unsupported | output_limited`.

**Alignment with the shipped crate.** The method names below were drafted before
`ai-platform-contract` existed. **`foundation/src/frame.rs::Method` is the authoritative closed
list** — seven methods, each with one legal issuer: utility → `context.get_snapshot`,
`action.dispatch`, `task.progress`, `task.finish`; shell → `model.complete`, `task.cancel`,
`utility.ping`. `task.progress` **and** `task.finish` are the notifications (`is_notification`), not
just progress. Mapping onto that list, plus one gap:

| Row below | Real method | Note |
|---|---|---|
| `ReportProgress` | `task.progress` | carries streamed tokens; also `task.finish` for the terminal state |
| everything else in 4.1 | **no method exists** | `GetSnapshot` is the only utility-initiated read in the closed list. Citation resolution, workspace listing and store access have **no method**; **selection does not need one** — it rides in params, per the ruling below |
| `ProposeAction` | `action.dispatch` | it exists in the codec and is described as "the *only* way anything reaches a page". **Release-0 denial is therefore not method absence** — it is `ActionLevel::allowed_in_release_zero()`, which must reject every level whose effect reaches a page. Scroll is classed `Navigate` and is allowed (decided 2026-11-13) |

**Ruled (Carol, 2026-11-13): no new `frame::Method` for selection or answer artifacts.** They belong in
**params**, not in a method name:

| Direction | Rides on | Params carry |
|---|---|---|
| shell → utility | `model.complete` (issuer `Peer::Shell`) | the user's question, the target `DocumentToken`s, and the optional selection |
| utility → shell | `task.finish` (notification) | the answer and its citations; streamed tokens go on `task.progress` |

The open work is therefore **two payload *types* in `foundation/src/contract.rs`, not a method**:
(1) a **task request** — question + target tokens + optional selection + `snapshot_version`; (2) an
**answer with citations**. `contract.rs` today ships `ContextSnapshot`, `ContextRef`,
`ConversationBinding`, `TaskProgress`/`TaskStage` and `ActionRequest`, and **nothing that names a model
request or an answer**, so R0a-6 cannot type-check until Bob adds those two types. The closed list stays
at 7 methods and the `Peer` side-gates stay untouched.

### 4.1 The utility process MAY request (Release 0 allowlist — exhaustive)

| Method | Returns | Constraint |
|---|---|---|
| `GetSnapshot(documentToken, opts)` | bounded `PageSnapshot` | token must be in the task's grant set; chunked; `snapshotVersion` echoed |
| `GetSnapshotDelta(doc, baseVersion)` | `SnapshotDelta` | Release 0 returns `unsupported` (deltas deferred) |
| `GetSelection(documentToken)` | quote, prefix, suffix, rects | same document only |
| `FindInSnapshot(requestId, query, limit)` | `blockId[]` + scores | read-only, `limit ≤ 200` |
| `ListWorkspaceContext(workspaceId)` | `{tabId, doc, title, origin, coverage, opaqueHandle}[]` | **metadata only** — never body text of a non-granted document (§8) |
| `ResolveCitation(requestId, blockId)` | quote + rect | Release 0: no scroll action; geometry is for display only |
| `ProposeAction(req)` | `PendingApproval` id — **never performs** | Release 0: always `blocked` |
| `ReportProgress(requestId, phase, tokenCount)` | — | ≤ 20/s; no page text |
| `ReportTelemetry(metrics)` | — | counters, durations, enum outcomes only. No URL query, no text (§8) |
| `RequestModelPolicy(taskKind, origin)` | allowed providers, local-only flag | shell decides, utility obeys |
| `StorePut/StoreGet(namespace, key, blob)` | scoped KV | namespace allowlist + per-namespace quota; **the shell performs the write**, the utility holds no file handle |

### 4.2 It MAY NEVER request

| Denied | Defends against |
|---|---|
| Navigate, open/close/switch tab, change focus, resize, window ops | §3 "no direct authority over tabs"; §2.4 uncontrolled auto-open |
| Execute script, mutate DOM, synthetic input, clipboard write, download, file picker | §9 "no page mutations" in Release 0; §7.1 approval classes later |
| Read cookies, localStorage, sessionStorage, IndexedDB, credentials, auth headers, history, other profiles | §8 "never collected" — and note `wry` exposes `cookies()`/`set_cookie()` on the shell side, so **the shell must never proxy them** |
| Enumerate tabs/workspaces outside the granted `workspaceId`; body text of a non-granted document | §8 leakage; the §11 "no cross-workspace leakage" gate |
| Fetch an arbitrary URL, or any URL named by model output or page text | prompt injection → SSRF → exfiltration. Model output is data, never a capability |
| Spawn processes, open files by path, create listeners, load libraries | sandbox integrity; §8 no persisted screenshots |
| Persist outside `StorePut` namespaces; write to "personal memory" without explicit user consent | §8 storage defaults |
| Use a `grantEpoch` the shell has advanced, or a `documentToken` outside the grant | §5 stale rules |
| Subscribe to raw event streams (`SelectionChanged`, `ViewportChanged`) | unbounded context creep; context arrives only via explicit reads |
| Assume a reply will be delivered after its document died | §5 "target a stable handle, never the active tab resolved earlier" |

### 4.3 Grant model (what makes 4.1/4.2 enforceable, not aspirational)

| Rule | Value |
|---|---|
| Issued on user intent | `{taskId, workspaceId, documentToken set, snapshotVersion ceiling, allowlist id, deadline, token budget, byte budget}` |
| TTL | ≤ 10 min or the task deadline, whichever is sooner |
| Revoked on | navigation commit (`NavigationId` change), frame destroyed, tab closed, workspace change, user Stop, policy change → **epoch bump**; every in-flight request fails `stale`, every stream closes |
| Capability indirection | requests carry tokens; the shell resolves token → controller/frame and **re-verifies at dispatch** |
| Frame size / rate | ≤ 2 MiB per reply, ≤ 256 KiB per snapshot chunk (exceed → `output_limited` + `truncatedFrom`); ≤ 20 req/s/task |
| Backpressure | one outstanding snapshot read per task; more queue, never fan out |
| Cancellation | `Cancel(requestId)` idempotent → exactly one terminal state; no late reply surfaces |
| Startup | client is `null` until hello succeeds; restart once on crash, then fail closed with `unavailable` shown in the context header |
| Observability | `(taskId, method, doc, snapshotVersion, outcome, durationUs)` per call, **no payloads** — and a test that greps emitted events for corpus text |

### 4.4 Threat model

| Threat | Boundary that stops it |
|---|---|
| Model output instructs us to open/fetch/act | `ProposeAction` only; no fetch of model-named origins; Release 0 deny-all |
| Page text carries an injection payload that becomes an instruction | provenance labels (SAF-01) + typed structured output + egress allowlist |
| Compromised page sends fake rects or fake content | identity from shell events only; nothing security-relevant positioned from renderer geometry (`FORK-OR-CDP.md` §3.2/§4); probe output treated as untrusted |
| Stale answer applied to a new document | `grantEpoch` + `snapshotVersion` ceiling + revoke-on-commit |
| Compromised utility process | no capabilities beyond 4.1; no file handles; no listeners; killable; writes only via quota'd namespaces |
| Transport impersonation | R1 private channel + R2 per-spawn token + protocol version, fail closed |
| Audit log leaks content | schema has no payload fields; SAF-04 test over the corpus |

## 5. Inconsistencies in §3/§6 as written

1. **§3's diagram collapses two components into "Context Service"** while its text puts probes in the
   renderer and forbids long work in the UI process. Split into **Context Coordinator** (shell) and
   **probe** (page, main world, untrusted).
2. **"Local model execution where practical" has no resource contract and no story.** Epic E has no
   MOD-* story that runs a model locally and no runtime is chosen. Until one exists, `private_page →
   require_tags: [local-execution]` (§6.2) must resolve to **`blocked`**, not to a cloud fallback —
   otherwise the routing table advertises a privacy guarantee the build does not have.
3. **§8's storage defaults name owners, not the writer.** This spec assigns every profile write to
   the shell, which is the only component that can make "raw screenshots not persisted" a rule rather
   than a habit.
4. **"Isolated IPC" is undefined in §3.** §4 is the definition. If the intent was OS-level sandboxing
   of the utility process (job object, restricted token, no filesystem), say so — that is a real
   deliverable and it is not free; recommend it for Release 1, behind the Release-0 deny-all.

## 6. `verify`

| # | To verify | How |
|---|---|---|
| L1 | Transport numbers against R4/R10 | benchmark once Bob lands a candidate: 2 MiB blob round trip, 10k-message stream, cancellation latency at 50% CPU load |
| L2 | Actual per-message and per-connection overhead of the chosen transport | measure, don't assume; it sets the snapshot chunk size |
| L3 | `rusqlite` `bundled` build cost inside a **second** binary in this workspace | `cargo build --release -p ai-utility` on this box; it is the only C compilation we already have |
| L4 | Whether the shell can hold model egress while still letting pages load (i.e. our kill switch is real) | block all utility egress, confirm browsing unaffected and the context header says `blocked` |
| L5 | Job-object RSS enforcement actually kills rather than throttles | run the utility under a job with a 150 MB limit and a synthetic allocation |
| L6 | **That the two proxy stores really diverge** — argued from API design, never measured here | on a proxied/domain-joined network: set the WinINet (browser) proxy, leave WinHTTP at its default, and confirm `WinHttpGetProxyForUrl` reports **no proxy** while pages load fine through the browser's proxy; then `netsh winhttp import proxy source=ie` and confirm the provider call starts working. On this box both stores are direct (`netsh winhttp show proxy` → `Direct access (no proxy server)`, `ProxyEnable` = `0x0`), which demonstrates nothing either way |
