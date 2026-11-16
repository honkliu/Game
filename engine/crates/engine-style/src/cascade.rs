//! The cascade and the style tree (§4 Stage 3, §5.4–5.5).
//!
//! Per element, per longhand: collect every declaration that matches, pick the
//! highest (origin, importance, specificity, source order), then resolve
//! CSS-wide keywords against the parent and the initial value.
//!
//! **Origin order** (CSS Cascade 4 §6.1, lowest to highest):
//!
//! | # | origin | important |
//! |---|--------|-----------|
//! | 1 | user-agent | no |
//! | 2 | user | no |
//! | 3 | author | no |
//! | 4 | inline `style` attribute | no |
//! | 5 | author | yes |
//! | 6 | user | yes |
//! | 7 | user-agent | yes |
//!
//! The rank is packed into one `u64` so comparing two declarations is a single
//! integer compare: `origin_rank << 56 | specificity.key() << 24 | order`.
//!
//! **Font-size is resolved first.** `em` in every other declaration resolves
//! against the element's own computed font size, so the winning `font-size` has
//! to be known before any other property can be interpreted (§5.5, "the classic
//! em-cascade bug").

use std::sync::LazyLock;

use ahash::AHashMap;
use engine_core::NodeId;
use engine_css::properties::ComputedStyle;
use engine_css::{
    parse_declarations, DeclaredValue, DeclarationBlock, LengthCtx, Origin, Specificity,
    Stylesheet, WideKeyword,
};
use engine_dom::Document;
use engine_select::{matches_selector, MatchCtx, MatchIndex};

use crate::apply::{self, ApplyResult};
use crate::media_eval::{self, MediaContext};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// The computed style of one node; `None` for non-elements.
#[derive(Clone, Debug, Default)]
pub struct StyleTree {
    /// One entry per DOM node.
    pub styles: Vec<Option<ComputedStyle>>,
    /// Elements whose style was computed.
    pub styled_elements: usize,
    pub report: StyleReport,
}

#[derive(Clone, Debug, Default)]
pub struct StyleReport {
    /// `(property, reason)` for declarations we recognised but could not apply.
    pub unsupported_declarations: Vec<(String, String)>,
    /// Selectors that can never match.
    pub unsupported_selectors: Vec<String>,
    /// At-rules the CSS parser skipped (`@font-face`, `@media` we could not
    /// evaluate, ...).
    pub skipped_at_rules: Vec<String>,
}

impl StyleTree {
    pub fn get(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.styles.get(id.index()).and_then(|s| s.as_ref())
    }

