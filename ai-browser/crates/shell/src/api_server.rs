//! Profile-scoped, authenticated MCP access to the shared shell API catalog.
//! Native execution is supplied by the UI event loop, not by this transport.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ai_platform_contract::browser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const WORKERS: usize = 8;
const MAX_BODY: usize = 1024 * 1024;
const MAX_HEADERS: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiCaller {
    pub id: u64,
    pub builtin: bool,
}

#[derive(Debug)]
pub struct ApiRequest {
    pub caller: ApiCaller,
    pub name: String,
    pub args: Value,
    pub reply: mpsc::Sender<Result<Value, String>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub url: String,
    pub token: String,
}

struct Session {
    caller: ApiCaller,
    initialized: bool,
    touched: Instant,
}

struct State {
    stop: AtomicBool,
    builtin: String,
    external: String,
    host: String,
    sessions: Mutex<HashMap<String, Session>>,
    next_caller: AtomicU64,
    dispatch: Box<dyn Fn(ApiRequest) -> Result<(), String> + Send + Sync>,
}

pub struct ApiServer {
    state: Arc<State>,
    url: String,
    descriptor: PathBuf,
    workers: Vec<JoinHandle<()>>,
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "shell API entropy unavailable".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn matches_token(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() { return false; }
    actual.bytes().zip(expected.bytes()).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

impl ApiServer {
    pub fn start(data_dir: &Path, dispatch: impl Fn(ApiRequest) -> Result<(), String> + Send + Sync + 'static) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let host = listener.local_addr().map_err(|e| e.to_string())?.to_string();
        let url = format!("http://{host}/mcp");
        let state = Arc::new(State {
            stop: AtomicBool::new(false), builtin: random_token()?, external: random_token()?, host,
            sessions: Mutex::new(HashMap::new()), next_caller: AtomicU64::new(2), dispatch: Box::new(dispatch),
        });
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let descriptor = data_dir.join("shell-api.json");
        let info = ConnectionInfo { url: url.clone(), token: state.external.clone() };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let encoded = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
        options.open(&descriptor).and_then(|mut file| file.write_all(&encoded)).map_err(|e| e.to_string())?;
        let mut server = Self { state, url, descriptor, workers: Vec::new() };
        let listener = Arc::new(listener);
        for index in 0..WORKERS {
            let (listener, state) = (Arc::clone(&listener), Arc::clone(&server.state));
            let worker = std::thread::Builder::new().name(format!("shell-api-{index}")).spawn(move || {
                while !state.stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                            // Winsock inherits the listener's nonblocking mode.
                            if stream.set_nonblocking(false).is_ok() { serve(&mut stream, &state); }
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
                        Err(_) => break,
                    }
                }
            }).map_err(|e| e.to_string())?;
            server.workers.push(worker);
        }
        Ok(server)
    }

    pub fn builtin_connection(&self) -> ConnectionInfo {
        ConnectionInfo { url: self.url.clone(), token: self.state.builtin.clone() }
    }

    pub fn connection(&self) -> ConnectionInfo {
        ConnectionInfo { url: self.url.clone(), token: self.state.external.clone() }
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        for worker in self.workers.drain(..) { let _ = worker.join(); }
        // An older server must not remove a replacement instance's descriptor.
        let ours = std::fs::read(&self.descriptor).ok().and_then(|v| serde_json::from_slice::<ConnectionInfo>(&v).ok())
            .is_some_and(|info| info.url == self.url && matches_token(&info.token, &self.state.external));
        if ours { let _ = std::fs::remove_file(&self.descriptor); }
    }
}

