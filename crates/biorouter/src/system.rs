//! Shared system-prerequisite checks and CLI installation — the single source
//! of truth used by both the terminal (`biorouter doctor` / `setup-path`) and
//! the desktop app's dependency setup. Define a prerequisite once in [`specs`]
//! and every front-end picks it up.
//!
//! How the desktop reads it: there is **no** server route for this. The Electron
//! main process shells out to the bundled CLI — `biorouter doctor --format json
//! --no-update` — and maps the snake_case JSON onto its own camelCase type
//! (`ui/desktop/src/utils/dependencyChecker.ts`). It keeps a native probe
//! fallback for dev builds where the bundled CLI is absent, which is the one
//! place the catalog is duplicated.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// One external tool Biorouter can use, plus how to detect and install it.
struct Spec {
    name: &'static str,
    display_name: &'static str,
    /// Probe candidates tried in order until one reports a version.
    probes: &'static [(&'static str, &'static [&'static str])],
    /// Required tools block core flows; recommended ones unlock optional ones.
    required: bool,
    doc_url: &'static str,
    purpose: &'static str,
}

/// The catalog of prerequisites. **Edit here** to change what every front-end
/// checks for.
fn specs() -> &'static [Spec] {
    &[
        Spec {
            name: "git",
            display_name: "Git",
            probes: &[("git", &["--version"])],
            required: true,
            doc_url: "https://git-scm.com/downloads",
            purpose: "Clone and version repositories and knowledge bases",
        },
        Spec {
            name: "uv",
            display_name: "uv (Python toolchain)",
            probes: &[("uv", &["--version"])],
            required: true,
            doc_url: "https://docs.astral.sh/uv/",
            purpose: "Run Python MCP extensions and install .brxt bundles",
        },
        Spec {
            name: "python",
            display_name: "Python 3",
            probes: &[("python3", &["--version"]), ("python", &["--version"])],
            required: false,
            doc_url: "https://www.python.org/downloads/",
            purpose: "Back Python-based extensions (uv can manage this for you)",
        },
        Spec {
            name: "node",
            display_name: "Node.js (npx)",
            probes: &[("node", &["--version"])],
            required: false,
            doc_url: "https://nodejs.org/",
            purpose: "Run npx-based MCP servers such as Playwright",
        },
        Spec {
            name: "aws",
            display_name: "AWS CLI",
            probes: &[("aws", &["--version"])],
            required: false,
            doc_url: "https://aws.amazon.com/cli/",
            purpose: "Optional: UCSF Bedrock / Amazon Bedrock credentials",
        },
        Spec {
            name: "llama-server",
            display_name: "llama-server (local models)",
            probes: &[("llama-server", &["--version"])],
            required: false,
            doc_url: "https://github.com/ggml-org/llama.cpp/releases",
            purpose: "Run built-in local models (bundled with the desktop app)",
        },
        // `rustc -vV` is a *health* probe, not just a presence probe: a broken
        // Homebrew rust (whose `rustc` aborts after an `llvm` upgrade) exits
        // non-zero here and is correctly reported as unavailable, steering the
        // user to rustup. Some Python extension deps (e.g. cryptography ≥49,
        // which dropped Intel-Mac wheels) are compiled from source and need it.
        Spec {
            name: "rust",
            display_name: "Rust toolchain (rustc)",
            probes: &[("rustc", &["-vV"])],
            required: false,
            doc_url: "https://rustup.rs",
            purpose: "Compile source-only Python extension deps (e.g. cryptography on Intel Macs)",
        },
    ]
}

/// The detected state of a single prerequisite. This is the shape both
/// `biorouter doctor --json` and the desktop dependency setup consume.
#[derive(Clone, Debug, Serialize)]
pub struct DependencyStatus {
    pub name: String,
    pub display_name: String,
    pub installed: bool,
    /// The probe was still running at the timeout and was killed, so this tool's
    /// presence is UNKNOWN rather than disproved. `installed` is false either
    /// way; this separates "could not tell" from "not there" for any surface
    /// that would otherwise tell the user to install what they already have.
    #[serde(default)]
    pub timed_out: bool,
    pub version: Option<String>,
    pub required: bool,
    pub purpose: String,
    pub doc_url: String,
    /// A shell command that installs it on the current OS, when one is known.
    pub install_command: Option<String>,
    /// True when `install_command` needs elevated privileges (Linux apt/dnf).
    pub requires_sudo: bool,
    /// A download/instructions page when there is no one-line install.
    pub download_url: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum LinuxDistro {
    Deb,
    Rpm,
    Unknown,
}

fn linux_distro() -> LinuxDistro {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        let c = content.to_lowercase();
        if c.contains("ubuntu") || c.contains("debian") {
            return LinuxDistro::Deb;
        }
        if ["fedora", "rhel", "centos", "rocky", "alma"]
            .iter()
            .any(|d| c.contains(d))
        {
            return LinuxDistro::Rpm;
        }
    }
    if Path::new("/etc/debian_version").exists() {
        return LinuxDistro::Deb;
    }
    if Path::new("/etc/redhat-release").exists() {
        return LinuxDistro::Rpm;
    }
    LinuxDistro::Unknown
}

/// How to install a prerequisite on the current OS.
struct InstallInfo {
    command: Option<String>,
    requires_sudo: bool,
    download_url: Option<String>,
}

fn s(x: &str) -> Option<String> {
    Some(x.to_string())
}

