//! The NDJSON loop: frames in from the shell on stdin, frames out on stdout.
//!
//! stdout carries protocol frames only; everything else goes to stderr. Each
//! task runs on its own worker thread so the reader keeps servicing `task.cancel`
//! and `browser.event` while a model is thinking.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ai_platform_contract::contract::{
    CancelReason, CancelledAt, CancellationRequest, CancellationResult, TaskRequestView,
};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::{self, Frame, Method, Peer, ResponsePayload};
use ai_platform_contract::identity::DocumentToken;
use ai_platform_contract::model::BrowserEvent;

use crate::audit;
use crate::agent;
use crate::control::{lock, StopReason, TaskControl};
use crate::provider::Catalog;
use crate::shell::ShellLink;
use crate::tabs::Tabs;
use crate::task::{self, TaskEnv};

const MAX_CONCURRENT_TASKS: usize = 8;
/// An agent turn may run long (many tools); this is the backstop, the user can stop any time.
const AGENT_DEADLINE_MS: i64 = 30 * 60 * 1000;
const POLL: Duration = Duration::from_millis(50);

struct TaskEntry {
    control: Arc<TaskControl>,
    document: DocumentToken,
    sequence: u64,
}

struct Shared {
    credentials: HashMap<String, String>,
    out: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<u64, Sender<ResponsePayload>>>,
    next_id: AtomicU64,
    tasks: Mutex<HashMap<u64, TaskEntry>>,
    tabs: Tabs,
    config_path: PathBuf,
    agent_store: agent::store::Store,
    hub: agent::mcp::Hub,
    approvals: agent::Approvals,
    agent_runs: Mutex<HashMap<u64, Arc<TaskControl>>>,
}

impl Shared {
    fn write(&self, frame: &Frame) {
        let Ok(line) = frame::encode(frame) else { return };
        let mut out = lock(&self.out);
        // A closed pipe means the shell is gone; stdin EOF will end the loop.
        let _ = out.write_all(line.as_bytes());
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}

impl ShellLink for Shared {
    fn notify(&self, method: Method, params: &serde_json::Value) {
        self.write(&Frame::notification(method, params));
    }

    fn call(
        &self,
        method: Method,
        params: &serde_json::Value,
        control: &TaskControl,
    ) -> Result<ResponsePayload, Failure> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        lock(&self.pending).insert(id, tx);
        self.write(&Frame::request(id, method, params));
        loop {
            match rx.recv_timeout(POLL) {
                Ok(payload) => return Ok(payload),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::unavailable("shell.pipe_closed"));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(failure) = control.failure() {
                        lock(&self.pending).remove(&id);
                        return Err(failure);
                    }
                }
            }
        }
    }
}

fn request_id(line: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("kind")?.as_str()? != "request" {
        return None;
    }
    value.get("id")?.as_u64()
}

fn respond<T: serde::Serialize>(shared: &Shared, id: u64, value: &T) {
    let frame = match ResponsePayload::ok(value) {
        Ok(payload) => Frame::ok(id, payload),
        Err(failure) => Frame::failure(id, failure),
    };
    shared.write(&frame);
}

fn handle_complete(shared: &Arc<Shared>, workers: &mut Vec<JoinHandle<()>>, id: u64, params: serde_json::Value) {
    let view: TaskRequestView = match serde_json::from_value(params) {
        Ok(view) => view,
        Err(_) => return shared.write(&Frame::failure(id, Failure::unsupported("request.invalid"))),
    };
    if view.cancel.task != view.task {
        return shared.write(&Frame::failure(id, Failure::unsupported("request.cancel_token_mismatch")));
    }
    let (config, config_failure) = task::load_config(&shared.config_path);
    let control = Arc::new(TaskControl::new(task::effective_deadline(view.cancel.deadline_ms_since_epoch, &config)));
    {
        let mut tasks = lock(&shared.tasks);
        if tasks.contains_key(&view.task.0) {
            return shared.write(&Frame::failure(id, Failure::unsupported("task.duplicate")));
        }
        if tasks.len() >= MAX_CONCURRENT_TASKS {
            return shared.write(&Frame::failure(id, Failure::unavailable("utility.busy")));
        }
        tasks.insert(
            view.task.0,
            TaskEntry {
                control: Arc::clone(&control),
                document: view.document.document_token,
                sequence: view.document.navigation_sequence,
            },
        );
    }
    if !shared.tabs.claim(view.document.tab_id.0, view.document.document_token, view.document.navigation_sequence) {
        control.stop(StopReason::Stale("task.document_replaced"));
    }
    respond(shared, id, &serde_json::json!({ "accepted": true, "task": view.task }));

    let worker_shared = Arc::clone(shared);
    let spawned = std::thread::Builder::new().name(format!("task-{}", view.task.0)).spawn(move || {
        let env = TaskEnv { shell: &*worker_shared, tabs: &worker_shared.tabs, credentials: &worker_shared.credentials };
        task::run(&env, &view, &control, &config, config_failure);
        lock(&worker_shared.tasks).remove(&view.task.0);
    });
    match spawned {
        Ok(handle) => workers.push(handle),
        Err(_) => audit::line("task.spawn_failed", None, &[]),
    }
}

