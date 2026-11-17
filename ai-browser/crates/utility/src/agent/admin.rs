//! `agent.control` management operations: settings, prompts, skills, MCP servers and
//! saved conversations. Everything the user can edit in the Agent panel lands here.

use serde_json::{json, Value};

use super::mcp::Hub;
use super::store::{slug, valid_id, McpServerConfig, Store};
use super::tool_config::{self, HttpTool, ToolOverride};
use crate::control::{now_ms, TaskControl};
use crate::provider::{Catalog, ChatRequest};

fn s<'a>(p: &'a Value, key: &str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or("")
}

fn strings(p: &Value, key: &str) -> Vec<String> {
    p.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default()
}

pub fn state(store: &Store, hub: &Hub) -> Value {
    let (tools, tools_error) = tool_config::catalog(store, hub);
    let config = store.config();
    json!({
        "config": {
            "auto_approve": config.auto_approve,
            "custom_instructions": config.custom_instructions,
            "max_rounds": config.max_rounds,
        },
        "skills": store.skills().iter().map(|k| k.to_json(false)).collect::<Vec<_>>(),
        "prompts": store.prompts(),
        "mcp": hub.status(&config.mcp_servers),
        "tools": tools,
        "tools_error": tools_error,
    })
}

/// Pulls the first JSON object out of free text (models wrap JSON in prose or fences).
fn first_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

fn distill(store: &Store, catalog: &Catalog, p: &Value) -> Result<Value, String> {
    let transcript: String = p
        .get("messages")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|m| format!("{}: {}", s(m, "role"), s(m, "content").chars().take(2500).collect::<String>()))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    if transcript.trim().is_empty() {
        return Err("there is no conversation to turn into a skill".into());
    }
    let model = catalog
        .models
        .iter()
        .find(|m| m.unavailable.is_none() && m.manifest.supports_tools && m.manifest.provider_id != crate::provider::EXTRACTIVE_PROVIDER_ID)
        .ok_or("no model available")?;
    let provider = catalog.provider(&model.manifest.provider_id).ok_or("provider unavailable")?;
    let prompt = format!(
        "Turn the conversation below into a reusable skill: instructions an assistant can follow the next time a similar task comes up. \
         Generalize (no one-off names or values). Reply with ONLY a JSON object: \
         {{\"name\": short title, \"description\": one sentence on when to use it, \"triggers\": [3-6 short phrases], \"instructions\": markdown steps}}.\n\n{transcript}"
    );
    let request = ChatRequest {
        model_id: model.manifest.model_id.clone(),
        messages: vec![json!({ "role": "user", "content": prompt })],
        tools: Vec::new(),
        max_tokens: 2_000,
        temperature: 0.2,
    };
    let control = TaskControl::new(now_ms() + 120_000);
    let reply = provider.chat(&request, &control, &mut |_| {}).map_err(|f| f.code)?;
    let draft = first_object(&reply.text).unwrap_or_else(|| json!({ "name": "New skill", "description": "", "triggers": [], "instructions": reply.text }));
    let name = s(&draft, "name").trim();
    let name = if name.is_empty() { "New skill" } else { name };
    let existing = store.skills();
    let mut id = slug(name);
    if id.is_empty() {
        id = "new-skill".into();
    }
    let base = id.clone();
    let mut n = 2;
    while existing.iter().any(|k| k.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    Ok(json!({
        "id": id, "name": name, "description": s(&draft, "description"),
        "triggers": strings(&draft, "triggers"), "mode": "auto", "instructions": s(&draft, "instructions"),
    }))
}

