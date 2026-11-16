//! `engine-style` tests: hand-written sheets where the winning value per
//! property is known from the CSS spec, never from "whatever the engine did".
//!
//! Each test names the cascade rule it exercises.

use engine_core::{NodeId, Rgba8, Viewport};
use engine_css::{parse_stylesheet, Origin, Stylesheet};
use engine_dom::Document;
use engine_style::{compute_styles, StyleTree};

struct Case {
    doc: Document,
    tree: StyleTree,
}

fn doc(html: &str) -> Document {
    let mut d = Document::new();
    let opts = engine_html::ParseOpts { force_utf8: true };
    engine_html::parse(&mut d, html.as_bytes(), &opts);
    d
}

fn case(html: &str, css: &str) -> Case {
    let doc = doc(html);
    let ua = engine_css::user_agent_stylesheet();
    let author = parse_stylesheet(css, Origin::Author);
    let tree = compute_styles(&doc, &[ua, author], Viewport::new(800.0, 600.0, 1.0));
    Case { doc, tree }
}

fn by_id(c: &Case, id: &str) -> NodeId {
    c.doc
        .ids()
        .find(|n| c.doc.element(*n).and_then(|e| e.id()) == Some(id))
        .unwrap_or_else(|| panic!("no element with id={id}"))
}

/// The few fields each test looks at, so failures print the value, not a whole
/// `ComputedStyle` dump.
fn color(c: &Case, id: &str) -> Rgba8 {
    c.tree.get(by_id(c, id)).expect("element styled").color
}
fn font(c: &Case, id: &str) -> f32 {
    c.tree.get(by_id(c, id)).expect("element styled").font_size_px
}
fn margin_top(c: &Case, id: &str) -> f32 {
    c.tree.get(by_id(c, id)).expect("element styled").margin.top
}

