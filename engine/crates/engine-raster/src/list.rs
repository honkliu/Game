//! The display list.
//!
//! **Deviation from ARCHITECTURE.md §3.1 (recorded):** the document puts
//! `DisplayItem`/`DisplayList` in `engine-paint` and says `engine-raster`
//! renders it. That is a dependency cycle — `engine-paint` already depends on
//! `engine-text`, which depends on `engine-raster` for glyph blitting. The type
//! therefore lives here, in `engine-raster`, and `engine-paint::build` produces
//! it. The shapes and variants are exactly §8.1's; only the crate that owns the
//! struct moved.

use engine_core::{CornerRadii, EdgeSizes, Point, Rect, Rgba8};

/// Identifies a decoded image in the paint stage's `ImageCache`.
pub type ImageKey = engine_core::ImageKey;

/// One glyph after positioning. `advance` is the pen movement in device px.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// Glyph id from the font's `cmap`.
    pub glyph_id: u16,
    /// Horizontal offset from the run origin, device px.
    pub x: f32,
    /// Vertical offset from the baseline, device px (negative = up).
    pub y: f32,
    /// Sub-pixel horizontal offset quantized to 1/4 px (§9.4).
    pub x_subpixel: u8,
}

/// A run of glyphs in one font at one size.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    /// Opaque font identity: `(FontId, size_quarter_px)` packed by the text stage.
    pub font: u32,
    /// Font size in device px.
    pub size: f32,
    pub glyphs: Vec<ShapedGlyph>,
    /// Total advance of the run, device px.
    pub advance: f32,
}

/// How an image is fitted into its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundRepeat {
    NoRepeat,
    Repeat,
    RepeatX,
    RepeatY,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Button,
    TextInput,
    Checkbox,
    Radio,
    Submit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlState {
    Normal,
    Hover,
    Active,
    Focused,
    Checked,
    Disabled,
}

/// The paint primitives. Order is painter's order; clips are a stack.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    FillRect {
        rect: Rect,
        color: Rgba8,
        radii: CornerRadii,
    },
    Border {
        rect: Rect,
        widths: EdgeSizes,
        colors: [Rgba8; 4],
    },
    Outline {
        rect: Rect,
        color: Rgba8,
        width: f32,
    },
    Image {
        dst: Rect,
        src: ImageKey,
        fit: ObjectFit,
        opacity: f32,
        repeat: BackgroundRepeat,
    },
    LinearGradient {
        rect: Rect,
        from: Rgba8,
        to: Rgba8,
        angle_deg: f32,
    },
    GlyphRun {
        origin: Point,
        run: GlyphRun,
        color: Rgba8,
    },
    TextCursor {
        rect: Rect,
        color: Rgba8,
    },
    Scrollbar {
        track: Rect,
        thumb: Rect,
        axis: Axis,
    },
    Control {
        rect: Rect,
        kind: ControlKind,
        state: ControlState,
        radius: f32,
    },
    PushClip {
        rect: Rect,
    },
    PopClip,
}

/// A flat, ordered list of paint primitives with the box that produced each one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub items: Vec<(DisplayItem, Option<engine_core::BoxId>)>,
    /// Union of everything the list draws.
    pub bounds: Rect,
}

impl DisplayList {
    pub fn new() -> Self {
        DisplayList::default()
    }

    pub fn push(&mut self, item: DisplayItem) {
        self.push_from(item, None);
    }

    pub fn push_from(&mut self, item: DisplayItem, box_id: Option<engine_core::BoxId>) {
        if let Some(b) = item_bounds(&item) {
            self.bounds = self.bounds.union(&b);
        }
        self.items.push((item, box_id));
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Bounding box of an item, used for damage tracking and surface sizing.
pub fn item_bounds(item: &DisplayItem) -> Option<Rect> {
    match item {
        DisplayItem::FillRect { rect, .. }
        | DisplayItem::Border { rect, .. }
        | DisplayItem::LinearGradient { rect, .. }
        | DisplayItem::Control { rect, .. }
        | DisplayItem::PushClip { rect } => Some(*rect),
        DisplayItem::Outline { rect, width, .. } => {
            Some(rect.inflate(EdgeSizes::uniform(*width)))
        }
        DisplayItem::Image { dst, .. } => Some(*dst),
        DisplayItem::GlyphRun { origin, run, .. } => Some(Rect::new(
            origin.x,
            origin.y - run.size,
            run.advance,
            run.size * 1.5,
        )),
        DisplayItem::TextCursor { rect, .. } => Some(*rect),
        DisplayItem::Scrollbar { track, .. } => Some(*track),
        DisplayItem::PopClip => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_accumulate() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(0.0, 0.0, 2.0, 2.0),
            color: Rgba8::BLACK,
            radii: CornerRadii::ZERO,
        });
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(5.0, 5.0, 2.0, 2.0),
            color: Rgba8::BLACK,
            radii: CornerRadii::ZERO,
        });
        assert_eq!(dl.bounds, Rect::new(0.0, 0.0, 7.0, 7.0));
        assert_eq!(dl.len(), 2);
    }
}