//! Element tag names, namespaces, and the small static tables the DOM needs
//! (void elements, raw-text elements, block defaults).

use std::fmt;

/// An element's namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Namespace {
    #[default]
    Html,
    Svg,
    MathMl,
}

/// A lowercased element tag name. Interned via a static table for the common
/// HTML elements and a boxed string for everything else, so equality on the
/// hot selector path is a `u16` compare instead of a string compare.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tag(TagRepr);

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum TagRepr {
    Known(u16),
    Other(Box<str>),
}

macro_rules! known_tags {
    ($($name:ident => $s:literal),* $(,)?) => {
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u16)]
        pub enum KnownTag { $($name),* }

        impl Tag {
            /// Intern a tag name. Comparison with `Tag::known(KnownTag::Div)`
            /// is a single integer compare.
            pub fn lookup(name: &str) -> Tag {
                $(if name.eq_ignore_ascii_case($s) { return Tag(TagRepr::Known(KnownTag::$name as u16)); })*
                Tag(TagRepr::Other(name.to_ascii_lowercase().into_boxed_str()))
            }

            pub fn known(k: KnownTag) -> Tag {
                Tag(TagRepr::Known(k as u16))
            }

            pub fn as_known(&self) -> Option<KnownTag> {
                match &self.0 {
                    TagRepr::Known(n) => Some(KNOWN_BY_IDX[*n as usize]),
                    TagRepr::Other(_) => None,
                }
            }
        }

        const KNOWN_BY_IDX: &[KnownTag] = &[$(KnownTag::$name),*];
    };
}

known_tags! {
    A => "a",
    Abbr => "abbr",
    Address => "address",
    Article => "article",
    Aside => "aside",
    B => "b",
    Bdi => "bdi",
    Bdo => "bdo",
    Blockquote => "blockquote",
    Body => "body",
    Br => "br",
    Button => "button",
    Caption => "caption",
    Center => "center",
    Cite => "cite",
    Code => "code",
    Col => "col",
    Colgroup => "colgroup",
    Dd => "dd",
    Del => "del",
    Details => "details",
    Dfn => "dfn",
    Dialog => "dialog",
    Div => "div",
    Dl => "dl",
    Dt => "dt",
    Em => "em",
    Embed => "embed",
    Fieldset => "fieldset",
    Figcaption => "figcaption",
    Figure => "figure",
    Footer => "footer",
    Form => "form",
    H1 => "h1",
    H2 => "h2",
    H3 => "h3",
    H4 => "h4",
    H5 => "h5",
    H6 => "h6",
    Head => "head",
    Header => "header",
    Hgroup => "hgroup",
    Hr => "hr",
    Html => "html",
    I => "i",
    Iframe => "iframe",
    Img => "img",
    Input => "input",
    Ins => "ins",
    Kbd => "kbd",
    Label => "label",
    Legend => "legend",
    Li => "li",
    Link => "link",
    Main => "main",
    Mark => "mark",
    Menu => "menu",
    Meta => "meta",
    Meter => "meter",
    Nav => "nav",
    Noscript => "noscript",
    Object => "object",
    Ol => "ol",
    Optgroup => "optgroup",
    Option => "option",
    Output => "output",
    P => "p",
    Param => "param",
    Picture => "picture",
    Pre => "pre",
    Progress => "progress",
    Q => "q",
    Rp => "rp",
    Rt => "rt",
    Ruby => "ruby",
    S => "s",
    Samp => "samp",
    Script => "script",
    Section => "section",
    Select => "select",
    Small => "small",
    Source => "source",
    Span => "span",
    Strike => "strike",
    Strong => "strong",
    Style => "style",
    Sub => "sub",
    Summary => "summary",
    Sup => "sup",
    Table => "table",
    Tbody => "tbody",
    Td => "td",
    Template => "template",
    Textarea => "textarea",
    Tfoot => "tfoot",
    Th => "th",
    Thead => "thead",
    Time => "time",
    Title => "title",
    Tr => "tr",
    Track => "track",
    U => "u",
    Ul => "ul",
    Var => "var",
    Video => "video",
    Wbr => "wbr",
}

