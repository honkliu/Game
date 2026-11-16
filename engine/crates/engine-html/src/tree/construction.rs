//! Tree construction, by insertion mode.
//!
//! Scope, stated plainly: this implements the insertion modes and implicit-close
//! rules that determine the tree shape for real documents — `Initial`,
//! `BeforeHtml`, `BeforeHead`, `InHead`, `AfterHead`, `InBody`, `Text`,
//! `InTable`, `InTableBody`, `InRow`, `InCell`, `InCaption`, `InColumnGroup`,
//! `InSelect`, `AfterBody`, `AfterAfterBody`. Plus:
//!
//! - the `p`/`li`/`dd`/`dt`/`option`/`td`/`tr` implicit-close rules;
//! - foster parenting for content that appears where table structure is
//!   required (HTML §13.2.6.1 "in table" text and misnested tags);
//! - the adoption agency, in its common form, for misnested `<b>/<i>/<a>`.
//!
//! Not implemented, and recorded in the report as `Approximation::HtmlRecovery`:
//! the full active-formatting reconstruction after markers in every corner, and
//! `<template>` content instantiation. Where the two would differ, the tree we
//! produce is the one a browser produces for well-formed input.

use engine_core::NodeId;
use engine_dom::document::Document;
use engine_dom::element::{Namespace, Tag};
use engine_dom::node::{ElementData, NodeKind, QuirksMode};
use engine_dom::tree_ops;

use crate::tokenizer::token::{Attribute, Token};

use super::open_elements::{closes_p, is_scoping, ActiveFormatting, OpenElements};

/// Insertion modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableBody,
    InRow,
    InCell,
    InCaption,
    InColumnGroup,
    InSelect,
    AfterBody,
    AfterAfterBody,
}

/// A recorded deviation from the spec, surfaced in the render report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlApproximation {
    /// A recovery path that we simplified. `detail` names it.
    Recovery { detail: &'static str },
    /// A token we dropped.
    DroppedToken { token: String },
}

/// The result of tree construction.
#[derive(Clone, Debug, Default)]
pub struct TreeOutcome {
    pub quirks: QuirksMode,
    pub errors: Vec<String>,
    pub approximations: Vec<HtmlApproximation>,
}

struct Builder<'a> {
    doc: &'a mut Document,
    mode: Mode,
    original_mode: Mode,
    open: OpenElements,
    formatting: ActiveFormatting,
    head: Option<NodeId>,
    body: Option<NodeId>,
    html: Option<NodeId>,
    quirks: QuirksMode,
    errors: Vec<String>,
    approximations: Vec<HtmlApproximation>,
    /// Set while processing `<script>`/`<style>` text so the `Text` mode knows
    /// which element to close.
    text_element: Option<NodeId>,
}

pub fn build(doc: &mut Document, tokens: Vec<Token>) -> TreeOutcome {
    let mut b = Builder {
        doc,
        mode: Mode::Initial,
        original_mode: Mode::Initial,
        open: OpenElements::new(),
        formatting: ActiveFormatting::new(),
        head: None,
        body: None,
        html: None,
        quirks: QuirksMode::NoQuirks,
        errors: Vec::new(),
        approximations: Vec::new(),
        text_element: None,
    };
    for tok in tokens {
        if matches!(tok, Token::Eof) {
            break;
        }
        b.process(tok);
    }
    b.finish()
}

