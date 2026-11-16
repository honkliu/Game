//! Tree construction: insertion modes, the stack of open elements, and the
//! active-formatting list.
//!
//! The architecture (§3.1) lists `foster.rs`, `implicit_close.rs`, and
//! `adoption.rs` as separate modules. Their logic lives inside
//! `construction.rs` on purpose: the foster-parenting rule is four lines, the
//! implicit-close rules are two lookup tables, and the adoption agency as
//! implemented here is a simplified generic unwind. Splitting them into files
//! would spread one algorithm over four places without a seam that helps.

pub mod construction;
pub mod open_elements;

pub use construction::{build, HtmlApproximation, TreeOutcome};