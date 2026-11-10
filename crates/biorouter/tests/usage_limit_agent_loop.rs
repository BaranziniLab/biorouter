//! Usage limits in the reply loop: a reached limit ends the turn BEFORE the
//! next model request, a message to a paused chat makes no model request,
//! lifting the limit resumes the chat, and the monthly limit pauses every chat.
//! See `docs/agent-loop/usage-limits.md`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::usage_limit::{
    self, ChatLimitOverride, CHAT_LIMIT_REACHED_PREFIX, CHAT_LIMIT_WARNING_PREFIX,
    MONTH_LIMIT_REACHED_PREFIX,
};
use biorouter::agents::{Agent, AgentEvent, SessionConfig};
use biorouter::config::Config;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SideUsageEntry;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::object;

/// The config root of this binary is a scratch directory, so the monthly limit
/// this file writes never reaches the developer's own config.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = tempfile::TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

/// The month figures are over every chat in the binary's store, and the
/// monthly test writes the global config, so the tests here run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Always calls a tool (fresh arguments, so no repetition guard fires) and
/// reports 100 billed tokens a call.
struct MockSpendyProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for MockSpendyProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
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
        "mock-spendy"
    }
}

async fn spendy_agent(name: &str) -> Result<(Agent, String, Arc<AtomicUsize>)> {
    let agent = Agent::new();
    let session = agent
        .config
        .session_manager
        .create_session(PathBuf::default(), name.to_string(), SessionType::User)
        .await?;
    let calls = Arc::new(AtomicUsize::new(0));
    agent
        .update_provider(
            Arc::new(MockSpendyProvider {
                calls: Arc::clone(&calls),
            }),
            &session.id,
        )
        .await?;
    Ok((agent, session.id, calls))
}

fn config(id: &str, max_turns: u32) -> SessionConfig {
    SessionConfig {
        id: id.to_string(),
        schedule_id: None,
        max_turns: Some(max_turns),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
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
            AgentEvent::Message(response) => {
                if let Some(MessageContent::ActionRequired(action)) = response.content.first() {
                    if let biorouter::conversation::message::ActionRequiredData::ToolConfirmation {
                        id,
                        ..
                    } = &action.data
                    {
                        agent
                            .handle_confirmation(
                                id.clone(),
                                biorouter::permission::PermissionConfirmation {
                                    principal_type: biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                    permission: biorouter::permission::Permission::AllowOnce,
                                },
                            )
                            .await;
                    }
                }
                responses.push(response);
            }
            AgentEvent::TurnAborted { code, message } => {
                return Err(anyhow::anyhow!(
                    "turn aborted ({}): {message}",
                    code.wire_code()
                ));
            }
            _ => {}
        }
    }
    Ok(responses)
}

fn notices(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::SystemNotification(n) => Some(n.msg.clone()),
            _ => None,
        })
        .collect()
}

async fn stored_notices(agent: &Agent, session_id: &str) -> Result<Vec<String>> {
    let session = agent
        .config
        .session_manager
        .get_session(session_id, true)
        .await?;
    Ok(notices(
        session.conversation.expect("conversation").messages(),
    ))
}

async fn seed_spend(agent: &Agent, session_id: &str, key: &str, tokens: i64) -> Result<()> {
    agent
        .config
        .session_manager
        .apply_side_usage_event(SideUsageEntry {
            event_key: key.to_string(),
            session_id: session_id.to_string(),
            provider: "mock-spendy".to_string(),
            model_id: "mock-model".to_string(),
            input_tokens: Some(tokens),
            output_tokens: Some(0),
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_reached_chat_limit_stops_before_the_next_model_request() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-stop").await?;
    // 100 tokens a call: the first call takes the chat to 83% (a warning), the
    // second to 166%, and no third request may be made.
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(120),
            max_usd: None,
        }),
    )
    .await?;

    let responses = drain(&agent, config(&id, 100)).await?;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the limit must stop the turn before a third model request"
    );
    let live = notices(&responses);
    assert_eq!(
        live.iter()
            .filter(|n| n.starts_with(CHAT_LIMIT_WARNING_PREFIX))
            .count(),
        1,
        "one warning per reply: {live:?}"
    );
    let stop = live
        .iter()
        .find(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX))
        .unwrap_or_else(|| panic!("no stop notice: {live:?}"));
    assert!(stop.contains("200 tokens used"), "{stop}");
    assert!(stop.contains("120 tokens"), "{stop}");
    // No wrap-up call and no BR-35 budget dance.
    assert!(
        !live.iter().any(|n| n.contains("Budget reached")),
        "{live:?}"
    );

    // Durable: the stop notice is stored with the chat.
    assert!(
        stored_notices(&agent, &id)
            .await?
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "the stop notice must survive a reload"
    );
    Ok(())
}

#[tokio::test]
async fn a_message_to_a_paused_chat_is_stored_and_makes_no_model_request() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-paused").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(50),
            max_usd: None,
        }),
    )
    .await?;
    seed_spend(&agent, &id, "paused:1", 60).await?;

    let responses = drain(&agent, config(&id, 3)).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no model request");
    assert!(
        notices(&responses)
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "the paused chat says why"
    );
    let session = agent.config.session_manager.get_session(&id, true).await?;
    let conversation = session.conversation.expect("conversation");
    assert!(
        conversation
            .messages()
            .iter()
            .any(|m| m.as_concat_text() == "Hello"),
        "the person's message is stored"
    );
    assert!(stored_notices(&agent, &id)
        .await?
        .iter()
        .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)));

    // Lifting the limit for this chat resumes it.
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Unlimited),
    )
    .await?;
    let responses = drain(&agent, config(&id, 2)).await?;
    assert!(calls.load(Ordering::SeqCst) > 0, "a lifted limit resumes");
    assert!(!notices(&responses)
        .iter()
        .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)));
    Ok(())
}

#[tokio::test]
async fn the_monthly_limit_pauses_a_chat_that_lifted_its_own_limit() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-month").await?;
    // Even an explicit per-chat "unlimited" does not lift the month.
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Unlimited),
    )
    .await?;
    let month = agent
        .config
        .session_manager
        .spend_snapshot(&id, false)
        .await?;
    Config::global().set_param(
        "usage",
        serde_json::json!({ "monthly_token_limit": month.month_tokens + 10 }),
    )?;
    seed_spend(&agent, &id, "month:1", 20).await?;

    let result = drain(&agent, config(&id, 3)).await;
    Config::global().delete("usage")?;
    let responses = result?;
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no model request");
    let live = notices(&responses);
    let stop = live
        .iter()
        .find(|n| n.starts_with(MONTH_LIMIT_REACHED_PREFIX))
        .unwrap_or_else(|| panic!("no monthly stop notice: {live:?}"));
    assert!(stop.contains("Settings > App > Usage"), "{stop}");
    Ok(())
}
