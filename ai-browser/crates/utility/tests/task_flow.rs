#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The task flow against an in-memory shell: citations, routing constraints,
//! cancel, deadline, staleness and the act loop. One `task.finish`, always.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use ai_platform_contract::config::AiConfig;
use ai_platform_contract::contract::{CancelReason, TaskOutcome, TaskRequestView};
use ai_platform_contract::failure::{Failure, FailureKind};
use ai_platform_contract::frame::{Method, ResponsePayload};
use ai_platform_contract::identity::DocumentToken;
use ai_platform_contract::model::BrowserEvent;
use ai_utility::control::{now_ms, StopReason, TaskControl};
use ai_utility::tabs::Tabs;
use ai_utility::task::{self, TaskEnv};
use common::*;
use serde_json::{json, Value};

const URL: &str = "https://shop.example/returns";

fn view(intent: &str, question: &str, options: Value) -> TaskRequestView {
    view_with(1, 10, 1, intent, question, "navigate", 0, options)
}

#[allow(clippy::too_many_arguments)]
fn view_with(task: u64, doc: u64, seq: u64, intent: &str, question: &str, ceiling: &str, deadline: i64, options: Value) -> TaskRequestView {
    let mut options = options;
    options["intent"] = json!(intent);
    serde_json::from_value(json!({
        "task": task,
        "question": question,
        "document": claim(doc, seq),
        "snapshot_version": 1,
        "action_ceiling": ceiling,
        "cancel": {"task": task, "deadline_ms_since_epoch": deadline},
        "options": options,
    }))
    .unwrap()
}

fn claim(doc: u64, seq: u64) -> Value {
    json!({"profile_id": 1, "workspace_id": 1, "tab_id": 1, "frame_tree_node_id": 0, "document_token": doc, "navigation_sequence": seq})
}

fn snapshot(doc: u64, seq: u64) -> Value {
    let mut blocks = vec![];
    for b in page_blocks() {
        blocks.push(json!({"block_id": b.block_id, "text": b.text}));
    }
    json!({
        "document": claim(doc, seq),
        "snapshot_version": 1,
        "captured_at_ms": 0,
        "url": URL,
        "origin": "https://shop.example",
        "title": "Returns",
        "coverage": {},
        "payload": {
            "blocks": blocks,
            "links": [
                {"link_id": "l0", "text": "Pricing", "url": "https://shop.example/pricing"},
                {"link_id": "l1", "text": "Contact support", "url": "https://shop.example/contact"}
            ],
            "frames_not_read": 0,
            "dropped_blocks": 0
        }
    })
}

fn action_result(approval_required: bool, granted: bool, blocked_by: Option<&str>) -> Value {
    let mut decision = json!({"level": "navigate", "approval_required": approval_required, "approval_granted": granted});
    if let Some(by) = blocked_by {
        decision["blocked_by"] = json!(by);
    }
    json!({"action_id": "x", "decision": decision, "verified": true, "result_payload": {"opened": true}})
}

fn plain_shell() -> FakeShell {
    FakeShell::new(|method, _params, _control| match method {
        Method::GetSnapshot => Ok(ResponsePayload::Ok(snapshot(10, 1))),
        Method::DispatchAction => Ok(ResponsePayload::Ok(action_result(false, true, None))),
        _ => Err(Failure::unsupported("test.unexpected")),
    })
}

fn config(extra: Value) -> AiConfig {
    serde_json::from_value(extra).unwrap()
}

fn http_config(base_url: &str) -> AiConfig {
    config(json!({
        "providers": [{
            "id": "fake", "kind": "open_ai_compatible", "enabled": true, "execution": "local",
            "base_url": base_url,
            "models": [{"model_id": "fake-model", "supports_tools": true}],
        }]
    }))
}

fn run(shell: &FakeShell, tabs: &Tabs, view: &TaskRequestView, config: &AiConfig) -> TaskOutcome {
    let control = TaskControl::new(task::effective_deadline(view.cancel.deadline_ms_since_epoch, config));
    run_with(shell, tabs, view, config, &control)
}

fn run_with(shell: &FakeShell, tabs: &Tabs, view: &TaskRequestView, config: &AiConfig, control: &TaskControl) -> TaskOutcome {
    task::run(&TaskEnv { shell, tabs, credentials: &Default::default() }, view, control, config, None);
    let finishes = shell.finishes();
    assert_eq!(finishes.len(), 1, "exactly one task.finish");
    serde_json::from_value(finishes[0].clone()).unwrap()
}

