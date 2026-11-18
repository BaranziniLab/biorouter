biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::providers::base::Provider;
use biorouter::providers::openai::{
    OpenAiProvider, OPEN_AI_DEFAULT_FAST_MODEL, OPEN_AI_DEFAULT_MODEL,
};

#[test]
fn openai_model_refresh_offers_the_current_flagship_and_exact_new_models() {
    let metadata = OpenAiProvider::metadata();
    assert_eq!(OPEN_AI_DEFAULT_MODEL, "gpt-6.2-astra");
    assert_eq!(metadata.default_model, "gpt-6.2-astra");
    assert_eq!(metadata.known_models[0].name, "gpt-6.2-astra");
    assert_eq!(metadata.known_models[1].name, "gpt-6.2-sol");
    assert_eq!(OPEN_AI_DEFAULT_FAST_MODEL, "gpt-6-luna");

    for id in [
        "gpt-6.2-astra",
        "gpt-6.2-sol",
        "gpt-6.1-sol",
        "gpt-6.1-terra",
    ] {
        let model = metadata
            .known_models
            .iter()
            .find(|model| model.name == id)
            .unwrap_or_else(|| panic!("missing official model {id}"));
        assert_eq!(model.context_limit, 1_050_000, "{id}");
        assert_eq!(model.supports_vision, Some(true), "{id}");
    }
    for invented in ["gpt-6.2", "gpt-6.2-luna", "gpt-6-terra"] {
        assert!(metadata
            .known_models
            .iter()
            .all(|model| model.name != invented));
    }
}

#[cfg(unix)]
mod codex {
    use super::*;
    use biorouter::agents::effort::ReasoningEffort;
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    use biorouter::providers::codex::{CodexProvider, CODEX_DEFAULT_MODEL};
    use biorouter::providers::errors::ProviderError;
    use futures::StreamExt;
    use serde_json::{json, Value};
    use std::io::Write;
    use std::path::{Path, PathBuf};

    struct FakeCodex {
        home: tempfile::TempDir,
        command: PathBuf,
        requests: PathBuf,
    }

