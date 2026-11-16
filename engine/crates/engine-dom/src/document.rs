//! The DOM node arena.
//!
//! **Deviation from ARCHITECTURE.md §3.1 (recorded):** the document describes
//! the arena as `Cell<Vec<Node>>` with `RefCell`/unsafe-free interior mutability
//! so `parse(&Document, ...)` can mutate through a shared reference. Interior
//! mutability here buys nothing except the ability to hold two `&mut` borrows
//! at once, which is exactly the bug class we want the borrow checker to catch.
//! This takes `&mut Document` everywhere instead: `engine-html::parse` returns
//! the document, and mutation during tree construction is a plain `&mut` flow.

use engine_core::NodeId;

use crate::node::{ElementData, Node, NodeKind};
use crate::tree_ops::DomError;

/// A DOM document: a node arena plus the root id.
#[derive(Clone, Debug)]
pub struct Document {
    nodes: Vec<Node>,
    root: NodeId,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document with a `Document` root at id 0.
    pub fn new() -> Self {
        let nodes = vec![Node::new(NodeKind::Document)];
        Document { nodes, root: NodeId(0) }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Append `node` as the last child of the document root.
    ///
    /// Exists so callers do not hold `root` (an immutable borrow of `self`)
    /// while taking `&mut self`: the borrow checker rejects
    /// `append_child(&mut doc, doc.root(), n)`.
    pub fn append_to_root(&mut self, node: NodeId) -> Result<(), DomError> {
        let root = self.root;
        crate::tree_ops::append_child(self, root, node)
    }

    /// Insert `node` before `reference`, which must be a child of the root.
    /// Same borrow rationale as `append_to_root`.
    pub fn insert_before_in_root(&mut self, node: NodeId, reference: NodeId) -> Result<(), DomError> {
        let root = self.root;
        crate::tree_ops::insert_before(self, root, node, reference)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    pub fn try_node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.nodes[id.index()].kind
    }

    pub fn element(&self, id: NodeId) -> Option<&ElementData> {
        self.node(id).kind.as_element()
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut ElementData> {
        self.node_mut(id).kind.as_element_mut()
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).first_child
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).next_sibling
    }

    /// Every id in the arena, in creation order.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len() as u32).map(NodeId)
    }

    /// Pre-order descendants of `id`, excluding `id` itself.
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants { doc: self, next: self.node(id).first_child, root: id }
    }

    /// Pre-order walk of the whole document, including the root.
    pub fn walk(&self) -> Descendants<'_> {
        Descendants { doc: self, next: Some(self.root), root: self.root }
    }

    /// Allocate a detached node and return its id.
    pub fn create_node(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node::new(kind));
        id
    }

    /// `<template>` contents: an interposed, non-rendered fragment.
    pub fn create_template_contents(&mut self) -> NodeId {
        self.create_node(NodeKind::TemplateContents)
    }

    // --- The mutation surface. `tree_ops` is the only module that touches
    // --- the link fields directly; everything else goes through these.

    pub(crate) fn slot(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    pub(crate) fn slot_ref(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Mark every ancestor of `id` dirty (style/relayout invalidation).
    pub fn mark_ancestors_dirty(&mut self, id: NodeId) {
        let mut cur = Some(id);
        while let Some(n) = cur {
            let node = &mut self.nodes[n.index()];
            if node.dirty {
                break;
            }
            node.dirty = true;
            cur = node.parent;
        }
    }

    /// Clear every dirty flag (after a successful restyle).
    pub fn clear_dirty(&mut self) {
        for n in &mut self.nodes {
            n.dirty = false;
        }
    }
}

/// Pre-order descendant iterator. Depth-first using the sibling links plus the
/// parent pointer: O(1) per step, no explicit stack, no allocation.
pub struct Descendants<'a> {
    doc: &'a Document,
    next: Option<NodeId>,
    /// The node whose subtree is being walked. `descendants(root)` starts from
    /// the root's first child; `walk()` starts from the root itself.
    root: NodeId,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let cur = self.next?;
        let node = self.doc.node(cur);
        self.next = if let Some(c) = node.first_child {
            Some(c)
        } else {
            // No children: move to the next sibling, climbing ancestors until
            // one has a sibling, never rising above the walk root.
            let mut n = cur;
            loop {
                if n == self.root {
                    break None;
                }
                let nn = self.doc.node(n);
                if let Some(s) = nn.next_sibling {
                    break Some(s);
                }
                match nn.parent {
                    Some(p) => n = p,
                    None => break None,
                }
            }
        };
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Namespace, Tag};

    fn el(doc: &mut Document, tag: &str) -> NodeId {
        doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup(tag), Namespace::Html)))
    }

    #[test]
    fn new_document_has_only_root() {
        let d = Document::new();
        assert_eq!(d.len(), 1);
        assert!(matches!(d.kind(d.root()), NodeKind::Document));
        assert_eq!(d.first_child(d.root()), None);
    }

    #[test]
    fn descendants_walks_preorder() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        d.append_to_root(a).unwrap();
        crate::tree_ops::append_child(&mut d, a, b).unwrap();
        crate::tree_ops::append_child(&mut d, a, c).unwrap();
        let got: Vec<_> = d.descendants(d.root()).collect();
        assert_eq!(got, vec![a, b, c], "pre-order: parent then children in order");
    }

    #[test]
    fn descendants_includes_root_when_asked() {
        let d = Document::new();
        let got: Vec<_> = d.walk().collect();
        assert_eq!(got, vec![d.root()]);
    }

    #[test]
    fn descendants_does_not_escape_subtree() {
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        d.append_to_root(a).unwrap();
        d.append_to_root(c).unwrap();
        crate::tree_ops::append_child(&mut d, a, b).unwrap();
        let got: Vec<_> = d.descendants(a).collect();
        assert_eq!(got, vec![b], "sibling c is outside a's subtree");
    }

    #[test]
    fn deep_descendants_order() {
        // root > a > b > c, plus root > d
        let mut d = Document::new();
        let a = el(&mut d, "a");
        let b = el(&mut d, "b");
        let c = el(&mut d, "c");
        let e = el(&mut d, "d");
        d.append_to_root(a).unwrap();
        crate::tree_ops::append_child(&mut d, a, b).unwrap();
        crate::tree_ops::append_child(&mut d, b, c).unwrap();
        d.append_to_root(e).unwrap();
        let got: Vec<_> = d.descendants(d.root()).collect();
        assert_eq!(got, vec![a, b, c, e]);
    }
}