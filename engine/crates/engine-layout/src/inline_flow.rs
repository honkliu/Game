//! Line layout for inline content: whitespace collapsing, word wrapping,
//! `text-align`, and `line-height`.
//!
//! **Scope: text runs only.** A line holds text boxes and their nearest inline
//! ancestor's text properties — not inline box fragments, atomic inlines, or
//! per-fragment backgrounds. An inline-level *box* child (`inline-block`, a
//! replaced element) is laid out as its own anonymous block by the caller, so a
//! line never positions one. This is the stated simplification: wrapped inline
//! text runs are enough; there is no full inline fragmentation.
//!
//! Whitespace (CSS 2.1 §16.6, the cases that matter):
//! - Collapsing context: a run of spaces/tabs/newlines becomes one space;
//!   leading whitespace of a line box is removed and trailing whitespace on
//!   every line is removed.
//! - `pre` / `pre-wrap`: newlines are mandatory breaks and spaces are kept
//!   (tabs collapse to one space width; no tab stops in v1).
//! - `nowrap`: only mandatory newlines break.
//!
//! Complexity: one pass to tokenise, one pass to break. Every measurement is
//! cached per (style, token) so a word is measured once, not once per line it
//! could have gone on.

use std::sync::Arc;

use engine_core::{Rect, Rgba8};
use engine_css::properties::{FontStyle, LineHeight, TextAlign};
use engine_css::ComputedStyle;
use engine_text::{FontContext, TextMetrics};

use crate::box_tree::is_blank;

/// One styled piece of text: a text box, and the style whose text properties
/// apply to it.
pub struct Item<'a> {
    pub text: &'a str,
    pub style: &'a ComputedStyle,
}

/// A positioned line of text. All coordinates are relative to the inline
/// formatting context's origin, so the caller adds one offset.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The line box: `x` is 0, `w` is the available width.
    pub rect: Rect,
    /// Baseline y, relative to the context origin.
    pub baseline: f32,
    /// The space-collapsed text of the line, already trimmed.
    pub text: String,
    /// Left edge of the text inside the line box, after `text-align`.
    pub text_x: f32,
    /// The style used to shape and paint this line.
    pub style: Arc<ComputedStyle>,
    /// The text colour, resolved from the style chain.
    pub color: Rgba8,
    /// Underline the line.
    pub underline: bool,
    /// Strike through the line.
    pub line_through: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Word,
    Space,
    Break,
}

struct Token {
    kind: TokenKind,
    text: String,
    width: f32,
    /// Index into the `styles` table.
    style: usize,
}

