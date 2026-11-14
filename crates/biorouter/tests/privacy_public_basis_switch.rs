//! DR-33 (design §2.5): with the master switch off, no public basis is written.
//!
//! A public decision model saved while tiers are off is a name without a basis,
//! so when tiers come back on it lowers nothing (doubt never lowers): a Versa
//! chat that collected patient notes with tiers off can never be made public by
//! adding a Baseten monitor afterwards. Only declassification can do that.
//!
//! Its own binary because the switch is a process-global atomic: flipping it in
//! `--lib` would race every other test in the crate. Both tests here hold one
//! lock so they cannot see each other's flip.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::privacy::decision_axes::{
    self, BasisAuthor, PublicBasis, PUBLIC_BASIS_STATE, PUBLIC_BASIS_VERSION,
};
use biorouter::session::session_manager::{BasisWrite, BasisWriteOutcome, SessionType};
use biorouter::session::SessionManager;

static SWITCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Restore(bool);
impl Drop for Restore {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.0);
    }
}

async fn lower(sm: &SessionManager, id: &str) -> BasisWriteOutcome<()> {
    let basis = PublicBasis::new(id, BasisAuthor::Person, "test", vec![]);
    sm.write_public_basis(id, BasisWrite::Set(basis), |data| {
        data.set_extension_state(
            "decision_settings",
            "v1",
            serde_json::json!({
                "defaultModel": { "provider": "baseten_decision", "model": "joey-1" },
                "monitors": [],
            }),
        );
        Ok(())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn with_the_switch_off_the_save_lands_and_no_basis_is_written() {
    let _serial = SWITCH.lock().await;
    let _restore = Restore(biorouter_mcp::privacy_toggle::privacy_tiers_enabled());
    let dir = tempfile::TempDir::new().unwrap();
    let sm = SessionManager::new(dir.path().to_path_buf());
    let id = sm
        .create_session(
            dir.path().to_path_buf(),
            "off".to_string(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;

    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
    assert_eq!(lower(&sm, &id).await, BasisWriteOutcome::Written(()));
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);

    let row = sm.get_session(&id, false).await.unwrap();
    assert!(
        row.extension_data
            .get_extension_state("decision_settings", "v1")
            .is_some(),
        "the save itself must still land"
    );
    assert!(
        row.extension_data
            .get_extension_state(PUBLIC_BASIS_STATE, PUBLIC_BASIS_VERSION)
            .is_none(),
        "a basis was written with the master switch off"
    );
    assert!(
        decision_axes::from_row(&row).is_identity(),
        "a name saved with tiers off lowered the chat once they came back on"
    );
}

#[tokio::test]
async fn with_the_switch_on_the_same_save_writes_its_basis() {
    let _serial = SWITCH.lock().await;
    let _restore = Restore(biorouter_mcp::privacy_toggle::privacy_tiers_enabled());
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    let dir = tempfile::TempDir::new().unwrap();
    let sm = SessionManager::new(dir.path().to_path_buf());
    let id = sm
        .create_session(
            dir.path().to_path_buf(),
            "on".to_string(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;
    assert_eq!(lower(&sm, &id).await, BasisWriteOutcome::Written(()));
    let row = sm.get_session(&id, false).await.unwrap();
    assert!(!decision_axes::from_row(&row).is_identity());
}
