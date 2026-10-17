//! Fixed, bounded facts about the server and the account a bridge runs as (design OW-8, OW-9):
//! the answers to `bridge.hello` and `bridge.probe`, and `biorouter-crew probe`, `doctor` and
//! `--version --output-format json`.
//!
//! Every check is a fixed file read, system call or argv with no model input, bounded to
//! [`PROBE_TIMEOUT`] and [`PROBE_OUTPUT`], and none reaches the network. It never reads
//! environment variables (commands are found by a login shell started with a fixed, minimal
//! environment), process lists, other people's homes, SSH configuration or shell history, so
//! what it reports may be shown to every member of a workspace as its first context document.
use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// What the member's own `biorouter-crew bridge` can do beyond relaying, as `bridge.hello`
/// reports it. An older bridge forwards `bridge.hello` to the broker, which refuses it: that
/// refusal is how a daemon tells an old bridge.
pub const BRIDGE_CAPABILITIES: [&str; 3] =
    ["remote_full_v1", "remote_transfer_v1", "bridge_probe_v1"];
/// The longest any one probe may run.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The most output kept from any one probe command.
pub const PROBE_OUTPUT: usize = 65_536;
/// At most this many filesystems are reported.
const MAX_STORAGE: usize = 12;
/// A filesystem that is not the home's, `/tmp`'s or a scratch area's is reported only from
/// this size, so a server's small system volumes do not crowd out its data storage.
const LARGE_FILESYSTEM_GIB: u64 = 100;
/// Filesystem types that hold data, as `/proc/self/mounts` names them.
const DATA_FILESYSTEMS: [&str; 18] = [
    "ext2",
    "ext3",
    "ext4",
    "xfs",
    "btrfs",
    "zfs",
    "nfs",
    "nfs4",
    "lustre",
    "gpfs",
    "beegfs",
    "cifs",
    "smb3",
    "ceph",
    "cephfs",
    "panfs",
    "wekafs",
    "fuse.glusterfs",
];
/// Commands the login-shell probe looks for, by name. Fixed: nothing a model or a member writes
/// reaches it.
const PROBED_COMMANDS: [&str; 20] = [
    "sbatch",
    "squeue",
    "sinfo",
    "srun",
    "qsub",
    "qstat",
    "qconf",
    "pbsnodes",
    "bsub",
    "condor_submit",
    "docker",
    "podman",
    "apptainer",
    "singularity",
    "nvidia-smi",
    "conda",
    "mamba",
    "python3",
    "R",
    "julia",
];
/// Where a conda installation is looked for, relative to the account's home or absolute.
const CONDA_PLACES: [&str; 10] = [
    "~/anaconda3",
    "~/miniconda3",
    "~/miniforge3",
    "~/mambaforge",
    "~/micromamba",
    "/opt/conda",
    "/opt/anaconda3",
    "/opt/miniconda3",
    "/usr/local/anaconda3",
    "/usr/local/miniconda3",
];
/// Files whose presence means environment modules (Lmod or Tcl modules) are installed.
const MODULE_MARKERS: [&str; 6] = [
    "/etc/profile.d/modules.sh",
    "/etc/profile.d/lmod.sh",
    "/etc/profile.d/z00_lmod.sh",
    "/usr/share/lmod",
    "/usr/share/Modules",
    "/opt/apps/lmod",
];

/// The account this process runs as, from the account database (never from `HOME`/`SHELL`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThisAccount {
    pub uid: u32,
    pub user: String,
    pub home: PathBuf,
    /// The login shell, or `None` when the database names none.
    pub shell: Option<PathBuf>,
}

