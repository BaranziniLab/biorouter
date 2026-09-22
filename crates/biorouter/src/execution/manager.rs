use crate::agents::{Agent, AgentConfig};
use crate::config::paths::Paths;
use crate::config::permission::PermissionManager;
use crate::config::{BioRouterMode, Config};
use crate::scheduler::Scheduler;
use crate::scheduler_trait::SchedulerTrait;
use crate::session::SessionManager;
use anyhow::Result;
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OnceCell, RwLock};
use tracing::{debug, info};

const DEFAULT_MAX_SESSION: usize = 100;

static AGENT_MANAGER: OnceCell<Arc<AgentManager>> = OnceCell::const_new();

/// The config root the process-global manager ([`AgentManager::instance`])
/// seeds into: `Paths::config_dir()` — an environment read — taken the first
/// time anything asks, and never again.
///
/// The config-side twin of `SessionManager`'s `SHARED_STORE_ROOT`. Caching the
/// answer is not what protects a test binary; *when* the first ask happens is.
/// Two test ctors make it happen before `main`, before any test can relocate
/// `BIOROUTER_PATH_ROOT`: this crate's lib binary (`src/test_sandbox.rs`) and
/// `biorouter-server`'s (its own `src/test_sandbox.rs`, which its integration
/// binaries `#[path]`-include). Any other binary — `biorouter-cli`'s tests
/// among them, whose ctor freezes the session store and `Config::global()` but
/// not this — resolves it at whichever instant first calls `instance()`, and
/// if a test is holding `BIOROUTER_PATH_ROOT` on a `TempDir` of its own then,
/// the global manager seeds into that `TempDir` for the life of the process;
/// see [`AgentManager::new`].
///
/// In the daemon nothing moves `BIOROUTER_PATH_ROOT`, so this is the answer
/// `Paths::config_dir()` gives at `instance()`'s first call — the value `new`
/// used to read for itself.
///
/// A `OnceLock` rather than a `LazyLock` only so that
/// [`AgentManager::shared_config_root_if_resolved`] can look without resolving
/// (see `SHARED_STORE_ROOT` for why a guard needs that); both resolve the
/// same value at the same first call.
static SHARED_CONFIG_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// One pinned agent and how many concurrent runs are holding it.
struct PinnedAgent {
    agent: Arc<Agent>,
    runs: usize,
}

pub struct AgentManager {
    sessions: Arc<RwLock<LruCache<String, Arc<Agent>>>>,
    /// BR-71 decision 10: agents that must NOT be evicted while they run —
    /// glass-box subagents (Task 33) and consulted Agent Drafter workers
    /// (Task 41). The LRU is a memory bound for *idle* agents; an agent with a
    /// live turn is not idle, and evicting it would restore the very bug
    /// `register_agent` exists to fix.
    pinned: Arc<RwLock<HashMap<String, PinnedAgent>>>,
    /// Concrete, not `dyn SchedulerTrait`, so the daemon can start the one
    /// thing only the concrete scheduler can do — watch its file
    /// ([`Self::watch_schedule_file`]). Everyone else gets the trait object from
    /// [`Self::scheduler`].
    scheduler: Arc<Scheduler>,
    session_manager: Arc<SessionManager>,
    default_provider: Arc<RwLock<Option<Arc<dyn crate::providers::base::Provider>>>>,
}

impl AgentManager {
    /// Build a manager whose first-run seeding writes into `config_dir` — with
    /// one known exception, the scheduler's copy of the Meditation workflow,
    /// which follows the ambient data dir (last paragraph below).
    ///
    /// ⚠ **The config root is a PARAMETER, and no caller may leave it to the
    /// environment.** Everything this constructor seeds — the Soul KB, the
    /// built-in skills, the update-soul skill, the Meditation workflow — is
    /// written under `config_dir`, and the seeding is *spawned* (BR-55), so it
    /// lands at an arbitrary point after `new` returns. `BIOROUTER_PATH_ROOT`
    /// is process-global and the lib test binary runs tests on parallel
    /// threads, so any root resolved from it at an instant the caller does not
    /// own belongs to whichever unrelated test holds `env_lock` then.
    ///
    /// That has now bitten twice, one layer apart:
    ///
    /// 1. The seeders resolved `Paths::config_dir()` themselves, inside the
    ///    spawned task. `knowledge::conversation_ingest::tests::missing_or_\
    ///    disabled_soul_skill_fails_before_raw_staging` — whose entire
    ///    assertion is that no `update-soul` skill exists in the temp root it
    ///    owns — was handed one by a manager it never built (CI
    ///    `test (ubuntu-latest)`, PR #191 run 34304297956). The fix moved the
    ///    read into this constructor and threaded it to the seeders.
    /// 2. That left the read here, and here was still unowned for every caller
    ///    that did not hold `env_lock` while constructing — the test module's
    ///    own `create_test_manager` and a `subagent_handler` test.
    ///    `first_run_seeding_lands_in_the_root_the_manager_was_built_with`
    ///    failed with its own message, *"followed the ambient
    ///    BIOROUTER_PATH_ROOT into /tmp/.tmpVnx6I7"* (CI `test (ubuntu-latest)`,
    ///    rust.yml run 35480469489 attempt 1, job 105997233573, 2026-09-20,
    ///    re-run green). Its own manager was innocent: while it held the
    ///    environment on a root it had given nobody, a sibling test running on
    ///    another thread (the log shows `deregistering_…`, `has_session_…`,
    ///    `peek_agent_…` and `overlapping_…` finishing beside it, all built by
    ///    `create_test_manager`) called `new`, read that root as "the root I
    ///    was built with", and — correctly by its own contract — seeded an
    ///    `update-soul` skill into it. Reproduced locally on the pre-fix tree
    ///    (macOS, 16 cores): 1 of 256 runs of `execution::manager` under 8-way
    ///    concurrent load, same message, same line; 0 of 60 without the load.
    ///    That rarity is why a re-run went green, and why
    ///    `create_test_manager_seeds_its_own_root_while_another_test_holds_\
    ///    the_environment` now forces the interleaving instead of waiting for
    ///    it.
    ///
    /// So `new` no longer reads the environment for its config root, and the
    /// caller that owns the root passes it: a test passes a directory of its
    /// own, and [`Self::instance`] passes [`Self::shared_config_root`]. That
    /// closes the read for every manager built with an explicit root — every
    /// `AgentManager::new` in this crate's tests. It does not make the global
    /// manager environment-free:
    ///
    /// - `shared_config_root()` IS `Paths::config_dir()`, taken once at its
    ///   first ask. Only a binary whose ctor asks before `main` (this crate's
    ///   lib tests and `biorouter-server`'s; not `biorouter-cli`'s) is
    ///   protected; elsewhere the first `instance()` still decides.
    /// - `instance()` reads `Config::global()` for the session cap, and
    ///   `get_or_create_agent` reads it for the mode and takes
    ///   `PermissionManager::instance()` — two more one-shot cells resolved
    ///   from `BIOROUTER_PATH_ROOT` at *their* first touch. The lib test ctor
    ///   freezes `Config::global()` (so do the CLI's); nothing freezes
    ///   `PERMISSION_MANAGER`, whose constructor reads and creates files.
    /// - the spawned init's scheduler workflow copy, below.
    ///
    /// Neither occurrence was a production bug: outside tests a process builds
    /// exactly one manager, through `instance()`, and nothing moves
    /// `BIOROUTER_PATH_ROOT` while it runs, so the root read at construction
    /// always was the root seeded. The second made a correct detector read as a flake, and a
    /// detector that gets re-run green catches nothing.
    ///
    /// ⚠ Not the same as "the spawned init reads no environment". At least one
    /// ambient read remains on it, and it is the scheduler's: `ensure_meditation_\
    /// schedule` calls `add_scheduled_job(make_copy: true)`, which copies the
    /// Meditation workflow into `get_default_scheduled_workflows_dir()` —
    /// `Paths::data_dir()` at that instant. Measured 2026-09-21 with the
    /// environment held on a root this manager was not given:
    /// `data/scheduled_workflows/daily-meditation.yaml`, and the job's stored
    /// `source`, both land in that root. In the daemon it is the same data dir
    /// `schedule.json` lives in, so production is consistent; in a test binary
    /// it is this family of unowned read, and
    /// `first_run_seeding_lands_in_the_root_the_manager_was_built_with` does
    /// not look at it. Closing it means giving the scheduler a store of its
    /// own, which the owns-source containment check in `scheduler.rs` also
    /// depends on.
    pub async fn new(
        session_manager: Arc<SessionManager>,
        schedule_file_path: PathBuf,
        config_dir: PathBuf,
        max_sessions: Option<usize>,
    ) -> Result<Self> {
        let scheduler = Scheduler::new(schedule_file_path, session_manager.clone()).await?;

        // Runs to completion before this constructor returns, so on the success
        // path no client ever observes a pre-OKF base or a store without its
        // built-in OKF Soul. It deliberately does NOT gate construction: a
        // daemon that refuses to build `AppState` has no HTTP listener, no GUI
        // and no way for the user to reach the store and repair whatever the
        // message names — strictly worse than a degraded Knowledge surface, and
        // the reconciliation is retried on every startup and every
        // `biorouter knowledge` command anyway.
        let reconcile_root = config_dir.clone();
        match tokio::task::spawn_blocking(move || {
            crate::knowledge::soul::ensure_soul_kb(&reconcile_root)
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!("failed to reconcile the Soul knowledge base: {error:#}");
            }
            Err(error) => tracing::error!("Soul knowledge reconciliation task failed: {error}"),
        }

        let capacity = NonZeroUsize::new(max_sessions.unwrap_or(DEFAULT_MAX_SESSION))
            .unwrap_or_else(|| NonZeroUsize::new(100).unwrap());

        let manager = Self {
            sessions: Arc::new(RwLock::new(LruCache::new(capacity))),
            pinned: Arc::new(RwLock::new(HashMap::new())),
            scheduler,
            session_manager,
            default_provider: Arc::new(RwLock::new(None)),
        };

        // BR-55 keeps skill/workflow/schedule seeding off the listener's hot
        // path. Knowledge reconciliation is the deliberate exception above: it
        // runs before this point so that clients never observe the pre-OKF bases
        // startup is purging, or an empty store before its built-in OKF Soul
        // exists.
        let scheduler = manager.scheduler();
        if std::env::var_os("BIOROUTER_BLOCKING_STARTUP").is_some() {
            Self::run_first_run_init(scheduler, config_dir).await;
        } else {
            tokio::spawn(Self::run_first_run_init(scheduler, config_dir));
        }

        Ok(manager)
    }

