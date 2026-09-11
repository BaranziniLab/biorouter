//! Central secret-redaction boundary (BR-23).
//!
//! Historically the `.biorouterignore` deny list lived only inside the Developer
//! MCP server (`developer::rmcp_developer`), so any *other* extension — compute,
//! files, a third-party MCP server, or a different shell wrapper — that read a
//! `.env`/`secrets.*`/private-key file bypassed it entirely. `SecretGuard`
//! extracts that logic into one shared type so it can be enforced at the
//! extension-manager dispatch boundary — the single choke point every tool call
//! flows through — as well as inside the Developer server itself.
//!
//! Two behavioural improvements over the old Developer-local matcher:
//!   * The built-in secret patterns are an always-on *floor* (they apply even
//!     when a project ships its own `.biorouterignore`, which previously silently
//!     dropped them). A user can still opt back into a specific file with a
//!     gitignore negation (`!path`) because the floor is added *before* the
//!     user's patterns and gitignore matching is last-match-wins.
//!   * The default deny set is widened beyond `.env`/`secrets.*` to cover private
//!     keys (`*.pem`, `id_rsa`, `id_ed25519`, …) and cloud-credential files
//!     (`.aws/credentials`, `*.p12`, `*.pfx`).

use etcetera::AppStrategy;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// Built-in secret/credential deny patterns applied as an always-on floor.
///
/// Kept deliberately tight — every entry names a file that is a secret by
/// convention — so the exists-gated dispatch scan does not block legitimate
/// reads of ordinary config files.
pub const DEFAULT_SECRET_PATTERNS: &[&str] = &[
    "**/.env",
    "**/.env.*",
    "**/secrets.*",
    "**/*.pem",
    "**/id_rsa",
    "**/id_dsa",
    "**/id_ecdsa",
    "**/id_ed25519",
    "**/*.p12",
    "**/*.pfx",
    "**/.aws/credentials",
    "**/.codex/auth.json",
    "**/.claude/.credentials.json",
];

/// Object keys whose string values are treated as file paths and scanned in
/// full (every whitespace token). Tokens under any other key are only scanned
/// when they contain a path separator, so a `.env` mentioned in prose (e.g. a
/// `content`/`message` field) does not trip the boundary.
fn key_is_pathlike(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("path")
        || k.contains("file")
        || k.contains("dir")
        || matches!(
            k.as_str(),
            "command"
                | "cmd"
                | "script"
                | "target"
                | "output"
                | "out"
                | "input"
                | "src"
                | "source"
                | "dest"
                | "destination"
                | "location"
                | "uri"
                | "url"
                | "folder"
        )
}

/// A reusable secret/credential access guard rooted at a working directory.
#[derive(Clone)]
pub struct SecretGuard {
    /// Everything: the built-in floor, the global ignore file, and the
    /// project's own `.biorouterignore`. Authoritative for paths inside `root`.
    ignore: Gitignore,
    /// The floor and the global ignore file only — the statements that are
    /// about *this machine*, not about one project. Authoritative for paths
    /// outside `root`.
    machine_wide: Gitignore,
    root: PathBuf,
    /// `root` with symlinks resolved, so containment can be decided for a path
    /// that names the same directory by a different route (on macOS every
    /// `/var/…` temp dir is really `/private/var/…`). Falls back to `root`.
    canonical_root: PathBuf,
}

impl SecretGuard {
    /// Build a guard rooted at `cwd`, combining the always-on secret floor with
    /// any project-local (`<cwd>/.biorouterignore`) and global
    /// (`<config>/.biorouterignore`) ignore files. User patterns are layered
    /// *after* the floor so a `!path` negation can un-ignore a specific file the
    /// user explicitly wants read.
    pub fn for_dir(cwd: &Path) -> Self {
        Self::build(cwd, &ignore_sources(cwd))
    }

