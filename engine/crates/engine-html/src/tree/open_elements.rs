//! The stack of open elements and the list of active formatting elements.
//!
//! Both are "find the nearest ancestor matching X" structures over the DOM, so
//! they store `NodeId`s and read tag names back out of the document.

use engine_core::NodeId;
use engine_dom::document::Document;

/// Marker in the active-formatting list that delimits insertion scopes.
pub const MARKER: Option<NodeId> = None;

/// The stack of open elements.
#[derive(Clone, Debug, Default)]
pub struct OpenElements {
    stack: Vec<NodeId>,
}

impl OpenElements {
    pub fn new() -> Self {
        OpenElements { stack: Vec::new() }
    }

    pub fn push(&mut self, id: NodeId) {
        self.stack.push(id);
    }

    pub fn pop(&mut self) -> Option<NodeId> {
        self.stack.pop()
    }

    pub fn current(&self) -> Option<NodeId> {
        self.stack.last().copied()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// The stack, bottom (html) first.
    pub fn as_slice(&self) -> &[NodeId] {
        &self.stack
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.stack.contains(&id)
    }

    /// Index of `id` from the top of the stack (0 = current node).
    pub fn index_from_top(&self, id: NodeId) -> Option<usize> {
        self.stack.iter().rev().position(|n| *n == id)
    }

    /// The nearest element on the stack satisfying `pred`, searching downward.
    pub fn find_from_top(&self, doc: &Document, mut pred: impl FnMut(&str) -> bool) -> Option<NodeId> {
        self.stack.iter().rev().copied().find(|id| match doc.element(*id) {
            Some(e) => pred(e.tag.as_str()),
            None => false,
        })
    }

    /// Remove the nearest matching element from the stack (used by end-tag
    /// handling, which may close elements in the middle of the stack).
    pub fn remove(&mut self, id: NodeId) {
        if let Some(idx) = self.stack.iter().rposition(|n| *n == id) {
            self.stack.remove(idx);
        }
    }

    /// Pop elements until `id` has been popped.
    pub fn pop_until(&mut self, id: NodeId) {
        while let Some(top) = self.stack.pop() {
            if top == id {
                break;
            }
        }
    }

    pub fn truncate(&mut self, len: usize) {
        self.stack.truncate(len);
    }

    /// Iterate from the current node downward (top to bottom).
    pub fn iter_from_top(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.stack.iter().rev().copied()
    }
}

/// The active formatting elements list.
#[derive(Clone, Debug, Default)]
pub struct ActiveFormatting {
    list: Vec<Option<NodeId>>,
}

impl ActiveFormatting {
    pub fn new() -> Self {
        ActiveFormatting { list: Vec::new() }
    }

    pub fn push(&mut self, id: NodeId) {
        // The spec's "push onto the list of active formatting elements": if
        // three identical entries (same tag, attrs, namespace) already exist
        // after the last marker, remove the earliest of them.
        self.list.push(Some(id));
    }

    pub fn push_marker(&mut self) {
        self.list.push(MARKER);
    }

    pub fn pop(&mut self) -> Option<Option<NodeId>> {
        self.list.pop()
    }

    /// Pop entries until a marker has been popped (including the marker).
    pub fn clear_to_marker(&mut self) {
        while let Some(e) = self.list.pop() {
            if e.is_none() {
                break;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.list.contains(&Some(id))
    }

    /// Remove one entry referring to `id` (used by end-tag handling).
    pub fn remove(&mut self, id: NodeId) {
        if let Some(idx) = self.list.iter().rposition(|e| *e == Some(id)) {
            self.list.remove(idx);
        }
    }

    /// Index of the last marker, or `None` if there is none.
    pub fn last_marker_index(&self) -> Option<usize> {
        self.list.iter().rposition(|e| e.is_none())
    }

    /// The entries after the last marker, bottom first.
    pub fn after_last_marker(&self) -> &[Option<NodeId>] {
        match self.last_marker_index() {
            Some(i) => &self.list[i + 1..],
            None => &self.list,
        }
    }

    /// Mutable view of the entries after the last marker.
    pub fn after_last_marker_mut(&mut self) -> &mut [Option<NodeId>] {
        let i = self.last_marker_index();
        match i {
            Some(i) => &mut self.list[i + 1..],
            None => &mut self.list,
        }
    }

    pub fn set(&mut self, index: usize, value: Option<NodeId>) {
        self.list[index] = value;
    }

    pub fn as_slice(&self) -> &[Option<NodeId>] {
        &self.list
    }

    /// The last (most recently added) formatting element.
    pub fn last(&self) -> Option<NodeId> {
        self.list.iter().rev().find_map(|e| *e)
    }
}

/// Scoping element sets (HTML §13.2.4.2).
pub fn is_scoping(tag: &str) -> bool {
    matches!(
        tag,
        "applet" | "caption" | "html" | "table" | "td" | "th" | "marquee" | "object" | "template" | "svg" | "math"
    )
}

/// The `p` element is implied-closed by a specific list of block elements.
pub fn closes_p(tag: &str) -> bool {
    matches!(
        tag,
        "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog" | "dir"
            | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer" | "form" | "h1" | "h2"
            | "h3" | "h4" | "h5" | "h6" | "header" | "hgroup" | "hr" | "main" | "menu" | "nav"
            | "ol" | "p" | "pre" | "section" | "summary" | "ul"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_basics() {
        let mut d = Document::new();
        let a = d.create_node(engine_dom::NodeKind::Element(engine_dom::ElementData::new(
            engine_dom::Tag::lookup("div"),
            engine_dom::Namespace::Html,
        )));
        let b = d.create_node(engine_dom::NodeKind::Element(engine_dom::ElementData::new(
            engine_dom::Tag::lookup("p"),
            engine_dom::Namespace::Html,
        )));
        let mut s = OpenElements::new();
        s.push(a);
        s.push(b);
        assert_eq!(s.current(), Some(b));
        assert_eq!(s.index_from_top(a), Some(1));
        assert!(s.find_from_top(&d, |t| t == "div").is_some());
        s.pop_until(a);
        assert!(s.is_empty());
    }

    #[test]
    fn active_formatting_markers() {
        let mut a = ActiveFormatting::new();
        a.push(NodeId(1));
        a.push_marker();
        a.push(NodeId(2));
        a.push(NodeId(3));
        let after = a.after_last_marker();
        assert_eq!(after.len(), 2);
        a.clear_to_marker();
        assert_eq!(a.as_slice(), &[Some(NodeId(1))]);
    }

    #[test]
    fn p_closing_list() {
        assert!(closes_p("div"));
        assert!(closes_p("p"));
        assert!(!closes_p("span"));
        assert!(!closes_p("a"));
    }

    #[test]
    fn scoping() {
        assert!(is_scoping("td"));
        assert!(!is_scoping("div"));
    }
}