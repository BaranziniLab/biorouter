//! Where live dictation can run, decided before any device opens (DD-73; the detection
//! order of `docs/dictation/surfaces.md`, "The terminal").
//!
//! [`Environment::sample`] reads the variables, files and processes the order needs, once;
//! [`detect`] is a pure function of that sample and of what the engine, the policy and the
//! capture child's probe said, so the order and each sentence are tested as a table. The
//! heuristics choose the wording only: apart from SSH on macOS (step 5), they never refuse
//! a device that works, because step 7 asks the capture child first. A host that can never
//! capture live answers with its routes before the model check (step 8), so nobody is told
//! to download a 264 to 574 MB model only to be refused on the next run.

use biorouter_dictation_capture::{ErrorCode, Probe};

use super::messages;

/// The operating system, as the sentences need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

/// Windows Subsystem for Linux, when this Linux is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wsl {
    Wsl1,
    /// `wslg`: `/mnt/wslg/PulseServer` exists; `libpulse`: `libpulse.so.0` is installed.
    Wsl2 {
        wslg: bool,
        libpulse: bool,
    },
}

/// The terminal app macOS charges the microphone to: the app that started this process
/// (`__CFBundleIdentifier`), or `TERM_PROGRAM` when that is unset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalApp {
    pub name: String,
    pub bundle_id: Option<String>,
}

/// One sample of everything the detection order reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environment {
    pub os: Option<Os>,
    /// `BIOROUTER_TERMINAL` is set: an agent's shell.
    pub agent_shell: bool,
    /// `prompt_can_run()`: stdin and stderr are terminals.
    pub terminal: bool,
    /// `SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY` is set.
    pub ssh_variables: bool,
    /// An `sshd` or `sshd-session` process is an ancestor of this one.
    pub sshd_ancestor: bool,
    /// macOS only: whether this process runs in the logged in desktop session (`launchctl
    /// managername` answers `Aqua`). `None` when it was not asked or could not be read.
    pub desktop_session: Option<bool>,
    /// `/.dockerenv`, `/run/.containerenv`, `KUBERNETES_SERVICE_HOST`, or a container
    /// runtime named in `/proc/1/cgroup`.
    pub container: bool,
    pub wsl: Option<Wsl>,
    /// A `biorouter serve` process of this account on this host, by a scan of `/proc`
    /// (Linux), and the port its command line names.
    pub serve_port: Option<u16>,
    pub terminal_app: Option<TerminalApp>,
    /// The name the SSH sentences put where the laptop's command names this computer.
    pub host: String,
}

impl Environment {
    /// Over SSH, for the wording of step 7: the variables, or an `sshd` ancestor (tmux
    /// keeps variables from the session that started its server).
    pub fn over_ssh(&self) -> bool {
        self.ssh_variables || self.sshd_ancestor
    }

    /// Step 5, the one heuristic that refuses: macOS over SSH. A tmux pane can hold stale
    /// SSH variables in a desktop session, so when macOS says this is the desktop session
    /// the variables are not believed.
    pub fn mac_over_ssh(&self) -> bool {
        self.os == Some(Os::Mac) && self.over_ssh() && self.desktop_session != Some(true)
    }

    /// Reads the environment once. The macOS session is asked only when SSH signals exist,
    /// and the process scans run only on Linux.
    pub fn sample() -> Environment {
        let os = Os::current();
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let ssh_variables = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
            .iter()
            .any(|name| var(name).is_some());
        let sshd_ancestor = process::sshd_ancestor();
        let desktop_session = if os == Os::Mac && (ssh_variables || sshd_ancestor) {
            process::mac_desktop_session()
        } else {
            None
        };
        let linux = os == Os::Linux;
        Environment {
            os: Some(os),
            agent_shell: var(super::consent::AGENT_SHELL_ENV).is_some(),
            terminal: crate::commands::needs_terminal::prompt_can_run(),
            ssh_variables,
            sshd_ancestor,
            desktop_session,
            container: linux
                && (std::path::Path::new("/.dockerenv").exists()
                    || std::path::Path::new("/run/.containerenv").exists()
                    || var("KUBERNETES_SERVICE_HOST").is_some()
                    || std::fs::read_to_string("/proc/1/cgroup")
                        .is_ok_and(|cgroup| names_a_container_runtime(&cgroup))),
            wsl: if linux { wsl_from_host() } else { None },
            serve_port: if linux { process::serve_port() } else { None },
            terminal_app: terminal_app(
                var("__CFBundleIdentifier").and_then(|v| v.into_string().ok()),
                var("TERM_PROGRAM").and_then(|v| v.into_string().ok()),
            ),
            host: biorouter::dictation::service::host(),
        }
    }
}

