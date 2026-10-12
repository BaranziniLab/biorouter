//! `platform__report_bug` end to end through a real [`Agent::reply`].
//!
//! ⚠ What this covers that the unit tests cannot: that a tool call the MODEL
//! makes actually reaches the handler. Those two halves live in different files
//! and neither refers to the other — the tool is declared in `platform_tools.rs`
//! and routed by a `if tool_call.name == …` branch in the agent's dispatch — so
//! a tool can be advertised with no route and the only symptom is
//! `Tool 'platform__report_bug' not found` at the moment the user asks for it.
//! `platform_tools`' own guard greps `agent.rs` for the branch, which proves the
//! text exists; this proves the call arrives.
//!
//! ⚠ It also cannot post. Nothing here approves the card, and
//! `issue::file_with_gh` refuses outright from a test binary besides — a
//! property this file now ASSERTS rather than assumes, because the guard it
//! used to name (`cfg!(test)`) does not hold here at all. See
//! `filing_is_refused_from_an_integration_test_binary` at the bottom.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::platform_tools::PLATFORM_REPORT_BUG_TOOL_NAME;
use biorouter::agents::{Agent, AgentConfig, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::{SessionManager, SessionType};
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Calls `platform__report_bug` on its first turn, then answers with text.
///
/// It also records the tool list it was handed, which is the other half of the
/// question: a tool the model is never offered is one it can never call, and
/// that failure looks nothing like a missing dispatch branch.
struct ReportingProvider {
    turns: AtomicUsize,
    offered: Mutex<Vec<String>>,
    arguments: serde_json::Value,
}

#[async_trait]
impl Provider for ReportingProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "reporting",
            "Reporting",
            "",
            "reporting-model",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "reporting"
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        *self.offered.lock().unwrap() = tools.iter().map(|t| t.name.to_string()).collect();
        let usage = ProviderUsage::new("reporting-model".to_string(), Usage::default());
        if self.turns.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok((
                Message::assistant().with_tool_request(
                    "report-1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: PLATFORM_REPORT_BUG_TOOL_NAME.into(),
                        arguments: self.arguments.as_object().cloned(),
                        meta: None,
                    }),
                ),
                usage,
            ));
        }
        Ok((Message::assistant().with_text("done"), usage))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("reporting-model")
    }
}

/// Restores the process-global proof-availability flag on drop.
///
/// ⚠ Necessary, and the reason is a measured failure: without it, the test that
/// sets the flag to `false` ran concurrently with the others in this binary and
/// emptied THEIR tool rosters — which reads exactly like "the tool is never
/// advertised", i.e. like the bug the suite exists to catch. `#[serial]` on top
/// keeps the window closed; the guard keeps a panicking test from leaving the
/// flag off for everything after it.
struct ProofAvailable(bool);

impl ProofAvailable {
    fn set(available: bool) -> Self {
        let previous = biorouter::pending_user_action::user_proof_available();
        biorouter::pending_user_action::set_user_proof_available(available);
        Self(previous)
    }
}

impl Drop for ProofAvailable {
    fn drop(&mut self) {
        biorouter::pending_user_action::set_user_proof_available(self.0);
    }
}

