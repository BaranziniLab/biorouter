//! Every `/dictation/*` route as the daemon serves it: merged by `routes::configure` and
//! behind the same `check_token` middleware `commands/agent.rs` wraps the router in
//! (DD-29). Without the secret, every route answers 401 and nothing happens; with it,
//! the routes are there.
//!
//! PR-25 adds the daemon binary contract of P-10 to this binary.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{middleware, Router};
use biorouter_server::auth::check_token;
use tower::ServiceExt;

const SECRET: &str = "dictation-auth-secret";

async fn daemon() -> Router {
    let state = biorouter_server::state::AppState::new()
        .await
        .expect("app state");
    biorouter_server::routes::configure(state, SECRET.to_string()).layer(
        middleware::from_fn_with_state(SECRET.to_string(), check_token),
    )
}

/// Every route of the contract's table, with a body the route would take.
fn routes() -> Vec<(&'static str, String, Body)> {
    let id = "00000000000000000000000000000000";
    let json = |value: serde_json::Value| Body::from(value.to_string());
    vec![
        ("GET", "/dictation/status".into(), Body::empty()),
        (
            "POST",
            "/dictation/streams".into(),
            json(
                serde_json::json!({"sample_rate": 16000, "format": "s16le", "target_kind": "external"}),
            ),
        ),
        (
            "PUT",
            format!("/dictation/streams/{id}/audio?seq=0"),
            Body::from(vec![0u8; 3200]),
        ),
        (
            "POST",
            format!("/dictation/streams/{id}/finish"),
            Body::empty(),
        ),
        ("GET", format!("/dictation/streams/{id}"), Body::empty()),
        ("DELETE", format!("/dictation/streams/{id}"), Body::empty()),
        (
            "POST",
            "/dictation/models/install".into(),
            json(serde_json::json!({"model": "whisper-small.en-q8_0"})),
        ),
        (
            "POST",
            "/dictation/models/cancel".into(),
            json(serde_json::json!({"model": "whisper-small.en-q8_0"})),
        ),
        (
            "POST",
            "/dictation/models/delete".into(),
            json(serde_json::json!({"model": "whisper-small.en-q8_0"})),
        ),
        (
            "POST",
            "/dictation/models/select".into(),
            json(serde_json::json!({"language": "en"})),
        ),
        ("POST", "/dictation/models/rescan".into(), Body::empty()),
        ("POST", "/dictation/bench".into(), Body::empty()),
    ]
}

fn request(method: &str, uri: &str, body: Body, secret: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if method != "GET" && method != "DELETE" {
        let content = if uri.contains("/audio") {
            "application/octet-stream"
        } else {
            "application/json"
        };
        builder = builder.header("content-type", content);
    }
    if let Some(secret) = secret {
        builder = builder.header("X-Secret-Key", secret);
    }
    builder.body(body).unwrap()
}

/// DD-29: the secret on every route. A wrong secret is no secret. `check_token` answers
/// 401, and 429 once an address has failed too often in its window; both refuse.
#[tokio::test]
#[serial_test::serial]
async fn every_dictation_route_refuses_a_request_without_the_secret() {
    let app = daemon().await;
    let mut first = true;
    for secret in [None, Some("not-the-secret")] {
        for (method, uri, body) in routes() {
            let response = app
                .clone()
                .oneshot(request(method, &uri, body, secret))
                .await
                .unwrap();
            let status = response.status();
            if first {
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
                first = false;
            }
            assert!(
                status == StatusCode::UNAUTHORIZED || status == StatusCode::TOO_MANY_REQUESTS,
                "{method} {uri} with {secret:?} answered {status}"
            );
        }
    }
}

/// The routes are registered on the daemon: with the secret, none of them is missing
/// (404) or answered by another route's method (405). The stream routes name a stream
/// that does not exist and answer 410 or 204; the model routes answer what this
/// sandbox, which has no models, makes of them.
#[tokio::test]
#[serial_test::serial]
async fn every_dictation_route_is_served_with_the_secret() {
    let app = daemon().await;
    for (method, uri, body) in routes() {
        // Never start a download from a test: install is the only route that could.
        if uri.ends_with("/install") {
            continue;
        }
        let response = app
            .clone()
            .oneshot(request(method, &uri, body, Some(SECRET)))
            .await
            .unwrap();
        let status = response.status();
        assert!(
            status != StatusCode::NOT_FOUND
                && status != StatusCode::METHOD_NOT_ALLOWED
                && status != StatusCode::UNAUTHORIZED,
            "{method} {uri} answered {status}"
        );
    }
}