fn failed(outcome: &TaskOutcome) -> &Failure {
    match outcome {
        TaskOutcome::Failed(f) => &f.failure,
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn extractive_answer_streams_densely_and_cites_verbatim() {
    let shell = plain_shell();
    let outcome = run(&shell, &Tabs::default(), &view("ask", QUESTION, json!({})), &AiConfig::default());
    let TaskOutcome::Answered(answer) = outcome else { panic!("{outcome:?}") };
    assert_eq!(answer.citations[0].anchor.block_id.as_deref(), Some("b0"));
    for citation in &answer.citations {
        let block = page_blocks().into_iter().find(|b| Some(b.block_id.as_str()) == citation.anchor.block_id.as_deref()).unwrap();
        assert!(block.text.contains(&citation.anchor.quote));
        assert_eq!(citation.navigation_sequence, 1);
    }
    let events = shell.events();
    let seqs: Vec<u64> = events.iter().filter(|e| e["event"] == "delta").map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(!seqs.is_empty());
    assert_eq!(seqs, (0..seqs.len() as u64).collect::<Vec<_>>(), "delta seq is dense from 0");
    let streamed: String = events.iter().filter(|e| e["event"] == "delta").map(|e| e["text"].as_str().unwrap().to_string()).collect();
    assert!(answer.answer.starts_with(streamed.trim_end()) || streamed.contains(answer.answer.trim_end()));
    let routed = events.iter().find(|e| e["event"] == "routed").expect("routed event");
    assert_eq!(routed["chosen"]["provider_id"], "extractive");
    assert_eq!(events.last().unwrap()["stage"], "finished");
    assert_eq!(shell.called(Method::GetSnapshot), 1);
}

#[test]
fn citations_that_are_not_verbatim_are_rejected_and_reported() {
    let text = "ANSWER: Returns take 90 days.\nQUOTE b0: Returns are accepted within 90 days of delivery.\nQUOTE b0: Returns are accepted within 30 days of delivery.";
    let server = FakeServer::start(script_for(text));
    let outcome = run(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &http_config(&server.base_url));
    let TaskOutcome::Answered(answer) = outcome else { panic!("{outcome:?}") };
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].anchor.quote, "Returns are accepted within 30 days of delivery.");
    assert_eq!(answer.limit.expect("rejection is reported").dropped_items, 1);
    assert!(!answer.answer.contains("SECRET-REASONING"));
}

#[test]
fn a_model_that_only_invents_quotes_yields_an_uncited_answer() {
    let server = FakeServer::start(script_for("ANSWER: Whatever.\nQUOTE b0: This sentence is not on the page."));
    let outcome = run(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &http_config(&server.base_url));
    let TaskOutcome::Answered(answer) = outcome else { panic!("{outcome:?}") };
    assert!(answer.citations.is_empty());
    assert_eq!(answer.limit.unwrap().dropped_items, 1);
}

#[test]
fn empty_reasoning_only_reply_is_no_answer_with_the_budget_numbers() {
    let server = FakeServer::start(Script { status: 200, steps: vec![reasoning_chunk("hmm"), finish_chunk("length", Some(4096))] });
    let outcome = run(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &http_config(&server.base_url));
    let TaskOutcome::NoAnswer(none) = outcome else { panic!("{outcome:?}") };
    assert_eq!(none.failure.code, "model.reasoning_consumed_budget");
    assert_eq!(none.reasoning_tokens, Some(4096));
}

#[test]
fn extractive_no_match_is_no_answer() {
    let outcome = run(&plain_shell(), &Tabs::default(), &view("ask", "What is the airspeed of a swallow?", json!({})), &AiConfig::default());
    assert!(matches!(outcome, TaskOutcome::NoAnswer(_)), "{outcome:?}");
}

