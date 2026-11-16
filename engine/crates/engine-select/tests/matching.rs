//! `engine-select` black-box tests: real HTML in, exact match answers out.
//!
//! Every test parses a small document with `engine-html`, builds the index, and
//! asks the matcher about a selector written the way a page author would. The
//! assertions are exact booleans per element, in document order — not
//! "something matched".

use engine_dom::Document;
use engine_select::{matches_selector, MatchCtx, MatchIndex};

use engine_css::{parse_selector_list, tokenize, SelectorList};

struct Fixture {
    doc: Document,
    index: MatchIndex,
}

impl Fixture {
    fn new(html: &str) -> Fixture {
        let mut doc = Document::new();
        let opts = engine_html::ParseOpts { force_utf8: true };
        engine_html::parse(&mut doc, html.as_bytes(), &opts);
        let index = MatchIndex::build(&doc);
        Fixture { doc, index }
    }

    fn sel(&self, src: &str) -> SelectorList {
        parse_selector_list(&tokenize(src)).unwrap_or_else(|e| panic!("selector `{src}`: {e:?}"))
    }

    /// Match every element in document order, returning the element's `id`
    /// attribute (or its tag with an index) alongside the verdict.
    fn matches(&self, src: &str) -> Vec<(String, bool)> {
        let list = self.sel(src);
        let ctx = MatchCtx::new(&self.doc, &self.index);
        let mut out = Vec::new();
        for id in self.doc.ids() {
            let Some(el) = self.doc.element(id) else { continue };
            let name = match el.id() {
                Some(i) => i.to_string(),
                None => format!("{}{}", el.tag.as_str(), id.0),
            };
            let any = list.selectors.iter().any(|s| matches_selector(s, id, &ctx));
            out.push((name, any));
        }
        out
    }

    /// Only the elements that matched, tagged by `id` or by `<tag>#n` in
    /// element order, so expectations do not have to know arena ids.
    fn matched_ids(&self, src: &str) -> Vec<String> {
        self.matches(src).into_iter().filter(|(_, m)| *m).map(|(n, _)| n).collect()
    }

    /// The tag names of every element the selector matches, in order.
    fn tags_matched(&self, src: &str) -> Vec<String> {
        let list = self.sel(src);
        let ctx = MatchCtx::new(&self.doc, &self.index);
        let mut out = Vec::new();
        for id in self.doc.ids() {
            let Some(el) = self.doc.element(id) else { continue };
            if list.selectors.iter().any(|s| matches_selector(s, id, &ctx)) {
                out.push(el.tag.as_str().to_string());
            }
        }
        out
    }
}

const TREE: &str = r#"<!DOCTYPE html><html><body>
  <div id="a" class="box">
    <p id="p1" class="lead">one</p>
    <p id="p2">two</p>
    <span id="s1" class="lead">three</span>
  </div>
  <div id="b">
    <p id="p3">four</p>
  </div>
  <ul id="u">
    <li id="l1">a</li>
    <li id="l2">b</li>
    <li id="l3" class="last">c</li>
  </ul>
  <a id="lnk" href="https://example.com/x" title="Example Page" lang="fr-CA">link</a>
  <input id="ck" type="checkbox" checked>
  <input id="dis" type="text" disabled>
</body></html>"#;

#[test]
fn type_selector_matches_by_tag() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("p"), vec!["p1", "p2", "p3"]);
    assert_eq!(f.matched_ids("li"), vec!["l1", "l2", "l3"]);
    assert_eq!(f.matched_ids("blockquote"), Vec::<String>::new());
}

#[test]
fn universal_matches_every_element() {
    let f = Fixture::new(TREE);
    let all = f.matches("*");
    // 1 html + 1 body + 2 div + 3 p + 1 span + 1 ul + 3 li + 1 a + 2 input
    // + 1 head = 16 elements. (`head` is an element even though the UA sheet
    // hides it, so `*` matches it.)
    assert_eq!(all.len(), 16);
    assert!(all.iter().all(|(_, m)| *m), "`*` matches everything");
}

#[test]
fn class_and_id() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids(".lead"), vec!["p1", "s1"]);
    assert_eq!(f.matched_ids("#p2"), vec!["p2"]);
    assert_eq!(f.matched_ids(".box"), vec!["a"]);
    assert_eq!(f.matched_ids(".nope"), Vec::<String>::new());
    assert_eq!(f.matched_ids("#box"), Vec::<String>::new(), "id and class are different features");
}

#[test]
fn descendant_combinator() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("div p"), vec!["p1", "p2", "p3"]);
    assert_eq!(f.matched_ids("#a p"), vec!["p1", "p2"]);
    assert_eq!(f.matched_ids("#b p"), vec!["p3"]);
    assert_eq!(f.matched_ids("ul li"), vec!["l1", "l2", "l3"]);
}

