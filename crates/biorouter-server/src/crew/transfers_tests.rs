use super::*;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

fn private_root() -> tempfile::TempDir {
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
fn corrupt_partial_size_symlink_and_hardlink_are_rejected_before_resume() {
    let root = private_root();
    let part = root.path().join("partial");
    fs::write(&part, b"four").unwrap();
    let file = fs::File::open(&part).unwrap();
    assert!(validate_partial(&file, 3).is_err());
    drop(file);

    let hardlink = root.path().join("partial-hardlink");
    fs::hard_link(&part, &hardlink).unwrap();
    let file = fs::File::open(&part).unwrap();
    assert_eq!(file.metadata().unwrap().nlink(), 2);
    assert!(validate_partial(&file, 4).is_err());
    drop(file);

    let link = root.path().join("partial-link");
    symlink(&part, &link).unwrap();
    assert!(local_files::select(&link, Direction::Upload, false).is_err());
}

#[tokio::test]
async fn startup_recovery_marks_unfinished_publication_unconfirmed() {
    let root = private_root();
    let id = "0123456789abcdef0123456789abcdef";
    let other_id = "fedcba9876543210fedcba9876543210";
    let receipt = json!({
        id: {
            "id": id,
            "request_id": "request",
            "connection_id": "connection",
            "channel_id": "channel",
            "direction": "download",
            "name": "report.csv",
            "size": 4,
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "offset": 4,
            "blob_id": "blob",
            "state": "publishing",
            "error": null,
            "binding": "binding",
            "intent": "intent",
            "destination_identity": null
        },
        other_id: {
            "id": other_id,
            "request_id": "request-2",
            "connection_id": "connection",
            "channel_id": "channel",
            "direction": "download",
            "name": "report-2.csv",
            "size": 4,
            "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "offset": 4,
            "blob_id": "blob-2",
            "state": "publication_unconfirmed",
            "error": null,
            "binding": "binding",
            "intent": "intent",
            "destination_identity": null
        }
    });
    fs::write(
        root.path().join("receipts.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let service = TransferService::open(root.path()).unwrap();
    let state = service.state.lock().await;
    assert_eq!(state.receipts[id].state, "publication_unconfirmed");
    assert!(state.receipts[id].error.is_none());
    assert_eq!(state.receipts[other_id].state, "publication_unconfirmed");
    assert!(state.receipts[other_id].error.is_none());
}

#[test]
fn startup_rejects_malformed_receipts_without_constructing_a_service() {
    let root = private_root();
    fs::write(root.path().join("receipts.json"), b"not-json").unwrap();
    assert!(TransferService::open(root.path()).is_err());

    let root = private_root();
    fs::write(
        root.path().join("receipts.json"),
        br#"{"0123456789abcdef0123456789abcdef":{"id":"bad"}}"#,
    )
    .unwrap();
    assert!(TransferService::open(root.path()).is_err());
}

#[test]
fn private_partial_permissions_are_required_for_cleanup() {
    let root = private_root();
    let path = root.path().join("partial");
    fs::write(&path, b"partial").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let file = fs::File::open(&path).unwrap();
    assert!(validate_cleanup_partial(&file, 7).is_err());
}
