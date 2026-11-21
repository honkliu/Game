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

    pub(super) fn canvas_area_rect(&self) -> Rc {
        self.canvas_area()
    }

    fn canvas_area(&self) -> Rc {
        let l = self.layout();
        Rc { x: l.page_x, y: l.page_y, w: l.page_w, h: l.page_h }
    }

    fn canvas_geoms(&self) -> Option<(Canvas, Vec<CellGeom>)> {
        let cv = self.canvas_of(self.canvas_shown?)?.clone();
        let geoms = layout::canvas_cells(self.canvas_area(), cv.cols, cv.rows, cv.zoomed.then_some(cv.focus), &cv.col_ws, &cv.row_h);
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

    pub(super) fn wv_canvas_state_get(&self) -> Value {
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
        let dividers: Vec<Value> = if cv.zoomed {
            Vec::new()
        } else {
            layout::canvas_dividers(self.canvas_area(), cv.cols, cv.rows, &cv.col_ws, &cv.row_h)
                .iter()
                .map(|d| json!({ "vertical": d.vertical, "index": d.index, "row": d.row, "rect": rc(d.rect) }))
                .collect()
        };
        json!({
            "id": id.0,
            "cols": cv.cols,
            "rows": cv.rows,
            "capacity": cv.capacity(),
            "zoomed": cv.zoomed,
            "focus": cv.focus,
            "cells": cells,
            "dividers": dividers,
        })
    }

    /// `canvas.resize {vertical, index, pos}` drags a boundary (`pos` in window units);
    /// `canvas.resize {reset: true}` restores equal shares.
    pub(super) fn wv_canvas_resize(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        let area = self.canvas_area();
        let cv = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()).ok_or("no canvas")?;
        if p.get("reset").and_then(Value::as_bool) == Some(true) {
            cv.col_ws.clear();
            cv.row_h.clear();
        } else {
            let vertical = p.get("vertical").and_then(Value::as_bool).unwrap_or(true);
            let index = p.get("index").and_then(Value::as_u64).ok_or("canvas.resize: index")? as usize;
            let pos = p.get("pos").and_then(Value::as_f64).ok_or("canvas.resize: pos")?;
            if vertical {
                let row = (p.get("row").and_then(Value::as_u64).unwrap_or(0) as usize).min(cv.rows.saturating_sub(1));
                if cv.col_ws.len() != cv.rows {
                    cv.col_ws.resize(cv.rows, Vec::new());
                }
                cv.col_ws[row] = layout::drag_divider(&cv.col_ws[row], cv.cols, index, pos, area.x, area.w);
            } else {
                cv.row_h = layout::drag_divider(&cv.row_h, cv.rows, index, pos, area.y, area.h);
            }
        }
        // Only the geometry changed: re-tile without the focus, AI and tab-list work a
        // full `canvas_refresh` does, so dragging a divider stays cheap.
        if self.canvas_shown == Some(canvas) {
            self.apply_layout();
        }
        Ok(None)
    }

    /// What an agent needs to see after a canvas action: which pages sit in which cell.
    pub(super) fn wv_canvas_state_brief(&self) -> Value {
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
        let Some(cv) = self.canvas_of(canvas).cloned() else { return };
        let focused = cv.focused();
        self.canvas_shown = Some(canvas);
        self.active = focused;
        let members = cv.members();
        for m in &members {
            self.wv_webview_create(*m);
        }
        let others: Vec<TabId> = self.live.keys().copied().filter(|k| !members.contains(k)).collect();
        for o in others {
            if let Some(live) = self.live.get(&o) {
                let _ = live.webview.set_visible(false);
            }
        }
        self.apply_layout();
        self.wv_webviews_suspend_background(&cv.all_members());
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
        } else {
            if let Some(shell) = self.shell.as_ref() {
                let _ = shell.focus();
            }
            self.ai_hide_toolbar();
            let status = self.wv_ai_status_get();
            self.emit("ai.context", status);
        }
        self.agent_context_changed();
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
    fn wv_canvas_page_open(&mut self, canvas: TabId, cell: usize, url: &str) -> TabId {
        let id = self.tabs.alloc_id();
        let mut page = Tab::new(id, url);
        page.member_of = Some(canvas);
        let after = self
            .canvas_of(canvas)
            .and_then(|cv| cv.members().last().copied())
            .unwrap_or(canvas);
        self.tabs.insert_after(Some(after), page);
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            cv.cover(cell, id);
        }
        id
    }

    pub(super) fn wv_canvas_popup_open(&mut self, canvas: TabId, cell: usize, url: &str) {
        self.wv_canvas_page_open(canvas, cell, url);
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            cv.focus = cell;
        }
        self.canvas_refresh(canvas);
    }

    /// Forget a page: its webview, AI state and tab entry.
    fn wv_canvas_page_destroy(&mut self, id: TabId) {
        self.live.remove(&id);
        self.ai_on_tab_closed(id);
        self.last_nav_emit.remove(&id);
        if self.tabs.close(id).is_some() {
            self.emit("tab.closed", json!({ "tab_id": id.0 }));
        }
    }

    pub(super) fn wv_canvas_close(&mut self, canvas: TabId) {
        let members = self.canvas_of(canvas).map(Canvas::all_members).unwrap_or_default();
        for m in members {
            self.wv_canvas_page_destroy(m);
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

    pub(super) fn cell_arg(&self, canvas: TabId, p: &Value) -> Option<usize> {
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
    pub(super) fn wv_canvas_open(&mut self, p: &Value) -> Result<Option<Value>, String> {
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
        for (i, u) in urls.iter().enumerate() {
            self.wv_canvas_page_open(id, i, u);
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
    pub(super) fn wv_canvas_cell_open(&mut self, p: &Value) -> Result<Option<Value>, String> {
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
                    self.wv_webview_create(existing);
                }
                self.wv_page_navigate(existing, &url, true);
                existing
            }
            None => self.wv_canvas_page_open(canvas, cell, &url),
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
    pub(super) fn wv_canvas_page_focus(&mut self, id: TabId) {
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
    pub(super) fn wv_canvas_cell_focus(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        if self.canvas_shown != Some(canvas) {
            self.show_canvas(canvas, None);
        }
        if let Some(step) = p.get("step").and_then(Value::as_i64) {
            let cv = self.canvas_of(canvas).ok_or("no canvas")?;
            let cell = (cv.focus as i64 + step).rem_euclid(cv.capacity() as i64) as usize;
            return self.wv_canvas_cell_focus(&json!({ "canvas_id": canvas.0, "cell": cell }));
        }
        if let Some(c) = self.cell_arg(canvas, p) {
            let empty = self.canvas_of(canvas).is_some_and(|cv| c < cv.cells.len() && cv.cells[c].is_none());
            if empty {
                if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
                    cv.focus = c;
                }
                self.canvas_refresh(canvas);
                return Ok(Some(json!({ "tab_id": null })));
            }
        }
        let cv = self.canvas_of(canvas).ok_or("no canvas")?;
        let target = match self.cell_arg(canvas, p) {
            Some(c) => cv.cells.get(c).copied().flatten(),
            None => None,
        };
        let id = target.ok_or("that cell has no page")?;
        self.wv_canvas_page_focus(id);
        Ok(Some(json!({ "tab_id": id.0 })))
    }

    /// `canvas.zoom {zoomed?}` — the focused page fills the canvas; again to go back.
    pub(super) fn wv_canvas_zoom(&mut self, p: &Value) -> Result<Option<Value>, String> {
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
    pub(super) fn wv_canvas_layout_set(&mut self, p: &Value) -> Result<Option<Value>, String> {
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
            self.wv_canvas_page_destroy(*d);
        }
        self.canvas_refresh(canvas);
        Ok(Some(json!({ "cols": cols, "rows": rows, "closed": dropped.len() })))
    }

    /// Close the top page, revealing its predecessor or leaving the cell empty.
    pub(super) fn wv_canvas_cell_close(&mut self, p: &Value) -> Result<Option<Value>, String> {
        let canvas = self.canvas_arg(p)?;
        let cell = self.cell_arg(canvas, p).ok_or("canvas.clear: say which cell")?;
        let cv = self.canvas_of(canvas).ok_or("no canvas")?;
        let page = cv.cells.get(cell).copied().flatten().ok_or("that cell is already empty")?;
        if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            cv.uncover(cell);
        }
        self.wv_canvas_page_destroy(page);
        self.canvas_refresh(canvas);
        Ok(Some(json!({ "closed": page.0 })))
    }

    pub(super) fn wv_canvas_focused_cell_clear(&mut self, canvas: TabId) {
        let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) else { return };
        let closed = cv.clear_focused_cell();
        if closed.is_empty() {
            self.wv_tab_close(canvas);
            return;
        }
        for page in closed {
            self.wv_canvas_page_destroy(page);
        }
        self.canvas_refresh(canvas);
    }

    /// Closing a page via the rail or API reveals the previous layer.
    pub(super) fn wv_canvas_member_close(&mut self, canvas: TabId, page: TabId) {
        let cell = self.canvas_of(canvas).and_then(|cv| cv.cell_of(page));
        if let Some(cell) = cell {
            let _ = self.wv_canvas_cell_close(&json!({ "canvas_id": canvas.0, "cell": cell }));
        } else if let Some(cv) = self.tabs.get_mut(canvas).and_then(|t| t.canvas.as_mut()) {
            for stack in &mut cv.layers {
                stack.retain(|id| *id != page);
            }
            cv.popup_next.remove(&page);
            self.wv_canvas_page_destroy(page);
            self.canvas_refresh(canvas);
        }
    }

    /// Entries from a restored session: drop cells that point at missing pages.
    pub(super) fn canvas_sanitize(&mut self) {
        let canvases: Vec<TabId> = self.tabs.tabs().iter().filter(|t| t.canvas.is_some()).map(|t| t.id).collect();
        for c in canvases {
            let Some(cv) = self.canvas_of(c) else { continue };
            let mut cells = Vec::with_capacity(cv.capacity());
            let mut layers = Vec::with_capacity(cv.capacity());
            // Older sessions allocated New Tab pages merely to focus empty cells.
            for i in 0..cv.capacity() {
                let mut stack: Vec<TabId> = cv.layers.get(i).into_iter().flatten().copied()
                    .chain(cv.cells.get(i).copied().flatten())
                    .filter(|id| self.tabs.get(*id).is_some_and(|t| t.member_of == Some(c) && t.url != tabs::NEWTAB_URL))
                    .collect();
                cells.push(stack.pop());
                layers.push(stack);
            }
            if let Some(cv) = self.tabs.get_mut(c).and_then(|t| t.canvas.as_mut()) {
                cv.cells = cells;
                cv.layers = layers;
                cv.focus = cv.focus.min(cv.capacity().saturating_sub(1));
            }
            self.canvas_retitle(c);
        }
        // Pages whose canvas is gone are orphans: drop them.
        let orphans: Vec<TabId> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.member_of.is_some_and(|c| self.canvas_of(c).is_none_or(|cv| !cv.contains(t.id))))
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
            .and_then(|t| t.member_of.or_else(|| t.canvas.as_ref().map(|_| t.id)))
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
        self.canvas_local_mouse_down(cx, cy);
    }

    pub(super) fn canvas_local_mouse_down(&mut self, cx: i32, cy: i32) {
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
            let _ = self.wv_canvas_cell_focus(&json!({ "cell": i }));
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
                self.wv_canvas_page_focus(tab);
                true
            }
            // Selections and open requests can come from scripts; only real input
            // (the `focus` probe and the host mouse watcher) moves the focus.
            _ => false,
        }
    }
}
