//! Integration tests for the /llamacpp HTTP routes.
//!
//! These avoid actually spawning a llama-server (that's covered by the
//! `llamacpp_integration` tests in the `biorouter` crate); they verify the
//! status/catalog contract the GUI onboarding card depends on, and input
//! validation on /llamacpp/ensure and /llamacpp/warmup.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{body::Body, http::Request};
use tower::ServiceExt;

fn app() -> axum::Router {
    biorouter_server::routes::llamacpp::router()
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn status_returns_catalog_with_default_first_class() {
    let res = app()
        .oneshot(
            Request::builder()
                .uri("/llamacpp/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let v = body_json(res).await;
    let catalog = v.get("catalog").and_then(|c| c.as_array()).unwrap();
    assert!(!catalog.is_empty(), "catalog must not be empty");

    // Exactly one default, and it is Qwen3.8 27B on every machine.
    let defaults: Vec<_> = catalog
        .iter()
        .filter(|m| m.get("is_default").and_then(|d| d.as_bool()) == Some(true))
        .collect();
    assert_eq!(defaults.len(), 1);
    let default_name = defaults[0].get("name").and_then(|n| n.as_str()).unwrap();
    assert_eq!(default_name, "qwen3.8-27b");
    assert!(defaults[0]
        .get("context_limit")
        .and_then(|n| n.as_u64())
        .is_some_and(|n| n >= 65_536));

    // 64k or 128k: a 32k window cannot hold Biorouter's instructions and tools.
    let default_context_size = v
        .get("system")
        .and_then(|s| s.get("default_context_size"))
        .and_then(|n| n.as_u64())
        .unwrap();
    assert!(
        [65_536, 131_072].contains(&default_context_size),
        "{default_context_size}"
    );
    assert!(
        v.get("system")
            .and_then(|s| s.get("accelerator_memory_kind"))
            .and_then(|k| k.as_str())
            .is_some(),
        "system info should expose whether recommendations use unified memory or VRAM"
    );
    assert!(
        v.get("system")
            .and_then(|s| s.get("model_cache_dir"))
            .and_then(|p| p.as_str())
            .is_some_and(|p| p.ends_with(".ollama/models") || !p.is_empty()),
        "system info should expose the Ollama-compatible model store"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("official_url")
            .and_then(|url| url.as_str())
            .is_some_and(|url| url.starts_with("https://ollama.com/library/"))),
        "catalog entries should expose the official Ollama library URL"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("family")
            .and_then(|family| family.as_str())
            .is_some_and(|family| ["Qwen3.8", "Gemma 4"].contains(&family))),
        "catalog entries should expose their model family"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("recommended_gpu_memory_gib")
            .and_then(|n| n.as_u64())
            .is_some()),
        "catalog entries should expose GPU-addressable memory recommendations"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("speed_hint")
            .and_then(|hint| hint.as_str())
            .is_some_and(|hint| !hint.trim().is_empty())),
        "catalog entries should expose a human-readable expected-speed hint"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("active_params_b")
            .and_then(|n| n.as_u64())
            .is_some_and(|n| n > 0)),
        "catalog entries should expose active parameters per token"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("downloaded")
            .and_then(|downloaded| downloaded.as_bool())
            .is_some()),
        "catalog entries should expose whether the model is already downloaded"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("download_status")
            .and_then(|status| status.as_str())
            .is_some_and(|status| ["downloaded", "partial", "not_downloaded"].contains(&status))),
        "catalog entries should expose a documented download status"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("download_source")
            .and_then(|status| status.as_str())
            .is_some_and(|status| ["ollama", "huggingface_cache", "none"].contains(&status))),
        "catalog entries should expose where any local copy was found"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("fallback_downloaded")
            .and_then(|downloaded| downloaded.as_bool())
            .is_some()),
        "catalog entries should expose whether the llama.cpp fallback is cached"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("fallback_download_status")
            .and_then(|status| status.as_str())
            .is_some_and(|status| ["downloaded", "partial", "not_downloaded"].contains(&status))),
        "catalog entries should expose fallback download status"
    );
    assert!(
        catalog.iter().all(|m| m
            .get("suitability_status")
            .and_then(|status| status.as_str())
            .is_some_and(|status| [
                "suitable",
                "above_recommendation",
                "below_minimum",
                "cpu_only",
                "unknown_resources"
            ]
            .contains(&status))),
        "catalog entries should expose a documented suitability status"
    );

    // Exactly the two supported models, the default first.
    let names: Vec<&str> = catalog
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
        .collect();
    assert_eq!(names, vec!["qwen3.8-27b", "gemma4-31b"]);
    let entry = |name: &str| {
        catalog
            .iter()
            .find(|m| m.get("name").and_then(|n| n.as_str()) == Some(name))
            .unwrap()
    };
    let field = |name: &str, key: &str| {
        entry(name)
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    assert_eq!(
        field("qwen3.8-27b", "ollama_name").as_deref(),
        Some("qwen3.8:27b")
    );
    assert_eq!(
        field("qwen3.8-27b", "hf_spec").as_deref(),
        Some("ggml-org/Qwen3.8-27B-GGUF:Q4_K_M")
    );
    assert_eq!(
        field("gemma4-31b", "ollama_name").as_deref(),
        Some("gemma4:31b")
    );
    assert_eq!(
        field("gemma4-31b", "hf_spec").as_deref(),
        Some("google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0")
    );
    // Launch policy is not part of the catalog the GUI reads.
    for key in [
        "gguf_sha256",
        "sampling_thinking",
        "spec_type",
        "ollama_gguf_names",
    ] {
        assert!(entry("qwen3.8-27b").get(key).is_none(), "{key} leaked");
    }

    // Sidecar state is one of the documented values.
    let state = v
        .get("sidecar")
        .and_then(|s| s.get("state"))
        .and_then(|s| s.as_str())
        .unwrap();
    assert!(
        ["no_binary", "stopped", "starting", "ready", "error"].contains(&state),
        "unexpected sidecar state {state}"
    );
    assert!(
        v.get("sidecar")
            .and_then(|s| s.get("warmed"))
            .and_then(|w| w.as_bool())
            .is_some(),
        "sidecar status should expose warmed state"
    );
}

