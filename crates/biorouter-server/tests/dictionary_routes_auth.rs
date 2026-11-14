//! Every `/dictionary/*` route as the daemon serves it, merged by `routes::configure` and
//! behind the same `check_token` middleware `commands/agent.rs` wraps the router in
//! (PR-16): without the daemon secret, or with a wrong one, every route is refused (401,
//! or 429 once this address has failed too often in its window) before any handler or the
//! entry gate runs, and nothing is written; with it, every route is there.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use biorouter::security::dictionary_store;
use biorouter_server::state::AppState;
use serde_json::json;
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictionary-auth-secret";

/// Every route and method, with a body that would be admitted with the secret.
fn every_route(id: &str) -> Vec<(&'static str, String, Option<serde_json::Value>)> {
    vec![
        ("GET", "/dictionary/entries".into(), None),
        (
            "POST",
            "/dictionary/entries".into(),
            Some(json!({"written": "EPAS1"})),
        ),
        (
            "PATCH",
            format!("/dictionary/entries/{id}"),
            Some(json!({"note": "a gene"})),
        ),
        ("DELETE", format!("/dictionary/entries/{id}"), None),
        (
            "POST",
            format!("/dictionary/entries/{id}/tier"),
            Some(json!({"tier": "private"})),
        ),
        (
            "POST",
            "/dictionary/entries/delete".into(),
            Some(json!({"ids": [id]})),
        ),
        (
            "POST",
            "/dictionary/entries/keep".into(),
            Some(json!({"ids": [id]})),
        ),
        (
            "POST",
            "/dictionary/suggestions/accept".into(),
            Some(json!({"ids": [id]})),
        ),
        (
            "POST",
            "/dictionary/suggestions/dismiss".into(),
            Some(json!({"ids": [id]})),
        ),
        (
            "POST",
            "/dictionary/import".into(),
            Some(json!({"format": "lines", "content": "UCSF"})),
        ),
        ("GET", "/dictionary/export".into(), None),
        (
            "POST",
            "/dictionary/observations".into(),
            Some(json!({"session_id": null, "observations": []})),
        ),
        ("GET", "/dictionary/settings".into(), None),
        (
            "POST",
            "/dictionary/settings".into(),
            Some(json!({"agent_writes": "suggest"})),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn every_route_takes_the_daemon_secret() {
    let _ = std::fs::remove_dir_all(dictionary_store::dictionary_dir());
    let state = AppState::new().await.expect("app state");
    let app = biorouter_server::routes::configure(Arc::clone(&state), SECRET.to_string()).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let id = biorouter::security::dictionary_store::store::EntryId::random().to_string();
    let mut first = true;
    for (method, uri, body) in every_route(&id) {
        for secret in [None, Some("not-the-secret")] {
            let mut builder = Request::builder()
                .method(method)
                .uri(&uri)
                .header("content-type", "application/json");
            if let Some(secret) = secret {
                builder = builder.header("X-Secret-Key", secret);
            }
            let body = body
                .as_ref()
                .map_or_else(Body::empty, |b| Body::from(b.to_string()));
            let response = app
                .clone()
                .oneshot(builder.body(body).unwrap())
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
    assert!(
        !dictionary_store::dictionary_dir().exists(),
        "a refused request wrote nothing"
    );
    for (method, uri, body) in every_route(&id) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&uri)
                    .header("content-type", "application/json")
                    .header("X-Secret-Key", SECRET)
                    .body(
                        body.as_ref()
                            .map_or_else(Body::empty, |b| Body::from(b.to_string())),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert!(
            ![
                StatusCode::UNAUTHORIZED,
                StatusCode::TOO_MANY_REQUESTS,
                StatusCode::NOT_FOUND,
                StatusCode::METHOD_NOT_ALLOWED,
            ]
            .contains(&status)
                || (status == StatusCode::NOT_FOUND && uri.contains(&id)),
            "{method} {uri} with the secret answered {status}"
        );
    }
}
