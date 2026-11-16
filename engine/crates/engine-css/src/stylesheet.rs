//! Stylesheet parsing: at-rules, qualified rules, and declaration blocks.
//!
//! Error recovery: an invalid declaration is dropped and parsing resumes at the
//! next `;` at the current nesting level; an invalid rule is dropped and parsing
//! resumes at the next `}`. We never abandon the whole sheet on one error.

use engine_core::Length;

use crate::declaration::{parse_declarations, DeclarationBlock, DeclError};
use crate::properties::ComputedStyle;
use crate::selectors::SelectorList;
use crate::selector_parser::parse_selector_list;
use crate::tokenizer::state::tokenize;
use crate::tokenizer::token::Token;
use crate::values::LengthCtx;

/// Where a sheet came from; drives cascade order (§4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    UserAgent,
    User,
    Author,
}

/// A media condition we can evaluate.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaQuery {
    /// `all`, or an empty query.
    All,
    /// `screen` (the only media type we match).
    Screen,
    /// `print` and anything else never matches.
    Never,
    Feature { name: String, value: Length },
    And(Vec<MediaQuery>),
    Or(Vec<MediaQuery>),
    Not(Box<MediaQuery>),
    /// A condition we cannot evaluate (`@container`-style), treated as false.
    Unsupported(String),
}

#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selectors: SelectorList,
    pub declarations: DeclarationBlock,
    /// Media conditions from enclosing `@media` blocks; all must hold.
    pub media: Vec<MediaQuery>,
    /// Source order within the sheet, for tie-breaking.
    pub order: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    pub errors: Vec<CssError>,
    /// At-rules we parsed and skipped, for the report.
    pub skipped_at_rules: Vec<String>,
    pub origin: Option<Origin>,
    /// `@import` URLs, resolved by the caller.
    pub imports: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssError {
    pub message: String,
    pub pos: usize,
}

