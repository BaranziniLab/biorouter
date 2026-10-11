//! #68, end-to-end: delete a session working directory that is a *subdirectory*
//! of `BIOROUTER_WORKING_DIR` and observe what `text_editor` actually does.
//!
//! #64 closed the wider case — a vanished session directory re-rooting the jail
//! onto the **process** cwd, typically `/` under the packaged desktop app. It
//! left one substitution standing: `session_cwd_or_fallback` still handed the
//! jail to `BIOROUTER_WORKING_DIR`. Both values are ones the application
//! sanctioned (`ui/desktop/src/main.ts` sets that variable to the window's
//! working directory), so this was never an escape to an arbitrary path. It was
//! still a base substitution, and this is the geometry where a substitution
//! *widens*: session inside env base, so the two do not vanish together, and the
//! jail moves up to the parent.
//!
//! The companion binary `developer_working_dir_vanishes.rs` pins the #64 case
//! with `BIOROUTER_WORKING_DIR` deliberately unset. This one is its mirror: the
//! env base is present throughout and is the thing the jail must not fall to.
//!
//! Driven through the **public tool an agent actually calls**, so it pins the
//! behaviour a user would see rather than an internal helper's. Like its
//! companion it owns the process cwd and `BIOROUTER_WORKING_DIR`, so it is a
//! single sequential test in its own integration binary.

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
async fn text_editor_jail_does_not_widen_to_the_env_base() {
    let _restore = RestoreEnv {
        cwd: std::env::current_dir().ok(),
        working_dir: std::env::var("BIOROUTER_WORKING_DIR").ok(),
    };

    // The env base is the parent; the session works one level down inside it.
    let env_base = tempfile::tempdir().unwrap();
    let env_path = std::fs::canonicalize(env_base.path()).unwrap();
    std::env::set_var("BIOROUTER_WORKING_DIR", &env_path);

    // Park the process somewhere else entirely, so nothing here can be confused
    // for the #64 process-cwd substitution: the only candidate that could widen
    // the jail is the env base.
    let elsewhere = tempfile::tempdir().unwrap();
    std::env::set_current_dir(std::fs::canonicalize(elsewhere.path()).unwrap()).unwrap();

    let session_path = env_path.join("session");
    std::fs::create_dir(&session_path).unwrap();
    let server = DeveloperServer::new().with_working_dir(session_path.clone());

    // A real file inside the env base but outside the session jail.
    let probe = env_path.join("sibling.txt");
    std::fs::write(&probe, "original contents").unwrap();
    let probe_str = probe.to_str().unwrap().to_string();

    // --- while the session subdirectory exists -----------------------------
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
    server
        .text_editor(write_params(&probe_str, "CLOBBERED"))
        .await
        .expect_err("sanity: a sibling of the session dir must be refused");
    assert_eq!(
        std::fs::read_to_string(&probe).unwrap(),
        "original contents",
        "sanity: the refused write must not have touched the file"
    );

    // --- the session subdirectory disappears, the env base survives --------
    std::fs::remove_dir_all(&session_path).unwrap();
    assert!(!session_path.exists(), "the session dir really is gone");
    assert!(env_path.is_dir(), "the env base is still there");

    // Collect BOTH outcomes before asserting, so a regression reports the whole
    // picture (relative and absolute) instead of stopping at the first one.
    //
    // A relative path: pre-fix this landed in the env base — one level above the
    // directory the session was actually given.
    let relative = server
        .text_editor(write_params("inside.txt", "after"))
        .await;
    let escaped_relative = env_path.join("inside.txt");
    let relative_escaped = escaped_relative.exists();

    // An absolute path the jail refused a moment ago. If the base had widened to
    // the env base, `probe` now sits inside the jail.
    let absolute = server
        .text_editor(write_params(&probe_str, "CLOBBERED"))
        .await;
    let probe_after = std::fs::read_to_string(&probe).unwrap();

    assert!(
        !relative_escaped,
        "a relative path escaped into BIOROUTER_WORKING_DIR once the session dir \
         vanished: {} was created, so the jail base widened to the parent",
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

    // THE POINT: the previously-refused sibling is still refused.
    assert!(
        absolute.is_err(),
        "the jail must not widen to BIOROUTER_WORKING_DIR: a write outside the \
         session dir succeeded once the session dir vanished"
    );
    assert_eq!(
        probe_after, "original contents",
        "and nothing outside the jail may be written"
    );
}
