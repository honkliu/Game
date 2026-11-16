// Copyright 2026 The Chromium Authors
// Use of this source code or applicable license file.

#ifndef AI_BROWSER_FOUNDATION_CPP_IDENTITY_H_
#define AI_BROWSER_FOUNDATION_CPP_IDENTITY_H_

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <string_view>

#include "ai_browser/foundation/cpp/failure.h"

// Identity model for the AI platform (backlog FND-01).
//
// In-tree location: //ai_browser/foundation/cpp/
//
// Ownership rules. The browser process is the sole authority for these
// questions; nothing else may answer them:
//
//   Who mints identity?      DocumentIdentityService, running on the browser
//                            process UI thread. Renderers, the AI utility
//                            process and the model never construct a
//                            DocumentKey — the type has no public constructor,
//                            which is enforced by static_assert in
//                            cpp/test/identity_unittest.cc, not by convention.
//   What crosses a process boundary?  An opaque token integer, plus ids that
//                            the receiving side re-validates. A payload never
//                            asserts profile, workspace or navigation sequence;
//                            those come from the brokered pipe and the registry.
//   What is metadata?        URL, origin, title. Metadata is stored beside a
//                            document, never inside its key, so a URL change
//                            cannot renumber identity and identity cannot be
//                            guessed from a URL.
//   What invalidates what?   Closing a tab invalidates its documents. Changing
//                            workspace invalidates the moved tab's documents.
//                            Each new document commit gets a fresh token that is
//                            never recycled, plus a navigation sequence that
//                            only ever increases within a tab.
//   Who ends identity?       Only the browser process, from navigation and tab
//                            lifecycle events — never from a renderer polling
//                            its own URL (backlog §2.3).
//
// Threading: not thread safe. It is called from the browser UI thread, which is
// where navigation and tab lifecycle are delivered.

namespace ai_browser {

namespace detail {

// Strong id over a uint64. Becomes a base::IdType64<Tag>-derived type at
// integration time; spelled out here so the contract builds and tests without a
// Chromium checkout. Zero means "unset".
template <typename Tag>
class OpaqueId {
 public:
  constexpr OpaqueId() = default;
  constexpr explicit OpaqueId(uint64_t value) : value_(value) {}

  constexpr uint64_t value() const { return value_; }
  constexpr bool is_set() const { return value_ != 0; }

  friend constexpr bool operator==(const OpaqueId&, const OpaqueId&) = default;
  friend constexpr std::strong_ordering operator<=>(const OpaqueId&,
                                                    const OpaqueId&) = default;

 private:
  uint64_t value_ = 0;
};

}  // namespace detail

// A browser profile. Two profiles never share a workspace, a document token, or
// a conversation binding.
class ProfileId : public detail::OpaqueId<struct ProfileIdTag> {
 public:
  using detail::OpaqueId<ProfileIdTag>::OpaqueId;
};

// A workspace: the set of tabs whose contents the assistant may read together.
// The unit of context isolation and the scope every request is checked against.
class WorkspaceId : public detail::OpaqueId<struct WorkspaceIdTag> {
 public:
  using detail::OpaqueId<WorkspaceIdTag>::OpaqueId;
};

// A tab, as minted by this service. Deliberately not a tab index, which the
// user changes by dragging, and not `the active tab`, which is a lookup that can
// go stale between planning and dispatch (backlog §5).
class TabId : public detail::OpaqueId<struct TabIdTag> {
 public:
  using detail::OpaqueId<TabIdTag>::OpaqueId;
};

// A committed document, as minted by this service. The integer is what crosses
// processes; on its own it buys nothing, since only Resolve() can turn it into
// authority, and it fails for a value that was never minted or already retired.
class DocumentTokenId : public detail::OpaqueId<struct DocumentTokenIdTag> {
 public:
  using detail::OpaqueId<DocumentTokenIdTag>::OpaqueId;
};

// A unit of assistant work, scoped to the workspace that created it.
class TaskId : public detail::OpaqueId<struct TaskIdTag> {
 public:
  using detail::OpaqueId<TaskIdTag>::OpaqueId;
};

// Capture generation of one document. 0 means "nothing captured yet" and is
// never a legal argument to an action. Numbering restarts at 1 for a new
// document, which is safe because tokens are never recycled, so
// (token, version) is unique for the life of the process.
struct SnapshotVersion {
  static constexpr uint64_t kNone = 0;

  constexpr SnapshotVersion() = default;
  constexpr explicit SnapshotVersion(uint64_t value) : value(value) {}

  constexpr bool is_captured() const { return value != kNone; }

  uint64_t value = kNone;

  friend constexpr bool operator==(const SnapshotVersion&,
                                   const SnapshotVersion&) = default;
  friend constexpr std::strong_ordering operator<=>(const SnapshotVersion&,
                                                    const SnapshotVersion&) =
      default;
};

// Why a stored answer no longer matches the live document. Mirrors
// `ai_browser.mojom.StaleStatus` (mojom/ai_platform_context.mojom). Displayed,
// never swallowed: a stale answer may stay visible but must be labelled.
enum class StaleStatus : uint16_t {
  kCurrent = 0,
  kContentChanged = 1,
  kDocumentReplaced = 2,
  kTabClosed = 3,
};

// The complete identity of one document (backlog §2.3). Every field is
// required to distinguish two documents, so equality is field-wise — and no
// field is a URL.
//
// Obtained only from DocumentIdentityService. The constructor is private on
// purpose: an endpoint that could build one could claim to be any document.
class DocumentKey {
 public:
  ProfileId profile_id() const { return profile_id_; }
  WorkspaceId workspace_id() const { return workspace_id_; }
  TabId tab_id() const { return tab_id_; }
  // Passed through from content::FrameTreeNodeId; this service does not mint it.
  uint64_t frame_tree_node_id() const { return frame_tree_node_id_; }
  DocumentTokenId document_token() const { return document_token_; }
  uint64_t navigation_sequence() const { return navigation_sequence_; }

