//! Tab and tab-group state.
//!
//! Pure bookkeeping: no webview handles here. `app.rs` owns the webviews and
//! keeps this in sync. Everything the UI consumes is derived from this module
//! so `state.tabs` is exactly one source of truth.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Maximum closed-tab history (§3.5 `Ctrl+Shift+T`: "stack of 25").
pub const CLOSED_STACK_LEN: usize = 25;

/// The internal new-tab page. Rendered by the shell UI, loaded in a page
/// webview like any other page (§4.1: "no privileged API").
pub const NEWTAB_URL: &str = "native://newtab";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupColor {
    Blue,
    Green,
    Amber,
    Red,
    Violet,
    Slate,
}

impl GroupColor {
    /// Cycle used when `group.create` omits a color.
    pub fn next(self) -> Self {
        match self {
            GroupColor::Blue => GroupColor::Green,
            GroupColor::Green => GroupColor::Amber,
            GroupColor::Amber => GroupColor::Red,
            GroupColor::Red => GroupColor::Violet,
            GroupColor::Violet => GroupColor::Slate,
            GroupColor::Slate => GroupColor::Blue,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub color: GroupColor,
    pub collapsed: bool,
}

/// Most columns/rows a canvas may have: at most 12 pages, 6 by default (3×2).
pub const CANVAS_MAX_COLS: usize = 4;
pub const CANVAS_MAX_ROWS: usize = 3;
pub const CANVAS_DEFAULT: (usize, usize) = (3, 2);
/// The URL of a canvas tab. A canvas has no page of its own: its cells do.
pub const CANVAS_URL: &str = "native://canvas";

/// A tab that is a grid of pages. Each occupied cell is an ordinary tab (so
/// navigation, titles, history and the assistant's tools work per cell) that is
/// hidden from the rail and points back here through `Tab::member_of`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub cols: usize,
    pub rows: usize,
    /// Row-major; `None` is an empty cell. Always `cols * rows` long.
    pub cells: Vec<Option<TabId>>,
    /// The cell that receives omnibox navigation, keyboard focus and AI context.
    pub focus: usize,
    /// The focused cell fills the whole canvas; the others stay alive behind it.
    pub zoomed: bool,
    /// Relative widths of the columns of each row (`col_ws[row]`; every row splits on its
    /// own) and relative row heights; empty means equal shares.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub col_ws: Vec<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_h: Vec<f64>,
    /// Covered pages, oldest first, for each cell. Only `cells` are visible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Vec<TabId>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub popup_next: BTreeMap<TabId, usize>,
}

impl Canvas {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.clamp(1, CANVAS_MAX_COLS);
        let rows = rows.clamp(1, CANVAS_MAX_ROWS);
        Self { cols, rows, cells: vec![None; cols * rows], focus: 0, zoomed: false, col_ws: Vec::new(), row_h: Vec::new(), layers: Vec::new(), popup_next: BTreeMap::new() }
    }

    pub fn capacity(&self) -> usize {
        self.cols * self.rows
    }

    pub fn occupied(&self) -> usize {
        self.cells.iter().flatten().count()
    }

    pub fn members(&self) -> Vec<TabId> {
        self.cells.iter().flatten().copied().collect()
    }

    pub fn cell_of(&self, id: TabId) -> Option<usize> {
        self.cells.iter().position(|c| *c == Some(id))
    }

    pub fn all_members(&self) -> Vec<TabId> {
        self.members().into_iter().chain(self.layers.iter().flatten().copied()).collect()
    }

    pub fn contains(&self, id: TabId) -> bool {
        self.cell_of(id).is_some() || self.layers.iter().any(|stack| stack.contains(&id))
    }

    /// Empty cells win; otherwise cycle row-major, always excluding the opener.
    pub fn popup_target(&mut self, opener: TabId) -> Option<usize> {
        let source = self.cell_of(opener)?;
        let next = *self.popup_next.get(&opener).unwrap_or(&0);
        let target = self.cells.iter().enumerate().find(|(i, cell)| *i != source && cell.is_none()).map(|(i, _)| i)
            .or_else(|| (0..self.capacity()).map(|offset| (next + offset) % self.capacity()).find(|i| *i != source))?;
        self.popup_next.insert(opener, (target + 1) % self.capacity());
        Some(target)
    }

