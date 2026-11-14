//! The session database, reached from inside `execute_code` — the door the
//! inspector chain never sees (issue #56).
//!
//! `SessionStoreInspector` is a [`ToolInspector`]: it runs in the agent loop,
//! over the tool requests a model returned, before the agent dispatches them.
//! The tool calls a script makes from inside `execute_code` do not go through
//! that loop at all — the JS host hands them straight to
//! `ExtensionManager::dispatch_tool_call` — so nothing inspected them, and the
//! module's compensation was a scan of the *script text* for a literal store
//! path. One line walks past it:
//!
//! ```js
//! const p = root + "/data/sessions/" + ["sessions", "db"].join(".");
//! shell({ command: "cat " + p });
//! ```
//!
//! There is no store path anywhere in that source. The same evasion is what made
//! `global_memory`'s boundary refusals necessary, and this module's header
//! declared the gap openly rather than closing it.
//!
//! Every test below is written in a shape the script scan cannot see, so what
//! they pin is the **boundary**, not the scan. The store here is a planted file
//! under a temp root (`BIOROUTER_PATH_ROOT`), never the developer's own, and the
//! control at the bottom reads a decoy beside it successfully — which is what
//! makes "the marker did not come back" evidence of a refusal rather than of a
//! harness that cannot read files at all.
//!
//! Run with `cargo test -p biorouter --test session_store_dispatch_boundary`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::path::Path;
use std::sync::Arc;

use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::object;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::extension_manager::ExtensionManager;

const SESSION: &str = "session-store-boundary";

/// Planted inside the fake session database. Every conversation on the machine
/// is in the real one; this stands in for the first line of any of them.
const MARKER_IN_THE_STORE: &str = "TRANSCRIPT-OF-EVERY-CHAT-4471";

/// Planted in a decoy file *beside* the store directory, for the control.
const MARKER_IN_A_DECOY: &str = "ORDINARY-PROJECT-FILE-9052";

/// The store as this test's `BIOROUTER_PATH_ROOT` resolves it: `Paths::data_dir()`
/// joined with `sessions/sessions.db`, which is what `session_store.rs` derives
/// and what `SessionStorage::new` joins.
fn store_db(root: &Path) -> std::path::PathBuf {
    root.join("data").join("sessions").join("sessions.db")
}

/// Plant a fake store (and its WAL sibling) plus one ordinary file beside it.
fn plant(root: &Path) {
    let db = store_db(root);
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    std::fs::write(&db, format!("sqlite-ish {MARKER_IN_THE_STORE}\n")).unwrap();
    let mut wal = db.clone().into_os_string();
    wal.push("-wal");
    std::fs::write(
        std::path::PathBuf::from(wal),
        format!("newest turns {MARKER_IN_THE_STORE}\n"),
    )
    .unwrap();

    let decoy = root.join("data").join("notes.txt");
    std::fs::write(&decoy, format!("{MARKER_IN_A_DECOY}\n")).unwrap();
}

