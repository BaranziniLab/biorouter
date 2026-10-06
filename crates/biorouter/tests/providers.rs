//! Live provider calls require an exact named test with `--ignored --exact`.
//! Configured credentials alone never opt an ordinary regression run into network calls.

use anyhow::Result;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::providers::anthropic::ANTHROPIC_DEFAULT_MODEL;
use biorouter::providers::azure::AZURE_DEFAULT_MODEL;
use biorouter::providers::base::Provider;
use biorouter::providers::bedrock::BEDROCK_DEFAULT_MODEL;
use biorouter::providers::create_with_named_model;
use biorouter::providers::databricks::DATABRICKS_DEFAULT_MODEL;
use biorouter::providers::errors::ProviderError;
use biorouter::providers::google::GOOGLE_DEFAULT_MODEL;
use biorouter::providers::litellm::LITELLM_DEFAULT_MODEL;
use biorouter::providers::ollama::OLLAMA_DEFAULT_MODEL;
use biorouter::providers::openai::OPEN_AI_DEFAULT_MODEL;
use biorouter::providers::sagemaker_tgi::SAGEMAKER_TGI_DEFAULT_MODEL;
use biorouter::providers::snowflake::SNOWFLAKE_DEFAULT_MODEL;
use biorouter::providers::versa_bedrock::VERSA_BEDROCK_DEFAULT_MODEL;
use biorouter::providers::xai::XAI_DEFAULT_MODEL;
use biorouter::providers::xiaomi_mimo::XIAOMI_MIMO_DEFAULT_MODEL;
use biorouter::providers::zai::ZAI_DEFAULT_MODEL;
use dotenvy::dotenv;
use rmcp::model::{AnnotateAble, Content, RawImageContent};
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::object;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
enum TestStatus {
    Passed,
    Skipped,
    Failed,
}

impl std::fmt::Display for TestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestStatus::Passed => write!(f, "✅"),
            TestStatus::Skipped => write!(f, "⏭️"),
            TestStatus::Failed => write!(f, "❌"),
        }
    }
}

struct TestReport {
    results: Mutex<HashMap<String, TestStatus>>,
}

impl TestReport {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(HashMap::new()),
        })
    }

    fn record_status(&self, provider: &str, status: TestStatus) {
        let mut results = self.results.lock().unwrap();
        results.insert(provider.to_string(), status);
    }

    fn record_pass(&self, provider: &str) {
        self.record_status(provider, TestStatus::Passed);
    }

    fn record_skip(&self, provider: &str) {
        self.record_status(provider, TestStatus::Skipped);
    }

    fn record_fail(&self, provider: &str) {
        self.record_status(provider, TestStatus::Failed);
    }

    fn print_summary(&self) {
        println!("\n============== Providers ==============");
        let results = self.results.lock().unwrap();
        let mut providers: Vec<_> = results.iter().collect();
        providers.sort_by(|a, b| a.0.cmp(b.0));

        for (provider, status) in providers {
            println!("{} {}", status, provider);
        }
        println!("=======================================\n");
    }
}

lazy_static::lazy_static! {
    static ref TEST_REPORT: Arc<TestReport> = TestReport::new();
    static ref ENV_LOCK: Mutex<()> = Mutex::new(());
}

struct ProviderTester {
    provider: Arc<dyn Provider>,
    name: String,
}

impl ProviderTester {
    fn new(provider: Arc<dyn Provider>, name: String) -> Self {
        Self { provider, name }
    }

    async fn test_basic_response(&self) -> Result<()> {
        let message = Message::user().with_text("Just say hello!");

        let (response, _) = self
            .provider
            .complete("You are a helpful assistant.", &[message], &[])
            .await?;

        assert_eq!(
            response.content.len(),
            1,
            "Expected single content item in response"
        );

        assert!(
            matches!(response.content[0], MessageContent::Text(_)),
            "Expected text response"
        );

        Ok(())
    }

