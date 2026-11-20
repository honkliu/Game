use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::sync::oneshot::Sender;

use ai_platform_contract::contract::{ActionLevel, ActionRequest};
use ai_platform_contract::identity::DocumentKey;
use serde_json::{json, Value};

use super::{AppState, TabId};
use crate::api_server::{ApiCaller, ApiRequest, ApiServer};
use crate::ipc::{Request, Surface};

pub(super) enum ReplyTarget {
    External(Sender<Result<Value, String>>),
}

pub(super) struct PendingCall {
    pub target: ReplyTarget,
    pub caller: ApiCaller,
    pub started: Instant,
    pub deferred: bool,
    pub action: Option<(ActionRequest, ActionLevel, DocumentKey)>,
}

#[derive(Default)]
pub(super) struct ClientState {
    pub observed: HashMap<u64, DocumentKey>,
    pub opened_tabs: HashSet<u64>,
}

#[derive(Default)]
pub(super) struct State {
    pub server: Option<ApiServer>,
    pub river_mcp_error: Option<String>,
    pub current: u64,
    pub next_id: u64,
    pub pending: HashMap<u64, PendingCall>,
    pub clients: HashMap<u64, ClientState>,
    pub evals: HashMap<u64, super::operations::Evaluation>,
    pub site_data: HashMap<u64, super::devtools::PendingCall>,
}

impl AppState {
    pub(super) fn wv_api_caller(&self) -> ApiCaller {
        self.api
            .pending
            .get(&self.api.current)
            .map(|p| p.caller)
            .unwrap_or(ApiCaller {
                id: 0,
                builtin: true,
                surface: None,
            })
    }

    pub(super) fn wv_ui_bootstrap(&self, surface: Surface) -> String {
        let connection = self
            .api
            .server
            .as_ref()
            .ok_or_else(|| "MCP server unavailable".to_string())
            .and_then(|server| server.ui_connection(surface));
        let Ok(connection) = connection else {
            return "console.error('Shell MCP connection unavailable')".to_string();
        };
        let path = match surface {
            Surface::Shell => "/index.html",
            Surface::Overlay => "/overlay.html",
            Surface::Sidebar => "/sidebar/sidebar.html",
            Surface::Settings(_) => "/pages/settings.html",
            Surface::ApiCall(_) => return String::new(),
        };
        let Ok(encoded) = serde_json::to_string(&connection) else {
            return String::new();
        };
        format!("(()=>{{if(window.top!==window)return;const u=new URL(location.href);if(!['asset:','http:','https:'].includes(u.protocol)||!['shell','asset.shell'].includes(u.hostname)||u.pathname!=={path:?})return;Object.defineProperty(window,'__shellMcpBootstrap',{{value:{encoded},configurable:true}});}})()")
    }

    fn wv_resource_read(&mut self, uri: &str) -> Result<Value, String> {
        match uri {
            "shell://context" => Ok(self.wv_agent_context_get()),
            "shell://tabs" => Ok(self.tabs_payload()),
            "shell://groups" => Ok(self.groups_json()),
            "shell://window" => Ok(self.wv_window_state_get()),
            "shell://settings" => Ok(self.wv_settings_state_get()),
            "shell://downloads" => Ok(serde_json::json!({"downloads": self.downloads})),
            "shell://chat" => Ok(serde_json::json!({
                "session": self.ai.ui.snapshot(),
                "status": self.wv_ai_status_get(),
                "models": self.ai.models,
            })),
            _ => Err(format!("unknown shell resource: {uri}")),
        }
    }

    pub(super) fn wv_api_begin(&mut self, caller: ApiCaller, target: ReplyTarget) -> u64 {
        self.api.next_id += 1;
        let id = self.api.next_id;
        self.api.pending.insert(
            id,
            PendingCall {
                target,
                caller,
                started: Instant::now(),
                deferred: false,
                action: None,
            },
        );
        id
    }

    pub(super) fn wv_api_defer(&mut self, id: u64) {
        if let Some(p) = self.api.pending.get_mut(&id) {
            p.deferred = true;
        }
    }

    pub(super) fn wv_api_observed(&self, caller: u64, tab: TabId) -> Option<DocumentKey> {
        self.api
            .clients
            .get(&caller)
            .and_then(|c| c.observed.get(&tab.0))
            .copied()
    }

    pub(super) fn wv_api_record_read(&mut self, caller: u64, tab: TabId, document: DocumentKey) {
        self.api
            .clients
            .entry(caller)
            .or_default()
            .observed
            .insert(tab.0, document);
    }

