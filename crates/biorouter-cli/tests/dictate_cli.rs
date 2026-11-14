//! `biorouter dictate`, `biorouter dictation devices` and `test`, and the capture child,
//! through the real binary (PR-25; `testing-and-release.md`, "CLI capture and front ends").
//! Offline, and no test opens a real microphone: the ones that record use the debug
//! build's test source (`BIOROUTER_DICTATION_CAPTURE_TEST_SOURCE`), and the ones that need
//! a terminal run `biorouter` on a pseudo terminal.
//!
//! * the capture child answers a probe without opening anything, never with the Null
//!   backend, and with no `ALSA lib` line on its stderr, even when ALSA's configuration
//!   fails (Linux); frames round trip from `ready` to `stopped` with every sample counted;
//!   stdin's end of file ends it with `stopped` and exit 0; a killed child leaves nothing
//!   behind for the next; it refuses `start` in an agent's shell;
//! * `dictate` refuses without a terminal (MSG-NO-TTY) and in an agent's shell
//!   (MSG-AGENT-SHELL), both exit 2; `--private-terms` is refused with
//!   MSG-PRIVATE-TERMS-REFUSED without a terminal, in an agent's shell and with no private
//!   model configured, and asks MSG-PRIVATE-TERMS, then goes on, where it is admitted;
//!   `--copy --no-history` is refused where WSL cannot honour it (exit 2);
//! * over SSH with no microphone and no model: the routes, exit 2, and no install prompt;
//!   with this account's `biorouter serve` running (Linux), MSG-SSH-SERVICE with its port;
//!   locally with no model: MSG-NO-MODEL-CLI, exit 2;
//! * exact zeros are the permission sentence (exit 3), Escape cancels (exit 4), and
//!   `--seconds` stops the recording by itself;
//! * with the debug build's fixed transcript (`BIOROUTER_DICTATION_TEST_TEXT`, since the
//!   for-tests model hears nothing): the review with stdout piped, as `"$(biorouter
//!   dictate)"` runs it (Enter uses, `e` edits, Escape discards, nothing on stdout before
//!   the person decides), `--copy` with the exact text reaching a test clipboard (exit 0,
//!   stdout empty), and `--copy` with no clipboard (MSG-NO-CLIPBOARD, the text on stdout,
//!   exit 5);
//! * `dictation devices` and `status` report the probe, `no_audio_system` where there is no
//!   audio system, as the package smoke containers do; `dictation test` needs a terminal.
//!
//! Ignored, by hand: a real microphone delivers frames within 2 % of wall time and its first
//! frame within 300 ms, and a killed child leaves the device to the next at once (M-7); and
//! with the real catalog models (`BIOROUTER_DICTATION_MODELS_DIR`) the review (Enter, `e`,
//! Escape) and the latency from stop to text.

#![cfg(feature = "dictation")]

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before
// `main`, as every test binary here does.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::io::Read;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use biorouter_cli::dictation::messages;
use serde_json::Value;

const TEST_SOURCE: &str = "BIOROUTER_DICTATION_CAPTURE_TEST_SOURCE";
/// A debug build's fixed transcript, in place of the engine's.
const TEST_TEXT: &str = biorouter_cli::dictation::controller::TEST_TEXT_ENV;
/// A debug build's stand in for the clipboard.
const TEST_CLIPBOARD: &str = biorouter_cli::dictation::clipboard::TEST_CLIPBOARD_ENV;
/// What the fixed transcript says.
#[cfg(unix)]
const SAID: &str = "the patient's EPAS1 levels rose";

fn biorouter() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_biorouter"))
}

#[cfg(unix)]
fn vendored(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

/// A fresh Biorouter root and HOME.
fn sandbox() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    root
}

/// The environment every run gets: the sandbox, and none of the caller's dictation or
/// SSH variables.
fn base(command: &mut Command, root: &Path) {
    command
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env("NO_COLOR", "1")
        .env_remove("BIOROUTER_TERMINAL")
        .env_remove("BIOROUTER_DICTATION_MODELS_DIR")
        .env_remove("BIOROUTER_DICTATION_TEST_MODEL")
        .env_remove("BIOROUTER_DICTATION_WORKER")
        .env_remove("BIOROUTER_CLI_DICTATION_DEVICE")
        .env_remove(TEST_SOURCE)
        .env_remove(TEST_TEXT)
        .env_remove(TEST_CLIPBOARD)
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_CLIENT")
        .env_remove("SSH_TTY");
}

/// `biorouter <args>` with stdin closed: no terminal.
fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(biorouter());
    command.args(args).stdin(Stdio::null());
    base(&mut command, root);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("run biorouter")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}{}", stdout(output), stderr(output)))
}

fn assert_refused(output: &Output, sentence: &str) {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}{}",
        stdout(output),
        stderr(output)
    );
    assert_eq!(
        stderr(output).trim(),
        sentence,
        "stdout: {}",
        stdout(output)
    );
}