/// An `ExtensionManager` with the built-in `developer` extension (the tools that
/// take a path) and the `code_execution` platform extension.
async fn manager(root: &Path) -> Arc<ExtensionManager> {
    let session_manager = Arc::new(biorouter::session::SessionManager::new(
        root.join("sessions-runtime"),
    ));
    let manager = Arc::new(ExtensionManager::new(
        Arc::new(Mutex::new(None)),
        session_manager,
    ));

    manager
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "developer".to_string(),
            display_name: Some("Developer".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add developer");

    manager
        .add_extension(ExtensionConfig::Platform {
            name: "code_execution".to_string(),
            description: "Execute JavaScript code in a sandboxed environment".to_string(),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add code_execution");

    manager
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run a script through `execute_code` and report its text, whether it
/// succeeded or was refused.
async fn exec(manager: &Arc<ExtensionManager>, code: &str) -> String {
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": code })),
    };
    match manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
    {
        Err(e) => e.to_string(),
        Ok(dispatched) => match dispatched.result.await {
            Err(e) => e.message.to_string(),
            Ok(result) => text_of(&result),
        },
    }
}

/// What a refusal has to tell the model, or it is a dead end: that this is
/// about the transcript store, and that `chatrecall` is the route that works.
fn assert_refusal_is_actionable(what: &str, text: &str) {
    assert!(
        text.contains("session database") || text.contains("transcript"),
        "{what}: the refusal does not say what was refused: {text}"
    );
    assert!(
        text.contains("chatrecall"),
        "{what}: the refusal does not name the route that still works, so the model reports \
         the product as broken rather than gated: {text}"
    );
}

/// A temp root that is this process's Biorouter data dir for the whole test, so
/// the store the gate resolves is the planted one and never the developer's.
///
/// The guard is returned rather than dropped inside a setup helper: `Paths` reads
/// `BIOROUTER_PATH_ROOT` on **every** call, so a guard released before the
/// dispatch would silently move the barrier back onto the real store — the test
/// would still pass, and would be testing something else.
fn sandbox() -> (tempfile::TempDir, env_lock::EnvGuard<'static>) {
    let root = tempfile::tempdir().unwrap();
    let guard = env_lock::lock_env([
        (
            "BIOROUTER_PATH_ROOT",
            Some(root.path().to_string_lossy().into_owned()),
        ),
        (
            "BIOROUTER_WORKING_DIR",
            Some(root.path().to_string_lossy().into_owned()),
        ),
    ]);
    plant(root.path());
    (root, guard)
}

// ---------------------------------------------------------------------------
// The control, first: the harness really can read a file this way.
// ---------------------------------------------------------------------------

/// Everything below asserts that a marker did **not** come back. That assertion
/// is worthless unless a marker can come back, so this reads an ordinary file
/// beside the store, through the same script, the same tool and the same
/// dispatcher — and it must arrive.
///
/// It is also the scope check: the barrier is the store directory, not the data
/// directory it sits in, and a gate that swallowed the whole config root would
/// break every legitimate script on the machine.
#[tokio::test]
#[serial_test::serial]
async fn an_ordinary_file_beside_the_store_still_reads_from_a_script() {
    let (root, _env) = sandbox();
    let m = manager(root.path()).await;

    let decoy = root.path().join("data").join("notes.txt");
    let out = exec(
        &m,
        &format!(
            r#"import {{ shell }} from "developer";
               const p = {decoy:?};
               record_result(shell({{ command: "cat " + p }}));"#
        ),
    )
    .await;

    assert!(
        out.contains(MARKER_IN_A_DECOY),
        "a script could not read an ordinary file, so every refusal below proves nothing: {out}"
    );
}

// ---------------------------------------------------------------------------
// The boundary.
// ---------------------------------------------------------------------------

/// The evasion, exactly. The path is assembled at runtime from pieces, so there
/// is no store path in the script text for the inspector's scan to find — and
/// the inner `shell` call never met an inspector anyway.
#[tokio::test]
#[serial_test::serial]
async fn a_runtime_assembled_store_path_inside_execute_code_is_refused() {
    let (root, _env) = sandbox();
    let m = manager(root.path()).await;

    let data_dir = root.path().join("data");
    let out = exec(
        &m,
        &format!(
            r#"import {{ shell }} from "developer";
               const base = {data_dir:?};
               const dir = ["sess", "ions"].join("");
               const file = ["sessions", "db"].join(".");
               const p = base + "/" + dir + "/" + file;
               record_result(shell({{ command: "cat " + p }}));"#
        ),
    )
    .await;

    assert!(
        !out.contains(MARKER_IN_THE_STORE),
        "a script read every conversation on this machine, with no inspector anywhere: {out}"
    );
    assert_refusal_is_actionable("runtime-assembled cat", &out);
}

/// Every shape that reaches the same bytes, each computed rather than written:
/// the WAL sibling (which holds the *newest* turns), a path argument rather than
/// a command line, a copy out of the directory, and a write into it.
#[tokio::test]
#[serial_test::serial]
async fn every_computed_route_to_the_store_inside_execute_code_is_refused() {
    let (root, _env) = sandbox();
    let m = manager(root.path()).await;

    let data_dir = root.path().join("data");
    let base = format!("const base = {data_dir:?}; const s = base + \"/\" + \"sessions\";");

    for (what, code) in [
        (
            "write-ahead log",
            format!(
                r#"import {{ shell }} from "developer";
                   {base}
                   record_result(shell({{ command: "cat " + s + "/sessions.db" + "-wal" }}));"#
            ),
        ),
        (
            "path argument, not a command",
            format!(
                r#"import {{ text_editor }} from "developer";
                   {base}
                   record_result(text_editor({{ command: "view", path: s + "/sessions.db" }}));"#
            ),
        ),
        (
            "copy the whole directory out",
            format!(
                r#"import {{ shell }} from "developer";
                   {base}
                   record_result(shell({{ command: "tar cf - " + s + " | wc -c" }}));"#
            ),
        ),
        (
            "cd into it and open by bare name",
            format!(
                r#"import {{ shell }} from "developer";
                   {base}
                   record_result(shell({{ command: "cd " + s + " && cat sessions.db" }}));"#
            ),
        ),
        (
            "overwrite it",
            format!(
                r#"import {{ shell }} from "developer";
                   {base}
                   record_result(shell({{ command: "echo wiped > " + s + "/sessions.db" }}));"#
            ),
        ),
    ] {
        let out = exec(&m, &code).await;
        assert!(
            !out.contains(MARKER_IN_THE_STORE),
            "{what}: the store's contents came back to the script: {out}"
        );
        assert_refusal_is_actionable(what, &out);
    }

    // Nothing a script asked for reached the disk: the transcript it wanted to
    // destroy is still there, byte for byte.
    let db = store_db(root.path());
    assert!(
        std::fs::read_to_string(&db)
            .unwrap()
            .contains(MARKER_IN_THE_STORE),
        "a script overwrote the session database"
    );
}
