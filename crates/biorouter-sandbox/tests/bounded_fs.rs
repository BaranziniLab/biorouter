use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use biorouter_sandbox::bounded_fs::authorized_path;
use biorouter_sandbox::{AccessMode, BoundedFs, DispatchAuthority, RootGrant};

fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write(fs: &BoundedFs, path: impl AsRef<Path>, content: &[u8]) {
    fs.bind(path, AccessMode::Write)
        .unwrap()
        .atomic_write(content)
        .unwrap();
}

fn read(fs: &BoundedFs, path: impl AsRef<Path>) -> Vec<u8> {
    fs.bind(path, AccessMode::Read).unwrap().read().unwrap()
}

fn denied<T: std::fmt::Debug>(result: io::Result<T>) {
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn normal_create_read_list_rename_remove_and_limits() {
    let fixture = root();
    let fs = BoundedFs::open(fixture.path()).unwrap();
    write(&fs, "nested/file.txt", b"normal-content");
    assert_eq!(read(&fs, "nested/file.txt"), b"normal-content");
    assert_eq!(
        read(&fs, fixture.path().join("nested/file.txt")),
        b"normal-content"
    );
    let entries = fs.bind("nested", AccessMode::Read).unwrap().list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "file.txt");
    assert!(!entries[0].is_dir);
    let read_only = fs.bind("nested/file.txt", AccessMode::Read).unwrap();
    denied(read_only.atomic_write(b"denied"));
    denied(read_only.remove_file());
    assert_eq!(read_only.read_limited(14).unwrap(), b"normal-content");
    assert_eq!(
        read_only.read_limited(13).unwrap_err().kind(),
        io::ErrorKind::FileTooLarge
    );
    let source = fs.bind("nested/file.txt", AccessMode::Write).unwrap();
    let readonly_target = fs.bind("destination.txt", AccessMode::Read).unwrap();
    denied(source.rename_to(&readonly_target));
    let destination = fs.bind("moved/file.txt", AccessMode::Write).unwrap();
    source.rename_to(&destination).unwrap();
    assert_eq!(destination.read().unwrap(), b"normal-content");
    destination.remove_file().unwrap();
    assert!(!destination.exists().unwrap());
}

#[test]
fn absolute_outside_and_parent_traversal_are_rejected() {
    let fixture = root();
    let outside = root();
    let fs = BoundedFs::open(fixture.path()).unwrap();
    denied(fs.bind(outside.path().join("private.txt"), AccessMode::Write));
    denied(fs.bind("../private.txt", AccessMode::Write));
    denied(fs.bind("nested/../../private.txt", AccessMode::Read));
}

#[test]
fn registry_scopes_tokens_to_session_action_mode_and_exact_targets() {
    let fixture = root();
    let fs = Arc::new(BoundedFs::open(fixture.path()).unwrap());
    write(&fs, "allowed.txt", b"allowed");
    write(&fs, "other.txt", b"other");
    let read_grant = RootGrant::new(fs.clone(), AccessMode::Read);
    let write_grant = RootGrant::new(fs, AccessMode::Write)
        .with_targets(vec![PathBuf::from("allowed.txt")])
        .unwrap();
    let authority =
        DispatchAuthority::register("session-a", "action-a", vec![read_grant, write_grant])
            .unwrap();
    let bind = |token: &str, session: &str, action: &str, path: &str, mode| {
        authorized_path(token, session, action, path, mode)
    };
    denied(bind(
        "model-token",
        "session-a",
        "action-a",
        "allowed.txt",
        AccessMode::Read,
    ));
    denied(bind(
        authority.token(),
        "session-b",
        "action-a",
        "allowed.txt",
        AccessMode::Read,
    ));
    denied(bind(
        authority.token(),
        "session-a",
        "action-b",
        "allowed.txt",
        AccessMode::Read,
    ));
    denied(bind(
        authority.token(),
        "session-a",
        "action-a",
        "other.txt",
        AccessMode::Write,
    ));
    let writable = bind(
        authority.token(),
        "session-a",
        "action-a",
        "allowed.txt",
        AccessMode::Write,
    )
    .unwrap();
    writable.atomic_write(b"updated").unwrap();
    assert_eq!(writable.read().unwrap(), b"updated");
    drop(authority);
    denied(writable.read());
}

