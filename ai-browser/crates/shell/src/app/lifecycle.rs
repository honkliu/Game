use super::*;

impl AppState {
    pub(super) fn lifecycle_probe(&mut self, id: TabId, raw: &str) -> bool {
        if raw.len() > 2_000 {
            return false;
        }
        let Ok(value) = serde_json::from_str::<Value>(raw) else { return false };
        if value["ai"] != "lifecycle" {
            return false;
        }
        let Some(tab) = self.tabs.get(id) else { return true };
        let source = value["url"].as_str().unwrap_or("");
        let source_pdf = crate::pdf::source_of(source);
        if !tab.url.starts_with("native://") && source_pdf.as_deref().unwrap_or(source) != tab.url {
            return true;
        }
        let Some(live) = self.live.get_mut(&id) else { return true };
        live.lifecycle_known = true;
        live.editing |= value["editing"].as_bool().unwrap_or(true);
        live.playing = value["playing"].as_bool().unwrap_or(true);
        if let Some(y) = live.pending_scroll {
            if value["ready"] == true {
                live.pending_scroll = None;
                let _ = live.webview.evaluate_script(&format!(
                    "requestAnimationFrame(function(){{window.scrollTo(0,{y})}})"
                ));
            }
        } else if let Some(y) = value["scroll_y"].as_f64().filter(|y| y.is_finite() && *y >= 0.0 && *y < 1e8) {
            if let Some(tab) = self.tabs.get_mut(id) {
                if (tab.scroll_y - y).abs() >= 1.0 {
                    tab.scroll_y = y;
                    self.mark_dirty();
                }
            }
        }
        true
    }

    pub(super) fn lifecycle_tick(&mut self) {
        if self.last_tab_prune.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_tab_prune = Instant::now();
        let keep = self.canvas_shown.and_then(|id| self.canvas_of(id))
            .map(|canvas| canvas.members())
            .unwrap_or_else(|| self.active.into_iter().collect());
        self.wv_webviews_suspend_background(&keep);
    }
}
