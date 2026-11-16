//! Selector parsing: a token stream → `SelectorList`.
//!
//! Parsing is permissive in the same way CSS is: an invalid selector in a list
//! invalidates the *whole* list (so `a, ::bogus{}` drops the rule), which the
//! caller enforces by treating `Err` as "drop this rule". Unsupported but
//! syntactically valid pseudo-classes are kept as `Unsupported` so the report
//! can name them.

use crate::selectors::{
    AttributeSelector, AttrOp, Combinator, CompoundSelector, PseudoClass, Selector, SelectorList,
    SimpleSelector, Specificity, TypeSelector,
};
use crate::tokenizer::token::Token;

/// Parse errors; the caller drops the rule and reports the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorError {
    pub message: String,
}

fn err(msg: impl Into<String>) -> SelectorError {
    SelectorError { message: msg.into() }
}

/// Maximum nesting of selector-list arguments (`:not(`, `:is(`, `:where(`,
/// `:has(`, and the `of S` list of `:nth-child`).
///
/// `pseudo()` builds each nested list with a fresh `Parser` that recursively
/// calls `selector_list`, so input like `:not(:not(:not(…)))` nests one stack
/// frame per level. A hostile stylesheet can therefore overflow the stack,
/// which on Windows is `STATUS_STACK_BUFFER_OVERRUN` — an uncatchable process
/// abort, not a panic. Browsers accept deeper nesting than this; the cap is
/// chosen so real stylesheets (which nest a handful of levels) are never
/// affected, and is recorded as a parse error rather than panicking.
pub const MAX_SELECTOR_NESTING: u32 = 64;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Current selector-list nesting depth; checked against
    /// `MAX_SELECTOR_NESTING` on entry to every nested list.
    depth: u32,
}

impl Parser {
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

    /// Is the next token something that can start a compound?
    fn at_compound_start(&self) -> bool {
        matches!(
            self.peek(),
            Token::Ident(_)
                | Token::Hash { .. }
                | Token::Delim('*' | '.')
                | Token::Colon
                | Token::LBracket
                | Token::Delim('|')
        )
    }