/// Maximum nesting of at-rule blocks (`@media` inside `@media` inside …).
///
/// `at_rule` recurses for every `@media`/`@supports` block, so
/// `"@media screen{".repeat(n)` is n stack frames deep. Real sheets nest one or
/// two levels (`@media` containing `@supports`); the cap is far above that and
/// exists so a hostile sheet becomes a parse error with the offending block
/// skipped, not a process abort.
pub const MAX_AT_RULE_NESTING: u32 = 16;

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    ctx: &'a LengthCtx,
    sheet: Stylesheet,
    media_stack: Vec<MediaQuery>,
    order: u32,
    /// Current at-rule block nesting depth, bounded by `MAX_AT_RULE_NESTING`.
    at_rule_depth: u32,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Token::Whitespace) {
            self.pos += 1;
        }
    }

    fn error(&mut self, msg: impl Into<String>) {
        if self.sheet.errors.len() < 256 {
            self.sheet.errors.push(CssError { message: msg.into(), pos: self.pos });
        }
    }

    fn stylesheet(mut self) -> Stylesheet {
        loop {
            self.skip_ws();
            match self.peek().clone() {
                Token::Eof => break,
                Token::Cdo | Token::Cdc => {
                    self.pos += 1;
                }
                Token::AtKeyword(name) => {
                    self.pos += 1;
                    self.at_rule(name);
                }
                _ => {
                    self.qualified_rule();
                }
            }
        }
        self.sheet
    }

    /// Collect the raw tokens of a block body, up to and including the `}`.
    fn block_body(&mut self) -> Vec<Token> {
        let mut depth = 1i32;
        let mut out = Vec::new();
        loop {
            match self.bump() {
                Token::Eof => break,
                Token::LBrace | Token::Function(_) | Token::LParen | Token::LBracket => {
                    depth += 1;
                    out.push(self.tokens[self.pos - 1].clone());
                }
                Token::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(Token::RBrace);
                }
                Token::RParen | Token::RBracket => {
                    depth -= 1;
                    // Still closes the sub-group; keep it for the sub-parser.
                    out.push(self.tokens[self.pos - 1].clone());
                    // Restore: these do not close the top-level brace.
                    depth += 1;
                }
                t => out.push(t),
            }
        }
        out.push(Token::Eof);
        out
    }

    /// Collect an at-rule prelude up to `;` or `{`, returning (prelude, has_block).
    fn at_rule_prelude(&mut self) -> (Vec<Token>, bool) {
        let mut out = Vec::new();
        let mut depth = 0i32;
        loop {
            match self.peek().clone() {
                Token::Eof => return (out, false),
                Token::Function(_) | Token::LParen | Token::LBracket => {
                    depth += 1;
                    out.push(self.bump());
                }
                Token::RParen | Token::RBracket => {
                    depth -= 1;
                    out.push(self.bump());
                }
                Token::Semicolon if depth == 0 => {
                    self.pos += 1;
                    return (out, false);
                }
                Token::LBrace if depth == 0 => {
                    self.pos += 1;
                    return (out, true);
                }
                _ => out.push(self.bump()),
            }
        }
    }

    fn at_rule(&mut self, name: String) {
        let lower = name.to_ascii_lowercase();
        let (prelude, has_block) = self.at_rule_prelude();
        if has_block && self.at_rule_depth >= MAX_AT_RULE_NESTING {
            // Too deep to recurse safely. Skip the block wholesale (the
            // `block_body` scan is iterative) and record it rather than
            // letting the stack die on `@media{@media{@media{…`.
            self.error(format!("at-rule nesting deeper than {MAX_AT_RULE_NESTING}"));
            self.block_body();
            self.sheet.skipped_at_rules.push(format!("@{lower}"));
            return;
        }
        match lower.as_str() {
            "media" => {
                let q = parse_media_query(&prelude);
                if has_block {
                    self.media_stack.push(q);
                    self.at_rule_depth += 1;
                    loop {
                        self.skip_ws();
                        match self.peek().clone() {
                            Token::Eof => break,
                            Token::RBrace => {
                                self.pos += 1;
                                break;
                            }
                            Token::AtKeyword(n) => {
                                self.pos += 1;
                                self.at_rule(n);
                            }
                            _ => self.qualified_rule(),
                        }
                    }
                    self.at_rule_depth -= 1;
                    self.media_stack.pop();
                }
            }
            "import" => {
                for t in &prelude {
                    if let Token::String(url) | Token::Url(url) = t {
                        self.sheet.imports.push(url.clone());
                    }
                }
            }
            "charset" | "layer" | "namespace" => {}
            "supports" => {
                if has_block {
                    // `@supports (prop: value)` is true iff we parse the property.
                    let ok = supports_condition(&prelude, self.ctx);
                    let body = self.block_body();
                    if ok {
                        let mut sub = Parser {
                            tokens: body,
                            pos: 0,
                            ctx: self.ctx,
                            sheet: Stylesheet::default(),
                            media_stack: Vec::new(),
                            order: 0,
                            at_rule_depth: self.at_rule_depth + 1,
                        };
                        loop {
                            sub.skip_ws();
                            match sub.peek().clone() {
                                Token::Eof => break,
                                _ => sub.qualified_rule(),
                            }
                        }
                        let base = self.order;
                        for (i, mut r) in sub.sheet.rules.into_iter().enumerate() {
                            r.order = base + i as u32;
                            r.media.extend(self.media_stack.iter().cloned());
                            self.sheet.rules.push(r);
                        }
                        self.order = base + sub.order + 1;
                        self.sheet.errors.extend(sub.sheet.errors);
                    }
                }
            }
            "font-face" | "keyframes" | "page" | "counter-style" | "property" | "container"
            | "scope" => {
                if has_block {
                    self.block_body();
                }
                self.sheet.skipped_at_rules.push(format!("@{lower}"));
            }
            _ => {
                if has_block {
                    self.block_body();
                }
                self.sheet.skipped_at_rules.push(format!("@{lower}"));
            }
        }
    }

    fn qualified_rule(&mut self) {
        // Gather the prelude up to `{`.
        let mut prelude = Vec::new();
        loop {
            match self.peek().clone() {
                Token::Eof => {
                    return;
                }
                Token::LBrace => {
                    self.pos += 1;
                    break;
                }
                t => {
                    prelude.push(t);
                    self.pos += 1;
                }
            }
        }
        let selectors = match parse_selector_list(&prelude) {
            Ok(s) => s,
            Err(e) => {
                self.error(format!("invalid selector: {}", e.message));
                self.block_body();
                return;
            }
        };
        let body = self.block_body();
        let mut block = DeclarationBlock::default();
        let errs: Vec<DeclError> = parse_declarations(&body[..body.len() - 1], self.ctx, &mut block);
        for e in errs {
            self.error(format!("{}: {}", e.property, e.message));
        }
        let order = self.order;
        self.order += 1;
        self.sheet.rules.push(StyleRule {
            selectors,
            declarations: block,
            media: self.media_stack.clone(),
            order,
        });
    }
}

