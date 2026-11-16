//! Text blitting interface.
//!
//! The text stage (`engine-text`) owns font parsing and glyph rasterization; the
//! raster crate only needs to call back into it while walking a glyph run. The
//! trait has exactly one method — the rule of two says do not abstract until
//! there is a second implementation, and the second implementation here is the
//! test double used by `render.rs`'s unit test, plus a future GPU path. If that
//! double ever disappears, inline this.

use engine_core::{Rect, Rgba8};

use crate::list::GlyphRun;
use crate::surface::Surface;

/// Lays glyph bitmaps into a surface. Implemented by `engine-text::FontContext`.
pub trait GlyphBlitter {
    /// Draw `run` with its baseline at `(x, y)`.
    fn blit_run(
        &self,
        surface: &mut Surface,
        x: f32,
        y: f32,
        run: &GlyphRun,
        color: Rgba8,
        clip: Rect,
    );
}