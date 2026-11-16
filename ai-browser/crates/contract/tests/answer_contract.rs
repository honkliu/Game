//! The assistant round trip: `model.complete(params = TaskRequest)` out,
//! `task.finish(params = TaskOutcome)` back, and the citation-staleness rule that
//! decides whether an answer's evidence may be rendered as evidence.
//!
//! Four things these tests exist to stop:
//!
//! 1. **A stale citation being a bool.** The reward for verifying is
//!    [`TrustedCitation`], which has no visible constructor, so the scroll-to-passage
//!    path cannot be wired up without passing the check. Every test that wants success
//!    gets it from `verify`, never by building the type.
//! 2. **The fixture drifting from the codec.** Both golden lines are compared
//!    byte-for-byte against what `frame::encode` produces from a registry built here,
//!    so a hand-edited fixture the code no longer emits fails.
//! 3. **`reasoning_content` finding a home.** The served provider is a reasoning model
//!    whose answer field came back empty; no shape here may hold that text.
//! 4. **`verify` being written against a bool it never got.** Each refusal path is
//!    asserted on its `kind` *and* its `code`, because the UI branches on the code.
//!
//! The fixture scenario, which the assertions re-derive rather than assume: the user
//! asks about `https://example.com/pricing`, the site's "Annual" button is a
//! `history.pushState` route change (backlog §4.3 — a new document, and WebView2
//! reports no `NavigationId` for it), and one of the two citations still quotes the
//! pre-click document.

#![allow(clippy::unwrap_used)]

use ai_platform_contract::contract::{
    ActionLevel, AnnotationAnchor, AnswerCitation, AnswerNotProduced, AnswerWithCitations,
    CancelReason, CancellationToken, CancellationRequest, CitationRole, TaskOutcome, TaskRequest,
    TaskRequestView, CITATION_CAPTURE_UNSEEN_CODE, CITATION_STALE_CODE,
};
use ai_platform_contract::failure::{Failure, FailureKind};
use ai_platform_contract::frame::{self, Frame, Method, Notification, Peer, Request};
use ai_platform_contract::identity::{
    DocumentIdentityRegistry, DocumentKey, DocumentToken, ProfileId, RequestScope, SnapshotVersion,
    TabId, TaskId, UpstreamNavigationId, WorkspaceId,
};

const PRICING: &str = "https://example.com/pricing";
const PRICING_ANNUAL: &str = "https://example.com/pricing#annual";
const QUESTION: &str = "Is the annual plan cheaper than paying monthly?";
const ANSWER: &str = "Yes. The annual plan is $99 a year, which the pricing table puts about \
    17% below twelve monthly payments.";
const LIVE_QUOTE: &str = "$99 a year, about 17% below twelve monthly payments";

/// The request the shell sent, exactly as it went out on the pipe.
const REQUEST_FIXTURE: &str =
    include_str!("fixtures/round_trip_ask_and_answer.model_complete.request.json");
/// The answer that came back: two citations, the second quoting the document the route
/// change replaced.
const FINISH_FIXTURE: &str = include_str!("fixtures/round_trip_ask_and_answer.task.finish.json");
/// The same task against a provider that spent its whole completion budget on hidden
/// reasoning and answered nothing: `content` empty, `reasoning_content` non-empty,
/// `finish_reason` "length", `usage.reasoning_tokens` 9 against an 8-token budget.
const NO_ANSWER_FIXTURE: &str = include_str!("fixtures/no_answer_after_reasoning.task.finish.json");

struct Scenario {
    registry: DocumentIdentityRegistry,
    scope: RequestScope,
    tab: TabId,
    task: TaskId,
    /// The document the question was asked about — live.
    live: DocumentKey,
    live_version: SnapshotVersion,
    /// The document the route change replaced.
    replaced: DocumentKey,
    replaced_version: SnapshotVersion,
}

/// Builds the fixture's world. The token, sequence, and version numbers the golden
/// files name are what this registry actually mints — asserted below, not assumed.
fn scenario() -> Scenario {
    let mut registry = DocumentIdentityRegistry::new();
    let profile = registry.create_profile();
    let workspace = registry.create_workspace(profile).unwrap();
    let tab = registry.open_tab(workspace).unwrap();
    let scope = RequestScope { profile_id: profile, workspace_id: workspace };

    let replaced =
        registry.on_document_committed(tab, 41, Some(UpstreamNavigationId(501)), PRICING).unwrap();
    let replaced_version =
        registry.next_snapshot_version(replaced.document_token(), &scope).unwrap();

    // The user clicks "Annual". WebView2 raises no navigation for a pushState route
    // change, so the shell learns of it from injected script and mints the document.
    let live = registry.on_history_state_changed(tab, PRICING_ANNUAL).unwrap();
    let live_version = registry.next_snapshot_version(live.document_token(), &scope).unwrap();
    let task = registry.create_task(workspace).unwrap();

    assert_eq!(replaced.document_token(), DocumentToken(1));
    assert_eq!(replaced.navigation_sequence(), 1);
    assert_eq!(live.document_token(), DocumentToken(2));
    assert_eq!(live.navigation_sequence(), 2);
    assert_eq!(live.upstream_navigation_id(), None, "pushState carries no WebView2 id");
    assert_eq!(live.frame_tree_node_id(), replaced.frame_tree_node_id());
    assert_eq!(live_version, SnapshotVersion(1));
    assert_eq!(replaced_version, SnapshotVersion(1));
    assert_eq!(task, TaskId(1));

    Scenario { registry, scope, tab, task, live, live_version, replaced, replaced_version }
}

