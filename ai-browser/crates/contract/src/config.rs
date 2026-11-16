//! `ai.json`: the one configuration file both processes read (QAL-03, MOD-03/04).
//!
//! Re-read on every task, not cached at start-up, which is what makes a kill switch
//! work "without requiring a new binary": flip a flag, the next task obeys it.
//! Everything fails *closed* — a missing file yields [`AiConfig::default`], which
//! has cloud models off and no network provider configured, and an unreadable or
//! malformed file disables AI rather than guessing.
//!
//! Credentials are never in this file. A provider names the *environment variable*
//! its key is read from ([`ProviderEntry::api_key_env`]); the key itself is only
//! ever in the utility's process environment.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::failure::Failure;
use crate::model::{Execution, Modality};

/// Independently disableable capabilities. Every external provider class and every
/// action level has its own switch (QAL-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureFlags {
    /// Master switch. Off: no AI surface, no capture, no tasks.
    pub ai_enabled: bool,
    pub local_models_enabled: bool,
    /// Off by default: a fresh install sends nothing off-machine.
    pub cloud_models_enabled: bool,
    pub selection_toolbar: bool,
    pub page_context: bool,
    pub multi_tab_context: bool,
    /// Highest action level the AI may be granted, by name:
    /// `observe` | `navigate` | `interact` | `commit`.
    pub max_action_level: String,
    pub annotations: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        FeatureFlags {
            ai_enabled: true,
            local_models_enabled: true,
            cloud_models_enabled: false,
            selection_toolbar: true,
            page_context: true,
            multi_tab_context: true,
            max_action_level: "navigate".into(),
            annotations: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Built in. Answers from the page's own passages with no model and no network.
    Extractive,
    /// Any server speaking the OpenAI chat-completions protocol (hosted APIs,
    /// llama.cpp, Ollama, vLLM, LM Studio).
    OpenAiCompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub model_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    #[serde(default)]
    pub modalities: Vec<Modality>,
    #[serde(default)]
    pub supports_tools: bool,
}

fn default_context_window() -> u32 {
    32_768
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub id: String,
    pub kind: ProviderKind,
    #[serde(default)]
    pub enabled: bool,
    pub execution: Execution,
    /// `http(s)://host:port/v1`. Required for `OpenAiCompatible`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Name of the environment variable holding the API key. Never the key.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    #[serde(default)]
    pub data_policy_tags: Vec<String>,
}

/// One routing rule (§6.2): which models to prefer for a class of work and which
/// properties are mandatory.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteRule {
    /// Aliases or `provider/model` ids, most preferred first.
    pub prefer: Vec<String>,
    /// Every tag must be present on the model (`local-execution`, …).
    pub require_tags: Vec<String>,
    pub require_modalities: Vec<Modality>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DomainPolicy {
    /// Hosts (and their subdomains) whose pages never go to a non-local model.
    pub cloud_denied: Vec<String>,
    /// Hosts on which the AI may not act at all, only observe.
    pub automation_denied: Vec<String>,
}

impl DomainPolicy {
    fn matches(list: &[String], host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        list.iter().any(|entry| {
            let entry = entry.trim().trim_start_matches("*.").to_ascii_lowercase();
            !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
        })
    }

    pub fn denies_cloud(&self, host: &str) -> bool {
        Self::matches(&self.cloud_denied, host)
    }

    pub fn denies_automation(&self, host: &str) -> bool {
        Self::matches(&self.automation_denied, host)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub schema: u32,
    pub flags: FeatureFlags,
    pub providers: Vec<ProviderEntry>,
    pub routes: BTreeMap<String, RouteRule>,
    pub domains: DomainPolicy,
    /// Models tried in order when the chosen one fails, after routing.
    pub fallback_order: Vec<String>,
    /// Per-task wall clock, ms.
    pub task_deadline_ms: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        let mut routes = BTreeMap::new();
        routes.insert(
            "selection_explanation".into(),
            RouteRule { prefer: vec!["local-fast".into(), "cloud-fast".into()], ..Default::default() },
        );
        routes.insert(
            "page_summary".into(),
            RouteRule { prefer: vec!["local-long-context".into(), "cloud-standard".into()], ..Default::default() },
        );
        routes.insert(
            "cross_tab_research".into(),
            RouteRule { prefer: vec!["cloud-reasoning".into(), "cloud-standard".into()], ..Default::default() },
        );
        routes.insert(
            "private_page".into(),
            RouteRule { require_tags: vec!["local-execution".into()], ..Default::default() },
        );
        routes.insert(
            "visual_fallback".into(),
            RouteRule { require_modalities: vec![Modality::Image], ..Default::default() },
        );
        AiConfig {
            schema: 1,
            flags: FeatureFlags::default(),
            providers: Vec::new(),
            routes,
            domains: DomainPolicy::default(),
            fallback_order: Vec::new(),
            task_deadline_ms: 60_000,
        }
    }
}

impl AiConfig {
    /// A config that turns everything off. Used when the file exists but cannot be
    /// trusted: fail to disabled, never to a permissive default.
    pub fn disabled() -> AiConfig {
        let mut config = AiConfig::default();
        config.flags.ai_enabled = false;
        config.flags.cloud_models_enabled = false;
        config.flags.local_models_enabled = false;
        config.flags.max_action_level = "observe".into();
        config
    }

    /// Missing file → defaults. Present but unparseable → [`AiConfig::disabled`]
    /// plus the reason, so the shell can say *why* AI is off.
    pub fn load(path: &Path) -> (AiConfig, Option<Failure>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<AiConfig>(&text) {
                Ok(config) => (config, None),
                Err(error) => (
                    AiConfig::disabled(),
                    Some(Failure::unavailable("config.invalid").with_detail(error.to_string())),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (AiConfig::default(), None),
            Err(error) => (
                AiConfig::disabled(),
                Some(Failure::unavailable("config.unreadable").with_detail(error.to_string())),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// The highest action level the flags allow.
    pub fn max_level(&self) -> crate::contract::ActionLevel {
        use crate::contract::ActionLevel::*;
        match self.flags.max_action_level.as_str() {
            "commit" => Commit,
            "interact" => Interact,
            "navigate" => Navigate,
            _ => Observe,
        }
    }
}
