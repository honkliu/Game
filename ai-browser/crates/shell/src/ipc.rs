//! The IPC contract (DESIGN §6.2 / §6.3).
//!
//! One envelope in, one envelope out. Mirrored by hand in `ui/app.js` — the
//! mirror is a small set of string literals for command names and event names,
//! which is cheaper to keep honest than a codegen step for ~40 strings.
//!
//! Wire shape (UI → host), sent with `window.ipc.postMessage(JSON)`:
//! ```json
//! {"id": 12, "cmd": "nav.open", "payload": {"url": "...", "disposition": "current"}}
//! ```
//! Host → UI (delivered by `window.__native._dispatch(json)` via evaluate_script):
//! ```json
//! {"id": 12, "ok": true, "data": {...}}          // command reply
//! {"event": "tab.navigated", "payload": {...}}   // state event
//! {"id": 12, "ok": false, "error": "message"}    // command failure
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Shell,
    Overlay,
    Sidebar,
    Settings(crate::tabs::TabId),
}

impl Surface {
    pub fn target(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Overlay => "overlay",
            Self::Sidebar => "sidebar",
            Self::Settings(_) => "settings",
        }
    }
}

/// A command from the UI shell.
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

/// A host → UI event.
#[derive(Debug, Clone, Serialize)]
pub struct Event<'a> {
    pub event: &'a str,
    pub payload: Value,
}

impl<'a> Event<'a> {
    pub fn new(event: &'a str, payload: Value) -> Self {
        Self { event, payload }
    }
}

/// Every command name the host accepts. An exhaustive match in `app.rs` means
/// adding a command here forces an implementation.
pub const COMMANDS: &[&str] = &[
    "ai.session",
    "nav.open",
    "nav.back",
    "nav.forward",
    "nav.reload",
    "nav.reload_ignoring_cache",
    "nav.stop",
    "tab.select",
    "tab.close",
    "tab.move",
    "tab.pin",
    "tab.unpin",
    "tab.reopen_closed",
    "tab.new",
    "tab.duplicate",
    "tab.bookmark",
    "tab.bookmark_all",
    "tab.zoom_set",
    "tab.zoom_reset",
    "group.create",
    "group.rename",
    "group.set_color",
    "group.collapse",
    "group.delete",
    "find.start",
    "find.query",
    "find.next",
    "find.prev",
    "find.stop",
    "omnibox.query",
    "omnibox.activate",
    "history.query",
    "history.forget",
    "history.clear",
    "bookmark.toggle",
    "bookmark.remove",
    "download.action",
    "download.clear_list",
    "settings.update",
    "settings.open_folder",
    "settings.open",
    "shell.focus_omnibox",
    "shell.toggle_rail",
    "shell.open_palette",
    "shell.close_top_overlay",
    "shell.ready",
    "window.minimize",
    "window.toggle_maximize",
    "window.drag",
    "window.resize",
    "window.close",
    "diagnostics.self_test",
    "page.reveal_in_folder",
    "page.copy_url",
    "shortcut",
];

pub fn is_known_command(cmd: &str) -> bool {
    COMMANDS.contains(&cmd)
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
        let r: Request = serde_json::from_str(r#"{"cmd":"nav.reload","payload":{}}"#).unwrap();
        assert_eq!(r.cmd, "nav.reload");
        assert!(r.id.is_none());
    }

    #[test]
    fn request_without_payload_parses() {
        let r: Request = serde_json::from_str(r#"{"id":3,"cmd":"window.close"}"#).unwrap();
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
        for c in COMMANDS {
            assert!(seen.insert(*c), "duplicate command {c}");
        }
    }
}