async fn run_turn(arguments: serde_json::Value) -> (Vec<String>, String) {
    let dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    let permission_manager = Arc::new(PermissionManager::new(dir.path().to_path_buf()));
    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        session_manager,
        permission_manager,
        None,
        BioRouterMode::Auto,
    )));
    let session = agent
        .config
        .session_manager
        .create_session(
            PathBuf::from("/workspace/demo"),
            "bug-report-e2e".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();

    let provider = Arc::new(ReportingProvider {
        turns: AtomicUsize::new(0),
        offered: Mutex::new(Vec::new()),
        arguments,
    });
    agent
        .update_provider(Arc::clone(&provider) as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();

    // `no_human_surface` is the production statement of "there is nobody to
    // answer a card". It is what stops the file half parking forever, and it is
    // the same thing `POST /agent/call_tool` sets for the same reason.
    let events = biorouter::user_surface::without_human_surface(async {
        let mut stream = agent
            .reply(
                Message::user().with_text("report a bug"),
                SessionConfig {
                    id: session.id.clone(),
                    schedule_id: None,
                    max_turns: Some(3),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        let mut collected = Vec::new();
        while let Some(event) = stream.next().await {
            collected.push(event);
        }
        collected
    })
    .await;

    // The tool response the agent produced, whatever its shape.
    let mut responses = String::new();
    for event in events.into_iter().flatten() {
        if let biorouter::agents::AgentEvent::Message(message) = event {
            for content in &message.content {
                if let MessageContent::ToolResponse(response) = content {
                    match &response.tool_result {
                        Ok(result) => {
                            for item in &result.content {
                                if let Some(text) = item.as_text() {
                                    responses.push_str(&text.text);
                                    responses.push('\n');
                                }
                            }
                        }
                        Err(error) => {
                            responses.push_str(&error.message);
                            responses.push('\n');
                        }
                    }
                }
            }
        }
    }
    let offered = provider.offered.lock().unwrap().clone();
    (offered, responses)
}

/// The tool is offered to the model, and calling it reaches the handler.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn the_model_is_offered_the_reporter_and_its_call_reaches_the_handler() {
    let _proof = ProofAvailable::set(true);
    let (offered, response) = run_turn(serde_json::json!({"action": "analyze"})).await;

    assert!(
        offered
            .iter()
            .any(|name| name == PLATFORM_REPORT_BUG_TOOL_NAME),
        "the model was never offered the bug reporter, so no dispatch branch could \
         have helped: {offered:?}"
    );
    assert!(
        !response.contains("not found"),
        "the advertised tool has no route in the agent's dispatch: {response}"
    );
    // The push-back: an empty session with no description asks rather than files.
    assert!(
        response.contains("ASK THE USER"),
        "the handler's own answer must come back through the loop: {response}"
    );
}

/// With a description, the analysis comes back through the loop carrying the
/// investigation method pinned to THIS build's version — the documentation,
/// the source at its tag, and a request for a suspected cause — rather than an
/// instruction to file at once.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn a_described_problem_gets_the_investigation_pinned_to_this_version() {
    let _proof = ProofAvailable::set(true);
    let (_offered, response) = run_turn(serde_json::json!({
        "action": "analyze",
        "description": "the chart panel is blank when the dataset has one row"
    }))
    .await;

    assert!(response.contains("The user is reporting"), "{response}");
    assert!(
        response.contains("https://biorouter.ucsf.edu/docs"),
        "{response}"
    );
    assert!(
        response.contains(&format!(
            "https://github.com/BaranziniLab/biorouter/tree/v{}",
            env!("CARGO_PKG_VERSION")
        )),
        "the source must be read at the version the user runs: {response}"
    );
    assert!(response.contains("`suspected_cause`"), "{response}");
    assert!(!response.contains("ASK THE USER"), "{response}");
}

/// A `file` call with a real report gets as far as the approval and no further.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn filing_stops_at_the_approval_and_posts_nothing() {
    let _proof = ProofAvailable::set(true);
    let (_offered, response) = run_turn(serde_json::json!({
        "action": "file",
        "title": "Auto Visualiser renders a blank panel for a single-row dataset",
        "description": "Asking for a chart of a one-row table produces an empty panel.",
        "steps": ["Open a chat", "Ask for a bar chart of one row"],
        "expected": "The chart renders with one bar."
    }))
    .await;

    assert!(
        response.contains("Nothing was filed"),
        "with nobody to approve, the tool must report that and post nothing: {response}"
    );
    assert!(!response.contains("Filed:"), "{response}");
    assert!(!response.contains("not found"), "{response}");
}

/// ⚠ Advertisement is not unconditional. On a daemon that cannot obtain proof
/// of a person the tool is withheld from the model entirely — and this asserts
/// it at the roster the model is actually handed, not at the gate struct.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn a_daemon_that_cannot_ask_a_person_does_not_offer_the_reporter() {
    let _proof = ProofAvailable::set(false);
    let (offered, _response) = run_turn(serde_json::json!({"action": "analyze"})).await;

    assert!(
        !offered
            .iter()
            .any(|name| name == PLATFORM_REPORT_BUG_TOOL_NAME),
        "a tool whose approval can never be granted must not be advertised: {offered:?}"
    );
}