/// Whether `/proc/1/cgroup` names a container runtime.
pub fn names_a_container_runtime(cgroup: &str) -> bool {
    [
        "docker",
        "containerd",
        "kubepods",
        "libpod",
        "lxc",
        "podman",
    ]
    .iter()
    .any(|runtime| cgroup.contains(runtime))
}

/// WSL from `/proc/version` and the WSLg socket. WSL2's kernel names itself
/// `microsoft-standard-WSL2`; WSL1 reports a `Microsoft` kernel with no `WSL2`.
fn wsl_from_host() -> Option<Wsl> {
    let version = std::fs::read_to_string("/proc/version").ok()?;
    let libpulse = [
        "/usr/lib/x86_64-linux-gnu/libpulse.so.0",
        "/usr/lib/aarch64-linux-gnu/libpulse.so.0",
        "/usr/lib64/libpulse.so.0",
        "/usr/lib/libpulse.so.0",
        "/lib/x86_64-linux-gnu/libpulse.so.0",
    ]
    .iter()
    .any(|path| std::path::Path::new(path).exists());
    wsl_from_version(
        &version,
        std::path::Path::new("/mnt/wslg/PulseServer").exists(),
        libpulse,
    )
}

pub fn wsl_from_version(version: &str, wslg: bool, libpulse: bool) -> Option<Wsl> {
    let lower = version.to_ascii_lowercase();
    if !lower.contains("microsoft") {
        return None;
    }
    if lower.contains("wsl2") {
        Some(Wsl::Wsl2 { wslg, libpulse })
    } else {
        Some(Wsl::Wsl1)
    }
}

/// The terminal app from the bundle id macOS passes down, else from `TERM_PROGRAM`.
pub fn terminal_app(
    bundle_id: Option<String>,
    term_program: Option<String>,
) -> Option<TerminalApp> {
    const KNOWN: &[(&str, &str)] = &[
        ("com.apple.Terminal", "Terminal"),
        ("com.googlecode.iterm2", "iTerm2"),
        ("com.mitchellh.ghostty", "Ghostty"),
        ("com.microsoft.VSCode", "VS Code"),
        ("dev.warp.Warp-Stable", "Warp"),
        ("net.kovidgoyal.kitty", "kitty"),
        ("org.alacritty", "Alacritty"),
        ("com.github.wez.wezterm", "WezTerm"),
    ];
    if let Some(id) = bundle_id {
        let name = KNOWN
            .iter()
            .find(|(known, _)| *known == id)
            .map_or_else(|| id.clone(), |(_, name)| (*name).to_string());
        return Some(TerminalApp {
            name,
            bundle_id: Some(id),
        });
    }
    let program = term_program?;
    let (name, id) = match program.as_str() {
        "Apple_Terminal" => ("Terminal", Some("com.apple.Terminal")),
        "iTerm.app" => ("iTerm2", Some("com.googlecode.iterm2")),
        "ghostty" => ("Ghostty", Some("com.mitchellh.ghostty")),
        "vscode" => ("VS Code", Some("com.microsoft.VSCode")),
        "WarpTerminal" => ("Warp", Some("dev.warp.Warp-Stable")),
        other => (other, None),
    };
    Some(TerminalApp {
        name: name.to_string(),
        bundle_id: id.map(str::to_string),
    })
}

/// Why this host cannot capture live (step 7), in the order its wording is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoCaptureReason {
    /// `service_port`: a `biorouter serve` runs here (MSG-SSH-SERVICE).
    Ssh {
        service_port: Option<u16>,
    },
    Container,
    Wsl1,
    Wsl2NoWslg,
    Wsl2NoPulse,
    LinuxNoAudio,
    NoDevice,
}

/// The detection order's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureVerdict {
    /// Step 1.
    NotCompiled,
    /// Step 2: `enabled: false`, or `capture: false` (`capture_only`); the sentence is the
    /// policy's own.
    PolicyOff {
        capture_only: bool,
        sentence: String,
    },
    /// Step 3.
    AgentShell,
    /// Step 4.
    NoTerminal,
    /// Step 5.
    MacOverSsh,
    /// Step 6, with MSG-CPU's sentence (MSG-CPU-ROSETTA under Rosetta).
    CpuUnsupported {
        sentence: String,
    },
    /// Step 7, checked before the model.
    NoCapture(NoCaptureReason),
    /// Step 8.
    NoModel,
    Ready,
}

