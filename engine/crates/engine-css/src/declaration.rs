//! Declaration parsing: one longhand or shorthand at a time, applied onto a
//! `ComputedStyle`-shaped declaration set.
//!
//! Shorthands expand here, at parse time, so the cascade only ever sees
//! longhands (§5.3). An invalid declaration returns `None` and is dropped; the
//! caller resumes at the next `;`.

use engine_core::{Length, Rgba8};

use crate::properties::{BorderStyle, Display, LineHeight};
use crate::tokenizer::token::Token;
use crate::values::{finite_f32, parse_color, parse_length, LengthCtx};

/// One longhand declaration: property name (lowercased) plus its parsed value.
#[derive(Clone, Debug, PartialEq)]
pub enum DeclaredValue {
    Length(Length),
    Color(Rgba8),
    Number(f32),
    Ident(String),
    /// A CSS-wide keyword that the cascade resolves.
    WideKeyword(WideKeyword),
    /// A list of family names.
    StringList(Vec<String>),
    Raw(Vec<Token>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WideKeyword {
    Inherit,
    Initial,
    Unset,
    Revert,
}

/// A parsed declaration block: longhands in source order, with `!important`.
#[derive(Clone, Debug, Default)]
pub struct DeclarationBlock {
    pub decls: Vec<Declaration>,
}

#[derive(Clone, Debug)]
pub struct Declaration {
    pub property: String,
    pub value: DeclaredValue,
    pub important: bool,
}

impl DeclarationBlock {
    /// Look up the last declaration for `property` (source order already means
    /// later wins within a block).
    pub fn get(&self, property: &str) -> Option<&Declaration> {
        self.decls.iter().rev().find(|d| d.property == property)
    }
}

/// Parse a comma-split list of declarations.
pub fn parse_declarations(tokens: &[Token], ctx: &LengthCtx, out: &mut DeclarationBlock) -> Vec<DeclError> {
    let mut errors = Vec::new();
    for chunk in split_on_semicolons(tokens) {
        if chunk.iter().all(|t| t.is_whitespace()) {
            continue;
        }
        match parse_one_declaration(chunk, ctx) {
            Ok(decls) => out.decls.extend(decls),
            Err(e) => errors.push(e),
        }
    }
    errors
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclError {
    pub property: String,
    pub message: String,
}

fn split_on_semicolons(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::LParen | Token::LBracket | Token::LBrace | Token::Function(_) => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => depth -= 1,
            Token::Semicolon if depth == 0 => {
                out.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        out.push(&tokens[start..]);
    }
    out
}

/// Values of one declaration (after the colon and before `!important`).
fn value_tokens(tokens: &[Token]) -> (&[Token], bool) {
    // Find a top-level `!important`.
    let mut depth = 0i32;
    let mut bang: Option<usize> = None;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::LParen | Token::LBracket | Token::Function(_) => depth += 1,
            Token::RParen | Token::RBracket => depth -= 1,
            Token::Delim('!') if depth == 0 => bang = Some(i),
            _ => {}
        }
    }
    match bang {
        Some(i) => {
            let rest: Vec<&Token> = tokens[i + 1..].iter().filter(|t| !t.is_whitespace()).collect();
            let important = rest.len() == 1 && rest[0].as_ident().is_some_and(|s| s.eq_ignore_ascii_case("important"));
            (&tokens[..i], important)
        }
        None => (tokens, false),
    }
}

fn ks(s: &[Token]) -> Vec<&Token> {
    s.iter().filter(|t| !t.is_whitespace()).collect()
}

fn wide_keyword(toks: &[&Token]) -> Option<WideKeyword> {
    if toks.len() != 1 {
        return None;
    }
    let id = toks[0].as_ident()?;
    Some(match id.to_ascii_lowercase().as_str() {
        "inherit" => WideKeyword::Inherit,
        "initial" => WideKeyword::Initial,
        "unset" => WideKeyword::Unset,
        "revert" => WideKeyword::Revert,
        _ => return None,
    })
}

fn parse_one_declaration(tokens: &[Token], ctx: &LengthCtx) -> Result<Vec<Declaration>, DeclError> {
    let toks = ks(tokens);
    if toks.is_empty() {
        return Err(DeclError { property: String::new(), message: "empty declaration".into() });
    }
    let property = match toks[0] {
        Token::Ident(p) => p.to_ascii_lowercase(),
        other => {
            return Err(DeclError {
                property: String::new(),
                message: format!("expected property name, got {}", other.name()),
            })
        }
    };
    if !matches!(toks.get(1), Some(Token::Colon)) {
        return Err(DeclError { property, message: "expected ':'".into() });
    }
    let value_slice = &tokens[position_after_colon(tokens)..];
    let (raw_value, important) = value_tokens(value_slice);
    let v = ks(raw_value);
    if v.is_empty() {
        return Err(DeclError { property, message: "empty value".into() });
    }

    let mut out = Vec::new();
    // CSS-wide keywords apply to every longhand of a shorthand.
    if let Some(kw) = wide_keyword(&v) {
        for name in longhands_of(&property) {
            out.push(Declaration { property: name.to_string(), value: DeclaredValue::WideKeyword(kw), important });
        }
        return Ok(out);
    }

    let mut push = |name: &str, value: DeclaredValue| {
        out.push(Declaration { property: name.to_string(), value, important });
    };

    match property.as_str() {
        // --- Display / box generation ---
        "display" => {
            let id = v[0].as_ident().ok_or_else(|| bad(&property, "expected keyword"))?;
            let d = parse_display(id).ok_or_else(|| bad(&property, format!("unknown display `{id}`")))?;
            push("display", DeclaredValue::Ident(display_name(d).into()));
        }
        "position" => push("position", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "float" => push("float", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "clear" => push("clear", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "visibility" => push("visibility", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "overflow" => {
            let a = one_ident(&v, &property)?.to_ascii_lowercase();
            push("overflow-x", DeclaredValue::Ident(a.clone()));
            push("overflow-y", DeclaredValue::Ident(a));
        }
        "overflow-x" => push("overflow-x", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "overflow-y" => push("overflow-y", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "z-index" => match v[0] {
            Token::Number { value, is_integer } if *is_integer => {
                // `as i32` saturates, so `z-index: 9…9` (400 digits) would
                // silently become `i32::MAX` and pin the box above everything.
                // CSS integers are bounded by the implementation; reject.
                let n = finite_f32(*value)
                    .filter(|n| n.abs() <= 16_777_216.0)
                    .ok_or_else(|| bad(&property, "z-index out of range"))?;
                push("z-index", DeclaredValue::Number(n))
            }
            Token::Ident(i) if i.eq_ignore_ascii_case("auto") => push("z-index", DeclaredValue::Ident("auto".into())),
            _ => return Err(bad(&property, "expected integer or auto")),
        },

        // --- Box model ---
        "width" | "height" | "min-width" | "max-width" | "min-height" | "max-height" | "top"
        | "right" | "bottom" | "left" | "flex-basis" | "gap" | "row-gap" | "column-gap" => {
            let l = parse_length(v[0], ctx).ok_or_else(|| bad(&property, "invalid length"))?;
            if property == "gap" {
                push("row-gap", DeclaredValue::Length(l));
                push("column-gap", DeclaredValue::Length(l));
            } else if property == "min-width" && matches!(l, Length::Auto) {
                // `min-width: auto` is the initial value.
                push(&property, DeclaredValue::Ident("auto".into()));
            } else {
                push(&property, DeclaredValue::Length(l));
            }
        }
        "box-sizing" => push("box-sizing", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "margin" | "padding" => {
            let sides = expand_four(&v, ctx).ok_or_else(|| bad(&property, "expected 1-4 lengths"))?;
            let names = if property == "margin" {
                ["margin-top", "margin-right", "margin-bottom", "margin-left"]
            } else {
                ["padding-top", "padding-right", "padding-bottom", "padding-left"]
            };
            for (name, val) in names.iter().zip(sides.iter()) {
                push(name, DeclaredValue::Length(*val));
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" => {
            let l = parse_length(v[0], ctx).ok_or_else(|| bad(&property, "invalid length"))?;

            if property.starts_with("padding") && matches!(l, Length::Auto) {
                return Err(bad(&property, "padding cannot be auto"));
            }
            push(&property, DeclaredValue::Length(l));
        }

        // --- Borders ---
        "border" => expand_border(&v, ctx, None, &mut push)?,
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let side = property.strip_prefix("border-").unwrap_or_default().to_string();
            expand_border(&v, ctx, Some(&side), &mut push)?;
        }
        "border-width" => {
            let sides = expand_four_nonauto(&v, ctx).ok_or_else(|| bad(&property, "expected widths"))?;
            for (name, val) in ["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"]
                .iter()
                .zip(sides.iter())
            {
                push(name, DeclaredValue::Length(*val));
            }
        }
        "border-style" => {
            let s = expand_four_idents(&v, &property)?;
            for (name, id) in ["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"]
                .iter()
                .zip(s.iter())
            {
                push(name, DeclaredValue::Ident(id.clone()));
            }
        }
        "border-color" => {
            let c = expand_four_colors(&v)?;
            for (name, col) in ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"]
                .iter()
                .zip(c.iter())
            {
                push(name, DeclaredValue::Color(*col));
            }
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            let l = parse_length(v[0], ctx).ok_or_else(|| bad(&property, "invalid width"))?;
            push(&property, DeclaredValue::Length(l));
        }
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            push(&property, DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase()));
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            let c = parse_color(&v.iter().map(|t| (*t).clone()).collect::<Vec<_>>())
                .ok_or_else(|| bad(&property, "invalid color"))?;
            push(&property, DeclaredValue::Color(c));
        }
        "border-radius" => {
            let sides = expand_four_nonauto(&v, ctx).ok_or_else(|| bad(&property, "expected radii"))?;
            for (name, val) in
                ["border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius", "border-bottom-left-radius"]
                    .iter()
                    .zip(sides.iter())
            {
                push(name, DeclaredValue::Length(*val));
            }
        }
        "outline" => {
            // We only honour the width and colour of an outline.
            let mut width = Length::Px(1.0);
            let mut color = Rgba8::BLACK;
            for t in &v {
                if let Some(l) = parse_length(t, ctx) {
                    if !matches!(l, Length::Auto) {
                        width = l;
                    }
                } else if let Some(c) = parse_color(&[(*t).clone()]) {
                    color = c;
                }
            }
            push("outline-width", DeclaredValue::Length(width));
            push("outline-color", DeclaredValue::Color(color));
        }

        // --- Background ---
        "background" => expand_background(&v, &mut push)?,
        "background-color" => {
            let c = parse_color(&v.iter().map(|t| (*t).clone()).collect::<Vec<_>>())
                .ok_or_else(|| bad(&property, "invalid color"))?;
            push("background-color", DeclaredValue::Color(c));
        }
        "background-repeat" => push("background-repeat", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "background-size" => {
            let mut parts = Vec::new();
            for t in &v {
                if let Token::Ident(i) = t {
                    parts.push(i.to_ascii_lowercase());
                } else if let Some(l) = parse_length(t, ctx) {
                    parts.push(format!("{l}"));
                }
            }
            push("background-size", DeclaredValue::StringList(parts));
        }
        "opacity" => match v[0] {
            Token::Number { value, .. } => push("opacity", DeclaredValue::Number((*value as f32).clamp(0.0, 1.0))),
            Token::Percentage(p) => push("opacity", DeclaredValue::Number((*p as f32 / 100.0).clamp(0.0, 1.0))),
            _ => return Err(bad(&property, "expected number")),
        },

        // --- Text ---
        "color" => {
            let c = parse_color(&v.iter().map(|t| (*t).clone()).collect::<Vec<_>>())
                .ok_or_else(|| bad(&property, "invalid color"))?;
            push("color", DeclaredValue::Color(c));
        }
        "font" => expand_font(&v, ctx, &mut push)?,
        "font-size" => {
            // Keywords: `medium` (initial), `smaller`/`larger` (relative to the
            // parent, resolved by the cascade), and the absolute scale.
            if let Token::Ident(kw) = v[0] {
                let lower = kw.to_ascii_lowercase();
                let factor = match lower.as_str() {
                    "medium" => Some(1.0),
                    "smaller" => Some(1.0 / 1.2),
                    "larger" => Some(1.2),
                    "xx-small" => Some(0.6),
                    "x-small" => Some(0.75),
                    "small" => Some(0.83),
                    "large" => Some(1.2),
                    "x-large" => Some(1.5),
                    "xx-large" => Some(2.0),
                    _ => None,
                };
                if let Some(f) = factor {
                    // A factor of 1.0 with keyword `medium` means "unset".
                    if lower == "medium" {
                        push("font-size", DeclaredValue::WideKeyword(WideKeyword::Initial));
                    } else {
                        // Encoded as a percentage so the cascade resolves it
                        // against the parent's computed font size.
                        push("font-size", DeclaredValue::Length(Length::Pct(f * 100.0)));
                    }
                    return Ok(out);
                }
                return Err(bad(&property, "unknown font-size keyword"));
            }
            let l = parse_length(v[0], ctx).ok_or_else(|| bad(&property, "invalid size"))?;
            if !matches!(l, Length::Px(_) | Length::Pct(_)) {
                return Err(bad(&property, "font-size must be a length or percentage"));
            }
            push("font-size", DeclaredValue::Length(l));
        }
        "font-weight" => {
            let w = match v[0] {
                Token::Number { value, .. } => *value as f32,
                Token::Ident(i) => match i.to_ascii_lowercase().as_str() {
                    "normal" => 400.0,
                    "bold" => 700.0,
                    "bolder" => 700.0,
                    "lighter" => 300.0,
                    _ => return Err(bad(&property, "invalid weight")),
                },
                _ => return Err(bad(&property, "invalid weight")),
            };
            push("font-weight", DeclaredValue::Number(w.clamp(1.0, 1000.0)));
        }
        "font-style" => push("font-style", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "font-family" => {
            let mut fams = Vec::new();
            let mut cur = String::new();
            for t in &v {
                match t {
                    Token::Ident(i) => {
                        if !cur.is_empty() {
                            cur.push(' ');
                        }
                        cur.push_str(i);
                    }
                    Token::String(s) => {
                        if !cur.is_empty() {
                            fams.push(std::mem::take(&mut cur));
                        }
                        fams.push(s.clone());
                    }
                    Token::Comma => {
                        if !cur.is_empty() {
                            fams.push(std::mem::take(&mut cur));
                        }
                    }
                    _ => return Err(bad(&property, "invalid font family")),
                }
            }
            if !cur.is_empty() {
                fams.push(cur);
            }
            if fams.is_empty() {
                return Err(bad(&property, "empty font family"));
            }
            push("font-family", DeclaredValue::StringList(fams));
        }
        "line-height" => {
            let lh = match v[0] {
                Token::Ident(i) if i.eq_ignore_ascii_case("normal") => LineHeight::Normal,
                Token::Number { value, .. } => LineHeight::Number(*value as f32),
                t => {
                    let l = parse_length(t, ctx).ok_or_else(|| bad(&property, "invalid line-height"))?;
                    match l {
                        Length::Px(p) => LineHeight::Px(p),
                        Length::Pct(p) => LineHeight::Number(p / 100.0),
                        Length::Auto => return Err(bad(&property, "invalid line-height")),
                    }
                }
            };
            push("line-height", DeclaredValue::Ident(format!("{lh:?}")));
        }
        "letter-spacing" => {
            let l = parse_length(v[0], ctx).ok_or_else(|| bad(&property, "invalid spacing"))?;
            push("letter-spacing", DeclaredValue::Length(l));
        }
        "text-align" => push("text-align", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "text-decoration" | "text-decoration-line" => {
            let mut flags = Vec::new();
            for t in &v {
                if let Token::Ident(i) = t {
                    let i = i.to_ascii_lowercase();
                    if matches!(i.as_str(), "underline" | "line-through" | "none") {
                        flags.push(i);
                    } else if matches!(i.as_str(), "overline" | "blink") {
                        // Parsed but not drawn; keep it out of the flags.
                    } else {
                        // colour or style part of the shorthand; ignore.
                    }
                }
            }
            if flags.is_empty() {
                flags.push("none".into());
            }
            push("text-decoration-line", DeclaredValue::StringList(flags));
        }
        "white-space" => push("white-space", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "vertical-align" => {
            let l = parse_length(v[0], ctx);
            match l {
                Some(Length::Px(p)) if p != 0.0 => push("vertical-align", DeclaredValue::Length(Length::Px(p))),
                _ => push("vertical-align", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
            }
        }

        // --- Lists ---
        "list-style" => {
            for t in &v {
                if let Token::Ident(i) = t {
                    let i = i.to_ascii_lowercase();
                    if matches!(i.as_str(), "inside" | "outside") {
                        push("list-style-position", DeclaredValue::Ident(i));
                    } else if i == "none" {
                        push("list-style-type", DeclaredValue::Ident("none".into()));
                    } else {
                        push("list-style-type", DeclaredValue::Ident(i));
                    }
                }
            }
        }
        "list-style-type" => push("list-style-type", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "list-style-position" => push("list-style-position", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),

        // --- Flex ---
        "flex-direction" => push("flex-direction", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "flex-wrap" => push("flex-wrap", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "flex-flow" => {
            for t in &v {
                if let Token::Ident(i) = t {
                    let i = i.to_ascii_lowercase();
                    if matches!(i.as_str(), "row" | "row-reverse" | "column" | "column-reverse") {
                        push("flex-direction", DeclaredValue::Ident(i));
                    } else {
                        push("flex-wrap", DeclaredValue::Ident(i));
                    }
                }
            }
        }
        "flex" => expand_flex(&v, ctx, &mut push)?,
        "flex-grow" => {
            let n = number_or_zero(v[0]).ok_or_else(|| bad(&property, "expected number"))?;
            push("flex-grow", DeclaredValue::Number(n.max(0.0)));
        }
        "flex-shrink" => {
            let n = number_or_zero(v[0]).ok_or_else(|| bad(&property, "expected number"))?;
            push("flex-shrink", DeclaredValue::Number(n.max(0.0)));
        }
        "justify-content" => push("justify-content", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "align-items" => push("align-items", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),

        // --- Misc ---
        "cursor" => push("cursor", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "content" => {
            if v.len() == 1 {
                if let Token::String(s) = v[0] {
                    push("content", DeclaredValue::StringList(vec![s.clone()]));
                    return Ok(out);
                }
                if let Token::Ident(i) = v[0] {
                    if i.eq_ignore_ascii_case("none") || i.eq_ignore_ascii_case("normal") {
                        push("content", DeclaredValue::Ident("none".into()));
                        return Ok(out);
                    }
                }
            }
            return Err(bad(&property, "only string content is supported"));
        }
        "table-layout" => push("table-layout", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "border-collapse" => push("border-collapse", DeclaredValue::Ident(one_ident(&v, &property)?.to_ascii_lowercase())),
        "border-spacing" => {
            let mut vals = Vec::new();
            for t in &v {
                if let Some(l) = parse_length(t, ctx) {
                    vals.push(format!("{l}"));
                }
            }
            push("border-spacing", DeclaredValue::StringList(vals));
        }
        // Recognised-but-ignored properties, so they do not produce errors.
        "text-transform" | "font-variant" | "word-spacing" | "tab-size" | "direction" | "unicode-bidi"
        | "quotes" | "caption-side" | "empty-cells" | "vertical-align-note" => {
            // Dropped without an error; listed in the report once per property.
        }
        _ => {
            return Err(DeclError { property, message: "unsupported property".into() });
        }
    }
    Ok(out)
}

fn bad(prop: &str, msg: impl Into<String>) -> DeclError {
    DeclError { property: prop.to_string(), message: msg.into() }
}

fn one_ident<'a>(v: &[&'a Token], prop: &str) -> Result<&'a str, DeclError> {
    if v.len() != 1 {
        return Err(bad(prop, "expected a single keyword"));
    }
    v[0].as_ident().ok_or_else(|| bad(prop, "expected keyword"))
}

fn number_or_zero(t: &Token) -> Option<f32> {
    match t {
        Token::Number { value, .. } => finite_f32(*value),
        _ => None,
    }
}

/// Position just after the top-level `:` that separates property and value.
fn position_after_colon(tokens: &[Token]) -> usize {
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Function(_) | Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => depth -= 1,
            Token::Colon if depth == 0 => return i + 1,
            _ => {}
        }
    }
    tokens.len()
}

fn expand_four(v: &[&Token], ctx: &LengthCtx) -> Option<[Length; 4]> {
    let mut vals = Vec::new();
    for t in v {
        vals.push(parse_length(t, ctx)?);
    }
    four(vals)
}

fn expand_four_nonauto(v: &[&Token], ctx: &LengthCtx) -> Option<[Length; 4]> {
    let mut vals = Vec::new();
    for t in v {
        let l = parse_length(t, ctx)?;
        if matches!(l, Length::Auto) {
            return None;
        }
        vals.push(l);
    }
    four(vals)
}

fn four<T: Clone>(vals: Vec<T>) -> Option<[T; 4]> {
    match vals.len() {
        1 => Some([vals[0].clone(), vals[0].clone(), vals[0].clone(), vals[0].clone()]),
        2 => Some([vals[0].clone(), vals[1].clone(), vals[0].clone(), vals[1].clone()]),
        3 => Some([vals[0].clone(), vals[1].clone(), vals[2].clone(), vals[1].clone()]),
        4 => {
            let mut it = vals.into_iter();
            Some([it.next()?, it.next()?, it.next()?, it.next()?])
        }
        _ => None,
    }
}

fn expand_four_idents(v: &[&Token], prop: &str) -> Result<[String; 4], DeclError> {
    let mut vals = Vec::new();
    for t in v {
        vals.push(t.as_ident().ok_or_else(|| bad(prop, "expected keyword"))?.to_ascii_lowercase());
    }
    four(vals).ok_or_else(|| bad(prop, "expected 1-4 keywords"))
}

fn expand_four_colors(v: &[&Token]) -> Result<[Rgba8; 4], DeclError> {
    let mut vals = Vec::new();
    for t in v {
        vals.push(parse_color(&[(*t).clone()]).ok_or_else(|| bad("border-color", "invalid color"))?);
    }
    four(vals).ok_or_else(|| bad("border-color", "expected 1-4 colors"))
}

fn expand_border(
    v: &[&Token],
    ctx: &LengthCtx,
    side: Option<&str>,
    push: &mut impl FnMut(&str, DeclaredValue),
) -> Result<(), DeclError> {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    for t in v {
        if style.is_none() {
            if let Token::Ident(i) = t {
                if let Some(s) = parse_border_style(&i.to_ascii_lowercase()) {
                    style = Some(s);
                    continue;
                }
                if i.eq_ignore_ascii_case("thin") {
                    width = Some(Length::Px(1.0));
                    continue;
                }
                if i.eq_ignore_ascii_case("medium") {
                    width = Some(Length::Px(3.0));
                    continue;
                }
                if i.eq_ignore_ascii_case("thick") {
                    width = Some(Length::Px(5.0));
                    continue;
                }
            }
        }
        if width.is_none() {
            if let Some(l) = parse_length(t, ctx) {
                if !matches!(l, Length::Auto) {
                    width = Some(l);
                    continue;
                }
            }
        }
        if color.is_none() {
            if let Some(c) = parse_color(&[(*t).clone()]) {
                color = Some(c);
                continue;
            }
        }
    }
    // CSS says `border: 1px solid` leaves the colour at `currentcolor`.
    let width = width.unwrap_or(Length::Px(3.0));
    let style = style.unwrap_or(BorderStyle::None);
    match side {
        Some(s) => {
            push(&format!("border-{s}-width"), DeclaredValue::Length(width));
            push(&format!("border-{s}-style"), DeclaredValue::Ident(border_style_name(style).into()));
            if let Some(c) = color {
                push(&format!("border-{s}-color"), DeclaredValue::Color(c));
            }
        }
        None => {
            for s in ["top", "right", "bottom", "left"] {
                push(&format!("border-{s}-width"), DeclaredValue::Length(width));
                push(&format!("border-{s}-style"), DeclaredValue::Ident(border_style_name(style).into()));
                if let Some(c) = color {
                    push(&format!("border-{s}-color"), DeclaredValue::Color(c));
                }
            }
        }
    }
    Ok(())
}

fn expand_background(v: &[&Token], push: &mut impl FnMut(&str, DeclaredValue)) -> Result<(), DeclError> {
    for t in v {
        if let Some(c) = parse_color(&[(*t).clone()]) {
            push("background-color", DeclaredValue::Color(c));
            continue;
        }
        if let Token::Ident(i) = t {
            let i = i.to_ascii_lowercase();
            if matches!(i.as_str(), "repeat" | "no-repeat" | "repeat-x" | "repeat-y") {
                push("background-repeat", DeclaredValue::Ident(i));
            } else if matches!(i.as_str(), "cover" | "contain") {
                push("background-size", DeclaredValue::StringList(vec![i]));
            }
        }
    }
    Ok(())
}

fn expand_font(
    v: &[&Token],
    ctx: &LengthCtx,
    push: &mut impl FnMut(&str, DeclaredValue),
) -> Result<(), DeclError> {
    // `font: [style] [weight] size[/line-height] family`
    let mut idx = 0;
    let mut style = None;
    let mut weight = None;
    while idx < v.len() {
        match v[idx] {
            Token::Ident(i) => {
                let il = i.to_ascii_lowercase();
                if matches!(il.as_str(), "italic" | "oblique") {
                    style = Some(il);
                    idx += 1;
                } else if matches!(il.as_str(), "bold" | "bolder") {
                    weight = Some(700.0);
                    idx += 1;
                } else if matches!(il.as_str(), "normal" | "lighter") {
                    idx += 1;
                } else {
                    break;
                }
            }
            Token::Number { value, .. } if *value >= 1.0 && *value <= 1000.0 => {
                weight = Some(*value as f32);
                idx += 1;
            }
            _ => break,
        }
    }
    if idx >= v.len() {
        return Err(bad("font", "missing font size and family"));
    }
    let size = parse_length(v[idx], ctx).ok_or_else(|| bad("font", "invalid font size"))?;
    if !matches!(size, Length::Px(_) | Length::Pct(_)) {
        return Err(bad("font", "invalid font size"));
    }
    idx += 1;
    let mut line_height = None;
    if let Some(Token::Delim('/')) = v.get(idx) {
        idx += 1;
        if let Some(t) = v.get(idx) {
            line_height = match t {
                Token::Number { value, .. } => Some(LineHeight::Number(*value as f32)),
                tok => parse_length(tok, ctx).and_then(|l| match l {
                    Length::Px(p) => Some(LineHeight::Px(p)),
                    Length::Pct(p) => Some(LineHeight::Number(p / 100.0)),
                    Length::Auto => None,
                }),
            };
            idx += 1;
        }
    }
    if let Some(s) = style {
        push("font-style", DeclaredValue::Ident(s));
    }
    if let Some(w) = weight {
        push("font-weight", DeclaredValue::Number(w));
    }
    push("font-size", DeclaredValue::Length(size));
    if let Some(lh) = line_height {
        push("line-height", DeclaredValue::Ident(format!("{lh:?}")));
    }
    let mut fams = Vec::new();
    let mut cur = String::new();
    for t in &v[idx.min(v.len())..] {
        match t {
            Token::Ident(i) => {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(i);
            }
            Token::String(s) => {
                if !cur.is_empty() {
                    fams.push(std::mem::take(&mut cur));
                }
                fams.push(s.clone());
            }
            Token::Comma if !cur.is_empty() => {
                fams.push(std::mem::take(&mut cur));
            }
            _ => {}
        }
    }
    if !cur.is_empty() {
        fams.push(cur);
    }
    if !fams.is_empty() {
        push("font-family", DeclaredValue::StringList(fams));
    }
    Ok(())
}

fn expand_flex(
    v: &[&Token],
    ctx: &LengthCtx,
    push: &mut impl FnMut(&str, DeclaredValue),
) -> Result<(), DeclError> {
    // `flex: none | auto | [<grow> <shrink>? <basis>?]`
    if v.len() == 1 {
        if let Token::Ident(i) = v[0] {
            match i.to_ascii_lowercase().as_str() {
                "none" => {
                    push("flex-grow", DeclaredValue::Number(0.0));
                    push("flex-shrink", DeclaredValue::Number(0.0));
                    push("flex-basis", DeclaredValue::Ident("auto".into()));
                    return Ok(());
                }
                "auto" => {
                    push("flex-grow", DeclaredValue::Number(1.0));
                    push("flex-shrink", DeclaredValue::Number(1.0));
                    push("flex-basis", DeclaredValue::Ident("auto".into()));
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    let mut nums = Vec::new();
    let mut basis = None;
    for t in v {
        if let Some(n) = number_or_zero(t) {
            nums.push(n);
        } else if let Some(l) = parse_length(t, ctx) {
            basis = Some(l);
        }
    }
    push("flex-grow", DeclaredValue::Number(nums.first().copied().unwrap_or(0.0)));
    push("flex-shrink", DeclaredValue::Number(nums.get(1).copied().unwrap_or(1.0)));
    push(
        "flex-basis",
        basis.map(DeclaredValue::Length).unwrap_or(DeclaredValue::Ident("auto".into())),
    );
    Ok(())
}

fn parse_border_style(s: &str) -> Option<BorderStyle> {
    Some(match s {
        "none" | "hidden" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        _ => return None,
    })
}

pub fn border_style_name(s: BorderStyle) -> &'static str {
    match s {
        BorderStyle::None => "none",
        BorderStyle::Solid => "solid",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Double => "double",
    }
}

fn parse_display(id: &str) -> Option<Display> {
    Some(match id.to_ascii_lowercase().as_str() {
        "inline" => Display::Inline,
        "block" => Display::Block,
        "inline-block" => Display::InlineBlock,
        "flex" => Display::Flex,
        "inline-flex" => Display::InlineFlex,
        "grid" => Display::Grid,
        "inline-grid" => Display::InlineGrid,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "table-header-group" | "table-row-group" | "table-footer-group" | "table-caption"
        | "table-column" | "table-column-group" => Display::Table,
        "list-item" => Display::ListItem,
        "none" => Display::None,
        "contents" => Display::Contents,
        _ => return None,
    })
}

pub fn display_name(d: Display) -> &'static str {
    match d {
        Display::Inline => "inline",
        Display::Block => "block",
        Display::InlineBlock => "inline-block",
        Display::Flex => "flex",
        Display::InlineFlex => "inline-flex",
        Display::Grid => "grid",
        Display::InlineGrid => "inline-grid",
        Display::Table => "table",
        Display::TableRow => "table-row",
        Display::TableCell => "table-cell",
        Display::ListItem => "list-item",
        Display::None => "none",
        Display::Contents => "contents",
    }
}

/// All longhand names a shorthand expands to. Used for CSS-wide keywords.
///
/// Every entry returns a `&'static [&'static str]`; a longhand maps to itself.
/// Properties we do not support never reach this function (they error earlier).
fn longhands_of(prop: &str) -> &'static [&'static str] {
    match prop {
        "margin" => &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "padding" => &["padding-top", "padding-right", "padding-bottom", "padding-left"],
        "border" => &[
            "border-top-width", "border-top-style", "border-top-color",
            "border-right-width", "border-right-style", "border-right-color",
            "border-bottom-width", "border-bottom-style", "border-bottom-color",
            "border-left-width", "border-left-style", "border-left-color",
        ],
        "border-width" => &["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"],
        "border-style" => &["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"],
        "border-color" => &["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
        "border-top" => &["border-top-width", "border-top-style", "border-top-color"],
        "border-right" => &["border-right-width", "border-right-style", "border-right-color"],
        "border-bottom" => &["border-bottom-width", "border-bottom-style", "border-bottom-color"],
        "border-left" => &["border-left-width", "border-left-style", "border-left-color"],
        "border-radius" => &[
            "border-top-left-radius", "border-top-right-radius",
            "border-bottom-right-radius", "border-bottom-left-radius",
        ],
        "background" => &["background-color", "background-repeat", "background-size"],
        "overflow" => &["overflow-x", "overflow-y"],
        "font" => &["font-style", "font-weight", "font-size", "line-height", "font-family"],
        "flex" => &["flex-grow", "flex-shrink", "flex-basis"],
        "flex-flow" => &["flex-direction", "flex-wrap"],
        "gap" => &["row-gap", "column-gap"],
        "list-style" => &["list-style-type", "list-style-position"],
        "text-decoration" => &["text-decoration-line"],
        "outline" => &["outline-width", "outline-color"],
        other => longhand_self(other),
    }
}

/// A longhand's single name, as a `&'static str` slice.
fn longhand_self(name: &str) -> &'static [&'static str] {
    macro_rules! table {
        ($($n:literal),* $(,)?) => {
            match name {
                $($n => &[$n],)*
                _ => &[],
            }
        };
    }
    table![
        "display", "position", "float", "clear", "visibility", "z-index", "width", "height",
        "min-width", "max-width", "min-height", "max-height", "top", "right", "bottom", "left",
        "box-sizing", "margin-top", "margin-right", "margin-bottom", "margin-left",
        "padding-top", "padding-right", "padding-bottom", "padding-left",
        "border-top-width", "border-right-width", "border-bottom-width", "border-left-width",
        "border-top-style", "border-right-style", "border-bottom-style", "border-left-style",
        "border-top-color", "border-right-color", "border-bottom-color", "border-left-color",
        "border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius",
        "border-bottom-left-radius", "background-color", "background-repeat", "background-size",
        "opacity", "color", "font-size", "font-weight", "font-style", "font-family",
        "line-height", "letter-spacing", "text-align", "text-decoration-line", "white-space",
        "vertical-align", "list-style-type", "list-style-position", "flex-direction",
        "flex-wrap", "flex-grow", "flex-shrink", "flex-basis", "justify-content", "align-items",
        "row-gap", "column-gap", "overflow-x", "overflow-y", "content", "cursor", "table-layout",
        "outline-width", "outline-color", "border-collapse", "border-spacing",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::state::tokenize;

    fn decls(src: &str) -> (DeclarationBlock, Vec<DeclError>) {
        let toks = tokenize(src);
        let mut block = DeclarationBlock::default();
        let errs = parse_declarations(&toks[..toks.len() - 1], &LengthCtx::default(), &mut block);
        (block, errs)
    }

    fn value<'a>(block: &'a DeclarationBlock, prop: &str) -> Option<&'a DeclaredValue> {
        block.get(prop).map(|d| &d.value)
    }

    #[test]
    fn simple_longhands() {
        let (b, e) = decls("color: red; width: 10px;");
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(value(&b, "color"), Some(&DeclaredValue::Color(Rgba8::rgb(255, 0, 0))));
        assert_eq!(value(&b, "width"), Some(&DeclaredValue::Length(Length::Px(10.0))));
    }

    #[test]
    fn important_flag() {
        let (b, _) = decls("color: red !important;");
        assert!(b.get("color").unwrap().important);
    }

    #[test]
    fn margin_shorthand_expands() {
        let (b, _) = decls("margin: 1px 2px 3px 4px;");
        assert_eq!(value(&b, "margin-top"), Some(&DeclaredValue::Length(Length::Px(1.0))));
        assert_eq!(value(&b, "margin-right"), Some(&DeclaredValue::Length(Length::Px(2.0))));
        assert_eq!(value(&b, "margin-bottom"), Some(&DeclaredValue::Length(Length::Px(3.0))));
        assert_eq!(value(&b, "margin-left"), Some(&DeclaredValue::Length(Length::Px(4.0))));
    }

    #[test]
    fn margin_two_values() {
        let (b, _) = decls("margin: 1px 2px;");
        assert_eq!(value(&b, "margin-top"), Some(&DeclaredValue::Length(Length::Px(1.0))));
        assert_eq!(value(&b, "margin-right"), Some(&DeclaredValue::Length(Length::Px(2.0))));
        assert_eq!(value(&b, "margin-bottom"), Some(&DeclaredValue::Length(Length::Px(1.0))));
        assert_eq!(value(&b, "margin-left"), Some(&DeclaredValue::Length(Length::Px(2.0))));
    }

    #[test]
    fn border_shorthand_expands_all_sides() {
        let (b, e) = decls("border: 1px solid red;");
        assert!(e.is_empty(), "{e:?}");
        for side in ["top", "right", "bottom", "left"] {
            assert_eq!(
                value(&b, &format!("border-{side}-width")),
                Some(&DeclaredValue::Length(Length::Px(1.0)))
            );
            assert_eq!(
                value(&b, &format!("border-{side}-style")),
                Some(&DeclaredValue::Ident("solid".into()))
            );
            assert_eq!(
                value(&b, &format!("border-{side}-color")),
                Some(&DeclaredValue::Color(Rgba8::rgb(255, 0, 0)))
            );
        }
    }

    #[test]
    fn border_without_color_leaves_currentcolor() {
        let (b, _) = decls("border: 1px solid;");
        assert!(b.get("border-top-color").is_none(), "colour not set means currentcolor");
    }

    #[test]
    fn invalid_declaration_is_reported_and_skipped() {
        let (b, e) = decls("width: banana; height: 5px;");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].property, "width");
        assert_eq!(value(&b, "height"), Some(&DeclaredValue::Length(Length::Px(5.0))));
    }

    #[test]
    fn unsupported_property_errors() {
        let (_, e) = decls("column-count: 2;");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].message, "unsupported property");
    }

    #[test]
    fn wide_keyword_expands_through_shorthand() {
        let (b, _) = decls("margin: inherit;");
        for side in ["top", "right", "bottom", "left"] {
            assert_eq!(
                value(&b, &format!("margin-{side}")),
                Some(&DeclaredValue::WideKeyword(WideKeyword::Inherit))
            );
        }
    }

    #[test]
    fn font_shorthand() {
        let (b, e) = decls("font: italic bold 14px/1.5 Arial, sans-serif;");
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(value(&b, "font-style"), Some(&DeclaredValue::Ident("italic".into())));
        assert_eq!(value(&b, "font-weight"), Some(&DeclaredValue::Number(700.0)));
        assert_eq!(value(&b, "font-size"), Some(&DeclaredValue::Length(Length::Px(14.0))));
        assert_eq!(
            value(&b, "font-family"),
            Some(&DeclaredValue::StringList(vec!["Arial".into(), "sans-serif".into()]))
        );
    }

    #[test]
    fn flex_none() {
        let (b, _) = decls("flex: none;");
        assert_eq!(value(&b, "flex-grow"), Some(&DeclaredValue::Number(0.0)));
        assert_eq!(value(&b, "flex-shrink"), Some(&DeclaredValue::Number(0.0)));
        assert_eq!(value(&b, "flex-basis"), Some(&DeclaredValue::Ident("auto".into())));
    }

    #[test]
    fn flex_one() {
        let (b, _) = decls("flex: 1;");
        assert_eq!(value(&b, "flex-grow"), Some(&DeclaredValue::Number(1.0)));
        assert_eq!(value(&b, "flex-shrink"), Some(&DeclaredValue::Number(1.0)));
    }

    #[test]
    fn display_values() {
        let (b, _) = decls("display: inline-block;");
        assert_eq!(value(&b, "display"), Some(&DeclaredValue::Ident("inline-block".into())));
        let (_, e) = decls("display: banana;");
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn zero_length_allowed_nonzero_without_unit_rejected() {
        let (b, _) = decls("margin-top: 0;");
        assert_eq!(value(&b, "margin-top"), Some(&DeclaredValue::Length(Length::Px(0.0))));
        let (_, e) = decls("margin-top: 5;");
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn missing_colon_errors() {
        let (_, e) = decls("color red;");
        assert_eq!(e.len(), 1);
        assert!(e[0].message.contains("':'"));
    }

    #[test]
    fn multiple_declarations_keep_order() {
        let (b, _) = decls("color: red; color: blue;");
        assert_eq!(value(&b, "color"), Some(&DeclaredValue::Color(Rgba8::rgb(0, 0, 255))));
    }
}