#[test]
fn revocation_reaches_retained_paths_derived_directories_and_policy_reads() {
    let fixture = root();
    fs::create_dir(fixture.path().join("sub")).unwrap();
    fs::write(fixture.path().join("sub/file"), "before").unwrap();
    fs::write(fixture.path().join(".biorouterignore"), "*.secret").unwrap();
    let fs = Arc::new(BoundedFs::open(fixture.path()).unwrap());
    let grant = RootGrant::new(fs, AccessMode::Read);
    let authority = DispatchAuthority::register("session", "action", vec![grant.clone()]).unwrap();
    let bound = authorized_path(
        authority.token(),
        "session",
        "action",
        "sub",
        AccessMode::Read,
    )
    .unwrap();
    assert_eq!(bound.read_root_policy().unwrap(), b"*.secret");
    let sub = bound.open_dir().unwrap();
    denied(sub.bind("new", AccessMode::Write));
    let file = sub.bind("file", AccessMode::Read).unwrap();
    assert_eq!(file.read().unwrap(), b"before");
    grant.revoke();
    denied(bound.list());
    denied(bound.read_root_policy());
    denied(file.read());
    denied(sub.bind("file", AccessMode::Read));
}

#[test]
fn empty_exact_target_list_does_not_grant_the_folder() {
    let fixture = root();
    let fs = Arc::new(BoundedFs::open(fixture.path()).unwrap());
    let grant = RootGrant::new(fs, AccessMode::Write)
        .with_targets(vec![])
        .unwrap();
    let authority = DispatchAuthority::register("session", "action", vec![grant]).unwrap();
    denied(authorized_path(
        authority.token(),
        "session",
        "action",
        "new",
        AccessMode::Write,
    ));
}

#[test]
fn exact_directory_target_cannot_become_a_descendant_grant() {
    let fixture = root();
    fs::create_dir(fixture.path().join("context")).unwrap();
    fs::write(fixture.path().join("context/child"), "child").unwrap();
    let fs = Arc::new(BoundedFs::open(fixture.path()).unwrap());
    let grant = RootGrant::new(fs, AccessMode::Read)
        .with_targets(vec![PathBuf::from("context")])
        .unwrap();
    let authority = DispatchAuthority::register("session", "action", vec![grant]).unwrap();
    let bound = authorized_path(
        authority.token(),
        "session",
        "action",
        "context",
        AccessMode::Read,
    )
    .unwrap();
    assert!(bound.is_dir().unwrap());
    assert_eq!(bound.list().unwrap().len(), 1);
    denied(bound.open_dir());
    denied(authorized_path(
        authority.token(),
        "session",
        "action",
        "context/child",
        AccessMode::Read,
    ));
}

#[test]
fn dispatch_aliases_preserve_approved_actual_target_scope_and_lifetime() {
    let workspace = root();
    let external = root();
    fs::write(external.path().join("approved"), "approved").unwrap();
    fs::write(external.path().join("other"), "other").unwrap();
    let actual = external.path().join("approved");
    let alias = workspace.path().join("external-link/approved");
    let grant = RootGrant::new(
        Arc::new(BoundedFs::open(external.path()).unwrap()),
        AccessMode::Read,
    )
    .with_targets(vec![actual.clone()])
    .unwrap();
    let authority = DispatchAuthority::register_aliases(
        "session",
        "action",
        vec![grant],
        vec![(alias.clone(), actual.clone())],
    )
    .unwrap();
    let bound = authorized_path(
        authority.token(),
        "session",
        "action",
        &alias,
        AccessMode::Read,
    )
    .unwrap();
    assert_eq!(bound.read().unwrap(), b"approved");
    assert_eq!(bound.absolute_path(), actual);
    denied(authorized_path(
        authority.token(),
        "session",
        "action",
        &alias,
        AccessMode::Write,
    ));
    denied(authorized_path(
        authority.token(),
        "session",
        "action",
        workspace.path().join("external-link/other"),
        AccessMode::Read,
    ));
    let token = authority.token().to_owned();
    drop(authority);
    denied(bound.read());
    denied(authorized_path(
        &token,
        "session",
        "action",
        &alias,
        AccessMode::Read,
    ));
}

