//! Block flow (CSS 2.1 §9.4.1, §10.3.3, §10.6.3, §8.3.1).
//!
//! [`place_block`] resolves a box's horizontal geometry, lays out its children,
//! resolves its height, and reports the margins that escape to the parent so the
//! parent can collapse them. That return value is the whole reason this function
//! is one function rather than three passes.
//!
//! **Margin collapsing model.** A box's margins do not live on the box; they
//! live in the gap between boxes. Concretely:
//! - a box reports `outer_before` / `outer_after`: the collapsed margin that
//!   extends *outside* its border box on each edge. These are `Some` only when
//!   that edge may interact with its parent — no top/bottom border or padding and
//!   not a formatting-context root on that edge;
//! - a box consumes its children's escaping margins and folds them into its own
//!   reported values, so a chain of nested boxes with nobody separating them
//!   collapses as one margin;
//! - a container stacks its children by folding each child's `outer_before`
//!   into the running margin with [`collapse_margins`], which is
//!   `max(positive) - max(|negative|)` (§8.3.1).
//!
//! **Approximated:** collapsing *through* an empty block into its following
//! sibling; collapsing of floats; and collapsing across `overflow` boundaries in
//! the vertical-direction-only case. Recorded as
//! [`Approximation::EmptyBlockCollapse`] / [`crate::MAX_LAYOUT_DEPTH`].

use std::sync::Arc;

use engine_core::{BoxId, EdgeSizes, FragmentId, Length, Rect, Size};
use engine_css::properties::{BorderStyle, BoxSizing, Position};
use engine_css::ComputedStyle;
use engine_text::FontContext;

use crate::box_tree::{Approximation, BoxKind, BoxTree, LayoutBox};
use crate::fragments::{Fragment, LineFragment, ResolvedEdges, TextFragment, UsedMargins};
use crate::inline_flow::{self, Item, Line};
use crate::{LayoutTree, MAX_LAYOUT_DEPTH};

/// Resolve a computed length against a percentage base. `None` for `auto`.
pub fn resolve(l: Length, base: f32) -> Option<f32> {
    match l {
        Length::Px(v) => Some(v),
        Length::Pct(p) => Some(base * p / 100.0),
        Length::Auto => None,
    }
}

/// Resolve, treating `auto` as zero.
pub fn resolve_zero(l: Length, base: f32) -> f32 {
    resolve(l, base).unwrap_or(0.0)
}

/// The used box-model edges of a style against a containing-block width.
pub fn used_edges(style: &ComputedStyle, cb_width: f32) -> ResolvedEdges {
    let p = |l: Length| resolve_zero(l, cb_width).max(0.0);
    let padding = EdgeSizes::new(p(style.padding.top), p(style.padding.right), p(style.padding.bottom), p(style.padding.left));
    // A border with `border-style: none` has zero used width (§8.5.3).
    let bw = |i: usize, l: Length| {
        if style.border_styles[i] == BorderStyle::None {
            0.0
        } else {
            resolve_zero(l, cb_width).max(0.0)
        }
    };
    let border = EdgeSizes::new(
        bw(0, style.border_widths.top),
        bw(1, style.border_widths.right),
        bw(2, style.border_widths.bottom),
        bw(3, style.border_widths.left),
    );
    // Margins keep their sign: negative margins are legal.
    let margin = EdgeSizes::new(
        resolve_zero(style.margin.top, cb_width),
        resolve_zero(style.margin.right, cb_width),
        resolve_zero(style.margin.bottom, cb_width),
        resolve_zero(style.margin.left, cb_width),
    );
    ResolvedEdges { margin, border, padding }
}

/// The collapsed value of two adjoining margins (§8.3.1):
/// `max(positive margins) - max(|negative margins|)`. Associative, so folding a
/// set in any order gives the same answer.
pub fn collapse_margins(a: f32, b: f32) -> f32 {
    let pos = a.max(0.0).max(b.max(0.0));
    let neg = (-a).max(0.0).max((-b).max(0.0));
    pos - neg
}

/// Fold a slice of margins.
pub fn collapse_all(margins: impl IntoIterator<Item = f32>) -> f32 {
    margins.into_iter().fold(0.0, collapse_margins)
}

/// The result of laying out one block-level box.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub fragment: FragmentId,
    /// Margin extending before the border box, collapsed and ready to fold with
    /// a sibling's. `None` when this edge cannot collapse (a top border or
    /// padding, or a formatting-context root at this edge).
    pub outer_before: Option<f32>,
    /// Margin extending after the border box. `None` when it cannot collapse.
    pub outer_after: Option<f32>,
}

