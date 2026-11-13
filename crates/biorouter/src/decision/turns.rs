//! Which agent turn each chat is in, so every decision a turn causes (a monitor
//! evaluation, a `decide` tool batch, a phase gate) is logged with the same
//! `turn_id` and surfaces can aggregate per turn instead of listing every call.
//!
//! The agent loop calls [`begin_turn`] when a reply starts and [`end_turn`] when
//! it ends; tools and gates read [`current_turn`]. Process-local, like the other
//! turn registries: a CLI or TUI process and the daemon each track their own.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use indexmap::IndexMap;

static CURRENT: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Turns holding late cost at once. A turn whose reply never drained it is
/// dropped oldest first once there are more, as the monitor's per-turn tally
/// is.
const MAX_LATE_COST_TURNS: usize = 512;

/// Dollars that priced decision requests answered too late (or after their
/// caller went away) cost each live turn, not yet drained into its reply
/// budget. Kept apart from the monitor's spend so the service never waits on
/// the monitor engine; the reply loop drains both.
static LATE_COST: LazyLock<Mutex<IndexMap<String, f64>>> =
    LazyLock::new(|| Mutex::new(IndexMap::new()));

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

/// Add `cost` dollars that a late decision answer cost to `turn_id`, but only
/// while that turn is still `session_id`'s live turn: a turn that has ended
/// has no reply left to charge, and charging the next one would bill it for
/// work it never asked for.
pub fn note_late_cost(session_id: &str, turn_id: &str, cost: f64) {
    if !cost.is_finite() || cost <= 0.0 {
        return;
    }
    if current_turn(session_id).as_deref() != Some(turn_id) {
        return;
    }
    let mut late = LATE_COST.lock().unwrap_or_else(PoisonError::into_inner);
    if !late.contains_key(turn_id) && late.len() >= MAX_LATE_COST_TURNS {
        late.shift_remove_index(0);
    }
    *late.entry(turn_id.to_string()).or_default() += cost;
}

/// Take the late cost [`note_late_cost`] added to `turn_id` since the last
/// take, leaving none behind.
pub fn take_late_cost(turn_id: &str) -> f64 {
    LATE_COST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .shift_remove(turn_id)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_cost_counts_against_the_live_turn_only_and_is_taken_once() {
        begin_turn("turns-late-a", "late-t1");
        note_late_cost("turns-late-a", "late-t1", 0.25);
        note_late_cost("turns-late-a", "late-t1", 0.5);
        // Another chat's turn, an ended turn and a non-cost add nothing.
        note_late_cost("turns-late-other", "late-t1", 1.0);
        note_late_cost("turns-late-a", "late-t0", 1.0);
        note_late_cost("turns-late-a", "late-t1", f64::NAN);
        note_late_cost("turns-late-a", "late-t1", -1.0);
        assert_eq!(take_late_cost("late-t1"), 0.75);
        assert_eq!(take_late_cost("late-t1"), 0.0, "taken once");
        assert_eq!(take_late_cost("late-t0"), 0.0);
        end_turn("turns-late-a", "late-t1");
        note_late_cost("turns-late-a", "late-t1", 1.0);
        assert_eq!(take_late_cost("late-t1"), 0.0, "the turn has ended");
    }

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
