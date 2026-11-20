// Issue #522 at the dispatch choke point: the free text of the shipped Todo
// tools and of Code Execution's `execute_code` source is left out of the secret
// scan, the tool still receives it unchanged, and nothing else is left out:
// not their other fields, not a lookalike client, not Compute's `code`.

use super::*;
use crate::privacy::{CallCapability, ProviderTier};
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};
use rmcp::object;

fn platform(name: &str, available_tools: &[&str]) -> ExtensionConfig {
    ExtensionConfig::Platform {
        name: name.to_string(),
        description: name.to_string(),
        bundled: Some(true),
        available_tools: available_tools.iter().map(|t| (*t).to_string()).collect(),
    }
}

fn call_with(name: &str, arguments: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        name: name.to_string().into(),
        arguments: arguments.as_object().cloned(),
        meta: None,
    }
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|part| part.as_text())
        .map(|part| part.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Admitted, run, and not an error.
async fn admitted(
    em: &ExtensionManager,
    session_id: &str,
    name: &str,
    arguments: serde_json::Value,
    cap: CallCapability,
) -> CallToolResult {
    let result = em
        .dispatch_tool_call(
            session_id,
            call_with(name, arguments.clone()),
            cap,
            CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("{name} {arguments} was refused: {error:#}"))
        .result
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{name} {arguments}: {result:?}");
    result
}

/// Refused at admission by the secret guard, for `path`, before any future
/// exists. A routing, privacy or schema error is not this refusal.
async fn refused_for(
    em: &ExtensionManager,
    session_id: &str,
    name: &str,
    arguments: serde_json::Value,
    cap: CallCapability,
    path: &str,
) {
    let error = match em
        .dispatch_tool_call(
            session_id,
            call_with(name, arguments.clone()),
            cap,
            CancellationToken::new(),
        )
        .await
    {
        Ok(_) => panic!("{name} {arguments} was admitted"),
        Err(error) => error,
    };
    let data = error
        .downcast_ref::<ErrorData>()
        .unwrap_or_else(|| panic!("{name}: not a guard refusal: {error:#}"));
    assert_eq!(data.code, ErrorCode::INVALID_PARAMS, "{name}: {data:?}");
    assert!(
        data.message.contains("secret/credential deny pattern") && data.message.contains(path),
        "{name}: {}",
        data.message
    );
}

#[tokio::test]
async fn issue522_shipped_todo_free_text_is_admitted_and_reaches_the_tool_unchanged() {
    let (dir, em, _sm, id) = manager_with_a_session().await;
    let em = Arc::new(em);
    em.set_working_dir(dir.path().to_path_buf()).await;
    em.add_extension(platform("todo", &[])).await.unwrap();
    let cap = CallCapability::for_test_restricted();

    admitted(
        &em,
        &id,
        "todo__todo_write",
        serde_json::json!({
            "content": "- [ ] prepare configuration/.env copy for the user\n\
                        - [ ] read configuration/.env.example"
        }),
        cap,
    )
    .await;
    admitted(
        &em,
        &id,
        "todo__todo_add",
        serde_json::json!({"items": [
            "configuration/.env still needs the user",
            {"text": "copy configuration/.env.local by hand"}
        ]}),
        cap,
    )
    .await;
    admitted(
        &em,
        &id,
        "todo__todo_expand",
        serde_json::json!({
            "id": "2",
            "items": ["diff configuration/.env.example against configuration/.env"]
        }),
        cap,
    )
    .await;
    let renamed = admitted(
        &em,
        &id,
        "todo__todo_update",
        serde_json::json!({"id": "1", "text": "configuration/.env copy remains with the user"}),
        cap,
    )
    .await;
    assert!(text_of(&renamed).contains("configuration/.env copy remains with the user"));
    let batch = admitted(
        &em,
        &id,
        "todo__todo_update",
        serde_json::json!({"updates": [
            {"id": "3", "text": "configuration/.env remains user-owned"},
            {"id": "4", "status": "completed"}
        ]}),
        cap,
    )
    .await;
    let batch = text_of(&batch);
    assert!(batch.contains("configuration/.env remains user-owned"), "{batch}");
    assert!(batch.contains("copy configuration/.env.local by hand"), "{batch}");
    admitted(
        &em,
        &id,
        "todo__plan_write",
        serde_json::json!({"plan": "The user writes configuration/.env; I read only configuration/.env.example."}),
        cap,
    )
    .await;
}

#[tokio::test]
async fn issue522_shipped_todo_and_code_execution_keep_their_other_fields_strict() {
    let (dir, em, _sm, id) = manager_with_a_session().await;
    let em = Arc::new(em);
    em.set_working_dir(dir.path().to_path_buf()).await;
    em.add_extension(platform("todo", &[])).await.unwrap();
    em.add_extension(platform("code_execution", &[]))
        .await
        .unwrap();
    let cap = CallCapability::for_test_restricted();
    admitted(
        &em,
        &id,
        "todo__todo_write",
        serde_json::json!({"content": "- [ ] one\n- [ ] two"}),
        cap,
    )
    .await;

    // Each pair: a field the projection does not cover, readable with an
    // ordinary value and refused with a protected one.
    for (name, benign, protected) in [
        (
            "todo__todo_write",
            serde_json::json!({"content": "- [ ] safe", "resource": "notes/checklist.md"}),
            serde_json::json!({"content": "- [ ] safe", "resource": "configuration/.env"}),
        ),
        (
            "todo__todo_add",
            serde_json::json!({"items": ["safe"], "after": "1"}),
            serde_json::json!({"items": ["safe"], "after": "configuration/.env"}),
        ),
        (
            "todo__todo_update",
            serde_json::json!({"id": "1", "status": "completed", "text": {"nested": "safe"}}),
            serde_json::json!({
                "id": "1",
                "status": "completed",
                "text": {"nested": "configuration/.env"}
            }),
        ),
        (
            "todo__todo_update",
            serde_json::json!({"updates": [
                {"id": "1", "status": "pending", "text": {"nested": "safe"}}
            ]}),
            serde_json::json!({"updates": [
                {"id": "1", "status": "pending", "text": {"nested": "configuration/.env"}}
            ]}),
        ),
        (
            "code_execution__execute_code",
            serde_json::json!({"code": "1 + 1", "resource": "notes/source.js"}),
            serde_json::json!({"code": "1 + 1", "resource": "configuration/.env"}),
        ),
        (
            "code_execution__execute_code",
            serde_json::json!({"code": "1 + 1", "command": "render safe output"}),
            serde_json::json!({"code": "1 + 1", "command": "cat configuration/.env"}),
        ),
        (
            "code_execution__execute_code",
            serde_json::json!({"code": "1 + 1", "tool_graph": [
                {"tool": "developer/shell", "description": "read configuration/.env.example"}
            ]}),
            serde_json::json!({"code": "1 + 1", "tool_graph": [
                {"tool": "configuration/.env", "description": "safe"}
            ]}),
        ),
    ] {
        admitted(&em, &id, name, benign, cap).await;
        refused_for(&em, &id, name, protected, cap, "configuration/.env").await;
    }
    // Source of the wrong JSON type is not source: it is scanned whole.
    admitted(
        &em,
        &id,
        "code_execution__execute_code",
        serde_json::json!({"code": "// configuration/.env is the user's\n1 + 1"}),
        cap,
    )
    .await;
    refused_for(
        &em,
        &id,
        "code_execution__execute_code",
        serde_json::json!({"code": ["1 + 1", {"path": "configuration/.env"}]}),
        cap,
        "configuration/.env",
    )
    .await;
}

#[tokio::test]
async fn issue522_lookalikes_and_other_extensions_keep_free_text_strict() {
    let (dir, em, _sm, id) = manager_with_a_session().await;
    let em = Arc::new(em);
    em.set_working_dir(dir.path().to_path_buf()).await;
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    let fake_todo = CountingClient::new("fake-todo");
    let fake_code = CountingClient::new("fake-code");
    let fake_nested = CountingClient::new("fake-nested");
    let fake_unknown = CountingClient::new("fake-unknown");
    let fake_external = CountingClient::new("fake-external");
    // The same names and configs as the shipped extensions, injected rather
    // than built by the registry.
    em.add_client(
        "todo".into(),
        platform("todo", &["todo_update"]),
        Arc::new(fake_todo.clone()),
        None,
        None,
    )
    .await;
    em.add_client(
        "code_execution".into(),
        platform("code_execution", &["execute_code"]),
        Arc::new(fake_code.clone()),
        None,
        None,
    )
    .await;
    em.add_client(
        "todo__nested".into(),
        platform("todo", &["todo_update"]),
        Arc::new(fake_nested.clone()),
        None,
        None,
    )
    .await;
    em.add_client(
        "unknown_platform".into(),
        platform("unknown_platform", &["inspect"]),
        Arc::new(fake_unknown.clone()),
        None,
        None,
    )
    .await;
    em.add_client(
        "code_execution_external".into(),
        ExtensionConfig::Stdio {
            name: "code_execution_external".into(),
            description: "inert external fixture".into(),
            cmd: "never-spawned".into(),
            args: vec![],
            envs: Envs::default(),
            env_keys: vec![],
            timeout: None,
            bundled: Some(true),
            available_tools: vec!["execute_code".into()],
        },
        Arc::new(fake_external.clone()),
        None,
        None,
    )
    .await;

    for (name, benign, protected, client) in [
        (
            "todo__todo_update",
            serde_json::json!({"id": "1", "text": "safe task"}),
            serde_json::json!({"id": "1", "text": "configuration/.env still needs the user"}),
            &fake_todo,
        ),
        (
            "code_execution__execute_code",
            serde_json::json!({"code": "// safe\n1 + 1"}),
            serde_json::json!({"code": "// configuration/.env\n1 + 1"}),
            &fake_code,
        ),
        (
            "todo__nested__todo_update",
            serde_json::json!({"id": "1", "text": "safe task"}),
            serde_json::json!({"id": "1", "text": "configuration/.env"}),
            &fake_nested,
        ),
        (
            "unknown_platform__inspect",
            serde_json::json!({"content": "safe content"}),
            serde_json::json!({"content": "configuration/.env"}),
            &fake_unknown,
        ),
        (
            "code_execution_external__execute_code",
            serde_json::json!({"code": "// safe\n1 + 1"}),
            serde_json::json!({"code": "// configuration/.env\n1 + 1"}),
            &fake_external,
        ),
    ] {
        let before = client.contacted();
        admitted(&em, &id, name, benign, cap).await;
        assert_eq!(client.contacted(), before + 1, "{name}");
        refused_for(&em, &id, name, protected, cap, "configuration/.env").await;
        assert_eq!(client.contacted(), before + 1, "{name} was contacted");
    }
}

/// A sandbox that runs nothing and records what it was asked to run.
#[derive(Clone, Default)]
struct RecordingSandbox {
    runs: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}

#[async_trait::async_trait]
impl biorouter_sandbox::SandboxClient for RecordingSandbox {
    fn label(&self) -> &'static str {
        "recording-test"
    }

    async fn available(&self) -> bool {
        true
    }

    async fn exec(
        &self,
        argv: &[String],
        _stdin: Option<&str>,
    ) -> biorouter_sandbox::Result<biorouter_sandbox::ExecOutput> {
        self.runs.lock().unwrap().push(argv.to_vec());
        Ok(biorouter_sandbox::ExecOutput {
            stdout: "recorded".to_string(),
            stderr: String::new(),
            code: 0,
            timed_out: false,
        })
    }

    async fn read(&self, _guest_path: &str) -> biorouter_sandbox::Result<Vec<u8>> {
        Ok(Vec::new())
    }

    async fn write(&self, _guest_path: &str, _bytes: &[u8]) -> biorouter_sandbox::Result<()> {
        Ok(())
    }

    async fn list(&self, _guest_dir: &str) -> biorouter_sandbox::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn snapshot(&self) -> biorouter_sandbox::Result<String> {
        Ok("recording-snapshot".to_string())
    }
}

