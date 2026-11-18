//! The remote tunnel on a daemon that holds no proof-of-user key: the one
//! `biorouter serve` starts (SD-7), `just run-server` and a hand-run
//! `biorouterd agent`.
//!
//! Measured before the gate (fix wave p1w2, GAP-SV-01): on a keyless daemon,
//! `POST /tunnel/start` holding only `X-Secret-Key` answered 200, opened a
//! relay to the daemon and saved `tunnel_auto_start: true`, and
//! `GET /tunnel/status` handed the same caller the relay URL and the tunnel's
//! secret. On a `serve` host that secret sits in the daemon's environment,
//! where a chat's shell reads it with `ps eww`, so the caller could be a model.
//! Opening the daemon to the internet is the person's decision, and this daemon
//! cannot tell a person from a program, so it refuses both start and stop, and
//! status shows the state alone.
//!
//! Its own test binary on purpose: the installed digest is a process-global
//! `OnceLock`, and `tunnel_requires_user.rs` installs one. Nothing here installs
//! a digest, which is how `biorouter serve` starts its daemon.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so the refused writes below are checked against a config.yaml
// that is this binary's own.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use biorouter::config::Config;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::tunnel::{TUNNEL_NEEDS_USER, TUNNEL_NO_USER_KEY};
use biorouter_server::state::AppState;
use biorouter_server::tunnel::TunnelState;
use serial_test::serial;
use tower::ServiceExt;

/// The server secret this binary's "daemon" was launched with.
const TEST_SECRET: &str = "tunnel-no-user-key";

/// A relay address nothing listens on, so a tunnel that does start reaches no one.
const DEAD_RELAY: &str = "http://127.0.0.1:9";

fn assert_the_daemon_is_keyless() {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
}

fn dead_relay_env() -> env_lock::EnvGuard<'static> {
    env_lock::lock_env([
        ("BIOROUTER_TUNNEL_WORKER_URL", Some(DEAD_RELAY)),
        ("BIOROUTER_TUNNEL", None),
    ])
}

fn auto_start() -> Option<bool> {
    Config::global().get_param::<bool>("tunnel_auto_start").ok()
}

/// The request a page or a shell on a `serve` host can send: the secret, the
/// host's provider stated the way the browser states it, and a guess at
/// `X-User-Action`. Layered through the same `check_token` middleware the real
/// daemon installs.
async fn send(state: Arc<AppState>, method: &str, uri: &str) -> (StatusCode, serde_json::Value) {
    let app = biorouter_server::routes::tunnel::routes(state).layer(
        axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET)
        .header("X-Caller-Provider", "versa_azure")
        .header("X-User-Action", "biorouter-dev-user-action")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into())),
    )
}

fn error_of(body: &serde_json::Value) -> &str {
    body.get("error")
        .and_then(|error| error.as_str())
        .unwrap_or_else(|| panic!("the refusal is not in the route's error envelope: {body}"))
}

/// Fails on the ungated routes: start answers 200, the manager leaves Idle, and
/// `tunnel_auto_start` is saved.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_refuses_tunnel_start_and_stop_with_a_sentence() {
    assert_the_daemon_is_keyless();
    let _env = dead_relay_env();
    let _ = Config::global().delete("tunnel_auto_start");
    let state = AppState::new().await.unwrap();

    for route in ["/tunnel/start", "/tunnel/stop"] {
        let (status, body) = send(state.clone(), "POST", route).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a keyless daemon answered `{route}` for a caller it cannot tell from a model: {body}"
        );
        assert_eq!(error_of(&body), TUNNEL_NO_USER_KEY);
        assert_ne!(
            error_of(&body),
            TUNNEL_NEEDS_USER,
            "the person on a keyless daemon was handed the sentence for a desktop daemon"
        );
        assert_eq!(
            state.tunnel_manager.get_info().await.state,
            TunnelState::Idle,
            "the refused `{route}` moved the tunnel anyway"
        );
        assert_eq!(
            auto_start(),
            None,
            "the refused `{route}` saved tunnel_auto_start anyway"
        );
    }

    state.tunnel_manager.stop(false).await;
}

/// A tunnel that is up on this daemon (here: auto-started at boot, the way a
/// `tunnel_auto_start: true` saved on the desktop would start it) shows its
/// state to a secret-only caller and nothing that lets anyone use it.
///
/// Fails on the ungated status route: the body carries the relay URL, the
/// hostname and the tunnel's secret.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn status_withholds_the_url_and_secret_without_proof() {
    assert_the_daemon_is_keyless();
    let _env = dead_relay_env();
    Config::global()
        .set_param("tunnel_auto_start", true)
        .unwrap();
    let state = AppState::new().await.unwrap();
    state.tunnel_manager.check_auto_start().await;
    assert_ne!(
        state.tunnel_manager.get_info().await.state,
        TunnelState::Idle,
        "the tunnel did not auto-start, so the status check below would measure nothing"
    );
    assert!(
        !state.tunnel_manager.get_info().await.url.is_empty(),
        "the manager holds no URL, so a blank status would prove nothing"
    );

    let (status, body) = send(state.clone(), "GET", "/tunnel/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["state"].is_string(),
        "status stopped reporting the state: {body}"
    );
    for field in ["url", "hostname", "secret"] {
        assert_eq!(
            body[field], "",
            "status handed `{field}` to a caller that did not prove it is the person: {body}"
        );
    }

    state.tunnel_manager.stop(true).await;
    let _ = Config::global().delete("tunnel_auto_start");
}
