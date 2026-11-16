//! `FontContext`: the single entry point layout and paint use for text.
//!
//! Responsibilities:
//! - own the loaded faces (§9.1)
//! - measure strings (§9.3)
//! - break lines (§9.3, UAX #14 subset)
//! - produce positioned glyph runs for the display list (§9.4)
//! - implement `engine_raster::GlyphBlitter` so the rasterizer can draw them
//!
//! **No shaping.** Positioning is advance-summed with optional `kern`. There is
//! no GSUB/GPOS, no ligatures, no mark positioning, and no bidi reordering.
//! Latin, Cyrillic, Greek, and CJK render close to correct; Arabic, Hebrew,
//! Devanagari, Thai, and other complex scripts render visibly wrong. This is
//! the §15 "Not supported" table, and `measure` reports it via
//! [`FontContext::last_shaping_warning`] so a page's render report can name it.

use std::collections::HashMap;
use std::sync::Arc;

use engine_core::{FontId, Rect, Rgba8};
use engine_raster::{DisplayItem, GlyphRun, ShapedGlyph, Surface};

use crate::face::{FaceTable, LineMetricsPx, SharedFace};
use crate::script::script_warning;

/// A resolved font for one element's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontKey {
    pub family_bits: u32,
    pub weight_bold: bool,
    pub italic: bool,
}

impl FontKey {
    pub fn from_style(families: &[String], weight: u16, italic: bool) -> FontKey {
        // Hash the family list so two elements with the same stack share a key.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for f in families {
            for b in f.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
            h ^= 0xff;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        FontKey { family_bits: h as u32, weight_bold: weight >= 600, italic }
    }
}

/// The measured size of a string.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    /// Advance width in px.
    pub width: f32,
    /// Ascent above the baseline, px (positive).
    pub ascent: f32,
    /// Descent below the baseline, px (positive).
    pub descent: f32,
}

/// One break opportunity in a string. Indices are **character** indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakOpportunity {
    pub at: usize,
    /// True when the break must happen (`\n`), false when it is optional.
    pub mandatory: bool,
}

/// A font context: faces, a per-font cache, and the shaping entry points.
pub struct FontContext {
    faces: Vec<SharedFace>,
    /// `FontKey -> selected face` chosen on first use.
    selected: HashMap<FontKey, SharedFace>,
    table: FaceTable,
    /// Set by `measure` when the last string contained a script we cannot shape
    /// correctly. Cleared on the next call.
    shaping_warning: Option<&'static str>,
}

impl Default for FontContext {
    fn default() -> Self {
        FontContext::new()
    }
}