/// When a key is typed: after a delay, or once the terminal shows a text (and a moment more,
/// so the command is reading keys), whatever the machine's load.
#[cfg(unix)]
enum Typed {
    After(Duration, &'static [u8]),
    When(&'static str, &'static [u8]),
    /// A held key: the bytes again and again, every 40 ms, until [`HELD_AFTER`] after the
    /// terminal shows a text, as a person lets go of a key once they see something.
    HeldUntil(&'static str, &'static [u8]),
}

/// How long a held key goes on after what the person waits for appears: a quick reaction,
/// and well inside the review's 300 ms grace on a loaded machine.
#[cfg(unix)]
const HELD_AFTER: Duration = Duration::from_millis(100);

/// `biorouter <args>` with stdin and stderr on a pseudo terminal, so `prompt_can_run()` is
/// true as for a person at a terminal; stdout stays a pipe. `keys` are typed into the
/// terminal in order. What the command drew on the terminal is returned as `stderr`.
#[cfg(unix)]
fn at_a_terminal(root: &Path, args: &[&str], env: &[(&str, &str)], keys: Vec<Typed>) -> Output {
    at_a_terminal_without(root, args, env, &[], keys)
}

/// [`at_a_terminal`], with the variables in `remove` taken out of the environment.
#[cfg(unix)]
fn at_a_terminal_without(
    root: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    remove: &[&str],
    keys: Vec<Typed>,
) -> Output {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::sync::{Arc, Mutex};

    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty writes two new descriptors into `master` and `slave`; the name, the
    // terminal settings and the window size are not asked for.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    // SAFETY: both descriptors are open and owned by nothing else.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let mut command = Command::new(biorouter());
    command
        .args(args)
        .stdin(Stdio::from(slave.try_clone().expect("a second descriptor")))
        .stderr(Stdio::from(slave))
        .stdout(Stdio::piped());
    base(&mut command, root);
    // This PTY acts as a supported terminal even when the test runner has TERM=dumb.
    command.env("TERM", "xterm");
    for key in remove {
        command.env_remove(key);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    let child = command.spawn().expect("run biorouter");
    drop(command);
    let terminal = std::fs::File::from(master);
    let mut typing = terminal.try_clone().expect("the terminal's other end");
    let written = Arc::new(Mutex::new(Vec::new()));
    let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let typist = {
        let written = Arc::clone(&written);
        let exited = Arc::clone(&exited);
        std::thread::spawn(move || {
            for key in keys {
                let bytes = match key {
                    Typed::After(delay, bytes) => {
                        std::thread::sleep(delay);
                        bytes
                    }
                    Typed::When(text, bytes) => {
                        let deadline = Instant::now() + Duration::from_secs(90);
                        loop {
                            let seen =
                                String::from_utf8_lossy(&written.lock().unwrap()).contains(text);
                            if seen {
                                break;
                            }
                            if exited.load(std::sync::atomic::Ordering::SeqCst)
                                || Instant::now() > deadline
                            {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        std::thread::sleep(Duration::from_millis(300));
                        bytes
                    }
                    Typed::HeldUntil(text, bytes) => {
                        let deadline = Instant::now() + Duration::from_secs(90);
                        let mut seen_at: Option<Instant> = None;
                        while seen_at.is_none_or(|at| at.elapsed() < HELD_AFTER) {
                            if exited.load(std::sync::atomic::Ordering::SeqCst)
                                || Instant::now() > deadline
                                || typing.write_all(bytes).is_err()
                            {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(40));
                            if seen_at.is_none()
                                && String::from_utf8_lossy(&written.lock().unwrap()).contains(text)
                            {
                                seen_at = Some(Instant::now());
                            }
                        }
                        b""
                    }
                };
                if typing.write_all(bytes).is_err() {
                    return;
                }
            }
        })
    };
    let reader = {
        let written = Arc::clone(&written);
        let mut terminal = terminal;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = terminal.read(&mut buf) {
                if n == 0 {
                    break;
                }
                written.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        })
    };
    let mut output = child.wait_with_output().expect("biorouter ends");
    exited.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = typist.join();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !reader.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    output.stderr = written.lock().unwrap().clone();
    output
}

/// The terminal's text with the line drawing removed: carriage returns and the erase
/// sequence the status line uses.
#[cfg(unix)]
fn plain(terminal: &str) -> String {
    terminal.replace("\x1b[2K", "").replace('\r', "")
}

#[cfg(unix)]
fn for_tests_model() -> PathBuf {
    vendored("models/for-tests-ggml-base.en.bin")
}

#[cfg(unix)]
fn can_dictate() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

// ── the capture child ──────────────────────────────────────────────────────────────

use biorouter_dictation::protocol::{read_frame_blocking, to_json, write_frame_blocking};
use biorouter_dictation_capture::{samples_of, FromCapture, ToCapture};

/// The capture child, its stdin and stdout piped and its stderr kept.
fn capture_child(root: &Path, env: &[(&str, &str)]) -> std::process::Child {
    let mut command = Command::new(biorouter());
    command
        .args(["__br-dictation-worker", "capture"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    base(&mut command, root);
    for (key, value) in env {
        command.env(key, value);
    }
    command.spawn().expect("start the capture child")
}

fn send(child: &mut std::process::Child, message: &ToCapture) {
    let stdin = child.stdin.as_mut().expect("stdin");
    write_frame_blocking(stdin, &to_json(message), &[]).expect("a frame to the child");
}

fn receive(stdout: &mut impl Read) -> Option<(FromCapture, Vec<u8>)> {
    let frame = read_frame_blocking(stdout).ok()?;
    Some((
        serde_json::from_slice(&frame.json).expect("a message"),
        frame.payload,
    ))
}

/// A probe and nothing opened: the first frame, then exit 0 when stdin closes. Whatever
/// answers is never the Null backend, and ALSA's complaints stay out of stderr.
#[test]
fn the_capture_child_probes_without_opening_anything() {
    let root = sandbox();
    let mut command = Command::new(biorouter());
    command
        .args(["__br-dictation-worker", "capture"])
        .stdin(Stdio::null());
    base(&mut command, root.path());
    let output = command.output().expect("run the capture child");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let mut out = std::io::Cursor::new(output.stdout.clone());
    let (message, _) = receive(&mut out).expect("a probe");
    let FromCapture::Probe(probe) = message else {
        panic!("the first frame is {message:?}");
    };
    assert_ne!(probe.backend.as_deref(), Some("Null"), "{probe:?}");
    assert!(!probe.compiled.iter().any(|b| b == "Null"), "{probe:?}");
    assert!(!probe.test_source);
    assert!(
        receive(&mut out).is_none(),
        "nothing after the probe without start"
    );
    assert!(!stderr(&output).contains("ALSA lib"), "{}", stderr(&output));
}

/// `ready`, then a level before each audio frame, then on `stop` the rest and `stopped`,
/// whose count is every sample sent; and the samples keep pace with the wall clock.
#[test]
fn frames_round_trip_through_the_capture_child() {
    if !cfg!(debug_assertions) {
        return;
    }
    let root = sandbox();
    let mut child = capture_child(root.path(), &[(TEST_SOURCE, "tone")]);
    let mut out = child.stdout.take().unwrap();
    assert!(matches!(receive(&mut out), Some((FromCapture::Probe(p), _)) if p.test_source));
    let asked = Instant::now();
    send(&mut child, &ToCapture::Start { device: None });
    let Some((FromCapture::Ready(ready), _)) = receive(&mut out) else {
        panic!("no ready");
    };
    assert_eq!(ready.rate, 16_000);
    let mut samples = 0u64;
    let mut first_audio = None;
    let mut levels = 0;
    while samples < 8_000 {
        match receive(&mut out).expect("a frame") {
            (FromCapture::Level(level), _) => {
                levels += 1;
                assert!(level.peak > 0.2, "{level:?}");
            }
            (FromCapture::Audio, payload) => {
                first_audio.get_or_insert(asked.elapsed());
                samples += samples_of(&payload).len() as u64;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    send(&mut child, &ToCapture::Stop);
    let stopped = loop {
        match receive(&mut out).expect("a frame") {
            (FromCapture::Audio, payload) => samples += samples_of(&payload).len() as u64,
            (FromCapture::Stopped { frames }, _) => break frames,
            (FromCapture::Level(_), _) => levels += 1,
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(stopped, samples);
    assert!(levels >= 5, "{levels}");
    let first = first_audio.unwrap();
    assert!(
        first < Duration::from_millis(300),
        "first audio after {first:?}"
    );
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// How long a wait for a child may take before it counts as a hang. It bounds a hang and
/// is not a speed claim: a busy CI runner stalls a process for seconds, so a test asserts
/// why a child ended (its exit code, its last frame), not how fast.
const HANG_GUARD: Duration = Duration::from_secs(30);

/// Stdin's end of file ends a recording child cleanly, and is what ends it: the child is
/// still recording when stdin closes, its last frame is `stopped` (no `stop` was sent),
/// and it exits 0. A kill or a crash by a signal has no exit code and writes no
/// `stopped`. There is no wall clock bound: one held this to 2 s from the close, which on
/// a busy runner measures the load rather than the end of file path. The wait is held to
/// [`HANG_GUARD`].
#[test]
fn end_of_file_ends_the_capture_child_cleanly() {
    if !cfg!(debug_assertions) {
        return;
    }
    let root = sandbox();
    let mut child = capture_child(root.path(), &[(TEST_SOURCE, "tone")]);
    let mut out = child.stdout.take().unwrap();
    receive(&mut out).expect("a probe");
    send(&mut child, &ToCapture::Start { device: None });
    assert!(matches!(
        receive(&mut out),
        Some((FromCapture::Ready(_), _))
    ));
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the recording child exited before its stdin closed"
    );
    drop(child.stdin.take());
    let reader = std::thread::spawn(move || {
        let mut last = None;
        while let Some((message, _)) = receive(&mut out) {
            last = Some(message);
        }
        last
    });
    let deadline = Instant::now() + HANG_GUARD;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the capture child did not exit within {HANG_GUARD:?} of end of file");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let last = reader.join().unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(
        matches!(last, Some(FromCapture::Stopped { .. })),
        "{last:?}"
    );
}

/// A child killed while recording leaves nothing that stops the next one from starting and
/// streaming. This runs on the test source, so it holds the protocol and the process
/// handling only; that the real device is released is
/// `a_killed_child_releases_the_real_microphone`, by hand (M-7).
#[test]
fn a_killed_capture_child_leaves_the_microphone_to_the_next() {
    if !cfg!(debug_assertions) {
        return;
    }
    let root = sandbox();
    for round in 0..2 {
        let mut child = capture_child(root.path(), &[(TEST_SOURCE, "tone")]);
        let mut out = child.stdout.take().unwrap();
        receive(&mut out).expect("a probe");
        send(&mut child, &ToCapture::Start { device: None });
        assert!(
            matches!(receive(&mut out), Some((FromCapture::Ready(_), _))),
            "round {round}"
        );
        assert!(matches!(
            receive(&mut out),
            Some((FromCapture::Level(_), _))
        ));
        child.kill().unwrap();
        child.wait().unwrap();
    }
}

/// ALSA's configuration errors stay off the child's stderr. PulseAudio is pointed at a
/// socket that is not there, so miniaudio falls through to ALSA, and ALSA at a
/// configuration file that is not there, so listing its devices fails with the
/// `ALSA lib …` lines the shim's handler drops. Without libasound on the machine there is
/// nothing to print, and the test says so.
#[cfg(target_os = "linux")]
#[test]
fn alsa_errors_stay_off_the_capture_childs_stderr() {
    let root = sandbox();
    let mut command = Command::new(biorouter());
    command
        .args(["__br-dictation-worker", "capture"])
        .stdin(Stdio::null());
    base(&mut command, root.path());
    command
        .env("PULSE_SERVER", "unix:/nonexistent/biorouter-test/pulse")
        .env(
            "ALSA_CONFIG_PATH",
            "/nonexistent/biorouter-test/asound.conf",
        );
    let output = command.output().expect("run the capture child");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let (message, _) = receive(&mut std::io::Cursor::new(output.stdout.clone())).expect("a probe");
    let FromCapture::Probe(probe) = message else {
        panic!("the first frame is {message:?}");
    };
    let libasound = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
    ]
    .iter()
    .any(|dir| Path::new(dir).join("libasound.so.2").exists());
    if !libasound {
        eprintln!("no libasound.so.2 here, so ALSA had nothing to print: {probe:?}");
    }
    let err = stderr(&output);
    assert!(
        !err.contains("ALSA lib") && !err.to_ascii_lowercase().contains("alsa"),
        "ALSA wrote to the capture child's stderr: {err}"
    );
}

/// The child refuses `start` in an agent's shell before it touches the device: this runs
/// the real microphone backend, and nothing is opened.
#[test]
fn the_capture_child_refuses_an_agent_shell() {
    let root = sandbox();
    let mut child = capture_child(root.path(), &[("BIOROUTER_TERMINAL", "1")]);
    let mut out = child.stdout.take().unwrap();
    receive(&mut out).expect("a probe");
    send(&mut child, &ToCapture::Start { device: None });
    let (message, _) = receive(&mut out).expect("an answer");
    assert_eq!(
        message,
        FromCapture::Error {
            code: biorouter_dictation_capture::ErrorCode::AgentShell
        }
    );
    assert_eq!(child.wait().unwrap().code(), Some(2));
}

// ── refusals before anything starts ────────────────────────────────────────────────

#[test]
fn dictate_needs_a_terminal() {
    let root = sandbox();
    assert_refused(&run(root.path(), &["dictate"], &[]), messages::MSG_NO_TTY);
    let output = run(root.path(), &["dictate", "--format", "json"], &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error_code"], "no_terminal");
    assert_refused(
        &run(root.path(), &["dictation", "test"], &[]),
        messages::MSG_NO_TTY,
    );
}

#[test]
fn dictate_refuses_an_agent_shell() {
    let root = sandbox();
    assert_refused(
        &run(root.path(), &["dictate"], &[("BIOROUTER_TERMINAL", "1")]),
        messages::MSG_AGENT_SHELL,
    );
    #[cfg(unix)]
    {
        let output = at_a_terminal(
            root.path(),
            &["dictate"],
            &[("BIOROUTER_TERMINAL", "1")],
            Vec::new(),
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(
            plain(&stderr(&output)).contains(messages::MSG_AGENT_SHELL),
            "{}",
            stderr(&output)
        );
    }
}

/// DD-77, O-8: `--private-terms` needs a person at their own terminal and a private model
/// configured; anything else is MSG-PRIVATE-TERMS-REFUSED, exit 2.
#[test]
fn private_terms_are_refused_unless_dd77_admits_them() {
    let root = sandbox();
    let refused = messages::MSG_PRIVATE_TERMS_REFUSED;
    assert_refused(
        &run(root.path(), &["dictate", "--private-terms"], &[]),
        refused,
    );
    assert_refused(
        &run(
            root.path(),
            &["dictate", "--private-terms"],
            &[("BIOROUTER_TERMINAL", "1")],
        ),
        refused,
    );
    #[cfg(unix)]
    {
        // At a terminal, with no provider configured: caller_capability() is Public.
        let output = at_a_terminal(
            root.path(),
            &["dictate", "--private-terms"],
            &[],
            Vec::new(),
        );
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert!(
            plain(&stderr(&output)).contains(refused),
            "{}",
            stderr(&output)
        );
    }
}

/// Admitted, `--private-terms` asks MSG-PRIVATE-TERMS with `[y/N]` before the microphone
/// opens, and the recording goes on after the answer.
#[cfg(unix)]
#[test]
fn private_terms_ask_before_the_microphone_opens() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    std::fs::create_dir_all(root.path().join("config")).unwrap();
    // A private provider's name: only its declared tier is read, nothing is sent to it.
    std::fs::write(
        root.path().join("config").join("config.yaml"),
        "BIOROUTER_PROVIDER: versa_azure\nBIOROUTER_MODEL: gpt-5.5\n",
    )
    .unwrap();
    let model = for_tests_model();
    let output = at_a_terminal(
        root.path(),
        &[
            "dictate",
            "--private-terms",
            "--seconds",
            "1",
            "--no-review",
            "--format",
            "json",
        ],
        &[
            (TEST_SOURCE, "tone"),
            ("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap()),
        ],
        vec![Typed::When("[y/N]", b"y\n")],
    );
    let terminal = plain(&stderr(&output));
    assert!(
        terminal.contains(&format!("{} [y/N]", messages::MSG_PRIVATE_TERMS)),
        "{terminal}"
    );
    // The for-tests model hears nothing: the run went past the question to the decode.
    assert_eq!(
        output.status.code(),
        Some(3),
        "{terminal}{}",
        stdout(&output)
    );
    assert_eq!(json(&output)["error_code"], "no_speech");
}

/// No microphone over SSH and no model: the routes and exit 2, never the install question,
/// because step 7 comes before step 8.
#[cfg(unix)]
#[test]
fn over_ssh_without_a_model_the_routes_and_no_install_prompt() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let output = at_a_terminal(
        root.path(),
        &["dictate"],
        &[
            (TEST_SOURCE, "none"),
            ("SSH_CONNECTION", "10.0.0.2 52000 10.0.0.9 22"),
        ],
        Vec::new(),
    );
    let terminal = plain(&stderr(&output));
    assert_eq!(output.status.code(), Some(2), "{terminal}");
    assert!(
        terminal.contains("This computer has no microphone you can reach over SSH."),
        "{terminal}"
    );
    assert!(
        terminal.contains("biorouter dictionary export -"),
        "{terminal}"
    );
    assert!(!terminal.contains("dictation install"), "{terminal}");
}

/// A computer that can record but has no model is told which to install (exit 2).
#[cfg(unix)]
#[test]
fn without_a_model_dictate_names_the_one_to_install() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let output = at_a_terminal(
        root.path(),
        &["dictate"],
        &[(TEST_SOURCE, "tone")],
        Vec::new(),
    );
    let terminal = plain(&stderr(&output));
    assert_eq!(output.status.code(), Some(2), "{terminal}");
    assert!(
        terminal.contains("Dictation needs a speech model on this computer ("),
        "{terminal}"
    );
    assert!(
        terminal.contains("or type `/dictate install`."),
        "{terminal}"
    );
}

/// Exact zeros for 1.5 s: the permission sentence for this system, exit 3.
#[cfg(unix)]
#[test]
fn exact_zeros_are_the_permission_sentence() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let model = for_tests_model();
    let output = at_a_terminal(
        root.path(),
        &["dictate", "--seconds", "3", "--format", "json"],
        &[
            (TEST_SOURCE, "zeros"),
            ("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap()),
        ],
        Vec::new(),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let error = json(&output);
    assert_eq!(error["error_code"], "silence");
    let message = error["message"].as_str().unwrap();
    if cfg!(target_os = "macos") {
        assert!(
            message.starts_with("macOS gave Biorouter only silence"),
            "{message}"
        );
    } else {
        assert_eq!(message, messages::MSG_SILENT);
    }
}

/// Escape while recording cancels: exit 4, nothing on stdout.
#[cfg(unix)]
#[test]
fn escape_cancels_the_recording() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let model = for_tests_model();
    let output = at_a_terminal(
        root.path(),
        &["dictate"],
        &[
            (TEST_SOURCE, "tone"),
            ("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap()),
        ],
        // Once the recording line is drawn: a key typed earlier waits in the terminal and
        // cancels the moment the recording starts reading keys.
        vec![Typed::When("● REC", b"\x1b")],
    );
    let terminal = plain(&stderr(&output));
    assert_eq!(output.status.code(), Some(4), "{terminal}");
    assert!(
        terminal.contains(messages::MSG_DICTATION_CANCELLED),
        "{terminal}"
    );
    assert!(
        terminal.contains("● REC 0:0"),
        "the recording line was drawn: {terminal}"
    );
    assert!(output.stdout.is_empty());
}

/// `--seconds` stops by itself, and the for-tests model hears nothing in a tone (exit 3).
#[cfg(unix)]
#[test]
fn seconds_stop_the_recording_by_itself() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let model = for_tests_model();
    let started = Instant::now();
    let output = at_a_terminal(
        root.path(),
        &["dictate", "--seconds", "2", "--format", "json"],
        &[
            (TEST_SOURCE, "tone"),
            ("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap()),
        ],
        Vec::new(),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(json(&output)["error_code"], "no_speech");
    assert!(started.elapsed() < Duration::from_secs(30));
}

/// The environment of a recording that comes back as [`SAID`]: the test source's tone,
/// the for-tests model loaded as usual, and the debug build's fixed transcript.
#[cfg(unix)]
fn saying(model: &Path, more: &[(&'static str, &'static str)]) -> Vec<(&'static str, String)> {
    let mut env = vec![
        (TEST_SOURCE, "tone".to_string()),
        (
            "BIOROUTER_DICTATION_TEST_MODEL",
            model.to_str().unwrap().to_string(),
        ),
        (TEST_TEXT, SAID.to_string()),
    ];
    env.extend(more.iter().map(|(k, v)| (*k, (*v).to_string())));
    env
}

#[cfg(unix)]
fn pairs<'a>(env: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    env.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// The review (DD-76) with stdout piped, as `"$(biorouter dictate)"` runs: the text is
/// drawn on the terminal and nothing reaches stdout until the person decides. Enter uses
/// it, `e` edits it on the terminal, Escape discards it (exit 4, stdout empty).
#[cfg(unix)]
#[test]
fn the_review_uses_edits_or_discards_before_stdout() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let env = saying(&for_tests_model(), &[]);
    let env = pairs(&env);
    let args = ["dictate", "--seconds", "1"];
    // Each key waits until the review line is drawn and the grace has passed.
    let after_review = |keys: &'static [u8]| {
        vec![
            Typed::When(messages::LINE_REVIEW, b""),
            Typed::After(Duration::from_millis(200), keys),
        ]
    };
    let used = at_a_terminal(root.path(), &args, &env, after_review(b"\r"));
    let terminal = plain(&stderr(&used));
    assert_eq!(used.status.code(), Some(0), "{terminal}");
    assert_eq!(stdout(&used), format!("{SAID}\n"));
    assert!(terminal.contains(SAID), "the text was shown: {terminal}");
    assert!(terminal.contains(messages::LINE_REVIEW), "{terminal}");

    let mut keys = after_review(b"e");
    keys.push(Typed::After(Duration::from_millis(800), b" today\r"));
    let edited = at_a_terminal(root.path(), &args, &env, keys);
    let terminal = plain(&stderr(&edited));
    assert_eq!(edited.status.code(), Some(0), "{terminal}");
    assert_eq!(stdout(&edited), format!("{SAID} today\n"), "{terminal}");

    let discarded = at_a_terminal(root.path(), &args, &env, after_review(b"\x1b"));
    let terminal = plain(&stderr(&discarded));
    assert_eq!(discarded.status.code(), Some(4), "{terminal}");
    assert!(discarded.stdout.is_empty(), "{}", stdout(&discarded));
    assert!(
        terminal.contains(messages::MSG_DICTATION_CANCELLED),
        "{terminal}"
    );
}

/// DD-76: an Enter held down from the recording on, so that presses are waiting in the
/// terminal and still arriving when the text appears, neither uses nor edits it; the
/// person's Escape, once they have read it, discards it (exit 4, stdout empty). The key is
/// let go [`HELD_AFTER`] after the review line shows, inside the review's grace. With the
/// grace set to zero this test failed (exit 0, the text on stdout).
#[cfg(unix)]
#[test]
fn a_held_enter_does_not_send_the_text_unreviewed() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let env = saying(&for_tests_model(), &[]);
    let output = at_a_terminal(
        root.path(),
        &["dictate"],
        &pairs(&env),
        vec![
            Typed::When("● REC", b""),
            // Past MSG-TOO-SHORT's half second before the first press.
            Typed::After(Duration::from_millis(600), b""),
            Typed::HeldUntil(messages::LINE_REVIEW, b"\r"),
            Typed::After(Duration::from_millis(400), b"\x1b"),
        ],
    );
    let terminal = plain(&stderr(&output));
    assert_eq!(
        output.status.code(),
        Some(4),
        "{terminal}{}",
        stdout(&output)
    );
    assert!(output.stdout.is_empty(), "{}", stdout(&output));
    assert!(terminal.contains(SAID), "the text was shown: {terminal}");
}

/// `--copy` where no clipboard is reachable (no display): MSG-NO-CLIPBOARD on stderr, the
/// text on stdout, exit 5, after the review as always.
#[cfg(unix)]
#[test]
fn copy_without_a_clipboard_prints_the_text_and_exits_5() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let mut runs = vec![saying(&for_tests_model(), &[(TEST_CLIPBOARD, "none")])];
    // A Linux host with no display reaches the same answer without the stand in.
    let wsl = std::fs::read_to_string("/proc/version")
        .is_ok_and(|v| v.to_ascii_lowercase().contains("microsoft"));
    if cfg!(target_os = "linux") && !wsl {
        runs.push(saying(&for_tests_model(), &[]));
    }
    for env in runs {
        let output = at_a_terminal_without(
            root.path(),
            &["dictate", "--copy", "--seconds", "1"],
            &pairs(&env),
            &["DISPLAY", "WAYLAND_DISPLAY", "WSL_DISTRO_NAME"],
            vec![
                Typed::When(messages::LINE_REVIEW, b""),
                Typed::After(Duration::from_millis(200), b"\r"),
            ],
        );
        let terminal = plain(&stderr(&output));
        assert_eq!(output.status.code(), Some(5), "{terminal}");
        assert!(terminal.contains(messages::MSG_NO_CLIPBOARD), "{terminal}");
        assert_eq!(stdout(&output), format!("{SAID}\n"));
    }
}

/// The reviewed text reaches the shared clipboard helper before `dictate --copy` reports
/// success. A different expected text makes the same backend refuse the copy.
#[cfg(unix)]
#[test]
fn copy_success_requires_the_reviewed_text() {
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let model = for_tests_model();
    let accepted = "EPAS1 rose today";
    let args = ["dictate", "--copy", "--seconds", "1"];
    let reviewed = || {
        vec![
            Typed::When(messages::LINE_REVIEW, b""),
            Typed::After(Duration::from_millis(200), b"e"),
            // Select the whole editor line, then type a full replacement.
            Typed::After(Duration::from_millis(800), b"\x01\x0bEPAS1 rose today\r"),
        ]
    };

    let mut success_env = saying(&model, &[]);
    success_env.push((TEST_CLIPBOARD, format!("expect:{accepted}")));
    let copied = at_a_terminal(root.path(), &args, &pairs(&success_env), reviewed());
    let terminal = plain(&stderr(&copied));
    assert_eq!(copied.status.code(), Some(0));
    assert!(copied.stdout.is_empty());
    assert!(terminal.contains(SAID));
    assert!(terminal.contains(messages::LINE_REVIEW));
    let key = if cfg!(target_os = "macos") {
        "⌘V"
    } else {
        "Ctrl+V"
    };
    assert!(terminal.contains(&messages::msg_clipboard_copied(
        accepted.split_whitespace().count(),
        key
    )));

    let wrong_env = saying(&model, &[(TEST_CLIPBOARD, "expect:different text")]);
    let refused = at_a_terminal(root.path(), &args, &pairs(&wrong_env), reviewed());
    let terminal = plain(&stderr(&refused));
    assert_eq!(refused.status.code(), Some(5));
    assert!(terminal.contains(messages::MSG_NO_CLIPBOARD));
    assert!(stdout(&refused) == format!("{accepted}\n"));
}

/// `--copy --no-history` under WSL, whose clipboard cannot keep a copy out of Windows
/// clipboard history: refused before anything starts (exit 2), rather than copied anyway.
#[test]
fn no_history_is_refused_where_wsl_cannot_honour_it() {
    if !cfg!(debug_assertions) {
        return;
    }
    let root = sandbox();
    let output = run(
        root.path(),
        &["dictate", "--copy", "--no-history"],
        &[(TEST_CLIPBOARD, "wsl")],
    );
    assert_refused(&output, messages::MSG_NO_HISTORY_WSL);
    let json_output = run(
        root.path(),
        &["dictate", "--copy", "--no-history", "--format", "json"],
        &[(TEST_CLIPBOARD, "wsl")],
    );
    assert_eq!(json_output.status.code(), Some(2));
    assert_eq!(json(&json_output)["error_code"], "no_history_refused");
    // Elsewhere `--no-history` is honoured, so the run goes on to the terminal check.
    assert_refused(
        &run(root.path(), &["dictate", "--copy", "--no-history"], &[]),
        messages::MSG_NO_TTY,
    );
}

/// Over SSH on a Linux host where this account runs `biorouter serve`: MSG-SSH-SERVICE with
/// that service's port, in place of MSG-SSH-ROUTES, found by the scan of `/proc`. The
/// service here is a stand in whose command line reads `biorouter serve --port 9123`.
#[cfg(target_os = "linux")]
#[test]
fn over_ssh_a_running_serve_is_named_by_its_port() {
    use std::os::unix::process::CommandExt;
    if !cfg!(debug_assertions) || !can_dictate() {
        return;
    }
    let root = sandbox();
    let fake = tempfile::tempdir().unwrap();
    std::fs::write(fake.path().join("serve"), "sleep 60\n").unwrap();
    let mut service = Command::new("bash")
        .arg("-c")
        .arg("exec -a biorouter bash serve --port 9123")
        .current_dir(fake.path())
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("start the stand in service");
    let pgid = service.id() as libc::pid_t;
    // The stand in's command line is in place once bash has read its script.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let cmdline = std::fs::read(format!("/proc/{}/cmdline", service.id())).unwrap_or_default();
        if cmdline.starts_with(b"biorouter\0serve\0") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = at_a_terminal(
        root.path(),
        &["dictate"],
        &[
            (TEST_SOURCE, "none"),
            ("SSH_CONNECTION", "10.0.0.2 52000 10.0.0.9 22"),
        ],
        Vec::new(),
    );
    // SAFETY: `pgid` is the group the stand in leads; this ends it and its `sleep`.
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
    let _ = service.wait();
    let terminal = plain(&stderr(&output));
    assert_eq!(output.status.code(), Some(2), "{terminal}");
    assert!(
        terminal.contains("A Biorouter service already runs on this computer, on port 9123."),
        "{terminal}"
    );
    assert!(
        !terminal.contains("This computer has no microphone you can reach over SSH."),
        "{terminal}"
    );
    assert!(!terminal.contains("dictation install"), "{terminal}");
}

// ── devices, status and test ───────────────────────────────────────────────────────

#[test]
fn dictation_devices_report_the_probe() {
    let root = sandbox();
    let output = run(
        root.path(),
        &["dictation", "devices", "--format", "json"],
        &[],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let devices = json(&output);
    assert_eq!(devices["schema"], 1);
    let available = devices["available"].as_bool().unwrap();
    assert_eq!(available, devices["reason"].is_null(), "{devices}");
    assert_eq!(
        available,
        !devices["devices"].as_array().unwrap().is_empty(),
        "{devices}"
    );
    if cfg!(debug_assertions) {
        let none = run(
            root.path(),
            &["dictation", "devices", "--format", "json"],
            &[(TEST_SOURCE, "none")],
        );
        assert_eq!(json(&none)["reason"], "no_audio_system");
    }
}

/// The status's `capture` comes from the probe: `no_audio_system` where no audio system
/// answers, as in the package smoke containers.
#[test]
fn the_status_reports_the_capture_probe() {
    if !cfg!(debug_assertions) {
        return;
    }
    let root = sandbox();
    let output = run(
        root.path(),
        &["dictation", "status", "--format", "json"],
        &[(TEST_SOURCE, "none")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let status = json(&output);
    assert_eq!(status["capture"]["available"], false);
    assert_eq!(status["capture"]["reason"], "no_audio_system");
    let text = run(
        root.path(),
        &["dictation", "status"],
        &[(TEST_SOURCE, "none")],
    );
    assert!(
        stdout(&text).contains("Microphone:    no audio system here"),
        "{}",
        stdout(&text)
    );
}

// ── by hand ────────────────────────────────────────────────────────────────────────

/// M-7 on a real microphone: frames within 2 % of wall time at a 100 ms period, the first
/// within 300 ms. Opens the microphone, so macOS asks the terminal app the first time.
#[test]
#[ignore = "opens the real microphone"]
fn a_real_microphone_keeps_time() {
    let root = sandbox();
    let mut child = capture_child(root.path(), &[]);
    let mut out = child.stdout.take().unwrap();
    receive(&mut out).expect("a probe");
    let asked = Instant::now();
    send(&mut child, &ToCapture::Start { device: None });
    let Some((FromCapture::Ready(ready), _)) = receive(&mut out) else {
        panic!("the microphone did not open");
    };
    let opened = Instant::now();
    let (mut samples, mut first) = (0u64, None);
    while opened.elapsed() < Duration::from_secs(5) {
        if let Some((FromCapture::Audio, payload)) = receive(&mut out) {
            first.get_or_insert(asked.elapsed());
            samples += samples_of(&payload).len() as u64;
        }
    }
    let wall = opened.elapsed().as_secs_f64();
    send(&mut child, &ToCapture::Stop);
    let share = samples as f64 / 16_000.0 / wall;
    println!(
        "{} ({}, {} Hz): {samples} samples in {wall:.2} s ({:.1} %), first audio after {:?}",
        ready.device,
        ready.backend,
        ready.device_rate,
        share * 100.0,
        first
    );
    assert!((share - 1.0).abs() <= 0.02, "{share}");
    assert!(first.unwrap() < Duration::from_millis(300));
}

/// M-7 on a real microphone: a child killed with SIGKILL while recording leaves the device
/// to the next child at once, which opens it and streams within 2 s. Opens the microphone.
#[test]
#[ignore = "opens the real microphone"]
fn a_killed_child_releases_the_real_microphone() {
    let root = sandbox();
    for round in 0..2 {
        let mut child = capture_child(root.path(), &[]);
        let mut out = child.stdout.take().unwrap();
        receive(&mut out).expect("a probe");
        let asked = Instant::now();
        send(&mut child, &ToCapture::Start { device: None });
        let Some((FromCapture::Ready(ready), _)) = receive(&mut out) else {
            panic!("round {round}: the microphone did not open");
        };
        let mut samples = 0usize;
        while samples < 1_600 {
            if let Some((FromCapture::Audio, payload)) = receive(&mut out) {
                samples += samples_of(&payload).len();
            }
        }
        let opened = asked.elapsed();
        println!(
            "round {round}: {} ({}) streaming {opened:?} after start",
            ready.device, ready.backend
        );
        assert!(opened < Duration::from_secs(2), "round {round}: {opened:?}");
        child.kill().unwrap();
        child.wait().unwrap();
    }
}

/// The catalog models in `BIOROUTER_DICTATION_MODELS_DIR` and a speech clip from
/// `BIOROUTER_DICTATION_BENCH_DIR/audio` as the test source.
#[cfg(unix)]
fn real_models() -> (String, PathBuf) {
    let models = std::env::var("BIOROUTER_DICTATION_MODELS_DIR")
        .expect("set BIOROUTER_DICTATION_MODELS_DIR to a folder holding the catalog models");
    let bench = std::env::var("BIOROUTER_DICTATION_BENCH_DIR")
        .expect("set BIOROUTER_DICTATION_BENCH_DIR to the bench folder with audio/*.wav");
    (models, PathBuf::from(bench).join("audio"))
}

/// The review (DD-76): Enter uses the text, `e` edits it, Escape discards it; nothing
/// reaches stdout before the person decides.
#[cfg(unix)]
#[test]
#[ignore = "needs the real speech models"]
fn the_review_uses_edits_or_discards() {
    let (models, audio) = real_models();
    let clip = audio.join("s01_Samantha.wav");
    let root = sandbox();
    let env = [
        ("BIOROUTER_DICTATION_MODELS_DIR", models.as_str()),
        (TEST_SOURCE, clip.to_str().unwrap()),
    ];
    let args = [
        "dictate",
        "--seconds",
        "6",
        "--model",
        "whisper-small.en-q8_0",
    ];
    let used = at_a_terminal(
        root.path(),
        &args,
        &env,
        vec![Typed::When(messages::LINE_REVIEW, b"\r")],
    );
    assert_eq!(used.status.code(), Some(0), "{}", stderr(&used));
    let text = stdout(&used);
    assert!(text.split_whitespace().count() >= 3, "{text}");
    assert!(plain(&stderr(&used)).contains(messages::LINE_REVIEW));
    let edited = at_a_terminal(
        root.path(),
        &args,
        &env,
        vec![
            Typed::When(messages::LINE_REVIEW, b"e"),
            Typed::After(Duration::from_millis(800), b" EPAS1\r"),
        ],
    );
    assert_eq!(edited.status.code(), Some(0), "{}", stderr(&edited));
    assert!(
        stdout(&edited).trim_end().ends_with("EPAS1"),
        "{}",
        stdout(&edited)
    );
    let discarded = at_a_terminal(
        root.path(),
        &args,
        &env,
        vec![Typed::When(messages::LINE_REVIEW, b"\x1b")],
    );
    assert_eq!(discarded.status.code(), Some(4), "{}", stderr(&discarded));
    assert!(discarded.stdout.is_empty());
}

/// Stop to text with the real models, on this computer (PR-25 latency): each clip played
/// through the test source in real time, stopped by `--seconds`, transcribed with
/// `--no-review --format json`. Prints one line per run; run with `--nocapture`.
///
/// `BIOROUTER_DICTATE_LATENCY_ROOT` names a Biorouter root whose own folder holds the
/// models (`biorouter dictation install --from`), as a person's install does; without it
/// the models are read from `BIOROUTER_DICTATION_MODELS_DIR`, a read only folder whose
/// files each process hashes once before the microphone opens.
#[cfg(unix)]
#[test]
#[ignore = "needs the real speech models; a timing"]
fn dictate_latency_real_models() {
    let (models, audio) = real_models();
    let fresh = sandbox();
    let installed = std::env::var("BIOROUTER_DICTATE_LATENCY_ROOT").ok();
    let root = installed
        .as_deref()
        .map_or_else(|| fresh.path().to_path_buf(), PathBuf::from);
    let models = if installed.is_some() {
        String::new()
    } else {
        models
    };
    let root = root.as_path();
    // `BIOROUTER_DICTATE_LATENCY_CLIPS`: other recordings, comma separated, such as a long
    // one that shows segments decoded while the person speaks.
    let clips: Vec<PathBuf> = match std::env::var("BIOROUTER_DICTATE_LATENCY_CLIPS") {
        Ok(list) => list.split(',').map(PathBuf::from).collect(),
        Err(_) => [
            "s01_Samantha.wav",
            "s03_Daniel.wav",
            "s05_Karen.wav",
            "c01_Daniel.wav",
        ]
        .iter()
        .map(|c| audio.join(c))
        .collect(),
    };
    for model in ["whisper-small.en-q8_0", "whisper-large-v3-turbo-q5_0"] {
        for path in &clips {
            let clip = path.file_name().unwrap().to_string_lossy();
            let seconds =
                (std::fs::metadata(path).unwrap().len() as f64 / 32_000.0).ceil() as u32 + 1;
            let mut env = vec![(TEST_SOURCE, path.to_str().unwrap())];
            if !models.is_empty() {
                env.push(("BIOROUTER_DICTATION_MODELS_DIR", models.as_str()));
            }
            let output = at_a_terminal(
                root,
                &[
                    "dictate",
                    "--seconds",
                    &seconds.to_string(),
                    "--model",
                    model,
                    "--no-review",
                    "--format",
                    "json",
                ],
                &env,
                Vec::new(),
            );
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
            let out = json(&output);
            let t = &out["timings"];
            println!(
                "{model} {clip} ({} s): stop_to_text {} ms (tail {} ms over {} s, {} segments, {} while speaking), probe {} ms, resolve {} ms, device ready {} ms, first audio {} ms, model ready {} ms (load {} warm up {}), backend {}: {}",
                out["audio_s"], t["stop_to_text_ms"], t["tail_ms"], t["tail_audio_s"], t["segments"],
                t["segments_while_speaking"], t["probe_ms"], t["resolve_ms"], t["device_ready_ms"],
                t["first_audio_ms"], t["model_ready_ms"], t["load_ms"], t["warmup_ms"],
                t["backend"], out["text"]
            );
        }
    }
}
