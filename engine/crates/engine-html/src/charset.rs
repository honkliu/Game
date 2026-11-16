//! Byte-to-text decoding.
//!
//! Encoding detection is deliberately narrow: BOM, then `<meta charset>` in the
//! first 1024 bytes, then UTF-8. We support UTF-8 and the Windows-1252 range
//! (which is what "Latin-1" in the wild almost always means). Anything else is
//! decoded as UTF-8 with replacement characters, and the caller records the
//! approximation.

/// The encodings this engine decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Windows1252,
}

/// Reasons a decode was lossy, for the render report.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DecodeReport {
    /// The encoding actually used.
    pub encoding: Option<Encoding>,
    /// How it was chosen.
    pub source: &'static str,
    /// Number of U+FFFD replacements produced.
    pub replacements: usize,
}

impl DecodeReport {
    pub fn is_lossless(&self) -> bool {
        self.replacements == 0
    }
}

/// Decode a document's bytes to text, detecting the encoding.
pub fn decode(bytes: &[u8]) -> (String, DecodeReport) {
    // 1. BOM.
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let (s, n) = lossy_utf8(rest);
        return (s, DecodeReport { encoding: Some(Encoding::Utf8), source: "bom", replacements: n });
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        // UTF-16: not supported. Decode as UTF-8 so the failure is visible, and
        // report it, rather than guessing.
        let (s, n) = lossy_utf8(bytes);
        return (
            s,
            DecodeReport { encoding: Some(Encoding::Utf8), source: "utf16-unsupported", replacements: n },
        );
    }

    // 2. `<meta charset>` in the first 1024 bytes.
    let head_len = bytes.len().min(1024);
    if let Some(enc) = sniff_meta_charset(&bytes[..head_len]) {
        return match enc {
            Encoding::Utf8 => {
                let (s, n) = lossy_utf8(bytes);
                (s, DecodeReport { encoding: Some(Encoding::Utf8), source: "meta", replacements: n })
            }
            Encoding::Windows1252 => {
                (decode_1252(bytes), DecodeReport { encoding: Some(Encoding::Windows1252), source: "meta", replacements: 0 })
            }
        };
    }

    // 3. UTF-8, with a strict attempt first.
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), DecodeReport { encoding: Some(Encoding::Utf8), source: "default", replacements: 0 }),
        Err(_) => {
            // Invalid UTF-8 with no declaration: assume Windows-1252, which is
            // what browsers do for unlabelled legacy content and is far more
            // likely to produce readable text than replacement characters.
            (decode_1252(bytes), DecodeReport { encoding: Some(Encoding::Windows1252), source: "default-invalid-utf8", replacements: 0 })
        }
    }
}

fn lossy_utf8(bytes: &[u8]) -> (String, usize) {
    let mut out = String::with_capacity(bytes.len());
    let mut replacements = 0;
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            // One replacement per maximal invalid subsequence.
            out.push('\u{FFFD}');
            replacements += 1;
        }
    }
    (out, replacements)
}

/// Windows-1252: Latin-1 except 0x80..=0x9F, which map to typographic
/// characters. Undefined bytes in that range become the C1 control of the same
/// value, matching the WHATWG index for windows-1252.
fn decode_1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
        '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
        '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
        '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x00..=0x7F => out.push(b as char),
            0x80..=0x9F => out.push(HIGH[(b - 0x80) as usize]),
            _ => out.push(b as char),
        }
    }
    out
}

/// Find `<meta charset=...>` or `<meta http-equiv="content-type" content="...charset=...">`.
fn sniff_meta_charset(head: &[u8]) -> Option<Encoding> {
    let hay = String::from_utf8_lossy(head).to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(pos) = hay[search_from..].find("<meta") {
        let start = search_from + pos;
        let end = hay[start..].find('>').map(|e| start + e).unwrap_or(hay.len());
        let tag = &hay[start..end];
        if let Some(enc) = charset_from_tag(tag) {
            return Some(enc);
        }
        search_from = end;
    }
    None
}

fn charset_from_tag(tag: &str) -> Option<Encoding> {
    // charset="..." or charset=...
    if let Some(i) = tag.find("charset") {
        let rest = &tag[i + 7..];
        let rest = rest.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
        let value: String = rest
            .trim_start_matches(['"', '\''])
            .chars()
            .take_while(|c| *c != '"' && *c != '\'' && *c != ';' && *c != ' ' && *c != '>')
            .collect();
        return map_label(&value);
    }
    None
}

/// Map a charset label to a supported encoding. Unknown labels return `None`
/// so the caller falls through to the default.
pub fn map_label(label: &str) -> Option<Encoding> {
    let l = label.trim().to_ascii_lowercase();
    match l.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some(Encoding::Utf8),
        "windows-1252" | "cp1252" | "x-cp1252" | "iso-8859-1" | "iso8859-1" | "latin1" | "us-ascii"
        | "ascii" | "iso_8859-1" => Some(Encoding::Windows1252),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8() {
        let (s, r) = decode("héllo 世界".as_bytes());
        assert_eq!(s, "héllo 世界");
        assert!(r.is_lossless());
        assert_eq!(r.source, "default");
    }

    #[test]
    fn utf8_bom_stripped() {
        let mut b = vec![0xEF, 0xBB, 0xBF];
        b.extend_from_slice(b"hi");
        let (s, r) = decode(&b);
        assert_eq!(s, "hi");
        assert_eq!(r.source, "bom");
    }

    #[test]
    fn meta_charset_1252() {
        let html = b"<html><head><meta charset=\"windows-1252\"></head><body>\xE9</body></html>";
        let (s, r) = decode(html);
        assert!(s.contains('é'), "0xE9 in windows-1252 is e-acute");
        assert_eq!(r.encoding, Some(Encoding::Windows1252));
        assert_eq!(r.source, "meta");
    }

    #[test]
    fn meta_charset_unquoted() {
        let html = b"<meta charset=utf-8><body>hi</body>";
        let (_, r) = decode(html);
        assert_eq!(r.encoding, Some(Encoding::Utf8));
    }

    #[test]
    fn invalid_utf8_without_declaration_falls_back_to_1252() {
        let bytes = b"caf\xE9";
        let (s, r) = decode(bytes);
        assert_eq!(s, "café");
        assert_eq!(r.encoding, Some(Encoding::Windows1252));
    }

    #[test]
    fn windows_1252_high_range() {
        // 0x93/0x94 are curly double quotes in windows-1252.
        let (s, _) = decode(b"\x93hi\x94");
        assert_eq!(s, "\u{201C}hi\u{201D}");
    }

    #[test]
    fn label_mapping() {
        assert_eq!(map_label("UTF-8"), Some(Encoding::Utf8));
        assert_eq!(map_label("latin1"), Some(Encoding::Windows1252));
        assert_eq!(map_label("shift_jis"), None);
    }

    #[test]
    fn meta_beyond_first_1024_not_sniffed() {
        let mut html = vec![b' '; 2000];
        html.extend_from_slice(b"<meta charset=windows-1252>");
        let (_, r) = decode(&html);
        assert_eq!(r.source, "default", "only the first 1024 bytes are sniffed");
    }
}