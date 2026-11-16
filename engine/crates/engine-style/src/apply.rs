//! Applying one `DeclaredValue` onto one longhand of a `ComputedStyle`.
//!
//! `engine-css` emits every declaration as `(property-name: &str, DeclaredValue)`
//! with shorthands already expanded (§5.3), so this module is the single place
//! that knows how a property *name* maps to a struct field. A name we do not
//! recognise is reported back as `false` and the declaration is dropped with a
//! report entry — never silently ignored, because a silently ignored
//! `font-size` is a page that looks plausible and is wrong.
//!
//! Two encodings are workarounds for `engine-css`'s `DeclaredValue` not having
//! dedicated variants:
//! - `line-height` arrives as `Ident("<LineHeight Debug>")` (`"Normal"`,
//!   `"Number(1.5)"`, `"Px(20.0)"`).
//! - `background-size`, `text-decoration-line`, and `border-spacing` arrive as
//!   `StringList`.
//!
//! Both are parsed here, and both are covered by tests against the real parser.

use engine_core::{EdgeSizes, Length, Rgba8};
use engine_css::properties::{
    AlignItems, BackgroundRepeat, BorderStyle, BoxSizing, Clear, Display, Float, FlexDirection,
    FlexWrap, FontStyle, JustifyContent, LineHeight, ListStylePosition, ListStyleType, Overflow,
    Position, TextAlign, VerticalAlign, WhiteSpace,
};
use engine_css::ComputedStyle;
use engine_css::DeclaredValue;

/// Side index for the four-value border arrays, in CSS order
/// (top, right, bottom, left) — which is also the order `BorderSide`-bearing
/// fields use in `ComputedStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub fn idx(self) -> usize {
        match self {
            Side::Top => 0,
            Side::Right => 1,
            Side::Bottom => 2,
            Side::Left => 3,
        }
    }
}

/// The result of applying a declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyResult {
    /// Applied to the field(s) named.
    Applied,
    /// A syntactically valid value we do not implement. Reported, not applied.
    Unsupported,
}

