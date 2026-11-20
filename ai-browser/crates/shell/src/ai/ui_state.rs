//! Bounded, host-owned transcript state for UI reload/reconnection.

use std::collections::VecDeque;

use serde_json::{json, Value};

const MAX_TURNS: usize = 40;
const MAX_TEXT: usize = 128_000;

#[derive(Default)]
pub struct UiState {
    sequence: u64,
    turns: VecDeque<Value>,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").chars().take(MAX_TEXT).collect()
}

fn append(value: &mut Value, key: &str, part: &str) {
    let mut current = text(value, key);
    current.extend(part.chars().take(MAX_TEXT.saturating_sub(current.chars().count())));
    value[key] = json!(current);
}

impl UiState {
    pub fn begin_agent(&mut self, run: u64, question: &str, echo: Value) {
        self.sequence += 1;
        self.turns.push_back(json!({
            "kind": "agent", "runId": run, "question": question,
            "status": "running", "thinking": [], "tools": [], "text": "",
            "rounds": 0, "echo": echo,
        }));
        self.trim();
    }

    fn trim(&mut self) {
        while self.turns.len() > MAX_TURNS {
            let Some(index) = self.turns.iter().position(|t| !matches!(t["status"].as_str(), Some("running" | "pending"))) else {
                break;
            };
            self.turns.remove(index);
        }
    }

    pub fn clear_finished(&mut self) {
        self.sequence += 1;
        self.turns.retain(|t| matches!(t["status"].as_str(), Some("running" | "pending")));
    }

    pub fn snapshot(&self) -> Value {
        json!({ "sequence": self.sequence, "turns": self.turns })
    }

    pub fn agent_turn(&self, run: u64) -> Option<&Value> {
        self.turns.iter().find(|turn| turn["runId"] == run)
    }

    pub fn label_discussion(&mut self, run: u64, discussion: Value) {
        if let Some(turn) = self.turns.iter_mut().find(|turn| turn["runId"] == run) {
            turn["discussion"] = discussion;
        }
    }

