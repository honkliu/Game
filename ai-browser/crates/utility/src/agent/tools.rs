//! The agent's tool registry: what the model may call, how risky each tool is, and the
//! tools the utility runs itself (web, skills, stored results).

use std::sync::Mutex;

use serde_json::{json, Value};

use super::{tool_config, web, Env};
use crate::control::lock;

/// Results longer than this are stored; the model gets a preview and drills in.
const STASH_OVER: usize = 12_000;
const PREVIEW: usize = 3_000;
const READ_CAP: usize = 64 * 1024;
const GREP_LINES: usize = 50;
const MAX_STASHED: usize = 40;
const FETCH_TEXT: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Executed over the running shell's fixed MCP connection.
    Browser,
    /// Executed here.
    Local,
    /// Executed by an MCP server.
    Mcp { server: String, tool: String },
    Http(tool_config::HttpTool),
}

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub kind: Kind,
    /// Changes something (a page, a remote system): asks the user unless trusted.
    pub write: bool,
    pub trusted: bool,
}

impl ToolDef {
    pub fn schema(&self) -> Value {
        json!({ "type": "function", "function": {
            "name": self.name, "description": self.description, "parameters": self.parameters } })
    }

    pub fn server(&self) -> Value {
        match &self.kind {
            Kind::Mcp { server, .. } => json!(server),
            _ => Value::Null,
        }
    }
}

fn def(name: &str, description: &str, properties: Value, required: &[&str], kind: Kind, write: bool) -> ToolDef {
    ToolDef {
        id: name.into(),
        name: name.into(),
        description: description.into(),
        parameters: json!({ "type": "object", "properties": properties, "required": required }),
        kind,
        write,
        trusted: false,
    }
}

#[derive(Default)]
pub struct Stash {
    items: Vec<(String, String)>,
    counter: u32,
}

impl Stash {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn put(&mut self, text: String) -> String {
        self.counter += 1;
        let id = format!("r{}", self.counter);
        self.items.push((id.clone(), text));
        if self.items.len() > MAX_STASHED {
            self.items.remove(0);
        }
        id
    }

    fn get(&self, id: &str) -> Option<&str> {
        self.items.iter().find(|(i, _)| i == id).map(|(_, t)| t.as_str())
    }
}

/// Turns a tool's value into the text the model sees, stashing it when it is large.
pub fn envelope(stash: &Mutex<Stash>, value: Value) -> String {
    let text = match value {
        Value::String(s) => s,
        other => other.to_string(),
    };
    if text.len() <= STASH_OVER {
        return text;
    }
    let total = text.chars().count();
    let preview: String = text.chars().take(PREVIEW).collect();
    let id = lock(stash).put(text);
    json!({
        "stashed": true, "result_id": id, "total_chars": total, "preview": preview,
        "hint": "The full result is stored. If enabled in Tools settings, read_result pages through it and grep_result searches it; otherwise only this preview is available."
    })
    .to_string()
}

pub fn builtins() -> Vec<ToolDef> {
    let l = Kind::Local;
    let mut tools: Vec<ToolDef> = ai_platform_contract::browser::definitions().into_iter()
        .map(|api| ToolDef {
            id: api.name.clone(), name: api.name, description: api.description,
            parameters: api.parameters, kind: Kind::Browser, write: !api.read_only, trusted: false,
        }).collect();
    tools.push(def(
        "fetch_url",
        "Download a public web page or text/JSON file and return readable text. Cannot run scripts or log in.",
        json!({"url":{"type":"string"}}), &["url"], l.clone(), false,
    ));
    tools.push(def(
        "fetch_skill", "Load full skill instructions by id from the system prompt catalog.",
        json!({"skill_id":{"type":"string"}}), &["skill_id"], l.clone(), false,
    ));
    tools.push(def(
        "read_result", "Read part of a stored large tool result.",
        json!({"result_id":{"type":"string"},"offset":{"type":"integer"},"length":{"type":"integer","maximum":65536}}),
        &["result_id"], l.clone(), false,
    ));
    tools.push(def(
        "grep_result", "Search a stored tool result for text (case-insensitive; | separates alternatives). Returns up to 50 matching lines.",
        json!({"result_id":{"type":"string"},"pattern":{"type":"string"}}),
        &["result_id","pattern"], l, false,
    ));
    tools
}