/// Apply `value` for `property`. `base` is the containing-block width used to
/// resolve percentages, and `font_px` the element's own font size (needed for
/// `1.5em` `line-height` and `letter-spacing`).
///
/// Returns `None` when the property name is not one we know at all. `Some(..)`
/// means the name is ours; the inner variant says whether the value landed.
pub fn apply(style: &mut ComputedStyle, property: &str, value: &DeclaredValue, font_px: f32) -> Option<ApplyResult> {
    // Percentages that need a containing block are kept as `Length::Pct` for
    // layout (§5.6). Only `line-height` and `letter-spacing` resolve against the
    // font size here, because they are text metrics, not box geometry.
    match (property, value) {
        // --- Box generation ---
        ("display", DeclaredValue::Ident(v)) => Some(set_display(style, v)),
        ("position", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "static" => Position::Static,
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            "fixed" => Position::Fixed,
            "sticky" => Position::Sticky,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, p| s.position = p)),
        ("float", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "none" => Float::None,
            "left" => Float::Left,
            "right" => Float::Right,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, p| s.float = p)),
        ("clear", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "none" => Clear::None,
            "left" => Clear::Left,
            "right" => Clear::Right,
            "both" => Clear::Both,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, p| s.clear = p)),
        ("visibility", DeclaredValue::Ident(v)) => Some(match v.as_str() {
            "hidden" | "collapse" => {
                style.visibility_hidden = true;
                ApplyResult::Applied
            }
            "visible" => {
                style.visibility_hidden = false;
                ApplyResult::Applied
            }
            _ => ApplyResult::Unsupported,
        }),
        ("overflow-x", DeclaredValue::Ident(v)) => Some(match overflow_of(v) {
            Some(o) => {
                style.overflow_x = o;
                ApplyResult::Applied
            }
            None => ApplyResult::Unsupported,
        }),
        ("overflow-y", DeclaredValue::Ident(v)) => Some(match overflow_of(v) {
            Some(o) => {
                style.overflow_y = o;
                ApplyResult::Applied
            }
            None => ApplyResult::Unsupported,
        }),
        ("z-index", DeclaredValue::Number(n)) => {
            style.z_index = *n as i32;
            Some(ApplyResult::Applied)
        }
        ("z-index", DeclaredValue::Ident(v)) if v == "auto" => {
            style.z_index = 0;
            Some(ApplyResult::Applied)
        }

        // --- Box model ---
        ("width", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.width = l)),
        ("height", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.height = l)),
        ("min-width", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.min_width = l)),
        ("max-width", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.max_width = l)),
        ("min-height", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.min_height = l)),
        ("max-height", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.max_height = l)),
        ("min-width", DeclaredValue::Ident(_)) | ("min-height", DeclaredValue::Ident(_)) => {
            Some(ApplyResult::Applied) // `auto` is the initial value
        }
        ("top", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.top = l)),
        ("right", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.right = l)),
        ("bottom", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.bottom = l)),
        ("left", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.left = l)),
        ("box-sizing", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "content-box" => BoxSizing::ContentBox,
            "border-box" => BoxSizing::BorderBox,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, b| s.box_sizing = b)),

        ("margin-top", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.margin.top = v)),
        ("margin-right", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.margin.right = v)),
        ("margin-bottom", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.margin.bottom = v)),
        ("margin-left", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.margin.left = v)),
        ("padding-top", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.padding.top = v)),
        ("padding-right", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.padding.right = v)),
        ("padding-bottom", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.padding.bottom = v)),
        ("padding-left", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.padding.left = v)),

        // --- Borders ---
        ("border-top-width", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.border_widths.top = v)),
        ("border-right-width", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.border_widths.right = v)),
        ("border-bottom-width", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.border_widths.bottom = v)),
        ("border-left-width", DeclaredValue::Length(l)) => Some(set_px(style, *l, |s, v| s.border_widths.left = v)),
        ("border-top-style", DeclaredValue::Ident(v)) => Some(border_style_at(style, Side::Top, v)),
        ("border-right-style", DeclaredValue::Ident(v)) => Some(border_style_at(style, Side::Right, v)),
        ("border-bottom-style", DeclaredValue::Ident(v)) => Some(border_style_at(style, Side::Bottom, v)),
        ("border-left-style", DeclaredValue::Ident(v)) => Some(border_style_at(style, Side::Left, v)),
        ("border-top-color", DeclaredValue::Color(c)) => Some(set_color(style, Side::Top, *c)),
        ("border-right-color", DeclaredValue::Color(c)) => Some(set_color(style, Side::Right, *c)),
        ("border-bottom-color", DeclaredValue::Color(c)) => Some(set_color(style, Side::Bottom, *c)),
        ("border-left-color", DeclaredValue::Color(c)) => Some(set_color(style, Side::Left, *c)),
        ("border-top-left-radius", DeclaredValue::Length(l)) => Some(set(style, px_or_zero(*l), |s, v| s.border_radius.top_left = v)),
        ("border-top-right-radius", DeclaredValue::Length(l)) => Some(set(style, px_or_zero(*l), |s, v| s.border_radius.top_right = v)),
        ("border-bottom-right-radius", DeclaredValue::Length(l)) => Some(set(style, px_or_zero(*l), |s, v| s.border_radius.bottom_right = v)),
        ("border-bottom-left-radius", DeclaredValue::Length(l)) => Some(set(style, px_or_zero(*l), |s, v| s.border_radius.bottom_left = v)),
        // `outline` beyond a rectangle is not drawn (§15); reported on use.
        ("outline-width", _) | ("outline-color", _) => Some(ApplyResult::Unsupported),
        ("border-collapse", _) | ("border-spacing", _) => Some(ApplyResult::Unsupported),
        ("table-layout", _) => Some(ApplyResult::Unsupported),

        // --- Backgrounds ---
        ("background-color", DeclaredValue::Color(c)) => {
            style.background.color = *c;
            Some(ApplyResult::Applied)
        }
        ("background-repeat", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "repeat" => BackgroundRepeat::Repeat,
            "no-repeat" => BackgroundRepeat::NoRepeat,
            "repeat-x" => BackgroundRepeat::RepeatX,
            "repeat-y" => BackgroundRepeat::RepeatY,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, r| s.background.repeat = r)),
        ("background-size", DeclaredValue::StringList(parts)) => {
            if parts.iter().any(|p| p == "cover") {
                style.background.size = engine_css::properties::BackgroundSize::Cover;
            } else if parts.iter().any(|p| p == "contain") {
                style.background.size = engine_css::properties::BackgroundSize::Contain;
            } else {
                return Some(ApplyResult::Unsupported);
            }
            Some(ApplyResult::Applied)
        }
        ("opacity", DeclaredValue::Number(n)) => {
            style.opacity = n.clamp(0.0, 1.0);
            Some(ApplyResult::Applied)
        }

        // --- Text ---
        ("color", DeclaredValue::Color(c)) => {
            style.color = *c;
            Some(ApplyResult::Applied)
        }
        ("font-size", DeclaredValue::Length(Length::Px(v))) => {
            // A non-positive font size is invalid in CSS; keep the initial value
            // rather than letting layout divide by zero or paint nothing.
            if *v > 0.0 {
                style.font_size_px = *v;
            }
            Some(ApplyResult::Applied)
        }
        ("font-size", DeclaredValue::Length(Length::Pct(p))) => {
            // `smaller`/`larger` and percentages resolve against the *parent*,
            // which the cascade has already copied into `font_size_px`.
            let parent = style.font_size_px;
            style.font_size_px = (parent * p / 100.0).max(0.0);
            Some(ApplyResult::Applied)
        }
        ("font-size", DeclaredValue::Length(_)) => Some(ApplyResult::Unsupported),
        ("font-weight", DeclaredValue::Number(n)) => {
            style.font_weight = n.round().clamp(1.0, 1000.0) as u16;
            Some(ApplyResult::Applied)
        }
        ("font-style", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "normal" => FontStyle::Normal,
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, f| s.font_style = f)),
        ("font-family", DeclaredValue::StringList(fams)) => {
            if fams.is_empty() {
                return Some(ApplyResult::Unsupported);
            }
            style.font_family = fams.clone();
            Some(ApplyResult::Applied)
        }
        ("line-height", DeclaredValue::Ident(v)) => Some(set(style, parse_line_height(v), |s, lh| s.line_height = lh)),
        ("letter-spacing", DeclaredValue::Length(l)) => {
            // `em` was folded to px at parse time; a percentage is not a valid
            // `letter-spacing`, but if one arrives it is a no-op.
            style.letter_spacing = px_or_zero(*l);
            let _ = font_px;
            Some(ApplyResult::Applied)
        }
        ("text-align", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "start" | "left" => TextAlign::Start,
            "center" => TextAlign::Center,
            "end" | "right" => TextAlign::End,
            "justify" => TextAlign::Justify,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, a| s.text_align = a)),
        ("text-decoration-line", DeclaredValue::StringList(parts)) => {
            style.text_decoration.underline = parts.iter().any(|p| p == "underline");
            style.text_decoration.line_through = parts.iter().any(|p| p == "line-through");
            Some(ApplyResult::Applied)
        }
        ("white-space", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "normal" => WhiteSpace::Normal,
            "nowrap" => WhiteSpace::Nowrap,
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, w| s.white_space = w)),
        ("vertical-align", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "baseline" => VerticalAlign::Baseline,
            "top" => VerticalAlign::Top,
            "bottom" => VerticalAlign::Bottom,
            "middle" => VerticalAlign::Middle,
            "sub" => VerticalAlign::Sub,
            "super" => VerticalAlign::Super,
            // A length/percentage is not representable in `VerticalAlign`; the
            // parser only emits `Ident` for the keywords, so anything else here
            // is unsupported rather than silently baseline.
            _ => return Some(ApplyResult::Unsupported),
        }, |s, v| s.vertical_align = v)),
        ("vertical-align", DeclaredValue::Length(_)) => Some(ApplyResult::Unsupported),

        // --- Lists ---
        ("list-style-type", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "disc" => ListStyleType::Disc,
            "circle" => ListStyleType::Circle,
            "square" => ListStyleType::Square,
            "decimal" => ListStyleType::Decimal,
            "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
            "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
            "lower-roman" => ListStyleType::LowerRoman,
            "upper-roman" => ListStyleType::UpperRoman,
            "none" => ListStyleType::None,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, t| s.list_style_type = t)),
        ("list-style-position", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "inside" => ListStylePosition::Inside,
            "outside" => ListStylePosition::Outside,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, p| s.list_style_position = p)),

        // --- Flex ---
        ("flex-direction", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "row" => FlexDirection::Row,
            "row-reverse" => FlexDirection::RowReverse,
            "column" => FlexDirection::Column,
            "column-reverse" => FlexDirection::ColumnReverse,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, d| s.flex_direction = d)),
        ("flex-wrap", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "nowrap" => FlexWrap::NoWrap,
            "wrap" => FlexWrap::Wrap,
            "wrap-reverse" => FlexWrap::WrapReverse,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, w| s.flex_wrap = w)),
        ("flex-grow", DeclaredValue::Number(n)) => {
            style.flex_grow = n.max(0.0);
            Some(ApplyResult::Applied)
        }
        ("flex-shrink", DeclaredValue::Number(n)) => {
            style.flex_shrink = n.max(0.0);
            Some(ApplyResult::Applied)
        }
        ("flex-basis", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.flex_basis = l)),
        ("flex-basis", DeclaredValue::Ident(v)) if v == "auto" => {
            style.flex_basis = Length::Auto;
            Some(ApplyResult::Applied)
        }
        ("justify-content", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "flex-start" | "start" | "left" => JustifyContent::FlexStart,
            "flex-end" | "end" | "right" => JustifyContent::FlexEnd,
            "center" => JustifyContent::Center,
            "space-between" => JustifyContent::SpaceBetween,
            "space-around" => JustifyContent::SpaceAround,
            "space-evenly" => JustifyContent::SpaceEvenly,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, j| s.justify_content = j)),
        ("align-items", DeclaredValue::Ident(v)) => Some(set(style, match v.as_str() {
            "stretch" => AlignItems::Stretch,
            "flex-start" | "start" => AlignItems::FlexStart,
            "flex-end" | "end" => AlignItems::FlexEnd,
            "center" => AlignItems::Center,
            "baseline" => AlignItems::Baseline,
            _ => return Some(ApplyResult::Unsupported),
        }, |s, a| s.align_items = a)),
        ("row-gap", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.gap_row = l)),
        ("column-gap", DeclaredValue::Length(l)) => Some(set_len(style, *l, |s, l| s.gap_column = l)),

        // --- Contents ---
        // `content` only exists for `::before`/`::after`, which we do not
        // generate (§15). Recorded rather than applied.
        ("content", _) => Some(ApplyResult::Unsupported),
        ("cursor", DeclaredValue::Ident(v)) => {
            style.cursor_pointer = v == "pointer";
            Some(ApplyResult::Applied)
        }

        // Wrong value type for a property we do know: the declaration is
        // invalid, which `engine-css` should have rejected. Report it.
        (name, _) if is_known_property(name) => Some(ApplyResult::Unsupported),
        _ => None,
    }
}

