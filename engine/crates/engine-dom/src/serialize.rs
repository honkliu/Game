//! Canonical serialization of a document subtree.
//!
//! This is the oracle format for the HTML tree-construction fixtures. It is
//! deliberately not `innerHTML`: it is stable, explicit, and one line per node
//! so a failing diff points at exactly one element.
//!
//! ```text
//! #document
//!   <!DOCTYPE html>
//!   <html>
//!     <head>
//!     <body>
//!       #text "Hello"
//!       <p id="x" class="a b">
//!         #text "world"
//! ```
//!
//! Two-space indent per depth. Text is escaped and always quoted. Attributes
//! are emitted in document order, which tree construction specifies, so the
//! fixtures catch attribute reordering.
//!
//! **Every walk in this module is iterative, not recursive.** A DOM's nesting
//! depth is attacker-controlled (`<div>` repeated 10,000 times is a valid
//! document), and these walkers run on ordinary element subtrees, not just in
//! tests. A recursive walker overflows the stack at roughly 1,700 levels on a
//! 1 MB debug-build thread — and a stack overflow on Windows is
//! `STATUS_STACK_BUFFER_OVERRUN`, an uncatchable process abort. Each walker
//! therefore carries its own explicit stack and has no depth limit at all;
//! depth costs heap, never stack. `deep_nesting_does_not_overflow_*` in the
//! tests below is the regression guard.

use std::fmt::Write as _;

use engine_core::NodeId;

use crate::document::Document;
use crate::node::NodeKind;

/// Serialize the whole document.
pub fn serialize(doc: &Document) -> String {
    let mut out = String::new();
    write_node(doc, doc.root(), 0, &mut out);
    out
}

/// Serialize one node's subtree.
pub fn serialize_subtree(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    write_node(doc, node, 0, &mut out);
    out
}

/// One pending action in an explicit pre-order walk.
/// `NodeId` is `Copy` and 4 bytes, so the stack stores it, not an index.
enum Step {
    /// Emit this node, then descend into its children.
    Enter(NodeId, usize),
    /// Emit the element's end tag at this depth.
    Close(NodeId, usize),
}

/// Indentation for `depth` levels.
///
/// Deliberately a `write!` of spaces rather than `{:width$}`: `format!`'s width
/// argument panics with "Formatting argument out of range" above 65_535, and
/// the width here is `depth * 2`. A 32_768-deep tree — a 320 KB page of
/// `<div>` — therefore used to panic the process inside `serialize`. A string
/// of spaces has no such limit; the cost is proportional to the output that is
/// already being written anyway.
fn indent(out: &mut String, depth: usize) {
    let n = depth.saturating_mul(2);
    out.reserve(n);
    for _ in 0..n {
        out.push(' ');
    }
}

fn write_line(out: &mut String, depth: usize, body: &str) {
    indent(out, depth);
    out.push_str(body);
    out.push('\n');
}

fn children(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut cur = doc.first_child(id);
    while let Some(c) = cur {
        out.push(c);
        cur = doc.next_sibling(c);
    }
    out
}

fn write_node(doc: &Document, id: NodeId, depth: usize, out: &mut String) {
    let mut stack = vec![Step::Enter(id, depth)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Close(n, d) => {
                let name = doc.element(n).map(|e| e.tag.as_str()).unwrap_or("");
                write_line(out, d, &format!("</{name}>"));
            }
            Step::Enter(id, d) => {
                let node = doc.node(id);
                match &node.kind {
                    NodeKind::Document => {
                        write_line(out, d, "#document");
                        push_children(doc, id, d + 1, &mut stack);
                    }
                    NodeKind::Doctype { name } => {
                        write_line(out, d, &format!("<!DOCTYPE {name}>"));
                    }
                    NodeKind::Element(e) => {
                        let mut line = format!("<{}", e.tag.as_str());
                        for a in &e.attrs {
                            let _ = write!(line, " {}=\"{}\"", a.name, escape_text(&a.value));
                        }
                        line.push('>');
                        write_line(out, d, &line);
                        if !crate::element::is_void(&e.tag) {
                            // Push the close first so it runs after the children.
                            stack.push(Step::Close(id, d));
                            push_children(doc, id, d + 1, &mut stack);
                        }
                    }
                    NodeKind::Text(t) => {
                        write_line(out, d, &format!("#text \"{}\"", escape_text(t)));
                    }
                    NodeKind::Comment(c) => {
                        write_line(out, d, &format!("<!-- {} -->", escape_text(c)));
                    }
                    NodeKind::TemplateContents => {
                        write_line(out, d, "#template-contents");
                        push_children(doc, id, d + 1, &mut stack);
                    }
                }
            }
        }
    }
}

/// Push children in reverse so the first child is consumed first.
fn push_children(doc: &Document, id: NodeId, depth: usize, stack: &mut Vec<Step>) {
    let kids = children(doc, id);
    for c in kids.into_iter().rev() {
        stack.push(Step::Enter(c, depth));
    }
}

