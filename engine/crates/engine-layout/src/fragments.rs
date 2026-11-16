//! The laid-out tree: one `Fragment` per box, in absolute coordinates.
//!
//! Layout produces absolute coordinates directly (no per-element transform in
//! v1). A fragment records the four box-model edges explicitly rather than
//! making every consumer re-derive them:
//!
//! ```text
//!   margin box
//!     border box       -> Fragment::rect
//!       padding box    -> Fragment::padding_box
//!         content box  -> Fragment::content
//! ```
//!
//! `rect` is the border box because that is what the rasterizer's `Border`
//! primitive and the background fill take.

use std::sync::Arc;

use engine_core::{BoxId, EdgeSizes, FragmentId, NodeId, Rect, Rgba8, Size};
use engine_css::ComputedStyle;

use crate::box_tree::BoxKind;

/// A resolved set of box-model edges against some containing-block width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolvedEdges {
    pub margin: EdgeSizes,
    pub border: EdgeSizes,
    pub padding: EdgeSizes,
}

impl ResolvedEdges {
    pub fn horizontal_extra(&self) -> f32 {
        self.margin.horizontal() + self.border.horizontal() + self.padding.horizontal()
    }

    pub fn vertical_extra(&self) -> f32 {
        self.margin.vertical() + self.border.vertical() + self.padding.vertical()
    }
}

/// The used margins of a box; left/right may come from `margin: auto` centring,
/// which the style does not record.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UsedMargins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// A positioned run of text inside a block.
#[derive(Clone, Debug)]
pub struct TextFragment {
    /// The concatenated (uncollapsed) source text.
    pub text: String,
    /// One entry per line box, in order.
    pub lines: Vec<LineFragment>,
    /// The width the text wrapped against.
    pub available_width: f32,
    /// The block's style; per-line styles live on `LineFragment::style`.
    pub style: Arc<ComputedStyle>,
}

/// One line box of a text run. All coordinates are absolute.
#[derive(Clone, Debug)]
pub struct LineFragment {
    pub rect: Rect,
    /// Absolute baseline y; glyphs are drawn sitting on it.
    pub baseline: f32,
    /// The whitespace-collapsed text of the line, trimmed.
    pub text: String,
    /// Left edge of the text inside the line box, after `text-align`.
    pub text_x: f32,
    /// The style of the line: font, colour, decoration.
    pub style: Arc<ComputedStyle>,
    pub color: Rgba8,
    pub underline: bool,
    pub line_through: bool,
}

/// One laid-out box.
#[derive(Clone, Debug)]
pub struct Fragment {
    /// The box that generated this fragment.
    pub box_id: BoxId,
    /// The DOM node, for hit testing and paint back-references.
    pub node: NodeId,
    /// Border box, absolute.
    pub rect: Rect,
    /// Content box, absolute.
    pub content: Rect,
    /// Padding box, absolute: what `overflow` clips to.
    pub padding_box: Rect,
    pub margin: UsedMargins,
    pub border: EdgeSizes,
    pub padding: EdgeSizes,
    pub children: Vec<FragmentId>,
    /// Present when this box clips its children.
    pub clip: Option<Rect>,
    /// A text run, when this fragment carries inline content.
    pub text: Option<TextFragment>,
    pub kind: BoxKind,
    /// Intrinsic size for a replaced box.
    pub intrinsic: Size,
    /// The collapsed margin that escapes past the border box's before edge, when
    /// that edge may collapse with the parent. See `block.rs`.
    pub outer_before: Option<f32>,
    /// The collapsed margin that escapes past the after edge.
    pub outer_after: Option<f32>,
}

/// The laid-out document.
#[derive(Default)]
pub struct LayoutTree {
    pub fragments: Vec<Fragment>,
    pub root: Option<FragmentId>,
    /// Total document height: the root's border box bottom, at least the
    /// viewport height.
    pub content_height: f32,
}

impl LayoutTree {
    pub fn get(&self, id: FragmentId) -> &Fragment {
        &self.fragments[id.index()]
    }

    pub(crate) fn push(&mut self, f: Fragment) -> FragmentId {
        let id = FragmentId(self.fragments.len() as u32);
        self.fragments.push(f);
        id
    }

    /// Every fragment in paint order (pre-order, children after their parent).
    pub fn walk(&self) -> Vec<FragmentId> {
        let mut order = Vec::with_capacity(self.fragments.len());
        let Some(root) = self.root else { return order };
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            order.push(id);
            for &c in self.get(id).children.iter().rev() {
                stack.push(c);
            }
        }
        order
    }

    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }
}

/// The size of a fragment's border box.
pub fn border_box_size(f: &Fragment) -> Size {
    Size::new(f.rect.w, f.rect.h)
}