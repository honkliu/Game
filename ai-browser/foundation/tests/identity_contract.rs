//! FND-01 acceptance: identity substitution and URL-as-identity.
//!
//! Every test asserts on concrete values, and the load-bearing one is
//! `same_url_reload_yields_a_different_document_key`: a reload changes nothing but
//! `navigationSequence` and the token, so if identity were derived from the URL the
//! two keys would compare equal and a stale plan would be actionable.

#![allow(clippy::unwrap_used)]

use ai_platform_contract::contract::ActionRequest;
use ai_platform_contract::identity::{
    DocumentIdentityRegistry, DocumentToken, ProfileId, RequestScope, SnapshotVersion, StaleStatus,
    TabId, UpstreamNavigationId, WorkspaceId,
};

const URL: &str = "https://example.com/pricing";

struct Fixture {
    registry: DocumentIdentityRegistry,
    profile: ProfileId,
    workspace: WorkspaceId,
    tab: TabId,
}

fn fixture() -> Fixture {
    let mut registry = DocumentIdentityRegistry::new();
    let profile = registry.create_profile();
    let workspace = registry.create_workspace(profile).unwrap();
    let tab = registry.open_tab(workspace).unwrap();
    Fixture { registry, profile, workspace, tab }
}

fn scope(fixture: &Fixture) -> RequestScope {
    RequestScope { profile_id: fixture.profile, workspace_id: fixture.workspace }
}

/// The backlog's acceptance criterion, stated as behaviour: a same-URL reload
/// produces a different document, and the URL is identical in both.
#[test]
fn same_url_reload_yields_a_different_document_key() {
    let mut f = fixture();
    let before = f
        .registry
        .on_document_committed(f.tab, 7, Some(UpstreamNavigationId(101)), URL)
        .unwrap();
    let after = f
        .registry
        .on_document_committed(f.tab, 7, Some(UpstreamNavigationId(102)), URL)
        .unwrap();

    assert_eq!(before.navigation_sequence() + 1, after.navigation_sequence());
    assert_ne!(before, after, "a reload must not look like the same document");
    assert_ne!(before.document_token(), after.document_token());
    assert_eq!(f.registry.url(before.document_token()).unwrap(), URL);
    assert_eq!(f.registry.url(after.document_token()).unwrap(), URL);

    // The old identity is not merely different, it is unusable: the reload retires it
    // in the same step that mints the new one.
    let stale = f.registry.resolve(before.document_token(), &scope(&f)).unwrap_err();
    assert_eq!(stale.kind.as_str(), "stale");
    assert_eq!(stale.code, "identity.token_retired");
    assert!(stale.replan_required, "a retired document cannot be acted on without replanning");
    assert!(!stale.retryable);
}

/// Structural form of "a URL is metadata, not identity": the key has no field a URL
/// could occupy.
#[test]
fn document_key_has_no_field_a_url_could_live_in() {
    let mut f = fixture();
    let key = f
        .registry
        .on_document_committed(f.tab, 7, Some(UpstreamNavigationId(101)), URL)
        .unwrap();
    let json = serde_json::to_value(&key).unwrap();
    // `serde_json::Value` orders object keys (BTreeMap), but assert the set explicitly
    // so the test is about which fields exist, not about map iteration order.
    let mut fields: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "document_token",
            "frame_tree_node_id",
            "navigation_sequence",
            "profile_id",
            "tab_id",
            "upstream_navigation_id",
            "workspace_id",
        ]
    );
    for urlish in ["url", "uri", "href", "canonical_url", "origin", "title"] {
        assert!(!fields.contains(&urlish), "{urlish} must not be an identity field");
    }
}

