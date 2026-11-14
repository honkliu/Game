//! Document identity and the ownership rules around it (backlog FND-01).
//!
//! The browser shell process owns these answers and nothing else may give them:
//!
//! | Question | Owner |
//! |---|---|
//! | Who mints identity? | [`DocumentIdentityRegistry`], in the shell process only. The AI utility process never holds a registry. |
//! | What crosses the boundary? | An opaque [`DocumentToken`] plus ids the receiver re-validates. Never a profile, workspace, or navigation sequence asserted by the sender. |
//! | What is metadata? | URL, origin, title. Stored *beside* a document, never inside its key, so a URL cannot renumber identity and identity cannot be inferred from a URL. |
//! | What invalidates what? | Tab close retires its documents. Workspace change retires the moved tab's documents. Each new document commit takes a fresh token (never recycled) and a navigation sequence that only increases within a tab. |
//! | Who ends identity? | Only the shell, from WebView2 navigation and tab lifecycle events — never from page script reporting its own URL. |
//!
//! ## The WebView2 anchor
//!
//! The bound SDK (`webview2-com-sys-0.39.1`) exposes a browser-assigned
//! `NavigationId() -> u64` on four event argument interfaces:
//! `ICoreWebView2NavigationStartingEventArgs`, `ICoreWebView2NavigationCompletedEventArgs`,
//! `ICoreWebView2ContentLoadingEventArgs`, `ICoreWebView2DOMContentLoadedEventArgs`.
//! It is recorded on the key as [`DocumentKey::upstream_navigation_id`] so an
//! audit can tie our identity to what WebView2 actually reported.
//!
//! It is *not* the authority, for three verifiable reasons:
//! 1. It is absent from the same-document surfaces. `SourceChanged` and history
//!    navigations carry no `NavigationId`, and a `history.pushState` route change
//!    never reaches a navigation event at all — it is visible only from injected
//!    script. Those changes still need a new sequence (backlog §4.3).
//! 2. Whether the counter is per-webview or per-browser, and whether a
//!    back/forward entry reuses an id, is not stated by the bindings. The symbol
//!    proves the shape, not the behaviour. Until it is measured on a live webview
//!    we must not depend on uniqueness or monotonicity.
//! 3. It is per-webview, while our key also spans profile and workspace, which
//!    WebView2 has no concept of.
//!
//! So `navigation_sequence` is our own per-tab counter and drives every
//! decision; the upstream id is recorded beside it. See
//! `tools/README.md` for the live-webview experiment that has to run before the
//! upstream id is promoted to anything load-bearing.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::failure::Failure;

/// A browser profile. Two profiles never share a workspace, a document token, or
/// a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub u64);

/// A workspace: the set of tabs whose contents the assistant may read together.
/// The unit of context isolation, and the scope every request is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub u64);

/// A tab as minted here. Deliberately not a tab index (the user changes that by
/// dragging) and not "the active tab" (a lookup that goes stale between planning
/// and dispatch, backlog §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TabId(pub u64);

/// Opaque handle for one committed document.
///
/// Forgeable on purpose, and worthless to forge: it is only ever an index into
/// the shell's registry, so a made-up value resolves to nothing. The authority is
/// [`DocumentKey`], which is deliberately not constructible from a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocumentToken(pub u64);

/// A unit of assistant work, scoped to the workspace that created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(pub u64);

/// Capture generation of one document. `NONE` (0) means "nothing captured yet"
/// and is never a legal argument to an action. Numbering restarts at 1 for a new
/// document, which is safe only because tokens are never recycled — so
/// `(document_token, snapshot_version)` is unique for the life of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SnapshotVersion(pub u64);

impl SnapshotVersion {
    pub const NONE: u64 = 0;

    pub fn is_captured(self) -> bool {
        self.0 != Self::NONE
    }
}

