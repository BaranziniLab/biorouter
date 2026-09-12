use crate::{
    agents::{
        subagent_result::{SubagentResult, SubagentTokens},
        subagent_task_config::TaskConfig,
        Agent, AgentConfig, AgentEvent, SessionConfig,
    },
    conversation::{
        message::{Message, MessageContent},
        Conversation,
    },
    prompt_template::render_global_file,
    session::SessionManager,
    workflow::Workflow,
};
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use rmcp::model::Role;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

#[derive(Serialize)]
struct SubagentPromptContext {
    max_turns: usize,
    subagent_id: String,
    task_instructions: String,
    tool_count: usize,
    available_tools: String,
}

/// Why a child's turn ended early: `(wire code, human-readable message)`.
type TurnAbort = (String, String);

type AgentMessagesFuture =
    Pin<Box<dyn Future<Output = Result<(Conversation, Option<String>, Option<TurnAbort>)>> + Send>>;

fn delegated_initial_message(user_task: String, mut pending: Vec<Message>) -> Message {
    if pending.is_empty() {
        return Message::user().with_text(user_task);
    }

    let mut initial = pending.remove(0);
    initial.role = Role::User;
    let mut content = Message::user().with_text(user_task).content;
    content.push(MessageContent::text(
        "\n\nHuman steering received before this delegated run started:\n",
    ));
    content.append(&mut initial.content);
    for mut additional in pending {
        content.push(MessageContent::text(
            "\n\nAdditional pre-start user steering:\n",
        ));
        content.append(&mut additional.content);
    }
    initial.content = content;
    initial
}

/// What [`ensure_initial_user_direct_is_durable`] established about the accepted
/// pre-start steering that rode into the delegated prompt.
///
/// The distinction that matters is between the two ends and the two middles: a
/// caller may acknowledge the initialization queue only for a durability this
/// call actually VERIFIED. `NotApplicable` and `Unverified` verified nothing, so
/// settling on either would retire the recovery copy of a message that may exist
/// in no transcript at all.
enum InitialInputDurability {
    /// The delegated message carries no `UserDirect` provenance, so no accepted
    /// steering rode into it and there is nothing to make durable.
    NotApplicable,
    /// `Agent::reply` already persisted it; the message carries the row's id.
    /// Nothing to publish — `reply` publishes its own copy as the stream's first
    /// event.
    AlreadyDurable(Message),
    /// Written here, because `reply` returned without persisting it (a privacy
    /// refusal or a command error yields its own stream and never reaches the
    /// prologue's write). The caller owns publishing it.
    Inserted(Message),
    /// The idempotency probe could not READ the transcript, so durability is
    /// unknown. Nothing was written — a blind write is how you get two rows —
    /// and nothing may be settled.
    Unverified(anyhow::Error),
}

impl InitialInputDurability {
    /// Whether the initialization queue may be acknowledged.
    ///
    /// The invariant on [`crate::agents::subagent_handle::settle_initial_runtime_inputs`],
    /// in one place both call sites read. It was previously applied on the error
    /// branch only, which left the success branch settling on a durability it
    /// had not established.
    fn verified(&self) -> bool {
        matches!(self, Self::AlreadyDurable(_) | Self::Inserted(_))
    }
}

async fn ensure_initial_user_direct_is_durable(
    session_manager: &SessionManager,
    session_id: &str,
    initial: Option<Message>,
) -> Result<InitialInputDurability> {
    let Some(mut message) = initial else {
        return Ok(InitialInputDurability::NotApplicable);
    };

    // ⚠ The probe is NOT conditioned on the message carrying an id. It used to
    // be, and an id-less accepted message therefore skipped it and always took
    // the insert path — after `Agent::reply` had already stored the same
    // combined prompt under a uid it minted itself. That is production-reachable
    // (`biorouter session send` posts `"id": null`) and produced two rows for
    // one steer, the second of them agent-invisible. `add_message` cannot catch
    // it either: with no id it mints a fresh uuid, so its own replay detector
    // never fires.
    //
    // Identity for an id-less message is therefore the content tuple, and it is
    // the SAME predicate the queue admits by — see `is_same_accepted_input`.
    let existing = match session_manager.get_session(session_id, true).await {
        Ok(session) => session.conversation.and_then(|conversation| {
            conversation
                .messages()
                .iter()
                .find(|stored| {
                    crate::agents::subagent_handle::is_same_accepted_input(&message, stored)
                })
                .cloned()
        }),
        // A transient read failure (the documented two-daemon SQLITE_BUSY case)
        // must not take down a stream that is already live and whose prompt is
        // very likely already durable. Report it instead of `?`-ing out of a
        // started run; the caller keeps the recovery copy because nothing here
        // was verified.
        Err(read_error) => return Ok(InitialInputDurability::Unverified(read_error)),
    };
    if let Some(existing) = existing {
        message.id = existing.id;
        return Ok(InitialInputDurability::AlreadyDurable(message));
    }

    message = message.with_visibility(true, false);
    session_manager
        .add_message_adopting_uid(session_id, &mut message)
        .await?;
    Ok(InitialInputDurability::Inserted(message))
}

/// BR-71 §4.2 glass-box: a child turn's bracket on the session bus, closed on
/// **every** exit.
///
/// The bus contract is `TurnStarted` then exactly one terminal. Honouring only
/// the happy path is not enough — an observer that saw the start and never sees
/// a terminal waits forever on a turn that is long over, and the daemon has
/// already told it the child is busy (the server turn lease is taken before the
/// run begins). So the terminal is published from `Drop`, which is the only
/// construct that also covers a panic unwinding through the run and the future
/// being dropped outright (a `tokio` task abort — invisible to any
/// cancellation-token probe, because nothing was cancelled). This is the same
/// guarantee `biorouter-server`'s `supervise_turn` gives the interactive runner.
///
/// [`close`](BusTurnBracket::close) is the normal path and disarms the drop, so
/// a run never publishes two terminals.
struct BusTurnBracket {
    session_id: String,
    /// The run's cancellation token, so the *drop* path can tell a stopped run
    /// from a failed one. It is only read on that path: a run that reaches
    /// `close` passes the reason it computed itself.
    cancel_probe: Option<CancellationToken>,
    /// `false` once a terminal has been published, by either path.
    open: bool,
}

impl BusTurnBracket {
    /// Publish `TurnStarted` and take responsibility for the terminal.
    fn open(session_id: String, turn_id: String, cancel_probe: Option<CancellationToken>) -> Self {
        crate::session_events::publish(
            &session_id,
            crate::session_events::SessionBusEvent::TurnStarted { turn_id },
        );
        Self {
            session_id,
            cancel_probe,
            open: true,
        }
    }

    /// Close the bracket with the reason the run computed.
    fn close(mut self, reason: &str) {
        self.publish_terminal(reason);
    }

    /// Whether the run's token was tripped — the `cancelled` rung of the ladder.
    fn run_cancelled(&self) -> bool {
        self.cancel_probe
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    fn publish_terminal(&mut self, reason: &str) {
        if !self.open {
            return;
        }
        self.open = false;
        crate::session_events::publish(
            &self.session_id,
            crate::session_events::SessionBusEvent::TurnFinished {
                reason: reason.to_string(),
                // `None`, and that is the documented value here, not an
                // omission: `SessionBusEvent`'s doc reserves `Some(..)` for
                // brackets published after the BR-52 authoritative store read,
                // which a subagent run — headless of the daemon — never
                // performs. Synthesising `TokenState::default()` would put a
                // plausible-looking all-zero reading on the wire.
                token_state: None,
            },
        );
    }
}

impl Drop for BusTurnBracket {
    fn drop(&mut self) {
        // A run that never reached `close` did not end normally. It was either
        // stopped or it failed, and only the token can tell those apart.
        let reason = if self.run_cancelled() {
            "cancelled"
        } else {
            "error"
        };
        self.publish_terminal(reason);
    }
}

/// RAII release of a child's `AgentManager` registration, on every exit path a
/// run can take — including a panic unwinding through it and the future being
/// dropped outright.
///
/// Module scope, not a nested `struct` inside `get_agent_messages`, so the
/// no-runtime path below can be tested.
struct Deregister {
    manager: Option<(
        std::sync::Arc<crate::execution::manager::AgentManager>,
        std::sync::Arc<Agent>,
    )>,
    session_id: String,
}

impl Drop for Deregister {
    fn drop(&mut self) {
        let Some((manager, agent)) = self.manager.take() else {
            return;
        };
        let session_id = std::mem::take(&mut self.session_id);
        // `tokio::spawn` PANICS when there is no runtime, and this guard can be
        // dropped without one: a future dropped during runtime shutdown, or
        // moved out of the runtime that built it. A panic inside `Drop` while
        // another panic unwinds ABORTS the process, so an unconditional spawn
        // here turns a shutdown race into a crash. Ask for the handle instead.
        //
        // The release is spawned, not awaited, because `Drop` cannot be async —
        // so a child stays resolvable for a scheduler-dependent moment after its
        // run ends (which is why the tests poll rather than assume) and, with no
        // runtime, not released at all. Leaking a pin in a process that is
        // exiting costs nothing; aborting it costs the user's session.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    manager.deregister_agent_if_same(&session_id, &agent).await;
                });
            }
            Err(_) => tracing::debug!(
                "no tokio runtime while releasing the subagent registration for {session_id}; \
                 the pin is left to the process exit"
            ),
        }
    }
}

/// The extension that carries the workspace tools (and, since BR-71 decision 22,
/// the spawn tool). Not reachable from `Agent::SPAWN_EXTENSION`, which is
/// private to `agent.rs`.
const WORKSPACE_EXTENSION_NAME: &str = "workspace";

/// BR-71 §5 belt-and-braces beside the dispatch guard: the workspace extension
/// is never loaded into a child. NOTE the interaction with Task 18: a child has
/// `SessionType::SubAgent`, so `subagents_enabled` is already false for it (the
/// `Some(SessionType::SubAgent)` early return inside `subagents_enabled`) and
/// the auto-injection never fires either — this strip covers the case where the
/// PARENT's inherited extension list carries an explicitly user-enabled
/// `workspace` entry.
fn strip_workspace_extension(
    extensions: Vec<crate::agents::extension::ExtensionConfig>,
) -> Vec<crate::agents::extension::ExtensionConfig> {
    // ⚠ **Case-INSENSITIVE, because the two spellings are both real.** The
    // registry key is `workspace`; the name the daemon puts in a config entry
    // and sends to the GUI is `Workspace` (`workspace_extension::EXTENSION_NAME`).
    // A `!=` comparison therefore missed the entry that actually appears in an
    // inherited list, and a measured child was advertised seven workspace tools
    // including `workspace_read_conversation` and `workspace_send_prompt`.
    //
    // The security boundary held: dispatch refuses them for a SubAgent whatever
    // is advertised. But the child was invited to call tools that always fail
    // and spent a turn doing it, and the tab header reported a grant the child
    // did not hold.
    //
    // `has_non_injected_extensions` already compares this same name with
    // `eq_ignore_ascii_case`, so the two were disagreeing about one word. They
    // agree now.
    extensions
        .into_iter()
        .filter(|e| !e.name().eq_ignore_ascii_case(WORKSPACE_EXTENSION_NAME))
        .collect()
}

/// Load a child's granted extensions, returning **the ones that actually made
/// it in**.
///
/// ⚠ A failure here is logged and skipped, not fatal: a child that lost one of
/// six extensions still runs, and refusing the whole spawn over it would be the
/// worse trade. The consequence is that "what was requested" and "what the
/// child holds" are different lists on exactly the spawns that went wrong — so
/// every claim made to the user about this child (the runtime profile projection
/// which `GET /sessions/{id}/extensions` serves to the tab header, and the spawn
/// record's "Granted extensions" prose) must be built from the return value,
/// never from the input.
async fn load_granted_extensions(
    agent: &Agent,
    extensions: Vec<crate::agents::extension::ExtensionConfig>,
) -> Vec<crate::agents::extension::ExtensionConfig> {
    let mut loaded = Vec::new();
    for extension in strip_workspace_extension(extensions) {
        match agent.add_extension(extension.clone()).await {
            Ok(()) => loaded.push(extension),
            Err(e) => debug!(
                "Failed to add extension '{}' to subagent: {}",
                extension.name(),
                e
            ),
        }
    }
    loaded
}

