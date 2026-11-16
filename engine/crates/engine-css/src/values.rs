//! Value parsing: lengths, colors, keywords, and `calc()`.

use engine_core::{CalcExpr, Length, Rgba8};

use crate::tokenizer::token::Token;

/// Convert a parsed `f64` to a finite `f32`, or reject it.
///
/// This is the single choke point for "a number from a page becomes a layout
/// coordinate". `f64::INFINITY` (from a numeric literal too long for `f64`),
/// `NaN`, and values that overflow `f32` all become `None`, so the declaration
/// is dropped and reported by the caller instead of poisoning layout with
/// `inf`. See `parse_length` for why this matters.
pub fn finite_f32(v: f64) -> Option<f32> {
    let f = v as f32;
    f.is_finite().then_some(f)
}

/// Parse a length from a single token. `font_size_px` and `root_font_size_px`
/// resolve `em`/`rem`; `viewport` resolves `vw`/`vh`. Relative units that
/// cannot be resolved here (percentages) stay as `Length::Pct`.
pub struct LengthCtx {
    pub font_size_px: f32,
    pub root_font_size_px: f32,
    pub viewport_w: f32,
    pub viewport_h: f32,
}

impl Default for LengthCtx {
    fn default() -> Self {
        LengthCtx { font_size_px: 16.0, root_font_size_px: 16.0, viewport_w: 800.0, viewport_h: 600.0 }
    }
}

/// Parse a length from one token.
pub fn parse_length(tok: &Token, ctx: &LengthCtx) -> Option<Length> {
    match tok {
        Token::Number { value, .. } => {
            // A bare number is only a length where 0 is allowed; reject non-zero
            // so `width: 5` is dropped rather than silently treated as px.
            if *value == 0.0 {
                Some(Length::Px(0.0))
            } else {
                None
            }
        }
        Token::Percentage(p) => finite_f32(*p).map(Length::Pct),
        Token::Dimension { value, unit } => {
            let v = finite_f32(*value)?;
            let px = match unit.to_ascii_lowercase().as_str() {
                "px" => v,
                "em" => v * ctx.font_size_px,
                "rem" => v * ctx.root_font_size_px,
                "ex" => v * ctx.font_size_px * engine_core::EX_FALLBACK_EM,
                "ch" => v * ctx.font_size_px * 0.5,
                "cm" => engine_core::css_units::cm_to_px(v),
                "mm" => engine_core::css_units::mm_to_px(v),
                "in" => engine_core::css_units::in_to_px(v),
                "pt" => engine_core::css_units::pt_to_px(v),
                "pc" => engine_core::css_units::pc_to_px(v),
                "q" => engine_core::css_units::q_to_px(v),
                "vw" => ctx.viewport_w * v / 100.0,
                "vh" => ctx.viewport_h * v / 100.0,
                "vmin" => ctx.viewport_w.min(ctx.viewport_h) * v / 100.0,
                "vmax" => ctx.viewport_w.max(ctx.viewport_h) * v / 100.0,
                // Unsupported units (§15: `cqw`, `lh`, `vi`, `vb`) are rejected.
                _ => return None,
            };
            // A finite literal can still overflow to `inf` after unit scaling
            // (`1e38em`), so the result is checked too.
            finite_f32(px as f64).map(Length::Px)
        }
        Token::Ident(id) if id.eq_ignore_ascii_case("auto") => Some(Length::Auto),
        _ => None,
    }
}

/// Parse a `calc()` from tokens inside `calc(...)`, up to the closing paren.
pub fn parse_calc(tokens: &[Token], ctx: &LengthCtx) -> Option<Length> {
    let mut p = CalcParser { tokens, pos: 0, ctx, depth: 0 };
    let expr = p.sum()?;
    p.skip_ws();
    if p.pos < p.tokens.len() {
        return None;
    }
    expr.eval()
}

