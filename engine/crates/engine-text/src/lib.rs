//! `engine-text` — system fonts, measurement, line breaking, glyph blitting.
//!
//! **Shaping is not implemented.** Positioning is advance-summed with optional
//! `kern`; there is no GSUB/GPOS, no ligatures, no mark positioning, and no
//! bidi reordering (§9.2). Latin, Cyrillic, Greek, and CJK render close to
//! correct; Arabic, Hebrew, Devanagari, and Thai render visibly wrong.
//! `FontContext::measure` records a warning when it meets such a script, so the
//! render report can name it.
//!
//! Fonts come from `%SystemRoot%\Fonts` (§9.1). A machine without the listed
//! faces still works: `measure` returns zero width and nothing panics — but the
//! render report will show zero-size text, which is the honest outcome.

pub mod context;
pub mod face;
pub mod font_dir;
pub mod line_break;
pub mod script;

pub use context::{BreakOpportunity, FontContext, FontKey, TextMetrics};
pub use face::{FaceTable, FontFace, LineMetricsPx, SharedFace};
pub use font_dir::generic_fallback;