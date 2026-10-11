//! Persisted, per-path undo history for the developer extension (BR-44).
//!
//! Historically the `text_editor` undo stack was an in-process
//! `Arc<Mutex<HashMap<PathBuf, Vec<String>>>>` created fresh in
//! `DeveloperServer::new()`: it died with the developer-server process, was
//! never persisted, and only covered files touched via `text_editor`
//! (`shell` redirects and plain `write`s were invisible to it).
//!
//! `FileHistory` keeps the same in-memory LIFO-of-whole-file-snapshots
//! semantics but adds two things the review asked for:
//!   1. When a session working directory is known, it mirrors the stack to a
//!      JSON file under the config dir so `undo_edit` survives a restart.
//!   2. It is also fed by `shell` redirect targets and the `write` command, so
//!      `undo_edit` covers `>`/`>>` writes and whole-file overwrites, not just
//!      `str_replace`/`insert`/diff edits.
//!
//! This is the incremental, `text_editor`-native counterpart to the richer
//! shadow-git checkpointing in BR-43.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Cap on retained snapshots per file. Bounds RAM and the persisted JSON so a
/// hot-edited file cannot grow the history without bound; when exceeded the
/// oldest snapshot is dropped.
const MAX_SNAPSHOTS_PER_FILE: usize = 50;

/// On-disk shape of the history. Keyed by the path's lossy string form, which
/// round-trips for UTF-8 paths (every practical case).
#[derive(Default, Serialize, Deserialize)]
struct PersistedHistory {
    files: HashMap<String, Vec<String>>,
}

/// Per-path LIFO of whole-file snapshots backing `undo_edit`.
#[derive(Default)]
pub struct FileHistory {
    inner: Mutex<HashMap<PathBuf, Vec<String>>>,
    /// When set, every mutation is mirrored to this JSON file (best-effort).
    persist_path: Option<PathBuf>,
}

impl FileHistory {
    /// In-memory only — the pre-BR-44 behavior. Used when no session working
    /// directory is known (e.g. the bare `biorouter mcp developer` transport).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Persist history to `<config>/developer_undo/<hash>.json`, keyed by the
    /// session working directory, loading any prior history for that directory.
    /// Falls back to in-memory when persistence is disabled or no config
    /// directory can be resolved.
    pub fn persistent(working_dir: &Path) -> Self {
        match persist_path_for(working_dir) {
            Some(path) => {
                let inner = load(&path).unwrap_or_default();
                Self {
                    inner: Mutex::new(inner),
                    persist_path: Some(path),
                }
            }
            None => Self::in_memory(),
        }
    }

    /// Snapshot the current on-disk content of `path` (empty string if it does
    /// not yet exist) onto its undo stack.
    pub fn snapshot(&self, path: &Path) -> std::io::Result<()> {
        let content = if path.exists() {
            std::fs::read_to_string(path)?
        } else {
            String::new()
        };
        self.push(path, content);
        Ok(())
    }

    fn push(&self, path: &Path, content: String) {
        let mut guard = self.inner.lock().unwrap();
        let stack = guard.entry(path.to_path_buf()).or_default();
        stack.push(content);
        let overflow = stack.len().saturating_sub(MAX_SNAPSHOTS_PER_FILE);
        if overflow > 0 {
            stack.drain(0..overflow);
        }
        self.persist(&guard);
    }

    /// Pop the most recent snapshot for `path`, if any.
    pub fn pop(&self, path: &Path) -> Option<String> {
        let mut guard = self.inner.lock().unwrap();
        let popped = guard.get_mut(path).and_then(Vec::pop);
        if popped.is_some() {
            self.persist(&guard);
        }
        popped
    }

    fn persist(&self, map: &HashMap<PathBuf, Vec<String>>) {
        let Some(path) = &self.persist_path else {
            return;
        };
        // Best-effort: a failed persist must never break an edit or an undo.
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let persisted = PersistedHistory {
            files: map
                .iter()
                .map(|(k, v)| (k.to_string_lossy().into_owned(), v.clone()))
                .collect(),
        };
        if let Ok(json) = serde_json::to_string(&persisted) {
            let _ = std::fs::write(path, json);
        }
    }

