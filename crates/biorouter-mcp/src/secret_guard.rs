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
//!
//! # H1: judge the path, not the token (2026-09)
//!
//! Until QA-C's H1 the argument scan split a command on whitespace, matched each
//! token against the deny set as a literal path, and let a match through unless
//! that literal existed. `cat /Users/x/.aws/credentials` was refused;
//! `cat ~/.aws/credentials`, `cat $HOME/.aws/credentials`, `cat ~/.aws/cred*` and
//! `cd ~/.aws && head credentials` all handed a public model the AWS key,
//! because the ignore crate expands nothing and no directory named `~` exists.
//!
//! Now a command is read the way the shell will read it ([`resolve`], over
//! [`lex`] and [`expand`]): `~`, `~user`, `$VAR`, `${VAR:-…}` and assignments made
//! earlier in the same command are expanded; relative paths resolve against the
//! directory the command is in *at that point*, through `cd`, `pushd` and a
//! sibling `working_directory` argument; globs are matched against the real
//! directory; symlinks are canonicalised; `sh -c`, `eval` and here-documents are
//! followed. The resolved path is matched component by component, with case
//! folded, and a match is a refusal — whether or not the file exists.
//!
//! This is still a static check of text, and it says so: a path computed at run
//! time by a program (`python -c` assembling it, base64) is invisible to it.
//! That is why tool *output* is scanned for credential material as well
//! (`biorouter::guardrails::secret_output`).

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

mod expand;
mod glob;
mod lex;
mod resolve;

pub use expand::ShellEnv;

/// Built-in secret/credential deny patterns applied as an always-on floor.
///
/// Kept deliberately tight — every entry names a file that is a secret by
/// convention — because the dispatch scan refuses a match whether or not the
/// file exists (H1), so a loose entry would refuse ordinary config files.
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
    // Q4-56: the long tail of well-known credential stores. Each holds a password or token in
    // a shape the output redaction does not recognise (a netrc `password` line, a pgpass
    // `host:port:db:user:pass` row, a `https://user:token@host` URL, docker's base64 `auth`, a
    // lowercase kubeconfig `token:`), so the name is the only thing that can stop a read.
    "**/.netrc",
    "**/_netrc",
    "**/.pgpass",
    "**/.git-credentials",
    "**/.docker/config.json",
    "**/.kube/config",
    "**/.config/gh/hosts.yml",
    // The shared daemon's user-action key, saved by whoever started the daemon so the desktop
    // app and `biorouter crew` can connect without asking the person for anything.
    "**/daemon/user-action-key.json",
    // BB-25: the cookie jars that hold a person's logins. Chrome's own (macOS
    // `Library/Application Support/Google/Chrome/<profile>`, Windows `Google/Chrome/User Data`,
    // Linux `.config/google-chrome`) with its saved passwords and payment data beside them, and
    // the Biorouter Browser's profile, which holds what a Chrome import set.
    "**/Google/Chrome/**/Cookies",
    "**/Google/Chrome/**/Login Data",
    "**/Google/Chrome/**/Login Data For Account",
    "**/Google/Chrome/**/Web Data",
    "**/google-chrome/**/Cookies",
    "**/google-chrome/**/Login Data",
    "**/google-chrome/**/Login Data For Account",
    "**/google-chrome/**/Web Data",
    "**/Partitions/biorouter-embedded-browser/**/Cookies",
];

/// Conventional `.env` templates, reopened on top of the floor (issue #522).
///
/// `**/.env.*` above also matches the templates a repository commits next to
/// its `.env`, which hold variable names and no values, so an agent could not
/// read one or run the usual `cp .env.example .env`. These negations are added
/// right after the floor and before any `.biorouterignore`, so gitignore's
/// last-match-wins order still lets a project or the global file deny a
/// template again. They name exact file names only: `.env`, `.env.local` and
/// `.env.production` stay denied, and a template that is a symlink is judged
/// as the file it reaches, so a link from `.env.example` to `.env` is refused.
pub const DEFAULT_SECRET_TEMPLATE_EXCEPTIONS: &[&str] = &[
    "!**/.env.example",
    "!**/.env.sample",
    "!**/.env.template",
    "!**/.env.dist",
    "!**/.env.tpl",
    "!**/.env.defaults",
    "!**/.env.*.example",
];

