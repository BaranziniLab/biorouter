use super::{
    classify, fit, sanitize_stderr, ChildState, SshFailure, SshFailureKind, Transport, MAX_FRAME,
    SSH_FAILURE_DETAIL_LIMIT, STDERR_CAPTURE_LIMIT,
};
use serde_json::json;
use std::{fs, path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{process::Command, sync::Mutex};

/// A local stand-in for ssh: `sh -c script`, with all three streams piped and
/// adopted exactly as `Transport::connect` adopts the real ssh child.
fn spawn_peer(script: &str, marker: Option<&Path>) -> Transport {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .arg("crew-transport-test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(marker) = marker {
        command.arg(marker);
    }
    crate::subprocess::prepare_agent_child_command(&mut command);
    Transport::from_child(command.spawn().expect("spawn local transport peer"))
        .expect("adopt local transport peer")
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
        institution_id: None,
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

// --- Typed SSH failure classification -------------------------------------
//
// Each fixture is what OpenSSH (or the remote shell) really prints, fed to a
// fake ssh that reads the request line, writes the fixture to stderr and exits
// with the given status, so the whole path runs: the drain, the exit grace,
// classification and the detail.

const PERMISSION_DENIED: &str = "alice@hpc.example.edu: Permission denied (publickey,gssapi-keyex,gssapi-with-mic,keyboard-interactive).\r\n";
const HOST_KEY_UNKNOWN: &str = "No ED25519 host key is known for hpc.example.edu and you have requested strict checking.\r\nHost key verification failed.\r\n";
const OFFERED_FINGERPRINT: &str = "SHA256:Wm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm8";
const NEW_FINGERPRINT: &str = "SHA256:TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmU";

fn host_key_changed_banner() -> String {
    [
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
        "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
        "IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!",
        "Someone could be eavesdropping on you right now (man-in-the-middle attack)!",
        "It is also possible that a host key has just been changed.",
        "The fingerprint for the ED25519 key sent by the remote host is",
        &format!("{NEW_FINGERPRINT}."),
        "Please contact your system administrator.",
        "Add correct host key in /Users/alice/.ssh/known_hosts to get rid of this message.",
        "Offending ED25519 key in /Users/alice/.ssh/known_hosts:3",
        "Host key for hpc.example.edu has changed and you have requested strict checking.",
        "Host key verification failed.",
    ]
    .iter()
    .map(|line| format!("{line}\r\n"))
    .collect()
}

/// Run a fake ssh that prints `stderr` and exits with `exit` after reading
/// the request, and return the typed failure the transport reports.
async fn ssh_failure_for(stderr: &[u8], exit: i32) -> SshFailure {
    let fixture = tempfile::NamedTempFile::new().unwrap();
    fs::write(fixture.path(), stderr).unwrap();
    let script = format!(r#"IFS= read -r line; cat "$1" >&2; exit {exit}"#);
    let mut transport = spawn_peer(&script, Some(fixture.path()));
    let error = transport
        .request(
            "synthetic.hello",
            request_params(),
            None,
            None,
            Some("hello".into()),
        )
        .await
        .expect_err("a fake ssh that exits must fail the exchange");
    assert!(!transport.is_usable());
    transport.close().await;
    let failure = error
        .downcast_ref::<SshFailure>()
        .cloned()
        .expect("a fatal transport failure carries a typed SshFailure");
    assert_eq!(failure.code, "ssh_eof", "{failure:?}");
    assert_eq!(failure.status, format!("exit_{exit}"), "{failure:?}");
    failure
}

fn legacy_message(code: &str, status: &str, description: &str) -> String {
    // The exact format string the transport used before failures were typed.
    format!("Crew SSH failure [{code}; child_before_cleanup={status}]: {description}; reconnect. Submitted operation outcome may be unknown; inspect history before retrying")
}

/// `(case, stderr, exit status, expected kind)` for every fixed OpenSSH text.
const STDERR_FIXTURES: &[(&str, &str, i32, SshFailureKind)] = &[
    ("publickey refused", PERMISSION_DENIED, 255, SshFailureKind::AuthRequired),
    (
        "keyboard-interactive only",
        "alice@hpc.example.edu: Permission denied (keyboard-interactive).\r\n",
        255,
        SshFailureKind::AuthRequired,
    ),
    (
        "too many authentication failures",
        "Received disconnect from 192.0.2.10 port 22:2: Too many authentication failures\r\nDisconnected from 192.0.2.10 port 22\r\n",
        255,
        SshFailureKind::AuthRequired,
    ),
    (
        "authentication failed",
        "Authentication failed.\r\n",
        255,
        SshFailureKind::AuthRequired,
    ),
    (
        "a stale control socket, then the real refusal",
        "Control socket connect(/tmp/crew-control.sock): Connection refused\r\nalice@hpc.example.edu: Permission denied (publickey,password).\r\n",
        255,
        SshFailureKind::AuthRequired,
    ),
    ("unknown host key", HOST_KEY_UNKNOWN, 255, SshFailureKind::HostKeyUnknown),
    (
        "bare verification failure",
        "Host key verification failed.\r\n",
        255,
        SshFailureKind::HostKeyUnknown,
    ),
    (
        "changed host key without banner",
        "Host key for hpc.example.edu has changed and you have requested strict checking.\r\nHost key verification failed.\r\n",
        255,
        SshFailureKind::HostKeyChanged,
    ),
    (
        "revoked host key",
        "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n@       WARNING: REVOKED HOST KEY DETECTED!               @\r\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\nHost key verification failed.\r\n",
        255,
        SshFailureKind::HostKeyChanged,
    ),
    (
        "unresolvable host",
        "ssh: Could not resolve hostname hpc.example.invalid: nodename nor servname provided, or not known\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "refused",
        "ssh: connect to host hpc.example.edu port 22: Connection refused\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "timed out (Linux)",
        "ssh: connect to host hpc.example.edu port 22: Connection timed out\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "timed out (macOS)",
        "ssh: connect to host hpc.example.edu port 22: Operation timed out\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "no route",
        "ssh: connect to host 192.0.2.10 port 22: No route to host\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "network unreachable",
        "ssh: connect to host 192.0.2.10 port 22: Network is unreachable\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "jump host cannot reach the target",
        "channel 0: open failed: connect failed: Name or service not known\r\nstdio forwarding failed\r\nConnection closed by UNKNOWN port 65535\r\n",
        255,
        SshFailureKind::Unreachable,
    ),
    (
        "bash: bridge missing",
        "bash: line 1: /home/alice/.local/bin/biorouter-crew: No such file or directory\n",
        127,
        SshFailureKind::BridgeMissing,
    ),
    (
        "zsh: bridge missing, text only",
        "zsh:1: no such file or directory: /home/alice/.local/bin/biorouter-crew\n",
        1,
        SshFailureKind::BridgeMissing,
    ),
    (
        "dash: bridge missing, text only",
        "sh: 1: /home/alice/.local/bin/biorouter-crew: not found\n",
        1,
        SshFailureKind::BridgeMissing,
    ),
    (
        "bridge not executable",
        "bash: line 1: /home/alice/.local/bin/biorouter-crew: Permission denied\n",
        126,
        SshFailureKind::BridgeMissing,
    ),
    (
        "a stopped broker is not a missing bridge",
        "Error: No such file or directory (os error 2)\n",
        1,
        SshFailureKind::Other,
    ),
    // The bridge's own startup errors: SSH connected, so the status is the
    // bridge's (anyhow's `Error: …`, exit 1), and the SSH rules must not read it.
    (
        "a stopped broker's stale socket is not an unreachable host",
        "Error: Connection refused (os error 111)\n",
        1,
        SshFailureKind::Other,
    ),
    (
        "a runtime-directory permission error is not a sign-in failure",
        "Error: Permission denied (os error 13)\n",
        1,
        SshFailureKind::Other,
    ),
    (
        "a bridge timeout is not an unreachable host",
        "Error: Connection timed out (os error 110)\n",
        1,
        SshFailureKind::Other,
    ),
    (
        "a stale control socket is not unreachability",
        "Control socket connect(/tmp/crew-control.sock): Connection refused\r\n",
        255,
        SshFailureKind::Other,
    ),
    (
        "algorithm negotiation is never labelled a host-key problem",
        "Unable to negotiate with 192.0.2.10 port 22: no matching host key type found. Their offer: ssh-rsa\r\n",
        255,
        SshFailureKind::Other,
    ),
    (
        "a reset during key exchange",
        "kex_exchange_identification: read: Connection reset by peer\r\nConnection reset by 192.0.2.10 port 22\r\n",
        255,
        SshFailureKind::Other,
    ),
];

#[tokio::test]
async fn each_openssh_stderr_fixture_maps_to_its_kind() {
    let changed = host_key_changed_banner();
    let banner_case = (
        "changed host key banner",
        changed.as_str(),
        255,
        SshFailureKind::HostKeyChanged,
    );
    for &(name, stderr, exit, kind) in STDERR_FIXTURES.iter().chain([&banner_case]) {
        let failure = ssh_failure_for(stderr.as_bytes(), exit).await;
        assert_eq!(failure.kind, kind, "{name}: {failure:?}");
        assert!(
            failure.detail.is_some(),
            "{name}: stderr must reach the detail"
        );
    }
}

#[tokio::test]
async fn exit_127_without_stderr_is_a_missing_bridge() {
    let failure = ssh_failure_for(b"", 127).await;
    assert_eq!(failure.kind, SshFailureKind::BridgeMissing);
    assert_eq!(failure.api_code(), "crew_bridge_missing");
    assert_eq!(failure.detail, None);
}

#[tokio::test]
async fn exit_255_without_stderr_is_never_guessed() {
    let failure = ssh_failure_for(b"", 255).await;
    assert_eq!(failure.kind, SshFailureKind::Other);
    assert_eq!(failure.api_code(), "crew_ssh_failed");
    assert_eq!(failure.detail, None);
}

#[tokio::test]
async fn display_text_is_byte_identical_to_the_untyped_message() {
    let failure = ssh_failure_for(PERMISSION_DENIED.as_bytes(), 255).await;
    let expected = legacy_message("ssh_eof", "exit_255", "SSH connection closed");
    assert_eq!(
        expected,
        "Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: SSH connection closed; reconnect. Submitted operation outcome may be unknown; inspect history before retrying"
    );
    assert_eq!(failure.to_string(), expected);
    let error = anyhow::Error::new(failure.clone());
    assert_eq!(format!("{error}"), expected);
    assert_eq!(format!("{error:#}"), expected, "no cause chain is appended");
    assert_eq!(
        error.chain().count(),
        1,
        "the failure is its own root cause"
    );

    // The same holds for every wire code, whatever the detail says.
    let constructed = SshFailure {
        kind: SshFailureKind::Unreachable,
        code: "ssh_timeout".into(),
        status: "running".into(),
        description: "Crew request timed out".into(),
        detail: Some("ssh: connect to host hpc.example.edu port 22: Connection refused".into()),
    };
    assert_eq!(
        constructed.to_string(),
        legacy_message("ssh_timeout", "running", "Crew request timed out")
    );
}

#[tokio::test]
async fn the_detail_never_reaches_display_or_debug() {
    let failure = ssh_failure_for(HOST_KEY_UNKNOWN.as_bytes(), 255).await;
    let detail = failure.detail.clone().expect("stderr captured");
    assert!(detail.contains("Host key verification failed."), "{detail}");
    let error = anyhow::Error::new(failure);
    for rendered in [
        format!("{error}"),
        format!("{error:#}"),
        format!("{error:?}"),
        format!("{error:#?}"),
    ] {
        assert!(
            !rendered.contains("hpc.example.edu") && !rendered.contains("verification"),
            "stderr leaked into a rendering meant for logs: {rendered}"
        );
    }
}

#[tokio::test]
async fn a_failure_while_ssh_still_runs_is_not_classified_from_stderr() {
    // A pre-auth banner can say anything; once the session is up, stderr cannot
    // explain a protocol failure, so it is only carried as detail.
    let script = r#"
        printf '%s\n' 'Notice: Permission denied (publickey) means your key is not registered.' >&2
        while IFS= read -r line; do
            printf '%s\n' '{"version":1,"id":"wrong","result":{}}'
        done
    "#;
    let mut transport = spawn_peer(script, None);
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
    let failure = error.downcast_ref::<SshFailure>().expect("typed failure");
    assert_eq!(failure.code, "ssh_response_id_mismatch");
    assert_eq!(failure.status, "running");
    assert_eq!(failure.kind, SshFailureKind::Other);
    assert!(failure
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("Permission denied (publickey)")));
    transport.close().await;
}

#[tokio::test]
async fn host_key_fingerprints_are_summarised_first() {
    let changed = ssh_failure_for(host_key_changed_banner().as_bytes(), 255).await;
    assert_eq!(changed.kind, SshFailureKind::HostKeyChanged);
    let detail = changed.detail.expect("detail");
    assert!(
        detail.starts_with(&format!(
            "New host key fingerprint (offered by the server): {NEW_FINGERPRINT}\n\n"
        )),
        "{detail}"
    );
    assert!(detail.contains("Offending ED25519 key in /Users/alice/.ssh/known_hosts:3"));

    let offered = format!(
        "The authenticity of host 'hpc.example.edu (192.0.2.10)' can't be established.\r\nED25519 key fingerprint is {OFFERED_FINGERPRINT}.\r\nHost key verification failed.\r\n"
    );
    let unknown = ssh_failure_for(offered.as_bytes(), 255).await;
    assert_eq!(unknown.kind, SshFailureKind::HostKeyUnknown);
    assert!(unknown
        .detail
        .as_deref()
        .is_some_and(|detail| detail.starts_with(&format!(
            "Offered host key fingerprint: {OFFERED_FINGERPRINT}\n"
        ))));

    // Strict checking prints no fingerprint for an unknown key: nothing is invented.
    let bare = ssh_failure_for(HOST_KEY_UNKNOWN.as_bytes(), 255).await;
    assert_eq!(
        bare.detail.as_deref(),
        Some(
            "No ED25519 host key is known for hpc.example.edu and you have requested strict checking.\nHost key verification failed."
        )
    );
}

#[tokio::test]
async fn fingerprints_and_the_final_cause_survive_truncation() {
    // A long pre-auth legal banner ahead of the changed-key report.
    let mut stderr: String = (0..80)
        .map(|n| {
            format!(
                "Authorized use only. Banner line {n:02} of the site's acceptable-use notice.\r\n"
            )
        })
        .collect();
    stderr.push_str(&host_key_changed_banner());
    assert!(stderr.len() > SSH_FAILURE_DETAIL_LIMIT * 2 && stderr.len() < STDERR_CAPTURE_LIMIT);

    let failure = ssh_failure_for(stderr.as_bytes(), 255).await;
    assert_eq!(failure.kind, SshFailureKind::HostKeyChanged);
    let detail = failure.detail.expect("detail");
    assert!(detail.len() <= SSH_FAILURE_DETAIL_LIMIT, "{}", detail.len());
    assert!(detail.starts_with(&format!(
        "New host key fingerprint (offered by the server): {NEW_FINGERPRINT}"
    )));
    assert!(
        detail.contains("Banner line 00"),
        "the opening is kept: {detail}"
    );
    assert!(detail.contains("\n…\n"), "the cut is marked: {detail}");
    assert!(
        detail.ends_with("Host key verification failed."),
        "{detail}"
    );
}

#[tokio::test]
async fn stderr_larger_than_the_pipe_never_blocks_and_is_truncated() {
    // 256 KiB of stderr before the peer even reads its request: four Linux pipe
    // buffers. Undrained, the peer would block forever and the request time out.
    let script = r#"
        yes 'debug1: a very chatty ssh' | head -c 262144 >&2
        IFS= read -r line
        printf '%s\n' '{"version":1,"id":"loud","result":{"ok":true}}'
        IFS= read -r line
        exit 255
    "#;
    let mut transport = spawn_peer(script, None);
    let answered = tokio::time::timeout(
        Duration::from_secs(20),
        transport.request(
            "synthetic.loud",
            request_params(),
            None,
            None,
            Some("loud".into()),
        ),
    )
    .await
    .expect("a full stderr pipe must never block the peer")
    .unwrap();
    assert_eq!(answered, json!({"ok": true}));
    assert_eq!(transport.stderr.snapshot().len(), STDERR_CAPTURE_LIMIT);

    let error = transport
        .request(
            "synthetic.closing",
            request_params(),
            None,
            None,
            Some("closing".into()),
        )
        .await
        .expect_err("the peer exits after its first answer");
    let failure = error.downcast_ref::<SshFailure>().expect("typed failure");
    assert_eq!(failure.status, "exit_255");
    assert_eq!(failure.kind, SshFailureKind::Other);
    let detail = failure.detail.as_deref().expect("detail");
    assert!(detail.len() <= SSH_FAILURE_DETAIL_LIMIT, "{}", detail.len());
    assert!(detail.contains("\n…\n"));
    transport.close().await;
}

#[tokio::test]
async fn control_characters_are_stripped_from_the_detail() {
    let noisy = "\u{1b}[31malice@hpc.example.edu: Permission denied (publickey).\u{1b}[0m\u{7}\r\n\
                 \u{1b}]0;spoofed window title\u{7}banner\u{0}\u{8}text\u{202e}reversed\u{2066}\r\n\
                 fake line\rreal line\r\n\tindented\u{7f}\u{9b}\r\n";
    let failure = ssh_failure_for(noisy.as_bytes(), 255).await;
    assert_eq!(failure.kind, SshFailureKind::AuthRequired);
    let detail = failure.detail.expect("detail");
    assert!(
        detail.chars().all(|c| c == '\n' || !c.is_control()),
        "{detail:?}"
    );
    assert!(!detail.contains('\u{202e}') && !detail.contains('\u{2066}'));
    assert!(!detail.contains("[31m") && !detail.contains("spoofed"));
    assert_eq!(
        detail,
        "alice@hpc.example.edu: Permission denied (publickey).\nbannertextreversed\nfake line\nreal line\n indented"
    );
}

#[test]
fn only_an_exited_child_is_classified() {
    assert_eq!(
        classify(
            ChildState::Running,
            "alice@h: Permission denied (publickey)."
        ),
        SshFailureKind::Other
    );
    assert_eq!(
        classify(ChildState::Unknown, "Host key verification failed."),
        SshFailureKind::Other
    );
    assert_eq!(
        classify(
            ChildState::Exited(Some(255)),
            "Host key verification failed."
        ),
        SshFailureKind::HostKeyUnknown
    );
    assert_eq!(
        classify(ChildState::Exited(Some(127)), ""),
        SshFailureKind::BridgeMissing
    );
    assert_eq!(
        classify(ChildState::Exited(Some(255)), ""),
        SshFailureKind::Other
    );
}

#[test]
fn only_ssh_s_own_status_is_read_by_the_ssh_rules() {
    // Every SSH-level text, under a status ssh itself never exits with for its
    // own failure: the remote bridge's 1, and a signal death (no status).
    let ssh_level = [
        "alice@h: Permission denied (publickey).",
        "Error: Permission denied (os error 13)",
        "Host key verification failed.",
        "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!",
        "Error: Connection refused (os error 111)",
        "ssh: Could not resolve hostname h: Name or service not known",
    ];
    for state in [
        ChildState::Exited(Some(1)),
        ChildState::Exited(Some(2)),
        ChildState::Exited(None),
    ] {
        for text in ssh_level {
            assert_eq!(
                classify(state, text),
                SshFailureKind::Other,
                "{state:?}: {text}"
            );
        }
        // The bridge-missing rules are the remote shell's, so they still apply.
        assert_eq!(
            classify(
                state,
                "sh: 1: /home/alice/.local/bin/biorouter-crew: not found"
            ),
            SshFailureKind::BridgeMissing,
            "{state:?}"
        );
    }
    // The same texts under 255 are ssh's own report.
    assert_eq!(
        classify(ChildState::Exited(Some(255)), ssh_level[0]),
        SshFailureKind::AuthRequired
    );
    assert_eq!(
        classify(ChildState::Exited(Some(255)), ssh_level[4]),
        SshFailureKind::Unreachable
    );
}

#[test]
fn every_kind_has_its_api_code() {
    use SshFailureKind::*;
    for (kind, code) in [
        (AuthRequired, "crew_ssh_auth_required"),
        (HostKeyUnknown, "crew_ssh_host_key_unknown"),
        (HostKeyChanged, "crew_ssh_host_key_changed"),
        (Unreachable, "crew_ssh_unreachable"),
        (BridgeMissing, "crew_bridge_missing"),
        (Other, "crew_ssh_failed"),
    ] {
        assert_eq!(kind.api_code(), code);
    }
}

#[test]
fn sanitize_and_fit_respect_character_boundaries() {
    assert_eq!(sanitize_stderr("\r\n\r\n  \r\nok  \r\n"), "ok");
    assert_eq!(sanitize_stderr("a\u{1b}P1;2|payload\u{1b}\\b"), "ab");
    let text = "é".repeat(2000);
    let fitted = fit(&text, SSH_FAILURE_DETAIL_LIMIT);
    assert!(fitted.len() <= SSH_FAILURE_DETAIL_LIMIT);
    assert!(fitted.contains("\n…\n"));
    assert_eq!(fit("short", SSH_FAILURE_DETAIL_LIMIT), "short");
}

/// T-53: a key the server refuses before this bridge ever answered reads as a sign-in refusal
/// that names the server and login, never as an unknown outcome: SSH refuses a key before it
/// runs the remote command, so no request reached the broker. The kind, and so the typed code
/// and the sign-in flow, is unchanged, and OpenSSH's words stay in the detail. Once the bridge
/// has answered, a later failure keeps the unknown-outcome wording.
#[tokio::test]
async fn a_refused_key_before_any_answer_is_a_plain_sign_in_refusal() {
    let fixture = tempfile::NamedTempFile::new().unwrap();
    fs::write(fixture.path(), PERMISSION_DENIED).unwrap();
    for (login, expected) in [
        (
            "crew_dave@52.33.141.141",
            "Couldn't sign in to 52.33.141.141 as crew_dave: the server refused this computer's SSH key.",
        ),
        (
            "lab-server",
            "Couldn't sign in to lab-server: the server refused this computer's SSH key.",
        ),
    ] {
        let mut transport = spawn_peer(
            r#"IFS= read -r line; cat "$1" >&2; exit 255"#,
            Some(fixture.path()),
        );
        transport.sign_in = Some(super::SignInTarget::from_login(login));
        let error = transport
            .request("hello", request_params(), None, None, Some("hello".into()))
            .await
            .expect_err("a refused key fails the exchange");
        transport.close().await;
        let failure = error.downcast_ref::<SshFailure>().cloned().unwrap();
        assert_eq!(failure.kind, SshFailureKind::AuthRequired);
        assert_eq!(failure.api_code(), "crew_ssh_auth_required");
        assert_eq!(failure.status, "exit_255");
        assert_eq!(error.to_string(), expected);
        assert_eq!(format!("{error:#}"), expected);
        assert!(failure
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Permission denied")));
    }

    let mut answered = spawn_peer(
        r#"IFS= read -r line; printf '%s\n' '{"id":"first","result":{}}'; IFS= read -r line; cat "$1" >&2; exit 255"#,
        Some(fixture.path()),
    );
    answered.sign_in = Some(super::SignInTarget::from_login("crew_dave@52.33.141.141"));
    answered
        .request("hello", request_params(), None, None, Some("first".into()))
        .await
        .unwrap();
    let error = answered
        .request("hello", request_params(), None, None, Some("second".into()))
        .await
        .expect_err("the bridge went away");
    answered.close().await;
    assert_eq!(
        error.to_string(),
        legacy_message("ssh_eof", "exit_255", "SSH connection closed")
    );
}
