//! Image blitting: decode-agnostic.
//!
//! The `png`/`jpeg-decoder` decode path lives in `engine-paint`; this crate only
//! knows how to scale and place an already-decoded RGBA buffer. Separable box
//! filter for downscale, bilinear for upscale (§8.4).

use engine_core::{Rect, Rgba8};

use crate::list::{BackgroundRepeat, ObjectFit};
use crate::surface::Surface;

/// A decoded, straight-alpha RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, straight alpha.
    pub pixels: Vec<u8>,
}

impl DecodedImage {
    pub fn get(&self, x: u32, y: u32) -> Rgba8 {
        if x >= self.width || y >= self.height {
            return Rgba8::TRANSPARENT;
        }
        let o = ((y as usize) * (self.width as usize) + x as usize) * 4;
        Rgba8 {
            r: self.pixels[o],
            g: self.pixels[o + 1],
            b: self.pixels[o + 2],
            a: self.pixels[o + 3],
        }
    }
}

/// Source of decoded images.
pub trait ImageStore {
    fn get(&self, key: engine_core::ImageKey) -> Option<&DecodedImage>;
}

/// Bilinear sample at continuous coordinates `(u, v)` in image space.
fn sample_bilinear(img: &DecodedImage, u: f32, v: f32) -> Rgba8 {
    let u = u.clamp(0.0, (img.width.max(1) - 1) as f32);
    let v = v.clamp(0.0, (img.height.max(1) - 1) as f32);
    let x0 = u.floor() as u32;
    let y0 = v.floor() as u32;
    let x1 = (x0 + 1).min(img.width.saturating_sub(1));
    let y1 = (y0 + 1).min(img.height.saturating_sub(1));
    let tx = u - x0 as f32;
    let ty = v - y0 as f32;
    let a = img.get(x0, y0);
    let b = img.get(x1, y0);
    let c = img.get(x0, y1);
    let d = img.get(x1, y1);
    let lerp = |p: u8, q: u8, t: f32| p as f32 + (q as f32 - p as f32) * t;
    let ch = |c: Rgba8| (c.r, c.g, c.b, c.a);
    let (ar, ag, ab, aa) = ch(a);
    let (br, bg, bb, ba) = ch(b);
    let (cr, cg, cb, ca) = ch(c);
    let (dr, dg, db, da) = ch(d);
    let top = |p: u8, q: u8| lerp(p, q, tx);
    let bot = |p: u8, q: u8| lerp(p, q, tx);
    Rgba8::from_f64(
        top(ar, br) as f64 + (bot(cr, dr) as f64 - top(ar, br) as f64) * ty as f64,
        top(ag, bg) as f64 + (bot(cg, dg) as f64 - top(ag, bg) as f64) * ty as f64,
        top(ab, bb) as f64 + (bot(cb, db) as f64 - top(ab, bb) as f64) * ty as f64,
        (top(aa, ba) as f64 + (bot(ca, da) as f64 - top(aa, ba) as f64) * ty as f64) / 255.0,
    )
}

