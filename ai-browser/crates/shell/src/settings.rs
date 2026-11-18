//! `settings.json` — every control in DESIGN §4.8.
//!
//! Every field is `serde(default)` and the file is written whole on each change
//! (it is ~500 bytes; an atomic replace is cheaper than a delta format and has
//! no partial-write window).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default search template. DuckDuckGo's HTML endpoint: a plain GET, no
/// client-side suggest API (§4.8 Search, §2 D-keep list).
pub const DEFAULT_SEARCH_TEMPLATE: &str = "https://html.duckduckgo.com/html/?q=%s";

pub const DEFAULT_HISTORY_CAP: u32 = 100_000;
pub const DEFAULT_SLEEP_MINUTES: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanLayout {
    pub cols: usize,
    pub rows: usize,
    pub display_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Appearance
    pub theme: Theme,
    pub rail_collapsed: bool,
    /// Tabs in a vertical rail at the left instead of the default strip on top.
    pub vertical_tabs: bool,
    pub compact_tab_rows: bool,
    pub density: Density,

    // Search
    pub search_template: String,

    // Privacy
    pub clear_history_on_exit: bool,
    pub clear_cookies_on_exit: bool,
    pub clear_cache_on_exit: bool,
    /// §4.8: default on, "no truncation".
    pub always_show_full_url: bool,

    // Advanced
    pub history_cap: u32,
    pub sleep_after_minutes: u32,
    pub download_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_layout: Option<SpanLayout>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            rail_collapsed: false,
            vertical_tabs: false,
            compact_tab_rows: false,
            density: Density::Comfortable,
            search_template: DEFAULT_SEARCH_TEMPLATE.to_string(),
            clear_history_on_exit: false,
            clear_cookies_on_exit: false,
            clear_cache_on_exit: false,
            always_show_full_url: true,
            history_cap: DEFAULT_HISTORY_CAP,
            sleep_after_minutes: DEFAULT_SLEEP_MINUTES,
            download_dir: String::new(),
            span_layout: None,
        }
    }
}

/// A settings change from the UI. The UI sends `{key, value}` (§6.2
/// `settings.update`); applying is a `match`, so an unknown key is a reported
/// error rather than a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    UnknownKey(String),
    BadValue { key: &'static str, value: String },
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::UnknownKey(k) => write!(f, "unknown settings key: {k}"),
            UpdateError::BadValue { key, value } => {
                write!(f, "invalid value for {key}: {value}")
            }
        }
    }
}

