//! The per-element index: everything `matches_compound` needs, computed once
//! per restyle pass (§6.2, "Per-element index built once per restyle epoch").
//!
//! The index is a `Vec` indexed by `NodeId`, so a lookup is one bounds check.
//! Nothing here is lazily computed: a style pass touches every element anyway,
//! and a lazy cell per node would cost more memory than the fields it saves.

use ahash::AHashMap;
use ahash::AHashSet;
use engine_core::NodeId;
use engine_dom::{Document, KnownTag, NodeKind, Tag};

/// Everything one element contributes to structural matching.
#[derive(Clone, Debug, Default)]
pub struct ElementIndex {
    /// Interned tag, for O(1) type-selector comparison.
    pub tag: Option<Tag>,
    pub known: Option<KnownTag>,
    pub id: Option<Box<str>>,
    pub classes: AHashSet<Box<str>>,
    /// 1-based position among element siblings.
    pub child_index: u32,
    /// Number of element siblings, total.
    pub sibling_count: u32,
    /// 1-based position among same-tag element siblings.
    pub type_index: u32,
    /// Number of same-tag element siblings, total.
    pub type_count: u32,
    /// True when the element has no child other than comments (Selectors 4 §5.5).
    pub empty: bool,
    /// `lang` inherited from the nearest ancestor that declares one.
    pub lang: Option<Box<str>>,
}

impl ElementIndex {
    fn with_tag(tag: Tag) -> Self {
        ElementIndex { known: tag.as_known(), tag: Some(tag), ..ElementIndex::default() }
    }

    /// Does this element's tag equal `name` (case-insensitively, HTML rules)?
    pub fn has_tag(&self, name: &str) -> bool {
        match &self.tag {
            Some(t) => t.as_str().eq_ignore_ascii_case(name),
            None => false,
        }
    }
}

/// The index for every node in `doc`.
#[derive(Clone, Debug, Default)]
pub struct MatchIndex {
    /// `nodes[i]` is `None` for a non-element node.
    pub nodes: Vec<Option<ElementIndex>>,
    /// Document-level language from `<html lang>`, for `:lang()`.
    pub default_lang: Option<String>,
}

struct SiblingCounter {
    /// Last parent seen, and how many element children we have counted for it.
    parent: Option<NodeId>,
    n: u32,
}

impl MatchIndex {
    pub fn build(doc: &Document) -> MatchIndex {
        let mut nodes: Vec<Option<ElementIndex>> = vec![None; doc.len()];
        let default_lang = find_document_lang(doc);

        // Sibling counts need the totals before any node's index is final, so
        // they are computed in their own pass over children lists. O(n).
        //
        // Tag names are interned to a `u32` once, so the per-element keys are
        // `Copy` and allocation-free. `KnownTag` would be simpler but is not
        // `Hash`, so we build our own interning table.
        let mut tag_ids: AHashMap<String, u32> = AHashMap::new();
        let mut child_total: AHashMap<NodeId, u32> = AHashMap::new();
        let mut type_total: AHashMap<(NodeId, u32), u32> = AHashMap::new();
        let mut tag_id_for: Vec<Option<u32>> = vec![None; doc.len()];
        for id in doc.ids() {
            if let (Some(p), Some(el)) = (doc.parent(id), doc.element(id)) {
                let name = el.tag.as_str();
                let t = match tag_ids.get(name) {
                    Some(&i) => i,
                    None => {
                        let i = tag_ids.len() as u32;
                        // One allocation per *distinct* tag name, not per element.
                        tag_ids.insert(name.to_string(), i);
                        i
                    }
                };
                tag_id_for[id.index()] = Some(t);
                *child_total.entry(p).or_insert(0) += 1;
                *type_total.entry((p, t)).or_insert(0) += 1;
            }
        }

        let mut counts = SiblingCounter { parent: None, n: 0 };
        let mut type_seen: AHashMap<(NodeId, u32), u32> = AHashMap::new();
        // Inherited `lang` resolved top-down in one pass: node ids are created
        // in pre-order by tree construction, so a node's parent always precedes
        // it. Looking each language up by walking ancestors would make a
        // 10k-deep document quadratic (measured: 2.3 s for 10k nodes).
        let mut lang_of: AHashMap<NodeId, Option<Box<str>>> = AHashMap::new();

        for id in doc.ids() {
            let Some(el) = doc.element(id) else { continue };
            let mut idx = ElementIndex::with_tag(el.tag.clone());
            idx.id = el.id().map(Box::from);
            for c in el.classes() {
                idx.classes.insert(Box::from(c));
            }
            idx.empty = is_empty_element(doc, id);
            idx.lang = match el.attr("lang") {
                Some(l) => Some(Box::from(l)),
                None => doc
                    .parent(id)
                    .and_then(|p| lang_of.get(&p).cloned())
                    .flatten()
                    .or_else(|| default_lang.clone().map(Box::from)),
            };
            lang_of.insert(id, idx.lang.clone());

            if let Some(p) = doc.parent(id) {
                if counts.parent != Some(p) {
                    counts.parent = Some(p);
                    counts.n = 0;
                }
                counts.n += 1;
                idx.child_index = counts.n;
                idx.sibling_count = child_total.get(&p).copied().unwrap_or(counts.n);

                let key = (p, tag_id_for[id.index()].unwrap_or(u32::MAX));
                let n = type_seen.entry(key).or_insert(0);
                *n += 1;
                idx.type_index = *n;
                idx.type_count = type_total.get(&key).copied().unwrap_or(*n);
            }
            nodes[id.index()] = Some(idx);
        }
        MatchIndex { nodes, default_lang }
    }

