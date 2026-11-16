//! Canvas tabs: one tab that is a grid of live pages.
//!
//! A canvas (`Tab::canvas`) owns up to `cols × rows` cells. Every occupied cell is an
//! ordinary tab with `member_of` set, so navigation, titles, history, find and the
//! assistant's tools keep working per page; the rail simply does not list cells.
//! `AppState::active` is always the *focused cell* while a canvas is shown, so the
//! omnibox, the AI context and the agent's "current page" follow the cell the user is in.
//!
//! The shell webview sits behind every page webview and shows through the gaps the grid
//! leaves, so cell title strips and empty-cell placeholders are drawn by the shell UI
//! from the geometry in `canvas_payload` (one source of truth: `layout::canvas_cells`).

use serde_json::{json, Value};

use super::AppState;
use crate::layout::{self, CellGeom, Rc};
use crate::omnibox::{self, Intent};
use crate::tabs::{self, Canvas, Tab, TabId};

impl AppState {
    pub(super) fn canvas_of(&self, id: TabId) -> Option<&Canvas> {
        self.tabs.get(id).and_then(|t| t.canvas.as_ref())
    }

    /// The canvas a page belongs to, when that canvas is the one on screen.
    pub(super) fn shown_canvas_of(&self, member: TabId) -> Option<TabId> {
        let parent = self.tabs.get(member)?.member_of?;
        (self.canvas_shown == Some(parent)).then_some(parent)
    }

    fn canvas_area(&self) -> Rc {
        let l = self.layout();
        Rc { x: l.page_x, y: l.page_y, w: l.page_w, h: l.page_h }
    }

    fn canvas_geoms(&self) -> Option<(Canvas, Vec<CellGeom>)> {
        let cv = self.canvas_of(self.canvas_shown?)?.clone();
        let geoms = layout::canvas_cells(self.canvas_area(), cv.cols, cv.rows, cv.zoomed.then_some(cv.focus));
        Some((cv, geoms))
    }

    /// Where a page webview sits, and whether it is shown. A page in the canvas on
    /// screen takes its cell body; any other page takes the whole page area (`None`:
    /// visibility is decided by `activate`).
    pub(super) fn tab_geom(&self, id: TabId) -> (Rc, Option<bool>) {
        if let Some((cv, geoms)) = self.canvas_geoms() {
            if let Some(cell) = cv.cell_of(id) {
                let g = geoms[cell];
                return (g.body, Some(!g.hidden));
            }
        }
        (self.canvas_area(), None)
    }

