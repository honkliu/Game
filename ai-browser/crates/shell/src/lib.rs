//! native-browser — a Rust shell over WebView2 and WKWebView.
//!
//! Module map:
//! - `ai`       the AI platform surface: action gate, process-role split, `--proof`
//! - `app`      event loop, webview orchestration, IPC dispatch
//! - `layout`   webview geometry math
//! - `keys`     the complete keyboard map (§3.5), resolved from Win32 VKs
//! - `omnibox`  URL/search/command classification
//! - `tabs`     tab/group bookkeeping
//! - `history`  SQLite history, ranking, query tokens
//! - `bookmarks`, `store`  SQLite persistence
//! - `downloads`  download records + filename sanitization
//! - `session`, `settings`  JSON persistence
//! - `host`     internal page URLs (new tab, error, settings)
//! - `ipc`      the JSON envelope mirrored in `ui/app.js`
//! - `ui`       `asset://` protocol serving `ui/` from disk
//! - `sys`      the only raw Win32 beyond wry/tao
//! - `webview2` the runtime gate

pub mod ai;
pub mod app;
pub mod bookmarks;
pub mod downloads;
pub mod history;
pub mod host;
pub mod ipc;
pub mod keys;
pub mod layout;
pub mod omnibox;
pub mod pdf;
pub mod platform;
pub mod session;
pub mod settings;
pub mod store;
pub mod sys;
pub mod tabs;
pub mod tab_lifecycle;
pub mod ui;
#[cfg(windows)]
pub mod webview2;