    impl FakeCodex {
        fn new(pages: Value, account: &str) -> Self {
            let home = tempfile::tempdir().unwrap();
            let command = home.path().join("codex-fixture");
            let requests = home.path().join("requests.jsonl");
            std::fs::write(home.path().join("auth.json"), r#"{"auth_mode":"chatgpt"}"#).unwrap();
            let script = format!(
                r#"#!/usr/bin/env python3
import json, sys
pages = json.loads({pages})
account = {account}
requests = {requests}
if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.169.0")
    sys.exit(0)
if sys.argv[1:] == ["login", "status"]:
    print("Logged in using ChatGPT")
    sys.exit(0)
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    with open(requests, "a") as output:
        output.write(json.dumps(message) + "\n")
    if "id" not in message:
        continue
    reply = {{"id": message["id"]}}
    if method == "initialize":
        reply["result"] = {{}}
    elif method == "account/read":
        reply["result"] = {{"account": {{"type": account}}}}
    elif method == "model/list":
        params = message["params"]
        assert params.get("includeHidden") is False
        page = pages[params.get("cursor") or "first"]
        if "error" in page:
            reply["error"] = page["error"]
        else:
            reply["result"] = page
    elif method == "thread/start":
        reply["result"] = {{"thread": {{"id": "fixture-thread"}}}}
    elif method == "turn/start":
        reply["result"] = {{"turn": {{"id": "fixture-turn"}}}}
    else:
        reply["error"] = {{"code": -32601, "message": "unexpected request"}}
    print(json.dumps(reply), flush=True)
    if method == "turn/start":
        print(json.dumps({{"method": "item/completed", "params": {{"item": {{
            "id": "fixture-message", "type": "agentMessage", "text": "fixture answer"
        }}}}}}), flush=True)
        print(json.dumps({{"method": "turn/completed", "params": {{
            "turn": {{"id": "fixture-turn", "status": "completed"}}
        }}}}), flush=True)
"#,
                pages = serde_json::to_string(&pages.to_string()).unwrap(),
                account = serde_json::to_string(account).unwrap(),
                requests = serde_json::to_string(&requests).unwrap(),
            );
            // Write in another process so parallel Linux tests cannot inherit a writable executable.
            let mut writer = std::process::Command::new("/bin/sh")
                .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
                .arg(&command)
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            writer
                .stdin
                .take()
                .unwrap()
                .write_all(script.as_bytes())
                .unwrap();
            assert!(writer.wait().unwrap().success());
            Self {
                home,
                command,
                requests,
            }
        }

        fn home(&self) -> &Path {
            self.home.path()
        }

        fn methods(&self) -> Vec<String> {
            std::fs::read_to_string(&self.requests)
                .unwrap_or_default()
                .lines()
                .map(|line| {
                    serde_json::from_str::<Value>(line).unwrap()["method"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect()
        }

        fn turn_effort(&self) -> Option<String> {
            std::fs::read_to_string(&self.requests)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|message| message["method"] == "turn/start")
                .unwrap()["params"]["effort"]
                .as_str()
                .map(str::to_string)
        }
    }

    #[tokio::test]
    async fn codex_model_refresh_uses_account_catalog_before_static_and_canonical_models() {
        let fixture = FakeCodex::new(
            json!({
                "first": {"data": [
                    {"id": "old", "model": "gpt-6-astra", "isDefault": false},
                    {"model": "hidden-model", "hidden": true, "isDefault": true},
                    {"model": "audio-only", "inputModalities": ["audio"]},
                    {"model": "", "isDefault": true}
                ], "nextCursor": "second"},
                "second": {"data": [
                    {"id": "new", "model": "account-only-default-fixture", "isDefault": true,
                     "inputModalities": ["text", "image"]},
                    {"model": "gpt-6.2-sol", "inputModalities": ["text", "image"]},
                    {"model": "gpt-6-astra"},
                    {"model": "gpt-6.1-terra", "inputModalities": ["text", "image"]}
                ], "nextCursor": null}
            }),
            "chatgpt",
        );
        let _guard = env_lock::lock_env([
            ("CODEX_HOME", fixture.home().to_str()),
            ("CODEX_COMMAND", fixture.command.to_str()),
            ("BIOROUTER_PATH_ROOT", fixture.home().to_str()),
        ]);
        let provider = CodexProvider::for_tests(fixture.command.clone(), "gpt-5.5");
        let models = provider.fetch_recommended_models().await.unwrap().unwrap();
        assert_eq!(
            models,
            [
                "account-only-default-fixture",
                "gpt-6-astra",
                "gpt-6.2-sol",
                "gpt-6.1-terra"
            ]
        );
        assert_eq!(provider.get_model_config().model_name, "gpt-5.5");
        assert_eq!(CodexProvider::metadata().default_model, CODEX_DEFAULT_MODEL);
        let methods = fixture.methods();
        assert_eq!(
            methods
                .iter()
                .filter(|method| *method == "model/list")
                .count(),
            2
        );
        assert!(!methods
            .iter()
            .any(|method| method == "thread/start" || method == "turn/start"));
    }

    #[tokio::test]
    async fn codex_model_refresh_falls_back_only_when_catalog_is_unavailable_or_empty() {
        for page in [
            json!({"data": [], "nextCursor": null}),
            json!({"error": {"code": -32601, "message": "method not found"}}),
            json!({"data": "malformed"}),
        ] {
            let fixture = FakeCodex::new(json!({"first": page}), "chatgpt");
            let _guard = env_lock::lock_env([
                ("CODEX_HOME", fixture.home().to_str()),
                ("CODEX_COMMAND", fixture.command.to_str()),
                ("BIOROUTER_PATH_ROOT", fixture.home().to_str()),
            ]);
            let provider = CodexProvider::for_tests(fixture.command.clone(), "gpt-5.5");
            let models = provider.fetch_recommended_models().await.unwrap().unwrap();
            let expected: Vec<_> = CodexProvider::metadata()
                .known_models
                .into_iter()
                .map(|model| model.name)
                .collect();
            assert_eq!(models, expected);
            assert_eq!(models.first().map(String::as_str), Some("gpt-6-astra"));
            assert!(fixture
                .methods()
                .iter()
                .any(|method| method == "model/list"));
        }
    }

    #[tokio::test]
    async fn codex_model_refresh_does_not_mask_an_explicit_metered_account() {
        let fixture = FakeCodex::new(json!({"first": {"data": []}}), "apiKey");
        let _guard = env_lock::lock_env([
            ("CODEX_HOME", fixture.home().to_str()),
            ("CODEX_COMMAND", fixture.command.to_str()),
            ("BIOROUTER_PATH_ROOT", fixture.home().to_str()),
        ]);
        let provider = CodexProvider::for_tests(fixture.command.clone(), "gpt-5.5");
        let result = provider.fetch_recommended_models().await;
        assert!(matches!(result, Err(ProviderError::Authentication(_))));
        assert!(!fixture
            .methods()
            .iter()
            .any(|method| method == "model/list"));
    }

    #[tokio::test]
    async fn codex_model_refresh_uses_only_the_runtime_effort_ladder_in_blocking_turns() {
        for (intent, expected) in [
            (None, Some("medium")),
            (Some(ReasoningEffort::Normal), Some("medium")),
            (Some(ReasoningEffort::Quick), Some("medium")),
            (Some(ReasoningEffort::Deep), Some("high")),
        ] {
            let fixture = FakeCodex::new(
                json!({"first": {"data": [{
                    "model": "account-model-fixture", "isDefault": true,
                    "defaultReasoningEffort": "medium",
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "ultra"},
                        {"reasoningEffort": "high"},
                        {"reasoningEffort": "medium"}
                    ]
                }]}}),
                "chatgpt",
            );
            let _guard = env_lock::lock_env([
                ("CODEX_HOME", fixture.home().to_str()),
                ("CODEX_COMMAND", fixture.command.to_str()),
                ("BIOROUTER_PATH_ROOT", fixture.home().to_str()),
            ]);
            let provider =
                CodexProvider::for_tests(fixture.command.clone(), "account-model-fixture");
            let model =
                ModelConfig::new_or_fail("account-model-fixture").with_reasoning_effort(intent);
            provider
                .complete_with_model(
                    &model,
                    "fixture system",
                    &[Message::user().with_text("fixture question")],
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(fixture.turn_effort().as_deref(), expected, "{intent:?}");
            assert_eq!(
                fixture
                    .methods()
                    .iter()
                    .filter(|method| *method == "model/list")
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn codex_model_refresh_streaming_uses_runtime_default_and_unknown_fallback_omits_effort()
    {
        for (page, expected) in [
            (
                json!({"data": [{"model": "account-model-fixture", "defaultReasoningEffort": "low",
                "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "medium"}]}]}),
                Some("low"),
            ),
            (
                json!({"error": {"code": -32601, "message": "unavailable"}}),
                None,
            ),
        ] {
            let fixture = FakeCodex::new(json!({"first": page}), "chatgpt");
            let _guard = env_lock::lock_env([
                ("CODEX_HOME", fixture.home().to_str()),
                ("CODEX_COMMAND", fixture.command.to_str()),
                ("BIOROUTER_PATH_ROOT", fixture.home().to_str()),
            ]);
            let provider =
                CodexProvider::for_tests(fixture.command.clone(), "account-model-fixture");
            let stream = provider
                .stream(
                    "fixture system",
                    &[Message::user().with_text("fixture question")],
                    &[],
                )
                .await
                .unwrap();
            let events: Vec<_> = stream.collect().await;
            assert!(events.iter().all(Result::is_ok));
            assert_eq!(fixture.turn_effort().as_deref(), expected);
            assert_eq!(
                fixture
                    .methods()
                    .iter()
                    .filter(|method| *method == "model/list")
                    .count(),
                1
            );
        }
    }
}
