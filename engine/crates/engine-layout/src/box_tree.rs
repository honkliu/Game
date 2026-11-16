//! Box tree construction (CSS 2.1 §9.2): one box per element that generates
//! one, plus text boxes and the anonymous blocks that keep a block container's
//! children either all block-level or all inline-level.
//!
//! Rules implemented:
//! - `display: none` prunes the element and its subtree.
//! - `display: contents` prunes the element's own box; its children are
//!   promoted into the parent.
//! - A `<br>` becomes a text box holding `"\n"` with `white-space: pre`, which
//!   the inline formatter treats as a forced line break.
//! - A block container with any block-level child wraps each run of inline-level
//!   children in an anonymous block (§9.2.1.1). The anonymous block keeps the
//!   parent's *text* properties and none of its box decoration.
//! - Whitespace-only text directly in a block container is dropped. Whitespace
//!   inside an inline box is kept and collapsed at line layout.
//!
//! Recursion over DOM depth is capped at [`MAX_DEPTH`] so a hostile document
//! cannot overflow the stack; deeper content is dropped and counted.

use std::sync::Arc;

use engine_core::{BoxId, EdgeSizes, Length, NodeId, Rgba8, Size};
use engine_css::properties::{BorderStyle, Display, Overflow, Position};
use engine_css::ComputedStyle;
use engine_dom::{Document, ElementData, KnownTag, NodeKind};
use engine_style::StyleTree;

/// The maximum DOM depth box generation follows before giving up.
pub const MAX_DEPTH: u32 = 256;

/// What kind of box this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxKind {
    /// A block-level box: fills its containing block's width, stacks vertically.
    Block,
    /// An inline-level box (`display: inline`), laid out inside a line.
    Inline,
    /// A text run (leaf).
    Text,
    /// An anonymous block wrapping a run of inline-level children (§9.2.1.1).
    Anonymous,
    /// A replaced element (`<img>`, form control) with an intrinsic size.
    Replaced,
    /// A list item; laid out as a block, marker painted at paint time.
    ListItem,
}

impl BoxKind {
    /// True for the boxes the block formatter stacks vertically.
    pub fn is_block_level(self) -> bool {
        matches!(self, BoxKind::Block | BoxKind::Anonymous | BoxKind::ListItem | BoxKind::Replaced)
    }
}

/// Whether an element is block-level under its computed `display`.
///
/// Grid, table, and flex are lowered to block layout here and recorded in the
/// report by [`crate::LayoutTree::layout`]; the box itself is
/// [`BoxKind::Block`].
pub fn is_block_level_display(display: Display) -> bool {
    matches!(
        display,
        Display::Block
            | Display::ListItem
            | Display::Flex
            | Display::Grid
            | Display::Table
            | Display::TableRow
            | Display::TableCell
    )
}

/// A lowered or approximated layout behaviour, recorded so the caller can report
/// it rather than pretending the approximation did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approximation {
    /// `display: grid` laid out as blocks.
    Grid,
    /// A `display: table*` box laid out as blocks.
    Table,
    /// `display: flex` laid out as a block stack.
    Flex,
    /// An absolutely or fixed positioned box, omitted from normal flow.
    OutOfFlow(Position),
    /// DOM nesting deeper than [`MAX_DEPTH`]; the subtree was dropped.
    NestingDepthLimit,
    /// Layout nesting deeper than [`crate::MAX_LAYOUT_DEPTH`].
    LayoutDepthLimit,
    /// Two adjoining empty blocks collapse in an order v1 does not model.
    EmptyBlockCollapse,
}

/// One box in the tree.
#[derive(Clone, Debug)]
pub struct LayoutBox {
    pub kind: BoxKind,
    /// The element that generated this box; [`NodeId::INVALID`] for anonymous
    /// boxes and for the root box, which stands in for the initial containing
    /// block.
    pub node: NodeId,
    /// The computed style used for layout and paint.
    pub style: Arc<ComputedStyle>,
    /// The text carried by a [`BoxKind::Text`] box.
    pub text: Option<String>,
    pub children: Vec<BoxId>,
    /// Intrinsic size of a replaced box in CSS px.
    pub intrinsic: Size,
    /// True if this box establishes a block formatting context (CSS 2.1 §9.4.1):
    /// an `overflow` other than `visible`, or a non-static `position`. A BFC
    /// suppresses margin collapsing between the box and its children.
    pub establishes_bfc: bool,
}

