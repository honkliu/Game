//! The shell's half of the AI conversation: everything that touches a real tab.
//!
//! The rules live in `crate::ai::session` (what may run, what an answer must prove);
//! this file carries them out. It is the bidirectional part of the design:
//!
//! * browser → AI: a question with its page capture (`model.complete`), a stop
//!   (`task.cancel`), and a stream of content-free `browser.event`s (tab switched,
//!   navigation committed, selection changed, tab closed, config changed);
//! * AI → browser: `context.get_snapshot` (read any readable tab), `action.dispatch`
//!   (scroll, open a candidate tab, switch tab, close a tab it opened, list tabs),
//!   progress and the final answer.
//!
//! Every inbound frame is untrusted. An action is admitted by `AiSession::admit_action`
//! before anything runs; an answer is verified against live identity and against the
//! text that was actually captured before it is shown with citations.

use serde_json::{json, Value};

use ai_platform_contract::config::AiConfig;
use ai_platform_contract::contract::{
    ActionDecision, ActionLevel, ActionRequest, ActionResult, AnswerWithCitations, CancelReason,
    CancellationRequest, CancellationToken, ContextSnapshotRequest, TaskOutcome, TaskRequest,
};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::{Frame, Method, ResponsePayload};
use ai_platform_contract::identity::{DocumentToken, SnapshotVersion, TaskId};
use ai_platform_contract::model::{
    BrowserEvent, ChatTurn, RoutePreference, SelectionQuote, TaskEvent, TaskFailed, TaskIntent, TaskOptions,
};

use super::{AppMessage, AppState};
use crate::ai::answer::failure_message;
use crate::ai::context::{self, PageContext};
use crate::ai::link::Link;
use crate::ai::provider::ProviderInfo;
use crate::ai::session::{host_of, now_ms, snapshot_of, Pending, Selection, TaskState};
use crate::layout::{self, Overlay, Panel};
use crate::tabs::TabId;

/// Selection toolbar and inline card sizes, logical units.
pub const TOOLBAR_W: i32 = 360;
pub const TOOLBAR_H: i32 = 40;
pub const CARD_W: i32 = 460;
pub const CARD_H: i32 = 340;

/// Slack after a task's deadline before the shell gives up on the utility.
const DEADLINE_GRACE_MS: i64 = 5_000;

/// Who asked for a capture.
#[derive(Debug, Clone, Copy)]
pub enum CaptureFor {
    /// The page of this task, taken when the question was asked.
    Task(u64),
    /// A utility `context.get_snapshot` for another tab.
    Snapshot { request: u64, token: u64, version: u64 },
    /// Another tab the user chose to include in this task.
    Extra(u64),
}

/// A page-script request (from the `window.open('')` stub) to open an http(s) URL in a new tab.
fn open_request(raw: &str) -> Option<String> {
    if raw.len() > 8 * 1024 {
        return None;
    }
    let v: Value = serde_json::from_str(raw).ok()?;
    if v.get("ai")?.as_str()? != "open" {
        return None;
    }
    let url = v.get("url")?.as_str()?;
    (url.starts_with("http://") || url.starts_with("https://")).then(|| url.to_string())
}

pub(super) fn is_readable_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || (url.starts_with("file:///") && crate::pdf::is_pdf_url(url))
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The flash/scroll script. Numbers only: nothing page- or model-derived is spliced
/// into script text.
fn scroll_script(y: f64, rect: Option<[f64; 4]>) -> String {
    let flash = match rect {
        Some([x, ry, w, h]) => format!("window.__aiFlash&&window.__aiFlash({x},{ry},{w},{h});"),
        None => String::new(),
    };
    format!("try{{window.scrollTo({{top:{y},behavior:'smooth'}});{flash}}}catch(e){{}}")
}

impl AppState {
    // ---- link ----------------------------------------------------------

    fn ai_config_path(&self) -> std::path::PathBuf {
        self.data_dir.join("ai.json")
    }

    /// Re-read `ai.json`, so a kill switch takes effect on the next question
    /// without a restart. A changed config is announced to the utility.
    fn ai_reload_config(&mut self) {
        let path = self.ai_config_path();
        let (config, problem) = AiConfig::load(&path);
        if !path.exists() {
            let _ = config.save(&path);
        }
        self.ai.link_error = problem.map(|f| failure_message(&f));
        if config != self.ai.config {
            self.ai.config = config;
            self.ai.provider_view = None;
            self.ai_notify(BrowserEvent::ConfigChanged);
            self.ai.models.clear();
        }
    }

    pub(super) fn ai_ensure_link(&mut self) -> Result<(), Failure> {
        self.ai_reload_config();
        self.ai.availability()?;
        if let Some(link) = self.ai.link.as_mut() {
            if link.alive() {
                return Ok(());
            }
            self.ai.link = None;
        }
        let proxy_frame = self.proxy();
        let proxy_closed = self.proxy();
        let credentials = self.ai_key_env().map_err(|error|
            Failure::unavailable("credentials.unavailable").with_detail(error))?;
        self.ai.provider_view = None;
        self.ai.link_generation += 1;
        let generation = self.ai.link_generation;
        let link = Link::spawn(
            &self.ai_config_path(),
            &credentials,
            move |frame| {
                let _ = proxy_frame.send_event(AppMessage::AiFrame(generation, frame));
            },
            move |reason| {
                let _ = proxy_closed.send_event(AppMessage::AiLinkClosed(generation, reason));
            },
        )?;
        self.note("ai engine started (in-process)".to_string());
        self.ai.link = Some(link);
        Ok(())
    }

    pub(super) fn ai_send(&mut self, frame: Frame) {
        let result = match self.ai.link.as_ref() {
            Some(link) => link.send(&frame),
            None => Err(Failure::unavailable("utility.not_running")),
        };
        if let Err(f) = result {
            self.note(format!("ai send: {}", failure_message(&f)));
        }
    }

    /// Tell the utility something happened in the browser. No link, no audience.
    pub(super) fn ai_notify(&mut self, event: BrowserEvent) {
        if self.ai.link.is_none() {
            return;
        }
        match event.to_params() {
            Ok(params) => self.ai_send(Frame::notification(Method::BrowserEvent, &params)),
            Err(f) => self.note(format!("ai event: {}", failure_message(&f))),
        }
    }

    pub(super) fn ai_request(&mut self, method: Method, params: &Value, what: Pending) -> Result<(), Failure> {
        let id = {
            let link = self.ai.link.as_mut().ok_or_else(|| Failure::unavailable("utility.not_running"))?;
            link.next_id()
        };
        self.ai.pending.insert(id, what);
        let frame = Frame::request(id, method, params);
        self.ai.link.as_ref().ok_or_else(|| Failure::unavailable("utility.not_running"))?.send(&frame)
    }

    // ---- browser → AI --------------------------------------------------

    /// `ai.ask`: begin a task about the active tab.
    pub(super) fn ai_ask(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let question = p.get("question").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let intent: TaskIntent = p
            .get("intent")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_value(Value::String(s.to_string())).ok())
            .unwrap_or_default();
        let use_selection = p.get("selection").and_then(Value::as_bool).unwrap_or(false);
        if question.is_empty() && !use_selection && matches!(intent, TaskIntent::Ask | TaskIntent::Act) {
            return Err("type a question first".into());
        }
        self.ai_ensure_link().map_err(|f| failure_message(&f))?;

        let tab = self.active.ok_or("there is no active tab")?;
        // A tab that is not a readable page (new tab, settings, an error page) still gets
        // a conversation: the task is bound to an empty stand-in document.
        let page_free = self.ai.identity.current(tab.0).is_none();
        let key = match self.ai.identity.current(tab.0) {
            Some(key) => key,
            None => self
                .ai
                .identity
                .on_document_committed(tab.0, "https://assistant.invalid/")
                .map_err(|f| failure_message(&f))?,
        };
        if !page_free && !self.live.contains_key(&tab) {
            return Err("the page is not loaded yet".into());
        }
        let token = key.document_token();
        let host = self.ai.identity.url(token).and_then(host_of).unwrap_or_default();