fn citation(
    document: DocumentToken,
    navigation_sequence: u64,
    snapshot_version: u64,
    block_id: &str,
    quote: &str,
    role: CitationRole,
) -> AnswerCitation {
    AnswerCitation {
        anchor: AnnotationAnchor {
            document,
            snapshot_version: SnapshotVersion(snapshot_version),
            block_id: Some(block_id.to_string()),
            quote: quote.to_string(),
            prefix: None,
            suffix: None,
            text_position: None,
        },
        navigation_sequence,
        role,
    }
}

/// The live citation: block `b17` of the document now on screen.
fn live_citation(f: &Scenario) -> AnswerCitation {
    let mut c = citation(
        f.live.document_token(),
        f.live.navigation_sequence(),
        f.live_version.0,
        "b17",
        LIVE_QUOTE,
        CitationRole::DirectAnswer,
    );
    c.anchor.prefix = Some("The pricing table lists ".to_string());
    c.anchor.suffix = Some(" for the annual plan.".to_string());
    c
}

/// The deliberately stale one: block `b3` of the document the route change replaced.
fn replaced_citation(f: &Scenario) -> AnswerCitation {
    let mut c = citation(
        f.replaced.document_token(),
        f.replaced.navigation_sequence(),
        f.replaced_version.0,
        "b3",
        "Monthly billing is $9.99 with no discount",
        CitationRole::Counterpoint,
    );
    c.anchor.text_position = Some(1_024);
    c
}

fn answer_with(citations: Vec<AnswerCitation>) -> AnswerWithCitations {
    AnswerWithCitations { task: TaskId(1), answer: ANSWER.to_string(), citations, limit: None }
}

fn request(task: TaskId, document: DocumentKey, ceiling: ActionLevel) -> TaskRequest {
    TaskRequest {
        task,
        question: QUESTION.to_string(),
        document,
        snapshot_version: SnapshotVersion(1),
        action_ceiling: ceiling,
        cancel: CancellationToken { task, deadline_ms_since_epoch: 1_794_355_260_000 },
        options: Default::default(),
    }
}

fn decoded_params(line: &str, from: Peer) -> serde_json::Value {
    match frame::decode(line, from).unwrap() {
        Frame::Notification(Notification { params, .. }) => params,
        Frame::Request(Request { params, .. }) => params,
        other => panic!("expected a request or notification, got {other:?}"),
    }
}

fn fixture(line: &str) -> &str {
    line.trim_end_matches('\n')
}

/// Every citation refusal the UI has to handle is `stale`, and `Failure::stale` is the
/// only constructor that can produce one — so `replan_required` is asserted here rather
/// than at each call site, which is the point of putting the rule in the constructor.
fn assert_stale_refusal(error: Failure, expect_in_detail: &[&str]) {
    assert_eq!(error.kind, FailureKind::Stale, "{error:?}");
    assert_eq!(error.code, CITATION_STALE_CODE);
    assert!(error.replan_required, "a stale citation must force a replan");
    assert!(!error.retryable, "re-asking about the same document returns the same stale quote");
    let detail = error.detail.unwrap_or_default();
    for needle in expect_in_detail {
        assert!(detail.contains(needle), "detail {detail:?} should name {needle:?}");
    }
}

/// Every object key at every depth, so "this shape cannot hold X" is a claim about the
/// shape rather than about one level of it.
fn all_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys = Vec::new();
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                keys.push(key.clone());
                keys.extend(all_keys(child));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                keys.extend(all_keys(item));
            }
        }
        _ => {}
    }
    keys
}

// --- The golden wire. ---

/// The request fixture is what the codec emits for this scenario, not a sketch of it.
#[test]
fn the_request_fixture_is_what_the_shell_actually_sends() {
    let f = scenario();
    let line = frame::encode(&Frame::request(
        1,
        Method::CompleteModel,
        &request(f.task, f.live, ActionLevel::Navigate).to_params().unwrap(),
    ))
    .unwrap();
    assert_eq!(line, fixture(REQUEST_FIXTURE));

    // The frame the fixture describes comes from the shell; the utility may not send it.
    let decoded = frame::decode(fixture(REQUEST_FIXTURE), Peer::Shell).unwrap();
    let Frame::Request(decoded) = decoded else { panic!("expected a request") };
    assert_eq!(decoded.method, Method::CompleteModel);
    assert!(!decoded.method.is_notification());
    assert_eq!(
        frame::decode(fixture(REQUEST_FIXTURE), Peer::Utility).unwrap_err().code,
        "transport.method_from_wrong_side"
    );
}

/// Same for the answer, in both citation states.
#[test]
fn the_answer_fixture_is_what_the_utility_actually_sends() {
    let f = scenario();
    let outcome =
        TaskOutcome::Answered(answer_with(vec![live_citation(&f), replaced_citation(&f)]));
    let line =
        frame::encode(&Frame::notification(Method::FinishTask, &outcome.to_params().unwrap()))
            .unwrap();
    assert_eq!(line, fixture(FINISH_FIXTURE));

    // `task.finish` is a notification, so it may not arrive as a request.
    assert!(Method::FinishTask.is_notification());
    let request_form = fixture(FINISH_FIXTURE).replacen("notification", "request", 1);
    assert_eq!(
        frame::decode(&request_form, Peer::Utility).unwrap_err().code,
        "transport.frame_kind_mismatch"
    );
}

