//! Drawing primitives on a `Surface`.
//!
//! Everything here is a pure function of its inputs: same arguments → same
//! bytes. There is no threading, no time, no RNG, and no floating-point
//! reassociation (§3.1 `determinism.rs`).

use engine_core::{CornerRadii, EdgeSizes, Rect, Rgba8};

use crate::list::{Axis, ControlKind, ControlState};
use crate::polygon::Polygon;
use crate::surface::Surface;

/// A clip that permits every pixel. Shared by tests and callers that do not clip.
pub const NO_CLIP: Rect = Rect { x: -100000.0, y: -100000.0, w: 200000.0, h: 200000.0 };

/// Paint an RGBA color into a coverage buffer at `origin`.
fn composite(surface: &mut Surface, origin: (i32, i32), w: u32, h: u32, cov: &[u8], color: Rgba8) {
    let [r, g, b, a] = color.premultiplied();
    let src = [r, g, b, a];
    for row in 0..h {
        let sy = origin.1 + row as i32;
        if sy < 0 || sy >= surface.height as i32 {
            continue;
        }
        for col in 0..w {
            let c = cov[(row * w + col) as usize];
            if c == 0 {
                continue;
            }
            let sx = origin.0 + col as i32;
            if sx < 0 || sx >= surface.width as i32 {
                continue;
            }
            surface.blend_premul_at(sx as u32, sy as u32, src, c);
        }
    }
}

/// Fill an axis-aligned rectangle. `clip` restricts writes.
///
/// Integer-aligned rects take a direct write path that is byte-exact; anything
/// fractional goes through the coverage rasterizer so edges are anti-aliased.
pub fn fill_rect(surface: &mut Surface, rect: Rect, color: Rgba8, clip: Rect) {
    if color.a == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let r = rect.intersect(&clip);
    let integral = rect.x.fract() == 0.0
        && rect.y.fract() == 0.0
        && rect.w.fract() == 0.0
        && rect.h.fract() == 0.0;
    if integral {
        if let Some((x0, y0, x1, y1)) = surface.clip_to_bounds(r) {
            let [pr, pg, pb, pa] = color.premultiplied();
            let src = [pr, pg, pb, pa];
            for y in y0..y1 {
                for x in x0..x1 {
                    surface.blend_premul_at(x, y, src, 255);
                }
            }
            return;
        }
        return;
    }
    let mut poly = Polygon::new();
    poly.rect(rect);
    fill_polygon(surface, &poly, color, clip);
}

/// Fill a rectangle whose corners may be rounded. `clip` restricts writes.
pub fn fill_rounded_rect(
    surface: &mut Surface,
    rect: Rect,
    radii: CornerRadii,
    color: Rgba8,
    clip: Rect,
) {
    if color.a == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    if radii.is_zero() {
        fill_rect(surface, rect, color, clip);
        return;
    }
    let mut poly = Polygon::new();
    poly.rounded_rect(rect, radii);
    fill_polygon(surface, &poly, color, clip);
}

/// Rasterize a polygon through coverage and composite it.
fn fill_polygon(surface: &mut Surface, poly: &Polygon, color: Rgba8, clip: Rect) {
    let target = poly_bounds(poly).intersect(&clip).intersect(&surface.bounds());
    if target.is_empty() {
        return;
    }
    // Pad by one pixel so anti-aliased edges at the target boundary are included.
    let padded = target.inflate(EdgeSizes::uniform(1.0)).intersect(&surface.bounds());
    let Some((x0, y0, x1, y1)) = surface.clip_to_bounds(padded) else {
        return;
    };
    let (w, h) = (x1 - x0, y1 - y0);
    if w == 0 || h == 0 {
        return;
    }
    let cov = poly.coverage(w, h, x0 as i32, y0 as i32);
    // Zero out coverage outside the (unpadded) clip.
    let clip_px = clip.intersect(&surface.bounds());
    let cov = mask_to_clip(cov, w, h, x0, y0, clip_px);
    composite(surface, (x0 as i32, y0 as i32), w, h, &cov, color);
}

