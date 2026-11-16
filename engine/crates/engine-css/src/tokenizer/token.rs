//! CSS tokens (CSS Syntax Level 3 §4).

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Ident(String),
    /// A function token: the name including the opening paren is *not* part of
    /// it; the `(` has already been consumed.
    Function(String),
    AtKeyword(String),
    Hash { value: String, is_id: bool },
    Delim(char),
    Number { value: f64, is_integer: bool },
    Percentage(f64),
    Dimension { value: f64, unit: String },
    String(String),
    Url(String),
    Whitespace,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Eof,
}

impl Token {
    /// The token's CSS serialization, used in error messages and tests.
    pub fn name(&self) -> &'static str {
        match self {
            Token::Ident(_) => "ident",
            Token::Function(_) => "function",
            Token::AtKeyword(_) => "at-keyword",
            Token::Hash { .. } => "hash",
            Token::Delim(_) => "delim",
            Token::Number { .. } => "number",
            Token::Percentage(_) => "percentage",
            Token::Dimension { .. } => "dimension",
            Token::String(_) => "string",
            Token::Url(_) => "url",
            Token::Whitespace => "whitespace",
            Token::Cdo => "CDO",
            Token::Cdc => "CDC",
            Token::Colon => "colon",
            Token::Semicolon => "semicolon",
            Token::Comma => "comma",
            Token::LBracket => "[",
            Token::RBracket => "]",
            Token::LParen => "(",
            Token::RParen => ")",
            Token::LBrace => "{",
            Token::RBrace => "}",
            Token::Eof => "EOF",
        }
    }

    pub fn is_whitespace(&self) -> bool {
        matches!(self, Token::Whitespace)
    }

    pub fn as_ident(&self) -> Option<&str> {
        match self {
            Token::Ident(s) => Some(s),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(Token::Ident("x".into()).name(), "ident");
        assert_eq!(Token::Delim('+').name(), "delim");
    }

    #[test]
    fn as_ident() {
        assert_eq!(Token::Ident("div".into()).as_ident(), Some("div"));
        assert_eq!(Token::Whitespace.as_ident(), None);
    }
}