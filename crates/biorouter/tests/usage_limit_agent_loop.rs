//! Usage limits in the reply loop: a reached limit, or one the next request is
//! estimated to pass, ends the turn BEFORE that model request; the turn that
//! reaches a limit stores why; the near-limit warning is stored once per
//! crossing; a message to a paused chat makes no model request, lifting the
//! limit resumes the chat, and the monthly limit pauses every chat.
//! See `docs/agent-loop/usage-limits.md`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

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
use biorouter::session::session_manager::{SessionType, UsageLedgerEntry};
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
/// reports 100 billed tokens a call, or answers in text with `text_usage`
/// billed tokens (80% input) and so ends the turn.
struct MockSpendyProvider {
    calls: Arc<AtomicUsize>,
    text_usage: Option<i32>,
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
        if let Some(total) = self.text_usage {
            let input = total * 4 / 5;
            let usage = ProviderUsage::new(
                "mock-model".to_string(),
                Usage::new(Some(input), Some(total - input), Some(total)),
            );
            return Ok((Message::assistant().with_text("Done."), usage));
        }
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
    spendy_agent_with(name, None).await
}

async fn spendy_agent_with(
    name: &str,
    text_usage: Option<i32>,
) -> Result<(Agent, String, Arc<AtomicUsize>)> {
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
                text_usage,
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
    drain_with(agent, session_config, "Hello").await
}

async fn drain_with(
    agent: &Agent,
    session_config: SessionConfig,
    text: &str,
) -> Result<Vec<Message>> {
    let reply_stream = agent
        .reply(Message::user().with_text(text), session_config, None)
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

/// A turn request the chat made earlier, so the next one is estimated from it
/// (80 tokens in, 20 out, the mock's own shape) rather than from the size of
/// the first prompt.
async fn seed_request(agent: &Agent, session_id: &str, key: &str) -> Result<()> {
    agent
        .config
        .session_manager
        .apply_usage_event(UsageLedgerEntry {
            event_key: key.to_string(),
            session_id: session_id.to_string(),
            schedule_id: None,
            current_total_tokens: Some(100),
            current_input_tokens: Some(80),
            current_output_tokens: Some(20),
            billed_total_tokens: Some(100),
            input_tokens: Some(80),
            output_tokens: Some(20),
            model_id: Some("mock-model".to_string()),
            provider: Some("mock-spendy".to_string()),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
        })
        .await?;
    Ok(())
}

async fn chat_tokens(agent: &Agent, session_id: &str) -> Result<i64> {
    Ok(agent
        .config
        .session_manager
        .spend_snapshot(session_id, false)
        .await?
        .chat_tokens)
}

#[tokio::test]
async fn a_request_that_would_pass_the_chat_limit_is_not_made() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-stop").await?;
    // 100 tokens a call from 100: the requests run while the next one, about
    // 100 tokens, still fits under 450, and none runs past it.
    seed_request(&agent, &id, "stop:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(450),
            max_usd: None,
        }),
    )
    .await?;

    let responses = drain(&agent, config(&id, 100)).await?;
    assert!(
        agent.usage_limit_interrupted_reply(),
        "the gate cut the reply short"
    );
    let used = chat_tokens(&agent, &id).await?;
    assert!(used <= 450, "no request runs past the limit: {used} used");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "three requests fit under the limit, the fourth would pass it"
    );
    let live = notices(&responses);
    let stop = live
        .iter()
        .find(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX))
        .unwrap_or_else(|| panic!("no stop notice: {live:?}"));
    assert!(stop.contains("400 tokens used"), "{stop}");
    assert!(stop.contains("of its limit of 450 tokens"), "{stop}");
    assert!(stop.contains("The next request is estimated at"), "{stop}");
    assert!(stop.contains("50 tokens is left"), "{stop}");
    // No wrap-up call and no BR-35 budget dance.
    assert!(
        !live.iter().any(|n| n.contains("Budget reached")),
        "{live:?}"
    );

    // Durable: the stop notice is stored with the chat.
    let stored = stored_notices(&agent, &id).await?;
    assert!(
        stored
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "the stop notice must survive a reload: {stored:?}"
    );
    Ok(())
}