/// Tight bounds of a polygon. Tracked as min/max rather than via `Rect::union`,
/// because points and zero-height rects are `is_empty()` and would be dropped.
fn poly_bounds(poly: &Polygon) -> Rect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut any = false;
    for (x0, y0, x1, y1) in poly.edges() {
        for (x, y) in [(x0, y0), (x1, y1)] {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            any = true;
        }
    }
    if !any {
        return Rect::ZERO;
    }
    Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn mask_to_clip(mut cov: Vec<u8>, w: u32, h: u32, ox: u32, oy: u32, clip: Rect) -> Vec<u8> {
    if clip.is_empty() {
        cov.fill(0);
        return cov;
    }
    let cx0 = clip.x.floor() as i64;
    let cy0 = clip.y.floor() as i64;
    let cx1 = clip.right().ceil() as i64;
    let cy1 = clip.bottom().ceil() as i64;
    for row in 0..h {
        let sy = oy as i64 + row as i64;
        if sy < cy0 || sy >= cy1 {
            let base = (row * w) as usize;
            cov[base..base + w as usize].fill(0);
            continue;
        }
        for col in 0..w {
            let sx = ox as i64 + col as i64;
            if sx < cx0 || sx >= cx1 {
                cov[(row * w + col) as usize] = 0;
            }
        }
    }
    cov
}

/// Paint a 1-px (or `width`-px) solid border around `rect`, inside its edges.
///
/// Per-side colors allow the CSS rule that abutting border colors taper at
/// diagonals; with `widths` all equal and non-zero, the corners are mitered by
/// simply assigning the corner pixel to the nearest side (we draw each side as
/// an exact rectangle and overlap at corners, last write wins — documented in
/// the architecture as a solid-border approximation).
pub fn stroke_border(
    surface: &mut Surface,
    rect: Rect,
    widths: EdgeSizes,
    colors: [Rgba8; 4],
    clip: Rect,
) {
    if widths.top > 0.0 {
        fill_rect(
            surface,
            Rect::new(rect.x, rect.y, rect.w, widths.top),
            colors[0],
            clip,
        );
    }
    if widths.right > 0.0 {
        fill_rect(
            surface,
            Rect::new(rect.right() - widths.right, rect.y, widths.right, rect.h),
            colors[1],
            clip,
        );
    }
    if widths.bottom > 0.0 {
        fill_rect(
            surface,
            Rect::new(rect.x, rect.bottom() - widths.bottom, rect.w, widths.bottom),
            colors[2],
            clip,
        );
    }
    if widths.left > 0.0 {
        fill_rect(
            surface,
            Rect::new(rect.x, rect.y, widths.left, rect.h),
            colors[3],
            clip,
        );
    }
}

/// Fill a 1-px-thick outline just outside `rect` (for `outline` property).
pub fn stroke_outline(surface: &mut Surface, rect: Rect, color: Rgba8, width: f32, clip: Rect) {
    if width <= 0.0 {
        return;
    }
    let w = width;
    fill_rect(surface, Rect::new(rect.x - w, rect.y - w, rect.w + 2.0 * w, w), color, clip);
    fill_rect(
        surface,
        Rect::new(rect.x - w, rect.bottom(), rect.w + 2.0 * w, w),
        color,
        clip,
    );
    fill_rect(surface, Rect::new(rect.x - w, rect.y, w, rect.h), color, clip);
    fill_rect(surface, Rect::new(rect.right(), rect.y, w, rect.h), color, clip);
}

