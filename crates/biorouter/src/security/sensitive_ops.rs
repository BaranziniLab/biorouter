//! Fully-Automatic-mode sensitive-operation gate (owner Directive 2).
//!
//! # Policy (an explicit product decision — the single auditable list lives here)
//!
//! In [`BioRouterMode::Auto`] ("Fully Automatic") the agent and its extensions
//! may **create, read, edit and delete files anywhere on the user's machine
//! WITHOUT a prompt**. The one exception is a small, explicit set of *extremely
//! sensitive system operations*, which are routed through the **same** approval
//! flow every other prompt already uses — an [`InspectionAction::RequireApproval`]
//! that the agent surfaces with the confirmation TTL in
//! [`crate::agents::tool_execution`]. This is an **ask**, never a new hard block.
//!
//! What this gate does NOT change:
//!   * The always-on catastrophic-command denylist (BR-20) and the auditable
//!     command policy engine (BR-21) remain the floor — a `rm -rf /` is still a
//!     non-bypassable *deny*, not merely an ask, in every mode.
//!   * `.biorouterignore` / `SecretGuard` stays **absolute and mode-independent**:
//!     it denies reads/writes of user-declared secrets at the extension-manager
//!     dispatch boundary, and this gate never relaxes it.
//!   * Every mode other than `Auto`. The inspector is **inert** outside `Auto`
//!     (returns no results), because those modes already gate these operations:
//!     `Approve` prompts for everything, `SmartApprove` grades a write as
//!     destructive, and `Chat` runs no tools. Non-Auto behaviour is therefore
//!     provably unchanged — the whole gate is one early return away from a no-op.
//!
//! # What counts as "extremely sensitive" (the single list)
//!
//! A **mutating** file operation (create / write / edit / delete / move-onto —
//! never a read/`view`) whose target path is any of:
//!   1. the filesystem root or a whole volume ([`Blast::Root`] /
//!      [`Blast::WildcardAtRoot`]);
//!   2. the bare home directory itself ([`Blast::HomeBare`]);
//!   3. a protected system directory ([`Blast::SystemDir`]) — `/System`, `/usr`,
//!      `/bin`, `/sbin`, `/etc`, `/Library`, `/boot`, a Windows system root, …
//!      **except** the OS temp trees (`/tmp`, `/var/folders/**`, `$TMPDIR`),
//!      which live under a system dir but are ordinary scratch space;
//!   4. a credential / persistence location under `$HOME` that classifies as
//!      *ordinary* for the blast rule but is a known secret store — SSH keys,
//!      GPG/AWS/gcloud/kube/docker creds, macOS keychains, launchd agents, and
//!      browser credential stores (see [`SENSITIVE_HOME_SUBPATHS`]).
//!
//! …plus one criterion that is not about *where* the target is at all:
//!   5. the **recursive deletion of an established directory** — an `rm -r`
//!      (or `Remove-Item -Recurse`, or `rd /s`) of a path that is an existing,
//!      non-empty directory this session did not create. Criteria 1-4 measure
//!      blast radius by location and are silent on volume, but `rm -rf` on a
//!      populated tree is a different risk class from writing one file:
//!      `~/Desktop/kdps-build` classifies as [`Blast::Ordinary`], and a real
//!      Auto-mode agent destroyed an unpushed git repository there with no
//!      prompt of any kind. See [`established_directory_probe`] for what makes
//!      a directory "established", and note what is deliberately NOT escalated:
//!      an empty directory, a plain file, a symlink, a temp tree, a regenerable
//!      build/dependency/cache directory ([`REGENERABLE_DIR_NAMES`]), a tree
//!      whose whole content post-dates the session, and anything the session is
//!      on record as having created. A prompt on every `rm -rf node_modules`
//!      would get Auto mode switched off, which is a worse outcome than the
//!      hole.
//!
//! …and one that is about trust rather than files (numbered 7, as the owner's
//! D6 ruling names it):
//!   7. an agent shell that makes ssh **trust a server's host key** nobody
//!      checked: `ssh`, `scp`, `sftp` (and `autossh`, `slogin`, `ssh-copy-id`) with
//!      `StrictHostKeyChecking` set to `accept-new`, `no`, `off` or `false` in any
//!      spelling OpenSSH reads (`-o Key=value`, `-oKey=value`, `-o 'Key value'`,
//!      a flag cluster such as `-vo`, any letter case, a `ProxyCommand` that runs
//!      such an ssh, a `-F` configuration file that sets it); `KnownHostsCommand`
//!      set to any command but `none` (`ssh-keyscan %H` hands ssh the server's own
//!      key), and `UserKnownHostsFile` or `GlobalKnownHostsFile` set to anything
//!      but the default (`ssh-keyscan host > /tmp/kh` then
//!      `-o UserKnownHostsFile=/tmp/kh` trusts the scan), read in all the same
//!      places; the same options given to `sshfs` (`-o` options, comma-separated
//!      as FUSE reads them, and `-F`), and the ssh command line it runs with
//!      `-o ssh_command=CMD`; the same ssh command line handed over by
//!      `rsync -e`/`--rsh`, `mosh --ssh`, `GIT_SSH_COMMAND`, `RSYNC_RSH` or git's
//!      `core.sshCommand`,
//!      with the variable set in any shell's spelling (POSIX `NAME=value`,
//!      PowerShell `$env:NAME = value`, cmd `set NAME=value`, `setx`); a wrapper
//!      program run in ssh's place (`GIT_SSH`, `scp -S`, `sftp -S`, a remote
//!      shell named by path), read as a `-F` file is; `ANSIBLE_HOST_KEY_CHECKING`
//!      off; and `ssh-keygen -R`, which drops
//!      the key this computer trusts so the next connection takes whatever it is
//!      offered. A write INTO `known_hosts` (`ssh-keyscan … >> ~/.ssh/known_hosts`,
//!      `tee -a`) is already criterion 4, since `.ssh` is a credential path.
//!      Reading a key (`ssh-keygen -F`, `ssh-keyscan` with no redirect, `cat`)
//!      and plain `ssh` are never graded. Why: Crew forces
//!      `StrictHostKeyChecking yes` on every hop, so the only thing standing
//!      between an unverified server and a connection as the person is the
//!      known-hosts file, and the person (not the agent) adds to it after
//!      checking the fingerprint ("user warns, agent never"). A person's own
//!      terminal is untouched: this gate grades agent tool calls only.
//!
//! ## Provenance is corroborated, never taken on the model's word
//!
//! The "this session created it" exemption is the one input an *attacker in the
//! loop* writes, so it is only consulted for a call that actually ran and
//! succeeded (the paired `ToolResponse`, which the agent writes, not the
//! request, which the model writes) *and* only where the filesystem does not
//! contradict it — a directory holding content older than the session was not
//! created by it. Both halves close a real hole: a **denied** `mkdir` sat in
//! the request stream and vouched for the delete that followed, and
//! `mkdir -p X` on an existing `X` succeeds while creating nothing, so a model
//! whose `rm -rf X` had just been refused could manufacture its own permission
//! and retry on the next turn.
//!
//! ⚠ Over a directory that already holds a `.git`, the filesystem cannot
//! adjudicate (its content may all post-date the session), so provenance there
//! is restricted to verbs that FAIL on a populated existing tree — `git clone`,
//! `git worktree add`, `cargo new`. `git init` and every bare `mkdir` spelling
//! are excluded precisely because they succeed as no-ops and are therefore
//! replayable over the victim.
//!
//! A **glob** delete (`rm -rf *`, `rm -rf kdps-*`) is graded on its containing
//! directory, because the shell expands the pattern and the paths destroyed are
//! never in the command text. It fails **closed** rather than falling silently
//! outside the criterion.
//!
//! Reads are intentionally out of scope here: secret-file reads are already
//! denied by `SecretGuard`. Only mutations escalate, so that ordinary Auto-mode
//! work — writing scratch files, editing the workspace, deleting build output —
//! keeps running with no prompt.
//!
//! Desktop observation and interaction use the separate host-owned Biorouter Copilot
//! task consent gate at dispatch, including nested code execution. They do not
//! receive a second, per-action approval from this filesystem inspector.
//!
//! # Boundary (documented for security review)
//!
//! This gate inspects three argument shapes on a tool call:
//!   1. the **path arguments of file-editor-style tool calls** (`text_editor`
//!      and `*_file` / `mkdir` / … tools) — the original behaviour;
//!   2. **shell command lines** (`developer/shell` and any command-bearing tool)
//!      — a redirect (`>` / `>>`) into a sensitive target, or the path argument
//!      of a mutating binary (`cp`, `mv`, `tee`, `install`, `Set-Content`, …);
//!   3. the **`code` body of `code_execution/execute_code`** — the opaque JS
//!      blob the default code-execution mode wraps every file op in. Its
//!      *inner* tool calls are dispatched straight through the extension
//!      manager and never reach an agent-layer inspector, so a sensitive write
//!      hidden inside a script would otherwise run with no prompt (the R2-01
//!      regression: `echo … >> ~/.ssh/config` executed silently in Auto mode).
//!      The body's string literals are scanned as embedded shell commands, and
//!      each inner `callee({…})` site is graded by the *same* classifier the
//!      outer tool call gets — a mutating callee, and only its own path-keyed
//!      arguments as targets.
//!
//!      A candidate path is always tied to the concrete call that would mutate
//!      it. Scanning every literal in a script because *some* token elsewhere
//!      looked like a write is a lexical cross-product, and it produced issue
//!      #106: `kb_write_page` (a knowledge-base tool whose name contains
//!      "write") beside an unrelated `page.path.split("/")` read the utility
//!      literal `"/"` as the filesystem root and stalled Auto mode on a prompt.
//!
//! Reads are never escalated: only redirect targets, mutating-binary targets,
//! mutating editor writes and recursive deletes count. A `cat /etc/hosts`, an
//! `ls` or a `view` of a sensitive path yields nothing — the read-only binaries
//! are deliberately absent from [`SHELL_MUTATING_BINARIES`] and criterion 5
//! asks for a recursion flag on a delete verb, so neither list can escalate one.
//!
//! Criterion 5 rides shapes 2 and 3 (command lines, including the ones embedded
//! in an `execute_code` body); it is deliberately NOT wired to shape 1, because
//! a file tool's `delete`/`remove` argument says nothing about recursion and
//! grading it would turn every `delete_file` into a candidate prompt.
//! Relative targets are resolved through any `cd` earlier in the same line:
//! the incident arrived as `cd <dir> && rm -rf <relative>`, so a classifier that
//! resolved against the session cwd would have missed the exact shape that
//! motivated the rule ([`resolved_command_targets`]).
//!
//! Known gaps (documented, not silently ignored):
//!   * A target *dynamically* constructed inside a script
//!     (`shell({command: \`… >> ${dir}/config\`})` with `dir` computed at
//!     runtime) cannot be resolved by static scanning and is not escalated.
//!     Gating the code-execution extension's *inner* dispatch boundary
//!     (`code_execution_extension::run_tool_handler`) against the same
//!     sensitivity check — with a deny, since that layer cannot surface an
//!     interactive ask — is the recommended deeper follow-up.
//!   * Criterion 5's glob handling keys off [`TargetPath::fixed_prefix`], which
//!     `normalize_for` sets for `*` and `?` only. A bracket expression
//!     (`rm -rf kdps-[0-9]`) or a brace expansion (`rm -rf {a,b}`) therefore
//!     resolves to one literal path that does not exist, and is not escalated.
//!     Teaching the normalizer those two metacharacter classes is the fix, and
//!     it belongs in `policy::target` beside the ones already there.
//!   * Criterion 5 reads the delete verb's own operands, so a deletion whose
//!     targets arrive from somewhere else — `find … -exec rm -rf {} \;`,
//!     `… | xargs rm -rf` — has no target to classify and is not escalated.
//!     That is the same shape as the gap above and has the same answer; it is
//!     not worth a heuristic here, because this gate exists to catch a
//!     *mistake*, not to withstand a path deliberately routed around it.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::agents::types::SharedProvider;
use crate::config::BioRouterMode;
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::privacy::CallCapability;
use crate::security::command_text_from;
use crate::security::policy::command::{redirect_targets, ParsedCommand, Segment};
use crate::security::policy::target::{
    classify, normalize_for, Blast, Dialect, EnvFacts, TargetPath,
};
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};

/// `text_editor`-style `command` values that MUTATE the target (everything but
/// `view`). An editor call with no explicit command fails safe → treated as a
/// mutation (so it is checked, not silently allowed).
const MUTATING_EDITOR_COMMANDS: &[&str] = &[
    "write",
    "create",
    "str_replace",
    "insert",
    "undo_edit",
    "diff",
    "delete",
    "append",
];

/// Tool-name substrings that mark a non-editor tool as a file mutation. Only
/// consulted together with a sensitive target path, so an over-broad match here
/// cannot escalate an ordinary-path call.
const MUTATING_NAME_HINTS: &[&str] = &[
    "write",
    "create",
    "edit",
    "delete",
    "remove",
    "append",
    "save",
    "move",
    "rename",
    "mkdir",
    "put",
    "unlink",
    "rmdir",
    "overwrite",
    "patch",
    "copy",
    "export",
    "import",
];

/// Tool-name substrings that mark a tool as read-only, vetoing a mutating-hint
/// match (belt-and-suspenders: the hint sets do not overlap).
const READONLY_NAME_HINTS: &[&str] = &["view", "read", "list", "search", "preview", "get_"];

/// Credential / persistence directories under `$HOME`. Stored lowercased and
/// `/`-separated (relative to home); a mutation at or beneath any of these is
/// sensitive even though it lives under `$HOME` (so it is [`Blast::Ordinary`]).
const SENSITIVE_HOME_SUBPATHS: &[&str] = &[
    ".ssh",           // SSH private keys, authorized_keys, config
    ".gnupg",         // GPG keyring
    ".aws",           // AWS credentials/config
    ".config/gcloud", // Google Cloud creds
    ".kube",          // Kubernetes creds
    ".docker",        // Docker registry auth
    // macOS
    "library/keychains",                         // user keychain
    "library/launchagents",                      // per-user launchd persistence
    "library/preferences/com.apple.loginwindow", // login hooks
    "library/application support/google/chrome",
    "library/application support/chromium",
    "library/application support/bravesoftware",
    "library/application support/firefox",
    "library/application support/microsoft edge",
    // Linux
    ".config/google-chrome",
    ".config/chromium",
    ".config/bravesoftware",
    ".mozilla",
    // Windows (%LOCALAPPDATA% / %APPDATA% expand under home for the classifier)
    "appdata/local/google/chrome",
    "appdata/roaming/mozilla",
];

/// True for a normalized path inside an OS temp tree. macOS `$TMPDIR` lives
/// under `/var/folders/**` (a system dir), and `/tmp` is `/private/tmp`; agents
/// write scratch files there constantly, so a temp target is never sensitive.
fn is_temp_path(norm: &str) -> bool {
    let p = norm.to_ascii_lowercase();
    if p == "/tmp" || p == "/private/tmp" || p == "/var/tmp" || p == "/private/var/tmp" {
        return true;
    }
    const TEMP_PREFIXES: &[&str] = &[
        "/tmp/",
        "/private/tmp/",
        "/var/folders/",
        "/private/var/folders/",
        "/var/tmp/",
        "/private/var/tmp/",
    ];
    if TEMP_PREFIXES.iter().any(|t| p.starts_with(t)) {
        return true;
    }
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        let t = tmpdir.trim_end_matches('/').to_ascii_lowercase();
        if !t.is_empty() && (p == t || p.starts_with(&format!("{t}/"))) {
            return true;
        }
    }
    false
}

/// True for the benign character devices that live under `/dev` (a system dir)
/// but are ubiquitous, harmless write targets: `/dev/null` is the universal
/// bit-bucket, and `/dev/std{out,err,in}`, `/dev/tty`, `/dev/fd/N`, and the
/// zero/random generators are standard redirect destinations. Writing to any of
/// them mutates nothing on disk. Without this exemption a routine
/// `… 2>/dev/null` (or a `/dev/null` literal inside code authored by an
/// `execute_code` write) trips the Auto-mode sensitive-write escalation and
/// parks the agent on a permission prompt it should never see.
fn is_benign_device(norm: &str) -> bool {
    let p = norm.to_ascii_lowercase();
    matches!(
        p.as_str(),
        "/dev/null"
            | "/dev/zero"
            | "/dev/full"
            | "/dev/stdin"
            | "/dev/stdout"
            | "/dev/stderr"
            | "/dev/tty"
            | "/dev/random"
            | "/dev/urandom"
    ) || p.starts_with("/dev/fd/")
}

/// The human-facing reason a target is sensitive, or `None` if it is ordinary.
fn sensitivity_reason(tp: &TargetPath, env: &EnvFacts) -> Option<&'static str> {
    match classify(tp, env) {
        Blast::Root | Blast::WildcardAtRoot => {
            return Some("the filesystem root or a whole volume")
        }
        Blast::HomeBare => return Some("your home directory itself"),
        Blast::SystemDir => {
            if !is_temp_path(&tp.norm) && !is_benign_device(&tp.norm) {
                return Some("a protected system directory");
            }
            // A temp path under a system dir (e.g. /var/folders) or a benign
            // pseudo-device (/dev/null, /dev/stderr, …) is ordinary.
        }
        Blast::Ordinary => {}
    }

    // Credential / persistence locations under $HOME (Blast::Ordinary above).
    if !env.home.is_empty() {
        let p = tp.norm.to_ascii_lowercase();
        let home = normalize_for(env.platform, &env.home, env)
            .norm
            .to_ascii_lowercase();
        if !home.is_empty() {
            if let Some(rel) = p.strip_prefix(&format!("{home}/")) {
                for sub in SENSITIVE_HOME_SUBPATHS {
                    if rel == *sub || rel.starts_with(&format!("{sub}/")) {
                        return Some("a credential or persistence path in your home directory");
                    }
                }
            }
        }
    }
    None
}

/// Whether this tool call is a file **mutation** (create/write/edit/delete/…),
/// as opposed to a read/`view`. Editor tools carry the operation in `command`;
/// other file tools are inferred from the tool name.
fn operation_is_mutating(tool_name: &str, args: &Map<String, Value>) -> bool {
    let name = tool_name.to_ascii_lowercase();
    let is_editor = name.contains("text_editor") || name.contains("str_replace");
    if is_editor {
        return match args.get("command").and_then(Value::as_str) {
            Some(cmd) => {
                MUTATING_EDITOR_COMMANDS.contains(&cmd.trim().to_ascii_lowercase().as_str())
            }
            None => true, // no explicit command → fail safe (check it)
        };
    }
    // ⚠ A tool whose destructive verb lives in an ARGUMENT rather than in its
    // name is invisible to a name-only rule, and the shipped example is
    // `webdocuments__cache`: it is named for the thing it manages, and
    // `{command: "delete", path: …}` calls `fs::remove_file` on whatever that
    // path names. "cache" contains no mutating hint and no read-only hint, so
    // the rule below answered `false` and no approval card was ever built —
    // in Auto mode the delete simply happened.
    //
    // The editor branch above already treats `command` as authoritative for the
    // one tool family that was known to carry it; this generalises that to any
    // tool that does. Only a single-token value counts as a verb: `developer__shell`
    // also carries `command`, but its value is a command LINE and is scanned as
    // one by `command_findings`, so admitting whitespace here would grade the
    // same string twice under two different meanings.
    if let Some(verb) = args.get("command").and_then(Value::as_str) {
        let verb = verb.trim().to_ascii_lowercase();
        if !verb.is_empty()
            && !verb.contains(char::is_whitespace)
            && (MUTATING_EDITOR_COMMANDS.contains(&verb.as_str())
                || MUTATING_NAME_HINTS.iter().any(|hint| verb.contains(hint)))
        {
            return true;
        }
    }
    if READONLY_NAME_HINTS.iter().any(|h| name.contains(h)) {
        return false;
    }
    MUTATING_NAME_HINTS.iter().any(|h| name.contains(h))
}

