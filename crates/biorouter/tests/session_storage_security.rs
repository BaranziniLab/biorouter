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

#[cfg(unix)]
#[tokio::test]
async fn existing_database_aliases_are_refused_without_mutating_outside_files() {
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        for hard_link in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let outside = fixture.path().join("public-backup");
            fs::write(&outside, b"SYNTHETIC_OUTSIDE_CANARY").unwrap();
            fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
            let sessions = fixture.path().join("sessions");
            fs::create_dir(&sessions).unwrap();
            let alias = sessions.join(name);
            if hard_link {
                fs::hard_link(&outside, &alias).unwrap();
            } else {
                symlink(&outside, &alias).unwrap();
            }
            let before = fs::metadata(&outside).unwrap();
            let manager = SessionManager::new(fixture.path().to_path_buf());
            let error = manager.validate_storage().unwrap_err();
            assert!(error.to_string().contains("BIOROUTER_PATH_ROOT"));
            assert!(manager
                .create_session(
                    fixture.path().to_path_buf(),
                    "SYNTHETIC_REFUSED_ALIAS".into(),
                    SessionType::User,
                )
                .await
                .is_err());
            let after = fs::metadata(&outside).unwrap();
            assert_eq!(fs::read(&outside).unwrap(), b"SYNTHETIC_OUTSIDE_CANARY");
            assert_eq!(after.permissions().mode(), before.permissions().mode());
            assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
            assert_eq!(after.nlink(), before.nlink());
            if name != "sessions.db" {
                assert!(!sessions.join("sessions.db").exists());
            }
            fs::remove_file(alias).unwrap();
            manager.validate_storage().unwrap();
            let recovered = manager
                .create_session(
                    fixture.path().to_path_buf(),
                    "SYNTHETIC_ALIAS_RECOVERY".into(),
                    SessionType::User,
                )
                .await
                .unwrap();
            assert_eq!(
                manager
                    .get_session(&recovered.id, false)
                    .await
                    .unwrap()
                    .name,
                "SYNTHETIC_ALIAS_RECOVERY"
            );
            manager.close().await;
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn nonregular_database_entries_are_refused_without_blocking_or_mutation() {
    use std::os::unix::ffi::OsStrExt;

    for directory in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let sessions = fixture.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        let database = sessions.join("sessions.db");
        if directory {
            fs::create_dir(&database).unwrap();
        } else {
            let name = std::ffi::CString::new(database.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        let manager = SessionManager::new(fixture.path().to_path_buf());
        assert!(manager.validate_storage().is_err());
        assert!(!sessions.join("sessions.db-wal").exists());
        manager.close().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn existing_and_new_database_files_are_private_and_history_reopens() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempfile::tempdir().unwrap();
    let sessions = fixture.path().join("sessions");
    let manager = SessionManager::new(fixture.path().to_path_buf());
    let session = manager
        .create_session(
            fixture.path().to_path_buf(),
            "SYNTHETIC_PRIVATE_DATABASE".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let path = sessions.join(name);
        if path.exists() {
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    manager.close().await;
    fs::set_permissions(
        sessions.join("sessions.db"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let reopened = SessionManager::new(fixture.path().to_path_buf());
    reopened.validate_storage().unwrap();
    assert_eq!(
        fs::metadata(sessions.join("sessions.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        reopened.get_session(&session.id, false).await.unwrap().name,
        "SYNTHETIC_PRIVATE_DATABASE"
    );
    reopened.close().await;
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

#[cfg(windows)]
#[tokio::test]
async fn existing_database_hard_links_are_refused_before_changing_outside_acls() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{ACL, DACL_SECURITY_INFORMATION};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
    };

    fn dacl(path: &Path) -> Vec<u8> {
        let file = fs::OpenOptions::new()
            .access_mode(READ_CONTROL)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(path)
            .unwrap();
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle().cast(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut acl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, 0);
        let bytes = if acl.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(acl.cast(), usize::from((*acl).AclSize)) }.to_vec()
        };
        unsafe { LocalFree(descriptor) };
        bytes
    }

    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let fixture = tempfile::tempdir().unwrap();
        let sessions = fixture.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        let outside = fixture.path().join("outside-canary");
        fs::write(&outside, b"SYNTHETIC_OUTSIDE_CANARY").unwrap();
        fs::hard_link(&outside, sessions.join(name)).unwrap();
        let before = dacl(&outside);
        let manager = SessionManager::new(fixture.path().to_path_buf());
        assert!(manager.validate_storage().is_err());
        assert!(manager
            .create_session(
                fixture.path().to_path_buf(),
                "SYNTHETIC_REFUSED_ALIAS".into(),
                SessionType::User,
            )
            .await
            .is_err());
        assert_eq!(dacl(&outside), before);
        assert_eq!(fs::read(&outside).unwrap(), b"SYNTHETIC_OUTSIDE_CANARY");
        if name != "sessions.db" {
            assert!(!sessions.join("sessions.db").exists());
        }
        fs::remove_file(sessions.join(name)).unwrap();
        manager.validate_storage().unwrap();
        assert_eq!(dacl(&outside), before);
        manager.close().await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn known_database_directories_are_refused_before_sqlite_opens() {
    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let fixture = tempfile::tempdir().unwrap();
        let sessions = fixture.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        fs::create_dir(sessions.join(name)).unwrap();
        let manager = SessionManager::new(fixture.path().to_path_buf());
        assert!(manager.validate_storage().is_err());
        if name != "sessions.db" {
            assert!(!sessions.join("sessions.db").exists());
        }
        manager.close().await;
    }
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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn existing_database_and_sidecar_acls_are_repaired_on_the_opened_files() {
    let fixture = tempfile::tempdir().unwrap();
    let sessions = fixture.path().join("sessions");
    fs::create_dir(&sessions).unwrap();
    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let path = sessions.join(name);
        fs::write(&path, b"SYNTHETIC_HISTORY_FILE").unwrap();
        let output = std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read,readattr"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let manager = SessionManager::new(fixture.path().to_path_buf());
    manager.validate_storage().unwrap();
    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let path = sessions.join(name);
        let listing = std::process::Command::new("/bin/ls")
            .arg("-le")
            .arg(&path)
            .output()
            .unwrap();
        assert!(listing.status.success());
        assert!(!String::from_utf8_lossy(&listing.stdout).contains("everyone"));
        assert_eq!(fs::read(path).unwrap(), b"SYNTHETIC_HISTORY_FILE");
    }
    manager.close().await;
}