/// Standalone function to run a complete subagent task, returning a structured
/// result envelope. A run that fails, or one that ends on a tool call without a
/// final text message, still yields a meaningful `SubagentResult` (BR-40) —
/// never the old lossy "No text content in last message" string.
pub async fn run_complete_subagent_task(
    config: AgentConfig,
    workflow: Workflow,
    task_config: TaskConfig,
    return_last_only: bool,
    session_id: String,
    cancellation_token: Option<CancellationToken>,
) -> SubagentResult {
    let session_manager = config.session_manager.clone();

    // BR-71 reconciliation #2 — one token per run, addressable from everywhere:
    // a child of the parent-supplied token (parent-cancel still propagates to
    // the child; cancelling the CHILD never kills the parent's turn), handed to
    // the server turn lease, the active-work guard, and the agent loop alike.
    let run_token = cancellation_token
        .as_ref()
        .map(tokio_util::sync::CancellationToken::child_token)
        .unwrap_or_default();

    // Hold the server's per-session turn lock for the run when the daemon is
    // present (headless: None — today's behavior). Makes is_turn_active(child)
    // true, keeps one-turn-per-session, and routes POST /agent/cancel /
    // workspace_close scope:"turn" / the tab's Stop to run_token.
    let _turn_lease: Option<Box<dyn crate::workspace_services::WorkspaceTurnLease>> =
        match crate::workspace_services::get() {
            Some(services) => match services.begin_turn(&session_id, run_token.clone()) {
                Ok(lease) => Some(lease),
                Err(conflict) => {
                    return SubagentResult::from_error(format!(
                        "subagent session is unexpectedly busy: {conflict}"
                    ));
                }
            },
            None => None,
        };

    // Surface this subagent in the process-wide "active work" view (BR-42) for
    // the run's whole lifetime. The guard deregisters on drop, so an early
    // return or panic never leaks a phantom "still running" entry. Cancel routes
    // to the run's own token — always present now, so the run is addressable
    // whether or not the parent supplied one.
    let _active_work = {
        use biorouter_mcp::active_work::{ActiveWorkGuard, ActiveWorkKind};
        let title = subagent_work_title(&workflow);
        // Was `cancellation_token.clone().map(...)`, i.e. None when the parent
        // supplied no token. Now always Some, built from `run_token`, so the
        // active-work cancel reaches the run whether or not the parent had one.
        let cancel: std::sync::Arc<dyn Fn() + Send + Sync> = {
            let token = run_token.clone();
            std::sync::Arc::new(move || token.cancel())
        };
        ActiveWorkGuard::register(
            ActiveWorkKind::Subagent,
            title,
            Some(format!("child session {session_id}")),
            Some(task_config.parent_session_id.clone()),
            Some(cancel),
        )
    };

    let (messages, final_output, aborted) = match get_agent_messages(
        config,
        workflow,
        task_config,
        session_id.clone(),
        // BR-71 §4.2: the bus bracket's turn id. The server lease's id when the
        // daemon handed us one, so an observer correlates the child's turn with
        // POST /agent/cancel's `turn_id`; `None` headless, where the run mints a
        // stable synthetic id instead.
        _turn_lease
            .as_ref()
            .map(|lease| lease.turn_id().to_string()),
        Some(run_token.clone()),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let mut result = SubagentResult::from_error(format!("Failed to execute task: {e}"));
            if run_token.is_cancelled() {
                result.mark_cancelled();
            }
            result.human_intervened = session_manager
                .get_session(&session_id, true)
                .await
                .ok()
                .and_then(|session| session.conversation)
                .as_ref()
                .is_some_and(super::subagent_result::conversation_has_user_direct);
            result.tokens = fetch_subagent_tokens(&session_manager, &session_id).await;
            return result;
        }
    };

    // An aborted turn is a failure even though the loop left a perfectly
    // well-formed assistant message behind explaining it. Deciding this here,
    // rather than letting `from_conversation` read that message as a summary, is
    // what keeps a subagent that never ran from reporting `completed`.
    let mut result = match aborted {
        Some((code, message)) => SubagentResult::from_aborted_turn(&messages, &code, message),
        None => SubagentResult::from_conversation(&messages, final_output, return_last_only),
    };
    if run_token.is_cancelled() {
        result.mark_cancelled();
    }
    result.human_intervened = super::subagent_result::conversation_has_user_direct(&messages);
    result.tokens = fetch_subagent_tokens(&session_manager, &session_id).await;
    result
}

/// A short, human-readable label for the active-work view: the subagent's task
/// prompt (or, failing that, its instructions), collapsed to one line and
/// truncated. Falls back to a generic label when the workflow carries neither.
fn subagent_work_title(workflow: &Workflow) -> String {
    let raw = workflow
        .prompt
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(workflow.instructions.as_deref())
        .unwrap_or("subagent task");
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title: String = one_line.chars().take(120).collect();
    if one_line.chars().count() > 120 {
        title.push('…');
    }
    title
}

/// Read the child session's lifetime token totals for the result envelope.
/// Best-effort: a missing session or all-zero counts yields `None`.
async fn fetch_subagent_tokens(
    session_manager: &SessionManager,
    session_id: &str,
) -> Option<SubagentTokens> {
    let session = session_manager.get_session(session_id, false).await.ok()?;
    let total = session.accumulated_total_tokens.unwrap_or(0);
    let input = session.accumulated_input_tokens.unwrap_or(0);
    let output = session.accumulated_output_tokens.unwrap_or(0);
    if total == 0 && input == 0 && output == 0 {
        return None;
    }
    Some(SubagentTokens {
        total,
        input,
        output,
    })
}

