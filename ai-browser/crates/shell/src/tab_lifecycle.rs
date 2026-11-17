use std::time::Duration;

pub const WARM_BACKGROUND_TABS: usize = 3;

pub struct Candidate {
    pub id: u64,
    pub idle: Duration,
    pub protected: bool,
}

pub fn eviction_candidates(mut tabs: Vec<Candidate>, timeout: Duration) -> Vec<u64> {
    let overflow = tabs.len().saturating_sub(WARM_BACKGROUND_TABS);
    tabs.sort_by_key(|tab| std::cmp::Reverse(tab.idle));
    let mut evict = Vec::new();
    for tab in tabs {
        if !tab.protected && (tab.idle >= timeout || evict.len() < overflow) {
            evict.push(tab.id);
        }
    }
    evict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u64, idle: u64, protected: bool) -> Candidate {
        Candidate { id, idle: Duration::from_secs(idle), protected }
    }

    #[test]
    fn keeps_recent_tabs_and_discards_oldest_under_pressure() {
        let timeout = Duration::from_secs(300);
        assert!(eviction_candidates(vec![tab(1, 5, false)], timeout).is_empty());
        assert_eq!(eviction_candidates(vec![
            tab(1, 1, false), tab(2, 20, false), tab(3, 10, false), tab(4, 5, false),
        ], timeout), vec![2]);
    }

    #[test]
    fn expires_idle_tabs_but_never_discards_protected_work() {
        assert_eq!(eviction_candidates(vec![tab(1, 600, true), tab(2, 600, false)],
            Duration::from_secs(300)), vec![2]);
    }
}