/// Lay out `items` into lines of at most `available_width` px.
///
/// `fallback` supplies the font and line-height for an empty context.
pub fn layout(
    items: &[Item<'_>],
    fallback: &Arc<ComputedStyle>,
    available_width: f32,
    fonts: &mut FontContext,
) -> Vec<Line> {
    let width = available_width.max(0.0);

    // Distinct styles. `Arc` identity is not reliable here (the same computed
    // style can be cloned into several `Arc`s), so compare by pointer to the
    // `ComputedStyle` the `&str` came from; that *is* stable for items that
    // share an element.
    let mut styles: Vec<Arc<ComputedStyle>> = Vec::new();
    let mut style_of_item: Vec<usize> = Vec::with_capacity(items.len());
    for item in items {
        let existing = styles.iter().position(|s| std::ptr::eq(&**s, item.style));
        let idx = match existing {
            Some(i) => i,
            None => {
                styles.push(Arc::new(item.style.clone()));
                styles.len() - 1
            }
        };
        style_of_item.push(idx);
    }
    if styles.is_empty() {
        styles.push(Arc::clone(fallback));
    }

    let tokens = tokenize(items, &style_of_item, &styles, fonts);
    break_lines(&tokens, &styles, width, fonts)
}

/// Collapse whitespace and split into words, spaces, and breaks.
fn tokenize(
    items: &[Item<'_>],
    style_of_item: &[usize],
    styles: &[Arc<ComputedStyle>],
    fonts: &mut FontContext,
) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut pending_space: Option<usize> = None;

    for (item, &style_idx) in items.iter().zip(style_of_item) {
        let style = &styles[style_idx];
        let collapse = style.white_space.collapses_spaces();
        let preserve_newlines = style.white_space.preserves_newlines();

        // Split the item into maximal word runs and white runs once, then map
        // each piece to a token. `split` (not `split_whitespace`) keeps the
        // shape so styles for a space are attributable to the right item.
        let mut run_start = 0;
        let mut run_is_space: Option<bool> = None;
        for (i, c) in item.text.char_indices() {
            let is_space = c.is_whitespace();
            if run_is_space == Some(is_space) {
                continue;
            }
            if let Some(prev_space) = run_is_space {
                emit_run(&item.text[run_start..i], prev_space, style_idx, style, styles, fonts, collapse, preserve_newlines, &mut tokens, &mut pending_space);
            }
            run_start = i;
            run_is_space = Some(is_space);
        }
        if let Some(sp) = run_is_space {
            emit_run(&item.text[run_start..], sp, style_idx, style, styles, fonts, collapse, preserve_newlines, &mut tokens, &mut pending_space);
        }
    }
    if tokens.first().is_some_and(|t| t.kind == TokenKind::Space) {
        tokens.remove(0);
    }
    tokens
}

#[allow(clippy::too_many_arguments)]
fn emit_run(
    run: &str,
    is_space: bool,
    style_idx: usize,
    style: &ComputedStyle,
    styles: &[Arc<ComputedStyle>],
    fonts: &mut FontContext,
    collapse: bool,
    preserve_newlines: bool,
    tokens: &mut Vec<Token>,
    pending_space: &mut Option<usize>,
) {
    if !is_space {
        if let Some(sp_style) = pending_space.take() {
            let s = &styles[sp_style];
            let w = fonts.char_advance(' ', &s.font_family, s.font_weight, is_italic(s), s.font_size_px);
            tokens.push(Token { kind: TokenKind::Space, text: " ".to_string(), width: w, style: sp_style });
        }
        let m = measure(fonts, run, style);
        tokens.push(Token { kind: TokenKind::Word, text: run.to_string(), width: m, style: style_idx });
        return;
    }
    // Whitespace run.
    if collapse {
        if preserve_newlines && run.contains('\n') {
            // A preserved newline in `pre-line` forces a break; everything
            // around it collapses. Emit a break and clear a pending space.
            while let Some(t) = tokens.last() {
                if t.kind == TokenKind::Space {
                    tokens.pop();
                } else {
                    break;
                }
            }
            *pending_space = None;
            tokens.push(Token { kind: TokenKind::Break, text: String::new(), width: 0.0, style: style_idx });
        } else {
            *pending_space = Some(style_idx);
        }
        return;
    }
    // `pre` / `pre-wrap`: spaces are content, newlines break.
    *pending_space = None;
    for c in run.chars() {
        if c == '\n' {
            // Remove trailing spaces before a break — they do not render.
            while tokens.last().is_some_and(|t| t.kind == TokenKind::Space) {
                tokens.pop();
            }
            tokens.push(Token { kind: TokenKind::Break, text: String::new(), width: 0.0, style: style_idx });
        } else {
            let w = fonts.char_advance(' ', &style.font_family, style.font_weight, is_italic(style), style.font_size_px);
            tokens.push(Token { kind: TokenKind::Space, text: " ".to_string(), width: w, style: style_idx });
        }
    }
}

fn break_lines(tokens: &[Token], styles: &[Arc<ComputedStyle>], width: f32, fonts: &mut FontContext) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut cur: Vec<&Token> = Vec::new();
    let mut cur_w = 0.0f32;
    let mut cur_style = tokens.first().map_or(0, |t| t.style);

    for tok in tokens {
        match tok.kind {
            TokenKind::Break => {
                push_line(&mut lines, &mut cur, &mut cur_w, styles, cur_style, width, fonts);
                cur_style = tok.style;
            }
            TokenKind::Space => {
                if cur.is_empty() {
                    // Leading whitespace on a line box does not render.
                    continue;
                }
                // Never let a space alone overflow the line.
                if cur_w + tok.width > width {
                    push_line(&mut lines, &mut cur, &mut cur_w, styles, cur_style, width, fonts);
                    cur_style = tok.style;
                    continue;
                }
                cur.push(tok);
                cur_w += tok.width;
            }
            TokenKind::Word => {
                let style_can_wrap = !styles[tok.style].white_space.preserves_newlines()
                    || styles[tok.style].white_space.wraps();
                let must_break = style_can_wrap && !cur.is_empty() && cur_w + tok.width > width;
                if must_break {
                    push_line(&mut lines, &mut cur, &mut cur_w, styles, cur_style, width, fonts);
                    cur_style = tok.style;
                }
                if cur.is_empty() {
                    cur_style = tok.style;
                }
                cur.push(tok);
                cur_w += tok.width;
            }
        }
    }
    if !cur.is_empty() {
        push_line(&mut lines, &mut cur, &mut cur_w, styles, cur_style, width, fonts);
    }
    lines
}

