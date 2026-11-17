//! Agent state kept on disk next to `ai.json`: settings, skills and saved conversations.
//!
//! Everything here is plain files so a user can read, edit, back up or share them:
//!
//! ```text
//! agent.json                    settings (approvals, instructions, MCP servers, skill modes)
//! skills/<id>/SKILL.md          one skill: `---` frontmatter, then the instructions
//! conversations/<id>.json       one saved conversation
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_SKILL_BYTES: usize = 64 * 1024;
const MAX_CONVERSATION_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    /// Streamable-HTTP endpoint. Mutually exclusive with `command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Bearer token for the endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Local server started over stdio: executable and arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Tools run without asking even though they are not marked read-only.
    #[serde(default)]
    pub trust: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Run tools that change things (click, type, MCP writes) without asking.
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default)]
    pub custom_instructions: String,
    #[serde(default = "default_rounds")]
    pub max_rounds: u32,
    /// Skill id → `always-on` | `auto` | `off`; overrides the skill's own default.
    #[serde(default)]
    pub skill_modes: HashMap<String, String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub tools: Option<super::tool_config::ToolConfig>,
}

fn default_rounds() -> u32 {
    25
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            auto_approve: false,
            custom_instructions: String::new(),
            max_rounds: default_rounds(),
            skill_modes: HashMap::new(),
            mcp_servers: Vec::new(),
            tools: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    /// `always-on`, `auto` or `off` (after the user's override).
    pub mode: String,
    pub builtin: bool,
    pub instructions: String,
}

impl Skill {
    pub fn to_json(&self, with_body: bool) -> Value {
        let mut v = json!({
            "id": self.id, "name": self.name, "description": self.description,
            "triggers": self.triggers, "mode": self.mode, "builtin": self.builtin,
        });
        if with_body {
            v["instructions"] = Value::String(self.instructions.clone());
        }
        v
    }
}

/// Editable prompts: name, what the placeholders are, built-in text.
const PROMPTS: [(&str, &str, &str); 3] = [
    (
        "system",
        "The agent's system prompt. Placeholders: {{date}}, {{browser_context}}, {{skills}}, {{loaded_skills}}, {{instructions}}.",
        include_str!("prompts/system.md"),
    ),
    (
        "context",
        "How the browser state is shown to the model. Placeholders: {{active_tab}}, {{tabs}}, {{selection}}, {{page_content}} (the readable text of the active page). If {{page_content}} is missing from your text, the page content is appended anyway.",
        include_str!("prompts/context.md"),
    ),
    (
        "compaction",
        "Instruction for summarizing old turns when the context fills up. Placeholder: {{transcript}}.",
        include_str!("prompts/compaction.md"),
    ),
];

const BUILTIN: [(&str, &str); 4] = [
    ("research", include_str!("skills/research.md")),
    ("summarize-page", include_str!("skills/summarize-page.md")),
    ("compare-tabs", include_str!("skills/compare-tabs.md")),
    ("fill-form", include_str!("skills/fill-form.md")),
];

/// `a-z`, `0-9`, `-`, `_`; 1..=48. Used for every id that becomes a path component.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(48).collect()
}

fn parse_skill(id: &str, text: &str, builtin: bool) -> Option<Skill> {
    let rest = text.strip_prefix("---")?;
    let (front, body) = rest.split_once("\n---")?;
    let (mut name, mut description, mut triggers, mut activation) = (id.to_string(), String::new(), Vec::new(), None);
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "name" => name = value,
            "description" => description = value,
            "triggers" => {
                triggers = value.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
            }
            "activation" => activation = Some(value),
            _ => {}
        }
    }
    Some(Skill {
        id: id.to_string(),
        name,
        description,
        triggers,
        mode: activation.filter(|m| valid_mode(m)).unwrap_or_else(|| "auto".into()),
        builtin,
        instructions: body.trim().to_string(),
    })
}

