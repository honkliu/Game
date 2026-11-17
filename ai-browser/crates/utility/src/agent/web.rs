//! Public-only `fetch_url` access and pure HTML/search-result parsing.
//!
//! Requests are made by the utility (not the page), so they carry no cookies and
//! cannot reach the user's logged-in sessions. Only public http(s) hosts are allowed,
//! redirects are followed by hand so every hop is checked again, and bodies are capped.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

const MAX_BODY: u64 = 3 * 1024 * 1024;
const MAX_HOPS: usize = 4;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) ai-browser-agent/1.0")
        .build()
}

fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast()
                || v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// Only public http(s) destinations. Returns the reason when refused.
pub fn check_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).ok_or("only http(s) URLs can be fetched")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !h.ends_with(']') || authority.starts_with('[') && authority.contains("]:") => {
            (h.trim_matches(['[', ']']), p.parse::<u16>().unwrap_or(443))
        }
        _ => (authority.trim_matches(['[', ']']), if url.starts_with("https") { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err("URL has no host".into());
    }
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") || host.ends_with(".internal") {
        return Err("local addresses are not fetched by the agent".into());
    }
    let addrs = (host, port).to_socket_addrs().map_err(|_| "host did not resolve".to_string())?;
    for addr in addrs {
        if private_ip(addr.ip()) {
            return Err("private network addresses are not fetched by the agent".into());
        }
    }
    Ok(())
}

pub struct Fetched {
    pub final_url: String,
    pub content_type: String,
    pub body: String,
}

pub fn fetch(url: &str) -> Result<Fetched, String> {
    let agent = agent();
    let mut current = url.to_string();
    for _ in 0..=MAX_HOPS {
        check_url(&current)?;
        match agent.get(&current).set("Accept", "text/html,application/xhtml+xml,text/plain,application/json;q=0.9,*/*;q=0.5").call() {
            Ok(response) => {
                let content_type = response.content_type().to_string();
                let mut bytes = Vec::new();
                response.into_reader().take(MAX_BODY).read_to_end(&mut bytes).map_err(|e| format!("read failed: {e}"))?;
                return Ok(Fetched { final_url: current, content_type, body: String::from_utf8_lossy(&bytes).to_string() });
            }
            Err(ureq::Error::Status(code, response)) if (300..400).contains(&code) => {
                let location = response.header("location").ok_or("redirect without a location")?;
                current = resolve(&current, location);
            }
            Err(ureq::Error::Status(code, _)) => return Err(format!("the server answered HTTP {code}")),
            Err(ureq::Error::Transport(t)) => return Err(format!("could not connect ({:?})", t.kind())),
        }
    }
    Err("too many redirects".into())
}

fn resolve(base: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let origin_end = base[scheme_end..].find('/').map_or(base.len(), |i| i + scheme_end);
    if let Some(path) = location.strip_prefix('/') {
        return format!("{}/{}", &base[..origin_end], path);
    }
    let dir_end = base.rfind('/').filter(|i| *i >= origin_end).unwrap_or(origin_end);
    format!("{}/{}", &base[..dir_end], location)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn remove_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_lowercase();
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find(&open) {
        let start = at + i;
        out.push_str(&html[at..start]);
        match lower[start..].find(&close) {
            Some(j) => at = start + j + close.len(),
            None => return out,
        }
    }
    out.push_str(&html[at..]);
    out
}

/// Title and readable text of an HTML document, block structure kept as line breaks.
pub fn html_to_text(html: &str) -> (String, String) {
    let title = html
        .to_lowercase()
        .find("<title")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
        .and_then(|s| html[s..].to_lowercase().find("</title>").map(|e| decode_entities(html[s..s + e].trim())))
        .unwrap_or_default();
    let mut cleaned = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "template", "head"] {
        cleaned = remove_blocks(&cleaned, tag);
    }
    let mut out = String::with_capacity(cleaned.len() / 2);
    let mut in_tag = false;
    let mut tag = String::new();
    for c in cleaned.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("").to_lowercase();
                if matches!(name.as_str(), "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "ul" | "ol" | "table" | "header" | "footer" | "pre" | "blockquote") {
                    out.push('\n');
                } else if matches!(name.as_str(), "td" | "th") {
                    out.push(' ');
                }
            }
            _ if in_tag => {
                if tag.len() < 16 {
                    tag.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    let text = decode_entities(&out);
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() {
            lines.push(line);
        }
    }
    (title, lines.join("\n"))
}

/// DuckDuckGo's HTML endpoint, parsed for `(title, url, snippet)`.
pub fn search_results(html: &str, limit: usize) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("class=\"result__a\"") {
        rest = &rest[i..];
        let href = attr(rest, "href").unwrap_or_default();
        let title_end = rest.find("</a>").unwrap_or(rest.len());
        let title = html_to_text(&rest[rest.find('>').map_or(0, |p| p + 1)..title_end]).1;
        rest = &rest[title_end.min(rest.len())..];
        let snippet = match rest.find("result__snippet") {
            Some(s) if s < 3000 => {
                let seg = &rest[s..];
                let from = seg.find('>').map_or(0, |p| p + 1);
                let to = seg.find("</a>").unwrap_or(seg.len());
                html_to_text(&seg[from..to.max(from)]).1
            }
            _ => String::new(),
        };
        let url = real_url(&decode_entities(&href));
        if url.starts_with("http") && !title.is_empty() {
            results.push((title, url, snippet));
        }
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn attr(tag_start: &str, name: &str) -> Option<String> {
    let head_end = tag_start.find('>')?;
    let head = &tag_start[..head_end];
    let key = format!("{name}=\"");
    let s = head.find(&key)? + key.len();
    let e = head[s..].find('"')?;
    Some(head[s..s + e].to_string())
}

/// DuckDuckGo wraps result links as `//duckduckgo.com/l/?uddg=<encoded>`.
fn real_url(href: &str) -> String {
    let Some(i) = href.find("uddg=") else {
        return href.trim_start_matches("//").to_string();
    };
    let enc = href[i + 5..].split('&').next().unwrap_or("");
    let bytes = enc.as_bytes();
    let mut out = Vec::new();
    let mut k = 0;
    while k < bytes.len() {
        if bytes[k] == b'%' && k + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&enc[k + 1..k + 3], 16) {
                out.push(v);
                k += 3;
                continue;
            }
        }
        out.push(if bytes[k] == b'+' { b' ' } else { bytes[k] });
        k += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn private_and_local_destinations_are_refused() {
        for url in ["http://127.0.0.1/", "http://localhost:8080/x", "http://10.1.2.3/", "http://192.168.0.5/", "http://169.254.169.254/latest", "ftp://x/", "http://[::1]/"] {
            assert!(check_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn html_becomes_text_with_a_title() {
        let (title, text) = html_to_text("<html><head><title>T &amp; U</title><style>p{}</style></head><body><h1>Hi</h1><p>one <b>two</b></p><script>x()</script></body></html>");
        assert_eq!(title, "T & U");
        assert_eq!(text, "Hi\none two");
    }

    #[test]
    fn duckduckgo_redirect_links_are_unwrapped() {
        assert_eq!(real_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1&rut=x"), "https://example.com/a?b=1");
    }

    #[test]
    fn relative_redirects_resolve() {
        assert_eq!(resolve("https://a.com/x/y", "/z"), "https://a.com/z");
        assert_eq!(resolve("https://a.com/x/y", "w"), "https://a.com/x/w");
    }
}
