//! `engine-style` — cascade, inheritance, and the computed-style tree.
//!
//! The stage between selector matching and layout: take the DOM, the sheets,
//! and a viewport, and produce one `ComputedStyle` per element.
//!
//! Deviations from ARCHITECTURE.md, recorded:
//! - `var()` substitution (`custom_props.rs`) is **not implemented**. The CSS
//!   parser does not emit custom-property declarations at all (it rejects
//!   `--x` as an unsupported property), so there is nothing to substitute. A
//!   `color: var(--c)` declaration is dropped by the parser and appears in
//!   `css_errors`. This is a larger gap than the architecture implies: pages
//!   that theme themselves with custom properties will lose those colours.
//! - `@import` is not resolved here (`stylesheet.imports` is collected but the
//!   fetch layer that would resolve it is not part of this stage).
//! - Inline `style` attributes are parsed with a default 16px length context, so
//!   `em` inside an inline style resolves against 16px rather than the
//!   element's own size. Recorded in `StyleReport::unsupported_declarations`.

pub mod apply;
pub mod cascade;
pub mod media_eval;

pub use apply::{apply, ApplyResult, Side};
pub use cascade::{
    compute_styles, compute_styles_with_media, is_inherited, StyleReport, StyleTree, ALL_LONGHANDS,
};
pub use media_eval::{all_hold, eval as eval_media, MediaContext};