/// The whole box tree, in arena form.
#[derive(Debug, Default)]
pub struct BoxTree {
    pub boxes: Vec<LayoutBox>,
    pub root: Option<BoxId>,
}

impl BoxTree {
    pub fn get(&self, id: BoxId) -> &LayoutBox {
        &self.boxes[id.index()]
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    fn push(&mut self, b: LayoutBox) -> BoxId {
        let id = BoxId(self.boxes.len() as u32);
        self.boxes.push(b);
        id
    }

    fn children_mut(&mut self, id: BoxId) -> &mut Vec<BoxId> {
        &mut self.boxes[id.index()].children
    }
}

/// What box generation produced, for tests and reporting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildStats {
    pub boxes: usize,
    pub anonymous_blocks: usize,
    pub text_boxes: usize,
    pub pruned_subtrees: usize,
    pub out_of_flow: usize,
    /// Lowerings and omissions the caller should report.
    pub approximations: Vec<Approximation>,
}

impl BuildStats {
    fn note(&mut self, a: Approximation) {
        if !self.approximations.contains(&a) {
            self.approximations.push(a);
        }
    }
}

/// Convert the styled document into a box tree rooted at `<body>`.
///
/// Returns an empty tree when the document has no `<body>` or the body has no
/// computed style, which only happens for a document that failed to parse.
pub fn build(doc: &Document, style: &StyleTree) -> (BoxTree, BuildStats) {
    let mut tree = BoxTree::default();
    let mut stats = BuildStats::default();
    {
        let mut builder = Builder { tree: &mut tree, doc, style, stats: &mut stats, depth: 0 };
        builder.build_body();
    }
    stats.boxes = tree.boxes.len();
    (tree, stats)
}

struct Builder<'a> {
    tree: &'a mut BoxTree,
    doc: &'a Document,
    style: &'a StyleTree,
    stats: &'a mut BuildStats,
    depth: u32,
}