#[test]
fn child_combinator_excludes_grandchildren() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("div > p"), vec!["p1", "p2", "p3"]);
    assert_eq!(f.matched_ids("#a > p"), vec!["p1", "p2"]);
    // The span is a grandchild of body, not a child.
    assert_eq!(f.matched_ids("body > span"), Vec::<String>::new());
    // `ul > li` matches; `div > li` does not.
    assert_eq!(f.matched_ids("ul > li"), vec!["l1", "l2", "l3"]);
    assert_eq!(f.matched_ids("div > li"), Vec::<String>::new());
}

#[test]
fn adjacent_and_general_sibling() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("p + p"), vec!["p2"], "p2 follows p1");
    assert_eq!(f.matched_ids("#p1 + p"), vec!["p2"]);
    // The span directly follows p2.
    assert_eq!(f.matched_ids("p + span"), vec!["s1"]);
    // General sibling: every p after the first p in the same parent.
    assert_eq!(f.matched_ids("p ~ span"), vec!["s1"]);
    assert_eq!(f.matched_ids("li ~ li"), vec!["l2", "l3"]);
    assert_eq!(f.matched_ids("#l1 ~ li"), vec!["l2", "l3"]);
}

#[test]
fn attribute_operators() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("[href]"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[href='https://example.com/x']"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[href^='https://']"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[href$='/x']"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[href*='example']"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[title~='Page']"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[lang|='fr']"), vec!["lnk"], "dash-match: fr-CA starts with fr-");
    assert_eq!(f.matched_ids("[lang|='fr-CA']"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[title^='Page']"), Vec::<String>::new(), "prefix is position-sensitive");
    assert_eq!(f.matched_ids("a[href][title]"), vec!["lnk"]);
    assert_eq!(f.matched_ids("[title='EXAMPLE PAGE']"), Vec::<String>::new());
    assert_eq!(f.matched_ids("[title='EXAMPLE PAGE' i]"), vec!["lnk"], "`i` flag");
}

#[test]
fn nth_child_variants() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("li:nth-child(1)"), vec!["l1"]);
    assert_eq!(f.matched_ids("li:nth-child(2)"), vec!["l2"]);
    assert_eq!(f.matched_ids("li:nth-child(odd)"), vec!["l1", "l3"]);
    assert_eq!(f.matched_ids("li:nth-child(even)"), vec!["l2"]);
    assert_eq!(f.matched_ids("li:nth-child(2n+1)"), vec!["l1", "l3"]);
    assert_eq!(f.matched_ids("li:nth-last-child(1)"), vec!["l3"]);
    assert_eq!(f.matched_ids("#a > :nth-child(2)"), vec!["p2"]);
}

#[test]
fn structural_pseudo_classes() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("li:first-child"), vec!["l1"]);
    assert_eq!(f.matched_ids("li:last-child"), vec!["l3"]);
    assert_eq!(f.tags_matched("html:root"), vec!["html"]);
    assert_eq!(f.tags_matched(":root"), vec!["html"]);
    assert_eq!(f.matched_ids("p:first-of-type"), vec!["p1", "p3"]);
    assert_eq!(f.matched_ids("p:last-of-type"), vec!["p2", "p3"]);
    // `:only-of-type` on the span under #a.
    assert_eq!(f.matched_ids("span:only-of-type"), vec!["s1"]);
    assert_eq!(f.matched_ids("p:empty"), Vec::<String>::new());
}

#[test]
fn state_pseudo_classes() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids(":checked"), vec!["ck"]);
    assert_eq!(f.matched_ids(":disabled"), vec!["dis"]);
    assert_eq!(f.matched_ids("input:enabled"), vec!["ck"]);
    assert_eq!(f.matched_ids("a:link"), vec!["lnk"]);
}

#[test]
fn not_is_and_where() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("li:not(.last)"), vec!["l1", "l2"]);
    assert_eq!(f.matched_ids("p:is(.lead, #p3)"), vec!["p1", "p3"]);
    assert_eq!(f.matched_ids("p:where(.lead)"), vec!["p1"]);
    // `:not()` with a combinator inside.
    assert_eq!(f.matched_ids("li:not(#u > li:first-child)"), vec!["l2", "l3"]);
}

#[test]
fn has_descendant_search() {
    let f = Fixture::new(TREE);
    // #a contains a span.
    assert_eq!(f.matched_ids("div:has(span)"), vec!["a"]);
    // body has a p (depth 1) and a span (depth 2) — both inside MAX_HAS_DEPTH.
    assert_eq!(f.matched_ids("body:has(span)").len(), 1);
    assert_eq!(f.matched_ids("div:has(blockquote)"), Vec::<String>::new());
}

