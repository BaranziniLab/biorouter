//! Cached, gitignore-aware workspace file map for the agent's context (BR-1).
//!
//! Before this, the model was told exactly one thing about its surroundings — a
//! single `Working directory: <path>` line in the per-turn MOIM `<info-msg>`
//! block (`extension_manager.rs`) — and nothing about the *shape* of the
//! project: no file tree, no listing, no `ls` snapshot
//! (`docs/history/agent-loop-review/subsystem-reviews/state-awareness-and-version-control.md` gap #1). The agent
//! rediscovered structure by hand every session. This module produces a bounded
//! file listing (Cline/Claude-Code `environment_details` level) and
//! [`collect_moim`](crate::agents::extension_manager::ExtensionManager::collect_moim)
//! appends it right after the working-directory line.
//!
//! It respects the same trust boundary as the rest of the agent's file access:
//! `.gitignore` / `.ignore` (via the `ignore` crate) and the local + global
//! `.biorouterignore` files, so anything the user hid from the agent stays out
//! of the map too.
//!
//! Cost is bounded three ways so re-injecting the map every action is cheap:
//! - The walk is depth- and entry-capped (`CONTEXT_WORKSPACE_SUMMARY_MAX_DEPTH`
//!   / `_MAX_ENTRIES`).
//! - The rendered text is token-capped and coordinates with the BR-2 injection
//!   budget by reusing [`truncate_to_tokens`].
//! - Results are cached per working directory for a short TTL
//!   (`CONTEXT_WORKSPACE_SUMMARY_TTL_SECS`), so a long multi-tool turn walks the
//!   tree at most once per TTL rather than on every provider call.
//!
//! The whole feature is gated by `CONTEXT_WORKSPACE_SUMMARY` (default on); set it
//! to `false` to fall back to the old one-line behavior.
//!
//! # The walk is never on the turn's critical path
//!
//! This module used to walk the tree **synchronously, on the tokio worker
//! driving the turn**, with no timeout, no cancellation point and no log line.
//! On 2026-09-10 a test drive of `main` measured what that costs: with the
//! default working directory (`$HOME` on macOS) every turn blocked in
//! `std::fs::read_dir` → `__opendir2` under `~/Library/Group Containers` —
//! Dropbox, iCloud, GlobalProtect and Office app-group containers — and never
//! reached the provider at all. 100% of `/usr/bin/sample` samples sat in that
//! one stack, four captures over two sessions; the daemon idled at 0.0% CPU
//! while the composer said "Thinking" for 8m32s. `Stop` could not end it,
//! because a blocking `std::fs` call has no cancellation point, and each wedged
//! turn leaked its worker for the life of the process.
//!
//! Four rules keep that from recurring, and each is load-bearing:
//!
//! 1. **No filesystem syscall happens on the async path.** Not the walk, not the
//!    root's `stat`, not `canonicalize`. Every one of them belongs to the
//!    blocking task, where a wedge costs a pool thread instead of the turn. This
//!    is why the cache is keyed on the path *as given* rather than on its
//!    canonical form, and why the fast path is TTL-only.
//! 2. **The walk runs under [`tokio::task::spawn_blocking`] with a finite
//!    budget** (`CONTEXT_WORKSPACE_SUMMARY_BUDGET_MS`). When the budget lapses
//!    the turn proceeds *without* a map, one WARN names the directory and the
//!    lever, and the negative result is cached for the TTL so the next turn pays
//!    nothing. The budget cannot be configured to zero: a "disabled" budget
//!    would be a way to reinstate the hang.
//! 3. **The wait is tied to the turn's cancellation token**, so `Stop` is
//!    honoured immediately even though the walk itself cannot be cancelled.
//! 4. **One walk per root, ever, while one is in flight** (single-flight). A
//!    blocked `std::fs` call cannot be cancelled, so a thread that wedges is
//!    lost — the single-flight marker is what bounds the loss to **one thread
//!    per root** instead of one per turn. A walk that never returns therefore
//!    never releases its marker, and that is deliberate.
//!
//! # A home directory is not a workspace
//!
//! Rule 2 bounds the damage; it does not make walking `$HOME` a good idea. A
//! home directory is the union of every project the user owns *and* the
//! operating system's own per-user state — on macOS that state includes the
//! cloud-provider containers that produced the wedge above, and on any platform
//! it is tens of thousands of entries that say nothing about the task at hand.
//! So [`skip_reason`] refuses three kinds of root outright, with no walk at all:
//! the home directory itself, a filesystem root, and anything at or beneath an
//! *opaque tree* (`~/Library`, `~/AppData`, `~/.Trash`, and any path holding a
//! `Group Containers`, `CloudStorage`, `Mobile Documents` or `FileProvider`
//! component).
//!
//! The same trees are pruned *during* a walk that starts somewhere legitimate,
//! and `same_file_system(true)` stops the walk crossing a mount point — a
//! network share or a File Provider volume is a mount, and an unresponsive one
//! is the other way this blocks.
//!
//! See [`docs/agent-loop/workspace-map.md`](../../../../docs/agent-loop/workspace-map.md).

