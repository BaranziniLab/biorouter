//! Stopping `biorouter apps serve` stops the daemon it started.
//!
//! It did not. The command handled `ctrl_c` and nothing else, so SIGTERM ran no
//! code here at all: the default action ended `apps serve` on the spot and left
//! `biorouterd` running, holding the port, serving the app, and holding the
//! daemon's secret. `kill <pid of apps serve>` — what a process manager, a
//! script or an editor's stop button sends — orphaned it every time.
//!
//! This is the defect PR #226 fixed for `biorouter serve`; these tests are the
//! same measurement, of the same two layers, against the same helpers the two
//! commands now share (`commands::serve::stop_daemon` and `--exit-with-parent`).
//! They run the real binaries and stop the command the way a process manager or
//! an operator does: by pid.
//!
//! ⚠ They need a `biorouterd` built from this tree beside the `biorouter` under
//! test. `cargo test -p biorouter-cli` does not build another package's binary,
//! so build it first:
//!
//! ```text
//! cargo build -p biorouter-cli -p biorouter-server
//! cargo test -p biorouter-cli --test apps_serve_lifecycle
//! ```
//!
//! Unix only: the second layer (`--exit-with-parent`) is Unix only, and there is
//! no SIGTERM to send on Windows.
#![cfg(unix)]

// Each `tests/*.rs` is its own crate, so the lib's `#[cfg(test)] mod
// test_sandbox;` is not compiled into this binary. Declare its own copy, so
// anything here that reaches a process-global cell resolves under a throwaway
// root rather than the developer's real config and session store. The children
// this file spawns pass their own `BIOROUTER_PATH_ROOT`, which still wins.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

#[path = "support/reserved_port.rs"]
mod reserved_port;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use reserved_port::ReservedPort;

/// How long the command may take to stop. Its grace for the daemon is ten
/// seconds; this leaves room for a loaded machine beyond that, so a pass means
/// "the daemon did not survive", not "it survived for less than N seconds".
const STOP_BUDGET: Duration = Duration::from_secs(30);

/// A debug daemon's cold start on a busy machine.
const READY_BUDGET: Duration = Duration::from_secs(120);

const APP_ID: &str = "lifecycle-app";

fn biorouter() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_biorouter"))
}

/// `apps serve` starts the daemon that sits beside it, so this is the one it
/// runs.
fn biorouterd() -> PathBuf {
    biorouter().with_file_name("biorouterd")
}

/// Refuse to run against a daemon these tests cannot be about, and warm its
/// first exec while we are here: a freshly linked binary's first run costs
/// seconds at almost no CPU, which would otherwise land inside the readiness
/// wait.
fn require_a_daemon_from_this_tree() {
    let daemon = biorouterd();
    assert!(
        daemon.is_file(),
        "{} does not exist. `cargo test -p biorouter-cli` does not build another \
         package's binary; run `cargo build -p biorouter-server --bin biorouterd` first.",
        daemon.display()
    );
    let help = Command::new(&daemon)
        .args(["agent", "--help"])
        .output()
        .expect("run biorouterd agent --help");
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("--exit-with-parent"),
        "{} predates --exit-with-parent, so it is older than this test. Rebuild it with \
         `cargo build -p biorouter-server --bin biorouterd`.",
        daemon.display()
    );
}

