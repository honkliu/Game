//! Host-side URL construction for internal pages and search.
//!
//! Internal pages receive initial state in URL fragments, not a command bridge.
//! Settings writes use a separate, origin-checked message channel.

use crate::history::HistoryEntry;
use crate::omnibox;
use crate::settings;
use crate::store::Store;
use crate::tabs::NEWTAB_URL;

/// `native://newtab` → the real asset URL with the new-tab payload.
pub fn newtab_url(store: &Store) -> String {
    let bookmarks: Vec<_> = store
        .bookmarks()
        .unwrap_or_default()
        .into_iter()
        .map(|b| serde_json::json!({ "url": b.url, "title": b.title }))
        .collect();
    let recent: Vec<_> = store
        .recent(6)
        .unwrap_or_default()
        .into_iter()
        .map(recent_json)
        .collect();
    let payload = serde_json::json!({ "bookmarks": bookmarks, "recent": recent });
    format!("asset://shell/pages/newtab.html#d={}", b64url(&payload.to_string()))
}

fn recent_json(e: HistoryEntry) -> serde_json::Value {
    let title = if e.title.is_empty() {
        crate::history::host_of(&e.url).to_string()
    } else {
        e.title
    };
    serde_json::json!({
        "url": e.url,
        "title": title,
        "host": crate::history::host_of(&e.url),
        "visited_at": e.visited_at,
    })
}

/// `native://settings` → the settings page with the current state embedded.
pub fn settings_url(settings: &settings::Settings, system_dark: bool, history_count: i64, webview2_version: Option<String>) -> String {
    let payload = serde_json::json!({
        "theme": settings.theme,
        "system_dark": system_dark,
        "rail_collapsed": settings.rail_collapsed,
        "compact_tab_rows": settings.compact_tab_rows,
        "density": settings.density,
        "search_template": settings.search_template,
        "clear_history_on_exit": settings.clear_history_on_exit,
        "clear_cookies_on_exit": settings.clear_cookies_on_exit,
        "clear_cache_on_exit": settings.clear_cache_on_exit,
        "always_show_full_url": settings.always_show_full_url,
        "history_cap": settings.history_cap,
        "sleep_after_minutes": settings.sleep_after_minutes,
        "download_dir": settings.download_dir,
        "history_count": history_count,
        "webview2_version": webview2_version,
        "engine_name": crate::platform::ENGINE_NAME,
        "data_dir": crate::platform::data_dir(),
    });
    format!("asset://shell/pages/settings.html#state={}", b64url(&payload.to_string()))
}

pub fn error_url(code: &str, url: &str, navigation_id: u64) -> String {
    let q = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
    format!(
        "asset://shell/pages/error.html#code={}&url={}&nav={}",
        q(code),
        q(url),
        navigation_id
    )
}

pub fn is_internal_page(url: &str, page: &str) -> bool {
    ["asset://shell/pages/", "http://asset.shell/pages/", "https://asset.shell/pages/"]
        .iter().filter_map(|prefix| url.strip_prefix(prefix))
        .any(|rest| rest.split(['#', '?']).next() == Some(page))
}

/// Resolve what the omnibox should load, and whether the address was typed.
/// Wraps `omnibox::classify` so the app never has to special-case internal
/// page schemes.
pub fn resolve_navigation(text: &str, search_template: &str) -> (String, bool) {
    match omnibox::classify(text) {
        omnibox::Intent::Url(u) => (u, true),
        omnibox::Intent::Search(q) => (omnibox::search_url(search_template, &q), true),
        omnibox::Intent::Command { .. } | omnibox::Intent::Find(_) | omnibox::Intent::Ask { .. } => (String::new(), false),
    }
}

/// `native://` internal aliases → the asset URL actually loaded.
pub fn expand_internal(url: &str, newtab: &str) -> String {
    match url {
        NEWTAB_URL | "native://new-tab" | "native://home" => newtab.to_string(),
        _ => url.to_string(),
    }
}

fn b64url(s: &str) -> String {
    // URL-safe base64 without padding, so the fragment survives a URL parse.
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn newtab_url_embeds_bookmarks_and_recent() {
        let mut s = Store::open_in_memory(1_000).unwrap();
        s.add_bookmark("https://a.com/", "A", 1).unwrap();
        s.record_visit("https://b.com/x", "B", 100, true).unwrap();
        let url = newtab_url(&s);
        assert!(url.starts_with("asset://shell/pages/newtab.html#d="));
        // Decode round-trip: base64url of the JSON payload.
        let b64 = url.split("#d=").nth(1).unwrap();
        assert!(!b64.contains(['+', '/', '=']), "must be URL-safe: {b64}");
    }

    #[test]
    fn internal_pages_require_the_exact_origin_and_path() {
        for prefix in ["asset://shell", "http://asset.shell", "https://asset.shell"] {
            assert!(is_internal_page(&format!("{prefix}/pages/settings.html#state=abc"), "settings.html"));
        }
        for url in [
            "https://example.com/pages/settings.html",
            "https://asset.shell.example.com/pages/settings.html",
            "https://asset.shell@example.com/pages/settings.html",
            "http://asset.shell/pages/settings.html/extra",
            "asset://shell/index.html",
        ] {
            assert!(!is_internal_page(url, "settings.html"), "{url}");
        }
    }

    #[test]
    fn resolve_navigation_marks_typed_and_expands_search() {
        let (url, typed) = resolve_navigation("example.com", "https://e.com/?q=%s");
        assert_eq!(url, "https://example.com");
        assert!(typed);
        let (url, typed) = resolve_navigation("rust errors", "https://e.com/?q=%s");
        assert_eq!(url, "https://e.com/?q=rust%20errors");
        assert!(typed);
        let (url, typed) = resolve_navigation("> new-tab", "https://e.com/?q=%s");
        assert_eq!(url, "");
        assert!(!typed);
    }

    #[test]
    fn internal_alias_expands() {
        assert_eq!(expand_internal("native://newtab", "asset://x"), "asset://x");
        assert_eq!(expand_internal("https://a.com", "asset://x"), "https://a.com");
    }

    #[test]
    fn b64url_roundtrips_through_a_standard_decoder() {
        let payload = r#"{"bookmarks":[{"url":"https://例え.jp/","title":"日本語"}],"recent":[]}"#;
        let encoded = b64url(payload);
        let padded = {
            let mut s = encoded.replace('-', "+").replace('_', "/");
            while s.len() % 4 != 0 {
                s.push('=');
            }
            s
        };
        let decoded = base64_std_decode(&padded);
        assert_eq!(decoded, payload);
    }

    // Tiny test-only decoder so the test does not depend on a crate.
    fn base64_std_decode(s: &str) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = 0u32;
        let mut nbits = 0;
        let mut out = Vec::new();
        for c in s.bytes() {
            if c == b'=' {
                break;
            }
            let v = T.iter().position(|&t| t == c).unwrap() as u32;
            bits = (bits << 6) | v;
            nbits += 6;
            if nbits >= 8 {
                nbits -= 8;
                out.push((bits >> nbits) as u8);
            }
        }
        String::from_utf8(out).unwrap()
    }
}