/// A `NavigationId` reported by WebView2, recorded for audit correlation. See the
/// module docs for why this is not the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UpstreamNavigationId(pub u64);

/// Why a document stopped being its tab's current document. Retired records are
/// kept rather than erased: a session holds on the order of one per navigation,
/// and an old conversation turn must still be classifiable, which needs to know
/// how a document died.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetireReason {
    SupersededByNavigation,
    MovedToWorkspace,
    TabClosed,
    WorkspaceClosed,
}

/// Why a stored answer no longer matches what is on screen (backlog §5).
/// Displayed, never swallowed: a stale answer may stay visible but must be
/// labelled, and an action planned from it cannot run without replanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleStatus {
    Current,
    ContentChanged,
    DocumentReplaced,
    TabClosed,
}

/// The complete identity of one document (backlog §2.3). Every field is required
/// to distinguish two documents, so equality is field-wise — and no field is a
/// URL.
///
/// Obtainable only from [`DocumentIdentityRegistry`]. The constructor is private
/// and this type is `Serialize` but not `Deserialize`: a peer that could build one
/// could claim to be any document. The test double for that claim is the
/// `compile_fail` example below, plus
/// `tests/identity_contract.rs::inbound_frames_cannot_carry_a_document_key`.
///
/// ```compile_fail
/// use ai_platform_contract::identity::{DocumentKey, DocumentToken, ProfileId, TabId, WorkspaceId};
/// // Every value here is readable off a frame by any peer. That must not be
/// // enough to produce a key.
/// let forged = DocumentKey {
///     profile_id: ProfileId(1),
///     workspace_id: WorkspaceId(2),
///     tab_id: TabId(3),
///     frame_tree_node_id: 4,
///     document_token: DocumentToken(5),
///     navigation_sequence: 6,
///     upstream_navigation_id: None,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DocumentKey {
    profile_id: ProfileId,
    workspace_id: WorkspaceId,
    tab_id: TabId,
    /// Passed through from the webview's frame tree; this registry does not mint
    /// it. 0 until the shell has one to record.
    frame_tree_node_id: u64,
    document_token: DocumentToken,
    navigation_sequence: u64,
    upstream_navigation_id: Option<UpstreamNavigationId>,
}

impl DocumentKey {
    pub fn profile_id(&self) -> ProfileId {
        self.profile_id
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub fn tab_id(&self) -> TabId {
        self.tab_id
    }

    pub fn frame_tree_node_id(&self) -> u64 {
        self.frame_tree_node_id
    }

    pub fn document_token(&self) -> DocumentToken {
        self.document_token
    }

    pub fn navigation_sequence(&self) -> u64 {
        self.navigation_sequence
    }

    pub fn upstream_navigation_id(&self) -> Option<UpstreamNavigationId> {
        self.upstream_navigation_id
    }
}

/// The scope a frame arrived on, established when the pipe was brokered. Taken
/// from the connection, never from the message — that is what stops a peer from
/// naming someone else's workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestScope {
    pub profile_id: ProfileId,
    pub workspace_id: WorkspaceId,
}

#[derive(Debug)]
struct WorkspaceRecord {
    profile_id: ProfileId,
}

#[derive(Debug)]
struct TabRecord {
    profile_id: ProfileId,
    workspace_id: WorkspaceId,
    /// The document an action against this tab must name. `None` before the first
    /// commit and after the tab's document was retired by a workspace move.
    current_token: Option<DocumentToken>,
    current_frame_tree_node_id: u64,
    next_navigation_sequence: u64,
}

#[derive(Debug)]
struct DocumentRecord {
    profile_id: ProfileId,
    workspace_id: WorkspaceId,
    tab_id: TabId,
    frame_tree_node_id: u64,
    navigation_sequence: u64,
    upstream_navigation_id: Option<UpstreamNavigationId>,
    snapshot_version: SnapshotVersion,
    url: String,
    is_current: bool,
    retire_reason: Option<RetireReason>,
}