use crate::config::paths::Paths;
use crate::config::Config;
use crate::context_budget::truncate_to_tokens;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Whether the workspace map is injected at all.
pub const DEFAULT_ENABLED: bool = true;
/// Maximum rendered entries (files + directories) before the tail is elided.
pub const DEFAULT_MAX_ENTRIES: usize = 200;
/// Maximum directory depth walked below the working directory.
pub const DEFAULT_MAX_DEPTH: usize = 3;
/// Token cap for the rendered map (coordinates with the BR-2 MOIM budget).
pub const DEFAULT_MAX_TOKENS: usize = 2_000;
/// Cache time-to-live: an upper bound on how stale the served map can be.
pub const DEFAULT_TTL_SECS: u64 = 30;
/// How long a turn will wait for the walk before giving up on it and proceeding
/// without a map. Generous enough that an ordinary project never notices, short
/// enough that a user does.
pub const DEFAULT_BUDGET_MS: u64 = 1_500;

/// Hard ceiling on filesystem entries visited during a single walk, independent
/// of `max_entries`, so a pathological directory (100k flat files) can't make
/// the bounded walk unbounded. If hit, the map is marked truncated.
const SCAN_CAP: usize = 20_000;

/// Path components whose subtree is never walked, wherever they appear. Each is
/// a place the operating system or a sync client mediates reads, so an
/// `opendir` there can block on a daemon rather than on a disk.
const OPAQUE_COMPONENTS: &[&str] = &[
    // macOS app-group containers: Dropbox, iCloud, Office, GlobalProtect. The
    // directory that produced the measured wedge.
    "Group Containers",
    // ~/Library/CloudStorage — every File Provider cloud mount.
    "CloudStorage",
    // ~/Library/Mobile Documents — iCloud Drive.
    "Mobile Documents",
    // File Provider caches and domain state.
    "FileProvider",
    ".Trash",
];

/// Children of the home directory that are opaque. Matched home-relative rather
/// than by name alone, because `Library/` is an ordinary directory name inside a
/// project (an R library, a component library) and must stay walkable there.
///
/// ⚠ `AppData` is deliberately NOT on this list, though it is the obvious
/// Windows counterpart to `~/Library`. `std::env::temp_dir()` on Windows is
/// `%USERPROFILE%\AppData\Local\Temp`, so refusing that subtree would refuse
/// every scratch workspace — and empty the walk in every `build_summary` test
/// that runs there. Nothing in `AppData` blocks the way a File Provider mount
/// does; the components above are what capture the measured hazard, and they
/// are matched wherever they appear, including on Windows.
const HOME_OPAQUE_CHILDREN: &[&str] = &["Library", ".Trash"];

fn config_bool(key: &str, default: bool) -> bool {
    Config::global().get_param::<bool>(key).unwrap_or(default)
}

fn config_usize(key: &str, default: usize) -> usize {
    Config::global().get_param::<usize>(key).unwrap_or(default)
}

fn config_u64(key: &str, default: u64) -> u64 {
    Config::global().get_param::<u64>(key).unwrap_or(default)
}

fn enabled() -> bool {
    config_bool("CONTEXT_WORKSPACE_SUMMARY", DEFAULT_ENABLED)
}

fn ttl() -> Duration {
    Duration::from_secs(config_u64(
        "CONTEXT_WORKSPACE_SUMMARY_TTL_SECS",
        DEFAULT_TTL_SECS,
    ))
}

/// The per-turn wait budget. **Always finite**: a configured `0` reads as "use
/// the default", not as "wait forever". `CONTEXT_WORKSPACE_SUMMARY: false` is
/// how the feature is turned off; there is deliberately no setting that puts an
/// uncancellable filesystem walk back on the turn's critical path.
fn budget() -> Duration {
    let ms = config_u64("CONTEXT_WORKSPACE_SUMMARY_BUDGET_MS", DEFAULT_BUDGET_MS);
    Duration::from_millis(if ms == 0 { DEFAULT_BUDGET_MS } else { ms })
}

/// Why a root gets no workspace map at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The working directory *is* the user's home directory.
    Home,
    /// The working directory is a filesystem root (`/`, `C:\`).
    FilesystemRoot,
    /// The working directory is at or beneath a tree whose reads are mediated
    /// by the OS or a sync client (see [`OPAQUE_COMPONENTS`]).
    OpaqueTree,
}

impl SkipReason {
    /// A phrase for the log line, written for whoever is wondering why their
    /// chat has no file map.
    pub fn describe(self) -> &'static str {
        match self {
            SkipReason::Home => {
                "the working directory is the home directory, which is not a workspace"
            }
            SkipReason::FilesystemRoot => "the working directory is a filesystem root",
            SkipReason::OpaqueTree => {
                "the working directory is inside a system or cloud-sync tree whose reads can block"
            }
        }
    }
}

/// True when `dir` is at or beneath a tree this module refuses to read.
fn is_opaque_tree(dir: &Path, home: Option<&Path>) -> bool {
    let opaque_component = dir.components().any(|component| match component {
        Component::Normal(name) => OPAQUE_COMPONENTS
            .iter()
            .any(|opaque| name.eq_ignore_ascii_case(opaque)),
        _ => false,
    });
    if opaque_component {
        return true;
    }
    let Some(home) = home else { return false };
    HOME_OPAQUE_CHILDREN
        .iter()
        .any(|child| dir.starts_with(home.join(child)))
}

