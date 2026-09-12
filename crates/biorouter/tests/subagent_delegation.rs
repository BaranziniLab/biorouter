//! SUB-NN gates: delegation to subagents, at the edges and under stress.
//!
//! Everything here drives the real agent loop through the real `subagent` tool;
//! only the provider is scripted (see `subagent_support`). The questions are the
//! ones a user asks of a delegating agent: did the child run, did its result come
//! back attached to *its own* call, does a failure surface honestly, and does
//! cancelling the parent take the children with it.

mod subagent_support;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::extension::{ExtensionConfig, PlatformExtensionContext};
use biorouter::agents::mcp_client::{McpClientTrait, McpMeta};
use biorouter::agents::workspace_extension::WorkspaceClient;
use biorouter::agents::{Agent, AgentConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::CallCapability;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use rmcp::model::{CallToolResult, Tool};
use serde_json::Value;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use subagent_support::{
    drain, structured_results, tool_responses, Call, Harness, ScriptedSubagentProvider,
};

fn call(id: &str, c: Call) -> (String, Call) {
    (id.to_string(), c)
}

fn shared_session_manager() -> Arc<SessionManager> {
    static SESSION_MANAGER: OnceLock<Arc<SessionManager>> = OnceLock::new();
    SESSION_MANAGER
        .get_or_init(|| {
            let data_dir = TempDir::new().expect("shared test data directory");
            let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
            std::mem::forget(data_dir);
            session_manager
        })
        .clone()
}

/// The steer `steering_mid_delegation_costs_no_subagent_results` sends. A const
/// because both the test and the provider watching for it in context have to
/// spell it the same way.
const STEER: &str = "also summarise what each of them found";

/// The rendezvous a [`ParkedParentProvider`] gives the test driving it: the
/// parent announces that it has reached the boundary after its spawn batch, and
/// does not proceed until the test releases it.
///
/// ⚠ **This replaces a wall-clock `sleep`, and the difference is the whole
/// point.** A steer is only accepted while the turn's interrupt queue is open,
/// and the loop closes that queue in `close_and_drain` the moment it commits to
/// exiting — which is BEFORE it waits for its children. So a test that wants to
/// steer mid-delegation has to establish two things at once: the children are in
/// flight, and the parent has not yet reached its exit check. The old harness
/// tried to buy that window by sleeping 250 ms inside the parent's second
/// provider call, i.e. by out-running the loop. Under load the two detached
/// child tasks are not scheduled inside 250 ms, the parent reaches
/// `close_and_drain` first, and `try_queue_soft_interrupt` answers `TurnEnded`
/// — a failure that is about scheduler luck and not about the product.
/// Measured on `origin/main` at `1b199445`: 7 failures in 18 runs of this binary
/// with six copies of it running at once.
///
/// A `Notify` permit is stored when nothing is waiting yet, so neither half of
/// the handshake depends on which side arrives first.
struct ParentRendezvous {
    /// Signalled once, when the parent parks after its spawn batch.
    parked: tokio::sync::Notify,
    /// Awaited by that parked call; the test signals it after queueing a steer.
    release: tokio::sync::Notify,
}

impl ParentRendezvous {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            parked: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }

    /// Wait for the parent to reach the post-spawn boundary. The timeout is a
    /// liveness guard for a hung turn, never the thing that makes the ordering
    /// hold — that is the handshake itself.
    async fn await_parked(&self) {
        tokio::time::timeout(Duration::from_secs(30), self.parked.notified())
            .await
            .expect("the parent reaches the boundary after its spawn batch");
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

/// A parent provider that PARKS at the boundary after the spawn batch instead of
/// sleeping there. See [`ParentRendezvous`].
struct ParkedParentProvider {
    inner: Arc<ScriptedSubagentProvider>,
    parent_calls: AtomicUsize,
    rendezvous: Arc<ParentRendezvous>,
    /// Set once a PARENT call is shown [`STEER`] in its context.
    ///
    /// Recorded here rather than counted as "one extra parent call", because the
    /// number of calls a turn makes after a steer is not a property of the steer:
    /// native supervision adds one of its own when the children report. What the
    /// test actually claims is that the model SAW the steer during this turn, and
    /// that is exactly what this observes.
    saw_steer: std::sync::atomic::AtomicBool,
}

impl ParkedParentProvider {
    /// Park the parent's FIRST post-spawn call, and only that one: the loop makes
    /// further calls once it consumes the steer and once the children report, and
    /// parking either would hang the turn on a release nobody sends.
    async fn park_after_spawn(&self, system_prompt: &str) {
        if system_prompt.contains("You are a specialized subagent") {
            return;
        }
        if self.parent_calls.fetch_add(1, Ordering::SeqCst) != 1 {
            return;
        }
        self.rendezvous.parked.notify_one();
        self.rendezvous.release.notified().await;
    }

    fn saw_steer(&self) -> bool {
        self.saw_steer.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for ParkedParentProvider {
    async fn complete(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        if !system_prompt.contains("You are a specialized subagent")
            && messages
                .iter()
                .any(|message| message.as_concat_text().contains(STEER))
        {
            self.saw_steer.store(true, Ordering::SeqCst);
        }
        self.park_after_spawn(system_prompt).await;
        self.inner.complete(system_prompt, messages, tools).await
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
        self.inner.get_model_config()
    }

    fn metadata() -> ProviderMetadata {
        ScriptedSubagentProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "parked-parent-scripted-subagent"
    }
}

async fn harness(batch: Vec<(String, Call)>) -> Harness {
    let (harness, parked) = build_harness(batch, false).await;
    assert!(parked.is_none(), "a plain harness parks nothing");
    harness
}

/// A harness whose parent parks at the boundary after its spawn batch, plus the
/// handle that releases it and counts its calls.
async fn harness_with_parked_parent(
    batch: Vec<(String, Call)>,
) -> (Harness, Arc<ParkedParentProvider>) {
    let (harness, parked) = build_harness(batch, true).await;
    (harness, parked.expect("the parked parent was requested"))
}

async fn build_harness(
    batch: Vec<(String, Call)>,
    park_parent: bool,
) -> (Harness, Option<Arc<ParkedParentProvider>>) {
    std::env::set_var("BIOROUTER_SUBAGENT_MAX_TURNS", "3");

    let work_dir = TempDir::new().expect("test working directory");
    let session_manager = shared_session_manager();
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
            "subagent-stress".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("parent session is created");
    let scripted = Arc::new(ScriptedSubagentProvider::new(batch));
    let ledger = scripted.ledger.clone();
    let parked = park_parent.then(|| {
        Arc::new(ParkedParentProvider {
            inner: Arc::clone(&scripted),
            parent_calls: AtomicUsize::new(0),
            rendezvous: ParentRendezvous::new(),
            saw_steer: std::sync::atomic::AtomicBool::new(false),
        })
    });
    let provider: Arc<dyn Provider> = match parked.clone() {
        Some(parked) => parked,
        None => scripted,
    };
    agent
        .update_provider(provider, &session.id)
        .await
        .expect("scripted provider binds");
    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "Developer tools".to_string(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("developer extension registers");

    (
        Harness {
            agent: Arc::new(agent),
            session_id: session.id,
            ledger,
            work_dir,
        },
        parked,
    )
}

#[derive(Debug)]
struct SpawnReceipt {
    call_id: String,
    handle: String,
    child_session_id: String,
}

#[derive(Debug)]
struct CollectedDelegation {
    status: String,
    summary: String,
    error: Option<String>,
}

fn result_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn spawn_receipts(messages: &[Message]) -> Vec<SpawnReceipt> {
    let structured = structured_results(messages);
    tool_responses(messages)
        .into_iter()
        .filter_map(|(call_id, text, is_error)| {
            let snapshot = structured.get(&call_id)?;
            let handle = snapshot.get("handle")?.as_str()?.to_string();
            let child_session_id = snapshot.get("child_session_id")?.as_str()?.to_string();

            assert!(!is_error, "background spawn {call_id} failed: {text}");
            assert!(
                text.contains("Subagent started in the background"),
                "spawn {call_id} must return the background contract: {text}"
            );
            assert!(
                text.contains(&handle),
                "spawn omitted handle {handle}: {text}"
            );
            assert!(
                text.contains(&child_session_id),
                "spawn omitted child session {child_session_id}: {text}"
            );
            assert!(
                text.contains("workspace_watch") && text.contains("workspace_read_conversation"),
                "spawn must teach the parent how to collect the child: {text}"
            );

            Some(SpawnReceipt {
                call_id,
                handle,
                child_session_id,
            })
        })
        .collect()
}

async fn workspace_call(
    workspace: &WorkspaceClient,
    parent_session_id: &str,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let arguments = serde_json::from_value(arguments).expect("workspace arguments are an object");
    let result = workspace
        .call_tool(
            name,
            Some(arguments),
            McpMeta::new(parent_session_id, CallCapability::public_enforced())
                .with_workspace_child_scope_only(true),
            CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("{name} transport failed: {error}"));
    let text = result_text(&result);
    assert_ne!(
        result.is_error,
        Some(true),
        "{name} rejected the parent's direct child: {text}"
    );
    result
}

async fn collect_delegations(
    h: &subagent_support::Harness,
    messages: &[Message],
) -> HashMap<String, CollectedDelegation> {
    let receipts = spawn_receipts(messages);
    assert!(
        !receipts.is_empty(),
        "the parent turn did not return any background delegation handles"
    );

    let workspace = WorkspaceClient::new(PlatformExtensionContext {
        extension_manager: None,
        session_manager: h.agent.config.session_manager.clone(),
    })
    .expect("workspace collector starts");
    let child_session_ids: Vec<_> = receipts
        .iter()
        .map(|receipt| receipt.child_session_id.clone())
        .collect();
    let watch = workspace_call(
        &workspace,
        &h.session_id,
        "workspace_watch",
        serde_json::json!({
            "session_ids": child_session_ids,
            "mode": "all",
            "timeout_s": 10
        }),
    )
    .await;
    let watch_text = result_text(&watch);
    assert!(
        watch_text.contains("Completed:"),
        "workspace_watch did not collect the delegated batch: {watch_text}"
    );
    let parent_turn_text = messages
        .iter()
        .map(Message::as_concat_text)
        .collect::<Vec<_>>()
        .join("\n");

    let mut collected = HashMap::new();
    for receipt in receipts {
        let handle =
            biorouter::agents::subagent_handle::get_for_session(&h.session_id, &receipt.handle)
                .unwrap_or_else(|| {
                    panic!(
                        "spawn response {} named missing child {}",
                        receipt.call_id, receipt.child_session_id
                    )
                });
        assert_eq!(handle.child_session_id, receipt.child_session_id);
        assert!(
            handle.latest_generation_collected(),
            "workspace_watch returned without collecting {}",
            receipt.child_session_id
        );
        let result = handle
            .result()
            .unwrap_or_else(|| panic!("{} has no terminal result", receipt.child_session_id));
        let collected_before_watch = parent_turn_text.contains(&result.summary);
        assert!(
            watch_text.contains(&receipt.child_session_id),
            "workspace_watch omitted {}: {watch_text}",
            receipt.child_session_id
        );
        if collected_before_watch {
            assert!(
                parent_turn_text.contains(&result.summary),
                "native parent supervision collected {} without injecting its exact result",
                receipt.child_session_id
            );
            assert!(
                watch_text.contains(&result.summary) || watch_text.contains("already idle"),
                "a previously collected child was neither repeated nor identified as idle: {watch_text}"
            );
        } else {
            assert!(
                watch_text.contains(&result.summary),
                "workspace_watch did not return the exact result for {}: {watch_text}",
                receipt.child_session_id
            );
        }

        let read = workspace_call(
            &workspace,
            &h.session_id,
            "workspace_read_conversation",
            serde_json::json!({
                "session_id": receipt.child_session_id,
                "view": "summary"
            }),
        )
        .await;
        let read_text = result_text(&read);
        assert!(
            read_text.contains(&result.summary),
            "workspace_read_conversation did not return the exact child outcome: {read_text}"
        );

        let previous = collected.insert(
            receipt.call_id.clone(),
            CollectedDelegation {
                status: result.status.as_str().to_string(),
                summary: result.summary,
                error: result.error,
            },
        );
        assert!(previous.is_none(), "duplicate delegation call id");
    }
    collected
}

/// SUB: a single delegation runs the child and returns its summary, attached to
/// the call that asked for it.
#[tokio::test]
async fn a_single_subagent_runs_and_returns_its_own_summary() {
    let h = harness(vec![call("only", Call::sub("solo", "ok:solo"))]).await;
    let messages = drain(&h.agent, &h.session_id)
        .await
        .expect("turn completes");

    let responses = tool_responses(&messages);
    assert_eq!(responses.len(), 1, "one call, one response: {responses:?}");
    let (id, text, is_error) = &responses[0];
    assert_eq!(id, "only");
    assert!(!is_error, "a clean background spawn is not an error");
    assert!(
        text.contains("Subagent started in the background"),
        "the spawn returns a supervision handle: {text}"
    );

    let collected = collect_delegations(&h, &messages).await;
    let only = &collected["only"];
    assert_eq!(only.status, "completed");
    assert!(
        only.summary.contains("child solo done"),
        "the parent's watch/read round trip returns the child's own summary: {}",
        only.summary
    );
    assert_eq!(h.ledger.distinct_started(), vec!["ok:solo".to_string()]);
}

/// SUB: several DIFFERENT subagents in one batch. The headline requirement —
/// every result must stay welded to the call that asked for it.
#[tokio::test]
async fn parallel_subagents_keep_their_results_separate() {
    let h = harness(vec![
        call("alpha", Call::sub("alpha", "ok:alpha")),
        call("beta", Call::sub("beta", "ok:beta")),
        call("gamma", Call::sub("gamma", "ok:gamma")),
    ])
    .await;
    let messages = drain(&h.agent, &h.session_id)
        .await
        .expect("turn completes");

    let responses = tool_responses(&messages);
    assert_eq!(responses.len(), 3, "every call answered: {responses:?}");

    for (id, text, is_error) in &responses {
        assert!(!is_error, "{id} should start cleanly: {text}");
        assert!(
            text.contains("Subagent started in the background"),
            "response {id} carries the background handle contract: {text}"
        );
    }

    let collected = collect_delegations(&h, &messages).await;
    assert_eq!(collected.len(), 3, "every spawned child was collected");
    for (id, result) in &collected {
        assert_eq!(result.status, "completed", "{id}: {}", result.summary);
        assert!(
            result.summary.contains(&format!("child {id} done")),
            "collected result {id} carries {id}'s summary, not a sibling's: {}",
            result.summary
        );
        // Cross-talk check: no other child's summary leaked into this result.
        for other in ["alpha", "beta", "gamma"] {
            if other != id {
                assert!(
                    !result.summary.contains(&format!("child {other} done")),
                    "result {id} leaked {other}'s summary: {}",
                    result.summary
                );
            }
        }
    }

    let mut started = h.ledger.distinct_started();
    started.sort();
    assert_eq!(started, vec!["ok:alpha", "ok:beta", "ok:gamma"]);
}

/// SUB: a child cannot spawn a child. It is refused, the refusal reaches the
/// child as an ordinary tool error, and the child still returns a summary — the
/// recursion stops without taking the run down with it.
#[tokio::test]
async fn a_subagent_cannot_spawn_a_subagent_and_fails_cleanly() {
    let h = harness(vec![call("nester", Call::sub("nester", "nest:nester"))]).await;
    let messages = drain(&h.agent, &h.session_id)
        .await
        .expect("turn completes");

    let responses = tool_responses(&messages);
    assert_eq!(responses.len(), 1);
    let (_, text, is_error) = &responses[0];
    assert!(
        !is_error,
        "starting the child must not fail the parent's call: {text}"
    );
    let collected = collect_delegations(&h, &messages).await;
    let nester = &collected["nester"];
    assert_eq!(nester.status, "completed");
    assert!(
        nester.summary.contains("could not nest"),
        "the child reported the refusal through watch/read: {}",
        nester.summary
    );

    // The grandchild never ran: only the child's own script was ever started.
    let started = h.ledger.distinct_started();
    assert_eq!(
        started,
        vec!["nest:nester".to_string()],
        "a grandchild must never start: {started:?}"
    );
}

/// SUB: three ways a child can go wrong, all in one batch — each surfaces
/// honestly and none of them swallows the others.
#[test]
fn failing_silent_and_slow_children_all_surface() {
    biorouter::execution::runtime::build_agent_runtime()
        .expect("agent runtime builds")
        .block_on(async {
            tokio::spawn(async {
                let h = harness(vec![
                    call("broken", Call::sub("broken", "fail:broken")),
                    call("mute", Call::sub("mute", "silent:mute")),
                    call("plodder", Call::sub("plodder", "slow:plodder:400")),
                ])
                .await;
                let messages = drain(&h.agent, &h.session_id)
                    .await
                    .expect("turn completes");

                let responses = tool_responses(&messages);
                assert_eq!(responses.len(), 3, "nothing vanished: {responses:?}");
                assert!(
                    responses.iter().all(|(_, text, is_error)| !is_error
                        && text.contains("Subagent started in the background")),
                    "all three calls must return supervision handles: {responses:?}"
                );
                let collected = collect_delegations(&h, &messages).await;

                let broken = &collected["broken"];
                assert!(
                    broken.status == "error",
                    "SUB-02: workspace_watch/read must identify an aborted child as an error: {}",
                    broken.summary
                );
                assert!(
                    broken.summary.contains("Subagent failed"),
                    "the failure says so in words: {}",
                    broken.summary
                );
                assert!(
                    broken.error.as_deref().is_some_and(
                        |error| error.contains("scripted provider failure for child broken")
                    ),
                    "and names the underlying cause: {}",
                    broken.summary
                );

                // A child that only ever calls tools and never writes a summary. It runs out
                // of turns, and the loop's own stop notice is what comes back.
                //
                // SUB-03 (documented, unfixed): that notice is an ordinary assistant text
                // message, so the envelope grades this run `completed`. The prose is honest
                // — it says outright that it stopped for the cap, "not because the task is
                // necessarily complete" — and this test pins that the parent receives it
                // verbatim, which is what lets the model react. The *status* is the part
                // that overstates, and correcting it needs a structural signal the agent
                // loop does not yet emit for a turn-limit stop. Pinned as-is so the day the
                // loop grows that signal, this assertion is the thing that fails and points
                // at the envelope.
                let mute = &collected["mute"];
                assert!(
                    mute.summary.contains("action limit for this turn"),
                    "the turn-cap stop notice must reach the parent verbatim: {}",
                    mute.summary
                );
                assert_eq!(
                    mute.status, "completed",
                    "SUB-03: a turn-capped child still grades `completed`; see the comment above"
                );
                assert!(
                    !mute.summary.contains("No text content in last message"),
                    "never the old lossy placeholder: {}",
                    mute.summary
                );

                let plodder = &collected["plodder"];
                assert_eq!(plodder.status, "completed");
                assert!(plodder.summary.contains("child plodder done"));
            })
            .await
            .expect("delegation test runs on an agent worker");
        });
}

/// SUB: many children at once. Nothing is lost, nothing crosses over, and the
/// concurrency cap is respected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wide_batch_of_subagents_loses_nothing() {
    const WIDTH: usize = 12;
    let batch: Vec<_> = (0..WIDTH)
        .map(|i| {
            let name = format!("w{i}");
            call(&name.clone(), Call::sub(&name, &format!("ok:{name}")))
        })
        .collect();

    let h = harness(batch).await;
    let messages = drain(&h.agent, &h.session_id)
        .await
        .expect("turn completes");

    let responses = tool_responses(&messages);
    assert_eq!(
        responses.len(),
        WIDTH,
        "every one of {WIDTH} subagents returned a handle: {responses:?}"
    );
    for (id, text, is_error) in &responses {
        assert!(!is_error, "{id} failed to start: {text}");
        assert!(
            text.contains("Subagent started in the background"),
            "{id} did not return a supervision handle: {text}"
        );
    }

    let collected = collect_delegations(&h, &messages).await;
    assert_eq!(collected.len(), WIDTH, "every child result was collected");
    for (id, result) in &collected {
        assert_eq!(result.status, "completed", "{id}: {}", result.summary);
        assert!(
            result.summary.contains(&format!("child {id} done")),
            "{id} got somebody else's answer: {}",
            result.summary
        );
    }

    assert_eq!(
        h.ledger.distinct_started().len(),
        WIDTH,
        "every child actually ran"
    );

    // The fork-bomb guard's default ceiling is 8 concurrent subagents.
    let peak = h.ledger.peak();
    assert!(
        peak <= 8,
        "concurrency cap breached: {peak} children ran at once"
    );
}

/// SUB: a batch that mixes delegation with ordinary parallel tool calls — the
/// two dispatch paths coexist and each result still lands on its own call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subagents_and_ordinary_tools_share_a_batch() {
    let h = harness(vec![
        call("sub_one", Call::sub("sub_one", "ok:sub_one")),
        call("shell_one", Call::Shell("echo shell-one-ran".into())),
        call("sub_two", Call::sub("sub_two", "ok:sub_two")),
        call("shell_two", Call::Shell("echo shell-two-ran".into())),
    ])
    .await;
    let messages = drain(&h.agent, &h.session_id)
        .await
        .expect("turn completes");

    let responses = tool_responses(&messages);
    assert_eq!(responses.len(), 4, "all four answered: {responses:?}");

    let by_id = |want: &str| {
        responses
            .iter()
            .find(|(id, ..)| id == want)
            .unwrap_or_else(|| panic!("no response for {want}"))
            .1
            .clone()
    };
    assert!(by_id("sub_one").contains("Subagent started in the background"));
    assert!(by_id("sub_two").contains("Subagent started in the background"));
    assert!(by_id("shell_one").contains("shell-one-ran"));
    assert!(by_id("shell_two").contains("shell-two-ran"));

    let collected = collect_delegations(&h, &messages).await;
    assert_eq!(collected.len(), 2);
    assert!(collected["sub_one"].summary.contains("child sub_one done"));
    assert!(collected["sub_two"].summary.contains("child sub_two done"));
}

/// SUB: the user steers while subagents are in flight. A soft interrupt must
/// not cost the delegated work — every child still reports, and the steer lands
/// in the same turn.
///
/// ⚠ **Both preconditions are established by a handshake, never by a sleep.**
/// See [`ParentRendezvous`] for what the wall-clock version measured instead
/// (scheduler luck) and how often it was wrong. The parent is parked inside its
/// post-spawn provider call for the whole middle of this test, which is what
/// makes "the children are in flight" and "the turn's steer window is still
/// open" simultaneously true rather than merely likely.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn steering_mid_delegation_costs_no_subagent_results() {
    let (h, parked) = harness_with_parked_parent(vec![
        call("left", Call::sub("left", "slow:left:600")),
        call("right", Call::sub("right", "slow:right:600")),
    ])
    .await;

    let agent = h.agent.clone();
    let session_id = h.session_id.clone();
    let turn = tokio::spawn(async move { drain(&agent, &session_id).await });

    // 1. The spawn batch has dispatched and the parent is parked, so the loop
    //    cannot reach the exit check that closes the interrupt queue.
    parked.rendezvous.await_parked().await;
    // 2. Both children really are inside the provider. Bounded only as a hung-run
    //    guard: the parent is parked, so nothing is racing this.
    tokio::time::timeout(Duration::from_secs(30), async {
        while h.ledger.distinct_started().len() != 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("both children reach the provider while the parent is parked");
    // 3. Steer, with the window provably open.
    h.agent
        .try_queue_soft_interrupt(STEER.to_string(), None)
        .expect("the parent turn is still accepting a steer while its children run");
    parked.rendezvous.release();

    let messages = turn
        .await
        .expect("turn task joins")
        .expect("turn completes");

    let responses = tool_responses(&messages);
    assert_eq!(
        responses.len(),
        2,
        "steering must not drop a delegation handle: {responses:?}"
    );
    let collected = collect_delegations(&h, &messages).await;
    assert!(collected["left"].summary.contains("child left done"));
    assert!(collected["right"].summary.contains("child right done"));

    let steer_landed = messages
        .iter()
        .filter(|m| m.role == rmcp::model::Role::User)
        .flat_map(|m| m.content.iter())
        .any(|c| match c {
            biorouter::conversation::message::MessageContent::Text(t) => t.text.contains(STEER),
            _ => false,
        });
    assert!(steer_landed, "the steer reached the conversation");
    assert!(
        !h.agent.has_soft_interrupts(),
        "the queued steer was consumed, not left pending"
    );
    // The claim in this test's name that the old timing could not check: the steer
    // was answered INSIDE this turn. `close_and_drain` finds it at the exit check,
    // reopens the loop for one more step, and that step puts it in the model's
    // context. A turn that merely persisted it on the way out — which is what
    // happens whenever the steer lands after the exit check — would still satisfy
    // every assertion above, because `settle_soft_interrupts_for_turn` persists
    // and yields it either way.
    assert!(
        parked.saw_steer(),
        "the steer must reach the model in this turn, not be carried over past its end"
    );
}

/// Issue #56 DR-26 / Task 50 Step 2: **a subagent inherits its parent's
/// cross-affiliation grants and can never exceed them.**
///
/// The inheritance is `privacy::grant::is_granted`'s walk up
/// `sessions.parent_session_id`, and this is the end-to-end half of it: that
/// walk is only meaningful if a REAL delegation stamps that column, and only
/// safe if a spawn manufactures no authority of its own. Both are asserted here,
/// against a child this run actually created.
///
/// ⚠ **The mint side is deliberately NOT here, and could not be.** Writing a
/// grant requires naming `privacy::grant`'s `UserCrossAffiliationGrant`, and
/// Task 49's `the_proof_of_user_is_constructed_in_exactly_one_place` fails the
/// build for any file under `crates/` outside that module and the one HTTP
/// handler whose *code* mentions it — an integration binary cannot mint one,
/// which is the control working. (That audit skips comments, so this paragraph
/// may name the type; it did not until review found it scanning whole files, and
/// the contortion of writing a comment that avoids a word is exactly the tax a
/// comment-blind audit charges. `grant::record_for_test` is `#[cfg(test)]
/// pub(crate)`, so it is not reachable from an integration binary either —
/// making it reachable is the one repair that would actually cost something.)
///
/// So the granted-parent direction lives in
/// `privacy::grant::tests::a_subagent_inherits_its_parents_grants_and_the_parent_inherits_nothing`,
/// where `record_for_test` IS reachable, and it is the discriminating half: the
/// "nothing is granted" assertions below would pass equally against an
/// `is_granted` that returned `false` unconditionally. What that test cannot
/// see, and this one can, is whether production ever links the child to the
/// parent at all: with `parent_session_id` unstamped, the walk silently finds
/// nothing and every inheritance assertion elsewhere stays green while the
/// feature is dead.
///
/// ⚠ **Residual, on the gate's own wording.** "A child cannot hold a grant its
/// parent lacks" is not what `is_granted` enforces: its walk reads *upward*, so
/// a grant recorded directly on a child is honoured for that child and invisible
/// to the parent. Nothing in production mints one there — the single writer is
/// an HTTP handler behind `X-User-Action`, i.e. the user — so it is latent
/// rather than live, and narrowing the walk to "the ancestor chain must hold it"
/// would silently void an approval a user gave at the point of refusal inside a
/// subagent's turn. What IS enforced, and what this test asserts, is the half
/// DR-26 states: a spawn manufactures no authority of its own.
#[tokio::test]
async fn a_subagent_is_linked_to_its_parent_and_gains_no_grant_by_being_spawned() {
    let h = harness(vec![call("only", Call::sub("solo", "ok:solo"))]).await;
    let messages = drain(&h.agent, &h.session_id)
        .await
        .expect("turn completes");
    let collected = collect_delegations(&h, &messages).await;
    assert!(collected["only"].summary.contains("child solo done"));

    let sm = &h.agent.config.session_manager;
    // `list_sessions()` is the History projection and filters `sub_agent` rows
    // out; asking for the type explicitly is what makes an absent child
    // distinguishable from a hidden one.
    let children: Vec<_> = sm
        .list_sessions_by_types(&[biorouter::session::session_manager::SessionType::SubAgent])
        .await
        .expect("the session store is readable")
        .into_iter()
        .filter(|s| s.parent_session_id.as_deref() == Some(h.session_id.as_str()))
        .collect();
    assert_eq!(
        children.len(),
        1,
        "a delegation must stamp exactly one child with this parent's id; without \
         that column the grant walk has nothing to climb, and 'a subagent inherits \
         its parent's grants' is true of nothing"
    );
    let child = &children[0];
    assert_ne!(child.id, h.session_id);

    // Nobody granted anything, so nothing is granted — at the parent, at the
    // child, and for every shape of model affiliation the triple can take. A
    // spawn is not a way to acquire authority.
    let institution = biorouter::privacy::affiliation::ModelAffiliation::institution(
        biorouter::privacy::affiliation::InstitutionId::new("ucsf"),
    );
    for model in [
        None,
        Some(biorouter::privacy::affiliation::ModelAffiliation::Local),
        Some(institution),
    ] {
        for session in [h.session_id.as_str(), child.id.as_str()] {
            assert!(
                !biorouter::privacy::grant::is_granted(sm, session, "ucsfomopagent", model).await,
                "session {session} holds a cross-affiliation grant for {model:?} that no \
                 user ever gave"
            );
        }
    }
}
