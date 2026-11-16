//! `engine-core` — types shared by every pipeline stage.
//!
//! No I/O, no OS calls, no global state (§3.1). Anything a second crate would
//! otherwise have to define twice lives here: lengths, geometry, colors, arena
//! ids, the shared error type, and DPI/viewport scalars.

pub mod color;
pub mod error;
pub mod geom;
pub mod ids;
pub mod length;
pub mod units;

pub use color::Rgba8;
pub use error::EngineError;
pub use geom::{CornerRadii, EdgeSizes, Point, Rect, Size, union_all};
pub use ids::{BoxId, FontId, FragmentId, ImageKey, NodeId, SelectorId, StyleId};
pub use length::{CalcExpr, Length, units as css_units};
pub use units::{DEFAULT_FONT_SIZE_PX, EX_FALLBACK_EM, Scale, Viewport};

/// Convenience `Result` alias matching the architecture's stage signatures.
pub type Result<T> = std::result::Result<T, EngineError>;