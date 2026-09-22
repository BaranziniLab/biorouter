use super::{Transport, MAX_FRAME};
use serde_json::json;
use std::{fs, path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{io::BufReader, process::Command, sync::Mutex};

fn spawn_peer(script: &str, marker: Option<&Path>) -> Transport {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .arg("crew-transport-test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(marker) = marker {
        command.arg(marker);
    }
    let mut child = command.spawn().expect("spawn local transport peer");
    let stdin = child.stdin.take().expect("peer stdin");
    let stdout = BufReader::new(child.stdout.take().expect("peer stdout"));
    Transport {
        child,
        stdin,
        stdout,
        unusable: false,
    }
}

async fn wait_for_marker(path: &Path) {
    for _ in 0..100 {
        if fs::read_to_string(path).is_ok_and(|contents| !contents.is_empty()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("local peer did not observe a request");
}

fn request_params() -> serde_json::Value {
    json!({"workspace": "synthetic-crew-transport-test"})
}

fn assert_safe_failure(error: anyhow::Error, code: &str) -> String {
    let message = format!("{error:#}");
    assert!(
        message.contains(&format!("Crew SSH failure [{code}; child_before_cleanup=")),
        "unexpected transport failure: {message}"
    );
    for sentinel in [
        "not-json",
        "partial",
        "wrong",
        "sentinel-code",
        "synthetic-crew-transport-test",
    ] {
        assert!(
            !message.contains(sentinel),
            "raw wire/request sentinel leaked into transport error: {message}"
        );
    }
    message
}

#[tokio::test]
async fn valid_response_rearms_transport_for_the_next_request() {
    let script = r#"
        n=0
        while IFS= read -r line; do
            n=$((n+1))
            if [ "$n" -eq 1 ]; then
                printf '%s\n' '{"version":1,"id":"first","result":{"sequence":1}}'
            else
                printf '%s\n' '{"version":1,"id":"second","result":{"sequence":2}}'
            fi
        done
    "#;
    let mut transport = spawn_peer(script, None);

    assert_eq!(
        transport
            .request(
                "synthetic.echo",
                request_params(),
                None,
                None,
                Some("first".into())
            )
            .await
            .unwrap(),
        json!({"sequence": 1})
    );
    assert!(transport.is_usable());
    assert_eq!(
        transport
            .request(
                "synthetic.echo",
                request_params(),
                None,
                None,
                Some("second".into())
            )
            .await
            .unwrap(),
        json!({"sequence": 2})
    );
    transport.close().await;
}

#[tokio::test]
async fn structured_broker_denial_keeps_transport_usable() {
    let script = r#"
        n=0
        while IFS= read -r line; do
            n=$((n+1))
            if [ "$n" -eq 1 ]; then
                printf '%s\n' '{"version":1,"id":"denied","error":{"code":"forbidden","message":"synthetic denial"}}'
            else
                printf '%s\n' '{"version":1,"id":"allowed","result":{"accepted":true}}'
            fi
        done
    "#;
    let mut transport = spawn_peer(script, None);

    assert!(transport
        .request(
            "synthetic.denied",
            request_params(),
            None,
            None,
            Some("denied".into())
        )
        .await
        .is_err());
    assert!(transport.is_usable());
    assert_eq!(
        transport
            .request(
                "synthetic.allowed",
                request_params(),
                None,
                None,
                Some("allowed".into())
            )
            .await
            .unwrap(),
        json!({"accepted": true})
    );
    transport.close().await;
}

#[tokio::test]
async fn oversized_local_request_is_rejected_before_poisoning_transport() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let script = r#"
        while IFS= read -r line; do
            printf x >> "$1"
            printf '%s\n' '{"version":1,"id":"small","result":{"accepted":true}}'
        done
    "#;
    let mut transport = spawn_peer(script, Some(marker.path()));
    let oversized = json!({"payload": "x".repeat(MAX_FRAME) });

    assert!(transport
        .request(
            "synthetic.oversized",
            oversized,
            None,
            None,
            Some("large".into())
        )
        .await
        .is_err());
    assert!(transport.is_usable());
    assert_eq!(
        transport
            .request(
                "synthetic.small",
                request_params(),
                None,
                None,
                Some("small".into())
            )
            .await
            .unwrap(),
        json!({"accepted": true})
    );
    assert_eq!(fs::read_to_string(marker.path()).unwrap(), "x");
    transport.close().await;
}

#[tokio::test]
async fn response_id_mismatch_makes_transport_unusable_without_a_second_write() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let script = r#"
        while IFS= read -r line; do
            printf x >> "$1"
            printf '%s\n' '{"version":1,"id":"wrong","result":{}}'
        done
    "#;
    let mut transport = spawn_peer(script, Some(marker.path()));

    let error = transport
        .request(
            "synthetic.bad_id",
            request_params(),
            None,
            None,
            Some("expected".into()),
        )
        .await
        .expect_err("mismatched response IDs must fail the exchange");
    let message = assert_safe_failure(error, "ssh_response_id_mismatch");
    assert!(
        message.contains("child_before_cleanup=running"),
        "{message}"
    );
    assert!(!transport.is_usable());
    assert!(transport
        .request(
            "synthetic.retry",
            request_params(),
            None,
            None,
            Some("retry".into())
        )
        .await
        .is_err());
    wait_for_marker(marker.path()).await;
    assert_eq!(fs::read_to_string(marker.path()).unwrap(), "x");
    transport.close().await;
}

#[tokio::test]
async fn eof_marks_transport_unusable_and_blocks_retry() {
    let mut transport = spawn_peer("IFS= read -r line; exit 0", None);

    let error = transport
        .request(
            "synthetic.eof",
            request_params(),
            None,
            None,
            Some("eof".into()),
        )
        .await
        .expect_err("peer EOF must fail the exchange");
    assert_safe_failure(error, "ssh_eof");
    assert!(!transport.is_usable());
    assert!(transport
        .request(
            "synthetic.retry",
            request_params(),
            None,
            None,
            Some("retry".into())
        )
        .await
        .is_err());
    transport.close().await;
}

#[tokio::test]
async fn incomplete_response_marks_transport_unusable() {
    let script = r#"IFS= read -r line; printf '%s' '{"version":1,"id":"partial","result":{}}'"#;
    let mut transport = spawn_peer(script, None);

    let error = transport
        .request(
            "synthetic.partial",
            request_params(),
            None,
            None,
            Some("partial".into()),
        )
        .await
        .expect_err("a response without a terminating newline must fail");
    assert_safe_failure(error, "ssh_frame_incomplete");
    assert!(!transport.is_usable());
    transport.close().await;
}

#[tokio::test]
async fn cancellation_after_write_leaves_transport_unusable() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let script = r#"
        while IFS= read -r line; do
            printf x >> "$1"
            sleep 5
        done
    "#;
    let mut transport = spawn_peer(script, Some(marker.path()));

    let cancelled = tokio::time::timeout(
        Duration::from_millis(250),
        transport.request(
            "synthetic.slow",
            request_params(),
            None,
            None,
            Some("slow".into()),
        ),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "peer deliberately withheld its response"
    );
    assert!(!transport.is_usable());
    wait_for_marker(marker.path()).await;
    assert!(transport
        .request(
            "synthetic.retry",
            request_params(),
            None,
            None,
            Some("retry".into())
        )
        .await
        .is_err());
    assert_eq!(fs::read_to_string(marker.path()).unwrap(), "x");
    transport.close().await;
}