/// A major version this build does not speak is refused without reading the payload,
/// on both halves of the round trip and in both directions.
#[test]
fn an_unsupported_major_rejects_both_halves_before_the_payload_is_read() {
    for (label, line, peer) in [
        ("request", REQUEST_FIXTURE, Peer::Shell),
        ("finish", FINISH_FIXTURE, Peer::Utility),
    ] {
        for bad_major in [2u16, 0] {
            let bumped = fixture(line).replacen(
                r#""schema_version":{"major":1,"minor":0}"#,
                &format!(r#""schema_version":{{"major":{bad_major},"minor":0}}"#),
                1,
            );
            assert_ne!(bumped, fixture(line), "{label} fixture should carry major 1");
            let error = frame::decode(&bumped, peer).unwrap_err();
            assert_eq!(error.kind, FailureKind::Unsupported, "{label} at major {bad_major}");
            assert_eq!(error.code, "schema.major_unsupported");
            assert!(
                error.detail.unwrap().contains("payload was not interpreted"),
                "{label} at major {bad_major} must not be partially parsed"
            );
        }
    }
    // Negative control: the same frames at major 1 reach interpretation fine, so the
    // version code is not just what every rejection looks like.
    assert!(frame::decode(fixture(REQUEST_FIXTURE), Peer::Shell).is_ok());
    assert!(frame::decode(fixture(FINISH_FIXTURE), Peer::Utility).is_ok());
}

// --- The request side: what the peer may hold. ---

/// The utility's inbound type is the shell's bytes with the authority downgraded to a
/// claim: identical JSON, different Rust type, and every field in it still has to be
/// resolved before it means anything.
#[test]
fn the_utility_view_of_the_request_parses_the_shells_bytes_and_holds_no_authority() {
    let f = scenario();
    let built = request(f.task, f.live, ActionLevel::Navigate);
    let params = decoded_params(fixture(REQUEST_FIXTURE), Peer::Shell);
    let view: TaskRequestView = serde_json::from_value(params.clone()).unwrap();
    assert_eq!(view.task, TaskId(1));
    assert_eq!(view.question, QUESTION);
    assert_eq!(view.document.document_token, DocumentToken(2));
    assert_eq!(view.document.navigation_sequence, 2, "the sequence a citation must echo");
    assert_eq!(view.document.upstream_navigation_id, None, "pushState carries no WebView2 id");
    assert_eq!(view.document.tab_id, f.tab);
    assert_eq!(view.snapshot_version, SnapshotVersion(1));
    assert_eq!(view.action_ceiling, ActionLevel::Navigate);

    // The projection is information-preserving, so there is no second spelling of the
    // request that could drift from the first.
    assert_eq!(TaskRequestView::of(&built), view);
    let view_json = serde_json::to_value(&view).unwrap();
    assert_eq!(
        view_json,
        serde_json::to_value(&built).unwrap(),
        "a view and a request must be the same bytes"
    );
    let mut claim_fields: Vec<&str> =
        view_json["document"].as_object().unwrap().keys().map(String::as_str).collect();
    claim_fields.sort_unstable();
    assert_eq!(
        claim_fields,
        vec![
            "document_token",
            "frame_tree_node_id",
            "navigation_sequence",
            "profile_id",
            "tab_id",
            "upstream_navigation_id",
            "workspace_id"
        ],
        "the claim mirrors the key field-for-field"
    );

    // And a claim is only an index into somebody's registry. In a registry that never
    // minted it, it resolves to nothing; in a scope rebuilt from frame-readable
    // numbers, it is somebody else's document.
    let empty_registry = DocumentIdentityRegistry::new();
    assert_eq!(
        empty_registry.resolve(view.document.document_token, &f.scope).unwrap_err().code,
        "identity.token_unknown"
    );
    assert_eq!(
        f.registry
            .resolve(
                view.document.document_token,
                &RequestScope { profile_id: ProfileId(1), workspace_id: WorkspaceId(4_242) }
            )
            .unwrap_err()
            .code,
        "identity.cross_scope"
    );
}

/// The ceiling is the shell's policy statement, and it fails closed in both directions:
/// an ungraded action kind gets nothing, and a ceiling above what Release 0 permits
/// grants nothing rather than everything.
#[test]
fn the_action_ceiling_fails_closed_in_both_directions() {
    let f = scenario();
    let navigate = request(f.task, f.live, ActionLevel::Navigate);
    assert!(navigate.ceiling_permits("read_document"));
    assert!(navigate.ceiling_permits("scroll_to"), "Release 0 ships citation scroll-to");
    assert!(!navigate.ceiling_permits("type_text"), "above the ceiling");
    assert!(!navigate.ceiling_permits("harvest_cookies"), "ungraded kinds get nothing");

    let observe_only = TaskRequest { action_ceiling: ActionLevel::Observe, ..navigate.clone() };
    assert!(!observe_only.ceiling_permits("scroll_to"), "the ceiling is honoured downward too");
    assert!(observe_only.ceiling_permits("read_document"));

    // A ceiling that claims more than Release 0 allows is not a grant.
    let overreaching = TaskRequest { action_ceiling: ActionLevel::Commit, ..navigate.clone() };
    assert!(!overreaching.ceiling_permits("scroll_to"), "a TaskRequest cannot widen Release 0");
    assert!(!overreaching.ceiling_permits("purchase"));
}

#[test]
fn a_cancellation_token_stops_only_its_own_task() {
    let token = CancellationToken { task: TaskId(7), deadline_ms_since_epoch: 1_000 };
    assert!(token.matches(&CancellationRequest { task: TaskId(7), reason: CancelReason::UserStopped }));
    assert!(
        token.matches(&CancellationRequest { task: TaskId(7), reason: CancelReason::PolicyRevoked }),
        "whatever the reason, the work stops; the reason is for the UI"
    );
    assert!(!token.matches(&CancellationRequest { task: TaskId(8), reason: CancelReason::UserStopped }));

    // `0` is the unset sentinel in every clock field here, so "no deadline" and
    // "already expired" must not collide. A real bound of 1 does expire.
    let unbounded = CancellationToken { task: TaskId(7), deadline_ms_since_epoch: 0 };
    assert!(!unbounded.expired_at(0));
    assert!(!unbounded.expired_at(i64::MAX));
    assert!(CancellationToken { task: TaskId(7), deadline_ms_since_epoch: 1 }.expired_at(1));
    assert!(!token.expired_at(999));
    assert!(token.expired_at(1_000));
}

// --- The answer side: the staleness invariant. ---

/// The fixture's second citation quotes the document the route change replaced, so the
/// answer is refused outright — by type, with no bool for a caller to miss.
#[test]
fn the_fixture_answer_is_refused_because_one_citation_is_stale() {
    let f = scenario();
    let params = decoded_params(fixture(FINISH_FIXTURE), Peer::Utility);
    let TaskOutcome::Answered(answer) = serde_json::from_value::<TaskOutcome>(params).unwrap()
    else {
        panic!("the fixture is an answered outcome")
    };
    assert_eq!(answer.citations.len(), 2);
    assert_eq!(answer.citations[0].navigation_sequence, 2);
    assert_eq!(answer.citations[1].navigation_sequence, 1, "the stale one, by construction");

    let error = answer.verify(&f.registry, &f.scope).unwrap_err();
    assert_stale_refusal(error, &["citation 1", "identity.token_retired"]);
}

/// Deleting the stale citation leaves evidence that can be acted on, and the only road
/// there is `verify`.
#[test]
fn dropping_the_stale_citation_leaves_trusted_evidence() {
    let f = scenario();
    let params = decoded_params(fixture(FINISH_FIXTURE), Peer::Utility);
    let TaskOutcome::Answered(mut answer) =
        serde_json::from_value::<TaskOutcome>(params).unwrap()
    else {
        panic!("the fixture is an answered outcome")
    };
    answer.citations.retain(|c| c.anchor.document == f.live.document_token());
    assert_eq!(answer.citations.len(), 1);

    let trusted = answer.verify(&f.registry, &f.scope).unwrap();
    assert_eq!(trusted.answer(), ANSWER);
    assert_eq!(trusted.len(), 1);
    assert!(!trusted.is_empty());
    let citation = &trusted.citations()[0];
    assert_eq!(citation.quote(), LIVE_QUOTE);
    assert_eq!(citation.role(), CitationRole::DirectAnswer);
    assert_eq!(citation.block_id(), Some("b17"), "the scroll_to target survives verification");
    // What a bool could not hand back: the resolved document, ready for dispatch.
    assert_eq!(citation.document(), f.live);
    assert_eq!(citation.anchor().prefix.as_deref(), Some("The pricing table lists "));
}

/// The invariant in its purest form: the document is live and in scope, and the only
/// thing wrong is the sequence the citation claims.
#[test]
fn a_live_document_with_a_mis_stated_sequence_is_stale() {
    let f = scenario();
    assert!(f.registry.resolve(f.live.document_token(), &f.scope).is_ok(), "the document is live");

    let lying = citation(
        f.live.document_token(),
        f.live.navigation_sequence() - 1,
        f.live_version.0,
        "b17",
        LIVE_QUOTE,
        CitationRole::Supporting,
    );
    assert_stale_refusal(
        answer_with(vec![lying]).verify(&f.registry, &f.scope).unwrap_err(),
        &["citation 0", "sequence 1 but the live document is 2"],
    );

    // The honest version of the same citation passes.
    let honest = citation(
        f.live.document_token(),
        f.live.navigation_sequence(),
        f.live_version.0,
        "b17",
        LIVE_QUOTE,
        CitationRole::Supporting,
    );
    assert_eq!(answer_with(vec![honest]).verify(&f.registry, &f.scope).unwrap().len(), 1);
}

/// A citation naming a capture the document never had is stale under its own code: the
/// document may be perfectly live and the snapshot claim still false.
#[test]
fn a_citation_naming_a_capture_that_never_happened_is_stale() {
    let mut f = scenario();

    // Version 0 is the "nothing captured yet" sentinel and is never quotable.
    let none = citation(
        f.live.document_token(),
        f.live.navigation_sequence(),
        SnapshotVersion::NONE,
        "b17",
        LIVE_QUOTE,
        CitationRole::Supporting,
    );
    let error = answer_with(vec![none]).verify(&f.registry, &f.scope).unwrap_err();
    assert_eq!(error.kind, FailureKind::Stale);
    assert_eq!(error.code, CITATION_CAPTURE_UNSEEN_CODE);
    assert!(error.replan_required);
    assert!(error.detail.unwrap().contains("highest is 1"));

    // A version above what the document reached is the same lie by a different route.
    let ahead = citation(
        f.live.document_token(),
        f.live.navigation_sequence(),
        f.live_version.0 + 1,
        "b17",
        LIVE_QUOTE,
        CitationRole::Supporting,
    );
    let error = answer_with(vec![ahead]).verify(&f.registry, &f.scope).unwrap_err();
    assert_eq!(error.code, CITATION_CAPTURE_UNSEEN_CODE);

    // A live document that has never been captured at all cannot be quoted from, even
    // though a citation naming version 1 looks well-formed.
    let fresh_tab = f.registry.open_tab(f.scope.workspace_id).unwrap();
    let fresh = f.registry.on_document_committed(fresh_tab, 51, None, "https://example.com/blank").unwrap();
    let uncaptured = citation(
        fresh.document_token(),
        fresh.navigation_sequence(),
        1,
        "b1",
        "text that cannot exist yet",
        CitationRole::Supporting,
    );
    let error = answer_with(vec![uncaptured]).verify(&f.registry, &f.scope).unwrap_err();
    assert_eq!(error.code, CITATION_CAPTURE_UNSEEN_CODE);
    assert!(error.detail.unwrap().contains("nothing has been captured"));

    // An *older* capture of the same live document is not a lie: scrolling makes newer
    // versions, and backlog §5 labels that, it does not refuse it.
    let second = f.registry.next_snapshot_version(f.live.document_token(), &f.scope).unwrap();
    assert_eq!(second, SnapshotVersion(2));
    let older = citation(
        f.live.document_token(),
        f.live.navigation_sequence(),
        1,
        "b17",
        LIVE_QUOTE,
        CitationRole::Supporting,
    );
    assert_eq!(answer_with(vec![older]).verify(&f.registry, &f.scope).unwrap().len(), 1);
}

/// A citation reaching outside the scope the frame arrived on is a substitution
/// attempt. It keeps `blocked` and must not be softened to stale, or the shell cannot
/// tell an attack from the user clicking a link.
#[test]
fn a_citation_from_another_workspace_is_blocked_not_stale() {
    let mut f = scenario();
    let foreign_workspace = f.registry.create_workspace(f.scope.profile_id).unwrap();
    let foreign_tab = f.registry.open_tab(foreign_workspace).unwrap();
    let foreign = f.registry.on_document_committed(foreign_tab, 61, None, PRICING).unwrap();

    let borrowed = citation(
        foreign.document_token(),
        foreign.navigation_sequence(),
        1,
        "b1",
        "cheaper over there",
        CitationRole::Supporting,
    );
    let error = answer_with(vec![borrowed]).verify(&f.registry, &f.scope).unwrap_err();
    assert_eq!(error.kind, FailureKind::Blocked);
    assert_eq!(error.code, "identity.cross_scope");
    assert!(!error.replan_required, "replanning inside this workspace cannot reach it");
}

/// A task id is scoped too, and a workspace close deletes its tasks — so a late answer
/// naming a task the user already cleared has nowhere to land.
#[test]
fn an_answer_for_a_task_this_workspace_does_not_own_is_refused() {
    let mut f = scenario();
    let error = AnswerWithCitations { task: TaskId(999), ..answer_with(vec![live_citation(&f)]) }
        .verify(&f.registry, &f.scope)
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Stale);
    assert_eq!(error.code, "identity.task_unknown");

    let other_workspace = f.registry.create_workspace(f.scope.profile_id).unwrap();
    let foreign_task = f.registry.create_task(other_workspace).unwrap();
    let error = AnswerWithCitations { task: foreign_task, ..answer_with(vec![live_citation(&f)]) }
        .verify(&f.registry, &f.scope)
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Blocked);
    assert_eq!(error.code, "identity.task_cross_workspace");

    // One live document in this workspace: the route change already retired the other.
    assert_eq!(f.registry.on_workspace_closed(f.scope.workspace_id), 1);
    assert_eq!(
        answer_with(vec![live_citation(&f)]).verify(&f.registry, &f.scope).unwrap_err().code,
        "identity.task_unknown",
        "closing the workspace must retire the task its answers name"
    );
}