impl Builder<'_> {
    fn build_body(&mut self) {
        let Some(body) = self
            .doc
            .ids()
            .find(|id| self.doc.element(*id).is_some_and(|e| e.tag.is(KnownTag::Body)))
        else {
            return;
        };
        let Some(body_style) = self.style.get(body) else {
            return;
        };
        if body_style.display == Display::None {
            return;
        }
        // The root box represents the initial containing block. It carries the
        // body's text style but not the body's margins, padding, border, or
        // background: those belong to the body box, which is its child. The UA
        // sheet's `body { margin: 8px }` depends on that split.
        let mut root_style = body_style.clone();
        root_style.margin = EdgeSizes::ZERO;
        root_style.padding = EdgeSizes::ZERO;
        root_style.border_widths = EdgeSizes::ZERO;
        root_style.border_styles = [BorderStyle::None; 4];
        root_style.background.color = Rgba8::TRANSPARENT;
        root_style.width = Length::Auto;
        root_style.height = Length::Auto;
        root_style.display = Display::Block;
        let root = self.tree.push(LayoutBox {
            kind: BoxKind::Block,
            node: NodeId::INVALID,
            style: Arc::new(root_style),
            text: None,
            children: Vec::new(),
            intrinsic: Size::ZERO,
            establishes_bfc: true,
            approximation: None,
        });
        let body_box = self.generate_element(body, body_style.clone());
        self.tree.children_mut(root).push(body_box);
        self.tree.root = Some(root);
    }

    /// Generate one element's box and, recursively, its children.
    fn generate_element(&mut self, node: NodeId, style: ComputedStyle) -> BoxId {
        let display = style.display;
        let approximation = match display {
            Display::Grid | Display::InlineGrid => Some(Approximation::Grid),
            Display::Table | Display::TableRow | Display::TableCell => Some(Approximation::Table),
            Display::Flex | Display::InlineFlex => Some(Approximation::Flex),
            _ => None,
        };
        let kind = if display == Display::ListItem {
            BoxKind::ListItem
        } else if self.is_replaced(node) {
            BoxKind::Replaced
        } else if is_block_level_display(display) || display == Display::InlineBlock {
            // An inline-block is a block box that participates in a line as an
            // atomic inline; it is wrapped in an anonymous block by its parent.
            BoxKind::Block
        } else {
            BoxKind::Inline
        };
        let establishes_bfc = matches!(kind, BoxKind::Block | BoxKind::ListItem)
            && (style.overflow_x.is_clipping()
                || style.overflow_y.is_clipping()
                || style.position != Position::Static);
        let intrinsic = if kind == BoxKind::Replaced {
            intrinsic_size(self.doc, node)
        } else {
            Size::ZERO
        };
        let id = self.tree.push(LayoutBox {
            kind,
            node,
            style: Arc::new(style),
            text: None,
            children: Vec::new(),
            intrinsic,
            establishes_bfc,
            approximation,
        });
        if kind != BoxKind::Replaced {
            self.build_children(node, id);
        }
        id
    }

    /// Generate `node`'s children into `parent`, inserting anonymous blocks.
    fn build_children(&mut self, node: NodeId, parent: BoxId) {
        if self.depth >= MAX_DEPTH {
            self.stats.pruned_subtrees += 1;
            return;
        }
        let Some(parent_style) = self.style.get(node).cloned() else {
            return;
        };
        let parent_kind = self.tree.boxes[parent.index()].kind;
        let wraps_inline_runs = matches!(parent_kind, BoxKind::Block | BoxKind::ListItem);
        self.depth += 1;
        self.generate_run(node, parent, &parent_style, wraps_inline_runs, &mut None);
        self.depth -= 1;
    }

    /// Generate the children of `node`, appending them to `parent` and
    /// wrapping runs of inline-level boxes in anonymous blocks when
    /// `wraps_inline_runs` is set.
    ///
    /// `open_anon` is the currently open anonymous block, if any. It is threaded
    /// as a parameter so a `display: contents` element can contribute children
    /// to the *same* anonymous run as its siblings.
    fn generate_run(
        &mut self,
        node: NodeId,
        parent: BoxId,
        parent_style: &ComputedStyle,
        wraps_inline_runs: bool,
        open_anon: &mut Option<BoxId>,
    ) {
        let mut child = self.doc.first_child(node);
        while let Some(c) = child {
            let next = self.doc.next_sibling(c);
            match self.doc.kind(c) {
                NodeKind::Element(el) => {
                    let Some(cs) = self.style.get(c) else {
                        child = next;
                        continue;
                    };
                    match cs.display {
                        Display::None => self.stats.pruned_subtrees += 1,
                        Display::Contents => {
                            // No box of its own; its children join this run.
                            let cs = cs.clone();
                            self.generate_run(c, parent, &cs, wraps_inline_runs, open_anon);
                        }
                        _ if matches!(cs.position, Position::Absolute | Position::Fixed) => {
                            // Out of normal flow. There is no positioned-ancestor
                            // search or static-position resolution yet, so the
                            // box is dropped rather than flowed as if static —
                            // flowing it would shift every sibling below it.
                            // Recorded; the caller reports it.
                            self.stats.out_of_flow += 1;
                            self.note(Approximation::OutOfFlow(cs.position));
                        }
                        _ if el.tag.is(KnownTag::Br) => {
                            let id = self.make_br_box(c, cs.clone());
                            self.place_in_run(id, false, parent, wraps_inline_runs, open_anon);
                        }
                        display => {
                            let is_block = is_block_level_display(display)
                                || (self.is_replaced(c) && !display.is_inline_level());
                            let id = self.generate_element(c, cs.clone());
                            self.place_in_run(id, is_block, parent, wraps_inline_runs, open_anon);
                        }
                    }
                }
                NodeKind::Text(t) => {
                    // Whitespace that would not render: dropped directly in a
                    // block container, kept inside an inline box where line
                    // layout collapses it.
                    if is_blank(t) && wraps_inline_runs {
                        child = next;
                        continue;
                    }
                    let id = self.make_text_box(c, t.clone(), parent_style.clone());
                    self.place_in_run(id, false, parent, wraps_inline_runs, open_anon);
                }
                _ => {}
            }
            child = next;
        }
    }

    fn place_in_run(
        &mut self,
        id: BoxId,
        is_block: bool,
        parent: BoxId,
        wraps_inline_runs: bool,
        open_anon: &mut Option<BoxId>,
    ) {
        if !wraps_inline_runs {
            self.tree.children_mut(parent).push(id);
            return;
        }
        if is_block {
            // A block-level child closes the open anonymous run; the block is
            // appended directly. Any later inline content opens a new run.
            *open_anon = None;
            self.tree.children_mut(parent).push(id);
            return;
        }
        let anon = match *open_anon {
            Some(a) => a,
            None => {
                let a = self.make_anonymous(&self.tree.boxes[parent.index()].style.clone());
                self.tree.children_mut(parent).push(a);
                *open_anon = Some(a);
                a
            }
        };
        self.tree.children_mut(anon).push(id);
    }

    fn make_br_box(&mut self, node: NodeId, mut style: ComputedStyle) -> BoxId {
        // `white-space: pre` makes the newline survive collapsing, so the inline
        // formatter sees a mandatory break.
        style.white_space = engine_css::properties::WhiteSpace::Pre;
        self.make_text_box(node, "\n".to_string(), style)
    }

    fn make_text_box(&mut self, node: NodeId, text: String, style: ComputedStyle) -> BoxId {
        self.stats.text_boxes += 1;
        self.tree.push(LayoutBox {
            kind: BoxKind::Text,
            node,
            style: Arc::new(style),
            text: Some(text),
            children: Vec::new(),
            intrinsic: Size::ZERO,
            establishes_bfc: false,
            approximation: None,
        })
    }

    /// An anonymous block: the parent's text properties, none of its box
    /// decoration. It must not shift the layout it exists only to organise.
    fn make_anonymous(&mut self, parent: &ComputedStyle) -> BoxId {
        let mut s = parent.clone();
        s.display = Display::Block;
        s.margin = EdgeSizes::ZERO;
        s.padding = EdgeSizes::ZERO;
        s.border_widths = EdgeSizes::ZERO;
        s.border_styles = [BorderStyle::None; 4];
        s.background.color = Rgba8::TRANSPARENT;
        s.width = Length::Auto;
        s.height = Length::Auto;
        s.min_width = Length::Auto;
        s.max_width = Length::Auto;
        s.overflow_x = Overflow::Visible;
        s.overflow_y = Overflow::Visible;
        self.stats.anonymous_blocks += 1;
        self.tree.push(LayoutBox {
            kind: BoxKind::Anonymous,
            node: NodeId::INVALID,
            style: Arc::new(s),
            text: None,
            children: Vec::new(),
            intrinsic: Size::ZERO,
            establishes_bfc: false,
            approximation: None,
        })
    }

    fn is_replaced(&self, node: NodeId) -> bool {
        self.doc.element(node).is_some_and(|e| is_replaced_tag(&e.tag))
    }
}

