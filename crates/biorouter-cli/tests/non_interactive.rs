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

/// SF-F8: `biorouter --version` names the program, and a version or help probe, the first
/// thing an install script runs, writes nothing: no `config.yaml`, no privacy master-switch
/// record, no log directory. The root starts empty and stays empty.
#[test]
fn a_version_or_help_probe_names_the_program_and_writes_nothing() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let probe = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_biorouter"))
            .args(args)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_STATE_HOME", home.join(".local/state"))
            .env("BIOROUTER_PATH_ROOT", &root)
            .env("BIOROUTER_DISABLE_KEYRING", "true")
            .stdin(Stdio::null())
            .output()
            .expect("run the biorouter binary")
    };
    let version = probe(&["--version"]);
    assert_eq!(version.status.code(), Some(0), "{}", text(&version.stderr));
    assert_eq!(
        text(&version.stdout),
        format!("biorouter {}\n", env!("CARGO_PKG_VERSION"))
    );
    for args in [
        &["--help"][..],
        &["session", "--help"],
        &["help", "run"],
        &["-V"],
    ] {
        let out = probe(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
    for written in [&root, &home] {
        assert_eq!(
            snapshot(written),
            Vec::new(),
            "a probe wrote under {}",
            written.display()
        );
        let entries: Vec<_> = std::fs::read_dir(written).unwrap().collect();
        assert!(entries.is_empty(), "a probe created {entries:?}");
    }
}

/// `echo | biorouter configure` exits 2 with the sentence, and the whole
/// configuration directory is byte-identical afterwards.
///
/// The sandbox is started once first. Every `biorouter` process that runs a
/// command runs the privacy master switch's one-time migration at start-up,
/// which writes `privacy-tiers.json` beside a config that lacks one; a real
/// install already has it. Snapshotting after that start makes the comparison
/// about what `configure` does, not about what start-up does once per install.
/// `info` starts up and only reads; `--version` no longer starts up at all.
#[test]
fn configure_under_a_pipe_exits_2_with_guidance_and_leaves_config_byte_identical() {
    let sandbox = Sandbox::new();
    let original = b"# hand-edited\nBIOROUTER_PROVIDER:   versa_azure\nBIOROUTER_MODEL: gpt-5.5\n";
    std::fs::write(sandbox.config_yaml(), original).unwrap();
    let warm = sandbox.run(&["info"], "");
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

/// R-30: resuming a chat in the terminal is opening it, not activity in it.
///
/// `biorouter session --resume` binds the model the chat records and saves the
/// extension roster it loaded, and both writes used to stamp `updated_at`, so a
/// resume moved the chat to the top of `session list` (and of the desktop's
/// lists, which read the same store). Measured before the fix, 2026-10-11:
/// index 115 to 0 in `session list`, even for a resume the person then
/// cancelled at the missing-extension prompt, because the bind runs before it.
///
/// Three resumes of the older of two chats, each against a fresh copy of the
/// same store: one that continues (stdin closed, so the interactive loop ends
/// at once), one cancelled at the missing-extension prompt, and one where the
/// person typed a different `--model`. The first two must leave the chat where
/// it was. The third is a model switch, which still moves it, and it is what
/// proves the run reached the bind at all: without it "unchanged" could mean
/// the CLI never got that far.
#[tokio::test]
async fn resuming_a_chat_in_the_terminal_does_not_move_it_but_switching_its_model_does() {
    use biorouter::model::ModelConfig;

    const MODEL: &str = "claude-sonnet-4-5";

    /// Two chats, the older one recording `anthropic` and `roster` as its
    /// extensions, each written in a second of its own (`datetime('now')` has
    /// one-second resolution). Returns (older, newer).
    async fn seed(data: &Path, roster: serde_json::Value) -> (String, String) {
        let sm = SessionManager::new(data.to_path_buf());
        let older = sm
            .create_session(data.to_path_buf(), "Older chat".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(&older.id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        sm.update(&older.id)
            .provider_name("anthropic")
            .model_config(ModelConfig::new(MODEL).unwrap())
            .apply()
            .await
            .unwrap();
        sm.update_extension_state(&older.id, "enabled_extensions", "v0", move |_| Ok(roster))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        let newer = sm
            .create_session(data.to_path_buf(), "Newer chat".into(), SessionType::User)
            .await
            .unwrap();
        sm.add_message(&newer.id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        // The resume below runs in a later second than anything seeded, so a
        // stamp cannot hide inside a second the rows already hold.
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        sm.close().await;
        (older.id, newer.id)
    }

    async fn updated_at(data: &Path, id: &str) -> chrono::DateTime<chrono::Utc> {
        let sm = SessionManager::new(data.to_path_buf());
        let at = sm.get_session(id, false).await.unwrap().updated_at;
        sm.close().await;
        at
    }

    /// The ids `biorouter session list --format json` prints, in its order.
    fn listed(sandbox: &Sandbox) -> Vec<String> {
        let out = sandbox.run(&["session", "list", "--format", "json"], "");
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)));
        rows.iter()
            .filter_map(|row| row["id"].as_str().map(str::to_string))
            .collect()
    }

    /// `biorouter session --resume --session-id <id> [extra…]` with a
    /// placeholder key: constructing the provider opens no connection, and
    /// nothing here sends a message.
    fn resume(sandbox: &Sandbox, id: &str, extra: &[&str]) -> Output {
        let mut args = vec!["session", "--resume", "--session-id", id];
        args.extend_from_slice(extra);
        let mut child = Command::new(env!("CARGO_BIN_EXE_biorouter"))
            .args(&args)
            .env("HOME", &sandbox.home)
            .env("XDG_CONFIG_HOME", sandbox.home.join(".config"))
            .env("BIOROUTER_PATH_ROOT", &sandbox.root)
            .env("BIOROUTER_DISABLE_KEYRING", "true")
            .env("ANTHROPIC_API_KEY", "placeholder-never-sent")
            .env_remove("BIOROUTER_PROVIDER")
            .env_remove("BIOROUTER_MODEL")
            .env_remove("BIOROUTER_SERVER__SECRET_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the biorouter binary");
        drop(child.stdin.take());
        child.wait_with_output().unwrap()
    }

    let cases: [(&str, serde_json::Value, &[&str], bool); 4] = [
        (
            "a resume that continues",
            serde_json::json!({ "extensions": [] }),
            &[],
            false,
        ),
        (
            // Typing the model the chat already records restores it: that is
            // not a switch, so the chat stays where it was.
            "a resume that names the model the chat already records",
            serde_json::json!({ "extensions": [] }),
            &["--provider", "anthropic", "--model", MODEL],
            false,
        ),
        (
            "a resume cancelled at the missing-extension prompt",
            serde_json::json!({ "extensions": [{
                "type": "stdio",
                "name": "r30-extension-that-is-not-installed",
                "cmd": "r30-extension-that-is-not-installed",
                "args": [],
                "description": "named by the chat, installed nowhere",
            }] }),
            &[],
            false,
        ),
        (
            "a resume that switches the model",
            serde_json::json!({ "extensions": [] }),
            &["--model", "claude-opus-4-1"],
            true,
        ),
    ];
    for (case, roster, extra, moves) in cases {
        let sandbox = Sandbox::new();
        std::fs::write(
            sandbox.config_yaml(),
            format!("BIOROUTER_PROVIDER: anthropic\nBIOROUTER_MODEL: {MODEL}\n"),
        )
        .unwrap();
        let data = sandbox.root.join("data");
        let (older, newer) = seed(&data, roster).await;
        let before = updated_at(&data, &older).await;
        assert_eq!(
            listed(&sandbox),
            [newer.clone(), older.clone()],
            "{case}: premise"
        );

        let out = resume(&sandbox, &older, extra);
        let stderr = text(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{case}: {}{stderr}",
            text(&out.stdout)
        );
        assert!(
            !stderr.contains("Failed to initialize agent"),
            "{case}: {stderr}"
        );

        let after = updated_at(&data, &older).await;
        if moves {
            assert!(after > before, "{case}: a model switch moves the chat");
            assert_eq!(listed(&sandbox), [older.clone(), newer.clone()], "{case}");
        } else {
            assert_eq!(
                after, before,
                "{case}: opening a chat is not activity (R-30)"
            );
            assert_eq!(listed(&sandbox), [newer.clone(), older.clone()], "{case}");
        }
    }
}

/// R-30, the other half, from the terminal: renaming a chat IS activity.
/// `biorouter session rename` moves the chat to the top of `session list`, and
/// so of the desktop's lists, which read the same store. The resume test above
/// pins that opening does not; this pins that the CLI's own rename still does.
#[tokio::test]
async fn renaming_a_chat_in_the_terminal_moves_it_to_the_top_of_session_list() {
    let sandbox = Sandbox::new();
    let data = sandbox.root.join("data");
    let sm = SessionManager::new(data.clone());
    let mut ids = Vec::new();
    for name in ["Older chat", "Newer chat"] {
        let id = sm
            .create_session(data.clone(), name.into(), SessionType::User)
            .await
            .unwrap()
            .id;
        sm.add_message(&id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        ids.push(id);
        // `datetime('now')` has one-second resolution: each write, and the
        // rename below, in a second of its own, so nothing ties.
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    }
    sm.close().await;
    let (older, newer) = (ids[0].clone(), ids[1].clone());

    let listed = || -> Vec<String> {
        let out = sandbox.run(&["session", "list", "--format", "json"], "");
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)));
        rows.iter()
            .filter_map(|row| row["id"].as_str().map(str::to_string))
            .collect()
    };
    assert_eq!(listed(), [newer.clone(), older.clone()], "premise");

    let out = sandbox.run(
        &[
            "session",
            "rename",
            "--session-id",
            &older,
            "--new-name",
            "Renamed in the terminal",
        ],
        "",
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert_eq!(
        listed(),
        [older, newer],
        "a rename is activity (R-30): the renamed chat heads the list"
    );
}
