//! `biorouterd`'s dictation worker roles, run as the real binary (PR-09, DD-4, DD-5).
//!
//! * `biorouterd __br-dictation-worker capture` exits 2: capture code is linked only into
//!   `biorouter`, never into the network facing daemon (P-10);
//! * the offline self test passes on the CPU and prints its one JSON line, whenever an
//!   engine is linked;
//! * an unknown role exits 2.
//!
//! `rust.yml`'s `test` job runs this binary on all three operating systems: ubuntu in the
//! offline integration step, macOS and the Windows MSVC leg in "cargo test (dictation
//! worker roles)". A build without an engine (`MSVC_NATIVE = false` in the dictation
//! crate's `build.rs`, or any `br_no_native` target) answers 2 for the self test, so that
//! test returns early there.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before
// `main`, as every test binary here does. The worker reads none of them.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::PathBuf;
use std::process::Command;

const MARKER: &str = "__br-dictation-worker";

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor")
        .join("whisper.cpp")
        .join("source")
        .join(relative)
}

fn daemon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_biorouterd"))
}

#[test]
fn the_daemon_has_no_capture_role() {
    let status = daemon()
        .args([MARKER, "capture"])
        .status()
        .expect("run biorouterd");
    assert_eq!(status.code(), Some(2));
}

#[test]
fn an_unknown_role_exits_2() {
    let status = daemon()
        .args([MARKER, "no-such-role"])
        .status()
        .expect("run biorouterd");
    assert_eq!(status.code(), Some(2));
}

#[cfg(feature = "local-dictation")]
#[test]
fn the_daemon_passes_the_self_test_on_the_cpu() {
    if !biorouter_dictation::ENGINE_COMPILED {
        return;
    }
    let output = daemon()
        .arg(MARKER)
        .arg("self-test")
        .arg("--model")
        .arg(fixture("models/for-tests-ggml-base.en.bin"))
        .arg("--vad")
        .arg(fixture("models/for-tests-silero-v6.2.0-ggml.bin"))
        .arg("--wav")
        .arg(fixture("samples/jfk.wav"))
        .arg("--cpu")
        .output()
        .expect("run biorouterd");
    let stdout = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout {stdout:?} stderr {stderr:?}"
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one stdout line: {stdout:?}");
    assert!(lines[0].contains(r#""selftest":"ok""#), "{stdout}");
    assert!(lines[0].contains(r#""backend":"cpu""#), "{stdout}");
}
