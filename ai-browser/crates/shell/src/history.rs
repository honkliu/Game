//! History: query tokens, ranking, and the SQL that backs them.
//!
//! SQLite is the source of truth; ranking runs here in Rust so it is
//! unit-testable (W4: history-only search p95 ≤ 50 ms over 50k rows, including
//! the DB query; DB query gate ≤ 15 ms).
//!
//! No FTS5: BM25 over a 50k-row title/url corpus is slower than `score()` and
//! adds a second definition of "matched". A LIKE prefilter runs in SQLite, the
//! ranking runs here.
//!
//! All functions take the shared `Connection` from `store::Store` — the
//! application has exactly one database file and one connection.

use rusqlite::{params, Connection, OptionalExtension, Row};

/// One stored visit-history entry (aggregated per URL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    /// Unix seconds.
    pub visited_at: i64,
    pub visit_count: u32,
    pub typed_count: u32,
}

impl HistoryEntry {
    pub fn host(&self) -> &str {
        host_of(&self.url)
    }
}

/// Extract the host from a URL without a URL crate.
/// `https://u:p@a.b:8443/c?q` → `a.b`, `http://[::1]:8080/` → `[::1]`.
pub fn host_of(url: &str) -> &str {
    let after_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let authority = match authority.rsplit_once('@') {
        Some((_, h)) => h,
        None => authority,
    };
    if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal: the `]` ends the host; a port may follow.
        return match rest.split_once(']') {
            Some((host, _)) => &authority[..host.len() + 2],
            None => authority,
        };
    }
    match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => authority,
    }
}

/// Typed filter tokens (§4.5 / D8): `site:`, `before:`, `after:`, `type:`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Free-text terms; every term must match title or url.
    pub terms: Vec<String>,
    pub site: Option<String>,
    /// Exclusive lower bound, unix seconds.
    pub after: Option<i64>,
    /// Exclusive upper bound, unix seconds.
    pub before: Option<i64>,
    pub kind: Option<HistoryKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Typed,
    Visited,
}

/// Parse the query string used by both the history panel and the palette.
///
/// Unknown `foo:bar` tokens stay plain text — typing a URL must not be eaten
/// by a `scheme:` filter.
pub fn parse_query(input: &str) -> HistoryQuery {
    let mut q = HistoryQuery::default();
    for word in input.split_whitespace() {
        if let Some(v) = word.strip_prefix("site:") {
            if !v.is_empty() {
                q.site = Some(v.to_ascii_lowercase());
                continue;
            }
        }
        if let Some(v) = word.strip_prefix("before:") {
            if let Some(ts) = parse_day(v) {
                q.before = Some(ts);
                continue;
            }
        }
        if let Some(v) = word.strip_prefix("after:") {
            if let Some(ts) = parse_day(v) {
                q.after = Some(ts);
                continue;
            }
        }
        if let Some(v) = word.strip_prefix("type:") {
            match v {
                "typed" => {
                    q.kind = Some(HistoryKind::Typed);
                    continue;
                }
                "visited" => {
                    q.kind = Some(HistoryKind::Visited);
                    continue;
                }
                _ => {}
            }
        }
        q.terms.push(word.to_ascii_lowercase());
    }
    q
}

/// Parse `YYYY-MM-DD` (UTC midnight) or a bare unix-seconds integer.
/// Deliberately no date crate: this is the only date parsing in the app.
pub fn parse_day(s: &str) -> Option<i64> {
    if let Ok(secs) = s.parse::<i64>() {
        return Some(secs);
    }
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400)
}

/// Score a candidate against lowercased terms. Higher is better.
///
/// `recency × frequency × match_quality` (§3.3), decomposed so each factor is
/// testable:
/// - recency: exponential decay, 7-day half-life.
/// - frequency: `log2(1 + visit_count)` — 1000 visits are not 1000× better.
/// - quality: title prefix > title word prefix > title substring > host > url.
pub fn score(entry: &HistoryEntry, terms: &[String], now: i64) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let title_l = entry.title.to_ascii_lowercase();
    let url_l = entry.url.to_ascii_lowercase();

    let mut quality = 0.0f64;
    for term in terms {
        let t = if title_l.starts_with(term.as_str()) {
            10.0
        } else if title_l
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.starts_with(term.as_str()))
        {
            7.0
        } else if title_l.contains(term.as_str()) {
            5.0
        } else if host_of(&url_l).starts_with(term.as_str()) {
            4.0
        } else if url_l.contains(term.as_str()) {
            2.0
        } else {
            return 0.0; // a term did not match: reject
        };
        quality += t;
    }
    let typed = if entry.typed_count > 0 { 1.5 } else { 1.0 };
    let age_days = (now.saturating_sub(entry.visited_at).max(0) as f64) / 86_400.0;
    let recency = 0.5f64.powf(age_days / 7.0);
    let frequency = (1.0 + entry.visit_count as f64).log2().max(1.0);
    quality * typed * recency * frequency
}

