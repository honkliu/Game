//! The model platform's shared vocabulary (backlog §6, MOD-01..MOD-03, §5).
//!
//! These types cross the shell/utility pipe in both directions or are shown in
//! the shell's UI, so they live in the contract crate. Nothing here contains
//! provider credentials: a manifest *describes* a model, it never carries the
//! means to call one.
//!
//! Three groups:
//!
//! - **Task options** ([`TaskOptions`], [`TaskIntent`], [`SelectionQuote`],
//!   [`RoutePreference`]) ride inside `model.complete` params next to the question.
//! - **Models and routing** ([`ModelManifest`], [`RoutingDecision`]) answer
//!   `model.list` and make every routing decision inspectable (MOD-03).
//! - **Progress events** ([`TaskEvent`]) are the `task.progress` notification
//!   params: stage changes, the routing decision, and streamed text deltas.
//!
//! The snapshot body the utility pulls with `context.get_snapshot` is
//! [`PagePayload`]; it carries passages and ids, never geometry (the shell keeps
//! geometry — a model has no use for pixel coordinates, and a citation is
//! resolved to a position by the shell, not by the model).

use serde::{Deserialize, Serialize};

use crate::contract::TaskProgress;
use crate::failure::Failure;
use crate::identity::TaskId;

// --- Task options -----------------------------------------------------------

/// What the user asked the browser to do with the content. The selection toolbar
/// (§2.1) is `Ask | Explain | Verify | Translate | Rewrite`; `Summarize` is the
/// page-level default behind "summarize this page".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskIntent {
    #[default]
    Ask,
    Explain,
    Verify,
    Translate,
    Rewrite,
    Summarize,
    /// "Do this": the AI may plan browser actions (open, switch, scroll, …) up to
    /// the task's action ceiling and the configured maximum level (§1 mode 4).
    Act,
}

impl TaskIntent {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskIntent::Ask => "ask",
            TaskIntent::Explain => "explain",
            TaskIntent::Verify => "verify",
            TaskIntent::Translate => "translate",
            TaskIntent::Rewrite => "rewrite",
            TaskIntent::Summarize => "summarize",
            TaskIntent::Act => "act",
        }
    }
}

/// The passage the user selected. **Page-provenance content**: it was read off the
/// page by a renderer probe, so it is data to reason about and never an
/// instruction to follow (§SAF-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionQuote {
    pub exact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
}

/// Which model the user wants, as a preference the router may only *narrow*,
/// never widen: `LocalOnly` is a hard constraint, not a hint.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RoutePreference {
    /// Let the routing policy choose.
    #[default]
    Auto,
    /// Only models that execute on this machine. A cloud model is never a fallback.
    LocalOnly,
    /// A specific model. Falls back along the configured chain only if
    /// `allow_fallback` is set; otherwise an unavailable model is a typed failure.
    Model {
        provider_id: String,
        model_id: String,
        #[serde(default)]
        allow_fallback: bool,
    },
}

/// Everything about a task that is not identity, the question, or its deadline.
/// Every field has a default so a request from an older shell still parses.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskOptions {
    #[serde(default)]
    pub intent: TaskIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SelectionQuote>,
    #[serde(default)]
    pub route: RoutePreference,
    /// Set by the shell when the page's domain forbids cloud models
    /// (§8 "disable cloud models by domain"). Overrides `route`.
    #[serde(default)]
    pub force_local: bool,
    /// Target language for `Translate`, BCP-47. Absent means the user's UI language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_language: Option<String>,
    /// Earlier turns of this conversation, oldest first, so a follow-up is
    /// answered in context. The shell bounds the count and the length.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<ChatTurn>,
}

/// One earlier question or answer of the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatTurn {
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
}

// --- Models and routing -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    /// Runs on this machine; page text does not leave it.
    Local,
    Cloud,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    Free,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    Interactive,
    Standard,
    Batch,
}

