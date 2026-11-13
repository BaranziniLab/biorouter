//! The **first-crossing disclosure**: the state
//! [`visibility::requires_first_crossing_approval`] always needed and never had.
//!
//! A private-capability caller writing into a public conversation is permitted
//! (R4 — a private session may spawn and drive public children, and a rule that
//! lets you spawn one but never prompt it grants nothing). The prompt text is
//! still private-origin content arriving at a public model, so the **first**
//! such write from a given caller into a given target shows the user the exact
//! payload and waits for an answer.
//!
//! ⚠ **This module exists because retiring the lineage clause made the
//! disclosure load-bearing.** While WRITE was `VIS ∧ L ∈ {self, child}`, the
//! public targets a private caller could reach were the ones it had itself
//! spawned, so the crossing was one the caller had already arranged. It can now
//! write into any public conversation on the machine — a chat the user opened,
//! is reading, and never connected to this agent. The predicate had been sitting
//! unwired with an "OPERATOR DECISION OUTSTANDING" note against it; widening the
//! target set is what decided it.
//!
//! # Why the ledger is here and not in the permission store
//!
//! The approval card carries a `prompt`, and both the desktop and the CLI
//! deliberately suppress "Always Allow" whenever one is present
//! (`ToolCallConfirmation.tsx`, `session/mod.rs`'s `prompt_tool_confirmation`).
//! So the permission store can never learn this grant, and "first" has to be
//! remembered somewhere else. It is remembered per **(caller, target) pair**,
//! exactly as the predicate's own doc specifies.
//!
//! # Two properties that are easy to get backwards
//!
//! * **The crossing is recorded when the write LANDS, never when it is asked
//!   about.** [`needs_disclosure`] is a question; [`record`] is the answer's
//!   consequence. A refusal — the user denying the card, or the tier gate
//!   refusing underneath it — records nothing, so the next attempt asks again.
//!   Recording at the question would let one denied call buy silence for the
//!   next one.
//! * **It is process-local and deliberately not persisted.** The disclosure is
//!   about a running agent surprising a watching user, so "again after a
//!   restart" is the safe direction; a durable grant would silently outlive the
//!   session that earned it.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use super::{visibility, ProviderTier, SessionClassification};

/// Pairs that have already disclosed, as `(caller_session_id, target_session_id)`.
///
/// Unbounded in principle, bounded in practice by the number of conversations a
/// single process holds; each entry is two session ids.
static CROSSED: LazyLock<Mutex<HashSet<(String, String)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Whether this write is a first crossing, i.e. whether it must disclose its
/// payload before it happens.
///
/// `false` for every same-tier write, for a public caller (which cannot reach a
/// private target at all — that is a refusal, not a disclosure), and for a pair
/// that has already crossed.
pub fn needs_disclosure(
    caller: ProviderTier,
    target: SessionClassification,
    caller_session_id: &str,
    target_session_id: &str,
) -> bool {
    // One ask of the predicate in this file: the ceiling-aware form with no
    // ceiling is exactly the once-per-pair rule.
    needs_disclosure_to(caller, target, caller_session_id, target_session_id, false)
}

/// [`needs_disclosure`], for a target that may carry a spawn ceiling (DR-33,
/// design §3.4).
///
/// A subagent born under a parent that was public by a decision model runs
/// under that parent's Public ceiling. If the parent later becomes Private (its
/// last public decision model removed, then its next turn), its writes into the
/// still-public child — `workspace_send_prompt`, a steer, a continuation — are
/// private-to-public crossings, and **no approval is remembered for them**:
/// every such payload asks, showing the payload. One approval must not let a
/// now-private chat stream its content into a child it started while it was
/// public.
///
/// `target_ceiling_names_caller` is
/// [`crate::privacy::decision_axes::ceiling_names_caller`] of the target row.
pub fn needs_disclosure_to(
    caller: ProviderTier,
    target: SessionClassification,
    caller_session_id: &str,
    target_session_id: &str,
    target_ceiling_names_caller: bool,
) -> bool {
    if !visibility::requires_first_crossing_approval(caller, target) {
        return false;
    }
    target_ceiling_names_caller || !already_crossed(caller_session_id, target_session_id)
}

