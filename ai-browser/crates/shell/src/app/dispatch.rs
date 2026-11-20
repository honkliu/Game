use super::*;

impl AppState {
    pub(super) fn wv_execute(
        &mut self,
        req: &Request,
        source: Surface,
    ) -> Result<Option<Value>, String> {
        let mut args = if req.payload.is_null() {
            json!({})
        } else {
            req.payload.clone()
        };
        if let Err(failure) = self.wv_api_prepare_task(&req.cmd, &mut args) {
            self.wv_reply_failure(self.api.current, failure);
            return Ok(None);
        }
        let p = &args;
        ai_platform_contract::browser::validate(&req.cmd, p)?;
        let operation = p.get("operation").and_then(Value::as_str).unwrap_or("");
        let id_arg = || {
            p.get("tab_id")
                .and_then(Value::as_u64)
                .map(TabId)
                .or(self.active)
        };
        match (req.cmd.as_str(), operation) {
            ("shell_ui", "ready") => self.wv_ui_ready(),
            ("shell_ai", "ask") => self.wv_ai_ask(p),
            ("shell_ai", "stop") => self.wv_ai_stop(p),
            ("shell_ai", "status_get") => Ok(Some(self.wv_ai_status_get())),
            ("shell_ai", "session_get") => Ok(Some(self.ai.ui.snapshot())),
            ("shell_ai", "models_list") => self.wv_ai_model_list(),
            ("shell_ai", "config_set") => self.wv_ai_config_set(p),
            ("shell_ai", "provider_set") => self.wv_ai_provider_set(p),
            ("shell_ai", "provider_delete") => self.wv_ai_provider_delete(p),
            ("shell_ai", "conversation_open") => {
                if self.ai.agent_run != 0
                    || !self.ai.tasks.is_empty()
                    || !self.ai.agent_jobs.is_empty()
                {
                    return Err("stop the current task before opening a conversation".into());
                }
                let id = p
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("conversation ID required")?;
                self.settings_page.conversation = Some(id.to_string());
                self.ai_show_pane();
                self.emit_raw("sidebar", "conversation.open", json!({}));
                Ok(None)
            }
            ("shell_ai", "conversation_pending_get") => Ok(Some(
                json!({ "id": self.settings_page.conversation.take() }),
            )),
            ("shell_ai", "history_clear") => self.wv_ai_history_clear(),
            ("shell_agent", "run") => self.wv_agent_run(p),
            ("shell_agent", "stop") => self.wv_agent_stop(),
            ("shell_agent", "approval_resolve") => self.wv_agent_approval_resolve(p),
            ("shell_agent", "state_get" | "config_set" | "context_preview")
            | (
                "shell_agent_tool" | "shell_agent_skill" | "shell_agent_prompt" | "shell_agent_mcp"
                | "shell_conversation",
                _,
            ) => self.wv_agent_manage(&req.cmd, p, source),
            ("shell_agent", "context_get") => Ok(Some(self.wv_agent_context_get())),
            ("shell_ai", "pane_resize") => {
                let wanted = p
                    .get("width")
                    .and_then(Value::as_f64)
                    .ok_or("ai.pane.resize: missing width")?
                    .round() as i32;
                let l = self.layout();
                let rail = self.settings.tabs().rail_w();
                let w = layout::set_ai_width(wanted, l.shell_w, rail);
                self.apply_layout();
                Ok(Some(json!({ "width": w })))
            }
            ("shell_ai", "citation_reveal") => self.wv_ai_citation_reveal(p),
            ("shell_ai", "selection_run") => self.wv_ai_selection_run(p),
            ("shell_page_navigate", "") => {
                let raw = p
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("url is required")?;
                let url = if self.wv_api_caller().id == 0 {
                    raw.to_string()
                } else {
                    operations::http_url(raw)?
                };
                let canvas = p
                    .get("tab_id")
                    .and_then(Value::as_u64)
                    .map(TabId)
                    .filter(|id| self.canvas_of(*id).is_some())
                    .or_else(|| {
                        if p.get("tab_id").is_none() && self.active.is_none() {
                            self.canvas_shown
                        } else {
                            None
                        }
                    });
                if let Some(canvas) = canvas.filter(|id| {
                    self.canvas_of(*id)
                        .is_some_and(|canvas| canvas.focused().is_none())
                }) {
                    let cell = self.canvas_of(canvas).map_or(0, |canvas| canvas.focus);
                    let result = self.wv_canvas_cell_open(
                        &json!({ "canvas_id": canvas.0, "cell": cell, "url": url }),
                    )?;
                    if let Some(tab) = result.as_ref().and_then(|result| result["tab_id"].as_u64())
                    {
                        self.wv_api_record_open(self.wv_api_caller().id, TabId(tab));
                    }
                    return Ok(result);
                }
                let tab = self.wv_page_target(p)?;
                self.wv_page_navigate(tab, &url, true);
                Ok(Some(
                    json!({ "tab_id": tab.0, "url": url, "dispatched": true, "confirmed": false }),
                ))
            }
            ("shell_page_read", "") | ("shell_page_click", "") | ("shell_page_fill", "") => {
                let id = self.api.current;
                self.wv_api_defer(id);
                match req.cmd.as_str() {
                    "shell_page_read" => self.wv_page_read(id, p),
                    "shell_page_click" => self.wv_page_click(id, p),
                    _ => self.wv_page_fill(id, p),
                }
                Ok(None)
            }
            ("shell_page_scroll_to", "") => self.wv_page_scroll_to(p),
            ("shell_page_snapshot", "") => {
                let id = self.api.current;
                self.wv_api_defer(id);
                self.wv_page_snapshot(id, p.clone());
                Ok(None)
            }
            ("shell_cookie", _) | ("shell_storage", _) => {
                let id = self.api.current;
                self.wv_site_data_call(
                    id,
                    if req.cmd == "shell_cookie" {
                        "cookie"
                    } else {
                        "storage"
                    },
                    p,
                )?;
                self.wv_api_defer(id);
                Ok(None)
            }
            ("shell_page_navigation", "back")
            | ("shell_page_navigation", "forward")
            | ("shell_page_navigation", "reload")
            | ("shell_page_navigation", "stop") => self.wv_page_navigation(operation, p, &req.cmd),
            (
                "shell_canvas",
                "open" | "cell_open" | "cell_focus" | "zoom" | "layout_set" | "resize"
                | "cell_close",
            ) => {
                let caller = self.wv_api_caller().id;
                let previous: std::collections::HashSet<_> = if caller == 0 {
                    Default::default()
                } else {
                    self.tabs.tabs().iter().map(|tab| tab.id.0).collect()
                };
                if self.wv_api_caller().id != 0 {
                    if let Some(url) = p.get("url").and_then(Value::as_str) {
                        operations::http_url(url)?;
                    }
                    if let Some(urls) = p.get("urls").and_then(Value::as_array) {
                        for url in urls {
                            operations::http_url(url.as_str().ok_or("urls must contain strings")?)?;
                        }
                    }
                }
                let result = match operation {
                    "open" => self.wv_canvas_open(p),
                    "cell_open" => self.wv_canvas_cell_open(p),
                    "cell_focus" => self.wv_canvas_cell_focus(p),
                    "zoom" => self.wv_canvas_zoom(p),
                    "layout_set" => self.wv_canvas_layout_set(p),
                    "resize" => self.wv_canvas_resize(p),
                    _ => self.wv_canvas_cell_close(p),
                }?;
                if caller != 0 {
                    let created: Vec<_> = self
                        .tabs
                        .tabs()
                        .iter()
                        .filter(|tab| !previous.contains(&tab.id.0))
                        .map(|tab| tab.id)
                        .collect();
                    for tab in created {
                        self.wv_api_record_open(caller, tab);
                    }
                }
                let mut value = result.unwrap_or_else(|| json!({}));
                value["canvas"] = self.wv_canvas_state_brief();
                Ok(Some(value))
            }
            ("shell_page_popups_dismiss", "") => {
                let targets = self.wv_popups_targets(p)?;
                let id = self.api.current;
                self.wv_api_defer(id);
                self.wv_popups_dismiss(id, targets, true);
                Ok(None)
            }
            ("shell_ui", "menu_set") => self.wv_ui_menu_set(p),
            ("shell_canvas", "state_get") => Ok(Some(self.wv_canvas_state_get())),
            ("shell_tab", "activate") => {
                let id = self.wv_tab_target(p)?;
                self.wv_tab_activate(id);
                Ok(Some(json!({ "tab_id": id.0 })))
            }
            ("shell_tab", "close") => {
                if let Some(id) = id_arg().or(self.canvas_shown) {
                    if !self.wv_api_owns(self.wv_api_caller().id, id) {
                        return Err("only tabs opened by this client can be closed".into());
                    }
                    self.wv_tab_close(id);
                    return Ok(Some(json!({ "closed": id.0 })));
                }
                Ok(None)
            }
            ("shell_tab", "open") => {
                let caller = self.wv_api_caller();
                let url = p
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or(tabs::NEWTAB_URL);
                let url = if caller.id == 0 || p.get("url").is_none() {
                    url.to_string()
                } else {
                    operations::http_url(url)?
                };
                let parent = p
                    .get("parent_tab_id")
                    .and_then(Value::as_u64)
                    .map(TabId)
                    .or(self.active);
                let focus = !p
                    .get("background")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let id = self.wv_tab_open(&url, parent, focus);
                self.wv_api_record_open(caller.id, id);
                if caller.builtin && caller.id != 0 {
                    self.ai.ai_tabs.insert(id.0);
                }
                Ok(Some(json!({ "tab_id": id.0, "url": url })))
            }
            ("shell_tab", "list") => self.wv_tab_list(),
            ("shell_tab", "duplicate") => self.wv_tab_duplicate(p),
            ("shell_tab", "reopen") => self.wv_tab_reopen(),
            ("shell_tab", "pin") | ("shell_tab", "unpin") => self.wv_tab_pinned_set(operation, p),
            ("shell_tab", "move") => self.wv_tab_move(p),

            ("shell_bookmark", "all_tabs") => self.wv_bookmark_all_tabs(),
            ("shell_page_zoom", "set") | ("shell_page_zoom", "reset") => {
                self.zoom_request = self.zoom_request.wrapping_add(1);
                let Some(id) = id_arg() else { return Ok(None) };
                let factor = if operation == "reset" {
                    1.0
                } else {
                    p.get("factor")
                        .and_then(Value::as_f64)
                        .filter(|factor| factor.is_finite() && *factor > 0.0)
                        .ok_or("Zoom must be a positive finite number.")?
                };
                if self.live.contains_key(&id) {
                    self.wv_page_zoom_set(id, factor);
                }
                if let Some(t) = self.tabs.get_mut(id) {
                    t.zoom = factor;
                }
                self.emit(
                    "tab.zoom_changed",
                    json!({ "tab_id": id.0, "factor": factor }),
                );
                self.mark_dirty();
                Ok(Some(json!({ "factor": factor })))
            }
            ("shell_page_zoom", "in" | "out" | "fit") => {
                let tab = self.wv_page_target(p)?;
                if operation == "fit" {
                    self.wv_page_zoom_fit(tab)?;
                } else {
                    self.wv_page_zoom_step(
                        tab,
                        if operation == "in" {
                            "zoom_in"
                        } else {
                            "zoom_out"
                        },
                    );
                }
                Ok(Some(
                    json!({ "tab_id": tab.0, "factor": self.tabs.get(tab).map(|tab| tab.zoom) }),
                ))
            }
            ("shell_tab_group", "create") => self.wv_tab_group_create(p),
            ("shell_tab_group", "rename")
            | ("shell_tab_group", "color_set")
            | ("shell_tab_group", "collapsed_set")
            | ("shell_tab_group", "delete") => self.wv_tab_group_update(operation, p),
            ("shell_page_find", "start")
            | ("shell_page_find", "query")
            | ("shell_page_find", "next")
            | ("shell_page_find", "previous")
            | ("shell_page_find", "stop") => self.wv_page_find(operation, p),
            ("shell_omnibox", "query") => self.wv_omnibox_query(p),
            ("shell_omnibox", "activate") => self.wv_omnibox_activate(p),
            ("shell_history", "query") => self.wv_history_query(p),
            ("shell_history", "delete") => self.wv_history_delete(p),
            ("shell_history", "clear") => self.wv_history_clear(p),
            ("shell_bookmark", "toggle") => self.wv_bookmark_toggle(p),
            ("shell_bookmark", "delete") => self.wv_bookmark_delete(p),
            ("shell_download", "open" | "reveal" | "entry_remove") => {
                self.wv_download(operation, p)
            }
            ("shell_download", "list_clear") => self.wv_download_list_clear(),
            ("shell_settings", "set") => {
                let key = p.get("key").and_then(Value::as_str).ok_or("missing key")?;
                let value = p.get("value").cloned().unwrap_or(Value::Null);
                self.wv_settings_set(key, &value)?;
                Ok(Some(self.wv_settings_state_get()))
            }
            ("shell_settings", "profile_folder_open") => self.wv_settings_profile_folder_open(),
            ("shell_settings", "open") => {
                let section = p
                    .get("section")
                    .and_then(Value::as_str)
                    .unwrap_or("general");
                self.wv_settings_section_open(section, p.get("skill_draft").cloned());
                Ok(None)
            }
            ("shell_settings", "state_get") => Ok(Some(json!({
                "section": self.settings_page.section,
                "skill_draft": self.settings_page.skill_draft.take(),
                "settings": self.wv_settings_state_get(), "downloads": self.downloads,
            }))),
            ("shell_settings", "section_set") => self.wv_settings_section_set(p),
            ("shell_ui", "omnibox_focus") => self.wv_ui_omnibox_focus(),
            ("shell_ui", "page_focus") => self.wv_ui_page_focus(),
            ("shell_ui", "palette_open") => {
                self.wv_ui_palette_open();
                Ok(None)
            }
            ("shell_ui", "tab_rail_toggle") => self.wv_ui_tab_rail_toggle(),
            ("shell_diagnostics", "measure_overlay") => self.wv_diagnostics_overlay_measure(p),
            ("shell_diagnostics", "measure_wait") => self.wv_diagnostics_wait(p),
            ("shell_ui", "overlay_close_top") => self.wv_ui_overlay_close_top(),
            ("shell_ui", "omnibox_dropdown_set") => self.wv_ui_omnibox_dropdown_set(p),

            ("shell_ui", "shortcut") => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("");
                let args = p.get("args").cloned().unwrap_or(Value::Null);
                self.wv_ui_shortcut(name, args);
                Ok(None)
            }
            ("shell_window", "minimize") => self.wv_window_minimize(),
            ("shell_window", "maximize_toggle") => {
                self.wv_window_maximize_toggle()?;
                Ok(None)
            }
            ("shell_window", "span_toggle") => self.wv_window_span_toggle(),
            ("shell_window", "span_select") => self.wv_window_span_select(p),
            ("shell_window", "state_get") => {
                self.refresh_window_displays(true);
                Ok(Some(self.wv_window_state_get()))
            }
            ("shell_window", "drag") => self.wv_window_drag(),
            ("shell_window", "resize") => self.wv_window_resize(p),
            ("shell_window", "close") => self.wv_window_close(),
            ("shell_page_url_copy", "") => self.wv_page_url_copy(),
            ("shell_diagnostics", "self_test") => {
                let report = self.wv_diagnostics_self_test();
                Ok(Some(report))
            }
            (name, operation) => Err(format!(
                "unsupported shell API operation: {name}/{operation}"
            )),
        }
    }
}
