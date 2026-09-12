//! BR-58: bound tool parallelism and add write-side ordering.
//!
//! Before this module, approved tool futures were merged with
//! `stream::select_all` (`agent.rs`) and polled concurrently with **no**
//! concurrency cap and **no** cross-tool isolation. An assistant message with
//! many tool calls therefore ran them all at once, and two write-side calls
//! targeting the same file (e.g. concurrent `text_editor` `str_replace`s) could
//! interleave with no ordering guarantee — the last writer wins non-
//! deterministically and an edit can land on top of a half-written file.
//!
//! Two independent mechanisms fix this, mirroring the subagent
//! `SUBAGENT_SEMAPHORE` and Codex CLI's exclusive write-lock model:
//!
//! 1. **A global concurrency semaphore** ([`acquire`]) over every dispatched
//!    tool future. Generous default (8, like subagents), env-overridable via
//!    `BIOROUTER_TOOL_MAX_CONCURRENT`. This caps the thundering-herd on disk /
//!    network without meaningfully slowing legitimately-parallel read tools.
//!
//! 2. **Per-path exclusive write locks.** A write-side tool that targets one or
//!    more files takes an exclusive lock on each (lexically-normalised) path for
//!    the duration of its execution, so two writers to overlapping paths run
//!    strictly one-after-the-other. Read tools and writers to disjoint paths are
//!    unaffected. Toggle with `BIOROUTER_TOOL_WRITE_ORDERING` (`0`/`false` off).
//!
//! Lock order is always **semaphore → path locks (sorted)**, so there is no
//! circular waiting and the design is deadlock-free: a running tool only ever
//! holds resources and runs to completion; it never waits on a resource a parked
//! tool holds.
//!
//! ⚠ **That last sentence is a rule about the tools, not a property of this
//! module**, and two things have broken it since. `workspace_watch` /
//! `workspace_send_prompt` park on work in other sessions and are exempted from
//! the permit by name (`agent::is_parking_workspace_tool`). `execute_code` parks
//! on a *person* — since QA finding F7 a script's own tool calls raise approval
//! cards — but it is not a do-nothing wrapper, and in the shipped Code Execution
//! default it is very nearly the only tool there is, so exempting it by name
//! would leave this semaphore bounding nothing at all. It instead hands the
//! permit back for exactly the parked interval and queues for it again
//! afterwards: [`ToolDispatchGuard::parking_handle`] and
//! [`DispatchPermitHandle::while_parked`].

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use regex::Regex;
use serde_json::{Map, Value};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

/// Generous default parallel-tool ceiling, matching the subagent default.
const DEFAULT_MAX_CONCURRENT_TOOLS: usize = 8;