#[tokio::test]
async fn malformed_json_response_marks_transport_unusable() {
    let mut transport = spawn_peer("IFS= read -r line; printf '%s\\n' 'not-json'", None);

    let error = transport
        .request(
            "synthetic.malformed",
            request_params(),
            None,
            None,
            Some("bad".into()),
        )
        .await
        .expect_err("malformed JSON must fail frame validation");
    let message = assert_safe_failure(error, "ssh_invalid_json");
    assert!(message.contains("invalid JSON"), "{message}");
    assert!(!transport.is_usable());
    transport.close().await;
}

#[tokio::test]
async fn malformed_error_envelope_has_safe_category_without_wire_details() {
    let script = r#"IFS= read -r line; printf '%s\n' '{"version":1,"id":"envelope","error":{"code":"sentinel-code","message":123}}'"#;
    let mut transport = spawn_peer(script, None);

    let error = transport
        .request(
            "synthetic.error_envelope",
            request_params(),
            None,
            None,
            Some("envelope".into()),
        )
        .await
        .expect_err("malformed broker error envelopes must fail validation");
    let message = assert_safe_failure(error, "ssh_invalid_envelope");
    assert!(
        message.contains("invalid broker error envelope"),
        "{message}"
    );
    assert!(!transport.is_usable());
    transport.close().await;
}

