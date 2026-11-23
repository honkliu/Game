//! One task, start to finish: gates → snapshot → route → reduce → prompt →
//! provider chain → verification → exactly one `task.finish`.

use std::cell::Cell;
use std::path::Path;
use std::time::Instant;

use ai_platform_contract::config::AiConfig;
use ai_platform_contract::contract::{
    ActionLevel, AnswerCitation, AnswerNotProduced, AnswerWithCitations, CitationRole, ContextSnapshotRequest,
    OutputLimit, TaskOutcome, TaskProgress, TaskRequestView, TaskStage, AnnotationAnchor,
};
use ai_platform_contract::failure::{Failure, FailureKind};
use ai_platform_contract::frame::{Method, ResponsePayload};
use ai_platform_contract::identity::{SnapshotVersion, TaskId};
use ai_platform_contract::model::{ModelRef, PagePayload, TaskEvent, TaskFailed, TaskIntent};
use serde::Deserialize;

use crate::act::{self, ActContext, Dispatched, Parsed};
use crate::audit;
use crate::control::{now_ms, StopReason, TaskControl};
use crate::prompt::{self, PromptInput};
use crate::provider::{
    Catalog, CompletionRequest, Mode, ProviderReply, StepObservation, DEFAULT_COMPLETION_TOKEN_BUDGET,
};
use crate::reduce::{self, ReduceInput};
use crate::reply;
use crate::router::{self, RouteInput};
use crate::shell::ShellLink;
use crate::tabs::Tabs;

const MAX_PROMPT_LINKS: usize = 80;

pub struct TaskEnv<'a> {
    pub credentials: &'a std::collections::HashMap<String, String>,
    pub shell: &'a dyn ShellLink,
    pub tabs: &'a Tabs,
}

/// The effective deadline: the request's when set, else now + the configured
/// default; the earlier of the two when both exist.
pub fn effective_deadline(request_deadline_ms: i64, config: &AiConfig) -> i64 {
    let configured = if config.task_deadline_ms == 0 {
        0
    } else {
        now_ms().saturating_add(config.task_deadline_ms.min(i64::MAX as u64) as i64)
    };
    match (request_deadline_ms, configured) {
        (0, c) => c,
        (r, 0) => r,
        (r, c) => r.min(c),
    }
}

pub fn max_tokens_from_env() -> u32 {
    std::env::var("AI_PROVIDER_MAX_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map_or(DEFAULT_COMPLETION_TOKEN_BUDGET, |v| v.clamp(256, 32_768))
}

pub fn load_config(path: &Path) -> (AiConfig, Option<Failure>) {
    AiConfig::load(path)
}

#[derive(Deserialize)]
struct SnapshotView {
    document: ai_platform_contract::contract::DocumentClaim,
    snapshot_version: SnapshotVersion,
    #[serde(default)]
    url: String,
    payload: serde_json::Value,
}

struct Emitter<'a> {
    shell: &'a dyn ShellLink,
    task: TaskId,
    seq: Cell<u64>,
}

impl Emitter<'_> {
    fn event(&self, event: &TaskEvent) {
        if let Ok(params) = event.to_params() {
            self.shell.notify(Method::PublishProgress, &params);
        }
    }

    fn stage(&self, stage: TaskStage, completed_steps: u32, total_steps: Option<u32>) {
        self.event(&TaskEvent::Stage(TaskProgress { task: self.task, stage, completed_steps, total_steps }));
    }

    fn delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let seq = self.seq.get();
        self.seq.set(seq + 1);
        self.event(&TaskEvent::Delta { task: self.task, seq, text: text.to_string() });
    }
}

fn stop_check(control: &TaskControl) -> Result<(), Failure> {
    match control.failure() {
        Some(failure) => Err(failure),
        None => Ok(()),
    }
}