/// Add the built-in floor, then its template exceptions, to a builder. Both
/// guard constructors call this, so their floors cannot drift apart.
fn add_builtin_floor(builder: &mut GitignoreBuilder) {
    for pattern in DEFAULT_SECRET_PATTERNS
        .iter()
        .chain(DEFAULT_SECRET_TEMPLATE_EXCEPTIONS)
    {
        let _ = builder.add_line(None, pattern);
    }
}

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
                // An MCP resource locator. Read as a path, so a bare name and a
                // template that is a symlink are judged (issue #522).
                | "resource"
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

    /// Load policy through the same retained directory used by a bounded operation.
    pub fn for_bounded_path(
        path: &biorouter_sandbox::bounded_fs::BoundedPath,
    ) -> std::io::Result<Self> {
        let cwd = path.root();
        let mut builder = GitignoreBuilder::new(cwd);
        let mut machine_builder = GitignoreBuilder::new(cwd);
        add_builtin_floor(&mut builder);
        add_builtin_floor(&mut machine_builder);
        if let Ok(dirs) = crate::paths::platform_dirs() {
            let global = dirs.config.join(".biorouterignore");
            let _ = builder.add(&global);
            let _ = machine_builder.add(&global);
        }
        match path.read_root_policy().and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        }) {
            Ok(content) => {
                for line in content.lines() {
                    let _ = builder.add_line(Some(cwd.join(".biorouterignore")), line);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(std::io::Error::new(error.kind(), format!("Cannot read security policy '{}': {error}. Repair this policy file before retrying file access.", cwd.join(".biorouterignore").display()))),
        }
        Ok(Self {
            ignore: builder.build().unwrap_or_else(|_| Gitignore::empty()),
            machine_wide: machine_builder
                .build()
                .unwrap_or_else(|_| Gitignore::empty()),
            root: cwd.to_path_buf(),
            canonical_root: cwd.to_path_buf(),
        })
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

        add_builtin_floor(&mut builder);
        add_builtin_floor(&mut machine_builder);

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
    /// **Case is folded as well** (H1). APFS and NTFS are case-insensitive, so
    /// `~/.AWS/Credentials` opens the real `~/.aws/credentials`, and the
    /// patterns are case-sensitive. A path is therefore denied if it *or its
    /// lowercase form* is. That can only add refusals: on a case-sensitive
    /// filesystem it refuses a name that differs from a secret's by case.
    ///
    /// Normally IO-free, so it stays cheap to call per token: the containment
    /// question is only asked when the project's patterns actually change the
    /// verdict, and only reaches the filesystem when the path is not already a
    /// textual descendant of the root.
    pub fn is_denied(&self, path: &Path) -> bool {
        if self.is_denied_as_spelled(path, path) {
            return true;
        }
        match path.to_str() {
            // The lowercase form is a second spelling to MATCH, never a second
            // place: whether the project's own patterns apply is asked of `path`.
            // On a case-sensitive filesystem the lowercase form of a project
            // under `/tmp/.tmpAbC…` names nothing, so a project's `!.netrc`
            // used to lose to the floor there (Linux only).
            Some(text) if text.chars().any(char::is_uppercase) => {
                self.is_denied_as_spelled(Path::new(&text.to_lowercase()), path)
            }
            _ => false,
        }
    }

    /// `spelled` is matched against the patterns; `located` is the path whose
    /// containment in the root decides whether the project's patterns apply.
    fn is_denied_as_spelled(&self, spelled: &Path, located: &Path) -> bool {
        let everything = self.ignore.matched(spelled, false).is_ignore();
        let machine_wide = self.machine_wide.matched(spelled, false).is_ignore();
        if everything == machine_wide {
            // The project's own file did not move the needle either way.
            return everything;
        }
        if self.is_inside_root(located) {
            // Inside the project the project has the last word — it may add a
            // rule, and it may negate one of the floor's with `!path`.
            everything
        } else {
            machine_wide
        }
    }

    /// [`Self::is_denied`] for a path about to be opened: also judged after
    /// its symlinks are resolved, so a link into `~/.aws` is judged as the file
    /// it reaches (and, on macOS, `realpath` returns the on-disk case, so
    /// `credentialſ` is judged as `credentials`). A path that does not exist
    /// yet is judged by its nearest existing ancestor's real location.
    pub fn is_denied_resolved(&self, path: &Path) -> bool {
        let lexical = resolve::lexical_normalize(path);
        if self.is_denied(&lexical) {
            return true;
        }
        let physical = resolve::canonical_prefix(path);
        physical != lexical && self.is_denied(&physical)
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

    /// Scan a tool call's arguments for a reference to a denied (secret) path.
    /// Returns the offending text as the call spelled it, or `None`.
    ///
    /// See [`Self::find_denied_access`], which this wraps, for what is scanned
    /// and how.
    pub fn find_denied_path(&self, arguments: &Map<String, Value>) -> Option<String> {
        self.find_denied_access(arguments)
            .map(|denied| denied.shown)
    }

    /// [`Self::find_denied_path`] against an explicit environment — how the
    /// tests run every spelling against a throwaway HOME.
    pub fn find_denied_path_in(
        &self,
        arguments: &Map<String, Value>,
        env: &ShellEnv,
    ) -> Option<String> {
        self.find_denied_access_in(arguments, env)
            .map(|denied| denied.shown)
    }

    /// Scan a tool call's arguments for a reference to a denied (secret) path,
    /// with the process environment `$HOME` and friends expand against.
    ///
    /// **Fail closed (H1).** A candidate that matches the deny set is refused
    /// whether or not it exists. The old scan asked `exists()` of the literal,
    /// unexpanded token, which is exactly how `~/.aws/credentials`,
    /// `$HOME/.aws/credentials` and `cd ~/.aws && head credentials` reached a
    /// public model. A consequence worth knowing: *creating* a file whose name
    /// is in the deny set (`> .env`) is refused too, as `text_editor` already
    /// refused it; a `.biorouterignore` negation reopens one specific file.
    ///
    /// Three kinds of argument, by key:
    ///   * a **command** (`command`, `cmd`, `script`, `shell_command`, …) is read
    ///     the way the shell will read it — quotes, `~`, `$VAR`, globs, brace
    ///     expansion, `cd`, nested `sh -c` / `eval` / here-documents — by
    ///     [`resolve`], and every path it could open is judged after resolution;
    ///   * a **path** (`path`, `file_path`, `dir`, `output`, …) is resolved the
    ///     same way as a single word, symlinks included;
    ///   * anything else is **prose**: only tokens that carry a path separator
    ///     are considered, so `.env` mentioned in a `content`/`message` field
    ///     does not trip the boundary.
    ///
    /// Every string also gets the literal whitespace-token pass the guard has
    /// always made, now without the `exists()` gate. It is IO-free and it is
    /// what keeps this change from permitting anything the old scan refused.
    /// The whole string is one more literal candidate, except for a command
    /// line of several words, whose words the resolver reads (see [`Whole`]).
    ///
    /// This scan does not know which tool it is reading, so prose keeps the
    /// separator rule even where a tool never opens it. The dispatch boundary,
    /// which does know, leaves out the free-text fields of the bundled Todo
    /// tools and the source of Code Execution's `execute_code`, whose inner
    /// calls are each dispatched, and judged, on their own (issue #522).
    ///
    /// A sibling `working_directory` / `cwd` argument is treated as another
    /// directory the command may run in, because for the Developer shell it is
    /// one.
    pub fn find_denied_access(&self, arguments: &Map<String, Value>) -> Option<DeniedAccess> {
        self.find_denied_access_in(arguments, &ShellEnv::from_process())
    }

    /// [`Self::find_denied_access`] against an explicit environment.
    pub fn find_denied_access_in(
        &self,
        arguments: &Map<String, Value>,
        env: &ShellEnv,
    ) -> Option<DeniedAccess> {
        let mut bases = vec![resolve::Base::Dir(self.root.clone())];
        for (key, value) in arguments {
            if key_is_working_directory(key) {
                if let Value::String(dir) = value {
                    bases.extend(self.working_directory_bases(dir, env));
                }
            }
        }
        let mut resolver = resolve::Resolver::new(self, env);
        let mut found = None;
        for (key, value) in arguments {
            self.walk_value(Some(key), value, &bases, &mut resolver, &mut found);
            if found.is_some() {
                break;
            }
        }
        found
    }

    /// Judge a shell command that will run in `cwd` — the Developer server's
    /// own check, which knows the directory the command really runs in.
    pub fn find_denied_in_command(
        &self,
        command: &str,
        cwd: &Path,
        env: &ShellEnv,
    ) -> Option<DeniedAccess> {
        let mut resolver = resolve::Resolver::new(self, env);
        if let Err(finding) =
            resolver.scan_command(command, &[resolve::Base::Dir(cwd.to_path_buf())])
        {
            return Some(DeniedAccess::from(finding));
        }
        let mut found = None;
        self.legacy_scan(command, true, Whole::OnlyOneWord, &mut found);
        found
    }

    /// Judge a script in a language that is not a shell (Ruby, PowerShell,
    /// AppleScript): every path-looking literal, and every string it hands to
    /// a shell.
    pub fn find_denied_in_code(
        &self,
        code: &str,
        cwd: &Path,
        env: &ShellEnv,
    ) -> Option<DeniedAccess> {
        let mut found = None;
        self.legacy_scan(code, true, Whole::OnlyOneWord, &mut found);
        if found.is_some() {
            return found;
        }
        let mut resolver = resolve::Resolver::new(self, env);
        resolver
            .scan_code(code, &[resolve::Base::Dir(cwd.to_path_buf())])
            .err()
            .map(DeniedAccess::from)
    }

    fn working_directory_bases(&self, dir: &str, env: &ShellEnv) -> Vec<resolve::Base> {
        let dir = dir.trim();
        if dir.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut push = |path: PathBuf| {
            let path = if path.is_absolute() {
                path
            } else {
                self.root.join(path)
            };
            out.push(resolve::Base::Dir(resolve::lexical_normalize(&path)));
        };
        push(PathBuf::from(dir));
        if let Some(rest) = dir.strip_prefix('~') {
            if let Some(home) = env.home() {
                push(home.join(rest.trim_start_matches(['/', '\\'])));
            }
        }
        out
    }

    fn walk_value(
        &self,
        key: Option<&str>,
        value: &Value,
        bases: &[resolve::Base],
        resolver: &mut resolve::Resolver<'_>,
        found: &mut Option<DeniedAccess>,
    ) {
        if found.is_some() {
            return;
        }
        match value {
            Value::String(s) => self.scan_string(key, s, bases, resolver, found),
            Value::Array(items) => {
                for item in items {
                    self.walk_value(key, item, bases, resolver, found);
                    if found.is_some() {
                        return;
                    }
                }
            }
            Value::Object(map) => {
                for (k, v) in map {
                    self.walk_value(Some(k), v, bases, resolver, found);
                    if found.is_some() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_string(
        &self,
        key: Option<&str>,
        s: &str,
        bases: &[resolve::Base],
        resolver: &mut resolve::Resolver<'_>,
        found: &mut Option<DeniedAccess>,
    ) {
        let pathlike_key = key.map(key_is_pathlike).unwrap_or(false);
        // Either pass refusing is a refusal. For a command the resolver goes
        // first only so the message names the path it resolved to; the literal
        // pass would refuse `cat ~/.aws/credentials` on its own, as one path
        // ending in `.aws/credentials`.
        let kind = key.map(key_kind).unwrap_or(KeyKind::Prose);
        if kind == KeyKind::Command {
            if let Err(finding) = resolver.scan_command(s, bases) {
                *found = Some(DeniedAccess::from(finding));
                return;
            }
        }
        let whole = if kind == KeyKind::Command {
            Whole::OnlyOneWord
        } else {
            Whole::Always
        };
        self.legacy_scan(s, pathlike_key, whole, found);
        if found.is_some() || kind != KeyKind::Path {
            return;
        }
        if let Err(finding) = resolver.scan_path_value(s, bases) {
            *found = Some(DeniedAccess::from(finding));
        }
    }

    /// The whole string, then each whitespace token, taken literally — the
    /// scan the guard has always made, minus its `exists()` gate.
    fn legacy_scan(
        &self,
        s: &str,
        pathlike_key: bool,
        whole: Whole,
        found: &mut Option<DeniedAccess>,
    ) {
        let trimmed = s.trim();

        // Whole-string candidate (covers a plain `{"path": ".env"}` argument).
        let whole_is_a_candidate = match whole {
            Whole::Always => true,
            Whole::OnlyOneWord => !trimmed.contains(char::is_whitespace),
        };
        if whole_is_a_candidate
            && (pathlike_key || has_separator(trimmed))
            && self.candidate_is_denied(trimmed)
        {
            *found = Some(DeniedAccess::literal(trimmed));
            return;
        }

        // Token candidates (covers shell command lines and multi-path values).
        for token in trimmed.split_whitespace() {
            let tok = token.trim_matches(|c| c == '"' || c == '\'');
            if tok.is_empty() || tok.starts_with('-') {
                continue;
            }
            if (pathlike_key || has_separator(tok)) && self.candidate_is_denied(tok) {
                *found = Some(DeniedAccess::literal(tok));
                return;
            }
        }
    }

    /// Pattern match only, and IO-free. There used to be an `exists()` check
    /// after the match, and it is gone on purpose (H1): it was asked of the
    /// unexpanded token, so `~/.aws/credentials` matched the pattern and was
    /// then let through because no directory named `~` exists. A match is a
    /// refusal.
    fn candidate_is_denied(&self, candidate: &str) -> bool {
        if candidate.is_empty() {
            return false;
        }
        let path = Path::new(candidate);
        // `join` replaces the base when `path` is absolute, so this handles both
        // relative and absolute candidates.
        let resolved = self.root.join(path);
        self.is_denied(&resolved) || self.is_denied(path)
    }
}

/// A refused reference to a secret, and how to explain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeniedAccess {
    /// The text as the tool call spelled it.
    pub shown: String,
    /// What it resolved to, when that differs usefully from `shown`.
    pub resolved: Option<PathBuf>,
    reason: resolve::Reason,
    heredoc: Option<resolve::BodyReading>,
}

impl DeniedAccess {
    fn literal(shown: &str) -> Self {
        Self {
            shown: shown.to_string(),
            resolved: None,
            reason: resolve::Reason::Pattern,
            heredoc: None,
        }
    }

    /// The refusal, written for the model that made the call: what was refused,
    /// why, and the one way to reopen a file on purpose.
    pub fn message(&self) -> String {
        let target = match &self.resolved {
            Some(path) if path.to_string_lossy() != self.shown => {
                format!("'{}' (resolves to {})", self.shown, path.display())
            }
            _ => format!("'{}'", self.shown),
        };
        let refusal = match self.reason {
            resolve::Reason::Pattern => format!(
                "Access to {target} is blocked: it matches a secret/credential deny pattern \
                 (.env, private key, cloud or provider credentials). The check does not depend \
                 on whether the file exists or how the path is spelled. Add a negation to \
                 .biorouterignore to allow one specific file."
            ),
            resolve::Reason::UnresolvedInSecretDirectory => format!(
                "Access to {target} is blocked: part of the path cannot be known until the \
                 command runs, and it would be read from a directory that holds a protected \
                 secret/credential file. Name the file you need explicitly instead."
            ),
            resolve::Reason::PatternTail => format!(
                "Access to {target} is blocked: part of the path cannot be known until the \
                 command runs, and the rest of it could name a protected secret/credential file. \
                 Name the file you need explicitly instead."
            ),
            resolve::Reason::TooComplex => format!(
                "The command is blocked: the secret guard could not verify which files {target} \
                 reads, because the command nests shells too deeply or names more links than it \
                 can follow. Split it into simpler commands."
            ),
            resolve::Reason::BrowserKeyHelper => "The command is blocked: it runs `biorouterd \
                 browser-key`, the helper that reads the key to the person's Chrome logins for \
                 the desktop app's Import from Chrome. Only the Biorouter app runs it, when the \
                 person imports from Chrome in Settings. Do not run it in any other way; if the \
                 task needs the person's Chrome logins, ask them to import from Chrome themselves."
                .to_string(),
        };
        // #546: a model that wrote the body as text looks for the word among
        // its commands and finds nothing, so say where it was.
        match self.heredoc {
            None => refusal,
            Some(resolve::BodyReading::Script) => format!(
                "{refusal} The word is inside a here-document body, which the guard reads as a \
                 script this command may run. If the body is the content of a file, write it in \
                 a call of its own as `cat > FILE <<'EOF'`, with the delimiter quoted, so the \
                 body is read as file content."
            ),
            Some(resolve::BodyReading::Data) => format!(
                "{refusal} The word is inside a here-document body written to a file. The guard \
                 still reads such a body, because the file can be run later."
            ),
        }
    }
}

impl From<resolve::Finding> for DeniedAccess {
    fn from(finding: resolve::Finding) -> Self {
        Self {
            shown: finding.shown,
            resolved: finding.path,
            reason: finding.reason,
            heredoc: finding.heredoc,
        }
    }
}

/// Whether [`SecretGuard::legacy_scan`] also takes the whole string as one
/// path.
///
/// A command line or a script is several words. Each word is a candidate of
/// its own, and the resolver reads the command as the shell will, keeping a
/// quoted path with spaces together, so the whole line names no file. It used
/// to be judged as one anyway, which refused `cat config/.env.example | head`
/// because the text after its last `/`, `.env.example | head`, matches
/// `**/.env.*` (issue #522). Every word of it is still judged.
///
/// Any other value keeps the whole-string candidate: a path argument can hold
/// spaces, and so can a path an unknown tool takes under a key that does not
/// look like one (`/keys/My Server.pem`), where no single word matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Whole {
    Always,
    OnlyOneWord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    /// Read the way a shell will read it.
    Command,
    /// A single path a tool will open.
    Path,
    Prose,
}

fn key_kind(key: &str) -> KeyKind {
    let k = key.to_ascii_lowercase();
    if k.contains("command")
        || k.contains("cmd")
        || k.contains("script")
        || k.contains("shell")
        || matches!(k.as_str(), "bash" | "sh" | "zsh" | "exec")
    {
        KeyKind::Command
    } else if key_is_pathlike(key) {
        KeyKind::Path
    } else {
        KeyKind::Prose
    }
}

/// A sibling argument naming the directory a command runs in.
fn key_is_working_directory(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "working_directory"
            | "working_dir"
            | "workdir"
            | "cwd"
            | "directory"
            | "dir"
            | "current_dir"
            | "current_directory"
    )
}

fn has_separator(s: &str) -> bool {
    s.contains('/') || s.contains('\\')
}

/// Ignore files that contribute to a guard rooted at `cwd`, in the order they
/// are layered: global (`<config>/.biorouterignore`) first, then project-local
/// (`<cwd>/.biorouterignore`). Only files that exist are returned, so creating
/// or deleting one changes the list itself and therefore the fingerprint.
fn ignore_sources(cwd: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::with_capacity(2);

    if let Ok(dirs) = crate::paths::platform_dirs() {
        let global = dirs.config.join(".biorouterignore");
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
        // Forward-slash the embedded absolute path: Windows accepts `/` as a
        // separator, so a row that is not specifically testing backslash
        // handling reads the same on every platform. The backslash-as-separator
        // behaviour is proven by the dedicated `cd` tests and the lexer's own
        // `windows_backslash_is_a_separator_not_an_escape`.
        let h = home.display().to_string().replace('\\', "/");
        let mut rows: Vec<(&'static str, String)> = vec![
            ("absolute", format!("cat {h}/.aws/credentials")),
            ("tilde", "cat ~/.aws/credentials".into()),
            ("$HOME", "cat $HOME/.aws/credentials".into()),
            ("${HOME}", "cat ${HOME}/.aws/credentials".into()),
            ("quoted $HOME", "cat \"$HOME/.aws/credentials\"".into()),
            (
                "variable from $HOME",
                "H=$HOME; cat $H/.aws/credentials".into(),
            ),
            ("absolute glob", format!("cat {h}/.aws/cred*")),
            ("tilde glob", "cat ~/.aws/cred*".into()),
            ("cd && head", format!("cd {h}/.aws && head credentials")),
            ("cd ; cat", "cd ~/.aws; cat credentials".into()),
            ("pushd", "pushd ~/.aws >/dev/null && cat credentials".into()),
            ("cd with no argument", "cd; cat .aws/credentials".into()),
            ("ssh private key", "cat ~/.ssh/id_ed25519".into()),
            ("pem glob", "cat ~/.ssh/*.pem".into()),
            ("every ssh file", "head -n 3 ~/.ssh/*".into()),
            (
                "provider-key store",
                "cat ~/.config/biorouter/secrets.yaml".into(),
            ),
            ("case variant", "cat ~/.AWS/Credentials".into()),
            ("brace expansion", "cat ~/.aws/{credentials,config}".into()),
            ("quote splice", "cat ~/.aws/cred\"\"entials".into()),
            ("ANSI-C quoting", "cat ~/.aws/$'cred\\x65ntials'".into()),
            ("input redirect", "cat < ~/.aws/credentials".into()),
            (
                "cd then redirect",
                "cd ~/.aws && wc -c < credentials".into(),
            ),
            ("bash -c", "bash -c 'cat ~/.aws/credentials'".into()),
            (
                "sh -c with cd",
                "sh -c \"cd ~/.aws && cat credentials\"".into(),
            ),
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
            (
                "find by name",
                "find ~ -name credentials -exec cat {} \\;".into(),
            ),
            (
                "dotglob",
                "cd ~ && shopt -s dotglob && cat */credentials".into(),
            ),
        ];
        rows.push(("ssh config by variable", "K=~/.ssh; cat \"$K\"/id_*".into()));
        // `cred\entials` is a POSIX escape that resolves to `credentials`. This
        // row was `#[cfg(not(windows))]` for one commit, on the reasoning that a
        // Windows shell reads `\` as a separator and would look for
        // `.aws/cred/entials` — true of the Windows grammar *alone*, and no
        // longer true of the guard: a Windows build reads the input under both
        // grammars (`resolve::grammars`) and the POSIX one still refuses this.
        // The row therefore belongs on every platform, and its passing on
        // Windows is independent evidence the union is wired up.
        rows.push(("backslash escape", "cat ~/.aws/cred\\entials".into()));
        rows
    }

    /// Commands that touch the same directories without naming a secret.
    pub(crate) fn h1_ordinary_commands(home: &Path) -> Vec<String> {
        let h = home.display().to_string().replace('\\', "/");
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
        assert!(g.is_denied(Path::new("daemon/user-action-key.json")));
        assert!(g.is_denied(Path::new(
            ".local/state/biorouter/daemon/user-action-key.json"
        )));
        assert!(!g.is_denied(Path::new("normal.txt")));
        assert!(!g.is_denied(Path::new("data.csv")));
    }

    /// The credential stores Q4-56 added, by name, at any depth and in any case.
    const Q4_56_STORES: &[&str] = &[
        ".netrc",
        "_netrc",
        ".pgpass",
        ".git-credentials",
        ".docker/config.json",
        ".kube/config",
        ".config/gh/hosts.yml",
    ];

    /// Q4-56: security round 4 uploaded each of these through Crew with a 200, and the same
    /// gap let any chat's shell read them. Each is now on the floor.
    #[test]
    fn q4_56_the_long_tail_of_credential_stores_is_on_the_floor() {
        let fake = FakeHome::new();
        let g = fake.guard();
        for store in Q4_56_STORES {
            assert!(g.is_denied(Path::new(store)), "{store}");
            assert!(g.is_denied(&fake.home.join(store)), "~/{store}");
            assert!(g.is_denied(&fake.project.join(store)), "project/{store}");
            let shouted = store.to_uppercase();
            assert!(g.is_denied(Path::new(&shouted)), "{shouted}");
        }
        // Their neighbours are not credential stores, and a name alone does not make one.
        for ordinary in [
            ".git/config",
            ".ssh/config",
            "config",
            "config.json",
            "hosts.yml",
            ".docker/daemon.json",
            ".kube/cache/discovery.json",
            ".config/gh/config.yml",
            "netrc.md",
            ".pgpass.example",
            "deploy/.kube-config",
        ] {
            assert!(!g.is_denied(Path::new(ordinary)), "{ordinary}");
            assert!(!g.is_denied(&fake.home.join(ordinary)), "~/{ordinary}");
        }
    }

    /// BB-25: the cookie and password stores of Chrome and of the Biorouter Browser's own profile.
    const BROWSER_STORES: &[&str] = &[
        "Library/Application Support/Google/Chrome/Default/Cookies",
        "Library/Application Support/Google/Chrome/Default/Network/Cookies",
        "Library/Application Support/Google/Chrome/Profile 1/Login Data",
        "Library/Application Support/Google/Chrome/Default/Login Data For Account",
        "Library/Application Support/Google/Chrome/Default/Web Data",
        ".config/google-chrome/Default/Cookies",
        ".config/google-chrome/Profile 2/Login Data",
        ".config/google-chrome/Default/Web Data",
        "AppData/Local/Google/Chrome/User Data/Default/Network/Cookies",
        "Library/Application Support/Biorouter/Partitions/biorouter-embedded-browser/Cookies",
        "Library/Application Support/Biorouter/Partitions/biorouter-embedded-browser/Network/Cookies",
    ];

    #[test]
    fn bb_25_browser_cookie_and_password_stores_are_on_the_floor() {
        let fake = FakeHome::new();
        let g = fake.guard();
        for store in BROWSER_STORES {
            assert!(g.is_denied(&fake.home.join(store)), "~/{store}");
        }
        // Chrome's other files, and files merely called Cookies elsewhere, stay readable.
        for ordinary in [
            "Library/Application Support/Google/Chrome/Default/Bookmarks",
            "Library/Application Support/Google/Chrome/Default/Preferences",
            "Library/Application Support/Google/Chrome/Local State",
            "Library/Application Support/Biorouter/biorouter-browser/bookmarks.json",
            "project/Cookies",
            "project/src/cookies/index.ts",
            "notes/Login Data.md",
        ] {
            assert!(!g.is_denied(&fake.home.join(ordinary)), "~/{ordinary}");
        }
        let (g, env) = (fake.guard(), fake.env());
        for command in [
            "cat ~/Library/Application\\ Support/Google/Chrome/Default/Cookies",
            "sqlite3 \"$HOME/Library/Application Support/Google/Chrome/Default/Login Data\" .dump",
            "cp ~/.config/google-chrome/Default/Cookies /tmp/c",
            "cd ~/.config/google-chrome/Default && sqlite3 'Web Data' .tables",
            "cat ~/Library/Application\\ Support/Biorouter/Partitions/biorouter-embedded-browser/Cookies",
        ] {
            assert!(
                scan_command(&g, &env, command).is_some(),
                "reached a browser store: {command}"
            );
        }
    }

    /// BB-23: the browser-key helper reads the key to the person's Chrome cookies, silently once
    /// the Keychain prompt was answered with Always Allow, so an agent's shell never runs it, in
    /// any spelling the guard can read. Searching the code for it stays possible.
    #[test]
    fn bb_23_the_browser_key_helper_is_refused_in_every_spelling() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let refused = [
            "biorouterd browser-key --source chrome",
            "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd browser-key --source chrome",
            "~/repo/target/debug/biorouterd browser-key --source=chrome | xxd",
            "BIOROUTERD browser-key --source chrome",
            "C:\\Program Files\\Biorouter\\resources\\bin\\biorouterd.exe browser-key --source chrome",
            "env FOO=1 biorouterd browser-key --source chrome",
            "exec nohup biorouterd browser-key --source chrome > /tmp/key",
            "\"biorouterd\" 'browser-key' --source chrome",
            "bio\"rou\"terd brow'ser'-key --source chrome",
            "biorouterd browser\\-key --source chrome",
            "B=biorouterd; $B browser-key --source chrome",
            "K=browser-key; biorouterd \"$K\" --source chrome",
            "sh -c 'biorouterd browser-key --source chrome'",
            "bash -lc \"cd /tmp && biorouterd browser-key --source chrome\"",
            "eval 'biorouterd browser-key --source chrome'",
            "alias k='biorouterd browser-key'; k --source chrome",
            "biorouterd $(echo browser-key) --source chrome",
            "cp \"$(which biorouterd)\" /tmp/x && /tmp/x browser-key --source chrome",
            "/tmp/renamed browser-key --source chrome",
            "echo browser-key --source chrome | xargs /tmp/renamed",
            "python3 -c \"import subprocess; subprocess.run(['biorouterd','browser-key','--source','chrome'])\"",
            "cat <<EOF | sh\nbiorouterd browser-key --source chrome\nEOF",
            "cargo run -p biorouter-server --bin biorouterd -- browser-key --source chrome",
        ];
        for command in refused {
            let denied = g
                .find_denied_access_in(json!({ "command": command }).as_object().unwrap(), &env)
                .unwrap_or_else(|| panic!("ran the browser-key helper: {command}"));
            assert!(
                denied.message().contains("Only the Biorouter app runs it"),
                "{command}: {}",
                denied.message()
            );
            assert!(
                g.find_denied_in_command(command, &fake.home, &env)
                    .is_some(),
                "the Developer shell's own check missed: {command}"
            );
        }
        assert!(g
            .find_denied_in_code(
                "system(\"biorouterd browser-key --source chrome\")",
                &fake.home,
                &env
            )
            .is_some());
        for ordinary in [
            "rg browser-key crates/",
            "grep -rn 'browser-key' docs/desktop-ui/built-in-browser.md",
            "cargo test -p biorouter-server --bin biorouterd browser_key_tests",
            "biorouterd --version",
            "ps aux | grep biorouterd",
            "biorouter session list --source mine",
        ] {
            assert!(
                scan_command(&g, &env, ordinary).is_none(),
                "refused an ordinary command: {ordinary}"
            );
        }
    }

    /// Q4-56 through the resolver: every spelling H1 closed for `~/.aws/credentials` is closed
    /// for the new stores too, against a fake HOME that really holds them.
    #[test]
    fn q4_56_every_spelling_of_the_new_stores_is_refused() {
        let fake = FakeHome::new();
        for store in Q4_56_STORES {
            let path = fake.home.join(store);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "placeholder\n").unwrap();
        }
        let (g, env) = (fake.guard(), fake.env());
        for command in [
            "cat ~/.netrc",
            "cat $HOME/.pgpass",
            "head -1 \"$HOME\"/.git-credentials",
            "cat ~/.docker/config*",
            "cd ~/.kube && cat config",
            "cat ~/.KUBE/Config",
            "cat ~/.config/gh/hosts.yml",
            "bash -c 'cat ~/.netrc'",
            "cat ~/_netrc",
            "find ~ -name .pgpass -exec cat {} \\;",
        ] {
            assert!(
                scan_command(&g, &env, command).is_some(),
                "reached a credential store: {command}"
            );
        }
        // The folders around them still work.
        for command in [
            "ls ~/.kube",
            "ls ~/.docker",
            "cat ~/.config/gh/config.yml",
            "kubectl get pods",
            "git config --list",
            "cat .git/config",
        ] {
            assert_eq!(
                scan_command(&g, &env, command),
                None,
                "refused an ordinary command: {command}"
            );
        }
    }

    /// A person can still allow one of the new stores on purpose: a negation, in the project's
    /// `.biorouterignore` or the machine-wide one, reopens that file and nothing else.
    #[test]
    fn q4_56_a_negation_reopens_one_store() {
        let fake = FakeHome::new();
        // A capital in the root, always: on a case-sensitive filesystem the
        // lowercase spelling of `<root>/.netrc` names nothing inside the project,
        // and the negation must still hold (it failed on Linux whenever the
        // random temporary directory's name carried a capital).
        let project = fake.project.join("Lab");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(".biorouterignore"),
            "!**/.kube/config\n!.netrc\n",
        )
        .unwrap();
        let project_file = project.join(".biorouterignore");
        let g = SecretGuard::build(&project, &[project_file]);
        assert!(!g.is_denied(Path::new(".kube/config")));
        assert!(!g.is_denied(&project.join(".netrc")));
        assert!(g.is_denied(Path::new(".pgpass")));
        assert!(g.is_denied(Path::new(".docker/config.json")));
        // The project's word stops at the project: the home's stores stay refused.
        assert!(g.is_denied(&fake.home.join(".kube/config")));

        // The machine-wide file (what Crew consults, rooted at the filesystem root) reopens
        // it everywhere.
        let global = fake.home.join(".config/biorouter/.biorouterignore");
        fs::write(&global, "!**/.kube/config\n").unwrap();
        let root = fake.home.ancestors().last().unwrap().to_path_buf();
        let machine = SecretGuard::build(&root, &[global]);
        assert!(!machine.is_denied(&fake.home.join(".kube/config")));
        assert!(machine.is_denied(&fake.home.join(".netrc")));
        assert!(machine.is_denied(&fake.home.join(".git-credentials")));
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

    /// H1: the verdict does not depend on existence. This test used to assert
    /// the opposite — that `config/.env` passed because no such file existed,
    /// so creating one was allowed — and that `exists()` gate is the exact
    /// mechanism that let `~/.aws/credentials` through: it was asked of the
    /// unexpanded token, which never exists. `text_editor` has always refused
    /// to write a denied name; the scan now agrees with it.
    #[test]
    fn find_denied_does_not_depend_on_existence() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let g = guard_at(dir.path());

        let args = json!({ "path": ".env" });
        assert_eq!(
            g.find_denied_path(args.as_object().unwrap()),
            Some(".env".to_string())
        );

        let args = json!({ "path": "config/.env" });
        assert_eq!(
            g.find_denied_path(args.as_object().unwrap()),
            Some("config/.env".to_string()),
            "a denied name must be refused whether or not it exists yet"
        );
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
        let global = crate::paths::platform_dirs()
            .map(|dirs| dirs.config.join(".biorouterignore"))
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

    /// Each layer has to hold on its own. The literal token pass refuses
    /// several rows by itself — `cat ~/.aws/credentials`, taken whole as one
    /// path, ends in `.aws/credentials` — so the table above would stay green
    /// if the resolver regressed. This runs every row through the resolver
    /// alone.
    #[test]
    fn h1_the_resolver_alone_refuses_every_spelling() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let leaked: Vec<String> = h1_leaking_spellings(&fake.home)
            .into_iter()
            .filter(|(_, command)| {
                resolve::Resolver::new(&g, &env)
                    .scan_command(command, &[resolve::Base::Dir(fake.project.clone())])
                    .is_ok()
            })
            .map(|(name, command)| format!("  {name}: {command}"))
            .collect();
        assert!(
            leaked.is_empty(),
            "the resolver alone let {} spelling(s) through:\n{}",
            leaked.len(),
            leaked.join("\n")
        );
    }

    #[test]
    fn h1_refusal_messages_say_what_was_resolved_and_why() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let denied = g
            .find_denied_access_in(
                json!({ "command": "cd ~/.aws && head credentials" })
                    .as_object()
                    .unwrap(),
                &env,
            )
            .expect("refused");
        let message = denied.message();
        assert!(message.contains("'credentials'"), "{message}");
        // The resolved path renders with the OS separator, so compare
        // separator-agnostically — the guard matched `.aws/credentials`
        // whether it prints with `/` (unix) or `\` (Windows).
        assert!(
            message.replace('\\', "/").contains(".aws/credentials"),
            "{message}"
        );
        assert!(
            message.contains("secret/credential deny pattern"),
            "{message}"
        );
        assert!(message.contains("does not depend"), "{message}");

        let unresolved = g
            .find_denied_access_in(
                json!({ "command": "cat ~/.ssh/$(ls ~/.ssh | head -1)" })
                    .as_object()
                    .unwrap(),
                &env,
            )
            .expect("refused");
        assert!(
            unresolved
                .message()
                .contains("cannot be known until the command runs"),
            "{}",
            unresolved.message()
        );
    }

    /// W1-50 (p1 full-suite pass, 2026-11-16): a secret named inside a nested
    /// script was refused, but the refusal named the whole script as one path
    /// and joined it to the working directory: "Access to 'cat
    /// ~/.aws/credentials' (resolves to <project>/cat ~/.aws/credentials)".
    /// The script argument was judged as a path word before the nested parse
    /// ever read it. The refusal now names the path inside the script and what
    /// it resolves to, as the direct and here-document forms always did.
    #[test]
    fn h1_a_nested_script_refusal_names_the_inner_path() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let slashed = |path: &Path| path.to_string_lossy().replace('\\', "/");
        let secret = slashed(&fake.home.join(".aws/credentials"));
        let project = slashed(&fake.project);
        for command in [
            "bash -c 'cat ~/.aws/credentials'".to_string(),
            "eval 'cat ~/.aws/credentials'".to_string(),
            "eval \"c\"\"at ~/.aws/cred\"\"entials\"".to_string(),
            format!("find {project} -maxdepth 0 -exec sh -c 'cat ~/.aws/credentials' \\;"),
            "echo x | xargs -I{} sh -c 'cat ~/.aws/credentials'".to_string(),
        ] {
            let denied = g
                .find_denied_access_in(json!({ "command": command }).as_object().unwrap(), &env)
                .unwrap_or_else(|| panic!("{command} must be refused"));
            let message = denied.message();
            assert_eq!(denied.shown, "~/.aws/credentials", "{command}: {message}");
            assert_eq!(
                denied.resolved.as_deref().map(slashed),
                Some(secret.clone()),
                "{command}: {message}"
            );
            assert!(!message.contains("cat "), "{command}: {message}");
        }

        // `$HOME` inside double quotes is expanded by the outer shell, so the
        // script the inner one reads already names the absolute path.
        let denied = g
            .find_denied_access_in(
                json!({ "command": "sh -c \"cat $HOME/.aws/credentials\"" })
                    .as_object()
                    .unwrap(),
                &env,
            )
            .expect("refused");
        let message = denied.message();
        assert_eq!(slashed(Path::new(&denied.shown)), secret, "{message}");
        assert!(!message.contains("cat "), "{message}");
    }

    /// A value nobody can know, with nothing known around it, is not refused:
    /// refusing `cat "$(ls *.csv)"` would refuse every command substitution.
    /// That residue belongs to the tool-output scan.
    #[test]
    fn h1_an_entirely_unknown_argument_is_not_refused() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        for command in [
            "cat \"$(ls *.csv)\"",
            "cat $UNSET_FOR_TEST",
            "wc -l `cat list.txt`",
        ] {
            assert_eq!(scan_command(&g, &env, command), None, "{command}");
        }
        // …but a known end is still judged: `$X/.aws/credentials` ends in a secret.
        assert!(scan_command(&g, &env, "cat $UNSET_FOR_TEST/.aws/credentials").is_some());
        assert!(scan_command(&g, &env, "cat \"$(pwd)\"/.env").is_some());
    }

    #[test]
    fn h1_nesting_past_the_limit_fails_closed() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let mut command = "echo hi".to_string();
        for _ in 0..8 {
            command = format!("sh -c {}", shell_quote(&command));
        }
        assert!(
            scan_command(&g, &env, &command).is_some(),
            "a command nested past the limit cannot be verified and must be refused"
        );
        let mut shallow = "echo hi".to_string();
        for _ in 0..2 {
            shallow = format!("sh -c {}", shell_quote(&shallow));
        }
        assert_eq!(scan_command(&g, &env, &shallow), None);
    }

    fn shell_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }

    /// The same 8-deep nesting as above, asked of a **Windows** build.
    ///
    /// This is the fail-open `92dd1a42` introduced and the union closes.
    /// `shell_quote` splices quotes the POSIX way (`'\''`), which a
    /// Windows-mode lexer keeps as four literal characters — so under that
    /// reading the command is one flat `sh -c` with a long literal argument,
    /// never nesting, and `MAX_NESTING` is never reached. A Windows host gets
    /// POSIX command lines constantly (git-bash, WSL, MSYS, the coding-agent
    /// bridge), so binding the grammar to `cfg!(windows)` let an unverifiable
    /// command through on exactly the host the commit was written for.
    ///
    /// The first assertion pins the hole (the Windows reading alone does NOT
    /// refuse); the second is the fix (the union does). Revert the union and
    /// the second assertion fails — that is what makes it load-bearing rather
    /// than decorative.
    #[test]
    fn h1_windows_grammar_alone_misses_the_nesting_the_union_refuses() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let bases = [resolve::Base::Dir(fake.project.clone())];
        let mut command = "echo hi".to_string();
        for _ in 0..8 {
            command = format!("sh -c {}", shell_quote(&command));
        }

        // The bug, pinned: read only as Windows would, the nesting disappears.
        assert!(
            resolve::Resolver::new(&g, &env)
                .under_one_grammar_for_tests(true, &command, &bases)
                .is_ok(),
            "the Windows reading was expected to MISS this nesting; if it now \
             catches it, the union below is no longer what closes the hole and \
             this test needs rewriting rather than deleting"
        );

        // The fix: a Windows *host* judges it under both readings, and the
        // POSIX one reaches the depth limit.
        assert!(
            resolve::Resolver::new(&g, &env)
                .scan_command_as_host(&command, &bases, true)
                .is_err(),
            "FAIL-OPEN: a Windows build did not refuse an 8-deep POSIX-quoted \
             `sh -c` nesting it cannot verify"
        );

        // And a unix host is unaffected — one reading, still refused.
        assert!(resolve::Resolver::new(&g, &env)
            .scan_command_as_host(&command, &bases, false)
            .is_err());
    }

    /// The union's two short-circuits, which are what keep it from costing
    /// anything: a unix build never runs a second pass, and neither does a
    /// Windows build on an input with no `\` in it (both readings of such an
    /// input are the same token stream, because every site in `lex` that
    /// consults the flag sits inside a `'\\'` match arm).
    #[test]
    fn h1_the_second_grammar_runs_only_when_it_can_differ() {
        assert_eq!(
            resolve::grammars_for_tests("cat ~/.aws/credentials", false),
            1
        );
        assert_eq!(resolve::grammars_for_tests(r"cat c:\x", false), 1);
        assert_eq!(
            resolve::grammars_for_tests("cat ~/.aws/credentials", true),
            1
        );
        assert_eq!(resolve::grammars_for_tests(r"cat c:\x", true), 2);
    }

    /// `**` is found by a bounded walk, zsh-style: it does not descend into
    /// dot-directories, so it refuses exactly what the shell would reach.
    #[test]
    fn h1_recursive_globs_are_walked() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        assert_eq!(scan_command(&g, &env, "wc -l **/*.py"), None);
        fs::create_dir_all(fake.project.join("deep/er")).unwrap();
        fs::write(fake.project.join("deep/er/secrets.py"), "TOKEN = 1\n").unwrap();
        assert!(
            scan_command(&g, &env, "wc -l **/*.py").is_some(),
            "**/*.py reaches deep/er/secrets.py"
        );
    }

    #[test]
    fn is_denied_folds_case() {
        let dir = tempdir().unwrap();
        let g = SecretGuard::build(dir.path(), &[]);
        assert!(g.is_denied(Path::new("/h/.AWS/Credentials")));
        assert!(g.is_denied(Path::new("ID_RSA")));
        assert!(g.is_denied(Path::new("Secrets.YAML")));
        assert!(!g.is_denied(Path::new("Notes.TXT")));
    }

    #[test]
    #[cfg(unix)]
    fn is_denied_resolved_follows_a_symlink() {
        let fake = FakeHome::new().with_links();
        let g = fake.guard();
        assert!(!g.is_denied(&fake.project.join("creds-link")));
        assert!(g.is_denied_resolved(&fake.project.join("creds-link")));
        assert!(g.is_denied_resolved(&fake.project.join("aws-link/credentials")));
        assert!(!g.is_denied_resolved(&fake.project.join("data.csv")));
    }

    /// The scan runs on every tool call, so its cost has to stay flat: a long
    /// command, a large here-document and a big content field all finish well
    /// inside a budget no tool call would notice.
    ///
    /// **Two assertions, and only one of them is about time.** The wall clock is
    /// a *hang detector* and nothing more — it cannot tell "the resolver got
    /// twice as slow" from "the runner was busy", and reading it as a
    /// performance gate is how its ceiling went 5 s → 30 s in one commit while
    /// silently absorbing a real 2x growth in the workload. Measured 2026-09-11
    /// on an idle M-series Mac: this whole workload is **10.5 s in a debug
    /// build** (which is what CI runs: `rust.yml`'s `test` job is
    /// `cargo test --lib --bins`) and **651 ms in release** (what ships). So
    /// 30 s is ~2.9x an *idle* debug run, not headroom — treat it as the floor
    /// under a hang, and do not widen it again without re-measuring.
    ///
    /// The assertion that actually catches a regression is the second one: the
    /// whole workload against a typical single-command scan timed in the **same
    /// process**, which is immune to a slow or loaded runner. Measured ratios
    /// were 3200x (debug) and 1745x (release), so the 8000x bound leaves ~2.5x
    /// of room while still failing on an algorithmic change.
    #[test]
    fn h1_scan_cost_stays_bounded() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let long_command: String = (0..2000)
            .map(|i| format!("echo file{i}.txt;"))
            .collect::<Vec<_>>()
            .join(" ");
        let heredoc = format!(
            "cat > notes.md <<'EOF'\n{}\nEOF",
            "- see src/main.py and data/table.csv for the numbers\n".repeat(2000)
        );
        let content = "path/to/some/file.txt and another/one.md ".repeat(5000);
        let started = std::time::Instant::now();
        assert_eq!(scan_command(&g, &env, &long_command), None);
        assert_eq!(scan_command(&g, &env, &heredoc), None);
        assert_eq!(
            g.find_denied_path_in(json!({ "content": content }).as_object().unwrap(), &env),
            None
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(30),
            "scanning took {elapsed:?} — this ceiling is a hang detector, so \
             reaching it means an unbounded walk, not a slow runner"
        );

        // The machine-independent half: one ordinary command line, timed here,
        // is the unit the workload above is measured in.
        let unit_command =
            "cd src && grep -rn 'fn main' . | head -20 && python3 -c 'print(1)' > out.txt";
        let unit_started = std::time::Instant::now();
        const UNIT_RUNS: u32 = 20;
        for _ in 0..UNIT_RUNS {
            assert_eq!(scan_command(&g, &env, unit_command), None);
        }
        let unit = unit_started.elapsed() / UNIT_RUNS;
        const MAX_RATIO: u32 = 8000;
        assert!(
            elapsed < unit * MAX_RATIO,
            "the bounded workload cost {elapsed:?}, {} times one ordinary \
             command scan ({unit:?}); the bound is {MAX_RATIO}x and it is \
             machine-independent, so this is an algorithmic regression, not a \
             slow runner",
            elapsed.as_secs_f64() / unit.as_secs_f64()
        );
    }

    #[test]
    #[cfg(unix)]
    fn h1_a_symlink_into_a_secret_store_is_refused() {
        let fake = FakeHome::new().with_links();
        let (g, env) = (fake.guard(), fake.env());
        for command in [
            "cat aws-link/credentials",
            "cat creds-link",
            "cat aws-link/cred*",
        ] {
            assert!(
                scan_command(&g, &env, command).is_some(),
                "a link into the fake ~/.aws reached the secret: {command}"
            );
        }
    }

    /// `padding` links to a benign file, then one into a fake `.netrc`.
    #[cfg(unix)]
    fn padded_link_command(fake: &FakeHome, padding: usize) -> String {
        let links = fake.project.join("ln");
        fs::create_dir_all(&links).unwrap();
        fs::write(fake.project.join("benign.txt"), "hello\n").unwrap();
        fs::write(
            fake.home.join(".netrc"),
            "machine example.com login me password hunter2fake\n",
        )
        .unwrap();
        let mut words = vec!["cat".to_string()];
        for i in 0..padding {
            let name = format!("l{i}");
            if !links.join(&name).exists() {
                std::os::unix::fs::symlink(fake.project.join("benign.txt"), links.join(&name))
                    .unwrap();
            }
            words.push(format!("ln/{name}"));
        }
        if !links.join("zz").exists() {
            std::os::unix::fs::symlink(fake.home.join(".netrc"), links.join("zz")).unwrap();
        }
        words.push("ln/zz".to_string());
        words.join(" ")
    }

    /// SG-30 (rq2-040, rq2-043). The scan resolves at most 512 symlinks, and it
    /// used to answer "not denied" for every path after that: 700 benign links
    /// and then a link to `.netrc` read the password. A certain path the scan
    /// cannot verify inside its budget must be refused, as the docs say.
    #[test]
    #[cfg(unix)]
    fn h1_a_credential_link_past_the_symlink_budget_is_refused() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        // The instrument first: the link alone, and behind a few benign links,
        // is refused, so whatever the long command answers is the budget's
        // doing.
        let short = padded_link_command(&fake, 8);
        assert!(
            scan_command(&g, &env, "cat ln/zz").is_some(),
            "the credential link alone must be refused"
        );
        assert!(
            scan_command(&g, &env, &short).is_some(),
            "the credential link behind a few links must be refused"
        );
        let long = padded_link_command(&fake, 600);
        assert!(
            scan_command(&g, &env, &long).is_some(),
            "FAIL-OPEN: a link to a credential after more links than the scan \
             resolves was let through"
        );
    }

    /// The other half of the budget rule: past the budget, a plain file in a
    /// folder the scan has already resolved is still judged, from where that
    /// folder really is, so a long list of them is allowed and a credential
    /// reached through a linked folder seen earlier is still refused as the
    /// file it reaches (a pattern refusal, not a "too complex" one).
    #[test]
    fn h1_many_plain_files_past_the_budget_are_still_allowed() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let many = fake.project.join("many");
        fs::create_dir_all(&many).unwrap();
        let mut files = Vec::new();
        for i in 0..600 {
            fs::write(many.join(format!("f{i}.txt")), "x\n").unwrap();
            files.push(format!("many/f{i}.txt"));
        }
        let files = files.join(" ");
        assert_eq!(
            scan_command(&g, &env, &format!("cat {files}")),
            None,
            "600 plain files in one folder were refused"
        );

        #[cfg(unix)]
        {
            let fake = fake.with_links();
            let g = fake.guard();
            // `aws-link` is resolved (to the fake `~/.aws`) before the budget
            // runs out; `aws-link/credentials` is judged after it has.
            let command = format!("cat aws-link/config {files} aws-link/credentials");
            let denied = g
                .find_denied_access_in(json!({ "command": command }).as_object().unwrap(), &env)
                .expect("a credential reached through a linked folder past the budget");
            assert_eq!(
                denied.reason,
                resolve::Reason::Pattern,
                "it should be judged as the file it reaches: {}",
                denied.message()
            );
        }
    }

    /// The budget's shortcut skips a resolution only for a name the disk
    /// cannot spell differently in a way the deny set cares about; anything a
    /// case-insensitive or short-name filesystem could open under another
    /// spelling is resolved in full, so `realpath` still reports the real name.
    #[test]
    fn h1_only_names_the_disk_cannot_respell_skip_a_resolution() {
        for stable in ["f0.txt", "my file (1).csv", "data-v2_final.tsv", ".bashrc"] {
            assert!(resolve::is_stable_name_for_tests(stable), "{stable}");
        }
        for unstable in [
            "credential\u{17f}",
            "CREDEN~1",
            "credentials.",
            "credentials ",
            "credentials:stream",
            "caf\u{e9}.txt",
            "",
        ] {
            assert!(!resolve::is_stable_name_for_tests(unstable), "{unstable:?}");
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
        assert!(g
            .find_denied_path_in(args.as_object().unwrap(), &env)
            .is_some());
    }

    /// External scripting tools can carry executable bodies under `script`.
    #[test]
    fn h1_external_script_arguments_are_scanned_like_a_command() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let args = json!({ "language": "shell", "script": "cd ~/.aws\nhead -5 credentials\n" });
        assert!(g
            .find_denied_path_in(args.as_object().unwrap(), &env)
            .is_some());
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

    fn issue546_assert_heredoc_denied(fake: &FakeHome, command: &str) {
        let (g, env) = (fake.guard(), fake.env());
        assert!(
            scan_command(&g, &env, command).is_some(),
            "the command guard allowed protected here-document input: {command}"
        );
        assert!(
            resolve::Resolver::new(&g, &env)
                .scan_command(command, &[resolve::Base::Dir(fake.project.clone())])
                .is_err(),
            "the resolver alone allowed protected here-document input: {command}"
        );
    }

    #[test]
    fn issue546_quoted_r_data_is_not_a_shell_path() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        for delimiter in ["'EOF'", "\"EOF\"", "\\EOF", "'E'OF"] {
            let command = format!(
                "cat > summarize.R <<{delimiter}\nres <- data.frame(IHS = c(1, Inf))\nres$IHS[is.finite(res$IHS)]\nEOF\n"
            );
            assert_eq!(scan_command(&g, &env, &command), None, "{command}");
        }
    }

    #[test]
    fn issue546_backquoted_markdown_data_is_not_shell_execution() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let command =
            "cat > notes.md <<'EOF'\nUse `res$IHS[is.finite(res$IHS)]` when filtering.\nEOF\n";
        assert_eq!(scan_command(&g, &env, command), None);
    }

    #[test]
    fn issue546_six_hundred_punctuated_prose_words_are_allowed() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let body = (0..600)
            .map(|index| format!("sentence{index}."))
            .collect::<Vec<_>>()
            .join(" ");
        let command = format!("cat > notes.md <<'EOF'\n{body}\nEOF\n");
        assert_eq!(scan_command(&g, &env, &command), None);
    }

    #[test]
    fn issue546_unknown_stdin_receivers_keep_the_secret_floor() {
        let fake = FakeHome::new();
        for receiver in ["unclassified_reader", "at now", "batch", "crontab -"] {
            issue546_assert_heredoc_denied(
                &fake,
                &format!("{receiver} <<'EOF'\ncd ~/.aws\ncat credentials\nEOF\n"),
            );
        }
    }

    #[test]
    fn issue546_executable_and_filename_stdin_keep_the_secret_floor() {
        let fake = FakeHome::new();
        for command in [
            "bash <<'EOF'\ncd ~/.aws\ncat credentials\nEOF\n",
            "fish <<'EOF'\ncd ~/.aws; cat credentials\nEOF\n",
            "cat <<'EOF' | sh\ncd ~/.aws\ncat credentials\nEOF\n",
            "cd ~/.aws && xargs cat <<'EOF'\ncredentials\nEOF\n",
            "cd ~/.aws && while read f; do cat \"$f\"; done <<'EOF'\ncredentials\nEOF\n",
            "cat > x.sh <<'EOF'\ncd ~/.aws\ncat credentials\nEOF\nbash x.sh\n",
            "xargs sh -c <<'EOF'\ncd ~/.aws && cat credentials\nEOF\n",
            "docker exec -i fixture sh <<'EOF'\ncd ~/.aws && cat credentials\nEOF\n",
            "python3 - <<'EOF'\nimport os\nos.system('cat ~/.aws/credentials')\nEOF\n",
            "tar -T - <<'EOF'\n$HOME/.aws/credentials\nEOF\n",
            "cat > notes.md <<EOF\n$(cat ~/.aws/credentials)\nEOF\n",
        ] {
            issue546_assert_heredoc_denied(&fake, command);
        }
    }

    #[test]
    fn issue546_literal_and_obfuscated_body_paths_keep_the_secret_floor() {
        let fake = FakeHome::new();
        for command in [
            "cat > notes.md <<'EOF'\n~/.aws/credentials\nEOF\n",
            "cat > notes.md <<'EOF'\ncd ~/.aws\ncat cred\"\"entials\nEOF\n",
            "cat > notes.md <<'EOF'\ncd ~/.aws\ncat $'cred\\x65ntials'\nEOF\n",
            "cat > notes.md <<'EOF'\ncd ~/.aws\ncat cred$(printf e)ntials\nEOF\n",
        ] {
            issue546_assert_heredoc_denied(&fake, command);
        }
    }

    #[test]
    fn issue546_body_handling_does_not_hide_a_later_outer_access() {
        let fake = FakeHome::new();
        issue546_assert_heredoc_denied(
            &fake,
            "cat > notes.md <<'EOF'\nordinary prose\nEOF\ncd ~/.aws && cat credentials\n",
        );
    }

    #[test]
    #[cfg(unix)]
    fn issue546_quoted_data_keeps_symlink_target_denial() {
        let fake = FakeHome::new().with_links();
        let g = fake.guard();
        assert!(!g.is_denied(&fake.project.join("creds-link")));
        for command in [
            "cat > notes.md <<'EOF'\ncat creds-link\nEOF\n",
            "cat > notes.md <<'EOF'\ncat aws-link/credentials\nEOF\n",
        ] {
            issue546_assert_heredoc_denied(&fake, command);
        }
    }

    #[test]
    #[cfg(unix)]
    fn issue546_quoted_body_link_past_the_budget_is_refused() {
        let fake = FakeHome::new();
        for padding in [8, 600] {
            let body = padded_link_command(&fake, padding);
            let g = fake.guard();
            assert!(!g.is_denied(&fake.project.join("ln/zz")));
            issue546_assert_heredoc_denied(&fake, "cat ln/zz");
            issue546_assert_heredoc_denied(
                &fake,
                &format!("cat > notes.md <<'EOF'\n{body}\nEOF\n"),
            );
        }
    }

    /// The R script of `issue546_quoted_r_data_is_not_a_shell_path`. Its
    /// `res$IHS[…]` is refused whenever the body is read as a script this
    /// command runs, which makes it a probe for whether a body was classified
    /// as data.
    const ISSUE546_R_BODY: &str =
        "res <- data.frame(IHS = c(1, Inf))\nres$IHS[is.finite(res$IHS)]\n";

    /// Item 5 of #546: a refusal for a word inside a here-document body says
    /// so, and says how the body was read, so a model that wrote the body as
    /// text is not left looking for the word among its commands.
    #[test]
    fn issue546_a_refusal_from_a_body_says_it_came_from_the_body() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let message = |command: &str| {
            let args = json!({ "command": command });
            let from_dispatch = g
                .find_denied_access_in(args.as_object().unwrap(), &env)
                .unwrap_or_else(|| panic!("allowed: {command:?}"))
                .message();
            let from_shell = g
                .find_denied_in_command(command, &fake.project, &env)
                .unwrap_or_else(|| panic!("the shell's own check allowed: {command:?}"))
                .message();
            assert_eq!(from_dispatch, from_shell, "{command:?}");
            from_dispatch
        };
        let r = ISSUE546_R_BODY;

        // Read as a script: another command in the input could run the file.
        let script = message(&format!("cat > a.R <<'EOF'\n{r}EOF\nRscript a.R\n"));
        assert!(script.contains("here-document body"), "{script}");
        assert!(script.contains("reads as a script"), "{script}");
        assert!(script.contains("`cat > FILE <<'EOF'`"), "{script}");
        let unquoted = message(&format!("cat > a.R <<EOF\n{r}EOF\n"));
        assert!(unquoted.contains("reads as a script"), "{unquoted}");

        // Read as file content, and still refused for a protected path.
        let data = message("cat > notes.md <<'EOF'\n~/.aws/credentials\nEOF\n");
        assert!(
            data.contains("here-document body written to a file"),
            "{data}"
        );
        assert!(!data.contains("reads as a script"), "{data}");

        // A word outside every body keeps the plain message.
        for command in [
            "cat ~/.aws/credentials",
            "cat > notes.md <<'EOF'\nordinary prose\nEOF\ncd ~/.aws && cat credentials\n",
        ] {
            let outside = message(command);
            assert!(!outside.contains("here-document"), "{command:?}: {outside}");
        }
    }

    /// A Windows host reads the input under both grammars, and the Windows
    /// reading keeps `\` literal. The delimiter is still read the POSIX way, so
    /// `<<\EOF` ends at `EOF` and the body is data under both readings; and the
    /// grammar comes back for the words after it.
    #[test]
    fn issue546_a_windows_host_reads_every_quoted_delimiter_as_data() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let bases = [resolve::Base::Dir(fake.project.clone())];
        for delimiter in ["'EOF'", "\"EOF\"", "\\EOF", "'E'OF"] {
            for target in ["summarize.R", r"out\summarize.R"] {
                let command = format!("cat > {target} <<{delimiter}\n{ISSUE546_R_BODY}EOF\n");
                for windows_host in [true, false] {
                    assert!(
                        resolve::Resolver::new(&g, &env)
                            .scan_command_as_host(&command, &bases, windows_host)
                            .is_ok(),
                        "{command:?} (windows host: {windows_host})"
                    );
                }
                assert!(
                    resolve::Resolver::new(&g, &env)
                        .under_one_grammar_for_tests(true, &command, &bases)
                        .is_ok(),
                    "the Windows reading alone refused {command:?}"
                );
            }
        }
        // The floor still holds in that body on a Windows host.
        for body in [
            "cd ~/.aws\ncat cred\"\"entials\n",
            "cat ~/.aws/credentials\n",
            "cd ~/.aws\ncat cred$(printf e)ntials\n",
        ] {
            let command = format!("cat > notes.md <<\\EOF\n{body}EOF\n");
            assert!(
                resolve::Resolver::new(&g, &env)
                    .scan_command_as_host(&command, &bases, true)
                    .is_err(),
                "{command:?}"
            );
        }
    }

    /// Only `cat > FILE <<'DELIM'`, alone or beside more of the same, is data.
    /// Each form below changes one thing, and each puts the R body back under
    /// the script reading that refuses it.
    #[test]
    fn issue546_only_a_plain_file_write_is_data() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        let r = ISSUE546_R_BODY;
        for command in [
            format!("cat > a.R <<'EOF'\n{r}EOF\n"),
            format!("cat <<'EOF' > a.R\n{r}EOF\n"),
            format!("> a.R cat <<'EOF'\n{r}EOF\n"),
            format!("cat >> a.R <<-'EOF'\n\t{r}\tEOF\n"),
            format!("cat >| a.R <<'EOF'\n{r}EOF\n"),
            format!("cat > a.R <<'EOF'\n{r}EOF\ncat > b.R <<'EOF'\n{r}EOF\n"),
            format!("cat > a.R <<'A' && cat > b.md <<\"B\"\n{r}A\n{r}B\n"),
        ] {
            assert_eq!(scan_command(&g, &env, &command), None, "{command}");
        }
        for command in [
            // Something else in the same input could run what was written.
            format!("cat > a.R <<'EOF'\n{r}EOF\nRscript a.R\n"),
            format!("mkdir -p out && cat > a.R <<'EOF'\n{r}EOF\n"),
            format!("cat > a.R <<'EOF'\n{r}EOF\nbash a.R\n"),
            // The output goes somewhere that may run it.
            format!("cat <<'EOF' | tee a.R\n{r}EOF\n"),
            format!("cat > a.R <<'EOF' | sh\n{r}EOF\n"),
            format!("(cat > a.R <<'EOF'\n{r}EOF\n)"),
            format!("cat <<'EOF'\n{r}EOF\n"),
            format!("cat 2>&1 <<'EOF'\n{r}EOF\n"),
            // Not bare `cat`, or not only one file.
            format!("sudo cat > a.R <<'EOF'\n{r}EOF\n"),
            format!("X=1 cat > a.R <<'EOF'\n{r}EOF\n"),
            format!("\\cat > a.R <<'EOF'\n{r}EOF\n"),
            format!("cat -v > a.R <<'EOF'\n{r}EOF\n"),
            format!("cat - > a.R <<'EOF'\n{r}EOF\n"),
            format!("cat > a.R > b.R <<'EOF'\n{r}EOF\n"),
            format!("cat > \"$OUT\" <<'EOF'\n{r}EOF\n"),
            format!("cat > a.R <<'EOF' <<'EOG'\n{r}EOF\n{r}EOG\n"),
            // A delimiter that does not make the body plain text everywhere.
            format!("cat > a.R <<EOF\n{r}EOF\n"),
            format!("cat > a.R <<$'EOF'\n{r}EOF\n"),
            format!("cat > a.R <<'EOF'\n{r}"),
            format!("cat > a.R <<'EOF'\n{r}EOF\r\n"),
        ] {
            assert!(
                scan_command(&g, &env, &command).is_some(),
                "a form that is not a plain file write was read as data: {command:?}"
            );
        }
    }

    /// The lexer reads `\r` as a blank and the shell reads it as part of a
    /// word, so after `<<'EOF'\r` the shell's delimiter is `EOF\r`. Its body
    /// then ends at the first `EOF\r` line, and what this lexer would call the
    /// rest of the body runs as commands. Such a body is never data.
    #[test]
    fn issue546_a_carriage_return_after_the_delimiter_is_never_data() {
        let fake = FakeHome::new();
        issue546_assert_heredoc_denied(
            &fake,
            "cat > a.md <<'EOF'\r\nEOF\r\ncd \"$(echo ~/.aws)\"\ncat credentials\nEOF\n",
        );
    }

    /// The secret floor reads every word of a data body, whatever program the
    /// line starts with, so a link or a spelling trick is refused in any
    /// position.
    #[test]
    #[cfg(unix)]
    fn issue546_aliases_in_any_position_of_a_data_body_are_refused() {
        let fake = FakeHome::new().with_links();
        for body in [
            "head creds-link",
            "X=1 cat creds-link",
            "unknownprefix aws-link/credentials",
            "creds-link",
            "less aws-link/cred*",
            "head ~/.a\"ws\"/credentials",
            "printf '%s' \"$(cat creds-link)\"",
            "Use `cat creds-link` to check.",
        ] {
            issue546_assert_heredoc_denied(
                &fake,
                &format!("cat > notes.md <<'EOF'\nprose first\n{body}\nEOF\n"),
            );
        }
    }

    /// Prose stops spending the symlink budget, and nothing else does: a link
    /// in a data body after more real files than the budget covers, or after
    /// more path-looking words than the code pass reads, is still refused, and
    /// so is an unstable name that the disk answers with anything but "not
    /// found".
    #[test]
    #[cfg(unix)]
    fn issue546_a_data_body_keeps_the_budget_for_real_files_and_links() {
        let fake = FakeHome::new().with_links();
        let (g, env) = (fake.guard(), fake.env());
        let many = fake.project.join("many");
        fs::create_dir_all(&many).unwrap();
        let mut files = Vec::new();
        for i in 0..600 {
            fs::write(many.join(format!("f{i}.txt")), "x\n").unwrap();
            files.push(format!("many/f{i}.txt"));
        }
        // The control: the same body without the link is allowed.
        let files = files.join(" ");
        let plain = format!("cat > notes.md <<'EOF'\nreader {files}\nEOF\n");
        assert_eq!(scan_command(&g, &env, &plain), None);
        // 600 real files, then a link.
        issue546_assert_heredoc_denied(
            &fake,
            &format!("cat > notes.md <<'EOF'\nreader {files} creds-link\nEOF\n"),
        );
        // More than 2048 path-looking words, then a link.
        let long: Vec<String> = (0..2100)
            .map(|i| format!("many/f{}.txt", i % 600))
            .collect();
        issue546_assert_heredoc_denied(
            &fake,
            &format!(
                "cat > notes.md <<'EOF'\n{}\nreader creds-link\nEOF\n",
                long.join("\n")
            ),
        );
        // `data.csv/x.` is "not a directory", not "not found": resolved, and
        // past the budget an unstable name is unverifiable.
        let words: Vec<String> = (0..600).map(|i| format!("data.csv/x{i}.")).collect();
        issue546_assert_heredoc_denied(
            &fake,
            &format!("cat > notes.md <<'EOF'\n{}\nEOF\n", words.join(" ")),
        );
    }

    /// A zsh `(D)` in a data body turns dot-file globbing on for that body,
    /// as it would if the body ran, and for nothing else in the command.
    #[test]
    fn issue546_dot_globbing_stays_inside_its_data_body() {
        let fake = FakeHome::new();
        fs::write(fake.project.join(".env"), "TOKEN=fixture\n").unwrap();
        let (g, env) = (fake.guard(), fake.env());
        assert!(scan_command(&g, &env, "cat > a.zsh <<'EOF'\nls *(D)\nEOF\n").is_some());
        assert!(scan_command(
            &g,
            &env,
            "cat > a.sh <<'EOF'\nshopt -s dotglob\ncat *\nEOF\n"
        )
        .is_some());
        for command in [
            "cat > a.md <<'EOF'\nGrade (D) students\nEOF\ncat > b.md <<'EOF'\n* item\nEOF\n",
            "cat > a.sh <<'EOF'\nshopt -s dotglob\nEOF\ncat > b.md <<'EOF'\n* item\nEOF\n",
        ] {
            assert_eq!(scan_command(&g, &env, command), None, "{command}");
        }
    }

    /// A data body is one level deeper, like any nested script, so nesting
    /// past the limit inside it is still refused.
    #[test]
    fn issue546_nesting_inside_a_data_body_keeps_the_limit() {
        let fake = FakeHome::new();
        let (g, env) = (fake.guard(), fake.env());
        // The body is one level, so four `sh -c` inside it is five: past the
        // limit of four, exactly as it was before data bodies existed.
        let mut deep = "echo hi".to_string();
        for _ in 0..4 {
            deep = format!("sh -c {}", shell_quote(&deep));
        }
        issue546_assert_heredoc_denied(&fake, &format!("cat > x.sh <<'EOF'\n{deep}\nEOF\n"));
        let mut shallow = "echo hi".to_string();
        for _ in 0..3 {
            shallow = format!("sh -c {}", shell_quote(&shallow));
        }
        assert_eq!(
            scan_command(&g, &env, &format!("cat > x.sh <<'EOF'\n{shallow}\nEOF\n")),
            None
        );
    }

    /// Code read from stdin names files by strings with no `/` in them. A
    /// string that is a protected path is refused when the body is a script
    /// or code; a data body that only mentions one reads nothing.
    #[test]
    fn issue546_code_from_stdin_is_judged_by_its_strings() {
        let fake = FakeHome::new();
        fs::write(fake.project.join(".env"), "TOKEN=fixture\n").unwrap();
        let (g, env) = (fake.guard(), fake.env());
        for command in [
            "python3 - <<'EOF'\nprint(open('.env').read())\nEOF\n",
            "python - <<'EOF'\nprint(open(\".env\").read())\nEOF\n",
            "node <<'EOF'\nrequire('fs').readFileSync(\"id_ed25519\")\nEOF\n",
            "cat <<'EOF' | python3\nopen('secrets.yaml')\nEOF\n",
        ] {
            issue546_assert_heredoc_denied(&fake, command);
        }
        for command in [
            "python3 - <<'EOF'\nprint('hello, world')\nprint(open(\"data.csv\").read())\nEOF\n",
            "cat > load.py <<'EOF'\nprint(open('.env').read())\nEOF\n",
        ] {
            assert_eq!(scan_command(&g, &env, command), None, "{command}");
        }
    }
}