#[test]
fn force_local_never_touches_the_cloud_model() {
    let server = FakeServer::start(good_answer_script());
    let mut cfg = http_config(&server.base_url);
    cfg.providers[0].execution = ai_platform_contract::model::Execution::Cloud;
    cfg.flags.cloud_models_enabled = true;
    let shell = plain_shell();
    run(&shell, &Tabs::default(), &view("ask", QUESTION, json!({})), &cfg);
    assert_eq!(server.requests().len(), 1, "sanity: cloud is used when allowed");

    let shell = plain_shell();
    let outcome = run(&shell, &Tabs::default(), &view("ask", QUESTION, json!({"force_local": true})), &cfg);
    assert!(matches!(outcome, TaskOutcome::Answered(_)));
    assert_eq!(server.requests().len(), 1, "force_local sent nothing to the cloud model");
    assert_eq!(shell.events().iter().find(|e| e["event"] == "routed").unwrap()["chosen"]["provider_id"], "extractive");

    cfg.domains.cloud_denied = vec!["shop.example".into()];
    run(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &cfg);
    assert_eq!(server.requests().len(), 1, "a cloud-denied domain is a hard constraint");
}

#[test]
fn provider_failure_falls_back_along_the_chain() {
    let server = FakeServer::start(Script { status: 503, steps: vec![] });
    let outcome = run(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &http_config(&server.base_url));
    assert!(matches!(outcome, TaskOutcome::Answered(_)), "{outcome:?}");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn kill_switches_and_bad_config_fail_closed() {
    let shell = plain_shell();
    let mut cfg = AiConfig::default();
    cfg.flags.ai_enabled = false;
    let outcome = run(&shell, &Tabs::default(), &view("ask", QUESTION, json!({})), &cfg);
    assert_eq!(failed(&outcome).kind, FailureKind::Blocked);
    assert_eq!(shell.called(Method::GetSnapshot), 0, "no capture when AI is off");

    let shell = plain_shell();
    let mut cfg = AiConfig::default();
    cfg.flags.page_context = false;
    assert_eq!(failed(&run(&shell, &Tabs::default(), &view("ask", QUESTION, json!({})), &cfg)).code, "config.page_context_disabled");

    let shell = plain_shell();
    let v = view("ask", QUESTION, json!({}));
    task::run(&TaskEnv { shell: &shell, tabs: &Tabs::default(), credentials: &Default::default() }, &v, &TaskControl::new(0), &AiConfig::disabled(), Some(Failure::unsupported("config.invalid")));
    let outcome: TaskOutcome = serde_json::from_value(shell.finishes()[0].clone()).unwrap();
    assert_eq!(failed(&outcome).code, "config.invalid");
}

#[test]
fn navigation_during_snapshot_makes_the_task_stale_and_dispatches_nothing() {
    let tabs = Arc::new(Tabs::default());
    let shell_tabs = Arc::clone(&tabs);
    let shell = FakeShell::new(move |method, _p, control| match method {
        Method::GetSnapshot => {
            // The browser reports the navigation while the snapshot is in flight.
            let out = shell_tabs.apply(&BrowserEvent::DocumentDestroyed { document: DocumentToken(10) });
            assert_eq!(out.dead, vec![DocumentToken(10)]);
            control.stop(StopReason::Stale("task.document_replaced"));
            Ok(ResponsePayload::Ok(snapshot(10, 1)))
        }
        _ => panic!("nothing else may be called after staleness"),
    });
    let outcome = run(&shell, &tabs, &view("act", "open link 0", json!({})), &AiConfig::default());
    let failure = failed(&outcome);
    assert_eq!((failure.kind, failure.code.as_str()), (FailureKind::Stale, "task.document_replaced"));
    assert_eq!(shell.called(Method::DispatchAction), 0);
}

#[test]
fn a_task_on_an_already_replaced_document_never_reads_it() {
    let tabs = Tabs::default();
    tabs.apply(&BrowserEvent::DocumentDestroyed { document: DocumentToken(10) });
    let shell = plain_shell();
    let outcome = run(&shell, &tabs, &view("ask", QUESTION, json!({})), &AiConfig::default());
    assert_eq!(failed(&outcome).kind, FailureKind::Stale);
    assert_eq!(shell.called(Method::GetSnapshot), 0);
}

#[test]
fn a_snapshot_of_a_different_document_is_stale() {
    let shell = FakeShell::new(|_, _, _| Ok(ResponsePayload::Ok(snapshot(10, 2))));
    let outcome = run(&shell, &Tabs::default(), &view("ask", QUESTION, json!({})), &AiConfig::default());
    assert_eq!(failed(&outcome).kind, FailureKind::Stale);
}

#[test]
fn staleness_during_the_model_call_withholds_the_answer() {
    let mut script = good_answer_script();
    script.steps.insert(1, Step::Sleep(600));
    let server = FakeServer::start(script);
    let control = Arc::new(TaskControl::new(0));
    let stopper = Arc::clone(&control);
    let shell = FakeShell::new(move |method, _p, _c| match method {
        Method::GetSnapshot => {
            let stopper = Arc::clone(&stopper);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                stopper.stop(StopReason::Stale("task.document_replaced"));
            });
            Ok(ResponsePayload::Ok(snapshot(10, 1)))
        }
        _ => Err(Failure::unsupported("x")),
    });
    let outcome = run_with(&shell, &Tabs::default(), &view("ask", QUESTION, json!({})), &http_config(&server.base_url), &control);
    assert_eq!(failed(&outcome).kind, FailureKind::Stale);
}

