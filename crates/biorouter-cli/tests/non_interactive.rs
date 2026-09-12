//! The real `biorouter` binary, run the way a script runs it: stdin, stdout and
//! stderr all pipes, against a throwaway `BIOROUTER_PATH_ROOT` (QA-D F5a / F9,
//! QA-F F9).
//!
//! The unit tests beside each command pin the decisions; this pins what only a
//! separate process can show — the exit status `main` maps the refusal to, that
//! the sentence (and not cliclack's `Error: not connected`) reaches stderr, and
//! that the files on disk are untouched afterwards.
//!
//! ⚠ Every child gets its own `HOME`, `XDG_CONFIG_HOME` and
//! `BIOROUTER_PATH_ROOT`, so nothing here can reach the developer's real
//! configuration or session store, whatever the parent process was started with.

// Each `tests/*.rs` is its own crate, so the lib's `#[cfg(test)] mod
// test_sandbox;` is not compiled into this binary. Declare its own copy, so
// anything here that reaches a process-global cell resolves under a throwaway
// root rather than the developer's real config and session store. The children
// this file spawns pass their own `BIOROUTER_PATH_ROOT`, which still wins.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use biorouter::conversation::message::Message;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let home = dir.path().join("home");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        Self {
            _dir: dir,
            home,
            root,
        }
    }

    fn config_yaml(&self) -> PathBuf {
        self.root.join("config").join("config.yaml")
    }

    /// Run `biorouter <args>` with every stream a pipe, `stdin` written and
    /// closed — `echo … | biorouter …`.
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_biorouter"))
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("BIOROUTER_PATH_ROOT", &self.root)
            .env("BIOROUTER_DISABLE_KEYRING", "true")
            .env_remove("BIOROUTER_PROVIDER")
            .env_remove("BIOROUTER_MODEL")
            .env_remove("BIOROUTER_SERVER__SECRET_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the biorouter binary");
        // Best effort: a command that refuses before reading may already have
        // exited and closed its end, and that is exactly the case under test.
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Every file under `dir` with its bytes, sorted by path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                files.push((path.strip_prefix(dir).unwrap().to_path_buf(), bytes));
            }
        }
    }
    files.sort();
    files
}

/// `echo | biorouter configure` exits 2 with the sentence, and the whole
/// configuration directory is byte-identical afterwards.
///
/// The sandbox is started once first. Every `biorouter` process — `--version`
/// included — runs the privacy master switch's one-time migration at start-up,
/// which writes `privacy-tiers.json` beside a config that lacks one; a real
/// install already has it. Snapshotting after that start makes the comparison
/// about what `configure` does, not about what start-up does once per install.
#[test]
fn configure_under_a_pipe_exits_2_with_guidance_and_leaves_config_byte_identical() {
    let sandbox = Sandbox::new();
    let original = b"# hand-edited\nBIOROUTER_PROVIDER:   versa_azure\nBIOROUTER_MODEL: gpt-5.5\n";
    std::fs::write(sandbox.config_yaml(), original).unwrap();
    let warm = sandbox.run(&["--version"], "");
    assert_eq!(warm.status.code(), Some(0), "{}", text(&warm.stderr));
    let before = snapshot(&sandbox.root.join("config"));
    assert!(
        before
            .iter()
            .any(|(path, _)| path == Path::new("config.yaml")),
        "{before:?}"
    );

    let out = sandbox.run(&["configure"], "\n");
    let stderr = text(&out.stderr);

    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("`biorouter configure` is interactive and needs a terminal"),
        "{stderr}"
    );
    assert!(
        stderr.contains("biorouter models set --provider"),
        "{stderr}"
    );
    assert!(!stderr.contains("not connected"), "{stderr}");
    assert_eq!(std::fs::read(sandbox.config_yaml()).unwrap(), original);
    assert_eq!(
        snapshot(&sandbox.root.join("config")),
        before,
        "a refused configure changed the configuration directory"
    );
}

/// Both project commands refuse under a pipe with the same exit status.
#[test]
fn project_and_projects_under_a_pipe_exit_2_with_guidance() {
    let sandbox = Sandbox::new();
    let project_dir = sandbox.home.display().to_string();
    std::fs::write(
        sandbox.root.join("data").join("projects.json"),
        serde_json::json!({
            "projects": {
                project_dir.clone(): {
                    "path": project_dir,
                    "last_accessed": "2026-09-10T12:00:00Z",
                    "last_instruction": null,
                    "last_session_id": null
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    for command in ["project", "projects"] {
        let out = sandbox.run(&[command], "\n");
        let stderr = text(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{command}: {stderr}");
        assert!(
            stderr.contains(&format!("`biorouter {command}` is interactive")),
            "{command}: {stderr}"
        );
        assert!(!stderr.contains("not connected"), "{command}: {stderr}");
    }
}

/// A store with a chat that has a message, a message-less chat, and a
/// subagent run. Returns their ids in that order.
async fn seed_three_row_kinds(data_dir: &Path) -> (String, String, String) {
    let sm = SessionManager::new(data_dir.to_path_buf());
    let chat = sm
        .create_session(
            data_dir.to_path_buf(),
            "Cohort review".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    sm.add_message(&chat.id, &Message::user().with_text("hello"))
        .await
        .unwrap();
    let empty = sm
        .create_session(
            data_dir.to_path_buf(),
            "CLI Session".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let subagent = sm
        .create_session(
            data_dir.to_path_buf(),
            "Subagent: audit".into(),
            SessionType::SubAgent,
        )
        .await
        .unwrap();
    sm.close().await;
    (chat.id, empty.id, subagent.id)
}

async fn exists(data_dir: &Path, id: &str) -> bool {
    let sm = SessionManager::new(data_dir.to_path_buf());
    let found = sm.get_session(id, false).await.is_ok();
    sm.close().await;
    found
}

/// `session remove` under a pipe: without `--yes` it exits 2 and deletes
/// nothing, even with `y` piped in; with `--yes` it removes a message-less row
/// and a subagent run — the two kinds it used to answer "not found" for.
#[tokio::test]
async fn session_remove_under_a_pipe_needs_yes_and_then_reaches_every_row_kind() {
    let sandbox = Sandbox::new();
    let data = sandbox.root.join("data");
    let (chat, empty, subagent) = seed_three_row_kinds(&data).await;

    let out = sandbox.run(&["session", "remove", "--session-id", &empty], "y\n");
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--yes"), "{stderr}");
    assert!(!stderr.contains("not connected"), "{stderr}");
    assert!(
        exists(&data, &empty).await,
        "a refused remove deleted the row"
    );

    for id in [&empty, &subagent] {
        let out = sandbox.run(&["session", "remove", "--session-id", id, "--yes"], "");
        let stdout = text(&out.stdout);
        assert_eq!(
            out.status.code(),
            Some(0),
            "remove {id}: {stdout}{}",
            text(&out.stderr)
        );
        assert!(
            stdout.contains(&format!("Session `{id}` removed.")),
            "{stdout}"
        );
        assert!(!exists(&data, id).await, "{id} is still in the store");
    }
    assert!(exists(&data, &chat).await, "only the named rows go");
}
