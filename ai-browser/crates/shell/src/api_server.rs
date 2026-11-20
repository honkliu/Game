//! Profile-scoped, authenticated MCP access to the shared shell API catalog.
//! Native execution is supplied by the UI event loop, not by this transport.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::ipc::Surface;
use ai_platform_contract::browser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{oneshot, watch, Notify, Semaphore};

const MAX_CONNECTIONS: usize = 512;
const MAX_CALLS_PER_KIND: usize = 64;
const MAX_BODY: usize = 1024 * 1024;
const MAX_HEADERS: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SESSIONS: usize = 256;
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_EVENTS: usize = 1024;
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiCaller {
    pub id: u64,
    pub builtin: bool,
    pub surface: Option<Surface>,
}

#[derive(Debug)]
pub struct ApiRequest {
    pub caller: ApiCaller,
    pub name: String,
    pub args: Value,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub url: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UiConnectionInfo {
    pub url: String,
    pub token: String,
    pub cursor: u64,
    pub client_id: String,
    pub revision: u64,
}

impl From<UiConnectionInfo> for ConnectionInfo {
    fn from(info: UiConnectionInfo) -> Self {
        Self {
            url: info.url,
            token: info.token,
            client_id: Some(info.client_id),
        }
    }
}

struct Session {
    caller: ApiCaller,
    credential: String,
    initialized: bool,
    touched: Instant,
    subscriptions: HashSet<String>,
    pending: HashSet<String>,
    changed: Arc<Notify>,
    stream_generation: u64,
    connection: Option<Value>,
    previous_credential: Option<(String, Instant)>,
}

impl Session {
    fn accepts(&self, credential: &str) -> bool {
        matches_token(&self.credential, credential)
            || self
                .previous_credential
                .as_ref()
                .is_some_and(|(previous, until)| {
                    Instant::now() < *until && matches_token(previous, credential)
                })
    }
}

struct PublishedEvent {
    sequence: u64,
    target: Option<Surface>,
    event: String,
    payload: Value,
    bytes: usize,
}

#[derive(Default)]
struct Hub {
    sessions: HashMap<String, Session>,
    events: VecDeque<PublishedEvent>,
    sequence: u64,
    bytes: usize,
}

impl Hub {
    fn prune_sessions(&mut self) {
        self.sessions.retain(|_, session| {
            let keep = session.touched.elapsed() < SESSION_TTL;
            if !keep {
                session.changed.notify_waiters();
            }
            keep
        });
    }
}

struct State {
    shutdown: watch::Sender<bool>,
    config: Mutex<Config>,
    hub: Mutex<Hub>,
    clients: Mutex<Vec<(String, ApiCaller)>>,
    ui_calls: Semaphore,
    agent_calls: Semaphore,
    next_caller: AtomicU64,
    revision: AtomicU64,
    dispatch: Box<dyn Fn(ApiRequest) -> Result<(), String> + Send + Sync>,
}

pub struct ApiServer {
    state: Arc<State>,
    descriptor: PathBuf,
    workers: Vec<JoinHandle<()>>,
    runtime: tokio::runtime::Handle,
    listeners: watch::Sender<Arc<tokio::net::TcpListener>>,
    bind: fn(u16) -> std::io::Result<TcpListener>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    pub token: String,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.token.is_empty()
            || self.token.len() > 512
            || !self.token.bytes().all(|byte| (33..=126).contains(&byte))
        {
            return Err(
                "Token must contain 1-512 printable ASCII characters without spaces".into(),
            );
        }
        Ok(())
    }
}

pub struct PreparedConfig {
    config: Config,
    listener: Option<Arc<tokio::net::TcpListener>>,
}

