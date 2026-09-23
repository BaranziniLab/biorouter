//! Explicit live-broker acceptance for source ACL revocation during observation.
//!
//! This test is ignored by default. It requires a disposable Bob profile in
//! `BIOROUTER_PATH_ROOT`, plus an independently configured Alice profile and a
//! pre-seeded derived frame. The setup is deliberately performed by the
//! supported CLI/API outside this process; the test only consumes the real
//! manager and receiver paths under test.

use super::*;
use std::{env, fs, future::Future, process::Stdio, sync::Arc};
use tokio::io::AsyncWriteExt;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("live acceptance requires {name}"))
}

fn proven_headers() -> HeaderMap {
    crate::routes::session::diverge_tests::install_test_user_action_key();
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-User-Action",
        crate::routes::session::diverge_tests::TEST_USER_ACTION_KEY
            .parse()
            .expect("test proof is a valid header value"),
    );
    headers
}

fn observer_for(
    connection: &biorouter::crew::Connection,
    channel: String,
    cursor: String,
    permit: Option<OwnedSemaphorePermit>,
) -> Observer {
    Observer {
        headers: proven_headers(),
        connection: connection.id.clone(),
        request: ObserveRequest {
            channel_id: Some(channel),
            after: Some(cursor.clone()),
            initial: Initial::Latest,
        },
        cursor: Some(cursor),
        pending: VecDeque::new(),
        binding: connection_binding(connection).expect("saved connection serializes"),
        epoch: None,
        first: false,
        state_due: false,
        sleep_due: false,
        last_state: None,
        limit: 200,
        deadline: tokio::time::Instant::now() + Duration::from_secs(60),
        done: false,
        _permit: permit,
    }
}

async fn revoke_source() {
    let bin = required("BIOROUTER_LIVE_BIN");
    let alice_root = required("BIOROUTER_LIVE_ALICE_ROOT");
    let alice_connection = required("BIOROUTER_LIVE_ALICE_CONNECTION");
    let source = required("BIOROUTER_LIVE_SOURCE_CHANNEL");
    let principal = required("BIOROUTER_LIVE_BOB_PRINCIPAL");
    let approval_file = required("BIOROUTER_LIVE_ALICE_APPROVAL_FILE");
    let approval = fs::read_to_string(approval_file).expect("read live approval from fixture file");
    let mut command = tokio::process::Command::new(bin);
    command
        .args([
            "crew",
            "--connection",
            &alice_connection,
            "--no-start",
            "--approval-key-stdin",
            "remove-member",
            &source,
            &principal,
        ])
        .env("BIOROUTER_PATH_ROOT", alice_root)
        .env(
            "BIOROUTER_DEV_PROFILE_ROOT",
            required("BIOROUTER_LIVE_ALICE_PROFILE_ROOT"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    biorouter_mcp::developer::shell::no_console_window(&mut command);
    let mut child = command
        .spawn()
        .expect("spawn ordinary Alice revoke command");
    child
        .stdin
        .take()
        .expect("approval stdin")
        .write_all(approval.as_bytes())
        .await
        .expect("write approval to child");
    let output = child
        .wait_with_output()
        .await
        .expect("wait for Alice revoke");
    assert!(output.status.success(), "ordinary Alice revoke failed");
}

#[tokio::test]
#[ignore = "requires an explicitly provisioned disposable three-user Crew broker"]
async fn real_source_acl_revocation_clears_enqueued_and_waiting_observer_frames() {
    let frame: Bytes = fs::read(required("BIOROUTER_LIVE_FRAME_FILE"))
        .expect("read pre-seeded derived frame")
        .into();
    let frame_value: Value = serde_json::from_slice(&frame).expect("derived frame is JSON");
    let canary = required("BIOROUTER_LIVE_CANARY");
    assert!(frame_value.to_string().contains(&canary));
    let message = frame_value["messages"]
        .as_array()
        .and_then(|messages| messages.first())
        .expect("derived frame contains one real message");
    assert_eq!(
        message["id"].as_str(),
        Some(required("BIOROUTER_LIVE_MESSAGE_ID").as_str())
    );
    assert_eq!(
        message["body"].as_str(),
        Some(required("BIOROUTER_LIVE_MESSAGE_BODY").as_str())
    );
    assert_eq!(
        frame_value["cursor"].as_str(),
        Some(required("BIOROUTER_LIVE_FRAME_CURSOR").as_str())
    );
    let cursor = required("BIOROUTER_LIVE_CURSOR");
    let channel = required("BIOROUTER_LIVE_DEST_CHANNEL");
    let connection_id = required("BIOROUTER_LIVE_BOB_CONNECTION");
    let crew = manager().expect("Bob manager initializes");
    let connection = crew
        .connection(&connection_id)
        .await
        .expect("Bob connection exists");

    let mut positive = observer_for(&connection, channel.clone(), cursor.clone(), None);
    assert_eq!(
        positive
            .admit_frame(&frame, &CancellationToken::new())
            .await
            .unwrap(),
        frame_value
    );

    let (sender, receiver) = mpsc::channel(1);
    sender
        .try_send(frame.clone())
        .expect("first frame is queued");
    let waiting_sender = sender.clone();
    let waiting_frame = frame.clone();
    let (attempted_sender, attempted) = oneshot::channel();
    let waiting = tokio::spawn(async move {
        let mut send = Box::pin(waiting_sender.send(waiting_frame));
        let mut attempted_sender = Some(attempted_sender);
        futures::future::poll_fn(move |cx| {
            let result = send.as_mut().poll(cx);
            if result.is_pending() {
                if let Some(sender) = attempted_sender.take() {
                    let _ = sender.send(());
                }
            }
            result
        })
        .await
    });
    drop(sender);
    attempted
        .await
        .expect("second send was observed pending on full queue");

    revoke_source().await;

    let observer = Arc::new(Mutex::new(observer_for(
        &connection,
        channel.clone(),
        cursor,
        Some(SLOTS.clone().try_acquire_owned().expect("observer slot")),
    )));
    let mut receiver = ObservationReceiver {
        receiver,
        terminal: None,
        deferred_terminal: None,
        observer,
        cancel: CancellationToken::new(),
        finished: false,
    };
    let terminal = receiver
        .next_frame()
        .await
        .expect("revocation emits clear frame");
    let terminal_value: Value = serde_json::from_slice(&terminal).expect("terminal frame is JSON");
    assert_eq!(terminal_value["type"], "error");
    assert!(matches!(
        terminal_value["code"].as_str(),
        Some("stale_cursor" | "policy_changed")
    ));
    assert_eq!(terminal_value["clear"], true);
    assert!(!terminal
        .windows(canary.len())
        .any(|window| window == canary.as_bytes()));
    assert!(receiver.next_frame().await.is_none());
    let _waiting_result = waiting.await.expect("waiting sender task");
    assert!(
        receiver.receiver.try_recv().is_err(),
        "queued waiting frame must be drained"
    );

    crew.human_request(
        &connection_id,
        "messages.history",
        json!({"channel_id": channel, "limit": 1, "latest": true}),
        None,
    )
    .await
    .expect("Bob destination channel remains usable after source revocation");
}
