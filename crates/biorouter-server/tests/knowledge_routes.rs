// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

/// Integration tests for the /knowledge HTTP routes.
///
/// The router under test takes `Arc<KnowledgeService>` directly, so we don't
/// need to construct a full `AppState`.  Each test gets a fresh tempdir-backed
/// `KnowledgeService` via `build_test_router()`.
use axum::{body::Body, http::Request, Router};
use biorouter_mcp::knowledge::{page_fixtures::valid_page, service::KnowledgeService};
use std::sync::Arc;
use tower::ServiceExt;

fn build_test_router() -> (tempfile::TempDir, Router) {
    tier_route::install_test_user_action_key();
    let dir = tempfile::tempdir().unwrap();
    let svc = Arc::new(KnowledgeService::new(dir.path().to_path_buf()));
    let router = biorouter_server::routes::knowledge::router(svc);
    (dir, router)
}

/// `POST /active` as the Knowledge view sends it — carrying the user's proof.
///
/// ⚠ Since QA's 2026-09-10 H2 sweep the selection is filtered for a caller
/// WITHOUT that proof (private and absent bases dropped, a write unable to move
/// what it cannot see), so these mechanics tests speak as the user, which is who
/// the renderer is. What an unproven caller sees and may change is
/// `h2_http_barrier`'s subject.
async fn post_active(app: &Router, body: serde_json::Value) -> (u16, serde_json::Value) {
    tier_route::install_test_user_action_key();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/active")
                .header("content-type", "application/json")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

/// `GET /active`, with the user's proof — see [`post_active`].
async fn get_active(app: &Router, session_id: Option<&str>) -> serde_json::Value {
    tier_route::install_test_user_action_key();
    let uri = match session_id {
        Some(sid) => format!("/active?session_id={sid}"),
        None => "/active".to_string(),
    };
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Build a router and also return the underlying KB root directory so tests
/// can seed files directly on disk (needed for routes that read from `raw/`
/// where there is no write API).
fn build_test_router_with_root() -> (tempfile::TempDir, std::path::PathBuf, Router) {
    tier_route::install_test_user_action_key();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let svc = Arc::new(KnowledgeService::new(root.clone()));
    let router = biorouter_server::routes::knowledge::router(svc);
    (dir, root, router)
}

#[tokio::test]
async fn get_location_returns_kb_path() {
    let (_d, root, app) = build_test_router_with_root();
    // Create a KB so the directory exists.
    let create_body = serde_json::to_vec(&serde_json::json!({"id": "loc", "name": "Loc"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/loc/location")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let path = v.get("path").and_then(|p| p.as_str()).unwrap();
    assert!(
        path.contains("loc"),
        "path should point at the KB dir: {path}"
    );
    assert!(
        path.starts_with(&root.to_string_lossy().into_owned()),
        "path should be under the test root"
    );
}

#[tokio::test]
async fn get_location_404_for_unknown_kb() {
    // The person at the keyboard is told the base is not there. A caller
    // without the proof is told what it is told for a private base (403) —
    // QA 2026-09-10 H2 — so the 404 is not an oracle for which ids exist.
    tier_route::install_test_user_action_key();
    let (_d, app) = build_test_router();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/nope/location")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/nope/location")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 5: read-only routes
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_bases_empty_returns_empty_array() {
    let (_d, app) = build_test_router();
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
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn create_then_get_base() {
    let (_d, app) = build_test_router();
    let create_body = serde_json::to_vec(&serde_json::json!({"id": "t", "name": "Test"})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "POST /bases should return 200");

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/t")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET /bases/t should return 200");
}

#[tokio::test]
async fn set_default_model_persists_on_manifest() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "models", "Models").await;

    let body = serde_json::to_vec(&serde_json::json!({
        "model": {
            "provider": "versa_azure",
            "model": "gpt-5.5-2026-04-24"
        }
    }))
    .unwrap();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/bases/models/default-model")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["default_model"]["provider"], "versa_azure");
    assert_eq!(manifest["default_model"]["model"], "gpt-5.5-2026-04-24");

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["default_model"]["provider"], "versa_azure");
    assert_eq!(manifest["default_model"]["model"], "gpt-5.5-2026-04-24");
}

#[tokio::test]
async fn update_base_metadata_roundtrip() {
    let (_d, app) = build_test_router();
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "rename", "name": "Original"})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "POST /bases should return 200");

    let update_body = serde_json::to_vec(&serde_json::json!({
        "name": "Renamed Knowledge Base",
        "color": "#123456"
    }))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/bases/rename")
                .header("content-type", "application/json")
                .body(Body::from(update_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "PUT /bases/rename should return 200");

    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["id"], "renamed-knowledge-base");
    assert_eq!(manifest["name"], "Renamed Knowledge Base");
    assert_eq!(manifest["color"], "#123456");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/renamed-knowledge-base")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        200,
        "GET /bases/renamed-knowledge-base should return 200"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["id"], "renamed-knowledge-base");
    assert_eq!(manifest["name"], "Renamed Knowledge Base");
    assert_eq!(manifest["color"], "#123456");

    // Asked as the user: an unproven caller is told nothing about an id that
    // names no base (QA 2026-09-10 H2), so only the user can see the 404.
    tier_route::install_test_user_action_key();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/rename")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        404,
        "GET /bases/rename should return 404 after rename"
    );
}

#[tokio::test]
async fn get_graph_returns_ok_on_new_kb() {
    let (_d, app) = build_test_router();
    // Create a KB first.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "g", "name": "Graph Test"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/g/graph")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET /bases/g/graph should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // A fresh KB graph has empty nodes/edges.
    assert!(v.get("nodes").is_some(), "graph should have nodes key");
    assert!(v.get("edges").is_some(), "graph should have edges key");
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 6: page CRUD routes
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_pages_empty_on_new_kb() {
    let (_d, app) = build_test_router();
    // Create KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "lp", "name": "List Pages"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/lp/pages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        v.as_array().unwrap().len(),
        0,
        "new KB should have no pages"
    );
}

