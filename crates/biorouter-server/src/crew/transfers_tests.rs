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
            pause_reason: None,
            binding: "receipt-binding".into(),
            requires_private: false,
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
        pause_reason: None,
        binding: "pending-binding".into(),
        requires_private: false,
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
                requires_private: false,
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
        pause_reason: None,
        binding: "expired-binding".into(),
        requires_private: false,
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
            requires_private: false,
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
            pause_reason: None,
            binding: "binding".into(),
            requires_private: false,
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
                requires_private: false,
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
            requires_private: false,
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
            pause_reason: None,
            binding: "binding".into(),
            requires_private: false,
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
        pause_reason: Some("server_storage".into()),
        binding: "binding".into(),
        requires_private: false,
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
    assert!(
        accepted.pause_reason.is_none(),
        "a resumed transfer is not paused"
    );
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
        pause_reason: None,
        binding: "binding".into(),
        requires_private: false,
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
            assert_eq!(message, "You're not in that channel.");
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

/// T3-BE-14: a transfer the workspace's server could not save (its disk is full, or its storage
/// failed) is paused, not failed: it keeps its offset, says why in the workspace's own sentence,
/// and carries `pause_reason: server_storage`, so the app offers Resume as the CLI always could.
/// Whether the server merely could not write this one file or has stopped saving, the resume is
/// the same. Any other refusal still fails, with no pause reason.
#[test]
fn a_transfer_the_server_could_not_save_is_paused_not_failed() {
    let refused = |code: &str, message: &str| {
        anyhow::anyhow!(
            "Crew broker refused request: {}",
            json!({"code": code, "message": format!("{code}: {message}")})
        )
        .context("Couldn't send the next part")
    };
    for (error, sentence) in [
        (
            refused("storage_full", "The workspace server is out of disk space, so this could not be saved. Ask the host to free space on the server, then try again."),
            "The workspace server is out of disk space, so this could not be saved. Ask the host to free space on the server, then try again.",
        ),
        (
            refused("storage_failed", "The workspace server could not save a change to disk and has stopped saving changes. Reading still works. Ask the host to check the server's storage and restart Crew."),
            "The workspace server could not save a change to disk and has stopped saving changes. Reading still works. Ask the host to check the server's storage and restart Crew.",
        ),
    ] {
        for (direction, state) in [
            (Direction::Upload, "uploading"),
            (Direction::Upload, "starting"),
            (Direction::Download, "downloading"),
        ] {
            let mut receipt = stopped_receipt(direction, state);
            receipt.offset = 2;
            let (stopped, message) = stopped_transfer(&receipt, &error);
            assert_eq!(stopped, "needs_file_selection", "{error:#}");
            assert_eq!(message, sentence);
            assert_eq!(pause_reason(stopped, &error), Some("server_storage"));
        }
    }
    let forbidden = refused("forbidden", "channel unavailable");
    let (stopped, _) =
        stopped_transfer(&stopped_receipt(Direction::Upload, "uploading"), &forbidden);
    assert_eq!(stopped, "failed");
    assert_eq!(pause_reason(stopped, &forbidden), None);
    let paused = anyhow::anyhow!("Transfer paused");
    assert_eq!(pause_reason("needs_file_selection", &paused), None);
}

/// T3-BE-14: a paused transfer keeps its reason and its offset across a restart, so it is
/// still offered for resuming, from where it stopped.
#[tokio::test]
async fn a_transfer_paused_for_server_storage_stays_resumable_across_a_restart() {
    let root = private_root();
    let mut receipt = stopped_receipt(Direction::Upload, "needs_file_selection");
    receipt.offset = 2;
    receipt.error = Some("The workspace server is out of disk space.".into());
    receipt.pause_reason = Some("server_storage".into());
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
    assert_eq!(reopened.state, "needs_file_selection");
    assert_eq!(reopened.offset, 2);
    assert_eq!(reopened.pause_reason.as_deref(), Some("server_storage"));
    // An older receipt, saved before the field existed, reads as having no reason.
    let mut older = serde_json::to_value(&receipt).unwrap();
    older.as_object_mut().unwrap().remove("pause_reason");
    let older: Receipt = serde_json::from_value(older).unwrap();
    assert_eq!(older.pause_reason, None);
}

