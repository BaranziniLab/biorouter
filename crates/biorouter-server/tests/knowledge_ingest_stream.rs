//! Terminal-frame contract for the SSE ingest stream (issue #71).
//!
//! A digest either wrote knowledge or it did not, and the stream has to say
//! which. The reported bug was that it always said `done`: a provider request
//! that failed mid-run ends the sub-agent turn with no tool calls, the macro
//! read that as "the agent is finished", squash-committed an unchanged tree and
//! the route emitted `event: done`. The user was told the PDF had been digested
//! into a knowledge base that had gained nothing.
//!
//! This file lives on its own so it gets its own test *process*:
//! `BIOROUTER_KNOWLEDGE_TEST_MODE` is read by `build_completer`, and
//! `knowledge_routes.rs` has several tests that assert the un-mocked provider
//! path (`ingest_rejects_invalid_model_with_400` and friends). Setting the
//! variable beside them would race.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{body::Body, http::Request};
use biorouter_mcp::knowledge::service::KnowledgeService;
use std::sync::Arc;
use tower::ServiceExt;

/// Issue #56 / the adversarial review of 2026-09-12: `POST /bases` is gated like
/// every other base-naming route, because refusing a taken id told a secret-only
/// caller which PRIVATE ids were taken. So every test here that mints a base
/// speaks as the person at the keyboard — which is who mints one in the product.
const TEST_USER_ACTION_KEY: &str = "knowledge-ingest-stream-user-action-key";

fn install_test_user_action_key() {
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(TEST_USER_ACTION_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
}

fn build_app() -> (tempfile::TempDir, axum::Router) {
    install_test_user_action_key();
    let dir = tempfile::tempdir().unwrap();
    let svc = Arc::new(KnowledgeService::new(dir.path().to_path_buf()));
    let app = biorouter_server::routes::knowledge::router(svc);
    (dir, app)
}

async fn create_kb(app: &axum::Router, id: &str) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({"id": id, "name": id})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "creating the knowledge base");
}

/// Drive `POST /bases/{id}/ingest` to completion and return the whole SSE body.
async fn ingest_stream(app: &axum::Router, kb: &str) -> String {
    let body = serde_json::to_vec(&serde_json::json!({
        "source": {"text": "Zone-2 training raises heart-rate variability.", "title": "HRV note"},
        // Any provider: test mode short-circuits `build_completer`.
        "model": {"provider": "google", "model": "gemini-3.5-flash-lite"}
    }))
    .unwrap();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bases/{kb}/ingest"))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "the ingest route opens an SSE stream");

    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The terminal `event: done` frame's data, parsed.
///
/// Split by the same two markers the error-frame reader below uses — the frame
/// name, then the blank line that ends an SSE frame — so a `done` payload that
/// grew a field is read whole rather than to the first newline.
fn done_payload(body: &str) -> serde_json::Value {
    let payload = body
        .split("event: done\ndata: ")
        .nth(1)
        .expect("done frame payload")
        .split("\n\n")
        .next()
        .expect("done frame terminator");
    serde_json::from_str(payload).expect("the done frame must be valid JSON")
}

/// Both phases live in one test function on purpose: they need different values
/// of the same process-wide environment variable, and `#[tokio::test]` bodies in
/// a single binary run concurrently.
#[tokio::test]
async fn the_ingest_stream_reports_a_failed_digest_as_an_error_not_as_done() {
    // ── Phase 1: a digest that really writes a page ends in `event: done` ─────
    //
    // Without this half the failure assertion below would also pass against a
    // route that emitted `event: error` unconditionally.
    std::env::set_var("BIOROUTER_KNOWLEDGE_TEST_MODE", "1");
    let (dir_ok, app_ok) = build_app();
    create_kb(&app_ok, "ok").await;
    let body = ingest_stream(&app_ok, "ok").await;

    assert!(
        body.contains("event: done"),
        "a digest that wrote a page must end in a done frame; stream was:\n{body}"
    );
    assert!(
        !body.contains("event: error"),
        "a successful digest must not emit an error frame; stream was:\n{body}"
    );
    // Any curated page under `knowledge/<kind>/`, rather than one hardcoded
    // directory. The pre-OKF layout was `knowledge/sources/`; current OKF writes
    // `knowledge/source/` and BioOKF `knowledge/concept/`, so naming one of them
    // makes this assertion fail whenever the default format moves — which is
    // exactly what happened: the fixture migrated to the current paths and this
    // check kept looking for the retired one, reporting a working digest as a
    // digest that wrote nothing.
    let knowledge_root = dir_ok.path().join("ok/knowledge");
    let wrote_a_page = std::fs::read_dir(&knowledge_root)
        .map(|kinds| {
            kinds.filter_map(Result::ok).any(|kind| {
                std::fs::read_dir(kind.path())
                    .map(|mut pages| {
                        pages.any(|page| {
                            page.map(|page| page.path().extension().is_some_and(|ext| ext == "md"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    assert!(
        wrote_a_page,
        "phase 1 must actually leave a curated page behind, else it proves nothing; \
         {} held none",
        knowledge_root.display()
    );

    // …and the done frame carries the macro's tail VERIFICATION, which is the
    // only place the user learns what the digest actually left behind. A run
    // that wrote non-conformant pages still ends in `done` — findings describe
    // the base, they do not fail the run (DR-7) — so without this field a
    // fifteen-bad-page digest and a clean one produce the same stream.
    let done = done_payload(&body);
    let verification = &done["verification"];
    assert!(
        verification.is_object(),
        "the done frame must carry a verification; frame was:\n{done}"
    );
    assert!(
        verification["ok"].is_boolean(),
        "verification must state ok/not-ok, got: {verification}"
    );
    assert!(
        verification["diagnostics"]["total"].is_number(),
        "verification must carry the PRE-CAP total, got: {verification}"
    );
    assert_eq!(
        verification["scan_error"],
        serde_json::Value::Null,
        "the scan ran, so it must not be reporting its own failure: {verification}"
    );

    // ── Phase 2: the reported failure — the provider hands back nothing ───────
    std::env::set_var("BIOROUTER_KNOWLEDGE_TEST_MODE", "empty-reply");
    let (dir_bad, app_bad) = build_app();
    create_kb(&app_bad, "bad").await;
    let body = ingest_stream(&app_bad, "bad").await;

    assert!(
        !body.contains("event: done"),
        "a digest that wrote nothing must never emit a done frame; stream was:\n{body}"
    );
    assert!(
        body.contains("event: error"),
        "a failed digest must end in an error frame; stream was:\n{body}"
    );

    // The frame has to be usable: valid JSON carrying a message that says what
    // went wrong, because it is the only thing the UI has to show the user.
    let payload = body
        .split("event: error\ndata: ")
        .nth(1)
        .expect("error frame payload")
        .split("\n\n")
        .next()
        .expect("error frame terminator");
    let parsed: serde_json::Value =
        serde_json::from_str(payload).expect("the error frame must be valid JSON");
    let message = parsed["message"].as_str().expect("message field");
    assert!(
        message.contains("no knowledge pages"),
        "the error must tell the user the digest wrote nothing, got: {message}"
    );

    // And the claim must be true: nothing landed under knowledge/.
    let sources = dir_bad.path().join("bad/knowledge/sources");
    assert!(
        std::fs::read_dir(&sources)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true),
        "the failed digest must not have written a source page"
    );

    std::env::remove_var("BIOROUTER_KNOWLEDGE_TEST_MODE");
}