/// Do the structured filters admit this entry? Terms are checked in `score`.
fn admits(entry: &HistoryEntry, q: &HistoryQuery) -> bool {
    if let Some(site) = &q.site {
        let host = entry.host().to_ascii_lowercase();
        if host != *site && !host.ends_with(&format!(".{site}")) {
            return false;
        }
    }
    if let Some(after) = q.after {
        if entry.visited_at < after {
            return false;
        }
    }
    if let Some(before) = q.before {
        if entry.visited_at >= before {
            return false;
        }
    }
    match q.kind {
        Some(HistoryKind::Typed) if entry.typed_count == 0 => return false,
        Some(HistoryKind::Visited) if entry.typed_count > 0 => return false,
        _ => {}
    }
    true
}

/// A failure the caller can branch on. One error type for both tables.
#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    /// The on-disk database cannot be used (I/O, permissions, corruption).
    Unusable(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "database error: {e}"),
            StoreError::Unusable(m) => write!(f, "database unusable: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

pub(crate) const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS history (
        id          INTEGER PRIMARY KEY,
        url         TEXT    NOT NULL UNIQUE,
        title       TEXT    NOT NULL DEFAULT '',
        visited_at  INTEGER NOT NULL,
        visit_count INTEGER NOT NULL DEFAULT 1,
        typed_count INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_history_visited ON history(visited_at DESC);
    CREATE INDEX IF NOT EXISTS idx_history_url_prefix ON history(url COLLATE NOCASE);
";

pub(crate) fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        visited_at: row.get(3)?,
        visit_count: row.get(4)?,
        typed_count: row.get(5)?,
    })
}

const SELECT_COLS: &str = "SELECT id, url, title, visited_at, visit_count, typed_count FROM history";

/// Record a visit and evict beyond `cap`. `typed` marks an omnibox-initiated
/// navigation, which outranks a followed link in `score()`.
pub(crate) fn record_visit(
    conn: &mut Connection,
    url: &str,
    title: &str,
    at: i64,
    typed: bool,
    cap: i64,
) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO history (url, title, visited_at, visit_count, typed_count)
         VALUES (?1, ?2, ?3, 1, ?4)
         ON CONFLICT(url) DO UPDATE SET
            visited_at  = excluded.visited_at,
            visit_count = history.visit_count + 1,
            typed_count = history.typed_count + excluded.typed_count,
            title       = CASE WHEN excluded.title <> '' THEN excluded.title ELSE history.title END",
        params![url, title, at, i32::from(typed)],
    )?;
    tx.execute(
        "DELETE FROM history WHERE id IN (
             SELECT id FROM history ORDER BY visited_at DESC LIMIT -1 OFFSET ?1
         )",
        params![cap],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn update_title(conn: &Connection, url: &str, title: &str) -> Result<(), StoreError> {
    if title.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE history SET title = ?1 WHERE url = ?2",
        params![title, url],
    )?;
    Ok(())
}

pub(crate) fn count(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))?)
}

