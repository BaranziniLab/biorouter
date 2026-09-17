//! The machine-wide memory store, reached with a *file* tool rather than a
//! memory tool (issue #63 review, finding 2).
//!
//! The consent gate recognises four tool names. The store is four text files in
//! a directory the agent can otherwise address like any other, so the gate was
//! closed and the window was open: `text_editor view
//! ~/.config/biorouter/memory/clinical.txt` reads exactly what
//! `retrieve_memories(category="clinical", is_global=true)` would have put to
//! the user, and `webdocuments cache --delete` removes it. In Auto mode
//! the developer server also relaxes its containment jail, so an absolute path
//! anywhere resolves.
//!
//! The reviewer's verdict — "tool-name scanning cannot protect a machine-wide
//! file store while generic filesystem access remains available" — is closed
//! here at the *storage* boundary: the servers that take a path refuse the
//! resolved global-memory root, whatever tool, whatever mode, and whatever route
//! reached them. That is unforgeable in a way an argument scan is not.
//!
//! **Scope.** This closes the memory root, not the general filesystem barrier —
//! an unsandboxed `developer__shell` still `cat`s any file on the machine, and
//! that is issue #56's separate design, deliberately not built here. The shell's
//! *literal* references are denied one layer up, by the agent's global-memory
//! inspector.

use biorouter_mcp::developer::rmcp_developer::TextEditorParams;
use biorouter_mcp::webdocuments::{CacheCommand, CacheParams};
use biorouter_mcp::{global_memory_dir, DeveloperServer, WebDocumentsServer};
use rmcp::handler::server::wrapper::Parameters;

const SECRET: &str = "PATIENT-SECRET-8811";

/// Point the whole process's memory store at a temp root and plant one global
/// memory in it. Returns the guard, the store dir and the planted file.
fn planted_store(
    root: &std::path::Path,
) -> (
    env_lock::EnvGuard<'static>,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let guard = env_lock::lock_env([(
        "BIOROUTER_PATH_ROOT",
        Some(root.to_string_lossy().into_owned()),
    )]);
    let store = global_memory_dir();
    std::fs::create_dir_all(&store).unwrap();
    let file = store.join("clinical.txt");
    std::fs::write(&file, format!("{SECRET}\n")).unwrap();
    (guard, store, file)
}

fn text_editor(command: &str, path: &str) -> Parameters<TextEditorParams> {
    Parameters(TextEditorParams {
        path: path.to_string(),
        command: command.to_string(),
        diff: None,
        view_range: None,
        file_text: Some("REPLACED BY A FILE TOOL\n".to_string()),
        old_str: Some(SECRET.to_string()),
        new_str: Some("REDACTED".to_string()),
        insert_line: Some(1),
    })
}

/// `text_editor view` on a global memory file is the review's first example:
/// the same disclosure the consent card exists to put to the user, taken with a
/// tool the gate does not know about, in the mode that relaxes the jail.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn text_editor_cannot_read_the_global_memory_store() {
    let root = tempfile::tempdir().unwrap();
    let (_env, _store, file) = planted_store(root.path());
    let server = DeveloperServer::new().with_working_dir(root.path().to_path_buf());
    let result = server
        .text_editor(text_editor("view", &file.to_string_lossy()))
        .await;

    let text = format!("{result:?}");
    assert!(
        !text.contains(SECRET),
        "a file tool read the machine-wide memory store: {text}"
    );
    assert!(
        result.is_err(),
        "the read was not refused, it merely returned nothing useful: {text}"
    );
}

/// Reading is the disclosure, but writing and deleting are worse: a memory the
/// user never saw can be silently rewritten, or the store emptied, with no card
/// and no record that it was memory at all.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn text_editor_cannot_write_or_edit_the_global_memory_store() {
    let root = tempfile::tempdir().unwrap();
    let (_env, store, file) = planted_store(root.path());
    let server = DeveloperServer::new().with_working_dir(root.path().to_path_buf());
    for command in ["write", "str_replace", "insert", "undo_edit"] {
        let result = server
            .text_editor(text_editor(command, &file.to_string_lossy()))
            .await;
        assert!(
            result.is_err(),
            "text_editor {command} was allowed into the machine-wide memory \
             store: {result:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("{SECRET}\n"),
        "a file tool changed a global memory"
    );

    // A category that does not exist yet is the same store.
    let fresh = store.join("planted.txt");
    let result = server
        .text_editor(text_editor("write", &fresh.to_string_lossy()))
        .await;
    assert!(
        result.is_err() && !fresh.exists(),
        "a file tool created a new global memory category: {result:?}"
    );
}