    pub fn cover(&mut self, cell: usize, page: TabId) {
        self.layers.resize_with(self.capacity(), Vec::new);
        if let Some(previous) = self.cells[cell].replace(page) {
            self.layers[cell].push(previous);
        }
    }

    pub fn uncover(&mut self, cell: usize) -> Option<TabId> {
        let closed = self.cells[cell].take();
        self.cells[cell] = self.layers.get_mut(cell).and_then(Vec::pop);
        if let Some(closed) = closed {
            self.popup_next.remove(&closed);
        }
        closed
    }

    pub fn focused(&self) -> Option<TabId> {
        self.cells.get(self.focus).copied().flatten()
    }

    /// Change the grid shape. Pages keep their reading order; the ones that no longer
    /// fit are returned so the caller can close them.
    pub fn reshape(&mut self, cols: usize, rows: usize) -> Vec<TabId> {
        let cols = cols.clamp(1, CANVAS_MAX_COLS);
        let rows = rows.clamp(1, CANVAS_MAX_ROWS);
        let focused = self.focused();
        let mut stacks = std::mem::take(&mut self.layers);
        stacks.resize_with(self.capacity(), Vec::new);
        let pages: Vec<_> = self.cells.iter().copied().zip(stacks).filter_map(|(page, stack)| page.map(|id| (id, stack))).collect();
        let cap = cols * rows;
        let (keep, dropped) = pages.split_at(pages.len().min(cap));
        let dropped: Vec<_> = dropped.iter().flat_map(|(id, stack)| std::iter::once(*id).chain(stack.iter().copied())).collect();
        self.cols = cols;
        self.rows = rows;
        self.col_ws.clear();
        self.row_h.clear();
        self.cells = vec![None; cap];
        self.layers = vec![Vec::new(); cap];
        for (i, (id, stack)) in keep.iter().enumerate() {
            self.cells[i] = Some(*id);
            self.layers[i] = stack.clone();
        }
        self.popup_next.clear();
        self.focus = focused.and_then(|f| self.cell_of(f)).unwrap_or(0);
        dropped
    }

    /// First empty cell, if any.
    pub fn first_empty(&self) -> Option<usize> {
        self.cells.iter().position(Option::is_none)
    }
}

/// Everything the rail needs about a tab (§6.2 `state.tabs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group_id: Option<GroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    pub zoom: f64,
    pub loading: bool,
    /// "load started but not yet navigated" — drives the spinner.
    pub sleeping: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    #[serde(default)]
    pub scroll_y: f64,
    /// Set on a canvas tab (a grid of pages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<Canvas>,
    /// Set on a page that lives in a canvas cell; such a tab is not in the rail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_of: Option<TabId>,
}

impl Tab {
    pub fn new(id: TabId, url: &str) -> Self {
        Self {
            id,
            url: url.to_string(),
            title: String::new(),
            pinned: false,
            group_id: None,
            favicon: None,
            zoom: 1.0,
            loading: false,
            sleeping: false,
            can_go_back: false,
            can_go_forward: false,
            scroll_y: 0.0,
            canvas: None,
            member_of: None,
        }
    }

    /// Title as rendered in the rail: fall back to host, then URL.
    pub fn display_title(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        if self.url == NEWTAB_URL {
            return "New tab".to_string();
        }
        let host = crate::history::host_of(&self.url);
        if host.is_empty() {
            self.url.clone()
        } else {
            host.to_string()
        }
    }
}

/// A closed tab that can be reopened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group_id: Option<GroupId>,
    pub index: usize,
}

/// Ordered tab list. Order in `tabs` is the rail order (pinned first is a
/// *rendering* rule; the host keeps pinned-ness as a flag so unpinning does not
/// need to re-sort).
#[derive(Debug, Default)]
pub struct TabSet {
    tabs: Vec<Tab>,
    next_id: u64,
    next_group_id: u64,
    closed: VecDeque<ClosedTab>,
}