/// Each identity field is load-bearing: documents that agree on URL and differ in
/// exactly one of tab, workspace, or navigation sequence are distinct keys.
#[test]
fn each_identity_field_alone_distinguishes_two_documents() {
    let mut f = fixture();
    let first = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();

    // Same tab, same URL, one navigation later.
    let reloaded = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();
    assert_eq!(first.tab_id(), reloaded.tab_id());
    assert_ne!(first, reloaded, "navigation_sequence alone must distinguish");

    // Same workspace, same URL, first navigation in a different tab.
    let other_tab = f.registry.open_tab(f.workspace).unwrap();
    let elsewhere = f.registry.on_document_committed(other_tab, 8, None, URL).unwrap();
    assert_eq!(elsewhere.navigation_sequence(), first.navigation_sequence());
    assert_eq!(elsewhere.workspace_id(), first.workspace_id());
    assert_ne!(elsewhere.tab_id(), first.tab_id());
    assert_ne!(elsewhere, first, "tab_id alone must distinguish");

    // Same URL, different workspace.
    let other_workspace = f.registry.create_workspace(f.profile).unwrap();
    let other_scope_tab = f.registry.open_tab(other_workspace).unwrap();
    let foreign = f.registry.on_document_committed(other_scope_tab, 9, None, URL).unwrap();
    assert_ne!(foreign.workspace_id(), first.workspace_id());
    assert_ne!(foreign, first, "workspace_id alone must distinguish");

    // The frame tree node is carried through from the webview unchanged.
    assert_eq!(elsewhere.frame_tree_node_id(), 8);
    assert_eq!(foreign.frame_tree_node_id(), 9);
}

/// Identity substitution across a workspace boundary is refused as `blocked`, not
/// reported as merely stale — otherwise a peer could probe neighbouring workspaces
/// by the difference in reply.
#[test]
fn token_from_another_workspace_is_blocked() {
    let mut f = fixture();
    let foreign_workspace = f.registry.create_workspace(f.profile).unwrap();
    let foreign_tab = f.registry.open_tab(foreign_workspace).unwrap();
    let foreign_key = f.registry.on_document_committed(foreign_tab, 3, None, URL).unwrap();

    let error = f
        .registry
        .resolve(foreign_key.document_token(), &scope(&f))
        .unwrap_err();
    assert_eq!(error.kind, ai_platform_contract::failure::FailureKind::Blocked);
    assert_eq!(error.code, "identity.cross_scope");
    assert!(!error.replan_required, "replanning inside this workspace cannot reach it");

    // A TaskId is scoped the same way.
    let foreign_task = f.registry.create_task(foreign_workspace).unwrap();
    let task_error = f.registry.resolve_task(foreign_task, &scope(&f)).unwrap_err();
    assert_eq!(task_error.code, "identity.task_cross_workspace");
    assert_eq!(task_error.kind, ai_platform_contract::failure::FailureKind::Blocked);
}

/// Naming somebody else's tab is a substitution attempt (`blocked`), which must not
/// be reported with the same code as an ordinary navigation (`stale`).
#[test]
fn token_from_another_tab_is_blocked_not_stale() {
    let mut f = fixture();
    let second_tab = f.registry.open_tab(f.workspace).unwrap();
    let in_second = f.registry.on_document_committed(second_tab, 4, None, URL).unwrap();

    // Both tabs are live and in scope; the token is valid, the tab claim is not.
    let error = f
        .registry
        .resolve_in_tab(in_second.document_token(), &scope(&f), f.tab)
        .unwrap_err();
    assert_eq!(error.code, "identity.cross_tab");
    assert_eq!(error.kind, ai_platform_contract::failure::FailureKind::Blocked);
}

/// The dispatch path refuses a document that navigated while the model was thinking.
#[test]
fn action_path_refuses_a_document_that_navigated_mid_plan() {
    let mut f = fixture();
    let planned_against = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();
    let version = f.registry.next_snapshot_version(planned_against.document_token(), &scope(&f)).unwrap();
    assert_eq!(version, SnapshotVersion(1));

    // The user clicks a link while the model is generating.
    let now_showing = f.registry.on_document_committed(f.tab, 7, None, "https://other.example/").unwrap();

    let error = f
        .registry
        .resolve_in_tab(planned_against.document_token(), &scope(&f), f.tab)
        .unwrap_err();
    assert_eq!(error.code, "identity.token_retired");
    assert_eq!(error.kind, ai_platform_contract::failure::FailureKind::Stale);
    assert!(error.replan_required);

    // The current document still dispatches.
    let resolved = f.registry.resolve_in_tab(now_showing.document_token(), &scope(&f), f.tab).unwrap();
    assert_eq!(resolved, now_showing);
}