/// `getpwuid_r(geteuid())`, with the buffer grown on `ERANGE`.
pub fn this_account() -> Result<ThisAccount> {
    let uid = unsafe { libc::geteuid() };
    let mut size = 16 * 1024;
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found = std::ptr::null_mut();
        let mut buffer = vec![0u8; size];
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if status == libc::ERANGE && size < 1024 * 1024 {
            size *= 4;
            continue;
        }
        ensure!(
            status == 0 && !found.is_null(),
            "identity_unavailable: This Unix account cannot be resolved."
        );
        // SAFETY: on success `found` points at `entry`, whose strings live in `buffer`.
        let entry = unsafe { &*found };
        let field = |pointer: *const libc::c_char| {
            (!pointer.is_null()).then(|| {
                unsafe { std::ffi::CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .trim()
                    .to_owned()
            })
        };
        let home = field(entry.pw_dir).filter(|home| home.starts_with('/'));
        return Ok(ThisAccount {
            uid,
            user: field(entry.pw_name).unwrap_or_default(),
            home: PathBuf::from(
                home.ok_or_else(|| anyhow!("identity_unavailable: This account has no home."))?,
            ),
            shell: field(entry.pw_shell)
                .filter(|shell| shell.starts_with('/'))
                .map(PathBuf::from),
        });
    }
}

