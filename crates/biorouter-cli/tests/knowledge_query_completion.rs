#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn wave3_query_the_cli_refuses_blank_answers_without_printing_success() {
    for answer in ["", " \n\t ", "A cited answer [[fixture]]."] {
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let root = sandbox.path().join("root");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        let service =
            biorouter::knowledge::service::KnowledgeService::new(root.join("config/knowledge"));
        service.create_base("k", "K", None).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let request_calls = calls.clone();
        let router = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move || {
                let calls = request_calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({
                        "id": "synthetic-query-completion",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "gpt-4o-mini",
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": answer},
                            "finish_reason": "stop"
                        }],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_biorouter"))
                .args([
                    "knowledge",
                    "query",
                    "What is known?",
                    "--kb",
                    "k",
                    "--provider",
                    "openai",
                    "--model",
                    "gpt-4o-mini",
                ])
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("CONFIG_DIR", root.join("config"))
                .env("BIOROUTER_PATH_ROOT", &root)
                .env("BIOROUTER_DISABLE_KEYRING", "true")
                .env("OPENAI_HOST", endpoint)
                .env("OPENAI_BASE_PATH", "v1/chat/completions")
                .env("OPENAI_API_KEY", "synthetic-loopback-query-key")
                .env_remove("OPENAI_CUSTOM_HEADERS")
                .env_remove("OPENAI_ORGANIZATION")
                .env_remove("OPENAI_PROJECT")
                .env_remove("BIOROUTER_KNOWLEDGE_TEST_MODE")
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await;
        server.abort();
        let _ = server.await;
        let output = output.expect("the isolated query must finish").unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{stdout}\n{stderr}");
        if answer.trim().is_empty() {
            assert!(
                !output.status.success(),
                "a blank answer exited successfully: {stdout}"
            );
            assert!(stderr.contains("answer"), "{stderr}");
            assert!(!stdout.contains("Answer:"), "{stdout}");
            assert!(!stdout.contains("saved as a page"), "{stdout}");
        } else {
            assert!(output.status.success(), "{stderr}");
            assert!(stdout.contains(answer), "{stdout}");
            assert!(stdout.contains("cited pages"), "{stdout}");
        }
    }
}
