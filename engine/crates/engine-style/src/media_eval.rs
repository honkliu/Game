//! `@media` evaluation (§4.3 Stage 3, §5.2).
//!
//! Only `screen` matches; `print` and every other media type never does. The
//! features we evaluate are the ones `engine-css` parses: `width`, `height`,
//! `aspect-ratio`, `orientation`, and their `min-`/`max-` forms, plus
//! `prefers-color-scheme` and `prefers-reduced-motion`. Anything else is
//! `Unsupported` and evaluates false, which is the conservative side: a rule
//! that was meant for a feature we do not implement is not applied.

use engine_core::{Length, Viewport};
use engine_css::MediaQuery;

/// The device facts a media query can ask about.
#[derive(Clone, Copy, Debug)]
pub struct MediaContext {
    pub viewport: Viewport,
    /// We are a light-mode-only engine; both preferences are fixed so goldens
    /// do not depend on the host machine's theme.
    pub dark: bool,
    pub reduced_motion: bool,
}

impl MediaContext {
    pub fn new(viewport: Viewport) -> Self {
        MediaContext { viewport, dark: false, reduced_motion: false }
    }
}

/// Evaluate one query.
pub fn eval(q: &MediaQuery, ctx: &MediaContext) -> bool {
    match q {
        MediaQuery::All | MediaQuery::Screen => true,
        MediaQuery::Never => false,
        MediaQuery::And(parts) => parts.iter().all(|p| eval(p, ctx)),
        MediaQuery::Or(parts) => parts.iter().any(|p| eval(p, ctx)),
        MediaQuery::Not(inner) => !eval(inner, ctx),
        MediaQuery::Unsupported(_) => false,
        MediaQuery::Feature { name, value } => feature(name, *value, ctx),
    }
}

/// Every query in a rule's `media` list must hold for the rule to apply.
pub fn all_hold(queries: &[MediaQuery], ctx: &MediaContext) -> bool {
    queries.iter().all(|q| eval(q, ctx))
}

fn feature(name: &str, value: Length, ctx: &MediaContext) -> bool {
    let vp = ctx.viewport;
    let px = match value {
        Length::Px(p) => p,
        // A media feature value is never a percentage in a parseable query; if
        // one arrives it cannot be satisfied.
        _ => return false,
    };
    // `engine-css` encodes keyword values as magic numbers (see
    // `stylesheet::keyword_code`): 1 = landscape, 2 = portrait, 3 = dark,
    // 4 = light, 5 = reduce, 6 = no-preference.
    match name {
        "width" => (vp.width_css - px).abs() < 0.5,
        "min-width" => vp.width_css >= px,
        "max-width" => vp.width_css <= px,
        "height" => (vp.height_css - px).abs() < 0.5,
        "min-height" => vp.height_css >= px,
        "max-height" => vp.height_css <= px,
        "aspect-ratio" => (vp.width_css / vp.height_css - px).abs() < 1e-3,
        "orientation" => match px as i32 {
            1 => vp.width_css >= vp.height_css,
            2 => vp.width_css < vp.height_css,
            _ => false,
        },
        "prefers-color-scheme" => match px as i32 {
            3 => ctx.dark,
            4 => !ctx.dark,
            _ => false,
        },
        "prefers-reduced-motion" => match px as i32 {
            5 => ctx.reduced_motion,
            6 => !ctx.reduced_motion,
            _ => false,
        },
        // Everything else (`color`, `resolution`, `hover`, ...) is unsupported.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_css::{parse_stylesheet, Origin, StyleRule};

    fn ctx(w: f32, h: f32) -> MediaContext {
        MediaContext::new(Viewport::new(w, h, 1.0))
    }

    fn queries(src: &str) -> Vec<MediaQuery> {
        let sheet = parse_stylesheet(src, Origin::Author);
        sheet.rules.iter().map(|r: &StyleRule| r.media[0].clone()).collect()
    }

    #[test]
    fn min_width_boundary() {
        let q = &queries("@media (min-width: 600px) { p { color: red } }")[0];
        assert!(eval(q, &ctx(600.0, 800.0)), "min-width is inclusive");
        assert!(!eval(q, &ctx(599.0, 800.0)));
        assert!(eval(q, &ctx(1200.0, 800.0)));
    }

    #[test]
    fn max_width_boundary() {
        let q = &queries("@media (max-width: 600px) { p { color: red } }")[0];
        assert!(eval(q, &ctx(600.0, 800.0)));
        assert!(!eval(q, &ctx(601.0, 800.0)));
    }

    #[test]
    fn screen_matches_print_does_not() {
        let sheet = parse_stylesheet(
            "@media screen { p { color: red } } @media print { p { color: blue } }",
            Origin::Author,
        );
        assert_eq!(sheet.rules.len(), 2);
        assert!(all_hold(&sheet.rules[0].media, &ctx(800.0, 600.0)));
        assert!(!all_hold(&sheet.rules[1].media, &ctx(800.0, 600.0)));
    }

    #[test]
    fn orientation_and_and() {
        let q = &queries("@media screen and (min-width: 500px) and (orientation: landscape) { p { color: red } }")[0];
        assert!(eval(q, &ctx(800.0, 600.0)));
        assert!(!eval(q, &ctx(400.0, 600.0)), "fails min-width");
        assert!(!eval(q, &ctx(600.0, 800.0)), "portrait fails landscape");
    }

    #[test]
    fn not_inverts() {
        let q = &queries("@media not screen { p { color: red } }")[0];
        assert!(!eval(q, &ctx(800.0, 600.0)));
    }

    #[test]
    fn unknown_feature_is_false() {
        let q = &queries("@media (hover: hover) { p { color: red } }")[0];
        assert!(!eval(q, &ctx(800.0, 600.0)), "unsupported feature must not match");
    }
}