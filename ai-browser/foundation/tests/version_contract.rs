//! FND-02 acceptance: an old consumer rejects an incompatible major version
//! cleanly, meaning it rejects *without interpreting* rather than misreading.
//!
//! The sharp case is a frame that would fail typed parsing under the current schema
//! but arrives under a wrong major. If the version gate ran after interpretation, the
//! caller would see a payload error and be tempted to "fix up" the message; the tests
//! below pin the order by asserting which code comes back.

#![allow(clippy::unwrap_used)]

use ai_platform_contract::failure::FailureKind;
use ai_platform_contract::frame::{self, Frame, Method, Peer};
use ai_platform_contract::version::{
    gate_schema_version, SchemaVersion, VersionDecision, CONSUMER_SCHEMA_MAJOR,
    CONSUMER_SCHEMA_MINOR,
};

/// A request shaped to fail typed parsing: a request with no `id`.
fn request_without_id(major: u16, minor: u16) -> String {
    format!(
        r#"{{"kind":"request","schema_version":{{"major":{major},"minor":{minor}}},"method":"model.complete","params":{{"prompt":"hello"}}}}"#
    )
}

#[test]
fn a_newer_major_is_rejected_before_the_payload_is_interpreted() {
    let line = request_without_id(CONSUMER_SCHEMA_MAJOR + 1, 0);
    let error = frame::decode(&line, Peer::Shell).unwrap_err();

    assert_eq!(error.kind, FailureKind::Unsupported);
    assert_eq!(
        error.code, "schema.major_unsupported",
        "a version rejection must not arrive disguised as a payload error"
    );
    assert!(error.detail.unwrap().contains("payload was not interpreted"));
}

#[test]
fn an_older_major_is_rejected_rather_than_assumed_compatible() {
    // "It still parses" is how a contract break turns into a data bug: a field that
    // was renamed reads as absent, and absent reads as false.
    let line = request_without_id(0, 9);
    let error = frame::decode(&line, Peer::Shell).unwrap_err();
    assert_eq!(error.kind, FailureKind::Unsupported);
    assert_eq!(error.code, "schema.major_unsupported");
}

#[test]
fn a_matching_major_with_a_higher_minor_is_accepted_and_unknown_fields_ignored() {
    // Additive-only minor versions: this is the forward-compatibility half, and it is
    // a behaviour of the codec, not a hope.
    let line = r#"{"kind":"request","schema_version":{"major":1,"minor":99},"id":7,"method":"model.complete","params":{"prompt":"hi","a_field_from_the_future":{"nested":true}}}"#;
    // Issued by Shell, the only side allowed to name `model.complete`. The side gate
    // runs regardless of version compatibility, and it rejected an earlier draft of
    // this test for naming the wrong peer.
    let decoded = frame::decode(line, Peer::Shell).unwrap();

    let Frame::Request(request) = decoded else {
        panic!("expected a request frame, got {decoded:?}");
    };
    assert_eq!(request.id, 7);
    assert_eq!(request.method, Method::CompleteModel);
    assert_eq!(request.schema_version.minor, 99);
    assert_eq!(request.params["a_field_from_the_future"]["nested"], serde_json::json!(true));
}

#[test]
fn the_gate_reports_accept_only_for_the_supported_major() {
    assert_eq!(gate_schema_version(&SchemaVersion::current()), VersionDecision::Accept);
    assert_eq!(
        gate_schema_version(&SchemaVersion { major: CONSUMER_SCHEMA_MAJOR, minor: CONSUMER_SCHEMA_MINOR + 5 }),
        VersionDecision::Accept
    );

    for major in [CONSUMER_SCHEMA_MAJOR - 1, CONSUMER_SCHEMA_MAJOR + 1, CONSUMER_SCHEMA_MAJOR + 7] {
        match gate_schema_version(&SchemaVersion { major, minor: 0 }) {
            VersionDecision::Reject(failure) => {
                assert_eq!(failure.kind, FailureKind::Unsupported);
                assert!(!failure.retryable, "a version mismatch cannot be retried away");
                assert!(!failure.replan_required, "replanning cannot fix a protocol mismatch");
            }
            VersionDecision::Accept => panic!("major {major} must not be accepted"),
        }
    }
}

#[test]
fn every_frame_this_crate_produces_carries_the_current_version() {
    let frames = [
        Frame::request(1, Method::Ping, &serde_json::json!({})),
        Frame::notification(Method::PublishProgress, &serde_json::json!({})),
        Frame::ok(1, frame::ResponsePayload::Ok(serde_json::json!(null))),
        Frame::failure(1, ai_platform_contract::failure::Failure::cancelled("task.stopped")),
    ];
    for frame in frames {
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["schema_version"]["major"], serde_json::json!(CONSUMER_SCHEMA_MAJOR));
        assert_eq!(json["schema_version"]["minor"], serde_json::json!(CONSUMER_SCHEMA_MINOR));
    }
}