/// F-1: a refused transfer stays refused, with its reason, when the daemon restarts; it is
/// not turned back into one that asks for the file again.
#[tokio::test]
async fn a_refused_transfer_stays_failed_across_a_restart() {
    let root = private_root();
    let mut receipt = stopped_receipt(Direction::Download, "failed");
    receipt.error = Some("You're not in that channel.".into());
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
        Some("You're not in that channel.")
    );
}

/// A saved connection in `mode`, as the Crew registry keeps it.
fn crew_connection(mode: biorouter::crew::ClusterMode) -> biorouter::crew::Connection {
    biorouter::crew::Connection {
        id: "mode-connection".into(),
        node_id: None,
        name: "okafor-lab".into(),
        ssh_target: "crew@example.test".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/run/crew.sock".into(),
        owner_uid: 10001,
        workspace_id: "mode-workspace".into(),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: "mode-cluster".into(),
        mode,
        institution_id: None,
        policy_epoch: 1,
        status: "connected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    }
}

const MODE_MISMATCH_TEXT: &str =
    "Your connection is Public, but this request required Private. Nothing was sent.";

/// T3-BE-5 (review): a file selection (`POST /crew/files`) is judged by the rule every other door
/// judges by. A personal Public connection whose workspace's signed `hello` says it is Private
/// for everyone is Private in force, so an upload that required Private (a terminal's
/// `--expected-mode private`) goes ahead. It used to be refused as "Your connection is Public,
/// but this request required Private" while status and privacy show said Private. The upload's
/// receipt then holds the requirement, and its attachment is begun as Private, so the workspace
/// restricts it even if that `hello` has gone stale since.
#[tokio::test]
async fn a_selection_that_required_the_privacy_in_force_begins_its_attachment_private() {
    use biorouter::crew::ClusterMode::{Private, Public};
    let connection = crew_connection(Public);
    let binding = judged_binding(&connection, Some(Private), Some(Private)).unwrap();
    assert_eq!(binding, binding_for_connection(&connection).unwrap());
    // What the desktop sends (the connection's own mode), and no expectation, go ahead too.
    for expected in [Some(Public), None] {
        assert_eq!(
            judged_binding(&connection, Some(Private), expected).unwrap(),
            binding
        );
    }
    // A workspace that allows Public, or one no signed `hello` has described: Public in force.
    for workspace in [Some(Public), None] {
        let refused = judged_binding(&connection, workspace, Some(Private)).unwrap_err();
        let typed = biorouter::crew::CrewRefusal::find(&refused).expect("a typed refusal");
        assert_eq!(typed.code(), "crew_mode_mismatch", "{workspace:?}");
        assert_eq!(refused.to_string(), MODE_MISMATCH_TEXT);
    }

    // The selection as `POST /crew/files` records it, and the transfer it starts.
    let root = private_root();
    let service = Arc::new(TransferService::open(root.path()).unwrap());
    let source = root.path().join("assay.csv");
    fs::write(&source, b"sample,signal\nS1,12.7\n").unwrap();
    let request = FileRequest {
        approval_pending: false,
        expected_mode: Some(Private),
        purpose: FilePurpose::Transfer,
        connection_id: connection.id.clone(),
        channel_id: "channel".into(),
        direction: Direction::Upload,
        path: source.clone(),
        overwrite: false,
        blob_id: None,
        transfer_id: None,
        request_id: Some("private-upload".into()),
    };
    let mut state = service.state.lock().await;
    state.capabilities.insert(
        "private-selection".into(),
        Capability {
            approval_pending: false,
            requires_private: selection_requires_private(&request),
            purpose: FilePurpose::Transfer,
            selection: local_files::select(&source, Direction::Upload, false).unwrap(),
            connection_id: connection.id.clone(),
            channel_id: "channel".into(),
            blob_id: None,
            transfer_id: None,
            expires: Instant::now() + Duration::from_secs(300),
            binding: binding.clone(),
            request_id: request.request_id.clone(),
            replay_receipt_id: None,
        },
    );
    let receipt = service
        .start_bound(
            &mut state,
            StartRequest {
                request_id: "private-upload".into(),
                connection_id: connection.id.clone(),
                channel_id: "channel".into(),
                direction: Direction::Upload,
                file_capability: "private-selection".into(),
                blob_id: None,
            },
            binding.clone(),
        )
        .unwrap();
    // Stop its worker before it reaches a workspace: this test has none.
    state.active[&receipt.id].cancel();
    assert!(receipt.requires_private);
    assert!(state.receipts[&receipt.id].requires_private);
    let persisted: Value =
        serde_json::from_slice(&fs::read(root.path().join("receipts.json")).unwrap()).unwrap();
    assert_eq!(persisted[&receipt.id]["requires_private"], json!(true));
    drop(state);

    // Its attachment is begun as Private, which the daemon then holds against the workspace's
    // latest `hello` and tells the workspace as Private.
    let begin = |receipt: &Receipt| {
        transfer_params(
            receipt,
            &connection,
            "blob.begin",
            json!({"channel_id": "channel", "personal_mode": "public"}),
        )
        .unwrap()["personal_mode"]
            .clone()
    };
    assert_eq!(begin(&receipt), json!("private"));
    // An upload that required nothing more is begun in the connection's own mode, as before.
    let mut own = receipt.clone();
    own.requires_private = false;
    assert_eq!(begin(&own), json!("public"));
    // Only `blob.begin` carries a mode.
    let chunk = transfer_params(&receipt, &connection, "blob.chunk", json!({"blob_id": "b"}));
    assert_eq!(chunk.unwrap(), json!({"blob_id": "b"}));
    assert!(transfer_params(&receipt, &connection, "blob.begin", json!([])).is_err());
}

/// T3-BE-5 (review): only an upload's selection that required Private is held to it: a download
/// has no attachment to begin, a cleanup approval moves nothing, and an upload that required
/// Public (what the desktop sends for a Public connection) or nothing is begun in the
/// connection's own mode.
#[test]
fn only_an_uploads_selection_that_required_private_holds_it() {
    use biorouter::crew::ClusterMode::{Private, Public};
    let request = |purpose, direction, expected_mode| FileRequest {
        approval_pending: false,
        expected_mode,
        purpose,
        connection_id: "connection".into(),
        channel_id: "channel".into(),
        direction,
        path: PathBuf::from("/tmp/fixture.txt"),
        overwrite: false,
        blob_id: None,
        transfer_id: None,
        request_id: None,
    };
    for (purpose, direction, expected, holds) in [
        (
            FilePurpose::Transfer,
            Direction::Upload,
            Some(Private),
            true,
        ),
        (
            FilePurpose::Transfer,
            Direction::Upload,
            Some(Public),
            false,
        ),
        (FilePurpose::Transfer, Direction::Upload, None, false),
        (
            FilePurpose::Transfer,
            Direction::Download,
            Some(Private),
            false,
        ),
        (
            FilePurpose::Cleanup,
            Direction::Download,
            Some(Private),
            false,
        ),
    ] {
        assert_eq!(
            selection_requires_private(&request(purpose, direction, expected)),
            holds,
            "{direction:?} {expected:?}"
        );
    }
}

/// T3-BE-5 (review): a resume never clears an upload's requirement of Private, and a resume
/// that requires it adds it, so an attachment not begun yet is begun as Private.
#[tokio::test]
async fn a_resume_keeps_a_private_requirement_and_can_add_one() {
    for (held, selected, holds) in [
        (true, false, true),
        (false, true, true),
        (false, false, false),
    ] {
        let root = private_root();
        let service = Arc::new(TransferService::open(root.path()).unwrap());
        let source = root.path().join("assay.csv");
        fs::write(&source, b"sample,signal\n").unwrap();
        // Its digest is not the file's, so its worker stops before any workspace is asked.
        let mut receipt = stopped_receipt(Direction::Upload, "needs_file_selection");
        receipt.blob_id = None;
        receipt.requires_private = held;
        let id = receipt.id.clone();
        {
            let mut state = service.state.lock().await;
            state.receipts.insert(id.clone(), receipt.clone());
            state.capabilities.insert(
                "resume-selection".into(),
                Capability {
                    approval_pending: false,
                    requires_private: selected,
                    purpose: FilePurpose::Transfer,
                    selection: local_files::select(&source, Direction::Upload, false).unwrap(),
                    connection_id: receipt.connection_id.clone(),
                    channel_id: receipt.channel_id.clone(),
                    blob_id: None,
                    transfer_id: Some(id.clone()),
                    expires: Instant::now() + Duration::from_secs(300),
                    binding: receipt.binding.clone(),
                    request_id: None,
                    replay_receipt_id: None,
                },
            );
        }
        let accepted = service.resume(&id, "resume-selection").await.unwrap();
        if let Some(token) = service.state.lock().await.active.get(&id) {
            token.cancel();
        }
        assert_eq!(accepted.requires_private, holds, "{held} then {selected}");
    }
}

/// T3-BE-5 (review): an upload that required Private never adds a part to an attachment the
/// workspace does not restrict. One begun for it is restricted (it was begun as Private); one a
/// resume finds, begun earlier under no such requirement while the workspace allowed Public, is
/// refused before any part is sent. A refusal of the requirement, there or where the daemon
/// re-reads the workspace's mode as the attachment is begun, ends the transfer `failed` with the
/// refusal's sentence: the requirement stays with it, so reselecting the file would only meet it
/// again.
#[test]
fn an_upload_that_required_private_never_adds_to_an_unrestricted_attachment() {
    let blob = |restricted: Option<bool>| {
        let mut blob = json!({"id": "blob", "channel_id": "channel", "size": 4,
            "sha256": "a".repeat(64), "offset": 0, "complete": false});
        if let Some(restricted) = restricted {
            blob["restricted"] = json!(restricted);
        }
        serde_json::from_value::<Blob>(blob).unwrap()
    };
    let mut receipt = stopped_receipt(Direction::Upload, "uploading");
    receipt.requires_private = true;
    refuse_unrestricted(&receipt, &blob(Some(true))).unwrap();
    for unrestricted in [Some(false), None] {
        let refused = refuse_unrestricted(&receipt, &blob(unrestricted)).unwrap_err();
        let typed = biorouter::crew::CrewRefusal::find(&refused).expect("a typed refusal");
        assert_eq!(typed.code(), "crew_mode_mismatch");
        assert_eq!(refused.to_string(), MODE_MISMATCH_TEXT);
    }
    receipt.requires_private = false;
    refuse_unrestricted(&receipt, &blob(Some(false))).unwrap();

    let refused = anyhow::Error::from(biorouter::crew::CrewRefusal::mode_mismatch(
        biorouter::crew::ClusterMode::Public,
        biorouter::crew::ClusterMode::Private,
    ))
    .context("Couldn't begin the attachment");
    for state in ["starting", "uploading"] {
        let (stopped, message) =
            stopped_transfer(&stopped_receipt(Direction::Upload, state), &refused);
        assert_eq!(stopped, "failed");
        assert_eq!(message, MODE_MISMATCH_TEXT);
        assert_eq!(pause_reason(stopped, &refused), None);
    }
}

/// T3-BE-5 (review): a replay of a start request is the same transfer only if it asks for the
/// same privacy, so a replay that required Private never answers with an upload begun without
/// it. A receipt that did not require Private keeps the digest it was saved with.
#[test]
fn a_replay_that_requires_private_is_not_the_upload_that_did_not() {
    let mut receipt = stopped_receipt(Direction::Upload, "uploading");
    receipt.local_selection = "selection".into();
    let saved = digest(
        &serde_json::to_vec(&json!([
            "crew-transfer-intent-v2",
            receipt.connection_id,
            receipt.channel_id,
            receipt.direction,
            receipt.blob_id,
            receipt.binding,
            receipt.local_selection
        ]))
        .unwrap(),
    );
    assert_eq!(transfer_intent(&receipt).unwrap(), saved);
    receipt.requires_private = true;
    assert_ne!(transfer_intent(&receipt).unwrap(), saved);
    // A receipt saved before the field existed reads as not requiring it.
    let mut older = serde_json::to_value(&receipt).unwrap();
    older.as_object_mut().unwrap().remove("requires_private");
    assert!(
        !serde_json::from_value::<Receipt>(older)
            .unwrap()
            .requires_private
    );
}