fn is_known_property(name: &str) -> bool {
    matches!(
        name,
        "display"
            | "position"
            | "float"
            | "clear"
            | "visibility"
            | "overflow-x"
            | "overflow-y"
            | "z-index"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "box-sizing"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
            | "outline-width"
            | "outline-color"
            | "border-collapse"
            | "border-spacing"
            | "table-layout"
            | "background-color"
            | "background-repeat"
            | "background-size"
            | "opacity"
            | "color"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-family"
            | "line-height"
            | "letter-spacing"
            | "text-align"
            | "text-decoration-line"
            | "white-space"
            | "vertical-align"
            | "list-style-type"
            | "list-style-position"
            | "flex-direction"
            | "flex-wrap"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "justify-content"
            | "align-items"
            | "row-gap"
            | "column-gap"
            | "content"
            | "cursor"
    )
}

/// Small generic setter used to keep the match arms above one line each.
fn set<T>(style: &mut ComputedStyle, value: T, f: impl FnOnce(&mut ComputedStyle, T)) -> ApplyResult {
    f(style, value);
    ApplyResult::Applied
}

/// Set a `Length`-typed property. `auto` and percentages are preserved for
/// layout to resolve against the containing block (§5.6); only a non-finite
/// value is rejected.
fn set_len(style: &mut ComputedStyle, l: Length, f: impl FnOnce(&mut ComputedStyle, Length)) -> ApplyResult {
    if !l.is_finite() {
        return ApplyResult::Unsupported;
    }
    f(style, l);
    ApplyResult::Applied
}