#[tokio::test]
async fn the_turn_that_reaches_the_limit_stores_why() -> Result<()> {
    let _serial = SERIAL.lock().await;
    // One request bills far more than the estimate (1,000 against about 120).
    let (agent, id, calls) = spendy_agent_with("usage-limit-turn-end", Some(1_000)).await?;
    seed_request(&agent, &id, "turn-end:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(500),
            max_usd: None,
        }),
    )
    .await?;

    let responses = drain(&agent, config(&id, 10)).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        notices(&responses)
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "the turn says it reached the limit"
    );
    // The turn finished its work; the notice is for the request after it. A
    // scheduled run that ends like this is a success.
    assert!(!agent.usage_limit_interrupted_reply());
    let stored = stored_notices(&agent, &id).await?;
    let stop = stored
        .iter()
        .find(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX))
        .unwrap_or_else(|| panic!("the reason must survive a reload: {stored:?}"));
    assert!(stop.contains("1,100 tokens used"), "{stop}");
    Ok(())
}

#[tokio::test]
async fn the_near_limit_warning_is_stored_once_per_crossing() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent_with("usage-limit-warn-once", Some(100)).await?;
    // 650 billed by a side call (not a request shape) plus one 100-token
    // request: 750 of 1,000, and each reply bills 100 more.
    seed_spend(&agent, &id, "side:seed:1", 650).await?;
    seed_request(&agent, &id, "warn:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(1_000),
            max_usd: None,
        }),
    )
    .await?;

    // 750 to 850: the reply crosses 80%, and its end stores the warning.
    let first = notices(&drain(&agent, config(&id, 10)).await?);
    assert_eq!(
        first
            .iter()
            .filter(|n| n.starts_with(CHAT_LIMIT_WARNING_PREFIX))
            .count(),
        1,
        "{first:?}"
    );
    // 850 to 950: no second warning; the next request (about 120) no longer
    // fits, so this turn's end stores the stop.
    let second = notices(&drain(&agent, config(&id, 10)).await?);
    assert!(
        !second
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_WARNING_PREFIX)),
        "warned once per crossing, not before every reply: {second:?}"
    );
    assert!(
        second
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "{second:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let stored = stored_notices(&agent, &id).await?;
    assert_eq!(
        stored
            .iter()
            .filter(|n| n.starts_with(CHAT_LIMIT_WARNING_PREFIX))
            .count(),
        1,
        "{stored:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_limit_smaller_than_one_request_says_so() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-too-small").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(50),
            max_usd: None,
        }),
    )
    .await?;

    let responses = drain(&agent, config(&id, 3)).await?;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the first request alone would pass the limit"
    );
    let live = notices(&responses);
    let stop = live
        .iter()
        .find(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX))
        .unwrap_or_else(|| panic!("no stop notice: {live:?}"));
    assert!(stop.contains("One request is estimated at"), "{stop}");
    assert!(
        stop.contains("more than the whole limit of 50 tokens"),
        "{stop}"
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
    assert!(agent.usage_limit_interrupted_reply());
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

/// Spends past the chat's limit on its first call and answers it with a
/// context overflow, so the limit is reached between the loop-top check and
/// the overflow arm's compaction. Any later call (a compaction, a judge)
/// answers `{"ok": true}`, and every call is counted.
struct MockOverflowAfterSpend {
    calls: Arc<AtomicUsize>,
    session_manager: Arc<biorouter::session::SessionManager>,
    session_id: std::sync::OnceLock<String>,
}

#[async_trait]
impl Provider for MockOverflowAfterSpend {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            let id = self.session_id.get().expect("session id set").clone();
            self.session_manager
                .apply_side_usage_event(SideUsageEntry {
                    event_key: "overflow:spend".to_string(),
                    session_id: id,
                    provider: "mock-overflow".to_string(),
                    model_id: "mock-model".to_string(),
                    input_tokens: Some(500),
                    output_tokens: Some(0),
                })
                .await
                .map_err(|e| ProviderError::ExecutionError(e.to_string()))?;
            return Err(ProviderError::ContextLengthExceeded(
                "the prompt is too long".to_string(),
            ));
        }
        let usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        Ok((Message::assistant().with_text("{\"ok\": true}"), usage))
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
        MockSpendyProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "mock-overflow"
    }
}