    async fn test_tool_usage(&self) -> Result<()> {
        let weather_tool = Tool::new(
            "get_weather",
            "Get the weather for a location",
            object!({
                "type": "object",
                "required": ["location"],
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "The city and state, e.g. San Francisco, CA"
                    }
                }
            }),
        );

        let message = Message::user().with_text("What's the weather like in San Francisco?");

        let (response1, _) = self
            .provider
            .complete(
                "You are a helpful weather assistant.",
                std::slice::from_ref(&message),
                std::slice::from_ref(&weather_tool),
            )
            .await?;

        println!("=== {}::reponse1 ===", self.name);
        dbg!(&response1);
        println!("===================");

        assert!(
            response1
                .content
                .iter()
                .any(|content| matches!(content, MessageContent::ToolRequest(_))),
            "Expected tool request in response"
        );

        let id = &response1
            .content
            .iter()
            .filter_map(|message| message.as_tool_request())
            .next_back()
            .expect("got tool request")
            .id;

        let weather = Message::user().with_tool_response(
            id,
            Ok(rmcp::model::CallToolResult {
                content: vec![Content::text(
                    "
                  50°F°C
                  Precipitation: 0%
                  Humidity: 84%
                  Wind: 2 mph
                  Weather
                  Saturday 9:00 PM
                  Clear",
                )],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let (response2, _) = self
            .provider
            .complete(
                "You are a helpful weather assistant.",
                &[message, response1, weather],
                &[weather_tool],
            )
            .await?;

        println!("=== {}::reponse2 ===", self.name);
        dbg!(&response2);
        println!("===================");

        assert!(
            response2
                .content
                .iter()
                .any(|content| matches!(content, MessageContent::Text(_))),
            "Expected text for final response"
        );

        Ok(())
    }

    async fn test_context_length_exceeded_error(&self) -> Result<()> {
        // Ollama and Xiaomi MiMo silently truncate oversized input to their
        // context window (MiMo caps at its ~1M window and returns Ok) rather
        // than returning a context-length error. They are asserted on
        // separately below, and so must still send the request.
        let truncates_silently =
            matches!(self.name.to_lowercase().as_str(), "ollama" | "xiaomi_mimo");

        let large_message_content = if self.name.to_lowercase() == "google" {
            "hello ".repeat(1_300_000)
        } else {
            "hello ".repeat(300_000)
        };

        // An over-limit request is only worth sending when the fixture can
        // actually overflow the window. Against a million-token model it
        // cannot: the request would not error, it would submit a very large
        // *billable* prompt and then fail the assertion below. This used to be
        // a hardcoded `name == "anthropic"` carve-out; the window is the real
        // reason, and stating it that way covers every large-context model
        // rather than the one that happened to be noticed. Each provider's
        // error-payload mapping is covered deterministically in unit tests, and
        // this suite still exercises it live through the basic, tool and image
        // requests.
        //
        // `len() / 4` OVERESTIMATES tokens for this fixture — it is repeated
        // "hello ", roughly 6 chars per token — which is the direction that
        // makes skipping safe: if even the overestimate fits inside the window,
        // the real prompt certainly does.
        let window = self.provider.get_model_config().context_limit();
        let upper_bound_tokens = large_message_content.len() / 4;
        if !truncates_silently && upper_bound_tokens <= window {
            println!(
                "Skipping {} live over-limit request: ~{} tokens cannot exceed a {}-token \
                 window, so the call would be billed without testing anything",
                self.name, upper_bound_tokens, window
            );
            return Ok(());
        }

        let messages = vec![
            Message::user().with_text("hi there. what is 2 + 2?"),
            Message::assistant().with_text("hey! I think it's 4."),
            Message::user().with_text(&large_message_content),
            Message::assistant().with_text("heyy!!"),
            Message::user().with_text("what's the meaning of life?"),
            Message::assistant().with_text("the meaning of life is 42"),
            Message::user().with_text(
                "did I ask you what's 2+2 in this message history? just respond with 'yes' or 'no'",
            ),
        ];

        let result = self
            .provider
            .complete("You are a helpful assistant.", &messages, &[])
            .await;

        println!("=== {}::context_length_exceeded_error ===", self.name);
        dbg!(&result);
        println!("===================");

        if truncates_silently {
            assert!(
                result.is_ok(),
                "Expected to succeed because of default truncation"
            );
            return Ok(());
        }

        assert!(
            result.is_err(),
            "Expected error when context window is exceeded"
        );
        assert!(
            matches!(result.unwrap_err(), ProviderError::ContextLengthExceeded(_)),
            "Expected error to be ContextLengthExceeded"
        );

        Ok(())
    }

    async fn test_image_content_support(&self) -> Result<()> {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use biorouter::conversation::message::Message;
        use std::fs;

        let image_path = "crates/biorouter/examples/test_assets/test_image.png";
        let image_data = match fs::read(image_path) {
            Ok(data) => data,
            Err(_) => {
                println!(
                    "Test image not found at {}, skipping image test",
                    image_path
                );
                return Ok(());
            }
        };

        let base64_image = BASE64.encode(image_data);
        let image_content = RawImageContent {
            data: base64_image,
            mime_type: "image/png".to_string(),
            meta: None,
        }
        .no_annotation();

        let message_with_image =
            Message::user().with_image(image_content.data.clone(), image_content.mime_type.clone());

        let result = self
            .provider
            .complete(
                "You are a helpful assistant. Describe what you see in the image briefly.",
                &[message_with_image],
                &[],
            )
            .await;

        println!("=== {}::image_content_support ===", self.name);
        let (response, _) = result?;
        println!("Image response: {:?}", response);
        assert!(
            response
                .content
                .iter()
                .any(|content| matches!(content, MessageContent::Text(_))),
            "Expected text response for image"
        );
        println!("===================");

        let screenshot_tool = Tool::new(
            "get_screenshot",
            "Get a screenshot of the current screen",
            object!({
                "type": "object",
                "properties": {}
            }),
        );

        let user_message = Message::user().with_text("Take a screenshot please");
        let tool_request = Message::assistant().with_tool_request(
            "test_id",
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: "get_screenshot".into(),
                arguments: Some(object!({})),
            }),
        );
        let tool_response = Message::user().with_tool_response(
            "test_id",
            Ok(rmcp::model::CallToolResult {
                content: vec![Content::image(
                    image_content.data.clone(),
                    image_content.mime_type.clone(),
                )],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let result2 = self
            .provider
            .complete(
                "You are a helpful assistant.",
                &[user_message, tool_request, tool_response],
                &[screenshot_tool],
            )
            .await;

        println!("=== {}::tool_image_response ===", self.name);
        let (response, _) = result2?;
        println!("Tool image response: {:?}", response);
        println!("===================");

        Ok(())
    }

    async fn run_test_suite(&self) -> Result<()> {
        self.test_basic_response().await?;
        self.test_tool_usage().await?;
        self.test_context_length_exceeded_error().await?;
        self.test_image_content_support().await?;
        Ok(())
    }
}

fn load_env() {
    if let Ok(path) = dotenv() {
        println!("Loaded environment from {:?}", path);
    }
}

/// The broad, credential-**optional** sweep: a provider with no credentials
/// configured records ⏭️ and returns `Ok`, so the suite can be run by anyone
/// with any subset of keys.
///
/// `name` is the provider's **registry key** — `metadata().name`, the exact
/// string [`create_with_named_model`] looks up. It is used verbatim, and it is
/// also the label in the report. It used to be a display name that was
/// `.to_lowercase()`d into a lookup, which silently produced a key that does
/// not exist for three providers ("Bedrock" → `bedrock`, but the registry holds
/// `aws_bedrock`); the resulting "Unknown provider" was then swallowed as one
/// more skip, so the test reported green having called nothing.
/// [`every_registry_key_used_by_this_suite_resolves`] is the guard that keeps
/// that from coming back, and it needs no credentials to catch it.
///
/// Tests that must not be allowed to pass without calling anything use
/// [`run_live_suite`] instead.
async fn test_provider(
    name: &str,
    model_name: &str,
    required_vars: &[&str],
    env_modifications: Option<HashMap<&str, Option<String>>>,
) -> Result<()> {
    TEST_REPORT.record_fail(name);

    let original_env = {
        let _lock = ENV_LOCK.lock().unwrap();

        load_env();

        let mut original_env = HashMap::new();
        for &var in required_vars {
            if let Ok(val) = std::env::var(var) {
                original_env.insert(var, val);
            }
        }
        if let Some(mods) = &env_modifications {
            for &var in mods.keys() {
                if let Ok(val) = std::env::var(var) {
                    original_env.insert(var, val);
                }
            }
        }

        if let Some(mods) = &env_modifications {
            for (&var, value) in mods.iter() {
                match value {
                    Some(val) => std::env::set_var(var, val),
                    None => std::env::remove_var(var),
                }
            }
        }

        let missing_vars = required_vars.iter().any(|var| std::env::var(var).is_err());
        if missing_vars {
            println!("Skipping {} tests - credentials not configured", name);
            TEST_REPORT.record_skip(name);
            return Ok(());
        }

        original_env
    };

    let provider = match create_with_named_model(name, model_name).await {
        Ok(p) => p,
        Err(e) => {
            println!("Skipping {} tests - failed to create provider: {}", name, e);
            TEST_REPORT.record_skip(name);
            return Ok(());
        }
    };

    {
        let _lock = ENV_LOCK.lock().unwrap();
        for (&var, value) in original_env.iter() {
            std::env::set_var(var, value);
        }
        if let Some(mods) = env_modifications {
            for &var in mods.keys() {
                if !original_env.contains_key(var) {
                    std::env::remove_var(var);
                }
            }
        }
    }

    let tester = ProviderTester::new(provider, name.to_string());
    match tester.run_test_suite().await {
        Ok(_) => {
            TEST_REPORT.record_pass(name);
            Ok(())
        }
        Err(e) => {
            println!("{} test failed: {}", name, e);
            TEST_REPORT.record_fail(name);
            Err(e)
        }
    }
}

#[test]
fn live_provider_tests_require_explicit_opt_in() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--list", "--ignored"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    let ignored: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    for name in [
        "test_openai_provider",
        "test_azure_provider",
        "test_bedrock_provider_long_term_credentials",
        "test_bedrock_provider_aws_profile_credentials",
        "test_bedrock_provider_sonnet_5",
        "test_versa_bedrock_provider",
        "test_versa_bedrock_provider_sonnet_4_6",
        "test_versa_bedrock_provider_sonnet_5",
        "test_versa_bedrock_refuses_models_ucsf_does_not_serve",
        "test_versa_azure_every_offered_model",
        "test_versa_azure_reasoning_effort_tool_loops",
        "test_versa_bedrock_every_offered_model",
        "test_versa_removed_models_are_refused",
        "test_databricks_provider",
        "test_ollama_provider",
        "test_anthropic_provider",
        "test_openrouter_provider",
        "test_google_provider",
        "test_snowflake_provider",
        "test_sagemaker_tgi_provider",
        "test_litellm_provider",
        "test_xai_provider",
        "test_zai_provider",
        "test_xiaomi_mimo_provider",
    ] {
        assert!(
            ignored.contains(&name),
            "live test must require opt-in: {name}"
        );
    }
    assert!(!ignored.contains(&"every_registry_key_used_by_this_suite_resolves"));
    assert!(!ignored.contains(&"live_provider_tests_require_explicit_opt_in"));
}