/// Documents what the roster looked like, so a future reader can tell an
/// advertisement problem from a routing one at a glance.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn the_platform_prefix_is_how_the_reporter_is_named_to_the_model() {
    let _proof = ProofAvailable::set(true);
    let (offered, _) = run_turn(serde_json::json!({"action": "analyze"})).await;
    assert!(
        offered
            .iter()
            .any(|name| name.starts_with("platform__") && name.ends_with("report_bug")),
        "{offered:?}"
    );
    // Not an extension: nothing in the manager advertises it.
    let _ = ExtensionConfig::default();
}

/// ⚠ Does the approval card actually reach the client?
///
/// The tool parks on a proof-backed approval, and every other assertion in this
/// suite runs under `without_human_surface`, where `park` registers nothing and
/// answers `Cancelled` at once. That proves the refusal path and says nothing
/// about the path that matters: a card that is raised but never delivered
/// leaves the user looking at a turn that has silently stopped, with no dialog
/// and no way to answer it.
///
/// So this one raises a REAL card and asserts it is yielded out of
/// `Agent::reply` as an `AgentEvent::Message` carrying `ActionRequired` — which
/// is the frame the desktop renders and the SSE route forwards. It then denies
/// it, so the turn ends instead of sitting out the 15-minute TTL.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn the_approval_card_is_yielded_out_of_the_reply_stream() {
    card_is_yielded(None).await;
}

/// ⚠ The same thing, with a LIVE (uncancelled) `CancellationToken`.
///
/// `/reply` always passes one; the in-process test above passed `None`, and
/// `next_batch_wake` is a `biased` select whose FIRST arm is that token. If the
/// card only surfaces when the arm is `pending::<()>()` forever, every real
/// chat turn is the failing configuration and only the test is the passing one.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn the_approval_card_is_yielded_even_with_a_live_cancel_token() {
    card_is_yielded(Some(CancellationToken::new())).await;
}

