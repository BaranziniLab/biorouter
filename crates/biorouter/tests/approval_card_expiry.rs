//! An approval card whose window closes unanswered must say so to every
//! surface that drew it, through the real reply loop.
//!
//! The bug (found live by the decisions lane): a tool-permission card kept its
//! Allow / Deny buttons after the daemon stopped waiting for it, so a late
//! click answered nothing (`POST /action-required/tool-confirmation` said
//! `unknown`) and the card collapsed to "no longer available". The loop now
//! follows the card with a notice that names it, so a surface can draw the card
//! as expired instead.
//!
//! Its own binary because the agent's prompt window is read from
//! `BIOROUTER_CONFIRMATION_TIMEOUT_SECS`, a process-global variable: set once,
//! before `main`, it shortens the window for this binary and nothing else.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, ConfirmationOutcome, SessionConfig};
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

/// Sandbox the config root (issue #54) and shorten the agent's permission
/// prompt to one second, before any test thread exists.
#[ctor::ctor]
fn one_second_approval_window() {
    std::env::set_var("BIOROUTER_CONFIRMATION_TIMEOUT_SECS", "1");
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

/// Asks for one shell call, then answers in text.
struct ShellOnce {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for ShellOnce {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let call = CallToolRequestParams {
                task: None,
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "echo synthetic"})),
            };
            return Ok((
                Message::assistant().with_tool_request("call_1", Ok(call)),
                usage,
            ));
        }
        Ok((Message::assistant().with_text("done"), usage))
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

fn card_id(message: &Message) -> Option<String> {
    message.content.iter().find_map(|content| match content {
        MessageContent::ActionRequired(action) => match &action.data {
            ActionRequiredData::ToolConfirmation { id, .. } => Some(id.clone()),
            _ => None,
        },
        _ => None,
    })
}

#[tokio::test]
async fn an_unanswered_permission_card_is_followed_by_its_expiry() -> Result<()> {
    let data_dir = TempDir::new()?;
    let work_dir = TempDir::new()?;
    let permission_dir = TempDir::new()?;
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    // A per-test permission store: the `instance()` singleton reads a real
    // `permission.yaml`, where an `always_allow` for the shell would skip the
    // card this test is about.
    let agent = Agent::with_config(AgentConfig::new(
        session_manager.clone(),
        Arc::new(PermissionManager::new(permission_dir.path().to_path_buf())),
        None,
        BioRouterMode::Approve,
    ));
    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "approval-expiry".to_string(),
            SessionType::Hidden,
        )
        .await?;
    agent
        .update_provider(
            Arc::new(ShellOnce {
                calls: AtomicUsize::new(0),
            }),
            &session.id,
        )
        .await?;

    let stream = agent
        .reply(
            Message::user().with_text("run a command"),
            SessionConfig {
                id: session.id.clone(),
                schedule_id: None,
                max_turns: Some(4),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await?;
    tokio::pin!(stream);
    // Nobody answers: the card's one-second window closes on its own.
    let mut yielded = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event? {
            yielded.push(message);
        }
    }

    let card_at = yielded
        .iter()
        .position(|message| card_id(message).is_some())
        .expect("Approve mode shows a permission card");
    let id = card_id(&yielded[card_at]).unwrap();
    let notice_at = yielded
        .iter()
        .position(|message| {
            biorouter::pending_user_action::expired_card_id(message) == Some(id.as_str())
        })
        .unwrap_or_else(|| panic!("no expiry named card {id}: {yielded:#?}"));
    assert!(card_at < notice_at, "the expiry follows the card it names");
    let notice = &yielded[notice_at];
    assert!(notice.metadata.user_visible && !notice.metadata.agent_visible);
    assert!(biorouter::pending_user_action::is_card_expiry_notice(
        notice
    ));

    // A click after the window closed lands nowhere.
    assert!(matches!(
        agent
            .handle_confirmation_for_session(
                &session.id,
                id.clone(),
                biorouter::permission::PermissionConfirmation {
                    principal_type:
                        biorouter::permission::permission_confirmation::PrincipalType::Tool,
                    permission: biorouter::permission::Permission::AllowOnce,
                },
                biorouter::pending_user_action::DecisionAuthority::unproven(),
            )
            .await,
        ConfirmationOutcome::Unknown
    ));

    // The notice lives where the card does, the stream and the turn's replay
    // log, and is never stored; the stored record is the tool response.
    let stored = session_manager
        .get_session(&session.id, true)
        .await?
        .conversation
        .expect("the turn was stored");
    assert!(
        !stored.messages().iter().any(|message| {
            biorouter::pending_user_action::is_card_expiry_notice(message)
                || card_id(message).is_some()
        }),
        "neither the card nor its expiry is stored"
    );
    assert!(
        stored.messages().iter().any(|message| message
            .content
            .iter()
            .any(|content| matches!(content, MessageContent::ToolResponse(response)
                if format!("{:?}", response.tool_result).contains("expired before the user answered")))),
        "the tool response records the expiry"
    );
    Ok(())
}