struct CalcParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    ctx: &'a LengthCtx,
    /// Paren nesting depth, bounded by `MAX_CALC_DEPTH`.
    depth: u32,
}

impl CalcParser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
    }

    fn sum(&mut self) -> Option<CalcExpr> {
        let mut left = self.product()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    self.skip_ws();
                    let right = self.product()?;
                    left = CalcExpr::Add(Box::new(left), Box::new(right));
                }
                Some(Token::Delim('-')) => {
                    self.pos += 1;
                    self.skip_ws();
                    let right = self.product()?;
                    left = CalcExpr::Sub(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Some(left)
    }

    fn product(&mut self) -> Option<CalcExpr> {
        let mut left = self.atom()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(Token::Delim('*')) => {
                    self.pos += 1;
                    self.skip_ws();
                    let right = self.atom()?;
                    left = CalcExpr::Mul(Box::new(left), Box::new(right));
                }
                Some(Token::Delim('/')) => {
                    self.pos += 1;
                    self.skip_ws();
                    let right = self.atom()?;
                    left = CalcExpr::Div(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Some(left)
    }

    fn atom(&mut self) -> Option<CalcExpr> {
        self.skip_ws();
        let tok = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        // Guard against `calc()` nesting deeper than the stack allows. `atom`
        // recurses through `sum()` for every `(`, so `calc(((((…` recurses once
        // per paren. Real stylesheets nest a handful of levels.
        if self.depth >= MAX_CALC_DEPTH {
            return None;
        }
        match tok {
            Token::Number { value, .. } => finite_f32(value).map(CalcExpr::num),
            Token::Percentage(p) => finite_f32(p).map(|p| CalcExpr::length(Length::Pct(p))),
            Token::Dimension { .. } => parse_length(&tok, self.ctx).map(CalcExpr::length),
            Token::LParen => {
                self.depth += 1;
                let e = self.sum();
                self.depth -= 1;
                let e = e?;
                self.skip_ws();
                match self.peek() {
                    Some(Token::RParen) => {
                        self.pos += 1;
                        Some(e)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// Maximum nesting of parentheses inside one `calc()` expression.
pub const MAX_CALC_DEPTH: u32 = 64;

/// Parse a color from tokens. Accepts `#rgb`, `#rrggbb`, `#rrggbbaa`, named
/// colors, `transparent`, `currentcolor`, and `rgb()/rgba()/hsl()`.
pub fn parse_color(tokens: &[Token]) -> Option<Rgba8> {
    if tokens.is_empty() {
        return None;
    }
    match &tokens[0] {
        Token::Hash { value, .. } => parse_hex(value),
        Token::Ident(name) => {
            if name.eq_ignore_ascii_case("transparent") {
                Some(Rgba8::TRANSPARENT)
            } else {
                engine_core::color::named_color(name)
            }
        }
        Token::Function(fname) => {
            let f = fname.to_ascii_lowercase();
            // The tokenizer emits the closing `)` as its own `RParen` token and
            // always appends `Eof`. Both are structural, not arguments: drop
            // them (plus whitespace) or `rgb(0 0 0)` looks like four components
            // and `rgb(255,0,0)` like seven. Nested parens inside the argument
            // (`rgb(var(--x), 0, 0)`) are kept by tracking depth.
            let mut depth = 0usize;
            let mut args = Vec::with_capacity(tokens.len());
            for t in &tokens[1..] {
                match t {
                    Token::Eof => break,
                    Token::RParen => {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    Token::Function(_) | Token::LParen => depth += 1,
                    _ => {}
                }
                if !t.is_whitespace() {
                    args.push(t);
                }
            }
            match f.as_str() {
                "rgb" | "rgba" => parse_rgb(&args),
                "hsl" | "hsla" => parse_hsl(&args),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_hex(h: &str) -> Option<Rgba8> {
    let digits: Vec<u8> = h.bytes().collect();
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    match digits.len() {
        3 => {
            let r = val(digits[0])?;
            let g = val(digits[1])?;
            let b = val(digits[2])?;
            Some(Rgba8::rgb(r * 17, g * 17, b * 17))
        }
        4 => {
            let r = val(digits[0])?;
            let g = val(digits[1])?;
            let b = val(digits[2])?;
            let a = val(digits[3])?;
            Some(Rgba8::rgba(r * 17, g * 17, b * 17, a * 17))
        }
        6 => {
            let byte = |i: usize| -> Option<u8> { Some(val(digits[i])? * 16 + val(digits[i + 1])?) };
            Some(Rgba8::rgb(byte(0)?, byte(2)?, byte(4)?))
        }
        8 => {
            let byte = |i: usize| -> Option<u8> { Some(val(digits[i])? * 16 + val(digits[i + 1])?) };
            Some(Rgba8::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?))
        }
        _ => None,
    }
}

fn channel(t: &Token) -> Option<f32> {
    match t {
        Token::Number { value, .. } => Some(*value as f32),
        Token::Percentage(p) => Some((*p as f32) * 2.55),
        _ => None,
    }
}

fn alpha(t: &Token) -> Option<f32> {
    match t {
        Token::Number { value, .. } => Some((*value as f32).clamp(0.0, 1.0)),
        Token::Percentage(p) => Some((*p as f32 / 100.0).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn parse_rgb(args: &[&Token]) -> Option<Rgba8> {
    // Comma-separated or space-separated with an optional `/ alpha`.
    let (comps, alpha_tok): (Vec<&Token>, Option<&Token>) = if args.iter().any(|t| matches!(t, Token::Comma)) {
        let parts: Vec<&Token> = args.iter().copied().filter(|t| !matches!(t, Token::Comma)).collect();
        match parts.as_slice() {
            [r, g, b] => (vec![*r, *g, *b], None),
            [r, g, b, a] => (vec![*r, *g, *b], Some(*a)),
            _ => return None,
        }
    } else if let Some(slash) = args.iter().position(|t| matches!(t, Token::Delim('/'))) {
        (args[..slash].to_vec(), args.get(slash + 1).copied())
    } else {
        (args.to_vec(), None)
    };
    if comps.len() != 3 {
        return None;
    }
    let r = channel(comps[0])?;
    let g = channel(comps[1])?;
    let b = channel(comps[2])?;
    let a = alpha_tok.and_then(alpha).unwrap_or(1.0);
    Some(Rgba8::from_f64(r as f64, g as f64, b as f64, a as f64))
}

fn parse_hsl(args: &[&Token]) -> Option<Rgba8> {
    let vals: Vec<&Token> = args.iter().copied().filter(|t| !matches!(t, Token::Comma)).collect();
    let (h_tok, rest) = vals.split_first()?;
    let h = match h_tok {
        Token::Number { value, .. } => *value as f32,
        Token::Dimension { value, unit } if unit.eq_ignore_ascii_case("deg") => *value as f32,
        _ => return None,
    };
    if rest.len() < 2 {
        return None;
    }
    let pct = |t: &Token| -> Option<f32> {
        match t {
            Token::Percentage(p) => Some((*p as f32 / 100.0).clamp(0.0, 1.0)),
            Token::Number { value, .. } => Some((*value as f32 / 100.0).clamp(0.0, 1.0)),
            _ => None,
        }
    };
    let s = pct(rest[0])?;
    let l = pct(rest[1])?;
    let a = rest.get(2).and_then(|t| alpha(t)).unwrap_or(1.0);
    Some(hsl_to_rgb(h, s, l, a))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32, a: f32) -> Rgba8 {
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    if s == 0.0 {
        let v = (l * 255.0).round() as f64;
        return Rgba8::from_f64(v, v, v, a as f64);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    Rgba8::from_f64(
        (hue(h + 1.0 / 3.0) * 255.0).round() as f64,
        (hue(h) * 255.0).round() as f64,
        (hue(h - 1.0 / 3.0) * 255.0).round() as f64,
        a as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::state::tokenize;

    fn ctx() -> LengthCtx {
        LengthCtx { font_size_px: 20.0, root_font_size_px: 16.0, viewport_w: 1000.0, viewport_h: 500.0 }
    }

    fn len(src: &str) -> Option<Length> {
        let toks = tokenize(src);
        parse_length(toks.first().unwrap(), &ctx())
    }

    #[test]
    fn px() {
        assert_eq!(len("10px"), Some(Length::Px(10.0)));
    }

    #[test]
    fn em_uses_font_size() {
        assert_eq!(len("2em"), Some(Length::Px(40.0)));
    }

    #[test]
    fn rem_uses_root_font_size() {
        assert_eq!(len("2rem"), Some(Length::Px(32.0)));
    }

    #[test]
    fn percent_stays_symbolic() {
        assert_eq!(len("50%"), Some(Length::Pct(50.0)));
    }

    #[test]
    fn viewport_units() {
        assert_eq!(len("10vw"), Some(Length::Px(100.0)));
        assert_eq!(len("10vh"), Some(Length::Px(50.0)));
        assert_eq!(len("10vmin"), Some(Length::Px(50.0)));
        assert_eq!(len("10vmax"), Some(Length::Px(100.0)));
    }

    #[test]
    fn absolute_units() {
        assert_eq!(len("1in"), Some(Length::Px(96.0)));
        assert_eq!(len("72pt"), Some(Length::Px(96.0)));
    }

    #[test]
    fn zero_without_unit_is_zero() {
        assert_eq!(len("0"), Some(Length::Px(0.0)));
    }

    #[test]
    fn nonzero_unitless_is_rejected() {
        assert_eq!(len("5"), None);
    }

    #[test]
    fn unsupported_units_rejected() {
        assert_eq!(len("2cqw"), None);
        assert_eq!(len("2lh"), None);
    }

    #[test]
    fn auto() {
        assert_eq!(len("auto"), Some(Length::Auto));
    }

    #[test]
    fn calc_add_and_scale() {
        let toks = tokenize("calc(10px + 2 * 5px)");
        // Skip the function token and the trailing paren.
        let inner: Vec<Token> = toks[1..toks.len() - 2].to_vec();
        assert_eq!(parse_calc(&inner, &ctx()), Some(Length::Px(20.0)));
    }

    #[test]
    fn calc_with_percent() {
        let toks = tokenize("calc(50% - 10px)");
        let inner: Vec<Token> = toks[1..toks.len() - 2].to_vec();
        // px+% is affine and rejected rather than silently folded.
        assert_eq!(parse_calc(&inner, &ctx()), None);
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_color(&tokenize("#f00")), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(parse_color(&tokenize("#ff0000")), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(parse_color(&tokenize("#ff000080")), Some(Rgba8::rgba(255, 0, 0, 128)));
        assert_eq!(parse_color(&tokenize("#f008")), Some(Rgba8::rgba(255, 0, 0, 136)));
    }

    #[test]
    fn named_and_transparent() {
        assert_eq!(parse_color(&tokenize("red")), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(parse_color(&tokenize("transparent")), Some(Rgba8::TRANSPARENT));
        assert_eq!(parse_color(&tokenize("notacolor")), None);
    }

    #[test]
    fn rgb_functional() {
        assert_eq!(parse_color(&tokenize("rgb(255, 0, 0)")), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(parse_color(&tokenize("rgba(255,0,0,0.5)")), Some(Rgba8::rgba(255, 0, 0, 128)));
        assert_eq!(parse_color(&tokenize("rgb(100% 0% 0%)")), Some(Rgba8::rgb(255, 0, 0)));
    }

    #[test]
    fn hsl_functional() {
        assert_eq!(parse_color(&tokenize("hsl(0, 100%, 50%)")), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(parse_color(&tokenize("hsl(120 100% 50%)")), Some(Rgba8::rgb(0, 255, 0)));
    }
}