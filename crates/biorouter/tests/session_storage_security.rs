use biorouter::session::{SessionManager, SessionType};
use std::fs;

#[tokio::test]
async fn explicit_real_directory_relocation_preserves_history_and_reopening() {
    let fixture = tempfile::tempdir().unwrap();
    let relocated = fixture.path().join("chosen-app-root/data");
    let manager = SessionManager::new(relocated.clone());
    manager.validate_storage().unwrap();
    let session = manager
        .create_session(
            fixture.path().to_path_buf(),
            "SYNTHETIC_RELOCATED_HISTORY".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager.close().await;
    assert!(relocated.join("sessions/sessions.db").is_file());
    let reopened = SessionManager::new(relocated);
    assert_eq!(
        reopened.get_session(&session.id, false).await.unwrap().name,
        "SYNTHETIC_RELOCATED_HISTORY"
    );
    reopened.close().await;
}

#[tokio::test]
async fn unsuitable_storage_returns_recovery_guidance_without_a_constructor_panic() {
    let fixture = tempfile::tempdir().unwrap();
    let sessions = fixture.path().join("sessions");
    fs::write(&sessions, b"SYNTHETIC_UNSUITABLE_PATH").unwrap();
    let manager = SessionManager::new(fixture.path().to_path_buf());
    let error = manager.validate_storage().unwrap_err();
    assert!(error.to_string().contains("BIOROUTER_PATH_ROOT"));
    assert_eq!(fs::read(&sessions).unwrap(), b"SYNTHETIC_UNSUITABLE_PATH");
    let failed = manager
        .create_session(
            fixture.path().to_path_buf(),
            "SYNTHETIC_REFUSED_HISTORY".into(),
            SessionType::User,
        )
        .await;
    assert!(failed.is_err());
    fs::remove_file(&sessions).unwrap();
    let session = manager
        .create_session(
            fixture.path().to_path_buf(),
            "SYNTHETIC_RECOVERED_HISTORY".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    assert_eq!(
        manager.get_session(&session.id, false).await.unwrap().name,
        "SYNTHETIC_RECOVERED_HISTORY"
    );
    manager.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn refused_link_target_is_unchanged_and_repair_can_resume_the_same_manager() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("other-owned-folder");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let sessions = fixture.path().join("sessions");
    symlink(&target, &sessions).unwrap();
    let manager = SessionManager::new(fixture.path().to_path_buf());
    assert!(manager.validate_storage().is_err());
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    fs::remove_file(&sessions).unwrap();
    manager.validate_storage().unwrap();
    assert_eq!(
        fs::metadata(&sessions).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let session = manager
        .create_session(
            fixture.path().to_path_buf(),
            "SYNTHETIC_LINK_RECOVERY".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    assert_eq!(
        manager.get_session(&session.id, false).await.unwrap().name,
        "SYNTHETIC_LINK_RECOVERY"
    );
    manager.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn junction_storage_is_refused_without_creating_a_database_in_its_target() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("other-owned-folder");
    fs::create_dir(&target).unwrap();
    let sessions = fixture.path().join("sessions");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&sessions)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manager = SessionManager::new(fixture.path().to_path_buf());
    assert!(manager.validate_storage().is_err());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    fs::remove_dir(&sessions).unwrap();
    manager.validate_storage().unwrap();
    manager.close().await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn extended_acl_does_not_retain_access_for_other_accounts() {
    let fixture = tempfile::tempdir().unwrap();
    let sessions = fixture.path().join("sessions");
    fs::create_dir(&sessions).unwrap();
    let output = std::process::Command::new("/bin/chmod")
        .args(["+a", "everyone allow list,search,readattr"])
        .arg(&sessions)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manager = SessionManager::new(fixture.path().to_path_buf());
    manager.validate_storage().unwrap();
    let listing = std::process::Command::new("/bin/ls")
        .arg("-lde")
        .arg(&sessions)
        .output()
        .unwrap();
    assert!(listing.status.success());
    assert!(!String::from_utf8_lossy(&listing.stdout).contains("everyone"));
    manager.close().await;
}
