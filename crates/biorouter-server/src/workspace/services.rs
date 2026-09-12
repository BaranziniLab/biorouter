//! The daemon's `WorkspaceServices` implementation over `AppState` (BR-71).
//! The GUI methods are backed by the `WorkspaceBridge` registry (Slice 2), which
//! `routes::workspace`'s `/ui/workspace` socket populates; with no window
//! attached they degrade to the headless answers.

use std::path::PathBuf;
use std::sync::Arc;

use biorouter::config::{get_enabled_extensions, get_extension_by_name};
use biorouter::conversation::message::Message;
use biorouter::session::session_manager::SessionType;
// `ExtensionState` is imported for its PROVIDED METHOD `to_extension_data`,
// called on `extensions_state` in `start_session`. A trait method is only
// callable with the trait in scope, so importing `EnabledExtensionsState` alone
// is E0599. `routes/agent.rs` imports it exactly this way.
use biorouter::session::extension_data::ExtensionState;
use biorouter::session::EnabledExtensionsState;
// `WorkspaceTurnLease` is in this list because `begin_turn` names it in its
// return type AND constructs `ServerTurnLease` below — importing only
// `WorkspaceServices` is an E0412 the moment `begin_turn` is written.
//
// `KbPrimaryChoice` and `KbSelectionView` are here for exactly the same reason:
// `KbPrimaryChoice` in `start_session` and `set_knowledge_bases`,
// `KbSelectionView` in `set_knowledge_bases`'s return type, its constructed
// value, and `knowledge_selection`.
use biorouter::workspace_services::{
    KbPrimaryChoice, KbSelectionView, WorkspaceServices, WorkspaceTurnLease,
};

use crate::state::AppState;

/// `WorkspaceServices` answers with a bare `String`, so the machine-readable
/// half of a refusal has to travel inside the prose. These are the codes
/// `POST /agent/resume` and the turn runner publish for the same two
/// conditions; an interface branches on the substring rather than on wording it
/// would have to keep in step.
const SUBAGENT_PROFILE_MISSING: &str = "subagent_runtime_profile_missing";
const SUBAGENT_PROFILE_RESTORE_FAILED: &str = "subagent_runtime_profile_restore_failed";

pub struct ServerWorkspaceServices {
    state: Arc<AppState>,
}

impl ServerWorkspaceServices {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Hydrate a session's extensions for a turn this crate is about to run,
    /// **reusing the eager load if one is already in flight**.
    ///
    /// `start_session` spawns a full `load_extensions_from_session` and
    /// registers the handle, exactly as `POST /agent/start` does. Loading again
    /// unconditionally means two concurrent loads on one agent: each spawns the
    /// session's stdio MCP subprocesses, and `add_extension_with_origin`'s
    /// double-check under the map lock drops the losing racer's — spawned,
    /// unreferenced, never reaped. `POST /agent/reply` has always avoided this
    /// by taking the pending results instead (`routes/agent.rs`); this is the
    /// same move, and `workspace_open { new: { prompt } }` — which chains
    /// `start_session` straight into a detached turn — is the caller that made
    /// it necessary here.
    async fn hydrate_extensions(
        &self,
        agent: &Arc<biorouter::agents::Agent>,
        session: &biorouter::session::Session,
    ) -> Vec<biorouter::agents::ExtensionLoadResult> {
        if let Some(results) = self.state.take_extension_loading_task(&session.id).await {
            self.state.remove_extension_loading_task(&session.id).await;
            return results;
        }
        agent.load_extensions_from_session(session).await
    }

    /// Drop a child whose delegated runtime profile could not be restored.
    ///
    /// `restore_subagent_runtime_profile` installs the profile's grants one at
    /// a time, so a failure part-way leaves a half-built agent cached under the
    /// session id — reachable, and already holding whichever extensions landed
    /// before the error. Without this the next injected turn would take that
    /// agent as-is. `POST /agent/resume` evicts on both of its refusal paths
    /// for the same reason.
    async fn evict_partially_restored_child(&self, session_id: &str) {
        let _ = self.state.agent_manager.remove_session(session_id).await;
    }
}

/// Publish this daemon's platform services to the `biorouter` crate, so the
/// workspace extension's tools can reach the turn lock, the detached turn
/// runner and (Slice 2) the GUI bridge. Without it those tools degrade to their
/// headless behaviour *inside the daemon*, which is silent: every one of them
/// still answers, just about a session-level world.
///
/// A named function rather than three lines inlined in `commands/agent.rs`,
/// because a bootstrap step that only exists at a call site is a bootstrap step
/// nothing can test. `biorouter::workspace_services::install` writes a
/// `OnceLock`, so this is first-call-wins and later calls are no-ops.
pub fn install_workspace_services(state: Arc<AppState>) {
    biorouter::workspace_services::install(Arc::new(ServerWorkspaceServices::new(state)));
}

#[async_trait::async_trait]
impl WorkspaceServices for ServerWorkspaceServices {
    fn gui_attached(&self) -> bool {
        crate::workspace::bridge::any_attached()
    }

    fn layout_snapshot(&self) -> Option<serde_json::Value> {
        crate::workspace::bridge::merged_layout()
    }

    fn is_turn_active(&self, session_id: &str) -> bool {
        self.state.is_turn_active(session_id)
    }

    fn cancel_turn(&self, session_id: &str) -> Option<String> {
        self.state.cancel_turn(session_id)
    }

    fn abandon_pending_continuations(&self, session_id: &str) -> bool {
        self.state
            .abandon_pending_continuations_for_session(session_id)
    }

