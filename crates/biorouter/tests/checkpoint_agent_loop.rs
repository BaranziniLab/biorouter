//! BR-43 agent-integration test: drive the *real* `Agent::reply` loop with a
//! mock provider and assert the turn-boundary checkpoint wiring fires and that
//! `CheckpointManager::restore` rewinds both the work-tree and the conversation.
//!
//! Mirrors the `ScriptedProvider` pattern in `hooks_agent_loop_tests.rs`. Runs
//! serially because it toggles process-global env (`BIOROUTER_CHECKPOINTS`,
//! `BIOROUTER_PATH_ROOT`) that the checkpoint config + data-dir resolution read.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::checkpoint::{CheckpointKind, RestoreAxis};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::object;
use tempfile::TempDir;

/// Emits a `developer__shell` tool call on the first turn (so the loop runs a
/// tool and hits the post-step checkpoint branch), then plain text.
struct ScriptedProvider {
    calls: AtomicUsize,
}

impl ScriptedProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if n == 0 {
            let tool_call = CallToolRequestParams {
                task: None,
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "echo hi"})),
            };
            let message = Message::assistant().with_tool_request("call_1", Ok(tool_call));
            return Ok((message, usage));
        }
        Ok((Message::assistant().with_text("all done"), usage))
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

async fn drain(agent: &Agent, user: &str, session_id: &str) -> Result<Vec<Message>> {
    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(8),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text(user), session_config, None)
        .await?;
    tokio::pin!(stream);
    let mut out = Vec::new();
    while let Some(ev) = stream.next().await {
        if let AgentEvent::Message(m) = ev? {
            if let Some(MessageContent::ActionRequired(action)) = m.content.first() {
                if let ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                    agent
                        .handle_confirmation(
                            id.clone(),
                            biorouter::permission::PermissionConfirmation {
                                principal_type:
                                    biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                permission: biorouter::permission::Permission::AllowOnce,
                            },
                        )
                        .await;
                }
            }
            out.push(m);
        }
    }
    Ok(out)
}

/// Pin the process-global env this file's tests drive for as long as the
/// returned guard lives.
///
/// Both variables are read on every `Paths::*` / checkpoint-config lookup, from
/// anywhere in the process. `#[serial]` orders this test against other
/// `#[serial]` tests only, and a bare `set_var`/`remove_var` pair leaks the
/// value on any early return or panic. The `env_lock` guard restores the
/// previous values from `Drop` and holds the same mutex the rest of the
/// workspace's `BIOROUTER_PATH_ROOT` tests take.
///
/// Values are owned `String`s so the guard does not borrow the caller's
/// `TempDir`.
fn scoped_checkpoint_env(path_root: &Path) -> env_lock::EnvGuard<'static> {
    env_lock::lock_env([
        (
            "BIOROUTER_PATH_ROOT",
            Some(path_root.to_string_lossy().into_owned()),
        ),
        ("BIOROUTER_CHECKPOINTS", Some("1".to_string())),
    ])
}

/// The loop test below pins `BIOROUTER_PATH_ROOT` and `BIOROUTER_CHECKPOINTS`
/// for its whole body. Both are process-global and read on every `Paths::*`
/// call, so if the test panics part-way — which is exactly what a failing
/// assertion does — the scratch root must not outlive the unwind: later tests
/// would resolve into a `TempDir` that has already been dropped.
///
/// The panic below is caught, so the stderr backtrace it prints is expected.
#[test]
#[serial_test::serial]
fn checkpoint_env_is_restored_when_the_loop_test_panics() {
    let before_root = std::env::var("BIOROUTER_PATH_ROOT").ok();
    let before_checkpoints = std::env::var("BIOROUTER_CHECKPOINTS").ok();
    let path_root = TempDir::new().unwrap();

    let outcome = std::panic::catch_unwind(|| {
        let _env = scoped_checkpoint_env(path_root.path());
        assert_eq!(
            std::env::var("BIOROUTER_PATH_ROOT").ok().as_deref(),
            path_root.path().to_str(),
            "the guard must actually pin the variable"
        );
        panic!("simulated assertion failure inside the guarded region");
    });

    assert!(outcome.is_err(), "the panic must propagate");
    assert_eq!(
        std::env::var("BIOROUTER_PATH_ROOT").ok(),
        before_root,
        "BIOROUTER_PATH_ROOT leaked out of a panicking checkpoint test"
    );
    assert_eq!(
        std::env::var("BIOROUTER_CHECKPOINTS").ok(),
        before_checkpoints,
        "BIOROUTER_CHECKPOINTS leaked out of a panicking checkpoint test"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn reply_loop_checkpoints_and_restore_rewinds_both_axes() {
    // Hermetic env: shadow repos under a temp data root, checkpoints enabled.
    // The guard holds the process-wide env lock for the whole test and restores
    // both variables on drop, including when an assertion below panics.
    let path_root = TempDir::new().unwrap();
    let _env = scoped_checkpoint_env(path_root.path());

    let work_dir = TempDir::new().unwrap();
    std::fs::write(work_dir.path().join("data.txt"), "v1").unwrap();

    let data_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    let config = AgentConfig::new(
        session_manager.clone(),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    );
    let agent = Agent::with_config(config);
    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "developer".to_string(),
            display_name: Some("Developer".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add developer extension");

    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "br43-loop".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    agent
        .update_provider(Arc::new(ScriptedProvider::new()), &session.id)
        .await
        .unwrap();

    // Drive one real reply turn.
    let msgs = drain(&agent, "please act", &session.id).await.unwrap();
    assert!(!msgs.is_empty(), "reply produced messages");

    // The turn-boundary wiring created at least the pre-turn checkpoint.
    let checkpoints = session_manager.list_checkpoints(&session.id).await.unwrap();
    assert!(
        !checkpoints.is_empty(),
        "real reply loop must produce a checkpoint"
    );
    let pre = checkpoints
        .iter()
        .find(|c| c.kind == CheckpointKind::PreStep)
        .expect("a pre_step checkpoint anchored at the turn");
    assert!(!pre.commit_sha.is_empty(), "shadow commit sha recorded");

    let msg_count_before = session_manager
        .get_session(&session.id, true)
        .await
        .unwrap()
        .message_count;
    assert!(msg_count_before > 0);

    // Simulate later edits on top of that checkpoint.
    std::fs::write(work_dir.path().join("data.txt"), "v2-edited").unwrap();
    std::fs::write(work_dir.path().join("later.txt"), "created after").unwrap();

    // Rewind files + conversation to the pre-turn checkpoint.
    let outcome = agent
        .checkpoints()
        .expect("checkpoints enabled")
        .restore(&session.id, &pre.id, RestoreAxis::Both, work_dir.path())
        .await
        .unwrap();

    // Files axis: original restored, post-checkpoint file removed.
    assert_eq!(
        std::fs::read_to_string(work_dir.path().join("data.txt")).unwrap(),
        "v1"
    );
    assert!(
        !work_dir.path().join("later.txt").exists(),
        "file created after the checkpoint must be removed on restore"
    );
    assert!(
        outcome.pre_restore_checkpoint_id.is_some(),
        "restore is reversible"
    );
    assert_eq!(outcome.truncated_from_ts, Some(pre.anchor_ts));

    // Conversation axis: the turn's messages were truncated.
    let msg_count_after = session_manager
        .get_session(&session.id, true)
        .await
        .unwrap()
        .message_count;
    assert!(
        msg_count_after < msg_count_before,
        "conversation must be truncated back past the turn (before={msg_count_before}, after={msg_count_after})"
    );
}
