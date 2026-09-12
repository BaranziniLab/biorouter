// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

/// End-to-end integration test that drives the /knowledge router through a
/// realistic, multi-step flow without requiring a live LLM provider:
///
///   create → raw-source → history → graph → export → import
///
/// The router is constructed from `Arc<KnowledgeService>` directly so we need
/// no full AppState, network, or real provider.
use axum::{body::Body, http::Request};
use biorouter_mcp::knowledge::{page_fixtures::valid_page, service::KnowledgeService};
use std::sync::Arc;
use tower::ServiceExt;

/// Issue #56 / the adversarial review of 2026-09-12: `POST /bases` is gated like
/// every other base-naming route, because refusing a taken id told a secret-only
/// caller which PRIVATE ids were taken. So every test here that mints a base
/// speaks as the person at the keyboard — which is who mints one in the product.
const TEST_USER_ACTION_KEY: &str = "knowledge-e2e-user-action-key";

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

async fn json_body(res: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn e2e_create_raw_history_graph_export_import() {
    let (_dir, app) = build_app();

    // ── 1. Create knowledge base ──────────────────────────────────────────────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({"id": "e2e", "name": "E2E"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 1: create base should return 200");
    let manifest = json_body(res).await;
    assert_eq!(manifest["id"].as_str().unwrap(), "e2e");
    assert_eq!(manifest["name"].as_str().unwrap(), "E2E");

    // ── 2. Verify the base appears in list ────────────────────────────────────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let bases = json_body(res).await;
    assert_eq!(
        bases.as_array().unwrap().len(),
        1,
        "step 2: should have 1 base"
    );

    // ── 3. Add a text source via POST /bases/e2e/raw ─────────────────────────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/e2e/raw")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "text": "Heart rate variability improves after zone-2 training.",
                        "title": "HRV note"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        200,
        "step 3: add raw source should return 200"
    );
    let raw_resp = json_body(res).await;
    assert!(
        raw_resp["source_id"].is_string(),
        "step 3: response must contain source_id"
    );
    assert!(
        raw_resp["source_md_path"].is_string(),
        "step 3: response must contain source_md_path"
    );

    // ── 4. History should have ≥2 entries (initial commit + raw source) ───────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/e2e/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 4: history should return 200");
    let history = json_body(res).await;
    assert!(
        history.as_array().unwrap().len() >= 2,
        "step 4: history should have ≥2 entries after raw source"
    );

    // ── 5. Graph should return 200 (may be empty for a plain text source) ─────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/e2e/graph")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 5: graph should return 200");
    let graph = json_body(res).await;
    assert!(
        graph.get("nodes").is_some(),
        "step 5: graph must have nodes"
    );
    assert!(
        graph.get("edges").is_some(),
        "step 5: graph must have edges"
    );

    // ── 6. Write a manual page so we have more content to round-trip ──────────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/bases/e2e/pages/knowledge/notes/hrv.md")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "content": valid_page("note", "HRV", "# HRV\n\nZone-2 training increases HRV."),
                        "commit_message": "add hrv note"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 6: write page should return 200");
    let write_resp = json_body(res).await;
    assert!(
        write_resp.get("commit_sha").is_some(),
        "step 6: write response must have commit_sha"
    );

    // ── 7. Read the page back ─────────────────────────────────────────────────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/e2e/pages/knowledge/notes/hrv.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 7: read page should return 200");
    let page = json_body(res).await;
    assert!(
        page["content"].as_str().unwrap_or("").contains("Zone-2"),
        "step 7: page content should contain written text"
    );

    // ── 8. Export the KB → binary body ≥100 bytes ─────────────────────────────
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/e2e/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 8: export should return 200");
    assert_eq!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/octet-stream"),
        "step 8: export content-type must be application/octet-stream"
    );
    let brkb_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        brkb_bytes.len() >= 100,
        "step 8: exported archive should be ≥100 bytes, got {}",
        brkb_bytes.len()
    );

    // ── 9. Import the archive → new base id ──────────────────────────────────
    let boundary = "----biorouter-e2e-boundary";
    let mut multipart: Vec<u8> = Vec::new();
    multipart.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    multipart.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"e2e.brkb\"\r\n",
    );
    multipart.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    multipart.extend_from_slice(&brkb_bytes);
    multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/import")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(multipart))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "step 9: import should return 200");
    let import_resp = json_body(res).await;
    let new_id = import_resp["id"]
        .as_str()
        .expect("step 9: response must have id");
    // "e2e" already exists, so import should produce "e2e-2"
    assert_eq!(
        new_id, "e2e-2",
        "step 9: import into existing root should suffix -2"
    );

    // ── 10. list_bases should now show 2 bases ────────────────────────────────
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let bases = json_body(res).await;
    assert_eq!(
        bases.as_array().unwrap().len(),
        2,
        "step 10: should have 2 bases (original + imported)"
    );
}
