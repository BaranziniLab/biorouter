use std::{env, ffi::OsString, path::Path, process::Stdio, sync::Once};

pub use biorouter_sandbox::environment::{
    is_daemon_private_env_key, strip_daemon_private_env, strip_daemon_private_env_std,
};
use biorouter_sandbox::shell_sandbox::{
    self, SandboxMode, SandboxPolicy, SandboxReport, SandboxTier,
};

#[cfg(unix)]
#[allow(unused_imports)] // False positive: trait is used for process_group method
use std::os::unix::process::CommandExt;

#[derive(Debug, Clone)]
pub struct ShellConfig {
    pub executable: String,
    pub args: Vec<String>,
    #[allow(dead_code)]
    pub envs: Vec<(OsString, OsString)>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        #[cfg(windows)]
        {
            Self::detect_windows_shell()
        }
        #[cfg(not(windows))]
        {
            let shell = env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
            Self {
                executable: shell,
                args: vec!["-c".to_string()], // -c is standard across bash/zsh/fish
                envs: vec![],
            }
        }
    }
}

impl ShellConfig {
    #[cfg(windows)]
    fn detect_windows_shell() -> Self {
        // Check for PowerShell first (more modern)
        if let Ok(ps_path) = which::which("pwsh") {
            // PowerShell 7+ (cross-platform PowerShell)
            Self {
                executable: ps_path.to_string_lossy().to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                ],
                envs: vec![],
            }
        } else if let Ok(ps_path) = which::which("powershell") {
            // Windows PowerShell 5.1
            Self {
                executable: ps_path.to_string_lossy().to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                ],
                envs: vec![],
            }
        } else {
            // Fall back to cmd.exe
            Self {
                executable: "cmd".to_string(),
                args: vec!["/c".to_string()],
                envs: vec![],
            }
        }
    }
}

pub fn expand_path(path_str: &str) -> String {
    if cfg!(windows) {
        // Expand Windows environment variables (%VAR%)
        let with_userprofile = path_str.replace(
            "%USERPROFILE%",
            &env::var("USERPROFILE").unwrap_or_default(),
        );
        // Add more Windows environment variables as needed
        with_userprofile.replace("%APPDATA%", &env::var("APPDATA").unwrap_or_default())
    } else {
        // Unix-style expansion
        shellexpand::tilde(path_str).into_owned()
    }
}

pub fn is_absolute_path(path_str: &str) -> bool {
    let path = Path::new(path_str);
    path.is_absolute() || (cfg!(windows) && path.has_root())
}

pub fn normalize_line_endings(text: &str) -> String {
    if cfg!(windows) {
        // Ensure CRLF line endings on Windows
        text.replace("\r\n", "\n").replace("\n", "\r\n")
    } else {
        // Ensure LF line endings on Unix
        text.replace("\r\n", "\n")
    }
}

/// Parse a plain truthy env value (`1`/`true`/`on`/`yes`, case-insensitive,
/// trimmed). Used for `BIOROUTER_SHELL_SANDBOX_NETWORK`; the main gate
/// `BIOROUTER_SHELL_SANDBOX` uses the three-valued [`SandboxMode`] instead.
pub(crate) fn env_flag_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

fn env_truthy(name: &str) -> bool {
    env::var(name).map(|v| env_flag_truthy(&v)).unwrap_or(false)
}

/// Env knob for the foreground shell budget (issue #72). Seconds; `0` disables
/// the watchdog entirely.
pub const FOREGROUND_TIMEOUT_ENV: &str = "BIOROUTER_SHELL_FOREGROUND_TIMEOUT_SECS";

/// Default wall-clock budget for a **foreground** shell command.
///
/// This is deliberately *not* the generic per-extension MCP timeout (300 s by
/// default, and shared by every tool the extension offers). That timeout is too
/// coarse to be a foreground policy: when it fires the model gets an opaque
/// transport-level error, and it says nothing about which kind of work a
/// foreground command is supposed to be.
///
/// 240 s sits comfortably inside the 300 s generic timeout, so the shell's own
/// watchdog is what fires — the tree is killed by the tool itself instead of
/// depending on a cancellation round-trip, and the model gets an error that
/// names the command, how long it ran, and the remedy (`background=true`, then
/// `shell_status` / `shell_kill`, which this server already
/// offers for exactly this case).
pub const FOREGROUND_TIMEOUT_DEFAULT: std::time::Duration = std::time::Duration::from_secs(240);

/// How often a still-running foreground command reports itself to the client
/// (issue #72). Dead air is the other half of the report: a command with no
/// output looked identical to a hung agent.
pub const FOREGROUND_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(15);

/// Pure core of [`foreground_timeout_from_env`]: `None` (unset) keeps the
/// default, `Some("0")` disables the watchdog, anything unparseable keeps the
/// default rather than silently running unbounded.
pub(crate) fn parse_foreground_timeout(raw: Option<&str>) -> Option<std::time::Duration> {
    match raw.map(str::trim) {
        None | Some("") => Some(FOREGROUND_TIMEOUT_DEFAULT),
        Some(value) => match value.parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(std::time::Duration::from_secs(secs)),
            Err(_) => {
                tracing::warn!(
                    "{FOREGROUND_TIMEOUT_ENV}={value:?} is not a whole number of seconds; \
                     using the {}s default",
                    FOREGROUND_TIMEOUT_DEFAULT.as_secs()
                );
                Some(FOREGROUND_TIMEOUT_DEFAULT)
            }
        },
    }
}

