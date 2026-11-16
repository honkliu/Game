//! Versioned context, action, progress, and cancellation payloads (backlog
//! FND-02). The shapes come from backlog §4.1, §5, §7.1 and §7.3.
//!
//! Two rules govern which of these derive `Deserialize`:
//!
//! - Anything that carries a resolved [`crate::identity::DocumentKey`] is
//!   `Serialize` **only** (`ContextSnapshot`, `ActionResult`, [`TaskRequest`], and the
//!   verified outputs [`TrustedAnswer`] / [`TrustedCitation`]). The reason is not
//!   ceremony: a derived `Deserialize` impl is generated *inside* this crate and can
//!   therefore fill a private field, so making any of these deserialisable would be a
//!   working back door that mints a `DocumentKey` — and the `compile_fail` example on
//!   `DocumentKey` would stay green while the door was open.
//! - Everything else derives both, because an id it carries is an *assertion* the
//!   receiver re-checks against its own registry, not authority.
//!
//! [`AnswerWithCitations::verify`] is where the second rule pays out: an answer arrives
//! fully deserialisable, and every claim inside it is re-checked before it can become a
//! [`TrustedCitation`].
//!
//! Semantic content types (`SemanticBlock`, `LinkDescriptor`, `ControlDescriptor`)
//! belong to Epic C and arrive as `serde_json::Value` payloads here, so this
//! envelope can freeze now without pre-empting the extraction design.

use serde::{Deserialize, Serialize};

use crate::failure::{Failure, FailureKind};
use crate::model::{TaskFailed, TaskOptions};
use crate::identity::{
    DocumentIdentityRegistry, DocumentKey, DocumentToken, ProfileId, RequestScope, SnapshotVersion,
    StaleStatus, TabId, TaskId, UpstreamNavigationId, WorkspaceId,
};

/// Quantifies an `output_limited` failure: what the bound was and what was
/// emitted. Carried beside a bounded result rather than inside the failure, so a
/// truncated payload is not representable without its bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLimit {
    pub byte_limit: u64,
    pub emitted_bytes: u64,
    pub dropped_items: u64,
}

impl OutputLimit {
    pub fn truncated(&self) -> bool {
        self.emitted_bytes < self.byte_limit || self.dropped_items > 0
    }
}

/// Why a requested region could not be captured. The three kinds stay separate
/// because the remedy differs: an inaccessible frame needs a permission change,
/// an unloaded region needs scrolling, unsupported content needs an extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageGapKind {
    InaccessibleFrame,
    UnloadedRegion,
    UnsupportedContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGap {
    pub kind: CoverageGapKind,
    /// Developer-facing description only; never page text (backlog §8).
    pub reason: String,
}

/// Completeness claim for a snapshot. `CompleteEnough` is a statement about the
/// question being asked, not about the page, so the requester supplies it — the
/// extractor must not guess it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Partial,
    Updating,
    Blocked,
    CompleteEnough,
}

/// Backlog §4.1: never claim "the AI has read the whole page" when third-party
/// frames, unloaded content, canvas text, or media are inaccessible.
///
/// `PartialEq` but not `Eq`: `estimated_completeness` is an `f32`, and floating
/// values do not form an equivalence relation. Equality here is "these two captures
/// report the same numbers", which is all any caller needs; nothing may use this
/// type as a map key or dedupe on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCoverage {
    pub status: CoverageStatus,
    pub captured_frames: u32,
    pub inaccessible_frames: Vec<CoverageGap>,
    pub unloaded_regions: Vec<CoverageGap>,
    pub unsupported_content: Vec<CoverageGap>,
    pub has_untranscribed_media: bool,
    /// 0.0..=1.0. Producers emit a real estimate or 0.0; the value is shown to
    /// the user and must not default to a reassuring number.
    pub estimated_completeness: f32,
}

/// Utility → shell request for one document's current snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSnapshotRequest {
    pub document: DocumentToken,
    /// Re-checked against the brokered scope, not trusted from this field
    /// (`identity::RequestScope` is the authority).
    pub workspace: WorkspaceId,
}

/// Shell → utility snapshot. Carries a resolved key, hence `Serialize` only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextSnapshot {
    pub document: DocumentKey,
    pub snapshot_version: SnapshotVersion,
    /// Milliseconds since the Unix epoch, from the shell clock.
    pub captured_at_ms: i64,
    /// Metadata only. URL is explicitly not identity (backlog §2.3).
    pub url: String,
    pub origin: String,
    pub title: String,
    pub coverage: ContextCoverage,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<OutputLimit>,
}

/// Reference to a slice of a specific capture, stored with a conversation turn so
/// an old answer can be re-evaluated against current state (backlog §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRef {
    pub document: DocumentToken,
    pub snapshot_version: SnapshotVersion,
    /// Empty means "the whole snapshot".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_ids: Vec<String>,
}

/// The immutable context binding of one user turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationBinding {
    pub conversation_id: String,
    pub workspace: WorkspaceId,
    pub context_refs: Vec<ContextRef>,
}