/// What the steps after the environment found: each `None` when that step did not run.
#[derive(Clone, Debug, Default)]
pub struct Findings {
    /// Built with the `dictation` feature.
    pub compiled: bool,
    /// The managed policy's refusal: `(capture_only, sentence)`.
    pub policy: Option<(bool, String)>,
    /// MSG-CPU's sentence when the processor is refused.
    pub cpu: Option<String>,
    pub probe: Option<Probe>,
    /// Whether a speech model is installed.
    pub model: Option<bool>,
}

/// The detection order, steps 1 to 8. Steps 9 and 10 happen when the device opens.
pub fn detect(env: &Environment, found: &Findings) -> CaptureVerdict {
    if !found.compiled {
        return CaptureVerdict::NotCompiled;
    }
    if let Some((capture_only, sentence)) = &found.policy {
        return CaptureVerdict::PolicyOff {
            capture_only: *capture_only,
            sentence: sentence.clone(),
        };
    }
    if env.agent_shell {
        return CaptureVerdict::AgentShell;
    }
    if !env.terminal {
        return CaptureVerdict::NoTerminal;
    }
    if env.mac_over_ssh() {
        return CaptureVerdict::MacOverSsh;
    }
    if let Some(sentence) = &found.cpu {
        return CaptureVerdict::CpuUnsupported {
            sentence: sentence.clone(),
        };
    }
    if let Some(probe) = &found.probe {
        if let Some(reason) = no_capture_reason(env, probe) {
            return CaptureVerdict::NoCapture(reason);
        }
    }
    if found.model == Some(false) {
        return CaptureVerdict::NoModel;
    }
    CaptureVerdict::Ready
}

/// Step 7: the probe found no backend or no device, and the environment picks the words.
pub fn no_capture_reason(env: &Environment, probe: &Probe) -> Option<NoCaptureReason> {
    let error = probe.error?;
    // ALSA with no capture device is a server with no sound card: no audio system, as far
    // as dictation is concerned. A sound server or CoreAudio with no device is a missing
    // microphone.
    let no_audio_system = error == ErrorCode::NoBackend
        || error == ErrorCode::NotCompiled
        || probe.backend.as_deref() == Some("ALSA");
    if env.over_ssh() {
        return Some(NoCaptureReason::Ssh {
            service_port: env.serve_port,
        });
    }
    if env.container {
        return Some(NoCaptureReason::Container);
    }
    match env.wsl {
        Some(Wsl::Wsl1) => return Some(NoCaptureReason::Wsl1),
        Some(Wsl::Wsl2 { wslg: false, .. }) => return Some(NoCaptureReason::Wsl2NoWslg),
        Some(Wsl::Wsl2 {
            libpulse: false, ..
        }) => return Some(NoCaptureReason::Wsl2NoPulse),
        _ => {}
    }
    if env.os == Some(Os::Linux) && no_audio_system {
        return Some(NoCaptureReason::LinuxNoAudio);
    }
    Some(NoCaptureReason::NoDevice)
}

/// The status JSON's `capture.reason` for a probe: `null` when it can record.
pub fn status_reason(probe: &Probe) -> Option<&'static str> {
    match probe.error? {
        ErrorCode::NotCompiled => Some("not_compiled"),
        ErrorCode::NoBackend => Some("no_audio_system"),
        _ if probe.backend.as_deref() == Some("ALSA") => Some("no_audio_system"),
        _ => Some("no_device"),
    }
}