/// The foreground shell budget this process was started with. Read once, at
/// `DeveloperServer` construction, so the value cannot change under a running
/// command.
pub fn foreground_timeout_from_env() -> Option<std::time::Duration> {
    parse_foreground_timeout(env::var(FOREGROUND_TIMEOUT_ENV).ok().as_deref())
}

/// `1m30s`-style rendering for the durations this module reports to a human and
/// to the model. Seconds only below a minute, so a short command does not get a
/// misleadingly precise `0m4s`.
pub fn humanize_secs(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

/// Build the mechanism-neutral [`SandboxPolicy`] for the shell tool: writable
/// roots are the session working dir (the natural project root) plus the process
/// temp dir; everything else on the filesystem is read-only; outbound network is
/// denied unless `BIOROUTER_SHELL_SANDBOX_NETWORK` is truthy. Unchanged from
/// BR-64.
fn build_sandbox_policy(working_dir: Option<&std::path::Path>) -> SandboxPolicy {
    let mut roots = Vec::new();
    if let Some(dir) = working_dir {
        roots.push(dir.to_path_buf());
    }
    roots.push(env::temp_dir());
    SandboxPolicy::new(roots).with_network(env_truthy("BIOROUTER_SHELL_SANDBOX_NETWORK"))
}

/// Warn (once per process) that the sandbox was requested but is unavailable, so
/// a fleet admin sees one loud line instead of a silent no-op buried in the log.
fn warn_sandbox_unavailable_once(report: &SandboxReport) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            "BIOROUTER_SHELL_SANDBOX=auto but no full OS sandbox is available on this host \
             ({}); running the shell tool unsandboxed. Set BIOROUTER_SHELL_SANDBOX=strict to \
             refuse instead.",
            report.summary()
        );
    });
}

/// Decide how to wrap the shell `program` under the requested [`SandboxMode`]
/// (BR-69). Returns:
/// - `Ok(None)` — run `program` directly (mode off, or `auto` on a host with no
///   full sandbox: warn once and fall open, exactly BR-64's opt-in behaviour).
/// - `Ok(Some((program, prefix_args)))` — run the wrapped program (Seatbelt on
///   macOS, Landlock/seccomp helper or bubblewrap on Linux).
/// - `Err(msg)` — `strict` mode on a host that cannot reach [`SandboxTier::Full`]:
///   the shell tool surfaces this as a tool error and refuses to run, rather
///   than the false assurance of silently degrading to unsandboxed.
fn shell_sandbox_wrap(
    program: &str,
    working_dir: Option<&std::path::Path>,
) -> Result<Option<(String, Vec<String>)>, String> {
    let mode = SandboxMode::from_env();
    if mode == SandboxMode::Off {
        return Ok(None);
    }

    let backend = shell_sandbox::detect();
    let report = backend.probe();

    if report.tier == SandboxTier::None {
        return match mode {
            SandboxMode::Strict => Err(format!(
                "BIOROUTER_SHELL_SANDBOX=strict but no OS sandbox is available: {}. Refusing to \
                 run. Set BIOROUTER_SHELL_SANDBOX=auto to run unsandboxed, or install/enable a \
                 sandbox.",
                report.summary()
            )),
            _ => {
                warn_sandbox_unavailable_once(&report);
                Ok(None)
            }
        };
    }

    // `strict` requires the top tier (writes confined AND network denied). A
    // partial tier (WriteOnly / ContainmentOnly) does not satisfy it — network
    // egress is the exfiltration channel that matters for prompt injection.
    if mode == SandboxMode::Strict && report.tier != SandboxTier::Full {
        return Err(format!(
            "BIOROUTER_SHELL_SANDBOX=strict requires a full sandbox but this host only offers \
             {}. Refusing to run.",
            report.summary()
        ));
    }

    let policy = build_sandbox_policy(working_dir);
    match backend.wrap(&policy, program) {
        Ok(w) => Ok(Some((w.program, w.prefix_args))),
        Err(e) => match mode {
            SandboxMode::Strict => Err(format!(
                "BIOROUTER_SHELL_SANDBOX=strict but the sandbox wrapper could not be built: {e}. \
                 Refusing to run."
            )),
            _ => {
                warn_sandbox_unavailable_once(&report);
                Ok(None)
            }
        },
    }
}

/// The assistant-visible one-line sandbox status prepended to shell output when
/// the gate is on, e.g. `[sandbox: landlock+seccomp — writes confined + network
/// denied]`. `None` when the gate is off (byte-for-byte no change to output).
/// The model sees it, so it can reason about a denial instead of retrying
/// blindly, and it is what a user screenshots in a bug report.
pub fn shell_sandbox_status_line(working_dir: Option<&std::path::Path>) -> Option<String> {
    let mode = SandboxMode::from_env();
    if mode == SandboxMode::Off {
        return None;
    }
    let report = shell_sandbox::detect().probe();
    if report.tier == SandboxTier::None {
        // `auto` fell open (strict would have errored before reaching output).
        return Some("[sandbox: none, command ran with full user authority]".to_string());
    }

    let policy = build_sandbox_policy(working_dir);
    let roots: Vec<String> = policy
        .writable_roots
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let net = if policy.allow_network {
        "network allowed"
    } else if report.tier == SandboxTier::Full {
        "network denied"
    } else {
        "network NOT denied"
    };
    let mut line = format!(
        "[sandbox: {}, writes confined to {}; {}",
        report.mechanism,
        roots.join(", "),
        net
    );
    if !report.degradations.is_empty() {
        line.push_str("; ");
        line.push_str(&report.degradations.join("; "));
    }
    line.push(']');
    Some(line)
}