/// Compute's `code` is a program that can open the path it names, so it keeps
/// the floor: only Code Execution's sandboxed source is left out.
#[tokio::test]
async fn issue522_compute_code_keeps_the_floor() {
    let (dir, em, _sm, id) = manager_with_a_session().await;
    let em = Arc::new(em);
    em.set_working_dir(dir.path().to_path_buf()).await;
    let sandbox = RecordingSandbox::default();
    let runs = Arc::clone(&sandbox.runs);
    em.add_inprocess_server(
        "compute",
        biorouter_mcp::compute_server::ComputeServer::new(Arc::new(sandbox)),
    )
    .await
    .unwrap();
    let cap = CallCapability::for_test_restricted();

    admitted(
        &em,
        &id,
        "compute__compute_python",
        serde_json::json!({"code": "# configuration/notes.md\nprint('safe')"}),
        cap,
    )
    .await;
    assert_eq!(runs.lock().unwrap().len(), 1);
    refused_for(
        &em,
        &id,
        "compute__compute_python",
        serde_json::json!({"code": "# configuration/.env\nprint(open('configuration/.env').read())"}),
        cap,
        "configuration/.env",
    )
    .await;
    assert_eq!(runs.lock().unwrap().len(), 1, "the refused program ran");
}

/// The projection itself: exactly the mapped strings are emptied, in a copy.
#[test]
fn issue522_free_text_fields_empty_only_their_own_strings() {
    let arguments = object!({
        "code": "// configuration/.env",
        "command": "cat configuration/.env",
        "tool_graph": [{"tool": "developer/shell", "description": "configuration/.env"}],
    });
    let scanned = FreeTextFields::ExecuteCode.withheld_from(&arguments);
    assert_eq!(scanned["code"], "");
    assert_eq!(scanned["command"], "cat configuration/.env");
    assert_eq!(scanned["tool_graph"][0]["tool"], "developer/shell");
    assert_eq!(scanned["tool_graph"][0]["description"], "");
    assert_eq!(arguments["code"], "// configuration/.env", "the original changed");

    let items = FreeTextFields::TodoItems.withheld_from(&object!({
        "id": "configuration/.env",
        "items": ["a/.env", {"text": "b/.env", "after": "c/.env"}, {"task": ["d/.env"]}],
    }));
    assert_eq!(items["id"], "configuration/.env");
    assert_eq!(items["items"][0], "");
    assert_eq!(items["items"][1]["text"], "");
    assert_eq!(items["items"][1]["after"], "c/.env");
    assert_eq!(items["items"][2]["task"][0], "d/.env");

    let todo = platform("todo", &[]);
    assert_eq!(
        FreeTextFields::of(true, "todo", &todo, "todo__todo_update"),
        Some(FreeTextFields::TodoUpdate)
    );
    assert_eq!(FreeTextFields::of(false, "todo", &todo, "todo__todo_update"), None);
    assert_eq!(
        FreeTextFields::of(true, "todo__nested", &todo, "todo__nested__todo_update"),
        None
    );
    assert_eq!(FreeTextFields::of(true, "todo", &todo, "todo__delete_all"), None);
    let code = platform("code_execution", &[]);
    assert_eq!(
        FreeTextFields::of(true, "code_execution", &code, "code_execution__read_module"),
        None
    );
    assert_eq!(
        FreeTextFields::of(true, "todo", &code, "todo__todo_update"),
        None,
        "a config that is not Todo's"
    );
}
