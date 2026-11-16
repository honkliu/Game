//! Adversarial-input tests for the CSS parser.
//!
//! Origin: a tokenizer state in this crate failed to advance, turning the
//! four-byte input `.a` into an infinite loop that grew a `Vec` until the
//! allocator asked for 10 GB. Every test here exists to make that class of bug
//! loud and local: the parse must terminate with a result on any input, and
//! recursion into attacker-controlled nesting must be capped by a named
//! constant and reported as an error.

use engine_css::selector_parser::{parse_selector_list, MAX_SELECTOR_NESTING};
use engine_css::values::{finite_f32, parse_calc, MAX_CALC_DEPTH};
use engine_css::{parse_stylesheet, tokenize, tokenize_with_errors, LengthCtx, Origin};

/// Tokens plus errors, from the one entry point that returns both.
fn tok(src: &str) -> (Vec<engine_css::Token>, Vec<engine_css::tokenizer::state::TokenizeError>) {
    tokenize_with_errors(src)
}

fn sheet(src: &str) -> engine_css::Stylesheet {
    parse_stylesheet(src, Origin::Author)
}

// --- Tokenizer termination ------------------------------------------------

#[test]
fn tokenizer_terminates_on_the_original_trigger_and_neighbours() {
    // `.a` is the reported trigger. The rest are the other ways to reach a
    // "delimiter that must not enter a number" path.
    for src in [".a", ".", "..", ". .", "-", "+", "-a", "+a", "-.", "#", "#.", "@", "\\"] {
        let (toks, errs) = tok(src);
        assert_eq!(toks.last(), Some(&engine_css::Token::Eof), "{src:?}");
        assert!(
            !errs.iter().any(|e| e.kind == "tokenizer-made-no-progress"),
            "{src:?}: tokenizer failed to advance"
        );
    }
}

#[test]
fn tokenizer_has_a_progress_guard() {
    // If the guard itself were removed and a regression reintroduced a
    // non-advancing state, this test would hang rather than fail. That is
    // acceptable: a hang *is* the finding, and the test name says so.
    let big = ".".repeat(1_000_000);
    let (toks, _) = tok(&big);
    assert_eq!(toks.iter().filter(|t| **t == engine_css::Token::Delim('.')).count(), 1_000_000);
    assert_eq!(toks.last(), Some(&engine_css::Token::Eof));
}

#[test]
fn tokenizer_terminates_on_unterminated_constructs() {
    for src in [
        "/*", "/* /* /*", "\"", "'", "\"abc", "url(", "url(x", "url('x", "url(x y)", "\\",
        "\\\n", "\\0", "\\110000", "a{", "}", ")))]]]", "<!--", "-->", "calc(",
    ] {
        let (toks, errs) = tok(src);
        assert_eq!(toks.last(), Some(&engine_css::Token::Eof), "{src:?}");
        assert!(
            !errs.iter().any(|e| e.kind == "tokenizer-made-no-progress"),
            "{src:?}: tokenizer failed to advance"
        );
    }
}

// --- Stylesheet parsing termination --------------------------------------

#[test]
fn stylesheet_parse_terminates_on_structural_abuse() {
    let cases = [
        "{".repeat(100_000),
        "}".repeat(100_000),
        "a{".repeat(100_000),
        "a{color:red;".repeat(50_000),
        "@media screen{".repeat(50_000),
        "@supports (a:b){".repeat(20_000),
        "(".repeat(100_000),
        "[".repeat(100_000),
        "a".repeat(1_000_000),
        ";".repeat(1_000_000),
        ":".repeat(1_000_000),
        ",".repeat(1_000_000),
    ];
    for src in cases {
        let s = sheet(&src);
        // The contract is "returned with a report", not "produced no rules":
        // malformed recovery legitimately yields something.
        let _ = (s.rules.len(), s.errors.len(), s.skipped_at_rules.len());
    }
}

#[test]
fn empty_and_trivial_sheets_parse() {
    for src in ["", " ", "\n\t", "/*x*/", "p{}", "p{}p{}"] {
        let s = sheet(src);
        assert!(s.errors.first().is_none_or(|e| !e.message.is_empty()));
    }
}

// --- Numeric overflow from the page --------------------------------------

#[test]
fn infinite_lengths_are_rejected_not_propagated() {
    // `f64` saturates to infinity past ~308 digits; CSS allows any number of
    // digits, so this is reachable from a page. `Length::Px(inf)` would become
    // an infinite layout coordinate and then an infinite raster target.
    let huge = "9".repeat(400);
    for prop in ["width", "height", "margin-top", "padding-left", "top", "font-size"] {
        let s = sheet(&format!("a{{{prop}:{huge}px}}"));
        assert!(
            s.rules.first().is_none_or(|r| r.declarations.get(prop).is_none()),
            "{prop}: infinite length was accepted"
        );
    }
    // A unit-scaled overflow (`1e38em` is finite, `* 16` is not) is caught too.
    let s = sheet("a{width:1e38em}");
    assert!(s.rules.first().is_none_or(|r| r.declarations.get("width").is_none()));
}

#[test]
fn unitless_helper_rejects_non_finite() {
    assert_eq!(finite_f32(1.0), Some(1.0));
    assert_eq!(finite_f32(f64::INFINITY), None);
    assert_eq!(finite_f32(f64::NAN), None);
    assert_eq!(finite_f32(1e40), None, "overflows f32");
    assert_eq!(finite_f32(f64::MAX), None);
}

