//! What the browser has told us about tabs and documents (`browser.event`).
//!
//! The utility has no tab authority; this is only the bookkeeping needed to
//! notice that a task's document is gone: a navigation replaces the tab's
//! document, a destroy or tab close ends it, a `pushState` re-sequences it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use ai_platform_contract::identity::DocumentToken;
use ai_platform_contract::model::BrowserEvent;

use crate::control::lock;

const MAX_DEAD: usize = 8192;

#[derive(Default)]
struct Inner {
    tab_document: HashMap<u64, DocumentToken>,
    document_tab: HashMap<u64, u64>,
    sequence: HashMap<u64, u64>,
    dead: HashSet<u64>,
    dead_order: VecDeque<u64>,
}

impl Inner {
    fn kill(&mut self, document: DocumentToken, out: &mut Invalidation) {
        if self.dead.insert(document.0) {
            self.dead_order.push_back(document.0);
            if self.dead_order.len() > MAX_DEAD {
                if let Some(old) = self.dead_order.pop_front() {
                    self.dead.remove(&old);
                }
            }
        }
        if let Some(tab) = self.document_tab.remove(&document.0) {
            if self.tab_document.get(&tab) == Some(&document) {
                self.tab_document.remove(&tab);
            }
        }
        self.sequence.remove(&document.0);
        out.dead.push(document);
    }
}

/// What an event invalidated.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Invalidation {
    pub dead: Vec<DocumentToken>,
    /// Documents whose sequence moved (same document, new history state).
    pub resequenced: Vec<(DocumentToken, u64)>,
}

#[derive(Default)]
pub struct Tabs {
    inner: Mutex<Inner>,
}

impl Tabs {
    pub fn apply(&self, event: &BrowserEvent) -> Invalidation {
        let mut inner = lock(&self.inner);
        let mut out = Invalidation::default();
        match event {
            BrowserEvent::NavigationCommitted { tab_id, document, navigation_sequence, replaced } => {
                if let Some(replaced) = replaced {
                    if replaced != document {
                        inner.kill(*replaced, &mut out);
                    }
                }
                if let Some(previous) = inner.tab_document.get(tab_id).copied() {
                    if previous != *document {
                        inner.kill(previous, &mut out);
                    }
                }
                inner.dead.remove(&document.0);
                inner.tab_document.insert(*tab_id, *document);
                inner.document_tab.insert(document.0, *tab_id);
                inner.sequence.insert(document.0, *navigation_sequence);
            }
            BrowserEvent::HistoryStateChanged { tab_id, document, navigation_sequence } => {
                inner.tab_document.insert(*tab_id, *document);
                inner.document_tab.insert(document.0, *tab_id);
                inner.sequence.insert(document.0, *navigation_sequence);
                out.resequenced.push((*document, *navigation_sequence));
            }
            BrowserEvent::DocumentDestroyed { document } => inner.kill(*document, &mut out),
            BrowserEvent::TabClosed { tab_id } => {
                if let Some(document) = inner.tab_document.get(tab_id).copied() {
                    inner.kill(document, &mut out);
                }
                inner.tab_document.remove(tab_id);
            }
            BrowserEvent::TabActivated { tab_id, document } => {
                if let Some(document) = document {
                    inner.tab_document.insert(*tab_id, *document);
                    inner.document_tab.insert(document.0, *tab_id);
                }
            }
            BrowserEvent::SelectionChanged { .. } | BrowserEvent::UserStopped { .. } | BrowserEvent::ConfigChanged => {}
        }
        out
    }

    /// Registers a task's document claim. `false` means it is already stale: the
    /// document is dead, or the tab/sequence the browser reported has moved on.
    pub fn claim(&self, tab_id: u64, document: DocumentToken, sequence: u64) -> bool {
        let mut inner = lock(&self.inner);
        if inner.dead.contains(&document.0) {
            return false;
        }
        if inner.tab_document.get(&tab_id).is_some_and(|current| *current != document) {
            return false;
        }
        if inner.sequence.get(&document.0).is_some_and(|known| *known != sequence) {
            return false;
        }
        inner.tab_document.insert(tab_id, document);
        inner.document_tab.insert(document.0, tab_id);
        inner.sequence.insert(document.0, sequence);
        true
    }

    pub fn document_of(&self, tab_id: u64) -> Option<DocumentToken> {
        lock(&self.inner).tab_document.get(&tab_id).copied()
    }

    pub fn is_dead(&self, document: DocumentToken) -> bool {
        lock(&self.inner).dead.contains(&document.0)
    }

    pub fn sequence_of(&self, document: DocumentToken) -> Option<u64> {
        lock(&self.inner).sequence.get(&document.0).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(tab: u64, doc: u64, seq: u64, replaced: Option<u64>) -> BrowserEvent {
        BrowserEvent::NavigationCommitted {
            tab_id: tab,
            document: DocumentToken(doc),
            navigation_sequence: seq,
            replaced: replaced.map(DocumentToken),
        }
    }

    #[test]
    fn navigation_kills_the_replaced_document() {
        let tabs = Tabs::default();
        tabs.apply(&nav(1, 10, 1, None));
        let out = tabs.apply(&nav(1, 11, 2, Some(10)));
        assert_eq!(out.dead, vec![DocumentToken(10)]);
        assert!(tabs.is_dead(DocumentToken(10)) && !tabs.is_dead(DocumentToken(11)));
        assert_eq!(tabs.document_of(1), Some(DocumentToken(11)));
    }

    #[test]
    fn destroy_close_and_pushstate() {
        let tabs = Tabs::default();
        tabs.apply(&nav(1, 10, 1, None));
        tabs.apply(&nav(2, 20, 1, None));
        assert_eq!(tabs.apply(&BrowserEvent::TabClosed { tab_id: 2 }).dead, vec![DocumentToken(20)]);
        let out = tabs.apply(&BrowserEvent::HistoryStateChanged { tab_id: 1, document: DocumentToken(10), navigation_sequence: 2 });
        assert_eq!(out.resequenced, vec![(DocumentToken(10), 2)]);
        assert_eq!(tabs.sequence_of(DocumentToken(10)), Some(2));
        assert_eq!(tabs.apply(&BrowserEvent::DocumentDestroyed { document: DocumentToken(10) }).dead, vec![DocumentToken(10)]);
        assert_eq!(tabs.document_of(1), None);
    }
}
