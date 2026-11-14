// Copyright 2026 The Chromium Authors
// Use of this source code or applicable license file.

#include "ai_browser/foundation/cpp/failure.h"

namespace ai_browser {

std::string_view ToString(FailureKind kind) {
  switch (kind) {
    case FailureKind::kUnavailable:
      return kFailureKindNames[0];
    case FailureKind::kBlocked:
      return kFailureKindNames[1];
    case FailureKind::kStale:
      return kFailureKindNames[2];
    case FailureKind::kCancelled:
      return kFailureKindNames[3];
    case FailureKind::kTimedOut:
      return kFailureKindNames[4];
    case FailureKind::kUnsupported:
      return kFailureKindNames[5];
    case FailureKind::kOutputLimited:
      return kFailureKindNames[6];
  }
  // Only reachable by casting an integer outside the enum's range.
  return "out_of_range";
}

Failure::Failure(FailureKind kind, std::string_view code)
    : Failure(kind, code, /*retryable=*/false, /*replan_required=*/false) {}

Failure::Failure(FailureKind kind, std::string_view code, bool retryable,
                 bool replan_required)
    : kind_(kind),
      code_(code),
      retryable_(retryable),
      replan_required_(replan_required) {}

Failure Failure::Unavailable(std::string_view code) {
  return Failure(FailureKind::kUnavailable, code, /*retryable=*/true,
                 /*replan_required=*/false);
}

Failure Failure::Blocked(std::string_view code) {
  return Failure(FailureKind::kBlocked, code, /*retryable=*/false,
                 /*replan_required=*/false);
}

Failure Failure::Stale(std::string_view code) {
  // Backlog §5: an action from a stale answer cannot execute without
  // replanning. That rule lives here so no call site can get it wrong.
  return Failure(FailureKind::kStale, code, /*retryable=*/false,
                 /*replan_required=*/true);
}

Failure Failure::Cancelled(std::string_view code) {
  return Failure(FailureKind::kCancelled, code, /*retryable=*/false,
                 /*replan_required=*/false);
}

Failure Failure::TimedOut(std::string_view code) {
  // The outcome is unknown, so a retry may be attempted — but the action
  // runtime gates an automatic retry on Idempotency and never auto-retries a
  // kUnsafe action, because a double submit is a different purchase.
  return Failure(FailureKind::kTimedOut, code, /*retryable=*/true,
                 /*replan_required=*/false);
}

Failure Failure::Unsupported(std::string_view code) {
  return Failure(FailureKind::kUnsupported, code, /*retryable=*/false,
                 /*replan_required=*/false);
}

Failure Failure::OutputLimited(std::string_view code) {
  // Asking for more is a different request with a different bound, not a retry
  // of this one.
  return Failure(FailureKind::kOutputLimited, code, /*retryable=*/false,
                 /*replan_required=*/false);
}

}  // namespace ai_browser