fn install_info(name: &str) -> InstallInfo {
    let none = InstallInfo {
        command: None,
        requires_sudo: false,
        download_url: None,
    };
    if cfg!(target_os = "macos") {
        let (cmd, url) = match name {
            "git" => ("xcode-select --install", "https://git-scm.com/download/mac"),
            "python" => ("brew install python3", "https://www.python.org/downloads/"),
            "uv" => (
                "curl -LsSf https://astral.sh/uv/install.sh | sh",
                "https://docs.astral.sh/uv/",
            ),
            "node" => ("brew install node", "https://nodejs.org/en/download"),
            "aws" => ("brew install awscli", "https://aws.amazon.com/cli/"),
            "llama-server" => (
                "brew install llama.cpp",
                "https://github.com/ggml-org/llama.cpp/releases",
            ),
            // rustup, NOT `brew install rust` — Homebrew's rust links libLLVM
            // dynamically and breaks on llvm upgrades; rustup is self-contained.
            // `sh -s -- -y`: the installer is piped (no controlling TTY), so
            // rustup-init aborts with "Unable to run interactively. Run with -y
            // to accept defaults" unless we pass `-y` through to it.
            "rust" => (
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                "https://rustup.rs",
            ),
            _ => return none,
        };
        InstallInfo {
            command: s(cmd),
            requires_sudo: false,
            download_url: s(url),
        }
    } else if cfg!(target_os = "windows") {
        // Run unattended: without these flags winget blocks on the source/package
        // agreement prompt the first time it's used, which fails in the app's
        // non-interactive shell (the same class of failure as rustup needing -y).
        const WG: &str =
            "--accept-package-agreements --accept-source-agreements --disable-interactivity";
        let (cmd, url): (String, &str) = match name {
            "git" => (format!("winget install --id Git.Git -e --source winget {WG}"), "https://git-scm.com/download/win"),
            "python" => (format!("winget install --id Python.Python.3 -e --source winget {WG}"), "https://www.python.org/downloads/"),
            "uv" => (
                "powershell -ExecutionPolicy ByPass -c \"irm https://astral.sh/uv/install.ps1 | iex\"".to_string(),
                "https://docs.astral.sh/uv/",
            ),
            "node" => (format!("winget install --id OpenJS.NodeJS -e --source winget {WG}"), "https://nodejs.org/en/download"),
            "aws" => (format!("winget install Amazon.AWSCLI {WG}"), "https://aws.amazon.com/cli/"),
            "llama-server" => (
                format!("winget install --id ggml.llamacpp -e --source winget {WG}"),
                "https://github.com/ggml-org/llama.cpp/releases",
            ),
            "rust" => (format!("winget install --id Rustlang.Rustup -e --source winget {WG}"), "https://rustup.rs"),
            _ => return none,
        };
        InstallInfo {
            command: Some(cmd),
            requires_sudo: false,
            download_url: s(url),
        }
    } else {
        // Linux
        if name == "uv" {
            return InstallInfo {
                command: s("curl -LsSf https://astral.sh/uv/install.sh | sh"),
                requires_sudo: false,
                download_url: s("https://docs.astral.sh/uv/"),
            };
        }
        if name == "llama-server" {
            // No standard distro package; prebuilt binaries on GitHub releases.
            return InstallInfo {
                command: None,
                requires_sudo: false,
                download_url: s("https://github.com/ggml-org/llama.cpp/releases"),
            };
        }
        if name == "rust" {
            // rustup over the distro `rustc`, which is often too old for modern
            // Rust-backed wheels. `sh -s -- -y` runs it non-interactively (the
            // installer is piped, so it has no TTY and would otherwise abort).
            return InstallInfo {
                command: s(
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                ),
                requires_sudo: false,
                download_url: s("https://rustup.rs"),
            };
        }
        let pkg = match name {
            "git" => "git",
            "python" => "python3",
            "node" => "nodejs npm",
            "aws" => "awscli",
            _ => return none,
        };
        let url = match name {
            "aws" => Some("https://aws.amazon.com/cli/".to_string()),
            _ => None,
        };
        match linux_distro() {
            LinuxDistro::Deb => InstallInfo {
                command: s(&format!("sudo apt-get install -y {pkg}")),
                requires_sudo: true,
                download_url: url,
            },
            LinuxDistro::Rpm => InstallInfo {
                command: s(&format!("sudo dnf install -y {pkg}")),
                requires_sudo: true,
                download_url: url,
            },
            LinuxDistro::Unknown => InstallInfo {
                command: None,
                requires_sudo: false,
                download_url: url,
            },
        }
    }
}

/// How long ONE PREREQUISITE may take to answer, across every probe it tries.
///
/// Per spec, not per probe: `python` tries `python3` then `python`, and
/// `llama-server` tries PATH then the bundled sidecar, so a per-probe budget
/// would make the real worst case twice this number.
///
/// 12 s admits the measured cold cost of a freshly installed binary — first
/// execution is an operating-system scan, not compute: `llama-server --version`
/// measured 8.33 s real at 0.04 s CPU on a fast Mac against 0.05 s warm — while
/// leaving `biorouter doctor` comfortably inside the two budgets that bound it:
/// the desktop's startup call (`ui/desktop/src/utils/dependencyChecker.ts`) and
/// the installed-package check's. Specs run concurrently, so this is also
/// `check_all`'s whole worst case, not a per-spec cost that sums.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

/// What one probe established about a command.
enum ProbeOutcome {
    /// The command ran and reported this version line (possibly empty).
    Version(String),
    /// The command is absent, or exited non-zero.
    Absent,
    /// The command had not answered by its deadline and was abandoned.
    ///
    /// Distinct from `Absent` on purpose: "we could not tell" and "it is not
    /// there" are different answers, and collapsing them is how a slow machine
    /// gets told to install software it already has.
    TimedOut,
}

/// Probe one command; returns its first version line on success.
///
/// Bounded by [`PROBE_TIMEOUT`]. Both pipes are drained by their own threads
/// while we wait, because reading them in sequence deadlocks as soon as a child
/// fills the one we are not reading.
/// Test seam: a probe expressed as a duration rather than an instant. Production
/// threads one deadline per prerequisite through [`probe_until`] instead.
#[cfg(test)]
fn probe_within(cmd: &str, args: &[&str], budget: std::time::Duration) -> ProbeOutcome {
    probe_until(cmd, args, std::time::Instant::now() + budget)
}

