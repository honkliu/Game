//! `engine-css` — the CSS parser.
//!
//! Pipeline: bytes → tokens (`tokenizer`) → rules (`stylesheet`) → declarations
//! (`declaration`, with shorthands expanded to longhands) → selectors
//! (`selectors` + `selector_parser`). The computed-style struct and the UA sheet
//! live here too, because both are *CSS*, not cascade policy.

pub mod declaration;
pub mod properties;
pub mod selectors;
pub mod selector_parser;
pub mod stylesheet;
pub mod tokenizer;
pub mod values;

pub use declaration::{parse_declarations, Declaration, DeclarationBlock, DeclaredValue, WideKeyword};
pub use properties::*;
pub use selectors::{
    Combinator, CompoundSelector, PseudoClass, Selector, SelectorList, SimpleSelector, Specificity,
    TypeSelector,
};
pub use selector_parser::{parse_selector_list, SelectorError};
pub use stylesheet::{
    initial_style, parse_stylesheet, parse_stylesheet_with_ctx, user_agent_stylesheet, CssError,
    MediaQuery, Origin, StyleRule, Stylesheet,
};
pub use tokenizer::{tokenize, tokenize_with_errors, Token};
pub use values::LengthCtx;