/// Outcome of re-evaluating a [`ConversationBinding`] against live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleAssessment {
    pub status: StaleStatus,
    /// True when an action planned from this binding must be re-planned. The
    /// shell, not the model, is the authority on this bit.
    pub replan_required: bool,
}

/// The four capability levels (backlog §7.1). The numeric order encodes
/// consequence and is part of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionLevel {
    /// Read-only: read page, find passage, inspect links.
    Observe,
    /// Reversible: open, close, switch, preview, and **scroll**.
    Navigate,
    /// Page mutation: type, filter, select, expand.
    Interact,
    /// Consequential: submit, upload, add to cart, purchase.
    Commit,
}

impl ActionLevel {
    /// Backlog §9 ("no page mutations") and §12 ("citations scroll to the
    /// supporting passage") conflict. Resolved 2026-11-13: host-driven scroll ships in
    /// Release 0 as `Navigate`-class read-only, because it changes no page state and is
    /// trivially reversible, while a citation the user cannot verify in place is worth
    /// about half as much. This predicate is where that decision is encoded, so the
    /// dispatch gate is one comparison rather than a list each handler re-derives.
    pub fn allowed_in_release_zero(self) -> bool {
        self <= ActionLevel::Navigate
    }
}

/// Which level an action kind requires.
///
/// The mapping lives in the contract, next to [`ActionLevel`], and not in the action
/// runtime. A requester that could pick its own level could downgrade a `Commit` into
/// an `Observe` and skip the approval step; a registry that graded levels per handler
/// would drift into the same hole one refactor away.
///
/// An unrecognised kind is `None`, and dispatch must refuse it with
/// `unsupported/action_kind_unknown`. Defaulting an unknown kind to `Observe` — the
/// tempting reading of "we don't know what it does, it's probably safe" — is exactly
/// how a mutation would reach a page in Release 0.
pub fn level_of_action_kind(action_kind: &str) -> Option<ActionLevel> {
    match action_kind {
        "read_document" | "find_passage" | "list_links" | "describe_image" | "list_tabs" | "read_tab" => Some(ActionLevel::Observe),
        "scroll_to" | "open_tab" | "close_tab" | "activate_tab" | "preview_link" | "navigate" | "go_back"
        | "go_forward" | "reload" | "highlight_passage" | "show_candidate" => {
            Some(ActionLevel::Navigate)
        }
        "type_text" | "set_filter" | "select_option" | "expand_control" => Some(ActionLevel::Interact),
        "submit_form" | "upload" | "add_to_cart" | "purchase" => Some(ActionLevel::Commit),
        _ => None,
    }
}

/// How a target should be resolved, in the preference order of backlog §7.2. A
/// request that cannot name a semantic target must say so: coordinate fallback is
/// a visible decision, not an implicit degradation inside the runtime.
///
/// **Backlog §7.2 tier 3 is DOM-only.** The backlog pairs this tier with the
/// accessibility tree, and there is no accessibility-tree API for a WebView2
/// embedder: `IAccessible` / `Accessibility` / `uiautomation` / `UiaRaise` have zero
/// matches across the 44,394 lines of `webview2-com-sys-0.39.1/src/bindings.rs`
/// (verified 2026-11-13). The DOM half of the tier is real because it goes through
/// `ICoreWebView2::ExecuteScript`, so `DomLocator` stays and the accessibility half
/// is absent rather than reserved. A tier that can never resolve is a silent
/// fallthrough waiting to happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetResolution {
    BrowserSemanticCommand,
    PageStructuredTool,
    /// CSS/XPath, resolved by the shell via `ExecuteScript`.
    DomLocator,
    VisualTarget,
    CoordinateFallback,
}

/// The order a detached annotation is re-attached in, most trustworthy first. This
/// lives in the contract, not in the overlay code, because the *sequence* is a
/// versioned product decision: inserting a tier changes which answer a user sees.
///
/// There is no accessibility-path tier, for the reason given on
/// [`TargetResolution`]. `Detached` is terminal: the overlay stops guessing and asks,
/// which is the behaviour backlog §2.1 requires ("an annotation may render only if
/// its anchor resolves against the current document").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorReattachmentTier {
    /// The semantic block id from the snapshot the annotation was made on.
    SemanticBlockId,
    /// Exact quote plus surrounding prefix and suffix.
    QuoteWithAffixes,
    /// Normalised character offset in the main content flow.
    NormalizedTextPosition,
    /// Approximate quote match, always labelled approximate in the UI.
    FuzzyQuoteMatch,
    /// Nothing resolved. Render the annotation detached and ask the user.
    Detached,
}

/// Where a user attached an inline comment (backlog §2.1). The contract states the
/// fields a resolvable anchor needs; resolution itself is ANN-01/02.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationAnchor {
    pub document: DocumentToken,
    pub snapshot_version: SnapshotVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// Character offset in the normalised main-content flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_position: Option<u64>,
}

