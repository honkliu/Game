//! The `act` tool loop: parse one model-proposed action, check it against the
//! snapshot and the ceiling, dispatch it to the shell, and turn the shell's
//! verdict into an observation (or a terminal failure).
//!
//! The model never supplies a URL or a target of its own: `link_id` / `block_id`
//! must exist in the snapshot, `tab_id` must be a tab the browser has told us
//! about, and the URL that goes on the wire is the snapshot link's.

use ai_platform_contract::contract::{
    level_of_action_kind, ActionLevel, ActionRequest, ActionTarget, Idempotency, TargetResolution,
};
use ai_platform_contract::failure::{Failure, FailureKind};
use ai_platform_contract::frame::ResponsePayload;
use ai_platform_contract::identity::{DocumentToken, SnapshotVersion, TaskId};
use ai_platform_contract::model::{PageBlock, PageLink};
use serde_json::{json, Value};

use crate::control::TaskControl;
use crate::provider::StepObservation;
use crate::shell::ShellLink;
use crate::tabs::Tabs;
use crate::text::clip_chars;

pub const MAX_STEPS: u32 = 6;
const OBSERVATION_CHARS: usize = 600;

pub const ALL_ACTIONS: [&str; 8] = [
    "open_tab",
    "navigate",
    "activate_tab",
    "read_tab",
    "list_tabs",
    "scroll_to",
    "highlight_passage",
    "show_candidate",
];

pub struct ActContext<'a> {
    pub task: TaskId,
    pub document: DocumentToken,
    pub snapshot_version: SnapshotVersion,
    pub links: &'a [PageLink],
    pub blocks: &'a [PageBlock],
    pub tabs: &'a Tabs,
    /// min(request ceiling, config cap, domain policy).
    pub ceiling: ActionLevel,
    pub deadline_ms: i64,
}

impl ActContext<'_> {
    fn permits(&self, kind: &str) -> bool {
        level_of_action_kind(kind).is_some_and(|level| level <= self.ceiling && level.allowed_in_release_zero())
    }

    pub fn allowed_actions(&self) -> Vec<&'static str> {
        ALL_ACTIONS.iter().copied().filter(|kind| self.permits(kind)).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub kind: String,
    pub target: Option<Value>,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Action(Planned),
    Finish(String),
    Fail(String),
    /// A well-formed proposal we will not send; the text goes back to the model.
    Refused(String),
    Unparseable,
}

