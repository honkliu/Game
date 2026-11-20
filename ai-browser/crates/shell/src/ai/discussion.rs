use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const MAX_DISCUSSIONS: usize = 32;
const MAX_TURNS: usize = 40;
const MAX_MESSAGE_BYTES: usize = 32 * 1024;
const MAX_CONTEXT_BYTES: usize = 256 * 1024;
const ANSWER_RESERVE: usize = 128_000 * 4;
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub struct Draft {
    pub id: String,
    pub turn_id: u64,
    pub participant: String,
    pub context: Value,
    pub messages: Value,
    pub reuse_context: bool,
    owner: u64,
    request_id: String,
    message: String,
    refresh: bool,
}

struct Turn {
    id: u64,
    run: u64,
    request_id: String,
    message: String,
    refresh: bool,
    status: &'static str,
    answer: String,
    error: Option<String>,
    context: Value,
}

struct Discussion {
    owner: u64,
    participant: String,
    context: Value,
    turns: Vec<Turn>,
    touched: Instant,
}

struct Waiter {
    request: u64,
    owner: u64,
    discussion: String,
    turn: u64,
    until: Instant,
}

#[derive(Default)]
pub struct Discussions {
    discussions: HashMap<String, Discussion>,
    waiters: Vec<Waiter>,
}