/// Target of an action, interpreted by the shell only against
/// `ActionRequest::expected_document`. A target descriptor is never portable
/// across documents or snapshots — which is what makes navigation invalidate
/// element handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTarget {
    pub resolution: TargetResolution,
    pub opaque_target: serde_json::Value,
}

/// Retry semantics after `timed_out`, where the outcome is genuinely unknown.
/// `Unsafe` means never auto-retry: a double submit is a different purchase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Idempotency {
    Safe,
    Conditional,
    Unsafe,
}

/// Utility → shell request to change something.
///
/// Three deliberate deviations from the backlog's sketch:
/// - `expected_document` is a [`DocumentToken`], not a `DocumentKey`, so profile
///   and workspace come from the brokered pipe rather than the payload.
/// - No `approvalClass`. The requester does not grade its own approval
///   requirement; the shell decides it and reports it in [`ActionDecision`].
/// - `expected_snapshot_version` is an assertion the shell re-checks; a mismatch
///   is `stale`, not a warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub action_id: String,
    pub task: TaskId,
    pub expected_document: DocumentToken,
    pub expected_snapshot_version: SnapshotVersion,
    /// Stable action name the shell recognises, e.g. "navigate", "scroll_to".
    pub action_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ActionTarget>,
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// Wall-clock upper bound in ms since the Unix epoch, compared against the
    /// shell clock. An already-expired deadline fails `timed_out` before dispatch
    /// rather than executing an abandoned plan.
    pub deadline_ms_since_epoch: i64,
    pub idempotency: Idempotency,
}

/// The shell's policy verdict, recorded before dispatch. Metadata only (backlog
/// §8): the audit may say a `Commit` action was approved by the user, never which
/// coordinates were clicked or what was typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDecision {
    pub level: ActionLevel,
    pub approval_required: bool,
    pub approval_granted: bool,
    /// The kill switch or per-domain automation setting that suppressed the
    /// action, if any. `None` when policy allowed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
}

/// Shell → utility completed action. Carries a resolved key, hence `Serialize`
/// only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionResult {
    pub action_id: String,
    pub decision: ActionDecision,
    /// The document the action actually executed against. `stale` if this is not
    /// the document the request named.
    pub executed_on: DocumentKey,
    pub observation_snapshot_version: SnapshotVersion,
    /// True when the observed state matched what the request declared it would
    /// cause. A false here is reported to the user, not silently retried.
    pub verified: bool,
    pub result_payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<OutputLimit>,
}

/// Where a task is in its lifecycle. Displayed and audited; not used for control
/// flow inside the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    Planning,
    AwaitingApproval,
    Executing,
    Verifying,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskProgress {
    pub task: TaskId,
    pub stage: TaskStage,
    pub completed_steps: u32,
    /// Absent when the total is genuinely unknown. The UI must not render a
    /// percentage from a guessed total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_steps: Option<u32>,
}

/// Why a task was cancelled. Distinguishing user intent from policy and from a
/// deadline matters because the UI must show "stopped" for the first and must not
/// present the other two as the user's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    UserStopped,
    Superseded,
    DeadlineElapsed,
    PolicyRevoked,
}

/// How far a cancellation actually got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelledAt {
    /// Nothing was in flight; no observable effect.
    BeforeFirstAction,
    /// Stopped between actions; earlier actions remain applied.
    BetweenActions,
    /// An action was already dispatched when the stop arrived. Its effect is
    /// unknown until re-observed.
    InFlightAction,
    /// The task had already completed; the stop changed nothing.
    AlreadyFinished,
}

/// Shell → utility stop request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationRequest {
    pub task: TaskId,
    pub reason: CancelReason,
}

/// The utility's answer to a stop. This is why cancellation is a contract rather
/// than a boolean: it states what may already have happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationResult {
    pub task: TaskId,
    pub cancelled_at: CancelledAt,
    /// Actions completed before the stop, so the user can still see what the task
    /// did after stopping it.
    pub actions_completed: u32,
    /// True when `cancelled_at` is `InFlightAction`: the side effect of that
    /// action is unknown and must be re-observed, never assumed absent.
    pub effect_unverified: bool,
}

// --- The assistant round trip: [`TaskRequest`] out, [`AnswerWithCitations`] back. ---
//
// Both payloads ride methods that already exist, so `frame::Method` stays closed:
//
// ```text
// shell --model.complete(params = TaskRequest)--------> utility
// shell <--task.finish(params = AnswerWithCitations)--- utility
// ```
//
// The asymmetry in their derives is the load-bearing part. `TaskRequest` carries a
// resolved [`crate::identity::DocumentKey`] and is `Serialize` **only**. That is not
// ceremony: a derived `Deserialize` impl is generated *inside* this crate and can
// therefore fill private fields, so making this type deserialisable would be a back
// door that mints a key — defeating the `compile_fail` example on `DocumentKey`
// while leaving it green. `AnswerWithCitations` carries only assertions, so the
// shell does deserialise it and then re-checks every one of them in
// [`AnswerWithCitations::verify`].

