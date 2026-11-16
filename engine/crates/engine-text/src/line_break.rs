//! Line breaking (§9.3, UAX #14 subset).
//!
//! Rules implemented, in order of priority:
//! 1. A mandatory break after every `\n` (`white-space: pre`-family).
//! 2. An optional break at a space, when `white-space` allows wrapping.
//! 3. An optional break after a hyphen and an en/em dash.
//! 4. An optional break before and after CJK ideographs (UAX #14 class ID),
//!    which is what makes Chinese and Japanese wrap at all.
//!
//! Not implemented: hyphenation dictionaries, the `line-break`/`word-break`
//! properties, and the full UAX #14 pair table. §15 lists hyphenation as
//! unsupported.
//!
//! All indices are **character** indices, which is what inline flow needs to
//! split a text run; byte slicing happens once, when a split is taken.
//!
//! **Termination.** `wrap`'s loop always advances `start` by at least one: every
//! branch that computes `end` clamps it to `start + 1` when the preferred break
//! would not advance. The tests cover zero width and a single giant word.

use crate::context::BreakOpportunity;

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF       // Hangul Jamo
        | 0x2E80..=0x2FFF     // CJK radicals, Kangxi
        | 0x3000..=0x303F     // CJK punctuation
        | 0x3040..=0x30FF     // Hiragana, Katakana
        | 0x3400..=0x4DBF     // CJK Ext A
        | 0x4E00..=0x9FFF     // CJK Unified
        | 0xA000..=0xA4CF     // Yi
        | 0xAC00..=0xD7AF     // Hangul syllables
        | 0xF900..=0xFAFF     // CJK compat
        | 0xFF00..=0xFF60     // Fullwidth forms
    )
}

/// All break opportunities in `text` as character indices, in increasing order.
///
/// An opportunity at index `i` means "a line may end just before character
/// `i`". The result never contains `0` (a break before the first character is
/// useless) and never contains `len`.
pub fn break_opportunities(text: &str, allow_wrap: bool, preserves_newlines: bool) -> Vec<BreakOpportunity> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<BreakOpportunity> = Vec::new();
    let n = chars.len();

    for i in 0..n {
        let c = chars[i];
        if c == '\n' && preserves_newlines {
            if i + 1 < n {
                out.push(BreakOpportunity { at: i + 1, mandatory: true });
            }
            continue;
        }
        if !allow_wrap || i + 1 >= n {
            continue;
        }
        // Break after the last space of a run, so the following word starts
        // the next line.
        if c == ' ' && chars[i + 1] != ' ' {
            out.push(BreakOpportunity { at: i + 1, mandatory: false });
            continue;
        }
        // Break after a hyphen or dash, keeping the dash on the line it belongs
        // to.
        if matches!(c, '-' | '\u{2013}' | '\u{2014}') {
            out.push(BreakOpportunity { at: i + 1, mandatory: false });
            continue;
        }
        // Break before and after a CJK ideograph.
        if is_cjk(chars[i + 1]) || is_cjk(c) {
            out.push(BreakOpportunity { at: i + 1, mandatory: false });
        }
    }
    out
}

fn width_of(chars: &[char], start: usize, end: usize, measure: &mut dyn FnMut(&str) -> f32) -> f32 {
    if end <= start {
        return 0.0;
    }
    let s: String = chars[start..end].iter().collect();
    measure(&s)
}