    /// First-run install of the built-in skills, the Soul KB, its Meditation
    /// workflow + update-soul skill, and the Daily Meditation 3:00 AM schedule.
    ///
    /// Every step is idempotent and best-effort (each logs a warning on failure
    /// and never returns an error), and none of it is required to serve a
    /// request, so [`AgentManager::new`] runs it in the background (BR-55). The
    /// synchronous skills seeding is blocking file I/O, so it goes through
    /// `spawn_blocking` to keep it off the async runtime.
    ///
    /// ⚠ `config_dir` is **passed in**, never resolved here. This function runs
    /// detached from the constructor that scheduled it, so a `Paths::config_dir()`
    /// call in this body reads whatever the process environment says at some
    /// later, unowned instant — see the note in [`AgentManager::new`].
    async fn run_first_run_init(
        scheduler: Arc<dyn SchedulerTrait>,
        config_dir: std::path::PathBuf,
    ) {
        let skills_dir = crate::agents::skills_extension::skills_root(&config_dir);
        if let Err(e) = tokio::task::spawn_blocking(move || {
            crate::agents::skills_extension::install_builtin_skills(&skills_dir)
        })
        .await
        {
            tracing::warn!("Failed to seed built-in skills: {e}");
        }
        crate::knowledge::soul::install(&config_dir, &scheduler).await;
    }

    pub async fn instance() -> Result<Arc<Self>> {
        AGENT_MANAGER
            .get_or_try_init(|| async {
                let max_sessions = Config::global()
                    .get_biorouter_max_active_agents()
                    .unwrap_or(DEFAULT_MAX_SESSION);
                // The SAME data directory the process-global session store uses,
                // asked for once rather than resolved a second time. In the
                // daemon the two reads agree — nothing moves
                // `BIOROUTER_PATH_ROOT` while it runs — so this is a no-op
                // there. In a test binary a second read of a process-global
                // variable at an instant nobody owns is the bug documented in
                // `AgentManager::new` below (PR #191) and the one that made
                // `sessions.db` land in a `TempDir` a sibling test deleted; see
                // `SHARED_STORE_ROOT` in `session_manager.rs`. `schedule.json`
                // and `sessions/sessions.db` are meant to be siblings, so taking
                // the store's own answer is also the more honest statement.
                let schedule_file_path = SessionManager::shared_store_root().join("schedule.json");
                // The config root likewise comes from a once-resolved cell rather
                // than a fresh `Paths::config_dir()`. The cell is itself that read,
                // taken at its first ask, so it only protects a test binary whose
                // ctor asks before `main` — see `SHARED_CONFIG_ROOT`.
                let config_dir = Self::shared_config_root().to_path_buf();
                let session_manager = Arc::new(SessionManager::instance());
                let manager = Self::new(
                    session_manager,
                    schedule_file_path,
                    config_dir,
                    Some(max_sessions),
                )
                .await?;
                Ok(Arc::new(manager))
            })
            .await
            .cloned()
    }

