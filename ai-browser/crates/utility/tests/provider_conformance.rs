#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The same behavioural suite against both providers, plus HTTP-only wire checks
//! against the in-process fake SSE server.

mod common;

use std::time::{Duration, Instant};

use ai_platform_contract::contract::CancelReason;
use ai_platform_contract::failure::FailureKind;
use ai_platform_contract::model::PageBlock;
use ai_utility::control::{now_ms, StopReason, TaskControl};
use ai_utility::provider::extractive::ExtractiveProvider;
use ai_utility::provider::openai::OpenAiCompatibleProvider;
use ai_utility::provider::{ModelProvider, ProviderReply};
use ai_utility::reply;
use common::*;

fn run_collect(provider: &dyn ModelProvider, control: &TaskControl) -> (Result<ProviderReply, ai_platform_contract::failure::Failure>, Vec<String>) {
    let mut deltas = Vec::new();
    let result = provider.complete(&completion_request(QUESTION), control, &mut |d| deltas.push(d.to_string()));
    (result, deltas)
}

/// What every provider must satisfy, whatever is behind it.
fn conformance(provider: &dyn ModelProvider) {
    // Streams, and the stream is exactly the reply.
    let (result, deltas) = run_collect(provider, &TaskControl::new(0));
    let reply_ = result.unwrap();
    assert!(!deltas.is_empty(), "no deltas streamed");
    assert_eq!(deltas.concat(), reply_.text, "deltas must concatenate to the reply");
    assert!(!reply_.text.contains("SECRET-REASONING"), "reasoning must never surface");

    // The text contract holds and its quotes survive verbatim verification.
    let parsed = reply::parse(&reply_.text);
    assert!(parsed.had_markers && !parsed.answer.is_empty(), "reply: {}", reply_.text);
    assert!(!parsed.quotes.is_empty());
    let blocks: Vec<PageBlock> = page_blocks();
    let (verified, rejected) = reply::verify_quotes(&parsed.quotes, &blocks);
    assert_eq!((verified.len(), rejected), (parsed.quotes.len(), 0));
    assert_eq!(verified[0].block_id, "b0");

    // Cancelled before the call: no work, a cancelled failure.
    let control = TaskControl::new(0);
    control.stop(StopReason::User(CancelReason::UserStopped));
    let (result, deltas) = run_collect(provider, &control);
    assert_eq!(result.unwrap_err().kind, FailureKind::Cancelled);
    assert!(deltas.is_empty());

    // Cancelled mid-stream: returns promptly with a cancelled failure.
    let control = TaskControl::new(0);
    let mut seen = 0;
    let result = provider.complete(&completion_request(QUESTION), &control, &mut |_| {
        seen += 1;
        control.stop(StopReason::User(CancelReason::UserStopped));
    });
    assert_eq!(result.unwrap_err().kind, FailureKind::Cancelled);
    assert_eq!(seen, 1);

    // An elapsed deadline is a timeout, not a cancel.
    let (result, _) = run_collect(provider, &TaskControl::new(now_ms() - 1));
    assert_eq!(result.unwrap_err().kind, FailureKind::TimedOut);

    // Stale beats everything and is reported as stale.
    let control = TaskControl::new(0);
    control.stop(StopReason::Stale("task.document_replaced"));
    let (result, _) = run_collect(provider, &control);
    assert_eq!(result.unwrap_err().kind, FailureKind::Stale);
}

#[test]
fn extractive_provider_conforms() {
    conformance(&ExtractiveProvider::new("extractive"));
}

#[test]
fn http_provider_conforms() {
    let server = FakeServer::start(good_answer_script());
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, Some("test-key-123".into())).unwrap();
    conformance(&provider);
}

#[test]
fn http_request_carries_the_wire_contract() {
    let server = FakeServer::start(good_answer_script());
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, Some("test-key-123".into())).unwrap();
    run_collect(&provider, &TaskControl::new(0)).0.unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.authorization.as_deref(), Some("Bearer test-key-123"));
    assert_eq!(request.body["stream"], true);
    assert_eq!(request.body["model"], "fake-model");
    assert!(request.body["max_tokens"].as_u64().unwrap() >= 1024, "reasoning models need a generous budget");
    let messages = request.body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[0]["content"].as_str().unwrap().contains("UNTRUSTED DATA"));
    assert!(messages[1]["content"].as_str().unwrap().contains(BLOCK_0));
    assert!(!messages[0]["content"].as_str().unwrap().contains(BLOCK_0), "page text stays out of the system prompt");
}

