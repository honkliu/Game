//! Contracts for the AI-native browser (backlog Epic A: FND-01, FND-02, FND-04).
//!
//! Kernel decision (2026-11-13): the existing WebView2 runtime, driven by a Rust
//! shell. There is no Chromium checkout and no fork, so there is no Mojo. The
//! boundary these types describe is:
//!
//! ```text
//! Rust shell process  <── NDJSON frames over the child's stdin/stdout ──>  AI utility process
//!   owns identity                                                          owns providers,
//!   owns tabs, workspaces,                                                 prompts, routing,
//!   navigation, policy,                                                    task planning,
//!   final action dispatch                                                  structured output
//! ```
//!
//! Two rules hold the boundary up, and both are enforced by types rather than by
//! review:
//!
//! 1. **Identity is minted, never asserted.** [`identity::DocumentKey`] has no
//!    public constructor and is not `Deserialize`, so no peer -- including a
//!    compromised AI utility process -- can put one in a message. A peer sends a
//!    [`identity::DocumentToken`] (an opaque integer) and only the shell's
//!    [`identity::DocumentIdentityRegistry`] can turn it into authority.
//!    `tests/identity_contract.rs` asserts this with `compile_fail` doctests and
//!    value assertions.
//! 2. **A frame can only name a method its side is allowed to issue.**
//!    [`frame::decode`] takes the [`frame::Peer`] established when the pipe was
//!    brokered and rejects a method issued from the wrong side before the payload
//!    is interpreted. This is what replaced Mojo's "only the process that owns an
//!    interface can bind it".
//!
//! Crate layout:
//! - [`failure`]  -- the one typed failure taxonomy (FND-04).
//! - [`version`]  -- schema major/minor gate; old consumers reject, not guess (FND-02).
//! - [`identity`] -- the document identity model and ownership rules (FND-01).
//! - [`contract`] -- versioned context / action / progress / cancellation payloads.
//! - [`frame`]    -- request/response/notification envelope and the frame codec.
//! - [`supervise`]-- spawn, cancel, and kill of the AI utility process.
//!
//! `src/bin/stub_utility.rs` is the process the supervision tests drive. It is a
//! test double, not a provider implementation.

pub mod contract;
pub mod failure;
pub mod frame;
pub mod identity;
pub mod supervise;
pub mod version;

/// Major version stamped on every frame produced by this crate. Paired with
/// [`version::CONSUMER_SCHEMA_MAJOR`]; they are equal today and may diverge
/// during a rolling update, which is what the gate exists to handle.
pub const PRODUCER_SCHEMA_MAJOR: u16 = version::CONSUMER_SCHEMA_MAJOR;

/// Highest minor version this crate emits.
pub const PRODUCER_SCHEMA_MINOR: u16 = version::CONSUMER_SCHEMA_MINOR;