/// The first balanced `{...}` in `text`, tolerant of prose and code fences around it.
fn first_json_object(text: &str) -> Option<Value> {
    let bytes = text.as_bytes();
    let mut start = 0;
    while let Some(offset) = text[start..].find('{') {
        let begin = start + offset;
        let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
        for (i, &b) in bytes.iter().enumerate().skip(begin) {
            if in_string {
                match (escaped, b) {
                    (true, _) => escaped = false,
                    (false, b'\\') => escaped = true,
                    (false, b'"') => in_string = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(value) = serde_json::from_str::<Value>(&text[begin..=i]) {
                            return Some(value);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
        start = begin + 1;
    }
    None
}

fn id_arg<'v>(object: &'v Value, key: &str) -> Option<&'v str> {
    object.get(key).and_then(Value::as_str)
}

fn tab_arg(object: &Value) -> Option<u64> {
    let value = object.get("tab_id")?;
    value.as_u64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn parse(text: &str, ctx: &ActContext<'_>) -> Parsed {
    let Some(object) = first_json_object(text) else { return Parsed::Unparseable };
    let Some(kind) = id_arg(&object, "action").or_else(|| id_arg(&object, "type")) else {
        return Parsed::Unparseable;
    };
    match kind {
        "finish" => {
            return Parsed::Finish(id_arg(&object, "message").unwrap_or("").trim().to_string());
        }
        "fail" => return Parsed::Fail(id_arg(&object, "reason").unwrap_or("").trim().to_string()),
        _ => {}
    }
    if !ALL_ACTIONS.contains(&kind) {
        return Parsed::Refused(format!("error: unknown action '{}'", clip_chars(kind, 40)));
    }
    if !ctx.permits(kind) {
        return Parsed::Refused(format!("error: action '{kind}' is above the permitted level"));
    }
    let link = |id: &str| ctx.links.iter().find(|l| l.link_id == id);
    let block_known = |id: &str| ctx.blocks.iter().any(|b| b.block_id == id);
    let planned = match kind {
        "open_tab" | "navigate" => {
            let Some(found) = id_arg(&object, "link_id").and_then(link) else {
                return Parsed::Refused("error: link_id is not in the page links".into());
            };
            Planned {
                kind: kind.into(),
                target: Some(json!({ "link_id": found.link_id })),
                arguments: json!({ "url": found.url }),
            }
        }
        "scroll_to" | "highlight_passage" => {
            let Some(block) = id_arg(&object, "block_id").filter(|id| block_known(id)) else {
                return Parsed::Refused("error: block_id is not in the page passages".into());
            };
            Planned { kind: kind.into(), target: Some(json!({ "block_id": block })), arguments: json!({}) }
        }
        "show_candidate" => {
            if let Some(found) = id_arg(&object, "link_id").and_then(link) {
                Planned {
                    kind: kind.into(),
                    target: Some(json!({ "link_id": found.link_id })),
                    arguments: json!({ "url": found.url }),
                }
            } else if let Some(block) = id_arg(&object, "block_id").filter(|id| block_known(id)) {
                Planned { kind: kind.into(), target: Some(json!({ "block_id": block })), arguments: json!({}) }
            } else {
                return Parsed::Refused("error: show_candidate needs a link_id or block_id from the page".into());
            }
        }
        "activate_tab" | "read_tab" => {
            let Some(tab) = tab_arg(&object).filter(|tab| ctx.tabs.document_of(*tab).is_some()) else {
                return Parsed::Refused("error: tab_id is not a known tab".into());
            };
            Planned { kind: kind.into(), target: Some(json!({ "tab_id": tab })), arguments: json!({}) }
        }
        _ => Planned { kind: kind.into(), target: None, arguments: json!({}) },
    };
    Parsed::Action(planned)
}

#[derive(Debug)]
pub enum Dispatched {
    Observation(StepObservation),
    /// The task must end with this failure (denied, blocked, stale, stopped).
    Terminal(Failure),
}

fn idempotency(kind: &str) -> Idempotency {
    if kind == "open_tab" {
        Idempotency::Conditional
    } else {
        Idempotency::Safe
    }
}

pub fn dispatch(
    ctx: &ActContext<'_>,
    shell: &dyn ShellLink,
    control: &TaskControl,
    planned: &Planned,
    ordinal: u32,
) -> Dispatched {
    let request = ActionRequest {
        action_id: format!("t{}-a{}", ctx.task.0, ordinal),
        task: ctx.task,
        expected_document: ctx.document,
        expected_snapshot_version: ctx.snapshot_version,
        action_kind: planned.kind.clone(),
        target: planned.target.clone().map(|opaque_target| ActionTarget {
            resolution: TargetResolution::BrowserSemanticCommand,
            opaque_target,
        }),
        arguments: planned.arguments.clone(),
        deadline_ms_since_epoch: ctx.deadline_ms,
        idempotency: idempotency(&planned.kind),
    };
    let metadata = match serde_json::to_value(&request) {
        Ok(params) => params,
        Err(_) => return Dispatched::Terminal(Failure::unsupported("contract.encode_failed")),
    };
    let mut params = planned.arguments.clone();
    if let Some(target) = &planned.target {
        if let Some(target) = target.as_object() {
            for (key, value) in target { params[key] = value.clone(); }
        }
    }
    let name = match planned.kind.as_str() {
        "open_tab" | "show_candidate" => { params["operation"] = json!("open"); "shell_tab" }
        "activate_tab" => { params["operation"] = json!("activate"); "shell_tab" }
        "list_tabs" => { params["operation"] = json!("list"); "shell_tab" }
        "read_tab" => "shell_page_read",
        "navigate" => "shell_page_navigate",
        "scroll_to" | "highlight_passage" => "shell_page_scroll_to",
        _ => return Dispatched::Terminal(Failure::unsupported("action.unknown")),
    };
    params["_task"] = metadata;
    control.action_started();
    let response = shell.call(name, &params, control);
    let observation = |summary: String| {
        Dispatched::Observation(StepObservation { action_kind: planned.kind.clone(), summary })
    };
    match response {
        Err(failure) => {
            // Stopped while in flight: the effect is unknown, so it is not counted as completed.
            control.action_finished(false);
            Dispatched::Terminal(failure)
        }
        Ok(ResponsePayload::Failure(failure)) => {
            control.action_finished(false);
            match failure.kind {
                FailureKind::Stale | FailureKind::Blocked | FailureKind::Cancelled | FailureKind::TimedOut => {
                    Dispatched::Terminal(failure)
                }
                _ => observation(format!("error: {}", failure.code)),
            }
        }
        Ok(ResponsePayload::Ok(result)) => {
            let decision = result.get("decision");
            let flag = |key: &str| decision.and_then(|d| d.get(key)).and_then(Value::as_bool);
            if let Some(by) = decision.and_then(|d| d.get("blocked_by")).and_then(Value::as_str) {
                control.action_finished(false);
                return Dispatched::Terminal(Failure::blocked("action.blocked_by_policy").with_detail(by.to_string()));
            }
            if flag("approval_required") == Some(true) && flag("approval_granted") != Some(true) {
                control.action_finished(false);
                return Dispatched::Terminal(Failure::blocked("action.approval_denied"));
            }
            control.action_finished(true);
            let verified = result.get("verified").and_then(Value::as_bool).unwrap_or(false);
            let payload = result.get("result_payload").filter(|p| !p.is_null()).map(Value::to_string);
            let mut summary = if verified { "ok".to_string() } else { "ok, effect not verified".to_string() };
            if let Some(payload) = payload {
                summary.push_str(": ");
                summary.push_str(clip_chars(&payload, OBSERVATION_CHARS));
            }
            observation(summary)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ai_platform_contract::model::BrowserEvent;

    fn fixtures() -> (Vec<PageLink>, Vec<PageBlock>, Tabs) {
        let links = vec![PageLink {
            link_id: "l0".into(),
            text: "Pricing".into(),
            url: "https://shop.example/pricing".into(),
            block_id: None,
        }];
        let blocks = vec![PageBlock { block_id: "b0".into(), text: "hello".into() }];
        let tabs = Tabs::default();
        tabs.apply(&BrowserEvent::NavigationCommitted {
            tab_id: 3,
            document: DocumentToken(30),
            navigation_sequence: 1,
            replaced: None,
        });
        (links, blocks, tabs)
    }

    fn ctx<'a>(links: &'a [PageLink], blocks: &'a [PageBlock], tabs: &'a Tabs, ceiling: ActionLevel) -> ActContext<'a> {
        ActContext {
            task: TaskId(1),
            document: DocumentToken(10),
            snapshot_version: SnapshotVersion(1),
            links,
            blocks,
            tabs,
            ceiling,
            deadline_ms: 0,
        }
    }

    #[test]
    fn only_snapshot_ids_are_accepted_and_urls_come_from_the_snapshot() {
        let (links, blocks, tabs) = fixtures();
        let c = ctx(&links, &blocks, &tabs, ActionLevel::Navigate);
        match parse("```json\n{\"action\":\"open_tab\",\"link_id\":\"l0\",\"url\":\"https://evil.example\"}\n```", &c) {
            Parsed::Action(p) => assert_eq!(p.arguments, json!({"url": "https://shop.example/pricing"})),
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse("{\"action\":\"open_tab\",\"link_id\":\"l9\"}", &c), Parsed::Refused(_)));
        assert!(matches!(parse("{\"action\":\"open_tab\",\"url\":\"https://x.example\"}", &c), Parsed::Refused(_)));
        assert!(matches!(parse("{\"action\":\"scroll_to\",\"block_id\":\"b7\"}", &c), Parsed::Refused(_)));
        assert!(matches!(parse("{\"action\":\"read_tab\",\"tab_id\":99}", &c), Parsed::Refused(_)));
        assert!(matches!(parse("{\"action\":\"read_tab\",\"tab_id\":3}", &c), Parsed::Action(_)));
        assert!(matches!(parse("{\"action\":\"purchase\"}", &c), Parsed::Refused(_)));
        assert_eq!(parse("sure thing", &c), Parsed::Unparseable);
        assert_eq!(parse("{\"action\":\"finish\",\"message\":\"done\"}", &c), Parsed::Finish("done".into()));
    }

    #[test]
    fn ceiling_filters_actions() {
        let (links, blocks, tabs) = fixtures();
        let observe = ctx(&links, &blocks, &tabs, ActionLevel::Observe);
        assert_eq!(observe.allowed_actions(), vec!["read_tab", "list_tabs"]);
        assert!(matches!(parse("{\"action\":\"open_tab\",\"link_id\":\"l0\"}", &observe), Parsed::Refused(_)));
        let interact = ctx(&links, &blocks, &tabs, ActionLevel::Interact);
        assert!(interact.allowed_actions().contains(&"open_tab"));
        assert!(matches!(parse("{\"action\":\"type_text\"}", &interact), Parsed::Refused(_)));
    }
}
