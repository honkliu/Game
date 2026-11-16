//! Context reduction: fit the page into the model's token budget.
//!
//! Order of operations: drop empty and duplicate blocks, score the rest against
//! the question (the selected passage outranks everything), take blocks by score
//! until the budget is spent, then restore reading order. A clipped block is a
//! *prefix* of the original, so any quote from it is still a verbatim substring
//! of the snapshot text the citation check runs against.

use std::collections::HashSet;

use ai_platform_contract::model::{PageBlock, TaskIntent};

use crate::provider::PromptBlock;
use crate::text::{clip_chars, collapse_ws, terms};

/// ~3 characters per token: conservative for mixed prose (measured anchor on the
/// release-zero endpoint was ~2.2 for dense text).
pub const CHARS_PER_TOKEN: usize = 3;
const MAX_PROMPT_TOKENS: usize = 12_000;
/// System prompt, framing, and the question itself.
const PROMPT_OVERHEAD_TOKENS: usize = 800;
const PER_BLOCK_OVERHEAD_CHARS: usize = 10;
const MIN_CLIP_CHARS: usize = 200;

pub struct ReduceInput<'a> {
    pub blocks: &'a [PageBlock],
    pub question: &'a str,
    pub selection: Option<&'a str>,
    pub intent: TaskIntent,
    pub token_budget: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reduced {
    pub blocks: Vec<PromptBlock>,
    /// Blocks seen, before dedupe.
    pub total_blocks: usize,
    pub duplicates: usize,
    /// Distinct blocks left out for budget.
    pub dropped: usize,
    pub clipped: bool,
    pub chars: usize,
}

impl Reduced {
    pub fn is_lossy(&self) -> bool {
        self.dropped > 0 || self.clipped
    }
}

/// Prompt budget in tokens for a model with `context_window`, leaving room for
/// the completion.
pub fn prompt_token_budget(context_window: u32, max_tokens: u32) -> usize {
    let window = context_window as usize;
    let completion = (max_tokens as usize).min(window / 2);
    window
        .saturating_sub(completion)
        .saturating_sub(PROMPT_OVERHEAD_TOKENS)
        .clamp(256, MAX_PROMPT_TOKENS)
}