    fn selector_list(&mut self, stop_at_paren: bool) -> Result<SelectorList, SelectorError> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            if matches!(self.peek(), Token::Eof) {
                break;
            }
            if stop_at_paren && matches!(self.peek(), Token::RParen) {
                break;
            }
            out.push(self.complex_selector()?);
            self.skip_ws();
            match self.peek() {
                Token::Comma => {
                    self.pos += 1;
                }
                Token::Eof => break,
                Token::RParen if stop_at_paren => break,
                other => return Err(err(format!("unexpected {} after selector", other.name()))),
            }
        }
        Ok(SelectorList { selectors: out })
    }

    fn complex_selector(&mut self) -> Result<Selector, SelectorError> {
        // Collect compounds left-to-right with the combinator between them,
        // then reverse so matching walks subject-first.
        let mut compounds = vec![self.compound()?];
        let mut combinators = Vec::new();
        loop {
            let start = self.pos;
            let mut saw_ws = false;
            while matches!(self.peek(), Token::Whitespace) {
                self.pos += 1;
                saw_ws = true;
            }
            let combinator = match self.peek() {
                Token::Delim('>') => {
                    self.pos += 1;
                    Combinator::Child
                }
                Token::Delim('+') => {
                    self.pos += 1;
                    Combinator::AdjacentSibling
                }
                Token::Delim('~') => {
                    self.pos += 1;
                    Combinator::GeneralSibling
                }
                Token::Delim('|') if matches!(self.tokens.get(self.pos + 1), Some(Token::Delim('|'))) => {
                    self.pos += 2;
                    Combinator::Column
                }
                _ if saw_ws && self.at_compound_start() => Combinator::Descendant,
                _ => {
                    self.pos = start;
                    break;
                }
            };
            self.skip_ws();
            if !self.at_compound_start() {
                self.pos = start;
                break;
            }
            combinators.push(combinator);
            compounds.push(self.compound()?);
        }

        let specificity = compounds
            .iter()
            .fold(Specificity::ZERO, |acc, c| acc.add(c.specificity()));

        compounds.reverse();
        combinators.reverse();
        Ok(Selector { compounds, combinators, specificity })
    }

    fn compound(&mut self) -> Result<CompoundSelector, SelectorError> {
        let mut simples = Vec::new();

        // An optional type selector (possibly namespaced) comes first.
        if let Some(ts) = self.type_selector()? {
            simples.push(SimpleSelector::Type(ts));
        }

        loop {
            match self.peek().clone() {
                Token::Hash { value, is_id } => {
                    self.pos += 1;
                    if !is_id {
                        return Err(err("hash is not an id"));
                    }
                    simples.push(SimpleSelector::Id(value));
                }
                Token::Delim('.') => {
                    self.pos += 1;
                    match self.bump() {
                        Token::Ident(name) => simples.push(SimpleSelector::Class(name)),
                        other => return Err(err(format!("expected class name, got {}", other.name()))),
                    }
                }
                Token::LBracket => {
                    self.pos += 1;
                    simples.push(SimpleSelector::Attribute(self.attribute_selector()?));
                }
                Token::Colon => {
                    self.pos += 1;
                    let (simple, is_element) = self.pseudo()?;
                    if is_element {
                        // `pseudo()` returns the element name in the `Unsupported`
                        // slot for pseudo-elements; unwrap it here.
                        if let PseudoClass::Unsupported(name) = simple {
                            simples.push(SimpleSelector::PseudoElement(name));
                        }
                    } else {
                        simples.push(SimpleSelector::PseudoClass(simple));
                    }
                }
                Token::Delim('&') => {
                    // Nesting is unsupported (§15): drop the rule, visibly.
                    return Err(err("nesting selector `&` is not supported"));
                }
                _ => break,
            }
        }

        if simples.is_empty() {
            return Err(err("empty compound selector"));
        }
        Ok(CompoundSelector { simples })
    }

    fn type_selector(&mut self) -> Result<Option<TypeSelector>, SelectorError> {
        // `*`, `ident`, `ns|ident`, `*|ident`, `|ident`, `ns|*`
        let save = self.pos;
        let mut namespace_seen = false;
        let mut name: Option<Option<String>> = None; // Some(None) = `*`

        match self.peek().clone() {
            Token::Ident(n) => {
                self.pos += 1;
                name = Some(Some(n));
            }
            Token::Delim('*') => {
                self.pos += 1;
                name = Some(None);
            }
            Token::Delim('|') => {
                self.pos += 1;
                namespace_seen = true;
            }
            _ => return Ok(None),
        }

        // A namespace prefix.
        if matches!(self.peek(), Token::Delim('|')) && !matches!(self.peek(), Token::Delim('|')) {
            // handled below
        }
        if matches!(self.peek(), Token::Delim('|')) {
            // `name` was actually a namespace prefix (or `*`/empty).
            if namespace_seen {
                // `|ident` form already consumed the pipe.
            } else {
                self.pos += 1;
            }
            match self.peek().clone() {
                Token::Ident(n) => {
                    self.pos += 1;
                    name = Some(Some(n));
                }
                Token::Delim('*') => {
                    self.pos += 1;
                    name = Some(None);
                }
                other => {
                    self.pos = save;
                    return Err(err(format!("expected type after namespace, got {}", other.name())));
                }
            }
        }

        Ok(name.map(|n| TypeSelector { name: n }))
    }

    fn attribute_selector(&mut self) -> Result<AttributeSelector, SelectorError> {
        self.skip_ws();
        let name = match self.bump() {
            Token::Ident(n) => n,
            other => return Err(err(format!("expected attribute name, got {}", other.name()))),
        };
        self.skip_ws();
        let op = match self.peek().clone() {
            Token::RBracket => {
                self.pos += 1;
                return Ok(AttributeSelector { name, op: AttrOp::Exists, value: None, case_insensitive: false });
            }
            Token::Delim('=') => {
                self.pos += 1;
                AttrOp::Equals
            }
            Token::Delim('~') => {
                self.pos += 1;
                self.expect_delim('=')?;
                AttrOp::Includes
            }
            Token::Delim('|') => {
                self.pos += 1;
                self.expect_delim('=')?;
                AttrOp::DashMatch
            }
            Token::Delim('^') => {
                self.pos += 1;
                self.expect_delim('=')?;
                AttrOp::Prefix
            }
            Token::Delim('$') => {
                self.pos += 1;
                self.expect_delim('=')?;
                AttrOp::Suffix
            }
            Token::Delim('*') => {
                self.pos += 1;
                self.expect_delim('=')?;
                AttrOp::Substring
            }
            other => return Err(err(format!("unexpected {} in attribute selector", other.name()))),
        };
        self.skip_ws();
        let value = match self.bump() {
            Token::Ident(v) | Token::String(v) => v,
            other => return Err(err(format!("expected attribute value, got {}", other.name()))),
        };
        self.skip_ws();
        let mut case_insensitive = false;
        if let Token::Ident(flag) = self.peek().clone() {
            if flag.eq_ignore_ascii_case("i") || flag.eq_ignore_ascii_case("s") {
                case_insensitive = flag.eq_ignore_ascii_case("i");
                self.pos += 1;
                self.skip_ws();
            }
        }
        match self.bump() {
            Token::RBracket => {}
            other => return Err(err(format!("expected ] , got {}", other.name()))),
        }
        Ok(AttributeSelector { name, op, value: Some(value), case_insensitive })
    }

    fn expect_delim(&mut self, c: char) -> Result<(), SelectorError> {
        match self.bump() {
            Token::Delim(d) if d == c => Ok(()),
            other => Err(err(format!("expected {c}, got {}", other.name()))),
        }
    }

    /// Returns the pseudo and whether it is a pseudo-element (`::x`).
    fn pseudo(&mut self) -> Result<(PseudoClass, bool), SelectorError> {
        // `::name` is an element; `:name` is a class, except for the four
        // legacy pseudo-elements that may be written with a single colon.
        if matches!(self.peek(), Token::Colon) {
            self.pos += 1;
            let name = match self.bump() {
                Token::Ident(n) => n,
                other => return Err(err(format!("expected pseudo name, got {}", other.name()))),
            };
            return Ok((PseudoClass::Unsupported(format!("::{name}")), true));
        }
        let (name, args) = match self.bump() {
            Token::Ident(n) => (n, None),
            Token::Function(n) => {
                let args = self.consume_function_args();
                (n, Some(args))
            }
            other => return Err(err(format!("expected pseudo-class, got {}", other.name()))),
        };
        let name_lower = name.to_ascii_lowercase();

        if matches!(name_lower.as_str(), "before" | "after" | "first-line" | "first-letter") && args.is_none() {
            return Ok((PseudoClass::Unsupported(format!("::{name_lower}")), true));
        }

        let pc = match name_lower.as_str() {
            "first-child" => PseudoClass::FirstChild,
            "last-child" => PseudoClass::LastChild,
            "only-child" => PseudoClass::OnlyChild,
            "first-of-type" => PseudoClass::FirstOfType,
            "last-of-type" => PseudoClass::LastOfType,
            "only-of-type" => PseudoClass::OnlyOfType,
            "root" => PseudoClass::Root,
            "empty" => PseudoClass::Empty,
            "hover" => PseudoClass::Hover,
            "focus" => PseudoClass::Focus,
            "focus-visible" => PseudoClass::FocusVisible,
            "focus-within" => PseudoClass::FocusWithin,
            "active" => PseudoClass::Active,
            "visited" => PseudoClass::Visited,
            "link" => PseudoClass::Link,
            "checked" => PseudoClass::Checked,
            "disabled" => PseudoClass::Disabled,
            "enabled" => PseudoClass::Enabled,
            "nth-child" => {
                let (a, b, of) = self.parse_nth(args)?;
                PseudoClass::NthChild { a, b, of }
            }
            "nth-last-child" => {
                let (a, b, _) = self.parse_nth(args)?;
                PseudoClass::NthLastChild { a, b }
            }
            "nth-of-type" => {
                let (a, b, _) = self.parse_nth(args)?;
                PseudoClass::NthOfType { a, b }
            }
            "not" => PseudoClass::Not(Box::new(self.nested_list(args)?)),
            "is" | "matches" | "any" => PseudoClass::Is(Box::new(self.nested_list(args)?)),
            "where" => PseudoClass::Where(Box::new(self.nested_list(args)?)),
            "has" => PseudoClass::Has(Box::new(self.nested_list(args)?)),
            other => PseudoClass::Unsupported(other.to_string()),
        };
        Ok((pc, false))
    }

    /// Parse a nested selector-list argument (`:not(...)`, `:is(...)`, …),
    /// enforcing `MAX_SELECTOR_NESTING`.
    fn nested_list(&mut self, args: Option<Vec<Token>>) -> Result<SelectorList, SelectorError> {
        let depth = self.depth + 1;
        if depth > MAX_SELECTOR_NESTING {
            return Err(err(format!("selector nesting deeper than {MAX_SELECTOR_NESTING}")));
        }
        self.parse_selector_list_arg(args.unwrap_or_else(|| vec![Token::Eof]), depth)
    }

    /// Consume tokens up to the matching `)`, accounting for nesting.
    fn consume_function_args(&mut self) -> Vec<Token> {
        let mut depth = 1;
        let mut out = Vec::new();
        loop {
            match self.bump() {
                Token::Eof => break,
                Token::Function(_) | Token::LParen => {
                    depth += 1;
                    out.push(self.tokens[self.pos - 1].clone());
                }
                Token::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(Token::RParen);
                }
                t => out.push(t),
            }
        }
        out.push(Token::Eof);
        out
    }

    fn sub_parser(args: Vec<Token>, depth: u32) -> Parser {
        Parser { tokens: args, pos: 0, depth }
    }

    fn parse_selector_list_arg(&mut self, args: Vec<Token>, depth: u32) -> Result<SelectorList, SelectorError> {
        let mut p = Self::sub_parser(args, depth);
        p.selector_list(false)
    }

    fn parse_nth(&mut self, args: Option<Vec<Token>>) -> Result<(i32, i32, Option<Box<Selector>>), SelectorError> {
        let args = args.unwrap_or_else(|| vec![Token::Eof]);
        let depth = self.depth + 1;
        if depth > MAX_SELECTOR_NESTING {
            return Err(err(format!("selector nesting deeper than {MAX_SELECTOR_NESTING}")));
        }
        let mut p = Self::sub_parser(args, depth);
        // The grammar is `odd | even | <integer> | <n-dimension> | ...`.
        // We accept: odd, even, An+B, B, An, n, -n+3, with optional `of S`.
        p.skip_ws();
        let (a, b) = p.parse_an_plus_b()?;
        p.skip_ws();
        let of = if let Token::Ident(id) = p.peek().clone() {
            if id.eq_ignore_ascii_case("of") {
                p.pos += 1;
                let list = p.selector_list(false)?;
                if list.selectors.len() != 1 {
                    return Err(err("`:nth-child(... of S)` supports exactly one selector"));
                }
                Some(Box::new(list.selectors.into_iter().next().expect("len checked")))
            } else {
                None
            }
        } else {
            None
        };
        Ok((a, b, of))
    }

    /// Optional `+B` / `-B` following an `An` term, with an optional whitespace
    /// between the sign and the integer (`2n + 3`, `-n +3`). A bare signed
    /// number (`-n+3` tokenizes as `Ident("-n"), Number(3)` with the sign folded
    /// into the number) is accepted too.
    fn parse_b_tail(&mut self) -> Result<i32, SelectorError> {
        self.skip_ws();
        match self.peek().clone() {
            Token::Number { value, is_integer } if is_integer => {
                self.pos += 1;
                Ok(value as i32)
            }
            Token::Delim(sign @ ('+' | '-')) => {
                self.pos += 1;
                self.skip_ws();
                match self.bump() {
                    Token::Number { value, is_integer } if is_integer => {
                        Ok(if sign == '-' { -(value as i32) } else { value as i32 })
                    }
                    other => Err(err(format!("expected integer after {sign}, got {}", other.name()))),
                }
            }
            _ => Ok(0),
        }
    }

    fn parse_an_plus_b(&mut self) -> Result<(i32, i32), SelectorError> {
        match self.bump() {
            Token::Ident(word) => {
                let w = word.to_ascii_lowercase();
                let a = match w.as_str() {
                    "odd" => return Ok((2, 1)),
                    "even" => return Ok((2, 0)),
                    "n" => 1,
                    "-n" => -1,
                    _ => match w.strip_suffix('n') {
                        // `2n`, `-2n`, `+2n`.
                        Some(rest) => match rest {
                            "" | "+" => 1,
                            "-" => -1,
                            _ => match rest.parse() {
                                Ok(a) => a,
                                Err(_) => return Err(err(format!("invalid nth argument `{word}`"))),
                            },
                        },
                        None => return Err(err(format!("invalid nth argument `{word}`"))),
                    },
                };
                // `-n+3` / `2n + 3`: the `An` term is a separate ident token from
                // the `B` term, so consume the tail here instead of returning 0.
                let b = self.parse_b_tail()?;
                Ok((a, b))
            }
            Token::Number { value, is_integer } => {
                if !is_integer {
                    return Err(err("nth argument must be an integer"));
                }
                let b = value as i32;
                // Could be `2n`-style where `n` is a separate ident token
                // (`2n` tokenizes as Dimension {2, "n"}).
                Ok((0, b))
            }
            Token::Dimension { value, unit } => {
                if !unit.eq_ignore_ascii_case("n") {
                    return Err(err("expected `n` unit in nth argument"));
                }
                let a = value as i32;
                let b = self.parse_b_tail()?;
                Ok((a, b))
            }
            other => Err(err(format!("invalid nth argument, got {}", other.name()))),
        }
    }
}

