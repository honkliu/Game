//! The shell's half of document identity: the contract's registry, wired to real
//! tabs and real navigations.
//!
//! [`DocumentIdentityRegistry`] ships in `ai-platform-contract` and is fully tested
//! there. What it cannot do is know which of *this* shell's tabs is which, so this
//! module owns exactly three things: the one profile and workspace this build has,
//! the map from a shell `TabId` to a contract `TabId`, and the call that tells the
//! registry a document committed. Everything else is a pass-through to the
//! registry's own resolution, so there is one answer to "is this document still
//! the one on screen".
//!
//! ## Why this is not just the shell's `navigation_seq`
//!
//! `AppState::navigation_seq` is a process-wide counter used to label log lines and
//! error payloads. It is not an identity: it does not distinguish two tabs, it is
//! not retired, and nothing resolves against it. The registry's
//! `navigation_sequence` is per-document, monotonic per tab, and reachable only
//! through a [`DocumentKey`] — which the contract makes unforgeable from a frame.
//! Citation staleness ([`super::answer`]) is decided on *that*, not on the log
//! counter.
//!
//! ## One profile, one workspace
//!
//! The contract models many. This build has one of each because the shell has one
//! user-data-folder and no workspace UI, and minting the real types is cheaper than
//! inventing a shell-local stand-in that has to be replaced when workspaces land.
//! The consequence is stated rather than hidden: `identity.cross_scope` cannot fire
//! in this build, because every document this process mints lives in the one scope.

use std::collections::HashMap;

use ai_platform_contract::failure::Failure;
use ai_platform_contract::identity::{
    DocumentIdentityRegistry, DocumentKey, DocumentToken, ProfileId, RequestScope, SnapshotVersion,
    StaleStatus, TabId as ContractTabId, TaskId, WorkspaceId,
};

/// Shell tab id → contract tab id.
///
/// Explicit rather than `TabId(shell.0)`: the shell recycles tab ids through
/// `tab.reopen_closed` and a restored session, and a contract token must never be
/// reissued for a different tab — the contract relies on tokens being unique for
/// the life of the process, which is exactly what a reused integer breaks.
pub struct Identity {
    registry: DocumentIdentityRegistry,
    profile: ProfileId,
    workspace: WorkspaceId,
    tabs: HashMap<u64, ContractTabId>,
    /// Shell tab → the token of the document it last committed. The registry keeps
    /// this itself but exposes no per-tab accessor, and re-deriving it by scanning
    /// every document would mean trusting `is_current` through a private field. This
    /// is a cache of a fact this module caused, and `current` still asks the registry
    /// whether the token resolves, so a retired token cannot be read as live.
    current: HashMap<u64, DocumentToken>,
    /// True once the profile and workspace exist. `new` mints them, so this is only
    /// false for the `Default` construction a test would have to avoid; the accessors
    /// refuse rather than unwrap.
    minted: bool,
}

impl Identity {
    pub fn new() -> Self {
        let mut registry = DocumentIdentityRegistry::new();
        let profile = registry.create_profile();
        // `create_workspace` only fails for an unknown profile, which cannot happen
        // three lines after minting it. Nothing is swallowed: the result is kept and
        // every later call checks `minted`, so an impossible-but-unreachable state
        // degrades to a typed refusal instead of a panic.
        let workspace = registry.create_workspace(profile);
        match workspace {
            Ok(workspace) => Identity {
                registry,
                profile,
                workspace,
                tabs: HashMap::new(),
                current: HashMap::new(),
                minted: true,
            },
            Err(_) => Identity {
                registry,
                profile,
                workspace: WorkspaceId(0),
                tabs: HashMap::new(),
                current: HashMap::new(),
                minted: false,
            },
        }
    }

    /// The scope every inbound request in this process is checked against. Taken
    /// from the connection — here, from the process itself — never from a payload,
    /// which is what makes `RequestScope` worth passing at all.
    pub fn scope(&self) -> Option<RequestScope> {
        self.minted.then_some(RequestScope {
            profile_id: self.profile,
            workspace_id: self.workspace,
        })
    }

