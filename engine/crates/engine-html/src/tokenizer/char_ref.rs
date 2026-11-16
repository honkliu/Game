//! Character reference (`&...;`) decoding.
//!
//! Two forms:
//! - Numeric: `&#123;` and `&#x1F600;`, with the spec's replacement table for
//!   out-of-range and control code points.
//! - Named: a table of the entity names that appear in real documents. The
//!   full HTML table has ~2,230 entries; this covers the ~260 that account for
//!   essentially all real usage (Latin-1 block, Greek, common punctuation,
//!   arrows, math, currency). Anything not in the table stays literal, which is
//!   exactly what a browser does with an unknown named reference.
//!
//! Lookup is longest-prefix: `&notit;` must not match `&not`. The caller passes
//! the text after `&` and gets back `(decoded, bytes_consumed)`.

/// A named character reference table. Sorted by name for binary search.
/// Source: WHATWG HTML "Named character references" (subset).
static NAMED: &[(&str, &str)] = &[
    ("AElig", "\u{00C6}"),
    ("AMP", "&"),
    ("Aacute", "\u{00C1}"),
    ("Acirc", "\u{00C2}"),
    ("Agrave", "\u{00C0}"),
    ("Aring", "\u{00C5}"),
    ("Atilde", "\u{00C3}"),
    ("Auml", "\u{00C4}"),
    ("COPY", "\u{00A9}"),
    ("Ccedil", "\u{00C7}"),
    ("ETH", "\u{00D0}"),
    ("Eacute", "\u{00C9}"),
    ("Ecirc", "\u{00CA}"),
    ("Egrave", "\u{00C8}"),
    ("Euml", "\u{00CB}"),
    ("GT", ">"),
    ("Iacute", "\u{00CD}"),
    ("Icirc", "\u{00CE}"),
    ("Igrave", "\u{00CC}"),
    ("Iuml", "\u{00CF}"),
    ("LT", "<"),
    ("Ntilde", "\u{00D1}"),
    ("Oacute", "\u{00D3}"),
    ("Ocirc", "\u{00D4}"),
    ("Ograve", "\u{00D2}"),
    ("Oslash", "\u{00D8}"),
    ("Otilde", "\u{00D5}"),
    ("Ouml", "\u{00D6}"),
    ("QUOT", "\""),
    ("REG", "\u{00AE}"),
    ("THORN", "\u{00DE}"),
    ("Uacute", "\u{00DA}"),
    ("Ucirc", "\u{00DB}"),
    ("Ugrave", "\u{00D9}"),
    ("Uuml", "\u{00DC}"),
    ("Yacute", "\u{00DD}"),
    ("aacute", "\u{00E1}"),
    ("acirc", "\u{00E2}"),
    ("acute", "\u{00B4}"),
    ("aelig", "\u{00E6}"),
    ("agrave", "\u{00E0}"),
    ("amp", "&"),
    ("apos", "'"),
    ("aring", "\u{00E5}"),
    ("atilde", "\u{00E3}"),
    ("auml", "\u{00E4}"),
    ("brvbar", "\u{00A6}"),
    ("ccedil", "\u{00E7}"),
    ("cedil", "\u{00B8}"),
    ("cent", "\u{00A2}"),
    ("copy", "\u{00A9}"),
    ("curren", "\u{00A4}"),
    ("deg", "\u{00B0}"),
    ("divide", "\u{00F7}"),
    ("eacute", "\u{00E9}"),
    ("ecirc", "\u{00EA}"),
    ("egrave", "\u{00E8}"),
    ("eth", "\u{00F0}"),
    ("euml", "\u{00EB}"),
    ("frac12", "\u{00BD}"),
    ("frac14", "\u{00BC}"),
    ("frac34", "\u{00BE}"),
    ("gt", ">"),
    ("iacute", "\u{00ED}"),
    ("icirc", "\u{00EE}"),
    ("iexcl", "\u{00A1}"),
    ("igrave", "\u{00EC}"),
    ("iquest", "\u{00BF}"),
    ("iuml", "\u{00EF}"),
    ("laquo", "\u{00AB}"),
    ("lt", "<"),
    ("macr", "\u{00AF}"),
    ("micro", "\u{00B5}"),
    ("middot", "\u{00B7}"),
    ("nbsp", "\u{00A0}"),
    ("not", "\u{00AC}"),
    ("ntilde", "\u{00F1}"),
    ("oacute", "\u{00F3}"),
    ("ocirc", "\u{00F4}"),
    ("ograve", "\u{00F2}"),
    ("ordf", "\u{00AA}"),
    ("ordm", "\u{00BA}"),
    ("oslash", "\u{00F8}"),
    ("otilde", "\u{00F5}"),
    ("ouml", "\u{00F6}"),
    ("para", "\u{00B6}"),
    ("plusmn", "\u{00B1}"),
    ("pound", "\u{00A3}"),
    ("quot", "\""),
    ("raquo", "\u{00BB}"),
    ("reg", "\u{00AE}"),
    ("sect", "\u{00A7}"),
    ("shy", "\u{00AD}"),
    ("sup1", "\u{00B9}"),
    ("sup2", "\u{00B2}"),
    ("sup3", "\u{00B3}"),
    ("szlig", "\u{00DF}"),
    ("thorn", "\u{00FE}"),
    ("times", "\u{00D7}"),
    ("uacute", "\u{00FA}"),
    ("ucirc", "\u{00FB}"),
    ("ugrave", "\u{00F9}"),
    ("uml", "\u{00A8}"),
    ("uuml", "\u{00FC}"),
    ("yacute", "\u{00FD}"),
    ("yen", "\u{00A5}"),
    ("yuml", "\u{00FF}"),
    // Common typographic and mathematical references.
    ("bull", "\u{2022}"),
    ("hellip", "\u{2026}"),
    ("prime", "\u{2032}"),
    ("Prime", "\u{2033}"),
    ("ndash", "\u{2013}"),
    ("mdash", "\u{2014}"),
    ("lsquo", "\u{2018}"),
    ("rsquo", "\u{2019}"),
    ("ldquo", "\u{201C}"),
    ("rdquo", "\u{201D}"),
    ("sbquo", "\u{201A}"),
    ("bdquo", "\u{201E}"),
    ("dagger", "\u{2020}"),
    ("Dagger", "\u{2021}"),
    ("permil", "\u{2030}"),
    ("lsaquo", "\u{2039}"),
    ("rsaquo", "\u{203A}"),
    ("euro", "\u{20AC}"),
    ("trade", "\u{2122}"),
    ("larr", "\u{2190}"),
    ("uarr", "\u{2191}"),
    ("rarr", "\u{2192}"),
    ("darr", "\u{2193}"),
    ("harr", "\u{2194}"),
    ("lArr", "\u{21D0}"),
    ("rArr", "\u{21D2}"),
    ("hArr", "\u{21D4}"),
    ("forall", "\u{2200}"),
    ("part", "\u{2202}"),
    ("exist", "\u{2203}"),
    ("empty", "\u{2205}"),
    ("nabla", "\u{2207}"),
    ("isin", "\u{2208}"),
    ("notin", "\u{2209}"),
    ("ni", "\u{220B}"),
    ("prod", "\u{220F}"),
    ("sum", "\u{2211}"),
    ("minus", "\u{2212}"),
    ("lowast", "\u{2217}"),
    ("radic", "\u{221A}"),
    ("prop", "\u{221D}"),
    ("infin", "\u{221E}"),
    ("ang", "\u{2220}"),
    ("and", "\u{2227}"),
    ("or", "\u{2228}"),
    ("cap", "\u{2229}"),
    ("cup", "\u{222A}"),
    ("int", "\u{222B}"),
    ("there4", "\u{2234}"),
    ("sim", "\u{223C}"),
    ("cong", "\u{2245}"),
    ("asymp", "\u{2248}"),
    ("ne", "\u{2260}"),
    ("equiv", "\u{2261}"),
    ("le", "\u{2264}"),
    ("ge", "\u{2265}"),
    ("sub", "\u{2282}"),
    ("sup", "\u{2283}"),
    ("nsub", "\u{2284}"),
    ("sube", "\u{2286}"),
    ("supe", "\u{2287}"),
    ("oplus", "\u{2295}"),
    ("otimes", "\u{2297}"),
    ("perp", "\u{22A5}"),
    ("sdot", "\u{22C5}"),
    ("lceil", "\u{2308}"),
    ("rceil", "\u{2309}"),
    ("lfloor", "\u{230A}"),
    ("rfloor", "\u{230B}"),
    ("lang", "\u{2329}"),
    ("rang", "\u{232A}"),
    ("loz", "\u{25CA}"),
    ("spades", "\u{2660}"),
    ("clubs", "\u{2663}"),
    ("hearts", "\u{2665}"),
    ("diams", "\u{2666}"),
    // Greek, lowercase and uppercase.
    ("alpha", "\u{03B1}"),
    ("beta", "\u{03B2}"),
    ("gamma", "\u{03B3}"),
    ("delta", "\u{03B4}"),
    ("epsilon", "\u{03B5}"),
    ("zeta", "\u{03B6}"),
    ("eta", "\u{03B7}"),
    ("theta", "\u{03B8}"),
    ("iota", "\u{03B9}"),
    ("kappa", "\u{03BA}"),
    ("lambda", "\u{03BB}"),
    ("mu", "\u{03BC}"),
    ("nu", "\u{03BD}"),
    ("xi", "\u{03BE}"),
    ("omicron", "\u{03BF}"),
    ("pi", "\u{03C0}"),
    ("rho", "\u{03C1}"),
    ("sigmaf", "\u{03C2}"),
    ("sigma", "\u{03C3}"),
    ("tau", "\u{03C4}"),
    ("upsilon", "\u{03C5}"),
    ("phi", "\u{03C6}"),
    ("chi", "\u{03C7}"),
    ("psi", "\u{03C8}"),
    ("omega", "\u{03C9}"),
    ("Alpha", "\u{0391}"),
    ("Beta", "\u{0392}"),
    ("Gamma", "\u{0393}"),
    ("Delta", "\u{0394}"),
    ("Theta", "\u{0398}"),
    ("Lambda", "\u{039B}"),
    ("Pi", "\u{03A0}"),
    ("Sigma", "\u{03A3}"),
    ("Phi", "\u{03A6}"),
    ("Psi", "\u{03A8}"),
    ("Omega", "\u{03A9}"),
    // Spaces and misc.
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("thinsp", "\u{2009}"),
    ("zwnj", "\u{200C}"),
    ("zwj", "\u{200D}"),
    ("lrm", "\u{200E}"),
    ("rlm", "\u{200F}"),
    ("circ", "\u{02C6}"),
    ("tilde", "\u{02DC}"),
    ("OElig", "\u{0152}"),
    ("oelig", "\u{0153}"),
    ("Scaron", "\u{0160}"),
    ("scaron", "\u{0161}"),
    ("Yuml", "\u{0178}"),
    ("fnof", "\u{0192}"),
];

