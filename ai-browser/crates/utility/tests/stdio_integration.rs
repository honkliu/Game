#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Drives the real `ai-utility` binary over stdio as a fake shell would.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use ai_platform_contract::contract::{
    ActionLevel, CancelReason, CancellationRequest, CancellationResult, CancelledAt, CancellationToken, TaskOutcome,
    TaskRequest, TaskRequestView,
};
use ai_platform_contract::failure::{Failure, FailureKind};
use ai_platform_contract::frame::{self, Frame, Method, Peer, Request, ResponsePayload};
use ai_platform_contract::identity::{DocumentIdentityRegistry, DocumentToken, RequestScope, TaskId};
use ai_platform_contract::model::{BrowserEvent, TaskOptions};
use ai_utility::control::now_ms;
use common::*;
use serde_json::{json, Value};

const URL: &str = "https://shop.example/returns";

struct Shell {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    stderr: std::thread::JoinHandle<String>,
    next_id: u64,
    registry: DocumentIdentityRegistry,
    scope: RequestScope,
    document: ai_platform_contract::identity::DocumentKey,
    version: ai_platform_contract::identity::SnapshotVersion,
    tab: ai_platform_contract::identity::TabId,
}

impl Shell {
    fn start(config: Option<Value>, name: &str) -> Shell {
        let path: PathBuf = [env!("CARGO_TARGET_TMPDIR"), &format!("{name}.ai.json")].iter().collect();
        match config {
            Some(config) => std::fs::write(&path, config.to_string()).unwrap(),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_ai-utility"))
            .env("AI_BROWSER_CONFIG", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });

        let mut registry = DocumentIdentityRegistry::new();
        let profile = registry.create_profile();
        let workspace = registry.create_workspace(profile).unwrap();
        let tab = registry.open_tab(workspace).unwrap();
        let document = registry.on_document_committed(tab, 0, None, URL).unwrap();
        let scope = RequestScope { profile_id: profile, workspace_id: workspace };
        let version = registry.next_snapshot_version(document.document_token(), &scope).unwrap();
        Shell { child, stdin, rx, stderr, next_id: 100, registry, scope, document, version, tab }
    }