/// The standard user-tool directories appended to the shell child's `PATH`
/// when missing (issue #24). A Finder/Dock-launched desktop app inherits
/// launchd's minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), and the shell
/// tool spawns `$SHELL -c` (non-login), so `~/.zprofile`'s `brew shellenv`
/// never runs — Homebrew/MacPorts/user-local tools like `gh` and `brew` were
/// "not found" even though installed.
///
/// This list mirrors `SearchPaths::builder()` in
/// `crates/biorouter/src/config/search_path.rs` (which already augments the
/// PATH of *external* MCP extensions). `biorouter-mcp` cannot import it — the
/// dependency direction is `biorouter` → `biorouter-mcp` — so this is a small
/// local copy; keep the two lists in sync by hand (cross-reference comment on
/// both sides).
fn standard_user_tool_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(unix)]
    {
        dirs.push(std::path::PathBuf::from(
            shellexpand::tilde("~/.local/bin").as_ref(),
        ));
        dirs.push(std::path::PathBuf::from("/usr/local/bin"));
        if cfg!(target_os = "macos") {
            dirs.push(std::path::PathBuf::from("/opt/homebrew/bin"));
            dirs.push(std::path::PathBuf::from("/opt/local/bin"));
        }
    }
    dirs
}

/// Pure core of [`augmented_path`]: append each of `extra` to `base` unless an
/// identical entry is already present. Existing entries keep their order and
/// priority (we append, never prepend, so a user whose PATH is already complete
/// sees byte-for-byte identical resolution), and duplicates within `extra`
/// collapse to one entry.
fn augment_path_with(base: Option<&std::ffi::OsStr>, extra: &[std::path::PathBuf]) -> OsString {
    let existing: Vec<std::path::PathBuf> =
        base.map(env::split_paths).into_iter().flatten().collect();
    let mut combined = existing;
    for dir in extra {
        if !combined.iter().any(|p| p == dir) {
            combined.push(dir.clone());
        }
    }
    env::join_paths(combined.iter())
        .unwrap_or_else(|_| base.map(OsString::from).unwrap_or_default())
}

/// The `PATH` handed to shell-tool children: the daemon's own `PATH` plus the
/// standard user-tool dirs ([`standard_user_tool_dirs`]) appended when absent.
pub(crate) fn augmented_path() -> OsString {
    augment_path_with(env::var_os("PATH").as_deref(), &standard_user_tool_dirs())
}

/// Configure a shell command with process group support for proper child process tracking.
///
/// On Unix systems, creates a new process group so child processes can be killed together.
/// On Windows, the default behavior already supports process tree termination.
///
/// The child's `PATH` is augmented with the standard user-tool directories
/// (Homebrew, MacPorts, `~/.local/bin`, `/usr/local/bin`) when missing — see
/// [`augmented_path`]. This covers both the foreground shell tool and
/// background jobs (`background.rs` also builds through here).
///
/// The child does **not** inherit BioRouter's daemon-private variables — see
/// [`strip_daemon_private_env`] and issue #57. The rest of the environment,
/// including the user's own credentials, is passed through unchanged.
///
/// Returns `Err(message)` only when `BIOROUTER_SHELL_SANDBOX=strict` and the
/// host cannot provide a full sandbox — the caller surfaces that as a tool error
/// (BR-69). In every other mode this is infallible.
pub fn configure_shell_command(
    shell_config: &ShellConfig,
    command: &str,
    working_dir: Option<&std::path::Path>,
) -> Result<tokio::process::Command, String> {
    // BR-69: optionally run the shell under an OS-level sandbox. When enabled,
    // the spawned program becomes `<wrapper> … -- <shell>` and the shell's own
    // `-c <command>` args are appended after the `--` separator below.
    let (program, sandbox_prefix) = match shell_sandbox_wrap(&shell_config.executable, working_dir)?
    {
        Some((prog, prefix)) => (prog, prefix),
        None => (shell_config.executable.clone(), Vec::new()),
    };

    let mut command_builder = tokio::process::Command::new(&program);

    if let Some(dir) = working_dir {
        command_builder.current_dir(dir);
    }

    command_builder
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .env("PATH", augmented_path())
        .env("BIOROUTER_TERMINAL", "1")
        .env("GIT_EDITOR", "sh -c 'echo \"Interactive Git commands are not supported in this environment.\" >&2; exit 1'")
        .env("GIT_SEQUENCE_EDITOR", "sh -c 'echo \"Interactive Git commands are not supported in this environment.\" >&2; exit 1'")
        .env("VISUAL", "sh -c 'echo \"Interactive editor not available in this environment.\" >&2; exit 1'")
        .env("EDITOR", "sh -c 'echo \"Interactive editor not available in this environment.\" >&2; exit 1'")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .args(&sandbox_prefix)
        .args(&shell_config.args)
        .arg(command);

    // Last, so nothing set above can re-admit a daemon credential (issue #57).
    strip_daemon_private_env(&mut command_builder);

    // On Unix systems, create a new process group so we can kill child processes
    #[cfg(unix)]
    {
        command_builder.process_group(0);
    }

    Ok(command_builder)
}