/// State threaded through one layout pass.
pub struct Ctx<'a> {
    pub fonts: &'a mut FontContext,
    pub viewport: Size,
    pub quirks: engine_dom::QuirksMode,
    pub approximations: Vec<Approximation>,
}

impl Ctx<'_> {
    pub(crate) fn record(&mut self, a: Approximation) {
        if !self.approximations.contains(&a) {
            self.approximations.push(a);
        }
    }
}

/// The percentage base for a height, or `None` when a percentage height does not
/// resolve (an indefinite containing block, which is every `height: auto`
/// ancestor padded to the viewport only at the root).
fn height_base(style: &ComputedStyle, base: Option<f32>) -> Option<f32> {
    if style.height.is_auto() {
        None
    } else {
        base
    }
}

/// Lay out `child` as a block-level box. `x`/`y` are the containing block's
/// **content** origin; the caller has already advanced `y` by whatever margin it
/// decided to place above this box, so `y` here is the box's border-box top.
///
/// `cb_height` is the containing block's content height when it is definite.
#[allow(clippy::too_many_arguments)]
pub fn place_block(
    tree: &BoxTree,
    child: BoxId,
    x: f32,
    y: f32,
    cb_width: f32,
    cb_height: Option<f32>,
    out: &mut LayoutTree,
    ctx: &mut Ctx<'_>,
    depth: u32,
) -> Placed {
    let b = tree.get(child);
    let style = &*b.style;
    if let Some(a) = b.approximation {
        ctx.record(a);
    }
    if depth >= MAX_LAYOUT_DEPTH {
        ctx.record(Approximation::NestingDepthLimit);
    }
    let e = used_edges(style, cb_width);

    // --- Horizontal geometry (§10.3.3) ------------------------------------
    let extra = e.padding.horizontal() + e.border.horizontal();
    let avail = (cb_width - extra - e.margin.left - e.margin.right).max(0.0);
    let (content_w, used_ml, used_mr) = match resolve(style.width, cb_width) {
        Some(specified) => {
            let cw = clamp_width(
                if style.box_sizing == BoxSizing::BorderBox {
                    (specified - extra).max(0.0)
                } else {
                    specified
                },
                style,
                cb_width,
            );
            let free = cb_width - (cw + extra + e.margin.left + e.margin.right);
            let (ml, mr) = auto_margins(style, free);
            (cw, ml.unwrap_or(e.margin.left), mr.unwrap_or(e.margin.right))
        }
        None => {
            let cw = clamp_width(avail, style, cb_width);
            let free = cb_width - (cw + extra + e.margin.left + e.margin.right);
            let (ml, mr) = auto_margins(style, free);
            (cw, ml.unwrap_or(e.margin.left), mr.unwrap_or(e.margin.right))
        }
    };

    let border_x = x + used_ml;
    let border_y = y;
    let content_x = border_x + e.border.left + e.padding.left;
    let content_y = border_y + e.border.top + e.padding.top;

    // --- Specified height -------------------------------------------------
    let specified_h = resolve(style.height, cb_height.unwrap_or(0.0)).filter(|_| !style.height.is_auto() && (cb_height.is_some() || !matches!(style.height, Length::Pct(_))));
    let mut content_h = specified_h.unwrap_or(0.0);
    if b.kind == BoxKind::Replaced && specified_h.is_none() {
        content_h = b.intrinsic.h;
    }

    let provisional = Rect::new(content_x, content_y, content_w, content_h);

    // --- Children ---------------------------------------------------------
    let children = if b.kind == BoxKind::Replaced || depth >= MAX_LAYOUT_DEPTH {
        Vec::new()
    } else {
        layout_inside(tree, b, provisional, out, ctx, depth + 1)
    };

    // --- Height resolution (§10.6.3) --------------------------------------
    let mut used_content_h = match specified_h {
        Some(h) => h,
        None => {
            if b.kind == BoxKind::Replaced {
                b.intrinsic.h
            } else {
                child_extent(out, content_y, content_w, &children)
            }
        }
    };
    if specified_h.is_none() {
        if let Some(min) = resolve(style.min_height, ctx.viewport.h) {
            used_content_h = used_content_h.max(min);
        }
        if let Some(max) = resolve(style.max_height, ctx.viewport.h) {
            used_content_h = used_content_h.min(max);
        }
    }
    used_content_h = used_content_h.max(0.0);

    let border_w = content_w + e.padding.horizontal() + e.border.horizontal();
    let border_h = used_content_h + e.padding.vertical() + e.border.vertical();
    let rect = Rect::new(border_x, border_y, border_w, border_h);
    let content = Rect::new(content_x, content_y, content_w, used_content_h);
    let padding_box = Rect::new(
        border_x + e.border.left,
        border_y + e.border.top,
        content_w + e.padding.horizontal(),
        used_content_h + e.padding.vertical(),
    );
    let clips = style.overflow_x.is_clipping() || style.overflow_y.is_clipping();
    let clip = clips.then_some(padding_box);

    // --- Which margins escape to the parent (§8.3.1) ----------------------
    //
    // A margin escapes only if nothing separates it from the parent's margin:
    // no border, no padding, and not a formatting-context root on that edge.
    // The root box's margins never collapse with anything (it is the viewport).
    let edge_open_top = e.border.top == 0.0 && e.padding.top == 0.0 && !b.establishes_bfc;
    let edge_open_bottom = e.border.bottom == 0.0 && e.padding.bottom == 0.0 && !b.establishes_bfc;

    let empty = children.is_empty() && specified_h.is_none() && b.kind != BoxKind::Replaced;
    let is_inline_block = matches!(style.display, engine_css::properties::Display::InlineBlock);
    let _ = is_inline_block;

    let (outer_before, outer_after) = if empty && edge_open_top && edge_open_bottom {
        // All four margins collapse as one (§8.3.1).
        let folded = collapse_all([e.margin.top, e.margin.bottom]);
        (Some(folded), None)
    } else {
        let before = if edge_open_top { Some(e.margin.top) } else { None };
        let after = if edge_open_bottom { Some(e.margin.bottom) } else { None };
        (before, after)
    };

    let mut frag = out.push(Fragment {
        box_id: child,
        node: b.node,
        rect,
        content,
        padding_box,
        margin: UsedMargins { top: e.margin.top, right: used_mr, bottom: e.margin.bottom, left: used_ml },
        border: e.border,
        padding: e.padding,
        children: Vec::new(),
        clip,
        text: None,
        kind: b.kind,
        intrinsic: b.intrinsic,
        outer_before: None,
        outer_after: None,
    });
    // Children were laid out with ids assigned before this fragment; attach and
    // then fold their escaping margins into this box's reported edges.
    let mut top_folded = outer_before;
    for &c in &children {
        let cf = out.get(c);
        if let Some(ob) = cf.outer_before {
            // The first child's before-margin is part of this box's before-edge.
            top_folded = Some(collapse_all([top_folded.unwrap_or(e.margin.top), ob]));
        }
    }
    let mut bottom_folded = outer_after;
    if let Some(&c) = children.last() {
        if let Some(oa) = out.get(c).outer_after {
            bottom_folded = Some(collapse_all([bottom_folded.unwrap_or(e.margin.bottom), oa]));
        }
    }
    {
        let f = &mut out.fragments[frag.index()];
        f.children = children;
        f.outer_before = top_folded;
        f.outer_after = bottom_folded;
    }
    Placed { fragment: frag, outer_before: top_folded, outer_after: bottom_folded }
}