fn port_is_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// The status code of `GET path`, or `None` when nothing answered.
fn http_status(port: u16, path: &str) -> Option<u16> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(500),
    )
    .ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut head = [0u8; 64];
    let read = stream.read(&mut head).ok()?;
    String::from_utf8_lossy(&head[..read])
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Whether `pid` is a process that has not yet exited. A zombie has exited: it
/// is waiting to be reaped by a parent, and holds no port and no memory.
fn is_running(pid: u32) -> bool {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("run ps");
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

/// Start time and command line: together they name one process, where a pid
/// alone can be recycled for an unrelated one once the original has exited.
/// Empty once the process is gone.
fn identity(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "lstart=,command=", "-p", &pid.to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn signal(pid: u32, name: &str) {
    let status = Command::new("kill")
        .args([&format!("-{name}"), &pid.to_string()])
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -{name} {pid} failed");
}

fn wait_for(budget: Duration, mut done: impl FnMut() -> bool) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < budget {
        if done() {
            return Some(start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// A running `apps serve` and the daemon it started.
struct Served {
    serve: Child,
    daemon: u32,
    /// The daemon's [`identity`] when it was found, so cleanup can tell it from
    /// a later process that happens to reuse its pid.
    daemon_identity: String,
    port: u16,
    /// Keeps [`Self::port`] from being chosen by another test until the
    /// daemon on it has been stopped. Dropped after [`Drop::drop`] runs.
    _reserved: ReservedPort,
    root: tempfile::TempDir,
}

/// `biorouter apps <args>` against the store, state and port of a test root,
/// touching nothing of the developer's own configuration, sessions or keychain.
fn apps_command(root: &std::path::Path, port: u16, args: &[&str]) -> Command {
    let mut command = Command::new(biorouter());
    command
        .arg("apps")
        .args(args)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_PATH_ROOT", root.join("biorouter"))
        .env("BIOROUTER_PORT", port.to_string())
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        // The key of the daemon under test must come from its record, which is
        // what this file measures, never from the shell running the tests.
        .env_remove("BIOROUTER_SERVER__SECRET_KEY")
        .stdin(Stdio::null());
    command
}

impl Served {
    fn start() -> Self {
        require_a_daemon_from_this_tree();

        let root = tempfile::tempdir().expect("temp dir");
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // `apps serve` refuses an id the store does not hold, and the store is
        // `<BIOROUTER_PATH_ROOT>/config/agent_drafter`.
        let path_root = root.path().join("biorouter");
        let app = path_root.join("config").join("agent_drafter").join(APP_ID);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("manifest.json"),
            format!(r#"{{"id":"{APP_ID}","title":"Lifecycle","kind":"static","updated_at":1}}"#),
        )
        .unwrap();
        let log = std::fs::File::create(root.path().join("serve.log")).unwrap();

        let reserved = reserved_port::reserve();
        let port = reserved.port;
        let serve = apps_command(root.path(), port, &["serve", APP_ID])
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("spawn biorouter apps serve");

        let mut served = Self {
            serve,
            daemon: 0,
            daemon_identity: String::new(),
            port,
            _reserved: reserved,
            root,
        };
        let ready = wait_for(READY_BUDGET, || {
            http_status(port, "/status") == Some(200)
                || matches!(served.serve.try_wait(), Ok(Some(_)))
        });
        assert!(
            ready.is_some() && matches!(served.serve.try_wait(), Ok(None)),
            "apps serve did not come up on port {port}:\n{}",
            served.log()
        );
        served.daemon = served.only_child();
        served.daemon_identity = identity(served.daemon);
        assert!(
            served.daemon_identity.contains("biorouterd")
                && served.daemon_identity.contains("agent"),
            "the command's child is not the daemon: {:?}",
            served.daemon_identity
        );
        served
    }

    /// The daemon: the command's one child.
    fn only_child(&self) -> u32 {
        let out = Command::new("pgrep")
            .args(["-P", &self.serve.id().to_string()])
            .output()
            .expect("run pgrep");
        let pids: Vec<u32> = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect();
        assert_eq!(
            pids.len(),
            1,
            "expected apps serve to have exactly one child, the daemon: {pids:?}"
        );
        pids[0]
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.root.path().join("serve.log")).unwrap_or_default()
    }

    /// The record through which a later `apps open` or `apps serve` reaches
    /// this daemon's key.
    fn daemon_record(&self) -> PathBuf {
        self.root
            .path()
            .join("biorouter")
            .join("state")
            .join("apps-daemon")
            .join(format!("{}.json", self.port))
    }

    /// Stop the command with `name` and assert that it exits and takes the
    /// daemon with it.
    fn stop_with(mut self, name: &str) -> ExitStatus {
        signal(self.serve.id(), name);
        let mut status = None;
        let took = wait_for(STOP_BUDGET, || {
            status = self.serve.try_wait().ok().flatten();
            status.is_some()
        });
        let status = match (took, status) {
            (Some(took), Some(status)) => {
                eprintln!("apps serve exited {took:?} after SIG{name}");
                status
            }
            _ => panic!(
                "apps serve was still running {STOP_BUDGET:?} after SIG{name} (its daemon is \
                 pid {}):\n{}",
                self.daemon,
                self.log()
            ),
        };

        // The command reaps the daemon before it exits, so both of these hold
        // the moment it is gone. They are what an operator stopping it needs to
        // be true: nothing left running, and nothing on the port.
        assert!(
            !is_running(self.daemon),
            "the daemon (pid {}) outlived apps serve after SIG{name}:\n{}",
            self.daemon,
            self.log()
        );
        assert!(
            !port_is_open(self.port),
            "port {} is still accepting connections after apps serve exited on SIG{name}",
            self.port
        );
        status
    }
}

impl Drop for Served {
    /// A failed assertion must not leave a daemon running on the machine. The
    /// daemon is killed only while its pid still names the process that was
    /// found at startup, so a pid recycled for something unrelated is left
    /// alone.
    fn drop(&mut self) {
        let _ = self.serve.kill();
        let _ = self.serve.wait();
        if self.daemon != 0 && identity(self.daemon) == self.daemon_identity {
            let _ = Command::new("kill")
                .args(["-KILL", &self.daemon.to_string()])
                .status();
        }
    }
}

/// The measured defect: `apps serve` listened for `ctrl_c` alone, so SIGTERM ran
/// no code here and the daemon was left holding the port.
///
/// Fails the shipped command on both assertions: the daemon is still running,
/// and the port still answers.
#[test]
fn sigterm_to_apps_serve_stops_its_daemon() {
    let status = Served::start().stop_with("TERM");
    assert!(
        status.success(),
        "a requested stop is a clean exit: {status}"
    );
}

/// SIGINT was handled before this change, but with a `start_kill` and no grace.
/// It must still stop both, and now through the same bounded stop-then-kill
/// `biorouter serve` uses.
#[test]
fn sigint_to_apps_serve_stops_its_daemon() {
    let status = Served::start().stop_with("INT");
    assert!(
        status.success(),
        "a requested stop is a clean exit: {status}"
    );
}

/// SIGKILL runs no code in `apps serve` at all, so this is the second layer
/// alone: the daemon sees that its parent is gone and stops itself.
///
/// Fails the shipped command, which passed no `--exit-with-parent`, so the
/// orphan ran until something else killed it.
#[test]
fn a_daemon_whose_apps_serve_was_killed_outright_stops_itself() {
    let mut served = Served::start();
    let daemon = served.daemon;
    served.serve.kill().expect("SIGKILL apps serve");
    served.serve.wait().expect("reap apps serve");

    let took = wait_for(STOP_BUDGET, || !is_running(daemon));
    match took {
        Some(took) => eprintln!("the orphaned daemon stopped {took:?} after apps serve was killed"),
        None => panic!(
            "the daemon (pid {daemon}) was still running {STOP_BUDGET:?} after apps serve was \
             killed:\n{}",
            served.log()
        ),
    }
    assert!(
        !port_is_open(served.port),
        "port {} is still accepting connections after the orphaned daemon stopped",
        served.port
    );
}

/// The launch path in the first line of `output` that carries one.
fn launch_path(output: &str) -> Option<String> {
    let at = output.find(&format!("/apps/{APP_ID}/?t="))?;
    let path: String = output
        .get(at..)?
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || "/?=-_".contains(*c))
        .collect();
    let token = path.rsplit('=').next()?;
    (token.len() == 64).then_some(path)
}

/// The status of `GET path` and whether the answer set a cookie.
fn redeem(port: u16, path: &str) -> Option<(u16, bool)> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(500),
    )
    .ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).ok()?;
    let answer = String::from_utf8_lossy(&answer);
    let head = answer.split("\r\n\r\n").next()?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    let cookie = head.lines().any(|line| {
        line.to_ascii_lowercase()
            .starts_with("set-cookie: biorouter_app_")
    });
    Some((status, cookie))
}

