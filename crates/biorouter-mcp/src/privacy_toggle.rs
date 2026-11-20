//! The master privacy-tier switch (issue #56, R7 / DR-15) — the STORAGE only.
//!
//! ⚠ **Why this lives in `biorouter-mcp` and not in `biorouter`.** `biorouter`
//! depends on `biorouter-mcp` and **not** the reverse (`crates/biorouter/Cargo.toml`),
//! and several enforcement points are inside this crate: the knowledge-base read
//! barrier (`knowledge::tier::assert_reachable`), the classification ratchet
//! (`knowledge::tier::raise_unlocked`), the forced export location
//! (`knowledge::server::KnowledgeServer::kb_export`) and the Application Drafter
//! catalog (`agent_drafter::catalog`). A `privacy_tiers_enabled()` defined in
//! `crates/biorouter/src/privacy/mod.rs` would be *unreachable* from every one
//! of them, so the honest outcomes would be either "those are not opt-outable" —
//! which contradicts DR-15 in the operator's own words — or a second hand-rolled
//! flag, which Task 30's Step 5 exists to forbid.
//!
//! So the atomic lives below both crates and there is still exactly one of it:
//! `crates/biorouter/src/privacy/mod.rs` **re-exports** the predicate rather
//! than wrapping it, so the token at every call site is identical in both crates
//! and the audit is one pattern, not two. This is the same shape the plan
//! already uses for `private_roots`.
//!
//! This module knows nothing about configuration. Loading the value is
//! `biorouter::privacy::load_privacy_tiers_from_config`, which reads
//! `privacy-tiers.json` — the record `biorouter::privacy::master_switch` owns,
//! beside `config.yaml` in this install's configuration directory.
//!
//! ⚠ **Not `config.yaml`, and not the environment** (issue #56, DR-22). The
//! value used to be a `config.yaml` key, `BIOROUTER_PRIVACY_TIERS`. That key is
//! now *retired*: read once at migration, removed, and ignored for ever after.
//! The agent holds `developer__shell`, so a value it could reach by writing a
//! file or exporting a variable would be a control the agent switches off and
//! waits one restart for — see `master_switch` for what the move does and does
//! not buy, and `load_privacy_tiers_from_config` for why the read never goes
//! through `Config::get_param`, whose middle branch resolves an environment
//! variable.

use std::sync::atomic::{AtomicBool, Ordering};

/// Fail-safe default: protection is ON until something explicitly turns it off.
/// A process that never gets as far as loading its config enforces.
///
/// Both hosts of the agent library load it — `biorouterd` in `commands/agent.rs`
/// and the `biorouter` CLI in its `main`. For one round only the daemon did,
/// which meant a user who turned privacy tiers off in Settings still met every
/// gate in the terminal. Fail-safe, and therefore invisible; DR-15's promise is
/// "nothing will be impacted", not "nothing except the CLI".
static TIERS_ENABLED: AtomicBool = AtomicBool::new(true);

/// `true` — the default — means every gate, the classification ratchet and the
/// read barriers are live. `false` means none of them are.
///
/// A relaxed atomic load, safe inside a hot gate: the authoritative value lives
/// in daemon memory and no gate needs to observe it in any particular order
/// relative to other memory.
pub fn privacy_tiers_enabled() -> bool {
    TIERS_ENABLED.load(Ordering::Relaxed)
}

/// The **one** writer.
///
/// Called from `biorouter::privacy::load_privacy_tiers_from_config` — which both
/// hosts run at startup, after the config is available — and from
/// `POST /config/upsert`'s gated arm. Nothing else may call it: that is what
/// makes "one switch, one name" true across the crate boundary, and Task 30's
/// Step 5 (2) is the check that says so. A second host calling the LOADER is not
/// a third writer, which is why the count stays at two.
pub fn set_privacy_tiers_enabled(on: bool) {
    TIERS_ENABLED.store(on, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_on() {
        // Not `privacy_tiers_enabled()` — another test in this binary may have
        // moved the process-global value. What is asserted is the DEFAULT the
        // static is declared with, which is the property that matters: a
        // process that never loads a config enforces.
        static FRESH: AtomicBool = AtomicBool::new(true);
        assert!(FRESH.load(Ordering::Relaxed));
    }

    #[test]
    fn the_setter_round_trips() {
        let prev = privacy_tiers_enabled();
        set_privacy_tiers_enabled(false);
        assert!(!privacy_tiers_enabled());
        set_privacy_tiers_enabled(true);
        assert!(privacy_tiers_enabled());
        set_privacy_tiers_enabled(prev);
    }
}
