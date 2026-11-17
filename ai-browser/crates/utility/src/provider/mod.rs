//! The model-provider seam: one trait, two implementations, and the catalog that
//! turns `ai.json` into routable models.
//!
//! Both providers speak the same *text* contract so the task layer has exactly
//! one parser and one citation verifier:
//!
//! ```text
//! ANSWER: <prose>
//! QUOTE b12: <text copied verbatim from passage b12>
//! ```
//!
//! (and, for [`Mode::Plan`], one JSON action object). Whatever a provider returns
//! is untrusted until [`crate::reply`] has checked it.

pub mod extractive;
pub mod openai;

use std::collections::HashMap;
use std::sync::Arc;

use ai_platform_contract::config::{AiConfig, ModelEntry, ProviderEntry, ProviderKind};
use ai_platform_contract::failure::Failure;
use ai_platform_contract::identity::TaskId;
use ai_platform_contract::model::{
    CostClass, Execution, LatencyClass, ModelManifest, Modality, PageLink, TaskIntent,
};
use serde::Serialize;

use crate::control::TaskControl;
use crate::router::RouteModel;

pub const EXTRACTIVE_PROVIDER_ID: &str = "extractive";
pub const EXTRACTIVE_MODEL_ID: &str = "extractive-v1";

/// Generous on purpose: a reasoning model spends part of the budget thinking
/// before it writes the first answer token (measured: 8 tokens → empty answer).
pub const DEFAULT_COMPLETION_TOKEN_BUDGET: u32 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Produce `ANSWER:` / `QUOTE` lines.
    Answer,
    /// Produce exactly one JSON action (the `act` loop).
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBlock {
    pub id: String,
    pub text: String,
}

/// What an earlier step of the act loop did and saw. `summary` may contain
/// page-provenance text (tab titles), so prompts wrap it as untrusted data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepObservation {
    pub action_kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// Everything a provider may use. Model-backed providers read `messages`; the
/// extractive provider reads the structured fields and ignores `messages`.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub task: TaskId,
    pub model_id: String,
    pub mode: Mode,
    pub intent: TaskIntent,
    pub question: String,
    pub selection: Option<String>,
    pub target_language: Option<String>,
    pub history: Vec<ai_platform_contract::model::ChatTurn>,
    pub blocks: Vec<PromptBlock>,
    pub links: Vec<PageLink>,
    pub steps: Vec<StepObservation>,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReply {
    /// The answer text only. Never reasoning text.
    pub text: String,
    pub finish_reason: Option<String>,
    /// A count the server reported; `None` when it said nothing.
    pub reasoning_tokens: Option<u32>,
    pub completion_token_budget: u32,
}

/// One agent-mode chat round: OpenAI-shaped `messages` and `tools`, as JSON.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model_id: String,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON text, as the model wrote it. Untrusted until parsed.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}

pub trait ModelProvider: Send + Sync {
    fn provider_id(&self) -> &str;

