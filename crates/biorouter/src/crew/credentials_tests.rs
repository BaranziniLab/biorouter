use super::*;
use serial_test::serial;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};
use zeroize::Zeroizing;

fn passphrase(value: &str) -> Zeroizing<String> {
    Zeroizing::new(value.to_owned())
}

fn new_vault() -> (tempfile::TempDir, CredentialVault) {
    let root = tempfile::tempdir().unwrap();
    let vault = CredentialVault::new(root.path().to_path_buf());
    (root, vault)
}

fn read_secret(vault: &CredentialVault, id: &str) -> String {
    vault
        .read(id, || {
            Err(anyhow::anyhow!("legacy fallback was unexpectedly used"))
        })
        .unwrap()
        .to_string()
}

#[test]
#[serial(crew_credentials)]
fn vault_initializes_unlocks_locks_and_restarts_without_keyring_fallback() {
    let (root, vault) = new_vault();
    let before = {
        let _keyring = env_lock::lock_env([
            ("BIOROUTER_DISABLE_KEYRING", None::<&str>),
            ("BIOROUTER_DEV_PROFILE_ROOT", None),
        ]);
        vault.status().unwrap()
    };
    assert_eq!(before.backend, "keyring");
    assert!(!before.initialized);
    assert!(!before.locked);

    vault
        .init(passphrase("correct horse battery staple"))
        .unwrap();
    vault
        .write("crew-token", "synthetic-secret", || {
            Err(anyhow::anyhow!("legacy fallback was unexpectedly used"))
        })
        .unwrap();
    assert_eq!(read_secret(&vault, "crew-token"), "synthetic-secret");
    assert_eq!(vault.status().unwrap().backend, "encrypted_vault");

    vault.lock().unwrap();
    assert!(vault
        .read("crew-token", || Ok(Zeroizing::new("fallback".into())))
        .is_err());
    assert!(vault.unlock(passphrase("wrong passphrase")).is_err());
    vault
        .unlock(passphrase("correct horse battery staple"))
        .unwrap();

    let restarted = CredentialVault::new(root.path().to_path_buf());
    let status = restarted.status().unwrap();
    assert!(status.initialized && status.locked);
    assert!(restarted
        .read("crew-token", || Ok(Zeroizing::new("fallback".into())))
        .is_err());
    restarted
        .unlock(passphrase("correct horse battery staple"))
        .unwrap();
    assert_eq!(read_secret(&restarted, "crew-token"), "synthetic-secret");
}

#[test]
#[serial(crew_credentials)]
fn vault_init_refuses_plaintext_and_prior_legacy_use() {
    let (root, vault) = new_vault();
    fs::write(root.path().join("credentials"), b"legacy-marker").unwrap();
    assert!(vault.init(passphrase("passphrase")).is_err());

    let (root, vault) = new_vault();
    let value = vault
        .read("legacy-id", || Ok(Zeroizing::new("legacy-value".into())))
        .unwrap();
    assert_eq!(&*value, "legacy-value");
    assert!(vault.init(passphrase("passphrase")).is_err());
    assert!(!root.path().join("credential-backend.json").exists());
}

#[test]
#[serial(crew_credentials)]
fn incomplete_or_legacy_backend_selection_never_falls_back() {
    let (root, vault) = new_vault();
    fs::write(
        root.path().join("credential-backend.json"),
        br#"{"version":1,"backend":"encrypted_vault"}"#,
    )
    .unwrap();
    assert!(vault.status().is_err());

    let (root, vault) = new_vault();
    fs::write(root.path().join("credential-vault.json"), b"{}").unwrap();
    assert!(vault.status().is_err());

    let (root, vault) = new_vault();
    fs::write(
        root.path().join("credential-backend.json"),
        br#"{"version":1,"backend":"keyring"}"#,
    )
    .unwrap();
    fs::write(root.path().join("credential-vault.json"), b"{}").unwrap();
    assert!(vault.status().is_err());
}

#[test]
#[serial(crew_credentials)]
#[cfg(unix)]
fn vault_rejects_corruption_profile_mismatch_and_symlinked_files() {
    let (root, vault) = new_vault();
    vault.init(passphrase("passphrase")).unwrap();
    vault.write("id", "value", || Ok(())).unwrap();
    let original = fs::read(root.path().join("credential-vault.json")).unwrap();

    let mut envelope: serde_json::Value = serde_json::from_slice(&original).unwrap();
    envelope["ciphertext"] = serde_json::Value::String("zz".into());
    fs::write(
        root.path().join("credential-vault.json"),
        serde_json::to_vec(&envelope).unwrap(),
    )
    .unwrap();
    let restarted = CredentialVault::new(root.path().to_path_buf());
    assert!(restarted.status().is_err());

    let (root, vault) = new_vault();
    vault.init(passphrase("passphrase")).unwrap();
    vault.write("id", "value", || Ok(())).unwrap();
    let real_vault = root.path().join("credential-vault.json");
    let saved = root.path().join("saved-vault.json");
    fs::rename(&real_vault, &saved).unwrap();
    symlink(&saved, &real_vault).unwrap();
    assert!(CredentialVault::new(root.path().to_path_buf())
        .status()
        .is_err());

    let (root, _vault) = new_vault();
    let real_root = root.path().join("real");
    fs::create_dir(&real_root).unwrap();
    let linked_root = root.path().join("linked");
    symlink(&real_root, &linked_root).unwrap();
    assert!(CredentialVault::new(linked_root)
        .init(passphrase("passphrase"))
        .is_err());
}