fn push_line(
    lines: &mut Vec<Line>,
    cur: &mut Vec<&Token>,
    cur_w: &mut f32,
    styles: &[Arc<ComputedStyle>],
    style_idx: usize,
    width: f32,
    fonts: &mut FontContext,
) {
    // Trailing whitespace does not render.
    while cur.last().is_some_and(|t| t.kind == TokenKind::Space) {
        cur.pop();
    }
    if cur.is_empty() {
        *cur_w = 0.0;
        return;
    }
    let style = Arc::clone(&styles[style_idx]);
    let m = measure(fonts, "", &style);

    let (text, text_w) = {
        let mut s = String::new();
        for t in cur.iter() {
            s.push_str(&t.text);
        }
        let w = *cur_w;
        (s, w)
    };

    let line_h = match style.line_height {
        LineHeight::Normal => m.ascent + m.descent,
        LineHeight::Number(n) => style.font_size_px * n,
        LineHeight::Px(v) => v,
    }
    .max(m.ascent + m.descent)
    .max(0.0);
    // Half-leading above and below keeps the glyphs vertically centred in a
    // line box taller than the font box.
    let half_leading = (line_h - (m.ascent + m.descent)) / 2.0;
    let baseline = lines.last().map_or(0.0, |l: &Line| l.rect.bottom()) + half_leading + m.ascent;

    let text_x = match style.text_align {
        TextAlign::Center => ((width - text_w) / 2.0).max(0.0),
        TextAlign::End => (width - text_w).max(0.0),
        // `justify` needs inter-word stretching, which v1 does not do; it
        // renders as `start`.
        TextAlign::Start | TextAlign::Justify => 0.0,
    };
    let y = lines.last().map_or(0.0, |l: &Line| l.rect.bottom());

    lines.push(Line {
        rect: Rect::new(0.0, y, width, line_h),
        baseline,
        text,
        text_x,
        color: style.color,
        underline: style.text_decoration.underline,
        line_through: style.text_decoration.line_through,
        style,
    });
    *cur = Vec::new();
    *cur_w = 0.0;
}

fn is_italic(s: &ComputedStyle) -> bool {
    s.font_style != FontStyle::Normal
}

fn measure(fonts: &mut FontContext, text: &str, style: &ComputedStyle) -> TextMetrics {
    fonts.measure(text, &style.font_family, style.font_weight, is_italic(style), style.font_size_px, style.letter_spacing)
}

/// The height of an empty line box for `style` — used by a block whose inline
/// content is empty but which still has a line box (an empty `<p>`).
pub fn empty_line_height(style: &ComputedStyle, fonts: &mut FontContext) -> f32 {
    let m = measure(fonts, "", style);
    let font_box = m.ascent + m.descent;
    match style.line_height {
        LineHeight::Normal => font_box,
        LineHeight::Number(n) => style.font_size_px * n,
        LineHeight::Px(v) => v,
    }
    .max(font_box)
}

/// True when `text` renders nothing: empty or whitespace only.
pub fn is_blank_text(text: &str) -> bool {
    is_blank(text)
}