fn supports_condition(prelude: &[Token], ctx: &LengthCtx) -> bool {
    // Accept `(prop: value)` where `prop` is a property we can parse.
    let idents: Vec<&Token> = prelude.iter().filter(|t| !t.is_whitespace()).collect();
    let Some(Token::LParen) = idents.first() else {
        return false;
    };
    let mut inner = Vec::new();
    let mut depth = 0;
    for t in prelude {
        match t {
            Token::LParen => {
                depth += 1;
            }
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ if depth > 0 => inner.push(t.clone()),
            _ => {}
        }
    }
    let mut block = DeclarationBlock::default();
    let errs = parse_declarations(&inner, ctx, &mut block);
    !block.decls.is_empty() && errs.is_empty()
}

/// Parse a `@media` prelude into a `MediaQuery`.
pub fn parse_media_query(tokens: &[Token]) -> MediaQuery {
    let ks: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
    if ks.is_empty() {
        return MediaQuery::All;
    }

    // Split on top-level commas into alternatives.
    let mut alternatives = Vec::new();
    let mut cur = Vec::new();
    for t in &ks {
        if matches!(t, Token::Comma) {
            alternatives.push(std::mem::take(&mut cur));
        } else {
            cur.push(*t);
        }
    }
    alternatives.push(cur);

    let mut queries: Vec<MediaQuery> = alternatives.into_iter().map(|alt| parse_single_media(&alt)).collect();
    if queries.len() == 1 {
        queries.remove(0)
    } else {
        MediaQuery::Or(queries)
    }
}

fn parse_single_media(tokens: &[&Token]) -> MediaQuery {
    if tokens.is_empty() {
        return MediaQuery::All;
    }
    let mut i = 0;
    let mut negate = false;
    if let Some(Token::Ident(id)) = tokens.first() {
        if id.eq_ignore_ascii_case("not") {
            negate = true;
            i = 1;
        } else if id.eq_ignore_ascii_case("only") {
            i = 1;
        }
    }
    // Media type.
    let mut conds = Vec::new();
    let mut base = MediaQuery::All;
    if let Some(Token::Ident(id)) = tokens.get(i) {
        let l = id.to_ascii_lowercase();
        match l.as_str() {
            "all" => {
                i += 1;
            }
            "screen" => {
                base = MediaQuery::Screen;
                i += 1;
            }
            "print" | "speech" | "tty" | "tv" | "projection" | "handheld" | "braille" | "embossed"
            | "aural" => {
                base = MediaQuery::Never;
                i += 1;
            }
            _ => {}
        }
    }
    // Conditions.
    while i < tokens.len() {
        match tokens[i] {
            Token::Ident(id) if id.eq_ignore_ascii_case("and") => i += 1,
            Token::LParen => {
                let mut depth = 0;
                let mut inner = Vec::new();
                while i < tokens.len() {
                    match tokens[i] {
                        Token::LParen => {
                            depth += 1;
                            if depth > 1 {
                                inner.push(tokens[i].clone());
                            }
                        }
                        Token::RParen => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                            inner.push(tokens[i].clone());
                        }
                        t => inner.push(t.clone()),
                    }
                    i += 1;
                }
                let refs: Vec<&Token> = inner.iter().collect();
                conds.push(parse_media_feature(&refs));
            }
            _ => i += 1,
        }
    }
    let mut q = if conds.is_empty() {
        base
    } else {
        let mut parts = vec![base];
        parts.extend(conds);
        MediaQuery::And(parts)
    };
    if negate {
        q = MediaQuery::Not(Box::new(q));
    }
    q
}