#[tokio::test]
#[ignore = "live OpenAI call; run an exact named test with --ignored --exact"]
async fn test_openai_provider() -> Result<()> {
    test_provider("openai", OPEN_AI_DEFAULT_MODEL, &["OPENAI_API_KEY"], None).await
}

#[tokio::test]
#[ignore = "live Azure call; run an exact named test with --ignored --exact"]
async fn test_azure_provider() -> Result<()> {
    test_provider(
        "azure_openai",
        AZURE_DEFAULT_MODEL,
        &[
            "AZURE_OPENAI_API_KEY",
            "AZURE_OPENAI_ENDPOINT",
            "AZURE_OPENAI_DEPLOYMENT_NAME",
        ],
        None,
    )
    .await
}

// ===========================================================================
// Live Bedrock / Versa Bedrock checks
//
// These make REAL, BILLED API calls, so they are `#[ignore]`d and must be asked
// for by name. In exchange for being opt-in they are held to a stricter rule
// than the sweep above: **no early exit may return `Ok`**. Every one of them is
// an `Err`, because the state being replaced was a pair of tests that reported
// green while calling nothing at all — a missing credential returned `Ok`, and
// so did an "Unknown provider" from a registry key that never existed.
//
//     cargo test -p biorouter --test providers test_versa_bedrock_provider -- --ignored --exact --test-threads=1
// ===========================================================================

/// How a live test establishes that a credential is actually available.
#[derive(Clone, Copy)]
enum Credential {
    /// Present only if the process environment carries it.
    Env(&'static str),
    /// Present if the environment carries it **or** Biorouter's secret store
    /// does. Versa Bedrock resolves its keys through `Config::get_secret`,
    /// which reads the environment first and the OS keychain second — so on a
    /// machine where the UCSF keys live in the keychain (the normal desktop
    /// install) there is no environment variable to find, and an env-only check
    /// would hard-fail the exact configuration this test exists to cover.
    EnvOrSecret(&'static str),
}

impl Credential {
    fn key(self) -> &'static str {
        match self {
            Credential::Env(key) | Credential::EnvOrSecret(key) => key,
        }
    }

    fn is_present(self) -> bool {
        match self {
            Credential::Env(key) => std::env::var(key).is_ok(),
            Credential::EnvOrSecret(key) => {
                std::env::var(key).is_ok()
                    || biorouter::config::Config::global()
                        .get_secret::<String>(key)
                        .is_ok()
            }
        }
    }
}

/// Run the full provider suite live, failing loudly instead of skipping.
///
/// `registry_key` is `metadata().name` and is passed to the factory verbatim.
/// `label` is only the report row, so two tests of the same provider do not
/// overwrite each other's status.
async fn run_live_suite(
    label: &str,
    registry_key: &str,
    model_name: &str,
    required: &[Credential],
    env_modifications: Option<HashMap<&str, Option<String>>>,
) -> Result<()> {
    TEST_REPORT.record_fail(label);

    let original_env = {
        let _lock = ENV_LOCK.lock().unwrap();

        load_env();

        let mut original_env = HashMap::new();
        for credential in required {
            if let Ok(val) = std::env::var(credential.key()) {
                original_env.insert(credential.key(), val);
            }
        }
        if let Some(mods) = &env_modifications {
            for &var in mods.keys() {
                if let Ok(val) = std::env::var(var) {
                    original_env.insert(var, val);
                }
            }
        }

        if let Some(mods) = &env_modifications {
            for (&var, value) in mods.iter() {
                match value {
                    Some(val) => std::env::set_var(var, val),
                    None => std::env::remove_var(var),
                }
            }
        }

        original_env
    };

    // The environment modifications above are part of what is under test (the
    // AWS_PROFILE variant works by *removing* the long-term keys), so they stay
    // applied across the whole run and are restored exactly once, on every
    // path, before the result is inspected.
    let outcome = live_suite_inner(registry_key, model_name, required).await;

    {
        let _lock = ENV_LOCK.lock().unwrap();
        for (&var, value) in original_env.iter() {
            std::env::set_var(var, value);
        }
        if let Some(mods) = env_modifications {
            for &var in mods.keys() {
                if !original_env.contains_key(var) {
                    std::env::remove_var(var);
                }
            }
        }
    }

    match outcome {
        Ok(()) => {
            TEST_REPORT.record_pass(label);
            Ok(())
        }
        Err(e) => {
            println!("{} live test failed: {:#}", label, e);
            TEST_REPORT.record_fail(label);
            Err(e)
        }
    }
}

async fn live_suite_inner(
    registry_key: &str,
    model_name: &str,
    required: &[Credential],
) -> Result<()> {
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|credential| !credential.is_present())
        .map(Credential::key)
        .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "{} live test requires credentials that are not configured: {}. \
             This test is #[ignore]d, so reaching it means it was asked for by \
             name — it fails rather than skipping, because a skipped live test \
             that reports success is worse than no test.",
            registry_key,
            missing.join(", ")
        );
    }

    let provider = create_with_named_model(registry_key, model_name)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to construct provider {:?} with model {:?}: {}. A registry-key \
                 typo surfaces here as \"Unknown provider\"; the valid keys are asserted \
                 by every_registry_key_used_by_this_suite_resolves.",
                registry_key,
                model_name,
                e
            )
        })?;

    ProviderTester::new(provider, registry_key.to_string())
        .run_test_suite()
        .await
}

