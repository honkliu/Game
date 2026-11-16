//! OpenAI-compatible `/chat/completions` over ureq (rustls), streamed as SSE.
//!
//! Hard rules this file exists to keep:
//! - The API key lives in a private field, is never formatted, logged or put in an
//!   error detail, and redirects are disabled so it cannot follow one elsewhere.
//! - `reasoning_content` is observed (to know the model was thinking) and then
//!   dropped; it is never streamed, stored or returned.
//! - Page content is only ever in the request body; nothing here logs a body,
//!   a response, or a prompt.
//! - Cancellation does not depend on the socket: the blocking read runs on its
//!   own thread and the caller polls the stop flag, so a stalled server cannot
//!   hold a task past its deadline.

use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use ai_platform_contract::failure::Failure;
use serde_json::{json, Value};

use super::{ChatReply, ChatRequest, CompletionRequest, ModelProvider, ProviderReply, ToolCall};
use crate::control::TaskControl;

const POLL: Duration = Duration::from_millis(40);
/// A reasoning model can think for minutes before its first answer token; the
/// task deadline, not this, is the real bound.
const READ_TIMEOUT: Duration = Duration::from_secs(600);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LINE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FALLBACK_BODY: usize = 4 * 1024 * 1024;

pub struct OpenAiCompatibleProvider {
    id: String,
    url: String,
    key: Option<String>,
    agent: ureq::Agent,
}

enum Msg {
    Line(String),
    Failed(Failure),
    Eof,
}

impl OpenAiCompatibleProvider {
    pub fn new(id: &str, base_url: &str, key: Option<String>) -> Result<OpenAiCompatibleProvider, &'static str> {
        let base = base_url.trim();
        if !(base.starts_with("http://") || base.starts_with("https://")) || base.len() < 9 {
            return Err("bad_base_url");
        }
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .user_agent("ai-utility")
            .build();
        Ok(OpenAiCompatibleProvider {
            id: id.to_string(),
            url: format!("{}/chat/completions", base.trim_end_matches('/')),
            key: key.filter(|k| !k.is_empty()),
            agent,
        })
    }
}

fn status_failure(code: u16) -> Failure {
    let failure = match code {
        401 | 403 => Failure::blocked("provider.unauthorized"),
        408 | 504 => Failure::timed_out("provider.timeout"),
        429 => Failure::unavailable("provider.rate_limited"),
        500..=599 => Failure::unavailable("provider.server_error"),
        _ => Failure::unsupported("provider.request_rejected"),
    };
    failure.with_detail(format!("http status {code}"))
}

fn read_stream(agent: ureq::Agent, url: String, key: Option<String>, body: String, tx: mpsc::Sender<Msg>) {
    let mut request = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream");
    if let Some(key) = &key {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }
    let response = match request.send_string(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) => {
            let _ = tx.send(Msg::Failed(status_failure(code)));
            return;
        }
        Err(ureq::Error::Transport(transport)) => {
            let failure = Failure::unavailable("provider.unreachable").with_detail(format!("{:?}", transport.kind()));
            let _ = tx.send(Msg::Failed(failure));
            return;
        }
    };
    let mut reader = BufReader::new(response.into_reader());
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match (&mut reader).take(MAX_LINE_BYTES).read_until(b'\n', &mut buf) {
            Ok(0) => {
                let _ = tx.send(Msg::Eof);
                return;
            }
            Ok(_) => {
                if buf.len() as u64 >= MAX_LINE_BYTES && !buf.ends_with(b"\n") {
                    let _ = tx.send(Msg::Failed(Failure::output_limited("provider.line_too_long")));
                    return;
                }
                let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                if tx.send(Msg::Line(line)).is_err() {
                    return;
                }
            }
            Err(_) => {
                let _ = tx.send(Msg::Failed(Failure::unavailable("provider.stream_broken")));
                return;
            }
        }
    }
}

#[derive(Default)]
struct StreamState {
    calls: Vec<ToolCall>,
    text: String,
    finish_reason: Option<String>,
    reasoning_tokens: Option<u32>,
    saw_data: bool,
    other: String,
}

fn count(value: Option<&Value>) -> Option<u32> {
    value.and_then(Value::as_u64).map(|n| n.min(u32::MAX as u64) as u32)
}

fn apply_usage(state: &mut StreamState, usage: &Value) {
    // Top-level on the release-zero server, nested on OpenAI proper.
    state.reasoning_tokens = count(usage.get("reasoning_tokens"))
        .or_else(|| count(usage.pointer("/completion_tokens_details/reasoning_tokens")))
        .or(state.reasoning_tokens);
}

