//! The shell's half of Agent mode.
//!
//! The utility runs the loop; the shell owns the browser. This file forwards run
//! events, approvals, cancellation and management calls. Browser operations live
//! in `api` and `operations`, shared by local and external MCP clients.
//!
//! Page content that comes back from a tool is untrusted data and is only ever handed
//! to the model as a tool result, never executed or spliced into a script. Values the
//! model chooses (selectors, text, values) enter page scripts only as JSON string
//! literals.

use serde_json::{json, Value};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::{Method, ResponsePayload};

use super::ai_glue::is_readable_url;
use super::{AppMessage, AppState};
use crate::ai::answer::failure_message;
use crate::ai::session::Pending;
use crate::tabs::{Tab, TabId};

/// How much of the active page's text goes into the model's context up front, and how
/// long the shell waits for the page to answer before starting without it.
const CONTENT_MAX_CHARS: usize = 20_000;
const CONTENT_WAIT_MS: u64 = 4_000;

/// The active page's readable content: the article or main region when there is one
/// (otherwise the whole body), its outline, its description and the current values of
/// its form fields (typed text is not part of `innerText`). Passwords and card numbers
/// are listed as present but their values are never read.
const CONTENT_SCRIPT: &str = include_str!("../ai/page_context.js");

impl AppState {
    // ---- browser → agent ------------------------------------------------

    /// `agent.run`: one agent turn over the conversation the sidebar holds.
    pub(super) fn wv_agent_run(&mut self, p: &Value) -> Result<Option<Value>, String> {
        if self.ai.agent_run != 0 {
            return Err("the agent is already working; stop it first".into());
        }
        let messages = p.get("messages").cloned().filter(Value::is_array).ok_or("no messages")?;
        self.ai_ensure_link().map_err(|f| failure_message(&f))?;
        self.ai.agent_seq += 1;
        let run = self.ai.agent_seq;
        let ctx = self.wv_agent_context_get();
        let mut params = json!({
            "run_id": run, "messages": messages, "page": ctx["page"], "tabs": ctx["tabs"],
            "model": p.get("model"), "loaded_skills": p.get("loaded_skills").cloned().unwrap_or_else(|| json!([])),
            "nav_mode": p.get("nav_mode"),
        });
        // The user edited the context in the pane: send exactly that, read nothing.
        let edited = p.get("context_override").and_then(Value::as_str).is_some();
        if edited {
            params["context_override"] = p["context_override"].clone();
        }
        self.ai.agent_run = run;
        self.api.clients.entry(1).or_default().observed.clear();
        if let Some((tab, document)) = self.active.and_then(|tab|
            self.ai.identity.current(tab.0).map(|document| (tab.0, document))) {
            self.wv_api_record_read(1, TabId(tab), document);
        }
        let question = messages.as_array().and_then(|m| m.iter().rev()
            .find(|m| m["role"] == "user")).and_then(|m| m["content"].as_str()).unwrap_or("");
        self.ai.ui.begin_agent(run, question, json!({
            "title": ctx["page"]["title"], "url": ctx["page"]["url"],
            "selectionChars": ctx["page"]["selection"].as_str().map_or(0, |s| s.chars().count()),
            "tabs": ctx["tabs"].as_array().map_or(0, Vec::len),
            "edited": edited,
        }));
        if edited {
            self.ai.agent_job_seq += 1;
            let job = self.ai.agent_job_seq;
            let tab = self.active.map(|tab| tab.0);
            let document = tab.and_then(|tab| self.ai.identity.current(tab));
            self.ai.agent_jobs.insert(job, crate::ai::session::AgentJob { tab, document, method: Method::AgentRun, params, pending: Pending::AgentRun(run), run: Some(run) });
            self.agent_job_done(job, None);
        } else {
            self.agent_send_with_content(Method::AgentRun, params, Pending::AgentRun(run), Some(run));
        }
        Ok(Some(json!({ "run_id": run, "context": ctx })))
    }