/// W2-HRD-1, the dead end: `apps serve` printed one link that works once and
/// for five minutes, and "run this again for another". Its daemon's key was
/// kept to itself, so the second run found a daemon on the port it did not know
/// and refused it, and so did `apps open`, which the daemon's own refusal page
/// recommends. The only way on was to stop the first `apps serve`.
///
/// Fails the shipped command: the second run exits non-zero with "does not
/// know", and no record exists for it to read.
#[test]
fn a_second_apps_serve_mints_another_link_on_the_running_daemon() {
    let served = Served::start();
    let first = wait_for(READY_BUDGET, || launch_path(&served.log()).is_some())
        .and_then(|_| launch_path(&served.log()))
        .unwrap_or_else(|| panic!("apps serve printed no launch link:\n{}", served.log()));
    assert!(
        served
            .log()
            .contains(&format!("biorouter apps open {APP_ID}`")),
        "the advice under the link must name a command that works while this serves:\n{}",
        served.log()
    );
    let record = served.daemon_record();
    assert!(
        record.is_file(),
        "the daemon's key was not recorded at {}",
        record.display()
    );

    let second = apps_command(served.root.path(), served.port, &["serve", APP_ID])
        .output()
        .expect("run a second apps serve");
    let second_output = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        second.status.success(),
        "a second apps serve refused the running daemon ({}):\n{second_output}",
        second.status
    );
    let again = launch_path(&second_output)
        .unwrap_or_else(|| panic!("the second apps serve printed no link:\n{second_output}"));
    assert_ne!(first, again, "each run gets a link of its own");
    assert!(
        second_output.contains("reusing the daemon already running"),
        "{second_output}"
    );

    // Both links are this daemon's, and each works once.
    for link in [&again, &first] {
        assert_eq!(
            redeem(served.port, link),
            Some((200, true)),
            "{link} did not open the app"
        );
        let spent = redeem(served.port, link).expect("the daemon answers");
        assert!(
            spent != (200, true),
            "{link} opened the app twice: {spent:?}"
        );
    }
    assert_eq!(
        served.only_child(),
        served.daemon,
        "the second run must reuse the daemon, not start another"
    );

    // Stopping the command that started the daemon removes its record.
    let status = served.stop_with("TERM");
    assert!(
        status.success(),
        "a requested stop is a clean exit: {status}"
    );
    assert!(
        !record.exists(),
        "the record of a stopped daemon was left at {}",
        record.display()
    );
}