/// Every registry key this file passes to the factory must actually exist.
///
/// This is the cheap instrument the live tests lacked: no credentials, no
/// network, no billing, and it runs in the ordinary `cargo test` sweep. The
/// defect it pins is that `create_with_named_model("bedrock", …)` fails with
/// "Unknown provider: bedrock" — the registry key is `aws_bedrock` — and that
/// failure was being swallowed as a skip, so the test stayed green. Azure
/// (`azure_openai`) and SageMaker TGI (`sagemaker_tgi`) were broken the same
/// way and just as invisibly.
#[tokio::test]
async fn every_registry_key_used_by_this_suite_resolves() {
    const KEYS_USED: &[&str] = &[
        "anthropic",
        "aws_bedrock",
        "azure_openai",
        "databricks",
        "google",
        "litellm",
        "ollama",
        "openai",
        "openrouter",
        "sagemaker_tgi",
        "snowflake",
        "versa_bedrock",
        "xai",
        "xiaomi_mimo",
        "zai",
    ];

    let registered: Vec<String> = biorouter::providers::providers()
        .await
        .into_iter()
        .map(|(metadata, _)| metadata.name)
        .collect();

    let unknown: Vec<&str> = KEYS_USED
        .iter()
        .copied()
        .filter(|key| !registered.iter().any(|name| name == key))
        .collect();

    assert!(
        unknown.is_empty(),
        "these tests look up provider keys that the registry does not hold: {:?}.\n\
         A lookup miss is reported as a skip, so the affected tests pass without \
         calling anything. Registered keys: {:?}",
        unknown,
        registered
    );
}

