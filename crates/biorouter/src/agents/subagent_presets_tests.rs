//! J-P03: the three spawn presets, end to end. Every spawn test here starts a
//! real child through the tool's own JSON arguments under a parent holding
//! Todo (a writer) and Chat Recall (a reader) unless it says otherwise, and
//! reads what the child's model was really offered and what came back from
//! the calls it made, so none of them depends on how the presets are
//! implemented. The door tests (`a_preset_child_cannot_turn_on_*`,
//! `another_chat_cannot_add_*`, `an_ext_marker_cannot_attach_*`) fail on a
//! tree without the presets, where the `preset` field is ignored and each door
//! attaches the extension.

use super::*;
use crate::agents::subagent_presets::SEALED_REFUSAL;
use crate::conversation::message::Message;
use std::path::Path;
use std::sync::Arc;

/// A child's model that records the tools it is offered and the messages it
/// sees, makes `calls` on its first request, writes `write_into` (a file
/// standing in for another process, or a tool that claims to only read)
/// during that request, and then finishes.
#[derive(Default)]
struct ScriptedChild {
    systems: std::sync::Mutex<Vec<String>>,
    offered: std::sync::Mutex<Vec<Vec<String>>>,
    seen: std::sync::Mutex<Vec<String>>,
    calls: Vec<(String, Value)>,
    write_into: Option<std::path::PathBuf>,
}

impl ScriptedChild {
    fn calling(calls: Vec<(&str, Value)>) -> Self {
        Self {
            calls: calls
                .into_iter()
                .map(|(name, arguments)| (name.to_string(), arguments))
                .collect(),
            ..Self::default()
        }
    }

    fn first_offer(&self) -> Vec<String> {
        self.offered
            .lock()
            .unwrap()
            .first()
            .cloned()
            .expect("the child reached its model")
    }

    fn every_offer(&self) -> Vec<String> {
        self.offered.lock().unwrap().concat()
    }

    /// The answer the child's model was shown for its call `id`, from the
    /// last request it saw.
    fn tool_response(&self, id: &str) -> String {
        fn find(value: &Value, id: &str) -> Option<String> {
            match value {
                Value::Object(map) => {
                    if map.get("id").and_then(Value::as_str) == Some(id)
                        && map.contains_key("toolResult")
                    {
                        return Some(value.to_string());
                    }
                    map.values().find_map(|inner| find(inner, id))
                }
                Value::Array(items) => items.iter().find_map(|inner| find(inner, id)),
                _ => None,
            }
        }
        let seen: Value = serde_json::from_str(&self.last_seen()).unwrap_or_default();
        find(&seen, id).unwrap_or_else(|| panic!("no answer to {id} in {}", self.last_seen()))
    }