#[derive(Debug)]
struct TaskRecord {
    workspace_id: WorkspaceId,
}

/// Browser-process registry of document identity. One instance, in the shell
/// process, on the UI thread — which is where WebView2 raises navigation and tab
/// lifecycle events anyway, so no locking is needed and none is taken.
///
/// `Default` is implemented as `new()` rather than derived: a derived default
/// would start the counters at 0 and hand out ids whose value is the "unset"
/// sentinel, which is the one value every consumer treats as absent.
pub struct DocumentIdentityRegistry {
    // Ids start at 1; 0 is the "unset" value of every id type.
    next_profile_id: u64,
    next_workspace_id: u64,
    next_tab_id: u64,
    next_document_token: u64,
    next_task_id: u64,
    profiles: HashMap<u64, ()>,
    workspaces: HashMap<u64, WorkspaceRecord>,
    tabs: HashMap<u64, TabRecord>,
    documents: HashMap<u64, DocumentRecord>,
    tasks: HashMap<u64, TaskRecord>,
}

impl Default for DocumentIdentityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentIdentityRegistry {
    pub fn new() -> DocumentIdentityRegistry {
        DocumentIdentityRegistry {
            next_profile_id: 1,
            next_workspace_id: 1,
            next_tab_id: 1,
            next_document_token: 1,
            next_task_id: 1,
            profiles: HashMap::new(),
            workspaces: HashMap::new(),
            tabs: HashMap::new(),
            documents: HashMap::new(),
            tasks: HashMap::new(),
        }
    }

    // --- Minting. These are the only origins of an id. ---

    pub fn create_profile(&mut self) -> ProfileId {
        let id = ProfileId(self.next_profile_id);
        self.next_profile_id += 1;
        self.profiles.insert(id.0, ());
        id
    }

    pub fn create_workspace(&mut self, profile_id: ProfileId) -> Result<WorkspaceId, Failure> {
        if !self.profiles.contains_key(&profile_id.0) {
            return Err(Failure::blocked("identity.profile_unknown"));
        }
        let id = WorkspaceId(self.next_workspace_id);
        self.next_workspace_id += 1;
        self.workspaces.insert(
            id.0,
            WorkspaceRecord {
                profile_id,
            },
        );
        Ok(id)
    }

    pub fn open_tab(&mut self, workspace_id: WorkspaceId) -> Result<TabId, Failure> {
        let Some(profile_id) = self.workspaces.get(&workspace_id.0).map(|w| w.profile_id) else {
            return Err(Failure::blocked("identity.workspace_unknown"));
        };
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        // The tab inherits the profile of the workspace that owns it; a caller
        // cannot open a tab into a profile it does not hold.
        self.tabs.insert(
            id.0,
            TabRecord {
                profile_id,
                workspace_id,
                current_token: None,
                current_frame_tree_node_id: 0,
                next_navigation_sequence: 1,
            },
        );
        Ok(id)
    }

    /// A new document committed in `tab` (WebView2 `NavigationStarting` /
    /// `NavigationCompleted`): fresh token, next navigation sequence, previous
    /// document retired.
    pub fn on_document_committed(
        &mut self,
        tab_id: TabId,
        frame_tree_node_id: u64,
        upstream_navigation_id: Option<UpstreamNavigationId>,
        url: &str,
    ) -> Result<DocumentKey, Failure> {
        self.commit_new_document(tab_id, frame_tree_node_id, upstream_navigation_id, url)
    }

    /// A same-document History API route change (backlog §4.3).
    ///
    /// Reusing the previous token here would be the cheap answer and is wrong:
    /// element handles and action targets held from the old client-side route may
    /// point at nodes that no longer exist, so the route change takes a new
    /// sequence and a new token. WebView2 reports no `NavigationId` for this — it
    /// is detected by injected script — hence the `None`.
    pub fn on_history_state_changed(
        &mut self,
        tab_id: TabId,
        url: &str,
    ) -> Result<DocumentKey, Failure> {
        let frame_tree_node_id = match self.tabs.get(&tab_id.0) {
            None => return Err(Failure::blocked("identity.tab_unknown")),
            Some(tab) => tab.current_frame_tree_node_id,
        };
        self.commit_new_document(tab_id, frame_tree_node_id, None, url)
    }