/// Citations may legitimately come from several live documents in one workspace — that
/// is the point of a workspace-scoped grant — so verification is per citation.
#[test]
fn citations_from_two_live_documents_in_one_workspace_both_trust() {
    let mut f = scenario();
    let sibling_tab = f.registry.open_tab(f.scope.workspace_id).unwrap();
    let sibling = f
        .registry
        .on_document_committed(sibling_tab, 71, None, "https://example.com/reviews")
        .unwrap();
    let sibling_version =
        f.registry.next_snapshot_version(sibling.document_token(), &f.scope).unwrap();

    let answer = answer_with(vec![
        live_citation(&f),
        citation(
            sibling.document_token(),
            sibling.navigation_sequence(),
            sibling_version.0,
            "r2",
            "4.6 out of 5 across 1,120 reviews",
            CitationRole::Supporting,
        ),
    ]);
    let trusted = answer.verify(&f.registry, &f.scope).unwrap();
    assert_eq!(trusted.len(), 2);
    assert_eq!(trusted.citations()[1].document(), sibling);

    // The instant that tab navigates, its citation stops being evidence while the first
    // is untouched — exactly why this is not one bool per answer.
    f.registry
        .on_document_committed(sibling_tab, 71, None, "https://example.com/reviews?page=2")
        .unwrap();
    assert_stale_refusal(answer.verify(&f.registry, &f.scope).unwrap_err(), &["citation 1"]);
    let alone = AnswerWithCitations { citations: vec![live_citation(&f)], ..answer };
    assert_eq!(alone.verify(&f.registry, &f.scope).unwrap().len(), 1);
}