    fn last_seen(&self) -> String {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl crate::providers::base::Provider for ScriptedChild {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new(
            "scripted-child",
            "Scripted child",
            "",
            "scripted-child-model",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "scripted-child"
    }

    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("scripted-child-model")
    }

    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> std::result::Result<
        (Message, crate::providers::base::ProviderUsage),
        crate::providers::errors::ProviderError,
    > {
        self.systems.lock().unwrap().push(system.to_string());
        let first = {
            let mut offered = self.offered.lock().unwrap();
            offered.push(tools.iter().map(|tool| tool.name.to_string()).collect());
            offered.len() == 1
        };
        self.seen
            .lock()
            .unwrap()
            .push(serde_json::to_string(messages).unwrap_or_default());
        let usage = crate::providers::base::ProviderUsage::new(
            "scripted-child-model".into(),
            crate::providers::base::Usage::default(),
        );
        if first {
            if let Some(path) = &self.write_into {
                std::fs::write(path, "written while the subagent ran").unwrap();
            }
            if !self.calls.is_empty() {
                let mut message = Message::assistant();
                for (index, (name, arguments)) in self.calls.iter().enumerate() {
                    message = message.with_tool_request(
                        format!("call-{index}"),
                        Ok(rmcp::model::CallToolRequestParams {
                            name: name.clone().into(),
                            arguments: arguments.as_object().cloned(),
                            meta: None,
                            task: None,
                        }),
                    );
                }
                return Ok((message, usage));
            }
        }
        Ok((
            Message::assistant().with_text("Checked. Nothing to add."),
            usage,
        ))
    }
}

fn bundled(name: &str) -> crate::agents::ExtensionConfig {
    crate::agents::extension_manager::resolve_bundled_extension(name)
        .unwrap_or_else(|| panic!("{name} is a bundled capability"))
        .into_config(String::new())
}

/// Clears the process's workspace services on every exit, as the spawn tests
/// beside these do.
struct ClearWorkspaceServices;
impl Drop for ClearWorkspaceServices {
    fn drop(&mut self) {
        crate::workspace_services::clear_test_override();
    }
}

/// One finished spawn: the tool result, the store it ran in, and, for a
/// background spawn, the result its handle settled.
struct Spawned {
    result: std::result::Result<rmcp::model::CallToolResult, ErrorData>,
    sm: Arc<crate::session::SessionManager>,
    parent: String,
    background: Option<crate::agents::subagent_result::SubagentResult>,
}

impl Spawned {
    /// The one child this spawn started.
    async fn child(&self) -> crate::session::Session {
        let mut rows = self.sm.list_subagent_children(&self.parent).await.unwrap();
        assert_eq!(rows.len(), 1, "one child");
        let id = rows.remove(0).session.id;
        self.sm.get_session(&id, false).await.unwrap()
    }
}

/// How a spawn is started: from which roster, with which sub-workflows, and
/// on which path.
#[derive(Default)]
struct Setup {
    /// The parent's live roster; Todo and Chat Recall when unset.
    roster: Option<Vec<crate::agents::ExtensionConfig>>,
    sub_workflows: HashMap<String, SubWorkflow>,
    /// The supervised background path every model-facing spawn takes, waited
    /// on through the child's handle. Otherwise the blocking path.
    background: bool,
    /// Run with no person to answer a card, so anything that would wait for
    /// one is refused at once instead.
    no_person: bool,
}

async fn spawn(child: Arc<ScriptedChild>, body: Value, working: &Path) -> Spawned {
    spawn_with(child, body, working, Setup::default()).await
}

/// Spawn `body` (plus `visible: false`) from a parent with `working` as its
/// working folder, and wait for the child to finish.
async fn spawn_with(
    child: Arc<ScriptedChild>,
    body: Value,
    working: &Path,
    setup: Setup,
) -> Spawned {
    crate::workspace_services::set_for_tests(None);
    let _workspace_services = ClearWorkspaceServices;
    let _serialised = super::tests::QUEUE_DEPTH_TESTS.lock().await;

    let store = tempfile::TempDir::new().unwrap();
    let sm = Arc::new(crate::session::SessionManager::new(
        store.path().to_path_buf(),
    ));
    let parent = sm
        .create_session(
            working.to_path_buf(),
            "a chat that delegates".to_string(),
            crate::session::session_manager::SessionType::User,
        )
        .await
        .unwrap()
        .id;
    let config = AgentConfig::new(
        sm.clone(),
        crate::config::permission::PermissionManager::instance(),
        None,
        crate::config::BioRouterMode::Auto,
    );
    let watched = Arc::clone(&child);
    let provider: Arc<dyn crate::providers::base::Provider> = child;
    let roster = setup
        .roster
        .unwrap_or_else(|| vec![bundled("todo"), bundled("chatrecall")]);
    let task_config = TaskConfig::new(provider, &parent, working, roster);
    let mut body = body;
    body["visible"] = json!(false);
    if !setup.background {
        body["background"] = json!(false);
    }
    let run = handle_subagent_tool_inner(
        &config,
        body,
        task_config,
        setup.sub_workflows,
        working.to_path_buf(),
        None,
        setup.background.then_some(true),
    )
    .result;
    let run: std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send>> = if setup.no_person {
        Box::pin(crate::user_surface::without_human_surface(run))
    } else {
        Box::pin(run)
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), run)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the spawn did not return; the child's model was asked {} times, last with: {}",
                watched.offered.lock().unwrap().len(),
                watched.last_seen()
            )
        });
    let background = match (&result, setup.background) {
        (Ok(_), true) => {
            let handle = crate::agents::subagent_handle::list_for_session(&parent)
                .into_iter()
                .next()
                .expect("a background spawn registers its handle before it returns");
            Some(
                tokio::time::timeout(
                    std::time::Duration::from_secs(60),
                    handle.wait_until_complete(),
                )
                .await
                .expect("the background child finishes"),
            )
        }
        _ => None,
    };
    // The store must outlive the assertions on its rows.
    std::mem::forget(store);
    Spawned {
        result,
        sm,
        parent,
        background,
    }
}

fn is_platform_tool(name: &str) -> bool {
    name.starts_with("platform__")
}

fn is_todo_tool(name: &str) -> bool {
    name.starts_with("todo__")
}

fn is_chat_recall_tool(name: &str) -> bool {
    name.ends_with("__chatrecall")
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The tool-less reviewer: no extension and no Biorouter tool. It reads only
/// what its instructions carry. (`extensions: []` alone still left every
/// `platform__*` tool, which is why this preset exists.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_no_tools_child_is_offered_no_tool_at_all() {
    let working = tempfile::TempDir::new().unwrap();
    let child = Arc::new(ScriptedChild::default());
    let spawned = spawn(
        child.clone(),
        json!({ "instructions": "Review this paragraph: ...", "preset": "no_tools" }),
        working.path(),
    )
    .await;
    let finished = spawned.result.expect("a no_tools spawn runs");
    assert_ne!(finished.is_error, Some(true), "{:?}", finished.content);
    assert_eq!(
        child.every_offer(),
        Vec::<String>::new(),
        "a no_tools child is offered nothing, not even Biorouter's own tools"
    );
    let system = child
        .systems
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(
        system.contains("You were started with no tools (the no_tools preset)"),
        "the child is told why: {system}"
    );
    let told = format!("{:?}", finished.content);
    assert!(
        told.contains("Preset no_tools"),
        "and so is its parent: {told}"
    );
}

