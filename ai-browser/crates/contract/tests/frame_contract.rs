//! FND-02/FND-03 boundary tests: the closed method list, the side gate, the size
//! bound, and FND-04's taxonomy on the wire.

#![allow(clippy::unwrap_used)]

use ai_platform_contract::failure::{Failure, FailureKind, WIRE_FAILURE_KINDS};
use ai_platform_contract::frame::{self, Frame, Method, Notification, Peer, ResponsePayload};

fn decode_json(json: serde_json::Value, from: Peer) -> Result<Frame, Failure> {
    frame::decode(&json.to_string(), from)
}

fn request(method: Method) -> serde_json::Value {
    serde_json::json!({
        "kind": "request",
        "schema_version": {"major": 1, "minor": 0},
        "id": 3,
        "method": method.as_str(),
        "params": {},
    })
}

fn notification(method: Method) -> serde_json::Value {
    serde_json::json!({
        "kind": "notification",
        "schema_version": {"major": 1, "minor": 0},
        "method": method.as_str(),
        "params": {},
    })
}

/// Negative control for `tests/version_contract.rs`: a frame under the supported
/// major *does* reach interpretation and fails there, so the version code and the
/// payload code are genuinely different outcomes rather than the same one.
#[test]
fn a_supported_major_does_reach_payload_interpretation() {
    let line = r#"{"kind":"request","schema_version":{"major":1,"minor":0},"method":"model.complete","params":{}}"#;
    let error = frame::decode(line, Peer::Shell).unwrap_err();
    assert_eq!(error.code, "transport.payload_invalid");
}

#[test]
fn the_utility_cannot_issue_a_shell_only_method() {
    // If the utility could issue `model.complete` to itself it could start a provider
    // call no policy approved, and could forge the reply to its own cancellation.
    for method in [Method::CompleteModel, Method::CancelTask, Method::Ping] {
        assert_eq!(method.issuer(), Peer::Shell);
        let error = decode_json(request(method), Peer::Utility).unwrap_err();
        assert_eq!(error.kind, FailureKind::Blocked);
        assert_eq!(error.code, "transport.method_from_wrong_side");
        assert!(error.detail.unwrap().contains(method.as_str()));
    }
}

#[test]
fn the_shell_cannot_issue_a_utility_only_method() {
    for method in [Method::PublishProgress, Method::FinishTask, Method::AgentEvent] {
        assert_eq!(method.issuer(), Peer::Utility);
        let error = decode_json(request(method), Peer::Shell).unwrap_err();
        assert_eq!(error.code, "transport.method_from_wrong_side");
    }
}

#[test]
fn a_method_issued_from_its_own_side_decodes() {
    // Guards against the side gate being written so it rejects everything.
    let decoded = decode_json(notification(Method::AgentEvent), Peer::Utility).unwrap();
    let Frame::Notification(note) = decoded else {
        panic!("expected a notification");
    };
    assert_eq!(note.method, Method::AgentEvent);

    let decoded = decode_json(request(Method::CompleteModel), Peer::Shell).unwrap();
    assert!(matches!(decoded, Frame::Request(_)));
}

#[test]
fn notification_only_methods_must_arrive_as_notifications() {
    for method in [Method::PublishProgress, Method::FinishTask] {
        assert!(method.is_notification());
        let error = decode_json(request(method), Peer::Utility).unwrap_err();
        assert_eq!(error.code, "transport.frame_kind_mismatch");
        assert!(decode_json(notification(method), Peer::Utility).is_ok());
    }
}

#[test]
fn request_only_methods_must_not_arrive_as_notifications() {
    let error = decode_json(notification(Method::CompleteModel), Peer::Shell).unwrap_err();
    assert_eq!(error.kind, FailureKind::Blocked);
    assert_eq!(error.code, "transport.frame_kind_mismatch");
}

#[test]
fn an_unknown_method_is_rejected_rather_than_ignored() {
    let mut json = request(Method::Ping);
    json["method"] = serde_json::json!("action.dispatch_everything");
    let error = decode_json(json, Peer::Shell).unwrap_err();
    assert_eq!(error.code, "transport.method_unknown");
    assert_eq!(error.detail.unwrap(), "action.dispatch_everything");
}

#[test]
fn the_frame_bound_is_enforced_before_parsing() {
    let oversized = format!(
        r#"{{"kind":"notification","schema_version":{{"major":1,"minor":0}},"method":"task.progress","params":{{"blob":"{}"}}}}"#,
        "x".repeat(frame::MAX_FRAME_BYTES),
    );
    assert!(oversized.len() > frame::MAX_FRAME_BYTES);
    let error = frame::decode(&oversized, Peer::Utility).unwrap_err();
    assert_eq!(error.kind, FailureKind::OutputLimited);
    assert_eq!(error.code, "transport.frame_too_large");
}