    fn commit_new_document(
        &mut self,
        tab_id: TabId,
        frame_tree_node_id: u64,
        upstream_navigation_id: Option<UpstreamNavigationId>,
        url: &str,
    ) -> Result<DocumentKey, Failure> {
        // Copy the tab's state out before touching `documents`, so no borrow of
        // `self.tabs` is live across the mutation.
        let (profile_id, workspace_id, navigation_sequence, previous_token) =
            match self.tabs.get(&tab_id.0) {
                None => return Err(Failure::blocked("identity.tab_unknown")),
                Some(tab) => {
                    (tab.profile_id, tab.workspace_id, tab.next_navigation_sequence, tab.current_token)
                }
            };

        // The previous document stops being actionable in the same instant the new
        // one exists, so there is no window in which two documents are both live
        // for one tab (backlog §2.3 steps 1 and 3).
        if let Some(previous) = previous_token {
            if let Some(record) = self.documents.get_mut(&previous.0) {
                record.is_current = false;
                record.retire_reason = Some(RetireReason::SupersededByNavigation);
            }
        }

        let token = DocumentToken(self.next_document_token);
        self.next_document_token += 1;
        self.documents.insert(
            token.0,
            DocumentRecord {
                profile_id,
                workspace_id,
                tab_id,
                frame_tree_node_id,
                navigation_sequence,
                upstream_navigation_id,
                snapshot_version: SnapshotVersion(SnapshotVersion::NONE),
                url: url.to_string(),
                is_current: true,
                retire_reason: None,
            },
        );

        if let Some(tab) = self.tabs.get_mut(&tab_id.0) {
            tab.current_token = Some(token);
            tab.current_frame_tree_node_id = frame_tree_node_id;
            tab.next_navigation_sequence = navigation_sequence + 1;
        }

        Ok(DocumentKey {
            profile_id,
            workspace_id,
            tab_id,
            frame_tree_node_id,
            document_token: token,
            navigation_sequence,
            upstream_navigation_id,
        })
    }

    // --- Invalidation. Each returns how many documents it retired. ---

    pub fn on_tab_closed(&mut self, tab_id: TabId) -> usize {
        let retired = self.retire_matching(
            |record| record.tab_id == tab_id,
            RetireReason::TabClosed,
        );
        self.tabs.remove(&tab_id.0);
        retired
    }

    pub fn on_workspace_closed(&mut self, workspace_id: WorkspaceId) -> usize {
        let retired = self.retire_matching(
            |record| record.workspace_id == workspace_id,
            RetireReason::WorkspaceClosed,
        );
        self.tabs.retain(|_, tab| tab.workspace_id != workspace_id);
        self.workspaces.remove(&workspace_id.0);
        // Tasks die with the workspace so no task can outlive the context grant
        // that allowed it.
        self.tasks.retain(|_, task| task.workspace_id != workspace_id);
        retired
    }

    /// Moving a tab between workspaces retires its documents. Context bound to the
    /// old workspace does not follow the tab: a tab dragged elsewhere starts with
    /// no assistant context at all, which is the conservative direction for
    /// cross-workspace leakage (backlog §12).
    pub fn move_tab_to_workspace(
        &mut self,
        tab_id: TabId,
        destination: WorkspaceId,
    ) -> Result<usize, Failure> {
        if !self.tabs.contains_key(&tab_id.0) {
            return Err(Failure::blocked("identity.tab_unknown"));
        }
        if !self.workspaces.contains_key(&destination.0) {
            return Err(Failure::blocked("identity.workspace_unknown"));
        }
        if self.tabs[&tab_id.0].workspace_id == destination {
            return Ok(0);
        }
        let retired = self.retire_matching(
            |record| record.tab_id == tab_id,
            RetireReason::MovedToWorkspace,
        );
        if let Some(tab) = self.tabs.get_mut(&tab_id.0) {
            tab.current_token = None;
            tab.workspace_id = destination;
        }
        Ok(retired)
    }