/// An answer may cite nothing at all — model knowledge rather than page content. Whether
/// *this* answer must cite something is a product rule above the contract.
#[test]
fn an_uncited_answer_verifies_and_carries_no_evidence() {
    let f = scenario();
    let trusted = answer_with(vec![]).verify(&f.registry, &f.scope).unwrap();
    assert!(trusted.is_empty());
    assert_eq!(trusted.len(), 0);
    assert_eq!(trusted.answer(), ANSWER);
}

/// Trust is not a wire property. A serialised `TrustedAnswer` is content for the
/// overlay; the same content has to be checked again against whatever is live now.
#[test]
fn trust_does_not_survive_the_user_navigating() {
    let mut f = scenario();
    let answer = answer_with(vec![live_citation(&f)]);
    let trusted = answer.verify(&f.registry, &f.scope).unwrap();
    let json = serde_json::to_value(&trusted).unwrap();
    assert_eq!(json["citations"][0]["document"]["navigation_sequence"], serde_json::json!(2));
    assert_eq!(json["citations"][0]["role"], serde_json::json!("direct_answer"));
    assert_eq!(json["answer"], serde_json::json!(ANSWER));

    // The user clicks through while the answer card is still open.
    f.registry
        .on_document_committed(f.tab, 41, Some(UpstreamNavigationId(502)), "https://example.com/checkout")
        .unwrap();
    assert_stale_refusal(answer.verify(&f.registry, &f.scope).unwrap_err(), &["citation 0"]);
    assert_eq!(
        f.registry.assess_staleness(&f.live, f.live_version),
        ai_platform_contract::identity::StaleStatus::DocumentReplaced,
        "the label the UI shows and the refusal the dispatcher sees must agree"
    );
}