    /// The style for an element, or the shared initial style otherwise.
    /// `ComputedStyle` owns a `Vec`, so the fallback is a `LazyLock`.
    pub fn style_of(&self, id: NodeId) -> &ComputedStyle {
        self.get(id).unwrap_or(&INITIAL_STYLE)
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
}

static INITIAL_STYLE: LazyLock<ComputedStyle> = LazyLock::new(ComputedStyle::default);

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Compute the style tree for `doc` under `sheets` at `viewport`.
///
/// `sheets` may be in any order; each sheet's `origin` decides its rank.
pub fn compute_styles(
    doc: &Document,
    sheets: &[Stylesheet],
    viewport: engine_core::Viewport,
) -> StyleTree {
    compute_styles_with_media(doc, sheets, &MediaContext::new(viewport))
}

pub fn compute_styles_with_media(
    doc: &Document,
    sheets: &[Stylesheet],
    media: &MediaContext,
) -> StyleTree {
    let index = MatchIndex::build(doc);
    let ctx = MatchCtx::new(doc, &index);
    let inline = InlineStyles::collect(doc);

    // Flatten every rule across sheets. Source order is global so that later
    // sheets win within an origin, the way one concatenated sheet would behave.
    let mut rules: Vec<RuleRef<'_>> = Vec::new();
    let mut report = StyleReport::default();
    // Global source order: each sheet's own `rule.order` starts at 0, so two
    // author sheets would tie. Offset each sheet past the previous one's rule
    // count so "later sheet wins" holds within an origin.
    let mut order_base: u32 = 0;
    for sheet in sheets {
        report.skipped_at_rules.extend(sheet.skipped_at_rules.iter().cloned());
        let origin = sheet.origin.unwrap_or(Origin::Author);
        for rule in &sheet.rules {
            let active = media_eval::all_hold(&rule.media, media);
            report
                .unsupported_selectors
                .extend(engine_select::unsupported_selectors(&rule.selectors));
            rules.push(RuleRef {
                selectors: &rule.selectors,
                decls: &rule.declarations,
                origin,
                order: order_base + rule.order,
                active,
            });
        }
        order_base = order_base.saturating_add(sheet.rules.len() as u32);
    }
    // Bucket the rules by the rightmost compound of every selector in their
    // list, so a restyle only tests rules that could match the element.
    let buckets = RuleBuckets::build(&rules);

    let mut tree = StyleTree {
        styles: vec![None; doc.len()],
        styled_elements: 0,
        report,
    };

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut hits: Vec<u32> = Vec::new();

    for id in doc.ids() {
        let Some(el) = doc.element(id) else { continue };
        if el.tag.is(engine_dom::KnownTag::Template) {
            continue; // §15: template contents are not rendered.
        }
        let parent = doc.parent(id);
        let parent_style: Option<&ComputedStyle> =
            parent.and_then(|p| tree.styles.get(p.index())).and_then(|s| s.as_ref());

        // Starting point: initials + inherited from the parent.
        let mut style = ComputedStyle::default();
        if let Some(p) = parent_style {
            apply::reset_and_inherit(&mut style, p);
        }
        let parent_font = parent_style.map_or(engine_core::DEFAULT_FONT_SIZE_PX, |p| p.font_size_px);

        // Collect candidates for this element.
        candidates.clear();
        let el_index = index.get(id);
        // A node that is not an element cannot reach here (the loop skips it),
        // so a missing index entry means the two walks disagree — skip it.
        let Some(el_index) = el_index else { continue };
        buckets.candidates(Some(el_index), &mut hits);
        for &ri in hits.iter() {
            let r = &rules[ri as usize];
            if !r.active {
                continue;
            }
            // Every selector in the rule's list is tested; the rule applies if
            // any of them matches. Against the same `!important` the highest
            // matching specificity is the one that competes.
            let best = r
                .selectors
                .selectors
                .iter()
                .filter(|s| matches_selector(s, id, &ctx))
                .map(|s| s.specificity)
                .max();
            let Some(spec) = best else { continue };
            for d in &r.decls.decls {
                candidates.push(Candidate {
                    property: d.property.clone(),
                    value: d.value.clone(),
                    rank: rank_of(r.origin, d.important, spec, r.order),
                });
            }
        }
        if let Some(block) = inline.by_node.get(&id) {
            for d in &block.decls {
                candidates.push(Candidate {
                    property: d.property.clone(),
                    value: d.value.clone(),
                    rank: inline_rank(d.important),
                });
            }
        }

        // --- Pass 1: font-size ---
        let font_size = resolve_font_size(&candidates, parent_font);

        // --- Pass 2: winners ---
        let mut winners: AHashMap<&str, usize> = AHashMap::new();
        for (i, c) in candidates.iter().enumerate() {
            match winners.get(c.property.as_str()) {
                Some(&prev) if candidates[prev].rank >= c.rank => {}
                _ => {
                    winners.insert(&c.property, i);
                }
            }
        }
        let mut chosen: Vec<usize> = winners.into_values().collect();
        // Deterministic application order (property name, then rank).
        chosen.sort_by(|a, b| {
            let (a, b) = (&candidates[*a], &candidates[*b]);
            a.property.cmp(&b.property).then(a.rank.cmp(&b.rank))
        });

        for idx in chosen {
            let c = &candidates[idx];
            match &c.value {
                // `kw` is already a `WideKeyword` (Copy) behind a reference from
                // the `&c.value` match, so pass it through without a deref.
                DeclaredValue::WideKeyword(kw) => {
                    apply_wide_keyword(&mut style, parent_style, &c.property, *kw, font_size);
                }
                value => {
                    match apply::apply(&mut style, &c.property, value, font_size) {
                        Some(ApplyResult::Applied) | None => {}
                        Some(ApplyResult::Unsupported) => {
                            if tree.report.unsupported_declarations.len() < 256 {
                                tree.report
                                    .unsupported_declarations
                                    .push((c.property.clone(), "not implemented".into()));
                            }
                        }
                    }
                }
            }
        }

        // A hostile page can set `font-size: 0` (valid CSS, unusable text) or a
        // negative percentage. Clamp to the initial size so layout and text
        // never see a zero or negative scale.
        if !(font_size.is_finite() && font_size > 0.0) {
            style.font_size_px = engine_core::DEFAULT_FONT_SIZE_PX;
        } else {
            style.font_size_px = font_size;
        }

        tree.styled_elements += 1;
        tree.styles[id.index()] = Some(style);
    }

    tree.report.unsupported_selectors.sort();
    tree.report.unsupported_selectors.dedup();
    tree.report.unsupported_declarations.sort();
    tree.report.unsupported_declarations.dedup();
    tree
}

// ---------------------------------------------------------------------------
// Rule collection
// ---------------------------------------------------------------------------

struct RuleRef<'a> {
    selectors: &'a engine_css::SelectorList,
    decls: &'a DeclarationBlock,
    origin: Origin,
    order: u32,
    active: bool,
}