#[tokio::test]
#[ignore = "live billed Bedrock call; run deliberately with --ignored"]
async fn test_bedrock_provider_long_term_credentials() -> Result<()> {
    run_live_suite(
        "aws_bedrock (long-term keys)",
        "aws_bedrock",
        BEDROCK_DEFAULT_MODEL,
        &[
            Credential::Env("AWS_ACCESS_KEY_ID"),
            Credential::Env("AWS_SECRET_ACCESS_KEY"),
        ],
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "live billed Bedrock call; run deliberately with --ignored"]
async fn test_bedrock_provider_aws_profile_credentials() -> Result<()> {
    // Removing the long-term keys is the point: without this the SDK would
    // satisfy the request from them and the profile path would go untested.
    let env_mods =
        HashMap::from_iter([("AWS_ACCESS_KEY_ID", None), ("AWS_SECRET_ACCESS_KEY", None)]);

    run_live_suite(
        "aws_bedrock (AWS_PROFILE)",
        "aws_bedrock",
        BEDROCK_DEFAULT_MODEL,
        &[Credential::Env("AWS_PROFILE")],
        Some(env_mods),
    )
    .await
}

/// The model in the issue #87 reports.
///
/// The two tests above pin [`BEDROCK_DEFAULT_MODEL`], which has never been
/// `claude-sonnet-5`, so no live test reached the model actually being reported
/// on. Pinned literally rather than to the default constant, so that moving the
/// default cannot quietly stop covering it.
#[tokio::test]
#[ignore = "live billed Bedrock call; run deliberately with --ignored"]
async fn test_bedrock_provider_sonnet_5() -> Result<()> {
    run_live_suite(
        "aws_bedrock (sonnet-5)",
        "aws_bedrock",
        "us.anthropic.claude-sonnet-5",
        &[
            Credential::Env("AWS_ACCESS_KEY_ID"),
            Credential::Env("AWS_SECRET_ACCESS_KEY"),
        ],
        None,
    )
    .await
}

/// Versa Bedrock — the UCSF MuleSoft proxy — had no live test at all, despite
/// being the path a UCSF `config.yaml` actually routes to. Its credentials are
/// normally in the OS keychain rather than the environment, hence
/// [`Credential::EnvOrSecret`].
#[tokio::test]
#[ignore = "live billed Versa Bedrock call; run deliberately with --ignored"]
async fn test_versa_bedrock_provider() -> Result<()> {
    run_live_suite(
        "versa_bedrock (default model)",
        "versa_bedrock",
        VERSA_BEDROCK_DEFAULT_MODEL,
        &[
            Credential::EnvOrSecret("VERSA_BEDROCK_ACCESS_KEY_ID"),
            Credential::EnvOrSecret("VERSA_BEDROCK_SECRET_ACCESS_KEY"),
        ],
        None,
    )
    .await
}

/// Sonnet 4.6 over the Versa proxy. Not in `VERSA_BEDROCK_KNOWN_MODELS`' first
/// position and not the default, so nothing else exercises it; UCSF entitlement
/// is per-account, so a failure here is a real signal about this account rather
/// than about the client.
#[tokio::test]
#[ignore = "live billed Versa Bedrock call; run deliberately with --ignored"]
async fn test_versa_bedrock_provider_sonnet_4_6() -> Result<()> {
    run_live_suite(
        "versa_bedrock (sonnet-4-6)",
        "versa_bedrock",
        "us.anthropic.claude-sonnet-4-6",
        &[
            Credential::EnvOrSecret("VERSA_BEDROCK_ACCESS_KEY_ID"),
            Credential::EnvOrSecret("VERSA_BEDROCK_SECRET_ACCESS_KEY"),
        ],
        None,
    )
    .await
}

/// Sonnet 5 over the Versa proxy: the id `VERSA_BEDROCK_KNOWN_MODELS` gained on
/// 2026-10-05, when it first answered through UCSF's gateway, and the only
/// Claude 5 model UCSF's account may invoke. Pinned literally, like the test
/// above, so a change to the list cannot quietly stop covering it.
#[tokio::test]
#[ignore = "live billed Versa Bedrock call; run deliberately with --ignored"]
async fn test_versa_bedrock_provider_sonnet_5() -> Result<()> {
    run_live_suite(
        "versa_bedrock (sonnet-5)",
        "versa_bedrock",
        "us.anthropic.claude-sonnet-5",
        &[
            Credential::EnvOrSecret("VERSA_BEDROCK_ACCESS_KEY_ID"),
            Credential::EnvOrSecret("VERSA_BEDROCK_SECRET_ACCESS_KEY"),
        ],
        None,
    )
    .await
}

/// The other half of the 2026-10-05 measurement: Opus 5.5 and Opus 5 left
/// `VERSA_BEDROCK_KNOWN_MODELS` because UCSF's account is refused them
/// (`AccessDeniedException`), and a chat still bound to one must hear that the
/// MODEL was refused, not that its working key pair was. If this starts
/// failing because a model answered, UCSF has enabled it: add it to the list.
#[tokio::test]
#[ignore = "live Versa Bedrock call; run deliberately with --ignored"]
async fn test_versa_bedrock_refuses_models_ucsf_does_not_serve() -> Result<()> {
    use biorouter::providers::errors::ProviderErrorKind;

    let _lock = ENV_LOCK.lock().unwrap();
    load_env();
    for credential in [
        Credential::EnvOrSecret("VERSA_BEDROCK_ACCESS_KEY_ID"),
        Credential::EnvOrSecret("VERSA_BEDROCK_SECRET_ACCESS_KEY"),
    ] {
        anyhow::ensure!(
            credential.is_present(),
            "{} is not configured; this live test fails rather than skipping",
            credential.key()
        );
    }
    drop(_lock);

    for model in ["us.anthropic.claude-opus-5-5", "us.anthropic.claude-opus-5"] {
        let provider = create_with_named_model("versa_bedrock", model).await?;
        let error = match provider
            .complete(
                "You are a test.",
                &[Message::user().with_text("Reply with the single word OK.")],
                &[],
            )
            .await
        {
            Ok(_) => anyhow::bail!("{model} answered through Versa: UCSF now serves it"),
            Err(error) => error,
        };
        let text = error.to_string();
        anyhow::ensure!(
            error.kind() == ProviderErrorKind::ModelUnavailable
                && text.contains(&format!("does not serve model `{model}`"))
                && text.contains("Your key pair works"),
            "{model}: expected a model refusal, got {text}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "live Databricks call; run an exact named test with --ignored --exact"]
async fn test_databricks_provider() -> Result<()> {
    test_provider(
        "databricks",
        DATABRICKS_DEFAULT_MODEL,
        &["DATABRICKS_HOST", "DATABRICKS_TOKEN"],
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "live Ollama call; run an exact named test with --ignored --exact"]
async fn test_ollama_provider() -> Result<()> {
    test_provider("ollama", OLLAMA_DEFAULT_MODEL, &["OLLAMA_HOST"], None).await
}

#[tokio::test]
#[ignore = "live Anthropic call; run an exact named test with --ignored --exact"]
async fn test_anthropic_provider() -> Result<()> {
    test_provider(
        "anthropic",
        ANTHROPIC_DEFAULT_MODEL,
        &["ANTHROPIC_API_KEY"],
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "live OpenRouter call; run an exact named test with --ignored --exact"]
async fn test_openrouter_provider() -> Result<()> {
    test_provider(
        "openrouter",
        OPEN_AI_DEFAULT_MODEL,
        &["OPENROUTER_API_KEY"],
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "live Google call; run an exact named test with --ignored --exact"]
async fn test_google_provider() -> Result<()> {
    test_provider("google", GOOGLE_DEFAULT_MODEL, &["GOOGLE_API_KEY"], None).await
}

#[tokio::test]
#[ignore = "live Snowflake call; run an exact named test with --ignored --exact"]
async fn test_snowflake_provider() -> Result<()> {
    test_provider(
        "snowflake",
        SNOWFLAKE_DEFAULT_MODEL,
        &["SNOWFLAKE_HOST", "SNOWFLAKE_TOKEN"],
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "live SageMaker call; run an exact named test with --ignored --exact"]
async fn test_sagemaker_tgi_provider() -> Result<()> {
    test_provider(
        "sagemaker_tgi",
        SAGEMAKER_TGI_DEFAULT_MODEL,
        &["SAGEMAKER_ENDPOINT_NAME"],
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "live LiteLLM call; run an exact named test with --ignored --exact"]
async fn test_litellm_provider() -> Result<()> {
    if std::env::var("LITELLM_HOST").is_err() {
        println!("LITELLM_HOST not set, skipping test");
        TEST_REPORT.record_skip("litellm");
        return Ok(());
    }

    let env_mods = HashMap::from_iter([
        ("LITELLM_HOST", Some("http://localhost:4000".to_string())),
        ("LITELLM_API_KEY", Some("".to_string())),
    ]);

    test_provider("litellm", LITELLM_DEFAULT_MODEL, &[], Some(env_mods)).await
}

#[tokio::test]
#[ignore = "live xAI call; run an exact named test with --ignored --exact"]
async fn test_xai_provider() -> Result<()> {
    test_provider("xai", XAI_DEFAULT_MODEL, &["XAI_API_KEY"], None).await
}

#[tokio::test]
#[ignore = "live Zai call; run an exact named test with --ignored --exact"]
async fn test_zai_provider() -> Result<()> {
    test_provider("zai", ZAI_DEFAULT_MODEL, &["ZAI_API_KEY"], None).await
}

#[tokio::test]
#[ignore = "live Xiaomi MiMo call; run an exact named test with --ignored --exact"]
async fn test_xiaomi_mimo_provider() -> Result<()> {
    test_provider(
        "xiaomi_mimo",
        XIAOMI_MIMO_DEFAULT_MODEL,
        &["XIAOMI_MIMO_API_KEY"],
        None,
    )
    .await
}

// ===========================================================================
// Live Versa catalog sweep
//
// Every model the two Versa providers OFFER, driven through BioRouter's own
// provider code rather than curl. `run_live_suite` above covers the blocking
// path only, and only for the models a test names; these cover each row of
// `VERSA_AZURE_DEPLOYMENTS` and `VERSA_BEDROCK_KNOWN_MODELS` on both paths,
// with a full tool loop: the model calls the tool, the result goes back, and
// the model answers from it. A loop is what breaks when a replayed
// `function_call` / `function_call_output` (Responses) or `toolUse` /
// `toolResult` (Converse) is malformed, and a single tool-call turn never
// replays anything.
//
// Each sweep runs every model and reports one row per model before failing,
// so one refused model does not hide the state of the others.
//
// First run 2026-10-05: all 11 Versa Azure rows and all 6 Versa Bedrock rows
// passed every check, the three gpt-5.6 rows on the Responses route under
// Deep and Quick effort as well (the gateway echoed `reasoning.effort` high
// and low back, and Quick's temperature was not sent).
//
//     cargo test -p biorouter --test providers versa_ -- --ignored --test-threads=1 --nocapture
// ===========================================================================

const VERSA_AZURE_CREDENTIALS: &[Credential] = &[Credential::EnvOrSecret("VERSA_AZURE_API_KEY")];
const VERSA_BEDROCK_CREDENTIALS: &[Credential] = &[
    Credential::EnvOrSecret("VERSA_BEDROCK_ACCESS_KEY_ID"),
    Credential::EnvOrSecret("VERSA_BEDROCK_SECRET_ACCESS_KEY"),
];

/// Fail, never skip, when a sweep's credentials are absent: like
/// [`run_live_suite`], reaching an `#[ignore]`d test means it was asked for.
fn require_versa_credentials(required: &[Credential]) -> Result<()> {
    let _lock = ENV_LOCK.lock().unwrap();
    load_env();
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|credential| !credential.is_present())
        .map(Credential::key)
        .collect();
    anyhow::ensure!(
        missing.is_empty(),
        "not configured: {}; this live test fails rather than skipping",
        missing.join(", ")
    );
    Ok(())
}

/// What the tool returns and the final answer must repeat. A model that
/// answers without reading the tool output cannot guess it.
const STATION_CODE: &str = "KX-4417";
const LOOP_SYSTEM: &str =
    "You are a terse assistant. Use the get_weather tool for any weather question.";

fn station_tool() -> Tool {
    Tool::new(
        "get_weather",
        "Get the current weather report for a city, including its reporting station code",
        object!({
            "type": "object",
            "required": ["location"],
            "properties": {
                "location": { "type": "string", "description": "City name" }
            }
        }),
    )
}

fn station_question() -> Message {
    Message::user().with_text(
        "Call get_weather for San Francisco, then reply with only the reporting station code it returns.",
    )
}

fn station_report(id: &str) -> Message {
    Message::user().with_tool_response(
        id,
        Ok(rmcp::model::CallToolResult {
            content: vec![Content::text(format!(
                "San Francisco: 57F, clear. Reporting station code: {STATION_CODE}."
            ))],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        }),
    )
}

/// One provider turn: the assistant messages it produced and its usage.
struct LiveTurn {
    messages: Vec<Message>,
    usage: Option<biorouter::providers::base::ProviderUsage>,
}

impl LiveTurn {
    fn text(&self) -> String {
        self.messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_text)
            .collect::<Vec<_>>()
            .join("")
    }

    fn weather_call_id(&self) -> Option<String> {
        self.messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_tool_request)
            .find(|request| {
                request
                    .tool_call
                    .as_ref()
                    .is_ok_and(|call| call.name == "get_weather")
            })
            .map(|request| request.id.clone())
    }

    /// (input, output) tokens. Input counts the cache buckets too, which are
    /// disjoint from `input_tokens` by `Usage`'s invariant.
    fn tokens(&self) -> (i64, i64) {
        let Some(usage) = &self.usage else {
            return (0, 0);
        };
        let u = &usage.usage;
        let input = [
            u.input_tokens,
            u.cache_read_input_tokens,
            u.cache_creation_input_tokens,
        ]
        .iter()
        .map(|n| i64::from(n.unwrap_or(0)))
        .sum();
        (input, i64::from(u.output_tokens.unwrap_or(0)))
    }

    fn served_by(&self) -> String {
        self.usage
            .as_ref()
            .map(|usage| usage.model.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CallPath {
    Blocking,
    Streaming,
}

/// Drain a stream the way the agent loop records it: chunks that share an id
/// fold into one message (`Conversation::push`), and the last usage snapshot
/// wins.
async fn stream_turn(
    provider: &dyn Provider,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<LiveTurn, ProviderError> {
    use futures::StreamExt;

    let mut stream = provider.stream(system, messages, tools).await?;
    let mut folded = biorouter::conversation::Conversation::empty();
    let mut usage = None;
    while let Some(item) = stream.next().await {
        let (message, chunk_usage, _pending) = item?;
        if let Some(message) = message {
            folded.push(message);
        }
        if let Some(chunk_usage) = chunk_usage {
            usage = Some(chunk_usage);
        }
    }
    Ok(LiveTurn {
        messages: folded.into_messages(),
        usage,
    })
}

/// One turn on `path`. With `model` set, the blocking path goes through
/// `complete_with_model` with that config (the fast-model route); otherwise
/// through `complete`, which uses the provider's own config.
async fn live_turn(
    provider: &dyn Provider,
    path: CallPath,
    model: Option<&biorouter::model::ModelConfig>,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<LiveTurn, ProviderError> {
    match path {
        CallPath::Streaming => stream_turn(provider, system, messages, tools).await,
        CallPath::Blocking => {
            let (message, usage) = match model {
                Some(model) => {
                    provider
                        .complete_with_model(model, system, messages, tools)
                        .await?
                }
                None => provider.complete(system, messages, tools).await?,
            };
            Ok(LiveTurn {
                messages: vec![message],
                usage: Some(usage),
            })
        }
    }
}

/// A plain text turn: non-empty text and non-zero usage.
async fn check_text_turn(provider: &dyn Provider, path: CallPath) -> Result<String> {
    let turn = live_turn(
        provider,
        path,
        None,
        "You are a terse assistant.",
        &[Message::user().with_text("Reply with the single word: pong")],
        &[],
    )
    .await?;
    let (input, output) = turn.tokens();
    anyhow::ensure!(
        !turn.text().trim().is_empty(),
        "{path:?} text turn returned no text"
    );
    anyhow::ensure!(
        input > 0 && output > 0,
        "{path:?} text turn reported usage in={input} out={output}"
    );
    Ok(format!(
        "{path:?} text {:?} (in={input} out={output} model={})",
        turn.text().trim().chars().take(20).collect::<String>(),
        turn.served_by()
    ))
}

/// The two-step tool loop: the model must call `get_weather`, and after the
/// result is replayed it must answer with [`STATION_CODE`]. Both steps must
/// report non-zero usage. `model` routes the blocking path through
/// `complete_with_model`.
async fn check_tool_loop(
    provider: &dyn Provider,
    path: CallPath,
    model: Option<&biorouter::model::ModelConfig>,
) -> Result<String> {
    let tools = [station_tool()];
    let question = station_question();
    let first = live_turn(
        provider,
        path,
        model,
        LOOP_SYSTEM,
        std::slice::from_ref(&question),
        &tools,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{path:?} tool step 1: {e}"))?;
    let call_id = first.weather_call_id().ok_or_else(|| {
        anyhow::anyhow!(
            "{path:?} tool step 1: no get_weather call; text was {:?}",
            first.text()
        )
    })?;
    let (in1, out1) = first.tokens();
    anyhow::ensure!(
        in1 > 0 && out1 > 0,
        "{path:?} tool step 1 reported usage in={in1} out={out1}"
    );

    let mut history = vec![question];
    history.extend(first.messages.iter().cloned());
    history.push(station_report(&call_id));
    let second = live_turn(provider, path, model, LOOP_SYSTEM, &history, &tools)
        .await
        .map_err(|e| anyhow::anyhow!("{path:?} tool step 2 (result replayed): {e}"))?;
    let (in2, out2) = second.tokens();
    anyhow::ensure!(
        second.text().contains(STATION_CODE),
        "{path:?} tool step 2 did not repeat the tool's station code; text was {:?}",
        second.text()
    );
    anyhow::ensure!(
        in2 > 0 && out2 > 0,
        "{path:?} tool step 2 reported usage in={in2} out={out2}"
    );
    Ok(format!(
        "{path:?} tool loop ok (step1 in={in1} out={out1}; step2 in={in2} out={out2} answer={:?} model={})",
        second.text().trim().chars().take(30).collect::<String>(),
        second.served_by()
    ))
}

/// Run `check` for every model, print one row each, and fail at the end if
/// any row failed.
async fn sweep<F, Fut>(label: &str, models: &[&str], check: F) -> Result<()>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>>>,
{
    let mut failed = Vec::new();
    println!("\n===== {label} =====");
    for model in models {
        let started = std::time::Instant::now();
        match check(model.to_string()).await {
            Ok(rows) => {
                for row in rows {
                    println!("PASS {model}: {row}");
                }
                println!("     {model}: {:.1}s", started.elapsed().as_secs_f64());
            }
            Err(e) => {
                println!("FAIL {model}: {e:#}");
                failed.push(model.to_string());
            }
        }
    }
    anyhow::ensure!(failed.is_empty(), "{label}: failed for {failed:?}");
    Ok(())
}

/// Every offered Versa Azure model: a streamed text turn, and the tool loop on
/// both the streaming and the blocking path, at the default effort.
#[tokio::test]
#[ignore = "live billed Versa Azure calls; run deliberately with --ignored"]
async fn test_versa_azure_every_offered_model() -> Result<()> {
    use biorouter::providers::versa_azure::VERSA_AZURE_DEPLOYMENTS;

    require_versa_credentials(VERSA_AZURE_CREDENTIALS)?;
    let models: Vec<&str> = VERSA_AZURE_DEPLOYMENTS.iter().map(|(m, _)| *m).collect();
    sweep(
        "versa_azure: every offered model",
        &models,
        |model| async move {
            let provider = create_with_named_model("versa_azure", &model).await?;
            Ok(vec![
                check_text_turn(provider.as_ref(), CallPath::Streaming).await?,
                check_tool_loop(provider.as_ref(), CallPath::Streaming, None).await?,
                check_tool_loop(provider.as_ref(), CallPath::Blocking, None).await?,
            ])
        },
    )
    .await
}

/// The Responses route under a reasoning effort, set the way a turn sets it:
/// `ReasoningEffort::apply_to_model` on the model config, then the provider
/// rebuilt from it (`Agent::provider_with_effort`). Deep sends
/// `reasoning.effort: high`; Quick sends `low` AND fills in temperature 0,
/// which a reasoning model refuses, so Quick also proves the builder drops it.
/// gpt-5.5 is the Chat Completions control: same efforts, `reasoning_effort`
/// with tools on the route it stays on.
///
/// Also the fast-model path: a gpt-5.5 provider sending a gpt-5.6 request
/// through `complete_with_model` must take the Responses route for it.
#[tokio::test]
#[ignore = "live billed Versa Azure calls; run deliberately with --ignored"]
async fn test_versa_azure_reasoning_effort_tool_loops() -> Result<()> {
    use biorouter::agents::effort::ReasoningEffort;
    use biorouter::model::ModelConfig;

    require_versa_credentials(VERSA_AZURE_CREDENTIALS)?;
    let models = [
        "gpt-5.6-sol-2026-07-09",
        "gpt-5.6-terra-2026-07-09",
        "gpt-5.6-luna-2026-07-09",
        "gpt-5.5-2026-04-24",
    ];
    sweep(
        "versa_azure: reasoning effort + tool loops",
        &models,
        |model| async move {
            let mut rows = Vec::new();
            for effort in [ReasoningEffort::Deep, ReasoningEffort::Quick] {
                let config = effort.apply_to_model(ModelConfig::new(&model)?);
                let provider = biorouter::providers::create("versa_azure", config).await?;
                for path in [CallPath::Blocking, CallPath::Streaming] {
                    let row = check_tool_loop(provider.as_ref(), path, None)
                        .await
                        .map_err(|e| anyhow::anyhow!("effort {}: {e:#}", effort.as_str()))?;
                    rows.push(format!("effort={} {row}", effort.as_str()));
                }
            }
            if model.starts_with("gpt-5.6") {
                let chat = create_with_named_model("versa_azure", "gpt-5.5-2026-04-24").await?;
                let fast = ReasoningEffort::Deep.apply_to_model(ModelConfig::new(&model)?);
                let row = check_tool_loop(chat.as_ref(), CallPath::Blocking, Some(&fast))
                    .await
                    .map_err(|e| anyhow::anyhow!("fast-model path: {e:#}"))?;
                rows.push(format!("via gpt-5.5 complete_with_model effort=deep {row}"));
            }
            Ok(rows)
        },
    )
    .await
}

/// Every offered Versa Bedrock model: a streamed text turn, and the tool loop
/// on both paths.
#[tokio::test]
#[ignore = "live billed Versa Bedrock calls; run deliberately with --ignored"]
async fn test_versa_bedrock_every_offered_model() -> Result<()> {
    use biorouter::providers::versa_bedrock::VERSA_BEDROCK_KNOWN_MODELS;

    require_versa_credentials(VERSA_BEDROCK_CREDENTIALS)?;
    sweep(
        "versa_bedrock: every offered model",
        VERSA_BEDROCK_KNOWN_MODELS,
        |model| async move {
            let provider = create_with_named_model("versa_bedrock", &model).await?;
            Ok(vec![
                check_text_turn(provider.as_ref(), CallPath::Streaming).await?,
                check_tool_loop(provider.as_ref(), CallPath::Streaming, None).await?,
                check_tool_loop(provider.as_ref(), CallPath::Blocking, None).await?,
            ])
        },
    )
    .await
}

/// The models taken OUT of the two catalogs, through BioRouter:
///   * Versa Bedrock: `us.anthropic.claude-opus-5-5` is sent, AWS refuses it
///     (`AccessDeniedException`), and the user hears a model refusal naming
///     the offered models, on both paths, promptly (not retried).
///   * Versa Azure: `gpt-6-sol-2026-09-22` has no deployment, so it is refused
///     locally on every path. Proven against a mock gateway that records every
///     request: nothing reaches it, while a catalog model sent the same way
///     does (the instrument works).
#[tokio::test]
#[ignore = "live Versa Bedrock call; run deliberately with --ignored"]
async fn test_versa_removed_models_are_refused() -> Result<()> {
    require_versa_credentials(VERSA_BEDROCK_CREDENTIALS)?;
    let prompt = [Message::user().with_text("Reply with the single word OK.")];

    // Bedrock: sent, refused by AWS, surfaced as a model refusal.
    let removed = "us.anthropic.claude-opus-5-5";
    let provider = create_with_named_model("versa_bedrock", removed).await?;
    for path in [CallPath::Blocking, CallPath::Streaming] {
        let started = std::time::Instant::now();
        let error = match live_turn(
            provider.as_ref(),
            path,
            None,
            "You are a test.",
            &prompt,
            &[],
        )
        .await
        {
            Ok(_) => anyhow::bail!("{removed} answered on {path:?}: UCSF now serves it"),
            Err(error) => error,
        };
        let elapsed = started.elapsed();
        let text = error.to_string();
        println!(
            "versa_bedrock {removed} {path:?}: {:?} after {:.1}s: {text}",
            error.kind(),
            elapsed.as_secs_f64()
        );
        anyhow::ensure!(
            text.contains(&format!("does not serve model `{removed}`"))
                && text.contains("Your key pair works")
                && text.contains(VERSA_BEDROCK_DEFAULT_MODEL),
            "{path:?}: expected a model refusal naming the offered models, got {text}"
        );
        anyhow::ensure!(
            elapsed < std::time::Duration::from_secs(30),
            "{path:?}: the refusal took {elapsed:?}; it should not be retried"
        );
    }

    // Azure: refused before anything is sent. The instance is pointed at a
    // local TCP listener that counts connections (the endpoint must be https,
    // and nothing here speaks TLS, so any request that leaves fails at the
    // handshake, after it has been counted), with a placeholder key so the
    // real one never goes there.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&connections);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let previous: Vec<(&str, Option<String>)> = ["VERSA_AZURE_ENDPOINT", "VERSA_AZURE_API_KEY"]
        .into_iter()
        .map(|key| (key, std::env::var(key).ok()))
        .collect();
    std::env::set_var("VERSA_AZURE_ENDPOINT", format!("https://127.0.0.1:{port}"));
    std::env::set_var("VERSA_AZURE_API_KEY", "local-listener-placeholder");
    let outcome = async {
        let unmapped = "gpt-6-sol-2026-09-22";
        let provider = create_with_named_model("versa_azure", unmapped).await?;
        let tools = [station_tool()];
        for path in [CallPath::Blocking, CallPath::Streaming] {
            let error = match live_turn(provider.as_ref(), path, None, "t", &prompt, &tools).await {
                Ok(_) => anyhow::bail!("{unmapped} was answered on {path:?}"),
                Err(error) => error,
            };
            let text = error.to_string();
            println!("versa_azure {unmapped} {path:?}: {:?}: {text}", error.kind());
            anyhow::ensure!(
                text.contains(&format!("no Versa deployment for model `{unmapped}`"))
                    && text.contains("nothing was sent"),
                "{path:?}: expected the local refusal, got {text}"
            );
        }
        // The fast-model path, from a chat on a catalog model.
        let chat = create_with_named_model("versa_azure", "gpt-5.5-2026-04-24").await?;
        let fast = biorouter::model::ModelConfig::new(unmapped)?;
        let error = chat
            .complete_with_model(&fast, "t", &prompt, &tools)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("{unmapped} was answered as a fast model"))?;
        println!("versa_azure {unmapped} as a gpt-5.5 chat's fast model: {error}");
        anyhow::ensure!(
            error.to_string().contains("nothing was sent"),
            "fast-model path: {error}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let refused = connections.load(Ordering::SeqCst);
        anyhow::ensure!(
            refused == 0,
            "{refused} connection(s) were opened for {unmapped}"
        );
        // The instrument: a catalog model sent the same way DOES connect.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            chat.complete("t", &prompt, &[]),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let control = connections.load(Ordering::SeqCst);
        anyhow::ensure!(
            control > 0,
            "the control request never connected, so the listener's silence proves nothing"
        );
        println!(
            "versa_azure {unmapped}: 0 connections opened on 3 paths; control gpt-5.5 opened {control}"
        );
        Ok(())
    }
    .await;
    for (key, value) in previous {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    outcome
}

#[ctor::dtor]
fn print_test_report() {
    TEST_REPORT.print_summary();
}