/// The allowlist: exactly the named extensions of this chat's live roster,
/// named by the label a person sees, and none of Biorouter's own tools.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn an_only_extensions_child_is_offered_exactly_the_named_extensions() {
    let working = tempfile::TempDir::new().unwrap();
    let child = Arc::new(ScriptedChild::default());
    let spawned = spawn(
        child.clone(),
        json!({
            "instructions": "Find the chat where we chose the cohort.",
            "preset": "only_extensions",
            "extensions": ["Chat Recall"],
        }),
        working.path(),
    )
    .await;
    let finished = spawned.result.expect("an only_extensions spawn runs");
    assert_ne!(finished.is_error, Some(true), "{:?}", finished.content);
    let offered = child.first_offer();
    assert!(
        offered.iter().any(|name| is_chat_recall_tool(name)),
        "the named extension's tool is offered: {offered:?}"
    );
    assert!(
        offered.iter().all(|name| is_chat_recall_tool(name)),
        "and nothing else, Biorouter's own tools included: {offered:?}"
    );
}

/// `only_extensions` composes with `remove_extensions`: the list is applied,
/// then the removal, so the child holds what is left and nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn only_extensions_composes_with_remove_extensions() {
    let working = tempfile::TempDir::new().unwrap();
    let child = Arc::new(ScriptedChild::default());
    let spawned = spawn(
        child.clone(),
        json!({
            "instructions": "Find the chat where we chose the cohort.",
            "preset": "only_extensions",
            "extensions": ["Chat Recall", "Todo"],
            "remove_extensions": ["Todo"],
        }),
        working.path(),
    )
    .await;
    spawned.result.expect("the spawn runs");
    let offered = child.first_offer();
    assert!(
        !offered.is_empty() && offered.iter().all(|name| is_chat_recall_tool(name)),
        "the removal still applies under the preset: {offered:?}"
    );
}

/// The verifier keeps a reader (Chat Recall's one tool is annotated
/// read-only) and loses a writer (Todo's) and Biorouter's own tools.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_read_only_child_keeps_only_reading_tools() {
    let working = tempfile::TempDir::new().unwrap();
    let child = Arc::new(ScriptedChild::default());
    let spawned = spawn(
        child.clone(),
        json!({ "instructions": "Check the claims in notes.md.", "preset": "read_only" }),
        working.path(),
    )
    .await;
    let finished = spawned.result.expect("a read_only spawn runs");
    assert_ne!(finished.is_error, Some(true), "{:?}", finished.content);
    let offered = child.first_offer();
    assert!(
        offered.iter().any(|name| is_chat_recall_tool(name)),
        "a read-only tool stays: {offered:?}"
    );
    assert!(
        !offered.iter().any(|name| is_todo_tool(name)),
        "a writing tool goes: {offered:?}"
    );
    assert!(
        offered
            .iter()
            .all(|name| !is_platform_tool(name) || name == "platform__read_session_blob"),
        "Biorouter's acting tools go (schedules, workflows, ingest): {offered:?}"
    );
}

/// A roster is advice: a model can name a tool it was never offered. A preset
/// child's dispatch refuses it, whether it is one of Biorouter's own tools or
/// an inherited extension's, and nothing runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_preset_child_cannot_call_a_tool_it_was_not_offered() {
    let working = tempfile::TempDir::new().unwrap();
    let child = Arc::new(ScriptedChild::calling(vec![
        ("platform__manage_workflow", json!({ "action": "list" })),
        (
            "todo__todo_update",
            json!({ "items": [{ "text": "steps", "status": "pending" }] }),
        ),
    ]));
    let spawned = spawn(
        child.clone(),
        json!({ "instructions": "Review this paragraph: ...", "preset": "no_tools" }),
        working.path(),
    )
    .await;
    spawned.result.expect("the spawn runs");
    let seen = child.last_seen();
    for tool in ["platform__manage_workflow", "todo__todo_update"] {
        assert!(
            seen.contains(&format!(
                "`{tool}` is not available to this subagent: it was started with the no_tools preset"
            )),
            "{tool} is refused by the preset, and does not run: {seen}"
        );
    }
}