pub fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "shell API entropy unavailable".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn matches_token(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

impl ApiServer {
    pub fn start(
        data_dir: &Path,
        dispatch: impl Fn(ApiRequest) -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Self::start_configured(
            data_dir,
            Config {
                port: 0,
                token: random_token()?,
            },
            |port| TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)),
            dispatch,
        )
    }

    pub fn start_configured(
        data_dir: &Path,
        mut config: Config,
        bind: fn(u16) -> std::io::Result<TcpListener>,
        dispatch: impl Fn(ApiRequest) -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<Self, String> {
        config.validate()?;
        let listener = bind(config.port)
            .map_err(|e| format!("Could not listen on 0.0.0.0:{}: {e}", config.port))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        config.port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let listener = {
            let _guard = runtime.enter();
            tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?
        };
        let (listeners, listening) = watch::channel(Arc::new(listener));
        let (shutdown, _) = watch::channel(false);
        let state = Arc::new(State {
            shutdown,
            config: Mutex::new(config),
            hub: Mutex::new(Hub::default()),
            clients: Mutex::new(vec![(
                random_token()?,
                ApiCaller {
                    id: 1,
                    builtin: true,
                    surface: None,
                },
            )]),
            ui_calls: Semaphore::new(MAX_CALLS_PER_KIND),
            agent_calls: Semaphore::new(MAX_CALLS_PER_KIND),
            next_caller: AtomicU64::new(2),
            revision: AtomicU64::new(1),
            dispatch: Box::new(dispatch),
        });
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let descriptor = data_dir.join("shell-api.json");
        let active = state.config.lock().unwrap_or_else(|e| e.into_inner());
        let info = ConnectionInfo {
            url: format!("http://127.0.0.1:{}/mcp", active.port),
            token: active.token.clone(),
            client_id: None,
        };
        drop(active);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let encoded = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
        options
            .open(&descriptor)
            .and_then(|mut file| file.write_all(&encoded))
            .map_err(|e| e.to_string())?;
        let mut server = Self {
            state,
            descriptor,
            workers: Vec::new(),
            runtime: runtime.handle().clone(),
            listeners,
            bind,
        };
        let state = Arc::clone(&server.state);
        server.workers.push(
            std::thread::Builder::new()
                .name("shell-api".into())
                .spawn(move || runtime.block_on(accept_requests(listening, state)))
                .map_err(|e| e.to_string())?,
        );
        Ok(server)
    }

    pub fn builtin_connection(&self) -> ConnectionInfo {
        let mut connection = self.connection();
        connection.client_id = self
            .state
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find_map(|(id, caller)| (caller.id == 1).then(|| id.clone()));
        connection
    }

    pub fn connection(&self) -> ConnectionInfo {
        let config = self.state.config.lock().unwrap_or_else(|e| e.into_inner());
        ConnectionInfo {
            url: format!("http://127.0.0.1:{}/mcp", config.port),
            token: config.token.clone(),
            client_id: None,
        }
    }

    pub fn config(&self) -> Config {
        self.state
            .config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn client_id(&self, caller: ApiCaller) -> Result<String, String> {
        let mut clients = self.state.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((id, _)) = clients.iter().find(|(_, bound)| *bound == caller) {
            return Ok(id.clone());
        }
        let id = random_token()?;
        clients.push((id.clone(), caller));
        Ok(id)
    }

    pub fn prepare(&self, config: Config) -> Result<PreparedConfig, String> {
        config.validate()?;
        if config.port == 0 {
            return Err("Port must be between 1 and 65535".into());
        }
        if self.listeners.is_closed() {
            return Err("RiverMCP is not running".into());
        }
        let listener = if config.port != self.config().port {
            let listener = (self.bind)(config.port)
                .map_err(|e| format!("Could not listen on 0.0.0.0:{}: {e}", config.port))?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let _guard = self.runtime.enter();
            Some(Arc::new(
                tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?,
            ))
        } else {
            None
        };
        Ok(PreparedConfig { config, listener })
    }

    pub fn apply(&self, prepared: PreparedConfig) {
        let changed = self.config() != prepared.config;
        *self.state.config.lock().unwrap_or_else(|e| e.into_inner()) = prepared.config;
        if let Some(listener) = prepared.listener {
            self.listeners.send_replace(listener);
        }
        if !changed {
            return;
        }
        let revision = self.state.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let connection = self.connection();
        let mut hub = self.state.hub.lock().unwrap_or_else(|e| e.into_inner());
        hub.sessions.retain(|_, session| {
            if session.caller.builtin {
                session.previous_credential = Some((
                    session.credential.clone(),
                    Instant::now() + Duration::from_secs(30),
                ));
                session.credential = connection.token.clone();
                if session.caller.surface.is_some() {
                    session.connection = Some(
                        json!({"url":connection.url,"token":connection.token,"revision":revision}),
                    );
                }
                session.changed.notify_one();
                true
            } else {
                session.changed.notify_waiters();
                false
            }
        });
        drop(hub);
        // Discovery is an output for tooling, never the source of application settings.
        let _ = std::fs::write(
            &self.descriptor,
            serde_json::to_vec(&connection).unwrap_or_default(),
        );
    }

    pub fn ui_connection(&self, surface: Surface) -> Result<UiConnectionInfo, String> {
        if matches!(surface, Surface::ApiCall(_)) {
            return Err("API calls are not UI surfaces".into());
        }
        let client_id = self.client_id(ApiCaller {
            id: 0,
            builtin: true,
            surface: Some(surface),
        })?;
        let connection = self.connection();
        // Baseline at script issuance, before native code publishes initial surface state.
        let cursor = self
            .state
            .hub
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sequence;
        Ok(UiConnectionInfo {
            url: connection.url,
            token: connection.token,
            cursor,
            client_id,
            revision: self.state.revision.load(Ordering::Acquire),
        })
    }

    pub fn publish(&self, target: Option<Surface>, event: &str, payload: Value) {
        let mut hub = self.state.hub.lock().unwrap_or_else(|e| e.into_inner());
        hub.prune_sessions();
        hub.sequence += 1;
        let sequence = hub.sequence;
        let bytes = payload
            .to_string()
            .len()
            .saturating_add(event.len())
            .saturating_add(64);
        let published = PublishedEvent {
            sequence,
            target,
            event: event.into(),
            payload,
            bytes,
        };
        for session in hub.sessions.values_mut() {
            if visible(session.caller, &published) {
                for uri in std::iter::once("shell://events").chain(browser::event_resource(event)) {
                    if session.subscriptions.contains(uri) {
                        session.pending.insert(uri.into());
                    }
                }
                session.changed.notify_one();
            }
        }
        hub.bytes = hub.bytes.saturating_add(bytes);
        hub.events.push_back(published);
        while hub.events.len() > MAX_EVENTS || hub.bytes > MAX_EVENT_BYTES {
            if let Some(event) = hub.events.pop_front() {
                hub.bytes = hub.bytes.saturating_sub(event.bytes);
            }
        }
    }
}

fn visible(caller: ApiCaller, event: &PublishedEvent) -> bool {
    match (event.target, caller.surface) {
        (Some(target), Some(surface)) => target == surface,
        (Some(_), None) => false,
        (None, Some(Surface::Shell | Surface::Sidebar | Surface::Overlay)) => true,
        (None, Some(_)) => false,
        (None, None) => browser::event_is_public(&event.event),
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.state.shutdown.send_replace(true);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        // An older server must not remove a replacement instance's descriptor.
        let ours = std::fs::read(&self.descriptor)
            .ok()
            .and_then(|v| serde_json::from_slice::<ConnectionInfo>(&v).ok())
            .is_some_and(|info| {
                let active = self.connection();
                info.url == active.url && matches_token(&info.token, &active.token)
            });
        if ours {
            let _ = std::fs::remove_file(&self.descriptor);
        }
    }
}

async fn accept_requests(
    mut listeners: watch::Receiver<Arc<tokio::net::TcpListener>>,
    state: Arc<State>,
) {
    let mut shutdown = state.shutdown.subscribe();
    let mut requests = tokio::task::JoinSet::new();
    loop {
        if *shutdown.borrow() {
            break;
        }
        let listener = Arc::clone(&listeners.borrow_and_update());
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = listeners.changed() => continue,
            Some(_) = requests.join_next(), if !requests.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((mut stream, _peer)) = accepted else { break };
                if requests.len() >= MAX_CONNECTIONS { continue; }
                let state = Arc::clone(&state);
                requests.spawn(async move {
                    let _ = stream.set_nodelay(true);
                    serve(&mut stream, &state).await;
                });
            }
        }
    }
    requests.shutdown().await;
}

struct HttpRequest {
    method: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
    keep_alive: bool,
}