pub fn handle(store: &Store, hub: &Hub, catalog: &Catalog, p: &Value) -> Result<Value, String> {
    match s(p, "op") {
        "state" => Ok(state(store, hub)),
        "tools.enable" => {
            let id = s(p, "id");
            let enabled = p["enabled"].as_bool().ok_or("Enabled must be a boolean.")?;
            let mut tools = tool_config::load(store, hub)?;
            if !tool_config::catalog(store, hub).0.iter().any(|tool| tool["id"] == id) {
                return Err("Unknown tool.".into());
            }
            tools.enabled.insert(id.to_string(), enabled);
            let mut config = store.load_config()?;
            config.tools = Some(tools);
            store.save_config(&config)?;
            Ok(state(store, hub))
        }
        "tools.http.save" => {
            let tool: HttpTool = serde_json::from_value(p["tool"].clone()).map_err(|error| error.to_string())?;
            tool.validate()?;
            let mut tools = tool_config::load(store, hub)?;
            let existing = tools.http.iter().position(|old| old.id == tool.id);
            if existing.is_some() && p["editing"] != true {
                return Err("That tool ID already exists. Edit the existing tool instead.".into());
            }
            tools.enabled.insert(tool.id.clone(), p["enabled"].as_bool().unwrap_or(false));
            if let Some(index) = existing { tools.http[index] = tool; }
            else { tools.http.push(tool); }
            tool_config::validate_names(&tools, hub)?;
            let mut config = store.load_config()?;
            config.tools = Some(tools);
            store.save_config(&config)?;
            Ok(state(store, hub))
        }
        "tools.definition.save" | "tools.definition.reset" => {
            let id = s(p, "id");
            let mut tools = tool_config::load(store, hub)?;
            let known = tool_config::catalog(store, hub).0.into_iter().any(|tool| tool["id"] == id && tool["source"] != "http");
            if !known { return Err("Unknown non-HTTP tool.".into()); }
            if s(p, "op") == "tools.definition.reset" {
                tools.overrides.remove(id);
            } else {
                let edit: ToolOverride = serde_json::from_value(p["definition"].clone()).map_err(|error| error.to_string())?;
                edit.validate()?;
                tools.overrides.insert(id.to_string(), edit);
            }
            tool_config::validate_names(&tools, hub)?;
            let mut config = store.load_config()?;
            config.tools = Some(tools);
            store.save_config(&config)?;
            Ok(state(store, hub))
        }
        "tools.http.delete" => {
            let mut tools = tool_config::load(store, hub)?;
            let id = s(p, "id");
            tools.http.retain(|tool| tool.id != id);
            tools.enabled.remove(id);
            let mut config = store.load_config()?;
            config.tools = Some(tools);
            store.save_config(&config)?;
            Ok(state(store, hub))
        }
        "config.set" => {
            let mut c = store.config();
            if let Some(v) = p.get("auto_approve").and_then(Value::as_bool) {
                c.auto_approve = v;
            }
            if let Some(v) = p.get("custom_instructions").and_then(Value::as_str) {
                c.custom_instructions = v.chars().take(8_000).collect();
            }
            if let Some(v) = p.get("max_rounds").and_then(Value::as_u64) {
                c.max_rounds = v.clamp(1, 60) as u32;
            }
            store.save_config(&c)?;
            Ok(state(store, hub))
        }
        "prompt.save" => {
            store.save_prompt(s(p, "name"), s(p, "text"))?;
            Ok(state(store, hub))
        }
        "prompt.reset" => {
            store.reset_prompt(s(p, "name"))?;
            Ok(state(store, hub))
        }
        "skill.get" => store.skill(s(p, "id")).map(|k| k.to_json(true)).ok_or_else(|| "no such skill".into()),
        "skill.save" => {
            let name = s(p, "name");
            let id = match s(p, "id") {
                "" => slug(name),
                given => given.to_string(),
            };
            if !valid_id(&id) {
                return Err("give the skill a name made of letters or digits".into());
            }
            let skill = store.save_skill(&id, name, s(p, "description"), &strings(p, "triggers"), s(p, "mode"), s(p, "instructions"))?;
            // A saved skill's mode is its own; drop any stale override.
            let mut c = store.config();
            if c.skill_modes.remove(&skill.id).is_some() {
                store.save_config(&c)?;
            }
            Ok(state(store, hub))
        }
        "skill.delete" => {
            store.delete_skill(s(p, "id"))?;
            Ok(state(store, hub))
        }
        "skill.mode" => {
            let mode = s(p, "mode");
            if !super::store::valid_mode(mode) || store.skill(s(p, "id")).is_none() {
                return Err("bad skill or mode".into());
            }
            let mut c = store.config();
            c.skill_modes.insert(s(p, "id").to_string(), mode.to_string());
            store.save_config(&c)?;
            Ok(state(store, hub))
        }
        "skill.distill" => distill(store, catalog, p),
        // What the model will be told right now, rendered from the editable prompts.
        "context.preview" => Ok(json!({
            "context": super::browser_context(store, p),
            "system": super::system_prompt(store, p, &strings(p, "loaded_skills")),
        })),
        "mcp.save" => {
            let name = s(p, "name").trim();
            if name.is_empty() {
                return Err("give the server a name".into());
            }
            let id = match s(p, "id") {
                "" => slug(name),
                given => given.to_string(),
            };
            if !valid_id(&id) {
                return Err("bad server id".into());
            }
            let mut c = store.config();
            let old = c.mcp_servers.iter().find(|m| m.id == id).cloned();
            let url = Some(s(p, "url").trim().to_string()).filter(|u| !u.is_empty());
            let command = Some(s(p, "command").trim().to_string()).filter(|u| !u.is_empty());
            if url.is_none() == command.is_none() {
                return Err("set either a URL or a command, not both".into());
            }
            let token = match p.get("token").and_then(Value::as_str) {
                Some(t) if !t.is_empty() => Some(t.to_string()),
                Some(_) => None,
                None => old.as_ref().and_then(|o| o.token.clone()),
            };
            let entry = McpServerConfig {
                id: id.clone(),
                name: name.to_string(),
                url,
                token,
                command,
                args: strings(p, "args"),
                enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                trust: p.get("trust").and_then(Value::as_bool).unwrap_or(false),
            };
            match c.mcp_servers.iter().position(|m| m.id == id) {
                Some(i) => c.mcp_servers[i] = entry,
                None => c.mcp_servers.push(entry),
            }
            store.save_config(&c)?;
            hub.sync(&c.mcp_servers);
            let mut selection = tool_config::load(store, hub)?;
            for tool in hub.tools() {
                if matches!(&tool.kind, super::tools::Kind::Mcp { server, .. } if server == &id) {
                    selection.enabled.entry(tool.id).or_insert(true);
                }
            }
            c.tools = Some(selection);
            store.save_config(&c)?;
            Ok(state(store, hub))
        }
        "mcp.delete" => {
            let mut c = store.config();
            c.mcp_servers.retain(|m| m.id != s(p, "id"));
            store.save_config(&c)?;
            hub.sync(&c.mcp_servers);
            Ok(state(store, hub))
        }
        "mcp.sync" => {
            hub.sync(&store.config().mcp_servers);
            Ok(state(store, hub))
        }
        "mcp.retry" => {
            hub.sync(&store.config().mcp_servers);
            hub.reconnect(s(p, "id"));
            Ok(state(store, hub))
        }
        "conv.save" => {
            store.save_conversation(s(p, "id"), p.get("doc").unwrap_or(&Value::Null))?;
            Ok(json!({ "saved": true }))
        }
        "conv.list" => Ok(json!({ "conversations": store.list_conversations(s(p, "query")) })),
        "conv.load" => store.load_conversation(s(p, "id")),
        "conv.delete" => {
            store.delete_conversation(s(p, "id"))?;
            Ok(json!({ "deleted": true }))
        }
        other => Err(format!("unknown agent operation `{other}`")),
    }
}