fn handle_agent_run(shared: &Arc<Shared>, workers: &mut Vec<JoinHandle<()>>, id: u64, params: serde_json::Value) {
    let Some(run_id) = params.get("run_id").and_then(serde_json::Value::as_u64) else {
        return shared.write(&Frame::failure(id, Failure::unsupported("request.invalid")));
    };
    let control = Arc::new(TaskControl::new(crate::control::now_ms() + AGENT_DEADLINE_MS));
    {
        let mut runs = lock(&shared.agent_runs);
        if runs.contains_key(&run_id) {
            return shared.write(&Frame::failure(id, Failure::unsupported("agent.duplicate_run")));
        }
        if runs.len() >= MAX_CONCURRENT_TASKS {
            return shared.write(&Frame::failure(id, Failure::unavailable("utility.busy")));
        }
        runs.insert(run_id, Arc::clone(&control));
    }
    respond(shared, id, &serde_json::json!({ "accepted": true, "run_id": run_id }));
    let worker = Arc::clone(shared);
    let spawned = std::thread::Builder::new().name(format!("agent-{run_id}")).spawn(move || {
        let (config, _) = task::load_config(&worker.config_path);
        let catalog = Catalog::from_config_with_credentials(&config, &worker.credentials);
        worker.hub.sync(&worker.agent_store.config().mcp_servers);
        let env = agent::Env {
            shell: &*worker,
            approvals: &worker.approvals,
            store: &worker.agent_store,
            catalog: &catalog,
            mcp: &worker.hub,
        };
        agent::run(&env, &params, &control);
        lock(&worker.agent_runs).remove(&run_id);
    });
    match spawned {
        Ok(handle) => workers.push(handle),
        Err(_) => audit::line("agent.spawn_failed", None, &[]),
    }
}

/// Cancel, approve, and the management operations. Management can block on a network
/// connect (MCP), so it runs on its own thread and the reader keeps going.
fn handle_agent_control(shared: &Arc<Shared>, workers: &mut Vec<JoinHandle<()>>, id: u64, params: serde_json::Value) {
    let op = params.get("op").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    let run = params.get("run_id").and_then(serde_json::Value::as_u64).unwrap_or(0);
    match op.as_str() {
        "cancel" => {
            let found = match lock(&shared.agent_runs).get(&run) {
                Some(control) => {
                    control.stop(StopReason::User(CancelReason::UserStopped));
                    true
                }
                None => false,
            };
            respond(shared, id, &serde_json::json!({ "cancelled": found }));
        }
        "approve" => {
            let call = params.get("call_id").and_then(serde_json::Value::as_str).unwrap_or("");
            let allow = params.get("allow").and_then(serde_json::Value::as_bool).unwrap_or(false);
            respond(shared, id, &serde_json::json!({ "delivered": shared.approvals.answer(run, call, allow) }));
        }
        _ => {
            let worker = Arc::clone(shared);
            let spawned = std::thread::Builder::new().name("agent-admin".into()).spawn(move || {
                let (config, _) = task::load_config(&worker.config_path);
                let catalog = Catalog::from_config_with_credentials(&config, &worker.credentials);
                match agent::admin::handle(&worker.agent_store, &worker.hub, &catalog, &params) {
                    Ok(value) => respond(&worker, id, &value),
                    Err(message) => worker.write(&Frame::failure(id, Failure::unsupported("agent.control_failed").with_detail(message))),
                }
            });
            match spawned {
                Ok(handle) => workers.push(handle),
                Err(_) => shared.write(&Frame::failure(id, Failure::unavailable("utility.busy"))),
            }
        }
    }
}