async fn read_http(stream: &mut TcpStream, bytes: &mut Vec<u8>) -> Result<HttpRequest, u16> {
    let deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(bytes).map_err(|_| 400u16)? {
            httparse::Status::Complete(offset) => {
                if offset > MAX_HEADERS {
                    return Err(431);
                }
                if request.path != Some("/mcp") {
                    return Err(404);
                }
                let method = request.method.ok_or(400u16)?.to_string();
                let http11 = request.version == Some(1);
                let mut parsed = HashMap::new();
                for header in request.headers.iter() {
                    let name = header.name.to_ascii_lowercase();
                    let value = std::str::from_utf8(header.value)
                        .map_err(|_| 400u16)?
                        .trim()
                        .to_string();
                    if parsed.insert(name, value).is_some() {
                        return Err(400);
                    }
                }
                // This JSON transport deliberately does not accept transfer codings.
                if parsed.contains_key("transfer-encoding") {
                    return Err(400);
                }
                let length = parsed
                    .get("content-length")
                    .map(|v| v.parse::<usize>().map_err(|_| 400u16))
                    .transpose()?
                    .unwrap_or(0);
                if length > MAX_BODY {
                    return Err(413);
                }
                let keep_alive = http11
                    && !parsed.get("connection").is_some_and(|value| {
                        value
                            .split(',')
                            .any(|value| value.trim().eq_ignore_ascii_case("close"))
                    });
                let end = offset + length;
                while bytes.len() < end {
                    let mut chunk = [0u8; 8192];
                    let size = (end - bytes.len()).min(chunk.len());
                    let n = tokio::time::timeout_at(deadline, stream.read(&mut chunk[..size]))
                        .await
                        .map_err(|_| 408u16)?
                        .map_err(|_| 400u16)?;
                    if n == 0 {
                        return Err(400);
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let body = bytes[offset..end].to_vec();
                bytes.drain(..end);
                return Ok(HttpRequest {
                    method,
                    headers: parsed,
                    body,
                    keep_alive,
                });
            }
            httparse::Status::Partial => {
                if bytes.len() >= MAX_HEADERS {
                    return Err(431);
                }
                let mut chunk = [0u8; 4096];
                let n = tokio::time::timeout_at(deadline, stream.read(&mut chunk))
                    .await
                    .map_err(|_| 408u16)?
                    .map_err(|_| 400u16)?;
                if n == 0 {
                    return Err(400);
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

async fn respond(stream: &mut TcpStream, status: u16, body: Option<&Value>, session: Option<&str>) {
    respond_cors(stream, status, body, session, "", false).await;
}

async fn respond_cors(
    stream: &mut TcpStream,
    status: u16,
    body: Option<&Value>,
    session: Option<&str>,
    cors: &str,
    keep_alive: bool,
) {
    let data = body.map(Value::to_string).unwrap_or_default();
    let session = session
        .map(|s| format!("Mcp-Session-Id: {s}\r\n"))
        .unwrap_or_default();
    let connection = if keep_alive { "keep-alive" } else { "close" };
    let response = format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {connection}\r\nCache-Control: no-store\r\n{session}{cors}\r\n{data}", data.len());
    let _ = tokio::time::timeout(IO_TIMEOUT, stream.write_all(response.as_bytes())).await;
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

async fn serve(stream: &mut TcpStream, state: &State) {
    let mut buffer = Vec::new();
    let mut idle_timeout = IO_TIMEOUT;
    loop {
        if buffer.is_empty() {
            let mut probe = [0u8; 1];
            if !matches!(
                tokio::time::timeout(idle_timeout, stream.peek(&mut probe)).await,
                Ok(Ok(1..))
            ) {
                return;
            }
        }
        let request = match read_http(stream, &mut buffer).await {
            Ok(request) => request,
            Err(status) => return respond(stream, status, None, None).await,
        };
        let keep_alive = request.keep_alive && request.method != "GET";
        serve_request(stream, state, request, keep_alive).await;
        if !keep_alive {
            return;
        }
        idle_timeout = Duration::from_secs(15);
    }
}

async fn serve_request(
    stream: &mut TcpStream,
    state: &State,
    request: HttpRequest,
    keep_alive: bool,
) {
    let host = request
        .headers
        .get("host")
        .map(String::as_str)
        .unwrap_or("");
    let local = match stream.local_addr() {
        Ok(local) => local,
        Err(_) => return,
    };
    let loopback = stream.peer_addr().is_ok_and(|peer| peer.ip().is_loopback());
    let valid_host =
        host == local.to_string() || (loopback && host == format!("localhost:{}", local.port()));
    if !valid_host {
        return respond_cors(stream, 403, None, None, "", keep_alive).await;
    }
    let origin = request.headers.get("origin").map(String::as_str);
    let asset_origin = origin.is_some_and(approved_origin);
    if origin.is_some_and(|origin| !asset_origin && origin != format!("http://{host}")) {
        return respond_cors(stream, 403, None, None, "", keep_alive).await;
    }
    let cors = origin
        .filter(|_| asset_origin)
        .map(cors_headers)
        .unwrap_or_default();
    if request.method == "OPTIONS" {
        let valid = asset_origin
            && request
                .headers
                .get("access-control-request-method")
                .is_some_and(|method| matches!(method.as_str(), "POST" | "GET" | "DELETE"))
            && request
                .headers
                .get("access-control-request-headers")
                .is_none_or(|headers| {
                    headers.split(',').all(|header| {
                        matches!(
                            header.trim().to_ascii_lowercase().as_str(),
                            "authorization"
                                | "content-type"
                                | "mcp-session-id"
                                | "mcp-protocol-version"
                                | "last-event-id"
                                | "accept"
                        )
                    })
                });
        return respond_cors(
            stream,
            if valid { 204 } else { 403 },
            None,
            None,
            if valid { &cors } else { "" },
            keep_alive,
        )
        .await;
    }
    let credential = request
        .headers
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let authenticated = {
        let config = state.config.lock().unwrap_or_else(|e| e.into_inner());
        matches_token(credential, &config.token)
    } || (loopback
        && request.headers.get("mcp-session-id").is_some_and(|id| {
            state
                .hub
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sessions
                .get(id)
                .is_some_and(|session| session.caller.builtin && session.accepts(credential))
        }));
    if !authenticated {
        return respond_cors(stream, 401, None, None, "", keep_alive).await;
    }
    if asset_origin && !loopback {
        return respond_cors(stream, 403, None, None, "", keep_alive).await;
    }
    macro_rules! reply {
        ($status:expr, $body:expr, $session:expr) => {
            respond_cors(stream, $status, $body, $session, &cors, keep_alive).await
        };
    }
    if !matches!(request.method.as_str(), "POST" | "DELETE" | "GET") {
        return reply!(405, None, None);
    }
    let unsupported_protocol = request
        .headers
        .get("mcp-protocol-version")
        .is_some_and(|protocol| protocol != PROTOCOL_VERSION);
    if request.method != "POST" && unsupported_protocol {
        return reply!(400, None, None);
    }
    if request.method == "DELETE" {
        let removed = request.headers.get("mcp-session-id").is_some_and(|id| {
            let mut hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
            if hub.sessions.get(id).is_some_and(|s| {
                s.accepts(credential) && (!asset_origin || s.caller.surface.is_some())
            }) {
                if let Some(session) = hub.sessions.remove(id) {
                    session.changed.notify_waiters();
                }
                true
            } else {
                false
            }
        });
        return reply!(if removed { 200 } else { 404 }, None, None);
    }
    if request.method == "GET" {
        if !request.headers.get("accept").is_some_and(|accept| {
            accept
                .split(',')
                .any(|kind| kind.split(';').next().unwrap_or("").trim() == "text/event-stream")
        }) {
            return reply!(406, None, None);
        }
        let Some(id) = request.headers.get("mcp-session-id") else {
            return reply!(400, None, None);
        };
        let registration = (|| {
            let mut hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
            hub.prune_sessions();
            let session = hub
                .sessions
                .get_mut(id)
                .filter(|s| s.accepts(credential))
                .ok_or(404u16)?;
            if !session.initialized {
                return Err(400);
            }
            if asset_origin && session.caller.surface.is_none() {
                return Err(403);
            }
            session.touched = Instant::now();
            session.stream_generation += 1;
            session
                .pending
                .extend(session.subscriptions.iter().cloned());
            session.changed.notify_waiters();
            Ok((session.stream_generation, Arc::clone(&session.changed)))
        })();
        return match registration {
            Ok((generation, changed)) => {
                stream_events(stream, state, id, generation, changed, &cors).await
            }
            Err(status) => reply!(status, None, None),
        };
    }
    if request
        .headers
        .get("content-type")
        .is_none_or(|v| v.split(';').next().unwrap_or("").trim() != "application/json")
    {
        return reply!(415, None, None);
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(_) => {
            return reply!(
                400,
                Some(&rpc_error(&Value::Null, -32700, "Invalid JSON")),
                None
            )
        }
    };
    // A batch shares the HTTP session; initialization is always its own request.
    let messages = body
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![body.clone()]);
    if messages.is_empty() || messages.len() > 32 {
        return reply!(400, None, None);
    }
    let initializing = messages.len() == 1 && messages[0]["method"] == "initialize";
    if !initializing && unsupported_protocol {
        return reply!(400, None, None);
    }
    let mut session_header = None;
    let caller = if initializing {
        if request.headers.contains_key("mcp-session-id") {
            return reply!(400, None, None);
        }
        if messages[0]["jsonrpc"] != "2.0" || messages[0].get("id").is_none() {
            return reply!(400, None, None);
        }
        if messages[0]
            .pointer("/params/protocolVersion")
            .and_then(Value::as_str)
            .is_none_or(|version| version.is_empty())
        {
            return reply!(
                200,
                Some(&rpc_error(
                    &messages[0]["id"],
                    -32602,
                    "protocolVersion must be a non-empty string"
                )),
                None
            );
        }
        let session_id = match random_token() {
            Ok(id) => id,
            Err(_) => return reply!(503, None, None),
        };
        let native_client = messages[0]
            .pointer("/params/_meta/river.client_id")
            .and_then(Value::as_str);
        let native_caller = if loopback {
            state
                .clients
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find_map(|(id, caller)| {
                    native_client
                        .filter(|client| matches_token(client, id))
                        .map(|_| *caller)
                })
        } else {
            None
        };
        if asset_origin && !native_caller.is_some_and(|caller| caller.surface.is_some()) {
            return reply!(403, None, None);
        }
        let caller = native_caller.unwrap_or_else(|| ApiCaller {
            id: state.next_caller.fetch_add(1, Ordering::Relaxed),
            builtin: false,
            surface: None,
        });
        let inserted = {
            let mut hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
            hub.prune_sessions();
            if hub.sessions.len() >= MAX_SESSIONS {
                false
            } else {
                hub.sessions.insert(
                    session_id.clone(),
                    Session {
                        caller,
                        credential: credential.into(),
                        initialized: false,
                        touched: Instant::now(),
                        subscriptions: HashSet::new(),
                        pending: HashSet::new(),
                        changed: Arc::new(Notify::new()),
                        stream_generation: 0,
                        connection: None,
                        previous_credential: None,
                    },
                );
                true
            }
        };
        if !inserted {
            return reply!(503, None, None);
        }
        session_header = Some(session_id);
        caller
    } else {
        let Some(id) = request.headers.get("mcp-session-id") else {
            return reply!(400, None, None);
        };
        let caller = (|| {
            let mut hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
            hub.prune_sessions();
            let session = hub
                .sessions
                .get_mut(id)
                .filter(|s| s.accepts(credential))
                .ok_or(404u16)?;
            if asset_origin && session.caller.surface.is_none() {
                return Err(403);
            }
            if !session.initialized
                && !messages
                    .iter()
                    .all(|v| v["method"] == "notifications/initialized" || v["method"] == "ping")
            {
                return Err(400);
            }
            if messages
                .iter()
                .any(|v| v["method"] == "notifications/initialized")
            {
                session.initialized = true;
            }
            session.touched = Instant::now();
            Ok(session.caller)
        })();
        match caller {
            Ok(caller) => caller,
            Err(status) => return reply!(status, None, None),
        }
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
            if matches!(
                method,
                "notifications/initialized" | "notifications/cancelled"
            ) {
                continue;
            }
            return reply!(400, None, None);
        }
        let result = match method {
            "initialize" if initializing => Ok(
                json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{"listChanged":false},"resources":{"subscribe":true,"listChanged":false}},"serverInfo":{"name":"ai-browser-shell","version":env!("CARGO_PKG_VERSION")},"instructions":browser::CLIENT_INSTRUCTIONS}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(
                json!({"tools":browser::definitions().into_iter().map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.parameters,"annotations":{"readOnlyHint":tool.read_only}})).collect::<Vec<_>>()}),
            ),
            "tools/call" => call(state, caller, &message["params"]).await,
            "resources/list" => {
                if message["params"].get("cursor").is_some() {
                    Err((-32602, "Unknown resource-list cursor".into()))
                } else {
                    Ok(json!({"resources":browser::resources()}))
                }
            }
            "resources/templates/list" => {
                if message["params"].get("cursor").is_some() {
                    Err((-32602, "Unknown resource-template cursor".into()))
                } else {
                    Ok(json!({"resourceTemplates":browser::resource_templates()}))
                }
            }
            "resources/read" => read_resource(state, caller, &message["params"]).await,
            "resources/subscribe" | "resources/unsubscribe" => subscribe(
                state,
                request.headers.get("mcp-session-id").map(String::as_str),
                &message["params"],
                method == "resources/subscribe",
            ),
            _ => Err((-32601, "Unknown method".to_string())),
        };
        replies.push(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => rpc_error(&id, code, &message),
        });
    }
    if replies.is_empty() {
        return reply!(202, None, session_header.as_deref());
    }
    let reply = if body.is_array() {
        Value::Array(replies)
    } else {
        replies.remove(0)
    };
    reply!(200, Some(&reply), session_header.as_deref());
}