/// The highest Landlock ABI the kernel supports, or 0 when it has none (or this is not Linux).
pub fn landlock_abi() -> u32 {
    #[cfg(target_os = "linux")]
    {
        const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<libc::c_void>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        u32::try_from(abi).unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// Whether the kernel offers seccomp filters, which confined commands also need.
pub fn seccomp_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        unsafe { libc::prctl(libc::PR_GET_SECCOMP) >= 0 }
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Whether folder-mode (confined) commands can run here: Landlock ABI 3 and seccomp.
pub fn confined_execution_supported() -> bool {
    landlock_abi() >= 3 && seccomp_available()
}

/// The answer to a bridge-local method (`bridge.*`), which the bridge gives itself and never
/// forwards.
pub fn bridge_method(method: &str) -> Result<Value> {
    match method {
        "bridge.hello" => bridge_hello(),
        "bridge.probe" => Ok(probe()),
        _ => Err(anyhow!(
            "unsupported: This Crew bridge has no method {}.",
            method.chars().take(64).collect::<String>()
        )),
    }
}

/// `bridge.hello`: this bridge's version and capabilities, whether this kernel can confine
/// commands, and the account it runs as.
pub fn bridge_hello() -> Result<Value> {
    let account = this_account()?;
    Ok(json!({
        "bridge_version": env!("CARGO_PKG_VERSION"),
        "capabilities": BRIDGE_CAPABILITIES,
        "landlock_abi": landlock_abi(),
        "confined_execution_supported": confined_execution_supported(),
        "user": account.user,
        "uid": account.uid,
        "home": account.home,
        "shell": account.shell,
    }))
}

/// Run `program` with `args` and a fixed `env` (nothing inherited), stdin `/dev/null`, in its
/// own process group, for at most [`PROBE_TIMEOUT`]; its standard output and error, each cut at
/// [`PROBE_OUTPUT`], or `None` when it could not start or ran out of time.
fn run_bounded(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Option<(String, String)> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let read = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                let _ = stream.take(PROBE_OUTPUT as u64).read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = read(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let stderr = read(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // The command has exited; a grandchild still holding its output would keep a reader
    // waiting, so the group goes too.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    Some((stdout.join().ok()?, stderr.join().ok()?))
}

/// A shell the probe may give a POSIX script with `-lc`: the account's login shell when it is
/// of the Bourne family, else `/bin/sh`.
fn posix_login_shell(account: &ThisAccount) -> PathBuf {
    let posix = account.shell.as_ref().filter(|shell| {
        let name = shell.file_name().and_then(|n| n.to_str()).unwrap_or("");
        matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh" | "mksh") && shell.exists()
    });
    posix.cloned().unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// The commands a fresh login of this account finds, by name, and whether it has the `module`
/// command. The login shell starts with only `HOME`, `USER`, `LOGNAME`, `SHELL`, a minimal
/// `PATH` and `LANG`, so what it reports is what the account's own startup files set up, and
/// nothing of this process's environment is read.
fn login_commands(account: &ThisAccount) -> (Vec<(String, String)>, bool) {
    let mut script = String::from("for c in");
    for name in PROBED_COMMANDS {
        script.push(' ');
        script.push_str(name);
    }
    script.push_str(
        "; do p=$(command -v \"$c\" 2>/dev/null) && printf '@crew-probe %s %s\\n' \"$c\" \"$p\"; done; \
         command -v module >/dev/null 2>&1 && printf '@crew-probe-modules yes\\n'; exit 0",
    );
    let home = account.home.to_string_lossy().into_owned();
    let shell = posix_login_shell(account);
    let shell_text = shell.to_string_lossy().into_owned();
    let env = [
        ("HOME", home.as_str()),
        ("USER", account.user.as_str()),
        ("LOGNAME", account.user.as_str()),
        ("SHELL", shell_text.as_str()),
        ("PATH", "/usr/local/bin:/usr/bin:/bin"),
        ("LANG", "C"),
    ];
    let Some((stdout, _)) = run_bounded(&shell, &["-lc", &script], &env) else {
        return (Vec::new(), false);
    };
    let mut found = Vec::new();
    let mut modules = false;
    for line in stdout.lines() {
        if line == "@crew-probe-modules yes" {
            modules = true;
        } else if let Some(rest) = line.strip_prefix("@crew-probe ") {
            if let Some((name, path)) = rest.split_once(' ') {
                // Only the names asked for, at absolute paths (a shell function or alias, such
                // as `conda` after `conda init`, is a name, not a program to run).
                if PROBED_COMMANDS.contains(&name) && path.starts_with('/') {
                    found.push((name.to_owned(), path.to_owned()));
                }
            }
        }
    }
    (found, modules)
}

/// The first version number (`3.10.12`) in a command's `--version` text.
fn version_in(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .find(|word| {
            word.chars().next().is_some_and(|c| c.is_ascii_digit())
                && word.contains('.')
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
        })
        .map(|word| word.chars().take(32).collect())
}

/// `path` as the probe reports it: `~/…` inside the account's home, else unchanged, and `None`
/// when it is inside another home.
fn shown_path(path: &Path, home: &Path) -> Option<String> {
    if let Ok(inside) = path.strip_prefix(home) {
        return Some(if inside.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", inside.display())
        });
    }
    (!in_another_home(path, home)).then(|| path.display().to_string())
}

/// Whether `path` is inside a home directory other than `home`: a sibling of `home` (or below
/// one), or anything below `/home` that is not `home` itself or one of its ancestors.
fn in_another_home(path: &Path, home: &Path) -> bool {
    if home.starts_with(path) {
        return false;
    }
    let sibling = home
        .parent()
        .filter(|parent| parent.components().count() > 1)
        .is_some_and(|parent| path.starts_with(parent) && !path.starts_with(home));
    sibling || (path.starts_with("/home") && path != Path::new("/home") && !path.starts_with(home))
}

/// One mount from `/proc/self/mounts`: where, and of what type.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mount {
    path: PathBuf,
    fs: String,
}

/// `/proc/self/mounts` text as mounts, with the octal escapes (`\040` for a space) undone.
fn parse_mounts(text: &str) -> Vec<Mount> {
    let unescape = |field: &str| {
        let bytes = field.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            let digits = bytes.get(index + 1..index + 4).unwrap_or_default();
            if bytes[index] == b'\\'
                && digits.len() == 3
                && digits.iter().all(|b| (b'0'..=b'7').contains(b))
            {
                let value = digits
                    .iter()
                    .fold(0u32, |sum, b| sum * 8 + u32::from(b - b'0'));
                if let Ok(value) = u8::try_from(value) {
                    out.push(value);
                    index += 4;
                    continue;
                }
            }
            out.push(bytes[index]);
            index += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let _source = fields.next()?;
            let path = unescape(fields.next()?);
            let fs = fields.next()?.to_owned();
            Some(Mount {
                path: PathBuf::from(path),
                fs,
            })
        })
        .collect()
}

/// The mount `path` is on: the one with the longest mount point that contains it.
fn mount_of<'m>(mounts: &'m [Mount], path: &Path) -> Option<&'m Mount> {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.path))
        .max_by_key(|mount| mount.path.components().count())
}

