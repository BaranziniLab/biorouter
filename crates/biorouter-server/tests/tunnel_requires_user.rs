//! The remote tunnel on a daemon that HOLDS a user-action key (the desktop
//! application's) opens and closes only for a request that proves the person
//! at the keyboard sent it, and its status shows the relay URL and the tunnel's
//! secret only to that request.
//!
//! Measured before the gate (fix wave p1w2, GAP-SV-01): `POST /tunnel/start`
//! holding only `X-Secret-Key` answered 200, opened a public relay to the
//! daemon and saved `tunnel_auto_start: true`, and `GET /tunnel/status` handed
//! that caller the URL and secret. The daemon secret is recoverable from inside
//! a public chat's shell (AR-11), so that was a model able to expose the daemon
//! to the internet, and to keep it exposed across restarts.
//!
//! Its own binary, because the installed digest is a process-global `OnceLock`:
//! this one installs a key, `tunnel_no_user_key.rs` must not. The relay is an
//! address nothing listens on, so a tunnel that does start reaches no one.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so tunnel.lock and tunnel_auto_start land in this binary's own.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use biorouter::config::Config;
use biorouter_server::routes::tunnel::{TUNNEL_NEEDS_USER, TUNNEL_NO_USER_KEY};
use biorouter_server::state::AppState;
use biorouter_server::tunnel::TunnelState;
use serial_test::serial;
use tower::ServiceExt;

/// The server secret this binary's "daemon" was launched with.
const TEST_SECRET: &str = "tunnel-gate-secret";

/// The user-action key the desktop app would hold. The daemon keeps only its
/// digest, exactly as `commands::agent` installs one read off stdin.
const USER_KEY: &str = "tunnel-gate-user-action-key";

/// A relay address nothing listens on.
const DEAD_RELAY: &str = "http://127.0.0.1:9";

fn install_the_desktop_key() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-User-Action", USER_KEY.parse().unwrap());
    assert!(
        biorouter_server::auth::is_user_action(&headers),
        "the digest this binary installs did not take, so every positive arm below would be \
         measuring a refusal"
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

/// What a request carries besides the secret.
#[derive(Clone, Copy, Debug)]
enum Credential {
    /// Nothing: the caller AR-11 establishes, indistinguishable from a model.
    SecretOnly,
    /// A key that is not the one this daemon holds the digest of.
    WrongKey,
    /// A stated PRIVATE capability, which the reach gate honours for reading a
    /// chat and this gate must not honour for opening the daemon to the internet.
    PrivateCapability,
    /// The desktop app's proof.
    Proof,
}

async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    credential: Credential,
) -> (StatusCode, serde_json::Value) {
    let app = biorouter_server::routes::tunnel::routes(state.clone()).layer(
        axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET);
    let builder = match credential {
        Credential::SecretOnly => builder,
        Credential::WrongKey => builder.header("X-User-Action", "not-the-desktop-key"),
        Credential::PrivateCapability => builder.header("X-Caller-Provider", "versa_azure"),
        Credential::Proof => builder.header("X-User-Action", USER_KEY),
    };
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
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

const UNPROVEN: [Credential; 3] = [
    Credential::SecretOnly,
    Credential::WrongKey,
    Credential::PrivateCapability,
];

/// Fails on the ungated routes: the first start answers 200 and saves
/// `tunnel_auto_start`.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_secret_only_caller_cannot_start_the_tunnel() {
    install_the_desktop_key();
    let _env = dead_relay_env();
    let _ = Config::global().delete("tunnel_auto_start");
    let state = AppState::new().await.unwrap();

    for credential in UNPROVEN {
        for route in ["/tunnel/start", "/tunnel/stop"] {
            let (status, body) = send(&state, "POST", route, credential).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "`{route}` answered a {credential:?} caller that did not prove it is the user: \
                 {body}"
            );
            assert_eq!(error_of(&body), TUNNEL_NEEDS_USER);
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
    }

    state.tunnel_manager.stop(false).await;
}

/// A tunnel the person opened stays open when a secret-only caller asks to
/// close it, and its status shows the URL and secret only to the person.
///
/// Fails on the ungated routes: the secret-only stop answers 200 and closes it.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_secret_only_caller_can_neither_close_the_tunnel_nor_read_its_secret() {
    install_the_desktop_key();
    let _env = dead_relay_env();
    let state = AppState::new().await.unwrap();

    let (status, body) = send(&state, "POST", "/tunnel/start", Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for credential in UNPROVEN {
        let (status, body) = send(&state, "GET", "/tunnel/status", credential).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["state"].is_string(), "{body}");
        for field in ["url", "hostname", "secret"] {
            assert_eq!(
                body[field], "",
                "status handed `{field}` to a {credential:?} caller: {body}"
            );
        }

        let (status, body) = send(&state, "POST", "/tunnel/stop", credential).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(error_of(&body), TUNNEL_NEEDS_USER);
        assert_ne!(
            state.tunnel_manager.get_info().await.state,
            TunnelState::Idle,
            "a {credential:?} caller closed the tunnel the person opened"
        );
        assert_eq!(
            auto_start(),
            Some(true),
            "the refused stop cleared auto-start"
        );
    }

    let (status, body) = send(&state, "POST", "/tunnel/stop", Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = Config::global().delete("tunnel_auto_start");
}

/// The positive control, without which every refusal above could be a route
/// that simply stopped working.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_person_can_start_and_stop_it() {
    install_the_desktop_key();
    let _env = dead_relay_env();
    let _ = Config::global().delete("tunnel_auto_start");
    let state = AppState::new().await.unwrap();

    let (status, body) = send(&state, "POST", "/tunnel/start", Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["url"].as_str().is_some_and(|url| !url.is_empty()),
        "{body}"
    );
    assert_eq!(auto_start(), Some(true));

    let (status, body) = send(&state, "GET", "/tunnel/status", Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for field in ["url", "hostname", "secret"] {
        assert!(
            body[field].as_str().is_some_and(|value| !value.is_empty()),
            "the person's status lost `{field}`: {body}"
        );
    }
    // Nothing listens on the relay address, so the tunnel is not up, and the
    // daemon must not say it is.
    assert_ne!(
        body["state"], "running",
        "status reports a tunnel whose relay refused the connection as running: {body}"
    );

    let (status, body) = send(&state, "POST", "/tunnel/stop", Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        state.tunnel_manager.get_info().await.state,
        TunnelState::Idle
    );
    assert_eq!(auto_start(), Some(false));
    let _ = Config::global().delete("tunnel_auto_start");
}

/// This binary installs a key, so the keyless sentence must never reach it; if
/// it did, every assertion above would be measuring a different daemon.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_daemon_with_a_key_never_says_it_has_none() {
    install_the_desktop_key();
    let _env = dead_relay_env();
    let state = AppState::new().await.unwrap();
    let (status, body) = send(&state, "POST", "/tunnel/start", Credential::SecretOnly).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_ne!(error_of(&body), TUNNEL_NO_USER_KEY);
    state.tunnel_manager.stop(false).await;
}