/// A client-side route change is a new document even though nothing was fetched,
/// because element handles from the old route may point at nodes that no longer
/// exist (backlog §4.3).
#[test]
fn history_state_change_invalidates_the_previous_document() {
    let mut f = fixture();
    let route_a = f.registry.on_document_committed(f.tab, 7, Some(UpstreamNavigationId(50)), URL).unwrap();
    // pushState carries no WebView2 NavigationId at all.
    let route_b = f
        .registry
        .on_history_state_changed(f.tab, "https://example.com/pricing#annual")
        .unwrap();

    assert_eq!(route_b.upstream_navigation_id(), None);
    assert_eq!(route_b.frame_tree_node_id(), route_a.frame_tree_node_id());
    assert_eq!(route_b.navigation_sequence(), route_a.navigation_sequence() + 1);
    assert_ne!(route_b.document_token(), route_a.document_token());
    assert!(f
        .registry
        .resolve_in_tab(route_a.document_token(), &scope(&f), f.tab)
        .is_err());
}

/// WebView2 hands us a per-navigation `NavigationId`, but its scope and reuse rules
/// are unmeasured, so nothing may depend on it being unique. Two documents sharing
/// one upstream id must still be distinct.
#[test]
fn upstream_navigation_id_uniqueness_is_not_assumed() {
    let mut f = fixture();
    let first = f.registry.on_document_committed(f.tab, 7, Some(UpstreamNavigationId(42)), URL).unwrap();
    let second = f.registry.on_document_committed(f.tab, 7, Some(UpstreamNavigationId(42)), URL).unwrap();

    assert_eq!(first.upstream_navigation_id(), Some(UpstreamNavigationId(42)));
    assert_eq!(second.upstream_navigation_id(), Some(UpstreamNavigationId(42)));
    assert_ne!(first, second, "identity must not rest on an unverified upstream counter");

    let absent = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();
    assert_eq!(absent.upstream_navigation_id(), None);
}

#[test]
fn snapshot_version_zero_is_reserved_and_numbering_is_monotonic() {
    let mut f = fixture();
    let key = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();

    // Nothing captured yet is `unavailable`, not version 0 and not an error the
    // caller has to special-case as stale.
    let before = f.registry.current_snapshot_version(key.document_token(), &scope(&f)).unwrap_err();
    assert_eq!(before.code, "document.not_captured");
    assert_eq!(before.kind, ai_platform_contract::failure::FailureKind::Unavailable);
    assert!(!SnapshotVersion(0).is_captured());

    assert_eq!(f.registry.next_snapshot_version(key.document_token(), &scope(&f)).unwrap(), SnapshotVersion(1));
    assert_eq!(f.registry.next_snapshot_version(key.document_token(), &scope(&f)).unwrap(), SnapshotVersion(2));
    assert_eq!(f.registry.current_snapshot_version(key.document_token(), &scope(&f)).unwrap(), SnapshotVersion(2));

    // A new document restarts numbering at 1, which is safe only because tokens are
    // never recycled: (token, version) stays unique.
    let replacement = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();
    let unfetched = f.registry.current_snapshot_version(replacement.document_token(), &scope(&f));
    assert_eq!(unfetched.unwrap_err().code, "document.not_captured");
    assert_eq!(f.registry.next_snapshot_version(replacement.document_token(), &scope(&f)).unwrap(), SnapshotVersion(1));
    assert_ne!(replacement.document_token(), key.document_token());
}

