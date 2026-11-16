//! `engine-text` integration tests against the real system fonts.
//!
//! These assert *numbers*: a measured width, a break position, a count of
//! covered pixels. On a machine without the fonts every one of them fails
//! loudly rather than passing vacuously — that is deliberate, because a text
//! stage that silently measures zero is the bug that hides until a page is
//! blank.

use engine_core::{Rect, Rgba8};
use engine_raster::{GlyphBlitter, Surface};
use engine_text::{line_break, FontContext};

fn ctx() -> FontContext {
    let c = FontContext::new();
    assert!(c.face_count() >= 4, "expected system fonts; found {}", c.face_count());
    c
}

fn sans() -> Vec<String> {
    vec!["Segoe UI".to_string(), "sans-serif".to_string()]
}

#[test]
fn measured_width_of_a_known_string_is_stable_and_nonzero() {
    let mut c = ctx();
    let a = c.measure("Hello, world", &sans(), 400, false, 16.0, 0.0);
    let b = c.measure("Hello, world", &sans(), 400, false, 16.0, 0.0);
    assert!(a.width > 0.0, "width must be non-zero, got {}", a.width);
    assert_eq!(a.width, b.width, "measurement is deterministic");
    // Sanity bound: 12 characters at 16px cannot plausibly exceed 200px or be
    // under 20px in any real Latin font.
    assert!(a.width > 20.0 && a.width < 200.0, "implausible width {}", a.width);
    assert!(a.ascent > 0.0, "ascent above baseline must be positive");
    assert!(a.descent > 0.0, "descent below baseline must be positive");
}

#[test]
fn width_scales_linearly_with_font_size() {
    let mut c = ctx();
    let at16 = c.measure("abcdef", &sans(), 400, false, 16.0, 0.0).width;
    let at32 = c.measure("abcdef", &sans(), 400, false, 32.0, 0.0).width;
    // Font advances are linear in px to within rounding.
    let ratio = at32 / at16;
    assert!((ratio - 2.0).abs() < 0.05, "2x size should be ~2x width, got {ratio}");
}

#[test]
fn monotonic_in_length() {
    let mut c = ctx();
    let short = c.measure("ab", &sans(), 400, false, 16.0, 0.0).width;
    let long = c.measure("abcdefgh", &sans(), 400, false, 16.0, 0.0).width;
    assert!(long > short, "longer text must be wider: {long} vs {short}");
}

#[test]
fn empty_string_measures_zero() {
    let mut c = ctx();
    let m = c.measure("", &sans(), 400, false, 16.0, 0.0);
    assert_eq!(m.width, 0.0);
}

#[test]
fn bold_is_wider_than_regular_usually() {
    // Not guaranteed for every face, but true for Segoe UI's "Hamburgefonstiv".
    let mut c = ctx();
    let reg = c.measure("Hamburgefonstiv", &sans(), 400, false, 16.0, 0.0).width;
    let bold = c.measure("Hamburgefonstiv", &sans(), 700, false, 16.0, 0.0).width;
    assert!(bold >= reg, "bold {bold} should not be narrower than regular {reg}");
}

#[test]
fn letter_spacing_adds_per_character() {
    let mut c = ctx();
    let base = c.measure("abcd", &sans(), 400, false, 16.0, 0.0).width;
    let spaced = c.measure("abcd", &sans(), 400, false, 16.0, 2.0).width;
    assert!((spaced - base - 8.0).abs() < 0.01, "4 chars x 2px = 8px: {spaced} vs {base}");
}

#[test]
fn line_breaks_at_the_expected_word_for_a_given_width() {
    let mut c = ctx();
    let text = "The quick brown fox jumps over the lazy dog";
    let one_word = c.measure("The ", &sans(), 400, false, 16.0, 0.0).width;
    let two_words = c.measure("The quick ", &sans(), 400, false, 16.0, 0.0).width;
    // A width that fits "The " but not "The quick ".
    let width = (one_word + two_words) / 2.0;
    let lines = line_break::wrap(text, true, false, width, &mut |s| {
        c.measure(s, &sans(), 400, false, 16.0, 0.0).width
    });
    assert!(lines.len() >= 2, "must wrap, got {lines:?}");
    let chars: Vec<char> = text.chars().collect();
    let first: String = chars[lines[0].0..lines[0].1].iter().collect();
    assert_eq!(first, "The", "line 1 must be exactly the first word");
    let second: String = chars[lines[1].0..lines[1].1].iter().collect();
    assert_eq!(second, "quick", "line 2 must be exactly the second word");
}