  friend constexpr bool operator==(const DocumentKey&,
                                   const DocumentKey&) = default;

 private:
  friend class DocumentIdentityService;

  DocumentKey(ProfileId profile_id, WorkspaceId workspace_id, TabId tab_id,
              uint64_t frame_tree_node_id, DocumentTokenId document_token,
              uint64_t navigation_sequence);

  ProfileId profile_id_;
  WorkspaceId workspace_id_;
  TabId tab_id_;
  uint64_t frame_tree_node_id_;
  DocumentTokenId document_token_;
  uint64_t navigation_sequence_;
};

// The scope a request arrived on, established when the Mojo pipe was brokered.
// Supplied by the binding layer, never read from a message payload — that is
// what stops a peer from naming someone else's workspace.
struct RequestScope {
  ProfileId profile_id;
  WorkspaceId workspace_id;
};

// Browser-process registry of document identity. One per browser process.
class DocumentIdentityService {
 public:
  DocumentIdentityService();
  ~DocumentIdentityService();

  DocumentIdentityService(const DocumentIdentityService&) = delete;
  DocumentIdentityService& operator=(const DocumentIdentityService&) = delete;

  // --- Minting. The only origins of these ids. ---

  ProfileId CreateProfile();
  Result<WorkspaceId> CreateWorkspace(ProfileId profile_id);
  Result<TabId> OpenTab(WorkspaceId workspace_id);

  // A new document committed in `tab`: fresh token, navigation sequence + 1.
  // Called for DocumentCommitted and NavigationRedirected. `url` is recorded as
  // metadata only.
  Result<DocumentKey> OnDocumentCommitted(TabId tab_id,
                                          uint64_t frame_tree_node_id,
                                          std::string_view url);

  // A same-document History API route change (backlog §4.3): new navigation
  // sequence and a new document, so element handles held against the previous
  // route do not survive a client-side route change.
  Result<DocumentKey> OnHistoryStateChanged(TabId tab_id,
                                            std::string_view url);

  // --- Invalidation. Returns the number of documents retired. ---

  size_t OnTabClosed(TabId tab_id);
  size_t OnWorkspaceClosed(WorkspaceId workspace_id);

  // Moving a tab between workspaces retires its documents: context bound to the
  // old workspace must not follow the tab silently.
  Result<size_t> MoveTabToWorkspace(TabId tab_id, WorkspaceId destination);

  // --- Resolution. The only path from a token to authority. ---

  // kStale  identity/identity.token_unknown   — never minted.
  //         identity/identity.token_retired   — minted, then retired by a
  //                                             navigation, close, or move.
  // kBlocked identity/identity.cross_scope    — live, but owned by another
  //                                             profile or workspace than the
  //                                             scope it was presented on.
  // Resolution is used for display and staleness labelling, so a retired token
  // still resolves far enough to label an old answer; use ResolveInTab before an
  // action.
  Result<DocumentKey> Resolve(uint64_t document_token,
                              const RequestScope& scope) const;

  // The action path. Adds two requirements that Resolve deliberately omits: the
  // token must be the document the tab is showing *now*, and it must belong to
  // `expected_tab`. A plan therefore cannot land on a document that navigated
  // while the model was thinking, nor on a sibling tab that happened to be
  // resolved earlier as "active" (backlog §5, §7.3).
  Result<DocumentKey> ResolveInTab(uint64_t document_token,
                                   const RequestScope& scope,
                                   TabId expected_tab) const;

  // --- Snapshot numbering. ---

  Result<SnapshotVersion> NextSnapshotVersion(uint64_t document_token,
                                              const RequestScope& scope);
  Result<SnapshotVersion> CurrentSnapshotVersion(
      uint64_t document_token,
      const RequestScope& scope) const;

  // Classifies a key from an old conversation turn against live state
  // (backlog §5). `generated_from` is the snapshot version the answer was
  // actually built on, stored with the turn.
  StaleStatus AssessStaleness(const DocumentKey& key,
                              SnapshotVersion generated_from) const;

  // --- Metadata. Never part of identity. ---

  std::optional<std::string> url(uint64_t document_token) const;

  // --- Tasks, scoped to a workspace. ---

  Result<TaskId> CreateTask(WorkspaceId workspace_id);
  Result<TaskId> ResolveTask(uint64_t task_id, const RequestScope& scope) const;

  // Live documents held for a workspace. A workspace close must drive this to
  // zero; tests assert it so token maps cannot leak after the user clears
  // workspace memory (backlog §8).
  size_t live_document_count(WorkspaceId workspace_id) const;

 private:
  // Shared by OnDocumentCommitted and OnHistoryStateChanged: mint a token, take
  // the tab's next navigation sequence, and retire whatever the tab was showing.
  Result<DocumentKey> CommitNewDocument(TabId tab_id,
                                        uint64_t frame_tree_node_id,
                                        std::string_view url);

  struct Impl;
  const std::unique_ptr<Impl> impl_;
};

}  // namespace ai_browser

#endif  // AI_BROWSER_FOUNDATION_CPP_IDENTITY_H_