#[test]
fn staleness_reports_every_way_a_document_can_stop_being_current() {
    let mut f = fixture();
    let key = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();
    let version = f.registry.next_snapshot_version(key.document_token(), &scope(&f)).unwrap();
    assert_eq!(f.registry.assess_staleness(&key, version), StaleStatus::Current);

    // Infinite scroll appends content: same document, newer version.
    let later = f.registry.next_snapshot_version(key.document_token(), &scope(&f)).unwrap();
    assert_eq!(f.registry.assess_staleness(&key, version), StaleStatus::ContentChanged);
    assert_eq!(f.registry.assess_staleness(&key, later), StaleStatus::Current);

    // Navigating replaces the document.
    f.registry.on_document_committed(f.tab, 7, None, "https://example.com/other").unwrap();
    assert_eq!(f.registry.assess_staleness(&key, later), StaleStatus::DocumentReplaced);

    // An answer recorded against "no snapshot" is never labelled current.
    assert_eq!(f.registry.assess_staleness(&key, SnapshotVersion(0)), StaleStatus::DocumentReplaced);

    // Closing the tab is reported differently from navigating, because the UI offers
    // a different remedy (reopen vs. re-read).
    let closed_tab = f.registry.open_tab(f.workspace).unwrap();
    let closed_key = f.registry.on_document_committed(closed_tab, 9, None, URL).unwrap();
    let closed_version = f.registry.next_snapshot_version(closed_key.document_token(), &scope(&f)).unwrap();
    f.registry.on_tab_closed(closed_tab);
    assert_eq!(f.registry.assess_staleness(&closed_key, closed_version), StaleStatus::TabClosed);

    // Serialised as the snake_case names the UI switches on.
    assert_eq!(serde_json::to_value(StaleStatus::DocumentReplaced).unwrap(), serde_json::json!("document_replaced"));
}

#[test]
fn closing_a_workspace_retires_its_documents_and_tasks() {
    let mut f = fixture();
    let second_tab = f.registry.open_tab(f.workspace).unwrap();
    f.registry.on_document_committed(f.tab, 1, None, URL).unwrap();
    f.registry.on_document_committed(second_tab, 2, None, URL).unwrap();
    let task = f.registry.create_task(f.workspace).unwrap();
    assert_eq!(f.registry.live_document_count(f.workspace), 2);

    assert_eq!(f.registry.on_workspace_closed(f.workspace), 2);
    assert_eq!(f.registry.live_document_count(f.workspace), 0, "clearing workspace memory must leave no live document");
    assert_eq!(f.registry.resolve_task(task, &scope(&f)).unwrap_err().code, "identity.task_unknown");
    // The workspace itself is gone, so nothing can be opened into it afterwards.
    assert_eq!(f.registry.open_tab(f.workspace).unwrap_err().code, "identity.workspace_unknown");
}

#[test]
fn closing_one_tab_leaves_its_workspace_siblings_alone() {
    let mut f = fixture();
    let sibling = f.registry.open_tab(f.workspace).unwrap();
    let in_tab = f.registry.on_document_committed(f.tab, 1, None, URL).unwrap();
    let in_sibling = f.registry.on_document_committed(sibling, 2, None, URL).unwrap();

    assert_eq!(f.registry.on_tab_closed(f.tab), 1);
    assert_eq!(f.registry.assess_staleness(&in_tab, SnapshotVersion(1)), StaleStatus::TabClosed);
    assert!(f.registry.resolve(in_sibling.document_token(), &scope(&f)).is_ok());
    assert_eq!(f.registry.live_document_count(f.workspace), 1);
}

