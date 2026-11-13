//! An ACP process can never prove that a person made a decision: every card it shows is
//! answered with `DecisionAuthority::unproven()`, because the protocol cannot tell an editor's
//! click from an auto-allow policy. So it must say so, and offer no tool whose approval needs
//! that proof (SD-8). `crew__change` is the one this checks by name: every one of its actions
//! waits on a proof-backed card, which an ACP client could never allow.
//!
//! ⚠ A binary of its own, holding ONE test. It points `BIOROUTER_PATH_ROOT` at a temporary
//! directory, so Crew's policy is read from there and never from the person's own
//! configuration, and it installs a Crew tool host, which no test sharing its process could
//! undo. With nothing else in the process, neither can reach another test.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::agents::ExtensionConfig;
use biorouter::config::BioRouterMode;
use biorouter::crew::tool_host::{CrewToolHost, PersonToolCall};
use biorouter::model::ModelConfig;
use biorouter::providers::api_client::{ApiClient, AuthMethod};
use biorouter::providers::openai::OpenAiProvider;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use biorouter_acp::server::{BioRouterAcpAgent, BioRouterAcpConfig};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A host that answers nothing: installed only so the Crew person tools are offered at all,
/// which leaves the proof as the one thing that can withhold `crew__change`. No call is made.
struct NoCallsHost;

impl CrewToolHost for NoCallsHost {
    // `CrewToolHost` is an `async_trait`; this is the signature the macro gives it, written
    // out because this crate does not depend on the macro.
    fn call<'life0, 'async_trait>(
        &'life0 self,
        _call: PersonToolCall,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<serde_json::Value>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { anyhow::bail!("this test makes no Crew call") })
    }
}

/// A provider for an agent that runs no turn. Construction makes no network call.
fn no_turn_provider() -> Arc<dyn biorouter::providers::base::Provider> {
    let api_client = ApiClient::new(
        "http://127.0.0.1:1".to_string(),
        AuthMethod::BearerToken("test-key".to_string()),
    )
    .unwrap();
    Arc::new(OpenAiProvider::new(
        api_client,
        ModelConfig::new("gpt-5-nano").unwrap(),
    ))
}

#[test]
fn an_acp_process_offers_no_tool_that_needs_a_persons_proof() {
    let root = tempfile::tempdir().unwrap();
    // Set before anything reads it; this binary has no other test to race.
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    biorouter::crew::tool_host::install(Arc::new(NoCallsHost));

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            assert!(
                biorouter::pending_user_action::user_proof_available(),
                "premise: a process counts as able to prove a person acted until it says otherwise"
            );
            let data = root.path().join("acp");
            let agent = BioRouterAcpAgent::with_config(BioRouterAcpConfig {
                provider: no_turn_provider(),
                builtins: vec![],
                work_dir: root.path().to_path_buf(),
                data_dir: data.clone(),
                config_dir: data.clone(),
                biorouter_mode: BioRouterMode::Auto,
                extensions: vec![ExtensionConfig::Platform {
                    name: "crew".to_string(),
                    bundled: None,
                    description: "crew".to_string(),
                    available_tools: vec![],
                }],
            })
            .await
            .unwrap();
            assert!(
                !biorouter::pending_user_action::user_proof_available(),
                "an ACP process must report that it can never obtain a person's proof"
            );

            // The agent's own store: a chat the ACP client would have opened.
            let chat = SessionManager::new(data)
                .create_session(
                    root.path().to_path_buf(),
                    "ACP chat".to_string(),
                    SessionType::User,
                )
                .await
                .unwrap();
            let tools = agent.list_tool_names(&chat.id).await;
            assert!(
                tools.iter().any(|tool| tool == "crew__read"),
                "premise: this chat is offered the Crew person tools at all: {tools:?}"
            );
            assert!(
                !tools.iter().any(|tool| tool == "crew__change"),
                "crew__change waits on a card ACP can never allow, so it is not offered: {tools:?}"
            );
        });
}