async fn card_is_yielded(cancel_token: Option<CancellationToken>) {
    use biorouter::pending_user_action::{
        DecisionAuthority, PendingUserActions, UserActionOutcome,
    };
    use biorouter::permission::Permission;

    let _proof = ProofAvailable::set(true);
    let dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    let permission_manager = Arc::new(PermissionManager::new(dir.path().to_path_buf()));
    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        session_manager,
        permission_manager,
        None,
        BioRouterMode::Auto,
    )));
    let session = agent
        .config
        .session_manager
        .create_session(
            PathBuf::from("/workspace/demo"),
            "card-delivery".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    let session_id = session.id.clone();
    agent
        .update_provider(
            Arc::new(ReportingProvider {
                turns: AtomicUsize::new(0),
                offered: Mutex::new(Vec::new()),
                arguments: serde_json::json!({
                    "action": "file",
                    "title": "Auto Visualiser renders a blank panel for a single-row dataset",
                    "description": "A chart of a one-row table produces an empty panel.",
                    "steps": ["Open a chat", "Ask for a chart of one row"],
                    "expected": "The chart renders with one bar."
                }),
            }) as Arc<dyn Provider>,
            &session_id,
        )
        .await
        .unwrap();

    // Answer the card as soon as it is parked, so the turn terminates. DENY:
    // nothing in this suite may create a real issue, and `issue::file_with_gh`
    // refuses from a test binary besides.
    let denier = tokio::spawn({
        let session_id = session_id.clone();
        async move {
            let registry = PendingUserActions::global();
            for _ in 0..600 {
                let ids: Vec<String> = registry
                    .pending_cards_for_session(&session_id)
                    .into_iter()
                    .filter_map(|message| {
                        message
                            .content
                            .into_iter()
                            .find_map(|content| match content {
                                MessageContent::ActionRequired(action) => match action.data {
                                    ActionRequiredData::ToolConfirmation { id, .. } => Some(id),
                                    _ => None,
                                },
                                _ => None,
                            })
                    })
                    .collect();
                if let Some(id) = ids.first() {
                    registry.resolve_in_session(
                        &session_id,
                        id,
                        UserActionOutcome::Denied {
                            permission: Permission::DenyOnce,
                        },
                        // A DENIAL needs no proof: the gate keys on
                        // `is_allowed()`, so a refusal lands from any surface.
                        // Only an *allow* on a proof-backed approval is
                        // restricted -- which is the property that stops this
                        // suite ever creating a real issue.
                        DecisionAuthority::unproven(),
                    );
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            false
        }
    });

    let mut stream = agent
        .reply(
            Message::user().with_text("report a bug"),
            SessionConfig {
                id: session_id.clone(),
                schedule_id: None,
                max_turns: Some(3),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            cancel_token,
        )
        .await
        .unwrap();

    let mut card: Option<(String, String)> = None;
    while let Some(event) = stream.next().await {
        let Ok(biorouter::agents::AgentEvent::Message(message)) = event else {
            continue;
        };
        for content in &message.content {
            if let MessageContent::ActionRequired(action) = content {
                if let ActionRequiredData::ToolConfirmation {
                    tool_name,
                    arguments,
                    ..
                } = &action.data
                {
                    card = Some((
                        tool_name.clone(),
                        arguments
                            .get("body")
                            .and_then(|body| body.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    ));
                }
            }
        }
    }
    drop(stream);
    assert!(denier.await.unwrap(), "the card was never parked at all");

    let (tool_name, body) = card.expect(
        "the approval card must be YIELDED from the reply stream. Without it the desktop \
         renders no dialog, the turn stops with no explanation, and the parked call sits \
         out its whole time-to-live unanswerable.",
    );
    assert_eq!(tool_name, PLATFORM_REPORT_BUG_TOOL_NAME);
    assert!(body.contains("**Describe the bug**"), "{body}");
    assert!(body.contains("A chart of a one-row table"), "{body}");
}

/// ⚠ The card must arrive **while the tool is still parked**, not merely at some
/// point in the turn.
///
/// `the_approval_card_is_yielded_out_of_the_reply_stream` above passed even when
/// this was broken, and the reason is worth stating: its denier polls the
/// `PendingUserActions` registry directly, so it answers the card whether or not
/// the card ever reached the stream. The tool then returns, and the queued
/// message is drained and yielded afterwards — late, but present. A test that
/// only asks "was it yielded" cannot see the defect.
///
/// So this one closes the loop through the stream itself: the reader signals
/// when it *yields* a card, and only then is the card answered. On the broken
/// code that signal never comes, the tool sits on its 15-minute TTL, and the
/// timeout below fails the test — which is exactly what a user experiences.
///
/// The defect: `handle_approved_and_denied_tools` awaits `dispatch_tool_call`
/// sequentially, and the `platform__*` tools are dispatched by the agent loop
/// itself, so their WHOLE BODY runs there — before `combined` exists and before
/// `next_batch_wake`, the batch's card drain, is ever entered. Extension tools
/// are unaffected: their `ToolCallResult.result` is a deferred future that runs
/// inside the batch.
#[tokio::test]
#[serial_test::serial(user_proof_available)]
async fn the_card_reaches_the_stream_while_the_tool_is_still_parked() {
    use biorouter::pending_user_action::{
        DecisionAuthority, PendingUserActions, UserActionOutcome,
    };
    use biorouter::permission::Permission;

    let _proof = ProofAvailable::set(true);
    let dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    let permission_manager = Arc::new(PermissionManager::new(dir.path().to_path_buf()));
    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        session_manager,
        permission_manager,
        None,
        BioRouterMode::Auto,
    )));
    let session = agent
        .config
        .session_manager
        .create_session(
            PathBuf::from("/workspace/demo"),
            "card-while-parked".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    let session_id = session.id.clone();
    agent
        .update_provider(
            Arc::new(ReportingProvider {
                turns: AtomicUsize::new(0),
                offered: Mutex::new(Vec::new()),
                arguments: serde_json::json!({
                    "action": "file",
                    "title": "Auto Visualiser renders a blank panel for a single-row dataset",
                    "description": "A chart of a one-row table produces an empty panel.",
                    "steps": ["Open a chat", "Ask for a chart of one row"],
                    "expected": "The chart renders with one bar."
                }),
            }) as Arc<dyn Provider>,
            &session_id,
        )
        .await
        .unwrap();

    // The card is answered ONLY once the stream has yielded it.
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<String>();
    let denier = tokio::spawn({
        let session_id = session_id.clone();
        async move {
            let Ok(id) = seen_rx.await else {
                return false;
            };
            PendingUserActions::global().resolve_in_session(
                &session_id,
                &id,
                UserActionOutcome::Denied {
                    permission: Permission::DenyOnce,
                },
                DecisionAuthority::unproven(),
            );
            true
        }
    });

    let drained = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut stream = agent
            .reply(
                Message::user().with_text("report a bug"),
                SessionConfig {
                    id: session_id.clone(),
                    schedule_id: None,
                    max_turns: Some(3),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                Some(CancellationToken::new()),
            )
            .await
            .unwrap();
        let mut seen_tx = Some(seen_tx);
        while let Some(event) = stream.next().await {
            let Ok(biorouter::agents::AgentEvent::Message(message)) = event else {
                continue;
            };
            for content in &message.content {
                if let MessageContent::ActionRequired(action) = content {
                    if let ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                        if let Some(tx) = seen_tx.take() {
                            let _ = tx.send(id.clone());
                        }
                    }
                }
            }
        }
    })
    .await;

    assert!(
        drained.is_ok(),
        "the turn never finished: the approval card was not yielded while the tool was \
         parked, so nothing could answer it and the parked call sat on its time-to-live. \
         That is what a user sees as a chat that silently stops."
    );
    assert!(
        denier.await.unwrap(),
        "the card never reached the reply stream at all"
    );
}

