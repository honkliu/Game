//! Model routing with an inspectable decision (MOD-03).
//!
//! Hard constraints are applied first and can only *remove* candidates:
//! `LocalOnly`, `force_local`, a `cloud_denied` page domain and the cloud kill
//! switch each exclude every non-local model. Preference (aliases, `prefer`,
//! `fallback_order`) only orders what is left, so no preference can widen what a
//! constraint narrowed. Every candidate is reported with a stable reason code.

use ai_platform_contract::config::{AiConfig, RouteRule};
use ai_platform_contract::identity::TaskId;
use ai_platform_contract::model::{
    ModelManifest, ModelRef, RouteCandidate, RoutePreference, RoutingDecision, TaskIntent, TaskOptions,
};

/// A catalog model as the router sees it.
#[derive(Debug, Clone)]
pub struct RouteModel {
    pub manifest: ModelManifest,
    /// Why this model cannot be called right now, if it cannot.
    pub unavailable: Option<String>,
}

pub struct RouteInput<'a> {
    pub task: TaskId,
    pub intent: TaskIntent,
    pub options: &'a TaskOptions,
    pub config: &'a AiConfig,
    /// Host of the page the task is about, when it could be determined.
    pub host: Option<&'a str>,
    pub models: &'a [RouteModel],
}

#[derive(Debug, Clone)]
pub struct Routed {
    pub decision: RoutingDecision,
    /// `chosen` followed by `fallbacks`; what the task layer walks.
    pub chain: Vec<ModelRef>,
    /// Why only local models were eligible, if that constraint applied.
    pub local_only_cause: Option<&'static str>,
}

/// `http(s)://user@host:port/path` → `host`, lowercased. No crate for four fields.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn name_matches(manifest: &ModelManifest, name: &str) -> bool {
    manifest.aliases.iter().any(|alias| alias == name)
        || name == format!("{}/{}", manifest.provider_id, manifest.model_id)
        || name == manifest.model_id
}

fn first_match(manifest: &ModelManifest, names: &[String]) -> Option<usize> {
    names.iter().position(|name| name_matches(manifest, name))
}

fn base_class(intent: TaskIntent, has_selection: bool) -> &'static str {
    match intent {
        TaskIntent::Act => "act",
        TaskIntent::Summarize => "page_summary",
        TaskIntent::Explain | TaskIntent::Verify | TaskIntent::Translate | TaskIntent::Rewrite => "selection_explanation",
        TaskIntent::Ask if has_selection => "selection_explanation",
        TaskIntent::Ask => "page_summary",
    }
}