// External candidate: append to secret_guard.rs after the existing test modules.
#[cfg(test)]
mod issue522_guard_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn issue522_conventional_env_templates_are_allowed_by_both_builders() {
        use biorouter_sandbox::bounded_fs::{AccessMode, BoundedFs};

        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("config")).unwrap();
        let names = [
            ".env.example",
            ".env.sample",
            ".env.template",
            ".env.dist",
            ".env.tpl",
            ".env.defaults",
            ".env.production.example",
        ];
        for name in names {
            std::fs::write(root.path().join("config").join(name), "REGION=\n").unwrap();
        }
        let bounded = BoundedFs::open(root.path()).unwrap();
        let path = bounded
            .bind("config/.env.example", AccessMode::Read)
            .unwrap();
        let guards = [
            SecretGuard::for_dir(root.path()),
            SecretGuard::for_bounded_path(&path).unwrap(),
        ];
        for guard in guards {
            for name in names {
                let relative = format!("config/{name}");
                assert!(!guard.is_denied(Path::new(&relative)), "{relative}");
                assert!(
                    guard
                        .find_denied_access(json!({"path": relative}).as_object().unwrap())
                        .is_none(),
                    "{name}"
                );
            }
        }
    }

    /// The generic scan does not know which tool a key belongs to, and `code`
    /// under an unknown tool can be a program that opens the path (Compute's
    /// `compute_python`, an external runner). So a slash path in a prose key
    /// stays refused here; only the dispatch boundary, which knows the call is
    /// the bundled Todo or Code Execution tool, leaves those fields out
    /// (`agents::extension_manager`'s `issue522_` tests). A template and an
    /// ordinary file in the same prose stay readable.
    #[test]
    fn issue522_generic_prose_keys_keep_the_floor_for_slash_paths() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        for key in [
            "text",
            "content",
            "message",
            "title",
            "description",
            "commit_message",
            "code",
        ] {
            let protected = serde_json::Map::from_iter([(
                key.to_string(),
                json!("configuration/.env copy still needs the user"),
            )]);
            let denied = guard
                .find_denied_access(&protected)
                .unwrap_or_else(|| panic!("prose key {key} let a protected path through"));
            assert_eq!(denied.shown, "configuration/.env", "{key}");
            assert!(
                denied.message().contains("secret/credential deny pattern"),
                "{key}: {}",
                denied.message()
            );
            for benign in [
                "configuration/notes.md copy still needs the user",
                "the user still copies configuration/.env.example",
            ] {
                let args = serde_json::Map::from_iter([(key.to_string(), json!(benign))]);
                assert!(
                    guard.find_denied_access(&args).is_none(),
                    "prose key {key} refused {benign:?}"
                );
            }
        }
        let denied = guard
            .find_denied_access(
                json!({"updates": [{"id": "1", "text": "configuration/.env needs the user"}]})
                    .as_object()
                    .unwrap(),
            )
            .expect("a nested prose key keeps the floor");
        assert_eq!(denied.shown, "configuration/.env");
        assert!(guard
            .find_denied_access(
                json!({"updates": [{"id": "1", "text": "configuration/notes.md needs the user"}]})
                    .as_object()
                    .unwrap()
            )
            .is_none());
    }

    #[test]
    fn issue522_copy_template_reports_the_protected_destination() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        let denied = guard
            .find_denied_access(
                json!({"command": "cp config/.env.example config/.env"})
                    .as_object()
                    .unwrap(),
            )
            .expect("the .env destination stays protected");
        let shown = denied.shown.replace('\\', "/");
        assert!(
            shown.ends_with("config/.env"),
            "wrong operand refused: {shown}"
        );
        assert!(
            !shown.contains(".env.example"),
            "template source was refused: {shown}"
        );
    }

    #[test]
    fn issue522_typed_paths_and_resolved_commands_keep_the_secret_floor() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        let env = ShellEnv::with_vars([("HOME", root.path().to_string_lossy().into_owned())]);
        for args in [
            json!({"path": "config/.env"}),
            json!({"file_path": "config/.env.local"}),
            json!({"resource": "config/.env.production"}),
            json!({"path": ".env.example/id_rsa"}),
            json!({"command": "cat 'config/.env'"}),
            json!({"shell_command": "cat config/.e\"\"nv"}),
            json!({"command": "cat config/$'.e\\x6ev'"}),
            json!({"command": "sh -c 'cat config/.env'"}),
            json!({"command": "cat \"$HOME/.aws/credentials\""}),
            json!({"command": "cat ~/.ssh/id_ed25519"}),
            json!({"command": "cat keys/server.pem"}),
            json!({"command": "cp config/.env.template config/.env.production"}),
        ] {
            assert!(
                guard
                    .find_denied_access_in(args.as_object().unwrap(), &env)
                    .is_some(),
                "protected access passed: {args}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn issue522_template_alias_cannot_reopen_a_real_secret() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".env"), "SYNTHETIC=only\n").unwrap();
        std::fs::write(root.path().join("id_rsa"), "synthetic private fixture\n").unwrap();
        for (template, target) in [(".env.example", ".env"), (".env.sample", "id_rsa")] {
            std::os::unix::fs::symlink(root.path().join(target), root.path().join(template))
                .unwrap();
        }
        let guard = SecretGuard::for_dir(root.path());
        for template in [".env.example", ".env.sample"] {
            for key in ["path", "resource"] {
                let args = serde_json::Map::from_iter([(key.to_string(), json!(template))]);
                assert!(
                    guard.find_denied_access(&args).is_some(),
                    "template alias reopened {template} through {key}"
                );
            }
        }
    }

    #[test]
    fn issue522_explicit_project_policy_can_still_deny_a_template() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".biorouterignore"), ".env.example\n").unwrap();
        use biorouter_sandbox::bounded_fs::{AccessMode, BoundedFs};

        std::fs::write(root.path().join(".env.example"), "REGION=\n").unwrap();
        let bounded = BoundedFs::open(root.path()).unwrap();
        let path = bounded.bind(".env.example", AccessMode::Read).unwrap();
        for guard in [
            SecretGuard::for_dir(root.path()),
            SecretGuard::for_bounded_path(&path).unwrap(),
        ] {
            assert!(guard.is_denied(Path::new(".env.example")));
            assert!(guard.is_denied(Path::new(".env")));
            assert!(guard.is_denied(Path::new("keys/server.pem")));
        }
    }
}