/// Size and free space of the filesystem at `path` in GiB, from `statvfs` on its own thread, so
/// a hung network mount holds up nobody: the answer arrives on the receiver, or never.
fn space_gib(path: &Path) -> std::sync::mpsc::Receiver<Option<(u64, u64)>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        let _ = sender.send(None);
        return receiver;
    };
    std::thread::spawn(move || {
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        let answer = (unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } == 0).then(|| {
            let info = unsafe { info.assume_init() };
            let unit = info.f_frsize;
            let gib = |blocks: u64| blocks.saturating_mul(unit) / (1 << 30);
            // Block counts are 64-bit on Linux and 32-bit on macOS, so the conversion is a
            // no-op on one of them.
            #[allow(clippy::useless_conversion)]
            let (blocks, available) = (u64::from(info.f_blocks), u64::from(info.f_bavail));
            (gib(blocks), gib(available))
        });
        let _ = sender.send(answer);
    });
    receiver
}

/// The filesystems worth naming: the home's, `/tmp`'s, any `/scratch*`, then other data
/// filesystems of at least [`LARGE_FILESYSTEM_GIB`], never one inside another person's home,
/// one entry per distinct filesystem, at most [`MAX_STORAGE`].
fn storage(mounts: &[Mount], home: &Path) -> Vec<Value> {
    let mut wanted: Vec<(PathBuf, bool)> = Vec::new();
    for path in [home.to_path_buf(), PathBuf::from("/tmp")] {
        if let Some(mount) = mount_of(mounts, &path) {
            wanted.push((mount.path.clone(), true));
        }
    }
    if let Ok(entries) = std::fs::read_dir("/") {
        let mut scratch: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("scratch"))
                    && path.is_dir()
            })
            .collect();
        scratch.sort();
        wanted.extend(scratch.into_iter().take(4).map(|path| (path, true)));
    }
    for mount in mounts {
        // System areas, and the single files a container bind-mounts under /etc.
        let system = [
            "/proc",
            "/sys",
            "/dev",
            "/run",
            "/boot",
            "/snap",
            "/etc",
            "/var/lib/docker",
            "/var/lib/containers",
        ]
        .iter()
        .any(|prefix| mount.path.starts_with(prefix));
        if DATA_FILESYSTEMS.contains(&mount.fs.as_str()) && !system {
            wanted.push((mount.path.clone(), false));
        }
    }
    // Every filesystem is asked at once and answers within one shared [`PROBE_TIMEOUT`].
    let asked: Vec<_> = wanted
        .into_iter()
        .filter(|(path, _)| !in_another_home(path, home))
        .take(48)
        .map(|(path, always)| {
            let answer = space_gib(&path);
            (path, always, answer)
        })
        .collect();
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut seen = std::collections::BTreeSet::new();
    let mut reported = Vec::new();
    for (path, always, answer) in asked {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(Some((size_gib, avail_gib))) = answer.recv_timeout(remaining) else {
            continue;
        };
        if reported.len() >= MAX_STORAGE {
            continue;
        }
        if !always && size_gib < LARGE_FILESYSTEM_GIB {
            continue;
        }
        let fs = mount_of(mounts, &path)
            .map(|mount| mount.fs.clone())
            .unwrap_or_default();
        // One pool reached through several mounts (ZFS datasets, bind mounts) is one entry.
        if !seen.insert((fs.clone(), size_gib, avail_gib)) {
            continue;
        }
        let Some(shown) = shown_path(&path, home) else {
            continue;
        };
        reported
            .push(json!({"path": shown, "fs": fs, "size_gib": size_gib, "avail_gib": avail_gib}));
    }
    reported
}

