//! A premultiplied BGRA8 CPU surface.
//!
//! Storage format is fixed by §3.2/§2: BGRA8 premultiplied, which is exactly
//! `DXGI_FORMAT_B8G8R8A8_UNORM` for the present path and GDI's 32-bpp DIB
//! layout for the fallback. Storing premultiplied means blending is the cheap
//! `dst = src + dst * (1 - src.a)` without unpacking alpha per pixel.

// `chunks_exact(4)` reads more clearly than `as_chunks::<4>()` at these call
// sites, and clippy's suggested rewrite does not improve the byte handling.
#![allow(clippy::chunks_exact_to_as_chunks)]

use engine_core::{Rect, Rgba8};

pub const BPP: usize = 4;

/// A CPU bitmap. `stride` is `width * 4` today; the field exists so a
/// compatible sub-surface (a view into a larger buffer) remains expressible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixels: Vec<u8>,
}

impl Surface {
    /// A transparent-black surface.
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * BPP as u32;
        Surface { width, height, stride, pixels: vec![0; (stride as usize) * height as usize] }
    }

    /// A surface filled with an opaque color.
    pub fn filled(width: u32, height: u32, color: Rgba8) -> Self {
        let mut s = Surface::new(width, height);
        s.clear(color);
        s
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f32, self.height as f32)
    }

    #[inline]
    pub fn row_offset(&self, y: u32) -> usize {
        y as usize * self.stride as usize
    }

    #[inline]
    pub fn pixel_offset(&self, x: u32, y: u32) -> usize {
        self.row_offset(y) + x as usize * BPP
    }

    /// Read a pixel as a straight (un-premultiplied) color. Useful for tests and
    /// for the PNG writer, which wants straight alpha.
    pub fn get(&self, x: u32, y: u32) -> Rgba8 {
        let o = self.pixel_offset(x, y);
        let (b, g, r, a) = (self.pixels[o], self.pixels[o + 1], self.pixels[o + 2], self.pixels[o + 3]);
        if a == 0 {
            return Rgba8::TRANSPARENT;
        }
        let un = |c: u8| -> u8 { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        Rgba8 { r: un(r), g: un(g), b: un(b), a }
    }

    /// Write a pixel with straight color, replacing content (no blending).
    pub fn set(&mut self, x: u32, y: u32, color: Rgba8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let o = self.pixel_offset(x, y);
        let [r, g, b, a] = color.premultiplied();
        self.pixels[o] = b;
        self.pixels[o + 1] = g;
        self.pixels[o + 2] = r;
        self.pixels[o + 3] = a;
    }

    /// Fill the whole surface, replacing content.
    pub fn clear(&mut self, color: Rgba8) {
        let [r, g, b, a] = color.premultiplied();
        if a == 0 && r == 0 && g == 0 && b == 0 {
            self.pixels.fill(0);
            return;
        }
        let px = [b, g, r, a];
        for chunk in self.pixels.chunks_exact_mut(BPP) {
            chunk.copy_from_slice(&px);
        }
    }

    /// Source-over blend one premultiplied pixel.
    ///
    /// `src` is `[r, g, b, a]`, already premultiplied. `coverage` scales the
    /// whole source contribution (anti-aliasing edge weight). Premultiplied
    /// source-over is `out = src + dst * (1 - src.a)`, applied per channel
    /// including alpha.
    #[inline]
    pub fn blend_premul_at(&mut self, x: u32, y: u32, src: [u8; 4], coverage: u8) {
        if x >= self.width || y >= self.height || coverage == 0 {
            return;
        }
        let o = self.pixel_offset(x, y);
        // Scale the source by coverage. Every channel scales together, so the
        // premultiplied invariant (c <= a) is preserved.
        let s = |c: u8| ((c as u32 * coverage as u32 + 127) / 255) as u8;
        let (sr, sg, sb, sa) = (s(src[0]), s(src[1]), s(src[2]), s(src[3]));
        if sa == 255 {
            // Fast, exact path: an opaque source fully replaces the pixel.
            self.pixels[o] = sb;
            self.pixels[o + 1] = sg;
            self.pixels[o + 2] = sr;
            self.pixels[o + 3] = 255;
            return;
        }
        // The fraction of the destination that survives is (1 - sa), where sa is
        // the *effective* source alpha after coverage scaling.
        let keep = 255 - sa as u32;
        let add = |s: u8, d: u8| -> u8 {
            (s as u32 + (d as u32 * keep + 127) / 255).min(255) as u8
        };
        self.pixels[o] = add(sb, self.pixels[o]);
        self.pixels[o + 1] = add(sg, self.pixels[o + 1]);
        self.pixels[o + 2] = add(sr, self.pixels[o + 2]);
        self.pixels[o + 3] = add(sa, self.pixels[o + 3]);
    }

    /// Convert to straight-alpha RGBA8 row-major, for PNG output.
    pub fn to_rgba_straight(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len());
        for chunk in self.pixels.chunks_exact(BPP) {
            let (b, g, r, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let un = |c: u8| -> u8 { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
                out.extend_from_slice(&[un(r), un(g), un(b), a]);
            }
        }
        out
    }

    /// The drawable region in integer pixels, clamped to this surface.
    pub fn clip_to_bounds(&self, r: Rect) -> Option<(u32, u32, u32, u32)> {
        let r = r.intersect(&self.bounds());
        if r.w <= 0.0 || r.h <= 0.0 {
            return None;
        }
        let x0 = r.x.floor().max(0.0) as u32;
        let y0 = r.y.floor().max(0.0) as u32;
        let x1 = (r.right().ceil() as u32).min(self.width);
        let y1 = (r.bottom().ceil() as u32).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some((x0, y0, x1, y1))
        }
    }

    /// Pixel statistics used by the golden/screenshot verification path:
    /// number of distinct colours and number of non-white pixels.
    pub fn stats(&self) -> SurfaceStats {
        use std::collections::HashSet;
        let mut distinct = HashSet::new();
        let mut non_white = 0u64;
        for chunk in self.pixels.chunks_exact(BPP) {
            distinct.insert(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            // Compare in straight terms; premultiplied white is (255,255,255,255).
            if !(chunk[0] == 255 && chunk[1] == 255 && chunk[2] == 255 && chunk[3] == 255) {
                non_white += 1;
            }
        }
        SurfaceStats { distinct_colours: distinct.len() as u32, non_white_pixels: non_white }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceStats {
    pub distinct_colours: u32,
    pub non_white_pixels: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_surface_is_transparent() {
        let s = Surface::new(4, 3);
        assert_eq!(s.pixels.len(), 4 * 4 * 3);
        assert!(s.pixels.iter().all(|&b| b == 0));
        assert_eq!(s.get(0, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn set_and_get_roundtrip_opaque() {
        let mut s = Surface::new(2, 2);
        s.set(1, 1, Rgba8::rgb(10, 20, 30));
        assert_eq!(s.get(1, 1), Rgba8::rgb(10, 20, 30));
        assert_eq!(s.get(0, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn storage_is_bgra() {
        let mut s = Surface::new(1, 1);
        s.set(0, 0, Rgba8::rgb(0x11, 0x22, 0x33));
        assert_eq!(&s.pixels[..4], &[0x33, 0x22, 0x11, 0xff]);
    }

    #[test]
    fn set_out_of_bounds_is_noop() {
        let mut s = Surface::new(2, 2);
        s.set(9, 9, Rgba8::BLACK);
        assert_eq!(s, Surface::new(2, 2));
    }

    #[test]
    fn clear_fills_opaque() {
        let mut s = Surface::new(3, 3);
        s.clear(Rgba8::rgb(1, 2, 3));
        assert!(s.pixels.chunks_exact(4).all(|c| c == [3, 2, 1, 255]));
    }

    #[test]
    fn premultiplied_half_alpha_unpacks_exactly() {
        let mut s = Surface::new(1, 1);
        s.set(0, 0, Rgba8::rgba(200, 100, 50, 128));
        // premultiplied r = 200*128/255 = 100, g = 50, b = 25
        assert_eq!(&s.pixels[..4], &[25, 50, 100, 128]);
        let back = s.get(0, 0);
        assert_eq!(back.a, 128);
        // Round-trip through 8-bit premultiplication is lossy by at most a
        // couple of levels; assert the bound rather than pretend it's exact.
        assert!((back.r as i32 - 200).abs() <= 2, "r={}", back.r);
        assert!((back.g as i32 - 100).abs() <= 2, "g={}", back.g);
        assert!((back.b as i32 - 50).abs() <= 2, "b={}", back.b);
    }

    #[test]
    fn clip_to_bounds_clamps() {
        let s = Surface::new(10, 10);
        let c = s.clip_to_bounds(Rect::new(-5.0, -5.0, 100.0, 100.0));
        assert_eq!(c, Some((0, 0, 10, 10)));
        assert_eq!(s.clip_to_bounds(Rect::new(20.0, 20.0, 5.0, 5.0)), None);
    }

    #[test]
    fn stats_counts_distinct_and_non_white() {
        let mut s = Surface::filled(2, 1, Rgba8::WHITE);
        s.set(0, 0, Rgba8::BLACK);
        let st = s.stats();
        assert_eq!(st.non_white_pixels, 1);
        assert_eq!(st.distinct_colours, 2);
    }

    #[test]
    fn straight_rgba_is_row_major_rgba() {
        let mut s = Surface::new(1, 1);
        s.set(0, 0, Rgba8::rgb(1, 2, 3));
        assert_eq!(s.to_rgba_straight(), vec![1, 2, 3, 255]);
    }
}