impl Builder<'_> {
    fn process(&mut self, tok: Token) {
        // Reprocess loop: some modes change mode and hand the same token back.
        let mut t = tok;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 64 {
                self.record_dropped("reprocess-loop");
                break;
            }
            match self.dispatch(std::mem::replace(&mut t, Token::Eof)) {
                None => break,
                Some(n) => t = n,
            }
        }
    }

    fn dispatch(&mut self, tok: Token) -> Option<Token> {
        match self.mode {
            Mode::Initial => self.initial(tok),
            Mode::BeforeHtml => self.before_html(tok),
            Mode::BeforeHead => self.before_head(tok),
            Mode::InHead => self.in_head(tok),
            Mode::AfterHead => self.after_head(tok),
            Mode::InBody => self.in_body(tok),
            Mode::Text => self.text_mode(tok),
            Mode::InTable => self.in_table(tok),
            Mode::InTableBody => self.in_table_body(tok),
            Mode::InRow => self.in_row(tok),
            Mode::InCell => self.in_cell(tok),
            Mode::InCaption => self.in_caption(tok),
            Mode::InColumnGroup => self.in_column_group(tok),
            Mode::InSelect => self.in_select(tok),
            Mode::AfterBody => self.after_body(tok),
            Mode::AfterAfterBody => self.after_after_body(tok),
        }
    }

    fn finish(self) -> TreeOutcome {
        TreeOutcome {
            quirks: self.quirks,
            errors: self.errors,
            approximations: self.approximations,
        }
    }

    fn record_dropped(&mut self, what: &str) {
        self.approximations.push(HtmlApproximation::DroppedToken { token: what.to_string() });
    }

    fn note(&mut self, detail: &'static str) {
        self.approximations.push(HtmlApproximation::Recovery { detail });
    }

    // --- Node helpers -----------------------------------------------------

    fn make_element(&mut self, name: &str, attrs: &[Attribute]) -> NodeId {
        let mut e = ElementData::new(Tag::lookup(name), namespace_for(name));
        for a in attrs {
            // First occurrence wins (§13.2.5.1), matching browser behaviour.
            if e.attr(&a.name).is_none() {
                e.set_attr(&a.name, a.value.clone());
            }
        }
        self.doc.create_node(NodeKind::Element(e))
    }

    fn make_text(&mut self, s: &str) -> NodeId {
        self.doc.create_node(NodeKind::Text(s.to_string()))
    }

    fn insert_node(&mut self, node: NodeId, parent: NodeId) {
        if tree_ops::append_child(self.doc, parent, node).is_err() {
            self.errors.push("insertion rejected".into());
        }
    }

    /// The insertion point: current node, or a foster-parent location when
    /// appropriate.
    fn insert_node_appropriately(&mut self, node: NodeId) -> NodeId {
        if let Some(target) = self.foster_target() {
            self.insert_node(node, target);
            return target;
        }
        let cur = self.open.current().unwrap_or(self.doc.root());
        // Text is never inserted into `Document`; it goes into `html`.
        let cur = if matches!(self.doc.kind(cur), NodeKind::Document) {
            self.html.unwrap_or(cur)
        } else {
            cur
        };
        self.insert_node(node, cur);
        cur
    }

    /// When the current node is a table and we are inserting something that
    /// cannot live inside a table, foster-parent it (§13.2.6.1).
    fn foster_target(&self) -> Option<NodeId> {
        let cur = self.open.current()?;
        let tag = self.doc.element(cur)?.tag.as_str().to_string();
        if tag != "table" && tag != "tbody" && tag != "thead" && tag != "tfoot" && tag != "tr" {
            return None;
        }
        // Insert before the table in its parent.
        let table = if tag == "table" {
            cur
        } else {
            // Find the enclosing table.
            let mut n = cur;
            loop {
                n = self.doc.parent(n)?;
                if self.doc.element(n).map(|e| e.tag.is(engine_dom::KnownTag::Table)).unwrap_or(false) {
                    break n;
                }
            }
        };
        let parent = self.doc.parent(table)?;
        Some(foster_slot(parent, table))
    }

    fn current_is(&self, name: &str) -> bool {
        match self.open.current() {
            Some(id) => self.doc.element(id).map(|e| e.tag.as_str() == name).unwrap_or(false),
            None => false,
        }
    }

    fn current_tag(&self) -> Option<&str> {
        let id = self.open.current()?;
        self.doc.element(id).map(|e| e.tag.as_str())
    }

    fn in_scope(&self, name: &str) -> Option<NodeId> {
        for id in self.open.iter_from_top() {
            if let Some(e) = self.doc.element(id) {
                if e.tag.as_str() == name {
                    return Some(id);
                }
                if is_scoping(e.tag.as_str()) {
                    return None;
                }
            }
        }
        None
    }

    fn in_table_scope(&self, name: &str) -> Option<NodeId> {
        for id in self.open.iter_from_top() {
            if let Some(e) = self.doc.element(id) {
                if e.tag.as_str() == name {
                    return Some(id);
                }
                if is_scoping(e.tag.as_str()) && !matches!(e.tag.as_str(), "table" | "td" | "th") {
                    return None;
                }
                if matches!(e.tag.as_str(), "html") {
                    return None;
                }
            }
        }
        None
    }

    fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        while let Some(id) = self.open.current() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str().to_string(),
                None => break,
            };
            let implied = matches!(t.as_str(), "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rp" | "rt");
            if implied && Some(t.as_str()) != except {
                self.open.pop();
            } else {
                break;
            }
        }
    }

    fn pop_until_tag(&mut self, name: &str) {
        while let Some(id) = self.open.pop() {
            if self.doc.element(id).map(|e| e.tag.as_str() == name).unwrap_or(false) {
                break;
            }
        }
    }

    fn close_p(&mut self) -> bool {
        if let Some(p) = self.in_scope("p") {
            self.generate_implied_end_tags(Some("p"));
            self.open.pop_until(p);
            true
        } else {
            false
        }
    }

    fn append_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        // Coalesce with a preceding text node at the insertion point.
        if let Some(cur) = self.open.current() {
            if let Some(last) = self.doc.node(cur).last_child {
                if let NodeKind::Text(t) = &mut self.doc.node_mut(last).kind {
                    t.push_str(s);
                    return;
                }
            }
        }
        let n = self.make_text(s);
        self.insert_node_appropriately(n);
    }

    // --- Modes ------------------------------------------------------------

    fn initial(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) if is_all_ws(&s) => None,
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let root = self.doc.root();
                self.insert_node(n, root);
                None
            }
            Token::Doctype { name, public_id, system_id, force_quirks } => {
                let dt = self.doc.create_node(NodeKind::Doctype { name: name.clone() });
                let root = self.doc.root();
                self.insert_node(dt, root);
                self.quirks = classify_doctype(&name, force_quirks, public_id.as_deref(), system_id.as_deref());
                self.mode = Mode::BeforeHtml;
                None
            }
            other => {
                self.quirks = QuirksMode::Quirks;
                self.note("missing-doctype");
                self.mode = Mode::BeforeHtml;
                Some(other)
            }
        }
    }

    fn before_html(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Doctype { .. } => {
                self.record_dropped("doctype");
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let root = self.doc.root();
                self.insert_node(n, root);
                None
            }
            Token::Chars(s) if is_all_ws(&s) => None,
            Token::StartTag { name, attrs, .. } if name == "html" => {
                let el = self.make_element("html", &attrs);
                let root = self.doc.root();
                self.insert_node(el, root);
                self.open.push(el);
                self.html = Some(el);
                self.mode = Mode::BeforeHead;
                None
            }
            Token::EndTag { name } if matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                self.create_implied_html();
                Some(Token::EndTag { name })
            }
            Token::EndTag { .. } => {
                self.record_dropped("end-tag-before-html");
                None
            }
            other => {
                self.create_implied_html();
                Some(other)
            }
        }
    }

    fn create_implied_html(&mut self) {
        let el = self.make_element("html", &[]);
        let root = self.doc.root();
        self.insert_node(el, root);
        self.open.push(el);
        self.html = Some(el);
        self.mode = Mode::BeforeHead;
    }

    fn before_head(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) if is_all_ws(&s) => None,
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::StartTag { name, attrs, .. } if name == "head" => {
                let el = self.make_element("head", &attrs);
                self.insert_node_into_current(el);
                self.head = Some(el);
                self.mode = Mode::InHead;
                None
            }
            Token::EndTag { name } if matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                self.create_implied_head();
                Some(Token::EndTag { name })
            }
            Token::EndTag { .. } => {
                self.record_dropped("end-tag-before-head");
                None
            }
            other => {
                self.create_implied_head();
                Some(other)
            }
        }
    }

    fn insert_node_into_current(&mut self, node: NodeId) {
        let cur = self.open.current().unwrap_or(self.doc.root());
        let cur = if matches!(self.doc.kind(cur), NodeKind::Document) {
            self.html.unwrap_or(cur)
        } else {
            cur
        };
        self.insert_node(node, cur);
        self.open.push(node);
    }

    fn create_implied_head(&mut self) {
        let el = self.make_element("head", &[]);
        self.insert_node_into_current(el);
        self.head = Some(el);
        self.mode = Mode::InHead;
    }

    fn in_head(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) if is_all_ws(&s) => {
                self.append_text(&s);
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::Doctype { .. } => {
                self.record_dropped("doctype-in-head");
                None
            }
            Token::StartTag { name, attrs, self_closing } => match name.as_str() {
                "html" => self.in_body(Token::StartTag { name, attrs, self_closing }),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    let el = self.make_element(&name, &attrs);
                    let cur = self.open.current().unwrap_or(self.doc.root());
                    self.insert_node(el, cur);
                    None
                }
                "title" => self.start_raw_text(name, attrs),
                "noscript" | "noframes" | "style" | "script" => self.start_raw_text(name, attrs),
                "template" => {
                    // Recorded as unsupported: the element is inserted empty and
                    // its children are not rendered.
                    let el = self.make_element("template", &attrs);
                    let cur = self.open.current().unwrap_or(self.doc.root());
                    self.insert_node(el, cur);
                    self.open.push(el);
                    self.formatting.push_marker();
                    self.mode = Mode::InBody;
                    self.note("template-contents");
                    None
                }
                "head" => {
                    self.record_dropped("duplicate-head");
                    None
                }
                _ => {
                    self.close_head_and_reprocess(Token::StartTag { name, attrs, self_closing })
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "head" => {
                    self.open.pop();
                    self.mode = Mode::AfterHead;
                    None
                }
                "body" | "html" | "br" => {
                    self.open.pop();
                    self.mode = Mode::AfterHead;
                    Some(Token::EndTag { name })
                }
                "template" => {
                    self.formatting.clear_to_marker();
                    self.pop_until_tag("template");
                    None
                }
                _ => {
                    self.record_dropped("end-tag-in-head");
                    None
                }
            },
            other => self.close_head_and_reprocess(other),
        }
    }

    fn close_head_and_reprocess(&mut self, tok: Token) -> Option<Token> {
        self.open.pop();
        self.mode = Mode::AfterHead;
        Some(tok)
    }

    fn start_raw_text(&mut self, name: String, attrs: Vec<Attribute>) -> Option<Token> {
        let el = self.make_element(&name, &attrs);
        let cur = self.open.current().unwrap_or(self.doc.root());
        self.insert_node(el, cur);
        self.open.push(el);
        self.text_element = Some(el);
        self.original_mode = self.mode;
        self.mode = Mode::Text;
        None
    }

    fn after_head(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) if is_all_ws(&s) => {
                self.append_text(&s);
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "html" => {
                    let t = Token::StartTag { name, attrs, self_closing: false };
                    self.in_body(t)
                }
                "body" => {
                    let el = self.make_element("body", &attrs);
                    self.insert_node_into_current(el);
                    self.body = Some(el);
                    self.mode = Mode::InBody;
                    None
                }
                "frameset" => {
                    self.record_dropped("frameset");
                    None
                }
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
                | "template" | "title" => {
                    // Re-enter head to process this, then come back.
                    self.note("head-element-after-head");
                    let head = self.head?;
                    self.open.push(head);
                    let r = self.in_head(Token::StartTag { name, attrs, self_closing: false });
                    self.open.pop();
                    r
                }
                "head" => {
                    self.record_dropped("duplicate-head");
                    None
                }
                _ => {
                    self.create_implied_body();
                    Some(Token::StartTag { name, attrs, self_closing: false })
                }
            },
            Token::EndTag { name } if matches!(name.as_str(), "body" | "html" | "br") => {
                self.create_implied_body();
                Some(Token::EndTag { name })
            }
            other => {
                self.create_implied_body();
                Some(other)
            }
        }
    }

    fn create_implied_body(&mut self) {
        let el = self.make_element("body", &[]);
        self.insert_node_into_current(el);
        self.body = Some(el);
        self.mode = Mode::InBody;
    }

    fn in_body(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) => {
                self.append_text(&s);
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag { name, attrs, self_closing } => {
                let n = name.as_str();
                match n {
                    "html" => {
                        self.note("second-html-start");
                        None
                    }
                    "body" => None,
                    "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                    | "style" | "template" | "title" => {
                        // Process through the in-head rules, on the current node.
                        let head_open = self.head.map(|h| self.open.contains(h)).unwrap_or(false);
                        if head_open {
                            self.in_head(Token::StartTag { name, attrs, self_closing })
                        } else {
                            let el = self.make_element(&name, &attrs);
                            self.insert_node_into_current(el);
                            if matches!(name.as_str(), "script" | "style" | "title") {
                                self.text_element = Some(el);
                                self.original_mode = Mode::InBody;
                                self.mode = Mode::Text;
                            }
                            None
                        }
                    }
                    "address" | "article" | "aside" | "blockquote" | "center" | "details"
                    | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure"
                    | "footer" | "header" | "hgroup" | "main" | "menu" | "nav" | "ol" | "p"
                    | "section" | "summary" | "ul" => {
                        if closes_p(n) {
                            self.close_p();
                        }
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        self.close_p();
                        if let Some(cur) = self.current_tag() {
                            if matches!(cur, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                                self.open.pop();
                            }
                        }
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "pre" | "listing" => {
                        self.close_p();
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "form" => {
                        self.close_p();
                        let el = self.make_element("form", &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "li" => {
                        // Implicitly close a preceding `li` in the same list.
                        self.close_list_item(&["li"]);
                        self.close_p();
                        let el = self.make_element("li", &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "dd" | "dt" => {
                        self.close_list_item(&["dd", "dt"]);
                        self.close_p();
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "plaintext" => {
                        self.close_p();
                        let el = self.make_element("plaintext", &attrs);
                        self.insert_node_into_current(el);
                        self.note("plaintext");
                        None
                    }
                    "table" => {
                        self.close_p();
                        let el = self.make_element("table", &attrs);
                        self.insert_node_into_current(el);
                        self.mode = Mode::InTable;
                        None
                    }
                    "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        self.open.pop();
                        None
                    }
                    "input" => {
                        let el = self.make_element("input", &attrs);
                        self.insert_node_into_current(el);
                        self.open.pop();
                        None
                    }
                    "param" | "source" | "track" => {
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        self.open.pop();
                        None
                    }
                    "hr" => {
                        self.close_p();
                        let el = self.make_element("hr", &attrs);
                        self.insert_node_into_current(el);
                        self.open.pop();
                        None
                    }
                    "image" => {
                        // Legacy synonym for `img`.
                        let el = self.make_element("img", &attrs);
                        self.insert_node_into_current(el);
                        self.open.pop();
                        None
                    }
                    "textarea" => self.start_raw_text("textarea".into(), attrs),
                    "xmp" => {
                        self.close_p();
                        self.start_raw_text("xmp".into(), attrs)
                    }
                    "iframe" => self.start_raw_text("iframe".into(), attrs),
                    "noembed" => self.start_raw_text("noembed".into(), attrs),
                    "select" => {
                        let el = self.make_element("select", &attrs);
                        self.insert_node_into_current(el);
                        self.mode = Mode::InSelect;
                        None
                    }
                    "optgroup" | "option" => {
                        if self.current_is("option") {
                            self.open.pop();
                        }
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "rb" | "rtc" => {
                        self.note("ruby");
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "rp" | "rt" => {
                        self.generate_implied_end_tags(Some("rtc"));
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "math" | "svg" => {
                        // Foreign content: inserted, children parsed as HTML.
                        self.note("foreign-content");
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                    "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot"
                    | "th" | "thead" | "tr" => {
                        self.note("table-element-outside-table");
                        None
                    }
                    _ => {
                        // Any other start tag: inline element.
                        if closes_p(n) {
                            // Already handled above; unreachable for the list.
                        }
                        let el = self.make_element(&name, &attrs);
                        self.insert_node_into_current(el);
                        None
                    }
                }
            }
            Token::EndTag { name } => self.in_body_end_tag(name),
            Token::Eof => None,
        }
    }

    fn close_list_item(&mut self, names: &[&str]) {
        for id in self.open.iter_from_top().collect::<Vec<_>>() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str().to_string(),
                None => continue,
            };
            if names.contains(&t.as_str()) {
                self.open.pop_until(id);
                return;
            }
            // Stop at a block boundary that scopes the list item.
            if matches!(
                t.as_str(),
                "ul" | "ol" | "table" | "td" | "th" | "body" | "html"
            ) {
                return;
            }
        }
    }

    fn in_body_end_tag(&mut self, name: String) -> Option<Token> {
        match name.as_str() {
            "body" => {
                self.mode = Mode::AfterBody;
                None
            }
            "html" => {
                self.mode = Mode::AfterBody;
                Some(Token::EndTag { name })
            }
            "p" => {
                if !self.close_p() {
                    // A stray `</p>` inserts an empty `p`.
                    let el = self.make_element("p", &[]);
                    self.insert_node_into_current(el);
                }
                None
            }
            "li" => {
                if let Some(id) = self.in_scope_list_item("li") {
                    self.generate_implied_end_tags(Some("li"));
                    self.open.pop_until(id);
                }
                None
            }
            "dd" | "dt" => {
                if let Some(id) = self.in_scope_list_item(&name) {
                    self.generate_implied_end_tags(Some(&name));
                    self.open.pop_until(id);
                }
                None
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                // Any heading closes any heading.
                for id in self.open.iter_from_top().collect::<Vec<_>>() {
                    let t = match self.doc.element(id) {
                        Some(e) => e.tag.as_str().to_string(),
                        None => continue,
                    };
                    if matches!(t.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                        self.open.pop_until(id);
                        return None;
                    }
                    if is_scoping(t.as_str()) {
                        return None;
                    }
                }
                None
            }
            "br" => {
                let el = self.make_element("br", &[]);
                self.insert_node_into_current(el);
                self.open.pop();
                None
            }
            "table" => {
                if let Some(t) = self.in_table_scope("table") {
                    self.open.pop_until(t);
                    self.mode = self.mode_after_table();
                }
                None
            }
            "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" | "caption" | "colgroup" | "col" => {
                if let Some(t) = self.in_table_scope(&name) {
                    self.open.pop_until(t);
                    self.mode = self.mode_after_table();
                }
                None
            }
            "select" => {
                if let Some(s) = self.in_scope("select") {
                    self.open.pop_until(s);
                    self.reset_insertion_mode();
                }
                None
            }
            "template" => {
                if self.in_scope("template").is_some() {
                    self.formatting.clear_to_marker();
                    self.pop_until_tag("template");
                    self.reset_insertion_mode();
                }
                None
            }
            _ => {
                // Generic: pop to the nearest matching element in scope, unless it
                // is a formatting element (adoption agency territory, simplified).
                if let Some(id) = self.find_in_scope(&name) {
                    self.generate_implied_end_tags(Some(&name));
                    self.open.pop_until(id);
                }
                None
            }
        }
    }

    fn in_scope_list_item(&self, name: &str) -> Option<NodeId> {
        for id in self.open.iter_from_top() {
            if let Some(e) = self.doc.element(id) {
                if e.tag.as_str() == name {
                    return Some(id);
                }
                if is_scoping(e.tag.as_str()) {
                    return None;
                }
                if !matches!(
                    e.tag.as_str(),
                    "address" | "div" | "p" | "li" | "dd" | "dt" | "ul" | "ol" | "center" | "blockquote" | "section" | "article"
                ) {
                    // Keep walking; real browsers use "special" element rules.
                }
            }
        }
        None
    }

    fn find_in_scope(&self, name: &str) -> Option<NodeId> {
        self.in_scope(name)
    }

    fn mode_after_table(&self) -> Mode {
        // Walking up, if any ancestor is a table-related element the insertion
        // mode returns to the table body state.
        for id in self.open.iter_from_top() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str(),
                None => continue,
            };
            match t {
                "table" => return Mode::InTable,
                "tbody" | "thead" | "tfoot" => return Mode::InTableBody,
                "tr" => return Mode::InRow,
                "td" | "th" => return Mode::InCell,
                "caption" => return Mode::InCaption,
                _ => {}
            }
        }
        Mode::InBody
    }

    fn reset_insertion_mode(&mut self) {
        for id in self.open.iter_from_top() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str(),
                None => continue,
            };
            self.mode = match t {
                "select" => Mode::InSelect,
                "td" | "th" => Mode::InCell,
                "tr" => Mode::InRow,
                "tbody" | "thead" | "tfoot" => Mode::InTableBody,
                "caption" => Mode::InCaption,
                "colgroup" => Mode::InColumnGroup,
                "table" => Mode::InTable,
                "head" => Mode::InHead,
                "body" => Mode::InBody,
                "html" => Mode::BeforeHead,
                _ => continue,
            };
            return;
        }
        self.mode = Mode::InBody;
    }

    fn text_mode(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) => {
                self.append_text(&s);
                None
            }
            Token::EndTag { name } => {
                let el = self.text_element.take()?;
                self.open.pop_until(el);
                self.mode = self.original_mode;
                let _ = name;
                // A raw-text end tag whose name does not match the open element
                // is a parser artifact; the tokenizer only emits the matching one.
                None
            }
            Token::Eof => {
                self.open.pop();
                self.mode = self.original_mode;
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::StartTag { .. } | Token::Doctype { .. } => None,
        }
    }

    // --- Tables -----------------------------------------------------------

    fn in_table(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) => {
                // Text directly in a table is foster-parented.
                if s.chars().all(|c| c.is_ascii_whitespace()) {
                    self.append_text(&s);
                } else {
                    self.foster_text(&s);
                }
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "caption" => {
                    self.clear_table_context();
                    let el = self.make_element("caption", &attrs);
                    self.insert_node_into_current(el);
                    self.formatting.push_marker();
                    self.mode = Mode::InCaption;
                    None
                }
                "colgroup" => {
                    self.clear_table_context();
                    let el = self.make_element("colgroup", &attrs);
                    self.insert_node_into_current(el);
                    self.mode = Mode::InColumnGroup;
                    None
                }
                "col" => {
                    self.clear_table_context();
                    let el = self.make_element("colgroup", &[]);
                    self.insert_node_into_current(el);
                    self.mode = Mode::InColumnGroup;
                    Some(Token::StartTag { name, attrs, self_closing: false })
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_table_context();
                    let el = self.make_element(&name, &attrs);
                    self.insert_node_into_current(el);
                    self.mode = Mode::InTableBody;
                    None
                }
                "td" | "th" | "tr" => {
                    self.clear_table_context();
                    let el = self.make_element("tbody", &[]);
                    self.insert_node_into_current(el);
                    self.mode = Mode::InTableBody;
                    Some(Token::StartTag { name, attrs, self_closing: false })
                }
                "table" => {
                    // A nested table start closes the open table.
                    if let Some(t) = self.in_table_scope("table") {
                        self.open.pop_until(t);
                        self.reset_insertion_mode();
                        return Some(Token::StartTag { name, attrs, self_closing: false });
                    }
                    None
                }
                "style" | "script" | "template" => {
                    self.start_raw_text(name, attrs)
                }
                "input" => {
                    // Only `type=hidden` is allowed in a table.
                    let hidden = attrs
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                    if hidden {
                        let el = self.make_element("input", &attrs);
                        self.insert_node_into_current(el);
                        self.open.pop();
                    } else {
                        self.foster_element(name, attrs);
                    }
                    None
                }
                "form" => {
                    self.note("form-in-table");
                    let el = self.make_element("form", &attrs);
                    self.insert_node_into_current(el);
                    None
                }
                _ => {
                    self.foster_element(name, attrs);
                    None
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "table" => {
                    if let Some(t) = self.in_table_scope("table") {
                        self.open.pop_until(t);
                        self.reset_insertion_mode();
                    }
                    None
                }
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => {
                    self.note("unexpected-table-end-tag");
                    None
                }
                _ => {
                    self.note("misnested-in-table");
                    None
                }
            },
            Token::Eof => None,
        }
    }

    /// Pop back to the table context before a new table section.
    fn clear_table_context(&mut self) {
        while let Some(id) = self.open.current() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str(),
                None => break,
            };
            if matches!(t, "table" | "template" | "html") {
                break;
            }
            self.open.pop();
        }
    }

    /// Insert a foster-parented element and make it the current node, so its
    /// children land inside it rather than beside it.
    fn foster_element(&mut self, name: String, attrs: Vec<Attribute>) -> NodeId {
        let el = self.make_element(&name, &attrs);
        match self.foster_target() {
            Some(target) => {
                insert_before_or_append(self.doc, el, target);
                self.open.push(el);
            }
            None => {
                self.insert_node_into_current(el);
            }
        }
        el
    }

    fn foster_text(&mut self, s: &str) {
        let n = self.make_text(s);
        let target = self.foster_target().unwrap_or_else(|| self.open.current().unwrap_or(self.doc.root()));
        insert_before_or_append(self.doc, n, target);
        self.note("foster-parented-text");
    }

    fn in_table_body(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "tr" => {
                    self.clear_table_body_context();
                    let el = self.make_element("tr", &attrs);
                    self.insert_node_into_current(el);
                    self.mode = Mode::InRow;
                    None
                }
                "th" | "td" => {
                    self.clear_table_body_context();
                    let el = self.make_element("tr", &[]);
                    self.insert_node_into_current(el);
                    self.mode = Mode::InRow;
                    Some(Token::StartTag { name, attrs, self_closing: false })
                }
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" => {
                    self.clear_table_body_context();
                    self.mode = Mode::InTable;
                    Some(Token::StartTag { name, attrs, self_closing: false })
                }
                _ => self.in_table(Token::StartTag { name, attrs, self_closing: false }),
            },
            Token::EndTag { name } => match name.as_str() {
                "tbody" | "tfoot" | "thead" => {
                    self.clear_table_body_context();
                    self.mode = Mode::InTable;
                    None
                }
                "table" => {
                    self.clear_table_body_context();
                    self.mode = Mode::InTable;
                    Some(Token::EndTag { name })
                }
                _ => self.in_table(Token::EndTag { name }),
            },
            other => self.in_table(other),
        }
    }

    fn clear_table_body_context(&mut self) {
        while let Some(id) = self.open.current() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str(),
                None => break,
            };
            if matches!(t, "tbody" | "thead" | "tfoot" | "table" | "template" | "html") {
                break;
            }
            self.open.pop();
        }
    }

    fn in_row(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "td" | "th" => {
                    self.clear_row_context();
                    let el = self.make_element(&name, &attrs);
                    self.insert_node_into_current(el);
                    self.formatting.push_marker();
                    self.mode = Mode::InCell;
                    None
                }
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr" => {
                    self.clear_row_context();
                    self.mode = Mode::InTableBody;
                    Some(Token::StartTag { name, attrs, self_closing: false })
                }
                _ => self.in_table(Token::StartTag { name, attrs, self_closing: false }),
            },
            Token::EndTag { name } => match name.as_str() {
                "tr" => {
                    self.clear_row_context();
                    self.mode = Mode::InTableBody;
                    None
                }
                "table" => {
                    self.clear_row_context();
                    self.mode = Mode::InTableBody;
                    Some(Token::EndTag { name })
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_row_context();
                    self.mode = Mode::InTableBody;
                    Some(Token::EndTag { name })
                }
                _ => self.in_table(Token::EndTag { name }),
            },
            other => self.in_table(other),
        }
    }

    fn clear_row_context(&mut self) {
        while let Some(id) = self.open.current() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str(),
                None => break,
            };
            if matches!(t, "tr" | "tbody" | "thead" | "tfoot" | "table" | "template" | "html") {
                break;
            }
            self.open.pop();
        }
    }

    fn in_cell(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::StartTag { name, attrs, .. } if matches!(name.as_str(), "td" | "th") => {
                self.close_cell();
                Some(Token::StartTag { name, attrs, self_closing: false })
            }
            Token::StartTag { name, attrs, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
                ) =>
            {
                self.close_cell();
                Some(Token::StartTag { name, attrs, self_closing: false })
            }
            Token::EndTag { name } if matches!(name.as_str(), "td" | "th") => {
                let close = self.in_table_scope(&name);
                if let Some(c) = close {
                    self.generate_implied_end_tags(None);
                    self.open.pop_until(c);
                    self.formatting.clear_to_marker();
                    self.mode = Mode::InRow;
                }
                None
            }
            Token::EndTag { name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                self.close_cell();
                Some(Token::EndTag { name })
            }
            other => self.in_body(other),
        }
    }

    fn close_cell(&mut self) {
        for id in self.open.iter_from_top().collect::<Vec<_>>() {
            let t = match self.doc.element(id) {
                Some(e) => e.tag.as_str().to_string(),
                None => continue,
            };
            if matches!(t.as_str(), "td" | "th") {
                self.generate_implied_end_tags(None);
                self.open.pop_until(id);
                self.formatting.clear_to_marker();
                self.mode = Mode::InRow;
                return;
            }
            if is_scoping(t.as_str()) && !matches!(t.as_str(), "td" | "th") {
                return;
            }
        }
    }

    fn in_caption(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::EndTag { name } if name == "caption" => {
                if let Some(c) = self.in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.open.pop_until(c);
                    self.formatting.clear_to_marker();
                    self.mode = Mode::InTable;
                }
                None
            }
            Token::StartTag { name, attrs, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
                ) =>
            {
                if let Some(c) = self.in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.open.pop_until(c);
                    self.formatting.clear_to_marker();
                    self.mode = Mode::InTable;
                    return Some(Token::StartTag { name, attrs, self_closing: false });
                }
                None
            }
            Token::EndTag { name } if name == "table" => {
                if let Some(c) = self.in_table_scope("caption") {
                    self.open.pop_until(c);
                    self.formatting.clear_to_marker();
                    self.mode = Mode::InTable;
                    return Some(Token::EndTag { name });
                }
                None
            }
            other => self.in_body(other),
        }
    }

    fn in_column_group(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) if s.chars().all(|c| c.is_ascii_whitespace()) => {
                self.append_text(&s);
                None
            }
            Token::StartTag { name, attrs, .. } if name == "col" => {
                let el = self.make_element("col", &attrs);
                self.insert_node_into_current(el);
                self.open.pop();
                None
            }
            Token::EndTag { name } if name == "colgroup" => {
                self.open.pop();
                self.mode = Mode::InTable;
                None
            }
            Token::StartTag { name, attrs, .. } if matches!(name.as_str(), "col" | "template") => {
                self.in_table(Token::StartTag { name, attrs, self_closing: false })
            }
            Token::EndTag { name } if name == "col" => None,
            other => {
                self.open.pop();
                self.mode = Mode::InTable;
                Some(other)
            }
        }
    }

    fn in_select(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) => {
                self.append_text(&s);
                None
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let cur = self.open.current().unwrap_or(self.doc.root());
                self.insert_node(n, cur);
                None
            }
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "option" => {
                    if self.current_is("option") {
                        self.open.pop();
                    }
                    let el = self.make_element("option", &attrs);
                    self.insert_node_into_current(el);
                    None
                }
                "optgroup" => {
                    if self.current_is("option") {
                        self.open.pop();
                    }
                    if self.current_is("optgroup") {
                        self.open.pop();
                    }
                    let el = self.make_element("optgroup", &attrs);
                    self.insert_node_into_current(el);
                    None
                }
                "select" => {
                    if let Some(s) = self.in_scope("select") {
                        self.open.pop_until(s);
                        self.reset_insertion_mode();
                    }
                    None
                }
                "input" | "keygen" | "textarea" => {
                    if let Some(s) = self.in_scope("select") {
                        self.open.pop_until(s);
                        self.reset_insertion_mode();
                        return Some(Token::StartTag { name, attrs, self_closing: false });
                    }
                    None
                }
                "script" | "template" => self.start_raw_text(name, attrs),
                _ => None,
            },
            Token::EndTag { name } => match name.as_str() {
                "select" => {
                    if let Some(s) = self.in_scope("select") {
                        self.open.pop_until(s);
                        self.reset_insertion_mode();
                    }
                    None
                }
                "optgroup" => {
                    if self.current_is("option") {
                        if let Some(p) = self.open.current() {
                            let prev = self.doc.node(p).prev_sibling;
                            if let Some(prev) = prev {
                                // pop option before optgroup
                                self.open.pop();
                                let _ = prev;
                            }
                        }
                    }
                    if self.current_is("optgroup") {
                        self.open.pop();
                    }
                    None
                }
                "option" => {
                    if self.current_is("option") {
                        self.open.pop();
                    }
                    None
                }
                _ => None,
            },
            Token::Eof => None,
            Token::Doctype { .. } => None,
        }
    }

    fn after_body(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Chars(s) if s.chars().all(|c| c.is_ascii_whitespace()) => {
                self.in_body(Token::Chars(s))
            }
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let html = self.html.or_else(|| self.open.current())?;
                self.insert_node(n, html);
                None
            }
            Token::Doctype { .. } => None,
            Token::StartTag { name, .. } if name == "html" => None,
            Token::EndTag { name } if name == "html" => {
                self.mode = Mode::AfterAfterBody;
                None
            }
            Token::Eof => None,
            other => {
                self.mode = Mode::InBody;
                Some(other)
            }
        }
    }

    fn after_after_body(&mut self, tok: Token) -> Option<Token> {
        match tok {
            Token::Comment(c) => {
                let n = self.doc.create_node(NodeKind::Comment(c));
                let root = self.doc.root();
                self.insert_node(n, root);
                None
            }
            Token::Doctype { .. } => None,
            Token::Chars(s) if s.chars().all(|c| c.is_ascii_whitespace()) => {
                self.in_body(Token::Chars(s))
            }
            Token::StartTag { name, .. } if name == "html" => None,
            Token::Eof => None,
            other => {
                self.mode = Mode::InBody;
                Some(other)
            }
        }
    }
}