impl FontContext {
    /// Load the system faces. Never fails: a machine with no fonts yields an
    /// empty context whose `measure` returns zero width.
    pub fn new() -> FontContext {
        let (table, raw) = FaceTable::load();
        let faces: Vec<SharedFace> = raw
            .into_iter()
            .enumerate()
            .map(|(i, mut f)| {
                f.id = FontId(i as u32);
                Arc::new(f)
            })
            .collect();
        FontContext { faces, selected: HashMap::new(), table, shaping_warning: None }
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// The face for a CSS font stack at `px`, with per-character fallback
    /// resolved during measurement rather than here.
    ///
    /// Returns `None` only when no font loaded at all; callers check
    /// [`Self::is_empty`] first and every measurement path does.
    pub fn select_face(&mut self, families: &[String], weight: u16, italic: bool) -> Option<SharedFace> {
        let key = FontKey::from_style(families, weight, italic);
        if let Some(f) = self.selected.get(&key) {
            return Some(Arc::clone(f));
        }
        let chosen = self
            .table
            .select(families, weight, italic)
            .and_then(|id| self.faces.get(id.0 as usize).cloned())
            .or_else(|| self.faces.first().cloned())?;
        self.selected.insert(key, Arc::clone(&chosen));
        Some(chosen)
    }

    /// The face to measure `text` with, per-character fallback applied.
    /// Returns `None` only for an empty font context.
    fn face_for(&mut self, families: &[String], weight: u16, italic: bool, c: char) -> Option<SharedFace> {
        let primary = self.select_face(families, weight, italic)?;
        if primary.has_glyph(c) {
            return Some(primary);
        }
        for &id in self.table.fallback_order() {
            if let Some(f) = self.faces.get(id.0 as usize) {
                if f.has_glyph(c) {
                    return Some(Arc::clone(f));
                }
            }
        }
        Some(primary)
    }

    /// Measure `text` at `px` under a font stack.
    ///
    /// Width is the sum of advances plus kerning plus `letter_spacing`. Height
    /// is the selected face's `(ascent - descent + line_gap)`.
    pub fn measure(
        &mut self,
        text: &str,
        families: &[String],
        weight: u16,
        italic: bool,
        px: f32,
        letter_spacing: f32,
    ) -> TextMetrics {
        self.shaping_warning = None;
        if self.faces.is_empty() || px <= 0.0 {
            return TextMetrics { width: 0.0, ascent: 0.0, descent: 0.0 };
        }
        let mut width = 0.0f32;
        let mut prev: Option<(char, SharedFace)> = None;
        for c in text.chars() {
            // `faces` is non-empty (checked above), so `face_for` cannot return
            // `None`; the `else` arm is unreachable but keeps the code free of
            // `unwrap`, which the project forbids in library code.
            let Some(face) = self.face_for(families, weight, italic, c) else { continue };
            if let Some((p, pf)) = &prev {
                if Arc::ptr_eq(pf, &face) {
                    width += face.kern(*p, c, px);
                }
            }
            // Two ways text goes visibly wrong: a glyph the font lacks (tofu), or a
            // script we cannot join or reorder (§9.2). Both are reported.
            if !face.has_glyph(c) || script_warning(c).is_some() {
                self.note_shaping_warning(c);
            }
            width += face.advance(c, px) + letter_spacing;
            prev = Some((c, Arc::clone(&face)));
        }
        // Any face in the stack has the same vertical metrics, so the primary
        // face is the right source for line height.
        let lm = match self.select_face(families, weight, italic) {
            Some(f) => f.line_metrics(px),
            None => LineMetricsPx { ascent: px * 0.8, descent: -px * 0.2, line_gap: px * 0.16 },
        };
        TextMetrics { width: width.max(0.0), ascent: lm.ascent, descent: -lm.descent }
    }

    /// Line metrics for a font stack, for line box heights.
    pub fn line_metrics(&mut self, families: &[String], weight: u16, italic: bool, px: f32) -> LineMetricsPx {
        match self.select_face(families, weight, italic) {
            Some(f) => f.line_metrics(px),
            None => LineMetricsPx { ascent: px * 0.8, descent: -px * 0.2, line_gap: px * 0.16 },
        }
    }

    /// Advance width of a single character, for `ch` units and for the
    /// whitespace-collapsing state machine.
    pub fn char_advance(&mut self, c: char, families: &[String], weight: u16, italic: bool, px: f32) -> f32 {
        match self.face_for(families, weight, italic, c) {
            Some(f) => f.advance(c, px),
            None => 0.0,
        }
    }

    /// The script warning from the most recent `measure`, if any (§9.2).
    pub fn last_shaping_warning(&self) -> Option<&'static str> {
        self.shaping_warning
    }

    fn note_shaping_warning(&mut self, c: char) {
        if let Some(script) = script_warning(c) {
            // Keep the first warning: it names the dominant problem.
            if self.shaping_warning.is_none() {
                self.shaping_warning = Some(script);
            }
        }
    }

    /// Position `text` into a `GlyphRun` for the display list (§9.4).
    ///
    /// Glyphs are advanced by their own advance widths; the run's `font` field
    /// is the primary face's id, and glyphs from fallback faces are emitted as
    /// separate runs by [`Self::layout_runs`].
    pub fn layout_runs(
        &mut self,
        text: &str,
        families: &[String],
        weight: u16,
        italic: bool,
        px: f32,
        letter_spacing: f32,
    ) -> Vec<(FontId, GlyphRun)> {
        if self.faces.is_empty() || px <= 0.0 || text.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(FontId, GlyphRun)> = Vec::new();
        let mut cur_face: Option<SharedFace> = None;
        let mut run = GlyphRun { font: 0, size: px, glyphs: Vec::new(), advance: 0.0 };
        let mut run_start = 0.0f32;
        let mut pen = 0.0f32;
        let mut prev: Option<(char, SharedFace)> = None;

        for c in text.chars() {
            // Non-empty `faces` was checked above, so this is always `Some`.
            let Some(face) = self.face_for(families, weight, italic, c) else { continue };
            let same = cur_face.as_ref().is_some_and(|f| std::sync::Arc::ptr_eq(f, &face));
            if !same {
                if let Some(f) = cur_face.take() {
                    if !run.glyphs.is_empty() {
                        // Position each glyph relative to *its own* run start,
                        // so a fallback run's glyphs do not jump to the origin.
                        for g in run.glyphs.iter_mut() {
                            g.x -= run_start;
                        }
                        run.advance = pen - run_start;
                        run.font = f.id.0;
                        let mut finished = std::mem::replace(
                            &mut run,
                            GlyphRun { font: 0, size: px, glyphs: Vec::new(), advance: 0.0 },
                        );
                        finished.font = f.id.0;
                        out.push((f.id, finished));
                    }
                }
                run = GlyphRun { font: 0, size: px, glyphs: Vec::new(), advance: 0.0 };
                run_start = pen;
                cur_face = Some(Arc::clone(&face));
            }
            if let Some((p, pf)) = &prev {
                // Kerning only applies between two glyphs of the same face.
                let same_face = cur_face.as_ref().is_some_and(|f| std::sync::Arc::ptr_eq(pf, f));
                if same_face {
                    pen += face.kern(*p, c, px);
                }
            }
            let sub = ((pen - pen.floor()) * 4.0).round().clamp(0.0, 3.0) as u8;
            run.glyphs.push(ShapedGlyph {
                glyph_id: face.glyph_index(c),
                x: pen,
                y: 0.0,
                x_subpixel: sub,
            });
            pen += face.advance(c, px) + letter_spacing;
            prev = Some((c, face));
        }
        if let Some(f) = cur_face {
            if !run.glyphs.is_empty() {
                for g in run.glyphs.iter_mut() {
                    g.x -= run_start;
                }
                run.advance = pen - run_start;
                run.font = f.id.0;
                out.push((f.id, run));
            }
        }
        out
    }