        let selection = if use_selection {
            let sel = self.ai.selections.get(&tab.0).ok_or("nothing is selected on the page")?;
            Some(SelectionQuote { exact: sel.text.clone(), prefix: None, suffix: None })
        } else {
            None
        };
        let question = if question.is_empty() {
            match intent {
                TaskIntent::Explain => "Explain the selected text.",
                TaskIntent::Verify => "Is the selected text accurate? Check it against the page.",
                TaskIntent::Translate => "Translate the selected text.",
                TaskIntent::Rewrite => "Rewrite the selected text more clearly.",
                TaskIntent::Summarize => "Summarize this page.",
                _ => "What is this about?",
            }
            .to_string()
        } else {
            question
        };

        let route = match p.get("model").and_then(Value::as_str) {
            Some(m) if m.contains('/') => {
                let (provider_id, model_id) = m.split_once('/').unwrap_or(("", m));
                RoutePreference::Model {
                    provider_id: provider_id.to_string(),
                    model_id: model_id.to_string(),
                    allow_fallback: false,
                }
            }
            _ if p.get("local_only").and_then(Value::as_bool).unwrap_or(false) => RoutePreference::LocalOnly,
            _ => RoutePreference::Auto,
        };
        let options = TaskOptions {
            intent,
            selection,
            route,
            force_local: self.ai.config.domains.denies_cloud(&host) || !self.ai.config.flags.cloud_models_enabled,
            target_language: p.get("target_language").and_then(Value::as_str).map(str::to_string),
            history: self.ai.history.clone(),
        };

        let extra_tabs = self.ai_extra_tabs(p, tab);
        let task = self.ai.identity.create_task().map_err(|f| failure_message(&f))?;
        let version = self.ai.identity.snapshot_captured(token).map_err(|f| failure_message(&f))?;
        let turn_id = self.ai.answers.begin(task, tab.0, &question, version, ProviderInfo::pending());
        let ceiling = if intent == TaskIntent::Act && self.ai.config.max_level() >= ActionLevel::Navigate {
            ActionLevel::Navigate
        } else {
            ActionLevel::Observe
        };
        let deadline = now_ms() + self.ai.config.task_deadline_ms as i64;
        self.ai.tasks.insert(
            task.0,
            TaskState {
                turn_id,
                tab: tab.0,
                document: key,
                snapshot: version,
                started: std::time::Instant::now(),
                ceiling,
                deadline_ms: deadline,
                opened_tabs: Vec::new(),
                text: String::new(),
                next_delta: 0,
                model: String::new(),
                local: true,
                stopped: false,
                question: question.clone(),
                options,
                extra_tabs,
                primary: None,
                merged: Vec::new(),
                extra_attempts: 0,
            },
        );