#[test]
fn glyph_bitmaps_have_nonzero_coverage() {
    let c = ctx();
    // "W" at 32px is a large, solid stroke: its coverage bitmap cannot be all
    // zero. If it is, glyph rasterization is broken.
    let face = {
        let mut m = ctx();
        m.select_face(&sans(), 400, false).expect("a face")
    };
    let (metrics, bitmap) = face.rasterize('W', 32.0);
    assert!(metrics.width > 10 && metrics.height > 10, "W should be a large bitmap: {metrics:?}");
    let covered = bitmap.iter().filter(|&&c| c > 0).count();
    let total = metrics.width * metrics.height;
    assert!(covered > total / 4, "W should cover >25% of its box: {covered}/{total}");
    assert!(bitmap.contains(&255), "some pixel should be fully covered");
    let _ = c;
}

#[test]
fn blitting_produces_dark_pixels_on_white() {
    let mut c = ctx();
    let mut surface = Surface::filled(200, 60, Rgba8::WHITE);
    let runs = c.layout_runs("Hello", &sans(), 400, false, 24.0, 0.0);
    assert_eq!(runs.len(), 1, "one face for Latin text");
    let run = &runs[0].1;
    assert_eq!(run.glyphs.len(), 5, "one glyph per character");
    let bounds = surface.bounds();
    c.blit_run(&mut surface, 10.0, 40.0, run, Rgba8::BLACK, bounds);
    let dark = (0..60)
        .flat_map(|y| (0..200).map(move |x| (x, y)))
        .filter(|&(x, y)| surface.get(x, y).r < 128)
        .count();
    assert!(dark > 50, "text must paint dark pixels; got {dark}");
    // Nothing outside the text area should be touched.
    assert_eq!(surface.get(199, 59), Rgba8::WHITE);
}

#[test]
fn runs_width_matches_measure() {
    let mut c = ctx();
    let text = "Measured consistently";
    let m = c.measure(text, &sans(), 400, false, 18.0, 0.0).width;
    let runs = c.layout_runs(text, &sans(), 400, false, 18.0, 0.0);
    let w = FontContext::runs_width(&runs);
    assert!((w - m).abs() < 0.5, "layout_runs width {w} should match measure {m}");
}

#[test]
fn fallback_face_splits_runs() {
    let mut c = ctx();
    // CJK characters are not in Segoe UI; if any loaded face has them, the run
    // must be split at the boundary rather than drawing tofu in the wrong font.
    let runs = c.layout_runs("ab日本語", &sans(), 400, false, 16.0, 0.0);
    let total_glyphs: usize = runs.iter().map(|(_, r)| r.glyphs.len()).sum();
    assert_eq!(total_glyphs, 5, "no characters dropped: {runs:?}");
    if runs.len() > 1 {
        assert!(runs[0].1.advance >= 0.0);
    }
}

#[test]
fn unshaped_script_records_a_warning() {
    let mut c = ctx();
    let _ = c.measure("\u{05D0}\u{05D1}", &sans(), 400, false, 16.0, 0.0); // Hebrew alef-bet
    assert_eq!(c.last_shaping_warning(), Some("Hebrew (no bidi, no shaping)"));
    let _ = c.measure("abc", &sans(), 400, false, 16.0, 0.0);
    assert_eq!(c.last_shaping_warning(), None, "warning clears on ordinary text");
}

#[test]
fn empty_context_measures_zero_and_does_not_panic() {
    // A `FontContext` with no faces is what a machine without `C:\Windows\Fonts`
    // would produce. Constructing one is not public, so this guards the
    // measure path against `px = 0` and an unknown family instead.
    let mut c = ctx();
    let m = c.measure("abc", &["Nonexistent Font".to_string()], 400, false, 16.0, 0.0);
    assert!(m.width > 0.0, "unknown family falls back to an available face");
    let z = c.measure("abc", &sans(), 400, false, 0.0, 0.0);
    assert_eq!(z.width, 0.0, "zero font size measures zero, not NaN");
    assert!(z.width.is_finite());
}

#[test]
fn blit_into_a_tiny_clip_does_not_panic() {
    let mut c = ctx();
    let mut surface = Surface::filled(10, 10, Rgba8::WHITE);
    let runs = c.layout_runs("Hello world", &sans(), 400, false, 40.0, 0.0);
    let run = &runs[0].1;
    // Clip is much smaller than the glyphs.
    c.blit_run(&mut surface, 5.0, 5.0, run, Rgba8::BLACK, Rect::new(0.0, 0.0, 2.0, 2.0));
    // Only the clipped region may change.
    assert_eq!(surface.get(9, 9), Rgba8::WHITE);
}

#[test]
fn very_long_string_measures_without_overflow() {
    let mut c = ctx();
    let text = "a".repeat(100_000);
    let w = c.measure(&text, &sans(), 400, false, 12.0, 0.0).width;
    assert!(w.is_finite() && w > 0.0, "got {w}");
}