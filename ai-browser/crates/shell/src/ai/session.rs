//! Everything the shell knows about AI work in flight, kept free of windows and
//! webviews so the rules can be tested.
//!
//! The shell is the authority: it mints identity, decides what an action may do,
//! and holds the only copy of page geometry. This module is where those decisions
//! are made; `app::ai_glue` only carries them out against real tabs.

use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use ai_platform_contract::config::AiConfig;
use ai_platform_contract::contract::{
    level_of_action_kind, ActionLevel, ActionRequest, ContextCoverage, ContextSnapshot, CoverageGap,
    CoverageGapKind, CoverageStatus,
};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::identity::{DocumentKey, SnapshotVersion};
use ai_platform_contract::model::{PageBlock, PageLink, PagePayload};

use super::answer::AnswerStore;
use super::context::{ContextStore, PageContext};
use super::identity::Identity;
use super::link::Link;
use super::policy::ReleaseZeroGate;

/// The selection text the page reports is untrusted and bounded.
pub const MAX_SELECTION_CHARS: usize = 4_000;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What the page last said is selected in one tab. **Page-provenance**: it came
/// from script running in the page, so it is bounded, never executed, and only ever
/// used as the text of a question the user chose to ask.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub text: String,
    /// `[left, top, width, height]` in CSS pixels of the page viewport.
    pub rect: Option<[f64; 4]>,
}

impl Selection {
    /// Parse one probe message, or `None` if it is not a well-formed selection.
    pub fn from_probe(raw: &str) -> Option<Selection> {
        if raw.len() > 64 * 1024 {
            return None;
        }
        let value: Value = serde_json::from_str(raw).ok()?;
        if value.get("ai").and_then(Value::as_str) != Some("sel") {
            return None;
        }
        let text: String = value
            .get("text")
            .and_then(Value::as_str)?
            .chars()
            .take(MAX_SELECTION_CHARS)
            .collect();
        let rect = value.get("rect").and_then(Value::as_array).and_then(|r| {
            let mut out = [0.0f64; 4];
            if r.len() != 4 {
                return None;
            }
            for (slot, v) in out.iter_mut().zip(r) {
                let n = v.as_f64()?;
                if !n.is_finite() || n.abs() > 1.0e6 {
                    return None;
                }
                *slot = n;
            }
            Some(out)
        });
        Some(Selection { text, rect })
    }
}