fn os_release() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|value| value.trim().trim_matches('"').to_owned())
    };
    value("PRETTY_NAME")
        .or_else(|| value("NAME"))
        .map(|name| name.chars().take(128).collect())
}

fn uname() -> (Option<String>, Option<String>) {
    let mut name = std::mem::MaybeUninit::<libc::utsname>::uninit();
    if unsafe { libc::uname(name.as_mut_ptr()) } != 0 {
        return (None, None);
    }
    let name = unsafe { name.assume_init() };
    let field = |chars: &[libc::c_char]| {
        unsafe { std::ffi::CStr::from_ptr(chars.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    };
    (Some(field(&name.release)), Some(field(&name.machine)))
}

fn hostname() -> Option<String> {
    let mut buffer = [0u8; 256];
    (unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0).then(|| {
        let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[..end]).into_owned()
    })
}

fn glibc_version() -> Option<String> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let version = unsafe { libc::gnu_get_libc_version() };
        (!version.is_null()).then(|| {
            unsafe { std::ffi::CStr::from_ptr(version) }
                .to_string_lossy()
                .into_owned()
        })
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

fn memory_gib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kib: u64 = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((kib + (1 << 19)) / (1 << 20))
}

/// Whether this process is in the group named `name` (the groups its login gave it).
fn in_group(name: &str) -> bool {
    let Ok(name) = std::ffi::CString::new(name) else {
        return false;
    };
    let mut entry = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 64 * 1024];
    let status = unsafe {
        libc::getgrnam_r(
            name.as_ptr(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return false;
    }
    let gid = unsafe { (*found).gr_gid };
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    let mut groups = vec![0 as libc::gid_t; count.max(0) as usize];
    let count = unsafe { libc::getgroups(groups.len() as libc::c_int, groups.as_mut_ptr()) };
    groups.truncate(count.max(0) as usize);
    let primary = unsafe { libc::getegid() };
    primary == gid || groups.contains(&gid)
}

/// GPUs `nvidia-smi` reports: `{name, memory_mib}` each.
fn gpus(nvidia_smi: Option<&str>) -> Vec<Value> {
    let Some(program) = nvidia_smi else {
        return Vec::new();
    };
    let Some((stdout, _)) = run_bounded(
        Path::new(program),
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
        &[("PATH", "/usr/bin:/bin"), ("LANG", "C")],
    ) else {
        return Vec::new();
    };
    stdout
        .lines()
        .filter_map(|line| {
            let (name, memory) = line.rsplit_once(',')?;
            Some(json!({"name": name.trim().chars().take(128).collect::<String>(), "memory_mib": memory.trim().parse::<u64>().ok()}))
        })
        .take(32)
        .collect()
}

/// The scheduler the found commands belong to.
fn scheduler_kind(found: &[&str]) -> Option<&'static str> {
    let has = |name: &str| found.contains(&name);
    if has("sbatch") {
        Some("slurm")
    } else if has("bsub") {
        Some("lsf")
    } else if has("condor_submit") {
        Some("htcondor")
    } else if has("qconf") {
        Some("sge")
    } else if has("qsub") {
        Some("pbs")
    } else {
        None
    }
}

