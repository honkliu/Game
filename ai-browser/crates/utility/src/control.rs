//! Cooperative stop for one task: user stop, deadline, staleness, shutdown.
//!
//! One shared object instead of a bare flag because the *reason* matters: the
//! shell must be told "stopped" for the first, "timed out" for the second and
//! "stale" for the third, and the first reason to arrive wins.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use ai_platform_contract::contract::CancelReason;
use ai_platform_contract::failure::Failure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    User(CancelReason),
    /// The document the task was bound to is gone or replaced. Carries a stable code.
    Stale(&'static str),
    Deadline,
    Shutdown,
}

impl StopReason {
    pub fn failure(&self) -> Failure {
        match self {
            StopReason::User(CancelReason::UserStopped) => Failure::cancelled("task.cancelled_by_user"),
            StopReason::User(CancelReason::Superseded) => Failure::cancelled("task.superseded"),
            StopReason::User(CancelReason::DeadlineElapsed) | StopReason::Deadline => {
                Failure::timed_out("task.deadline_elapsed")
            }
            StopReason::User(CancelReason::PolicyRevoked) => Failure::blocked("task.policy_revoked"),
            StopReason::Stale(code) => Failure::stale(code),
            StopReason::Shutdown => Failure::cancelled("utility.shutdown"),
        }
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, StopReason::Stale(_))
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug)]
pub struct TaskControl {
    flag: AtomicBool,
    reason: Mutex<Option<StopReason>>,
    /// Wall clock ms since the epoch; 0 means no deadline.
    deadline_ms: i64,
    actions_completed: AtomicU32,
    action_in_flight: AtomicBool,
}

impl TaskControl {
    pub fn new(deadline_ms: i64) -> TaskControl {
        TaskControl {
            flag: AtomicBool::new(false),
            reason: Mutex::new(None),
            deadline_ms,
            actions_completed: AtomicU32::new(0),
            action_in_flight: AtomicBool::new(false),
        }
    }

    /// Records the reason if none is recorded yet. Returns whether this call won.
    pub fn stop(&self, reason: StopReason) -> bool {
        let mut slot = lock(&self.reason);
        let won = slot.is_none();
        if won {
            *slot = Some(reason);
        }
        self.flag.store(true, Ordering::SeqCst);
        won
    }

    pub fn is_stopped(&self) -> bool {
        if self.flag.load(Ordering::SeqCst) {
            return true;
        }
        if self.deadline_ms != 0 && now_ms() >= self.deadline_ms {
            self.stop(StopReason::Deadline);
            return true;
        }
        false
    }

    pub fn reason(&self) -> Option<StopReason> {
        self.is_stopped();
        lock(&self.reason).clone()
    }

    /// The failure to finish with, when stopped.
    pub fn failure(&self) -> Option<Failure> {
        self.reason().map(|reason| reason.failure())
    }

    pub fn deadline_ms(&self) -> i64 {
        self.deadline_ms
    }

    pub fn action_started(&self) {
        self.action_in_flight.store(true, Ordering::SeqCst);
    }

    pub fn action_finished(&self, completed: bool) {
        self.action_in_flight.store(false, Ordering::SeqCst);
        if completed {
            self.actions_completed.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn action_in_flight(&self) -> bool {
        self.action_in_flight.load(Ordering::SeqCst)
    }

    pub fn actions_completed(&self) -> u32 {
        self.actions_completed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn first_reason_wins() {
        let control = TaskControl::new(0);
        assert!(!control.is_stopped());
        assert!(control.stop(StopReason::Stale("task.document_replaced")));
        assert!(!control.stop(StopReason::User(CancelReason::UserStopped)));
        assert!(control.reason().is_some_and(|r| r.is_stale()));
    }

    #[test]
    fn unset_deadline_never_expires_but_a_past_one_does() {
        assert!(!TaskControl::new(0).is_stopped());
        let control = TaskControl::new(1);
        assert!(control.is_stopped());
        assert_eq!(control.failure().map(|f| f.code), Some("task.deadline_elapsed".to_string()));
    }
}