fn parse_bool(v: &serde_json::Value) -> Option<bool> {
    match v {
        serde_json::Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn parse_u32(v: &serde_json::Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl Settings {
    /// Where the tabs are drawn.
    pub fn tabs(&self) -> crate::layout::Tabs {
        if self.vertical_tabs {
            self.rail_collapsed.into()
        } else {
            crate::layout::Tabs::Strip
        }
    }
    /// Apply one `settings.update` payload. Range clamps are intentional and
    /// bounded (a user typing 0 for sleep means "never sleep" → clamp to 1).
    pub fn apply(&mut self, key: &str, value: &serde_json::Value) -> Result<(), UpdateError> {
        match key {
            "theme" => {
                self.theme = match value.as_str() {
                    Some("light") => Theme::Light,
                    Some("dark") => Theme::Dark,
                    Some("system") => Theme::System,
                    _ => return Err(UpdateError::BadValue { key: "theme", value: value.to_string() }),
                }
            }
            "rail_collapsed" => {
                self.rail_collapsed =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "rail_collapsed", value: value.to_string() })?
            }
            "vertical_tabs" => {
                self.vertical_tabs =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "vertical_tabs", value: value.to_string() })?
            }
            "compact_tab_rows" => {
                self.compact_tab_rows =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "compact_tab_rows", value: value.to_string() })?
            }
            "density" => {
                self.density = match value.as_str() {
                    Some("comfortable") => Density::Comfortable,
                    Some("compact") => Density::Compact,
                    _ => return Err(UpdateError::BadValue { key: "density", value: value.to_string() }),
                }
            }
            "search_template" => {
                let s = value
                    .as_str()
                    .ok_or(UpdateError::BadValue { key: "search_template", value: value.to_string() })?;
                // A template without %s cannot carry a query; reject loudly
                // instead of navigating to a page that ignores the user.
                if !s.contains("%s") {
                    return Err(UpdateError::BadValue { key: "search_template", value: value.to_string() });
                }
                self.search_template = s.to_string();
            }
            "clear_history_on_exit" => {
                self.clear_history_on_exit =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "clear_history_on_exit", value: value.to_string() })?
            }
            "clear_cookies_on_exit" => {
                self.clear_cookies_on_exit =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "clear_cookies_on_exit", value: value.to_string() })?
            }
            "clear_cache_on_exit" => {
                self.clear_cache_on_exit =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "clear_cache_on_exit", value: value.to_string() })?
            }
            "always_show_full_url" => {
                self.always_show_full_url =
                    parse_bool(value).ok_or(UpdateError::BadValue { key: "always_show_full_url", value: value.to_string() })?
            }
            "history_cap" => {
                let n = parse_u32(value)
                    .ok_or(UpdateError::BadValue { key: "history_cap", value: value.to_string() })?;
                self.history_cap = n.clamp(1_000, 1_000_000);
            }
            "sleep_after_minutes" => {
                let n = parse_u32(value)
                    .ok_or(UpdateError::BadValue { key: "sleep_after_minutes", value: value.to_string() })?;
                self.sleep_after_minutes = n.clamp(1, 240);
            }
            "download_dir" => {
                self.download_dir = value
                    .as_str()
                    .ok_or(UpdateError::BadValue { key: "download_dir", value: value.to_string() })?
                    .to_string();
            }
            other => return Err(UpdateError::UnknownKey(other.to_string())),
        }
        Ok(())
    }

    /// Theme resolution against the OS preference (`prefers-color-scheme`).
    pub fn effective_theme(&self, system_dark: bool) -> Theme {
        match self.theme {
            Theme::System if system_dark => Theme::Dark,
            Theme::System => Theme::Light,
            t => t,
        }
    }
}

/// Load settings, falling back to defaults. A corrupt file is *not* silently
/// replaced on load — it is returned so the caller can log and move the bad
/// file aside; a browser must start even when its config is damaged.
pub fn load(path: &Path) -> (Settings, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(s) => (s, None),
            Err(e) => recover_from_backup(path, format!("settings.json unreadable ({e})")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
        Err(e) => recover_from_backup(path, format!("settings.json not readable ({e})")),
    }
}

/// An interrupted `save` can leave the previous file as `settings.json.bak`.
/// Prefer it to defaults, and say so.
fn recover_from_backup(path: &Path, why: String) -> (Settings, Option<String>) {
    let bak = path.with_extension("json.bak");
    if let Ok(text) = std::fs::read_to_string(&bak) {
        if let Ok(s) = serde_json::from_str::<Settings>(&text) {
            return (s, Some(format!("{why}; recovered from settings.json.bak")));
        }
    }
    (Settings::default(), Some(format!("{why}; using defaults")))
}

/// Atomic write: temp file in the same directory, then replace.
///
/// On Windows `rename` refuses to overwrite, so the destination is moved aside
/// first and removed only after the new file is in place. A crash in between
/// leaves the previous settings recoverable rather than no file at all.
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bak = path.with_extension("json.bak");
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, json)?;
    let _ = std::fs::remove_file(&bak);
    let _ = std::fs::rename(path, &bak);
    std::fs::rename(&tmp, path)?;
    let _ = std::fs::remove_file(&bak);
    Ok(())
}

