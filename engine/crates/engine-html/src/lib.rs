//! `engine-html` — HTML5 tokenizer and tree construction, written from scratch.
//!
//! Entry point: [`parse`]. The tokenizer is independently testable and is the
//! first stage of the pipeline; tree construction consumes its tokens.

pub mod charset;
pub mod parse;
pub mod tokenizer;
pub mod tree;

pub use parse::{parse, HtmlError, ParseOpts, ParseOutcome};
pub use tokenizer::{tokenize, Attribute, Token, TokenizerError};
pub use tree::HtmlApproximation;