    pub(super) fn wv_api_owns(&self, caller: u64, tab: TabId) -> bool {
        caller == 0
            || self
                .api
                .clients
                .get(&caller)
                .is_some_and(|c| c.opened_tabs.contains(&tab.0))
    }

    pub(super) fn wv_api_record_open(&mut self, caller: u64, tab: TabId) {
        if caller != 0 {
            self.api
                .clients
                .entry(caller)
                .or_default()
                .opened_tabs
                .insert(tab.0);
        }
    }

    pub(super) fn wv_api_external(&mut self, request: ApiRequest) {
        if request.name == "resources/read" {
            let result = request.args["uri"]
                .as_str()
                .ok_or_else(|| "resource URI required".to_string())
                .and_then(|uri| self.wv_resource_read(uri));
            let _ = request.reply.send(result);
            return;
        }
        if let Some(Surface::Settings(tab)) = request.caller.surface {
            let req = Request {
                id: None,
                cmd: request.name.clone(),
                payload: request.args.clone(),
            };
            if self.settings_view(tab).is_none() || !super::settings_page::allowed(&req) {
                let _ = request
                    .reply
                    .send(Err("command not available in Settings".into()));
                return;
            }
        }
        if self.api.pending.len() >= 128 {
            let _ = request.reply.send(Err("the browser is busy".into()));
            return;
        }
        let id = self.wv_api_begin(request.caller, ReplyTarget::External(request.reply));
        let req = Request {
            id: Some(id),
            cmd: request.name,
            payload: request.args,
        };
        let previous = std::mem::replace(&mut self.api.current, id);
        let result = self.wv_execute(&req, Surface::ApiCall(id));
        self.api.current = previous;
        if self.api.pending.get(&id).is_some_and(|p| !p.deferred) || result.is_err() {
            match result {
                Ok(value) => self.wv_reply_ok(id, value.unwrap_or(Value::Null)),
                Err(error) => self.wv_reply_err(id, error),
            }
        }
    }

