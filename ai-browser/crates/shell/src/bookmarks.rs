//! Bookmarks.
//!
//! §9 keeps bookmarks out of a bookmarks bar: they surface only as new-tab
//! tiles (§4.1) and via `Ctrl+D`. The store is a flat, ordered list in the same
//! SQLite file as history — one file, one backup.

use rusqlite::{params, Connection, Row};
use serde::Serialize;

use crate::history::{escape_like_for_query, HistoryQuery, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
    /// Unix seconds.
    pub created_at: i64,
    /// 0-based tile position.
    pub position: i64,
}

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS bookmarks (
        id         INTEGER PRIMARY KEY,
        url        TEXT    NOT NULL UNIQUE,
        title      TEXT    NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        position   INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_bookmarks_position ON bookmarks(position);
";

fn row_to_bookmark(row: &Row<'_>) -> rusqlite::Result<Bookmark> {
    Ok(Bookmark {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        created_at: row.get(3)?,
        position: row.get(4)?,
    })
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<Bookmark>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, url, title, created_at, position FROM bookmarks ORDER BY position, id",
    )?;
    let rows = stmt.query_map([], row_to_bookmark)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

/// Add or update (by URL). Returns the bookmark's id.
pub(crate) fn add(conn: &Connection, url: &str, title: &str, at: i64) -> Result<i64, StoreError> {
    let next_pos: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM bookmarks",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO bookmarks (url, title, created_at, position)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(url) DO UPDATE SET
            title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE bookmarks.title END",
        params![url, title, at, next_pos],
    )?;
    Ok(conn.query_row("SELECT id FROM bookmarks WHERE url = ?1", params![url], |r| {
        r.get(0)
    })?)
}

pub(crate) fn id_for(conn: &Connection, url: &str) -> Result<Option<i64>, StoreError> {
    match conn.query_row(
        "SELECT id FROM bookmarks WHERE url = ?1",
        params![url],
        |r| r.get(0),
    ) {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Toggle: returns `true` if the URL is now bookmarked.
pub(crate) fn toggle(
    conn: &Connection,
    url: &str,
    title: &str,
    at: i64,
) -> Result<bool, StoreError> {
    if let Some(id) = id_for(conn, url)? {
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", params![id])?;
        return Ok(false);
    }
    add(conn, url, title, at)?;
    Ok(true)
}

pub(crate) fn remove(conn: &Connection, id: i64) -> Result<(), StoreError> {
    conn.execute("DELETE FROM bookmarks WHERE id = ?1", params![id])?;
    Ok(())
}

/// Case-insensitive substring match on title/url, in stored order. Shares the
/// LIKE-escaping rules with history so `%` in a URL is literal.
pub(crate) fn search(conn: &Connection, q: &HistoryQuery) -> Result<Vec<Bookmark>, StoreError> {
    let mut sql = String::from(
        "SELECT id, url, title, created_at, position FROM bookmarks WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();
    for term in &q.terms {
        let i = binds.len() + 1;
        sql.push_str(&format!(
            " AND (lower(title) LIKE ?{i} ESCAPE '\\' OR lower(url) LIKE ?{} ESCAPE '\\')",
            i + 1
        ));
        let pat = format!("%{}%", escape_like_for_query(term));
        binds.push(pat.clone());
        binds.push(pat);
    }
    sql.push_str(" ORDER BY position, id");
    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::ToSql> =
        binds.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(params_ref.as_slice(), row_to_bookmark)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory(1_000).unwrap()
    }

    #[test]
    fn toggle_adds_then_removes() {
        let s = store();
        assert!(s.toggle_bookmark("https://a.com/", "A", 1).unwrap());
        assert_eq!(s.bookmarks().unwrap().len(), 1);
        assert!(!s.toggle_bookmark("https://a.com/", "A", 2).unwrap());
        assert!(s.bookmarks().unwrap().is_empty());
    }

    #[test]
    fn add_is_idempotent_and_updates_title() {
        let s = store();
        let id1 = s.add_bookmark("https://a.com/", "", 1).unwrap();
        let id2 = s.add_bookmark("https://a.com/", "Better", 2).unwrap();
        assert_eq!(id1, id2);
        let list = s.bookmarks().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Better");
    }

    #[test]
    fn order_is_stable_and_search_escapes_wildcards() {
        let s = store();
        s.add_bookmark("https://b.com/", "B", 2).unwrap();
        s.add_bookmark("https://a.com/", "A", 1).unwrap();
        assert_eq!(
            s.bookmarks().unwrap().iter().map(|x| x.url.as_str()).collect::<Vec<_>>(),
            vec!["https://b.com/", "https://a.com/"]
        );
        s.add_bookmark("https://c.com/100%", "pct", 3).unwrap();
        let hits = s
            .search_bookmarks(&HistoryQuery {
                terms: vec!["100%".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://c.com/100%");
    }
}