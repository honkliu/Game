//! `engine-dom` — the DOM arena.
//!
//! Ownership: the arena owns all nodes in one `Vec`; identity is `NodeId`.
//! `tree_ops` is the sole mutator of link fields, so the link invariants have
//! exactly one place to be established and one place to test.

pub mod document;
pub mod element;
pub mod mutation;
pub mod node;
pub mod serialize;
pub mod tree_ops;

pub use document::{Descendants, Document};
pub use element::{is_void, KnownTag, Namespace, Tag};
pub use node::{Attr, ElementData, Node, NodeKind, QuirksMode};
pub use tree_ops::DomError;