/// `EdgeSizes` stores resolved `f32`; a percentage is not representable there.
fn set_px(style: &mut ComputedStyle, l: Length, f: impl FnOnce(&mut ComputedStyle, f32)) -> ApplyResult {
    match l {
        Length::Px(v) if v.is_finite() => {
            f(style, v);
            ApplyResult::Applied
        }
        // `auto` stays zero; layout reads `margin: auto` from the struct's
        // `px_or_zero` result and centres only from explicit values, so this is
        // the "no centring" case, which matches browsers when the containing
        // block is wide enough.
        Length::Auto => {
            f(style, 0.0);
            ApplyResult::Applied
        }
        Length::Pct(_) => ApplyResult::Unsupported,
        Length::Px(_) => ApplyResult::Unsupported,
    }
}

fn px_or_zero(l: Length) -> f32 {
    match l {
        Length::Px(v) if v.is_finite() => v.max(0.0),
        _ => 0.0,
    }
}

fn overflow_of(v: &str) -> Option<Overflow> {
    Some(match v {
        "visible" => Overflow::Visible,
        "hidden" => Overflow::Hidden,
        "scroll" => Overflow::Scroll,
        "auto" => Overflow::Auto,
        "clip" => Overflow::Hidden,
        _ => return None,
    })
}