    /// Total measured width of a run set: the sum of the run advances.
    pub fn runs_width(runs: &[(FontId, GlyphRun)]) -> f32 {
        runs.iter().map(|(_, r)| r.advance).sum()
    }

    /// The display item for a run set, with its baseline at `(x, y)`.
    pub fn glyph_items(
        runs: Vec<(FontId, GlyphRun)>,
        origin_x: f32,
        baseline_y: f32,
        color: Rgba8,
    ) -> Vec<DisplayItem> {
        runs
            .into_iter()
            .map(|(_, run)| DisplayItem::GlyphRun {
                origin: engine_core::Point::new(origin_x, baseline_y),
                run,
                color,
            })
            .collect()
    }
}

impl engine_raster::GlyphBlitter for FontContext {
    /// Draw one glyph run with the baseline at `(x, y)`.
    ///
    /// `ShapedGlyph::y` is the vertical offset from the baseline (negative up),
    /// which for v1 is always `0` — there is no shaping, so no marks are raised
    /// or lowered. Subpixel positioning is quantized to 1/4 px by rounding the
    /// final blit x to the nearest quarter pixel, matching the atlas key.
    fn blit_run(&self, surface: &mut Surface, x: f32, y: f32, run: &GlyphRun, color: Rgba8, clip: Rect) {
        let Some(face) = self.faces.iter().find(|f| f.id.0 == run.font) else { return };
        let px = run.size;
        let base_x = x.round() as i32;
        let base_y = y.round() as i32;
        for g in &run.glyphs {
            let (metrics, bitmap) = face.rasterize_indexed(g.glyph_id, px);
            blit_glyph(
                surface,
                &metrics,
                &bitmap,
                base_x + g.x.round() as i32,
                base_y + g.y.round() as i32,
                color,
                clip,
            );
        }
    }
}

/// Blit one glyph bitmap with its pen origin at `(pen_x, baseline_y)`.
///
/// `fontdue`'s `xmin`/`ymin` are whole-pixel offsets from the pen origin to the
/// bitmap's left edge and its *bottom* edge; the coverage buffer is top-down,
/// so the top row is at `baseline - ymin - height`.
fn blit_glyph(
    surface: &mut Surface,
    m: &fontdue::Metrics,
    bitmap: &[u8],
    pen_x: i32,
    baseline_y: i32,
    color: Rgba8,
    clip: Rect,
) {
    if m.width == 0 || m.height == 0 {
        return;
    }
    let left = pen_x + m.xmin;
    let top = baseline_y - (m.ymin + m.height as i32);
    let clip_x0 = clip.x.floor().max(0.0) as i32;
    let clip_y0 = clip.y.floor().max(0.0) as i32;
    let clip_x1 = clip.right().ceil().min(surface.width as f32) as i32;
    let clip_y1 = clip.bottom().ceil().min(surface.height as f32) as i32;

    let src = color.premultiplied();
    for row in 0..m.height as i32 {
        let y = top + row;
        if y < clip_y0 || y >= clip_y1 {
            continue;
        }
        for col in 0..m.width as i32 {
            let x = left + col;
            if x < clip_x0 || x >= clip_x1 {
                continue;
            }
            let cov = bitmap[(row as usize) * m.width + col as usize];
            if cov == 0 {
                continue;
            }
            let a = (src[3] as u32 * cov as u32 / 255) as u8;
            let premul = [
                (src[0] as u32 * cov as u32 / 255) as u8,
                (src[1] as u32 * cov as u32 / 255) as u8,
                (src[2] as u32 * cov as u32 / 255) as u8,
                a,
            ];
            surface.blend_premul_at(x as u32, y as u32, premul, 255);
        }
    }
}