//! The machine-wide memory store, reached by the routes that never meet an
//! inspector (issue #63 review, finding 3).
//!
//! The #63 consent gate is a [`ToolInspector`]: it runs in the agent loop, over
//! the tool requests a model returned, before the agent dispatches them. Two
//! production routes dispatch through the very same `ExtensionManager` without
//! ever passing through that loop —
//!
//! * the tool calls a script makes from inside `execute_code`, which go straight
//!   to `ExtensionManager::dispatch_tool_call` from the JS host;
//! * `POST /agent/call_tool`, which does the same from an HTTP handler;
//!
//! — and the branch's answer to the first was a **static scan of the script
//! text**, which the review broke in one line: `const flag = true;
//! retrieve_memories({category:"clinical", is_global:flag})` has no truthy
//! literal after `is_global`, so the scan sees nothing and the read runs.
//!
//! That is the wrong place to fix it. A scan of source text can always be
//! out-computed; a *dispatch* has the real tool name and the real arguments, so
//! the boundary itself can decide. These tests pin the boundary, not the scan:
//! every one of them is written in a shape the static scan cannot see.
//!
//! Run with `cargo test -p biorouter --test global_memory_dispatch_boundary`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::Arc;

use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::object;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::extension_manager::ExtensionManager;

const SESSION: &str = "global-memory-boundary";

