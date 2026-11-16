//! CSS lengths, units, and the `calc()` expression tree.
//!
//! The computed value of a length is kept *unresolved* where the CSS spec keeps
//! it unresolved: percentages need a containing block, `em` needs a font size,
//! `rem` needs the root font size. `engine-style` resolves `em`/`rem`/viewport
//! units down to `Px` or `Pct` at computed-value time (§5.6); layout resolves
//! `Pct` against the correct containing block.

use std::fmt;

/// A CSS length as it reaches layout. `Pct` is intentionally left in the type
/// because only layout knows the containing block (§5.6).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Length {
    /// Device pixels (CSS px multiplied by the device pixel ratio at
    /// computed-value time).
    Px(f32),
    /// Percentage of the containing block's relevant axis.
    Pct(f32),
    /// `auto` — resolved by the layout algorithm.
    #[default]
    Auto,
}

impl Length {
    pub const ZERO: Length = Length::Px(0.0);

    pub fn px(v: f32) -> Self {
        Length::Px(v)
    }

    pub fn pct(v: f32) -> Self {
        Length::Pct(v)
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Length::Auto)
    }

    pub fn is_zero(&self) -> bool {
        matches!(self, Length::Px(v) if *v == 0.0) || matches!(self, Length::Pct(v) if *v == 0.0)
    }

    /// Resolve against a containing-block extent. `auto` resolves to `base`
    /// (the caller decides; used for `margin: auto` centering paths only where
    /// the caller has already decided auto means zero).
    pub fn resolve(&self, base: f32) -> f32 {
        match *self {
            Length::Px(v) => v,
            Length::Pct(p) => base * p / 100.0,
            Length::Auto => base,
        }
    }

    /// Resolve, treating `auto` as zero. This is the common case for padding,
    /// border widths, and offsets where `auto` has already been handled.
    pub fn resolve_auto_zero(&self, base: f32) -> f32 {
        match *self {
            Length::Px(v) => v,
            Length::Pct(p) => base * p / 100.0,
            Length::Auto => 0.0,
        }
    }

    pub fn scale(&self, k: f32) -> Length {
        match *self {
            Length::Px(v) => Length::Px(v * k),
            Length::Pct(v) => Length::Pct(v * k),
            Length::Auto => Length::Auto,
        }
    }

    /// True if both lengths are finite (auto is finite by definition).
    pub fn is_finite(&self) -> bool {
        match *self {
            Length::Px(v) | Length::Pct(v) => v.is_finite(),
            Length::Auto => true,
        }
    }
}

/// Absolute-length unit conversions. The CSS reference pixel is 1/96 inch
/// (CSS Values 3 §6.2), so every absolute unit is a rational multiple of 96 px.
pub mod units {
    /// CSS px per inch.
    pub const PX_PER_IN: f32 = 96.0;
    /// CSS px per cm (96 / 2.54).
    pub const PX_PER_CM: f32 = 96.0 / 2.54;
    /// CSS px per mm (96 / 25.4).
    pub const PX_PER_MM: f32 = 96.0 / 25.4;
    /// CSS px per pt (1/72 in).
    pub const PX_PER_PT: f32 = 96.0 / 72.0;
    /// CSS px per pc (12 pt).
    pub const PX_PER_PC: f32 = 96.0 / 6.0;
    /// CSS px per Q (1/40 cm).
    pub const PX_PER_Q: f32 = PX_PER_CM / 40.0;

    pub fn cm_to_px(v: f32) -> f32 {
        v * PX_PER_CM
    }
    pub fn mm_to_px(v: f32) -> f32 {
        v * PX_PER_MM
    }
    pub fn in_to_px(v: f32) -> f32 {
        v * PX_PER_IN
    }
    pub fn pt_to_px(v: f32) -> f32 {
        v * PX_PER_PT
    }
    pub fn pc_to_px(v: f32) -> f32 {
        v * PX_PER_PC
    }
    pub fn q_to_px(v: f32) -> f32 {
        v * PX_PER_Q
    }
}

