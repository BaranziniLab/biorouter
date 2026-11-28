//! J-P03: the clamp on a real agent. Every way a tool reaches a running chat
//! is shut, the extension manager's own dispatch refuses what the clamp does
//! not hold, and a cold child gets its clamp back from its runtime profile.

use super::*;
use crate::agents::{Agent, AgentConfig};
use crate::config::permission::PermissionManager;
use crate::config::BioRouterMode;
use crate::session::session_manager::SessionType;
use crate::session::SessionManager;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ToolAnnotations};

fn bundled(name: &str) -> ExtensionConfig {
    crate::agents::extension_manager::resolve_bundled_extension(name)
        .unwrap()
        .into_config(String::new())
}

fn agent(sm: &Arc<SessionManager>) -> Arc<Agent> {
    Arc::new(Agent::with_config(AgentConfig::new(
        sm.clone(),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    )))
}

fn names(tools: &[Tool]) -> Vec<String> {
    tools.iter().map(|tool| tool.name.to_string()).collect()
}

async fn child(sm: &SessionManager, root: &Path) -> crate::session::Session {
    sm.create_session(root.to_path_buf(), "child".into(), SessionType::SubAgent)
        .await
        .unwrap()
}

/// A third-party server with one reader and one writer, recording every call
/// that reaches it.
struct LabClient {
    called: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl crate::agents::mcp_client::McpClientTrait for LabClient {
    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ServiceError> {
        let schema = serde_json::json!({"type": "object"})
            .as_object()
            .unwrap()
            .clone();
        Ok(rmcp::model::ListToolsResult {
            tools: vec![
                Tool::new("search".to_string(), String::new(), schema.clone()).annotate(
                    ToolAnnotations {
                        title: None,
                        read_only_hint: Some(true),
                        destructive_hint: Some(false),
                        idempotent_hint: None,
                        open_world_hint: None,
                    },
                ),
                Tool::new("write_row".to_string(), String::new(), schema),
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        _arguments: Option<rmcp::model::JsonObject>,
        _meta: crate::agents::mcp_client::McpMeta,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, rmcp::ServiceError> {
        self.called.lock().unwrap().push(name.to_string());
        Ok(CallToolResult {
            content: vec![Content::text(format!("{name} ran"))],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        })
    }

    fn get_info(&self) -> Option<&rmcp::model::InitializeResult> {
        None
    }
}

fn lab_config() -> ExtensionConfig {
    ExtensionConfig::stdio("lab", "unused-fixture-command", "lab fixture", 30_u64)
}

/// An in-process server with nothing on it, for the seal on
/// `add_inprocess_server`.
#[derive(Clone)]
struct EmptyServer;
impl rmcp::ServerHandler for EmptyServer {}

/// `manage_extensions` (straight to the manager), `workspace_set_tools`
/// and an `/ext:` marker (through `Agent::add_extension`), a Frontend config
/// (which never reaches the manager), an in-process server and a client
/// injected directly all fail once a clamp is installed, and the clamp itself
/// cannot be swapped for a wider one.
#[tokio::test]
async fn a_clamped_agent_refuses_every_way_to_attach_a_tool() {
    let temp = tempfile::TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let session = child(&sm, temp.path()).await;
    let agent = agent(&sm);
    agent.add_extension(bundled("chatrecall")).await.unwrap();
    let loaded = agent.get_extension_configs().await;
    let offered = agent.list_tools(&session.id, None).await;
    assert!(
        names(&offered)
            .iter()
            .any(|name| name.starts_with("platform__")),
        "an unclamped child is offered Biorouter's own tools: {:?}",
        names(&offered)
    );
    let clamp = ToolClamp::for_preset(SpawnPreset::ReadOnly, &offered, &loaded);
    agent.install_spawn_tool_clamp(clamp.clone()).unwrap();
    assert!(agent.extension_manager.is_sealed());

    let now = names(&agent.list_tools(&session.id, None).await);
    assert!(
        now.iter().all(|name| name.ends_with("__chatrecall")),
        "only the reader is left: {now:?}"
    );

    let refused = agent.add_extension(bundled("todo")).await.unwrap_err();
    assert!(refused.to_string().contains(SEALED_REFUSAL), "{refused}");
    let refused = agent
        .extension_manager
        .add_extension(bundled("todo"))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains(SEALED_REFUSAL), "{refused}");
    let frontend = ExtensionConfig::Frontend {
        name: "page".into(),
        description: String::new(),
        tools: vec![Tool::new(
            "click".to_string(),
            String::new(),
            serde_json::json!({"type": "object"})
                .as_object()
                .unwrap()
                .clone(),
        )],
        instructions: None,
        bundled: None,
        available_tools: Vec::new(),
    };
    assert!(agent.add_extension(frontend).await.is_err());
    let refused = agent
        .extension_manager
        .add_inprocess_server("late_server", EmptyServer)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains(SEALED_REFUSAL), "{refused}");
    agent
        .extension_manager
        .add_client(
            "lab".to_string(),
            lab_config(),
            Arc::new(LabClient {
                called: Arc::default(),
            }),
            None,
            None,
        )
        .await;
    for key in ["todo", "late_server", "lab"] {
        assert!(
            !agent.extension_manager.is_extension_enabled(key).await,
            "{key} was attached to a sealed manager"
        );
    }
    assert!(
        !names(&agent.list_tools(&session.id, None).await)
            .iter()
            .any(|name| name.contains("todo") || name.contains("click")),
        "nothing was attached"
    );

    // The same clamp again is a no-op (a restore on a live agent); any
    // other is refused.
    agent.install_spawn_tool_clamp(clamp).unwrap();
    let wider = ToolClamp::for_preset(SpawnPreset::OnlyExtensions, &offered, &loaded);
    assert!(agent.install_spawn_tool_clamp(wider).is_err());

    // Taking away still works.
    agent.remove_extension("chatrecall").await.unwrap();
    assert!(agent.list_tools(&session.id, None).await.is_empty());
}

/// The extension manager's own dispatch, the choke point a Code Execution
/// script, `POST /agent/call_tool` and the coding-agent bridge share, refuses
/// a tool the clamp does not hold, here an extension's writer that stands in
/// for a tool added in the middle of the run, and nothing reaches the server.
#[tokio::test]
async fn a_clamped_managers_dispatch_refuses_every_tool_outside_the_clamp() {
    let temp = tempfile::TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let session = child(&sm, temp.path()).await;
    let agent = agent(&sm);
    let called = Arc::new(Mutex::new(Vec::new()));
    agent
        .extension_manager
        .add_client(
            "lab".to_string(),
            lab_config(),
            Arc::new(LabClient {
                called: Arc::clone(&called),
            }),
            None,
            None,
        )
        .await;
    let offered = agent.list_tools(&session.id, None).await;
    let clamp = ToolClamp::for_preset(SpawnPreset::ReadOnly, &offered, &[lab_config()]);
    assert!(clamp.offers("lab__search") && !clamp.offers("lab__write_row"));
    agent.install_spawn_tool_clamp(clamp).unwrap();

    let call = |name: &str| CallToolRequestParams {
        name: name.to_string().into(),
        arguments: Some(serde_json::Map::new()),
        meta: None,
        task: None,
    };
    let refused = match agent
        .extension_manager
        .dispatch_tool_call(
            &session.id,
            call("lab__write_row"),
            crate::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
    {
        Ok(_) => panic!("the manager dispatched a tool outside the clamp"),
        Err(error) => error.to_string(),
    };
    assert!(
        refused.contains("not available to this subagent"),
        "{refused}"
    );
    let admitted = agent
        .extension_manager
        .dispatch_tool_call(
            &session.id,
            call("lab__search"),
            crate::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("a clamped reader is dispatched");
    admitted.result.await.expect("the reader runs");
    assert_eq!(*called.lock().unwrap(), ["search"]);
}

/// A child continued after its agent was evicted is restored from its
/// runtime profile with the clamp, so Biorouter's own tools, which no
/// extension grant names, do not come back.
#[tokio::test]
async fn a_cold_preset_child_is_restored_with_its_clamp() {
    let temp = tempfile::TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let session = child(&sm, temp.path()).await;
    let clamp = ToolClamp::for_preset(SpawnPreset::NoTools, &[], &[]);
    crate::agents::subagent_runtime_profile::SubagentRuntimeProfile::new(
        "a reviewer".into(),
        None,
        Vec::new(),
        &[],
        &[],
    )
    .unwrap()
    .with_tool_clamp(Some(clamp))
    .unwrap()
    .persist(&sm, &session.id)
    .await
    .unwrap();
    let stored = sm.get_session(&session.id, false).await.unwrap();
    assert_eq!(
        crate::agents::subagent_runtime_profile::spawn_preset_of(&stored.extension_data),
        Some(SpawnPreset::NoTools)
    );

    let cold = agent(&sm);
    assert!(cold
        .restore_subagent_runtime_profile(&stored)
        .await
        .unwrap());
    assert_eq!(
        names(&cold.list_tools(&session.id, None).await),
        Vec::<String>::new()
    );
    assert!(cold.add_extension(bundled("todo")).await.is_err());
    // A second restore on the same live agent changes nothing.
    assert!(cold
        .restore_subagent_runtime_profile(&stored)
        .await
        .unwrap());
}

/// A runtime profile written before presets existed (or by an ordinary
/// spawn) carries no clamp: it restores as before, names no preset, and
/// leaves the manager unsealed.
#[tokio::test]
async fn a_profile_without_a_clamp_restores_an_ordinary_child() {
    let temp = tempfile::TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(temp.path().to_path_buf()));
    let session = child(&sm, temp.path()).await;
    crate::agents::subagent_runtime_profile::SubagentRuntimeProfile::new(
        "an ordinary child".into(),
        None,
        Vec::new(),
        &[],
        &[],
    )
    .unwrap()
    .persist(&sm, &session.id)
    .await
    .unwrap();
    let stored = sm.get_session(&session.id, false).await.unwrap();
    assert_eq!(
        crate::agents::subagent_runtime_profile::spawn_preset_of(&stored.extension_data),
        None
    );
    let cold = agent(&sm);
    assert!(cold
        .restore_subagent_runtime_profile(&stored)
        .await
        .unwrap());
    assert!(!cold.extension_manager.is_sealed());
    cold.add_extension(bundled("todo")).await.unwrap();
}
