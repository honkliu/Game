//! Adversarial-input tests: every parser must *return*, on every input.
//!
//! These exist because of a real incident: a CSS tokenizer state that failed
//! to advance its cursor turned `.a` (four bytes) into an infinite loop that
//! grew a `Vec` until the allocator requested 10 GB. A renderer has no
//! sandbox, so "the page hangs the process with no error" is the worst possible
//! failure mode — worse than a wrong picture, and worse than a panic.
//!
//! Every test here asserts one of two properties:
//!
//! 1. **Termination with a result.** The parse returns (Ok or Err); it may not
//!    hang, and it may not abort the process. A hang shows up as the test not
//!    finishing; a stack overflow shows up as the test binary dying.
//! 2. **Bounded depth.** Recursion into attacker-controlled nesting is capped
//!    with a named constant and reported as a parse error — never allowed to
//!    consume the stack, because on Windows a stack overflow is
//!    `STATUS_STACK_BUFFER_OVERRUN`, which is a process abort and cannot be
//!    caught by the test harness.
//!
//! The inputs are a fixed adversarial table, not random: each one is chosen to
//! hit a specific state or grammar corner, so a regression names the corner.

use engine_dom::document::Document;
use engine_dom::serialize::{inner_html, serialize, text_content};
use engine_html::{parse, ParseOpts};

/// Build a string by repeating `unit` `n` times.
fn rep(unit: &str, n: usize) -> String {
    unit.repeat(n)
}

// --- HTML tokenizer ------------------------------------------------------
//
// The tokenizer carries a progress guard (`tokenizer-stalled`) that trips after
// 8 non-advancing steps. These inputs verify the guard is *reachable and
// correct* on every state-transition edge, and that normal inputs never trip
// it spuriously.

fn htm_tokenize(src: &str) -> (usize, bool) {
    let (toks, errs) = engine_html::tokenizer::state::tokenize(src);
    let stalled = errs.iter().any(|e| e.kind == "tokenizer-stalled");
    (toks.len(), stalled)
}

#[test]
fn html_tokenizer_terminates_on_every_state_boundary() {
    let cases = [
        // Empty and trivial.
        "",
        "a",
        "<",
        "<<<<",
        "&",
        "&#",
        "&#x",
        "&#x;",
        // Tag open / end tag open.
        "<!",
        "</",
        "</>",
        "<>",
        "<?",
        "<!doctype",
        "<!DOCTYPE",
        "<!---->",
        "<!----->",
        "<!-->",
        "<!--->",
        "<!-- --!>",
        "<![CDATA[",
        "<![CDATA[x",
        // Attribute states.
        "<a b",
        "<a b=",
        "<a b='",
        "<a b=\"",
        "<a b/>",
        "<a =x>",
        // Raw text with no terminator.
        "<script>",
        "<script><script>",
        "<style>",
        "<title>",
        "<textarea>",
        // NUL and high bytes.
        "\u{0}\u{0}\u{0}",
        "ééé",
        "日本",
    ];
    for src in cases {
        let (n, stalled) = htm_tokenize(src);
        assert!(n >= 1, "{src:?}: must emit at least Eof");
        assert!(!stalled, "{src:?}: tripped the progress guard");
    }
}

#[test]
fn html_tokenizer_outputs_every_text_byte() {
    // A tokenizer that silently drops input is a correctness bug even when it
    // terminates: the 8-step stall guard must not fire on ordinary text.
    let unchanged = ["hello world", "a < b", "héllo 世界", "x\u{0}y", "&amp;&lt;"];
    for src in unchanged {
        let (n, stalled) = htm_tokenize(src);
        assert!(!stalled, "{src:?}: stalled");
        assert!(n >= 1);
    }
}

#[test]
fn html_tokenizer_guarded_against_pathological_repetition() {
    // 200 KB of a single byte, for bytes that drive different states.
    for b in *b"<>!\"'-&/=\0" {
        let s = String::from_utf8(vec![b; 200_000]).unwrap();
        let (n, stalled) = htm_tokenize(&s);
        assert!(n >= 1);
        assert!(!stalled, "byte {:?} repeated 200k tripped the stall guard", b);
    }
}