fn approved_origin(origin: &str) -> bool {
    matches!(
        origin,
        "http://asset.shell"
            | "http://asset.pages"
            | "https://asset.shell"
            | "https://asset.pages"
            | "asset://shell"
            | "asset://pages"
            | "null"
    )
}

fn cors_headers(origin: &str) -> String {
    format!("Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: POST, GET, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Mcp-Session-Id, Mcp-Protocol-Version, Last-Event-ID, Accept\r\nAccess-Control-Expose-Headers: Mcp-Session-Id, Mcp-Protocol-Version\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Max-Age: 3600\r\n")
}

fn subscribe(
    state: &State,
    session: Option<&str>,
    params: &Value,
    subscribe: bool,
) -> Result<Value, (i32, String)> {
    let uri = params["uri"]
        .as_str()
        .ok_or((-32602, "Resource URI required".into()))?;
    if !browser::resources()
        .iter()
        .any(|resource| resource.uri == uri)
    {
        return Err((-32002, "Unknown resource".into()));
    }
    let mut hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
    let session = session
        .and_then(|id| hub.sessions.get_mut(id))
        .ok_or((-32602, "Session required".into()))?;
    if subscribe {
        session.subscriptions.insert(uri.into());
        session.pending.insert(uri.into());
    } else {
        session.subscriptions.remove(uri);
        session.pending.remove(uri);
    }
    session.changed.notify_one();
    Ok(json!({}))
}

async fn read_resource(
    state: &State,
    caller: ApiCaller,
    params: &Value,
) -> Result<Value, (i32, String)> {
    let uri = params["uri"]
        .as_str()
        .ok_or((-32602, "Resource URI required".into()))?;
    let value = if uri == "shell://events" || uri.starts_with("shell://events?") {
        let after = if uri == "shell://events" {
            None
        } else {
            let cursor = uri
                .strip_prefix("shell://events?after=")
                .ok_or((-32602, "Invalid event cursor".into()))?;
            if cursor.is_empty() || !cursor.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err((-32602, "Invalid event cursor".into()));
            }
            Some(
                cursor
                    .parse::<u64>()
                    .map_err(|_| (-32602, "Invalid event cursor".into()))?,
            )
        };
        let hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
        if after.is_some_and(|after| after > hub.sequence) {
            return Err((-32602, "Future event cursor".into()));
        }
        let oldest = hub
            .events
            .front()
            .map(|event| event.sequence)
            .unwrap_or_else(|| hub.sequence.saturating_add(1));
        let events: Vec<_> = after.into_iter().flat_map(|after| hub.events.iter().filter(move |event| event.sequence > after && visible(caller, event)))
            .map(|event| json!({"sequence":event.sequence,"event":event.event,"payload":event.payload})).collect();
        json!({"sequence":hub.sequence,"oldest_sequence":oldest,"reset":after.is_some_and(|after| after.saturating_add(1) < oldest),"events":events})
    } else if uri == "shell://interface" {
        json!({"tools":browser::definitions(),"resources":browser::resources(),"resourceTemplates":browser::resource_templates(),"events":browser::events(),"instructions":browser::CLIENT_INSTRUCTIONS})
    } else {
        if !browser::resources()
            .iter()
            .any(|resource| resource.uri == uri)
        {
            return Err((-32002, "Unknown resource".into()));
        }
        dispatch(state, caller, "resources/read", json!({"uri":uri}))
            .await
            .map_err(|error| (-32603, error))?
    };
    Ok(json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}))
}

async fn stream_events(
    stream: &mut TcpStream,
    state: &State,
    id: &str,
    generation: u64,
    changed: Arc<Notify>,
    cors: &str,
) {
    let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\nX-Accel-Buffering: no\r\n{cors}\r\n: connected\n\n");
    if !write_stream(stream, headers.as_bytes()).await {
        return;
    }
    loop {
        // Register before examining pending updates so a concurrent publication cannot be lost.
        let notified = changed.notified();
        let (pending, connection) = {
            let mut hub = state.hub.lock().unwrap_or_else(|e| e.into_inner());
            let Some(session) = hub.sessions.get_mut(id) else {
                return;
            };
            if session.stream_generation != generation {
                return;
            }
            session.touched = Instant::now();
            let mut pending: Vec<_> = session.pending.drain().collect();
            pending.sort();
            (pending, session.connection.take())
        };
        if let Some(connection) = connection {
            let notification = json!({"jsonrpc":"2.0","method":"notifications/river/connection_changed","params":connection});
            if !write_stream(
                stream,
                format!("event: message\ndata: {notification}\n\n").as_bytes(),
            )
            .await
            {
                return;
            }
        }
        if !pending.is_empty() {
            let mut buffer = String::new();
            for uri in pending {
                let notification = json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"uri":uri}});
                buffer.push_str(&format!("event: message\ndata: {notification}\n\n"));
            }
            if !write_stream(stream, buffer.as_bytes()).await {
                return;
            }
            continue;
        }
        let mut probe = [0u8; 1];
        tokio::select! {
            _ = notified => {},
            _ = stream.peek(&mut probe) => return,
            _ = tokio::time::sleep(Duration::from_secs(15)) => {
                if !write_stream(stream, b": heartbeat\n\n").await { return; }
            }
        }
    }
}