/// Record that this pair has now crossed. Called by the handler once the write
/// is committed, never by the gate that asks about it.
pub fn record(caller_session_id: &str, target_session_id: &str) {
    lock().insert((caller_session_id.to_string(), target_session_id.to_string()));
}

fn already_crossed(caller_session_id: &str, target_session_id: &str) -> bool {
    lock().contains(&(caller_session_id.to_string(), target_session_id.to_string()))
}

/// A poisoned mutex is not a reason to skip a disclosure, and it is also not a
/// reason to abort a turn: the guard is taken back and the worst case is that
/// the set is stale, which asks again.
fn lock() -> std::sync::MutexGuard<'static, HashSet<(String, String)>> {
    CROSSED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Test-only: forget every recorded crossing.
#[cfg(test)]
pub fn reset_for_test() {
    lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderTier::{Private as CPriv, Public as CPub};
    use SessionClassification::{Private as TPriv, Public as TPub};

    /// Serialized against the process-global ledger, so each test starts clean
    /// and two of them cannot interleave on the same pair.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        g
    }

    #[test]
    fn only_a_private_caller_writing_into_a_public_target_discloses() {
        let _g = guard();
        assert!(needs_disclosure(CPriv, TPub, "a", "b"));
        // Same tier both ways: nothing is crossing, so nothing is disclosed.
        assert!(!needs_disclosure(CPriv, TPriv, "a", "b"));
        assert!(!needs_disclosure(CPub, TPub, "a", "b"));
        // A public caller cannot reach a private target at all. That is
        // `may_write` refusing, and a disclosure here would imply the write was
        // about to happen.
        assert!(!needs_disclosure(CPub, TPriv, "a", "b"));
    }

    #[test]
    fn the_disclosure_is_once_per_pair_and_not_once_per_caller() {
        let _g = guard();
        assert!(needs_disclosure(CPriv, TPub, "caller", "first"));
        record("caller", "first");
        assert!(!needs_disclosure(CPriv, TPub, "caller", "first"));
        // A SECOND public target is a second crossing. Keying on the caller
        // alone would let one approval cover every public chat on the machine.
        assert!(needs_disclosure(CPriv, TPub, "caller", "second"));
        // And the pair is ordered: the target having disclosed to someone else
        // says nothing about this caller.
        assert!(needs_disclosure(CPriv, TPub, "other-caller", "first"));
    }

    /// DR-33 (design §3.4): a now-private parent writing into a child born
    /// under its Public ceiling asks EVERY time, however often it has crossed.
    #[test]
    fn a_child_under_the_callers_ceiling_asks_every_time() {
        let _g = guard();
        assert!(needs_disclosure_to(CPriv, TPub, "parent", "child", true));
        record("parent", "child");
        assert!(
            needs_disclosure_to(CPriv, TPub, "parent", "child", true),
            "a remembered approval silenced a private parent's write into its ceilinged child"
        );
        // An ordinary target keeps the once-per-pair rule.
        assert!(!needs_disclosure_to(CPriv, TPub, "parent", "child", false));
        // And the ceiling never invents a crossing that is not one.
        assert!(!needs_disclosure_to(CPub, TPub, "parent", "child", true));
        assert!(!needs_disclosure_to(CPriv, TPriv, "parent", "child", true));
    }

    #[test]
    fn asking_does_not_record() {
        let _g = guard();
        // The failure this rules out: a denied approval buying silence for the
        // retry. `needs_disclosure` is called once per attempt and must answer
        // the same way until a write actually lands.
        for _ in 0..3 {
            assert!(
                needs_disclosure(CPriv, TPub, "caller", "target"),
                "asking recorded the crossing, so a denial would silence the retry"
            );
        }
        record("caller", "target");
        assert!(!needs_disclosure(CPriv, TPub, "caller", "target"));
    }
}
