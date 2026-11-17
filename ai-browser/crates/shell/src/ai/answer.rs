//! The shell's memory of what was answered, and the only door to a citation click.
//!
//! A turn is the unit: one question, one task, one answer or one refusal, bound to
//! the document the capture came from. Turns are kept so the overlay can be closed
//! and reopened without asking the provider again, and so a click on a citation
//! three minutes later is checked against live identity rather than against
//! whatever was on screen when the chip was drawn.
//!
//! ## The click path refuses, and says which rule refused it
//!
//! [`AnswerStore::scroll_y`] does not decide staleness itself. It hands the stored
//! [`AnswerWithCitations`] to the contract's own
//! [`AnswerWithCitations::verify`], which is the single implementation of "is this
//! quote still about the document on screen" — live document, matching
//! `navigation_sequence`, and a snapshot version the document actually reached.
//! Duplicating that check here would be a second opinion, and second opinions
//! drift.
//!
//! What this module adds is the part the contract cannot know: which passage of
//! *this* capture sat at which scroll offset. The answer carries a `block_id`; the
//! y-coordinate came from the capture, and is recorded beside the turn at the
//! moment the answer landed. A citation whose block is not in that map is refused
//! rather than scrolled to a guess.

use std::collections::VecDeque;

use serde_json::{Value, json};

use ai_platform_contract::contract::{AnswerWithCitations, TaskOutcome};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::identity::SnapshotVersion;
use ai_platform_contract::identity::TaskId;

use super::context::PageContext;
use super::identity::Identity;
use super::provider::ProviderInfo;

/// Turns retained. Twenty questions per session; older ones scroll out of the
/// card and stop being clickable, which is the same lifetime the user sees.
pub const MAX_TURNS: usize = 20;

/// One citation's geometry, recorded from the capture the answer was built on.
#[derive(Debug, Clone)]
struct Placement {
    block_id: Option<String>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// One question and what came back for it.
pub struct Turn {
    pub turn_id: u64,
    pub task: TaskId,
    /// The shell's tab id — the thing `resolve_in_tab` and the scroll both need.
    pub tab_id: u64,
    pub question: String,
    pub status: &'static str,
    pub answer: Option<AnswerWithCitations>,
    pub failure: Option<Failure>,
    /// The capture generation the answer was built from, for
    /// [`Identity::assess_staleness`].
    pub generated_from: SnapshotVersion,
    pub provider: ProviderInfo,
    pub elapsed_ms: u64,
    /// Parallel to `answer.citations`, filled when the answer landed.
    placements: Vec<Placement>,
    /// Set when the tab navigated. The text stays readable; the chips stop working.
    pub stale: bool,
}

/// Answers kept for the session, oldest evicted first.
#[derive(Default)]
pub struct AnswerStore {
    turns: VecDeque<Turn>,
    next_turn: u64,
}

impl AnswerStore {
    /// Forget finished turns (a new conversation). Running ones are left alone.
    pub fn clear_turns(&mut self) {
        self.turns.retain(|t| t.status == "pending");
    }

    /// Open a turn before the work starts, so the overlay has an id to merge the
    /// final payload into. Returns the turn id.
    pub fn begin(
        &mut self,
        task: TaskId,
        tab_id: u64,
        question: &str,
        generated_from: SnapshotVersion,
        provider: ProviderInfo,
    ) -> u64 {
        self.next_turn += 1;
        let turn_id = self.next_turn;
        self.turns.push_back(Turn {
            turn_id,
            task,
            tab_id,
            question: question.to_string(),
            status: "pending",
            answer: None,
            failure: None,
            generated_from,
            provider,
            elapsed_ms: 0,
            placements: Vec::new(),
            stale: false,
        });
        while self.turns.len() > MAX_TURNS {
            self.turns.pop_front();
        }
        turn_id
    }

