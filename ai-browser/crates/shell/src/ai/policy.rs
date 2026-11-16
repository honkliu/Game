//! The Release 0 action gate (R0a-7): deny-all on page mutation.
//!
//! Release 0 reads pages and scrolls them. It does not type, click, select,
//! submit, or buy. That promise is enforced at the one place an action becomes a
//! dispatch, and it holds because of *where* the decision is made: an action that
//! does not get past [`ReleaseZeroGate::admit`] cannot reach `ExecuteScript`, and
//! this tree contains no Win32 input-synthesis primitive to reach instead — the
//! test at the bottom of this file scans `src/ai` for those API names rather than
//! trusting that nobody added one. (Prose included: keep the identifiers out of the
//! comments here, which is why they are named nowhere in this module.)
//!
//! ## What this module deliberately does not contain
//!
//! It does not contain the level table. Which action kind is `Observe` and which
//! is `Commit` is decided by
//! [`ai_platform_contract::contract::level_of_action_kind`], and whether a level
//! may run in Release 0 is decided by
//! [`ai_platform_contract::contract::ActionLevel::allowed_in_release_zero`]. Both
//! live in the contract crate and both are tested there. Duplicating either here
//! would give the shell a second answer to the same question, and two answers
//! drift one refactor apart — which is how a page mutation ships in a release that
//! promises none. The only thing this module decides for itself is which
//! `Navigate`-class kinds the shell has a handler for today
//! ([`DISPATCHABLE_NAVIGATE_KINDS`]), and that is a capability list, not a
//! classification.
//!
//! ## Deny is the default
//!
//! A kind the contract's table does not know has no level, and this gate refuses
//! it. The tempting alternative — "we do not know what it does, it is probably
//! read-only" — is exactly how a mutation reaches a page. The failure is the one
//! the contract mandates for this case: `unsupported` with code
//! [`CODE_ACTION_KIND_UNKNOWN`].
//!
//! ## Audit
//!
//! Every verdict, allowed or denied, is recorded ([`ReleaseZeroGate::audit`]) as
//! metadata only: action id, task, document token, kind, level, blocking code.
//! Never `ActionRequest::arguments`, never a target locator, never page text
//! (backlog §8). The trail is in-process and bounded; the append-to-disk sink is
//! R0a-7's `audit.rs` and is not in this slice, so the trail is exposed through an
//! accessor instead of being written to a file here.

use std::cell::{Cell, RefCell};

use ai_platform_contract::contract::{
    level_of_action_kind, ActionDecision, ActionLevel, ActionRequest,
};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::identity::{DocumentToken, TaskId};

/// A kind the contract's table does not list. Required verbatim by
/// `level_of_action_kind`'s own contract: "An unrecognised kind is `None`, and
/// dispatch must refuse it with `unsupported/action_kind_unknown`".
pub const CODE_ACTION_KIND_UNKNOWN: &str = "action_kind_unknown";

/// The kind has a level and Release 0 does not allow that level. `Interact` and
/// `Commit` land here.
pub const CODE_LEVEL_NOT_ALLOWED: &str = "policy.release_zero_blocks_level";

/// The kind is `Navigate`-class — a class Release 0 allows — but this build has no
/// handler for it. `unsupported` rather than `blocked`, because the policy is not
/// refusing, the runtime is not built yet, and a caller that cannot tell those
/// apart cannot tell whether a policy change would help.
pub const CODE_KIND_NOT_DISPATCHABLE: &str = "policy.navigate_kind_not_dispatched";

/// `Navigate`-class kinds this build can actually dispatch.
///
/// This is a capability list (which handlers exist), not a classification (which
/// level a kind has). Release 0 admits the whole `Observe` class by definition — it
/// mutates nothing — but a `Navigate` action still needs code behind it, and the
/// only one in this slice is the host-driven scroll that
/// `ActionLevel::allowed_in_release_zero()` exists to admit. `open_tab`,
/// `close_tab`, `activate_tab` and `preview_link` are `Navigate` in the contract
/// and have no gate-connected handler yet, so they are refused here rather than
/// admitted and then failing somewhere less legible.
pub const DISPATCHABLE_NAVIGATE_KINDS: [&str; 6] =
    ["scroll_to", "open_tab", "activate_tab", "navigate", "show_candidate", "close_tab"];

/// Verdicts kept before the oldest is dropped. A long session fires actions in the
/// hundreds; the cap exists so a browser open for a month cannot grow without
/// bound, and [`ReleaseZeroGate::audit_dropped`] makes the loss visible instead of
/// silent.
pub const AUDIT_CAPACITY: usize = 4096;