/// Split `text` into lines that fit `max_width`, using `measure` to size a
/// candidate line. Returns the lines as `(char_start, char_end)` ranges, with
/// trailing spaces trimmed.
///
/// Greedy: take the longest prefix that fits, breaking at the latest
/// opportunity inside it. A mandatory `\n` ends a line regardless of width. If
/// nothing fits (a word wider than the line), the word still gets its own line
/// rather than breaking mid-word — except a single "word" wider than the whole
/// line, which breaks by character so the page is not a horizontal mile wide.
pub fn wrap(
    text: &str,
    allow_wrap: bool,
    preserves_newlines: bool,
    max_width: f32,
    measure: &mut dyn FnMut(&str) -> f32,
) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let ops = break_opportunities(text, allow_wrap, preserves_newlines);
    let max_width = max_width.max(0.0);
    let mut start = 0usize;

    while start < n {
        // A mandatory break ends the line unconditionally.
        if preserves_newlines {
            if let Some(pos) = chars[start..].iter().position(|&c| c == '\n') {
                let mut end = start + pos;
                while end > start && chars[end - 1] == ' ' {
                    end -= 1;
                }
                out.push((start, end));
                start = start + pos + 1;
                continue;
            }
        }

        if !allow_wrap || width_of(&chars, start, n, measure) <= max_width {
            let mut end = n;
            while end > start && chars[end - 1] == ' ' {
                end -= 1;
            }
            out.push((start, end));
            break;
        }

        // Find the last break opportunity at which the prefix fits.
        let mut best: Option<usize> = None;
        let mut first_op: Option<usize> = None;
        for op in &ops {
            if op.at <= start {
                continue;
            }
            if width_of(&chars, start, op.at, measure) <= max_width {
                first_op.get_or_insert(op.at);
                best = Some(op.at);
            } else {
                break;
            }
        }

        let mut end = match best {
            Some(b) => b,
            // No opportunity fits. Break mid-word by character so the line
            // makes progress; the loop is safe because `end` grows by ≥ 1.
            None => {
                let mut e = start + 1;
                while e < n && width_of(&chars, start, e + 1, measure) <= max_width && chars[e - 1] != '\n' {
                    e += 1;
                }
                e.max(start + 1)
            }
        };
        let _ = first_op;
        // Trim trailing spaces; they are consumed by the break.
        while end > start && chars[end - 1] == ' ' {
            end -= 1;
        }
        // Guarantee progress even if the line is entirely spaces.
        if end <= start {
            end = start + 1;
        }
        out.push((start, end));
        // Skip the spaces (or the single newline) at the start of the next line.
        let mut next = end;
        while next < n && chars[next] == ' ' {
            next += 1;
        }
        if next < n && chars[next] == '\n' && preserves_newlines {
            next += 1;
        }
        start = next.max(start + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic fake measurer: every char is 10px wide.
    fn ten(c: &str) -> f32 {
        c.chars().count() as f32 * 10.0
    }

    fn lines(text: &str, width: f32) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        wrap(text, true, false, width, &mut ten)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect()
    }

    #[test]
    fn single_word_no_break() {
        assert_eq!(lines("hello", 1000.0), vec!["hello"]);
    }

    #[test]
    fn breaks_at_space() {
        // "aaa bbb" is 70px; at 40px "aaa" (30) fits and the whole does not.
        assert_eq!(lines("aaa bbb", 40.0), vec!["aaa", "bbb"]);
    }

    #[test]
    fn breaks_twice() {
        // "aa bb cc" at 50px: "aa" (20) fits, "aa bb" (50) would fit but the
        // break opportunity trims to "aa", then "bb cc" (50) fits exactly.
        assert_eq!(lines("aa bb cc", 50.0), vec!["aa", "bb cc"]);
        // At 30px only one word per line fits.
        assert_eq!(lines("aa bb cc", 30.0), vec!["aa", "bb", "cc"]);
    }

    #[test]
    fn exactly_fitting_line_does_not_break() {
        // 6 chars = 60px at width 60.
        assert_eq!(lines("aa bb", 60.0), vec!["aa bb"]);
    }

    #[test]
    fn mandatory_newline() {
        let chars: Vec<char> = "ab\ncd".chars().collect();
        let got: Vec<String> = wrap("ab\ncd", true, true, 1000.0, &mut ten)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        assert_eq!(got, vec!["ab", "cd"]);
    }

    #[test]
    fn no_wrap_keeps_one_line() {
        let chars: Vec<char> = "aaa bbb".chars().collect();
        let got: Vec<String> = wrap("aaa bbb", false, false, 10.0, &mut ten)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        assert_eq!(got, vec!["aaa bbb"]);
    }

    #[test]
    fn oversized_word_gets_its_own_line_when_it_has_no_opportunities() {
        // "a bbbb" at 30px: "a" fits, "bbbb" is 40px. The break opportunity is
        // at index 2 (after the space), and the word cannot be split there, so
        // the word is broken by character.
        let out = lines("a bbbb", 30.0);
        assert_eq!(out.concat(), "abbbb", "no characters lost");
        assert!(out.len() >= 2, "got {out:?}");
    }

    #[test]
    fn long_word_breaks_by_character() {
        let out = lines("abcdefghij", 30.0);
        assert!(out.len() >= 3, "got {out:?}");
        assert_eq!(out.concat(), "abcdefghij", "no characters lost");
    }

    #[test]
    fn hyphen_is_a_break_opportunity() {
        assert_eq!(lines("well-known", 60.0), vec!["well-", "known"]);
    }

    #[test]
    fn cjk_breaks_between_ideographs() {
        // 5 ideographs at 10px each; width 30 holds exactly 3 per line.
        assert_eq!(lines("日本語です", 30.0), vec!["日本語", "です"]);
    }

    #[test]
    fn empty_text_is_no_lines() {
        assert!(lines("", 100.0).is_empty());
    }

    #[test]
    fn trailing_spaces_do_not_start_a_line() {
        assert_eq!(lines("aa  bb", 30.0), vec!["aa", "bb"]);
    }

    #[test]
    fn zero_width_terminates() {
        // Hostile: a zero-width line must still advance and terminate.
        let out = lines("abc", 0.0);
        assert_eq!(out.concat(), "abc");
    }

    #[test]
    fn all_spaces_terminate() {
        let out = lines("      ", 10.0);
        assert!(out.len() <= 6, "got {out:?}");
    }
}