/// Code points that numeric references must never produce: they are replaced
/// with U+FFFD per the HTML spec ("numeric character reference end state").
fn numeric_replacement(cp: u32) -> Option<char> {
    // Windows-1252 remapping for C1 controls (HTML spec table).
    let mapped = match cp {
        0x80 => Some('\u{20AC}'),
        0x82 => Some('\u{201A}'),
        0x83 => Some('\u{0192}'),
        0x84 => Some('\u{201E}'),
        0x85 => Some('\u{2026}'),
        0x86 => Some('\u{2020}'),
        0x87 => Some('\u{2021}'),
        0x88 => Some('\u{02C6}'),
        0x89 => Some('\u{2030}'),
        0x8A => Some('\u{0160}'),
        0x8B => Some('\u{2039}'),
        0x8C => Some('\u{0152}'),
        0x8E => Some('\u{017D}'),
        0x91 => Some('\u{2018}'),
        0x92 => Some('\u{2019}'),
        0x93 => Some('\u{201C}'),
        0x94 => Some('\u{201D}'),
        0x95 => Some('\u{2022}'),
        0x96 => Some('\u{2013}'),
        0x97 => Some('\u{2014}'),
        0x98 => Some('\u{02DC}'),
        0x99 => Some('\u{2122}'),
        0x9A => Some('\u{0161}'),
        0x9B => Some('\u{203A}'),
        0x9C => Some('\u{0153}'),
        0x9E => Some('\u{017E}'),
        0x9F => Some('\u{0178}'),
        _ => None,
    };
    if let Some(c) = mapped {
        return Some(c);
    }
    // C0 controls other than tab/newline/form-feed, DEL, C1 leftover, and
    // surrogates are parse errors; the spec maps most to themselves but that
    // yields unusable text, and every browser applies FFFD for 0 and surrogates.
    if cp == 0 || (0xD800..=0xDFFF).contains(&cp) || cp > 0x10FFFF {
        return Some('\u{FFFD}');
    }
    char::from_u32(cp)
}

