use super::*;
use crate::ai::session::Pending;
use ai_platform_contract::frame::Method;

pub(super) struct State {
    pub section: String,
    pub skill_draft: Option<Value>,
    pub conversation: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            section: "general".into(),
            skill_draft: None,
            conversation: None,
        }
    }
}

pub(super) fn valid_section(section: &str) -> bool {
    [
        "general",
        "model",
        "tools",
        "skills",
        "mcp",
        "rivermcp",
        "prompts",
        "instructions",
        "history",
        "downloads",
        "chats",
    ]
    .contains(&section)
}

pub(super) fn allowed(request: &Request) -> bool {
    let operation = request.payload["operation"].as_str().unwrap_or("");
    match (request.cmd.as_str(), operation) {
        (
            "shell_settings",
            "state_get"
            | "section_set"
            | "set"
            | "profile_folder_open"
            | "river_mcp_get"
            | "river_mcp_apply",
        )
        | (
            "shell_ai",
            "status_get" | "models_list" | "provider_set" | "provider_delete" | "config_set"
            | "conversation_open",
        )
        | ("shell_history", "query" | "delete" | "clear")
        | ("shell_download", "open" | "reveal" | "entry_remove" | "list_clear")
        | ("shell_agent", "state_get" | "config_set") => true,
        (
            "shell_agent_tool" | "shell_agent_skill" | "shell_agent_prompt" | "shell_agent_mcp"
            | "shell_conversation",
            _,
        ) => ai_platform_contract::browser::validate(&request.cmd, &request.payload).is_ok(),
        ("shell_page_navigate", "") | ("shell_tab", "open") => request.payload["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://") || url.starts_with("http://")),
        _ => false,
    }
}

impl AppState {
    pub(super) fn wv_river_mcp_get(&self) -> Result<Value, String> {
        if self.wv_api_caller().surface.is_none() {
            return Err("RiverMCP settings are available in the browser UI".into());
        }
        let server = self.api.server.as_ref().ok_or("RiverMCP is not running")?;
        Ok(
            json!({"port":self.settings.river_mcp.port,"token":self.settings.river_mcp.token,
            "active_port":server.config().port,"listen_address":"0.0.0.0","error":self.api.river_mcp_error}),
        )
    }

    pub(super) fn wv_river_mcp_apply(&mut self, value: &Value) -> Result<Value, String> {
        if self.wv_api_caller().surface.is_none() {
            return Err("RiverMCP settings are available in the browser UI".into());
        }
        let port = value["port"]
            .as_u64()
            .filter(|port| (1..=65535).contains(port))
            .ok_or("Port must be between 1 and 65535")? as u16;
        let token = value["token"]
            .as_str()
            .ok_or("Token is required")?
            .to_string();
        let config = crate::api_server::Config { port, token };
        let server = self.api.server.as_ref().ok_or("RiverMCP is not running")?;
        let prepared = server.prepare(config.clone())?;
        let mut candidate = self.settings.clone();
        candidate.river_mcp = config;
        settings::save(&self.settings_path(), &candidate)
            .map_err(|error| format!("Could not save RiverMCP settings: {error}"))?;
        server.apply(prepared);
        self.settings = candidate;
        self.api.river_mcp_error = None;
        if self.ai.link.is_some()
            && self
                .ai_request(
                    Method::AgentControl,
                    &json!({"op":"state.get"}),
                    Pending::Control(None),
                )
                .is_err()
        {
            self.ai.link = None;
            self.note("AI process will reconnect to RiverMCP on its next request");
        }
        self.wv_river_mcp_get()
    }

    pub(super) fn wv_settings_open(&mut self) {
        self.wv_settings_section_open("general", None);
    }