#[test]
fn z_index_does_not_saturate_to_i32_max() {
    // `inf as f32 as i32` saturates to i32::MAX, which would pin the element
    // above every other box on the page.
    let s = sheet(&format!("a{{z-index:{}}}", "9".repeat(400)));
    assert!(s.rules.first().is_none_or(|r| r.declarations.get("z-index").is_none()));
    // In-range values still work.
    let s = sheet("a{z-index:3}");
    assert!(s.rules.first().is_some_and(|r| r.declarations.get("z-index").is_some()));
}

// --- Selector recursion ---------------------------------------------------

#[test]
fn selector_nesting_is_capped_with_an_error() {
    let depth = MAX_SELECTOR_NESTING as usize + 10;
    let src = format!("{}b{}", ":not(".repeat(depth), ")".repeat(depth));
    let toks = tokenize(&src);
    let r = parse_selector_list(&toks);
    assert!(r.is_err(), "nesting past the cap must be a parse error, not a panic");
    let msg = r.err().map(|e| e.message).unwrap_or_default();
    assert!(msg.contains("nesting"), "error should name the cause, got {msg:?}");
}

#[test]
fn selector_nesting_below_the_cap_still_parses() {
    for depth in [1usize, 10, 32, 63] {
        let src = format!("{}b{}", ":not(".repeat(depth), ")".repeat(depth));
        let toks = tokenize(&src);
        let r = parse_selector_list(&toks);
        assert!(r.is_ok(), "depth {depth} should be within the cap: {:?}", r.err());
    }
}

#[test]
fn deeply_nested_selector_argument_does_not_overflow_the_stack() {
    // A recursive-descent selector parser would die here. The cap converts the
    // input into a parse error before the stack is at risk.
    for unit in [":not(", ":is(", ":where(", ":has("] {
        let depth = 200_000;
        let src = format!("a{unit}{}b{}", unit.repeat(depth), ")".repeat(depth + 1));
        let toks = tokenize(&src);
        let r = parse_selector_list(&toks);
        assert!(r.is_err(), "{unit}: unbounded nesting must be rejected");
    }
}

#[test]
fn nth_child_of_nesting_is_capped_too() {
    let depth = MAX_SELECTOR_NESTING as usize + 5;
    let src = format!(
        "a{}{}b{}",
        ":nth-child(1 of ".repeat(depth),
        "",
        ")".repeat(depth)
    );
    let toks = tokenize(&src);
    let r = parse_selector_list(&toks);
    assert!(r.is_err(), "`of S` nesting must be capped: {r:?}");
}

#[test]
fn at_rule_block_nesting_is_capped_with_an_error() {
    // `at_rule` recurses once per nested `@media` block (its body loop calls
    // `at_rule` again for a nested `@AtKeyword`). Before the cap,
    // `"@media screen{".repeat(50_000)` — a 750 KB sheet — overflowed the
    // stack (STATUS_STACK_BUFFER_OVERRUN on Windows: a process abort, not a
    // catchable panic).
    //
    // `@supports` is *not* covered by this: it collects its body into a fresh
    // flat sub-parser (`block_body`) and so does not recurse. An earlier draft
    // of this test asserted the error for `@supports` too and failed, which is
    // how that was established.
    let src = "@media screen{".repeat(50_000);
    let s = sheet(&src);
    assert!(
        s.errors.iter().any(|e| e.message.contains("at-rule nesting")),
        "expected a recorded nesting error, got {:?}",
        s.errors.iter().map(|e| &e.message).take(3).collect::<Vec<_>>()
    );
    assert!(s.skipped_at_rules.iter().any(|r| r == "@media"));
}

#[test]
fn at_rule_nesting_below_the_cap_still_applies() {
    use engine_css::stylesheet::MAX_AT_RULE_NESTING;
    let depth = MAX_AT_RULE_NESTING as usize - 1;
    let src = format!("{}a{{color:red}}{}", "@media screen{".repeat(depth), "}".repeat(depth));
    let s = sheet(&src);
    assert_eq!(s.rules.len(), 1, "the innermost rule must survive: {:?}", s.errors);
}

// --- calc recursion (latent: not yet reachable from declarations) --------

#[test]
fn calc_paren_nesting_is_capped() {
    let inner = format!("{}1px{}", "(".repeat(MAX_CALC_DEPTH as usize + 5), ")".repeat(MAX_CALC_DEPTH as usize + 5));
    let toks = tokenize(&inner);
    let r = parse_calc(&toks[..toks.len() - 1], &LengthCtx::default());
    assert!(r.is_none(), "calc nesting past the cap must be rejected");
}

#[test]
fn calc_within_the_cap_evaluates() {
    let inner = format!("{}1px{}", "(".repeat(10), ")".repeat(10));
    let toks = tokenize(&inner);
    assert_eq!(
        parse_calc(&toks[..toks.len() - 1], &LengthCtx::default()),
        Some(engine_core::Length::Px(1.0))
    );
}

// --- Fuzz-lite over every prefix -----------------------------------------

#[test]
fn every_prefix_of_a_hostile_sheet_parses() {
    let full = "@media screen and (min-width:600px){a[href]:not(.x):nth-child(2n+1){color:#fff;width:calc(100% - 2px)}}@import url(x.css);@font-face{font-family:X}";
    for i in 0..=full.len() {
        if full.is_char_boundary(i) {
            let s = sheet(&full[..i]);
            let _ = s.rules.len();
            // Tokenizing a prefix must also always terminate.
            let (t, _) = tok(&full[..i]);
            assert_eq!(t.last(), Some(&engine_css::Token::Eof), "prefix {i}");
        }
    }
}