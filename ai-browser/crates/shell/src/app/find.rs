use super::*;

impl AppState {
    pub(super) fn find_query(&mut self, command: &str, params: &Value) -> Result<Option<Value>, String> {
        let tab = self.active.ok_or("no active page")?;
        self.find_seq += 1;
        let sequence = self.find_seq;
        let query = if command == "find.stop" { "" } else {
            params["query"].as_str().or_else(|| params["text"].as_str()).unwrap_or("")
        };
        if query.chars().count() > 1_000 {
            return Err("find query is too long".into());
        }
        let prior = self.find.as_ref().filter(|result| result.tab_id == tab.0 && result.query == query);
        let previous = prior.map_or(0, |result| result.active as i64);
        let total = prior.map_or(0, |result| result.total);
        let index = match command {
            "find.next" => previous,
            "find.prev" => previous - 2,
            _ => 0,
        };
        let script = format!("({})({},{},{},{},{},{})", include_str!("../../ui/find.js"),
            serde_json::to_string(query).unwrap_or_default(),
            params["case_sensitive"].as_bool().unwrap_or(false),
            params["regex_or_whole_word"].as_bool().unwrap_or(false),
            params["highlight_all"].as_bool().unwrap_or(false), index, sequence);
        #[cfg(windows)]
        if let Some(pdf) = self.live.get(&tab).and_then(|live| live.native_pdf.as_ref()) {
            let proxy = self.proxy();
            pdf.find(query.to_string(), index, move |raw| {
                let _ = proxy.send_event(AppMessage::FindDone(tab, sequence, raw));
            })?;
            if command == "find.stop" {
                self.find = None;
                self.emit("find.result", json!({ "closed": true }));
            } else {
                self.find = Some(FindResult { tab_id: tab.0, query: query.to_string(),
                    active: 0, total, supported_flags: Vec::new() });
            }
            return Ok(None);
        }
        if command == "find.stop" {
            self.find = None;
            if let Some(live) = self.live.get(&tab) {
                let _ = live.webview.evaluate_script(&script);
            }
            self.emit("find.result", json!({ "closed": true }));
            return Ok(None);
        }
        let active = if total > 0 { index.rem_euclid(total as i64) as u64 + 1 } else { 0 };
        self.find = Some(FindResult { tab_id: tab.0, query: query.to_string(),
            active, total, supported_flags: vec!["case_sensitive", "whole_word"] });
        let proxy = self.proxy();
        self.live.get(&tab).ok_or("the page is not loaded")?.webview
            .evaluate_script_with_callback(&script, move |raw| {
                let _ = proxy.send_event(AppMessage::FindDone(tab, sequence, raw));
            }).map_err(|error| error.to_string())?;
        Ok(None)
    }

    pub(super) fn find_done(&mut self, tab: TabId, sequence: u64, raw: String) {
        if self.find_seq != sequence || self.active != Some(tab) { return; }
        let value = serde_json::from_str::<String>(&raw).ok()
            .and_then(|inner| serde_json::from_str::<Value>(&inner).ok());
        if let Some(value) = value {
            self.find_result(tab, sequence, value);
        }
    }

    pub(super) fn find_probe(&mut self, tab: TabId, raw: &str) -> bool {
        if raw.len() > 8_000 { return false; }
        let Ok(value) = serde_json::from_str::<Value>(raw) else { return false };
        if value["ai"] != "find" { return false; }
        if let Some(sequence) = value["sequence"].as_u64() {
            self.find_result(tab, sequence, value["result"].clone());
        }
        true
    }

    fn find_result(&mut self, tab: TabId, sequence: u64, mut value: Value) {
        if self.find_seq != sequence || self.active != Some(tab) || value["pending"] == true { return; }
        if let Some(result) = self.find.as_mut() {
            result.total = value["total"].as_u64().unwrap_or(0);
            result.active = value["active"].as_u64().unwrap_or(0);
            value["tab_id"] = json!(tab.0);
            value["query"] = json!(result.query);
            self.emit("find.result", value);
        }
    }
}