    fn retire_matching(
        &mut self,
        matches: impl Fn(&DocumentRecord) -> bool,
        reason: RetireReason,
    ) -> usize {
        // Linear scan, bounded by navigations per session. Add a per-tab or
        // per-workspace index if a long session makes a close measurable; do not
        // add one now.
        let mut retired = 0;
        for record in self.documents.values_mut() {
            if record.is_current && matches(record) {
                record.is_current = false;
                record.retire_reason = Some(reason);
                retired += 1;
            }
        }
        retired
    }

    // --- Resolution: the only path from a token to authority. ---

    /// Resolves far enough to *label* an old answer, which is why it accepts a
    /// retired token. Use [`DocumentIdentityRegistry::resolve_in_tab`] before an
    /// action.
    ///
    /// Failure codes:
    /// - `stale` / `identity.token_unknown` — never minted.
    /// - `stale` / `identity.token_retired` — minted, then retired by a
    ///   navigation, close, or move.
    /// - `blocked` / `identity.cross_scope` — live, but owned by a different
    ///   profile or workspace than the scope it arrived on.
    pub fn resolve(&self, token: DocumentToken, scope: &RequestScope) -> Result<DocumentKey, Failure> {
        let Some(record) = self.documents.get(&token.0) else {
            return Err(Failure::stale("identity.token_unknown"));
        };
        // Scope is checked before liveness deliberately: a peer probing tokens
        // belonging to a neighbouring workspace gets `blocked` and cannot use a
        // retired/live difference to learn which tokens once existed there.
        if record.profile_id != scope.profile_id || record.workspace_id != scope.workspace_id {
            return Err(Failure::blocked("identity.cross_scope"));
        }
        if !record.is_current {
            return Err(Failure::stale("identity.token_retired"));
        }
        Ok(Self::key_of(token, record))
    }

    /// The action path. `resolve` answers "is this a live document inside your
    /// scope"; this adds "and is it the document you are about to act on". The second
    /// half is what backlog §5 demands — "a navigation action must target a stable
    /// tab/document handle, not the active tab resolved earlier" — so a plan built
    /// against one tab can never be dispatched into another.
    ///
    /// There is deliberately no "is this the tab's *current* document" check. A
    /// document stops being current at the exact moment its tab shows a different one
    /// (`commit_new_document` retires the previous token in the same step), so such a
    /// check could never fire, and `resolve`'s `identity.token_retired` already reports
    /// that event. Emitting a second code for one event is how a caller's error
    /// handling drifts out of sync with the truth.
    pub fn resolve_in_tab(
        &self,
        token: DocumentToken,
        scope: &RequestScope,
        expected_tab: TabId,
    ) -> Result<DocumentKey, Failure> {
        let resolved = self.resolve(token, scope)?;
        if resolved.tab_id != expected_tab {
            return Err(Failure::blocked("identity.cross_tab"));
        }
        Ok(resolved)
    }

    fn key_of(token: DocumentToken, record: &DocumentRecord) -> DocumentKey {
        DocumentKey {
            profile_id: record.profile_id,
            workspace_id: record.workspace_id,
            tab_id: record.tab_id,
            frame_tree_node_id: record.frame_tree_node_id,
            document_token: token,
            navigation_sequence: record.navigation_sequence,
            upstream_navigation_id: record.upstream_navigation_id,
        }
    }

    // --- Snapshot numbering. ---

