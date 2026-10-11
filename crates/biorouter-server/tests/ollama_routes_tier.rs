//! `/ollama/recommended`'s `tier`, for each kind of `OLLAMA_HOST`.
//!
//! A binary of its own because the route reads `OLLAMA_HOST` from the process
//! environment, the way the provider does, and this test changes it row by
//! row: in `ollama_routes.rs` that would race the tests that need it pointed
//! at their fake server. Nothing here touches the network — building the
//! provider and asking its tier reads config only.
//!
//! The desktop decides "this computer" from this field. It used to decide by
//! its own list of host names, which disagreed with the provider's loopback
//! rule in both directions: it called `0.0.0.0` this computer, under a card
//! promising "Free, private, offline", while the tier made the provider
//! Public; and it called `::1` another computer while the tier said Private.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real config.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{body::Body, http::Request};
use serde_json::Value;
use tower::ServiceExt;

async fn recommended() -> Value {
    let res = biorouter_server::routes::ollama::router()
        .oneshot(
            Request::builder()
                .uri("/ollama/recommended")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// One test, so the rows run in order against one `OLLAMA_HOST` at a time.
#[tokio::test]
async fn the_tier_is_the_providers_own_for_every_kind_of_host() {
    for (host, base_url, tier) in [
        ("localhost", "http://localhost:11434/", "private"),
        ("127.0.0.1:11500", "http://127.0.0.1:11500/", "private"),
        // Loopback by the provider's rule, though the renderer cannot probe it.
        ("::1", "http://[::1]:11434/", "private"),
        // What `ollama serve` is told to listen on everywhere. It reaches this
        // machine, but the loopback rule is deliberately lexical and narrow,
        // so a chat on it is Public, and the card must not say private.
        ("0.0.0.0", "http://0.0.0.0:11434/", "public"),
        (
            "http://192.168.1.10:11434",
            "http://192.168.1.10:11434/",
            "public",
        ),
        (
            "gpu-box.lab.example",
            "http://gpu-box.lab.example:11434/",
            "public",
        ),
    ] {
        std::env::set_var("OLLAMA_HOST", host);
        let v = recommended().await;
        assert_eq!(v["base_url"], base_url, "OLLAMA_HOST={host}: {v}");
        assert_eq!(v["tier"], tier, "OLLAMA_HOST={host}: {v}");
    }
    std::env::remove_var("OLLAMA_HOST");
}
