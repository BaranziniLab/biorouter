//! Which agent turn each chat is in, so every decision a turn causes (a monitor
//! evaluation, a `decide` tool batch, a phase gate) is logged with the same
//! `turn_id` and surfaces can aggregate per turn instead of listing every call.
//!
//! The agent loop calls [`begin_turn`] when a reply starts and [`end_turn`] when
//! it ends; tools and gates read [`current_turn`]. Process-local, like the other
//! turn registries: a CLI or TUI process and the daemon each track their own.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

static CURRENT: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record that `session_id` is now in turn `turn_id` (replacing any earlier one).
pub fn begin_turn(session_id: &str, turn_id: &str) {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session_id.to_string(), turn_id.to_string());
}

/// Forget `session_id`'s turn, but only if it is still `turn_id` (a newer turn
/// that began meanwhile is left alone).
pub fn end_turn(session_id: &str, turn_id: &str) {
    let mut map = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    if map
        .get(session_id)
        .is_some_and(|current| current == turn_id)
    {
        map.remove(session_id);
    }
}

/// The turn `session_id` is in now, if a reply is running.
pub fn current_turn(session_id: &str) -> Option<String> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session_id)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_tracked_per_session_and_only_its_own_end_clears_it() {
        begin_turn("turns-test-a", "t1");
        assert_eq!(current_turn("turns-test-a").as_deref(), Some("t1"));
        begin_turn("turns-test-a", "t2");
        end_turn("turns-test-a", "t1");
        assert_eq!(current_turn("turns-test-a").as_deref(), Some("t2"));
        end_turn("turns-test-a", "t2");
        assert_eq!(current_turn("turns-test-a"), None);
        assert_eq!(current_turn("turns-test-other"), None);
    }
}
