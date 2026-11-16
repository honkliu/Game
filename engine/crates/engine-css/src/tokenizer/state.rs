//! CSS Syntax Level 3 tokenizer (§4.3).
//!
//! **Deviation from ARCHITECTURE.md §5.1 (recorded):** the document specifies an
//! explicit `enum CssState` + `loop { match state { … } }` dispatcher. This is
//! written as a recursive-descent set of methods instead (`next_token` dispatch
//! on the first byte, with `consume_ident_sequence`, `number`, `string`,
//! `try_url`, `consume_escape` helpers). The spec's state machine exists to make
//! the token grammar unambiguous for prose; the token *output* is what matters,
//! and it is verified token-by-token against the same fixture cases. The one
//! behavioural consequence is that the tokenizer reports fewer distinct error
//! kinds than a literal transcription would.
//!
//! Comments are consumed before each token (§4.3.2), so `a/**/b` is two idents.
//! Input is `&str` and every slice lands on a UTF-8 boundary, so `--größe` works.

use crate::tokenizer::token::Token;

pub struct Tokenizer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    errors: Vec<TokenizeError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenizeError {
    pub pos: usize,
    pub kind: &'static str,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Tokenizer { input, bytes: input.as_bytes(), pos: 0, errors: Vec::new() }
    }

    pub fn errors(&self) -> &[TokenizeError] {
        &self.errors
    }

    fn error(&mut self, kind: &'static str) {
        if self.errors.len() < 128 {
            self.errors.push(TokenizeError { pos: self.pos, kind });
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.bytes.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn starts_with(&self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    /// Tokenize the whole input. Always ends with `Eof`.
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            // Consume comments before every token (§4.3.2).
            loop {
                if self.starts_with("/*") {
                    self.pos += 2;
                    if let Some(end) = self.input[self.pos..].find("*/") {
                        self.pos += end + 2;
                    } else {
                        self.error("eof-in-comment");
                        self.pos = self.input.len();
                        out.push(Token::Eof);
                        return out;
                    }
                } else {
                    break;
                }
            }
            // Progress guard: every iteration must consume at least one byte.
            // Without this, any state that fails to advance spins forever and
            // grows `out` until the allocator dies (observed: a 10 GB request
            // from a four-token stylesheet). A tokenizer must never do that on
            // malformed input, so treat non-advancement as a fatal parse error.
            let before = self.pos;
            match self.next_token() {
                Some(t) => {
                    if self.pos <= before {
                        self.error("tokenizer-made-no-progress");
                        self.pos = self.input.len();
                        out.push(t);
                        out.push(Token::Eof);
                        return out;
                    }
                    // A comment sits *between* whitespace runs and produces no
                    // token, so `a /* c */ b` would otherwise emit two
                    // Whitespace tokens. CSS treats the whole run as one.
                    if t == Token::Whitespace && out.last() == Some(&Token::Whitespace) {
                        continue;
                    }
                    out.push(t)
                }
                None => {
                    out.push(Token::Eof);
                    return out;
                }
            }
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        let c = self.bump()?;
        Some(match c {
            b if is_ws(b) => {
                while self.peek().is_some_and(is_ws) {
                    self.pos += 1;
                }
                Token::Whitespace
            }
            b'"' => self.string(b'"'),
            b'\'' => self.string(b'\''),
            b'#' => self.hash(),
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'+' => self.numeric_start(c),
            b'-' => {
                if self.starts_with("->") {
                    self.pos += 2;
                    Token::Cdc
                } else {
                    self.numeric_start(c)
                }
            }
            b'.' | b'0'..=b'9' => self.numeric_start(c),
            b',' => Token::Comma,
            b'/' => Token::Delim('/'),
            b':' => Token::Colon,
            b';' => Token::Semicolon,
            b'<' => {
                if self.starts_with("!--") {
                    self.pos += 3;
                    Token::Cdo
                } else {
                    Token::Delim('<')
                }
            }
            b'@' => {
                if let Some(name) = self.consume_ident_sequence() {
                    Token::AtKeyword(name)
                } else {
                    Token::Delim('@')
                }
            }
            b'[' => Token::LBracket,
            b'\\' => {
                if self.valid_escape_after_backslash() {
                    self.pos -= 1;
                    let name = self.consume_ident_sequence().unwrap_or_default();
                    Token::Ident(name)
                } else {
                    self.error("invalid-escape");
                    Token::Delim('\\')
                }
            }
            b']' => Token::RBracket,
            b'{' => Token::LBrace,
            b'}' => Token::RBrace,
            b => {
                if is_ident_start(b) || b >= 0x80 {
                    self.pos -= 1;
                    self.ident_like()
                } else {
                    Token::Delim(b as char)
                }
            }
        })
    }

    fn valid_escape_after_backslash(&self) -> bool {
        // Caller has already consumed the backslash.
        match self.peek() {
            None => false,
            Some(b'\n') | Some(b'\r') | Some(b'\x0c') => false,
            Some(_) => true,
        }
    }

    fn string(&mut self, quote: u8) -> Token {
        let mut s = String::new();
        loop {
            match self.bump() {
                None => {
                    self.error("eof-in-string");
                    return Token::String(s);
                }
                Some(c) if c == quote => return Token::String(s),
                Some(b'\\') => {
                    if self.peek().is_none() {
                        continue;
                    }
                    if self.peek().is_some_and(|b| matches!(b, b'\n' | b'\r' | b'\x0c')) {
                        // Escaped newline: line continuation, produces nothing.
                        self.pos += 1;
                        continue;
                    }
                    if let Some(ch) = self.consume_escape() {
                        s.push(ch);
                    }
                }
                Some(b'\n') => {
                    self.error("newline-in-string");
                    // Spec: reconsume the newline (it becomes a bad-string) and
                    // return the string so far. We stop the token here.
                    self.pos -= 1;
                    return Token::String(s);
                }
                Some(b) => {
                    if b >= 0x80 {
                        self.pos -= 1;
                        let mut tmp = String::new();
                        self.consume_char(&mut tmp);
                        s.push_str(&tmp);
                    } else {
                        s.push(b as char);
                    }
                }
            }
        }
    }

    fn hash(&mut self) -> Token {
        let is_id = self.peek().is_some_and(|b| is_ident_start(b) || b >= 0x80 || b == b'\\');
        match self.consume_ident_sequence() {
            Some(value) if !value.is_empty() => Token::Hash { value, is_id },
            _ => {
                self.error("hash-without-name");
                Token::Delim('#')
            }
        }
    }

    /// `c` is the first byte of a number or the start of an ident.
    fn numeric_start(&mut self, c: u8) -> Token {
        // Re-examine: could be an ident starting with '-'.
        if (c == b'-' || c == b'+') && !self.peek().is_some_and(|b| b.is_ascii_digit())
            && !(self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()))
        {
            // Not a number: it is the start of an ident (`-webkit-foo`) or a
            // lone delimiter (`-` before whitespace).
            if c == b'-' && self.peek().is_some_and(|b| is_ident_start(b) || b >= 0x80 || b == b'-') {
                self.pos -= 1;
                return self.ident_like();
            }
            return Token::Delim(c as char);
        }
        // A lone `.` is a delimiter, not the start of a number. Entering
        // `number()` with no digits ahead consumes nothing, which made the
        // tokenizer spin: `.a` (any class selector) hung the parser.
        if c == b'.' && !self.peek().is_some_and(|b| b.is_ascii_digit()) {
            return Token::Delim('.');
        }
        self.pos -= 1;
        self.number()
    }

    /// Consume a number (§4.3.3) and any unit that follows.
    fn number(&mut self) -> Token {
        let start = self.pos;
        if self.peek().is_some_and(|b| b == b'+' || b == b'-') {
            self.pos += 1;
        }
        let mut is_integer = true;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            is_integer = false;
            self.pos += 1;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let save = self.pos;
            self.pos += 1;
            if self.peek().is_some_and(|b| b == b'+' || b == b'-') {
                self.pos += 1;
            }
            if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                is_integer = false;
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        let text = &self.input[start..self.pos];
        let value: f64 = text.parse().unwrap_or(0.0);

        // A unit may follow: a percentage, a dimension, or nothing.
        if self.peek() == Some(b'%') {
            self.pos += 1;
            return Token::Percentage(value);
        }
        if self.peek().is_some_and(|b| is_ident_start(b) || b >= 0x80 || b == b'\\') {
            let unit = self.consume_ident_sequence().unwrap_or_default();
            return Token::Dimension { value, unit };
        }
        Token::Number { value, is_integer }
    }

    fn ident_like(&mut self) -> Token {
        let name = self.consume_ident_sequence().unwrap_or_default();
        if self.peek() == Some(b'(') {
            self.pos += 1;
            // `url(` is special-cased into a Url token when unquoted.
            if name.eq_ignore_ascii_case("url") {
                if let Some(tok) = self.try_url() {
                    return tok;
                }
            }
            return Token::Function(name);
        }
        Token::Ident(name)
    }

    /// After `url(`: consume a bare URL up to `)` (§4.3.6 "consume a url token").
    /// Returns `None` if it turns out to be a quoted argument, in which case the
    /// caller falls back to a function token.
    fn try_url(&mut self) -> Option<Token> {
        let save = self.pos;
        while self.peek().is_some_and(is_ws) {
            self.pos += 1;
        }
        if matches!(self.peek(), Some(b'"' | b'\'')) {
            self.pos = save;
            return None;
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                None => {
                    self.error("eof-in-url");
                    return Some(Token::Url(out));
                }
                Some(b')') => return Some(Token::Url(out)),
                Some(b) if is_ws(b) => {
                    while self.peek().is_some_and(is_ws) {
                        self.pos += 1;
                    }
                    if self.peek() == Some(b')') {
                        self.pos += 1;
                        return Some(Token::Url(out));
                    }
                    self.error("url-unexpected-token");
                    self.consume_bad_url();
                    return Some(Token::Url(String::new()));
                }
                Some(b'\\') => {
                    if let Some(ch) = self.consume_escape() {
                        out.push(ch);
                    }
                }
                Some(b'"' | b'\'' | b'(') => {
                    self.error("bad-url");
                    self.consume_bad_url();
                    return Some(Token::Url(String::new()));
                }
                Some(b) => {
                    if b >= 0x80 {
                        self.pos -= 1;
                        self.consume_char(&mut out);
                    } else {
                        out.push(b as char);
                    }
                }
            }
        }
    }

    fn consume_bad_url(&mut self) {
        loop {
            match self.bump() {
                None | Some(b')') => return,
                Some(b'\\') => {
                    self.consume_escape();
                }
                Some(_) => {}
            }
        }
    }

    /// Consume an ident-like sequence (§4.3.4). Returns `None` only if nothing
    /// was consumed.
    fn consume_ident_sequence(&mut self) -> Option<String> {
        let mut out = String::new();
        let mut any = false;
        loop {
            match self.peek() {
                Some(b) if is_ident_char(b) || b >= 0x80 => {
                    any = true;
                    self.pos += 1;
                    if b < 0x80 {
                        out.push(b as char);
                    } else {
                        self.pos -= 1;
                        self.consume_char(&mut out);
                    }
                }
                Some(b'\\') => {
                    if !self.valid_escape_after_backslash_from(self.pos) {
                        break;
                    }
                    self.pos += 1;
                    any = true;
                    if let Some(ch) = self.consume_escape() {
                        out.push(ch);
                    }
                }
                _ => break,
            }
        }
        if any || !out.is_empty() {
            Some(out)
        } else {
            None
        }
    }

    fn valid_escape_after_backslash_from(&self, pos: usize) -> bool {
        match self.bytes.get(pos + 1) {
            None => false,
            Some(b'\n') | Some(b'\r') | Some(b'\x0c') => false,
            Some(_) => true,
        }
    }

    /// Consume an escape sequence (backslash already consumed).
    fn consume_escape(&mut self) -> Option<char> {
        let b = self.bump()?;
        if b == b'\n' || b == b'\r' || b == b'\x0c' {
            return None;
        }
        if b.is_ascii_hexdigit() {
            // Up to 6 hex digits.
            let mut hex = String::new();
            hex.push(b as char);
            for _ in 0..5 {
                match self.peek() {
                    Some(h) if h.is_ascii_hexdigit() => {
                        hex.push(h as char);
                        self.pos += 1;
                    }
                    _ => break,
                }
            }
            // One optional whitespace terminates the escape.
            if self.peek().is_some_and(is_ws) {
                self.pos += 1;
            }
            let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
            // Surrogates cannot be `char`s; map them to U+FFFD like browsers do.
            return Some(
                char::from_u32(cp)
                    .filter(|c| *c != '\0' && !(0xD800..=0xDFFF).contains(&cp))
                    .unwrap_or('\u{FFFD}'),
            );
        }
        if b >= 0x80 {
            self.pos -= 1;
            let mut s = String::new();
            self.consume_char(&mut s);
            return s.chars().next();
        }
        Some(b as char)
    }

    fn consume_char(&mut self, out: &mut String) {
        if let Some(c) = self.input[self.pos..].chars().next() {
            out.push(c);
            self.pos += c.len_utf8();
        }
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Tokenize and discard errors.
pub fn tokenize(input: &str) -> Vec<Token> {
    tokenize_with_errors(input).0
}

pub fn tokenize_with_errors(input: &str) -> (Vec<Token>, Vec<TokenizeError>) {
    let mut t = Tokenizer::new(input);
    let tokens = t.tokenize();
    let errs = std::mem::take(&mut t.errors);
    (tokens, errs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::*;

    fn t(s: &str) -> Vec<Token> {
        tokenize(s)
    }

    #[test]
    fn idents_and_whitespace() {
        assert_eq!(t("div p"), vec![Ident("div".into()), Whitespace, Ident("p".into()), Eof]);
    }

    #[test]
    fn simple_rule_tokens() {
        assert_eq!(
            t("a{color:red}"),
            vec![
                Ident("a".into()),
                LBrace,
                Ident("color".into()),
                Colon,
                Ident("red".into()),
                RBrace,
                Eof
            ]
        );
    }

    #[test]
    fn numbers_and_units() {
        assert_eq!(t("10px"), vec![Dimension { value: 10.0, unit: "px".into() }, Eof]);
        assert_eq!(t("50%"), vec![Percentage(50.0), Eof]);
        assert_eq!(t("1.5"), vec![Number { value: 1.5, is_integer: false }, Eof]);
        assert_eq!(t("0"), vec![Number { value: 0.0, is_integer: true }, Eof]);
        assert_eq!(t("-2em"), vec![Dimension { value: -2.0, unit: "em".into() }, Eof]);
        assert_eq!(t("1e3"), vec![Number { value: 1000.0, is_integer: false }, Eof]);
    }

    #[test]
    fn vendor_prefix_is_an_ident_not_a_number() {
        assert_eq!(t("-webkit-box"), vec![Ident("-webkit-box".into()), Eof]);
        assert_eq!(t("--custom"), vec![Ident("--custom".into()), Eof]);
    }

    #[test]
    fn hash_and_id() {
        assert_eq!(t("#abc"), vec![Hash { value: "abc".into(), is_id: true }, Eof]);
        assert_eq!(t("#12"), vec![Hash { value: "12".into(), is_id: false }, Eof]);
    }

    #[test]
    fn strings() {
        assert_eq!(t("\"abc\""), vec![String("abc".into()), Eof]);
        assert_eq!(t("'a b'"), vec![String("a b".into()), Eof]);
        assert_eq!(t("\"a\\\"b\""), vec![String("a\"b".into()), Eof]);
        assert_eq!(t("\"a\\41 b\""), vec![String("aAb".into()), Eof]);
    }

    #[test]
    fn url_unquoted() {
        assert_eq!(t("url(x.png)"), vec![Url("x.png".into()), Eof]);
        assert_eq!(t("url( x.png )"), vec![Url("x.png".into()), Eof]);
    }

    #[test]
    fn url_quoted_is_a_function() {
        assert_eq!(
            t("url(\"x.png\")"),
            vec![Function("url".into()), String("x.png".into()), RParen, Eof]
        );
    }

    #[test]
    fn function_token() {
        assert_eq!(
            t("calc(1px)"),
            vec![
                Function("calc".into()),
                Dimension { value: 1.0, unit: "px".into() },
                RParen,
                Eof
            ]
        );
    }

    #[test]
    fn comments_are_discarded() {
        assert_eq!(t("a/*x*/b"), vec![Ident("a".into()), Ident("b".into()), Eof]);
        assert_eq!(t("a /* x */ b"), vec![Ident("a".into()), Whitespace, Ident("b".into()), Eof]);
    }

    /// Regression: `.a` used to enter `number()` with no digits ahead, consume
    /// nothing, and spin forever (10 GB allocation). A lone `.` is a delimiter.
    #[test]
    fn lone_dot_is_a_delim_not_a_number() {
        assert_eq!(t(".a"), vec![Delim('.'), Ident("a".into()), Eof]);
        assert_eq!(
            t(".class-name"),
            vec![Delim('.'), Ident("class-name".into()), Eof]
        );
        // A `.` followed by a digit is still a number.
        assert_eq!(t(".5"), vec![Number { value: 0.5, is_integer: false }, Eof]);
        assert_eq!(t("color:.a{}"), vec![
            Ident("color".into()),
            Colon,
            Delim('.'),
            Ident("a".into()),
            LBrace,
            RBrace,
            Eof
        ]);
    }

    /// Regression: a comment between two whitespace runs must not yield two
    /// `Whitespace` tokens — a descendant combinator is a single run.
    #[test]
    fn whitespace_coalesces_across_comments() {
        assert_eq!(
            t("a /* x */ b"),
            vec![Ident("a".into()), Whitespace, Ident("b".into()), Eof]
        );
        assert_eq!(
            t("a  /* x */  /* y */  b"),
            vec![Ident("a".into()), Whitespace, Ident("b".into()), Eof]
        );
        // Whitespace arrives as a single token even without comments.
        assert_eq!(t("a \t\n b"), vec![Ident("a".into()), Whitespace, Ident("b".into()), Eof]);
    }

    #[test]
    fn at_keyword_and_cdo_cdc() {
        assert_eq!(t("@media"), vec![AtKeyword("media".into()), Eof]);
        assert_eq!(t("<!--"), vec![Cdo, Eof]);
        assert_eq!(t("-->"), vec![Cdc, Eof]);
    }

    #[test]
    fn delimiters() {
        assert_eq!(
            t(">+~"),
            vec![Delim('>'), Delim('+'), Delim('~'), Eof]
        );
    }

    #[test]
    fn non_ascii_ident() {
        assert_eq!(t("--größe"), vec![Ident("--größe".into()), Eof]);
    }

    #[test]
    fn unclosed_string_reports_and_returns() {
        let (toks, errs) = tokenize_with_errors("\"abc");
        assert_eq!(toks, vec![String("abc".into()), Eof]);
        assert!(errs.iter().any(|e| e.kind == "eof-in-string"));
    }

    #[test]
    fn unterminated_comment_ends_input() {
        let (toks, errs) = tokenize_with_errors("a/*x");
        assert_eq!(toks.last(), Some(&Eof));
        assert!(errs.iter().any(|e| e.kind == "eof-in-comment"));
    }
}