/// The one hand-made exception to the read-only rule, through the agent's
/// dispatch with Biorouter's real Developer: `text_editor` views a file and
/// cannot write, replace, insert or undo; the shell is refused; nothing in the
/// folder changes, and the folder check says so.
///
/// On the stack production gives an agent turn: a dispatch through the real
/// Developer runs deeper than a test thread's default stack.
#[test]
#[serial_test::serial(workspace_services)]
fn a_read_only_childs_text_editor_may_view_and_nothing_else() {
    crate::execution::runtime::run_on_agent_stack(|| {
        crate::execution::runtime::build_agent_runtime()
            .unwrap()
            .block_on(read_only_text_editor_may_view_and_nothing_else())
    })
    .unwrap();
}

async fn read_only_text_editor_may_view_and_nothing_else() {
    let working = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(working.path()).unwrap();
    let notes = root.join("notes.md");
    std::fs::write(&notes, "VIEWED-CONTENT-7f3a").unwrap();
    let path = |name: &str| root.join(name).to_string_lossy().into_owned();
    let child = Arc::new(ScriptedChild::calling(vec![
        (
            "developer__text_editor",
            json!({ "command": "view", "path": path("notes.md") }),
        ),
        (
            "developer__text_editor",
            json!({ "command": "write", "path": path("new.md"), "file_text": "x" }),
        ),
        (
            "developer__text_editor",
            json!({
                "command": "str_replace",
                "path": path("notes.md"),
                "old_str": "VIEWED",
                "new_str": "EDITED",
            }),
        ),
        (
            "developer__text_editor",
            json!({
                "command": "insert",
                "path": path("notes.md"),
                "insert_line": 0,
                "new_str": "INSERTED",
            }),
        ),
        (
            "developer__text_editor",
            json!({ "command": "undo_edit", "path": path("notes.md") }),
        ),
        // An absolute path: a live child's Developer runs in the process
        // directory, not this folder, so a relative one would land elsewhere.
        (
            "developer__shell",
            json!({ "command": format!("touch {}", path("made.txt")) }),
        ),
    ]));
    let spawned = spawn_with(
        child.clone(),
        json!({ "instructions": "Check notes.md.", "preset": "read_only" }),
        &root,
        Setup {
            roster: Some(vec![bundled("developer")]),
            // A live child's Developer is bound to the process directory, not
            // this folder, so a read here would raise a folder card. With no
            // person to answer it, the folder layer refuses at once; what this
            // test asks is only that the preset let the view through.
            no_person: true,
            ..Setup::default()
        },
    )
    .await;
    let finished = spawned.result.expect("the spawn runs");
    let offered = child.first_offer();
    assert!(
        offered.contains(&"developer__text_editor".to_string())
            && !offered.contains(&"developer__shell".to_string()),
        "{offered:?}"
    );
    let answer = |index: usize| child.tool_response(&format!("call-{index}"));
    let view = answer(0);
    assert!(
        !view.contains("may only view") && !view.contains("not available to this subagent"),
        "the preset lets the view through: {view}"
    );
    for (index, command) in [
        (1, "write"),
        (2, "str_replace"),
        (3, "insert"),
        (4, "undo_edit"),
    ] {
        let refused = answer(index);
        assert!(
            refused.contains("may only view a file"),
            "{command} is refused by the preset: {refused}"
        );
    }
    let shell = answer(5);
    assert!(
        shell.contains("`developer__shell` is not available to this subagent"),
        "{shell}"
    );
    assert!(!root.join("new.md").exists(), "nothing was written");
    assert!(!root.join("made.txt").exists(), "nothing was run");
    assert_eq!(
        std::fs::read_to_string(&notes).unwrap(),
        "VIEWED-CONTENT-7f3a"
    );
    let check = &finished.structured_content.as_ref().unwrap()["folder_check"];
    assert_eq!(check["unchanged"], json!(true), "{check}");
}

/// The verifier's parent records the working folder before the child runs
/// and again after, and reports a change: here a file written while the child
/// ran, which no tool the child held could have written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_read_only_child_reports_a_change_to_the_working_folder() {
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("notes.md"), "the claims").unwrap();
    std::fs::write(working.path().join("results.csv"), "a,b\n1,2\n").unwrap();
    let child = Arc::new(ScriptedChild {
        write_into: Some(working.path().join("results.csv")),
        ..ScriptedChild::default()
    });
    let spawned = spawn(
        child.clone(),
        json!({ "instructions": "Check the claims in notes.md.", "preset": "read_only" }),
        working.path(),
    )
    .await;
    let finished = spawned.result.expect("a read_only spawn runs");
    let check = finished
        .structured_content
        .as_ref()
        .and_then(|value| value.get("folder_check"))
        .unwrap_or_else(|| panic!("the result carries a folder check: {finished:?}"));
    assert_eq!(check["changed"], json!(["results.csv"]), "{check}");
    assert_eq!(check["unchanged"], json!(false), "{check}");
    let text = format!("{:?}", finished.content);
    assert!(
        text.contains("results.csv") && text.contains("changed while the subagent ran"),
        "the parent's text names the change: {text}"
    );

    // And a run that changes nothing says so.
    let quiet = Arc::new(ScriptedChild::default());
    let spawned = spawn(
        quiet,
        json!({ "instructions": "Check the claims in notes.md.", "preset": "read_only" }),
        working.path(),
    )
    .await;
    let finished = spawned.result.expect("a read_only spawn runs");
    let check = &finished.structured_content.as_ref().unwrap()["folder_check"];
    assert_eq!(check["unchanged"], json!(true), "{check}");
}