async fn write_stream(stream: &mut TcpStream, bytes: &[u8]) -> bool {
    matches!(
        tokio::time::timeout(IO_TIMEOUT, stream.write_all(bytes)).await,
        Ok(Ok(()))
    )
}

async fn call(state: &State, caller: ApiCaller, params: &Value) -> Result<Value, (i32, String)> {
    let name = params["name"]
        .as_str()
        .ok_or((-32602, "Tool name required".into()))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !caller.builtin && args.get("_task").is_some() {
        return Err((-32602, "Reserved task metadata".into()));
    }
    browser::validate(name, &args).map_err(|e| (-32602, e.to_string()))?;
    let result = dispatch(state, caller, name, args).await;
    Ok(match result {
        Ok(value) => {
            json!({"content":[{"type":"text","text":value.to_string()}],"isError":value.get("failure").is_some(),"structuredContent":value})
        }
        Err(message) => json!({"content":[{"type":"text","text":message}],"isError":true}),
    })
}

async fn dispatch(
    state: &State,
    caller: ApiCaller,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let calls = if caller.surface.is_some() {
        &state.ui_calls
    } else {
        &state.agent_calls
    };
    let _permit = calls
        .try_acquire()
        .map_err(|_| "Too many pending Shell calls".to_string())?;
    let (reply, receive) = oneshot::channel();
    (state.dispatch)(ApiRequest {
        caller,
        name: name.into(),
        args,
        reply,
    })?;
    let timeout = if caller.surface.is_some() {
        Duration::from_secs(150)
    } else {
        CALL_TIMEOUT
    };
    match tokio::time::timeout(timeout, receive).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("Shell API unavailable".into()),
        Err(_) => Err("Shell API timed out; the operation may have completed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpStream;
    use std::sync::mpsc;

    struct Fixture {
        server: Option<ApiServer>,
        directory: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_dispatch(|request| {
                request.reply.send(Ok(json!({"name":request.name,"args":request.args,"caller":request.caller.id,"builtin":request.caller.builtin,"surface":request.caller.surface.map(Surface::target)}))).map_err(|_| "receiver closed".into())
            })
        }

        fn with_dispatch(
            dispatch: impl Fn(ApiRequest) -> Result<(), String> + Send + Sync + 'static,
        ) -> Self {
            let directory = std::env::current_dir()
                .unwrap()
                .join(format!(".api-server-test-{}", random_token().unwrap()));
            let server = ApiServer::start(&directory, dispatch).unwrap();
            Self {
                server: Some(server),
                directory,
            }
        }

        fn server(&self) -> &ApiServer {
            self.server.as_ref().unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            drop(self.server.take());
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    struct Response {
        status: u16,
        headers: HashMap<String, String>,
        body: Value,
    }

    fn socket(info: &ConnectionInfo) -> TcpStream {
        let address = info
            .url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/mcp")
            .unwrap();
        let socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
    }

    fn send(
        info: &ConnectionInfo,
        method: &str,
        session: Option<&str>,
        body: Value,
        extra: &str,
    ) -> Response {
        let mut socket = socket(info);
        let address = info
            .url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/mcp")
            .unwrap();
        let body = body.to_string();
        let session = session
            .map(|id| format!("Mcp-Session-Id: {id}\r\n"))
            .unwrap_or_default();
        write!(socket, "{method} /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{session}{extra}\r\n{body}", info.token, body.len()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let mut lines = headers.lines();
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().into()))
            .collect();
        Response {
            status,
            headers,
            body: serde_json::from_str(body).unwrap_or(Value::Null),
        }
    }

    fn rpc(info: &ConnectionInfo, session: &str, method: &str, params: Value) -> Response {
        send(
            info,
            "POST",
            Some(session),
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
            "",
        )
    }

    fn initialize(info: &ConnectionInfo) -> String {
        let response = send(
            info,
            "POST",
            None,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","_meta":{"river.client_id":info.client_id}}}),
            "",
        );
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body["result"]["capabilities"]["resources"]["subscribe"],
            true
        );
        let session = response.headers.get("mcp-session-id").unwrap().clone();
        let response = send(
            info,
            "POST",
            Some(&session),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            "",
        );
        assert_eq!(response.status, 202);
        session
    }

    fn resource(info: &ConnectionInfo, session: &str, uri: &str) -> Value {
        let response = rpc(info, session, "resources/read", json!({"uri":uri}));
        assert_eq!(response.status, 200);
        serde_json::from_str(
            response.body["result"]["contents"][0]["text"]
                .as_str()
                .expect("resource contents missing"),
        )
        .unwrap()
    }

    fn stream(info: &ConnectionInfo, session: &str) -> BufReader<TcpStream> {
        let mut socket = socket(info);
        let address = info
            .url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/mcp")
            .unwrap();
        write!(socket, "GET /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nMcp-Session-Id: {session}\r\nAccept: text/event-stream\r\n\r\n", info.token).unwrap();
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("HTTP/1.1 200"));
        loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
        }
        reader
    }

    fn notification(reader: &mut BufReader<TcpStream>) -> Value {
        let mut data = None;
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if let Some(value) = line.strip_prefix("data: ") {
                data = Some(serde_json::from_str(value.trim()).unwrap());
            }
            if line == "\n" {
                if let Some(value) = data.take() {
                    return value;
                }
            }
        }
    }

    fn subscribe_resource(info: &ConnectionInfo, session: &str, uri: &str) {
        let response = rpc(info, session, "resources/subscribe", json!({"uri":uri}));
        assert_eq!(response.body["result"], json!({}));
    }

    fn assert_stream_closed(reader: &mut BufReader<TcpStream>) {
        // Winsock can report a reset instead of EOF when a terminated stream had data in flight.
        if let Err(error) = std::io::copy(reader, &mut std::io::sink()) {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset,
                "stream did not close after explicit teardown"
            );
        }
    }

    #[test]
    fn initialization_offers_supported_version_and_validates_subsequent_headers() {
        let fixture = Fixture::new();
        let info = fixture.server().connection();
        for requested in [PROTOCOL_VERSION, "2025-06-18", "2025-11-25", "2099-01-01"] {
            for extra in [
                String::new(),
                format!("MCP-Protocol-Version: {requested}\r\n"),
            ] {
                let response = send(
                    &info,
                    "POST",
                    None,
                    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":requested}}),
                    &extra,
                );
                assert_eq!(response.status, 200);
                assert_eq!(response.body["result"]["protocolVersion"], PROTOCOL_VERSION);
                let session = response.headers.get("mcp-session-id").unwrap();
                let protocol = format!("MCP-Protocol-Version: {PROTOCOL_VERSION}\r\n");
                let ready = send(
                    &info,
                    "POST",
                    Some(session),
                    json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                    &protocol,
                );
                assert_eq!(ready.status, 202);
                let tools = send(
                    &info,
                    "POST",
                    Some(session),
                    json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                    &protocol,
                );
                assert_eq!(tools.status, 200);
                assert!(tools.body["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == "shell_tab"));
                for method in ["POST", "GET", "DELETE"] {
                    let invalid = send(
                        &info,
                        method,
                        Some(session),
                        json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
                        "MCP-Protocol-Version: unsupported\r\n",
                    );
                    assert_eq!(invalid.status, 400);
                }
                assert_eq!(
                    send(&info, "DELETE", Some(session), Value::Null, &protocol).status,
                    200
                );
            }
        }
    }

    #[test]
    fn initialization_rejects_missing_or_invalid_version_without_creating_a_session() {
        let fixture = Fixture::new();
        let server = fixture.server();
        let info = server.connection();
        for params in [
            json!({}),
            json!({"protocolVersion":null}),
            json!({"protocolVersion":42}),
            json!({"protocolVersion":""}),
        ] {
            let response = send(
                &info,
                "POST",
                None,
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":params}),
                "",
            );
            assert_eq!(response.status, 200);
            assert_eq!(response.body["error"]["code"], -32602);
            assert!(!response.headers.contains_key("mcp-session-id"));
            assert!(server.state.hub.lock().unwrap().sessions.is_empty());
        }
    }

    #[test]
    fn listener_reconfiguration_is_transactional_and_migrates_native_sessions_only() {
        let fixture = Fixture::new();
        let server = fixture.server();
        assert!(server
            .listeners
            .borrow()
            .local_addr()
            .unwrap()
            .ip()
            .is_unspecified());
        let old = server.config();
        let ui: ConnectionInfo = server.ui_connection(Surface::Shell).unwrap().into();
        let ui_session = initialize(&ui);
        subscribe_resource(&ui, &ui_session, "shell://events");
        let mut updates = stream(&ui, &ui_session);
        notification(&mut updates);
        let external = server.connection();
        let external_session = initialize(&external);
        let occupied = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        assert!(server
            .prepare(Config {
                port: occupied.local_addr().unwrap().port(),
                token: "next-token".into()
            })
            .is_err());
        assert_eq!(server.config(), old);
        assert_eq!(rpc(&ui, &ui_session, "ping", json!({})).status, 200);
        let free = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let prepared = server
            .prepare(Config {
                port,
                token: "next-token".into(),
            })
            .unwrap();
        assert_eq!(server.config(), old);
        server.apply(prepared);
        let migration = notification(&mut updates);
        assert_eq!(
            migration["method"],
            "notifications/river/connection_changed"
        );
        assert_eq!(migration["params"]["token"], "next-token");
        assert!(migration["params"]["url"]
            .as_str()
            .unwrap()
            .contains(&format!(":{port}/mcp")));
        let current = server.connection();
        assert_eq!(current.token, server.builtin_connection().token);
        let mut revoked = current.clone();
        revoked.token = old.token;
        assert_eq!(
            rpc(&revoked, &external_session, "ping", json!({})).status,
            401
        );
        assert_eq!(
            rpc(&current, &external_session, "ping", json!({})).status,
            404
        );
        let next_ui: ConnectionInfo = server.ui_connection(Surface::Shell).unwrap().into();
        assert_eq!(next_ui.client_id, ui.client_id);
        let reconnected = initialize(&next_ui);
        assert_eq!(
            resource(&next_ui, &reconnected, "shell://context")["surface"],
            "shell"
        );
    }

    #[test]
    fn token_changes_on_the_same_port_revoke_external_sessions_without_changing_ui_identity() {
        let fixture = Fixture::new();
        let server = fixture.server();
        let original = server.connection();
        let session = initialize(&original);
        let before = server.ui_connection(Surface::Sidebar).unwrap();
        let prepared = server
            .prepare(Config {
                port: server.config().port,
                token: "replacement-token".into(),
            })
            .unwrap();
        assert!(prepared.listener.is_none());
        server.apply(prepared);
        assert_eq!(server.connection().url, original.url);
        assert_eq!(rpc(&original, &session, "ping", json!({})).status, 401);
        assert_eq!(
            server.ui_connection(Surface::Sidebar).unwrap().client_id,
            before.client_id
        );
        let session = initialize(&server.connection());
        assert_eq!(
            rpc(&server.connection(), &session, "ping", json!({})).status,
            200
        );
    }

    #[test]
    fn persistent_connections_preserve_pipelined_requests_and_honor_close() {
        let fixture = Fixture::new();
        let info = fixture.server().connection();
        let session = initialize(&info);
        let mut socket = socket(&info);
        let address = info
            .url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/mcp")
            .unwrap();
        for id in 1..=2 {
            let body = json!({"jsonrpc":"2.0","id":id,"method":"ping"}).to_string();
            let connection = if id == 1 { "keep-alive" } else { "close" };
            write!(socket, "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nMcp-Session-Id: {session}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n{body}", info.token, body.len()).unwrap();
        }
        let mut reader = BufReader::new(socket);
        for id in 1..=2 {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("HTTP/1.1 200"));
            let mut length = None;
            let mut connection = None;
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length: ") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
                if let Some(value) = line.strip_prefix("Connection: ") {
                    connection = Some(value.trim().to_string());
                }
            }
            assert_eq!(
                connection.as_deref(),
                Some(if id == 1 { "keep-alive" } else { "close" })
            );
            let mut body = vec![0; length.unwrap()];
            reader.read_exact(&mut body).unwrap();
            let reply: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(reply["id"], id);
            assert_eq!(reply["result"], json!({}));
        }
        assert_eq!(reader.read(&mut [0u8; 1]).unwrap(), 0);
    }

    #[test]
    fn incomplete_http_requests_do_not_delay_commands_or_notifications() {
        let fixture = Fixture::new();
        let info = fixture.server().connection();
        let session = initialize(&info);
        subscribe_resource(&info, &session, "shell://events");
        let mut events = stream(&info, &session);
        notification(&mut events);
        let address = info
            .url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/mcp")
            .unwrap();
        let _pending: Vec<_> = (0..16)
            .map(|_| {
                let mut socket = socket(&info);
                write!(
                    socket,
                    "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Length: 100\r\n\r\n"
                )
                .unwrap();
                socket
            })
            .collect();
        let start = Instant::now();
        let response = rpc(
            &info,
            &session,
            "tools/call",
            json!({"name":"shell_tab","arguments":{"operation":"list"}}),
        );
        assert_eq!(response.body["result"]["isError"], false);
        fixture.server().publish(None, "tab.updated", json!({}));
        assert_eq!(notification(&mut events)["params"]["uri"], "shell://events");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "incomplete requests stalled independent work"
        );
    }

    #[test]
    fn pending_agent_calls_are_bounded_without_consuming_ui_capacity() {
        let (sent, received) = mpsc::channel();
        let fixture = Fixture::with_dispatch(move |request| {
            if request.caller.surface.is_some() {
                request
                    .reply
                    .send(Ok(json!({"responsive":true})))
                    .map_err(|_| "closed".into())
            } else {
                sent.send(request).map_err(|_| "closed".into())
            }
        });
        let info = fixture.server().connection();
        let session = initialize(&info);
        let ui: ConnectionInfo = fixture
            .server()
            .ui_connection(Surface::Shell)
            .unwrap()
            .into();
        let ui_session = initialize(&ui);
        let mut calls = Vec::new();
        let mut pending = Vec::new();
        for _ in 0..MAX_CALLS_PER_KIND {
            let (info, session) = (info.clone(), session.clone());
            calls.push(std::thread::spawn(move || {
                rpc(
                    &info,
                    &session,
                    "tools/call",
                    json!({"name":"shell_tab","arguments":{"operation":"list"}}),
                )
            }));
            pending.push(
                received
                    .recv_timeout(Duration::from_secs(3))
                    .expect("pending native call occupied a request worker"),
            );
        }
        let overload = rpc(
            &info,
            &session,
            "tools/call",
            json!({"name":"shell_tab","arguments":{"operation":"list"}}),
        );
        assert_eq!(overload.body["result"]["isError"], true);
        assert!(overload.body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Too many pending"));
        let start = Instant::now();
        let response = rpc(
            &ui,
            &ui_session,
            "tools/call",
            json!({"name":"shell_tab","arguments":{"operation":"list"}}),
        );
        assert_eq!(
            response.body["result"]["structuredContent"]["responsive"],
            true
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "agent calls stalled the UI"
        );
        for request in pending {
            request.reply.send(Ok(json!({"done":true}))).unwrap();
        }
        for call in calls {
            assert_eq!(call.join().unwrap().body["result"]["isError"], false);
        }
        assert_eq!(
            fixture.server().state.agent_calls.available_permits(),
            MAX_CALLS_PER_KIND
        );
    }

    #[test]
    fn discovery_reads_and_existing_tools_share_the_dispatch() {
        let fixture = Fixture::new();
        let info = fixture.server().connection();
        let session = initialize(&info);
        let response = rpc(&info, &session, "resources/list", json!({}));
        assert_eq!(
            response.body["result"]["resources"]
                .as_array()
                .unwrap()
                .len(),
            9
        );
        assert!(response.body["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource["description"]
                .as_str()
                .is_some_and(|description| !description.is_empty())));
        let response = rpc(&info, &session, "resources/templates/list", json!({}));
        assert_eq!(
            response.body["result"]["resourceTemplates"][0]["uriTemplate"],
            "shell://events{?after}"
        );
        let interface = resource(&info, &session, "shell://interface");
        assert_eq!(interface["events"].as_array().unwrap().len(), 50);
        assert_eq!(
            interface["tools"].as_array().unwrap().len(),
            browser::definitions().len()
        );
        let context = resource(&info, &session, "shell://context");
        assert_eq!(context["name"], "resources/read");
        assert_eq!(context["args"], json!({"uri":"shell://context"}));
        assert_eq!(context["surface"], Value::Null);
        assert_eq!(context["builtin"], false);
        let tools = rpc(&info, &session, "tools/list", json!({}));
        assert_eq!(
            tools.body["result"]["tools"].as_array().unwrap().len(),
            browser::definitions().len()
        );
        let tool = rpc(
            &info,
            &session,
            "tools/call",
            json!({"name":"shell_tab","arguments":{"operation":"list"}}),
        );
        assert_eq!(
            tool.body["result"]["structuredContent"]["name"],
            "shell_tab"
        );
        assert_eq!(tool.body["result"]["isError"], false);
        let reserved = rpc(
            &info,
            &session,
            "tools/call",
            json!({"name":"shell_tab","arguments":{"operation":"list","_task":1}}),
        );
        assert_eq!(reserved.body["error"]["code"], -32602);
        let unknown = rpc(
            &info,
            &session,
            "resources/read",
            json!({"uri":"shell://unknown"}),
        );
        assert_eq!(unknown.body["error"]["code"], -32002);
        assert_eq!(
            rpc(
                &info,
                &session,
                "resources/subscribe",
                json!({"uri":"shell://unknown"})
            )
            .body["error"]["code"],
            -32002
        );
    }

    #[test]
    fn cursor_replay_filters_private_events_and_keeps_global_progress() {
        let fixture = Fixture::new();
        let server = fixture.server();
        let external = server.connection();
        let shell: ConnectionInfo = server.ui_connection(Surface::Shell).unwrap().into();
        let sidebar: ConnectionInfo = server.ui_connection(Surface::Sidebar).unwrap().into();
        let settings: ConnectionInfo = server
            .ui_connection(Surface::Settings(crate::tabs::TabId(42)))
            .unwrap()
            .into();
        let external_session = initialize(&external);
        let shell_session = initialize(&shell);
        let sidebar_session = initialize(&sidebar);
        let settings_session = initialize(&settings);
        server.publish(None, "tab.created", json!({"id":1}));
        server.publish(None, "shell.toast", json!({"message":"private"}));
        server.publish(
            Some(Surface::Shell),
            "ai.answer",
            json!({"answer":"targeted"}),
        );
        server.publish(
            Some(Surface::Settings(crate::tabs::TabId(42))),
            "state.settings",
            json!({"value":"settings only"}),
        );
        let baseline = resource(&shell, &shell_session, "shell://events");
        assert_eq!(baseline["sequence"], 4);
        assert_eq!(baseline["events"], json!([]));
        let external_events = resource(&external, &external_session, "shell://events?after=0");
        assert_eq!(external_events["sequence"], 4);
        assert_eq!(external_events["events"].as_array().unwrap().len(), 1);
        assert_eq!(external_events["events"][0]["event"], "tab.created");
        assert_eq!(
            resource(&shell, &shell_session, "shell://events?after=0")["events"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            resource(&sidebar, &sidebar_session, "shell://events?after=0")["events"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let settings_events = resource(&settings, &settings_session, "shell://events?after=0");
        assert_eq!(settings_events["events"].as_array().unwrap().len(), 1);
        assert_eq!(settings_events["events"][0]["sequence"], 4);
        for uri in [
            "shell://events?after=5",
            "shell://events?after=-1",
            "shell://events?after=",
            "shell://events?before=0",
            "shell://events?after=0&x=1",
            "shell://events?after=18446744073709551616",
        ] {
            assert_eq!(
                rpc(
                    &external,
                    &external_session,
                    "resources/read",
                    json!({"uri":uri})
                )
                .body["error"]["code"],
                -32602
            );
        }
        let context = resource(&shell, &shell_session, "shell://context");
        assert_eq!(context["caller"], 0);
        assert_eq!(context["builtin"], true);
        assert_eq!(context["surface"], "shell");
        // A shared bearer authorizes access; only native-issued identities establish UI roles.
        assert_eq!(
            resource(&external, &external_session, "shell://context")["builtin"],
            false
        );
        assert_ne!(shell.client_id, sidebar.client_id);
        let descriptor: ConnectionInfo =
            serde_json::from_slice(&std::fs::read(&server.descriptor).unwrap()).unwrap();
        assert!(matches_token(&descriptor.token, &external.token));
        assert!(matches_token(&descriptor.token, &shell.token));
    }

    #[test]
    fn ui_bootstrap_cursor_precedes_initial_publications_and_refreshes_on_script_reissue() {
        let fixture = Fixture::new();
        let server = fixture.server();
        server.publish(Some(Surface::Overlay), "overlay.state", json!({"old":true}));
        for (surface, event) in [
            (Surface::Overlay, "overlay.state"),
            (Surface::Sidebar, "sidebar.selection"),
        ] {
            let bootstrap = server.ui_connection(surface).unwrap();
            let cursor = bootstrap.cursor;
            server.publish(Some(surface), event, json!({"initial":true}));
            let reissued = server.ui_connection(surface).unwrap();
            assert!(matches_token(&bootstrap.token, &reissued.token));
            assert_eq!(reissued.cursor, cursor + 1);
            let info: ConnectionInfo = bootstrap.into();
            let session = initialize(&info);
            let replay = resource(&info, &session, &format!("shell://events?after={cursor}"));
            assert_eq!(replay["events"].as_array().unwrap().len(), 1);
            assert_eq!(replay["events"][0]["event"], event);
            assert_eq!(replay["events"][0]["payload"], json!({"initial":true}));
            assert_eq!(
                resource(&info, &session, "shell://events")["events"],
                json!([])
            );
        }
        let settings: ConnectionInfo = server
            .ui_connection(Surface::Settings(crate::tabs::TabId(42)))
            .unwrap()
            .into();
        let session = initialize(&settings);
        assert_eq!(
            resource(&settings, &session, "shell://events?after=0")["events"],
            json!([])
        );
        for uri in ["shell://overlay", "shell://sidebar", "shell://shell"] {
            assert_eq!(
                rpc(&settings, &session, "resources/read", json!({"uri":uri})).body["error"]
                    ["code"],
                -32002
            );
        }
    }

    #[test]
    fn bounded_replay_detects_gaps_and_prunes_inactive_sessions() {
        let fixture = Fixture::new();
        let server = fixture.server();
        let caller = ApiCaller {
            id: 2,
            builtin: false,
            surface: None,
        };
        for index in 0..MAX_EVENTS + 2 {
            server.publish(None, "tab.updated", json!({"index":index}));
        }
        let read = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(read_resource(
                &server.state,
                caller,
                &json!({"uri":"shell://events?after=0"}),
            ))
            .unwrap();
        let replay: Value =
            serde_json::from_str(read["contents"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(replay["events"].as_array().unwrap().len(), MAX_EVENTS);
        assert_eq!(replay["oldest_sequence"], 3);
        assert_eq!(replay["reset"], true);
        server.publish(None, "ai.delta", json!("x".repeat(MAX_EVENT_BYTES)));
        let hub = server.state.hub.lock().unwrap();
        assert!(hub.bytes <= MAX_EVENT_BYTES);
        assert!(hub.events.is_empty());
        drop(hub);
        let info = server.connection();
        let session = initialize(&info);
        server
            .state
            .hub
            .lock()
            .unwrap()
            .sessions
            .get_mut(&session)
            .unwrap()
            .touched = Instant::now() - SESSION_TTL - Duration::from_secs(1);
        server.publish(None, "tab.updated", json!({}));
        assert_eq!(rpc(&info, &session, "ping", json!({})).status, 404);
    }

    #[test]
    fn sse_notifies_reconnects_unsubscribes_and_does_not_starve_tools() {
        let mut fixture = Fixture::new();
        let info = fixture.server().connection();
        let mut streams = Vec::new();
        let mut sessions = Vec::new();
        for _ in 0..10 {
            let session = initialize(&info);
            subscribe_resource(&info, &session, "shell://events");
            let mut reader = stream(&info, &session);
            let update = notification(&mut reader);
            assert_eq!(update["method"], "notifications/resources/updated");
            assert_eq!(update["params"]["uri"], "shell://events");
            streams.push(reader);
            sessions.push(session);
        }
        let response = rpc(
            &info,
            &sessions[0],
            "tools/call",
            json!({"name":"shell_tab","arguments":{"operation":"list"}}),
        );
        assert_eq!(response.body["result"]["isError"], false);
        fixture
            .server()
            .publish(None, "tab.created", json!({"id":5}));
        for reader in &mut streams {
            assert_eq!(notification(reader)["params"]["uri"], "shell://events");
        }
        let response = rpc(
            &info,
            &sessions[0],
            "resources/unsubscribe",
            json!({"uri":"shell://events"}),
        );
        assert_eq!(response.body["result"], json!({}));
        subscribe_resource(&info, &sessions[0], "shell://tabs");
        assert_eq!(
            notification(&mut streams[0])["params"]["uri"],
            "shell://tabs"
        );
        fixture
            .server()
            .publish(None, "tab.updated", json!({"id":5}));
        assert_eq!(
            notification(&mut streams[0])["params"]["uri"],
            "shell://tabs"
        );
        // Reopening the stream re-notifies subscriptions even if an update happened before GET.
        let mut replacement = stream(&info, &sessions[0]);
        assert_eq!(
            notification(&mut replacement)["params"]["uri"],
            "shell://tabs"
        );
        let response = send(&info, "DELETE", Some(&sessions[0]), Value::Null, "");
        assert_eq!(response.status, 200);
        assert_stream_closed(&mut replacement);
        assert_eq!(rpc(&info, &sessions[0], "ping", json!({})).status, 404);
        let start = Instant::now();
        drop(fixture.server.take());
        assert!(start.elapsed() < IO_TIMEOUT);
        assert!(!fixture.directory.join("shell-api.json").exists());
        // Drain already-sent notifications and verify shutdown closes every socket.
        for mut reader in streams {
            assert_stream_closed(&mut reader);
        }
    }

    #[test]
    fn cors_preflight_is_allowlisted_but_actual_calls_require_surface_credentials() {
        let fixture = Fixture::new();
        let server = fixture.server();
        let external = server.connection();
        let builtin = server.builtin_connection();
        let ui: ConnectionInfo = server.ui_connection(Surface::Shell).unwrap().into();
        for origin in [
            "http://asset.shell",
            "http://asset.pages",
            "https://asset.shell",
            "https://asset.pages",
            "asset://shell",
            "asset://pages",
            "null",
        ] {
            let extra = format!("Origin: {origin}\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: Authorization, Content-Type, Mcp-Session-Id\r\nAccess-Control-Request-Private-Network: true\r\n");
            let mut unauthenticated = external.clone();
            unauthenticated.token.clear();
            let preflight = send(&unauthenticated, "OPTIONS", None, Value::Null, &extra);
            assert_eq!(preflight.status, 204);
            assert_eq!(preflight.headers["access-control-allow-origin"], origin);
            assert_eq!(
                preflight.headers["access-control-allow-private-network"],
                "true"
            );
            assert!(preflight.headers["access-control-allow-headers"].contains("Mcp-Session-Id"));
            let mut initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}});
            for denied in [&external, &builtin] {
                assert_eq!(
                    send(
                        denied,
                        "POST",
                        None,
                        initialize.clone(),
                        &format!("Origin: {origin}\r\n")
                    )
                    .status,
                    403
                );
            }
            initialize["params"]["_meta"] = json!({"river.client_id":ui.client_id});
            let response = send(
                &ui,
                "POST",
                None,
                initialize,
                &format!("Origin: {origin}\r\n"),
            );
            assert_eq!(response.status, 200);
            assert_eq!(response.headers["access-control-allow-origin"], origin);
            assert!(response.headers["access-control-expose-headers"].contains("Mcp-Session-Id"));
        }
        assert_eq!(
            send(
                &ui,
                "OPTIONS",
                None,
                Value::Null,
                "Origin: https://evil.example\r\nAccess-Control-Request-Method: POST\r\n"
            )
            .status,
            403
        );
        assert_eq!(
            send(
                &ui,
                "POST",
                None,
                Value::Null,
                "Origin: https://evil.example\r\n"
            )
            .status,
            403
        );
        assert_eq!(send(&ui, "OPTIONS", None, Value::Null, "Origin: null\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: X-Unknown\r\n").status, 403);
    }

    #[test]
    fn https_asset_preflight_matches_webview2_headers_without_authentication() {
        let fixture = Fixture::new();
        let info = fixture.server().connection();
        let mut socket = socket(&info);
        let address = info
            .url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/mcp")
            .unwrap();
        write!(socket, "OPTIONS /mcp HTTP/1.1\r\nHost: {address}\r\nOrigin: https://asset.shell\r\nConnection: close\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: authorization,content-type,mcp-protocol-version,mcp-session-id\r\nAccess-Control-Request-Private-Network: true\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 204"));
        assert!(response.contains("Access-Control-Allow-Origin: https://asset.shell\r\n"));
        assert!(response.contains("Access-Control-Allow-Private-Network: true\r\n"));
        assert!(response
            .contains("Access-Control-Expose-Headers: Mcp-Session-Id, Mcp-Protocol-Version\r\n"));
    }

    #[test]
    fn sse_target_and_ui_only_events_do_not_notify_other_sessions() {
        let fixture = Fixture::new();
        let server = fixture.server();
        let external = server.connection();
        let shell: ConnectionInfo = server.ui_connection(Surface::Shell).unwrap().into();
        let sidebar: ConnectionInfo = server.ui_connection(Surface::Sidebar).unwrap().into();
        let mut clients: Vec<_> = [&external, &shell, &sidebar]
            .into_iter()
            .map(|info| {
                let session = initialize(info);
                subscribe_resource(info, &session, "shell://events");
                let mut reader = stream(info, &session);
                notification(&mut reader);
                reader
            })
            .collect();
        server.publish(Some(Surface::Shell), "ai.answer", json!({"text":"private"}));
        assert_eq!(
            notification(&mut clients[1])["params"]["uri"],
            "shell://events"
        );
        for index in [0, 2] {
            clients[index]
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(150)))
                .unwrap();
            let error = clients[index].read_line(&mut String::new()).unwrap_err();
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ));
            clients[index]
                .get_ref()
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
        }
        server.publish(None, "shell.toast", json!({"text":"UI only"}));
        notification(&mut clients[1]);
        notification(&mut clients[2]);
        clients[0]
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        let error = clients[0].read_line(&mut String::new()).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        clients[0]
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        server.publish(None, "tab.updated", json!({"id":9}));
        for client in &mut clients {
            notification(client);
        }
    }
}
