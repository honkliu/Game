//! The text contract every provider answers in, and the check that makes a
//! citation mean something.
//!
//! A `QUOTE` survives only if its text is a verbatim substring of the passage it
//! names (whitespace runs may differ, because a model re-wraps lines; the kept
//! quote is then re-cut from the passage so it is literally a substring). A quote
//! naming an unknown passage, or text that exists only in a *different* passage,
//! is dropped and counted.

use ai_platform_contract::contract::CitationRole;
use ai_platform_contract::model::PageBlock;

use crate::text::{clip_chars, collapse_ws};

pub const MAX_QUOTE_CHARS: usize = 600;
pub const MIN_QUOTE_CHARS: usize = 4;
pub const MAX_CITATIONS: usize = 8;
const AFFIX_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawQuote {
    pub block_id: String,
    pub role: Option<CitationRole>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReply {
    pub answer: String,
    pub quotes: Vec<RawQuote>,
    /// False when the model ignored the format entirely; `answer` is then the
    /// whole reply and nothing is cited.
    pub had_markers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedQuote {
    pub block_id: String,
    pub quote: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub role: Option<CitationRole>,
}

fn quote_header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("QUOTE")?;
    (rest.starts_with(' ') || rest.starts_with(':')).then_some(rest)
}

fn parse_role(word: &str) -> Option<CitationRole> {
    match word.trim_matches(['(', ')', '[', ']']).to_ascii_lowercase().as_str() {
        "direct" | "direct_answer" => Some(CitationRole::DirectAnswer),
        "supporting" | "support" => Some(CitationRole::Supporting),
        "counterpoint" | "counter" => Some(CitationRole::Counterpoint),
        "context" => Some(CitationRole::Context),
        _ => None,
    }
}

/// The answer prose of a (possibly partial) reply: the `ANSWER:` line and its
/// continuation lines up to the first `QUOTE` line. Line breaks and indentation are
/// kept, because the answer is Markdown (lists, tables, code fences). Monotonic as
/// the reply grows, except for trailing whitespace, so it doubles as the streaming
/// filter.
pub fn visible_answer(raw: &str) -> String {
    let pieces: Vec<&str> = raw.split('\n').collect();
    let mut out = String::new();
    let mut in_answer = false;
    for (index, piece) in pieces.iter().enumerate() {
        let in_progress = index + 1 == pieces.len();
        let piece = piece.trim_end_matches('\r');
        let start = piece.trim_start();
        let content: &str;
        if let Some(rest) = start.strip_prefix("ANSWER:") {
            in_answer = true;
            content = rest.trim_start();
        } else if quote_header(start).is_some() {
            in_answer = false;
            continue;
        } else if in_answer {
            // A half-typed marker must not leak as answer text.
            if in_progress && !start.is_empty() && ("QUOTE".starts_with(start) || "ANSWER:".starts_with(start)) {
                continue;
            }
            content = piece;
        } else {
            continue;
        }
        let content = if in_progress { content } else { content.trim_end() };
        if content.is_empty() && out.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(content);
    }
    out
}

pub fn parse(content: &str) -> ParsedReply {
    let mut quotes = Vec::new();
    let mut had_markers = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("ANSWER:") {
            had_markers = true;
        }
        let Some(rest) = quote_header(line) else { continue };
        had_markers = true;
        let Some((head, text)) = rest.split_once(':') else { continue };
        let mut words = head.split_whitespace();
        let Some(id) = words.next() else { continue };
        let role = words.next().and_then(parse_role);
        quotes.push(RawQuote {
            block_id: id.trim_matches(['[', ']', '(', ')']).to_string(),
            role,
            text: text.trim().trim_matches(['"', '\u{201c}', '\u{201d}']).trim().to_string(),
        });
    }
    let answer = if had_markers {
        visible_answer(content).trim().to_string()
    } else {
        content.trim().to_string()
    };
    ParsedReply { answer, quotes, had_markers }
}

/// Byte range of `quote` inside `text`, matching exactly or modulo whitespace runs.
pub fn locate(text: &str, quote: &str) -> Option<(usize, usize)> {
    let needle = quote.trim();
    if needle.is_empty() {
        return None;
    }
    if let Some(start) = text.find(needle) {
        return Some((start, start + needle.len()));
    }
    let needle = collapse_ws(needle);
    // (byte index in `norm`, original start, original end) for every kept char.
    let mut norm = String::new();
    let mut map: Vec<(usize, usize, usize)> = Vec::new();
    let mut last_was_space = true;
    for (at, c) in text.char_indices() {
        let end = at + c.len_utf8();
        if c.is_whitespace() {
            if !last_was_space {
                map.push((norm.len(), at, end));
                norm.push(' ');
                last_was_space = true;
            }
        } else {
            map.push((norm.len(), at, end));
            norm.push(c);
            last_was_space = false;
        }
    }
    let found = norm.find(&needle)?;
    let found_end = found + needle.len();
    let first = map.iter().find(|entry| entry.0 == found)?;
    let last = map.iter().rev().find(|entry| entry.0 < found_end)?;
    Some((first.1, last.2))
}

