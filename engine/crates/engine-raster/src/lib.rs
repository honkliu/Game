//! `engine-raster` — the deterministic CPU rasterizer.
//!
//! Contract (§2, §10.3): rendering the same display list into the same surface
//! twice, in the same process, produces byte-identical output. No threads, no
//! time, no RNG, no `f64` in the pixel path, no iteration-order dependence.

pub mod draw;
pub mod image;
pub mod list;
pub mod polygon;
pub mod render;
pub mod surface;
pub mod text;

pub use draw::{
    fill_linear_gradient, fill_rect, fill_rounded_rect, stroke_border, stroke_outline, NO_CLIP,
};
pub use image::{blit_image, DecodedImage, ImageStore};
pub use list::{
    item_bounds, Axis, BackgroundRepeat, ControlKind, ControlState, DisplayItem, DisplayList,
    GlyphRun, ObjectFit, ShapedGlyph,
};
pub use render::render;
pub use surface::{Surface, SurfaceStats};
pub use text::GlyphBlitter;

/// Render the same list twice into fresh surfaces and assert byte equality.
/// Used by the golden harness before comparing against a reference PNG so a
/// determinism failure is never misreported as a rendering regression (§13.3).
pub fn verify_determinism(
    width: u32,
    height: u32,
    build: impl Fn() -> DisplayList,
    images: &dyn ImageStore,
    glyphs: &dyn GlyphBlitter,
) -> Result<(), String> {
    let mut a = Surface::new(width, height);
    let mut b = Surface::new(width, height);
    let bounds = a.bounds();
    render(&mut a, &build(), bounds, images, glyphs);
    render(&mut b, &build(), bounds, images, glyphs);
    if a.pixels != b.pixels {
        let diff = a
            .pixels
            .iter()
            .zip(b.pixels.iter())
            .filter(|(x, y)| x != y)
            .count();
        return Err(format!("non-deterministic render: {diff} differing bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod determinism_tests {
    use super::*;
    use crate::draw::NO_CLIP;
    use engine_core::{CornerRadii, EdgeSizes, Rect, Rgba8};

    struct NoImages;
    impl ImageStore for NoImages {
        fn get(&self, _k: engine_core::ImageKey) -> Option<&DecodedImage> {
            None
        }
    }
    struct NoGlyphs;
    impl GlyphBlitter for NoGlyphs {
        fn blit_run(&self, _: &mut Surface, _: f32, _: f32, _: &GlyphRun, _: Rgba8, _: Rect) {}
    }

    fn scene() -> DisplayList {
        let mut dl = DisplayList::new();
        for i in 0u8..7 {
            let f = i as f32;
            // Saturating arithmetic: this is test scene data, and a wrap would
            // silently change what the determinism test renders.
            dl.push(DisplayItem::FillRect {
                rect: Rect::new(f * 13.37, f * 7.77, 41.9, 23.3),
                color: Rgba8::rgba(
                    30 + i * 20,
                    200 - i * 10,
                    90,
                    150 + i.saturating_mul(10),
                ),
                radii: CornerRadii::uniform(f * 1.7),
            });
            dl.push(DisplayItem::Border {
                rect: Rect::new(f * 13.37, f * 7.77, 41.9, 23.3),
                widths: EdgeSizes::new(f * 0.5, 1.0, f * 0.25, 2.0),
                colors: [
                    Rgba8::BLACK,
                    Rgba8::rgb(255, 0, 0),
                    Rgba8::rgb(0, 255, 0),
                    Rgba8::rgb(0, 0, 255),
                ],
            });
        }
        dl.push(DisplayItem::LinearGradient {
            rect: Rect::new(5.0, 5.0, 200.0, 100.0),
            from: Rgba8::rgb(10, 20, 30),
            to: Rgba8::rgb(200, 210, 220),
            angle_deg: 33.0,
        });
        let _ = NO_CLIP;
        dl
    }

    #[test]
    fn two_renders_of_the_same_list_are_byte_identical() {
        verify_determinism(320, 240, scene, &NoImages, &NoGlyphs).expect("deterministic");
    }

    #[test]
    fn determinism_helper_compares_bytes() {
        let mut s1 = Surface::new(2, 2);
        let mut s2 = Surface::new(2, 2);
        s1.set(0, 0, Rgba8::BLACK);
        s2.set(0, 1, Rgba8::BLACK);
        assert_ne!(s1.pixels, s2.pixels);
    }
}