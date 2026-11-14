//! Versioned context, action, progress, and cancellation payloads (backlog
//! FND-02). The shapes come from backlog §4.1, §5, §7.1 and §7.3.
//!
//! Two rules govern which of these derive `Deserialize`:
//!
//! - Anything that carries a resolved [`crate::identity::DocumentKey`] is
//!   `Serialize` **only** (`ContextSnapshot`, `ActionResult`). Those messages travel
//!   shell → utility, and a type that cannot be deserialised cannot be injected
//!   from the other side no matter what a peer sends.
//! - Everything else derives both, because an id it carries is an *assertion* the
//!   receiver re-checks against its own registry, not authority.
//!
//! Semantic content types (`SemanticBlock`, `LinkDescriptor`, `ControlDescriptor`)
//! belong to Epic C and arrive as `serde_json::Value` payloads here, so this
//! envelope can freeze now without pre-empting the extraction design.

use serde::{Deserialize, Serialize};

use crate::identity::{
    DocumentKey, DocumentToken, SnapshotVersion, StaleStatus, TaskId, WorkspaceId,
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
        "read_document" | "find_passage" | "list_links" | "describe_image" => Some(ActionLevel::Observe),
        "scroll_to" | "open_tab" | "close_tab" | "activate_tab" | "preview_link" => {
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