impl CaptureVerdict {
    /// The sentences to print, in order. `host` names this computer in the SSH commands.
    pub fn sentences(&self, env: &Environment) -> Vec<String> {
        let host = if env.host.is_empty() {
            "<host>"
        } else {
            env.host.as_str()
        };
        let routes = |lines: &mut Vec<String>, service_port: Option<u16>| {
            lines.push(match service_port {
                Some(port) => messages::msg_ssh_service(&port.to_string(), host),
                None => messages::msg_ssh_routes(host),
            });
            lines.push(messages::msg_sync_hint(host));
        };
        let mut lines = Vec::new();
        match self {
            CaptureVerdict::NotCompiled => lines.push(biorouter::dictation::MSG_NO_ENGINE.into()),
            CaptureVerdict::PolicyOff { sentence, .. } => lines.push(sentence.clone()),
            CaptureVerdict::AgentShell => lines.push(messages::MSG_AGENT_SHELL.into()),
            CaptureVerdict::NoTerminal => lines.push(messages::MSG_NO_TTY.into()),
            CaptureVerdict::MacOverSsh => {
                lines.push(messages::MSG_SSH_MAC.into());
                routes(&mut lines, None);
            }
            CaptureVerdict::CpuUnsupported { sentence } => lines.push(sentence.clone()),
            CaptureVerdict::NoCapture(reason) => match reason {
                NoCaptureReason::Ssh { service_port } => {
                    if env.os == Some(Os::Linux) {
                        lines.push(messages::MSG_LINUX_NOAUDIO.into());
                    }
                    routes(&mut lines, *service_port);
                }
                NoCaptureReason::Container => lines.push(messages::MSG_CONTAINER.into()),
                NoCaptureReason::Wsl1 => lines.push(messages::MSG_WSL1.into()),
                NoCaptureReason::Wsl2NoWslg => lines.push(messages::MSG_WSL2_NOWSLG.into()),
                NoCaptureReason::Wsl2NoPulse => lines.push(messages::MSG_WSL2_NOPULSE.into()),
                NoCaptureReason::LinuxNoAudio => {
                    lines.push(messages::MSG_LINUX_NOAUDIO.into());
                    lines.push(messages::MSG_LINUX_ROUTES.into());
                }
                NoCaptureReason::NoDevice => lines.push(messages::MSG_NO_DEVICE.into()),
            },
            // The model's sentence names the model and its size; the command fills it in.
            CaptureVerdict::NoModel | CaptureVerdict::Ready => {}
        }
        lines
    }

    /// Every refusal before a device opens exits 2.
    pub fn exit_code(&self) -> i32 {
        match self {
            CaptureVerdict::Ready => super::exit::OK,
            _ => super::exit::DECLINED,
        }
    }
}

/// The step 10 sentence: exact zeros for 1.5 s.
pub fn zero_sentence(env: &Environment) -> String {
    match env.os {
        Some(Os::Mac) => {
            let app = env.terminal_app.clone().unwrap_or(TerminalApp {
                name: "your terminal app".into(),
                bundle_id: None,
            });
            messages::msg_tcc_zero(&app.name, app.bundle_id.as_deref().unwrap_or("<bundle id>"))
        }
        Some(Os::Windows) => messages::MSG_WIN_ZERO.into(),
        _ => messages::MSG_SILENT.into(),
    }
}

/// The processes the detection order looks at.
mod process {
    /// Whether an `sshd` or `sshd-session` process is an ancestor of this one, at most 32
    /// generations up.
    pub fn sshd_ancestor() -> bool {
        let mut pid = std::process::id();
        for _ in 0..32 {
            let Some((parent, name)) = parent_of(pid) else {
                return false;
            };
            if name == "sshd" || name == "sshd-session" {
                return true;
            }
            if parent <= 1 || parent == pid {
                return false;
            }
            pid = parent;
        }
        false
    }

    #[cfg(target_os = "linux")]
    fn parent_of(pid: u32) -> Option<(u32, String)> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `pid (comm) state ppid ...`; comm may hold spaces and parentheses.
        let open = stat.find('(')?;
        let close = stat.rfind(')')?;
        let name = stat.get(open + 1..close)?.to_string();
        let ppid = stat
            .get(close + 1..)?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        Some((ppid, name))
    }

    #[cfg(target_os = "macos")]
    fn parent_of(pid: u32) -> Option<(u32, String)> {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: `info` is a writable buffer of `size` bytes.
        let read = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
                size,
            )
        };
        if read != size {
            return None;
        }
        let name: Vec<u8> = info
            .pbi_comm
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        Some((info.pbi_ppid, String::from_utf8_lossy(&name).into_owned()))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn parent_of(_pid: u32) -> Option<(u32, String)> {
        None
    }

    /// `launchctl managername` answers `Aqua` in the logged in desktop session.
    pub fn mac_desktop_session() -> Option<bool> {
        let output = std::process::Command::new("/bin/launchctl")
            .arg("managername")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim() == "Aqua")
    }

    /// The port of a `biorouter serve` (or `headless`) process of this account found in
    /// `/proc`. Another account's service is skipped: its token is not this person's, so
    /// its port would be a wrong instruction, and on a shared server it is not theirs to
    /// be told.
    #[cfg(target_os = "linux")]
    pub fn serve_port() -> Option<u16> {
        use std::os::unix::fs::MetadataExt;
        let me = std::process::id().to_string();
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        let uid = unsafe { libc::geteuid() };
        std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .find_map(|entry| {
                let name = entry.file_name();
                let pid = name.to_str()?;
                if !pid.bytes().all(|b| b.is_ascii_digit()) || pid == me {
                    return None;
                }
                // `/proc/<pid>` belongs to the process's effective user.
                if entry.metadata().ok()?.uid() != uid {
                    return None;
                }
                let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
                let args: Vec<String> = cmdline
                    .split(|b| *b == 0)
                    .filter(|a| !a.is_empty())
                    .map(|a| String::from_utf8_lossy(a).into_owned())
                    .collect();
                super::serve_port_of(&args)
            })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn serve_port() -> Option<u16> {
        None
    }
}

