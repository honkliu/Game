//! `session.json` — window rect, rail state, ordered tabs with scroll offsets,
//! groups, active tab, the closed-tab stack, and the downloads list (§6.4).
//!
//! Writes are debounced by the caller (`app.rs`) and go through an atomic
//! replace, so a forced kill at any moment leaves either the old or the new
//! file, never a torn one.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::downloads::Download;
use crate::tabs::{Group, GroupColor};

/// Bump when the shape changes incompatibly; a mismatched file is ignored
/// (fresh session) rather than half-restored.
pub const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowRect {
    fn default() -> Self {
        Self {
            x: 80,
            y: 60,
            width: 1280,
            height: 820,
            maximized: false,
        }
    }
}

/// One tab as restored. `scroll_y` is applied after `NavigationCompleted`;
/// if the injection fails the host logs it and leaves the field as-is (§6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabRecord {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub scroll_y: f64,
    pub group_id: Option<u64>,
    pub pinned: bool,
    pub zoom: f64,
    pub last_active_at: i64,
    /// A canvas tab's grid; its pages are the records whose `member_of` is this id.
    pub canvas: Option<crate::tabs::Canvas>,
    pub member_of: Option<u64>,
}

impl Default for TabRecord {
    fn default() -> Self {
        Self {
            id: 0,
            url: crate::tabs::NEWTAB_URL.to_string(),
            title: String::new(),
            scroll_y: 0.0,
            group_id: None,
            pinned: false,
            zoom: 1.0,
            last_active_at: 0,
            canvas: None,
            member_of: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupRecord {
    pub id: u64,
    pub name: String,
    pub color: GroupColor,
    pub collapsed: bool,
}

impl Default for GroupRecord {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            color: GroupColor::Blue,
            collapsed: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClosedRecord {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group_id: Option<u64>,
    pub index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub version: u32,
    pub window: WindowRect,
    pub rail_collapsed: bool,
    pub theme_override: Option<String>,
    pub tabs: Vec<TabRecord>,
    pub groups: Vec<GroupRecord>,
    pub active_tab_id: Option<u64>,
    pub closed: Vec<ClosedRecord>,
    pub downloads: Vec<Download>,
}

/// The live state a session is captured from. One struct instead of seven
/// positional arguments keeps call sites readable and field order irrelevant.
pub struct Captured {
    pub window: WindowRect,
    pub rail_collapsed: bool,
    pub tabs: Vec<TabRecord>,
    pub groups: Vec<GroupRecord>,
    pub active_tab_id: Option<u64>,
    pub closed: Vec<ClosedRecord>,
    pub downloads: Vec<Download>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            window: WindowRect::default(),
            rail_collapsed: false,
            theme_override: None,
            tabs: Vec::new(),
            groups: Vec::new(),
            active_tab_id: None,
            closed: Vec::new(),
            downloads: Vec::new(),
        }
    }
}

impl Session {
    /// Trim to the invariants the restorer relies on: unique tab ids, group
    /// ids that exist, an active id that is present, ≤ 25 closed entries.
    /// Runs on load so a hand-edited or partially-written file cannot make the
    /// restorer pick a nonexistent active tab.
    pub fn normalize(&mut self) {
        let group_ids: std::collections::HashSet<u64> = self.groups.iter().map(|g| g.id).collect();
        let mut seen = std::collections::HashSet::new();
        self.tabs.retain(|t| seen.insert(t.id));
        for t in &mut self.tabs {
            if let Some(g) = t.group_id {
                if !group_ids.contains(&g) {
                    t.group_id = None;
                }
            }
            if !t.zoom.is_finite() || t.zoom <= 0.0 {
                t.zoom = 1.0;
            }
            if !t.scroll_y.is_finite() || t.scroll_y < 0.0 {
                t.scroll_y = 0.0;
            }
        }
        if self
            .active_tab_id
            .is_some_and(|id| !self.tabs.iter().any(|t| t.id == id))
        {
            self.active_tab_id = self.tabs.first().map(|t| t.id);
        }
        if self.closed.len() > crate::tabs::CLOSED_STACK_LEN {
            let excess = self.closed.len() - crate::tabs::CLOSED_STACK_LEN;
            self.closed.drain(..excess);
        }
        if self.window.width < 320 {
            self.window.width = 320;
        }
        if self.window.height < 240 {
            self.window.height = 240;
        }
    }

    /// Build the persisted form from live state, normalized.
    pub fn capture(c: Captured) -> Self {
        let mut s = Session {
            version: SESSION_VERSION,
            window: c.window,
            rail_collapsed: c.rail_collapsed,
            theme_override: None,
            tabs: c.tabs,
            groups: c.groups,
            active_tab_id: c.active_tab_id,
            closed: c.closed,
            downloads: c.downloads,
        };
        s.normalize();
        s
    }
}

/// Convert live tab state into a record. `scroll_y` lives in the page's JS, so
/// it is supplied by the caller; everything else comes from the tab itself.
pub fn tab_to_record(tab: &crate::tabs::Tab, scroll_y: f64, now: i64) -> TabRecord {
    TabRecord {
        id: tab.id.0,
        url: tab.url.clone(),
        title: tab.title.clone(),
        scroll_y,
        group_id: tab.group_id.map(|g| g.0),
        pinned: tab.pinned,
        zoom: tab.zoom,
        last_active_at: now,
        canvas: tab.canvas.clone(),
        member_of: tab.member_of.map(|c| c.0),
    }
}

pub fn group_to_record(g: &Group) -> GroupRecord {
    GroupRecord {
        id: g.id.0,
        name: g.name.clone(),
        color: g.color,
        collapsed: g.collapsed,
    }
}

/// Read the session file. Any problem (missing, corrupt, wrong version)
/// yields `None` plus a human-readable reason when there is something to say.
///
/// A `.bak` left behind by an interrupted `save` is preferred over starting
/// fresh, so a crash mid-save costs at most the newest state, never the tab
/// set.
pub fn load(path: &Path) -> (Option<Session>, Option<String>) {
    match load_one(path) {
        // A missing file is not an error, but it *is* the state an interrupted
        // save leaves behind (old file renamed away, new one not yet in place),
        // so a surviving `.bak` is still the best available session.
        Ok((None, _)) => match recover_backup(path) {
            Some((s, warn)) => (Some(s), Some(warn)),
            None => (None, None),
        },
        Err(warn) => match recover_backup(path) {
            Some((s, extra)) => (Some(s), Some(format!("{warn}; {extra}"))),
            None => (None, Some(warn)),
        },
        Ok(v) => v,
    }
}

fn recover_backup(path: &Path) -> Option<(Session, String)> {
    let bak = path.with_extension("json.bak");
    if !bak.is_file() {
        return None;
    }
    match load_one(&bak) {
        Ok((Some(s), _)) => Some((s, "recovered from session.json.bak".to_string())),
        _ => None,
    }
}

fn load_one(path: &Path) -> Result<(Option<Session>, Option<String>), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(e) => return Err(format!("session.json unreadable: {e}")),
    };
    match serde_json::from_str::<Session>(&text) {
        Ok(mut s) => {
            if s.version != SESSION_VERSION {
                return Err(format!(
                    "session.json version {} ignored (expected {SESSION_VERSION})",
                    s.version
                ));
            }
            s.normalize();
            Ok((Some(s), None))
        }
        Err(e) => Err(format!("session.json corrupt: {e}")),
    }
}

