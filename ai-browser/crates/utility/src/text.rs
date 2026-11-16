//! Small text helpers shared by context reduction and the extractive provider.
//! Deterministic and allocation-light; no stemming library, just enough suffix
//! stripping that "returns" matches "return".

const STOP_WORDS: &[&str] = &[
    "what", "where", "when", "which", "who", "whom", "whose", "why", "how", "is", "are", "was", "were", "be",
    "been", "being", "do", "does", "did", "the", "an", "of", "about", "and", "for", "with", "this", "that",
    "these", "those", "from", "into", "can", "could", "would", "should", "will", "you", "your", "tell", "me",
    "please", "its", "has", "have", "had", "not", "but", "any", "all",
];

pub fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn stem(word: &str) -> String {
    let word = word.to_lowercase();
    let n = word.chars().count();
    let strip = |suffix: &str, min: usize| -> Option<String> {
        (n > min && word.ends_with(suffix)).then(|| word[..word.len() - suffix.len()].to_string())
    };
    if let Some(base) = strip("ies", 4) {
        return format!("{base}y");
    }
    for (suffix, min) in [("ing", 5), ("ed", 4), ("es", 4)] {
        if let Some(base) = strip(suffix, min) {
            return base;
        }
    }
    if !word.ends_with("ss") {
        if let Some(base) = strip("s", 3) {
            return base;
        }
    }
    word
}

/// Distinct, stemmed, non-stop words of `text`, sorted.
pub fn terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() > 2 && !STOP_WORDS.contains(&word.to_lowercase().as_str()))
        .map(stem)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every word of `text` (len > 2) stemmed, in order, stop words included.
pub fn word_stems(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() > 2)
        .map(stem)
        .collect()
}

/// Sentence slices of `text`, trimmed, in order. Slices borrow from `text`, so
/// a sentence is always a verbatim substring of its block.
pub fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(at, c)) in chars.iter().enumerate() {
        let boundary = match c {
            '\n' => true,
            '.' | '!' | '?' => chars.get(i + 1).is_none_or(|&(_, next)| next.is_whitespace()),
            _ => false,
        };
        if boundary {
            let end = at + c.len_utf8();
            let piece = text[start..end].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = end;
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

pub fn clip_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_match_simple_inflections() {
        assert_eq!(terms("What are the return policies?"), vec!["policy", "return"]);
        assert_eq!(terms("Returns returned returning"), vec!["return"]);
    }

    #[test]
    fn sentences_are_verbatim_slices() {
        let text = "Free returns within 30 days. Items must be unused!  See www.example.com for details.";
        let parts = sentences(text);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| text.contains(p)));
        assert_eq!(parts[2], "See www.example.com for details.");
    }
}