/// Backlog §6.1 `ModelManifest`, plus the alias list routing rules match against
/// (`local-fast`, `cloud-standard`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub modalities: Vec<Modality>,
    pub supports_tools: bool,
    pub supports_structured_output: bool,
    pub supports_streaming: bool,
    pub supports_embeddings: bool,
    pub execution: Execution,
    /// Tokens. Used by context reduction to size the prompt budget.
    pub context_window: u32,
    /// Free-form data-handling labels (`local-execution`, `no-retention`, …) that
    /// `require_tags` in a routing rule must all be present in.
    pub data_policy_tags: Vec<String>,
    pub cost_class: CostClass,
    pub latency_class: LatencyClass,
    /// Names a routing rule's `prefer` list may use to refer to this model.
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl ModelManifest {
    pub fn reference(&self) -> ModelRef {
        ModelRef { provider_id: self.provider_id.clone(), model_id: self.model_id.clone() }
    }

    pub fn is_local(&self) -> bool {
        self.execution == Execution::Local
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

/// One candidate the router looked at and why it was kept or dropped. The list is
/// the inspectable part of MOD-03: "why did my private page go to the cloud" has
/// an answer in here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCandidate {
    pub model: ModelRef,
    pub accepted: bool,
    /// A stable machine code (`excluded.cloud_disabled`, `excluded.missing_tag`,
    /// `chosen.preferred`, …), never free text built from page content.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub task: TaskId,
    /// The rule that applied (`selection_explanation`, `page_summary`, …).
    pub route_class: String,
    /// `None` means nothing was eligible and the task fails closed.
    pub chosen: Option<ModelRef>,
    /// Remaining eligible models, in the order they would be tried on failure.
    pub fallbacks: Vec<ModelRef>,
    pub considered: Vec<RouteCandidate>,
    /// True when the chosen model keeps data on this machine.
    pub local_execution: bool,
}

// --- Progress events ---------------------------------------------------------

/// `params` of the `task.progress` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEvent {
    Stage(TaskProgress),
    Routed(RoutingDecision),
    /// A chunk of the model's *answer text*. Never reasoning text. `seq` starts at
    /// 0 and is dense, so the shell can detect a dropped frame.
    Delta { task: TaskId, seq: u64, text: String },
}

impl TaskEvent {
    pub fn task(&self) -> TaskId {
        match self {
            TaskEvent::Stage(progress) => progress.task,
            TaskEvent::Routed(decision) => decision.task,
            TaskEvent::Delta { task, .. } => *task,
        }
    }

    pub fn to_params(&self) -> Result<serde_json::Value, Failure> {
        serde_json::to_value(self).map_err(|error| {
            Failure::unsupported("contract.encode_failed").with_detail(error.to_string())
        })
    }
}

/// Terminal state of a task that ended without an answer: stopped, timed out,
/// refused by policy, or a provider error. Exactly one terminal `task.finish` is
/// emitted per task (ACT-06).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskFailed {
    pub task: TaskId,
    pub failure: Failure,
}

// --- Snapshot payload ---------------------------------------------------------

/// One citable passage, as the utility sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageBlock {
    /// Snapshot-local (`b0`, `b1`, …). Meaningful only with the snapshot version
    /// it arrived with.
    pub block_id: String,
    pub text: String,
}

/// A link on the page. **Page-provenance**: the text and URL are whatever the page
/// says; the AI may open one only if it appears in the snapshot it was shown
/// (`open_tab` with a `link_id`), which is what keeps model-invented navigation out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageLink {
    /// Snapshot-local (`l0`, `l1`, …).
    pub link_id: String,
    pub text: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
}

/// What the browser tells the AI as it happens (shell → utility notification,
/// `browser.event`). Content-free by design: identity and counts, never page text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BrowserEvent {
    TabActivated { tab_id: u64, document: Option<crate::identity::DocumentToken> },
    /// A new document committed in a tab. Any task bound to the previous document
    /// of that tab is now stale and must not act.
    NavigationCommitted {
        tab_id: u64,
        document: crate::identity::DocumentToken,
        navigation_sequence: u64,
        replaced: Option<crate::identity::DocumentToken>,
    },
    /// `history.pushState`-style route change: same document, new sequence.
    HistoryStateChanged { tab_id: u64, document: crate::identity::DocumentToken, navigation_sequence: u64 },
    DocumentDestroyed { document: crate::identity::DocumentToken },
    TabClosed { tab_id: u64 },
    SelectionChanged { document: crate::identity::DocumentToken, selected_chars: u32 },
    /// The user pressed stop in the browser UI.
    UserStopped { task: TaskId },
    /// `ai.json` changed; the utility re-reads it before its next task.
    ConfigChanged,
}

impl BrowserEvent {
    pub fn to_params(&self) -> Result<serde_json::Value, Failure> {
        serde_json::to_value(self).map_err(|error| {
            Failure::unsupported("contract.encode_failed").with_detail(error.to_string())
        })
    }
}

/// `ContextSnapshot::payload` for a page capture.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PagePayload {
    pub blocks: Vec<PageBlock>,
    #[serde(default)]
    pub links: Vec<PageLink>,
    /// Frames present in the capture that were not read.
    #[serde(default)]
    pub frames_not_read: u32,
    /// Passages dropped to stay within the shell's bound.
    #[serde(default)]
    pub dropped_blocks: u32,
}