    #[cfg(test)]
    pub fn depth(&self, path: &Path) -> usize {
        self.inner.lock().unwrap().get(path).map_or(0, Vec::len)
    }
}

fn load(path: &Path) -> Option<HashMap<PathBuf, Vec<String>>> {
    let json = std::fs::read_to_string(path).ok()?;
    let persisted: PersistedHistory = serde_json::from_str(&json).ok()?;
    Some(
        persisted
            .files
            .into_iter()
            .map(|(k, v)| (PathBuf::from(k), v))
            .collect(),
    )
}

/// `<config>/developer_undo/<hash-of-working-dir>.json`, honoring the
/// `BIOROUTER_PATH_ROOT` test override the rest of the app uses. Returns `None`
/// (in-memory only) when persistence is opted out or no config dir resolves.
fn persist_path_for(working_dir: &Path) -> Option<PathBuf> {
    if let Ok(v) = std::env::var("BIOROUTER_PERSIST_UNDO") {
        let v = v.trim().to_ascii_lowercase();
        if matches!(v.as_str(), "0" | "false" | "no" | "off") {
            return None;
        }
    }
    let base = undo_base_dir()?;
    Some(base.join(format!("{}.json", dir_key(working_dir))))
}

fn undo_base_dir() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("BIOROUTER_PATH_ROOT") {
        return Some(PathBuf::from(root).join("config").join("developer_undo"));
    }
    crate::paths::platform_dirs()
        .ok()
        .map(|dirs| dirs.config.join("developer_undo"))
}

/// A stable hash of the (canonicalized when possible) working directory, so
/// different spellings of the same directory share one history file.
fn dir_key(working_dir: &Path) -> String {
    let canonical =
        std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Extract file targets a shell command writes to via redirection, so their
/// pre-command contents can be snapshotted for `undo_edit`.
///
/// Conservative by design — only the unambiguous file-redirect forms:
/// `> f`, `>> f`, `1> f`, `2> f`, `&> f`, `>| f`, including the fused
/// `>f`/`>>f` spellings. It deliberately skips fd-dups (`2>&1`, `>&2`), process
/// substitution (`>(cmd)`), quoted `>` inside string literals, and device
/// sinks (`/dev/null`, `/dev/stdout`, ...). Quoting of the *target* itself is
/// handled (surrounding quotes are stripped); here-docs are out of scope. A
/// missed target simply means no undo entry, never a broken command.
pub fn redirect_targets(command: &str) -> Vec<String> {
    let bytes = command.as_bytes();
    let mut targets = Vec::new();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\'' if !in_double => {
                in_single = !in_single;
                i += 1;
            }
            b'"' if !in_single => {
                in_double = !in_double;
                i += 1;
            }
            b'>' if !in_single && !in_double => {
                // Consume the run of '>' (handles '>>').
                let mut j = i;
                while j < bytes.len() && bytes[j] == b'>' {
                    j += 1;
                }
                // Optional clobber-override '>|'.
                if j < bytes.len() && bytes[j] == b'|' {
                    j += 1;
                }
                // Skip inter-token whitespace.
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                // '&' => fd-dup (`2>&1`), '(' => process substitution: not files.
                if j < bytes.len() && bytes[j] != b'&' && bytes[j] != b'(' {
                    let quote = matches!(bytes[j], b'"' | b'\'').then_some(bytes[j]);
                    let raw = if let Some(q) = quote {
                        // Quoted target: read to the matching close quote so a
                        // path with spaces survives.
                        let start = j + 1;
                        j = start;
                        while j < bytes.len() && bytes[j] != q {
                            j += 1;
                        }
                        let target = String::from_utf8_lossy(&bytes[start..j]).into_owned();
                        if j < bytes.len() {
                            j += 1; // consume the closing quote
                        }
                        target
                    } else {
                        let start = j;
                        while j < bytes.len() && !is_token_terminator(bytes[j]) {
                            j += 1;
                        }
                        strip_quotes(String::from_utf8_lossy(&bytes[start..j]).as_ref())
                    };
                    if !raw.is_empty() && !is_device(&raw) {
                        targets.push(raw);
                    }
                }
                i = j.max(i + 1);
            }
            _ => i += 1,
        }
    }
    targets
}