/// Rules partitioned by the cheapest discriminating feature of their first
/// selector's rightmost compound. Not on `RightmostBuckets` because that type
/// is keyed per *selector*; here the unit is the rule.
#[derive(Default)]
struct RuleBuckets {
    by_id: AHashMap<String, Vec<u32>>,
    by_class: AHashMap<String, Vec<u32>>,
    by_tag: AHashMap<String, Vec<u32>>,
    universal: Vec<u32>,
}

impl RuleBuckets {
    fn build(rules: &[RuleRef<'_>]) -> RuleBuckets {
        let mut b = RuleBuckets::default();
        for (i, r) in rules.iter().enumerate() {
            let i = i as u32;
            if r.selectors.selectors.is_empty() {
                b.universal.push(i);
                continue;
            }
            // A rule with a selector list (`div, p, h1 { display: block }`) must
            // be reachable through *every* selector in the list, not just the
            // first. The UA sheet's block-element rule has 39 selectors; keying
            // it only on `html` would make `div` never see `display: block`.
            let mut buckets_for: Vec<(&str, String)> = Vec::new();
            let mut any_universal = false;
            for sel in &r.selectors.selectors {
                let Some(c) = sel.compounds.first() else {
                    any_universal = true;
                    continue;
                };
                let mut id = None;
                let mut class = None;
                let mut tag = None;
                for s in &c.simples {
                    match s {
                        engine_css::SimpleSelector::Id(v) => id = Some(v.clone()),
                        engine_css::SimpleSelector::Class(v) if class.is_none() => class = Some(v.clone()),
                        engine_css::SimpleSelector::Type(t) => tag = t.name.clone(),
                        _ => {}
                    }
                }
                if let Some(v) = id {
                    buckets_for.push(("id", v));
                } else if let Some(v) = class {
                    buckets_for.push(("class", v));
                } else if let Some(v) = tag {
                    buckets_for.push(("tag", v));
                } else {
                    any_universal = true;
                }
            }
            if any_universal || buckets_for.is_empty() {
                b.universal.push(i);
            }
            for (kind, key) in buckets_for {
                let map = match kind {
                    "id" => &mut b.by_id,
                    "class" => &mut b.by_class,
                    _ => &mut b.by_tag,
                };
                let v = map.entry(key).or_default();
                if v.last() != Some(&i) {
                    v.push(i);
                }
            }
        }
        b
    }

    /// Rule indices that could match `el`. Superset of the true matches.
    fn candidates(&self, el: Option<&engine_select::ElementIndex>, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.universal);
        let Some(el) = el else { return };
        if let Some(id) = &el.id {
            if let Some(v) = self.by_id.get(id.as_ref()) {
                out.extend_from_slice(v);
            }
        }
        for c in &el.classes {
            if let Some(v) = self.by_class.get(c.as_ref()) {
                out.extend_from_slice(v);
            }
        }
        if let Some(t) = &el.tag {
            if let Some(v) = self.by_tag.get(t.as_str()) {
                out.extend_from_slice(v);
            }
        }
    }
}

