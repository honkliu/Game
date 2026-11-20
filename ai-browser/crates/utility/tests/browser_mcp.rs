#![allow(clippy::unwrap_used, clippy::expect_used)]

mod tabs { pub use ai_platform_contract::identity::TabId; }
#[cfg_attr(windows, path = r"..\..\shell\src\ipc.rs")]
#[cfg_attr(not(windows), path = "../../shell/src/ipc.rs")]
#[allow(dead_code)]
mod ipc;

#[cfg_attr(windows, path = r"..\..\shell\src\api_server.rs")]
#[cfg_attr(not(windows), path = "../../shell/src/api_server.rs")]
mod api_server;

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use ai_platform_contract::contract::CancelReason;
use ai_platform_contract::failure::{Failure, FailureKind};
use ai_utility::browser::{Client, ConnectionInfo};
use ai_utility::control::{StopReason, TaskControl};
use api_server::{ApiRequest, ApiServer};
use serde_json::{json, Value};

static PROFILE_ID: AtomicU64 = AtomicU64::new(1);

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        Self(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-artifacts")
            .join(format!("mcp-{}-{}", std::process::id(), PROFILE_ID.fetch_add(1, Ordering::Relaxed))))
    }
}
impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn post(info: &api_server::ConnectionInfo, session: Option<&str>, body: Value) -> ureq::Response {
    let mut request = ureq::post(&info.url).set("Content-Type", "application/json")
        .set("Accept", "application/json, text/event-stream")
        .set("Authorization", &format!("Bearer {}", info.token)).timeout(Duration::from_secs(5));
    if let Some(session) = session { request = request.set("Mcp-Session-Id", session); }
    request.send_string(&body.to_string()).unwrap()
}

fn value(response: ureq::Response) -> Value {
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).unwrap();
    serde_json::from_str(&body).unwrap()
}

fn initialize(info: &api_server::ConnectionInfo) -> String {
    let response = post(info, None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}));
    let session = response.header("mcp-session-id").unwrap().to_string();
    assert_eq!(value(response)["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(post(info, Some(&session), json!({"jsonrpc":"2.0","method":"notifications/initialized"})).status(), 202);
    session
}

fn client(info: api_server::ConnectionInfo) -> Client {
    Client::new(ConnectionInfo { url: info.url, token: info.token }).unwrap()
}

#[test]
fn authenticated_discovery_sessions_canonical_calls_and_client_isolation() {
    let profile = Profile::new();
    let server = ApiServer::start(&profile.0, |request| {
        request.reply.send(Ok(json!({"caller":request.caller.id,"builtin":request.caller.builtin,"name":request.name,"args":request.args}))).unwrap();
        Ok(())
    }).unwrap();
    let info = server.connection();
    let descriptor: api_server::ConnectionInfo = serde_json::from_slice(&std::fs::read(profile.0.join("shell-api.json")).unwrap()).unwrap();
    assert_eq!(descriptor.url, info.url);
    assert_eq!(descriptor.token, info.token);
    assert_ne!(info.token, server.builtin_connection().token);
    assert!(matches!(ureq::post(&info.url).send_string("{}"), Err(ureq::Error::Status(401, _))));
    for (header, content) in [("Origin", "https://evil.example"), ("Host", "evil.example")] {
        assert!(matches!(ureq::post(&info.url).set("Authorization", &format!("Bearer {}",info.token)).set(header,content).send_string("{}"), Err(ureq::Error::Status(403, _))));
    }
    let session = initialize(&info);
    let list = value(post(&info, Some(&session), json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), ai_platform_contract::browser::definitions().len());
    assert!(tools.iter().all(|tool| tool["name"].as_str().unwrap().starts_with("shell_")));
    for (name, args) in [("open_tab", json!({"url":"https://example.test"})), ("shell_tab",json!({"operation":"new"})), ("shell_tab",json!({"operation":"list","_task":{}}))] {
        let response = value(post(&info, Some(&session), json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":args}})));
        assert_eq!(response["error"]["code"], -32602);
    }
    let control = TaskControl::new(0);
    let first = client(server.connection()).call("shell_tab", &json!({"operation":"list"}), &control).unwrap();
    let second = client(server.connection()).call("shell_tab", &json!({"operation":"list"}), &control).unwrap();
    let builtin = client(server.builtin_connection()).call("shell_tab", &json!({"operation":"list","_task":{}}), &control).unwrap();
    assert!(first["caller"].as_u64().unwrap() >= 2);
    assert_ne!(first["caller"], second["caller"]);
    assert_eq!(builtin["caller"], 1);
    assert_eq!(builtin["builtin"], true);
    assert_eq!(first["name"], "shell_tab");
    assert_eq!(first["args"]["operation"], "list");
    let old_url = info.url;
    let before = Instant::now();
    drop(server);
    assert!(before.elapsed() < Duration::from_secs(4));
    assert!(!profile.0.join("shell-api.json").exists());
    assert!(ureq::get(&old_url).timeout(Duration::from_secs(1)).call().is_err());
}

#[test]
fn typed_policy_failures_survive_the_shared_transport() {
    let profile = Profile::new();
    let server = ApiServer::start(&profile.0, |request| {
        request.reply.send(Ok(json!({"failure":Failure::blocked("action.approval_denied")}))).unwrap();
        Ok(())
    }).unwrap();
    let failure = client(server.builtin_connection()).call("shell_tab", &json!({"operation":"open","_task":{}}), &TaskControl::new(0)).unwrap_err();
    assert_eq!(failure.kind, FailureKind::Blocked);
    assert_eq!(failure.code, "action.approval_denied");
}

#[test]
fn parallel_calls_cancel_without_replay_and_shutdown_unblocks_native_waits() {
    let profile = Profile::new();
    let (sent, received) = mpsc::channel::<ApiRequest>();
    let server = ApiServer::start(&profile.0, move |request| sent.send(request).map_err(|_| "closed".into())).unwrap();
    let client = client(server.builtin_connection());
    let control = Arc::new(TaskControl::new(0));
    let (first_client, first_control) = (client.clone(), Arc::clone(&control));
    let first = std::thread::spawn(move || first_client.call("shell_tab", &json!({"operation":"open","url":"https://example.test"}), &first_control));
    let pending = received.recv_timeout(Duration::from_secs(5)).unwrap();
    let second_client = client.clone();
    let second = std::thread::spawn(move || second_client.call("shell_tab", &json!({"operation":"list"}), &TaskControl::new(0)));
    let independent = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(independent.args["operation"], "list");
    independent.reply.send(Ok(json!({"tabs":[]}))).unwrap();
    assert_eq!(second.join().unwrap().unwrap(), json!({"tabs":[]}));
    let before = Instant::now();
    control.stop(StopReason::User(CancelReason::UserStopped));
    assert_eq!(first.join().unwrap().unwrap_err().kind, FailureKind::Cancelled);
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(received.recv_timeout(Duration::from_millis(100)).is_err(), "a cancelled mutation must not replay");
    let before = Instant::now();
    drop(server);
    assert!(before.elapsed() < Duration::from_secs(4));
    drop(pending);
}
