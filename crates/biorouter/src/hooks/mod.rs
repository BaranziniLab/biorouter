//! User-configurable lifecycle hooks (Claude Code-style).
//!
//! Hooks are shell commands or LLM-judge prompts that run on agent lifecycle
//! events (tool calls, prompts, compaction, session lifecycle). They are
//! configured under `hooks:` in `~/.config/biorouter/config.yaml` and,
//! when enabled via `hooks.allow_project_hooks`, in a project-level
//! `.biorouter/hooks.yaml` inside the session working directory.
//!
//! Execution is failure-open: a crashing, timing-out, or misconfigured hook
//! never blocks the agent. Only explicit decisions block (exit code 2, a
//! `block`/`deny` decision in stdout JSON, or a prompt-hook `ok: false`).

pub mod command_runner;
pub mod config;
pub mod event;
pub mod inspector;
pub mod matcher;
pub mod outcome;
pub mod prompt_runner;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

pub use config::{HookDefinition, HookMatcherGroup, HooksConfig};
pub use event::{HookEvent, HookPayload};
pub use inspector::{apply_tool_input_rewrites, HookInspector};
pub use matcher::InputMatcher;
pub use outcome::{HookAggregate, HookDecision, HookOutcome};

use crate::agents::types::SharedProvider;
use crate::managed::ManagedPolicy;
use crate::providers::base::Provider;
use config::CachedProjectHooks;

/// Default timeout for command hooks.
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 60;
/// Default timeout for prompt (LLM judge) hooks.
pub const DEFAULT_PROMPT_TIMEOUT_SECS: u64 = 30;
/// Maximum consecutive Stop-hook blocks per session before Biorouter
/// overrides the hook and stops anyway.
pub const STOP_HOOK_BLOCK_CAP: u32 = 5;

/// BR-19: maximum consecutive PostToolUse blocks honored per session before
/// Biorouter overrides the hook and delivers the tool result anyway. A
/// PostToolUse block feeds a correction back to the model, which will typically
/// retry the tool — a hook that blocks unconditionally would otherwise wedge
/// the turn in a retry loop. Same shape as [`STOP_HOOK_BLOCK_CAP`], smaller
/// because each block costs a full tool round trip.
pub const POST_TOOL_HOOK_BLOCK_CAP: u32 = 3;

/// BR-19: cap on staged tool-hook effects held per session. Bounded for the
/// same reason as [`MAX_FIRED_OUTCOMES`]: a caller that never drains (a turn
/// cancelled between the inspector and the injection point) must not grow the
/// buffer without bound. Oldest entries are dropped.
const MAX_STAGED_TOOL_HOOKS: usize = 64;

/// BR-28: how long a turn boundary waits for still-running observe-only hook
/// tasks ([`HooksManager::fire`]) before giving up *on this boundary*. Kept
/// small on purpose — settling must never stall the agent loop behind a slow
/// Notification / compaction hook. Unfinished tasks stay registered and are
/// re-joined at the next boundary (nothing is aborted; each hook's own timeout
/// still bounds how long it can run).
pub const FIRE_JOIN_BUDGET: Duration = Duration::from_millis(250);

/// BR-28: budget for a shutdown-style join (session end), where waiting for a
/// hook to finish is the point rather than an interruption.
pub const FIRE_JOIN_BUDGET_SHUTDOWN: Duration = Duration::from_secs(5);

/// BR-28: cap on captured `fire()` aggregates held in memory. A session that
/// never settles (e.g. a subagent that exits before its start hook returns)
/// must not grow the buffer without bound; the oldest entry is dropped.
const MAX_FIRED_OUTCOMES: usize = 64;

/// Issue #56 Gate H: cap on the per-session classifications one
/// [`HooksManager`] remembers. Nothing clears the store at session end, and
/// `biorouter web` hosts an unbounded number of sessions on a single manager.
/// The oldest entry is evicted, and an evicted session reads as *unknown*,
/// which refuses a public prompt-hook provider rather than admitting one — so
/// the bound can only cost liveness, never disclosure.
const MAX_SESSION_CLASSIFICATIONS: usize = 256;

/// BR-28: the aggregate of an observe-only hook event dispatched via
/// [`HooksManager::fire`] — Notification, SubagentStart/Stop, Pre/PostCompact.
///
/// These used to be spawned detached with the whole [`HookAggregate`] dropped
/// on the floor, so a `systemMessage` was invisible, a failing hook untraceable,
/// and there was no way to know a compaction/subagent hook had even run. The
/// aggregate is now captured here and drained by the caller at a turn or
/// shutdown boundary via [`HooksManager::settle_fired`].
#[derive(Debug, Clone)]
pub struct FiredHookOutcome {
    pub event: HookEvent,
    pub session_id: String,
    pub aggregate: HookAggregate,
}

/// BR-19: the non-decision effects of a PreToolUse / PermissionRequest hook,
/// staged at the hook's call site (the [`HookInspector`], the permission gate in
/// `agents::tool_execution`) for the agent loop to apply on the tool path.
///
/// Both call sites used to read only `aggregate.decision`, so a hook's
/// `updatedInput`, `additionalContext` and `systemMessage` were silently
/// discarded. They are staged here instead: the rewrite is taken *before*
/// dispatch (`take_tool_input_rewrites`), the context/messages at the turn's
/// injection point (`drain_tool_hook_context`).
#[derive(Debug, Clone)]
pub struct StagedToolHook {
    pub event: HookEvent,
    pub tool_request_id: String,
    pub tool_name: String,
    /// Rewritten tool arguments to apply before dispatch (PreToolUse only).
    /// Taken out by [`HooksManager::take_tool_input_rewrites`].
    pub updated_input: Option<serde_json::Value>,
    pub additional_context: Vec<String>,
    pub system_messages: Vec<String>,
}

/// Result of consulting Stop hooks at turn exit.
#[derive(Debug, Clone, PartialEq)]
pub enum StopHookVerdict {
    /// No hook objected — finish the turn.
    Proceed,
    /// A hook blocked the stop; keep working and feed `reason` to the model.
    Blocked { reason: String },
    /// Hooks kept blocking past [`STOP_HOOK_BLOCK_CAP`]; stop anyway.
    CapReached,
}

pub struct HooksManager {
    global: HooksConfig,
    allow_project_hooks: bool,
    project_cache: RwLock<HashMap<PathBuf, CachedProjectHooks>>,
    provider: SharedProvider,
    custom_providers: Mutex<HashMap<(String, String), Arc<dyn Provider>>>,
    sessions_started: Mutex<HashSet<String>>,
    stop_blocks: Mutex<HashMap<String, u32>>,
    /// Hooks registered programmatically at runtime, scoped to one session
    /// (e.g. the `/goal` Stop-hook evaluator). Not persisted; cleared when the
    /// owning feature clears them or the process exits.
    session_hooks: RwLock<HashMap<String, HashMap<HookEvent, Vec<HookDefinition>>>>,
    /// Trusted admin/managed policy (BR-65). Managed hook groups run first and
    /// cannot be disabled; a managed `allow_project_hooks` override wins over
    /// the user/env opt-in. Inert when no managed file is present.
    managed: Arc<ManagedPolicy>,
    /// BR-28: detached `fire()` tasks still in flight, so a turn/shutdown
    /// boundary can join them instead of letting them outlive the turn and race
    /// process shutdown. `std::sync::Mutex` (never held across an await) so the
    /// synchronous `fire()` can register a handle without blocking.
    pending_fires: std::sync::Mutex<Vec<JoinHandle<()>>>,
    /// BR-28: aggregates captured from finished `fire()` tasks, awaiting a
    /// [`Self::settle_fired`] drain by the owning session. Bounded by
    /// [`MAX_FIRED_OUTCOMES`].
    fired: std::sync::Mutex<VecDeque<FiredHookOutcome>>,
    /// BR-19: consecutive honored PostToolUse blocks per session, capped by
    /// [`POST_TOOL_HOOK_BLOCK_CAP`] so a hook that always blocks cannot wedge
    /// the turn (same pattern as `stop_blocks`).
    post_tool_blocks: Mutex<HashMap<String, u32>>,
    /// BR-19: tool-path hook effects staged by the inspector / permission gate,
    /// keyed by session id. Bounded by [`MAX_STAGED_TOOL_HOOKS`].
    /// `std::sync::Mutex` (never held across an await) so the staging call sites
    /// stay synchronous.
    staged_tool_hooks: std::sync::Mutex<HashMap<String, VecDeque<StagedToolHook>>>,
    /// Issue #56 Gate H: the classification of each session that has stated one
    /// on this manager, mirrored from `Agent::cached_classification` at the one
    /// seam that writes it (`Agent::reply`), so the two cannot disagree about
    /// the same turn.
    ///
    /// **Keyed by session id, not a single scalar.** A `HooksManager` is built
    /// one per `Agent` and `AgentManager` holds one agent per session, so a
    /// scalar looks sufficient — but `biorouter web` shares ONE `Arc<Agent>`
    /// across every chat it hosts (`commands/web.rs`) and spawns concurrent
    /// turns from caller-supplied session ids. A scalar there records whichever
    /// turn wrote last: a private turn stores `Private`, an overlapping public
    /// turn overwrites it with `Public`, and the private turn's Stop hook — the
    /// one carrying `transcript_tail` — then resolves a public prompt provider.
    ///
    /// A field rather than a parameter because a prompt hook's provider is
    /// resolved five frames below `dispatch`, and threading a classification
    /// through `dispatch`/`fire`/`stop`/`pre_tool_use`/… would change every
    /// public entry point on this type for the benefit of one hook kind. The
    /// session id needs no such threading: `HookPayload` already carries it the
    /// whole way down.
    ///
    /// **Absent means unknown, and unknown is fail-closed** — the same direction
    /// as `CachedClassification`'s `Private` default. A hook that fires before
    /// the owning agent has stated the classification is skipped with a message
    /// saying exactly that, rather than run against an unknown session, and
    /// hooks are failure-open by doctrine so a skipped hook never blocks a turn.
    ///
    /// A `VecDeque` rather than a `HashMap` so the bound is the structure
    /// itself: at most [`MAX_SESSION_CLASSIFICATIONS`] entries, oldest evicted
    /// first, scanned linearly on a path that runs at most once per turn.
    session_classifications:
        std::sync::Mutex<VecDeque<(String, crate::privacy::SessionClassification)>>,
}