/// Probe one command, abandoning it at `deadline`.
///
/// The deadline covers the WHOLE probe, including reading the child's output.
/// Bounding only the child's exit is not enough: a child can answer and exit
/// while a descendant it spawned still holds the inherited pipe, and a blocking
/// read then waits for that descendant instead. Measured at 30 s against a 2 s
/// budget before this was fixed -- the same mechanism that makes a hung pipe
/// indistinguishable from slow work on the Python side of the harness.
fn probe_until(cmd: &str, args: &[&str], deadline: std::time::Instant) -> ProbeOutcome {
    use std::io::Read;
    use std::process::Stdio;

    let mut command = Command::new(cmd);
    command
        .args(args)
        // Never inherit the caller's stdin: a probe that waits on a terminal
        // that will never answer is indistinguishable from a slow one.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own process group, so abandoning a probe can take its descendants
    // with it. Without this a grandchild survives holding the inherited pipe.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    biorouter_mcp::developer::shell::strip_daemon_private_env_std(&mut command);
    let Ok(mut child) = command.spawn() else {
        return ProbeOutcome::Absent;
    };
    // Each pipe is drained by its own thread -- reading them in sequence
    // deadlocks as soon as the child fills the one we are not reading -- and
    // each reports through a channel so the wait for it can be bounded. A
    // thread still blocked on a pipe a descendant holds is detached and
    // harmless; its send simply never arrives.
    let drain = |handle: Option<_>| {
        let (sender, receiver) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = handle {
                let _: Option<usize> = std::io::Read::read_to_end(&mut stream, &mut bytes).ok();
            }
            let _: Result<(), _> = sender.send(bytes);
        });
        receiver
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(_) => return ProbeOutcome::Absent,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    terminate(&mut child);
                    return ProbeOutcome::TimedOut;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    };
    if !status.success() {
        return ProbeOutcome::Absent;
    }
    let pick = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .next()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
    };
    // Bounded by the SAME deadline. A descendant holding the pipe cannot make a
    // probe outlive its budget; what it costs is the version string, not time.
    let remaining = || deadline.saturating_duration_since(std::time::Instant::now());
    let stdout = out.recv_timeout(remaining()).unwrap_or_default();
    let stderr = err.recv_timeout(remaining()).unwrap_or_default();
    ProbeOutcome::Version(pick(&stdout).or_else(|| pick(&stderr)).unwrap_or_default())
}

/// Stop a probe and everything it started.
///
/// `Child::kill` signals only the direct child, so a descendant would survive
/// holding the pipe. On Unix the probe is its own process group (set at spawn),
/// which is what makes one signal reach the whole tree.
fn terminate(child: &mut std::process::Child) {
    // Before the child is reaped, so its pid cannot have been reused. Negative
    // pid addresses the group. Best effort: the group is already gone if the
    // child exited, which is not an error worth reporting.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Decrements the in-flight count however `check_spec` returns.
#[cfg(test)]
fn scopeguard_leave() -> impl Drop {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            leave_probe();
        }
    }
    Leave
}

/// A known per-OS install command for a prerequisite, if any.
pub fn install_command(name: &str) -> Option<String> {
    install_info(name).command
}

/// Check a prerequisite by trying each probe in turn.
fn check_spec(spec: &Spec) -> DependencyStatus {
    #[cfg(test)]
    enter_probe();
    #[cfg(test)]
    let _leave = scopeguard_leave();
    // ONE deadline for the whole prerequisite. `python` tries python3 then
    // python, and llama-server tries PATH then the bundled sidecar; a budget
    // per probe would silently double the worst case for exactly those two.
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let mut version = None;
    let mut timed_out = false;
    // Each probe in turn; a timeout is remembered but does not stop the next
    // candidate (Windows' `python3` -> `python` fallback depends on that).
    for (cmd, args) in spec.probes {
        match probe_until(cmd, args, deadline) {
            ProbeOutcome::Version(found) => {
                version = Some(found);
                break;
            }
            ProbeOutcome::TimedOut => timed_out = true,
            ProbeOutcome::Absent => {}
        }
    }
    // llama-server usually isn't on PATH: the desktop app bundles it next to
    // the Biorouter binaries. Fall back to the sidecar's resolution logic.
    if version.is_none() && spec.name == "llama-server" {
        if let Some(bin) = crate::providers::llamacpp_sidecar::find_binary() {
            match probe_until(&bin.display().to_string(), &["--version"], deadline) {
                ProbeOutcome::Version(found) => version = Some(found),
                ProbeOutcome::TimedOut => timed_out = true,
                ProbeOutcome::Absent => {}
            }
        }
    }
    let install = install_info(spec.name);
    DependencyStatus {
        name: spec.name.to_string(),
        display_name: spec.display_name.to_string(),
        installed: version.is_some(),
        timed_out: timed_out && version.is_none(),
        version,
        required: spec.required,
        purpose: spec.purpose.to_string(),
        doc_url: spec.doc_url.to_string(),
        install_command: install.command,
        requires_sudo: install.requires_sudo,
        download_url: install.download_url,
    }
}

/// Check every prerequisite. This is what `biorouter doctor` and the desktop
/// dependency setup both consume.
/// Observes how many probes were in flight at once. Test-only: the concurrency
/// is otherwise unfalsifiable, because every real probe answers in milliseconds
/// when warm and a serial run clears any wall-clock threshold just as easily.
#[cfg(test)]
pub(crate) static PROBE_HIGH_WATER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static PROBES_IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn enter_probe() {
    use std::sync::atomic::Ordering;
    let now = PROBES_IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
    PROBE_HIGH_WATER.fetch_max(now, Ordering::SeqCst);
}

#[cfg(test)]
fn leave_probe() {
    PROBES_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
}

pub fn check_all() -> Vec<DependencyStatus> {
    // Concurrently, because these probes are dominated by the operating system's
    // first-execution scan of a freshly installed binary — near-zero CPU, seconds
    // of wall clock each. Run in series they add up: `biorouter doctor` is on the
    // desktop's startup path under a 15 s budget (ui/desktop/src/utils/
    // dependencyChecker.ts) and on the installed-package check's 60 s budget, and
    // a cold machine blew through both. Order of the results is preserved.
    let specs = specs();
    std::thread::scope(|scope| {
        let handles: Vec<_> = specs
            .iter()
            .map(|spec| scope.spawn(move || check_spec(spec)))
            .collect();
        handles
            .into_iter()
            .zip(specs.iter())
            .map(|(handle, spec)| {
                handle.join().unwrap_or_else(|_| {
                    // A panic here is a defect in the probe, not a slow machine,
                    // so it must NOT be dressed up as a timeout -- that would
                    // print "did not answer in time" and hide a bug. The row is
                    // otherwise built exactly as check_spec builds it.
                    let install = install_info(spec.name);
                    DependencyStatus {
                        name: spec.name.to_string(),
                        display_name: spec.display_name.to_string(),
                        installed: false,
                        timed_out: false,
                        version: None,
                        required: spec.required,
                        purpose: spec.purpose.to_string(),
                        doc_url: spec.doc_url.to_string(),
                        install_command: install.command,
                        requires_sudo: install.requires_sudo,
                        download_url: install.download_url,
                    }
                })
            })
            .collect()
    })
}

