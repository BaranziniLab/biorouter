//! A turn that fails must *look* like it failed.
//!
//! Before `AgentEvent::TurnAborted`, a provider failure was downgraded into an
//! assistant chat message ("Ran into this error: …") and the stream then ended
//! **normally**. Nothing downstream could tell a 403 from a completed turn
//! without regex-matching English prose, so `biorouter run` exited 0 on an auth
//! failure, `--output-format json` said `"status":"completed"`, and telemetry
//! recorded the failed run as a success.
//!
//! These tests drive the real reply loop with a provider that fails, and assert
//! the loop yields the typed terminal event — and that a *successful* turn still
//! does not, so the abort path cannot fire spuriously.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::turn_abort::{exit, TurnAbortCode};
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::{ProviderError, ProviderErrorKind};
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::Tool;
use tempfile::TempDir;

/// A provider that always fails with a caller-chosen error.
struct FailingProvider {
    calls: AtomicUsize,
    make_error: fn() -> ProviderError,
}

impl FailingProvider {
    fn new(make_error: fn() -> ProviderError) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            make_error,
        }
    }
}

#[async_trait]
impl Provider for FailingProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err((self.make_error)())
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system_prompt, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for testing".to_string(),
            default_model: "mock-model".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "mock-test"
    }
}

/// A provider that answers normally — the control case.
struct HappyProvider;

#[async_trait]
impl Provider for HappyProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Ok((
            Message::assistant().with_text("all good"),
            ProviderUsage::new(
                "mock-model".to_string(),
                Usage::new(Some(1), Some(1), Some(2)),
            ),
        ))
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system_prompt, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for testing".to_string(),
            default_model: "mock-model".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "mock-test"
    }
}

async fn agent_with(provider: Arc<dyn Provider>) -> (Agent, String, TempDir) {
    let work_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    let config = AgentConfig::new(
        session_manager.clone(),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    );
    let agent = Agent::with_config(config);

    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "turn-abort-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    agent.update_provider(provider, &session.id).await.unwrap();
    std::mem::forget(data_dir);
    (agent, session.id, work_dir)
}

/// Drain a turn, returning every abort event it yielded.
async fn aborts_from_turn(agent: &Agent, session_id: &str) -> Result<Vec<TurnAbortCode>> {
    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text("go"), session_config, None)
        .await?;
    tokio::pin!(stream);

    let mut aborts = Vec::new();
    while let Some(event) = stream.next().await {
        if let Ok(AgentEvent::TurnAborted { code, .. }) = event {
            aborts.push(code);
        }
    }
    Ok(aborts)
}

/// The headline case: a 403. This is the exact failure that used to exit 0.
#[tokio::test]
async fn an_auth_failure_aborts_the_turn_with_an_auth_code() {
    let provider = Arc::new(FailingProvider::new(|| {
        ProviderError::Authentication("403 Forbidden: IP not allowlisted".into())
    }));
    let (agent, session_id, _work) = agent_with(provider).await;

    let aborts = aborts_from_turn(&agent, &session_id).await.unwrap();

    assert_eq!(
        aborts.len(),
        1,
        "a failed provider call must yield exactly one TurnAborted"
    );
    match &aborts[0] {
        TurnAbortCode::ProviderFailure { kind } => {
            assert_eq!(*kind, ProviderErrorKind::Auth);
            assert!(kind.is_auth());
        }
        other => panic!("expected a provider failure, got {other:?}"),
    }
    assert_eq!(
        aborts[0].exit_code(),
        exit::PROVIDER_AUTH,
        "an auth failure must exit 75, not 0"
    );
}

/// A transient server error is still an abort — but a *different* exit code, so
/// a caller can retry it rather than go looking for a bad credential.
#[tokio::test]
async fn a_server_error_aborts_with_the_retryable_code() {
    let provider = Arc::new(FailingProvider::new(|| {
        ProviderError::ServerError("502 Bad Gateway".into())
    }));
    let (agent, session_id, _work) = agent_with(provider).await;

    let aborts = aborts_from_turn(&agent, &session_id).await.unwrap();

    assert_eq!(aborts.len(), 1);
    match &aborts[0] {
        TurnAbortCode::ProviderFailure { kind } => {
            assert_eq!(*kind, ProviderErrorKind::Server);
            assert!(kind.is_transient(), "a 502 is worth retrying");
            assert!(!kind.is_auth());
        }
        other => panic!("expected a provider failure, got {other:?}"),
    }
    assert_eq!(aborts[0].exit_code(), exit::PROVIDER_FAILED);
}

