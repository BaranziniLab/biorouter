//! #68: a directly-constructed Developer server must read `.biorouterignore`
//! from the same directory its file tools are jailed to.
//!
//! `biorouter mcp developer` and `biorouterd mcp developer` build the server
//! with `DeveloperServer::new()` and never call `with_working_dir` — the base
//! reaches that child process as `BIOROUTER_WORKING_DIR`. The jail honours it
//! (`effective_cwd` reads it), but the `SecretGuard` was rooted at the process
//! cwd, which for a spawned child is whatever directory the parent happened to
//! be in. The two then disagree about where "here" is, and the protected file
//! in the directory the tools are actually working in is readable.
//!
//! Driven through the public `text_editor` tool. Owns the process cwd and
//! `BIOROUTER_WORKING_DIR`, so it is a single test in its own integration
//! binary.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use std::path::PathBuf;

use biorouter_mcp::developer::rmcp_developer::TextEditorParams;
use biorouter_mcp::DeveloperServer;
use rmcp::handler::server::wrapper::Parameters;

fn view_params(path: &str) -> Parameters<TextEditorParams> {
    Parameters(TextEditorParams {
        path: path.to_string(),
        command: "view".to_string(),
        diff: None,
        view_range: None,
        file_text: None,
        old_str: None,
        new_str: None,
        insert_line: None,
    })
}

/// Restores the process cwd and `BIOROUTER_WORKING_DIR` on drop, so a panicking
/// assertion cannot strand the rest of the binary on a temporary directory.
/// Declared *after* the directories it leaves, so it runs *before* they are
/// removed.
struct RestoreEnv {
    cwd: Option<PathBuf>,
    working_dir: Option<String>,
}

impl Drop for RestoreEnv {
    fn drop(&mut self) {
        if let Some(dir) = self.cwd.take() {
            let _ = std::env::set_current_dir(dir);
        }
        match self.working_dir.take() {
            Some(v) => std::env::set_var("BIOROUTER_WORKING_DIR", v),
            None => std::env::remove_var("BIOROUTER_WORKING_DIR"),
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn secret_guard_follows_the_env_working_dir_for_a_directly_constructed_server() {
    // The directories outlive the guard that returns the process to safety.
    let process_dir = tempfile::tempdir().unwrap();
    let env_dir = tempfile::tempdir().unwrap();

    let _restore = RestoreEnv {
        cwd: std::env::current_dir().ok(),
        working_dir: std::env::var("BIOROUTER_WORKING_DIR").ok(),
    };

    let process_path = std::fs::canonicalize(process_dir.path()).unwrap();
    let env_path = std::fs::canonicalize(env_dir.path()).unwrap();
    std::env::set_current_dir(&process_path).unwrap();
    std::env::set_var("BIOROUTER_WORKING_DIR", &env_path);

    // The directory the tools work in protects one of its files. The name
    // matches no built-in secret pattern, so only this ignore file can deny it.
    std::fs::write(env_path.join(".biorouterignore"), "proprietary-notes.md\n").unwrap();
    let protected = env_path.join("proprietary-notes.md");
    std::fs::write(&protected, "READ ALLOWED").unwrap();

    // Exactly how `biorouter mcp developer` builds it: no bound working dir.
    let server = DeveloperServer::new();

    // Sanity: the jail really is rooted at the env directory, not the process
    // cwd — otherwise the assertion below would prove nothing about the two
    // agreeing.
    let in_process_dir = process_path.join("elsewhere.md");
    std::fs::write(&in_process_dir, "not the jail's business").unwrap();
    let outside = server
        .text_editor(view_params(in_process_dir.to_str().unwrap()))
        .await
        .expect_err("sanity: the jail is the env dir, so the process cwd is outside it");
    assert!(
        outside.message.contains("outside the working directory"),
        "sanity: expected a jail refusal, got: {}",
        outside.message
    );

    // THE POINT: the guard must read the ignore file of the directory the jail
    // is rooted at.
    let result = server
        .text_editor(view_params(protected.to_str().unwrap()))
        .await;
    let err = result.map(|out| {
        out.content
            .iter()
            .find_map(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default()
    });
    let err = match err {
        Ok(text) => panic!(
            "the working directory's .biorouterignore was never read: the protected \
             file came back as {text:?}, so the secret guard is rooted somewhere the \
             file tools are not"
        ),
        Err(e) => e,
    };
    assert!(
        err.message.contains(".biorouterignore"),
        "the refusal must name the reason, got: {}",
        err.message
    );
}