/// Check a single prerequisite by name (e.g. `"uv"`). Returns `None` if there is
/// no such spec.
pub fn status_of(name: &str) -> Option<DependencyStatus> {
    specs().iter().find(|s| s.name == name).map(check_spec)
}

// ── self-update check ────────────────────────────────────────────────────────

/// Result of comparing the running version against the latest GitHub release.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub update_available: bool,
    pub url: String,
}

/// True if dotted version `a` is strictly newer than `b` (numeric, segment-wise;
/// pre-release suffixes compared as 0).
fn version_newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| {
        s.trim_start_matches('v')
            .split(['.', '-', '+'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (va, vb) = (parse(a), parse(b));
    for i in 0..va.len().max(vb.len()) {
        let (x, y) = (
            va.get(i).copied().unwrap_or(0),
            vb.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Best-effort check for a newer release on GitHub. Returns `None` on any
/// network/parse error (offline-friendly); times out after 5s.
pub async fn check_for_update() -> Option<UpdateInfo> {
    const CURRENT: &str = env!("CARGO_PKG_VERSION");
    let client = reqwest::Client::builder()
        .user_agent(concat!("biorouter/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let resp = client
        .get("https://api.github.com/repos/BaranziniLab/biorouter/releases/latest")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    let latest = json
        .get("tag_name")?
        .as_str()?
        .trim_start_matches('v')
        .to_string();
    let url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/BaranziniLab/biorouter/releases")
        .to_string();
    Some(UpdateInfo {
        update_available: version_newer(&latest, CURRENT),
        current: CURRENT.to_string(),
        latest,
        url,
    })
}

// ── CLI-on-PATH install ──────────────────────────────────────────────────────

/// Where (if anywhere) a `biorouter` executable currently resolves on PATH.
pub fn biorouter_on_path() -> Option<PathBuf> {
    let exe = if cfg!(target_os = "windows") {
        "biorouter.exe"
    } else {
        "biorouter"
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(exe);
            candidate.is_file().then_some(candidate)
        })
    })
}

fn dir_on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == dir))
        .unwrap_or(false)
}

/// Outcome of installing the CLI onto PATH.
#[derive(Debug, Serialize)]
pub struct CliInstall {
    pub link: PathBuf,
    pub target_dir: PathBuf,
    pub on_path: bool,
}

/// Candidate install directories, best first.
fn install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(target_os = "windows") {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Biorouter").join("bin"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/local/bin"));
        if let Ok(home) = etcetera::home_dir() {
            dirs.push(home.join(".local").join("bin"));
            dirs.push(home.join("bin"));
        }
    }
    dirs
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".biorouter-write-test");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// The path a file really has, with symlinks followed and Windows' verbatim
/// prefix stripped.
///
/// Falls back to the path as given when it cannot be resolved: a caller that
/// treats the result as a hint and checks the file it derives is better served
/// by a guess than by a failure. `biorouter-cli`'s `commands::exe_path` module
/// carries the full account of why this matters — it is the one that discovered
/// it — and delegates here so there is a single implementation.
pub fn real_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .map(|p| dunce::simplified(&p).to_path_buf())
        .unwrap_or_else(|_| path.to_path_buf())
}

/// File name of the breadcrumb an installed copy leaves beside itself, naming
/// the directory it was installed from.
///
/// Defined once, here, and read by `biorouter-cli`'s resolvers rather than
/// re-spelled there: an installer and a reader that disagree about this string
/// fail silently, and only on the platform that needs it.
pub const INSTALL_ORIGIN_FILE: &str = ".biorouter-origin";

/// Record, beside an installed copy of the CLI, the directory it was taken from.
///
/// Windows has no symlink to follow. [`install_cli`] *copies* `biorouter.exe`
/// into a `PATH` directory, so `biorouterd.exe` and the interface bundle that
/// shipped with it are no longer anywhere near the executable — and neither
/// `biorouter serve` nor `biorouter apps` could find them. This one line of text
/// is how the copy gets back to them, and it is rewritten on every install so it
/// keeps pointing at the application after an update moves it.
///
/// A copy is not made instead: `biorouterd.exe` is over 200 MB and the interface
/// bundle another 12, and a copy taken at install time goes stale the moment the
/// application updates.
///
/// **Writes nothing when the source and the target are the same directory.** The
/// installed copy is itself on `PATH`, so `biorouter setup-path` can be run
/// *from* it — at which point the source directory is the install directory, and
/// a breadcrumb naming its own directory would resolve nothing while destroying
/// the usable one the application wrote.
pub fn record_install_origin(source_dir: &Path, target_dir: &Path) -> std::io::Result<()> {
    if same_dir(source_dir, target_dir) {
        return Ok(());
    }
    let origin = real_path(source_dir);
    std::fs::write(
        target_dir.join(INSTALL_ORIGIN_FILE),
        origin.to_string_lossy().as_bytes(),
    )
}

/// The directory an installed copy in `install_dir` was taken from, if it left a
/// breadcrumb.
///
/// Tolerant by construction, because every one of these is a state a real
/// machine reaches: no file (a Unix install, or a copy from before this
/// existed), an empty or truncated file (a write interrupted by a crash), an
/// unreadable one, a leading byte-order mark or trailing newline from a text
/// editor, or a path that no longer exists.
///
/// A recorded directory that has since been deleted is returned anyway rather
/// than being filtered out here: the caller checks for the file it wants, and
/// the stale path then appears in the list of places it looked, which is the
/// only way the reader learns their breadcrumb is out of date.
pub fn install_origin(install_dir: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(install_dir.join(INSTALL_ORIGIN_FILE)).ok()?;
    let line = raw.trim_start_matches('\u{feff}').lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let path = Path::new(line);
    // Anything that is not an absolute path is not something we wrote. Resolving
    // it against the working directory would invent a location rather than
    // report one.
    path.is_absolute()
        .then(|| dunce::simplified(path).to_path_buf())
}

