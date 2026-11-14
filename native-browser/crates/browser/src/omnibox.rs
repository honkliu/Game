//! Omnibox input classification.
//!
//! Implements §3.3 "Mode resolution (deterministic, evaluated in order)".
//! This module is pure: no I/O, no state, so it is unit-testable and runs in
//! the hot path of every keystroke without allocation beyond the input String.

use serde::Serialize;

/// What the omnibox will do with the current text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Url,
    Search,
    Command,
    Find,
}

impl Mode {
    /// The label rendered in the mode chip (§3.3).
    pub fn chip_label(self) -> &'static str {
        match self {
            Mode::Url => "URL",
            Mode::Search => "Search",
            Mode::Command => "› Command",
            Mode::Find => "🔎 Find",
        }
    }
}

/// Result of classifying the omnibox text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Navigate to this absolute URL.
    Url(String),
    /// Issue this string as a search query on the configured engine.
    Search(String),
    /// Run the command verb (`> foo bar` → verb `foo`, args `bar`).
    Command { verb: String, rest: String },
    /// Find `query` in the current page.
    Find(String),
}

impl Intent {
    pub fn mode(&self) -> Mode {
        match self {
            Intent::Url(_) => Mode::Url,
            Intent::Search(_) => Mode::Search,
            Intent::Command { .. } => Mode::Command,
            Intent::Find(_) => Mode::Find,
        }
    }
}

/// Schemes we accept verbatim without a heuristic.
const KNOWN_SCHEMES: &[&str] = &[
    "http", "https", "file", "ftp", "view-source", "about", "data", "native", "asset", "mailto",
    "ws", "wss", "chrome-extension",
];

fn is_known_scheme(scheme: &str) -> bool {
    let lower = scheme.to_ascii_lowercase();
    KNOWN_SCHEMES.iter().any(|s| *s == lower)
}

/// Split `"http://x"` into `("http", "//x")` when the prefix is scheme-shaped
/// (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`).
fn split_scheme(input: &str) -> Option<(&str, &str)> {
    let colon = input.find(':')?;
    let scheme = &input[..colon];
    if scheme.is_empty() {
        return None;
    }
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some((scheme, &input[colon + 1..]))
}

fn is_ipv4(host: &str) -> bool {
    let mut parts = 0;
    for part in host.split('.') {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        match part.parse::<u16>() {
            Ok(v) if v <= 255 => {}
            _ => return false,
        }
        parts += 1;
    }
    parts == 4
}

fn is_localhost(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
}

/// A last label likely to be a TLD: 2+ ASCII alphabetic chars, or an IDN
/// punycode label (`xn--…`). Deliberately shape-only — we do not ship a TLD
/// list and we do not make DNS lookups, per the zero-egress rule.
fn is_tld_shaped(label: &str) -> bool {
    if label.starts_with("xn--") {
        return label.len() > 4;
    }
    label.len() >= 2 && label.bytes().all(|b| b.is_ascii_alphabetic())
}

/// Split a bare (schemeless) input into `(host_port, rest)`.
///
/// `"docs.rs/serde"` → `("docs.rs", "/serde")`, `"a.b?q"` → `("a.b", "?q")`.
fn split_host_port(s: &str) -> (&str, &str) {
    match s.find(['/', '?', '#']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// Separate `host` from an optional `:port`.
///
/// `rsplit_once(':')` alone mis-splits IPv6 literals: `[::1]:8080` would become
/// host `[::1]` only by luck, and `[::1]` (no port) would become host `[:` with
/// port `:1]`. Bracketed literals are handled by their `]` instead.
fn host_and_port(host_port: &str) -> (&str, Option<&str>) {
    if let Some(rest) = host_port.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((host, after)) => (
                &host_port[..host.len() + 2],
                after.strip_prefix(':').filter(|p| is_port(p)),
            ),
            // Unterminated `[`: not a bracketed host at all.
            None => (host_port, None),
        };
    }
    match host_port.rsplit_once(':') {
        Some((h, p)) if is_port(p) => (h, Some(p)),
        _ => (host_port, None),
    }
}

fn is_port(p: &str) -> bool {
    !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())
}

/// A bracketed IPv6 literal: `[::1]`, `[2001:db8::1]`.
fn is_ipv6_literal(host: &str) -> bool {
    let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) else {
        return false;
    };
    !inner.is_empty()
        && inner.contains(':')
        && inner
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.' || b == b'%')
}

/// Does this schemeless input start with `localhost` or an IP literal,
/// optionally with `:port` and a path? (`127.0.0.1:3000/api`, `localhost/a`,
/// `[::1]:8080`)
fn is_local_host_or_ip(s: &str) -> bool {
    let (host_port, _) = split_host_port(s);
    let (host, _) = host_and_port(host_port);
    is_localhost(host) || is_ipv4(host) || is_ipv6_literal(host)
}

