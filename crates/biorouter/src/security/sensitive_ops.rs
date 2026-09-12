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
//! And one criterion that is **not** a file operation at all:
//!
//!   5. a call to a tool that **captures the screen or drives the computer**
//!      ([`SCREEN_AND_CONTROL_TOOLS`]) made by a model whose provider is
//!      [`ProviderTier::Public`].
//!
//! # Criterion 6: ambient capture by a public model
//!
//! `claude_code` and `codex` run inference on the user's own consumer
//! subscription, which carries **no Business Associate Agreement and no
//! zero-data-retention agreement** — which is exactly why they are declared
//! `ProviderTier::Public`. They hold the full Developer and Computer Controller
//! rosters, and two of those tools are unlike every other one in this module:
//!
//!   * `screen_capture` sends a picture of the screen to the model.
//!   * `computer_control` drives the machine (AppleScript / PowerShell / desktop
//!     automation).
//!
//! **A file read is bounded; a screenshot is ambient.** `.biorouterignore`, the
//! `SecretGuard` and the working directory bound what a *file* operation can
//! reach, and all three work because the agent named the file. A screenshot
//! names nothing — it captures whatever happens to be on screen at that instant,
//! an EHR window or a patient chart included, none of which is a file anything
//! chose to read. PHI that never touched disk can leave the machine in a PNG,
//! and no path-based control in this module can see it coming.
//!
//! The existing `ensure_screen_capture_permitted` is a different control
//! answering a different question: it is the macOS TCC screen-recording grant to
//! **Biorouter as an application** — "may this app screenshot at all" — taken
//! once, and it knows nothing about which model receives the image.
//!
//! So this is an **ask, and only an ask**. The user can approve it and proceed,
//! including standing in front of a public model and deciding a screenshot is
//! fine. It is not a block, and it never becomes one.
//!
//! Three decisions worth stating, because each of them is a place a future
//! change could quietly gut the criterion:
//!
//!   * **`automation_script` is deliberately NOT in the set.** It runs a
//!     shell/Ruby/PowerShell/Batch script the model wrote — it is `developer/shell`'s
//!     twin, not a screen or computer driver. Escalating it would escalate
//!     ordinary scripting on every public model, which is the same over-broad
//!     rule that makes escalating `developer/shell` itself unacceptable: a grant
//!     that prompts on everything is a grant nobody keeps enabled. The honest
//!     gap this leaves — a shell (or an `automation_script`) that runs
//!     `screencapture` or `osascript` by hand — is *identical* to the one
//!     `developer/shell` already has, and closing it belongs to a command-level
//!     rule over the command text, not to a rule over tool names.
//!   * **`list_windows` is in the set even though it is no longer advertised.**
//!     It was folded into `screen_capture { list_only: true }`, but the retired
//!     name still dispatches, and window titles are ambient in exactly the way a
//!     screenshot is ("Epic — DOE, JANE"). A criterion a model can dodge by
//!     naming a retired tool is not a criterion.
//!   * **The tier is read off the [`CallCapability`] threaded into the call, and
//!     the master privacy switch does not turn this off.** See
//!     [`SensitiveOpsInspector::caller_tier`] for both, and for what an absent
//!     capability means.
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
use crate::privacy::{CallCapability, ProviderTier};
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

/// Criterion 6's set: the tools that capture the screen or drive the machine,
/// each paired with the clause that describes it to the user.
///
/// Keyed by the **unprefixed** tool name, because that is the only spelling that
/// is stable across the three shapes one of these calls arrives in: the
/// server-prefixed `developer__screen_capture` a model emits, the bare
/// `screen_capture` a bridged or renamed roster can carry, and the
/// `screen_capture({…})` callee inside an `execute_code` body (where the import
/// is destructured, so the module prefix is gone by the time the call is
/// written). See [`unprefixed_tool_name`].
///
/// ⚠ This set is about **ambient** reach, not about danger in general. A tool
/// belongs here when its blast radius is "whatever is in front of the user right
/// now" rather than "the argument the model passed". That is the line that keeps
/// `developer__shell` and `computercontroller__automation_script` out of it —
/// see the module docs, where the reasoning and its residual gap are stated.
const SCREEN_AND_CONTROL_TOOLS: &[(&str, &str)] = &[
    (
        "screen_capture",
        "sends a picture of whatever is on your screen right now to the model",
    ),
    (
        "list_windows",
        "reads the titles of every window open on your screen right now",
    ),
    (
        "computer_control",
        "drives your computer directly, through AppleScript, PowerShell or desktop automation",
    ),
];

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