/// Whether two paths name the same directory, comparing what is on disk when it
/// can and the spellings when it cannot.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Put `source` at `link`: a symlink on Unix, a copy plus an origin breadcrumb
/// on Windows.
///
/// Split out of [`install_cli`] so the platform difference is one small function
/// rather than two arms in the middle of a long one.
fn place_cli(source: &Path, link: &Path, target_dir: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let _ = target_dir;
        std::os::unix::fs::symlink(source, link).map_err(|e| {
            anyhow::anyhow!(
                "Failed to link {} -> {}: {}",
                link.display(),
                source.display(),
                e
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(source, link).map(|_| ()).map_err(|e| {
            anyhow::anyhow!(
                "Failed to copy {} -> {}: {}",
                source.display(),
                link.display(),
                e
            )
        })?;
        // The copy has no siblings, so leave it a note saying where it came
        // from. A failure here is not a failed install — everything except
        // `serve` and `apps` works without it — so it is reported, not raised.
        if let Some(source_dir) = source.parent() {
            if let Err(e) = record_install_origin(source_dir, target_dir) {
                tracing::warn!(
                    "installed the CLI, but could not record where it came from in {}: {e}. \
                     `biorouter serve` may not be able to find biorouterd or the interface.",
                    target_dir.display()
                );
            }
        }
    }
    Ok(())
}

/// Install (symlink on Unix, copy on Windows) `source` onto a PATH directory so
/// `biorouter` is callable from any terminal. `source` is normally the running
/// executable (CLI) or the bundled binary (desktop). Returns where it landed.
pub fn install_cli(source: &Path) -> anyhow::Result<CliInstall> {
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    let exe_name = if cfg!(target_os = "windows") {
        "biorouter.exe"
    } else {
        "biorouter"
    };

    // If a `biorouter` already resolves on PATH, that's the binary the user's
    // shell actually runs — overwrite *it* so an update/reinstall takes effect.
    // Otherwise a stale copy that sits earlier on PATH (e.g. an old 1.x in
    // /usr/local/bin or a user dir) keeps shadowing a fresh symlink we'd drop in
    // ~/.local/bin, and the "update" silently never applies. Only do this when
    // the existing location is writable and isn't the source itself.
    let existing_on_path = biorouter_on_path().and_then(|p| {
        let dir = p.parent()?.to_path_buf();
        let canon = p.canonicalize().unwrap_or(p);
        (dir.is_dir() && is_writable(&dir) && canon != source).then_some(dir)
    });

    // Prefer a writable, already-on-PATH dir; else the first writable dir; else
    // the first candidate (creating it if needed).
    let dirs = install_dirs();
    let target_dir = existing_on_path
        .or_else(|| {
            dirs.iter()
                .find(|d| d.is_dir() && is_writable(d) && dir_on_path(d))
                .or_else(|| dirs.iter().find(|d| d.is_dir() && is_writable(d)))
                .cloned()
        })
        .or_else(|| dirs.first().cloned())
        .ok_or_else(|| anyhow::anyhow!("No suitable install directory found"))?;

    std::fs::create_dir_all(&target_dir)
        .map_err(|e| anyhow::anyhow!("Could not create {}: {}", target_dir.display(), e))?;
    if !is_writable(&target_dir) {
        anyhow::bail!(
            "{} is not writable. Re-run with elevated permissions, or add a writable directory to PATH.",
            target_dir.display()
        );
    }

    let link = target_dir.join(exe_name);
    if link.exists() || link.symlink_metadata().is_ok() {
        let _ = std::fs::remove_file(&link);
    }

    place_cli(&source, &link, &target_dir)?;

    Ok(CliInstall {
        on_path: dir_on_path(&target_dir),
        link,
        target_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::{install_command, specs, status_of};

    #[test]
    fn rust_spec_exists_and_is_optional() {
        let spec = specs()
            .iter()
            .find(|s| s.name == "rust")
            .expect("rust spec");
        assert!(
            !spec.required,
            "rust must be optional, not a blocking prereq"
        );
        // Health probe, not a presence probe: -vV exits non-zero on a broken
        // toolchain so it reads as unavailable.
        assert_eq!(spec.probes[0], ("rustc", &["-vV"][..]));
    }

    #[test]
    fn rust_install_command_uses_rustup_not_brew() {
        let cmd = install_command("rust").expect("rust install command on this OS");
        assert!(cmd.contains("rustup") || cmd.contains("Rustup"));
        assert!(!cmd.contains("brew install rust"));
    }

    #[test]
    fn rust_install_command_is_non_interactive() {
        // The app runs installers without a TTY, so the rustup-init pipe must
        // accept defaults (`-y`) and winget must not block on its agreement
        // prompts — otherwise the install aborts with "Unable to run
        // interactively" (Unix) / a hung agreement prompt (Windows).
        let cmd = install_command("rust").expect("rust install command on this OS");
        if cmd.contains("rustup.rs") {
            assert!(
                cmd.contains("-y"),
                "piped rustup install must pass -y: {cmd}"
            );
        } else {
            assert!(
                cmd.contains("--accept-source-agreements"),
                "winget install must accept agreements non-interactively: {cmd}"
            );
        }
    }

    #[test]
    fn status_of_rust_resolves() {
        // Should return a status (installed or not) rather than None.
        assert!(status_of("rust").is_some());
    }
}

/// The breadcrumb a Windows install leaves so the copied `biorouter.exe` can
/// still find `biorouterd.exe` and the interface bundle.
///
/// None of these are gated on Windows: the reader and the writer are plain path
/// and string handling, and a rule only one platform ever runs is a rule only
/// one platform's CI can catch a regression in.
#[cfg(test)]
mod install_origin_tests {
    use super::{install_origin, record_install_origin, INSTALL_ORIGIN_FILE};
    use std::path::{Path, PathBuf};

    /// A fixture shaped like the shipped Windows application: the two binaries
    /// in `resources/bin`, the interface bundle beside them in `resources/web`.
    fn application(root: &Path) -> PathBuf {
        let bin = root.join("resources").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join(format!("biorouter{}", std::env::consts::EXE_SUFFIX)),
            b"x",
        )
        .unwrap();
        std::fs::write(
            bin.join(format!("biorouterd{}", std::env::consts::EXE_SUFFIX)),
            b"x",
        )
        .unwrap();
        let web = root.join("resources").join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), b"<!doctype html>").unwrap();
        bin
    }

    #[test]
    fn an_install_records_the_directory_it_was_copied_from() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = application(&tmp.path().join("Application"));
        let install_dir = tmp.path().join("Local").join("Biorouter").join("bin");
        std::fs::create_dir_all(&install_dir).unwrap();

        record_install_origin(&source_dir, &install_dir).unwrap();

        assert_eq!(
            install_origin(&install_dir).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&source_dir).unwrap()),
            "the copy must be able to name the application it came from"
        );
    }

    /// Rewritten on every install, because an application update moves the
    /// directory the previous breadcrumb names.
    #[test]
    fn a_second_install_replaces_the_recorded_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let first = application(&tmp.path().join("Biorouter-1.90.3"));
        let second = application(&tmp.path().join("Biorouter-1.91.0"));
        let install_dir = tmp.path().join("Local").join("Biorouter").join("bin");
        std::fs::create_dir_all(&install_dir).unwrap();

        record_install_origin(&first, &install_dir).unwrap();
        record_install_origin(&second, &install_dir).unwrap();

        assert_eq!(
            install_origin(&install_dir).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&second).unwrap()),
            "the newest install must win, or an update strands the CLI on the old bundle"
        );
    }

    /// The trap that silently destroys a working install: the installed copy is
    /// itself on `PATH`, so `biorouter setup-path` can be run **from** it. The
    /// source directory is then the install directory, and a naive writer would
    /// record the install directory as its own origin — resolving nothing, and
    /// overwriting the usable breadcrumb the application card wrote.
    #[test]
    fn installing_from_the_install_directory_leaves_the_breadcrumb_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let application_bin = application(&tmp.path().join("Application"));
        let install_dir = tmp.path().join("Local").join("Biorouter").join("bin");
        std::fs::create_dir_all(&install_dir).unwrap();
        record_install_origin(&application_bin, &install_dir).unwrap();

        // `biorouter setup-path`, run from the copy on PATH.
        record_install_origin(&install_dir, &install_dir).unwrap();

        assert_eq!(
            install_origin(&install_dir).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&application_bin).unwrap()),
            "re-installing from the install directory must not overwrite the good breadcrumb"
        );
    }

    #[test]
    fn installing_from_the_install_directory_writes_nothing_at_all() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("Local").join("Biorouter").join("bin");
        std::fs::create_dir_all(&install_dir).unwrap();

        record_install_origin(&install_dir, &install_dir).unwrap();

        assert!(
            install_origin(&install_dir).is_none(),
            "a breadcrumb naming its own directory resolves nothing; none is better"
        );
        assert!(
            !install_dir.join(INSTALL_ORIGIN_FILE).exists(),
            "no breadcrumb should have been created"
        );
    }

    /// A Unix install, or a copy made before this existed.
    #[test]
    fn a_missing_breadcrumb_is_none_and_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(install_origin(tmp.path()).is_none());
        assert!(install_origin(&tmp.path().join("no-such-directory")).is_none());
    }

    /// A write interrupted by a crash, or a file a user emptied.
    #[test]
    fn an_empty_or_unparseable_breadcrumb_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        for content in ["", "   ", "\n\n", "not a path", "./relative/bin"] {
            std::fs::write(tmp.path().join(INSTALL_ORIGIN_FILE), content).unwrap();
            assert!(
                install_origin(tmp.path()).is_none(),
                "expected no origin from {content:?}"
            );
        }
    }

    /// A text editor's byte-order mark and trailing newline must not make the
    /// path unrecognisable.
    #[test]
    fn whitespace_and_a_byte_order_mark_are_tolerated() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = application(&tmp.path().join("Application"));
        let install_dir = tmp.path().join("Local");
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(
            install_dir.join(INSTALL_ORIGIN_FILE),
            format!("\u{feff}  {}  \r\n", source_dir.display()),
        )
        .unwrap();

        assert_eq!(
            install_origin(&install_dir).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&source_dir).unwrap())
        );
    }

    /// A breadcrumb naming a directory that has since been deleted is returned
    /// rather than filtered out: the caller checks for the file it wants, and
    /// the stale path then appears in the list of places it looked. Swallowing
    /// it here would leave the reader with a failure that names one location
    /// fewer than were actually tried.
    #[test]
    fn a_stale_breadcrumb_is_still_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("Application").join("resources").join("bin");
        let install_dir = tmp.path().join("Local");
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(
            install_dir.join(INSTALL_ORIGIN_FILE),
            gone.to_string_lossy().as_bytes(),
        )
        .unwrap();

        assert_eq!(
            install_origin(&install_dir),
            Some(gone),
            "a stale breadcrumb must survive to the error message"
        );
    }

    /// Runs everywhere and is only capable of failing on Windows, where
    /// `install_cli` canonicalises its source and `canonicalize` returns
    /// `\\?\C:\…`. That string would reach `BIOROUTER_SERVE_UI` in a child
    /// process's environment and the "Tried:" list a user reads.
    #[test]
    fn the_recorded_origin_is_never_a_windows_verbatim_path() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = application(&tmp.path().join("Application"));
        // Exactly what `install_cli` hands the writer: the parent of a
        // canonicalised source path.
        let canonical_source = std::fs::canonicalize(
            source_dir.join(format!("biorouter{}", std::env::consts::EXE_SUFFIX)),
        )
        .unwrap();
        let install_dir = tmp.path().join("Local");
        std::fs::create_dir_all(&install_dir).unwrap();

        record_install_origin(canonical_source.parent().unwrap(), &install_dir).unwrap();

        let written = std::fs::read_to_string(install_dir.join(INSTALL_ORIGIN_FILE)).unwrap();
        assert!(
            !written.starts_with(r"\\?\"),
            "a verbatim path was written into the breadcrumb: {written}"
        );
        let read_back = install_origin(&install_dir).expect("an origin");
        assert!(
            !read_back.to_string_lossy().starts_with(r"\\?\"),
            "a verbatim path was read back out of the breadcrumb: {}",
            read_back.display()
        );
    }

    #[test]
    fn a_verbatim_path_already_in_a_breadcrumb_is_stripped_on_the_way_out() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(INSTALL_ORIGIN_FILE),
            r"\\?\C:\Program Files\Biorouter\resources\bin",
        )
        .unwrap();

        let read_back = install_origin(tmp.path());
        // Only Windows recognises that string as an absolute path at all. There
        // it must come back simplified; on every other platform the point is
        // that it is refused rather than resolved against the working directory.
        #[cfg(windows)]
        assert_eq!(
            read_back,
            Some(PathBuf::from(r"C:\Program Files\Biorouter\resources\bin"))
        );
        #[cfg(not(windows))]
        assert_eq!(read_back, None);
    }
}