#[test]
fn no_key_sends_no_authorization_header() {
    let server = FakeServer::start(good_answer_script());
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, None).unwrap();
    run_collect(&provider, &TaskControl::new(0)).0.unwrap();
    assert_eq!(server.requests()[0].authorization, None);
}

#[test]
fn reasoning_only_reply_is_empty_and_reports_the_reasoning_tokens() {
    let server = FakeServer::start(Script {
        status: 200,
        steps: vec![reasoning_chunk("SECRET-REASONING 1"), reasoning_chunk("SECRET-REASONING 2"), finish_chunk("length", Some(4096))],
    });
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, None).unwrap();
    let (result, deltas) = run_collect(&provider, &TaskControl::new(0));
    let reply_ = result.unwrap();
    assert!(reply_.text.is_empty() && deltas.is_empty());
    assert_eq!(reply_.finish_reason.as_deref(), Some("length"));
    assert_eq!(reply_.reasoning_tokens, Some(4096));
    assert_eq!(reply_.completion_token_budget, 4096);
}

#[test]
fn http_errors_map_to_the_failure_taxonomy_without_leaking_bodies() {
    for (status, kind) in [(401, FailureKind::Blocked), (429, FailureKind::Unavailable), (503, FailureKind::Unavailable), (400, FailureKind::Unsupported)] {
        let server = FakeServer::start(Script { status, steps: vec![] });
        let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, Some("test-key-123".into())).unwrap();
        let failure = run_collect(&provider, &TaskControl::new(0)).0.unwrap_err();
        assert_eq!(failure.kind, kind, "status {status}");
        let shown = format!("{failure:?}");
        assert!(!shown.contains("SECRET-BODY") && !shown.contains("test-key-123"));
    }
}

#[test]
fn unreachable_server_is_unavailable() {
    let provider = OpenAiCompatibleProvider::new("fake", "http://127.0.0.1:1/v1", None).unwrap();
    let failure = run_collect(&provider, &TaskControl::new(0)).0.unwrap_err();
    assert_eq!(failure.kind, FailureKind::Unavailable);
}

#[test]
fn stalled_server_cannot_hold_the_task_past_its_deadline() {
    let server = FakeServer::start(Script { status: 200, steps: vec![content_chunk("ANSWER: sl"), Step::Sleep(10_000)] });
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, None).unwrap();
    let started = Instant::now();
    let (result, _) = run_collect(&provider, &TaskControl::new(now_ms() + 400));
    assert_eq!(result.unwrap_err().kind, FailureKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn stalled_server_cannot_hold_a_cancel() {
    let server = FakeServer::start(Script { status: 200, steps: vec![Step::Sleep(10_000)] });
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, None).unwrap();
    let control = std::sync::Arc::new(TaskControl::new(0));
    let stopper = std::sync::Arc::clone(&control);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        stopper.stop(StopReason::User(CancelReason::UserStopped));
    });
    let started = Instant::now();
    let (result, _) = run_collect(&provider, &control);
    assert_eq!(result.unwrap_err().kind, FailureKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn non_streaming_body_is_still_understood() {
    let body = serde_json::json!({
        "choices": [{"message": {"content": "ANSWER: ok\nQUOTE b0: Returns are accepted within 30 days of delivery.", "reasoning_content": "SECRET-REASONING"}, "finish_reason": "stop"}]
    });
    let server = FakeServer::start(Script { status: 200, steps: vec![Step::Raw(body.to_string())] });
    let provider = OpenAiCompatibleProvider::new("fake", &server.base_url, None).unwrap();
    let reply_ = run_collect(&provider, &TaskControl::new(0)).0.unwrap();
    assert!(reply_.text.starts_with("ANSWER: ok") && !reply_.text.contains("SECRET"));
}
