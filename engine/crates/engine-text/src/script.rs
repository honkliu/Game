//! Which scripts we cannot shape (§9.2), used to warn in the render report.
//!
//! The check is by code-point range, not by font coverage: Segoe UI *has*
//! Hebrew and Arabic glyphs, so a coverage test would stay silent while the
//! text renders in logical order with isolated letterforms — visibly wrong.

/// The script a code point belongs to, when we cannot shape it.
pub fn script_warning(c: char) -> Option<&'static str> {
    Some(match c as u32 {
        0x0590..=0x05FF => "Hebrew (no bidi, no shaping)",
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF => "Arabic (no bidi, no shaping)",
        0x0900..=0x097F => "Devanagari (no shaping)",
        0x0980..=0x09FF => "Bengali (no shaping)",
        0x0B80..=0x0BFF => "Tamil (no shaping)",
        0x0E00..=0x0E7F => "Thai (no shaping)",
        0x1000..=0x109F => "Myanmar (no shaping)",
        0x1780..=0x17FF => "Khmer (no shaping)",
        _ => return None,
    })
}

/// A code point in a script we render with advance-summed positioning only.
pub fn is_unshaped_script(c: char) -> bool {
    script_warning(c).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_and_cjk_are_not_flagged() {
        for c in ['a', 'Z', 'é', 'Я', '日', '한'] {
            assert_eq!(script_warning(c), None, "{c:?}");
        }
    }

    #[test]
    fn flagged_scripts_are_named() {
        assert_eq!(script_warning('\u{05D0}'), Some("Hebrew (no bidi, no shaping)"));
        assert_eq!(script_warning('\u{0627}'), Some("Arabic (no bidi, no shaping)"));
        assert_eq!(script_warning('\u{0915}'), Some("Devanagari (no shaping)"));
    }
}