fn is_token_terminator(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t' | b'\n' | b'|' | b'&' | b';' | b'<' | b'>' | b')' | b'('
    )
}

fn strip_quotes(s: &str) -> String {
    let trimmed = s.trim();
    for q in ['"', '\''] {
        if let Some(inner) = trimmed.strip_prefix(q).and_then(|t| t.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    trimmed.trim_matches(['"', '\'']).to_string()
}

fn is_device(target: &str) -> bool {
    target == "/dev/null"
        || target == "/dev/stdout"
        || target == "/dev/stderr"
        || target.starts_with("/dev/fd/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    /// Pin the process-global env the persistence tests drive
    /// (`BIOROUTER_PATH_ROOT`, `BIOROUTER_PERSIST_UNDO`) for as long as the
    /// returned guard lives.
    ///
    /// A module-local mutex plus `set_var`/`remove_var` is not enough on two
    /// counts: the values leak out of any test that panics between the set and
    /// the remove, and a module-local mutex serialises only *these* tests while
    /// `BIOROUTER_PATH_ROOT` is read on every config-dir lookup anywhere in the
    /// process. `env_lock` restores the previous values from `Drop` and takes
    /// the same process-wide lock the rest of the workspace's
    /// `BIOROUTER_PATH_ROOT` tests take.
    ///
    /// Values are owned `String`s so the guard does not borrow the caller's
    /// `TempDir`.
    fn scoped_undo_env(root: &Path, persist: Option<&str>) -> env_lock::EnvGuard<'static> {
        env_lock::lock_env([
            (
                "BIOROUTER_PATH_ROOT",
                Some(root.to_string_lossy().into_owned()),
            ),
            ("BIOROUTER_PERSIST_UNDO", persist.map(str::to_owned)),
        ])
    }

    #[test]
    fn in_memory_push_and_pop_lifo() {
        let h = FileHistory::in_memory();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "one").unwrap();
        h.snapshot(&f).unwrap();
        std::fs::write(&f, "two").unwrap();
        h.snapshot(&f).unwrap();
        assert_eq!(h.depth(&f), 2);
        assert_eq!(h.pop(&f).as_deref(), Some("two"));
        assert_eq!(h.pop(&f).as_deref(), Some("one"));
        assert_eq!(h.pop(&f), None);
    }

    #[test]
    fn snapshot_of_missing_file_is_empty_string() {
        let h = FileHistory::in_memory();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("nope.txt");
        h.snapshot(&f).unwrap();
        assert_eq!(h.pop(&f).as_deref(), Some(""));
    }

    #[test]
    fn depth_is_capped() {
        let h = FileHistory::in_memory();
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("hot.txt");
        for n in 0..(MAX_SNAPSHOTS_PER_FILE + 10) {
            std::fs::write(&f, format!("v{n}")).unwrap();
            h.snapshot(&f).unwrap();
        }
        assert_eq!(h.depth(&f), MAX_SNAPSHOTS_PER_FILE);
        // The oldest entries were dropped; the newest is still on top.
        assert_eq!(
            h.pop(&f).as_deref(),
            Some(format!("v{}", MAX_SNAPSHOTS_PER_FILE + 9).as_str())
        );
    }

    #[test]
    #[serial]
    fn persists_across_instances() {
        let root = TempDir::new().unwrap();
        let _env = scoped_undo_env(root.path(), None);
        let work = TempDir::new().unwrap();
        let f = work.path().join("keep.txt");
        std::fs::write(&f, "original").unwrap();

        {
            let h = FileHistory::persistent(work.path());
            h.snapshot(&f).unwrap();
        } // dropped — only the on-disk copy survives

        // A fresh instance for the same working dir reloads the stack.
        let h2 = FileHistory::persistent(work.path());
        assert_eq!(h2.pop(&f).as_deref(), Some("original"));
    }

    #[test]
    #[serial]
    fn persistence_opt_out_stays_in_memory() {
        let root = TempDir::new().unwrap();
        let _env = scoped_undo_env(root.path(), Some("0"));
        let work = TempDir::new().unwrap();
        let h = FileHistory::persistent(work.path());
        assert!(h.persist_path.is_none());
    }

    /// The persistence tests above pin process-global env vars. If one of them
    /// panics — an assertion failure, a `.unwrap()` on a missing file — the
    /// scratch root must not survive the unwind: every later `Paths::*` call in
    /// this process would then resolve into a `TempDir` that has been dropped.
    ///
    /// The panic below is caught, so the stderr backtrace it prints is expected.
    ///
    /// `#[serial]` is load-bearing *here* for a reason the guard cannot cover:
    /// the "before" sample is necessarily read before the lock is acquired, so
    /// without it this test could sample the ambient value while a sibling
    /// persistence test had the variable pinned to its own `TempDir` and then
    /// compare against that dead value. (Observed once in ~14 runs of the full
    /// `developer::` suite before this annotation.) The other two mutators
    /// carry it too, so the three cannot interleave.
    #[test]
    #[serial]
    fn env_is_restored_when_a_persistence_test_panics() {
        let before_root = std::env::var("BIOROUTER_PATH_ROOT").ok();
        let before_optout = std::env::var("BIOROUTER_PERSIST_UNDO").ok();
        let root = TempDir::new().unwrap();

        let outcome = std::panic::catch_unwind(|| {
            let _env = scoped_undo_env(root.path(), Some("0"));
            assert_eq!(
                std::env::var("BIOROUTER_PATH_ROOT").ok().as_deref(),
                root.path().to_str(),
                "the guard must actually pin the variable"
            );
            panic!("simulated assertion failure inside the guarded region");
        });

        assert!(outcome.is_err(), "the panic must propagate");
        assert_eq!(
            std::env::var("BIOROUTER_PATH_ROOT").ok(),
            before_root,
            "BIOROUTER_PATH_ROOT leaked out of a panicking persistence test"
        );
        assert_eq!(
            std::env::var("BIOROUTER_PERSIST_UNDO").ok(),
            before_optout,
            "BIOROUTER_PERSIST_UNDO leaked out of a panicking persistence test"
        );
    }

    #[test]
    fn redirect_targets_basic_forms() {
        assert_eq!(redirect_targets("echo hi > out.txt"), vec!["out.txt"]);
        assert_eq!(redirect_targets("echo hi >> out.log"), vec!["out.log"]);
        assert_eq!(redirect_targets("echo hi>fused.txt"), vec!["fused.txt"]);
        assert_eq!(redirect_targets("cmd 2> err.log"), vec!["err.log"]);
        assert_eq!(redirect_targets("cmd &> all.log"), vec!["all.log"]);
        assert_eq!(redirect_targets("cmd >| clobber.txt"), vec!["clobber.txt"]);
    }

    #[test]
    fn redirect_targets_multiple() {
        assert_eq!(
            redirect_targets("cmd 2>&1 > out.txt; other >> two.txt"),
            vec!["out.txt", "two.txt"]
        );
    }

    #[test]
    fn redirect_targets_skips_non_files() {
        assert!(redirect_targets("cmd 2>&1").is_empty());
        assert!(redirect_targets("cmd >&2").is_empty());
        assert!(redirect_targets("cmd > /dev/null").is_empty());
        assert!(redirect_targets("diff <(a) >(b)").is_empty());
        assert!(redirect_targets("echo no redirect here").is_empty());
    }

    #[test]
    fn redirect_targets_ignores_quoted_gt() {
        assert!(redirect_targets("echo \"a > b\"").is_empty());
        assert!(redirect_targets("echo 'x > y'").is_empty());
        // A real redirect after a quoted one is still found.
        assert_eq!(
            redirect_targets("echo \"a > b\" > real.txt"),
            vec!["real.txt"]
        );
    }

    #[test]
    fn redirect_targets_strips_quotes_on_target() {
        assert_eq!(
            redirect_targets("echo hi > \"sp ace.txt\""),
            vec!["sp ace.txt"]
        );
    }
}
