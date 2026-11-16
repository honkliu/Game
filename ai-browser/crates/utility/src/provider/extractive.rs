//! Offline, deterministic provider: answers by quoting the page.
//!
//! No model, no network, no randomness: the same blocks and question always
//! produce the same reply. It is honest about what it is — `translate` and
//! `rewrite` return the selection unchanged with a note saying so, and `verify`
//! says it can only show what the page says, not judge truth.
//!
//! Replies use the shared `ANSWER:` / `QUOTE` text contract. Every quote is a
//! whole sentence cut from a block, so it is a verbatim substring by construction.

use std::collections::{HashMap, HashSet};

use ai_platform_contract::failure::Failure;
use ai_platform_contract::model::{PageLink, TaskIntent};
use serde_json::json;

use super::{CompletionRequest, Mode, ModelProvider, ProviderReply};
use crate::control::TaskControl;
use crate::reply::locate;
use crate::text::{collapse_ws, sentences, terms, word_stems};

const MAX_SENTENCES: usize = 3;
const STREAM_CHUNK_CHARS: usize = 24;

pub struct ExtractiveProvider {
    id: String,
}

impl ExtractiveProvider {
    pub fn new(id: &str) -> ExtractiveProvider {
        ExtractiveProvider { id: id.to_string() }
    }
}

struct Sentence<'a> {
    block_id: &'a str,
    text: &'a str,
    stems: HashSet<String>,
    /// Position in reading order across the whole request.
    order: usize,
}

fn all_sentences(request: &CompletionRequest) -> Vec<Sentence<'_>> {
    let mut out = Vec::new();
    for block in &request.blocks {
        for text in sentences(&block.text) {
            let order = out.len();
            out.push(Sentence { block_id: &block.id, text, stems: word_stems(text).into_iter().collect(), order });
        }
    }
    out
}

fn overlap(sentence: &Sentence<'_>, query: &[String]) -> usize {
    query.iter().filter(|term| sentence.stems.contains(*term)).count()
}

/// Best sentences for `query`, strongest first (ties in reading order).
fn rank<'a, 'b>(sentences: &'b [Sentence<'a>], query: &[String]) -> Vec<(usize, &'b Sentence<'a>)> {
    let mut scored: Vec<(usize, &Sentence<'a>)> = sentences
        .iter()
        .map(|s| (overlap(s, query), s))
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.order.cmp(&b.1.order)));
    scored
}

fn top_in_reading_order<'a, 'b>(ranked: &[(usize, &'b Sentence<'a>)], limit: usize) -> Vec<&'b Sentence<'a>> {
    let best = ranked.first().map(|(score, _)| *score).unwrap_or(0);
    let mut picked: Vec<&Sentence<'a>> = ranked
        .iter()
        .filter(|(score, _)| score * 2 >= best)
        .take(limit)
        .map(|(_, s)| *s)
        .collect();
    picked.sort_by_key(|s| s.order);
    picked
}