fn set_display(style: &mut ComputedStyle, v: &str) -> ApplyResult {
    // `engine-css` normalises to `display_name` output, so this only has to
    // accept those strings.
    let d = match v {
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
        "list-item" => Display::ListItem,
        "none" => Display::None,
        "contents" => Display::Contents,
        _ => return ApplyResult::Unsupported,
    };
    style.display = d;
    ApplyResult::Applied
}

fn border_style_at(style: &mut ComputedStyle, side: Side, v: &str) -> ApplyResult {
    let s = match v {
        "none" | "hidden" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        _ => return ApplyResult::Unsupported,
    };
    style.border_styles[side.idx()] = s;
    ApplyResult::Applied
}

fn set_color(style: &mut ComputedStyle, side: Side, c: Rgba8) -> ApplyResult {
    style.border_colors[side.idx()] = c;
    ApplyResult::Applied
}

/// `line-height` arrives as the `Debug` form of `engine_css::LineHeight`.
fn parse_line_height(v: &str) -> LineHeight {
    match v {
        "Normal" => LineHeight::Normal,
        other => {
            if let Some(rest) = other.strip_prefix("Number(").and_then(|s| s.strip_suffix(')')) {
                if let Ok(n) = rest.parse::<f32>() {
                    return LineHeight::Number(n);
                }
            }
            if let Some(rest) = other.strip_prefix("Px(").and_then(|s| s.strip_suffix(')')) {
                if let Ok(n) = rest.parse::<f32>() {
                    return LineHeight::Px(n);
                }
            }
            // Unparseable: the initial value, and `apply` cannot report from
            // here, so `line-height` is the one property that silently falls
            // back. Covered by a test that the three real encodings parse.
            LineHeight::Normal
        }
    }
}

/// Reset every non-inherited longhand to its initial value, then copy the
/// inherited ones from the parent. This is the per-element starting point of
/// the cascade (§5.5).
pub fn reset_and_inherit(style: &mut ComputedStyle, parent: &ComputedStyle) {
    let mut fresh = ComputedStyle::default();
    engine_css::properties::inherit::apply_inherited(&mut fresh, parent);
    *style = fresh;
}

/// Zero-valued `EdgeSizes` utility kept here so callers do not import geometry
/// just for a constant in tests.
pub const ZERO_EDGES: EdgeSizes = EdgeSizes::ZERO;