/// Atomic save. Returns the error instead of logging so the caller can rate-limit
/// repeated failures.
///
/// On Windows, `rename` over an existing file fails with
/// `ERROR_ALREADY_EXISTS`, so the destination must be removed first — but
/// removing first means a crash between the two calls leaves *no* session file
/// at all, which is exactly the torn-state the doc comment above promises not
/// to produce. Instead, keep the previous file under a `.bak` name and only
/// delete it once the new file is in place; `load` reads the main file first
/// and falls back to the backup. Worst case is one stale save, never none.
pub fn save(path: &Path, session: &Session) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bak = path.with_extension("json.bak");
    let json = serde_json::to_string(session)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, &json)?;

    // Move the previous state aside (best effort: there may be none).
    let _ = std::fs::remove_file(&bak);
    let _ = std::fs::rename(path, &bak);
    std::fs::rename(&tmp, path)?;
    let _ = std::fs::remove_file(&bak);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::DownloadState;

    fn rec(id: u64, url: &str, title: &str, scroll_y: f64) -> TabRecord {
        let mut t = crate::tabs::Tab::new(crate::tabs::TabId(id), url);
        t.title = title.into();
        tab_to_record(&t, scroll_y, 1_700_000_000)
    }

    fn sample() -> Session {
        let mut a = rec(1, "https://a.com/", "A", 120.5);
        a.pinned = true;
        a.zoom = 1.25;
        a.last_active_at = 1_700_000_000;
        let mut b = rec(2, "https://b.com/", "B", 0.0);
        b.group_id = Some(9);
        b.last_active_at = 1_700_000_100;
        Session::capture(crate::session::Captured {
            window: WindowRect {
                x: 10,
                y: 20,
                width: 1440,
                height: 900,
                maximized: true,
            },
            rail_collapsed: true,
            tabs: vec![a, b],
            groups: vec![GroupRecord {
                id: 9,
                name: "Work".into(),
                color: GroupColor::Amber,
                collapsed: false,
            }],
            active_tab_id: Some(2),
            closed: vec![ClosedRecord {
                url: "https://c.com/".into(),
                title: "C".into(),
                pinned: false,
                group_id: None,
                index: 0,
            }],
            downloads: vec![Download {
                id: 7,
                url: "https://x/y.zip".into(),
                file: "y.zip".into(),
                path: r"C:\dl\y.zip".into(),
                state: DownloadState::Completed,
                total_bytes: Some(10),
                received_bytes: Some(10),
            }],
        })
    }

    #[test]
    fn round_trip_preserves_order_scroll_and_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let s = sample();
        save(&path, &s).unwrap();
        let (loaded, warn) = load(&path);
        assert!(warn.is_none());
        let loaded = loaded.expect("session must load");
        assert_eq!(loaded, s);
        assert_eq!(loaded.tabs[0].scroll_y, 120.5);
        assert_eq!(loaded.tabs[1].group_id, Some(9));
        assert_eq!(loaded.active_tab_id, Some(2));
        assert!(loaded.window.maximized);
    }

    #[test]
    fn corrupt_or_dropped_active_tab_is_repaired() {
        let mut s = sample();
        s.active_tab_id = Some(999);
        s.tabs[1].group_id = Some(1234); // nonexistent group
        s.tabs[0].zoom = f64::NAN;
        s.normalize();
        assert_eq!(s.active_tab_id, Some(1));
        assert_eq!(s.tabs[1].group_id, None);
        assert_eq!(s.tabs[0].zoom, 1.0);
    }

    #[test]
    fn restores_zoom_outside_previous_app_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut s = sample();
        s.tabs[0].zoom = 0.1;
        s.tabs[1].zoom = 8.0;
        save(&path, &s).unwrap();
        let (loaded, warn) = load(&path);
        assert!(warn.is_none());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.tabs[0].zoom, 0.1);
        assert_eq!(loaded.tabs[1].zoom, 8.0);
    }

    #[test]
    fn closed_stack_is_capped_on_load() {
        let mut s = Session {
            closed: (0..40)
                .map(|i| ClosedRecord {
                    index: i,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.closed.len(), crate::tabs::CLOSED_STACK_LEN);
        // The *newest* entries survive.
        assert_eq!(s.closed.last().unwrap().index, 39);
    }

    #[test]
    fn version_mismatch_is_ignored_not_half_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, r#"{"version":999,"tabs":[{"id":1,"url":"https://a"}]}"#).unwrap();
        let (s, warn) = load(&path);
        assert!(s.is_none());
        assert!(warn.unwrap().contains("version 999"));
    }

    #[test]
    fn save_is_atomic_when_target_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        save(&path, &sample()).unwrap();
        let mut s2 = sample();
        s2.active_tab_id = Some(1);
        save(&path, &s2).unwrap();
        let (loaded, _) = load(&path);
        assert_eq!(loaded.unwrap().active_tab_id, Some(1));
        assert!(!dir.path().join("session.json.tmp").exists());
        assert!(!dir.path().join("session.json.bak").exists(), "backup is cleaned up");
    }

    #[test]
    fn load_recovers_a_session_stranded_in_the_backup() {
        // The state a crash between "move old aside" and "rename new in" leaves:
        // no session.json, one session.json.bak. Old behaviour started fresh.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        save(&path, &sample()).unwrap();
        std::fs::rename(&path, path.with_extension("json.bak")).unwrap();
        let (loaded, warn) = load(&path);
        let loaded = loaded.expect("session must be recovered from the backup");
        assert_eq!(loaded.tabs.len(), 2);
        assert!(warn.unwrap().contains("recovered"));
    }

    #[test]
    fn empty_tabs_clears_dangling_active_id() {
        let mut s = Session {
            active_tab_id: Some(5),
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.active_tab_id, None);
    }
}