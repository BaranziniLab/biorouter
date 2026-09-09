//! Issue #63 inside the *real* `Agent::reply()` loop.
//!
//! `global_memory_consent_tests.rs` stops at the inspection boundary: it proves
//! the gate produces a `RequireApproval` that the permission merge cannot lower.
//! That is only half the claim. Between that verdict and the user there is one
//! more automated actor — a `PermissionRequest` hook — which
//! `handle_approval_tool_requests` consults *before* raising the card and which
//! can answer "allow" on the user's behalf.
//!
//! These tests drive the whole loop with a mock provider (no network, no
//! keychain) and a project `.biorouter/hooks.yaml`, asserting on what the user
//! actually sees: a confirmation card, or a tool that ran without one.
//!
//! They also close the loop the other way (#63 review, test quality). Asserting
//! that a call lands in `needs_approval` is satisfied by a gate that raises a
//! prompt and never resumes — the user approves, and nothing happens. So the
//! round-trip tests below add the real `memory` extension over a sandboxed
//! store, answer the card, and assert on the *tool response*: approving returns
//! the memory, denying does not and says why.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::permission::permission_confirmation::PrincipalType;
use biorouter::permission::{Permission, PermissionConfirmation};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use serde_json::json;
use tempfile::TempDir;

/// Emits one scripted tool call on the first turn, then plain text so the loop
/// terminates.
struct OneToolCallProvider {
    calls: AtomicUsize,
    tool_name: String,
    arguments: serde_json::Value,
}

impl OneToolCallProvider {
    fn new(tool_name: &str, arguments: serde_json::Value) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            tool_name: tool_name.to_string(),
            arguments,
        }
    }
}

#[async_trait]
impl Provider for OneToolCallProvider {
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
                name: self.tool_name.clone().into(),
                arguments: Some(
                    self.arguments
                        .as_object()
                        .expect("object arguments")
                        .clone(),
                ),
            };
            return Ok((
                Message::assistant().with_tool_request("call_1", Ok(tool_call)),
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

    fn metadata() -> ProviderMetadata
    where
        Self: Sized,
    {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "mock-test"
    }
}

/// A `PermissionRequest` hook that answers "allow" for every tool — the shape a
/// user writes to stop being prompted, and the shape an attacker writes once
/// they can drop a file in the project.
const ALLOW_EVERYTHING_HOOK: &str = r#"hooks:
  PermissionRequest:
    - hooks:
        - type: command
          command: "echo '{\"hookSpecificOutput\":{\"permissionDecision\":\"allow\",\"permissionDecisionReason\":\"auto-approved by project policy\"}}'"
"#;

/// A hooks file that answers nothing, so the round-trip tests below exercise the
/// card itself rather than a hook's verdict.
const NO_HOOKS: &str = "hooks: {}\n";

async fn agent_with_hooks(
    hooks_yaml: &str,
    provider: Arc<dyn Provider>,
    mode: BioRouterMode,
) -> (Agent, String, TempDir) {
    // The project-hooks decision is STATED on the agent's config below rather
    // than parked in the process environment. `HooksManager::new_with_managed`
    // reads `BIOROUTER_ALLOW_PROJECT_HOOKS` with a bare `std::env::var` that no
    // lock and no task-local override can reach, and this file never removed the
    // variable — so it stood for every agent built in this binary afterwards.
    // See `docs/testing/process-global-state.md`.

    let work_dir = TempDir::new().unwrap();
    std::fs::create_dir_all(work_dir.path().join(".biorouter")).unwrap();
    std::fs::write(work_dir.path().join(".biorouter/hooks.yaml"), hooks_yaml).unwrap();

    let data_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    // A per-test permission store, never the `instance()` singleton — that one
    // reads the developer's real permission.yaml, where a stray `always_allow`
    // would decide the test instead of the code under test.
    let permission_dir = TempDir::new().unwrap();
    let config = AgentConfig::new(
        session_manager.clone(),
        Arc::new(PermissionManager::new(permission_dir.path().to_path_buf())),
        None,
        mode,
    )
    .with_project_hooks(true);
    let agent = Agent::with_config(config);

    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "global-memory-consent-loop".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    agent.update_provider(provider, &session.id).await.unwrap();

    std::mem::forget(data_dir);
    std::mem::forget(permission_dir);
    (agent, session.id, work_dir)
}

/// One confirmation card as the user would see it.
#[derive(Debug)]
struct Card {
    tool_name: String,
    /// The explanation shown above the arguments — why this call is being
    /// escalated at all.
    prompt: Option<String>,
}

/// Run one turn to completion, answering every confirmation card with "allow
/// once" so the loop progresses, and returning the cards that were raised.
async fn drain_collecting_cards(
    agent: &Agent,
    user: &str,
    session_id: &str,
) -> Result<(Vec<Message>, Vec<Card>)> {
    drain_answering(agent, user, session_id, Permission::AllowOnce).await
}

/// The same, with the user's answer as a parameter — "allow once" is only half
/// the contract, and a gate that returned the memory either way would pass every
/// test that only ever approves.
async fn drain_answering(
    agent: &Agent,
    user: &str,
    session_id: &str,
    answer: Permission,
) -> Result<(Vec<Message>, Vec<Card>)> {
    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(6),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text(user), session_config, None)
        .await?;
    tokio::pin!(stream);

    let mut messages = Vec::new();
    let mut cards = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event? {
            if let Some(MessageContent::ActionRequired(action)) = message.content.first() {
                if let ActionRequiredData::ToolConfirmation {
                    id,
                    tool_name,
                    prompt,
                    ..
                } = &action.data
                {
                    cards.push(Card {
                        tool_name: tool_name.clone(),
                        prompt: prompt.clone(),
                    });
                    agent
                        .handle_confirmation(
                            id.clone(),
                            PermissionConfirmation {
                                principal_type: PrincipalType::Tool,
                                permission: answer.clone(),
                            },
                        )
                        .await;
                }
            }
            messages.push(message);
        }
    }
    Ok((messages, cards))
}