/// Keeps the quotes that really are in the passage they name. Returns the
/// survivors and how many were rejected.
pub fn verify_quotes(quotes: &[RawQuote], blocks: &[PageBlock]) -> (Vec<VerifiedQuote>, usize) {
    let mut verified: Vec<VerifiedQuote> = Vec::new();
    let mut rejected = 0usize;
    for raw in quotes {
        let Some(block) = blocks.iter().find(|block| block.block_id == raw.block_id) else {
            rejected += 1;
            continue;
        };
        let candidate = clip_chars(raw.text.trim(), MAX_QUOTE_CHARS);
        if candidate.chars().count() < MIN_QUOTE_CHARS {
            rejected += 1;
            continue;
        }
        let Some((start, end)) = locate(&block.text, candidate) else {
            rejected += 1;
            continue;
        };
        let quote = block.text[start..end].to_string();
        if verified.iter().any(|v| v.block_id == raw.block_id && v.quote == quote) {
            continue;
        }
        if verified.len() >= MAX_CITATIONS {
            rejected += 1;
            continue;
        }
        let prefix: String = block.text[..start].chars().rev().take(AFFIX_CHARS).collect::<Vec<_>>().into_iter().rev().collect();
        let suffix: String = block.text[end..].chars().take(AFFIX_CHARS).collect();
        verified.push(VerifiedQuote {
            block_id: block.block_id.clone(),
            quote,
            prefix: (!prefix.trim().is_empty()).then_some(prefix),
            suffix: (!suffix.trim().is_empty()).then_some(suffix),
            role: raw.role,
        });
    }
    (verified, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks() -> Vec<PageBlock> {
        vec![
            PageBlock { block_id: "b0".into(), text: "Returns are accepted within 30 days of delivery.\nItems must be unused.".into() },
            PageBlock { block_id: "b1".into(), text: "Shipping is free over $50.".into() },
        ]
    }

    #[test]
    fn parses_answer_quotes_and_roles() {
        let reply = "ANSWER: Thirty days.\nIt is free.\nQUOTE b0: \"Returns are accepted within 30 days of delivery.\"\nQUOTE [b1] (counterpoint): Shipping is free over $50.\n";
        let parsed = parse(reply);
        assert!(parsed.had_markers);
        assert_eq!(parsed.answer, "Thirty days.\nIt is free.");
        assert_eq!(parsed.quotes.len(), 2);
        assert_eq!(parsed.quotes[1].block_id, "b1");
        assert_eq!(parsed.quotes[1].role, Some(CitationRole::Counterpoint));
        assert_eq!(parsed.quotes[0].text, "Returns are accepted within 30 days of delivery.");
    }

    #[test]
    fn answer_keeps_markdown_structure() {
        let reply = "ANSWER: Options:\n\n- one\n  - nested\n\n| a | b |\n|---|---|\n| 1 | 2 |\nQUOTE b0: x\n";
        assert_eq!(parse(reply).answer, "Options:\n\n- one\n  - nested\n\n| a | b |\n|---|---|\n| 1 | 2 |");
    }

    #[test]
    fn unformatted_reply_is_an_uncited_answer() {
        let parsed = parse("Just prose, no markers.");
        assert!(!parsed.had_markers);
        assert_eq!(parsed.answer, "Just prose, no markers.");
        assert!(parsed.quotes.is_empty());
    }

    #[test]
    fn streaming_filter_matches_the_final_answer_for_every_split() {
        let reply = "ANSWER: Thirty days, all told.\nSecond line.\nQUOTE b0: Returns are accepted within 30 days of delivery.\n";
        let finalized = parse(reply).answer;
        let mut emitted = String::new();
        let mut raw = String::new();
        for c in reply.chars() {
            raw.push(c);
            let visible = visible_answer(&raw);
            assert!(!visible.contains("QUOTE") && !visible.contains("ANSWER"));
            if let Some(rest) = visible.strip_prefix(emitted.as_str()) {
                emitted.push_str(rest);
            }
        }
        assert_eq!(emitted.trim(), finalized);
    }

    #[test]
    fn rejects_quotes_that_are_not_verbatim() {
        let quotes = vec![
            RawQuote { block_id: "b0".into(), role: None, text: "Returns are accepted within 60 days of delivery.".into() },
            RawQuote { block_id: "b0".into(), role: None, text: "Shipping is free over $50.".into() },
            RawQuote { block_id: "b9".into(), role: None, text: "Shipping is free over $50.".into() },
            RawQuote { block_id: "b1".into(), role: None, text: "free shipping over $50".into() },
            RawQuote { block_id: "b1".into(), role: None, text: "   ".into() },
        ];
        let (verified, rejected) = verify_quotes(&quotes, &blocks());
        assert!(verified.is_empty());
        assert_eq!(rejected, 5);
    }

    #[test]
    fn accepts_verbatim_and_rewrapped_quotes_as_literal_substrings() {
        let blocks = blocks();
        let quotes = vec![
            RawQuote { block_id: "b0".into(), role: None, text: "accepted within 30 days".into() },
            RawQuote { block_id: "b0".into(), role: None, text: "of delivery. Items must  be unused.".into() },
        ];
        let (verified, rejected) = verify_quotes(&quotes, &blocks);
        assert_eq!(rejected, 0);
        assert_eq!(verified.len(), 2);
        for quote in &verified {
            assert!(blocks[0].text.contains(&quote.quote), "{:?}", quote.quote);
        }
        assert_eq!(verified[1].quote, "of delivery.\nItems must be unused.");
        assert_eq!(verified[0].prefix.as_deref(), Some("Returns are "));
    }
}