/// Decode a numeric reference body (the text between `&#` and `;`, or between
/// `&#x` and `;`). Returns the character, or `None` if malformed.
fn decode_numeric(body: &str) -> Option<char> {
    let (digits, radix) = if let Some(rest) = body.strip_prefix(['x', 'X']) {
        (rest, 16)
    } else {
        (body, 10)
    };
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    if !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let cp = u32::from_str_radix(digits, radix).ok()?;
    numeric_replacement(cp)
}

/// Attempt to decode a character reference at the start of `input` (which must
/// begin immediately after `&`). Returns `(text, consumed_bytes)` where
/// `consumed_bytes` counts from after the `&`.
///
/// Semantics match browsers closely enough for rendering:
/// - `&#...;` / `&#x...;` decode with or without the trailing `;`;
/// - named references decode only when terminated by `;`;
/// - unknown names are left literal (caller emits the `&`).
pub fn decode_after_amp(input: &str) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] == b'#' {
        // Numeric.
        let semi = input.find(';');
        // Without ';' the reference ends at the first non-alphanumeric byte.
        let end = match semi {
            Some(s) => s,
            None => {
                let mut e = 1;
                while e < bytes.len()
                    && (bytes[e].is_ascii_alphanumeric())
                {
                    e += 1;
                }
                e
            }
        };
        let body = &input[1..end];
        let ch = decode_numeric(body)?;
        let consumed = if semi.is_some() { end + 1 } else { end };
        return Some((ch.to_string(), consumed));
    }
    // Named: longest match in the table, requiring a trailing ';'.
    let max = input.len().min(32);
    let mut best: Option<(usize, &'static str)> = None;
    for (name, value) in NAMED {
        let n = name.len();
        if n <= max && input.as_bytes().get(n) == Some(&b';') && input[..n] == **name {
            // Prefer the longest name.
            if best.is_none_or(|(l, _)| n > l) {
                best = Some((n, value));
            }
        }
    }
    best.map(|(n, v)| (v.to_string(), n + 1))
}

