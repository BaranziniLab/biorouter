//! Task 30A (issue #56, DR-17 requirement 3): the disclosure is independent of
//! the master toggle.
//!
//! ⚠ **Why this is an integration binary and not `--lib`.** The master switch is
//! a PROCESS-GLOBAL atomic and `cargo test` runs a crate's unit tests in
//! parallel threads of ONE process, so a unit test that moved it would be read
//! by the ~60 privacy tests beside it — which would fail spuriously or, worse,
//! pass while asserting nothing. `crates/biorouter/tests/privacy_toggle.rs`
//! documents the same hazard and takes the same way out. Each `tests/*.rs` file
//! is its own process, so this file's writes reach nothing but this file.
//!
//! ⚠ **This is the one part of the feature that must work with the toggle OFF.**
//! DR-15 turns off gates, the ratchet and refusals; it does not turn off the
//! truth. With enforcement off the exposure is *larger*, so wiring the
//! disclosure behind `privacy_tiers_enabled()` — the natural mistake, because
//! every other privacy surface reads it — would silence it in exactly the
//! configuration where the risk is highest.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::privacy::disclosure;
use biorouter::providers::base::ProviderMetadata;

async fn meta(name: &str) -> ProviderMetadata {
    biorouter::providers::providers()
        .await
        .into_iter()
        .find(|(m, _)| m.name == name)
        .map(|(m, _)| m)
        .unwrap_or_else(|| panic!("no registry entry for `{name}`"))
}

#[tokio::test]
async fn the_disclosure_is_independent_of_the_master_toggle() {
    // DR-15 turns enforcement off. It does not turn the truth off, and with
    // enforcement off the exposure is larger, not smaller.
    let public = meta("openai").await;
    let private = meta("versa_azure").await;
    for enabled in [true, false] {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(enabled);
        assert!(
            disclosure::required_for(&public),
            "the disclosure went quiet with the master toggle {enabled}"
        );
        // …and it does not START being required for a private model either: the
        // predicate reads one thing and the toggle is not it.
        assert!(!disclosure::required_for(&private));
    }
    // Leave the process as it was found, for any test scheduled after this one.
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
}