/// `margin: auto`: split free space, or give it to one side. `None` = not auto.
fn auto_margins(style: &ComputedStyle, free: f32) -> (Option<f32>, Option<f32>) {
    let left = style.margin.left.is_auto();
    let right = style.margin.right.is_auto();
    match (left, right) {
        (true, true) if free > 0.0 => (Some(free / 2.0), Some(free / 2.0)),
        (true, true) => (Some(0.0), Some(0.0)),
        (true, false) if free > 0.0 => (Some(free), None),
        (false, true) if free > 0.0 => (None, Some(free)),
        (true, false) => (Some(0.0), None),
        (false, true) => (None, Some(0.0)),
        (false, false) => (None, None),
    }
}

fn clamp_width(w: f32, style: &ComputedStyle, cb: f32) -> f32 {
    let mut w = w;
    if let Some(min) = resolve(style.min_width, cb) {
        w = w.max(min);
    }
    if let Some(max) = resolve(style.max_width, cb) {
        w = w.min(max);
    }
    w.max(0.0)
}

/// Lay out a box's inside and return its child fragments in paint order.
fn layout_inside(tree: &BoxTree, b: &LayoutBox, content: Rect, out: &mut LayoutTree, ctx: &mut Ctx<'_>, depth: u32) -> Vec<FragmentId> {
    if b.children.is_empty() {
        return Vec::new();
    }
    let any_block = b.children.iter().any(|&c| tree.get(c).kind.is_block_level());
    if !any_block {
        return vec![layout_text_run(tree, b, content, out, ctx)];
    }
    layout_block_children(tree, b, content, out, ctx, depth)
}