#[tokio::test]
async fn oversized_response_has_safe_category_without_wire_details() {
    let script = r#"IFS= read -r line; head -c 1048577 /dev/zero"#;
    let mut transport = spawn_peer(script, None);

    let error = transport
        .request(
            "synthetic.oversized_response",
            request_params(),
            None,
            None,
            Some("oversized-response".into()),
        )
        .await
        .expect_err("responses over MAX_FRAME must be rejected");
    assert_safe_failure(error, "ssh_frame_too_large");
    assert!(!transport.is_usable());
    transport.close().await;
}

#[tokio::test]
async fn naturally_exited_peer_reports_safe_exit_status_before_cleanup() {
    let mut transport = spawn_peer("exit 23", None);
    let status = transport.child.wait().await.unwrap();
    assert_eq!(status.code(), Some(23));

    let error = transport
        .request(
            "synthetic.natural_exit",
            request_params(),
            None,
            None,
            Some("natural-exit".into()),
        )
        .await
        .expect_err("writing to a naturally exited peer must fail");
    let message = assert_safe_failure(error, "ssh_write_io_broken_pipe");
    assert!(
        message.contains("child_before_cleanup=exit_23"),
        "{message}"
    );
}

#[tokio::test]
async fn stale_failed_transport_cannot_retire_a_newer_replacement() {
    use super::super::{ClusterMode, Connection, CrewManager};

    let root = tempfile::tempdir().unwrap();
    let manager = CrewManager::new(root.path().to_path_buf()).unwrap();
    let connection_id = "stale-transport-test";
    manager.registry.lock().await.connections.push(Connection {
        id: connection_id.into(),
        node_id: None,
        name: "stale transport fixture".into(),
        ssh_target: "fixture@example.test".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/stale-transport.sock".into(),
        owner_uid: 10001,
        workspace_id: "stale-transport-workspace".into(),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: "stale-transport-cluster".into(),
        mode: ClusterMode::Public,
        policy_epoch: 1,
        status: "connected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    });

    let failed = Arc::new(Mutex::new(spawn_peer("sleep 5", None)));
    let replacement = Arc::new(Mutex::new(spawn_peer("sleep 5", None)));
    manager
        .transports
        .lock()
        .await
        .insert(connection_id.into(), replacement.clone());

    manager
        .retire_failed_transport(connection_id, &failed)
        .await
        .unwrap();

    let current = manager
        .transports
        .lock()
        .await
        .get(connection_id)
        .cloned()
        .expect("newer transport remains published");
    assert!(Arc::ptr_eq(&current, &replacement));
    assert!(current.lock().await.is_usable());
    let registry = manager.registry.lock().await;
    let connection = registry
        .connections
        .iter()
        .find(|connection| connection.id == connection_id)
        .unwrap();
    assert_eq!(connection.status, "connected");
    assert!(connection.last_error.is_none());
    drop(registry);

    replacement.lock().await.unusable = true;
    manager
        .retire_failed_transport(connection_id, &replacement)
        .await
        .unwrap();
    assert!(!manager.transports.lock().await.contains_key(connection_id));
    let registry = manager.registry.lock().await;
    let connection = registry
        .connections
        .iter()
        .find(|connection| connection.id == connection_id)
        .unwrap();
    assert_eq!(connection.status, "disconnected");
    assert!(connection
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("inspect any submitted operation")));
    assert_eq!(connection.mode, ClusterMode::Public);
    drop(registry);
    assert!(replacement.lock().await.child.try_wait().unwrap().is_some());

    failed.lock().await.close().await;
}