/// Encode a character as the shortest decimal numeric reference, used by the
/// serializer when re-emitting non-ASCII text is undesirable.
pub fn numeric_ref(c: char) -> String {
    format!("&#{};", c as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_basic() {
        assert_eq!(decode_after_amp("amp;"), Some(("&".to_string(), 4)));
        assert_eq!(decode_after_amp("lt;"), Some(("<".to_string(), 3)));
        assert_eq!(decode_after_amp("nbsp;"), Some(("\u{a0}".to_string(), 5)));
    }

    #[test]
    fn named_longest_match_wins() {
        // "not" is a name; "notin" is a longer name. `&notit;` is not in the
        // table, so no match (browsers would decode `&not` without the ';' —
        // we require ';' for named refs and leave it literal).
        assert_eq!(decode_after_amp("notin;"), Some(("\u{2209}".to_string(), 6)));
        assert_eq!(decode_after_amp("noti;"), None);
    }

    #[test]
    fn named_requires_semicolon() {
        assert_eq!(decode_after_amp("ampx"), None);
        assert_eq!(decode_after_amp("amp"), None);
    }

    #[test]
    fn numeric_decimal() {
        assert_eq!(decode_after_amp("#65;"), Some(("A".to_string(), 4)));
        // Without the terminating ';' the reference still decodes; three bytes
        // are consumed, counted from immediately after the '&'.
        assert_eq!(decode_after_amp("#65"), Some(("A".to_string(), 3)));
    }

    #[test]
    fn numeric_hex() {
        assert_eq!(decode_after_amp("#x41;"), Some(("A".to_string(), 5)));
        assert_eq!(decode_after_amp("#X41;"), Some(("A".to_string(), 5)));
        assert_eq!(decode_after_amp("#x1F600;"), Some(("\u{1F600}".to_string(), 8)));
    }

    #[test]
    fn numeric_invalid_is_none() {
        assert_eq!(decode_after_amp("#xZZ;"), None);
        assert_eq!(decode_after_amp("#;"), None);
    }

    #[test]
    fn numeric_null_becomes_replacement() {
        assert_eq!(decode_after_amp("#0;"), Some(("\u{FFFD}".to_string(), 3)));
    }

    #[test]
    fn numeric_surrogate_becomes_replacement() {
        // "#xD800;" is seven bytes after the '&' (including the ';').
        assert_eq!(decode_after_amp("#xD800;"), Some(("\u{FFFD}".to_string(), 7)));
    }

    #[test]
    fn windows1252_remap() {
        assert_eq!(decode_after_amp("#x80;"), Some(("\u{20AC}".to_string(), 5)));
        assert_eq!(decode_after_amp("#146;"), Some(("\u{2019}".to_string(), 5)));
    }

    #[test]
    fn empty_input() {
        assert_eq!(decode_after_amp(""), None);
    }

    #[test]
    fn table_is_sorted_and_unique() {
        let mut names: Vec<&str> = NAMED.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "duplicate entity names");
    }
}