fn compose(note: Option<&str>, picked: &[&Sentence<'_>]) -> String {
    let mut prose = String::new();
    if let Some(note) = note {
        prose.push_str(note);
        prose.push(' ');
    }
    prose.push_str(&picked.iter().map(|s| s.text).collect::<Vec<_>>().join(" "));
    let mut out = format!("ANSWER: {prose}\n");
    for sentence in picked {
        out.push_str(&format!("QUOTE {}: {}\n", sentence.block_id, sentence.text));
    }
    out
}

fn focus_text(request: &CompletionRequest) -> String {
    request
        .selection
        .as_deref()
        .filter(|s| !collapse_ws(s).is_empty())
        .map(collapse_ws)
        .unwrap_or_else(|| collapse_ws(&request.question))
}

fn no_match() -> Failure {
    Failure::no_answer("extract.no_match").with_detail("no passage shares the question's terms")
}

fn answer_text(request: &CompletionRequest) -> Result<String, Failure> {
    let sentences = all_sentences(request);
    if sentences.is_empty() {
        return Err(Failure::no_answer("extract.no_passages"));
    }
    match request.intent {
        TaskIntent::Ask | TaskIntent::Act => {
            let query = terms(&format!("{} {}", request.question, request.selection.as_deref().unwrap_or("")));
            if query.is_empty() {
                return Err(Failure::no_answer("extract.no_search_terms"));
            }
            let ranked = rank(&sentences, &query);
            if ranked.is_empty() {
                return Err(no_match());
            }
            Ok(compose(None, &top_in_reading_order(&ranked, MAX_SENTENCES)))
        }
        TaskIntent::Summarize => {
            let mut frequency: HashMap<&str, usize> = HashMap::new();
            for sentence in &sentences {
                for stem in &sentence.stems {
                    *frequency.entry(stem.as_str()).or_default() += 1;
                }
            }
            let query = terms(&request.question);
            let mut scored: Vec<(usize, &Sentence<'_>)> = sentences
                .iter()
                .filter(|s| s.stems.len() >= 4)
                .map(|s| {
                    let base: usize = s.stems.iter().map(|stem| frequency.get(stem.as_str()).copied().unwrap_or(0)).sum();
                    // Normalised by length so long sentences do not always win.
                    ((base * 100 + overlap(s, &query) * 10_000) / s.stems.len(), s)
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.order.cmp(&b.1.order)));
            let mut picked: Vec<&Sentence<'_>> = scored.iter().take(MAX_SENTENCES).map(|(_, s)| *s).collect();
            if picked.is_empty() {
                picked = sentences.iter().take(MAX_SENTENCES).collect();
            }
            picked.sort_by_key(|s| s.order);
            Ok(compose(Some("Key sentences from the page (extracted, not rewritten):"), &picked))
        }
        TaskIntent::Explain => {
            let focus = focus_text(request);
            let hit = sentences.iter().position(|s| collapse_ws(s.text).contains(&focus) || focus.contains(&collapse_ws(s.text)));
            let picked: Vec<&Sentence<'_>> = match hit {
                Some(index) => {
                    let mut picked = vec![&sentences[index]];
                    if let Some(next) = sentences.get(index + 1).filter(|n| n.block_id == sentences[index].block_id) {
                        picked.push(next);
                    }
                    picked
                }
                None => {
                    let ranked = rank(&sentences, &terms(&focus));
                    if ranked.is_empty() {
                        return Err(no_match());
                    }
                    top_in_reading_order(&ranked, 2)
                }
            };
            Ok(compose(Some("Offline mode cannot paraphrase; this is what the page says around it:"), &picked))
        }
        TaskIntent::Verify => {
            let claim = focus_text(request);
            let ranked = rank(&sentences, &terms(&claim));
            if ranked.is_empty() {
                return Err(no_match());
            }
            let verbatim = sentences.iter().any(|s| collapse_ws(s.text).contains(&claim));
            let note = if verbatim {
                "The page contains this text, which shows the page says it, not that it is true:"
            } else {
                "Offline mode cannot judge whether this is true. The page says this on the topic:"
            };
            Ok(compose(Some(note), &top_in_reading_order(&ranked, 2)))
        }
        TaskIntent::Translate | TaskIntent::Rewrite => {
            let Some(selection) = request.selection.as_deref().map(collapse_ws).filter(|s| !s.is_empty()) else {
                return Err(Failure::no_answer("extract.needs_selection")
                    .with_detail("translate/rewrite needs selected text; the offline provider cannot do either"));
            };
            let note = if request.intent == TaskIntent::Translate {
                let language = request.target_language.as_deref().unwrap_or("the requested language");
                format!("Offline mode cannot translate into {language}. The selected text, unchanged:")
            } else {
                "Offline mode cannot rewrite text. The selected text, unchanged:".to_string()
            };
            let mut out = format!("ANSWER: {note} {selection}\n");
            if let Some(block) = request.blocks.iter().find(|b| locate(&b.text, &selection).is_some()) {
                out.push_str(&format!("QUOTE {}: {}\n", block.id, selection));
            }
            Ok(out)
        }
    }
}

fn plain(text: &str) -> String {
    collapse_ws(&text.to_lowercase())
}

fn best_link<'a>(links: &'a [PageLink], target: &str) -> Option<&'a PageLink> {
    let target = plain(target);
    let filler = ["the", "a", "an", "link", "page", "tab", "button", "to"];
    let target: String = target.split(' ').filter(|w| !filler.contains(w)).collect::<Vec<_>>().join(" ");
    if target.is_empty() {
        return None;
    }
    if let Some(link) = links.iter().find(|l| plain(&l.text) == target) {
        return Some(link);
    }
    if let Some(link) = links.iter().find(|l| {
        let text = plain(&l.text);
        !text.is_empty() && (text.contains(&target) || target.contains(&text))
    }) {
        return Some(link);
    }
    let query = terms(&target);
    let mut best: Option<(usize, &PageLink)> = None;
    for link in links {
        let stems: HashSet<String> = word_stems(&link.text).into_iter().collect();
        let score = query.iter().filter(|t| stems.contains(*t)).count();
        if score > 0 && best.is_none_or(|(top, _)| score > top) {
            best = Some((score, link));
        }
    }
    best.map(|(_, link)| link)
}