// --- "No answer" is a state, not a string. ---

/// The empty-`content` case must not be reachable as an `Answered` arm.
#[test]
fn an_empty_answer_is_refused_as_no_answer_rather_than_rendered() {
    let f = scenario();
    for blank in ["", "   ", "\n\t "] {
        let answer = AnswerWithCitations {
            task: f.task,
            answer: blank.to_string(),
            citations: vec![live_citation(&f)],
            limit: None,
        };
        let error = answer.verify(&f.registry, &f.scope).unwrap_err();
        assert_eq!(error.kind, FailureKind::NoAnswer, "{blank:?} must not render as an answer");
        assert_eq!(error.code, "model.answer_absent");
        assert!(!error.retryable, "the same request returns the same nothing");
        assert!(!error.replan_required, "the plan was sound; the call needs a bigger budget");
    }
}

/// The measured probe: an 8-token budget, `content` empty, all 9 usage tokens spent on
/// reasoning. The fixture carries the two numbers the shell can act on and none of the
/// text the model did not answer with.
#[test]
fn the_no_answer_fixture_is_the_measured_probe_shape() {
    let params = decoded_params(fixture(NO_ANSWER_FIXTURE), Peer::Utility);
    let outcome: TaskOutcome = serde_json::from_value(params).unwrap();
    assert_eq!(outcome.task(), TaskId(1));
    let TaskOutcome::NoAnswer(detail) = outcome else { panic!("the fixture is a no-answer") };
    assert_eq!(detail.failure.kind, FailureKind::NoAnswer);
    assert_eq!(detail.failure.code, "model.reasoning_consumed_budget");
    assert_eq!(detail.completion_token_budget, 8);
    assert_eq!(detail.reasoning_tokens, Some(9));
    assert!(!detail.failure.retryable);
    assert!(!detail.failure.replan_required);

    assert_eq!(
        serde_json::to_value(&detail).unwrap(),
        serde_json::json!({
            "task": 1,
            "failure": {
                "kind": "no_answer",
                "code": "model.reasoning_consumed_budget",
                "retryable": false,
                "replan_required": false,
            },
            "completion_token_budget": 8,
            "reasoning_tokens": 9,
        }),
        "the fixture must be exactly what the constructor emits"
    );

    // Encoding it from the probe numbers reproduces the fixture end to end.
    let rebuilt = TaskOutcome::NoAnswer(AnswerNotProduced::budget_consumed_by_reasoning(
        TaskId(1),
        8,
        9,
    ));
    let line =
        frame::encode(&Frame::notification(Method::FinishTask, &rebuilt.to_params().unwrap()))
            .unwrap();
    assert_eq!(line, fixture(NO_ANSWER_FIXTURE));
}