/// True when a probe message is the page's "my content changed" notice. The notice
/// carries a reason from a fixed set and nothing else; anything richer is ignored.
pub fn is_change_probe(raw: &str) -> bool {
    if raw.len() > 1024 {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(raw) else { return false };
    value.get("ai").and_then(Value::as_str) == Some("changed")
        && matches!(value.get("reason").and_then(Value::as_str), Some("dom" | "input" | "click" | "route" | "load"))
}

/// What the shell last read from a tab's page for the assistant's live context. The
/// text itself is not kept here (it is re-read when a run starts); this is what the
/// pane needs to show that the content moved and how much there is.
#[derive(Debug, Clone, Default)]
pub struct PageSnapshot {
    pub read_seq: u64,
    /// Increases every time the read content differs from the previous read.
    pub rev: u64,
    pub hash: u64,
    pub chars: usize,
    pub headings: usize,
    pub fields: usize,
    pub filled: usize,
    pub preview: String,
    pub at_ms: i64,
    /// A read is in flight / another change arrived during it / a re-read is scheduled.
    pub reading: bool,
    pub dirty: bool,
    pub scheduled: bool,
}

/// FNV-1a: a cheap, stable fingerprint to tell "same content" from "changed".
pub fn fingerprint(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.as_bytes().iter().chain(std::iter::once(&0u8)) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

pub struct TaskState {
    pub turn_id: u64,
    pub tab: u64,
    pub document: DocumentKey,
    pub snapshot: SnapshotVersion,
    pub started: Instant,
    pub ceiling: ActionLevel,
    pub deadline_ms: i64,
    /// Tabs this task opened, the only tabs `close_tab` may close.
    pub opened_tabs: Vec<u64>,
    /// Streamed answer text so far (the model's answer, never its reasoning).
    pub text: String,
    pub next_delta: u64,
    pub model: String,
    pub local: bool,
    pub stopped: bool,
    pub question: String,
    pub options: ai_platform_contract::model::TaskOptions,
    /// Other tabs the user chose to include; each is read in turn before the task starts.
    pub extra_tabs: Vec<u64>,
    /// The active tab's snapshot, held while the extra tabs are being read.
    pub primary: Option<PageContext>,
    /// Passages from the extra tabs, ids prefixed `t<tab>.` so a citation names its tab.
    pub merged: Vec<super::context::Block>,
    /// Reads of the extra tab in progress that came back empty (a woken tab may still be loading).
    pub extra_attempts: u8,
}

/// What an outstanding shell → utility request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Complete(u64),
    Cancel(u64),
    Models,
    /// An `agent.run` acknowledgement.
    AgentRun(u64),
    /// An `agent.control` reply for the UI request with this id (0: nobody waits).
    Control(crate::ipc::Surface, u64),
}

pub struct AiSession {
    pub link_generation: u64,
    pub provider_key_saved: bool,
    pub ui: super::ui_state::UiState,
    pub identity: Identity,
    pub contexts: ContextStore,
    pub answers: AnswerStore,
    pub gate: ReleaseZeroGate,
    pub link: Option<Link>,
    pub link_error: Option<String>,
    pub config: AiConfig,
    pub tasks: HashMap<u64, TaskState>,
    pub pending: HashMap<u64, Pending>,
    pub selections: HashMap<u64, Selection>,
    /// Tabs the AI opened. Shown as managed and never closed by anyone but the
    /// task that opened them or the user.
    pub ai_tabs: HashSet<u64>,
    pub models: Vec<Value>,
    pub toolbar_open: bool,
    pub anchor: (i32, i32),
    /// The conversation so far, sent with each question so follow-ups have context.
    pub history: Vec<ai_platform_contract::model::ChatTurn>,
    /// Tab → the document token its passages were read from, for citations into other tabs.
    pub extra_docs: HashMap<u64, u64>,
    /// The agent run in progress (0 when none) and the counter for new ones.
    pub agent_run: u64,
    pub agent_seq: u64,
    /// Browser tool calls that wait for a page script: utility request id → the call.
    pub agent_evals: HashMap<u64, AgentEval>,
    pub agent_documents: HashMap<u64, DocumentKey>,
    /// Runs and previews waiting for the active page's content before they are sent
    /// to the engine: job id → what to send.
    pub agent_jobs: HashMap<u64, AgentJob>,
    pub agent_job_seq: u64,
    /// Per tab: the last content read for the live context (see `PageSnapshot`).
    pub page_snaps: HashMap<u64, PageSnapshot>,
}

/// A request to the engine that carries the active page's content, held until the
/// page has been read.
#[derive(Debug)]
pub struct AgentJob {
    pub tab: Option<u64>,
    pub document: Option<DocumentKey>,
    pub method: ai_platform_contract::frame::Method,
    pub params: serde_json::Value,
    pub pending: Pending,
    /// The run this job starts, so a Stop during the read can cancel it.
    pub run: Option<u64>,
}

/// A browser tool call that runs a script in a tab and answers when it returns.
#[derive(Debug, Clone)]
pub struct AgentEval {
    pub tab: u64,
    pub document: Option<DocumentKey>,
    pub run: Option<u64>,
    pub script: String,
    pub attempts: u8,
    /// A page read is retried while the page is still empty (a freshly woken tab).
    pub is_read: bool,
    /// A popup sweep over several pages: the pages still to do and what each answered.
    pub chain: Option<Chain>,
}

#[derive(Debug, Clone)]
pub struct Chain {
    pub rest: Vec<u64>,
    pub results: Vec<serde_json::Value>,
    /// Answer the agent's request at the end (otherwise announce it to the shell UI).
    pub reply: bool,
}

impl Default for AiSession {
    fn default() -> Self {
        AiSession::new()
    }
}

impl AiSession {
    pub fn new() -> AiSession {
        AiSession {
            link_generation: 0,
            provider_key_saved: false,
            ui: super::ui_state::UiState::default(),
            identity: Identity::new(),
            contexts: ContextStore::default(),
            answers: AnswerStore::default(),
            gate: ReleaseZeroGate::new(),
            link: None,
            link_error: None,
            config: AiConfig::default(),
            tasks: HashMap::new(),
            pending: HashMap::new(),
            selections: HashMap::new(),
            ai_tabs: HashSet::new(),
            models: Vec::new(),
            toolbar_open: false,
            anchor: (0, 0),
            history: Vec::new(),
            extra_docs: HashMap::new(),
            agent_run: 0,
            agent_seq: 0,
            agent_evals: HashMap::new(),
            agent_documents: HashMap::new(),
            agent_jobs: HashMap::new(),
            agent_job_seq: 0,
            page_snaps: HashMap::new(),
        }
    }

    /// Whether any question may be asked right now, as a typed reason when not.
    pub fn availability(&self) -> Result<(), Failure> {
        if !self.config.flags.ai_enabled {
            return Err(Failure::blocked("ai.disabled").with_detail("AI is turned off in ai.json"));
        }
        if !self.config.flags.page_context {
            return Err(Failure::blocked("ai.page_context_disabled"));
        }
        Ok(())
    }

    /// Decide one action request. Order is deliberate: unknown task, kill
    /// switches, level, domain, then staleness, then deadline — so a refusal names
    /// the policy before it names the page.
    pub fn admit_action(&self, request: &ActionRequest, now_ms: i64) -> Result<ActionLevel, Failure> {
        let task = self
            .tasks
            .get(&request.task.0)
            .ok_or_else(|| Failure::stale("action.task_unknown"))?;
        if task.stopped {
            return Err(Failure::cancelled("action.task_stopped"));
        }
        if !self.config.flags.ai_enabled {
            return Err(Failure::blocked("ai.disabled"));
        }
        let level = self.gate.admit(request)?;
        let Some(wanted) = level_of_action_kind(&request.action_kind) else {
            return Err(Failure::unsupported(super::policy::CODE_ACTION_KIND_UNKNOWN));
        };
        if wanted > task.ceiling {
            return Err(Failure::blocked("action.above_task_ceiling"));
        }
        if wanted > self.config.max_level() {
            return Err(Failure::blocked("action.above_configured_max_level"));
        }
        let host = self
            .identity
            .url(task.document.document_token())
            .and_then(host_of)
            .unwrap_or_default();
        if wanted > ActionLevel::Observe && self.config.domains.denies_automation(&host) {
            return Err(Failure::blocked("action.domain_automation_denied"));
        }
        if request.deadline_ms_since_epoch <= now_ms {
            return Err(Failure::timed_out("action.deadline_passed"));
        }
        // Staleness: the action must be about the document this task started on,
        // and that document must still be what its tab shows.
        if request.expected_document != task.document.document_token() {
            return Err(Failure::stale("action.document_mismatch"));
        }
        self.identity
            .resolve_in_tab(request.expected_document, task.tab)
            .map_err(|f| f.with_detail("the task's page has been replaced"))?;
        let current = self.identity.snapshot_version(request.expected_document)?;
        if current != request.expected_snapshot_version {
            return Err(Failure::stale("action.snapshot_version_mismatch"));
        }
        Ok(level)
    }

    /// The URL of a link the task was shown. A link id that is not in the task's
    /// snapshot is refused, which is what keeps a model-invented URL from being
    /// opened.
    pub fn link_url(&self, task: u64, link_id: &str) -> Result<String, Failure> {
        let t = self.tasks.get(&task).ok_or_else(|| Failure::stale("action.task_unknown"))?;
        let ctx = self
            .contexts
            .get(t.document.document_token(), t.snapshot)
            .ok_or_else(|| Failure::stale("action.snapshot_evicted"))?;
        ctx.links
            .iter()
            .find(|l| l.link_id == link_id)
            .map(|l| l.url.clone())
            .ok_or_else(|| Failure::blocked("action.link_not_in_snapshot"))
    }
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

pub fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            format!("{scheme}://{authority}")
        }
        None => String::new(),
    }
}