#[test]
fn actual_parent_history_keys_unify_different_approved_root_scopes() {
    let fixture = root();
    fs::create_dir(fixture.path().join("child")).unwrap();
    fs::write(fixture.path().join("child/file"), "existing").unwrap();
    let ancestor = BoundedFs::open(fixture.path()).unwrap();
    let child = ancestor.subdirectory("child").unwrap();
    assert_eq!(
        ancestor
            .bind("child/file", AccessMode::Read)
            .unwrap()
            .try_history_key()
            .unwrap(),
        child
            .bind("file", AccessMode::Read)
            .unwrap()
            .try_history_key()
            .unwrap()
    );
    let new = ancestor.bind("new-parent/file", AccessMode::Write).unwrap();
    assert_eq!(
        new.try_history_key().unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    new.atomic_write(b"new").unwrap();
    let new_parent = ancestor.subdirectory("new-parent").unwrap();
    assert_eq!(
        new.try_history_key().unwrap(),
        new_parent
            .bind("file", AccessMode::Read)
            .unwrap()
            .try_history_key()
            .unwrap()
    );
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    #[test]
    fn ambiguous_windows_names_are_rejected() {
        let fixture = root();
        fs::write(fixture.path().join("credentials.secret"), "private").unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        for alias in [
            "credentials.secret::$DATA",
            "credentials.secret.",
            "credentials.secret ",
        ] {
            denied(fs.bind(alias, AccessMode::Read));
            denied(fs.bind(alias, AccessMode::Write));
        }
    }

    #[test]
    fn short_and_case_aliases_return_the_actual_long_policy_name() {
        let fixture = root();
        let original = fixture.path().join("CredentialsLongName.secret");
        fs::write(&original, "private").unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        let folded = fs
            .bind("credentialslongname.secret", AccessMode::Read)
            .unwrap();
        assert_eq!(
            folded.relative_path(),
            Path::new("CredentialsLongName.secret")
        );
        let wide: Vec<u16> = original.as_os_str().encode_wide().chain(Some(0)).collect();
        let required = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
        assert_ne!(
            required,
            0,
            "GetShortPathNameW failed: {}",
            io::Error::last_os_error()
        );
        let mut buffer = vec![0_u16; required as usize];
        let written = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), required) };
        assert_ne!(
            written,
            0,
            "GetShortPathNameW failed: {}",
            io::Error::last_os_error()
        );
        let short = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..written as usize]));
        let bound = fs
            .bind(short.file_name().unwrap(), AccessMode::Read)
            .unwrap();
        assert_eq!(
            bound.relative_path(),
            Path::new("CredentialsLongName.secret")
        );
        assert_eq!(bound.read().unwrap(), b"private");
        if short == original {
            eprintln!(
                "8.3 aliases disabled on this filesystem; long-name and case-alias checks passed"
            );
        }
    }

    #[test]
    fn retained_windows_directory_handles_block_root_name_substitution() {
        let fixture = root();
        let selected = fixture.path().join("workspace");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file"), "approved").unwrap();
        let fs = BoundedFs::open(&selected).unwrap();
        assert!(fs::rename(&selected, fixture.path().join("moved")).is_err());
        assert_eq!(read(&fs, "file"), b"approved");
        write(&fs, "file", b"edited");
        assert_eq!(read(&fs, "file"), b"edited");
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use biorouter_sandbox::{DockerSandbox, LocalProcessSandbox, SandboxClient, SandboxSpec};
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn private_history_repairs_owned_storage_and_new_publication_is_owner_only() {
        let fixture = root();
        fs::set_permissions(fixture.path(), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir(fixture.path().join("history")).unwrap();
        fs::set_permissions(
            fixture.path().join("history"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::write(fixture.path().join("history/old.json"), "synthetic-old").unwrap();
        fs::set_permissions(
            fixture.path().join("history/old.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        let history = fs.create_private_subdirectory("history").unwrap();
        history
            .bind("old.json", AccessMode::Write)
            .unwrap()
            .protect_owner_private()
            .unwrap();
        history
            .bind("nested/new.json", AccessMode::Write)
            .unwrap()
            .atomic_write_private(b"synthetic-new")
            .unwrap();
        for (path, mode) in [
            ("history", 0o700),
            ("history/old.json", 0o600),
            ("history/nested", 0o700),
            ("history/nested/new.json", 0o600),
        ] {
            assert_eq!(
                fs::metadata(fixture.path().join(path))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                mode,
                "{path}"
            );
        }
        assert_eq!(
            fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
            0o755,
            "unrelated ancestor permissions must stay intact"
        );
        assert_eq!(read(&history, "old.json"), b"synthetic-old");
        assert_eq!(read(&history, "nested/new.json"), b"synthetic-new");
    }

    #[test]
    fn private_storage_never_chmods_a_substituted_link_target() {
        let fixture = root();
        let outside = root();
        fs::write(fixture.path().join("file"), "approved").unwrap();
        fs::write(outside.path().join("canary"), "outside").unwrap();
        fs::set_permissions(
            outside.path().join("canary"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        let bound = fs.bind("file", AccessMode::Write).unwrap();
        fs::remove_file(fixture.path().join("file")).unwrap();
        symlink(outside.path().join("canary"), fixture.path().join("file")).unwrap();
        symlink(outside.path(), fixture.path().join("history")).unwrap();
        assert!(bound.protect_owner_private().is_err());
        assert!(bound.atomic_write_private(b"denied").is_err());
        assert!(fs.create_private_subdirectory("history").is_err());
        assert_eq!(
            fs::metadata(outside.path().join("canary"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
        assert_eq!(fs::read(outside.path().join("canary")).unwrap(), b"outside");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn normal_replacement_preserves_acl_and_private_replacement_removes_it() {
        use std::process::Command;
        let fixture = root();
        let path = fixture.path().join("history.json");
        fs::write(&path, "synthetic-old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Command::new("/bin/chmod")
            .args(["+a", "everyone allow read"])
            .arg(&path)
            .status()
            .unwrap()
            .success());
        let acl = || {
            String::from_utf8(
                Command::new("/bin/ls")
                    .arg("-le")
                    .arg(&path)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
        };
        assert!(acl().contains("everyone allow read"));
        let fs = BoundedFs::open(fixture.path()).unwrap();
        let bound = fs.bind("history.json", AccessMode::Write).unwrap();
        bound.atomic_write(b"normal-update").unwrap();
        assert!(acl().contains("everyone allow read"));
        bound.atomic_write_private(b"private-update").unwrap();
        assert!(!acl().contains("everyone"));
        assert_eq!(read(&fs, "history.json"), b"private-update");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn contained_links_work_without_authorizing_external_or_dangling_links() {
        let fixture = root();
        let outside = root();
        fs::create_dir(fixture.path().join("real")).unwrap();
        fs::write(fixture.path().join("real/file"), "inside").unwrap();
        fs::write(outside.path().join("canary"), "outside").unwrap();
        symlink("real", fixture.path().join("relative")).unwrap();
        symlink(
            fixture.path().join("real/file"),
            fixture.path().join("absolute"),
        )
        .unwrap();
        symlink(outside.path(), fixture.path().join("escape")).unwrap();
        symlink("missing", fixture.path().join("dangling")).unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        assert_eq!(read(&fs, "relative/file"), b"inside");
        assert_eq!(read(&fs, "absolute"), b"inside");
        write(&fs, "relative/new", b"new");
        assert_eq!(read(&fs, "real/new"), b"new");
        assert_eq!(
            fs.bind("relative/file", AccessMode::Read)
                .unwrap()
                .relative_path(),
            Path::new("real/file")
        );
        denied(fs.bind("escape/canary", AccessMode::Read));
        denied(fs.bind("escape/new", AccessMode::Write));
        denied(fs.bind("dangling", AccessMode::Write));
        assert!(!fixture.path().join("missing").exists());
        assert_eq!(fs::read(outside.path().join("canary")).unwrap(), b"outside");
    }

    #[test]
    fn bound_parent_survives_substitution_without_touching_outside_canary() {
        let fixture = root();
        let outside = root();
        fs::create_dir(fixture.path().join("parent")).unwrap();
        fs::write(fixture.path().join("parent/file"), "approved").unwrap();
        fs::write(outside.path().join("file"), "outside-canary").unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        let bound = fs.bind("parent/file", AccessMode::Write).unwrap();
        fs::rename(
            fixture.path().join("parent"),
            fixture.path().join("original-parent"),
        )
        .unwrap();
        symlink(outside.path(), fixture.path().join("parent")).unwrap();
        assert_eq!(bound.read().unwrap(), b"approved");
        bound.atomic_write(b"edited").unwrap();
        assert_eq!(
            fs::read(fixture.path().join("original-parent/file")).unwrap(),
            b"edited"
        );
        assert_eq!(
            fs::read(outside.path().join("file")).unwrap(),
            b"outside-canary"
        );
        denied(fs.bind("parent/file", AccessMode::Write));
    }

    #[test]
    fn root_identity_and_policy_remain_bound_after_root_name_replacement() {
        let container = root();
        let selected = container.path().join("workspace");
        let moved = container.path().join("original");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file"), "approved").unwrap();
        fs::write(selected.join(".biorouterignore"), "*.private").unwrap();
        let original = BoundedFs::open(&selected).unwrap();
        let bound = original.bind("file", AccessMode::Write).unwrap();
        let original_key = bound.history_key();
        fs::rename(&selected, &moved).unwrap();
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file"), "replacement").unwrap();
        fs::write(selected.join(".biorouterignore"), "").unwrap();
        assert_eq!(read(&original, selected.join("file")), b"approved");
        assert_eq!(bound.read_root_policy().unwrap(), b"*.private");
        write(&original, "file", b"edited-original");
        let replacement = BoundedFs::open(&selected).unwrap();
        assert_eq!(read(&replacement, "file"), b"replacement");
        assert_ne!(
            bound.history_key(),
            replacement
                .bind("file", AccessMode::Read)
                .unwrap()
                .history_key()
        );
        let reopened_original = BoundedFs::open(&moved).unwrap();
        assert_eq!(
            original_key,
            reopened_original
                .bind("file", AccessMode::Read)
                .unwrap()
                .history_key()
        );
    }

    #[test]
    fn substitutions_at_final_leaf_and_missing_parent_cannot_follow_links() {
        let fixture = root();
        let outside = root();
        fs::write(fixture.path().join("file"), "approved").unwrap();
        fs::write(outside.path().join("file"), "outside-canary").unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        let bound = fs.bind("file", AccessMode::Write).unwrap();
        fs::remove_file(fixture.path().join("file")).unwrap();
        symlink(outside.path().join("file"), fixture.path().join("file")).unwrap();
        assert!(bound.read().is_err());
        assert!(bound.atomic_write(b"denied").is_err());
        let missing = fs.bind("new-parent/file", AccessMode::Write).unwrap();
        symlink(outside.path(), fixture.path().join("new-parent")).unwrap();
        assert!(missing.atomic_write(b"denied").is_err());
        assert_eq!(
            fs::read(outside.path().join("file")).unwrap(),
            b"outside-canary"
        );
    }

    #[test]
    fn exact_target_grant_does_not_follow_a_retargeted_internal_alias() {
        let fixture = root();
        fs::write(fixture.path().join("allowed"), "allowed").unwrap();
        fs::write(fixture.path().join("other"), "other").unwrap();
        symlink("allowed", fixture.path().join("alias")).unwrap();
        let fs = Arc::new(BoundedFs::open(fixture.path()).unwrap());
        let grant = RootGrant::new(fs, AccessMode::Write)
            .with_targets(vec![PathBuf::from("alias")])
            .unwrap();
        let authority = DispatchAuthority::register("session", "action", vec![grant]).unwrap();
        fs::remove_file(fixture.path().join("alias")).unwrap();
        symlink("other", fixture.path().join("alias")).unwrap();
        denied(authorized_path(
            authority.token(),
            "session",
            "action",
            "alias",
            AccessMode::Write,
        ));
        assert_eq!(fs::read(fixture.path().join("other")).unwrap(), b"other");
    }

    #[test]
    fn atomic_replace_detaches_hardlinks_and_preserves_access_mode() {
        let fixture = root();
        let outside = root();
        fs::write(outside.path().join("canary"), "original").unwrap();
        fs::hard_link(outside.path().join("canary"), fixture.path().join("alias")).unwrap();
        fs::set_permissions(
            fixture.path().join("alias"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let fs = BoundedFs::open(fixture.path()).unwrap();
        write(&fs, "alias", b"changed");
        assert_eq!(read(&fs, "alias"), b"changed");
        assert_eq!(
            fs::read(outside.path().join("canary")).unwrap(),
            b"original"
        );
        assert_eq!(
            fs::metadata(fixture.path().join("alias"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }

    #[test]
    fn fifo_read_is_refused_without_waiting_for_a_writer() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let fixture = root();
        let fifo = CString::new(fixture.path().join("fifo").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let fs = BoundedFs::open(fixture.path()).unwrap();
        denied(fs.bind("fifo", AccessMode::Read).unwrap().read());
    }

    #[tokio::test]
    async fn local_and_docker_host_file_apis_hold_the_original_workspace() {
        let container = root();
        let selected = container.path().join("workspace");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file"), "approved").unwrap();
        let local = LocalProcessSandbox::new(SandboxSpec::new(&selected));
        let docker = DockerSandbox::new(SandboxSpec::new(&selected));
        fs::rename(&selected, container.path().join("original")).unwrap();
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file"), "replacement-canary").unwrap();
        for sandbox in [&local as &dyn SandboxClient, &docker as &dyn SandboxClient] {
            assert_eq!(sandbox.read("file").await.unwrap(), b"approved");
            sandbox.write("new", b"created").await.unwrap();
            assert!(sandbox.list(".").await.unwrap().contains(&"new".to_owned()));
        }
        assert_eq!(
            fs::read(selected.join("file")).unwrap(),
            b"replacement-canary"
        );
        assert!(!selected.join("new").exists());
    }
}