fn handle_cancel(shared: &Shared, id: u64, params: serde_json::Value) {
    let request: CancellationRequest = match serde_json::from_value(params) {
        Ok(request) => request,
        Err(_) => return shared.write(&Frame::failure(id, Failure::unsupported("request.invalid"))),
    };
    let result = match lock(&shared.tasks).get(&request.task.0) {
        None => CancellationResult {
            task: request.task,
            cancelled_at: CancelledAt::AlreadyFinished,
            actions_completed: 0,
            effect_unverified: false,
        },
        Some(entry) => {
            let in_flight = entry.control.action_in_flight();
            let completed = entry.control.actions_completed();
            entry.control.stop(StopReason::User(request.reason));
            let cancelled_at = if in_flight {
                CancelledAt::InFlightAction
            } else if completed > 0 {
                CancelledAt::BetweenActions
            } else {
                CancelledAt::BeforeFirstAction
            };
            CancellationResult {
                task: request.task,
                cancelled_at,
                actions_completed: completed,
                effect_unverified: in_flight,
            }
        }
    };
    audit::line("task.cancel", Some(request.task), &[("at", &format!("{:?}", result.cancelled_at))]);
    respond(shared, id, &result);
}

fn handle_event(shared: &Shared, params: serde_json::Value) {
    let Ok(event) = serde_json::from_value::<BrowserEvent>(params) else {
        audit::line("browser_event.invalid", None, &[]);
        return;
    };
    if let BrowserEvent::UserStopped { task } = &event {
        if let Some(entry) = lock(&shared.tasks).get(&task.0) {
            entry.control.stop(StopReason::User(CancelReason::UserStopped));
        }
        return;
    }
    let invalidated = shared.tabs.apply(&event);
    for entry in lock(&shared.tasks).values() {
        if invalidated.dead.contains(&entry.document) {
            entry.control.stop(StopReason::Stale("task.document_replaced"));
        }
        if invalidated.resequenced.iter().any(|(doc, seq)| *doc == entry.document && *seq != entry.sequence) {
            entry.control.stop(StopReason::Stale("task.history_state_changed"));
        }
    }
}

fn handle_frame(shared: &Arc<Shared>, workers: &mut Vec<JoinHandle<()>>, frame: Frame) {
    match frame {
        Frame::Response(response) => {
            if let Some(tx) = lock(&shared.pending).remove(&response.id) {
                let _ = tx.send(response.payload);
            }
        }
        Frame::Notification(note) => {
            if note.method == Method::BrowserEvent {
                handle_event(shared, note.params);
            }
        }
        Frame::Request(request) => match request.method {
            Method::CompleteModel => handle_complete(shared, workers, request.id, request.params),
            Method::CancelTask => handle_cancel(shared, request.id, request.params),
            Method::AgentRun => handle_agent_run(shared, workers, request.id, request.params),
            Method::AgentControl => handle_agent_control(shared, workers, request.id, request.params),
            Method::ListModels => {
                let (config, _) = task::load_config(&shared.config_path);
                respond(shared, request.id, &Catalog::from_config_with_credentials(&config, &shared.credentials).list());
            }
            Method::Ping => respond(shared, request.id, &"pong"),
            _ => shared.write(&Frame::failure(request.id, Failure::unsupported("method.not_served"))),
        },
    }
}

/// Serves frames until `input` ends, then stops every task and waits for the
/// workers (each still sends its one `task.finish`).
pub fn run(input: impl BufRead, output: Box<dyn Write + Send>, config_path: &Path) -> std::io::Result<()> {
    run_with_credentials(input, output, config_path, HashMap::new())
}

pub fn run_with_credentials(input: impl BufRead, output: Box<dyn Write + Send>, config_path: &Path,
    credentials: HashMap<String, String>) -> std::io::Result<()> {
    let shared = Arc::new(Shared {
        credentials,
        out: Mutex::new(output),
        pending: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(1),
        tasks: Mutex::new(HashMap::new()),
        tabs: Tabs::default(),
        config_path: config_path.to_path_buf(),
        agent_store: agent::store::Store::new(config_path.parent().unwrap_or(Path::new("."))),
        hub: agent::mcp::Hub::default(),
        approvals: agent::Approvals::default(),
        agent_runs: Mutex::new(HashMap::new()),
    });
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match frame::decode(&line, Peer::Shell) {
            Ok(frame) => handle_frame(&shared, &mut workers, frame),
            Err(failure) => {
                audit::line("frame.rejected", None, &[("code", &failure.code)]);
                if let Some(id) = request_id(&line) {
                    shared.write(&Frame::failure(id, failure));
                }
            }
        }
        workers.retain(|worker| !worker.is_finished());
    }
    for entry in lock(&shared.tasks).values() {
        entry.control.stop(StopReason::Shutdown);
    }
    for control in lock(&shared.agent_runs).values() {
        control.stop(StopReason::Shutdown);
    }
    lock(&shared.pending).clear();
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}