/// Whether `dir` gets a workspace map at all, resolved against the real home
/// directory. See the module doc: a home directory is not a workspace.
pub fn skip_reason(dir: &Path) -> Option<SkipReason> {
    skip_reason_with_home(dir, Paths::home_dir().as_deref())
}

/// [`skip_reason`] with the home directory injected, so the rule is unit
/// testable without touching the process environment.
pub fn skip_reason_with_home(dir: &Path, home: Option<&Path>) -> Option<SkipReason> {
    // `Path`'s equality and `starts_with` both compare *components*, so a
    // trailing slash or an interior `.` is already normalised away.
    if dir.parent().is_none() {
        return Some(SkipReason::FilesystemRoot);
    }
    if home.is_some_and(|home| dir == home) {
        return Some(SkipReason::Home);
    }
    if is_opaque_tree(dir, home) {
        return Some(SkipReason::OpaqueTree);
    }
    None
}

/// Bounds for a single map render, resolved from config with generous defaults.
#[derive(Clone, Copy, Debug)]
pub struct SummaryConfig {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_tokens: usize,
}

impl SummaryConfig {
    fn from_config() -> Self {
        Self {
            max_entries: config_usize("CONTEXT_WORKSPACE_SUMMARY_MAX_ENTRIES", DEFAULT_MAX_ENTRIES),
            max_depth: config_usize("CONTEXT_WORKSPACE_SUMMARY_MAX_DEPTH", DEFAULT_MAX_DEPTH),
            max_tokens: config_usize("CONTEXT_WORKSPACE_SUMMARY_MAX_TOKENS", DEFAULT_MAX_TOKENS),
        }
    }
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

struct CacheEntry {
    computed_at: Instant,
    /// `None` means "walked, nothing to show" — still cached so an empty or
    /// unreadable directory isn't rewalked every action — or "the walk did not
    /// finish inside its budget", which is cached for exactly the same reason.
    summary: Option<String>,
}

/// Cache, single-flight markers and warn-once markers under **one** mutex, so
/// there is no lock ordering to get wrong between them. Every critical section
/// here is a few map operations long: the mutex is never held across the walk.
#[derive(Default)]
struct WalkState {
    cache: HashMap<PathBuf, CacheEntry>,
    /// Roots with a walk in flight. An entry that never clears is a walk that
    /// never returned — see rule 4 in the module doc.
    inflight: HashSet<PathBuf>,
    /// How many walks this process has started per root. Instrumentation: it is
    /// what lets a test prove single-flight held, per root rather than
    /// process-wide so tests of different roots can run in parallel.
    walk_starts: HashMap<PathBuf, usize>,
    /// Roots that have already produced a budget WARN, so a slow directory
    /// costs one log line rather than one per turn.
    warned_budget: HashSet<PathBuf>,
    /// Roots that have already produced a skip log line, same reason.
    warned_skip: HashSet<PathBuf>,
}

static STATE: Lazy<Mutex<WalkState>> = Lazy::new(|| Mutex::new(WalkState::default()));

/// Releases the single-flight marker on any *return* from the walk task,
/// including a panic. A thread blocked forever in `opendir` never drops it,
/// which is what bounds the leak to one thread per root.
struct InflightGuard(PathBuf);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = STATE.lock() {
            state.inflight.remove(&self.0);
        }
    }
}

/// Absolute path to the global `.biorouterignore`, mirroring the developer MCP
/// server so the workspace map honors the same global excludes.
///
/// Touches the filesystem (`is_file`), so it is resolved inside the blocking
/// task rather than on the turn's async path.
fn global_biorouterignore() -> Option<PathBuf> {
    let path = Paths::config_dir().join(".biorouterignore");
    path.is_file().then_some(path)
}

fn log_skip_once(dir: &Path, reason: SkipReason) {
    let first = {
        let mut state = STATE.lock().unwrap();
        state.warned_skip.insert(dir.to_path_buf())
    };
    if !first {
        return;
    }
    tracing::info!(
        working_dir = %dir.display(),
        "workspace map: no file map for this chat because {}. Start the chat in a project \
         directory to get one.",
        reason.describe()
    );
}

fn warn_budget_once(dir: &Path, budget: Duration) {
    let first = {
        let mut state = STATE.lock().unwrap();
        state.warned_budget.insert(dir.to_path_buf())
    };
    if !first {
        return;
    }
    tracing::warn!(
        working_dir = %dir.display(),
        budget_ms = budget.as_millis() as u64,
        "workspace map: reading this directory did not finish within the budget, so turns here \
         run without a file map. Something under it is slow or unresponsive to read — a network \
         mount or a cloud-sync folder is the usual cause. Set CONTEXT_WORKSPACE_SUMMARY: false in \
         config.yaml to switch the map off, or raise CONTEXT_WORKSPACE_SUMMARY_BUDGET_MS to wait \
         longer."
    );
}