#[test]
#[serial(crew_credentials)]
fn vault_write_failure_preserves_old_bytes_and_nonce_changes_on_success() {
    let (root, vault) = new_vault();
    vault.init(passphrase("passphrase")).unwrap();
    vault.write("id", "old", || Ok(())).unwrap();
    let path = root.path().join("credential-vault.json");
    let first = fs::read(&path).unwrap();
    let first_nonce = serde_json::from_slice::<serde_json::Value>(&first).unwrap()["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(vault
        .write("id", &"x".repeat(16 * 1024 + 1), || Ok(()))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), first);

    vault.write("id", "new", || Ok(())).unwrap();
    let second = fs::read(&path).unwrap();
    let second_envelope = serde_json::from_slice::<serde_json::Value>(&second).unwrap();
    let second_nonce = second_envelope["nonce"].as_str().unwrap();
    assert_ne!(first_nonce, second_nonce);
    assert_eq!(read_secret(&vault, "id"), "new");
}

#[test]
#[serial(crew_credentials)]
fn binary_credential_map_rejects_truncated_duplicate_and_oversized_inputs() {
    assert!(decode_contents(&[1, 0, 0, 0]).is_err());

    let mut duplicate = Vec::new();
    duplicate.extend_from_slice(&1u32.to_le_bytes());
    duplicate.extend_from_slice(&2u32.to_le_bytes());
    for value in [b"a".as_slice(), b"b".as_slice()] {
        duplicate.extend_from_slice(&1u32.to_le_bytes());
        duplicate.extend_from_slice(b"x");
        duplicate.extend_from_slice(&(value.len() as u32).to_le_bytes());
        duplicate.extend_from_slice(value);
    }
    assert!(decode_contents(&duplicate).is_err());

    let mut oversized = Vec::new();
    oversized.extend_from_slice(&1u32.to_le_bytes());
    oversized.extend_from_slice(&((MAX_ENTRIES + 1) as u32).to_le_bytes());
    assert!(decode_contents(&oversized).is_err());
}

#[test]
#[serial(crew_credentials)]
fn concurrent_initialization_has_one_winner_and_one_persisted_backend() {
    let root = tempfile::tempdir().unwrap();
    let first = Arc::new(CredentialVault::new(root.path().to_path_buf()));
    let second = first.clone();
    let barrier = Arc::new(Barrier::new(2));
    let one = barrier.clone();
    let a = thread::spawn(move || {
        one.wait();
        first.init(passphrase("passphrase"))
    });
    let two = barrier.clone();
    let b = thread::spawn(move || {
        two.wait();
        second.init(passphrase("passphrase"))
    });
    let results = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        CredentialVault::new(root.path().to_path_buf())
            .status()
            .unwrap()
            .initialized
    );
}

#[test]
#[serial(crew_credentials)]
fn profile_change_invalidates_an_unlocked_vault() {
    let (_root, vault) = new_vault();
    vault.init(passphrase("passphrase")).unwrap();
    vault.write("id", "value", || Ok(())).unwrap();
    let mut state = vault.state().unwrap();
    state.unlocked.as_mut().unwrap().profile_identity =
        "33333333-3333-4333-8333-333333333333".into();
    drop(state);
    assert!(vault
        .read("id", || Ok(Zeroizing::new("fallback".into())))
        .is_err());
}

/// T-49: a development profile with the keyring disabled keeps keys as files, and status says
/// so; it never claims the system keychain.
#[test]
#[serial(crew_credentials)]
fn status_names_the_store_the_keys_are_really_in() {
    let (_root, vault) = new_vault();
    let profile = tempfile::tempdir().unwrap();
    let profile_root = profile.path().to_str().unwrap().to_owned();
    let file_mode = [
        ("BIOROUTER_DISABLE_KEYRING", Some("true")),
        ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_root.as_str())),
    ];
    {
        let _files = env_lock::lock_env(file_mode);
        let status = vault.status().unwrap();
        assert_eq!(status.backend, "file");
        assert!(!status.initialized && !status.locked);
    }
    {
        // Disabling the keyring without an absolute development profile is not file mode.
        let _relative = env_lock::lock_env([
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
            ("BIOROUTER_DEV_PROFILE_ROOT", Some("relative/profile")),
        ]);
        assert_eq!(vault.status().unwrap().backend, "keyring");
    }
}