    /// The config root [`Self::instance`] seeds into ([`SHARED_CONFIG_ROOT`]).
    ///
    /// Reading it is what freezes it, which is why the `biorouter` lib and
    /// `biorouter-server` test ctors call it before any test can relocate
    /// `BIOROUTER_PATH_ROOT`. It is one environment read and a `PathBuf` — no
    /// disk, no runtime — so it is safe before `main`. In production it is a
    /// plain accessor.
    pub fn shared_config_root() -> &'static Path {
        SHARED_CONFIG_ROOT.get_or_init(Paths::config_dir)
    }

    /// [`Self::shared_config_root`] if something has already resolved it, and
    /// `None` otherwise — **without** resolving it.
    ///
    /// For the test ctors' guards; production never calls it. A guard that
    /// calls `shared_config_root()` to learn whether the ctor froze it resolves
    /// it itself when the freeze is missing — while the ctor's sandbox is still
    /// the ambient root — and passes. This read is how it tells the two apart.
    pub fn shared_config_root_if_resolved() -> Option<&'static Path> {
        SHARED_CONFIG_ROOT.get().map(PathBuf::as_path)
    }

    pub fn scheduler(&self) -> Arc<dyn SchedulerTrait> {
        Arc::clone(&self.scheduler) as Arc<dyn SchedulerTrait>
    }

    /// Keep this manager's scheduler in step with changes other processes make
    /// to the schedule file (QA 2026-09-10, F2).
    ///
    /// For the daemon to call once at startup, beside its `config.yaml` watcher —
    /// and deliberately not called from [`Self::new`]: a manager is also built in
    /// other processes (a terminal session that runs a subagent builds one), and
    /// only the long-lived daemon should be adopting jobs other processes add.
    pub fn watch_schedule_file(&self) {
        self.scheduler.spawn_file_watcher();
    }

    /// Get the shared SessionManager for session-only operations
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    pub async fn set_default_provider(&self, provider: Arc<dyn crate::providers::base::Provider>) {
        debug!("Setting default provider on AgentManager");
        *self.default_provider.write().await = Some(provider);
    }

    /// BR-71: put an externally-built, fully-configured agent (a glass-box
    /// subagent, or a consulted Agent Drafter worker) into the registry under
    /// its session id, so every server resolution path — `POST /interrupt`,
    /// `POST /reply`, workspace steer — returns the LIVE instance instead of
    /// minting a default agent that no running loop drains.
    ///
    /// **It SHADOWS a racing placeholder; it does not replace one.** This is a
    /// correction (2026-07-31 review): the comment here used to say "overwrites
    /// any placeholder entry an early racing resolution created", and no such
    /// mechanism exists. [`Self::get_or_create_agent`] consults the pin, drops
    /// that guard, and only then reads the cache — so a resolution landing
    /// between the run's `begin_turn` and its `register_agent` mints a bare
    /// agent and `put`s it in the LRU under the same id. Nothing here evicts it.
    /// The pin outranks it for every read while the run lasts (which is the
    /// window this whole API is about), and it resurfaces once the pin goes.
    ///
    /// Both halves of that are deliberate, and the tests say so
    /// (`a_registration_shadows_a_racing_placeholder_it_does_not_replace_it`):
    ///
    /// - Not evicting is *required*. From in here a placeholder is
    ///   indistinguishable from the entry a consulted Agent Drafter worker got
    ///   from an ordinary `get_agent` (`routes/apps.rs:1663`), which
    ///   `deregistering_does_not_evict_a_cache_entry_it_did_not_create` requires
    ///   survive — evicting would nuke a cached worker on every consult.
    /// - Leaving it is *harmless*. A placeholder is exactly what
    ///   `get_or_create_agent` would have produced for that id anyway (same
    ///   constructor, same default provider), so a post-run resolution is no
    ///   worse off than if the run had never registered at all.
    ///
    /// **Pinned out of the LRU** (decision 10). The `sessions` cache holds 100
    /// agents and evicts the least-recently-used; a registered child is
    /// *running*, and evicting it would silently restore the pre-BR-71 bug —
    /// a steer would mint a fresh agent that no loop drains. The pin is a plain
    /// `HashMap` sidecar consulted before the cache, so a pinned entry cannot
    /// be evicted by any amount of unrelated agent creation.
    pub async fn register_agent(&self, session_id: String, agent: Arc<Agent>) {
        let mut pinned = self.pinned.write().await;
        match pinned.get_mut(&session_id) {
            // REFCOUNTED, not overwritten. Two runs can legitimately register
            // the same `Arc` back to back — a durable Agent Drafter worker
            // consulted twice in quick succession does exactly this (Task 41),
            // because `build_worker` reuses its cached `WorkerHandle.agent`. If
            // the second registration merely overwrote, the FIRST run's
            // deregistration — which is `tokio::spawn`ed and can land after the
            // second has begun — would see `Arc::ptr_eq` match and remove a LIVE
            // registration mid-turn. "Only clear your own" guards against a
            // different successor; it does not guard against the same handle
            // registered again.
            Some(entry) if Arc::ptr_eq(&entry.agent, &agent) => entry.runs += 1,
            _ => {
                pinned.insert(session_id, PinnedAgent { agent, runs: 1 });
            }
        }
    }

    /// Release ONE registration of `session_id` → `agent`, and unpin only when
    /// the last one goes. The `Arc::ptr_eq` test is the TurnGuard discipline
    /// (`impl TurnGuard` / `impl Drop for TurnGuard`, `state.rs:65-98`): a
    /// finished run may only clear its own registration, never a successor's.
    ///
    /// Note what this deliberately does NOT do: it does not touch the `sessions`
    /// LRU. `register_agent` does not put anything there either, so there is
    /// nothing of ours to remove — and an entry that IS there was put there by
    /// an ordinary `get_or_create_agent`, which is how a consulted Agent Drafter
    /// worker gets its agent (`routes/apps.rs:1663`). Popping it would evict a
    /// cached worker this run never created, on every consult.
    pub async fn deregister_agent_if_same(&self, session_id: &str, agent: &Arc<Agent>) {
        let mut pinned = self.pinned.write().await;
        let Some(entry) = pinned.get_mut(session_id) else {
            return;
        };
        if !Arc::ptr_eq(&entry.agent, agent) {
            return;
        }
        entry.runs -= 1;
        if entry.runs == 0 {
            pinned.remove(session_id);
        }
    }

    pub fn new_scoped_agent(&self) -> Arc<Agent> {
        let config = AgentConfig::new(
            Arc::clone(&self.session_manager),
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        )
        .with_project_hooks(false);
        Arc::new(Agent::with_config(config))
    }

    pub async fn get_or_create_agent(&self, session_id: String) -> Result<Arc<Agent>> {
        // BR-71: a pinned (running, externally-built) agent always wins — it is
        // the instance whose loop drains the soft-interrupt queue.
        if let Some(entry) = self.pinned.read().await.get(&session_id) {
            return Ok(Arc::clone(&entry.agent));
        }
        {
            let mut sessions = self.sessions.write().await;
            if let Some(existing) = sessions.get(&session_id) {
                return Ok(Arc::clone(existing));
            }
        }

        let mode = Config::global()
            .get_biorouter_mode()
            .unwrap_or(BioRouterMode::Auto);
        let permission_manager = PermissionManager::instance();
        let config = AgentConfig::new(
            Arc::clone(&self.session_manager),
            permission_manager,
            Some(self.scheduler()),
            mode,
        );
        let agent = Arc::new(Agent::with_config(config));
        if let Some(provider) = &*self.default_provider.read().await {
            agent
                .update_provider(Arc::clone(provider), &session_id)
                .await?;
        }

        let mut sessions = self.sessions.write().await;
        if let Some(existing) = sessions.get(&session_id) {
            Ok(Arc::clone(existing))
        } else {
            sessions.put(session_id, agent.clone());
            Ok(agent)
        }
    }

    /// Look up a live agent WITHOUT creating one. `get_or_create_agent` reads
    /// the process-wide mode at creation time, so using it to *inspect* a
    /// target's mode reads today's global config and then leaves a bare,
    /// provider-less, extension-less agent cached under that session id.
    ///
    /// `sessions` is an LRU, so reading it needs the write lock (`get` promotes
    /// the entry) — the same call `get_or_create_agent`'s hit path makes.
    ///
    /// BR-71: the PINNED sidecar is consulted first, for the same reason
    /// `get_or_create_agent` consults it — a registered, running agent is the
    /// live instance and is never in the LRU. Without this, a glass-box child
    /// mid-run peeks as "no live agent" and `workspace_send_prompt` reads its
    /// mode off nothing.
    pub async fn peek_agent(&self, session_id: &str) -> Option<Arc<Agent>> {
        if let Some(entry) = self.pinned.read().await.get(session_id) {
            return Some(Arc::clone(&entry.agent));
        }
        self.sessions.write().await.get(session_id).map(Arc::clone)
    }

    /// Evict a session from both halves of the registry.
    ///
    /// The unpin is **unconditional and discards the refcount**: an explicit
    /// stop (`POST /agent/stop`, `workspace_close scope:"agent"`) outranks any
    /// number of live registrations, so a session with `runs: 2` goes away in
    /// one call rather than needing two. Both of the stopped run's outstanding
    /// deregistrations then find no entry, or a successor's, and are no-ops —
    /// `a_stale_deregistration_after_a_stop_cannot_clear_a_successor` is what
    /// keeps that true.
    ///
    /// ⚠ **One shape this does not cover, for whoever writes Task 41.** Because
    /// the count is discarded rather than remembered, an agent re-registered
    /// under the SAME id after a stop starts again at `runs: 1` while the
    /// stopped run's releases are still in flight — and those releases identify
    /// their registration by `Arc` pointer alone. If the re-registered handle is
    /// the same `Arc`, the first stale release matches and takes 1 → 0, unpinning
    /// a *live* agent mid-turn. That needs all three of: overlapping runs on one
    /// id, an explicit stop, and the same `Arc` re-registered before the releases
    /// land. Unreachable from the subagent path, which mints a fresh session id
    /// per run and so never re-registers under a stopped one — but it is exactly
    /// the durable-worker shape the refcount was written for, where
    /// `build_worker` hands back its cached `WorkerHandle.agent`. A consult that
    /// can follow a stop on the same id must not reuse the cached handle (or the
    /// pin needs an identity finer than the pointer).
    pub async fn remove_session(&self, session_id: &str) -> Result<()> {
        let was_pinned = self.pinned.write().await.remove(session_id).is_some();
        let mut sessions = self.sessions.write().await;
        // "Not found" must mean NEITHER half knew the session. A registered
        // child lives only in the pin, so testing the LRU alone would report a
        // successful stop as a 404 (`POST /agent/stop`) or as a tool failure
        // (`workspace_close scope:"agent"` → `ServerWorkspaceServices::stop_agent`).
        if sessions.pop(session_id).is_none() && !was_pinned {
            return Err(anyhow::anyhow!("Session {} not found", session_id));
        }
        info!("Removed session {}", session_id);
        Ok(())
    }

    pub async fn clear_sessions(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let count = sessions.len();
        sessions.clear();
        count
    }

    pub async fn has_session(&self, session_id: &str) -> bool {
        // BR-71: a pinned (registered, running) agent is live even though it was
        // never put in the LRU — see `register_agent`. Without this line
        // `workspace_list` reports `live: false` for every glass-box subagent,
        // and in the HEADLESS configuration (no daemon, so `running` is false
        // for every row and there is no GUI tab either) the default
        // `scope: "open"` returns an empty list for the whole workspace —
        // exactly the configuration decision 21 exists to preserve.
        //
        // **This does NOT hold the pin guard across the LRU acquisition**, and
        // the point is worth writing down because it reads as if it might: a
        // reviewer called it an undocumented `pinned -> sessions` lock order.
        // Both operands of a lazy boolean are their own temporary scope — `a ||
        // b` is `if a { true } else { b }`, and an `if` condition is a scope —
        // so the guard from the left operand is dropped BEFORE `self.sessions`
        // is even touched. (Minimal check, edition 2021: with a `Drop` type in
        // the left operand, "drop" prints before the right operand runs.)
        //
        // So no path in this file ever holds two of these guards at once —
        // `get_or_create_agent` and `peek_agent` drop theirs at the end of the
        // `if let`, `remove_session` at the end of its `let` — which is what
        // makes a future `sessions -> pinned` path safe instead of a deadlock.
        // The invariant is "one guard at a time", not "always this order", and
        // `has_session_does_not_hold_the_pin_lock_while_it_waits` is what keeps
        // it from being re-argued from first principles: it goes red for any
        // rewrite that does hold the pin across the await.
        self.pinned.read().await.contains_key(session_id)
            || self.sessions.read().await.contains(session_id)
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::execution::SessionExecutionMode;
    use crate::session::SessionManager;

    use super::AgentManager;

    /// A manager that seeds into `temp_dir` and nowhere else.
    ///
    /// ⚠ The config root is `temp_dir`'s own, never `Paths::config_dir()`. These
    /// tests do not hold `env_lock`, so an ambient read here resolves to
    /// whichever test is relocating `BIOROUTER_PATH_ROOT` at that instant — and
    /// this manager's spawned first-run init then seeds an `update-soul` skill
    /// into that test's root. That is exactly how
    /// `first_run_seeding_lands_in_the_root_the_manager_was_built_with` failed
    /// in CI run 35480469489 (see `AgentManager::new`).
    async fn create_test_manager(temp_dir: &TempDir) -> AgentManager {
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let schedule_path = temp_dir.path().join("schedule.json");
        AgentManager::new(
            session_manager,
            schedule_path,
            temp_dir.path().join("config"),
            Some(100),
        )
        .await
        .unwrap()
    }

    /// Where a manager built on `root/config` puts the `update-soul` skill —
    /// the file whose appearance in a root nobody gave out is the flake.
    fn soul_skill(root: &std::path::Path) -> std::path::PathBuf {
        root.join("config")
            .join("skills")
            .join(crate::agents::skills_extension::KNOWLEDGE_BUNDLE)
            .join(crate::knowledge::soul::SOUL_SKILL_DIR)
            .join("SKILL.md")
    }

    /// BR-55: `new` spawns `run_first_run_init` in the background, so a manager
    /// must be fully usable (scheduler present, agents creatable) the instant
    /// `new` returns — without waiting for skills/Soul install. It must also be
    /// panic-free and idempotent, since a second Biorouter process runs it too.
    #[tokio::test]
    async fn test_manager_usable_before_first_run_init_completes() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        // The scheduler (a required field, built synchronously) is available and
        // the manager can create an agent immediately after `new` returns.
        let _scheduler = manager.scheduler();
        let session = uuid::Uuid::new_v4().to_string();
        manager.get_or_create_agent(session.clone()).await.unwrap();
        assert!(manager.has_session(&session).await);

        // Running the deferred init directly must be panic-free and idempotent
        // (best-effort: every step logs a warning on failure rather than erroring).
        let config_dir = temp_dir.path().join("config");
        AgentManager::run_first_run_init(manager.scheduler(), config_dir.clone()).await;
        AgentManager::run_first_run_init(manager.scheduler(), config_dir).await;
    }

    /// BR-71: `peek_agent` is a LOOKUP. Its whole reason to exist is that
    /// `get_or_create_agent` cannot be used to *inspect* a session — its miss
    /// path reads today's process-wide `biorouter_mode` and then caches a bare,
    /// provider-less, extension-less agent under that id, which the turn runner
    /// will happily pick up. `workspace_send_prompt mode:"turn"` asks this
    /// question about targets the user has not opened, so a `peek_agent` that
    /// quietly delegated to `get_or_create_agent` would mint an agent for every
    /// one of them while still answering "found".
    #[tokio::test]
    async fn peek_agent_finds_live_agents_and_creates_none() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session = uuid::Uuid::new_v4().to_string();

        assert!(manager.peek_agent(&session).await.is_none());
        // …and asking did not answer itself into existence.
        assert!(!manager.has_session(&session).await);
        assert_eq!(manager.session_count().await, 0);
        assert!(manager.peek_agent(&session).await.is_none());

        let created = manager.get_or_create_agent(session.clone()).await.unwrap();
        let peeked = manager.peek_agent(&session).await.expect("now live");
        assert!(
            Arc::ptr_eq(&created, &peeked),
            "peek must hand back THE live agent, not an equivalent one: the \
             caller reads its `config.biorouter_mode`"
        );
    }

    /// BR-71: a subagent run registers its ALREADY-CONFIGURED agent so the
    /// server's get_or_create_agent (the /interrupt and /reply resolution
    /// path — `AppState::get_agent_for_route`, `state.rs:341`; `get_agent` is
    /// `:334`) returns the LIVE instance, not a fresh default one.
    #[tokio::test]
    async fn register_agent_makes_get_or_create_return_the_live_instance() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let child = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                session_manager,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));

        manager
            .register_agent("child-1".to_string(), child.clone())
            .await;
        let resolved = manager
            .get_or_create_agent("child-1".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&child, &resolved),
            "steer/interrupt must reach the SAME live agent the run drives"
        );

        // Deregistration removes exactly our entry; a successor registered
        // meanwhile survives (the TurnGuard-style only-clear-your-own rule).
        manager.deregister_agent_if_same("child-1", &child).await;
        assert!(!manager.has_session("child-1").await);

        let replacement = manager
            .get_or_create_agent("child-1".to_string())
            .await
            .unwrap();
        manager.deregister_agent_if_same("child-1", &child).await; // stale — no-op
        let still = manager
            .get_or_create_agent("child-1".to_string())
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&replacement, &still));
    }

    /// Decision 10: 100 intervening agent creations must NOT evict a running
    /// registered child. Without the pin this test fails and a mid-run steer
    /// silently reaches a fresh agent that no loop drains.
    #[tokio::test]
    async fn a_registered_agent_survives_lru_pressure() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let child = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                session_manager,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));
        manager
            .register_agent("pinned-child".to_string(), child.clone())
            .await;

        for i in 0..150 {
            let _ = manager
                .get_or_create_agent(format!("filler-{i}"))
                .await
                .unwrap();
        }

        let resolved = manager
            .get_or_create_agent("pinned-child".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&child, &resolved),
            "a running registered agent must survive LRU pressure"
        );
        manager
            .deregister_agent_if_same("pinned-child", &child)
            .await;
        // Once deregistered it is ordinary again: a fresh resolution mints a
        // NEW agent rather than resurrecting the pinned one.
        let after = manager
            .get_or_create_agent("pinned-child".to_string())
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&child, &after));
    }

    /// Registration is REFCOUNTED, so overlapping runs on the same agent cannot
    /// unregister each other.
    ///
    /// The case this exists for is Task 41: a durable Agent Drafter worker is
    /// consulted twice in quick succession and `build_worker` hands back the
    /// SAME `Arc` both times. Consult #1's deregistration is `tokio::spawn`ed
    /// and can land after consult #2 has already registered and started its
    /// turn. With a plain insert/remove, `Arc::ptr_eq` matches, the live
    /// registration is dropped mid-turn, and the "steerable via /interrupt"
    /// property silently disappears — the exact bug `register_agent` was added
    /// to fix, reintroduced by its own cleanup.
    #[tokio::test]
    async fn overlapping_registrations_of_the_same_agent_do_not_cancel_each_other() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let worker = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                session_manager,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));

        // Two overlapping runs on one worker.
        manager
            .register_agent("worker".to_string(), worker.clone())
            .await;
        manager
            .register_agent("worker".to_string(), worker.clone())
            .await;

        // The first finishes and cleans up …
        manager.deregister_agent_if_same("worker", &worker).await;
        // … the second is still live and must still resolve to THIS instance.
        let resolved = manager
            .get_or_create_agent("worker".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&worker, &resolved),
            "a live overlapping registration must survive its predecessor's cleanup"
        );

        // Only when the last one releases does the pin go.
        manager.deregister_agent_if_same("worker", &worker).await;
        let after = manager
            .get_or_create_agent("worker".to_string())
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&worker, &after));
    }

    /// `deregister` must not evict an LRU entry it never created — the entry a
    /// consulted worker got from an ordinary `get_agent` (`routes/apps.rs:1663`).
    #[tokio::test]
    async fn deregistering_does_not_evict_a_cache_entry_it_did_not_create() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        // An ordinary cached agent, exactly as `state.get_agent` produces.
        let cached = manager
            .get_or_create_agent("worker".to_string())
            .await
            .unwrap();
        // A run registers that same agent, then finishes.
        manager
            .register_agent("worker".to_string(), cached.clone())
            .await;
        manager.deregister_agent_if_same("worker", &cached).await;

        let after = manager
            .get_or_create_agent("worker".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&cached, &after),
            "the LRU entry predates the registration and must outlive it"
        );
    }

    /// What a registration does to an entry that is ALREADY in the LRU under the
    /// same id: it shadows it, it does not replace it.
    ///
    /// The entry in question is the placeholder a resolution racing the
    /// registration leaves behind — `get_or_create_agent` consults the pin,
    /// drops that guard, and only then reads the cache, so a `/reply` or a steer
    /// landing between the run's `begin_turn` and its `register_agent` mints a
    /// bare agent and caches it under the child's id.
    ///
    /// This exists because the doc on `register_agent` claimed the opposite
    /// ("overwrites any placeholder entry…") for a mechanism that was never
    /// written. Behaviour was fine; the sentence was evidence for a property
    /// nothing checked. Now something checks it — in both directions, because
    /// each direction is load-bearing for a different caller: the pin must win
    /// DURING the run (this task's whole point), and the cache entry must
    /// survive AFTER it (`deregistering_does_not_evict_a_cache_entry_it_did_not_create`
    /// — from in here a placeholder and a consulted worker's own cached agent
    /// are the same thing).
    #[tokio::test]
    async fn a_registration_shadows_a_racing_placeholder_it_does_not_replace_it() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        // The race: a resolution gets there first and caches a bare agent.
        let placeholder = manager
            .get_or_create_agent("child-2".to_string())
            .await
            .unwrap();

        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let live = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                session_manager,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));
        manager
            .register_agent("child-2".to_string(), live.clone())
            .await;

        // For the run's whole duration the LIVE child wins anyway — the pin is
        // consulted before the cache, so losing the race costs nothing.
        let during = manager
            .get_or_create_agent("child-2".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&live, &during),
            "the pin must outrank a cache entry that got there first, or a steer \
             mid-run reaches the placeholder no loop drains"
        );
        assert!(
            Arc::ptr_eq(&live, &manager.peek_agent("child-2").await.expect("pinned")),
            "…and on the peek path too, which is how `workspace_send_prompt` reads \
             the target's permission mode"
        );

        // Afterwards the placeholder is still there: shadowed, never replaced.
        manager.deregister_agent_if_same("child-2", &live).await;
        let after = manager
            .get_or_create_agent("child-2".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&placeholder, &after),
            "the placeholder must resurface: `register_agent` deliberately does \
             not evict the LRU, because from in here it cannot tell a placeholder \
             from a consulted worker's own cached agent"
        );
    }

    /// An explicit stop of a session that exists ONLY as a pin must succeed.
    ///
    /// A registered child is never put in the `sessions` LRU, so the pre-BR-71
    /// body — which reports "not found" whenever `LruCache::pop` misses — would
    /// evict the pin and then return `Err`. `POST /agent/stop` maps that to a
    /// 404 and `workspace_close scope:"agent"` (via
    /// `ServerWorkspaceServices::stop_agent`) surfaces it to the model as a
    /// failure, for a stop that in fact worked. "Not found" must mean neither
    /// half knew the session, not "the LRU half didn't".
    #[tokio::test]
    async fn removing_a_pin_only_session_succeeds_and_unpins_it() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let child = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                session_manager,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));
        manager
            .register_agent("stop-me".to_string(), child.clone())
            .await;
        assert!(manager.has_session("stop-me").await);

        manager.remove_session("stop-me").await.unwrap();
        assert!(!manager.has_session("stop-me").await);

        // An explicit stop outranks the registration outright: the run's own
        // later deregistration finds nothing and is a no-op, and the id is
        // ordinary again.
        manager.deregister_agent_if_same("stop-me", &child).await;
        let after = manager
            .get_or_create_agent("stop-me".to_string())
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&child, &after));
    }

    /// The "only clear your own" rule has to survive an explicit stop, which is
    /// the one place the refcount is thrown away rather than decremented.
    ///
    /// `remove_session` unpins unconditionally, so a stopped run's outstanding
    /// deregistrations — `Deregister::drop` spawns them, they can land whenever —
    /// arrive at an id that has since been claimed by a NEW run. They must not
    /// touch it. Without the `Arc::ptr_eq` guard in `deregister_agent_if_same`
    /// they would, and the symptom is the pre-BR-71 bug wearing a disguise: a
    /// steer mid-run silently reaches a freshly minted agent no loop drains,
    /// caused by a run that already finished.
    ///
    /// The residual hazard this canNOT pin — the successor being the SAME `Arc`,
    /// where pointer identity cannot tell the registrations apart — is written
    /// up on `remove_session` for Task 41, because it is unreachable from the
    /// subagent path (a fresh session id per run) and closing it needs a
    /// registration identity finer than the pointer.
    #[tokio::test]
    async fn a_stale_deregistration_after_a_stop_cannot_clear_a_successor() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let new_agent = || {
            Arc::new(crate::agents::Agent::with_config(
                crate::agents::AgentConfig::new(
                    session_manager.clone(),
                    crate::config::permission::PermissionManager::instance(),
                    None,
                    crate::config::BioRouterMode::Auto,
                ),
            ))
        };
        let stopped = new_agent();
        let successor = new_agent();

        // Two overlapping runs on one agent (`runs: 2`) …
        manager
            .register_agent("stopped".to_string(), stopped.clone())
            .await;
        manager
            .register_agent("stopped".to_string(), stopped.clone())
            .await;
        // … then an explicit stop, which discards the count outright.
        manager.remove_session("stopped").await.unwrap();
        assert!(!manager.has_session("stopped").await);

        // A new run claims the id before either stopped guard has dropped.
        manager
            .register_agent("stopped".to_string(), successor.clone())
            .await;

        // Both stale releases land late. Neither may disturb the successor.
        manager.deregister_agent_if_same("stopped", &stopped).await;
        manager.deregister_agent_if_same("stopped", &stopped).await;

        let resolved = manager
            .get_or_create_agent("stopped".to_string())
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&successor, &resolved),
            "a finished run's deregistration must only ever clear its OWN \
             registration; after a stop it owns nothing, and the live successor \
             has to survive both of its releases"
        );
    }

    /// A pinned agent is a live agent for `peek_agent` too. `peek_agent` is how
    /// `workspace_send_prompt` reads a target's permission mode without minting
    /// one; a running glass-box child that peeked as absent would take the
    /// conservative "no live agent, assume approval required" branch while its
    /// own loop is right there holding the pin.
    #[tokio::test]
    async fn peek_agent_sees_a_pinned_agent() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let child = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                session_manager,
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ),
        ));
        assert!(manager.peek_agent("peek-child").await.is_none());

        manager
            .register_agent("peek-child".to_string(), child.clone())
            .await;
        let peeked = manager.peek_agent("peek-child").await.expect("pinned");
        assert!(Arc::ptr_eq(&child, &peeked));

        manager.deregister_agent_if_same("peek-child", &child).await;
        assert!(manager.peek_agent("peek-child").await.is_none());
    }

    #[test]
    fn test_execution_mode_constructors() {
        assert_eq!(
            SessionExecutionMode::chat(),
            SessionExecutionMode::Interactive
        );
        assert_eq!(
            SessionExecutionMode::scheduled(),
            SessionExecutionMode::Background
        );

        let parent = "parent-123".to_string();
        assert_eq!(
            SessionExecutionMode::task(parent.clone()),
            SessionExecutionMode::SubTask {
                parent_session: parent
            }
        );
    }

    #[tokio::test]
    async fn test_session_isolation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        let session1 = uuid::Uuid::new_v4().to_string();
        let session2 = uuid::Uuid::new_v4().to_string();

        let agent1 = manager.get_or_create_agent(session1.clone()).await.unwrap();

        let agent2 = manager.get_or_create_agent(session2.clone()).await.unwrap();

        // Different sessions should have different agents
        assert!(!Arc::ptr_eq(&agent1, &agent2));

        // Getting the same session should return the same agent
        let agent1_again = manager.get_or_create_agent(session1).await.unwrap();

        assert!(Arc::ptr_eq(&agent1, &agent1_again));
    }

    #[tokio::test]
    async fn test_session_limit() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        let sessions: Vec<_> = (0..100).map(|i| format!("session-{}", i)).collect();

        for session in &sessions {
            manager.get_or_create_agent(session.clone()).await.unwrap();
        }

        // Create a new session after cleanup
        let new_session = "new-session".to_string();
        let _new_agent = manager.get_or_create_agent(new_session).await.unwrap();

        assert_eq!(manager.session_count().await, 100);
    }

    #[tokio::test]
    async fn test_remove_session() {
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session = String::from("remove-test");

        manager.get_or_create_agent(session.clone()).await.unwrap();
        assert!(manager.has_session(&session).await);

        manager.remove_session(&session).await.unwrap();
        assert!(!manager.has_session(&session).await);

        assert!(manager.remove_session(&session).await.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(create_test_manager(&temp_dir).await);
        let session = String::from("concurrent-test");

        let mut handles = vec![];
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            let sess = session.clone();
            handles.push(tokio::spawn(async move {
                mgr.get_or_create_agent(sess).await.unwrap()
            }));
        }

        let agents: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        for agent in &agents[1..] {
            assert!(Arc::ptr_eq(&agents[0], agent));
        }

        assert_eq!(manager.session_count().await, 1);
    }

    #[tokio::test]
    async fn test_concurrent_session_creation_race_condition() {
        // Test that concurrent attempts to create the same new session ID
        // result in only one agent being created (tests double-check pattern)
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(create_test_manager(&temp_dir).await);
        let session_id = String::from("race-condition-test");

        // Spawn multiple tasks trying to create the same NEW session simultaneously
        let mut handles = vec![];
        for _ in 0..20 {
            let sess = session_id.clone();
            let mgr_clone = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr_clone.get_or_create_agent(sess).await.unwrap()
            }));
        }

        // Collect all agents
        let agents: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        for agent in &agents[1..] {
            assert!(
                Arc::ptr_eq(&agents[0], agent),
                "All concurrent requests should get the same agent"
            );
        }
        assert_eq!(manager.session_count().await, 1);
    }

    #[tokio::test]
    async fn test_set_default_provider() {
        use crate::providers::testprovider::TestProvider;

        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        // Create a test provider for replaying (doesn't need inner provider)
        let temp_file = temp_dir.path().join("test_provider.json");

        // Create an empty test provider (will fail on actual use but that's ok for this test)
        std::fs::write(&temp_file, "{}").unwrap();
        let test_provider = TestProvider::new_replaying(temp_file.to_str().unwrap()).unwrap();

        manager.set_default_provider(Arc::new(test_provider)).await;

        let session = String::from("provider-test");
        let _agent = manager.get_or_create_agent(session.clone()).await.unwrap();

        assert!(manager.has_session(&session).await);
    }

    #[tokio::test]
    async fn test_eviction_updates_last_used() {
        // Test that accessing a session updates its last_used timestamp
        // and affects eviction order
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;

        let sessions: Vec<_> = (0..100).map(|i| format!("session-{}", i)).collect();

        for session in &sessions {
            manager.get_or_create_agent(session.clone()).await.unwrap();
            // Small delay to ensure different timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Access the first session again to update its last_used
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        manager
            .get_or_create_agent(sessions[0].clone())
            .await
            .unwrap();

        // Now create a 101st session - should evict session2 (least recently used)
        let session101 = String::from("session-101");
        manager
            .get_or_create_agent(session101.clone())
            .await
            .unwrap();

        assert!(manager.has_session(&sessions[0]).await);
        assert!(!manager.has_session(&sessions[1]).await);
        assert!(manager.has_session(&session101).await);
    }

    /// `has_session` must never hold the pin guard while it waits for the LRU.
    ///
    /// It doesn't — both operands of `||` are their own temporary scope, so the
    /// pin guard is gone before `sessions` is touched — but that is a subtle
    /// enough rule that a reviewer read the one-liner as a `pinned -> sessions`
    /// nesting, and the two forms are indistinguishable from their answers. So
    /// pin the property rather than the argument: this goes red for any rewrite
    /// that DOES hold the pin across the await (verified against
    /// `let g = pinned.read().await; g.contains_key(..) || sessions.read().await
    /// .contains(..)`, which fails here on the 2 s timeout).
    ///
    /// It matters because nothing else in this file holds two of these guards at
    /// once, and that — not a fixed order — is what makes a future
    /// `sessions -> pinned` path safe instead of a deadlock.
    ///
    /// The probe is parked on the LRU write lock we hold; if it were also
    /// holding the pin READ lock, the write acquisition below could not
    /// complete and the timeout fires with the invariant named.
    #[tokio::test]
    async fn has_session_does_not_hold_the_pin_lock_while_it_waits() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(create_test_manager(&temp_dir).await);

        // Hold the LRU so an un-pinned lookup must park on it.
        let sessions_guard = manager.sessions.write().await;
        let probe = {
            let manager = Arc::clone(&manager);
            tokio::spawn(async move { manager.has_session("not-pinned-not-cached").await })
        };
        // Let the probe get past the (uncontended) pin read and block on the LRU.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Acquiring it IS the assertion; holding it is not the point, so it is
        // dropped immediately (an unbound `must_use` guard warns).
        let pin_guard =
            tokio::time::timeout(std::time::Duration::from_secs(2), manager.pinned.write())
                .await
                .expect(
                    "has_session must not hold the pin lock while it waits on the LRU: a \
             `pinned -> sessions` nesting here deadlocks any future path that takes \
             them the other way round",
                );
        drop(pin_guard);

        drop(sessions_guard);
        assert!(!probe.await.unwrap());
    }

    #[tokio::test]
    async fn test_remove_nonexistent_session_error() {
        // Test that removing a non-existent session returns an error
        let temp_dir = TempDir::new().unwrap();
        let manager = create_test_manager(&temp_dir).await;
        let session = String::from("never-created");

        let result = manager.remove_session(&session).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    /// The first-run seeding must land in the root the manager was BUILT with,
    /// never in whatever `BIOROUTER_PATH_ROOT` happens to be ambient by the
    /// time the background task actually runs.
    ///
    /// This is the root cause of the `missing_or_disabled_soul_skill_fails_\
    /// before_raw_staging` flake (CI `test (ubuntu-latest)`, PR #191 run
    /// 34304297956): `new` **spawns** `run_first_run_init`, which used to
    /// resolve `Paths::config_dir()` at seed time — inside a detached task,
    /// long after `new` returned. Whichever other test held `env_lock` at that
    /// moment owned the directory the soul skill was written into, so a test
    /// whose entire assertion is "no `update-soul` skill exists in MY root" was
    /// handed one by a manager it never built. A lock in the reader cannot
    /// close that — the writer never asks for it (the family recorded in
    /// `model.rs`, "only serialises callers that *ask* for it").
    ///
    /// ⚠ **This test is a detector for its OWN manager, and it runs in a
    /// process of its own** (`test_sandbox::in_a_process_of_its_own`), so its
    /// manager is the only one it can see. It holds the environment on
    /// `ambient` — a root it hands to nobody — for its whole body, so if that
    /// manager's `update-soul` seeding resolves its root from
    /// `BIOROUTER_PATH_ROOT` instead of the root it was built with, the skill
    /// lands in `ambient` and the first assertion fails. That skill is ALL it
    /// looks at: the scheduler's workflow copy follows the ambient data dir into
    /// `ambient/data/` unobserved (see [`AgentManager::new`]).
    ///
    /// It used to run in the shared test process, where it also caught any
    /// SIBLING manager that seeded inside its window — by chance, which is how
    /// it went red in CI run 35480469489 (2026-09-20) with its own manager
    /// innocent: a sibling built by `create_test_manager` without the lock read
    /// `ambient` as its construction root and seeded it. It moved out because
    /// holding a foreign root for up to 30 s there handed that root to every
    /// test that resolves `Paths` without the env lock, at least 466 of them.
    /// The sibling case is now
    /// `create_test_manager_seeds_its_own_root_while_another_test_holds_the_environment`'s,
    /// forced every run instead of caught by chance.
    ///
    /// Holding `ambient` from BEFORE construction, rather than swapping to it
    /// after `new` returns as this test first did, is strictly stronger now
    /// that the root is a parameter: an ambient read in `new` itself (a caller
    /// or a refactor that falls back to `Paths::config_dir()`) is caught as
    /// surely as one in the spawned init. (While it still shared the process,
    /// that also widened its window for a sibling constructing alongside it —
    /// measured 2026-09-21 on macOS, with `create_test_manager` put back to an
    /// ambient read: the old shape caught the sibling in 1 of 256 loaded runs
    /// of this module, this one in 9 of 192.)
    ///
    /// The runtime is deliberately `current_thread`: the spawned init can then
    /// only run when this test yields, so it provably runs inside the window
    /// in which `ambient` is held, and a regression in THIS test's manager
    /// fails every time rather than sometimes.
    #[tokio::test(flavor = "current_thread")]
    async fn first_run_seeding_lands_in_the_root_the_manager_was_built_with() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let built_with = TempDir::new().unwrap();
        let ambient = TempDir::new().unwrap();
        let sessions = TempDir::new().unwrap();

        // Some *other* root is the ambient one for the whole test. No manager's
        // `update-soul` seeding may follow it there. (The scheduler's workflow
        // copy still does, into `ambient/data/`; this test does not look.)
        let _env = crate::test_sandbox::relocate_path_root(ambient.path().to_str().unwrap());

        let session_manager = Arc::new(SessionManager::new(sessions.path().to_path_buf()));
        let manager = AgentManager::new(
            session_manager,
            sessions.path().join("schedule.json"),
            built_with.path().join("config"),
            Some(4),
        )
        .await
        .unwrap();
        // `new` returns without ever yielding after the spawn, so on a
        // current-thread runtime the init has provably not run yet — it runs
        // in the loop below, while `ambient` is still held.

        // The loop exits as soon as the seed lands in EITHER root — ~50 ms in
        // practice, in both the passing and the failing direction — so the
        // generous cap is paid only by a genuinely stuck run, never by a green
        // one. A fix for a flake must not itself be timing-sensitive on a
        // loaded runner: the first `ensure_soul_kb` initialises a git repo.
        for _ in 0..3_000 {
            if soul_skill(built_with.path()).is_file() || soul_skill(ambient.path()).exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert!(
            !soul_skill(ambient.path()).exists(),
            "a first-run seeding followed the ambient BIOROUTER_PATH_ROOT into \
             {} — a root no manager was ever given. That is the flake: any test \
             holding the environment when a background init fires is handed an \
             `update-soul` skill it did not install. It need not be this test's \
             manager — any `AgentManager::new` whose caller resolved its root \
             from the environment while this test held it seeds here too.",
            ambient.path().display()
        );
        assert!(
            soul_skill(built_with.path()).is_file(),
            "the first-run seeding never reached the manager's own root at {}",
            built_with.path().display()
        );

        drop(manager);
    }

    /// The CI 35480469489 interleaving, forced rather than waited for: a
    /// manager built by this module's own `create_test_manager` **while some
    /// other test holds `BIOROUTER_PATH_ROOT`** must seed into its own temp
    /// dir, and leave the holder's root untouched.
    ///
    /// `first_run_seeding_lands_in_the_root_the_manager_was_built_with` caught
    /// this, but only when a sibling happened to construct inside its window
    /// AND finish seeding before its own manager did — measured on this macOS
    /// host at 1 run in 256 of `execution::manager` under 8-way concurrent
    /// load, and 0 in 60 without load; CI hit it once in a full lib run on a
    /// 4-vCPU runner. A regression caught that rarely reads as a flake and gets
    /// re-run green, which is exactly what happened. Here the lock is simply
    /// held across the helper's construction, so the helper's root is decided
    /// while a foreign one is ambient every time — revert the helper to
    /// `Paths::config_dir()` and this fails on every run.
    ///
    /// The wait is on a real signal: the Daily Meditation job is the LAST
    /// thing `soul::install` registers, after the skill is written, so once the
    /// scheduler lists it the seeding has finished wherever it was going to
    /// land. A bounded poll rather than a sleep, so a loaded runner makes this
    /// slower and never red.
    #[tokio::test]
    async fn create_test_manager_seeds_its_own_root_while_another_test_holds_the_environment() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let foreign = TempDir::new().unwrap();
        let own = TempDir::new().unwrap();

        // Stand-in for the detector above (or any relocating test): the
        // environment names a root this manager is never given.
        let _env = crate::test_sandbox::relocate_path_root(foreign.path().to_str().unwrap());

        let manager = create_test_manager(&own).await;

        let meditation = crate::knowledge::soul::MEDITATION_SCHEDULE_ID;
        let mut registered = false;
        for _ in 0..3_000 {
            let jobs = manager.scheduler().list_scheduled_jobs().await;
            if jobs.iter().any(|job| job.id == meditation) {
                registered = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            registered,
            "the first-run init never registered `{meditation}` within 30 s, so this \
             test cannot say where the seeding went"
        );

        assert!(
            !soul_skill(foreign.path()).exists(),
            "a manager from `create_test_manager` seeded `update-soul` into {} — the \
             root another test was holding, not the one it was handed. The helper \
             resolved its config root from the environment; that is the CI \
             35480469489 flake.",
            foreign.path().display()
        );
        assert!(
            soul_skill(own.path()).is_file(),
            "the manager's own root {} never received its `update-soul` skill",
            own.path().display()
        );
    }
}