/// The configured maximum number of tool futures allowed to run concurrently.
///
/// `BIOROUTER_TOOL_MAX_CONCURRENT` overrides it; a non-positive or unparseable
/// value falls back to the default. Set it very high to effectively disable the
/// cap.
pub fn max_concurrent_tools() -> usize {
    std::env::var("BIOROUTER_TOOL_MAX_CONCURRENT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_CONCURRENT_TOOLS)
}

/// Whether write-side path serialization is active. On by default; only an
/// explicit `0`/`false`/`no`/`off` disables it (an escape hatch — leaving it on
/// is strictly safer).
fn write_ordering_enabled() -> bool {
    match std::env::var("BIOROUTER_TOOL_WRITE_ORDERING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// §6.2c: whether a tool response is streamed to the transcript the instant it
/// completes (in COMPLETION order) rather than after the whole batch finishes
/// (in request order). On by default; only an explicit `0`/`false`/`no`/`off`
/// in `BIOROUTER_TOOL_RESPONSE_STREAMING` restores the pre-§6.2c behaviour where
/// every response is yielded from the post-batch loop in request order — a full
/// rollback, mirroring `BIOROUTER_TOOL_WRITE_ORDERING`/`BIOROUTER_TOOL_CALL_BATCHING`.
///
/// This flag changes only the *streamed transcript* ordering: the PERSISTED
/// `messages_to_add` is built in request order either way (invariant §6.5-2), so
/// SQLite session history and every provider replay are identical regardless.
pub fn tool_response_streaming_enabled() -> bool {
    match std::env::var("BIOROUTER_TOOL_RESPONSE_STREAMING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

static TOOL_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(max_concurrent_tools())));

/// Registry of per-path exclusive locks. `Weak` so a path's lock is dropped once
/// no tool references it, keeping the map from growing unbounded across a long-
/// running daemon's lifetime. The outer `std::Mutex` only guards the (brief)
/// map lookup/insert; the per-path `AsyncMutex` is what a tool actually holds
/// across its `await`s.
static PATH_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Prune dead (dropped) weak entries once the map grows past this many paths, so
/// a session that touches thousands of distinct files does not leak map slots.
const PATH_LOCK_PRUNE_THRESHOLD: usize = 1024;

/// The concurrency permit, in a cell a tool that parks on a person can hand it
/// back through. `None` means this dispatch holds no permit — an exempt tool, a
/// closed semaphore in a test teardown, or a park in progress.
type PermitCell = Arc<AsyncMutex<Option<OwnedSemaphorePermit>>>;

/// RAII guard held for the lifetime of a tool's execution. Dropping it releases
/// the concurrency permit and any write-path locks.
pub struct ToolDispatchGuard {
    permit: PermitCell,
    /// The semaphore `permit` came from, so a [`DispatchPermitHandle`] queues for
    /// the same one it handed a permit back to. A field rather than a reach for
    /// the static, because the tests need their own.
    semaphore: Arc<Semaphore>,
    _path_guards: Vec<OwnedMutexGuard<()>>,
}

impl ToolDispatchGuard {
    /// A handle for a tool that **parks on a person** to hand its concurrency
    /// permit back for the duration of the wait.
    ///
    /// Only for genuine parking — an approval card, an elicitation — never around
    /// work. The permit exists to bound work, and a tool waiting on a human is
    /// doing none; holding one there is what turns eight parked calls into a
    /// stalled daemon (`Semaphore::new(8)`, shared by every session in the
    /// process), which is the hazard `agent::is_parking_workspace_tool` was
    /// added for.
    pub fn parking_handle(&self) -> DispatchPermitHandle {
        DispatchPermitHandle {
            // Weak: a handle that outlives the dispatch it came from must not
            // keep that dispatch's permit alive.
            permit: Arc::downgrade(&self.permit),
            semaphore: Arc::clone(&self.semaphore),
        }
    }
}

/// A tool's way of not holding its dispatch permit while it is parked on a
/// person. See [`ToolDispatchGuard::parking_handle`].
#[derive(Clone)]
pub struct DispatchPermitHandle {
    permit: Weak<AsyncMutex<Option<OwnedSemaphorePermit>>>,
    semaphore: Arc<Semaphore>,
}

impl DispatchPermitHandle {
    /// Await `parked` holding no concurrency permit, then queue for one again
    /// before the tool resumes.
    ///
    /// Waiting for the permit back starves nobody: this call holds nothing while
    /// it waits, which is the whole difference from the situation it replaces. If
    /// `parked` is dropped part-way (a cancelled turn) the permit simply stays
    /// released and the guard drops with nothing to free.
    pub async fn while_parked<F: std::future::Future>(&self, parked: F) -> F::Output {
        let Some(cell) = self.permit.upgrade() else {
            // The dispatch this handle came from is already over.
            return parked.await;
        };
        let handed_back = cell.lock().await.take();
        if handed_back.is_none() {
            // Nothing to hand back: an exempt dispatch, or an outer park already
            // did. Never hold the cell's lock across the await below.
            return parked.await;
        }
        drop(handed_back);

        let outcome = parked.await;
        if let Ok(permit) = self.semaphore.clone().acquire_owned().await {
            *cell.lock().await = Some(permit);
        }
        outcome
    }
}

/// Acquire the concurrency permit and any write-path locks for a tool call, in
/// deadlock-free order (permit first, then paths sorted). The returned guard
/// must be held for the whole duration of the tool's execution.
pub async fn acquire(
    tool_name: &str,
    arguments: Option<&Map<String, Value>>,
    working_dir: &Path,
) -> ToolDispatchGuard {
    // 1. Bound total parallelism. The static Semaphore never closes, so a
    //    failure here can only mean a poisoned/closed sem in a test teardown —
    //    fail open (run the tool) rather than wedge the loop.
    let permit = acquire_permit_cell(&TOOL_SEMAPHORE).await;

    // 2. Serialize overlapping write paths.
    let path_guards = if write_ordering_enabled() {
        let paths = write_paths_for_tool(tool_name, arguments, working_dir);
        acquire_path_locks(paths).await
    } else {
        Vec::new()
    };

    ToolDispatchGuard {
        permit,
        semaphore: TOOL_SEMAPHORE.clone(),
        _path_guards: path_guards,
    }
}

/// Take one permit from `semaphore` into a cell a park can hand it back through.
async fn acquire_permit_cell(semaphore: &Arc<Semaphore>) -> PermitCell {
    Arc::new(AsyncMutex::new(
        semaphore.clone().acquire_owned().await.ok(),
    ))
}

/// Take an exclusive lock on each path, in a stable sorted order so that two
/// tools locking an overlapping set can never deadlock on acquisition order.
async fn acquire_path_locks(mut paths: Vec<PathBuf>) -> Vec<OwnedMutexGuard<()>> {
    if paths.is_empty() {
        return Vec::new();
    }
    paths.sort();
    paths.dedup();

    // Resolve each path to its shared AsyncMutex under the brief map lock, then
    // await the locks outside it (never hold the std mutex across an await).
    let mutexes: Vec<Arc<AsyncMutex<()>>> = {
        let mut map = PATH_LOCKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if map.len() > PATH_LOCK_PRUNE_THRESHOLD {
            map.retain(|_, weak| weak.strong_count() > 0);
        }

        paths
            .iter()
            .map(|path| match map.get(path).and_then(Weak::upgrade) {
                Some(existing) => existing,
                None => {
                    let fresh = Arc::new(AsyncMutex::new(()));
                    map.insert(path.clone(), Arc::downgrade(&fresh));
                    fresh
                }
            })
            .collect()
    };

    let mut guards = Vec::with_capacity(mutexes.len());
    for mutex in mutexes {
        guards.push(mutex.lock_owned().await);
    }
    guards
}

// ---------------------------------------------------------------------------
// Write-side path classification
//
// Mirrors the desktop `fileArtifactPathsFromToolCall` tool→path mapping so the
// two stay in step: the same set of tools that "leave a file changed on disk"
// are the ones whose writes we serialize.
// ---------------------------------------------------------------------------

/// Tools whose arguments name a file the agent is creating or rewriting.
const FILE_WRITING_TOOLS: &[&str] = &[
    "create_file",
    "edit_file",
    "multi_edit",
    "notebook_edit",
    "str_replace_editor",
    "text_editor",
    "write_file",
];

/// `text_editor`-style commands that leave a file changed on disk. `view` and
/// `undo_edit` do not mutate.
const MUTATING_EDITOR_COMMANDS: &[&str] = &["create", "diff", "insert", "str_replace", "write"];

/// Argument keys under which a write tool names its target file.
const PATH_ARGUMENT_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filePath",
    "filename",
    "file_name",
    "target_file",
    "absolute_path",
];

/// Shell redirections and the conventional output flags. Deliberately narrow —
/// matching every path-like token in a command turns an `ls` into spurious
/// write locks.
static SHELL_OUTPUT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:>>?|(?:^|\s)(?:--outfile|--output|--out|-out|-o)[=\s]+)\s*(?:"([^"]+)"|'([^']+)'|([^\s;|&>]+))"#,
    )
    .expect("valid shell-output regex")
});