/// A 2-stop linear gradient, angle in CSS degrees (0 = to top, 90 = to right).
///
/// Approximation recorded in the report by the paint stage: only two stops are
/// supported (§15, `gradients beyond 2-stop linear`).
pub fn fill_linear_gradient(
    surface: &mut Surface,
    rect: Rect,
    from: Rgba8,
    to: Rgba8,
    angle_deg: f32,
    clip: Rect,
) {
    let draw = rect.intersect(&clip).intersect(&surface.bounds());
    let Some((x0, y0, x1, y1)) = surface.clip_to_bounds(draw) else {
        return;
    };
    // CSS angle: 0deg points up, 90deg points right. Convert to a direction vector.
    let rad = angle_deg.to_radians();
    let (dx, dy) = (rad.sin(), -rad.cos());
    // Gradient line length for the rect (CSS Images 3 §3.1.1).
    let len = (rect.w * dx).abs() + (rect.h * dy).abs();
    if len <= 0.0 {
        return;
    }
    let cx = rect.center().x;
    let cy = rect.center().y;
    let _ = (from.premultiplied(), to.premultiplied());
    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5 - cx;
            let py = y as f32 + 0.5 - cy;
            let t = ((px * dx + py * dy) / len + 0.5).clamp(0.0, 1.0);
            let lerp = |a: u8, b: u8| -> u8 {
                (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
            };
            let c = Rgba8 {
                r: lerp(from.r, to.r),
                g: lerp(from.g, to.g),
                b: lerp(from.b, to.b),
                a: lerp(from.a, to.a),
            };
            surface.set(x, y, c);
        }
    }
}

/// A viewport scrollbar. Track and thumb are painted with the fixed token set
/// described in §8.5 so the result is stable across Windows versions.
pub fn fill_scrollbar(
    surface: &mut Surface,
    track: Rect,
    thumb: Rect,
    axis: Axis,
    clip: Rect,
) {
    let track_color = Rgba8::rgb(0xf0, 0xf0, 0xf0);
    let thumb_color = Rgba8::rgb(0xc0, 0xc0, 0xc0);
    let border_color = Rgba8::rgb(0xd6, 0xd6, 0xd6);
    fill_rect(surface, track, track_color, clip);
    // 1px separator on the side facing the content.
    let sep = match axis {
        Axis::Vertical => Rect::new(track.x, track.y, 1.0, track.h),
        Axis::Horizontal => Rect::new(track.x, track.y, track.w, 1.0),
    };
    fill_rect(surface, sep, border_color, clip);
    fill_rounded_rect(surface, thumb, CornerRadii::uniform(3.0), thumb_color, clip);
}