/// The path every model-facing spawn takes: the supervised background one.
/// The parent's immediate answer names the preset, and the settled result the
/// parent reads with `workspace_watch` carries the folder check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_background_read_only_spawn_reports_its_folder_check_to_the_parent() {
    let working = tempfile::TempDir::new().unwrap();
    std::fs::write(working.path().join("results.csv"), "a,b\n1,2\n").unwrap();
    let child = Arc::new(ScriptedChild {
        write_into: Some(working.path().join("results.csv")),
        ..ScriptedChild::default()
    });
    let spawned = spawn_with(
        child.clone(),
        json!({ "instructions": "Check the results.", "preset": "read_only" }),
        working.path(),
        Setup {
            background: true,
            ..Setup::default()
        },
    )
    .await;
    let started = spawned.result.expect("the background spawn starts");
    assert!(
        text_of(&started).contains("Preset read_only"),
        "{}",
        text_of(&started)
    );
    let settled = spawned.background.expect("the handle settled");
    let check = settled
        .folder_check
        .as_ref()
        .expect("a background verifier's result carries the check");
    assert_eq!(check.changed, ["results.csv"], "{check:?}");
    let watched = settled.to_agent_text();
    assert!(
        watched.contains("results.csv") && watched.contains("changed while the subagent ran"),
        "what workspace_watch shows the parent names the change: {watched}"
    );
    let offered = child.first_offer();
    assert!(
        offered.iter().all(|name| !is_todo_tool(name)),
        "the clamp holds on the background path too: {offered:?}"
    );
}

/// Door 1, failing first: a child holding the Extension Manager cannot turn
/// an extension on with `manage_extensions`. Without the presets the call
/// reaches the extension's start (and fails only because the fixture's
/// command does not exist).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_preset_child_cannot_turn_on_an_extension_with_the_extension_manager() {
    let _pinned = crate::test_sandbox::pin_sandbox_path_root();
    let name = format!("PresetDoorProbe{}", uuid::Uuid::new_v4().simple());
    let config = crate::agents::ExtensionConfig::stdio(
        name.as_str(),
        "biorouter-preset-door-probe-is-not-a-command",
        "a probe",
        5_u64,
    );
    let key = config.key();
    crate::config::extensions::set_extension(crate::config::extensions::ExtensionEntry {
        enabled: true,
        config,
    });
    struct RemoveEntry(String);
    impl Drop for RemoveEntry {
        fn drop(&mut self) {
            crate::config::extensions::remove_extension(&self.0);
        }
    }
    let _remove = RemoveEntry(key.clone());

    let working = tempfile::TempDir::new().unwrap();
    let child = Arc::new(ScriptedChild::calling(vec![(
        "extensionmanager__manage_extensions",
        json!({ "action": "enable", "extension_name": name }),
    )]));
    let spawned = spawn_with(
        child.clone(),
        json!({
            "instructions": "Look up the cohort.",
            "preset": "only_extensions",
            "extensions": ["Chat Recall", "Extension Manager"],
        }),
        working.path(),
        Setup {
            roster: Some(vec![bundled("chatrecall"), bundled("extensionmanager")]),
            ..Setup::default()
        },
    )
    .await;
    spawned.result.expect("the spawn runs");
    let seen = child.last_seen();
    assert!(
        seen.contains("no extension or tool can be added to it"),
        "the Extension Manager's enable is refused by the seal: {seen}"
    );
    assert!(
        !child
            .every_offer()
            .iter()
            .any(|tool| tool.starts_with(&format!("{key}__"))),
        "nothing of it was offered"
    );
}