/// The shell's instruction to the utility: answer this question about this exact
/// document generation, and do nothing above this consequence level.
///
/// `Serialize` only, hence unforgeable in both directions: the utility cannot put
/// one on the wire (there is no `Deserialize`), and the shell cannot accidentally
/// accept a peer's version of one. Build it from the key the registry just handed
/// back, not from anything that arrived in a frame.
///
/// ```text
/// let key = registry.on_document_committed(tab, frame, upstream, url)?; // authority
/// let version = registry.next_snapshot_version(key.document_token(), &scope)?;
/// let request = TaskRequest { task, question, document: key, .. };
/// ```
///
/// A frame cannot produce one, which is the same proof as [`DocumentKey`]'s but worth
/// stating at the type that actually carries a key across the pipe:
///
/// ```compile_fail
/// use ai_platform_contract::contract::TaskRequest;
/// // Every value in this line is readable off `model.complete` params by any peer.
/// let injected: TaskRequest = serde_json::from_str(
///     r#"{"task":1,"question":"what is the price","document":{"profile_id":1,"workspace_id":1,
///         "tab_id":1,"frame_tree_node_id":1,"document_token":1,"navigation_sequence":1,
///         "upstream_navigation_id":null},"snapshot_version":1,"action_ceiling":"observe",
///         "cancel":{"task":1,"deadline_ms_since_epoch":1}}"#,
/// )
/// .unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskRequest {
    /// Minted by [`crate::identity::DocumentIdentityRegistry::create_task`], so it is
    /// already scoped to a workspace and dies with it.
    pub task: TaskId,
    /// The user's words. Subject to the same content discipline as everything else:
    /// it goes to a provider and is audited as metadata only (backlog §8).
    pub question: String,
    /// The document the question is about, as resolved by the shell. Serialised in
    /// full so the utility can echo `navigation_sequence` onto every citation it
    /// lifts from this capture — which is what makes a stale citation *checkable*
    /// rather than merely suspicious.
    pub document: DocumentKey,
    /// The capture the answer must be based on. A number the utility does not get to
    /// choose: [`crate::identity::DocumentIdentityRegistry::current_snapshot_version`]
    /// is the only honest source.
    pub snapshot_version: SnapshotVersion,
    /// Consequence ceiling for actions this task may plan. [`ActionLevel::Observe`]
    /// means "answer only"; anything above what Release 0 permits is refused at
    /// dispatch regardless of what this field says — see [`TaskRequest::ceiling_permits`].
    pub action_ceiling: ActionLevel,
    /// How this task is stopped.
    pub cancel: CancellationToken,
    /// Intent, selection, routing preference. Defaults keep older shells parsing.
    pub options: TaskOptions,
}

impl TaskRequest {
    /// The `params` object of a `model.complete` request. Exists so neither side
    /// hand-rolls `to_value` and a caller cannot pass a `TaskRequest` where a
    /// `Value` is expected without going through the same path every time.
    pub fn to_params(&self) -> Result<serde_json::Value, Failure> {
        serde_json::to_value(self).map_err(|error| {
            Failure::unsupported("contract.encode_failed").with_detail(error.to_string())
        })
    }

    /// Whether the task's ceiling lets this action kind through, as one predicate
    /// instead of a list each handler re-derives — the same reasoning as
    /// [`ActionLevel::allowed_in_release_zero`].
    ///
    /// Fail closed on two counts: an unclassified kind has no level ([`level_of_action_kind`]),
    /// and a ceiling above Release 0's permit is not a grant. The dispatch gate in the
    /// shell still re-checks both; this one is so the planner stops planning work that
    /// cannot run.
    pub fn ceiling_permits(&self, action_kind: &str) -> bool {
        match level_of_action_kind(action_kind) {
            Some(level) => level <= self.action_ceiling && self.action_ceiling.allowed_in_release_zero(),
            None => false,
        }
    }
}

/// A document identity as it appears on the wire: the same seven fields
/// [`DocumentKey`] serialises to, in the same names, readable by a peer that holds no
/// registry.
///
/// This is *not* authority, and the difference is the type rather than a convention:
/// [`crate::identity::DocumentIdentityRegistry::resolve`] is the only road from
/// `document_token` to a [`DocumentKey`], and every one of these fields is either
/// ignored or re-checked on receipt. The shell never deserialises this type at all —
/// the only inbound identity claims it accepts are [`ActionRequest::expected_document`]
/// and [`ContextSnapshotRequest::document`], both bare tokens.
///
/// It mirrors the key field-for-field on purpose, so a peer's view of a request parses
/// the exact bytes the shell emitted instead of a second spelling that can drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentClaim {
    pub profile_id: ProfileId,
    pub workspace_id: WorkspaceId,
    pub tab_id: TabId,
    pub frame_tree_node_id: u64,
    /// An index into the *shell's* registry. Useful for asking about it, worthless for
    /// claiming anything.
    pub document_token: DocumentToken,
    /// The per-tab counter that makes a same-URL reload a different document. A
    /// citation echoes this value; [`AnswerWithCitations::verify`] is what it is
    /// compared against.
    pub navigation_sequence: u64,
    /// Recorded, never load-bearing — see the module docs of [`crate::identity`].
    pub upstream_navigation_id: Option<UpstreamNavigationId>,
}