/// A reasoning model's chain of thought has nowhere to go. A field for it would let
/// hidden text be quoted back as evidence from the page, so the shapes are asserted
/// closed at every depth — and a peer that stuffs it into the JSON is dropped.
#[test]
fn reasoning_text_has_nowhere_to_be_parked() {
    const FORBIDDEN: [&str; 5] =
        ["reasoning_content", "content", "chain_of_thought", "thinking", "reasoning"];
    // `reasoning_tokens` is the one allowed neighbour: a count, not text.
    let leaked = "We need to respond to user: \\\"";

    // The answered arm: injection at three depths, all dropped on the floor.
    let mut answered: serde_json::Value = serde_json::from_str(fixture(FINISH_FIXTURE)).unwrap();
    answered["params"][FORBIDDEN[0]] = serde_json::json!(leaked);
    answered["params"]["citations"][0][FORBIDDEN[3]] = serde_json::json!(leaked);
    answered["params"]["citations"][0]["anchor"]["thinking"] = serde_json::json!(leaked);
    let TaskOutcome::Answered(answer) =
        serde_json::from_value::<TaskOutcome>(answered["params"].clone()).unwrap()
    else {
        panic!("still an answered outcome")
    };
    let out = serde_json::to_value(&answer).unwrap();
    for name in FORBIDDEN {
        assert!(!all_keys(&out).iter().any(|key| key == name), "{name} survived a round trip");
    }
    assert!(!out.to_string().contains(leaked), "injected text must not be stored");

    // The no-answer arm, same assertion, plus the exact field set.
    let detail = AnswerNotProduced::budget_consumed_by_reasoning(TaskId(1), 8, 9);
    let built = serde_json::to_value(&detail).unwrap();
    for name in FORBIDDEN {
        assert!(!all_keys(&built).iter().any(|key| key == name), "{name} in the no-answer arm");
    }
    let mut fields: Vec<&str> = built.as_object().unwrap().keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(fields, vec!["completion_token_budget", "failure", "reasoning_tokens", "task"]);
    let mut failure_fields: Vec<&str> =
        built["failure"].as_object().unwrap().keys().map(String::as_str).collect();
    failure_fields.sort_unstable();
    assert_eq!(failure_fields, vec!["code", "kind", "replan_required", "retryable"]);

    // `detail` is a real field, and it is the producer's own developer-facing text. A
    // provider's bytes must never be routed into it — which is why neither constructor
    // above takes one.
    let mut with_detail: serde_json::Value = serde_json::to_value(&detail).unwrap();
    with_detail["failure"]["detail"] = serde_json::json!(leaked);
    let reread: AnswerNotProduced = serde_json::from_value(with_detail).unwrap();
    assert_eq!(reread.failure.detail.as_deref(), Some(leaked));
}

/// `reasoning_tokens` is `Option` because a server that omits usage must not let the UI
/// claim where the tokens went.
#[test]
fn a_server_that_reports_no_usage_does_not_earn_an_explanation() {
    let unexplained = AnswerNotProduced::unexplained(TaskId(1), "model.content_filtered", 512);
    assert_eq!(unexplained.reasoning_tokens, None);
    let json = serde_json::to_value(&unexplained).unwrap();
    assert!(json.get("reasoning_tokens").is_none(), "absent is not zero");
    assert_eq!(json["failure"]["kind"], serde_json::json!("no_answer"));
    assert_eq!(json["failure"]["code"], serde_json::json!("model.content_filtered"));
    assert_eq!(json["completion_token_budget"], serde_json::json!(512));

    // A newer server reporting usage fields this build has never seen does not break
    // the frame, and does not invent a number either.
    let with_extra: TaskOutcome = serde_json::from_value(serde_json::json!({
        "outcome": "no_answer",
        "task": 1,
        "failure": {
            "kind": "no_answer",
            "code": "model.content_filtered",
            "retryable": false,
            "replan_required": false
        },
        "completion_token_budget": 512,
        "cached_tokens": 40
    }))
    .unwrap();
    let TaskOutcome::NoAnswer(detail) = with_extra else { panic!("no-answer arm") };
    assert_eq!(detail.reasoning_tokens, None);
}

/// A no-answer is its own taxonomy entry: not a truncation of an answer that exists, and
/// not a provider that could not be reached.
#[test]
fn no_answer_is_neither_output_limited_nor_unavailable() {
    let no_answer = Failure::no_answer("model.reasoning_consumed_budget");
    assert_eq!(no_answer.kind.as_str(), "no_answer");
    assert!(no_answer.kind.is_defined(), "it is a defined kind, not the sink");
    assert_ne!(no_answer.kind, FailureKind::OutputLimited);
    assert_ne!(no_answer.kind, FailureKind::Unavailable);
    assert_ne!(no_answer.kind, FailureKind::Cancelled);
    assert_ne!(no_answer.kind, FailureKind::TimedOut);

    // A consumer from before this variant existed sinks it rather than failing the
    // frame, which is what made adding it a minor change instead of a break.
    assert_eq!(
        serde_json::from_value::<FailureKind>(serde_json::json!("no_answer")).unwrap(),
        FailureKind::NoAnswer
    );
    let answer = AnswerWithCitations {
        task: TaskId(1),
        answer: String::new(),
        citations: vec![],
        limit: None,
    };
    let f = scenario();
    assert_eq!(answer.verify(&f.registry, &f.scope).unwrap_err().kind, FailureKind::NoAnswer);
}

// --- The wire shape of an answer. ---

