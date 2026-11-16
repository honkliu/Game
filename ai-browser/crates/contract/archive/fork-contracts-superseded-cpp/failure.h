// Copyright 2026 The Chromium Authors
// Use of this source code or applicable license file.

#ifndef AI_BROWSER_FOUNDATION_CPP_FAILURE_H_
#define AI_BROWSER_FOUNDATION_CPP_FAILURE_H_

#include <cstdint>
#include <string_view>
#include <variant>

namespace ai_browser {

// The one failure taxonomy for the AI platform (backlog FND-04). Mirror of
// `ai_browser.mojom.FailureKind` in
// //ai_browser/foundation/mojom/ai_platform_failure.mojom; the mojom file is the
// normative source and tools/contract/contract_check.py asserts the two stay
// identical, including the numeric values.
//
// No module in this program declares a second error enumeration. Adding a
// failure mode means adding a member here and in the mojom enum in the same
// commit, or reusing an existing member with a new `code` string.
enum class FailureKind : uint16_t {
  // The capability exists but cannot be reached right now.
  kUnavailable = 0,
  // Policy, permission, or a user setting refused it. Also the class for a
  // token presented outside the scope that owns it.
  kBlocked = 1,
  // The context the operation was planned against no longer describes the
  // document. Requires replanning, never a blind retry.
  kStale = 2,
  // The user or browser ended the work. Not an error condition.
  kCancelled = 3,
  // A deadline elapsed. Side effects may already exist.
  kTimedOut = 4,
  // Well formed and in scope, but this build cannot do it.
  kUnsupported = 5,
  // Succeeded but truncated against a bound; carries an OutputLimit.
  kOutputLimited = 6,
};

// Exact cardinality required by FND-04. A new member has to be a deliberate
// change to this constant as well as to the enum.
inline constexpr uint16_t kFailureKindCount = 7;

// Wire names, shared by logs, telemetry and the generated JSON schema. Indexed
// by FailureKind so there is no second list to keep in sync.
inline constexpr std::string_view kFailureKindNames[kFailureKindCount] = {
    "unavailable", "blocked",    "stale",      "cancelled",
    "timed_out",   "unsupported", "output_limited"};

// Stable string for a kind. The `kFailureKindCount` bound guards a cast of an
// out-of-range integer into the enum, which is the only way to reach the last
// return; in-tree it becomes NOTREACHED_IN_CLIENT_CODE().
std::string_view ToString(FailureKind kind);

// Quantifies kOutputLimited. Mirror of mojom `OutputLimit`.
struct OutputLimit {
  uint32_t byte_limit = 0;
  uint32_t emitted_bytes = 0;
  uint32_t dropped_items = 0;

  bool truncated() const {
    return emitted_bytes < byte_limit || dropped_items > 0;
  }
};

// Mirror of mojom `Failure`. `code` is a stable namespaced telemetry id chosen
// by the producer; `detail` is developer-facing text. Neither may contain page
// text, coordinates, credentials or customer content (backlog §8).
class Failure {
 public:
  Failure(FailureKind kind, std::string_view code);

  FailureKind kind() const { return kind_; }
  std::string_view code() const { return code_; }
  std::string_view detail() const { return detail_; }
  void SetDetail(std::string_view detail) { detail_ = detail; }

  // Re-issuing the identical request could succeed without new context or a
  // policy change.
  bool retryable() const { return retryable_; }
  // A new plan built from freshly observed context is required.
  bool replan_required() const { return replan_required_; }

  // Factories that encode the taxonomy's rules, so no call site can state that
  // a stale operation is retryable.
  static Failure Unavailable(std::string_view code);
  static Failure Blocked(std::string_view code);
  static Failure Stale(std::string_view code);
  static Failure Cancelled(std::string_view code);
  static Failure TimedOut(std::string_view code);
  static Failure Unsupported(std::string_view code);
  static Failure OutputLimited(std::string_view code);

 private:
  Failure(FailureKind kind, std::string_view code, bool retryable,
          bool replan_required);

  FailureKind kind_;
  std::string_view code_;
  std::string_view detail_;
  bool retryable_ = false;
  bool replan_required_ = false;
};

// `base::expected<T, Failure>` equivalent, spelled out here so these contract
// headers build and their tests run without a Chromium checkout. Swap the body
// at integration time; call sites use ok()/value()/error() either way.
template <typename T>
class Result {
 public:
  Result(T value) : storage_(std::in_place_index<0>, std::move(value)) {}
  Result(Failure failure)
      : storage_(std::in_place_index<1>, std::move(failure)) {}

  bool ok() const { return storage_.index() == 0; }
  const T& value() const { return std::get<0>(storage_); }
  T& value() { return std::get<0>(storage_); }
  const Failure& error() const { return std::get<1>(storage_); }

 private:
  std::variant<T, Failure> storage_;
};

}  // namespace ai_browser

#endif  // AI_BROWSER_FOUNDATION_CPP_FAILURE_H_