pub fn route(input: &RouteInput<'_>) -> Routed {
    let config = input.config;
    let options = input.options;

    let local_only_cause = if options.force_local {
        Some("excluded.force_local")
    } else if input.host.is_some_and(|host| config.domains.denies_cloud(host)) {
        Some("excluded.domain_cloud_denied")
    } else if options.route == RoutePreference::LocalOnly {
        Some("excluded.local_only_requested")
    } else {
        None
    };

    let class = base_class(input.intent, options.selection.is_some());
    let mut rule: RouteRule = config.routes.get(class).cloned().unwrap_or_default();
    let route_class = if local_only_cause.is_some() {
        if let Some(private) = config.routes.get("private_page") {
            for tag in &private.require_tags {
                if !rule.require_tags.contains(tag) {
                    rule.require_tags.push(tag.clone());
                }
            }
            for modality in &private.require_modalities {
                if !rule.require_modalities.contains(modality) {
                    rule.require_modalities.push(*modality);
                }
            }
        }
        "private_page"
    } else {
        class
    };

    let (requested, allow_fallback) = match &options.route {
        RoutePreference::Model { provider_id, model_id, allow_fallback } => {
            (Some((provider_id.as_str(), model_id.as_str())), *allow_fallback)
        }
        _ => (None, true),
    };

    struct Eligible<'m> {
        model: &'m RouteModel,
        rank: (u8, usize, usize),
        why: &'static str,
    }
    let mut considered = Vec::with_capacity(input.models.len());
    let mut eligible: Vec<Eligible<'_>> = Vec::new();

    for (position, model) in input.models.iter().enumerate() {
        let manifest = &model.manifest;
        let is_requested = requested.is_some_and(|(p, m)| manifest.provider_id == p && manifest.model_id == m);
        let exclusion: Option<&'static str> = if let Some(reason) = &model.unavailable {
            Some(if reason == "provider_disabled" { "excluded.provider_disabled" } else { "excluded.provider_unavailable" })
        } else if requested.is_some() && !allow_fallback && !is_requested {
            Some("excluded.not_requested")
        } else if manifest.is_local() {
            (!config.flags.local_models_enabled).then_some("excluded.local_models_disabled")
        } else if let Some(cause) = local_only_cause {
            Some(cause)
        } else if !config.flags.cloud_models_enabled {
            Some("excluded.cloud_disabled")
        } else {
            None
        };
        let exclusion = exclusion
            .or_else(|| {
                rule.require_tags
                    .iter()
                    .any(|tag| !manifest.data_policy_tags.contains(tag))
                    .then_some("excluded.missing_tag")
            })
            .or_else(|| {
                rule.require_modalities
                    .iter()
                    .any(|modality| !manifest.modalities.contains(modality))
                    .then_some("excluded.missing_modality")
            })
            .or_else(|| {
                (input.intent == TaskIntent::Act && !manifest.supports_tools).then_some("excluded.no_tool_support")
            });
        if let Some(reason) = exclusion {
            considered.push(RouteCandidate { model: manifest.reference(), accepted: false, reason: reason.into() });
            continue;
        }
        let (rank, why) = if is_requested {
            ((0, 0, position), "chosen.explicit")
        } else if let Some(index) = first_match(manifest, &rule.prefer) {
            ((1, index, position), "chosen.preferred")
        } else if let Some(index) = first_match(manifest, &config.fallback_order) {
            ((2, index, position), "chosen.fallback_order")
        } else {
            ((3, 0, position), "chosen.default_order")
        };
        eligible.push(Eligible { model, rank, why });
    }

    eligible.sort_by_key(|entry| entry.rank);
    for (index, entry) in eligible.iter().enumerate() {
        considered.push(RouteCandidate {
            model: entry.model.manifest.reference(),
            accepted: true,
            reason: if index == 0 { entry.why.into() } else { "eligible.fallback".into() },
        });
    }

    let mut chain: Vec<ModelRef> = eligible.iter().map(|entry| entry.model.manifest.reference()).collect();
    if requested.is_some() && !allow_fallback {
        chain.truncate(1);
    }
    let chosen = chain.first().cloned();
    let local_execution = eligible.first().is_some_and(|entry| entry.model.manifest.is_local());
    let decision = RoutingDecision {
        task: input.task,
        route_class: route_class.into(),
        chosen: chosen.clone(),
        fallbacks: chain.iter().skip(1).cloned().collect(),
        considered,
        local_execution: chosen.is_some() && local_execution,
    };
    Routed { decision, chain, local_only_cause }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ai_platform_contract::model::{CostClass, Execution, LatencyClass, Modality};

    fn model(provider: &str, id: &str, execution: Execution, aliases: &[&str]) -> RouteModel {
        let local = execution == Execution::Local;
        RouteModel {
            manifest: ModelManifest {
                provider_id: provider.into(),
                model_id: id.into(),
                display_name: id.into(),
                modalities: vec![Modality::Text],
                supports_tools: true,
                supports_structured_output: true,
                supports_streaming: true,
                supports_embeddings: false,
                execution,
                context_window: 8192,
                data_policy_tags: if local { vec!["local-execution".into()] } else { vec![] },
                cost_class: CostClass::Low,
                latency_class: LatencyClass::Standard,
                aliases: aliases.iter().map(|a| a.to_string()).collect(),
            },
            unavailable: None,
        }
    }

    fn catalog() -> Vec<RouteModel> {
        vec![
            model("cloud", "big", Execution::Cloud, &["cloud-fast", "cloud-standard"]),
            model("llama", "small", Execution::Local, &["local-fast"]),
            model("extractive", "extractive-v1", Execution::Local, &["extractive"]),
        ]
    }

    fn cloud_config() -> AiConfig {
        let mut config = AiConfig::default();
        config.flags.cloud_models_enabled = true;
        config
    }

    fn run(config: &AiConfig, options: &TaskOptions, host: Option<&str>, intent: TaskIntent, models: &[RouteModel]) -> Routed {
        route(&RouteInput { task: TaskId(1), intent, options, config, host, models })
    }

    fn chosen_id(routed: &Routed) -> Option<String> {
        routed.decision.chosen.as_ref().map(|m| m.model_id.clone())
    }

    #[test]
    fn cloud_is_off_by_default() {
        let routed = run(&AiConfig::default(), &TaskOptions::default(), None, TaskIntent::Ask, &catalog());
        assert_eq!(chosen_id(&routed).as_deref(), Some("small"));
        assert!(routed.decision.local_execution);
        assert!(routed.decision.considered.iter().any(|c| c.reason == "excluded.cloud_disabled"));
    }

    #[test]
    fn aliases_and_prefer_order_choose_the_model() {
        let config = cloud_config();
        let routed = run(&config, &TaskOptions::default(), None, TaskIntent::Summarize, &catalog());
        // page_summary prefers local-long-context then cloud-standard.
        assert_eq!(chosen_id(&routed).as_deref(), Some("big"));
        assert_eq!(routed.decision.route_class, "page_summary");
        assert_eq!(routed.chain.len(), 3);
        let selection_options = TaskOptions { intent: TaskIntent::Explain, ..Default::default() };
        let routed = run(&config, &selection_options, None, TaskIntent::Explain, &catalog());
        assert_eq!(chosen_id(&routed).as_deref(), Some("small"));
        assert_eq!(routed.decision.route_class, "selection_explanation");
        assert_eq!(routed.decision.fallbacks.len(), 2);
    }

    #[test]
    fn hard_constraints_beat_preference() {
        let mut config = cloud_config();
        config.domains.cloud_denied = vec!["bank.example".into()];
        // Prefer cloud for everything; constraints must still win.
        config.routes.insert("page_summary".into(), RouteRule { prefer: vec!["cloud-standard".into()], ..Default::default() });

        let denied = run(&config, &TaskOptions::default(), Some("www.bank.example"), TaskIntent::Summarize, &catalog());
        assert_eq!(chosen_id(&denied).as_deref(), Some("small"));
        assert_eq!(denied.decision.route_class, "private_page");
        assert_eq!(denied.local_only_cause, Some("excluded.domain_cloud_denied"));
        assert!(denied.chain.iter().all(|m| m.provider_id != "cloud"));

        let forced = TaskOptions { force_local: true, route: RoutePreference::Model { provider_id: "cloud".into(), model_id: "big".into(), allow_fallback: true }, ..Default::default() };
        let routed = run(&config, &forced, None, TaskIntent::Summarize, &catalog());
        assert!(routed.chain.iter().all(|m| m.provider_id != "cloud"));
        assert!(routed.decision.considered.iter().any(|c| c.reason == "excluded.force_local"));

        let local_only = TaskOptions { route: RoutePreference::LocalOnly, ..Default::default() };
        let routed = run(&config, &local_only, None, TaskIntent::Summarize, &catalog());
        assert!(routed.decision.local_execution);
        assert!(routed.decision.considered.iter().any(|c| c.reason == "excluded.local_only_requested"));

        let mut off = config.clone();
        off.flags.cloud_models_enabled = false;
        let routed = run(&off, &TaskOptions::default(), None, TaskIntent::Summarize, &catalog());
        assert!(routed.chain.iter().all(|m| m.provider_id != "cloud"));
    }

    #[test]
    fn require_tags_drop_models_that_lack_them() {
        let mut config = cloud_config();
        config.routes.insert("page_summary".into(), RouteRule { require_tags: vec!["no-retention".into()], ..Default::default() });
        let mut models = catalog();
        models[1].manifest.data_policy_tags.push("no-retention".into());
        let routed = run(&config, &TaskOptions::default(), None, TaskIntent::Summarize, &models);
        assert_eq!(routed.chain.len(), 1);
        assert_eq!(chosen_id(&routed).as_deref(), Some("small"));
        assert_eq!(routed.decision.considered.iter().filter(|c| c.reason == "excluded.missing_tag").count(), 2);
    }

    #[test]
    fn fallback_order_orders_the_unpreferred_rest() {
        let mut config = cloud_config();
        config.routes.clear();
        config.fallback_order = vec!["extractive".into(), "cloud/big".into()];
        let routed = run(&config, &TaskOptions::default(), None, TaskIntent::Ask, &catalog());
        assert_eq!(chosen_id(&routed).as_deref(), Some("extractive-v1"));
        assert_eq!(routed.decision.fallbacks[0].model_id, "big");
        assert_eq!(routed.decision.fallbacks[1].model_id, "small");
    }

    #[test]
    fn explicit_model_without_fallback_fails_closed_when_unavailable() {
        let config = cloud_config();
        let mut models = catalog();
        models[0].unavailable = Some("api_key_missing".into());
        let options = TaskOptions {
            route: RoutePreference::Model { provider_id: "cloud".into(), model_id: "big".into(), allow_fallback: false },
            ..Default::default()
        };
        let routed = run(&config, &options, None, TaskIntent::Ask, &models);
        assert!(routed.decision.chosen.is_none());
        assert!(routed.chain.is_empty());
        assert!(routed.decision.considered.iter().any(|c| c.reason == "excluded.provider_unavailable"));

        let options = TaskOptions {
            route: RoutePreference::Model { provider_id: "cloud".into(), model_id: "big".into(), allow_fallback: true },
            ..Default::default()
        };
        let routed = run(&config, &options, None, TaskIntent::Ask, &models);
        assert_eq!(chosen_id(&routed).as_deref(), Some("small"));
    }

    #[test]
    fn act_needs_tool_support() {
        let mut models = catalog();
        models[1].manifest.supports_tools = false;
        let routed = run(&AiConfig::default(), &TaskOptions::default(), None, TaskIntent::Act, &models);
        assert_eq!(chosen_id(&routed).as_deref(), Some("extractive-v1"));
        assert!(routed.decision.considered.iter().any(|c| c.reason == "excluded.no_tool_support"));
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://User@Shop.Example.com:8443/a?b#c").as_deref(), Some("shop.example.com"));
        assert_eq!(host_of("http://[::1]:80/").as_deref(), Some("::1"));
        assert_eq!(host_of("about:blank"), None);
    }
}
