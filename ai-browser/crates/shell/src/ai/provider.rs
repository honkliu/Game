//! What produced an answer, as the card states it.
//!
//! The shell makes no model or network calls: providers live in the AI utility
//! process (`ai-browser/utility`). What stays here is the label the shell shows
//! beside an answer, derived from the utility's routing decision, so the UI cannot
//! describe a remote model as local or the reverse.

/// Provenance of one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// `"offline"` when the model ran on this machine, `"http"` when the prompt
    /// left it.
    pub kind: &'static str,
    /// Display name of the model that answered.
    pub model: String,
    /// True when page text was sent off this machine.
    pub egress: bool,
}

impl ProviderInfo {
    /// Before routing has said anything: nothing has left the machine yet.
    pub fn pending() -> ProviderInfo {
        ProviderInfo { kind: "offline", model: String::new(), egress: false }
    }

    pub fn from_route(model: &str, local_execution: bool) -> ProviderInfo {
        ProviderInfo {
            kind: if local_execution { "offline" } else { "http" },
            model: model.to_string(),
            egress: !local_execution,
        }
    }
}