/// Blit an image into `dst`, honouring `object-fit` and background-repeat.
pub fn blit_image(
    surface: &mut Surface,
    img: &DecodedImage,
    dst: Rect,
    fit: ObjectFit,
    opacity: f32,
    repeat: BackgroundRepeat,
    clip: Rect,
) {
    if img.width == 0 || img.height == 0 || opacity <= 0.0 {
        return;
    }
    let target = dst.intersect(&clip).intersect(&surface.bounds());
    let Some((x0, y0, x1, y1)) = surface.clip_to_bounds(target) else {
        return;
    };
    let (iw, ih) = (img.width as f32, img.height as f32);
    let (fw, fh) = (dst.w, dst.h);
    if fw <= 0.0 || fh <= 0.0 {
        return;
    }
    let (scale_x, scale_y, off_x, off_y) = match fit {
        ObjectFit::Fill => (fw / iw, fh / ih, 0.0, 0.0),
        ObjectFit::Contain | ObjectFit::ScaleDown => {
            let s = (fw / iw).min(fh / ih);
            let s = if matches!(fit, ObjectFit::ScaleDown) { s.min(1.0) } else { s };
            (s, s, (fw - iw * s) * 0.5, (fh - ih * s) * 0.5)
        }
        ObjectFit::Cover => {
            let s = (fw / iw).max(fh / ih);
            (s, s, (fw - iw * s) * 0.5, (fh - ih * s) * 0.5)
        }
        ObjectFit::None => (1.0, 1.0, (fw - iw) * 0.5, (fh - ih) * 0.5),
    };
    let opacity = opacity.clamp(0.0, 1.0);
    let repeat_x = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX);
    let repeat_y = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY);
    let tile_w = iw * scale_x;
    let tile_h = ih * scale_y;

    for y in y0..y1 {
        for x in x0..x1 {
            // Distance from the top-left of the (possibly fit-inset) content box,
            // in destination pixels, measured at the pixel's centre.
            let mut px = (x as f32 + 0.5) - dst.x - off_x;
            let mut py = (y as f32 + 0.5) - dst.y - off_y;
            if tile_w > 0.0 && tile_h > 0.0 {
                // Wrap into the tile. A non-repeating axis falls through to the
                // bounds test below and leaves the pixel unpainted.
                if repeat_x {
                    px = px.rem_euclid(tile_w);
                }
                if repeat_y {
                    py = py.rem_euclid(tile_h);
                }
            }
            if px < 0.0 || py < 0.0 || (px >= tile_w && !repeat_x) || (py >= tile_h && !repeat_y) {
                continue;
            }
            // Map the destination sample point to texel space. Destination
            // sample centre at `px` -> source coordinate `px / scale`; a texel
            // is sampled at its own centre, so shift by half a texel.
            let u = px / scale_x - 0.5;
            let v = py / scale_y - 0.5;
            let mut c = sample_bilinear(img, u, v);
            if opacity < 1.0 {
                c.a = (c.a as f32 * opacity).round() as u8;
            }
            if c.a == 0 {
                continue;
            }
            let [r, g, b, a] = c.premultiplied();
            surface.blend_premul_at(x, y, [r, g, b, a], 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: Rgba8) -> DecodedImage {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            px.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
        DecodedImage { width: w, height: h, pixels: px }
    }

    #[test]
    fn fill_draws_whole_box() {
        let img = solid(2, 2, Rgba8::rgb(0, 128, 0));
        let mut s = Surface::new(10, 10);
        let b = s.bounds();
        blit_image(&mut s, &img, Rect::new(2.0, 2.0, 4.0, 4.0), ObjectFit::Fill, 1.0, BackgroundRepeat::NoRepeat, b);
        assert_eq!(s.get(3, 3), Rgba8::rgb(0, 128, 0));
        assert_eq!(s.get(1, 1), Rgba8::TRANSPARENT);
    }

    #[test]
    fn contain_preserves_aspect_and_letterboxes() {
        // 4x2 image into 8x8 box: contain -> 8x4 centred vertically at y=2.
        let img = solid(4, 2, Rgba8::rgb(255, 0, 0));
        let mut s = Surface::new(10, 10);
        let b = s.bounds();
        blit_image(&mut s, &img, Rect::new(0.0, 0.0, 8.0, 8.0), ObjectFit::Contain, 1.0, BackgroundRepeat::NoRepeat, b);
        assert_eq!(s.get(4, 1).a, 0, "letterbox above");
        assert_eq!(s.get(4, 4), Rgba8::rgb(255, 0, 0));
        assert_eq!(s.get(4, 7).a, 0, "letterbox below");
    }

    #[test]
    fn opacity_scales_alpha() {
        let img = solid(2, 2, Rgba8::rgb(0, 0, 0));
        let mut s = Surface::filled(4, 4, Rgba8::WHITE);
        let b = s.bounds();
        blit_image(&mut s, &img, Rect::new(0.0, 0.0, 4.0, 4.0), ObjectFit::Fill, 0.5, BackgroundRepeat::NoRepeat, b);
        let p = s.get(1, 1);
        assert!((p.r as i32 - 128).abs() <= 2, "r={}", p.r);
    }

    #[test]
    fn repeat_tiles_at_natural_size() {
        // A background image repeats at its natural size; with object-fit: none
        // and a 4x4 box the 2x2 image tiles 2x2. (With `fill` the image would be
        // stretched to the whole box, so tiling would be invisible — that is
        // correct CSS, and the reason this test uses `none`.)
        let mut px = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                let c = if x == 0 && y == 0 { Rgba8::rgb(255, 0, 0) } else { Rgba8::rgb(0, 0, 255) };
                px.extend_from_slice(&[c.r, c.g, c.b, c.a]);
            }
        }
        let img = DecodedImage { width: 2, height: 2, pixels: px };
        let mut s = Surface::new(4, 4);
        let b = s.bounds();
        blit_image(&mut s, &img, Rect::new(0.0, 0.0, 4.0, 4.0), ObjectFit::None, 1.0, BackgroundRepeat::Repeat, b);
        // object-fit: none centres the natural-size image, so it is inset by
        // (4-2)/2 = 1px; the tiling grid therefore starts at (1,1).
        assert_eq!(s.get(1, 1), Rgba8::rgb(255, 0, 0), "first red texel at the tile origin");
        assert_eq!(s.get(3, 3), Rgba8::rgb(255, 0, 0), "red texel repeats after one 2px tile");
        assert_eq!(s.get(2, 1), Rgba8::rgb(0, 0, 255), "blue texel of the first tile");
    }

    #[test]
    fn fill_stretches_image_over_the_box() {
        // The counterpart of the test above: `fill` scales 2x2 to 4x4, so there
        // is no visible tiling and the top-left half is the red texel area.
        let mut px = Vec::new();
        for _ in 0..4 {
            px.extend_from_slice(&[255, 0, 0, 255]);
        }
        let img = DecodedImage { width: 2, height: 2, pixels: px };
        let mut s = Surface::new(4, 4);
        let b = s.bounds();
        blit_image(&mut s, &img, Rect::new(0.0, 0.0, 4.0, 4.0), ObjectFit::Fill, 1.0, BackgroundRepeat::Repeat, b);
        assert_eq!(s.get(3, 3), Rgba8::rgb(255, 0, 0), "whole box is the stretched image");
    }
}