/// Rule 5's host test: has a dot, no space, and a TLD-shaped last label.
/// The *path* may contain dots and slashes; only the host segment is judged.
///
/// A single trailing dot is a valid fully-qualified host (`example.com.`) and
/// is accepted; DNS treats it as the root-anchored form of the same name.
fn looks_like_bare_host(s: &str) -> bool {
    let (host_port, _) = split_host_port(s);
    let (host, _) = host_and_port(host_port);
    if host.contains(char::is_whitespace) {
        return false;
    }
    // `host_port` may be bracketed IPv6 or `host:port`; for the dotted-name
    // rule only a plain name can qualify, so a bracketed literal is not a
    // "bare host" (it is already handled as an IP).
    let host = host.strip_suffix('.').unwrap_or(host);
    if !host.contains('.') || host.starts_with('.') {
        return false;
    }
    match host.rsplit('.').next() {
        Some(last) => is_tld_shaped(last),
        None => false,
    }
}

/// Classify omnibox text per §3.3. Order matters and is tested.
///
/// The search engine template substitution happens in `search_url`, not here:
/// this function only decides *intent*.
pub fn classify(raw: &str) -> Intent {
    // Rules 1-3 use the *literal* prefix (no trimming) so a leading space
    // means "search", matching the design's "starts with `> `" wording.
    if let Some(rest) = raw.strip_prefix("> ") {
        let (verb, args) = match rest.split_once(char::is_whitespace) {
            Some((v, a)) => (v, a),
            None => (rest, ""),
        };
        return Intent::Command {
            verb: verb.to_string(),
            rest: args.to_string(),
        };
    }
    // A bare ">" with nothing after it is command mode with an empty verb.
    if raw == ">" {
        return Intent::Command {
            verb: String::new(),
            rest: String::new(),
        };
    }
    if let Some(rest) = raw.strip_prefix("? ") {
        return Intent::Search(rest.to_string());
    }
    if raw == "?" {
        return Intent::Search(String::new());
    }
    if let Some(rest) = raw.strip_prefix("/ ") {
        return Intent::Find(rest.to_string());
    }

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Intent::Search(String::new());
    }

    // Rule 4a: explicit scheme. Checked first so `http://localhost:1` is
    // not mistaken for a bare host.
    if let Some((scheme, _)) = split_scheme(trimmed) {
        if is_known_scheme(scheme) {
            return Intent::Url(trimmed.to_string());
        }
    }

    // Rule 4b: bare localhost / IPv4, with an optional port and path.
    // Done before the bare-host rule because `localhost:8080` is scheme-shaped
    // (`localhost` matches scheme syntax) but is obviously not a scheme.
    if is_local_host_or_ip(trimmed) {
        return Intent::Url(format!("http://{trimmed}"));
    }

    // Rule 5: dot + TLD-shaped last label, no spaces.
    if !trimmed.contains(char::is_whitespace) && looks_like_bare_host(trimmed) {
        return Intent::Url(format!("https://{trimmed}"));
    }

    // Rule 6.
    Intent::Search(trimmed.to_string())
}