/// Runs the task and sends its single `task.finish`.
pub fn run(
    env: &TaskEnv<'_>,
    view: &TaskRequestView,
    control: &TaskControl,
    config: &AiConfig,
    config_failure: Option<Failure>,
) {
    let task = view.task;
    let started = Instant::now();
    audit::line("task.start", Some(task), &[("intent", &view.options.intent.as_str())]);
    let emitter = Emitter { shell: env.shell, task, seq: Cell::new(0) };
    let result = execute(env, view, control, config, config_failure, &emitter);
    let outcome = match (result, control.failure()) {
        (Ok(outcome), None) => outcome,
        (Err(failure), None) => TaskOutcome::Failed(TaskFailed { task, failure }),
        (Err(failure), Some(_)) => TaskOutcome::Failed(TaskFailed { task, failure }),
        // Stopped after the work finished: the stop wins, so a stale answer is never delivered.
        (Ok(_), Some(failure)) => TaskOutcome::Failed(TaskFailed { task, failure }),
    };
    let label = match &outcome {
        TaskOutcome::Answered(a) => format!("answered citations={}", a.citations.len()),
        TaskOutcome::NoAnswer(n) => format!("no_answer code={}", n.failure.code),
        TaskOutcome::Failed(f) => format!("failed kind={} code={}", f.failure.kind.as_str(), f.failure.code),
    };
    audit::line(
        "task.finish",
        Some(task),
        &[("outcome", &label), ("elapsed_ms", &started.elapsed().as_millis()), ("deltas", &emitter.seq.get())],
    );
    emitter.stage(TaskStage::Finished, control.actions_completed(), None);
    match outcome.to_params() {
        Ok(params) => env.shell.notify(Method::FinishTask, &params),
        Err(failure) => {
            let fallback = TaskOutcome::Failed(TaskFailed { task, failure });
            if let Ok(params) = fallback.to_params() {
                env.shell.notify(Method::FinishTask, &params);
            }
        }
    }
}

fn execute(
    env: &TaskEnv<'_>,
    view: &TaskRequestView,
    control: &TaskControl,
    config: &AiConfig,
    config_failure: Option<Failure>,
    em: &Emitter<'_>,
) -> Result<TaskOutcome, Failure> {
    let task = view.task;
    let intent = view.options.intent;
    if let Some(failure) = config_failure {
        return Err(failure);
    }
    if !config.flags.ai_enabled {
        return Err(Failure::blocked("config.ai_disabled"));
    }
    if !config.flags.page_context {
        return Err(Failure::blocked("config.page_context_disabled"));
    }
    if intent == TaskIntent::Act {
        if config.max_level() < ActionLevel::Navigate {
            return Err(Failure::blocked("config.actions_disabled"));
        }
        if view.action_ceiling < ActionLevel::Navigate {
            return Err(Failure::blocked("task.action_ceiling_observe"));
        }
    }
    if env.tabs.is_dead(view.document.document_token) {
        control.stop(StopReason::Stale("task.document_replaced"));
    }
    stop_check(control)?;

    em.stage(TaskStage::Planning, 0, None);
    let request = serde_json::to_value(ContextSnapshotRequest {
        document: view.document.document_token,
        workspace: view.document.workspace_id,
    })
    .map_err(|_| Failure::unsupported("contract.encode_failed"))?;
    let asked = Instant::now();
    let response = env.shell.call("shell_page_snapshot", &request, control)?;
    let snapshot: SnapshotView = match response {
        ResponsePayload::Failure(failure) => return Err(failure),
        ResponsePayload::Ok(value) => serde_json::from_value(value)
            .map_err(|_| Failure::unsupported("snapshot.invalid"))?,
    };
    stop_check(control)?;
    if snapshot.document.document_token != view.document.document_token
        || snapshot.document.navigation_sequence != view.document.navigation_sequence
    {
        control.stop(StopReason::Stale("task.snapshot_document_mismatch"));
        return Err(Failure::stale("task.snapshot_document_mismatch"));
    }
    let payload: PagePayload = serde_json::from_value(snapshot.payload)
        .map_err(|_| Failure::unsupported("snapshot.payload_invalid"))?;
    audit::line(
        "snapshot",
        Some(task),
        &[
            ("blocks", &payload.blocks.len()),
            ("links", &payload.links.len()),
            ("dropped", &payload.dropped_blocks),
            ("ms", &asked.elapsed().as_millis()),
        ],
    );

    let catalog = Catalog::from_config_with_credentials(config, env.credentials);
    let host = router::host_of(&snapshot.url);
    let routed = router::route(&RouteInput {
        task,
        intent,
        options: &view.options,
        config,
        host: host.as_deref(),
        models: &catalog.models,
    });
    em.event(&TaskEvent::Routed(routed.decision.clone()));
    if routed.chain.is_empty() {
        return Err(if routed.local_only_cause.is_some() {
            Failure::blocked("route.local_only_no_model")
        } else {
            Failure::unavailable("route.no_eligible_model")
        });
    }

    let max_tokens = max_tokens_from_env();
    let context_window = catalog
        .models
        .iter()
        .find(|m| {
            m.manifest.provider_id == routed.chain[0].provider_id && m.manifest.model_id == routed.chain[0].model_id
        })
        .map_or(8_192, |m| m.manifest.context_window);
    let selection = view.options.selection.as_ref().map(|s| s.exact.clone()).filter(|s| !s.trim().is_empty());
    let reduced = reduce::reduce(&ReduceInput {
        blocks: &payload.blocks,
        question: &view.question,
        selection: selection.as_deref(),
        intent,
        token_budget: reduce::prompt_token_budget(context_window, max_tokens),
    });
    audit::line(
        "reduce",
        Some(task),
        &[
            ("kept", &reduced.blocks.len()),
            ("total", &reduced.total_blocks),
            ("duplicates", &reduced.duplicates),
            ("chars", &reduced.chars),
        ],
    );

    let mut run = Run {
        env,
        view,
        control,
        config,
        catalog: &catalog,
        chain: &routed.chain,
        em,
        base: CompletionRequest {
            task,
            model_id: String::new(),
            mode: Mode::Answer,
            intent,
            question: view.question.clone(),
            selection,
            target_language: view.options.target_language.clone(),
            history: view.options.history.clone(),
            blocks: reduced.blocks,
            links: payload.links.iter().take(MAX_PROMPT_LINKS).cloned().collect(),
            steps: Vec::new(),
            messages: Vec::new(),
            max_tokens,
        },
        payload: &payload,
        snapshot_version: snapshot.snapshot_version,
        host,
    };
    if intent == TaskIntent::Act {
        run.act()
    } else {
        run.answer()
    }
}