    pub(super) fn canvas_payload(&self) -> Value {
        let Some(id) = self.canvas_shown else { return Value::Null };
        let Some((cv, geoms)) = self.canvas_geoms() else { return Value::Null };
        let rc = |r: Rc| json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h });
        let cells: Vec<Value> = cv
            .cells
            .iter()
            .zip(&geoms)
            .enumerate()
            .map(|(i, (cell, g))| {
                let t = cell.and_then(|c| self.tabs.get(c));
                json!({
                    "index": i,
                    "tab_id": cell.map(|c| c.0),
                    "title": t.map(Tab::display_title),
                    "url": t.map(|t| t.url.clone()),
                    "focused": i == cv.focus,
                    "hidden": g.hidden,
                    "frame": rc(g.frame),

                    "body": rc(g.body),
                })
            })
            .collect();
        json!({
            "id": id.0,
            "cols": cv.cols,
            "rows": cv.rows,
            "capacity": cv.capacity(),
            "zoomed": cv.zoomed,
            "focus": cv.focus,
            "cells": cells,
        })
    }

    /// What an agent needs to see after a canvas action: which pages sit in which cell.
    pub(super) fn canvas_payload_brief(&self) -> Value {
        let Some(id) = self.canvas_shown else { return Value::Null };
        let Some(cv) = self.canvas_of(id) else { return Value::Null };
        let cells: Vec<Value> = cv
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let t = c.and_then(|c| self.tabs.get(c));
                json!({ "cell": i, "tab_id": c.map(|c| c.0), "url": t.map(|t| t.url.clone()), "focused": i == cv.focus })
            })
            .collect();
        json!({ "cols": cv.cols, "rows": cv.rows, "zoomed": cv.zoomed, "cells": cells })
    }

    /// Text typed into a cell: a URL, or a search.
    fn resolve_typed(&self, text: &str) -> String {
        match omnibox::classify(text) {
            Intent::Url(u) => u,
            Intent::Search(q) => omnibox::search_url(&self.settings.search_template, &q),
            _ => {
                let t = text.trim();
                if t.is_empty() {
                    tabs::NEWTAB_URL.to_string()
                } else {
                    omnibox::search_url(&self.settings.search_template, t)
                }
            }
        }
    }

    fn canvas_retitle(&mut self, id: TabId) {
        let Some(cv) = self.canvas_of(id) else { return };
        let title = format!("Canvas {}×{} · {}/{}", cv.cols, cv.rows, cv.occupied(), cv.capacity());
        if let Some(t) = self.tabs.get_mut(id) {
            t.title = title;
        }
    }

    // ---- showing --------------------------------------------------------

    /// Put a canvas on screen: build every cell's page, tile them, focus one.
    pub(super) fn show_canvas(&mut self, canvas: TabId, focus: Option<TabId>) {
        if self.canvas_of(canvas).is_none() {
            return;
        }
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            if let Some(i) = focus.and_then(|f| cv.cell_of(f)) {
                cv.focus = i;
            }
        }
        // A canvas always has a page to focus.
        if self.canvas_of(canvas).is_some_and(|cv| cv.occupied() == 0) {
            self.canvas_add_page(canvas, 0, tabs::NEWTAB_URL);
        }
        let Some(cv) = self.canvas_of(canvas).cloned() else { return };
        let focused = cv.focused().or_else(|| cv.members().first().copied());
        self.canvas_shown = Some(canvas);
        self.active = focused;
        let members = cv.members();
        for m in &members {
            self.build_webview(*m);
        }
        let others: Vec<TabId> = self.live.keys().copied().filter(|k| !members.contains(k)).collect();
        for o in others {
            if let Some(live) = self.live.get(&o) {
                let _ = live.webview.set_visible(false);
            }
        }
        self.apply_layout();
        self.sleep_background_tabs(&members);
        if let Some(f) = focused {
            if let Some(live) = self.live.get(&f) {
                let _ = live.webview.focus();
            }
            let title = self.tabs.get(f).map(|t| t.display_title()).unwrap_or_default();
            self.emit("tab.activated", json!({ "tab_id": f.0, "title": title }));
        }
        self.emit_tabs();
        if let Some(f) = focused {
            self.ai_on_activated(f);
            self.agent_context_changed();
        }
    }

    /// Re-tile and refresh after the cell set changed.
    fn canvas_refresh(&mut self, canvas: TabId) {
        self.canvas_retitle(canvas);
        if self.canvas_shown == Some(canvas) {
            self.show_canvas(canvas, None);
        } else {
            self.emit_tabs();
        }
    }

    // ---- editing --------------------------------------------------------

    /// Create the page for a cell (not shown yet; `canvas_refresh` builds it).
    fn canvas_add_page(&mut self, canvas: TabId, cell: usize, url: &str) -> TabId {
        let id = self.tabs.alloc_id();
        let mut page = Tab::new(id, url);
        page.member_of = Some(canvas);
        let after = self
            .canvas_of(canvas)
            .and_then(|cv| cv.members().last().copied())
            .unwrap_or(canvas);
        self.tabs.insert_after(Some(after), page);
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            cv.cells[cell] = Some(id);
        }
        id
    }

    /// Forget a page: its webview, AI state and tab entry.
    fn canvas_drop_page(&mut self, id: TabId) {
        self.live.remove(&id);
        self.ai_on_tab_closed(id);
        self.last_nav_emit.remove(&id);
        if self.tabs.close(id).is_some() {
            self.emit("tab.closed", json!({ "tab_id": id.0 }));
        }
    }

    pub(super) fn canvas_close(&mut self, canvas: TabId) {
        let members = self.canvas_of(canvas).map(Canvas::members).unwrap_or_default();
        for m in members {
            self.canvas_drop_page(m);
        }
    }

    fn canvas_arg(&self, p: &Value) -> Result<TabId, String> {
        p.get("canvas_id")
            .and_then(Value::as_u64)
            .map(TabId)
            .or(self.canvas_shown)
            .filter(|id| self.canvas_of(*id).is_some())
            .ok_or_else(|| "no canvas: open one with canvas.create".to_string())
    }

    fn cell_arg(&self, canvas: TabId, p: &Value) -> Option<usize> {
        let cv = self.canvas_of(canvas)?;
        // `row`/`col` are 0-based like `cell`, which counts row by row.
        if let (Some(r), Some(c)) = (p.get("row").and_then(Value::as_u64), p.get("col").and_then(Value::as_u64)) {
            return Some(if (c as usize) < cv.cols { r as usize * cv.cols + c as usize } else { cv.capacity() });
        }
        match (p.get("cell").and_then(Value::as_u64), p.get("tab_id").and_then(Value::as_u64)) {
            (Some(c), _) => Some(c as usize),
            (None, Some(t)) => cv.cell_of(TabId(t)),
            _ => None,
        }
    }

    /// `canvas.create {cols?, rows?, urls?}` — a new canvas tab, shown at once.
    pub(super) fn canvas_create(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let dim = |k: &str, d: usize| p.get(k).and_then(Value::as_u64).map_or(d, |v| v as usize);
        let (cols, rows) = (dim("cols", tabs::CANVAS_DEFAULT.0), dim("rows", tabs::CANVAS_DEFAULT.1));
        if !(1..=tabs::CANVAS_MAX_COLS).contains(&cols) || !(1..=tabs::CANVAS_MAX_ROWS).contains(&rows) {
            return Err(format!(
                "a canvas is 1..{} columns by 1..{} rows",
                tabs::CANVAS_MAX_COLS,
                tabs::CANVAS_MAX_ROWS
            ));
        }
        let urls: Vec<String> = p
            .get("urls")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|u| self.resolve_typed(u)).collect())
            .unwrap_or_default();
        if urls.len() > cols * rows {
            return Err(format!("{} pages do not fit a {cols}×{rows} canvas (at most {})", urls.len(), cols * rows));
        }
        let id = self.tabs.alloc_id();
        let mut tab = Tab::new(id, tabs::CANVAS_URL);
        tab.canvas = Some(Canvas::new(cols, rows));
        let anchor = self.active.map(|a| self.tabs.get(a).and_then(|t| t.member_of).unwrap_or(a));
        self.tabs.insert_after(anchor, tab);
        let first = if urls.is_empty() { vec![tabs::NEWTAB_URL.to_string()] } else { urls };
        for (i, u) in first.iter().enumerate() {
            self.canvas_add_page(id, i, u);
        }
        self.canvas_retitle(id);
        if let Some(t) = self.tabs.get(id) {
            let payload = self.tab_json(t);
            self.emit("tab.created", json!({ "tab": payload }));
        }
        self.show_canvas(id, None);
        let pages = self.canvas_of(id).map(Canvas::members).unwrap_or_default();
        Ok(Some(json!({ "canvas_id": id.0, "cols": cols, "rows": rows, "tab_ids": pages.iter().map(|t| t.0).collect::<Vec<_>>() })))
    }

    /// `canvas.set {url, cell?, canvas_id?}` — load a page into a cell (the first empty
    /// one by default), creating it if the cell is empty, and focus it.
    pub(super) fn canvas_set(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        let url = self.resolve_typed(p.get("url").and_then(Value::as_str).unwrap_or(""));
        let cv = self.canvas_of(canvas).ok_or("no canvas")?;
        let cell = match self.cell_arg(canvas, p) {
            Some(c) => c,
            None => cv
                .first_empty()
                .ok_or_else(|| format!("the canvas is full: at most {} pages", cv.capacity()))?,
        };
        if cell >= cv.capacity() {
            return Err(format!("cell {cell} is outside the {}×{} canvas (at most {} pages)", cv.cols, cv.rows, cv.capacity()));
        }
        let slot = cv.cells[cell];
        let page = match slot {
            Some(existing) => {
                if self.shown_canvas_of(existing).is_some() {
                    self.build_webview(existing);
                }
                self.navigate(existing, &url, true);
                existing
            }
            None => self.canvas_add_page(canvas, cell, &url),
        };
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            if p.get("focus").and_then(Value::as_bool).unwrap_or(true) {
                cv.focus = cell;
            }
        }
        if self.canvas_shown != Some(canvas) && p.get("show").and_then(Value::as_bool).unwrap_or(true) {
            self.canvas_retitle(canvas);
            self.show_canvas(canvas, Some(page));
        } else {
            self.canvas_refresh(canvas);
        }
        Ok(Some(json!({ "canvas_id": canvas.0, "cell": cell, "tab_id": page.0 })))
    }

    /// Focus a page in the canvas on screen (a click, a title strip, the agent).
    pub(super) fn canvas_focus_tab(&mut self, id: TabId) {
        let Some(canvas) = self.shown_canvas_of(id) else { return };
        let Some(cell) = self.canvas_of(canvas).and_then(|cv| cv.cell_of(id)) else { return };
        let (changed, zoomed) = match self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            Some(cv) => {
                let changed = cv.focus != cell;
                cv.focus = cell;
                (changed, cv.zoomed)
            }
            None => return,
        };
        if self.active == Some(id) && !changed {
            return;
        }
        self.active = Some(id);
        if zoomed {
            self.apply_layout();
        }
        if let Some(live) = self.live.get(&id) {
            let _ = live.webview.focus();
        }
        let title = self.tabs.get(id).map(|t| t.display_title()).unwrap_or_default();
        self.emit("tab.activated", json!({ "tab_id": id.0, "title": title }));
        self.emit_tabs();
        self.ai_on_activated(id);
        self.agent_context_changed();
    }

    /// `canvas.focus {tab_id | cell | step}`.
    pub(super) fn canvas_focus(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        if self.canvas_shown != Some(canvas) {
            self.show_canvas(canvas, None);
        }
        // Focusing an empty cell gives it a new tab page, so the one address bar
        // on top drives every cell.
        if let Some(c) = self.cell_arg(canvas, p) {
            let empty = self.canvas_of(canvas).is_some_and(|cv| c < cv.cells.len() && cv.cells[c].is_none());
            if empty {
                self.canvas_add_page(canvas, c, tabs::NEWTAB_URL);
                if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
                    cv.focus = c;
                }
                self.canvas_refresh(canvas);
            }
        }
        let cv = self.canvas_of(canvas).ok_or("no canvas")?;
        let target = if let Some(step) = p.get("step").and_then(Value::as_i64) {
            let order: Vec<TabId> = cv.members();
            let cur = cv.focused().and_then(|f| order.iter().position(|m| *m == f)).unwrap_or(0) as i64;
            order.get((cur + step).rem_euclid(order.len().max(1) as i64) as usize).copied()
        } else {
            match self.cell_arg(canvas, p) {
                Some(c) => cv.cells.get(c).copied().flatten(),
                None => None,
            }
        };
        let id = target.ok_or("that cell has no page")?;
        self.canvas_focus_tab(id);
        Ok(Some(json!({ "tab_id": id.0 })))
    }

    /// `canvas.zoom {zoomed?}` — the focused page fills the canvas; again to go back.
    pub(super) fn canvas_zoom(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        let want = p.get("zoomed").and_then(Value::as_bool);
        let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) else { return Err("no canvas".into()) };
        cv.zoomed = want.unwrap_or(!cv.zoomed);
        let zoomed = cv.zoomed;
        if self.canvas_shown == Some(canvas) {
            self.apply_layout();
            self.emit_tabs();
        }
        Ok(Some(json!({ "zoomed": zoomed })))
    }

    /// `canvas.layout {cols, rows}` — reshape the grid; pages that no longer fit close.
    pub(super) fn canvas_layout(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        let cv = self.canvas_of(canvas).ok_or("no canvas")?;
        let get = |k: &str, d: usize| p.get(k).and_then(Value::as_u64).map_or(d, |v| v as usize);
        let (cols, rows) = (get("cols", cv.cols), get("rows", cv.rows));
        if !(1..=tabs::CANVAS_MAX_COLS).contains(&cols) || !(1..=tabs::CANVAS_MAX_ROWS).contains(&rows) {
            return Err(format!(
                "a canvas is 1..{} columns by 1..{} rows",
                tabs::CANVAS_MAX_COLS,
                tabs::CANVAS_MAX_ROWS
            ));
        }
        let dropped = match self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            Some(cv) => cv.reshape(cols, rows),
            None => Vec::new(),
        };
        for d in &dropped {
            self.canvas_drop_page(*d);
        }
        self.canvas_refresh(canvas);
        Ok(Some(json!({ "cols": cols, "rows": rows, "closed": dropped.len() })))
    }

    /// `canvas.clear {cell | tab_id}` — close one page. The last page is blanked instead,
    /// so a canvas never has nothing to focus; closing the canvas is `tab.close`.
    pub(super) fn canvas_clear(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        let cell = self.cell_arg(canvas, p).ok_or("canvas.clear: say which cell")?;
        let cv = self.canvas_of(canvas).ok_or("no canvas")?;
        let page = cv.cells.get(cell).copied().flatten().ok_or("that cell is already empty")?;
        if cv.occupied() == 1 {
            self.navigate(page, tabs::NEWTAB_URL, false);
            return Ok(Some(json!({ "blanked": true })));
        }
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            cv.cells[cell] = None;
            if cv.focus == cell {
                cv.focus = cv.cells.iter().position(Option::is_some).unwrap_or(0);
            }
        }
        self.canvas_drop_page(page);
        self.canvas_refresh(canvas);
        Ok(Some(json!({ "closed": page.0 })))
    }

    /// Closing a page (Ctrl+W, the rail) inside a canvas clears its cell.
    pub(super) fn canvas_close_member(&mut self, canvas: TabId, page: TabId) {
        let cell = self.canvas_of(canvas).and_then(|cv| cv.cell_of(page));
        if let Some(cell) = cell {
            let _ = self.canvas_clear(&json!({ "canvas_id": canvas.0, "cell": cell }));
        }
    }

    /// Entries from a restored session: drop cells that point at missing pages.
    pub(super) fn canvas_sanitize(&mut self) {
        let canvases: Vec<TabId> = self.tabs.tabs().iter().filter(|t| t.canvas.is_some()).map(|t| t.id).collect();
        for c in canvases {
            let valid: Vec<Option<TabId>> = match self.canvas_of(c) {
                Some(cv) => cv
                    .cells
                    .iter()
                    .map(|cell| cell.filter(|m| self.tabs.get(*m).is_some_and(|t| t.member_of == Some(c))))
                    .collect(),
                None => continue,
            };
            if let Some(cv) = self.tabs.get_mut(c).and_then(|t| t.canvas.as_mut()) {
                cv.cells = valid;
                cv.cells.resize(cv.cols * cv.rows, None);
            }
            self.canvas_retitle(c);
        }
        // Pages whose canvas is gone are orphans: drop them.
        let orphans: Vec<TabId> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.member_of.is_some_and(|c| self.canvas_of(c).is_none_or(|cv| cv.cell_of(t.id).is_none())))
            .map(|t| t.id)
            .collect();
        for o in orphans {
            self.tabs.close(o);
        }
        if self.active.is_some_and(|a| self.tabs.get(a).is_none()) {
            self.active = self.tabs.last();
        }
        self.canvas_shown = self
            .active
            .and_then(|a| self.tabs.get(a))
            .and_then(|t| t.member_of)
            .filter(|c| self.canvas_of(*c).is_some());
    }

    /// A left click at screen position (x, y): focus the canvas cell under it.
    pub(super) fn canvas_mouse_down(&mut self, x: i32, y: i32) {
        if self.canvas_shown.is_none() {
            return;
        }
        let Some(origin) = self.window.as_ref().and_then(|w| w.inner_position().ok()) else { return };
        // Geometry is in logical units; the cursor and window origin are physical.
        let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0).max(0.1);
        let (cx, cy) = (((x - origin.x) as f64 / scale) as i32, ((y - origin.y) as f64 / scale) as i32);
        let area = self.canvas_area();
        if cx < area.x || cx >= area.x + area.w || cy < area.y || cy >= area.y + area.h {
            return;
        }
        let Some((cv, geoms)) = self.canvas_geoms() else { return };
        // Any spot in the canvas picks a cell: inside a frame, or the nearest one when
        // the click lands in a gap or beside the grid.
        let dist = |g: &CellGeom| {
            let dx = (g.frame.x - cx).max(cx - (g.frame.x + g.frame.w)).max(0);
            let dy = (g.frame.y - cy).max(cy - (g.frame.y + g.frame.h)).max(0);
            dx * dx + dy * dy
        };
        let hit = geoms.iter().enumerate().filter(|(_, g)| !g.hidden).min_by_key(|(_, g)| dist(g)).map(|(i, _)| i);
        if let Some(i) = hit.filter(|i| *i != cv.focus) {
            let _ = self.canvas_focus(&json!({ "cell": i }));
        }
    }

    /// The probe kind of a page message, e.g. `focus`, `sel`, `changed`.
    fn probe_kind(raw: &str) -> Option<String> {
        serde_json::from_str::<Value>(raw).ok()?.get("ai")?.as_str().map(str::to_string)
    }

    /// A page in a canvas cell reported the user acting in it: that cell takes focus.
    /// Returns true when the message was only a focus report.
    pub(super) fn canvas_probe(&mut self, tab: TabId, raw: &str) -> bool {
        if self.shown_canvas_of(tab).is_none() {
            return false;
        }
        match Self::probe_kind(raw).as_deref() {
            Some("focus") => {
                self.canvas_focus_tab(tab);
                true
            }
            // Selections and open requests can come from scripts; only real input
            // (the `focus` probe and the host mouse watcher) moves the focus.
            _ => false,
        }
    }
}