// ── debugging a failed prerequisite ──────────────────────────────────────────

/// Everything a fresh Biorouter session needs to diagnose a failed install.
///
/// The desktop app has the same idea in TypeScript
/// (`ui/desktop/src/utils/dependencyDebugPrompt.ts`); this is the terminal's
/// copy, so `biorouter doctor --fix <dep>` opens a session with the same
/// briefing the desktop's "Debug with Biorouter" button produces. Keeping the
/// two in step is a docs-level obligation, not a compile-time one — they render
/// for different front ends and neither can import the other.
pub struct DependencyFailure<'a> {
    pub status: &'a DependencyStatus,
    /// Combined stdout/stderr, when an install was actually attempted.
    pub output: Option<&'a str>,
    /// The error the user was shown, when there is one.
    pub error: Option<&'a str>,
}

/// Captured output can be enormous (a `uv sync` compiling from source). The tail
/// is where the failure is, so keep the tail.
const MAX_OUTPUT_CHARS: usize = 4_000;

fn truncate_tail(output: &str) -> String {
    let trimmed = output.trim_end();
    // Count characters, not bytes — slicing a multi-byte boundary would panic.
    let total = trimmed.chars().count();
    if total <= MAX_OUTPUT_CHARS + 64 {
        return trimmed.to_string();
    }
    let kept: String = trimmed
        .chars()
        .skip(total - MAX_OUTPUT_CHARS)
        .collect::<String>();
    format!(
        "…[{} earlier characters omitted]…\n{}",
        total - MAX_OUTPUT_CHARS,
        kept
    )
}