fn parse_media_feature(tokens: &[&Token]) -> MediaQuery {
    let non_ws: Vec<&&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
    if non_ws.len() == 1 {
        if let Token::Ident(name) = non_ws[0] {
            return feature(name, Length::Px(1.0));
        }
    }
    // name : value
    if non_ws.len() >= 3 && matches!(non_ws[1], Token::Colon) {
        if let Token::Ident(name) = non_ws[0] {
            let value_tok = non_ws[2];
            let ctx = LengthCtx::default();
            if let Some(l) = crate::values::parse_length(value_tok, &ctx) {
                return feature(name, l);
            }
            if let Token::Ident(v) = value_tok {
                // e.g. `orientation: landscape`, `prefers-color-scheme: dark`
                return MediaQuery::Feature { name: name.to_ascii_lowercase(), value: Length::Px(keyword_code(v)) };
            }
        }
    }
    // Range syntax `(400px <= width <= 700px)` is not supported (§5.2).
    MediaQuery::Unsupported("media range".into())
}

fn keyword_code(s: &str) -> f32 {
    // Encode keyword values as a numeric tag the evaluator recognises.
    match s.to_ascii_lowercase().as_str() {
        "landscape" => 1.0,
        "portrait" => 2.0,
        "dark" => 3.0,
        "light" => 4.0,
        "reduce" => 5.0,
        "no-preference" => 6.0,
        _ => 0.0,
    }
}

fn feature(name: &str, value: Length) -> MediaQuery {
    MediaQuery::Feature { name: name.to_ascii_lowercase(), value }
}

/// Parse a stylesheet with a default length context.
pub fn parse_stylesheet(src: &str, origin: Origin) -> Stylesheet {
    parse_stylesheet_with_ctx(src, origin, &LengthCtx::default())
}

pub fn parse_stylesheet_with_ctx(src: &str, origin: Origin, ctx: &LengthCtx) -> Stylesheet {
    let tokens = tokenize(src);
    let parser = Parser {
        tokens,
        pos: 0,
        ctx,
        sheet: Stylesheet::default(),
        media_stack: Vec::new(),
        order: 0,
        at_rule_depth: 0,
    };
    let mut sheet = parser.stylesheet();
    sheet.origin = Some(origin);
    sheet
}

/// The user-agent stylesheet, parsed once per process.
pub fn user_agent_stylesheet() -> Stylesheet {
    parse_stylesheet(include_str!("ua.css"), Origin::UserAgent)
}