/// Escape a string for the canonical fixture format.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// `innerHTML` for the DOM API surface: children of `node`, unindented.
pub fn inner_html(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    let mut stack: Vec<Step> = Vec::new();
    push_children(doc, node, 0, &mut stack);
    write_html(doc, stack, &mut out);
    out
}

/// `outerHTML` for a single node.
pub fn outer_html(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    let stack = vec![Step::Enter(node, 0)];
    write_html(doc, stack, &mut out);
    out
}

/// Iterative HTML serialization over an explicit stack. Depth is irrelevant
/// here (it is unused), so `Close` only needs the element's name.
fn write_html(doc: &Document, mut stack: Vec<Step>, out: &mut String) {
    while let Some(step) = stack.pop() {
        match step {
            Step::Close(n, _) => {
                let name = doc.element(n).map(|e| e.tag.as_str()).unwrap_or("");
                let _ = write!(out, "</{}>", name);
            }
            Step::Enter(id, d) => {
                match &doc.node(id).kind {
                    NodeKind::Document | NodeKind::Doctype { .. } => {}
                    NodeKind::Element(e) => {
                        let _ = write!(out, "<{}", e.tag.as_str());
                        for a in &e.attrs {
                            let _ = write!(out, " {}=\"{}\"", a.name, escape_html_attr(&a.value));
                        }
                        out.push('>');
                        if !crate::element::is_void(&e.tag) {
                            stack.push(Step::Close(id, d));
                            push_children(doc, id, d, &mut stack);
                        }
                    }
                    NodeKind::Text(t) => out.push_str(&escape_html_text(t)),
                    NodeKind::Comment(c) => {
                        let _ = write!(out, "<!--{c}-->");
                    }
                    NodeKind::TemplateContents => push_children(doc, id, d, &mut stack),
                }
            }
        }
    }
}

fn escape_html_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_html_attr(s: &str) -> String {
    escape_html_text(s).replace('"', "&quot;")
}

/// `textContent`: concatenation of all descendant text.
pub fn text_content(doc: &Document, node: NodeId) -> String {
    let mut out = String::new();
    // No end tags and no depth-sensitivity: a plain work list of nodes.
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        match &doc.node(id).kind {
            NodeKind::Text(t) => out.push_str(t),
            NodeKind::Element(_) | NodeKind::Document | NodeKind::TemplateContents => {
                for c in children(doc, id).into_iter().rev() {
                    stack.push(c);
                }
            }
            NodeKind::Comment(_) | NodeKind::Doctype { .. } => {}
        }
    }
    out
}