/// Pick a fence long enough that a fence inside `body` cannot end it early.
fn fence(body: &str) -> String {
    let longest = body
        .split(|c| c != '`')
        .map(|run| run.len())
        .max()
        .unwrap_or(0);
    let delim = "`".repeat(std::cmp::max(3, longest + 1));
    format!("{delim}\n{body}\n{delim}")
}

/// Build the opening message for a session that is meant to fix a prerequisite.
///
/// Structure is fixed on purpose: goal, what failed, the evidence, the machine,
/// then the rules — evidence above rules so a model that skims still reads the
/// error.
pub fn debug_prompt(failure: &DependencyFailure<'_>) -> String {
    let d = failure.status;
    let mut out = String::new();

    out.push_str(&format!(
        "I am trying to install **{}**, a required system dependency for Biorouter, \
         and it is not working. Please diagnose why and fix it on this machine.\n\n",
        d.display_name
    ));

    out.push_str("## What failed\n\n");
    out.push_str(&format!(
        "- What failed: {} (`{}`)\n",
        d.display_name, d.name
    ));
    out.push_str(&format!("- What Biorouter needs it for: {}\n", d.purpose));
    out.push_str(&format!(
        "- Currently detected as: {}\n",
        if d.installed {
            d.version.as_deref().unwrap_or("installed")
        } else if d.timed_out {
            // Do not hand the debugging agent an absence nobody established.
            "check timed out — presence unknown, not disproved"
        } else {
            "not installed"
        }
    ));
    if let Some(cmd) = &d.install_command {
        out.push_str(&format!("- Install command Biorouter suggests: `{cmd}`\n"));
    }
    if d.requires_sudo {
        out.push_str("- This install normally needs administrator privileges.\n");
    }
    if !d.doc_url.is_empty() {
        out.push_str(&format!("- Official install page: {}\n", d.doc_url));
    }
    if let Some(err) = failure.error {
        out.push_str(&format!("- Error reported: {err}\n"));
    }

    if let Some(output) = failure.output.filter(|o| !o.trim().is_empty()) {
        out.push_str(&format!(
            "\n## Output from the failed command\n\n{}\n",
            fence(&truncate_tail(output))
        ));
    }

    out.push_str("\n## This machine\n\n");
    out.push_str(&format!(
        "- Platform: {} ({})\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    out.push_str(&format!(
        "- Biorouter version: {}\n",
        env!("CARGO_PKG_VERSION")
    ));
    if let Ok(path) = std::env::var("PATH") {
        out.push_str(&format!(
            "- PATH this shell searches:\n{}\n",
            fence(&path.replace([':', ';'], "\n"))
        ));
    }

    out.push_str(&format!(
        "\n## What I need from you\n\n\
         1. Work out the actual cause — not the most common cause.\n\
         2. Fix it using the shell. Prefer the least invasive fix that works.\n\
         3. Verify by running `{} --version` yourself and show me the result.\n\
         4. Tell me in plain language what was wrong and what you changed.\n\n\
         Rules:\n\
         - Ask me first before anything needing `sudo`, anything that removes or downgrades \
         software I did not ask about, or anything that edits my shell profile.\n\
         - If the fix needs me to do something by hand, say so and give me the exact steps.\n\
         - If it turns out to be installed already and merely not on PATH, say that — the fix \
         is the PATH, not another install.\n",
        d.name
    ));

    out
}

#[cfg(test)]
mod debug_prompt_tests {
    use super::*;

    fn status(name: &str) -> DependencyStatus {
        DependencyStatus {
            name: name.to_string(),
            display_name: format!("{name} (test)"),
            installed: false,
            timed_out: false,
            version: None,
            required: true,
            purpose: "test purpose".to_string(),
            doc_url: "https://example.invalid".to_string(),
            install_command: Some("brew install thing".to_string()),
            requires_sudo: false,
            download_url: None,
        }
    }