fn strip_any<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| text.strip_prefix(p)).map(str::trim)
}

fn plan_text(request: &CompletionRequest) -> String {
    if let Some(last) = request.steps.last() {
        let action = if last.summary.starts_with("error:") {
            json!({"action": "fail", "reason": "the last action did not complete"})
        } else {
            json!({"action": "finish", "message": format!("Done: {}.", last.action_kind.replace('_', " "))})
        };
        return action.to_string();
    }
    let q = plain(&request.question);
    let fail = |reason: &str| json!({"action": "fail", "reason": reason}).to_string();

    if ["list tabs", "show tabs", "show my tabs", "what tabs"].iter().any(|p| q.starts_with(p)) {
        return json!({"action": "list_tabs"}).to_string();
    }
    if let Some(rest) = strip_any(&q, &["open link ", "click link ", "go to link ", "link "]) {
        let number: String = rest.chars().filter(char::is_ascii_digit).collect();
        let wanted = format!("l{number}");
        return match request.links.iter().find(|l| l.link_id == wanted) {
            Some(link) => {
                let kind = if q.starts_with("go to") { "navigate" } else { "open_tab" };
                json!({"action": kind, "link_id": link.link_id}).to_string()
            }
            None => fail("that link id is not in this page's snapshot"),
        };
    }
    if let Some(rest) = strip_any(&q, &["scroll to ", "highlight ", "find "]) {
        let needle = rest.trim_matches('"');
        let kind = if q.starts_with("highlight") { "highlight_passage" } else { "scroll_to" };
        if let Some(block) = request.blocks.iter().find(|b| plain(&b.text).contains(needle)) {
            return json!({"action": kind, "block_id": block.id}).to_string();
        }
        let sentences = all_sentences(request);
        let ranked = rank(&sentences, &terms(needle));
        return match ranked.first() {
            Some((_, sentence)) => json!({"action": kind, "block_id": sentence.block_id}).to_string(),
            None => fail("no passage on this page matches that"),
        };
    }
    if let Some(rest) = strip_any(&q, &["go to ", "navigate to ", "visit ", "take me to "]) {
        return match best_link(&request.links, rest) {
            Some(link) => json!({"action": "navigate", "link_id": link.link_id}).to_string(),
            None => fail("no link on this page matches that"),
        };
    }
    if let Some(rest) = strip_any(&q, &["open ", "click on ", "click ", "follow "]) {
        return match best_link(&request.links, rest) {
            Some(link) => json!({"action": "open_tab", "link_id": link.link_id}).to_string(),
            None => fail("no link on this page matches that"),
        };
    }
    fail("the offline provider only understands: open link N, open/go to <link text>, scroll to <text>, list tabs")
}

impl ModelProvider for ExtractiveProvider {
    fn provider_id(&self) -> &str {
        &self.id
    }

