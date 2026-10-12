//! Integration tests for the /ollama HTTP routes, against a fake Ollama on
//! loopback.
//!
//! A binary of its own because the routes read `OLLAMA_HOST` from the process
//! environment, the way the provider does: every test here points it at the
//! same fake server, once, before any of them reads it. The developer's own
//! `ollama serve` on 11434 is never asked anything.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real config.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::OnceLock;

use axum::{body::Body, http::Request, routing::post, Json, Router};
use serde_json::{json, Value};
use tower::ServiceExt;

/// The fake server answers `/api/show` for this tag only, with a trained
/// window smaller than either automatic tier (64k / 128k), so the answer does
/// not depend on how much memory the machine running the test has.
const KNOWN_MODEL: &str = "fake-qwen:27b";
const KNOWN_TRAINED_WINDOW: u64 = 32_768;

/// Starts the fake Ollama (once) and points `OLLAMA_HOST` at it. Every test
/// calls this first, so none reads the environment before it is set.
fn fake_ollama() -> &'static str {
    static BASE: OnceLock<String> = OnceLock::new();
    BASE.get_or_init(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let app = Router::new().route("/api/show", post(show));
                axum::serve(listener, app).await.unwrap();
            });
        });
        let base = format!("http://127.0.0.1:{port}");
        // A developer's own settings would change the expected window.
        std::env::remove_var("OLLAMA_NUM_CTX");
        std::env::remove_var("BIOROUTER_CONTEXT_LIMIT");
        std::env::set_var("OLLAMA_HOST", &base);
        base
    })
}

async fn show(Json(body): Json<Value>) -> (axum::http::StatusCode, Json<Value>) {
    if body.get("model").and_then(Value::as_str) == Some(KNOWN_MODEL) {
        (
            axum::http::StatusCode::OK,
            Json(json!({
                "capabilities": ["completion", "tools", "thinking"],
                "model_info": {
                    "general.architecture": "qwen3",
                    "qwen3.context_length": KNOWN_TRAINED_WINDOW,
                },
            })),
        )
    } else {
        // Ollama's own answer for a tag that is not pulled.
        (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "model not found" })),
        )
    }
}

fn app() -> axum::Router {
    biorouter_server::routes::ollama::router()
}

async fn get(uri: &str) -> axum::response::Response {
    app()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body_json(res: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn recommended_lists_the_two_supported_models_default_first() {
    let base = fake_ollama();
    let res = get("/ollama/recommended").await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;

    let tags: Vec<&str> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["tag"].as_str().unwrap())
        .collect();
    // The product decision: exactly these two, Qwen first. `gpt-oss:20b`, the
    // tag onboarding used to pull, is not among them.
    assert_eq!(tags, ["qwen3.8:27b", "gemma4:31b"]);
    assert_eq!(v["models"][0]["is_default"], true);
    assert_eq!(v["models"][1]["is_default"], false);
    assert_eq!(v["default_model"], "qwen3.8:27b");
    assert_eq!(v["min_memory_gib"], 32);
    assert!(v["models"][0]["approx_download_gb"].as_u64().unwrap() > 0);

    // The server a chat would use, resolved as the provider resolves it.
    assert_eq!(v["base_url"], format!("{base}/"));
    assert_eq!(v["host_configured"], true);
    // A loopback server: the tier a chat on it gets. The other hosts are in
    // `ollama_routes_tier.rs`, which owns its process's `OLLAMA_HOST`.
    assert_eq!(v["tier"], "private");

    let status = v["suitability"].as_str().unwrap();
    assert!(
        ["suitable", "below_minimum", "cpu_only", "unknown_resources"].contains(&status),
        "{status}"
    );
    assert!(!v["suitability_message"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn context_window_is_the_window_the_provider_sends() {
    fake_ollama();
    let res = get(&format!("/ollama/context_window?model={KNOWN_MODEL}")).await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    assert_eq!(v["model"], KNOWN_MODEL);
    // Capped at the trained window, whatever this machine's memory tier is:
    // the provider never sends Ollama more than the model was trained for.
    assert_eq!(v["context_window"], KNOWN_TRAINED_WINDOW);
}

#[tokio::test]
async fn context_window_is_null_for_a_model_the_server_cannot_describe() {
    fake_ollama();
    let res = get("/ollama/context_window?model=not-pulled:7b").await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    // Null rather than a name-based guess: a guess is what told the gauge 128k
    // while Ollama served 4k.
    assert!(v["context_window"].is_null(), "{v}");
}

#[tokio::test]
async fn context_window_requires_a_model() {
    fake_ollama();
    let res = get("/ollama/context_window?model=%20").await;
    assert_eq!(res.status(), 400);
    let res = get("/ollama/context_window").await;
    assert_eq!(res.status(), 400);
}
