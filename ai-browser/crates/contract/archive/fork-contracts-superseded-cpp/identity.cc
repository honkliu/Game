// Copyright 2026 The Chromium Authors
// Use of this source code or applicable license file.

#include "ai_browser/foundation/cpp/identity.h"

#include <functional>
#include <unordered_map>
#include <vector>

namespace ai_browser {

struct DocumentIdentityService::Impl {
  // Why a document stopped being its tab's current document. Retired records
  // are kept rather than erased: a session holds on the order of one record per
  // navigation, and an old conversation turn must still be classifiable
  // (`AssessStaleness`), which needs to know how a document died.
  enum class RetireReason : uint8_t {
    kNone = 0,
    kSupersededByNavigation,
    kMovedToWorkspace,
    kTabClosed,
    kWorkspaceClosed,
  };

  struct WorkspaceRecord {
    ProfileId profile_id;
  };

  struct TabRecord {
    ProfileId profile_id;
    WorkspaceId workspace_id;
    // The document an action against this tab must name. Zero before the first
    // commit, and never reused.
    uint64_t current_token = 0;
    uint64_t current_frame_tree_node_id = 0;
    uint64_t next_navigation_sequence = 1;
    bool is_open = true;
  };

  struct DocumentRecord {
    ProfileId profile_id;
    WorkspaceId workspace_id;
    TabId tab_id;
    uint64_t frame_tree_node_id = 0;
    uint64_t navigation_sequence = 0;
    uint64_t snapshot_version = SnapshotVersion::kNone;
    std::string url;
    bool is_current = true;
    RetireReason retire_reason = RetireReason::kNone;
  };

  struct TaskRecord {
    WorkspaceId workspace_id;
  };

  // Ids start at 1; 0 is the unset value of OpaqueId.
  uint64_t next_profile_id = 1;
  uint64_t next_workspace_id = 1;
  uint64_t next_tab_id = 1;
  uint64_t next_document_token = 1;
  uint64_t next_task_id = 1;

  std::unordered_map<uint64_t, WorkspaceRecord> workspaces;
  std::unordered_map<uint64_t, TabRecord> tabs;
  std::unordered_map<uint64_t, DocumentRecord> documents;
  std::unordered_map<uint64_t, TaskRecord> tasks;