/// Every path-like value in the argument map (string or array-of-strings).
fn path_values(args: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    for (key, value) in args {
        if !crate::security::is_path_argument_key(key) {
            continue;
        }
        match value {
            Value::String(s) => out.push(s.clone()),
            Value::Array(items) => {
                for item in items {
                    if let Some(s) = item.as_str() {
                        out.push(s.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Shell binaries whose path arguments are *write* targets. POSIX basenames,
/// PowerShell canonical cmdlets (matched case-insensitively — [`ParsedCommand`]
/// normalizes `ri` → `Remove-Item`), and `cmd.exe` verbs. Read-only tools
/// (`cat`, `less`, `grep`, `ls`) are deliberately absent, so reading a sensitive
/// path is never escalated.
#[rustfmt::skip]
const SHELL_MUTATING_BINARIES: &[&str] = &[
    // POSIX
    "cp", "mv", "rm", "rmdir", "unlink", "mkdir", "touch", "tee", "install", "ln", "dd",
    "truncate", "shred", "chmod", "chown", "chgrp", "mkfifo", "mknod", "patch", "rsync",
    // PowerShell cmdlets (canonicalized by the parser)
    "set-content", "add-content", "out-file", "new-item", "copy-item", "move-item",
    "remove-item", "clear-content", "rename-item",
    // cmd.exe
    "copy", "move", "del", "erase", "md", "rd", "ren", "rename", "xcopy", "robocopy",
];

/// True for the code-execution `execute_code` tool, whose opaque JS body carries
/// the real (inner) tool calls and must be scanned for embedded sensitive writes.
fn is_execute_code(tool_name: &str) -> bool {
    tool_name.to_ascii_lowercase().contains("execute_code")
}

/// A sensitive **write** in a shell command line: any redirect target (`>` /
/// `>>`), or the path argument of a mutating binary. Returns the finding phrase,
/// or `None` for a read.
fn command_writes_sensitively(command: &str, env: &EnvFacts, found: &mut Findings) {
    // Redirects are unconditional writes, wherever in the line they appear.
    for rt in redirect_targets(command) {
        let tp = normalize_for(env.platform, &rt, env);
        if let Some(reason) = sensitivity_reason(&tp, env) {
            if found.push(write_finding(&tp.norm, reason)) {
                return;
            }
        }
    }
    // Mutating binaries: their (already classified) path/redirect targets.
    let parsed = ParsedCommand::parse_for(command, env.platform, env);
    for seg in &parsed.segments {
        if !SHELL_MUTATING_BINARIES.contains(&seg.binary.to_ascii_lowercase().as_str()) {
            continue;
        }
        for hit in &seg.targets {
            if let Some(reason) = sensitivity_reason(&hit.path, env) {
                if found.push(write_finding(&hit.path.norm, reason)) {
                    return;
                }
            }
        }
    }
}

/// The finding phrase for criteria 1-4, which all describe a write to a
/// sensitive *location*. Criterion 5 writes its own (it has to name what would
/// be destroyed, not which rule matched), and so does criterion 7
/// ([`host_key_finding`], which names the key that would be trusted).
fn write_finding(path: &str, reason: &str) -> String {
    format!("writes to {path} ({reason})")
}

/// How many sensitive operations one tool call is graded for.
///
/// Not a display limit — [`Findings::describe`] shows fewer than this. It is the
/// **work** limit: criterion 5 spends a bounded directory walk per target, so a
/// script naming a thousand deletes would otherwise put a thousand walks on the
/// agent's critical path. At the cap the card says "at least N", which is the
/// honest thing to say about a list that was not finished.
const MAX_GRADED_FINDINGS: usize = 32;

/// How many of them the approval card names.
///
/// A card the user cannot read is not consent either, so the list is short and
/// the remainder is counted rather than printed.
const SHOWN_FINDINGS: usize = 6;

/// The sensitive operations found in one tool call.
///
/// ⚠ **Ordered and de-duplicated, and both matter.** The order is the order the
/// call would perform them, so the first entry is still the one the old
/// single-finding card named — that is what keeps every existing expectation
/// about *which* operation is reported intact. De-duplication is not cosmetic:
/// an `execute_code` body is scanned twice (string literals as command lines,
/// then inner tool calls), so the same write can be reached down both paths and
/// would otherwise be counted twice in a number the user is asked to trust.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Findings {
    reasons: Vec<String>,
    /// The grading stopped at [`MAX_GRADED_FINDINGS`], so `reasons.len()` is a
    /// lower bound on how many operations the call really performs.
    capped: bool,
}

impl Findings {
    /// Record one finding. Returns `true` once the budget is spent, so a scanning
    /// loop can stop rather than keep paying for probes it will not report.
    fn push(&mut self, reason: String) -> bool {
        if self.capped {
            return true;
        }
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
        if self.reasons.len() >= MAX_GRADED_FINDINGS {
            self.capped = true;
        }
        self.capped
    }

    fn is_full(&self) -> bool {
        self.capped
    }

    pub fn is_empty(&self) -> bool {
        self.reasons.is_empty()
    }

    pub fn into_reasons(self) -> Vec<String> {
        self.reasons
    }

    /// The clause naming what the call would affect, for the approval card and
    /// for the [`InspectionResult::reason`] beside it.
    ///
    /// One finding reads as a sentence, because that is the overwhelmingly common
    /// case and a one-item bulleted list is worse prose than a clause. Several
    /// read as a counted list: the count first, so the user learns the blast
    /// radius before the paths, and the tail summarised rather than printed.
    pub fn describe(&self) -> String {
        match self.reasons.as_slice() {
            [] => String::new(),
            [only] => format!("This tool call {only}."),
            many => {
                let at_least = if self.capped { "at least " } else { "" };
                let mut out = format!(
                    "This tool call affects {at_least}{} places on this machine:",
                    many.len()
                );
                for reason in many.iter().take(SHOWN_FINDINGS) {
                    out.push_str("\n  • it ");
                    out.push_str(reason);
                }
                if many.len() > SHOWN_FINDINGS {
                    out.push_str(&format!(
                        "\n  • …and {} more, not listed here.",
                        many.len() - SHOWN_FINDINGS
                    ));
                }
                out
            }
        }
    }

    /// The short form for logs and for the finding's own `reason` field.
    fn summary(&self) -> String {
        match self.reasons.as_slice() {
            [] => String::new(),
            [only] => format!("Sensitive file operation ({only})"),
            many => {
                let at_least = if self.capped { "at least " } else { "" };
                format!(
                    "Sensitive file operations at {at_least}{} paths (first: {})",
                    many.len(),
                    many[0]
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Criterion 5 — recursive deletion of an established directory
// ---------------------------------------------------------------------------

/// Everything outside the tool call itself that the sensitive-operation rules
/// consult. Grouped into one value so criterion 5 can ask about the session
/// without every classifier growing a third, fourth and fifth parameter.
///
/// The provenance set is derived **lazily and once**: walking a long
/// conversation is wasted work for the overwhelming majority of calls, which
/// name no deletion target at all.
pub struct CallSession<'a> {
    working_dir: &'a Path,
    started_at: SystemTime,
    messages: &'a [Message],
    current_request_ids: &'a [String],
    created_dirs: OnceCell<HashSet<String>>,
    /// The replay-proof subset. See [`strongly_created_dir_operands`].
    strongly_created_dirs: OnceCell<HashSet<String>>,
}

impl<'a> CallSession<'a> {
    pub fn new(
        working_dir: &'a Path,
        started_at: SystemTime,
        messages: &'a [Message],
        current_request_ids: &'a [String],
    ) -> Self {
        Self {
            working_dir,
            started_at,
            messages,
            current_request_ids,
            created_dirs: OnceCell::new(),
            strongly_created_dirs: OnceCell::new(),
        }
    }

    /// A session with a working directory and nothing else: no start time and no
    /// history. Criterion 5 then rests on the two facts that need neither — a
    /// directory holding a git repository, and one holding content the
    /// filesystem cannot date.
    pub fn workspace_only(working_dir: &'a Path) -> Self {
        Self::new(working_dir, UNIX_EPOCH, &[], &[])
    }

    pub fn working_dir(&self) -> &Path {
        self.working_dir
    }

    fn started_at(&self) -> SystemTime {
        self.started_at
    }

    /// Normalized paths of every directory an **earlier** tool call in this
    /// conversation asked to bring into existence *and actually did*.
    fn created_dirs(&self, env: &EnvFacts) -> &HashSet<String> {
        self.created_dirs
            .get_or_init(|| self.scan_created_dirs(env, false))
    }

    /// As [`Self::created_dirs`], but only the directories vouched for by a verb
    /// that cannot succeed onto a populated existing tree. This is the set the
    /// `has_git` branch must use — see [`strongly_created_dir_operands`].
    fn strongly_created_dirs(&self, env: &EnvFacts) -> &HashSet<String> {
        self.strongly_created_dirs
            .get_or_init(|| self.scan_created_dirs(env, true))
    }

    /// The ids of the tool calls this conversation records as having actually
    /// **run and succeeded**.
    ///
    /// A `ToolRequest` is text the *model* wrote; the paired `ToolResponse` is
    /// written by the agent once the call returned. Provenance therefore reads
    /// the response, so a creation that was denied at the approval prompt,
    /// cancelled, or that failed outright vouches for nothing — otherwise a
    /// refused `mkdir` still unlocks the delete that follows it, which is the
    /// same "a denied operation becomes its own permission" shape criterion 5
    /// exists to close.
    ///
    /// A tool fails two ways, the convention used across this crate (see
    /// `tool_monitor::failure_text`): a transport/protocol `Err`, or an `Ok`
    /// flagged `is_error`.
    fn executed_request_ids(&self) -> HashSet<&'a str> {
        let mut out = HashSet::new();
        for message in self.messages {
            for content in &message.content {
                let MessageContent::ToolResponse(response) = content else {
                    continue;
                };
                if matches!(&response.tool_result, Ok(result) if result.is_error != Some(true)) {
                    out.insert(response.id.as_str());
                }
            }
        }
        out
    }

    fn scan_created_dirs(&self, env: &EnvFacts, strong_only: bool) -> HashSet<String> {
        let executed = self.executed_request_ids();
        let mut out = HashSet::new();
        for message in self.messages {
            for content in &message.content {
                let MessageContent::ToolRequest(request) = content else {
                    continue;
                };
                // A call does not vouch for itself: `rm -rf x && mkdir x` is the
                // exact shape of the incident this criterion exists for, so the
                // batch under inspection is never its own provenance.
                if self.current_request_ids.contains(&request.id) {
                    continue;
                }
                // …and a call that never ran vouches for nothing either.
                if !executed.contains(request.id.as_str()) {
                    continue;
                }
                let Ok(call) = &request.tool_call else {
                    continue;
                };
                let Some(args) = call.arguments.as_ref() else {
                    continue;
                };
                collect_created_dirs(&call.name, args, env, strong_only, &mut out);
            }
        }
        out
    }
}

/// When the session began, as a wall-clock instant. A row whose `created_at` is
/// the epoch default carries no usable start, and resolves to [`UNIX_EPOCH`],
/// which switches the *age* half of criterion 5 off rather than dating every
/// file on the disk as pre-session.
fn session_started_at(session: &crate::session::Session) -> SystemTime {
    let secs = session.created_at.timestamp();
    if secs <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    }
}

/// Directory names that name **regenerable** output — build products, installed
/// dependencies, caches. Deleting one costs a rebuild, never data, and an agent
/// in Auto mode does it constantly; a prompt here is the failure mode that gets
/// Auto mode switched off. Consulted by basename only, and **overridden by a
/// `.git`**: a directory that is itself a repository is not build output,
/// whatever it is called.
const REGENERABLE_DIR_NAMES: &[&str] = &[
    "node_modules",
    "bower_components",
    "vendor",
    "target",
    "dist",
    "build",
    "out",
    "obj",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".output",
    ".vite",
    ".angular",
    ".turbo",
    ".parcel-cache",
    ".cache",
    ".gradle",
    ".terraform",
    ".tox",
    ".eggs",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".venv",
    "venv",
    "coverage",
    "htmlcov",
    ".sass-cache",
    ".ipynb_checkpoints",
];

/// Entry budget for [`probe_directory`]. The probe answers three yes/no
/// questions and produces one number for the prompt; it is not an inventory, so
/// it stops early and says so rather than walking a tree of any size on the
/// agent's critical path.
const DIR_PROBE_MAX_ENTRIES: usize = 1000;

/// Depth budget for [`probe_directory`], for the same reason.
const DIR_PROBE_MAX_DEPTH: usize = 6;

/// Is this segment a **recursive** directory delete? Recursion is required, not
/// force: `rm -r dir` destroys just as much as `rm -rf dir`, while `rm -f file`
/// and `rmdir dir` (which refuses a non-empty directory) destroy nothing this
/// criterion is about.
fn is_recursive_delete(seg: &Segment) -> bool {
    match seg.dialect {
        Dialect::Posix => {
            seg.binary.eq_ignore_ascii_case("rm") && posix_has_recursive_flag(&seg.argv)
        }
        // The parser canonicalizes `ri` / `rd` / `del` to `Remove-Item` and
        // expands an unambiguous `-rec` prefix to `-Recurse`.
        Dialect::PowerShell => {
            seg.binary.eq_ignore_ascii_case("remove-item") && seg.has_arg("-Recurse")
        }
        Dialect::Cmd => {
            matches!(seg.binary.as_str(), "rd" | "rmdir")
                && seg.argv.iter().any(|t| t.eq_ignore_ascii_case("/s"))
        }
    }
}

/// `-r`, `-R` or `--recursive`, including inside a bundled short cluster
/// (`-rf`, `-fr`, `-vrf`). Scanning stops at `--`, after which a token is an
/// operand even when it opens with a dash.
fn posix_has_recursive_flag(argv: &[String]) -> bool {
    for tok in argv.iter().skip(1) {
        if tok == "--" {
            return false;
        }
        if tok == "--recursive" {
            return true;
        }
        if tok.starts_with("--") {
            continue;
        }
        if let Some(cluster) = tok.strip_prefix('-') {
            if cluster.chars().any(|c| c == 'r' || c == 'R') {
                return true;
            }
        }
    }
    false
}

/// A segment's operand tokens — its path arguments, with this dialect's flags
/// removed. A PowerShell parameter *value* (`-Path C:\x`) survives, because it
/// does not itself open with a dash.
fn segment_operands(seg: &Segment) -> Vec<String> {
    let mut out = Vec::new();
    let mut end_of_flags = false;
    for tok in seg.argv.iter().skip(1) {
        let is_flag = match seg.dialect {
            Dialect::Posix => {
                if !end_of_flags && tok == "--" {
                    end_of_flags = true;
                    continue;
                }
                !end_of_flags && tok.len() > 1 && tok.starts_with('-')
            }
            Dialect::PowerShell => tok.starts_with('-'),
            Dialect::Cmd => tok.starts_with('/'),
        };
        if !is_flag {
            out.push(tok.clone());
        }
    }
    out
}

/// The directory a `cd`-style segment moves to, if it can be resolved.
///
/// This is what makes `cd /a/b && rm -rf c` classify `/a/b/c` rather than
/// `<session cwd>/c` — the shape the incident arrived in. `cd -` names the
/// previous directory, which is not recoverable from the text, so it leaves the
/// tracked directory alone.
fn cd_destination(seg: &Segment) -> Option<String> {
    let binary = seg.binary.to_ascii_lowercase();
    if !matches!(
        binary.as_str(),
        "cd" | "chdir" | "pushd" | "set-location" | "sl"
    ) {
        return None;
    }
    match segment_operands(seg).into_iter().next() {
        Some(dest) if dest == "-" => None,
        Some(dest) => Some(dest),
        // A bare `cd` goes home; a bare `pushd` swaps, which is not resolvable.
        None if binary == "pushd" => None,
        None => Some("~".to_string()),
    }
}

/// Resolve the operands `pick` selects against the working directory **in
/// effect at that point in the command line**, following `cd` as the shell
/// would. Segments arrive in textual order, so a `cd` only affects what comes
/// after it.
fn resolved_command_targets<F>(command: &str, env: &EnvFacts, pick: F) -> Vec<TargetPath>
where
    F: Fn(&Segment) -> Vec<String>,
{
    let parsed = ParsedCommand::parse_for(command, env.platform, env);
    let mut here = env.clone();
    let mut out = Vec::new();
    for seg in &parsed.segments {
        if let Some(dest) = cd_destination(seg) {
            here.cwd = normalize_for(here.platform, &dest, &here).norm;
            continue;
        }
        for raw in pick(seg) {
            if raw.trim().is_empty() {
                continue;
            }
            out.push(normalize_for(here.platform, &raw, &here));
        }
    }
    out
}

/// Every path a recursive delete in `command` would remove.
fn recursive_delete_targets(command: &str, env: &EnvFacts) -> Vec<TargetPath> {
    resolved_command_targets(command, env, |seg| {
        if is_recursive_delete(seg) {
            segment_operands(seg)
        } else {
            Vec::new()
        }
    })
}

/// Every directory `command` would bring into existence — the provenance half
/// of criterion 5.
fn directory_creation_targets(command: &str, env: &EnvFacts) -> Vec<TargetPath> {
    resolved_command_targets(command, env, created_dir_operands)
}

/// The operands of a directory-creating segment. Deliberately short: a command
/// missing from here costs at most one approval prompt for a directory the
/// session did make, whereas a wrong entry silently *withholds* a prompt.
fn created_dir_operands(seg: &Segment) -> Vec<String> {
    let binary = seg.binary.to_ascii_lowercase();
    let operands = segment_operands(seg);
    match binary.as_str() {
        "mkdir" | "md" | "new-item" => operands,
        "git" => git_created_dirs(&operands),
        "cargo" => match operands.split_first() {
            Some((sub, rest)) if matches!(sub.as_str(), "new" | "init") => {
                rest.first().cloned().into_iter().collect()
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The operands of a segment whose verb **cannot succeed onto a directory that
/// already holds a populated repository**.
///
/// ⚠ This is the narrower sibling of [`created_dir_operands`], and the
/// difference is the whole point. The wide list above is safe wherever the
/// filesystem is also consulted, because a tree the session really made holds
/// no pre-session content and the age test alone answers. It is NOT safe in the
/// one shape where the age test cannot answer — a directory bearing a `.git`
/// whose every entry post-dates the session — because `mkdir -p <existing repo>`
/// and `git init <existing repo>` both **succeed while creating nothing**. A
/// model whose `rm -rf <repo>` was refused could issue either one, get a real
/// success response, and retry into a silent recursive delete: the denied
/// operation minting its own permission, which is exactly the hole criterion 5
/// exists to close.
///
/// So for that case provenance is restricted to verbs that FAIL on a populated
/// existing directory, and therefore cannot be replayed over the victim:
/// `git clone`, `git worktree add`, `cargo new`. `git init` and every bare
/// `mkdir` spelling are deliberately absent.
fn strongly_created_dir_operands(seg: &Segment) -> Vec<String> {
    let binary = seg.binary.to_ascii_lowercase();
    let operands = segment_operands(seg);
    match binary.as_str() {
        "git" => match operands.split_first() {
            Some((sub, rest)) if sub == "clone" => match rest {
                [] => Vec::new(),
                [url] => clone_dir_from_url(url).into_iter().collect(),
                [_url, dir, ..] => vec![dir.clone()],
            },
            Some((sub, rest)) if sub == "worktree" => match rest.split_first() {
                Some((add, tail)) if add == "add" => tail.first().cloned().into_iter().collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        },
        // `cargo new` refuses an existing directory; `cargo init` does not.
        "cargo" => match operands.split_first() {
            Some((sub, rest)) if sub == "new" => rest.first().cloned().into_iter().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// [`directory_creation_targets`], restricted to the replay-proof verbs.
fn strong_directory_creation_targets(command: &str, env: &EnvFacts) -> Vec<TargetPath> {
    resolved_command_targets(command, env, strongly_created_dir_operands)
}

/// `git clone <url> [dir]`, `git init <dir>`, `git worktree add <dir>`.
///
/// Worth the special case rather than left to the age heuristic: a clone the
/// session made is exactly a directory that is non-empty and holds a `.git`,
/// which is otherwise the strongest signal criterion 5 has.
fn git_created_dirs(operands: &[String]) -> Vec<String> {
    let Some((sub, rest)) = operands.split_first() else {
        return Vec::new();
    };
    match sub.as_str() {
        "clone" => match rest {
            [] => Vec::new(),
            [url] => clone_dir_from_url(url).into_iter().collect(),
            [_url, dir, ..] => vec![dir.clone()],
        },
        // A bare `git init` / `cargo init` does not create its directory.
        "init" => rest.first().cloned().into_iter().collect(),
        "worktree" => match rest.split_first() {
            Some((add, tail)) if add == "add" => tail.first().cloned().into_iter().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The directory `git clone <url>` creates when given no destination: the URL's
/// last component with any `.git` suffix removed.
fn clone_dir_from_url(url: &str) -> Option<String> {
    let leaf = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let leaf = leaf.strip_suffix(".git").unwrap_or(leaf);
    (!leaf.is_empty()).then(|| leaf.to_string())
}

/// Tool names that create a directory outside a shell (`files__mkdir`,
/// `create_directory`, …).
fn tool_name_creates_directories(tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();
    ["mkdir", "create_dir", "make_dir", "new_dir"]
        .iter()
        .any(|hint| name.contains(hint))
}

/// Record the directories one historical tool call asked to create: its own path
/// arguments when it is a directory-creating tool, the command line it carries,
/// and — for `execute_code` — the command lines embedded in its body.
fn collect_created_dirs(
    tool_name: &str,
    args: &Map<String, Value>,
    env: &EnvFacts,
    strong_only: bool,
    out: &mut HashSet<String>,
) {
    // A dedicated create-directory TOOL is a structured call, not a command line
    // the model composed — but it is still `mkdir`-shaped: it succeeds on an
    // existing directory and so cannot vouch in the replay-proof set.
    if !strong_only && tool_name_creates_directories(tool_name) {
        for raw in path_values(args) {
            if !raw.trim().is_empty() {
                out.insert(normalize_for(env.platform, &raw, env).norm);
            }
        }
    }
    if let Some(command) = command_text_from(tool_name, args) {
        let targets = if strong_only {
            strong_directory_creation_targets(&command, env)
        } else {
            directory_creation_targets(&command, env)
        };
        out.extend(targets.into_iter().map(|t| t.norm));
    }
    // ⚠ A string LITERAL inside an `execute_code` body is not evidence the
    // command ran — it is evidence the model typed it. The call's success bit
    // says the JS returned, not that any particular literal was executed, so a
    // dead `"mkdir -p /victim"` sitting in the source would otherwise vouch.
    // Same reasoning disqualifies it from the replay-proof set entirely.
    if !strong_only && is_execute_code(tool_name) {
        if let Some(code) = args.get("code").and_then(Value::as_str) {
            for literal in extract_string_literals(code) {
                out.extend(
                    directory_creation_targets(&literal, env)
                        .into_iter()
                        .map(|t| t.norm),
                );
            }
        }
    }
}

/// What a recursive delete of one directory would actually destroy, as far as a
/// bounded look at the filesystem can say.
struct DirectoryProbe {
    /// Non-directory entries visited.
    files: usize,
    /// Every entry visited, directories included.
    entries: usize,
    /// The walk hit [`DIR_PROBE_MAX_ENTRIES`], so the counts are lower bounds.
    capped: bool,
    /// The target holds a `.git`, or is one. Destroying a repository's history
    /// is unrecoverable in a way that losing a working tree is not.
    has_git: bool,
    /// The filesystem dates the directory, or something in it, before the
    /// session began — so this is not content the session produced.
    holds_pre_session_content: bool,
}

impl DirectoryProbe {
    /// The size clause of the approval message. The prompt has to let the user
    /// recognise a mistake at a glance, which a rule name cannot do.
    fn describe(&self) -> String {
        let more = if self.capped { "+" } else { "" };
        let size = if self.files > 0 {
            format!("{}{more} files", self.files)
        } else {
            format!("{}{more} entries", self.entries)
        };
        if self.has_git {
            format!("{size}, contains a git repository")
        } else {
            size
        }
    }
}

/// Is `path` a git repository's working tree, or the `.git` directory itself?
///
/// One `stat`, which is what lets it gate the bounded walk below rather than
/// being read off it: `rm -rf node_modules` is the single commonest recursive
/// delete an Auto-mode agent makes, and it must not cost a directory walk to
/// reach the name exemption.
fn holds_git_repository(path: &Path) -> bool {
    path.join(".git").exists() || path.file_name().is_some_and(|name| name == ".git")
}

/// Does the basename name regenerable output? See [`REGENERABLE_DIR_NAMES`].
fn is_regenerable_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| REGENERABLE_DIR_NAMES.contains(&name.to_ascii_lowercase().as_str()))
}

/// Look at `path` on the host filesystem. `None` unless it is an existing,
/// non-empty **directory** — a missing path, a plain file, and a symlink (which
/// `rm -rf` unlinks without touching its target) are all outside this criterion.
fn probe_directory(
    path: &Path,
    session_start: SystemTime,
    has_git: bool,
) -> Option<DirectoryProbe> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let mut probe = DirectoryProbe {
        files: 0,
        entries: 0,
        capped: false,
        has_git,
        holds_pre_session_content: predates_session(&meta, session_start),
    };
    let mut queue = vec![(path.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if probe.entries >= DIR_PROBE_MAX_ENTRIES {
                probe.capped = true;
                return Some(probe);
            }
            probe.entries += 1;
            // `DirEntry::metadata` does not follow symlinks, which is what
            // `rm -rf` does too.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if predates_session(&meta, session_start) {
                probe.holds_pre_session_content = true;
            }
            if meta.is_dir() {
                if depth + 1 < DIR_PROBE_MAX_DEPTH {
                    queue.push((entry.path(), depth + 1));
                }
            } else {
                probe.files += 1;
            }
        }
    }
    (probe.entries > 0).then_some(probe)
}

/// Did this entry exist before the session started?
///
/// Birth time is preferred wherever the platform records it: an archive
/// extraction or a `cp -p` carries the *original* mtime, and would otherwise
/// read as pre-session content the session in fact produced. When neither time
/// is readable the answer is "no" — an unanswerable question must not become a
/// prompt, or an unusual filesystem turns every delete into one.
fn predates_session(meta: &std::fs::Metadata, session_start: SystemTime) -> bool {
    if session_start <= UNIX_EPOCH {
        return false;
    }
    match meta.created().or_else(|_| meta.modified()) {
        Ok(born) => born < session_start,
        Err(_) => false,
    }
}

/// Criterion 5, for one resolved deletion target: the recursive deletion of a
/// directory that already existed, is not empty, and is not this session's own
/// work.
///
/// A **glob** target (`rm -rf *`, `rm -rf kdps-*`) is graded on its containing
/// directory instead. The shell expands the pattern before `rm` runs, so the
/// paths that would be destroyed are not in the command text and cannot be
/// probed one by one — and a pattern destroys strictly *more* than the single
/// path beside it, so it must not be the one shape that walks through. It fails
/// **closed**: the container is graded on exactly the evidence and exemptions a
/// named target gets, so `rm -rf node_modules/*` and a delete inside a temp tree
/// stay silent while `rm -rf *` in a repository asks.
fn established_directory_reason(
    target: &TargetPath,
    session: &CallSession<'_>,
    env: &EnvFacts,
) -> Option<String> {
    if let Some(prefix) = target.fixed_prefix.as_deref() {
        let probe = established_directory_probe(prefix, session, env)?;
        return Some(format!(
            "recursively deletes everything matching {} inside {prefix} — an established \
             directory this session did not create ({})",
            target.norm,
            probe.describe()
        ));
    }
    let probe = established_directory_probe(&target.norm, session, env)?;
    Some(format!(
        "recursively deletes {} — an existing directory this session did not create ({})",
        target.norm,
        probe.describe()
    ))
}

/// Is the directory at `norm` *established* — already there, holding something,
/// and not this session's own work? `Some(probe)` when a recursive delete of it
/// has to be approved.
///
/// The order of the tests is the order of their cost — a string compare, then
/// one `stat`, then a bounded walk, and only then the conversation scan.
fn established_directory_probe(
    norm: &str,
    session: &CallSession<'_>,
    env: &EnvFacts,
) -> Option<DirectoryProbe> {
    if is_temp_path(norm) {
        return None;
    }
    let path = Path::new(norm);
    // A repository is never build output, whatever it is called, so the name
    // exemption yields to a `.git`.
    let has_git = holds_git_repository(path);
    if !has_git && is_regenerable_name(path) {
        return None;
    }
    let probe = probe_directory(path, session.started_at(), has_git)?;
    // Content the session itself produced is not the user's work: a scratch tree
    // built and torn down inside one turn must never prompt.
    if !has_git && !probe.holds_pre_session_content {
        return None;
    }
    // Provenance is the one input a *model* authors, so it may only excuse what
    // the filesystem does not contradict: a directory holding content older than
    // the session was not brought into existence by it, whatever an earlier
    // `mkdir` asked for. Uncorroborated, the exemption is forgeable twice over —
    // `mkdir -p X` on an existing X succeeds and creates nothing, and a model
    // whose `rm -rf X` was just refused can issue exactly that and retry, which
    // turns a denied delete into its own permission on the next turn. The
    // corroboration costs nothing where the exemption was earned: a tree the
    // session really did make has no pre-session content to find.
    //
    // ⚠ WHICH provenance set, and why it is not the wide one. The step above
    // returns for every NON-git directory in this state, so the only shape that
    // reaches here is `has_git && !holds_pre_session_content` — a repository
    // whose every entry post-dates the session. That is precisely the shape the
    // filesystem cannot adjudicate, so the model's word is the only input left,
    // and the wide set is forgeable in exactly one move: `mkdir -p <repo>` and
    // `git init <repo>` both SUCCEED while creating nothing, so a refused
    // `rm -rf <repo>` is unlocked by replaying either one and retrying. Reached
    // easily, too: any repository created after the session opened — by the user
    // in their own terminal, by a subagent, by `gh repo clone` or `cp -r` — is
    // git-bearing with no pre-session content.
    //
    // So this branch takes only verbs that FAIL on a populated existing
    // directory and therefore cannot be replayed over the victim.
    let vouched = if has_git {
        session.strongly_created_dirs(env).contains(norm)
    } else {
        session.created_dirs(env).contains(norm)
    };
    if !probe.holds_pre_session_content && vouched {
        return None;
    }
    Some(probe)
}

/// Every recursive delete in `command` that criterion 5 escalates.
///
/// ⚠ **All of them, not the first.** The approval card names what it found, and a
/// script that clears six established directories must not be described by one of
/// them — the user would approve five deletions the card never mentioned. That is
/// what makes each target worth its own bounded probe; [`Findings`] caps how many
/// probes one call can cost.
fn command_deletes_established_directory(
    command: &str,
    env: &EnvFacts,
    session: &CallSession<'_>,
    found: &mut Findings,
) {
    for target in recursive_delete_targets(command, env) {
        if let Some(reason) = established_directory_reason(&target, session, env) {
            if found.push(reason) {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Criterion 7 — an agent shell that trusts a server's host key
// ---------------------------------------------------------------------------

/// The SSH clients whose `-o` and `-F` options decide whether a host key is checked.
const SSH_CLIENTS: &[&str] = &["ssh", "scp", "sftp", "slogin", "autossh", "ssh-copy-id"];

/// `StrictHostKeyChecking` values that let ssh go on with a key nobody checked. OpenSSH reads
/// `no`, `off` and `false` as off, and `accept-new` writes an unknown key into known_hosts.
const TRUSTING_VALUES: &[&str] = &["accept-new", "no", "off", "false"];

/// Environment variables whose value is a whole ssh command line.
const SSH_COMMAND_VARIABLES: &[&str] = &["GIT_SSH_COMMAND", "RSYNC_RSH"];

/// Environment variables whose value names a program git runs in ssh's place, with the
/// arguments ssh would get.
const SSH_PROGRAM_VARIABLES: &[&str] = &["GIT_SSH"];

/// Ansible's switch for host key checking.
const ANSIBLE_HOST_KEY_CHECKING: &str = "ANSIBLE_HOST_KEY_CHECKING";

/// The values Ansible's boolean reading takes as off, lower case.
const ANSIBLE_FALSE_VALUES: &[&str] = &["false", "no", "off", "0", "n", "f"];

/// The ssh options that decide whether an unchecked host key is trusted, lower case.
/// `StrictHostKeyChecking` off takes any key; `KnownHostsCommand` takes whatever keys a command
/// prints (`ssh-keyscan %H` prints the server's own); a known-hosts file other than the default
/// holds whatever was put there, such as a scan of the server.
const HOST_KEY_OPTIONS: &[&str] = &[
    "stricthostkeychecking",
    "knownhostscommand",
    "userknownhostsfile",
    "globalknownhostsfile",
];

/// The programs criterion 7 reads the arguments of.
const HOST_KEY_PROGRAMS: &[&str] = &[
    "ssh",
    "scp",
    "sftp",
    "slogin",
    "autossh",
    "ssh-copy-id",
    "ssh-keygen",
    "rsync",
    "git",
    "mosh",
    "sshfs",
    "mount.sshfs",
    "mount.fuse.sshfs",
];

/// sshfs, which mounts a server's files over ssh, under every name its packages install it.
const SSHFS_PROGRAMS: &[&str] = &["sshfs", "mount.sshfs", "mount.fuse.sshfs"];

/// sshfs's short options that take a value: `-o` options, `-F` an ssh configuration file and
/// `-p` a port.
const SSHFS_VALUE_FLAGS: &str = "oFp";

/// How deep an ssh command line handed over inside another (`rsync -e`, a `ProxyCommand`) is
/// followed.
const MAX_TRUST_DEPTH: u8 = 4;

/// How much of a `-F` configuration file or a wrapper program is read.
const MAX_SSH_CONFIG_BYTES: u64 = 64 * 1024;

/// How many files (a `-F` configuration, a wrapper program) grading one command line may
/// open, so files that name each other cannot keep the inspection reading. A file past the
/// budget is judged as one that cannot be read.
const MAX_TRUST_FILES: u8 = 8;

/// The short options of `client` that take a value, as its getopt string declares them, so a
/// cluster such as `-vo Key=value` or `-p22` is read the way the client reads it.
fn ssh_value_flags(client: &str) -> &'static str {
    match client {
        "scp" => "cDFiJloPSX",
        "sftp" => "BbcDFiJloPRSsX",
        // autossh takes ssh's options plus its own `-M port`.
        "autossh" => "BbcDEeFIiJLlMmOoPpQRSWw",
        // ssh-copy-id hands its `-o` and `-F` to the ssh it runs.
        "ssh-copy-id" => "FJopt",
        _ => "BbcDEeFIiJLlmOoPpQRSWw",
    }
}

/// A program's name as a command line spells it: the basename, lower case, without `.exe`.
fn program_name(token: &str) -> String {
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let lower = base.to_ascii_lowercase();
    match lower.strip_suffix(".exe") {
        Some(stem) => stem.to_owned(),
        None => lower,
    }
}

/// `NAME=value` with a shell variable name, as `(NAME, value)`.
fn shell_assignment(token: &str) -> Option<(&str, &str)> {
    let (name, value) = token.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    ((first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then_some((name, value))
}

/// The tokens of a command line as a POSIX shell splits them, or by whitespace when the quoting
/// does not balance (so an unbalanced line still has its words graded).
fn shell_words(command: &str) -> Vec<String> {
    shlex::split(command).unwrap_or_else(|| command.split_whitespace().map(String::from).collect())
}

/// Strip one pair of matching quotes.
fn unquote(text: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// An ssh option as `-o` or a configuration line writes it (`Key=value`, `Key value`,
/// `Key = value`), as its lower-case key and its value.
fn ssh_setting(option: &str) -> Option<(String, &str)> {
    let option = unquote(option.trim()).trim();
    let at = option.find(|c: char| c == '=' || c.is_whitespace())?;
    let (key, rest) = option.split_at(at);
    let value = rest.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
    Some((key.to_ascii_lowercase(), unquote(value.trim()).trim()))
}

/// The setting `option` makes when it trusts an unchecked key, named as OpenSSH spells it:
/// `StrictHostKeyChecking` off, `KnownHostsCommand` set to a command, or a known-hosts file
/// other than the default. Punctuation a script leaves after a value (`no;`, `no)`) is not part
/// of the value.
fn trusting_setting(option: &str, env: &EnvFacts) -> Option<String> {
    let (key, value) = ssh_setting(option)?;
    let value = value.trim_end_matches([';', ')', ',']).trim();
    let lower = value.to_ascii_lowercase();
    match key.as_str() {
        "stricthostkeychecking" => TRUSTING_VALUES
            .contains(&lower.as_str())
            .then(|| format!("StrictHostKeyChecking={lower}")),
        // `none` is OpenSSH's own word for no command.
        "knownhostscommand" => {
            (!lower.is_empty() && lower != "none").then(|| "KnownHostsCommand".to_owned())
        }
        "userknownhostsfile" => (!default_known_hosts(value, USER_KNOWN_HOSTS, env))
            .then(|| "UserKnownHostsFile set to another file".to_owned()),
        "globalknownhostsfile" => (!default_known_hosts(value, GLOBAL_KNOWN_HOSTS, env))
            .then(|| "GlobalKnownHostsFile set to another file".to_owned()),
        _ => None,
    }
}

/// OpenSSH's default user known-hosts files.
const USER_KNOWN_HOSTS: &[&str] = &["~/.ssh/known_hosts", "~/.ssh/known_hosts2"];

/// OpenSSH's default global known-hosts files, on Unix and in Windows OpenSSH.
const GLOBAL_KNOWN_HOSTS: &[&str] = &[
    "/etc/ssh/ssh_known_hosts",
    "/etc/ssh/ssh_known_hosts2",
    r"C:\ProgramData\ssh\ssh_known_hosts",
    r"C:\ProgramData\ssh\ssh_known_hosts2",
];

/// Whether every file `value` names (a whitespace-separated list, as OpenSSH reads it) is one
/// of `defaults`, each compared as this computer resolves the path. An empty value leaves the
/// default in place. `none` and any other file are not the default.
fn default_known_hosts(value: &str, defaults: &[&str], env: &EnvFacts) -> bool {
    let resolve = |path: &str| {
        // `%d` is ssh's own token for the home directory, and Windows OpenSSH spells its data
        // directory `__PROGRAMDATA__`.
        let spelled = path.replace("%d", "~");
        let lower = spelled.to_ascii_lowercase();
        let spelled = ["__programdata__", "%programdata%"]
            .iter()
            .find_map(|token| {
                lower.starts_with(token).then(|| {
                    let rest = spelled.get(token.len()..).unwrap_or_default();
                    format!(r"C:\ProgramData{rest}")
                })
            })
            .unwrap_or(spelled);
        normalize_for(env.platform, &spelled, env).norm
    };
    let defaults: Vec<String> = defaults.iter().map(|path| resolve(path)).collect();
    value
        .split_whitespace()
        .all(|file| defaults.contains(&resolve(unquote(file))))
}

/// The value written after an option's name in free text, as far as a shell or `printf` would
/// end it: the first word, never past a quote, a backslash escape or a separator.
fn value_after(text: &str) -> &str {
    text.trim_start_matches(|c: char| c == '=' || c == '"' || c == '\'' || c.is_whitespace())
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';' | ')' | ','))
        .next()
        .unwrap_or_default()
}

/// The trusting setting `text` writes anywhere, in any spelling: the fallback for a `-F` file
/// or a wrapper program this inspection cannot read, which the same line may be writing.
fn mentions_trusting_setting(text: &str, env: &EnvFacts) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    HOST_KEY_OPTIONS.iter().find_map(|key| {
        lower.match_indices(key).find_map(|(at, _)| {
            // ASCII lower-casing keeps every byte where it was, so the original spelling of
            // the value (a path keeps its case) sits at the same offset.
            let value = value_after(text.get(at + key.len()..).unwrap_or_default());
            let value = if *key == "stricthostkeychecking" {
                value
                    .split(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
                    .next()
                    .unwrap_or_default()
            } else {
                value
            };
            trusting_setting(&format!("{key}={value}"), env)
        })
    })
}

/// The text of `file`, read only when it is a regular file and only its first
/// [`MAX_SSH_CONFIG_BYTES`]: a FIFO, a terminal or `/dev/stdin` would block the inspection until
/// something wrote to it.
fn read_small_file(file: &str) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::metadata(file)
        .and_then(|found| {
            if found.is_file() {
                std::fs::File::open(file)
            } else {
                Err(std::io::Error::other("not a regular file"))
            }
        })
        .and_then(|opened| opened.take(MAX_SSH_CONFIG_BYTES).read_to_string(&mut text))
        .ok()
        .map(|_| text)
}

/// One grading of a command line for criterion 7: the facts its paths resolve against, the
/// whole line (judged instead of a file it names that cannot be read, since the line may be
/// writing that file), and how many more files the grading may open.
struct TrustScan<'a> {
    env: &'a EnvFacts,
    raw: &'a str,
    files_left: std::cell::Cell<u8>,
}

impl<'a> TrustScan<'a> {
    fn new(env: &'a EnvFacts, raw: &'a str) -> Self {
        Self {
            env,
            raw,
            files_left: std::cell::Cell::new(MAX_TRUST_FILES),
        }
    }

    /// The setting by which the file `path` names trusts an unchecked key, with `line_trusts`
    /// grading each line that is not a comment. A file that cannot be read, or one past the
    /// budget, is judged by the whole command line instead.
    fn file_trusts(
        &self,
        path: &str,
        line_trusts: impl Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let file = normalize_for(self.env.platform, path, self.env).norm;
        let left = self.files_left.get();
        let text = (left > 0)
            .then(|| {
                self.files_left.set(left - 1);
                read_small_file(&file)
            })
            .flatten();
        let setting = match text {
            Some(text) => text
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with('#'))
                .find_map(line_trusts),
            None => mentions_trusting_setting(self.raw, self.env),
        };
        setting.map(|setting| format!("{setting} in {file}"))
    }
}

/// The setting by which one ssh option (an `-o` value or a configuration line) trusts an
/// unchecked key, directly or through a `ProxyCommand` that runs such an ssh.
fn option_trusts(option: &str, scan: &TrustScan<'_>, depth: u8) -> Option<String> {
    trusting_setting(option, scan.env).or_else(|| match ssh_setting(option) {
        Some((key, command)) if key == "proxycommand" => {
            ssh_command_trusts(command, scan, depth + 1)
        }
        _ => None,
    })
}

/// Every short option in `args` that takes a value, with its value, read as getopt reads a
/// client's arguments: clusters, a value attached (`-p22`) or in the next word, `--` ending
/// the options. `value_flags` is the client's options that take a value.
fn short_option_values(args: &[String], value_flags: &str) -> Vec<(char, String)> {
    let mut values = Vec::new();
    let mut at = 0;
    while at < args.len() {
        let token = &args[at];
        at += 1;
        if token == "--" {
            break;
        }
        let Some(cluster) = token.strip_prefix('-') else {
            continue;
        };
        if cluster.starts_with('-') {
            continue;
        }
        for (index, flag) in cluster.char_indices() {
            if !value_flags.contains(flag) {
                continue;
            }
            let attached = cluster.get(index + flag.len_utf8()..).unwrap_or_default();
            if !attached.is_empty() {
                values.push((flag, attached.to_owned()));
            } else if let Some(next) = args.get(at) {
                values.push((flag, next.clone()));
                at += 1;
            }
            break;
        }
    }
    values
}

/// Whether the ssh configuration file `path` (a `-F` argument) sets an option that trusts an
/// unchecked key, directly or through a `ProxyCommand`.
fn config_file_trusts(path: &str, scan: &TrustScan<'_>, depth: u8) -> Option<String> {
    scan.file_trusts(path, |line| option_trusts(line, scan, depth))
}

/// Whether the program `path`, run in ssh's place (a `GIT_SSH` value, `scp -S`, a remote shell
/// or `ProxyCommand` named by path), runs an ssh that trusts an unchecked key. An ssh client
/// adds nothing of its own. A wrapper script is read line by line, as a `-F` file is. A bare
/// name is a program found on the PATH, which this inspection does not open.
fn program_trusts(path: &str, scan: &TrustScan<'_>, depth: u8) -> Option<String> {
    let path = unquote(path.trim());
    if depth > MAX_TRUST_DEPTH
        || !path.contains(['/', '\\'])
        || SSH_CLIENTS.contains(&program_name(path).as_str())
    {
        return None;
    }
    scan.file_trusts(path, |line| ssh_command_trusts(line, scan, depth + 1))
}

/// The setting by which the ssh client `client`, run with `args`, would trust an unchecked
/// host key: a `-o` option, a `ProxyCommand` that runs such an ssh, a `-F` file, or the
/// program `scp -S` and `sftp -S` run in ssh's place.
fn ssh_args_trust(
    client: &str,
    args: &[String],
    scan: &TrustScan<'_>,
    depth: u8,
) -> Option<String> {
    for (flag, value) in short_option_values(args, ssh_value_flags(client)) {
        let found = match flag {
            'o' => option_trusts(&value, scan, depth),
            'F' => config_file_trusts(&value, scan, depth),
            // ssh's own `-S` is a control socket, not a program.
            'S' if matches!(client, "scp" | "sftp") => program_trusts(&value, scan, depth + 1),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The setting by which `command`, a command line handed to another program (`rsync -e`,
/// `GIT_SSH_COMMAND`, a `ProxyCommand`), runs an ssh client that trusts an unchecked key. A
/// command line that names no ssh client runs its first word, which is read as a wrapper.
fn ssh_command_trusts(command: &str, scan: &TrustScan<'_>, depth: u8) -> Option<String> {
    if depth > MAX_TRUST_DEPTH {
        return None;
    }
    let words = shell_words(command);
    match words
        .iter()
        .position(|word| SSH_CLIENTS.contains(&program_name(word).as_str()))
    {
        Some(at) => ssh_args_trust(&program_name(&words[at]), &words[at + 1..], scan, depth),
        None => first_value_word(words.iter().map(String::as_str))
            .and_then(|program| program_trusts(&program, scan, depth + 1)),
    }
}

/// What follows each mention of the environment variable `name` in `words`, in any shell's
/// spelling of setting it: POSIX `NAME=value` (also after `export` or `env`), PowerShell's
/// `$env:NAME = value` and `${env:NAME}`, `Set-Item env:NAME value`, cmd's `set NAME=value`
/// (whose value runs to the end of the line, unquoted) and `setx NAME value`. Each is the rest
/// of the mentioning word after the name, then every later word, so the value is in it however
/// the shell split it. `name` matches as a whole identifier in any letter case, so `GIT_SSH` is
/// never found inside `GIT_SSH_COMMAND`.
fn variable_values(words: &[String], name: &str) -> Vec<String> {
    let needle = name.to_ascii_lowercase();
    let outside = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
    let mut values = Vec::new();
    for (at, word) in words.iter().enumerate() {
        let lower = word.to_ascii_lowercase();
        for (start, _) in lower.match_indices(&needle) {
            let end = start + needle.len();
            let before = lower.get(..start).and_then(|text| text.chars().next_back());
            let after = lower.get(end..).and_then(|text| text.chars().next());
            if !outside(before) || !outside(after) {
                continue;
            }
            // ASCII lower-casing keeps every byte where it was.
            let mut value = word
                .get(end..)
                .unwrap_or_default()
                .trim_start_matches(|c: char| {
                    matches!(c, '}' | '=' | ':' | ',' | '"' | '\'') || c.is_whitespace()
                })
                .to_owned();
            for rest in &words[at + 1..] {
                value.push(' ');
                value.push_str(rest);
            }
            values.push(value);
        }
    }
    values
}

/// The first of `words` that is a value: past the `=` and `-Value` a PowerShell assignment
/// puts first, and without the punctuation a script leaves around it.
fn first_value_word<'w>(words: impl IntoIterator<Item = &'w str>) -> Option<String> {
    words
        .into_iter()
        .map(|word| word.trim_matches(|c: char| matches!(c, '=' | ',' | ';' | ')' | '"' | '\'')))
        .find(|word| !word.is_empty() && !word.eq_ignore_ascii_case("-value"))
        .map(str::to_owned)
}

/// The finding for an environment variable `words` sets that makes ssh trust an unchecked key:
/// a whole ssh command line (`GIT_SSH_COMMAND`, `RSYNC_RSH`), a wrapper program run in ssh's
/// place (`GIT_SSH`), or Ansible's host key checking turned off.
fn variables_trust_host_key(words: &[String], scan: &TrustScan<'_>) -> Option<String> {
    for name in SSH_COMMAND_VARIABLES {
        for value in variable_values(words, name) {
            if let Some(setting) = ssh_command_trusts(&value, scan, 1) {
                return Some(host_key_finding(&setting, name));
            }
        }
    }
    for name in SSH_PROGRAM_VARIABLES {
        for value in variable_values(words, name) {
            // A program path, not a shell command line: split as written, so a Windows path
            // keeps its backslashes.
            let program = first_value_word(value.split_whitespace());
            if let Some(setting) = program.and_then(|path| program_trusts(&path, scan, 1)) {
                return Some(host_key_finding(&setting, name));
            }
        }
    }
    variable_values(words, ANSIBLE_HOST_KEY_CHECKING)
        .iter()
        .find_map(|value| {
            let word = first_value_word(value.split_whitespace())?.to_ascii_lowercase();
            ANSIBLE_FALSE_VALUES.contains(&word.as_str()).then(|| {
                host_key_finding(&format!("{ANSIBLE_HOST_KEY_CHECKING}={word}"), "Ansible")
            })
        })
}

/// The command `mosh` runs to reach the server: `--ssh=CMD` or `--ssh CMD`.
fn mosh_ssh_command(args: &[String]) -> Option<String> {
    args.iter().enumerate().find_map(|(at, token)| {
        if let Some(command) = token.strip_prefix("--ssh=") {
            return Some(command.to_owned());
        }
        (token == "--ssh")
            .then(|| args.get(at + 1).cloned())
            .flatten()
    })
}

/// The options one sshfs `-o` value holds, as FUSE reads it: split at each comma, a backslash
/// keeping the character after it (`\,` is a comma inside an option).
fn fuse_options(value: &str) -> Vec<String> {
    let mut options = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        let kept = match c {
            ',' => {
                options.push(String::new());
                continue;
            }
            '\\' => chars.next().unwrap_or('\\'),
            c => c,
        };
        if let Some(option) = options.last_mut() {
            option.push(kept);
        }
    }
    options.retain(|option| !option.trim().is_empty());
    options
}

/// The finding for sshfs run with `args`: an ssh option among its `-o` options (sshfs hands
/// every one it does not know to the ssh it runs, a `ProxyCommand` included), the command
/// `-o ssh_command=CMD` runs in ssh's place, or a `-F` configuration file.
fn sshfs_trusts(args: &[String], scan: &TrustScan<'_>) -> Option<String> {
    for (flag, value) in short_option_values(args, SSHFS_VALUE_FLAGS) {
        let found = match flag {
            'o' => fuse_options(&value)
                .iter()
                .find_map(|option| match option.split_once('=') {
                    Some((key, command)) if key.trim().eq_ignore_ascii_case("ssh_command") => {
                        ssh_command_trusts(command, scan, 1)
                            .map(|setting| host_key_finding(&setting, "sshfs -o ssh_command"))
                    }
                    _ => option_trusts(option, scan, 0)
                        .map(|setting| host_key_finding(&setting, "sshfs")),
                }),
            'F' => config_file_trusts(&value, scan, 0)
                .map(|setting| host_key_finding(&setting, "sshfs")),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The command `rsync` would run as its remote shell: `-e CMD`, `-eCMD` (also inside a cluster
/// such as `-avze`), `--rsh=CMD` or `--rsh CMD`.
fn rsync_remote_shell(args: &[String]) -> Option<String> {
    for (at, token) in args.iter().enumerate() {
        if let Some(command) = token.strip_prefix("--rsh=") {
            return Some(command.to_owned());
        }
        if token == "--rsh" {
            return args.get(at + 1).cloned();
        }
    }
    // rsync's short options that take a value, so `-B1024e` is not read as `-e`.
    short_option_values(args, "BeMfT")
        .into_iter()
        .find_map(|(flag, value)| (flag == 'e').then_some(value))
}

/// The ssh command line git is told to use: `-c core.sshCommand=CMD`, or
/// `git config [scope] core.sshCommand CMD`, which keeps it.
fn git_ssh_command(args: &[String]) -> Option<String> {
    const KEY: &str = "core.sshcommand";
    for (at, token) in args.iter().enumerate() {
        let lower = token.to_ascii_lowercase();
        if let Some(command) = lower
            .strip_prefix("-c")
            .unwrap_or(lower.as_str())
            .strip_prefix(KEY)
            .and_then(|rest| rest.strip_prefix('='))
        {
            // The original spelling: only the key was lower-cased to match, and ASCII
            // lower-casing keeps every byte where it was.
            let start = token.len() - command.len();
            return token.get(start..).map(str::to_owned);
        }
        if lower == KEY {
            return args.get(at + 1).cloned();
        }
    }
    None
}

/// Whether `ssh-keygen` run with `args` removes a host's key from a known-hosts file (`-R`),
/// read as its getopt reads clusters and values.
fn removes_known_host(args: &[String]) -> bool {
    // ssh-keygen's options that take a value; `R` is among them and is the one asked about.
    const VALUE_FLAGS: &str = "abCDEFfGIJjKMmNnOPRrstVwYZz";
    short_option_values(args, VALUE_FLAGS)
        .iter()
        .any(|(flag, _)| *flag == 'R')
}

/// The finding for an ssh client, or a program that hands one a command line, run as `words`
/// in one command: what it would trust and how.
fn words_trust_host_key(words: &[String], scan: &TrustScan<'_>) -> Option<String> {
    if let Some(found) = variables_trust_host_key(words, scan) {
        return Some(found);
    }
    let at = words.iter().position(|word| {
        shell_assignment(word).is_none() && HOST_KEY_PROGRAMS.contains(&program_name(word).as_str())
    })?;
    let program = program_name(&words[at]);
    let args = &words[at + 1..];
    match program.as_str() {
        "ssh-keygen" => removes_known_host(args).then(|| {
            "removes a server's trusted host key (ssh-keygen -R), so the next connection would take whatever key it is offered".to_owned()
        }),
        "rsync" => rsync_remote_shell(args)
            .and_then(|shell| ssh_command_trusts(&shell, scan, 1))
            .map(|setting| host_key_finding(&setting, "rsync -e")),
        "git" => git_ssh_command(args)
            .and_then(|command| ssh_command_trusts(&command, scan, 1))
            .map(|setting| host_key_finding(&setting, "core.sshCommand")),
        "mosh" => mosh_ssh_command(args)
            .and_then(|command| ssh_command_trusts(&command, scan, 1))
            .map(|setting| host_key_finding(&setting, "mosh --ssh")),
        sshfs if SSHFS_PROGRAMS.contains(&sshfs) => sshfs_trusts(args, scan),
        client => {
            ssh_args_trust(client, args, scan, 0).map(|setting| host_key_finding(&setting, client))
        }
    }
}

/// The finding phrase for criterion 7: what would be trusted, and by which setting.
fn host_key_finding(setting: &str, through: &str) -> String {
    format!("lets ssh trust a server's host key nobody checked ({setting}, through {through})")
}

/// Criterion 7: every way `command` makes ssh trust a server's host key nobody checked. Each
/// command of the line is graded as the parser splits it (pipes, `&&`, `sh -c` scripts), and
/// the line's own words once more, because a wrapper (`env NAME=… git …`) hands an
/// assignment over before the parser keeps the command's words.
fn command_trusts_host_key(command: &str, env: &EnvFacts, found: &mut Findings) {
    let parsed = ParsedCommand::parse_for(command, env.platform, env);
    let whole = shell_words(command);
    let graded = parsed
        .segments
        .iter()
        .map(|segment| segment.argv.as_slice())
        .chain(std::iter::once(whole.as_slice()));
    let scan = TrustScan::new(env, command);
    for words in graded {
        if let Some(reason) = words_trust_host_key(words, &scan) {
            if found.push(reason) {
                return;
            }
        }
    }
}

/// Every criterion a shell command line can trip: the four path-classification
/// ones (via [`command_writes_sensitively`]), criterion 7 (a host key trusted
/// unchecked) and criterion 5 — and every hit of each, not the first.
fn command_findings(
    command: &str,
    env: &EnvFacts,
    session: &CallSession<'_>,
    found: &mut Findings,
) {
    command_writes_sensitively(command, env, found);
    if found.is_full() {
        return;
    }
    command_trusts_host_key(command, env, found);
    if found.is_full() {
        return;
    }
    command_deletes_established_directory(command, env, session, found);
}

/// Read the JS string / template literal that opens at `chars[start]` (a `"`,
/// `'` or backtick), returning its raw inner text and the index just past the
/// closing quote (or the end of input for an unterminated literal). Escapes are
/// kept verbatim (a `\n` stays two characters, never a real newline) so an
/// embedded path token is preserved exactly.
///
/// The one string scanner in this module: every other walker delegates here, so
/// a parenthesis, colon or brace *inside* a literal can never be mistaken for
/// program syntax by one walker while another skips it.
fn read_string_literal(chars: &[char], start: usize) -> (String, usize) {
    let quote = chars[start];
    let mut i = start + 1;
    let mut cur = String::new();
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            cur.push(ch);
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if ch == quote {
            i += 1;
            break;
        }
        cur.push(ch);
        i += 1;
    }
    (cur, i)
}

/// Extract the raw inner text of every JS string / template literal in a script.
/// Best-effort: interpolation (`${…}`) and other dynamic construction are not
/// resolved (the documented gap).
fn extract_string_literals(code: &str) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '"' | '\'' | '`') {
            let (lit, next) = read_string_literal(&chars, i);
            out.push(lit);
            i = next;
        } else {
            i += 1;
        }
    }
    out
}

/// True for a character that may appear in a JS identifier.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Index of the `)` closing the `(` at `open`, or `None` when unbalanced.
/// String literals are skipped, so a parenthesis inside one never shifts depth.
fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' | '`' => {
                let (_, next) = read_string_literal(chars, i);
                i = next;
                continue;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The string literals of a `[…]` array starting at `chars[open]`, plus the index
/// just past its `]`. Only top-level elements are collected.
fn read_string_array(chars: &[char], open: usize) -> (Vec<String>, usize) {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' | '`' => {
                let (lit, next) = read_string_literal(chars, i);
                if depth == 1 {
                    items.push(lit);
                }
                i = next;
                continue;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return (items, i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (items, chars.len())
}

/// One inner tool call recovered from an `execute_code` body: the callee name
/// and the arguments that are *statically literal enough to classify*. A value
/// built at runtime (a variable, an interpolation, a computed expression) is
/// simply absent from the map rather than guessed at — the documented gap.
struct InnerCall {
    callee: String,
    args: Map<String, Value>,
}

/// The `key: "value"` / `key: ["a", "b"]` pairs of one call's argument span.
///
/// A pair inside a **nested parenthesized subexpression** is skipped: it belongs
/// to that expression, not to this call. That containment rule is the fix for
/// issue #106 — in `kb_write_page({ path: page.path.split("/").pop() })` the
/// utility literal `"/"` sits inside `split(…)`, so it is never offered as this
/// call's write target. (The nested call is still classified in its own right;
/// [`extract_inner_calls`] visits it separately.)
fn named_string_args(span: &[char]) -> Map<String, Value> {
    let mut out = Map::new();
    let mut i = 0;
    while i < span.len() {
        if span[i] == '(' {
            let Some(end) = matching_paren(span, i) else {
                break;
            };
            i = end + 1;
            continue;
        }
        // One token: a quoted key, a bare identifier key, or neither.
        let (token, after) = if matches!(span[i], '"' | '\'' | '`') {
            let (lit, next) = read_string_literal(span, i);
            (Some(lit), next)
        } else if is_ident_char(span[i]) {
            let start = i;
            let mut end = i;
            while end < span.len() && is_ident_char(span[end]) {
                end += 1;
            }
            (Some(span[start..end].iter().collect::<String>()), end)
        } else {
            (None, i + 1)
        };
        let Some(key) = token else {
            i = after;
            continue;
        };
        let mut j = after;
        while j < span.len() && span[j].is_whitespace() {
            j += 1;
        }
        if j >= span.len() || span[j] != ':' {
            i = after; // not a key after all
            continue;
        }
        j += 1;
        while j < span.len() && span[j].is_whitespace() {
            j += 1;
        }
        if j >= span.len() {
            break;
        }
        match span[j] {
            '"' | '\'' | '`' => {
                let (lit, next) = read_string_literal(span, j);
                out.insert(key, Value::String(lit));
                i = next;
            }
            '[' => {
                let (items, next) = read_string_array(span, j);
                if !items.is_empty() {
                    out.insert(
                        key,
                        Value::Array(items.into_iter().map(Value::String).collect()),
                    );
                }
                i = next;
            }
            // A dynamic value: resume *at* the expression so any call inside it
            // is still seen, and so its own parentheses are skipped wholesale.
            _ => i = j,
        }
    }
    out
}

/// Every `callee(…)` call site in a script, paired with the named literal
/// arguments it was given.
///
/// Scanning continues *inside* each argument span, so a nested call
/// (`record_result(text_editor({…}))`) is recovered as a call in its own right.
fn extract_inner_calls(code: &str) -> Vec<InnerCall> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '"' | '\'' | '`') {
            let (_, next) = read_string_literal(&chars, i);
            i = next;
            continue;
        }
        if !is_ident_char(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_ident_char(chars[i]) {
            i += 1;
        }
        let mut j = i;
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= chars.len() || chars[j] != '(' {
            continue;
        }
        let Some(end) = matching_paren(&chars, j) else {
            continue;
        };
        out.push(InnerCall {
            callee: chars[start..i].iter().collect(),
            args: named_string_args(&chars[j + 1..end]),
        });
        // Deliberately do NOT skip past `end`: a call nested in the argument
        // span is a call site of its own and must be classified too.
    }
    out
}

/// A mutating file operation — named tool plus its arguments — whose target path
/// is sensitive. The single classifier behind both the outer tool call and the
/// inner calls recovered from an `execute_code` body, so an inner call is graded
/// **exactly** as the same call made directly would be, and the two can never
/// drift apart.
fn mutating_path_writes(
    tool_name: &str,
    args: &Map<String, Value>,
    env: &EnvFacts,
    found: &mut Findings,
) {
    if !operation_is_mutating(tool_name, args) {
        return;
    }
    for raw in path_values(args) {
        if raw.trim().is_empty() {
            continue;
        }
        let tp = normalize_for(env.platform, &raw, env);
        if let Some(reason) = sensitivity_reason(&tp, env) {
            if found.push(write_finding(&tp.norm, reason)) {
                return;
            }
        }
    }
}

/// Scan an `execute_code` JS body for a sensitive operation. Two independent
/// passes,
/// each of which associates a candidate path with the **concrete call that would
/// mutate it** rather than with the script as a whole:
///
///  1. *Embedded shell command lines.* Every string literal is tried as a
///     command line; only a redirect target or the path argument of a mutating
///     binary counts, so a literal that is not a write is inert. The scan stays
///     script-wide on purpose — a command is routinely assembled into a variable
///     (`const cmd = "… >> ~/.ssh/config"; shell({command: cmd})`) and would
///     escape a call-scoped scan.
///  2. *Inner tool calls.* Each `callee({…})` site is graded by
///     [`mutating_path_writes`], i.e. exactly as the same tool call made directly
///     would be: the callee must read as a mutation, and only its own
///     path-keyed arguments are candidate targets.
///
/// Pass 2 replaces a lexical cross-product that flagged issue #106: the old scan
/// enabled itself whenever *any* write-ish substring appeared anywhere in the
/// source, and then offered *every* literal in the script as a filesystem
/// target. A KB page write (`kb_write_page`, whose name contains "write") beside
/// an unrelated `page.path.split("/")` therefore normalized the utility literal
/// `"/"` to the filesystem root and parked Auto mode on an approval prompt.
fn code_findings(code: &str, env: &EnvFacts, session: &CallSession<'_>, found: &mut Findings) {
    for lit in extract_string_literals(code) {
        command_findings(&lit, env, session, found);
        if found.is_full() {
            return;
        }
    }
    for call in extract_inner_calls(code) {
        mutating_path_writes(&call.callee, &call.args, env, found);
        if found.is_full() {
            return;
        }
    }
}

/// Classify a tool call: `Some(reason)` when it is an extremely sensitive
/// system operation that must be approved even in Auto mode; `None` otherwise.
///
/// Inspects three shapes (see the module boundary docs): file-editor path
/// arguments, shell command lines, and the `execute_code` JS body. Reads the
/// host environment (`$HOME`, cwd) via [`EnvFacts::host`]; path canonicalization
/// itself never touches the filesystem.
///
/// Criterion 5 is the one exception, and deliberately so: "an existing,
/// non-empty directory" is not a property of the command text, so a command line
/// naming a recursive delete earns one bounded look at the target on disk. That
/// look is reached only *after* a delete target has been parsed out, so an
/// ordinary call still costs no I/O.
pub fn sensitive_file_operation(
    tool_name: &str,
    args: &Map<String, Value>,
    session: &CallSession<'_>,
) -> Option<String> {
    sensitive_file_operations(tool_name, args, session)
        .into_reasons()
        .into_iter()
        .next()
}

/// [`sensitive_file_operation`], but EVERY sensitive operation the call would
/// perform, in the order the call performs them.
///
/// This is what the approval card is built from. The singular form above is the
/// yes/no question — "must this call be approved at all" — and stays because
/// dozens of tests, and the criteria's own documentation, are phrased that way.
pub fn sensitive_file_operations(
    tool_name: &str,
    args: &Map<String, Value>,
    session: &CallSession<'_>,
) -> Findings {
    let env = EnvFacts::host(&session.working_dir().to_string_lossy());
    let mut found = Findings::default();

    // 1. File-editor / file-tool path arguments (mutations only).
    mutating_path_writes(tool_name, args, &env, &mut found);

    // 2. Shell command lines (developer/shell and any command-bearing tool).
    if !found.is_full() {
        if let Some(command) = command_text_from(tool_name, args) {
            command_findings(&command, &env, session, &mut found);
        }
    }

    // 3. code_execution/execute_code JS body — its inner tool calls bypass every
    //    agent-layer inspector, so scan the script itself.
    if !found.is_full() && is_execute_code(tool_name) {
        if let Some(code) = args.get("code").and_then(Value::as_str) {
            code_findings(code, &env, session, &mut found);
        }
    }

    found
}

/// This inspector's name, as it appears on an [`InspectionResult`]. Named so
/// [`crate::tool_inspection::NON_DELEGABLE_APPROVAL_INSPECTORS`] cannot drift
/// from it.
pub const SENSITIVE_OPS_INSPECTOR_NAME: &str = "sensitive_ops";

/// Inspector that, **in Auto mode only**, escalates the sensitive-operation set
/// to the standard approval flow. See the module docs for the policy.
pub struct SensitiveOpsInspector;

impl SensitiveOpsInspector {
    #[must_use]
    pub fn new(_provider: SharedProvider) -> Self {
        Self
    }

    #[must_use]
    pub fn unbound() -> Self {
        Self
    }

    /// The whole gate, over one batch. Both trait entry points funnel here, so
    /// the capability-carrying one can never grow behaviour the plain one lacks.
    async fn inspect_batch(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
        _capability: Option<CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        // Inert outside Auto — every other mode already gates these operations,
        // so this keeps non-Auto behaviour provably unchanged (one early return).
        // Criterion 5 sits behind it too: `Approve` prompts for every tool call,
        // `SmartApprove` grades these as non-read, and `Chat` runs no tools.
        if biorouter_mode != BioRouterMode::Auto {
            return Ok(vec![]);
        }

        // One `CallSession` for the whole batch: the directory-provenance scan
        // behind criterion 5 is memoized on it, so a conversation is walked at
        // most once per inspection no matter how many calls the model made.
        let request_ids: Vec<String> = tool_requests.iter().map(|r| r.id.clone()).collect();
        let call_session = CallSession::new(
            &session.working_dir,
            session_started_at(session),
            messages,
            &request_ids,
        );

        let mut results = Vec::new();
        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            if crate::security::computer_use::is_computer_use_tool(&tool_call.name) {
                continue;
            }
            let Some(args) = tool_call.arguments.as_ref() else {
                continue;
            };
            let found = sensitive_file_operations(&tool_call.name, args, &call_session);
            if !found.is_empty() {
                let summary = found.summary();
                tracing::warn!(
                    counter.biorouter.sensitive_op_escalated = 1,
                    tool_name = %tool_call.name,
                    tool_request_id = %request.id,
                    %summary,
                    "Sensitive file operation escalated to approval in Auto mode"
                );
                // The card names EVERY operation it found, not the first. A script
                // that recursively deletes six directories used to be described by
                // one of them, so the user approved five deletions the card had not
                // mentioned — an approval that does not describe its own blast
                // radius is not consent.
                let affected = found.describe();
                results.push(InspectionResult {
                    tool_request_id: request.id.clone(),
                    action: InspectionAction::RequireApproval(Some(format!(
                        "🔒 Sensitive system operation in Fully-Automatic mode.\n\
                         {affected}\n\
                         Approve it to continue, or deny it. Ordinary file changes \
                         run without a prompt in this mode."
                    ))),
                    reason: summary,
                    confidence: 1.0,
                    inspector_name: self.name().to_string(),
                    finding_id: Some(format!("SENS-{}", Uuid::new_v4().simple())),
                });
            }
        }

        Ok(results)
    }
}

#[async_trait]
impl ToolInspector for SensitiveOpsInspector {
    fn name(&self) -> &'static str {
        SENSITIVE_OPS_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_batch(tool_requests, messages, biorouter_mode, session, None)
            .await
    }

    async fn inspect_with_capability(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
        capability: Option<CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_batch(tool_requests, messages, biorouter_mode, session, capability)
            .await
    }

    // `is_enabled` uses the trait default (always registered): the mode gate
    // lives in `inspect_batch`, so a mid-session mode change is honoured without
    // re-plumbing the inspector list.
}

/// ⚠ WHY 25 TESTS BELOW ARE `#[cfg(not(target_os = "windows"))]`.
///
/// They spell their fixtures as POSIX command lines — `rm -rf <path>` — and
/// assert what the POSIX arm of [`is_recursive_delete`] does with them. On
/// Windows that assertion is about a command that is never parsed: production
/// picks the dialect with `Platform::default_dialect()`, which is
/// `Dialect::PowerShell` there, and PowerShell reads `rm` as an alias of
/// `Remove-Item` whose recursive switch is `-Recurse`, not `-rf`. So the parser
/// correctly finds no recursive delete, every one of those tests fails, and the
/// failure is the FIXTURE being POSIX rather than the gate being broken.
///
/// This was mis-diagnosed once already, so it is worth stating plainly: these
/// failures are NOT evidence that the recursive-delete gate is inert on
/// Windows. The Windows path through this criterion is `Remove-Item -Recurse`,
/// and it needs its own fixtures rather than a `cfg` — that is a real coverage
/// gap and it is tracked, but it is a gap in the TESTS, not in the gate.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::policy::target::Platform;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn nix_env() -> EnvFacts {
        EnvFacts::for_platform(Platform::Linux, "/home/me/proj", "/home/me")
    }

    /// A session with a working directory and no history — the shape every
    /// pre-criterion-5 test was written against.
    fn test_session() -> CallSession<'static> {
        CallSession::workspace_only(Path::new("/home/me/proj"))
    }

    // --- path classification (pure) ---------------------------------------

    #[test]
    fn system_dirs_are_sensitive() {
        let env = nix_env();
        for raw in ["/etc/passwd", "/usr/local/bin/x", "/boot/grub", "/"] {
            let tp = normalize_for(Platform::Linux, raw, &env);
            assert!(
                sensitivity_reason(&tp, &env).is_some(),
                "{raw} must be sensitive"
            );
        }
    }

    #[test]
    fn macos_system_and_credential_dirs_are_sensitive() {
        let env = EnvFacts::for_platform(Platform::Macos, "/Users/me/proj", "/Users/me");
        for raw in [
            "/Library/LaunchDaemons/evil.plist",
            "/System/Library/x",
            "~/.ssh/authorized_keys",
            "~/Library/Keychains/login.keychain-db",
            "~/Library/Application Support/Google/Chrome/Default/Login Data",
            "~/.aws/credentials",
        ] {
            let tp = normalize_for(Platform::Macos, raw, &env);
            assert!(
                sensitivity_reason(&tp, &env).is_some(),
                "{raw} must be sensitive (norm={})",
                tp.norm
            );
        }
    }

    #[test]
    fn ordinary_and_temp_targets_are_not_sensitive() {
        let env = nix_env();
        for raw in [
            "/home/me/proj/out.csv",
            "~/Downloads/report.pdf",
            "/home/me/Documents/notes.txt",
            "./build/index.html",
            "/tmp/scratch.txt",
            "/var/folders/ab/T/tmp123/x.bin", // macOS TMPDIR shape, still ordinary
        ] {
            let tp = normalize_for(Platform::Linux, raw, &env);
            assert!(
                sensitivity_reason(&tp, &env).is_none(),
                "{raw} must be ordinary (norm={})",
                tp.norm
            );
        }
    }

    // --- mutation detection -----------------------------------------------

    /// The verb can live in an ARGUMENT instead of the tool's name, and then a
    /// name-only rule cannot see it. `webdocuments__cache` is the shipped case:
    /// `{command: "delete", path: …}` calls `fs::remove_file` on that path, so
    /// in Auto mode the file went without an approval card ever being built.
    #[test]
    fn a_verb_in_an_argument_is_a_mutation_even_when_the_name_is_inert() {
        assert!(
            operation_is_mutating(
                "webdocuments__cache",
                &args(json!({"command": "delete", "path": "~/.ssh/id_rsa"}))
            ),
            "cache delete must read as a mutation"
        );
        for read_only in [
            json!({"command": "view", "path": "~/.ssh/id_rsa"}),
            json!({"command": "list"}),
        ] {
            assert!(
                !operation_is_mutating("webdocuments__cache", &args(read_only.clone())),
                "{read_only} must stay read-only"
            );
        }
    }

    /// ⚠ The coverage this replaced was an ACCIDENT, which is the whole reason
    /// moving the tool removed it silently.
    ///
    /// `cache` was `computercontroller__cache` until the Computer Use (now
    /// Biorouter Copilot) change,
    /// and that name matches [`MUTATING_NAME_HINTS`] — not because anything in
    /// it is a verb, but because `"put"` is a substring of `"com·put·er"`. So
    /// EVERY `computercontroller__*` tool graded as mutating, `cache delete`
    /// among them, and the gate appeared to work. Renaming the server to
    /// `webdocuments` dropped the letters and the gate with them.
    ///
    /// Pinned so nobody re-derives the coverage from the name. A rule that
    /// holds only while a product name happens to contain three letters is not
    /// a rule.
    #[test]
    fn the_old_names_matched_the_mutating_hints_only_by_accident() {
        assert!(MUTATING_NAME_HINTS.contains(&"put"));
        assert!(
            "computercontroller__cache".contains("put"),
            "the old coverage came from com-PUT-er, not from a verb"
        );
        assert!(
            !"webdocuments__cache".contains("put"),
            "the new name has no accidental hint, which is what exposed the gap"
        );
        // Both names must now be graded on the ARGUMENT, not on their letters.
        for tool in ["webdocuments__cache", "computercontroller__cache"] {
            assert!(operation_is_mutating(
                tool,
                &args(json!({"command": "delete", "path": "~/x"}))
            ));
        }
    }

    /// ⚠ A command LINE is not a verb. `developer__shell` also carries
    /// `command`, and its value is graded as a shell command by
    /// `command_findings`. Admitting whitespace here would grade one string
    /// twice under two different meanings, and the false positives land as
    /// approval prompts in Auto mode — the failure mode that gets a gate
    /// switched off rather than obeyed.
    #[test]
    fn a_command_line_is_not_read_as_a_verb() {
        for line in ["rm -rf /tmp/x", "cp a b", "git remote -v", "echo move"] {
            assert!(
                !operation_is_mutating("developer__shell", &args(json!({"command": line}))),
                "{line:?} must not be graded as a verb by the name rule"
            );
        }
    }

    #[test]
    fn editor_view_is_not_a_mutation_but_writes_are() {
        assert!(!operation_is_mutating(
            "developer__text_editor",
            &args(json!({"command": "view", "path": "/etc/hosts"}))
        ));
        for cmd in ["write", "str_replace", "insert", "undo_edit"] {
            assert!(
                operation_is_mutating(
                    "developer__text_editor",
                    &args(json!({"command": cmd, "path": "/etc/hosts"}))
                ),
                "{cmd} is a mutation"
            );
        }
    }

    #[test]
    fn file_tool_names_classify_by_verb() {
        assert!(operation_is_mutating(
            "files__write_file",
            &args(json!({"path": "x"}))
        ));
        assert!(operation_is_mutating(
            "files__delete_file",
            &args(json!({"path": "x"}))
        ));
        assert!(operation_is_mutating(
            "knowledge__kb_export",
            &args(json!({"dest_path": "x"}))
        ));
        assert!(operation_is_mutating(
            "knowledge__kb_import",
            &args(json!({"src_path": "x"}))
        ));
        assert!(!operation_is_mutating(
            "files__read_file",
            &args(json!({"path": "x"}))
        ));
        assert!(!operation_is_mutating(
            "files__list_dir",
            &args(json!({"path": "x"}))
        ));
    }

    // --- the public entry point -------------------------------------------

    // Unix-only: uses the host-platform classifier with the Unix system path
    // `/etc`, which is a system dir only on Unix. On Windows `/etc` is an ordinary
    // path, so this asserts Unix behaviour and runs on Unix. (Windows system-dir
    // handling is covered by the policy layer's platform-forced tests.)
    #[cfg(unix)]
    #[test]
    fn editor_write_to_system_dir_is_flagged() {
        let finding = sensitive_file_operation(
            "developer__text_editor",
            &args(json!({"command": "write", "path": "/etc/cron.d/backdoor"})),
            &test_session(),
        );
        assert!(finding.is_some(), "write to /etc must be flagged");
    }

    #[test]
    fn editor_view_of_system_dir_is_not_flagged() {
        let finding = sensitive_file_operation(
            "developer__text_editor",
            &args(json!({"command": "view", "path": "/etc/hosts"})),
            &test_session(),
        );
        assert!(finding.is_none(), "reading /etc must NOT be flagged");
    }

    #[test]
    fn editor_write_to_ordinary_path_is_not_flagged() {
        let finding = sensitive_file_operation(
            "developer__text_editor",
            &args(json!({"command": "write", "path": "/tmp/qa/hi.txt"})),
            &test_session(),
        );
        assert!(
            finding.is_none(),
            "an ordinary tmp write must not be flagged"
        );
    }

    /// Issue #108. `platform__ingest_source` is the one first-class ingestion
    /// operation, and an explicit "ingest these PDFs" in Auto mode must not earn
    /// a generic filesystem prompt on top of the request the user already made.
    ///
    /// It reads as ordinary here because the classifier asks the right question:
    /// the tool's name carries no mutating verb, so its `paths` never become
    /// candidate write targets. That is a property worth pinning rather than
    /// assuming — a rename to something like `write_sources` would silently
    /// start escalating every ingest, and nothing else in the tree would notice.
    #[test]
    fn the_source_ingest_tool_is_not_a_filesystem_mutation() {
        for arguments in [
            json!({"paths": ["/Users/me/papers/ms-2024.pdf"], "kb_id": "ms-papers"}),
            json!({"path": "/Users/me/papers", "kb_id": "ms-papers"}),
            json!({"sources": ["~/Downloads/a.pdf", "https://example.org/b.pdf"]}),
        ] {
            let finding = sensitive_file_operation(
                crate::agents::platform_tools::PLATFORM_INGEST_SOURCE_TOOL_NAME,
                &args(arguments.clone()),
                &test_session(),
            );
            assert!(
                finding.is_none(),
                "ingesting documents must not request approval in Auto mode ({arguments}), \
                 got {finding:?}"
            );
        }
    }

    // --- the inspector, end to end (the directive's gates) -----------------

    use crate::conversation::message::ToolRequest;
    use rmcp::model::{CallToolRequestParams, CallToolResult};

    fn write_request(id: &str, path: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "developer__text_editor".into(),
                arguments: Some(args(json!({ "command": "write", "path": path }))),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    /// Gate (b): in Auto mode a sensitive-path write is routed to approval.
    /// Unix-only: `/etc` is a host system dir only on Unix.
    #[cfg(unix)]
    #[tokio::test]
    async fn auto_mode_routes_sensitive_write_to_approval() {
        let inspector = SensitiveOpsInspector::unbound();
        let requests = vec![write_request("req_sys", "/etc/cron.d/backdoor")];
        let results = inspector
            .inspect(
                &requests,
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        let r = results
            .iter()
            .find(|r| r.tool_request_id == "req_sys")
            .expect("sensitive write should produce a result");
        assert!(matches!(r.action, InspectionAction::RequireApproval(_)));
        assert_eq!(r.inspector_name, "sensitive_ops");
        assert!(r.finding_id.as_deref().unwrap_or("").starts_with("SENS-"));
    }

    /// Gate (a): in Auto mode an ordinary out-of-workspace write yields NO
    /// escalation, so the permission inspector's Auto `Allow` stands (no prompt).
    #[tokio::test]
    async fn auto_mode_leaves_ordinary_write_unescalated() {
        let inspector = SensitiveOpsInspector::unbound();
        // Absolute, outside the session working dir, but ordinary.
        let requests = vec![write_request("req_ok", "/tmp/qa-r1b/hi.txt")];
        let results = inspector
            .inspect(
                &requests,
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "an ordinary write must not be escalated, got {results:?}"
        );
    }

    // --- shell command lines (R2-01) --------------------------------------

    #[test]
    fn shell_redirect_into_ssh_config_is_flagged() {
        let env = nix_env();
        // The exact demonstrated exploit: a silent append to ~/.ssh/config.
        let hit = command_writes_sensitively("echo 'Host x' >> ~/.ssh/config", &env);
        assert!(
            hit.is_some(),
            "append-redirect into ~/.ssh/config must be flagged"
        );
    }

    #[test]
    fn shell_mutating_binary_into_sensitive_target_is_flagged() {
        let env = nix_env();
        for cmd in [
            "cp ./key /etc/cron.d/backdoor",
            "tee /etc/hosts",
            "install -m 600 k ~/.ssh/authorized_keys",
            "mv ./x ~/.aws/credentials",
        ] {
            assert!(
                command_writes_sensitively(cmd, &env).is_some(),
                "{cmd} must be flagged"
            );
        }
    }

    #[test]
    fn shell_reads_of_sensitive_paths_are_not_flagged() {
        let env = nix_env();
        for cmd in [
            "cat /etc/hosts",
            "less ~/.ssh/config",
            "grep x /etc/passwd",
            "ls -la /etc",
        ] {
            assert!(
                command_writes_sensitively(cmd, &env).is_none(),
                "{cmd} is a read, must not be flagged"
            );
        }
    }

    #[test]
    fn shell_ordinary_writes_are_not_flagged() {
        let env = nix_env();
        for cmd in [
            "echo hi > /tmp/x",
            "cp a /home/me/Downloads/b",
            "tee /home/me/proj/out.txt",
        ] {
            assert!(
                command_writes_sensitively(cmd, &env).is_none(),
                "{cmd} must not be flagged"
            );
        }
    }

    /// Redirects to `/dev/null` (and the other benign character devices) are the
    /// universal bit-bucket — never a sensitive write. Regression: a routine
    /// `… 2>/dev/null` used to trip the `/dev` system-dir escalation and park the
    /// agent on an approval prompt even in Auto mode.
    #[test]
    fn shell_redirect_to_dev_null_is_not_flagged() {
        let env = nix_env();
        for cmd in [
            "find landing -type f 2>/dev/null | sort",
            "git status --short 2>/dev/null",
            "grep -r foo . >/dev/null 2>&1",
            "make >/dev/null",
            "cat huge.log > /dev/null",
        ] {
            assert!(
                command_writes_sensitively(cmd, &env).is_none(),
                "{cmd} redirects to /dev/null and must NOT be flagged"
            );
        }
    }

    #[test]
    fn dev_null_and_std_streams_are_not_sensitive_editor_targets() {
        for path in [
            "/dev/null",
            "/dev/stdout",
            "/dev/stderr",
            "/dev/tty",
            "/dev/fd/2",
        ] {
            let finding = sensitive_file_operation(
                "developer__text_editor",
                &args(json!({ "command": "write", "path": path })),
                &test_session(),
            );
            assert!(
                finding.is_none(),
                "a write target of {path} (a benign device) must NOT be flagged"
            );
        }
    }

    /// A real `/etc` write must still be caught — the exemption is scoped to the
    /// benign devices only, not all of `/dev` or the rest of the system tree.
    #[test]
    fn dev_null_exemption_does_not_leak_to_other_system_paths() {
        let env = nix_env();
        assert!(
            command_writes_sensitively("echo x > /dev/sda", &env).is_some(),
            "a write to a real block device must still be flagged"
        );
        assert!(
            command_writes_sensitively("echo x > /etc/passwd", &env).is_some(),
            "a write to /etc must still be flagged"
        );
    }

    /// The observed drive stall: an `execute_code` body whose only sensitive-looking
    /// token is a `2>/dev/null` redirect (either live or embedded in authored code)
    /// must run without an approval prompt.
    #[test]
    fn execute_code_with_dev_null_redirect_is_not_flagged() {
        let env = nix_env();
        let code = r#"import { shell, text_editor } from "developer";
const repo = "/home/me/proj";
const status = shell({ command: `cd ${repo} && git status --short && find landing -type f -print 2>/dev/null | sort` });
record_result({ status });"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "an execute_code body whose only /dev reference is 2>/dev/null must NOT be flagged"
        );
    }

    // --- execute_code bodies (R2-01) --------------------------------------

    #[test]
    fn execute_code_embedded_ssh_write_is_flagged() {
        let env = nix_env();
        let code = r#"import { shell } from "developer";
shell({ command: `printf 'Host evil\n' >> ~/.ssh/config` });
record_result("done");"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_some(),
            "an embedded shell redirect into ~/.ssh/config must be flagged"
        );
    }

    #[test]
    fn execute_code_editor_write_to_sensitive_path_is_flagged() {
        let env = nix_env();
        let code = r#"import { text_editor } from "developer";
text_editor({ command: "write", path: "~/.ssh/authorized_keys", file_text: "ssh-rsa AAAA" });"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_some(),
            "an embedded text_editor write to ~/.ssh must be flagged"
        );
    }

    #[test]
    fn execute_code_view_of_sensitive_path_is_not_flagged() {
        let env = nix_env();
        let code = r#"import { text_editor } from "developer";
const c = text_editor({ command: "view", path: "/etc/hosts" });
record_result(c);"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "a view of /etc/hosts must not be flagged"
        );
    }

    #[test]
    fn execute_code_markdown_angle_bracket_placeholder_is_not_flagged() {
        // Regression (BIOOKF-I1-DEFECT-1): a text_editor write whose file_text is a
        // markdown doc containing an angle-bracket placeholder line like
        // `knowledge/<type>/<slug>.md` must NOT be read as a shell redirect to `/`.
        // The `>` of `<type>` was captured by redirect_targets, yielding target `/`
        // → Blast::Root → a spurious "writes to / (the filesystem root or a whole
        // volume)" approval prompt on an ordinary /tmp write.
        let env = nix_env();
        let code = r#"import { text_editor } from "developer";
text_editor({ command: "write", path: "/tmp/biookf-rebuild/SPEC.md", file_text: `# BioOKF SPEC

Directory layout:

  raw/
  knowledge/
    <type>/
      <slug>.md
  index.md
  log.md

Every concept doc's type is one of the 28.
` });
record_result("done");"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "markdown prose with <type>/<slug> placeholders must not be flagged as a root write"
        );
    }

    // --- issue #106: a candidate path belongs to the call that mutates it ----

    /// The reported repro. `kb_write_page` is a knowledge-base tool, not a file
    /// editor, but its name contains "write"; the old whole-source gate let that
    /// enable a scan of *every* literal in the script, so the unrelated utility
    /// literal `"/"` from `page.path.split("/")` normalized to the filesystem
    /// root and stalled Auto mode on an approval prompt it should never show.
    #[test]
    fn execute_code_kb_page_write_beside_slash_utility_is_not_flagged() {
        let env = nix_env();
        let code = r##"import { kb_write_page } from "knowledge";
const pages = [
  { path: "knowledge/disease/multiple-sclerosis.md", body: "# Multiple sclerosis" },
  { path: "knowledge/gene/hla-drb1.md", body: "# HLA-DRB1" },
];
for (const page of pages) {
  const filename = page.path.split("/").pop();
  kb_write_page({ kb_id: "ms-papers", path: page.path, body: page.body });
  record_result(`wrote ${filename}`);
}"##;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "a KB page write beside an unrelated split(\"/\") must NOT be flagged, got {:?}",
            code_is_sensitive(code, &env, &test_session())
        );
    }

    /// The containment rule, exercised where it actually bites: the `"/"` sits
    /// *inside* the mutating call's own argument list. It is an argument of
    /// `split`, never of `kb_write_page`, so it is not a candidate target.
    #[test]
    fn a_utility_literal_inside_a_mutating_calls_arguments_is_not_its_target() {
        let env = nix_env();
        let code = r#"import { kb_write_page } from "knowledge";
kb_write_page({ path: page.path.split("/").pop(), body: text });"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "a literal inside a nested call is that call's argument, not the outer write target"
        );
    }

    /// The other half of the same rule: dropping the whole-source scan must not
    /// drop coverage. A genuinely sensitive path in the mutating call's *own*
    /// path argument is still flagged, including when the call is nested inside
    /// another expression.
    #[test]
    fn a_sensitive_path_in_the_mutating_calls_own_argument_is_still_flagged() {
        let env = nix_env();
        for code in [
            r#"kb_write_page({ path: "/etc/cron.d/x", body: "pwn" });"#,
            r#"record_result(text_editor({ command: "write", path: "~/.ssh/config", file_text: "Host evil" }));"#,
            r#"files__write_file({ path: page.dir.split("/").pop(), output_path: "/etc/shadow" });"#,
        ] {
            assert!(
                code_is_sensitive(code, &env, &test_session()).is_some(),
                "a sensitive path in the mutating call's own argument must be flagged: {code}"
            );
        }
    }

    /// Content is not a target. A markdown body that merely *mentions* a
    /// sensitive path is not a write to it — `body` / `file_text` are not
    /// path-keyed arguments, so only `path` is graded.
    #[test]
    fn a_sensitive_path_quoted_in_page_content_is_not_a_write_target() {
        let env = nix_env();
        let code = r#"kb_write_page({
  path: "knowledge/procedure/key-rotation.md",
  body: "Rotate the key stored at ~/.ssh/id_ed25519, then reload /etc/ssh/sshd_config.",
});"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "a sensitive path named in prose content is not a write to it"
        );
    }

    /// End-to-end at the inspector (acceptance criterion): an ordinary
    /// knowledge-page write batch in Auto mode produces no approval request.
    #[tokio::test]
    async fn auto_mode_leaves_a_knowledge_page_write_batch_unescalated() {
        let inspector = SensitiveOpsInspector::unbound();
        let req = ToolRequest {
            id: "req_kb".to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "code_execution__execute_code".into(),
                arguments: Some(args(json!({
                    "code": "import { kb_write_page } from \"knowledge\";\n\
                             for (const page of pages) {\n\
                               const slug = page.path.split(\"/\").pop();\n\
                               kb_write_page({ path: page.path, body: page.body });\n\
                             }"
                }))),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        };
        let results = inspector
            .inspect(
                &[req],
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "an ordinary KB page write must not request approval in Auto mode, got {results:?}"
        );
    }

    // --- the inner-call parser (pure) -------------------------------------

    #[test]
    fn inner_calls_are_scoped_to_their_own_parentheses() {
        let calls =
            extract_inner_calls(r#"kb_write_page({ path: p.split("/").pop(), kb_id: "ms" });"#);
        let outer = calls
            .iter()
            .find(|c| c.callee == "kb_write_page")
            .expect("the outer call is recovered");
        assert!(
            !outer.args.contains_key("path"),
            "a dynamic path must be absent, not guessed: {:?}",
            outer.args
        );
        assert_eq!(outer.args.get("kb_id").and_then(Value::as_str), Some("ms"));
        assert!(
            calls.iter().any(|c| c.callee == "split"),
            "the nested call is still recovered in its own right"
        );
    }

    #[test]
    fn inner_call_args_read_literals_arrays_and_quoted_keys() {
        let calls =
            extract_inner_calls(r#"rm_files({ "paths": ["/etc/a", "/etc/b"], dry: false });"#);
        let call = calls.iter().find(|c| c.callee == "rm_files").unwrap();
        assert_eq!(
            call.args
                .get("paths")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(
            !call.args.contains_key("dry"),
            "a non-literal value is absent"
        );
    }

    /// Parentheses, colons and braces inside a string literal are text, not
    /// syntax — the one string scanner keeps every walker in agreement.
    #[test]
    fn syntax_inside_a_string_literal_does_not_confuse_the_parser() {
        let calls = extract_inner_calls(
            r#"note({ text: "not a key: (nor a paren", path: "/etc/passwd" });"#,
        );
        let call = calls.iter().find(|c| c.callee == "note").unwrap();
        assert_eq!(
            call.args.get("path").and_then(Value::as_str),
            Some("/etc/passwd"),
            "args after a literal containing syntax are still read: {:?}",
            call.args
        );
    }

    #[test]
    fn execute_code_ordinary_work_is_not_flagged() {
        let env = nix_env();
        let code = r#"import { shell, text_editor } from "developer";
const files = shell({ command: "ls -la /tmp" });
text_editor({ command: "write", path: "/tmp/out.txt", file_text: files });
record_result("ok");"#;
        assert!(
            code_is_sensitive(code, &env, &test_session()).is_none(),
            "ordinary /tmp scratch work must not be flagged"
        );
    }

    /// End-to-end at the inspector: a `developer/shell` sensitive write in Auto
    /// mode is routed to approval (uses `/etc`, a host system dir, so the fixture
    /// is deterministic regardless of `$HOME`). Unix-only: `/etc` is a system dir
    /// only on Unix.
    #[cfg(unix)]
    #[tokio::test]
    async fn auto_mode_routes_shell_sensitive_write_to_approval() {
        let inspector = SensitiveOpsInspector::unbound();
        let req = ToolRequest {
            id: "req_shell".to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "developer__shell".into(),
                arguments: Some(args(json!({ "command": "cp ./k /etc/cron.d/backdoor" }))),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        };
        let results = inspector
            .inspect(
                &[req],
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert!(
            results.iter().any(|r| r.tool_request_id == "req_shell"
                && matches!(r.action, InspectionAction::RequireApproval(_))),
            "shell write to /etc must be routed to approval, got {results:?}"
        );
    }

    /// End-to-end: an `execute_code` script that hides a sensitive shell write in
    /// its body is routed to approval in Auto mode (the R2-01 gate). Unix-only:
    /// `/etc` is a host system dir only on Unix.
    #[cfg(unix)]
    #[tokio::test]
    async fn auto_mode_routes_execute_code_sensitive_write_to_approval() {
        let inspector = SensitiveOpsInspector::unbound();
        let req = ToolRequest {
            id: "req_code".to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "code_execution__execute_code".into(),
                arguments: Some(args(json!({
                    "code": "import { shell } from \"developer\";\nshell({ command: \"echo pwn >> /etc/cron.d/x\" });"
                }))),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        };
        let results = inspector
            .inspect(
                &[req],
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert!(
            results.iter().any(|r| r.tool_request_id == "req_code"
                && matches!(r.action, InspectionAction::RequireApproval(_))),
            "execute_code hiding a sensitive write must be routed to approval, got {results:?}"
        );
    }

    /// Gate (c): every non-Auto mode is inert — even a sensitive write yields no
    /// result, so those modes' existing behaviour is provably unchanged.
    #[tokio::test]
    async fn non_auto_modes_are_inert() {
        let inspector = SensitiveOpsInspector::unbound();
        let requests = vec![write_request("req_sys", "/etc/cron.d/backdoor")];
        for mode in [
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
            BioRouterMode::Chat,
        ] {
            let results = inspector
                .inspect(&requests, &[], mode, &crate::session::Session::default())
                .await
                .unwrap();
            assert!(
                results.is_empty(),
                "{mode:?} must be inert (no sensitive-ops result), got {results:?}"
            );
        }
    }

    // --- criterion 5: recursive deletion of an established directory -------
    //
    // The incident these pin: in Auto mode an agent ran
    // `cd ~/Desktop && rm -rf kdps-build && mkdir …` and destroyed an unpushed
    // git repository with no prompt of any kind. Criteria 1-4 were silent
    // because they only ask *where* a path is, and `~/Desktop/kdps-build` is
    // `Blast::Ordinary`.

    use crate::session::Session;
    use std::path::PathBuf;

    /// A scratch tree **outside** the OS temp trees.
    ///
    /// Criterion 5 deliberately exempts `/tmp` and `$TMPDIR`, so a `TempDir`
    /// fixture would exercise the exemption rather than the rule — it would pass
    /// for a gate that had been deleted. The build directory is the nearest
    /// always-writable location that is not scratch space by policy.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sensitive-ops-tests")
                .join(Uuid::new_v4().simple().to_string());
            std::fs::create_dir_all(&root).expect("scratch root");
            let root = root.canonicalize().expect("canonical scratch root");
            assert!(
                !is_temp_path(&root.to_string_lossy()),
                "the fixture root must not be a temp path, or it tests the exemption: {}",
                root.display()
            );
            Self { root }
        }

        fn path(&self) -> &Path {
            &self.root
        }

        fn dir(&self, name: &str) -> PathBuf {
            let dir = self.root.join(name);
            std::fs::create_dir_all(&dir).expect("fixture directory");
            dir
        }

        /// A directory holding `files` ordinary files.
        fn populated(&self, name: &str, files: usize) -> PathBuf {
            let dir = self.dir(name);
            for i in 0..files {
                std::fs::write(dir.join(format!("f{i}.txt")), "content").expect("fixture file");
            }
            dir
        }

        /// A directory that is a git repository (a `.git` beside its content).
        fn repo(&self, name: &str) -> PathBuf {
            let dir = self.populated(name, 3);
            std::fs::create_dir_all(dir.join(".git")).expect("fixture .git");
            std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").expect("fixture HEAD");
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// A session that began *after* the fixtures were written. Fixtures are
    /// created milliseconds ago, so dating the session into the future is the
    /// deterministic way to make them "pre-existing" without reaching for a
    /// mtime-rewriting dependency — it states exactly the property the rule
    /// reads (content that predates the session).
    fn started_after_fixtures() -> SystemTime {
        SystemTime::now() + Duration::from_secs(600)
    }

    /// The single-finding forms these tests are written against. Production reads
    /// [`Findings`] so an approval card can name every operation a call performs;
    /// the yes/no question — "does this call have to be approved at all" — is what
    /// almost every test below is really asking, and it stays spelled that way.
    fn command_writes_sensitively(command: &str, env: &EnvFacts) -> Option<String> {
        let mut found = Findings::default();
        super::command_writes_sensitively(command, env, &mut found);
        found.into_reasons().into_iter().next()
    }

    fn code_is_sensitive(code: &str, env: &EnvFacts, session: &CallSession<'_>) -> Option<String> {
        code_findings_of(code, env, session)
            .into_reasons()
            .into_iter()
            .next()
    }

    /// [`code_is_sensitive`], keeping every hit.
    fn code_findings_of(code: &str, env: &EnvFacts, session: &CallSession<'_>) -> Findings {
        let mut found = Findings::default();
        code_findings(code, env, session, &mut found);
        found
    }

    /// Run the shell-command half of the classifier against the host filesystem.
    fn shell_finding(command: &str, session: &CallSession<'_>) -> Option<String> {
        shell_findings(command, session)
            .into_reasons()
            .into_iter()
            .next()
    }

    /// [`shell_finding`], keeping every hit — what the approval card is built
    /// from.
    fn shell_findings(command: &str, session: &CallSession<'_>) -> Findings {
        let env = EnvFacts::host(&session.working_dir().to_string_lossy());
        let mut found = Findings::default();
        command_findings(command, &env, session, &mut found);
        found
    }

    /// A session that began *before* the fixtures were written — the shape a
    /// directory the session really did create arrives in, since its content
    /// then post-dates the session start.
    fn started_before_fixtures() -> SystemTime {
        SystemTime::now() - Duration::from_secs(600)
    }

    /// One *completed* tool call: the request the model wrote, plus the response
    /// the agent wrote once it returned. `ran_ok = false` is the response of a
    /// call that was refused at the approval prompt or failed — history that
    /// records an intention, never an effect.
    fn command_history(id: &str, command: &str, ran_ok: bool) -> Vec<Message> {
        vec![
            Message::assistant().with_tool_request(
                id,
                Ok(CallToolRequestParams {
                    task: None,
                    name: "developer__shell".into(),
                    arguments: Some(args(json!({ "command": command }))),
                    meta: None,
                }),
            ),
            Message::user().with_tool_response(
                id,
                Ok(CallToolResult {
                    content: vec![rmcp::model::Content::text(if ran_ok {
                        "ok"
                    } else {
                        "denied by the user"
                    })],
                    structured_content: None,
                    is_error: Some(!ran_ok),
                    meta: None,
                }),
            ),
        ]
    }

    fn mkdir_history(id: &str, path: &Path) -> Vec<Message> {
        command_history(id, &format!("mkdir -p {}", path.display()), true)
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn recursive_delete_of_a_pre_existing_non_empty_directory_asks() {
        let scratch = Scratch::new();
        let victim = scratch.populated("analysis", 4);
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);

        let finding = shell_finding(&format!("rm -rf {}", victim.display()), &session)
            .expect("a populated, pre-session directory must be escalated");
        assert!(
            finding.contains("recursively deletes") && finding.contains("4 files"),
            "the prompt must name what would be destroyed, got: {finding}"
        );
        assert!(
            finding.contains(&victim.to_string_lossy().to_string()),
            "the prompt must name the absolute path, got: {finding}"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The fixture is a repository, and the session started before it existed —
    /// the shape a directory the session really did create arrives in. The `.git`
    /// is what keeps the provenance load-bearing: without it the age rule alone
    /// would exempt the fixture and the test would pass for a deleted exemption.
    #[test]
    fn recursive_delete_of_a_directory_this_session_created_does_not_ask() {
        let scratch = Scratch::new();
        let victim = scratch.repo("scratch-build");
        let command = format!("rm -rf {}", victim.display());

        // Control: the identical fixture and command WITHOUT the history is
        // escalated, so the exemption below is the provenance doing the work
        // rather than the fixture never having matched.
        let no_history = CallSession::new(scratch.path(), started_before_fixtures(), &[], &[]);
        assert!(shell_finding(&command, &no_history).is_some());

        // A `git clone` rather than a `mkdir`: over a REPOSITORY, provenance is
        // restricted to verbs that cannot succeed onto a populated existing tree
        // (see `strongly_created_dir_operands`), because there the filesystem
        // cannot corroborate and `mkdir -p` would be replayable over the victim.
        let history = command_history(
            "req_clone",
            &format!(
                "git clone https://example.com/scratch-build.git {}",
                victim.display()
            ),
            true,
        );
        let session = CallSession::new(scratch.path(), started_before_fixtures(), &history, &[]);
        assert_eq!(
            shell_finding(&command, &session),
            None,
            "a directory an earlier call in this session created is the session's own work"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The retry-escalation attack, and the reason provenance is corroborated
    /// against the filesystem: a model whose `rm -rf` was refused issues
    /// `mkdir -p <same path>` — which succeeds on an existing directory and
    /// creates nothing — and retries the delete. The history now says the
    /// session made it. The directory's own contents say otherwise, and they
    /// win: a denied operation must not become its own permission.
    #[test]
    fn a_mkdir_over_an_established_directory_is_not_provenance_for_deleting_it() {
        let scratch = Scratch::new();
        let victim = scratch.populated("kdps-build", 4);
        let command = format!("rm -rf {}", victim.display());

        // The `mkdir -p` really ran, really succeeded, and is a separate call in
        // an earlier turn — every condition the old rule asked for.
        let history = mkdir_history("req_mkdir", &victim);
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &history, &[]);
        assert!(
            shell_finding(&command, &session).is_some(),
            "a mkdir over a directory full of pre-session content creates nothing, \
             and must not launder the delete that follows it"
        );

        // Control: the identical history against a directory whose content the
        // session really did produce still exempts, so the corroboration narrows
        // the exemption rather than removing it.
        // Non-git on purpose: over a repository a bare `mkdir` no longer vouches
        // at all, which `a_replayed_mkdir_cannot_unlock_a_repository` pins.
        let own = scratch.populated("session-made", 2);
        let own_history = mkdir_history("req_mkdir2", &own);
        let own_session =
            CallSession::new(scratch.path(), started_before_fixtures(), &own_history, &[]);
        assert_eq!(
            shell_finding(&format!("rm -rf {}", own.display()), &own_session),
            None
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// Provenance is read off the *response* the agent wrote, not the request the
    /// model wrote: a creation that was refused at the approval prompt (or that
    /// failed) never happened, and vouches for nothing.
    #[test]
    fn a_creation_that_never_ran_is_not_provenance() {
        let scratch = Scratch::new();
        let victim = scratch.repo("kdps-build");
        let command = format!("rm -rf {}", victim.display());

        let denied = command_history(
            "req_mkdir",
            &format!("mkdir -p {}", victim.display()),
            false,
        );
        let session = CallSession::new(scratch.path(), started_before_fixtures(), &denied, &[]);
        assert!(
            shell_finding(&command, &session).is_some(),
            "a mkdir the user denied is not evidence the session created anything"
        );

        // Control: the same call with a successful response DOES exempt it, so
        // this asserts the outcome rather than a fixture that never matched.
        //
        // ⚠ The control uses a NON-git directory deliberately. A successful
        // `mkdir` vouches only where the filesystem can corroborate it; over a
        // repository it proves nothing, which is what
        // `a_replayed_mkdir_cannot_unlock_a_repository` pins.
        let plain = scratch.populated("scratch-out", 2);
        let plain_command = format!("rm -rf {}", plain.display());
        let ran = mkdir_history("req_mkdir", &plain);
        let ok_session = CallSession::new(scratch.path(), started_before_fixtures(), &ran, &[]);
        assert_eq!(shell_finding(&plain_command, &ok_session), None);
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// A refused delete must not be able to mint its own permission — the whole
    /// point of criterion 5 — and the one shape where the filesystem cannot say
    /// so is a repository whose every entry post-dates the session.
    ///
    /// There, provenance is the only input left, and it is the model's word. So
    /// it is restricted to verbs that CANNOT succeed onto a populated existing
    /// tree. `mkdir -p` and `git init` both succeed there while creating
    /// nothing, so replaying either one over the victim must not unlock it.
    #[test]
    fn a_replayed_mkdir_cannot_unlock_a_repository() {
        let scratch = Scratch::new();
        let victim = scratch.repo("kdps-build");
        let command = format!("rm -rf {}", victim.display());
        // The session opened before the repo appeared, so nothing in it predates
        // the session and the age test cannot adjudicate: exactly the gap.
        let started = started_before_fixtures();

        for replay in ["mkdir -p", "git init"] {
            let history = command_history(
                "req_replay",
                &format!("{replay} {}", victim.display()),
                true,
            );
            let session = CallSession::new(scratch.path(), started, &history, &[]);
            assert!(
                shell_finding(&command, &session).is_some(),
                "`{replay}` succeeds without creating anything on an existing \
                 repository, so replaying it must not launder the delete"
            );
        }

        // Control: a verb that genuinely cannot land on a populated existing
        // directory still earns the exemption, so this narrows rather than
        // disables provenance.
        let cloned = command_history(
            "req_clone",
            &format!(
                "git clone https://example.com/kdps-build.git {}",
                victim.display()
            ),
            true,
        );
        let clone_session = CallSession::new(scratch.path(), started, &cloned, &[]);
        assert_eq!(
            shell_finding(&command, &clone_session),
            None,
            "a repository this session cloned is still the session's own work"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The provenance rule must not let a call vouch for *itself*: the incident
    /// arrived as `rm -rf X && mkdir X`, so reading the batch under inspection as
    /// history would have excused the very deletion it is meant to catch.
    #[test]
    fn a_mkdir_in_the_same_call_is_not_provenance_for_its_own_delete() {
        let scratch = Scratch::new();
        let victim = scratch.populated("kdps-build", 2);
        let command = format!("rm -rf {v} && mkdir {v}", v = victim.display());

        // Even with that exact call already recorded in the conversation, the id
        // under inspection is skipped.
        let history = vec![Message::assistant().with_tool_request(
            "req_now",
            Ok(CallToolRequestParams {
                task: None,
                name: "developer__shell".into(),
                arguments: Some(args(json!({ "command": command.clone() }))),
                meta: None,
            }),
        )];
        let current = vec!["req_now".to_string()];
        let session =
            CallSession::new(scratch.path(), started_after_fixtures(), &history, &current);

        assert!(
            shell_finding(&command, &session).is_some(),
            "a delete must not be excused by a mkdir in the same tool call"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn recursive_delete_of_an_empty_directory_does_not_ask() {
        let scratch = Scratch::new();
        let empty = scratch.dir("empty");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);

        let command = format!("rm -rf {}", empty.display());
        assert_eq!(
            shell_finding(&command, &session),
            None,
            "an empty directory holds nothing to lose"
        );

        // Control: one file in the same directory and it is escalated, so
        // emptiness is the only thing that changed.
        std::fs::write(empty.join("one.txt"), "x").expect("fixture file");
        assert!(shell_finding(&command, &session).is_some());
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn a_missing_path_a_plain_file_and_a_symlink_do_not_ask() {
        let scratch = Scratch::new();
        let real = scratch.populated("real", 2);
        let file = scratch.path().join("notes.txt");
        std::fs::write(&file, "hello").expect("fixture file");
        let link = scratch.path().join("link-to-real");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).expect("fixture symlink");

        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        let mut targets = vec![scratch.path().join("does-not-exist"), file];
        if cfg!(unix) {
            // `rm -rf` on a symlink unlinks the link, never the tree behind it.
            targets.push(link);
        }
        for target in targets {
            assert_eq!(
                shell_finding(&format!("rm -rf {}", target.display()), &session),
                None,
                "{} is not an established directory",
                target.display()
            );
        }
        // Control: the directory the symlink points at IS escalated when named
        // directly, so the symlink exemption is about the link, not the tree.
        assert!(shell_finding(&format!("rm -rf {}", real.display()), &session).is_some());
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The `cd`-then-relative shape the incident actually arrived in. Resolving
    /// against the session working directory instead would miss it entirely.
    #[test]
    fn a_relative_target_after_cd_resolves_against_the_cd_and_asks() {
        let scratch = Scratch::new();
        let victim = scratch.populated("kdps-build", 3);
        // The session's own working directory is somewhere else entirely.
        let elsewhere = scratch.dir("workspace");
        let session = CallSession::new(&elsewhere, started_after_fixtures(), &[], &[]);

        let finding = shell_finding(
            &format!(
                "cd {} && rm -rf kdps-build && mkdir kdps-build",
                scratch.path().display()
            ),
            &session,
        )
        .expect("the target must resolve through the cd");
        assert!(
            finding.contains(&victim.to_string_lossy().to_string()),
            "the resolved path must be the cd'd one, got: {finding}"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// A target that holds a git repository is escalated on that evidence alone —
    /// here the session start is *now*, so the age heuristic says every file is
    /// the session's own work and only the `.git` speaks. Destroying history is
    /// unrecoverable in a way that losing a working tree is not, and that is
    /// precisely what the incident destroyed.
    #[test]
    fn a_target_containing_a_git_repository_asks_even_when_the_age_is_ambiguous() {
        let scratch = Scratch::new();
        let repo = scratch.repo("kdps-build");
        let session = CallSession::new(scratch.path(), SystemTime::now(), &[], &[]);

        let finding = shell_finding(&format!("rm -rf {}", repo.display()), &session)
            .expect("a git repository must be escalated");
        assert!(
            finding.contains("contains a git repository"),
            "the prompt must say a repository is at stake, got: {finding}"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The same fixture, with the session on record as having created it: a repo
    /// the agent cloned or initialised itself is its own work, and tearing it
    /// down must stay silent.
    #[test]
    fn a_git_repository_this_session_cloned_does_not_ask() {
        let scratch = Scratch::new();
        let repo = scratch.repo("cloned");
        let history = command_history(
            "req_clone",
            &format!(
                "cd {} && git clone https://example.org/cloned.git",
                scratch.path().display()
            ),
            true,
        );
        let command = format!("rm -rf {}", repo.display());

        // Control: without the clone in the history this same repository is
        // escalated (it is the `.git` fixture the positive test uses), on the
        // same session clock — so the exemption below is the provenance.
        let no_history = CallSession::new(scratch.path(), started_before_fixtures(), &[], &[]);
        assert!(shell_finding(&command, &no_history).is_some());

        let session = CallSession::new(scratch.path(), started_before_fixtures(), &history, &[]);
        assert_eq!(
            shell_finding(&command, &session),
            None,
            "a repository this session cloned is not the user's work"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The false-positive guard the rule lives or dies by. An agent in Auto mode
    /// clears build output constantly; a prompt on every one of these would get
    /// Auto mode switched off, which is a worse outcome than the hole.
    #[test]
    fn regenerable_build_and_dependency_directories_do_not_ask() {
        let scratch = Scratch::new();
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        for name in [
            "node_modules",
            "target",
            "dist",
            "build",
            "__pycache__",
            ".venv",
        ] {
            let dir = scratch.populated(name, 3);
            assert_eq!(
                shell_finding(&format!("rm -rf {}", dir.display()), &session),
                None,
                "clearing {name} must not prompt"
            );
        }
        // Control: the identical fixture under a name nobody regenerates IS
        // escalated, so the exemption is the name list, not an inert rule.
        let user_work = scratch.populated("manuscript-figures", 3);
        assert!(shell_finding(&format!("rm -rf {}", user_work.display()), &session).is_some());
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// …but a repository is never build output, whatever it is called.
    #[test]
    fn a_regenerable_name_holding_a_repository_still_asks() {
        let scratch = Scratch::new();
        let dir = scratch.repo("build");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        assert!(
            shell_finding(&format!("rm -rf {}", dir.display()), &session).is_some(),
            "the name exemption must not launder a git repository"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// Content the session itself produced is not the user's work: a tree built
    /// and torn down inside one session must never prompt.
    #[test]
    fn a_directory_whose_content_postdates_the_session_does_not_ask() {
        let scratch = Scratch::new();
        let dir = scratch.populated("results", 5);
        let command = format!("rm -rf {}", dir.display());

        // Control: dating the session *after* the same fixtures escalates it.
        let older = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        assert!(shell_finding(&command, &older).is_some());

        // The session began before the fixtures existed.
        let session = CallSession::new(
            scratch.path(),
            SystemTime::now() - Duration::from_secs(600),
            &[],
            &[],
        );
        assert_eq!(
            shell_finding(&command, &session),
            None,
            "a tree produced during the session is the session's own output"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// A glob destroys strictly more than the single path beside it, so it must
    /// not be the one shape that walks through. The shell expands it before `rm`
    /// runs, so the criterion grades the containing directory instead.
    #[test]
    fn a_glob_delete_asks_when_its_containing_directory_is_established() {
        let scratch = Scratch::new();
        let victim = scratch.repo("kdps-build");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);

        for command in [
            format!("rm -rf {}/*", victim.display()),
            format!("cd {} && rm -rf *", victim.display()),
            format!("cd {} && rm -rf kdps-*", scratch.path().display()),
        ] {
            assert!(
                shell_finding(&command, &session).is_some(),
                "`{command}` deletes at least as much as naming the path outright"
            );
        }
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// …and it inherits every exemption the named target has, so the fail-closed
    /// rule does not become the prompt that gets Auto mode switched off.
    #[test]
    fn a_glob_delete_inherits_the_containing_directorys_exemptions() {
        let scratch = Scratch::new();
        let regenerable = scratch.populated("node_modules", 3);
        let fresh = scratch.populated("session-output", 3);
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);

        assert_eq!(
            shell_finding(&format!("rm -rf {}/*", regenerable.display()), &session),
            None,
            "clearing a regenerable directory must not prompt, glob or not"
        );
        // Control: the same glob one level up, where the container is the user's
        // own tree, IS escalated.
        assert!(
            shell_finding(&format!("rm -rf {}/*", scratch.path().display()), &session).is_some()
        );

        // Content that post-dates the session is the session's own output.
        let own = CallSession::new(scratch.path(), started_before_fixtures(), &[], &[]);
        assert_eq!(
            shell_finding(&format!("rm -rf {}/*", fresh.display()), &own),
            None
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The temp trees stay scratch space, exactly as they are for criteria 1-4.
    #[test]
    fn a_recursive_delete_inside_a_temp_tree_does_not_ask() {
        let temp = std::env::temp_dir().join(format!("sens-ops-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&temp).expect("temp fixture");
        std::fs::write(temp.join("a.txt"), "x").expect("temp file");
        std::fs::create_dir_all(temp.join(".git")).expect("temp .git");

        let session = CallSession::new(Path::new("/"), started_after_fixtures(), &[], &[]);
        let finding = shell_finding(&format!("rm -rf {}", temp.display()), &session);

        // Control: the same shape outside the temp trees IS escalated, so this
        // asserts the exemption rather than a rule that never fires.
        let scratch = Scratch::new();
        let outside = scratch.repo("same-shape");
        let control = shell_finding(&format!("rm -rf {}", outside.display()), &session);

        let _ = std::fs::remove_dir_all(&temp);
        assert!(control.is_some());
        assert_eq!(
            finding, None,
            "the OS temp trees are ordinary scratch space"
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn reads_of_an_established_directory_never_escalate() {
        let scratch = Scratch::new();
        let victim = scratch.repo("analysis");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        for verb in ["ls -la", "cat", "grep -r x", "du -sh", "find"] {
            let command = format!("{verb} {}", victim.display());
            assert_eq!(
                shell_finding(&command, &session),
                None,
                "`{command}` is a read and must never escalate"
            );
        }
        // Control: the same path under a recursive delete IS escalated, so this
        // is about the verb rather than about an unmatched fixture.
        assert!(shell_finding(&format!("rm -rf {}", victim.display()), &session).is_some());
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// A non-recursive delete destroys nothing this criterion is about: `rm -f`
    /// on a directory fails, and `rmdir` refuses a non-empty one.
    #[test]
    fn a_non_recursive_delete_does_not_ask() {
        let scratch = Scratch::new();
        let victim = scratch.repo("analysis");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        for command in ["rm -f", "rm", "rmdir", "mv"] {
            assert_eq!(
                shell_finding(&format!("{command} {}", victim.display()), &session),
                None,
                "`{command}` is not a recursive directory delete"
            );
        }
        // Control: the recursive spelling of the same command on the same
        // fixture is escalated.
        assert!(shell_finding(&format!("rm -rf {}", victim.display()), &session).is_some());
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn every_recursive_flag_spelling_is_detected() {
        let scratch = Scratch::new();
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        for flags in [
            "-rf",
            "-fr",
            "-r",
            "-R",
            "-Rf",
            "--recursive",
            "-vrf",
            "-r -f",
        ] {
            let victim = scratch.populated(&format!("case{}", flags.replace([' ', '-'], "")), 2);
            assert!(
                shell_finding(&format!("rm {flags} {}", victim.display()), &session).is_some(),
                "`rm {flags}` is a recursive delete"
            );
        }
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// `--` ends option parsing, so a *file* named `-rf` after it is an operand
    /// and the command is not recursive at all.
    /// Real paths have spaces in them. The tokenizer strips the quoting before
    /// this rule sees the operand, and a target that failed to reassemble would
    /// simply not exist on disk — a silent miss rather than a visible error.
    #[test]
    fn a_quoted_target_containing_spaces_is_resolved() {
        let scratch = Scratch::new();
        let victim = scratch.populated("My Analysis 2026", 3);
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        for command in [
            format!("rm -rf \"{}\"", victim.display()),
            format!("rm -rf '{}'", victim.display()),
            format!(
                "cd \"{}\" && rm -rf \"My Analysis 2026\"",
                scratch.path().display()
            ),
        ] {
            assert!(
                shell_finding(&command, &session).is_some(),
                "`{command}` names a real directory and must be escalated"
            );
        }
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn a_dashed_operand_after_the_end_of_options_is_not_a_recursive_flag() {
        let scratch = Scratch::new();
        let victim = scratch.populated("odd", 2);
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        assert_eq!(
            shell_finding(&format!("rm -- -rf {}", victim.display()), &session),
            None,
            "after `--` a dashed token is a filename, not a recursion flag"
        );
        // Control: the same flag BEFORE `--` is a recursion flag.
        assert!(shell_finding(&format!("rm -rf -- {}", victim.display()), &session).is_some());
    }

    // --- the delete parser, per dialect (pure, no filesystem) --------------

    fn segments_of(command: &str, platform: Platform, dialect: Dialect) -> Vec<Segment> {
        let env = match platform {
            Platform::Windows => {
                EnvFacts::for_platform(platform, r"C:\Users\me\proj", r"C:\Users\me")
            }
            _ => EnvFacts::for_platform(platform, "/home/me/proj", "/home/me"),
        };
        ParsedCommand::parse_for_dialect(command, platform, dialect, &env).segments
    }

    fn detects_recursive_delete(command: &str, platform: Platform, dialect: Dialect) -> bool {
        segments_of(command, platform, dialect)
            .iter()
            .any(is_recursive_delete)
    }

    /// The Windows spellings, exercised the way the policy layer's Windows
    /// matrix is: with the platform and dialect forced, so they run on a mac.
    #[test]
    fn powershell_and_cmd_recursive_deletes_are_detected() {
        for command in [
            r"Remove-Item -Recurse -Force C:\Users\me\proj\data",
            r"Remove-Item -Recurse C:\Users\me\proj\data",
            // Aliases and unambiguous parameter prefixes are canonicalized by
            // the parser before this rule ever sees them.
            r"ri -rec -force C:\Users\me\proj\data",
            r"rd -Recurse C:\Users\me\proj\data",
            // `-r` is a prefix of Recurse alone in Remove-Item's parameter set.
            r"Remove-Item -r -Force C:\Users\me\proj\data",
        ] {
            assert!(
                detects_recursive_delete(command, Platform::Windows, Dialect::PowerShell),
                "`{command}` is a recursive delete"
            );
        }
        assert!(
            !detects_recursive_delete(
                r"Remove-Item C:\Users\me\proj\data\notes.txt",
                Platform::Windows,
                Dialect::PowerShell
            ),
            "Remove-Item without -Recurse is not a recursive directory delete"
        );

        for command in [
            r"rd /s /q C:\Users\me\proj\data",
            r"rmdir /S C:\Users\me\proj\data",
        ] {
            assert!(
                detects_recursive_delete(command, Platform::Windows, Dialect::Cmd),
                "`{command}` is a recursive delete"
            );
        }
        assert!(
            !detects_recursive_delete(
                r"rd /q C:\Users\me\proj\data",
                Platform::Windows,
                Dialect::Cmd
            ),
            "`rd` without /s only removes an empty directory"
        );
    }

    /// A PowerShell command reached through `pwsh -c` on a POSIX host is parsed
    /// in the PowerShell dialect, so criterion 5 sees it there too.
    #[cfg(unix)]
    #[test]
    fn a_powershell_delete_invoked_through_pwsh_is_escalated() {
        let scratch = Scratch::new();
        let victim = scratch.repo("analysis");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        assert!(
            shell_finding(
                &format!(
                    "pwsh -c \"Remove-Item -Recurse -Force {}\"",
                    victim.display()
                ),
                &session
            )
            .is_some(),
            "a PowerShell recursive delete must be escalated wherever it is hosted"
        );
    }

    /// The `cd` tracker is a pure property of the command text, so pin it
    /// directly rather than only through a filesystem fixture.
    #[test]
    fn delete_targets_follow_cd_across_a_command_line() {
        let env = nix_env();
        let targets = recursive_delete_targets("cd /a/b && rm -rf c", &env);
        assert_eq!(
            targets.iter().map(|t| t.norm.as_str()).collect::<Vec<_>>(),
            vec!["/a/b/c"],
            "a relative target must resolve against the cd, not the session cwd"
        );

        // A `cd` only affects what follows it.
        let targets = recursive_delete_targets("rm -rf early && cd /a/b && rm -rf late", &env);
        assert_eq!(
            targets.iter().map(|t| t.norm.as_str()).collect::<Vec<_>>(),
            vec!["/home/me/proj/early", "/a/b/late"]
        );
    }

    #[test]
    fn directory_creation_targets_cover_the_common_makers() {
        let env = nix_env();
        for (command, expected) in [
            ("mkdir out", "/home/me/proj/out"),
            ("mkdir -p a/b/c", "/home/me/proj/a/b/c"),
            (
                "git clone https://example.org/thing.git",
                "/home/me/proj/thing",
            ),
            (
                "git clone https://example.org/thing.git dest",
                "/home/me/proj/dest",
            ),
            ("git init fresh", "/home/me/proj/fresh"),
            ("git worktree add wt-1", "/home/me/proj/wt-1"),
            ("cargo new pkg", "/home/me/proj/pkg"),
            ("cd /a/b && mkdir made-here", "/a/b/made-here"),
        ] {
            let norms: Vec<String> = directory_creation_targets(command, &env)
                .into_iter()
                .map(|t| t.norm)
                .collect();
            assert!(
                norms.iter().any(|n| n == expected),
                "`{command}` should record {expected}, got {norms:?}"
            );
        }
        // A bare `git init` does not bring its directory into existence.
        assert!(directory_creation_targets("git init", &env).is_empty());
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The `execute_code` path: an embedded delete reaches the same rule, because
    /// a script's inner tool calls never pass an agent-layer inspector.
    #[test]
    fn an_execute_code_body_hiding_a_recursive_delete_is_escalated() {
        let scratch = Scratch::new();
        let victim = scratch.repo("analysis");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        let env = EnvFacts::host(&scratch.path().to_string_lossy());
        let code = format!(
            "import {{ shell }} from \"developer\";\nshell({{ command: `rm -rf {}` }});",
            victim.display()
        );
        assert!(
            code_is_sensitive(&code, &env, &session).is_some(),
            "a recursive delete hidden in an execute_code body must be escalated"
        );
    }

    // --- D8: the card names its whole blast radius -------------------------

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// **The card must name every directory the script destroys, not the first.**
    ///
    /// A script that clears six established trees used to produce a card naming
    /// one of them, so a user who read the card and approved it approved five
    /// deletions it never mentioned. An approval that does not describe its own
    /// blast radius is not consent.
    #[test]
    fn a_script_deleting_many_directories_names_all_of_them() {
        let scratch = Scratch::new();
        let victims: Vec<PathBuf> = ["alpha", "beta", "gamma"]
            .iter()
            .map(|name| scratch.populated(name, 2))
            .collect();
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        let env = EnvFacts::host(&scratch.path().to_string_lossy());
        let code = victims
            .iter()
            .map(|v| format!("shell({{ command: `rm -rf {}` }});", v.display()))
            .collect::<Vec<_>>()
            .join("\n");

        let found = code_findings_of(&code, &env, &session);
        let reasons = found.clone().into_reasons();
        assert_eq!(
            reasons.len(),
            3,
            "every established directory the script clears is a finding: {reasons:?}"
        );
        let described = found.describe();
        for victim in &victims {
            assert!(
                described.contains(&victim.to_string_lossy().to_string()),
                "the card must name {}, got: {described}",
                victim.display()
            );
        }
        assert!(
            described.contains("affects 3 places"),
            "the card must lead with how many, got: {described}"
        );
    }

    /// The same rule for one shell command line naming several targets — the
    /// shape `rm -rf a b c` arrives in.
    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn one_delete_command_with_several_operands_names_all_of_them() {
        let scratch = Scratch::new();
        let a = scratch.populated("one", 2);
        let b = scratch.repo("two");
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);

        let found = shell_findings(&format!("rm -rf {} {}", a.display(), b.display()), &session);
        let described = found.describe();
        assert!(
            described.contains(&a.to_string_lossy().to_string())
                && described.contains(&b.to_string_lossy().to_string()),
            "both operands must be named, got: {described}"
        );
        // The repository clause still rides its own entry, so the more serious of
        // the two is not flattened into a count.
        assert!(
            described.contains("contains a git repository"),
            "the card must keep what makes each target serious, got: {described}"
        );
    }

    /// A long list is summarised rather than dumped: the user is told how many,
    /// shown enough to recognise a mistake, and told what was left out. A card
    /// nobody can read is not consent either.
    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    #[test]
    fn a_very_long_list_is_counted_and_truncated_rather_than_dumped() {
        let scratch = Scratch::new();
        let victims: Vec<PathBuf> = (0..10)
            .map(|i| scratch.populated(&format!("v{i}"), 1))
            .collect();
        let session = CallSession::new(scratch.path(), started_after_fixtures(), &[], &[]);
        let command = format!(
            "rm -rf {}",
            victims
                .iter()
                .map(|v| v.display().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );

        let described = shell_findings(&command, &session).describe();
        assert!(
            described.contains("affects 10 places"),
            "the count is the part the user needs first, got: {described}"
        );
        assert_eq!(
            described.matches("  • ").count(),
            SHOWN_FINDINGS + 1,
            "six findings plus the one line accounting for the rest: {described}"
        );
        assert!(
            described.contains("…and 4 more, not listed here."),
            "the tail must be accounted for, not dropped, got: {described}"
        );
    }

    /// One finding still reads as the sentence it always was. The overwhelming
    /// majority of these calls trip exactly one criterion, and a one-item bullet
    /// list is worse prose than a clause.
    #[test]
    fn a_single_finding_still_reads_as_one_sentence() {
        let mut found = Findings::default();
        found.push("writes to /etc/passwd (a protected system directory)".into());
        assert_eq!(
            found.describe(),
            "This tool call writes to /etc/passwd (a protected system directory)."
        );
    }

    /// An `execute_code` body is scanned twice — string literals as command
    /// lines, then inner tool calls — so the same write is reachable down both
    /// paths. Counting it twice would put a number in front of the user that the
    /// script does not support.
    #[test]
    fn the_same_operation_found_twice_is_counted_once() {
        let mut found = Findings::default();
        assert!(!found.push("writes to /etc/hosts (a protected system directory)".into()));
        assert!(!found.push("writes to /etc/hosts (a protected system directory)".into()));
        assert_eq!(found.into_reasons().len(), 1);
    }

    /// The grading budget is a work limit, and at it the card says "at least" —
    /// a number that stopped counting must not be presented as a total.
    #[test]
    fn a_capped_scan_says_at_least_rather_than_an_exact_count() {
        let mut found = Findings::default();
        for i in 0..MAX_GRADED_FINDINGS {
            let full = found.push(format!(
                "writes to /etc/f{i} (a protected system directory)"
            ));
            assert_eq!(full, i + 1 == MAX_GRADED_FINDINGS);
        }
        assert!(found.is_full());
        let described = found.describe();
        assert!(
            described.contains(&format!("affects at least {MAX_GRADED_FINDINGS} places")),
            "got: {described}"
        );
    }

    // --- the inspector, end to end ----------------------------------------

    fn shell_request(id: &str, command: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "developer__shell".into(),
                arguments: Some(args(json!({ "command": command }))),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    fn session_at(working_dir: &Path, created_at: chrono::DateTime<chrono::Utc>) -> Session {
        Session {
            working_dir: working_dir.to_path_buf(),
            created_at,
            ..Session::default()
        }
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// D8, end to end: the card an `execute_code` script gets is the card that
    /// names what the script does — all of it.
    ///
    /// The same run is the regression test for the defect: on the single-finding
    /// card only `first` appeared, and approving it also approved `second` and
    /// `third` without their ever being shown.
    #[tokio::test]
    async fn an_execute_code_card_names_every_directory_the_script_clears() {
        let scratch = Scratch::new();
        let workspace = scratch.dir("workspace");
        let victims: Vec<PathBuf> = ["first", "second", "third"]
            .iter()
            .map(|name| scratch.populated(name, 2))
            .collect();
        let code = victims
            .iter()
            .map(|v| format!("shell({{ command: `rm -rf {}` }});", v.display()))
            .collect::<Vec<_>>()
            .join("\n");

        let results = SensitiveOpsInspector::unbound()
            .inspect(
                &[ToolRequest {
                    id: "req_code".into(),
                    tool_call: Ok(CallToolRequestParams {
                        task: None,
                        name: "code_execution__execute_code".into(),
                        arguments: Some(args(json!({ "code": code }))),
                        meta: None,
                    }),
                    metadata: None,
                    tool_meta: None,
                }],
                &[],
                BioRouterMode::Auto,
                &session_at(
                    &workspace,
                    chrono::Utc::now() + chrono::Duration::seconds(600),
                ),
            )
            .await
            .unwrap();

        let result = results
            .iter()
            .find(|r| r.tool_request_id == "req_code")
            .expect("the script must be escalated");
        let InspectionAction::RequireApproval(Some(prompt)) = &result.action else {
            panic!("expected an approval prompt, got {:?}", result.action);
        };
        for victim in &victims {
            assert!(
                prompt.contains(&victim.to_string_lossy().to_string()),
                "the card must name {}, got: {prompt}",
                victim.display()
            );
        }
        assert!(
            prompt.contains("affects 3 places"),
            "the card must lead with the blast radius, got: {prompt}"
        );
        assert!(
            result.reason.contains("at 3 paths"),
            "the logged reason must carry the count too, got: {}",
            result.reason
        );
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// The incident, end to end: the exact command, in the exact mode, now
    /// produces an approval request naming what it would destroy.
    #[tokio::test]
    async fn auto_mode_routes_the_incident_command_to_approval() {
        let scratch = Scratch::new();
        let victim = scratch.repo("kdps-build");
        let workspace = scratch.dir("workspace");
        let command = format!(
            "cd {} && rm -rf kdps-build && mkdir kdps-build",
            scratch.path().display()
        );

        let results = SensitiveOpsInspector::unbound()
            .inspect(
                &[shell_request("req_rm", &command)],
                &[],
                BioRouterMode::Auto,
                &session_at(
                    &workspace,
                    chrono::Utc::now() + chrono::Duration::seconds(600),
                ),
            )
            .await
            .unwrap();

        let result = results
            .iter()
            .find(|r| r.tool_request_id == "req_rm")
            .expect("the incident command must be escalated");
        let InspectionAction::RequireApproval(Some(prompt)) = &result.action else {
            panic!("expected an approval prompt, got {:?}", result.action);
        };
        assert!(
            prompt.contains(&victim.to_string_lossy().to_string())
                && prompt.contains("contains a git repository"),
            "the prompt must name the path and the repository, got: {prompt}"
        );
        assert_eq!(result.inspector_name, SENSITIVE_OPS_INSPECTOR_NAME);
    }

    #[cfg(not(target_os = "windows"))] // POSIX-dialect fixture; see the note above the module.
    /// Gate (c) for criterion 5: every non-Auto mode stays inert, for every
    /// spelling — the same early return that keeps criteria 1-4 out of them.
    #[tokio::test]
    async fn criterion_five_is_inert_outside_auto_mode() {
        let scratch = Scratch::new();
        let repo = scratch.repo("kdps-build");
        let plain = scratch.populated("analysis", 4);
        let workspace = scratch.dir("workspace");
        let session = session_at(
            &workspace,
            chrono::Utc::now() + chrono::Duration::seconds(600),
        );

        let commands = [
            format!("rm -rf {}", repo.display()),
            format!("rm -r {}", plain.display()),
            format!("rm --recursive {}", plain.display()),
            format!("cd {} && rm -rf kdps-build", scratch.path().display()),
            format!("pwsh -c \"Remove-Item -Recurse -Force {}\"", repo.display()),
        ];
        let requests: Vec<ToolRequest> = commands
            .iter()
            .enumerate()
            .map(|(i, c)| shell_request(&format!("req_{i}"), c))
            .collect();

        // Each one really is escalated in Auto, so inertness elsewhere is a
        // statement about the mode gate rather than about a fixture that never
        // matched.
        let auto = SensitiveOpsInspector::unbound()
            .inspect(&requests, &[], BioRouterMode::Auto, &session)
            .await
            .unwrap();
        assert_eq!(
            auto.len(),
            requests.len(),
            "every spelling must be escalated in Auto, got {auto:?}"
        );

        for mode in [
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
            BioRouterMode::Chat,
        ] {
            let results = SensitiveOpsInspector::unbound()
                .inspect(&requests, &[], mode, &session)
                .await
                .unwrap();
            assert!(
                results.is_empty(),
                "{mode:?} must be inert for criterion 5, got {results:?}"
            );
        }
    }

    /// Reads of the same paths never escalate in any mode, Auto included.
    #[tokio::test]
    async fn auto_mode_leaves_reads_of_an_established_directory_alone() {
        let scratch = Scratch::new();
        let repo = scratch.repo("kdps-build");
        let workspace = scratch.dir("workspace");
        let requests: Vec<ToolRequest> = ["ls -la", "cat", "grep -r pattern"]
            .iter()
            .enumerate()
            .map(|(i, verb)| {
                shell_request(&format!("req_{i}"), &format!("{verb} {}", repo.display()))
            })
            .collect();

        let results = SensitiveOpsInspector::unbound()
            .inspect(
                &requests,
                &[],
                BioRouterMode::Auto,
                &session_at(
                    &workspace,
                    chrono::Utc::now() + chrono::Duration::seconds(600),
                ),
            )
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "reading a directory must never escalate, got {results:?}"
        );
    }

    // --- criterion 7: a host key an agent's shell would trust unchecked ----

    fn trust_findings(command: &str, env: &EnvFacts) -> Findings {
        let mut found = Findings::default();
        command_trusts_host_key(command, env, &mut found);
        found
    }

    /// Every spelling the owner's D6 ruling names asks first: the three trusting values and
    /// `false`, `-oKey=value`, `-o 'Key value'`, a flag cluster, any letter case, the same
    /// ssh handed over by git, rsync or a `ProxyCommand`, and `ssh-keygen -R`. A fixture
    /// that matched nothing would make the inertness tests below meaningless, so each one is
    /// asserted here to be found.
    #[test]
    fn criterion_seven_grades_every_spelling_that_trusts_an_unchecked_host_key() {
        let env = nix_env();
        for command in [
            "ssh -o StrictHostKeyChecking=accept-new you@hpc.example.edu true",
            "ssh -o StrictHostKeyChecking=no hpc.example.edu",
            "ssh -o StrictHostKeyChecking=off hpc.example.edu",
            "ssh -o StrictHostKeyChecking=false hpc.example.edu",
            "ssh -oStrictHostKeyChecking=no hpc.example.edu",
            "ssh -o 'StrictHostKeyChecking no' hpc.example.edu",
            "ssh -o \"StrictHostKeyChecking = accept-new\" hpc.example.edu",
            "ssh -o stricthostkeychecking=No hpc.example.edu",
            "ssh -vo StrictHostKeyChecking=accept-new hpc.example.edu",
            "ssh -p 2222 -i ~/.ssh/id_ed25519 -o StrictHostKeyChecking=no hpc.example.edu",
            "ssh hpc.example.edu -o StrictHostKeyChecking=no uptime",
            "/usr/bin/ssh -o StrictHostKeyChecking=no hpc.example.edu",
            "scp -o StrictHostKeyChecking=no results.csv hpc.example.edu:",
            "sftp -oStrictHostKeyChecking=accept-new hpc.example.edu",
            "ssh-copy-id -i ~/.ssh/id_ed25519.pub -o StrictHostKeyChecking=no hpc.example.edu",
            "sshpass -e ssh -o StrictHostKeyChecking=no hpc.example.edu",
            "ssh -o 'ProxyCommand ssh -o StrictHostKeyChecking=no -W %h:%p gate' hpc.example.edu",
            "GIT_SSH_COMMAND='ssh -o StrictHostKeyChecking=no' git clone git@hpc.example.edu:lab.git",
            "env GIT_SSH_COMMAND='ssh -o StrictHostKeyChecking=accept-new' git pull",
            "export GIT_SSH_COMMAND=\"ssh -oStrictHostKeyChecking=no\"",
            "git -c core.sshCommand='ssh -o StrictHostKeyChecking=no' fetch",
            "git config --global core.sshCommand 'ssh -o StrictHostKeyChecking=accept-new'",
            "rsync -e 'ssh -o StrictHostKeyChecking=no' -a src/ hpc.example.edu:dst/",
            "rsync -avze 'ssh -o StrictHostKeyChecking=no' src/ hpc.example.edu:dst/",
            "rsync -a --rsh='ssh -oStrictHostKeyChecking=no' src/ hpc.example.edu:dst/",
            "RSYNC_RSH='ssh -o StrictHostKeyChecking=no' rsync -a src/ hpc.example.edu:dst/",
            "ssh-keygen -R hpc.example.edu",
            "ssh-keygen -f ~/.ssh/known_hosts -R '[hpc.example.edu]:2222'",
            "cd /tmp && ssh -o StrictHostKeyChecking=no hpc.example.edu",
            "bash -c \"ssh -o StrictHostKeyChecking=no hpc.example.edu\"",
            "true | ssh -o StrictHostKeyChecking=accept-new hpc.example.edu 'cat > x'",
        ] {
            let found = trust_findings(command, &env);
            assert!(!found.is_empty(), "{command} must ask first");
            let described = found.describe();
            assert!(
                described.contains("host key"),
                "{command}: the card must say it is about a host key, got {described}"
            );
        }
    }

    /// Reads of a key, a connection that checks the key, and everything else stay
    /// promptless: the gate is about trusting a key, not about using ssh.
    #[test]
    fn criterion_seven_leaves_reads_and_checked_connections_alone() {
        let env = nix_env();
        for command in [
            "ssh hpc.example.edu",
            "ssh -o StrictHostKeyChecking=yes hpc.example.edu",
            "ssh -o StrictHostKeyChecking=ask hpc.example.edu",
            "ssh -p 2222 -o ConnectTimeout=5 hpc.example.edu uptime",
            "ssh-copy-id -i ~/.ssh/id_ed25519.pub hpc.example.edu",
            "ssh-keygen -F hpc.example.edu",
            "ssh-keygen -lf ~/.ssh/known_hosts",
            "ssh-keygen -t ed25519 -f ./key -N ''",
            "ssh-keyscan hpc.example.edu",
            "ssh-keyscan -t ed25519 hpc.example.edu | ssh-keygen -lf -",
            "cat ~/.ssh/known_hosts",
            "grep -i stricthostkeychecking ~/.ssh/config",
            "git clone git@hpc.example.edu:lab.git",
            "git -c core.sshCommand='ssh -i ~/.ssh/lab' fetch",
            "rsync -a src/ hpc.example.edu:dst/",
            "rsync -e ssh -a src/ hpc.example.edu:dst/",
            "echo StrictHostKeyChecking",
        ] {
            let found = trust_findings(command, &env);
            assert!(found.is_empty(), "{command} must not ask, got {found:?}");
        }
    }

    /// A `-F` file is read: one that trusts unchecked keys asks, one that checks them does
    /// not, and one this inspection cannot read is judged by the line that may be writing it.
    #[cfg(unix)]
    #[test]
    fn criterion_seven_reads_the_config_file_ssh_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let trusting = dir.path().join("trusting");
        let checking = dir.path().join("checking");
        std::fs::write(
            &trusting,
            "# lab\nHost *\n    StrictHostKeyChecking=accept-new\n",
        )
        .unwrap();
        std::fs::write(&checking, "Host *\n    StrictHostKeyChecking yes\n").unwrap();
        let env =
            EnvFacts::for_platform(Platform::host(), &dir.path().to_string_lossy(), "/home/me");
        let asks = trust_findings(&format!("ssh -F {} hpc", trusting.display()), &env);
        assert!(!asks.is_empty(), "a trusting -F file must ask");
        assert!(
            !trust_findings("scp -F trusting a hpc:", &env).is_empty(),
            "a relative -F file is read from the working directory"
        );
        assert!(trust_findings(&format!("ssh -F {} hpc", checking.display()), &env).is_empty());
        let written = trust_findings(
            "printf 'StrictHostKeyChecking no\\n' > /tmp/never-there && ssh -F /tmp/never-there/cfg hpc",
            &env,
        );
        assert!(
            !written.is_empty(),
            "a -F file the same line writes must ask"
        );
        assert!(trust_findings("ssh -F /nonexistent/cfg hpc", &env).is_empty());
    }

    /// A `-F` argument that is not a regular file (a FIFO here) is never opened, so the
    /// inspection cannot block on it; the line is judged by its own words instead.
    #[cfg(unix)]
    #[test]
    fn criterion_seven_never_blocks_on_a_config_that_is_not_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("cfg");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs");
        assert!(made.success(), "mkfifo made the fixture");
        let env = nix_env();
        let plain = format!("ssh -F {} hpc", fifo.display());
        let trusting = format!(
            "printf 'StrictHostKeyChecking no\\n' > {0} & ssh -F {0} hpc",
            fifo.display()
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let found = (
                trust_findings(&plain, &env).is_empty(),
                trust_findings(&trusting, &env).is_empty(),
            );
            let _ = sender.send(found);
        });
        let (plain_empty, trusting_empty) = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the inspection must not block on a FIFO");
        assert!(plain_empty, "a FIFO nobody writes trusts nothing");
        assert!(
            !trusting_empty,
            "a FIFO the same line fills is judged by the line"
        );
    }

    /// The other OpenSSH options that trust a key nobody checked, in every place an option is
    /// read: `KnownHostsCommand` (any command but `none`), and a user or global known-hosts
    /// file other than the default, which holds whatever was put there, such as a scan.
    #[test]
    fn criterion_seven_grades_the_known_hosts_options() {
        let env = nix_env();
        for command in [
            "ssh -o KnownHostsCommand='ssh-keyscan %H' hpc.example.edu",
            "ssh -o 'KnownHostsCommand /usr/bin/ssh-keyscan -t ed25519 %H' hpc.example.edu",
            "scp -oknownhostscommand=/tmp/scan results.csv hpc.example.edu:",
            "ssh-keyscan hpc.example.edu > /tmp/kh; ssh -o UserKnownHostsFile=/tmp/kh hpc.example.edu",
            "ssh -o UserKnownHostsFile=/dev/null hpc.example.edu",
            "ssh -o UserKnownHostsFile=none hpc.example.edu",
            "ssh -o 'UserKnownHostsFile ~/.ssh/known_hosts /tmp/kh' hpc.example.edu",
            "sftp -o GlobalKnownHostsFile=/tmp/kh hpc.example.edu",
            "ssh -o 'ProxyCommand ssh -o UserKnownHostsFile=/tmp/kh -W %h:%p gate' hpc",
            "GIT_SSH_COMMAND='ssh -o KnownHostsCommand=\"ssh-keyscan %H\"' git pull",
            "rsync -e 'ssh -o UserKnownHostsFile=/tmp/kh' -a src/ hpc.example.edu:dst/",
        ] {
            let found = trust_findings(command, &env);
            assert!(!found.is_empty(), "{command} must ask first");
            assert!(found.describe().contains("host key"), "{command}");
        }
        for command in [
            "ssh -o KnownHostsCommand=none hpc.example.edu",
            "ssh -o UserKnownHostsFile=~/.ssh/known_hosts hpc.example.edu",
            "ssh -o 'UserKnownHostsFile ~/.ssh/known_hosts ~/.ssh/known_hosts2' hpc.example.edu",
            "ssh -o UserKnownHostsFile=/home/me/.ssh/known_hosts hpc.example.edu",
            "ssh -o UserKnownHostsFile=%d/.ssh/known_hosts hpc.example.edu",
            "ssh -o GlobalKnownHostsFile=/etc/ssh/ssh_known_hosts hpc.example.edu",
            "grep -i KnownHostsCommand ~/.ssh/config",
        ] {
            let found = trust_findings(command, &env);
            assert!(found.is_empty(), "{command} must not ask, got {found:?}");
        }
    }

    /// A `-F` file is read for the same options, and for a `ProxyCommand` that runs a trusting
    /// ssh, which the file reading used to skip.
    #[cfg(unix)]
    #[test]
    fn criterion_seven_reads_the_known_hosts_options_in_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let env =
            EnvFacts::for_platform(Platform::host(), &dir.path().to_string_lossy(), "/home/me");
        for (name, body, asks) in [
            (
                "command",
                "Host *\n    KnownHostsCommand ssh-keyscan %H\n",
                true,
            ),
            ("file", "Host *\n    UserKnownHostsFile /tmp/kh\n", true),
            ("global", "Host *\n    GlobalKnownHostsFile=/tmp/kh\n", true),
            (
                "proxy",
                "Host hpc\n    ProxyCommand ssh -o StrictHostKeyChecking=no -W %h:%p gate\n",
                true,
            ),
            (
                "default",
                "Host *\n    UserKnownHostsFile ~/.ssh/known_hosts\n",
                false,
            ),
            ("none", "Host *\n    KnownHostsCommand none\n", false),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            let found = trust_findings(&format!("ssh -F {} hpc", path.display()), &env);
            assert_eq!(!found.is_empty(), asks, "{name}: {found:?}");
        }
        let written = trust_findings(
            "printf 'KnownHostsCommand ssh-keyscan %H\\n' > /tmp/never-there/cfg && ssh -F /tmp/never-there/cfg hpc",
            &env,
        );
        assert!(
            !written.is_empty(),
            "a -F file the same line writes must ask"
        );
    }

    /// PowerShell and cmd set the variable that carries an ssh command line too, and neither
    /// spelling is a POSIX `NAME=value` word. Graded on both a Unix host (a `pwsh -c` line is
    /// split the POSIX way first) and a Windows one, where PowerShell is the default shell.
    #[test]
    fn criterion_seven_reads_every_shells_spelling_of_the_ssh_variables() {
        let windows =
            EnvFacts::for_platform(Platform::Windows, r"C:\Users\me\proj", r"C:\Users\me");
        for env in [nix_env(), windows] {
            for command in [
                "$env:GIT_SSH_COMMAND = 'ssh -o StrictHostKeyChecking=no'",
                "$env:GIT_SSH_COMMAND='ssh -o StrictHostKeyChecking=no'; git pull",
                "$Env:git_ssh_command = \"ssh -o StrictHostKeyChecking=accept-new\"",
                "${env:GIT_SSH_COMMAND} = 'ssh -o StrictHostKeyChecking=no'",
                "Set-Item env:GIT_SSH_COMMAND -Value 'ssh -o StrictHostKeyChecking=no'",
                "[Environment]::SetEnvironmentVariable('GIT_SSH_COMMAND', 'ssh -o StrictHostKeyChecking=no', 'User')",
                "set GIT_SSH_COMMAND=ssh -o StrictHostKeyChecking=no",
                "set \"GIT_SSH_COMMAND=ssh -o StrictHostKeyChecking=no\"",
                "setx GIT_SSH_COMMAND \"ssh -o StrictHostKeyChecking=no\"",
                "cmd /c \"set RSYNC_RSH=ssh -o StrictHostKeyChecking=no && rsync -a src hpc:dst\"",
                "$env:ANSIBLE_HOST_KEY_CHECKING = 'False'",
                "set ANSIBLE_HOST_KEY_CHECKING=0",
            ] {
                let found = trust_findings(command, &env);
                assert!(
                    !found.is_empty(),
                    "{command} must ask first on {:?}",
                    env.platform
                );
                assert!(found.describe().contains("host key"), "{command}");
            }
            for command in [
                "$env:GIT_SSH_COMMAND = 'ssh -i lab'",
                "echo $env:GIT_SSH_COMMAND",
                "set GIT_SSH_COMMAND=ssh -i lab",
                "$env:MY_GIT_SSH_COMMAND_NOTE = 'ssh -o StrictHostKeyChecking=no'",
                "$env:ANSIBLE_HOST_KEY_CHECKING = 'True'",
            ] {
                let found = trust_findings(command, &env);
                assert!(
                    found.is_empty(),
                    "{command} must not ask on {:?}, got {found:?}",
                    env.platform
                );
            }
        }
    }

    /// The indirections: a wrapper program run in ssh's place (`GIT_SSH`, `scp -S`,
    /// `sftp -S`, a remote shell or `ProxyCommand` named by path) is read as a `-F` file is,
    /// `mosh --ssh` hands over a command line, and Ansible's switch turns the check off.
    #[cfg(unix)]
    #[test]
    fn criterion_seven_follows_wrappers_mosh_and_ansible() {
        let dir = tempfile::tempdir().unwrap();
        let trusting = dir.path().join("trusting-ssh");
        let checking = dir.path().join("checking-ssh");
        std::fs::write(
            &trusting,
            "#!/bin/sh\n# lab wrapper\nexec ssh -o StrictHostKeyChecking=no \"$@\"\n",
        )
        .unwrap();
        std::fs::write(&checking, "#!/bin/sh\nexec ssh -i ~/.ssh/lab \"$@\"\n").unwrap();
        let env = nix_env();
        let wrapped = |template: &str, wrapper: &Path| {
            trust_findings(&template.replace("{w}", &wrapper.to_string_lossy()), &env)
        };
        for template in [
            "GIT_SSH={w} git pull",
            "export GIT_SSH={w}",
            "$env:GIT_SSH = '{w}'",
            "scp -S {w} results.csv hpc.example.edu:",
            "sftp -S{w} hpc.example.edu",
            "rsync -e {w} -a src/ hpc.example.edu:dst/",
            "GIT_SSH_COMMAND={w} git pull",
            "ssh -o 'ProxyCommand {w} -W %h:%p gate' hpc.example.edu",
        ] {
            assert!(
                !wrapped(template, &trusting).is_empty(),
                "{template} must ask"
            );
            assert!(
                wrapped(template, &checking).is_empty(),
                "{template} with a wrapper that checks keys must not ask"
            );
        }
        for command in [
            "printf 'exec ssh -o StrictHostKeyChecking=no \"$@\"' > /tmp/never-there/w && GIT_SSH=/tmp/never-there/w git pull",
            "mosh --ssh=\"ssh -o StrictHostKeyChecking=no\" hpc.example.edu",
            "mosh --ssh 'ssh -o StrictHostKeyChecking=accept-new' hpc.example.edu",
            "ANSIBLE_HOST_KEY_CHECKING=False ansible-playbook site.yml",
            "export ANSIBLE_HOST_KEY_CHECKING=0",
            "env ANSIBLE_HOST_KEY_CHECKING=no ansible all -m ping",
        ] {
            assert!(!trust_findings(command, &env).is_empty(), "{command} must ask");
        }
        for command in [
            "GIT_SSH=/usr/bin/ssh git pull",
            "GIT_SSH=plink git pull",
            "GIT_SSH=/nonexistent/wrapper git pull",
            "scp -S /usr/bin/ssh results.csv hpc.example.edu:",
            "ssh -S /tmp/control hpc.example.edu",
            "mosh --ssh=\"ssh -p 2222\" hpc.example.edu",
            "ANSIBLE_HOST_KEY_CHECKING=True ansible-playbook site.yml",
            "echo $ANSIBLE_HOST_KEY_CHECKING",
            "GIT_SSH_VARIANT=ssh git pull",
        ] {
            let found = trust_findings(command, &env);
            assert!(found.is_empty(), "{command} must not ask, got {found:?}");
        }
    }

    /// sshfs hands its `-o` options to the ssh it runs, so every spelling ssh's own `-o` is read
    /// in asks here too: comma-separated among FUSE options, attached, any letter case, the
    /// known-hosts options, a `ProxyCommand`, an `-o ssh_command` that runs a trusting ssh, a
    /// `-F` file, the mount helpers' names and a shell wrapping it. A mount that checks keys
    /// does not.
    #[test]
    fn criterion_seven_grades_sshfs_options_in_every_spelling() {
        let env = nix_env();
        for command in [
            "sshfs -o StrictHostKeyChecking=no hpc.example.edu:/data /mnt/data",
            "sshfs hpc.example.edu:/data /mnt/data -o StrictHostKeyChecking=accept-new",
            "sshfs -oStrictHostKeyChecking=off hpc.example.edu: /mnt/data",
            "sshfs -o stricthostkeychecking=False hpc.example.edu: /mnt/data",
            "sshfs -o 'StrictHostKeyChecking no' hpc.example.edu: /mnt/data",
            "sshfs -o allow_other,reconnect,StrictHostKeyChecking=no hpc.example.edu: /mnt/data",
            "sshfs -o reconnect -o ServerAliveInterval=15,StrictHostKeyChecking=accept-new hpc: /mnt",
            "sshfs -o UserKnownHostsFile=/dev/null hpc.example.edu: /mnt/data",
            "sshfs -o GlobalKnownHostsFile=/tmp/kh hpc.example.edu: /mnt/data",
            "sshfs -o 'KnownHostsCommand=ssh-keyscan %H' hpc.example.edu: /mnt/data",
            "sshfs -o 'ProxyCommand=ssh -o StrictHostKeyChecking=no -W %h:%p gate' hpc: /mnt",
            "sshfs -o ssh_command='ssh -o StrictHostKeyChecking=no' hpc.example.edu: /mnt/data",
            "sshfs -o 'reconnect,ssh_command=sshpass -e ssh -oStrictHostKeyChecking=accept-new' hpc: /mnt",
            "sshfs -o 'SSH_COMMAND=ssh -o UserKnownHostsFile=/tmp/kh' hpc: /mnt",
            r"sshfs -o 'ssh_command=ssh\ -o\ StrictHostKeyChecking=no' hpc: /mnt",
            "printf 'StrictHostKeyChecking no\\n' > /tmp/never-there/cfg && sshfs -F /tmp/never-there/cfg hpc: /mnt",
            "/usr/bin/sshfs -o StrictHostKeyChecking=no hpc.example.edu: /mnt/data",
            "sudo sshfs -o allow_other,StrictHostKeyChecking=no hpc: /mnt",
            "mount.fuse.sshfs hpc.example.edu:/data /mnt/data -o StrictHostKeyChecking=no",
            "mount.sshfs hpc.example.edu:/data /mnt/data -o UserKnownHostsFile=/dev/null",
            "bash -c \"sshfs -o StrictHostKeyChecking=no hpc: /mnt\"",
        ] {
            let found = trust_findings(command, &env);
            assert!(!found.is_empty(), "{command} must ask first");
            let described = found.describe();
            assert!(described.contains("host key"), "{command}: {described}");
            assert!(described.contains("sshfs"), "{command}: {described}");
        }
        for command in [
            "sshfs hpc.example.edu:/data /mnt/data",
            "sshfs -o StrictHostKeyChecking=yes hpc.example.edu: /mnt/data",
            "sshfs -o reconnect,ServerAliveInterval=15,allow_other hpc: /mnt",
            "sshfs -p 2222 -o IdentityFile=~/.ssh/lab hpc: /mnt",
            "sshfs -o ssh_command='ssh -i ~/.ssh/lab' hpc: /mnt",
            "sshfs -o UserKnownHostsFile=~/.ssh/known_hosts hpc: /mnt",
            "sshfs -o KnownHostsCommand=none hpc: /mnt",
            "sshfs -F /nonexistent/cfg hpc: /mnt",
            "fusermount -u /mnt/data",
            "umount /mnt/data",
        ] {
            let found = trust_findings(command, &env);
            assert!(found.is_empty(), "{command} must not ask, got {found:?}");
        }
    }

    /// sshfs reads a `-F` file and runs an `-o ssh_command` wrapper the way ssh and git do:
    /// the file is read, a trusting one asks and a checking one does not.
    #[cfg(unix)]
    #[test]
    fn criterion_seven_reads_the_files_sshfs_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let trusting = dir.path().join("trusting");
        let checking = dir.path().join("checking");
        std::fs::write(&trusting, "Host *\n    StrictHostKeyChecking accept-new\n").unwrap();
        std::fs::write(&checking, "Host *\n    StrictHostKeyChecking yes\n").unwrap();
        let wrapper = dir.path().join("trusting-ssh");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nexec ssh -o StrictHostKeyChecking=no \"$@\"\n",
        )
        .unwrap();
        let env =
            EnvFacts::for_platform(Platform::host(), &dir.path().to_string_lossy(), "/home/me");
        for (command, asks) in [
            (format!("sshfs -F {} hpc: /mnt", trusting.display()), true),
            (format!("sshfs hpc: /mnt -F{}", trusting.display()), true),
            ("sshfs -F trusting hpc: /mnt".to_owned(), true),
            (format!("sshfs -F {} hpc: /mnt", checking.display()), false),
            (
                format!("sshfs -o ssh_command={} hpc: /mnt", wrapper.display()),
                true,
            ),
            (
                format!("sshfs -o ssh_command={} hpc: /mnt", checking.display()),
                false,
            ),
        ] {
            let found = trust_findings(&command, &env);
            assert_eq!(!found.is_empty(), asks, "{command}: {found:?}");
        }
    }

    #[test]
    fn fuse_options_split_at_commas_and_keep_an_escaped_one() {
        assert_eq!(
            fuse_options(r"allow_other,ssh_command=ssh\ -o\ A=b\,c,,reconnect"),
            ["allow_other", "ssh_command=ssh -o A=b,c", "reconnect"]
        );
        assert!(fuse_options("").is_empty());
    }

    /// Files that name each other cannot keep the inspection reading: grading one line opens
    /// at most `MAX_TRUST_FILES` files, and a wrapper that names itself on every line spends
    /// exactly that budget and no more.
    #[cfg(unix)]
    #[test]
    fn criterion_seven_opens_a_bounded_number_of_files() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("loop");
        let line = format!("{} -W %h:%p\n", wrapper.display());
        std::fs::write(&wrapper, line.repeat(500)).unwrap();
        let env = nix_env();
        let command = format!("GIT_SSH={} git pull", wrapper.display());
        let scan = TrustScan::new(&env, &command);
        let words = shell_words(&command);
        assert_eq!(words_trust_host_key(&words, &scan), None);
        assert_eq!(scan.files_left.get(), 0, "the whole budget was the bound");
    }

    /// What criterion 7 leaves to criterion 4: a write into known_hosts already asks, because
    /// `.ssh` is a credential path.
    #[test]
    fn a_key_appended_to_known_hosts_stays_a_credential_write() {
        let env = nix_env();
        for command in [
            "ssh-keyscan hpc.example.edu >> ~/.ssh/known_hosts",
            "ssh-keyscan hpc.example.edu | tee -a ~/.ssh/known_hosts",
        ] {
            assert!(
                command_writes_sensitively(command, &env).is_some(),
                "{command} must still ask"
            );
        }
    }

    /// Gate (c) for criterion 7, at the inspector: Fully Automatic asks with a card that names
    /// the host key, and every other mode stays inert, as for every other criterion.
    #[tokio::test]
    async fn criterion_seven_asks_in_auto_mode_and_nowhere_else() {
        let requests: Vec<ToolRequest> = [
            "ssh -o StrictHostKeyChecking=accept-new you@hpc.example.edu true",
            "ssh-keygen -R hpc.example.edu",
            "GIT_SSH_COMMAND='ssh -o StrictHostKeyChecking=no' git pull",
        ]
        .iter()
        .enumerate()
        .map(|(i, command)| shell_request(&format!("req_{i}"), command))
        .collect();
        let session = session_at(Path::new("/home/me/proj"), chrono::Utc::now());
        let auto = SensitiveOpsInspector::unbound()
            .inspect(&requests, &[], BioRouterMode::Auto, &session)
            .await
            .unwrap();
        assert_eq!(auto.len(), requests.len(), "{auto:?}");
        for result in &auto {
            let InspectionAction::RequireApproval(Some(prompt)) = &result.action else {
                panic!("expected an approval prompt, got {:?}", result.action);
            };
            assert!(prompt.contains("host key"), "{prompt}");
            assert_eq!(result.inspector_name, SENSITIVE_OPS_INSPECTOR_NAME);
        }
        for mode in [
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
            BioRouterMode::Chat,
        ] {
            let results = SensitiveOpsInspector::unbound()
                .inspect(&requests, &[], mode, &session)
                .await
                .unwrap();
            assert!(
                results.is_empty(),
                "{mode:?} must be inert for criterion 7, got {results:?}"
            );
        }
    }
}