/// True for tags with a replaced, intrinsically sized box.
pub fn is_replaced_tag(tag: &engine_dom::Tag) -> bool {
    matches!(
        tag.as_known(),
        Some(
            KnownTag::Img
                | KnownTag::Video
                | KnownTag::Canvas
                | KnownTag::Iframe
                | KnownTag::Input
                | KnownTag::Button
                | KnownTag::Select
                | KnownTag::Textarea
        )
    )
}

/// `true` when `s` contains nothing but HTML whitespace.
pub fn is_blank(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
}

/// The intrinsic size of a replaced element, in CSS px.
///
/// There is no image cache at box-build time, so an `<img>` with no
/// `width`/`height` attribute gets the CSS replaced-element default 300x150
/// (§10.3.2). Form controls get their typical UA sizes.
fn intrinsic_size(doc: &Document, node: NodeId) -> Size {
    let Some(el) = doc.element(node) else { return Size::new(300.0, 150.0) };
    let attr_px = |e: &ElementData, name: &str| -> Option<f32> {
        e.attr(name)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    match el.tag.as_known() {
        Some(KnownTag::Input) => {
            let ty = el.attr("type").unwrap_or("text").to_ascii_lowercase();
            match ty.as_str() {
                "checkbox" | "radio" => Size::new(13.0, 13.0),
                "submit" | "button" | "reset" => Size::new(80.0, 22.0),
                _ => Size::new(170.0, 21.0),
            }
        }
        Some(KnownTag::Button) => Size::new(80.0, 22.0),
        Some(KnownTag::Select) => Size::new(120.0, 22.0),
        Some(KnownTag::Textarea) => Size::new(200.0, 60.0),
        // `<img>`, `<video>`, `<canvas>`, `<iframe>`: a `width`/`height`
        // *attribute* is a presentational hint; honour it when present.
        _ => Size::new(
            attr_px(el, "width").unwrap_or(300.0),
            attr_px(el, "height").unwrap_or(150.0),
        ),
    }
}