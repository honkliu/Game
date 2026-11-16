//! Dirty-flag propagation used to schedule restyle and relayout.
//!
//! A node is dirty when its own computed style or geometry may have changed.
//! Any change below a node invalidates that node's style only if the change
//! affects a selector that could match the ancestor (a descendant or `:has()`
//! dependency) - which we conservatively assume. The conservative cost is
//! re-styling a short ancestor chain; the alternative is a dependency-tracking
//! graph this engine does not need yet.

use engine_core::NodeId;

use crate::document::Document;

/// Mark `id` and all its ancestors dirty.
pub fn invalidate(doc: &mut Document, id: NodeId) {
    let mut cur = Some(id);
    while let Some(n) = cur {
        let node = doc.node_mut(n);
        if node.dirty {
            break;
        }
        node.dirty = true;
        cur = node.parent;
    }
}

/// Mark only `id` dirty (a change that cannot affect ancestors).
pub fn invalidate_self(doc: &mut Document, id: NodeId) {
    doc.node_mut(id).dirty = true;
}

/// The dirty set, in arena order. Restyle walks this.
pub fn dirty_nodes(doc: &Document) -> impl Iterator<Item = NodeId> + '_ {
    doc.ids().filter(move |id| doc.node(*id).dirty)
}

pub fn any_dirty(doc: &Document) -> bool {
    doc.ids().any(|id| doc.node(id).dirty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Namespace, Tag};
    use crate::node::{ElementData, NodeKind};
    use crate::tree_ops::append_child;

    fn el(doc: &mut Document, tag: &str) -> NodeId {
        doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup(tag), Namespace::Html)))
    }

    #[test]
    fn invalidate_marks_ancestor_chain() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        d.append_to_root(a).unwrap();
        append_child(&mut d, a, b).unwrap();
        append_child(&mut d, b, c).unwrap();
        d.clear_dirty();

        invalidate(&mut d, c);
        assert!(d.node(c).dirty);
        assert!(d.node(b).dirty);
        assert!(d.node(a).dirty);
        let root = d.root();
        assert!(d.node(root).dirty, "root is an ancestor");
    }

    #[test]
    fn invalidate_self_only() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        d.append_to_root(a).unwrap();
        append_child(&mut d, a, b).unwrap();
        d.clear_dirty();
        invalidate_self(&mut d, b);
        assert!(d.node(b).dirty);
        assert!(!d.node(a).dirty);
    }

    #[test]
    fn clear_dirty_resets_everything() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        d.append_to_root(a).unwrap();
        assert!(any_dirty(&d));
        d.clear_dirty();
        assert!(!any_dirty(&d));
    }

    #[test]
    fn structural_insert_marks_parent_dirty() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        d.append_to_root(a).unwrap();
        d.clear_dirty();
        let b = el(&mut d, "b");
        append_child(&mut d, a, b).unwrap();
        assert!(d.node(a).dirty, "insertion dirties the parent");
        let got: Vec<_> = dirty_nodes(&d).collect();
        assert!(got.contains(&a) && got.contains(&b));
    }
}