#[test]
fn user_cancel_mid_stream_still_finishes_once_as_cancelled() {
    let server = FakeServer::start(Script { status: 200, steps: vec![content_chunk("ANSWER: sl"), Step::Sleep(10_000)] });
    let control = Arc::new(TaskControl::new(0));
    let stopper = Arc::clone(&control);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stopper.stop(StopReason::User(CancelReason::UserStopped));
    });
    let started = Instant::now();
    let outcome = run_with(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &http_config(&server.base_url), &control);
    assert_eq!(failed(&outcome).kind, FailureKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn request_deadline_is_enforced_and_reported_as_timed_out() {
    let server = FakeServer::start(Script { status: 200, steps: vec![Step::Sleep(10_000)] });
    let v = view_with(1, 10, 1, "ask", QUESTION, "navigate", now_ms() + 400, json!({}));
    let started = Instant::now();
    let outcome = run(&plain_shell(), &Tabs::default(), &v, &http_config(&server.base_url));
    assert_eq!(failed(&outcome).kind, FailureKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));

    let past = view_with(1, 10, 1, "ask", QUESTION, "navigate", now_ms() - 1, json!({}));
    let shell = plain_shell();
    assert_eq!(failed(&run(&shell, &Tabs::default(), &past, &AiConfig::default())).kind, FailureKind::TimedOut);
}

#[test]
fn configured_deadline_applies_when_the_request_has_none() {
    let server = FakeServer::start(Script { status: 200, steps: vec![Step::Sleep(10_000)] });
    let mut cfg2 = http_config(&server.base_url);
    cfg2.task_deadline_ms = 250;
    let started = Instant::now();
    let outcome = run(&plain_shell(), &Tabs::default(), &view("ask", QUESTION, json!({})), &cfg2);
    assert_eq!(failed(&outcome).kind, FailureKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(task::effective_deadline(0, &cfg2) > 0 && task::effective_deadline(5, &cfg2) == 5);
}

#[test]
fn act_opens_only_a_snapshot_link_with_the_snapshot_url() {
    let shell = plain_shell();
    let outcome = run(&shell, &Tabs::default(), &view("act", "open link 1", json!({})), &AiConfig::default());
    assert!(matches!(outcome, TaskOutcome::Answered(_)), "{outcome:?}");
    let sent = shell.calls_of(Method::DispatchAction);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["action_kind"], "open_tab");
    assert_eq!(sent[0]["expected_document"], 10);
    assert_eq!(sent[0]["target"]["opaque_target"]["link_id"], "l1");
    assert_eq!(sent[0]["arguments"]["url"], "https://shop.example/contact");

    let shell = plain_shell();
    let outcome = run(&shell, &Tabs::default(), &view("act", "open link 7", json!({})), &AiConfig::default());
    assert!(matches!(outcome, TaskOutcome::NoAnswer(_)), "{outcome:?}");
    assert_eq!(shell.called(Method::DispatchAction), 0, "an unknown link is never invented");
}

#[test]
fn act_by_link_text() {
    let shell = plain_shell();
    run(&shell, &Tabs::default(), &view("act", "go to Contact support", json!({})), &AiConfig::default());
    assert_eq!(shell.calls_of(Method::DispatchAction)[0]["arguments"]["url"], "https://shop.example/contact");
}

