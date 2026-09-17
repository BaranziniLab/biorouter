use biorouter::config::Config;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn fixture() -> (tempfile::TempDir, Config) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::new_with_file_secrets(
        directory.path().join("config.yaml"),
        directory.path().join("secrets.yaml"),
    )
    .unwrap();
    config
        .set_param(
            "extensions",
            json!({ "target": extension("target", &["ISSUE327_ONLY"]) }),
        )
        .unwrap();
    config
        .set_secret("ISSUE327_ONLY", &"synthetic-only")
        .unwrap();
    (directory, config)
}

fn extension(name: &str, keys: &[&str]) -> Value {
    json!({ "name": name, "type": "stdio", "cmd": "unused", "args": [], "enabled": false, "env_keys": keys })
}

#[test]
fn credential_purge_deletes_only_reviewed_unshared_saved_values() {
    let (_directory, config) = fixture();
    config
        .set_secret("ISSUE327_UNRELATED", &"keep-synthetic")
        .unwrap();
    let references = BTreeMap::new();
    let before = config
        .extension_credentials("target", &references, None)
        .unwrap();
    assert!(before[0].stored);
    let after = config
        .extension_credentials("target", &references, Some(&["ISSUE327_ONLY".into()]))
        .unwrap();
    assert!(!after[0].stored);
    assert!(config.get_secret::<String>("ISSUE327_ONLY").is_err());
    assert_eq!(
        config.get_secret::<String>("ISSUE327_UNRELATED").unwrap(),
        "keep-synthetic"
    );
    let extensions: Value = config.get_param("extensions").unwrap();
    assert_eq!(extensions["target"]["env_keys"][0], "ISSUE327_ONLY");
    // Missing saved values are an idempotent no-op, not a failure or a write.
    assert!(config
        .extension_credentials("target", &references, Some(&["ISSUE327_ONLY".into()]))
        .is_ok());
}

#[test]
fn credential_purge_rejects_arbitrary_keys_without_partial_deletion() {
    let (_directory, config) = fixture();
    config
        .set_secret("ISSUE327_UNRELATED", &"keep-synthetic")
        .unwrap();
    assert!(config
        .extension_credentials(
            "target",
            &BTreeMap::new(),
            Some(&["ISSUE327_ONLY".into(), "ISSUE327_UNRELATED".into()])
        )
        .is_err());
    assert!(config.get_secret::<String>("ISSUE327_ONLY").is_ok());
    assert!(config.get_secret::<String>("ISSUE327_UNRELATED").is_ok());
}

#[test]
fn credential_purge_rechecks_disabled_extension_sharing_after_review() {
    let (_directory, config) = fixture();
    assert!(config
        .extension_credentials("target", &BTreeMap::new(), None)
        .unwrap()[0]
        .used_by
        .is_empty());
    config
        .set_param(
            "extensions",
            json!({
                "target": extension("target", &["ISSUE327_ONLY"]),
                "other": extension("other", &["issue327_only"])
            }),
        )
        .unwrap();
    assert!(config
        .extension_credentials("target", &BTreeMap::new(), Some(&["ISSUE327_ONLY".into()]))
        .is_err());
    assert!(config.get_secret::<String>("ISSUE327_ONLY").is_ok());
}

#[test]
fn credential_purge_rejects_removed_reference_after_review() {
    let (_directory, config) = fixture();
    config
        .extension_credentials("target", &BTreeMap::new(), None)
        .unwrap();
    config
        .set_param("extensions", json!({"target": extension("target", &[])}))
        .unwrap();
    assert!(config
        .extension_credentials("target", &BTreeMap::new(), Some(&["ISSUE327_ONLY".into()]))
        .is_err());
    assert!(config.get_secret::<String>("ISSUE327_ONLY").is_ok());
}

#[test]
fn credential_purge_protects_provider_and_application_keys() {
    let (_directory, config) = fixture();
    let references = BTreeMap::from([("ISSUE327_ONLY".into(), vec!["Provider: synthetic".into()])]);
    assert!(config
        .extension_credentials("target", &references, Some(&["ISSUE327_ONLY".into()]))
        .is_err());
    config.set_param("extensions", json!({"target": extension("target", &["OLLAMA_HOST", "BIOROUTER_PRIVACY_TIERS", "BIOROUTER_PRIVACY_MIXING_POLICY"])})).unwrap();
    for credential in config
        .extension_credentials("target", &BTreeMap::new(), None)
        .unwrap()
    {
        assert!(!credential.used_by.is_empty());
        assert!(config
            .extension_credentials("target", &BTreeMap::new(), Some(&[credential.key]))
            .is_err());
    }
}

#[test]
fn credential_purge_fails_closed_on_malformed_other_extension_or_file() {
    let (directory, config) = fixture();
    config.set_param("extensions", json!({"target": extension("target", &["ISSUE327_ONLY"]), "broken": {"env_keys": ["ISSUE327_ONLY"]}})).unwrap();
    assert!(config
        .extension_credentials("target", &BTreeMap::new(), Some(&["ISSUE327_ONLY".into()]))
        .is_err());
    std::fs::write(directory.path().join("config.yaml"), "extensions: [broken").unwrap();
    assert!(config
        .extension_credentials("target", &BTreeMap::new(), Some(&["ISSUE327_ONLY".into()]))
        .is_err());
    assert!(config.get_secret::<String>("ISSUE327_ONLY").is_ok());
}

#[test]
fn credential_purge_preserves_secrets_saved_after_cache_was_loaded() {
    let (directory, config) = fixture();
    config
        .extension_credentials("target", &BTreeMap::new(), None)
        .unwrap();
    let other_process = Config::new_with_file_secrets(
        directory.path().join("config.yaml"),
        directory.path().join("secrets.yaml"),
    )
    .unwrap();
    other_process
        .set_secret("ISSUE327_NEW", &"synthetic-new")
        .unwrap();
    config
        .extension_credentials("target", &BTreeMap::new(), Some(&["ISSUE327_ONLY".into()]))
        .unwrap();
    assert_eq!(
        config.get_secret::<String>("ISSUE327_NEW").unwrap(),
        "synthetic-new"
    );
}

#[test]
fn credential_purge_protects_streamable_http_extension_references() {
    let (_directory, config) = fixture();
    config
        .set_param(
            "extensions",
            json!({
                "target": extension("target", &["ISSUE327_ONLY"]),
                "http": {
                    "name": "http", "type": "streamable_http", "uri": "http://localhost/unused",
                    "enabled": false, "description": "", "env_keys": ["ISSUE327_ONLY"]
                }
            }),
        )
        .unwrap();
    let review = config
        .extension_credentials("target", &BTreeMap::new(), None)
        .unwrap();
    assert_eq!(review[0].used_by, vec!["Extension: http"]);
    assert!(config
        .extension_credentials("target", &BTreeMap::new(), Some(&["ISSUE327_ONLY".into()]))
        .is_err());
}