fn summary(context: &Value) -> Value {
    let page = &context["page"];
    let mut result = serde_json::Map::new();
    for key in [
        "tab_id",
        "title",
        "url",
        "document_token",
        "selection",
        "canvas_id",
        "cell",
    ] {
        if let Some(value) = page.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    Value::Object(result)
}

impl Discussions {
    fn discussion(&self, owner: u64, id: &str) -> Result<&Discussion, String> {
        self.discussions
            .get(id)
            .filter(|d| d.owner == owner)
            .ok_or_else(|| "Discussion not found for this MCP client".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(selection: &str) -> Value {
        json!({"page":{"tab_id":7,"document_token":9,"url":"https://example.test/","title":"Source","selection":selection},"tabs":[]})
    }

    fn start(state: &mut Discussions) {
        let draft = state
            .prepare(
                1,
                "discussion",
                "request-1",
                "Explain this",
                "VSCode",
                context("original"),
                false,
            )
            .unwrap();
        state.start(draft, 10);
    }

    #[test]
    fn followups_reuse_answers_and_selection_until_context_is_explicitly_refreshed() {
        let mut state = Discussions::default();
        start(&mut state);
        state
            .capture(
                10,
                &json!({"tab_id":7,"selection":"original","content":"captured article"}),
                &json!([]),
            )
            .unwrap();
        state.complete(
            10,
            &json!({"status":"done","text":"River's first answer"}),
            false,
        );
        let second = state
            .prepare(
                1,
                "discussion",
                "request-2",
                "Why?",
                "ignored label",
                context("different"),
                false,
            )
            .unwrap();
        assert!(second.reuse_context);
        assert_eq!(second.context["page"]["selection"], "original");
        assert_eq!(second.context["page"]["content"], "captured article");
        assert_eq!(second.participant, "VSCode");
        assert_eq!(
            second.messages,
            json!([
                {"role":"user","content":"Explain this"},
                {"role":"assistant","content":"River's first answer"},
                {"role":"user","content":"Why?"}
            ])
        );
        state.start(second, 11);
        state.complete(
            11,
            &json!({"status":"done","text":"River's follow-up"}),
            false,
        );
        let third = state
            .prepare(
                1,
                "discussion",
                "request-3",
                "Use the new selection",
                "VSCode",
                context("different"),
                true,
            )
            .unwrap();
        assert!(!third.reuse_context);
        assert_eq!(third.context["page"]["selection"], "different");
        state.start(third, 12);
        assert_eq!(
            state.result(1, "discussion", Some(1)).unwrap()["context"]["selection"],
            "original"
        );
        assert_eq!(
            state.result(1, "discussion", Some(2)).unwrap()["answer"],
            "River's follow-up"
        );
    }

    #[test]
    fn oversized_capture_fails_instead_of_silently_discarding_the_pinned_article() {
        let mut state = Discussions::default();
        start(&mut state);
        let page =
            json!({"tab_id":7,"selection":"original","content":"x".repeat(MAX_CONTEXT_BYTES)});
        assert!(state.capture(10, &page, &json!([])).is_err());
        assert!(state.discussions["discussion"].context["page"]["content"].is_null());
        assert!(state.capture(99, &page, &json!([])).is_ok());
    }

    #[test]
    fn request_ids_recover_the_same_turn_and_other_clients_cannot_read_or_stop_it() {
        let mut state = Discussions::default();
        start(&mut state);
        assert_eq!(
            state
                .existing(1, "request-1", None, "Explain this", false)
                .unwrap(),
            Some(("discussion".into(), 1))
        );
        assert!(state
            .existing(1, "request-1", None, "Different request", false)
            .is_err());
        assert!(state
            .existing(
                1,
                "request-1",
                Some("other discussion"),
                "Explain this",
                false
            )
            .is_err());
        assert!(state.result(2, "discussion", None).is_err());
        assert!(state.running_run(2, "discussion", None).is_err());
        assert!(state
            .prepare(
                2,
                "discussion",
                "request-2",
                "Hijack",
                "Other",
                context("new"),
                false
            )
            .is_err());
        assert!(state
            .prepare(
                1,
                "discussion",
                "request-2",
                "Follow up too soon",
                "VSCode",
                context("new"),
                false
            )
            .is_err());
        state.complete(10, &json!({"status":"done","text":"done"}), false);
        assert_eq!(
            state
                .existing(1, "request-1", Some("discussion"), "Explain this", false)
                .unwrap(),
            Some(("discussion".into(), 1))
        );
        assert_eq!(state.discussions["discussion"].turns.len(), 1);
    }

    #[test]
    fn waiting_times_out_without_cancelling_and_all_waiters_receive_the_ui_answer() {
        let mut state = Discussions::default();
        start(&mut state);
        let now = Instant::now();
        let duration = Duration::from_secs(10);
        assert!(state
            .wait(1, "discussion", None, 100, duration, now)
            .unwrap()
            .is_none());
        assert!(state
            .ready(now + Duration::from_secs(9), |_| true)
            .is_empty());
        let timed = state.ready(now + duration, |_| true);
        assert_eq!(timed[0].0, 100);
        assert_eq!(timed[0].1.as_ref().unwrap()["status"], "running");
        assert_eq!(state.running_run(1, "discussion", None).unwrap(), Some(10));
        for request in [101, 102, 103] {
            state
                .wait(1, "discussion", Some(1), request, duration, now + duration)
                .unwrap();
        }
        let ui = json!({"status":"done","text":"Exactly the text displayed by River"});
        state.complete(10, &ui, false);
        let ready = state.ready(now + duration, |id| id != 103);
        assert_eq!(ready.len(), 2);
        assert!(ready
            .iter()
            .all(|(_, result)| result.as_ref().unwrap()["answer"] == ui["text"]));
        assert!(state
            .wait(1, "discussion", None, 104, duration, now)
            .unwrap()
            .is_some());
        state.complete(10, &json!({"status":"error","error":"late event"}), false);
        assert_eq!(
            state.result(1, "discussion", None).unwrap()["status"],
            "completed"
        );
    }

    #[test]
    fn stop_and_failure_are_terminal_results_and_followups_do_not_invent_an_answer() {
        let mut state = Discussions::default();
        start(&mut state);
        state.stopping(10);
        assert_eq!(
            state.result(1, "discussion", None).unwrap()["status"],
            "stopping"
        );
        state.complete(10, &json!({"status":"error","error":"Stopped."}), true);
        let result = state.result(1, "discussion", None).unwrap();
        assert_eq!(result["status"], "stopped");
        assert!(result["answer"].is_null());
        let next = state
            .prepare(
                1,
                "discussion",
                "request-2",
                "Try again",
                "VSCode",
                context("new"),
                false,
            )
            .unwrap();
        assert!(next.messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("stopped"));
        state.start(next, 11);
        state.complete(
            11,
            &json!({"status":"error","error":"Provider unavailable"}),
            false,
        );
        assert_eq!(
            state.result(1, "discussion", None).unwrap()["status"],
            "failed"
        );
        assert!(state
            .prepare(
                1,
                "discussion",
                "",
                "Question",
                "VSCode",
                context("new"),
                false
            )
            .is_err());
    }
}

impl Discussions {
    pub fn existing(
        &self,
        owner: u64,
        request: &str,
        id: Option<&str>,
        message: &str,
        refresh: bool,
    ) -> Result<Option<(String, u64)>, String> {
        for (key, discussion) in &self.discussions {
            if discussion.owner != owner {
                continue;
            }
            if let Some(turn) = discussion.turns.iter().find(|t| t.request_id == request) {
                if turn.message != message
                    || turn.refresh != refresh
                    || id.is_some_and(|id| id != key)
                {
                    return Err(
                        "request_id was already used for a different discussion message".into(),
                    );
                }
                return Ok(Some((key.clone(), turn.id)));
            }
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        owner: u64,
        id: &str,
        request_id: &str,
        message: &str,
        participant: &str,
        current: Value,
        refresh: bool,
    ) -> Result<Draft, String> {
        if request_id.is_empty()
            || request_id.len() > 128
            || !request_id.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("discuss requires a unique printable request_id of 1 to 128 characters; reuse it only when retrying the same turn".into());
        }
        if message.trim().is_empty() || message.len() > MAX_MESSAGE_BYTES {
            return Err("Discussion message must contain 1 to 32768 UTF-8 bytes".into());
        }
        if participant.trim().is_empty()
            || participant.chars().count() > 64
            || participant.chars().any(char::is_control)
        {
            return Err("participant must contain 1 to 64 visible characters".into());
        }
        let previous = self.discussions.get(id);
        if previous.is_some_and(|d| d.owner != owner) {
            return Err("Discussion not found for this MCP client".into());
        }
        if previous.is_some_and(|d| {
            d.turns
                .iter()
                .any(|t| matches!(t.status, "running" | "stopping"))
        }) {
            return Err(
                "Wait for the current discussion turn before sending another message".into(),
            );
        }
        if previous.is_some_and(|d| d.turns.len() >= MAX_TURNS) {
            return Err("Discussion history is full; start a new discussion".into());
        }
        let reuse_context = previous.is_some() && !refresh;
        let context = if reuse_context {
            previous.unwrap().context.clone()
        } else {
            current
        };
        let context_bytes = serde_json::to_vec(&context)
            .map_err(|e| e.to_string())?
            .len();
        if context_bytes > MAX_CONTEXT_BYTES {
            return Err("Discussion context is too large; reduce the open-page context".into());
        }
        let participant = previous
            .map(|d| d.participant.clone())
            .unwrap_or_else(|| participant.to_string());
        let mut messages = Vec::new();
        if let Some(discussion) = previous {
            for turn in &discussion.turns {
                messages.push(json!({"role":"user","content":turn.message}));
                let answer = if turn.status == "completed" {
                    turn.answer.clone()
                } else {
                    format!(
                        "River's previous turn {}: {}",
                        turn.status,
                        turn.error.as_deref().unwrap_or("No completed answer")
                    )
                };
                messages.push(json!({"role":"assistant","content":answer}));
            }
        }
        messages.push(json!({"role":"user","content":message}));
        let turn_id = previous.map_or(1, |d| d.turns.len() as u64 + 1);
        let reserve = MAX_CONTEXT_BYTES
            + ANSWER_RESERVE
            + message.len()
            + summary(&context).to_string().len();
        while self.bytes() + reserve > MAX_BYTES
            || (!self.discussions.contains_key(id) && self.discussions.len() >= MAX_DISCUSSIONS)
        {
            let oldest = self
                .discussions
                .iter()
                .filter(|(key, discussion)| {
                    key.as_str() != id
                        && !discussion
                            .turns
                            .iter()
                            .any(|t| matches!(t.status, "running" | "stopping"))
                })
                .min_by_key(|(_, d)| d.touched)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                return Err("Discussion storage is full; start a shorter discussion".into());
            };
            self.discussions.remove(&oldest);
        }
        Ok(Draft {
            id: id.into(),
            turn_id,
            participant,
            context,
            messages: json!(messages),
            reuse_context,
            owner,
            request_id: request_id.into(),
            message: message.into(),
            refresh,
        })
    }

    fn bytes(&self) -> usize {
        self.discussions
            .values()
            .map(|d| {
                d.context.to_string().len()
                    + d.turns
                        .iter()
                        .map(|t| {
                            t.message.len()
                                + t.answer.len()
                                + t.context.to_string().len()
                                + t.request_id.len()
                                + t.error.as_ref().map_or(0, String::len)
                        })
                        .sum::<usize>()
            })
            .sum()
    }

    pub fn start(&mut self, draft: Draft, run: u64) {
        let discussion = self
            .discussions
            .entry(draft.id)
            .or_insert_with(|| Discussion {
                owner: draft.owner,
                participant: draft.participant,
                context: Value::Null,
                turns: Vec::new(),
                touched: Instant::now(),
            });
        discussion.context = draft.context;
        discussion.touched = Instant::now();
        discussion.turns.push(Turn {
            id: draft.turn_id,
            run,
            request_id: draft.request_id,
            message: draft.message,
            refresh: draft.refresh,
            status: "running",
            answer: String::new(),
            error: None,
            context: summary(&discussion.context),
        });
    }

    pub fn capture(&mut self, run: u64, page: &Value, tabs: &Value) -> Result<(), String> {
        if let Some(discussion) = self
            .discussions
            .values_mut()
            .find(|d| d.turns.last().is_some_and(|t| t.run == run))
        {
            let context = json!({"page":page,"tabs":tabs});
            if context.to_string().len() > MAX_CONTEXT_BYTES {
                return Err("The captured discussion context is too large; reduce the open-page context and send a new turn with refresh_context: true".into());
            }
            discussion.context = context;
        }
        Ok(())
    }

    pub fn contains_run(&self, run: u64) -> bool {
        self.discussions
            .values()
            .any(|d| d.turns.iter().any(|t| t.run == run))
    }

    pub fn complete(&mut self, run: u64, ui: &Value, stopped: bool) {
        for discussion in self.discussions.values_mut() {
            let Some(turn) = discussion.turns.iter_mut().find(|t| t.run == run) else {
                continue;
            };
            if !matches!(turn.status, "running" | "stopping") {
                return;
            }
            match ui["status"].as_str() {
                Some("done") => {
                    turn.status = "completed";
                    turn.answer = ui["text"].as_str().unwrap_or("").into();
                }
                Some("error") => {
                    turn.status = if stopped { "stopped" } else { "failed" };
                    turn.error = Some(
                        ui["error"]
                            .as_str()
                            .unwrap_or("River's agent failed")
                            .into(),
                    );
                }
                _ => return,
            }
            discussion.touched = Instant::now();
            return;
        }
    }

    pub fn result(&self, owner: u64, id: &str, turn: Option<u64>) -> Result<Value, String> {
        let discussion = self.discussion(owner, id)?;
        let turn = match turn {
            Some(id) => discussion.turns.iter().find(|t| t.id == id),
            None => discussion.turns.last(),
        }
        .ok_or("Discussion turn not found")?;
        let running = matches!(turn.status, "running" | "stopping");
        Ok(
            json!({"discussion_id":id,"turn_id":turn.id,"run_id":turn.run,"participant":discussion.participant,
            "assistant":"River","status":turn.status,"answer":if turn.status == "completed" { json!(turn.answer) } else { Value::Null },
            "error":turn.error,"context":turn.context,"context_pinned":true,
            "next_operation":if running { json!("discussion_wait") } else { Value::Null }}),
        )
    }

    pub fn running_run(
        &self,
        owner: u64,
        id: &str,
        turn: Option<u64>,
    ) -> Result<Option<u64>, String> {
        let result = self.result(owner, id, turn)?;
        Ok(
            matches!(result["status"].as_str(), Some("running" | "stopping"))
                .then(|| result["run_id"].as_u64().unwrap()),
        )
    }

    pub fn stopping(&mut self, run: u64) {
        for discussion in self.discussions.values_mut() {
            if let Some(turn) = discussion
                .turns
                .iter_mut()
                .find(|t| t.run == run && t.status == "running")
            {
                turn.status = "stopping";
            }
        }
    }

    pub fn wait(
        &mut self,
        owner: u64,
        id: &str,
        turn: Option<u64>,
        request: u64,
        duration: Duration,
        now: Instant,
    ) -> Result<Option<Value>, String> {
        let result = self.result(owner, id, turn)?;
        if duration.is_zero() || !matches!(result["status"].as_str(), Some("running" | "stopping"))
        {
            return Ok(Some(result));
        }
        self.waiters.push(Waiter {
            request,
            owner,
            discussion: id.into(),
            turn: result["turn_id"].as_u64().unwrap(),
            until: now + duration,
        });
        Ok(None)
    }

    pub fn ready(
        &mut self,
        now: Instant,
        pending: impl Fn(u64) -> bool,
    ) -> Vec<(u64, Result<Value, String>)> {
        let mut ready = Vec::new();
        for waiter in std::mem::take(&mut self.waiters) {
            if !pending(waiter.request) {
                continue;
            }
            let result = self.result(waiter.owner, &waiter.discussion, Some(waiter.turn));
            let finished = result.as_ref().map_or(true, |v| {
                !matches!(v["status"].as_str(), Some("running" | "stopping"))
            });
            if finished || now >= waiter.until {
                ready.push((waiter.request, result));
            } else {
                self.waiters.push(waiter);
            }
        }
        ready
    }
}