/// The base tool name as the model calls it, stripping any `extension__` prefix.
fn base_tool_name(tool_name: &str) -> &str {
    tool_name
        .rsplit_once("__")
        .map_or(tool_name, |(_, base_name)| base_name)
}

/// The absolute, lexically-normalised paths a tool call is about to write. Empty
/// for read-only tools, non-mutating editor commands, or paths that cannot be
/// safely anchored to `working_dir`.
pub fn write_paths_for_tool(
    tool_name: &str,
    arguments: Option<&Map<String, Value>>,
    working_dir: &Path,
) -> Vec<PathBuf> {
    let Some(args) = arguments else {
        return Vec::new();
    };
    let name = base_tool_name(tool_name);

    if name == "shell" || name == "bash" {
        let Some(Value::String(command)) = args.get("command") else {
            return Vec::new();
        };
        let mut paths = Vec::new();
        for caps in SHELL_OUTPUT_RE.captures_iter(command) {
            let candidate = caps
                .get(1)
                .or_else(|| caps.get(2))
                .or_else(|| caps.get(3))
                .map(|m| m.as_str());
            if let Some(resolved) = candidate.and_then(|c| resolve_path(c, working_dir)) {
                paths.push(resolved);
            }
        }
        return paths;
    }

    if !FILE_WRITING_TOOLS.contains(&name) {
        return Vec::new();
    }

    // `text_editor` multiplexes read and write behind a `command` argument.
    if let Some(Value::String(command)) = args.get("command") {
        if !MUTATING_EDITOR_COMMANDS.contains(&command.as_str()) {
            return Vec::new();
        }
    }

    for key in PATH_ARGUMENT_KEYS {
        if let Some(Value::String(value)) = args.get(*key) {
            if value.trim().is_empty() {
                continue;
            }
            return resolve_path(value, working_dir).into_iter().collect();
        }
    }
    Vec::new()
}