#[tokio::test]
async fn a_page_written_over_http_reaches_the_graph() {
    // `get_graph` serves `graph-cache.json` whenever it can read one, and a base
    // is created carrying an EMPTY cache. So this route's write is only visible
    // in the graph if the route refreshes the cache — and it did not: measured
    // against a running daemon, two pages written over this route with a
    // `[[wiki]]` link between them left `GET /graph` answering `{"nodes": [],
    // "edges": []}`, while the same request after deleting the cache file
    // returned both nodes and the edge.
    //
    // That failure is invisible to `write_then_read_page_roundtrip` below,
    // because reading the page back reads the file rather than the derivation.
    // The MCP `kb_write_page` tool carries the same fix and a comment describing
    // the same symptom; this is the other writer.
    let (_d, app) = build_test_router();
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "gc", "name": "Graph Cache"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let write_body = serde_json::to_vec(&serde_json::json!({
        "content": valid_page("note", "Graphed", "# Graphed\n\nA page that must reach the graph."),
        "commit_message": "add a page that must reach the graph"
    }))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/bases/gc/pages/knowledge/notes/graphed.md")
                .header("content-type", "application/json")
                .body(Body::from(write_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "PUT page should return 200");

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/gc/graph")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET graph should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let nodes = v["nodes"]
        .as_array()
        .expect("graph must carry a node array");
    assert!(
        !nodes.is_empty(),
        "a page written over HTTP must appear in the graph; got {v}. An empty \
         graph beside a page that GET /pages lists is the stale-cache symptom: \
         the write did not rebuild graph-cache.json, and nothing in the UI can \
         repair that because \"Refresh graph\" re-reads the same cache"
    );
}

#[tokio::test]
async fn write_then_read_page_roundtrip() {
    let (_d, app) = build_test_router();
    // Create KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "wr", "name": "Write Read"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Write a page.
    let write_body = serde_json::to_vec(&serde_json::json!({
        "content": valid_page("note", "Hello", "# Hello\n\nWorld"),
        "commit_message": "add hello page"
    }))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/bases/wr/pages/knowledge/notes/hello.md")
                .header("content-type", "application/json")
                .body(Body::from(write_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "PUT page should return 200");

    // Read the page back.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/wr/pages/knowledge/notes/hello.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET page should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v["content"].as_str().unwrap().contains("Hello"),
        "page content should contain written text"
    );
}

#[tokio::test]
async fn read_page_on_missing_path_returns_404() {
    let (_d, app) = build_test_router();
    // Create KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "miss", "name": "Missing"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/miss/pages/knowledge/notes/nonexistent.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        404,
        "reading a missing page should return 404"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 7: history + preview + restore
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn history_write_restore_roundtrip() {
    let (_d, app) = build_test_router();

    // 1. Create base.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "hist", "name": "History"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. list_history after create → 1 entry (the initial commit).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/hist/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let history: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let history_arr = history.as_array().unwrap();
    assert!(
        !history_arr.is_empty(),
        "should have at least the initial commit"
    );
    // Capture the first (oldest) commit SHA.
    let first_sha = history_arr
        .last()
        .unwrap()
        .get("commit_sha")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // 3. Write a page.
    let write_body = serde_json::to_vec(&serde_json::json!({
        "content": valid_page("note", "Temp Page", "# Temp Page"),
        "commit_message": "add temp page"
    }))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/bases/hist/pages/knowledge/notes/temp.md")
                .header("content-type", "application/json")
                .body(Body::from(write_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "write page should succeed");

    // 4. list_history should now have 2+ entries.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/hist/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let history2: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        history2.as_array().unwrap().len() >= 2,
        "should have at least 2 commits after write"
    );

    // 5. restore_state to the first commit (before the page write).
    let restore_body = serde_json::to_vec(&serde_json::json!({"commit_sha": first_sha})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/hist/restore")
                .header("content-type", "application/json")
                .body(Body::from(restore_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "restore should succeed");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let restore_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        restore_resp.get("new_commit_sha").is_some(),
        "restore should return new_commit_sha"
    );

    // 6. list_history again → 3+ entries (initial + write + restore).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/hist/history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let history3: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        history3.as_array().unwrap().len() >= 3,
        "should have 3+ entries after restore"
    );

    // 7. The page should be gone after restore to initial commit.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/hist/pages/knowledge/notes/temp.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        404,
        "page should not exist after restoring to initial commit"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 8: POST /bases/:id/raw
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn add_raw_source_text() {
    let (_d, app) = build_test_router();

    // Create a KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "raw", "name": "Raw Test"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // POST a text source.
    let body = serde_json::to_vec(&serde_json::json!({
        "text": "hello world lab note",
        "title": "Note"
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/raw/raw")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "POST /bases/raw/raw should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v.get("source_id").is_some(),
        "response should have source_id"
    );
    assert!(
        v.get("source_md_path").is_some(),
        "response should have source_md_path"
    );
}

#[tokio::test]
async fn add_raw_source_html_multipart_uses_part_mime() {
    let (_d, app) = build_test_router();

    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "rawhtml", "name": "Raw Html"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let res = app
        .clone()
        .oneshot(multipart_file_request(
            "/bases/rawhtml/raw",
            "upload",
            Some("text/html; charset=utf-8"),
            b"<html><body><h1>Hello</h1><p>World</p></body></html>".to_vec(),
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let path = v["source_md_path"].as_str().unwrap();

    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/bases/rawhtml/page?path={path}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(page["content"].as_str().unwrap().contains("# Hello"));
}

#[tokio::test]
async fn add_raw_source_accepts_supported_file_formats() {
    let (_d, app) = build_test_router();

    struct Case<'a> {
        slug: &'a str,
        filename: &'a str,
        mime: Option<&'a str>,
        bytes: Vec<u8>,
        expected: Option<&'a str>,
    }

    let cases = vec![
        Case {
            slug: "html",
            filename: "article.html",
            mime: Some("text/html"),
            bytes: include_bytes!(
                "../../biorouter-mcp/src/knowledge/convert/fixtures/article.html"
            )
            .to_vec(),
            expected: Some("# Example article"),
        },
        Case {
            slug: "markdown",
            filename: "note.md",
            mime: Some("text/markdown"),
            bytes: b"# Markdown source\n\nDigest this note.".to_vec(),
            expected: Some("# Markdown source"),
        },
        Case {
            slug: "text",
            filename: "note.txt",
            mime: Some("text/plain"),
            bytes: b"Plain text note for digestion.".to_vec(),
            expected: Some("Plain text note for digestion."),
        },
        Case {
            slug: "csv",
            filename: "table.csv",
            mime: Some("text/csv"),
            bytes: b"name,score\nAlice,9\nBob,7\n".to_vec(),
            expected: Some("| name | score |"),
        },
        Case {
            slug: "docx",
            filename: "sample.docx",
            mime: Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            bytes: include_bytes!(
                "../../biorouter-mcp/src/computercontroller/tests/data/sample.docx"
            )
            .to_vec(),
            expected: None,
        },
        Case {
            slug: "pdf",
            filename: "sample.pdf",
            mime: Some("application/pdf"),
            bytes: include_bytes!("../../biorouter-mcp/src/computercontroller/tests/data/test.pdf")
                .to_vec(),
            expected: None,
        },
    ];

    for (index, case) in cases.into_iter().enumerate() {
        let kb_id = format!("fmt{index}");
        create_kb(app.clone(), &kb_id, case.slug).await;

        let res = app
            .clone()
            .oneshot(multipart_file_request(
                &format!("/bases/{kb_id}/raw"),
                case.filename,
                case.mime,
                case.bytes,
                &[],
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "{} upload should return 200", case.slug);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let path = v["source_md_path"].as_str().unwrap();

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/bases/{kb_id}/page?path={path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "{} page should be readable", case.slug);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let content = page["content"].as_str().unwrap();
        if let Some(expected) = case.expected {
            assert!(
                content.contains(expected),
                "{} converted content should contain {expected:?}, got {content:?}",
                case.slug
            );
        } else {
            assert!(
                !content.trim().is_empty(),
                "{} converted content should not be empty",
                case.slug
            );
        }
    }
}

#[tokio::test]
async fn add_raw_source_rejects_empty_body() {
    let (_d, app) = build_test_router();

    // Create a KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "rj", "name": "Reject"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // POST an empty JSON object — no url, no text, no file.
    let body = serde_json::to_vec(&serde_json::json!({})).unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/rj/raw")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 400, "empty body should return 400");
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 10: GET /bases/:id/export + POST /bases/import
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn export_then_import_roundtrip() {
    let (_d, app) = build_test_router();

    // 1. Create a KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "ex", "name": "Export"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. Add a small text source so the archive has content.
    let source_body = serde_json::to_vec(&serde_json::json!({
        "text": "content for export",
        "title": "Test"
    }))
    .unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/ex/raw")
                .header("content-type", "application/json")
                .body(Body::from(source_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // 3. Export the KB.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/ex/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET /bases/ex/export should return 200");
    assert_eq!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/octet-stream"),
        "export should return octet-stream"
    );
    let brkb_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        brkb_bytes.len() > 100,
        "exported archive should have content"
    );

    // 4. Import via multipart — build a minimal multipart body by hand.
    // Boundary and body structure per RFC 2046.
    let boundary = "TESTBOUNDARY";
    let mut multipart_body: Vec<u8> = Vec::new();
    multipart_body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"ex.brkb\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    multipart_body.extend_from_slice(&brkb_bytes);
    multipart_body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

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
                .body(Body::from(multipart_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "POST /bases/import should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let new_id = v.get("id").and_then(|v| v.as_str()).unwrap();
    // The original id was "ex" and it already exists, so the import should assign "ex-2".
    assert_eq!(
        new_id, "ex-2",
        "import into existing root should suffix with -2"
    );

    // 5. The imported KB should show up in list_bases (we should now have 2 bases).
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let bases: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let count = bases.as_array().unwrap().len();
    assert_eq!(
        count, 2,
        "should have 2 knowledge bases after import (original + imported)"
    );
}

#[tokio::test]
async fn import_route_allows_archives_above_axums_default_and_reports_bad_zip_as_400() {
    let (_d, app) = build_test_router();
    let bytes = vec![0_u8; 3 * 1024 * 1024];
    let res = app
        .oneshot(multipart_file_request(
            "/bases/import",
            "large-invalid.brkb",
            Some("application/octet-stream"),
            bytes,
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "the route must override Axum's 2 MiB default and let archive validation answer"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("readable zip archive"),
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn import_route_maps_scaffold_and_strict_manifest_errors_to_400() {
    use std::io::Write;

    for (format, include_schema, expected) in [("okf", false, "schema.md"), ("okff", true, "okff")]
    {
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("fixture/manifest.yaml", options).unwrap();
            zip.write_all(
                format!("id: fixture\nname: Fixture\nschema_version: 3\nformat: {format}\n")
                    .as_bytes(),
            )
            .unwrap();
            for required in ["index.md", "log.md"] {
                zip.start_file(format!("fixture/{required}"), options)
                    .unwrap();
                zip.write_all(b"# fixture\n").unwrap();
            }
            if include_schema {
                zip.start_file("fixture/schema.md", options).unwrap();
                zip.write_all(b"# schema\n").unwrap();
            }
            zip.start_file("fixture/knowledge/x.md", options).unwrap();
            zip.write_all(b"---\ntype: Note\nidentifier: x\n---\n")
                .unwrap();
            zip.start_file("fixture/.brkb-provenance", options).unwrap();
            zip.write_all(br#"{"schema":3,"tier":"public","owners":[],"format":"okf"}"#)
                .unwrap();
            zip.finish().unwrap();
        }

        let (_d, app) = build_test_router();
        let res = app
            .oneshot(multipart_file_request(
                "/bases/import",
                "fixture.brkb",
                Some("application/octet-stream"),
                bytes.into_inner(),
                &[],
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 400);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains(expected),
            "{}",
            String::from_utf8_lossy(&body)
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 11: reclassify + override_credibility routes
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn reclassify_route_returns_credibility() {
    let (_d, app) = build_test_router();

    // Create KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "rc", "name": "Reclassify"})).unwrap();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Add a text source.
    let source_body = serde_json::to_vec(&serde_json::json!({
        "text": "personal research note",
        "title": "Note"
    }))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/rc/raw")
                .header("content-type", "application/json")
                .body(Body::from(source_body))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let source: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let source_id = source.get("source_id").and_then(|v| v.as_str()).unwrap();

    // Reclassify.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bases/rc/sources/{source_id}/reclassify"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "reclassify should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v.get("credibility").is_some(),
        "response should have credibility"
    );
    assert!(
        v["credibility"].get("tier").is_some(),
        "credibility should have tier"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 9: SSE macro routes — 400 on invalid model
// ──────────────────────────────────────────────────────────────────────────────

/// Helper: create a KB named `id` in `app`, assert 200.
async fn create_kb(app: Router, id: &str, name: &str) {
    let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": name})).unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "helper create_kb should return 200");
}

fn multipart_file_request(
    uri: &str,
    filename: &str,
    file_content_type: Option<&str>,
    bytes: Vec<u8>,
    fields: &[(&str, &str)],
) -> Request<Body> {
    let boundary = "XBOUNDARY";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    if let Some(content_type) = file_content_type {
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
    }
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(&bytes);
    for (name, value) in fields {
        body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
    }
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

fn ingest_multipart_request(
    uri: &str,
    filename: &str,
    file_content_type: Option<&str>,
    bytes: Vec<u8>,
    provider: &str,
    model: &str,
) -> Request<Body> {
    multipart_file_request(
        uri,
        filename,
        file_content_type,
        bytes,
        &[("provider", provider), ("model", model)],
    )
}

#[tokio::test]
async fn ingest_rejects_invalid_model_with_400() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "ing", "Ingest Test").await;

    let body = serde_json::to_vec(&serde_json::json!({
        "source": {"text": "hello", "title": "x"},
        "model": {"provider": "nonexistent_provider_xyz", "model": "x"}
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/ing/ingest")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "ingest with unknown provider should return 400"
    );
}

#[tokio::test]
async fn expand_path_returns_stageable_children_for_directory() {
    let (_d, app) = build_test_router();
    let temp = tempfile::tempdir().unwrap();
    let bundle_dir = temp.path().join("bundle");
    std::fs::create_dir_all(bundle_dir.join("docs")).unwrap();
    std::fs::write(bundle_dir.join("docs/readme.md"), "# Archive file").unwrap();

    let body = serde_json::to_vec(&serde_json::json!({
        "path": bundle_dir.to_string_lossy()
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/expand-path")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 1);
    assert!(v["files"][0]["relative_path"]
        .as_str()
        .unwrap()
        .contains("bundle/docs/readme.md"));
}

#[tokio::test]
async fn expand_path_cleans_archive_wrapper_and_metadata_noise() {
    let (_d, app) = build_test_router();
    let temp = tempfile::tempdir().unwrap();
    let archive_path = temp.path().join("bundle.zip");
    let file = std::fs::File::create(&archive_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    use std::io::Write;
    zip.start_file("bundle/docs/readme.md", options).unwrap();
    zip.write_all(b"# Archive child").unwrap();
    zip.start_file("__MACOSX/bundle/docs/._readme.md", options)
        .unwrap();
    zip.write_all(b"metadata").unwrap();
    zip.finish().unwrap();

    let body = serde_json::to_vec(&serde_json::json!({
        "path": archive_path.to_string_lossy()
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/expand-path")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 1);
    assert_eq!(v["files"][0]["relative_path"], "bundle/docs/readme.md");
    assert!(v["warnings"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn ingest_rejects_brkb_uploads_in_dropzone_route() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "ing", "Ingest Test").await;

    let res = app
        .oneshot(ingest_multipart_request(
            "/bases/ing/ingest",
            "archive.brkb",
            None,
            b"not-a-real-archive".to_vec(),
            "test-provider",
            "test-model",
        ))
        .await
        .unwrap();

    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn ingest_rejects_oversized_csv_uploads_before_model_check() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "ing", "Ingest Test").await;

    let res = app
        .oneshot(ingest_multipart_request(
            "/bases/ing/ingest",
            "huge.csv",
            None,
            vec![b'a'; 8 * 1024 * 1024 + 1],
            "test-provider",
            "test-model",
        ))
        .await
        .unwrap();

    assert!(
        res.status() == 400 || res.status() == 413,
        "oversized uploads should be rejected before digestion starts, got {}",
        res.status()
    );
}

#[tokio::test]
async fn ingest_accepts_html_upload_before_model_validation() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "ing", "Ingest Test").await;

    let res = app
        .oneshot(ingest_multipart_request(
            "/bases/ing/ingest",
            "upload",
            Some("text/html; charset=utf-8"),
            b"<html><body><h1>Hello</h1></body></html>".to_vec(),
            "nonexistent_provider_xyz",
            "x",
        ))
        .await
        .unwrap();

    assert_eq!(
        res.status(),
        400,
        "multipart HTML should parse successfully and then fail on invalid model"
    );
}

#[tokio::test]
async fn query_rejects_invalid_model_with_400() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "qry", "Query Test").await;

    let body = serde_json::to_vec(&serde_json::json!({
        "question": "What is HRV?",
        "model": {"provider": "nonexistent_provider_xyz", "model": "x"}
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/qry/query")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "query with unknown provider should return 400"
    );
}

#[tokio::test]
async fn lint_rejects_invalid_model_with_400_when_autofix() {
    let (_d, app) = build_test_router();
    create_kb(app.clone(), "lnt", "Lint Test").await;

    // autofix=true requires a real completer → 400 on bad provider.
    let body = serde_json::to_vec(&serde_json::json!({
        "model": {"provider": "nonexistent_provider_xyz", "model": "x"},
        "autofix": true
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases/lnt/lint")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "lint with unknown provider + autofix=true should return 400"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// check_model route
// ──────────────────────────────────────────────────────────────────────────────

// ──────────────────────────────────────────────────────────────────────────────
// Plan 5 Task 1: GET /bases/:id/page?path=... — markdown body for NodePreview
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn read_page_returns_markdown_body() {
    let (_d, root, app) = build_test_router_with_root();

    // Create the KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "rp", "name": "Read Page"})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Seed a knowledge/ page directly on disk.
    let knowledge_dir = root.join("rp").join("knowledge").join("notes");
    std::fs::create_dir_all(&knowledge_dir).unwrap();
    std::fs::write(
        knowledge_dir.join("hello.md"),
        valid_page("note", "Hello", "body text"),
    )
    .unwrap();

    // Happy path: GET /bases/rp/page?path=knowledge/notes/hello.md
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/rp/page?path=knowledge/notes/hello.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET /bases/rp/page should return 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v["content"].as_str().unwrap().contains("body text"),
        "response content should contain page body, got: {v}"
    );

    // Missing page → 404.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/rp/page?path=knowledge/notes/nope.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 404, "missing page should return 404");

    // Path traversal → 400. `../../etc/passwd` is rejected by the
    // `starts_with("knowledge/")` / `starts_with("raw/")` allowlist *before*
    // the `..` check ever fires.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/rp/page?path=../../etc/passwd")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "path traversal should return 400 not 500"
    );

    // Real traversal: passes the prefix allowlist (`starts_with("knowledge/")`)
    // but contains `..` — must be rejected by the dedicated traversal check.
    // This exercises a different code path than the test above.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/rp/page?path=knowledge/../../etc/passwd")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "real path-traversal attempt should return 400 not 500"
    );
}

#[tokio::test]
async fn read_page_returns_raw_source_md() {
    let (_d, root, app) = build_test_router_with_root();

    // Create the KB.
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "rs", "name": "Raw Source"})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Seed a raw/<src-id>/source.md file directly on disk so we don't have to
    // round-trip through the converter.
    let raw_dir = root.join("rs").join("raw").join("test");
    std::fs::create_dir_all(&raw_dir).unwrap();
    std::fs::write(raw_dir.join("source.md"), "# raw\n\nraw source body\n").unwrap();

    // Happy path: GET /bases/rs/page?path=raw/test/source.md
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/rs/page?path=raw/test/source.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        200,
        "GET /bases/rs/page?path=raw/.../source.md should return 200"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v["content"].as_str().unwrap().contains("raw source body"),
        "response content should contain raw source body, got: {v}"
    );
}

#[tokio::test]
async fn read_page_rejects_invalid_kb_id_with_400() {
    // Regression test for the dead-branch bug: the handler previously checked
    // for "invalid kb id" in the error string, but `validate_kb_id` emits
    // "kb-id may only contain a-z, 0-9, and '-'" (etc.). The mismatch meant an
    // invalid kb-id slipped through to 500. With the typed-error refactor this
    // must now route to 400.
    let (_d, app) = build_test_router();

    // "INVALID--KB" violates both the lowercase rule and the `--` rule. We do
    // not need to create the KB; validation fires before any filesystem touch.
    //
    // Asked as the user, who is owed the handler's 400. A caller without the
    // proof never reaches the handler: a malformed id is answered as a private
    // one is (QA 2026-09-10 H2), which also keeps a `..` out of every path
    // join below the gate for that caller.
    tier_route::install_test_user_action_key();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bases/INVALID--KB/page?path=knowledge/x.md")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "invalid kb-id must return 400, not 500 (regression test)"
    );
    let res = app
        .oneshot(
            Request::builder()
                .uri("/bases/INVALID--KB/page?path=knowledge/x.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

#[tokio::test]
async fn check_model_returns_502_for_unknown_provider() {
    let (_d, app) = build_test_router();

    let body = serde_json::to_vec(&serde_json::json!({
        "model": {"provider": "nonexistent_provider_xyz", "model": "x"}
    }))
    .unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/check-model")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    // The provider build fails → 502 Bad Gateway with ok: false
    assert_eq!(
        res.status(),
        502,
        "check_model with unknown provider should return 502"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false, "ok should be false");
    assert!(
        v.get("error").is_some(),
        "response should have an error field"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Plan 6 Task 2: GET + POST /knowledge/active — cross-session active-KB sync
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fresh_selection_reports_soul_as_the_default_primary_when_bootstrapped() {
    let (_d, app) = build_test_router();
    create_bases(&app, &["soul"]).await;

    let machine = get_active(&app, None).await;
    assert_eq!(machine["primary_kb"].as_str(), Some("soul"));
    assert_eq!(machine["active_kb"].as_str(), Some("soul"));
    assert_eq!(machine["kb_ids"], serde_json::json!(["soul"]));

    let fresh_chat = get_active(&app, Some("fresh-chat")).await;
    assert_eq!(fresh_chat["primary_kb"].as_str(), Some("soul"));
    assert_eq!(fresh_chat["active_kb"].as_str(), Some("soul"));
}

#[tokio::test]
async fn active_kb_roundtrip() {
    // The Knowledge view, which sends the user's proof; see `post_active`.
    tier_route::install_test_user_action_key();
    let (_d, app) = build_test_router();

    // Empty initially.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/active")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET /active on fresh root should be 200");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v["active_kb"].is_null(),
        "active_kb should be null on a fresh root"
    );
    assert_eq!(
        v["hidden_kbs"].as_array().map(|items| items.len()),
        Some(0),
        "hidden_kbs should be empty on a fresh root"
    );

    // Create a KB to point at.
    let create_body = serde_json::to_vec(&serde_json::json!({"id": "act", "name": "Act"})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "POST /bases should return 200");

    // Set it.
    let set_body = serde_json::to_vec(&serde_json::json!({
        "kb_id": "act",
        "hidden_kbs": ["hidden-a"]
    }))
    .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/active")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(set_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        200,
        "POST /active with valid id should be 200"
    );

    // Read it back.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/active")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let after: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        after["active_kb"].as_str().unwrap(),
        "act",
        "active_kb should round-trip the set value"
    );
    assert_eq!(
        after["hidden_kbs"],
        serde_json::json!(["hidden-a"]),
        "hidden_kbs should round-trip the set value"
    );

    // Clearing is now an explicit flag. A body that simply does not mention
    // the primary leaves it alone, so a hidden-only edit can never nuke it —
    // the same composability rule the app-grant fix relies on.
    let clear_body = serde_json::to_vec(&serde_json::json!({"clear_primary": true})).unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/active")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(clear_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "POST /active with null should clear");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/active")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let cleared: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        cleared["active_kb"].is_null(),
        "active_kb should be null after clear"
    );
    assert_eq!(
        cleared["hidden_kbs"],
        serde_json::json!(["hidden-a"]),
        "clearing active_kb should not reset hidden_kbs"
    );

    // Invalid kb id returns 400.
    let bad_body = serde_json::to_vec(&serde_json::json!({"kb_id": "INVALID--KB"})).unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/active")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(bad_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        400,
        "POST /active with invalid kb id should return 400"
    );
}

#[tokio::test]
async fn primary_kb_can_be_scoped_per_session() {
    let (_d, app) = build_test_router();

    for (id, name) in [("act", "Act"), ("session-kb", "Session KB")] {
        let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": name})).unwrap();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }

    // Machine-wide: both bases in play, "act" is the primary.
    let global = post_active(&app, serde_json::json!({"primary_kb": "act"})).await;
    assert_eq!(global.0, 200);
    assert_eq!(global.1["primary_kb"].as_str(), Some("act"));
    assert_eq!(
        global.1["kb_ids"],
        serde_json::json!(["act", "session-kb"]),
        "the response carries the session's whole set, not just the pointer"
    );

    // session-a narrows to one base and points at it.
    let scoped = post_active(
        &app,
        serde_json::json!({
            "primary_kb": "session-kb",
            "session_id": "session-a",
            "hidden_kbs": ["act"],
        }),
    )
    .await;
    assert_eq!(scoped.0, 200);
    assert_eq!(scoped.1["primary_kb"].as_str(), Some("session-kb"));
    assert_eq!(scoped.1["kb_ids"], serde_json::json!(["session-kb"]));
    assert_eq!(
        scoped.1["active_kb"].as_str(),
        Some("session-kb"),
        "the deprecated mirror must track the primary for one release"
    );

    // The machine scope is untouched, and a session that never overrode inherits it.
    let machine = get_active(&app, None).await;
    assert_eq!(machine["primary_kb"].as_str(), Some("act"));
    let other = get_active(&app, Some("session-b")).await;
    assert_eq!(other["primary_kb"].as_str(), Some("act"));
    assert_eq!(other["kb_ids"], serde_json::json!(["act", "session-kb"]));
}

async fn create_bases(app: &Router, ids: &[&str]) {
    for id in ids {
        let body = serde_json::to_vec(&serde_json::json!({"id": id, "name": id})).unwrap();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "failed to create base {id}");
    }
}

/// The deprecated `kb_id` alias exists so a renderer bundle built before
/// `primary_kb` keeps working against a fresh daemon. That bundle's *only*
/// spelling for "forget the primary" is `kb_id: null` — it always sends the
/// field, and sends `null` to clear (the pre-branch route read
/// `body.kb_id.as_deref()` straight into the setter, where `None` meant clear).
///
/// Typed `Option<String>`, an explicit `null` and an omitted field both arrive
/// as `None`, so the alias could express every value except the one it was
/// kept for: a stale bundle's clear became a silent no-op, and the generated
/// TypeScript still advertised `kb_id?: string | null` as if it worked.
#[tokio::test]
async fn the_deprecated_alias_distinguishes_an_explicit_null_from_an_omission() {
    let (_d, app) = build_test_router();
    create_bases(&app, &["alpha", "beta", "soul"]).await;

    let set = post_active(&app, serde_json::json!({"kb_id": "alpha"})).await;
    assert_eq!(set.0, 200);
    assert_eq!(set.1["primary_kb"].as_str(), Some("alpha"));

    let cleared = post_active(&app, serde_json::json!({"kb_id": null})).await;
    assert_eq!(cleared.0, 200);
    assert!(
        cleared.1["primary_kb"].is_null(),
        "an explicit null on the deprecated alias must clear, as it always did"
    );
    assert!(cleared.1["active_kb"].is_null());

    // And an omitted alias still means "leave the pointer alone", so a modern
    // set-only edit does not clear the primary as a side effect.
    post_active(&app, serde_json::json!({"kb_id": "alpha"})).await;
    let set_only = post_active(&app, serde_json::json!({"hidden_kbs": ["beta"]})).await;
    assert_eq!(
        set_only.1["primary_kb"].as_str(),
        Some("alpha"),
        "an absent alias is not a clear"
    );
}

/// `clear_primary` writes a durable "this chat has no primary" override — and
/// `delete_base` installs one in every chat that had pinned the deleted base.
/// Until `inherit_primary` existed there was no way back over the wire, so such
/// a chat could never follow the machine-wide default again.
#[tokio::test]
async fn inherit_primary_lets_a_chat_follow_the_machine_default_again() {
    let (_d, app) = build_test_router();
    create_bases(&app, &["alpha", "beta", "soul"]).await;
    post_active(&app, serde_json::json!({"primary_kb": "alpha"})).await;

    let cleared = post_active(
        &app,
        serde_json::json!({"clear_primary": true, "session_id": "s1"}),
    )
    .await;
    assert!(cleared.1["primary_kb"].is_null());
    // The override is durable: a set-only edit does not lift it.
    let set_only = post_active(
        &app,
        serde_json::json!({"hidden_kbs": [], "session_id": "s1"}),
    )
    .await;
    assert!(
        set_only.1["primary_kb"].is_null(),
        "an explicit no-primary must survive an unrelated edit"
    );

    let inherited = post_active(
        &app,
        serde_json::json!({"inherit_primary": true, "session_id": "s1"}),
    )
    .await;
    assert_eq!(inherited.0, 200);
    assert_eq!(inherited.1["primary_kb"].as_str(), Some("alpha"));
    assert_eq!(
        inherited.1["active_kb"].as_str(),
        Some("alpha"),
        "the deprecated mirror tracks it too"
    );

    // Following, not a one-time copy: the chat tracks later machine moves.
    post_active(&app, serde_json::json!({"primary_kb": "beta"})).await;
    assert_eq!(
        get_active(&app, Some("s1")).await["primary_kb"].as_str(),
        Some("beta")
    );

    // At machine scope inherit removes the explicit preference and restores
    // the shipped product default.
    let machine = post_active(&app, serde_json::json!({"inherit_primary": true})).await;
    assert_eq!(machine.0, 200);
    assert_eq!(machine.1["primary_kb"].as_str(), Some("soul"));
}

/// The three primary gestures are mutually exclusive: pin a base, hold none, or
/// follow the machine default. Two of them in one body is a 400 naming both
/// fields, not a silent precedence rule that hands the caller a 200 for an
/// outcome it did not ask for and cannot detect.
#[tokio::test]
async fn conflicting_primary_fields_are_rejected_instead_of_ranked() {
    let (_d, app) = build_test_router();
    create_bases(&app, &["alpha", "beta"]).await;
    post_active(&app, serde_json::json!({"primary_kb": "alpha"})).await;

    for body in [
        serde_json::json!({"primary_kb": "beta", "clear_primary": true}),
        serde_json::json!({"primary_kb": "beta", "inherit_primary": true}),
        serde_json::json!({"clear_primary": true, "inherit_primary": true}),
        serde_json::json!({"kb_id": "beta", "clear_primary": true}),
    ] {
        let rejected = post_active(&app, body.clone()).await;
        assert_eq!(rejected.0, 400, "{body} must be rejected");
    }
    assert_eq!(
        get_active(&app, None).await["primary_kb"].as_str(),
        Some("alpha"),
        "a rejected body persists nothing"
    );

    // Two fields spelling the *same* gesture is not a conflict: that is how a
    // bundle predating `clear_primary` clears.
    let agreeing = post_active(
        &app,
        serde_json::json!({"primary_kb": null, "clear_primary": true}),
    )
    .await;
    assert_eq!(agreeing.0, 200);
    assert!(agreeing.1["primary_kb"].is_null());
}

/// A 400 must mean *nothing happened*. The two halves of this body are applied
/// together — the set first, so the primary can be validated against the state
/// the request produces — and the write used to be ordered before the
/// validation, so "hide beta, and point at a base that does not exist" returned
/// an error while the hide stuck and the stored pointer was left outside the
/// resulting set. A caller that treats 400 as "my request was ignored", which
/// is the only reasonable reading, then held a stale picture of the session.
///
/// The service is where the ordering was fixed (decide-validate-commit); this
/// test pins the property at the surface a client actually sees, across both
/// scopes and both ways a body can be rejected.
#[tokio::test]
async fn a_rejected_post_leaves_the_selection_exactly_as_it_was() {
    let (_d, app) = build_test_router();
    create_bases(&app, &["alpha", "beta"]).await;

    for session in [None, Some("s1")] {
        let mut base = serde_json::json!({"hidden_kbs": [], "primary_kb": "beta"});
        if let Some(sid) = session {
            base["session_id"] = serde_json::json!(sid);
        }
        let before = post_active(&app, base).await;
        assert_eq!(before.0, 200);
        assert_eq!(before.1["primary_kb"].as_str(), Some("beta"));

        // Rejected two ways: a primary outside the resulting set, and a
        // malformed id in the set itself. Both bodies also carry a hide that
        // must not survive the rejection.
        for bad in [
            serde_json::json!({"hidden_kbs": ["beta"], "primary_kb": "ghost"}),
            serde_json::json!({"hidden_kbs": ["beta", "../escape"]}),
        ] {
            let mut bad = bad;
            if let Some(sid) = session {
                bad["session_id"] = serde_json::json!(sid);
            }
            let rejected = post_active(&app, bad.clone()).await;
            assert_eq!(rejected.0, 400, "expected a rejection for {bad}");

            let after = get_active(&app, session).await;
            assert_eq!(
                after["kb_ids"],
                serde_json::json!(["alpha", "beta"]),
                "a rejected request must not narrow the set ({bad})"
            );
            assert_eq!(
                after["hidden_kbs"],
                serde_json::json!([]),
                "a rejected request must not persist its hide ({bad})"
            );
            assert_eq!(
                after["primary_kb"].as_str(),
                Some("beta"),
                "a rejected request must not move the pointer ({bad})"
            );
        }
    }
}

/// The merged model's one invariant, at the wire. A primary that is not in the
/// resulting set is rejected with both halves named; the un-hide and the
/// re-point travel in ONE body so the GUI's "make primary" on an off row is a
/// single request validated against the state it produces.
#[tokio::test]
async fn primary_must_be_a_member_of_the_resulting_set() {
    let (_d, app) = build_test_router();
    for id in ["alpha", "beta"] {
        let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": id})).unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    let bad = post_active(
        &app,
        serde_json::json!({"primary_kb": "beta", "hidden_kbs": ["beta"]}),
    )
    .await;
    assert_eq!(bad.0, 400, "a hidden base cannot be the primary");

    let good = post_active(
        &app,
        serde_json::json!({"primary_kb": "beta", "hidden_kbs": []}),
    )
    .await;
    assert_eq!(good.0, 200);
    assert_eq!(good.1["primary_kb"].as_str(), Some("beta"));
}

/// A set-only edit must never move the pointer, and hiding the primary must
/// promote deterministically rather than leaving a dangling write target.
#[tokio::test]
async fn set_only_edit_keeps_the_primary_until_it_leaves_the_set() {
    let (_d, app) = build_test_router();
    for id in ["alpha", "beta", "gamma"] {
        let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": id})).unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
    }
    post_active(&app, serde_json::json!({"primary_kb": "beta"})).await;

    let narrowed = post_active(&app, serde_json::json!({"hidden_kbs": ["gamma"]})).await;
    assert_eq!(narrowed.1["primary_kb"].as_str(), Some("beta"));

    let orphaned = post_active(&app, serde_json::json!({"hidden_kbs": ["beta"]})).await;
    assert_eq!(
        orphaned.1["primary_kb"].as_str(),
        Some("alpha"),
        "hiding the primary promotes to the first remaining member"
    );
}

/// The same promotion, from the chat the user is actually in. Most chats never
/// pin their own primary — they display the machine-wide one — so this is the
/// common path, and it used to be the broken one: the repair only fired for a
/// chat with its own stored pin, so hiding "this chat's primary" left the
/// pinning chat on beta and the inheriting chat with nothing.
#[tokio::test]
async fn hiding_the_primary_promotes_for_an_inheriting_chat_too() {
    let (_d, app) = build_test_router();
    create_bases(&app, &["alpha", "beta", "gamma"]).await;

    // The machine default every chat starts out displaying.
    post_active(&app, serde_json::json!({"primary_kb": "alpha"})).await;
    // One chat pins it explicitly; the other never says anything.
    post_active(
        &app,
        serde_json::json!({"primary_kb": "alpha", "session_id": "pinned"}),
    )
    .await;
    assert_eq!(
        get_active(&app, Some("inherits")).await["primary_kb"].as_str(),
        Some("alpha"),
        "the two chats show the user the same primary"
    );

    let pinned = post_active(
        &app,
        serde_json::json!({"hidden_kbs": ["alpha"], "session_id": "pinned"}),
    )
    .await;
    let inherits = post_active(
        &app,
        serde_json::json!({"hidden_kbs": ["alpha"], "session_id": "inherits"}),
    )
    .await;
    assert_eq!(
        inherits.1["primary_kb"], pinned.1["primary_kb"],
        "one gesture, one answer, whether the chat pinned its primary or inherited it"
    );
    assert_eq!(inherits.1["primary_kb"].as_str(), Some("beta"));
    assert_eq!(
        get_active(&app, Some("inherits")).await["primary_kb"].as_str(),
        Some("beta"),
        "the promotion is persisted for the chat, not re-derived per response"
    );

    // The machine pointer is untouched, so every other chat still follows it.
    assert_eq!(
        get_active(&app, None).await["primary_kb"].as_str(),
        Some("alpha")
    );
    assert_eq!(
        get_active(&app, Some("bystander")).await["primary_kb"].as_str(),
        Some("alpha")
    );
}

/// Task 10C's scope line, asserted rather than described: the seven
/// `/knowledge/*` read handlers and this export are the USER in the Knowledge
/// view, not a model, and DR-14 governs what a MODEL can reach.
///
/// Driven through the route, because the defect would be a rule applied in the
/// SERVICE and therefore to everyone. The handler is
/// `export_brkb(State(svc), Path(id))` — it takes NO query parameters, calls
/// `svc.export_brkb(&id)` and returns the archive as the response BODY with
/// `Content-Disposition: attachment`. It never writes to disk. What the route
/// can witness is the thing that matters: a location rule implemented one layer
/// down, in `KnowledgeService::export_brkb`, would change this route too — the
/// user would stop being able to download a private base from their own
/// Knowledge view. So assert the bytes come back.
///
/// ⚠ "The user" is the request carrying the user's proof, which the desktop's
/// Knowledge view sends. Until QA's 2026-09-10 H2 sweep this test's export
/// carried nothing and was served — the same request a public chat's shell makes
/// with a recovered daemon secret, which is now refused (`h2_http_barrier`).
#[tokio::test]
async fn the_users_own_export_route_is_not_subject_to_the_models_location_rule() {
    use axum::http::header;

    tier_route::install_test_user_action_key();
    let (_d, root, app) = build_test_router_with_root();
    let create_body =
        serde_json::to_vec(&serde_json::json!({"id": "omop", "name": "Omop"})).unwrap();
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bases")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), 200);

    biorouter_mcp::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
    let page = root.join("omop").join("knowledge").join("x.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "SENTINEL-COHORT-N-412").unwrap();

    let r = app
        .oneshot(
            Request::builder()
                .uri("/bases/omop/export")
                .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "the user's own export of a private base was refused"
    );
    assert_eq!(
        r.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    let body = axum::body::to_bytes(r.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(body.to_vec())).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("knowledge/x.md")),
        "the route returned {} bytes but not the archive",
        body.len()
    );
    // …and nothing was relocated into the knowledge root as a side effect: the
    // model's rule writes `<root>/.exports/`, and the user's route writes nothing.
    assert!(!biorouter_mcp::knowledge::paths::model_export_dir(&root).exists());
}

// ── Issue #56, Task 10B: the caller-provenance matrix for the macro routes ───

mod privacy_ratchet {
    use super::{build_test_router_with_root, create_kb};
    use axum::{body::Body, http::Request, Router};
    use tower::ServiceExt;

    /// Pin every variable these rows depend on, and restore them on drop.
    ///
    /// `env_lock` and not a hand-rolled guard plus `#[serial]`: `#[serial]`
    /// serialises against other `#[serial]` tests only, while the ~39 others in
    /// this binary run concurrently, and the rest of the workspace already
    /// reaches for `env_lock`'s process-wide lock for exactly this. Two
    /// mechanisms in one process do not compose.
    ///
    /// ⚠ `BIOROUTER_KNOWLEDGE_TEST_MODE` must be OFF: `build_completer`'s early
    /// return hands back a `TestModeCompleter` and Public before any provider
    /// exists, which would make both rows Public and the matrix vacuous.
    ///
    /// The PROVIDER NAME is `ollama` in both rows and only `OLLAMA_HOST` moves
    /// — Task 5 makes a loopback Ollama Private and a non-loopback one Public —
    /// so an implementation keyed on `body.model.provider` gives the same answer
    /// twice and fails one row, and so does either hardcoded literal.
    fn lock_env_for(host: &str) -> env_lock::EnvGuard<'static> {
        env_lock::lock_env([
            ("BIOROUTER_KNOWLEDGE_TEST_MODE", None),
            ("BIOROUTER_LEAD_MODEL", None),
            ("BIOROUTER_LEAD_PROVIDER", None),
            ("OLLAMA_HOST", Some(host.to_string())),
            ("OLLAMA_TIMEOUT", Some("1".to_string())),
        ])
    }

    async fn post_json(app: &Router, uri: &str, body: serde_json::Value) {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            200,
            "{uri} must build a provider and start streaming"
        );
        // Drain the SSE body so the macro task gets to run to completion.
        let _ = axum::body::to_bytes(res.into_body(), usize::MAX).await;
    }

    /// The macro raise happens inside a `tokio::spawn`ed task, so give it a
    /// bounded chance to land rather than racing it.
    async fn await_private(root: &std::path::Path, kb: &str) -> bool {
        for _ in 0..200 {
            if biorouter_mcp::knowledge::tier::is_private(root, kb) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        false
    }

    fn model(provider: &str, model: &str) -> serde_json::Value {
        serde_json::json!({ "provider": provider, "model": model })
    }

    /// Builds one macro route's request body around a `ModelRef`.
    type BodyFor = fn(serde_json::Value) -> serde_json::Value;

    #[tokio::test]
    // Kept alongside `lock_env_for`, which is what actually excludes the other
    // tests in this binary: `#[serial]` costs nothing and still orders this
    // against any future `#[serial]` test that touches the environment without
    // taking the workspace lock.
    #[serial_test::serial]
    async fn mutating_macro_routes_ratchet_and_read_only_queries_do_not() {
        // The gate a `grep -c caller_is_private` cannot be: every route reports
        // NON-ZERO whether it passes the right value, a hardcoded `true`, or a
        // hardcoded `false`. Both rows, per route — the PUBLIC row is the one
        // the previous gate could not fail, and under-ratcheting is the
        // direction that launders (a private transcript into a base that stays
        // public).
        //
        // `ingest-conversation` is absent on purpose: it loads sessions from the
        // process-global `SessionManager`, i.e. the developer's real session
        // database. Its capability is the same `build_completer` value the three
        // routes below pin, and it is covered structurally.
        let routes: Vec<(&str, BodyFor, bool)> = vec![
            (
                "ingest",
                |m| serde_json::json!({ "source": {"text": "n=412", "title": "t"}, "model": m }),
                true,
            ),
            (
                "query",
                |m| serde_json::json!({ "question": "what is n?", "model": m }),
                false,
            ),
            (
                "lint",
                |m| serde_json::json!({ "autofix": true, "model": m }),
                true,
            ),
        ];

        // ⚠ The private row's port is 1, not 11434: `is_loopback_host` reads the
        // HOST, so any port is Private, and nothing can listen on 1 without
        // root. Pointing it at the real Ollama port would make this row drive a
        // live local model — several real sub-agent turns — on any developer
        // machine that happens to be running one, which is how an earlier
        // variant of this matrix came to sit for thirteen minutes.
        for (route, body, writes_base) in routes {
            for (host, caller_is_private) in [
                ("http://127.0.0.1:1", true),
                ("http://ollama.invalid:11434", false),
            ] {
                let _env = lock_env_for(host);
                let (_d, root, app) = build_test_router_with_root();
                create_kb(app.clone(), "kb", "KB").await;
                assert!(!biorouter_mcp::knowledge::tier::is_private(&root, "kb"));

                post_json(
                    &app,
                    &format!("/bases/kb/{route}"),
                    body(model("ollama", "qwen3.5:4b")),
                )
                .await;

                if caller_is_private && writes_base {
                    assert!(
                        await_private(&root, "kb").await,
                        "{route} with a private model did not ratchet the base"
                    );
                } else {
                    assert!(
                        !biorouter_mcp::knowledge::tier::is_private(&root, "kb"),
                        "{route} unexpectedly privatised the base"
                    );
                }
            }
        }
    }

    // ⚠ The other half of provenance — that `providers::create` intercepts
    // `BIOROUTER_LEAD_MODEL` BEFORE the registry lookup, so the INSTANCE it
    // returns need not be the requested name's provider — is asserted in
    // `biorouter-cli`'s
    // `the_cli_capability_follows_the_instance_not_the_name_the_user_typed`,
    // which reads `build_completer`'s returned tier directly and runs no macro.
    //
    // It cannot be asserted here. These routes only expose the tier by running
    // a macro, and a lead/worker composite whose lead is a public provider makes
    // that macro issue a real request on a client with a 600-second timeout —
    // the run this replaces sat there for thirteen minutes. What this file DOES
    // pin is the property that matters at the route: the matrix above sends the
    // SAME provider name (`ollama`) in both rows and varies only `OLLAMA_HOST`,
    // so a route keyed on `body.model.provider` gives one answer twice and fails
    // a row. That the tier and the completer come from one `Arc` is
    // `the_completer_and_the_capability_come_from_the_same_provider`, and every
    // production caller of `ProviderCompleter::new` is gone.

    // ── Issue #56, Task 10C: the barrier at CP2, over HTTP ───────────────────

    /// A macro run as the Knowledge view starts one: with the user's proof.
    ///
    /// ⚠ Since QA's 2026-09-10 H2 sweep a private base answers a caller WITHOUT
    /// that proof before the macro route runs at all (`gate_knowledge_base`),
    /// so the tests below — which are about CP2, the MODEL's capability — speak
    /// as the user in order to reach it. The two gates ask different questions:
    /// may this caller address the base, and may this model read it.
    async fn post_json_raw(app: &Router, uri: &str, body: serde_json::Value) -> (u16, String) {
        super::tier_route::install_test_user_action_key();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("X-User-Action", super::tier_route::TEST_USER_ACTION_KEY)
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_public_model_macro_cannot_run_against_a_private_base_over_http() {
        // ⚠ DEVIATION from the task text, recorded rather than hidden. The task
        // writes this as `post_query(...)` against a bare root; there is no such
        // helper, and the caller's capability is not a parameter of these routes
        // — it is read off the provider `build_completer` constructs. So the
        // PUBLIC caller is spelled the way the ratchet matrix above spells it: a
        // non-loopback `OLLAMA_HOST`.
        let _env = lock_env_for("http://ollama.invalid:11434");
        let (_d, root, app) = build_test_router_with_root();
        create_kb(app.clone(), "omop", "OMOP").await;
        std::fs::write(
            root.join("omop").join("knowledge").join("x.md"),
            "# x\n\nSENTINEL-BODY\n",
        )
        .unwrap();
        biorouter_mcp::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();

        let (status, body) = post_json_raw(
            &app,
            "/bases/omop/query",
            serde_json::json!({
                "question": "what is n?",
                "model": model("ollama", "qwen3.5:4b"),
            }),
        )
        .await;
        assert_eq!(
            status, 409,
            "a public model ran a macro on a private base: {body}"
        );
        assert!(body.contains("private"), "{body}");

        // And the Knowledge view still reads the page: the user is not a model.
        // ⚠ "The user" is now the request carrying the user's proof, which is
        // what the desktop sends. Until QA's 2026-09-10 H2 sweep this read
        // carried nothing at all and was served anyway — which is the same
        // request a public chat's shell makes with a recovered daemon secret.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bases/omop/page?path=knowledge/x.md")
                    .header("X-User-Action", super::tier_route::TEST_USER_ACTION_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            200,
            "the Knowledge view was locked out of the user's own notes"
        );
    }

    // ── The read-only lint, BOTH directions ─────────────────────────────────
    //
    // `POST /bases/{id}/lint` with `autofix: false` is the DEFAULT lint and the
    // read-only one, and it used to report a hardcoded `ProviderTier::Public`
    // whatever model the caller named. A Public capability can never reach a
    // private base, so the refusal was unconditional — and the refusal text
    // asks the user to switch this chat to a private model, which was the one
    // remedy that could not possibly work while the model was not being read.
    //
    // The two rows below differ in ONE thing, `OLLAMA_HOST`, exactly as the
    // ratchet matrix above does: same route, same base, same provider name,
    // same body. So the pair is also the proof that the refusal's remedy is
    // now followable — switching to a private model is precisely the difference
    // between the 409 row and the 200 row.

    /// Seed a private base with one orphan page, so a lint has a real finding.
    async fn private_base_with_an_orphan(app: &Router, root: &std::path::Path) {
        create_kb(app.clone(), "omop", "OMOP").await;
        let notes = root.join("omop").join("knowledge").join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(
            notes.join("lonely.md"),
            "---\ntitle: Lonely\n---\nSENTINEL-BODY, linked by nothing\n",
        )
        .unwrap();
        biorouter_mcp::knowledge::tier::raise_unlocked(root, "omop", true).unwrap();
    }

    fn read_only_lint(m: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "autofix": false, "model": m })
    }

    /// ⚠ **The row the literal could not pass.** Its sibling below is satisfied
    /// by "refuse everyone", which is what the route did; only this one
    /// distinguishes a barrier from an outage. Same shape, and the same reason,
    /// as `routes::knowledge::tests::
    /// a_private_model_may_ingest_its_own_private_conversation_over_http`.
    ///
    /// Driven end to end rather than against `assert_macro_target_reachable`,
    /// because one capability feeds TWO gates: the pre-check that chooses the
    /// status code, and CP2 inside `lint_macro::lint`, which chooses whether the
    /// stream carries a report or an error. A test that stopped at the 200 would
    /// pass on a fix that merely moved the refusal into the stream.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_private_model_may_run_a_read_only_lint_on_its_own_private_base() {
        // Private the way the matrix above spells it: a LOOPBACK `OLLAMA_HOST`,
        // so the capability follows the constructed instance and not the
        // provider name — port 1, which nothing can listen on without root.
        let _env = lock_env_for("http://127.0.0.1:1");
        let (_d, root, app) = build_test_router_with_root();
        private_base_with_an_orphan(&app, &root).await;

        let (status, body) = post_json_raw(
            &app,
            "/bases/omop/lint",
            read_only_lint(model("ollama", "qwen3.5:4b")),
        )
        .await;
        assert_eq!(
            status, 200,
            "a private model was refused a read-only lint of its own private base: {body}"
        );

        // The stream carried a REPORT, not an error frame: CP2 admitted it too.
        let payload = body
            .split("event: done\ndata: ")
            .nth(1)
            .unwrap_or_else(|| {
                panic!("no terminal done frame — the barrier moved into the stream:\n{body}")
            })
            .split("\n\n")
            .next()
            .unwrap();
        let result: serde_json::Value =
            serde_json::from_str(payload).expect("the done frame must be valid JSON");
        assert_eq!(
            result["report"]["orphans"],
            serde_json::json!(["knowledge/notes/lonely.md"]),
            "the lint ran but found nothing, so it did not really read the base: {result}"
        );
        assert!(
            result["commit_sha"].is_null() && result["fixes_applied"] == 0,
            "a read-only lint must still write nothing: {result}"
        );
    }

    /// The barrier itself, unchanged: a public model may not read a private base
    /// even to lint it, because a lint's scan reads every page.
    ///
    /// And the refusal is worth asserting, not just the status: it must name the
    /// real reason and a remedy the caller can act on — the same remedy the row
    /// above then follows to a 200. The text is `assert_reachable`'s own, never
    /// a second spelling of it, so these substrings are checked here and owned
    /// there.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_public_model_still_cannot_run_a_read_only_lint_on_a_private_base() {
        let _env = lock_env_for("http://ollama.invalid:11434");
        let (_d, root, app) = build_test_router_with_root();
        private_base_with_an_orphan(&app, &root).await;

        let (status, body) = post_json_raw(
            &app,
            "/bases/omop/lint",
            read_only_lint(model("ollama", "qwen3.5:4b")),
        )
        .await;
        assert_eq!(
            status, 409,
            "a public model read a private base through a read-only lint: {body}"
        );
        assert!(
            body.contains("only a private model may read or write it"),
            "the refusal did not name the real reason: {body}"
        );
        assert!(
            body.contains("switch this chat to a private model"),
            "the refusal did not name a remedy the caller can act on: {body}"
        );
        // A refusal that leaked the base's contents would defeat itself.
        assert!(
            !body.contains("SENTINEL-BODY") && !body.contains("lonely.md"),
            "the refusal named what it refused: {body}"
        );
    }
}

/// Issue #56 DR-18 — `POST /knowledge/bases/{id}/tier`, the user's own
/// publicize / privatize control.
///
/// ⚠ The keyless-daemon half of this lives in its OWN test binary,
/// `tests/knowledge_tier_no_user_key.rs`. The installed digest is a process
/// global (`OnceLock`), so "a daemon that was handed a key" and "a daemon that
/// was not" are not both reachable inside one binary — the second
/// `install_user_action_digest` is a no-op by construction. Same reason
/// `knowledge_ingest_stream` is its own binary.
mod tier_route {
    use super::*;
    use biorouter_mcp::knowledge::tier;

    /// The server secret this binary's "daemon" was launched with.
    pub(super) const TEST_SECRET: &str = "task-29a-kb-tier-route-secret";
    /// The raw user-action key the launcher would have minted.
    pub(super) const TEST_USER_ACTION_KEY: &str = "task-29a-kb-tier-user-action-key";

    pub(super) fn install_test_user_action_key() {
        let digest: [u8; 32] =
            <sha2::Sha256 as sha2::Digest>::digest(TEST_USER_ACTION_KEY.as_bytes()).into();
        biorouter_server::auth::install_user_action_digest(Some(digest));
    }

    /// The knowledge router behind the SAME `check_token` layer
    /// `commands::agent::run` installs in front of the real one. Layering it is
    /// what makes the 401 arm mean anything: `router()` alone is unauthenticated,
    /// so a test against it would assert that a route nobody guards lets everyone
    /// through.
    pub(super) fn guarded_router() -> (tempfile::TempDir, std::path::PathBuf, Router) {
        install_test_user_action_key();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let svc = Arc::new(biorouter_mcp::knowledge::service::KnowledgeService::new(
            root.clone(),
        ));
        let router = biorouter_server::routes::knowledge::router(svc).layer(
            axum::middleware::from_fn_with_state(
                TEST_SECRET.to_string(),
                biorouter_server::auth::check_token,
            ),
        );
        (dir, root, router)
    }

    async fn post_tier(
        app: &Router,
        kb_id: &str,
        tier: &str,
        secret: Option<&str>,
        user_action: Option<&str>,
    ) -> (u16, String) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/bases/{kb_id}/tier"))
            .header("content-type", "application/json");
        if let Some(key) = secret {
            builder = builder.header("X-Secret-Key", key);
        }
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let req = builder
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "tier": tier })).unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Create `omop` through the real route and ratchet it private, the state
    /// every publicize test starts from.
    async fn seed(root: &std::path::Path, app: &Router) {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .header("X-Secret-Key", TEST_SECRET)
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({ "id": "omop", "name": "OMOP" }))
                            .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        tier::raise_unlocked(root, "omop", /* caller_is_private */ true).unwrap();
    }

    /// Identical to Task 29's `the_route_needs_more_than_the_secret_key`, and it
    /// must stay identical: §9.3 A1 puts the secret inside any developer-enabled
    /// agent shell, so `X-Secret-Key` alone is not a human.
    #[tokio::test]
    async fn the_tier_route_needs_more_than_the_secret_key() {
        install_test_user_action_key();
        let (_d, root, app) = guarded_router();
        seed(&root, &app).await;

        let (status, _) = post_tier(&app, "omop", "public", None, None).await;
        assert_eq!(status, 401);

        let (status, body) = post_tier(&app, "omop", "public", Some(TEST_SECRET), None).await;
        assert_eq!(
            status, 403,
            "a secret-key-only caller publicized a knowledge base"
        );
        assert!(
            body.contains("Do not retry"),
            "the refusal must foreclose the retry: {body}"
        );
        assert!(
            tier::is_private(&root, "omop"),
            "the refused call moved the tier anyway"
        );

        let (status, body) = post_tier(
            &app,
            "omop",
            "public",
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 200, "got {body}");
        assert!(!tier::is_private(&root, "omop"));
    }

    /// Both directions, not just the disclosing one. Privatizing needs no
    /// confirmation *dialog*, but it is still not a thing a model may do —
    /// admitting one direction without the proof is how the tool channel gets the
    /// pointer back.
    #[tokio::test]
    async fn privatizing_needs_the_proof_too() {
        install_test_user_action_key();
        let (_d, root, app) = guarded_router();
        seed(&root, &app).await;
        // Start from public, so "private" is a real change rather than a no-op.
        let (status, _) = post_tier(
            &app,
            "omop",
            "public",
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 200);

        let (status, _) = post_tier(&app, "omop", "private", Some(TEST_SECRET), None).await;
        assert_eq!(status, 403);
        assert!(
            !tier::is_private(&root, "omop"),
            "an unproven caller privatized a base"
        );

        let (status, body) = post_tier(
            &app,
            "omop",
            "private",
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 200, "got {body}");
        assert!(tier::is_private(&root, "omop"));
    }

    /// The renderer cannot render a chip for a tier it was never told. The
    /// listing carries it; `manifest.yaml` does not, and must not — the tier
    /// lives in `.kb-tiers` and a second copy would be a second answer.
    #[tokio::test]
    async fn the_bases_listing_carries_the_tier_and_the_manifest_does_not() {
        install_test_user_action_key();
        let (_d, root, app) = guarded_router();
        seed(&root, &app).await;

        // The Knowledge view's listing — with the user's proof, which is what
        // lists a private base at all since QA's 2026-09-10 H2 sweep.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bases")
                    .header("X-Secret-Key", TEST_SECRET)
                    .header("X-User-Action", TEST_USER_ACTION_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let listing: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(listing[0]["id"], "omop");
        assert_eq!(listing[0]["tier"], "private");
        // …and the manifest on disk is untouched by any of it.
        let manifest = std::fs::read_to_string(root.join("omop").join("manifest.yaml")).unwrap();
        assert!(
            !manifest.contains("tier"),
            "the tier was persisted into manifest.yaml: {manifest}"
        );
    }

    /// The blast radius the publicize dialog names is computed from the tree, not
    /// from whatever the renderer had lying around.
    #[tokio::test]
    async fn the_tier_read_reports_what_a_publicize_would_release() {
        install_test_user_action_key();
        let (_d, root, app) = guarded_router();
        seed(&root, &app).await;
        std::fs::write(
            root.join("omop").join("knowledge").join("cohort.md"),
            "# cohort\n\nn=412\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("omop").join("raw").join("s1")).unwrap();
        std::fs::write(
            root.join("omop").join("raw").join("s1").join("meta.yaml"),
            "id: s1\n",
        )
        .unwrap();

        // As the publicize dialog asks it: with the user's proof. A caller
        // without it is refused this private base's tier and counts outright.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bases/omop/tier")
                    .header("X-Secret-Key", TEST_SECRET)
                    .header("X-User-Action", TEST_USER_ACTION_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["tier"], "private");
        assert_eq!(v["page_count"], 1);
        assert_eq!(v["raw_source_count"], 1);
    }

    /// A base that is not there is a 404, not a silent success that leaves the
    /// user believing they released something.
    #[tokio::test]
    async fn an_unknown_base_is_not_publicized() {
        install_test_user_action_key();
        let (_d, _root, app) = guarded_router();
        let (status, _) = post_tier(
            &app,
            "no-such-base",
            "public",
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 404);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Stage 6 — the OKF surface on HTTP
//
// Every assertion in this module reads the JSON a client actually receives,
// never a Rust value. That is the whole of the stage: Stage 2 grew `GraphNode`
// and `GraphEdge` and its own tests proved the **deriver** fills them, which is
// a different claim from "they reach the renderer". A `skip_serializing_if`
// that fires too eagerly, a schema the OpenAPI registration missed, or a route
// serving a cache written by an older shape would each leave those tests green
// and the client blind.
// ──────────────────────────────────────────────────────────────────────────────
mod okf_surface {
    use super::*;

    async fn post_bases(app: &Router, body: serde_json::Value) -> (u16, serde_json::Value) {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (
            status,
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        )
    }

    async fn get_json(app: &Router, uri: &str) -> (u16, serde_json::Value) {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    /// The default is OKF and it is stated, not left to be inferred.
    ///
    /// `format` is `#[serde(default)]` on the manifest, so a response that
    /// omitted the key would still deserialize to `okf` in every Rust reader and
    /// be invisible here — the assertion is on the JSON, where a missing key is
    /// `null` and fails.
    #[tokio::test]
    async fn a_base_created_over_http_without_a_format_is_okf() {
        let (_d, app) = build_test_router();
        let (status, manifest) =
            post_bases(&app, serde_json::json!({"id": "gen", "name": "Gen"})).await;
        assert_eq!(status, 200, "{manifest}");
        assert_eq!(manifest["format"], "okf", "{manifest}");
        assert!(
            manifest["okf_version"].is_string(),
            "an OKF bundle declares its revision: {manifest}"
        );
        assert!(
            manifest["biookf_version"].is_null(),
            "a plain-OKF base must not claim a BioOKF revision: {manifest}"
        );
    }

    /// The profile asked for is the profile created — in the manifest, in the
    /// listing, and in the `schema.md` the sub-agent is taught from. The last of
    /// the three is the one that matters most and the one a route test is
    /// otherwise tempted to skip: the format's whole effect is on writes, and a
    /// manifest that says `biookf` over an OKF schema would teach the wrong
    /// vocabulary for the life of the base (Stage 3).
    #[tokio::test]
    async fn creating_a_base_over_http_in_biookf_scaffolds_the_biookf_bundle() {
        let (_d, root, app) = build_test_router_with_root();
        let (status, manifest) = post_bases(
            &app,
            serde_json::json!({"id": "lit", "name": "Literature", "format": "biookf"}),
        )
        .await;
        assert_eq!(status, 200, "{manifest}");
        assert_eq!(manifest["format"], "biookf", "{manifest}");
        assert!(
            manifest["biookf_version"].is_string(),
            "a BioOKF bundle declares its profile revision: {manifest}"
        );
        assert!(
            manifest["okf_version"].is_string(),
            "…and still declares the OKF revision it profiles: {manifest}"
        );

        let (status, bases) = get_json(&app, "/bases").await;
        assert_eq!(status, 200);
        assert_eq!(
            bases[0]["format"], "biookf",
            "the listing carries it too: {bases}"
        );

        let schema = std::fs::read_to_string(root.join("lit").join("schema.md")).unwrap();
        assert!(
            schema.contains("BioOKF"),
            "a biookf base must be scaffolded with the BioOKF schema, got:\n{schema}"
        );
    }

    /// A request is not a file (DR-7 / DR-12).
    ///
    /// `KbFormat`'s `Deserialize` is lenient on purpose, so a typed parameter
    /// would have answered 200 here and created a plain-OKF base under a name
    /// the caller never asked for — discovered pages later, with no conversion
    /// available (DR-22/DR-26). The second half of the test is the load-bearing
    /// one: the refusal happens before anything is created.
    #[tokio::test]
    async fn a_misspelt_format_is_refused_with_400_and_creates_nothing() {
        let (_d, root, app) = build_test_router_with_root();
        let (status, body) = post_bases(
            &app,
            serde_json::json!({"id": "lit", "name": "Literature", "format": "bio-okf"}),
        )
        .await;
        assert_eq!(status, 400, "a misspelt format created a base: {body}");
        let message = body.as_str().unwrap_or_default();
        for word in ["bio-okf", "okf", "biookf"] {
            assert!(
                message.contains(word),
                "the refusal must name what was asked for and what exists, got: {message}"
            );
        }

        assert!(
            !root.join("lit").exists(),
            "a refused create must not leave a half-scaffolded base on disk"
        );
        // Asked as the user, who is told it is not there; a caller without the
        // proof gets the refusal it gets for a private base (QA 2026-09-10 H2).
        super::tier_route::install_test_user_action_key();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bases/lit")
                    .header("X-User-Action", super::tier_route::TEST_USER_ACTION_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 404, "and the base must not be readable");
    }

    /// The typed graph, on the wire.
    ///
    /// One BioOKF page carrying a frontmatter `edges:` entry with the full §8.1
    /// provenance triplet, a §7.3 statistic, a §7.2 qualifier, an unmodelled
    /// attribute and a dangling object — so a single request exercises every
    /// channel Stage 2 opened, including the two open maps and the `external`
    /// placeholder.
    #[tokio::test]
    async fn the_graph_route_serves_every_typed_field_stage_2_added() {
        let (_d, root, app) = build_test_router_with_root();
        create_kb(app.clone(), "tox", "Tox").await;
        let kb = root.join("tox");
        let dir = kb.join("knowledge").join("chemical-substance");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tocilizumab.md"),
            "---\n\
             type: ChemicalSubstance\n\
             identifier: Tocilizumab\n\
             subtype: monoclonal antibody\n\
             status: draft\n\
             edges:\n\
             \x20 - predicate: not_treats\n\
             \x20   object: Neutropenia\n\
             \x20   knowledge_level: statistical_association\n\
             \x20   agent_type: manual_agent\n\
             \x20   primary_source: PMID:12345678\n\
             \x20   publications: [PMID:12345678, PMID:87654321]\n\
             \x20   p_value: 3.0e-6\n\
             \x20   species_context: human\n\
             ---\n\
             Tocilizumab does not treat neutropenia.\n",
        )
        .unwrap();
        // `create_base` wrote a cache for the empty base and the page-write route
        // does not invalidate it, so the route would otherwise answer from the
        // snapshot taken before this page existed. Clearing it puts the request
        // on `get_graph`'s re-derive path — the same path DR-13 makes every
        // pre-Stage-2 cache take.
        std::fs::remove_file(kb.join(".biorouter-knowledge").join("graph-cache.json")).unwrap();

        let (status, graph) = get_json(&app, "/bases/tox/graph").await;
        assert_eq!(status, 200, "{graph}");

        let nodes = graph["nodes"].as_array().unwrap();
        let node = nodes
            .iter()
            .find(|n| n["identifier"] == "Tocilizumab")
            .unwrap_or_else(|| panic!("no typed node on the wire: {graph}"));
        assert_eq!(node["node_type"], "ChemicalSubstance", "{node}");
        assert_eq!(node["subtype"], "monoclonal antibody", "{node}");
        assert_eq!(node["status"], "draft", "{node}");
        assert_eq!(node["degree"], 1, "{node}");

        // The dangling object is recorded, not dropped: OKF §11 makes a broken
        // cross-link something a consumer MUST tolerate, and the placeholder is
        // the curation queue made visible.
        let external = nodes
            .iter()
            .find(|n| n["external"] == true)
            .unwrap_or_else(|| panic!("the unresolved object did not reach the wire: {graph}"));
        assert_eq!(external["label"], "Neutropenia", "{external}");

        let edges = graph["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1, "{graph}");
        let edge = &edges[0];
        assert_eq!(edge["predicate"], "not_treats", "{edge}");
        assert_eq!(
            edge["relation"], "not_treats",
            "the deprecated alias still carries the identical value: {edge}"
        );
        assert_eq!(edge["negated"], true, "{edge}");
        assert_eq!(edge["knowledge_level"], "statistical_association", "{edge}");
        assert_eq!(edge["agent_type"], "manual_agent", "{edge}");
        assert_eq!(edge["primary_source"], "PMID:12345678", "{edge}");
        assert_eq!(
            edge["publications"],
            serde_json::json!(["PMID:12345678", "PMID:87654321"]),
            "{edge}"
        );
        // DR-27's two open maps, and that they stay two. A p-value filed as
        // context is a category error no renderer can undo.
        assert_eq!(
            edge["quantitative"]["p_value"],
            serde_json::json!(3.0e-6),
            "a numeric statistic must arrive as a JSON number: {edge}"
        );
        assert!(
            edge["qualifiers"]["p_value"].is_null(),
            "a §7.3 statistic must not also land in the context map: {edge}"
        );
        assert_eq!(edge["qualifiers"]["species_context"], "human", "{edge}");
    }

    /// A legacy page is untouched by all of it, in the same response shape.
    ///
    /// The typed fields are `Option` (DR-28) and `skip_serializing_if`, so a
    /// legacy node arrives without them rather than with invented values — a
    /// `status: "stable"` stamped on a page that states none would turn a
    /// consumer's §5.4 assumption into the producer's assertion.
    #[tokio::test]
    async fn an_untyped_page_arrives_with_no_typed_fields_rather_than_invented_ones() {
        let (_d, root, app) = build_test_router_with_root();
        create_kb(app.clone(), "old", "Old").await;
        let kb = root.join("old");
        let dir = kb.join("knowledge").join("notes");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("hrv.md"),
            "---\ntitle: HRV\nkind: note\n---\nbody\n",
        )
        .unwrap();
        std::fs::remove_file(kb.join(".biorouter-knowledge").join("graph-cache.json")).unwrap();

        let (status, graph) = get_json(&app, "/bases/old/graph").await;
        assert_eq!(status, 200, "{graph}");
        let node = &graph["nodes"][0];
        for absent in ["node_type", "subtype", "status", "external", "stale"] {
            assert!(
                node[absent].is_null(),
                "a legacy page must not gain `{absent}`: {node}"
            );
        }
        assert_eq!(
            node["degree"], 0,
            "…but degree is counted, and zero is an answer: {node}"
        );
    }

    /// The lint stream's terminal frame carries the typed diagnostics, not only
    /// the four bags of strings it has always carried.
    ///
    /// `autofix: false` builds no provider at all, so this drives the real route
    /// end to end with no model and no test-mode env var — which is also why it
    /// can live beside the un-mocked provider tests in this binary.
    #[tokio::test]
    async fn the_lint_stream_terminal_frame_carries_typed_diagnostics() {
        let (_d, root, app) = build_test_router_with_root();
        create_kb(app.clone(), "lint6", "Lint6").await;
        let dir = root.join("lint6").join("knowledge").join("notes");
        std::fs::create_dir_all(&dir).unwrap();
        // Linked by nothing, so the hygiene scan raises `kb.orphan` for it.
        std::fs::write(
            dir.join("lonely.md"),
            "---\ntitle: Lonely\n---\nnobody links here\n",
        )
        .unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases/lint6/lint")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "model": {"provider": "nonexistent_provider_xyz", "model": "x"},
                            "autofix": false
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "a read-only lint needs no provider");
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let stream = String::from_utf8(bytes.to_vec()).unwrap();

        let payload = stream
            .split("event: done\ndata: ")
            .nth(1)
            .unwrap_or_else(|| panic!("no terminal done frame; stream was:\n{stream}"))
            .split("\n\n")
            .next()
            .unwrap();
        let result: serde_json::Value =
            serde_json::from_str(payload).expect("the done frame must be valid JSON");
        // A `LintResult` wrapping a `LintReport`, and the wrapper is asserted
        // rather than reached through: it is the shape the published schema
        // claims, so a flattening would be a silent contract break.
        assert!(
            result["commit_sha"].is_null() && result["fixes_applied"] == 0,
            "a read-only lint changed nothing and must say so: {result}"
        );
        let report = &result["report"];

        assert_eq!(
            report["orphans"],
            serde_json::json!(["knowledge/notes/lonely.md"]),
            "the four lists are still there — the typed layer adds, it does not replace: {report}"
        );
        let items = report["diagnostics"]["items"]
            .as_array()
            .unwrap_or_else(|| panic!("the terminal frame carries no typed diagnostics: {report}"));
        let orphan = items
            .iter()
            .find(|d| d["rule"] == "kb.orphan")
            .unwrap_or_else(|| panic!("no kb.orphan diagnostic: {report}"));
        // The four fields the UI matches on. `rule` is the stable one; `message`
        // is prose and will be reworded, which is why nothing keys off it.
        assert_eq!(orphan["severity"], "warning", "{orphan}");
        assert_eq!(orphan["subject"], "knowledge/notes/lonely.md", "{orphan}");
        assert!(
            orphan["message"].as_str().is_some_and(|m| !m.is_empty()),
            "{orphan}"
        );

        // …and the format layer reaches the same list under its own prefix. The
        // page states no `type`, which OKF §4.1 makes the one always-required
        // key — so this is the distinction the prefixes exist to draw, on the
        // wire: "your base is untidy" (`kb.`) beside "this file is not
        // conformant" (`okf.`).
        let conformance = items
            .iter()
            .find(|d| d["rule"] == "okf.type.missing")
            .unwrap_or_else(|| panic!("the OKF layer did not reach the frame: {report}"));
        assert_eq!(conformance["severity"], "error", "{conformance}");
        assert_eq!(
            conformance["path"], "knowledge/notes/lonely.md",
            "{conformance}"
        );
        assert_eq!(
            report["diagnostics"]["total"],
            serde_json::json!(items.len()),
            "an uncapped report's total is its length: {report}"
        );
    }
}

/// `POST /knowledge/bases/{id}/merge` — the user's own KB-to-KB merge.
///
/// ⚠ It borrows `tier_route`'s key and router builder rather than minting its
/// own, and that is forced rather than tidy: the installed user-action digest is
/// a process-global `OnceLock`, so a second `install_user_action_digest` in this
/// binary is a no-op by construction and merge tests keyed on their own secret
/// would every one of them 403 for a reason that has nothing to do with merging.
mod merge_route {
    use super::tier_route::{
        guarded_router, install_test_user_action_key, TEST_SECRET, TEST_USER_ACTION_KEY,
    };
    use super::*;
    use biorouter_mcp::knowledge::tier;

    async fn create(app: &Router, id: &str) {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("X-User-Action", tier_route::TEST_USER_ACTION_KEY)
                    .header("content-type", "application/json")
                    .header("X-Secret-Key", TEST_SECRET)
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({ "id": id, "name": id })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }

    async fn post_merge(
        app: &Router,
        kb_id: &str,
        body: serde_json::Value,
        secret: Option<&str>,
        user_action: Option<&str>,
    ) -> (u16, String) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/bases/{kb_id}/merge"))
            .header("content-type", "application/json");
        if let Some(key) = secret {
            builder = builder.header("X-Secret-Key", key);
        }
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let req = builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The proof-of-user is the whole of what separates this route from the tool
    /// channel: it skips the caller barrier because the user can already read
    /// both bases, so a caller holding only the secret key — which §9.3 A1 puts
    /// inside any developer-enabled agent shell — must not reach it.
    #[tokio::test]
    async fn the_merge_route_needs_more_than_the_secret_key() {
        install_test_user_action_key();
        let (_d, root, app) = guarded_router();
        create(&app, "dst").await;
        create(&app, "src").await;
        std::fs::write(
            root.join("src/knowledge/only-here.md"),
            valid_page("note", "Only Here", "b"),
        )
        .unwrap();

        let body = serde_json::json!({ "source_kb_id": "src", "dry_run": false });
        let (status, _) = post_merge(&app, "dst", body.clone(), None, None).await;
        assert_eq!(status, 401);

        let (status, refusal) =
            post_merge(&app, "dst", body.clone(), Some(TEST_SECRET), None).await;
        assert_eq!(status, 403, "a secret-key-only caller merged two bases");
        assert!(
            refusal.contains("Do not retry"),
            "the refusal must foreclose the retry: {refusal}"
        );
        assert!(
            !root.join("dst/knowledge/only-here.md").exists(),
            "the refused call merged anyway"
        );

        let (status, out) = post_merge(
            &app,
            "dst",
            body,
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 200, "got {out}");
        assert!(root.join("dst/knowledge/only-here.md").exists(), "{out}");
    }

    /// The body's `dry_run` defaults to **true**. A client that forgets the field
    /// gets the preview, never the merge — this is the least reversible
    /// operation in the subsystem and `POST /restore` restores a whole tree.
    ///
    /// The same call also proves the fold reaches the HTTP surface: a private
    /// source raises the public destination, and the *preview* does not.
    #[tokio::test]
    async fn an_omitted_dry_run_previews_and_a_stated_false_merges() {
        install_test_user_action_key();
        let (_d, root, app) = guarded_router();
        create(&app, "pubdst").await;
        create(&app, "privsrc").await;
        std::fs::write(
            root.join("privsrc/knowledge/secret-note.md"),
            valid_page("note", "Secret Note", "b"),
        )
        .unwrap();
        tier::raise_unlocked(&root, "privsrc", /* caller_is_private */ true).unwrap();
        assert!(!tier::is_private(&root, "pubdst"));

        let (status, preview) = post_merge(
            &app,
            "pubdst",
            serde_json::json!({ "source_kb_id": "privsrc" }),
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 200, "got {preview}");
        let preview: serde_json::Value = serde_json::from_str(&preview).unwrap();
        assert_eq!(preview["dry_run"], serde_json::json!(true));
        assert_eq!(preview["destination_tier"], serde_json::json!("private"));
        assert_eq!(preview["pages_carried"].as_array().unwrap().len(), 1);
        assert!(
            !root.join("pubdst/knowledge/secret-note.md").exists(),
            "the default-on preview wrote to the destination"
        );
        assert!(
            !tier::is_private(&root, "pubdst"),
            "the preview ratcheted the destination's tier"
        );

        let (status, out) = post_merge(
            &app,
            "pubdst",
            serde_json::json!({ "source_kb_id": "privsrc", "dry_run": false }),
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, 200, "got {out}");
        assert!(root.join("pubdst/knowledge/secret-note.md").exists());
        assert!(
            tier::is_private(&root, "pubdst"),
            "a public base absorbed a private base's pages and stayed public"
        );
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// QA 2026-09-10, H2 — a private knowledge base over HTTP
//
// The tool path refused a public caller (`kb_read_page`, `kb_search`,
// `kb_list_pages`, `kb_export`; `kb_list_bases` omits the base), while every
// `/knowledge/bases/{id}/…` route handed the same base's pages, graph, history
// and a `.brkb` of the whole tree to a caller holding nothing but the daemon
// secret — which a public chat's own shell recovered with `ps eww`. These tests
// are that caller, and the person at the keyboard beside it.
// ──────────────────────────────────────────────────────────────────────────────
mod h2_http_barrier {
    use super::tier_route::{install_test_user_action_key, TEST_USER_ACTION_KEY};
    use super::*;
    use biorouter_mcp::knowledge::tier;

    /// Appears in the seeded pages and nowhere else, so "the content came
    /// back" is an assertion rather than an impression.
    const SENTINEL: &str = "qa-h2-private-page-marker-not-real-data";
    const PRIVATE_KB: &str = "omop";
    const PUBLIC_KB: &str = "notes";
    /// A well-formed id that names no base on this machine.
    const ABSENT_KB: &str = "no-such-base";

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        proof: bool,
    ) -> (u16, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        if proof {
            builder = builder.header("X-User-Action", TEST_USER_ACTION_KEY);
        }
        let body = match body {
            Some(json) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&json).unwrap())
            }
            None => Body::empty(),
        };
        let res = app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Two bases through the real routes, each with one page and one commit;
    /// the first is then ratcheted private the way a private chat's ingest
    /// leaves it. Returns that base's commit for the history-shaped routes.
    async fn seed(app: &Router, root: &std::path::Path) -> String {
        for (id, name) in [(PRIVATE_KB, "OMOP"), (PUBLIC_KB, "Notes")] {
            create_kb(app.clone(), id, name).await;
            let (status, body) = call(
                app,
                "PUT",
                &format!("/bases/{id}/pages/knowledge/x.md"),
                Some(serde_json::json!({
                    "content": valid_page("note", "X", &format!("# X\n\n{SENTINEL} in {id}")),
                    "commit_message": "seed",
                })),
                false,
            )
            .await;
            assert_eq!(status, 200, "seeding {id}: {body}");
        }
        tier::raise_unlocked(root, PRIVATE_KB, true).unwrap();
        let (status, body) = call(
            app,
            "GET",
            &format!("/bases/{PRIVATE_KB}/history"),
            None,
            true,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let history: serde_json::Value = serde_json::from_str(&body).unwrap();
        history[0]["commit_sha"].as_str().unwrap().to_string()
    }

    fn model() -> serde_json::Value {
        // Unknown to the registry: an admitted macro stops at `build_completer`
        // with a 400, long before any model is reached.
        serde_json::json!({ "provider": "qa-h2-no-such-provider", "model": "m" })
    }

    /// Every route under `/bases/{id}`, as `(method, uri, body)`.
    ///
    /// ⚠ **Destructive last**, for the reason the chat sweep gives: before this
    /// change `DELETE` removed the base outright, and every row after it would
    /// then have been probing an absent id.
    fn base_addressing_routes(
        id: &str,
        sha: &str,
    ) -> Vec<(&'static str, String, Option<serde_json::Value>)> {
        vec![
            ("GET", format!("/bases/{id}"), None),
            ("GET", format!("/bases/{id}/tier"), None),
            ("GET", format!("/bases/{id}/graph"), None),
            ("GET", format!("/bases/{id}/location"), None),
            ("GET", format!("/bases/{id}/page?path=knowledge/x.md"), None),
            ("GET", format!("/bases/{id}/pages"), None),
            ("GET", format!("/bases/{id}/pages/knowledge/x.md"), None),
            ("GET", format!("/bases/{id}/history"), None),
            (
                "POST",
                format!("/bases/{id}/preview"),
                Some(serde_json::json!({ "commit_sha": sha, "path": "knowledge/x.md" })),
            ),
            ("GET", format!("/bases/{id}/export"), None),
            (
                "POST",
                format!("/bases/{id}/query"),
                Some(serde_json::json!({ "question": "what is in it?", "model": model() })),
            ),
            (
                "POST",
                format!("/bases/{id}/lint"),
                Some(serde_json::json!({ "model": model() })),
            ),
            ("POST", format!("/bases/{id}/sources/s1/reclassify"), None),
            (
                "PUT",
                format!("/bases/{id}/sources/s1/credibility"),
                Some(serde_json::json!({})),
            ),
            (
                "POST",
                format!("/bases/{id}/tier"),
                Some(serde_json::json!({ "tier": "public" })),
            ),
            (
                "POST",
                format!("/bases/{id}/merge"),
                Some(serde_json::json!({ "source_kb_id": PUBLIC_KB })),
            ),
            (
                "PUT",
                format!("/bases/{id}"),
                Some(serde_json::json!({ "name": "renamed by an unproven caller" })),
            ),
            (
                "PUT",
                format!("/bases/{id}/default-model"),
                Some(serde_json::json!({ "model": model() })),
            ),
            (
                "PUT",
                format!("/bases/{id}/pages/knowledge/x.md"),
                Some(serde_json::json!({
                    "content": valid_page("note", "X", "overwritten by an unproven caller"),
                    "commit_message": "overwrite",
                })),
            ),
            (
                "POST",
                format!("/bases/{id}/raw"),
                Some(serde_json::json!({ "text": "an unproven raw source", "title": "t" })),
            ),
            (
                "POST",
                format!("/bases/{id}/ingest"),
                Some(serde_json::json!({ "source": { "text": "t" }, "model": model() })),
            ),
            (
                "POST",
                format!("/bases/{id}/ingest-conversation"),
                Some(serde_json::json!({ "session_ids": ["29990101_1"], "model": model() })),
            ),
            (
                "POST",
                format!("/bases/{id}/restore"),
                Some(serde_json::json!({ "commit_sha": sha })),
            ),
            ("DELETE", format!("/bases/{id}"), None),
        ]
    }

    /// **H2.** Every route under `/bases/{id}` answers an unproven caller on a
    /// private base exactly as the page read does — the same status and the
    /// same bytes — and answers a base that does not exist the same way, so the
    /// refusal is not an oracle for which ids name a private base.
    ///
    /// Collected rather than asserted row by row, so a regression names every
    /// door it reopened.
    #[tokio::test]
    async fn every_route_that_names_a_private_base_refuses_an_unproven_caller_as_the_read_does() {
        install_test_user_action_key();
        let (_d, root, app) = build_test_router_with_root();
        let sha = seed(&app, &root).await;

        let (read_status, read_body) = call(
            &app,
            "GET",
            &format!("/bases/{PRIVATE_KB}/page?path=knowledge/x.md"),
            None,
            false,
        )
        .await;
        assert_eq!(read_status, 403, "the private page was served: {read_body}");
        assert!(
            !read_body.contains(SENTINEL),
            "the refusal carried the page"
        );

        let mut leaks = Vec::new();
        for id in [PRIVATE_KB, ABSENT_KB] {
            for (method, uri, body) in base_addressing_routes(id, &sha) {
                let (status, got) = call(&app, method, &uri, body, false).await;
                if status != read_status || got != read_body {
                    leaks.push(format!("{method} {uri} -> {status}: {got:.160}"));
                }
            }
        }
        assert!(
            leaks.is_empty(),
            "a caller holding nothing but the daemon secret was answered differently from the \
             page read by {} route(s):\n  {}",
            leaks.len(),
            leaks.join("\n  ")
        );

        // …and nothing moved: the base is still there, still private, and its
        // page still says what it said.
        assert!(root.join(PRIVATE_KB).join("knowledge/x.md").exists());
        assert!(tier::is_private(&root, PRIVATE_KB));
        let page = std::fs::read_to_string(root.join(PRIVATE_KB).join("knowledge/x.md")).unwrap();
        assert!(
            page.contains(SENTINEL),
            "an unproven caller rewrote a private page"
        );
    }

    /// The other half — "refuse the unproven caller" is satisfied by "refuse
    /// everyone", and the Knowledge view must keep working. The person at the
    /// keyboard reads the private base in full, and gets the honest 404 for a
    /// base that is not there.
    #[tokio::test]
    async fn the_person_at_the_keyboard_still_reads_their_own_private_base() {
        install_test_user_action_key();
        let (_d, root, app) = build_test_router_with_root();
        let sha = seed(&app, &root).await;

        for uri in [
            format!("/bases/{PRIVATE_KB}/page?path=knowledge/x.md"),
            format!("/bases/{PRIVATE_KB}/pages/knowledge/x.md"),
        ] {
            let (status, body) = call(&app, "GET", &uri, None, true).await;
            assert_eq!(status, 200, "{uri}: {body}");
            assert!(
                body.contains(SENTINEL),
                "{uri} came back without the page: {body}"
            );
        }
        for uri in [
            format!("/bases/{PRIVATE_KB}"),
            format!("/bases/{PRIVATE_KB}/tier"),
            format!("/bases/{PRIVATE_KB}/graph"),
            format!("/bases/{PRIVATE_KB}/location"),
            format!("/bases/{PRIVATE_KB}/pages"),
            format!("/bases/{PRIVATE_KB}/history"),
            format!("/bases/{PRIVATE_KB}/export"),
        ] {
            let (status, body) = call(&app, "GET", &uri, None, true).await;
            assert_eq!(status, 200, "{uri}: {body:.200}");
        }
        let (status, body) = call(
            &app,
            "POST",
            &format!("/bases/{PRIVATE_KB}/preview"),
            Some(serde_json::json!({ "commit_sha": sha, "path": "knowledge/x.md" })),
            true,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains(SENTINEL));

        let (status, _) = call(&app, "GET", &format!("/bases/{ABSENT_KB}"), None, true).await;
        assert_eq!(
            status, 404,
            "the user is entitled to know the base is not there"
        );
    }

    /// A public base is untouched for a caller that proves nothing.
    #[tokio::test]
    async fn a_public_base_is_untouched_for_an_unproven_caller() {
        install_test_user_action_key();
        let (_d, root, app) = build_test_router_with_root();
        seed(&app, &root).await;
        let (status, body) = call(
            &app,
            "GET",
            &format!("/bases/{PUBLIC_KB}/page?path=knowledge/x.md"),
            None,
            false,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains(SENTINEL));
        let (status, body) = call(
            &app,
            "GET",
            &format!("/bases/{PUBLIC_KB}/export"),
            None,
            false,
        )
        .await;
        assert_eq!(status, 200, "{body:.200}");
    }

    /// `GET /knowledge/bases` OMITS a private base from an unproven caller —
    /// omission, not a 404 for the list and not a redacted row, because a
    /// base's id and name are user-authored content (the tool path's
    /// `kb_list_bases` makes the same choice).
    #[tokio::test]
    async fn the_bases_listing_omits_a_private_base_from_an_unproven_caller() {
        install_test_user_action_key();
        let (_d, root, app) = build_test_router_with_root();
        seed(&app, &root).await;

        let (status, body) = call(&app, "GET", "/bases", None, false).await;
        assert_eq!(status, 200, "{body}");
        let ids: Vec<String> = serde_json::from_str::<serde_json::Value>(&body)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_string())
            .collect();
        assert!(ids.contains(&PUBLIC_KB.to_string()), "{ids:?}");
        assert!(
            !ids.contains(&PRIVATE_KB.to_string()),
            "an unproven caller was listed a private base: {ids:?}"
        );
        assert!(
            !body.contains("OMOP"),
            "the private base's name leaked: {body}"
        );

        let (status, body) = call(&app, "GET", "/bases", None, true).await;
        assert_eq!(status, 200);
        assert!(
            body.contains(PRIVATE_KB) && body.contains(PUBLIC_KB),
            "{body}"
        );
    }

    /// `/knowledge/active` is the second listing of base ids, and the one the
    /// Knowledge view hydrates from. An unproven caller sees only what it can
    /// reach — and may not change what it cannot see: its writes leave a
    /// private base's hidden state and a private primary exactly where they
    /// were. Without that, a renderer that prunes ids missing from its
    /// (filtered) list would silently rewrite the machine-wide selection.
    #[tokio::test]
    async fn the_selection_shows_and_changes_only_what_an_unproven_caller_can_reach() {
        install_test_user_action_key();
        let (_d, root, app) = build_test_router_with_root();
        seed(&app, &root).await;

        // The user pins the private base as the machine-wide primary.
        let (status, body) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "primary_kb": PRIVATE_KB, "hidden_kbs": [] })),
            true,
        )
        .await;
        assert_eq!(status, 200, "{body}");

        // An unproven reader is told nothing about it.
        let (status, body) = call(&app, "GET", "/active", None, false).await;
        assert_eq!(status, 200, "{body}");
        let seen: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            !body.contains(PRIVATE_KB),
            "an unproven caller was shown a private base: {body}"
        );
        assert_eq!(seen["primary_kb"], serde_json::Value::Null);

        // It cannot clear what it cannot see…
        let (status, body) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "clear_primary": true })),
            false,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        // …cannot name it…
        let (named, named_body) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "primary_kb": PRIVATE_KB })),
            false,
        )
        .await;
        let (absent, absent_body) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "primary_kb": ABSENT_KB })),
            false,
        )
        .await;
        assert_eq!(named, 403, "{named_body}");
        assert_eq!(
            (named, named_body.as_str()),
            (absent, absent_body.as_str()),
            "naming a private base and naming no base answered differently"
        );
        assert!(
            !absent_body.contains(PRIVATE_KB),
            "the refusal enumerated a private id: {absent_body}"
        );

        // …and cannot hide it: an id it cannot reach is not its to move.
        let (status, body) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "hidden_kbs": [PRIVATE_KB] })),
            false,
        )
        .await;
        assert_eq!(status, 200, "{body}");

        let (status, body) = call(&app, "GET", "/active", None, true).await;
        assert_eq!(status, 200, "{body}");
        let truth: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(truth["primary_kb"], serde_json::json!(PRIVATE_KB), "{body}");
        assert!(
            !truth["hidden_kbs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == PRIVATE_KB),
            "an unproven caller hid a private base: {body}"
        );

        // The user hides it; an unproven caller that rewrites the set cannot
        // bring it back.
        let (status, _) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "hidden_kbs": [PRIVATE_KB], "clear_primary": true })),
            true,
        )
        .await;
        assert_eq!(status, 200);
        let (status, body) = call(
            &app,
            "POST",
            "/active",
            Some(serde_json::json!({ "hidden_kbs": [] })),
            false,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(!body.contains(PRIVATE_KB), "{body}");
        let (_, body) = call(&app, "GET", "/active", None, true).await;
        let truth: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            truth["hidden_kbs"],
            serde_json::json!([PRIVATE_KB]),
            "an unproven caller un-hid a private base it could not see: {body}"
        );
    }

    /// `POST /bases/{id}/ingest-conversation` names chats as well as a base,
    /// and streams what the macro makes of them back to the caller. So the
    /// caller must be able to reach every chat it names — the same gate, with
    /// the same refusal, as `GET /sessions/{id}` — before a transcript is read.
    #[tokio::test]
    async fn conversation_ingest_refuses_a_private_chat_to_an_unproven_caller() {
        use biorouter::session::session_manager::{SessionManager, SessionType};
        install_test_user_action_key();
        let (_d, root, app) = build_test_router_with_root();
        seed(&app, &root).await;

        let manager = SessionManager::instance();
        let chat = manager
            .create_session(
                std::path::PathBuf::from("/tmp/qa_h2_ingest"),
                "QA H2 ingest (test fixture)".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        manager
            .add_message(
                &chat.id,
                &biorouter::conversation::message::Message::user().with_text(SENTINEL),
            )
            .await
            .unwrap();
        manager
            .update(&chat.id)
            .provider_name("versa_azure")
            .model_config(biorouter::model::ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(
                biorouter::privacy::SessionClassification::Private,
                "turn:versa_azure",
            )
            .apply()
            .await
            .unwrap();

        let ingest = |id: String| serde_json::json!({ "session_ids": [id], "model": model() });
        let uri = format!("/bases/{PUBLIC_KB}/ingest-conversation");
        let (private_status, private_body) =
            call(&app, "POST", &uri, Some(ingest(chat.id.clone())), false).await;
        let (absent_status, absent_body) = call(
            &app,
            "POST",
            &uri,
            Some(ingest("29990101_424242".into())),
            false,
        )
        .await;
        assert_eq!(private_status, 403, "{private_body}");
        assert_eq!(
            (private_status, private_body.as_str()),
            (absent_status, absent_body.as_str()),
            "a private chat and an absent one answered differently"
        );
        assert!(!private_body.contains(SENTINEL));

        // The person at the keyboard gets past the gate to the handler's own
        // answer — the unknown provider's 400.
        let (status, body) = call(&app, "POST", &uri, Some(ingest(chat.id.clone())), true).await;
        assert_eq!(status, 400, "{body}");

        manager.delete_session(&chat.id).await.unwrap();
    }
}