/// SIGKILL a whole process group *now*, with no `await` — the form needed from
/// `Drop`, where there is no async context to run [`kill_process_group`] in.
///
/// `pid` must be the pid of a process spawned through [`configure_shell_command`],
/// which puts it in a new process group of its own (`process_group(0)`), so its
/// pid is also its process-group id. Passing anything else is a bug: the negative
/// pid would name someone else's group.
pub(crate) fn kill_process_group_now(pid: u32) {
    #[cfg(unix)]
    {
        // No SIGTERM/grace pass: the only caller is the drop path, where nobody
        // is left to read the command's output anyway, and a grace period would
        // mean sleeping inside `Drop`.
        let result = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        if result != 0 {
            // ESRCH just means the group is already gone — the common case when
            // the command had already finished.
            tracing::debug!("SIGKILL to process group {pid} returned {result}");
        }
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("taskkill");
        command.args(["/F", "/T", "/PID", &pid.to_string()]);
        strip_daemon_private_env_std(&mut command);
        // Fire and forget: `Drop` must not block.
        let _ = command.spawn();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

/// RAII backstop that kills a foreground shell command's whole process group if
/// the run ends without the command being waited on — i.e. when the tool future
/// is dropped (issue #72).
///
/// `tokio::process::Command::kill_on_drop(true)` is not enough on Unix: it
/// SIGKILLs only the direct child, the `$SHELL -c …` process. Everything that
/// shell forked survives, is reparented to init, and keeps running with no owner
/// — the orphaned `find "$HOME" …` scan in the report. The shell is spawned into
/// a process group of its own precisely so one `killpg` can take the tree; this
/// guard is what performs it on the paths that never reach an explicit kill.
///
/// Declare it **after** the `Child` so it drops first, while the child pid is
/// still un-reaped and therefore still unambiguously names the group.
/// [`disarm`](Self::disarm) it as soon as the child has been waited on: after
/// that the pid may be recycled and the signal would land on a stranger.
pub struct ProcessGroupReaper {
    pid: Option<u32>,
}

impl ProcessGroupReaper {
    /// Arm the reaper for a child spawned by [`configure_shell_command`].
    /// `None` (the platform gave us no pid) makes it inert.
    pub fn arm(pid: Option<u32>) -> Self {
        Self { pid }
    }

    /// Stop the guard from firing. Call this once the child has been waited on
    /// or explicitly killed — the work is over and the pid is no longer ours.
    pub fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for ProcessGroupReaper {
    fn drop(&mut self) {
        if let Some(pid) = self.pid.take() {
            tracing::debug!("reaping abandoned shell process group {pid}");
            kill_process_group_now(pid);
        }
    }
}

/// Ties a spawned task's lifetime to a scope: aborts it on drop.
///
/// The foreground heartbeat loops forever by construction, so its owner must
/// stop it — and `execute_shell_command` can leave by `?` from inside a
/// `select!` arm as well as by returning a value. An explicit `abort()` after
/// the `select!` is skipped by the `?` path, and a heartbeat that outlives its
/// command notifies the client about a command that finished long ago. Making it
/// RAII removes the class of mistake instead of the one instance.
pub struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl AbortOnDrop {
    pub fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self(handle)
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Registers a running **foreground** shell command in the process-wide
/// active-work registry for as long as it runs, with a cancel action that kills
/// its process group (issue #72).
///
/// Background jobs and subagents have been listed there since BR-42; foreground
/// commands were the one kind of long-running work with no entry, which is why
/// an unexpectedly expensive one was invisible while it blocked the turn and
/// there was no way to stop just it. `GET /active_work` now shows it with its
/// elapsed time; `POST /active_work/{id}/cancel` kills its group.
///
/// RAII, so an early return, an error or a panic can never leave a phantom
/// "still running" entry behind.
///
/// `session_id` is the chat that ran the command. The entry carries it because
/// the listing shows a row only to a caller that could open its chat, and
/// answers a row that names no chat as a private chat's (issue #56).
pub struct ForegroundWorkGuard {
    _guard: crate::active_work::ActiveWorkGuard,
}

impl ForegroundWorkGuard {
    pub fn register(command: &str, pid: Option<u32>, session_id: Option<String>) -> Self {
        let cancel: Option<std::sync::Arc<dyn Fn() + Send + Sync>> = pid.map(|pid| {
            std::sync::Arc::new(move || kill_process_group_now(pid))
                as std::sync::Arc<dyn Fn() + Send + Sync>
        });
        Self {
            _guard: crate::active_work::ActiveWorkGuard::register(
                crate::active_work::ActiveWorkKind::ForegroundCommand,
                first_line(command),
                Some(command.to_string()),
                session_id,
                cancel,
            ),
        }
    }
}

/// First line of a command, truncated, for the one-line titles the active-work
/// view and the heartbeat use. A heredoc or a multi-line script must not turn
/// into a wall of text in a list.
pub fn first_line(command: &str) -> String {
    const MAX: usize = 120;
    let line = command.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(MAX).collect();
    if line.chars().count() > MAX || command.lines().nth(1).is_some() {
        out.push('…');
    }
    out
}

/// The error a foreground command gets when it blows its budget (issue #72).
///
/// Deliberately actionable rather than a bare "timed out": it names the command,
/// how long it actually ran, the budget it broke, the knob that changes the
/// budget, and the workflow this server already provides for work of that size.
/// The opaque generic-extension timeout it replaces told the model none of that.
pub fn foreground_timeout_message(
    command: &str,
    elapsed: std::time::Duration,
    limit: std::time::Duration,
) -> String {
    format!(
        "The foreground shell command was killed after {} because it exceeded the {}s \
         foreground limit: {}\n\
         Its whole process group was terminated, so nothing is left running.\n\
         If this command legitimately takes that long, re-run it with background=true and \
         watch it with shell_status / shell_kill. Otherwise narrow it (a \
         smaller search root, a more specific pattern) and try again. Operators can change \
         the limit with {} (seconds; 0 disables it).",
        humanize_secs(elapsed),
        limit.as_secs(),
        first_line(command),
        FOREGROUND_TIMEOUT_ENV,
    )
}

/// Kill a process and all its child processes using platform-specific approaches.
///
/// On Unix systems, kills the entire process group.
/// On Windows, kills the process tree.
pub async fn kill_process_group(
    child: &mut tokio::process::Child,
    pid: Option<u32>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(unix)]
    {
        if let Some(pid) = pid {
            // Try SIGTERM first
            let sigterm_result = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            if sigterm_result != 0 {
                tracing::warn!(
                    "SIGTERM to process group {} failed with errno {}",
                    pid,
                    sigterm_result
                );
            }

            // Wait a brief moment for graceful shutdown
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // Force kill with SIGKILL
            let sigkill_result = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            if sigkill_result != 0 {
                tracing::warn!(
                    "SIGKILL to process group {} failed with errno {}",
                    pid,
                    sigkill_result
                );
            }
        }

        // Last fallback, return the result of tokio's kill
        child.kill().await.map_err(|e| e.into())
    }

    #[cfg(windows)]
    {
        if let Some(pid) = pid {
            // Use taskkill to kill the process tree on Windows
            let mut command = tokio::process::Command::new("taskkill");
            command.args(["/F", "/T", "/PID", &pid.to_string()]);
            strip_daemon_private_env(&mut command);
            let _kill_result = command.output().await;
        }

        // Return the result of tokio's kill
        child.kill().await.map_err(|e| e.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_absolute_paths_accept_native_and_forward_slash_forms() {
        for absolute in [
            r"C:\workspace\out.txt",
            "C:/workspace/out.txt",
            r"\workspace\out.txt",
            r"\\server\share\out.txt",
        ] {
            assert!(is_absolute_path(absolute), "expected absolute: {absolute}");
        }
        for relative in [r"workspace\out.txt", "workspace/out.txt", r"C:out.txt"] {
            assert!(!is_absolute_path(relative), "expected relative: {relative}");
        }
    }

    #[test]
    fn network_flag_parses_truthy_values() {
        for on in ["1", "true", "on", "yes", "  ON ", "True", "Yes"] {
            assert!(env_flag_truthy(on), "expected {on:?} to enable");
        }
        for off in ["", "0", "off", "no", "false", "docker", " ", "seatbelt"] {
            assert!(!env_flag_truthy(off), "expected {off:?} to stay off");
        }
    }

    #[test]
    fn unset_gate_runs_the_plain_shell() {
        // Default (gate unset) must be byte-for-byte the old behavior: the
        // spawned program is the shell itself, not a sandbox wrapper, and no
        // status line is prepended.
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return; // don't fight an externally-set gate in this process
        }
        let cfg = ShellConfig::default();
        let cmd = configure_shell_command(&cfg, "echo hi", None).expect("infallible when off");
        assert_eq!(cmd.as_std().get_program().to_string_lossy(), cfg.executable);
        assert!(
            shell_sandbox_status_line(None).is_none(),
            "no status line when the gate is off"
        );
    }

    #[test]
    fn shell_sandbox_wrap_is_none_when_gate_off() {
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return;
        }
        assert!(matches!(shell_sandbox_wrap("/bin/sh", None), Ok(None)));
    }

    // ---- issue #24: shell PATH augmentation ----------------------------------
    // Pure-logic tests pass the base PATH as a parameter (no env mutation — the
    // workspace has a known env-var test race).

    #[cfg(unix)]
    #[test]
    fn augment_path_appends_missing_dirs_in_order() {
        use std::ffi::OsStr;
        use std::path::PathBuf;
        // launchd's minimal PATH, the exact shape from issue #24.
        let base = "/usr/bin:/bin:/usr/sbin:/sbin";
        let extra = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        let got = augment_path_with(Some(OsStr::new(base)), &extra);
        assert_eq!(
            got.to_string_lossy(),
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin"
        );
    }

    #[cfg(unix)]
    #[test]
    fn augment_path_no_dupes_and_order_preserved() {
        use std::ffi::OsStr;
        use std::path::PathBuf;
        // A PATH that already contains the standard dirs must come back
        // byte-for-byte unchanged: no duplicates, original priority intact.
        let base = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin";
        let extra = vec![
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
        ];
        let got = augment_path_with(Some(OsStr::new(base)), &extra);
        assert_eq!(got.to_string_lossy(), base);
    }

    #[cfg(unix)]
    #[test]
    fn augment_path_handles_unset_base_and_dupes_in_extra() {
        use std::path::PathBuf;
        let extra = vec![
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/local/bin"),
        ];
        let got = augment_path_with(None, &extra);
        assert_eq!(got.to_string_lossy(), "/usr/local/bin:/opt/local/bin");
    }

    #[cfg(unix)]
    #[test]
    fn standard_dirs_mirror_search_paths_list() {
        // Guard the hand-maintained mirror of SearchPaths::builder()
        // (crates/biorouter/src/config/search_path.rs).
        let dirs = standard_user_tool_dirs();
        let strs: Vec<String> = dirs
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(strs.iter().any(|p| p.ends_with("/.local/bin")));
        assert!(strs.contains(&"/usr/local/bin".to_string()));
        if cfg!(target_os = "macos") {
            assert!(strs.contains(&"/opt/homebrew/bin".to_string()));
            assert!(strs.contains(&"/opt/local/bin".to_string()));
        }
    }

    // ---- issue #57: the daemon's auth secret must not reach a tool child ------

    /// The policy, over one environment: exactly the daemon's own key goes.
    /// The listing mirrors what `biorouterd.ts` actually hands the daemon, plus
    /// the user variables and the extension credential that must survive.
    #[test]
    fn policy_strips_the_daemon_secret_and_nothing_else() {
        let daemon_env = [
            // from ui/desktop/src/biorouterd.ts
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "PATH",
            "BIOROUTER_PORT",
            "BIOROUTER_SERVER__SECRET_KEY",
            "BIOROUTER_DISABLE_KEYRING",
            "BIOROUTER_AUTOVIS_CDN",
            // ordinary user/OS environment
            "LANG",
            "LC_ALL",
            "TMPDIR",
            "SHELL",
            "SSH_AUTH_SOCK",
            "HTTPS_PROXY",
            "no_proxy",
            "CONDA_PREFIX",
            "VIRTUAL_ENV",
            "JAVA_HOME",
            // the user's own credentials: not ours to censor — an agent with a
            // shell can read ~/.aws/credentials anyway, and stripping these
            // breaks `aws`, `gh` and `huggingface-cli`
            "AWS_SECRET_ACCESS_KEY",
            "GH_TOKEN",
            "HF_TOKEN",
            // credentials extensions declare via env_keys
            "SPOKEAGENT_PASSCODE",
            "CLINICAL_RECORDS_TOKEN",
        ];
        let stripped: Vec<&str> = daemon_env
            .iter()
            .copied()
            .filter(|k| is_daemon_private_env_key(k))
            .collect();
        assert_eq!(stripped, vec!["BIOROUTER_SERVER__SECRET_KEY"]);
    }

    /// The property the namespace-deny shape buys: a credential added to the
    /// daemon's own server namespace later is denied without editing this file.
    /// Only the second group needs the credential-name net.
    #[test]
    fn policy_denies_future_daemon_credentials() {
        for key in [
            "BIOROUTER_SERVER__SESSION_SIGNING_KEY",
            "BIOROUTER_SERVER__ADMIN_PASSWORD",
            // named nothing like a secret, and still denied — this is the case
            // a deny-list of literal names would miss
            "BIOROUTER_SERVER__PAIRING",
            "GOOSE_SERVER__SECRET_KEY",
        ] {
            assert!(
                is_daemon_private_env_key(key),
                "{key} is daemon-private configuration and must not reach a child"
            );
        }
        // BioRouter credentials that live outside BIOROUTER_SERVER__.
        for key in [
            "BIOROUTER_ACP_WS_TOKEN",
            "BIOROUTER_WS_TOKEN",
            "BIOROUTER_EDITOR_API_KEY",
        ] {
            assert!(is_daemon_private_env_key(key), "{key} carries a credential");
        }
    }

    /// Non-credential BioRouter knobs a child legitimately reads keep flowing —
    /// including `BIOROUTER_CLIENT_KEY_PATH`, which names a file rather than
    /// holding a secret.
    #[test]
    fn policy_keeps_non_credential_biorouter_knobs() {
        for key in [
            "BIOROUTER_PORT",
            "BIOROUTER_WORKING_DIR",
            "BIOROUTER_DISABLE_KEYRING",
            "BIOROUTER_TERMINAL",
            "BIOROUTER_CLIENT_KEY_PATH",
            "BIOROUTER_SHELL_SANDBOX",
        ] {
            assert!(
                !is_daemon_private_env_key(key),
                "{key} is not a credential and a child may need it"
            );
        }
    }

    /// Windows environment names are case-insensitive, so the policy is too.
    #[test]
    fn policy_is_case_insensitive() {
        assert!(is_daemon_private_env_key("biorouter_server__secret_key"));
        assert!(is_daemon_private_env_key("Biorouter_Acp_Ws_Token"));
    }

    /// `strip_daemon_private_env` runs last precisely so an explicitly set
    /// value — an extension's declared `envs`, which a `.brxt` bundle can
    /// carry — cannot re-admit the daemon's key.
    #[test]
    fn explicitly_set_daemon_secret_is_removed_again() {
        let mut cmd = tokio::process::Command::new("printenv");
        cmd.env("BIOROUTER_SERVER__SECRET_KEY", "declared-by-an-extension");
        cmd.env("SPOKEAGENT_PASSCODE", "keep-me");
        strip_daemon_private_env(&mut cmd);

        let envs: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(
            envs.contains(&("BIOROUTER_SERVER__SECRET_KEY".to_string(), None)),
            "expected an explicit removal, got {envs:?}"
        );
        assert!(envs.contains(&(
            "SPOKEAGENT_PASSCODE".to_string(),
            Some("keep-me".to_string())
        )));
    }

    /// Child half of [`daemon_secret_never_reaches_a_shell_child`].
    ///
    /// The leak is in the *inherited* environment, so exercising it means
    /// controlling this process's environment — and `set_var` is unsound in a
    /// threaded test binary (this workspace has a known env-var test race). So
    /// the parent re-invokes this very test binary with the canary exported,
    /// and this half spawns a real shell child through the real
    /// `configure_shell_command` and prints the environment that child actually
    /// received. `#[ignore]` keeps it out of a normal run.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn leak_probe_prints_shell_child_env() {
        let cfg = ShellConfig::default();
        let mut cmd = configure_shell_command(&cfg, "printenv", None).expect("infallible when off");
        let out = cmd.output().await.expect("shell child must spawn");
        println!("BEGIN_CHILD_ENV");
        println!("{}", probe_report(&String::from_utf8_lossy(&out.stdout)));
        println!("END_CHILD_ENV");
    }

    /// What a probe reports back: BioRouter's own namespace, the variables the
    /// parent injected, and — under *any* name — anything whose value carries
    /// the canary, so a copy of the secret under a different key is still
    /// caught. Everything else is dropped: printing the whole environment of
    /// whoever runs the suite would be its own small leak.
    #[cfg(unix)]
    fn probe_report(raw: &str) -> String {
        let canary = env::var("BR_TEST_CANARY").unwrap_or_default();
        raw.lines()
            .filter(|line| {
                let key = line.split('=').next().unwrap_or("");
                // The channel that carries the canary *to* the probe is itself
                // inherited; it is not the leak under test.
                if key == "BR_TEST_CANARY" {
                    return false;
                }
                key.starts_with("BIOROUTER_")
                    || key.starts_with("GOOSE_")
                    || matches!(
                        key,
                        "PATH" | "HOME" | "BR_TEST_USER_VAR" | "SPOKEAGENT_PASSCODE"
                    )
                    || (!canary.is_empty() && line.contains(&canary))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The name libtest knows the probe by (`module_path!()` minus the crate).
    #[cfg(unix)]
    fn probe_test_name(probe: &str) -> String {
        let m = module_path!();
        let without_crate = m.split_once("::").map(|(_, rest)| rest).unwrap_or(m);
        format!("{without_crate}::{probe}")
    }

    /// Run the ignored probe in a fresh copy of this test binary whose
    /// environment is the daemon's: the auth secret, the port, an
    /// extension-declared credential and an ordinary user variable.
    #[cfg(unix)]
    fn run_leak_probe(probe: &str, canary: &str) -> String {
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                &probe_test_name(probe),
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BIOROUTER_SERVER__SECRET_KEY", canary)
            .env("BR_TEST_CANARY", canary)
            .env("BIOROUTER_PORT", "54321")
            .env("SPOKEAGENT_PASSCODE", "extension-credential-ok")
            .env("BR_TEST_USER_VAR", "user-env-ok")
            .output()
            .expect("re-invoking the test binary must work");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let body = stdout
            .split_once("BEGIN_CHILD_ENV\n")
            .and_then(|(_, rest)| rest.split_once("END_CHILD_ENV"))
            .map(|(body, _)| body.to_string());
        body.unwrap_or_else(|| {
            panic!(
                "probe produced no child environment.\nstdout:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        })
    }

    #[cfg(unix)]
    #[test]
    fn daemon_secret_never_reaches_a_shell_child() {
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return; // don't fight an externally-set gate in this process
        }
        const CANARY: &str = "canary-daemon-secret-9f3a";
        let child_env = run_leak_probe("leak_probe_prints_shell_child_env", CANARY);

        assert!(
            !child_env.contains(CANARY),
            "issue #57: the daemon's auth secret reached the shell child, so any \
             session can call biorouterd as an authenticated client.\nchild env:\n{child_env}"
        );
        assert!(
            !child_env.contains("BIOROUTER_SERVER__SECRET_KEY"),
            "the key name itself must be gone, not just the value:\n{child_env}"
        );
    }

    /// The other direction: stripping the daemon's credential must not take the
    /// user's environment (or a legitimately exported extension credential)
    /// with it. A truncated child environment is its own regression (issue #24
    /// was exactly that, for `PATH`).
    #[cfg(unix)]
    #[test]
    fn shell_child_still_receives_the_user_environment() {
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return;
        }
        let child_env = run_leak_probe("leak_probe_prints_shell_child_env", "canary-unused");
        for expected in [
            "PATH=",
            "HOME=",
            "BIOROUTER_PORT=54321",
            "SPOKEAGENT_PASSCODE=extension-credential-ok",
            "BR_TEST_USER_VAR=user-env-ok",
        ] {
            assert!(
                child_env.lines().any(|l| l.starts_with(expected)),
                "shell child lost {expected:?}; removing too much is its own \
                 regression.\nchild env:\n{child_env}"
            );
        }
    }

    // ---- issue #72: the foreground budget ----------------------------------
    // Pure-logic tests pass the raw value as a parameter (no env mutation — the
    // workspace has a known env-var test race).

    #[test]
    fn foreground_timeout_defaults_and_overrides() {
        // Unset (and blank) keeps the default rather than running unbounded.
        assert_eq!(
            parse_foreground_timeout(None),
            Some(FOREGROUND_TIMEOUT_DEFAULT)
        );
        assert_eq!(
            parse_foreground_timeout(Some("   ")),
            Some(FOREGROUND_TIMEOUT_DEFAULT)
        );
        // An explicit budget is honoured, whitespace and all.
        assert_eq!(
            parse_foreground_timeout(Some(" 45 ")),
            Some(std::time::Duration::from_secs(45))
        );
        // 0 is the documented opt-out.
        assert_eq!(parse_foreground_timeout(Some("0")), None);
        // Garbage must not silently disable the watchdog — that would turn a
        // typo into an unbounded foreground command.
        assert_eq!(
            parse_foreground_timeout(Some("five minutes")),
            Some(FOREGROUND_TIMEOUT_DEFAULT)
        );
        assert_eq!(
            parse_foreground_timeout(Some("-1")),
            Some(FOREGROUND_TIMEOUT_DEFAULT)
        );
    }

    /// The default has to fire *inside* the generic 300s per-extension MCP
    /// timeout, or the opaque transport error wins the race and the model never
    /// sees the actionable message.
    #[test]
    fn foreground_default_fires_before_the_generic_extension_timeout() {
        assert!(
            FOREGROUND_TIMEOUT_DEFAULT < std::time::Duration::from_secs(300),
            "the foreground budget must be strictly inside the 300s extension timeout"
        );
    }

    #[test]
    fn durations_read_as_seconds_below_a_minute_and_minutes_above() {
        assert_eq!(humanize_secs(std::time::Duration::from_secs(0)), "0s");
        assert_eq!(humanize_secs(std::time::Duration::from_secs(59)), "59s");
        assert_eq!(humanize_secs(std::time::Duration::from_secs(60)), "1m0s");
        assert_eq!(humanize_secs(std::time::Duration::from_secs(192)), "3m12s");
    }

    #[test]
    fn first_line_summarises_without_leaking_a_wall_of_text() {
        assert_eq!(first_line("ls -la"), "ls -la");
        // A multi-line script is marked as truncated even when line 1 is short.
        assert_eq!(first_line("cat <<'EOF'\nbody\nEOF"), "cat <<'EOF'…");
        // And a single very long line is cut.
        let long = "x".repeat(500);
        let summarised = first_line(&long);
        assert!(summarised.chars().count() <= 121, "{summarised}");
        assert!(summarised.ends_with('…'));
    }

    /// The message is the whole point of owning the timeout: a bare "timed out"
    /// tells the model nothing it can act on.
    #[test]
    fn the_timeout_message_names_the_cause_and_the_remedy() {
        let message = foreground_timeout_message(
            "find \"$HOME\" -type d -name 'x' 2>/dev/null",
            std::time::Duration::from_secs(245),
            std::time::Duration::from_secs(240),
        );
        assert!(
            message.contains("4m5s"),
            "how long it actually ran: {message}"
        );
        assert!(message.contains("240s"), "the budget it broke: {message}");
        assert!(message.contains("find "), "the command: {message}");
        assert!(
            message.contains("background=true") && message.contains("shell_status"),
            "the workflow for work of that size: {message}"
        );
        assert!(
            message.contains(FOREGROUND_TIMEOUT_ENV),
            "the knob that changes the budget: {message}"
        );
        assert!(
            message.contains("process group"),
            "and the reassurance that nothing was left running: {message}"
        );
    }

    /// The heartbeat loops forever, so leaving the scope must stop it. An
    /// explicit `abort()` at the end of `execute_shell_command` was skipped by
    /// the `?` inside one of its `select!` arms, which would have left a task
    /// notifying the client about a command that finished long ago.
    #[tokio::test]
    async fn abort_on_drop_stops_an_endless_task() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let ticks = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = ticks.clone();
        let guard = AbortOnDrop::new(tokio::spawn(async move {
            loop {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }));

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert!(ticks.load(Ordering::SeqCst) > 0, "the task must have run");
        drop(guard);

        let after_drop = ticks.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(
            ticks.load(Ordering::SeqCst),
            after_drop,
            "the task must stop the moment its guard drops"
        );
    }

    // ---- issue #72: the process-group reaper -------------------------------

    /// True while `pid` still exists (signal 0 is the standard liveness probe).
    #[cfg(unix)]
    fn pid_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    /// The guard must actually take the group down when it fires. Without it,
    /// `kill_on_drop` alone leaves everything the shell forked running.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_armed_reaper_kills_the_group_on_drop() {
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return; // don't fight an externally-set gate in this process
        }
        let cfg = ShellConfig::default();
        let mut cmd = configure_shell_command(&cfg, "sleep 30", None).expect("infallible when off");
        let mut child = cmd.spawn().expect("shell child must spawn");
        let pid = child.id().expect("a spawned child has a pid");

        drop(ProcessGroupReaper::arm(Some(pid)));

        // Without the kill this waits the full 30 seconds.
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .expect("the reaper must have killed the group")
            .expect("wait must succeed");
    }

    /// …and it must be completely inert once disarmed. The normal-completion
    /// path disarms right after `wait()`, because a reaped pid can be recycled
    /// and the negative-pid signal would then land on a stranger's process group.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_disarmed_reaper_signals_nothing() {
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return;
        }
        let cfg = ShellConfig::default();
        let mut cmd = configure_shell_command(&cfg, "sleep 30", None).expect("infallible when off");
        let mut child = cmd.spawn().expect("shell child must spawn");
        let pid = child.id().expect("a spawned child has a pid");

        {
            let mut reaper = ProcessGroupReaper::arm(Some(pid));
            reaper.disarm();
        }

        assert!(
            pid_alive(pid),
            "a disarmed reaper must not signal anything on drop"
        );
        let _ = child.kill().await;
    }

    #[test]
    fn configure_shell_command_sets_augmented_path() {
        if env::var("BIOROUTER_SHELL_SANDBOX").is_ok() {
            return; // don't fight an externally-set gate in this process
        }
        let cfg = ShellConfig::default();
        let cmd = configure_shell_command(&cfg, "echo hi", None).expect("infallible when off");
        let path_env = cmd
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == OsString::from("PATH").as_os_str())
            .and_then(|(_, v)| v)
            .expect("shell command must carry an explicit PATH env");
        assert_eq!(path_env, augmented_path().as_os_str());
        #[cfg(unix)]
        {
            let path_str = path_env.to_string_lossy();
            assert!(
                path_str.contains("/usr/local/bin"),
                "augmented PATH must contain /usr/local/bin, got {path_str}"
            );
            if cfg!(target_os = "macos") {
                assert!(
                    path_str.contains("/opt/homebrew/bin"),
                    "augmented PATH must contain /opt/homebrew/bin, got {path_str}"
                );
            }
        }
    }
}