/// The public entry point: a cached, config-bounded workspace map for
/// `working_dir`, or `None` when disabled, skipped, empty, unreadable, or not
/// produced inside this turn's budget.
///
/// `cancel` is the **turn's** cancellation token. The walk it starts cannot
/// itself be cancelled — no blocking `std::fs` call can — but the *wait* can,
/// so a `Stop` returns from here immediately and leaves the walk to finish (or
/// not) on the blocking pool.
pub async fn workspace_summary(
    working_dir: &Path,
    cancel: Option<&CancellationToken>,
) -> Option<String> {
    if !enabled() {
        return None;
    }
    // Started after the disabled bail so the span covers only real work.
    let _phase = crate::agents::phase_timing::Phase::start("agent.workspace_summary");

    if let Some(reason) = skip_reason(working_dir) {
        log_skip_once(working_dir, reason);
        return None;
    }

    // Both of these read config, which is cheap and does not touch the
    // workspace; the walk itself — and every filesystem call it needs, down to
    // resolving the global ignore file — is deferred into the closure so it can
    // only ever run on the blocking pool.
    let cfg = SummaryConfig::from_config();
    let walk_dir = working_dir.to_path_buf();
    summary_bounded(
        working_dir.to_path_buf(),
        cancel,
        ttl(),
        budget(),
        move || build_summary(&walk_dir, &cfg, global_biorouterignore().as_deref()),
    )
    .await
}

/// The cache, single-flight and budget machinery, with the walk itself injected.
///
/// Split out from [`workspace_summary`] so a test can supply a walk that
/// provably stalls or provably blocks until released — the two behaviours this
/// exists to survive, and neither of which can be conjured out of a real
/// directory tree on demand.
async fn summary_bounded<F>(
    key: PathBuf,
    cancel: Option<&CancellationToken>,
    ttl: Duration,
    budget: Duration,
    walk: F,
) -> Option<String>
where
    F: FnOnce() -> Option<String> + Send + 'static,
{
    // Fast path, single-flight admission and the stale value in one short
    // critical section. Deliberately syscall-free — see rule 1 in the module
    // doc.
    let stale = {
        let mut state = STATE.lock().unwrap();
        if let Some(entry) = state.cache.get(&key) {
            if !ttl.is_zero() && entry.computed_at.elapsed() < ttl {
                return entry.summary.clone();
            }
        }
        let cached = state
            .cache
            .get(&key)
            .and_then(|entry| entry.summary.clone());
        if state.inflight.contains(&key) {
            // A walk of this root is already running. Serving whatever it last
            // produced is the whole of the second turn's cost: starting a
            // second walk of a root the first one may be wedged on is how a
            // single slow directory became one leaked thread per turn.
            return cached;
        }
        state.inflight.insert(key.clone());
        *state.walk_starts.entry(key.clone()).or_insert(0) += 1;
        cached
    };

    let task_key = key.clone();
    let mut handle = tokio::task::spawn_blocking(move || {
        let _inflight = InflightGuard(task_key.clone());
        let summary = walk();
        // `ttl == 0` disables caching entirely, as it always has: with no entry
        // written there is nothing for a later turn to serve.
        if !ttl.is_zero() {
            let mut state = STATE.lock().unwrap();
            state.cache.insert(
                task_key,
                CacheEntry {
                    computed_at: Instant::now(),
                    summary: summary.clone(),
                },
            );
        }
        summary
    });

    // `biased` puts cancellation first: a `Stop` must return from here on the
    // very next poll, not after whichever branch happens to be ready.
    let outcome = match cancel {
        Some(token) => tokio::select! {
            biased;
            _ = token.cancelled() => Wait::Cancelled,
            joined = &mut handle => Wait::Finished(joined.ok().flatten()),
            _ = tokio::time::sleep(budget) => Wait::OverBudget,
        },
        None => tokio::select! {
            joined = &mut handle => Wait::Finished(joined.ok().flatten()),
            _ = tokio::time::sleep(budget) => Wait::OverBudget,
        },
    };

    match outcome {
        Wait::Finished(summary) => summary,
        // The turn is going away. Leave the walk running: it owns the
        // single-flight marker and will cache its result for the next turn.
        Wait::Cancelled => None,
        Wait::OverBudget => {
            warn_budget_once(&key, budget);
            if ttl.is_zero() {
                // Caching is off, so there is nothing to stamp and nothing to
                // serve. The walk keeps its single-flight marker regardless.
                return None;
            }
            let mut state = STATE.lock().unwrap();
            let landed = state
                .cache
                .get(&key)
                .and_then(|entry| entry.summary.clone());
            if !state.inflight.contains(&key) {
                // The walk finished in the gap between the timeout firing and
                // this lock. Its result stands; never stamp a negative over it.
                return landed;
            }
            // Cache the negative (or the stale value, if there is one) for the
            // TTL so the next turn pays nothing at all. The walk, if it ever
            // finishes, overwrites this.
            let carry = landed.or(stale);
            state.cache.insert(
                key,
                CacheEntry {
                    computed_at: Instant::now(),
                    summary: carry.clone(),
                },
            );
            carry
        }
    }
}