/// Initial computed style, used as the root's parent.
pub fn initial_style() -> ComputedStyle {
    ComputedStyle::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(src: &str) -> Stylesheet {
        parse_stylesheet(src, Origin::Author)
    }

    #[test]
    fn single_rule() {
        let s = sheet("p { color: red; }");
        assert_eq!(s.rules.len(), 1);
        assert_eq!(s.rules[0].selectors.selectors.len(), 1);
        assert!(s.rules[0].declarations.get("color").is_some());
    }

    #[test]
    fn multiple_rules_and_selector_lists() {
        let s = sheet("h1, h2 { margin: 0 } p.x { color: blue }");
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.rules[0].selectors.selectors.len(), 2);
    }

    #[test]
    fn invalid_declaration_dropped_parsing_continues() {
        let s = sheet("p { color: banana; width: 10px; }");
        assert_eq!(s.rules.len(), 1);
        let d = &s.rules[0].declarations;
        assert!(d.get("color").is_none());
        assert!(d.get("width").is_some());
        assert!(!s.errors.is_empty());
    }

    #[test]
    fn invalid_selector_drops_rule() {
        let s = sheet("p { color: red } & .x { color: blue } p { width: 1px }");
        assert_eq!(s.rules.len(), 2, "the nesting rule is dropped, neighbours survive");
        assert!(s.errors.iter().any(|e| e.message.contains("invalid selector")));
    }

    #[test]
    fn comments_ignored() {
        let s = sheet("/* c */ p { color: red } /* d */");
        assert_eq!(s.rules.len(), 1);
    }

    #[test]
    fn media_query_parsed() {
        let s = sheet("@media screen and (min-width: 600px) { p { color: red } }");
        assert_eq!(s.rules.len(), 1);
        assert!(!s.rules[0].media.is_empty());
    }

    #[test]
    fn skipped_at_rules_are_recorded() {
        let s = sheet("@font-face { font-family: X; src: url(x.woff) } p { color: red }");
        assert_eq!(s.rules.len(), 1);
        assert!(s.skipped_at_rules.contains(&"@font-face".to_string()));
    }

    #[test]
    fn import_collected() {
        let s = sheet("@import url(other.css); p { color: red }");
        assert_eq!(s.imports, vec!["other.css".to_string()]);
        assert_eq!(s.rules.len(), 1);
    }

    #[test]
    fn supports_true_for_supported_property() {
        let s = sheet("@supports (color: red) { p { color: green } }");
        assert_eq!(s.rules.len(), 1, "color is supported, so the block applies");
    }

    #[test]
    fn supports_false_for_unsupported() {
        let s = sheet("@supports (column-count: 2) { p { color: green } }");
        assert_eq!(s.rules.len(), 0, "column-count is unsupported");
    }

    #[test]
    fn source_order_assigned() {
        let s = sheet("p { color: red } p { color: blue }");
        assert!(s.rules[0].order < s.rules[1].order);
    }

    #[test]
    fn ua_sheet_parses_without_errors() {
        let s = user_agent_stylesheet();
        assert!(s.rules.len() > 20, "got {} rules", s.rules.len());
        assert!(
            s.errors.is_empty(),
            "UA sheet must parse cleanly: {:?}",
            s.errors.iter().take(5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ua_sheet_has_expected_rules() {
        let s = user_agent_stylesheet();
        // A UA sheet legitimately emits several rules for one element: `h1` is
        // part of the shared `display: block` list *and* has its own headings
        // rule. Select the rule whose selector list is exactly `h1` — matching
        // "any selector mentions h1" hits the block list (whose only declaration
        // is `display`), which is the false failure this test guards against.
        fn selected_by_tag(r: &StyleRule, tag: &str) -> bool {
            r.selectors.selectors.len() == 1
                && match r.selectors.selectors[0].compounds.as_slice() {
                    [c] => match c.simples.as_slice() {
                        [crate::selectors::SimpleSelector::Type(t)] => t.name.as_deref() == Some(tag),
                        _ => false,
                    },
                    _ => false,
                }
        }
        let h1 = s.rules.iter().find(|r| selected_by_tag(r, "h1"));
        assert!(h1.is_some(), "no rule selected by exactly `h1` in the UA sheet");
        assert!(h1.unwrap().declarations.get("font-size").is_some(), "h1 must set font-size");
        let body = s.rules.iter().find(|r| selected_by_tag(r, "body"));
        assert!(body.is_some(), "no rule selected by exactly `body` in the UA sheet");
        assert!(body.unwrap().declarations.get("margin-top").is_some(), "body must set a margin");
    }

    #[test]
    fn important_parsed_in_sheet() {
        let s = sheet("p { color: red !important }");
        assert!(s.rules[0].declarations.get("color").unwrap().important);
    }

    #[test]
    fn media_query_all() {
        assert_eq!(parse_media_query(&tokenize("")[..0]), MediaQuery::All);
    }

    #[test]
    fn media_screen_never_matches_print() {
        let q = parse_media_query(&{
            let t = tokenize("print");
            t[..t.len() - 1].to_vec()
        });
        assert_eq!(q, MediaQuery::Never);
    }
}