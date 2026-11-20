//! Native dispatch values and UI surface identities. Public commands, resource
//! reads and event subscriptions all travel through the MCP server; these types
//! are not a second WebView command transport.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Shell,
    Overlay,
    Sidebar,
    Settings(crate::tabs::TabId),
    ApiCall(u64),
}

impl Surface {
    pub fn target(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Overlay => "overlay",
            Self::Sidebar => "sidebar",
            Self::Settings(_) => "settings",
            Self::ApiCall(_) => "api",
        }
    }
}

/// A command passed to the native dispatcher.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Correlates the reply with the pending promise. `null` for fire-and-forget.
    #[serde(default)]
    pub id: Option<u64>,
    pub cmd: String,
    #[serde(default)]
    pub payload: Value,
}

/// A reply to a command.
#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok(id: Option<u64>, data: Value) -> Self {
        Self {
            id,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn empty(id: Option<u64>) -> Self {
        Self {
            id,
            ok: true,
            data: None,
            error: None,
        }
    }

    pub fn err(id: Option<u64>, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            data: None,
            error: Some(error.into()),
        }
    }
}

/// Whether the shared public shell API catalog contains this name.
pub fn is_known_command(cmd: &str) -> bool {
    ai_platform_contract::browser::find(cmd).is_some()
}

/// Turn a Rust string into a JS string literal safe to embed in
/// `evaluate_script`. `serde_json` escaping is exactly JS string escaping for
/// the BMP and produces surrogate pairs for astral chars — and `<`/`>`/`&`
/// are left alone, which is fine because we never interpolate into HTML.
pub fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Wall-clock seconds since the Unix epoch. The whole app stores time as i64
/// seconds; nothing needs sub-second resolution except the startup timers,
/// which use `sys::process_uptime_ms`.
pub fn now_secs() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // A clock before 1970 is not a panic condition for a browser.
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_without_id_parses() {
        let r: Request = serde_json::from_str(r#"{"cmd":"shell_page_navigation","payload":{"operation":"reload"}}"#).unwrap();
        assert_eq!(r.cmd, "shell_page_navigation");
        assert!(r.id.is_none());
    }

    #[test]
    fn request_without_payload_parses() {
        let r: Request = serde_json::from_str(r#"{"id":3,"cmd":"shell_page_url_copy"}"#).unwrap();
        assert_eq!(r.id, Some(3));
        assert!(!r.payload.is_object());
        assert!(r.payload.is_null());
    }

    #[test]
    fn reply_omits_absent_fields() {
        let s = serde_json::to_string(&Reply::empty(Some(1))).unwrap();
        assert_eq!(s, r#"{"id":1,"ok":true}"#);
    }

    #[test]
    fn js_string_escapes_quotes_and_newlines() {
        assert_eq!(js_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(js_string("x\ny"), r#""x\ny""#);
        // Non-ASCII passes through as UTF-8; the script tag carries UTF-8.
        assert_eq!(js_string("é"), "\"é\"");
    }

    #[test]
    fn command_list_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for api in ai_platform_contract::browser::definitions() {
            assert!(seen.insert(api.name.clone()), "duplicate command {}", api.name);
        }
    }
}