    /// Build a guard from an explicit, already-resolved list of ignore files.
    ///
    /// `sources` must be exactly what [`ignore_sources`] returns for `cwd` —
    /// the cache fingerprints that same list, so any divergence between the
    /// files read here and the files fingerprinted there would be a staleness
    /// hole. Keep the two in lockstep by never calling this with a hand-built
    /// list outside tests.
    fn build(cwd: &Path, sources: &[PathBuf]) -> Self {
        let mut builder = GitignoreBuilder::new(cwd);
        let mut machine_builder = GitignoreBuilder::new(cwd);

        for pat in DEFAULT_SECRET_PATTERNS {
            let _ = builder.add_line(None, pat);
            let _ = machine_builder.add_line(None, pat);
        }

        let project_local = cwd.join(".biorouterignore");
        for source in sources {
            let _ = builder.add(source);
            // The project's own ignore file is the one statement that is scoped
            // to the project; everything else here (the floor, the global
            // `<config>/.biorouterignore`) is machine-wide.
            if source != &project_local {
                let _ = machine_builder.add(source);
            }
        }

        // Degrade to an empty matcher on a malformed ignore file rather than
        // panicking on the dispatch path.
        let ignore = builder.build().unwrap_or_else(|_| Gitignore::empty());
        let machine_wide = machine_builder
            .build()
            .unwrap_or_else(|_| Gitignore::empty());
        Self {
            ignore,
            machine_wide,
            root: cwd.to_path_buf(),
            canonical_root: std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf()),
        }
    }

    /// Cached counterpart of [`SecretGuard::for_dir`], for the tool-dispatch
    /// hot path.
    ///
    /// Building a guard costs a `GitignoreBuilder`, two `stat`s, up to two file
    /// reads and a globset compile — all synchronous `std::fs` on a tokio
    /// worker, on every tool call. This memoises the guard per resolved working
    /// directory.
    ///
    /// **Correctness over savings.** The cache is a security boundary: if a
    /// user adds a path to `.biorouterignore` and we serve a stale guard, the
    /// agent reads a file the user explicitly protected. So invalidation
    /// compares the *exact bytes* of every ignore file, not an mtime — mtime
    /// has filesystem-dependent granularity and can miss a same-second edit of
    /// equal length. The two ignore files are small; re-reading them still
    /// skips the builder and the globset compile, which dominate.
    ///
    /// The fingerprint is taken **before** the guard is built, never after. A
    /// write racing the build then leaves a guard stamped with the *older*
    /// fingerprint, so the next call sees a mismatch and rebuilds. Stamping
    /// afterwards would cache new-content-under-new-fingerprint only by luck
    /// and could pin stale contents.
    pub fn cached_for_dir(cwd: &Path) -> Arc<Self> {
        let root = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        let sources = ignore_sources(&root);
        let fingerprint = fingerprint_sources(&sources);

        if let Some(fp) = fingerprint.as_ref() {
            if let Ok(cache) = GUARD_CACHE.lock() {
                if let Some(entry) = cache.get(&root) {
                    if &entry.fingerprint == fp {
                        return entry.guard.clone();
                    }
                }
            }
        }

        let guard = Arc::new(Self::build(&root, &sources));

        if let Some(fp) = fingerprint {
            if let Ok(mut cache) = GUARD_CACHE.lock() {
                // Unbounded growth guard: a long-lived daemon can see many
                // working directories. Clearing wholesale is fine — the cache
                // is a pure memo.
                if cache.len() >= MAX_CACHE_ENTRIES && !cache.contains_key(&root) {
                    cache.clear();
                }
                cache.insert(
                    root,
                    CacheEntry {
                        guard: guard.clone(),
                        fingerprint: fp,
                    },
                );
            }
        }

        guard
    }

    /// The underlying gitignore matcher, for callers (e.g. the code analyzer)
    /// that traverse trees and need the raw matcher.
    pub fn gitignore(&self) -> &Gitignore {
        &self.ignore
    }

    /// True when `path` matches a deny pattern.
    ///
    /// **A project's patterns apply only inside that project.** The built-in
    /// floor and the global `<config>/.biorouterignore` are statements about
    /// this machine and apply to every path; `<root>/.biorouterignore` is a
    /// statement about one directory tree. Applying the latter everywhere made
    /// an unrelated file that merely shares a basename with a project rule
    /// unreadable — gitignore patterns without a slash match at any depth, and
    /// the guard is consulted for absolute paths outside the root (Auto mode
    /// resolves those past the containment jail on purpose).
    ///
    /// Normally IO-free, so it stays cheap to call per token: the containment
    /// question is only asked when the project's patterns actually change the
    /// verdict, and only reaches the filesystem when the path is not already a
    /// textual descendant of the root.
    pub fn is_denied(&self, path: &Path) -> bool {
        let everything = self.ignore.matched(path, false).is_ignore();
        let machine_wide = self.machine_wide.matched(path, false).is_ignore();
        if everything == machine_wide {
            // The project's own file did not move the needle either way.
            return everything;
        }
        if self.is_inside_root(path) {
            // Inside the project the project has the last word — it may add a
            // rule, and it may negate one of the floor's with `!path`.
            everything
        } else {
            machine_wide
        }
    }

    /// Whether `path` names something inside this guard's root. A relative path
    /// is by construction relative to the root (that is how the matcher and
    /// [`Self::candidate_is_denied`] treat it).
    fn is_inside_root(&self, path: &Path) -> bool {
        if path.is_relative() {
            return true;
        }
        if path.starts_with(&self.root) || path.starts_with(&self.canonical_root) {
            return true;
        }
        // Only now pay for IO: a path can still be inside the project by a
        // different route (a symlinked ancestor). Canonicalize the deepest
        // ancestor that exists — the tail below it cannot contain links.
        let mut ancestor = path;
        loop {
            if let Ok(real) = ancestor.canonicalize() {
                return real.starts_with(&self.canonical_root);
            }
            match ancestor.parent() {
                Some(parent) => ancestor = parent,
                None => return false,
            }
        }
    }

    /// Scan a tool call's arguments for a reference to a denied (secret) file
    /// that *exists* on disk, resolved against the guard's root. Returns the
    /// offending display path, or `None`.
    ///
    /// Conservative by design:
    ///   * a candidate is only reported when the resolved path exists, so a
    ///     benign mention of `.env` that does not name a real file never trips
    ///     the boundary, and creating a *new* secret file (not yet on disk) is
    ///     not blocked;
    ///   * a bare token (no path separator) is only considered under a path-like
    ///     key, so `.env` appearing inside a `content`/`message` field is
    ///     ignored even when such a file happens to exist.
    /// [`Self::find_denied_path`] against an explicit environment.
    ///
    /// FAIL-BEFORE SHIM (H1): the existing matcher never expands anything, so
    /// the environment is ignored here and the H1 table runs against today's
    /// behaviour unchanged.
    pub fn find_denied_path_in(
        &self,
        arguments: &Map<String, Value>,
        _env: &ShellEnv,
    ) -> Option<String> {
        self.find_denied_path(arguments)
    }

    pub fn find_denied_path(&self, arguments: &Map<String, Value>) -> Option<String> {
        let mut found = None;
        for (key, value) in arguments {
            self.walk_value(Some(key), value, &mut found);
            if found.is_some() {
                break;
            }
        }
        found
    }

    fn walk_value(&self, key: Option<&str>, value: &Value, found: &mut Option<String>) {
        if found.is_some() {
            return;
        }
        match value {
            Value::String(s) => self.scan_string(key, s, found),
            Value::Array(items) => {
                for item in items {
                    self.walk_value(key, item, found);
                    if found.is_some() {
                        return;
                    }
                }
            }
            Value::Object(map) => {
                for (k, v) in map {
                    self.walk_value(Some(k), v, found);
                    if found.is_some() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_string(&self, key: Option<&str>, s: &str, found: &mut Option<String>) {
        let pathlike_key = key.map(key_is_pathlike).unwrap_or(false);
        let trimmed = s.trim();

        // Whole-string candidate (covers a plain `{"path": ".env"}` argument).
        if (pathlike_key || has_separator(trimmed)) && self.candidate_is_denied(trimmed) {
            *found = Some(trimmed.to_string());
            return;
        }

        // Token candidates (covers shell command lines and multi-path values).
        for token in trimmed.split_whitespace() {
            let tok = token.trim_matches(|c| c == '"' || c == '\'');
            if tok.is_empty() || tok.starts_with('-') {
                continue;
            }
            if (pathlike_key || has_separator(tok)) && self.candidate_is_denied(tok) {
                *found = Some(tok.to_string());
                return;
            }
        }
    }

    /// Pattern-match first (IO-free), then confirm the file exists. Ordering
    /// keeps the common case (large non-path arguments) from issuing a `stat`
    /// per token.
    fn candidate_is_denied(&self, candidate: &str) -> bool {
        if candidate.is_empty() {
            return false;
        }
        let path = Path::new(candidate);
        // `join` replaces the base when `path` is absolute, so this handles both
        // relative and absolute candidates.
        let resolved = self.root.join(path);
        if !(self.is_denied(&resolved) || self.is_denied(path)) {
            return false;
        }
        resolved.exists() || path.exists()
    }
}

fn has_separator(s: &str) -> bool {
    s.contains('/') || s.contains('\\')
}

/// The environment a scan expands `~` and `$VAR` against.
///
/// FAIL-BEFORE SHIM (H1): carried but unused by the matcher above.
#[derive(Debug, Clone, Default)]
pub struct ShellEnv {
    #[allow(dead_code)]
    vars: HashMap<String, String>,
}

impl ShellEnv {
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    pub fn with_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: vars.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
        }
    }
}

/// Ignore files that contribute to a guard rooted at `cwd`, in the order they
/// are layered: global (`<config>/.biorouterignore`) first, then project-local
/// (`<cwd>/.biorouterignore`). Only files that exist are returned, so creating
/// or deleting one changes the list itself and therefore the fingerprint.
fn ignore_sources(cwd: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::with_capacity(2);

    if let Ok(strategy) = etcetera::choose_app_strategy(crate::APP_STRATEGY.clone()) {
        let global = strategy.config_dir().join(".biorouterignore");
        if global.is_file() {
            sources.push(global);
        }
    }

    let local = cwd.join(".biorouterignore");
    if local.is_file() {
        sources.push(local);
    }

    sources
}

/// Largest ignore file we are willing to hold in the fingerprint. Beyond this
/// the entry is simply not cached (fail-open on performance, never on
/// correctness).
const MAX_FINGERPRINT_BYTES: u64 = 256 * 1024;

/// Cap on distinct working directories memoised at once.
const MAX_CACHE_ENTRIES: usize = 64;

/// Exact contents of every ignore file backing a guard, paired with its path.
/// Equality here is the cache's entire validity condition.
type Fingerprint = Vec<(PathBuf, Vec<u8>)>;

struct CacheEntry {
    guard: Arc<SecretGuard>,
    fingerprint: Fingerprint,
}

static GUARD_CACHE: LazyLock<Mutex<HashMap<PathBuf, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Read every source's bytes. `None` means "do not cache this directory" — an
/// oversized or unreadable ignore file, where we cannot prove freshness later.
fn fingerprint_sources(sources: &[PathBuf]) -> Option<Fingerprint> {
    let mut fingerprint = Fingerprint::with_capacity(sources.len());
    for source in sources {
        let meta = std::fs::metadata(source).ok()?;
        if meta.len() > MAX_FINGERPRINT_BYTES {
            return None;
        }
        let bytes = std::fs::read(source).ok()?;
        fingerprint.push((source.clone(), bytes));
    }
    Some(fingerprint)
}

/// Fixtures for H1's tables, shared by this module's tests and the Developer
/// server's, which run the same rows through its own check.
#[cfg(test)]
pub(crate) mod h1_fixtures {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ---- H1 (QA-C, 2026-09-10): every spelling of a secret ----------------
    //
    // Measured from a public-model chat in Auto mode: `/Users/…/.aws/credentials`
    // was refused, while `~/…`, `$HOME/…`, a glob and `cd … && head` all handed
    // the model the AWS key. Every row here runs against a throwaway HOME with
    // fake credentials — never the operator's real `~/.aws`, `~/.ssh` or
    // `~/.config/biorouter`, and never the real global `.biorouterignore`
    // (`SecretGuard::build` with no sources).

    /// Made-up credential material, assembled at run time so no key-shaped
    /// literal sits in the source for a secret scanner to trip on.
    pub(crate) fn fake_aws_credentials() -> String {
        let id = format!("{}{}", "AKIA", "FAKEFAKEFAKE0000");
        let secret = format!("{}{}", "fakeSecretKeyForTestsOnly", "0".repeat(15));
        format!("[default]\naws_access_key_id = {id}\naws_secret_access_key = {secret}\n")
    }

    pub(crate) fn fake_private_key() -> String {
        let kind = ["OPENSSH", "PRIVATE", "KEY"].join(" ");
        format!(
            "-----BEGIN {kind}-----\nZmFrZSBrZXkgbWF0ZXJpYWwgZm9yIHRlc3RzIG9ubHk=\n-----END {kind}-----\n"
        )
    }

    /// A throwaway HOME holding fake credentials, and a project beside it.
    pub(crate) struct FakeHome {
        _dir: tempfile::TempDir,
        pub(crate) home: PathBuf,
        pub(crate) project: PathBuf,
    }

    impl FakeHome {
        pub(crate) fn new() -> Self {
            let dir = tempdir().unwrap();
            // Canonical, so a macOS `/var` → `/private/var` link cannot make
            // the same file look like two.
            let root = fs::canonicalize(dir.path()).unwrap();
            let home = root.join("home");
            let project = root.join("project");
            for sub in [".aws", ".ssh", ".config/biorouter"] {
                fs::create_dir_all(home.join(sub)).unwrap();
            }
            fs::create_dir_all(project.join("src")).unwrap();
            fs::write(home.join(".aws/credentials"), fake_aws_credentials()).unwrap();
            fs::write(home.join(".aws/config"), "[default]\nregion = us-west-2\n").unwrap();
            fs::write(home.join(".ssh/id_ed25519"), fake_private_key()).unwrap();
            fs::write(home.join(".ssh/deploy.pem"), fake_private_key()).unwrap();
            fs::write(
                home.join(".ssh/id_ed25519.pub"),
                "ssh-ed25519 AAAAC3fake tester@example\n",
            )
            .unwrap();
            fs::write(home.join(".ssh/config"), "Host example\n  User tester\n").unwrap();
            fs::write(
                home.join(".config/biorouter/secrets.yaml"),
                "OPENAI_API_KEY: not-a-real-key-for-tests\n",
            )
            .unwrap();
            fs::write(
                home.join(".config/biorouter/config.yaml"),
                "BIOROUTER_PROVIDER: versa_azure\n",
            )
            .unwrap();
            fs::write(home.join("notes.txt"), "hello\n").unwrap();
            fs::write(project.join("data.csv"), "a,b\n1,2\n").unwrap();
            fs::write(project.join("src/main.py"), "print('hi')\n").unwrap();
            Self {
                _dir: dir,
                home,
                project,
            }
        }

        /// Links from the project into the fake HOME's credential stores.
        #[cfg(unix)]
        pub(crate) fn with_links(self) -> Self {
            std::os::unix::fs::symlink(self.home.join(".aws"), self.project.join("aws-link"))
                .unwrap();
            std::os::unix::fs::symlink(
                self.home.join(".aws/credentials"),
                self.project.join("creds-link"),
            )
            .unwrap();
            self
        }

        pub(crate) fn env(&self) -> ShellEnv {
            ShellEnv::with_vars([
                ("HOME", self.home.to_string_lossy().into_owned()),
                ("USER", "tester".to_string()),
            ])
        }

        pub(crate) fn guard(&self) -> SecretGuard {
            SecretGuard::build(&self.project, &[])
        }
    }

    /// The command spellings H1 measured leaking, and the families beside them.
    /// Shared with the Developer server's own table, which runs the same rows
    /// through its path.
    pub(crate) fn h1_leaking_spellings(home: &Path) -> Vec<(&'static str, String)> {
        let h = home.display();
        let mut rows: Vec<(&'static str, String)> = vec![
            ("absolute", format!("cat {h}/.aws/credentials")),
            ("tilde", "cat ~/.aws/credentials".into()),
            ("$HOME", "cat $HOME/.aws/credentials".into()),
            ("${HOME}", "cat ${HOME}/.aws/credentials".into()),
            ("quoted $HOME", "cat \"$HOME/.aws/credentials\"".into()),
            ("variable from $HOME", "H=$HOME; cat $H/.aws/credentials".into()),
            ("absolute glob", format!("cat {h}/.aws/cred*")),
            ("tilde glob", "cat ~/.aws/cred*".into()),
            ("cd && head", format!("cd {h}/.aws && head credentials")),
            ("cd ; cat", "cd ~/.aws; cat credentials".into()),
            ("pushd", "pushd ~/.aws >/dev/null && cat credentials".into()),
            ("cd with no argument", "cd; cat .aws/credentials".into()),
            ("ssh private key", "cat ~/.ssh/id_ed25519".into()),
            ("pem glob", "cat ~/.ssh/*.pem".into()),
            ("every ssh file", "head -n 3 ~/.ssh/*".into()),
            ("provider-key store", "cat ~/.config/biorouter/secrets.yaml".into()),
            ("case variant", "cat ~/.AWS/Credentials".into()),
            ("brace expansion", "cat ~/.aws/{credentials,config}".into()),
            ("quote splice", "cat ~/.aws/cred\"\"entials".into()),
            ("backslash escape", "cat ~/.aws/cred\\entials".into()),
            ("ANSI-C quoting", "cat ~/.aws/$'cred\\x65ntials'".into()),
            ("input redirect", "cat < ~/.aws/credentials".into()),
            ("cd then redirect", "cd ~/.aws && wc -c < credentials".into()),
            ("bash -c", "bash -c 'cat ~/.aws/credentials'".into()),
            ("sh -c with cd", "sh -c \"cd ~/.aws && cat credentials\"".into()),
            (
                "here-document to a shell",
                "bash <<'EOF'\ncd ~/.aws\ncat credentials\nEOF".into(),
            ),
            ("for loop", "for f in ~/.aws/*; do cat \"$f\"; done".into()),
            ("directory variable + glob", "D=~/.aws; cat $D/cred*".into()),
            ("eval", "eval 'cat ~/.aws/credentials'".into()),
            ("zsh alternation", "cat ~/.aws/(credentials|config)".into()),
            (
                "unknown last component",
                "cat ~/.aws/cred$(printf e)ntials".into(),
            ),
            (
                "cd into a substitution",
                "cd \"$(echo ~/.aws)\" && cat credentials".into(),
            ),
            (
                "path in python",
                format!("python3 -c \"print(open('{h}/.aws/credentials').read())\""),
            ),
            (
                "shell-out in python",
                "python3 -c \"import os; os.system('cd ~/.aws && cat credentials')\"".into(),
            ),
            ("find by name", "find ~ -name credentials -exec cat {} \\;".into()),
            ("dotglob", "cd ~ && shopt -s dotglob && cat */credentials".into()),
        ];
        rows.push(("ssh config by variable", "K=~/.ssh; cat \"$K\"/id_*".into()));
        rows
    }

    /// Commands that touch the same directories without naming a secret.
    pub(crate) fn h1_ordinary_commands(home: &Path) -> Vec<String> {
        let h = home.display();
        vec![
            "cat ~/.ssh/id_ed25519.pub".into(),
            "cat ~/.ssh/*.pub".into(),
            "ls ~/.ssh".into(),
            "ls -la ~/.aws".into(),
            format!("cat {h}/.aws/config"),
            "cat ~/.aws/config".into(),
            "cd ~/.aws && ls".into(),
            "cd ~/.ssh && cat config".into(),
            "cat ~/.config/biorouter/config.yaml".into(),
            "cat data.csv".into(),
            "cat ~/notes.txt".into(),
            "echo hello".into(),
            "git status && git log --oneline -3".into(),
            "ls *".into(),
            "grep -rn TODO src".into(),
            "for f in *.csv; do wc -l \"$f\"; done".into(),
            "python3 -c 'print(1 + 1)'".into(),
            "cat $(ls *.csv)".into(),
            "cd src && python3 main.py".into(),
            "find . -name '*.py'".into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::h1_fixtures::*;
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    fn guard_at(dir: &Path) -> SecretGuard {
        SecretGuard::for_dir(dir)
    }

    #[test]
    fn widened_default_patterns_match() {
        let dir = tempdir().unwrap();
        let g = guard_at(dir.path());
        for name in [
            ".env",
            ".env.local",
            "secrets.yaml",
            "server.pem",
            "id_rsa",
            "id_ed25519",
            "keystore.p12",
            "cert.pfx",
        ] {
            assert!(g.is_denied(Path::new(name)), "expected {name} to be denied");
        }
        assert!(g.is_denied(Path::new(".aws/credentials")));
        assert!(g.is_denied(Path::new(".codex/auth.json")));
        assert!(g.is_denied(Path::new(".claude/.credentials.json")));
        assert!(!g.is_denied(Path::new("normal.txt")));
        assert!(!g.is_denied(Path::new("data.csv")));
    }

    /// Directive 2, gate (d): `.biorouterignore`/secret denial is ABSOLUTE and
    /// mode-independent. `SecretGuard` takes no permission mode, and the
    /// extension-manager dispatch boundary (`extension_manager.rs`, the single
    /// choke point every tool call flows through) applies it with no mode
    /// branch — so a user-declared secret stays blocked even in Fully-Automatic
    /// mode, where ordinary file ops run without a prompt. This guards against a
    /// future refactor accidentally making the secret boundary mode-conditional.
    #[test]
    fn biorouterignore_denial_is_mode_independent() {
        let dir = tempdir().unwrap();
        // A user-declared secret via .biorouterignore, plus a built-in floor pattern.
        fs::write(dir.path().join(".biorouterignore"), "private-notes.txt\n").unwrap();
        fs::write(dir.path().join("private-notes.txt"), "top secret").unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let g = guard_at(dir.path());

        // Denial does not depend on any mode argument — there is none to pass.
        assert!(g.is_denied(Path::new("private-notes.txt")));
        assert!(g.is_denied(Path::new(".env")));
        for key in ["path", "file_path", "command"] {
            let args = json!({ key: "private-notes.txt" });
            assert_eq!(
                g.find_denied_path(args.as_object().unwrap()),
                Some("private-notes.txt".to_string()),
                "a .biorouterignore secret must stay blocked regardless of mode ({key})"
            );
        }
    }

    #[test]
    fn find_denied_requires_existing_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let g = guard_at(dir.path());

        // Existing secret referenced by a path-like key -> blocked.
        let args = json!({ "path": ".env" });
        assert_eq!(
            g.find_denied_path(args.as_object().unwrap()),
            Some(".env".to_string())
        );

        // Non-existent secret -> not blocked (creating a new .env is allowed).
        let args = json!({ "path": "config/.env" });
        assert_eq!(g.find_denied_path(args.as_object().unwrap()), None);
    }

    #[test]
    fn shell_command_token_is_scanned() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let g = guard_at(dir.path());
        let args = json!({ "command": "cat .env" });
        assert_eq!(
            g.find_denied_path(args.as_object().unwrap()),
            Some(".env".to_string())
        );
    }

    #[test]
    fn prose_mention_under_non_path_key_is_not_blocked() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let g = guard_at(dir.path());
        // A bare `.env` (no separator) under a non-path key must not trip the
        // boundary even though the file exists.
        let args = json!({ "content": "remember to copy .env.example to .env" });
        assert_eq!(g.find_denied_path(args.as_object().unwrap()), None);
    }

    #[test]
    fn separator_path_under_any_key_is_scanned() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/.env"), "SECRET=1").unwrap();
        let g = guard_at(dir.path());
        // Unknown key, but the token carries a path separator + resolves to an
        // existing secret -> blocked.
        let args = json!({ "resource": "sub/.env" });
        assert_eq!(
            g.find_denied_path(args.as_object().unwrap()),
            Some("sub/.env".to_string())
        );
    }

    #[test]
    fn benign_arguments_pass() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("data.csv"), "a,b").unwrap();
        let g = guard_at(dir.path());
        let args = json!({ "path": "data.csv", "command": "wc -l data.csv" });
        assert_eq!(g.find_denied_path(args.as_object().unwrap()), None);
    }

    #[test]
    fn secret_floor_survives_custom_biorouterignore() {
        let dir = tempdir().unwrap();
        // A project that ignores build artifacts must not lose .env protection.
        fs::write(dir.path().join(".biorouterignore"), "target/\n*.log\n").unwrap();
        let g = guard_at(dir.path());
        assert!(g.is_denied(Path::new(".env")));
        assert!(g.is_denied(Path::new("secrets.yaml")));
        assert!(g.is_denied(Path::new("build.log")));
        assert!(!g.is_denied(Path::new("normal.txt")));
    }

    // ---- cache (BR / 6.2d) ----------------------------------------------
    //
    // The staleness tests are the point of this cache. A `.biorouterignore`
    // edit that the cache does not honour is a security regression, not a
    // performance bug.

    #[test]
    fn cache_returns_the_same_guard_for_the_same_dir() {
        let dir = tempdir().unwrap();
        let a = SecretGuard::cached_for_dir(dir.path());
        let b = SecretGuard::cached_for_dir(dir.path());
        assert!(
            Arc::ptr_eq(&a, &b),
            "second lookup rebuilt the guard instead of hitting the cache"
        );
    }

    #[test]
    fn cache_honours_a_new_biorouterignore_rule() {
        let dir = tempdir().unwrap();
        let ignore = dir.path().join(".biorouterignore");
        fs::write(&ignore, "# nothing yet\n").unwrap();

        let before = SecretGuard::cached_for_dir(dir.path());
        assert!(
            !before.is_denied(Path::new("private-notes.txt")),
            "precondition: the file is readable before the user protects it"
        );

        // The user protects a path. The very next dispatch must honour it.
        fs::write(&ignore, "# nothing yet\nprivate-notes.txt\n").unwrap();

        let after = SecretGuard::cached_for_dir(dir.path());
        assert!(
            after.is_denied(Path::new("private-notes.txt")),
            "STALE GUARD: a .biorouterignore edit was not honoured. The agent \
             would read a file the user explicitly protected"
        );
    }

    #[test]
    fn cache_honours_a_removed_biorouterignore_rule() {
        let dir = tempdir().unwrap();
        let ignore = dir.path().join(".biorouterignore");
        fs::write(&ignore, "notes.txt\n").unwrap();
        assert!(SecretGuard::cached_for_dir(dir.path()).is_denied(Path::new("notes.txt")));

        // Deleting the file entirely changes the source list, not just its
        // bytes — the fingerprint must catch that too.
        fs::remove_file(&ignore).unwrap();
        assert!(
            !SecretGuard::cached_for_dir(dir.path()).is_denied(Path::new("notes.txt")),
            "STALE GUARD: a deleted .biorouterignore was still in effect"
        );
    }

    /// The global `<config>/.biorouterignore` is the second file a guard reads.
    /// Mutating the real config dir from a test is not safe (it is process-wide
    /// and shared with every other test), so exercise the multi-source
    /// fingerprint through the same helper the global file flows through.
    #[test]
    fn fingerprint_tracks_every_ignore_source() {
        let dir = tempdir().unwrap();
        let global = dir.path().join("global.biorouterignore");
        let local = dir.path().join(".biorouterignore");
        fs::write(&global, "a.txt\n").unwrap();
        fs::write(&local, "b.txt\n").unwrap();
        let sources = vec![global.clone(), local.clone()];

        let first = fingerprint_sources(&sources).expect("fingerprintable");

        // A change to the *non-local* (global-position) source must invalidate.
        fs::write(&global, "a.txt\nc.txt\n").unwrap();
        let second = fingerprint_sources(&sources).expect("fingerprintable");
        assert_ne!(
            first, second,
            "a change to the global ignore file left the fingerprint unchanged"
        );

        // And it really does reach the built guard.
        let guard = SecretGuard::build(dir.path(), &sources);
        assert!(guard.is_denied(Path::new("c.txt")));
        assert!(guard.is_denied(Path::new("b.txt")));
    }

    #[test]
    fn fingerprint_covers_the_global_config_ignore_file() {
        // Structural guard: `ignore_sources` is what both the builder and the
        // fingerprint consume, so the global file can never be read by one and
        // missed by the other.
        let dir = tempdir().unwrap();
        let sources = ignore_sources(dir.path());
        let global = etcetera::choose_app_strategy(crate::APP_STRATEGY.clone())
            .map(|s| s.config_dir().join(".biorouterignore"))
            .expect("app strategy");
        if global.is_file() {
            assert!(
                sources.contains(&global),
                "an existing global .biorouterignore was not tracked"
            );
        } else {
            assert!(!sources.contains(&global));
        }
    }

    #[test]
    fn distinct_dirs_get_distinct_guards() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        fs::write(a.path().join(".biorouterignore"), "only-in-a.txt\n").unwrap();

        let ga = SecretGuard::cached_for_dir(a.path());
        let gb = SecretGuard::cached_for_dir(b.path());

        assert!(!Arc::ptr_eq(&ga, &gb));
        assert!(ga.is_denied(Path::new("only-in-a.txt")));
        assert!(
            !gb.is_denied(Path::new("only-in-a.txt")),
            "a guard was served across working directories"
        );
    }

    #[test]
    fn cached_guard_matches_uncached_guard() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(".biorouterignore"),
            "!secrets.public\nx.log\n",
        )
        .unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();

        let cached = SecretGuard::cached_for_dir(dir.path());
        let fresh = SecretGuard::for_dir(dir.path());
        for p in [".env", "secrets.public", "x.log", "normal.txt"] {
            assert_eq!(
                cached.is_denied(Path::new(p)),
                fresh.is_denied(Path::new(p)),
                "cached and uncached guards disagree on {p}"
            );
        }
        let args = json!({ "path": ".env" });
        assert_eq!(
            cached.find_denied_path(args.as_object().unwrap()),
            Some(".env".to_string())
        );
    }

    /// #68 regression: a project's own `.biorouterignore` is scoped to that
    /// project. Once the guard is re-rooted onto the session working directory,
    /// its patterns are matched against *every* candidate — including absolute
    /// paths outside the project, which Auto mode deliberately lets through the
    /// containment jail. A bare gitignore pattern matches a basename anywhere,
    /// so an unrelated file that merely shares a name with something the project
    /// ignores was refused as "restricted by .biorouterignore".
    ///
    /// The floor (and the global ignore file) still apply everywhere: they are
    /// machine-wide statements about what is a secret, not project-local ones.
    #[test]
    fn project_patterns_stop_at_the_project_root() {
        let project = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(project.path().join(".biorouterignore"), "notes.txt\n").unwrap();
        fs::write(project.path().join("notes.txt"), "project notes").unwrap();
        fs::write(outside.path().join("notes.txt"), "someone else's notes").unwrap();
        fs::write(outside.path().join(".env"), "SECRET=1").unwrap();

        let g = guard_at(project.path());

        // Inside the project the pattern is in force, by absolute and relative path.
        assert!(g.is_denied(&project.path().join("notes.txt")));
        assert!(g.is_denied(Path::new("notes.txt")));

        // Outside it, the project's pattern says nothing.
        assert!(
            !g.is_denied(&outside.path().join("notes.txt")),
            "a project pattern denied an unrelated file outside the project"
        );

        // But the built-in floor is not project-scoped.
        assert!(
            g.is_denied(&outside.path().join(".env")),
            "the built-in secret floor must keep applying outside the project"
        );
    }

    /// The other half of the scoping: "outside the project" is a question about
    /// the directory, not about how the path spells it. A file reached through
    /// a symlink that lands inside the project is inside the project, and the
    /// project's own rules still cover it — otherwise scoping the patterns
    /// would have handed anyone a way to read a protected file by naming it
    /// through a link.
    #[test]
    #[cfg(unix)]
    fn project_patterns_survive_a_symlinked_route_into_the_project() {
        let project = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();
        fs::write(project.path().join(".biorouterignore"), "notes.txt\n").unwrap();
        fs::write(project.path().join("notes.txt"), "project notes").unwrap();

        // A path outside the root textually, inside it in fact.
        let link = elsewhere.path().join("link");
        std::os::unix::fs::symlink(project.path(), &link).unwrap();

        let g = guard_at(project.path());
        assert!(
            g.is_denied(&link.join("notes.txt")),
            "a protected project file became readable by naming it through a symlink"
        );
    }

    /// The same scoping through the argument scanner, which is what the
    /// extension-manager dispatch boundary actually calls.
    #[test]
    fn find_denied_path_scopes_project_patterns_too() {
        let project = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(project.path().join(".biorouterignore"), "notes.txt\n").unwrap();
        fs::write(project.path().join("notes.txt"), "project notes").unwrap();
        let stranger = outside.path().join("notes.txt");
        fs::write(&stranger, "someone else's notes").unwrap();

        let g = guard_at(project.path());
        assert_eq!(
            g.find_denied_path(json!({ "path": "notes.txt" }).as_object().unwrap()),
            Some("notes.txt".to_string()),
            "the project's own file must still be blocked"
        );
        assert_eq!(
            g.find_denied_path(
                json!({ "path": stranger.to_string_lossy() })
                    .as_object()
                    .unwrap()
            ),
            None,
            "a project pattern blocked an unrelated file outside the project"
        );
    }

    fn scan_command(g: &SecretGuard, env: &ShellEnv, command: &str) -> Option<String> {
        g.find_denied_path_in(json!({ "command": command }).as_object().unwrap(), env)
    }

    #[test]
    fn h1_every_spelling_of_a_secret_is_refused() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let leaked: Vec<String> = h1_leaking_spellings(&fake.home)
            .into_iter()
            .filter(|(_, command)| scan_command(&g, &env, command).is_none())
            .map(|(name, command)| format!("  {name}: {command}"))
            .collect();
        assert!(
            leaked.is_empty(),
            "{} spelling(s) reached a secret:\n{}",
            leaked.len(),
            leaked.join("\n")
        );
    }

    #[test]
    #[cfg(unix)]
    fn h1_a_symlink_into_a_secret_store_is_refused() {
        let fake = FakeHome::new().with_links();
        let (g, env) = (fake.guard(), fake.env());
        for command in ["cat aws-link/credentials", "cat creds-link", "cat aws-link/cred*"] {
            assert!(
                scan_command(&g, &env, command).is_some(),
                "a link into the fake ~/.aws reached the secret: {command}"
            );
        }
    }

    #[test]
    fn h1_ordinary_commands_are_not_refused() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        for command in h1_ordinary_commands(&fake.home) {
            assert_eq!(
                scan_command(&g, &env, &command),
                None,
                "refused an ordinary command: {command}"
            );
        }
    }

    /// The Developer shell's per-call `working_directory` moves the directory
    /// the command runs in, so the dispatch scan must resolve `command` there
    /// too. Otherwise `{"command": "cat credentials", "working_directory":
    /// "<home>/.aws"}` passes both checks.
    #[test]
    fn h1_a_working_directory_argument_moves_the_base() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let args = json!({
            "command": "cat credentials",
            "working_directory": fake.home.join(".aws").to_string_lossy(),
        });
        assert!(g.find_denied_path_in(args.as_object().unwrap(), &env).is_some());
    }

    /// `computercontroller__automation_script` carries its body under `script`.
    #[test]
    fn h1_an_automation_script_is_scanned_like_a_command() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let args = json!({ "language": "shell", "script": "cd ~/.aws\nhead -5 credentials\n" });
        assert!(g.find_denied_path_in(args.as_object().unwrap(), &env).is_some());
    }

    #[test]
    fn negation_can_reopen_a_specific_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".biorouterignore"), "!secrets.public\n").unwrap();
        let g = guard_at(dir.path());
        // The negation reopens this one file...
        assert!(!g.is_denied(Path::new("secrets.public")));
        // ...but the rest of the floor still stands.
        assert!(g.is_denied(Path::new(".env")));
        assert!(g.is_denied(Path::new("secrets.yaml")));
    }
}
