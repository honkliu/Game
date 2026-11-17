use super::*;

pub(super) struct State {
    pub section: String,
    pub skill_draft: Option<Value>,
    pub conversation: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self { section: "general".into(), skill_draft: None, conversation: None }
    }
}

pub(super) fn valid_section(section: &str) -> bool {
    ["general", "model", "tools", "skills", "mcp", "prompts", "instructions", "history", "downloads", "chats"].contains(&section)
}

fn allowed(request: &Request) -> bool {
    match request.cmd.as_str() {
        "settings.state" | "settings.section" | "settings.update" | "settings.open_folder" |
        "ai.status" | "ai.models" | "ai.provider.set" | "ai.provider.delete" | "ai.config.set" |
        "ai.conversation.open" | "history.query" | "history.forget" | "history.clear" |
        "download.action" | "download.clear_list" => true,
        "nav.open" => request.payload["url"].as_str().is_some_and(|url| url.starts_with("https://") || url.starts_with("http://")),
        "agent.call" => request.payload["op"].as_str().is_some_and(|op| {
            op == "state" || op == "config.set" ||
            ["tools.", "prompt.", "skill.", "mcp.", "conv."].iter().any(|prefix| op.starts_with(prefix))
        }),
        _ => false,
    }
}

impl AppState {
    pub(super) fn open_settings(&mut self) {
        self.open_settings_section("general", None);
    }

    pub(super) fn open_settings_section(&mut self, section: &str, skill_draft: Option<Value>) {
        self.settings_page.section = if valid_section(section) { section } else { "general" }.to_string();
        self.settings_page.skill_draft = skill_draft;
        self.close_overlay();
        let existing = self.tabs.tabs().iter().find(|tab| tab.url == "native://settings").map(|tab| tab.id);
        if let Some(tab) = existing {
            self.activate(tab);
        } else {
            self.create_tab("native://settings", self.active, true);
        }
        let payload = json!({ "section": self.settings_page.section, "skill_draft": self.settings_page.skill_draft });
        if let Some(id) = self.active {
            self.emit_surface(Surface::Settings(id), "settings.section", payload);
        }
    }

    pub(super) fn settings_view(&self, id: TabId) -> Option<&WebView> {
        self.live.get(&id).map(|live| &live.webview)
            .filter(|view| view.url().ok().is_some_and(|url| host::is_internal_page(&url, "settings.html")))
    }

    pub(super) fn emit_surface(&mut self, source: Surface, event: &str, payload: Value) {
        if let Surface::Settings(id) = source {
            let envelope = json!({ "event": event, "payload": payload });
            if let Some(view) = self.settings_view(id) {
                let _ = view.evaluate_script(&format!("window.__native&&window.__native._dispatch({envelope})"));
            }
        } else {
            self.emit_raw(source.target(), event, payload);
        }
    }

    pub(super) fn emit_settings(&mut self, event: &str, payload: Value) {
        if !matches!(event, "ai.context" | "ai.models" | "state.settings" | "state.downloads" | "state.history_meta" | "agent.event" | "ai.answer") {
            return;
        }
        if event == "agent.event" && !matches!(payload["type"].as_str(), Some("start" | "done" | "error")) {
            return;
        }
        let ids: Vec<TabId> = self.tabs.tabs().iter().filter(|tab| tab.url == "native://settings").map(|tab| tab.id).collect();
        for id in ids {
            self.emit_surface(Surface::Settings(id), event, payload.clone());
        }
    }

    pub(super) fn update_preferences(&mut self, key: &str, value: &Value) -> Result<(), String> {
        let mut candidate = self.settings.clone();
        candidate.apply(key, value).map_err(|error| error.to_string())?;
        settings::save(&self.settings_path(), &candidate)
            .map_err(|error| format!("Could not save setting: {error}"))?;
        self.settings = candidate;
        if key == "history_cap" {
            self.store.vacuum_after_cap_change(self.settings.history_cap).map_err(|error| error.to_string())?;
        }
        #[cfg(target_os = "macos")]
        if key == "download_dir" {
            *self.download_destination.borrow_mut() = PathBuf::from(self.download_dir());
        }
        if matches!(key, "rail_collapsed" | "vertical_tabs" | "density") { self.apply_layout(); }
        self.emit("state.settings", self.settings_payload());
        self.emit_tabs();
        Ok(())
    }

    pub(super) fn settings_probe(&mut self, tab: TabId, raw: &str) -> bool {
        if raw.len() > 512_000 { return false; }
        let Ok(value) = serde_json::from_str::<Value>(raw) else { return false };
        if value["ai"] != "settings" && value.get("cmd").is_none() { return false; }
        if self.settings_view(tab).is_none() { return true; }
        if value.get("cmd").is_some() {
            if let Ok(request) = serde_json::from_value::<Request>(value) {
                let source = Surface::Settings(tab);
                let reply = if allowed(&request) { self.dispatch(source, raw) }
                    else { Some(Reply::err(request.id, "command not available in Settings")) };
                if let Some(reply) = reply { self.reply(source, reply); }
            }
            return true;
        }
        let key = value["key"].as_str().unwrap_or("");
        let result = if key == "open_folder" {
            if sys::shell_open_folder(&self.data_dir.to_string_lossy()) { Ok(()) }
            else { Err("Could not open the data folder".to_string()) }
        } else {
            self.update_preferences(key, &value["value"])
        };
        let response = json!({ "ok": result.is_ok(), "error": result.err(), "settings": self.settings_payload() });
        if let Some(live) = self.live.get(&tab) {
            let _ = live.webview.evaluate_script(&format!("window.__settingsResult&&window.__settingsResult({response})"));
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_bridge_does_not_expose_shell_or_agent_execution() {
        for cmd in ["window.close", "shell.ready", "agent.run", "ai.ask", "tab.close"] {
            assert!(!allowed(&Request { id: None, cmd: cmd.into(), payload: json!({}) }));
        }
        assert!(allowed(&Request { id: None, cmd: "agent.call".into(), payload: json!({"op":"tools.definition.save"}) }));
        assert!(!allowed(&Request { id: None, cmd: "agent.call".into(), payload: json!({"op":"browser.eval"}) }));
        assert!(!allowed(&Request { id: None, cmd: "nav.open".into(), payload: json!({"url":"file:///secret"}) }));
    }
}
