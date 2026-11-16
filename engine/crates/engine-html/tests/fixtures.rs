//! Fixture-driven tree-construction tests.
//!
//! Each fixture is `testdata/fixtures/html/<name>.html` plus
//! `<name>.tree.txt`, the canonical serialization from
//! `engine_dom::serialize`. The `.tree.txt` files are hand-verified against the
//! HTML spec's rules for each case; they are the oracle.
//!
//! Regenerate with `GEN=1 cargo test -p engine-html --test fixtures`. That mode
//! prints the tree it produced so the diff can be *reviewed* before pinning —
//! it never silently accepts what the parser emitted.

use std::fs;
use std::path::{Path, PathBuf};

use engine_dom::document::Document;
use engine_dom::serialize::serialize;
use engine_html::{parse, ParseOpts};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/fixtures/html")
}

fn cases() -> Vec<(String, PathBuf, PathBuf)> {
    let dir = fixture_dir();
    let mut out = Vec::new();
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for e in entries {
        let p = e.expect("dir entry").path();
        if p.extension().and_then(|s| s.to_str()) == Some("html") {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let tree = p.with_extension("tree.txt");
            out.push((stem, p, tree));
        }
    }
    out.sort();
    out
}

fn render(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut doc = Document::new();
    let _ = parse(&mut doc, &bytes, &ParseOpts::default());
    serialize(&doc)
}

#[test]
fn html_fixtures_match_expected_trees() {
    let cases = cases();
    assert!(!cases.is_empty(), "no fixtures found in {}", fixture_dir().display());

    let regenerate = std::env::var("GEN").is_ok();
    let mut failures = Vec::new();

    for (name, html, tree) in &cases {
        let got = render(html);
        if regenerate {
            fs::write(tree, &got).unwrap_or_else(|e| panic!("write {}: {e}", tree.display()));
            eprintln!("--- regenerated {name} ---\n{got}");
            continue;
        }
        let want = match fs::read_to_string(tree) {
            Ok(s) => s.replace("\r\n", "\n"),
            Err(_) => {
                failures.push(format!("{name}: missing {}", tree.display()));
                continue;
            }
        };
        if got != want {
            failures.push(format!(
                "{name}: tree mismatch\n--- got ---\n{got}\n--- want ---\n{want}"
            ));
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Pin the gaps we know about, so a future change cannot quietly "fix" them
/// without a human noticing, and so the golden fixtures are never mistaken for
/// a claim of full spec conformance.
///
/// Each of these is a documented approximation in `ARCHITECTURE.md` §15
/// (`html5 tree construction full insertion-mode fidelity`).
#[test]
fn known_tree_construction_gaps_are_pinned() {
    let render_fragment = |src: &str| -> String {
        let mut doc = Document::new();
        let _ = parse(&mut doc, src.as_bytes(), &ParseOpts::default());
        serialize(&doc)
    };

    // Gap 1: a block inside an inline is not pulled out. Browsers close the
    // `<span>` before the `<div>`; we nest it. Visually the difference is the
    // span's background/border not being split, which is a minor deviation.
    let s = render_fragment("<p><span>x<div>y</div></span></p>");
    assert!(
        s.contains("<div>") && s.contains("    #text \"y\""),
        "expected the div nested inside the span (documented gap); got:\n{s}"
    );

    // Gap 2: the adoption agency is simplified to a generic scope unwind, so a
    // nested `<a>` stays nested instead of closing the outer anchor.
    let s = render_fragment("<a href=1>one<a href=2>two</a>");
    assert!(
        s.matches("<a ").count() == 2 && s.contains("        <a href=\"2\">"),
        "expected nested anchors (documented gap); got:\n{s}"
    );

    // `<template>` contents are inserted but never rendered.
    let mut doc = Document::new();
    let out = parse(&mut doc, b"<template><p>x</p></template>", &ParseOpts::default());
    assert!(
        out.approximations
            .iter()
            .any(|a| matches!(a, engine_html::HtmlApproximation::Recovery { detail } if *detail == "template-contents")),
        "template handling must be reported"
    );
}