//! The only module that mutates DOM link fields (`parent`, `first_child`,
//! `last_child`, `prev_sibling`, `next_sibling`). Everything - tree
//! construction and the JS bindings - goes through these operations, so the
//! link invariants are maintained in one place.
//!
//! Invariants after every operation:
//! - `n.prev_sibling` has `next_sibling == n` and vice versa;
//! - first child has no `prev_sibling`, last child has no `next_sibling`;
//! - a node's `parent` agrees with its siblings' parent;
//! - a node is never its own ancestor (checked on insert).

use engine_core::NodeId;

use crate::document::Document;
use crate::node::NodeKind;

/// Why an insertion was rejected. These are programmer errors in the DOM API
/// (a browser throws `HierarchyRequestError`), so the caller decides whether to
/// surface them; tree construction never hits them, and the JS bindings map
/// them to a DOM exception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomError {
    /// The new parent is a text/comment node, or the child would become its
    /// own ancestor.
    HierarchyRequest,
    /// The reference child is not actually a child of the given parent.
    NotFound,
}

impl std::fmt::Display for DomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomError::HierarchyRequest => f.write_str("hierarchy request error"),
            DomError::NotFound => f.write_str("not found error"),
        }
    }
}

impl std::error::Error for DomError {}

/// Whether a node kind may contain children.
fn can_have_children(kind: &NodeKind) -> bool {
    !matches!(kind, NodeKind::Text(_) | NodeKind::Comment(_) | NodeKind::Doctype { .. })
}

/// Is `maybe_ancestor` an inclusive ancestor of `node`?
fn is_inclusive_ancestor(doc: &Document, maybe_ancestor: NodeId, node: NodeId) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n == maybe_ancestor {
            return true;
        }
        cur = doc.parent(n);
    }
    false
}

/// Detach `node` from its parent, if it has one. Returns the old parent.
pub fn remove_from_parent(doc: &mut Document, node: NodeId) -> Option<NodeId> {
    let (parent, prev, next) = {
        let n = doc.slot_ref(node);
        (n.parent, n.prev_sibling, n.next_sibling)
    };
    let parent = parent?;

    if let Some(p) = prev {
        doc.slot(p).next_sibling = next;
    } else {
        doc.slot(parent).first_child = next;
    }
    if let Some(nx) = next {
        doc.slot(nx).prev_sibling = prev;
    } else {
        doc.slot(parent).last_child = prev;
    }
    let s = doc.slot(node);
    s.parent = None;
    s.prev_sibling = None;
    s.next_sibling = None;
    Some(parent)
}

/// Insert `node` as the last child of `parent`.
pub fn append_child(doc: &mut Document, parent: NodeId, node: NodeId) -> Result<(), DomError> {
    if !can_have_children(doc.kind(parent)) || is_inclusive_ancestor(doc, node, parent) {
        return Err(DomError::HierarchyRequest);
    }
    remove_from_parent(doc, node);

    let last = doc.slot_ref(parent).last_child;
    if let Some(l) = last {
        doc.slot(l).next_sibling = Some(node);
    } else {
        doc.slot(parent).first_child = Some(node);
    }
    doc.slot(parent).last_child = Some(node);
    let s = doc.slot(node);
    s.parent = Some(parent);
    s.prev_sibling = last;
    s.next_sibling = None;
    doc.slot(parent).dirty = true;
    doc.slot(node).dirty = true;
    Ok(())
}

/// Insert `node` immediately before `reference`, which must be a child of
/// `parent`.
pub fn insert_before(
    doc: &mut Document,
    parent: NodeId,
    node: NodeId,
    reference: NodeId,
) -> Result<(), DomError> {
    if !can_have_children(doc.kind(parent)) || is_inclusive_ancestor(doc, node, parent) {
        return Err(DomError::HierarchyRequest);
    }
    if doc.slot_ref(reference).parent != Some(parent) {
        return Err(DomError::NotFound);
    }
    if node == reference {
        // Moving a node before itself is a no-op.
        return Ok(());
    }
    remove_from_parent(doc, node);

    let prev = doc.slot_ref(reference).prev_sibling;
    if let Some(p) = prev {
        doc.slot(p).next_sibling = Some(node);
    } else {
        doc.slot(parent).first_child = Some(node);
    }
    doc.slot(reference).prev_sibling = Some(node);
    let s = doc.slot(node);
    s.parent = Some(parent);
    s.prev_sibling = prev;
    s.next_sibling = Some(reference);
    doc.slot(parent).dirty = true;
    doc.slot(node).dirty = true;
    Ok(())
}