    /// Native tool-calling chat (agent mode). Providers without it refuse.
    fn chat(
        &self,
        _request: &ChatRequest,
        _control: &TaskControl,
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatReply, Failure> {
        Err(Failure::unsupported("provider.no_tool_calling"))
    }

    /// Streams answer text through `on_delta` and returns the full reply.
    /// Must return promptly with a `cancelled`/`timed_out`/`stale` failure
    /// (taken from `control`) once `control.is_stopped()`.
    fn complete(
        &self,
        request: &CompletionRequest,
        control: &TaskControl,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ProviderReply, Failure>;
}

/// One `model.list` row: the manifest plus whether it can be used right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelListEntry {
    pub manifest: ModelManifest,
    pub available: bool,
    /// Stable code (`provider_disabled`, `api_key_missing`, …); never a secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

pub struct Catalog {
    pub models: Vec<RouteModel>,
    providers: HashMap<String, Arc<dyn ModelProvider>>,
}

fn manifest_for(entry: &ProviderEntry, model: &ModelEntry) -> ModelManifest {
    let mut tags = entry.data_policy_tags.clone();
    if entry.execution == Execution::Local && !tags.iter().any(|t| t == "local-execution") {
        tags.push("local-execution".into());
    }
    ModelManifest {
        provider_id: entry.id.clone(),
        model_id: model.model_id.clone(),
        display_name: model.display_name.clone().unwrap_or_else(|| model.model_id.clone()),
        modalities: if model.modalities.is_empty() { vec![Modality::Text] } else { model.modalities.clone() },
        supports_tools: model.supports_tools,
        supports_structured_output: model.supports_tools,
        supports_streaming: true,
        supports_embeddings: false,
        execution: entry.execution,
        context_window: model.context_window,
        data_policy_tags: tags,
        cost_class: if entry.execution == Execution::Local { CostClass::Free } else { CostClass::Medium },
        latency_class: if entry.execution == Execution::Local { LatencyClass::Interactive } else { LatencyClass::Standard },
        aliases: model.aliases.clone(),
    }
}

fn extractive_model() -> ModelEntry {
    ModelEntry {
        model_id: EXTRACTIVE_MODEL_ID.into(),
        display_name: Some("Built-in extractive (offline)".into()),
        aliases: vec!["extractive".into()],
        context_window: 1_000_000,
        modalities: vec![Modality::Text],
        supports_tools: true,
    }
}

fn builtin_extractive_entry() -> ProviderEntry {
    ProviderEntry {
        id: EXTRACTIVE_PROVIDER_ID.into(),
        kind: ProviderKind::Extractive,
        enabled: true,
        execution: Execution::Local,
        base_url: None,
        api_key_env: None,
        models: vec![extractive_model()],
        data_policy_tags: vec!["no-network".into(), "no-retention".into()],
    }
}

impl Catalog {
    /// Builds the catalog from a freshly loaded config. The built-in extractive
    /// provider is always appended last, so a fresh install answers offline and
    /// it is the last resort in a fallback chain.
    pub fn from_config(config: &AiConfig) -> Catalog {
        Self::from_config_with_credentials(config, &HashMap::new())
    }

    pub fn from_config_with_credentials(config: &AiConfig, credentials: &HashMap<String, String>) -> Catalog {
        let mut models = Vec::new();
        let mut providers: HashMap<String, Arc<dyn ModelProvider>> = HashMap::new();
        let mut entries: Vec<ProviderEntry> = config.providers.clone();
        if !entries.iter().any(|entry| entry.id == EXTRACTIVE_PROVIDER_ID) {
            entries.push(builtin_extractive_entry());
        }
        for entry in &entries {
            let mut unavailable = if entry.enabled { None } else { Some("provider_disabled".to_string()) };
            if unavailable.is_none() {
                match build_provider(entry, credentials) {
                    Ok(provider) => {
                        providers.insert(entry.id.clone(), provider);
                    }
                    Err(reason) => unavailable = Some(reason),
                }
            }
            let model_entries: Vec<ModelEntry> = if entry.models.is_empty() && entry.kind == ProviderKind::Extractive {
                vec![extractive_model()]
            } else {
                entry.models.clone()
            };
            for model in &model_entries {
                models.push(RouteModel { manifest: manifest_for(entry, model), unavailable: unavailable.clone() });
            }
        }
        Catalog { models, providers }
    }

    pub fn provider(&self, provider_id: &str) -> Option<Arc<dyn ModelProvider>> {
        self.providers.get(provider_id).cloned()
    }

    pub fn list(&self) -> Vec<ModelListEntry> {
        self.models
            .iter()
            .map(|model| ModelListEntry {
                manifest: model.manifest.clone(),
                available: model.unavailable.is_none(),
                unavailable_reason: model.unavailable.clone(),
            })
            .collect()
    }
}

fn build_provider(entry: &ProviderEntry, credentials: &HashMap<String, String>) -> Result<Arc<dyn ModelProvider>, String> {
    match entry.kind {
        ProviderKind::Extractive => Ok(Arc::new(extractive::ExtractiveProvider::new(&entry.id))),
        ProviderKind::OpenAiCompatible => {
            let base_url = entry.base_url.as_deref().ok_or_else(|| "no_base_url".to_string())?;
            let key = match entry.api_key_env.as_deref() {
                None => None,
                Some(name) => match credentials.get(name).cloned().or_else(|| std::env::var(name).ok()) {
                    Some(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
                    _ => return Err("api_key_missing".to_string()),
                },
            };
            let provider = openai::OpenAiCompatibleProvider::new(&entry.id, base_url, key)
                .map_err(|reason| reason.to_string())?;
            Ok(Arc::new(provider))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_still_has_the_offline_provider() {
        let catalog = Catalog::from_config(&AiConfig::default());
        let list = catalog.list();
        assert_eq!(list.len(), 1);
        assert!(list[0].available);
        assert!(list[0].manifest.is_local());
        assert!(list[0].manifest.data_policy_tags.iter().any(|t| t == "local-execution"));
        assert!(catalog.provider(EXTRACTIVE_PROVIDER_ID).is_some());
    }

    #[test]
    fn missing_key_marks_the_model_unavailable_without_naming_a_secret() {
        let mut config = AiConfig::default();
        config.providers.push(ProviderEntry {
            id: "cloud".into(),
            kind: ProviderKind::OpenAiCompatible,
            enabled: true,
            execution: Execution::Cloud,
            base_url: Some("https://example.invalid/v1".into()),
            api_key_env: Some("AI_UTILITY_TEST_KEY_THAT_IS_NOT_SET".into()),
            models: vec![ModelEntry {
                model_id: "m".into(),
                display_name: None,
                aliases: vec![],
                context_window: 8192,
                modalities: vec![],
                supports_tools: true,
            }],
            data_policy_tags: vec![],
        });
        let catalog = Catalog::from_config(&config);
        let cloud = catalog.list().into_iter().find(|e| e.manifest.provider_id == "cloud").unwrap();
        assert!(!cloud.available);
        assert_eq!(cloud.unavailable_reason.as_deref(), Some("api_key_missing"));
        assert!(catalog.provider("cloud").is_none());
    }
}