#[cfg(test)]
mod issue522_unclassified_controls {
    use super::*;
    use serde_json::json;

    #[test]
    fn issue522_unclassified_fields_and_command_aliases_keep_literal_refusals() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        for args in [
            json!({"resource": "configuration/.env"}),
            json!({"unknown_argument": "configuration/.env"}),
            json!({"shell_script": "cat configuration/.env"}),
            json!({"text": "ordinary note", "nested": {"path": "configuration/.env"}}),
            json!({"code": "// ordinary source", "command": "cat configuration/.env"}),
        ] {
            assert!(guard
                .find_denied_access(args.as_object().unwrap())
                .is_some());
        }
    }
}

/// Issue #522 through the resolver: a template is reopened wherever the guard
/// can see its name, and nothing it shares a prefix with is.
#[cfg(test)]
mod issue522_template_resolution {
    use super::*;
    use serde_json::json;

    fn command_denial(guard: &SecretGuard, env: &ShellEnv, command: &str) -> Option<String> {
        guard
            .find_denied_access_in(json!({ "command": command }).as_object().unwrap(), env)
            .map(|denied| denied.shown)
    }

    #[test]
    fn issue522_template_with_an_unknown_root_is_readable_and_its_neighbours_are_not() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        let env = ShellEnv::with_vars([("HOME", root.path().to_string_lossy().into_owned())]);
        for command in [
            "cat \"$(git rev-parse --show-toplevel)/config/.env.example\"",
            "cp \"$UNSET_ROOT/config/.env.sample\" config/settings.txt",
            "cat \"$(pwd)/.env.production.example\"",
            "cat \"$(pwd)/.ENV.EXAMPLE\"",
        ] {
            assert_eq!(command_denial(&guard, &env, command), None, "{command}");
        }
        for command in [
            "cat \"$(git rev-parse --show-toplevel)/config/.env\"",
            "cat \"$(git rev-parse --show-toplevel)/config/.env.local\"",
            "cat \"$UNSET_ROOT/.env.production\"",
            "cat \"$UNSET_ROOT/.env.example.bak\"",
            "cat \"$UNSET_ROOT/.env.Example\"",
            "cat \"$UNSET_ROOT/.env.example/id_rsa\"",
        ] {
            assert!(command_denial(&guard, &env, command).is_some(), "{command}");
        }
    }

    /// The literal pass used to take a whole command line as one path, so the
    /// text after its last `/` (`.env.example | head -5`) matched `**/.env.*`.
    #[test]
    fn issue522_a_template_followed_by_more_words_is_readable() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        let env = ShellEnv::with_vars([("HOME", root.path().to_string_lossy().into_owned())]);
        for command in [
            "cat config/.env.example | head -5",
            "cp config/.env.example .",
            "grep -n REGION config/.env.example && echo done",
        ] {
            assert_eq!(command_denial(&guard, &env, command), None, "{command}");
            assert_eq!(
                guard
                    .find_denied_in_command(command, root.path(), &env)
                    .map(|denied| denied.shown),
                None,
                "{command}"
            );
        }
        for command in [
            "cat config/.env | head -5",
            "cp config/.env.local .",
            "grep -n REGION config/.env.example config/.env.production && echo done",
        ] {
            assert!(command_denial(&guard, &env, command).is_some(), "{command}");
            assert!(
                guard
                    .find_denied_in_command(command, root.path(), &env)
                    .is_some(),
                "{command}"
            );
        }
    }

    /// Only a command line loses the whole-string candidate: an unknown tool
    /// may take a path with spaces under a key that does not look like one.
    #[test]
    fn issue522_prose_keeps_the_whole_value_as_one_candidate() {
        let root = tempfile::tempdir().unwrap();
        let guard = SecretGuard::for_dir(root.path());
        for value in ["/keys/My Server.pem", "config/.env.example copy"] {
            let args = serde_json::Map::from_iter([("attachment".to_string(), json!(value))]);
            assert!(guard.find_denied_access(&args).is_some(), "{value}");
        }
    }

    #[test]
    fn issue522_a_project_deny_still_reaches_a_template_with_an_unknown_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".biorouterignore"), ".env.example\n").unwrap();
        let guard = SecretGuard::for_dir(root.path());
        let env = ShellEnv::with_vars([("HOME", root.path().to_string_lossy().into_owned())]);
        assert!(command_denial(
            &guard,
            &env,
            "cat \"$(git rev-parse --show-toplevel)/config/.env.example\""
        )
        .is_some());
    }

    #[test]
    fn issue522_globs_open_a_template_and_never_the_secret_beside_it() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        std::fs::create_dir(&config).unwrap();
        std::fs::write(config.join(".env.example"), "REGION=\n").unwrap();
        let guard = SecretGuard::for_dir(root.path());
        let env = ShellEnv::with_vars([("HOME", root.path().to_string_lossy().into_owned())]);
        assert_eq!(command_denial(&guard, &env, "cat config/.e*"), None);
        assert_eq!(
            command_denial(&guard, &env, "cat config/.[e]nv.example"),
            None
        );

        std::fs::write(config.join(".env.local"), "SYNTHETIC=only\n").unwrap();
        assert!(command_denial(&guard, &env, "cat config/.e*").is_some());
        assert_eq!(
            command_denial(&guard, &env, "cat config/.[e]nv.example"),
            None
        );

        std::fs::write(config.join(".env"), "SYNTHETIC=only\n").unwrap();
        assert!(command_denial(&guard, &env, "cat config/.[e]nv").is_some());
        assert!(command_denial(&guard, &env, "cat config/.EN?").is_some());
        assert!(command_denial(&guard, &env, "cat CONFIG/.ENV").is_some());
    }
}
