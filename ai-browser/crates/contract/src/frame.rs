//! Frame envelope and codec for the shell ↔ AI-utility-process boundary
//! (backlog FND-02, FND-03's boundary rule).
//!
//! One JSON object per line (NDJSON) over the child's stdin/stdout. Decoding is
//! deliberately two-phase, because the order of the checks *is* the security
//! property:
//!
//! 1. size bound,
//! 2. syntax (`serde_json::Value` — no schema interpretation yet),
//! 3. version gate ([`crate::version`]),
//! 4. **side gate**: does the peer that sent this get to name this method?,
//! 5. only then: typed interpretation of `params`.
//!
//! Mojo used to give rule 4 for free: a process that was never handed a
//! `PendingReceiver<ActionDispatcher>` could not call it. Over a byte pipe there is
//! no such ambient fact, so it becomes an explicit gate at the single place every
//! inbound frame passes through. `tests/frame_contract.rs` checks that a rejected
//! frame is rejected *before* interpretation, by sending a payload that would fail
//! typed parsing under a wrong major version and asserting the reported code is the
//! version code, not a payload code.
//!
//! ## Why a closed method list
//!
//! [`Method`] is an enum, so "the utility may not issue `model.complete`" is a value
//! in one table (`Method::issuer`) rather than a check scattered across handlers.
//! Adding a method forces that table to be filled in; adding a *string* method
//! would not.

use serde::{Deserialize, Serialize};

use crate::failure::Failure;
use crate::version::{gate_schema_version, SchemaVersion, VersionDecision};

/// Hard bound on one inbound frame. A line-delimited protocol with no cap lets a
/// wedged peer grow the reader's buffer without limit.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Which side of the pipe a frame arrived from, as established when the pipe was
/// brokered — not as claimed by the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    /// The Rust browser shell: owns identity, tabs, workspaces, policy, dispatch.
    Shell,
    /// The AI utility process: owns providers, prompts, routing, planning.
    Utility,
}

impl Peer {
    pub fn other(self) -> Peer {
        match self {
            Peer::Shell => Peer::Utility,
            Peer::Utility => Peer::Shell,
        }
    }
}

/// The complete set of callable methods, with the side allowed to issue each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    /// utility → shell. The utility asks for context it cannot observe itself.
    #[serde(rename = "context.get_snapshot")]
    GetSnapshot,
    /// utility → shell. The *only* way anything reaches a page.
    #[serde(rename = "action.dispatch")]
    DispatchAction,
    /// utility → shell, notification.
    #[serde(rename = "task.progress")]
    PublishProgress,
    /// utility → shell, notification.
    #[serde(rename = "task.finish")]
    FinishTask,
    /// shell → utility. Model work is the utility's job; the shell never calls a
    /// provider itself, and the utility must not be able to conjure a second one
    /// by asking itself.
    #[serde(rename = "model.complete")]
    CompleteModel,
    /// shell → utility. Only the shell's stop button exists.
    #[serde(rename = "task.cancel")]
    CancelTask,
    /// shell → utility, request. Liveness probe for the supervisor.
    #[serde(rename = "utility.ping")]
    Ping,
    /// shell → utility, request. The models the utility can route to right now.
    #[serde(rename = "model.list")]
    ListModels,
    /// shell → utility, notification. The browser telling the AI what just happened
    /// (tab activated, navigation committed, selection changed, …). This is the
    /// return half of the two-way channel: without it the AI only knows what it
    /// last asked about.
    #[serde(rename = "browser.event")]
    BrowserEvent,
    /// shell → utility, request. Starts an agent run (a tool-using conversation turn);
    /// the reply only acknowledges it, the outcome arrives as `agent.event`s.
    #[serde(rename = "agent.run")]
    AgentRun,
    /// shell → utility, request. Cancel a run, answer an approval, or manage skills,
    /// MCP servers and saved conversations (`{ "op": ... }`).
    #[serde(rename = "agent.control")]
    AgentControl,
    /// utility → shell, notification. Progress of an agent run: deltas, tool calls, approvals.
    #[serde(rename = "agent.event")]
    AgentEvent,
    /// utility → shell, request. One browser-side tool call of an agent run.
    #[serde(rename = "agent.tool")]
    AgentTool,
}

impl Method {
    /// The side allowed to *issue* this method. The reply always flows back the
    /// other way.
    pub fn issuer(self) -> Peer {
        match self {
            Method::GetSnapshot | Method::DispatchAction | Method::PublishProgress
            | Method::FinishTask | Method::AgentEvent | Method::AgentTool => Peer::Utility,
            Method::CompleteModel | Method::CancelTask | Method::Ping | Method::ListModels
            | Method::BrowserEvent | Method::AgentRun | Method::AgentControl => Peer::Shell,
        }
    }

    /// Whether this method is fire-and-forget. A notification has no `id` and must
    /// never be answered, so a peer cannot stall a reply by replying to it.
    pub fn is_notification(self) -> bool {
        matches!(self, Method::PublishProgress | Method::FinishTask | Method::BrowserEvent | Method::AgentEvent)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Method::GetSnapshot => "context.get_snapshot",
            Method::DispatchAction => "action.dispatch",
            Method::PublishProgress => "task.progress",
            Method::FinishTask => "task.finish",
            Method::CompleteModel => "model.complete",
            Method::CancelTask => "task.cancel",
            Method::Ping => "utility.ping",
            Method::ListModels => "model.list",
            Method::BrowserEvent => "browser.event",
            Method::AgentRun => "agent.run",
            Method::AgentControl => "agent.control",
            Method::AgentEvent => "agent.event",
            Method::AgentTool => "agent.tool",
        }
    }
}

