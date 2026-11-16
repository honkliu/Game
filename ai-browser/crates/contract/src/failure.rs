//! The one typed failure taxonomy (backlog FND-04).
//!
//! Every fallible operation in the AI platform — context capture, action
//! dispatch, model calls, cancellation — reports through [`Failure`] with a
//! [`FailureKind`] from this file. No module declares its own error enum; a new
//! failure mode is either a new variant here or a new `code` string under an
//! existing variant. `tools/check_conformance.py` fails if a wire fixture uses a
//! kind outside [`WIRE_FAILURE_KINDS`], and the versioned schemas in `schema/v1/`
//! enumerate exactly those eight.
//!
//! The taxonomy grew from seven to eight when the provider endpoint was measured:
//! [`FailureKind::NoAnswer`] has its own variant because an empty
//! completion is neither a truncation ([`FailureKind::OutputLimited`], where some
//! answer exists) nor an unavailability ([`FailureKind::Unavailable`], where
//! re-issuing could help), and every alternative would have left the caller to
//! infer it from `answer.is_empty()`.

use serde::{Deserialize, Serialize};

/// Machine-readable failure class.
///
/// The eight wire variants are the taxonomy required by FND-04. `Unknown` is not a
/// ninth failure mode: it is the deserialisation sink for a kind this binary
/// does not know, which is what makes adding a variant an additive (minor) change
/// instead of a frame that fails to parse. It is never produced deliberately, and
/// `as_str()` renders it as `"unknown"` so a log can never mistake it for one of
/// the eight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The capability exists but cannot be reached right now: provider
    /// unreachable, local model not loaded, renderer probe not attached.
    Unavailable,
    /// A policy, permission, or user setting refused the operation; also the kind
    /// used when a token or method is presented from outside the scope or side
    /// that owns it. Retrying without a policy change cannot succeed.
    Blocked,
    /// The context this operation was planned against no longer describes the
    /// document: navigated, replaced, tab closed, or a snapshot assumption
    /// violated. Never retried as-is — see [`Failure::stale`].
    Stale,
    /// The user or the browser ended the work. Not an error: partial output may
    /// be valid and the UI must not present it as a failure.
    Cancelled,
    /// A deadline elapsed. Side effects may already exist, so the caller must
    /// re-observe rather than assume nothing happened.
    TimedOut,
    /// Well formed and in scope, but this build cannot do it: unsupported page
    /// type, provider capability missing, action level behind a kill switch.
    Unsupported,
    /// The operation succeeded but output was truncated against a bound. Always
    /// paired with an [`crate::contract::OutputLimit`]; reporting plain success
    /// would be a false claim of completeness (backlog §4.1).
    OutputLimited,
    /// The model was reached, ran, and produced **no answer**. Distinct from
    /// [`FailureKind::OutputLimited`], where an answer exists but is short of what
    /// was asked for, and from [`FailureKind::Unavailable`], where re-issuing the
    /// same request could work.
    ///
    /// Measured against the release-zero provider on 2026-11-13: an 8-token probe of
    /// the served reasoning model returned `"content": ""`, a non-empty
    /// `"reasoning_content"`, `"finish_reason": "length"`, and
    /// `usage.reasoning_tokens: 9` — every token of the budget went to hidden
    /// reasoning. Treating that as `Ok("")` renders an empty answer card and invites
    /// a caller to retry the identical request, which returns the identical nothing.
    NoAnswer,
    /// A kind this binary has never seen. Forward-compat sink only.
    #[serde(other)]
    Unknown,
}

/// The eight wire values of the taxonomy, in declaration order. Kept as data so
/// the conformance tool and the schemas can assert against one list instead of
/// re-typing the names.
pub const WIRE_FAILURE_KINDS: [&str; 8] = [
    "unavailable",
    "blocked",
    "stale",
    "cancelled",
    "timed_out",
    "unsupported",
    "output_limited",
    "no_answer",
];

impl FailureKind {
    /// Stable wire/telemetry name. This is the value in the JSON `kind` field.
    pub fn as_str(self) -> &'static str {
        match self {
            FailureKind::Unavailable => "unavailable",
            FailureKind::Blocked => "blocked",
            FailureKind::Stale => "stale",
            FailureKind::Cancelled => "cancelled",
            FailureKind::TimedOut => "timed_out",
            FailureKind::Unsupported => "unsupported",
            FailureKind::OutputLimited => "output_limited",
            FailureKind::NoAnswer => "no_answer",
            FailureKind::Unknown => "unknown",
        }
    }

    /// True for the eight defined kinds, false for the [`FailureKind::Unknown`]
    /// sink. UI code that must not silently accept an unrecognised degradation
    /// branches on this.
    pub fn is_defined(self) -> bool {
        self != FailureKind::Unknown
    }
}

/// A failure carried across the process boundary or returned to the UI.
///
/// `code` is a stable namespaced telemetry id chosen by the producer (for
/// example `"identity.token_retired"`). `detail` is developer-facing text.
/// Neither may contain page text, coordinates, credentials, or customer content:
/// the action audit records metadata only (backlog §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub code: String,
    /// Re-issuing the identical request could succeed without new context or a
    /// policy change.
    pub retryable: bool,
    /// A new plan built from freshly observed context is required before
    /// continuing (backlog §5).
    pub replan_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Failure {
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn new(kind: FailureKind, code: &str, retryable: bool, replan_required: bool) -> Self {
        Failure {
            kind,
            code: code.to_string(),
            retryable,
            replan_required,
            detail: None,
        }
    }

    pub fn unavailable(code: &str) -> Self {
        Self::new(FailureKind::Unavailable, code, true, false)
    }

    pub fn blocked(code: &str) -> Self {
        Self::new(FailureKind::Blocked, code, false, false)
    }

    /// Backlog §5: an action planned from a stale answer cannot execute without
    /// replanning. That rule lives here, in the only constructor for this kind,
    /// so a call site cannot state that a stale operation is merely retryable.
    pub fn stale(code: &str) -> Self {
        Self::new(FailureKind::Stale, code, false, true)
    }

    pub fn cancelled(code: &str) -> Self {
        Self::new(FailureKind::Cancelled, code, false, false)
    }

    /// The outcome is unknown, so a retry may be attempted — but the action
    /// runtime gates an *automatic* retry on
    /// [`crate::contract::Idempotency`] and never auto-retries an `Unsafe`
    /// action, because a double submit is a different purchase.
    pub fn timed_out(code: &str) -> Self {
        Self::new(FailureKind::TimedOut, code, true, false)
    }

    pub fn unsupported(code: &str) -> Self {
        Self::new(FailureKind::Unsupported, code, false, false)
    }

    /// Asking for more output is a different request with a different bound, not
    /// a retry of this one.
    pub fn output_limited(code: &str) -> Self {
        Self::new(FailureKind::OutputLimited, code, false, false)
    }

    /// The model ran and returned no answer. Not retryable, for the same reason
    /// [`Failure::output_limited`] is not: re-issuing the identical request, same
    /// token budget, returns the identical empty completion. The remedy is a
    /// different request (larger budget, non-reasoning path, or no model call), and
    /// `replan_required` stays false because the *plan* was sound; only the call
    /// needs re-issuing with different parameters. Raising it here would make the
    /// shell rebuild context it already has.
    pub fn no_answer(code: &str) -> Self {
        Self::new(FailureKind::NoAnswer, code, false, false)
    }
}