/// The port a `biorouter serve` command line listens on, or `None` when `args` is not one.
pub fn serve_port_of(args: &[String]) -> Option<u16> {
    let program = args.first()?;
    let base = std::path::Path::new(program).file_name()?.to_str()?;
    if base != "biorouter" {
        return None;
    }
    let command = args.iter().skip(1).find(|a| !a.starts_with('-'))?;
    if command != "serve" && command != "headless" {
        return None;
    }
    let mut port = crate::commands::serve::DEFAULT_PORT;
    let mut rest = args.iter().skip(2);
    while let Some(arg) = rest.next() {
        if let Some(value) = arg.strip_prefix("--port=") {
            port = value.parse().ok()?;
        } else if arg == "--port" || arg == "-p" {
            port = rest.next()?.parse().ok()?;
        }
    }
    Some(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter_dictation_capture::Device;

    fn local_mac() -> Environment {
        Environment {
            os: Some(Os::Mac),
            terminal: true,
            host: "lab-mac".into(),
            ..Environment::default()
        }
    }

    fn linux() -> Environment {
        Environment {
            os: Some(Os::Linux),
            terminal: true,
            host: "leo".into(),
            ..Environment::default()
        }
    }

    fn found() -> Findings {
        Findings {
            compiled: true,
            probe: Some(working()),
            model: Some(true),
            ..Findings::default()
        }
    }

    fn working() -> Probe {
        Probe {
            compiled: vec!["Core Audio".into()],
            backend: Some("Core Audio".into()),
            devices: vec![Device {
                name: "MacBook Pro Microphone".into(),
                default: true,
            }],
            error: None,
            test_source: false,
        }
    }

    fn no_backend() -> Probe {
        Probe {
            compiled: vec!["PulseAudio".into(), "ALSA".into()],
            error: Some(ErrorCode::NoBackend),
            ..Probe::default()
        }
    }

    fn with_probe(probe: Probe) -> Findings {
        Findings {
            probe: Some(probe),
            ..found()
        }
    }

    /// The detection order, one row per step, each with its exit code: the first step
    /// that fails decides, whatever the later ones would say.
    #[test]
    fn the_order_is_the_tables() {
        let mut agent = local_mac();
        agent.agent_shell = true;
        agent.terminal = false;
        let mut piped = local_mac();
        piped.terminal = false;
        let mut mac_ssh = local_mac();
        mac_ssh.ssh_variables = true;
        let policy = Findings {
            policy: Some((
                true,
                biorouter::managed::MANAGED_DICTATION_CAPTURE_OFF.into(),
            )),
            ..found()
        };
        let cpu = Findings {
            cpu: Some("MSG-CPU".into()),
            probe: Some(no_backend()),
            model: Some(false),
            ..found()
        };
        let rows: Vec<(&str, Environment, Findings, CaptureVerdict)> = vec![
            (
                "not compiled wins over everything",
                agent.clone(),
                Findings::default(),
                CaptureVerdict::NotCompiled,
            ),
            (
                "policy before the agent's shell",
                agent.clone(),
                policy.clone(),
                CaptureVerdict::PolicyOff {
                    capture_only: true,
                    sentence: biorouter::managed::MANAGED_DICTATION_CAPTURE_OFF.into(),
                },
            ),
            (
                "an agent's shell before the terminal",
                agent,
                found(),
                CaptureVerdict::AgentShell,
            ),
            ("no terminal", piped, found(), CaptureVerdict::NoTerminal),
            (
                "macOS over SSH before the processor",
                mac_ssh,
                cpu.clone(),
                CaptureVerdict::MacOverSsh,
            ),
            (
                "the processor before the probe",
                linux(),
                cpu,
                CaptureVerdict::CpuUnsupported {
                    sentence: "MSG-CPU".into(),
                },
            ),
            (
                "the probe before the model",
                linux(),
                Findings {
                    probe: Some(no_backend()),
                    model: Some(false),
                    ..found()
                },
                CaptureVerdict::NoCapture(NoCaptureReason::LinuxNoAudio),
            ),
            (
                "no model",
                local_mac(),
                Findings {
                    model: Some(false),
                    ..found()
                },
                CaptureVerdict::NoModel,
            ),
            ("ready", local_mac(), found(), CaptureVerdict::Ready),
        ];
        for (name, env, found, want) in rows {
            let verdict = detect(&env, &found);
            assert_eq!(verdict, want, "{name}");
            let code = verdict.exit_code();
            assert_eq!(
                code,
                if want == CaptureVerdict::Ready { 0 } else { 2 },
                "{name}"
            );
        }
    }

    /// Step 7's wording, in its order: SSH (variables, or an `sshd` ancestor), a container,
    /// WSL1, WSL2 without WSLg, WSL2 without `libpulse.so.0`, other Linux, else no device.
    #[test]
    fn the_capture_probe_picks_its_words_from_the_environment() {
        let mut ssh_vars = linux();
        ssh_vars.ssh_variables = true;
        let mut ssh_ancestor = linux();
        ssh_ancestor.sshd_ancestor = true;
        let mut ssh_service = linux();
        ssh_service.ssh_variables = true;
        ssh_service.serve_port = Some(9000);
        let mut docker = linux();
        docker.container = true;
        let mut ssh_into_docker = docker.clone();
        ssh_into_docker.ssh_variables = true;
        let mut wsl1 = linux();
        wsl1.wsl = Some(Wsl::Wsl1);
        let mut no_wslg = linux();
        no_wslg.wsl = Some(Wsl::Wsl2 {
            wslg: false,
            libpulse: true,
        });
        let mut no_pulse = linux();
        no_pulse.wsl = Some(Wsl::Wsl2 {
            wslg: true,
            libpulse: false,
        });
        let mut wslg = linux();
        wslg.wsl = Some(Wsl::Wsl2 {
            wslg: true,
            libpulse: true,
        });
        let no_device = Probe {
            backend: Some("Core Audio".into()),
            error: Some(ErrorCode::NoDevice),
            ..Probe::default()
        };
        let pulse_no_source = Probe {
            backend: Some("PulseAudio".into()),
            error: Some(ErrorCode::NoDevice),
            ..Probe::default()
        };
        let alsa_no_card = Probe {
            backend: Some("ALSA".into()),
            error: Some(ErrorCode::NoDevice),
            ..Probe::default()
        };
        use NoCaptureReason as R;
        let rows: Vec<(&str, Environment, Probe, R)> = vec![
            (
                "SSH variables",
                ssh_vars,
                no_backend(),
                R::Ssh { service_port: None },
            ),
            (
                "an sshd ancestor without variables (tmux)",
                ssh_ancestor,
                no_backend(),
                R::Ssh { service_port: None },
            ),
            (
                "a biorouter serve runs here",
                ssh_service,
                no_backend(),
                R::Ssh {
                    service_port: Some(9000),
                },
            ),
            (
                "SSH into a container is SSH",
                ssh_into_docker,
                no_backend(),
                R::Ssh { service_port: None },
            ),
            ("a container", docker, no_backend(), R::Container),
            ("WSL1", wsl1, no_backend(), R::Wsl1),
            ("WSL2 without WSLg", no_wslg, no_backend(), R::Wsl2NoWslg),
            (
                "WSL2 without libpulse",
                no_pulse,
                no_backend(),
                R::Wsl2NoPulse,
            ),
            (
                "WSL2 with WSLg and libpulse is other Linux",
                wslg,
                no_backend(),
                R::LinuxNoAudio,
            ),
            (
                "Linux with no sound system",
                linux(),
                no_backend(),
                R::LinuxNoAudio,
            ),
            (
                "Linux with ALSA and no card",
                linux(),
                alsa_no_card,
                R::LinuxNoAudio,
            ),
            (
                "Linux with a sound server and no source",
                linux(),
                pulse_no_source,
                R::NoDevice,
            ),
            (
                "a Mac with no microphone",
                local_mac(),
                no_device,
                R::NoDevice,
            ),
        ];
        for (name, env, probe, want) in rows {
            assert_eq!(no_capture_reason(&env, &probe), Some(want), "{name}");
            assert_eq!(
                detect(&env, &with_probe(probe)),
                CaptureVerdict::NoCapture(want),
                "{name}"
            );
        }
        assert_eq!(no_capture_reason(&linux(), &working()), None);
    }

    /// No model plus SSH gives MSG-SSH-ROUTES and exit 2, never the install question:
    /// step 7 comes before step 8.
    #[test]
    fn ssh_without_a_model_gets_the_routes_and_no_install_prompt() {
        let mut env = linux();
        env.ssh_variables = true;
        let verdict = detect(
            &env,
            &Findings {
                probe: Some(no_backend()),
                model: Some(false),
                ..found()
            },
        );
        assert_eq!(
            verdict,
            CaptureVerdict::NoCapture(NoCaptureReason::Ssh { service_port: None })
        );
        assert_eq!(verdict.exit_code(), 2);
        let lines = verdict.sentences(&env);
        assert_eq!(
            lines,
            vec![
                messages::MSG_LINUX_NOAUDIO.to_string(),
                messages::msg_ssh_routes("leo"),
                messages::msg_sync_hint("leo"),
            ]
        );
        assert!(!lines.iter().any(|l| l.contains("dictation install")));
        // With a `biorouter serve` running here, its port takes the routes' place.
        env.serve_port = Some(8765);
        let service = CaptureVerdict::NoCapture(NoCaptureReason::Ssh {
            service_port: Some(8765),
        })
        .sentences(&env);
        assert_eq!(service[1], messages::msg_ssh_service("8765", "leo"));
    }

    /// Stale SSH variables in a tmux pane of the desktop session do not refuse a Mac that
    /// can record; a real SSH session (not the desktop session) is refused at step 5.
    #[test]
    fn stale_tmux_variables_do_not_refuse_a_working_mac() {
        let mut stale = local_mac();
        stale.ssh_variables = true;
        stale.desktop_session = Some(true);
        assert!(!stale.mac_over_ssh());
        assert_eq!(detect(&stale, &found()), CaptureVerdict::Ready);
        let mut real = local_mac();
        real.ssh_variables = true;
        real.desktop_session = Some(false);
        assert_eq!(detect(&real, &found()), CaptureVerdict::MacOverSsh);
        let lines = CaptureVerdict::MacOverSsh.sentences(&real);
        assert_eq!(lines[0], messages::MSG_SSH_MAC);
        assert_eq!(lines[1], messages::msg_ssh_routes("lab-mac"));
        // Linux never refuses on SSH alone: the probe decides.
        let mut linux_ssh = linux();
        linux_ssh.ssh_variables = true;
        assert_eq!(detect(&linux_ssh, &found()), CaptureVerdict::Ready);
    }

    #[test]
    fn each_reason_has_its_sentences() {
        let env = linux();
        let say = |r: NoCaptureReason| CaptureVerdict::NoCapture(r).sentences(&env);
        assert_eq!(
            say(NoCaptureReason::Container),
            vec![messages::MSG_CONTAINER]
        );
        assert_eq!(say(NoCaptureReason::Wsl1), vec![messages::MSG_WSL1]);
        assert_eq!(
            say(NoCaptureReason::Wsl2NoWslg),
            vec![messages::MSG_WSL2_NOWSLG]
        );
        assert_eq!(
            say(NoCaptureReason::Wsl2NoPulse),
            vec![messages::MSG_WSL2_NOPULSE]
        );
        assert_eq!(
            say(NoCaptureReason::LinuxNoAudio),
            vec![messages::MSG_LINUX_NOAUDIO, messages::MSG_LINUX_ROUTES]
        );
        assert_eq!(
            say(NoCaptureReason::NoDevice),
            vec![messages::MSG_NO_DEVICE]
        );
        assert_eq!(
            CaptureVerdict::AgentShell.sentences(&env),
            vec![messages::MSG_AGENT_SHELL]
        );
        assert_eq!(
            CaptureVerdict::NoTerminal.sentences(&env),
            vec![messages::MSG_NO_TTY]
        );
        assert_eq!(
            CaptureVerdict::NotCompiled.sentences(&env),
            vec![biorouter::dictation::MSG_NO_ENGINE]
        );
    }

    #[test]
    fn the_zero_sentence_names_the_terminal_app() {
        let mut mac = local_mac();
        mac.terminal_app = terminal_app(Some("com.googlecode.iterm2".into()), None);
        assert_eq!(
            zero_sentence(&mac),
            messages::msg_tcc_zero("iTerm2", "com.googlecode.iterm2")
        );
        let windows = Environment {
            os: Some(Os::Windows),
            ..Environment::default()
        };
        assert_eq!(zero_sentence(&windows), messages::MSG_WIN_ZERO);
        assert_eq!(zero_sentence(&linux()), messages::MSG_SILENT);
    }

    #[test]
    fn the_terminal_app_comes_from_the_bundle_id_then_term_program() {
        assert_eq!(
            terminal_app(Some("com.apple.Terminal".into()), Some("vscode".into())),
            Some(TerminalApp {
                name: "Terminal".into(),
                bundle_id: Some("com.apple.Terminal".into())
            })
        );
        assert_eq!(
            terminal_app(Some("org.example.Term".into()), None)
                .unwrap()
                .name,
            "org.example.Term"
        );
        assert_eq!(
            terminal_app(None, Some("vscode".into()))
                .unwrap()
                .bundle_id
                .as_deref(),
            Some("com.microsoft.VSCode")
        );
        assert_eq!(terminal_app(None, None), None);
    }

    /// `/.dockerenv`, `/run/.containerenv` and `KUBERNETES_SERVICE_HOST` are read as they
    /// are; a Kubernetes pod or a Podman container is also named in `/proc/1/cgroup`.
    #[test]
    fn container_runtimes_in_the_cgroup() {
        assert!(names_a_container_runtime(
            "0::/kubepods/besteffort/pod1234/abcd"
        ));
        assert!(names_a_container_runtime("12:pids:/docker/0123abcd"));
        assert!(names_a_container_runtime(
            "0::/machine.slice/libpod-1.scope"
        ));
        assert!(!names_a_container_runtime("0::/user.slice/user-1000.slice"));
    }

    #[test]
    fn wsl_from_the_kernel_version() {
        let wsl2 = "Linux version 5.15.167.4-microsoft-standard-WSL2 (root@...) #1 SMP";
        let wsl1 =
            "Linux version 4.4.0-22621-Microsoft (Microsoft@Microsoft.com) (gcc version 5.4.0)";
        assert_eq!(
            wsl_from_version(wsl2, true, false),
            Some(Wsl::Wsl2 {
                wslg: true,
                libpulse: false
            })
        );
        assert_eq!(wsl_from_version(wsl1, false, false), Some(Wsl::Wsl1));
        assert_eq!(
            wsl_from_version("Linux version 6.8.0-45-generic", true, true),
            None
        );
    }

    #[test]
    fn a_serve_command_line_names_its_port() {
        let args = |line: &str| line.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(serve_port_of(&args("/usr/bin/biorouter serve")), Some(8765));
        assert_eq!(
            serve_port_of(&args("biorouter serve --host 0.0.0.0 --port 9001")),
            Some(9001)
        );
        assert_eq!(
            serve_port_of(&args("biorouter headless --port=7000")),
            Some(7000)
        );
        assert_eq!(serve_port_of(&args("biorouter session")), None);
        assert_eq!(serve_port_of(&args("/usr/bin/biorouterd agent")), None);
        assert_eq!(serve_port_of(&args("vim serve")), None);
    }

    /// The status reason a probe gives, as `scripts/check-dictation-status-json.py` reads
    /// it: `no_audio_system` in the package smoke containers.
    #[test]
    fn the_status_reason_of_a_probe() {
        assert_eq!(status_reason(&working()), None);
        assert_eq!(status_reason(&no_backend()), Some("no_audio_system"));
        assert_eq!(
            status_reason(&Probe {
                backend: Some("ALSA".into()),
                error: Some(ErrorCode::NoDevice),
                ..Probe::default()
            }),
            Some("no_audio_system")
        );
        assert_eq!(
            status_reason(&Probe {
                backend: Some("WASAPI".into()),
                error: Some(ErrorCode::NoDevice),
                ..Probe::default()
            }),
            Some("no_device")
        );
        assert_eq!(
            status_reason(&Probe {
                error: Some(ErrorCode::NotCompiled),
                ..Probe::default()
            }),
            Some("not_compiled")
        );
    }
}
