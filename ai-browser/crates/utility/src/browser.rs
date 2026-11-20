//! The running shell's fixed MCP connection. It is not a user-configured server.
//! Each tool call has its own HTTP request; only initial session setup is locked.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::ResponsePayload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::control::{lock, TaskControl};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub url: String,
    pub token: String,
}

struct Inner {
    info: ConnectionInfo,
    agent: ureq::Agent,
    session: Mutex<Option<String>>,
    next_id: AtomicU64,
    active: AtomicUsize,
}

#[derive(Clone)]
pub struct Client(Arc<Inner>);

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) { self.0.store(true, Ordering::Release); }
}

impl Client {
    pub fn new(info: ConnectionInfo) -> Result<Self, Failure> {
        let url = url::Url::parse(&info.url).map_err(|_| Failure::unsupported("browser.invalid_connection"))?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(host)) => host == "localhost",
            None => false,
        };
        if !loopback || url.scheme() != "http" || !url.username().is_empty() || url.password().is_some()
            || url.path() != "/mcp" || url.query().is_some() || url.fragment().is_some() || info.token.is_empty() {
            return Err(Failure::unsupported("browser.invalid_connection"));
        }
        Ok(Self(Arc::new(Inner {
            info,
            agent: ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(3))
                .timeout(Duration::from_secs(125)).build(),
            session: Mutex::new(None), next_id: AtomicU64::new(1), active: AtomicUsize::new(0),
        })))
    }

    pub fn connection(&self) -> &ConnectionInfo { &self.0.info }

    fn rpc(&self, method: &str, params: Value, session: Option<&str>, notification: bool) -> Result<(Value, Option<String>), Failure> {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let mut body = json!({"jsonrpc":"2.0","method":method,"params":params});
        if !notification { body["id"] = json!(id); }
        let mut request = self.0.agent.post(&self.0.info.url)
            .timeout(Duration::from_secs(if method == "tools/call" { 125 } else { 5 }))
            .set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream")
            .set("Authorization", &format!("Bearer {}", self.0.info.token));
        if let Some(session) = session { request = request.set("Mcp-Session-Id", session); }
        let response = request.send_string(&body.to_string()).map_err(|_| Failure::unavailable("browser.transport_failed"))?;
        let session = response.header("mcp-session-id").map(str::to_string);
        if notification { return Ok((Value::Null, session)); }
        let mut bytes = Vec::new();
        response.into_reader().take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)
            .map_err(|_| Failure::unavailable("browser.read_failed"))?;
        if bytes.len() > 2 * 1024 * 1024 { return Err(Failure::unavailable("browser.response_too_large")); }
        let reply: Value = serde_json::from_slice(&bytes).map_err(|_| Failure::unavailable("browser.invalid_response"))?;
        if reply["id"].as_u64() != Some(id) || reply["jsonrpc"] != "2.0" {
            return Err(Failure::unavailable("browser.invalid_response"));
        }
        if reply.get("error").is_some() { return Err(Failure::unsupported("browser.request_rejected")); }
        Ok((reply["result"].clone(), session))
    }

    fn session(&self) -> Result<String, Failure> {
        let mut session = lock(&self.0.session);
        if let Some(session) = session.as_ref() { return Ok(session.clone()); }
        let (initialized, issued) = self.rpc("initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"ai-browser-runtime","version":env!("CARGO_PKG_VERSION")}}), None, false)?;
        if initialized["protocolVersion"] != "2025-03-26" { return Err(Failure::unsupported("browser.protocol_version")); }
        let issued = issued.ok_or_else(|| Failure::unavailable("browser.missing_session"))?;
        self.rpc("notifications/initialized", json!({}), Some(&issued), true)?;
        let (catalog, _) = self.rpc("tools/list", json!({}), Some(&issued), false)?;
        if !catalog["tools"].is_array() { return Err(Failure::unavailable("browser.invalid_catalog")); }
        *session = Some(issued.clone());
        Ok(issued)
    }

    /// Never reconnect or replay a call: transport failure leaves its effect unknown.
    pub fn call(&self, name: &str, args: &Value, control: &TaskControl) -> Result<Value, Failure> {
        match self.call_response(name, args, control)? {
            ResponsePayload::Ok(value) => Ok(value),
            ResponsePayload::Failure(failure) => Err(failure),
        }
    }

    /// Keep a native refusal distinct from a lost reply with an unknown effect.
    pub fn call_response(&self, name: &str, args: &Value, control: &TaskControl) -> Result<ResponsePayload, Failure> {
        if let Some(failure) = control.failure() { return Err(failure); }
        ai_platform_contract::browser::validate(name, args).map_err(|_| Failure::unsupported("browser.invalid_arguments"))?;
        if self.0.active.fetch_add(1, Ordering::AcqRel) >= 16 {
            self.0.active.fetch_sub(1, Ordering::AcqRel);
            return Err(Failure::unavailable("browser.busy"));
        }
        let (client, name, args) = (self.clone(), name.to_string(), args.clone());
        let (send, receive) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_return = CancelOnDrop(Arc::clone(&cancelled));
        let spawn = std::thread::Builder::new().name("browser-mcp-call".into()).spawn(move || {
            let result = (|| {
                let session = client.session()?;
                if cancelled.load(Ordering::Acquire) { return Err(Failure::unavailable("browser.cancelled")); }
                let (result, _) = client.rpc("tools/call", json!({"name":name,"arguments":args}), Some(&session), false)?;
                if let Some(failure) = result.pointer("/structuredContent/failure") {
                    let failure = serde_json::from_value(failure.clone()).map_err(|_| Failure::unavailable("browser.invalid_failure"))?;
                    return Ok(ResponsePayload::Failure(failure));
                }
                if result["isError"] == true {
                    let text = result.pointer("/content/0/text").and_then(Value::as_str).unwrap_or("Browser operation failed");
                    return Ok(ResponsePayload::Failure(serde_json::from_str::<Failure>(text).unwrap_or_else(|_| Failure::unsupported("browser.operation_failed").with_detail(text.to_string()))));
                }
                if let Some(value) = result.get("structuredContent") { return Ok(ResponsePayload::Ok(value.clone())); }
                let text = result.pointer("/content/0/text").and_then(Value::as_str).ok_or_else(|| Failure::unavailable("browser.invalid_result"))?;
                serde_json::from_str(text).map(ResponsePayload::Ok).map_err(|_| Failure::unavailable("browser.invalid_result"))
            })();
            client.0.active.fetch_sub(1, Ordering::AcqRel);
            let _ = send.send(result);
        });
        if spawn.is_err() {
            self.0.active.fetch_sub(1, Ordering::AcqRel);
            return Err(Failure::unavailable("browser.busy"));
        }
        let deadline = Instant::now() + Duration::from_secs(130);
        loop {
            if let Some(failure) = control.failure() { return Err(failure); }
            if Instant::now() >= deadline { return Err(Failure::unavailable("browser.timeout")); }
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Failure::unavailable("browser.disconnected")),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