    pub fn next_snapshot_version(
        &mut self,
        token: DocumentToken,
        scope: &RequestScope,
    ) -> Result<SnapshotVersion, Failure> {
        let Some(record) = self.documents.get_mut(&token.0) else {
            return Err(Failure::stale("identity.token_unknown"));
        };
        if record.profile_id != scope.profile_id || record.workspace_id != scope.workspace_id {
            return Err(Failure::blocked("identity.cross_scope"));
        }
        if !record.is_current {
            return Err(Failure::stale("identity.token_retired"));
        }
        record.snapshot_version = SnapshotVersion(record.snapshot_version.0 + 1);
        Ok(record.snapshot_version)
    }

    pub fn current_snapshot_version(
        &self,
        token: DocumentToken,
        scope: &RequestScope,
    ) -> Result<SnapshotVersion, Failure> {
        let Some(record) = self.documents.get(&token.0) else {
            return Err(Failure::stale("identity.token_unknown"));
        };
        if record.profile_id != scope.profile_id || record.workspace_id != scope.workspace_id {
            return Err(Failure::blocked("identity.cross_scope"));
        }
        if !record.is_current {
            return Err(Failure::stale("identity.token_retired"));
        }
        if !record.snapshot_version.is_captured() {
            return Err(Failure::unavailable("document.not_captured"));
        }
        Ok(record.snapshot_version)
    }

    /// Classifies a key from an old conversation turn against live state
    /// (backlog §5). `generated_from` is the snapshot version the answer was
    /// actually built on, stored with the turn.
    pub fn assess_staleness(&self, key: &DocumentKey, generated_from: SnapshotVersion) -> StaleStatus {
        let Some(record) = self.documents.get(&key.document_token.0) else {
            return StaleStatus::DocumentReplaced;
        };
        if !record.is_current {
            return match record.retire_reason {
                Some(RetireReason::TabClosed) | Some(RetireReason::WorkspaceClosed) => {
                    StaleStatus::TabClosed
                }
                Some(RetireReason::SupersededByNavigation)
                | Some(RetireReason::MovedToWorkspace)
                | None => StaleStatus::DocumentReplaced,
            };
        }
        if !generated_from.is_captured() {
            // An answer recorded against "no snapshot" cannot be labelled current.
            return StaleStatus::DocumentReplaced;
        }
        if record.snapshot_version != generated_from {
            return StaleStatus::ContentChanged;
        }
        StaleStatus::Current
    }

    /// Metadata, never identity. `None` for a token this registry never issued.
    pub fn url(&self, token: DocumentToken) -> Option<&str> {
        self.documents.get(&token.0).map(|record| record.url.as_str())
    }

    // --- Tasks, scoped to a workspace. ---

    pub fn create_task(&mut self, workspace_id: WorkspaceId) -> Result<TaskId, Failure> {
        if !self.workspaces.contains_key(&workspace_id.0) {
            return Err(Failure::blocked("identity.workspace_unknown"));
        }
        let id = TaskId(self.next_task_id);
        self.next_task_id += 1;
        self.tasks.insert(id.0, TaskRecord { workspace_id });
        Ok(id)
    }

    pub fn resolve_task(&self, task_id: TaskId, scope: &RequestScope) -> Result<TaskId, Failure> {
        let Some(task) = self.tasks.get(&task_id.0) else {
            return Err(Failure::stale("identity.task_unknown"));
        };
        if task.workspace_id != scope.workspace_id {
            return Err(Failure::blocked("identity.task_cross_workspace"));
        }
        Ok(task_id)
    }

    /// Live documents held for a workspace. A workspace close must drive this to
    /// zero; the tests assert it so the token map cannot leak after the user
    /// clears workspace memory (backlog §8).
    pub fn live_document_count(&self, workspace_id: WorkspaceId) -> usize {
        self.documents
            .values()
            .filter(|record| record.workspace_id == workspace_id && record.is_current)
            .count()
    }
}