#[test]
fn act_handles_denial_policy_block_and_failure_results() {
    let denied = FakeShell::new(|m, _, _| match m {
        Method::GetSnapshot => Ok(ResponsePayload::Ok(snapshot(10, 1))),
        _ => Ok(ResponsePayload::Ok(action_result(true, false, None))),
    });
    let outcome = run(&denied, &Tabs::default(), &view("act", "open link 0", json!({})), &AiConfig::default());
    assert_eq!(failed(&outcome).code, "action.approval_denied");

    let blocked = FakeShell::new(|m, _, _| match m {
        Method::GetSnapshot => Ok(ResponsePayload::Ok(snapshot(10, 1))),
        _ => Ok(ResponsePayload::Ok(action_result(false, false, Some("kill_switch")))),
    });
    let outcome = run(&blocked, &Tabs::default(), &view("act", "open link 0", json!({})), &AiConfig::default());
    assert_eq!(failed(&outcome).kind, FailureKind::Blocked);

    let stale = FakeShell::new(|m, _, _| match m {
        Method::GetSnapshot => Ok(ResponsePayload::Ok(snapshot(10, 1))),
        _ => Ok(ResponsePayload::Failure(Failure::stale("action.document_replaced"))),
    });
    let outcome = run(&stale, &Tabs::default(), &view("act", "open link 0", json!({})), &AiConfig::default());
    assert_eq!(failed(&outcome).kind, FailureKind::Stale);

    // A non-fatal failure is an observation; the extractive planner then gives up honestly.
    let flaky = FakeShell::new(|m, _, _| match m {
        Method::GetSnapshot => Ok(ResponsePayload::Ok(snapshot(10, 1))),
        _ => Ok(ResponsePayload::Failure(Failure::unavailable("action.tab_busy"))),
    });
    let outcome = run(&flaky, &Tabs::default(), &view("act", "open link 0", json!({})), &AiConfig::default());
    assert!(matches!(outcome, TaskOutcome::NoAnswer(_)), "{outcome:?}");
    assert_eq!(flaky.called(Method::DispatchAction), 1);
}

#[test]
fn act_respects_ceiling_and_config_and_domain_policy() {
    let shell = plain_shell();
    let observe = view_with(1, 10, 1, "act", "open link 0", "observe", 0, json!({}));
    assert_eq!(failed(&run(&shell, &Tabs::default(), &observe, &AiConfig::default())).kind, FailureKind::Blocked);
    assert_eq!(shell.called(Method::GetSnapshot), 0);

    let mut cfg = AiConfig::default();
    cfg.flags.max_action_level = "observe".into();
    let shell = plain_shell();
    assert_eq!(failed(&run(&shell, &Tabs::default(), &view("act", "open link 0", json!({})), &cfg)).code, "config.actions_disabled");

    let mut cfg = AiConfig::default();
    cfg.domains.automation_denied = vec!["shop.example".into()];
    let shell = plain_shell();
    let outcome = run(&shell, &Tabs::default(), &view("act", "open link 0", json!({})), &cfg);
    assert!(matches!(outcome, TaskOutcome::NoAnswer(_)), "{outcome:?}");
    assert_eq!(shell.called(Method::DispatchAction), 0, "automation-denied domains are observe-only");
}

#[test]
fn model_driven_act_loop_is_bounded_and_refuses_invented_targets() {
    // The model keeps proposing an invented link; each is refused and fed back until the step limit.
    let server = FakeServer::start(script_for(r#"{"action":"open_tab","link_id":"l99"}"#));
    let shell = plain_shell();
    let outcome = run(&shell, &Tabs::default(), &view("act", "open the secret page", json!({})), &http_config(&server.base_url));
    assert_eq!(failed(&outcome).code, "act.step_limit");
    assert_eq!(server.requests().len(), 6);
    assert_eq!(shell.called(Method::DispatchAction), 0);
    let last = server.requests().pop().unwrap();
    assert!(last.body["messages"][1]["content"].as_str().unwrap().contains("link_id is not in the page links"));

    // A valid proposal is dispatched with the snapshot's URL, never the model's.
    let server = FakeServer::start(script_for(r#"{"action":"open_tab","link_id":"l0","url":"https://evil.example"}"#));
    let shell = plain_shell();
    run(&shell, &Tabs::default(), &view("act", "open pricing", json!({})), &http_config(&server.base_url));
    let sent = shell.calls_of(Method::DispatchAction);
    assert!(!sent.is_empty());
    assert_eq!(sent[0]["arguments"]["url"], "https://shop.example/pricing");
}