    fn send(&mut self, frame: &Frame) {
        let line = frame::encode(frame).unwrap();
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    fn send_raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: Method, params: &Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&Frame::request(id, method, params));
        id
    }

    fn recv(&self, within: Duration) -> Option<Frame> {
        let line = self.rx.recv_timeout(within).ok()?;
        Some(frame::decode(&line, Peer::Utility).unwrap_or_else(|e| panic!("utility sent an invalid frame {line}: {e:?}")))
    }

    /// The next request from the utility, skipping progress notifications.
    fn next_request(&self) -> Request {
        loop {
            if let Frame::Request(request) = self.recv(Duration::from_secs(10)).expect("timed out waiting for a request") {
                return request;
            }
        }
    }

    /// Next response to `id`; other frames seen meanwhile are returned too.
    fn response_to(&self, id: u64) -> (ResponsePayload, Vec<Frame>) {
        let mut others = Vec::new();
        loop {
            match self.recv(Duration::from_secs(10)).expect("timed out waiting for a response") {
                Frame::Response(r) if r.id == id => return (r.payload, others),
                other => others.push(other),
            }
        }
    }

    fn new_task(&mut self, question: &str, options: TaskOptions) -> (TaskId, Value) {
        let task = self.registry.create_task(self.scope.workspace_id).unwrap();
        let request = TaskRequest {
            task,
            question: question.into(),
            document: self.document,
            snapshot_version: self.version,
            action_ceiling: ActionLevel::Navigate,
            cancel: CancellationToken { task, deadline_ms_since_epoch: now_ms() + 30_000 },
            options,
        };
        (task, TaskRequestView::of(&request).to_params().unwrap())
    }

    fn snapshot_payload(&self) -> ResponsePayload {
        let blocks: Vec<Value> = page_blocks().iter().map(|b| json!({"block_id": b.block_id, "text": b.text})).collect();
        ResponsePayload::Ok(json!({
            "document": serde_json::to_value(self.document).unwrap(),
            "snapshot_version": self.version,
            "captured_at_ms": now_ms(),
            "url": URL,
            "origin": "https://shop.example",
            "title": "Returns",
            "coverage": {},
            "payload": {"blocks": blocks, "links": [{"link_id": "l0", "text": "Pricing", "url": "https://shop.example/pricing"}]},
        }))
    }

    fn answer_snapshot(&mut self, request: &Request) {
        assert_eq!(request.method, Method::GetSnapshot);
        assert_eq!(request.params["document"], json!(self.document.document_token()));
        let payload = self.snapshot_payload();
        self.send(&Frame::ok(request.id, payload));
    }

    /// Serves utility requests until `task.finish`, then waits for stragglers.
    fn run_until_finish(&mut self, mut on_request: impl FnMut(&mut Shell, &Request)) -> (Vec<Frame>, Vec<TaskOutcome>) {
        let mut seen = Vec::new();
        let mut finishes = Vec::new();
        loop {
            let frame = self.recv(Duration::from_secs(15)).expect("no task.finish");
            match &frame {
                Frame::Request(request) => on_request(self, request),
                Frame::Notification(n) if n.method == Method::FinishTask => {
                    finishes.push(serde_json::from_value(n.params.clone()).unwrap());
                    seen.push(frame);
                    break;
                }
                _ => seen.push(frame),
            }
        }
        while let Some(frame) = self.recv(Duration::from_millis(400)) {
            if let Frame::Notification(n) = &frame {
                if n.method == Method::FinishTask {
                    finishes.push(serde_json::from_value(n.params.clone()).unwrap());
                }
            }
            seen.push(frame);
        }
        (seen, finishes)
    }

    /// Closes stdin and returns everything the process wrote to stderr.
    fn shutdown(mut self) -> String {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "utility exited with {status}");
        self.stderr.join().unwrap()
    }
}

fn stalled_http_config() -> (FakeServer, Value) {
    let server = FakeServer::start(Script { status: 200, steps: vec![Step::Sleep(20_000)] });
    let config = json!({
        "providers": [{"id": "slow", "kind": "open_ai_compatible", "enabled": true, "execution": "local",
                       "base_url": server.base_url, "models": [{"model_id": "m", "supports_tools": true}]}],
    });
    (server, config)
}