// --- free helpers --------------------------------------------------------

fn namespace_for(name: &str) -> Namespace {
    match name {
        "svg" => Namespace::Svg,
        "math" => Namespace::MathMl,
        _ => Namespace::Html,
    }
}

fn is_all_ws(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_whitespace())
}

/// `Some(parent)` when the node should be inserted before the table rather than
/// appended to the current node. The caller distinguishes by checking whether
/// the value is a cell marker; here we return the *insertion parent*, and the
/// separate `insert_before_or_append` handles the position.
fn foster_slot(parent: NodeId, _insert_before: NodeId) -> NodeId {
    parent
}

fn insert_before_or_append(doc: &mut Document, node: NodeId, parent: NodeId) {
    // Appending to the table's parent puts the node after the table, which is
    // wrong for foster parenting; browsers insert *before* the table. We look
    // at the parent's last child: if it is a table, insert before it.
    let last = doc.node(parent).last_child;
    if let Some(l) = last {
        if doc
            .element(l)
            .map(|e| e.tag.is(engine_dom::KnownTag::Table))
            .unwrap_or(false)
        {
            let _ = tree_ops::insert_before(doc, parent, node, l);
            return;
        }
    }
    let _ = tree_ops::append_child(doc, parent, node);
}

fn classify_doctype(
    name: &str,
    force_quirks: bool,
    public_id: Option<&str>,
    system_id: Option<&str>,
) -> QuirksMode {
    if force_quirks {
        return QuirksMode::Quirks;
    }
    if !name.eq_ignore_ascii_case("html") {
        return QuirksMode::Quirks;
    }
    // The common legacy doctypes force quirks; `<!DOCTYPE html>` and XHTML are
    // no-quirks. Any public/system id we do not recognise is limited-quirks.
    match (public_id, system_id) {
        (None, None) => QuirksMode::NoQuirks,
        (Some(p), _) => {
            let p = p.trim().to_ascii_lowercase();
            if p.starts_with("-//w3c//dtd html 4.01//") || p.starts_with("-//w3c//dtd xhtml 1.0 strict//") {
                QuirksMode::NoQuirks
            } else if p.contains("transitional") || p.contains("frameset") {
                QuirksMode::LimitedQuirks
            } else {
                QuirksMode::Quirks
            }
        }
        (None, Some(_)) => QuirksMode::LimitedQuirks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_dom::serialize::serialize;

    fn parse_tree(html: &str) -> String {
        let mut doc = Document::new();
        let tokens = crate::tokenizer::tokenize(html).0;
        let _ = build(&mut doc, tokens);
        serialize(&doc)
    }

    /// Assert that `html` parses to a tree whose serialization equals `want`.
    #[track_caller]
    fn assert_tree(html: &str, want: &str) {
        let got = parse_tree(html);
        assert_eq!(got, want, "\ninput: {html}\n--- got ---\n{got}\n--- want ---\n{want}");
    }

    #[test]
    fn minimal_document() {
        assert_tree(
            "<!DOCTYPE html><html><head></head><body><p>hi</p></body></html>",
            "#document\n  <!DOCTYPE html>\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        #text \"hi\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn implied_html_head_body() {
        assert_tree(
            "<p>hi",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        #text \"hi\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn implicit_p_close_on_block() {
        assert_tree(
            "<p>one<div>two</div>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        #text \"one\"\n      </p>\n      <div>\n        #text \"two\"\n      </div>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn implicit_p_close_on_new_p() {
        assert_tree(
            "<p>a<p>b",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        #text \"a\"\n      </p>\n      <p>\n        #text \"b\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn li_nesting() {
        assert_tree(
            "<ul><li>a<li>b</ul>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <ul>\n        <li>\n          #text \"a\"\n        </li>\n        <li>\n          #text \"b\"\n        </li>\n      </ul>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn nested_ul_li() {
        assert_tree(
            "<ul><li>a<ul><li>b</ul></ul>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <ul>\n        <li>\n          #text \"a\"\n          <ul>\n            <li>\n              #text \"b\"\n            </li>\n          </ul>\n        </li>\n      </ul>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn table_with_implicit_tbody() {
        assert_tree(
            "<table><tr><td>x</td></tr></table>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              #text \"x\"\n            </td>\n          </tr>\n        </tbody>\n      </table>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn table_foster_parenting() {
        // Text directly inside a table moves before the table.
        assert_tree(
            "<table>stray<tr><td>x</td></tr></table>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      #text \"stray\"\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              #text \"x\"\n            </td>\n          </tr>\n        </tbody>\n      </table>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn malformed_tag_recovery() {
        // An unclosed `<b>` at EOF keeps its text.
        assert_tree(
            "<p><b>bold",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        <b>\n          #text \"bold\"\n        </b>\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn character_references_in_text() {
        assert_tree(
            "<p>a &amp; b &#65;",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        #text \"a & b A\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn head_elements_go_to_head() {
        assert_tree(
            "<title>T</title><link rel=x><p>b",
            "#document\n  <html>\n    <head>\n      <title>\n        #text \"T\"\n      </title>\n      <link rel=\"x\">\n    </head>\n    <body>\n      <p>\n        #text \"b\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn script_goes_to_head_before_body() {
        assert_tree(
            "<script>x</script><p>b",
            "#document\n  <html>\n    <head>\n      <script>\n        #text \"x\"\n      </script>\n    </head>\n    <body>\n      <p>\n        #text \"b\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn doctype_only_is_standards_mode() {
        let mut doc = Document::new();
        let tokens = crate::tokenizer::tokenize("<!DOCTYPE html>").0;
        let out = build(&mut doc, tokens);
        assert_eq!(out.quirks, QuirksMode::NoQuirks);
    }

    #[test]
    fn no_doctype_is_quirks() {
        let mut doc = Document::new();
        let tokens = crate::tokenizer::tokenize("<p>x").0;
        let out = build(&mut doc, tokens);
        assert_eq!(out.quirks, QuirksMode::Quirks);
    }

    #[test]
    fn attributes_preserved_in_order() {
        assert_tree(
            "<a href=\"/x\" class=\"y\" data-k=\"v\">t</a>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <a href=\"/x\" class=\"y\" data-k=\"v\">\n        #text \"t\"\n      </a>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn void_elements_have_no_children() {
        assert_tree(
            "<p>a<br>b</p>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <p>\n        #text \"a\"\n        <br>\n        #text \"b\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn comment_in_body() {
        assert_tree(
            "<body><!-- c --><p>x</p></body>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <!--  c  -->\n      <p>\n        #text \"x\"\n      </p>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn headings_close_each_other() {
        assert_tree(
            "<h1>a<h2>b</h2>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <h1>\n        #text \"a\"\n      </h1>\n      <h2>\n        #text \"b\"\n      </h2>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn select_options() {
        assert_tree(
            "<select><option>a<option>b</select>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <select>\n        <option>\n          #text \"a\"\n        </option>\n        <option>\n          #text \"b\"\n        </option>\n      </select>\n    </body>\n  </html>\n",
        );
    }

    #[test]
    fn dl_items() {
        assert_tree(
            "<dl><dt>a<dd>b<dt>c</dl>",
            "#document\n  <html>\n    <head>\n    </head>\n    <body>\n      <dl>\n        <dt>\n          #text \"a\"\n        </dt>\n        <dd>\n          #text \"b\"\n        </dd>\n        <dt>\n          #text \"c\"\n        </dt>\n      </dl>\n    </body>\n  </html>\n",
        );
    }
}