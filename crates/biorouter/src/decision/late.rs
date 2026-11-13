//! Decision requests still running after their caller stopped waiting.
//!
//! A decision model is asked on a task of its own (`DecisionService`'s
//! `decide_one`), so the request finishes, and the tokens it was billed for are
//! counted, even when the caller's deadline passed or the caller went away (a
//! stopped turn, a dropped HTTP request). Each request holds an
//! [`InFlightGuard`] from the moment it is sent until its answer is stored,
//! on time or late, so a process about to exit can wait for them with
//! [`settle_late_answers`] instead of losing the count.
//!
//! The count is process-wide, like the decision service itself.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// Decision requests whose task has not finished.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Woken whenever one of them finishes.
static SETTLED: Notify = Notify::const_new();

/// One decision request in flight. Dropping it (when the task ends, returns or
/// panics) takes it off the count, so a panic cannot leak it.
#[must_use = "the request counts as in flight only while the guard is held"]
#[derive(Debug)]
pub struct InFlightGuard {
    _private: (),
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        SETTLED.notify_waiters();
    }
}

/// Count one decision request as in flight until the guard is dropped.
pub fn enter_in_flight() -> InFlightGuard {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    InFlightGuard { _private: () }
}

/// Decision requests in flight now, in this process.
pub fn in_flight() -> usize {
    IN_FLIGHT.load(Ordering::SeqCst)
}

/// Wait until no decision request is in flight, or `max` has passed, and
/// return how many are still in flight (0 when every one settled). A process
/// calls this before it exits, so an answer that arrived after its caller
/// stopped waiting is still stored and counted in Usage.
pub async fn settle_late_answers(max: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + max;
    loop {
        // Registered before the count is read: `notify_waiters` wakes a
        // `Notified` from the moment it is created, so a request that ends
        // between the read and the wait is not missed.
        let settled = SETTLED.notified();
        let remaining = in_flight();
        if remaining == 0 {
            return 0;
        }
        if tokio::time::timeout_at(deadline, settled).await.is_err() {
            return in_flight();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guard counts while held and is released however its holder ends,
    /// a panic included.
    #[tokio::test]
    async fn a_guard_is_released_when_its_task_panics() {
        let entered = std::sync::Arc::new(Notify::new());
        let release = std::sync::Arc::new(Notify::new());
        let (entered_task, release_task) = (entered.clone(), release.clone());
        let task = tokio::spawn(async move {
            let _guard = enter_in_flight();
            entered_task.notify_one();
            release_task.notified().await;
            panic!("the decision task stopped");
        });
        entered.notified().await;
        assert!(in_flight() >= 1);
        release.notify_one();
        assert!(task.await.unwrap_err().is_panic());
        // Other tests in this process may hold guards of their own, so wait
        // for the count rather than read it once.
        assert_eq!(settle_late_answers(Duration::from_secs(10)).await, 0);
    }
}