/// Set `textContent`: remove all children, then append one text node.
pub fn set_text_content(doc: &mut Document, node: NodeId, text: &str) {
    if !matches!(
        doc.kind(node),
        NodeKind::Element(_) | NodeKind::Document | NodeKind::TemplateContents
    ) {
        match &mut doc.node_mut(node).kind {
            NodeKind::Text(t) | NodeKind::Comment(t) => *t = text.to_string(),
            _ => {}
        }
        return;
    }
    let kids = children(doc, node);
    for k in kids {
        crate::tree_ops::remove(doc, k);
    }
    if !text.is_empty() {
        let t = doc.create_node(NodeKind::Text(text.to_string()));
        let _ = crate::tree_ops::append_child(doc, node, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Namespace, Tag};
    use crate::node::ElementData;
    use crate::tree_ops::append_child;

    fn el(doc: &mut Document, tag: &str) -> NodeId {
        doc.create_node(NodeKind::Element(ElementData::new(Tag::lookup(tag), Namespace::Html)))
    }

    #[test]
    fn serializes_nested_elements_with_indent() {
        let mut d = Document::new();
        let html = el(&mut d, "html");
        let body = el(&mut d, "body");
        d.append_to_root(html).unwrap();
        append_child(&mut d, html, body).unwrap();
        let expected = "#document\n  <html>\n    <body>\n    </body>\n  </html>\n";
        assert_eq!(serialize(&d), expected);
    }

    #[test]
    fn serializes_attributes_in_document_order() {
        let mut d = Document::new();
        let mut e = ElementData::new(Tag::lookup("p"), Namespace::Html);
        e.set_attr("id", "x".into());
        e.set_attr("class", "a b".into());
        let p = d.create_node(NodeKind::Element(e));
        d.append_to_root(p).unwrap();
        assert_eq!(serialize(&d), "#document\n  <p id=\"x\" class=\"a b\">\n  </p>\n");
    }

    #[test]
    fn void_elements_have_no_close() {
        let mut d = Document::new();
        let br = el(&mut d, "br");
        d.append_to_root(br).unwrap();
        assert_eq!(serialize(&d), "#document\n  <br>\n");
    }

    #[test]
    fn text_escapes_specials() {
        let mut d = Document::new();
        let t = d.create_node(NodeKind::Text("a\tb\nc\"d\\e".into()));
        d.append_to_root(t).unwrap();
        assert_eq!(serialize(&d), "#document\n  #text \"a\\tb\\nc\\\"d\\\\e\"\n");
    }

    #[test]
    fn doctype() {
        let mut d = Document::new();
        let dt = d.create_node(NodeKind::Doctype { name: "html".into() });
        d.append_to_root(dt).unwrap();
        assert_eq!(serialize(&d), "#document\n  <!DOCTYPE html>\n");
    }

    #[test]
    fn text_content_concatenates() {
        let mut d = Document::new();
        let p = el(&mut d, "p");
        d.append_to_root(p).unwrap();
        let t1 = d.create_node(NodeKind::Text("Hello ".into()));
        let b = el(&mut d, "b");
        let t2 = d.create_node(NodeKind::Text("world".into()));
        append_child(&mut d, p, t1).unwrap();
        append_child(&mut d, p, b).unwrap();
        append_child(&mut d, b, t2).unwrap();
        assert_eq!(text_content(&d, p), "Hello world");
    }

    #[test]
    fn set_text_content_replaces_children() {
        let mut d = Document::new();
        let p = el(&mut d, "p");
        d.append_to_root(p).unwrap();
        let t1 = d.create_node(NodeKind::Text("old".into()));
        append_child(&mut d, p, t1).unwrap();
        set_text_content(&mut d, p, "new");
        assert_eq!(text_content(&d, p), "new");
        assert_eq!(crate::tree_ops::child_count(&d, p), 1);
    }

    #[test]
    fn inner_html_escapes() {
        let mut d = Document::new();
        let p = el(&mut d, "p");
        d.append_to_root(p).unwrap();
        let t = d.create_node(NodeKind::Text("a < b & c".into()));
        append_child(&mut d, p, t).unwrap();
        assert_eq!(inner_html(&d, p), "a &lt; b &amp; c");
    }

    /// Build `depth` nested `<div>`s under the root and return the root.
    fn deep(d: &mut Document, depth: usize) -> NodeId {
        let root = d.root();
        let mut parent = root;
        for _ in 0..depth {
            let n = el(d, "div");
            append_child(d, parent, n).unwrap();
            parent = n;
        }
        root
    }

    // Far past anything a recursive walker survives (it dies at ~1_700 in a debug
    // build on a 1 MB thread). Not higher: this walker allocates one `Vec` per
    // node via `children()`, so the test is O(depth) small allocations in a
    // debug build — 20_000 keeps the suite fast while proving the property.
    const DEEP: usize = 20_000;

    #[test]
    fn deep_nesting_does_not_overflow_serialize() {
        let mut d = Document::new();
        deep(&mut d, DEEP);
        let s = serialize(&d);
        assert_eq!(s.lines().count(), DEEP * 2 + 1, "#document + open + close per level");
        assert!(s.ends_with("</div>\n"));
    }

    #[test]
    fn deep_nesting_does_not_overflow_inner_html() {
        let mut d = Document::new();
        let root = deep(&mut d, DEEP);
        let h = inner_html(&d, root);
        assert_eq!(h.len(), DEEP * "<div></div>".len());
        assert!(h.starts_with("<div><div>"));
    }

    #[test]
    fn deep_nesting_does_not_overflow_outer_html() {
        let mut d = Document::new();
        let root = deep(&mut d, DEEP);
        // `outer_html` of the Document node emits nothing (it is not an
        // element); the outermost real element is the first child.
        let outer = d.first_child(root).unwrap();
        let h = outer_html(&d, outer);
        assert!(h.starts_with("<div>"));
        assert!(h.ends_with("</div>"));
        assert_eq!(h.len(), DEEP * "<div></div>".len());
    }

    #[test]
    fn deep_nesting_does_not_overflow_text_content() {
        let mut d = Document::new();
        let root = deep(&mut d, DEEP);
        assert_eq!(text_content(&d, root), "", "no text nodes at all");
        // And with text at the bottom, the concatenation still reaches it.
        let leaf = d.node(root);
        let mut n = leaf.first_child.unwrap();
        while let Some(c) = d.first_child(n) {
            n = c;
        }
        let t = d.create_node(NodeKind::Text("bottom".into()));
        append_child(&mut d, n, t).unwrap();
        assert_eq!(text_content(&d, root), "bottom");
    }

    /// Regression: `serialize` used `{:width$}` for indentation, and the width
    /// argument panics ("Formatting argument out of range") above 65_535. At
    /// `depth * 2` that is any tree 32_768 levels deep — a 320 KB page of
    /// `<div>`, well inside what a hostile page can send.
    #[test]
    fn indentation_beyond_u16_width_does_not_panic() {
        let mut d = Document::new();
        deep(&mut d, 33_000);
        let s = serialize(&d);
        // Deepest indent is 33_000 levels * 2 spaces on the innermost open tag.
        assert!(s.lines().count() > 66_000);
        assert!(s.contains("</div>\n"));
    }
}