    /// Record the provider's outcome. `context` supplies the geometry, because it
    /// is the only place the rects exist; the capture itself is kept separately in
    /// [`super::context::ContextStore`].
    pub fn finish(
        &mut self,
        turn_id: u64,
        outcome: TaskOutcome,
        elapsed_ms: u64,
        context: &PageContext,
    ) {
        let Some(turn) = self.turns.iter_mut().find(|t| t.turn_id == turn_id) else {
            // A turn that scrolled out of the store while its question was in
            // flight. Nothing to update, and the answer is genuinely gone; the
            // caller logs it rather than pretending it landed.
            return;
        };
        turn.elapsed_ms = elapsed_ms;
        match outcome {
            TaskOutcome::Answered(answer) => {
                turn.placements = answer
                    .citations
                    .iter()
                    .map(|citation| match citation.anchor.block_id.as_deref() {
                        Some(id) => match context.find(id) {
                            Some(block) => Placement {
                                block_id: Some(id.to_string()),
                                x: block.x,
                                y: block.y,
                                w: block.w,
                                h: block.h,
                            },
                            None => Placement { block_id: Some(id.to_string()), x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
                        },
                        None => Placement { block_id: None, x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
                    })
                    .collect();
                turn.status = "ok";
                turn.answer = Some(answer);
            }
            TaskOutcome::NoAnswer(missing) => {
                turn.status = "error";
                turn.failure = Some(missing.failure);
            }
            TaskOutcome::Failed(failed) => {
                turn.status = "error";
                turn.failure = Some(failed.failure);
            }
        }
    }

    pub fn get(&self, turn_id: u64) -> Option<&Turn> {
        self.turns.iter().find(|t| t.turn_id == turn_id)
    }

    /// Record which model the router chose, as soon as it says so.
    pub fn set_provider(&mut self, turn_id: u64, provider: ProviderInfo) {
        if let Some(turn) = self.turns.iter_mut().find(|t| t.turn_id == turn_id) {
            turn.provider = provider;
        }
    }

    /// Mark every turn of one tab stale. Called when that tab's document is
    /// replaced; the answer text stays, the citation chips stop being actionable.
    pub fn invalidate_tab(&mut self, tab_id: u64) -> Vec<u64> {
        let mut ids = Vec::new();
        for turn in self.turns.iter_mut().filter(|t| t.tab_id == tab_id && !t.stale) {
            turn.stale = true;
            ids.push(turn.turn_id);
        }
        ids
    }

    /// Where to scroll for one citation, or the typed refusal.
    ///
    /// Order matters: the store is consulted for the turn, then the contract
    /// re-checks identity against live state, and only then is a scroll offset
    /// produced. A refusal therefore never reaches the script that would move the
    /// page.
    /// A citation into another tab that the question included (`t<tab>.<block>`):
    /// the tab, the scroll offset and the flash rect. `None` for an ordinary citation.
    /// The answer itself must still be live; the caller checks the other tab's document.
    pub fn foreign_citation(
        &self,
        identity: &Identity,
        turn_id: u64,
        citation_id: usize,
    ) -> Result<Option<(u64, f64, [f64; 4])>, Failure> {
        let turn = self.get(turn_id).ok_or_else(|| Failure::stale("citation.turn_gone"))?;
        let Some(placement) = turn.placements.get(citation_id) else { return Ok(None) };
        let Some(id) = placement.block_id.as_deref() else { return Ok(None) };
        let Some(tab) = id.strip_prefix('t').and_then(|r| r.split_once('.')).and_then(|(t, _)| t.parse::<u64>().ok()) else {
            return Ok(None);
        };
        if placement.h <= 0.0 && placement.y <= 0.0 {
            return Err(Failure::unavailable("citation.block_missing"));
        }
        let answer = turn.answer.as_ref().ok_or_else(|| Failure::stale("citation.turn_gone"))?;
        let scope = identity
            .scope()
            .ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        answer.verify(identity.registry(), &scope)?;
        let y = (placement.y - super::context::SCROLL_MARGIN).max(0.0).round();
        Ok(Some((tab, y, [placement.x, placement.y, placement.w, placement.h])))
    }

    #[cfg(windows)]
    pub fn native_citation_target(&self, identity: &Identity, turn_id: u64, citation_id: usize) -> Result<(u64, String), Failure> {
        let turn = self.get(turn_id).ok_or_else(|| Failure::stale("citation.turn_gone"))?;
        let answer = turn.answer.as_ref().ok_or_else(|| Failure::stale("citation.turn_gone"))?;
        let scope = identity.scope().ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        let trusted = answer.verify(identity.registry(), &scope)?;
        let citation = trusted.citations().get(citation_id).ok_or_else(|| Failure::stale("citation.block_missing"))?;
        let document = citation.document().document_token();
        let tab = identity.tab_of_document(document).ok_or_else(|| Failure::stale("citation.document_missing"))?;
        identity.resolve_in_tab(document, tab)?;
        Ok((tab, answer.citations[citation_id].anchor.quote.clone()))
    }

    pub fn scroll_y(&self, identity: &Identity, turn_id: u64, citation_id: usize) -> Result<f64, Failure> {
        let turn = self
            .get(turn_id)
            .ok_or_else(|| Failure::stale("citation.turn_gone"))?;
        let answer = turn
            .answer
            .as_ref()
            .ok_or_else(|| Failure::stale("citation.turn_gone"))?;
        let scope = identity
            .scope()
            .ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        // The one staleness implementation. Its first refusal wins, and its
        // `identity.token_retired` is reported as `citation.navigation_sequence_mismatch`
        // by the contract itself, so the code the user sees is the code the
        // invariant uses.
        let trusted = answer.verify(identity.registry(), &scope)?;
        let citation = trusted
            .citations()
            .get(citation_id)
            .ok_or_else(|| Failure::stale("citation.block_missing"))?;
        // The document the click was asked for must be the document this tab is
        // showing. `verify` proves the token is live; this proves it is *here*.
        identity
            .resolve_in_tab(citation.document().document_token(), turn.tab_id)
            .map_err(|failure| {
                failure.with_detail("the cited document is not the one in this tab")
            })?;
        let placement = turn
            .placements
            .get(citation_id)
            .ok_or_else(|| Failure::unavailable("citation.placement_lost"))?;
        let y = match placement.block_id.as_deref() {
            Some(_) if placement.h > 0.0 || placement.y > 0.0 => placement.y,
            // A block id that is not in the recorded geometry has no honest offset.
            // Scrolling to 0 would be a wrong answer with a smooth animation.
            _ => return Err(Failure::unavailable("citation.block_missing")),
        };
        Ok((y - super::context::SCROLL_MARGIN).max(0.0).round())
    }

    /// The overlay payload for one turn, carrying every fact the card shows: the
    /// answer, its provenance, and the citations with the identity they were
    /// checked against.
    pub fn payload(&self, turn_id: u64) -> Value {
        let Some(turn) = self.get(turn_id) else { return json!({}) };
        let citations: Vec<Value> = turn
            .answer
            .as_ref()
            .map(|answer| {
                answer
                    .citations
                    .iter()
                    .enumerate()
                    .filter_map(|(index, citation)| {
                        let placement = turn.placements.get(index)?;
                        Some(json!({
                            "citation_id": index,
                            "ordinal": index + 1,
                            "block_id": citation.anchor.block_id,
                            "quote": citation.anchor.quote,
                            "prefix": citation.anchor.prefix,
                            "suffix": citation.anchor.suffix,
                            "text_position": citation.anchor.text_position,
                            "document": citation.anchor.document.0,
                            "snapshot_version": citation.anchor.snapshot_version.0,
                            "navigation_sequence": citation.navigation_sequence,
                            "rect": {
                                "x": placement.x, "y": placement.y,
                                "w": placement.w, "h": placement.h,
                            },
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "turn_id": turn.turn_id,
            "task_id": turn.task.0,
            "tab_id": turn.tab_id,
            "status": turn.status,
            "question": turn.question,
            "text": turn.answer.as_ref().map(|a| a.answer.clone()).unwrap_or_default(),
            "provider": turn.provider.kind,
            "egress": turn.provider.egress,
            "model": turn.provider.model,
            "elapsed_ms": turn.elapsed_ms,
            "citations": citations,
            "failure": turn.failure.as_ref().map(failure_json),
            "stale": turn.stale,
        })
    }

    /// The number of turns kept, for the test that asserts the bound holds.
    pub fn len(&self) -> usize {
        self.turns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }
}

/// The wire shape of a failure: kind, code, the two advice flags, and the reason.
pub fn failure_json(failure: &Failure) -> Value {
    json!({
        "kind": failure.kind.as_str(),
        "code": failure.code,
        "retryable": failure.retryable,
        "replan_required": failure.replan_required,
        "detail": failure.detail,
    })
}

/// The message the overlay shows for a refusal: `"<kind>/<code> — <detail>"`, with
/// the detail dropped when there is none. One formatter so a rejection reads the
/// same wherever it came from.
pub fn failure_message(failure: &Failure) -> String {
    match &failure.detail {
        Some(detail) => format!("{}/{} — {detail}", failure.kind.as_str(), failure.code),
        None => format!("{}/{}", failure.kind.as_str(), failure.code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_platform_contract::contract::{AnswerCitation, CitationRole};
    use crate::ai::context::Block;

    fn context_with(token: ai_platform_contract::identity::DocumentToken, version: u64) -> PageContext {
        PageContext {
            document: token,
            snapshot_version: SnapshotVersion(version),
            navigation_sequence: 1,
            url: "https://example.com/".to_string(),
            title: "Example".to_string(),
            blocks: vec![Block {
                block_id: "b0".to_string(),
                text: "Example domains are reserved for documentation purposes.".to_string(),
                x: 90.0,
                y: 277.0,
                w: 1100.0,
                h: 44.0,
            }],
            links: vec![],
            frames_not_read: 0,
            dropped_blocks: 0,
        }
    }

    fn answer_for(context: &PageContext, sequence: u64) -> AnswerWithCitations {
        AnswerWithCitations {
            task: TaskId(1),
            answer: "It is reserved for documentation.".to_string(),
            citations: vec![AnswerCitation {
                anchor: ai_platform_contract::contract::AnnotationAnchor {
                    document: context.document,
                    snapshot_version: context.snapshot_version,
                    block_id: Some("b0".to_string()),
                    quote: context.blocks[0].text.clone(),
                    prefix: None,
                    suffix: None,
                    text_position: None,
                },
                navigation_sequence: sequence,
                role: CitationRole::Supporting,
            }],
            limit: None,
        }
    }

    /// A live citation scrolls to where the capture said the passage was.
    #[test]
    fn a_live_citation_resolves_to_the_capture_geometry() {
        let mut identity = Identity::new();
        let key = identity.on_document_committed(1, "https://example.com/").unwrap();
        let version = identity.snapshot_captured(key.document_token()).unwrap();
        let context = context_with(key.document_token(), version.0);
        let task = identity.create_task().unwrap();

        let mut store = AnswerStore::default();
        let turn = store.begin(
            task,
            1,
            "what is this for",
            version,
            ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false },
        );
        store.finish(
            turn,
            TaskOutcome::Answered(answer_for(&context, key.navigation_sequence())),
            12,
            &context,
        );

        let y = store.scroll_y(&identity, turn, 0).expect("a live citation scrolls");
        // The block's y minus the composer margin, exactly as `Block::scroll_y`
        // promises — the click lands on the passage, not 24 px under it.
        assert_eq!(y, 253.0);
    }

    #[cfg(windows)]
    #[test]
    fn native_pdf_citations_use_verified_text_without_requiring_dom_geometry() {
        let mut identity = Identity::new();
        let key = identity.on_document_committed(1, "file:///C:/document.pdf").unwrap();
        let version = identity.snapshot_captured(key.document_token()).unwrap();
        let mut context = context_with(key.document_token(), version.0);
        context.blocks[0].y = 0.0;
        context.blocks[0].h = 0.0;
        let task = identity.create_task().unwrap();
        let mut store = AnswerStore::default();
        let turn = store.begin(task, 1, "question", version,
            ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false });
        store.finish(turn, TaskOutcome::Answered(answer_for(&context, key.navigation_sequence())), 12, &context);
        assert_eq!(store.native_citation_target(&identity, turn, 0).unwrap(), (1, context.blocks[0].text.clone()));
        assert!(store.native_citation_target(&identity, turn, 1).is_err());
        identity.on_document_committed(1, "https://example.com/next").unwrap();
        assert!(store.native_citation_target(&identity, turn, 0).is_err(), "native citations must reject a replaced document");
    }

    /// The non-negotiable one: after the tab navigates, the citation refuses. The
    /// page must not move, and the refusal must name the mismatch.
    #[test]
    fn a_citation_is_refused_once_the_document_has_navigated() {
        let mut identity = Identity::new();
        let first = identity.on_document_committed(1, "https://a.example/").unwrap();
        let version = identity.snapshot_captured(first.document_token()).unwrap();
        let context = context_with(first.document_token(), version.0);
        let task = identity.create_task().unwrap();

        let mut store = AnswerStore::default();
        let turn = store.begin(
            task,
            1,
            "what is this for",
            version,
            ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false },
        );
        store.finish(
            turn,
            TaskOutcome::Answered(answer_for(&context, first.navigation_sequence())),
            12,
            &context,
        );
        // Live first, so the test proves it is the navigation that changes the answer.
        assert!(store.scroll_y(&identity, turn, 0).is_ok());

        let moved = identity.on_document_committed(1, "https://b.example/").unwrap();
        assert_eq!(
            store.invalidate_tab(1),
            vec![turn],
            "the tab's turns are marked stale for the UI"
        );

        let refusal = store
            .scroll_y(&identity, turn, 0)
            .expect_err("a citation must not scroll a document it did not quote");
        assert_eq!(
            refusal.code,
            ai_platform_contract::contract::CITATION_STALE_CODE,
            "the contract's own staleness code, not a shell-local invention"
        );
        assert_eq!(refusal.kind, ai_platform_contract::failure::FailureKind::Stale);
        assert!(refusal.replan_required, "a stale plan cannot be re-dispatched as-is");
        assert_ne!(
            moved.navigation_sequence(),
            first.navigation_sequence(),
            "and the sequences really did differ, which is what was checked"
        );
    }

    /// A citation that names a `navigation_sequence` the live document never had is
    /// refused too — the same rule, reached without navigating, because the check
    /// compares numbers rather than trusting a retired token.
    #[test]
    fn a_citation_naming_a_sequence_the_document_never_had_is_refused() {
        let mut identity = Identity::new();
        let key = identity.on_document_committed(1, "https://a.example/").unwrap();
        let version = identity.snapshot_captured(key.document_token()).unwrap();
        let context = context_with(key.document_token(), version.0);
        let task = identity.create_task().unwrap();

        let mut store = AnswerStore::default();
        let turn = store.begin(
            task,
            1,
            "q",
            version,
            ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false },
        );
        store.finish(
            turn,
            // The document's sequence is 1; the answer claims 9.
            TaskOutcome::Answered(answer_for(&context, 9)),
            12,
            &context,
        );

        let refusal = store.scroll_y(&identity, turn, 0).expect_err("sequence 9 never happened");
        assert_eq!(refusal.code, ai_platform_contract::contract::CITATION_STALE_CODE);
        assert!(
            refusal.detail.unwrap().contains("live document is 1"),
            "the refusal says which sequence is live"
        );
    }

    /// A citation quoting a capture the document never reached is a different
    /// refusal, and says so.
    #[test]
    fn a_citation_quoting_an_unseen_capture_is_refused_by_its_own_code() {
        let mut identity = Identity::new();
        let key = identity.on_document_committed(1, "https://a.example/").unwrap();
        let context = context_with(key.document_token(), 7);
        let task = identity.create_task().unwrap();

        let mut store = AnswerStore::default();
        let turn = store.begin(
            task,
            1,
            "q",
            SnapshotVersion(7),
            ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false },
        );
        store.finish(
            turn,
            TaskOutcome::Answered(answer_for(&context, key.navigation_sequence())),
            12,
            &context,
        );

        let refusal = store.scroll_y(&identity, turn, 0).expect_err("version 7 was never captured");
        assert_eq!(
            refusal.code,
            ai_platform_contract::contract::CITATION_CAPTURE_UNSEEN_CODE
        );
    }

    /// An unknown turn id is refused rather than scrolled to a default.
    #[test]
    fn a_turn_that_is_not_there_refuses() {
        let identity = Identity::new();
        let store = AnswerStore::default();
        let refusal = store.scroll_y(&identity, 404, 0).expect_err("no such turn");
        assert_eq!(refusal.code, "citation.turn_gone");
    }

    /// The store is bounded, and the oldest turn goes first.
    #[test]
    fn the_store_keeps_the_newest_turns_and_drops_the_oldest() {
        let mut store = AnswerStore::default();
        let mut ids = Vec::new();
        for n in 0..(MAX_TURNS + 5) {
            ids.push(store.begin(
                TaskId(n as u64 + 1),
                1,
                "q",
                SnapshotVersion(1),
                ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false },
            ));
        }
        assert_eq!(store.len(), MAX_TURNS);
        assert!(store.get(ids[0]).is_none(), "the oldest turn is gone");
        assert!(store.get(*ids.last().unwrap()).is_some(), "the newest is kept");
    }

    /// What the overlay is handed: provenance, citations, and the identity numbers
    /// the chip has to carry for the click to be checkable.
    #[test]
    fn the_payload_carries_the_answer_provenance_and_the_identity_it_quotes() {
        let mut identity = Identity::new();
        let key = identity.on_document_committed(1, "https://example.com/").unwrap();
        let version = identity.snapshot_captured(key.document_token()).unwrap();
        let context = context_with(key.document_token(), version.0);
        let task = identity.create_task().unwrap();

        let mut store = AnswerStore::default();
        let turn = store.begin(
            task,
            1,
            "what is this for",
            version,
            ProviderInfo { kind: "offline", model: "offline-extract".into(), egress: false },
        );
        store.finish(
            turn,
            TaskOutcome::Answered(answer_for(&context, key.navigation_sequence())),
            12,
            &context,
        );
        let payload = store.payload(turn);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["provider"], "offline");
        assert_eq!(payload["egress"], false, "an offline answer must say it did not call out");
        assert_eq!(payload["citations"][0]["ordinal"], 1);
        assert_eq!(payload["citations"][0]["navigation_sequence"], key.navigation_sequence());
        assert_eq!(payload["citations"][0]["rect"]["y"], 277.0);
    }

    /// A no-answer keeps the contract's code and numbers on the wire, because "the
    /// model spent the budget thinking" is a different message from "no answer".
    #[test]
    fn a_no_answer_keeps_its_code_and_budget_on_the_wire() {
        let mut identity = Identity::new();
        let key = identity.on_document_committed(1, "https://a.example/").unwrap();
        let version = identity.snapshot_captured(key.document_token()).unwrap();
        let context = context_with(key.document_token(), version.0);
        let task = identity.create_task().unwrap();

        let mut store = AnswerStore::default();
        let turn = store.begin(
            task,
            1,
            "q",
            version,
            ProviderInfo { kind: "http", model: "served-model".into(), egress: true },
        );
        store.finish(
            turn,
            TaskOutcome::NoAnswer(ai_platform_contract::contract::AnswerNotProduced::budget_consumed_by_reasoning(task, 4096, 427)),
            2052,
            &context,
        );
        let payload = store.payload(turn);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["failure"]["code"], "model.reasoning_consumed_budget");
        assert_eq!(payload["egress"], true);
        assert!(
            !store.scroll_y(&identity, turn, 0).is_ok(),
            "a turn with no answer has no citation to scroll to"
        );
    }
}