    pub fn record(&mut self, event: &str, payload: &mut Value) {
        if !matches!(event, "agent.event" | "ai.answer" | "ai.delta" | "ai.stage" | "ai.answer.stale") {
            return;
        }
        self.sequence += 1;
        payload["state_seq"] = json!(self.sequence);
        if event == "agent.event" {
            let run = payload["run_id"].as_u64().unwrap_or(0);
            let Some(turn) = self.turns.iter_mut().find(|t| t["runId"] == run) else { return };
            match payload["type"].as_str().unwrap_or("") {
                "round" => turn["rounds"] = payload["n"].clone(),
                "delta" => append(turn, "text", &text(payload, "text")),
                "tool_start" => {
                    let thinking = text(turn, "text");
                    if !thinking.trim().is_empty() {
                        let notes = turn["thinking"].as_array_mut().unwrap();
                        notes.push(json!(thinking.chars().take(4000).collect::<String>()));
                        if notes.len() > 16 { notes.remove(0); }
                    }
                    turn["text"] = json!("");
                    let tools = turn["tools"].as_array_mut().unwrap();
                    tools.push(json!({
                        "id": payload["call_id"], "name": payload["name"],
                        "args": text(payload, "args").chars().take(2000).collect::<String>(),
                        "state": "running",
                    }));
                    if tools.len() > 64 { tools.remove(0); }
                }
                "approval" | "tool_end" => {
                    if let Some(tool) = turn["tools"].as_array_mut().unwrap().iter_mut()
                        .find(|t| t["id"] == payload["call_id"]) {
                        if payload["type"] == "approval" {
                            tool["state"] = json!("waiting");
                            tool["approval"] = json!({
                                "call_id": payload["call_id"], "name": payload["name"],
                                "args": text(payload, "args"), "server": payload["server"],
                            });
                        } else {
                            tool["state"] = json!("done");
                            tool["ok"] = payload["ok"].clone();
                            tool["ms"] = payload["ms"].clone();
                            tool["preview"] = json!(text(payload, "preview").chars().take(2000).collect::<String>());
                        }
                    }
                }
                "context" => {
                    turn["echo"]["contentChars"] = payload["content_chars"].clone();
                    turn["echo"]["contentSent"] = payload["content_sent"].clone();
                    turn["echo"]["headings"] = payload["headings"].clone();
                }
                "link" => {
                    if !turn["links"].is_array() { turn["links"] = json!([]); }
                    let links = turn["links"].as_array_mut().unwrap();
                    if links.len() < 80 && !links.iter().any(|l| l["url"] == payload["url"]) {
                        links.push(json!({ "id": payload["call_id"], "url": payload["url"] }));
                    }
                }
                "done" => {
                    turn["status"] = json!("done");
                    turn["text"] = json!(text(payload, "text"));
                    if payload["rounds"].is_number() { turn["rounds"] = payload["rounds"].clone(); }
                }
                "error" => {
                    turn["status"] = json!("error");
                    turn["error"] = json!(if payload["stopped"] == true { "Stopped.".into() } else { text(payload, "message") });
                }
                _ => {}
            }
        } else if event == "ai.answer.stale" {
            if let Some(ids) = payload["turn_ids"].as_array() {
                for turn in self.turns.iter_mut().filter(|t| ids.contains(&t["turnId"])) {
                    turn["stale"] = json!(true);
                }
            }
        } else {
            let id = payload["turn_id"].as_u64().unwrap_or(0);
            if event == "ai.answer" && !self.turns.iter().any(|t| t["turnId"] == id) {
                self.turns.push_back(json!({
                    "kind": "ask", "turnId": id, "status": "pending", "question": "",
                    "streamed": "", "text": "", "citations": [], "stale": false,
                    "provider": "offline", "model": "", "elapsedMs": 0, "failure": null,
                }));
            }
            let Some(turn) = self.turns.iter_mut().find(|t| t["turnId"] == id) else { return };
            match event {
                "ai.answer" => {
                    for field in ["question", "status", "text", "provider", "model", "citations", "failure"] {
                        if !payload[field].is_null() { turn[field] = payload[field].clone(); }
                    }
                    turn["elapsedMs"] = payload["elapsed_ms"].clone();
                    if payload["stale"] == true { turn["stale"] = json!(true); }
                }
                "ai.delta" => append(turn, "streamed", &text(payload, "text")),
                "ai.stage" if payload["stage"] == "routed" => {
                    turn["routeLabel"] = json!(text(payload, "model"));
                }
                _ => {}
            }
        }
        self.trim();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_agent_state_survives_without_a_ui_and_clears_for_new_chat() {
        let mut state = UiState::default();
        state.begin_agent(7, "question", json!({}));
        state.record("agent.event", &mut json!({ "run_id": 7, "type": "delta", "text": "partial" }));
        state.record("agent.event", &mut json!({ "run_id": 7, "type": "done", "text": "answer", "rounds": 1 }));
        let snapshot = state.snapshot();
        assert_eq!(snapshot["turns"][0]["status"], "done");
        assert_eq!(snapshot["turns"][0]["text"], "answer");
        state.clear_finished();
        assert_eq!(state.snapshot()["turns"], json!([]));
    }

    #[test]
    fn retains_pending_work_and_final_cited_answers_without_duplicate_turns() {
        let mut state = UiState::default();
        state.record("ai.answer", &mut json!({ "turn_id": 1, "question": "q", "status": "pending" }));
        state.clear_finished();
        state.record("ai.delta", &mut json!({ "turn_id": 1, "text": "stream" }));
        state.record("ai.answer", &mut json!({ "turn_id": 1, "status": "ok", "text": "final", "citations": [{"ordinal":1}] }));
        assert_eq!(state.snapshot()["turns"].as_array().unwrap().len(), 1);
        assert_eq!(state.snapshot()["turns"][0]["text"], "final");
    }
}