/// The guard against over-eager failure classification: a turn that actually
/// worked must never yield an abort, or every successful CLI run starts exiting
/// nonzero.
#[tokio::test]
async fn a_successful_turn_yields_no_abort() {
    let (agent, session_id, _work) = agent_with(Arc::new(HappyProvider)).await;

    let aborts = aborts_from_turn(&agent, &session_id).await.unwrap();

    assert!(
        aborts.is_empty(),
        "a completed turn must not abort, got {aborts:?}"
    );
}

/// The human-readable message the desktop renders must survive alongside the new
/// typed event — we added a channel, we did not replace one.
#[tokio::test]
async fn the_human_readable_error_message_is_still_emitted() {
    let provider = Arc::new(FailingProvider::new(|| {
        ProviderError::Authentication("403 Forbidden".into())
    }));
    let (agent, session_id, _work) = agent_with(provider).await;

    let session_config = SessionConfig {
        id: session_id.clone(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text("go"), session_config, None)
        .await
        .unwrap();
    tokio::pin!(stream);

    let mut saw_message = false;
    let mut saw_abort = false;
    while let Some(event) = stream.next().await {
        match event {
            Ok(AgentEvent::Message(m)) => {
                if m.as_concat_text().contains("Ran into this error") {
                    saw_message = true;
                }
            }
            Ok(AgentEvent::TurnAborted { .. }) => saw_abort = true,
            _ => {}
        }
    }

    assert!(
        saw_message,
        "the assistant-facing error text must still stream"
    );
    assert!(
        saw_abort,
        "and the machine-checkable abort must accompany it"
    );
}

/// D4b: a steer the turn accepted and then aborted without ever showing the
/// model is stored — the person's words are never discarded — but marked
/// `unanswered`, so no client draws it as a message that landed.
///
/// Before the marker the carried-over row was byte-for-byte a consumed steer:
/// the desktop retired its "steering" chip on its echo, and the transcript
/// showed a user bubble nothing ever replied to (live, 5/5 at Stop).
#[tokio::test(flavor = "multi_thread")]
async fn a_steer_accepted_before_a_provider_abort_is_stored_as_unanswered() {
    use biorouter::conversation::message::SteerOutcome;

    /// Queues a steer from inside the provider call — the turn is certainly
    /// accepting then — and then fails with an abort-classified error.
    struct SteerThenAbort {
        agent: std::sync::OnceLock<Arc<Agent>>,
    }

    #[async_trait]
    impl Provider for SteerThenAbort {
        async fn complete(
            &self,
            _system_prompt: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.agent
                .get()
                .expect("agent installed before the turn")
                .try_queue_soft_interrupt("use the other dataset".into(), None)
                .expect("a running turn accepts a steer");
            Err(ProviderError::Authentication("403 Forbidden".into()))
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system_prompt: &str,
            messages: &[Message],
            tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.complete(system_prompt, messages, tools).await
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new("mock-model").unwrap()
        }

        fn metadata() -> ProviderMetadata {
            HappyProvider::metadata()
        }

        fn get_name(&self) -> &str {
            "mock-steer-then-abort"
        }
    }

    let provider = Arc::new(SteerThenAbort {
        agent: std::sync::OnceLock::new(),
    });
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    let agent = Arc::new(agent);
    assert!(provider.agent.set(Arc::clone(&agent)).is_ok());

    let session_config = SessionConfig {
        id: session_id.clone(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text("go"), session_config, None)
        .await
        .unwrap();
    tokio::pin!(stream);
    let mut steer_rows = Vec::new();
    let mut aborted = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            AgentEvent::Message(m) if m.as_concat_text() == "use the other dataset" => {
                steer_rows.push(m)
            }
            AgentEvent::TurnAborted { .. } => aborted = true,
            _ => {}
        }
    }

    assert!(aborted, "the provider failure must still abort the turn");
    assert_eq!(
        steer_rows.len(),
        1,
        "the accepted steer must be stored once"
    );
    assert_eq!(
        steer_rows[0].metadata.steer_outcome,
        Some(SteerOutcome::Unanswered),
        "a steer the model never read must not look delivered"
    );
}
