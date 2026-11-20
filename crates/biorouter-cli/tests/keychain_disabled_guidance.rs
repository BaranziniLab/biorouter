//! Issue #521: with `BIOROUTER_DISABLE_KEYRING` set, `biorouter run` reads keys
//! from the plaintext file store only and never asks the system keychain. A key
//! saved in the desktop app's Settings lives in the keychain, so it is not found,
//! and the error used to say the key "is not configured" and send the person to
//! Settings, which cannot fix it.
//!
//! The real binary, run against a throwaway root whose file store is empty. The
//! variable keeps the child off the system keychain, and each provider fails
//! before it sends any request, so nothing here reaches a credential store or
//! the network.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

struct Run {
    output: Output,
    secrets_file: PathBuf,
    _scratch: tempfile::TempDir,
}

/// `biorouter run` on `provider` with no key saved anywhere it can look.
async fn run_without_a_saved_key(provider: &str, model: &str) -> Run {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().join("home");
    let root = scratch.path().join("root");
    for path in [&home, &root.join("config"), &root.join("data")] {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_biorouter"));
    command
        .args([
            "run",
            "--provider",
            provider,
            "--model",
            model,
            "--no-session",
            "--max-turns",
            "1",
            "--text",
            "issue 521 fixture",
        ])
        .env_clear()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("BIOROUTER_PATH_ROOT", &root)
        // Any value turns the keychain off; the issue's reporter had it set by a
        // parent process.
        .env("BIOROUTER_DISABLE_KEYRING", "1")
        .env("BIOROUTER_CLASSIC", "1")
        .env("DO_NOT_TRACK", "1")
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in ["PATH", "SystemRoot", "WINDIR", "TMP", "TEMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = tokio::time::timeout(Duration::from_secs(120), command.output())
        .await
        .expect("the CLI must exit on its own")
        .unwrap();
    Run {
        output,
        secrets_file: root.join("config").join("secrets.yaml"),
        _scratch: scratch,
    }
}

fn stderr(run: &Run) -> String {
    String::from_utf8_lossy(&run.output.stderr).into_owned()
}

/// The pieces every keychain-disabled diagnostic carries: the file that was
/// read, that the variable is why the keychain was not, and the two remedies.
fn assert_names_the_file_store_and_both_remedies(run: &Run, said: &str) {
    assert!(
        said.contains(&run.secrets_file.display().to_string()),
        "the diagnostic must name the file that was actually checked: {said}"
    );
    assert!(
        said.contains("BIOROUTER_DISABLE_KEYRING is set, so the system keychain was not checked"),
        "the diagnostic must say why the keychain was not checked: {said}"
    );
    assert!(
        said.contains("Unset BIOROUTER_DISABLE_KEYRING"),
        "the diagnostic must say how to reach a key saved in the keychain: {said}"
    );
    assert!(
        said.contains("biorouter configure"),
        "the diagnostic must name the command that saves into the file store: {said}"
    );
    assert!(
        !said.contains("check your system keychain"),
        "the keychain was never read, so checking it is not the remedy: {said}"
    );
    assert!(
        !run.secrets_file.exists(),
        "a failed start must not write the file store"
    );
}

#[tokio::test]
async fn issue_521_versa_bedrock_names_the_file_store_instead_of_settings() {
    let run = run_without_a_saved_key(
        "versa_bedrock",
        biorouter::providers::versa_bedrock::VERSA_BEDROCK_DEFAULT_MODEL,
    )
    .await;
    let said = stderr(&run);
    assert!(!run.output.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "VERSA_BEDROCK_ACCESS_KEY_ID is not configured in the file store {}",
            run.secrets_file.display()
        )),
        "{said}"
    );
    assert!(
        !said.contains("Add it under Versa API Bedrock in Settings"),
        "Settings saves to the keychain this run does not read: {said}"
    );
    assert_names_the_file_store_and_both_remedies(&run, &said);
}

#[tokio::test]
async fn issue_521_any_keyed_provider_gets_the_file_store_advice_not_the_keychain_advice() {
    let run = run_without_a_saved_key("openai", "gpt-4o-mini").await;
    let said = stderr(&run);
    assert!(!run.output.status.success(), "{said}");
    assert!(said.contains("OPENAI_API_KEY"), "{said}");
    assert_names_the_file_store_and_both_remedies(&run, &said);
}