impl DocumentClaim {
    fn of_key(key: &DocumentKey) -> DocumentClaim {
        DocumentClaim {
            profile_id: key.profile_id(),
            workspace_id: key.workspace_id(),
            tab_id: key.tab_id(),
            frame_tree_node_id: key.frame_tree_node_id(),
            document_token: key.document_token(),
            navigation_sequence: key.navigation_sequence(),
            upstream_navigation_id: key.upstream_navigation_id(),
        }
    }
}

/// The utility's view of a [`TaskRequest`]: the same bytes, with the authority downgraded
/// to a claim.
///
/// `TaskRequest` is deliberately not deserialisable, so this is the only typed way for
/// the utility to read its own inbound `model.complete` params. The two serialise
/// identically — [`TaskRequestView::of`] is information-preserving — and they differ in
/// exactly one thing: the type of `document`. A [`DocumentKey`] can only come from the
/// registry, so the shell's copy *is* authority; a [`DocumentClaim`] can come from a
/// frame, which is precisely why nothing on the receiving side has to believe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRequestView {
    pub task: TaskId,
    pub question: String,
    pub document: DocumentClaim,
    pub snapshot_version: SnapshotVersion,
    pub action_ceiling: ActionLevel,
    pub cancel: CancellationToken,
    #[serde(default)]
    pub options: TaskOptions,
}

impl TaskRequestView {
    /// The projection the shell applies on the way out. Not a `From` impl: this copies
    /// identity *out* of a key, and an implicit conversion doing that is easier to
    /// reach for than to notice.
    pub fn of(request: &TaskRequest) -> TaskRequestView {
        TaskRequestView {
            task: request.task,
            question: request.question.clone(),
            document: DocumentClaim::of_key(&request.document),
            snapshot_version: request.snapshot_version,
            action_ceiling: request.action_ceiling,
            cancel: request.cancel,
            options: request.options.clone(),
        }
    }

    pub fn to_params(&self) -> Result<serde_json::Value, Failure> {
        serde_json::to_value(self).map_err(|error| {
            Failure::unsupported("contract.encode_failed").with_detail(error.to_string())
        })
    }
}

/// Why a passage is in the answer.
///
/// Closed and small on purpose: the overlay renders each role differently, so a
/// free-form string would either put unrenderable text on screen or be quietly
/// dropped. Unlike [`crate::failure::FailureKind`] this enum has **no** `Unknown`
/// sink — a citation whose role a consumer cannot render must fail the answer rather
/// than be labelled with a nearby role, so adding a variant is a change consumers
/// must be rebuilt for, not an additive one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationRole {
    /// The passage is the answer.
    DirectAnswer,
    /// The passage backs the answer.
    Supporting,
    /// The passage cuts against it. A model that cites only agreement is not
    /// evidencing anything, so this rung has to be expressible.
    Counterpoint,
    /// Neither: the user needs it to read the answer correctly.
    Context,
}

/// One quoted passage in an answer and what it is doing there (backlog §12: a
/// citation the user can jump to and check).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerCitation {
    /// Where the quote came from. Every field of this is an assertion the shell
    /// re-checks, which is why the anchor is reused as-is: an inline comment and a
    /// citation are the same kind of claim about a document.
    pub anchor: AnnotationAnchor,
    /// The `navigation_sequence` of the document generation the quote was lifted
    /// from, normally echoed from [`TaskRequestView::navigation_sequence`]. Recorded
    /// so a stored answer says *which* document it quoted, and compared against the
    /// live document by [`AnswerWithCitations::verify`].
    pub navigation_sequence: u64,
    pub role: CitationRole,
}

/// What a citation that did not survive verification is reported as, and why that
/// is one code: a replaced document and a mis-stated sequence are both "the passage
/// you are quoting is no longer on screen", and
/// [`crate::identity`] already established that emitting two codes for one event is
/// how a caller's error handling drifts out of sync with the truth.
pub const CITATION_STALE_CODE: &str = "citation.navigation_sequence_mismatch";

/// A citation quoting a capture the document never had — version 0, or a number
/// above what the registry has issued. A snapshot assumption violated, which is
/// `stale` in this taxonomy; the alternative readings (`unsupported`, `blocked`)
/// would tell the user to change something that is not the problem.
pub const CITATION_CAPTURE_UNSEEN_CODE: &str = "citation.snapshot_version_unseen";

