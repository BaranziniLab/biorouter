#![cfg(unix)]

use super::*;
use std::fs;
use std::future::Future;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

fn private_root() -> tempfile::TempDir {
    #[cfg(target_os = "macos")]
    let root = tempfile::tempdir_in("/private/tmp").unwrap();
    #[cfg(not(target_os = "macos"))]
    let root = tempfile::tempdir().unwrap();
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

#[test]
fn file_request_expected_mode_defaults_for_legacy_and_round_trips() {
    let legacy: FileRequest = serde_json::from_value(json!({
        "purpose":"transfer",
        "connection_id":"connection",
        "channel_id":"channel",
        "direction":"upload",
        "path":"/tmp/fixture.txt",
        "overwrite":false,
        "blob_id":null,
        "transfer_id":null
    }))
    .unwrap();
    assert!(legacy.expected_mode.is_none());

    let private: FileRequest = serde_json::from_value(json!({
        "expected_mode":"private",
        "purpose":"transfer",
        "connection_id":"connection",
        "channel_id":"channel",
        "direction":"upload",
        "path":"/tmp/fixture.txt",
        "overwrite":false,
        "blob_id":null,
        "transfer_id":null
    }))
    .unwrap();
    assert_eq!(
        private.expected_mode,
        Some(biorouter::crew::ClusterMode::Private)
    );
}

#[tokio::test]
async fn cleanup_selection_uses_receipt_binding_without_requiring_a_live_connection_mode() {
    let root = private_root();
    let service = TransferService::open(root.path()).unwrap();
    let transfer_id = "0123456789abcdef0123456789abcdef";
    service.state.lock().await.receipts.insert(
        transfer_id.into(),
        Receipt {
            id: transfer_id.into(),
            request_id: "request".into(),
            connection_id: "removed-connection".into(),
            channel_id: "channel".into(),
            direction: Direction::Download,
            name: "fixture.txt".into(),
            size: 4,
            sha256: "a".repeat(64),
            offset: 4,
            blob_id: Some("blob".into()),
            state: "needs_file_selection".into(),
            error: None,
            binding: "receipt-binding".into(),
            intent: "intent".into(),
            local_selection: String::new(),
            destination_identity: None,
        },
    );

    let binding = service
        .selection_binding(&FileRequest {
            expected_mode: Some(biorouter::crew::ClusterMode::Private),
            purpose: FilePurpose::Cleanup,
            connection_id: "removed-connection".into(),
            channel_id: "channel".into(),
            direction: Direction::Download,
            path: root.path().join("fixture.txt"),
            overwrite: false,
            blob_id: Some("blob".into()),
            transfer_id: Some(transfer_id.into()),
            request_id: None,
        })
        .await
        .unwrap();
    assert_eq!(binding, "receipt-binding");
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

#[tokio::test]
async fn launch_returns_starting_receipt_and_reserves_active_before_worker_progress() {
    let root = private_root();
    let service = Arc::new(TransferService::open(root.path()).unwrap());
    let source = root.path().join("fixture.txt");
    fs::write(&source, b"fixture").unwrap();
    let selection = local_files::select(&source, Direction::Upload, false).unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let receipt = Receipt {
        id: id.into(),
        request_id: "request".into(),
        connection_id: "connection".into(),
        channel_id: "channel".into(),
        direction: Direction::Upload,
        name: "fixture.txt".into(),
        size: 0,
        sha256: String::new(),
        offset: 0,
        blob_id: None,
        state: "needs_file_selection".into(),
        error: Some("stale resume error".into()),
        binding: "binding".into(),
        intent: "intent".into(),
        local_selection: String::new(),
        destination_identity: None,
    };

    let mut state = service.state.lock().await;
    let accepted = service.launch(&mut state, receipt, selection).unwrap();
    assert_eq!(accepted.state, "starting");
    assert!(accepted.error.is_none());
    assert!(state.active.contains_key(id));
    assert_eq!(state.receipts[id].state, "starting");
    assert!(state.receipts[id].error.is_none());
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("receipts.json")).unwrap()).unwrap();
    assert_eq!(persisted[id]["state"], "starting");
    assert!(persisted[id]["error"].is_null());
    drop(state);

    let mut resume = Box::pin(service.resume(id, "not-the-capability"));
    let waker: Waker = Waker::from(Arc::new(NoopWaker));
    let mut context = Context::from_waker(&waker);
    let result = match resume.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("active resume check unexpectedly yielded"),
    };
    let error = match result {
        Ok(_) => panic!("an active transfer must reject a concurrent resume"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("active"));

    let state = service.state.try_lock().unwrap();
    assert!(state.active.contains_key(id));
    assert_eq!(state.receipts[id].state, "starting");
    assert!(state.receipts[id].error.is_none());
    state.active[id].cancel();
}