/// Door 2, failing first: another chat's `workspace_set_tools` cannot add an
/// extension to a preset child, and is told why before anything changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn another_chat_cannot_add_tools_to_a_preset_child() {
    let working = tempfile::TempDir::new().unwrap();
    let spawned = spawn(
        Arc::new(ScriptedChild::default()),
        json!({
            "instructions": "Look up the cohort.",
            "preset": "only_extensions",
            "extensions": ["Chat Recall"],
        }),
        working.path(),
    )
    .await;
    spawned.result.as_ref().expect("the spawn runs");
    let child = spawned.child().await;
    let client = crate::agents::workspace_extension::WorkspaceClient::new(
        crate::agents::extension::PlatformExtensionContext {
            extension_manager: None,
            session_manager: spawned.sm.clone(),
        },
    )
    .unwrap();
    let arguments = json!({ "session_id": child.id, "add_extensions": ["Todo"] });
    let result = crate::agents::mcp_client::McpClientTrait::call_tool(
        &client,
        "workspace_set_tools",
        arguments.as_object().cloned(),
        crate::agents::mcp_client::McpMeta::new(
            spawned.parent.clone(),
            crate::privacy::CallCapability::public_enforced(),
        ),
        CancellationToken::new(),
    )
    .await
    .expect("the tool answers");
    let text = text_of(&result);
    assert_eq!(result.is_error, Some(true), "{text}");
    assert!(
        text.contains("started with the only_extensions preset"),
        "{text}"
    );
}

/// Door 3, failing first: an `/ext:` marker in a message to the child (here
/// its own first message, from a sub-workflow's prompt) cannot attach an
/// extension.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn an_ext_marker_cannot_attach_an_extension_to_a_preset_child() {
    let working = tempfile::TempDir::new().unwrap();
    let path = working.path().join("lookup.yaml");
    std::fs::write(
        &path,
        "version: \"1.0.0\"\ntitle: \"Lookup\"\ndescription: \"Looks things up\"\n\
         instructions: \"Look up the cohort.\"\nprompt: \"/ext:todo Begin.\"\n",
    )
    .unwrap();
    let sub_workflows = HashMap::from([(
        "lookup".to_string(),
        SubWorkflow {
            name: "lookup".to_string(),
            path: path.to_string_lossy().to_string(),
            values: None,
            sequential_when_repeated: false,
            description: Some("looks things up".to_string()),
        },
    )]);
    let child = Arc::new(ScriptedChild::default());
    let spawned = spawn_with(
        child.clone(),
        json!({
            "subworkflow": "lookup",
            "preset": "only_extensions",
            "extensions": ["Chat Recall"],
        }),
        working.path(),
        Setup {
            roster: Some(vec![bundled("chatrecall")]),
            sub_workflows,
            ..Setup::default()
        },
    )
    .await;
    spawned.result.expect("the spawn runs");
    let offered = child.every_offer();
    assert!(
        !offered.is_empty() && !offered.iter().any(|name| is_todo_tool(name)),
        "the marker attached nothing: {offered:?}"
    );
    assert!(
        child.last_seen().contains("/ext:todo"),
        "the marker did reach the child"
    );
}

/// A child continued after its agent was evicted is restored from its
/// runtime profile, granted extensions first and the clamp after them, so it
/// is offered exactly what it was offered when it started and nothing can be
/// added. Read-only with Developer, so the restore must keep `text_editor`
/// and not bring the shell back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_cold_preset_child_with_extensions_is_restored_with_its_clamp() {
    for (body, roster) in [
        (
            json!({ "instructions": "Check notes.md.", "preset": "read_only" }),
            vec![bundled("developer"), bundled("todo"), bundled("chatrecall")],
        ),
        (
            json!({
                "instructions": "Look up the cohort.",
                "preset": "only_extensions",
                "extensions": ["Chat Recall"],
            }),
            vec![bundled("todo"), bundled("chatrecall")],
        ),
    ] {
        let working = tempfile::TempDir::new().unwrap();
        let child = Arc::new(ScriptedChild::default());
        let spawned = spawn_with(
            child.clone(),
            body.clone(),
            working.path(),
            Setup {
                roster: Some(roster),
                ..Setup::default()
            },
        )
        .await;
        spawned.result.as_ref().expect("the spawn runs");
        let mut first: Vec<String> = child.first_offer();
        first.sort();
        let stored = spawned.child().await;

        let cold = Arc::new(crate::agents::Agent::with_config(AgentConfig::new(
            spawned.sm.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            crate::config::BioRouterMode::Auto,
        )));
        assert!(
            cold.restore_subagent_runtime_profile(&stored)
                .await
                .unwrap_or_else(|error| panic!("{body}: {error:#}")),
            "{body}: the profile was restored"
        );
        let mut restored: Vec<String> = cold
            .list_tools(&stored.id, None)
            .await
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        restored.sort();
        assert_eq!(restored, first, "{body}");
        assert!(
            !restored.iter().any(|name| name == "developer__shell"),
            "{body}"
        );
        let refused = cold.add_extension(bundled("todo")).await.unwrap_err();
        assert!(
            refused.to_string().contains(SEALED_REFUSAL),
            "{body}: {refused}"
        );
    }
}