/// The coverage claim for a capture, built only from facts the parser has.
pub fn coverage(ctx: &PageContext) -> ContextCoverage {
    let inaccessible: Vec<CoverageGap> = (0..ctx.frames_not_read)
        .map(|_| CoverageGap {
            kind: CoverageGapKind::InaccessibleFrame,
            reason: "a sub-frame was not read".to_string(),
        })
        .collect();
    let mut unsupported = Vec::new();
    if ctx.dropped_blocks > 0 {
        unsupported.push(CoverageGap {
            kind: CoverageGapKind::UnsupportedContent,
            reason: format!("{} passages over the per-page limit were dropped", ctx.dropped_blocks),
        });
    }
    let complete = inaccessible.is_empty() && unsupported.is_empty();
    let kept = ctx.blocks.len() as f32;
    let total = kept + ctx.dropped_blocks as f32;
    let share = if total > 0.0 { kept / total } else { 0.0 };
    ContextCoverage {
        status: if complete { CoverageStatus::CompleteEnough } else { CoverageStatus::Partial },
        captured_frames: 1,
        inaccessible_frames: inaccessible,
        unloaded_regions: Vec::new(),
        unsupported_content: unsupported,
        has_untranscribed_media: false,
        estimated_completeness: share / (1.0 + ctx.frames_not_read as f32),
    }
}

