use std::collections::HashSet;

use ai_platform_contract::config::{AiConfig, ModelEntry, ProviderEntry, ProviderKind};
use ai_platform_contract::model::Execution;
use serde_json::Value;

pub(super) fn key_env(id: &str) -> String {
    if id == "custom" {
        return "BROWSER_AI_KEY".into();
    }
    let encoded: String = id.bytes().map(|b| format!("{b:02X}")).collect();
    format!("BROWSER_AI_KEY_{encoded}")
}

pub(super) fn prepare(config: &AiConfig, p: &Value, saved_key: bool) -> Result<ProviderEntry, String> {
    let id = match p.get("id").and_then(Value::as_str) {
        None => "custom".to_string(),
        Some("") => (1..).map(|n| format!("provider-{n}"))
            .find(|id| !config.providers.iter().any(|entry| &entry.id == id)).unwrap(),
        Some(id) => id.to_string(),
    };
    if id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') || id == "extractive" {
        return Err("connection ID must use 1-64 letters, numbers, hyphens or underscores".into());
    }
    let previous = config.providers.iter().find(|entry| entry.id == id);
    if p.get("creating").and_then(Value::as_bool) == Some(true) && previous.is_some() {
        return Err("a connection with this ID already exists".into());
    }
    if previous.is_some_and(|entry| entry.kind != ProviderKind::OpenAiCompatible) {
        return Err("this connection is not an OpenAI-compatible endpoint".into());
    }
    let input_url = p.get("base_url").and_then(Value::as_str).unwrap_or("").trim().trim_end_matches('/');
    let base_url = input_url.strip_suffix("/chat/completions").unwrap_or(input_url);
    let endpoint = url::Url::parse(base_url).map_err(|_| "invalid endpoint")?;
    if base_url.len() > 300 || !matches!(endpoint.scheme(), "http" | "https")
        || endpoint.host_str().is_none() || !endpoint.username().is_empty() || endpoint.password().is_some()
        || endpoint.query().is_some() || endpoint.fragment().is_some()
    {
        return Err("endpoint must be an http(s) URL with a host and no credentials, query or fragment".into());
    }
    let models = if let Some(models) = p.get("models") {
        serde_json::from_value::<Vec<ModelEntry>>(models.clone()).map_err(|e| format!("invalid models: {e}"))?
    } else {
        let model_id = p.get("model_id").and_then(Value::as_str).unwrap_or("").trim().to_string();
        vec![ModelEntry {
            display_name: Some(model_id.clone()), model_id, aliases: vec![], context_window: 32_768,
            modalities: vec![], supports_tools: true,
        }]
    };
    if models.is_empty() || models.len() > 100 {
        return Err("configure between 1 and 100 models for this connection".into());
    }
    let mut ids = HashSet::new();
    for model in &models {
        if model.model_id.trim().is_empty() || model.model_id.len() > 120 || model.model_id != model.model_id.trim()
            || model.model_id.chars().any(char::is_control) || model.context_window == 0
        {
            return Err("each model needs a nonempty ID (up to 120 characters) and a positive context window".into());
        }
        if !ids.insert(&model.model_id) {
            return Err(format!("duplicate model ID: {}", model.model_id));
        }
    }
    let host = endpoint.host_str().unwrap_or_default();
    let local = host == "localhost" || host.starts_with("127.") || host == "::1" || host == "[::1]";
    let env = key_env(&id);
    let inherited = previous.and_then(|entry| entry.api_key_env.as_ref())
        .filter(|name| std::env::var(name).is_ok()).cloned();
    Ok(ProviderEntry {
        id,
        kind: ProviderKind::OpenAiCompatible,
        enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(previous.is_none_or(|entry| entry.enabled)),
        execution: if local { Execution::Local } else { Execution::Cloud },
        base_url: Some(base_url.to_string()),
        api_key_env: if saved_key || std::env::var(&env).is_ok() { Some(env) }
            else if inherited.is_some() { inherited }
            else { None },
        models,
        data_policy_tags: previous.map(|entry| entry.data_policy_tags.clone()).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adds_multiple_models_without_replacing_other_connections() {
        let mut config = AiConfig::default();
        let first = prepare(&config, &json!({"base_url":"http://localhost:8001/v1","model_id":"original"}), false).unwrap();
        config.providers.push(first.clone());
        let added = prepare(&config, &json!({
            "id":"", "base_url":"https://models.example/v1",
            "models":[{"model_id":"org/model-a","supports_tools":true},{"model_id":"model-b","context_window":65536}]
        }), true).unwrap();
        assert_eq!(added.id, "provider-1");
        assert_eq!(added.models.len(), 2);
        assert!(added.models[0].supports_tools);
        assert!(!added.models[1].supports_tools);
        assert_ne!(added.api_key_env, first.api_key_env);
        assert_eq!(config.providers, vec![first]);
    }

    #[test]
    fn edits_preserve_connection_metadata_and_disabled_state() {
        let mut config = AiConfig::default();
        let mut entry = prepare(&config, &json!({"id":"local","base_url":"http://localhost/v1","model_id":"old"}), false).unwrap();
        entry.enabled = false;
        entry.data_policy_tags.push("private".into());
        config.providers.push(entry);
        assert!(prepare(&config, &json!({
            "id":"local","creating":true,"base_url":"http://localhost/v1","model_id":"replacement"
        }), false).is_err());
        let edited = prepare(&config, &json!({"id":"local","base_url":"http://localhost/v1","model_id":"new"}), false).unwrap();
        assert!(!edited.enabled);
        assert_eq!(edited.data_policy_tags, vec!["private"]);
        assert_eq!(edited.models[0].model_id, "new");
    }

    #[test]
    fn rejects_invalid_or_duplicate_model_definitions() {
        let config = AiConfig::default();
        for p in [
            json!({"id":"bad/id","base_url":"https://host/v1","model_id":"model"}),
            json!({"base_url":"https://user:secret@host/v1","model_id":"model"}),
            json!({"base_url":"https://host/v1","models":[]}),
            json!({"base_url":"https://host/v1","models":[{"model_id":"same"},{"model_id":"same"}]}),
            json!({"base_url":"https://host/v1","models":[{"model_id":"x","context_window":0}]}),
        ] {
            assert!(prepare(&config, &p, false).is_err());
        }
        assert_ne!(key_env("a-b"), key_env("a_b"));
        assert_ne!(key_env("a"), key_env("A"));
    }

    #[test]
    fn sample_remote_model_needs_no_key_or_route_configuration() {
        let config = AiConfig::default();
        let model = prepare(&config, &json!({
            "id":"", "base_url":"https://sample.invalid/v1/chat/completions/",
            "model_id":"sample-model"
        }), false).unwrap();
        assert_eq!(model.base_url.as_deref(), Some("https://sample.invalid/v1"));
        assert_eq!(model.api_key_env, None);
        assert!(model.models[0].supports_tools);
        assert_eq!(config, AiConfig::default());
    }
}