#[tokio::test]
async fn a_limit_reached_at_a_context_overflow_ends_the_turn_there() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let agent = Agent::new();
    let session = agent
        .config
        .session_manager
        .create_session(
            PathBuf::default(),
            "usage-limit-overflow".to_string(),
            SessionType::User,
        )
        .await?;
    let id = session.id.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MockOverflowAfterSpend {
        calls: Arc::clone(&calls),
        session_manager: Arc::clone(&agent.config.session_manager),
        session_id: std::sync::OnceLock::new(),
    };
    provider.session_id.set(id.clone()).unwrap();
    agent.update_provider(Arc::new(provider), &id).await?;
    // From 100 used, the first request (about 120) fits under 300; the 500
    // its call spends does not.
    seed_request(&agent, &id, "overflow:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(300),
            max_usd: None,
        }),
    )
    .await?;

    // Self-critique on: a turn that fell through to the exit gates would make
    // one more model call (the judge) after the limit was reached.
    Config::global().set_param("BIOROUTER_SELF_CRITIQUE", true)?;
    let result = drain(&agent, config(&id, 5)).await;
    Config::global().delete("BIOROUTER_SELF_CRITIQUE")?;
    let responses = result?;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "no compaction and no exit-gate call after the limit is reached"
    );
    let live = notices(&responses);
    assert!(
        live.iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "{live:?}"
    );
    assert!(!live.iter().any(|n| n.contains("Compacting")), "{live:?}");
    Ok(())
}

/// A reply's first request carries the new message, which the previous
/// request plus its answer leaves out: a large paste is counted before it is
/// sent, though the composer and the prologue (which cannot see it) let the
/// message through.
#[tokio::test]
async fn a_large_new_message_is_counted_before_the_first_request() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-paste").await?;
    seed_spend(&agent, &id, "side:paste:1", 15_000).await?;
    seed_request(&agent, &id, "paste:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(20_000),
            max_usd: None,
        }),
    )
    .await?;
    // 15,100 used, and the previous request plus its answer (about 120)
    // fits. The paste is about 10,000 tokens, which does not.
    let paste = "The quick brown fox jumps over the lazy dog. ".repeat(700);
    let responses = drain_with(&agent, config(&id, 3), &paste).await?;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the request carrying the paste would pass the limit"
    );
    let live = notices(&responses);
    let stop = live
        .iter()
        .find(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX))
        .unwrap_or_else(|| panic!("no stop notice: {live:?}"));
    assert!(stop.contains("The next request is estimated at"), "{stop}");
    assert!(stop.contains("4,900 tokens is left"), "{stop}");
    assert_eq!(chat_tokens(&agent, &id).await?, 15_100, "nothing was spent");
    assert!(agent.usage_limit_interrupted_reply());
    Ok(())
}

/// The model calls a turn makes at its end are requests too: once the turn
/// leaves too little for the next request, the self-critique pass does not
/// run, and the turn ends on the stop notice.
#[tokio::test]
async fn the_self_critique_pass_is_not_made_at_a_limit() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent_with("usage-limit-critique", Some(150)).await?;
    seed_request(&agent, &id, "critique:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(300),
            max_usd: None,
        }),
    )
    .await?;
    // From 100 used the answer (about 126 estimated) fits, and bills 150:
    // 250 used, and the next request (about 180) no longer fits.
    Config::global().set_param("BIOROUTER_SELF_CRITIQUE", true)?;
    let result = drain(&agent, config(&id, 5)).await;
    Config::global().delete("BIOROUTER_SELF_CRITIQUE")?;
    let responses = result?;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the answer only, no self-critique judge"
    );
    assert!(
        notices(&responses)
            .iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "{:?}",
        notices(&responses)
    );
    Ok(())
}

