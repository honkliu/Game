//! Cancellation and kill against a real child process, not a mock.
//!
//! `src/bin/stub_utility.rs` is spawned as an actual OS process so the two paths
//! `supervise` documents are both exercised: a utility that is busy but listening, and
//! one that has stopped reading its stdin and therefore cannot be asked to stop.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ai_platform_contract::contract::{CancelReason, CancelledAt};
use ai_platform_contract::failure::FailureKind;
use ai_platform_contract::frame::{Method, ResponsePayload};
use ai_platform_contract::identity::TaskId;
use ai_platform_contract::supervise::UtilityProcess;

const STUB: &str = env!("CARGO_BIN_EXE_stub_utility");

fn spawn_stub() -> UtilityProcess {
    UtilityProcess::spawn(&PathBuf::from(STUB), &[]).expect("stub utility spawns")
}

#[test]
fn a_frame_round_trips_over_real_pipes() {
    let mut utility = spawn_stub();
    let id = utility.begin_call(Method::Ping, None, false, &serde_json::json!({})).unwrap();
    let payload = utility.await_response(id, Duration::from_secs(5), |_| {}).unwrap();

    match payload {
        ResponsePayload::Ok(value) => assert_eq!(value, serde_json::json!("pong")),
        ResponsePayload::Failure(failure) => panic!("ping failed: {failure:?}"),
    }
    assert!(utility.is_running(), "a live utility stays live between calls");
}

#[test]
fn a_busy_but_listening_utility_is_cancelled_without_being_killed() {
    let mut utility = spawn_stub();
    let task = TaskId(1);
    // Accepted, never answered, still reading stdin: the normal shape of a slow
    // provider call that the user decides to stop.
    let wedged = utility
        .begin_call(
            Method::CompleteModel,
            Some(task),
            false,
            &serde_json::json!({"hang": true}),
        )
        .unwrap();
    let started = Instant::now();
    assert_eq!(
        utility.await_response(wedged, Duration::from_millis(100), |_| {}).unwrap_err().code,
        "utility.response_deadline"
    );

    let result = utility.cancel(task, CancelReason::UserStopped, Duration::from_secs(5)).unwrap();
    assert_eq!(result.task, task);
    assert_eq!(result.cancelled_at, CancelledAt::BetweenActions);
    assert_eq!(result.actions_completed, 1);
    assert!(!result.effect_unverified);
    assert!(utility.is_running(), "a cooperative stop must not kill the process");
    assert!(started.elapsed() < Duration::from_secs(5), "cancel returns as soon as the reply lands");

    // The unanswered model call is still owed to its caller, and is reported as
    // abandoned rather than vanishing when the process is finally torn down.
    let abandoned = utility.abort(Duration::from_millis(500));
    assert_eq!(abandoned.len(), 1);
    assert_eq!(abandoned[0].task, Some(task));
    assert!(!abandoned[0].effect_unverified);
    assert_eq!(abandoned[0].failure.kind, FailureKind::Unavailable);
    assert_eq!(abandoned[0].failure.code, "utility.process_lost");
    assert!(!utility.is_running());
}

#[test]
fn a_utility_that_stopped_reading_stdin_is_killed_and_reaped() {
    let mut utility = spawn_stub();
    let task = TaskId(7);
    // `uninterruptible` makes the stub stop reading its stdin entirely, so no
    // cancellation message can reach it. This is the case the forced path exists for.
    let in_flight = utility
        .begin_call(
            Method::CompleteModel,
            Some(task),
            // The contract is that this call may have written to the page before dying.
            true,
            &serde_json::json!({"hang": true, "uninterruptible": true}),
        )
        .unwrap();
    assert_eq!(
        utility.await_response(in_flight, Duration::from_millis(100), |_| {}).unwrap_err().code,
        "utility.response_deadline"
    );

    // Escalation: EOF on stdin, a grace period, then terminate and reap.
    let started = Instant::now();
    let abandoned = utility.abort(Duration::from_millis(250));
    assert!(started.elapsed() >= Duration::from_millis(250), "the grace period is honoured");
    assert!(!utility.is_running(), "the process is reaped, not left running");

    assert_eq!(abandoned.len(), 1);
    assert_eq!(abandoned[0].request_id, in_flight);
    assert_eq!(abandoned[0].task, Some(task));
    assert_eq!(abandoned[0].failure.kind, FailureKind::Unavailable);
    assert_eq!(abandoned[0].failure.code, "utility.process_lost");
    assert!(abandoned[0].failure.retryable);
    assert!(
        abandoned[0].effect_unverified,
        "a killed action may already have reached the page; the caller must re-observe"
    );
}

#[test]
fn writing_to_a_dead_utility_reports_a_closed_pipe_not_an_io_error() {
    let mut utility = spawn_stub();
    let abandoned = utility.abort(Duration::from_millis(250));
    assert!(abandoned.is_empty(), "nothing was outstanding, so nothing is owed to a caller");

    let error = utility
        .begin_call(Method::Ping, None, false, &serde_json::json!({}))
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Unavailable);
    assert_eq!(error.code, "utility.pipe_closed");
    assert!(error.retryable, "restarting the utility is a legitimate recovery");
}

/// The reader thread must end with the child, or every killed utility leaks a thread
/// and a waiter blocks for its full timeout instead of failing at once.
#[test]
fn waiting_after_the_child_died_fails_at_once_instead_of_running_out_the_timeout() {
    let mut utility = spawn_stub();
    // A call the stub deliberately never answers, so the only thing that can end the
    // wait is the pipe closing.
    let pending = utility
        .begin_call(Method::CompleteModel, None, false, &serde_json::json!({"hang": true}))
        .unwrap();
    utility.abort(Duration::from_millis(500));

    let started = Instant::now();
    let error = utility
        .await_response(pending, Duration::from_secs(30), |_| {})
        .expect_err("the reply can no longer arrive");
    assert_eq!(error.code, "utility.pipe_closed");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a dead child must not cost the caller its whole deadline: {:?}",
        started.elapsed()
    );
    // Aborting twice is safe: the first abort already handed out every outstanding call.
    assert!(utility.abort(Duration::from_millis(50)).is_empty());
}