struct HttpRequest {
    method: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_http(stream: &mut TcpStream) -> Result<HttpRequest, u16> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut bytes = Vec::new();
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&bytes).map_err(|_| 400u16)? {
            httparse::Status::Complete(offset) => {
                if offset > MAX_HEADERS { return Err(431); }
                if request.path != Some("/mcp") { return Err(404); }
                let method = request.method.ok_or(400u16)?.to_string();
                let mut parsed = HashMap::new();
                for header in request.headers.iter() {
                    let name = header.name.to_ascii_lowercase();
                    let value = std::str::from_utf8(header.value).map_err(|_| 400u16)?.trim().to_string();
                    if parsed.insert(name, value).is_some() { return Err(400); }
                }
                // This JSON transport deliberately does not accept transfer codings.
                if parsed.contains_key("transfer-encoding") { return Err(400); }
                let length = parsed.get("content-length").map(|v| v.parse::<usize>().map_err(|_| 400u16)).transpose()?.unwrap_or(0);
                if length > MAX_BODY { return Err(413); }
                let mut body = bytes[offset..].to_vec();
                while body.len() < length {
                    let remaining = deadline.checked_duration_since(Instant::now()).ok_or(408u16)?;
                    stream.set_read_timeout(Some(remaining)).map_err(|_| 400u16)?;
                    let mut chunk = [0u8; 8192];
                    let size = (length - body.len()).min(chunk.len());
                    let n = stream.read(&mut chunk[..size]).map_err(|_| 408u16)?;
                    if n == 0 { return Err(400); }
                    body.extend_from_slice(&chunk[..n]);
                }
                body.truncate(length);
                return Ok(HttpRequest { method, headers: parsed, body });
            }
            httparse::Status::Partial => {
                if bytes.len() >= MAX_HEADERS { return Err(431); }
                let remaining = deadline.checked_duration_since(Instant::now()).ok_or(408u16)?;
                stream.set_read_timeout(Some(remaining)).map_err(|_| 400u16)?;
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).map_err(|_| 408u16)?;
                if n == 0 { return Err(400); }
                bytes.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: Option<&Value>, session: Option<&str>) {
    let data = body.map(Value::to_string).unwrap_or_default();
    let session = session.map(|s| format!("Mcp-Session-Id: {s}\r\n")).unwrap_or_default();
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let _ = write!(stream, "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{session}\r\n{data}", data.len());
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn serve(stream: &mut TcpStream, state: &State) {
    let request = match read_http(stream) { Ok(r) => r, Err(status) => return respond(stream, status, None, None) };
    let host = request.headers.get("host").map(String::as_str).unwrap_or("");
    let localhost = state.host.replacen("127.0.0.1", "localhost", 1);
    if host != state.host && host != localhost { return respond(stream, 403, None, None); }
    if let Some(origin) = request.headers.get("origin") {
        if origin != &format!("http://{}", state.host) && origin != &format!("http://{localhost}") {
            return respond(stream, 403, None, None);
        }
    }
    let credential = request.headers.get("authorization").and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let builtin = matches_token(credential, &state.builtin);
    if !builtin && !matches_token(credential, &state.external) { return respond(stream, 401, None, None); }
    if !matches!(request.method.as_str(), "POST" | "DELETE") { return respond(stream, 405, None, None); }
    if request.method == "DELETE" {
        let removed = request.headers.get("mcp-session-id").is_some_and(|id| {
            let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if sessions.get(id).is_some_and(|s| s.caller.builtin == builtin) { sessions.remove(id); true } else { false }
        });
        return respond(stream, if removed { 200 } else { 404 }, None, None);
    }
    if request.headers.get("content-type").is_none_or(|v| v.split(';').next().unwrap_or("").trim() != "application/json") {
        return respond(stream, 415, None, None);
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(_) => return respond(stream, 400, Some(&rpc_error(&Value::Null, -32700, "Invalid JSON")), None),
    };
    // A batch shares the HTTP session; initialization is always its own request.
    let messages = body.as_array().cloned().unwrap_or_else(|| vec![body.clone()]);
    if messages.is_empty() || messages.len() > 32 { return respond(stream, 400, None, None); }
    let initializing = messages.len() == 1 && messages[0]["method"] == "initialize";
    let mut session_header = None;
    let caller = if initializing {
        if request.headers.contains_key("mcp-session-id") { return respond(stream, 400, None, None); }
        if messages[0].pointer("/params/protocolVersion").and_then(Value::as_str) != Some("2025-03-26") {
            return respond(stream, 200, Some(&rpc_error(&messages[0]["id"], -32602, "Unsupported protocol version")), None);
        }
        let session_id = match random_token() { Ok(id) => id, Err(_) => return respond(stream, 503, None, None) };
        let caller = ApiCaller { id: if builtin { 1 } else { state.next_caller.fetch_add(1, Ordering::Relaxed) }, builtin };
        let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, s| s.touched.elapsed() < Duration::from_secs(24 * 60 * 60));
        if sessions.len() >= 256 { return respond(stream, 503, None, None); }
        sessions.insert(session_id.clone(), Session { caller, initialized: false, touched: Instant::now() });
        session_header = Some(session_id);
        caller
    } else {
        let Some(id) = request.headers.get("mcp-session-id") else { return respond(stream, 400, None, None) };
        let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let Some(session) = sessions.get_mut(id).filter(|s| s.caller.builtin == builtin) else { return respond(stream, 404, None, None) };
        if !session.initialized && !messages.iter().all(|v| v["method"] == "notifications/initialized" || v["method"] == "ping") {
            return respond(stream, 400, None, None);
        }
        if messages.iter().any(|v| v["method"] == "notifications/initialized") { session.initialized = true; }
        session.touched = Instant::now();
        session.caller
    };
    let mut replies = Vec::new();
    for message in messages {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        if message["jsonrpc"] != "2.0" || !message["method"].is_string() {
            replies.push(rpc_error(&id, -32600, "Invalid request"));
            continue;
        }
        let method = message["method"].as_str().unwrap_or("");
        if message.get("id").is_none() {
            if matches!(method, "notifications/initialized" | "notifications/cancelled") { continue; }
            return respond(stream, 400, None, None);
        }
        let result = match method {
            "initialize" if initializing => Ok(json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"ai-browser-shell","version":env!("CARGO_PKG_VERSION")}})),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":browser::definitions().into_iter().map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.parameters,"annotations":{"readOnlyHint":tool.read_only}})).collect::<Vec<_>>()})),
            "tools/call" => call(state, caller, &message["params"]),
            _ => Err((-32601, "Unknown method".to_string())),
        };
        replies.push(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => rpc_error(&id, code, &message),
        });
    }
    if replies.is_empty() { return respond(stream, 202, None, session_header.as_deref()); }
    let reply = if body.is_array() { Value::Array(replies) } else { replies.remove(0) };
    respond(stream, 200, Some(&reply), session_header.as_deref());
}

fn call(state: &State, caller: ApiCaller, params: &Value) -> Result<Value, (i32, String)> {
    let name = params["name"].as_str().ok_or((-32602, "Tool name required".into()))?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !caller.builtin && args.get("_task").is_some() { return Err((-32602, "Reserved task metadata".into())); }
    browser::validate(name, &args).map_err(|e| (-32602, e.to_string()))?;
    let (reply, receive) = mpsc::channel();
    (state.dispatch)(ApiRequest { caller, name: name.into(), args, reply }).map_err(|e| (-32603, e))?;
    let deadline = Instant::now() + CALL_TIMEOUT;
    let result = loop {
        if state.stop.load(Ordering::Acquire) { break Err("Shell API stopped".into()); }
        if Instant::now() >= deadline { break Err("Shell API timed out; the operation may have completed".into()); }
        match receive.recv_timeout(POLL) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("Shell API unavailable".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    Ok(match result {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}],"isError":value.get("failure").is_some(),"structuredContent":value}),
        Err(message) => json!({"content":[{"type":"text","text":message}],"isError":true}),
    })
}
