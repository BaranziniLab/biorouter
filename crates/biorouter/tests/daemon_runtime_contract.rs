#![cfg(unix)]

use biorouter::daemon_runtime::{
    descriptor_path, private_directory, profile_identity, profile_record, read_descriptor,
    read_private, write_private, Endpoint, ProfileIdentity, RuntimeOwner,
};
use serde_json::json;
use std::{
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt,
    os::unix::fs::{symlink, PermissionsExt},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn profile() -> ProfileIdentity {
    ProfileIdentity {
        version: 1,
        profile_id: "11111111-1111-4111-8111-111111111111".into(),
        config_dir: "/tmp/biorouter-runtime-test".into(),
    }
}

fn secure_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn private_directory_requires_absolute_owned_0700_real_directory() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    private_directory(&private).unwrap();
    assert_eq!(
        fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );

    let relative = Path::new("relative-runtime");
    assert!(private_directory(relative).is_err());

    let public = root.path().join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(private_directory(&public).is_err());

    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = root.path().join("private-link");
    symlink(&target, &link).unwrap();
    assert!(private_directory(&link).is_err());
}

#[test]
fn read_private_rejects_symlink_nonregular_public_and_hardlinked_records() {
    let root = tempfile::tempdir().unwrap();
    let record = root.path().join("record.json");
    write_private(&record, &profile()).unwrap();
    assert_eq!(
        read_private::<ProfileIdentity>(&record).unwrap().profile_id,
        profile().profile_id
    );

    let symlink_path = root.path().join("record-link.json");
    symlink(&record, &symlink_path).unwrap();
    assert!(read_private::<ProfileIdentity>(&symlink_path).is_err());

    let directory = root.path().join("record-dir");
    fs::create_dir(&directory).unwrap();
    assert!(read_private::<ProfileIdentity>(&directory).is_err());

    fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_private::<ProfileIdentity>(&record).is_err());
    fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();

    let hardlink = root.path().join("record-hardlink.json");
    fs::hard_link(&record, &hardlink).unwrap();
    assert!(read_private::<ProfileIdentity>(&hardlink).is_err());
}

#[test]
fn read_private_refuses_a_fifo_without_blocking() {
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("record.fifo");
    let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
    let started = Instant::now();
    assert!(read_private::<ProfileIdentity>(&fifo).is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn read_private_rejects_oversized_records_and_write_private_roundtrips_atomically() {
    let root = tempfile::tempdir().unwrap();
    let oversized = root.path().join("oversized.json");
    secure_file(&oversized, &vec![b'x'; 16 * 1024 + 1]);
    assert!(read_private::<serde_json::Value>(&oversized).is_err());

    let output = root.path().join("written.json");
    write_private(&output, &json!({"kind":"runtime","value":7})).unwrap();
    assert_eq!(
        read_private::<serde_json::Value>(&output).unwrap()["value"],
        7
    );
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn descriptor_identity_excludes_endpoint_and_secret_material() {
    let descriptor = biorouter::daemon_runtime::Descriptor {
        version: 1,
        profile_id: profile().profile_id,
        instance_id: "22222222-2222-4222-8222-222222222222".into(),
        pid: 42,
        endpoint: Endpoint::Unix {
            path: "/tmp/private/daemon.sock".into(),
        },
        api_secret: "secret-that-must-not-be-in-identity".into(),
        user_action_installed: true,
    };
    let identity = descriptor.identity();
    assert_eq!(identity.version, 1);
    assert_eq!(identity.profile_id, "11111111-1111-4111-8111-111111111111");
    assert_eq!(identity.instance_id, "22222222-2222-4222-8222-222222222222");
    assert_eq!(identity.pid, 42);
    assert!(identity.user_action_installed);
    assert!(!serde_json::to_string(&identity)
        .unwrap()
        .contains("secret-that-must-not-be-in-identity"));
}

#[test]
fn runtime_owner_child() {
    if std::env::var_os("BIOROUTER_RUNTIME_CHILD").is_none() {
        return;
    }

    let profile = profile_record().unwrap();
    assert_eq!(profile.profile_id, profile_identity().unwrap());
    let reread = profile_record().unwrap();
    assert_eq!(profile.profile_id, reread.profile_id);
    assert_eq!(profile.config_dir, reread.config_dir);

    let owner = RuntimeOwner::acquire("0123456789abcdef0123456789abcdef".into(), true).unwrap();
    assert!(RuntimeOwner::acquire("fedcba9876543210fedcba9876543210".into(), false).is_err());
    owner.publish().unwrap();
    let published = read_descriptor().unwrap();
    assert_eq!(published.instance_id, owner.descriptor.instance_id);
    assert_eq!(published.profile_id, profile.profile_id);

    let mut foreign_profile = owner.descriptor.clone();
    foreign_profile.profile_id = "33333333-3333-4333-8333-333333333333".into();
    write_private(&descriptor_path(), &foreign_profile).unwrap();
    assert!(read_descriptor().is_err());

    let mut stale_instance = owner.descriptor.clone();
    stale_instance.instance_id = "44444444-4444-4444-8444-444444444444".into();
    write_private(&descriptor_path(), &stale_instance).unwrap();
    let parsed_stale = read_descriptor().unwrap();
    assert_eq!(parsed_stale.profile_id, owner.descriptor.profile_id);
    assert!(parsed_stale.identity() != owner.descriptor.identity());

    let mut foreign_endpoint = owner.descriptor.clone();
    foreign_endpoint.endpoint = Endpoint::Unix {
        path: std::path::PathBuf::from("/tmp/foreign-daemon.sock"),
    };
    write_private(&descriptor_path(), &foreign_endpoint).unwrap();
    assert!(read_descriptor().is_err());

    owner.publish().unwrap();
    let mut newer_descriptor = owner.descriptor.clone();
    newer_descriptor.instance_id = "55555555-5555-4555-8555-555555555555".into();
    write_private(&descriptor_path(), &newer_descriptor).unwrap();
    drop(owner);
    assert_eq!(
        read_descriptor().unwrap().instance_id,
        newer_descriptor.instance_id
    );

    let replacement =
        RuntimeOwner::acquire("abcdef0123456789abcdef0123456789".into(), false).unwrap();
    replacement.publish().unwrap();
    drop(replacement);
    assert!(read_descriptor().is_err());
}

#[test]
fn profile_record_and_runtime_owner_are_process_isolated() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("runtime_owner_child")
        .arg("--nocapture")
        .env("BIOROUTER_PATH_ROOT", root.path())
        .env("BIOROUTER_RUNTIME_CHILD", "1")
        .env_remove("BIOROUTER_DEV_PROFILE_ROOT")
        .env_remove("BIOROUTER_DISABLE_KEYRING")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "runtime child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
