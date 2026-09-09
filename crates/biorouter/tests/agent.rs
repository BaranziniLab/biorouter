use std::sync::Arc;

use anyhow::Result;
use biorouter::agents::{Agent, AgentEvent};
use biorouter::config::extensions::{set_extension, ExtensionEntry};
use biorouter::config::Config;
use futures::StreamExt;

/// Sandbox the whole test binary's config root before any test runs (issue #54).
///
/// `extension_manager_tests` calls `set_extension`, which writes through
/// `Config::global()`. Unsandboxed that is the developer's real
/// `~/.config/biorouter/config.yaml`: a routine `cargo test --workspace`
/// rewrote it and pushed one entry off the end of the 5-deep backup chain, and
/// it would have flipped the `todo` extension to `enabled: false` for anyone
/// who had it switched on.
///
/// This has to be a `ctor` rather than a scoped guard inside the offending
/// test. `Config::global()` is a `OnceCell` that resolves its path exactly
/// once per process, so whichever test touches it first decides where *every
/// later* write in the binary lands. A guard installed inside one test module
/// therefore fixes nothing whenever another test in the binary got there first,
/// and the tests here run in parallel — so which one wins is a coin flip.
/// Running before `main` is the only placement that cannot lose that race.
///
/// Nothing needs restoring on panic the way a scoped guard would: the variable
/// belongs to this test process for its whole life, and tests that install
/// their own `BIOROUTER_PATH_ROOT` through `env_lock` restore it back to this
/// sandbox rather than to "unset". An outer `BIOROUTER_PATH_ROOT` set by the
/// caller is deliberately left alone, so an external harness can still choose
/// the sandbox.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    if std::env::var_os("BIOROUTER_PATH_ROOT").is_some() {
        return;
    }
    let root = tempfile::TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    // Held for the life of the process; a static is never dropped, which is
    // exactly the lifetime the sandbox needs.
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard for issue #54: nothing in this binary may resolve to the
    /// developer's live configuration.
    ///
    /// Asserting on `Config::global()` rather than on the environment is what
    /// makes this meaningful — it is the resolved path that a write actually
    /// follows, and it is frozen at first use.
    #[test]
    fn the_global_config_is_sandboxed_for_this_binary() {
        let root = std::env::var("BIOROUTER_PATH_ROOT")
            .expect("BIOROUTER_PATH_ROOT must be sandboxed before any test in this binary runs");
        let path = Config::global().path();
        assert!(
            path.starts_with(&root),
            "Config::global() resolved to {path}, outside the sandbox at {root}. \
             Something reached Config::global() before the sandbox was installed, \
             so config writes from this binary land in the developer's real config."
        );
    }

    #[cfg(test)]
    mod schedule_tool_tests {
        use super::*;
        use async_trait::async_trait;
        use biorouter::agents::platform_tools::PLATFORM_MANAGE_SCHEDULE_TOOL_NAME;
        use biorouter::agents::AgentConfig;
        use biorouter::config::permission::PermissionManager;
        use biorouter::config::BioRouterMode;
        use biorouter::scheduler::{ScheduledJob, SchedulerError};
        use biorouter::scheduler_trait::SchedulerTrait;
        use biorouter::session::{Session, SessionManager};
        use chrono::{DateTime, Utc};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tempfile::TempDir;

        struct MockScheduler {
            jobs: tokio::sync::Mutex<Vec<ScheduledJob>>,
        }

        impl MockScheduler {
            fn new() -> Self {
                Self {
                    jobs: tokio::sync::Mutex::new(Vec::new()),
                }
            }
        }

        #[async_trait]
        impl SchedulerTrait for MockScheduler {
            async fn add_scheduled_job(
                &self,
                job: ScheduledJob,
                _copy: bool,
            ) -> Result<(), SchedulerError> {
                let mut jobs = self.jobs.lock().await;
                jobs.push(job);
                Ok(())
            }

            async fn schedule_workflow(
                &self,
                _workflow_path: PathBuf,
                _cron_schedule: Option<String>,
            ) -> Result<(), SchedulerError> {
                Ok(())
            }

            async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
                let jobs = self.jobs.lock().await;
                jobs.clone()
            }

            async fn remove_scheduled_job(
                &self,
                id: &str,
                _remove: bool,
            ) -> Result<(), SchedulerError> {
                let mut jobs = self.jobs.lock().await;
                if let Some(pos) = jobs.iter().position(|job| job.id == id) {
                    jobs.remove(pos);
                    Ok(())
                } else {
                    Err(SchedulerError::JobNotFound(id.to_string()))
                }
            }

            async fn pause_schedule(&self, _id: &str) -> Result<(), SchedulerError> {
                Ok(())
            }

            async fn unpause_schedule(&self, _id: &str) -> Result<(), SchedulerError> {
                Ok(())
            }

            async fn run_now(&self, _id: &str) -> Result<String, SchedulerError> {
                Ok("test_session_123".to_string())
            }

            async fn sessions(
                &self,
                _sched_id: &str,
                _limit: usize,
            ) -> Result<Vec<(String, Session)>, SchedulerError> {
                Ok(vec![])
            }

            async fn update_schedule(
                &self,
                _sched_id: &str,
                _new_cron: String,
            ) -> Result<(), SchedulerError> {
                Ok(())
            }

            async fn kill_running_job(&self, _sched_id: &str) -> Result<(), SchedulerError> {
                Ok(())
            }

            async fn get_running_job_info(
                &self,
                _sched_id: &str,
            ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
                Ok(None)
            }
        }

        #[tokio::test]
        async fn test_schedule_management_tool_list() {
            let temp_dir = TempDir::new().unwrap();
            let data_dir = temp_dir.path().to_path_buf();
            let session_manager = Arc::new(SessionManager::new(data_dir.clone()));
            let permission_manager = Arc::new(PermissionManager::new(data_dir));
            let mock_scheduler = Arc::new(MockScheduler::new());
            let config = AgentConfig::new(
                session_manager,
                permission_manager,
                Some(mock_scheduler),
                BioRouterMode::Auto,
            );
            let agent = Agent::with_config(config);

            let tools = agent.list_tools("test-session-id", None).await;
            let schedule_tool = tools
                .iter()
                .find(|tool| tool.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME);
            assert!(schedule_tool.is_some());

            let tool = schedule_tool.unwrap();
            assert!(tool
                .description
                .clone()
                .unwrap_or_default()
                .contains("Manage biorouter's internal scheduled workflow execution"));
        }

        #[tokio::test]
        async fn test_no_schedule_management_tool_without_scheduler() {
            let agent = Agent::new();

            let tools = agent.list_tools("test-session-id", None).await;
            let schedule_tool = tools
                .iter()
                .find(|tool| tool.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME);
            assert!(schedule_tool.is_none());
        }

        #[tokio::test]
        async fn test_schedule_management_tool_in_platform_tools() {
            let temp_dir = TempDir::new().unwrap();
            let data_dir = temp_dir.path().to_path_buf();
            let session_manager = Arc::new(SessionManager::new(data_dir.clone()));
            let permission_manager = Arc::new(PermissionManager::new(data_dir));
            let mock_scheduler = Arc::new(MockScheduler::new());
            let config = AgentConfig::new(
                session_manager,
                permission_manager,
                Some(mock_scheduler),
                BioRouterMode::Auto,
            );
            let agent = Agent::with_config(config);

            let tools = agent
                .list_tools("test-session-id", Some("platform".to_string()))
                .await;

            // Check that the schedule management tool is included in platform tools
            let schedule_tool = tools
                .iter()
                .find(|tool| tool.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME);
            assert!(schedule_tool.is_some());

            let tool = schedule_tool.unwrap();
            assert!(tool
                .description
                .clone()
                .unwrap_or_default()
                .contains("Manage biorouter's internal scheduled workflow execution"));

            // Verify the tool has the expected actions in its schema
            if let Some(properties) = tool.input_schema.get("properties") {
                if let Some(action_prop) = properties.get("action") {
                    if let Some(enum_values) = action_prop.get("enum") {
                        let actions: Vec<String> = enum_values
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_str().unwrap().to_string())
                            .collect();

                        // Check that our session_content action is included
                        assert!(actions.contains(&"session_content".to_string()));
                        assert!(actions.contains(&"list".to_string()));
                        assert!(actions.contains(&"create".to_string()));
                        assert!(actions.contains(&"sessions".to_string()));
                    }
                }
            }
        }

        #[tokio::test]
        async fn test_schedule_management_tool_schema_validation() {
            let temp_dir = TempDir::new().unwrap();
            let data_dir = temp_dir.path().to_path_buf();
            let session_manager = Arc::new(SessionManager::new(data_dir.clone()));
            let permission_manager = Arc::new(PermissionManager::new(data_dir));
            let mock_scheduler = Arc::new(MockScheduler::new());
            let config = AgentConfig::new(
                session_manager,
                permission_manager,
                Some(mock_scheduler),
                BioRouterMode::Auto,
            );
            let agent = Agent::with_config(config);

            let tools = agent.list_tools("test-session-id", None).await;
            let schedule_tool = tools
                .iter()
                .find(|tool| tool.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME);
            assert!(schedule_tool.is_some());

            let tool = schedule_tool.unwrap();

            // Verify the tool schema has the session_id parameter for session_content action
            if let Some(properties) = tool.input_schema.get("properties") {
                assert!(properties.get("session_id").is_some());

                if let Some(session_id_prop) = properties.get("session_id") {
                    assert_eq!(
                        session_id_prop.get("type").unwrap().as_str().unwrap(),
                        "string"
                    );
                    assert!(session_id_prop
                        .get("description")
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .contains("Session identifier for session_content action"));
                }
            }
        }

        /// Issue #56 (R5), the third schedule-creating surface.
        ///
        /// `/loop` and `/schedule` record the chat that made them, so
        /// `resolve_scheduled_provider` can run the job on that chat's model
        /// instead of the user's commercial default. The agent's own
        /// `schedule_management` tool did not, so a schedule an agent creates on
        /// the user's behalf *from a private chat* still fell through to
        /// `Config::global()` — the exact hole the rest of the task closes.
        ///
        /// ⚠ The id comes from `dispatch_tool_call`'s own `session` argument,
        /// NOT from `session_context::current_session_id()`. That task-local is
        /// scoped around a *scheduled* run (`scheduler.rs`) and a *subagent* run
        /// (`subagent_handler.rs`) and nowhere else — in particular not around
        /// `Agent::reply` on the ordinary chat path — so it reads `None` in
        /// precisely the case this closes. The dispatcher has the real session
        /// in hand; that is the honest source.
        #[tokio::test]
        async fn an_agent_created_schedule_records_the_chat_that_asked_for_it() {
            use rmcp::model::CallToolRequestParams;

            let temp_dir = TempDir::new().unwrap();
            let data_dir = temp_dir.path().to_path_buf();
            let session_manager = Arc::new(SessionManager::new(data_dir.clone()));
            let permission_manager = Arc::new(PermissionManager::new(data_dir.clone()));
            let mock_scheduler = Arc::new(MockScheduler::new());
            let agent = Agent::with_config(AgentConfig::new(
                session_manager.clone(),
                permission_manager,
                Some(mock_scheduler.clone()),
                BioRouterMode::Auto,
            ));

            let session = session_manager
                .create_session(
                    data_dir.clone(),
                    "the chat that asked".to_string(),
                    biorouter::session::session_manager::SessionType::User,
                )
                .await
                .unwrap();

            let workflow_path = data_dir.join("nightly.yaml");
            std::fs::write(
                &workflow_path,
                "title: Nightly\ndescription: A nightly job\nprompt: do the thing\n",
            )
            .unwrap();

            let (_id, result) = agent
                .dispatch_tool_call(
                    CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: PLATFORM_MANAGE_SCHEDULE_TOOL_NAME.into(),
                        arguments: serde_json::json!({
                            "action": "create",
                            "workflow_path": workflow_path.to_str().unwrap(),
                            "cron_expression": "0 0 1 * * *",
                        })
                        .as_object()
                        .cloned(),
                    },
                    "req-1".to_string(),
                    None,
                    &session,
                )
                .await;
            // `ToolCallResult` is not `Debug`; the error side is what matters.
            assert!(result.is_ok(), "{:?}", result.as_ref().err());

            let jobs = mock_scheduler.list_scheduled_jobs().await;
            assert_eq!(jobs.len(), 1, "{jobs:?}");
            assert_eq!(
                jobs[0].creator_session_id.as_deref(),
                Some(session.id.as_str()),
                "the schedule must remember the chat it was created from, or its runs fall back \
                 to the global default and leave a private chat's work on a public model"
            );
        }
    }

    #[cfg(test)]
    mod retry_tests {
        use super::*;
        use biorouter::agents::types::{RetryConfig, SuccessCheck};

        #[tokio::test]
        async fn test_retry_success_check_execution() -> Result<()> {
            use biorouter::agents::retry::execute_success_checks;

            let retry_config = RetryConfig {
                max_retries: 3,
                checks: vec![],
                on_failure: None,
                timeout_seconds: Some(30),
                on_failure_timeout_seconds: Some(60),
            };

            let success_checks = vec![SuccessCheck::Shell {
                command: "echo 'test'".to_string(),
            }];

            let result = execute_success_checks(&success_checks, &retry_config).await;
            assert!(result.is_ok(), "Success check should pass");
            assert!(result.unwrap(), "Command should succeed");

            let fail_checks = vec![SuccessCheck::Shell {
                command: "false".to_string(),
            }];

            let result = execute_success_checks(&fail_checks, &retry_config).await;
            assert!(result.is_ok(), "Success check execution should not error");
            assert!(!result.unwrap(), "Command should fail");

            Ok(())
        }

        #[tokio::test]
        async fn test_retry_logic_with_validation_errors() -> Result<()> {
            let invalid_retry_config = RetryConfig {
                max_retries: 0,
                checks: vec![],
                on_failure: None,
                timeout_seconds: Some(0),
                on_failure_timeout_seconds: None,
            };

            let validation_result = invalid_retry_config.validate();
            assert!(
                validation_result.is_err(),
                "Should validate max_retries > 0"
            );
            assert!(validation_result
                .unwrap_err()
                .contains("max_retries must be greater than 0"));

            Ok(())
        }

        #[tokio::test]
        async fn test_retry_attempts_counter_reset() -> Result<()> {
            let agent = Agent::new();

            agent.reset_retry_attempts().await;
            let initial_attempts = agent.get_retry_attempts().await;
            assert_eq!(initial_attempts, 0);

            let new_attempts = agent.increment_retry_attempts().await;
            assert_eq!(new_attempts, 1);

            agent.reset_retry_attempts().await;
            let reset_attempts = agent.get_retry_attempts().await;
            assert_eq!(reset_attempts, 0);

            Ok(())
        }
    }

    #[cfg(test)]
    mod max_turns_tests {
        use super::*;
        use async_trait::async_trait;
        use biorouter::agents::SessionConfig;
        use biorouter::conversation::message::{Message, MessageContent};
        use biorouter::model::ModelConfig;
        use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
        use biorouter::providers::errors::ProviderError;
        use biorouter::session::session_manager::SessionType;
        use rmcp::model::{CallToolRequestParams, Tool};
        use rmcp::object;
        use std::path::PathBuf;

        struct MockToolProvider {}

        impl MockToolProvider {
            fn new() -> Self {
                Self {}
            }
        }

        #[async_trait]
        impl Provider for MockToolProvider {
            async fn complete(
                &self,
                _system_prompt: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                let tool_call = CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "test_tool".into(),
                    arguments: Some(object!({"param": "value"})),
                };
                let message = Message::assistant().with_tool_request("call_123", Ok(tool_call));

                let usage = ProviderUsage::new(
                    "mock-model".to_string(),
                    Usage::new(Some(10), Some(5), Some(15)),
                );

                Ok((message, usage))
            }

            async fn complete_with_model(
                &self,
                _model_config: &ModelConfig,
                system_prompt: &str,
                messages: &[Message],
                tools: &[Tool],
            ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
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
                    model_doc_link: "".to_string(),
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

        #[tokio::test]
        async fn test_max_turns_limit() -> Result<()> {
            let agent = Agent::new();
            let provider = Arc::new(MockToolProvider::new());
            let user_message = Message::user().with_text("Hello");

            let session = agent
                .config
                .session_manager
                .create_session(
                    PathBuf::default(),
                    "max-turn-test".to_string(),
                    SessionType::Hidden,
                )
                .await?;

            agent.update_provider(provider, &session.id).await?;

            let session_config = SessionConfig {
                id: session.id,
                schedule_id: None,
                max_turns: Some(1),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            };

            let reply_stream = agent.reply(user_message, session_config, None).await?;
            tokio::pin!(reply_stream);

            let mut responses = Vec::new();
            while let Some(response_result) = reply_stream.next().await {
                match response_result {
                    Ok(AgentEvent::Message(response)) => {
                        if let Some(MessageContent::ActionRequired(action)) =
                            response.content.first()
                        {
                            if let biorouter::conversation::message::ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                                agent.handle_confirmation(
                                    id.clone(),
                                    biorouter::permission::PermissionConfirmation {
                                        principal_type: biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                        permission: biorouter::permission::Permission::AllowOnce,
                                    }
                                ).await;
                            }
                        }
                        responses.push(response);
                    }
                    Ok(AgentEvent::McpNotification(_)) => {}
                    Ok(AgentEvent::ToolCallPending(_)) => {}
                    Ok(AgentEvent::MessagesPersisted(_)) => {}
                    Ok(AgentEvent::ModelChange { .. }) => {}
                    Ok(AgentEvent::PrivacyProviderPinned { .. }) => {}
                    Ok(AgentEvent::HistoryReplaced(_updated_conversation)) => {
                        // We should update the conversation here, but we're not reading it
                    }
                    Ok(AgentEvent::TokenUsage(_)) => {}
                    Ok(AgentEvent::TurnAborted { code, message }) => {
                        return Err(anyhow::anyhow!(
                            "turn aborted ({}): {message}",
                            code.wire_code()
                        ));
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

            assert!(
                !responses.is_empty(),
                "Expected at least 1 response, got {}",
                responses.len()
            );

            // Look for the max turns message as the last response
            let last_response = responses.last().unwrap();
            let last_content = last_response.content.first().unwrap();
            if let MessageContent::Text(text_content) = last_content {
                assert!(text_content
                    .text
                    .contains("I've reached my action limit for this turn"));
            } else {
                panic!("Expected text content in last message");
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod max_tool_calls_tests {
        use super::*;
        use async_trait::async_trait;
        use biorouter::agents::SessionConfig;
        use biorouter::conversation::message::{Message, MessageContent};
        use biorouter::model::ModelConfig;
        use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
        use biorouter::providers::errors::ProviderError;
        use biorouter::session::session_manager::SessionType;
        use rmcp::model::{CallToolRequestParams, Tool};
        use rmcp::object;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Fans out two *distinct* tool calls per response (unique args each time),
        // so the cumulative tool-call count grows twice as fast as the iteration
        // count and never trips the exact-duplicate repetition guard — the exact
        // "ever-changing args" runaway the per-turn tool-call cap is meant to bound.
        struct MockFanoutProvider {
            counter: AtomicUsize,
        }

        impl MockFanoutProvider {
            fn new() -> Self {
                Self {
                    counter: AtomicUsize::new(0),
                }
            }
        }

        #[async_trait]
        impl Provider for MockFanoutProvider {
            async fn complete(
                &self,
                _system_prompt: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                let base = self.counter.fetch_add(2, Ordering::SeqCst);
                let call_a = CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "test_tool".into(),
                    arguments: Some(object!({ "param": format!("value-{base}") })),
                };
                let call_b = CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "test_tool".into(),
                    arguments: Some(object!({ "param": format!("value-{}", base + 1) })),
                };
                let message = Message::assistant()
                    .with_tool_request(format!("call_{base}"), Ok(call_a))
                    .with_tool_request(format!("call_{}", base + 1), Ok(call_b));

                let usage = ProviderUsage::new(
                    "mock-model".to_string(),
                    Usage::new(Some(10), Some(5), Some(15)),
                );
                Ok((message, usage))
            }

            async fn complete_with_model(
                &self,
                _model_config: &ModelConfig,
                system_prompt: &str,
                messages: &[Message],
                tools: &[Tool],
            ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
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
                    model_doc_link: "".to_string(),
                    config_keys: vec![],
                    allows_unlisted_models: false,
                    tier: Default::default(),
                    runs_locally: false,
                    institutions: Vec::new(),
                }
            }

            fn get_name(&self) -> &str {
                "mock-fanout"
            }
        }

        #[tokio::test]
        async fn test_max_tool_calls_limit() -> Result<()> {
            let agent = Agent::new();
            let provider = Arc::new(MockFanoutProvider::new());
            let user_message = Message::user().with_text("Hello");

            let session = agent
                .config
                .session_manager
                .create_session(
                    PathBuf::default(),
                    "max-tool-calls-test".to_string(),
                    SessionType::Hidden,
                )
                .await?;

            agent.update_provider(provider, &session.id).await?;

            let session_config = SessionConfig {
                id: session.id,
                schedule_id: None,
                // High turn cap so the tool-call cap — not the iteration cap — is
                // what stops the reply. With 2 calls/iteration it trips at iter 3.
                max_turns: Some(100),
                max_tool_calls: Some(3),
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            };

            let reply_stream = agent.reply(user_message, session_config, None).await?;
            tokio::pin!(reply_stream);

            let mut responses = Vec::new();
            while let Some(response_result) = reply_stream.next().await {
                match response_result {
                    Ok(AgentEvent::Message(response)) => {
                        if let Some(MessageContent::ActionRequired(action)) =
                            response.content.first()
                        {
                            if let biorouter::conversation::message::ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                                agent.handle_confirmation(
                                    id.clone(),
                                    biorouter::permission::PermissionConfirmation {
                                        principal_type: biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                        permission: biorouter::permission::Permission::AllowOnce,
                                    }
                                ).await;
                            }
                        }
                        responses.push(response);
                    }
                    Ok(AgentEvent::McpNotification(_)) => {}
                    Ok(AgentEvent::ToolCallPending(_)) => {}
                    Ok(AgentEvent::MessagesPersisted(_)) => {}
                    Ok(AgentEvent::ModelChange { .. }) => {}
                    Ok(AgentEvent::PrivacyProviderPinned { .. }) => {}
                    Ok(AgentEvent::HistoryReplaced(_)) => {}
                    Ok(AgentEvent::TokenUsage(_)) => {}
                    Ok(AgentEvent::TurnAborted { code, message }) => {
                        return Err(anyhow::anyhow!(
                            "turn aborted ({}): {message}",
                            code.wire_code()
                        ));
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

            let stopped = responses.iter().any(|m| {
                matches!(m.content.first(), Some(MessageContent::Text(t)) if
                    t.text.contains("past my per-turn limit of 3"))
            });
            assert!(
                stopped,
                "expected the per-turn tool-call limit message; responses did not contain it"
            );
            Ok(())
        }
    }

    // BR-35: the per-reply budget. `max_turns` / `max_tool_calls` bound how many
    // steps a reply takes; this bounds what it *spends*. The token axis is the
    // one a mock provider can drive deterministically (the clock axis is the same
    // code path with a different number, and is unit-tested in `agents::budget`).
    #[cfg(test)]
    mod reply_budget_tests {
        use super::*;
        use async_trait::async_trait;
        use biorouter::agents::budget::ReplyBudget;
        use biorouter::agents::SessionConfig;
        use biorouter::conversation::message::{Message, MessageContent};
        use biorouter::model::ModelConfig;
        use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
        use biorouter::providers::errors::ProviderError;
        use biorouter::session::session_manager::SessionType;
        use rmcp::model::{CallToolRequestParams, Tool};
        use rmcp::object;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Always calls a tool (with fresh args, so no repetition guard fires) and
        /// reports 100 tokens a turn — an agent that would otherwise happily run to
        /// the iteration cap while burning tokens the whole way.
        struct MockSpendyProvider {
            counter: AtomicUsize,
        }

        #[async_trait]
        impl Provider for MockSpendyProvider {
            async fn complete(
                &self,
                _system_prompt: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                let n = self.counter.fetch_add(1, Ordering::SeqCst);
                let call = CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "test_tool".into(),
                    arguments: Some(object!({ "param": format!("value-{n}") })),
                };
                let message = Message::assistant().with_tool_request(format!("call_{n}"), Ok(call));
                let usage = ProviderUsage::new(
                    "mock-model".to_string(),
                    Usage::new(Some(80), Some(20), Some(100)),
                );
                Ok((message, usage))
            }

            async fn complete_with_model(
                &self,
                _model_config: &ModelConfig,
                system_prompt: &str,
                messages: &[Message],
                tools: &[Tool],
            ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
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
                "mock-spendy"
            }
        }

        async fn drain(agent: &Agent, session_config: SessionConfig) -> Result<Vec<Message>> {
            let reply_stream = agent
                .reply(Message::user().with_text("Hello"), session_config, None)
                .await?;
            tokio::pin!(reply_stream);

            let mut responses = Vec::new();
            while let Some(event) = reply_stream.next().await {
                match event? {
                    // BR-52: token-state pings carry no reply content.
                    AgentEvent::TokenUsage(_) => {}
                    AgentEvent::Message(response) => {
                        if let Some(MessageContent::ActionRequired(action)) =
                            response.content.first()
                        {
                            if let biorouter::conversation::message::ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                                agent.handle_confirmation(
                                    id.clone(),
                                    biorouter::permission::PermissionConfirmation {
                                        principal_type: biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                        permission: biorouter::permission::Permission::AllowOnce,
                                    }
                                ).await;
                            }
                        }
                        responses.push(response);
                    }
                    AgentEvent::McpNotification(_)
                    | AgentEvent::ToolCallPending(_)
                    | AgentEvent::MessagesPersisted(_)
                    | AgentEvent::ModelChange { .. }
                    | AgentEvent::PrivacyProviderPinned { .. }
                    | AgentEvent::HistoryReplaced(_) => {}
                    AgentEvent::TurnAborted { code, message } => {
                        return Err(anyhow::anyhow!(
                            "turn aborted ({}): {message}",
                            code.wire_code()
                        ));
                    }
                }
            }
            Ok(responses)
        }

        async fn spendy_agent(name: &str) -> Result<(Agent, String)> {
            let agent = Agent::new();
            let session = agent
                .config
                .session_manager
                .create_session(PathBuf::default(), name.to_string(), SessionType::Hidden)
                .await?;
            agent
                .update_provider(
                    Arc::new(MockSpendyProvider {
                        counter: AtomicUsize::new(0),
                    }),
                    &session.id,
                )
                .await?;
            Ok((agent, session.id))
        }

        #[tokio::test]
        async fn a_reply_that_blows_its_token_budget_is_stopped_and_says_so() -> Result<()> {
            let (agent, session_id) = spendy_agent("reply-budget-test").await?;

            let responses = drain(
                &agent,
                SessionConfig {
                    id: session_id.clone(),
                    schedule_id: None,
                    // Deliberately far out of reach: the *budget*, not the
                    // iteration cap, has to be what ends this reply.
                    max_turns: Some(100),
                    max_tool_calls: None,
                    budget: Some(ReplyBudget {
                        max_tokens: Some(150),
                        ..Default::default()
                    }),
                    retry_config: None,
                    reasoning_effort: None,
                },
            )
            .await?;

            let text = |m: &Message| m.as_concat_text();
            assert!(
                responses
                    .iter()
                    .any(|m| text(m).contains("I've reached the budget for this reply")),
                "expected an honest budget stop; got: {:?}",
                responses.iter().map(text).collect::<Vec<_>>()
            );
            // The meter is a system notification, not prose, so it is matched on
            // the content variant rather than the message text.
            assert!(
                responses.iter().any(|m| m.content.iter().any(|c| matches!(
                    c,
                    MessageContent::SystemNotification(n) if n.msg.contains("Budget reached")
                ))),
                "expected the user to be told the budget was reached before the stop"
            );

            // Graceful, not a kill: the model is asked in-context to wrap up (with
            // the numbers, so it can size its answer) before the turn is ended.
            let session = agent
                .config
                .session_manager
                .get_session(&session_id, true)
                .await?;
            let conversation = session.conversation.expect("conversation");
            assert!(
                conversation
                    .messages()
                    .iter()
                    .any(|m| m.as_concat_text().contains("[budget] This reply has used")),
                "the wrap-up instruction must reach the model, not just the user"
            );
            Ok(())
        }

        #[tokio::test]
        async fn no_budget_means_the_old_behaviour_exactly() -> Result<()> {
            let (agent, session_id) = spendy_agent("no-budget-test").await?;

            let responses = drain(
                &agent,
                SessionConfig {
                    id: session_id,
                    schedule_id: None,
                    // Low turn cap so the test is quick: with no budget, the
                    // iteration cap is what must stop it.
                    max_turns: Some(3),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
            )
            .await?;

            let text = |m: &Message| m.as_concat_text();
            assert!(
                responses
                    .iter()
                    .any(|m| text(m).contains("I've reached my action limit for this turn")),
                "an unbudgeted reply must still stop on the iteration cap"
            );
            assert!(
                !responses.iter().any(|m| text(m).contains("budget")),
                "an unbudgeted reply must never mention a budget: {:?}",
                responses.iter().map(text).collect::<Vec<_>>()
            );
            Ok(())
        }
    }

    #[cfg(test)]
    mod extension_manager_tests {
        use super::*;
        use biorouter::agents::extension::ExtensionConfig;
        use biorouter::agents::extension_manager_extension::{
            MANAGE_EXTENSIONS_TOOL_NAME, SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME,
        };
        use biorouter::agents::AgentConfig;
        use biorouter::config::permission::PermissionManager;
        use biorouter::config::BioRouterMode;
        use biorouter::session::SessionManager;

        async fn setup_agent_with_extension_manager() -> Agent {
            // Add the TODO extension to the config so it can be discovered by search_available_extensions
            // Set it as disabled initially so tests can enable it
            let todo_extension_entry = ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::Platform {
                    name: "todo".to_string(),
                    description: "Keep a running checklist through a multi-step task, so \
                                  Biorouter tracks what is done and what is left"
                        .to_string(),
                    bundled: Some(true),
                    available_tools: vec![],
                },
            };
            set_extension(todo_extension_entry);

            // Create agent with session_id from the start
            let temp_dir = tempfile::tempdir().unwrap();
            let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
            let config = AgentConfig::new(
                session_manager,
                PermissionManager::instance(),
                None,
                BioRouterMode::Auto,
            );

            let agent = Agent::with_config(config);

            // Now add the extension manager platform extension
            let ext_config = ExtensionConfig::Platform {
                name: "extensionmanager".to_string(),
                description: "Extension Manager".to_string(),
                bundled: Some(true),
                available_tools: vec![],
            };

            agent
                .add_extension(ext_config)
                .await
                .expect("Failed to add extension manager");
            agent
        }

        #[tokio::test]
        async fn test_extension_manager_tools_available() {
            let agent = setup_agent_with_extension_manager().await;
            let tools = agent.list_tools("test-session-id", None).await;

            // Note: Tool names are prefixed with the normalized extension name "extensionmanager"
            // not the display name "Extension Manager"
            let search_tool = tools.iter().find(|tool| {
                tool.name == format!("extensionmanager__{SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME}")
            });
            assert!(
                search_tool.is_some(),
                "search_available_extensions tool should be available"
            );

            let manage_tool = tools.iter().find(|tool| {
                tool.name == format!("extensionmanager__{MANAGE_EXTENSIONS_TOOL_NAME}")
            });
            assert!(
                manage_tool.is_some(),
                "manage_extensions tool should be available"
            );
        }
    }

    // BR-63: `/effort quick|normal|deep` — the slash-flag half of the
    // reasoning-effort control. Sticky per session; unknown values are refused
    // rather than silently ignored.
    #[cfg(test)]
    mod effort_command_tests {
        use super::*;
        use biorouter::agents::{ReasoningEffort, SessionConfig};

        fn session_config(id: &str) -> SessionConfig {
            SessionConfig {
                id: id.to_string(),
                schedule_id: None,
                max_turns: None,
                max_tool_calls: None,
                retry_config: None,
                budget: None,
                reasoning_effort: None,
            }
        }

        fn notification_text(message: &biorouter::conversation::message::Message) -> String {
            message
                .content
                .iter()
                .filter_map(|c| c.as_system_notification())
                .map(|n| n.msg.clone())
                .collect::<Vec<_>>()
                .join("\n")
        }

        #[tokio::test]
        async fn effort_command_sets_reports_and_clears_the_session_effort() {
            let agent = Agent::new();
            let config = session_config("effort-cmd-session");

            // Default: nothing set, so the loop behaves exactly as before.
            assert_eq!(
                agent.reasoning_effort("effort-cmd-session").await,
                ReasoningEffort::Normal
            );

            let reply = agent
                .execute_command("/effort deep", &config)
                .await
                .unwrap()
                .expect("/effort is handled as a command");
            assert!(notification_text(&reply).contains("deep"));
            assert_eq!(
                agent.reasoning_effort("effort-cmd-session").await,
                ReasoningEffort::Deep
            );

            // A typo must not silently reset the level.
            let reply = agent
                .execute_command("/effort sideways", &config)
                .await
                .unwrap()
                .expect("unknown levels still answer the user");
            assert!(notification_text(&reply).contains("Unknown effort"));
            assert_eq!(
                agent.reasoning_effort("effort-cmd-session").await,
                ReasoningEffort::Deep
            );

            // No argument reports the current level without changing it.
            let reply = agent
                .execute_command("/effort", &config)
                .await
                .unwrap()
                .expect("bare /effort reports");
            assert!(notification_text(&reply).contains("deep"));
            assert_eq!(
                agent.reasoning_effort("effort-cmd-session").await,
                ReasoningEffort::Deep
            );

            // Back to the default.
            agent
                .execute_command("/effort normal", &config)
                .await
                .unwrap()
                .expect("normal is a level too");
            assert_eq!(
                agent.reasoning_effort("effort-cmd-session").await,
                ReasoningEffort::Normal
            );
        }

        #[tokio::test]
        async fn effort_is_per_session() {
            let agent = Agent::new();
            agent
                .execute_command("/effort quick", &session_config("session-a"))
                .await
                .unwrap();

            assert_eq!(
                agent.reasoning_effort("session-a").await,
                ReasoningEffort::Quick
            );
            assert_eq!(
                agent.reasoning_effort("session-b").await,
                ReasoningEffort::Normal
            );
        }
    }

    /// BR-52: the agent carries the token state it just wrote in the event
    /// stream, so consumers never have to re-read SQLite per streamed token.
    #[cfg(test)]
    mod token_state_tests {
        use super::*;
        use async_trait::async_trait;
        use biorouter::agents::{AgentConfig, SessionConfig};
        use biorouter::config::permission::PermissionManager;
        use biorouter::config::BioRouterMode;
        use biorouter::conversation::message::Message;
        use biorouter::model::ModelConfig;
        use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
        use biorouter::providers::errors::ProviderError;
        use biorouter::session::session_manager::SessionType;
        use biorouter::session::SessionManager;
        use rmcp::model::Tool;
        use std::path::PathBuf;

        /// Answers with plain text (no tool calls, so the loop ends after one
        /// turn) and reports a known usage.
        struct MockTextProvider;

        #[async_trait]
        impl Provider for MockTextProvider {
            async fn complete(
                &self,
                _system_prompt: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                Ok((
                    Message::assistant().with_text("done"),
                    ProviderUsage::new(
                        "mock-model".to_string(),
                        Usage::new(Some(10), Some(5), Some(15)),
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
                "mock-text"
            }
        }

        #[tokio::test]
        async fn reply_stream_carries_the_recorded_token_state() -> Result<()> {
            let temp_dir = tempfile::tempdir().unwrap();
            let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
            let agent = Agent::with_config(AgentConfig::new(
                session_manager.clone(),
                PermissionManager::instance(),
                None,
                BioRouterMode::Auto,
            ));

            let session = session_manager
                .create_session(
                    PathBuf::default(),
                    "token-state-test".to_string(),
                    SessionType::Hidden,
                )
                .await?;

            agent
                .update_provider(Arc::new(MockTextProvider), &session.id)
                .await?;

            let session_config = SessionConfig {
                id: session.id.clone(),
                schedule_id: None,
                max_turns: Some(1),
                max_tool_calls: None,
                retry_config: None,
                budget: None,
                reasoning_effort: None,
            };

            let stream = agent
                .reply(Message::user().with_text("hi"), session_config, None)
                .await?;
            tokio::pin!(stream);

            let mut token_states = Vec::new();
            while let Some(event) = stream.next().await {
                if let AgentEvent::TokenUsage(state) = event? {
                    token_states.push(state);
                }
            }

            let state = token_states
                .last()
                .expect("the agent must emit its token state once the turn's usage is recorded");

            // The live gauge is this turn's usage...
            assert_eq!(state.input_tokens, 10);
            assert_eq!(state.output_tokens, 5);
            assert_eq!(state.total_tokens, 15);
            // ...and the lifetime counters agree with what was persisted, so a
            // consumer that trusts the carried state never drifts from the store.
            assert_eq!(state.accumulated_input_tokens, 10);
            assert_eq!(state.accumulated_output_tokens, 5);
            assert_eq!(state.accumulated_total_tokens, 15);

            let counts = session_manager.get_token_counts(&session.id).await?;
            assert_eq!(counts.total_tokens, Some(state.total_tokens));
            assert_eq!(
                counts.accumulated_total_tokens,
                Some(state.accumulated_total_tokens)
            );

            Ok(())
        }
    }
}