impl StreamState {
    fn apply_chunk(&mut self, chunk: &Value, on_delta: &mut dyn FnMut(&str)) -> Result<(), Failure> {
        self.saw_data = true;
        if chunk.get("error").is_some() {
            return Err(Failure::unavailable("provider.stream_error"));
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            apply_usage(self, usage);
        }
        let Some(choice) = chunk.pointer("/choices/0") else { return Ok(()) };
        if let Some(piece) = choice.pointer("/delta/content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            self.text.push_str(piece);
            on_delta(piece);
        }
        // `delta.reasoning_content` / `delta.reasoning` are deliberately not read.
        if let Some(calls) = choice.pointer("/delta/tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0).min(63) as usize;
                while self.calls.len() <= index {
                    self.calls.push(ToolCall::default());
                }
                let slot = &mut self.calls[index];
                if let Some(id) = call.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    slot.id = id.to_string();
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    slot.name.push_str(name);
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    if slot.arguments.len() < MAX_FALLBACK_BODY {
                        slot.arguments.push_str(args);
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        Ok(())
    }

    fn apply_line(&mut self, line: &str, on_delta: &mut dyn FnMut(&str)) -> Result<(), Failure> {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            return Ok(());
        }
        if let Some(payload) = line.strip_prefix("data:") {
            let payload = payload.trim();
            if payload == "[DONE]" {
                return Ok(());
            }
            return match serde_json::from_str::<Value>(payload) {
                Ok(chunk) => self.apply_chunk(&chunk, on_delta),
                Err(_) => Err(Failure::unsupported("provider.reply_unreadable")),
            };
        }
        if self.other.len() < MAX_FALLBACK_BODY {
            self.other.push_str(line);
        }
        Ok(())
    }

    /// A server that ignored `stream: true` answers with one JSON body.
    fn finish_non_streaming(&mut self, on_delta: &mut dyn FnMut(&str)) -> Result<(), Failure> {
        if self.saw_data || self.other.is_empty() {
            return Ok(());
        }
        let body: Value = serde_json::from_str(&self.other).map_err(|_| Failure::unsupported("provider.reply_unreadable"))?;
        if body.get("error").is_some() {
            return Err(Failure::unavailable("provider.stream_error"));
        }
        if let Some(usage) = body.get("usage").filter(|u| u.is_object()) {
            apply_usage(self, usage);
        }
        if let Some(content) = body.pointer("/choices/0/message/content").and_then(Value::as_str) {
            if !content.is_empty() {
                self.text.push_str(content);
                on_delta(content);
            }
        }
        if let Some(calls) = body.pointer("/choices/0/message/tool_calls").and_then(Value::as_array) {
            for call in calls {
                self.calls.push(ToolCall {
                    id: call.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                    name: call.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string(),
                    arguments: call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("").to_string(),
                });
            }
        }
        self.finish_reason = body.pointer("/choices/0/finish_reason").and_then(Value::as_str).map(str::to_string);
        Ok(())
    }
}

impl OpenAiCompatibleProvider {
    fn stream(&self, body: String, control: &TaskControl, on_delta: &mut dyn FnMut(&str)) -> Result<StreamState, Failure> {
        if let Some(failure) = control.failure() {
            return Err(failure);
        }
        let (tx, rx) = mpsc::channel();
        let (agent, url, key) = (self.agent.clone(), self.url.clone(), self.key.clone());
        thread::spawn(move || read_stream(agent, url, key, body, tx));

        let mut state = StreamState::default();
        loop {
            match rx.recv_timeout(POLL) {
                Ok(Msg::Line(line)) => {
                    state.apply_line(&line, on_delta)?;
                    if let Some(failure) = control.failure() {
                        return Err(failure);
                    }
                }
                Ok(Msg::Failed(failure)) => return Err(control.failure().unwrap_or(failure)),
                Ok(Msg::Eof) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(failure) = control.failure() {
                        return Err(failure);
                    }
                }
            }
        }
        state.finish_non_streaming(on_delta)?;
        if let Some(failure) = control.failure() {
            return Err(failure);
        }
        Ok(state)
    }
}

impl ModelProvider for OpenAiCompatibleProvider {
    fn provider_id(&self) -> &str {
        &self.id
    }

    fn chat(
        &self,
        request: &ChatRequest,
        control: &TaskControl,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatReply, Failure> {
        let mut body = json!({
            "model": request.model_id,
            "messages": request.messages,
            "stream": true,
            "stream_options": {"include_usage": true},
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(request.tools.clone());
            body["tool_choice"] = json!("auto");
        }
        let state = self.stream(body.to_string(), control, on_delta)?;
        let calls = state.calls.into_iter().filter(|c| !c.name.is_empty()).collect();
        Ok(ChatReply { text: state.text, tool_calls: calls, finish_reason: state.finish_reason })
    }

    fn complete(
        &self,
        request: &CompletionRequest,
        control: &TaskControl,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ProviderReply, Failure> {
        let messages: Vec<Value> = request.messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect();
        let body = json!({
            "model": request.model_id,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
            // Same page and question should give the same citations.
            "temperature": 0,
            "max_tokens": request.max_tokens,
        })
        .to_string();
        let state = self.stream(body, control, on_delta)?;
        Ok(ProviderReply {
            text: state.text,
            finish_reason: state.finish_reason,
            reasoning_tokens: state.reasoning_tokens,
            completion_token_budget: request.max_tokens,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_base_urls() {
        assert!(OpenAiCompatibleProvider::new("p", "ftp://x/v1", None).is_err());
        assert!(OpenAiCompatibleProvider::new("p", "localhost:8000", None).is_err());
        assert!(OpenAiCompatibleProvider::new("p", "http://127.0.0.1:8000/v1/", None).is_ok());
    }

    #[test]
    fn reasoning_text_is_never_collected() {
        let mut state = StreamState::default();
        let mut deltas = Vec::new();
        let mut sink = |s: &str| deltas.push(s.to_string());
        state
            .apply_line(r#"data: {"choices":[{"delta":{"reasoning_content":"SECRET"}}]}"#, &mut sink)
            .unwrap();
        state
            .apply_line(r#"data: {"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}],"usage":{"reasoning_tokens":7}}"#, &mut sink)
            .unwrap();
        assert_eq!(state.text, "hi");
        assert_eq!(deltas, ["hi"]);
        assert_eq!(state.reasoning_tokens, Some(7));
        assert_eq!(state.finish_reason.as_deref(), Some("stop"));
    }
}
