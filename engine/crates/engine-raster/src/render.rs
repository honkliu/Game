//! `render(surface, list, clip)` — walks the display list in painter's order
//! and applies each item through the primitive draws, maintaining a clip stack.

use engine_core::{Rect, Rgba8};

use crate::draw;
use crate::image::ImageStore;
use crate::list::{DisplayItem, DisplayList};
use crate::surface::Surface;
use crate::text::GlyphBlitter;

/// Draw `list` into `surface`, restricted to `clip`.
///
/// `images` and `glyphs` are the decoded/glyph-raster sources; both are trait
/// objects because the paint and text stages sit above this crate.
pub fn render(
    surface: &mut Surface,
    list: &DisplayList,
    clip: Rect,
    images: &dyn ImageStore,
    glyphs: &dyn GlyphBlitter,
) {
    let root_clip = clip.intersect(&surface.bounds());
    let mut stack: Vec<Rect> = vec![root_clip];
    let mut current = root_clip;
    let mut pop_guard = 0usize;

    for (item, _) in &list.items {
        match item {
            DisplayItem::PushClip { rect } => {
                stack.push(current);
                current = current.intersect(rect);
                pop_guard += 1;
            }
            DisplayItem::PopClip => {
                if pop_guard > 0 {
                    current = stack.pop().unwrap_or(root_clip);
                    pop_guard -= 1;
                }
            }
            DisplayItem::FillRect { rect, color, radii } => {
                if radii.is_zero() {
                    draw::fill_rect(surface, *rect, *color, current);
                } else {
                    draw::fill_rounded_rect(surface, *rect, *radii, *color, current);
                }
            }
            DisplayItem::Border { rect, widths, colors } => {
                draw::stroke_border(surface, *rect, *widths, *colors, current);
            }
            DisplayItem::Outline { rect, color, width } => {
                draw::stroke_outline(surface, *rect, *color, *width, current);
            }
            DisplayItem::LinearGradient { rect, from, to, angle_deg } => {
                draw::fill_linear_gradient(surface, *rect, *from, *to, *angle_deg, current);
            }
            DisplayItem::Image { dst, src, fit, opacity, repeat } => {
                if let Some(img) = images.get(*src) {
                    crate::image::blit_image(surface, img, *dst, *fit, *opacity, *repeat, current);
                }
            }
            DisplayItem::GlyphRun { origin, run, color } => {
                glyphs.blit_run(surface, origin.x, origin.y, run, *color, current);
            }
            DisplayItem::TextCursor { rect, color } => {
                draw::fill_rect(surface, *rect, *color, current);
            }
            DisplayItem::Scrollbar { track, thumb, axis } => {
                draw::fill_scrollbar(surface, *track, *thumb, *axis, current);
            }
            DisplayItem::Control { rect, kind, state, radius } => {
                draw::paint_control(surface, *rect, *kind, *state, *radius, current);
            }
        }
    }

    // Unbalanced PushClips are tolerated (the list is ours) but never leak a
    // clip into the next frame; `current` is a local.
    let _ = current;
    let _: Rgba8 = Rgba8::TRANSPARENT;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{DecodedImage, ImageStore};
    use crate::list::{DisplayItem, GlyphRun};
    use crate::text::GlyphBlitter;
    use engine_core::{CornerRadii, EdgeSizes};

    struct NoImages;
    impl ImageStore for NoImages {
        fn get(&self, _k: engine_core::ImageKey) -> Option<&DecodedImage> {
            None
        }
    }
    struct NoGlyphs;
    impl GlyphBlitter for NoGlyphs {
        fn blit_run(
            &self,
            _s: &mut Surface,
            _x: f32,
            _y: f32,
            _run: &GlyphRun,
            _c: Rgba8,
            _clip: Rect,
        ) {
        }
    }

    #[test]
    fn renders_items_in_order() {
        let mut s = Surface::new(10, 10);
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Rgba8::rgb(255, 0, 0),
            radii: CornerRadii::ZERO,
        });
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(2.0, 2.0, 4.0, 4.0),
            color: Rgba8::rgb(0, 0, 255),
            radii: CornerRadii::ZERO,
        });
        render(&mut s, &dl, Rect::new(0.0, 0.0, 10.0, 10.0), &NoImages, &NoGlyphs);
        assert_eq!(s.get(0, 0), Rgba8::rgb(255, 0, 0), "first item painted first");
        assert_eq!(s.get(3, 3), Rgba8::rgb(0, 0, 255), "second item overdraws");
    }

    #[test]
    fn clip_stack_restricts_then_restores() {
        let mut s = Surface::new(10, 10);
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushClip { rect: Rect::new(0.0, 0.0, 4.0, 4.0) });
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Rgba8::BLACK,
            radii: CornerRadii::ZERO,
        });
        dl.push(DisplayItem::PopClip);
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(6.0, 6.0, 2.0, 2.0),
            color: Rgba8::rgb(0, 255, 0),
            radii: CornerRadii::ZERO,
        });
        render(&mut s, &dl, Rect::new(0.0, 0.0, 10.0, 10.0), &NoImages, &NoGlyphs);
        assert_eq!(s.get(1, 1), Rgba8::BLACK, "inside clip");
        assert_eq!(s.get(5, 5), Rgba8::TRANSPARENT, "clipped away");
        assert_eq!(s.get(6, 6), Rgba8::rgb(0, 255, 0), "after PopClip, unclipped");
    }

    #[test]
    fn render_is_byte_deterministic() {
        let build = || {
            let mut dl = DisplayList::new();
            dl.push(DisplayItem::FillRect {
                rect: Rect::new(1.3, 2.7, 20.4, 15.1),
                color: Rgba8::rgba(40, 120, 220, 200),
                radii: CornerRadii::uniform(3.5),
            });
            dl.push(DisplayItem::Border {
                rect: Rect::new(1.3, 2.7, 20.4, 15.1),
                widths: EdgeSizes::uniform(1.0),
                colors: [Rgba8::BLACK; 4],
            });
            dl
        };
        let mut a = Surface::new(32, 24);
        let mut b = Surface::new(32, 24);
        let bounds = Rect::new(0.0, 0.0, 32.0, 24.0);
        render(&mut a, &build(), bounds, &NoImages, &NoGlyphs);
        render(&mut b, &build(), bounds, &NoImages, &NoGlyphs);
        assert_eq!(a.pixels, b.pixels);
    }

    #[test]
    fn outer_clip_bounds_everything() {
        let mut s = Surface::new(10, 10);
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Rgba8::BLACK,
            radii: CornerRadii::ZERO,
        });
        render(&mut s, &dl, Rect::new(0.0, 0.0, 3.0, 3.0), &NoImages, &NoGlyphs);
        assert_eq!(s.get(2, 2), Rgba8::BLACK);
        assert_eq!(s.get(3, 3), Rgba8::TRANSPARENT);
    }
}