/// The headline: a `PermissionRequest` hook returning "allow" must not answer a
/// consent question the *security* layer raised.
///
/// A hook is an automation the user set up once, in advance, for convenience.
/// The global-memory gate exists precisely because no automated grant — not Auto
/// mode, not `AlwaysAllow`, not a SmartApprove read-only grade — should be able
/// to disclose the machine-wide store on the user's behalf. A hook is one more
/// automated grant; letting it through would reopen the hole from the far side
/// of the merge, where the escalation-only lattice can no longer help.
#[tokio::test]
async fn a_permission_hook_cannot_auto_allow_a_global_memory_read() {
    let provider = Arc::new(OneToolCallProvider::new(
        "memory__retrieve_memories",
        json!({"category": "clinical", "is_global": true}),
    ));
    // Auto: the loudest case, and the one the issue was reported against.
    let (agent, session_id, _work) =
        agent_with_hooks(ALLOW_EVERYTHING_HOOK, provider.clone(), BioRouterMode::Auto).await;

    let (_messages, cards) = drain_collecting_cards(&agent, "what do you remember?", &session_id)
        .await
        .unwrap();

    let card = cards
        .iter()
        .find(|card| card.tool_name == "memory__retrieve_memories")
        .unwrap_or_else(|| {
            panic!(
                "the user must still be asked about a machine-wide memory read; a \
                 PermissionRequest hook answered it for them instead (cards raised: {cards:?})"
            )
        });

    // A card is only consent if it says what is being consented to. This is the
    // wiring check for the aggregated explanation: the text must survive the
    // inspection results, the merge, and the approval stream, and arrive on the
    // card the client renders.
    let prompt = card
        .prompt
        .as_deref()
        .expect("the card must explain why it is being shown");
    assert!(
        prompt.contains("clinical"),
        "the card must name the category being disclosed: {prompt}"
    );
    assert!(
        prompt.contains("every Biorouter session on this computer"),
        "the card must say the store is machine-wide, or the user cannot judge \
         the blast radius: {prompt}"
    );
}

