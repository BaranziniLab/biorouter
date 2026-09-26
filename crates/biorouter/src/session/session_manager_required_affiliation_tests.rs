#![cfg(test)]

use super::{SessionManager, SessionType};
use crate::conversation::message::Message;
use crate::privacy::affiliation::InstitutionId;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

const PRIVACY_TOGGLE_CHILD: &str = "BIOROUTER_REQUIRED_AFFILIATION_PRIVACY_CHILD";
const PRIVACY_TOGGLE_CHILD_SUCCESS: &str = "required-affiliation-privacy-child-passed";
const CREW_SCOPE_CHILD: &str = "BIOROUTER_REQUIRED_AFFILIATION_CREW_SCOPE_CHILD";
const CREW_SCOPE_CHILD_SUCCESS: &str = "required-affiliation-crew-scope-child-passed";

async fn fresh_session(manager: &SessionManager) -> crate::session::session_manager::Session {
    manager
        .create_session(
            PathBuf::from("/tmp/crew-affiliation-test"),
            "fixture".into(),
            SessionType::User,
        )
        .await
        .unwrap()
}

fn set_privacy_toggle(enabled: bool) -> bool {
    let previous = crate::privacy::privacy_tiers_enabled();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(enabled);
    previous
}

struct PrivacyToggleGuard(bool);

impl Drop for PrivacyToggleGuard {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.0);
    }
}

fn privacy_toggle(enabled: bool) -> PrivacyToggleGuard {
    PrivacyToggleGuard(set_privacy_toggle(enabled))
}

fn run_privacy_toggle_child(test_name: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(PRIVACY_TOGGLE_CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated privacy-toggle child failed: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(PRIVACY_TOGGLE_CHILD_SUCCESS),
        "isolated privacy-toggle child ran without its success marker\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_crew_scope_child(test_name: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(CREW_SCOPE_CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated Crew-scope child failed: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(CREW_SCOPE_CHILD_SUCCESS),
        "isolated Crew-scope child ran without its success marker\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn required_affiliation_persists_when_optional_privacy_recording_is_off() {
    if std::env::var_os(PRIVACY_TOGGLE_CHILD).is_none() {
        run_privacy_toggle_child(
            "session::session_manager::required_affiliation_tests::required_affiliation_persists_when_optional_privacy_recording_is_off",
        );
        return;
    }

    let _toggle = privacy_toggle(false);
    let temp = TempDir::new().unwrap();
    let manager = SessionManager::new(temp.path().to_path_buf());
    let session = fresh_session(&manager).await;

    manager
        .record_required_session_affiliation(&session.id, InstitutionId::new("UCSF"))
        .await
        .unwrap();

    assert_eq!(
        manager.session_affiliations(&session.id).await.unwrap(),
        [InstitutionId::new("ucsf")].into_iter().collect()
    );
    println!("{PRIVACY_TOGGLE_CHILD_SUCCESS}");
}

#[tokio::test]
async fn required_affiliation_rejects_a_missing_session() {
    let temp = TempDir::new().unwrap();
    let manager = SessionManager::new(temp.path().to_path_buf());
    let error = manager
        .record_required_session_affiliation("missing", InstitutionId::new("ucsf"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("Session unavailable"));
}

#[tokio::test]
async fn optional_affiliation_recording_still_obeys_the_privacy_opt_out() {
    if std::env::var_os(PRIVACY_TOGGLE_CHILD).is_none() {
        run_privacy_toggle_child(
            "session::session_manager::required_affiliation_tests::optional_affiliation_recording_still_obeys_the_privacy_opt_out",
        );
        return;
    }

    let _toggle = privacy_toggle(false);
    let temp = TempDir::new().unwrap();
    let manager = SessionManager::new(temp.path().to_path_buf());
    let session = fresh_session(&manager).await;

    manager
        .record_session_affiliation(&session.id, InstitutionId::new("ucsf"))
        .await
        .unwrap();

    assert!(manager
        .session_affiliations(&session.id)
        .await
        .unwrap()
        .is_empty());
    println!("{PRIVACY_TOGGLE_CHILD_SUCCESS}");
}

#[tokio::test]
async fn copy_and_diverge_carry_the_affiliation_union_before_the_transcript() {
    let temp = TempDir::new().unwrap();
    let manager = SessionManager::new(temp.path().to_path_buf());
    let source = fresh_session(&manager).await;
    manager
        .record_required_session_affiliation(&source.id, InstitutionId::new("ucsf"))
        .await
        .unwrap();
    manager
        .record_required_session_affiliation(&source.id, InstitutionId::new("stanford"))
        .await
        .unwrap();
    manager
        .add_message(&source.id, &Message::user().with_text("hello"))
        .await
        .unwrap();
    manager
        .add_message(&source.id, &Message::assistant().with_text("hi"))
        .await
        .unwrap();

    let expected = manager.session_affiliations(&source.id).await.unwrap();
    let copied = manager
        .copy_session(&source.id, "copy".into())
        .await
        .unwrap();
    let diverged = manager
        .diverge_session(&source.id, Some("branch".into()), None)
        .await
        .unwrap();

    assert_eq!(
        manager.session_affiliations(&copied.id).await.unwrap(),
        expected
    );
    assert_eq!(
        manager.session_affiliations(&diverged.id).await.unwrap(),
        expected
    );
    assert_eq!(copied.message_count, 2);
    assert_eq!(diverged.message_count, 2);
    assert_eq!(
        manager
            .get_session(&copied.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .map(|message| message.as_concat_text())
            .collect::<Vec<_>>(),
        vec!["hello", "hi"]
    );
}

#[tokio::test]
async fn malformed_source_affiliation_fails_before_copying_a_session() {
    let temp = TempDir::new().unwrap();
    let manager = SessionManager::new(temp.path().to_path_buf());
    let source = fresh_session(&manager).await;
    manager
        .add_message(&source.id, &Message::user().with_text("fixture"))
        .await
        .unwrap();
    let pool = manager.storage().pool().await.unwrap();
    sqlx::query("UPDATE sessions SET session_affiliations = ?1 WHERE id = ?2")
        .bind("not-json")
        .bind(&source.id)
        .execute(pool)
        .await
        .unwrap();

    let error = manager
        .copy_session(&source.id, "should not exist".into())
        .await;
    assert!(error.is_err());
    assert_eq!(manager.list_sessions().await.unwrap().len(), 1);
}

#[tokio::test]
async fn crew_scoped_copy_and_diverge_refuse_before_creating_children() {
    if std::env::var_os(CREW_SCOPE_CHILD).is_none() {
        run_crew_scope_child(
            "session::session_manager::required_affiliation_tests::crew_scoped_copy_and_diverge_refuse_before_creating_children",
        );
        return;
    }

    let temp = TempDir::new().unwrap();
    let manager = SessionManager::new(temp.path().to_path_buf());
    let source = fresh_session(&manager).await;
    manager
        .add_message(&source.id, &Message::user().with_text("fixture"))
        .await
        .unwrap();
    let crew = crate::crew::install_test_scope(&source.id, None).await;

    let copy_error = manager.copy_session(&source.id, "copy".into()).await;
    assert!(copy_error.is_err());
    let diverge_error = manager
        .diverge_session(&source.id, Some("branch".into()), None)
        .await;
    assert!(diverge_error.is_err());
    assert_eq!(manager.list_sessions().await.unwrap().len(), 1);

    crate::crew::remove_test_scope(&crew, &source.id).await;
    println!("{CREW_SCOPE_CHILD_SUCCESS}");
}
