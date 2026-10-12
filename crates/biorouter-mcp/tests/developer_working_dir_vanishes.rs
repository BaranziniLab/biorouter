//! #64, end-to-end: delete a session working directory out from under the
//! `text_editor` tool and observe what the tool actually does.
//!
//! The unit tests added with the fix drive the private `resolve_path_jailed`
//! helper. This drives the **public tool an agent actually calls**, through the
//! same entry point the MCP dispatch uses, against a real filesystem — so it
//! pins the behaviour a user would see rather than the behaviour of an internal
//! helper. The two things it has to show are the two things #64 weighed against
//! each other:
//!
//! 1. the process survives (the old `expect("should have a current working
//!    dir")` took it down), and
//! 2. the jail is **not** re-rooted onto the process cwd — a file that was
//!    refused while the session directory existed is still refused after it
//!    disappears, and is still not written.
//!
//! Point 2 is the one that matters: "don't panic" must not be paid for with a
//! wider security boundary.
//!
//! This test owns the process cwd and `BIOROUTER_WORKING_DIR`, so it is a single
//! sequential test in its own integration binary rather than several parallel
//! ones.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use std::path::PathBuf;

use biorouter_mcp::developer::rmcp_developer::TextEditorParams;
use biorouter_mcp::DeveloperServer;
use rmcp::handler::server::wrapper::Parameters;

fn write_params(path: &str, text: &str) -> Parameters<TextEditorParams> {
    Parameters(TextEditorParams {
        path: path.to_string(),
        command: "write".to_string(),
        diff: None,
        view_range: None,
        file_text: Some(text.to_string()),
        old_str: None,
        new_str: None,
        insert_line: None,
    })
}

/// Restores the process cwd and `BIOROUTER_WORKING_DIR` on drop, so a panicking
/// assertion cannot strand the rest of the binary on a deleted directory.
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
async fn text_editor_keeps_its_jail_when_the_session_directory_is_deleted() {
    let _restore = RestoreEnv {
        cwd: std::env::current_dir().ok(),
        working_dir: std::env::var("BIOROUTER_WORKING_DIR").ok(),
    };
    // A still-existing BIOROUTER_WORKING_DIR is a *different* fallback path
    // (see the report on this branch); this test is about the process cwd.
    std::env::remove_var("BIOROUTER_WORKING_DIR");

    // The process sits in `outside`. The session is jailed to `session`.
    // These are different directories, which is the whole point: if the jail
    // base silently moves from `session` to the process cwd, `outside` becomes
    // reachable.
    let outside = tempfile::tempdir().unwrap();
    let outside_path = std::fs::canonicalize(outside.path()).unwrap();
    std::env::set_current_dir(&outside_path).unwrap();

    let session = tempfile::tempdir().unwrap();
    let session_path = session.path().to_path_buf();
    let server = DeveloperServer::new().with_working_dir(session_path.clone());

    // A real file in the process cwd, outside the session jail.
    let probe = outside_path.join("outside-the-jail.txt");
    std::fs::write(&probe, "original contents").unwrap();
    let probe_str = probe.to_str().unwrap().to_string();

    // --- while the session directory exists -------------------------------
    // Writing inside the jail works...
    server
        .text_editor(write_params("inside.txt", "hello"))
        .await
        .expect("a relative path inside the session dir must be writable");
    assert_eq!(
        std::fs::read_to_string(session_path.join("inside.txt"))
            .unwrap()
            .trim_end(),
        "hello"
    );
    // ...and writing outside it is refused.
    server
        .text_editor(write_params(&probe_str, "CLOBBERED"))
        .await
        .expect_err("sanity: a path outside the session dir must be refused");
    assert_eq!(
        std::fs::read_to_string(&probe).unwrap(),
        "original contents",
        "sanity: the refused write must not have touched the file"
    );

    // --- the session directory disappears mid-session ----------------------
    drop(session);
    assert!(!session_path.exists(), "the session dir really is gone");

    // Collect BOTH outcomes before asserting, so a regression reports the whole
    // picture (relative and absolute) instead of stopping at the first one.
    //
    // A relative path: with the base gone there is nothing legitimate to resolve
    // it against. Pre-fix this silently landed in the *process cwd* — i.e. in
    // `outside`, a directory the session was never given.
    let relative = server
        .text_editor(write_params("inside.txt", "after"))
        .await;
    let escaped_relative = outside_path.join("inside.txt");
    let relative_escaped = escaped_relative.exists();

    // An absolute path that the jail refused a moment ago. If the base had been
    // re-rooted onto the process cwd, `probe` now sits inside the jail.
    let absolute = server
        .text_editor(write_params(&probe_str, "CLOBBERED"))
        .await;
    let probe_after = std::fs::read_to_string(&probe).unwrap();

    assert!(
        !relative_escaped,
        "a relative path escaped into the process cwd once the session dir vanished: \
         {} was created, so the jail base was silently re-rooted",
        escaped_relative.display()
    );
    let err = relative.expect_err("with no base to resolve against, the call must fail");
    assert!(
        err.message.contains("no longer exists"),
        "the error must say why, got: {}",
        err.message
    );
    assert!(
        err.message.contains(&session_path.display().to_string()),
        "and must name the directory that vanished, got: {}",
        err.message
    );

    // THE POINT: the previously-refused path outside the jail is still refused.
    assert!(
        absolute.is_err(),
        "the jail must not be re-rooted onto the process cwd: a write outside the \
         session dir succeeded once the session dir vanished"
    );
    assert_eq!(
        probe_after, "original contents",
        "and nothing outside the jail may be written"
    );
}
