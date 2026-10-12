//! DR-15 / AR-7 for the KB-to-KB merge: with the master privacy toggle OFF,
//! the merge's classification fold does not ratchet — and its **dry run** says
//! so rather than promising a raise that will not happen.
//!
//! ⚠ **Why this is an integration binary and not a `#[cfg(test)] mod` beside
//! the merge.** The toggle is a PROCESS-GLOBAL atomic and `cargo test` runs a
//! crate's unit tests in parallel threads of one process, so an OFF window
//! written next to the code would disable the barrier under the ~250 knowledge
//! unit tests that neither take the guard nor can be made to. Same reason
//! `privacy_toggle_export.rs` and `privacy_toggle_kb_listing.rs` are their own
//! binaries; each `tests/*.rs` file is its own process, so nothing outside this
//! file observes its writes.
//!
//! ⚠ **What it would catch.** The projection in
//! `KnowledgeService::projected_classification` re-derives the ratchet's
//! arithmetic rather than performing it — that is the one duplication in the
//! feature, and it is bounded by reading the toggle through
//! `tier::ratchets_are_live`, the same spelling both ratchets read. If a future
//! edit gives the preview a read of its own, this file is where the two come
//! apart: with the feature off, the preview would announce `private` and the
//! merge would leave the base public.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::knowledge::{
    affiliation::CallerAffiliation,
    merge::{MergeAuthority, UserKbMerge},
    service::KnowledgeService,
    tier,
    types::KbFormat,
};
use std::collections::BTreeSet;

/// Restores the toggle on drop, including on the unwind path a failing
/// assertion takes.
struct Restore(bool);

impl Drop for Restore {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.0);
    }
}

fn ucsf() -> CallerAffiliation {
    CallerAffiliation::Institution("ucsf".to_string())
}

/// A private base owned by UCSF, plus a public destination, both created through
/// the production path that stamps the tier in the same transaction.
fn seed(svc: &KnowledgeService) {
    svc.create_base("dst", "Destination", None)
        .expect("create the public destination");
    svc.create_base_as(
        "src",
        "Source",
        None,
        KbFormat::default(),
        /* caller_is_private */ true,
        &ucsf(),
    )
    .expect("create the private source");
    let page = svc.root().join("src").join("knowledge").join("x.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(
        &page,
        "---\ntype: Concept\nidentifier: X\n---\n\nSENTINEL-COHORT-N-412\n",
    )
    .unwrap();
}

/// ⚠ `#[serial]`, and it is not optional: the toggle is a process-global atomic
/// and both tests in this binary write it, so run in parallel the OFF window of
/// one lands inside the ON window of the other. Measured, not predicted — the
/// first run failed on `create_base_as` stamping the source PRIVATE inside a
/// window this test had just turned off.
#[tokio::test]
#[serial_test::serial(privacy_toggle_merge)]
async fn the_merge_fold_and_its_preview_both_follow_the_master_toggle() {
    let _restore = Restore(biorouter_mcp::privacy_toggle::privacy_tiers_enabled());
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);

    let tmp = tempfile::tempdir().unwrap();
    let svc = KnowledgeService::new(tmp.path().to_path_buf());
    seed(&svc);
    let root = tmp.path();
    assert!(tier::is_private(root, "src"));
    assert!(!tier::is_private(root, "dst"));

    // ON: the preview promises the raise, and the merge performs it.
    let user = UserKbMerge::from_user_action();
    let preview = svc
        .merge_bases("dst", "src", &MergeAuthority::User(&user), true)
        .await
        .expect("a preview");
    assert_eq!(preview.destination_tier, "private");
    assert_eq!(preview.owners_added, vec!["ucsf".to_string()]);
    assert!(
        !tier::is_private(root, "dst"),
        "the preview ratcheted the destination"
    );

    let applied = svc
        .merge_bases("dst", "src", &MergeAuthority::User(&user), false)
        .await
        .expect("a merge");
    assert_eq!(applied.destination_tier, "private");
    assert_eq!(applied.owners_added, vec!["ucsf".to_string()]);
    assert!(tier::is_private(root, "dst"));
    assert_eq!(
        tier::affiliation(root, "dst").owners(),
        Some(&BTreeSet::from(["ucsf".to_string()]))
    );

    // OFF, on a second pair. Nothing ratchets — and the preview must agree,
    // which is the half a re-derived toggle read gets wrong.
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
    let tmp2 = tempfile::tempdir().unwrap();
    let svc2 = KnowledgeService::new(tmp2.path().to_path_buf());
    // Created with the feature OFF, so `src2` is not even stamped private: with
    // nothing ratcheting there is nothing for the fold to carry, which is the
    // whole of DR-15's promise.
    seed(&svc2);
    let root2 = tmp2.path();
    assert!(!tier::is_private(root2, "src"));

    let preview = svc2
        .merge_bases("dst", "src", &MergeAuthority::User(&user), true)
        .await
        .expect("a preview");
    assert_eq!(preview.destination_tier, "public");
    assert!(preview.owners_added.is_empty());
    assert_eq!(
        preview.pages_carried.len(),
        1,
        "the merge itself must still work with the feature off: {preview:#?}"
    );

    let applied = svc2
        .merge_bases("dst", "src", &MergeAuthority::User(&user), false)
        .await
        .expect("a merge");
    assert_eq!(applied.destination_tier, "public");
    assert!(applied.owners_added.is_empty());
    assert!(!tier::is_private(root2, "dst"));
    assert!(tmp2.path().join("dst/knowledge/x.md").exists());

    // …and a tier stamped while the feature was on is NOT erased by turning it
    // off (AR-7). The first pair is still classified.
    assert!(tier::is_private(root, "dst"));
}

/// The stronger half of the same rule, and the one an implementation is most
/// likely to get wrong: with the feature ON, a base that was **already** private
/// before the merge stays private whatever the source is. The fold is a ratchet,
/// so a public source cannot lower it.
#[tokio::test]
#[serial_test::serial(privacy_toggle_merge)]
async fn a_public_source_never_lowers_a_private_destination() {
    let _restore = Restore(biorouter_mcp::privacy_toggle::privacy_tiers_enabled());
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);

    let tmp = tempfile::tempdir().unwrap();
    let svc = KnowledgeService::new(tmp.path().to_path_buf());
    let root = tmp.path();
    svc.create_base_as(
        "dst",
        "Destination",
        None,
        KbFormat::default(),
        /* caller_is_private */ true,
        &ucsf(),
    )
    .unwrap();
    svc.create_base("src", "Source", None).unwrap();
    let page = root.join("src").join("knowledge").join("x.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "---\ntype: Concept\nidentifier: X\n---\n\nb\n").unwrap();

    let user = UserKbMerge::from_user_action();
    let report = svc
        .merge_bases("dst", "src", &MergeAuthority::User(&user), false)
        .await
        .unwrap();

    assert_eq!(report.destination_tier, "private");
    assert!(tier::is_private(root, "dst"));
    assert_eq!(
        tier::affiliation(root, "dst").owners(),
        Some(&BTreeSet::from(["ucsf".to_string()])),
        "a merge must never drop an owner"
    );
}