/// The utility's finished answer (backlog FND-02). Travels as the `params` of the
/// `task.finish` notification, which is why it is `Deserialize`: everything inside
/// it is a claim about a document the shell re-checks.
///
/// Only an answer that *exists* belongs in here. "No answer" is
/// [`TaskOutcome::NoAnswer`], never `answer: ""` — and if a peer sends the empty
/// string anyway, [`AnswerWithCitations::verify`] refuses it as
/// [`crate::failure::FailureKind::NoAnswer`] rather than rendering a blank card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerWithCitations {
    /// Must be the task the shell issued. A notification naming a task this
    /// workspace does not own is refused — a task id outlives nothing, and a
    /// workspace close deletes its tasks precisely so a late answer has nowhere to
    /// land.
    pub task: TaskId,
    /// Model output. The shell may bound it ([`AnswerWithCitations::limit`]) but does
    /// not rewrite it; the provider's own provenance disclaimer belongs to the
    /// utility.
    pub answer: String,
    /// May be empty — an answer from model knowledge rather than page content cites
    /// nothing. Whether *this* answer must cite something is a product rule enforced
    /// above this type, not inside it.
    pub citations: Vec<AnswerCitation>,
    /// Present when the answer was truncated against a bound. Reporting a complete
    /// answer without it would be a false claim (backlog §4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<OutputLimit>,
}

impl AnswerWithCitations {
    pub fn to_params(&self) -> Result<serde_json::Value, Failure> {
        serde_json::to_value(self).map_err(|error| {
            Failure::unsupported("contract.encode_failed").with_detail(error.to_string())
        })
    }

    /// Checks every claim in this answer against live identity and returns the
    /// citations that survived, each bound to the resolved document it was checked
    /// against.
    ///
    /// **The invariant** (backlog §5, §12): a quoted passage is only trustworthy if
    /// the document it names is still the live one, which for this registry means its
    /// `navigation_sequence` is still the sequence carried by the citation. There is
    /// no `is_stale: bool` to forget to read. The reward for checking is
    /// [`TrustedCitation`], which has no public constructor — so the code that wants
    /// to scroll the user to a passage *had* to come through here. A refusal is
    /// [`Failure::stale`], whose constructor forces `replan_required = true`: the
    /// answer's plan was built on a document that is gone.
    ///
    /// Per-citation, not per-answer: citations in one answer may legitimately come
    /// from several documents in the same workspace, which is the point of a
    /// workspace-scoped context grant. Each is checked against its own document:
    ///
    /// - `stale` / [`CITATION_STALE_CODE`] — that document is no longer live, or the
    ///   sequence claimed is not the live document's.
    /// - `stale` / [`CITATION_CAPTURE_UNSEEN_CODE`] — the anchor quotes a capture the
    ///   document never had.
    /// - `blocked` / `identity.cross_scope` — the citation reaches outside the scope
    ///   this frame arrived on. Propagated from
    ///   [`crate::identity::DocumentIdentityRegistry::resolve`] unchanged and *not*
    ///   softened to stale: a substitution attempt must not look like the user
    ///   navigating.
    /// - `stale` / `identity.task_unknown`, `blocked` / `identity.task_cross_workspace`
    ///   — the answer belongs to no task this workspace owns.
    ///
    /// Citations are checked in order and the first refusal wins: a caller cannot
    /// recover a partial trust set by reordering, and one unusable citation is enough
    /// to make an answer's evidence claims unsafe to render as evidence.
    ///
    /// Before any citation is looked at, an answer with no text in it is refused as
    /// [`crate::failure::FailureKind::NoAnswer`] — see [`TaskOutcome`] for why that is
    /// a separate arm of the wire shape rather than a string state.
    pub fn verify(
        &self,
        registry: &DocumentIdentityRegistry,
        scope: &RequestScope,
    ) -> Result<TrustedAnswer, Failure> {
        // Emptiness is the one thing `answer: String` cannot make unrepresentable, so
        // it is refused on the way in. This is also what lets `TrustedAnswer::answer()`
        // promise the overlay a non-blank string.
        if self.answer.trim().is_empty() {
            return Err(Failure::no_answer("model.answer_absent").with_detail(
                "a finish frame carried an answer with no text; report it as \
                 TaskOutcome::NoAnswer rather than an empty answer",
            ));
        }
        registry.resolve_task(self.task, scope)?;

        let mut trusted = Vec::with_capacity(self.citations.len());
        for (index, citation) in self.citations.iter().enumerate() {
            let document = match registry.resolve(citation.anchor.document, scope) {
                Ok(key) => key,
                Err(error) if error.kind == FailureKind::Stale => {
                    // The document the quote came from is gone. Its sequence cannot
                    // equal anything, which is the same failure by a different route,
                    // so it reports as one — with the cause kept in `detail`.
                    return Err(Failure::stale(CITATION_STALE_CODE).with_detail(format!(
                        "citation {index} quotes document {} but that document is no longer live ({})",
                        citation.anchor.document.0, error.code
                    )));
                }
                Err(error) => return Err(error),
            };
            if citation.navigation_sequence != document.navigation_sequence() {
                return Err(Failure::stale(CITATION_STALE_CODE).with_detail(format!(
                    "citation {index} quotes navigation sequence {} but the live document is {}",
                    citation.navigation_sequence,
                    document.navigation_sequence()
                )));
            }
            // An anchor may only name a capture in `1..=highest`. Version 0 is the
            // "nothing captured" sentinel, a higher number is a capture that never
            // happened, and a live document nobody has ever captured cannot be quoted
            // from at all — even by a citation naming version 1.
            let captured =
                registry.current_snapshot_version(citation.anchor.document, scope).ok();
            let capture_unseen = match captured {
                None => true,
                Some(highest) => {
                    !citation.anchor.snapshot_version.is_captured()
                        || citation.anchor.snapshot_version > highest
                }
            };
            if capture_unseen {
                return Err(Failure::stale(CITATION_CAPTURE_UNSEEN_CODE).with_detail(format!(
                    "citation {index} quotes snapshot {} of document {}, which never reached it{}",
                    citation.anchor.snapshot_version.0,
                    citation.anchor.document.0,
                    match captured {
                        Some(highest) => format!(" (highest is {})", highest.0),
                        None => " (nothing has been captured)".to_string(),
                    }
                )));
            }
            trusted.push(TrustedCitation {
                anchor: citation.anchor.clone(),
                role: citation.role,
                document,
            });
        }

        Ok(TrustedAnswer { answer: self.answer.clone(), citations: trusted })
    }
}