    pub(super) fn wv_api_prune(&mut self) {
        self.discussion_flush();
        let expired: Vec<u64> = self
            .api
            .pending
            .iter()
            .filter(|(_, p)| {
                p.started.elapsed()
                    > Duration::from_secs(if p.caller.surface.is_some() { 150 } else { 60 })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.api.evals.remove(&id);
            self.api.site_data.remove(&id);
            self.wv_reply_err(
                id,
                "the browser operation timed out; read the page before retrying a write",
            );
        }
    }

    pub(super) fn wv_reply_ok(&mut self, id: u64, mut value: Value) {
        let Some(pending) = self.api.pending.remove(&id) else {
            return;
        };
        if let Some((request, level, document)) = pending.action {
            use ai_platform_contract::contract::{ActionDecision, ActionResult};
            let verified = value
                .get("confirmed")
                .or_else(|| value.get("verified"))
                .and_then(Value::as_bool)
                .unwrap_or(matches!(
                    request.action_kind.as_str(),
                    "list_tabs" | "activate_tab" | "close_tab" | "open_tab" | "show_candidate"
                ));
            if matches!(request.action_kind.as_str(), "open_tab" | "show_candidate") {
                if let Some(tab) = value.get("tab_id").and_then(Value::as_u64) {
                    if let Some(task) = self.ai.tasks.get_mut(&request.task.0) {
                        task.opened_tabs.push(tab);
                    }
                    if let Some(task) = self.ai.tasks.get(&request.task.0) {
                        self.emit("ai.candidate", json!({ "turn_id": task.turn_id, "tab_id": tab, "url": value["url"],
                            "title": request.arguments.get("title").cloned().unwrap_or(json!("")),
                            "reason": request.arguments.get("reason").cloned().unwrap_or(json!("")) }));
                    }
                }
            }
            let result = ActionResult {
                action_id: request.action_id,
                decision: ActionDecision {
                    level,
                    approval_required: false,
                    approval_granted: false,
                    blocked_by: None,
                },
                executed_on: document,
                observation_snapshot_version: request.expected_snapshot_version,
                verified,
                result_payload: value,
                limit: None,
            };
            value = match serde_json::to_value(result) {
                Ok(value) => value,
                Err(error) => json!({ "error": error.to_string() }),
            };
        }
        match pending.target {
            ReplyTarget::External(reply) => {
                let _ = reply.send(Ok(value));
            }
        }
    }

    pub(super) fn wv_reply_err(&mut self, id: u64, message: impl Into<String>) {
        let Some(pending) = self.api.pending.remove(&id) else {
            return;
        };
        match pending.target {
            ReplyTarget::External(reply) => {
                let _ = reply.send(Err(message.into()));
            }
        }
    }

    pub(super) fn wv_reply_failure(
        &mut self,
        id: u64,
        failure: ai_platform_contract::failure::Failure,
    ) {
        if let Some(pending) = self.api.pending.get_mut(&id) {
            pending.action = None;
        }
        self.wv_reply_ok(id, json!({ "failure": failure }));
    }

    pub(super) fn wv_api_prepare_task(
        &mut self,
        method: &str,
        args: &mut Value,
    ) -> Result<(), ai_platform_contract::failure::Failure> {
        use ai_platform_contract::failure::Failure;
        let Some(raw) = args.get("_task").cloned() else {
            return Ok(());
        };
        let caller = self.wv_api_caller();
        if !caller.builtin || caller.id == 0 {
            return Err(Failure::blocked("action.builtin_only"));
        }
        let request: ActionRequest = serde_json::from_value(raw).map_err(|error| {
            Failure::unsupported("action.request_unreadable").with_detail(error.to_string())
        })?;
        let level = self
            .ai
            .admit_action(&request, crate::ai::session::now_ms())?;
        let operation = args.get("operation").and_then(Value::as_str).unwrap_or("");
        let expected = match request.action_kind.as_str() {
            "list_tabs" => ("shell_tab", "list"),
            "open_tab" | "show_candidate" => ("shell_tab", "open"),
            "activate_tab" => ("shell_tab", "activate"),
            "close_tab" => ("shell_tab", "close"),
            "navigate" => ("shell_page_navigate", ""),
            "scroll_to" => ("shell_page_scroll_to", ""),
            _ => return Err(Failure::unsupported("action.no_handler")),
        };
        if (method, operation) != expected {
            return Err(Failure::blocked("action.api_mismatch"));
        }
        let task = self
            .ai
            .tasks
            .get(&request.task.0)
            .ok_or_else(|| Failure::stale("action.task_unknown"))?;
        let (tab, document, snapshot, opened) = (
            task.tab,
            task.document,
            task.snapshot,
            task.opened_tabs.clone(),
        );
        let find = |key: &str| {
            request.arguments.get(key).or_else(|| {
                request
                    .target
                    .as_ref()
                    .and_then(|target| target.opaque_target.get(key))
            })
        };
        let mut prepared = json!({ "operation": operation });
        match request.action_kind.as_str() {
            "list_tabs" => {
                if !self.ai.config.flags.multi_tab_context {
                    return Err(Failure::blocked("context.multi_tab_disabled"));
                }
            }
            "open_tab" | "show_candidate" | "navigate" => {
                let link = find("link_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Failure::unsupported("action.link_id_required"))?;
                prepared["url"] = json!(self.ai.link_url(request.task.0, link)?);
                prepared["tab_id"] = json!(tab);
                prepared["parent_tab_id"] = json!(tab);
                prepared["background"] = json!(true);
            }
            "activate_tab" | "close_tab" => {
                let target = find("tab_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Failure::unsupported("action.tab_id_required"))?;
                if !opened.contains(&target)
                    && !(request.action_kind == "activate_tab" && target == tab)
                {
                    return Err(Failure::blocked("action.tab_not_owned_by_task"));
                }
                if self.tabs.get(TabId(target)).is_none() {
                    return Err(Failure::stale("action.tab_gone"));
                }
                prepared["tab_id"] = json!(target);
            }
            "scroll_to" => {
                let id = find("block_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Failure::unsupported("action.block_id_required"))?;
                let context = self
                    .ai
                    .contexts
                    .get(document.document_token(), snapshot)
                    .ok_or_else(|| Failure::stale("action.snapshot_evicted"))?;
                let block = context
                    .find(id)
                    .ok_or_else(|| Failure::stale("action.block_missing"))?;
                prepared = json!({ "tab_id": tab, "block_id": id, "y": block.scroll_y(),
                    "rect": [block.x, block.y, block.w, block.h], "text": block.text });
            }
            _ => unreachable!(),
        }
        let pending = self
            .api
            .pending
            .get_mut(&self.api.current)
            .ok_or_else(|| Failure::unavailable("action.reply_missing"))?;
        pending.action = Some((request, level, document));
        *args = prepared;
        Ok(())
    }
}