        // The capture is asynchronous; the reply goes out now so the UI can show the
        // question and a pending card immediately.
        let proxy = self.proxy();
        let started = if page_free {
            self.ai.contexts.put(PageContext {
                document: token,
                snapshot_version: version,
                navigation_sequence: key.navigation_sequence(),
                url: String::new(),
                title: String::new(),
                blocks: Vec::new(),
                links: Vec::new(),
                frames_not_read: 0,
                dropped_blocks: 0,
            });
            self.ai_start_task(task.0);
            Ok(())
        } else {
            match self.live.get(&tab) {
                Some(live) => context::capture(&live.webview, move |result| {
                    let _ = proxy.send_event(AppMessage::AiCaptured(tab, CaptureFor::Task(task.0), result));
                }),
                None => Err(Failure::unavailable("context.no_webview")),
            }
        };
        if let Err(f) = started {
            self.ai_finish_failed(task.0, f);
        }
        let payload = self.ai.answers.payload(turn_id);
        self.emit("ai.answer", payload);
        Ok(Some(json!({ "accepted": true, "turn_id": turn_id, "task_id": task.0 })))
    }

    /// A capture finished; parse it and either hand the task to the utility or
    /// answer a pending `get_snapshot`.
    pub(super) fn ai_on_captured(&mut self, tab: TabId, what: CaptureFor, result: Result<String, String>) {
        if let CaptureFor::Extra(task) = what {
            return self.ai_on_extra_captured(tab, task, result);
        }
        let (token, version, seq) = match what {
            CaptureFor::Task(task) => {
                let Some(state) = self.ai.tasks.get(&task) else { return };
                (state.document.document_token(), state.snapshot, state.document.navigation_sequence())
            }
            CaptureFor::Extra(_) => return,
            CaptureFor::Snapshot { token, version, .. } => {
                let token = DocumentToken(token);
                let Some(scope) = self.ai.identity.scope() else { return };
                match self.ai.identity.registry().resolve(token, &scope) {
                    Ok(key) => (token, SnapshotVersion(version), key.navigation_sequence()),
                    Err(f) => return self.ai_snapshot_failed(what, f),
                }
            }
        };
        let parsed = result
            .map_err(|e| Failure::unavailable("context.capture_failed").with_detail(e))
            .and_then(|text| {
                serde_json::from_str::<Value>(&text)
                    .map_err(|e| Failure::unavailable("context.snapshot_unreadable").with_detail(e.to_string()))
            })
            .and_then(|payload| {
                let (url, title) = self
                    .tabs
                    .get(tab)
                    .map(|t| (t.url.clone(), t.display_title()))
                    .unwrap_or_default();
                context::parse(&payload, token, version, seq, &url, &title)
            });
        // The page may have been replaced while the capture was in flight.
        let still_current = self.ai.identity.current(tab.0).map(|k| k.document_token()) == Some(token);
        let ctx = match parsed {
            Ok(_) if !still_current => Err(Failure::stale("context.document_replaced_during_capture")),
            other => other,
        };
        match (what, ctx) {
            (CaptureFor::Task(task), Ok(ctx)) => {
                let first = self.ai.tasks.get(&task).and_then(|s| s.extra_tabs.first().copied());
                match first {
                    Some(extra) => {
                        if let Some(s) = self.ai.tasks.get_mut(&task) {
                            s.primary = Some(ctx);
                        }
                        self.ai_capture_extra(task, TabId(extra));
                    }
                    None => {
                        self.ai.contexts.put(ctx);
                        self.ai_start_task(task);
                    }
                }
            }
            (CaptureFor::Extra(_), _) => {}
            (CaptureFor::Task(task), Err(f)) => self.ai_finish_failed(task, f),
            (CaptureFor::Snapshot { request, .. }, Ok(ctx)) => {
                let title = ctx.title.clone();
                self.ai.contexts.put(ctx);
                self.ai_reply_snapshot(request, token, &title);
            }
            (what @ CaptureFor::Snapshot { .. }, Err(f)) => self.ai_snapshot_failed(what, f),
        }
    }

    fn ai_start_task(&mut self, task: u64) {
        let Some(state) = self.ai.tasks.get(&task) else { return };
        let request = TaskRequest {
            task: TaskId(task),
            question: state.question.clone(),
            document: state.document,
            snapshot_version: state.snapshot,
            action_ceiling: state.ceiling,
            cancel: CancellationToken { task: TaskId(task), deadline_ms_since_epoch: state.deadline_ms },
            options: state.options.clone(),
        };
        let sent = request
            .to_params()
            .and_then(|params| self.ai_request(Method::CompleteModel, &params, Pending::Complete(task)));
        if let Err(f) = sent {
            self.ai_finish_failed(task, f);
        }
    }

    /// `ai.stop`: the user pressed stop. The effect is reported as the utility
    /// states it, and the local turn is closed either way.
    pub(super) fn ai_stop(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let turn = p.get("turn_id").and_then(Value::as_u64);
        let task = self
            .ai
            .tasks
            .iter()
            .find(|(_, t)| turn.is_none_or(|id| t.turn_id == id))
            .map(|(id, _)| *id)
            .ok_or("nothing is running")?;
        if let Some(t) = self.ai.tasks.get_mut(&task) {
            t.stopped = true;
        }
        self.ai_notify(BrowserEvent::UserStopped { task: TaskId(task) });
        let params = serde_json::to_value(CancellationRequest { task: TaskId(task), reason: CancelReason::UserStopped })
            .map_err(|e| e.to_string())?;
        if self.ai_request(Method::CancelTask, &params, Pending::Cancel(task)).is_err() {
            // No utility to stop: close the turn here.
            self.ai_finish_failed(task, Failure::cancelled("task.user_stopped"));
        }
        Ok(Some(json!({ "stopping": true, "task_id": task })))
    }

    // ---- AI → browser --------------------------------------------------

    pub(super) fn ai_on_frame(&mut self, frame: Frame) {
        match frame {
            Frame::Request(request) => match request.method {
                Method::GetSnapshot => self.ai_on_get_snapshot(request.id, request.params),
                Method::DispatchAction => self.ai_on_action(request.id, request.params),
                Method::AgentTool => self.agent_on_tool(request.id, request.params),
                Method::Ping => {
                    if let Ok(payload) = ResponsePayload::ok(&json!({ "pong": true })) {
                        self.ai_send(Frame::ok(request.id, payload));
                    }
                }
                other => self.ai_send(Frame::failure(
                    request.id,
                    Failure::unsupported("protocol.method_not_served_by_shell").with_detail(other.as_str()),
                )),
            },
            Frame::Notification(note) => match note.method {
                Method::PublishProgress => self.ai_on_progress(note.params),
                Method::FinishTask => self.ai_on_finish(note.params),
                Method::AgentEvent => self.agent_on_event(note.params),
                other => self.note(format!("ai: ignored notification {}", other.as_str())),
            },
            Frame::Response(response) => {
                let Some(what) = self.ai.pending.remove(&response.id) else {
                    self.note(format!("ai: response {} matches no request", response.id));
                    return;
                };
                self.ai_on_response(what, response.payload);
            }
        }
    }

    fn ai_on_response(&mut self, what: Pending, payload: ResponsePayload) {
        match (what, payload) {
            (Pending::Complete(task), ResponsePayload::Failure(f)) => self.ai_finish_failed(task, f),
            (Pending::Complete(_), ResponsePayload::Ok(_)) => {}
            (Pending::Cancel(task), _) => {
                // The utility normally finishes the task itself; if it has not, the
                // user still gets a closed turn rather than a spinner.
                if self.ai.tasks.contains_key(&task) {
                    self.ai_finish_failed(task, Failure::cancelled("task.user_stopped"));
                }
            }
            (Pending::Models, ResponsePayload::Ok(value)) => {
                self.ai.models = value.as_array().cloned().unwrap_or_default();
                let models = self.ai.models.clone();
                self.emit("ai.models", json!({ "models": models }));
            }
            (Pending::Models, ResponsePayload::Failure(f)) => {
                self.note(format!("model.list: {}", failure_message(&f)))
            }
            (Pending::AgentRun(run), ResponsePayload::Failure(f)) => self.agent_run_failed(run, &f),
            (Pending::AgentRun(_), ResponsePayload::Ok(_)) => {}
            (Pending::Control(source, req), payload) => self.agent_control_reply(source, req, payload),
        }
    }

    pub(super) fn ai_on_link_closed(&mut self, reason: String) {
        self.note(format!("ai utility closed: {reason}"));
        self.ai.link = None;
        self.ai.pending.clear();
        let tasks: Vec<u64> = self.ai.tasks.keys().copied().collect();
        for task in tasks {
            self.ai_finish_failed(
                task,
                Failure::unavailable("utility.exited").with_detail("the AI process stopped; the browser is unaffected"),
            );
        }
        self.emit("ai.link", json!({ "state": "down", "reason": reason }));
        self.agent_link_closed();
    }

    fn ai_on_progress(&mut self, params: Value) {
        let Ok(event) = serde_json::from_value::<TaskEvent>(params) else { return };
        let task = event.task().0;
        let Some(state) = self.ai.tasks.get_mut(&task) else { return };
        let turn_id = state.turn_id;
        match event {
            TaskEvent::Delta { seq, text, .. } => {
                state.next_delta = seq + 1;
                if state.text.len() > 64 * 1024 {
                    return;
                }
                state.text.push_str(&text);
                self.emit("ai.delta", json!({ "turn_id": turn_id, "seq": seq, "text": text }));
            }
            TaskEvent::Routed(decision) => {
                let local = decision.local_execution;
                let model = decision
                    .chosen
                    .as_ref()
                    .map(|m| format!("{}/{}", m.provider_id, m.model_id))
                    .unwrap_or_default();
                state.model = model.clone();
                state.local = local;
                self.ai.answers.set_provider(turn_id, ProviderInfo::from_route(&model, local));
                self.emit(
                    "ai.stage",
                    json!({ "turn_id": turn_id, "stage": "routed", "model": model, "local": local,
                            "route_class": decision.route_class }),
                );
            }
            TaskEvent::Stage(progress) => {
                let stage = serde_json::to_value(progress.stage).unwrap_or(Value::Null);
                self.emit(
                    "ai.stage",
                    json!({ "turn_id": turn_id, "stage": stage, "completed": progress.completed_steps,
                            "total": progress.total_steps }),
                );
            }
        }
    }

    fn ai_on_finish(&mut self, params: Value) {
        let outcome: TaskOutcome = match serde_json::from_value(params) {
            Ok(o) => o,
            Err(e) => {
                self.note(format!("ai: unreadable task.finish: {e}"));
                return;
            }
        };
        let task = outcome.task().0;
        if !self.ai.tasks.contains_key(&task) {
            self.note(format!("ai: task.finish for unknown task {task}"));
            return;
        }
        let outcome = match outcome {
            TaskOutcome::Answered(mut answer) => match self.ai_verify_answer(task, &mut answer) {
                Ok(()) => {
                    self.ai_remember_turn(task, &answer.answer);
                    TaskOutcome::Answered(answer)
                }
                Err(failure) => TaskOutcome::Failed(TaskFailed { task: TaskId(task), failure }),
            },
            other => other,
        };
        self.ai_complete(task, outcome);
    }

    /// The shell's own check of an answer before it is shown with citations: the
    /// contract's identity check, and then that each quote is really in the text
    /// that was captured. Neither is delegated to the utility.
    fn ai_verify_answer(&self, task: u64, answer: &mut AnswerWithCitations) -> Result<(), Failure> {
        if answer.task.0 != task {
            return Err(Failure::blocked("answer.task_mismatch"));
        }
        let scope = self
            .ai
            .identity
            .scope()
            .ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        answer.verify(self.ai.identity.registry(), &scope)?;
        // A citation the browser cannot confirm is dropped rather than shown; the answer
        // text is the model's own and is kept, so one bad quote never costs the whole reply.
        let contexts = &self.ai.contexts;
        answer.citations.retain(|citation| {
            let anchor = &citation.anchor;
            let Some(ctx) = contexts.get(anchor.document, anchor.snapshot_version) else { return false };
            let Some(block) = anchor.block_id.as_deref().and_then(|id| ctx.find(id)) else { return false };
            let quote = collapse_ws(&anchor.quote);
            !quote.is_empty() && collapse_ws(&block.text).contains(&quote)
        });
        Ok(())
    }

    /// Keep the finished exchange so the next question has the conversation.
    fn ai_remember_turn(&mut self, task: u64, answer: &str) {
        const MAX_TURNS: usize = 12;
        const MAX_CHARS: usize = 2_000;
        let Some(state) = self.ai.tasks.get(&task) else { return };
        let clip = |s: &str| s.chars().take(MAX_CHARS).collect::<String>();
        let question = clip(&state.question);
        self.ai.history.push(ChatTurn { role: "user".into(), text: question });
        self.ai.history.push(ChatTurn { role: "assistant".into(), text: clip(answer) });
        let excess = self.ai.history.len().saturating_sub(MAX_TURNS);
        self.ai.history.drain(..excess);
    }

    /// i.history.clear: start a fresh conversation.
    pub(super) fn ai_history_clear(&mut self) -> Result<Option<Value>, String> {
        self.ai.history.clear();
        self.ai.answers.clear_turns();
        self.ai.ui.clear_finished();
        self.emit("ai.history.cleared", json!({ "state_seq": self.ai.ui.snapshot()["sequence"] }));
        Ok(Some(json!({ "cleared": true })))
    }

    fn ai_finish_failed(&mut self, task: u64, failure: Failure) {
        self.ai_complete(task, TaskOutcome::Failed(TaskFailed { task: TaskId(task), failure }));
    }

    fn ai_complete(&mut self, task: u64, outcome: TaskOutcome) {
        let Some(state) = self.ai.tasks.remove(&task) else { return };
        let elapsed = state.started.elapsed().as_millis() as u64;
        let fallback;
        let ctx: &PageContext = match self.ai.contexts.get(state.document.document_token(), state.snapshot) {
            Some(c) => c,
            None => {
                fallback = PageContext {
                    document: state.document.document_token(),
                    snapshot_version: state.snapshot,
                    navigation_sequence: state.document.navigation_sequence(),
                    url: String::new(),
                    title: String::new(),
                    blocks: Vec::new(),
                    links: Vec::new(),
                    frames_not_read: 0,
                    dropped_blocks: 0,
                };
                &fallback
            }
        };
        self.ai.answers.finish(state.turn_id, outcome, elapsed, ctx);
        let payload = self.ai.answers.payload(state.turn_id);
        self.emit("ai.answer", payload);
    }

    /// Fail tasks whose deadline passed without a verdict from the utility.
    pub(super) fn ai_tick(&mut self) {
        if self.ai.tasks.is_empty() {
            return;
        }
        let now = now_ms();
        let overdue: Vec<u64> = self
            .ai
            .tasks
            .iter()
            .filter(|(_, t)| now > t.deadline_ms + DEADLINE_GRACE_MS)
            .map(|(id, _)| *id)
            .collect();
        for task in overdue {
            let params = serde_json::to_value(CancellationRequest {
                task: TaskId(task),
                reason: CancelReason::DeadlineElapsed,
            });
            if let Ok(params) = params {
                let _ = self.ai_request(Method::CancelTask, &params, Pending::Cancel(task));
            }
            self.ai_finish_failed(task, Failure::timed_out("task.deadline_elapsed"));
        }
    }

    // ---- context.get_snapshot ------------------------------------------

    fn ai_on_get_snapshot(&mut self, id: u64, params: Value) {
        let request: ContextSnapshotRequest = match serde_json::from_value(params) {
            Ok(r) => r,
            Err(e) => {
                return self.ai_send(Frame::failure(
                    id,
                    Failure::unsupported("snapshot.request_unreadable").with_detail(e.to_string()),
                ))
            }
        };
        let token = request.document;
        // A conversation with no readable page holds an empty stand-in capture: answer
        // from it instead of trying to read a tab that has nothing to read.
        if self.ai.contexts.newest(token).is_some_and(|c| c.url.is_empty() && c.blocks.is_empty()) {
            return self.ai_reply_snapshot(id, token, "");
        }
        let Some(scope) = self.ai.identity.scope() else {
            return self.ai_send(Frame::failure(id, Failure::unavailable("identity.workspace_unavailable")));
        };
        if let Err(f) = self.ai.identity.registry().resolve(token, &scope) {
            return self.ai_send(Frame::failure(id, f));
        }
        if let Some(ctx) = self.ai.contexts.newest(token) {
            let title = ctx.title.clone();
            return self.ai_reply_snapshot(id, token, &title);
        }
        // Not captured yet: only another live tab can be captured on demand, and
        // only when multi-tab context is switched on.
        if !self.ai.config.flags.multi_tab_context {
            return self.ai_send(Frame::failure(id, Failure::blocked("context.multi_tab_disabled")));
        }
        let Some(tab) = self.ai.identity.tab_of_document(token).map(TabId) else {
            return self.ai_send(Frame::failure(id, Failure::stale("context.document_not_in_a_tab")));
        };
        let version = match self.ai.identity.snapshot_captured(token) {
            Ok(v) => v,
            Err(f) => return self.ai_send(Frame::failure(id, f)),
        };
        let proxy = self.proxy();
        let what = CaptureFor::Snapshot { request: id, token: token.0, version: version.0 };
        let started = match self.live.get(&tab) {
            Some(live) => context::capture(&live.webview, move |result| {
                let _ = proxy.send_event(AppMessage::AiCaptured(tab, what, result));
            }),
            None => Err(Failure::unavailable("tab.sleeping")
                .with_detail("that tab's page is not loaded; switch to it to let the assistant read it")),
        };
        if let Err(f) = started {
            self.ai_send(Frame::failure(id, f));
        }
    }

    /// Which other tabs the question asked to include: `"tabs": "all"` or a list of ids.
    /// Only loaded, readable pages other than the active one qualify.
    fn ai_extra_tabs(&self, p: &Value, active: TabId) -> Vec<u64> {
        const MAX_EXTRA_TABS: usize = 5;
        let wanted: Option<Vec<u64>> = match p.get("tabs") {
            Some(Value::Array(ids)) => Some(ids.iter().filter_map(Value::as_u64).collect()),
            Some(Value::String(s)) if s == "all" => None,
            _ => return Vec::new(),
        };
        self.tabs
            .tabs()
            .iter()
            .filter(|t| t.id != active && is_readable_url(&t.url))
            .filter(|t| wanted.as_ref().map_or(true, |w| w.contains(&t.id.0)))
            .take(MAX_EXTRA_TABS)
            .map(|t| t.id.0)
            .collect()
    }

    /// Read another tab. A background tab has no webview (it was slept), so it is woken
    /// hidden, given a moment to load, and read; the user's view does not change.
    fn ai_capture_extra(&mut self, task: u64, tab: TabId) {
        if let Some(s) = self.ai.tasks.get_mut(&task) {
            s.extra_attempts = 0;
        }
        if self.live.contains_key(&tab) {
            return self.ai_read_extra_now(task, tab);
        }
        self.build_webview(tab);
        match self.live.get(&tab) {
            Some(live) => {
                let _ = live.webview.set_visible(false);
                self.ai_read_extra_later(task, tab, 1200);
            }
            None => self.ai_extra_advance(task, tab),
        }
    }

    fn ai_read_extra_later(&self, task: u64, tab: TabId, ms: u64) {
        let proxy = self.proxy();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let _ = proxy.send_event(AppMessage::AiReadExtra(task, tab));
        });
    }

    pub(super) fn ai_read_extra_now(&mut self, task: u64, tab: TabId) {
        if !self.ai.tasks.contains_key(&task) {
            return;
        }
        let proxy = self.proxy();
        let started = match self.live.get(&tab) {
            Some(live) => context::capture(&live.webview, move |result| {
                let _ = proxy.send_event(AppMessage::AiCaptured(tab, CaptureFor::Extra(task), result));
            }),
            None => Err(Failure::unavailable("tab.sleeping")),
        };
        // A tab that cannot be read is skipped; the question still runs on the rest.
        if started.is_err() {
            self.ai_extra_advance(task, tab);
        }
    }

    fn ai_on_extra_captured(&mut self, tab: TabId, task: u64, result: Result<String, String>) {
        use crate::ai::context::Block;
        const MAX_EXTRA_BLOCKS: usize = 40;
        const MAX_ATTEMPTS: u8 = 8;
        if !self.ai.tasks.contains_key(&task) {
            return;
        }
        // A tab that was just woken may still be loading: an empty read is retried.
        let empty = result
            .as_ref()
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .and_then(|p| context::parse(&p, DocumentToken(0), SnapshotVersion(0), 0, "", "").ok())
            .map_or(true, |c| c.blocks.is_empty());
        if empty {
            let attempts = self.ai.tasks.get_mut(&task).map(|s| { s.extra_attempts += 1; s.extra_attempts }).unwrap_or(MAX_ATTEMPTS);
            if attempts < MAX_ATTEMPTS {
                return self.ai_read_extra_later(task, tab, 600);
            }
        }
        if let Some(key) = self.ai.identity.current(tab.0) {
            let token = key.document_token();
            if let Ok(version) = self.ai.identity.snapshot_captured(token) {
                let (url, title) = self.tabs.get(tab).map(|t| (t.url.clone(), t.display_title())).unwrap_or_default();
                let ctx = result
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|payload| context::parse(&payload, token, version, key.navigation_sequence(), &url, &title).ok());
                if let (Some(ctx), Some(state)) = (ctx, self.ai.tasks.get_mut(&task)) {
                    let prefix = format!("t{}.", tab.0);
                    state.merged.push(Block {
                        block_id: format!("{prefix}h"),
                        text: format!("[Another open tab: \"{}\" - {}]", ctx.title, ctx.url),
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    });
                    for b in ctx.blocks.into_iter().take(MAX_EXTRA_BLOCKS) {
                        state.merged.push(Block { block_id: format!("{prefix}{}", b.block_id), ..b });
                    }
                    self.ai.extra_docs.insert(tab.0, token.0);
                }
            }
        }
        self.ai_extra_advance(task, tab);
    }

    /// One extra tab is done (read or skipped): read the next, or start the task.
    fn ai_extra_advance(&mut self, task: u64, done: TabId) {
        let Some(state) = self.ai.tasks.get_mut(&task) else { return };
        state.extra_tabs.retain(|t| *t != done.0);
        if let Some(next) = state.extra_tabs.first().copied() {
            return self.ai_capture_extra(task, TabId(next));
        }
        if let Some(mut primary) = state.primary.take() {
            let mut blocks = std::mem::take(&mut state.merged);
            if !blocks.is_empty() {
                let head = crate::ai::context::Block {
                    block_id: "a.h".into(),
                    text: format!("[Active tab: \"{}\" - {}]", primary.title, primary.url),
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                };
                primary.blocks.insert(0, head);
                primary.blocks.append(&mut blocks);
            }
            self.ai.contexts.put(primary);
        }
        self.ai_start_task(task);
    }

    fn ai_snapshot_failed(&mut self, what: CaptureFor, failure: Failure) {
        if let CaptureFor::Snapshot { request, .. } = what {
            self.ai_send(Frame::failure(request, failure));
        }
    }

    fn ai_reply_snapshot(&mut self, id: u64, token: DocumentToken, title: &str) {
        let Some(scope) = self.ai.identity.scope() else {
            return self.ai_send(Frame::failure(id, Failure::unavailable("identity.workspace_unavailable")));
        };
        let key = match self.ai.identity.registry().resolve(token, &scope) {
            Ok(k) => k,
            Err(f) => return self.ai_send(Frame::failure(id, f)),
        };
        let Some(ctx) = self.ai.contexts.newest(token) else {
            return self.ai_send(Frame::failure(id, Failure::unavailable("context.not_captured")));
        };
        let snapshot = snapshot_of(key, ctx, title);
        match ResponsePayload::ok(&snapshot) {
            Ok(payload) => self.ai_send(Frame::ok(id, payload)),
            Err(f) => self.ai_send(Frame::failure(id, f)),
        }
    }

    // ---- action.dispatch -----------------------------------------------

    fn ai_on_action(&mut self, id: u64, params: Value) {
        let request: ActionRequest = match serde_json::from_value(params) {
            Ok(r) => r,
            Err(e) => {
                return self.ai_send(Frame::failure(
                    id,
                    Failure::unsupported("action.request_unreadable").with_detail(e.to_string()),
                ))
            }
        };
        let level = match self.ai.admit_action(&request, now_ms()) {
            Ok(level) => level,
            Err(failure) => {
                self.note(format!("ai action {} refused: {}", request.action_kind, failure_message(&failure)));
                return self.ai_send(Frame::failure(id, failure));
            }
        };
        match self.ai_execute(&request) {
            Ok((verified, payload)) => {
                let (document, snapshot) = match self.ai.tasks.get(&request.task.0) {
                    Some(t) => (t.document, t.snapshot),
                    None => return self.ai_send(Frame::failure(id, Failure::stale("action.task_unknown"))),
                };
                let result = ActionResult {
                    action_id: request.action_id.clone(),
                    decision: ActionDecision { level, approval_required: false, approval_granted: false, blocked_by: None },
                    executed_on: document,
                    observation_snapshot_version: snapshot,
                    verified,
                    result_payload: payload,
                    limit: None,
                };
                self.note(format!("ai action {} ok (verified={verified})", request.action_kind));
                match ResponsePayload::ok(&result) {
                    Ok(p) => self.ai_send(Frame::ok(id, p)),
                    Err(f) => self.ai_send(Frame::failure(id, f)),
                }
            }
            Err(failure) => {
                self.note(format!("ai action {} failed: {}", request.action_kind, failure_message(&failure)));
                self.ai_send(Frame::failure(id, failure));
            }
        }
    }

    /// Run one admitted action. Returns whether the effect could be confirmed, and
    /// what the utility may read about it. Anything the browser cannot confirm is
    /// reported as unverified rather than assumed.
    fn ai_execute(&mut self, request: &ActionRequest) -> Result<(bool, Value), Failure> {
        let task_id = request.task.0;
        let (turn_id, task_tab, snapshot, document, opened) = {
            let t = self.ai.tasks.get(&task_id).ok_or_else(|| Failure::stale("action.task_unknown"))?;
            (t.turn_id, TabId(t.tab), t.snapshot, t.document.document_token(), t.opened_tabs.clone())
        };
        let args = &request.arguments;
        // The utility puts model-chosen ids in the opaque target and resolved values in the
        // arguments; accept either so no action loses its operand.
        let find = |key: &str| {
            args.get(key).or_else(|| request.target.as_ref().and_then(|t| t.opaque_target.get(key)))
        };
        let text = |key: &str| find(key).and_then(Value::as_str);
        let tab_arg = || {
            find("tab_id")
                .and_then(Value::as_u64)
                .map(TabId)
                .ok_or_else(|| Failure::unsupported("action.tab_id_required"))
        };
        match request.action_kind.as_str() {
            "scroll_to" => {
                let block_id = text("block_id").ok_or_else(|| Failure::unsupported("action.block_id_required"))?;
                let ctx = self
                    .ai
                    .contexts
                    .get(document, snapshot)
                    .ok_or_else(|| Failure::stale("action.snapshot_evicted"))?;
                let block = ctx.find(block_id).ok_or_else(|| Failure::stale("action.block_missing"))?;
                let script = scroll_script(block.scroll_y(), Some([block.x, block.y, block.w, block.h]));
                let live = self.live.get(&task_tab).ok_or_else(|| Failure::stale("action.tab_not_live"))?;
                live.webview
                    .evaluate_script(&script)
                    .map_err(|e| Failure::unavailable("action.script_failed").with_detail(e.to_string()))?;
                Ok((false, json!({ "scrolled_to": block_id })))
            }
            "open_tab" | "show_candidate" => {
                let link_id = text("link_id").ok_or_else(|| Failure::unsupported("action.link_id_required"))?;
                let url = self.ai.link_url(task_id, link_id)?;
                let title = text("title").unwrap_or("").to_string();
                let reason = text("reason").unwrap_or("").to_string();
                let new = self.create_tab(&url, Some(task_tab), false);
                self.ai.ai_tabs.insert(new.0);
                if let Some(t) = self.ai.tasks.get_mut(&task_id) {
                    t.opened_tabs.push(new.0);
                }
                self.emit(
                    "ai.candidate",
                    json!({ "turn_id": turn_id, "tab_id": new.0, "url": url, "title": title, "reason": reason }),
                );
                Ok((self.tabs.get(new).is_some(), json!({ "tab_id": new.0, "url": url })))
            }
            "activate_tab" => {
                let target = tab_arg()?;
                if target != task_tab && !opened.contains(&target.0) {
                    return Err(Failure::blocked("action.tab_not_owned_by_task"));
                }
                if self.tabs.get(target).is_none() {
                    return Err(Failure::stale("action.tab_gone"));
                }
                self.activate(target);
                Ok((self.active == Some(target), json!({ "tab_id": target.0 })))
            }
            "close_tab" => {
                let target = tab_arg()?;
                if !opened.contains(&target.0) {
                    return Err(Failure::blocked("action.tab_not_opened_by_task"));
                }
                self.close_tab(target);
                Ok((self.tabs.get(target).is_none(), json!({ "closed": target.0 })))
            }
            "navigate" => {
                let link_id = text("link_id").ok_or_else(|| Failure::unsupported("action.link_id_required"))?;
                let url = self.ai.link_url(task_id, link_id)?;
                self.navigate(task_tab, &url, false);
                // Navigation starts; whether it lands is only known when it commits.
                Ok((false, json!({ "navigating_to": url })))
            }
            "list_tabs" => {
                if !self.ai.config.flags.multi_tab_context {
                    return Err(Failure::blocked("context.multi_tab_disabled"));
                }
                let rows: Vec<Value> = self
                    .tabs
                    .tabs()
                    .iter()
                    .map(|t| {
                        let doc = self.ai.identity.current(t.id.0).map(|k| k.document_token().0);
                        json!({
                            "tab_id": t.id.0, "title": t.display_title(), "url": t.url,
                            "active": self.active == Some(t.id), "sleeping": t.sleeping,
                            "ai_opened": self.ai.ai_tabs.contains(&t.id.0), "document": doc,
                        })
                    })
                    .collect();
                Ok((true, json!({ "tabs": rows })))
            }
            other => Err(Failure::unsupported("action.no_handler").with_detail(other)),
        }
    }

    // ---- citations ------------------------------------------------------

    pub(super) fn ai_citation_scroll(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let turn = p.get("turn_id").and_then(Value::as_u64).ok_or("turn_id required")?;
        let citation = p.get("citation_id").and_then(Value::as_u64).ok_or("citation_id required")? as usize;
        if let Some((other, y, rect)) = self
            .ai
            .answers
            .foreign_citation(&self.ai.identity, turn, citation)
            .map_err(|f| failure_message(&f))?
        {
            let recorded = self.ai.extra_docs.get(&other).copied();
            let current = self.ai.identity.current(other).map(|k| k.document_token().0);
            if recorded.is_none() || recorded != current {
                return Err("that tab has navigated since this answer".into());
            }
            let tab = TabId(other);
            if self.active != Some(tab) {
                self.activate(tab);
            }
            let live = self.live.get(&tab).ok_or("that tab is not loaded")?;
            live.webview.evaluate_script(&scroll_script(y, Some(rect))).map_err(|e| e.to_string())?;
            return Ok(Some(json!({ "scrolled": true, "y": y, "tab_id": other })));
        }
        let y = self
            .ai
            .answers
            .scroll_y(&self.ai.identity, turn, citation)
            .map_err(|f| failure_message(&f))?;
        let payload = self.ai.answers.payload(turn);
        let rect = payload["citations"][citation]["rect"].as_object().and_then(|r| {
            Some([r.get("x")?.as_f64()?, r.get("y")?.as_f64()?, r.get("w")?.as_f64()?, r.get("h")?.as_f64()?])
        });
        let tab = TabId(payload["tab_id"].as_u64().ok_or("turn has no tab")?);
        if self.active != Some(tab) {
            self.activate(tab);
        }
        let live = self.live.get(&tab).ok_or("the page is not loaded")?;
        live.webview.evaluate_script(&scroll_script(y, rect)).map_err(|e| e.to_string())?;
        Ok(Some(json!({ "scrolled": true, "y": y })))
    }

    // ---- selection toolbar & overlay placement --------------------------

    pub(super) fn ai_on_probe(&mut self, tab: TabId, raw: &str) {
        if let Some(url) = serde_json::from_str::<Value>(raw)
            .ok()
            .filter(|v| v.get("ai").and_then(Value::as_str) == Some("pdf"))
            .and_then(|v| v.get("url").and_then(Value::as_str).map(str::to_string))
        {
            if !crate::pdf::is_pdf_url(&url) {
                let viewer = crate::pdf::viewer_url(&url);
                self.navigate(tab, &viewer, false);
            }
            return;
        }
        if let Some(dir) = serde_json::from_str::<Value>(raw)
            .ok()
            .filter(|v| v.get("ai").and_then(Value::as_str) == Some("zoom"))
            .map(|v| v.get("dir").and_then(Value::as_i64).unwrap_or(0))
        {
            self.canvas_focus_tab(tab);
            let action = match dir {
                d if d > 0 => "zoom_in",
                0 => "zoom_reset",
                _ => "zoom_out",
            };
            self.zoom_tab(tab, action);
            return;
        }
        if self.canvas_probe(tab, raw) {
            return;
        }
        if let Some(url) = open_request(raw) {
            if self.active == Some(tab) {
                self.open_from_page(tab, &url);
            }
            return;
        }
        if crate::ai::session::is_change_probe(raw) {
            if self.active == Some(tab) {
                self.page_read_start(tab);
            }
            return;
        }
        let Some(sel) = Selection::from_probe(raw) else { return };
        if self.active != Some(tab) {
            return;
        }
        let Some(key) = self.ai.identity.current(tab.0) else { return };
        if sel.text.trim().is_empty() {
            if self.ai.selections.remove(&tab.0).is_some() {
                self.ai_notify(BrowserEvent::SelectionChanged { document: key.document_token(), selected_chars: 0 });
                self.agent_context_changed();
            }
            self.ai_hide_toolbar();
            return;
        }
        let chars = sel.text.chars().count() as u32;
        let rect = sel.rect;
        self.ai.selections.insert(tab.0, sel);
        self.ai_notify(BrowserEvent::SelectionChanged { document: key.document_token(), selected_chars: chars });
        self.agent_context_changed();
        let usable = self.ai.config.flags.selection_toolbar && self.ai.availability().is_ok();
        let free = matches!(self.overlay_rect, Overlay::Closed) || self.ai.toolbar_open;
        if let (true, true, Some(rect)) = (usable, free, rect) {
            self.ai_show_toolbar(tab, rect);
        }
    }

    fn ai_show_toolbar(&mut self, tab: TabId, rect: [f64; 4]) {
        let l = self.layout();
        let area = self.tab_geom(tab).0;
        let zoom = if self.is_viewer_tab(tab) {
            1.0
        } else {
            self.tabs.get(tab).map(|t| t.zoom).filter(|z| *z > 0.0).unwrap_or(1.0)
        };
        let x = area.x + (rect[0] * zoom) as i32;
        let above = area.y + (rect[1] * zoom) as i32 - TOOLBAR_H - 8;
        let y = if above >= l.page_y {
            above
        } else {
            area.y + ((rect[1] + rect[3]) * zoom) as i32 + 8
        };
        self.ai.anchor = (x, y);
        self.ai.toolbar_open = true;
        self.place_overlay(Overlay::Anchored { x, y, w: TOOLBAR_W, h: TOOLBAR_H }, "toolbar", false);
    }

    pub(super) fn ai_hide_toolbar(&mut self) {
        if self.ai.toolbar_open {
            self.ai.toolbar_open = false;
            if matches!(self.overlay_rect, Overlay::Anchored { h, .. } if h == TOOLBAR_H) {
                self.close_overlay();
            }
        }
    }

    /// Show an overlay surface at a rect and tell the overlay page what it is.
    fn place_overlay(&mut self, which: Overlay, kind: &str, focus: bool) {
        let (size, scale) = self.viewport();
        let l = layout::compute(size, scale, self.settings.tabs(), self.panel, which);
        self.overlay_rect = which;
        if let Some(ov) = self.overlay.as_ref() {
            let _ = ov.set_bounds(layout::rect(l.overlay_x, l.overlay_y, l.overlay_w, l.overlay_h));
            let _ = ov.set_visible(true);
            if focus {
                let _ = ov.focus();
            }
        }
        self.emit_raw("overlay", "overlay.state", json!({ "open": true, "kind": kind }));
    }

    /// Ctrl+Shift+A / the header button: toggle the docked AI side pane.
    pub(super) fn ai_open_ask(&mut self) {
        self.ai.toolbar_open = false;
        if self.panel == Panel::Ai {
            self.set_panel(Panel::None);
            return;
        }
        self.set_panel(Panel::Ai);
        if let Some(sb) = self.sidebar.as_ref() {
            let _ = sb.focus();
        }
        self.emit_raw("sidebar", "ai.focus", json!({}));
    }

    /// Make sure the side pane is open (unlike `ai_open_ask`, never closes it).
    pub(super) fn ai_show_pane(&mut self) {
        if self.panel != Panel::Ai {
            self.ai_open_ask();
        }
    }

    /// The side pane's webview, created on first use and docked right of the page.
    pub(super) fn ai_layout_sidebar(&mut self) {
        let l = self.layout();
        if self.panel != Panel::Ai {
            if let Some(sb) = self.sidebar.as_ref() {
                let _ = sb.set_visible(false);
                let _ = sb.set_bounds(layout::rect(0, 0, 0, 0));
            }
            return;
        }
        if self.sidebar.is_none() {
            self.ai_build_sidebar();
        }
        let x = l.page_x + l.page_w;
        if let Some(sb) = self.sidebar.as_ref() {
            let _ = sb.set_bounds(layout::rect(x, l.header_h, layout::ai_width(), (l.shell_h - l.header_h).max(0)));
            let _ = sb.set_visible(!self.menu_open);
        }
    }

    fn ai_build_sidebar(&mut self) {
        let proxy = self.proxy();
        let builder = wry::WebViewBuilder::new()
            .with_id("sidebar")
            .with_bounds(layout::rect(0, 0, 0, 0))
            .with_visible(false)
            .with_custom_protocol("asset".into(), super::asset_handler())
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let _ = proxy.send_event(AppMessage::Ipc(crate::ipc::Surface::Sidebar, req.body().clone()));
            })
            .with_navigation_handler(|url| super::is_asset_origin(&url))
            .with_url("asset://shell/sidebar/sidebar.html");
        let builder = crate::platform::configure(builder, self.environment.as_ref(), true);
        let Some(window) = self.window.as_ref() else { return };
        match builder.build_as_child(window) {
            Ok(wv) => {
                self.sidebar = Some(wv);
                self.note("ai sidebar webview ready");
            }
            Err(e) => self.note(format!("ai sidebar failed: {e}")),
        }
    }

    /// `ai.provider.set`: the in-browser endpoint form. The endpoint and model go to
    /// `ai.json`; the key stays in the OS vault and this engine's private memory.
    pub(super) fn ai_provider_set(&mut self, p: &Value) -> Result<Option<Value>, String> {
        if !self.ai.tasks.is_empty() || self.ai.agent_run != 0 || !self.ai.agent_jobs.is_empty() {
            return Err("stop AI work before changing the provider".into());
        }
        let key = p.get("api_key").and_then(Value::as_str).unwrap_or("").trim();
        if key.len() > 400 {
            return Err("key too long".into());
        }
        let clear = p["clear_key"].as_bool().unwrap_or(false);
        ai_utility::secrets::migrate_provider(&self.data_dir)?;
        self.ai_reload_config();
        let mut entry = super::providers::prepare(&self.ai.config, p, false)?;
        let secret = format!("provider.{}", entry.id);
        let saved = ai_utility::secrets::get(&self.data_dir, &secret)?;
        let previous = self.ai.config.providers.iter().find(|provider| provider.id == entry.id)
            .and_then(|provider| provider.base_url.as_deref()).and_then(|url| url::Url::parse(url).ok());
        let endpoint = url::Url::parse(entry.base_url.as_deref().unwrap()).map_err(|_| "invalid endpoint")?;
        if key.is_empty() && !clear && saved.is_some() && previous.is_some_and(|url| url.origin() != endpoint.origin()) {
            return Err("the endpoint host changed; enter its API key or remove the saved key first".into());
        }
        if !clear && (!key.is_empty() || saved.is_some()) {
            entry.api_key_env = Some(super::providers::key_env(&entry.id));
        }
        let mut cfg = self.ai.config.clone();
        if entry.enabled {
            cfg.flags.ai_enabled = true;
            if entry.execution == ai_platform_contract::model::Execution::Cloud {
                cfg.flags.cloud_models_enabled = true;
            }
        }
        if let Some(previous) = cfg.providers.iter_mut().find(|provider| provider.id == entry.id) {
            *previous = entry;
        } else {
            cfg.providers.push(entry);
        }
        let changed_key = !key.is_empty() || clear;
        if changed_key {
            ai_utility::secrets::set(&self.data_dir, &secret, (!clear).then_some(key))?;
        }
        if let Err(error) = cfg.save(&self.ai_config_path()) {
            if changed_key {
                ai_utility::secrets::set(&self.data_dir, &secret, saved.as_deref())
                    .map_err(|rollback| format!("{error}; credential rollback failed: {rollback}"))?;
            }
            return Err(error.to_string());
        }
        self.ai_provider_changed(cfg)
    }

    pub(super) fn ai_provider_delete(&mut self, p: &Value) -> Result<Option<Value>, String> {
        if !self.ai.tasks.is_empty() || self.ai.agent_run != 0 || !self.ai.agent_jobs.is_empty() {
            return Err("stop AI work before changing the provider".into());
        }
        ai_utility::secrets::migrate_provider(&self.data_dir)?;
        self.ai_reload_config();
        let id = p.get("id").and_then(Value::as_str).ok_or("connection ID required")?;
        let mut cfg = self.ai.config.clone();
        let index = cfg.providers.iter().position(|entry| entry.id == id
            && entry.kind == ai_platform_contract::config::ProviderKind::OpenAiCompatible).ok_or("connection not found")?;
        cfg.providers.remove(index);
        cfg.save(&self.ai_config_path()).map_err(|e| e.to_string())?;
        if let Err(error) = ai_utility::secrets::set(&self.data_dir, &format!("provider.{id}"), None) {
            self.ai.config.save(&self.ai_config_path()).map_err(|rollback| format!("{error}; config rollback failed: {rollback}"))?;
            return Err(error);
        }
        self.ai_provider_changed(cfg)
    }

    fn ai_provider_changed(&mut self, config: AiConfig) -> Result<Option<Value>, String> {
        self.ai.config = config;
        self.ai.provider_view = None;
        self.ai.link = None;
        self.ai.pending.clear();
        self.ai.models.clear();
        let status = self.ai_status();
        self.emit("ai.context", status.clone());
        Ok(Some(status))
    }

    /// The provider form's current values (never the key).
    fn ai_provider_view(&mut self) -> Value {
        if let Some(view) = self.ai.provider_view.as_ref() {
            return view.clone();
        }
        let custom = self.ai.config.providers.iter().find(|q| q.id == "custom");
        let connections: Vec<Value> = self.ai.config.providers.iter()
            .filter(|entry| entry.kind == ai_platform_contract::config::ProviderKind::OpenAiCompatible)
            .map(|entry| {
                let saved = ai_utility::secrets::get(&self.data_dir, &format!("provider.{}", entry.id));
                json!({
                    "id": entry.id, "base_url": entry.base_url, "models": entry.models,
                    "enabled": entry.enabled,
                    "key_saved": saved.as_ref().is_ok_and(|key| key.is_some()),
                    "key_error": saved.err(),
                })
            }).collect();
        let view = json!({
            "base_url": custom.and_then(|q| q.base_url.clone()),
            "model_id": custom.and_then(|q| q.models.first().map(|m| m.model_id.clone())),
            "key_saved": connections.iter().find(|entry| entry["id"] == "custom").is_some_and(|entry| entry["key_saved"] == true),
            "connections": connections,
            "configured": self.ai.config.providers.iter().any(|q| q.enabled
                && q.kind == ai_platform_contract::config::ProviderKind::OpenAiCompatible),
        });
        self.ai.provider_view = Some(view.clone());
        view
    }

    fn ai_key_env(&self) -> Result<Vec<(String, String)>, String> {
        ai_utility::secrets::migrate_provider(&self.data_dir)?;
        let mut credentials = Vec::new();
        for provider in &self.ai.config.providers {
            if let Some(env) = provider.api_key_env.as_ref() {
                if let Some(key) = ai_utility::secrets::get(&self.data_dir, &format!("provider.{}", provider.id))? {
                    credentials.push((env.clone(), key));
                }
            }
        }
        Ok(credentials)
    }

    /// A toolbar button. With the side pane open the pane owns the conversation, so it
    /// is told what was asked and runs it in whichever mode it is in; the selection
    /// itself travels with the request. Otherwise the toolbar becomes an inline card
    /// at the same spot and the task starts there.
    pub(super) fn ai_selection_act(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let (x, y) = self.ai.anchor;
        if self.panel == Panel::Ai {
            self.ai_hide_toolbar();
            let intent = p.get("intent").and_then(Value::as_str).unwrap_or("ask").to_string();
            self.emit("sidebar.selection", json!({ "intent": intent }));
            return Ok(Some(json!({ "routed": true })));
        }
        self.ai.toolbar_open = false;
        self.place_overlay(Overlay::Anchored { x, y, w: CARD_W, h: CARD_H }, "card", true);
        let mut args = p.clone();
        args["selection"] = Value::Bool(true);
        self.ai_ask(&args)
    }

    // ---- status ----------------------------------------------------------

    pub(super) fn ai_status(&mut self) -> Value {
        self.ai_reload_config();
        let tab = self.active.and_then(|id| self.tabs.get(id)).map(|t| (t.id, t.display_title(), t.url.clone()));
        let doc = tab.as_ref().and_then(|(id, _, _)| self.ai.identity.current(id.0));
        let context = doc.and_then(|k| self.ai.contexts.newest(k.document_token())).map(|c| {
            json!({ "coverage": crate::ai::session::coverage(c), "blocks": c.blocks.len(), "links": c.links.len() })
        });
        let selection = tab
            .as_ref()
            .and_then(|(id, _, _)| self.ai.selections.get(&id.0))
            .map(|s| json!({ "chars": s.text.chars().count() }));
        let unavailable = self.ai.availability().err().map(|f| failure_message(&f));
        let reason = unavailable.clone().or_else(|| self.ai.link_error.clone());
        json!({
            "enabled": unavailable.is_none(),
            "reason": reason,
            "askable": doc.is_some(),
            "tab": tab.map(|(id, title, url)| json!({
                "tab_id": id.0, "title": title, "host": host_of(&url).unwrap_or_default(),
                "ai_managed": self.ai.ai_tabs.contains(&id.0),
            })),
            "selection": selection,
            "context": context,
            "link": if self.ai.link.is_some() { "running" } else { "idle" },
            "models": self.ai.models,
            "cloud_enabled": self.ai.config.flags.cloud_models_enabled,
            "multi_tab": self.ai.config.flags.multi_tab_context,
            "other_tabs": self
                .tabs
                .tabs()
                .iter()
                .filter(|t| Some(t.id) != self.active && is_readable_url(&t.url))
                .map(|t| json!({ "tab_id": t.id.0, "title": t.display_title(), "host": host_of(&t.url).unwrap_or_default() }))
                .collect::<Vec<_>>(),
            "max_level": self.ai.config.flags.max_action_level,
            "busy": !self.ai.tasks.is_empty() || self.ai.agent_run != 0 || !self.ai.agent_jobs.is_empty(),
            "config_path": self.ai_config_path().display().to_string(),
            "provider": self.ai_provider_view(),
        })
    }

    /// `ai.models`: the catalogue the picker shows, refreshed from the utility.
    pub(super) fn ai_models(&mut self) -> Result<Option<Value>, String> {
        self.ai_ensure_link().map_err(|f| failure_message(&f))?;
        self.ai_request(Method::ListModels, &json!({}), Pending::Models).map_err(|f| failure_message(&f))?;
        Ok(Some(json!({ "models": self.ai.models })))
    }

    /// `ai.config.set`: the kill switches the UI exposes. Written to `ai.json`, which
    /// is also what the utility reads, so the two cannot disagree.
    pub(super) fn ai_config_set(&mut self, p: &Value) -> Result<Option<Value>, String> {
        self.ai_reload_config();
        let flag = p.get("flag").and_then(Value::as_str).ok_or("flag required")?;
        let on = p.get("value").and_then(Value::as_bool).ok_or("value must be true or false")?;
        let flags = &mut self.ai.config.flags;
        match flag {
            "ai_enabled" => flags.ai_enabled = on,
            "cloud_models_enabled" => flags.cloud_models_enabled = on,
            "local_models_enabled" => flags.local_models_enabled = on,
            "selection_toolbar" => flags.selection_toolbar = on,
            "page_context" => flags.page_context = on,
            "multi_tab_context" => flags.multi_tab_context = on,
            other => return Err(format!("unknown flag {other}")),
        }
        let path = self.ai_config_path();
        self.ai.config.save(&path).map_err(|e| e.to_string())?;
        self.ai_notify(BrowserEvent::ConfigChanged);
        if !on && flag == "ai_enabled" {
            self.ai_hide_toolbar();
        }
        Ok(Some(self.ai_status()))
    }

    // ---- lifecycle hooks -------------------------------------------------

    pub(super) fn ai_on_navigation(&mut self, id: TabId, url: &str) {
        self.ai.page_snaps.remove(&id.0);
        let replaced = self.ai.identity.current(id.0).map(|k| k.document_token());
        self.ai.selections.remove(&id.0);
        if self.active == Some(id) {
            self.ai_hide_toolbar();
        }
        if is_readable_url(url) {
            match self.ai.identity.on_document_committed(id.0, url) {
                Ok(key) => self.ai_notify(BrowserEvent::NavigationCommitted {
                    tab_id: id.0,
                    document: key.document_token(),
                    navigation_sequence: key.navigation_sequence(),
                    replaced,
                }),
                Err(f) => self.note(format!("ai identity: {}", failure_message(&f))),
            }
        } else if let Some(old) = replaced {
            self.ai.identity.forget_current(id.0);
            self.ai_notify(BrowserEvent::DocumentDestroyed { document: old });
        }
        if replaced.is_some() {
            let stale = self.ai.answers.invalidate_tab(id.0);
            if !stale.is_empty() {
                self.emit("ai.answer.stale", json!({ "turn_ids": stale, "status": "stale" }));
            }
        }
    }

    pub(super) fn ai_on_activated(&mut self, id: TabId) {
        self.ai_hide_toolbar();
        let document = self.ai.identity.current(id.0).map(|k| k.document_token());
        self.ai_notify(BrowserEvent::TabActivated { tab_id: id.0, document });
        let status = self.ai_status();
        self.emit("ai.context", status);
    }

    pub(super) fn ai_on_tab_closed(&mut self, id: TabId) {
        let stale = self.ai.answers.invalidate_tab(id.0);
        if !stale.is_empty() {
            self.emit("ai.answer.stale", json!({ "turn_ids": stale, "status": "closed" }));
        }
        self.ai.identity.on_tab_closed(id.0);
        self.ai.selections.remove(&id.0);
        self.ai.ai_tabs.remove(&id.0);
        self.ai_notify(BrowserEvent::TabClosed { tab_id: id.0 });
    }

    /// Browser shutdown: drop the link, which kills the utility with its job object.
    pub(super) fn ai_shutdown(&mut self) {
        self.ai.link = None;
    }
}