    /// The registry itself, for the one operation that is a whole check rather than
    /// a pass-through: [`ai_platform_contract::contract::AnswerWithCitations::verify`]
    /// takes a `&DocumentIdentityRegistry` and does the citation staleness check
    /// inside the contract, where it is tested. Read-only, so exposing it cannot
    /// mint or retire anything.
    pub fn registry(&self) -> &DocumentIdentityRegistry {
        &self.registry
    }

    /// The contract tab for a shell tab, minted on first sight. Tab creation is
    /// deliberately lazy: a restored session opens ten tabs at startup and only the
    /// ones that carry AI work need an identity, while `on_document_committed` still
    /// refuses (typed) for any tab this never saw.
    pub fn tab(&mut self, shell_tab: u64) -> Result<ContractTabId, Failure> {
        if !self.minted {
            return Err(Failure::unavailable("identity.workspace_unavailable"));
        }
        if let Some(existing) = self.tabs.get(&shell_tab) {
            return Ok(*existing);
        }
        let minted = self.registry.open_tab(self.workspace)?;
        self.tabs.insert(shell_tab, minted);
        Ok(minted)
    }

    /// A document committed in `shell_tab`. Previous documents in that tab are
    /// retired by the registry in the same step, which is what makes an old
    /// citation resolve to `identity.token_retired` rather than to a live document.
    pub fn on_document_committed(
        &mut self,
        shell_tab: u64,
        url: &str,
    ) -> Result<DocumentKey, Failure> {
        let tab = self.tab(shell_tab)?;
        // `frame_tree_node_id` is 0 until the shell can read one from the webview.
        // The contract documents 0 as "not yet recorded" rather than a value it
        // trusts, so passing it is honest; passing a made-up number would not be.
        let key = self
            .registry
            .on_document_committed(tab, 0, None, url)?;
        self.current.insert(shell_tab, key.document_token());
        Ok(key)
    }

    /// A same-document History API route change. Same contract path, new token and
    /// new sequence, because text offsets and rects from the old route are not
    /// promises about the new one.
    pub fn on_history_state_changed(
        &mut self,
        shell_tab: u64,
        url: &str,
    ) -> Result<DocumentKey, Failure> {
        let tab = self.tab(shell_tab)?;
        let key = self.registry.on_history_state_changed(tab, url)?;
        self.current.insert(shell_tab, key.document_token());
        Ok(key)
    }

    /// The live document of a shell tab, or `None` when the tab has never committed
    /// one. This is the only way the caller learns a document exists — there is no
    /// "assume the active tab" path here, by the same rule the contract writes into
    /// `resolve_in_tab`.
    pub fn current(&self, shell_tab: u64) -> Option<DocumentKey> {
        let scope = self.scope()?;
        let token = *self.current.get(&shell_tab)?;
        // The map is a cache of a fact this module caused, so the registry still
        // gets the last word: a token it has retired resolves to `None` here rather
        // than to a key that looks live.
        self.registry.resolve(token, &scope).ok()
    }

    /// The shell tab whose live document is `token`, if any.
    pub fn tab_of_document(&self, token: DocumentToken) -> Option<u64> {
        self.current
            .iter()
            .find(|(_, t)| **t == token)
            .map(|(tab, _)| *tab)
    }

    /// The tab now shows something that is not a readable page (a built-in page,
    /// an error page). Its old document is no longer "current", so a question
    /// cannot be bound to it.
    pub fn forget_current(&mut self, shell_tab: u64) {
        self.current.remove(&shell_tab);
    }

    /// Record that a snapshot was taken of `token`, returning its generation.
    pub fn snapshot_captured(&mut self, token: DocumentToken) -> Result<SnapshotVersion, Failure> {
        let scope = self.scope().ok_or_else(|| {
            Failure::unavailable("identity.workspace_unavailable")
        })?;
        self.registry.next_snapshot_version(token, &scope)
    }