impl Tag {
    pub fn as_str(&self) -> &str {
        match &self.0 {
            TagRepr::Known(n) => KNOWN_NAMES[*n as usize],
            TagRepr::Other(s) => s,
        }
    }

    pub fn is(&self, k: KnownTag) -> bool {
        matches!(&self.0, TagRepr::Known(n) if *n == k as u16)
    }
}

const KNOWN_NAMES: &[&str] = &[
    "a", "abbr", "address", "article", "aside", "b", "bdi", "bdo", "blockquote", "body", "br",
    "button", "caption", "center", "cite", "code", "col", "colgroup", "dd", "del", "details", "dfn",
    "dialog", "div", "dl", "dt", "em", "embed", "fieldset", "figcaption", "figure", "footer",
    "form", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hgroup", "hr", "html", "i",
    "iframe", "img", "input", "ins", "kbd", "label", "legend", "li", "link", "main", "mark",
    "menu", "meta", "meter", "nav", "noscript", "object", "ol", "optgroup", "option", "output", "p",
    "param", "picture", "pre", "progress", "q", "rp", "rt", "ruby", "s", "samp", "script",
    "section", "select", "small", "source", "span", "strike", "strong", "style", "sub", "summary",
    "sup", "table", "tbody", "td", "template", "textarea", "tfoot", "th", "thead", "time", "title",
    "tr", "track", "u", "ul", "var", "video", "wbr",
];

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tag({})", self.as_str())
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<KnownTag> for Tag {
    fn from(k: KnownTag) -> Self {
        Tag::known(k)
    }
}

/// Elements that never have children (HTML §13.1.2). Restricted to the tags
/// named in `known_tags!`; `<area>`/`<base>` are omitted there and treated as
/// unknown elements, which is harmless because the tokenizer keeps them empty
/// by other means.
pub const VOID_ELEMENTS: &[KnownTag] = &[
    KnownTag::Br, KnownTag::Col, KnownTag::Embed,
    KnownTag::Hr, KnownTag::Img, KnownTag::Input, KnownTag::Link, KnownTag::Meta,
    KnownTag::Param, KnownTag::Source, KnownTag::Track, KnownTag::Wbr,
];

/// Elements whose content is not parsed as markup.
pub const RAW_TEXT_ELEMENTS: &[KnownTag] =
    &[KnownTag::Script, KnownTag::Style, KnownTag::Textarea, KnownTag::Title];

pub fn is_void(tag: &Tag) -> bool {
    tag.as_known().is_some_and(|k| VOID_ELEMENTS.contains(&k))
}

/// Whitespace-only text handling helpers.
pub fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

pub fn is_all_whitespace(s: &str) -> bool {
    s.chars().all(is_ascii_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_case_insensitive_and_interned() {
        assert!(Tag::lookup("DIV").is(KnownTag::Div));
        assert!(Tag::lookup("div").is(KnownTag::Div));
        assert_eq!(Tag::lookup("div"), Tag::lookup("DiV"));
    }

    #[test]
    fn unknown_tags_are_lowercased() {
        let t = Tag::lookup("MyCustomElement");
        assert_eq!(t.as_str(), "mycustomelement");
        assert_eq!(t.as_known(), None);
    }

    #[test]
    fn known_tag_roundtrips_name() {
        assert_eq!(Tag::known(KnownTag::H2).as_str(), "h2");
        assert_eq!(Tag::lookup("blockquote"), Tag::known(KnownTag::Blockquote));
    }

    #[test]
    fn void_elements() {
        assert!(is_void(&Tag::lookup("br")));
        assert!(is_void(&Tag::lookup("img")));
        assert!(!is_void(&Tag::lookup("div")));
    }

    #[test]
    fn whitespace_helpers() {
        assert!(is_all_whitespace(" \t\r\n"));
        assert!(!is_all_whitespace(" a "));
        assert!(is_all_whitespace(""));
    }

    #[test]
    fn known_names_table_matches_enum_count() {
        assert_eq!(KNOWN_NAMES.len(), KNOWN_BY_IDX.len());
    }
}