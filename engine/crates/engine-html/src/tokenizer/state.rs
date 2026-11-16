//! The HTML5 tokenizer state machine.
//!
//! Implements the WHATWG "Tokenization" states that matter for tree
//! construction. States that only affect `<script>` (script data escape) are
//! simplified: script content is emitted as raw text, which is all the tree
//! constructor needs.
//!
//! Design notes:
//! - Input is `&[u8]`, decoded as UTF-8 up front by `charset.rs`. The tokenizer
//!   therefore never has to deal with re-decoding mid-stream.
//! - `Chars` runs are buffered in a `String` and flushed before any tag,
//!   comment, or EOF token, so the tree constructor sees contiguous text.
//! - The parser never panics: malformed input in every state has a defined
//!   recovery (emit what we have, switch to the data state).

use crate::tokenizer::char_ref::decode_after_amp;
use crate::tokenizer::token::{Attribute, Token};

/// Raw-text element kinds. Stored as an enum rather than a `String` so `State`
/// stays `Copy` and the per-byte dispatch does not allocate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawKind {
    Script,
    Style,
    Title,
    Textarea,
    Xmp,
    Iframe,
    Noembed,
    Noframes,
}

impl RawKind {
    fn from_tag(tag: &str) -> Option<RawKind> {
        Some(match tag {
            "script" => RawKind::Script,
            "style" => RawKind::Style,
            "title" => RawKind::Title,
            "textarea" => RawKind::Textarea,
            "xmp" => RawKind::Xmp,
            "iframe" => RawKind::Iframe,
            "noembed" => RawKind::Noembed,
            "noframes" => RawKind::Noframes,
            _ => return None,
        })
    }

    fn tag(self) -> &'static str {
        match self {
            RawKind::Script => "script",
            RawKind::Style => "style",
            RawKind::Title => "title",
            RawKind::Textarea => "textarea",
            RawKind::Xmp => "xmp",
            RawKind::Iframe => "iframe",
            RawKind::Noembed => "noembed",
            RawKind::Noframes => "noframes",
        }
    }

    /// RCDATA additionally decodes character references.
    fn is_rcdata(self) -> bool {
        matches!(self, RawKind::Title | RawKind::Textarea)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Data,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    BogusDoctype,
    /// Raw-text element content (`<script>`, `<style>`, `<title>`, `<textarea>`).
    RawText {
        kind: RawKind,
    },
}

