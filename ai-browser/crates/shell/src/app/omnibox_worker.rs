use super::*;
use std::sync::mpsc::{channel, Sender};

pub(super) struct Query {
    pub source: Surface,
    pub id: Option<u64>,
    pub text: String,
    pub limit: usize,
    pub base: Value,
}

pub(super) fn completion(url: &str, typed: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let mut candidate = if typed.contains("://") {
        url.to_string()
    } else {
        let rest = url.split_once("://")?.1;
        if typed.to_ascii_lowercase().starts_with("www.") { rest.to_string() }
        else { rest.strip_prefix("www.").unwrap_or(rest).to_string() }
    };
    let host_start = if typed.contains("://") { candidate.find("://")? + 3 } else { 0 };
    if !typed[typed.find("://").map_or(0, |i| i + 3)..].contains(['/', '?', '#']) {
        let end = candidate[host_start..].find(['/', '?', '#']).map(|i| i + host_start).unwrap_or(candidate.len());
        candidate.truncate(end);
    }
    if typed.is_empty() || candidate.len() <= typed.len() || !candidate.is_char_boundary(typed.len())
        || !candidate[..typed.len()].eq_ignore_ascii_case(typed) {
        return None;
    }
    Some(format!("{typed}{}", &candidate[typed.len()..]))
}

pub(super) fn start(path: PathBuf, proxy: EventLoopProxy<AppMessage>) -> Sender<Query> {
    let (sender, receiver) = channel::<Query>();
    std::thread::spawn(move || {
        let store = Store::open_read_only(&path).ok();
        while let Ok(query) = receiver.recv() {
            let mut latest = vec![query];
            while let Ok(next) = receiver.try_recv() {
                if let Some(position) = latest.iter().position(|old| old.source == next.source) {
                    let old = latest.remove(position);
                    let _ = proxy.send_event(AppMessage::OmniboxReady(old.source, old.id, json!({ "rows": [], "superseded": true })));
                }
                latest.push(next);
            }
            for mut query in latest {
                let mode = crate::omnibox::classify(&query.text).mode();
                if matches!(mode, Mode::Search | Mode::Url) {
                    if let Some(store) = &store {
                        let prefix = store.autocomplete(&query.text, query.limit).unwrap_or_default();
                        let inline = prefix.iter().find_map(|entry| completion(&entry.url, &query.text));
                        let entries = if query.text.trim().is_empty() {
                            store.recent(query.limit).unwrap_or_default()
                        } else {
                            store.search_history(&history::parse_query(&query.text), query.limit, ipc::now_secs()).unwrap_or_default()
                        };
                        let mut rows: Vec<Value> = prefix.iter().map(|entry| json!({
                            "kind": "history", "url": entry.url, "title": entry.title,
                            "hint": entry.url, "completion": completion(&entry.url, &query.text),
                        })).collect();
                        if let Some(original) = query.base["rows"].as_array() {
                            rows.extend(original.iter().cloned());
                        }
                        for entry in entries {
                            if !rows.iter().any(|row| row["url"] == entry.url) {
                                rows.push(json!({ "kind": "history", "url": entry.url,
                                    "title": entry.title, "hint": entry.url }));
                            }
                        }
                        rows.truncate(query.limit);
                        query.base["rows"] = json!(rows);
                        query.base["completion"] = json!(inline);
                    }
                }
                let _ = proxy.send_event(AppMessage::OmniboxReady(query.source, query.id, query.base));
            }
        }
    });
    sender
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completes_hosts_and_paths_without_forcing_a_scheme() {
        assert_eq!(completion("https://www.example.com/docs/start", "exa").as_deref(), Some("example.com"));
        assert_eq!(completion("https://example.com/docs/start", "example.com/do").as_deref(), Some("example.com/docs/start"));
        assert_eq!(completion("https://example.com/", "https://exa").as_deref(), Some("https://example.com"));
        assert!(completion("https://user:password@example.com/", "exa").is_none());
        assert!(completion("https://example.com/", "other").is_none());
    }
}
