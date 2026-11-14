//! Tab and tab-group state.
//!
//! Pure bookkeeping: no webview handles here. `app.rs` owns the webviews and
//! keeps this in sync. Everything the UI consumes is derived from this module
//! so `state.tabs` is exactly one source of truth.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

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
    /// spatial, explicitly not MRU).
    pub fn step(&self, from: TabId, delta: isize) -> Option<TabId> {
        if self.tabs.is_empty() {
            return None;
        }
        let cur = self.position(from)?;
        let n = self.tabs.len() as isize;
        let next = (cur as isize + delta).rem_euclid(n) as usize;
        Some(self.tabs[next].id)
    }

    /// `Ctrl+1..8` → index; `Ctrl+9` → last. Returns None when out of range.
    pub fn at_index(&self, index: usize) -> Option<TabId> {
        self.tabs.get(index).map(|t| t.id)
    }

    pub fn last(&self) -> Option<TabId> {
        self.tabs.last().map(|t| t.id)
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