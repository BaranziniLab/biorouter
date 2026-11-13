//! Issue #72 — cancelling a turn must reap the process tree the turn spawned.
//!
//! The reported repro is a foreground `developer/shell` command invoked from
//! *inside* `code_execution`:
//!
//! ```js
//! import { shell } from "developer";
//! shell({ command: "find \"$HOME\" -type d -name '…' 2>/dev/null" });
//! ```
//!
//! The user presses Stop, the turn ends, and the scan keeps running. These tests
//! model that with a command that leaves a durable trace: it writes a `started`
//! file, sleeps, then writes a `survived` file. If cancellation reaps the tree,
//! `survived` never appears. If anything is orphaned, it does.
//!
//! Both the direct dispatch (`developer__shell`) and the nested one
//! (`code_execution__execute_code` → `developer/shell`) are covered, because they
//! are two different cancellation paths and only one of them was broken.

#![cfg(unix)]

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::object;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::extension_manager::ExtensionManager;

const SESSION: &str = "nested-shell-cancel";

/// How long the orphan sleeps before it would leave its trace. Long enough that
/// a correctly reaped tree cannot possibly reach it, short enough to keep the
/// test quick.
const SURVIVE_AFTER: Duration = Duration::from_secs(4);

/// Build an ExtensionManager with `developer` + `code_execution` enabled — the
/// same wiring `code_execution_integration.rs` uses.
async fn manager() -> Arc<ExtensionManager> {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.keep();
    let session_manager = Arc::new(biorouter::session::SessionManager::new(temp_path));
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

/// A shell command that:
/// 1. forks a **grandchild** (`sh -c …`) which sleeps then touches `survived`,
/// 2. touches `started` so the test knows the tree is up,
/// 3. `wait`s, so the direct child stays alive too.
///
/// The grandchild is the point: SIGKILLing only the direct child (what
/// `kill_on_drop` does) reparents it to init and it runs to completion. Only a
/// process-group kill takes it out.
fn tree_command(dir: &Path) -> String {
    let started = dir.join("started").display().to_string();
    let survived = dir.join("survived").display().to_string();
    let secs = SURVIVE_AFTER.as_secs();
    format!("sh -c 'sleep {secs}; touch \"{survived}\"' & touch \"{started}\"; wait")
}

async fn wait_for(path: &Path, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Assert that after cancelling `token`, nothing from the spawned tree is left
/// alive to write `survived`.
async fn assert_tree_was_reaped(dir: &Path, token: CancellationToken) {
    let started = dir.join("started");
    let survived = dir.join("survived");

    assert!(
        wait_for(&started, Duration::from_secs(20)).await,
        "the shell command never started; the test proves nothing"
    );

    token.cancel();

    // Give the orphan every chance to surface: wait past its sleep.
    tokio::time::sleep(SURVIVE_AFTER + Duration::from_secs(3)).await;

    assert!(
        !survived.exists(),
        "issue #72: cancelling the turn left a descendant of the shell command \
         running: it woke up after the turn ended and wrote {}",
        survived.display()
    );
}

/// The nested path from the bug report: `execute_code` → `developer/shell`.
#[tokio::test]
async fn cancelling_a_turn_reaps_a_shell_tree_spawned_inside_code_execution() {
    let manager = manager().await;
    let dir = tempfile::tempdir().unwrap();
    let command = tree_command(dir.path());
    let code = format!(
        "import {{ shell }} from \"developer\";\nshell({{ command: {} }});\n",
        serde_json::to_string(&command).unwrap()
    );

    let token = CancellationToken::new();
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": code })),
    };
    let dispatched = manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            token.clone(),
        )
        .await
        .expect("dispatch");
    let call_task = tokio::spawn(dispatched.result);

    assert_tree_was_reaped(dir.path(), token.clone()).await;

    // The turn must also actually end — a cancel that hangs is its own bug.
    let ended = tokio::time::timeout(Duration::from_secs(10), call_task).await;
    assert!(
        ended.is_ok(),
        "the cancelled execute_code call never returned"
    );
}

/// The other half of Stop: `POST /agent/stop` trips the turn's token **and**
/// evicts the session, which drops the extension and closes its transport.
///
/// That teardown races the cancellation notification, so the reaping cannot
/// depend on the notification arriving first. Dropping the client ends the
/// extension's serve loop, whose drop guard cancels the request-scoped token
/// rmcp hands every tool call — so the shell tool has to be watching that token
/// too, not only the one the notification trips.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tearing_down_the_extension_reaps_a_running_shell_tree() {
    let manager = manager().await;
    let dir = tempfile::tempdir().unwrap();

    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "developer__shell".into(),
        arguments: Some(object!({ "command": tree_command(dir.path()) })),
    };
    let dispatched = manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch");
    let call_task = tokio::spawn(dispatched.result);

    assert!(
        wait_for(&dir.path().join("started"), Duration::from_secs(20)).await,
        "the shell command never started; the test proves nothing"
    );

    // No cancellation at all — just take the extension away, as session
    // eviction does.
    manager
        .remove_extension("developer")
        .await
        .expect("remove developer");
    call_task.abort();

    tokio::time::sleep(SURVIVE_AFTER + Duration::from_secs(3)).await;
    assert!(
        !dir.path().join("survived").exists(),
        "issue #72: tearing the extension down left the shell command's tree \
         running with nobody able to reach it"
    );
}

/// The direct path, for contrast: a plain `developer__shell` dispatch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_turn_reaps_a_shell_tree_spawned_directly() {
    let manager = manager().await;
    let dir = tempfile::tempdir().unwrap();

    let token = CancellationToken::new();
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "developer__shell".into(),
        arguments: Some(object!({ "command": tree_command(dir.path()) })),
    };
    let dispatched = manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            token.clone(),
        )
        .await
        .expect("dispatch");
    let call_task = tokio::spawn(dispatched.result);

    assert_tree_was_reaped(dir.path(), token.clone()).await;

    let ended = tokio::time::timeout(Duration::from_secs(10), call_task).await;
    assert!(ended.is_ok(), "the cancelled shell call never returned");
}
