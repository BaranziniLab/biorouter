//! BR-71: the MOUNTED `/ui/workspace` socket, end to end over a real loopback
//! TCP connection.
//!
//! `routes::workspace`'s unit tests cover the two pure halves —
//! `check_workspace_ws_auth` and `apply_inbound_frame` — in isolation, and
//! **four different builds that never reach either of them keep those green**:
//!
//! 1. `workspace_ws` without its `check_workspace_ws_auth` call: an entirely
//!    unauthenticated WebSocket, on the daemon's only path that is exempt from
//!    `check_token`;
//! 2. `handle_workspace_socket` without its `apply_inbound_frame` call: a socket
//!    that silently ignores every renderer frame, so `workspace_list` reports an
//!    empty `gui` forever and every round trip burns its full 10 s;
//! 3. `routes::configure` without `.merge(workspace::routes(…))`: the renderer
//!    gets 404;
//! 4. `auth::is_unauthenticated_path` without `"/ui/workspace"`: the renderer
//!    gets 401 — the failure the plan's 2026-07-28 gate amendment was written to
//!    prevent, in the direction it did not close.
//!
//! The single test below fails on all four (measured: 403 vs. a successful
//! upgrade, a dispatch timeout, 404, 401), because it builds the daemon's real
//! router exactly as `commands::agent::run` does — `routes::configure` plus the
//! `check_token` middleware — serves it on a loopback port, and speaks the real
//! handshake. Asserting the *precise* status (403, not merely "refused") is what
//! separates 3 and 4 from a working gate.
//!
//! **Why it is here and not beside the code it tests.** Two constraints meet:
//!
//! - `tower::ServiceExt::oneshot` cannot reach this handler at all. axum's
//!   `WebSocketUpgrade` extractor requires a `hyper::upgrade::OnUpgrade` request
//!   extension that only exists on a connection hyper is really serving
//!   (`axum-0.8.4` `extract/ws.rs::from_request_parts` → `ConnectionNotUpgradable`),
//!   so an in-memory request is rejected with 426 *before* the handler body runs
//!   and the 403 this test is about never happens. It has to be a real socket.
//! - Naming `check_token` from inside `src/routes/` is not possible: that
//!   directory is compiled into both the lib and the `biorouterd` binary
//!   (`main.rs` re-declares the module tree), and only the lib has `mod auth` —
//!   the binary pulls it from the lib, as `commands::agent` does
//!   (`use biorouter_server::auth::check_token`). A `crate::auth::…` path there
//!   is E0433 in the bin's test target and breaks `clippy --all-targets`.
//!
//! So it lives where this crate already puts router-level tests, beside
//! `knowledge_routes.rs` and `llamacpp_routes.rs`.
//!
//! ⚠ **It is therefore NOT reachable from Task 23's `--lib` gate.** Run both:
//!
//! ```text
//! cargo test -p biorouter-server --lib -- routes::workspace workspace::bridge auth::tests
//! cargo test -p biorouter-server --test workspace_socket
//! ```

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message as ClientMessage;