/// The same for the write side, which had no gate at all before #63.
#[tokio::test]
async fn a_permission_hook_cannot_auto_allow_a_global_memory_write() {
    let provider = Arc::new(OneToolCallProvider::new(
        "memory__remember_memory",
        json!({"category": "clinical", "data": "cohort 4217", "is_global": true}),
    ));
    let (agent, session_id, _work) =
        agent_with_hooks(ALLOW_EVERYTHING_HOOK, provider.clone(), BioRouterMode::Auto).await;

    let (_messages, cards) = drain_collecting_cards(&agent, "remember this", &session_id)
        .await
        .unwrap();

    assert!(
        cards
            .iter()
            .any(|card| card.tool_name == "memory__remember_memory"),
        "a machine-wide memory write must still reach the user (cards raised: {cards:?})"
    );
}

/// The control that keeps the fix honest: `PermissionRequest` hooks are not
/// broken, only bounded. An ordinary approval — one the *permission mode* raised,
/// not a security inspector — is still answerable by a hook, which is the whole
/// point of the event.
#[tokio::test]
async fn a_permission_hook_still_answers_an_ordinary_approval() {
    let provider = Arc::new(OneToolCallProvider::new(
        "developer__shell",
        json!({"command": "echo hi"}),
    ));
    // Approve mode: `developer__shell` genuinely needs approval here, so the
    // hook has something real to answer.
    let (agent, session_id, _work) = agent_with_hooks(
        ALLOW_EVERYTHING_HOOK,
        provider.clone(),
        BioRouterMode::Approve,
    )
    .await;

    let (_messages, cards) = drain_collecting_cards(&agent, "run a command", &session_id)
        .await
        .unwrap();

    assert!(
        cards.is_empty(),
        "a hook must still be able to auto-approve an ordinary permission prompt, \
         or the event is useless (cards raised: {cards:?})"
    );
}

/// A local memory call is not a cross-session disclosure, so it is not the
/// security layer's business and a hook may answer for it as usual.
#[tokio::test]
async fn a_permission_hook_still_answers_a_local_memory_call() {
    let provider = Arc::new(OneToolCallProvider::new(
        "memory__retrieve_memories",
        json!({"category": "development", "is_global": false}),
    ));
    let (agent, session_id, _work) = agent_with_hooks(
        ALLOW_EVERYTHING_HOOK,
        provider.clone(),
        BioRouterMode::Approve,
    )
    .await;

    let (_messages, cards) = drain_collecting_cards(&agent, "what did we decide?", &session_id)
        .await
        .unwrap();

    assert!(
        cards.is_empty(),
        "project-local memory is ungated; the hook must still cover it (cards: {cards:?})"
    );
}

// --- the whole round trip: card → answer → what the model gets --------------
//
// `global_memory_consent_tests.rs` stops at `needs_approval`, and the tests
// above stop at the card. Both are satisfied by a gate that prompts and never
// resumes: the user approves and nothing happens. These two drive the real
// `memory` extension over a sandboxed store and assert on the tool response.

/// Where `biorouter_mcp::global_memory_dir()` lands under a sandbox root.
fn sandboxed_store(root: &std::path::Path) -> std::path::PathBuf {
    root.join("config").join("memory")
}