struct Run<'a, 'e> {
    env: &'a TaskEnv<'a>,
    view: &'a TaskRequestView,
    control: &'a TaskControl,
    config: &'a AiConfig,
    catalog: &'a Catalog,
    chain: &'a [ModelRef],
    em: &'a Emitter<'e>,
    base: CompletionRequest,
    payload: &'a PagePayload,
    snapshot_version: SnapshotVersion,
    host: Option<String>,
}

impl Run<'_, '_> {
    fn prompt(&mut self, mode: Mode, allowed: &[&str], steps_left: u32) {
        self.base.mode = mode;
        self.base.messages = prompt::build(&PromptInput {
            task: self.view.task,
            mode,
            intent: self.base.intent,
            question: &self.base.question,
            selection: self.base.selection.as_deref(),
            target_language: self.base.target_language.as_deref(),
            history: &self.base.history,
            blocks: &self.base.blocks,
            links: &self.base.links,
            steps: &self.base.steps,
            allowed_actions: allowed,
            steps_left,
        });
    }

    /// Tries the chain in order. Falls back only while nothing has been streamed,
    /// so the user never sees two models' words spliced together.
    fn call_chain(&self, on_delta: &mut dyn FnMut(&str)) -> Result<ProviderReply, Failure> {
        let mut last = Failure::unavailable("route.no_provider");
        for model in self.chain {
            stop_check(self.control)?;
            let Some(provider) = self.catalog.provider(&model.provider_id) else { continue };
            let mut request = self.base.clone();
            request.model_id = model.model_id.clone();
            let streamed = Cell::new(false);
            let started = Instant::now();
            let result = provider.complete_with_thinking(&request, self.control, &mut |delta| {
                streamed.set(true);
                on_delta(delta);
            }, &mut |text| {
                streamed.set(true);
                self.em.event(&TaskEvent::Thinking { task: self.view.task, text: text.to_string() });
            });
            audit::line(
                "provider",
                Some(self.view.task),
                &[
                    ("provider", &model.provider_id),
                    ("ms", &started.elapsed().as_millis()),
                    ("ok", &result.is_ok()),
                ],
            );
            match result {
                Ok(reply) => return Ok(reply),
                Err(failure) => {
                    if streamed.get() || self.control.is_stopped() || failure.kind == FailureKind::NoAnswer {
                        return Err(failure);
                    }
                    last = failure;
                }
            }
        }
        Err(last)
    }