    fn begin_turn(
        &self,
        session_id: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Box<dyn WorkspaceTurnLease>, String> {
        let pump_cancel = cancel.clone();
        let guard = self
            .state
            .try_begin_turn_idempotent(session_id, cancel, None)
            .map_err(|conflict| {
                format!(
                    "a turn is already in flight for this session (running turn {})",
                    conflict.running_turn_id
                )
            })?;
        let stream = guard.stream();
        let bus = biorouter::session_events::subscribe(session_id);
        let writer = stream
            .claim_writer()
            .expect("a newly acquired workspace turn owns its stream");
        tokio::spawn(crate::routes::reply::pump_bus_into_stream(
            Arc::clone(&self.state),
            session_id.to_string(),
            bus,
            writer,
            pump_cancel,
            crate::routes::reply::sse_coalesce_window(),
        ));
        Ok(Box::new(ServerTurnLease { guard }))
    }

    async fn stop_agent(&self, session_id: &str) -> Result<(), String> {
        let _stop_guard = self.state.begin_agent_stop(session_id);
        self.state
            .abandon_pending_continuations_for_session(session_id);
        self.state
            .agent_manager
            .remove_session(session_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn start_detached_turn(
        &self,
        session_id: &str,
        message: Message,
    ) -> Result<String, String> {
        let request = super::turn::TurnRequest::new(session_id.to_string(), message);
        let cancel = tokio_util::sync::CancellationToken::new();
        let turn_guard = self
            .state
            .try_begin_turn_idempotent(
                session_id,
                cancel.clone(),
                request.extras.idempotency_key.clone(),
            )
            .map_err(|conflict| {
                format!(
                    "a turn is already in flight for this session (running turn {})",
                    conflict.running_turn_id
                )
            })?;
        let turn_id = turn_guard.turn_id().to_string();

        // Hydration happens only after the turn lock is ours. Besides making a
        // rejected follow-up side-effect free, this keeps a live delegated
        // child's exact capability profile from being replaced by the generic
        // extension snapshot used by ordinary sessions.
        let session = self
            .state
            .session_manager()
            .get_session(session_id, false)
            .await
            .map_err(|e| e.to_string())?;
        let agent = self
            .state
            .get_agent(session_id.to_string())
            .await
            .map_err(|e| e.to_string())?;

        if session.session_type == SessionType::SubAgent {
            agent
                .restore_persisted_provider_if_missing(&session)
                .await
                .map_err(|e| e.to_string())?;
            // A child with no runtime profile has no daemon-authored grant set,
            // and its legacy `EnabledExtensionsState` snapshot is not a stand-in
            // for one: that snapshot can name `workspace` with an empty
            // `available_tools`, which means EVERY tool, and
            // `load_extensions_from_session` would install it as an Explicit
            // entry OVER the four-tool injection delegation actually granted.
            // The child could then `workspace_open { new: { prompt } }` itself a
            // fresh User session carrying the machine's whole default extension
            // set — an escape from the delegated grant, out of a row the parent
            // never authored.
            let restored = match agent.restore_subagent_runtime_profile(&session).await {
                Ok(restored) => restored,
                Err(e) => {
                    self.evict_partially_restored_child(session_id).await;
                    return Err(format!(
                        "{SUBAGENT_PROFILE_RESTORE_FAILED}: this subagent's delegated runtime \
                         profile could not be restored ({e}). Ask the parent conversation to \
                         delegate the work again."
                    ));
                }
            };
            if !restored {
                self.evict_partially_restored_child(session_id).await;
                return Err(format!(
                    "{SUBAGENT_PROFILE_MISSING}: this subagent has no delegated runtime profile, \
                     so Biorouter cannot tell which tools it was granted and will not guess from \
                     its saved extension list. Ask the parent conversation to delegate the work \
                     again."
                ));
            }
        } else {
            let (provider_result, _extension_results) = tokio::join!(
                agent.restore_provider_from_session(&session),
                self.hydrate_extensions(&agent, &session),
            );
            provider_result.map_err(|e| e.to_string())?;
        }

        // A detached turn still owns a replay stream. The bus subscription is
        // opened before the runner and pumped independently of any tab, so a
        // closed child keeps running and `/agent/resume` can reattach later.
        let stream = turn_guard.stream();
        let bus = biorouter::session_events::subscribe(session_id);
        let writer = stream
            .claim_writer()
            .expect("a newly acquired detached turn owns its stream");
        tokio::spawn(crate::routes::reply::pump_bus_into_stream(
            Arc::clone(&self.state),
            session_id.to_string(),
            bus,
            writer,
            cancel.clone(),
            crate::routes::reply::sse_coalesce_window(),
        ));
        stream.spawn_orphan_reaper(cancel.clone(), crate::turn_stream::orphan_timeout());
        tokio::spawn(super::turn::run_turn(
            Arc::clone(&self.state),
            request,
            turn_guard,
            cancel,
        ));

        Ok(turn_id)
    }

    async fn start_session(
        &self,
        working_dir: PathBuf,
        extensions: Option<Vec<String>>,
        knowledge_bases: Vec<String>,
        primary: KbPrimaryChoice,
    ) -> Result<String, String> {
        // The minimal core of POST /agent/start (`start_agent`):
        // create → apply extension set → persist → eager-load in background.
        let configs = match extensions {
            None => get_enabled_extensions(),
            Some(names) => {
                let mut configs = Vec::with_capacity(names.len());
                for name in &names {
                    match get_extension_by_name(name) {
                        Some(c) => configs.push(c),
                        None => return Err(format!("unknown extension '{name}'")),
                    }
                }
                configs
            }
        };

        let manager = self.state.session_manager();
        let session = manager
            .create_session(
                working_dir,
                biorouter::session::DEFAULT_SESSION_NAME.to_string(),
                SessionType::User,
            )
            .await
            .map_err(|e| format!("failed to create session: {e}"))?;

        let mut extension_data = session.extension_data.clone();
        let extensions_state = EnabledExtensionsState::new(configs);
        extensions_state
            .to_extension_data(&mut extension_data)
            .map_err(|e| format!("failed to initialize extensions: {e}"))?;
        manager
            .update(&session.id)
            .extension_data(extension_data)
            .apply()
            .await
            .map_err(|e| format!("failed to save extension state: {e}"))?;

        if !knowledge_bases.is_empty() {
            // A fresh session has no primary of its own, so `Auto` here always
            // resolves to "pin the first id" — which is what makes the new
            // session's KB-less writes work at all. Passing an explicit
            // `Set(id)` from `workspace_open` is honoured unchanged.
            self.set_knowledge_bases(&session.id, &knowledge_bases, primary)?;
        }

        // Eager extension load, exactly as start_agent does.
        let state = self.state.clone();
        let session_for_spawn = manager
            .get_session(&session.id, false)
            .await
            .map_err(|e| e.to_string())?;
        let sid = session.id.clone();
        let task = tokio::spawn(async move {
            match state.get_agent(session_for_spawn.id.clone()).await {
                Ok(agent) => agent.load_extensions_from_session(&session_for_spawn).await,
                Err(e) => {
                    tracing::warn!("workspace start_session: agent create failed: {e}");
                    vec![]
                }
            }
        });
        self.state.set_extension_loading_task(sid, task).await;

        Ok(session.id)
    }

    fn set_knowledge_bases(
        &self,
        session_id: &str,
        kbs: &[String],
        primary: KbPrimaryChoice,
    ) -> Result<KbSelectionView, String> {
        // Same path `state.rs` already uses for `KnowledgeService` —
        // `biorouter-server` depends on `biorouter-mcp` directly.
        use biorouter_mcp::knowledge::service::PrimaryUpdate;

        // THE ONE SEAM where the subtractive model and the borrowed
        // `PrimaryUpdate` are visible. `set_visible_kbs` takes the ids that
        // should be VISIBLE and inverts them into the stored hidden list itself,
        // under the root lock — which is exactly why we call it rather than
        // reading the hidden list, editing it and writing it back: that
        // read-modify-write across two unlocked calls is a documented race where
        // two surfaces each write a list computed before the other's edit and
        // one silently disappears.
        //
        // Resolve `Auto` HERE, not in the tools, so every surface gets the same
        // answer. `Unchanged` is safe for the keep-it case because the service
        // re-validates membership against the resulting set and repairs the
        // pointer itself when the old target has just been hidden.
        let current = self
            .state
            .knowledge_service
            .primary_for_session(Some(session_id))
            .unwrap_or_default();
        let auto_pick: Option<String> = match &current {
            Some(id) if kbs.iter().any(|k| k == id) => None, // keep it
            _ => kbs.first().cloned(),
        };
        let update = match &primary {
            KbPrimaryChoice::Set(id) => PrimaryUpdate::Set(id.as_str()),
            KbPrimaryChoice::Clear => PrimaryUpdate::Clear,
            KbPrimaryChoice::Auto => match auto_pick.as_deref() {
                Some(id) => PrimaryUpdate::Set(id),
                // Either the current primary is still a member (keep it), or the
                // new set is empty and there is nothing legal to point at.
                None if current.is_some() && !kbs.is_empty() => PrimaryUpdate::Unchanged,
                None => PrimaryUpdate::Clear,
            },
        };

        let selection = self
            .state
            .knowledge_service
            .set_visible_kbs(Some(session_id), kbs, update)
            .map_err(|e| e.to_string())?;
        Ok(KbSelectionView {
            kb_ids: selection.kb_ids,
            primary_kb: selection.primary_kb,
        })
    }

    fn installed_knowledge_bases(&self) -> Option<Vec<String>> {
        // The same universe `set_visible_kbs` resolves visibility against
        // (`installed_kb_ids_unlocked` is `list_bases` ids), read without the
        // root lock: this answers a pre-flight question, and the setter still
        // re-decides under the lock. A registry that cannot be read answers
        // `None`, so a transient failure never refuses a real base.
        self.state
            .knowledge_service
            .list_bases()
            .ok()
            .map(|bases| bases.into_iter().map(|base| base.id).collect())
    }

    fn knowledge_selection(&self, session_id: &str) -> KbSelectionView {
        // `KnowledgeService::selection` takes the root lock and returns set +
        // pointer TOGETHER. Do not rebuild this from `session_kb_ids` +
        // `primary_for_session`: composing two unlocked reads is what made the
        // "primary is a member of the set" claim false whenever a writer landed
        // between them.
        //
        // Best-effort — a read error reports "no KBs", never fails a list.
        self.state
            .knowledge_service
            .selection(Some(session_id))
            .map(|s| KbSelectionView {
                kb_ids: s.kb_ids,
                primary_kb: s.primary_kb,
            })
            .unwrap_or_default()
    }

    /// Route to the window holding `near_session`, or fall back to the guess.
    ///
    /// ⚠ The fallback is not laziness (issue #78). A parent legitimately has no
    /// tab anywhere — a headless spawn, a tab closed mid-turn, or an echo not
    /// yet delivered through its 300 ms debounce — and the fire-and-forget
    /// contract requires that a spawn never break because a window could not be
    /// located. Falling back to today's behaviour is strictly better than
    /// dropping the frame.
    ///
    /// ⚠ **"Could not be located" includes "was located and then went away."**
    /// This used to resolve one bridge and emit into it, so a window that
    /// detached between the two — a reload, a close, the user quitting a
    /// window mid-spawn — produced an `Err` that both callers in
    /// `subagent_tool.rs` discard, and the frame was silently lost even with
    /// another window attached and able to take it. Resolution failure and
    /// delivery failure are the same event to this contract, so they take the
    /// same fallback; see [`emit_with_fallback`].
    async fn gui_command_near(
        &self,
        frame: serde_json::Value,
        wait_result: bool,
        near_session: &str,
    ) -> Result<serde_json::Value, String> {
        emit_with_fallback(
            crate::workspace::bridge::bridge_for_session(near_session),
            crate::workspace::bridge::focused_or_recent,
            frame,
            wait_result.then_some(GUI_ROUND_TRIP_TIMEOUT),
        )
        .await
    }

    async fn gui_command(
        &self,
        frame: serde_json::Value,
        wait_result: bool,
    ) -> Result<serde_json::Value, String> {
        let bridge = crate::workspace::bridge::focused_or_recent().ok_or("no GUI attached")?;
        emit_on(
            &bridge,
            frame,
            wait_result.then_some(GUI_ROUND_TRIP_TIMEOUT),
        )
        .await
    }
}

/// How long a `wait_result` frame parks for the renderer's `workspace_result`.
const GUI_ROUND_TRIP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Put one frame on one window. `Some(timeout)` is the caller's `wait_result`:
/// park for the renderer's `workspace_result` that long. It is a parameter
/// rather than the constant so a test can prove the fallback does not park
/// twice without taking twenty seconds to do it.
async fn emit_on(
    bridge: &crate::workspace::bridge::WorkspaceBridge,
    frame: serde_json::Value,
    wait: Option<std::time::Duration>,
) -> Result<serde_json::Value, String> {
    match wait {
        Some(timeout) => bridge.emit_and_wait(frame, timeout).await,
        None => bridge
            .emit(frame)
            .map(|()| serde_json::json!({ "sent": true })),
    }
}

/// Deliver `frame` to `primary`, and on **any** failure — including one that
/// only shows up at `emit` time — try once more on whatever `fallback` resolves
/// to *then*.
///
/// Three details are load-bearing:
///
/// * `fallback` is a closure, not a value, so the registry is re-read **after**
///   the first attempt failed. That is the whole point: the window that failed
///   has by then dropped out of `focused_or_recent`'s `is_attached` filter, and
///   a window that attached in the meantime is in.
/// * The retry is skipped when the guess lands on the *same* window that just
///   failed. Nothing is gained by re-emitting into a channel that just refused,
///   and for a `wait_result` frame it would mean parking for a second full
///   [`GUI_ROUND_TRIP_TIMEOUT`] on a window already known to be unresponsive.
/// * The error reported is the **first** one, not the fallback's. "the parent's
///   window is gone" is the diagnosis; "no GUI attached" is a consequence.
///
/// Split from [`ServerWorkspaceServices::gui_command_near`] for the reason
/// `pick_target` is split from `focused_or_recent` in `bridge.rs`: `BRIDGES` is
/// a process-wide static, so a test that asserts *which* window a frame landed
/// on is only containment-safe against a supplied candidate.
async fn emit_with_fallback<F>(
    primary: Option<crate::workspace::bridge::WorkspaceBridge>,
    fallback: F,
    frame: serde_json::Value,
    wait: Option<std::time::Duration>,
) -> Result<serde_json::Value, String>
where
    F: FnOnce() -> Option<crate::workspace::bridge::WorkspaceBridge>,
{
    let mut first_error: Option<String> = None;
    if let Some(bridge) = primary.as_ref() {
        match emit_on(bridge, frame.clone(), wait).await {
            Ok(value) => return Ok(value),
            Err(e) => first_error = Some(e),
        }
    }

    let second = fallback().filter(|second| {
        !primary
            .as_ref()
            .is_some_and(|first| first.same_window(second))
    });

    match second {
        Some(second) => emit_on(&second, frame, wait)
            .await
            .map_err(|e| first_error.unwrap_or(e)),
        None => Err(first_error.unwrap_or_else(|| "no GUI attached".to_string())),
    }
}

/// The daemon's lease: a wrapped `TurnGuard`. Dropping it releases the session's
/// turn slot exactly as the /reply task's guard does.
struct ServerTurnLease {
    guard: crate::state::TurnGuard,
}

impl biorouter::workspace_services::WorkspaceTurnLease for ServerTurnLease {
    fn turn_id(&self) -> &str {
        self.guard.turn_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::model::ModelConfig;
    use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
    use biorouter::providers::errors::ProviderError;
    use biorouter::session::session_manager::SessionType;
    use biorouter::workspace_services::WorkspaceServices;
    use rmcp::model::Tool;

    /// The shape a pre-runtime-profile child row carries. An empty
    /// `available_tools` means EVERY tool of that extension
    /// (`extension_manager.rs`), so this one value is a full grant of workspace
    /// control — including `workspace_open { new: { prompt } }`, which mints a
    /// User session holding the machine's default extension set. It is
    /// `workspace` and not some innocuous extension on purpose: with `todo`
    /// here, a "no `workspace__` tools" assertion passes against code that
    /// restores the snapshot verbatim.
    fn broad_workspace_snapshot() -> biorouter::agents::ExtensionConfig {
        biorouter::agents::ExtensionConfig::Platform {
            name: "workspace".into(),
            description: "Legacy broad workspace snapshot".into(),
            bundled: Some(true),
            available_tools: Vec::new(),
        }
    }

    async fn seed_extension_data(
        state: &Arc<crate::state::AppState>,
        name: &str,
        session_type: SessionType,
        seed: impl FnOnce(&mut biorouter::session::ExtensionData),
    ) -> (tempfile::TempDir, String) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut session = state
            .session_manager()
            .create_session(temp.path().to_path_buf(), name.to_string(), session_type)
            .await
            .unwrap();
        seed(&mut session.extension_data);
        state
            .session_manager()
            .update(&session.id)
            .extension_data(session.extension_data)
            .apply()
            .await
            .unwrap();
        (temp, session.id)
    }

    /// The minimum daemon-authored profile a child needs to be allowed a turn:
    /// a prompt and no grants at all.
    async fn seed_runtime_profile(
        state: &Arc<crate::state::AppState>,
        session_id: &str,
        prompt: &str,
    ) {
        let mut extension_data = state
            .session_manager()
            .get_session(session_id, false)
            .await
            .unwrap()
            .extension_data;
        extension_data.set_extension_state(
            "subagent_runtime_profile",
            "v2",
            serde_json::json!({ "format_version": 2, "system_prompt": prompt }),
        );
        state
            .session_manager()
            .update(session_id)
            .extension_data(extension_data)
            .apply()
            .await
            .unwrap();
    }

    async fn tool_names(agent: &Arc<biorouter::agents::Agent>, session_id: &str) -> Vec<String> {
        agent
            .list_tools(session_id, None)
            .await
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    struct NeverCompletesProvider;

    #[async_trait::async_trait]
    impl Provider for NeverCompletesProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "workspace-services-never-completes"
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("workspace-services-test")
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            std::future::pending().await
        }
    }

    /// NOTE — what these tests share with the rest of this crate's unit tests,
    /// and what they deliberately do not.
    ///
    /// * `AppState::new()` opens the **ONE shared session database** (through
    ///   `AgentManager::instance()` → `SessionManager::instance()`, both
    ///   process-global `OnceCell`s with no path seam). `workspace/turn.rs`,
    ///   `routes/session.rs`, `routes/session_events.rs` and `state.rs` all
    ///   carry the same warning: every test in the binary creates rows in the
    ///   same store. Keep session names unique and never assert on row counts.
    ///
    ///   It is **not** the developer's own history. This bullet used to argue
    ///   relocating it was impossible — "`BIOROUTER_PATH_ROOT` being set before
    ///   the `LazyLock` resolves `Paths::data_dir()` … which libtest's parallel
    ///   scheduling does not let a test decide" — which is right about a
    ///   `#[test]` and wrong about a constructor. `src/test_sandbox.rs`'s
    ///   `#[ctor]` runs before `main`, so it cannot lose that race: it points
    ///   the data root at a throwaway directory and freezes the store there.
    ///   Leaving the claim standing cost a Windows CI flake; read that file's
    ///   pinning section before touching anything that relocates the path root.
    ///   Fixing that is a crate-wide change (a path seam on `SessionManager`),
    ///   not a Task 9 one.
    /// * **Extensions are always passed explicitly**, never `None`. `None`
    ///   means "the developer's own enabled extension set", and loading it
    ///   walks `merge_environments` → `Config::get_secret` → the macOS
    ///   Keychain, which on an unsigned test binary blocks on an interactive
    ///   authorization dialog no test run can answer. That is not a
    ///   hypothetical: `cargo test -p biorouter-server --lib workspace::services`
    ///   hung there indefinitely (`SecKeychainFindGenericPassword` under
    ///   `Agent::load_extensions_from_session`) until this rule was applied.
    ///   `Some(vec![])` exercises the same code path with an empty set.

    #[tokio::test]
    async fn start_session_creates_a_user_session_in_the_requested_dir_and_rejects_unknown_extensions(
    ) {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let temp = tempfile::TempDir::new().unwrap();

        let err = services
            .start_session(
                temp.path().to_path_buf(),
                Some(vec!["no-such-ext".into()]),
                Vec::new(),
                KbPrimaryChoice::Auto,
            )
            .await
            .unwrap_err();
        assert!(err.contains("no-such-ext"));

        let sid = services
            .start_session(
                temp.path().to_path_buf(),
                Some(Vec::new()),
                Vec::new(),
                KbPrimaryChoice::Auto,
            )
            .await
            .unwrap();
        let session = state
            .session_manager()
            .get_session(&sid, false)
            .await
            .unwrap();
        assert_eq!(session.session_type, SessionType::User);
        // #44 conformance: the REQUESTED working directory is the session's,
        // set at creation. Without this assertion an implementation that
        // ignored `working_dir` entirely — and started every workspace session
        // in the daemon's cwd — passed the gate.
        assert_eq!(
            session.working_dir.canonicalize().unwrap(),
            temp.path().canonicalize().unwrap()
        );
        // The extension set the caller asked for is what was persisted.
        let persisted = biorouter::session::EnabledExtensionsState::from_extension_data(
            &session.extension_data,
        )
        .expect("start_session persists an extension state");
        assert!(
            persisted.extensions.is_empty(),
            "asked for no extensions, got {:?}",
            persisted
                .extensions
                .iter()
                .map(|e| e.name().to_string())
                .collect::<Vec<_>>()
        );
    }

    /// `start_session` kicks off an EAGER extension load in a background task
    /// and registers it, exactly as `POST /agent/start` does. A detached turn on
    /// that same fresh session — which is what `workspace_open { new: { prompt
    /// } }` does microseconds later, the first caller to chain the two — must
    /// consume that pending load, not start a second concurrent one.
    ///
    /// `POST /agent/reply` already does precisely this (`take_extension_loading_task`
    /// then *reuse*, `routes/agent.rs`), and for the reason that applies here
    /// too: two concurrent `load_extensions_from_session` calls on one agent
    /// each spawn the session's stdio MCP subprocesses, and
    /// `add_extension_with_origin`'s double-check under the map lock means the
    /// losing racer's children are simply dropped — spawned, unreferenced, and
    /// never reaped.
    #[tokio::test]
    async fn a_detached_turn_reuses_the_eager_extension_load_instead_of_racing_it() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let temp = tempfile::TempDir::new().unwrap();

        // There IS one to reuse — without this the assertion below would pass
        // against a `start_session` that never registered anything.
        let untouched = services
            .start_session(
                temp.path().to_path_buf(),
                Some(Vec::new()),
                Vec::new(),
                KbPrimaryChoice::Auto,
            )
            .await
            .unwrap();
        assert!(
            state
                .take_extension_loading_task(&untouched)
                .await
                .is_some(),
            "start_session registers an eager extension load"
        );

        let sid = services
            .start_session(
                temp.path().to_path_buf(),
                Some(Vec::new()),
                Vec::new(),
                KbPrimaryChoice::Auto,
            )
            .await
            .unwrap();
        let session = state
            .session_manager()
            .get_session(&sid, false)
            .await
            .unwrap();
        let agent = state.get_agent(sid.clone()).await.unwrap();

        // The hydration a detached turn performs, without running the turn:
        // `start_detached_turn` would go on to restore a provider and call the
        // real turn runner, and in a developer's own environment that provider
        // resolves — a test must not fire a live turn to check a load.
        services.hydrate_extensions(&agent, &session).await;

        assert!(
            state.take_extension_loading_task(&sid).await.is_none(),
            "the detached turn must consume the eager load, not leave it running \
             beside a second one of its own"
        );
    }

    #[tokio::test]
    async fn begin_turn_lease_is_attachable_and_cancel_turn_trips_its_token() {
        use tokio_util::sync::CancellationToken;
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());

        let token = CancellationToken::new();
        let lease = services
            .begin_turn("lease-s1", token.clone())
            .expect("lock acquired");
        assert!(lease.turn_id().starts_with("turn-"));
        assert!(services.is_turn_active("lease-s1"));
        assert_eq!(
            state.active_turn_id("lease-s1").as_deref(),
            Some(lease.turn_id()),
            "a delegated turn must be discoverable by /agent/resume"
        );

        let stream = match state.try_begin_turn_idempotent(
            "lease-s1",
            CancellationToken::new(),
            Some("attach-probe".into()),
        ) {
            Ok(_) => panic!("the live workspace turn must retain the session lock"),
            Err(conflict) => conflict.stream,
        };
        assert!(
            stream.has_writer(),
            "resume must never advertise a turn whose replay stream has no owner"
        );
        let mut reader = stream.attach(0);
        biorouter::session_events::publish(
            "lease-s1",
            biorouter::session_events::SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        );
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(10), reader.recv())
            .await
            .expect("the workspace bus pump must deliver a terminal frame");
        let terminal = match terminal {
            crate::turn_stream::ReaderEvent::Frame(frame, _) => frame.live_sse(),
            other => panic!("expected a terminal frame from the workspace bus, got {other:?}"),
        };
        assert!(terminal.contains("\"type\":\"Finish\""), "got: {terminal}");
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(10), reader.recv())
                .await
                .expect("the workspace bus pump must close after its terminal frame"),
            crate::turn_stream::ReaderEvent::Closed
        ));

        // A second begin_turn conflicts — the one-turn-per-session invariant.
        // Matched rather than `unwrap_err()`: the Ok arm is a
        // `Box<dyn WorkspaceTurnLease>`, which is not `Debug`, and making the
        // trait `Debug` purely to satisfy a test assertion would put a bound on
        // every future implementor for no production reason.
        let conflict = match services.begin_turn("lease-s1", CancellationToken::new()) {
            Ok(_) => panic!("a second begin_turn for a busy session must be refused"),
            Err(e) => e,
        };
        assert!(conflict.contains("already in flight"));

        // cancel_turn reaches the lease's token — this is what makes the tab's
        // Stop / workspace_close scope:"turn" work on a subagent run (Task 33).
        assert!(services.cancel_turn("lease-s1").is_some());
        assert!(token.is_cancelled());

        // Dropping the lease frees the session.
        drop(lease);
        assert!(!services.is_turn_active("lease-s1"));
    }

    /// A turn injected into a session with NO live agent must run against that
    /// session's persisted provider, not against the bare agent `get_agent`
    /// mints for a session nobody has opened this run — the population
    /// `workspace_send_prompt mode:"turn"` exists to reach.
    ///
    /// # How this test can fail, and why the obvious version cannot
    ///
    /// `start_turn` **spawns** the turn and returns `Ok(turn_id)` immediately
    /// (`turn.rs`), so a turn that dies on its first step with "Provider not
    /// set" is not visible in `start_detached_turn`'s return value at all. The
    /// first version of this test asserted `!err.contains("Provider not set")`
    /// only `if let Some(err)` — with the hydration deleted there is no error,
    /// so the assertion was skipped and the test passed on a missing feature.
    ///
    /// This version pins a provider that CANNOT be constructed and requires the
    /// resulting error. That error can only come from
    /// `restore_provider_from_session`, so it is positive evidence that the
    /// session row was read and its provider restored *before* the turn was
    /// started. Delete the hydration and this test fails with "expected the
    /// cold session's persisted provider to be restored".
    #[tokio::test]
    async fn a_turn_injected_into_a_cold_session_hydrates_it_from_its_session_row() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let temp = tempfile::TempDir::new().unwrap();

        // Cold BY CONSTRUCTION. An earlier version built the session through
        // `start_session` and then evicted the agent its eager load had created
        // — a race with a spawned task whose result had to be discarded to keep
        // the test green, which is not proof of eviction and not proof of a cold
        // session. Never entering the registry is the same state, reached
        // deterministically.
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "br71 workspace cold hydration".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert!(
            !state.agent_manager.has_session(&session.id).await,
            "the session must start with no cached agent for this test to mean anything"
        );

        state
            .session_manager()
            .update(&session.id)
            .provider_name("br71-no-such-provider")
            .model_config(biorouter::model::ModelConfig::new("br71-no-such-model").unwrap())
            .apply()
            .await
            .unwrap();

        let err = services
            .start_detached_turn(&session.id, Message::user().with_text("hello"))
            .await
            .unwrap_err();
        assert!(
            err.contains("br71-no-such-provider"),
            "expected the cold session's persisted provider to be restored before the turn \
             started, got: {err}"
        );
        // And the failure happened before any turn was started, so nothing
        // acquired the session's turn slot and leaked it.
        assert!(!state.is_turn_active(&session.id));
    }

    #[tokio::test]
    async fn a_detached_turn_is_immediately_attachable_through_its_replay_stream() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "br71 attachable detached turn".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        // A child takes a turn only with its delegated runtime profile in
        // hand; without one `start_detached_turn` refuses before it ever
        // reaches the stream this test is about.
        seed_runtime_profile(&state, &session.id, "BR71 ATTACHABLE CHILD").await;
        let agent = state.get_agent(session.id.clone()).await.unwrap();
        agent
            .update_provider(Arc::new(NeverCompletesProvider), &session.id)
            .await
            .unwrap();

        let turn_id = services
            .start_detached_turn(&session.id, Message::user().with_text("hello"))
            .await
            .unwrap();
        assert_eq!(
            state.active_turn_id(&session.id).as_deref(),
            Some(turn_id.as_str())
        );
        let conflict = state
            .try_begin_turn_idempotent(
                &session.id,
                tokio_util::sync::CancellationToken::new(),
                None,
            )
            .unwrap_err();
        assert!(conflict.stream.has_live_writer());
        state.cancel_turn(&session.id);
    }

    #[tokio::test]
    async fn a_rejected_busy_child_followup_does_not_hydrate_or_mutate_the_child() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let temp = tempfile::TempDir::new().unwrap();
        let child = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "br71 busy cold child".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        assert!(!state.agent_manager.has_session(&child.id).await);
        let _guard = state
            .try_begin_turn_idempotent(&child.id, tokio_util::sync::CancellationToken::new(), None)
            .unwrap();

        let error = services
            .start_detached_turn(&child.id, Message::user().with_text("steer"))
            .await
            .unwrap_err();

        assert!(error.contains("already in flight"), "{error}");
        assert!(
            !state.agent_manager.has_session(&child.id).await,
            "a rejected follow-up must not create or hydrate the child agent"
        );
    }

    /// The injected half of the delegation gate: `workspace_send_prompt
    /// mode:"turn"` reaches a cold child through here, and a child with no
    /// daemon-authored runtime profile must be refused rather than rebuilt from
    /// its saved extension list.
    ///
    /// That list is the escalation vector, not a fallback: nothing that wrote
    /// it was a delegation decision, `load_extensions_from_session` applies none
    /// of the clamps `restore_subagent_runtime_profile` applies, and it
    /// deliberately replaces an auto-injected entry with an Explicit one — so
    /// the four-tool `workspace` injection a subagent is granted becomes the
    /// whole extension, and the child can `workspace_open { new: { prompt } }`
    /// itself a User session with the machine's default extension set.
    #[tokio::test]
    async fn an_injected_turn_on_a_child_with_no_runtime_profile_is_refused_not_hydrated() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let (_temp, child_id) = seed_extension_data(
            &state,
            "br71 injected child no profile",
            SessionType::SubAgent,
            |extension_data| {
                biorouter::session::EnabledExtensionsState::new(vec![broad_workspace_snapshot()])
                    .to_extension_data(extension_data)
                    .unwrap();
            },
        )
        .await;
        // Held across the call so the tools can be read after the refusal has
        // evicted the agent from the manager.
        let agent = state.get_agent(child_id.clone()).await.unwrap();

        let error = services
            .start_detached_turn(&child_id, Message::user().with_text("steer"))
            .await
            .unwrap_err();

        assert!(
            error.contains(SUBAGENT_PROFILE_MISSING),
            "the refusal must carry a code an interface can branch on: {error}"
        );
        assert!(
            error.contains("delegate the work again"),
            "the refusal must say what to do: {error}"
        );
        let tools = tool_names(&agent, &child_id).await;
        assert!(
            tools.iter().all(|name| !name.starts_with("workspace__")),
            "the legacy snapshot granted workspace control to a subagent: {tools:?}"
        );
        assert!(
            !state.agent_manager.has_session(&child_id).await,
            "a refused child must not stay cached for the next turn to reuse"
        );
        assert!(
            !state.is_turn_active(&child_id),
            "a refused follow-up must release the turn slot it took"
        );
    }

    /// The other half of the same gate: a profile that exists but cannot be
    /// read is a refusal too, and it must not fall back to the snapshot either.
    #[tokio::test]
    async fn an_injected_turn_on_a_child_with_an_unreadable_runtime_profile_is_refused() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let (_temp, child_id) = seed_extension_data(
            &state,
            "br71 injected child corrupt profile",
            SessionType::SubAgent,
            |extension_data| {
                biorouter::session::EnabledExtensionsState::new(vec![broad_workspace_snapshot()])
                    .to_extension_data(extension_data)
                    .unwrap();
                extension_data.set_extension_state(
                    "subagent_runtime_profile",
                    "v999",
                    serde_json::json!({"system_prompt": "do not install"}),
                );
            },
        )
        .await;
        let agent = state.get_agent(child_id.clone()).await.unwrap();

        let error = services
            .start_detached_turn(&child_id, Message::user().with_text("steer"))
            .await
            .unwrap_err();

        assert!(
            error.contains(SUBAGENT_PROFILE_RESTORE_FAILED),
            "the refusal must carry a code an interface can branch on: {error}"
        );
        let tools = tool_names(&agent, &child_id).await;
        assert!(
            tools.iter().all(|name| !name.starts_with("workspace__")),
            "a failed restore fell back to the legacy snapshot: {tools:?}"
        );
        assert!(
            !state.agent_manager.has_session(&child_id).await,
            "a partially restored child must not stay cached"
        );
        assert!(!state.is_turn_active(&child_id));
    }

    /// The over-correction guard. The gate is scoped to `SubAgent` rows: an
    /// ordinary chat has no runtime profile either, and refusing it would take
    /// the whole application down rather than close a delegation hole. An
    /// ordinary chat's saved extension list is still exactly how a cold session
    /// is rebuilt, so it must still be applied here.
    ///
    /// The provider is deliberately unconstructible: `start_detached_turn`
    /// hydrates and restores the provider in one `join!`, so the provider error
    /// proves the call reached the ordinary-session arm while the tool list
    /// proves the snapshot was applied — without firing a live turn.
    #[tokio::test]
    async fn an_injected_turn_on_a_user_session_still_hydrates_its_saved_extensions() {
        let state = crate::state::AppState::new().await.unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        let (_temp, session_id) = seed_extension_data(
            &state,
            "br71 injected user session snapshot",
            SessionType::User,
            |extension_data| {
                biorouter::session::EnabledExtensionsState::new(vec![
                    biorouter::agents::ExtensionConfig::Platform {
                        name: "todo".into(),
                        description: "Todo".into(),
                        bundled: Some(true),
                        available_tools: Vec::new(),
                    },
                ])
                .to_extension_data(extension_data)
                .unwrap();
            },
        )
        .await;
        state
            .session_manager()
            .update(&session_id)
            .provider_name("br71-user-provider-not-in-the-factory")
            .model_config(ModelConfig::new("br71-user-model").unwrap())
            .apply()
            .await
            .unwrap();
        let agent = state.get_agent(session_id.clone()).await.unwrap();

        let error = services
            .start_detached_turn(&session_id, Message::user().with_text("steer"))
            .await
            .unwrap_err();

        assert!(
            !error.contains(SUBAGENT_PROFILE_MISSING)
                && !error.contains(SUBAGENT_PROFILE_RESTORE_FAILED),
            "the subagent gate must not refuse an ordinary chat: {error}"
        );
        assert!(
            error.contains("br71-user-provider-not-in-the-factory"),
            "expected the ordinary-session arm's provider restore, got: {error}"
        );
        let tools = tool_names(&agent, &session_id).await;
        assert!(
            tools.iter().any(|name| name == "todo__todo_write"),
            "an ordinary chat's saved extensions must still be hydrated: {tools:?}"
        );
    }

    /// The KB pair is the only non-trivial *decision* in this file: resolving
    /// `KbPrimaryChoice::Auto` against the RESULTING set, in one place, so every
    /// surface gets the same answer. Both methods returning
    /// `KbSelectionView::default()` — the shape of a stub — passed the gate
    /// before this test existed.
    ///
    /// Runs against a temp knowledge root (`new_with_knowledge_root`), because
    /// it creates bases and moves a write target: against the real service it
    /// would invent knowledge bases in the developer's sidebar.
    #[tokio::test]
    async fn the_kb_pair_resolves_auto_against_the_resulting_set() {
        let temp = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::new_with_knowledge_root(temp.path().to_path_buf())
            .await
            .unwrap();
        let services = ServerWorkspaceServices::new(state.clone());
        state
            .knowledge_service
            .create_base("alpha", "Alpha", None)
            .unwrap();
        state
            .knowledge_service
            .create_base("beta", "Beta", None)
            .unwrap();

        let sid = "br71-kb-scope";
        let set = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // Auto on a scope with no primary pins the first id — the thing that
        // makes a fresh session's KB-less writes work at all.
        let view = services
            .set_knowledge_bases(sid, &set(&["alpha", "beta"]), KbPrimaryChoice::Auto)
            .unwrap();
        assert_eq!(view.kb_ids, set(&["alpha", "beta"]));
        assert_eq!(view.primary_kb.as_deref(), Some("alpha"));
        // `knowledge_selection` reports the same claim the mutator returned —
        // set and pointer together, not two unlocked reads.
        assert_eq!(services.knowledge_selection(sid), view);

        // Auto KEEPS a primary that is still a member of the new set.
        let view = services
            .set_knowledge_bases(sid, &set(&["beta", "alpha"]), KbPrimaryChoice::Auto)
            .unwrap();
        assert_eq!(view.primary_kb.as_deref(), Some("alpha"));

        // Auto MOVES it when the old target leaves the set.
        let view = services
            .set_knowledge_bases(sid, &set(&["beta"]), KbPrimaryChoice::Auto)
            .unwrap();
        assert_eq!(view.kb_ids, set(&["beta"]));
        assert_eq!(view.primary_kb.as_deref(), Some("beta"));

        // Set pins explicitly.
        let view = services
            .set_knowledge_bases(
                sid,
                &set(&["alpha", "beta"]),
                KbPrimaryChoice::Set("beta".to_string()),
            )
            .unwrap();
        assert_eq!(view.primary_kb.as_deref(), Some("beta"));

        // Clear removes the write target without narrowing the set. This is
        // "no primary for this session", not "inherit the machine's".
        let view = services
            .set_knowledge_bases(sid, &set(&["alpha", "beta"]), KbPrimaryChoice::Clear)
            .unwrap();
        assert_eq!(view.kb_ids, set(&["alpha", "beta"]));
        assert_eq!(view.primary_kb, None);
        assert_eq!(services.knowledge_selection(sid), view);

        // An empty set clears the pointer too: no legal member to point at.
        let view = services
            .set_knowledge_bases(sid, &[], KbPrimaryChoice::Auto)
            .unwrap();
        assert!(view.kb_ids.is_empty());
        assert_eq!(view.primary_kb, None);

        // A pin outside the resulting set is refused whole — the service
        // validates against the result, and this seam must not paper over it.
        let err = services
            .set_knowledge_bases(
                sid,
                &set(&["alpha"]),
                KbPrimaryChoice::Set("beta".to_string()),
            )
            .unwrap_err();
        assert!(err.contains("beta"), "{err}");

        // An unknown session has no selection, and reading one never fails.
        assert_eq!(
            services.knowledge_selection("br71-kb-never-seen").kb_ids,
            set(&["alpha", "beta"]),
            "a scope that has hidden nothing sees every installed base"
        );
    }

    /// The daemon's bootstrap step (`commands/agent.rs`) must actually publish a
    /// working implementation — `install` being a no-op, or the daemon never
    /// calling it, is the difference between the workspace tools controlling the
    /// workspace and silently answering about a session-level world.
    ///
    /// **This test writes the process-global `OnceLock`** in
    /// `biorouter::workspace_services`, deliberately and once: it is the only
    /// way to exercise `install`, which every other test in the workspace suite
    /// avoids by using `set_for_tests` instead (see that module's doc comment on
    /// why a `OnceLock` written from a test is unrecoverable). Nothing else in
    /// `biorouter-server` reads `workspace_services::get()`, so pinning it for
    /// the rest of this binary's run is inert. If a future task adds a
    /// `biorouter-server` test that needs "no daemon installed", it will have to
    /// live in its own integration-test binary — not weaken this one.
    #[tokio::test]
    async fn the_bootstrap_publishes_a_working_implementation() {
        let state = crate::state::AppState::new().await.unwrap();
        install_workspace_services(state.clone());

        let installed =
            biorouter::workspace_services::get().expect("the daemon's services are installed");

        // Not just "something is there": it answers, and it answers about THIS
        // daemon's state. A stand-in that returned false for everything would
        // pass the first assertion, so the turn lock is the witness — the
        // installed services see a turn this test starts through `state`.
        let token = tokio_util::sync::CancellationToken::new();
        let _guard = state
            .try_begin_turn_idempotent("br71-bootstrap-witness", token, None)
            .expect("lock acquired");
        assert!(installed.is_turn_active("br71-bootstrap-witness"));
        assert!(!installed.is_turn_active("br71-bootstrap-never-started"));

        // The GUI half, now that Task 23 has wired it to the WorkspaceBridge
        // registry. This replaces Slice 1's `assert!(!gui_attached())` /
        // `assert!(layout_snapshot().is_none())` pair, which stated the STUB's
        // behaviour and, against the live registry, is a cross-test flake:
        // `BRIDGES` is a process-wide static shared with
        // `workspace::bridge::tests::registry_tracks_focus_and_merges_layouts`,
        // which holds two attached windows for part of its run. (Unaided the
        // collision was 0/15; widening that test's window with an 800 ms sleep
        // reproduced it 1/1.) So the assertions below are CONTAINMENT
        // assertions about a window this test owns — never global counts, and
        // never "nothing is attached".
        //
        // They are also strictly stronger than the pair they replace: a stub
        // returning a constant cannot make this daemon's services report an
        // echo that this test stored a moment ago, nor drop it again on detach.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let win = format!("br71-bootstrap-win-{nonce}");
        let bridge = crate::workspace::bridge::bridge_for(&win);
        let (_rx, conn) = bridge.attach();
        bridge.store_echo(serde_json::json!({"window_id": win, "focused_session": null}));

        assert!(
            installed.gui_attached(),
            "with a window attached the daemon's services must report a GUI"
        );
        let carries_our_window = |snapshot: Option<serde_json::Value>| -> bool {
            snapshot
                .and_then(|v| v.as_array().cloned())
                .is_some_and(|entries| {
                    entries
                        .iter()
                        .any(|e| e.get("window_id").and_then(|w| w.as_str()) == Some(win.as_str()))
                })
        };
        assert!(
            carries_our_window(installed.layout_snapshot()),
            "layout_snapshot must carry this window's echo: it IS workspace_list's `gui`"
        );

        // …and it is live, not a captured constant: closing the window drops it.
        bridge.detach(conn);
        assert!(
            !carries_our_window(installed.layout_snapshot()),
            "a closed window's stale echo must not still be reported as GUI state"
        );
    }

    /// A spawn frame must survive the parent's window going away *after* it was
    /// chosen — the gap `bridge_for_session(...).or_else(focused_or_recent)`
    /// left open, because that composition only falls back when the parent
    /// cannot be *found*, never when the found window then refuses the frame.
    ///
    /// Both callers in `subagent_tool.rs` are `let _ = …`, so the loss is
    /// silent: no error surfaces, no tab opens, no badge appears.
    #[tokio::test]
    async fn a_frame_survives_the_parents_window_detaching_after_it_was_chosen() {
        use crate::workspace::bridge::WorkspaceBridge;

        let parent_window = WorkspaceBridge::new();
        let (_parent_rx, conn) = parent_window.attach();
        let other_window = WorkspaceBridge::new();
        let (mut other_rx, _other_conn) = other_window.attach();

        // Resolution picked the parent's window; it closes before the emit.
        parent_window.detach(conn);

        let sent = emit_with_fallback(
            Some(parent_window.clone()),
            || Some(other_window.clone()),
            serde_json::json!({
                "type": "workspace", "cmd": "open_tab", "session_id": "child-1",
            }),
            None,
        )
        .await
        .expect("a window going away must not lose the frame when another is attached");
        assert_eq!(sent["sent"], true);
        let frame = other_rx
            .try_recv()
            .expect("the frame must land on the window that is still attached");
        assert_eq!(frame["session_id"], "child-1");

        // ⚠ The other half of the rule, and the one a "always use the fallback"
        // implementation fails: a LIVE parent window keeps its frames. Without
        // this the test above is satisfied by deleting the routing entirely.
        let live_parent = WorkspaceBridge::new();
        let (mut parent_rx, _conn) = live_parent.attach();
        emit_with_fallback(
            Some(live_parent),
            || Some(other_window.clone()),
            serde_json::json!({
                "type": "workspace", "cmd": "open_tab", "session_id": "child-2",
            }),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            parent_rx.try_recv().unwrap()["session_id"],
            "child-2",
            "the tab belongs beside its parent while that window is alive"
        );
        assert!(
            other_rx.try_recv().is_err(),
            "a live parent's frame must not also be sent to the fallback window"
        );

        // No window left anywhere: still an error, and it names the delivery
        // failure rather than the lookup that came after it.
        let dead = WorkspaceBridge::new();
        let (_rx, conn) = dead.attach();
        dead.detach(conn);
        let err = emit_with_fallback(Some(dead), || None, serde_json::json!({}), None)
            .await
            .unwrap_err();
        assert!(err.contains("no GUI window attached"), "{err}");
    }

    /// The retry must not land back on the window that just failed.
    ///
    /// A window that is attached but never answers fails only by *timing out*,
    /// so a fallback that re-picks it would park for a second full
    /// `GUI_ROUND_TRIP_TIMEOUT` against a window already known to be
    /// unresponsive — ten seconds of a spawn's latency for nothing.
    ///
    /// ⚠ Asserted by COUNTING FRAMES, not by timing. `emit_and_wait` puts the
    /// frame on the wire and *then* parks, so a second attempt is visible as a
    /// second frame in the window's receiver; a wall-clock assertion would need
    /// the real timeout to discriminate, and would be a stopwatch test on a
    /// loaded CI box either way. The timeout is a parameter so this runs in
    /// milliseconds.
    #[tokio::test]
    async fn the_fallback_does_not_retry_the_window_that_just_failed() {
        use crate::workspace::bridge::WorkspaceBridge;

        let wedged = WorkspaceBridge::new();
        let (mut rx, _conn) = wedged.attach(); // attached, but nothing ever replies

        let err = emit_with_fallback(
            Some(wedged.clone()),
            // `focused_or_recent` handing back the very same window: it is
            // attached, so nothing filters it out.
            || Some(wedged.clone()),
            serde_json::json!({"type": "workspace", "cmd": "open_tab"}),
            Some(std::time::Duration::from_millis(50)),
        )
        .await
        .unwrap_err();
        assert!(err.contains("timed out"), "{err}");

        assert!(rx.try_recv().is_ok(), "the first attempt is made");
        assert!(
            rx.try_recv().is_err(),
            "the same window must not be asked a second time"
        );
    }
}