/// A preset that contradicts the other arguments is refused at the tool's
/// entry, with nothing created.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(workspace_services)]
async fn a_contradictory_preset_is_refused_before_anything_exists() {
    for (body, expected) in [
        (
            json!({ "instructions": "x", "preset": "no_tools", "add_extensions": ["Chat Recall"] }),
            "no_tools",
        ),
        (
            json!({ "instructions": "x", "preset": "no_tools", "extensions": ["chatrecall"] }),
            "no_tools",
        ),
        (
            json!({ "instructions": "x", "preset": "no_tools", "add_skills": ["rna-qc"] }),
            "cannot be combined with add_skills",
        ),
        (
            json!({ "instructions": "x", "preset": "only_extensions" }),
            "only_extensions",
        ),
        (
            json!({ "instructions": "x", "preset": "only_extensions", "extensions": ["Memory"] }),
            "Memory",
        ),
        (
            json!({
                "instructions": "x",
                "preset": "only_extensions",
                "extensions": ["chatrecall"],
                "add_extensions": ["todo"],
            }),
            "only_extensions",
        ),
        (
            json!({
                "instructions": "x",
                "preset": "only_extensions",
                "extensions": ["Workspace Control"],
            }),
            "not switched per subagent",
        ),
        (
            json!({ "instructions": "x", "preset": "only_extensions", "extensions": ["Crew"] }),
            "not switched per subagent",
        ),
        (
            json!({
                "instructions": "x",
                "preset": "only_extensions",
                "extensions": ["Chat Recall"],
                "add_skills": ["rna-qc"],
            }),
            "add_skills needs Skills in extensions",
        ),
    ] {
        let working = tempfile::TempDir::new().unwrap();
        let spawned = spawn(
            Arc::new(ScriptedChild::default()),
            body.clone(),
            working.path(),
        )
        .await;
        let refused = spawned
            .result
            .expect_err(&format!("{body} is refused before anything exists"));
        assert!(
            refused.message.contains(expected) && refused.message.contains("Nothing was started"),
            "{body}: {}",
            refused.message
        );
        assert!(
            spawned
                .sm
                .list_subagent_children(&spawned.parent)
                .await
                .unwrap()
                .is_empty(),
            "{body}: no child row"
        );
    }
}

/// `only_extensions` with `add_skills` passes the preset check once Skills is
/// one of the named extensions, by label or by key.
#[test]
fn only_extensions_admits_add_skills_once_skills_is_named() {
    let inherited = vec![bundled("chatrecall"), bundled("skills")];
    for skills in ["Skills", "skills"] {
        let params: SubagentParams = serde_json::from_value(json!({
            "instructions": "x",
            "preset": "only_extensions",
            "extensions": ["Chat Recall", skills],
            "add_skills": ["rna-qc"],
        }))
        .unwrap();
        assert!(
            spawn_preset_refusal(&params, &inherited).is_none(),
            "{skills}"
        );
    }
}

/// The `extensions` list matches a capability by the label a person sees,
/// including the labels that do not normalise to the key (the old key-only
/// rule matched "Chat Recall" but silently dropped these).
#[test]
fn extensions_narrows_by_a_label_that_is_not_the_key() {
    for (label, key) in [
        ("Biorouter Copilot", "computercontroller"),
        ("Web & Documents", "webdocuments"),
    ] {
        let params: SubagentParams =
            serde_json::from_value(json!({ "instructions": "x", "extensions": [label] })).unwrap();
        let mut task_config = TaskConfig::new(
            Arc::new(ScriptedChild::default()),
            "parent-1",
            Path::new("."),
            vec![bundled(key), bundled("chatrecall")],
        );
        narrow_child_extensions_by_name(&mut task_config, &params);
        let kept: Vec<String> = task_config
            .extensions
            .iter()
            .map(crate::agents::subagent_presets::extension_key)
            .collect();
        assert_eq!(kept, [key], "{label}");
    }
}

/// A chat's model for the composition rows, stating its tier.
struct TierChat(crate::privacy::ProviderTier);

#[async_trait::async_trait]
impl crate::providers::base::Provider for TierChat {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new(
            "tier-chat",
            "Tier chat",
            "",
            "tier-chat-model",
            vec![],
            "",
            vec![],
        )
    }
    fn get_name(&self) -> &str {
        "tier-chat"
    }
    fn tier(&self) -> crate::privacy::ProviderTier {
        self.0
    }
    fn affiliation(&self) -> Option<crate::privacy::ModelAffiliation> {
        self.0
            .is_private()
            .then_some(crate::privacy::ModelAffiliation::Local)
    }
    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("tier-chat-model")
    }
    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> std::result::Result<
        (Message, crate::providers::base::ProviderUsage),
        crate::providers::errors::ProviderError,
    > {
        Err(crate::providers::errors::ProviderError::Authentication(
            "the composition rows never run a turn".to_string(),
        ))
    }
}