/// The body the utility receives: text and links, never geometry.
pub fn page_payload(ctx: &PageContext) -> Value {
    let payload = PagePayload {
        blocks: ctx
            .blocks
            .iter()
            .map(|b| PageBlock { block_id: b.block_id.clone(), text: b.text.clone() })
            .collect(),
        links: ctx
            .links
            .iter()
            .map(|l| PageLink { link_id: l.link_id.clone(), text: l.text.clone(), url: l.url.clone(), block_id: None })
            .collect(),
        frames_not_read: ctx.frames_not_read as u32,
        dropped_blocks: ctx.dropped_blocks as u32,
    };
    serde_json::to_value(payload).unwrap_or_else(|_| json!({}))
}

pub fn snapshot_of(key: DocumentKey, ctx: &PageContext, title: &str) -> ContextSnapshot {
    ContextSnapshot {
        document: key,
        snapshot_version: ctx.snapshot_version,
        captured_at_ms: now_ms(),
        url: ctx.url.clone(),
        origin: origin_of(&ctx.url),
        title: title.to_string(),
        coverage: coverage(ctx),
        payload: page_payload(ctx),
        limit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_probe_is_bounded_and_validated() {
        let ok = Selection::from_probe(r#"{"ai":"sel","text":"hello","rect":[1,2,3,4]}"#).unwrap();
        assert_eq!(ok.text, "hello");
        assert_eq!(ok.rect, Some([1.0, 2.0, 3.0, 4.0]));
        assert!(Selection::from_probe(r#"{"ai":"other","text":"x"}"#).is_none());
        assert!(Selection::from_probe("not json").is_none());
        let long = format!(r#"{{"ai":"sel","text":"{}"}}"#, "a".repeat(10_000));
        assert_eq!(Selection::from_probe(&long).unwrap().text.chars().count(), MAX_SELECTION_CHARS);
        let bad_rect = Selection::from_probe(r#"{"ai":"sel","text":"x","rect":[1,2,3]}"#).unwrap();
        assert_eq!(bad_rect.rect, None);
        let huge = Selection::from_probe(r#"{"ai":"sel","text":"x","rect":[1e12,0,0,0]}"#).unwrap();
        assert_eq!(huge.rect, None);
    }

    #[test]
    fn change_probe_accepts_only_known_reasons() {
        assert!(is_change_probe(r#"{"ai":"changed","reason":"input"}"#));
        assert!(!is_change_probe(r#"{"ai":"changed","reason":"anything else"}"#));
        assert!(!is_change_probe(r#"{"ai":"sel","text":"x"}"#));
        assert!(!is_change_probe(&format!(r#"{{"ai":"changed","reason":"dom","pad":"{}"}}"#, "a".repeat(2000))));
        assert_ne!(fingerprint(&["a", "b"]), fingerprint(&["ab", ""]));
        assert_eq!(fingerprint(&["same"]), fingerprint(&["same"]));
    }

    #[test]
    fn hosts_and_origins() {
        assert_eq!(host_of("https://User@Example.COM:8080/a?b"), Some("example.com".to_string()));
        assert_eq!(host_of("nope"), None);
        assert_eq!(origin_of("https://a.b/c?d"), "https://a.b");
    }

    #[test]
    fn disabled_config_refuses_before_any_work() {
        let mut session = AiSession::new();
        session.config = AiConfig::disabled();
        assert_eq!(session.availability().unwrap_err().code, "ai.disabled");
    }
}
