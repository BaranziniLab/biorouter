//! #68, the third route to the same widening: keep the *path* of the session
//! working directory alive but change what it points at.
//!
//! The jail refuses to move to another directory when its base disappears —
//! but it only ever asked "does this path name a directory?" and then
//! canonicalized it. `Path::is_dir` follows symlinks, so deleting
//! `/parent/session` and dropping a symlink to `/parent` in its place answers
//! yes, canonicalization then resolves the base to `/parent`, and every sibling
//! the jail refused a moment earlier is suddenly inside it. Nothing about the
//! base the caller sanctioned changed textually; the directory it denotes did.
//!
//! Driven through the public `text_editor` tool with a real symlink swap, so it
//! pins what a user would see rather than an internal helper's return value.
//! Owns the process environment, so it is a single test in its own integration
//! binary.
#![cfg(unix)]

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use std::os::unix::fs::symlink;
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

/// Restores `BIOROUTER_WORKING_DIR` on drop, so a panicking assertion cannot
/// leave the variable set for anything that runs after it.
struct RestoreEnv(Option<String>);

impl Drop for RestoreEnv {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => std::env::set_var("BIOROUTER_WORKING_DIR", v),
            None => std::env::remove_var("BIOROUTER_WORKING_DIR"),
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn text_editor_jail_survives_a_symlink_swap_of_its_base() {
    let _restore = RestoreEnv(std::env::var("BIOROUTER_WORKING_DIR").ok());
    // The bound base is the only thing under test; no env fallback in play.
    std::env::remove_var("BIOROUTER_WORKING_DIR");

    let parent_dir = tempfile::tempdir().unwrap();
    let parent: PathBuf = std::fs::canonicalize(parent_dir.path()).unwrap();
    let session = parent.join("session");
    std::fs::create_dir(&session).unwrap();

    let server = DeveloperServer::new().with_working_dir(session.clone());

    // A real file one level above the jail.
    let sibling = parent.join("sibling.txt");
    std::fs::write(&sibling, "original contents").unwrap();
    let sibling_str = sibling.to_str().unwrap().to_string();

    // --- while the session directory is a real directory -------------------
    server
        .text_editor(write_params("inside.txt", "hello"))
        .await
        .expect("a relative path inside the session dir must be writable");
    server
        .text_editor(write_params(&sibling_str, "CLOBBERED"))
        .await
        .expect_err("sanity: a sibling of the session dir must be refused");
    assert_eq!(
        std::fs::read_to_string(&sibling).unwrap(),
        "original contents",
        "sanity: the refused write must not have touched the file"
    );

    // --- the directory is replaced by a symlink to its own parent ----------
    std::fs::remove_dir_all(&session).unwrap();
    symlink(&parent, &session).unwrap();
    assert!(
        session.is_dir(),
        "precondition: the swapped-in symlink still answers is_dir()"
    );
    assert_eq!(
        std::fs::canonicalize(&session).unwrap(),
        parent,
        "precondition: the base now resolves to its own parent"
    );

    // Collect both outcomes before asserting, so a regression reports the whole
    // picture (absolute and relative) instead of stopping at the first one.
    let absolute = server
        .text_editor(write_params(&sibling_str, "CLOBBERED"))
        .await;
    let sibling_after = std::fs::read_to_string(&sibling).unwrap();

    let relative = server
        .text_editor(write_params("escaped.txt", "after"))
        .await;
    let escaped = parent.join("escaped.txt");
    let relative_escaped = escaped.exists();

    // THE POINT: the previously-refused sibling is still refused.
    assert!(
        absolute.is_err(),
        "the jail widened to the parent: a write outside the session dir \
         succeeded once the session dir was replaced by a symlink to it"
    );
    assert_eq!(
        sibling_after, "original contents",
        "and nothing outside the jail may be written"
    );
    assert!(
        !relative_escaped,
        "a relative path escaped into the parent once the session dir was \
         replaced by a symlink to it: {} was created",
        escaped.display()
    );
    let err = relative
        .expect_err("with a base that no longer denotes the same directory, the call must fail");
    assert!(
        err.message.contains(&session.display().to_string()),
        "the error must name the base that changed underneath us, got: {}",
        err.message
    );
}
