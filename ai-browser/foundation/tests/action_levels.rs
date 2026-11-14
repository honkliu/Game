//! Contract shape decisions fixed on 2026-11-13 after the WebView2 capability audit.
//! Each test exists to stop a specific regression, not to describe the type.

#![allow(clippy::unwrap_used)]

use ai_platform_contract::contract::{
    level_of_action_kind, ActionLevel, AnchorReattachmentTier, AnnotationAnchor, TargetResolution,
};
use ai_platform_contract::identity::{DocumentToken, SnapshotVersion};

/// Scroll-to-passage is Navigate-class read-only, so the Release-0 dispatch gate lets
/// it through while everything that mutates a page stays out.
#[test]
fn host_driven_scroll_is_navigate_class_and_passes_the_release_zero_gate() {
    assert_eq!(level_of_action_kind("scroll_to"), Some(ActionLevel::Navigate));
    assert!(ActionLevel::Navigate.allowed_in_release_zero());
    assert!(ActionLevel::Observe.allowed_in_release_zero());
    assert!(!ActionLevel::Interact.allowed_in_release_zero());
    assert!(!ActionLevel::Commit.allowed_in_release_zero());

    // The kinds that would have violated backlog §9 stay gated.
    for mutating in ["type_text", "set_filter", "submit_form", "purchase"] {
        let level = level_of_action_kind(mutating).unwrap();
        assert!(
            !level.allowed_in_release_zero(),
            "{mutating} classified at {level:?}, which Release 0 must not dispatch"
        );
    }
}

/// An action kind nobody classified must fail closed. Grading it `Observe` because it
/// is unknown is how a page mutation reaches the DOM in a release that promises none.
#[test]
fn an_unclassified_action_kind_resolves_to_no_level() {
    assert_eq!(level_of_action_kind("harvest_cookies"), None);
    assert_eq!(level_of_action_kind(""), None);
    assert_eq!(level_of_action_kind("Scroll_To"), None, "kind names are case-sensitive");
}

/// The accessibility half of backlog §7.2 tier 3 does not exist for a WebView2
/// embedder, so no variant may offer it — not now, and not "reserved for later".
#[test]
fn no_target_resolution_tier_promises_the_accessibility_tree() {
    let tiers = [
        TargetResolution::BrowserSemanticCommand,
        TargetResolution::PageStructuredTool,
        TargetResolution::DomLocator,
        TargetResolution::VisualTarget,
        TargetResolution::CoordinateFallback,
    ];
    let names: Vec<String> = tiers
        .iter()
        .map(|tier| serde_json::to_value(tier).unwrap().as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "browser_semantic_command",
            "page_structured_tool",
            "dom_locator",
            "visual_target",
            "coordinate_fallback",
        ]
    );
    for name in &names {
        assert!(!name.contains("accessib"), "{name} would promise an unavailable API");
    }
}

/// The reattachment ladder, in order, with no accessibility-path step and `Detached`
/// last and terminal.
#[test]
fn the_anchor_reattachment_ladder_has_no_unresolvable_step() {
    let ladder = [
        AnchorReattachmentTier::SemanticBlockId,
        AnchorReattachmentTier::QuoteWithAffixes,
        AnchorReattachmentTier::NormalizedTextPosition,
        AnchorReattachmentTier::FuzzyQuoteMatch,
        AnchorReattachmentTier::Detached,
    ];
    // Ordering is the contract: a caller walks it with `<`, so the declaration order
    // must be the trust order.
    for pair in ladder.windows(2) {
        assert!(pair[0] < pair[1], "{:?} must outrank {:?}", pair[0], pair[1]);
    }
    let names: Vec<String> = ladder
        .iter()
        .map(|tier| serde_json::to_value(tier).unwrap().as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "semantic_block_id",
            "quote_with_affixes",
            "normalized_text_position",
            "fuzzy_quote_match",
            "detached",
        ]
    );
    for name in &names {
        assert!(!name.contains("accessib"), "{name} would be a tier that can never resolve");
    }
}

/// An anchor must be able to carry every field the ladder needs, and nothing else — in
/// particular no accessibility path.
#[test]
fn an_anchor_carries_exactly_the_fields_the_ladder_reads() {
    let anchor = AnnotationAnchor {
        document: DocumentToken(11),
        snapshot_version: SnapshotVersion(3),
        block_id: Some("b17".to_string()),
        quote: "30-day returns".to_string(),
        prefix: Some("Shipping says ".to_string()),
        suffix: Some(" on all orders.".to_string()),
        text_position: Some(1_024),
    };
    let json = serde_json::to_value(&anchor).unwrap();
    let mut fields: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec!["block_id", "document", "prefix", "quote", "snapshot_version", "suffix", "text_position"]
    );

    // Every resolution tier below `SemanticBlockId` must survive a snapshot that has
    // no block ids at all: the optional fields are how tiers 2-4 resolve.
    let minimal = AnnotationAnchor {
        document: DocumentToken(11),
        snapshot_version: SnapshotVersion(3),
        block_id: None,
        quote: "30-day returns".to_string(),
        prefix: None,
        suffix: None,
        text_position: None,
    };
    let json = serde_json::to_value(&minimal).unwrap();
    assert_eq!(
        json.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["document", "quote", "snapshot_version"]
    );
    let round_tripped: AnnotationAnchor = serde_json::from_value(json).unwrap();
    assert_eq!(round_tripped, minimal);
}