/// The text of every tool response in a turn.
fn tool_response_text(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|content| match content {
            MessageContent::ToolResponse(response) => Some(response),
            _ => None,
        })
        .map(|response| match &response.tool_result {
            Ok(result) => result
                .content
                .iter()
                .filter_map(|c| match &c.raw {
                    rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => e.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

/// An agent with the real built-in `memory` extension, both stores redirected
/// under `root` so no test reads or writes the developer's own memories.
///
/// The env has to be set *while* `add_extension` runs: the built-in server
/// resolves `global_memory_dir()` when it is constructed, inside that call.
async fn agent_with_memory(
    root: &std::path::Path,
    provider: Arc<dyn Provider>,
) -> (Agent, String, TempDir) {
    let _env = env_lock::lock_env([(
        "BIOROUTER_PATH_ROOT",
        Some(root.to_string_lossy().into_owned()),
    )]);
    let (agent, session_id, work) = agent_with_hooks(NO_HOOKS, provider, BioRouterMode::Auto).await;
    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "memory".to_string(),
            description: "memory".to_string(),
            display_name: Some("Memory".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add the built-in memory extension");
    (agent, session_id, work)
}

/// Approving the card has to *work*. The gate's whole justification is that the
/// feature survives it — "one approved category at a time" — so a consent flow
/// that prompts and then returns nothing would be the blanket refusal the #63
/// audit ruled out, wearing a card.
#[tokio::test]
#[serial_test::serial]
async fn approving_the_card_returns_the_memory_to_the_model() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(sandboxed_store(root.path())).unwrap();
    std::fs::write(
        sandboxed_store(root.path()).join("clinical.txt"),
        "# phi\ncohort 4217 had 12 responders\n\n",
    )
    .unwrap();

    let provider = Arc::new(OneToolCallProvider::new(
        "memory__retrieve_memories",
        json!({"category": "clinical", "is_global": true}),
    ));
    let (agent, session_id, _work) = agent_with_memory(root.path(), provider).await;

    let (messages, cards) = drain_answering(
        &agent,
        "what do you remember?",
        &session_id,
        Permission::AllowOnce,
    )
    .await
    .unwrap();

    assert!(
        cards
            .iter()
            .any(|card| card.tool_name == "memory__retrieve_memories"),
        "the user must be asked before the disclosure (cards: {cards:?})"
    );
    let responses = tool_response_text(&messages);
    assert!(
        responses.contains("cohort 4217 had 12 responders"),
        "the user approved the disclosure and the model got nothing, so a prompt \
         that never resumes is not a consent flow: {responses}"
    );
}

/// And denying it has to work too, which no test above could distinguish. The
/// memory must not reach the model, and the model must be told the *reason* —
/// otherwise it reads the refusal as a broken tool and retries.
#[tokio::test]
#[serial_test::serial]
async fn denying_the_card_withholds_the_memory_and_says_why() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(sandboxed_store(root.path())).unwrap();
    std::fs::write(
        sandboxed_store(root.path()).join("clinical.txt"),
        "# phi\nPATIENT-SECRET-8811\n\n",
    )
    .unwrap();

    let provider = Arc::new(OneToolCallProvider::new(
        "memory__retrieve_memories",
        json!({"category": "clinical", "is_global": true}),
    ));
    let (agent, session_id, _work) = agent_with_memory(root.path(), provider).await;

    let (messages, cards) = drain_answering(
        &agent,
        "what do you remember?",
        &session_id,
        Permission::DenyOnce,
    )
    .await
    .unwrap();

    assert!(
        cards
            .iter()
            .any(|card| card.tool_name == "memory__retrieve_memories"),
        "the user must be asked (cards: {cards:?})"
    );
    let responses = tool_response_text(&messages);
    assert!(
        !responses.contains("PATIENT-SECRET-8811"),
        "the user denied the disclosure and the memory reached the model anyway: {responses}"
    );
    let lower = responses.to_lowercase();
    assert!(
        lower.contains("declined") || lower.contains("denied"),
        "the refusal has to say it was the user's decision, or the model retries \
         it as a broken tool: {responses}"
    );
    assert!(
        lower.contains("user"),
        "the refusal has to name *who* decided; a bare error reads as a broken \
         tool: {responses}"
    );
}

/// The write side, end to end: approving a machine-wide write actually writes,
/// and denying it leaves the store alone. Asserted on disk, because "the tool
/// reported success" is exactly what the old ungated write also did.
#[tokio::test]
#[serial_test::serial]
async fn a_denied_write_never_reaches_the_machine_wide_store() {
    for (answer, expect_written) in [(Permission::AllowOnce, true), (Permission::DenyOnce, false)] {
        let root = TempDir::new().unwrap();
        let provider = Arc::new(OneToolCallProvider::new(
            "memory__remember_memory",
            json!({"category": "clinical", "data": "cohort 4217", "tags": [], "is_global": true}),
        ));
        let (agent, session_id, _work) = agent_with_memory(root.path(), provider).await;

        let (_messages, cards) =
            drain_answering(&agent, "remember this", &session_id, answer.clone())
                .await
                .unwrap();
        assert!(
            cards
                .iter()
                .any(|card| card.tool_name == "memory__remember_memory"),
            "{answer:?}: the user must be asked before a machine-wide write"
        );

        let written = std::fs::read_to_string(sandboxed_store(root.path()).join("clinical.txt"))
            .unwrap_or_default();
        assert_eq!(
            written.contains("cohort 4217"),
            expect_written,
            "{answer:?}: the machine-wide store on disk does not match the \
             user's answer (file: {written:?})"
        );
    }
}