/// The store is refused as a *place*, so the directory itself and anything
/// under it are covered — not just the `.txt` files that happen to be in it
/// today.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn the_whole_store_directory_is_refused_not_just_its_files() {
    let root = tempfile::tempdir().unwrap();
    let (_env, store, _file) = planted_store(root.path());
    let server = DeveloperServer::new().with_working_dir(root.path().to_path_buf());
    for path in [
        store.clone(),
        store.join("nested").join("deep.txt"),
        // The same place spelled with a traversal.
        store.join("..").join("memory").join("clinical.txt"),
    ] {
        let result = server
            .text_editor(text_editor("view", &path.to_string_lossy()))
            .await;
        assert!(
            result.is_err(),
            "{} was not recognised as the memory store: {result:?}",
            path.display()
        );
    }
}

/// The barrier is scoped to the store. Everything else on the filesystem is
/// exactly as reachable as it was, or this is an outage rather than a fix.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn ordinary_files_are_untouched_by_the_barrier() {
    let root = tempfile::tempdir().unwrap();
    let (_env, store, _file) = planted_store(root.path());

    // A sibling of the store, and a file whose name merely mentions memory.
    let sibling = store.parent().unwrap().join("memories-notes.txt");
    std::fs::write(&sibling, "ORDINARY\n").unwrap();
    let elsewhere = root.path().join("project").join("memory.txt");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    std::fs::write(&elsewhere, "ORDINARY\n").unwrap();

    let server = DeveloperServer::new().with_working_dir(root.path().to_path_buf());
    for path in [sibling, elsewhere] {
        let result = server
            .text_editor(text_editor("view", &path.to_string_lossy()))
            .await;
        assert!(
            result.is_ok(),
            "{} is not in the memory store and must still be readable: {result:?}",
            path.display()
        );
    }

    // And the local (project) store is not the machine-wide one.
    let local = root
        .path()
        .join("project")
        .join(".biorouter")
        .join("memory");
    std::fs::create_dir_all(&local).unwrap();
    let local_file = local.join("development.txt");
    std::fs::write(&local_file, "ORDINARY\n").unwrap();
    let result = DeveloperServer::new()
        .with_working_dir(root.path().to_path_buf())
        .text_editor(text_editor("view", &local_file.to_string_lossy()))
        .await;
    assert!(
        result.is_ok(),
        "project-local memory crosses no session boundary and must stay \
         readable: {result:?}"
    );
}

/// `webdocuments`'s cache tool takes any path the model supplies and
/// reads or deletes it. It is a second generic file tool with the same reach
/// and the same blind spot.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn the_cache_tool_cannot_read_or_delete_the_global_memory_store() {
    let root = tempfile::tempdir().unwrap();
    let (_env, _store, file) = planted_store(root.path());

    let server = WebDocumentsServer::new();

    let viewed = server
        .cache(Parameters(CacheParams {
            command: CacheCommand::View,
            path: Some(file.to_string_lossy().into_owned()),
        }))
        .await;
    let text = format!("{viewed:?}");
    assert!(
        !text.contains(SECRET),
        "the cache tool read the machine-wide memory store: {text}"
    );
    assert!(viewed.is_err(), "the read was not refused: {text}");

    let deleted = server
        .cache(Parameters(CacheParams {
            command: CacheCommand::Delete,
            path: Some(file.to_string_lossy().into_owned()),
        }))
        .await;
    assert!(
        deleted.is_err(),
        "the cache tool was allowed to delete a global memory: {deleted:?}"
    );
    assert!(file.exists(), "a global memory was deleted by a cache tool");
}