#[test]
fn malformed_and_versionless_frames_are_rejected() {
    assert_eq!(frame::decode("{not json", Peer::Utility).unwrap_err().code, "transport.frame_malformed");
    assert_eq!(
        frame::decode(r#"{"kind":"request","method":"utility.ping"}"#, Peer::Shell).unwrap_err().code,
        "transport.version_header_missing"
    );
    assert_eq!(
        frame::decode(r#"{"schema_version":{"major":1,"minor":0},"method":"utility.ping"}"#, Peer::Shell)
            .unwrap_err()
            .code,
        "transport.frame_malformed"
    );
}

#[test]
fn a_frame_cannot_forge_a_second_line() {
    // Line-delimited framing means a newline is a frame boundary. The payload contains
    // one, and the encoder must not let it split the frame.
    let frame = Frame::notification(
        Method::PublishProgress,
        &serde_json::json!({"note": "line one\nline two"}),
    );
    let line = frame::encode(&frame).unwrap();
    assert_eq!(line.matches('\n').count(), 0);
    let decoded = frame::decode(&line, Peer::Utility).unwrap();
    let Frame::Notification(Notification { params, .. }) = decoded else {
        panic!("expected a notification");
    };
    assert_eq!(params["note"], serde_json::json!("line one\nline two"));
}

#[test]
fn a_response_is_either_a_value_or_a_failure_never_both_or_neither() {
    let json = serde_json::to_value(Frame::ok(9, ResponsePayload::Ok(serde_json::json!({"n": 1})))).unwrap();
    assert_eq!(json["payload"]["ok"]["n"], serde_json::json!(1));
    assert!(json["payload"].get("failure").is_none());

    let json = serde_json::to_value(Frame::failure(9, Failure::stale("identity.token_retired"))).unwrap();
    assert_eq!(json["payload"]["failure"]["kind"], serde_json::json!("stale"));
    assert_eq!(json["payload"]["failure"]["replan_required"], serde_json::json!(true));

    // Both at once is not representable: an externally tagged enum takes one arm, and
    // a hand-built object naming both fails to parse rather than picking one.
    let both = r#"{"kind":"response","schema_version":{"major":1,"minor":0},"id":9,"payload":{"ok":1,"failure":{"kind":"stale","code":"x","retryable":false,"replan_required":true}}}"#;
    assert!(frame::decode(both, Peer::Utility).is_err());
    let neither = r#"{"kind":"response","schema_version":{"major":1,"minor":0},"id":9,"payload":{}}"#;
    assert!(frame::decode(neither, Peer::Utility).is_err());
}

// --- FND-04: one shared taxonomy, exactly eight wire values. ---

#[test]
fn the_wire_taxonomy_is_exactly_the_required_kinds() {
    assert_eq!(
        WIRE_FAILURE_KINDS,
        [
            "unavailable",
            "blocked",
            "stale",
            "cancelled",
            "timed_out",
            "unsupported",
            "output_limited",
            "no_answer",
        ]
    );
    let all = [
        FailureKind::Unavailable,
        FailureKind::Blocked,
        FailureKind::Stale,
        FailureKind::Cancelled,
        FailureKind::TimedOut,
        FailureKind::Unsupported,
        FailureKind::OutputLimited,
        FailureKind::NoAnswer,
    ];
    let rendered: Vec<&str> = all.iter().map(|kind| kind.as_str()).collect();
    assert_eq!(rendered, WIRE_FAILURE_KINDS.to_vec());
    for kind in all {
        assert!(kind.is_defined());
        let json = serde_json::to_value(kind).unwrap();
        assert_eq!(json, serde_json::json!(kind.as_str()));
    }
}

#[test]
fn a_kind_from_a_newer_build_does_not_masquerade_as_a_known_one() {
    // Adding a variant is additive precisely because an unrecognised name lands in
    // `Unknown` instead of failing the frame or being mapped onto a nearby kind.
    let unknown: FailureKind = serde_json::from_value(serde_json::json!("page_mutated")).unwrap();
    assert_eq!(unknown, FailureKind::Unknown);
    assert!(!unknown.is_defined());
    assert_eq!(unknown.as_str(), "unknown");
}

#[test]
fn the_taxonomy_rules_are_not_overridable_at_the_call_site() {
    // Backlog §5: a stale answer cannot be acted on without replanning, and a blocked
    // scope check must not be dressed up as retryable. These two properties are what
    // the action runtime relies on, so they are asserted on the constructors.
    let stale = Failure::stale("identity.document_not_current");
    assert!(stale.replan_required);
    assert!(!stale.retryable);

    let blocked = Failure::blocked("identity.cross_scope");
    assert!(!blocked.retryable);
    assert!(!blocked.replan_required);

    let timed_out = Failure::timed_out("utility.response_deadline");
    assert!(timed_out.retryable);
    assert!(!timed_out.replan_required);

    assert!(Failure::unavailable("utility.process_lost").retryable);
    assert!(!Failure::cancelled("task.stopped").retryable);
    assert!(!Failure::output_limited("context.block_budget").retryable);
    assert!(!Failure::unsupported("schema.major_unsupported").retryable);
    // A model that answered nothing is not retryable as-is and needs no new context;
    // the remedy is a different request, so neither flag may be raised at the call site.
    let no_answer = Failure::no_answer("model.reasoning_consumed_budget");
    assert!(!no_answer.retryable);
    assert!(!no_answer.replan_required);
}