/// BR-71 §4.4: persist the child's rendered spawn context as its first message
/// — user_visible (the tab header shows it), agent_visible: false (the child's
/// model context already receives it as the system override; storing it
/// visibly must not double-inject it). Also stamps parent_session_id. The
/// record carries ALL grants the issue names — extensions, skills, and the
/// knowledge bases — so `workspace_read_conversation view:"spawn_context"` and
/// the tab header can show them without a second source of truth.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_spawn_context(
    session_manager: &SessionManager,
    child_session_id: &str,
    parent_session_id: &str,
    rendered_system_prompt: &str,
    task_instructions: &str,
    extension_names: &[String],
    skill_names: &[String],
    knowledge_bases: &[String],
) -> Result<()> {
    use crate::conversation::message::{MessageProvenance, ProvenanceKind};

    session_manager
        .update(child_session_id)
        .parent_session_id(Some(parent_session_id.to_string()))
        .apply()
        .await?;

    let body = format!(
        "## Subagent spawn context\n\nSpawned by session: {parent_session_id}\n\n\
         ### Task instructions\n{task_instructions}\n\n\
         ### Granted extensions\n{}\n\n\
         ### Granted skills\n{}\n\n\
         ### Knowledge bases\n{}\n\n\
         ### Rendered system prompt\n{rendered_system_prompt}",
        if extension_names.is_empty() {
            "(parent defaults)".to_string()
        } else {
            extension_names.join(", ")
        },
        if skill_names.is_empty() {
            "(none)".to_string()
        } else {
            skill_names.join(", ")
        },
        if knowledge_bases.is_empty() {
            "(none)".to_string()
        } else {
            knowledge_bases.join(", ")
        },
    );
    let mut record = Message::user().with_text(body);
    record.metadata.user_visible = true;
    record.metadata.agent_visible = false;
    // DELIBERATELY NOT `.pinned()`, and this is the product decision the
    // 2026-07-28 amendment owes the reader (Task 14 pins its `note`; this record
    // does not, and the difference is not an oversight):
    //
    // `pin_is_eligible` (`context_mgmt::pins`) requires the message to be
    // AGENT-VISIBLE, and this one is `agent_visible: false` by design — it is a
    // transcript header for the human and the tab, not context for the child's
    // model, which already received all of it as its rendered system prompt. A
    // pin here would be inert: silently unhonoured, and misleading to the next
    // reader who assumes it does something.
    //
    // The child's own copy of this content therefore cannot be lost to
    // compaction, because it is not in the child's context to begin with.
    //
    // What keeps the stored ROW alive across a whole-history rewrite is NOT
    // #51's foreign-tail carry-over, and the earlier draft of this comment said
    // it was. That guard only covers rows ABOVE `basis.max_rowid` that `known`
    // does not name (see `RewriteBasis`, agent.rs); this record is written
    // before the child's first turn, so it is inside `known` and below the
    // watermark — the DELETE half of `replace_conversation_preserving_tail`
    // covers it, and it survives only because it is in the `replacement`.
    //
    // It is in the replacement because every compaction path RE-EMITS every
    // original message and only flips `agent_visible`: both branches of
    // `compact_messages_with_window` push `msg.clone()` for each input, and the
    // bottom rung of the recovery ladder, `drop_oldest_agent_visible_turns`,
    // `map`s rather than filters. No path deletes a row. Anyone changing
    // compaction to PRUNE instead of hide must give this record an explicit
    // carve-out — the store will not save it.
    //
    // If this record is ever made agent-visible, revisit the pin decision too —
    // at that point it becomes exactly the "one message a child must never
    // lose" case and should be pinned.
    record.metadata.provenance = Some(MessageProvenance {
        kind: ProvenanceKind::SpawnContext,
        from_session_id: Some(parent_session_id.to_string()),
        from_session_name: None,
    });
    session_manager
        .add_message_adopting_uid(child_session_id, &mut record)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn get_agent_messages(
    config: AgentConfig,
    workflow: Workflow,
    task_config: TaskConfig,
    session_id: String,
    lease_turn_id: Option<String>,
    cancellation_token: Option<CancellationToken>,
) -> AgentMessagesFuture {
    Box::pin(async move {
        // BR-71 §4.2 glass-box: open the child's bus bracket HERE, at the top of
        // the run — not down at the reply stream — because the caller has
        // already taken the server's turn lease, so from this instant the daemon
        // answers `is_turn_active(child) == true`. Everything below can still
        // fail: `update_provider` and the system-prompt render both `?` out, and
        // a run that took one of those exits used to publish neither a start nor
        // a terminal, leaving an observer watching a session the daemon called
        // busy and then silently wasn't. The guard's `Drop` closes the bracket on
        // those paths (and on a panic, and on the future being dropped).
        //
        // Turn id: the server lease's id when Task 33 acquired one (so observers
        // correlate with /agent/cancel's turn_id); a stable synthetic id headless.
        let bus_bracket = BusTurnBracket::open(
            session_id.clone(),
            lease_turn_id.unwrap_or_else(|| format!("subagent-{session_id}")),
            cancellation_token.clone(),
        );

        let system_instructions = workflow.instructions.clone().unwrap_or_default();
        let user_task = workflow
            .prompt
            .clone()
            .unwrap_or_else(|| "Begin.".to_string());

        // Prep binding 4: `config` is moved into `Agent::with_config` on the
        // next line, so the spawn-context record's session handle is taken now.
        let session_manager = config.session_manager.clone();
        let agent = Arc::new(Agent::with_config(config));

        // BR-71: make the live child addressable by the server control plane.
        // Best-effort — AgentManager::instance() needs global config; unit
        // tests and bare-library embedding run fine without it.
        let registration = match crate::execution::manager::AgentManager::instance().await {
            Ok(manager) => {
                manager
                    .register_agent(session_id.clone(), agent.clone())
                    .await;
                Some((manager, agent.clone()))
            }
            Err(e) => {
                tracing::debug!("subagent not registered in AgentManager: {e}");
                None
            }
        };
        // Deregister on every exit path (scopeguard-free: a small Drop struct).
        let _deregister = Deregister {
            manager: registration,
            session_id: session_id.clone(),
        };

        let parent_working_dir = task_config.parent_working_dir.clone();

        // SubagentStart hook (observe-only). The child agent fires its own
        // tool/stop hooks while it runs.
        {
            let hooks = agent.hooks_manager();
            let mut payload = crate::hooks::HookPayload::new(
                crate::hooks::HookEvent::SubagentStart,
                &task_config.parent_session_id,
                parent_working_dir.to_string_lossy(),
            );
            payload.subagent_id = Some(session_id.clone());
            payload.message = Some(system_instructions.chars().take(500).collect());
            hooks.fire(
                crate::hooks::HookEvent::SubagentStart,
                None,
                payload,
                parent_working_dir.clone(),
            );
        }

        let uses_coding_agent_bridge = task_config.provider.uses_tool_bridge();
        agent
            .update_provider(task_config.provider, &session_id)
            .await
            .map_err(|e| anyhow!("Failed to set provider on sub agent: {}", e))?;

        // §5: the child never gets the workspace extension, so neither the
        // loaded set nor anything derived from it may name it — otherwise the
        // spawn record tells the user the child holds workspace control it does
        // not. The strip runs ONCE, inside the helper, and everything
        // downstream reads its result: a second copy of
        // `!= WORKSPACE_EXTENSION_NAME` is one edit away from disagreeing with
        // the grant it claims to describe, and the disagreement is silent,
        // because the record is prose the user reads rather than a value
        // anything checks.
        //
        // ⚠ **`loaded` is what ACTUALLY loaded, not what was asked for** — see
        // [`load_granted_extensions`]. Both consumers below are claims *to the
        // user* about what this child holds, so both are built from the
        // outcome.
        let loaded = load_granted_extensions(&agent, task_config.extensions).await;

        // Consumed by `persist_spawn_context` further down; bound here because
        // the record is written after several other preparation steps.
        let extension_names: Vec<String> = loaded.iter().map(|e| e.name().to_string()).collect();

        let has_response_schema = workflow.response.is_some();
        agent
            .apply_workflow_components(
                workflow.sub_workflows.clone(),
                workflow.response.clone(),
                true,
            )
            .await;

        // Prep binding 2: the prompt context below moves `system_instructions`
        // into `task_instructions`.
        let task_instructions_for_record = system_instructions.clone();

        let mut tools = agent.list_tools(&session_id, None).await;
        if uses_coding_agent_bridge {
            let knowledge_target =
                crate::agents::extension_manager::resolve_bundled_extension("knowledge");
            let trusted_knowledge = knowledge_target.as_ref().is_some_and(|target| {
                loaded
                    .iter()
                    .any(|extension| target.matches_config(extension))
            });
            let skills_target =
                crate::agents::extension_manager::resolve_bundled_extension("skills");
            let trusted_skills = skills_target.as_ref().is_some_and(|target| {
                loaded
                    .iter()
                    .any(|extension| target.matches_config(extension))
            });
            let extension_manager_target =
                crate::agents::extension_manager::resolve_bundled_extension("Extension Manager");
            let trusted_extension_manager =
                extension_manager_target.as_ref().is_some_and(|target| {
                    loaded
                        .iter()
                        .any(|extension| target.matches_config(extension))
                });
            tools.retain(|tool| {
                crate::agents::agent::coding_agent_bridge_allows_tool(
                    tool.name.as_ref(),
                    false,
                    trusted_knowledge,
                    trusted_skills,
                    trusted_extension_manager,
                )
            });
        }
        let subagent_prompt = render_global_file(
            "subagent_system.md",
            &SubagentPromptContext {
                max_turns: task_config
                    .max_turns
                    .expect("TaskConfig always sets max_turns"),
                subagent_id: session_id.clone(),
                task_instructions: system_instructions,
                tool_count: tools.len(),
                available_tools: tools
                    .iter()
                    .map(|t| t.name.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            },
        )
        .map_err(|e| anyhow!("Failed to render subagent system prompt: {}", e))?;
        // Prep binding 3: `override_system_prompt` takes the template BY VALUE.
        let rendered_prompt = subagent_prompt.clone();
        agent.override_system_prompt(subagent_prompt).await;

        // The visible spawn-context record below is intentionally model-hidden
        // prose. Cold continuation must never parse that prose to reconstruct
        // authority or behavior. The profile is also the child-tab projection:
        // extension references and their exact tool clamp therefore commit in
        // one value, with no credential-bearing ExtensionConfig beside it.
        let runtime_tool_names: Vec<String> =
            tools.iter().map(|tool| tool.name.to_string()).collect();
        let runtime_profile = crate::agents::subagent_runtime_profile::SubagentRuntimeProfile::new(
            rendered_prompt.clone(),
            workflow.response.clone(),
            workflow.sub_workflows.clone().unwrap_or_default(),
            &loaded,
            &runtime_tool_names,
        )?;
        runtime_profile
            .persist(&session_manager, &session_id)
            .await
            .context("Failed to persist subagent runtime profile")?;
        agent.mark_subagent_runtime_installed(&session_id).await;

        // BR-71 §4.4: record the child's spawn context as its first message,
        // before the reply stream starts. Grants for the record: extensions from
        // the task config; skills from the workflow; the child's knowledge bases
        // via the daemon services when installed (empty headless, where `get()`
        // returns `None`).
        //
        // This is NOT usually empty when the daemon is installed, contrary to an
        // earlier draft of this comment. `knowledge_selection` resolves to
        // `KnowledgeService::selection`, whose visible set is *every installed
        // base minus the hidden ones* (`selection_unlocked`), and a brand-new
        // child session has no `.hidden-kb-sessions/<digest>` file, so
        // `get_hidden_for_session_or_persisted` falls back to the machine-wide
        // hidden list. A child therefore inherits the machine's whole visible
        // set on its first read. That is the truth the record should carry — but
        // do not restate the old "a subagent inherits no KB" claim; it is wrong.
        //
        // The record names only the KB *set* — the primary is per-session mutable
        // state, not a grant, and recording a value that can change five minutes
        // later as part of an immutable spawn record is how a "source of truth"
        // starts lying.
        //
        // The read is dispatched to a blocking thread: the daemon implementation
        // takes `KnowledgeService`'s root `flock` and scans directories, so a
        // concurrent KB ingest macro holding that lock would otherwise park a
        // tokio worker for the length of the ingest, on every subagent spawn.
        let skill_names: Vec<String> = workflow.skills.clone().unwrap_or_default();
        let knowledge_bases = match crate::workspace_services::get() {
            Some(services) => {
                let kb_session_id = session_id.clone();
                tokio::task::spawn_blocking(move || {
                    services.knowledge_selection(&kb_session_id).kb_ids
                })
                .await
                .unwrap_or_default()
            }
            None => Vec::new(),
        };
        if let Err(e) = persist_spawn_context(
            &session_manager,
            &session_id,
            &task_config.parent_session_id,
            &rendered_prompt,
            &task_instructions_for_record,
            &extension_names,
            &skill_names,
            &knowledge_bases,
        )
        .await
        {
            // Best-effort, and the asymmetry with `create_subagent_session`'s
            // `?` on the SAME stamp is deliberate, not an accident of which
            // call site got a `?`:
            //
            // At birth nothing has been spent — no provider configured, no
            // extension loaded, no billed call — and `create_session` on that
            // same store failing already aborts the spawn, so a targeted UPDATE
            // failing one statement later means the store is gone and there is
            // nothing to salvage. Failing there costs an error message and
            // saves a permanently unparented row that no later path retries.
            //
            // Here the calculus is inverted: the provider and every extension
            // are already configured, the reply stream is one line away, and
            // the parent stamp is ALREADY durable from birth — so all that is
            // at risk is the transcript header. Killing a configured run to
            // save a header would be the worse trade.
            tracing::warn!("failed to persist subagent spawn context: {e}");
        }

        // Handoff without an admission gap: queued child input remains accepted
        // until the live agent interrupt queue is open. The reply loop reuses
        // this exact prepared turn on its first poll rather than clearing it.
        agent.prepare_soft_interrupt_turn();
        let pending_user_inputs =
            crate::agents::subagent_handle::mark_initial_runtime_ready(&session_id);
        let user_message = delegated_initial_message(user_task, pending_user_inputs);
        let initial_user_direct = user_message
            .metadata
            .provenance
            .as_ref()
            .is_some_and(|provenance| {
                provenance.kind == crate::conversation::message::ProvenanceKind::UserDirect
            })
            .then(|| user_message.clone());
        // `reply` consumes the message, but the id of the row it persists is only
        // known once it returns, and the result transcript has to carry that id
        // rather than the pre-persistence one. Keep the copy that rewrite needs.
        let mut transcript_message = user_message.clone();
        let mut conversation = Conversation::new_unvalidated(vec![transcript_message.clone()]);

        if let Some(activities) = workflow.activities {
            for activity in activities {
                info!("Workflow activity: {}", activity);
            }
        }
        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: task_config.max_turns.map(|v| v as u32),
            max_tool_calls: None,
            budget: None,
            retry_config: workflow.retry,
            // Subagents run at the model's default depth; a parent turn's effort
            // is not inherited (its exploration caps are the parent's, not this
            // task's). BR-63.
            reasoning_effort: None,
        };

        let mut aborted: Option<TurnAbort> = None;
        // `?` is safe again here: `bus_bracket`'s `Drop` publishes the terminal
        // for this exit (and picks `cancelled` over `error` when the run's token
        // was tripped, which a hand-written publish on this one path could not
        // do without duplicating the ladder).
        let mut stream =
            match crate::session_context::with_session_id(Some(session_id.clone()), async {
                agent
                    .reply(user_message, session_config, cancellation_token)
                    .await
            })
            .await
            {
                Ok(stream) => {
                    // `?` here still means a failed WRITE, which is a genuine
                    // store fault worth ending the run on. A failed READ no
                    // longer reaches it — it arrives as `Unverified`.
                    let durability = ensure_initial_user_direct_is_durable(
                        &session_manager,
                        &session_id,
                        initial_user_direct,
                    )
                    .await?;
                    match &durability {
                        InitialInputDurability::AlreadyDurable(durable) => {
                            transcript_message.id = durable.id.clone();
                            conversation =
                                Conversation::new_unvalidated(vec![transcript_message.clone()]);
                        }
                        InitialInputDurability::Inserted(durable) => {
                            transcript_message.id = durable.id.clone();
                            conversation =
                                Conversation::new_unvalidated(vec![transcript_message.clone()]);
                            crate::session_events::publish(
                                &session_id,
                                crate::session_events::SessionBusEvent::Agent(AgentEvent::Message(
                                    durable.clone(),
                                )),
                            );
                        }
                        InitialInputDurability::NotApplicable => {}
                        InitialInputDurability::Unverified(read_error) => {
                            tracing::warn!(
                                session_id,
                                "could not verify that accepted pre-start steering is durable; \
                                 leaving it recoverable at completion: {read_error}"
                            );
                        }
                    }
                    if durability.verified() {
                        crate::agents::subagent_handle::settle_initial_runtime_inputs(&session_id);
                    }
                    stream
                }
                Err(error) => {
                    let durable_initial = ensure_initial_user_direct_is_durable(
                        &session_manager,
                        &session_id,
                        initial_user_direct,
                    )
                    .await;
                    let mut settlement_errors = Vec::new();
                    match &durable_initial {
                        // Published even when it was already durable: `reply`
                        // failed, so it never published its own copy.
                        Ok(
                            InitialInputDurability::AlreadyDurable(message)
                            | InitialInputDurability::Inserted(message),
                        ) => {
                            crate::session_events::publish(
                                &session_id,
                                crate::session_events::SessionBusEvent::Agent(AgentEvent::Message(
                                    message.clone(),
                                )),
                            );
                        }
                        Ok(InitialInputDurability::NotApplicable) => {}
                        Ok(InitialInputDurability::Unverified(read_error)) => settlement_errors
                            .push(format!(
                                "accepted pre-start steering could not be confirmed durable: \
                                 {read_error}"
                            )),
                        Err(settlement_error) => settlement_errors.push(format!(
                            "accepted pre-start steering could not be persisted: {settlement_error}"
                        )),
                    }
                    if durable_initial
                        .as_ref()
                        .is_ok_and(InitialInputDurability::verified)
                    {
                        crate::agents::subagent_handle::settle_initial_runtime_inputs(&session_id);
                    }
                    let carried = agent.settle_carried_over_soft_interrupts(&session_id).await;
                    match carried {
                        Ok(carried) => {
                            for message in carried {
                                crate::session_events::publish(
                                    &session_id,
                                    crate::session_events::SessionBusEvent::Agent(
                                        AgentEvent::Message(message),
                                    ),
                                );
                            }
                        }
                        Err(settlement_error) => settlement_errors.push(format!(
                            "accepted live steering could not be persisted: {settlement_error}"
                        )),
                    }
                    if !settlement_errors.is_empty() {
                        return Err(anyhow!(
                            "Failed to get reply from agent: {error}; {}",
                            settlement_errors.join("; ")
                        ));
                    }
                    return Err(anyhow!("Failed to get reply from agent: {error}"));
                }
            };
        while let Some(message_result) = stream.next().await {
            match message_result {
                Ok(event) => {
                    // BR-71: glass-box — every child event is observable.
                    //
                    // TOTAL AND IN STREAM ORDER. This tee publishes EVERY
                    // variant, before the `match` below decides what the parent
                    // does with it, and it never reorders, filters or coalesces.
                    // Both halves are load-bearing:
                    //   * total, because an observer tab is a full client and
                    //     must receive `MessagesPersisted` (#59) even though the
                    //     parent's accumulation ignores it;
                    //   * in order, because the #59 invariant — no
                    //     `MessagesPersisted` may precede a `Message` frame
                    //     carrying one of the ids it publishes — is a property of
                    //     the PRODUCER's stream, and it survives only if every
                    //     relay preserves order. Publishing here, once, before
                    //     any per-variant handling, is what makes that free.
                    crate::session_events::publish(
                        &session_id,
                        crate::session_events::SessionBusEvent::Agent(event.clone()),
                    );
                    // NINE arms, no wildcard: a `_ => {}` here would silently
                    // swallow a tenth `AgentEvent` variant instead of failing
                    // the build.
                    match event {
                        AgentEvent::Message(msg) => conversation.push(msg),
                        AgentEvent::McpNotification(_)
                        | AgentEvent::ModelChange { .. }
                        | AgentEvent::ToolCallPending(_)
                        // Issue #56 Gate B, addressed to a HUMAN reading a
                        // composer. A subagent has no composer and its parent is
                        // not the user, so the parent accumulates nothing from
                        // it; the tee above still publishes it for an observer
                        // tab watching the child.
                        | AgentEvent::PrivacyProviderPinned { .. }
                        // #59: the subagent's own rows are already carried by
                        // the `Message` events above (which now name
                        // themselves); the parent has no `expectedMessageIds`
                        // to satisfy. The TEE above still publishes this — an
                        // observer tab DOES need it. Do not drop it.
                        | AgentEvent::MessagesPersisted(_)
                        | AgentEvent::TokenUsage(_) => {}
                        AgentEvent::HistoryReplaced(updated_conversation) => {
                            conversation = updated_conversation;
                        }
                        AgentEvent::TurnAborted { code, message } => {
                            // The subagent's turn failed. Its assistant Message
                            // (the human-readable "Ran into this error: …") is
                            // already in the conversation, so the parent still
                            // sees *what* happened — but as prose
                            // indistinguishable from a real summary. Carry the
                            // abort out so the envelope can say `error`.
                            tracing::error!(abort = code.wire_code(), "Subagent turn aborted: {message}");
                            aborted = Some((code.wire_code().to_string(), message));
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Error receiving message from subagent: {}", e);
                    aborted = Some(("stream_error".to_string(), e.to_string()));
                    break;
                }
            }
        }
        // Normal reply-loop completion already settles this exact queue. This
        // idempotent second settlement covers pre-stream immediate replies that
        // never entered `reply_internal` after the delegated handoff accepted a
        // steer. A 202 user message therefore becomes durable even when the
        // child exits before provider work begins.
        for message in agent
            .settle_carried_over_soft_interrupts(&session_id)
            .await?
        {
            crate::session_events::publish(
                &session_id,
                crate::session_events::SessionBusEvent::Agent(AgentEvent::Message(message.clone())),
            );
            conversation.push(message);
        }
        // The run reached the end of its stream, so it — not the guard — names
        // the reason. `bus_bracket` holds a clone of the run's token, taken
        // before the token moved into `agent.reply(..)` above, so a cancelled
        // run still reports `cancelled` rather than `error`.
        //
        // Wire note (deliberate, not an oversight): when the turn aborted, an
        // observer sees TWO frames it could read as terminal — the teed
        // `Agent(TurnAborted)`, which `routes::session_events::map_bus_event`
        // renders as `MessageEvent::Error` with the classified provider
        // envelope, and then this `TurnFinished { reason: "error" }`, which it
        // renders as `Finish`. That is the shape this crate can produce: the
        // classifier lives in `biorouter-server`, so the child cannot publish a
        // `SessionBusEvent::TurnError` with a faithful envelope itself, and the
        // mapper's own comment says its `TurnAborted` arm exists precisely for
        // publishers that tee raw agent events. The bracket must still close, so
        // the `Finish` stays — a consumer takes the first terminal it sees.
        let reason = if bus_bracket.run_cancelled() {
            "cancelled"
        } else if aborted.is_some() {
            "error"
        } else {
            "stop"
        };
        bus_bracket.close(reason);

        // BR-28: the subagent is done — join its SubagentStart hook rather than
        // leaving the detached task to outlive the subagent and race shutdown.
        // The aggregate is keyed by the *parent* session (that is the payload's
        // session_id), which the child's own turn boundaries never drain, so
        // this is its only settle point. A subagent's stream is observable — the
        // tee above publishes it to the session bus (BR-71) — but it is still not
        // part of the parent's `/reply` stream, so a `systemMessage` has nowhere
        // to surface but the log; errors are already warned by `dispatch`.
        for outcome in agent
            .hooks_manager()
            .settle_fired(
                &task_config.parent_session_id,
                crate::hooks::FIRE_JOIN_BUDGET_SHUTDOWN,
            )
            .await
        {
            for message in &outcome.aggregate.system_messages {
                info!("hooks: {} systemMessage: {}", outcome.event, message);
            }
        }

        let final_output = if has_response_schema {
            agent
                .final_output_tool
                .lock()
                .await
                .as_ref()
                .and_then(|tool| tool.final_output.clone())
        } else {
            None
        };

        Ok((conversation, final_output, aborted))
    })
}

#[cfg(test)]
mod tests {

    /// Await a subagent run with a CEILING.
    ///
    /// ⚠ Every one of these awaits was unbounded, and that is what turned an
    /// intermittent deadlock *inside* the run into a CANCELLED CI job rather
    /// than a test failure. The stuck run keeps its `serial_test` lock, every
    /// other test in the `workspace_services` group piles up behind it, and the
    /// runner eventually kills the job -- discarding the ~3000 results that had
    /// already passed and naming no test at all. The macOS log said only that
    /// two tests "have been running for over 60 seconds", and one of those two
    /// was merely queued behind the other.
    ///
    /// This does NOT fix the deadlock. It makes it legible and cheap: the one
    /// stuck run fails by name inside two minutes, releases its lock, and the
    /// rest of the suite reports normally. A run that is merely slow is well
    /// inside the ceiling -- these tests replay a cassette and finish in
    /// milliseconds.
    macro_rules! run_subagent_bounded {
        ($($arg:tt)*) => {
            match tokio::time::timeout(
                std::time::Duration::from_secs(120),
                run_complete_subagent_task($($arg)*),
            )
            .await
            {
                Ok(value) => value,
                Err(_) => panic!(
                    "run_complete_subagent_task did not finish within 120s. This is the \
                     known intermittent subagent deadlock; it is reported here instead of \
                     hanging, because an unbounded await starves every other test sharing \
                     the `workspace_services` lock and gets the whole job cancelled."
                ),
            }
        };
    }
    use super::*;
    use crate::conversation::message::ProvenanceKind;
    use crate::session::session_manager::SessionType;
    use crate::workspace_services::{WorkspaceServices, WorkspaceTurnLease};

    #[tokio::test]
    async fn accepted_initial_user_direct_message_is_durable_and_idempotent() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let session = manager
            .create_session(
                temp.path().to_path_buf(),
                "initial handoff".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let message = Message::user()
            .with_id("initial-user-direct")
            .with_text("change the delegated task")
            .with_provenance(crate::conversation::message::MessageProvenance {
                kind: ProvenanceKind::UserDirect,
                from_session_id: None,
                from_session_name: None,
            });

        let durability =
            ensure_initial_user_direct_is_durable(&manager, &session.id, Some(message.clone()))
                .await
                .unwrap();
        let InitialInputDurability::Inserted(stored) = &durability else {
            panic!("the first call writes the accepted message");
        };
        assert!(durability.verified());
        assert_eq!(stored.id.as_deref(), Some("initial-user-direct"));
        assert!(stored.metadata.user_visible);
        assert!(!stored.metadata.agent_visible);

        let again = ensure_initial_user_direct_is_durable(&manager, &session.id, Some(message))
            .await
            .unwrap();
        assert!(
            matches!(again, InitialInputDurability::AlreadyDurable(_)),
            "a second call must recognise the row rather than writing a second one"
        );
        assert!(again.verified());
        let conversation = manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(
            conversation
                .messages()
                .iter()
                .filter(|message| message.id.as_deref() == Some("initial-user-direct"))
                .count(),
            1
        );
    }

    /// A session store every query fails against: the sqlite file is present but
    /// is not a database, so the lazy pool's first use errors instead of opening.
    /// The closest a test can get to the transient store failure (SQLITE_BUSY
    /// under two daemons) the read path has to survive.
    fn unreadable_session_manager(temp: &tempfile::TempDir) -> SessionManager {
        let sessions = temp
            .path()
            .join(crate::session::session_manager::SESSIONS_FOLDER);
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join(crate::session::session_manager::DB_NAME),
            b"this is not a sqlite database",
        )
        .unwrap();
        SessionManager::new(temp.path().to_path_buf())
    }

    fn user_direct(text: &str) -> Message {
        Message::user().with_text(text).with_provenance(
            crate::conversation::message::MessageProvenance {
                kind: ProvenanceKind::UserDirect,
                from_session_id: None,
                from_session_name: None,
            },
        )
    }

    /// OD-1. The idempotency probe used to be skipped entirely when the accepted
    /// message carried no id, so the insert always ran — on top of the row
    /// `Agent::reply` had already written under a uid it minted itself. Two rows
    /// for one steer, and `add_message`'s own replay detector cannot see it
    /// because an id-less message gets a fresh uuid every time.
    ///
    /// Production-reachable: `biorouter session send` posts `"id": null`.
    #[tokio::test]
    async fn an_id_less_accepted_message_is_stored_exactly_once() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let session = manager
            .create_session(
                temp.path().to_path_buf(),
                "id-less handoff".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();

        // Exactly what `reply` does with the combined prompt on its ordinary
        // path: persist it, minting a uid the caller could not have predicted.
        let mut persisted_by_reply = user_direct("steer the delegated run");
        assert!(persisted_by_reply.id.is_none());
        manager
            .add_message_adopting_uid(&session.id, &mut persisted_by_reply)
            .await
            .unwrap();
        assert!(persisted_by_reply.id.is_some(), "reply minted a uid");

        let durability = ensure_initial_user_direct_is_durable(
            &manager,
            &session.id,
            Some(user_direct("steer the delegated run")),
        )
        .await
        .unwrap();
        let InitialInputDurability::AlreadyDurable(adopted) = &durability else {
            panic!("the id-less message must be recognised in the transcript");
        };
        assert_eq!(
            adopted.id, persisted_by_reply.id,
            "the probe adopts the id of the row that already holds this turn"
        );

        let conversation = manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(
            conversation
                .messages()
                .iter()
                .filter(|message| message.as_concat_text() == "steer the delegated run")
                .count(),
            1,
            "one accepted steer, one row"
        );
    }

    /// OD-3. A transient read failure (the two-daemon SQLITE_BUSY case) must not
    /// come back as `Err`: the call site's `?` sits inside the arm that already
    /// holds a live reply stream, so an `Err` there drops that stream and fails
    /// a delegated run whose prompt is very probably already durable.
    #[tokio::test]
    async fn a_failed_durability_read_is_reported_instead_of_ending_the_run() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = unreadable_session_manager(&temp);

        let durability = ensure_initial_user_direct_is_durable(
            &manager,
            "no-such-session",
            Some(user_direct("keep the stream alive")),
        )
        .await
        .expect("a read failure is not an error out of the run");
        assert!(matches!(durability, InitialInputDurability::Unverified(_)));
        assert!(
            !durability.verified(),
            "nothing was verified, so nothing may be settled"
        );
    }

    /// OD-2. The settle invariant, in the one place both call sites read it.
    /// It used to be applied on the error branch only, so the success branch
    /// acknowledged the initialization queue after a call that had established
    /// nothing — retiring the recovery copy of a message that, on `reply`'s two
    /// early returns (a privacy refusal, an `execute_command` error), exists in
    /// no transcript at all.
    #[test]
    fn only_a_verified_durability_may_settle_the_initialization_queue() {
        assert!(!InitialInputDurability::NotApplicable.verified());
        assert!(!InitialInputDurability::Unverified(anyhow!("busy")).verified());
        assert!(InitialInputDurability::AlreadyDurable(Message::user().with_text("a")).verified());
        assert!(InitialInputDurability::Inserted(Message::user().with_text("a")).verified());
    }

    /// The privacy-refusal shape, which is the scenario this whole path exists
    /// for and was untested: `reply` yields a refusal stream WITHOUT persisting
    /// the prompt, so the accepted steering reaches an empty transcript. It must
    /// still land, user-visible and agent-INVISIBLE — the refusal is precisely
    /// the case where the text must not enter the model's context on a later
    /// continuation — and the queue may then be settled.
    #[tokio::test]
    async fn accepted_steering_survives_a_reply_that_persisted_nothing() {
        let temp = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let session = manager
            .create_session(
                temp.path().to_path_buf(),
                "refused turn".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();

        let durability = ensure_initial_user_direct_is_durable(
            &manager,
            &session.id,
            Some(user_direct("the steer the refused turn never carried")),
        )
        .await
        .unwrap();
        let InitialInputDurability::Inserted(stored) = &durability else {
            panic!("an empty transcript means the accepted steering must be written here");
        };
        assert!(stored.metadata.user_visible);
        assert!(!stored.metadata.agent_visible);
        assert!(durability.verified());

        let conversation = manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let stored = conversation
            .messages()
            .iter()
            .find(|message| message.as_concat_text() == "the steer the refused turn never carried")
            .expect("the accepted steering is durable");
        assert_eq!(
            stored.metadata.provenance.as_ref().map(|value| value.kind),
            Some(ProvenanceKind::UserDirect),
            "provenance is what makes the result report human intervention"
        );
        assert!(crate::agents::subagent_result::conversation_has_user_direct(&conversation));
    }

    /// Audit OD-6, normal path. Several pre-start steers collapse into ONE
    /// delegated message — pinned here because the recovery path in
    /// `subagent_tool` persists them SEPARATELY, and that divergence must be
    /// visible rather than drifting silently.
    #[test]
    fn several_pre_start_steers_collapse_into_one_delegated_message() {
        let first = user_direct("first steer").with_id("steer-1");
        let second = user_direct("second steer").with_id("steer-2");

        let combined =
            delegated_initial_message("do the delegated work".into(), vec![first, second]);

        assert_eq!(combined.role, Role::User);
        assert_eq!(
            combined.id.as_deref(),
            Some("steer-1"),
            "the combined message inherits the FIRST accepted steer's identity"
        );
        assert_eq!(
            combined
                .metadata
                .provenance
                .as_ref()
                .map(|value| value.kind),
            Some(ProvenanceKind::UserDirect),
            "provenance must survive the collapse or the result stops reporting \
             human intervention"
        );
        let text = combined.as_concat_text();
        assert!(text.contains("do the delegated work"));
        assert!(text.contains("first steer"));
        assert!(text.contains("second steer"));
        assert!(text.contains("Additional pre-start user steering"));
    }

    /// With nothing queued the delegated message is just the task, and carries
    /// no provenance — which is what makes `initial_user_direct` `None` and, per
    /// the test above, blocks the settle.
    #[test]
    fn a_delegated_message_with_no_steering_is_just_the_task() {
        let plain = delegated_initial_message("do the delegated work".into(), vec![]);
        assert_eq!(plain.as_concat_text(), "do the delegated work");
        assert!(plain.metadata.provenance.is_none());
    }

    #[test]
    fn workspace_extension_is_stripped_from_child_grants() {
        let configs = vec![
            crate::agents::extension::ExtensionConfig::Platform {
                name: "workspace".into(),
                description: String::new(),
                bundled: None,
                available_tools: vec![],
            },
            crate::agents::extension::ExtensionConfig::Platform {
                name: "todo".into(),
                description: String::new(),
                bundled: None,
                available_tools: vec![],
            },
        ];
        let granted = strip_workspace_extension(configs);
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0].name(), "todo");
    }

    /// The child's recorded grant is **what loaded**, not what was asked for.
    ///
    /// `GET /sessions/{id}/extensions` serves the runtime profile projection as
    /// authoritative, so a set written from the request makes the subagent tab
    /// header claim an extension the child does not have — an over-claim, and
    /// the one direction that matters, because the user reads that header to
    /// decide what the child can do.
    ///
    /// The failing member is an unknown **platform** name: `add_extension`
    /// rejects it with `Unknown platform extension` before any process is
    /// spawned, so the failure is deterministic and costs nothing.
    /// ⚠ **The daemon spells it `Workspace`, the registry spells it `workspace`.**
    ///
    /// Found by a live stress pass, not by a test: a real child was advertised
    /// seven workspace tools because this filter compared with `!=` against the
    /// name that actually appears in an inherited config list. Dispatch still
    /// refused them, so the boundary held, but the child was invited to call
    /// tools that always fail and the tab header claimed a grant it did not
    /// hold.
    #[test]
    fn the_workspace_strip_ignores_case_because_both_spellings_are_real() {
        use crate::agents::extension::ExtensionConfig;
        let entry = |name: &str| ExtensionConfig::Platform {
            name: name.to_string(),
            description: String::new(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };

        for spelling in ["workspace", "Workspace", "WORKSPACE"] {
            let kept = strip_workspace_extension(vec![entry(spelling), entry("todo")]);
            assert_eq!(
                kept.len(),
                1,
                "{spelling} should have been stripped, got {:?}",
                kept.iter()
                    .map(|e| e.name().to_string())
                    .collect::<Vec<_>>()
            );
            assert_eq!(kept[0].name(), "todo");
        }

        // And it must not strip something that merely contains the word.
        let kept = strip_workspace_extension(vec![entry("workspace-notes")]);
        assert_eq!(kept.len(), 1, "only the exact name is the workspace grant");
    }

    #[tokio::test]
    async fn the_recorded_grant_is_what_loaded_not_what_was_requested() {
        let platform = |name: &str| crate::agents::extension::ExtensionConfig::Platform {
            name: name.into(),
            description: String::new(),
            bundled: None,
            available_tools: vec![],
        };
        let agent = Agent::new();

        let loaded = load_granted_extensions(
            &agent,
            vec![
                // Never granted to a child at all (§5).
                platform("workspace"),
                // Loads.
                platform("todo"),
                // Fails to load: the child does NOT hold this one.
                platform("no-such-platform-extension"),
            ],
        )
        .await;

        assert_eq!(
            loaded
                .iter()
                .map(|e| e.name().to_string())
                .collect::<Vec<_>>(),
            vec!["todo".to_string()],
            "a requested-but-unloadable extension must not be recorded as granted"
        );
    }

    /// The body of one `### `-delimited section of the spawn record, so a grant
    /// can be asserted to be in the RIGHT section. Six bare `contains` checks
    /// pass just as happily on a record that renders the skills under
    /// "Granted extensions" and the extensions under "Granted skills".
    fn section<'a>(body: &'a str, heading: &str) -> &'a str {
        let (_, rest) = body
            .split_once(heading)
            .unwrap_or_else(|| panic!("spawn record has no {heading} section:\n{body}"));
        rest.split("\n### ").next().unwrap_or(rest)
    }

    #[tokio::test]
    async fn spawn_context_is_persisted_visible_to_user_not_agent() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = sm
            .create_session(
                temp.path().to_path_buf(),
                "Subagent task".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();

        persist_spawn_context(
            &sm,
            &child.id,
            "parent-1",
            "SYSTEM PROMPT RENDERED HERE",
            "task: count the files",
            &["developer".to_string()],
            &["single-cell".to_string()],
            &["kb-papers".to_string(), "kb-methods".to_string()],
        )
        .await
        .unwrap();

        let reread = sm.get_session(&child.id, true).await.unwrap();
        assert_eq!(reread.parent_session_id.as_deref(), Some("parent-1"));
        let msgs = reread.conversation.unwrap().messages().to_vec();
        // Exactly one row: the record is written once per spawn. Without this a
        // double-write would still leave a correct-looking first message.
        assert_eq!(
            msgs.len(),
            1,
            "one spawn call must write exactly one record, got {msgs:#?}"
        );
        let record = msgs.first().expect("spawn context is the first message");
        // `MessageMetadata::default()` is already `user_visible: true`, so this
        // assertion documents the requirement rather than discriminating; the
        // discriminating half of the pair is the `agent_visible` one below,
        // whose default is `true`.
        assert!(record.metadata.user_visible);
        assert!(
            !record.metadata.agent_visible,
            "must not enter the child's model context"
        );
        assert_eq!(
            record.metadata.provenance.as_ref().unwrap().kind,
            ProvenanceKind::SpawnContext
        );
        let text: String = record.content.iter().filter_map(|c| c.as_text()).collect();
        assert!(text.contains("SYSTEM PROMPT RENDERED HERE"));
        assert!(text.contains("count the files"));
        assert!(text.contains("developer"));
        // §4.5/issue: the record carries ALL grants — extensions, skills, KB.
        assert!(text.contains("single-cell"));
        assert!(text.contains("kb-papers"));
        // Issue #45: the record shows EVERY active base, not just the first.
        assert!(text.contains("kb-methods"));

        // …and each grant is under its OWN heading. The `contains` checks above
        // are satisfied by a record that files every grant in the wrong section.
        assert_eq!(
            section(&text, "### Task instructions").trim(),
            "task: count the files"
        );
        assert_eq!(section(&text, "### Granted extensions").trim(), "developer");
        assert_eq!(section(&text, "### Granted skills").trim(), "single-cell");
        assert_eq!(
            section(&text, "### Knowledge bases").trim(),
            "kb-papers, kb-methods"
        );
        assert_eq!(
            section(&text, "### Rendered system prompt").trim(),
            "SYSTEM PROMPT RENDERED HERE"
        );
        // The parent is named in the record body too, not only in `provenance`.
        assert!(text.contains("Spawned by session: parent-1"));
    }

    /// The empty-grant rendering is its own case: a spawn with no extensions
    /// must say "(parent defaults)", not silently render an empty section that
    /// reads as "no extensions were granted".
    #[tokio::test]
    async fn spawn_context_names_the_empty_grants_explicitly() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = sm
            .create_session(
                temp.path().to_path_buf(),
                "Subagent task".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();

        persist_spawn_context(
            &sm,
            &child.id,
            "parent-2",
            "PROMPT",
            "do a thing",
            &[],
            &[],
            &[],
        )
        .await
        .unwrap();

        let reread = sm.get_session(&child.id, true).await.unwrap();
        let msgs = reread.conversation.unwrap().messages().to_vec();
        let text: String = msgs[0].content.iter().filter_map(|c| c.as_text()).collect();
        assert_eq!(
            section(&text, "### Granted extensions").trim(),
            "(parent defaults)"
        );
        assert_eq!(section(&text, "### Granted skills").trim(), "(none)");
        assert_eq!(section(&text, "### Knowledge bases").trim(), "(none)");
    }

    /// Headless (no WorkspaceServices installed): the run must not require the
    /// daemon — no lease, no panic, result envelope still produced (§2.1) — AND
    /// it must still register its child agent with the `AgentManager`, which is
    /// the whole point of Task 33.
    ///
    /// ⚠ The registration half is new (2026-07-28 gate sweep). This test used to
    /// end at `assert!(!rendered.is_empty())`, and `serde_json::to_string` of any
    /// `Serialize` value is non-empty — so it passed with `register_agent`,
    /// `begin_turn` and the `Deregister` guard **all absent**. The `AgentManager`
    /// tests are genuinely good, but they call `register_agent` by hand; nothing
    /// proved `run_complete_subagent_task` does.
    ///
    /// The sentinel below is what makes the registration observable without
    /// racing the `Deregister` drop: we register OUR agent under the child's id
    /// first. A run that registers replaces it and then deregisters its own
    /// entry (`deregister_agent_if_same`), leaving nothing. A run that does not
    /// register leaves the sentinel sitting there forever.
    ///
    /// ⚠ The sentinel proves ABSENCE, and one wrong implementation also produces
    /// absence: a run that never registers but tears down with a blunt
    /// `manager.remove_session(&child.id)` removes the sentinel and passes. The
    /// Step-5 grep pair closes that (`remove_session(` must be **0** in
    /// `subagent_handler.rs`); do not weaken it into "either deregistration is
    /// fine", because identity-scoped removal is the whole point — a plain
    /// remove would also evict a *live* agent registered by someone else.
    /// Serialized against the other test that runs a real subagent — see
    /// `subagent_run_publishes_lifecycle_to_the_bus` for why.
    /// `parallel(workspace_services)`: this run READS the process-global
    /// services slot and asserts the HEADLESS answer, so it must never overlap
    /// the lease tests, which override that slot for every thread in the
    /// binary. Without the key those overrides leak in and this run takes a
    /// spy's lease — which is how the omission was found, not theorised.
    #[tokio::test]
    #[serial_test::parallel(workspace_services)]
    #[serial_test::serial(subagent_session_bus, agent_manager_pin)]
    async fn subagent_run_without_daemon_services_still_completes() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = sm
            .create_session(
                temp.path().to_path_buf(),
                "child".into(),
                crate::session::session_manager::SessionType::SubAgent,
            )
            .await
            .unwrap();
        let config = AgentConfig::new(
            sm.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            crate::config::BioRouterMode::Auto,
        );
        // TestProvider replaying an empty cassette: fails on first use — the
        // run errors fast, which is all this needs — the pattern
        // `test_set_default_provider` uses in `execution::manager`'s tests.
        let cassette = temp.path().join("empty.json");
        std::fs::write(&cassette, "{}").unwrap();
        let provider = std::sync::Arc::new(
            crate::providers::testprovider::TestProvider::new_replaying(cassette.to_str().unwrap())
                .unwrap(),
        );
        let workflow: Workflow = serde_json::from_value(serde_json::json!({
            "title": "t", "description": "d",
            "instructions": "do the thing", "prompt": "go"
        }))
        .unwrap();
        let task_config = TaskConfig {
            provider,
            parent_session_id: "parent-1".into(),
            parent_working_dir: temp.path().to_path_buf(),
            extensions: vec![],
            max_turns: Some(3),
            privacy_tier: crate::privacy::SessionClassification::Public,
            dropped_private_extensions: Vec::new(),
            dropped_cross_affiliation_extensions: Vec::new(),
        };

        // The sentinel: an agent nobody else owns, parked under the child's id.
        // `AgentManager::instance()` resolves `Paths::data_dir()` and runs
        // `run_first_run_init` on first use, so it must never resolve to the
        // developer's real `~/.config/biorouter`. `crate::test_sandbox`'s `ctor`
        // guarantees that for every test in this binary, whether or not the
        // caller exported `BIOROUTER_PATH_ROOT`.
        let manager = crate::execution::manager::AgentManager::instance()
            .await
            .expect("AgentManager::instance (config root sandboxed by crate::test_sandbox)");
        let sentinel = std::sync::Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                sm.clone(),
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));
        manager
            .register_agent(child.id.clone(), sentinel.clone())
            .await;

        let result =
            run_subagent_bounded!(config, workflow, task_config, true, child.id.clone(), None);

        // The provider fails, so the envelope reports an error/incomplete run.
        // Assert its SHAPE — `SubagentResult` always carries a status and a
        // non-empty summary (`subagent_result.rs`) — rather than merely that
        // serializing it produced bytes.
        let rendered = serde_json::to_value(&result).unwrap();
        assert!(
            rendered.is_object(),
            "a structured envelope, got: {rendered}"
        );
        assert!(
            rendered.get("status").is_some(),
            "…with a status: {rendered}"
        );
        assert!(
            rendered
                .get("summary")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty()),
            "…and a non-empty summary, even for a run that failed: {rendered}"
        );

        // The wiring: the run replaced the sentinel with its own live child and
        // deregistered that child on the way out. `Deregister::drop` finishes the
        // work on a spawned task, so poll rather than assume it has landed.
        for _ in 0..100 {
            if !manager.has_session(&child.id).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            !manager.has_session(&child.id).await,
            "run_complete_subagent_task must register the live child (which replaces the \
             sentinel) and deregister it on exit; the sentinel is still there, so nothing \
             registered and /interrupt would mint a different agent"
        );
        // Belt and braces: if the run somehow left the SENTINEL registered, the
        // assertion above would already have fired — but make the failure mode
        // unambiguous for whoever reads the output.
        manager.deregister_agent_if_same(&child.id, &sentinel).await;
    }

    /// The run's teardown must be IDENTITY-SCOPED, not a blunt eviction.
    ///
    /// This is the half of the wiring neither existing gate can see.
    /// `subagent_run_without_daemon_services_still_completes` proves the pin is
    /// gone once the run ends; `the_run_holds_the_server_turn_lease_for_its_whole_run`
    /// proves the live child was in it while the run worked. Both are satisfied
    /// just as well by a teardown that calls `manager.remove_session(&session_id)`,
    /// because that also leaves the pin empty. Until now the only thing standing
    /// between the two was the Step-5 grep (`remove_session(` must be 0 in this
    /// file) — and a grep in a plan is not a gate: swap the blunt remover in
    /// later and every one of the 1857 lib tests stays green.
    ///
    /// The observable difference is the **LRU**, which `remove_session` pops and
    /// `deregister_agent_if_same` deliberately does not. A blunt teardown
    /// therefore evicts a cache entry the run never created. In production that
    /// entry is the agent a consulted Agent Drafter worker got from an ordinary
    /// `get_agent` (`routes/apps.rs`), thrown away on every consult — and, worse,
    /// a plain remove would evict a *live* registration belonging to someone
    /// else, which is the whole reason release is identity-scoped.
    ///
    /// So: park exactly such a bystander under the child's id and watch whether
    /// the run's exit takes it with it. `register_agent` never touches the LRU,
    /// so the run's pin merely shadows the bystander for the run's duration and
    /// it must be there again afterwards.
    ///
    /// (`deregistering_does_not_evict_a_cache_entry_it_did_not_create` asserts
    /// this of the METHOD, called by hand. This asserts it of the RUN, which is
    /// what the grep was standing in for.)
    #[tokio::test]
    #[serial_test::parallel(workspace_services)]
    #[serial_test::serial(subagent_session_bus, agent_manager_pin)]
    async fn the_runs_teardown_does_not_evict_a_cache_entry_it_did_not_create() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        // An id no store would mint, so this test shares neither a bus ring nor
        // an `AgentManager` entry with any other test in the binary.
        let child = "ghost-session-teardown-scope".to_string();

        let manager = crate::execution::manager::AgentManager::instance()
            .await
            .expect("AgentManager::instance (config root sandboxed by crate::test_sandbox)");

        // The bystander: an ORDINARY cached agent, exactly what `get_agent`
        // leaves behind for a session someone opened.
        let cached = manager.get_or_create_agent(child.clone()).await.unwrap();

        let (config, workflow, task_config) = bracket_fixture(&temp, &sm);
        let _ = run_subagent_bounded!(config, workflow, task_config, true, child.clone(), None);

        // `Deregister::drop` releases on a spawned task, so poll until the pin
        // stops answering — `peek_agent` reports the run's live child until then,
        // and it is neither the bystander nor `None`.
        let mut outcome = None;
        for _ in 0..200 {
            match manager.peek_agent(&child).await {
                Some(a) if Arc::ptr_eq(&a, &cached) => {
                    outcome = Some(true);
                    break;
                }
                None => {
                    outcome = Some(false);
                    break;
                }
                Some(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
        let _ = manager.remove_session(&child).await;
        match outcome {
            Some(true) => {}
            Some(false) => panic!(
                "the run's teardown evicted the cached agent it found under the child's id. \
                 Release must be `deregister_agent_if_same`, which clears only the pin; a \
                 `remove_session` here also pops the LRU, discarding an entry this run never \
                 created (in production, a consulted worker's own agent) and evicting live \
                 registrations belonging to other runs"
            ),
            None => panic!(
                "the run never released its registration: `peek_agent` still resolves an \
                 agent that is neither the bystander nor absent after 2 s"
            ),
        }
    }

    /// Releasing a registration with no runtime alive must not panic.
    ///
    /// `tokio::spawn` panics when there is no reactor, and this guard is dropped
    /// from `Drop` — where a panic during an unwind ABORTS the process. The
    /// shapes that reach it are real: a future dropped by runtime shutdown, or
    /// one moved out of the runtime that created it. The pin then leaks, which
    /// is the right trade for a process that is going away; crashing is not.
    ///
    /// The runtime is dropped BEFORE the guard, which is exactly the ordering
    /// that makes `tokio::spawn` panic here — swap `Handle::try_current` back
    /// for a bare `tokio::spawn` and this test fails on that panic.
    #[test]
    fn releasing_a_registration_without_a_runtime_does_not_panic() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let guard = runtime.block_on(async {
            let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
            let manager = std::sync::Arc::new(
                crate::execution::manager::AgentManager::new(
                    sm.clone(),
                    temp.path().join("schedule.json"),
                    Some(4),
                )
                .await
                .unwrap(),
            );
            let agent = std::sync::Arc::new(Agent::with_config(AgentConfig::new(
                sm,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            )));
            manager
                .register_agent("orphan-child".to_string(), agent.clone())
                .await;
            Deregister {
                manager: Some((manager, agent)),
                session_id: "orphan-child".to_string(),
            }
        });

        drop(runtime); // the runtime is gone …
        drop(guard); // … and this must still be survivable.
    }

    /// The turn id [`LeaseSpy`]'s lease hands out. Distinct from the synthetic
    /// `subagent-<id>` a headless run mints, so a bracket that adopted the
    /// wrong one is visible in the frame itself.
    const LEASE_TURN_ID: &str = "lease-turn-33";

    /// What the run looked like from INSIDE, at [`LeaseSpy`]'s observation
    /// point.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct MidRun {
        /// Was the server turn lease still held? `false` for a run that took the
        /// lease and dropped it immediately (`let _ = services.begin_turn(..)`),
        /// which reads as correct at every other seam.
        lease_held: bool,
        /// What `AgentManager` resolved for the child at that moment.
        pinned: PinnedMidRun,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PinnedMidRun {
        /// No sentinel was parked, so the spy did not look.
        NotChecked,
        /// Nothing at all is registered for the child.
        Nothing,
        /// Still the agent the test parked — so the run never registered.
        Sentinel,
        /// An agent that is neither: the live child the run registered.
        Registered,
    }

    /// A daemon stand-in that records what a run does with the **server turn
    /// lease** — the half of this task its title names.
    ///
    /// ⚠ Why this exists (2026-07-31 review). Every other test in this file runs
    /// headless: `workspace_services::get()` is `None`, so the whole `begin_turn`
    /// block in `run_complete_subagent_task` takes its `None => None` arm and
    /// deleting the block outright — lease, conflict return and all — was
    /// indistinguishable to the entire lib suite. Four properties were asserted
    /// only in prose: the lease is taken for the child, it is *held* for the
    /// run, `cancel_turn` on it trips the run's own token, and a busy session is
    /// refused instead of double-run.
    ///
    /// The mid-run observation point is `knowledge_selection`, which
    /// `get_agent_messages` calls after it has taken the lease and registered the
    /// child agent and before the reply stream. It is the only seam inside the
    /// run a `WorkspaceServices` implementation can observe, and it is what turns
    /// "the lease is still held while the child works" from an inference into an
    /// assertion.
    struct LeaseSpy {
        /// Every `begin_turn` call: the session id and the token it was handed.
        /// Holding the token is what lets `cancel_turn` behave like the daemon's.
        begun: std::sync::Mutex<Vec<(String, CancellationToken)>>,
        /// Sessions whose lease is alive: inserted by `begin_turn`, removed by
        /// `SpyLease::drop`. Shared with the lease, hence the `Arc`.
        active: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
        cancels: std::sync::Mutex<Vec<String>>,
        /// `Some(msg)` refuses the lease, the way a session with a turn already
        /// in flight does.
        conflict: Option<String>,
        /// Call `cancel_turn` from the mid-run hook — a `POST /agent/cancel`
        /// landing while the child is working, at a deterministic point instead
        /// of a raced one.
        cancel_mid_run: bool,
        /// The agent parked under the child's id before the run, so the mid-run
        /// peek can tell "the run registered its own" from "nothing did".
        sentinel: Option<Arc<Agent>>,
        mid_run: std::sync::Mutex<Option<MidRun>>,
    }

    struct SpyLease {
        session_id: String,
        active: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    }

    impl WorkspaceTurnLease for SpyLease {
        fn turn_id(&self) -> &str {
            LEASE_TURN_ID
        }
    }

    /// Releasing the session is what dropping a lease MEANS; the real one drops
    /// the server's `TurnGuard`.
    impl Drop for SpyLease {
        fn drop(&mut self) {
            self.active.lock().unwrap().remove(self.session_id.as_str());
        }
    }

    impl LeaseSpy {
        fn new() -> Self {
            Self {
                begun: std::sync::Mutex::new(Vec::new()),
                active: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
                cancels: std::sync::Mutex::new(Vec::new()),
                conflict: None,
                cancel_mid_run: false,
                sentinel: None,
                mid_run: std::sync::Mutex::new(None),
            }
        }
        fn refusing(mut self, message: &str) -> Self {
            self.conflict = Some(message.to_string());
            self
        }
        fn cancelling_mid_run(mut self) -> Self {
            self.cancel_mid_run = true;
            self
        }
        fn watching_for(mut self, sentinel: Arc<Agent>) -> Self {
            self.sentinel = Some(sentinel);
            self
        }
        fn install(self) -> Arc<Self> {
            let me = Arc::new(self);
            crate::workspace_services::set_for_tests(Some(me.clone()));
            me
        }
        fn begun(&self) -> Vec<String> {
            self.begun
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }
        fn mid_run(&self) -> MidRun {
            self.mid_run
                .lock()
                .unwrap()
                .expect("the run must reach the mid-run observation point")
        }
    }

    #[async_trait::async_trait]
    impl WorkspaceServices for LeaseSpy {
        fn gui_attached(&self) -> bool {
            false
        }
        fn layout_snapshot(&self) -> Option<serde_json::Value> {
            None
        }
        fn is_turn_active(&self, session_id: &str) -> bool {
            self.active.lock().unwrap().contains(session_id)
        }
        /// The daemon's shape: find the running turn's token, trip it, name the
        /// turn. A session with no lease has nothing to cancel.
        fn cancel_turn(&self, session_id: &str) -> Option<String> {
            self.cancels.lock().unwrap().push(session_id.to_string());
            let begun = self.begun.lock().unwrap();
            let (_, token) = begun.iter().find(|(id, _)| id == session_id)?;
            token.cancel();
            Some(LEASE_TURN_ID.to_string())
        }
        fn begin_turn(
            &self,
            session_id: &str,
            cancel: CancellationToken,
        ) -> Result<Box<dyn WorkspaceTurnLease>, String> {
            if let Some(conflict) = &self.conflict {
                return Err(conflict.clone());
            }
            self.begun
                .lock()
                .unwrap()
                .push((session_id.to_string(), cancel));
            self.active.lock().unwrap().insert(session_id.to_string());
            Ok(Box::new(SpyLease {
                session_id: session_id.to_string(),
                active: Arc::clone(&self.active),
            }))
        }
        async fn stop_agent(&self, _session_id: &str) -> Result<(), String> {
            Ok(())
        }
        async fn start_detached_turn(
            &self,
            _session_id: &str,
            _message: Message,
        ) -> Result<String, String> {
            Err("LeaseSpy starts no turns".into())
        }
        async fn start_session(
            &self,
            _working_dir: std::path::PathBuf,
            _extensions: Option<Vec<String>>,
            _knowledge_bases: Vec<String>,
            _primary: crate::workspace_services::KbPrimaryChoice,
        ) -> Result<String, String> {
            Err("LeaseSpy starts no sessions".into())
        }
        fn set_knowledge_bases(
            &self,
            _session_id: &str,
            _kbs: &[String],
            _primary: crate::workspace_services::KbPrimaryChoice,
        ) -> Result<crate::workspace_services::KbSelectionView, String> {
            Err("LeaseSpy sets no knowledge bases".into())
        }
        /// **The observation point.** `get_agent_messages` calls this from
        /// inside the run — after the lease and after the registration, before
        /// the reply stream — so what it sees here is what the daemon would see
        /// while the child works.
        ///
        /// The `block_on` is legal because that call site wraps this in
        /// `spawn_blocking`: a blocking-pool thread is not a runtime worker. If
        /// it is ever changed to a direct await, this panics loudly rather than
        /// silently recording nothing.
        fn knowledge_selection(
            &self,
            session_id: &str,
        ) -> crate::workspace_services::KbSelectionView {
            let pinned = match &self.sentinel {
                None => PinnedMidRun::NotChecked,
                Some(sentinel) => {
                    let live = tokio::runtime::Handle::current().block_on(async {
                        crate::execution::manager::AgentManager::instance()
                            .await
                            .expect("agent manager")
                            .peek_agent(session_id)
                            .await
                    });
                    match live {
                        None => PinnedMidRun::Nothing,
                        Some(a) if Arc::ptr_eq(sentinel, &a) => PinnedMidRun::Sentinel,
                        Some(_) => PinnedMidRun::Registered,
                    }
                }
            };
            *self.mid_run.lock().unwrap() = Some(MidRun {
                lease_held: self.is_turn_active(session_id),
                pinned,
            });
            if self.cancel_mid_run {
                self.cancel_turn(session_id);
            }
            crate::workspace_services::KbSelectionView::default()
        }
        async fn gui_command(
            &self,
            _frame: serde_json::Value,
            _wait_result: bool,
        ) -> Result<serde_json::Value, String> {
            Err("no GUI attached".into())
        }
    }

    /// With the daemon present, the run takes the server's per-session turn
    /// lock, **holds it for the whole run**, publishes its bracket under the
    /// lease's turn id, and has its live child registered while it works.
    ///
    /// Those are the four claims path 1 and path 3 of this task stand on
    /// (`is_turn_active(child)` true → `POST /interrupt` passes its
    /// precondition → `get_agent_for_route` resolves the REGISTERED child).
    /// Each is separately falsifiable here: drop the `begin_turn` block and
    /// `begun` is empty; bind the lease to `_` and `lease_held` is false; pass
    /// the lease's id nowhere and the bracket carries `subagent-<id>`; drop the
    /// `register_agent` call and the mid-run peek still finds the sentinel.
    #[tokio::test]
    #[serial_test::serial(workspace_services)]
    async fn the_run_holds_the_server_turn_lease_for_its_whole_run() {
        use crate::session_events::{self, SessionBusEvent};
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = sm
            .create_session(
                temp.path().to_path_buf(),
                "lease-held child".into(),
                crate::session::SessionType::SubAgent,
            )
            .await
            .unwrap()
            .id;
        let mut rx = session_events::subscribe(&child);

        let manager = crate::execution::manager::AgentManager::instance()
            .await
            .expect("AgentManager::instance (config root sandboxed by crate::test_sandbox)");
        let sentinel = std::sync::Arc::new(Agent::with_config(AgentConfig::new(
            sm.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            crate::config::BioRouterMode::Auto,
        )));
        manager
            .register_agent(child.clone(), sentinel.clone())
            .await;

        let spy = LeaseSpy::new().watching_for(sentinel.clone()).install();
        let (config, workflow, task_config) = bracket_fixture(&temp, &sm);
        let result =
            run_subagent_bounded!(config, workflow, task_config, true, child.clone(), None);
        crate::workspace_services::clear_test_override();

        assert_eq!(
            result.status,
            crate::agents::subagent_result::SubagentStatus::Error,
            "fixture precondition: the empty replay provider must fail after the run reaches \
             its mid-run observation point; got {result:?}"
        );
        assert_eq!(
            spy.begun(),
            vec![child.clone()],
            "the run must take the server turn lease exactly once, for the CHILD session"
        );
        let mid_run = spy.mid_run();
        assert!(
            mid_run.lease_held,
            "the lease must still be HELD while the child works; a run that released it \
             immediately reports the child idle for its whole run, and `mode:\"turn\"` on a \
             busy child would be accepted instead of refused"
        );
        assert_eq!(
            mid_run.pinned,
            PinnedMidRun::Registered,
            "…and the live child must be registered by then, or a mid-run steer resolves \
             the sentinel (here) / a freshly minted agent (in production) that no loop drains"
        );
        assert!(
            !spy.is_turn_active(&child),
            "the lease must be RELEASED when the run ends, or the child's session is busy \
             forever and its next turn is refused"
        );

        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.first(),
                Some(SessionBusEvent::TurnStarted { turn_id }) if turn_id == LEASE_TURN_ID
            ),
            "the bracket must adopt the LEASE's turn id, so an observer can correlate the \
             child's turn with the id `POST /agent/cancel` reports; got {events:?}"
        );

        // `Deregister::drop` releases on a spawned task, so poll.
        for _ in 0..100 {
            if !manager.has_session(&child).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            !manager.has_session(&child).await,
            "and the registration must not outlive the run"
        );
        manager.deregister_agent_if_same(&child, &sentinel).await;
    }

    /// Path 2 (Stop/abort): `cancel_turn` on the CHILD ends the child's run, and
    /// does not touch the parent's turn.
    ///
    /// The token the spy trips is the one the run handed to `begin_turn` — so a
    /// run that handed over anything else (a fresh token, or one it does not
    /// itself observe) closes its bracket as `error` here instead of
    /// `cancelled`. The parent assertion is the other half of reconciliation
    /// #2's "one token per run": the run's token is a CHILD of the parent's, so
    /// stopping the delegate must not stop the conversation that delegated.
    #[tokio::test]
    #[serial_test::serial(workspace_services)]
    async fn cancel_turn_on_the_child_stops_the_run_and_spares_the_parent() {
        use crate::session_events::{self, SessionBusEvent};
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = sm
            .create_session(
                temp.path().to_path_buf(),
                "lease-cancelled child".into(),
                crate::session::SessionType::SubAgent,
            )
            .await
            .unwrap()
            .id;
        let mut rx = session_events::subscribe(&child);

        let spy = LeaseSpy::new().cancelling_mid_run().install();
        let (config, workflow, task_config) = bracket_fixture(&temp, &sm);
        let parent_token = CancellationToken::new();
        let _ = run_subagent_bounded!(
            config,
            workflow,
            task_config,
            true,
            child.clone(),
            Some(parent_token.clone()),
        );
        crate::workspace_services::clear_test_override();

        assert_eq!(
            spy.cancels.lock().unwrap().as_slice(),
            std::slice::from_ref(&child),
            "fixture precondition: the mid-run hook must have called cancel_turn once"
        );
        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, .. }) if reason == "cancelled"
            ),
            "tripping the token the run handed to `begin_turn` must stop THE RUN; it is \
             the run's own token, not a fresh one made to satisfy the signature; got {events:?}"
        );
        assert!(
            !parent_token.is_cancelled(),
            "cancelling the child must not cancel the parent's turn: the run token is a \
             CHILD of the parent's, not the parent's own"
        );
    }

    /// A child session the daemon says is already busy must be REFUSED, not
    /// double-run — the one-turn-per-session invariant, from the side that can
    /// break it silently.
    ///
    /// Refused means refused: no bracket on the bus (an observer would see a
    /// turn that never ran) and no agent registered (a `/reply` would resolve a
    /// child nothing is driving).
    #[tokio::test]
    #[serial_test::serial(workspace_services)]
    async fn a_busy_child_session_is_refused_instead_of_double_run() {
        use crate::session_events::{self, SessionBusEvent};
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = "ghost-session-lease-conflict".to_string();
        let mut rx = session_events::subscribe(&child);

        let _spy = LeaseSpy::new()
            .refusing("turn-77 is already running")
            .install();
        let (config, workflow, task_config) = bracket_fixture(&temp, &sm);
        let result =
            run_subagent_bounded!(config, workflow, task_config, true, child.clone(), None);
        crate::workspace_services::clear_test_override();

        assert_eq!(
            result.status,
            crate::agents::subagent_result::SubagentStatus::Error,
            "a refused lease is a failed run; got {result:?}"
        );
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("turn-77") && e.contains("busy")),
            "…and the envelope must name the conflict the daemon reported; got {result:?}"
        );
        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events.is_empty(),
            "a refused run must never open a bracket; got {events:?}"
        );
        let manager = crate::execution::manager::AgentManager::instance()
            .await
            .expect("AgentManager::instance (config root sandboxed by crate::test_sandbox)");
        assert!(
            !manager.has_session(&child).await,
            "…and must never register an agent for a turn it did not start"
        );
    }

    /// ⚠ **Serialized, and it has to be.** The session bus is a process-global
    /// map keyed by session **id**, but ids are minted per *store* as
    /// `<date>_<n>` (`session_manager.rs`'s `CLAIM_NEXT_SESSION_N`, whose
    /// high-water mark is per store and so starts at 1 in each one), so
    /// two tests that each stand up their own `TempDir` `SessionManager` both
    /// get `<today>_1` and publish into the *same* ring. Anything asserting on
    /// the sequence then reads another test's frames interleaved with its own.
    ///
    /// This is not hypothetical and it is not new: it is why the frame count
    /// here varies between runs. The `exactly one TurnStarted` assertion below
    /// is what turned it from silent noise into a failure, and the serial key
    /// covers every test in this binary that runs a real subagent against a
    /// minted id (today: this one and
    /// `subagent_run_without_daemon_services_still_completes`). A new one must
    /// join the key — or use an id no store would mint, as the two bracket
    /// tests below do.
    ///
    /// `agent_manager_pin` is the SAME collision one layer up: this run
    /// registers `<today>_1` in the process-global `AgentManager` pin, and
    /// `workspace_extension`'s `the_default_scope_sees_a_registered_child_…`
    /// registers its own `<today>_1` there and then asserts on it. The bus key
    /// cannot cover that test — it publishes nothing — so the pin needs a key
    /// of its own, shared across both files.
    #[tokio::test]
    #[serial_test::parallel(workspace_services)]
    #[serial_test::serial(subagent_session_bus, agent_manager_pin)]
    async fn subagent_run_publishes_lifecycle_to_the_bus() {
        use crate::session_events::{self, SessionBusEvent};
        // A run with no provider fails fast — but must still bracket itself,
        // exactly like the detached runner (Task 8's test).
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = sm
            .create_session(
                temp.path().to_path_buf(),
                "child".into(),
                crate::session::session_manager::SessionType::SubAgent,
            )
            .await
            .unwrap();
        let mut rx = session_events::subscribe(&child.id);

        let config = AgentConfig::new(
            sm.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            crate::config::BioRouterMode::Auto,
        );
        // Workflow has NO Default (`title`/`description` are required, `version`
        // has a serde default) — build it via serde.
        let workflow: Workflow = serde_json::from_value(serde_json::json!({
            "title": "t", "description": "d",
            "instructions": "do the thing", "prompt": "go"
        }))
        .unwrap();
        // The verified cheap provider: TestProvider replaying an empty cassette
        // fails on first use — the exact pattern `test_set_default_provider`
        // uses in `execution::manager`'s tests.
        let cassette = temp.path().join("empty.json");
        std::fs::write(&cassette, "{}").unwrap();
        let provider = std::sync::Arc::new(
            crate::providers::testprovider::TestProvider::new_replaying(cassette.to_str().unwrap())
                .unwrap(),
        );
        let task_config = TaskConfig {
            provider,
            parent_session_id: "parent-1".into(),
            parent_working_dir: temp.path().to_path_buf(),
            extensions: vec![],
            max_turns: Some(3),
            privacy_tier: crate::privacy::SessionClassification::Public,
            dropped_private_extensions: Vec::new(),
            dropped_cross_affiliation_extensions: Vec::new(),
        };

        let _result =
            run_subagent_bounded!(config, workflow, task_config, true, child.id.clone(), None);

        // Drain the whole ring into a Vec, because every property this task is
        // about is a property of the SEQUENCE, not of set membership. The
        // original `saw_started && saw_finished` pair could not tell this
        // implementation apart from one that published the two brackets and
        // deleted the tee entirely — `_ => {}` swallowed every `Agent(..)`
        // frame, which is the task's actual deliverable.
        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let kinds: Vec<String> = events
            .iter()
            .map(|e| match e {
                SessionBusEvent::TurnStarted { turn_id } => format!("TurnStarted({turn_id})"),
                SessionBusEvent::TurnError { code, .. } => format!("TurnError({code})"),
                SessionBusEvent::TurnFinished { reason, .. } => format!("TurnFinished({reason})"),
                SessionBusEvent::Agent(AgentEvent::Message(_)) => "Agent(Message)".into(),
                SessionBusEvent::Agent(AgentEvent::MessagesPersisted(ids)) => {
                    format!("Agent(MessagesPersisted[{}])", ids.len())
                }
                SessionBusEvent::Agent(AgentEvent::TurnAborted { code, .. }) => {
                    format!("Agent(TurnAborted({}))", code.wire_code())
                }
                SessionBusEvent::Agent(other) => format!("Agent({other:?})"),
            })
            .collect();

        // 1. The bracket: exactly one of each, first and last, nothing outside.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, SessionBusEvent::TurnStarted { .. }))
                .count(),
            1,
            "exactly one TurnStarted; got {kinds:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e,
                    SessionBusEvent::TurnFinished { .. } | SessionBusEvent::TurnError { .. }
                ))
                .count(),
            1,
            "exactly one terminal frame; got {kinds:?}"
        );
        // The synthetic id, asserted exactly: headless there is no server turn
        // lease, so `lease_turn_id` is `None` and the run must mint
        // `subagent-<session id>`. Nothing else read `turn_id`, so a run that
        // threaded the parameter but dropped it on the floor passed before.
        assert!(
            matches!(
                &events[0],
                SessionBusEvent::TurnStarted { turn_id } if *turn_id == format!("subagent-{}", child.id)
            ),
            "first frame must be TurnStarted carrying the synthetic turn id \
             `subagent-{}`; got {kinds:?}",
            child.id
        );
        // The reason ladder, asserted by value: this run aborts (the empty
        // cassette has no recorded response), so the terminal must say `error`.
        // `TurnFinished { .. }` would have accepted a ladder hardcoded to
        // `stop`.
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, token_state: None })
                    if reason == "error"
            ),
            "last frame must be TurnFinished{{reason:\"error\", token_state:None}} \
             for a run whose provider failed; got {kinds:?}"
        );

        // 2. The tee exists at all, and is TOTAL — not filtered to the variants
        //    the parent's own `match` happens to act on. `MessagesPersisted` is
        //    the discriminating case: the parent explicitly ignores it (it has
        //    no `expectedMessageIds` to satisfy), so a tee written as "publish
        //    what I handle" drops it — and an observer tab, which IS a full
        //    client, would never learn its rows are durable (#59).
        let agent_frames = events
            .iter()
            .filter(|e| matches!(e, SessionBusEvent::Agent(_)))
            .count();
        // A deliberately loose floor: the exact frame count varies between runs
        // (the loop retries the failing provider), so the two named-variant
        // assertions below are what discriminate. This one only says "the tee
        // ran at all".
        assert!(
            agent_frames >= 2,
            "the child's own agent events must be teed onto the bus; got {kinds:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SessionBusEvent::Agent(AgentEvent::MessagesPersisted(_)))),
            "the tee must be TOTAL: `MessagesPersisted` is ignored by the parent's \
             accumulation but is exactly what an observer needs (#59); got {kinds:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SessionBusEvent::Agent(AgentEvent::TurnAborted { .. }))),
            "…including the variant that breaks the loop, which must be published \
             BEFORE the `break`; got {kinds:?}"
        );

        // 3. Order preservation — the #59 invariant this relay must not undo.
        //    No `MessagesPersisted` may name an id whose `Message` frame comes
        //    later in the stream; a client that saw the id first would render a
        //    row it has no content for. (Ids the stream never carries a
        //    `Message` for — the caller's own user message — are the client's
        //    already and are correctly unconstrained.)
        let message_position: std::collections::HashMap<String, usize> = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                SessionBusEvent::Agent(AgentEvent::Message(m)) => m.id.clone().map(|id| (id, i)),
                _ => None,
            })
            .collect();
        for (i, event) in events.iter().enumerate() {
            if let SessionBusEvent::Agent(AgentEvent::MessagesPersisted(persisted)) = event {
                for row in persisted {
                    if let Some(&msg_at) = message_position.get(&row.id) {
                        assert!(
                            msg_at < i,
                            "relay reordered the stream: MessagesPersisted at {i} names {} \
                             whose Message frame is at {msg_at}; got {kinds:?}",
                            row.id
                        );
                    }
                }
            }
        }

        // Task 32 follow-through (the overwrite guard): the spawn-context
        // record survives the child's own persistence as message[0].
        //
        // ⚠ `expect`, NOT `if let Some(first) = msgs.first()`. The `if let` form
        // goes vacuous in **exactly** the case it exists to catch: if
        // `persist_spawn_context` was never wired into `get_agent_messages`, the
        // child's conversation after a fast-failing provider is EMPTY, `first()`
        // is `None`, and the assertion is skipped. Task 32's own test calls the
        // helper directly, so that defect would have two gates and both green.
        // Do not reintroduce the guard "for the empty case" — the empty case is
        // the bug.
        let reread = sm.get_session(&child.id, true).await.unwrap();
        let msgs = reread.conversation.unwrap().messages().to_vec();
        let first = msgs.first().expect(
            "the spawn-context record must exist: an empty child conversation means \
             Task 32's persist_spawn_context is defined but never called from \
             get_agent_messages",
        );
        assert!(
            first.metadata.provenance.as_ref().is_some_and(|p| {
                p.kind == crate::conversation::message::ProvenanceKind::SpawnContext
            }),
            "spawn-context record must remain the FIRST message; got {:?}",
            first.metadata.provenance
        );
        assert!(
            !first.metadata.agent_visible,
            "…and stay out of the child's model context (Task 32); a run that \
             re-persisted it as an ordinary row would double-inject the system prompt"
        );
    }

    /// Build the fixture a bracket test needs, for a session id the caller
    /// chooses — including one that was never created.
    fn bracket_fixture(
        temp: &tempfile::TempDir,
        sm: &std::sync::Arc<SessionManager>,
    ) -> (AgentConfig, Workflow, TaskConfig) {
        let config = AgentConfig::new(
            sm.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            crate::config::BioRouterMode::Auto,
        );
        let workflow: Workflow = serde_json::from_value(serde_json::json!({
            "title": "t", "description": "d",
            "instructions": "do the thing", "prompt": "go"
        }))
        .unwrap();
        let cassette = temp.path().join("empty.json");
        std::fs::write(&cassette, "{}").unwrap();
        let provider = std::sync::Arc::new(
            crate::providers::testprovider::TestProvider::new_replaying(cassette.to_str().unwrap())
                .unwrap(),
        );
        let task_config = TaskConfig {
            provider,
            parent_session_id: "parent-1".into(),
            parent_working_dir: temp.path().to_path_buf(),
            extensions: vec![],
            max_turns: Some(3),
            privacy_tier: crate::privacy::SessionClassification::Public,
            dropped_private_extensions: Vec::new(),
            dropped_cross_affiliation_extensions: Vec::new(),
        };
        (config, workflow, task_config)
    }

    /// A run that never produces a single stream event must still bracket
    /// itself: `TurnStarted` first, a terminal last, nothing in between.
    ///
    /// The window matters because **in production** the server's turn lease is
    /// already held by the time `get_agent_messages` runs, so the daemon is
    /// answering `is_turn_active(child) == true` and a client has been told the
    /// child is busy. A run that returned `Err` from here without a terminal
    /// leaves an observer watching a session the daemon called busy and then
    /// silently wasn't.
    ///
    /// ⚠ Not in THIS test, and the distinction was worth writing down: no
    /// `WorkspaceServices` is installed here, so no lease is taken and
    /// `is_turn_active` is never true. This test gates the bracket's placement,
    /// nothing about the lease. The lease is gated by
    /// `the_run_holds_the_server_turn_lease_for_its_whole_run` above — which is
    /// where a reader auditing lease coverage should look, rather than reading
    /// this comment as evidence and stopping.
    ///
    /// Naming a session that was never created is what reaches that path for an
    /// ordinary reason rather than through a test-only seam: the child's first
    /// message insert violates the sessions foreign key, so `agent.reply(..)`
    /// fails at construction and the stream never yields.
    ///
    /// Two `?` exits sit *even earlier* (`update_provider`, the system-prompt
    /// render) and are not reachable from a unit test — neither fails for any
    /// input a caller controls. They are covered by the guard itself, gated by
    /// `an_unclosed_bracket_still_publishes_a_terminal_when_dropped` below.
    #[tokio::test]
    #[serial_test::parallel(workspace_services)]
    async fn subagent_run_with_no_stream_events_still_brackets_itself() {
        use crate::session_events::{self, SessionBusEvent};
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        // Deliberately never created.
        let ghost = "ghost-session-never-created".to_string();
        let mut rx = session_events::subscribe(&ghost);
        let (config, workflow, task_config) = bracket_fixture(&temp, &sm);

        let result =
            run_subagent_bounded!(config, workflow, task_config, true, ghost.clone(), None);
        assert_eq!(
            result.status,
            crate::agents::subagent_result::SubagentStatus::Error,
            "fixture precondition: the run must fail before its first event; got {result:?}"
        );

        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.first(),
                Some(SessionBusEvent::TurnStarted { turn_id }) if *turn_id == format!("subagent-{ghost}")
            ),
            "a run whose lease is held must open its bracket before the work that \
             can fail; got {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, .. }) if reason == "error"
            ),
            "…and must close it on the failing exit, or an observer waits forever; \
             got {events:?}"
        );
        assert_eq!(
            events.len(),
            2,
            "fixture precondition: this run must reach no stream event at all, so the \
             bracket is the only thing under test; got {events:?}"
        );
    }

    /// The same failure, but with the run's token already tripped: the terminal
    /// must say `cancelled`, not `error`.
    ///
    /// This exit never reaches the ladder at the bottom of the stream loop, so
    /// it is the one run-level gate on the guard's own reason probe. A run the
    /// user stopped is not a run that failed — the previous implementation
    /// hardcoded `"error"` on this path.
    #[tokio::test]
    #[serial_test::parallel(workspace_services)]
    async fn a_cancelled_run_that_never_reached_the_stream_closes_as_cancelled() {
        use crate::session_events::{self, SessionBusEvent};
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let ghost = "ghost-session-cancelled".to_string();
        let mut rx = session_events::subscribe(&ghost);
        let (config, workflow, task_config) = bracket_fixture(&temp, &sm);

        // `run_complete_subagent_task` derives the run token as a CHILD of this
        // one, so a parent that is already cancelled hands the run a cancelled
        // token — the shape a stopped parent turn produces.
        let parent_token = CancellationToken::new();
        parent_token.cancel();
        let _ = run_subagent_bounded!(
            config,
            workflow,
            task_config,
            true,
            ghost.clone(),
            Some(parent_token),
        );

        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, .. }) if reason == "cancelled"
            ),
            "a stopped run must close as `cancelled`, not `error`; got {events:?}"
        );
    }

    /// A run that dies **between the lease and the stream** still closes its
    /// bracket — the window `subagent_run_with_no_stream_events_still_brackets_itself`
    /// cannot reach.
    ///
    /// That window is where the bracket's *placement* is decided, and nothing
    /// else gates it: move the `BusTurnBracket::open` call back down next to
    /// `agent.reply(..)` and every other test in this file still passes, because
    /// they all fail at or after that line. The two `?` exits in the window
    /// (`update_provider`, the system-prompt render) cannot be made to fail for
    /// any input a caller controls — an `UPDATE` matching no row is not an
    /// error, and the prompt template is embedded in the binary — so the failure
    /// used here is a **panic**, which is one of the three exits the guard
    /// exists for anyway (the others being `?` and the future being dropped).
    ///
    /// `max_turns: None` reaches it: the prompt context `expect`s the value.
    /// If that `expect` is ever softened to a default, this run will reach the
    /// stream instead and the "exactly 2 frames" assertion below fails loudly
    /// rather than going quietly vacuous.
    #[tokio::test]
    #[serial_test::parallel(workspace_services)]
    async fn a_run_that_panics_before_the_stream_still_closes_its_bracket() {
        use crate::session_events::{self, SessionBusEvent};
        let temp = tempfile::TempDir::new().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(temp.path().to_path_buf()));
        // An id no store would mint, so this test needs no serial key.
        let ghost = "ghost-session-panics".to_string();
        let mut rx = session_events::subscribe(&ghost);
        let (config, workflow, mut task_config) = bracket_fixture(&temp, &sm);
        task_config.max_turns = None;

        // Spawned so the panic is contained here instead of failing the test:
        // it is the subject, not an accident.
        let joined = tokio::spawn(async move {
            run_subagent_bounded!(config, workflow, task_config, true, ghost, None)
        })
        .await;
        assert!(
            joined.is_err_and(|e| e.is_panic()),
            "fixture precondition: the run must panic before reaching the stream"
        );

        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            events.len(),
            2,
            "the run must have died before its first stream event; got {events:?}"
        );
        assert!(
            matches!(events.first(), Some(SessionBusEvent::TurnStarted { .. })),
            "the bracket must be OPEN before the work that can die: the lease is \
             already held, so the daemon is already reporting this child busy; \
             got {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, .. }) if reason == "error"
            ),
            "…and the unwind must close it; got {events:?}"
        );
    }

    /// The guard closes the bracket from `Drop`, which is the only thing that
    /// covers the exits no run-level test can reach: the two setup `?`s that
    /// fail for no caller-controllable input, a panic unwinding through the run,
    /// and the future being dropped outright (a `tokio` task abort — which no
    /// cancellation-token probe can observe, because the token was never
    /// cancelled).
    ///
    /// Asserted here rather than inferred from the shape of the code, because
    /// "there is a `Drop` impl" and "the `Drop` impl publishes the terminal an
    /// observer is waiting for" are different claims.
    #[test]
    fn an_unclosed_bracket_still_publishes_a_terminal_when_dropped() {
        use crate::session_events::{self, SessionBusEvent};
        let mut rx = session_events::subscribe("dropped-bracket");
        {
            let _bracket = BusTurnBracket::open("dropped-bracket".into(), "turn-9".into(), None);
        }
        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.first(),
                Some(SessionBusEvent::TurnStarted { turn_id }) if turn_id == "turn-9"
            ),
            "opening the bracket publishes TurnStarted; got {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, token_state: None }) if reason == "error"
            ),
            "dropping an unclosed bracket must publish a terminal; got {events:?}"
        );
    }

    /// …and a bracket dropped while the run's token is tripped closes as
    /// `cancelled`. Same reasoning as the run-level test: a stop is not a
    /// failure, and this is the branch a panic or task abort under cancellation
    /// takes.
    #[test]
    fn a_dropped_bracket_reports_cancelled_when_the_run_token_was_tripped() {
        use crate::session_events::{self, SessionBusEvent};
        let mut rx = session_events::subscribe("dropped-bracket-cancelled");
        let token = CancellationToken::new();
        token.cancel();
        drop(BusTurnBracket::open(
            "dropped-bracket-cancelled".into(),
            "turn-10".into(),
            Some(token),
        ));
        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, .. }) if reason == "cancelled"
            ),
            "got {events:?}"
        );
    }

    /// Closing explicitly publishes exactly one terminal — the subsequent `Drop`
    /// must not publish a second. A double terminal is a wire-contract
    /// violation: `workspace_watch` and `wait:"final_message"` both treat the
    /// first one as the end of the turn.
    #[test]
    fn closing_a_bracket_disarms_its_drop() {
        use crate::session_events::{self, SessionBusEvent};
        let mut rx = session_events::subscribe("closed-bracket");
        BusTurnBracket::open("closed-bracket".into(), "turn-11".into(), None).close("stop");
        let events: Vec<SessionBusEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, SessionBusEvent::TurnFinished { .. }))
                .count(),
            1,
            "exactly one terminal; got {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(SessionBusEvent::TurnFinished { reason, .. }) if reason == "stop"
            ),
            "and it carries the reason the caller chose; got {events:?}"
        );
    }
}