/// Parse a selector list from a token stream.
pub fn parse_selector_list(tokens: &[Token]) -> Result<SelectorList, SelectorError> {
    let mut p = Parser { tokens: tokens.to_vec(), pos: 0, depth: 0 };
    let list = p.selector_list(false)?;
    if list.is_empty() {
        return Err(err("empty selector list"));
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::state::tokenize;

    fn one(src: &str) -> Selector {
        let list = parse_selector_list(&tokenize(src)).unwrap_or_else(|e| panic!("{src}: {}", e.message));
        assert_eq!(list.selectors.len(), 1, "{src}");
        list.selectors.into_iter().next().unwrap()
    }

    fn kinds(sel: &Selector) -> Vec<Vec<String>> {
        sel.compounds
            .iter()
            .map(|c| {
                c.simples
                    .iter()
                    .map(|s| match s {
                        SimpleSelector::Type(t) => format!("type:{}", t.name.clone().unwrap_or_else(|| "*".into())),
                        SimpleSelector::Id(i) => format!("id:{i}"),
                        SimpleSelector::Class(c) => format!("class:{c}"),
                        SimpleSelector::Attribute(a) => format!("attr:{}", a.name),
                        SimpleSelector::PseudoClass(p) => format!("pseudo:{p:?}"),
                        SimpleSelector::PseudoElement(p) => format!("element:{p}"),
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn type_selector() {
        let s = one("div");
        assert_eq!(kinds(&s), vec![vec!["type:div"]]);
        assert_eq!(s.specificity, Specificity::new(0, 0, 1));
    }

    #[test]
    fn compound_selector() {
        let s = one("div.a#b");
        assert_eq!(kinds(&s), vec![vec!["type:div", "class:a", "id:b"]]);
        assert_eq!(s.specificity, Specificity::new(1, 1, 1));
    }

    #[test]
    fn descendant_selector_is_reversed() {
        let s = one("div p");
        assert_eq!(kinds(&s), vec![vec!["type:p"], vec!["type:div"]]);
        assert_eq!(s.combinators, vec![Combinator::Descendant]);
    }

    #[test]
    fn child_combinator() {
        let s = one("ul > li");
        assert_eq!(kinds(&s), vec![vec!["type:li"], vec!["type:ul"]]);
        assert_eq!(s.combinators, vec![Combinator::Child]);
    }

    #[test]
    fn sibling_combinators() {
        let s = one("h1 + p");
        assert_eq!(s.combinators, vec![Combinator::AdjacentSibling]);
        let s = one("h1 ~ p");
        assert_eq!(s.combinators, vec![Combinator::GeneralSibling]);
    }

    #[test]
    fn no_space_means_no_combinator() {
        let s = one("a.b");
        assert!(s.combinators.is_empty());
    }

    #[test]
    fn attribute_selectors() {
        let s = one("a[href]");
        assert_eq!(kinds(&s), vec![vec!["type:a", "attr:href"]]);
        let s = one("a[href=\"x\"]");
        match &s.compounds[0].simples[1] {
            SimpleSelector::Attribute(a) => {
                assert_eq!(a.op, AttrOp::Equals);
                assert_eq!(a.value.as_deref(), Some("x"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn attribute_operators() {
        for (src, op) in [
            ("[a~=b]", AttrOp::Includes),
            ("[a|=b]", AttrOp::DashMatch),
            ("[a^=b]", AttrOp::Prefix),
            ("[a$=b]", AttrOp::Suffix),
            ("[a*=b]", AttrOp::Substring),
        ] {
            let s = one(src);
            match &s.compounds[0].simples[0] {
                SimpleSelector::Attribute(a) => assert_eq!(a.op, op, "{src}"),
                _ => panic!("{src}"),
            }
        }
    }

    #[test]
    fn universal() {
        let s = one("*");
        assert_eq!(kinds(&s), vec![vec!["type:*"]]);
        assert_eq!(s.specificity, Specificity::ZERO);
    }

    #[test]
    fn pseudo_classes() {
        let s = one("p:first-child");
        assert!(matches!(&s.compounds[0].simples[1], SimpleSelector::PseudoClass(PseudoClass::FirstChild)));
    }

    #[test]
    fn pseudo_element() {
        let s = one("p::before");
        assert!(matches!(&s.compounds[0].simples[1], SimpleSelector::PseudoElement(_)));
        assert_eq!(s.specificity, Specificity::new(0, 0, 2));
    }

    #[test]
    fn legacy_pseudo_element_single_colon() {
        let s = one("p:before");
        assert!(matches!(&s.compounds[0].simples[1], SimpleSelector::PseudoElement(_)));
    }

    #[test]
    fn nth_child_odd() {
        let s = one("li:nth-child(odd)");
        match &s.compounds[0].simples[1] {
            SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => {
                assert_eq!((*a, *b), (2, 1));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nth_child_an_plus_b() {
        for (src, want) in [
            ("li:nth-child(3n+2)", (3, 2)),
            ("li:nth-child(-n+3)", (-1, 3)),
            ("li:nth-child(5)", (0, 5)),
            // Spacing variants: the `B` term may follow with whitespace, and
            // the sign may be separate from the integer.
            ("li:nth-child(2n + 3)", (2, 3)),
            ("li:nth-child(2n - 3)", (2, -3)),
            ("li:nth-child(n+3)", (1, 3)),
            ("li:nth-child(-2n+1)", (-2, 1)),
        ] {
            let s = one(src);
            match &s.compounds[0].simples[1] {
                SimpleSelector::PseudoClass(PseudoClass::NthChild { a, b, .. }) => {
                    assert_eq!((*a, *b), want, "{src}");
                }
                other => panic!("{src}: {other:?}"),
            }
        }
    }

    #[test]
    fn not_and_is() {
        let s = one("a:not(.x)");
        match &s.compounds[0].simples[1] {
            SimpleSelector::PseudoClass(PseudoClass::Not(l)) => assert_eq!(l.selectors.len(), 1),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.specificity, Specificity::new(0, 1, 1), "type a (0,0,1) + not(.x) (0,1,0)");
    }

    #[test]
    fn selector_list_parses() {
        let list = parse_selector_list(&tokenize("h1, h2 , h3")).unwrap();
        assert_eq!(list.selectors.len(), 3);
    }

    #[test]
    fn empty_list_is_error() {
        assert!(parse_selector_list(&tokenize("")).is_err());
    }

    #[test]
    fn nesting_is_rejected() {
        assert!(parse_selector_list(&tokenize("& .x")).is_err());
    }

    #[test]
    fn unsupported_pseudo_is_kept() {
        let s = one("a:has(.x)");
        match &s.compounds[0].simples[1] {
            SimpleSelector::PseudoClass(PseudoClass::Has(l)) => assert_eq!(l.selectors.len(), 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pseudo_class_specificity_sums() {
        let s = one("li:nth-child(2n+1)");
        assert_eq!(s.specificity, Specificity::new(0, 1, 1));
    }
}