    pub(super) fn wv_settings_section_open(&mut self, section: &str, skill_draft: Option<Value>) {
        self.settings_page.section = if valid_section(section) {
            section
        } else {
            "general"
        }
        .to_string();
        self.settings_page.skill_draft = skill_draft;
        self.wv_ui_overlay_close();
        let existing = self
            .tabs
            .tabs()
            .iter()
            .find(|tab| tab.url == "native://settings")
            .map(|tab| tab.id);
        if let Some(tab) = existing {
            self.wv_tab_activate(tab);
        } else {
            self.wv_tab_open("native://settings", self.active, true);
        }
        let payload = json!({ "section": self.settings_page.section, "skill_draft": self.settings_page.skill_draft });
        if let Some(id) = self.active {
            self.emit_surface(Surface::Settings(id), "settings.section", payload);
        }
    }

    pub(super) fn settings_view(&self, id: TabId) -> Option<&WebView> {
        self.live.get(&id).map(|live| &live.webview).filter(|view| {
            view.url()
                .ok()
                .is_some_and(|url| host::is_internal_page(&url, "settings.html"))
        })
    }

    pub(super) fn emit_surface(&mut self, source: Surface, event: &str, payload: Value) {
        if let Some(server) = self.api.server.as_ref() {
            server.publish(Some(source), event, payload);
        }
    }

    pub(super) fn emit_settings(&mut self, event: &str, payload: Value) {
        if !matches!(
            event,
            "ai.context"
                | "ai.models"
                | "state.settings"
                | "state.downloads"
                | "state.history_meta"
                | "agent.event"
                | "ai.answer"
        ) {
            return;
        }
        if event == "agent.event"
            && !matches!(payload["type"].as_str(), Some("start" | "done" | "error"))
        {
            return;
        }
        let ids: Vec<TabId> = self
            .tabs
            .tabs()
            .iter()
            .filter(|tab| tab.url == "native://settings")
            .map(|tab| tab.id)
            .collect();
        for id in ids {
            self.emit_surface(Surface::Settings(id), event, payload.clone());
        }
    }

    pub(super) fn wv_settings_set(&mut self, key: &str, value: &Value) -> Result<(), String> {
        let mut candidate = self.settings.clone();
        candidate
            .apply(key, value)
            .map_err(|error| error.to_string())?;
        settings::save(&self.settings_path(), &candidate)
            .map_err(|error| format!("Could not save setting: {error}"))?;
        self.settings = candidate;
        if key == "history_cap" {
            self.store
                .vacuum_after_cap_change(self.settings.history_cap)
                .map_err(|error| error.to_string())?;
        }
        #[cfg(target_os = "macos")]
        if key == "download_dir" {
            *self.download_destination.borrow_mut() = PathBuf::from(self.download_dir());
        }
        if matches!(key, "rail_collapsed" | "vertical_tabs" | "density") {
            self.apply_layout();
        }
        self.emit("state.settings", self.wv_settings_state_get());
        self.emit_tabs();
        Ok(())
    }

    pub(super) fn settings_probe(&mut self, tab: TabId, raw: &str) -> bool {
        if raw.len() > 512_000 {
            return false;
        }
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return false;
        };
        let _ = tab;
        value.get("cmd").is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_bridge_does_not_expose_shell_or_agent_execution() {
        for (cmd, operation) in [
            ("shell_window", "close"),
            ("shell_ui", "ready"),
            ("shell_agent", "run"),
            ("shell_ai", "ask"),
            ("shell_tab", "close"),
        ] {
            assert!(!allowed(&Request {
                id: None,
                cmd: cmd.into(),
                payload: json!({ "operation": operation })
            }));
        }
        assert!(allowed(&Request {
            id: None,
            cmd: "shell_agent_tool".into(),
            payload: json!({"operation":"definition_save"})
        }));
        assert!(!allowed(&Request {
            id: None,
            cmd: "shell_agent_tool".into(),
            payload: json!({"operation":"eval"})
        }));
        assert!(!allowed(&Request {
            id: None,
            cmd: "shell_page_navigate".into(),
            payload: json!({"url":"file:///secret"})
        }));
        assert!(!allowed(&Request {
            id: None,
            cmd: "agent.call".into(),
            payload: json!({"op":"tools.definition.save"})
        }));
    }
}
