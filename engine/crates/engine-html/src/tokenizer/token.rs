//! HTML tokens.
//!
//! Tag and attribute names are lowercased at emission (the tokenizer's
//! `attribute name` and `tag name` states lowercase ASCII), which is what the
//! tree constructor expects and what the canonical fixture format shows.

/// A character reference that could not be resolved is emitted as literal text
/// by the tokenizer; this type only carries resolved values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Doctype { name: String, public_id: Option<String>, system_id: Option<String>, force_quirks: bool },
    /// A start tag. `self_closing` is set for `<foo/>`; for HTML (non-foreign)
    /// elements the tree constructor ignores it except at EOF, per spec.
    StartTag { name: String, attrs: Vec<Attribute>, self_closing: bool },
    EndTag { name: String },
    Comment(String),
    /// A run of character data. Emitted in contiguous runs so the tree
    /// constructor coalesces adjacent text without a per-character call.
    Chars(String),
    Eof,
}

impl Token {
    pub fn is_start(&self, name: &str) -> bool {
        matches!(self, Token::StartTag { name: n, .. } if n == name)
    }

    pub fn is_end(&self, name: &str) -> bool {
        matches!(self, Token::EndTag { name: n } if n == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicates() {
        let t = Token::StartTag { name: "div".into(), attrs: vec![], self_closing: false };
        assert!(t.is_start("div"));
        assert!(!t.is_start("p"));
        let e = Token::EndTag { name: "div".into() };
        assert!(e.is_end("div"));
    }
}