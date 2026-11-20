//! Page-level "close all popups".
//!
//! One command, two entry points: the shell command `page.dismiss_popups` and the agent
//! tool `dismiss_popups`. Both take a scope: `cell` (the focused cell, or the one named by
//! `cell`/`row`+`col`), `tab` (every page of the current tab: all cells of a canvas), or
//! `all` (every open page). The script in `dismiss.js` clicks a popup's own close control
//! and hides what is left, so it works on any site without a filter list.

use serde_json::{json, Value};

use super::ai_glue::is_readable_url;
use super::AppState;
use super::operations::{Evaluation, Chain};
use crate::tabs::TabId;

const DISMISS_SCRIPT: &str = include_str!("dismiss.js");

impl AppState {
    /// The pages a dismiss request means, in reading order.
    pub(super) fn wv_popups_targets(&self, args: &Value) -> Result<Vec<TabId>, String> {
        let scope = args.get("scope").and_then(Value::as_str).unwrap_or("cell");
        let explicit = ["cell", "row", "col", "tab_id"].iter().any(|k| args.get(*k).is_some());
        let mut targets: Vec<TabId> = match (scope, self.canvas_shown) {
            ("all", _) => self.tabs.tabs().iter().filter(|t| t.canvas.is_none()).map(|t| t.id).collect(),
            ("tab" | "canvas", Some(canvas)) => self.canvas_of(canvas).map(|c| c.members()).unwrap_or_default(),
            (_, Some(canvas)) if explicit => {
                let cv = self.canvas_of(canvas).ok_or("no canvas on screen")?;
                match self.cell_arg(canvas, args).and_then(|i| cv.cells.get(i).copied().flatten()) {
                    Some(t) => vec![t],
                    None if args.get("tab_id").is_some() => vec![TabId(args["tab_id"].as_u64().unwrap_or(0))],
                    None => return Err("that cell is empty".into()),
                }
            }
            (_, Some(canvas)) => self.canvas_of(canvas).and_then(|c| c.focused()).into_iter().collect(),
            (_, None) => match args.get("tab_id").and_then(Value::as_u64) {
                Some(t) => vec![TabId(t)],
                None => self.active.into_iter().collect(),
            },
        };
        targets.retain(|t| self.tabs.get(*t).is_some_and(|tab| is_readable_url(&tab.url)));
        if targets.is_empty() {
            return Err("there is no web page here to clean (a new tab, settings and PDFs have no popups)".into());
        }
        Ok(targets)
    }

    /// Start dismissing in every target, one after another. `reply` answers the agent's
    /// request `id` with the summary; otherwise the summary is announced as an event.
    pub(super) fn wv_popups_dismiss(&mut self, id: u64, targets: Vec<TabId>, reply: bool) {
        let mut rest: Vec<u64> = targets.iter().rev().map(|t| t.0).collect();
        let Some(first) = rest.pop() else { return };
        let tab = TabId(first);
        self.wv_page_wake(tab);
        let caller = self.api.pending.get(&id).map(|p| p.caller).unwrap_or(self.wv_api_caller());
        let eval = Evaluation {
            tab: first,
            document: self.ai.identity.current(first),
            caller: caller.id,
            run: (reply && caller.builtin && caller.id != 0 && self.ai.agent_run != 0).then_some(self.ai.agent_run),
            script: DISMISS_SCRIPT.to_string(),
            attempts: 0,
            is_read: false,
            chain: Some(Chain { rest, results: Vec::new(), reply }),
        };
        self.api.evals.insert(id, eval);
        self.wv_eval_schedule(id, 0);
    }

    /// One target answered (or failed): record it and move to the next, or finish.
    pub(super) fn wv_popups_step(&mut self, id: u64, mut eval: Evaluation, raw: &str) {
        let mut row = serde_json::from_str::<String>(raw)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or_else(|| json!({ "error": "the page returned nothing readable" }));
        let tab = TabId(eval.tab);
        row["tab_id"] = json!(tab.0);
        row["title"] = json!(self.tabs.get(tab).map(|t| t.display_title()).unwrap_or_default());
        let Some(mut chain) = eval.chain.take() else { return };
        chain.results.push(row);
        while let Some(next) = chain.rest.pop() {
            if self.live.contains_key(&TabId(next)) {
                eval.tab = next;
                eval.document = self.ai.identity.current(next);
                eval.chain = Some(chain);
                self.api.evals.insert(id, eval);
                return self.wv_eval_schedule(id, 0);
            }
            chain.results.push(json!({ "tab_id": next, "error": "the page is not open" }));
        }
        let total: u64 = chain.results.iter().filter_map(|r| r.get("dismissed").and_then(Value::as_u64)).sum();
        let summary = json!({ "dismissed": total, "pages": chain.results });
        self.emit("page.popups_dismissed", summary.clone());
        if chain.reply {
            self.wv_reply_ok(id, summary);
        }
    }
}