/// The server facts `bridge.probe` and `biorouter-crew probe` report (design, "Server probe
/// and context draft"). A fact that cannot be learned is `null` or empty, never an error.
pub fn probe() -> Value {
    let account = this_account().ok();
    let home = account.as_ref().map(|a| a.home.clone()).unwrap_or_default();
    let (kernel, arch) = uname();
    let (found, login_modules) = account.as_ref().map(login_commands).unwrap_or_default();
    let path_of = |name: &str| {
        found
            .iter()
            .find(|(found, _)| found == name)
            .map(|(_, path)| path.as_str())
    };
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    // Version commands run side by side, so the probe as a whole stays near two timeouts.
    let versions: Vec<(&str, std::thread::JoinHandle<Option<String>>)> = [
        ("python3", "--version"),
        ("R", "--version"),
        ("julia", "--version"),
    ]
    .into_iter()
    .filter_map(|(name, flag)| {
        let program = path_of(name)?.to_owned();
        let home = home.to_string_lossy().into_owned();
        Some((
            name,
            std::thread::spawn(move || {
                let directory = Path::new(&program)
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let path = format!("{directory}:/usr/bin:/bin");
                let (stdout, stderr) = run_bounded(
                    Path::new(&program),
                    &[flag],
                    &[
                        ("PATH", path.as_str()),
                        ("HOME", home.as_str()),
                        ("LANG", "C"),
                    ],
                )?;
                version_in(&stdout).or_else(|| version_in(&stderr))
            }),
        ))
    })
    .collect();
    let gpus_thread = {
        let program = path_of("nvidia-smi").map(str::to_owned);
        std::thread::spawn(move || gpus(program.as_deref()))
    };
    let mounts = std::fs::read_to_string("/proc/self/mounts")
        .map(|text| parse_mounts(&text))
        .unwrap_or_default();
    let storage = if home.as_os_str().is_empty() {
        Vec::new()
    } else {
        storage(&mounts, &home)
    };
    let mut conda: Vec<String> = Vec::new();
    for place in CONDA_PLACES {
        let path = match place.strip_prefix("~/") {
            Some(inside) => home.join(inside),
            None => PathBuf::from(place),
        };
        if path.join("bin").join("conda").exists() || path.join("bin").join("micromamba").exists() {
            conda.push(place.to_owned());
        }
    }
    for name in ["conda", "mamba"] {
        let prefix = path_of(name)
            .and_then(|path| Path::new(path).parent()?.parent().map(Path::to_path_buf));
        if let Some(shown) = prefix.and_then(|prefix| shown_path(&prefix, &home)) {
            if !conda.contains(&shown) {
                conda.push(shown);
            }
        }
    }
    let modules = login_modules
        || MODULE_MARKERS
            .iter()
            .any(|marker| Path::new(marker).exists());
    let scheduler_commands: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| {
            matches!(
                *name,
                "sbatch"
                    | "squeue"
                    | "sinfo"
                    | "srun"
                    | "qsub"
                    | "qstat"
                    | "qconf"
                    | "pbsnodes"
                    | "bsub"
                    | "condor_submit"
            )
        })
        .collect();
    let mut languages = serde_json::Map::new();
    for (name, thread) in versions {
        if let Ok(Some(version)) = thread.join() {
            languages.insert(name.to_owned(), json!(version));
        }
    }
    json!({
        "schema": 1,
        "host": hostname(),
        "os": os_release(),
        "kernel": kernel,
        "arch": arch,
        "glibc": glibc_version(),
        "cpus": unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) }.max(0),
        "mem_gib": memory_gib(),
        "gpus": gpus_thread.join().unwrap_or_default(),
        "scheduler": {"kind": scheduler_kind(&names), "commands": scheduler_commands},
        "modules": modules,
        "conda": conda,
        "containers": {
            "docker": names.contains(&"docker"),
            "docker_group": in_group("docker"),
            "apptainer": names.contains(&"apptainer") || names.contains(&"singularity"),
            "podman": names.contains(&"podman"),
        },
        "languages": languages,
        "storage": storage,
        "landlock_abi": landlock_abi(),
        "user": account.as_ref().map(|a| a.user.clone()),
        "home": account.as_ref().map(|a| a.home.clone()),
    })
}