/// Characters left unencoded in a search query: unreserved per RFC 3986.
const QUERY_SAFE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Substitute a query into a URL template containing `%s`.
///
/// Anything else in the template (`%d`, a literal `%`) is left untouched so a
/// user-supplied template is not silently mangled.
pub fn search_url(template: &str, query: &str) -> String {
    let encoded = percent_encoding::utf8_percent_encode(query, QUERY_SAFE).to_string();
    match template.find("%s") {
        Some(idx) => {
            let mut out = String::with_capacity(template.len() + encoded.len());
            out.push_str(&template[..idx]);
            out.push_str(&encoded);
            out.push_str(&template[idx + 2..]);
            out
        }
        None => template.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_prefix_wins_over_everything() {
        assert_eq!(
            classify("> https://example.com"),
            Intent::Command {
                verb: "https://example.com".into(),
                rest: String::new()
            }
        );
        assert_eq!(
            classify("> tab close"),
            Intent::Command {
                verb: "tab".into(),
                rest: "close".into()
            }
        );
        assert_eq!(
            classify(">"),
            Intent::Command {
                verb: String::new(),
                rest: String::new()
            }
        );
    }

    #[test]
    fn forced_search_and_find_prefixes() {
        assert_eq!(classify("? rust errors"), Intent::Search("rust errors".into()));
        assert_eq!(classify("/ needle"), Intent::Find("needle".into()));
        // A legacy "?" with no space is URL-ish, not search.
        assert_eq!(classify("example.com?q=1"), Intent::Url("https://example.com?q=1".into()));
    }

    #[test]
    fn explicit_schemes_pass_through() {
        assert_eq!(
            classify("https://example.com/a?b=1"),
            Intent::Url("https://example.com/a?b=1".into())
        );
        assert_eq!(classify("about:blank"), Intent::Url("about:blank".into()));
        assert_eq!(classify("view-source:https://a.b"), Intent::Url("view-source:https://a.b".into()));
        assert_eq!(classify("mailto:a@b.c"), Intent::Url("mailto:a@b.c".into()));
    }

    #[test]
    fn localhost_and_ip_get_http_not_https() {
        assert_eq!(classify("localhost"), Intent::Url("http://localhost".into()));
        assert_eq!(
            classify("localhost:8080/app"),
            Intent::Url("http://localhost:8080/app".into())
        );
        assert_eq!(
            classify("127.0.0.1:3000"),
            Intent::Url("http://127.0.0.1:3000".into())
        );
        assert_eq!(classify("192.168.1.1"), Intent::Url("http://192.168.1.1".into()));
        assert_eq!(
            classify("127.0.0.1:3000/api?x=1"),
            Intent::Url("http://127.0.0.1:3000/api?x=1".into())
        );
        // 256 is not a valid octet → not an IP → falls through to search.
        assert_eq!(classify("999.1.1.1"), Intent::Search("999.1.1.1".into()));
    }

    #[test]
    fn bare_host_with_tld_is_url() {
        assert_eq!(classify("example.com"), Intent::Url("https://example.com".into()));
        assert_eq!(
            classify("docs.rs/serde/latest"),
            Intent::Url("https://docs.rs/serde/latest".into())
        );
        assert_eq!(classify("Example.COM"), Intent::Url("https://Example.COM".into()));
        assert_eq!(
            classify("xn--bcher-kva.de"),
            Intent::Url("https://xn--bcher-kva.de".into())
        );
    }

    #[test]
    fn sentence_with_dots_but_spaces_is_search() {
        assert_eq!(
            classify("what is a .png file"),
            Intent::Search("what is a .png file".into())
        );
        assert_eq!(classify("node.js"), Intent::Url("https://node.js".into()));
        // A one-character TLD shape is not a host (`.b` could be a version
        // number); `.c` likewise. Typing them out is the only way to fix it,
        // and the dropdown always offers the search row.
        assert_eq!(classify("a.b"), Intent::Search("a.b".into()));
        assert_eq!(classify("weird.c"), Intent::Search("weird.c".into()));
        // A single trailing dot is an FQDN, covered in `trailing_dot_fqdn_is_still_a_host`.
    }

    #[test]
    fn ipv6_literals_are_urls() {
        // `rsplit_once(':')` used to split `[::1]` into host `[:` / port `:1]`,
        // which fell through to search.
        assert_eq!(classify("[::1]"), Intent::Url("http://[::1]".into()));
        assert_eq!(
            classify("[::1]:8080/api"),
            Intent::Url("http://[::1]:8080/api".into())
        );
        assert_eq!(
            classify("[2001:db8::1]:443"),
            Intent::Url("http://[2001:db8::1]:443".into())
        );
        // Bracketed but not an IP (no colon): not a host, falls to search.
        assert_eq!(classify("[hello]"), Intent::Search("[hello]".into()));
    }

    #[test]
    fn trailing_dot_fqdn_is_still_a_host() {
        assert_eq!(
            classify("example.com."),
            Intent::Url("https://example.com.".into())
        );
        assert_eq!(
            classify("example.com./path"),
            Intent::Url("https://example.com./path".into())
        );
        // Two trailing dots is not a host name.
        assert_eq!(classify("example.com.."), Intent::Search("example.com..".into()));
    }

    #[test]
    fn host_and_port_splits_brackets_closing_first() {
        assert_eq!(host_and_port("[::1]:8080"), ("[::1]", Some("8080")));
        assert_eq!(host_and_port("[::1]"), ("[::1]", None));
        // A bare (unbracketed) IPv6 is not an authority: `::1` splits to host `:`
        // and port `1`, which matches no rule, so it ends up as a search. The
        // bracketed form is the one that must work.
        assert_eq!(classify("::1"), Intent::Search("::1".into()));
        assert_eq!(host_and_port("host:abc"), ("host:abc", None), "non-numeric port is not a port");
    }

    #[test]
    fn empty_is_empty_search() {
        assert_eq!(classify(""), Intent::Search(String::new()));
        assert_eq!(classify("   "), Intent::Search(String::new()));
    }

    #[test]
    fn mode_matches_intent() {
        assert_eq!(classify("> x").mode(), Mode::Command);
        assert_eq!(classify("/ x").mode(), Mode::Find);
        assert_eq!(classify("? x").mode(), Mode::Search);
        assert_eq!(classify("example.com").mode(), Mode::Url);
    }

    #[test]
    fn search_url_encodes_and_substitutes_once() {
        assert_eq!(
            search_url("https://duckduckgo.com/html/?q=%s", "a b&c"),
            "https://duckduckgo.com/html/?q=a%20b%26c"
        );
        // `%s` appears twice: only the first is replaced (find, not replace_all).
        assert_eq!(search_url("x%s%s", "q"), "xq%s");
        // No placeholder → template unchanged, query dropped: caller must not
        // silently pretend it searched. This is asserted so it cannot regress.
        assert_eq!(search_url("https://e.com/s", "q"), "https://e.com/s");
        // Unreserved characters survive.
        assert_eq!(search_url("%s", "a-b_c.d~e"), "a-b_c.d~e");
    }
}