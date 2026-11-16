//! The HTML tokenizer: state machine, token types, and character references.

pub mod char_ref;
pub mod state;
pub mod token;

pub use state::{tokenize, Tokenizer, TokenizerError};
pub use token::{Attribute, Token};