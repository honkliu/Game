#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! Test doubles shared by the integration tests: a scripted SSE server on
//! 127.0.0.1 (std::net only) and an in-memory shell.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::{Method, ResponsePayload};
use ai_platform_contract::identity::TaskId;
use ai_platform_contract::model::{PageBlock, TaskIntent};
use ai_utility::control::TaskControl;
use ai_utility::prompt::{self, PromptInput};
use ai_utility::provider::{CompletionRequest, Mode, PromptBlock, DEFAULT_COMPLETION_TOKEN_BUDGET};
use ai_utility::shell::ShellLink;

pub const BLOCK_0: &str = "Returns are accepted within 30 days of delivery. Refunds go to the original payment method.";
pub const BLOCK_1: &str = "Shipping is free on orders over $50. Express shipping takes two days.";
pub const QUESTION: &str = "How many days do I have for returns?";

pub fn page_blocks() -> Vec<PageBlock> {
    vec![
        PageBlock { block_id: "b0".into(), text: BLOCK_0.into() },
        PageBlock { block_id: "b1".into(), text: BLOCK_1.into() },
    ]
}

pub fn completion_request(question: &str) -> CompletionRequest {
    let blocks: Vec<PromptBlock> =
        page_blocks().into_iter().map(|b| PromptBlock { id: b.block_id, text: b.text }).collect();
    let messages = prompt::build(&PromptInput {
        task: TaskId(1),
        history: &[],
        mode: Mode::Answer,
        intent: TaskIntent::Ask,
        question,
        selection: None,
        target_language: None,
        blocks: &blocks,
        links: &[],
        steps: &[],
        allowed_actions: &[],
        steps_left: 0,
    });
    CompletionRequest {
        task: TaskId(1),
        history: Vec::new(),
        model_id: "fake-model".into(),
        mode: Mode::Answer,
        intent: TaskIntent::Ask,
        question: question.into(),
        selection: None,
        target_language: None,
        blocks,
        links: Vec::new(),
        steps: Vec::new(),
        messages,
        max_tokens: DEFAULT_COMPLETION_TOKEN_BUDGET,
    }
}

#[derive(Clone)]
pub enum Step {
    /// One SSE `data:` line.
    Data(String),
    Sleep(u64),
    /// Raw bytes written as-is.
    Raw(String),
}

#[derive(Clone)]
pub struct Script {
    pub status: u16,
    pub steps: Vec<Step>,
}

pub fn content_chunk(text: &str) -> Step {
    Step::Data(serde_json::json!({"choices": [{"delta": {"content": text}}]}).to_string())
}

pub fn reasoning_chunk(text: &str) -> Step {
    Step::Data(serde_json::json!({"choices": [{"delta": {"reasoning_content": text}}]}).to_string())
}

pub fn finish_chunk(reason: &str, reasoning_tokens: Option<u32>) -> Step {
    let mut value = serde_json::json!({"choices": [{"delta": {}, "finish_reason": reason}]});
    if let Some(tokens) = reasoning_tokens {
        value["usage"] = serde_json::json!({"reasoning_tokens": tokens});
    }
    Step::Data(value.to_string())
}

/// A script that streams `text` in small pieces, after some reasoning chatter.
pub fn script_for(text: &str) -> Script {
    let mut steps = vec![reasoning_chunk("thinking about SECRET-REASONING")];
    let mut rest = text;
    while !rest.is_empty() {
        let mut cut = rest.len().min(17);
        while !rest.is_char_boundary(cut) {
            cut += 1;
        }
        steps.push(content_chunk(&rest[..cut]));
        rest = &rest[cut..];
    }
    steps.push(finish_chunk("stop", Some(12)));
    steps.push(Step::Raw("data: [DONE]\n\n".into()));
    Script { status: 200, steps }
}

/// A well-formed streamed answer that quotes `BLOCK_0` verbatim.
pub fn good_answer_script() -> Script {
    script_for("ANSWER: Returns are accepted within 30 days.\nQUOTE b0: Returns are accepted within 30 days of delivery.")
}

