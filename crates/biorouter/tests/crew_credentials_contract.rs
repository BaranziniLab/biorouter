use biorouter::crew::CrewManager;
use serde_json::json;
use zeroize::Zeroizing;

fn passphrase(value: &str) -> Zeroizing<String> {
    Zeroizing::new(value.to_owned())
}

#[tokio::test]
async fn manager_vault_lifecycle_is_explicit_and_restart_locked() {
    let root = tempfile::tempdir().unwrap();
    let manager = CrewManager::new(root.path().to_path_buf()).unwrap();
    let status = manager.credential_status().await.unwrap();
    assert_eq!(status.backend, "keyring");
    assert!(!status.initialized);

    manager
        .init_vault(passphrase("correct horse battery staple"))
        .await
        .unwrap();
    let status = manager.credential_status().await.unwrap();
    assert_eq!(status.backend, "encrypted_vault");
    assert!(status.initialized);
    assert!(!status.locked);

    manager.lock_vault().await.unwrap();
    assert!(manager
        .unlock_vault(passphrase("wrong passphrase"))
        .await
        .is_err());
    assert!(manager.credential_status().await.unwrap().locked);
    manager
        .unlock_vault(passphrase("correct horse battery staple"))
        .await
        .unwrap();
    manager.lock_vault().await.unwrap();

    let restarted = CrewManager::new(root.path().to_path_buf()).unwrap();
    let status = restarted.credential_status().await.unwrap();
    assert_eq!(status.backend, "encrypted_vault");
    assert!(status.initialized && status.locked);
}

#[tokio::test]
async fn manager_vault_init_refuses_a_profile_with_existing_connection_registry() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("connections.json"),
        serde_json::to_vec(&json!({
            "connections": [{
                "id": "connection-1",
                "name": "fixture",
                "ssh_target": "fixture@example.test",
                "port": 22,
                "identity_file": null,
                "proxy_jump": null,
                "socket_path": "/tmp/fixture.sock",
                "owner_uid": 1000,
                "workspace_id": "workspace-1",
                "workspace_public_key": "synthetic-public-key",
                "remote_root": null,
                "remote_execution": false,
                "cluster_connection_id": "cluster-1",
                "mode": "private",
                "policy_epoch": 1,
                "status": "connected",
                "last_error": null,
                "device_id": "synthetic-device",
                "public_key": "synthetic-device-key"
            }],
            "scopes": {},
            "pending_device": null,
            "completed_preparations": {}
        }))
        .unwrap(),
    )
    .unwrap();
    let manager = CrewManager::new(root.path().to_path_buf()).unwrap();
    let error = manager
        .init_vault(passphrase("passphrase"))
        .await
        .expect_err("existing identity registry must block vault initialization");
    assert!(error.to_string().contains("fresh Crew profile"));
    assert!(!root.path().join("credential-vault.json").exists());
}