pub fn registry(env: &Env, _stash: &Mutex<Stash>) -> Vec<ToolDef> {
    let Ok(config) = tool_config::load(env.store, env.mcp) else { return Vec::new() };
    tool_config::definitions(&config, env.mcp).into_iter()
        .filter(|tool| config.enabled.get(&tool.id).copied().unwrap_or(false))
        .collect()
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("`{key}` is required"))
}

pub fn run_local(env: &Env, name: &str, args: &Value, stash: &Mutex<Stash>, loaded: &Mutex<Vec<String>>) -> Result<Value, String> {
    match name {
        "fetch_url" => {
            let url = str_arg(args, "url")?;
            let page = web::fetch(url)?;
            let text = if page.content_type.contains("html") {
                let (title, text) = web::html_to_text(&page.body);
                format!("Title: {title}\nURL: {}\n\n{text}", page.final_url)
            } else {
                format!("URL: {}\nContent-Type: {}\n\n{}", page.final_url, page.content_type, page.body)
            };
            Ok(Value::String(text.chars().take(FETCH_TEXT).collect()))
        }
        "fetch_skill" => {
            let wanted = str_arg(args, "skill_id")?;
            let skill = env.store.skill(wanted).filter(|s| s.mode != "off").ok_or_else(|| {
                let ids: Vec<String> = env.store.skills().into_iter().filter(|s| s.mode != "off").map(|s| s.id).collect();
                format!("No skill `{wanted}`. Available: {}", ids.join(", "))
            })?;
            let mut loaded = lock(loaded);
            loaded.retain(|i| *i != skill.id);
            loaded.push(skill.id.clone());
            Ok(Value::String(format!("# Skill: {}\n{}", skill.name, skill.instructions)))
        }
        "read_result" => {
            let id = str_arg(args, "result_id")?;
            let stash = lock(stash);
            let text = stash.get(id).ok_or("no such result_id")?;
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let length = (args.get("length").and_then(Value::as_u64).unwrap_or(16_000) as usize).min(READ_CAP);
            let slice: String = text.chars().skip(offset).take(length).collect();
            Ok(json!({ "offset": offset, "returned": slice.chars().count(), "total_chars": text.chars().count(), "text": slice }))
        }
        "grep_result" => {
            let id = str_arg(args, "result_id")?;
            let pattern = str_arg(args, "pattern")?.to_lowercase();
            let alternatives: Vec<&str> = pattern.split('|').map(str::trim).filter(|p| !p.is_empty()).collect();
            let stash = lock(stash);
            let text = stash.get(id).ok_or("no such result_id")?;
            let lines: Vec<String> = text
                .lines()
                .enumerate()
                .filter(|(_, l)| {
                    let l = l.to_lowercase();
                    alternatives.iter().any(|a| l.contains(a))
                })
                .take(GREP_LINES)
                .map(|(n, l)| format!("{}: {}", n + 1, l.chars().take(400).collect::<String>()))
                .collect();
            Ok(Value::String(if lines.is_empty() { "No matches.".into() } else { lines.join("\n") }))
        }
        other => Err(format!("no local tool `{other}`")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn large_results_are_stashed_with_a_preview() {
        let stash = Mutex::new(Stash::default());
        let small = envelope(&stash, json!("hello"));
        assert_eq!(small, "hello");
        let big = envelope(&stash, Value::String("x".repeat(20_000)));
        let v: Value = serde_json::from_str(&big).unwrap();
        assert_eq!(v["stashed"], true);
        assert_eq!(v["total_chars"], 20_000);
        assert!(lock(&stash).get("r1").is_some());
    }
}
