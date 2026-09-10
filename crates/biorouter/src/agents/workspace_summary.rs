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
//! - Results are cached per working directory and only recomputed when the
//!   directory's own mtime changes or a short TTL lapses
//!   (`CONTEXT_WORKSPACE_SUMMARY_TTL_SECS`), so a long multi-tool turn walks the
//!   tree at most once per TTL rather than on every provider call.
//!
//! The whole feature is gated by `CONTEXT_WORKSPACE_SUMMARY` (default on); set it
//! to `false` to fall back to the old one-line behavior.

use crate::config::paths::Paths;
use crate::config::Config;
use crate::context_budget::truncate_to_tokens;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Whether the workspace map is injected at all.
pub const DEFAULT_ENABLED: bool = true;
/// Maximum rendered entries (files + directories) before the tail is elided.
pub const DEFAULT_MAX_ENTRIES: usize = 200;
/// Maximum directory depth walked below the working directory.
pub const DEFAULT_MAX_DEPTH: usize = 3;
/// Token cap for the rendered map (coordinates with the BR-2 MOIM budget).
pub const DEFAULT_MAX_TOKENS: usize = 2_000;
/// Cache time-to-live: an upper bound on how stale the served map can be when
/// the working directory's own mtime hasn't changed (nested edits).
pub const DEFAULT_TTL_SECS: u64 = 30;

/// Hard ceiling on filesystem entries visited during a single walk, independent
/// of `max_entries`, so a pathological directory (100k flat files) can't make
/// the bounded walk unbounded. If hit, the map is marked truncated.
const SCAN_CAP: usize = 20_000;

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
    /// The working directory's own mtime when this entry was computed, if
    /// readable. Used as a cheap early-invalidation signal for top-level
    /// structural changes (new/removed/renamed direct children).
    dir_mtime: Option<SystemTime>,
    /// `None` means "walked, nothing to show" — still cached so an empty or
    /// unreadable directory isn't rewalked every action.
    summary: Option<String>,
}

static CACHE: Lazy<Mutex<HashMap<PathBuf, CacheEntry>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The key `working_dir` is cached under. Canonicalized so two spellings of one
/// directory share an entry; shared with [`forget_cached`] so a test can never
/// clear a key production would not have written.
fn cache_key(working_dir: &Path) -> PathBuf {
    std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf())
}

/// Drop one working directory's cached map. Test-only; production relies on
/// TTL/mtime invalidation.
///
/// Scoped to a single key, and deliberately not a whole-map `clear()`: `CACHE`
/// is process-global, and `cargo test` runs a crate's unit tests in parallel
/// threads of one process. Emptying it would reach into entries belonging to
/// whatever else is mid-flight — `collect_moim` caches under its own working
/// directory on every turn, and has a test of its own that does. Because each
/// test owns a unique `tempdir` key, keeping the reach this narrow is what lets
/// the cache tests run unserialized.
#[cfg(test)]
pub fn forget_cached(working_dir: &Path) {
    CACHE.lock().unwrap().remove(&cache_key(working_dir));
}

fn dir_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

/// Absolute path to the global `.biorouterignore`, mirroring the developer MCP
/// server so the workspace map honors the same global excludes.
fn global_biorouterignore() -> Option<PathBuf> {
    let path = Paths::config_dir().join(".biorouterignore");
    path.is_file().then_some(path)
}