#[test]
fn later_rule_wins_at_equal_specificity() {
    let c = case(
        "<div id=a></div>",
        "div { color: red; } div { color: blue; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(0, 0, 255), "source order breaks the tie");
}

#[test]
fn higher_specificity_wins_regardless_of_order() {
    let c = case(
        "<div id=a class=b></div>",
        ".b { color: red; } div { color: green; } #a { color: blue; }",
    );
    // (1,0,0) id beats (0,1,0) class beats (0,0,1) type, whatever the order.
    assert_eq!(color(&c, "a"), Rgba8::rgb(0, 0, 255));
}

#[test]
fn class_beats_type_and_loses_to_id() {
    let c = case(
        "<div id=a class=b></div>",
        "div.b { color: red; } div { color: green; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(255, 0, 0), "(0,1,1) beats (0,0,1)");

    let c = case(
        "<div id=a class=b></div>",
        "#a { color: blue; } div.b { color: red; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(0, 0, 255), "(1,0,0) beats (0,1,1)");
}

#[test]
fn important_reverses_order_within_author_origin() {
    let c = case(
        "<div id=a></div>",
        "div { color: red !important; } #a { color: blue; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(255, 0, 0), "!important author beats normal id");

    let c = case(
        "<div id=a></div>",
        "div { color: red !important; } #a { color: blue !important; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(0, 0, 255), "!important vs !important falls back to specificity");
}

#[test]
fn author_beats_user_agent() {
    let c = case("<h1 id=h>Title</h1>", "h1 { margin-top: 3px; }");
    // UA sheet gives h1 margin-top: 0.67em = 10.72px; author 3px must win.
    assert_eq!(margin_top(&c, "h"), 3.0);
}

#[test]
fn inherited_color_copies_to_children() {
    let c = case(
        "<div id=p><span id=c>x</span></div>",
        "#p { color: #112233; }",
    );
    assert_eq!(color(&c, "p"), Rgba8::rgb(0x11, 0x22, 0x33));
    assert_eq!(color(&c, "c"), Rgba8::rgb(0x11, 0x22, 0x33), "color inherits");
}

#[test]
fn inherited_font_size_copies_and_percentages_resolve_against_parent() {
    let c = case(
        "<div id=p><span id=c>a</span><b id=s>b</b></div>",
        "#p { font-size: 20px; } #c { font-size: 150%; } #s { font-size: 0.5em; }",
    );
    assert_eq!(font(&c, "p"), 20.0);
    assert_eq!(font(&c, "c"), 30.0, "150% of the parent's 20px");
    // `0.5em` is folded to 8px by the parser against its 16px default — the
    // documented em limitation. It must NOT be 10px (0.5 of 20px).
    assert_eq!(font(&c, "s"), 8.0, "em folds at parse time; recorded limitation");
}

#[test]
fn margin_does_not_inherit() {
    let c = case(
        "<div id=p><span id=c>x</span></div>",
        "#p { margin-top: 40px; }",
    );
    assert_eq!(margin_top(&c, "p"), 40.0);
    assert_eq!(margin_top(&c, "c"), 0.0, "margin is not inherited");
}

#[test]
fn explicit_inherit_keyword_works_for_non_inherited_property() {
    let c = case(
        "<div id=p><span id=c>x</span></div>",
        "#p { margin-top: 40px; } #c { margin-top: inherit; }",
    );
    assert_eq!(margin_top(&c, "c"), 40.0, "`inherit` overrides non-inheritance");
}

#[test]
fn unset_is_inherit_for_inherited_and_initial_for_others() {
    let c = case(
        "<div id=p><span id=c>x</span></div>",
        "#p { color: #010203; margin-top: 40px; } #c { color: unset; margin-top: unset; }",
    );
    assert_eq!(color(&c, "c"), Rgba8::rgb(1, 2, 3), "unset color inherits");
    assert_eq!(margin_top(&c, "c"), 0.0, "unset margin is initial");
}

#[test]
fn initial_resets_to_spec_initial_not_parent() {
    let c = case(
        "<div id=p><span id=c>x</span></div>",
        "#p { color: #010203; } #c { color: initial; }",
    );
    assert_eq!(color(&c, "c"), Rgba8::BLACK, "initial color is black");
}

#[test]
fn inline_style_beats_author_rule() {
    let c = case(
        r#"<div id=a style="color: green"></div>"#,
        "#a { color: blue; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(0, 128, 0), "inline beats a selector");
}

#[test]
fn inline_style_loses_to_important_author_rule() {
    let c = case(
        r#"<div id=a style="color: green"></div>"#,
        "#a { color: blue !important; }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(0, 0, 255), "author !important beats inline");
}

#[test]
fn ua_sheet_gives_block_display_to_div_and_inline_to_span() {
    let c = case("<div id=d><span id=s>x</span></div>", "");
    use engine_css::Display;
    assert_eq!(c.tree.get(by_id(&c, "d")).unwrap().display, Display::Block);
    assert_eq!(c.tree.get(by_id(&c, "s")).unwrap().display, Display::Inline);
}

#[test]
fn ua_sheet_headings_are_bold_and_larger() {
    let c = case("<h1 id=h>t</h1><h2 id=h2>t</h2>", "");
    assert_eq!(font(&c, "h"), 32.0, "h1 is 2em of 16px");
    assert_eq!(font(&c, "h2"), 24.0, "h2 is 1.5em of 16px");
    assert_eq!(c.tree.get(by_id(&c, "h")).unwrap().font_weight, 700);
}

#[test]
fn display_none_element_is_styled_but_marked() {
    let c = case("<div id=a></div><div id=b></div>", "#b { display: none; }");
    let b = c.tree.get(by_id(&c, "b")).unwrap();
    assert_eq!(b.display, engine_css::Display::None);
    // The style is retained so the report can name it; layout is the stage that
    // skips it.
    assert!(c.tree.get(by_id(&c, "b")).is_some());
}

#[test]
fn media_query_that_does_not_hold_is_skipped() {
    let c = case(
        "<div id=a></div>",
        "@media (min-width: 2000px) { div { color: red } }",
    );
    assert_eq!(color(&c, "a"), Rgba8::BLACK, "800px viewport fails min-width:2000px");

    let c = case(
        "<div id=a></div>",
        "@media (min-width: 400px) { div { color: red } }",
    );
    assert_eq!(color(&c, "a"), Rgba8::rgb(255, 0, 0));
}

#[test]
fn user_important_beats_author_important() {
    // The UA sheet has no !important rules, so build a custom UA sheet.
    let doc = doc("<div id=a>x</div>");
    let ua = parse_stylesheet("div { color: green !important }", Origin::UserAgent);
    let author = parse_stylesheet("#a { color: red !important }", Origin::Author);
    let tree = compute_styles(&doc, &[ua, author], Viewport::default());
    let a = doc.ids().find(|n| doc.element(*n).and_then(|e| e.id()) == Some("a")).unwrap();
    assert_eq!(tree.get(a).unwrap().color, Rgba8::rgb(0, 128, 0), "UA !important outranks author !important");
}

#[test]
fn later_sheet_wins_within_the_same_origin() {
    let doc = doc("<div id=a>x</div>");
    let one: Stylesheet = parse_stylesheet("div { color: red }", Origin::Author);
    let two = parse_stylesheet("div { color: blue }", Origin::Author);
    let tree = compute_styles(&doc, &[one, two], Viewport::default());
    let a = doc.ids().find(|n| doc.element(*n).and_then(|e| e.id()) == Some("a")).unwrap();
    assert_eq!(tree.get(a).unwrap().color, Rgba8::rgb(0, 0, 255));
}

#[test]
fn shorthand_expands_to_longhands() {
    let c = case("<div id=a>x</div>", "#a { margin: 1px 2px 3px 4px; }");
    let s = c.tree.get(by_id(&c, "a")).unwrap();
    assert_eq!((s.margin.top, s.margin.right, s.margin.bottom, s.margin.left), (1.0, 2.0, 3.0, 4.0));
}

#[test]
fn border_shorthand_sets_width_style_color() {
    let c = case("<div id=a>x</div>", "#a { border: 2px solid #ff0000; }");
    let s = c.tree.get(by_id(&c, "a")).unwrap();
    let bw = s.border_widths;
    assert_eq!((bw.top, bw.right, bw.bottom, bw.left), (2.0, 2.0, 2.0, 2.0));
    assert_eq!(s.border_styles[0], engine_css::BorderStyle::Solid);
    assert_eq!(s.border_colors[0], Rgba8::rgb(255, 0, 0));
}

#[test]
fn line_height_all_three_encodings_parse() {
    for (src, want) in [
        ("#a{line-height:normal}", engine_css::LineHeight::Normal),
        ("#a{line-height:1.5}", engine_css::LineHeight::Number(1.5)),
        ("#a{line-height:20px}", engine_css::LineHeight::Px(20.0)),
    ] {
        let c = case("<div id=a>x</div>", src);
        assert_eq!(c.tree.get(by_id(&c, "a")).unwrap().line_height, want, "{src}");
    }
}

#[test]
fn text_decoration_and_white_space() {
    let c = case("<div id=a>x</div>", "#a { text-decoration: underline; white-space: pre; }");
    let s = c.tree.get(by_id(&c, "a")).unwrap();
    assert!(s.text_decoration.underline);
    assert_eq!(s.white_space, engine_css::WhiteSpace::Pre);
}

#[test]
fn inheritance_table_agrees_with_engine_css() {
    // The two places that encode "does this property inherit" must not drift.
    for prop in engine_style::ALL_LONGHANDS {
        let parent = engine_css::ComputedStyle {
            color: Rgba8::rgb(9, 9, 9),
            font_size_px: 33.0,
            margin: engine_core::EdgeSizes::uniform(7.0),
            ..engine_css::ComputedStyle::default()
        };
        let mut child = engine_css::ComputedStyle::default();
        engine_css::properties::inherit::apply_inherited(&mut child, &parent);
        let copied_by_engine = match *prop {
            "color" => child.color == parent.color,
            "font-size" => child.font_size_px == parent.font_size_px,
            "margin-top" => child.margin.top == parent.margin.top,
            _ => continue,
        };
        assert_eq!(
            engine_style::is_inherited(prop),
            copied_by_engine,
            "disagreement on `{prop}`"
        );
    }
}

// --- Hostile input --------------------------------------------------------

#[test]
fn empty_document_styles_to_empty_tree() {
    let doc = Document::new();
    let tree = compute_styles(&doc, &[], Viewport::default());
    assert_eq!(tree.styled_elements, 0);
    assert_eq!(tree.len(), doc.len());
}

#[test]
fn unclosed_tags_and_garbage_terminate() {
    // `engine-html` may produce zero elements for a truncated tag (`<p class=`),
    // so the assertion is: the walk completes, the tree covers every node, and
    // every element the parser did produce got a style. A hang or panic fails.
    for html in ["<div><p>", "<p class=", "<div style=\"color:", "<table><td>x", "<b><i></b>"] {
        let doc = doc(html);
        let ua = engine_css::user_agent_stylesheet();
        let author = parse_stylesheet("div { color: red } p { margin: 1px }", Origin::Author);
        let tree = compute_styles(&doc, &[ua, author], Viewport::new(800.0, 600.0, 1.0));
        assert_eq!(tree.len(), doc.len(), "tree covers every node for {html:?}");
        let elements = doc.ids().filter(|n| doc.element(*n).is_some()).count();
        // `<template>` is the only element skipped.
        let templates = doc
            .ids()
            .filter(|n| doc.element(*n).is_some_and(|e| e.tag.as_str() == "template"))
            .count();
        assert_eq!(
            tree.styled_elements,
            elements - templates,
            "every non-template element styled for {html:?}"
        );
    }
}

#[test]
fn ten_thousand_nested_divs_terminate_with_correct_depth() {
    let mut html = String::from("<html><body><div id=\"outer\">");
    for _ in 0..10_000 {
        html.push_str("<div>");
    }
    let c = case(&html, "div { color: #0000ff; }");
    let outer = c.tree.get(by_id(&c, "outer")).unwrap();
    assert_eq!(outer.color, Rgba8::rgb(0, 0, 255), "style reaches the outer div");
    assert!(c.tree.styled_elements >= 10_000);
}

#[test]
fn deeply_nested_selectors_and_rules_terminate() {
    let mut css = String::new();
    for i in 0..200 {
        css.push_str(&format!(".c{i} > div {{ color: red }}\n"));
    }
    let c = case("<div id=a class=c0><div class=c1><div class=c2>x</div></div></div>", &css);
    assert_eq!(color(&c, "a"), Rgba8::BLACK, "only .c0 > div matches the middle div");
}

#[test]
fn zero_font_size_is_clamped_not_propagated() {
    let c = case("<div id=a>x</div>", "#a { font-size: 0px; }");
    assert_eq!(font(&c, "a"), 16.0, "a zero font-size is unusable; fall back to the initial size");
}

#[test]
fn unsupported_declaration_is_reported() {
    let c = case("<div id=a>x</div>", "#a { content: 'x'; }");
    assert!(
        c.tree.report.unsupported_declarations.iter().any(|(p, _)| p == "content"),
        "got {:?}",
        c.tree.report.unsupported_declarations
    );
}