    /// Read the active page's content, add it to `params.page`, then send the request.
    /// A page that cannot be read (new tab, still loading, slow) is not an error: the
    /// request goes ahead without content and the agent can still `read_page`.
    fn agent_send_with_content(&mut self, method: Method, params: Value, pending: Pending, run: Option<u64>) {
        self.ai.agent_job_seq += 1;
        let job = self.ai.agent_job_seq;
        let tab = self.active.filter(|id| self.tabs.get(*id).is_some_and(|t| is_readable_url(&t.url)));
        let document = tab.and_then(|tab| self.ai.identity.current(tab.0));
        self.ai.agent_jobs.insert(job, crate::ai::session::AgentJob { tab: tab.map(|tab| tab.0), document, method, params, pending, run });
        let started = tab.and_then(|id| self.live.get(&id)).map(|live| {
            let proxy = self.proxy();
            live.read_content(CONTENT_SCRIPT, move |raw| {
                let _ = proxy.send_event(AppMessage::AgentJob(job, Some(raw)));
            })
        });
        match started {
            Some(Ok(())) => {
                let proxy = self.proxy();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(CONTENT_WAIT_MS));
                    let _ = proxy.send_event(AppMessage::AgentJob(job, None));
                });
            }
            _ => self.agent_job_done(job, None),
        }
    }

    pub(super) fn agent_job_done(&mut self, job: u64, raw: Option<String>) {
        let Some(mut job) = self.ai.agent_jobs.remove(&job) else { return };
        let current = job.tab.and_then(|tab| self.ai.identity.current(tab));
        let valid = current == job.document && job.tab.is_none_or(|tab| self.tabs.get(TabId(tab)).is_some());
        let content: Option<Value> = raw
            .and_then(|r| serde_json::from_str::<String>(&r).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .filter(|v: &Value| valid && v.get("error").is_none()
                && job.params["page"]["url"].as_str().is_some_and(|url| v["url"] == url));
        if let (Some(content), Some(page)) = (content, job.params.get_mut("page").and_then(Value::as_object_mut)) {
            let text = content.get("text").and_then(Value::as_str).unwrap_or("");
            if !text.trim().is_empty() {
                let total = text.chars().count();
                page.insert("content".into(), json!(text.chars().take(CONTENT_MAX_CHARS).collect::<String>()));
                page.insert("content_chars".into(), json!(total));
                page.insert("description".into(), content.get("description").cloned().unwrap_or(Value::Null));
                page.insert("headings".into(), content.get("headings").cloned().unwrap_or_else(|| json!([])));
                page.insert("forms".into(), content.get("forms").cloned().unwrap_or_else(|| json!([])));
                let rev = job.tab.and_then(|t| self.ai.page_snaps.get(&t)).map_or(0, |s| s.rev);
                page.insert("content_rev".into(), json!(rev));
                page.insert("document_token".into(), json!(job.document.map(|key| key.document_token().0)));
                page.insert("coverage".into(), content["coverage"].clone());
            }
        }
        if let Some(run) = job.run {
            let page = job.params.get("page").cloned().unwrap_or(Value::Null);
            self.emit(
                "agent.event",
                json!({ "run_id": run, "type": "context",
                        "content_chars": page.get("content_chars"),
                        "content_sent": page.get("content").and_then(Value::as_str).map(|s| s.chars().count()),
                        "headings": page.get("headings").and_then(Value::as_array).map_or(0, Vec::len) }),
            );
        }
        let control_req = if let Pending::Control(req) = &job.pending { *req } else { None };
        if let Err(f) = self.ai_request(job.method, &job.params, job.pending) {
            match (job.run, control_req) {
                (Some(run), _) => self.agent_run_failed(run, &f),
                (None, Some(req)) => self.agent_control_reply(Some(req), ResponsePayload::Failure(f)),
                (None, None) => {}
            }
        }
    }

    /// What the agent is given about the browser right now: the active page (with the
    /// selected text, if any) and the open tabs. Shown in the pane and sent with a run.
    pub(super) fn wv_agent_context_get(&self) -> Value {
        let active = self.active;
        let page = active.and_then(|id| self.tabs.get(id)).map(|t| {
            let selection = self.ai.selections.get(&t.id.0).map(|s| s.text.clone()).filter(|s| !s.trim().is_empty());
            json!({ "tab_id": t.id.0, "title": t.display_title(), "url": t.url, "selection": selection,
                "canvas_id": t.member_of.map(|c| c.0),
                "cell": t.member_of.and_then(|c| self.canvas_of(c)).and_then(|c| c.cell_of(t.id)),
                "document_token": self.ai.identity.current(t.id.0).map(|key| key.document_token().0) })
        });
        let tabs: Vec<Value> = self
            .tabs
            .tabs()
            .iter()
            .map(|t| self.wv_tab_describe(t))
            .collect();
        let content = active.and_then(|id| self.ai.page_snaps.get(&id.0)).filter(|s| s.rev > 0).map(|s| {
            json!({ "rev": s.rev, "chars": s.chars, "headings": s.headings, "fields": s.fields,
                    "filled": s.filled, "preview": s.preview, "at": s.at_ms })
        });
        json!({ "page": page, "tabs": tabs, "content": content })
    }

    pub(super) fn wv_tab_describe(&self, tab: &Tab) -> Value {
        json!({
            "tab_id": tab.id.0, "title": tab.display_title(), "url": tab.url,
            "active": self.active == Some(tab.id), "opened_by_agent": self.ai.ai_tabs.contains(&tab.id.0),
            "kind": if tab.canvas.is_some() { "canvas" } else { "page" },
            "canvas_id": tab.member_of.map(|c| c.0),
            "cell": tab.member_of.and_then(|c| self.canvas_of(c)).and_then(|c| c.cell_of(tab.id)),
            "focused_tab_id": tab.canvas.as_ref().and_then(|c| c.focused()).map(|id| id.0),
        })
    }

    /// Tell the pane what the model would be given changed (selection, active page).
    pub(super) fn agent_context_changed(&mut self) {
        let ctx = self.wv_agent_context_get();
        self.emit("context.changed", ctx);
    }

    /// The page said its content changed: re-read it (one read at a time; a change
    /// during a read schedules exactly one more) and tell the pane if it differs.
    pub(super) fn page_read_start(&mut self, tab: TabId) {
        let readable = self.tabs.get(tab).is_some_and(|t| is_readable_url(&t.url));
        if !readable || !self.live.contains_key(&tab) {
            return;
        }
        let snap = self.ai.page_snaps.entry(tab.0).or_default();
        if snap.reading {
            snap.dirty = true;
            return;
        }
        snap.reading = true;
        snap.read_seq += 1;
        let sequence = snap.read_seq;
        let document = self.ai.identity.current(tab.0);
        let proxy = self.proxy();
        let started = self.live.get(&tab).map(|live| {
            live.read_content(CONTENT_SCRIPT, move |raw| {
                let _ = proxy.send_event(AppMessage::PageRead(tab, document, sequence, Some(raw)));
            })
        });
        if !matches!(started, Some(Ok(()))) {
            self.page_read_done(tab, document, sequence, None);
        } else {
            let proxy = self.proxy();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(CONTENT_WAIT_MS));
                let _ = proxy.send_event(AppMessage::PageRead(tab, document, sequence, None));
            });
        }
    }

    pub(super) fn page_read_done(&mut self, tab: TabId, document: Option<ai_platform_contract::identity::DocumentKey>, sequence: u64, raw: Option<String>) {
        if self.tabs.get(tab).is_none() || self.ai.identity.current(tab.0) != document {
            return;
        }
        let content: Option<Value> = raw
            .and_then(|r| serde_json::from_str::<String>(&r).ok())
            .and_then(|s| serde_json::from_str(&s).ok());
        if self.has_native_pdf(tab) {
            if let Some(error) = content.as_ref().and_then(|value| value["error"].as_str()) {
                self.note(format!("Native PDF text read for tab {}: {error}", tab.0));
            }
        }
        let content = content.filter(|v| v.get("error").is_none());
        #[cfg(windows)]
        if content.is_some() { self.sync_native_pdf_zoom(tab); }
        let Some(snap) = self.ai.page_snaps.get_mut(&tab.0)
            .filter(|snap| snap.reading && snap.read_seq == sequence) else { return };
        snap.reading = false;
        let again = std::mem::take(&mut snap.dirty);
        let mut changed = false;
        if let Some(c) = content {
            let text = c.get("text").and_then(Value::as_str).unwrap_or("");
            let forms = c.get("forms").and_then(Value::as_array).cloned().unwrap_or_default();
            let forms_text = serde_json::to_string(&forms).unwrap_or_default();
            let hash = crate::ai::session::fingerprint(&[text, &forms_text]);
            if hash != snap.hash {
                snap.hash = hash;
                snap.rev += 1;
                snap.chars = text.chars().count();
                snap.headings = c.get("headings").and_then(Value::as_array).map_or(0, Vec::len);
                snap.fields = forms.len();
                snap.filled = forms
                    .iter()
                    .filter(|f| f.get("value").and_then(Value::as_str).is_some_and(|v| !v.is_empty() && v != "unchecked" && v != "(hidden)"))
                    .count();
                snap.preview = text.chars().take(160).collect();
                snap.at_ms = crate::ai::session::now_ms();
                changed = true;
            }
        }
        if changed && self.active == Some(tab) {
            self.agent_context_changed();
        }
        if again {
            self.page_read_start(tab);
        }
    }

    pub(super) fn wv_agent_stop(&mut self) -> Result<Option<Value>, String> {
        let run = self.ai.agent_run;
        if run == 0 {
            return Ok(Some(json!({ "stopping": false })));
        }
        let queued: Vec<u64> = self.api.evals.iter()
            .filter(|(_, eval)| eval.run == Some(run)).map(|(id, _)| *id).collect();
        for id in queued {
            self.api.evals.remove(&id);
            self.wv_reply_err(id, "the user stopped the run");
        }
        // Still reading the page: nothing has reached the engine yet, so just drop it.
        let waiting: Vec<u64> = self.ai.agent_jobs.iter().filter(|(_, j)| j.run == Some(run)).map(|(id, _)| *id).collect();
        if !waiting.is_empty() {
            for id in waiting {
                self.ai.agent_jobs.remove(&id);
            }
            self.ai.agent_run = 0;
            self.emit("agent.event", json!({ "run_id": run, "type": "error", "stopped": true, "message": "Stopped." }));
            return Ok(Some(json!({ "stopping": true })));
        }
        let params = json!({ "op": "cancel", "run_id": run });
        self.ai_request(Method::AgentControl, &params, Pending::Control(None)).map_err(|f| failure_message(&f))?;
        Ok(Some(json!({ "stopping": true })))
    }

    pub(super) fn wv_agent_approval_resolve(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let params = json!({
            "op": "approve", "run_id": self.ai.agent_run,
            "call_id": p.get("call_id").and_then(Value::as_str).unwrap_or(""),
            "allow": p.get("allow").and_then(Value::as_bool).unwrap_or(false),
        });
        self.ai_request(Method::AgentControl, &params, Pending::Control(None)).map_err(|f| failure_message(&f))?;
        Ok(None)
    }

    /// A canonical management operation; the utility's reply completes its MCP call.
    pub(super) fn wv_agent_manage(&mut self, method: &str, p: &Value, source: crate::ipc::Surface) -> Result<Option<Value>, String> {
        let crate::ipc::Surface::ApiCall(req) = source else { return Err("management operations require an MCP request".into()) };
        self.wv_api_defer(req);
        let mut params = p.clone();
        let mut preview = false;
        if let Some(map) = params.as_object_mut() {
            map.insert("method".into(), json!(method));
            if method == "shell_agent" && map.get("operation").and_then(Value::as_str) == Some("context_preview") {
                let ctx = self.wv_agent_context_get();
                map.insert("page".into(), ctx["page"].clone());
                map.insert("tabs".into(), ctx["tabs"].clone());
                preview = true;
            }
        }
        self.ai_ensure_link().map_err(|f| failure_message(&f))?;
        if preview {
            // The preview shows the page content the model will get, so it reads the page too.
            self.agent_send_with_content(Method::AgentControl, params, Pending::Control(Some(req)), None);
            return Ok(None);
        }
        self.ai_request(Method::AgentControl, &params, Pending::Control(Some(req))).map_err(|f| failure_message(&f))?;
        Ok(None)
    }

    pub(super) fn agent_control_reply(&mut self, request: Option<u64>, payload: ResponsePayload) {
        if let Some(id) = request {
            match payload {
                ResponsePayload::Ok(value) => self.wv_reply_ok(id, value),
                ResponsePayload::Failure(failure) => self.wv_reply_err(id, failure_message(&failure)),
            }
        }
    }

    pub(super) fn agent_run_failed(&mut self, run: u64, failure: &Failure) {
        if self.ai.agent_run == run {
            self.ai.agent_run = 0;
        }
        self.emit(
            "agent.event",
            json!({ "run_id": run, "type": "error", "code": failure.code, "message": failure_message(failure) }),
        );
    }

    pub(super) fn agent_link_closed(&mut self) {
        let run = std::mem::take(&mut self.ai.agent_run);
        let pending: Vec<_> = self.api.pending.iter().filter(|(_, pending)| pending.caller.builtin && pending.caller.id != 0).map(|(id, _)| *id).collect();
        for id in pending {
            self.api.evals.remove(&id);
            self.api.site_data.remove(&id);
            self.wv_reply_err(id, "the AI process stopped");
        }
        self.ai.agent_jobs.clear();
        if run != 0 {
            self.emit(
                "agent.event",
                json!({ "run_id": run, "type": "error", "code": "utility.exited",
                        "message": "The AI process stopped. The browser is unaffected; try again." }),
            );
        }
    }

    // ---- agent → browser ------------------------------------------------

    /// Forward a run event to the sidebar; a `done`/`error` frees the slot.
    pub(super) fn agent_on_event(&mut self, event: Value) {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(kind, "done" | "error") {
            let run = event.get("run_id").and_then(Value::as_u64).unwrap_or(0);
            if self.ai.agent_run == run {
                self.ai.agent_run = 0;
            }
        }
        self.emit("agent.event", event);
    }

}