struct Candidate {
    property: String,
    value: DeclaredValue,
    rank: Rank,
}

// ---------------------------------------------------------------------------
// Inline styles
// ---------------------------------------------------------------------------

struct InlineStyles {
    by_node: AHashMap<NodeId, DeclarationBlock>,
}

impl InlineStyles {
    fn collect(doc: &Document) -> Self {
        let mut by_node = AHashMap::new();
        // A length context is not needed for inline text: every length unit is
        // already folded to px/pct at parse time except `em`, which the parser
        // resolves with its default font size. Inline `em` values are therefore
        // resolved against 16px instead of the element's own size — a recorded
        // limitation (see the report).
        let ctx = LengthCtx::default();
        for id in doc.ids() {
            let Some(el) = doc.element(id) else { continue };
            let Some(src) = el.attr("style") else { continue };
            let tokens = engine_css::tokenize(src);
            if tokens.is_empty() {
                continue;
            }
            let body = &tokens[..tokens.len() - 1];
            let mut block = DeclarationBlock::default();
            let _ = parse_declarations(body, &ctx, &mut block);
            by_node.insert(id, block);
        }
        InlineStyles { by_node }
    }
}

/// Inline `style` ranks above author normal (3) and below author `!important`
/// (5). An inline declaration always beats any selector at the same
/// importance, whatever its specificity, so it carries the maximum specificity.
fn inline_rank(important: bool) -> Rank {
    let origin_rank: u8 = if important { 5 } else { 4 };
    Rank { origin: origin_rank, spec: Specificity::new(u32::MAX, u32::MAX, u32::MAX), order: u32::MAX }
}

// ---------------------------------------------------------------------------
// Ranking, font size, wide keywords
// ---------------------------------------------------------------------------

/// Cascade precedence of one declaration (CSS Cascade 4 §6.1).
///
/// A struct with a derived `Ord` rather than a packed integer: packing forces
/// fixed bit widths on `order` and specificity, and a sheet with more than 256
/// rules silently loses tie-breaking. Three fields compare in the right order
/// and never overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    /// 1 (UA normal) ..= 7 (UA `!important`). Inline style is 4 / 5.
    origin: u8,
    spec: Specificity,
    order: u32,
}

fn rank_of(origin: Origin, important: bool, spec: Specificity, order: u32) -> Rank {
    let origin_rank: u8 = match (origin, important) {
        (Origin::UserAgent, false) => 1,
        (Origin::User, false) => 2,
        (Origin::Author, false) => 3,
        (Origin::Author, true) => 5,
        (Origin::User, true) => 6,
        (Origin::UserAgent, true) => 7,
    };
    Rank { origin: origin_rank, spec, order }
}

/// The winning `font-size` in px. Falls back to the inherited size.
fn resolve_font_size(candidates: &[Candidate], inherited: f32) -> f32 {
    let mut best: Option<&Candidate> = None;
    for c in candidates.iter().filter(|c| c.property == "font-size") {
        if best.is_none_or(|b| c.rank > b.rank) {
            best = Some(c);
        }
    }
    let Some(c) = best else { return inherited };
    match &c.value {
        DeclaredValue::Length(engine_core::Length::Px(v)) if *v > 0.0 => *v,
        DeclaredValue::Length(engine_core::Length::Pct(p)) => (inherited * p / 100.0).max(0.0),
        DeclaredValue::WideKeyword(WideKeyword::Initial) => engine_core::DEFAULT_FONT_SIZE_PX,
        // `inherit`/`unset`/`revert` and anything unusable keep the inherited
        // value; the element's own `font-size` is set from this result.
        _ => inherited,
    }
}