    /// The generation a document's newest snapshot is at. `unavailable` /
    /// `document.not_captured` before the first capture, which is the contract's own
    /// code for "nothing has been read from this document yet".
    pub fn snapshot_version(&self, token: DocumentToken) -> Result<SnapshotVersion, Failure> {
        let scope = self
            .scope()
            .ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        self.registry.current_snapshot_version(token, &scope)
    }

    /// Resolve a token a citation carried, asserting the tab it claims to belong to.
    /// The action path: `stale` for unknown or retired, `blocked` for another scope
    /// or another tab.
    pub fn resolve_in_tab(
        &self,
        token: DocumentToken,
        shell_tab: u64,
    ) -> Result<DocumentKey, Failure> {
        let scope = self
            .scope()
            .ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        let tab = self
            .tabs
            .get(&shell_tab)
            .copied()
            .ok_or_else(|| Failure::blocked("identity.tab_unknown"))?;
        self.registry.resolve_in_tab(token, &scope, tab)
    }

    /// How a stored answer relates to what is on screen now. Displayed, never
    /// swallowed — but refusal to act is [`super::answer::AnswerStore`]'s decision,
    /// not this label's.
    pub fn assess_staleness(&self, key: &DocumentKey, generated_from: SnapshotVersion) -> StaleStatus {
        self.registry.assess_staleness(key, generated_from)
    }

    /// A new unit of assistant work, scoped to this process's workspace.
    pub fn create_task(&mut self) -> Result<TaskId, Failure> {
        if !self.minted {
            return Err(Failure::unavailable("identity.workspace_unavailable"));
        }
        self.registry.create_task(self.workspace)
    }

    pub fn resolve_task(&self, task: TaskId) -> Result<TaskId, Failure> {
        let scope = self
            .scope()
            .ok_or_else(|| Failure::unavailable("identity.workspace_unavailable"))?;
        self.registry.resolve_task(task, &scope)
    }

    /// The URL the registry recorded for a token. Metadata only, and only ever for a
    /// token this process minted.
    pub fn url(&self, token: DocumentToken) -> Option<&str> {
        self.registry.url(token)
    }

    /// Forget a closed tab. The registry retires its documents so a citation cannot
    /// resolve against a tab that no longer exists; the mapping goes with it, because
    /// the shell may hand that tab id to a different page later.
    ///
    /// Returns how many documents were retired, which the caller logs rather than
    /// drops: a citation that stops resolving should be explainable.
    pub fn on_tab_closed(&mut self, shell_tab: u64) -> usize {
        let retired = self
            .tabs
            .remove(&shell_tab)
            .map(|tab| self.registry.on_tab_closed(tab))
            .unwrap_or(0);
        self.current.retain(|shell, _| *shell != shell_tab);
        retired
    }

    /// Live documents this process still holds for its workspace. Non-zero after a
    /// session that navigated a lot is expected (one per committed document, kept
    /// deliberately by the contract); it must go to zero when the tabs close.
    pub fn live_documents(&self) -> usize {
        if !self.minted {
            return 0;
        }
        self.registry.live_document_count(self.workspace)
    }

