//! A parsed font face: `fontdue::Font` plus the precomputed metrics layout
//! needs (§9.3).

use std::sync::Arc;

use engine_core::FontId;
use fontdue::{Font, FontSettings};

/// Index into `FontContext`'s face vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceId(pub u32);

/// A parses-and-ready font. Cheap to share (`Arc`) because a restyle must not
/// re-parse a TTF.
pub struct FontFace {
    pub id: FontId,
    pub name: String,
    font: Font,
}

impl FontFace {
    /// Parse `bytes`. Returns `None` for an unparsable file rather than
    /// panicking: a corrupt system font should not stop a page rendering.
    pub fn from_bytes(bytes: Vec<u8>, file_name: &str) -> Option<FontFace> {
        // `FontSettings::default()` is `collection_index: 0, scale: 40.0`; the
        // scale is only used by `line_metrics`-style helpers we do not call,
        // every measurement here passes an explicit px size.
        let font = Font::from_bytes(bytes, FontSettings::default()).ok()?;
        let name = font
            .name()
            .map(str::to_string)
            .unwrap_or_else(|| file_name.to_string());
        Some(FontFace { id: FontId(0), name, font })
    }

    pub fn inner(&self) -> &Font {
        &self.font
    }

    pub fn has_glyph(&self, c: char) -> bool {
        self.font.has_glyph(c)
    }

    pub fn glyph_index(&self, c: char) -> u16 {
        self.font.lookup_glyph_index(c)
    }

    /// Advance width of one character at `px`, excluding kerning.
    pub fn advance(&self, c: char, px: f32) -> f32 {
        let m = self.font.metrics_indexed(self.glyph_index(c), px);
        m.advance_width
    }

    /// Kerning adjustment between two adjacent characters at `px`. Returns
    /// `0.0` when the font has no `kern` table (§9.2: "kerning from `kern` if
    /// present, else no kerning").
    pub fn kern(&self, left: char, right: char, px: f32) -> f32 {
        self.font.horizontal_kern(left, right, px).unwrap_or(0.0)
    }

    /// Line metrics at `px`: ascent (positive, above baseline), descent
    /// (negative, below), line gap.
    pub fn line_metrics(&self, px: f32) -> LineMetricsPx {
        match self.font.horizontal_line_metrics(px) {
            Some(m) => LineMetricsPx { ascent: m.ascent, descent: m.descent, line_gap: m.line_gap },
            // A font without `hhea`/`OS/2` is legal but rare; CSS requires a
            // used value, so fall back to the CSS-typical 1.16 em box.
            None => LineMetricsPx { ascent: px * 0.8, descent: -px * 0.2, line_gap: px * 0.16 },
        }
    }

    /// Rasterize one glyph at `px` into a coverage bitmap.
    pub fn rasterize(&self, c: char, px: f32) -> (fontdue::Metrics, Vec<u8>) {
        self.font.rasterize(c, px)
    }

    /// Rasterize by glyph index, used when the glyph came from a fallback face
    /// during shaping.
    pub fn rasterize_indexed(&self, glyph: u16, px: f32) -> (fontdue::Metrics, Vec<u8>) {
        self.font.rasterize_indexed(glyph, px)
    }
}

/// Line metrics in px, with `descent` negative (matching `fontdue`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetricsPx {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl LineMetricsPx {
    /// The "normal" `line-height` box: `ascent - descent + line_gap`.
    pub fn line_height(&self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// A shared handle to a face.
pub type SharedFace = Arc<FontFace>;

pub use crate::font_dir::FaceTable;