/// What a `task.finish` notification carries.
///
/// `task.finish` is a notification, so there is no response envelope to put a
/// failure in — which is exactly how an empty `answer` string would become the only
/// way to say "nothing was produced", and an empty string is ambiguous between "the
/// model answered nothing" and "there is no answer". The two are different arms here
/// and cannot be confused by a caller that forgot to check.
///
/// ## There is no field that can hold reasoning text
///
/// The release-zero provider is a reasoning model and returns its chain of thought
/// in a separate `reasoning_content` field. It is not the answer, it is not citable,
/// and nothing here can persist it — the only reasoning-related field in this crate is
/// [`AnswerNotProduced::reasoning_tokens`], a count. A field for the text would put
/// hidden chain-of-thought in the same place citations anchor to, where it could be
/// quoted back at the user as evidence from the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum TaskOutcome {
    /// An answer exists. Its citations are still assertions until
    /// [`AnswerWithCitations::verify`] has run.
    Answered(AnswerWithCitations),
    /// No answer exists, with the reason and the numbers the shell needs to explain
    /// and remedy it.
    NoAnswer(AnswerNotProduced),
    /// The task ended without an answer for a reason that is not "the model said
    /// nothing": stopped, timed out, policy-blocked, provider error.
    Failed(TaskFailed),
}

impl TaskOutcome {
    pub fn to_params(&self) -> Result<serde_json::Value, Failure> {
        serde_json::to_value(self).map_err(|error| {
            Failure::unsupported("contract.encode_failed").with_detail(error.to_string())
        })
    }

    /// The task this outcome belongs to, whichever arm it is.
    pub fn task(&self) -> TaskId {
        match self {
            TaskOutcome::Answered(answer) => answer.task,
            TaskOutcome::NoAnswer(detail) => detail.task,
            TaskOutcome::Failed(failed) => failed.task,
        }
    }
}

/// The `no_answer` arm of [`TaskOutcome`], shaped by the one probe that produced it.
///
/// Measured 2026-11-13 against the served model `Qwen3.8-Flash-Next` (an
/// SGLang-hosted OpenAI-compatible endpoint): an 8-token completion returned
/// `"content": ""`, `"reasoning_content": "We need to respond to user: \""`,
/// `"finish_reason": "length"`, and `usage.reasoning_tokens: 9`. The budget, and the
/// reasoning count that proves where it went, are the two facts the shell can act on
/// — raise the budget, or answer without the reasoning path. The text is not a fact
/// the shell gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerNotProduced {
    pub task: TaskId,
    /// Always [`crate::failure::FailureKind::NoAnswer`] in practice; carried as a
    /// [`Failure`] so the one taxonomy carries the code, the retry advice, and the
    /// audit string instead of a second enum inventing its own words.
    pub failure: Failure,
    /// The completion budget the request asked for, in tokens. `0` means the shell
    /// sent no bound, in which case "budget exhausted" is not the explanation and
    /// `failure.code` says what is.
    pub completion_token_budget: u32,
    /// Tokens the provider spent on hidden reasoning, if it reported any. `None` is
    /// not zero: a server that omits usage leaves the shell unable to claim the
    /// budget went to reasoning, and the UI must not assert it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

impl AnswerNotProduced {
    /// The measured case: the whole completion budget was consumed by reasoning.
    pub fn budget_consumed_by_reasoning(
        task: TaskId,
        completion_token_budget: u32,
        reasoning_tokens: u32,
    ) -> AnswerNotProduced {
        AnswerNotProduced {
            task,
            failure: Failure::no_answer("model.reasoning_consumed_budget"),
            completion_token_budget,
            reasoning_tokens: Some(reasoning_tokens),
        }
    }