impl HooksManager {
    /// Build from the global Biorouter config, loading the managed policy.
    pub fn new(provider: SharedProvider) -> Self {
        Self::new_with_managed(provider, ManagedPolicy::load())
    }

    /// Build from the global Biorouter config with an already-loaded managed
    /// policy, so the agent can share one instance across hooks + inspectors.
    pub fn new_with_managed(provider: SharedProvider, managed: Arc<ManagedPolicy>) -> Self {
        let global = config::load_global_config();
        let allow_project_hooks = global.allow_project_hooks
            || std::env::var("BIOROUTER_ALLOW_PROJECT_HOOKS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
        Self::with_config_and_managed(global, allow_project_hooks, provider, managed)
    }

    /// Build with an explicit config (used by tests). No managed policy.
    pub fn with_config(
        global: HooksConfig,
        allow_project_hooks: bool,
        provider: SharedProvider,
    ) -> Self {
        Self::with_config_and_managed(
            global,
            allow_project_hooks,
            provider,
            Arc::new(ManagedPolicy::empty()),
        )
    }

    /// Build with an explicit config and managed policy. A managed
    /// `allow_project_hooks` override, when set, wins over the user/env value.
    pub fn with_config_and_managed(
        global: HooksConfig,
        user_allow_project_hooks: bool,
        provider: SharedProvider,
        managed: Arc<ManagedPolicy>,
    ) -> Self {
        let allow_project_hooks = managed
            .project_hooks_override()
            .unwrap_or(user_allow_project_hooks);
        Self {
            global,
            allow_project_hooks,
            project_cache: RwLock::new(HashMap::new()),
            provider,
            custom_providers: Mutex::new(HashMap::new()),
            sessions_started: Mutex::new(HashSet::new()),
            stop_blocks: Mutex::new(HashMap::new()),
            session_hooks: RwLock::new(HashMap::new()),
            managed,
            pending_fires: std::sync::Mutex::new(Vec::new()),
            fired: std::sync::Mutex::new(VecDeque::new()),
            post_tool_blocks: Mutex::new(HashMap::new()),
            staged_tool_hooks: std::sync::Mutex::new(HashMap::new()),
            session_classifications: std::sync::Mutex::new(VecDeque::new()),
        }
    }

    /// Issue #56 Gate H. State the classification of the session whose turn is
    /// running. Called by `Agent::reply` at the same seam that stores the
    /// agent's own cached classification — mirror it there and nowhere else, or
    /// the two answers drift.
    pub fn set_session_classification(
        &self,
        session_id: &str,
        classification: crate::privacy::SessionClassification,
    ) {
        let mut store = self
            .session_classifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        store.retain(|(id, _)| id != session_id);
        store.push_back((session_id.to_string(), classification));
        while store.len() > MAX_SESSION_CLASSIFICATIONS {
            store.pop_front();
        }
    }

    /// The classification stated for `session_id`, or `None` when no turn of
    /// that session has run on this manager (or its entry has been evicted).
    ///
    /// `None` is **not** `Public`: see the field's doc comment. Callers must
    /// treat it as at least as restrictive as `Private`.
    fn session_classification(
        &self,
        session_id: &str,
    ) -> Option<crate::privacy::SessionClassification> {
        self.session_classifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(id, _)| id == session_id)
            .map(|(_, classification)| *classification)
    }

    /// Replace the runtime hooks for (session, event). Session hooks are
    /// merged into every matching `dispatch` for that session, after the
    /// config-file hooks (most-restrictive decision still wins).
    pub async fn set_session_hooks(
        &self,
        session_id: &str,
        event: HookEvent,
        hooks: Vec<HookDefinition>,
    ) {
        self.session_hooks
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .insert(event, hooks);
    }

    /// Remove the runtime hooks for (session, event).
    pub async fn clear_session_hooks(&self, session_id: &str, event: HookEvent) {
        let mut map = self.session_hooks.write().await;
        if let Some(events) = map.get_mut(session_id) {
            events.remove(&event);
            if events.is_empty() {
                map.remove(session_id);
            }
        }
    }

    /// Whether any runtime hook is registered for (session, event).
    pub async fn has_session_hooks(&self, session_id: &str, event: HookEvent) -> bool {
        self.session_hooks
            .read()
            .await
            .get(session_id)
            .and_then(|events| events.get(&event))
            .is_some_and(|hooks| !hooks.is_empty())
    }

    async fn session_definitions(&self, session_id: &str, event: HookEvent) -> Vec<HookDefinition> {
        self.session_hooks
            .read()
            .await
            .get(session_id)
            .and_then(|events| events.get(&event))
            .cloned()
            .unwrap_or_default()
    }

    /// Consecutive Stop-hook blocks recorded for a session (resets when tools
    /// run or a stop proceeds). At [`STOP_HOOK_BLOCK_CAP`] the cap was hit.
    pub async fn stop_block_count(&self, session_id: &str) -> u32 {
        self.stop_blocks
            .lock()
            .await
            .get(session_id)
            .copied()
            .unwrap_or(0)
    }

    /// Resolved config for a working dir, in precedence order: managed groups
    /// (admin tier, always run, cannot be disabled), then global, then project.
    /// Merge semantics are unchanged (most-restrictive decision wins), so a
    /// managed `Stop`/`PreToolUse` block cannot be undone by a user hook.
    async fn resolved_groups(&self, event: HookEvent, working_dir: &Path) -> Vec<HookMatcherGroup> {
        let mut groups = self
            .managed
            .hooks()
            .events
            .get(&event)
            .cloned()
            .unwrap_or_default();
        groups.extend(self.global.events.get(&event).cloned().unwrap_or_default());
        if self.allow_project_hooks {
            let project = self.project_config(working_dir).await;
            if let Some(project_groups) = project.events.get(&event) {
                groups.extend(project_groups.clone());
            }
        }
        groups
    }

    async fn project_config(&self, working_dir: &Path) -> HooksConfig {
        let current_mtime = config::project_hooks_mtime(working_dir);
        {
            let cache = self.project_cache.read().await;
            if let Some(cached) = cache.get(working_dir) {
                if cached.mtime == current_mtime {
                    return cached.config.clone();
                }
            }
        }
        let fresh = config::read_project_hooks(working_dir);
        let result = fresh.config.clone();
        self.project_cache
            .write()
            .await
            .insert(working_dir.to_path_buf(), fresh);
        result
    }

    /// Cheap check whether any hook could run for this event + matcher key.
    ///
    /// Name-matcher only: a group's `input_matcher` (BR-27) is *not* evaluated
    /// here because the tool input is not available at the gate. This
    /// deliberately over-approximates — `dispatch` applies the input matcher
    /// and may then run nothing — so an input-matched hook is never gated out.
    pub async fn has_hooks(
        &self,
        event: HookEvent,
        matcher_key: Option<&str>,
        working_dir: &Path,
    ) -> bool {
        self.resolved_groups(event, working_dir)
            .await
            .iter()
            .any(|group| {
                !group.hooks.is_empty()
                    && match matcher_key {
                        Some(key) => matcher::matcher_matches(group.matcher.as_deref(), key),
                        None => true,
                    }
            })
    }

    /// Run all hooks matching (event, matcher_key) concurrently and merge
    /// their outcomes. Never returns an error: hook failures are recorded in
    /// the aggregate and treated as non-blocking.
    pub async fn dispatch(
        &self,
        event: HookEvent,
        matcher_key: Option<&str>,
        payload: &HookPayload,
        working_dir: &Path,
    ) -> HookAggregate {
        let groups = self.resolved_groups(event, working_dir).await;
        let tool_input = payload.tool_input.as_ref();
        let mut definitions: Vec<HookDefinition> = groups
            .iter()
            .filter(|group| group.matches(matcher_key, tool_input))
            .flat_map(|group| group.hooks.iter().cloned())
            .collect();

        // Runtime session-scoped hooks always match (no matcher).
        definitions.extend(self.session_definitions(&payload.session_id, event).await);

        if definitions.is_empty() {
            return HookAggregate::default();
        }

        debug!(
            "hooks: dispatching {} hook(s) for {} (matcher key: {:?})",
            definitions.len(),
            event,
            matcher_key
        );

        let payload_json = payload.to_json();
        let futures = definitions
            .into_iter()
            .map(|definition| self.run_one(definition, event, &payload_json, payload, working_dir));
        let outcomes = futures::future::join_all(futures).await;

        let aggregate = outcome::merge_outcomes(outcomes);
        for error in &aggregate.errors {
            warn!("hooks: non-blocking hook failure on {}: {}", event, error);
        }
        aggregate
    }

    /// Detached dispatch for observe-only events; never blocks the agent loop.
    ///
    /// BR-28: the spawned task's [`HookAggregate`] is no longer discarded — it
    /// is captured (when it carries anything: a system message, injected
    /// context, a decision, or an error) and the task handle is registered so a
    /// turn or shutdown boundary can join it via [`Self::settle_fired`]. Callers
    /// therefore *can* act on the combined outcome of a Notification /
    /// SubagentStart|Stop / Pre|PostCompact hook, and these tasks no longer
    /// silently outlive the turn.
    pub fn fire(
        self: &Arc<Self>,
        event: HookEvent,
        matcher_key: Option<String>,
        payload: HookPayload,
        working_dir: PathBuf,
    ) {
        let manager = Arc::clone(self);
        let session_id = payload.session_id.clone();
        let handle = tokio::spawn(async move {
            let aggregate = manager
                .dispatch(event, matcher_key.as_deref(), &payload, &working_dir)
                .await;
            if aggregate.is_empty() {
                return;
            }
            manager.record_fired(FiredHookOutcome {
                event,
                session_id,
                aggregate,
            });
        });
        let mut pending = self
            .pending_fires
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pending.retain(|handle| !handle.is_finished());
        pending.push(handle);
    }

