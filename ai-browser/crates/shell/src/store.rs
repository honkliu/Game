//! History and bookmarks share one SQLite database. Omnibox searches use a
//! read-only worker connection so history size cannot block native input.

use rusqlite::{params, Connection};
use std::path::Path;

use crate::bookmarks::Bookmark;
use crate::history::{HistoryEntry, HistoryQuery, StoreError};

pub struct Store {
    conn: Connection,
    cap: i64,
}

impl Store {
    pub fn open_read_only(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_millis(100))?;
        Ok(Self { conn, cap: 0 })
    }

    pub fn autocomplete(&self, text: &str, limit: usize) -> Result<Vec<HistoryEntry>, StoreError> {
        crate::history::autocomplete(&self.conn, text, limit)
    }
    /// Open (creating if needed) the database at `path`. `cap` bounds the
    /// history row count (§4.5).
    pub fn open(path: &Path, cap: u32) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StoreError::Unusable(format!("create data dir: {e}")))?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, cap)
    }

    /// In-memory store for tests.
    pub fn open_in_memory(cap: u32) -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?, cap)
    }

    fn init(conn: Connection, cap: u32) -> Result<Self, StoreError> {
        conn.busy_timeout(std::time::Duration::from_millis(2_000))?;
        conn.execute_batch(crate::history::SCHEMA)?;
        conn.execute_batch(crate::bookmarks::SCHEMA)?;
        Ok(Self {
            conn,
            cap: i64::from(cap.max(1)),
        })
    }

    // ---- history (delegated) -------------------------------------------

    pub fn record_visit(
        &mut self,
        url: &str,
        title: &str,
        at: i64,
        typed: bool,
    ) -> Result<(), StoreError> {
        crate::history::record_visit(&mut self.conn, url, title, at, typed, self.cap)
    }

    pub fn update_title(&self, url: &str, title: &str) -> Result<(), StoreError> {
        crate::history::update_title(&self.conn, url, title)
    }

    pub fn history_count(&self) -> Result<i64, StoreError> {
        crate::history::count(&self.conn)
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, StoreError> {
        crate::history::recent(&self.conn, limit)
    }

    pub fn search_history(
        &self,
        query: &HistoryQuery,
        limit: usize,
        now: i64,
    ) -> Result<Vec<HistoryEntry>, StoreError> {
        crate::history::search(&self.conn, query, limit, now)
    }

    pub fn forget_ids(&self, ids: &[i64]) -> Result<usize, StoreError> {
        crate::history::forget_ids(&self.conn, ids)
    }

    pub fn forget_site(&self, site: &str) -> Result<usize, StoreError> {
        crate::history::forget_site(&self.conn, site)
    }

    pub fn clear_history(&self) -> Result<usize, StoreError> {
        Ok(self.conn.execute("DELETE FROM history", [])?)
    }

    pub fn clear_history_range(&self, from: i64, to: i64) -> Result<usize, StoreError> {
        crate::history::clear_range(&self.conn, from, to)
    }

    pub fn entry_by_url(&self, url: &str) -> Result<Option<HistoryEntry>, StoreError> {
        crate::history::find_by_url(&self.conn, url)
    }

    // ---- bookmarks (delegated) -----------------------------------------

    pub fn bookmarks(&self) -> Result<Vec<Bookmark>, StoreError> {
        crate::bookmarks::list(&self.conn)
    }

    pub fn add_bookmark(&self, url: &str, title: &str, at: i64) -> Result<i64, StoreError> {
        crate::bookmarks::add(&self.conn, url, title, at)
    }

    pub fn toggle_bookmark(&self, url: &str, title: &str, at: i64) -> Result<bool, StoreError> {
        crate::bookmarks::toggle(&self.conn, url, title, at)
    }

    pub fn is_bookmarked(&self, url: &str) -> Result<Option<i64>, StoreError> {
        crate::bookmarks::id_for(&self.conn, url)
    }

    pub fn remove_bookmark(&self, id: i64) -> Result<(), StoreError> {
        crate::bookmarks::remove(&self.conn, id)
    }

    pub fn search_bookmarks(&self, q: &HistoryQuery) -> Result<Vec<Bookmark>, StoreError> {
        crate::bookmarks::search(&self.conn, q)
    }

    /// Forget everything on exit when the Privacy toggles say so.
    pub fn clear_all_browsing_rows(&self) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    /// Used by tests and the settings panel.
    pub fn vacuum_after_cap_change(&self, new_cap: u32) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM history WHERE id IN (
                 SELECT id FROM history ORDER BY visited_at DESC LIMIT -1 OFFSET ?1
             )",
            params![i64::from(new_cap.max(1))],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_and_bookmarks_share_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.db");
        {
            let mut s = Store::open(&path, 100).unwrap();
            s.record_visit("https://a.com/", "A", 100, true).unwrap();
            s.add_bookmark("https://a.com/", "A", 100).unwrap();
        }
        let s = Store::open(&path, 100).unwrap();
        assert_eq!(s.history_count().unwrap(), 1);
        assert_eq!(s.bookmarks().unwrap().len(), 1);
    }

    #[test]
    fn on_disk_search_meets_w4_gate_over_50k_rows() {
        // W4's gate is p95 ≤ 15 ms for the DB query over 50k entries. This is
        // one sample, not a p95 — the bench harness owns the real number; this
        // test fails loudly if the query regresses by an order of magnitude.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(&dir.path().join("bench.db"), 100_000).unwrap();
        {
            // Seed 50k rows in one transaction: the W4 corpus, not a prompt for
            // insert performance.
            let tx = s.conn.transaction().unwrap();
            {
                let mut stmt = tx
                    .prepare("INSERT INTO history (url, title, visited_at, visit_count, typed_count) VALUES (?1,?2,?3,1,0)")
                    .unwrap();
                for i in 0..50_000u32 {
                    let url = format!("https://site{}/page/{}", i % 500, i);
                    let title = format!("Document number {i} about topic {}", i % 37);
                    stmt.execute(params![url, title, 1_700_000_000i64 + i as i64])
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        }

        let q = crate::history::parse_query("topic 5");
        let t = std::time::Instant::now();
        let hits = s.search_history(&q, 8, 1_800_000_000).unwrap();
        let elapsed = t.elapsed();
        assert!(!hits.is_empty());
        assert!(
            elapsed.as_millis() < 15,
            "W4 gate: history query took {elapsed:?} over 50k rows"
        );
    }
}