async fn connect(
    addr: std::net::SocketAddr,
    query: &str,
    origin: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsError,
> {
    let mut request = format!("ws://{addr}/ui/workspace{query}")
        .into_client_request()
        .unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(axum::http::header::ORIGIN, origin.parse().unwrap());
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

fn refusal_status(err: WsError) -> axum::http::StatusCode {
    match err {
        WsError::Http(response) => response.status(),
        other => panic!("expected an HTTP refusal, got {other:?}"),
    }
}

/// Poll `condition` until it holds, or fail with `why`. The daemon side of each
/// assertion runs in another task, so every cross-task claim here is a poll.
async fn wait_until(mut condition: impl FnMut() -> bool, why: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if condition() {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "timed out: {why}");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_mounted_socket_authenticates_and_carries_frames_both_ways() {
    let state = biorouter_server::state::AppState::new().await.unwrap();

    let secret = "br71-mounted-socket-secret";
    let app = biorouter_server::routes::configure(state, secret.to_string()).layer(
        axum::middleware::from_fn_with_state(
            secret.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // `BRIDGES` is a process-wide static; own a window nothing else can see.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let window = format!("br71-route-win-{nonce}");
    let good = format!("?secret={secret}&window_id={window}");

    // FORBIDDEN, not UNAUTHORIZED and not NOT_FOUND: 401 means the exemption in
    // `auth::is_unauthenticated_path` is gone and the real renderer can never
    // connect; 404 means the route was never merged into `configure`.
    assert_eq!(
        refusal_status(
            connect(addr, &format!("?secret=nope&window_id={window}"), None)
                .await
                .expect_err("a wrong secret must be refused")
        ),
        axum::http::StatusCode::FORBIDDEN,
        "the socket's own gate must reject a wrong secret, and the query key is `secret`"
    );
    assert_eq!(
        refusal_status(
            connect(addr, &format!("?window_id={window}"), None)
                .await
                .expect_err("no secret at all must be refused")
        ),
        axum::http::StatusCode::FORBIDDEN
    );
    // The Origin header is genuinely read at the call site. `null` is the opaque
    // origin of every sandboxed agent-authored frame this app renders in its
    // artifact side panel — a `srcDoc` iframe sandboxed without
    // `allow-same-origin`.
    assert_eq!(
        refusal_status(
            connect(addr, &good, Some("null"))
                .await
                .expect_err("the opaque origin must be refused")
        ),
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        refusal_status(
            connect(addr, &good, Some("https://evil.com"))
                .await
                .expect_err("a web origin must be refused (CSWSH)")
        ),
        axum::http::StatusCode::FORBIDDEN
    );
    // QA-D F7, over the real handshake: another loopback port, the daemon's
    // own address under another scheme, and another loopback host. Each was
    // admitted until then — the first two by "any loopback origin", which is
    // why this test's own success case used to connect from port 5173. Nothing
    // here declares a renderer origin, so vite's page is just another port.
    //
    // ⚠ `file://` is in that list, and it is the one the security review of QA-D
    // F7 added. This binary declares no renderer — `BIOROUTER_RENDERER_ORIGIN`
    // is unset, which is what `biorouter serve` and a hand-run `biorouterd` look
    // like — and the gate used to admit that literal by name on EVERY daemon, so
    // a local `.html` opened in Chromium cleared it here. The packaged app's own
    // page is admitted by DECLARING it, which `routes::workspace`'s unit tests
    // cover: the declaration is read once into a process-global `LazyLock`, so it
    // cannot be varied per connection from one test binary.
    for origin in [
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:5173".to_string(),
        format!("https://{addr}"),
        format!("http://localhost:{}", addr.port()),
        "file://".to_string(),
        "null".to_string(),
    ] {
        assert_eq!(
            refusal_status(
                connect(addr, &good, Some(&origin))
                    .await
                    .expect_err("an origin that is not the daemon's own must be refused")
            ),
            axum::http::StatusCode::FORBIDDEN,
            "{origin}"
        );
    }

    // …and the real thing connects: a page served from the daemon's own
    // origin, which the handshake's `Host` names.
    let mut socket = connect(addr, &good, Some(&format!("http://{addr}")))
        .await
        .expect("the daemon's own origin with the right secret upgrades");

    let bridge = biorouter_server::workspace::bridge::bridge_for(&window);

    // INBOUND: a renderer frame reaches the bridge through the live loop.
    //
    // The `window_id` in the payload is a LIE, deliberately. A window's identity
    // is the connection's, and `merged_layout()` hands these echoes to the model
    // as `workspace_list`'s `gui` — where the id is what the agent then targets
    // commands at. A client-asserted id would let one authenticated window
    // impersonate another in the model's view of the workspace, and would
    // disagree with the `BRIDGES` key the echo is actually stored under.
    socket
        .send(ClientMessage::Text(
            serde_json::json!({
                "type": "workspace_echo",
                "window_id": "w-impersonated",
                "focused_session": "s-live",
                "layout": [],
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    wait_until(
        || {
            bridge.last_echo().and_then(|e| {
                e.get("focused_session")
                    .and_then(|f| f.as_str())
                    .map(String::from)
            }) == Some("s-live".to_string())
        },
        "the socket loop must dispatch an inbound workspace_echo into the bridge",
    )
    .await;
    assert_eq!(
        bridge.last_echo().unwrap()["window_id"],
        serde_json::Value::String(window.clone()),
        "the daemon must stamp the CONNECTION's window_id over the payload's claim"
    );

    // OUTBOUND: `attach`'s sender is really wired to this socket.
    bridge
        .emit(serde_json::json!({"cmd": "workspace_probe"}))
        .expect("the handler must have attached this window's bridge");
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("an emitted command must reach the socket")
        .expect("the socket is still open")
        .unwrap();
    let ClientMessage::Text(text) = received else {
        panic!("expected a text frame, got {received:?}");
    };
    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(frame["cmd"], "workspace_probe");

    // An OVERSIZED frame is dropped, and the socket survives it. `store_echo`'s
    // value is handed to the model verbatim as `workspace_list`'s `gui`, so an
    // uncapped frame is both unbounded daemon memory and an unbounded injection
    // into the agent's context. Dropping the frame rather than the connection is
    // deliberate: the echo is a periodic report, and one bad frame must not cost
    // the user their window channel.
    socket
        .send(ClientMessage::Text(
            serde_json::json!({
                "type": "workspace_echo",
                "focused_session": "s-oversized",
                "layout": ["x".repeat(200 * 1024)],
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // A full round trip issued AFTER it. WebSocket frames arrive in order on one
    // connection and the loop reads them in order, so once this resolves the
    // oversized frame has provably already been handled — which is what makes
    // the assertion below a fact rather than a race with a sleep in it.
    let waiter = {
        let bridge = bridge.clone();
        tokio::spawn(async move {
            bridge
                .emit_and_wait(
                    serde_json::json!({"cmd": "workspace_probe_2"}),
                    std::time::Duration::from_secs(5),
                )
                .await
        })
    };
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("the parked command must reach the socket")
        .expect("the socket is still open")
        .unwrap();
    let ClientMessage::Text(text) = received else {
        panic!("expected a text frame, got {received:?}");
    };
    let parked: serde_json::Value = serde_json::from_str(&text).unwrap();
    let request_id = parked["request_id"].as_str().expect("a minted request_id");
    socket
        .send(ClientMessage::Text(
            serde_json::json!({
                "type": "workspace_result",
                "request_id": request_id,
                "ok": true,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let resolved = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .expect("the renderer's workspace_result must unpark the round trip")
        .unwrap()
        .unwrap();
    assert_eq!(resolved["ok"], true);

    assert_eq!(
        bridge.last_echo().unwrap()["focused_session"],
        "s-live",
        "an oversized workspace_echo must be dropped, leaving the last good one"
    );

    // …and closing the window really detaches it, so `workspace_list` stops
    // claiming the user can see it.
    socket.close(None).await.unwrap();
    wait_until(
        || !bridge.is_attached(),
        "a closed socket must detach its bridge",
    )
    .await;
}
