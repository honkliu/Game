use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::AppState;

fn wait_duration(params: &Value) -> Result<Duration, String> {
    let ms = match params.get("wait_ms") {
        Some(value) => value
            .as_u64()
            .filter(|ms| *ms <= 45_000)
            .ok_or("wait_ms must be between 0 and 45000")?,
        None => 25_000,
    };
    Ok(Duration::from_millis(ms))
}

fn turn_id(params: &Value) -> Result<Option<u64>, String> {
    params
        .get("turn_id")
        .map(|value| {
            value
                .as_u64()
                .filter(|id| *id != 0)
                .ok_or_else(|| "turn_id must be a positive integer".into())
        })
        .transpose()
}

impl AppState {
    pub(super) fn wv_agent_discuss(&mut self, params: &Value) -> Result<Option<Value>, String> {
        let duration = wait_duration(params)?;
        let owner = self.wv_api_caller().id;
        let request_id = params["request_id"]
            .as_str()
            .ok_or("discuss requires request_id; reuse it when retrying the same turn")?;
        let message = params["message"]
            .as_str()
            .ok_or("discuss requires message")?;
        let id = params
            .get("discussion_id")
            .map(|id| id.as_str().ok_or("discussion_id must be a string"))
            .transpose()?;
        let refresh = params
            .get("refresh_context")
            .map(|v| v.as_bool().ok_or("refresh_context must be a boolean"))
            .transpose()?
            .unwrap_or(false);
        if let Some((id, turn)) = self
            .ai
            .discussions
            .existing(owner, request_id, id, message, refresh)?
        {
            return self.discussion_wait_for(owner, &id, Some(turn), duration);
        }
        if let Some(id) = id {
            self.ai.discussions.result(owner, id, None)?;
        }
        if self.ai.agent_run != 0 || !self.ai.tasks.is_empty() {
            return Err("River is already working; wait or stop the current task before starting a discussion turn".into());
        }
        let id = id
            .map(str::to_string)
            .map(Ok)
            .unwrap_or_else(crate::api_server::random_token)?;
        let participant = params
            .get("participant")
            .map(|v| v.as_str().ok_or("participant must be a string"))
            .transpose()?
            .unwrap_or("VSCode");
        let current = self.wv_agent_context_get();
        let draft = self.ai.discussions.prepare(
            owner,
            &id,
            request_id,
            message,
            participant,
            current,
            refresh,
        )?;
        let turn = draft.turn_id;
        let metadata = json!({"id":id,"turn_id":turn,"participant":draft.participant});
        let mut context = draft.context.clone();
        context["discussion"] = json!({"id":id,"turn_id":turn,"reuse_context":draft.reuse_context,"context_pinned":true});
        let args = json!({"messages":draft.messages,"model":params.get("model"),"nav_mode":params.get("nav_mode")});
        self.ai_show_pane();
        let result = self
            .wv_agent_start(&args, context, draft.reuse_context)?
            .ok_or("River did not return a run ID")?;
        let run = result["run_id"]
            .as_u64()
            .ok_or("River did not return a run ID")?;
        self.ai.discussions.start(draft, run);
        self.ai.ui.label_discussion(run, metadata);
        if let Some(snapshot) = self.ai.ui.agent_turn(run).cloned() {
            self.ai.discussions.complete(run, &snapshot, false);
            self.emit(
                "agent.event",
                json!({"type":"queued","run_id":run,"turn":snapshot}),
            );
        }
        self.discussion_wait_for(owner, &id, Some(turn), duration)
    }

    pub(super) fn wv_agent_discussion_wait(
        &mut self,
        params: &Value,
    ) -> Result<Option<Value>, String> {
        let id = params["discussion_id"]
            .as_str()
            .ok_or("discussion_id is required")?;
        self.discussion_wait_for(
            self.wv_api_caller().id,
            id,
            turn_id(params)?,
            wait_duration(params)?,
        )
    }

    fn discussion_wait_for(
        &mut self,
        owner: u64,
        id: &str,
        turn: Option<u64>,
        duration: Duration,
    ) -> Result<Option<Value>, String> {
        let request = self.api.current;
        if !self.api.pending.contains_key(&request) {
            return Err("Discussion operations require an MCP request".into());
        }
        let result =
            self.ai
                .discussions
                .wait(owner, id, turn, request, duration, Instant::now())?;
        if result.is_none() {
            self.wv_api_defer(request);
        }
        Ok(result)
    }

    pub(super) fn wv_agent_discussion_stop(
        &mut self,
        params: &Value,
    ) -> Result<Option<Value>, String> {
        let id = params["discussion_id"]
            .as_str()
            .ok_or("discussion_id is required")?;
        let owner = self.wv_api_caller().id;
        let turn = turn_id(params)?;
        let duration = wait_duration(params)?;
        if let Some(run) = self.ai.discussions.running_run(owner, id, turn)? {
            if self.ai.agent_run != run {
                return Err("The discussion run is no longer active".into());
            }
            self.wv_agent_stop()?;
            self.ai.discussions.stopping(run);
        }
        self.discussion_wait_for(owner, id, turn, duration)
    }

    pub(super) fn discussion_finished(&mut self, event: &Value) {
        let Some(run) = event["run_id"].as_u64() else {
            return;
        };
        if let Some(turn) = self.ai.ui.agent_turn(run) {
            self.ai
                .discussions
                .complete(run, turn, event["stopped"] == true);
        }
    }

    pub(super) fn discussion_flush(&mut self) {
        let pending = &self.api.pending;
        let ready = self
            .ai
            .discussions
            .ready(Instant::now(), |id| pending.contains_key(&id));
        for (request, result) in ready {
            match result {
                Ok(value) => self.wv_reply_ok(request, value),
                Err(error) => self.wv_reply_err(request, error),
            }
        }
    }
}