/// How the wait for a walk ended.
enum Wait {
    /// The walk returned inside the budget. `None` covers both "nothing to
    /// show" and a panicked/aborted task.
    Finished(Option<String>),
    /// The turn was cancelled.
    Cancelled,
    /// The budget lapsed with the walk still running.
    OverBudget,
}

/// Filesystem names can contain almost anything on Unix; scrub the two
/// sequences that would let a crafted filename break out of the enclosing MOIM
/// `<info-msg>` block, plus control characters that would corrupt the listing.
fn sanitize_name(name: &str) -> String {
    name.replace("<info-msg>", "\u{fffd}")
        .replace("</info-msg>", "\u{fffd}")
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// Walk `working_dir` (gitignore- and `.biorouterignore`-aware) and render a
/// bounded, indented file tree. Pure over its inputs so it is unit-testable
/// without the global cache or config. Returns `None` for an empty/unreadable
/// directory (nothing worth injecting).
///
/// **Blocking.** Call it from a blocking context only; [`workspace_summary`] is
/// the async entry point and puts it on the blocking pool.
pub fn build_summary(
    working_dir: &Path,
    cfg: &SummaryConfig,
    global_ignore: Option<&Path>,
) -> Option<String> {
    let home = Paths::home_dir();
    let mut builder = ignore::WalkBuilder::new(working_dir);
    builder
        .max_depth(Some(cfg.max_depth.max(1)))
        // Honor .gitignore even when the workspace isn't a git checkout, so the
        // map is gitignore-aware everywhere.
        .require_git(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .hidden(true)
        .parents(true)
        // A network share or a File Provider volume is a mount, and reading an
        // unresponsive one is the other way this walk blocks. Nothing below a
        // project root that lives on another filesystem is worth that risk.
        .same_file_system(true)
        // The same opaque trees `skip_reason` refuses as roots are pruned here,
        // for a walk that started somewhere legitimate and is about to descend
        // into one.
        .filter_entry(move |entry| {
            entry.depth() == 0 || !is_opaque_tree(entry.path(), home.as_deref())
        })
        // The agent's own project-ignore file, per-directory during the walk.
        .add_custom_ignore_filename(".biorouterignore");
    if let Some(global) = global_ignore {
        builder.add_ignore(global);
    }

    // Collect (relative path, is_dir), bounded by SCAN_CAP so a huge tree can't
    // make this walk unbounded.
    let mut entries: Vec<(PathBuf, bool)> = Vec::new();
    let mut scan_truncated = false;
    for result in builder.build() {
        let Ok(entry) = result else { continue };
        // Skip the root itself.
        if entry.depth() == 0 {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(working_dir) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push((rel.to_path_buf(), is_dir));
        if entries.len() >= SCAN_CAP {
            scan_truncated = true;
            break;
        }
    }

    if entries.is_empty() {
        return None;
    }

    // Sort by path so children fall directly under their parent, yielding a tree
    // when rendered with depth-based indentation.
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let total = entries.len();
    let shown = total.min(cfg.max_entries);
    let mut lines = String::new();
    for (rel, is_dir) in entries.iter().take(shown) {
        let depth = rel.components().count().saturating_sub(1);
        let name = rel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel.to_string_lossy().into_owned());
        let name = sanitize_name(&name);
        for _ in 0..depth {
            lines.push_str("  ");
        }
        lines.push_str(&name);
        if *is_dir {
            lines.push('/');
        }
        lines.push('\n');
    }

    let mut header = String::from(
        "Workspace file map (gitignore-aware; names only, may be truncated or slightly stale; run tools for the live tree):",
    );
    if total > shown || scan_truncated {
        header.push_str(&format!(
            "\n(showing {shown} of {}{} entries; run a directory tool for the rest)",
            total,
            if scan_truncated { "+" } else { "" }
        ));
    }

    let body = format!("{header}\n{}", lines.trim_end());
    // Coordinate with the BR-2 injection budget: never let the map alone exceed
    // its own token cap even if entry/depth caps left it large.
    Some(truncate_to_tokens(&body, cfg.max_tokens, "workspace map"))
}

// ---------------------------------------------------------------------------
// Test-only accessors into the module's process-global state. There is
// deliberately no `clear_cache`: every test keys off a root of its own
// (`unique_key`), because clearing shared state from one test breaks whichever
// other test is running beside it.
//
// Kept together at the bottom, after every production item, because the repo's
// source guards slice a file at its FIRST `#[cfg(test)]` and treat everything
// above it as the production half. An accessor placed mid-file silently
// truncates that view.
// ---------------------------------------------------------------------------

/// How many walks this process has started for `dir`. Per root, so tests using
/// different temp directories do not observe each other.
#[cfg(test)]
fn walk_starts(dir: &Path) -> usize {
    STATE
        .lock()
        .unwrap()
        .walk_starts
        .get(dir)
        .copied()
        .unwrap_or(0)
}

/// Pretend a walk of `dir` is already running, without starting one.
#[cfg(test)]
fn mark_inflight(dir: &Path) {
    STATE.lock().unwrap().inflight.insert(dir.to_path_buf());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Instant as StdInstant;

    fn cfg() -> SummaryConfig {
        SummaryConfig::default()
    }

    /// A root no other test shares, so the process-global cache and
    /// single-flight markers cannot make these tests observe each other.
    fn unique_key(tag: &str) -> PathBuf {
        PathBuf::from(format!(
            "/nonexistent/workspace-summary-test/{tag}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn test_lists_files_and_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("README.md"), "# hi").unwrap();

        let out = build_summary(dir.path(), &cfg(), None).expect("summary");
        assert!(out.contains("src/"), "directory listed with trailing slash");
        assert!(out.contains("main.rs"), "nested file listed");
        assert!(out.contains("README.md"), "top-level file listed");
        // main.rs is indented under src/.
        assert!(out.contains("  main.rs"), "nested file is indented");
    }

    #[test]
    fn test_empty_dir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build_summary(dir.path(), &cfg(), None).is_none());
    }

    #[test]
    fn test_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "secret.txt\nbuild/\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "shh").unwrap();
        fs::write(dir.path().join("keep.txt"), "ok").unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/out.o"), "bin").unwrap();

        let out = build_summary(dir.path(), &cfg(), None).expect("summary");
        assert!(out.contains("keep.txt"), "non-ignored file listed");
        assert!(!out.contains("secret.txt"), "gitignored file excluded");
        assert!(!out.contains("out.o"), "gitignored directory excluded");
    }

    #[test]
    fn test_respects_local_biorouterignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".biorouterignore"), "private/\n").unwrap();
        fs::create_dir_all(dir.path().join("private")).unwrap();
        fs::write(dir.path().join("private/creds.txt"), "top secret").unwrap();
        fs::write(dir.path().join("public.txt"), "ok").unwrap();

        let out = build_summary(dir.path(), &cfg(), None).expect("summary");
        assert!(out.contains("public.txt"));
        assert!(!out.contains("creds.txt"), ".biorouterignore honored");
        assert!(!out.contains("private/"), "ignored directory excluded");
    }

    #[test]
    fn test_respects_global_biorouterignore() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("app.env"), "SECRET=1").unwrap();
        fs::write(workspace.path().join("app.rs"), "fn main(){}").unwrap();

        let global_dir = tempfile::tempdir().unwrap();
        let global_ignore = global_dir.path().join(".biorouterignore");
        fs::write(&global_ignore, "*.env\n").unwrap();

        let out = build_summary(workspace.path(), &cfg(), Some(&global_ignore)).expect("summary");
        assert!(out.contains("app.rs"));
        assert!(!out.contains("app.env"), "global .biorouterignore honored");
    }

    #[test]
    fn test_max_depth_bounds_walk() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c/d")).unwrap();
        fs::write(dir.path().join("a/b/c/d/deep.txt"), "x").unwrap();
        fs::write(dir.path().join("a/top.txt"), "x").unwrap();

        let shallow = SummaryConfig {
            max_depth: 2,
            ..cfg()
        };
        let out = build_summary(dir.path(), &shallow, None).expect("summary");
        assert!(out.contains("top.txt"), "depth-2 file present");
        assert!(
            !out.contains("deep.txt"),
            "depth-5 file excluded by max_depth"
        );
    }

    #[test]
    fn test_max_entries_truncates_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..50 {
            fs::write(dir.path().join(format!("f{i:03}.txt")), "x").unwrap();
        }
        let capped = SummaryConfig {
            max_entries: 10,
            ..cfg()
        };
        let out = build_summary(dir.path(), &capped, None).expect("summary");
        assert!(
            out.contains("showing 10 of 50"),
            "truncation note present: {out}"
        );
        // At most 10 file lines (plus header lines).
        let file_lines = out.lines().filter(|l| l.trim().starts_with('f')).count();
        assert!(
            file_lines <= 10,
            "no more than max_entries files: {file_lines}"
        );
    }

    #[test]
    fn test_token_cap_truncates() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..200 {
            fs::write(dir.path().join(format!("verylongfilename_{i:05}.txt")), "x").unwrap();
        }
        let tiny = SummaryConfig {
            max_entries: 200,
            max_tokens: 50,
            ..cfg()
        };
        let out = build_summary(dir.path(), &tiny, None).expect("summary");
        assert!(
            out.contains("elided to fit the context budget"),
            "token cap applied: {out}"
        );
    }

    #[test]
    fn test_sanitize_name_neutralizes_moim_tags_and_controls() {
        assert_eq!(
            sanitize_name("weird<info-msg>line\nbreak</info-msg>"),
            "weird\u{fffd}line\u{fffd}break\u{fffd}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_filename_cannot_break_moim_block() {
        let dir = tempfile::tempdir().unwrap();
        // A Unix filename can't contain '/', so a literal "</info-msg>" can't
        // exist on disk — but the open tag and embedded newlines/control chars
        // are legal filename bytes, so both must be scrubbed.
        fs::write(dir.path().join("weird<info-msg>name.txt"), "x").unwrap();
        fs::write(dir.path().join("line\nbreak.txt"), "x").unwrap();
        let out = build_summary(dir.path(), &cfg(), None).expect("summary");
        assert!(
            !out.contains("<info-msg>"),
            "MOIM open tag scrubbed from filenames: {out}"
        );
        assert!(
            out.contains("line\u{fffd}break.txt"),
            "embedded newline neutralised so it can't add spurious lines: {out}"
        );
    }

    // -----------------------------------------------------------------------
    // M1: the walk is never on the turn's critical path.
    // -----------------------------------------------------------------------

    /// The finding, in one test: a root whose read never returns must not hold
    /// the turn. Before the fix this call was `build_summary` inline on the
    /// turn's worker and this test would hang forever rather than fail.
    #[tokio::test]
    async fn a_stalled_walk_returns_within_the_budget_instead_of_holding_the_turn() {
        let key = unique_key("stall");
        // Held for the life of the test so the walk closure blocks exactly as a
        // wedged `opendir` does: forever, with no cancellation point.
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let started = StdInstant::now();
        let summary = summary_bounded(
            key.clone(),
            None,
            Duration::from_secs(30),
            Duration::from_millis(50),
            move || {
                // Blocks until the sender is dropped at the end of the test.
                let _ = release_rx.recv();
                Some("a map nobody waited for".to_string())
            },
        )
        .await;
        let waited = started.elapsed();

        assert_eq!(summary, None, "the turn proceeds without a map");
        assert!(
            waited < Duration::from_secs(5),
            "returned in {waited:?}; the budget was 50ms, so this waited for the walk"
        );
        drop(release_tx);
    }

    /// The negative is cached, so a second turn inside the TTL pays nothing at
    /// all — not even the budget.
    #[tokio::test]
    async fn a_lapsed_budget_is_cached_so_the_next_turn_does_not_pay_again() {
        let key = unique_key("negative-cache");
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let first = summary_bounded(
            key.clone(),
            None,
            Duration::from_secs(30),
            Duration::from_millis(50),
            move || {
                let _ = release_rx.recv();
                None
            },
        )
        .await;
        assert_eq!(first, None);

        let started = StdInstant::now();
        let second = summary_bounded(
            key.clone(),
            None,
            Duration::from_secs(30),
            Duration::from_millis(50),
            || panic!("a second walk must not be started"),
        )
        .await;
        let waited = started.elapsed();

        assert_eq!(second, None);
        assert!(
            waited < Duration::from_millis(50),
            "served from the negative cache, so it never reached the budget: {waited:?}"
        );
        assert_eq!(walk_starts(&key), 1, "exactly one walk was ever started");
        drop(release_tx);
    }

    /// A blocked `std::fs` call cannot be cancelled, so the thread it is on is
    /// lost. Single-flight is what bounds that loss to one thread per root
    /// rather than one per turn.
    #[tokio::test]
    async fn a_second_turn_never_starts_a_second_walk_of_the_same_root() {
        let key = unique_key("single-flight");
        mark_inflight(&key);

        let summary = summary_bounded(
            key.clone(),
            None,
            Duration::from_secs(30),
            Duration::from_secs(30),
            || panic!("single-flight failed: a second walk of this root was started"),
        )
        .await;

        assert_eq!(summary, None, "nothing cached yet, so nothing to serve");
        assert_eq!(walk_starts(&key), 0, "no walk was started");
    }

    /// While one walk is in flight, a second turn is served whatever the last
    /// completed walk produced rather than being made to wait or to re-walk.
    #[tokio::test]
    async fn an_inflight_root_serves_the_last_good_map_to_the_next_turn() {
        let key = unique_key("serve-stale");

        let first = summary_bounded(
            key.clone(),
            None,
            Duration::from_millis(1),
            Duration::from_secs(30),
            || Some("first map".to_string()),
        )
        .await;
        assert_eq!(first.as_deref(), Some("first map"));

        // The TTL above has already lapsed, so the next call would ordinarily
        // re-walk; the in-flight marker says another one is already running.
        mark_inflight(&key);
        let second = summary_bounded(
            key.clone(),
            None,
            Duration::from_millis(1),
            Duration::from_secs(30),
            || panic!("single-flight failed"),
        )
        .await;

        assert_eq!(
            second.as_deref(),
            Some("first map"),
            "the last good map is served rather than nothing"
        );
        assert_eq!(walk_starts(&key), 1);
    }

    /// M2's other half: `Stop` must be honoured immediately. The walk cannot be
    /// cancelled, but the wait for it can.
    #[tokio::test]
    async fn a_cancelled_turn_returns_immediately_and_leaves_the_walk_running() {
        let key = unique_key("cancel");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let started = StdInstant::now();
        let summary = summary_bounded(
            key.clone(),
            Some(&cancel),
            Duration::from_secs(30),
            Duration::from_secs(30),
            move || {
                let _ = release_rx.recv();
                Some("too late".to_string())
            },
        )
        .await;
        let waited = started.elapsed();

        assert_eq!(summary, None);
        assert!(
            waited < Duration::from_secs(1),
            "cancellation returned in {waited:?}, not after the 30s budget"
        );
        drop(release_tx);
    }

    /// A walk that finishes inside the budget is served, and cached for the TTL.
    #[tokio::test]
    async fn a_fast_walk_is_served_and_then_cached_for_the_ttl() {
        let key = unique_key("happy");

        let first = summary_bounded(
            key.clone(),
            None,
            Duration::from_secs(30),
            Duration::from_secs(30),
            || Some("the map".to_string()),
        )
        .await;
        assert_eq!(first.as_deref(), Some("the map"));

        let second = summary_bounded(
            key.clone(),
            None,
            Duration::from_secs(30),
            Duration::from_secs(30),
            || panic!("a cached map must not be re-walked inside the TTL"),
        )
        .await;
        assert_eq!(second.as_deref(), Some("the map"));
        assert_eq!(walk_starts(&key), 1);
    }

    // -----------------------------------------------------------------------
    // M1: a home directory is not a workspace.
    // -----------------------------------------------------------------------

    #[test]
    fn the_home_directory_itself_gets_no_map() {
        let home = Path::new("/Users/example");
        assert_eq!(
            skip_reason_with_home(home, Some(home)),
            Some(SkipReason::Home)
        );
        // A trailing slash is the same directory.
        assert_eq!(
            skip_reason_with_home(Path::new("/Users/example/"), Some(home)),
            Some(SkipReason::Home)
        );
        // A project inside it is fine.
        assert_eq!(
            skip_reason_with_home(Path::new("/Users/example/code/thing"), Some(home)),
            None
        );
    }

    #[test]
    fn a_filesystem_root_gets_no_map() {
        assert_eq!(
            skip_reason_with_home(Path::new("/"), Some(Path::new("/Users/example"))),
            Some(SkipReason::FilesystemRoot)
        );
    }

    #[test]
    fn the_directories_that_produced_the_wedge_get_no_map() {
        let home = Path::new("/Users/example");
        for path in [
            "/Users/example/Library",
            "/Users/example/Library/Group Containers",
            "/Users/example/Library/Group Containers/group.com.apple.CloudDocs",
            "/Users/example/Library/CloudStorage/Dropbox",
            "/Users/example/Library/Mobile Documents/com~apple~CloudDocs",
            "/Users/example/.Trash",
        ] {
            assert_eq!(
                skip_reason_with_home(Path::new(path), Some(home)),
                Some(SkipReason::OpaqueTree),
                "{path} must get no workspace map"
            );
        }
    }

    #[test]
    fn a_project_directory_named_library_is_still_walkable() {
        let home = Path::new("/Users/example");
        // The `Library` rule is home-relative on purpose: an R project, or a
        // component library, may hold a directory of that name.
        assert_eq!(
            skip_reason_with_home(Path::new("/Users/example/code/app/Library"), Some(home)),
            None
        );
        assert_eq!(
            skip_reason_with_home(Path::new("/srv/project/library"), Some(home)),
            None
        );
    }

    #[test]
    fn an_opaque_subtree_is_pruned_from_a_walk_that_started_somewhere_legitimate() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "x").unwrap();
        fs::create_dir_all(dir.path().join("Group Containers/group.com.example")).unwrap();
        fs::write(
            dir.path()
                .join("Group Containers/group.com.example/blocked.txt"),
            "x",
        )
        .unwrap();

        let out = build_summary(dir.path(), &cfg(), None).expect("summary");
        assert!(out.contains("keep.txt"));
        assert!(
            !out.contains("blocked.txt"),
            "nothing under an opaque component is read: {out}"
        );
        assert!(
            !out.contains("group.com.example"),
            "the opaque tree is not descended into: {out}"
        );
    }

    /// A scratch directory is an ordinary workspace, and on Windows every one of
    /// them lives under `%USERPROFILE%\AppData\Local\Temp`. An `AppData` entry
    /// in the opaque list would therefore refuse a legitimate root on one
    /// platform and not the other two, and would empty the walk in every
    /// `build_summary` test above — which is how this was caught.
    #[test]
    fn a_temporary_directory_is_a_legitimate_workspace_on_every_platform() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            skip_reason(dir.path()),
            None,
            "a temp dir must get a workspace map: {}",
            dir.path().display()
        );
        // The same rule, stated without depending on where this platform puts
        // its temp directory: a Windows-shaped scratch path under the home
        // directory is a workspace like any other.
        let home = Path::new("/Users/example");
        assert_eq!(
            skip_reason_with_home(
                Path::new("/Users/example/AppData/Local/Temp/.tmpABC123"),
                Some(home)
            ),
            None
        );
        std::fs::write(dir.path().join("kept.txt"), "x").unwrap();
        let out = build_summary(dir.path(), &cfg(), None).expect("temp dirs are walked");
        assert!(out.contains("kept.txt"));
    }

    #[test]
    fn the_budget_can_never_be_configured_to_zero() {
        // A zero budget would be a way to put an uncancellable filesystem walk
        // back on the turn's critical path, which is the whole bug. The only
        // supported "off" is CONTEXT_WORKSPACE_SUMMARY: false.
        assert!(!budget().is_zero());
        assert_eq!(DEFAULT_BUDGET_MS, 1_500);
    }
}