/// Most recent `limit` rows, for the empty-state dropdown.
pub(crate) fn recent(conn: &Connection, limit: usize) -> Result<Vec<HistoryEntry>, StoreError> {
    let sql = format!("{SELECT_COLS} ORDER BY visited_at DESC LIMIT ?1");
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![limit as i64], row_to_entry)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub(crate) fn autocomplete(conn: &Connection, text: &str, limit: usize) -> Result<Vec<HistoryEntry>, StoreError> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) || text.starts_with(['>', '?', '/']) {
        return Ok(Vec::new());
    }
    let prefixes = if text.contains("://") {
        vec![text.to_string()]
    } else {
        vec![format!("https://{text}"), format!("https://www.{text}"),
            format!("http://{text}"), format!("http://www.{text}")]
    };
    let sql = format!("{SELECT_COLS} WHERE url LIKE ?1 ESCAPE '\\'
        ORDER BY typed_count DESC, visit_count DESC, visited_at DESC LIMIT ?2");
    let mut statement = conn.prepare_cached(&sql)?;
    let mut rows = Vec::new();
    for prefix in prefixes {
        rows.extend(statement.query_map(params![format!("{}%", escape_like(&prefix)), limit as i64], row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?);
    }
    rows.sort_by_key(|row| std::cmp::Reverse((row.typed_count, row.visit_count, row.visited_at)));
    rows.dedup_by(|a, b| a.url == b.url);
    rows.truncate(limit);
    Ok(rows)
}

/// Ranked search. The SQL prefilter returns at most 4× `limit` candidates
/// before ranking in Rust — enough for good ordering without scanning the table.
pub(crate) fn search(
    conn: &Connection,
    query: &HistoryQuery,
    limit: usize,
    now: i64,
) -> Result<Vec<HistoryEntry>, StoreError> {
    let pool: usize = (limit * 4).max(64);
    let mut candidates = candidates(conn, query, pool)?;
    candidates.retain(|e| admits(e, query));
    let mut scored: Vec<(f64, HistoryEntry)> = candidates
        .into_iter()
        .filter_map(|e| {
            let s = if query.terms.is_empty() {
                1.0
            } else {
                score(&e, &query.terms, now)
            };
            (s > 0.0).then_some((s, e))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.1.visited_at.cmp(&a.1.visited_at))
    });
    Ok(scored.into_iter().take(limit).map(|(_, e)| e).collect())
}

fn candidates(
    conn: &Connection,
    query: &HistoryQuery,
    pool: usize,
) -> Result<Vec<HistoryEntry>, StoreError> {
    let mut sql = String::from(SELECT_COLS);
    sql.push_str(" WHERE 1=1");
    let mut binds: Vec<String> = Vec::new();
    for term in &query.terms {
        let i = binds.len() + 1;
        sql.push_str(&format!(
            " AND (lower(title) LIKE ?{i} ESCAPE '\\' OR lower(url) LIKE ?{} ESCAPE '\\')",
            i + 1
        ));
        let pat = format!("%{}%", escape_like(term));
        binds.push(pat.clone());
        binds.push(pat);
    }
    sql.push_str(" ORDER BY visited_at DESC LIMIT ");
    sql.push_str(&pool.to_string());

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::ToSql> =
        binds.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(params_ref.as_slice(), row_to_entry)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Delete entries by id. Returns rows removed.
pub(crate) fn forget_ids(conn: &Connection, ids: &[i64]) -> Result<usize, StoreError> {
    let tx = conn.unchecked_transaction()?;
    let mut n = 0;
    {
        let mut stmt = tx.prepare_cached("DELETE FROM history WHERE id = ?1")?;
        for id in ids {
            n += stmt.execute(params![id])?;
        }
    }
    tx.commit()?;
    Ok(n)
}

/// Forget every visit to `site` and its subdomains. Returns rows removed.
pub(crate) fn forget_site(conn: &Connection, site: &str) -> Result<usize, StoreError> {
    let site_l = site.to_ascii_lowercase();
    // LIKE narrows the scan; the host check in Rust makes it exact, because
    // "host ends with .site" cannot be expressed against the raw URL.
    //
    // No `//` anchor: a stored URL may be schemeless (`example.com/x`), and
    // anchoring would hide it from this scan while `host_of` still resolves a
    // host for it.
    let mut stmt = conn.prepare(
        "SELECT id, url FROM history WHERE lower(url) LIKE ?1 ESCAPE '\\'",
    )?;
    let pat = format!("%{}%", escape_like(&site_l));
    let rows = stmt.query_map(params![pat], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut ids = Vec::new();
    for row in rows {
        let (id, url) = row?;
        let host = host_of(&url).to_ascii_lowercase();
        if host == site_l || host.ends_with(&format!(".{site_l}")) {
            ids.push(id);
        }
    }
    drop(stmt);
    forget_ids(conn, &ids)
}

pub(crate) fn clear_range(conn: &Connection, from: i64, to: i64) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM history WHERE visited_at >= ?1 AND visited_at < ?2",
        params![from, to],
    )?)
}