/// An incremental HTML tokenizer.
pub struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    state: State,
    /// Buffered character data, flushed before non-Chars tokens.
    text: String,
    /// Current tag being built.
    tag_name: String,
    attrs: Vec<Attribute>,
    attr_name: String,
    attr_value: String,
    self_closing: bool,
    /// True while building an end tag (`</name>`), which shares the tag-name and
    /// attribute states with start tags in the spec.
    is_end_tag: bool,
    /// Current comment being built.
    comment: String,
    /// Current doctype fields.
    doctype_name: String,
    doctype_public: Option<String>,
    doctype_system: Option<String>,
    force_quirks: bool,
    finished: bool,
    /// Errors collected for the render report.
    errors: Vec<TokenizerError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenizerError {
    pub pos: usize,
    pub kind: &'static str,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Tokenizer {
            input,
            pos: 0,
            state: State::Data,
            text: String::new(),
            tag_name: String::new(),
            attrs: Vec::new(),
            attr_name: String::new(),
            attr_value: String::new(),
            self_closing: false,
            is_end_tag: false,
            comment: String::new(),
            doctype_name: String::new(),
            doctype_public: None,
            doctype_system: None,
            force_quirks: false,
            finished: false,
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[TokenizerError] {
        &self.errors
    }

    fn error(&mut self, kind: &'static str) {
        // Cap the error list: a pathological document should not allocate
        // without bound, and the report only shows the first few anyway.
        if self.errors.len() < 256 {
            self.errors.push(TokenizerError { pos: self.pos, kind });
        }
    }

    /// Advance one byte. Returns `None` at EOF. ASCII-only: multi-byte UTF-8
    /// sequences are consumed as opaque bytes and copied through, which is
    /// correct because every state boundary is an ASCII byte.
    fn next_byte(&mut self) -> Option<u8> {
        let b = self.input.as_bytes().get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    /// Copy one UTF-8 scalar starting at the current position into `out`.
    /// Used when a non-ASCII byte appears where text is being accumulated.
    fn take_char_into(&mut self, out: &mut String) {
        let rest = &self.input[self.pos..];
        if let Some(c) = rest.chars().next() {
            out.push(c);
            self.pos += c.len_utf8();
        } else {
            self.pos += 1;
        }
    }

    fn starts_with_ignore_case(&self, s: &str) -> bool {
        let rest = &self.input[self.pos..];
        rest.len() >= s.len() && rest[..s.len()].eq_ignore_ascii_case(s)
    }

    fn emit_text(&mut self, out: &mut Vec<Token>) {
        if !self.text.is_empty() {
            out.push(Token::Chars(std::mem::take(&mut self.text)));
        }
    }

    fn start_attr(&mut self) {
        if !self.attr_name.is_empty() || !self.attr_value.is_empty() {
            self.attrs.push(Attribute {
                name: std::mem::take(&mut self.attr_name),
                value: std::mem::take(&mut self.attr_value),
            });
        }
    }

    /// Consume the whole input and return all tokens, ending with `Eof`.
    ///
    /// Guarded against a non-advancing state: a state that neither consumes a
    /// byte nor changes state would otherwise loop forever accumulating tokens
    /// (this actually happened, and the guard is what stops it from eating
    /// memory instead of failing loudly).
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut out = Vec::new();
        let mut last = (self.pos, self.state);
        let mut stalled = 0u32;
        while !self.finished {
            self.step(&mut out);
            let now = (self.pos, self.state);
            if now == last {
                stalled += 1;
                if stalled > 8 {
                    self.error("tokenizer-stalled");
                    self.emit_text(&mut out);
                    self.finish(&mut out);
                    break;
                }
            } else {
                stalled = 0;
                last = now;
            }
        }
        out
    }

    fn step(&mut self, out: &mut Vec<Token>) {
        // `State` is `Copy`, so dispatch by value costs nothing.
        match self.state {
            State::Data => self.data_state(out),
            State::TagOpen => self.tag_open_state(out),
            State::EndTagOpen => self.end_tag_open_state(out),
            State::TagName => self.tag_name_state(out),
            State::BeforeAttributeName => self.before_attr_name_state(out),
            State::AttributeName => self.attr_name_state(out),
            State::AfterAttributeName => self.after_attr_name_state(out),
            State::BeforeAttributeValue => self.before_attr_value_state(out),
            State::AttributeValueDoubleQuoted => self.attr_value_state(out, b'"'),
            State::AttributeValueSingleQuoted => self.attr_value_state(out, b'\''),
            State::AttributeValueUnquoted => self.attr_value_unquoted_state(out),
            State::AfterAttributeValueQuoted => self.after_attr_value_state(out),
            State::SelfClosingStartTag => self.self_closing_state(out),
            State::BogusComment => self.bogus_comment_state(out),
            State::MarkupDeclarationOpen => self.markup_declaration_open_state(out),
            State::CommentStart => self.comment_start_state(out),
            State::CommentStartDash => {
                match self.next_byte() {
                    Some(b'-') => self.state = State::CommentEnd,
                    Some(b'>') => {
                        self.error("abrupt-closing-of-empty-comment");
                        self.end_comment(out);
                    }
                    Some(_) => self.comment.push('-'),
                    None => {
                        self.error("eof-in-comment");
                        self.end_comment(out);
                    }
                }
            }
            State::Comment => self.comment_state(out),
            State::CommentLessThanSign => self.comment_lt_state(out),
            State::CommentLessThanSignBang => {
                self.state = if self.peek() == Some(b'-') { State::CommentLessThanSignBangDash } else { State::Comment };
            }
            State::CommentLessThanSignBangDash => {
                self.state = if self.peek() == Some(b'-') { State::CommentLessThanSignBangDashDash } else { State::CommentEndDash };
            }
            State::CommentLessThanSignBangDashDash => {
                if self.peek() == Some(b'>') {
                    self.state = State::CommentEnd;
                } else {
                    self.error("nested-comment");
                    self.state = State::CommentEnd;
                }
            }
            State::CommentEndDash => {
                if self.peek() == Some(b'-') {
                    self.pos += 1;
                    self.state = State::CommentEnd;
                } else {
                    self.comment.push('-');
                    self.state = State::Comment;
                }
            }
            State::CommentEnd => self.comment_end_state(out),
            State::CommentEndBang => {
                match self.next_byte() {
                    Some(b'-') => {
                        self.comment.push_str("--");
                        self.state = State::CommentEndDash;
                    }
                    Some(b'>') => self.end_comment(out),
                    Some(b) => {
                        self.comment.push_str("--!");
                        self.text_byte(b);
                        self.state = State::Comment;
                    }
                    None => {
                        self.error("eof-in-comment");
                        self.end_comment(out);
                    }
                }
            }
            State::Doctype => self.doctype_state(out),
            State::BeforeDoctypeName => self.before_doctype_name_state(out),
            State::DoctypeName => self.doctype_name_state(out),
            State::AfterDoctypeName => self.after_doctype_name_state(out),
            State::BogusDoctype => {
                match self.next_byte() {
                    Some(b'>') => {
                        self.emit_doctype(out);
                        self.state = State::Data;
                    }
                    Some(_) => {}
                    None => {
                        self.error("eof-in-doctype");
                        self.emit_doctype(out);
                        self.finish(out);
                    }
                }
            }
            State::RawText { .. } => self.raw_text_state(out),
        }
    }

    fn text_byte(&mut self, b: u8) {
        if b < 0x80 {
            self.text.push(b as char);
        } else {
            // Non-ASCII: back up and take the full scalar.
            self.pos -= 1;
            let mut s = String::new();
            self.take_char_into(&mut s);
            self.text.push_str(&s);
        }
    }

    // --- Data -------------------------------------------------------------

    fn data_state(&mut self, out: &mut Vec<Token>) {
        match self.next_byte() {
            None => {
                self.emit_text(out);
                self.finish(out);
            }
            Some(b'&') => {
                if let Some((s, consumed)) = decode_after_amp(&self.input[self.pos..]) {
                    self.pos += consumed;
                    self.text.push_str(&s);
                } else {
                    self.text.push('&');
                }
            }
            Some(b'<') => self.state = State::TagOpen,
            Some(b) => self.text_byte(b),
        }
    }

    fn finish(&mut self, out: &mut Vec<Token>) {
        if !self.finished {
            self.finished = true;
            self.emit_text(out);
            out.push(Token::Eof);
        }
    }

    // --- Tags -------------------------------------------------------------

    fn tag_open_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            Some(b'!') => {
                self.pos += 1;
                self.state = State::MarkupDeclarationOpen;
            }
            Some(b'/') => {
                self.pos += 1;
                self.state = State::EndTagOpen;
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.tag_name.clear();
                self.attrs.clear();
                self.attr_name.clear();
                self.attr_value.clear();
                self.self_closing = false;
                self.state = State::TagName;
            }
            Some(b'?') => {
                self.pos += 1;
                self.error("unexpected-question-mark-instead-of-tag-name");
                // The `?` is part of the comment text (the spec reconsumes it).
                self.comment.clear();
                self.comment.push('?');
                self.state = State::BogusComment;
            }
            None => {
                self.error("eof-before-tag-name");
                self.text.push('<');
                self.emit_text(out);
                self.finish(out);
            }
            Some(_) => {
                self.error("invalid-first-character-of-tag-name");
                self.text.push('<');
                self.state = State::Data;
            }
        }
    }

    fn end_tag_open_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.tag_name.clear();
                self.attrs.clear();
                self.attr_name.clear();
                self.attr_value.clear();
                self.is_end_tag = true;
                self.state = State::TagName;
            }
            Some(b'>') => {
                self.pos += 1;
                self.error("missing-end-tag-name");
                self.state = State::Data;
            }
            None => {
                self.error("eof-before-tag-name");
                self.text.push_str("</");
                self.emit_text(out);
                self.finish(out);
            }
            Some(_) => {
                self.error("invalid-first-character-of-tag-name");
                self.comment.clear();
                self.state = State::BogusComment;
            }
        }
    }

    fn tag_name_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => {
                self.pos += 1;
                self.state = State::BeforeAttributeName;
            }
            Some(b'/') => {
                self.pos += 1;
                self.state = State::SelfClosingStartTag;
            }
            Some(b'>') => {
                self.pos += 1;
                self.finish_tag(out);
            }
            Some(b) => {
                self.pos += 1;
                self.tag_name.push((b as char).to_ascii_lowercase());
            }
        }
    }

    /// Emit the tag just parsed as start or end (decided by whether we entered
    /// via `EndTagOpen`).
    fn finish_tag(&mut self, out: &mut Vec<Token>) {
        self.emit_text(out);
        if !self.attr_name.is_empty() || !self.attr_value.is_empty() {
            self.attrs.push(Attribute {
                name: std::mem::take(&mut self.attr_name),
                value: std::mem::take(&mut self.attr_value),
            });
        }
        let name = std::mem::take(&mut self.tag_name);
        let attrs = std::mem::take(&mut self.attrs);
        if self.is_end_tag {
            out.push(Token::EndTag { name });
        } else {
            // Raw-text elements have their content consumed verbatim. The tree
            // constructor only needs the text and the terminating end tag.
            let raw = RawKind::from_tag(&name);
            out.push(Token::StartTag { name: name.clone(), attrs, self_closing: self.self_closing });
            if let Some(kind) = raw {
                if !self.self_closing {
                    self.state = State::RawText { kind };
                    self.is_end_tag = false;
                    self.self_closing = false;
                    return;
                }
            }
        }
        self.is_end_tag = false;
        self.self_closing = false;
        self.state = State::Data;
    }

    fn before_attr_name_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => self.pos += 1,
            Some(b'/') | Some(b'>') => self.state = State::AfterAttributeName,
            Some(b'=') => {
                self.pos += 1;
                self.error("unexpected-equals-sign-before-attribute-name");
                self.attr_name.clear();
                self.attr_name.push('=');
                self.state = State::AttributeName;
            }
            Some(_) => {
                self.attr_name.clear();
                self.attr_value.clear();
                self.state = State::AttributeName;
            }
        }
    }

    fn attr_name_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) || b == b'/' || b == b'>' => self.state = State::AfterAttributeName,
            Some(b'=') => {
                self.pos += 1;
                self.state = State::BeforeAttributeValue;
            }
            Some(0) => {
                self.pos += 1;
                self.error("unexpected-null-character");
                self.attr_name.push('\u{FFFD}');
            }
            Some(c @ (b'"' | b'\'' | b'<')) => {
                self.pos += 1;
                self.error("unexpected-character-in-attribute-name");
                self.attr_name.push((c as char).to_ascii_lowercase());
            }
            Some(b) => {
                self.pos += 1;
                self.attr_name.push((b as char).to_ascii_lowercase());
            }
        }
    }

    fn after_attr_name_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => self.pos += 1,
            Some(b'/') => {
                self.pos += 1;
                self.state = State::SelfClosingStartTag;
            }
            Some(b'=') => {
                self.pos += 1;
                self.state = State::BeforeAttributeValue;
            }
            Some(b'>') => {
                self.pos += 1;
                self.finish_tag(out);
            }
            Some(_) => {
                self.start_attr();
                self.state = State::AttributeName;
            }
        }
    }

    fn before_attr_value_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => self.pos += 1,
            Some(b'"') => {
                self.pos += 1;
                self.state = State::AttributeValueDoubleQuoted;
            }
            Some(b'\'') => {
                self.pos += 1;
                self.state = State::AttributeValueSingleQuoted;
            }
            Some(b'>') => {
                self.pos += 1;
                self.error("missing-attribute-value");
                self.finish_tag(out);
            }
            Some(_) => self.state = State::AttributeValueUnquoted,
        }
    }

    fn attr_value_state(&mut self, out: &mut Vec<Token>, quote: u8) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if b == quote => {
                self.pos += 1;
                self.state = State::AfterAttributeValueQuoted;
            }
            Some(b'&') => {
                self.pos += 1;
                if let Some((s, consumed)) = decode_after_amp(&self.input[self.pos..]) {
                    self.pos += consumed;
                    self.attr_value.push_str(&s);
                } else {
                    self.attr_value.push('&');
                }
            }
            Some(0) => {
                self.pos += 1;
                self.error("unexpected-null-character");
                self.attr_value.push('\u{FFFD}');
            }
            Some(_) => {
                let mut s = String::new();
                self.take_char_into(&mut s);
                self.attr_value.push_str(&s);
            }
        }
    }

    fn attr_value_unquoted_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => {
                self.pos += 1;
                self.start_attr();
                self.state = State::BeforeAttributeName;
            }
            Some(b'>') => {
                self.pos += 1;
                self.finish_tag(out);
            }
            Some(b'&') => {
                self.pos += 1;
                if let Some((s, consumed)) = decode_after_amp(&self.input[self.pos..]) {
                    self.pos += consumed;
                    self.attr_value.push_str(&s);
                } else {
                    self.attr_value.push('&');
                }
            }
            Some(0) => {
                self.pos += 1;
                self.error("unexpected-null-character");
                self.attr_value.push('\u{FFFD}');
            }
            Some(b'"') | Some(b'\'') | Some(b'<') | Some(b'=') | Some(b'`') => {
                self.pos += 1;
                self.error("unexpected-character-in-unquoted-attribute-value");
                self.attr_value.push(self.input.as_bytes()[self.pos - 1] as char);
            }
            Some(_) => {
                let mut s = String::new();
                self.take_char_into(&mut s);
                self.attr_value.push_str(&s);
            }
        }
    }

    fn after_attr_value_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => {
                self.pos += 1;
                self.start_attr();
                self.state = State::BeforeAttributeName;
            }
            Some(b'/') => {
                self.pos += 1;
                self.start_attr();
                self.state = State::SelfClosingStartTag;
            }
            Some(b'>') => {
                self.pos += 1;
                self.start_attr();
                self.finish_tag(out);
            }
            Some(_) => {
                self.error("missing-whitespace-between-attributes");
                self.start_attr();
                self.state = State::BeforeAttributeName;
            }
        }
    }

    fn self_closing_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-tag");
                self.emit_text(out);
                self.finish(out);
            }
            Some(b'>') => {
                self.pos += 1;
                self.self_closing = true;
                self.finish_tag(out);
            }
            Some(_) => {
                self.error("unexpected-solidus-in-tag");
                self.state = State::BeforeAttributeName;
            }
        }
    }

    // --- Comments ---------------------------------------------------------

    fn bogus_comment_state(&mut self, out: &mut Vec<Token>) {
        // `self.comment` may already hold a seeding character (`?` from tag-open,
        // `[CDATA[` from markup-declaration-open), so it is not cleared here.
        loop {
            match self.next_byte() {
                None => {
                    self.emit_text(out);
                    out.push(Token::Comment(std::mem::take(&mut self.comment)));
                    self.finish(out);
                    return;
                }
                Some(b'>') => {
                    self.end_comment(out);
                    return;
                }
                Some(0) => self.comment.push('\u{FFFD}'),
                Some(b) => self.comment.push(b as char),
            }
        }
    }

    fn end_comment(&mut self, out: &mut Vec<Token>) {
        self.emit_text(out);
        out.push(Token::Comment(std::mem::take(&mut self.comment)));
        self.state = State::Data;
    }

    fn markup_declaration_open_state(&mut self, _out: &mut Vec<Token>) {
        if self.starts_with_ignore_case("--") {
            self.pos += 2;
            self.comment.clear();
            self.state = State::CommentStart;
        } else if self.starts_with_ignore_case("doctype") {
            self.pos += 7;
            self.state = State::Doctype;
        } else if self.starts_with_ignore_case("[CDATA[") {
            // CDATA is only meaningful in foreign content; in HTML it becomes a
            // bogus comment, which matches browser behaviour.
            self.pos += 7;
            self.error("cdata-in-html-content");
            self.comment.clear();
            self.comment.push_str("[CDATA[");
            self.state = State::BogusComment;
        } else {
            self.error("incorrectly-opened-comment");
            self.comment.clear();
            self.state = State::BogusComment;
        }
    }

    fn comment_start_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                self.state = State::CommentStartDash;
            }
            Some(b'>') => {
                self.pos += 1;
                self.error("abrupt-closing-of-empty-comment");
                self.end_comment(out);
            }
            _ => self.state = State::Comment,
        }
    }

    fn comment_state(&mut self, out: &mut Vec<Token>) {
        match self.next_byte() {
            None => {
                self.error("eof-in-comment");
                self.end_comment(out);
            }
            Some(b'<') => {
                self.comment.push('<');
                self.state = State::CommentLessThanSign;
            }
            Some(b'-') => self.state = State::CommentEndDash,
            Some(0) => {
                self.error("unexpected-null-character");
                self.comment.push('\u{FFFD}');
            }
            Some(b) => self.comment.push(b as char),
        }
    }

    fn comment_lt_state(&mut self, _out: &mut Vec<Token>) {
        match self.peek() {
            Some(b'!') => {
                self.pos += 1;
                self.comment.push('!');
                self.state = State::CommentLessThanSignBang;
            }
            Some(b'<') => {
                self.pos += 1;
                self.comment.push('<');
            }
            _ => self.state = State::Comment,
        }
    }

    fn comment_end_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            Some(b'>') => {
                self.pos += 1;
                self.end_comment(out);
            }
            Some(b'!') => {
                self.pos += 1;
                self.state = State::CommentEndBang;
            }
            Some(b'-') => {
                self.pos += 1;
                self.comment.push('-');
            }
            _ => self.state = State::Comment,
        }
    }

    // --- Doctype ----------------------------------------------------------

    fn doctype_state(&mut self, out: &mut Vec<Token>) {
        self.doctype_name.clear();
        self.doctype_public = None;
        self.doctype_system = None;
        self.force_quirks = false;
        // Spec: skip whitespace; anything other than whitespace is a parse
        // error and goes to before-doctype-name anyway.
        match self.next_byte() {
            None => {
                // EOF here must finish the stream. `emit_doctype` alone leaves
                // `finished` false, so the driver loop used to re-enter this
                // state with `pos` at EOF over and over, emitting one bogus
                // doctype per step until the stall guard tripped — nine
                // spurious doctype tokens from the 9-byte input `<!doctype`.
                self.error("eof-in-doctype");
                self.force_quirks = true;
                self.emit_doctype(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => self.state = State::BeforeDoctypeName,
            Some(_) => {
                self.pos -= 1;
                self.error("missing-whitespace-before-doctype-name");
                self.state = State::BeforeDoctypeName;
            }
        }
    }

    fn before_doctype_name_state(&mut self, out: &mut Vec<Token>) {
        match self.next_byte() {
            None => {
                self.error("eof-in-doctype");
                self.force_quirks = true;
                self.emit_doctype(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => {}
            Some(b'>') => {
                self.error("missing-doctype-name");
                self.force_quirks = true;
                self.state = State::Data;
                self.emit_doctype(out);
            }
            Some(0) => {
                self.error("unexpected-null-character");
                self.doctype_name.push('\u{FFFD}');
                self.state = State::DoctypeName;
            }
            Some(b) => {
                self.doctype_name.push((b as char).to_ascii_lowercase());
                self.state = State::DoctypeName;
            }
        }
    }

    fn doctype_name_state(&mut self, out: &mut Vec<Token>) {
        match self.next_byte() {
            None => {
                self.error("eof-in-doctype");
                self.force_quirks = true;
                self.emit_doctype(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => self.state = State::AfterDoctypeName,
            Some(b'>') => {
                self.state = State::Data;
                self.emit_doctype(out);
            }
            Some(0) => {
                self.error("unexpected-null-character");
                self.doctype_name.push('\u{FFFD}');
            }
            Some(b) => self.doctype_name.push((b as char).to_ascii_lowercase()),
        }
    }

    fn after_doctype_name_state(&mut self, out: &mut Vec<Token>) {
        match self.peek() {
            None => {
                self.error("eof-in-doctype");
                self.force_quirks = true;
                self.emit_doctype(out);
                self.finish(out);
            }
            Some(b) if is_ws(b) => self.pos += 1,
            Some(b'>') => {
                self.pos += 1;
                self.state = State::Data;
                self.emit_doctype(out);
            }
            Some(_) => {
                // PUBLIC/SYSTEM keywords: we record the ids but do not act on
                // them (quirks determination only needs "is this the html5
                // doctype"). Bogus mode consumes the rest up to '>'.
                if self.starts_with_ignore_case("public") {
                    self.pos += 6;
                    self.doctype_public = Some(String::new());
                    self.state = State::BogusDoctype;
                } else if self.starts_with_ignore_case("system") {
                    self.pos += 6;
                    self.doctype_system = Some(String::new());
                    self.state = State::BogusDoctype;
                } else {
                    self.error("invalid-character-sequence-after-doctype-name");
                    self.force_quirks = true;
                    self.state = State::BogusDoctype;
                }
            }
        }
    }

    fn emit_doctype(&mut self, out: &mut Vec<Token>) {
        self.emit_text(out);
        out.push(Token::Doctype {
            name: std::mem::take(&mut self.doctype_name),
            public_id: self.doctype_public.take(),
            system_id: self.doctype_system.take(),
            force_quirks: self.force_quirks,
        });
    }

    // --- Raw text ---------------------------------------------------------

    fn raw_text_state(&mut self, out: &mut Vec<Token>) {
        let kind = match self.state {
            State::RawText { kind } => kind,
            _ => unreachable!("raw_text_state called outside RawText"),
        };
        let end_tag = kind.tag();
        // Find `</name` followed by whitespace, '/', or '>'. Borrowed, not
        // copied: this runs once per complete raw-text element, and an O(n)
        // copy here would make it O(n^2) over a document.
        let hay = &self.input[self.pos..];
        let bytes = hay.as_bytes();
        let name_len = end_tag.len();
        let mut found = None;
        let mut i = 0;
        while i + 2 + name_len <= bytes.len() {
            if bytes[i] == b'<' && bytes[i + 1] == b'/' {
                let candidate = &hay[i + 2..];
                if candidate.len() >= name_len
                    && candidate[..name_len].eq_ignore_ascii_case(end_tag)
                {
                    let after = candidate.as_bytes().get(name_len).copied();
                    if matches!(
                        after,
                        None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'\x0c')
                            | Some(b'/') | Some(b'>')
                    ) {
                        found = Some(i);
                        break;
                    }
                }
            }
            i += 1;
        }
        match found {
            None => {
                let rest = &self.input[self.pos..];
                self.text.push_str(rest);
                self.pos = self.input.len();
                self.emit_text(out);
                self.finish(out);
            }
            Some(off) => {
                // RAWTEXT (`script`, `style`, `xmp`, …) takes the body verbatim.
                // RCDATA (`title`, `textarea`) additionally decodes character
                // references. `&` outside a reference stays literal in both.
                let body = &self.input[self.pos..self.pos + off];
                if kind.is_rcdata() {
                    let mut i = 0;
                    let bytes = body.as_bytes();
                    while i < bytes.len() {
                        if bytes[i] == b'&' {
                            if let Some((s, consumed)) = decode_after_amp(&body[i + 1..]) {
                                self.text.push_str(&s);
                                i += 1 + consumed;
                                continue;
                            }
                        }
                        let ch = body[i..].chars().next().unwrap_or('\u{FFFD}');
                        self.text.push(ch);
                        i += ch.len_utf8();
                    }
                } else {
                    self.text.push_str(body);
                }
                self.pos += off + 2 + name_len;
                // Consume optional whitespace and '>' (simplified: a proper end
                // tag may carry attributes, which are ignored here).
                while let Some(b) = self.peek() {
                    if is_ws(b) {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if self.peek() == Some(b'>') {
                    self.pos += 1;
                }
                self.emit_text(out);
                out.push(Token::EndTag { name: end_tag.to_string() });
                self.state = State::Data;
            }
        }
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

/// Convenience: tokenize a string, returning tokens (ending in `Eof`) plus the
/// tokenizer's parse errors.
pub fn tokenize(input: &str) -> (Vec<Token>, Vec<TokenizerError>) {
    let mut t = Tokenizer::new(input);
    let tokens = t.tokenize();
    (tokens, std::mem::take(&mut t.errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Token> {
        Tokenizer::new(s).tokenize()
    }

    fn start(name: &str) -> Token {
        Token::StartTag { name: name.into(), attrs: vec![], self_closing: false }
    }

    fn end(name: &str) -> Token {
        Token::EndTag { name: name.into() }
    }

    fn chars(s: &str) -> Token {
        Token::Chars(s.into())
    }

    #[test]
    fn plain_text() {
        assert_eq!(toks("hello"), vec![chars("hello"), Token::Eof]);
    }

    #[test]
    fn simple_element() {
        assert_eq!(
            toks("<div>x</div>"),
            vec![start("div"), chars("x"), end("div"), Token::Eof]
        );
    }

    #[test]
    fn attributes() {
        assert_eq!(
            toks("<a href=\"x\" class='y' id=z>"),
            vec![
                Token::StartTag {
                    name: "a".into(),
                    attrs: vec![
                        Attribute { name: "href".into(), value: "x".into() },
                        Attribute { name: "class".into(), value: "y".into() },
                        Attribute { name: "id".into(), value: "z".into() },
                    ],
                    self_closing: false,
                },
                Token::Eof
            ]
        );
    }

    #[test]
    fn attribute_names_lowercased() {
        let t = toks("<DIV CLASS=\"A\">");
        match &t[0] {
            Token::StartTag { name, attrs, .. } => {
                assert_eq!(name, "div");
                assert_eq!(attrs[0].name, "class");
                assert_eq!(attrs[0].value, "A", "values keep their case");
            }
            _ => panic!("expected start tag"),
        }
    }

    #[test]
    fn self_closing() {
        assert_eq!(
            toks("<br/>"),
            vec![Token::StartTag { name: "br".into(), attrs: vec![], self_closing: true }, Token::Eof]
        );
    }

    #[test]
    fn boolean_attribute() {
        assert_eq!(
            toks("<input disabled>"),
            vec![
                Token::StartTag {
                    name: "input".into(),
                    attrs: vec![Attribute { name: "disabled".into(), value: String::new() }],
                    self_closing: false,
                },
                Token::Eof
            ]
        );
    }

    #[test]
    fn character_references_in_text() {
        assert_eq!(toks("a &amp; b"), vec![chars("a & b"), Token::Eof]);
        assert_eq!(toks("&#65;&#x42;"), vec![chars("AB"), Token::Eof]);
    }

    #[test]
    fn character_reference_in_attribute() {
        assert_eq!(
            toks("<a href=\"?a=1&amp;b=2\">"),
            vec![
                Token::StartTag {
                    name: "a".into(),
                    attrs: vec![Attribute { name: "href".into(), value: "?a=1&b=2".into() }],
                    self_closing: false,
                },
                Token::Eof
            ]
        );
    }

    #[test]
    fn unknown_reference_stays_literal() {
        assert_eq!(toks("&bogus;"), vec![chars("&bogus;"), Token::Eof]);
    }

    #[test]
    fn comment() {
        assert_eq!(toks("<!-- hi -->"), vec![Token::Comment(" hi ".into()), Token::Eof]);
    }

    #[test]
    fn eof_in_every_doctype_state_finishes_the_stream() {
        // Each of these ends the input while in a different doctype sub-state.
        // Before the fix, `emit_doctype` without `finish` left `finished` false
        // and the driver re-entered the same state at EOF, emitting one bogus
        // doctype per step until the 8-step stall guard tripped: `<!doctype`
        // (9 bytes) produced nine doctype tokens. The contract is exactly one
        // doctype and no stall.
        for src in ["<!doctype", "<!doctype ", "<!doctype x", "<!doctype x ", "<!doctype public"] {
            let (toks, errs) = tokenize(src);
            let doctypes = toks.iter().filter(|t| matches!(t, Token::Doctype { .. })).count();
            assert_eq!(doctypes, 1, "{src:?}: expected one doctype, got {toks:?}");
            assert_eq!(toks.last(), Some(&Token::Eof), "{src:?}");
            assert!(
                !errs.iter().any(|e| e.kind == "tokenizer-stalled"),
                "{src:?}: tripped the progress guard"
            );
        }
    }

    #[test]
    fn empty_comment() {
        assert_eq!(toks("<!---->"), vec![Token::Comment(String::new()), Token::Eof]);
    }

    #[test]
    fn bogus_comment_from_question_mark() {
        assert_eq!(toks("<?xml x?>"), vec![Token::Comment("?xml x?".into()), Token::Eof]);
    }

    #[test]
    fn doctype_simple() {
        assert_eq!(
            toks("<!DOCTYPE html>"),
            vec![
                Token::Doctype {
                    name: "html".into(),
                    public_id: None,
                    system_id: None,
                    force_quirks: false
                },
                Token::Eof
            ]
        );
    }

    #[test]
    fn doctype_case_insensitive_and_missing_space() {
        let t = toks("<!doctypehtml>");
        match &t[0] {
            Token::Doctype { name, .. } => assert_eq!(name, "html"),
            other => panic!("expected doctype, got {other:?}"),
        }
    }

    #[test]
    fn doctype_public_becomes_bogus_but_still_html() {
        let t = toks("<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\">");
        match &t[0] {
            Token::Doctype { name, force_quirks, .. } => {
                assert_eq!(name, "html");
                assert!(!force_quirks, "a doctype with an id is still not force-quirks here");
            }
            other => panic!("expected doctype, got {other:?}"),
        }
    }

    #[test]
    fn totally_bogus_doctype_forces_quirks() {
        let t = toks("<!DOCTYPE>");
        match &t[0] {
            Token::Doctype { force_quirks, .. } => assert!(force_quirks),
            other => panic!("expected doctype, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_tag_at_eof_is_dropped_per_spec() {
        // An EOF inside a tag is a parse error and the incomplete tag is
        // discarded (HTML §13.2.5.6 "tag name state", EOF branch). Browsers do
        // the same, so this is spec behaviour, not a shortcut.
        let t = toks("<div");
        assert_eq!(t, vec![Token::Eof]);
    }

    #[test]
    fn unclosed_tag_at_eof_reports_an_error() {
        let (_, errs) = tokenize("<div");
        assert!(
            errs.iter().any(|e| e.kind == "eof-in-tag"),
            "the drop must be visible as a parse error, not silent"
        );
    }

    #[test]
    fn stray_lt_is_text() {
        assert_eq!(toks("a < b"), vec![chars("a < b"), Token::Eof]);
    }

    #[test]
    fn adjacent_chars_coalesce_across_reference() {
        // "a" + "&amp;" + "b" must arrive as one text run.
        assert_eq!(toks("a&amp;b"), vec![chars("a&b"), Token::Eof]);
    }

    #[test]
    fn text_and_tag_order_preserved() {
        let t = toks("x<i>y</i>z");
        assert_eq!(t, vec![chars("x"), start("i"), chars("y"), end("i"), chars("z"), Token::Eof]);
    }

    #[test]
    fn null_in_text_becomes_replacement() {
        // The tokenizer keeps NUL in Char runs; the spec's text handling drops
        // it, but preserving it visibly is more honest than silently deleting.
        let t = toks("a\u{0}b");
        assert_eq!(t, vec![chars("a\u{0}b"), Token::Eof]);
    }

    #[test]
    fn empty_input() {
        assert_eq!(toks(""), vec![Token::Eof]);
    }

    #[test]
    fn non_ascii_text_preserved() {
        assert_eq!(toks("héllo 世界"), vec![chars("héllo 世界"), Token::Eof]);
    }

    #[test]
    fn script_content_is_raw() {
        let t = toks("<script>if (a < b && c) { x(); }</script>");
        assert_eq!(
            t,
            vec![
                start("script"),
                chars("if (a < b && c) { x(); }"),
                end("script"),
                Token::Eof
            ]
        );
    }

    #[test]
    fn style_content_is_raw_and_keeps_braces() {
        let t = toks("<style>a > b { color: red }</style>");
        assert_eq!(t[1], chars("a > b { color: red }"));
    }

    #[test]
    fn rcdata_decodes_references() {
        let t = toks("<title>a &amp; b</title>");
        assert_eq!(t, vec![start("title"), chars("a & b"), end("title"), Token::Eof]);
    }

    #[test]
    fn rawtext_does_not_decode_references() {
        let t = toks("<script>x &amp; y</script>");
        assert_eq!(t[1], chars("x &amp; y"), "script content is verbatim");
    }

    #[test]
    fn raw_text_end_tag_case_insensitive() {
        let t = toks("<script>a</SCRIPT>");
        assert_eq!(t, vec![start("script"), chars("a"), end("script"), Token::Eof]);
    }

    #[test]
    fn raw_text_tag_lookalike_is_content() {
        // `</scripts>` is not the end tag.
        let t = toks("<script>a</scripts>b</script>");
        assert_eq!(t, vec![start("script"), chars("a</scripts>b"), end("script"), Token::Eof]);
    }
}