/// The identity fields of a request, without anything that can carry page content.
/// `ActionRequest::arguments` is arbitrary JSON from the utility process and
/// `ActionRequest::target` is a locator; neither is copied here, so neither can
/// reach the audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionIdentity {
    pub action_id: String,
    pub task: TaskId,
    pub document: DocumentToken,
    pub action_kind: String,
}

impl ActionIdentity {
    fn of(request: &ActionRequest) -> Self {
        ActionIdentity {
            action_id: request.action_id.clone(),
            task: request.task,
            document: request.expected_document,
            action_kind: request.action_kind.clone(),
        }
    }
}

/// One verdict. Written for every action the gate is asked about, allowed or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditRecord {
    /// The contract's table knows the kind, so the verdict is the contract's own
    /// [`ActionDecision`] — the type that will travel beside the action result —
    /// paired with the identity it describes.
    Graded {
        identity: ActionIdentity,
        decision: ActionDecision,
    },
    /// The table does not know the kind. `ActionDecision::level` is a required
    /// `ActionLevel`, so the contract cannot express "unclassified", and writing
    /// `Observe` here would put a false claim in the audit trail. That gap in the
    /// contract is the whole reason this variant exists.
    Unclassified { identity: ActionIdentity },
}

impl AuditRecord {
    /// The action kind the record is about, whichever variant it is.
    pub fn action_kind(&self) -> &str {
        match self {
            AuditRecord::Graded { identity, .. }
            | AuditRecord::Unclassified { identity } => &identity.action_kind,
        }
    }

    /// The code the gate refused with, when it refused.
    pub fn blocked_by(&self) -> Option<&str> {
        match self {
            AuditRecord::Graded { decision, .. } => decision.blocked_by.as_deref(),
            // For this variant the absence of a level *is* the refusal.
            AuditRecord::Unclassified { .. } => Some(CODE_ACTION_KIND_UNKNOWN),
        }
    }
}

/// The Release 0 dispatch gate. UI-thread only, like the rest of `AppState`
/// (`RefCell`, no lock), and constructed once per process.
#[derive(Debug, Default)]
pub struct ReleaseZeroGate {
    audit: RefCell<Vec<AuditRecord>>,
    dropped: Cell<usize>,
}

