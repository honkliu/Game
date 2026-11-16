//! The agent's tool registry: what the model may call, how risky each tool is, and the
//! tools the utility runs itself (web, skills, stored results).

use std::sync::Mutex;

use serde_json::{json, Value};

use super::{web, Env};
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
    /// Executed by the browser shell (`agent.tool`).
    Browser,
    /// Executed here.
    Local,
    /// Executed by an MCP server.
    Mcp { server: String, tool: String },
}

#[derive(Debug, Clone)]
pub struct ToolDef {
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
        "hint": "The full result is stored. Use read_result(result_id, offset, length) to page through it or grep_result(result_id, pattern) to search it."
    })
    .to_string()
}

pub fn registry(env: &Env, stash: &Mutex<Stash>) -> Vec<ToolDef> {
    let b = Kind::Browser;
    let l = Kind::Local;
    let tab = json!({ "type": "integer", "description": "Tab id from the browser context or list_tabs. Defaults to the active tab." });
    let mut tools = vec![
        def("list_tabs", "List the open browser tabs (id, title, url, active).", json!({}), &[], b.clone(), false),
        def(
            "read_page",
            "Read a tab: its title, URL, visible text, links and form fields/buttons (each with a CSS selector usable by click and fill). Read a page before answering questions about it.",
            json!({ "tab_id": tab }),
            &[],
            b.clone(),
            false,
        ),
        def(
            "open_tab",
            "Open a URL (http/https) in a new tab. By default it opens in the background so the user's view does not change. When a canvas is on screen, pass `row` and `col` (or `cell`) to load it into that cell instead.",
            json!({ "url": { "type": "string" }, "background": { "type": "boolean", "description": "Default true." }, "row": { "type": "integer" }, "col": { "type": "integer" }, "cell": { "type": "integer" } }),
            &["url"],
            b.clone(),
            false,
        ),
        def(
            "canvas",
            "Work with the canvas (a grid of up to 6 live pages in one tab). Actions: 'open' creates a canvas with `urls` (optionally cols x rows, default 3x2); 'set' loads `url` into `cell` (or the next empty cell); 'focus' makes `cell` the page the user works in; 'zoom' toggles one page full size; 'layout' reshapes to cols x rows; 'clear' closes the page in `cell`.",
            json!({
                "action": { "type": "string", "enum": ["open", "set", "focus", "zoom", "layout", "clear"] },
                "urls": { "type": "array", "items": { "type": "string" } },
                "url": { "type": "string" },
                "cell": { "type": "integer", "description": "0-based cell index, row by row." },
                "row": { "type": "integer", "description": "0-based row; use with col instead of cell." },
                "col": { "type": "integer", "description": "0-based column." },
                "cols": { "type": "integer" },
                "rows": { "type": "integer" }
            }),
            &["action"],
            b.clone(),
            false,
        ),
        def(
            "navigate",
            "Load a URL in an existing tab (the active tab by default). Replaces what that tab shows. When a canvas is on screen, pass `row` and `col` (or `cell`) to load it into that cell of the canvas.",
            json!({ "url": { "type": "string" }, "tab_id": tab, "row": { "type": "integer" }, "col": { "type": "integer" }, "cell": { "type": "integer" } }),
            &["url"],
            b.clone(),
            true,
        ),
        def("switch_tab", "Bring a tab to the front so the user sees it.", json!({ "tab_id": tab }), &["tab_id"], b.clone(), false),
        def("close_tab", "Close a tab you opened.", json!({ "tab_id": tab }), &["tab_id"], b.clone(), false),
        def(
            "click",
            "Click an element on a page, by CSS selector (from read_page) or by its visible text.",
            json!({ "selector": { "type": "string" }, "text": { "type": "string", "description": "Visible text of a link or button." }, "tab_id": tab }),
            &[],
            b.clone(),
            true,
        ),
        def(
            "dismiss_popups",
            "Close the popups, ads, cookie banners, modal dialogs and floating overlays covering a page (clicks their own close button, then hides what is left, and restores page scrolling). scope 'cell' (default) is the focused cell of the canvas, or the cell given by `cell` / `row`+`col`; scope 'tab' is every page in the current tab (all cells of a canvas); scope 'all' is every open page. Use it when the user asks to close popups or ads.",
            json!({
                "scope": { "type": "string", "enum": ["cell", "tab", "all"] },
                "cell": { "type": "integer", "description": "0-based cell index, row by row." },
                "row": { "type": "integer" },
                "col": { "type": "integer" },
                "tab_id": tab
            }),
            &[],
            b.clone(),
            false,
        ),
        def(
            "fill",
            "Type a value into an input, textarea or select identified by CSS selector. Does not submit.",
            json!({ "selector": { "type": "string" }, "value": { "type": "string" }, "tab_id": tab }),
            &["selector", "value"],
            b,
            true,
        ),
        def(
            "fetch_url",
            "Download a public web page or text/JSON file and return its readable text. Faster than opening a tab; cannot run scripts or log in.",
            json!({ "url": { "type": "string" } }),
            &["url"],
            l.clone(),
            false,
        ),
        def(
            "web_search",
            "Search the web. Returns titles, URLs and snippets.",
            json!({ "query": { "type": "string" }, "limit": { "type": "integer", "description": "1-10, default 6." } }),
            &["query"],
            l.clone(),
            false,
        ),
    ];
    if env.store.skills().iter().any(|s| s.mode == "auto") {
        tools.push(def(
            "fetch_skill",
            "Load the full instructions of a skill from the catalog in the system prompt, by id.",
            json!({ "skill_id": { "type": "string" } }),
            &["skill_id"],
            l.clone(),
            false,
        ));
    }
    if !lock(stash).is_empty() {
        tools.push(def(
            "read_result",
            "Read part of a stored large tool result.",
            json!({ "result_id": { "type": "string" }, "offset": { "type": "integer" }, "length": { "type": "integer", "description": "Max 65536." } }),
            &["result_id"],
            l.clone(),
            false,
        ));
        tools.push(def(
            "grep_result",
            "Search a stored tool result for text (case-insensitive; use | between alternatives). Returns up to 50 matching lines.",
            json!({ "result_id": { "type": "string" }, "pattern": { "type": "string" } }),
            &["result_id", "pattern"],
            l,
            false,
        ));
    }
    tools.extend(env.mcp.tools());
    tools
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
        "web_search" => {
            let query = str_arg(args, "query")?;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(6).clamp(1, 10) as usize;
            let rows = web::search(query, limit)?;
            if rows.is_empty() {
                return Ok(Value::String("No results.".into()));
            }
            Ok(Value::String(
                rows.iter()
                    .enumerate()
                    .map(|(i, (t, u, s))| format!("{}. {t}\n   {u}\n   {s}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
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