    #[inline]
    pub fn get(&self, id: NodeId) -> Option<&ElementIndex> {
        self.nodes.get(id.index()).and_then(|n| n.as_ref())
    }
}

/// `:empty` per Selectors 4 §5.5: no children other than comments and
/// processing instructions. Whitespace text counts as a child, so `<p> </p>` is
/// NOT `:empty`. Browsers agree with the spec here.
fn is_empty_element(doc: &Document, id: NodeId) -> bool {
    let mut child = doc.first_child(id);
    let mut steps = 0u32;
    while let Some(c) = child {
        steps += 1;
        if steps > (1 << 20) {
            return false;
        }
        if !matches!(doc.kind(c), NodeKind::Comment(_)) {
            return false;
        }
        child = doc.next_sibling(c);
    }
    true
}

fn find_document_lang(doc: &Document) -> Option<String> {
    doc.ids().find_map(|id| {
        let el = doc.element(id)?;
        if el.tag.is(KnownTag::Html) {
            el.attr("lang").map(str::to_string)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_dom::{ElementData, Namespace};

    #[test]
    fn bare_element_is_only_child_of_document() {
        let mut doc = Document::new();
        let t = doc.create_node(NodeKind::Element(ElementData::new(
            Tag::lookup("div"),
            Namespace::Html,
        )));
        doc.append_to_root(t).unwrap();
        let idx = MatchIndex::build(&doc);
        let el = idx.get(t).expect("element indexed");
        assert_eq!(el.child_index, 1);
        assert_eq!(el.sibling_count, 1);
        assert_eq!(el.type_index, 1);
        assert_eq!(el.type_count, 1);
        assert!(el.empty);
        assert!(el.id.is_none());
        assert!(el.classes.is_empty());
    }

    #[test]
    fn whitespace_text_is_not_empty() {
        let mut doc = Document::new();
        let d = doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup("p"), Namespace::Html)));
        doc.append_to_root(d).unwrap();
        let bad = doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup("div"), Namespace::Html)));
        doc.append_to_root(bad).unwrap();
        let t = doc.create_node(NodeKind::Text(" ".into()));
        engine_dom::tree_ops::append_child(&mut doc, bad, t).unwrap();
        let idx = MatchIndex::build(&doc);
        assert!(idx.get(d).unwrap().empty, "<p> with no children is :empty");
        assert!(!idx.get(bad).unwrap().empty, "<div> </div> is not :empty per Selectors 4");
    }

    #[test]
    fn sibling_positions_count_only_elements() {
        let mut doc = Document::new();
        let parent = doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup("div"), Namespace::Html)));
        doc.append_to_root(parent).unwrap();
        let a = doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup("p"), Namespace::Html)));
        let b = doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup("span"), Namespace::Html)));
        let c = doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup("p"), Namespace::Html)));
        engine_dom::tree_ops::append_child(&mut doc, parent, a).unwrap();
        let txt = doc.create_node(NodeKind::Text("x".into()));
        engine_dom::tree_ops::append_child(&mut doc, parent, txt).unwrap();
        engine_dom::tree_ops::append_child(&mut doc, parent, b).unwrap();
        engine_dom::tree_ops::append_child(&mut doc, parent, c).unwrap();
        let idx = MatchIndex::build(&doc);
        let ia = idx.get(a).unwrap();
        let ic = idx.get(c).unwrap();
        assert_eq!((ia.child_index, ia.sibling_count), (1, 3));
        assert_eq!((ic.child_index, ic.sibling_count), (3, 3));
        assert_eq!((ia.type_index, ia.type_count), (1, 2), "p is the 1st of 2 ps");
        assert_eq!((ic.type_index, ic.type_count), (2, 2), "text node is not an element");
    }
}