    /// The shell's own tab count, so a test can assert the two maps cannot drift.
    pub fn tracked_tabs(&self) -> usize {
        self.tabs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tab that has never committed has no document, rather than a zero-valued one.
    #[test]
    fn a_tab_has_no_document_until_it_commits_one() {
        let mut id = Identity::new();
        let tab = id.tab(7).expect("tab minted");
        assert_eq!(tab, ContractTabId(1), "contract ids are minted, not mirrored");
        assert!(id.current(7).is_none(), "no commit yet, so no document");
        // Minting twice must return the same contract tab: a second id for one shell
        // tab would let two documents claim the same page.
        assert_eq!(id.tab(7).unwrap(), tab);
        assert_eq!(id.tracked_tabs(), 1);
    }

    /// The identity claim the whole slice stands on: two commits in one tab are two
    /// documents, the older one stops resolving, and their sequences differ.
    #[test]
    fn a_navigation_makes_a_new_document_and_retires_the_old_one() {
        let mut id = Identity::new();
        let first = id.on_document_committed(1, "https://a.example/").unwrap();
        let second = id.on_document_committed(1, "https://b.example/").unwrap();

        assert_eq!(first.tab_id(), second.tab_id(), "same tab");
        assert_ne!(first.document_token(), second.document_token());
        assert_eq!(first.navigation_sequence() + 1, second.navigation_sequence());
        assert_eq!(id.current(1).unwrap().document_token(), second.document_token());
        // The old token no longer resolves — which is the whole mechanism a stale
        // citation is refused by.
        let stale = id
            .resolve_in_tab(first.document_token(), 1)
            .expect_err("a superseded document must not resolve");
        assert_eq!(stale.code, "identity.token_retired");
        assert_eq!(stale.kind, ai_platform_contract::failure::FailureKind::Stale);
    }

    /// A citation cannot be resolved against a tab other than the one it was made
    /// in, even though both tabs are live in the same scope.
    #[test]
    fn a_document_does_not_resolve_in_another_tab() {
        let mut id = Identity::new();
        let doc = id.on_document_committed(1, "https://a.example/").unwrap();
        id.on_document_committed(2, "https://b.example/").unwrap();
        let cross = id
            .resolve_in_tab(doc.document_token(), 2)
            .expect_err("tab 2 does not own tab 1's document");
        assert_eq!(cross.code, "identity.cross_tab");
        assert_eq!(cross.kind, ai_platform_contract::failure::FailureKind::Blocked);
        // And a tab this module never minted is refused, not lazily admitted.
        assert!(id.resolve_in_tab(doc.document_token(), 99).is_err());
    }

    /// Snapshot generations start at 1 per document, and an uncaptured document is
    /// `document.not_captured` rather than version 0.
    #[test]
    fn snapshot_generations_are_per_document_and_start_at_one() {
        let mut id = Identity::new();
        let doc = id.on_document_committed(1, "https://a.example/").unwrap();
        let before = id.snapshot_version(doc.document_token());
        assert_eq!(before.unwrap_err().code, "document.not_captured");
        assert_eq!(id.snapshot_captured(doc.document_token()).unwrap().0, 1);
        assert_eq!(id.snapshot_captured(doc.document_token()).unwrap().0, 2);
        assert_eq!(id.snapshot_version(doc.document_token()).unwrap().0, 2);

        let other = id.on_document_committed(2, "https://b.example/").unwrap();
        assert_eq!(
            other.navigation_sequence(),
            1,
            "a second tab's first document is not numbered after tab 1's"
        );
        assert!(id.snapshot_captured(other.document_token()).unwrap().0 == 1);
    }

    /// Closing a tab retires its documents; the tab's shell id is then free, and a
    /// citation against the old document stops resolving.
    #[test]
    fn closing_a_tab_retires_its_documents() {
        let mut id = Identity::new();
        let doc = id.on_document_committed(1, "https://a.example/").unwrap();
        assert_eq!(
            id.current(1).unwrap().document_token(),
            doc.document_token(),
            "a commit is visible through `current` without the caller having to cache it"
        );
        assert_eq!(id.live_documents(), 1);

        id.on_tab_closed(1);

        assert_eq!(id.live_documents(), 0, "a closed tab holds no live document");
        assert!(id.current(1).is_none());
        assert_eq!(id.tracked_tabs(), 0, "the shell id is free for a new page");
        let after = id.on_document_committed(1, "https://c.example/").unwrap();
        assert_ne!(
            after.document_token(),
            doc.document_token(),
            "tokens are never recycled, so a reused shell id cannot resurrect a citation"
        );
    }

    /// Tasks are minted and resolve in this scope.
    #[test]
    fn a_task_is_minted_in_this_workspaces_scope() {
        let mut id = Identity::new();
        let task = id.create_task().unwrap();
        assert_eq!(task.0, 1);
        assert_eq!(id.resolve_task(task).unwrap(), task);
        assert!(id.resolve_task(TaskId(404)).is_err());
    }
}