/// Detach `node` from the tree. Returns the former parent.
pub fn remove(doc: &mut Document, node: NodeId) -> Option<NodeId> {
    let p = remove_from_parent(doc, node);
    if let Some(p) = p {
        doc.slot(p).dirty = true;
    }
    p
}

/// Replace `old` with `new` in `old`'s position.
pub fn replace_child(doc: &mut Document, old: NodeId, new: NodeId) -> Result<(), DomError> {
    if old == new {
        return Ok(());
    }
    let parent = doc.slot_ref(old).parent.ok_or(DomError::NotFound)?;
    if is_inclusive_ancestor(doc, new, parent) {
        return Err(DomError::HierarchyRequest);
    }
    let next = doc.slot_ref(old).next_sibling;
    if let Some(n) = next {
        insert_before(doc, parent, new, n)?;
    } else {
        append_child(doc, parent, new)?;
    }
    remove_from_parent(doc, old);
    Ok(())
}

/// Move `node` into `parent`, appending it (the re-parent used by the
/// adoption-agency algorithm).
pub fn reparent(doc: &mut Document, node: NodeId, parent: NodeId) -> Result<(), DomError> {
    append_child(doc, parent, node)
}

/// Number of children. O(n) - used by tree construction tests, not hot paths.
pub fn child_count(doc: &Document, parent: NodeId) -> usize {
    let mut n = 0;
    let mut cur = doc.first_child(parent);
    while let Some(c) = cur {
        n += 1;
        cur = doc.next_sibling(c);
    }
    n
}

