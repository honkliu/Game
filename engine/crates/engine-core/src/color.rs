//! Colors: an 8-bit sRGB RGBA type, the CSS named-color table we own, and
//! premultiplication for blending.

/// Straight (non-premultiplied) sRGB color with 8-bit channels.
///
/// The rasterizer stores *premultiplied* BGRA8 in the surface (§3.2). This type
/// stays straight, because that is what CSS authors think in (`rgba(0,0,0,.5)`),
/// and conversion happens once at blend time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const TRANSPARENT: Rgba8 = Rgba8 { r: 0, g: 0, b: 0, a: 0 };
    pub const BLACK: Rgba8 = Rgba8 { r: 0, g: 0, b: 0, a: 255 };
    pub const WHITE: Rgba8 = Rgba8 { r: 255, g: 255, b: 255, a: 255 };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba8 { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba8 { r, g, b, a }
    }

    pub fn from_hex(h: u32) -> Self {
        Rgba8 {
            r: ((h >> 16) & 0xff) as u8,
            g: ((h >> 8) & 0xff) as u8,
            b: (h & 0xff) as u8,
            a: 255,
        }
    }

    pub fn is_opaque(&self) -> bool {
        self.a == 255
    }

    pub fn is_transparent(&self) -> bool {
        self.a == 0
    }

    /// `rgba()` syntax components (0..=255) plus a resolved alpha 0..=1.
    pub fn from_f64(r: f64, g: f64, b: f64, a: f64) -> Self {
        let c = |v: f64| v.clamp(0.0, 255.0).round() as u8;
        Rgba8 { r: c(r), g: c(g), b: c(b), a: (a.clamp(0.0, 1.0) * 255.0).round() as u8 }
    }

    /// Premultiply in the u8 domain (matches the rasterizer's 8-bit blend path).
    /// `r' = r * a / 255`, rounded to nearest.
    pub fn premultiplied(&self) -> [u8; 4] {
        let a = self.a as u32;
        let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        [pm(self.r), pm(self.g), pm(self.b), self.a]
    }

    /// Source-over composite in the u8 domain, operating on straight colors.
    /// Used by tests and by the window presenter's opaque path.
    pub fn source_over(self, dst: Rgba8) -> Rgba8 {
        if self.a == 255 {
            return self;
        }
        if self.a == 0 {
            return dst;
        }
        let sa = self.a as u32;
        let da = dst.a as u32;
        // out_a = sa + da*(255-sa)/255
        let out_a = sa + (da * (255 - sa) + 127) / 255;
        if out_a == 0 {
            return Rgba8::TRANSPARENT;
        }
        let blend = |s: u8, d: u8| -> u8 {
            // out_c = (s*sa + d*da*(255-sa)/255) / out_a
            let num = s as u32 * sa * 255 + d as u32 * da * (255 - sa);
            ((num + out_a * 255 / 2) / (out_a * 255)) as u8
        };
        Rgba8 { r: blend(self.r, dst.r), g: blend(self.g, dst.g), b: blend(self.b, dst.b), a: out_a as u8 }
    }
}

/// CSS color keywords we support. This is the CSS Color 3 keyword set that
/// appears in real stylesheets; it is ours, not a dependency (§1.1 `ahash` note).
/// Lookup is case-insensitive on the *cascade* side (values are lowercased first).
pub fn named_color(name: &str) -> Option<Rgba8> {
    // Sorted-ish by frequency of use in real CSS; linear scan over a static slice
    // is faster than a HashMap for ~150 entries at parse time and needs no init.
    let n = name.as_bytes();
    // Fast rejection via first-byte dispatch avoids most `eq_ignore_ascii_case` calls.
    let first = *n.first()? | 0x20;
    for (k, v) in NAMED.iter() {
        if k.as_bytes()[0] | 0x20 == first && k.eq_ignore_ascii_case(name) {
            return Some(Rgba8::from_hex(*v));
        }
    }
    None
}

// (name, 0xRRGGBB). Source: CSS Color Module Level 3 §4.3 named colors.
// Kept as data so the test can assert a handful of canonical values.
const NAMED: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("silver", 0xc0c0c0),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("white", 0xffffff),
    ("maroon", 0x800000),
    ("red", 0xff0000),
    ("purple", 0x800080),
    ("fuchsia", 0xff00ff),
    ("magenta", 0xff00ff),
    ("green", 0x008000),
    ("lime", 0x00ff00),
    ("olive", 0x808000),
    ("yellow", 0xffff00),
    ("navy", 0x000080),
    ("blue", 0x0000ff),
    ("teal", 0x008080),
    ("aqua", 0x00ffff),
    ("cyan", 0x00ffff),
    ("orange", 0xffa500),
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("blanchedalmond", 0xffebcd),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgrey", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("greenyellow", 0xadff2f),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgrey", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("oldlace", 0xfdf5e6),
    ("olivedrab", 0x6b8e23),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("rebeccapurple", 0x663399),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("whitesmoke", 0xf5f5f5),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Rgba8::from_hex(0x123456);
        assert_eq!((c.r, c.g, c.b, c.a), (0x12, 0x34, 0x56, 255));
    }

    #[test]
    fn named_lookup_case_insensitive() {
        assert_eq!(named_color("red"), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(named_color("RED"), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(named_color("ReD"), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(named_color("rebeccapurple"), Some(Rgba8::rgb(0x66, 0x33, 0x99)));
        assert_eq!(named_color("notacolor"), None);
        assert_eq!(named_color(""), None);
    }

    #[test]
    fn premultiply() {
        let c = Rgba8::rgba(255, 0, 0, 128);
        let p = c.premultiplied();
        // 255 * 128 / 255 = 128
        assert_eq!(&p[..3], &[128, 0, 0]);
        assert_eq!(p[3], 128);
    }

    #[test]
    fn source_over_opaque_replaces() {
        let dst = Rgba8::rgb(0, 0, 255);
        assert_eq!(Rgba8::rgb(255, 0, 0).source_over(dst), Rgba8::rgb(255, 0, 0));
    }

    #[test]
    fn source_over_half_red_on_blue() {
        // 128/255 alpha red over opaque blue: roughly (128, 0, 127)
        let dst = Rgba8::rgb(0, 0, 255);
        let out = Rgba8::rgba(255, 0, 0, 128).source_over(dst);
        assert!(out.r >= 127 && out.r <= 129, "r = {}", out.r);
        assert_eq!(out.g, 0);
        assert!(out.b >= 127 && out.b <= 128, "b = {}", out.b);
        assert_eq!(out.a, 255);
    }

    #[test]
    fn source_over_transparent_is_noop() {
        let dst = Rgba8::rgb(1, 2, 3);
        assert_eq!(Rgba8::TRANSPARENT.source_over(dst), dst);
    }
}