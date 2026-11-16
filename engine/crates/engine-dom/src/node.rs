//! DOM node kinds.

use engine_core::NodeId;

use crate::element::{Namespace, Tag};

/// One attribute. Names are lowercased; values are kept verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

/// Data attached to an `Element` node.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementData {
    pub tag: Tag,
    pub namespace: Namespace,
    pub attrs: Vec<Attr>,
}

impl ElementData {
    pub fn new(tag: Tag, namespace: Namespace) -> Self {
        ElementData { tag, namespace, attrs: Vec::new() }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .map(|a| a.value.as_str())
    }

    pub fn set_attr(&mut self, name: &str, value: String) {
        if let Some(a) = self.attrs.iter_mut().find(|a| a.name.eq_ignore_ascii_case(name)) {
            a.value = value;
        } else {
            self.attrs.push(Attr { name: name.to_ascii_lowercase(), value });
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|a| !a.name.eq_ignore_ascii_case(name));
    }

    pub fn has_attr(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }

    /// The `id` attribute, if present.
    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    /// Classes split on ASCII whitespace.
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attr("class")
            .into_iter()
            .flat_map(|c| c.split_ascii_whitespace())
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes().any(|c| c == class)
    }
}

/// Node kinds. A node's kind determines what the tree operations allow
/// (text nodes cannot have children).
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Document,
    Doctype { name: String },
    Element(ElementData),
    Text(String),
    Comment(String),
    /// Contents of `<template>`; not rendered (§15).
    TemplateContents,
}

impl NodeKind {
    pub fn as_element(&self) -> Option<&ElementData> {
        match self {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_tag(&self) -> Option<&Tag> {
        self.as_element().map(|e| &e.tag)
    }

    pub fn is_text(&self) -> bool {
        matches!(self, NodeKind::Text(_))
    }
}

/// A single node in the arena.
#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    /// Generation counter bumped on any structural or attribute mutation, used
    /// to invalidate style caches (§3.1 `mutation.rs`).
    pub dirty: bool,
}

impl Node {
    pub fn new(kind: NodeKind) -> Self {
        Node {
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            dirty: true,
        }
    }

    /// A child iterator over this node's children.
    pub fn children_of(&self) -> Option<NodeId> {
        self.first_child
    }
}

/// Quirks mode from the doctype (§4 Stage 2 `ParseOutcome::quirks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QuirksMode {
    #[default]
    NoQuirks,
    Quirks,
    LimitedQuirks,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_lookup_is_case_insensitive() {
        let mut e = ElementData::new(Tag::lookup("div"), Namespace::Html);
        e.set_attr("Class", "a b".into());
        assert_eq!(e.attr("class"), Some("a b"));
        assert_eq!(e.attr("CLASS"), Some("a b"));
        assert_eq!(e.attr("id"), None);
    }

    #[test]
    fn set_attr_replaces_in_place() {
        let mut e = ElementData::new(Tag::lookup("div"), Namespace::Html);
        e.set_attr("id", "one".into());
        e.set_attr("ID", "two".into());
        assert_eq!(e.attrs.len(), 1);
        assert_eq!(e.attr("id"), Some("two"));
    }

    #[test]
    fn classes_split_on_whitespace() {
        let mut e = ElementData::new(Tag::lookup("p"), Namespace::Html);
        e.set_attr("class", "  alpha\tbeta\n gamma ".into());
        let got: Vec<_> = e.classes().collect();
        assert_eq!(got, vec!["alpha", "beta", "gamma"]);
        assert!(e.has_class("beta"));
        assert!(!e.has_class("bet"));
    }

    #[test]
    fn remove_attr() {
        let mut e = ElementData::new(Tag::lookup("a"), Namespace::Html);
        e.set_attr("href", "x".into());
        e.remove_attr("HREF");
        assert!(!e.has_attr("href"));
    }
}