/// Lay a box's block-level children out as a vertical stack (§9.4.1).
fn layout_block_children(tree: &BoxTree, parent: &LayoutBox, content: Rect, out: &mut LayoutTree, ctx: &mut Ctx<'_>, depth: u32) -> Vec<FragmentId> {
    let mut ids: Vec<FragmentId> = Vec::with_capacity(parent.children.len());
    let mut cursor = content.y;
    // Margin accumulated but not yet turned into space: the collapsed run of
    // adjacent margins between the previous content and the next.
    let mut pending: Option<f32> = None;

    for (i, &c) in parent.children.iter().enumerate() {
        let child = tree.get(c);
        // A text box can appear directly only if box generation left the parent
        // without an anonymous wrapper; treat it as its own line box.
        if child.kind == BoxKind::Text {
            let placed = place_text_line(tree, parent, c, content.x, content.w, out, ctx, cursor, depth);
            let f = out.get(placed);
            let gap = pending.take().unwrap_or(0.0);
            let dy = (cursor + gap) - f.rect.y;
            shift(out, placed, dy);
            cursor += gap + f.rect.h;
            ids.push(placed);
            continue;
        }
        let top_explicit = child.style.margin.top;
        let top_margin = resolve_zero(top_explicit, content.w);
        let _ = i;
        // Place provisionally at `cursor`; the real top margin is decided below,
        // because collapsing with `pending` may change it.
        let placed = place_block(tree, c, content.x, cursor, content.w, Some(content.h), out, ctx, depth);
        let ob = placed.outer_before.unwrap_or(top_margin);
        let gap = match pending {
            None => ob,
            Some(p) => collapse_margins(p, ob),
        };
        let target_y = cursor + gap;
        let dy = target_y - out.get(placed.fragment).rect.y;
        shift(out, placed.fragment, dy);
        cursor = target_y + out.get(placed.fragment).rect.h;
        pending = placed.outer_after;
        ids.push(placed.fragment);
    }
    if let Some(p) = pending {
        // The last child's after-margin belongs to this box's own bottom edge;
        // `place_block` folds it already. Nothing to advance here.
        let _ = p;
    }
    ids
}

/// Lay out one bare text box (no anonymous wrapper) as a line of its own.
#[allow(clippy::too_many_arguments)]
fn place_text_line(
    _tree: &BoxTree,
    parent: &LayoutBox,
    child: BoxId,
    x: f32,
    width: f32,
    out: &mut LayoutTree,
    ctx: &mut Ctx<'_>,
    y: f32,
    _depth: u32,
) -> FragmentId {
    let b = _tree.get(child);
    let text = b.text.clone().unwrap_or_default();
    let owned = vec![(text, Arc::clone(&b.style))];
    let items: Vec<Item<'_>> = owned.iter().map(|(t, s)| Item { text: t.as_str(), style: s }).collect();
    let lines = inline_flow::layout(&items, &parent.style, width, ctx.fonts);
    let h: f32 = lines.iter().map(|l| l.rect.h).sum();
    let frag_lines = lines.iter().map(|l| line_to_fragment(l, x, y)).collect();
    out.push(Fragment {
        box_id: child,
        node: b.node,
        rect: Rect::new(x, y, width, h),
        content: Rect::new(x, y, width, h),
        padding_box: Rect::new(x, y, width, h),
        margin: UsedMargins::default(),
        border: EdgeSizes::ZERO,
        padding: EdgeSizes::ZERO,
        children: Vec::new(),
        clip: None,
        text: Some(TextFragment {
            text: items.iter().map(|i| i.text).collect::<Vec<_>>().join(""),
            lines: frag_lines,
            available_width: width,
            style: Arc::clone(&b.style),
        }),
        kind: BoxKind::Text,
        intrinsic: Size::ZERO,
        outer_before: Some(0.0),
        outer_after: Some(0.0),
    })
}