    fn record_fired(&self, outcome: FiredHookOutcome) {
        let mut fired = self.fired.lock().unwrap_or_else(PoisonError::into_inner);
        while fired.len() >= MAX_FIRED_OUTCOMES {
            fired.pop_front();
        }
        fired.push_back(outcome);
    }

    /// BR-28: join outstanding [`Self::fire`] tasks, waiting at most `budget`.
    /// Returns how many finished. Nothing is aborted — a task that misses the
    /// budget stays registered and is re-joined at the next boundary, so a slow
    /// hook delays only its own observability, never the agent loop.
    pub async fn join_fired(&self, budget: Duration) -> usize {
        let handles: Vec<JoinHandle<()>> = {
            let mut pending = self
                .pending_fires
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *pending)
        };
        if handles.is_empty() {
            return 0;
        }
        let deadline = tokio::time::Instant::now() + budget;
        let mut joined = 0usize;
        let mut unfinished = Vec::new();
        for mut handle in handles {
            match tokio::time::timeout_at(deadline, &mut handle).await {
                Ok(Ok(())) => joined += 1,
                Ok(Err(e)) => {
                    warn!("hooks: fired hook task failed: {e}");
                    joined += 1;
                }
                Err(_) => unfinished.push(handle),
            }
        }
        if !unfinished.is_empty() {
            debug!(
                "hooks: {} fired hook task(s) still running past the {:?} join budget",
                unfinished.len(),
                budget
            );
            self.pending_fires
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(unfinished);
        }
        joined
    }

    /// BR-28: take the captured aggregates of the observe-only hooks fired for
    /// `session_id`. Other sessions' outcomes are left in place.
    pub fn drain_fired(&self, session_id: &str) -> Vec<FiredHookOutcome> {
        let mut fired = self.fired.lock().unwrap_or_else(PoisonError::into_inner);
        let mut taken = Vec::new();
        let mut kept = VecDeque::with_capacity(fired.len());
        while let Some(outcome) = fired.pop_front() {
            if outcome.session_id == session_id {
                taken.push(outcome);
            } else {
                kept.push_back(outcome);
            }
        }
        *fired = kept;
        taken
    }

    /// BR-28: the turn/shutdown-boundary call — join what has finished (bounded
    /// by `budget`) and hand back this session's captured aggregates so the
    /// caller can surface their `systemMessage`s and errors.
    pub async fn settle_fired(&self, session_id: &str, budget: Duration) -> Vec<FiredHookOutcome> {
        self.join_fired(budget).await;
        self.drain_fired(session_id)
    }

    /// Number of `fire()` tasks still registered as in flight (observability).
    pub fn pending_fire_count(&self) -> usize {
        self.pending_fires
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    // ---- BR-19: tool-path hook effects (rewrite + context) ----

    /// BR-19: stage a tool-path hook's non-decision effects for the agent loop.
    /// Called by the [`HookInspector`] (PreToolUse) and the permission gate
    /// (PermissionRequest), whose own return channels can only carry a decision.
    pub fn stage_tool_hook(&self, session_id: &str, staged: StagedToolHook) {
        let mut buffer = self
            .staged_tool_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = buffer.entry(session_id.to_string()).or_default();
        while entry.len() >= MAX_STAGED_TOOL_HOOKS {
            entry.pop_front();
        }
        entry.push_back(staged);
    }

    /// BR-19: take **every** input rewrite staged for `session_id`, as
    /// `tool_request_id -> new arguments`. Destructive for the rewrite only —
    /// the staged context/system messages stay queued for
    /// [`Self::drain_tool_hook_context`] at the turn's injection point, which
    /// runs later (after the permission gate has staged its own).
    ///
    /// ⚠ "Every" is safe for the agent loop and **not** safe for a per-call
    /// caller. `Agent::inspect_and_gate_tool_requests` holds the whole batch of
    /// the model's tool requests at once, so a session-wide take is a take of
    /// exactly its own requests. A caller that handles one request at a time
    /// while others are in flight for the same session — the coding-agent tool
    /// bridge, which serves parallel `tools/call`s over HTTP with no
    /// serialization — would drain its siblings' rewrites, apply the one keyed on
    /// its own request id, and drop the rest on the floor: the hooks ran, the
    /// user believes their sandboxing applied, and the untouched arguments
    /// executed. Those callers must use [`Self::take_tool_input_rewrites_for`].
    pub fn take_tool_input_rewrites(&self, session_id: &str) -> HashMap<String, serde_json::Value> {
        self.take_staged_rewrites(session_id, |_| true)
    }

    /// BR-19: take only the rewrites keyed on `request_ids`, leaving every other
    /// entry staged for whoever owns it.
    ///
    /// This is [`Self::take_tool_input_rewrites`] narrowed to the requests the
    /// caller is actually about to dispatch, and it exists because the staging
    /// buffer is keyed by *session*, not by call. One session can have several
    /// tool calls in flight at once whenever something other than the agent loop
    /// is driving them, and then a session-wide take is theft: the first caller
    /// to reach it removes the rewrites its siblings staged microseconds earlier
    /// and cannot apply them (they are keyed on request ids it has never seen),
    /// so those calls dispatch un-rewritten. Nondeterministically, and with no
    /// error anywhere — which is the same silent failure the rewrite plumbing
    /// exists to prevent, only harder to see.
    pub fn take_tool_input_rewrites_for(
        &self,
        session_id: &str,
        request_ids: &[String],
    ) -> HashMap<String, serde_json::Value> {
        self.take_staged_rewrites(session_id, |id| request_ids.iter().any(|want| want == id))
    }

    /// Shared body of the two takes above: remove `updated_input` from the staged
    /// entries whose `tool_request_id` the caller wants, leave the rest alone.
    fn take_staged_rewrites(
        &self,
        session_id: &str,
        wanted: impl Fn(&str) -> bool,
    ) -> HashMap<String, serde_json::Value> {
        let mut buffer = self
            .staged_tool_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(entries) = buffer.get_mut(session_id) else {
            return HashMap::new();
        };
        let mut rewrites = HashMap::new();
        for entry in entries.iter_mut() {
            if !wanted(&entry.tool_request_id) {
                continue;
            }
            if let Some(input) = entry.updated_input.take() {
                rewrites.insert(entry.tool_request_id.clone(), input);
            }
        }
        rewrites
    }

    /// BR-19: drain **all** the staged tool-path hook effects for `session_id` so
    /// the turn can inject their `additionalContext` (as framed hook context) and
    /// surface their `systemMessage`s.
    ///
    /// Session-wide for the same reason [`Self::take_tool_input_rewrites`] is,
    /// and unsafe for a per-call caller for the same reason; see
    /// [`Self::drain_tool_hook_context_for`].
    pub fn drain_tool_hook_context(&self, session_id: &str) -> Vec<StagedToolHook> {
        let mut buffer = self
            .staged_tool_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        buffer
            .remove(session_id)
            .map(|entries| entries.into_iter().collect())
            .unwrap_or_default()
    }

    /// BR-19: drain only the staged effects belonging to `request_ids`.
    ///
    /// The counterpart of [`Self::take_tool_input_rewrites_for`], for a caller
    /// that finishes one tool call at a time. Two things go wrong if such a
    /// caller never drains at all: its own entries sit in the per-session buffer
    /// until the session next runs an *agent* turn, which then injects context
    /// about somebody else's long-finished tool call into its own transcript; and
    /// at [`MAX_STAGED_TOOL_HOOKS`] entries the buffer starts dropping its oldest,
    /// so a long-running bridged turn can silently evict staged effects. Draining
    /// session-wide instead would fix the leak and reintroduce the theft, taking
    /// entries a concurrent sibling call still needs.
    pub fn drain_tool_hook_context_for(
        &self,
        session_id: &str,
        request_ids: &[String],
    ) -> Vec<StagedToolHook> {
        let mut buffer = self
            .staged_tool_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(entries) = buffer.get_mut(session_id) else {
            return Vec::new();
        };
        let mut taken = Vec::new();
        let mut kept = VecDeque::with_capacity(entries.len());
        while let Some(entry) = entries.pop_front() {
            if request_ids.contains(&entry.tool_request_id) {
                taken.push(entry);
            } else {
                kept.push_back(entry);
            }
        }
        if kept.is_empty() {
            // Keep the map from accumulating an empty deque per session that has
            // ever staged anything; `clear_staged_tool_hooks` relies on absence
            // and presence meaning the same thing here.
            buffer.remove(session_id);
        } else {
            *entries = kept;
        }
        taken
    }

    /// BR-19: drop any tool-path hook effects still staged for `session_id`
    /// (a turn cancelled between the inspector and the injection point), so a
    /// fresh prompt does not inherit context about an aborted tool call.
    pub fn clear_staged_tool_hooks(&self, session_id: &str) {
        self.staged_tool_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session_id);
    }

    /// BR-19: record a honored PostToolUse block for `session_id`. Returns
    /// `true` while the block should be honored and `false` once
    /// [`POST_TOOL_HOOK_BLOCK_CAP`] consecutive blocks have been honored — at
    /// which point the result is delivered anyway, so a hook that blocks
    /// unconditionally cannot trap the turn in a retry loop.
    pub async fn note_post_tool_block(&self, session_id: &str) -> bool {
        let mut blocks = self.post_tool_blocks.lock().await;
        let count = blocks.entry(session_id.to_string()).or_insert(0);
        if *count >= POST_TOOL_HOOK_BLOCK_CAP {
            warn!(
                "hooks: PostToolUse block cap ({}) reached for session {}; delivering the tool result anyway",
                POST_TOOL_HOOK_BLOCK_CAP, session_id
            );
            return false;
        }
        *count += 1;
        true
    }

    /// BR-19: reset the consecutive PostToolUse-block counter (a tool result
    /// made it through unblocked).
    pub async fn reset_post_tool_blocks(&self, session_id: &str) {
        self.post_tool_blocks.lock().await.remove(session_id);
    }

    /// Consecutive PostToolUse blocks honored for a session (observability).
    pub async fn post_tool_block_count(&self, session_id: &str) -> u32 {
        self.post_tool_blocks
            .lock()
            .await
            .get(session_id)
            .copied()
            .unwrap_or(0)
    }

    async fn run_one(
        &self,
        definition: HookDefinition,
        event: HookEvent,
        payload_json: &str,
        payload: &HookPayload,
        working_dir: &Path,
    ) -> HookOutcome {
        match definition {
            HookDefinition::Command { command, timeout } => {
                let timeout =
                    Duration::from_secs(timeout.unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS).max(1));
                let envs = vec![
                    (
                        "BIOROUTER_HOOK_EVENT".to_string(),
                        event.as_str().to_string(),
                    ),
                    (
                        "BIOROUTER_SESSION_ID".to_string(),
                        payload.session_id.clone(),
                    ),
                    (
                        "BIOROUTER_PROJECT_DIR".to_string(),
                        working_dir.to_string_lossy().to_string(),
                    ),
                ];
                match command_runner::run_command_hook(
                    &command,
                    payload_json,
                    working_dir,
                    &envs,
                    timeout,
                )
                .await
                {
                    Ok(result) => outcome::interpret_command_result(
                        event,
                        result.exit_code,
                        &result.stdout,
                        &result.stderr,
                    ),
                    Err(e) => HookOutcome {
                        error: Some(format!("'{command}': {e}")),
                        ..Default::default()
                    },
                }
            }
            HookDefinition::Prompt {
                prompt,
                model,
                provider,
                timeout,
            } => {
                let timeout =
                    Duration::from_secs(timeout.unwrap_or(DEFAULT_PROMPT_TIMEOUT_SECS).max(1));
                let resolved = self
                    .resolve_prompt_provider(&payload.session_id, provider, model)
                    .await;
                match resolved {
                    Ok(provider) => {
                        prompt_runner::run_prompt_hook(
                            provider,
                            event,
                            &prompt,
                            payload_json,
                            timeout,
                        )
                        .await
                    }
                    // Issue #56 Gate H. The reason travels with the refusal
                    // rather than being flattened into a bare `None`, so it
                    // lands in `HookAggregate::errors` — which `dispatch` logs
                    // at `warn!`, and which every caller that reads the
                    // aggregate can show.
                    //
                    // ⚠ For the Stop hook specifically that is the LOG and
                    // nothing else: `HooksManager::stop` returns a
                    // `StopHookVerdict` built from `aggregate.decision` alone
                    // and drops `aggregate.errors` on the floor. A skipped
                    // prompt hook is therefore invisible to the user and to the
                    // model on that path. That is the plan's `last_warning()`
                    // requirement met and no more; do not describe it as a
                    // user-visible refusal.
                    Err(reason) => HookOutcome {
                        error: Some(reason),
                        ..Default::default()
                    },
                }
            }
        }
    }

    /// Provider for a prompt hook: an explicit provider+model pair builds
    /// (and caches) a dedicated provider; otherwise the agent's provider is
    /// used (its fast model when configured).
    ///
    /// `Err` carries the sentence the outcome shows rather than a bare `None`,
    /// so the one refusal that is a *decision* — issue #56 Gate H below — is
    /// distinguishable from a missing provider.
    ///
    /// The `else` branch is deliberately **not** gated: that is the session's own
    /// bound provider, which Gates A and B already govern. Gating it here would
    /// be a second answer to a question already answered, and it would break the
    /// `/goal` Stop-hook evaluator, whose definition carries no provider at all.
    async fn resolve_prompt_provider(
        &self,
        session_id: &str,
        provider_name: Option<String>,
        model: Option<String>,
    ) -> Result<Arc<dyn Provider>, String> {
        let Some((name, model)) = provider_name.zip(model) else {
            return self
                .provider
                .lock()
                .await
                .clone()
                .ok_or_else(|| "prompt hook: no provider available".to_string());
        };

        let key = (name.clone(), model.clone());
        let cached = {
            let cache = self.custom_providers.lock().await;
            cache.get(&key).map(Arc::clone)
        };
        // The gate below runs on BOTH paths — a cache hit is the common case
        // once a hook has fired once, so a gate that only guarded the freshly
        // created provider would protect the first turn of a session and nothing
        // after it. The cache entry is still recorded on a refusal, because the
        // provider itself is legitimate: it is this *session* that may not use
        // it, and the next one on this manager may.
        let provider = match cached {
            Some(provider) => provider,
            None => {
                let model_config = crate::model::ModelConfig::new(&model).map_err(|e| {
                    warn!("hooks: invalid prompt hook model '{}': {}", model, e);
                    format!("prompt hook: invalid model '{model}': {e}")
                })?;
                let provider = crate::providers::create(&name, model_config)
                    .await
                    .map_err(|e| {
                        warn!(
                            "hooks: failed to create prompt hook provider '{}': {}",
                            name, e
                        );
                        format!("prompt hook: no provider available: {e}")
                    })?;
                self.custom_providers
                    .lock()
                    .await
                    .insert(key, Arc::clone(&provider));
                provider
            }
        };

        // Issue #56 Gate H. The Stop hook's payload carries
        // `transcript_tail(&conversation)` at the end of every turn, and this
        // provider was named by a config file rather than by the session row.
        //
        // A session that has stated nothing is treated as `Private` — fail
        // closed — but it must not be TOLD it is private, because nobody read a
        // row: `SubagentStart` fires on a freshly built child agent whose
        // `reply` has not run, and `SessionEnd` can fire on a session with no
        // turn at all. Same decision, honest sentence.
        let stated = self.session_classification(session_id);
        crate::privacy::assert_alt_provider_allowed(
            "this prompt hook",
            provider.as_ref(),
            stated.unwrap_or(crate::privacy::SessionClassification::Private),
            "the hook's `provider:` field",
        )
        .map_err(|e| {
            let reason = match stated {
                Some(_) => e.to_string(),
                None => format!(
                    "No turn of this chat has stated whether it is private, so this prompt hook \
                     was skipped rather than run on `{}`, which is a public model. Hooks that \
                     fire outside a turn have no classification to check against.",
                    provider.get_name()
                ),
            };
            warn!("hooks: {reason}");
            reason
        })?;
        Ok(provider)
    }

    // ---- Typed wrappers used by the agent loop ----

    pub async fn pre_tool_use(
        &self,
        session_id: &str,
        working_dir: &Path,
        tool_name: &str,
        tool_input: &serde_json::Value,
    ) -> HookAggregate {
        let mut payload = HookPayload::new(
            HookEvent::PreToolUse,
            session_id,
            working_dir.to_string_lossy(),
        );
        payload.tool_name = Some(tool_name.to_string());
        payload.tool_input = Some(tool_input.clone());
        self.dispatch(
            HookEvent::PreToolUse,
            Some(tool_name),
            &payload,
            working_dir,
        )
        .await
    }

    pub async fn permission_request(
        &self,
        session_id: &str,
        working_dir: &Path,
        tool_name: &str,
        tool_input: &serde_json::Value,
    ) -> HookAggregate {
        let mut payload = HookPayload::new(
            HookEvent::PermissionRequest,
            session_id,
            working_dir.to_string_lossy(),
        );
        payload.tool_name = Some(tool_name.to_string());
        payload.tool_input = Some(tool_input.clone());
        self.dispatch(
            HookEvent::PermissionRequest,
            Some(tool_name),
            &payload,
            working_dir,
        )
        .await
    }

    pub async fn user_prompt_submit(
        &self,
        session_id: &str,
        working_dir: &Path,
        prompt: &str,
    ) -> HookAggregate {
        let mut payload = HookPayload::new(
            HookEvent::UserPromptSubmit,
            session_id,
            working_dir.to_string_lossy(),
        );
        payload.prompt = Some(prompt.to_string());
        self.dispatch(HookEvent::UserPromptSubmit, None, &payload, working_dir)
            .await
    }

    /// Consult Stop hooks at turn exit, enforcing the consecutive-block cap.
    /// `transcript_tail` (recent conversation, role-prefixed and truncated) is
    /// passed through to hooks so judges can evaluate what the agent did.
    pub async fn stop(
        &self,
        session_id: &str,
        working_dir: &Path,
        transcript_tail: Option<String>,
    ) -> StopHookVerdict {
        if !self.has_hooks(HookEvent::Stop, None, working_dir).await
            && !self.has_session_hooks(session_id, HookEvent::Stop).await
        {
            return StopHookVerdict::Proceed;
        }
        let blocks_so_far = {
            let blocks = self.stop_blocks.lock().await;
            blocks.get(session_id).copied().unwrap_or(0)
        };
        if blocks_so_far >= STOP_HOOK_BLOCK_CAP {
            warn!(
                "hooks: Stop hook block cap ({}) reached for session {}; stopping anyway",
                STOP_HOOK_BLOCK_CAP, session_id
            );
            return StopHookVerdict::CapReached;
        }
        let mut payload =
            HookPayload::new(HookEvent::Stop, session_id, working_dir.to_string_lossy());
        payload.stop_hook_active = Some(blocks_so_far > 0);
        payload.transcript_tail = transcript_tail;
        let aggregate = self
            .dispatch(HookEvent::Stop, None, &payload, working_dir)
            .await;
        match aggregate.decision {
            Some(HookDecision::Deny { reason }) => {
                let mut blocks = self.stop_blocks.lock().await;
                *blocks.entry(session_id.to_string()).or_insert(0) += 1;
                StopHookVerdict::Blocked { reason }
            }
            _ => {
                self.reset_stop_blocks(session_id).await;
                StopHookVerdict::Proceed
            }
        }
    }

    /// Reset the consecutive Stop-block counter (new turn or tools ran).
    pub async fn reset_stop_blocks(&self, session_id: &str) {
        self.stop_blocks.lock().await.remove(session_id);
    }

    /// Fire SessionStart hooks at most once per session per process.
    /// Returns None when already fired.
    pub async fn session_start_once(
        &self,
        session_id: &str,
        working_dir: &Path,
        source: &str,
    ) -> Option<HookAggregate> {
        {
            let mut started = self.sessions_started.lock().await;
            if !started.insert(session_id.to_string()) {
                return None;
            }
        }
        if !self
            .has_hooks(HookEvent::SessionStart, Some(source), working_dir)
            .await
        {
            return None;
        }
        let mut payload = HookPayload::new(
            HookEvent::SessionStart,
            session_id,
            working_dir.to_string_lossy(),
        );
        payload.source = Some(source.to_string());
        Some(
            self.dispatch(HookEvent::SessionStart, Some(source), &payload, working_dir)
                .await,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cwd() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    fn manager_with_yaml(yaml: &str) -> Arc<HooksManager> {
        let config: HooksConfig = serde_yaml::from_str(yaml).unwrap();
        Arc::new(HooksManager::with_config(
            config,
            false,
            Arc::new(Mutex::new(None)),
        ))
    }

    fn manager_with_yaml_commands(
        template: &str,
        commands: &[(&str, String)],
    ) -> Arc<HooksManager> {
        let yaml = commands
            .iter()
            .fold(template.to_string(), |yaml, (token, command)| {
                yaml.replace(token, &serde_json::to_string(command).unwrap())
            });
        manager_with_yaml(&yaml)
    }

    /// Issue #56 Gate H. `Agent::reply` fires the Stop hook with
    /// `transcript_tail(&conversation)` at the end of EVERY turn — real
    /// transcript text — and a prompt hook's `provider:`/`model:` pair names an
    /// endpoint the session row never records. Global hooks load from
    /// `config.yaml` unconditionally; project hooks from `.biorouter/hooks.yaml`
    /// when `allow_project_hooks` is set, and that file is agent-writable.
    /// Neither `Agent::update_provider` nor `Agent::reply` is on the path from
    /// the hook definition to `complete_fast`, so Gates A–F are blind to it.
    #[tokio::test]
    async fn a_prompt_hook_on_a_public_provider_is_skipped_for_a_private_session() {
        use crate::config::with_config_overrides;
        use crate::privacy::SessionClassification;
        use std::collections::HashMap;

        fn ollama_at(host: &str) -> HashMap<String, String> {
            HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())])
        }
        // `tier()` reads the base URL this instance resolved, so a host that is
        // not this machine is a real Public provider. `.invalid` never resolves,
        // so a completion that escaped the gate would fail rather than arrive.
        const OFF_MACHINE: &str = "https://api.example-saas.invalid";
        const THIS_MACHINE: &str = "http://localhost:11434";

        let manager = manager_with_yaml(
            r#"
Stop:
  - hooks:
      - type: prompt
        prompt: "Judge this."
        provider: "ollama"
        model: "qwen3"
"#,
        );
        let payload = HookPayload::new(HookEvent::Stop, "chat-1", test_cwd().to_string_lossy());

        manager.set_session_classification("chat-1", SessionClassification::Private);
        let refused = with_config_overrides(
            ollama_at(OFF_MACHINE),
            manager.dispatch(HookEvent::Stop, None, &payload, test_cwd()),
        )
        .await;
        assert!(
            refused
                .errors
                .iter()
                .any(|e| e.to_lowercase().contains("private")),
            "the skipped hook has to say why, in the outcome the user reads: {:?}",
            refused.errors
        );
        assert!(
            refused.decision.is_none(),
            "a hook that never ran cannot have decided anything: {:?}",
            refused.decision
        );

        // The gate discriminates. A private provider is still resolved for a
        // private chat — otherwise "refuse everything" would pass the assertion
        // above and prompt hooks would be quietly dead.
        //
        // A DISTINCT model name per case on purpose: `resolve_prompt_provider`
        // caches by (provider, model), so reusing `qwen3` here would answer from
        // the cache and assert nothing about the tier.
        assert!(
            with_config_overrides(
                ollama_at(THIS_MACHINE),
                manager.resolve_prompt_provider(
                    "chat-1",
                    Some("ollama".to_string()),
                    Some("qwen3-on-this-machine".to_string())
                ),
            )
            .await
            .is_ok(),
            "a private chat must still reach a private prompt-hook model"
        );
        // ...and a public chat is unaffected by the same public provider the
        // private one was refused. A DIFFERENT session id, not a mutation of
        // chat-1's: the store is per session, and this is the pair the bypass
        // below is about.
        manager.set_session_classification("chat-2", SessionClassification::Public);
        assert!(
            with_config_overrides(
                ollama_at(OFF_MACHINE),
                manager.resolve_prompt_provider(
                    "chat-2",
                    Some("ollama".to_string()),
                    Some("qwen3-off-machine".to_string())
                ),
            )
            .await
            .is_ok(),
            "a public chat must be unaffected"
        );

        // THE BYPASS. `biorouter web` shares one `Arc<Agent>` — and so one
        // `HooksManager` — across every chat it hosts, and spawns concurrent
        // turns from caller-supplied session ids. With a single classification
        // scalar, chat-2's public turn (just recorded above) overwrites chat-1's
        // `Private`, and chat-1's Stop hook — the one carrying `transcript_tail`
        // — then resolves the public provider. Ask for chat-1 again, AFTER the
        // public turn, and it must still be refused.
        //
        // It also pins the CACHE path: `qwen3-off-machine` is now in
        // `custom_providers`, so this resolve returns early from the cache. A
        // gate placed only on the fresh-create branch would protect the first
        // turn of a session and nothing after it.
        let refused = with_config_overrides(
            ollama_at(OFF_MACHINE),
            manager.resolve_prompt_provider(
                "chat-1",
                Some("ollama".to_string()),
                Some("qwen3-off-machine".to_string()),
            ),
        )
        .await
        .err()
        .expect("an overlapping public turn must not unlock a private chat's hooks");
        assert!(
            refused.to_lowercase().contains("private"),
            "the refusal has to say why, got: {refused}"
        );

        // A session nobody has classified is refused too — fail closed — but is
        // NOT told it is private, because no row was read. `SubagentStart` fires
        // on a freshly built child agent whose `reply` has not run, and
        // `SessionEnd` can fire on a session with no turn at all; calling those
        // "private" would be a claim of fact nobody checked.
        let unclassified = with_config_overrides(
            ollama_at(OFF_MACHINE),
            manager.resolve_prompt_provider(
                "chat-never-replied",
                Some("ollama".to_string()),
                Some("qwen3-off-machine".to_string()),
            ),
        )
        .await
        .err()
        .expect("an unclassified session must not reach a public prompt-hook provider");
        assert!(
            unclassified.contains("has stated whether it is private"),
            "an unclassified session must not be told it is private, got: {unclassified}"
        );
    }

    /// Issue #56 Gate H. The per-session classification store is bounded, and
    /// eviction must fall on the refusing side: an evicted session reads as
    /// unknown, and unknown is treated as `Private`.
    #[test]
    fn the_classification_store_is_bounded_and_evicts_towards_refusal() {
        use crate::privacy::SessionClassification;

        let manager =
            HooksManager::with_config(HooksConfig::default(), false, Arc::new(Mutex::new(None)));
        manager.set_session_classification("oldest", SessionClassification::Public);
        for i in 0..MAX_SESSION_CLASSIFICATIONS {
            manager.set_session_classification(&format!("chat-{i}"), SessionClassification::Public);
        }
        assert_eq!(
            manager.session_classifications.lock().unwrap().len(),
            MAX_SESSION_CLASSIFICATIONS,
            "the store must stay bounded"
        );
        assert_eq!(
            manager.session_classification("oldest"),
            None,
            "eviction is oldest-first, and the evicted entry must read as \
             unknown (which refuses) rather than as Public"
        );
        assert_eq!(
            manager.session_classification("chat-0"),
            Some(SessionClassification::Public),
            "only the overflow is evicted"
        );
        assert_eq!(
            manager.session_classification(&format!("chat-{}", MAX_SESSION_CLASSIFICATIONS - 1)),
            Some(SessionClassification::Public),
        );

        // Restating a session updates it in place rather than growing the store.
        manager.set_session_classification("chat-5", SessionClassification::Private);
        assert_eq!(
            manager.session_classification("chat-5"),
            Some(SessionClassification::Private)
        );
        assert_eq!(
            manager.session_classifications.lock().unwrap().len(),
            MAX_SESSION_CLASSIFICATIONS
        );
    }

    fn stdout_command(value: &str) -> String {
        if cfg!(target_os = "windows") {
            let escaped = value
                .replace('^', "^^")
                .replace('&', "^&")
                .replace('|', "^|")
                .replace('<', "^<")
                .replace('>', "^>");
            format!("echo {escaped}")
        } else {
            let quoted = value.replace('\'', "'\"'\"'");
            format!("printf '%s\\n' '{quoted}'")
        }
    }

    fn stderr_exit_two_command(reason: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("{} 1>&2 & exit /b 2", stdout_command(reason))
        } else {
            format!("{} >&2; exit 2", stdout_command(reason))
        }
    }

    fn exit_command(code: u8) -> String {
        if cfg!(target_os = "windows") {
            format!("exit /b {code}")
        } else {
            format!("exit {code}")
        }
    }

    fn delayed_stdout_command(value: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("ping -n 2 127.0.0.1 >NUL & {}", stdout_command(value))
        } else {
            format!("sleep 0.4; {}", stdout_command(value))
        }
    }

    fn stop_active_probe_command() -> String {
        let block = stdout_command(r#"{"decision":"block","reason":"first"}"#);
        if cfg!(target_os = "windows") {
            format!("findstr /C:false >NUL && {block}")
        } else {
            format!("if grep -q '\"stop_hook_active\":false' -; then {block}; fi")
        }
    }

    fn transcript_probe_command() -> String {
        let block = stdout_command(r#"{"decision":"block","reason":"saw tail"}"#);
        if cfg!(target_os = "windows") {
            format!("findstr /C:TAIL_MARKER >NUL && {block}")
        } else {
            format!("if grep -q 'TAIL_MARKER' -; then {block}; fi")
        }
    }

    #[tokio::test]
    async fn dispatch_runs_matching_hooks_only() {
        let manager = manager_with_yaml_commands(
            r#"
PreToolUse:
  - matcher: "developer__shell"
    hooks:
      - type: command
        command: $DENY
  - matcher: "other_tool"
    hooks:
      - type: command
        command: $UNMATCHED
"#,
            &[
                (
                    "$DENY",
                    stdout_command(
                        r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"shell blocked"}}"#,
                    ),
                ),
                ("$UNMATCHED", exit_command(2)),
            ],
        );
        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "developer__shell", &serde_json::json!({}))
            .await;
        assert!(aggregate.is_denied());
        assert_eq!(aggregate.deny_reason(), Some("shell blocked"));

        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "unmatched_tool", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());
    }

    // ---- BR-28: fire() aggregates are captured, not dropped ----

    fn notification_payload(session_id: &str) -> HookPayload {
        let mut payload = HookPayload::new(
            HookEvent::Notification,
            session_id,
            test_cwd().to_string_lossy(),
        );
        payload.message = Some("Permission required for developer__shell".to_string());
        payload
    }

    /// The whole point of BR-28: a `systemMessage` (and an error) from an
    /// observe-only, `fire`d event survives to the caller instead of being
    /// dropped with the detached task's aggregate.
    #[tokio::test]
    async fn fired_hook_aggregate_is_captured_and_settled() {
        let manager = manager_with_yaml_commands(
            r#"
Notification:
  - hooks:
      - type: command
        command: $MESSAGE
      - type: command
        command: "definitely-not-a-real-binary-biorouter"
"#,
            &[(
                "$MESSAGE",
                stdout_command(r#"{"systemMessage":"guard script ran"}"#),
            )],
        );

        manager.fire(
            HookEvent::Notification,
            Some("permission_prompt".to_string()),
            notification_payload("s1"),
            test_cwd().to_path_buf(),
        );

        let settled = manager.settle_fired("s1", FIRE_JOIN_BUDGET_SHUTDOWN).await;
        assert_eq!(settled.len(), 1);
        let outcome = &settled[0];
        assert_eq!(outcome.event, HookEvent::Notification);
        assert_eq!(outcome.session_id, "s1");
        assert_eq!(
            outcome.aggregate.system_messages,
            vec!["guard script ran".to_string()]
        );
        assert!(
            !outcome.aggregate.errors.is_empty(),
            "the failing hook's error must reach the caller too"
        );
        // Draining is destructive: the boundary consumed it.
        assert!(manager
            .settle_fired("s1", FIRE_JOIN_BUDGET)
            .await
            .is_empty());
    }

    /// Joining at a turn boundary is what stops a fired hook from outliving the
    /// turn, and a settled task is no longer registered as pending.
    #[tokio::test]
    async fn join_fired_awaits_outstanding_tasks() {
        let manager = manager_with_yaml_commands(
            r#"
Notification:
  - hooks:
      - type: command
        command: $MESSAGE
"#,
            &[("$MESSAGE", stdout_command(r#"{"systemMessage":"done"}"#))],
        );
        manager.fire(
            HookEvent::Notification,
            None,
            notification_payload("s1"),
            test_cwd().to_path_buf(),
        );
        assert_eq!(manager.pending_fire_count(), 1);

        assert_eq!(manager.join_fired(FIRE_JOIN_BUDGET_SHUTDOWN).await, 1);
        assert_eq!(manager.pending_fire_count(), 0);
        assert_eq!(manager.drain_fired("s1").len(), 1);
    }

    /// A slow hook must not stall the boundary: the join gives up on its budget,
    /// keeps the task registered, and the next boundary picks the outcome up.
    #[tokio::test]
    async fn slow_fired_hook_misses_its_budget_and_settles_later() {
        let manager = manager_with_yaml_commands(
            r#"
Notification:
  - hooks:
      - type: command
        command: $MESSAGE
"#,
            &[(
                "$MESSAGE",
                delayed_stdout_command(r#"{"systemMessage":"late but not lost"}"#),
            )],
        );
        manager.fire(
            HookEvent::Notification,
            None,
            notification_payload("s1"),
            test_cwd().to_path_buf(),
        );

        // First boundary: the hook is still running, so nothing is surfaced —
        // but the task is neither aborted nor forgotten.
        let early = manager.settle_fired("s1", Duration::from_millis(20)).await;
        assert!(early.is_empty());
        assert_eq!(manager.pending_fire_count(), 1);

        // A later boundary with a real budget settles it.
        let settled = manager.settle_fired("s1", FIRE_JOIN_BUDGET_SHUTDOWN).await;
        assert_eq!(
            settled
                .iter()
                .flat_map(|o| o.aggregate.system_messages.clone())
                .collect::<Vec<_>>(),
            vec!["late but not lost".to_string()]
        );
    }

    /// Outcomes are drained per session, so one session's turn boundary cannot
    /// swallow another's hook output (a `HooksManager` is shared across sessions).
    #[tokio::test]
    async fn fired_outcomes_drain_per_session() {
        let manager = manager_with_yaml_commands(
            r#"
Notification:
  - hooks:
      - type: command
        command: $MESSAGE
"#,
            &[("$MESSAGE", stdout_command(r#"{"systemMessage":"ping"}"#))],
        );
        for session in ["s1", "s2"] {
            manager.fire(
                HookEvent::Notification,
                None,
                notification_payload(session),
                test_cwd().to_path_buf(),
            );
        }
        manager.join_fired(FIRE_JOIN_BUDGET_SHUTDOWN).await;

        assert_eq!(manager.drain_fired("s1").len(), 1);
        assert!(manager.drain_fired("s1").is_empty());
        assert_eq!(manager.drain_fired("s2").len(), 1);
    }

    /// With no matching hook there is nothing to act on, so nothing is buffered
    /// — the common path stays allocation-free for the caller.
    #[tokio::test]
    async fn fire_with_no_matching_hook_buffers_nothing() {
        let manager = manager_with_yaml("{}");
        manager.fire(
            HookEvent::Notification,
            None,
            notification_payload("s1"),
            test_cwd().to_path_buf(),
        );
        assert!(manager
            .settle_fired("s1", FIRE_JOIN_BUDGET_SHUTDOWN)
            .await
            .is_empty());
    }

    // ---- BR-19: staged tool-hook effects + PostToolUse block cap ----

    fn staged(session_rewrite: Option<serde_json::Value>) -> StagedToolHook {
        StagedToolHook {
            event: HookEvent::PreToolUse,
            tool_request_id: "call_1".to_string(),
            tool_name: "developer__shell".to_string(),
            updated_input: session_rewrite,
            additional_context: vec!["the path was sandboxed".to_string()],
            system_messages: vec!["sandboxed a write".to_string()],
        }
    }

    /// The rewrite is taken *before* dispatch and the context *after* the
    /// permission gate has run, so taking one must not consume the other.
    #[test]
    fn taking_the_rewrite_leaves_the_context_for_the_later_drain() {
        let manager = manager_with_yaml("{}");
        manager.stage_tool_hook("s1", staged(Some(serde_json::json!({"path": "./safe"}))));

        let rewrites = manager.take_tool_input_rewrites("s1");
        assert_eq!(
            rewrites.get("call_1"),
            Some(&serde_json::json!({"path": "./safe"}))
        );
        // Taken once: a second take must not re-apply the same rewrite.
        assert!(manager.take_tool_input_rewrites("s1").is_empty());

        let drained = manager.drain_tool_hook_context("s1");
        assert_eq!(drained.len(), 1);
        assert_eq!(
            drained[0].additional_context,
            vec!["the path was sandboxed"]
        );
        assert_eq!(drained[0].system_messages, vec!["sandboxed a write"]);
        assert!(manager.drain_tool_hook_context("s1").is_empty());
    }

    /// A `HooksManager` is shared across sessions: one turn's drain must not
    /// swallow another session's staged effects.
    #[test]
    fn staged_effects_are_scoped_per_session() {
        let manager = manager_with_yaml("{}");
        manager.stage_tool_hook("s1", staged(None));
        manager.stage_tool_hook("s2", staged(None));
        assert_eq!(manager.drain_tool_hook_context("s1").len(), 1);
        assert_eq!(manager.drain_tool_hook_context("s2").len(), 1);
    }

    /// The staging buffer is keyed by session, so a caller that handles one tool
    /// call at a time while its siblings are in flight must be able to take only
    /// its own.
    ///
    /// The agent loop can use the session-wide take because it holds the model's
    /// whole batch of requests when it calls it. The coding-agent tool bridge
    /// cannot: it serves one request per HTTP handler with no serialization, and
    /// a session-wide take there removes rewrites keyed on request ids it has
    /// never seen and cannot apply — so those calls dispatch the arguments their
    /// PreToolUse hooks asked to replace, and nothing anywhere reports it.
    #[test]
    fn a_scoped_take_leaves_another_callers_rewrite_staged() {
        let manager = manager_with_yaml("{}");
        let mut mine = staged(Some(serde_json::json!({"path": "./mine"})));
        mine.tool_request_id = "mine".to_string();
        let mut theirs = staged(Some(serde_json::json!({"path": "./theirs"})));
        theirs.tool_request_id = "theirs".to_string();
        manager.stage_tool_hook("s1", mine);
        manager.stage_tool_hook("s1", theirs);

        let taken = manager.take_tool_input_rewrites_for("s1", &["mine".to_string()]);
        assert_eq!(taken.len(), 1, "only this caller's rewrite: {taken:?}");
        assert_eq!(
            taken.get("mine"),
            Some(&serde_json::json!({"path": "./mine"}))
        );

        let left = manager.take_tool_input_rewrites("s1");
        assert_eq!(
            left.get("theirs"),
            Some(&serde_json::json!({"path": "./theirs"})),
            "the other caller's rewrite must still be there for it to collect: {left:?}"
        );
    }

    /// The same scoping for the context drain, and for the same reason — a caller
    /// clearing up after its own call must not clear up after somebody else's.
    #[test]
    fn a_scoped_drain_leaves_another_callers_context_staged() {
        let manager = manager_with_yaml("{}");
        let mut mine = staged(None);
        mine.tool_request_id = "mine".to_string();
        let mut theirs = staged(None);
        theirs.tool_request_id = "theirs".to_string();
        manager.stage_tool_hook("s1", mine);
        manager.stage_tool_hook("s1", theirs);

        let drained = manager.drain_tool_hook_context_for("s1", &["mine".to_string()]);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].tool_request_id, "mine");

        let left = manager.drain_tool_hook_context("s1");
        assert_eq!(left.len(), 1, "the sibling's entry must survive: {left:?}");
        assert_eq!(left[0].tool_request_id, "theirs");
    }

    /// Draining the last entry removes the session's deque rather than leaving an
    /// empty one behind, so "no entries" and "no session" stay the same state.
    #[test]
    fn a_scoped_drain_of_everything_leaves_no_empty_session_entry() {
        let manager = manager_with_yaml("{}");
        manager.stage_tool_hook("s1", staged(None));
        assert_eq!(
            manager
                .drain_tool_hook_context_for("s1", &["call_1".to_string()])
                .len(),
            1
        );
        assert!(manager.drain_tool_hook_context("s1").is_empty());
        assert!(manager
            .drain_tool_hook_context_for("s1", &["call_1".to_string()])
            .is_empty());
    }

    /// A turn cancelled between staging and the drain must not grow the buffer
    /// without bound.
    #[test]
    fn staged_effects_are_bounded() {
        let manager = manager_with_yaml("{}");
        for _ in 0..(MAX_STAGED_TOOL_HOOKS + 10) {
            manager.stage_tool_hook("s1", staged(None));
        }
        assert_eq!(
            manager.drain_tool_hook_context("s1").len(),
            MAX_STAGED_TOOL_HOOKS
        );
    }

    /// A PostToolUse hook that blocks unconditionally would trap the turn in a
    /// block → retry → block loop; the cap releases it (same shape as the Stop
    /// hook's cap), and a clean result resets the counter.
    #[tokio::test]
    async fn post_tool_blocks_are_capped_and_reset() {
        let manager = manager_with_yaml("{}");
        for _ in 0..POST_TOOL_HOOK_BLOCK_CAP {
            assert!(manager.note_post_tool_block("s1").await);
        }
        assert_eq!(
            manager.post_tool_block_count("s1").await,
            POST_TOOL_HOOK_BLOCK_CAP
        );
        assert!(
            !manager.note_post_tool_block("s1").await,
            "past the cap the block must be overridden"
        );

        manager.reset_post_tool_blocks("s1").await;
        assert_eq!(manager.post_tool_block_count("s1").await, 0);
        assert!(manager.note_post_tool_block("s1").await);
    }

    /// A session that never settles must not grow the buffer without bound.
    #[test]
    fn fired_outcome_buffer_is_bounded() {
        let manager = manager_with_yaml("{}");
        for _ in 0..(MAX_FIRED_OUTCOMES + 8) {
            manager.record_fired(FiredHookOutcome {
                event: HookEvent::Notification,
                session_id: "orphan".to_string(),
                aggregate: HookAggregate {
                    system_messages: vec!["x".to_string()],
                    ..HookAggregate::default()
                },
            });
        }
        assert_eq!(manager.drain_fired("orphan").len(), MAX_FIRED_OUTCOMES);
    }

    // ---- BR-27: matching on tool_input content ----

    /// A guard rule can be scoped to dangerous *content* — only `rm -rf` shell
    /// commands pay for the hook, every other shell call skips it entirely.
    #[tokio::test]
    async fn input_matcher_narrows_a_group_to_matching_tool_input() {
        let manager = manager_with_yaml_commands(
            r#"
PreToolUse:
  - matcher: "developer__shell"
    input_matcher:
      command: "rm\\s+-rf"
    hooks:
      - type: command
        command: $DENY
"#,
            &[("$DENY", stderr_exit_two_command("no recursive deletes"))],
        );

        let denied = manager
            .pre_tool_use(
                "s1",
                test_cwd(),
                "developer__shell",
                &serde_json::json!({"command": "rm -rf /tmp/x"}),
            )
            .await;
        assert!(denied.is_denied());
        assert_eq!(denied.deny_reason(), Some("no recursive deletes"));

        // Same tool, harmless command: the hook never runs.
        let allowed = manager
            .pre_tool_use(
                "s1",
                test_cwd(),
                "developer__shell",
                &serde_json::json!({"command": "ls -la"}),
            )
            .await;
        assert!(allowed.decision.is_none());
        assert!(allowed.errors.is_empty());
    }

    /// The whole-input regex form, and the rule that a group with an
    /// `input_matcher` never fires on an event that carries no tool input.
    #[tokio::test]
    async fn whole_input_regex_form_and_no_input_events() {
        let manager = manager_with_yaml_commands(
            r#"
PreToolUse:
  - input_matcher: "/etc/"
    hooks:
      - type: command
        command: $DENY
Stop:
  - input_matcher: ".*"
    hooks:
      - type: command
        command: $STOP
"#,
            &[
                ("$DENY", stderr_exit_two_command("system path")),
                (
                    "$STOP",
                    stdout_command(r#"{"decision":"block","reason":"never"}"#),
                ),
            ],
        );

        let denied = manager
            .pre_tool_use(
                "s1",
                test_cwd(),
                "developer__text_editor",
                &serde_json::json!({"command": "write", "path": "/etc/hosts"}),
            )
            .await;
        assert!(denied.is_denied());

        let allowed = manager
            .pre_tool_use(
                "s1",
                test_cwd(),
                "developer__text_editor",
                &serde_json::json!({"command": "write", "path": "/home/me/notes.md"}),
            )
            .await;
        assert!(allowed.decision.is_none());

        // Stop carries no tool_input, so the input-matched group cannot fire.
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );
    }

    #[tokio::test]
    async fn multiple_hooks_merge_most_restrictive() {
        let manager = manager_with_yaml_commands(
            r#"
PreToolUse:
  - hooks:
      - type: command
        command: $ALLOW
      - type: command
        command: $DENY
"#,
            &[
                (
                    "$ALLOW",
                    stdout_command(r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#),
                ),
                ("$DENY", stderr_exit_two_command("blocked")),
            ],
        );
        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.is_denied());
        assert_eq!(aggregate.deny_reason(), Some("blocked"));
    }

    #[tokio::test]
    async fn failing_hook_is_failure_open() {
        let manager = manager_with_yaml_commands(
            r#"
PreToolUse:
  - hooks:
      - type: command
        command: $FAIL
      - type: command
        command: "definitely-not-a-real-binary-biorouter"
"#,
            &[("$FAIL", exit_command(1))],
        );
        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());
        assert!(!aggregate.errors.is_empty());
    }

    #[tokio::test]
    async fn stop_cap_enforced() {
        let manager = manager_with_yaml_commands(
            r#"
Stop:
  - hooks:
      - type: command
        command: $BLOCK
"#,
            &[(
                "$BLOCK",
                stdout_command(r#"{"decision":"block","reason":"keep going"}"#),
            )],
        );
        for _ in 0..STOP_HOOK_BLOCK_CAP {
            let verdict = manager.stop("s1", test_cwd(), None).await;
            assert_eq!(
                verdict,
                StopHookVerdict::Blocked {
                    reason: "keep going".to_string()
                }
            );
        }
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::CapReached
        );
        manager.reset_stop_blocks("s1").await;
        assert!(matches!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Blocked { .. }
        ));
    }

    #[tokio::test]
    async fn stop_without_hooks_proceeds() {
        let manager = manager_with_yaml("{}");
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );
    }

    #[tokio::test]
    async fn stop_hook_active_flag_set_on_second_block() {
        // Hook echoes back whether stop_hook_active was true by blocking
        // only when it is false — second call should then proceed.
        let manager = manager_with_yaml_commands(
            r#"
Stop:
  - hooks:
      - type: command
        command: $PROBE
"#,
            &[("$PROBE", stop_active_probe_command())],
        );
        assert!(matches!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Blocked { .. }
        ));
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );
    }

    #[tokio::test]
    async fn session_start_fires_once() {
        let manager = manager_with_yaml_commands(
            r#"
SessionStart:
  - hooks:
      - type: command
        command: $MESSAGE
"#,
            &[("$MESSAGE", stdout_command("remember the lab protocol"))],
        );
        let first = manager
            .session_start_once("s1", test_cwd(), "startup")
            .await;
        assert_eq!(
            first.unwrap().joined_context().as_deref(),
            Some("remember the lab protocol")
        );
        assert!(manager
            .session_start_once("s1", test_cwd(), "startup")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn session_hooks_merge_into_dispatch_for_their_session_only() {
        let manager = manager_with_yaml("{}");
        manager
            .set_session_hooks(
                "s1",
                HookEvent::PreToolUse,
                vec![HookDefinition::Command {
                    command: stderr_exit_two_command("blocked"),
                    timeout: None,
                }],
            )
            .await;

        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.is_denied());

        let aggregate = manager
            .pre_tool_use("s2", test_cwd(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());

        manager
            .clear_session_hooks("s1", HookEvent::PreToolUse)
            .await;
        assert!(!manager.has_session_hooks("s1", HookEvent::PreToolUse).await);
        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());
    }

    #[tokio::test]
    async fn stop_consults_session_hooks() {
        let manager = manager_with_yaml("{}");
        // No config hooks and no session hooks: proceed.
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );

        manager
            .set_session_hooks(
                "s1",
                HookEvent::Stop,
                vec![HookDefinition::Command {
                    command: stdout_command(r#"{"decision":"block","reason":"goal not met"}"#),
                    timeout: None,
                }],
            )
            .await;
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Blocked {
                reason: "goal not met".to_string()
            }
        );
        // Other sessions are unaffected.
        assert_eq!(
            manager.stop("other", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );

        manager.clear_session_hooks("s1", HookEvent::Stop).await;
        manager.reset_stop_blocks("s1").await;
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );
    }

    #[tokio::test]
    async fn stop_passes_transcript_tail_to_hooks() {
        // The hook blocks only when the transcript tail contains the marker,
        // proving the tail reaches hook stdin.
        let manager = manager_with_yaml_commands(
            r#"
Stop:
  - hooks:
      - type: command
        command: $PROBE
"#,
            &[("$PROBE", transcript_probe_command())],
        );
        assert_eq!(
            manager
                .stop("s1", test_cwd(), Some("TAIL_MARKER".to_string()))
                .await,
            StopHookVerdict::Blocked {
                reason: "saw tail".to_string()
            }
        );
        manager.reset_stop_blocks("s1").await;
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Proceed
        );
    }

    #[tokio::test]
    async fn project_hooks_require_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".biorouter")).unwrap();
        std::fs::write(
            dir.path().join(".biorouter/hooks.yaml"),
            "hooks:\n  PreToolUse:\n    - hooks: [{ type: command, command: \"exit 2\" }]\n",
        )
        .unwrap();

        let disabled = Arc::new(HooksManager::with_config(
            HooksConfig::default(),
            false,
            Arc::new(Mutex::new(None)),
        ));
        let aggregate = disabled
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());

        let enabled = Arc::new(HooksManager::with_config(
            HooksConfig::default(),
            true,
            Arc::new(Mutex::new(None)),
        ));
        let aggregate = enabled
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.is_denied());
    }

    #[tokio::test]
    async fn project_hooks_reload_on_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".biorouter")).unwrap();
        let path = dir.path().join(".biorouter/hooks.yaml");
        std::fs::write(&path, "hooks: {}\n").unwrap();

        let manager = Arc::new(HooksManager::with_config(
            HooksConfig::default(),
            true,
            Arc::new(Mutex::new(None)),
        ));
        let aggregate = manager
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());

        std::fs::write(
            &path,
            "hooks:\n  PreToolUse:\n    - hooks: [{ type: command, command: \"exit 2\" }]\n",
        )
        .unwrap();
        // Force a different mtime in case the filesystem clock is coarse.
        let new_time = std::time::SystemTime::now() + Duration::from_secs(2);
        let _ = filetime_set(&path, new_time);

        let aggregate = manager
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.is_denied());
    }

    fn filetime_set(path: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new().append(true).open(path)?;
        file.set_modified(time)
    }

    #[tokio::test]
    async fn prompt_hook_without_provider_fails_open() {
        let manager = manager_with_yaml(
            r#"
PreToolUse:
  - hooks:
      - type: prompt
        prompt: "Never allow anything."
"#,
        );
        let aggregate = manager
            .pre_tool_use("s1", test_cwd(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.decision.is_none());
        assert!(!aggregate.errors.is_empty());
    }

    // ---- BR-65: managed/enterprise hook tier ----

    fn managed_from_yaml(yaml: &str) -> Arc<crate::managed::ManagedPolicy> {
        let file: crate::managed::ManagedPolicyFile =
            serde_yaml::from_str(yaml).expect("managed yaml parses");
        Arc::new(crate::managed::ManagedPolicy::from_file(file))
    }

    /// A managed Stop hook that blocks cannot be cleared by a user Stop hook: the
    /// merge takes the most-restrictive decision, so a managed block wins.
    #[tokio::test]
    async fn managed_stop_hook_block_survives_user_hook() {
        let global_yaml = format!(
            "Stop:\n  - hooks: [{{ type: command, command: {} }}]\n",
            serde_json::to_string(&stdout_command("ok")).unwrap()
        );
        let global: HooksConfig = serde_yaml::from_str(&global_yaml).unwrap();
        let managed_yaml = format!(
            "hooks:\n  Stop:\n    - hooks:\n        - type: command\n          command: {}\n",
            serde_json::to_string(&stdout_command(
                r#"{"decision":"block","reason":"managed policy: finish the audit"}"#,
            ))
            .unwrap()
        );
        let managed = managed_from_yaml(&managed_yaml);
        let manager = Arc::new(HooksManager::with_config_and_managed(
            global,
            false,
            Arc::new(Mutex::new(None)),
            managed,
        ));
        assert_eq!(
            manager.stop("s1", test_cwd(), None).await,
            StopHookVerdict::Blocked {
                reason: "managed policy: finish the audit".to_string()
            }
        );
    }

    /// Managed hook groups resolve before global groups (admin tier first).
    #[tokio::test]
    async fn managed_hook_groups_resolve_before_global() {
        let global: HooksConfig = serde_yaml::from_str(
            "PreToolUse:\n  - hooks: [{ type: command, command: \"echo global\" }]\n",
        )
        .unwrap();
        let managed = managed_from_yaml(
            "hooks:\n  PreToolUse:\n    - hooks: [{ type: command, command: \"echo managed\" }]\n",
        );
        let manager = Arc::new(HooksManager::with_config_and_managed(
            global,
            false,
            Arc::new(Mutex::new(None)),
            managed,
        ));
        let groups = manager
            .resolved_groups(HookEvent::PreToolUse, test_cwd())
            .await;
        assert_eq!(groups.len(), 2);
        assert!(matches!(
            &groups[0].hooks[0],
            HookDefinition::Command { command, .. } if command == "echo managed"
        ));
        assert!(matches!(
            &groups[1].hooks[0],
            HookDefinition::Command { command, .. } if command == "echo global"
        ));
    }

    /// A managed `allow_project_hooks: false` override suppresses a present
    /// project hooks file even when the user opted in with `true`.
    #[tokio::test]
    async fn managed_forbids_project_hooks_over_user_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".biorouter")).unwrap();
        std::fs::write(
            dir.path().join(".biorouter/hooks.yaml"),
            "hooks:\n  PreToolUse:\n    - hooks: [{ type: command, command: \"exit 2\" }]\n",
        )
        .unwrap();

        // User opts in (true), but a managed override forbids project hooks.
        let managed = managed_from_yaml("allow_project_hooks: false\n");
        let forbidden = Arc::new(HooksManager::with_config_and_managed(
            HooksConfig::default(),
            true,
            Arc::new(Mutex::new(None)),
            managed,
        ));
        let aggregate = forbidden
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(
            aggregate.decision.is_none(),
            "managed override should suppress project hooks"
        );

        // Sanity: without the managed override, the same user opt-in runs them.
        let allowed = Arc::new(HooksManager::with_config_and_managed(
            HooksConfig::default(),
            true,
            Arc::new(Mutex::new(None)),
            Arc::new(crate::managed::ManagedPolicy::empty()),
        ));
        let aggregate = allowed
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(aggregate.is_denied());
    }

    /// A managed `allow_project_hooks: true` override forces project hooks on
    /// even when the user did not opt in.
    #[tokio::test]
    async fn managed_forces_project_hooks_over_user_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".biorouter")).unwrap();
        std::fs::write(
            dir.path().join(".biorouter/hooks.yaml"),
            "hooks:\n  PreToolUse:\n    - hooks: [{ type: command, command: \"exit 2\" }]\n",
        )
        .unwrap();

        let managed = managed_from_yaml("allow_project_hooks: true\n");
        let manager = Arc::new(HooksManager::with_config_and_managed(
            HooksConfig::default(),
            false, // user did NOT opt in
            Arc::new(Mutex::new(None)),
            managed,
        ));
        let aggregate = manager
            .pre_tool_use("s1", dir.path(), "any", &serde_json::json!({}))
            .await;
        assert!(
            aggregate.is_denied(),
            "managed true override should force project hooks on"
        );
    }

    /// A `config::with_config_overrides` entry is **invisible** to this module's
    /// project-hooks read, and that is not a subtlety — it was a live defect.
    ///
    /// `agents/subagent_tool.rs` inserted `BIOROUTER_ALLOW_PROJECT_HOOKS` into an
    /// override map to unlock project hooks for one arm of a refusal table. The
    /// task-local is consulted by `Config::get_param` (`config/base.rs`), and
    /// `HooksManager::new_with_managed` reads the flag with a bare
    /// `std::env::var` — so the arm was unlocked only when some *other* test had
    /// leaked the variable into the process, and it passed either way because the
    /// refusal it asserts holds in both states. The unlock is now stated on
    /// `AgentConfig::allow_project_hooks` instead.
    ///
    /// Written as a before/inside comparison rather than an `is_err()` assertion
    /// so it does not depend on the ambient environment of the machine running
    /// it. The other half of the contract — that an override DOES reach a
    /// `get_param` reader — is `config::base::tests::task_local_override_wins_over_env`.
    #[tokio::test]
    async fn a_config_override_never_reaches_the_project_hooks_env_read() {
        const KEY: &str = "BIOROUTER_ALLOW_PROJECT_HOOKS";
        let before = std::env::var(KEY).ok();
        let inside = crate::config::with_config_overrides(
            std::collections::HashMap::from([(KEY.to_string(), "true".to_string())]),
            async { std::env::var(KEY).ok() },
        )
        .await;
        assert_eq!(
            before, inside,
            "a task-local config override must not change what a bare `std::env::var` \
             reader sees; if this ever passes by accident, check that nothing in the \
             binary is writing {KEY}"
        );
    }
}