#[derive(Debug, Clone)]
pub struct Captured {
    pub authorization: Option<String>,
    pub path: String,
    pub body: serde_json::Value,
}

pub struct FakeServer {
    pub base_url: String,
    pub captured: Arc<Mutex<Vec<Captured>>>,
}

fn handle(mut stream: TcpStream, script: Script, captured: Arc<Mutex<Vec<Captured>>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
    let (mut length, mut authorization) = (0usize, None);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end().to_string();
        if line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            authorization = Some(line["authorization:".len()..].trim().to_string());
        }
    }
    let mut body = vec![0u8; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    captured.lock().unwrap().push(Captured {
        authorization,
        path,
        body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    });
    let reason = if script.status == 200 { "OK" } else { "ERR" };
    let content_type = if script.status == 200 { "text/event-stream" } else { "application/json" };
    let head = format!("HTTP/1.1 {} {reason}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n", script.status);
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    if script.status != 200 {
        let _ = stream.write_all(br#"{"error":{"message":"SECRET-BODY"}}"#);
        return;
    }
    for step in script.steps {
        let ok = match step {
            Step::Data(data) => stream.write_all(format!("data: {data}\n\n").as_bytes()).is_ok(),
            Step::Raw(raw) => stream.write_all(raw.as_bytes()).is_ok(),
            Step::Sleep(ms) => {
                thread::sleep(Duration::from_millis(ms));
                true
            }
        };
        if !ok || stream.flush().is_err() {
            return;
        }
    }
}

impl FakeServer {
    /// Serves `script` to every connection until the process exits.
    pub fn start(script: Script) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captured);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (script, sink) = (script.clone(), Arc::clone(&sink));
                thread::spawn(move || handle(stream, script, sink));
            }
        });
        FakeServer { base_url: format!("http://127.0.0.1:{port}/v1"), captured }
    }

    pub fn requests(&self) -> Vec<Captured> {
        self.captured.lock().unwrap().clone()
    }
}

type CallHandler = dyn Fn(Method, &serde_json::Value, &TaskControl) -> Result<ResponsePayload, Failure> + Send + Sync;

/// An in-memory shell: records notifications and calls, answers calls with a closure.
pub struct FakeShell {
    pub notifications: Mutex<Vec<(Method, serde_json::Value)>>,
    pub calls: Mutex<Vec<(Method, serde_json::Value)>>,
    handler: Box<CallHandler>,
}

impl FakeShell {
    pub fn new(
        handler: impl Fn(Method, &serde_json::Value, &TaskControl) -> Result<ResponsePayload, Failure> + Send + Sync + 'static,
    ) -> FakeShell {
        FakeShell { notifications: Mutex::new(Vec::new()), calls: Mutex::new(Vec::new()), handler: Box::new(handler) }
    }

    pub fn finishes(&self) -> Vec<serde_json::Value> {
        self.notifications
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| *m == Method::FinishTask)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn events(&self) -> Vec<serde_json::Value> {
        self.notifications
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| *m == Method::PublishProgress)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn called(&self, method: Method) -> usize {
        self.calls.lock().unwrap().iter().filter(|(m, _)| *m == method).count()
    }

    pub fn calls_of(&self, method: Method) -> Vec<serde_json::Value> {
        self.calls.lock().unwrap().iter().filter(|(m, _)| *m == method).map(|(_, p)| p.clone()).collect()
    }
}

impl ShellLink for FakeShell {
    fn notify(&self, method: Method, params: &serde_json::Value) {
        self.notifications.lock().unwrap().push((method, params.clone()));
    }

    fn call(&self, method: Method, params: &serde_json::Value, control: &TaskControl) -> Result<ResponsePayload, Failure> {
        self.calls.lock().unwrap().push((method, params.clone()));
        if let Some(failure) = control.failure() {
            return Err(failure);
        }
        (self.handler)(method, params, control)
    }
}