/// The public entry point: a cached, config-bounded workspace map for
/// `working_dir`, or `None` when disabled, empty, or unreadable.
pub fn workspace_summary(working_dir: &Path) -> Option<String> {
    if !enabled() {
        return None;
    }
    // Started after the disabled bail so the span covers only real work (this
    // does synchronous directory walking on the turn path when the cache misses).
    let _phase = crate::agents::phase_timing::Phase::start("agent.workspace_summary");

    let ttl = Duration::from_secs(config_u64(
        "CONTEXT_WORKSPACE_SUMMARY_TTL_SECS",
        DEFAULT_TTL_SECS,
    ));
    let key = cache_key(working_dir);
    let current_mtime = dir_mtime(working_dir);

    // Fast path: serve a cached map when the directory's mtime is unchanged and
    // the entry is younger than the TTL. `ttl == 0` disables caching entirely.
    if !ttl.is_zero() {
        if let Some(entry) = CACHE.lock().unwrap().get(&key) {
            let fresh = entry.computed_at.elapsed() < ttl && entry.dir_mtime == current_mtime;
            if fresh {
                return entry.summary.clone();
            }
        }
    }

    let cfg = SummaryConfig::from_config();
    let summary = build_summary(working_dir, &cfg, global_biorouterignore().as_deref());

    if !ttl.is_zero() {
        CACHE.lock().unwrap().insert(
            key,
            CacheEntry {
                computed_at: Instant::now(),
                dir_mtime: current_mtime,
                summary: summary.clone(),
            },
        );
    }
    summary
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
pub fn build_summary(
    working_dir: &Path,
    cfg: &SummaryConfig,
    global_ignore: Option<&Path>,
) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::FileTime;
    use std::fs;

    fn cfg() -> SummaryConfig {
        SummaryConfig::default()
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

    /// Pin `dir`'s own mtime, and confirm the filesystem kept the value.
    ///
    /// Both halves of the cache test below assert what [`workspace_summary`]
    /// does with [`dir_mtime`], so the timestamp is each half's precondition
    /// rather than a detail of it. A filesystem that quietly refuses the value
    /// must fail here, naming the timestamp, instead of downstream where a
    /// stale map reads as a cache bug — which is how this test used to fail.
    fn pin_dir_mtime(dir: &Path, at: FileTime) {
        filetime::set_file_mtime(dir, at).expect("set the directory's mtime");
        let read_back = FileTime::from_last_modification_time(
            &fs::metadata(dir).expect("read the directory's metadata"),
        );
        assert_eq!(
            read_back, at,
            "filesystem kept the directory mtime this test drives invalidation through"
        );
    }

    #[test]
    fn test_cache_serves_and_invalidates_on_mtime() {
        // Both halves are steered by two config keys, each resolved from the
        // environment or `config.yaml`. Assert them up front: a machine that
        // has turned the feature off — `CONTEXT_WORKSPACE_SUMMARY: false` is
        // the documented lever for a slow workspace walk — would otherwise
        // fail below with a message about the file map rather than about its
        // own configuration.
        assert!(
            enabled(),
            "CONTEXT_WORKSPACE_SUMMARY is off here, so the cache under test never runs"
        );
        assert!(
            config_u64("CONTEXT_WORKSPACE_SUMMARY_TTL_SECS", DEFAULT_TTL_SECS) > 0,
            "CONTEXT_WORKSPACE_SUMMARY_TTL_SECS is 0 here, which disables caching entirely"
        );

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("one.txt"), "x").unwrap();

        // Fixed timestamps rather than whatever the writes happen to leave
        // behind: mtime resolution is coarse (the Windows clock ticks roughly
        // every 15ms, and NTFS updates a directory's own mtime lazily), so two
        // writes microseconds apart can leave it byte-identical. Taking the
        // cache's invalidation signal from wall-clock writes is what made this
        // test flaky on windows-latest while macOS and Ubuntu passed.
        let before = FileTime::from_unix_time(1_000_000_000, 0);
        let after = FileTime::from_unix_time(1_000_000_060, 0);
        pin_dir_mtime(dir.path(), before);

        let first = workspace_summary(dir.path()).expect("first summary");
        assert!(first.contains("one.txt"));
        assert!(
            !first.contains("two.txt"),
            "two.txt does not exist yet, so the map cannot name it"
        );

        // Half 1 — the cache SERVES. Add a top-level file, then put the mtime
        // back where the cached entry recorded it: inside the TTL the stale map
        // must come back unchanged, which is the whole point of caching.
        fs::write(dir.path().join("two.txt"), "y").unwrap();
        pin_dir_mtime(dir.path(), before);
        assert_eq!(
            workspace_summary(dir.path()).expect("cached summary"),
            first,
            "an unchanged directory mtime inside the TTL serves the cached map"
        );

        // Half 2 — the cache INVALIDATES. Move the mtime clearly forward and
        // the new top-level file must surface, TTL notwithstanding.
        pin_dir_mtime(dir.path(), after);
        let refreshed = workspace_summary(dir.path()).expect("second summary");
        assert!(
            refreshed.contains("two.txt"),
            "a changed directory mtime rewalks and surfaces the new file: {refreshed}"
        );

        forget_cached(dir.path());
    }
}