/// The same for the `/goal` judge, a prompt Stop hook: it is not asked, and a
/// goal nobody judged is not cleared.
#[tokio::test]
async fn the_goal_judge_is_not_asked_at_a_limit() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent_with("usage-limit-goal", Some(150)).await?;
    seed_request(&agent, &id, "goal:seed").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(300),
            max_usd: None,
        }),
    )
    .await?;
    agent.set_goal(&id, "say hello".to_string()).await;

    let responses = drain(&agent, config(&id, 5)).await?;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the answer only, no goal judge"
    );
    assert!(
        agent.active_goal(&id).await.is_some(),
        "a goal nobody judged stays"
    );
    let live = notices(&responses);
    assert!(
        live.iter()
            .any(|n| n.starts_with("The /goal was not checked")),
        "{live:?}"
    );
    assert!(
        live.iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "{live:?}"
    );
    Ok(())
}

/// `/compact` is a billed model request over the whole history: a chat at its
/// limit is not compacted and gets the paused notice.
#[tokio::test]
async fn compact_at_a_limit_makes_no_model_request() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let (agent, id, calls) = spendy_agent("usage-limit-compact").await?;
    usage_limit::set_for_session(
        &agent.config.session_manager,
        &id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(50),
            max_usd: None,
        }),
    )
    .await?;
    seed_spend(&agent, &id, "compact:1", 60).await?;

    let responses = drain_with(&agent, config(&id, 3), "/compact").await?;
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no summarization request");
    let live = notices(&responses);
    assert!(
        live.iter()
            .any(|n| n.starts_with(CHAT_LIMIT_REACHED_PREFIX)),
        "{live:?}"
    );
    assert!(
        !live.iter().any(|n| n.contains("Compaction complete")),
        "{live:?}"
    );
    Ok(())
}

/// A subagent is held to its parent chat's limit, and its near-limit warning
/// is about that limit: it is stored in the parent's transcript, where the
/// warning is remembered and where the person reads, not in the subagent's.
#[tokio::test]
async fn a_subagents_warning_is_stored_in_its_parent_chat() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let agent = Agent::new();
    let manager = Arc::clone(&agent.config.session_manager);
    let parent = manager
        .create_session(
            PathBuf::default(),
            "usage-limit-parent".to_string(),
            SessionType::User,
        )
        .await?;
    let child = manager
        .create_session(
            PathBuf::default(),
            "usage-limit-child".to_string(),
            SessionType::SubAgent,
        )
        .await?;
    manager
        .update(&child.id)
        .parent_session_id(Some(parent.id.clone()))
        .apply()
        .await?;
    let calls = Arc::new(AtomicUsize::new(0));
    agent
        .update_provider(
            Arc::new(MockSpendyProvider {
                calls: Arc::clone(&calls),
                text_usage: Some(50),
            }),
            &child.id,
        )
        .await?;
    usage_limit::set_for_session(
        &manager,
        &parent.id,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(1_000),
            max_usd: None,
        }),
    )
    .await?;
    // 700 by the parent and 100 by the subagent: 80% of the parent's limit.
    seed_spend(&agent, &parent.id, "parent:spend", 700).await?;
    seed_request(&agent, &child.id, "child:seed").await?;

    let responses = drain(&agent, config(&child.id, 3)).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let warned = |notices: &[String]| {
        notices
            .iter()
            .filter(|n| n.starts_with(CHAT_LIMIT_WARNING_PREFIX))
            .count()
    };
    assert_eq!(
        warned(&stored_notices(&agent, &parent.id).await?),
        1,
        "the parent's transcript holds the warning about its limit"
    );
    assert_eq!(warned(&stored_notices(&agent, &child.id).await?), 0);
    assert_eq!(
        warned(&notices(&responses)),
        0,
        "not in the subagent's stream"
    );
    Ok(())
}