/// An `ExtensionManager` with the built-in `memory` extension and the
/// `code_execution` platform extension, both stores redirected into `root` so a
/// test never touches the user's real memories.
///
/// The env has to be set *while* `add_extension` runs: the built-in server
/// resolves `global_memory_dir()` when it is constructed, which is inside that
/// call.
async fn manager(root: &std::path::Path) -> Arc<ExtensionManager> {
    let _env = env_lock::lock_env([
        (
            "BIOROUTER_PATH_ROOT",
            Some(root.to_string_lossy().into_owned()),
        ),
        (
            "BIOROUTER_WORKING_DIR",
            Some(root.join("project").to_string_lossy().into_owned()),
        ),
    ]);

    let session_manager = Arc::new(biorouter::session::SessionManager::new(
        root.join("sessions"),
    ));
    let manager = Arc::new(ExtensionManager::new(
        Arc::new(Mutex::new(None)),
        session_manager,
    ));

    manager
        .add_extension(ExtensionConfig::Builtin {
            name: "memory".to_string(),
            description: "memory".to_string(),
            display_name: Some("Memory".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add memory");

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

/// Dispatch one tool call the way `execute_code`'s JS host and
/// `POST /agent/call_tool` both do, and report `(is_error, text)`. Dispatch
/// errors are reported rather than panicked on, because a refusal is allowed to
/// take either shape.
async fn dispatch(
    manager: &Arc<ExtensionManager>,
    tool: &str,
    arguments: serde_json::Value,
) -> (bool, String) {
    dispatch_as(
        manager,
        biorouter::privacy::CallCapability::public_enforced(),
        tool,
        arguments,
    )
    .await
}

/// [`dispatch`] with an explicit caller capability, so the *private* branch of
/// the built-in `_meta` bit can be exercised through the real dispatcher rather
/// than by hand-building an `rmcp::model::Meta` (issue #56 Gate H).
async fn dispatch_as(
    manager: &Arc<ExtensionManager>,
    cap: biorouter::privacy::CallCapability,
    tool: &str,
    arguments: serde_json::Value,
) -> (bool, String) {
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: tool.to_string().into(),
        arguments: arguments.as_object().cloned(),
    };
    match manager
        .dispatch_tool_call(SESSION, call, cap, CancellationToken::new())
        .await
    {
        Err(e) => (true, e.to_string()),
        Ok(dispatched) => match dispatched.result.await {
            Err(e) => (true, e.message.to_string()),
            Ok(result) => (result.is_error.unwrap_or(false), text_of(&result)),
        },
    }
}

/// Run a script through `execute_code` and report `(is_error, text)`.
async fn exec(manager: &Arc<ExtensionManager>, code: &str) -> (bool, String) {
    dispatch(
        manager,
        "code_execution__execute_code",
        serde_json::Value::Object(object!({ "code": code })),
    )
    .await
}

/// What a refusal has to tell the model, or it is a dead end: that the store is
/// machine-wide, and that an ordinary itemised memory call is the way through.
fn assert_refusal_is_actionable(what: &str, text: &str) {
    let lower = text.to_lowercase();
    assert!(
        lower.contains("memory"),
        "{what}: the refusal does not say what was refused: {text}"
    );
    assert!(
        lower.contains("retrieve_memories")
            || lower.contains("remember_memory")
            || lower.contains("directly")
            || lower.contains("approve"),
        "{what}: the refusal does not tell the model how to proceed, so the \
         feature reads as broken rather than gated: {text}"
    );
}

// ---------------------------------------------------------------------------
// The control: the gate must not have become a blanket ban.
// ---------------------------------------------------------------------------

/// `MemoryServer::new()` now fails closed on global memory, and the built-in
/// extension is the one caller that opts back in. Get that wiring wrong and
/// global memory is dead in the product — so it is pinned here, on the real
/// registry, not asserted about a constructor in isolation.
///
/// This is also the control for every refusal below: it dispatches the same
/// tool, through the same manager, and succeeds.
#[tokio::test]
#[serial_test::serial]
async fn the_builtin_memory_extension_still_reaches_the_global_store() {
    let root = tempfile::tempdir().unwrap();
    let m = manager(root.path()).await;

    let (is_error, text) = dispatch(
        &m,
        "memory__remember_memory",
        serde_json::json!({
            "category": "clinical",
            "data": "cohort 4217 had 12 responders",
            "tags": [],
            "is_global": true
        }),
    )
    .await;
    assert!(
        !is_error,
        "the agent's own memory extension can no longer write global memory, so \
         the consent gate has become a ban: {text}"
    );

    let (is_error, text) = dispatch(
        &m,
        "memory__retrieve_memories",
        serde_json::json!({ "category": "clinical", "is_global": true }),
    )
    .await;
    assert!(
        !is_error,
        "the approved named global read must work: {text}"
    );
    assert!(
        text.contains("cohort 4217"),
        "the approved read returned nothing: {text}"
    );
}

// ---------------------------------------------------------------------------
// execute_code: the inner calls the inspector never sees.
// ---------------------------------------------------------------------------

/// The review's exact counter-example. `is_global` is bound to a variable and
/// the category is assembled at runtime, so nothing in the script text matches
/// the static scan — and the inner call went straight to the extension manager.
#[tokio::test]
#[serial_test::serial]
async fn a_runtime_computed_global_read_inside_execute_code_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let m = manager(root.path()).await;

    // Plant something worth stealing, through the ordinary (gated) route.
    let (is_error, _) = dispatch(
        &m,
        "memory__remember_memory",
        serde_json::json!({
            "category": "clinical",
            "data": "PATIENT-SECRET-8811",
            "tags": [],
            "is_global": true
        }),
    )
    .await;
    assert!(!is_error, "fixture write failed");

    let (_, out) = exec(
        &m,
        r#"
        import { retrieve_memories } from "memory";
        const flag = true;
        const cat = "clin" + "ical";
        const r = retrieve_memories({ category: cat, is_global: flag });
        record_result(r);
        "#,
    )
    .await;

    assert!(
        !out.contains("PATIENT-SECRET-8811"),
        "a script read the machine-wide store with nobody asked: {out}"
    );
    assert_refusal_is_actionable("execute_code named global read", &out);
}

/// Every operation, not just the read the review demonstrated — the server-side
/// floor on this branch closed only the exact bulk read, leaving runtime-
/// assembled named reads, **every** global write, named deletes and the
/// whole-store delete open through this route.
#[tokio::test]
#[serial_test::serial]
async fn every_global_operation_inside_execute_code_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let m = manager(root.path()).await;

    let (is_error, _) = dispatch(
        &m,
        "memory__remember_memory",
        serde_json::json!({
            "category": "clinical",
            "data": "PATIENT-SECRET-8811",
            "tags": [],
            "is_global": true
        }),
    )
    .await;
    assert!(!is_error, "fixture write failed");

    // Each script computes `is_global` rather than writing a literal, so the
    // static scan cannot be what makes any of these fail.
    for (what, code) in [
        (
            "whole-store read",
            r#"import { retrieve_memories } from "memory";
               const g = [true][0];
               record_result(retrieve_memories({ category: "*", is_global: g }));"#,
        ),
        (
            "write",
            r#"import { remember_memory } from "memory";
               const g = !false;
               record_result(remember_memory({ category: "planted", data: "PLANTED-BY-SCRIPT", tags: [], is_global: g }));"#,
        ),
        (
            "named delete",
            r#"import { remove_memory_category } from "memory";
               const g = Boolean(1);
               record_result(remove_memory_category({ category: "clinical", is_global: g }));"#,
        ),
        (
            "entry delete",
            r#"import { remove_specific_memory } from "memory";
               const g = JSON.parse("true");
               record_result(remove_specific_memory({ category: "clinical", memory_content: "PATIENT", is_global: g }));"#,
        ),
        (
            "whole-store delete",
            r#"import { remove_memory_category } from "memory";
               const g = [true][0];
               record_result(remove_memory_category({ category: "*", is_global: g }));"#,
        ),
    ] {
        let (_, out) = exec(&m, code).await;
        assert!(
            !out.contains("PATIENT-SECRET-8811"),
            "{what}: the store's contents came back to the script: {out}"
        );
        assert_refusal_is_actionable(what, &out);
    }

    // Nothing a script asked for reached the disk: the memory it wanted to
    // plant is absent and the memory it wanted to destroy survives.
    let (is_error, text) = dispatch(
        &m,
        "memory__retrieve_memories",
        serde_json::json!({ "category": "clinical", "is_global": true }),
    )
    .await;
    assert!(!is_error, "the control read failed: {text}");
    assert!(
        text.contains("PATIENT-SECRET-8811"),
        "a script deleted a global memory: {text}"
    );

    let (_, planted) = dispatch(
        &m,
        "memory__retrieve_memories",
        serde_json::json!({ "category": "planted", "is_global": true }),
    )
    .await;
    assert!(
        !planted.contains("PLANTED-BY-SCRIPT"),
        "a script wrote to the machine-wide store: {planted}"
    );
}

/// The refusal is scoped to the machine-wide store. Project-local memory lives
/// under the directory the user opened, crosses no session boundary, and is not
/// gated anywhere else either — so scripts keep using it.
#[tokio::test]
#[serial_test::serial]
async fn local_memory_inside_execute_code_still_works() {
    let root = tempfile::tempdir().unwrap();
    let m = manager(root.path()).await;

    let (is_error, out) = exec(
        &m,
        r#"
        import { remember_memory, retrieve_memories } from "memory";
        remember_memory({ category: "development", data: "formats with black", tags: [], is_global: false });
        record_result(retrieve_memories({ category: "development", is_global: false }));
        "#,
    )
    .await;
    assert!(
        !is_error,
        "a local memory round-trip in a script failed: {out}"
    );
    assert!(
        out.contains("formats with black"),
        "the local store stopped answering inside execute_code: {out}"
    );
}

// ---------------------------------------------------------------------------
// Issue #56 Gate H: the capability channel the memory refusal rides on.
// ---------------------------------------------------------------------------

/// A capability whose tier is Private, built the only way production builds one
/// — [`CallCapability::sample`] over a real bound provider. `CallCapability`'s
/// test constructor is `pub(crate)`, and deliberately so: a test that could
/// fabricate a tier would not be testing the thing that produces it.
///
/// Ollama pointed at this machine is Private by `tier()`'s own rule (the same
/// instance the crate's other Gate H tests use); nothing is sent to it here,
/// only its tier is read.
async fn private_capability() -> biorouter::privacy::CallCapability {
    let provider = biorouter::config::with_config_overrides(
        std::collections::HashMap::from([(
            "OLLAMA_HOST".to_string(),
            "http://localhost:11434".to_string(),
        )]),
        biorouter::providers::create(
            "ollama",
            biorouter::model::ModelConfig::new("qwen3").unwrap(),
        ),
    )
    .await
    .expect("build an ollama provider");
    assert!(
        provider.tier().is_private(),
        "the fixture is only meaningful if this provider really is Private"
    );
    let shared = Arc::new(Mutex::new(Some(provider)));
    biorouter::privacy::CallCapability::sample(&shared).await
}

/// Issue #56 Gate H. `remember_memory`'s global refusal reads the caller's
/// capability out of `rmcp::model::Meta`, and the memory unit tests build that
/// meta by hand. Nothing there asserts that the *daemon* actually labels the
/// `memory` built-in — `dispatch_meta` only attaches the bit when
/// `BUILTIN_EXTENSIONS` contains the derived client name, and an absent bit
/// reads as **Public**, which is the permissive answer. Rename the extension
/// key, drop the `with_capability_private` call, or stop deriving `memory` from
/// the `memory__` prefix, and the gate silently stops existing.
///
/// So this pins the whole wire: a private capability in at
/// `dispatch_tool_call`, a refusal out of `remember_memory`, and nothing on
/// disk.
#[tokio::test]
#[serial_test::serial]
async fn a_private_capability_may_not_write_a_global_memory_through_the_real_dispatcher() {
    let root = tempfile::tempdir().unwrap();
    let m = manager(root.path()).await;
    let private = private_capability().await;

    let (is_error, text) = dispatch_as(
        &m,
        private,
        "memory__remember_memory",
        serde_json::json!({
            "category": "cohorts",
            "data": "n=412 T2D patients",
            "tags": [],
            "is_global": true
        }),
    )
    .await;
    assert!(
        is_error,
        "a private chat wrote a machine-wide memory: the capability bit never \
         reached the memory built-in. {text}"
    );
    assert!(
        text.to_lowercase().contains("private"),
        "the refusal has to say why, got: {text}"
    );
    assert!(
        !text.contains("n=412"),
        "a refusal that quotes what it refused is a disclosure with extra \
         steps: {text}"
    );

    // The local write is allowed, and carries the disclosure — also through the
    // real dispatcher, so the `else if caller_is_private` arm is reached by the
    // same wire and not only by a hand-built meta.
    let (is_error, text) = dispatch_as(
        &m,
        private,
        "memory__remember_memory",
        serde_json::json!({
            "category": "cohorts",
            "data": "n=412 T2D patients",
            "tags": [],
            "is_global": false
        }),
    )
    .await;
    assert!(!is_error, "the local write must still be allowed: {text}");
    // AR-3 was CLOSED, not merely disclosed, and that is why this assertion
    // moved. It used to demand the phrase "including one on a public model" —
    // the wording of a *conceded residual*, back when a private chat's local
    // note still reached the system prompt of every session opened in that
    // directory and all the user got was a warning. `compose_instructions` now
    // withholds private-written entries from that prompt (counted, never shown,
    // fetchable only from a chat that is itself on a private model), so the
    // sentence the test was demanding is one the code should no longer say.
    //
    // Assert the two halves of the PROMISE rather than a phrase, so a build that
    // quietly stops honouring it fails here — the direction that matters. A
    // disclosure claiming a public model cannot read something it can is worse
    // than no disclosure, because the user acts on it.
    assert!(
        text.contains("kept OUT of the system prompt"),
        "the disclosure no longer promises the note is withheld from the system \
         prompt: {text}"
    );
    assert!(
        text.contains("public model") && text.contains("cannot read it back"),
        "the disclosure no longer tells the user a public-model chat cannot read \
         this back, which is the half they act on: {text}"
    );

    // And the same two calls on a PUBLIC capability, or the assertions above
    // would also pass a build that refused everybody / disclosed to everybody.
    let (is_error, text) = dispatch(
        &m,
        "memory__remember_memory",
        serde_json::json!({
            "category": "notes",
            "data": "x",
            "tags": [],
            "is_global": true
        }),
    )
    .await;
    assert!(
        !is_error,
        "a public chat's global write must still work: {text}"
    );

    let (_, text) = dispatch(
        &m,
        "memory__remember_memory",
        serde_json::json!({
            "category": "notes",
            "data": "x",
            "tags": [],
            "is_global": false
        }),
    )
    .await;
    assert!(
        !text.contains("including one on a public model"),
        "the public-capability write must keep the shorter copy: {text}"
    );
}