    #[test]
    fn carries_the_evidence_and_the_rules() {
        let s = status("uv");
        let p = debug_prompt(&DependencyFailure {
            status: &s,
            output: Some("curl: (6) Could not resolve host"),
            error: Some("exit code 6"),
        });
        assert!(p.contains("uv (test)"));
        assert!(p.contains("brew install thing"));
        assert!(p.contains("Could not resolve host"));
        assert!(p.contains("uv --version"));
        assert!(p.contains("Ask me first"));
        // Evidence must precede instructions.
        assert!(
            p.find("Could not resolve host").unwrap() < p.find("What I need from you").unwrap()
        );
    }

    #[test]
    fn output_containing_a_fence_cannot_break_out() {
        let s = status("uv");
        let p = debug_prompt(&DependencyFailure {
            status: &s,
            output: Some("before\n```\ninner\n```\nafter"),
            error: None,
        });
        assert!(p.contains("````"));
    }

    #[test]
    fn truncation_keeps_the_tail_and_never_grows_the_input() {
        let long = format!("{}THE_ACTUAL_ERROR", "x".repeat(MAX_OUTPUT_CHARS * 2));
        let t = truncate_tail(&long);
        assert!(t.contains("THE_ACTUAL_ERROR"));
        assert!(t.chars().count() < long.chars().count());

        let just_over = "z".repeat(MAX_OUTPUT_CHARS + 16);
        assert!(truncate_tail(&just_over).chars().count() <= just_over.chars().count());
    }

    #[test]
    fn multibyte_output_does_not_panic() {
        let long = "é".repeat(MAX_OUTPUT_CHARS * 2);
        let t = truncate_tail(&long);
        assert!(t.contains("earlier characters omitted"));
    }

    #[test]
    fn works_with_nothing_but_a_status() {
        let s = status("git");
        let p = debug_prompt(&DependencyFailure {
            status: &s,
            output: None,
            error: None,
        });
        assert!(p.contains("git --version"));
        assert!(!p.contains("Output from the failed command"));
    }
}

#[cfg(all(test, unix))]
mod probe_bound_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A probe that never returns must be abandoned and its child reaped, rather
    /// than hanging `biorouter doctor` forever. Before this bound existed,
    /// `Command::output()` waited indefinitely on every platform.
    #[test]
    fn a_hanging_probe_is_bounded_killed_and_reported_as_unknown() {
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let outcome = probe_within("/bin/sh", &["-c", "sleep 60"], budget);
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome, ProbeOutcome::TimedOut),
            "a hanging probe must report TimedOut, not Absent: collapsing them tells \
             the user to install software they already have"
        );
        assert!(
            elapsed < budget * 10,
            "probe took {elapsed:?}, which is not bounded by {budget:?}"
        );
    }

    /// A child can answer, exit, and still leave a descendant holding the
    /// inherited pipe. Reading that pipe to EOF then waits for the DESCENDANT.
    /// Measured at 30 s against a 2 s budget before the read was bounded -- the
    /// same mechanism that makes a held pipe look like slow work on the Python
    /// side of the installed-package harness.
    #[test]
    fn a_descendant_holding_the_pipe_cannot_outlive_the_budget() {
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let outcome = probe_within(
            "/bin/sh",
            &["-c", "echo 1.2.3; sleep 30 & exit 0"],
            budget,
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed < budget * 10,
            "probe took {elapsed:?} for a {budget:?} budget: the read is not bounded"
        );
        // Whichever answer it lands on, it must not have BLOCKED for it.
        match outcome {
            ProbeOutcome::Version(_) | ProbeOutcome::TimedOut => {}
            ProbeOutcome::Absent => panic!("a command that exited 0 is not absent"),
        }
    }

    #[test]
    fn a_probe_that_answers_is_unaffected_by_the_bound() {
        let outcome = probe_within("/bin/echo", &["1.2.3"], Duration::from_secs(5));
        match outcome {
            ProbeOutcome::Version(version) => assert_eq!(version, "1.2.3"),
            _ => panic!("a command that answers must report its version line"),
        }
    }

    #[test]
    fn a_missing_command_is_absent_not_a_timeout() {
        assert!(matches!(
            probe_within("/nonexistent/biorouter-probe-fixture", &[], Duration::from_secs(5)),
            ProbeOutcome::Absent
        ));
        // A command that exists but fails is also Absent, not TimedOut.
        assert!(matches!(
            probe_within("/bin/sh", &["-c", "exit 3"], Duration::from_secs(5)),
            ProbeOutcome::Absent
        ));
    }

    /// A child that writes more than one pipe buffer must not deadlock the probe.
    /// Draining stdout and stderr in sequence hangs here; concurrent drains do not.
    #[test]
    fn a_noisy_child_does_not_deadlock_the_probe() {
        let outcome = probe_within(
            "/bin/sh",
            &["-c", "yes error | head -c 200000 >&2; echo 9.9.9"],
            Duration::from_secs(20),
        );
        match outcome {
            ProbeOutcome::Version(version) => assert_eq!(version, "9.9.9"),
            _ => panic!("a child that fills stderr must still yield its stdout version"),
        }
    }

    /// The probes run concurrently, so a cold machine pays roughly the slowest
    /// probe rather than the sum. This is what keeps `biorouter doctor` inside the
    /// desktop's 15 s startup budget and the installed-package check's 60 s one.
    /// Asserts the PROPERTY, not the clock. Every real probe answers in
    /// milliseconds when warm, so a wall-clock threshold is satisfied by a
    /// serial run too and would pin nothing: reverting `check_all` to
    /// `.iter().map(check_spec)` left the old version of this test green.
    #[test]
    fn check_all_runs_its_probes_concurrently() {
        use std::sync::atomic::Ordering;
        let specs = specs();
        PROBE_HIGH_WATER.store(0, Ordering::SeqCst);
        let statuses = check_all();
        let peak = PROBE_HIGH_WATER.load(Ordering::SeqCst);
        assert_eq!(statuses.len(), specs.len());
        for (status, spec) in statuses.iter().zip(specs.iter()) {
            assert_eq!(status.name, spec.name, "check_all must preserve spec order");
        }
        assert!(
            peak > 1,
            "at most {peak} probe was ever in flight: check_all ran its {} specs in \
             series, so a cold machine pays their SUM. That is what blew the \
             desktop's startup budget and the installed-package check's.",
            specs.len()
        );
    }
}