    fn answer(&mut self) -> Result<TaskOutcome, Failure> {
        let task = self.view.task;
        self.em.stage(TaskStage::Executing, 0, None);
        self.prompt(Mode::Answer, &[], 0);
        let mut raw = String::new();
        let mut emitted = String::new();
        let em = self.em;
        let result = {
            self.call_chain(&mut |delta| {
                raw.push_str(delta);
                let visible = reply::visible_answer(&raw);
                if let Some(fresh) = visible.get(emitted.len()..) {
                    if visible.starts_with(emitted.as_str()) && !fresh.is_empty() {
                        em.delta(fresh);
                        emitted.push_str(fresh);
                    }
                }
            })
        };
        let reply = match result {
            Ok(reply) => reply,
            Err(failure) if failure.kind == FailureKind::NoAnswer => {
                return Ok(TaskOutcome::NoAnswer(AnswerNotProduced {
                    task,
                    failure,
                    completion_token_budget: self.base.max_tokens,
                    reasoning_tokens: None,
                }));
            }
            Err(failure) => return Err(failure),
        };
        stop_check(self.control)?;
        let parsed = reply::parse(&reply.text);
        let answer = if parsed.had_markers { parsed.answer.trim().to_string() } else { reply.text.trim().to_string() };
        if answer.is_empty() {
            return Ok(TaskOutcome::NoAnswer(match (reply.finish_reason.as_deref(), reply.reasoning_tokens) {
                (Some("length"), Some(tokens)) if tokens > 0 => {
                    AnswerNotProduced::budget_consumed_by_reasoning(task, reply.completion_token_budget, tokens)
                }
                _ => AnswerNotProduced::unexplained(task, "model.answer_absent", reply.completion_token_budget),
            }));
        }
        if let Some(rest) = answer.strip_prefix(emitted.as_str()) {
            self.em.delta(rest);
        }

        self.em.stage(TaskStage::Verifying, 0, None);
        let (verified, rejected) = reply::verify_quotes(&parsed.quotes, &self.payload.blocks);
        let sequence = self.view.document.navigation_sequence;
        let citations: Vec<AnswerCitation> = verified
            .into_iter()
            .map(|quote| AnswerCitation {
                anchor: AnnotationAnchor {
                    document: self.view.document.document_token,
                    snapshot_version: self.snapshot_version,
                    block_id: Some(quote.block_id),
                    quote: quote.quote,
                    prefix: quote.prefix,
                    suffix: quote.suffix,
                    text_position: None,
                },
                navigation_sequence: sequence,
                role: quote.role.unwrap_or(CitationRole::DirectAnswer),
            })
            .collect();
        audit::line("verify", Some(task), &[("citations", &citations.len()), ("rejected", &rejected)]);
        let size = answer.len() as u64;
        let limit = (rejected > 0).then_some(OutputLimit { byte_limit: size, emitted_bytes: size, dropped_items: rejected as u64 });
        Ok(TaskOutcome::Answered(AnswerWithCitations { task, answer, citations, limit }))
    }

    fn act(&mut self) -> Result<TaskOutcome, Failure> {
        let task = self.view.task;
        let mut ceiling = self.view.action_ceiling.min(self.config.max_level());
        if self.host.as_deref().is_some_and(|h| self.config.domains.denies_automation(h)) {
            ceiling = ActionLevel::Observe;
        }
        let ctx = ActContext {
            task,
            document: self.view.document.document_token,
            snapshot_version: self.snapshot_version,
            links: &self.payload.links,
            blocks: &self.payload.blocks,
            tabs: self.env.tabs,
            ceiling,
            deadline_ms: self.control.deadline_ms(),
        };
        let allowed = ctx.allowed_actions();
        let mut steps: Vec<StepObservation> = Vec::new();
        let mut dispatched = 0u32;
        for step in 0..act::MAX_STEPS {
            stop_check(self.control)?;
            self.em.stage(TaskStage::Executing, dispatched, Some(act::MAX_STEPS));
            self.base.steps = steps.clone();
            self.prompt(Mode::Plan, &allowed, act::MAX_STEPS - step);
            let reply = self.call_chain(&mut |_| {})?;
            stop_check(self.control)?;
            match act::parse(&reply.text, &ctx) {
                Parsed::Finish(message) => {
                    let answer = if message.is_empty() { "Done.".to_string() } else { message };
                    return Ok(TaskOutcome::Answered(AnswerWithCitations { task, answer, citations: Vec::new(), limit: None }));
                }
                Parsed::Fail(_) => {
                    return Ok(TaskOutcome::NoAnswer(AnswerNotProduced::unexplained(
                        task,
                        "act.cannot_complete",
                        reply.completion_token_budget,
                    )));
                }
                Parsed::Unparseable => {
                    return Ok(TaskOutcome::NoAnswer(AnswerNotProduced::unexplained(
                        task,
                        "model.plan_unparseable",
                        reply.completion_token_budget,
                    )));
                }
                Parsed::Refused(note) => {
                    steps.push(StepObservation { action_kind: "refused".into(), summary: note });
                }
                Parsed::Action(planned) => {
                    dispatched += 1;
                    audit::line("action", Some(task), &[("kind", &planned.kind), ("n", &dispatched)]);
                    match act::dispatch(&ctx, self.env.shell, self.control, &planned, dispatched) {
                        Dispatched::Terminal(failure) => return Err(failure),
                        Dispatched::Observation(observation) => steps.push(observation),
                    }
                    self.em.stage(TaskStage::Verifying, dispatched, Some(act::MAX_STEPS));
                }
            }
        }
        Err(Failure::output_limited("act.step_limit"))
    }
}