/// A `calc()` tree node (CSS Values 4 accepted subset: `+ - * /` on lengths and
/// numbers; no mixed unit sums beyond px/pct which are kept symbolic).
#[derive(Clone, Debug, PartialEq)]
pub enum CalcExpr {
    Length(Length),
    Number(f32),
    Add(Box<CalcExpr>, Box<CalcExpr>),
    Sub(Box<CalcExpr>, Box<CalcExpr>),
    Mul(Box<CalcExpr>, Box<CalcExpr>),
    Div(Box<CalcExpr>, Box<CalcExpr>),
}

impl CalcExpr {
    pub fn length(l: Length) -> CalcExpr {
        CalcExpr::Length(l)
    }
    pub fn num(n: f32) -> CalcExpr {
        CalcExpr::Number(n)
    }

    /// Resolve to a `Length`. `em`/`rem`/viewport units must have been resolved
    /// to px/pct before this point (§5.6).
    ///
    /// A `px + %` sum is affine in the containing block and therefore not
    /// representable as a `Length`; such expressions evaluate to `None` and the
    /// cascade drops the declaration (reported, not silently wrong).
    pub fn eval(&self) -> Option<Length> {
        enum Value {
            /// A bare `number` literal — the only operand multiplication and
            /// division may take as their right side.
            Num(f32),
            Len(Length),
        }
        fn value(e: &CalcExpr) -> Option<Value> {
            match e {
                CalcExpr::Number(n) => Some(Value::Num(*n)),
                CalcExpr::Length(l) => Some(Value::Len(*l)),
                CalcExpr::Add(a, b) => match (value(a)?, value(b)?) {
                    (Value::Len(x), Value::Len(y)) => Some(Value::Len(add_like(x, y)?)),
                    // number + number is a plain number, not a length.
                    (Value::Num(x), Value::Num(y)) => Some(Value::Num(x + y)),
                    _ => None,
                },
                CalcExpr::Sub(a, b) => match (value(a)?, value(b)?) {
                    (Value::Len(x), Value::Len(y)) => Some(Value::Len(add_like(x, y.scale(-1.0))?)),
                    (Value::Num(x), Value::Num(y)) => Some(Value::Num(x - y)),
                    _ => None,
                },
                CalcExpr::Mul(a, b) => match (value(a)?, value(b)?) {
                    (Value::Num(k), Value::Len(l)) | (Value::Len(l), Value::Num(k)) => {
                        Some(Value::Len(l.scale(k)))
                    }
                    (Value::Num(x), Value::Num(y)) => Some(Value::Num(x * y)),
                    _ => None,
                },
                CalcExpr::Div(a, b) => match (value(a)?, value(b)?) {
                    (Value::Len(l), Value::Num(k)) if k != 0.0 && k.is_finite() => {
                        Some(Value::Len(l.scale(1.0 / k)))
                    }
                    (Value::Num(x), Value::Num(y)) if y != 0.0 && y.is_finite() => {
                        Some(Value::Num(x / y))
                    }
                    _ => None,
                },
            }
        }
        match value(self)? {
            Value::Len(l) => Some(l),
            Value::Num(n) => Some(Length::Px(n)),
        }
    }
}