/// Resolve the default download directory without a `dirs`-style crate:
/// `%USERPROFILE%\Downloads`, which is where Windows puts it for the default
/// shell layout; falls back to the profile root.
pub fn default_download_dir() -> String {
    let home = if cfg!(target_os = "macos") { "HOME" } else { "USERPROFILE" };
    if let Ok(profile) = std::env::var(home) {
        let downloads = PathBuf::from(&profile).join("Downloads");
        if cfg!(target_os = "macos") || downloads.is_dir() {
            return downloads.display().to_string();
        }
        return profile;
    }
    String::from(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let s = Settings {
            theme: Theme::Dark,
            history_cap: 4_242,
            rail_collapsed: true,
            span_layout: Some(SpanLayout { cols: 2, rows: 1, display_ids: vec!["monitor-3".into(), "monitor-1".into()] }),
            ..Default::default()
        };
        save(&path, &s).unwrap();
        let (loaded, warn) = load(&path);
        assert!(warn.is_none());
        assert_eq!(loaded, s);
    }

    #[test]
    fn save_twice_overwrites_and_leaves_no_debris() {
        // The old `rename` refused to replace on Windows; the fix must both
        // succeed and clean up.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save(&path, &Settings::default()).unwrap();
        let changed = Settings { theme: Theme::Dark, ..Default::default() };
        save(&path, &changed).unwrap();
        let (loaded, warn) = load(&path);
        assert!(warn.is_none());
        assert_eq!(loaded.theme, Theme::Dark);
        assert!(!dir.path().join("settings.json.tmp").exists());
        assert!(!dir.path().join("settings.json.bak").exists());
    }

    #[test]
    fn corrupt_main_file_recovers_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        save(&path, &Settings { theme: Theme::Dark, ..Default::default() }).unwrap();
        std::fs::rename(&path, path.with_extension("json.bak")).unwrap();
        std::fs::write(&path, "{ truncated").unwrap();
        let (s, warn) = load(&path);
        assert_eq!(s.theme, Theme::Dark);
        assert!(warn.unwrap().contains("recovered"));
    }

    #[test]
    fn corrupt_file_yields_defaults_and_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let (s, warn) = load(&path);
        assert_eq!(s, Settings::default());
        assert!(warn.is_some());
    }

    #[test]
    fn missing_file_is_not_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let (s, warn) = load(&dir.path().join("nope.json"));
        assert_eq!(s, Settings::default());
        assert!(warn.is_none());
    }

    #[test]
    fn partial_json_keeps_defaults_for_missing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
        let (s, warn) = load(&path);
        assert!(warn.is_none());
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.history_cap, DEFAULT_HISTORY_CAP, "serde(default) fills the rest");
    }

    #[test]
    fn apply_accepts_known_keys_and_rejects_unknown() {
        let mut s = Settings::default();
        s.apply("theme", &json!("dark")).unwrap();
        assert_eq!(s.theme, Theme::Dark);
        s.apply("history_cap", &json!(5)).unwrap();
        assert_eq!(s.history_cap, 1_000, "clamped to the floor");
        assert_eq!(
            s.apply("nope", &json!(1)),
            Err(UpdateError::UnknownKey("nope".into()))
        );
        assert!(matches!(
            s.apply("theme", &json!("neon")),
            Err(UpdateError::BadValue { .. })
        ));
    }

    #[test]
    fn search_template_must_contain_placeholder() {
        let mut s = Settings::default();
        assert!(s.apply("search_template", &json!("https://e.com/?q=%s")).is_ok());
        assert!(s.apply("search_template", &json!("https://e.com/")).is_err());
    }

    #[test]
    fn system_theme_follows_os() {
        let mut s = Settings::default();
        assert_eq!(s.effective_theme(true), Theme::Dark);
        assert_eq!(s.effective_theme(false), Theme::Light);
        s.theme = Theme::Light;
        assert_eq!(s.effective_theme(true), Theme::Light);
    }
}