pub fn reduce(input: &ReduceInput<'_>) -> Reduced {
    let budget_chars = input.token_budget.saturating_mul(CHARS_PER_TOKEN);
    let total_blocks = input.blocks.len();

    let mut seen = HashSet::new();
    let mut unique: Vec<(&PageBlock, String)> = Vec::new();
    let mut duplicates = 0usize;
    for block in input.blocks {
        let collapsed = collapse_ws(&block.text);
        if collapsed.is_empty() {
            continue;
        }
        if !seen.insert(collapsed.to_lowercase()) {
            duplicates += 1;
            continue;
        }
        unique.push((block, collapsed));
    }

    let mut query_terms = terms(input.question);
    let selection = input.selection.map(collapse_ws).filter(|s| !s.is_empty());
    if let Some(selection) = &selection {
        query_terms.extend(terms(selection));
        query_terms.sort();
        query_terms.dedup();
    }
    let reading_order_only = query_terms.is_empty() && selection.is_none() || input.intent == TaskIntent::Summarize;

    let mut scored: Vec<(f64, usize)> = unique
        .iter()
        .enumerate()
        .map(|(index, (_, text))| {
            let mut score = 0.0f64;
            if reading_order_only {
                score = 1.0 / (1.0 + index as f64);
            } else {
                let block_terms = terms(text);
                score += query_terms.iter().filter(|t| block_terms.contains(t)).count() as f64;
                // Tiny reading-order tiebreak so equal scores stay deterministic.
                score += 0.001 / (1.0 + index as f64);
            }
            if selection.as_ref().is_some_and(|s| text.contains(s.as_str())) {
                score += 1_000_000.0;
            }
            (score, index)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut remaining = budget_chars;
    let mut picked: Vec<(usize, String)> = Vec::new();
    let mut clipped = false;
    for (_, index) in scored {
        let text = &unique[index].1;
        let cost = text.chars().count() + PER_BLOCK_OVERHEAD_CHARS;
        if cost <= remaining {
            remaining -= cost;
            picked.push((index, text.clone()));
        } else if remaining >= MIN_CLIP_CHARS + PER_BLOCK_OVERHEAD_CHARS {
            let take = remaining - PER_BLOCK_OVERHEAD_CHARS;
            picked.push((index, clip_chars(text, take).to_string()));
            clipped = true;
            remaining = 0;
        }
    }
    picked.sort_by_key(|(index, _)| *index);

    let chars = picked.iter().map(|(_, text)| text.chars().count()).sum();
    let dropped = unique.len() - picked.len();
    Reduced {
        blocks: picked
            .into_iter()
            .map(|(index, text)| PromptBlock { id: unique[index].0.block_id.clone(), text })
            .collect(),
        total_blocks,
        duplicates,
        dropped,
        clipped,
        chars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: &str, text: &str) -> PageBlock {
        PageBlock { block_id: id.into(), text: text.into() }
    }

    fn input<'a>(blocks: &'a [PageBlock], question: &'a str, selection: Option<&'a str>, budget: usize) -> ReduceInput<'a> {
        ReduceInput { blocks, question, selection, intent: TaskIntent::Ask, token_budget: budget }
    }

    #[test]
    fn dedupes_and_keeps_reading_order() {
        let blocks = vec![
            block("b0", "Shipping takes five days."),
            block("b1", "Returns accepted within 30 days."),
            block("b2", "  shipping   takes five DAYS. "),
            block("b3", "   "),
        ];
        let reduced = reduce(&input(&blocks, "return policy", None, 1000));
        assert_eq!(reduced.duplicates, 1);
        assert_eq!(reduced.dropped, 0);
        let ids: Vec<_> = reduced.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["b0", "b1"]);
    }

    #[test]
    fn relevant_block_survives_a_tight_budget() {
        let filler = "lorem ipsum dolor sit amet ".repeat(20);
        let blocks = vec![
            block("b0", &filler),
            block("b1", &format!("{filler} unrelated")),
            block("b2", "The refund window is 30 days from delivery."),
            block("b3", &format!("{filler} more")),
        ];
        let reduced = reduce(&input(&blocks, "what is the refund window", None, 40));
        assert!(reduced.blocks.iter().any(|b| b.id == "b2"));
        assert!(reduced.dropped > 0 && reduced.is_lossy());
        assert!(reduced.chars <= 40 * CHARS_PER_TOKEN);
    }

    #[test]
    fn selection_block_is_prioritised_over_better_term_matches() {
        let blocks = vec![
            block("b0", "refund refund refund policy policy policy details here"),
            block("b1", "The warranty covers manufacturing defects only."),
        ];
        let reduced = reduce(&input(&blocks, "refund policy", Some("warranty covers manufacturing defects"), 22));
        assert_eq!(reduced.blocks.len(), 1);
        assert_eq!(reduced.blocks[0].id, "b1");
    }

    #[test]
    fn oversized_block_is_clipped_to_a_verbatim_prefix() {
        let long = "word ".repeat(400);
        let blocks = vec![block("b0", &long)];
        let reduced = reduce(&input(&blocks, "word", None, 100));
        assert!(reduced.clipped);
        assert!(collapse_ws(&long).starts_with(&reduced.blocks[0].text));
    }

    #[test]
    fn budget_follows_context_window() {
        assert_eq!(prompt_token_budget(8192, 4096), 3296);
        assert_eq!(prompt_token_budget(1_000_000, 4096), MAX_PROMPT_TOKENS);
        assert_eq!(prompt_token_budget(1024, 4096), 256);
    }
}
