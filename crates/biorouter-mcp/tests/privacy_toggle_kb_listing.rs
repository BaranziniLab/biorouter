//! DR-15 over the **one** predicate every knowledge-base listing now asks
//! (issue #56; audit finding 17).
//!
//! # What this pins, and why one test covers three surfaces
//!
//! Before finding 17 there were three spellings of "may this caller see this
//! base?" — `KnowledgeServer`'s five listings, `resolve_target_kb`'s
//! conversation-ingest candidate list in `biorouter`, and `Catalog::discover`'s
//! app catalogue here — and they disagreed about *which axes* and *whether the
//! master toggle applied*. `resolve_target_kb` did not read the toggle at all,
//! so with privacy tiers switched OFF it went on withholding the names of bases
//! the very next call would hand over in full. That is DR-15's promise
//! ("nothing will be impacted") broken in the direction nobody looks.
//!
//! All three now delegate to [`KbCaller::can_reach`], which is
//! `tier::assert_reachable` negated — so the toggle is read once, at the choke
//! point, for the "omit" decision and the "refuse" decision together. Asserting
//! the toggle at that predicate is therefore the assertion for every listing
//! that asks it; a surface that stopped asking it would be caught by the
//! source-text guards beside each one (`caller.rs`, `server.rs`).
//!
//! # Why this is an integration binary
//!
//! Verbatim the reason `privacy_toggle_export.rs` records: the toggle is a
//! PROCESS-GLOBAL atomic and `cargo test` runs a crate's unit tests in parallel
//! threads of ONE process. An OFF window opened inside `--lib` disables the
//! barrier under the ~250 knowledge tests that neither take a guard nor can be
//! made to. Each `tests/*.rs` file is its own process, so nothing outside this
//! file can observe its writes.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::knowledge::affiliation::CallerAffiliation;
use biorouter_mcp::knowledge::caller::KbCaller;
use biorouter_mcp::knowledge::service::KnowledgeService;
use biorouter_mcp::knowledge::types::KbFormat;

/// Restores the toggle on drop, including on the unwind path a failing
/// assertion takes.
struct Restore(bool);

impl Drop for Restore {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.0);
    }
}

#[test]
fn every_kb_listing_predicate_follows_the_master_toggle() {
    let _restore = Restore(biorouter_mcp::privacy_toggle::privacy_tiers_enabled());
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let svc = KnowledgeService::new(root.to_path_buf());

    // A base created through the production path that stamps BOTH axes in one
    // transaction — private, and owned by UCSF.
    svc.create_base_as(
        "omop",
        "OMOP",
        None,
        KbFormat::default(),
        /* caller_is_private */ true,
        &CallerAffiliation::Institution("ucsf".to_string()),
    )
    .expect("create the claimed private base");
    svc.create_base_as(
        "notes",
        "Notes",
        None,
        KbFormat::default(),
        /* caller_is_private */ false,
        &CallerAffiliation::Unstated,
    )
    .expect("create the public base");

    let public = KbCaller::restricted();
    let ucsf = KbCaller::new(true, CallerAffiliation::Institution("ucsf".to_string()));
    let stanford = KbCaller::new(true, CallerAffiliation::Institution("stanford".to_string()));

    // ── ON: the shipped default. Both axes decide. ───────────────────────────
    assert!(public.can_reach(root, "notes"));
    assert!(!public.can_reach(root, "omop"), "the tier axis");
    assert!(ucsf.can_reach(root, "omop"), "the owner reads its own base");
    assert!(
        !stanford.can_reach(root, "omop"),
        "the affiliation axis, the one finding 17's listings could not ask. \
         A private caller passes the tier axis, so without this row a listing \
         that asked the tier alone would look correct."
    );

    // ── OFF: nothing is withheld, on either axis. ────────────────────────────
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
    assert!(
        public.can_reach(root, "omop"),
        "with tiers off the barrier permits, so a listing that still hid this \
         base would withhold a NAME for content the very next call serves in full"
    );
    assert!(
        stanford.can_reach(root, "omop"),
        "DR-26's axis stops with the rest of the feature"
    );
    assert!(public.can_reach(root, "notes"));

    // ── ON again: the OFF column is a window, not a latch. ───────────────────
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    assert!(!public.can_reach(root, "omop"));
    assert!(!stanford.can_reach(root, "omop"));
}
