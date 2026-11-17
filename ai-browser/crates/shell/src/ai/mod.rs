//! The shell's AI surface (Release 0a).
//!
//! This is the directory where the browser process stops being only a browser and
//! becomes the authority the AI platform depends on. What lives here is decided by
//! ownership, not by topic: the shell owns identity, tabs, navigation, policy, and
//! the final action dispatch, so those live here; the AI utility process owns
//! providers, prompts, routing, and task planning, and has no code in this
//! directory at all. The shared vocabulary between the two is the path dependency
//! `ai-platform-contract` (`../ai-browser/foundation`), whose types are used as-is —
//! the shell does not mirror them, because one set of Rust types on both sides of
//! that boundary is the point of the design.
//!
//! | Module | Owns |
//! |---|---|
//! | [`policy`] | the Release 0 dispatch gate: which action kinds may run at all |
//! | [`processes`] | our process tree split by WebView2 role (browser vs renderer) |
//! | [`mode`] | the default-off, `--proof`-only toggles that make a budget measurable |
//! | [`proof`] | `--proof`: the measured PASS/FAIL harness for the R0a budgets |
//!
//! Deliberately absent, because Release 0a promises none of it: any action that
//! mutates a page, any Win32 input-synthesis primitive (the name scan in
//! [`policy`]'s tests keeps them out), element
//! handles, a context extractor, a second provider, and any accessibility-tree
//! surface (unavailable at this SDK). The contract types for several of those
//! already exist; the runtime does not, and nothing here invents one early.
//!
//! The single shared `ICoreWebView2Environment` stays where it already was —
//! [`crate::webview2::create_shared_environment`] — and is re-exported below so
//! `ai::` is the one path a caller reads from, with its construction counter next to
//! it (R0a-1).

pub mod answer;
pub mod context;
pub mod identity;
pub mod link;
pub mod mode;
pub mod policy;
pub mod processes;
pub mod proof;
pub mod provider;
pub mod session;
pub mod ui_state;

pub use crate::platform::{create_shared_environment, environments_constructed};