/// Context bound to a workspace must not follow a tab dragged out of it.
#[test]
fn moving_a_tab_between_workspaces_invalidates_its_context() {
    let mut f = fixture();
    let destination = f.registry.create_workspace(f.profile).unwrap();
    let key = f.registry.on_document_committed(f.tab, 1, None, URL).unwrap();

    assert_eq!(f.registry.move_tab_to_workspace(f.tab, destination).unwrap(), 1);
    assert_eq!(f.registry.live_document_count(f.workspace), 0);

    // Two views of the same retired document, and the codes differ for a reason. The
    // document's own scope says "retired"; the destination's scope says "not yours",
    // because scope is checked before liveness so a neighbouring workspace cannot use
    // the retired/live difference to learn which tokens existed.
    let original_scope = RequestScope { profile_id: f.profile, workspace_id: f.workspace };
    let retired = f.registry.resolve(key.document_token(), &original_scope).unwrap_err();
    assert_eq!(retired.code, "identity.token_retired");
    assert!(retired.replan_required);

    let moved_scope = RequestScope { profile_id: f.profile, workspace_id: destination };
    let not_yours = f.registry.resolve(key.document_token(), &moved_scope).unwrap_err();
    assert_eq!(not_yours.code, "identity.cross_scope");
    assert!(!not_yours.replan_required);
    assert_eq!(
        f.registry.resolve_in_tab(key.document_token(), &moved_scope, f.tab).unwrap_err().code,
        "identity.cross_scope",
        "the moved document is not silently re-scoped to the destination"
    );
    // The tab is usable in its new workspace, with a fresh document.
    let fresh = f.registry.on_document_committed(f.tab, 1, None, URL).unwrap();
    assert_eq!(fresh.workspace_id(), destination);
    assert!(f.registry.resolve_in_tab(fresh.document_token(), &moved_scope, f.tab).is_ok());
}

#[test]
fn a_fabricated_workspace_name_is_refused_not_accepted() {
    let mut f = fixture();
    // An id the registry never minted. Nothing trusts it for having been named.
    assert_eq!(f.registry.open_tab(WorkspaceId(9_999)).unwrap_err().code, "identity.workspace_unknown");
    assert_eq!(
        f.registry.create_workspace(ProfileId(9_999)).unwrap_err().code,
        "identity.profile_unknown"
    );
    assert_eq!(f.registry.create_task(WorkspaceId(0)).unwrap_err().code, "identity.workspace_unknown");
    assert_eq!(f.registry.resolve(DocumentToken(0), &scope(&f)).unwrap_err().code, "identity.token_unknown");
    assert_eq!(TabId(0).0, 0, "zero remains the unset sentinel, never a minted id");
}

/// The typed half of "identity cannot be injected": an inbound params object can name
/// a document token and nothing else. A `document`/`executed_on` object smuggled in
/// beside it has nowhere to land, so it is dropped rather than becoming authority.
/// The compile-time half is the `compile_fail` example on `DocumentKey`.
#[test]
fn inbound_frames_cannot_carry_a_document_key() {
    let mut f = fixture();
    let key = f.registry.on_document_committed(f.tab, 7, None, URL).unwrap();

    let mut params = serde_json::json!({
        "action_id": "a1",
        "task": 1,
        "expected_document": key.document_token(),
        "expected_snapshot_version": 1,
        "action_kind": "scroll_to",
        "deadline_ms_since_epoch": 1_700_000_000_000i64,
        "idempotency": "safe",
    });
    // What a hostile peer would like to assert about itself.
    params["executed_on"] = serde_json::to_value(&key).unwrap();
    params["workspace_id"] = serde_json::json!(key.workspace_id());

    let request: ActionRequest = serde_json::from_value(params).unwrap();
    assert_eq!(request.expected_document, key.document_token());
    assert_eq!(
        serde_json::to_value(&request).unwrap().as_object().unwrap().contains_key("executed_on"),
        false,
        "ActionRequest must have no field that could hold a claimed identity"
    );

    // And the token it did supply is only ever an assertion, checked against the
    // scope the frame arrived on.
    let foreign_scope = RequestScope { profile_id: f.profile, workspace_id: WorkspaceId(4242) };
    assert_eq!(f.registry.resolve(key.document_token(), &foreign_scope).unwrap_err().code, "identity.cross_scope");
}