/// `n`-th child (0-based).
pub fn nth_child(doc: &Document, parent: NodeId, n: usize) -> Option<NodeId> {
    let mut cur = doc.first_child(parent);
    for _ in 0..n {
        cur = doc.next_sibling(cur?);
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Namespace, Tag};
    use crate::node::ElementData;

    fn el(doc: &mut Document, tag: &str) -> NodeId {
        doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup(tag), Namespace::Html)))
    }

    fn children(doc: &Document, parent: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = doc.first_child(parent);
        while let Some(c) = cur {
            out.push(c);
            cur = doc.next_sibling(c);
        }
        out
    }

    /// Walk every node's links and assert they agree.
    fn assert_links_consistent(doc: &Document) {
        for id in doc.ids() {
            let n = doc.node(id);
            if let Some(fc) = n.first_child {
                assert_eq!(doc.node(fc).prev_sibling, None, "first child has no prev");
            }
            if let Some(lc) = n.last_child {
                assert_eq!(doc.node(lc).next_sibling, None, "last child has no next");
            }
            if let Some(p) = n.prev_sibling {
                assert_eq!(doc.node(p).next_sibling, Some(id), "prev/next disagree");
                assert_eq!(doc.node(p).parent, n.parent, "siblings share a parent");
            }
            if let Some(nx) = n.next_sibling {
                assert_eq!(doc.node(nx).prev_sibling, Some(id), "next/prev disagree");
                assert_eq!(doc.node(nx).parent, n.parent, "siblings share a parent");
            }
        }
    }

    #[test]
    fn append_orders_children() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        d.append_to_root(a).unwrap();
        d.append_to_root(b).unwrap();
        d.append_to_root(c).unwrap();
        let root = d.root();
        assert_eq!(children(&d, root), vec![a, b, c]);
        assert_eq!(d.parent(a), Some(root));
        assert_links_consistent(&d);
    }

    #[test]
    fn insert_before_middle() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let x = el(&mut d, "x");
        d.append_to_root(a).unwrap();
        d.append_to_root(b).unwrap();
        d.insert_before_in_root(x, b).unwrap();
        let root = d.root();
        assert_eq!(children(&d, root), vec![a, x, b]);
        assert_links_consistent(&d);
    }

    #[test]
    fn insert_before_first() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let x = el(&mut d, "x");
        d.append_to_root(a).unwrap();
        d.insert_before_in_root(x, a).unwrap();
        let root = d.root();
        assert_eq!(children(&d, root), vec![x, a]);
        assert_eq!(d.node(root).first_child, Some(x));
        assert_links_consistent(&d);
    }

    #[test]
    fn remove_middle_relinks_neighbours() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        d.append_to_root(a).unwrap();
        d.append_to_root(b).unwrap();
        d.append_to_root(c).unwrap();
        remove(&mut d, b);
        let root = d.root();
        assert_eq!(children(&d, root), vec![a, c]);
        assert_eq!(d.next_sibling(a), Some(c));
        assert_links_consistent(&d);
    }

    #[test]
    fn remove_first_and_last() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        d.append_to_root(a).unwrap();
        d.append_to_root(b).unwrap();
        let root = d.root();
        remove(&mut d, a);
        assert_eq!(d.node(root).first_child, Some(b));
        remove(&mut d, b);
        assert_eq!(d.node(root).first_child, None);
        assert_eq!(d.node(root).last_child, None);
        assert_links_consistent(&d);
    }

    #[test]
    fn append_moves_existing_node() {
        let mut d = Document::new();
        let p1 = el(&mut d, "p");
        let p2 = el(&mut d, "p");
        let x = el(&mut d, "x");
        d.append_to_root(p1).unwrap();
        d.append_to_root(p2).unwrap();
        append_child(&mut d, p1, x).unwrap();
        append_child(&mut d, p2, x).unwrap();
        assert_eq!(children(&d, p1), Vec::<NodeId>::new());
        assert_eq!(children(&d, p2), vec![x]);
        assert_links_consistent(&d);
    }

    #[test]
    fn text_cannot_have_children() {
        let mut d = Document::new();
        let t = d.create_node(NodeKind::Text("hi".into()));
        let a = el(&mut d, "a");
        assert_eq!(append_child(&mut d, t, a), Err(DomError::HierarchyRequest));
    }

    #[test]
    fn cannot_make_ancestor_its_own_child() {
        let mut d = Document::new();
        let p = el(&mut d, "p");
        let c = el(&mut d, "c");
        append_child(&mut d, p, c).unwrap();
        assert_eq!(append_child(&mut d, c, p), Err(DomError::HierarchyRequest));
        assert_eq!(append_child(&mut d, p, p), Err(DomError::HierarchyRequest));
    }

    #[test]
    fn replace_child_keeps_position() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        let x = el(&mut d, "x");
        d.append_to_root(a).unwrap();
        d.append_to_root(b).unwrap();
        d.append_to_root(c).unwrap();
        replace_child(&mut d, b, x).unwrap();
        let root = d.root();
        assert_eq!(children(&d, root), vec![a, x, c]);
        assert_links_consistent(&d);
    }

    #[test]
    fn child_count_and_nth() {
        let mut d = Document::new();
        let mut ns = Vec::new();
        for _ in 0..4 {
            let n = el(&mut d, "i");
            d.append_to_root(n).unwrap();
            ns.push(n);
        }
        let root = d.root();
        assert_eq!(child_count(&d, root), 4);
        assert_eq!(nth_child(&d, root, 2), Some(ns[2]));
        assert_eq!(nth_child(&d, root, 4), None);
    }

    #[test]
    fn detach_then_reattach_preserves_subtree() {
        let mut d = Document::new();
        let p = el(&mut d, "p");
        let c1 = el(&mut d, "c");
        let c2 = el(&mut d, "c");
        append_child(&mut d, p, c1).unwrap();
        append_child(&mut d, p, c2).unwrap();
        d.append_to_root(p).unwrap();
        remove(&mut d, p);
        assert_eq!(children(&d, p), vec![c1, c2], "subtree intact after detach");
        d.append_to_root(p).unwrap();
        assert_links_consistent(&d);
    }
}