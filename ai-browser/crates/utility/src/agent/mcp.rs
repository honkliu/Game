//! MCP (Model Context Protocol) client: tools from external servers become agent tools.
//!
//! Two transports, both JSON-RPC 2.0: streamable HTTP (`url`, optional bearer token)
//! and stdio (`command` + `args`, newline-delimited messages). Servers are connected
//! lazily, their `tools/list` is cached, and a tool is namespaced `mcp_<server>__<tool>`.
//! A tool not marked `readOnlyHint` asks the user before it runs unless the server is
//! trusted.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::store::McpServerConfig;
use super::tools::{Kind, ToolDef};
use crate::control::{lock, TaskControl};

const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESULT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
struct McpTool {
    name: String,
    description: String,
    schema: Value,
    read_only: bool,
}

enum Conn {
    Http { agent: ureq::Agent, url: String, token: Option<String>, session: Option<String> },
    Stdio { child: Child, stdin: ChildStdin, lines: Receiver<String> },
}

struct Server {
    config: McpServerConfig,
    conn: Option<Conn>,
    tools: Vec<McpTool>,
    status: &'static str,
    error: String,
    next_id: u64,
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(Conn::Stdio { child, .. }) = self.conn.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Default)]
pub struct Hub {
    servers: Mutex<HashMap<String, Arc<Mutex<Server>>>>,
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

fn tool_name(server: &str, tool: &str) -> String {
    let mut name = format!("mcp_{}__{}", safe_name(server), safe_name(tool));
    name.truncate(64);
    name
}

fn http_rpc(agent: &ureq::Agent, url: &str, token: &Option<String>, session: &mut Option<String>, body: &Value, want: Option<u64>) -> Result<Value, String> {
    let mut request = agent
        .post(url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    if let Some(session) = session.as_deref() {
        request = request.set("Mcp-Session-Id", session);
    }
    let response = match request.send_string(&body.to_string()) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(format!("server answered HTTP {code}")),
        Err(ureq::Error::Transport(t)) => return Err(format!("could not connect ({:?})", t.kind())),
    };
    if let Some(id) = response.header("mcp-session-id") {
        *session = Some(id.to_string());
    }
    let Some(want) = want else { return Ok(Value::Null) };
    let streaming = response.content_type().contains("event-stream");
    let mut text = String::new();
    response.into_reader().take(MAX_RESULT as u64).read_to_string(&mut text).map_err(|e| format!("read failed: {e}"))?;
    let messages: Vec<Value> = if streaming {
        text.lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str(d.trim()).ok())
            .collect()
    } else {
        vec![serde_json::from_str(&text).map_err(|_| "reply was not JSON".to_string())?]
    };
    messages
        .into_iter()
        .flat_map(|m| if let Value::Array(a) = m { a } else { vec![m] })
        .find(|m| m.get("id").and_then(Value::as_u64) == Some(want))
        .ok_or_else(|| "no reply to the request".to_string())
}

