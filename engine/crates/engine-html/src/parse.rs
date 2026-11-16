//! The HTML entry point: bytes in, a populated `Document` plus a report out.

use engine_dom::document::Document;
use engine_dom::node::{NodeKind, QuirksMode};

use crate::charset::{decode, DecodeReport};
use crate::tokenizer::state::Tokenizer;
use crate::tokenizer::TokenizerError;
use crate::tree::{self, HtmlApproximation};

/// Options for a parse.
#[derive(Clone, Debug, Default)]
pub struct ParseOpts {
    /// Force a character encoding instead of sniffing.
    pub force_utf8: bool,
}

/// An HTML parse error, positioned by tokenizer byte offset where known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlError {
    pub pos: usize,
    pub kind: String,
}

/// The result of parsing.
#[derive(Clone, Debug)]
pub struct ParseOutcome {
    pub quirks: QuirksMode,
    pub errors: Vec<HtmlError>,
    pub approximations: Vec<HtmlApproximation>,
    pub decode: DecodeReport,
    /// The `<body>` element, if one was created.
    pub body: Option<engine_core::NodeId>,
}

/// Parse `bytes` into `doc`, which must be freshly created.
///
/// This mutates `doc` in place and returns the parse outcome. See the deviation
/// note in `engine-dom::document` for why the arena is `&mut` rather than
/// interior-mutable.
pub fn parse(doc: &mut Document, bytes: &[u8], opts: &ParseOpts) -> ParseOutcome {
    let (text, decode_report) = if opts.force_utf8 {
        (String::from_utf8_lossy(bytes).into_owned(), DecodeReport::default())
    } else {
        decode(bytes)
    };

    let mut tz = Tokenizer::new(&text);
    let tokens = tz.tokenize();
    let tok_errors: Vec<TokenizerError> = tz.errors().to_vec();

    let outcome = tree::build(doc, tokens);

    let mut errors: Vec<HtmlError> = tok_errors
        .into_iter()
        .map(|e| HtmlError { pos: e.pos, kind: e.kind.to_string() })
        .collect();
    errors.extend(outcome.errors.into_iter().map(|k| HtmlError { pos: 0, kind: k }));

    // Find the body element for later stages.
    let body = doc
        .ids()
        .find(|id| matches!(doc.kind(*id), NodeKind::Element(e) if e.tag.is(engine_dom::KnownTag::Body)));

    ParseOutcome {
        quirks: outcome.quirks,
        errors,
        approximations: outcome.approximations,
        decode: decode_report,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_dom::serialize::serialize;

    #[test]
    fn parses_and_populates_document() {
        let mut doc = Document::new();
        let out = parse(&mut doc, b"<!DOCTYPE html><p>hello", &ParseOpts::default());
        assert_eq!(out.quirks, QuirksMode::NoQuirks);
        assert!(out.body.is_some(), "a body element must exist");
        let s = serialize(&doc);
        assert!(s.contains("#text \"hello\""));
    }

    #[test]
    fn decodes_windows1252_via_meta() {
        let html = b"<meta charset=windows-1252><p>caf\xE9";
        let mut doc = Document::new();
        let out = parse(&mut doc, html, &ParseOpts::default());
        assert_eq!(out.decode.encoding, Some(crate::charset::Encoding::Windows1252));
        let s = serialize(&doc);
        assert!(s.contains("café"), "got: {s}");
    }

    #[test]
    fn reports_quirks_without_doctype() {
        let mut doc = Document::new();
        let out = parse(&mut doc, b"<p>x", &ParseOpts::default());
        assert_eq!(out.quirks, QuirksMode::Quirks);
        assert!(out.approximations.iter().any(|a| matches!(a, HtmlApproximation::Recovery { detail } if *detail == "missing-doctype")));
    }

    #[test]
    fn tokenizer_errors_surface() {
        let mut doc = Document::new();
        let out = parse(&mut doc, b"<div", &ParseOpts::default());
        assert!(!out.errors.is_empty(), "unclosed tag should be reported");
    }
}