fn add_like(a: Length, b: Length) -> Option<Length> {
    match (a, b) {
        (Length::Auto, _) | (_, Length::Auto) => None,
        (Length::Px(x), Length::Px(y)) => Some(Length::Px(x + y)),
        (Length::Pct(x), Length::Pct(y)) => Some(Length::Pct(x + y)),
        // Affine in the containing block; not a `Length`. See `eval`.
        (Length::Px(_), Length::Pct(_)) | (Length::Pct(_), Length::Px(_)) => None,
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Length::Px(v) => write!(f, "{v}px"),
            Length::Pct(v) => write!(f, "{v}%"),
            Length::Auto => write!(f, "auto"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_resolves_trivially() {
        assert_eq!(Length::px(12.0).resolve(100.0), 12.0);
    }

    #[test]
    fn pct_resolves_against_base() {
        assert_eq!(Length::pct(50.0).resolve(200.0), 100.0);
        assert_eq!(Length::pct(12.5).resolve(800.0), 100.0);
    }

    #[test]
    fn auto_resolves_to_zero_when_asked() {
        assert_eq!(Length::Auto.resolve_auto_zero(100.0), 0.0);
    }

    #[test]
    fn absolute_units_against_96dpi_reference() {
        // 1in == 96px exactly; 2.54cm == 1in; 25.4mm == 1in; 72pt == 1in; 6pc == 1in.
        assert_eq!(units::in_to_px(1.0), 96.0);
        assert!((units::cm_to_px(2.54) - 96.0).abs() < 1e-3);
        assert!((units::mm_to_px(25.4) - 96.0).abs() < 1e-3);
        assert_eq!(units::pt_to_px(72.0), 96.0);
        assert_eq!(units::pc_to_px(6.0), 96.0);
        // 1Q = 1/40 cm = 0.25 mm (CSS Values 3 §6.2), so 40Q == 1cm == 37.795px.
        assert!((units::q_to_px(40.0) - units::PX_PER_CM).abs() < 1e-3);
        assert!((units::q_to_px(4.0) - units::PX_PER_MM).abs() < 1e-3);
    }

    #[test]
    fn calc_add_px() {
        let expr = CalcExpr::Add(
            Box::new(CalcExpr::Length(Length::Px(10.0))),
            Box::new(CalcExpr::Length(Length::Px(5.0))),
        );
        assert_eq!(expr.eval(), Some(Length::Px(15.0)));
    }

    #[test]
    fn calc_mul_by_number() {
        let expr = CalcExpr::Mul(
            Box::new(CalcExpr::Length(Length::Px(10.0))),
            Box::new(CalcExpr::Number(2.5)),
        );
        assert_eq!(expr.eval(), Some(Length::Px(25.0)));
    }

    #[test]
    fn calc_div_by_zero_rejected() {
        let expr = CalcExpr::Div(
            Box::new(CalcExpr::Length(Length::Px(10.0))),
            Box::new(CalcExpr::Number(0.0)),
        );
        assert_eq!(expr.eval(), None);
    }

    #[test]
    fn calc_div_by_number() {
        let expr = CalcExpr::Div(
            Box::new(CalcExpr::Length(Length::Px(30.0))),
            Box::new(CalcExpr::Number(3.0)),
        );
        assert_eq!(expr.eval(), Some(Length::Px(10.0)));
    }

    #[test]
    fn calc_px_plus_percent_is_rejected_not_mangled() {
        // Affine in the containing block; `Length` cannot represent it, so we
        // must drop rather than silently lose the percentage term.
        let expr = CalcExpr::Add(
            Box::new(CalcExpr::Length(Length::Px(10.0))),
            Box::new(CalcExpr::Length(Length::Pct(50.0))),
        );
        assert_eq!(expr.eval(), None);
    }

    #[test]
    fn calc_percent_scaled_by_number() {
        let expr = CalcExpr::Mul(
            Box::new(CalcExpr::Length(Length::Pct(50.0))),
            Box::new(CalcExpr::Number(0.5)),
        );
        assert_eq!(expr.eval(), Some(Length::Pct(25.0)));
    }

    #[test]
    fn calc_length_times_length_rejected() {
        let expr = CalcExpr::Mul(
            Box::new(CalcExpr::Length(Length::Px(2.0))),
            Box::new(CalcExpr::Length(Length::Px(3.0))),
        );
        assert_eq!(expr.eval(), None);
    }

    #[test]
    fn calc_sub_px() {
        let expr = CalcExpr::Sub(
            Box::new(CalcExpr::Length(Length::Px(30.0))),
            Box::new(CalcExpr::Length(Length::Px(12.0))),
        );
        assert_eq!(expr.eval(), Some(Length::Px(18.0)));
    }

    #[test]
    fn length_scale() {
        assert_eq!(Length::Px(2.0).scale(3.0), Length::Px(6.0));
        assert_eq!(Length::Pct(10.0).scale(0.5), Length::Pct(5.0));
        assert_eq!(Length::Auto.scale(2.0), Length::Auto);
    }

    #[test]
    fn display_formats() {
        assert_eq!(format!("{}", Length::Px(1.5)), "1.5px");
        assert_eq!(format!("{}", Length::Pct(50.0)), "50%");
        assert_eq!(format!("{}", Length::Auto), "auto");
    }
}