fn kinds(frames: &[Frame]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|f| match f {
            Frame::Notification(n) if n.method == Method::PublishProgress => Some(n.params["event"].as_str().unwrap().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn ping_list_and_wrong_direction_frames() {
    let mut shell = Shell::start(None, "ping");
    let id = shell.request(Method::Ping, &json!({}));
    assert!(matches!(shell.response_to(id).0, ResponsePayload::Ok(_)));

    let id = shell.request(Method::ListModels, &json!({}));
    let ResponsePayload::Ok(models) = shell.response_to(id).0 else { panic!() };
    let first = &models.as_array().unwrap()[0];
    assert_eq!(first["manifest"]["provider_id"], "extractive");
    assert_eq!(first["available"], true);

    // A method only the utility may issue, sent by the "shell", is refused with the same id.
    shell.send_raw(r#"{"kind":"request","schema_version":{"major":1,"minor":0},"id":777,"method":"context.get_snapshot","params":{}}"#);
    // Garbage is dropped without killing the loop.
    shell.send_raw("this is not json");
    let id = shell.request(Method::Ping, &json!({}));
    let (payload, others) = shell.response_to(id);
    assert!(matches!(payload, ResponsePayload::Ok(_)));
    let rejected = others.iter().find_map(|f| match f {
        Frame::Response(r) if r.id == 777 => Some(r.payload.clone()),
        _ => None,
    });
    let Some(ResponsePayload::Failure(failure)) = rejected else { panic!("wrong-direction request must be refused by id") };
    assert_eq!(failure.kind, FailureKind::Blocked);
    shell.shutdown();
}

#[test]
fn ask_over_stdio_finishes_exactly_once_with_verifiable_citations() {
    let mut shell = Shell::start(None, "ask");
    let (task, params) = shell.new_task(QUESTION, TaskOptions::default());
    let id = shell.request(Method::CompleteModel, &params);
    let (ack, _) = shell.response_to(id);
    assert!(matches!(ack, ResponsePayload::Ok(_)), "{ack:?}");

    let mut snapshot_requests = 0;
    let (frames, finishes) = shell.run_until_finish(|shell, request| {
        snapshot_requests += 1;
        shell.answer_snapshot(request);
    });
    assert_eq!(snapshot_requests, 1);
    assert_eq!(finishes.len(), 1, "exactly one task.finish");
    let TaskOutcome::Answered(answer) = &finishes[0] else { panic!("{:?}", finishes[0]) };
    assert_eq!(answer.task, task);
    assert!(!answer.citations.is_empty());

    let trusted = answer.verify(&shell.registry, &shell.scope).expect("the shell accepts the citations");
    assert!(trusted.citations().iter().all(|c| BLOCK_0.contains(c.quote()) || BLOCK_1.contains(c.quote())));

    let events = kinds(&frames);
    assert_eq!(events.first().map(String::as_str), Some("stage"));
    assert!(events.iter().any(|e| e == "routed") && events.iter().any(|e| e == "delta"));

    let stderr = shell.shutdown();
    assert!(stderr.contains("event=task.finish") && stderr.contains("provider=extractive"));
    assert!(!stderr.contains("Returns are accepted") && !stderr.contains("days do I have"), "audit must be content-free");
}

#[test]
fn cancel_over_stdio_acks_and_the_task_still_finishes_once() {
    let (_server, config) = stalled_http_config();
    let mut shell = Shell::start(Some(config), "cancel");
    let (task, params) = shell.new_task(QUESTION, TaskOptions::default());
    let id = shell.request(Method::CompleteModel, &params);
    shell.response_to(id);

    // Serve the snapshot, then cancel while the model is stalled.
    let snapshot_request = shell.next_request();
    shell.answer_snapshot(&snapshot_request);
    std::thread::sleep(Duration::from_millis(500));
    let cancel = serde_json::to_value(CancellationRequest { task, reason: CancelReason::UserStopped }).unwrap();
    let id = shell.request(Method::CancelTask, &cancel);
    let (payload, mut seen) = shell.response_to(id);
    let ResponsePayload::Ok(result) = payload else { panic!("{payload:?}") };
    let result: CancellationResult = serde_json::from_value(result).unwrap();
    assert_eq!((result.task, result.cancelled_at, result.effect_unverified), (task, CancelledAt::BeforeFirstAction, false));

    let (rest, finishes) = shell.run_until_finish(|_, r| panic!("unexpected request {r:?}"));
    seen.extend(rest);
    let finishes: Vec<TaskOutcome> = seen
        .iter()
        .filter_map(|f| match f {
            Frame::Notification(n) if n.method == Method::FinishTask => Some(serde_json::from_value(n.params.clone()).unwrap()),
            _ => None,
        })
        .chain(finishes.into_iter().take(0))
        .collect();
    assert_eq!(finishes.len(), 1, "exactly one task.finish");
    let TaskOutcome::Failed(failed) = &finishes[0] else { panic!("{:?}", finishes[0]) };
    assert_eq!(failed.failure.kind, FailureKind::Cancelled);

    // Cancelling a task that already finished says so.
    let id = shell.request(Method::CancelTask, &cancel);
    let ResponsePayload::Ok(result) = shell.response_to(id).0 else { panic!() };
    let result: CancellationResult = serde_json::from_value(result).unwrap();
    assert_eq!(result.cancelled_at, CancelledAt::AlreadyFinished);
    shell.shutdown();
}

#[test]
fn navigation_event_makes_the_running_task_stale() {
    let (_server, config) = stalled_http_config();
    let mut shell = Shell::start(Some(config), "stale");
    let (_task, params) = shell.new_task(QUESTION, TaskOptions::default());
    let id = shell.request(Method::CompleteModel, &params);
    shell.response_to(id);
    let snapshot_request = shell.next_request();
    shell.answer_snapshot(&snapshot_request);
    std::thread::sleep(Duration::from_millis(300));

    let event = serde_json::to_value(BrowserEvent::NavigationCommitted {
        tab_id: shell.tab.0,
        document: DocumentToken(shell.document.document_token().0 + 1000),
        navigation_sequence: 2,
        replaced: Some(shell.document.document_token()),
    })
    .unwrap();
    shell.send(&Frame::notification(Method::BrowserEvent, &event));

    let (frames, finishes) = shell.run_until_finish(|_, r| panic!("unexpected request {r:?}"));
    assert_eq!(finishes.len(), 1, "exactly one task.finish");
    let TaskOutcome::Failed(failed) = &finishes[0] else { panic!("{:?}", finishes[0]) };
    assert_eq!((failed.failure.kind, failed.failure.code.as_str()), (FailureKind::Stale, "task.document_replaced"));
    assert!(frames.iter().all(|f| !matches!(f, Frame::Request(r) if r.method == Method::DispatchAction)));
    shell.shutdown();
}

#[test]
fn user_stopped_event_cancels_and_closing_stdin_ends_running_tasks() {
    let (_server, config) = stalled_http_config();
    let mut shell = Shell::start(Some(config), "stopped");
    let (task, params) = shell.new_task(QUESTION, TaskOptions::default());
    let id = shell.request(Method::CompleteModel, &params);
    shell.response_to(id);
    let snapshot_request = shell.next_request();
    shell.answer_snapshot(&snapshot_request);
    std::thread::sleep(Duration::from_millis(300));
    let event = serde_json::to_value(BrowserEvent::UserStopped { task }).unwrap();
    shell.send(&Frame::notification(Method::BrowserEvent, &event));
    let (_, finishes) = shell.run_until_finish(|_, r| panic!("unexpected request {r:?}"));
    assert_eq!(finishes.len(), 1);
    let TaskOutcome::Failed(failed) = &finishes[0] else { panic!("{:?}", finishes[0]) };
    assert_eq!(failed.failure.kind, FailureKind::Cancelled);

    // A second task is abandoned by closing stdin: it still finishes once and the process exits cleanly.
    let (_, params) = shell.new_task(QUESTION, TaskOptions::default());
    let id = shell.request(Method::CompleteModel, &params);
    shell.response_to(id);
    let snapshot_request = shell.next_request();
    shell.answer_snapshot(&snapshot_request);
    std::thread::sleep(Duration::from_millis(200));
    drop(shell.stdin.take());
    let mut finishes = 0;
    while let Some(frame) = shell.recv(Duration::from_secs(10)) {
        if matches!(&frame, Frame::Notification(n) if n.method == Method::FinishTask) {
            finishes += 1;
        }
    }
    assert_eq!(finishes, 1);
    assert!(shell.child.wait().unwrap().success());
}

#[test]
fn duplicate_and_malformed_requests_are_refused() {
    let (_server, config) = stalled_http_config();
    let mut shell = Shell::start(Some(config), "dup");
    let (_task, params) = shell.new_task(QUESTION, TaskOptions::default());
    let id = shell.request(Method::CompleteModel, &params);
    assert!(matches!(shell.response_to(id).0, ResponsePayload::Ok(_)));
    let id = shell.request(Method::CompleteModel, &params);
    let (payload, _) = shell.response_to(id);
    assert!(matches!(payload, ResponsePayload::Failure(Failure { kind: FailureKind::Unsupported, .. })), "{payload:?}");
    let id = shell.request(Method::CompleteModel, &json!({"nonsense": true}));
    assert!(matches!(shell.response_to(id).0, ResponsePayload::Failure(_)));
    shell.shutdown();
}