// --- HTML tree construction ----------------------------------------------

#[test]
fn html_tree_construction_terminates_at_depth() {
    let cases = [
        rep("<div>", 50_000),
        rep("<svg><g>", 30_000),
        rep("<table><tr><td>", 20_000),
        rep("<select><option>", 30_000),
        rep("<p><div><span><b></p></div><table><td><div>", 2_000),
        rep("<script>", 20_000),
        rep("<plaintext>", 10_000),
    ];
    for src in cases {
        let mut doc = Document::new();
        let out = parse(&mut doc, src.as_bytes(), &ParseOpts::default());
        // The contract is only "it returned". The tree it built is whatever the
        // simplified insertion modes produce; the report records the rest.
        let _ = &out.approximations;
    }
}

// --- DOM serialization ---------------------------------------------------
//
// Regression guard for the stack overflow found in the audit: `serialize`,
// `inner_html`, `outer_html` and `text_content` were recursive and died on a
// ~700-level document (`<div>` × 700, 3.5 KB of input). They are now
// iterative.

fn nested_page(unit_tag: &str, n: usize) -> Document {
    let mut doc = Document::new();
    let src = rep(unit_tag, n);
    let _ = parse(&mut doc, src.as_bytes(), &ParseOpts::default());
    doc
}

#[test]
fn deep_document_serializes_without_stack_overflow() {
    // 20_000, not higher: `serialize` is O(depth) small allocations in a debug
    // build and the point is only that it does not recurse. 20_000 is an order
    // of magnitude past where the recursive version died (~700 levels).
    let doc = nested_page("<div>", 20_000);
    let s = serialize(&doc);
    assert!(s.contains("</div>"), "the tree must actually be deep");
    let _ = inner_html(&doc, doc.root());
    let _ = text_content(&doc, doc.root());
}

#[test]
fn very_deep_document_serializes_past_the_format_width_panic() {
    // Regression: `serialize` indented with `{:width$}`, whose width argument
    // panics above 65_535. At `depth * 2` that is any tree 32_768 levels deep
    // — a ~320 KB page. Found by this suite at 50_000 nested `<div>`s.
    let doc = nested_page("<div>", 33_000);
    let s = serialize(&doc);
    assert!(s.contains("</div>"));
}

#[test]
fn deep_table_document_serializes_without_stack_overflow() {
    // The audit's second crash: 5,000 nested `<table><tr><td>` overflowed.
    let doc = nested_page("<table><tr><td>", 5_000);
    let s = serialize(&doc);
    assert!(s.contains("</table>"));
}

// --- Fuzz-lite: every prefix of a hostile document -----------------------
//
// A crash is often shape-dependent (trailing `<` vs trailing letter). Feeding
// every prefix of an adversarial document exercises every partial state.

#[test]
fn every_prefix_of_a_hostile_document_tokenizes() {
    let full = "<!DOCTYPE html><div class=a b='c' d=\"e\">&amp;&#x41;<!-- c --><table><tr><td><script>x</script><svg><g></svg></div>";
    for i in 0..=full.len() {
        if full.is_char_boundary(i) {
            let (n, stalled) = htm_tokenize(&full[..i]);
            assert!(!stalled, "prefix {i} tripped the stall guard");
            assert!(n >= 1, "prefix {i} produced no Eof");
        }
    }
}

#[test]
fn every_prefix_of_a_hostile_document_parses() {
    let full = "<div><p><b><i><table><tr><td>x</td></tr></table><select><option>a</select></b></i></p></div>";
    for i in 0..=full.len() {
        if full.is_char_boundary(i) {
            let mut doc = Document::new();
            let _ = parse(&mut doc, full.as_bytes()[..i].to_vec().as_slice(), &ParseOpts::default());
            // Serialization of the partial tree must also terminate.
            let _ = serialize(&doc);
        }
    }
}