#[test]
fn specificity_tie_break_prefers_more_specific() {
    // The cascade picks the winner; the matcher's job is to say both match.
    // Assert exactly that: `.lead` and `p.lead` and `#p1` all match p1.
    let f = Fixture::new(TREE);
    let ctx = MatchCtx::new(&f.doc, &f.index);
    let p1 = f
        .doc
        .ids()
        .find(|id| f.doc.element(*id).and_then(|e| e.id()) == Some("p1"))
        .unwrap();
    for src in [".lead", "p.lead", "#p1", "div p", "body #p1"] {
        let list = f.sel(src);
        assert!(
            list.selectors.iter().any(|s| matches_selector(s, p1, &ctx)),
            "`{src}` should match #p1"
        );
    }
    // And the specificities are ordered as the spec requires.
    let sp = |src: &str| f.sel(src).selectors[0].specificity;
    assert!(sp("#p1") > sp("p.lead"));
    assert!(sp("p.lead") > sp(".lead"));
    assert!(sp(".lead") > sp("div p"));
}

#[test]
fn selector_list_is_an_or() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("h1, li.last, #p3"), vec!["p3", "l3"]);
}

// --- Hostile input: must terminate, never panic ---------------------------

#[test]
fn hostile_empty_and_unclosed_input_terminates() {
    for html in ["", "<div>", "<p><p><p>", "<table><td>x", "<b><i></b></i>", "<div class=", "<!"] {
        let f = Fixture::new(html);
        // Any selector against any tree must terminate.
        let _ = f.matched_ids("div p");
        let _ = f.matched_ids("*:has(p)");
        let _ = f.matched_ids(":nth-child(2n+1 of p)");
        let _ = f.matched_ids("[a^=''][b$='']");
    }
}

#[test]
fn deep_nesting_terminates() {
    let mut html = String::from("<html><body>");
    for i in 0..10_000 {
        html.push_str(&format!("<div class=\"d{i}\">"));
    }
    html.push_str("<p id=\"deep\">x</p>");
    let f = Fixture::new(&html);
    // Descendant matching across 10k compounds and 10k ancestors.
    let deep = f
        .doc
        .ids()
        .find(|id| f.doc.element(*id).and_then(|e| e.id()) == Some("deep"))
        .unwrap();
    let ctx = MatchCtx::new(&f.doc, &f.index);
    let mut sel_src = String::new();
    for _ in 0..100 {
        sel_src.push_str("div ");
    }
    sel_src.push('p');
    let list = f.sel(&sel_src);
    assert!(
        list.selectors.iter().any(|s| matches_selector(s, deep, &ctx)),
        "100-deep descendant chain must match"
    );
    // A selector that cannot match must also terminate, false.
    let list = f.sel("span p");
    assert!(!list.selectors.iter().any(|s| matches_selector(s, deep, &ctx)));
}

#[test]
fn deeply_nested_pseudo_arguments_do_not_overflow() {
    let f = Fixture::new(TREE);
    // `engine-css` caps selector nesting at 64, so 60 levels parses and must
    // evaluate to a definite answer. 60 `:not()` wrappers are an even number of
    // negations, so `p` stays matching.
    let mut src = String::from("p");
    for _ in 0..60 {
        src = format!("p:not({src})");
    }
    let list = f.sel(&src);
    let ctx = MatchCtx::new(&f.doc, &f.index);
    let p1 = f
        .doc
        .ids()
        .find(|id| f.doc.element(*id).and_then(|e| e.id()) == Some("p1"))
        .unwrap();
    assert!(
        list.selectors.iter().any(|s| matches_selector(s, p1, &ctx)),
        "even number of nested :not() is a no-op"
    );
}

#[test]
fn unsupported_pseudo_never_matches_and_is_reported() {
    let f = Fixture::new(TREE);
    assert_eq!(f.matched_ids("p:hover"), Vec::<String>::new());
    assert_eq!(f.matched_ids("p::before"), Vec::<String>::new());
    let list = f.sel("p::before, a:has(span), p:unknown-pseudo");
    let unsupported = engine_select::unsupported_selectors(&list);
    assert!(unsupported.iter().any(|s| s == "::before"), "got {unsupported:?}");
    assert!(unsupported.iter().any(|s| s == ":unknown-pseudo"), "got {unsupported:?}");
    assert!(unsupported.iter().any(|s| s.starts_with(":has()")), "got {unsupported:?}");
    // `:hover` parses to a supported variant; it matches only under the flag.
    let list = f.sel("a:hover");
    assert!(engine_select::unsupported_selectors(&list).is_empty());
    assert_eq!(f.matched_ids("a:hover"), Vec::<String>::new(), "no hover flag set");
}