pub fn valid_mode(mode: &str) -> bool {
    matches!(mode, "always-on" | "auto" | "off")
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Store {
    pub fn new(dir: &Path) -> Store {
        Store { dir: dir.to_path_buf() }
    }

    // ---- settings -------------------------------------------------------

    fn config_path(&self) -> PathBuf {
        self.dir.join("agent.json")
    }

    pub fn config(&self) -> AgentConfig {
        self.load_config().unwrap_or_default()
    }

    pub fn load_config(&self) -> Result<AgentConfig, String> {
        match fs::read_to_string(self.config_path()) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AgentConfig::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save_config(&self, config: &AgentConfig) -> Result<(), String> {
        let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        fs::write(self.config_path(), text).map_err(|e| e.to_string())
    }

    // ---- prompts --------------------------------------------------------

    fn prompts_dir(&self) -> PathBuf {
        self.dir.join("prompts")
    }

    /// The built-in text of an editable prompt, if `name` is one.
    pub fn default_prompt(name: &str) -> Option<&'static str> {
        PROMPTS.iter().find(|(n, _, _)| *n == name).map(|(_, _, text)| *text)
    }

    /// The prompt in force: the user's file when present and non-empty, else the built-in.
    pub fn prompt(&self, name: &str) -> String {
        let user = fs::read_to_string(self.prompts_dir().join(format!("{name}.md"))).ok().filter(|t| !t.trim().is_empty());
        user.or_else(|| Self::default_prompt(name).map(str::to_string)).unwrap_or_default()
    }

    pub fn save_prompt(&self, name: &str, text: &str) -> Result<(), String> {
        if Self::default_prompt(name).is_none() {
            return Err("unknown prompt".into());
        }
        if text.len() > MAX_SKILL_BYTES {
            return Err("prompt is too long".into());
        }
        fs::create_dir_all(self.prompts_dir()).map_err(|e| e.to_string())?;
        fs::write(self.prompts_dir().join(format!("{name}.md")), text).map_err(|e| e.to_string())
    }

    pub fn reset_prompt(&self, name: &str) -> Result<(), String> {
        if Self::default_prompt(name).is_none() {
            return Err("unknown prompt".into());
        }
        let path = self.prompts_dir().join(format!("{name}.md"));
        if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// `{name, title, help, text, default, customized}` for every editable prompt.
    pub fn prompts(&self) -> Vec<Value> {
        PROMPTS
            .iter()
            .map(|(name, help, default)| {
                let text = self.prompt(name);
                json!({ "name": name, "help": help, "text": text, "default": default, "customized": text != *default })
            })
            .collect()
    }

    // ---- skills ---------------------------------------------------------

    fn skills_dir(&self) -> PathBuf {
        self.dir.join("skills")
    }

    /// Built-in skills, overridden by same-id user skills, plus the user's own; modes come
    /// from `agent.json` when the user has changed them.
    pub fn skills(&self) -> Vec<Skill> {
        let config = self.config();
        let mut all: Vec<Skill> = Vec::new();
        for (id, text) in BUILTIN {
            if let Some(skill) = parse_skill(id, text, true) {
                all.push(skill);
            }
        }
        if let Ok(entries) = fs::read_dir(self.skills_dir()) {
            for entry in entries.flatten() {
                let id = entry.file_name().to_string_lossy().to_string();
                if !valid_id(&id) {
                    continue;
                }
                let Ok(text) = fs::read_to_string(entry.path().join("SKILL.md")) else { continue };
                if text.len() > MAX_SKILL_BYTES {
                    continue;
                }
                if let Some(skill) = parse_skill(&id, &text, false) {
                    match all.iter().position(|s| s.id == id) {
                        Some(i) => all[i] = skill,
                        None => all.push(skill),
                    }
                }
            }
        }
        for skill in &mut all {
            if let Some(mode) = config.skill_modes.get(&skill.id).filter(|m| valid_mode(m)) {
                skill.mode = mode.clone();
            }
        }
        all
    }

    /// Finds a skill by id, or by a slug of its name.
    pub fn skill(&self, id_or_name: &str) -> Option<Skill> {
        let wanted = id_or_name.trim().to_lowercase();
        let wanted_slug = slug(&wanted);
        self.skills().into_iter().find(|s| s.id == wanted || s.id == wanted_slug || slug(&s.name) == wanted_slug)
    }

    pub fn save_skill(
        &self,
        id: &str,
        name: &str,
        description: &str,
        triggers: &[String],
        mode: &str,
        instructions: &str,
    ) -> Result<Skill, String> {
        if !valid_id(id) {
            return Err("skill id must be lowercase letters, digits, - or _".into());
        }
        if name.trim().is_empty() || instructions.trim().is_empty() {
            return Err("a skill needs a name and instructions".into());
        }
        if instructions.len() > MAX_SKILL_BYTES {
            return Err("instructions are too long".into());
        }
        let mode = if valid_mode(mode) { mode } else { "auto" };
        let text = format!(
            "---\nname: {}\ndescription: {}\ntriggers: {}\nactivation: {}\n---\n{}\n",
            one_line(name),
            one_line(description),
            triggers.iter().map(|t| one_line(t)).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", "),
            mode,
            instructions.trim()
        );
        let dir = self.skills_dir().join(id);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join("SKILL.md"), text).map_err(|e| e.to_string())?;
        self.skill(id).ok_or_else(|| "skill was not saved".into())
    }

    pub fn delete_skill(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("bad skill id".into());
        }
        let dir = self.skills_dir().join(id);
        if !dir.exists() {
            return Err("only your own skills can be deleted (turn a built-in one off instead)".into());
        }
        fs::remove_dir_all(dir).map_err(|e| e.to_string())
    }

    // ---- conversations --------------------------------------------------

    fn conv_dir(&self) -> PathBuf {
        self.dir.join("conversations")
    }

    pub fn save_conversation(&self, id: &str, doc: &Value) -> Result<(), String> {
        if !valid_id(id) {
            return Err("bad conversation id".into());
        }
        let text = serde_json::to_string(doc).map_err(|e| e.to_string())?;
        if text.len() > MAX_CONVERSATION_BYTES {
            return Err("conversation is too large to save".into());
        }
        fs::create_dir_all(self.conv_dir()).map_err(|e| e.to_string())?;
        fs::write(self.conv_dir().join(format!("{id}.json")), text).map_err(|e| e.to_string())
    }

    pub fn load_conversation(&self, id: &str) -> Result<Value, String> {
        if !valid_id(id) {
            return Err("bad conversation id".into());
        }
        let text = fs::read_to_string(self.conv_dir().join(format!("{id}.json"))).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn delete_conversation(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("bad conversation id".into());
        }
        fs::remove_file(self.conv_dir().join(format!("{id}.json"))).map_err(|e| e.to_string())
    }

    /// Newest first: `{id, title, updated, count}` per conversation, optionally filtered by text.
    pub fn list_conversations(&self, query: &str) -> Vec<Value> {
        let needle = query.trim().to_lowercase();
        let mut rows: Vec<(u64, Value)> = Vec::new();
        let Ok(entries) = fs::read_dir(self.conv_dir()) else { return Vec::new() };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            let Ok(text) = fs::read_to_string(&path) else { continue };
            if !needle.is_empty() && !text.to_lowercase().contains(&needle) {
                continue;
            }
            let Ok(doc) = serde_json::from_str::<Value>(&text) else { continue };
            let updated = doc.get("updated").and_then(Value::as_u64).unwrap_or(0);
            let count = doc.get("messages").and_then(Value::as_array).map_or(0, Vec::len);
            let title = doc.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string();
            rows.push((updated, json!({ "id": id, "title": title, "updated": updated, "count": count })));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        rows.into_iter().map(|(_, v)| v).take(200).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agent-store-{tag}-{}-{}", std::process::id(), crate::control::now_ms()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builtin_skills_parse_and_a_user_skill_round_trips() {
        let dir = temp("skills");
        let store = Store::new(&dir);
        assert!(store.skills().iter().all(|s| s.builtin && !s.instructions.is_empty() && !s.description.is_empty()));
        store.save_skill("my-skill", "My skill", "does x", &["x".into(), "y".into()], "always-on", "Step 1.").unwrap();
        let skill = store.skill("My Skill").unwrap();
        assert_eq!((skill.mode.as_str(), skill.triggers.len(), skill.builtin), ("always-on", 2, false));
        let mut config = store.config();
        config.skill_modes.insert("my-skill".into(), "off".into());
        store.save_config(&config).unwrap();
        assert_eq!(store.skill("my-skill").unwrap().mode, "off");
        assert!(store.save_skill("../evil", "n", "d", &[], "auto", "b").is_err());
        store.delete_skill("my-skill").unwrap();
        assert!(store.skill("my-skill").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn conversations_save_list_search_delete() {
        let dir = temp("conv");
        let store = Store::new(&dir);
        store.save_conversation("c1", &json!({"title": "Alpha", "updated": 1, "messages": [{"role":"user","content":"needle"}]})).unwrap();
        store.save_conversation("c2", &json!({"title": "Beta", "updated": 2, "messages": []})).unwrap();
        assert_eq!(store.list_conversations("")[0]["id"], "c2");
        assert_eq!(store.list_conversations("NEEDLE").len(), 1);
        store.delete_conversation("c1").unwrap();
        assert!(store.load_conversation("c1").is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
