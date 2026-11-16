//! Unit-resolution context: DPI scaling and the root-font-size rule.
//!
//! `engine-core` stays I/O-free, but unit *semantics* (what `rem` means, what
//! viewport-relative units resolve against) belong with the types that carry
//! them. The actual `em`/`rem` → px folding happens in `engine-style`; this
//! module holds the scale factor and provides the helper arithmetic so both
//! crates agree on rounding.

/// Device pixel ratio. CSS px × `dpr` = device px (§3.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(pub f32);

impl Default for Scale {
    fn default() -> Self {
        Scale(1.0)
    }
}

impl Scale {
    pub fn css_to_device(&self, css_px: f32) -> f32 {
        css_px * self.0
    }

    pub fn device_to_css(&self, device_px: f32) -> f32 {
        device_px / self.0
    }
}

/// Viewport in CSS pixels plus DPR, shared by media queries and layout (§4 Stage 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width_css: f32,
    pub height_css: f32,
    pub dpr: f32,
}

impl Viewport {
    pub fn new(width_css: f32, height_css: f32, dpr: f32) -> Self {
        Viewport { width_css, height_css, dpr }
    }

    /// Device-pixel dimensions of the drawable surface.
    pub fn device_size(&self) -> (u32, u32) {
        let w = (self.width_css * self.dpr).round().max(1.0);
        let h = (self.height_css * self.dpr).round().max(1.0);
        (w as u32, h as u32)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { width_css: 800.0, height_css: 600.0, dpr: 1.0 }
    }
}

/// The CSS default font size, used for `medium` and the initial `font-size`.
pub const DEFAULT_FONT_SIZE_PX: f32 = 16.0;

/// `ex` and `ch` fallbacks when real font metrics are unavailable (§5.6).
pub const EX_FALLBACK_EM: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_converts_both_ways() {
        let s = Scale(1.5);
        assert_eq!(s.css_to_device(10.0), 15.0);
        assert_eq!(s.device_to_css(15.0), 10.0);
    }

    #[test]
    fn viewport_device_size_rounds() {
        let vp = Viewport::new(800.0, 600.0, 1.25);
        assert_eq!(vp.device_size(), (1000, 750));
    }

    #[test]
    fn viewport_device_size_never_zero() {
        let vp = Viewport::new(0.0, 0.0, 1.0);
        assert_eq!(vp.device_size(), (1, 1));
    }
}