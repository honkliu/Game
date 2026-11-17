use super::*;

impl AppState {
    pub(super) fn open_settings(&mut self) {
        self.close_overlay();
        let existing = self.tabs.tabs().iter().find(|tab| tab.url == "native://settings").map(|tab| tab.id);
        if let Some(tab) = existing {
            self.activate(tab);
        } else {
            self.create_tab("native://settings", self.active, true);
        }
    }

    pub(super) fn settings_probe(&mut self, tab: TabId, raw: &str) -> bool {
        if raw.len() > 16_000 { return false; }
        let Ok(value) = serde_json::from_str::<Value>(raw) else { return false };
        if value["ai"] != "settings" { return false; }
        let trusted = self.live.get(&tab).and_then(|live| live.webview.url().ok())
            .is_some_and(|url| host::is_internal_page(&url, "settings.html"));
        if !trusted { return true; }
        let key = value["key"].as_str().unwrap_or("");
        let result = if key == "open_folder" {
            if sys::shell_open_folder(&self.data_dir.to_string_lossy()) { Ok(()) }
            else { Err("Could not open the data folder".to_string()) }
        } else {
            (|| {
                let mut candidate = self.settings.clone();
                candidate.apply(key, &value["value"]).map_err(|error| error.to_string())?;
                settings::save(&self.settings_path(), &candidate)
                    .map_err(|error| format!("Could not save setting: {error}"))?;
                self.settings = candidate;
                Ok(())
            })()
        };
        if result.is_ok() && key != "open_folder" {
            if matches!(key, "rail_collapsed" | "vertical_tabs" | "density") { self.apply_layout(); }
            self.emit("state.settings", self.settings_payload());
            self.emit_tabs();
        }
        let response = json!({ "ok": result.is_ok(), "error": result.err(), "settings": self.settings_payload() });
        if let Some(live) = self.live.get(&tab) {
            let _ = live.webview.evaluate_script(&format!("window.__settingsResult&&window.__settingsResult({response})"));
        }
        true
    }
}