/// Response body. Externally tagged, so on the wire it is either
/// `{"ok": ...}` or `{"failure": {...}}` — a response cannot be both, and cannot be
/// neither.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsePayload {
    Ok(serde_json::Value),
    Failure(Failure),
}

impl ResponsePayload {
    pub fn ok<T: Serialize>(value: &T) -> Result<ResponsePayload, Failure> {
        Ok(ResponsePayload::Ok(serde_json::to_value(value).map_err(encode_error)?))
    }

    pub fn failure(failure: Failure) -> ResponsePayload {
        ResponsePayload::Failure(failure)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub schema_version: SchemaVersion,
    pub id: u64,
    pub method: Method,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub schema_version: SchemaVersion,
    /// Correlates to a [`Request::id`] issued by this peer. An id the receiver
    /// never issued is a protocol violation the supervisor rejects; it is never
    /// routed to a waiting caller.
    pub id: u64,
    pub payload: ResponsePayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub schema_version: SchemaVersion,
    pub method: Method,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frame {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl Frame {
    pub fn request(id: u64, method: Method, params: &serde_json::Value) -> Frame {
        Frame::Request(Request {
            schema_version: SchemaVersion::current(),
            id,
            method,
            params: params.clone(),
        })
    }

    pub fn notification(method: Method, params: &serde_json::Value) -> Frame {
        Frame::Notification(Notification {
            schema_version: SchemaVersion::current(),
            method,
            params: params.clone(),
        })
    }

    pub fn ok(id: u64, payload: ResponsePayload) -> Frame {
        Frame::Response(Response {
            schema_version: SchemaVersion::current(),
            id,
            payload,
        })
    }

    pub fn failure(id: u64, failure: Failure) -> Frame {
        Frame::Response(Response {
            schema_version: SchemaVersion::current(),
            id,
            payload: ResponsePayload::Failure(failure),
        })
    }
}

fn encode_error(error: serde_json::Error) -> Failure {
    Failure::unsupported("transport.encode_failed")
        .with_detail(error.to_string())
}

/// Serialises one frame to a single line. `serde_json` escapes embedded newlines,
/// so a frame cannot forge a second line.
pub fn encode(frame: &Frame) -> Result<String, Failure> {
    let line = serde_json::to_string(frame).map_err(encode_error)?;
    debug_assert!(!line.contains('\n'), "serde_json must escape newlines in strings");
    Ok(line)
}

/// Decodes one line that arrived from `from`, applying the size, version, and side
/// gates before any typed interpretation.
pub fn decode(line: &str, from: Peer) -> Result<Frame, Failure> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(Failure::output_limited("transport.frame_too_large")
            .with_detail(format!("{} bytes exceeds the {} byte frame bound", line.len(), MAX_FRAME_BYTES)));
    }

    // Phase 1: syntax only. `Value` carries no field expectations, so nothing here
    // can misread a future shape as a present one.
    let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
        Failure::unsupported("transport.frame_malformed").with_detail(error.to_string())
    })?;

    // Phase 2: the version header itself.
    let schema_version: SchemaVersion = value
        .get("schema_version")
        .and_then(|raw| serde_json::from_value(raw.clone()).ok())
        .ok_or_else(|| {
            Failure::unsupported("transport.version_header_missing")
                .with_detail("a frame must carry {schema_version:{major,minor}}")
        })?;

    match gate_schema_version(&schema_version) {
        VersionDecision::Accept => {}
        VersionDecision::Reject(failure) => return Err(failure),
    }

    // Phase 3: the side gate, straight off the raw object so it also runs ahead of
    // typed parsing.
    let kind = value.get("kind").and_then(|raw| raw.as_str()).ok_or_else(|| {
        Failure::unsupported("transport.frame_malformed").with_detail("frame has no string \"kind\"")
    })?;
    if let Some(raw_method) = value.get("method").and_then(|raw| raw.as_str()) {
        let method: Method = serde_json::from_value(serde_json::Value::String(raw_method.to_string()))
            .map_err(|_| {
                Failure::unsupported("transport.method_unknown").with_detail(raw_method.to_string())
            })?;
        if method.issuer() != from {
            return Err(Failure::blocked("transport.method_from_wrong_side").with_detail(format!(
                "{} may only be issued by {:?} on this pipe",
                method.as_str(),
                method.issuer()
            )));
        }
        let notification_issued = kind == "notification";
        if notification_issued != method.is_notification() {
            return Err(Failure::blocked("transport.frame_kind_mismatch").with_detail(format!(
                "{} is {}",
                method.as_str(),
                if method.is_notification() { "notification-only" } else { "request-only" }
            )));
        }
    } else if kind == "response" {
        // A response names no method; its `id` is the only correlation, checked by
        // the supervisor against requests it actually issued.
    } else {
        return Err(Failure::unsupported("transport.frame_malformed")
            .with_detail(format!("{kind} frame carries no string \"method\"")));
    }

    // Phase 4: typed interpretation, reached only by a frame that passed the gates.
    serde_json::from_value(value).map_err(|error| {
        Failure::unsupported("transport.payload_invalid").with_detail(error.to_string())
    })
}