#[tokio::test]
async fn ensure_rejects_unknown_model() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/llamacpp/ensure")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-4o"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn ensure_rejects_a_flag_shaped_raw_spec() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/llamacpp/ensure")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"owner/repo --host 0.0.0.0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn warmup_rejects_unknown_model_before_starting_sidecar() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/llamacpp/warmup")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-4o"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn delete_rejects_unknown_model() {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/llamacpp/delete")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-4o"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

fn cached_repo(root: &std::path::Path, repo_dir: &str, file: &str) -> std::path::PathBuf {
    let repo_dir = root.join(repo_dir);
    let snapshot_dir = repo_dir.join("snapshots").join("rev");
    std::fs::create_dir_all(&snapshot_dir).unwrap();
    std::fs::write(snapshot_dir.join(file), b"GGUF").unwrap();
    repo_dir
}

async fn delete(model: &str) -> serde_json::Value {
    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/llamacpp/delete")
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"model":"{model}"}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    body_json(res).await
}

#[tokio::test]
async fn delete_removes_cached_huggingface_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_string_lossy().into_owned();
    let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);

    let repo_dir = cached_repo(
        tmp.path(),
        "models--ggml-org--Qwen3.8-27B-GGUF",
        "Qwen3.8-27B-Q4_K_M.gguf",
    );
    let v = delete("qwen3.8-27b").await;
    assert_eq!(
        v.get("deleted_fallback_cache")
            .and_then(|deleted| deleted.as_bool()),
        Some(true)
    );
    assert!(!repo_dir.exists());
}

/// A retired catalog name deletes ITS OWN orphaned cache. A chat that names
/// it runs Qwen3.8, but deletion must not follow that alias, or "delete
/// gemma4" would remove the 19 GB default model instead.
#[tokio::test]
async fn deleting_a_retired_model_deletes_its_own_cache_only() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_string_lossy().into_owned();
    let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);

    let retired = cached_repo(
        tmp.path(),
        "models--google--gemma-4-E4B-it-qat-q4_0-gguf",
        "gemma-4-E4B_q4_0-it.gguf",
    );
    let current = cached_repo(
        tmp.path(),
        "models--ggml-org--Qwen3.8-27B-GGUF",
        "Qwen3.8-27B-Q4_K_M.gguf",
    );
    let v = delete("gemma4").await;
    assert_eq!(
        v.get("deleted_fallback_cache")
            .and_then(|deleted| deleted.as_bool()),
        Some(true)
    );
    assert!(!retired.exists());
    assert!(current.exists(), "the default model's cache must survive");
}

#[tokio::test]
async fn stop_is_idempotent() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    drop(listener);
    let _guard = env_lock::lock_env([("LLAMACPP_PORT", Some(port.as_str()))]);

    let res = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/llamacpp/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    let state = v
        .get("sidecar")
        .and_then(|s| s.get("state"))
        .and_then(|s| s.as_str())
        .unwrap();
    assert!(state == "stopped" || state == "no_binary");
}