/// `C:\dir`, `C:/dir` or a `\\server\share` UNC path.
fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    path.starts_with("\\\\")
}

/// Resolve a possibly-relative path against `working_dir` and normalise it
/// lexically, so two spellings of the same file map to one lock key. Returns
/// `None` for paths that escape the working dir (`..`) without an absolute
/// anchor, so an unanchored relative path never resolves against the wrong dir.
fn resolve_path(raw: &str, working_dir: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    if trimmed.is_empty() {
        return None;
    }

    let absolute =
        if trimmed.starts_with('/') || trimmed.starts_with('~') || is_windows_absolute(trimmed) {
            PathBuf::from(trimmed)
        } else {
            // Relative: anchor to the working dir, rejecting parent-escapes and the
            // bare `.`/`..` the way the desktop resolver does.
            let cleaned = trimmed
                .strip_prefix("./")
                .or_else(|| trimmed.strip_prefix(".\\"))
                .unwrap_or(trimmed);
            if cleaned == "."
                || cleaned == ".."
                || cleaned.starts_with("../")
                || cleaned.starts_with("..\\")
            {
                return None;
            }
            working_dir.join(cleaned)
        };

    Some(normalize_lexically(&absolute))
}

/// Collapse `.` and `..` components without touching the filesystem (the target
/// file may not exist yet), so `/a/b/../c.txt` and `/a/c.txt` share a lock key.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn base_tool_name_handles_unicode_prefixes() {
        assert_eq!(base_tool_name("développeur__text_editor"), "text_editor");
        assert_eq!(base_tool_name("text_editor"), "text_editor");
    }

    #[test]
    fn text_editor_write_yields_target_path() {
        let a = args(json!({ "command": "write", "path": "/tmp/report.md" }));
        let paths = write_paths_for_tool("developer__text_editor", Some(&a), Path::new("/work"));
        assert_eq!(paths, vec![PathBuf::from("/tmp/report.md")]);
    }

    #[test]
    fn text_editor_str_replace_and_insert_are_writes() {
        for cmd in ["str_replace", "insert", "create", "diff"] {
            let a = args(json!({ "command": cmd, "path": "/tmp/f.txt" }));
            let paths = write_paths_for_tool("text_editor", Some(&a), Path::new("/work"));
            assert_eq!(paths, vec![PathBuf::from("/tmp/f.txt")], "cmd={cmd}");
        }
    }

    #[test]
    fn text_editor_view_and_undo_are_not_writes() {
        for cmd in ["view", "undo_edit"] {
            let a = args(json!({ "command": cmd, "path": "/tmp/f.txt" }));
            let paths =
                write_paths_for_tool("developer__text_editor", Some(&a), Path::new("/work"));
            assert!(paths.is_empty(), "cmd={cmd} should not lock a path");
        }
    }

    #[test]
    fn relative_write_path_anchors_to_working_dir() {
        let a = args(json!({ "command": "write", "path": "results/plot.svg" }));
        let paths = write_paths_for_tool("text_editor", Some(&a), Path::new("/work/session"));
        assert_eq!(paths, vec![PathBuf::from("/work/session/results/plot.svg")]);
    }

    #[test]
    fn parent_escape_relative_path_is_rejected() {
        let a = args(json!({ "command": "write", "path": "../secret.txt" }));
        let paths = write_paths_for_tool("text_editor", Some(&a), Path::new("/work"));
        assert!(paths.is_empty());
    }

    #[test]
    fn write_file_uses_file_path_key() {
        let a = args(json!({ "file_path": "/tmp/out.json", "content": "{}" }));
        let paths = write_paths_for_tool("developer__write_file", Some(&a), Path::new("/work"));
        assert_eq!(paths, vec![PathBuf::from("/tmp/out.json")]);
    }

    #[test]
    fn distinct_spellings_normalise_to_one_key() {
        let a1 = args(json!({ "command": "write", "path": "/a/b/../c.txt" }));
        let a2 = args(json!({ "command": "write", "path": "/a/./c.txt" }));
        let p1 = write_paths_for_tool("text_editor", Some(&a1), Path::new("/work"));
        let p2 = write_paths_for_tool("text_editor", Some(&a2), Path::new("/work"));
        assert_eq!(p1, p2);
        assert_eq!(p1, vec![PathBuf::from("/a/c.txt")]);
    }

    #[test]
    fn shell_redirect_targets_are_captured() {
        let a = args(json!({ "command": "echo hi > /tmp/o.txt && cat x" }));
        let paths = write_paths_for_tool("developer__shell", Some(&a), Path::new("/work"));
        assert_eq!(paths, vec![PathBuf::from("/tmp/o.txt")]);
    }

    #[test]
    fn shell_output_flag_relative_is_anchored() {
        let a = args(json!({ "command": "mytool --output out/result.csv" }));
        let paths = write_paths_for_tool("shell", Some(&a), Path::new("/w"));
        assert_eq!(paths, vec![PathBuf::from("/w/out/result.csv")]);
    }

    #[test]
    fn plain_read_shell_locks_nothing() {
        let a = args(json!({ "command": "ls -la /etc | grep passwd" }));
        let paths = write_paths_for_tool("developer__shell", Some(&a), Path::new("/work"));
        assert!(paths.is_empty());
    }

    #[test]
    fn unknown_and_read_tools_lock_nothing() {
        let a = args(json!({ "path": "/tmp/x", "query": "foo" }));
        assert!(write_paths_for_tool("some__search", Some(&a), Path::new("/work")).is_empty());
        assert!(write_paths_for_tool("read_file", Some(&a), Path::new("/work")).is_empty());
        assert!(write_paths_for_tool("text_editor", None, Path::new("/work")).is_empty());
    }

    #[test]
    fn default_concurrency_is_generous() {
        // Only asserts the compiled default; env override is process-global and
        // would race other tests, so it is not exercised here.
        assert_eq!(DEFAULT_MAX_CONCURRENT_TOOLS, 8);
    }

    /// A tool parked on a person holds no permit, and takes one back — queueing
    /// if it must — before it resumes.
    ///
    /// Against its own one-permit semaphore rather than the process-global
    /// `TOOL_SEMAPHORE`, so the assertions are exact instead of racing every other
    /// test in this binary.
    #[tokio::test]
    async fn a_parked_tool_hands_its_permit_back_and_queues_for_it_again() {
        let semaphore = Arc::new(Semaphore::new(1));
        let guard = ToolDispatchGuard {
            permit: acquire_permit_cell(&semaphore).await,
            semaphore: Arc::clone(&semaphore),
            _path_guards: Vec::new(),
        };
        assert_eq!(
            semaphore.available_permits(),
            0,
            "the tool holds the permit"
        );

        let handle = guard.parking_handle();
        let (answer, parked) = tokio::sync::oneshot::channel::<()>();
        let waiting = tokio::spawn(async move { handle.while_parked(parked).await });

        for _ in 0..200 {
            if semaphore.available_permits() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            semaphore.available_permits(),
            1,
            "a parked tool must hold no permit — eight of them would stall the daemon"
        );

        // Somebody else takes the freed permit, so resuming has to queue.
        let other = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .expect("a permit");
        answer.send(()).expect("the park is still waiting");
        drop(other);

        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the parked tool resumes once a permit frees")
            .expect("the parked task completes")
            .expect("the park resolves");
        assert_eq!(
            semaphore.available_permits(),
            0,
            "the permit is taken back for the rest of the tool's work"
        );
        drop(guard);
        assert_eq!(semaphore.available_permits(), 1, "…and freed on drop");
    }

    /// A handle whose dispatch is already over must not resurrect a permit.
    #[tokio::test]
    async fn a_handle_that_outlived_its_dispatch_is_inert() {
        let semaphore = Arc::new(Semaphore::new(1));
        let guard = ToolDispatchGuard {
            permit: acquire_permit_cell(&semaphore).await,
            semaphore: Arc::clone(&semaphore),
            _path_guards: Vec::new(),
        };
        let handle = guard.parking_handle();
        drop(guard);
        assert_eq!(semaphore.available_permits(), 1);
        handle.while_parked(std::future::ready(())).await;
        assert_eq!(
            semaphore.available_permits(),
            1,
            "an inert handle must not take a permit nobody will release"
        );
    }

    #[tokio::test]
    async fn same_path_writes_are_serialized() {
        let path = vec![PathBuf::from("/work/shared.txt")];

        // Hold the lock, then confirm a second acquisition of the SAME path
        // cannot complete until the first is dropped.
        let g1 = acquire_path_locks(path.clone()).await;

        let blocked =
            tokio::time::timeout(Duration::from_millis(50), acquire_path_locks(path.clone())).await;
        assert!(blocked.is_err(), "second same-path acquire must block");

        drop(g1);
        let g2 = tokio::time::timeout(Duration::from_millis(500), acquire_path_locks(path.clone()))
            .await;
        assert!(g2.is_ok(), "acquire must proceed once the first lock frees");
    }

    #[tokio::test]
    async fn disjoint_paths_do_not_block() {
        let a = acquire_path_locks(vec![PathBuf::from("/work/a.txt")]).await;
        let b = tokio::time::timeout(
            Duration::from_millis(50),
            acquire_path_locks(vec![PathBuf::from("/work/b.txt")]),
        )
        .await;
        assert!(b.is_ok(), "disjoint paths must not serialize");
        drop((a, b));
    }

    #[tokio::test]
    async fn concurrent_same_path_never_overlaps() {
        // Two tasks racing on the same path: the in-critical-section counter
        // must never exceed 1.
        let path = PathBuf::from("/work/race.txt");
        let live = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let path = path.clone();
            let live = live.clone();
            let max = max.clone();
            handles.push(tokio::spawn(async move {
                let _g = acquire_path_locks(vec![path]).await;
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                live.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(max.load(Ordering::SeqCst), 1, "same-path writes overlapped");
    }
}