/// Form controls, painted by us rather than by the OS (§8.5). The palette is a
/// small fixed token set; this is an explicit, reported approximation.
pub fn paint_control(
    surface: &mut Surface,
    rect: Rect,
    kind: ControlKind,
    state: ControlState,
    radius: f32,
    clip: Rect,
) {
    match kind {
        ControlKind::Checkbox => {
            let border = Rgba8::rgb(0x76, 0x76, 0x76);
            fill_rect(surface, rect, Rgba8::WHITE, clip);
            stroke_border(surface, rect, EdgeSizes::uniform(1.0), [border; 4], clip);
            if state == ControlState::Checked {
                let inset = Rect::new(rect.x + 3.0, rect.y + 3.0, rect.w - 6.0, rect.h - 6.0);
                fill_rect(surface, inset, Rgba8::rgb(0x00, 0x5f, 0xb8), clip);
            }
        }
        ControlKind::Radio => {
            let border = Rgba8::rgb(0x76, 0x76, 0x76);
            let r = CornerRadii::uniform(rect.w.min(rect.h) * 0.5);
            fill_rounded_rect(surface, rect, r, Rgba8::WHITE, clip);
            fill_rounded_rect(
                surface,
                Rect::new(rect.x + 1.0, rect.y + 1.0, rect.w - 2.0, rect.h - 2.0),
                CornerRadii::uniform((rect.w.min(rect.h) - 2.0) * 0.5),
                border,
                clip,
            );
            if state == ControlState::Checked {
                let d = rect.w.min(rect.h) * 0.45;
                let dot = Rect::new(
                    rect.x + (rect.w - d) * 0.5,
                    rect.y + (rect.h - d) * 0.5,
                    d,
                    d,
                );
                fill_rounded_rect(surface, dot, CornerRadii::uniform(d * 0.5), Rgba8::rgb(0x00, 0x5f, 0xb8), clip);
            }
        }
        ControlKind::TextInput => {
            fill_rect(surface, rect, Rgba8::WHITE, clip);
            let c = if state == ControlState::Focused {
                Rgba8::rgb(0x00, 0x78, 0xd4)
            } else {
                Rgba8::rgb(0x76, 0x76, 0x76)
            };
            stroke_border(surface, rect, EdgeSizes::uniform(1.0), [c; 4], clip);
        }
        ControlKind::Button | ControlKind::Submit => {
            let bg = match state {
                ControlState::Hover => Rgba8::rgb(0xe5, 0xf1, 0xfb),
                ControlState::Active => Rgba8::rgb(0xcc, 0xe4, 0xf7),
                _ => Rgba8::rgb(0xef, 0xef, 0xef),
            };
            fill_rounded_rect(surface, rect, CornerRadii::uniform(radius), bg, clip);
            stroke_border(
                surface,
                rect,
                EdgeSizes::uniform(1.0),
                [Rgba8::rgb(0x76, 0x76, 0x76); 4],
                clip,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rect_exact_bytes() {
        let mut s = Surface::new(5, 5);
        fill_rect(&mut s, Rect::new(1.0, 1.0, 2.0, 2.0), Rgba8::rgb(255, 0, 0), NO_CLIP);
        // Inside: pure red, premultiplied-opaque => BGRA (0,0,255,255)
        assert_eq!(&s.pixels[s.pixel_offset(1, 1)..][..4], &[0, 0, 255, 255]);
        assert_eq!(&s.pixels[s.pixel_offset(2, 2)..][..4], &[0, 0, 255, 255]);
        // Outside: untouched
        assert_eq!(&s.pixels[s.pixel_offset(0, 1)..][..4], &[0, 0, 0, 0]);
        assert_eq!(&s.pixels[s.pixel_offset(3, 3)..][..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn fill_rect_respects_clip() {
        let mut s = Surface::new(10, 10);
        fill_rect(
            &mut s,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rgba8::rgb(0, 255, 0),
            Rect::new(0.0, 0.0, 5.0, 5.0),
        );
        assert_eq!(s.get(4, 4), Rgba8::rgb(0, 255, 0));
        assert_eq!(s.get(5, 5), Rgba8::TRANSPARENT);
    }

    #[test]
    fn fill_rect_alpha_blends_over_background() {
        let mut s = Surface::filled(1, 1, Rgba8::rgb(0, 0, 255));
        fill_rect(
            &mut s,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Rgba8::rgba(255, 0, 0, 128),
            NO_CLIP,
        );
        let out = s.get(0, 0);
        assert_eq!(out.a, 255, "blending over opaque blue keeps alpha 255");
        assert!((out.r as i32 - 128).abs() <= 1, "r={}", out.r);
        assert_eq!(out.g, 0);
        assert!((out.b as i32 - 127).abs() <= 1, "b={}", out.b);
    }

    #[test]
    fn fractional_rect_has_aa_edge() {
        let mut s = Surface::new(10, 10);
        fill_rect(&mut s, Rect::new(2.5, 0.0, 4.0, 4.0), Rgba8::BLACK, NO_CLIP);
        let left = s.get(2, 1);
        // Half-covered pixel: ~50% alpha black over transparent.
        assert!(left.a > 100 && left.a < 160, "a={}", left.a);
        let inside = s.get(3, 1);
        assert_eq!(inside.a, 255);
    }

    #[test]
    fn rounded_rect_corners_transparent() {
        let mut s = Surface::new(20, 20);
        fill_rounded_rect(
            &mut s,
            Rect::new(0.0, 0.0, 20.0, 20.0),
            CornerRadii::uniform(6.0),
            Rgba8::rgb(0, 0, 0),
            NO_CLIP,
        );
        assert_eq!(s.get(0, 0).a, 0, "corner must be outside the rounded shape");
        assert_eq!(s.get(10, 10), Rgba8::BLACK);
        assert_eq!(s.get(10, 0), Rgba8::BLACK, "top edge midpoint is inside");
    }

    #[test]
    fn border_draws_four_edges_one_px() {
        let mut s = Surface::new(10, 10);
        stroke_border(
            &mut s,
            Rect::new(1.0, 1.0, 8.0, 8.0),
            EdgeSizes::uniform(1.0),
            [Rgba8::BLACK, Rgba8::rgb(255, 0, 0), Rgba8::rgb(0, 255, 0), Rgba8::rgb(0, 0, 255)],
            NO_CLIP,
        );
        // top edge (y=1) black
        assert_eq!(s.get(5, 1), Rgba8::BLACK);
        // right edge (x=8) red
        assert_eq!(s.get(8, 5), Rgba8::rgb(255, 0, 0));
        // bottom edge (y=8) green
        assert_eq!(s.get(5, 8), Rgba8::rgb(0, 255, 0));
        // left edge (x=1) blue
        assert_eq!(s.get(1, 5), Rgba8::rgb(0, 0, 255));
        // interior untouched
        assert_eq!(s.get(5, 5), Rgba8::TRANSPARENT);
        // outside untouched
        assert_eq!(s.get(0, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn one_px_border_is_exactly_opaque() {
        let mut s = Surface::filled(12, 12, Rgba8::WHITE);
        stroke_border(
            &mut s,
            Rect::new(2.0, 2.0, 8.0, 8.0),
            EdgeSizes::uniform(1.0),
            [Rgba8::BLACK; 4],
            NO_CLIP,
        );
        for x in 2..10 {
            assert_eq!(s.get(x, 2), Rgba8::BLACK, "top row x={x}");
            assert_eq!(s.get(x, 9), Rgba8::BLACK, "bottom row x={x}");
        }
        for y in 2..10 {
            assert_eq!(s.get(2, y), Rgba8::BLACK, "left col y={y}");
            assert_eq!(s.get(9, y), Rgba8::BLACK, "right col y={y}");
        }
        // Interior stays white — no bleed.
        assert_eq!(s.get(5, 5), Rgba8::WHITE);
        // Just outside stays white.
        assert_eq!(s.get(1, 1), Rgba8::WHITE);
    }

    #[test]
    fn determinism_two_renders_byte_equal() {
        let draw = |s: &mut Surface| {
            fill_rounded_rect(
                s,
                Rect::new(3.3, 2.7, 50.2, 30.9),
                CornerRadii::uniform(7.25),
                Rgba8::rgba(10, 200, 40, 180),
                NO_CLIP,
            );
            stroke_border(
                s,
                Rect::new(3.3, 2.7, 50.2, 30.9),
                EdgeSizes::new(1.5, 2.0, 1.0, 3.0),
                [Rgba8::BLACK, Rgba8::rgb(1, 2, 3), Rgba8::rgb(9, 9, 9), Rgba8::rgb(4, 5, 6)],
                NO_CLIP,
            );
            fill_linear_gradient(
                s,
                Rect::new(10.0, 10.0, 20.0, 20.0),
                Rgba8::BLACK,
                Rgba8::WHITE,
                45.0,
                NO_CLIP,
            );
        };
        let mut a = Surface::new(64, 48);
        let mut b = Surface::new(64, 48);
        draw(&mut a);
        draw(&mut b);
        assert_eq!(a.pixels, b.pixels, "same input must produce identical bytes");
    }

    #[test]
    fn gradient_endpoints_hit_stops() {
        let mut s = Surface::new(10, 1);
        // 90deg = left-to-right.
        fill_linear_gradient(
            &mut s,
            Rect::new(0.0, 0.0, 10.0, 1.0),
            Rgba8::BLACK,
            Rgba8::WHITE,
            90.0,
            NO_CLIP,
        );
        let left = s.get(0, 0);
        let right = s.get(9, 0);
        assert!(left.r < 20, "left should be near black, got {}", left.r);
        assert!(right.r > 235, "right should be near white, got {}", right.r);
    }

    #[test]
    fn fully_transparent_fill_is_noop() {
        let mut s = Surface::new(4, 4);
        let before = s.clone();
        fill_rect(&mut s, Rect::new(0.0, 0.0, 4.0, 4.0), Rgba8::TRANSPARENT, NO_CLIP);
        assert_eq!(s, before);
    }

    #[test]
    fn outline_draws_outside_rect() {
        let mut s = Surface::filled(10, 10, Rgba8::WHITE);
        stroke_outline(&mut s, Rect::new(3.0, 3.0, 4.0, 4.0), Rgba8::BLACK, 1.0, NO_CLIP);
        assert_eq!(s.get(2, 2), Rgba8::BLACK, "outside the corner is outlined");
        assert_eq!(s.get(3, 3), Rgba8::WHITE, "the rect interior is not painted");
    }
}