fn private_connector() -> crate::agents::ExtensionConfig {
    crate::agents::ExtensionConfig::Builtin {
        name: "ucsfomopagent".to_string(),
        display_name: Some("ucsfomopagent".to_string()),
        description: String::new(),
        timeout: Some(30),
        bundled: Some(true),
        available_tools: Vec::new(),
    }
}

/// A preset composes with the tier and affiliation filter and never changes
/// what it decided: for a public and a private chat, under every preset, the
/// child is born with the tier it has without a preset, the same private
/// connector is dropped (with the note's input recorded), and the preset only
/// ever removes from what was left.
#[tokio::test]
async fn a_preset_never_changes_the_childs_tier_or_the_tier_filters_drops() {
    for tier in [
        crate::privacy::ProviderTier::Public,
        crate::privacy::ProviderTier::Private,
    ] {
        let resolve = |preset: Option<&str>, extensions: Option<Vec<&str>>| {
            let mut body = json!({ "instructions": "x" });
            if let Some(preset) = preset {
                body["preset"] = json!(preset);
            }
            if let Some(extensions) = extensions {
                body["extensions"] = json!(extensions);
            }
            let params: SubagentParams = serde_json::from_value(body).unwrap();
            let task_config = TaskConfig::new(
                Arc::new(TierChat(tier)),
                "parent-1",
                Path::new("."),
                vec![private_connector(), bundled("chatrecall"), bundled("todo")],
            );
            async move { resolve_child_privacy(task_config, &params).await }
        };
        for (preset, extensions) in [
            (Some("no_tools"), None),
            (Some("read_only"), None),
            (
                Some("only_extensions"),
                Some(vec!["ucsfomopagent", "Chat Recall"]),
            ),
        ] {
            let (baseline, baseline_privacy) = resolve(None, extensions.clone())
                .await
                .expect("the ordinary spawn is admitted");
            let (child, privacy) = resolve(preset, extensions)
                .await
                .unwrap_or_else(|error| panic!("{tier:?} {preset:?}: {error}"));
            assert_eq!(
                child.privacy_tier, baseline.privacy_tier,
                "{tier:?} {preset:?}"
            );
            assert_eq!(
                child.dropped_private_extensions, baseline.dropped_private_extensions,
                "{tier:?} {preset:?}"
            );
            assert_eq!(
                privacy.ceiling.is_some(),
                baseline_privacy.ceiling.is_some(),
                "{tier:?} {preset:?}"
            );
            let names = |config: &TaskConfig| -> Vec<String> {
                config
                    .extensions
                    .iter()
                    .map(crate::agents::ExtensionConfig::name)
                    .collect()
            };
            assert!(
                names(&child)
                    .iter()
                    .all(|name| names(&baseline).contains(name)),
                "{tier:?} {preset:?}: a preset only removes: {:?} vs {:?}",
                names(&child),
                names(&baseline)
            );
            assert_eq!(
                child.preset.map(|preset| preset.as_str()),
                preset,
                "{tier:?}"
            );
        }
        if tier.is_private() {
            continue;
        }
        // The public chat's private connector is dropped, preset or not.
        let (child, _) = resolve(Some("read_only"), None).await.unwrap();
        assert_eq!(child.dropped_private_extensions, ["ucsfomopagent"]);
    }
}

/// A preset spawn takes the default worker model and its fallback like any
/// other: a private chat's child does not move to a public worker, runs on the
/// chat's model, the parent is told why, and the preset still holds.
#[tokio::test]
async fn a_preset_spawn_falls_back_from_a_refused_default_worker_like_any_other() {
    use crate::agents::default_worker::{WORKER_MODEL_KEY, WORKER_PROVIDER_KEY};
    let parent: Arc<dyn crate::providers::base::Provider> =
        Arc::new(TierChat(crate::privacy::ProviderTier::Private));
    let task_config = TaskConfig::new(
        Arc::clone(&parent),
        "parent-1",
        Path::new("."),
        vec![bundled("chatrecall"), bundled("todo")],
    );
    let params: SubagentParams =
        serde_json::from_value(json!({ "instructions": "x", "preset": "read_only" })).unwrap();
    let (child, privacy) = crate::config::with_config_overrides(
        HashMap::from([
            (WORKER_PROVIDER_KEY.to_string(), "ollama".to_string()),
            (WORKER_MODEL_KEY.to_string(), "worker-model".to_string()),
            (
                "OLLAMA_HOST".to_string(),
                "https://ollama.example.com".to_string(),
            ),
        ]),
        resolve_child_privacy(task_config, &params),
    )
    .await
    .expect("the default never fails a spawn");
    assert!(Arc::ptr_eq(&child.provider, &parent));
    assert_eq!(
        child.privacy_tier,
        crate::privacy::SessionClassification::Private
    );
    assert!(privacy.default_worker_note.is_some());
    assert_eq!(
        child.preset,
        Some(crate::agents::subagent_presets::SpawnPreset::ReadOnly)
    );
}
