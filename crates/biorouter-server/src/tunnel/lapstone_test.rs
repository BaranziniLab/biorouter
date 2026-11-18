//! Integration tests for the Lapstone HTTP tunnel
//!
//! These tests verify the full tunnel flow:
//! 1. Start a local HTTP server
//! 2. Start the tunnel (connects to real Cloudflare worker via WebSocket)
//! 3. Make requests to the public HTTPS URL
//! 4. Verify they proxy through to the local server

use super::{lapstone, TunnelState};
use axum::{
    extract::Request,
    response::Json,
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

const TEST_TUNNEL_SECRET: &str = "test-tunnel-secret-12345";
const TEST_SERVER_SECRET: &str = "test-server-secret-67890";

/// Opt-in switch for the two tunnel tests below.
///
/// They dial a real third-party Cloudflare worker
/// (`cloudflare-tunnel-proxy.michael-neale.workers.dev`) over the public
/// internet. That makes them an integration suite that happens to live in
/// `src/`, and it means a red run can be a fact about somebody else's service
/// rather than about this repository. Measured on 2026-08-09: a DNS failure
/// with the network down, and a plain `503 Service Unavailable` from the
/// worker with it up. Neither is a regression anyone here can fix.
///
/// ⚠ The cost was larger than it looks, because the same two tests ran
/// TWELVE times per CI run. `.github/workflows/rust.yml` runs
/// `cargo test --workspace --lib --bins` across a three-OS matrix, and
/// `biorouter-server/src/main.rs` re-declares `mod tunnel;` rather than using
/// the library, so the `--lib` and `--bins` targets each carry a copy:
/// 2 tests x 2 targets x 3 platforms.
///
/// Gating on an env var rather than `#[ignore]` follows
/// `crates/biorouter/tests/providers.rs`, which self-skips the same way when
/// its credentials are absent. Run them with
/// `BIOROUTER_TEST_LAPSTONE_TUNNEL=1 cargo test -p biorouter-server --lib tunnel::`.
fn tunnel_tests_enabled() -> bool {
    std::env::var("BIOROUTER_TEST_LAPSTONE_TUNNEL").is_ok()
}

async fn find_available_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to port 0");
    let addr = listener.local_addr().expect("Failed to get local address");
    addr.port()
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "message": "Test server is running"
    }))
}

async fn echo_handler(req: Request) -> Json<Value> {
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let body = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    let body_str = String::from_utf8_lossy(&body).to_string();

    Json(json!({
        "headers": headers,
        "body": body_str
    }))
}

fn create_test_server() -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/echo", post(echo_handler))
}

async fn start_test_http_server(port: u16) -> tokio::task::JoinHandle<()> {
    let app = create_test_server();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    })
}

#[tokio::test]
async fn test_tunnel_end_to_end() {
    if !tunnel_tests_enabled() {
        println!(
            "Skipping test_tunnel_end_to_end - set BIOROUTER_TEST_LAPSTONE_TUNNEL to dial the live tunnel worker"
        );
        return;
    }
    let port = find_available_port().await;
    let server_handle = start_test_http_server(port).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let handle = Arc::new(RwLock::new(None));
    let (restart_tx, _restart_rx) = mpsc::channel(1);

    let tunnel_secret = TEST_TUNNEL_SECRET.to_string();
    let server_secret = TEST_SERVER_SECRET.to_string();
    let agent_id = super::generate_agent_id();

    let tunnel_info = lapstone::start(
        port,
        tunnel_secret.clone(),
        server_secret.clone(),
        agent_id.clone(),
        handle.clone(),
        restart_tx,
        Arc::new(RwLock::new(TunnelState::Starting)),
    )
    .await
    .expect("Failed to start tunnel");

    let public_url = &tunnel_info.url;
    println!("Tunnel public URL: {}", public_url);

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/health", public_url))
        .header("X-Secret-Key", &tunnel_secret)
        .send()
        .await
        .expect("Failed to make request to public URL");

    assert!(
        response.status().is_success(),
        "Response status: {}",
        response.status()
    );
    let body: Value = response.json().await.expect("Failed to parse JSON");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["message"], "Test server is running");

    lapstone::stop(handle).await;
    server_handle.abort();
}

#[tokio::test]
async fn test_tunnel_post_request() {
    if !tunnel_tests_enabled() {
        println!(
            "Skipping test_tunnel_post_request - set BIOROUTER_TEST_LAPSTONE_TUNNEL to dial the live tunnel worker"
        );
        return;
    }
    let port = find_available_port().await;
    let server_handle = start_test_http_server(port).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let handle = Arc::new(RwLock::new(None));
    let (restart_tx, _restart_rx) = mpsc::channel(1);

    let tunnel_secret = TEST_TUNNEL_SECRET.to_string();
    let server_secret = TEST_SERVER_SECRET.to_string();
    let agent_id = super::generate_agent_id();

    let tunnel_info = lapstone::start(
        port,
        tunnel_secret.clone(),
        server_secret.clone(),
        agent_id.clone(),
        handle.clone(),
        restart_tx,
        Arc::new(RwLock::new(TunnelState::Starting)),
    )
    .await
    .expect("Failed to start tunnel");

    let public_url = &tunnel_info.url;

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let client = reqwest::Client::new();
    let test_body = json!({"test": "data", "number": 42});
    let response = client
        .post(format!("{}/echo", public_url))
        .header("X-Secret-Key", &tunnel_secret)
        .header("Content-Type", "application/json")
        .json(&test_body)
        .send()
        .await
        .expect("Failed to make POST request");

    assert!(response.status().is_success());
    let body: Value = response.json().await.expect("Failed to parse JSON");
    assert!(body["body"].as_str().unwrap().contains("test"));
    assert!(body["body"].as_str().unwrap().contains("data"));
    assert!(body["body"].as_str().unwrap().contains("42"));

    lapstone::stop(handle).await;
    server_handle.abort();
}