pub(crate) fn find_by_url(conn: &Connection, url: &str) -> Result<Option<HistoryEntry>, StoreError> {
    let sql = format!("{SELECT_COLS} WHERE url = ?1");
    Ok(conn.query_row(&sql, params![url], row_to_entry).optional()?)
}

/// Escape LIKE wildcards for a query that carries `ESCAPE '\'`.
///
/// SQLite LIKE has no bracket character classes
/// ([reference](https://www.sqlite.org/lang_expr.html)), so `%`, `_` and the
/// escape character itself are escaped explicitly.
pub(crate) fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// Same escaping, exposed for the bookmark store's search.
pub(crate) fn escape_like_for_query(s: &str) -> String {
    escape_like(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;

    fn store() -> Store {
        Store::open_in_memory(100_000).expect("in-memory store")
    }

    fn seed(s: &mut Store) {
        s.record_visit("https://rust-lang.org/", "Rust Programming Language", NOW - 3 * DAY, true)
            .unwrap();
        s.record_visit("https://docs.rs/serde", "serde - Rust", NOW - DAY, false)
            .unwrap();
        s.record_visit("https://example.com/rust", "Random rust article", NOW - 30 * DAY, false)
            .unwrap();
        s.record_visit("https://news.ycombinator.com/", "Hacker News", NOW - 2 * DAY, false)
            .unwrap();
    }

    #[test]
    fn ranking_prefers_titles_then_recency_then_frequency() {
        let mut s = store();
        seed(&mut s);
        let hits = s.search_history(&parse_query("rust"), 10, NOW).unwrap();
        assert_eq!(hits.len(), 3, "all three rust rows must survive the term filter");
        assert_eq!(hits[0].url, "https://rust-lang.org/");
        assert_eq!(hits[1].url, "https://docs.rs/serde");
        assert_eq!(hits[2].url, "https://example.com/rust");
    }

    #[test]
    fn recency_dominates_at_equal_quality_and_frequency() {
        let a = HistoryEntry {
            id: 1,
            url: "https://a.com/x".into(),
            title: "same title".into(),
            visited_at: NOW - DAY,
            visit_count: 1,
            typed_count: 0,
        };
        let b = HistoryEntry {
            visited_at: NOW - 60 * DAY,
            ..a.clone()
        };
        let terms = vec!["same".to_string()];
        assert!(score(&a, &terms, NOW) > score(&b, &terms, NOW) * 50.0);
    }

    #[test]
    fn unmatched_term_rejects_the_entry() {
        let mut s = store();
        seed(&mut s);
        assert_eq!(s.search_history(&parse_query("rust zzz"), 10, NOW).unwrap().len(), 0);
        assert_eq!(s.search_history(&parse_query("zzz"), 10, NOW).unwrap().len(), 0);
    }

    #[test]
    fn frequency_is_log_scaled_not_linear() {
        let base = HistoryEntry {
            id: 1,
            url: "https://a.com/".into(),
            title: "q".into(),
            visited_at: NOW,
            visit_count: 1,
            typed_count: 0,
        };
        let terms = vec!["q".to_string()];
        let one = score(&base, &terms, NOW);
        let many = score(&HistoryEntry { visit_count: 4096, ..base }, &terms, NOW);
        let ratio = many / one;
        assert!((ratio - 12.0).abs() < 0.05, "log2(4097) ≈ 12, got {ratio}");
    }

    #[test]
    fn site_token_matches_subdomains_but_not_lookalikes() {
        let mut s = store();
        seed(&mut s);
        s.record_visit("https://evil-rust-lang.org/", "x", NOW, false).unwrap();
        let hits = s.search_history(&parse_query("site:rust-lang.org"), 10, NOW).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://rust-lang.org/");
    }

    #[test]
    fn date_and_type_tokens_filter() {
        let mut s = store();
        seed(&mut s);
        let after = s.search_history(&parse_query("rust after:1700000000"), 10, NOW).unwrap();
        assert_eq!(after.len(), 3);
        let typed = s.search_history(&parse_query("type:typed"), 10, NOW).unwrap();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].url, "https://rust-lang.org/");
        let visited = s.search_history(&parse_query("type:visited"), 10, NOW).unwrap();
        assert_eq!(visited.len(), 3);
    }

    #[test]
    fn parse_query_keeps_unknown_colon_tokens_as_text() {
        let q = parse_query("https://x.com site:y.com stray:token");
        assert_eq!(q.site.as_deref(), Some("y.com"));
        assert_eq!(q.terms, vec!["https://x.com", "stray:token"]);
    }

    #[test]
    fn like_wildcards_in_terms_do_not_widen_the_match() {
        let mut s = store();
        s.record_visit("https://a.com/100%", "100% real", NOW, false).unwrap();
        s.record_visit("https://b.com/100x", "100x fake", NOW + 1, false).unwrap();
        let hits = s.search_history(&parse_query("100%"), 10, NOW).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://a.com/100%");
    }

    #[test]
    fn underscore_is_literal_too() {
        let mut s = store();
        s.record_visit("https://a.com/a_b", "x", NOW, false).unwrap();
        s.record_visit("https://b.com/axb", "x", NOW + 1, false).unwrap();
        let hits = s.search_history(&parse_query("a_b"), 10, NOW).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://a.com/a_b");
    }

    #[test]
    fn fifo_eviction_respects_cap() {
        let mut s = Store::open_in_memory(3).unwrap();
        for i in 0..5 {
            s.record_visit(&format!("https://a.com/{i}"), "", NOW + i, false)
                .unwrap();
        }
        assert_eq!(s.history_count().unwrap(), 3);
        let recent = s.recent(10).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].url, "https://a.com/4");
        assert_eq!(recent[2].url, "https://a.com/2");
    }

    #[test]
    fn visit_aggregates_by_url_and_keeps_title() {
        let mut s = store();
        s.record_visit("https://a.com/", "", NOW, false).unwrap();
        s.record_visit("https://a.com/", "Title A", NOW + 1, true).unwrap();
        let e = s.entry_by_url("https://a.com/").unwrap().unwrap();
        assert_eq!(e.visit_count, 2);
        assert_eq!(e.typed_count, 1);
        assert_eq!(e.title, "Title A");
    }

    #[test]
    fn forget_site_removes_subdomains_only() {
        let mut s = store();
        s.record_visit("https://a.example.com/", "", NOW, false).unwrap();
        s.record_visit("https://example.com/", "", NOW, false).unwrap();
        s.record_visit("https://notexample.com/", "", NOW, false).unwrap();
        assert_eq!(s.forget_site("example.com").unwrap(), 2);
        assert_eq!(s.history_count().unwrap(), 1);
        assert_eq!(s.recent(5).unwrap()[0].url, "https://notexample.com/");
    }

    #[test]
    fn forget_site_covers_schemeless_urls() {
        // The LIKE prefilter used to require `//` before the site, so a URL
        // stored without a scheme was never even considered for deletion.
        let mut s = store();
        s.record_visit("example.com/page", "", NOW, false).unwrap();
        s.record_visit("https://example.com/", "", NOW, false).unwrap();
        assert_eq!(s.forget_site("example.com").unwrap(), 2);
        assert_eq!(s.history_count().unwrap(), 0);
    }

    #[test]
    fn parse_day_handles_dates_and_epochs() {
        assert_eq!(parse_day("1970-01-02"), Some(DAY));
        assert_eq!(parse_day("1700000000"), Some(1_700_000_000));
        assert_eq!(parse_day("2024-13-01"), None);
        assert_eq!(parse_day("nonsense"), None);
    }

    #[test]
    fn host_of_strips_scheme_port_and_userinfo() {
        assert_eq!(host_of("https://u:p@Example.COM:8443/x?y"), "Example.COM");
        assert_eq!(host_of("http://127.0.0.1:3000"), "127.0.0.1");
        assert_eq!(host_of("localhost:8080/a"), "localhost");
    }

    #[test]
    fn host_of_keeps_ipv6_brackets() {
        // `rsplit_once(':')` used to turn `[::1]:8080` into `[::1]` only by
        // accident and `[::1]` into `[:`, so the site filter never matched.
        assert_eq!(host_of("http://[::1]:8080/"), "[::1]");
        assert_eq!(host_of("http://[::1]/"), "[::1]");
        assert_eq!(host_of("http://[2001:db8::1]:443/x"), "[2001:db8::1]");
        assert_eq!(host_of("http://u:p@[::1]:8080/x"), "[::1]");
    }

    #[test]
    fn host_of_does_not_treat_a_path_colon_as_a_port() {
        // The authority ends at the first `/`; a `:443` in the path is not a
        // port and must not be stripped from the host.
        assert_eq!(host_of("https://example.com/a:443"), "example.com");
        assert_eq!(host_of("https://example.com"), "example.com");
    }
}