  // Linear scans below are bounded by navigations and tabs per session. Add a
  // per-tab or per-workspace index if a long session makes a close measurable;
  // do not add one now.
  size_t RetireIf(const std::function<bool(const DocumentRecord&)>& matches,
                  RetireReason reason) {
    size_t retired = 0;
    for (auto& [token, record] : documents) {
      if (record.is_current && matches(record)) {
        record.is_current = false;
        record.retire_reason = reason;
        ++retired;
      }
    }
    return retired;
  }
};

DocumentIdentityService::DocumentIdentityService()
    : impl_(std::make_unique<Impl>()) {}

DocumentIdentityService::~DocumentIdentityService() = default;

DocumentKey::DocumentKey(ProfileId profile_id, WorkspaceId workspace_id,
                         TabId tab_id, uint64_t frame_tree_node_id,
                         DocumentTokenId document_token,
                         uint64_t navigation_sequence)
    : profile_id_(profile_id),
      workspace_id_(workspace_id),
      tab_id_(tab_id),
      frame_tree_node_id_(frame_tree_node_id),
      document_token_(document_token),
      navigation_sequence_(navigation_sequence) {}

ProfileId DocumentIdentityService::CreateProfile() {
  return ProfileId(impl_->next_profile_id++);
}

Result<WorkspaceId> DocumentIdentityService::CreateWorkspace(
    ProfileId profile_id) {
  if (!profile_id.is_set()) {
    return Failure::Blocked("identity.profile_unset");
  }
  const WorkspaceId workspace_id(impl_->next_workspace_id++);
  impl_->workspaces.emplace(workspace_id.value(),
                            Impl::WorkspaceRecord{profile_id});
  return workspace_id;
}

Result<TabId> DocumentIdentityService::OpenTab(WorkspaceId workspace_id) {
  const auto workspace = impl_->workspaces.find(workspace_id.value());
  if (workspace == impl_->workspaces.end()) {
    return Failure::Blocked("identity.workspace_unknown");
  }
  const TabId tab_id(impl_->next_tab_id++);
  // The tab inherits the profile of the workspace that owns it; a caller cannot
  // open a tab into a profile it does not hold.
  impl_->tabs.emplace(tab_id.value(), Impl::TabRecord{
      /*profile_id=*/workspace->second.profile_id,
      /*workspace_id=*/workspace_id});
  return tab_id;
}

Result<DocumentKey> DocumentIdentityService::CommitNewDocument(
    TabId tab_id, uint64_t frame_tree_node_id, std::string_view url) {
  auto tab = impl_->tabs.find(tab_id.value());
  if (tab == impl_->tabs.end()) {
    return Failure::Blocked("identity.tab_unknown");
  }
  if (!tab->second.is_open) {
    return Failure::Stale("identity.tab_closed");
  }

  // The previous document stops being actionable at the same instant the new
  // one exists, so there is no window in which two documents are both live for
  // one tab (backlog §2.3 steps 1 and 3).
  if (tab->second.current_token != 0) {
    impl_->documents.find(tab->second.current_token)->second.is_current = false;
    impl_->documents.find(tab->second.current_token)->second.retire_reason =
        RetireReason::kSupersededByNavigation;
  }

  const uint64_t token_value = impl_->next_document_token++;
  Impl::DocumentRecord& record = impl_->documents[token_value];
  record.profile_id = tab->second.profile_id;
  record.workspace_id = tab->second.workspace_id;
  record.tab_id = tab_id;
  record.frame_tree_node_id = frame_tree_node_id;
  record.navigation_sequence = tab->second.next_navigation_sequence++;
  record.url = std::string(url);
  tab->second.current_token = token_value;
  tab->second.current_frame_tree_node_id = frame_tree_node_id;

  return DocumentKey(record.profile_id, record.workspace_id, record.tab_id,
                     record.frame_tree_node_id, DocumentTokenId(token_value),
                     record.navigation_sequence);
}

Result<DocumentKey> DocumentIdentityService::OnDocumentCommitted(
    TabId tab_id, uint64_t frame_tree_node_id, std::string_view url) {
  return CommitNewDocument(tab_id, frame_tree_node_id, url);
}

Result<DocumentKey> DocumentIdentityService::OnHistoryStateChanged(
    TabId tab_id, std::string_view url) {
  const auto tab = impl_->tabs.find(tab_id.value());
  if (tab == impl_->tabs.end()) {
    return Failure::Blocked("identity.tab_unknown");
  }
  // Reusing the old document token across a same-document route change would be
  // the cheap answer and is wrong: element handles and action targets held from
  // the previous client-side route may point at nodes that no longer exist, so
  // the route change takes a new sequence and a new token (backlog §4.3).
  return CommitNewDocument(tab_id, tab->second.current_frame_tree_node_id, url);
}

size_t DocumentIdentityService::OnTabClosed(TabId tab_id) {
  const size_t retired =
      impl_->RetireIf([tab_id](const Impl::DocumentRecord& record) {
        return record.tab_id == tab_id;
      }, RetireReason::kTabClosed);
  impl_->tabs.erase(tab_id.value());
  return retired;
}

size_t DocumentIdentityService::OnWorkspaceClosed(WorkspaceId workspace_id) {
  const size_t retired =
      impl_->RetireIf([workspace_id](const Impl::DocumentRecord& record) {
        return record.workspace_id == workspace_id;
      }, RetireReason::kWorkspaceClosed);

  for (auto it = impl_->tabs.begin(); it != impl_->tabs.end();) {
    if (it->second.workspace_id == workspace_id) {
      it = impl_->tabs.erase(it);
    } else {
      ++it;
    }
  }
  impl_->workspaces.erase(workspace_id.value());
  // Tasks die with the workspace so no task outlives the context grant that
  // allowed it.
  for (auto it = impl_->tasks.begin(); it != impl_->tasks.end();) {
    if (it->second.workspace_id == workspace_id) {
      it = impl_->tasks.erase(it);
    } else {
      ++it;
    }
  }
  return retired;
}

Result<size_t> DocumentIdentityService::MoveTabToWorkspace(
    TabId tab_id, WorkspaceId destination) {
  auto tab = impl_->tabs.find(tab_id.value());
  if (tab == impl_->tabs.end()) {
    return Failure::Blocked("identity.tab_unknown");
  }
  if (impl_->workspaces.find(destination.value()) ==
      impl_->workspaces.end()) {
    return Failure::Blocked("identity.workspace_unknown");
  }
  if (tab->second.workspace_id == destination) {
    return size_t{0};
  }

  // Context bound to the old workspace does not follow the tab: a tab dragged
  // into another workspace starts with no assistant context at all, which is
  // the conservative direction for cross-workspace leakage (backlog §12).
  const size_t retired =
      impl_->RetireIf([tab_id](const Impl::DocumentRecord& record) {
        return record.tab_id == tab_id;
      }, RetireReason::kMovedToWorkspace);
  tab->second.current_token = 0;
  tab->second.workspace_id = destination;
  return retired;
}

Result<DocumentKey> DocumentIdentityService::Resolve(
    uint64_t document_token, const RequestScope& scope) const {
  const auto it = impl_->documents.find(document_token);
  if (it == impl_->documents.end()) {
    return Failure::Stale("identity.token_unknown");
  }
  const Impl::DocumentRecord& record = it->second;
  // Scope is checked before liveness on purpose: a peer probing tokens from a
  // neighbouring workspace gets kBlocked, and cannot use the kStale/kBlocked
  // difference to learn which tokens once existed in its own scope.
  if (record.profile_id != scope.profile_id ||
      record.workspace_id != scope.workspace_id) {
    return Failure::Blocked("identity.cross_scope");
  }
  if (!record.is_current) {
    return Failure::Stale("identity.token_retired");
  }
  return DocumentKey(record.profile_id, record.workspace_id, record.tab_id,
                     record.frame_tree_node_id, DocumentTokenId(document_token),
                     record.navigation_sequence);
}

Result<DocumentKey> DocumentIdentityService::ResolveInTab(
    uint64_t document_token, const RequestScope& scope,
    TabId expected_tab) const {
  Result<DocumentKey> resolved = Resolve(document_token, scope);
  if (!resolved.ok()) {
    return resolved;
  }
  const auto tab = impl_->tabs.find(expected_tab.value());
  if (tab == impl_->tabs.end()) {
    return Failure::Stale("identity.tab_closed");
  }
  // The action path additionally requires this token to be what the tab is
  // showing now, so a plan cannot land on a document that navigated while the
  // model was thinking.
  if (tab->second.current_token != document_token) {
    return Failure::Stale("identity.document_not_current");
  }
  if (resolved.value().tab_id() != expected_tab) {
    return Failure::Blocked("identity.cross_tab");
  }
  return resolved;
}

Result<SnapshotVersion> DocumentIdentityService::NextSnapshotVersion(
    uint64_t document_token, const RequestScope& scope) const {
  const auto it = impl_->documents.find(document_token);
  if (it == impl_->documents.end()) {
    return Failure::Stale("identity.token_unknown");
  }
  Impl::DocumentRecord& record = it->second;
  if (record.profile_id != scope.profile_id ||
      record.workspace_id != scope.workspace_id) {
    return Failure::Blocked("identity.cross_scope");
  }
  if (!record.is_current) {
    return Failure::Stale("identity.token_retired");
  }
  record.snapshot_version += 1;
  return SnapshotVersion(record.snapshot_version);
}

Result<SnapshotVersion> DocumentIdentityService::CurrentSnapshotVersion(
    uint64_t document_token, const RequestScope& scope) const {
  const auto it = impl_->documents.find(document_token);
  if (it == impl_->documents.end()) {
    return Failure::Stale("identity.token_unknown");
  }
  const Impl::DocumentRecord& record = it->second;
  if (record.profile_id != scope.profile_id ||
      record.workspace_id != scope.workspace_id) {
    return Failure::Blocked("identity.cross_scope");
  }
  if (!record.is_current) {
    return Failure::Stale("identity.token_retired");
  }
  if (record.snapshot_version == SnapshotVersion::kNone) {
    return Failure::Unavailable("document.not_captured");
  }
  return SnapshotVersion(record.snapshot_version);
}

StaleStatus DocumentIdentityService::AssessStaleness(
    const DocumentKey& key, SnapshotVersion generated_from) const {
  const auto it = impl_->documents.find(key.document_token().value());
  if (it == impl_->documents.end()) {
    return StaleStatus::kDocumentReplaced;
  }
  const Impl::DocumentRecord& record = it->second;
  if (!record.is_current) {
    switch (record.retire_reason) {
      case RetireReason::kTabClosed:
      case RetireReason::kWorkspaceClosed:
        return StaleStatus::kTabClosed;
      case RetireReason::kSupersededByNavigation:
      case RetireReason::kMovedToWorkspace:
      case RetireReason::kNone:
        return StaleStatus::kDocumentReplaced;
    }
  }
  if (!generated_from.is_captured()) {
    // An answer recorded against "no snapshot" cannot be labelled current.
    return StaleStatus::kDocumentReplaced;
  }
  if (record.snapshot_version != generated_from.value) {
    return StaleStatus::kContentChanged;
  }
  return StaleStatus::kCurrent;
}

std::optional<std::string> DocumentIdentityService::url(
    uint64_t document_token) const {
  const auto it = impl_->documents.find(document_token);
  if (it == impl_->documents.end()) {
    return std::nullopt;
  }
  return it->second.url;
}

Result<TaskId> DocumentIdentityService::CreateTask(
    WorkspaceId workspace_id) {
  if (impl_->workspaces.find(workspace_id.value()) ==
      impl_->workspaces.end()) {
    return Failure::Blocked("identity.workspace_unknown");
  }
  const TaskId task_id(impl_->next_task_id++);
  impl_->tasks.emplace(task_id.value(), Impl::TaskRecord{workspace_id});
  return task_id;
}

Result<TaskId> DocumentIdentityService::ResolveTask(
    uint64_t task_id, const RequestScope& scope) const {
  const auto it = impl_->tasks.find(task_id);
  if (it == impl_->tasks.end()) {
    return Failure::Stale("identity.task_unknown");
  }
  if (it->second.workspace_id != scope.workspace_id) {
    return Failure::Blocked("identity.task_cross_workspace");
  }
  return TaskId(task_id);
}

size_t DocumentIdentityService::live_document_count(
    WorkspaceId workspace_id) const {
  size_t count = 0;
  for (const auto& [token, record] : impl_->documents) {
    if (record.workspace_id == workspace_id && record.is_current) {
      ++count;
    }
  }
  return count;
}

}  // namespace ai_browser