#[test]
fn an_answer_frame_is_one_tagged_object_with_the_citations_inside_it() {
    let f = scenario();
    let json =
        serde_json::to_value(TaskOutcome::Answered(answer_with(vec![live_citation(&f)]))).unwrap();
    assert_eq!(json["outcome"], serde_json::json!("answered"));
    // Externally the tag selects the arm; there is no nested `answered` object.
    assert!(json.get("answered").is_none());

    let mut fields: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(fields, vec!["answer", "citations", "outcome", "task"]);

    let citation_json = &json["citations"][0];
    let mut citation_fields: Vec<&str> =
        citation_json.as_object().unwrap().keys().map(String::as_str).collect();
    citation_fields.sort_unstable();
    assert_eq!(citation_fields, vec!["anchor", "navigation_sequence", "role"]);
    // The anchor is the shape an inline comment already uses, so the overlay and the
    // reattachment ladder need no citation-specific path.
    let mut anchor_fields: Vec<&str> =
        citation_json["anchor"].as_object().unwrap().keys().map(String::as_str).collect();
    anchor_fields.sort_unstable();
    assert_eq!(
        anchor_fields,
        vec!["block_id", "document", "prefix", "quote", "snapshot_version", "suffix"]
    );
}

/// The tag must name an arm and the arm's own fields must be there. An unknown tag is a
/// hard error rather than the `Unknown` sink `FailureKind` uses, because there is no
/// safe default arm: assuming "answered" would render a blank card.
#[test]
fn an_answer_frame_must_name_a_real_arm_and_fill_it_in() {
    for missing in [
        serde_json::json!({}),
        serde_json::json!({"outcome": "answered"}),
        serde_json::json!({"outcome": "answered", "task": 1, "citations": []}),
        serde_json::json!({"outcome": "answered", "task": 1, "answer": "x"}),
        serde_json::json!({"outcome": "no_answer"}),
        serde_json::json!({"task": 1, "answer": "x", "citations": []}),
        serde_json::json!({"outcome": "cancelled"}),
    ] {
        assert!(
            serde_json::from_value::<TaskOutcome>(missing.clone()).is_err(),
            "{missing} must not deserialize into an outcome"
        );
    }

    // What *is* true, and worth stating: serde ignores unrecognised fields, so a peer
    // that adds `failure` to an answered arm is dropped, not merged. The tag still wins.
    let both = serde_json::json!({
        "outcome": "answered",
        "task": 1,
        "answer": "x",
        "citations": [],
        "failure": {"kind": "no_answer", "code": "x", "retryable": false, "replan_required": false}
    });
    let parsed: TaskOutcome = serde_json::from_value(both).unwrap();
    let TaskOutcome::Answered(answer) = parsed else { panic!("the tag selects the arm") };
    assert_eq!(answer.limit, None);
    assert!(!serde_json::to_value(&answer).unwrap().to_string().contains("no_answer"));
}

/// The roles are closed: the overlay renders each one differently, so a role it has
/// never seen fails the frame instead of being labelled with a nearby role.
#[test]
fn an_unknown_citation_role_fails_rather_than_becoming_a_nearby_role() {
    let roles = [
        (CitationRole::DirectAnswer, "direct_answer"),
        (CitationRole::Supporting, "supporting"),
        (CitationRole::Counterpoint, "counterpoint"),
        (CitationRole::Context, "context"),
    ];
    for (role, name) in roles {
        assert_eq!(serde_json::to_value(role).unwrap(), serde_json::json!(name));
        assert_eq!(serde_json::from_value::<CitationRole>(serde_json::json!(name)).unwrap(), role);
    }
    for invented in ["hunch", "Direct_Answer", "supporting_evidence", ""] {
        assert!(
            serde_json::from_value::<CitationRole>(serde_json::json!(invented)).is_err(),
            "{invented:?} must not resolve to a role the overlay would render"
        );
    }
    // A counterpoint must be expressible at all: an answer that cites only agreement
    // is not evidencing anything, and this is the only variant that makes disagreement
    // visible in the UI.
    assert!(serde_json::to_value(CitationRole::Counterpoint).unwrap().is_string());
}

#[test]
fn every_id_in_an_inbound_answer_is_an_assertion_the_shell_reruns() {
    // The seam engineer 2 codes against: nothing in an inbound answer is authority, so
    // re-running verification against a different registry changes the verdict rather
    // than being ignored.
    let f = scenario();
    let answer = answer_with(vec![live_citation(&f)]);
    assert_eq!(answer.verify(&f.registry, &f.scope).unwrap().len(), 1);

    let empty_registry = DocumentIdentityRegistry::new();
    let error = answer.verify(&empty_registry, &f.scope).unwrap_err();
    assert_eq!(error.kind, FailureKind::Stale);
    assert_eq!(error.code, "identity.task_unknown");

    let view = TaskRequestView {
        cancel: CancellationToken { task: f.task, deadline_ms_since_epoch: 0 },
        ..TaskRequestView::of(&request(f.task, f.live, ActionLevel::Navigate))
    };
    let decoded: TaskRequestView = serde_json::from_value(view.to_params().unwrap()).unwrap();
    assert_eq!(decoded, view);
    assert_eq!(
        empty_registry.resolve(decoded.document.document_token, &f.scope).unwrap_err().code,
        "identity.token_unknown"
    );
    // A scope rebuilt from frame-readable numbers is not the brokered scope.
    assert_eq!(
        f.registry
            .resolve(
                decoded.document.document_token,
                &RequestScope { profile_id: ProfileId(1), workspace_id: WorkspaceId(4_242) }
            )
            .unwrap_err()
            .code,
        "identity.cross_scope"
    );
    // The claim's tab id survives the round trip, and it is still just a number: only
    // `resolve_in_tab` turns it into "the document you are about to act on".
    assert_eq!(decoded.document.tab_id, TabId(f.tab.0));
}