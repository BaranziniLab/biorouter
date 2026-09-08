//! End-to-end agent-loop tests for the hooks wiring in `Agent::reply()`.
//!
//! These drive the *real* reply loop with a mock provider (no network, no
//! keychain) and a per-test working directory holding a project
//! `.biorouter/hooks.yaml`, asserting that the integration points added to
//! agent.rs actually fire: UserPromptSubmit block + context injection,
//! PreToolUse deny feeding back into the tool result, the Stop-hook block loop,
//! and (BR-28) the turn-boundary settle that surfaces the aggregate of an
//! observe-only, `fire`d hook. Mirrors the MockToolProvider pattern in
//! tests/agent.rs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::object;
use tempfile::TempDir;

/// A provider that emits a `developer__shell` tool call on the first turn,
/// then plain text on subsequent turns (so the loop terminates).
struct ScriptedProvider {
    calls: AtomicUsize,
    /// When true the first turn requests a shell tool; otherwise text only.
    emit_tool: bool,
    final_text: String,
}

impl ScriptedProvider {
    fn new(emit_tool: bool, final_text: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            emit_tool,
            final_text: final_text.to_string(),
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
        if self.emit_tool && n == 0 {
            let tool_call = CallToolRequestParams {
                task: None,
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "echo hi"})),
            };
            let message = Message::assistant().with_tool_request("call_1", Ok(tool_call));
            return Ok((message, usage));
        }
        Ok((
            Message::assistant().with_text(self.final_text.clone()),
            usage,
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

/// Build an agent whose working dir is a temp dir containing the given
/// project hooks YAML, with project hooks enabled.
async fn agent_with_project_hooks(
    hooks_yaml: &str,
    provider: Arc<dyn Provider>,
) -> (Agent, String, TempDir) {
    agent_with_project_hooks_in_mode(hooks_yaml, provider, BioRouterMode::Auto).await
}

/// As above, but in an explicit mode. `Approve` is what raises a real permission
/// prompt, which is the only thing that fires a `Notification` hook.
async fn agent_with_project_hooks_in_mode(
    hooks_yaml: &str,
    provider: Arc<dyn Provider>,
    mode: BioRouterMode,
) -> (Agent, String, TempDir) {
    // SAFETY: tests run single-threaded per process for this env var; it only
    // flips on the project-hook opt-in the HooksManager reads at construction.
    std::env::set_var("BIOROUTER_ALLOW_PROJECT_HOOKS", "1");

    let work_dir = TempDir::new().unwrap();
    std::fs::create_dir_all(work_dir.path().join(".biorouter")).unwrap();
    std::fs::write(work_dir.path().join(".biorouter/hooks.yaml"), hooks_yaml).unwrap();

    let data_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    // A per-test permission store, never the `instance()` singleton: that one
    // reads the developer's real `~/.config/biorouter/permission.yaml`, so an
    // `always_allow` entry for `developer__shell` — what clicking "always allow"
    // once in the app leaves behind — auto-approves the call, no permission
    // prompt is raised, and the `Notification` hook the BR-28 tests assert on
    // never fires. Empty here means Approve mode really does prompt.
    let permission_dir = TempDir::new().unwrap();
    let config = AgentConfig::new(
        session_manager.clone(),
        Arc::new(PermissionManager::new(permission_dir.path().to_path_buf())),
        None,
        mode,
    );
    let agent = Agent::with_config(config);

    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "hook-loop-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    agent.update_provider(provider, &session.id).await.unwrap();

    // data_dir and permission_dir must outlive the agent; the returned tempdir
    // slot holds work_dir (sessions live under data_dir, but the session row is
    // already created and cached). We keep work_dir alive for the hooks file.
    std::mem::forget(data_dir);
    std::mem::forget(permission_dir);
    (agent, session.id, work_dir)
}

fn collect_texts(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::Text(t) => Some(t.text.clone()),
            MessageContent::SystemNotification(n) => Some(n.msg.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
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
            // Auto-approve any tool confirmation so the loop progresses.
            if let Some(MessageContent::ActionRequired(action)) = m.content.first() {
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
            out.push(m);
        }
    }
    Ok(out)
}

#[tokio::test]
async fn user_prompt_submit_block_short_circuits_reply() {
    let provider = Arc::new(ScriptedProvider::new(
        false,
        "this should never be produced",
    ));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  UserPromptSubmit:
    - hooks:
        - type: command
          command: "echo 'prompt blocked by policy' >&2; exit 2"
"#,
        provider.clone(),
    )
    .await;

    let messages = drain(&agent, "do something", &session_id).await.unwrap();
    let text = collect_texts(&messages);
    assert!(
        text.contains("Prompt blocked by hook: prompt blocked by policy"),
        "expected the block notice, got: {text}"
    );
    // The provider must never have been called: the prompt was blocked first.
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn user_prompt_submit_context_is_injected_for_the_model() {
    let provider = Arc::new(ScriptedProvider::new(false, "done"));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  UserPromptSubmit:
    - hooks:
        - type: command
          command: "echo 'remember: the codeword is ZEBRAFISH'"
"#,
        provider.clone(),
    )
    .await;

    let _ = drain(&agent, "hello", &session_id).await.unwrap();

    // The injected context should be persisted as a hidden user message.
    let session = agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await
        .unwrap();
    let convo = session.conversation.unwrap();
    let all = collect_texts(convo.messages());
    assert!(
        all.contains("ZEBRAFISH") && all.contains("hook-context"),
        "expected injected hook context in conversation, got: {all}"
    );
    assert!(provider.calls.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn pre_tool_use_deny_feeds_reason_back_to_model() {
    let provider = Arc::new(ScriptedProvider::new(true, "acknowledged"));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "echo 'shell is not allowed in this project' >&2; exit 2"
"#,
        provider.clone(),
    )
    .await;

    let _ = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();

    let session = agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await
        .unwrap();
    let convo = session.conversation.unwrap();
    // The denied tool response must carry the hook's reason back to the model.
    let found = convo.messages().iter().any(|m| {
        m.content.iter().any(|c| {
            if let MessageContent::ToolResponse(tr) = c {
                if let Ok(result) = &tr.tool_result {
                    return result.content.iter().any(|content| {
                        matches!(&content.raw, rmcp::model::RawContent::Text(t)
                            if t.text.contains("Hook feedback: shell is not allowed in this project"))
                    });
                }
            }
            false
        })
    });
    assert!(found, "expected hook deny reason in the tool response");
}

#[tokio::test]
async fn stop_hook_blocks_then_releases() {
    // Stop hook blocks exactly once (via a marker file), then allows. The
    // loop should run an extra iteration and surface the block notice.
    let marker = TempDir::new().unwrap();
    let marker_path = marker.path().join("block-once");
    std::fs::write(&marker_path, "1").unwrap();

    let provider = Arc::new(ScriptedProvider::new(false, "all done"));
    let yaml = format!(
        r#"hooks:
  Stop:
    - hooks:
        - type: command
          command: "if [ -f '{p}' ]; then rm '{p}'; echo 'finish the task first' >&2; exit 2; fi"
"#,
        p = marker_path.display()
    );
    let (agent, session_id, _work) = agent_with_project_hooks(&yaml, provider.clone()).await;

    let messages = drain(&agent, "hi", &session_id).await.unwrap();
    let text = collect_texts(&messages);
    assert!(
        text.contains("Stop hook blocked completion: finish the task first"),
        "expected the stop-block notice, got: {text}"
    );
    // Blocked once then released → provider called at least twice.
    assert!(
        provider.calls.load(Ordering::SeqCst) >= 2,
        "stop block should have forced another turn"
    );
}

// ---- BR-28: observe-only (`fire`d) hook aggregates reach the caller ----

/// A `Notification` hook fires when a permission prompt is raised. It is
/// dispatched detached (`fire`), and its whole `HookAggregate` used to be
/// dropped on the floor — so a `systemMessage` was invisible and there was no
/// way to know the hook had run. The turn boundary now joins the task and
/// surfaces the message to the user.
#[tokio::test]
async fn fired_notification_hook_system_message_reaches_the_user() {
    let provider = Arc::new(ScriptedProvider::new(true, "done"));
    let (agent, session_id, _work) = agent_with_project_hooks_in_mode(
        r#"hooks:
  Notification:
    - hooks:
        - type: command
          command: "echo '{\"systemMessage\":\"audit: permission prompt shown\"}'"
"#,
        provider.clone(),
        BioRouterMode::Approve,
    )
    .await;

    let messages = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();
    let text = collect_texts(&messages);
    assert!(
        text.contains("audit: permission prompt shown"),
        "expected the fired Notification hook's systemMessage to surface, got: {text}"
    );
}

/// The settle is bounded: a fired hook slower than the turn-boundary budget must
/// not stall the loop. The turn completes normally without the hook's message,
/// and the hook is still in flight afterwards — proof the loop moved on rather
/// than waiting for it. (The budget's timing semantics are pinned directly in
/// `hooks::tests::slow_fired_hook_misses_its_budget_and_settles_later`; asserting
/// on wall-clock here would be flaky under parallel-test contention.)
#[tokio::test]
async fn slow_fired_hook_does_not_stall_the_turn() {
    let provider = Arc::new(ScriptedProvider::new(true, "finished anyway"));
    let (agent, session_id, _work) = agent_with_project_hooks_in_mode(
        r#"hooks:
  Notification:
    - hooks:
        - type: command
          command: "sleep 20; echo '{\"systemMessage\":\"too late\"}'"
"#,
        provider.clone(),
        BioRouterMode::Approve,
    )
    .await;

    let messages = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();

    let text = collect_texts(&messages);
    assert!(
        text.contains("finished anyway"),
        "the turn must complete, got: {text}"
    );
    assert!(
        !text.contains("too late"),
        "a hook that misses its budget must not be waited on, got: {text}"
    );
    assert_eq!(
        agent.hooks_manager().pending_fire_count(),
        1,
        "the slow hook should still be in flight, so the turn did not wait for it"
    );
}

// ---- BR-19: input rewrite, PostToolUse block, and hook context on the tool path ----

/// Collect every tool call recorded in the persisted conversation.
async fn tool_calls(agent: &Agent, session_id: &str) -> Vec<(String, serde_json::Value)> {
    let session = agent
        .config
        .session_manager
        .get_session(session_id, true)
        .await
        .unwrap();
    session
        .conversation
        .unwrap()
        .messages()
        .iter()
        .flat_map(|m| m.content.clone())
        .filter_map(|c| match c {
            MessageContent::ToolRequest(req) => req.tool_call.ok().map(|call| {
                (
                    call.name.to_string(),
                    call.arguments
                        .map(serde_json::Value::Object)
                        .unwrap_or(serde_json::Value::Null),
                )
            }),
            _ => None,
        })
        .collect()
}

/// Collect the text of every tool response in the persisted conversation.
async fn tool_response_texts(agent: &Agent, session_id: &str) -> String {
    let session = agent
        .config
        .session_manager
        .get_session(session_id, true)
        .await
        .unwrap();
    session
        .conversation
        .unwrap()
        .messages()
        .iter()
        .flat_map(|m| m.content.clone())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(tr) => Some(tr),
            _ => None,
        })
        .flat_map(|tr| match tr.tool_result {
            Ok(result) => result
                .content
                .iter()
                .filter_map(|c| match &c.raw {
                    rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            Err(e) => vec![e.to_string()],
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The headline of BR-19: a PreToolUse hook rewrites the tool input (here it
/// sandboxes a destructive `rm -rf /` into the build dir) and the *rewritten*
/// call is what runs and what the transcript records. The model is told, via
/// injected hook context, that its arguments were changed.
#[tokio::test]
async fn pre_tool_use_hook_rewrites_the_tool_input() {
    let provider = Arc::new(ScriptedProvider::new(true, "understood"));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "echo '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{\"command\":\"echo sandboxed\"}}}'"
"#,
        provider.clone(),
    )
    .await;

    let _ = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();

    let calls = tool_calls(&agent, &session_id).await;
    let shell = calls
        .iter()
        .find(|(name, _)| name == "developer__shell")
        .expect("the shell call is recorded");
    assert_eq!(
        shell.1["command"], "echo sandboxed",
        "the hook's rewritten arguments must be what is dispatched and persisted, got: {:?}",
        shell.1
    );

    // And the model must be told its call was rewritten, or it would silently
    // work from a call it never made.
    let session = agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await
        .unwrap();
    let all = collect_texts(session.conversation.unwrap().messages());
    assert!(
        all.contains("rewrote the arguments of `developer__shell`") && all.contains("hook-context"),
        "expected the framed rewrite notice, got: {all}"
    );
}

/// A rewrite must not be a hole around the security/permission gates: a denied
/// call is never dispatched, so the hook's rewrite for it is dropped and the
/// deny reason still reaches the model.
#[tokio::test]
async fn a_denied_call_is_not_rewritten() {
    let provider = Arc::new(ScriptedProvider::new(true, "acknowledged"));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "echo '{\"hookSpecificOutput\":{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"no shell here\",\"updatedInput\":{\"command\":\"echo rewritten\"}}}'"
"#,
        provider.clone(),
    )
    .await;

    let _ = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();

    let calls = tool_calls(&agent, &session_id).await;
    let shell = calls
        .iter()
        .find(|(name, _)| name == "developer__shell")
        .expect("the shell call is recorded");
    assert_eq!(
        shell.1["command"], "echo hi",
        "a denied call keeps the model's original arguments"
    );
    let responses = tool_response_texts(&agent, &session_id).await;
    assert!(
        responses.contains("Hook feedback: no shell here"),
        "the deny reason still wins, got: {responses}"
    );
}

/// A PreToolUse hook's `additionalContext` used to be computed and dropped —
/// `HookInspector` read only the decision. It must now reach the model.
#[tokio::test]
async fn pre_tool_use_additional_context_reaches_the_model() {
    let provider = Arc::new(ScriptedProvider::new(true, "noted"));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "echo '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"additionalContext\":\"policy: prefer ripgrep over grep\"},\"systemMessage\":\"policy hook ran\"}'"
"#,
        provider.clone(),
    )
    .await;

    let messages = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();

    // systemMessage surfaces to the user...
    let text = collect_texts(&messages);
    assert!(
        text.contains("policy hook ran"),
        "expected the PreToolUse systemMessage, got: {text}"
    );

    // ...and additionalContext lands in the conversation, untrusted-framed.
    let session = agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await
        .unwrap();
    let all = collect_texts(session.conversation.unwrap().messages());
    assert!(
        all.contains("prefer ripgrep over grep") && all.contains("hook-context"),
        "expected the injected PreToolUse context, got: {all}"
    );
}

/// PostToolUse was observe-only: the block decision was computed and thrown
/// away. It must now turn the result into corrective feedback for the model.
#[tokio::test]
async fn post_tool_use_hook_can_block_the_result() {
    let provider = Arc::new(ScriptedProvider::new(true, "will fix"));
    let (agent, session_id, _work) = agent_with_project_hooks(
        r#"hooks:
  PostToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "echo 'lint failed on the file you wrote' >&2; exit 2"
  PostToolUseFailure:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "echo 'lint failed on the file you wrote' >&2; exit 2"
"#,
        provider.clone(),
    )
    .await;

    let messages = drain(&agent, "run a shell command", &session_id)
        .await
        .unwrap();

    let responses = tool_response_texts(&agent, &session_id).await;
    assert!(
        responses.contains("A PostToolUse hook blocked this result")
            && responses.contains("Hook feedback: lint failed on the file you wrote"),
        "expected the block fed back as a corrective tool result, got: {responses}"
    );
    let text = collect_texts(&messages);
    assert!(
        text.contains("Hook blocked the result of developer__shell"),
        "expected the user-facing block notice, got: {text}"
    );
    // The turn keeps working after the block rather than dying.
    assert!(provider.calls.load(Ordering::SeqCst) >= 2);
}
