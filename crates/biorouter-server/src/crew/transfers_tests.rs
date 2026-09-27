#![cfg(unix)]

use super::*;
use std::fs;
use std::future::Future;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

fn private_root() -> tempfile::TempDir {
    let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
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
            destination_selection: None,
            initial_target: None,
        },
    );

    let binding = service
        .selection_binding(&FileRequest {
            approval_pending: false,
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

#[tokio::test]
async fn pending_download_capability_cannot_be_consumed_by_start_or_resume_gate() {
    let root = private_root();
    let service = TransferService::open(root.path()).unwrap();
    let receipt = Receipt {
        id: "55555555555555555555555555555555".into(),
        request_id: "pending-request".into(),
        connection_id: "pending-connection".into(),
        channel_id: "pending-channel".into(),
        direction: Direction::Download,
        name: "pending.bin".into(),
        size: 4,
        sha256: "a".repeat(64),
        offset: 0,
        blob_id: Some("pending-blob".into()),
        state: "needs_file_selection".into(),
        error: None,
        binding: "pending-binding".into(),
        intent: "pending-intent".into(),
        local_selection: String::new(),
        destination_identity: None,
        destination_selection: None,
        initial_target: None,
    };
    let mut state = service.state.lock().await;
    for resuming in [false, true] {
        let destination = root.path().join(if resuming {
            "pending-resume.bin"
        } else {
            "pending-start.bin"
        });
        let selection = local_files::select(&destination, Direction::Download, false).unwrap();
        state.capabilities.insert(
            format!("pending-capability-{resuming}"),
            Capability {
                approval_pending: true,
                purpose: FilePurpose::Transfer,
                selection,
                connection_id: receipt.connection_id.clone(),
                channel_id: receipt.channel_id.clone(),
                blob_id: receipt.blob_id.clone(),
                transfer_id: resuming.then(|| receipt.id.clone()),
                expires: Instant::now() + Duration::from_secs(300),
                binding: receipt.binding.clone(),
                request_id: Some(receipt.request_id.clone()),
                replay_receipt_id: None,
            },
        );
        let capability = format!("pending-capability-{resuming}");
        let error = match TransferService::take_file(
            &mut state,
            &capability,
            &receipt,
            resuming,
            FilePurpose::Transfer,
        ) {
            Ok(_) => panic!("a pending capability must not be consumed"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Local file approval"));
    }
    assert!(state.capabilities.is_empty());
}

#[tokio::test]
async fn expired_capability_cannot_be_consumed_and_does_not_receive_a_new_ttl() {
    let root = private_root();
    let service = TransferService::open(root.path()).unwrap();
    let destination = root.path().join("expired.bin");
    let selection = local_files::select(&destination, Direction::Download, false).unwrap();
    let receipt = Receipt {
        id: "99999999999999999999999999999999".into(),
        request_id: "expired-request".into(),
        connection_id: "expired-connection".into(),
        channel_id: "expired-channel".into(),
        direction: Direction::Download,
        name: "expired.bin".into(),
        size: 0,
        sha256: "c".repeat(64),
        offset: 0,
        blob_id: Some("expired-blob".into()),
        state: "needs_file_selection".into(),
        error: None,
        binding: "expired-binding".into(),
        intent: "expired-intent".into(),
        local_selection: String::new(),
        destination_identity: None,
        destination_selection: None,
        initial_target: None,
    };
    let expired_at = Instant::now() - Duration::from_secs(1);
    let mut state = service.state.lock().await;
    state.capabilities.insert(
        "expired-capability".into(),
        Capability {
            approval_pending: false,
            purpose: FilePurpose::Transfer,
            selection,
            connection_id: receipt.connection_id.clone(),
            channel_id: receipt.channel_id.clone(),
            blob_id: receipt.blob_id.clone(),
            transfer_id: None,
            expires: expired_at,
            binding: receipt.binding.clone(),
            request_id: Some(receipt.request_id.clone()),
            replay_receipt_id: None,
        },
    );
    let error = match TransferService::take_file(
        &mut state,
        "expired-capability",
        &receipt,
        false,
        FilePurpose::Transfer,
    ) {
        Ok(_) => panic!("an expired capability must not be consumed"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("Local file approval"));
    assert!(state.capabilities.is_empty());
}

#[tokio::test]
async fn discarding_a_pending_capability_releases_a_selection_slot() {
    let root = private_root();
    let service = TransferService::open(root.path()).unwrap();
    let sentinel = root.path().join("protected-target.bin");
    fs::write(&sentinel, b"protected receipt sentinel").unwrap();
    let receipt_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    service.state.lock().await.receipts.insert(
        receipt_id.into(),
        Receipt {
            id: receipt_id.into(),
            request_id: "protected-request".into(),
            connection_id: "connection".into(),
            channel_id: "channel".into(),
            direction: Direction::Download,
            name: "protected-target.bin".into(),
            size: 0,
            sha256: String::new(),
            offset: 0,
            blob_id: Some("blob".into()),
            state: "needs_file_selection".into(),
            error: None,
            binding: "binding".into(),
            intent: "intent".into(),
            local_selection: String::new(),
            destination_identity: None,
            destination_selection: None,
            initial_target: None,
        },
    );
    let receipt_before = service.get(receipt_id).await.unwrap();
    for index in 0..32 {
        let path = root.path().join(format!("target-{index}.bin"));
        let selection = local_files::select(&path, Direction::Download, false).unwrap();
        service.state.lock().await.capabilities.insert(
            format!("capability-{index}"),
            Capability {
                approval_pending: true,
                purpose: FilePurpose::Transfer,
                selection,
                connection_id: "connection".into(),
                channel_id: "channel".into(),
                blob_id: Some("blob".into()),
                transfer_id: None,
                expires: Instant::now() + Duration::from_secs(300),
                binding: "binding".into(),
                request_id: None,
                replay_receipt_id: None,
            },
        );
    }
    assert_eq!(service.state.lock().await.capabilities.len(), 32);
    let discarded = service.discard("capability-0").await.unwrap();
    assert_eq!(discarded["discarded"], true);
    assert_eq!(service.state.lock().await.capabilities.len(), 31);
    assert_eq!(
        service.get(receipt_id).await.unwrap().state,
        receipt_before.state
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"protected receipt sentinel");

    let replacement = local_files::select(
        &root.path().join("replacement.bin"),
        Direction::Download,
        false,
    )
    .unwrap();
    service.state.lock().await.capabilities.insert(
        "replacement-capability".into(),
        Capability {
            approval_pending: true,
            purpose: FilePurpose::Transfer,
            selection: replacement,
            connection_id: "connection".into(),
            channel_id: "channel".into(),
            blob_id: Some("blob".into()),
            transfer_id: None,
            expires: Instant::now() + Duration::from_secs(300),
            binding: "binding".into(),
            request_id: None,
            replay_receipt_id: None,
        },
    );
    assert_eq!(service.state.lock().await.capabilities.len(), 32);
}

#[tokio::test]
async fn completed_replay_helpers_restore_original_receipt_and_target_approval() {
    let root = private_root();
    let service = TransferService::open(root.path()).unwrap();
    let destination = root.path().join("completed.bin");
    fs::write(&destination, b"approved target").unwrap();
    let selection = local_files::select(&destination, Direction::Download, true).unwrap();
    let destination_selection = local_files::destination_selection_identity(&selection).unwrap();
    let initial_target = match &selection {
        Selection::Destination { target, .. } => target.clone(),
        Selection::Source { .. } => panic!("expected a destination selection"),
    };
    let receipt_id = "66666666666666666666666666666666";
    let request_id = "completed-replay-request";
    service.state.lock().await.receipts.insert(
        receipt_id.into(),
        Receipt {
            id: receipt_id.into(),
            request_id: request_id.into(),
            connection_id: "connection".into(),
            channel_id: "channel".into(),
            direction: Direction::Download,
            name: "completed.bin".into(),
            size: 15,
            sha256: "b".repeat(64),
            offset: 15,
            blob_id: Some("blob".into()),
            state: "completed".into(),
            error: None,
            binding: "binding".into(),
            intent: "intent".into(),
            local_selection: local_files::selection_identity(&selection).unwrap(),
            destination_identity: Some(
                local_files::destination_identity(
                    match &selection {
                        Selection::Destination { directory, .. } => directory,
                        Selection::Source { .. } => panic!("expected destination"),
                    },
                    "completed.bin",
                )
                .unwrap(),
            ),
            destination_selection,
            initial_target: Some(initial_target),
        },
    );

    let request = FileRequest {
        approval_pending: false,
        expected_mode: None,
        purpose: FilePurpose::Transfer,
        connection_id: "connection".into(),
        channel_id: "channel".into(),
        direction: Direction::Download,
        path: destination.clone(),
        overwrite: true,
        blob_id: Some("blob".into()),
        transfer_id: None,
        request_id: Some(request_id.into()),
    };
    let replay = service
        .registration_replay(&request, "binding")
        .await
        .unwrap();
    assert_eq!(replay.as_deref(), Some(receipt_id));
    fs::remove_file(&destination).unwrap();
    fs::write(&destination, b"published new bytes").unwrap();
    let state = service.state.lock().await;
    let mut replay_selection = local_files::select_download_replay(&destination, true).unwrap();
    TransferService::bind_replay_selection(&state, replay.as_deref(), &mut replay_selection)
        .unwrap();
    assert_eq!(
        local_files::selection_identity(&replay_selection).unwrap(),
        state.receipts[receipt_id].local_selection
    );
    assert_eq!(fs::read(&destination).unwrap(), b"published new bytes");
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
        destination_selection: None,
        initial_target: None,
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

fn stopped_receipt(direction: Direction, state: &str) -> Receipt {
    Receipt {
        id: "0123456789abcdef0123456789abcdef".into(),
        request_id: "request".into(),
        connection_id: "connection".into(),
        channel_id: "channel".into(),
        direction,
        name: "restricted.csv".into(),
        size: 4,
        sha256: "a".repeat(64),
        offset: 0,
        blob_id: Some("blob".into()),
        state: state.into(),
        error: None,
        binding: "binding".into(),
        intent: "intent".into(),
        local_selection: String::new(),
        destination_identity: None,
        destination_selection: None,
        initial_target: None,
    }
}

/// F-1: a removed member's download is refused by the workspace (`forbidden: channel
/// unavailable`). It used to end `needs_file_selection`, "Reselect the original local file or
/// destination to resume", which no reselection could ever fix. It now ends `failed`, saying
/// why in the words the CLI uses. A stop the workspace did not answer is still resumable.
#[test]
fn a_transfer_the_workspace_refused_ends_failed_with_its_reason() {
    let refused = anyhow::anyhow!(
        "Crew broker refused request: {}",
        json!({"code": "forbidden", "message": "forbidden: channel unavailable"})
    );
    for direction in [Direction::Download, Direction::Upload] {
        for state in ["starting", "downloading", "uploading"] {
            let (stopped, message) = stopped_transfer(&stopped_receipt(direction, state), &refused);
            assert_eq!(stopped, "failed");
            assert_eq!(
                message,
                "That channel isn't available to you. It may be archived, or you may not be in it."
            );
            assert!(!message.contains("Reselect") && !message.contains('{'));
        }
    }
    // Refused while publishing: whether the file was published is still the question.
    let (stopped, _) = stopped_transfer(
        &stopped_receipt(Direction::Download, "publishing"),
        &refused,
    );
    assert_eq!(stopped, "publication_unconfirmed");
    // Not the workspace's answer: reconnecting and reselecting does resume these.
    let (stopped, message) = stopped_transfer(
        &stopped_receipt(Direction::Download, "downloading"),
        &anyhow::anyhow!("Crew connection is disconnected; authenticate and connect in Crew"),
    );
    assert_eq!(stopped, "needs_file_selection");
    assert!(
        message.starts_with("Authenticate and reconnect"),
        "{message}"
    );
    let (stopped, _) = stopped_transfer(
        &stopped_receipt(Direction::Upload, "uploading"),
        &anyhow::anyhow!("Transfer paused"),
    );
    assert_eq!(stopped, "needs_file_selection");
}

/// F-1: a refused transfer stays refused, with its reason, when the daemon restarts; it is
/// not turned back into one that asks for the file again.
#[tokio::test]
async fn a_refused_transfer_stays_failed_across_a_restart() {
    let root = private_root();
    let mut receipt = stopped_receipt(Direction::Download, "failed");
    receipt.error = Some(
        "That channel isn't available to you. It may be archived, or you may not be in it.".into(),
    );
    let id = receipt.id.clone();
    let mut receipts = serde_json::Map::new();
    receipts.insert(id.clone(), serde_json::to_value(&receipt).unwrap());
    fs::write(
        root.path().join("receipts.json"),
        serde_json::to_vec(&receipts).unwrap(),
    )
    .unwrap();
    fs::set_permissions(
        root.path().join("receipts.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let service = TransferService::open(root.path()).unwrap();
    let state = service.state.lock().await;
    let reopened = &state.receipts[&id];
    assert_eq!(reopened.state, "failed");
    assert_eq!(
        reopened.error.as_deref(),
        Some("That channel isn't available to you. It may be archived, or you may not be in it.")
    );
}