    fn complete(
        &self,
        request: &CompletionRequest,
        control: &TaskControl,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ProviderReply, Failure> {
        if let Some(failure) = control.failure() {
            return Err(failure);
        }
        let text = match request.mode {
            Mode::Answer => answer_text(request)?,
            Mode::Plan => plan_text(request),
        };
        if request.mode == Mode::Answer {
            let chars: Vec<char> = text.chars().collect();
            for chunk in chars.chunks(STREAM_CHUNK_CHARS) {
                if let Some(failure) = control.failure() {
                    return Err(failure);
                }
                let piece: String = chunk.iter().collect();
                on_delta(&piece);
            }
        }
        Ok(ProviderReply {
            text,
            finish_reason: Some("stop".into()),
            reasoning_tokens: None,
            completion_token_budget: request.max_tokens,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::provider::{PromptBlock, StepObservation};
    use ai_platform_contract::identity::TaskId;

    fn request(intent: TaskIntent, question: &str) -> CompletionRequest {
        CompletionRequest {
            task: TaskId(1),
            model_id: "extractive-v1".into(),
            mode: if intent == TaskIntent::Act { Mode::Plan } else { Mode::Answer },
            intent,
            question: question.into(),
            selection: None,
            target_language: None,
            history: Vec::new(),
            blocks: vec![
                PromptBlock { id: "b0".into(), text: "Our store ships worldwide. Orders leave the warehouse in two days.".into() },
                PromptBlock { id: "b1".into(), text: "Returns are accepted within 30 days of delivery. Refunds go to the original payment method.".into() },
            ],
            links: vec![
                PageLink { link_id: "l0".into(), text: "Shipping details".into(), url: "https://shop.example/shipping".into(), block_id: None },
                PageLink { link_id: "l1".into(), text: "Return policy".into(), url: "https://shop.example/returns".into(), block_id: None },
            ],
            steps: vec![],
            messages: vec![],
            max_tokens: 1024,
        }
    }

    fn run(request: &CompletionRequest) -> Result<ProviderReply, Failure> {
        ExtractiveProvider::new("extractive").complete(request, &TaskControl::new(0), &mut |_| {})
    }

    #[test]
    fn ask_quotes_the_best_matching_sentence() {
        let reply = run(&request(TaskIntent::Ask, "How long do I have to return an item?")).unwrap();
        assert!(reply.text.contains("QUOTE b1: Returns are accepted within 30 days of delivery."), "{}", reply.text);
        assert!(!reply.text.contains("warehouse"));
    }

    #[test]
    fn ask_with_no_overlap_is_an_honest_no_answer() {
        let failure = run(&request(TaskIntent::Ask, "What is the airspeed of a swallow?")).unwrap_err();
        assert_eq!(failure.code, "extract.no_match");
    }

    #[test]
    fn translate_and_rewrite_admit_they_cannot() {
        let mut req = request(TaskIntent::Translate, "");
        req.selection = Some("Orders leave the warehouse in two days.".into());
        req.target_language = Some("fr".into());
        let reply = run(&req).unwrap();
        assert!(reply.text.contains("cannot translate into fr"));
        assert!(reply.text.contains("QUOTE b0: Orders leave the warehouse in two days."));
        req.selection = None;
        assert_eq!(run(&req).unwrap_err().code, "extract.needs_selection");
    }

    #[test]
    fn verify_does_not_claim_truth() {
        let mut req = request(TaskIntent::Verify, "");
        req.selection = Some("Returns are accepted within 60 days".into());
        let reply = run(&req).unwrap();
        assert!(reply.text.contains("cannot judge whether this is true"));
        assert!(reply.text.contains("30 days"));
    }

    #[test]
    fn explain_points_at_the_selected_sentence_and_its_neighbour() {
        let mut req = request(TaskIntent::Explain, "");
        req.selection = Some("Returns are accepted within 30 days of delivery.".into());
        let reply = run(&req).unwrap();
        assert!(reply.text.contains("QUOTE b1: Refunds go to the original payment method."));
    }

    #[test]
    fn summarize_is_deterministic() {
        let req = request(TaskIntent::Summarize, "");
        assert_eq!(run(&req).unwrap().text, run(&req).unwrap().text);
        assert!(run(&req).unwrap().text.starts_with("ANSWER: Key sentences"));
    }

    fn plan(question: &str) -> serde_json::Value {
        serde_json::from_str(&run(&request(TaskIntent::Act, question)).unwrap().text).unwrap()
    }

    #[test]
    fn act_plans_are_deterministic_and_only_name_snapshot_links() {
        assert_eq!(plan("open link 1"), json!({"action": "open_tab", "link_id": "l1"}));
        assert_eq!(plan("go to the return policy"), json!({"action": "navigate", "link_id": "l1"}));
        assert_eq!(plan("Open shipping"), json!({"action": "open_tab", "link_id": "l0"}));
        assert_eq!(plan("open link 9")["action"], "fail");
        assert_eq!(plan("open the careers page")["action"], "fail");
        assert_eq!(plan("scroll to refunds go to")["block_id"], "b1");
        assert_eq!(plan("list tabs"), json!({"action": "list_tabs"}));
        assert_eq!(plan("order me a pizza")["action"], "fail");
    }

    #[test]
    fn act_finishes_after_a_step() {
        let mut req = request(TaskIntent::Act, "open link 1");
        req.steps.push(StepObservation { action_kind: "open_tab".into(), summary: "ok".into() });
        let reply: serde_json::Value = serde_json::from_str(&run(&req).unwrap().text).unwrap();
        assert_eq!(reply["action"], "finish");
        req.steps[0].summary = "error: unsupported".into();
        let reply: serde_json::Value = serde_json::from_str(&run(&req).unwrap().text).unwrap();
        assert_eq!(reply["action"], "fail");
    }
}