fn stdio_rpc(stdin: &mut ChildStdin, lines: &Receiver<String>, body: &Value, want: Option<u64>, timeout: Duration, control: Option<&TaskControl>) -> Result<Value, String> {
    writeln!(stdin, "{body}").and_then(|_| stdin.flush()).map_err(|e| format!("server pipe closed: {e}"))?;
    let Some(want) = want else { return Ok(Value::Null) };
    let end = Instant::now() + timeout;
    loop {
        if let Some(f) = control.and_then(TaskControl::failure) {
            return Err(f.code);
        }
        match lines.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if v.get("id").and_then(Value::as_u64) == Some(want) {
                        return Ok(v);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if Instant::now() > end {
                    return Err("timed out waiting for the server".into());
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Err("server exited".into()),
        }
    }
}

impl Server {
    fn rpc(&mut self, method: &str, params: Value, timeout: Duration, control: Option<&TaskControl>) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let reply = match self.conn.as_mut().ok_or("not connected")? {
            Conn::Http { agent, url, token, session } => http_rpc(agent, url, token, session, &body, Some(id))?,
            Conn::Stdio { stdin, lines, .. } => stdio_rpc(stdin, lines, &body, Some(id), timeout, control)?,
        };
        if let Some(err) = reply.get("error") {
            return Err(err.get("message").and_then(Value::as_str).unwrap_or("server error").to_string());
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&mut self, method: &str) {
        let body = json!({ "jsonrpc": "2.0", "method": method });
        match self.conn.as_mut() {
            Some(Conn::Http { agent, url, token, session }) => {
                let _ = http_rpc(agent, url, token, session, &body, None);
            }
            Some(Conn::Stdio { stdin, lines, .. }) => {
                let _ = stdio_rpc(stdin, lines, &body, None, Duration::ZERO, None);
            }
            None => {}
        }
    }

    fn open(&mut self) -> Result<(), String> {
        let config = self.config.clone();
        self.conn = Some(if let Some(url) = config.url.as_deref() {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("the MCP URL must be http(s)".into());
            }
            let agent = ureq::AgentBuilder::new()
                .redirects(0)
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(CALL_TIMEOUT)
                .user_agent("ai-browser-mcp")
                .build();
            Conn::Http { agent, url: url.to_string(), token: config.token.clone().filter(|t| !t.is_empty()), session: None }
        } else if let Some(command) = config.command.as_deref().filter(|c| !c.trim().is_empty()) {
            let mut cmd = if cfg!(windows) && !command.contains(['\\', '/']) {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(command);
                c
            } else {
                Command::new(command)
            };
            cmd.args(&config.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            let mut child = cmd.spawn().map_err(|e| format!("could not start `{command}`: {e}"))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            Conn::Stdio { child, stdin, lines: rx }
        } else {
            return Err("the server needs a url or a command".into());
        });
        self.rpc(
            "initialize",
            json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "ai-browser", "version": "0.1" } }),
            INIT_TIMEOUT,
            None,
        )?;
        self.notify("notifications/initialized");
        let list = self.rpc("tools/list", json!({}), LIST_TIMEOUT, None)?;
        self.tools = list
            .get("tools")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|t| {
                        Some(McpTool {
                            name: t.get("name")?.as_str()?.to_string(),
                            description: t.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                            schema: t.get("inputSchema").cloned().filter(Value::is_object).unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                            read_only: t.pointer("/annotations/readOnlyHint").and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(())
    }

    fn connect(&mut self) {
        match self.open() {
            Ok(()) => {
                self.status = "connected";
                self.error.clear();
            }
            Err(e) => {
                self.conn = None;
                self.tools.clear();
                self.status = "error";
                self.error = e;
            }
        }
    }
}

impl Hub {
    /// Bring the connected set in line with the configuration. Unchanged servers keep
    /// their connection; new or edited ones are (re)connected; removed ones are dropped.
    pub fn sync(&self, wanted: &[McpServerConfig]) {
        let mut servers = lock(&self.servers);
        servers.retain(|id, s| wanted.iter().any(|w| &w.id == id && w.enabled) && lock(s).config.id == *id);
        let mut fresh: Vec<Arc<Mutex<Server>>> = Vec::new();
        for config in wanted.iter().filter(|c| c.enabled) {
            let keep = servers.get(&config.id).is_some_and(|s| lock(s).config == *config);
            if keep {
                continue;
            }
            let server = Arc::new(Mutex::new(Server {
                config: config.clone(),
                conn: None,
                tools: Vec::new(),
                status: "connecting",
                error: String::new(),
                next_id: 0,
            }));
            servers.insert(config.id.clone(), Arc::clone(&server));
            fresh.push(server);
        }
        drop(servers);
        // Connect in parallel so a slow server does not delay the others.
        std::thread::scope(|scope| {
            for server in &fresh {
                scope.spawn(move || lock(server).connect());
            }
        });
    }

    /// Reconnect one server now (the UI's "retry").
    pub fn reconnect(&self, id: &str) {
        let server = lock(&self.servers).get(id).cloned();
        if let Some(server) = server {
            lock(&server).connect();
        }
    }

    pub fn tools(&self) -> Vec<ToolDef> {
        let servers = lock(&self.servers);
        let mut ids: Vec<&String> = servers.keys().collect();
        ids.sort();
        let mut out = Vec::new();
        for id in ids {
            let server = lock(&servers[id]);
            for t in &server.tools {
                out.push(ToolDef {
                    name: tool_name(id, &t.name),
                    description: format!("[{}] {}", server.config.name, t.description),
                    parameters: t.schema.clone(),
                    kind: Kind::Mcp { server: id.clone(), tool: t.name.clone() },
                    write: !t.read_only,
                    trusted: server.config.trust,
                });
            }
        }
        out
    }

    pub fn call(&self, server: &str, tool: &str, args: &Value, control: &TaskControl) -> Result<Value, String> {
        let entry = lock(&self.servers).get(server).cloned().ok_or("that MCP server is not connected")?;
        let mut s = lock(&entry);
        if s.conn.is_none() {
            s.connect();
        }
        let result = s.rpc("tools/call", json!({ "name": tool, "arguments": args }), CALL_TIMEOUT, Some(control))?;
        let text: String = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .map(|p| match p.get("type").and_then(Value::as_str) {
                        Some("text") => p.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                        Some(other) => format!("[{other} content omitted]"),
                        None => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|| result.to_string());
        if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            return Err(text);
        }
        Ok(Value::String(text))
    }

    pub fn status(&self, configured: &[McpServerConfig]) -> Vec<Value> {
        let servers = lock(&self.servers);
        configured
            .iter()
            .map(|c| match servers.get(&c.id) {
                Some(s) => {
                    let s = lock(s);
                    json!({
                        "id": c.id, "name": c.name, "enabled": c.enabled, "trust": c.trust,
                        "url": c.url, "command": c.command, "args": c.args, "has_token": c.token.is_some(),
                        "status": s.status, "error": s.error,
                        "tools": s.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "read_only": t.read_only })).collect::<Vec<_>>(),
                    })
                }
                None => json!({
                    "id": c.id, "name": c.name, "enabled": c.enabled, "trust": c.trust,
                    "url": c.url, "command": c.command, "args": c.args, "has_token": c.token.is_some(),
                    "status": if c.enabled { "disconnected" } else { "disabled" }, "error": "", "tools": [],
                }),
            })
            .collect()
    }
}
