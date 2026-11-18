//! Account catalogue policy and existing Copilot wire compatibility, using loopback only.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::config::paths::Paths;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::base::Provider;
use biorouter::providers::githubcopilot::{
    GithubCopilotProvider, GITHUB_COPILOT_DEFAULT_MODEL, GITHUB_COPILOT_KNOWN_MODELS,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[ctor::ctor]
fn sandbox_global_config() {
    if Paths::path_root_override().is_some() {
        return;
    }
    let root = TempDir::new().expect("scratch Copilot configuration root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

struct Fixture {
    server: MockServer,
    cache: std::path::PathBuf,
    cache_before: Vec<u8>,
}

impl Fixture {
    async fn new(response: Value, status: u16) -> Self {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer synthetic-copilot-only"))
            .respond_with(ResponseTemplate::new(status).set_body_json(response))
            .mount(&server)
            .await;
        let cache = Paths::in_config_dir("githubcopilot/info.json");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let cache_before = serde_json::to_vec(&json!({
            "expires_at": "2099-01-01T00:00:00Z",
            "info": {
                "token": "synthetic-copilot-only",
                "expires_at": 4070908800_i64,
                "refresh_in": 3600,
                "endpoints": {"api": server.uri()}
            }
        }))
        .unwrap();
        std::fs::write(&cache, &cache_before).unwrap();
        Self {
            server,
            cache,
            cache_before,
        }
    }

    async fn provider(&self, model: &str) -> GithubCopilotProvider {
        GithubCopilotProvider::from_env(ModelConfig::new(model).unwrap())
            .await
            .unwrap()
    }

    async fn assert_only_catalogue_read(&self) {
        self.assert_catalogue_reads(1).await;
    }

    async fn assert_catalogue_reads(&self, expected: usize) {
        let requests = self.server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            expected,
            "actual authenticated catalogue GET count"
        );
        for request in requests {
            assert_eq!(request.method.as_str(), "GET");
            assert_eq!(request.url.path(), "/models");
        }
        assert_eq!(std::fs::read(&self.cache).unwrap(), self.cache_before);
    }
}

fn chat(id: &str) -> Value {
    json!({
        "id": id,
        "model_picker_enabled": true,
        "is_chat_default": false,
        "is_chat_fallback": false,
        "policy": {"state": "enabled"},
        "supported_endpoints": ["/chat/completions"],
        "capabilities": {
            "type": "chat",
            "supports": {"streaming": true, "tool_calls": true}
        }
    })
}

#[test]
fn copilot_refresh_declared_lts_fallback_retains_capability_metadata() {
    let metadata = GithubCopilotProvider::metadata();
    assert_eq!(GITHUB_COPILOT_DEFAULT_MODEL, "gpt-5.3-codex");
    assert_eq!(metadata.default_model, "gpt-5.3-codex");
    assert!(GITHUB_COPILOT_KNOWN_MODELS.contains(&GITHUB_COPILOT_DEFAULT_MODEL));
    for saved_model in ["gpt-5.6-terra", "claude-opus-4.8"] {
        assert!(metadata
            .known_models
            .iter()
            .any(|model| model.name == saved_model));
    }
    assert!(metadata.allows_unlisted_models);
}

#[test]
fn copilot_refresh_real_metadata_preserves_saved_vision_mime_without_api_prices() {
    let _guard = env_lock::lock_env([
        ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
        ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
    ]);
    let metadata = GithubCopilotProvider::metadata();
    for id in [
        "grok-4.7",
        "gemini-3.8-flash",
        "claude-opus-4.8",
        "claude-sonnet-5",
    ] {
        let model = metadata
            .known_models
            .iter()
            .find(|model| model.name == id)
            .unwrap_or_else(|| panic!("saved {id} needs real picker capability metadata"));
        assert_eq!(model.supports_vision, Some(true), "{id}");
        let mimes = model
            .supported_input_mime_types
            .as_ref()
            .expect("typed MIME metadata");
        if id == "grok-4.7" {
            assert_eq!(mimes, &["image/png", "image/jpeg", "image/jpg"], "{id}");
        } else {
            assert!(mimes.contains(&"image/webp".to_string()), "{id}");
            assert!(mimes.contains(&"image/gif".to_string()), "{id}");
        }
        assert_eq!(
            model.input_token_cost, None,
            "Copilot is not direct vendor billing"
        );
        assert_eq!(
            model.output_token_cost, None,
            "Copilot is not direct vendor billing"
        );
    }
}

#[tokio::test]
async fn copilot_refresh_account_default_wins_over_cli_daily_preference() {
    let _guard = SERIAL.lock().await;
    let mut server_default = chat("gpt-6-astra");
    server_default["is_chat_default"] = json!(true);
    let mut lts = chat("gpt-5.3-codex");
    lts["is_chat_fallback"] = json!(true);
    let fixture = Fixture::new(
        json!({"data": [lts, chat("claude-sonnet-5.5"), server_default]}),
        200,
    )
    .await;
    let provider = fixture.provider("gpt-5.6-terra").await;
    let models = provider.fetch_recommended_models().await.unwrap().unwrap();
    assert_eq!(
        models,
        ["gpt-6-astra", "gpt-5.3-codex", "claude-sonnet-5.5"]
    );
    assert_eq!(provider.get_model_config().model_name, "gpt-5.6-terra");
    fixture.assert_only_catalogue_read().await;
}

#[tokio::test]
async fn copilot_refresh_eligible_reported_sonnet_is_daily_preference_without_api_default() {
    let _guard = SERIAL.lock().await;
    let fixture = Fixture::new(
        json!({"data": [
            chat("gpt-5.3-codex"), chat("claude-opus-4.8"),
            chat("claude-sonnet-5.5"), chat("claude-sonnet-5.5")
        ]}),
        200,
    )
    .await;
    let provider = fixture.provider("claude-opus-4.8").await;
    assert_eq!(
        provider.fetch_recommended_models().await.unwrap().unwrap(),
        ["claude-sonnet-5.5", "gpt-5.3-codex", "claude-opus-4.8"]
    );
    assert_eq!(provider.get_model_config().model_name, "claude-opus-4.8");
    fixture.assert_only_catalogue_read().await;
}

#[tokio::test]
async fn copilot_refresh_filters_ineligible_and_incompatible_account_rows_without_canonical_loss() {
    let _guard = SERIAL.lock().await;
    let mut hidden = chat("hidden-model");
    hidden["model_picker_enabled"] = json!(false);
    hidden["is_chat_default"] = json!(true);
    let mut disabled = chat("claude-sonnet-5.5");
    disabled["policy"]["state"] = json!("disabled");
    let mut unconfigured = chat("claude-haiku-5.5");
    unconfigured["policy"]["state"] = json!("unconfigured");
    let mut embedding = chat("embedding-model");
    embedding["capabilities"]["type"] = json!("embeddings");
    let mut responses = chat("gpt-6.1-sol");
    responses["supported_endpoints"] = json!(["/responses"]);
    let mut messages = chat("claude-fable-5.2");
    messages["supported_endpoints"] = json!(["/v1/messages"]);
    let fixture = Fixture::new(
        json!({"data": [
            hidden, disabled, unconfigured, embedding, responses, messages,
            chat(""), chat("account-chat-fixture"), chat("gpt-5.3-codex"),
            chat("account-chat-fixture")
        ]}),
        200,
    )
    .await;
    assert_eq!(
        fixture
            .provider("gpt-5.3-codex")
            .await
            .fetch_recommended_models()
            .await
            .unwrap()
            .unwrap(),
        ["account-chat-fixture", "gpt-5.3-codex"]
    );
    fixture.assert_only_catalogue_read().await;
}

#[tokio::test]
async fn copilot_refresh_auto_only_catalogue_does_not_invent_named_availability() {
    let _guard = SERIAL.lock().await;
    let mut hidden_sonnet = chat("claude-sonnet-5.5");
    hidden_sonnet["model_picker_enabled"] = json!(false);
    let mut hidden_lts = chat("gpt-5.3-codex");
    hidden_lts["model_picker_enabled"] = json!(false);
    hidden_lts["is_chat_default"] = json!(true);
    let fixture = Fixture::new(json!({"data": [hidden_sonnet, hidden_lts]}), 200).await;
    assert_eq!(
        fixture
            .provider("gpt-5.3-codex")
            .await
            .fetch_recommended_models()
            .await
            .unwrap(),
        Some(Vec::new())
    );
    fixture.assert_only_catalogue_read().await;
}

#[tokio::test]
async fn copilot_refresh_legacy_string_catalogue_remains_compatible() {
    let _guard = SERIAL.lock().await;
    let fixture = Fixture::new(json!({"data": ["gpt-5.3-codex", "gpt-6-astra"]}), 200).await;
    assert_eq!(
        fixture
            .provider("gpt-5.3-codex")
            .await
            .fetch_supported_models()
            .await
            .unwrap()
            .unwrap(),
        ["gpt-5.3-codex", "gpt-6-astra"]
    );
    fixture.assert_only_catalogue_read().await;
}

#[tokio::test]
async fn copilot_refresh_http_error_body_cannot_advertise_an_account_model() {
    let _guard = SERIAL.lock().await;
    let fixture = Fixture::new(json!({"data": [chat("claude-sonnet-5.5")]}), 403).await;
    let provider = fixture.provider("gpt-5.3-codex").await;
    assert!(provider.fetch_supported_models().await.is_err());
    assert_eq!(
        provider.fetch_recommended_models().await.unwrap(),
        Some(vec!["gpt-5.3-codex".to_string()]),
        "unavailable discovery returns an explicit LTS suggestion without expanding capability metadata"
    );
    fixture.assert_catalogue_reads(2).await;
}

#[tokio::test]
async fn copilot_refresh_preserves_explicit_old_bindings_and_existing_stream_decisions() {
    let _guard = SERIAL.lock().await;
    let fixture = Fixture::new(json!({"data": []}), 200).await;
    for (id, streams) in [
        ("gpt-5.6-terra", true),
        ("gemini-3.5-flash", true),
        ("claude-opus-4.8", false),
        ("claude-sonnet-5.5", false),
    ] {
        let provider = fixture.provider(id).await;
        assert_eq!(provider.get_model_config().model_name, id);
        assert_eq!(provider.supports_streaming(), streams, "{id}");
    }
    assert!(fixture.server.received_requests().await.unwrap().is_empty());
    assert_eq!(std::fs::read(&fixture.cache).unwrap(), fixture.cache_before);
}

#[tokio::test]
async fn copilot_refresh_claude_55_keeps_nonzero_choice_tool_promotion_on_existing_endpoint() {
    let _guard = SERIAL.lock().await;
    let fixture = Fixture::new(json!({"data": []}), 200).await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "claude-sonnet-5.5",
            "choices": [
                {"index": 0, "message": {"role": "assistant", "content": "Working"}},
                {"index": 1, "message": {"role": "assistant", "content": null,
                    "tool_calls": [{"id": "native-call", "type": "function", "function": {
                        "name": "fixture_tool", "arguments": "{\"value\":7}"
                    }}]}}
            ],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
        })))
        .expect(1)
        .mount(&fixture.server)
        .await;
    let model = ModelConfig::new("claude-sonnet-5.5").unwrap();
    let provider = fixture.provider("claude-sonnet-5.5").await;
    assert!(!provider.supports_streaming());
    let (message, _) = provider
        .complete_with_model(
            &model,
            "fixture",
            &[Message::user().with_text("fixture")],
            &[],
        )
        .await
        .unwrap();
    let call = message
        .content
        .iter()
        .find_map(|content| content.as_tool_request())
        .expect("nonzero Claude choice must retain its tool request");
    assert_eq!(call.id, "native-call");
    assert_eq!(
        call.tool_call.as_ref().unwrap().name.as_ref(),
        "fixture_tool"
    );
    let requests = fixture.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let payload: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(payload["model"], "claude-sonnet-5.5");
    assert!(matches!(
        payload.get("stream"),
        None | Some(Value::Bool(false))
    ));
    assert_eq!(std::fs::read(&fixture.cache).unwrap(), fixture.cache_before);
}