/// Why a broker cannot host a workspace in this account, each a code, mirroring the checks the
/// broker itself makes when it starts: `linux_required`, `pidfd_unavailable` (a safe `stop`
/// needs `pidfd_open`), `machine_id_unavailable` (node identity), `state_on_network_filesystem`
/// and `home_writable_by_others` (the state directory's ancestors).
pub fn hosting_blockers(account: &ThisAccount) -> Vec<&'static str> {
    let mut blockers = Vec::new();
    if !cfg!(target_os = "linux") {
        blockers.push("linux_required");
        return blockers;
    }
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid(), 0) };
        if fd < 0 {
            blockers.push("pidfd_unavailable");
        } else {
            unsafe { libc::close(fd as libc::c_int) };
        }
    }
    if !machine_id_usable() {
        blockers.push("machine_id_unavailable");
    }
    let state = account.home.join(".local/share/biorouter-crew");
    let existing = state
        .ancestors()
        .find(|path| path.exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    if on_network_filesystem(&existing) {
        blockers.push("state_on_network_filesystem");
    }
    let canonical = std::fs::canonicalize(&existing).unwrap_or(existing);
    let writable = canonical.ancestors().any(|ancestor| {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(ancestor)
            .map(|m| m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0)
            .unwrap_or(false)
    });
    if writable {
        blockers.push("home_writable_by_others");
    }
    blockers
}

/// The broker's node identity rule (`node_identity` in `broker.rs`): the first of
/// `/etc/machine-id` and `/var/lib/dbus/machine-id` that exists is a root-owned regular file
/// nobody else may write, holding 32 hex digits that are not all zero.
fn machine_id_usable() -> bool {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return false,
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        if !(metadata.is_file()
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0
            && metadata.len() <= 128)
        {
            return false;
        }
        let mut value = String::new();
        if file.take(129).read_to_string(&mut value).is_err() {
            return false;
        }
        let value = value.trim();
        return value.len() == 32
            && value.bytes().all(|b| b.is_ascii_hexdigit())
            && value.bytes().any(|b| b != b'0');
    }
    false
}

/// The broker's network filesystem rule: NFS and SMB/CIFS are refused for workspace state.
fn on_network_filesystem(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
            return false;
        };
        let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return false;
        }
        let kind = unsafe { info.assume_init() }.f_type;
        kind == 0x6969 || kind == 0xff534d42_u64 as libc::c_long || kind == 0x517B
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

/// What a workspace's agents cannot do on this server, each a code: `landlock_abi_below_3` and
/// `seccomp_unavailable` mean folder-mode (confined) commands cannot run; full access still can.
pub fn execution_limits() -> Vec<&'static str> {
    let mut limits = Vec::new();
    if landlock_abi() < 3 {
        limits.push("landlock_abi_below_3");
    }
    if !seccomp_available() {
        limits.push("seccomp_unavailable");
    }
    limits
}