/// Lay out a block container whose children are all inline-level as one text
/// run, producing a single text fragment.
fn layout_text_run(tree: &BoxTree, b: &LayoutBox, content: Rect, out: &mut LayoutTree, ctx: &mut Ctx<'_>) -> FragmentId {
    let mut owned: Vec<(String, Arc<ComputedStyle>)> = Vec::new();
    collect_text(tree, b, &mut owned);
    let items: Vec<Item<'_>> = owned.iter().map(|(t, s)| Item { text: t.as_str(), style: s }).collect();
    let lines = inline_flow::layout(&items, &b.style, content.w, ctx.fonts);
    let total_h: f32 = lines.iter().map(|l| l.rect.h).sum();
    let frag_lines: Vec<LineFragment> = lines.iter().map(|l| line_to_fragment(l, content.x, content.y)).collect();
    out.push(Fragment {
        box_id: b.node,
        node: b.node,
        rect: Rect::new(content.x, content.y, content.w, total_h),
        content,
        padding_box: content,
        margin: UsedMargins::default(),
        border: EdgeSizes::ZERO,
        padding: EdgeSizes::ZERO,
        children: Vec::new(),
        clip: None,
        text: Some(TextFragment {
            text: items.iter().map(|i| i.text).collect::<Vec<_>>().join(""),
            lines: frag_lines,
            available_width: content.w,
            style: Arc::clone(&b.style),
        }),
        kind: BoxKind::Anonymous,
        intrinsic: Size::ZERO,
        // An inline formatting context never collapses a margin outward: the
        // line boxes start at the content edge and end above the content edge.
        outer_before: Some(0.0),
        outer_after: Some(0.0),
    })
}

fn line_to_fragment(l: &Line, origin_x: f32, origin_y: f32) -> LineFragment {
    LineFragment {
        rect: Rect::new(origin_x + l.rect.x, origin_y + l.rect.y, l.rect.w, l.rect.h),
        baseline: origin_y + l.baseline,
        text: l.text.clone(),
        text_x: l.text_x,
        style: Arc::clone(&l.style),
        color: l.color,
        underline: l.underline,
        line_through: l.line_through,
    }
}

/// Every `(text, style)` in an inline subtree. The style attached is the
/// innermost inline ancestor's, which is what font, colour, and line-height use.
fn collect_text(tree: &BoxTree, b: &LayoutBox, out: &mut Vec<(String, Arc<ComputedStyle>)>) {
    for &c in &b.children {
        let child = tree.get(c);
        match child.kind {
            BoxKind::Text => {
                if let Some(t) = &child.text {
                    out.push((t.clone(), Arc::clone(&child.style)));
                }
            }
            BoxKind::Inline => collect_text(tree, child, out),
            _ => {}
        }
    }
}

/// The height of a block's children, from the content origin: the last child's
/// border box plus its escaping bottom margin. Escaping top margins of the first
/// child are already outside the content box, so the measure starts at the
/// first child's border box.
fn child_extent(out: &LayoutTree, content_y: f32, _width: f32, children: &[FragmentId]) -> f32 {
    let Some(&last) = children.last() else { return 0.0 };
    let f = out.get(last);
    let bottom = f.rect.bottom() + f.outer_after.unwrap_or(f.margin.bottom);
    (bottom - content_y).max(0.0)
}

/// Translate a fragment and its subtree by `dy`.
pub fn shift(out: &mut LayoutTree, id: FragmentId, dy: f32) {
    if dy == 0.0 {
        return;
    }
    let kids = out.get(id).children.clone();
    {
        let f = &mut out.fragments[id.index()];
        f.rect.y += dy;
        f.content.y += dy;
        f.padding_box.y += dy;
        if let Some(c) = f.clip.as_mut() {
            c.y += dy;
        }
        if let Some(t) = f.text.as_mut() {
            for l in t.lines.iter_mut() {
                l.rect.y += dy;
                l.baseline += dy;
            }
        }
    }
    for k in kids {
        shift(out, k, dy);
    }
}

/// True when a style's `overflow` clips.
pub fn clips(style: &ComputedStyle) -> bool {
    style.overflow_x.is_clipping() || style.overflow_y.is_clipping()
}

/// A border box as rounded integers, for assertions.
pub fn rect_i(r: Rect) -> (i32, i32, i32, i32) {
    (r.x.round() as i32, r.y.round() as i32, r.w.round() as i32, r.h.round() as i32)
}

/// The content size of a fragment.
pub fn content_size(f: &Fragment) -> Size {
    Size::new(f.content.w, f.content.h)
}

/// Whether the display value forms a block container (§9.2.1).
pub fn is_block_container(display: engine_css::properties::Display) -> bool {
    matches!(
        display,
        engine_css::properties::Display::Block
            | engine_css::properties::Display::ListItem
            | engine_css::properties::Display::InlineBlock
            | engine_css::properties::Display::Flex
            | engine_css::properties::Display::Grid
    )
}

/// `true` for a box whose position is out of normal flow.
pub fn is_out_of_flow(style: &ComputedStyle) -> bool {
    matches!(style.position, Position::Absolute | Position::Fixed)
}

/// The percentage base for a height, or `None` when it does not resolve.
pub fn definite_height_base(style: &ComputedStyle, base: Option<f32>) -> Option<f32> {
    height_base(style, base)
}