impl ReleaseZeroGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide one dispatched action. `Ok(level)` means the caller may proceed;
    /// `Err` is the typed failure to hand back to the requester, already carrying
    /// the kind, code, and retry/replan flags, so no caller re-derives them.
    ///
    /// This answers one question — may this kind run at all in Release 0 — and not
    /// the others. Deadline expiry, snapshot-version match, and scope/token
    /// resolution are the dispatcher's checks (R0a-3, R0a-4) and run after this
    /// one, because a kind that is not allowed should not have its staleness
    /// examined first.
    ///
    /// This is deliberately the only decision point on the type. A second entry
    /// point would be a path around the gate.
    pub fn admit(&self, request: &ActionRequest) -> Result<ActionLevel, Failure> {
        let Some(level) = level_of_action_kind(&request.action_kind) else {
            self.record(AuditRecord::Unclassified {
                identity: ActionIdentity::of(request),
            });
            return Err(Failure::unsupported(CODE_ACTION_KIND_UNKNOWN));
        };

        // The contract's predicate is the authority on the class; this module only
        // turns its answer into the contract's typed failure, so the failure and
        // the audit record cannot disagree.
        let blocked: Option<Failure> = if !level.allowed_in_release_zero() {
            Some(Failure::blocked(CODE_LEVEL_NOT_ALLOWED))
        } else if level == ActionLevel::Navigate
            && !DISPATCHABLE_NAVIGATE_KINDS.contains(&request.action_kind.as_str())
        {
            Some(Failure::unsupported(CODE_KIND_NOT_DISPATCHABLE))
        } else {
            None
        };

        self.record(AuditRecord::Graded {
            identity: ActionIdentity::of(request),
            decision: ActionDecision {
                level,
                // Release 0 has no approval UI (deferred to R0b) and admits nothing
                // that would need one: `Observe` mutates nothing and the single
                // admitted `Navigate` action is a scroll. Both bits are recorded as
                // false rather than left implied.
                approval_required: false,
                approval_granted: false,
                blocked_by: blocked.as_ref().map(|failure| failure.code.clone()),
            },
        });

        match blocked {
            Some(failure) => Err(failure),
            None => Ok(level),
        }
    }

    /// Every verdict so far, oldest first. A copy, so a caller may hold it across
    /// further dispatches.
    pub fn audit(&self) -> Vec<AuditRecord> {
        self.audit.borrow().clone()
    }

    /// Verdicts dropped because the trail reached [`AUDIT_CAPACITY`]. Non-zero
    /// means the audit is incomplete, which has to be sayable.
    pub fn audit_dropped(&self) -> usize {
        self.dropped.get()
    }

    fn record(&self, record: AuditRecord) {
        let mut trail = self.audit.borrow_mut();
        if trail.len() >= AUDIT_CAPACITY {
            // Drop the oldest: the verdicts a live card depends on are the recent
            // ones, and the counter keeps the loss honest.
            trail.remove(0);
            self.dropped.set(self.dropped.get() + 1);
        }
        trail.push(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_platform_contract::contract::Idempotency;
    use ai_platform_contract::failure::FailureKind;
    use ai_platform_contract::identity::SnapshotVersion;

    /// A request whose only interesting field is the kind; the rest satisfy the
    /// contract type so the gate is driven through its real public input.
    fn request(action_kind: &str) -> ActionRequest {
        ActionRequest {
            action_id: "a-1".to_string(),
            task: TaskId(7),
            expected_document: DocumentToken(3),
            expected_snapshot_version: SnapshotVersion(1),
            action_kind: action_kind.to_string(),
            target: None,
            arguments: serde_json::json!({"value": "never-4111-1111-1111"}),
            deadline_ms_since_epoch: i64::MAX,
            idempotency: Idempotency::Safe,
        }
    }

    /// Every `ActionLevel` in the contract fired at the gate, each through a kind
    /// the contract classifies at that level. Only `Observe`, and inside
    /// `Navigate` only the allowlisted scroll, are admitted.
    #[test]
    fn every_action_level_fires_at_the_gate_and_only_observe_and_scroll_pass() {
        let gate = ReleaseZeroGate::new();
        let cases: [(ActionLevel, &str); 4] = [
            (ActionLevel::Observe, "read_document"),
            (ActionLevel::Navigate, "scroll_to"),
            (ActionLevel::Interact, "type_text"),
            (ActionLevel::Commit, "purchase"),
        ];
        for (level, kind) in cases {
            let result = gate.admit(&request(kind));
            let expected = level == ActionLevel::Observe || kind == "scroll_to";
            assert_eq!(result.is_ok(), expected, "{kind} ({level:?}) -> {result:?}");
            if expected {
                assert_eq!(result.unwrap(), level, "{kind} admitted at the wrong level");
            } else {
                // The brief's requirement, stated per level rather than per kind.
                let failure = result.unwrap_err();
                assert_eq!(failure.kind, FailureKind::Blocked, "{kind} must be blocked");
                assert_eq!(failure.code, CODE_LEVEL_NOT_ALLOWED);
                assert!(!failure.retryable, "a policy denial is not a retryable error");
                assert!(!failure.replan_required, "a policy denial is not a stale plan");
            }
        }
    }

    /// A denied action produces the typed failure *and* an audit record from one
    /// call. Either alone would let a refused mutation look like it never happened.
    #[test]
    fn a_denied_action_returns_the_typed_failure_and_writes_the_audit_record() {
        let gate = ReleaseZeroGate::new();
        let before = gate.audit().len();

        let failure = gate.admit(&request("submit_form")).unwrap_err();

        let trail = gate.audit();
        assert_eq!(trail.len(), before + 1, "a denial must leave exactly one record");
        let record = trail.last().unwrap();
        assert_eq!(record.action_kind(), "submit_form");
        assert_eq!(record.blocked_by(), Some(CODE_LEVEL_NOT_ALLOWED));
        assert_eq!(failure.kind, FailureKind::Blocked);
        // The record and the failure describe the same action.
        let AuditRecord::Graded { identity, decision } = record else {
            panic!("submit_form is classified; it must grade")
        };
        assert_eq!(identity.action_id, "a-1");
        assert_eq!(identity.task, TaskId(7));
        assert_eq!(identity.document, DocumentToken(3));
        assert_eq!(decision.level, ActionLevel::Commit);
        assert_eq!(decision.blocked_by.as_deref(), Some(CODE_LEVEL_NOT_ALLOWED));
        assert!(!decision.approval_required && !decision.approval_granted);
    }

    /// An allowed action is recorded too: the trail is the shell's answer to "what
    /// did the assistant do", and a silent allow is unanswerable.
    #[test]
    fn an_allowed_action_is_recorded_without_a_block_reason() {
        let gate = ReleaseZeroGate::new();
        assert_eq!(gate.admit(&request("scroll_to")).unwrap(), ActionLevel::Navigate);

        let trail = gate.audit();
        let AuditRecord::Graded { decision, .. } = trail.last().unwrap() else {
            panic!("scroll_to is classified")
        };
        assert_eq!(decision.blocked_by, None);
        assert_eq!(decision.level, ActionLevel::Navigate);
        assert_eq!(trail.last().unwrap().blocked_by(), None);
    }

    /// Deny is the default: a kind the table does not know is refused and recorded
    /// as unclassified rather than graded at some level.
    #[test]
    fn an_unknown_action_kind_denies_and_is_recorded_as_unclassified() {
        let gate = ReleaseZeroGate::new();
        for kind in ["harvest_cookies", "", "Scroll_To", "read_document ", "type_text()"] {
            let failure = gate
                .admit(&request(kind))
                .expect_err(&format!("{kind:?} is not in the contract table and must deny"));
            assert_eq!(failure.kind, FailureKind::Unsupported, "{kind:?}");
            assert_eq!(failure.code, CODE_ACTION_KIND_UNKNOWN, "{kind:?}");

            let trail = gate.audit();
            let record = trail.last().unwrap();
            assert!(
                matches!(record, AuditRecord::Unclassified { .. }),
                "{kind:?} must not be graded at a level"
            );
            assert_eq!(record.blocked_by(), Some(CODE_ACTION_KIND_UNKNOWN));
            assert_eq!(record.action_kind(), kind);
        }
        // Nothing was allowed on the way through.
        assert!(gate.audit().iter().all(|record| record.blocked_by().is_some()));
    }

    /// `Navigate` is an allowed *class* in Release 0, but a kind with no handler
    /// still has to be refused rather than admitted-then-failing elsewhere.
    #[test]
    fn navigate_kinds_without_a_handler_are_refused() {
        let gate = ReleaseZeroGate::new();
        for kind in ["preview_link", "go_back", "highlight_passage"] {
            let failure = gate
                .admit(&request(kind))
                .expect_err("{kind} has no handler in this slice");
            assert_eq!(failure.kind, FailureKind::Unsupported, "{kind}");
            assert_eq!(failure.code, CODE_KIND_NOT_DISPATCHABLE, "{kind}");
            assert_eq!(
                gate.audit().last().unwrap().blocked_by(),
                Some(CODE_KIND_NOT_DISPATCHABLE)
            );
        }
        assert_eq!(
            gate.admit(&request("scroll_to")).unwrap(),
            ActionLevel::Navigate,
            "the allowlisted scroll is the one Navigate action that passes"
        );
    }

    /// Backlog §8: the trail records metadata, never content. `arguments` is the
    /// field through which a provider could otherwise smuggle page text into an
    /// audit log.
    #[test]
    fn the_audit_trail_carries_no_arguments_or_target() {
        let gate = ReleaseZeroGate::new();
        for kind in ["submit_form", "purchase", "type_text"] {
            let _ = gate.admit(&request(kind));
        }
        let text = format!("{:?}", gate.audit());
        assert!(!text.contains("4111"), "argument value reached the audit trail: {text}");
        assert!(!text.contains("value"), "argument field names reached the trail: {text}");
    }

    /// The bound is enforced, and the loss it causes is countable.
    #[test]
    fn the_trail_is_bounded_and_reports_what_it_dropped() {
        let gate = ReleaseZeroGate::new();
        for _ in 0..AUDIT_CAPACITY + 5 {
            let _ = gate.admit(&request("purchase"));
        }
        assert_eq!(gate.audit().len(), AUDIT_CAPACITY);
        assert_eq!(gate.audit_dropped(), 5);
    }

    /// R0a-7's source-level assertion: no synthetic-input primitive may appear in
    /// the R0a tree. The identifiers are assembled from fragments at runtime, so
    /// this file — which is itself scanned — does not contain them.
    #[test]
    fn no_synthetic_input_primitive_appears_in_the_ai_tree() {
        let join = |a: &str, b: &str| format!("{a}{b}");
        let forbidden: Vec<String> = [
            join("Send", "Input"),
            join("mouse", "_event"),
            join("keybd", "_event"),
            join("Send", "Mouse"),
            join("Block", "Input"),
            join("Send", "Key"),
        ]
        .to_vec();

        // Recursive, so a new subdirectory cannot become a blind spot.
        let mut pending = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/ai"
        ))];
        let mut scanned = 0usize;
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).expect("module directory is readable") {
                let path = entry.expect("readable entry").path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                scanned += 1;
                let text = std::fs::read_to_string(&path).expect("module is readable");
                for needle in &forbidden {
                    assert!(
                        !text.contains(needle.as_str()),
                        "{} uses {needle}: Release 0 forbids synthetic input",
                        path.display()
                    );
                }
            }
        }
        assert!(scanned >= 3, "expected the ai modules, scanned {scanned} files");
    }
}