impl TabSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn tabs_mut(&mut self) -> &mut [Tab] {
        &mut self.tabs
    }

    pub fn position(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn alloc_id(&mut self) -> TabId {
        self.next_id += 1;
        TabId(self.next_id)
    }

    pub fn alloc_group_id(&mut self) -> GroupId {
        self.next_group_id += 1;
        GroupId(self.next_group_id)
    }

    /// Ensure a future id is not reused after a restore.
    pub fn observe_id(&mut self, id: TabId) {
        self.next_id = self.next_id.max(id.0);
    }

    pub fn observe_group_id(&mut self, id: GroupId) {
        self.next_group_id = self.next_group_id.max(id.0);
    }

    /// Append at the end when `after` is `None`; otherwise insert after the
    /// anchor tab (and after its group's tail, so groups stay contiguous).
    pub fn insert_after(&mut self, after: Option<TabId>, tab: Tab) -> TabId {
        let id = tab.id;
        let idx = match after.and_then(|a| self.position(a)) {
            Some(i) => {
                // Insert after the last tab of the same group so groups stay
                // contiguous, else directly after the anchor.
                let gid = tab.group_id;
                let mut j = i;
                while j + 1 < self.tabs.len()
                    && gid.is_some()
                    && self.tabs[j + 1].group_id == gid
                {
                    j += 1;
                }
                j + 1
            }
            None => self.tabs.len(),
        };
        self.tabs.insert(idx, tab);
        id
    }

    /// Insert at `index` clamped to the list length. Used by session restore so
    /// the saved order is preserved exactly.
    pub fn insert_at(&mut self, index: usize, tab: Tab) -> TabId {
        let id = tab.id;
        let idx = index.min(self.tabs.len());
        self.tabs.insert(idx, tab);
        id
    }

    /// Remove a tab, recording it for `Ctrl+Shift+T`. Returns the removed tab.
    pub fn close(&mut self, id: TabId) -> Option<Tab> {
        let idx = self.position(id)?;
        let tab = self.tabs.remove(idx);
        // Canvas cells and canvases are not reopened by Ctrl+Shift+T.
        if tab.member_of.is_some() || tab.canvas.is_some() {
            return Some(tab);
        }
        if self.closed.len() == CLOSED_STACK_LEN {
            self.closed.pop_front();
        }
        self.closed.push_back(ClosedTab {
            url: tab.url.clone(),
            title: tab.title.clone(),
            pinned: tab.pinned,
            group_id: tab.group_id,
            index: idx,
        });
        Some(tab)
    }

    pub fn take_closed(&mut self) -> Option<ClosedTab> {
        self.closed.pop_back()
    }

    /// The closed stack, oldest first, for session persistence.
    pub fn closed_snapshot(&self) -> Vec<ClosedTab> {
        self.closed.iter().cloned().collect()
    }

    pub fn closed_len(&self) -> usize {
        self.closed.len()
    }

    /// Reordering for drag-and-drop (§3.2). `before` and `after` are mutually
    /// exclusive; `group_id` moves the tab into/out of a group.
    pub fn move_tab(
        &mut self,
        id: TabId,
        before: Option<TabId>,
        after: Option<TabId>,
        group_id: Option<Option<GroupId>>,
    ) -> bool {
        let Some(from) = self.position(id) else {
            return false;
        };
        if before == Some(id) || after == Some(id) {
            return false;
        }
        let mut tab = self.tabs.remove(from);
        if let Some(g) = group_id {
            tab.group_id = g;
        }
        let target = if let Some(b) = before {
            self.position(b)
        } else if let Some(a) = after {
            self.position(a).map(|p| p + 1)
        } else {
            None
        };
        let idx = target.unwrap_or(self.tabs.len()).min(self.tabs.len());
        self.tabs.insert(idx, tab);
        true
    }

    /// Next/previous tab in spatial order with wrap-around (§3.5 `Ctrl+Tab` is
    /// spatial, explicitly not MRU). Canvas cells are skipped: a canvas is one stop.
    pub fn step(&self, from: TabId, delta: isize) -> Option<TabId> {
        let from = self.get(from).map(|t| t.member_of.unwrap_or(t.id))?;
        let rail: Vec<TabId> = self.rail().map(|t| t.id).collect();
        if rail.is_empty() {
            return None;
        }
        let cur = rail.iter().position(|id| *id == from)?;
        let n = rail.len() as isize;
        Some(rail[(cur as isize + delta).rem_euclid(n) as usize])
    }

    /// The tabs that have a rail row: everything except canvas cells.
    pub fn rail(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| t.member_of.is_none())
    }

    /// `Ctrl+1..8` → index; `Ctrl+9` → last. Returns None when out of range.
    pub fn at_index(&self, index: usize) -> Option<TabId> {
        self.rail().nth(index).map(|t| t.id)
    }

    pub fn last(&self) -> Option<TabId> {
        self.rail().last().map(|t| t.id)
    }

    pub fn pinned(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| t.pinned)
    }

    pub fn unpinned(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| !t.pinned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(set: &mut TabSet, url: &str) -> TabId {
        let id = set.alloc_id();
        set.insert_after(None, Tab::new(id, url));
        id
    }

    #[test]
    fn step_wraps_in_spatial_order() {
        let mut s = TabSet::new();
        let a = tab(&mut s, "a");
        let b = tab(&mut s, "b");
        let c = tab(&mut s, "c");
        assert_eq!(s.step(a, 1), Some(b));
        assert_eq!(s.step(c, 1), Some(a), "wraps forward");
        assert_eq!(s.step(a, -1), Some(c), "wraps backward");
        assert_eq!(s.step(b, -1), Some(a));
    }

    #[test]
    fn canvas_reshape_keeps_pages_in_order_and_returns_the_overflow() {
        let mut c = Canvas::new(3, 2);
        assert_eq!(c.capacity(), 6);
        for i in 0..6 {
            c.cells[i] = Some(TabId(10 + i as u64));
        }
        c.focus = 4;
        let dropped = c.reshape(2, 2);
        assert_eq!(dropped, vec![TabId(14), TabId(15)]);
        assert_eq!(c.members(), vec![TabId(10), TabId(11), TabId(12), TabId(13)]);
        assert_eq!(c.focus, 0, "a focused page that no longer fits falls back to the first");
        assert!(c.reshape(9, 9).is_empty());
        assert_eq!((c.cols, c.rows), (CANVAS_MAX_COLS, CANVAS_MAX_ROWS));
        assert_eq!(c.cells.len(), c.capacity());
    }

    #[test]
    fn canvas_cells_are_not_rail_stops_and_are_not_reopened() {
        let mut s = TabSet::new();
        let a = tab(&mut s, "a");
        let cv = s.alloc_id();
        let mut canvas = Tab::new(cv, CANVAS_URL);
        canvas.canvas = Some(Canvas::new(2, 1));
        s.insert_after(None, canvas);
        let m = s.alloc_id();
        let mut member = Tab::new(m, "m");
        member.member_of = Some(cv);
        s.insert_after(Some(cv), member);
        let z = tab(&mut s, "z");
        assert_eq!(s.step(a, 1), Some(cv));
        assert_eq!(s.step(cv, 1), Some(z), "the cell is skipped");
        assert_eq!(s.step(m, 1), Some(z), "a cell steps from its canvas");
        assert_eq!(s.at_index(1), Some(cv));
        assert_eq!(s.at_index(2), Some(z));
        s.close(m).unwrap();
        s.close(cv).unwrap();
        assert_eq!(s.closed_len(), 0);
    }

    #[test]
    fn canvas_popups_use_empty_cells_then_cycle_without_covering_the_opener() {
        let mut c = Canvas::new(2, 2);
        c.cover(0, TabId(1));
        assert_eq!(c.popup_target(TabId(1)), Some(1));
        c.cover(1, TabId(2));
        assert_eq!(c.popup_target(TabId(1)), Some(2));
        c.cover(2, TabId(3));
        assert_eq!(c.popup_target(TabId(1)), Some(3));
        c.cover(3, TabId(4));
        for expected in [1, 2, 3, 1] {
            assert_eq!(c.popup_target(TabId(1)), Some(expected));
        }
        for expected in [0, 1, 2, 0] {
            assert_eq!(c.popup_target(TabId(4)), Some(expected));
        }
        c.cells[2] = None;
        assert_eq!(c.popup_target(TabId(1)), Some(2), "empty cells take priority over the cursor");
        let mut single = Canvas::new(1, 1);
        single.cover(0, TabId(1));
        assert_eq!(single.popup_target(TabId(1)), None);
    }

    #[test]
    fn canvas_layers_restore_in_order_survive_serialization_and_follow_reshape() {
        let mut c = Canvas::new(2, 2);
        c.cover(0, TabId(1));
        c.cover(0, TabId(2));
        c.cover(0, TabId(3));
        c.cover(3, TabId(4));
        c.cover(3, TabId(5));
        c.popup_target(TabId(3));
        let mut c: Canvas = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert!(c.contains(TabId(1)));
        assert_eq!(c.occupied(), 2);
        assert_eq!(c.uncover(0), Some(TabId(3)));
        assert_eq!(c.cells[0], Some(TabId(2)));
        assert_eq!(c.uncover(0), Some(TabId(2)));
        assert_eq!(c.cells[0], Some(TabId(1)));
        assert!(c.reshape(2, 1).is_empty());
        assert_eq!(c.cells[1], Some(TabId(5)));
        assert_eq!(c.reshape(1, 1), vec![TabId(5), TabId(4)]);
        assert_eq!(c.uncover(0), Some(TabId(1)));
        assert_eq!(c.occupied(), 0);
        assert!(c.all_members().is_empty());
    }

    #[test]
    fn close_records_on_bounded_stack() {
        let mut s = TabSet::new();
        let ids: Vec<TabId> = (0..30).map(|i| tab(&mut s, &format!("u{i}"))).collect();
        for id in &ids {
            s.close(*id).unwrap();
        }
        assert_eq!(s.closed_len(), CLOSED_STACK_LEN);
        // The 25 most recently closed survive: u29 down to u5.
        let newest = s.take_closed().unwrap();
        assert_eq!(newest.url, "u29");
        let mut last = newest;
        while let Some(c) = s.take_closed() {
            last = c;
        }
        assert_eq!(last.url, "u5");
    }

    #[test]
    fn reopen_restores_saved_index() {
        let mut s = TabSet::new();
        tab(&mut s, "a");
        let b = tab(&mut s, "b");
        tab(&mut s, "c");
        s.close(b);
        let closed = s.take_closed().unwrap();
        assert_eq!(closed.index, 1);
        let id = s.alloc_id();
        s.insert_at(closed.index, Tab::new(id, &closed.url));
        assert_eq!(s.position(id), Some(1));
    }

    #[test]
    fn move_reorders_and_regroups() {
        let mut s = TabSet::new();
        let a = tab(&mut s, "a");
        let b = tab(&mut s, "b");
        let c = tab(&mut s, "c");
        assert!(s.move_tab(c, Some(a), None, None));
        assert_eq!(s.tabs().iter().map(|t| t.id).collect::<Vec<_>>(), vec![c, a, b]);
        let g = s.alloc_group_id();
        assert!(s.move_tab(b, None, Some(c), Some(Some(g))));
        assert_eq!(s.get(b).unwrap().group_id, Some(g));
        // Moving a tab relative to itself is a no-op, not a deletion.
        assert!(!s.move_tab(b, Some(b), None, None));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn insert_after_groups_new_tabs_with_their_group() {
        let mut s = TabSet::new();
        let g = s.alloc_group_id();
        let a = tab(&mut s, "a");
        let mut t1 = Tab::new(s.alloc_id(), "g1");
        t1.group_id = Some(g);
        s.insert_after(Some(a), t1);
        let mut t2 = Tab::new(s.alloc_id(), "g2");
        t2.group_id = Some(g);
        let id2 = s.insert_after(Some(a), t2);
        // A second child of g lands after the first, not between a and g1.
        assert_eq!(s.position(id2), Some(2));
    }

    #[test]
    fn ids_never_reuse_after_observe() {
        let mut s = TabSet::new();
        s.observe_id(TabId(50));
        s.observe_group_id(GroupId(7));
        assert_eq!(s.alloc_id(), TabId(51));
        assert_eq!(s.alloc_group_id(), GroupId(8));
    }

    #[test]
    fn display_title_falls_back() {
        let mut t = Tab::new(TabId(1), "https://example.com/x");
        assert_eq!(t.display_title(), "example.com");
        t.title = "Real".into();
        assert_eq!(t.display_title(), "Real");
        let nt = Tab::new(TabId(2), NEWTAB_URL);
        assert_eq!(nt.display_title(), "New tab");
    }
}