    /// A no-answer with no usage information to explain it. Distinct from
    /// [`AnswerNotProduced::budget_consumed_by_reasoning`] so the UI never reports
    /// "reasoning used your whole budget" on a server that said nothing about
    /// reasoning.
    pub fn unexplained(task: TaskId, code: &str, completion_token_budget: u32) -> AnswerNotProduced {
        AnswerNotProduced {
            task,
            failure: Failure::no_answer(code),
            completion_token_budget,
            reasoning_tokens: None,
        }
    }
}

/// An answer whose citations were all checked against live identity, and nothing
/// more.
///
/// Serialisable so the shell can hand verified content to the overlay, but
/// deliberately not deserialisable: trust is a fact about a *moment*, and a type that
/// could be rebuilt from a wire string would put it back on the shelf as trustable
/// after the user navigated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrustedAnswer {
    answer: String,
    citations: Vec<TrustedCitation>,
}

impl TrustedAnswer {
    pub fn answer(&self) -> &str {
        &self.answer
    }

    /// Never empty by construction of anything but an empty input: every element has
    /// already been resolved against the registry.
    pub fn citations(&self) -> &[TrustedCitation] {
        &self.citations
    }

    pub fn is_empty(&self) -> bool {
        self.citations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.citations.len()
    }
}

/// A citation that has been checked. Obtained only from
/// [`AnswerWithCitations::verify`].
///
/// ```compile_fail
/// use ai_platform_contract::contract::{AnswerCitation, CitationRole, TrustedCitation};
/// use ai_platform_contract::identity::{DocumentToken, SnapshotVersion};
/// // The shape is all public, so building one "to get the overlay working" looks
/// // easy. It is not: no constructor is visible outside this crate.
/// let trusted = TrustedCitation {
///     anchor: AnswerCitation {
///         anchor: ai_platform_contract::contract::AnnotationAnchor {
///             document: DocumentToken(1),
///             snapshot_version: SnapshotVersion(1),
///             block_id: None,
///             quote: "30-day returns".to_string(),
///             prefix: None,
///             suffix: None,
///             text_position: None,
///         },
///         navigation_sequence: 1,
///         role: CitationRole::Supporting,
///     },
///     document: unreachable!(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrustedCitation {
    anchor: AnnotationAnchor,
    role: CitationRole,
    /// The live document this citation was just resolved to. Carrying the key — not a
    /// copy of its numbers — is what lets the overlay hand it straight to dispatch.
    document: DocumentKey,
}

impl TrustedCitation {
    pub fn anchor(&self) -> &AnnotationAnchor {
        &self.anchor
    }

    pub fn role(&self) -> CitationRole {
        self.role
    }

    pub fn document(&self) -> DocumentKey {
        self.document
    }

    pub fn quote(&self) -> &str {
        &self.anchor.quote
    }

    /// The `scroll_to` target, in the field the action runtime reads. A citation
    /// without a block id is not untrustworthy — it resolves lower down
    /// [`AnchorReattachmentTier`] — it just cannot be scrolled to by block id yet.
    pub fn block_id(&self) -> Option<&str> {
        self.anchor.block_id.as_deref()
    }
}

/// How the shell stops one task.
///
/// Over Mojo, cancellation could be a capability the holder watched. Over a byte
/// pipe it is two facts the utility can act on without trusting anyone: the id a
/// `task.cancel` request will name, and a wall-clock bound after which the shell
/// stops waiting whether or not the stop frame ever arrives. The utility must answer
/// that request with a [`CancellationResult`]; the deadline needs no answer because
/// the shell has already stopped caring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationToken {
    /// The task a stop will name. Kept beside the deadline so a deep call stack handed
    /// only this token can still match an incoming `task.cancel` against its own work.
    pub task: TaskId,
    /// Wall-clock bound in ms since the Unix epoch, compared against the shell clock —
    /// the same convention as [`ActionRequest::deadline_ms_since_epoch`], so one
    /// injected clock serves both.
    pub deadline_ms_since_epoch: i64,
}

impl CancellationToken {
    /// Whether the task is at or past its bound. `expired_at(0)` is false: the
    /// sentinel `0` is the *unset* value in every clock field in this crate, so
    /// "no deadline" and "expired before it began" must not collide. A caller that
    /// genuinely wants an already-expired deadline sets `1`, which does fail.
    pub fn expired_at(&self, now_ms_since_epoch: i64) -> bool {
        self.deadline_ms_since_epoch != 0 && now_ms_since_epoch >= self.deadline_ms_since_epoch
    }

    /// Whether a stop request is addressed to this token's task. Reason is
    /// deliberately not part of the match: whatever the reason, the work stops, and
    /// the reason is for the audit and the UI (`UserStopped` must not be reported as
    /// a policy revocation).
    pub fn matches(&self, stop: &CancellationRequest) -> bool {
        self.task == stop.task
    }
}