/// `biorouter-crew doctor --output-format json`: the probe, this binary's version, and what
/// blocks hosting or limits agents here.
pub fn doctor() -> Value {
    let mut facts = probe();
    facts["version"] = json!(env!("CARGO_PKG_VERSION"));
    facts["hosting_blockers"] = match this_account() {
        Ok(account) => json!(hosting_blockers(&account)),
        Err(_) => json!(["identity_unavailable"]),
    };
    facts["execution_limits"] = json!(execution_limits());
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_numbers_are_found_in_version_text() {
        assert_eq!(version_in("Python 3.10.12\n").as_deref(), Some("3.10.12"));
        assert_eq!(
            version_in("R version 4.1.2 (2021-11-01) -- \"Bird Hippie\"").as_deref(),
            Some("4.1.2")
        );
        assert_eq!(version_in("julia version 1.9.3").as_deref(), Some("1.9.3"));
        assert_eq!(version_in("no version here"), None);
    }

    #[test]
    fn mounts_are_parsed_with_their_escapes() {
        let mounts = parse_mounts(
            "pool1/home /pool1/home zfs rw 0 0\n/dev/sda1 /mnt/my\\040data ext4 rw 0 0\nproc /proc proc rw 0 0\n",
        );
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[1].path, PathBuf::from("/mnt/my data"));
        assert_eq!(mounts[1].fs, "ext4");
        let home = mount_of(&mounts, Path::new("/pool1/home/wanjun/project")).unwrap();
        assert_eq!(home.path, PathBuf::from("/pool1/home"));
    }

    /// Another person's home is never named, whatever spelling it takes; the account's own home
    /// and the directories above it are.
    #[test]
    fn other_peoples_homes_are_never_reported() {
        let home = Path::new("/pool1/home/wanjun");
        assert!(in_another_home(Path::new("/pool1/home/alice"), home));
        assert!(in_another_home(
            Path::new("/pool1/home/alice/anaconda3"),
            home
        ));
        assert!(in_another_home(Path::new("/home/bob"), home));
        assert!(!in_another_home(Path::new("/pool1/home"), home));
        assert!(!in_another_home(Path::new("/pool1/home/wanjun/data"), home));
        assert!(!in_another_home(Path::new("/opt/conda"), home));
        assert_eq!(
            shown_path(Path::new("/pool1/home/wanjun/anaconda3"), home).as_deref(),
            Some("~/anaconda3")
        );
        assert_eq!(shown_path(Path::new("/pool1/home/alice/x"), home), None);
        assert_eq!(
            shown_path(Path::new("/opt/conda"), home).as_deref(),
            Some("/opt/conda")
        );
    }

    #[test]
    fn the_scheduler_is_named_by_its_commands() {
        assert_eq!(scheduler_kind(&["sbatch", "squeue"]), Some("slurm"));
        assert_eq!(scheduler_kind(&["qsub", "qconf"]), Some("sge"));
        assert_eq!(scheduler_kind(&["qsub", "qstat"]), Some("pbs"));
        assert_eq!(scheduler_kind(&["bsub"]), Some("lsf"));
        assert_eq!(scheduler_kind(&["docker"]), None);
    }

    /// The facts have the shape the context draft reads, with every key present.
    #[test]
    fn the_probe_has_the_contract_shape() {
        let facts = probe();
        for key in [
            "schema",
            "host",
            "os",
            "kernel",
            "arch",
            "glibc",
            "cpus",
            "mem_gib",
            "gpus",
            "scheduler",
            "modules",
            "conda",
            "containers",
            "languages",
            "storage",
            "landlock_abi",
            "user",
            "home",
        ] {
            assert!(facts.get(key).is_some(), "missing {key}: {facts}");
        }
        assert_eq!(facts["schema"], 1);
        assert!(facts["gpus"].is_array() && facts["conda"].is_array());
        assert!(facts["storage"].is_array());
        assert!(facts["scheduler"]["commands"].is_array());
        for key in ["docker", "docker_group", "apptainer", "podman"] {
            assert!(facts["containers"][key].is_boolean(), "{key}");
        }
        assert!(facts["modules"].is_boolean());
        assert!(serde_json::to_vec(&facts).unwrap().len() < 64 * 1024);
        let doctor = doctor();
        assert!(doctor["hosting_blockers"].is_array());
        assert!(doctor["execution_limits"].is_array());
        assert_eq!(doctor["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn bridge_hello_describes_this_bridge_and_account() {
        let hello = bridge_method("bridge.hello").unwrap();
        assert_eq!(hello["bridge_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            hello["capabilities"],
            json!(["remote_full_v1", "remote_transfer_v1", "bridge_probe_v1"])
        );
        assert_eq!(hello["uid"], unsafe { libc::geteuid() });
        assert!(hello["home"]
            .as_str()
            .is_some_and(|home| home.starts_with('/')));
        assert_eq!(
            hello["confined_execution_supported"],
            landlock_abi() >= 3 && seccomp_available()
        );
        let error = bridge_method("bridge.anything").unwrap_err().to_string();
        assert!(error.starts_with("unsupported: "), "{error}");
    }
}