/// The belt to the denial's braces, and the reason it had to be rewritten.
///
/// `cfg(test)` is set only while the crate under compilation is built with
/// `--test`. This file links `biorouter` compiled NORMALLY, so `cfg!(test)` is
/// **false** here — the guard that three comments and a commit message credited
/// with making a test-time `gh issue create` impossible was, in this binary,
/// not present at all. What kept this suite honest was that it denies the card,
/// and the privileged `DecisionAuthority` constructors an approving test would
/// reach for are `pub`.
///
/// So the guard is a runtime check now, and this asserts it from the side
/// `cfg!(test)` cannot see. A failure here means a test that approves the card
/// would file a real, public, permanent issue on someone's tracker.
#[tokio::test]
async fn filing_is_refused_from_an_integration_test_binary() {
    let body_file = std::env::temp_dir().join("biorouter-bug-report-guard-probe.md");
    // A previous run under a REGRESSED guard would have left this behind, and a
    // stale file would then fail the assertion below after the guard was fixed.
    let _ = std::fs::remove_file(&body_file);
    let error = biorouter::agents::bug_report::issue::file_with_gh(
        "BaranziniLab/biorouter",
        "a title nobody should ever see on the tracker",
        "a body nobody should ever see on the tracker",
        &body_file,
    )
    .await
    .expect_err("a test binary must never reach `gh issue create`");

    assert!(
        error
            .to_string()
            .contains("refusing to create a GitHub issue"),
        "the refusal did not fire; this binary can file real issues: {error}"
    );
    // It bails before writing, so there is nothing to clean up — asserted, so a
    // later edit that moves the guard below the write is noticed.
    assert!(
        !body_file.exists(),
        "the guard ran after the body was written to disk"
    );
}

/// The compose path's twin of the guard above, from the same blind spot.
///
/// After approval the compose page is opened in the user's browser. A test
/// that approved a card would otherwise pop a browser — carrying the report in
/// its address — on whatever machine ran the suite, and `cfg!(test)` is false
/// in this binary, so only the runtime check can stop it.
#[tokio::test]
async fn the_compose_page_is_never_opened_from_an_integration_test_binary() {
    assert!(
        !biorouter::agents::bug_report::issue::open_in_browser(
            "https://example.invalid/biorouter-bug-report-guard-probe"
        )
        .await,
        "a test binary must never open a browser"
    );
}