fn apply_wide_keyword(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    property: &str,
    kw: WideKeyword,
    font_size: f32,
) {
    // `ComputedStyle` owns a `Vec<String>` for `font-family`, so it cannot be
    // a `const`; a shared `LazyLock` avoids rebuilding it per element.
    // `font-size` is set from `resolve_font_size` afterwards, so a wide keyword
    // on it only matters for `initial`.
    if property == "font-size" {
        if kw == WideKeyword::Initial {
            style.font_size_px = engine_core::DEFAULT_FONT_SIZE_PX;
        } else {
            style.font_size_px = font_size;
        }
        return;
    }
    match kw {
        WideKeyword::Inherit => {
            if let Some(p) = parent {
                copy_property(style, p, property);
            } else {
                copy_property(style, &INITIAL_STYLE, property);
            }
        }
        WideKeyword::Initial => copy_property(style, &INITIAL_STYLE, property),
        // `unset` = inherit for inherited properties, initial otherwise.
        // `revert` is approximated as `unset` because we do not track which
        // origin each declaration came from per longhand (§15, reported).
        WideKeyword::Unset | WideKeyword::Revert => {
            if is_inherited(property) {
                if let Some(p) = parent {
                    copy_property(style, p, property);
                } else {
                    copy_property(style, &INITIAL_STYLE, property);
                }
            } else {
                copy_property(style, &INITIAL_STYLE, property);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-property copy and the inheritance table
// ---------------------------------------------------------------------------

/// Copy one property's value from `src` to `dst`.
///
/// One arm per longhand. A reflective property table would be a bigger
/// abstraction for a list that changes only when the CSS longhand set does.
fn copy_property(dst: &mut ComputedStyle, src: &ComputedStyle, property: &str) {
    match property {
        "display" => dst.display = src.display,
        "position" => dst.position = src.position,
        "float" => dst.float = src.float,
        "clear" => dst.clear = src.clear,
        "overflow-x" => dst.overflow_x = src.overflow_x,
        "overflow-y" => dst.overflow_y = src.overflow_y,
        "z-index" => dst.z_index = src.z_index,
        "width" => dst.width = src.width,
        "height" => dst.height = src.height,
        "min-width" => dst.min_width = src.min_width,
        "max-width" => dst.max_width = src.max_width,
        "min-height" => dst.min_height = src.min_height,
        "max-height" => dst.max_height = src.max_height,
        "margin-top" => dst.margin.top = src.margin.top,
        "margin-right" => dst.margin.right = src.margin.right,
        "margin-bottom" => dst.margin.bottom = src.margin.bottom,
        "margin-left" => dst.margin.left = src.margin.left,
        "padding-top" => dst.padding.top = src.padding.top,
        "padding-right" => dst.padding.right = src.padding.right,
        "padding-bottom" => dst.padding.bottom = src.padding.bottom,
        "padding-left" => dst.padding.left = src.padding.left,
        "border-top-width" => dst.border_widths.top = src.border_widths.top,
        "border-right-width" => dst.border_widths.right = src.border_widths.right,
        "border-bottom-width" => dst.border_widths.bottom = src.border_widths.bottom,
        "border-left-width" => dst.border_widths.left = src.border_widths.left,
        "border-top-style" => dst.border_styles[0] = src.border_styles[0],
        "border-right-style" => dst.border_styles[1] = src.border_styles[1],
        "border-bottom-style" => dst.border_styles[2] = src.border_styles[2],
        "border-left-style" => dst.border_styles[3] = src.border_styles[3],
        "border-top-color" => dst.border_colors[0] = src.border_colors[0],
        "border-right-color" => dst.border_colors[1] = src.border_colors[1],
        "border-bottom-color" => dst.border_colors[2] = src.border_colors[2],
        "border-left-color" => dst.border_colors[3] = src.border_colors[3],
        "border-top-left-radius" => dst.border_radius.top_left = src.border_radius.top_left,
        "border-top-right-radius" => dst.border_radius.top_right = src.border_radius.top_right,
        "border-bottom-right-radius" => dst.border_radius.bottom_right = src.border_radius.bottom_right,
        "border-bottom-left-radius" => dst.border_radius.bottom_left = src.border_radius.bottom_left,
        "top" => dst.top = src.top,
        "right" => dst.right = src.right,
        "bottom" => dst.bottom = src.bottom,
        "left" => dst.left = src.left,
        "background-color" => dst.background.color = src.background.color,
        "opacity" => dst.opacity = src.opacity,
        "color" => dst.color = src.color,
        "font-size" => dst.font_size_px = src.font_size_px,
        "font-weight" => dst.font_weight = src.font_weight,
        "font-style" => dst.font_style = src.font_style,
        "font-family" => dst.font_family = src.font_family.clone(),
        "line-height" => dst.line_height = src.line_height,
        "letter-spacing" => dst.letter_spacing = src.letter_spacing,
        "text-align" => dst.text_align = src.text_align,
        "text-decoration-line" => dst.text_decoration = src.text_decoration,
        "white-space" => dst.white_space = src.white_space,
        "vertical-align" => dst.vertical_align = src.vertical_align,
        "list-style-type" => dst.list_style_type = src.list_style_type,
        "list-style-position" => dst.list_style_position = src.list_style_position,
        "flex-direction" => dst.flex_direction = src.flex_direction,
        "flex-wrap" => dst.flex_wrap = src.flex_wrap,
        "justify-content" => dst.justify_content = src.justify_content,
        "align-items" => dst.align_items = src.align_items,
        "flex-grow" => dst.flex_grow = src.flex_grow,
        "flex-shrink" => dst.flex_shrink = src.flex_shrink,
        "flex-basis" => dst.flex_basis = src.flex_basis,
        "row-gap" => dst.gap_row = src.gap_row,
        "column-gap" => dst.gap_column = src.gap_column,
        "box-sizing" => dst.box_sizing = src.box_sizing,
        "visibility" => dst.visibility_hidden = src.visibility_hidden,
        "cursor" => dst.cursor_pointer = src.cursor_pointer,
        "content" => dst.content = src.content.clone(),
        _ => {}
    }
}

/// Whether a longhand inherits (§5.5). A test asserts this agrees with
/// `engine_css::properties::inherit::apply_inherited` for every name listed.
pub fn is_inherited(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-family"
            | "line-height"
            | "letter-spacing"
            | "text-align"
            | "white-space"
            | "list-style-type"
            | "list-style-position"
            | "visibility"
            | "cursor"
            | "table-layout"
            | "border-spacing"
    )
}

/// Every longhand name the cascade can copy, for the inheritance test.
pub const ALL_LONGHANDS: &[&str] = &[
    "display", "position", "float", "clear", "overflow-x", "overflow-y", "z-index", "width",
    "height", "min-width", "max-width", "min-height", "max-height", "margin-top", "margin-right",
    "margin-bottom", "margin-left", "padding-top", "padding-right", "padding-bottom",
    "padding-left", "border-top-width", "border-right-width", "border-bottom-width",
    "border-left-width", "border-top-style", "border-right-style", "border-bottom-style",
    "border-left-style", "border-top-color", "border-right-color", "border-bottom-color",
    "border-left-color", "border-top-left-radius", "border-top-right-radius",
    "border-bottom-right-radius", "border-bottom-left-radius", "top", "right", "bottom", "left",
    "background-color", "opacity", "color", "font-size", "font-weight", "font-style",
    "font-family", "line-height", "letter-spacing", "text-align", "text-decoration-line",
    "white-space", "vertical-align", "list-style-type", "list-style-position", "flex-direction",
    "flex-wrap", "justify-content", "align-items", "flex-grow", "flex-shrink", "flex-basis",
    "row-gap", "column-gap", "box-sizing", "visibility", "cursor",
];