/// A tool name with its `server__` prefix stripped, lowercased.
///
/// `developer__screen_capture` and a bare `screen_capture` are the same tool,
/// and criterion 6 must not be dodgeable by the spelling a roster happens to
/// use. `rsplit_once` rather than `split_once` so a hypothetical
/// `a__b__capture` still yields its final segment.
fn unprefixed_tool_name(tool_name: &str) -> String {
    let name = tool_name.trim();
    name.rsplit_once("__")
        .map_or(name, |(_, tool)| tool)
        .to_ascii_lowercase()
}

/// Criterion 6's classifier for one *named* call: `Some((tool, what it does))`
/// when the name is in [`SCREEN_AND_CONTROL_TOOLS`].
fn screen_or_control_tool(tool_name: &str) -> Option<(&'static str, &'static str)> {
    let base = unprefixed_tool_name(tool_name);
    SCREEN_AND_CONTROL_TOOLS
        .iter()
        .find(|(name, _)| *name == base)
        .copied()
}

/// Criterion 6's classifier for one tool call, **including one made from inside
/// an `execute_code` body**.
///
/// The code-execution path is not a nicety. Its inner calls are handed straight
/// to the extension manager and never reach an agent-layer inspector (the same
/// reason §3 of the boundary docs exists), and the live app has been observed
/// routing `developer__shell` through `code_execution` — so a rule the model can
/// dodge by wrapping the call in a script is not a rule. A callee inside the
/// script is matched by the same [`screen_or_control_tool`] the outer name is,
/// so the two can never drift.
///
/// Unlike the write scan, this one asks nothing about arguments: `screen_capture`
/// with no arguments at all still captures the screen. That also makes it
/// deliberately conservative inside a script — [`extract_inner_calls`] reports
/// every `ident(` outside a string literal, so a local helper (or a commented-out
/// line) that happens to be spelled `screen_capture(` matches. Erring that way is
/// correct for a criterion whose only outcome is an ask the user can approve.
///
/// ⚠ `args` is OPTIONAL, and that is load-bearing rather than convenience. A
/// `screen_capture` with **no arguments object at all** is not a degenerate
/// call — it is the DEFAULT one, capturing the primary display. Several
/// producers really do send it that way: `routes/tool_bridge.rs` maps a missing
/// or null MCP `arguments` to `None`, and Google's parser yields `None` when a
/// `functionCall` omits `args`. Taking `&Map` here forced the caller behind the
/// `let Some(args) = … else { continue }` guard that criteria 1–5 need for a
/// path, which skipped this criterion entirely for exactly those calls.
pub fn screen_or_control_call(
    tool_name: &str,
    args: Option<&Map<String, Value>>,
) -> Option<(&'static str, &'static str)> {
    if let Some(hit) = screen_or_control_tool(tool_name) {
        return Some(hit);
    }
    if is_execute_code(tool_name) {
        if let Some(code) = args.and_then(|a| a.get("code")).and_then(Value::as_str) {
            for call in extract_inner_calls(code) {
                if let Some(hit) = screen_or_control_tool(&call.callee) {
                    return Some(hit);
                }
            }
        }
    }
    None
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
/// sensitive *location*. Criterion 6 writes its own (it has to name what would
/// be destroyed, not which rule matched).
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

/// Every criterion a shell command line can trip: the four path-classification
/// ones (via [`command_writes_sensitively`]) and criterion 5 — and every hit of
/// each, not the first.
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
pub struct SensitiveOpsInspector {
    /// The provider to sample when a caller hands this inspector no admitted
    /// [`CallCapability`] — the ordinary agent-loop path, which inspects
    /// *before* the dispatch that would admit one.
    ///
    /// `None` is "there is nothing to fall back on", and it resolves to
    /// [`ProviderTier::Public`]. See [`Self::caller_tier`].
    provider: Option<SharedProvider>,
}

impl SensitiveOpsInspector {
    /// The production constructor: fall back to sampling `provider` for callers
    /// that supply no capability of their own.
    #[must_use]
    pub fn new(provider: SharedProvider) -> Self {
        Self {
            provider: Some(provider),
        }
    }

    /// For a stack whose every caller pins a capability at the seam — the
    /// coding-agent bridge and the workflow tool turn both do — so there is no
    /// provider here to sample and no pretence that there is.
    ///
    /// Reaching this constructor from a path that does *not* pin a capability is
    /// safe rather than silent: criterion 5 then reads the caller as Public and
    /// asks. It is the wrong constructor, not a hole.
    #[must_use]
    pub fn unbound() -> Self {
        Self { provider: None }
    }

    /// The tier of the model this batch was admitted on — criterion 5's second
    /// condition.
    ///
    /// ⚠ **The threaded capability is asked, never re-derived.** A
    /// [`CallCapability`] is sampled ONCE per call and carried precisely because
    /// `Agent::update_provider` can reassign the provider mutex with no turn
    /// lock; a gate that re-read the provider here would be the second read that
    /// type exists to collapse. So `Some(_)` short-circuits before the provider
    /// field is even looked at.
    ///
    /// ⚠ **`None` fails CLOSED.** The trait's default `inspect_with_capability`
    /// forwards without one, so an absent capability is the *normal* state on
    /// some paths rather than an error, and a gate that quietly did nothing
    /// there would be a gate that no-ops on exactly the entries nobody
    /// classified. With a provider in hand the honest answer is a fresh sample —
    /// the ordinary agent loop's real tier, which is what keeps a *private*
    /// model from being prompted for a screenshot that never leaves the
    /// institution. With no provider either, the answer is `Public`: the most
    /// restrictive reading this type can express, and the one that asks.
    ///
    /// ⚠ **`enforced()` is deliberately NOT consulted, so the privacy master
    /// switch does not turn criterion 5 off.** That switch governs privacy
    /// *refusals* (DR-15: with tiers off nothing is refused). This refuses
    /// nothing — it is an Auto-mode approval in the sensitive-ops gate, whose
    /// other four criteria are not switchable either, and whose outcome the user
    /// can always grant. Gating it on the switch would delete an ambient-capture
    /// prompt for a class of provider that has no BAA, silently, from a control
    /// that was never part of the privacy-tier feature.
    ///
    /// Sampled at most once per batch, and only after a name check has
    /// established that the batch contains such a call at all: an ordinary turn
    /// must not pay a provider-mutex read for a criterion that cannot apply to
    /// it.
    async fn caller_tier(&self, capability: Option<CallCapability>) -> ProviderTier {
        if let Some(capability) = capability {
            return capability.tier();
        }
        match &self.provider {
            Some(provider) => CallCapability::sample(provider).await.tier(),
            None => ProviderTier::Public,
        }
    }

    /// The whole gate, over one batch. Both trait entry points funnel here, so
    /// the capability-carrying one can never grow behaviour the plain one lacks.
    async fn inspect_batch(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
        capability: Option<CallCapability>,
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
        // Criterion 5's candidates, gathered before any provider is sampled.
        let mut ambient: Vec<(&ToolRequest, &'static str, &'static str)> = Vec::new();

        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            // BEFORE the `args` guard below, deliberately. That guard exists
            // for criteria 1-5, which all need a PATH out of the arguments; this
            // criterion needs only the tool name, and a `screen_capture` with no
            // arguments object is the default call rather than a malformed one.
            // Leaving it below the guard let the whole criterion be skipped by
            // omitting `arguments` — reachable today through the coding-agent
            // bridge and through Google's function-call parser.
            if let Some((tool, what)) =
                screen_or_control_call(&tool_call.name, tool_call.arguments.as_ref())
            {
                ambient.push((request, tool, what));
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

        if ambient.is_empty() {
            return Ok(results);
        }
        if self.caller_tier(capability).await.is_private() {
            // A model covered by the user's own institution: the capture never
            // leaves it, so criterion 5 has nothing to ask about.
            return Ok(results);
        }

        for (request, tool, what) in ambient {
            tracing::warn!(
                counter.biorouter.sensitive_op_escalated = 1,
                tool_name = %tool,
                tool_request_id = %request.id,
                "Screen/computer-control call by a public-tier model escalated to approval in Auto mode"
            );
            results.push(InspectionResult {
                tool_request_id: request.id.clone(),
                action: InspectionAction::RequireApproval(Some(format!(
                    "🔒 Screen and computer access by a public model, in Fully-Automatic mode.\n\
                     This tool call runs `{tool}`, which {what}. The model answering this \
                     chat runs on a public provider — a consumer plan with no Business \
                     Associate Agreement and no zero-data-retention agreement — so whatever \
                     is visible goes to it.\n\
                     Approve it if that is fine for what is on your screen right now, or deny \
                     it. Biorouter asks every time because a capture takes whatever happens \
                     to be in front of you, not a file anything chose to open."
                ))),
                reason: format!("Public-tier model calling {tool} ({what})"),
                confidence: 1.0,
                inspector_name: self.name().to_string(),
                finding_id: Some(format!("SENS-{}", Uuid::new_v4().simple())),
            });
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

    /// Criterion 5 needs the caller's tier, and the tier a call was admitted on
    /// arrives here — sampled once at the seam and threaded — rather than being
    /// re-read.
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

    // --- criterion 5: ambient capture by a public-tier model ---------------

    /// The marker that a result came from criterion 5 rather than from one of the
    /// four path-based ones. Asserting on `RequireApproval` alone would let a
    /// spurious *filesystem* finding stand in for the tier finding these tests are
    /// about, which is the way this whole block could go green while measuring
    /// nothing.
    fn ambient_findings(results: &[InspectionResult]) -> Vec<&InspectionResult> {
        results
            .iter()
            .filter(|r| r.reason.starts_with("Public-tier model calling"))
            .collect()
    }

    fn call_request(id: &str, name: &str, arguments: Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: name.to_string().into(),
                arguments: Some(args(arguments)),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    /// One call, through the capability-carrying entry point, on the `unbound()`
    /// inspector — so what a test measures is the capability it passed, never a
    /// provider it did not set up.
    async fn inspect_call(
        name: &str,
        arguments: Value,
        mode: BioRouterMode,
        capability: Option<CallCapability>,
    ) -> Vec<InspectionResult> {
        SensitiveOpsInspector::unbound()
            .inspect_with_capability(
                &[call_request("req_ambient", name, arguments)],
                &[],
                mode,
                &crate::session::Session::default(),
                capability,
            )
            .await
            .unwrap()
    }

    fn public() -> Option<CallCapability> {
        Some(CallCapability::for_test(ProviderTier::Public, true))
    }

    fn private() -> Option<CallCapability> {
        Some(CallCapability::for_test(ProviderTier::Private, true))
    }

    /// The criterion, both ways round, for both named tools: a public model asks,
    /// a private one does not. This is the whole product decision in one test —
    /// a screenshot bound for a consumer plan with no BAA is offered to the user,
    /// and one bound for the user's own institution is not.
    #[tokio::test]
    async fn a_public_model_asks_before_capturing_the_screen_and_a_private_one_does_not() {
        for (tool, arguments) in [
            ("developer__screen_capture", json!({})),
            (
                "computercontroller__computer_control",
                json!({"script": "tell application \"Safari\" to activate"}),
            ),
        ] {
            let asked = inspect_call(tool, arguments.clone(), BioRouterMode::Auto, public()).await;
            let findings = ambient_findings(&asked);
            assert_eq!(
                findings.len(),
                1,
                "a public model calling {tool} must ask, got {asked:?}"
            );
            assert!(matches!(
                findings[0].action,
                InspectionAction::RequireApproval(_)
            ));
            assert_eq!(findings[0].inspector_name, "sensitive_ops");
            assert_eq!(findings[0].tool_request_id, "req_ambient");

            let quiet = inspect_call(tool, arguments, BioRouterMode::Auto, private()).await;
            assert!(
                ambient_findings(&quiet).is_empty(),
                "a private model calling {tool} must NOT ask, got {quiet:?}"
            );
        }
    }

    /// The retired `list_windows` name still dispatches, and window titles are
    /// ambient in exactly the way a frame buffer is. A criterion a model can dodge
    /// by naming a folded-away tool is not a criterion.
    #[tokio::test]
    async fn the_retired_window_inventory_tool_is_in_the_set() {
        let results = inspect_call(
            "developer__list_windows",
            json!({}),
            BioRouterMode::Auto,
            public(),
        )
        .await;
        assert_eq!(ambient_findings(&results).len(), 1, "{results:?}");
    }

    /// Reached through `execute_code`, which is the shape that matters: a script's
    /// inner calls are handed straight to the extension manager and never reach an
    /// agent-layer inspector, and the live app has been observed routing
    /// `developer__shell` through `code_execution`. A rule the model can dodge by
    /// wrapping the call in a script is not a rule.
    #[tokio::test]
    async fn a_capture_hidden_inside_execute_code_still_asks() {
        let results = inspect_call(
            "code_execution__execute_code",
            json!({
                "code": "import { screen_capture } from \"developer\";\n\
                         const shot = screen_capture({ display: 0 });\n\
                         record_result(shot);"
            }),
            BioRouterMode::Auto,
            public(),
        )
        .await;
        assert_eq!(
            ambient_findings(&results).len(),
            1,
            "a screen capture inside execute_code must ask, got {results:?}"
        );

        // And the same body under a private model stays silent, so the escalation
        // is the TIER's doing and not merely the script's shape.
        let quiet = inspect_call(
            "code_execution__execute_code",
            json!({
                "code": "import { computer_control } from \"computercontroller\";\n\
                         computer_control({ script: \"tell application \\\"Finder\\\" to activate\" });"
            }),
            BioRouterMode::Auto,
            private(),
        )
        .await;
        assert!(ambient_findings(&quiet).is_empty(), "{quiet:?}");
    }

    /// An absent capability is the trait default's normal state, not an error, and
    /// on an `unbound()` inspector there is nothing to sample. It must read as
    /// Public and ask — a gate that silently no-ops when the capability is missing
    /// is the failure this fallback exists to prevent.
    /// A call carrying **no `arguments` object at all** must still be graded.
    ///
    /// ⚠ This is the shape the criterion was silently blind to. The
    /// `let Some(args) = … else { continue }` guard that criteria 1-5 need for a
    /// path sat ABOVE the criterion's candidate collection, so omitting
    /// `arguments` skipped the whole rule — and `screen_capture` with no
    /// arguments is not a malformed call, it is the DEFAULT one (it captures the
    /// primary display). Two production producers really do send it that way:
    /// `routes/tool_bridge.rs` maps a missing/null MCP `arguments` to `None`,
    /// and Google's function-call parser yields `None` when `args` is omitted.
    ///
    /// Every other test in this module builds requests with
    /// `arguments: Some(…)`, which is precisely why the hole survived them.
    #[tokio::test]
    async fn a_call_with_no_arguments_object_is_still_graded() {
        let bare = ToolRequest {
            id: "req-bare".to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "developer__screen_capture".to_string().into(),
                arguments: None,
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        };
        let results = SensitiveOpsInspector::unbound()
            .inspect_with_capability(
                std::slice::from_ref(&bare),
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
                public(),
            )
            .await
            .expect("inspection runs");
        assert!(
            results
                .iter()
                .any(|r| r.reason.starts_with("Public-tier model calling")),
            "a screen capture with no arguments object is the DEFAULT call, not a \
             malformed one, and must still ask; got {:?}",
            results.iter().map(|r| &r.reason).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn an_absent_capability_fails_closed_and_asks() {
        let results = inspect_call(
            "developer__screen_capture",
            json!({}),
            BioRouterMode::Auto,
            None,
        )
        .await;
        assert_eq!(
            ambient_findings(&results).len(),
            1,
            "no capability must be read as public, got {results:?}"
        );

        // The plain `inspect` entry point forwards without a capability, so it must
        // reach the identical verdict: the two entry points cannot diverge.
        let plain = SensitiveOpsInspector::unbound()
            .inspect(
                &[call_request(
                    "req_ambient",
                    "developer__screen_capture",
                    json!({}),
                )],
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert_eq!(ambient_findings(&plain).len(), 1, "{plain:?}");
    }

    /// Criterion 6 is inert outside Auto, exactly as the other five are: those
    /// modes already gate every tool call, and the module's one early return is
    /// what keeps that provable.
    #[tokio::test]
    async fn criterion_six_is_inert_outside_auto_mode() {
        for mode in [
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
            BioRouterMode::Chat,
        ] {
            for tool in [
                "developer__screen_capture",
                "computercontroller__computer_control",
            ] {
                let results = inspect_call(tool, json!({"script": "x"}), mode, public()).await;
                assert!(
                    results.is_empty(),
                    "{mode:?} must be inert for {tool}, got {results:?}"
                );
            }
        }
    }

    /// The load-bearing negative. A public model was granted the Developer roster
    /// so it could do work; if criterion 5 escalated ordinary Developer tools the
    /// grant would prompt on everything and be switched off, which is worse than
    /// not having it. `automation_script` is here for the same reason and is the
    /// decision recorded in the module docs: it runs a script the model wrote, so
    /// it is `shell`'s twin rather than a screen or computer driver.
    #[tokio::test]
    async fn ordinary_tools_are_not_escalated_by_criterion_five_on_a_public_model() {
        for (tool, arguments) in [
            ("developer__shell", json!({"command": "ls -la /tmp"})),
            (
                "developer__text_editor",
                json!({"command": "write", "path": "/tmp/out.txt", "file_text": "hi"}),
            ),
            (
                "computercontroller__automation_script",
                json!({"language": "shell", "script": "sort file.txt | uniq"}),
            ),
            (
                "computercontroller__web_scrape",
                json!({"url": "https://example.org"}),
            ),
            ("developer__analyze", json!({"path": "/tmp/x.rs"})),
        ] {
            let results = inspect_call(tool, arguments, BioRouterMode::Auto, public()).await;
            assert!(
                ambient_findings(&results).is_empty(),
                "{tool} must NOT be escalated by criterion 5, got {results:?}"
            );
        }
    }

    /// The approval copy is what the user acts on, so pin its shape: it names the
    /// tool, states why it is being asked, and is an ASK — never a block.
    #[tokio::test]
    async fn the_approval_message_names_the_tool_and_never_says_blocked() {
        let results = inspect_call(
            "developer__screen_capture",
            json!({}),
            BioRouterMode::Auto,
            public(),
        )
        .await;
        let finding = ambient_findings(&results)[0];
        let InspectionAction::RequireApproval(Some(message)) = &finding.action else {
            panic!("criterion 5 must ask, not deny: {:?}", finding.action);
        };
        assert!(message.contains("screen_capture"), "{message}");
        assert!(
            message.contains("Business Associate Agreement"),
            "{message}"
        );
        assert!(message.contains("Approve it"), "{message}");
        let lowered = message.to_ascii_lowercase();
        assert!(
            !lowered.contains("blocked") && !lowered.contains("not allowed"),
            "the user can approve this; the copy must not read as a block: {message}"
        );
    }

    // --- criterion 5's name matching (pure) --------------------------------

    #[test]
    fn a_tool_is_matched_prefixed_or_bare() {
        for name in [
            "developer__screen_capture",
            "screen_capture",
            "SCREEN_CAPTURE",
            "computercontroller__computer_control",
            "computer_control",
        ] {
            assert!(
                screen_or_control_tool(name).is_some(),
                "{name} must be in the screen/control set"
            );
        }
        for name in [
            "developer__shell",
            "shell",
            "computercontroller__automation_script",
            "automation_script",
            "developer__text_editor",
            // Substring-matching would have taken this one; the comparison is on
            // the whole unprefixed name.
            "knowledge__kb_screen_capture_notes",
        ] {
            assert!(
                screen_or_control_tool(name).is_none(),
                "{name} must NOT be in the screen/control set"
            );
        }
    }

    #[test]
    fn an_execute_code_body_without_a_capture_is_not_a_candidate() {
        assert!(screen_or_control_call(
            "code_execution__execute_code",
            Some(&args(json!({
                "code": "import { shell } from \"developer\";\nshell({ command: \"ls\" });"
            }))),
        )
        .is_none());
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
}
