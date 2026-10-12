mod common;

use biorouter::agents::ExtensionConfig;
use biorouter::config::BioRouterMode;
use biorouter::model::ModelConfig;
use biorouter::providers::api_client::{ApiClient, AuthMethod};
use biorouter::providers::openai::OpenAiProvider;
use biorouter_acp::server::{serve, BioRouterAcpAgent, BioRouterAcpConfig};
use common::{setup_mock_openai, spawn_mcp_http_server, FAKE_CODE};
use fs_err as fs;
use sacp::schema::{
    ContentBlock, ContentChunk, InitializeRequest, LoadSessionRequest, McpServer, McpServerHttp,
    NewSessionRequest, PermissionOptionKind, PromptRequest, ProtocolVersion,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, StopReason, TextContent,
    ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use sacp::{ClientToAgent, JrConnectionCx};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use test_case::test_case;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use wiremock::MockServer;

fn run_async_test(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

#[test]
fn test_acp_basic_completion() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompt = "what is 1+1";
        let mock_server = setup_mock_openai(vec![(
            format!(r#"</info-msg>\n{prompt}""#),
            include_str!("./test_data/openai_basic_response.txt"),
        )])
        .await;

        run_acp_session(
            &mock_server,
            vec![],
            &[],
            temp_dir.path(),
            BioRouterMode::Auto,
            None,
            |cx, session_id, updates| async move {
                let response = cx
                    .send_request(PromptRequest::new(
                        session_id,
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task()
                    .await
                    .unwrap();

                assert_eq!(response.stop_reason, StopReason::EndTurn);
                wait_for(
                    &updates,
                    &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                        TextContent::new("2"),
                    ))),
                )
                .await;
            },
        )
        .await;
    });
}

#[test]
fn test_acp_with_mcp_http_server() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompt = "Use the get_code tool and output only its result.";
        let (mcp_url, _handle) = spawn_mcp_http_server().await;

        let mock_server = setup_mock_openai(vec![
            (
                format!(r#"</info-msg>\n{prompt}""#),
                include_str!("./test_data/openai_tool_call_response.txt"),
            ),
            (
                format!(r#""content":"{FAKE_CODE}""#),
                include_str!("./test_data/openai_tool_result_response.txt"),
            ),
        ])
        .await;

        run_acp_session(
            &mock_server,
            vec![McpServer::Http(McpServerHttp::new("lookup", mcp_url))],
            &[],
            temp_dir.path(),
            BioRouterMode::Auto,
            None,
            |cx, session_id, updates| async move {
                let response = cx
                    .send_request(PromptRequest::new(
                        session_id,
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task()
                    .await
                    .unwrap();

                assert_eq!(response.stop_reason, StopReason::EndTurn);
                wait_for(
                    &updates,
                    &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                        TextContent::new(FAKE_CODE),
                    ))),
                )
                .await;
            },
        )
        .await;
    });
}

/// A cheap provider for tests that only build the agent (no LLM turn is run).
/// Construction makes no network call.
fn dummy_provider() -> Arc<dyn biorouter::providers::base::Provider> {
    let api_client = ApiClient::new(
        "http://127.0.0.1:1".to_string(),
        AuthMethod::BearerToken("test-key".to_string()),
    )
    .unwrap();
    Arc::new(OpenAiProvider::new(
        api_client,
        ModelConfig::new("gpt-5-nano").unwrap(),
    ))
}

/// Regression test for #10 (extensions disabled in acp sessions): extensions
/// resolved from the user's config (the list `biorouter acp` now passes to
/// `new(_, true)`) must be registered on the shared ACP agent. Before the fix,
/// `with_config` only loaded `--with-builtin` extensions, so a plain
/// `biorouter acp` had none — including the `extensionmanager` tool needed to
/// enable others at runtime.
#[test]
fn test_acp_loads_config_extensions() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = BioRouterAcpConfig {
            provider: dummy_provider(),
            builtins: vec![],
            work_dir: temp_dir.path().to_path_buf(),
            data_dir: temp_dir.path().to_path_buf(),
            config_dir: temp_dir.path().to_path_buf(),
            biorouter_mode: BioRouterMode::Auto,
            extensions: vec![ExtensionConfig::Platform {
                name: "extensionmanager".to_string(),
                bundled: None,
                description: "extensionmanager".to_string(),
                available_tools: vec![],
            }],
        };

        let agent = BioRouterAcpAgent::with_config(config).await.unwrap();
        let names = agent.list_extensions().await;
        assert!(
            names.iter().any(|n| n == "extensionmanager"),
            "config extension was not loaded onto the ACP agent: {names:?}"
        );
    });
}

/// The WebSocket transport passes an empty `extensions` list (its peer is
/// untrusted artifact content), so no config extensions are registered. Guards
/// the security gate that keeps the user's stdio MCP tools off the WS transport.
#[test]
fn test_acp_no_config_extensions_when_empty() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = BioRouterAcpConfig {
            provider: dummy_provider(),
            builtins: vec![],
            work_dir: temp_dir.path().to_path_buf(),
            data_dir: temp_dir.path().to_path_buf(),
            config_dir: temp_dir.path().to_path_buf(),
            biorouter_mode: BioRouterMode::Auto,
            extensions: vec![],
        };

        let agent = BioRouterAcpAgent::with_config(config).await.unwrap();
        let names = agent.list_extensions().await;
        assert!(
            !names.iter().any(|n| n == "extensionmanager"),
            "unexpected extension registered on empty-config ACP agent: {names:?}"
        );
    });
}

#[test]
fn test_acp_with_builtin_and_mcp() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompt =
            "Search for get_code and text_editor tools. Use them to save the code to result.txt.";
        let (lookup_url, _lookup_handle) = spawn_mcp_http_server().await;

        let mock_server = setup_mock_openai(vec![
            (
                format!(r#"</info-msg>\n{prompt}""#),
                include_str!("./test_data/openai_builtin_search.txt"),
            ),
            (
                r#"module_lookup[\"get_code\"]({}): string - Get the code"#.into(),
                include_str!("./test_data/openai_builtin_read_modules.txt"),
            ),
            (
                r#"lookup[\"get_code\"]({}): string - Get the code"#.into(),
                include_str!("./test_data/openai_builtin_execute.txt"),
            ),
            (
                // The text_editor write reports the *resolved absolute path*
                // ("Successfully wrote to /…/result.txt"), so match the stable
                // message prefix rather than a bare filename.
                r#"Successfully wrote to"#.into(),
                include_str!("./test_data/openai_builtin_final.txt"),
            ),
        ])
        .await;

        run_acp_session(
            &mock_server,
            vec![McpServer::Http(McpServerHttp::new("lookup", lookup_url))],
            &["code_execution", "developer"],
            temp_dir.path(),
            BioRouterMode::Auto,
            None,
            |cx, session_id, updates| async move {
                let response = cx
                    .send_request(PromptRequest::new(
                        session_id,
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task()
                    .await
                    .unwrap();

                assert_eq!(response.stop_reason, StopReason::EndTurn);
                wait_for(
                    &updates,
                    &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                        TextContent::new(FAKE_CODE),
                    ))),
                )
                .await;
            },
        )
        .await;
    });
}

/// Simulates a client restart: complete one turn over a first connection, then
/// reconnect with a second client and `session/load` the same session id. The
/// agent must rehydrate the persisted conversation and replay the user-visible
/// history — the user's prompt and the assistant's reply — as session updates.
#[test]
fn test_acp_load_session_replays_history() {
    run_async_test(async {
        let data_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        let prompt = "what is 1+1";

        let mock_server = setup_mock_openai(vec![(
            format!(r#"</info-msg>\n{prompt}""#),
            include_str!("./test_data/openai_basic_response.txt"),
        )])
        .await;

        // First client: create a session, run one turn, remember the session id.
        let captured = Arc::new(Mutex::new(None));
        run_acp_client(
            &mock_server,
            &[],
            data_dir.path(),
            BioRouterMode::Auto,
            None,
            {
                let captured = captured.clone();
                let cwd = work_dir.path().to_path_buf();
                move |cx, updates| async move {
                    let session = cx
                        .send_request(NewSessionRequest::new(cwd))
                        .block_task()
                        .await
                        .unwrap();

                    cx.send_request(PromptRequest::new(
                        session.session_id.clone(),
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task()
                    .await
                    .unwrap();

                    wait_for(
                        &updates,
                        &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("2"),
                        ))),
                    )
                    .await;

                    *captured.lock().unwrap() = Some(session.session_id);
                }
            },
        )
        .await;

        let session_id = captured
            .lock()
            .unwrap()
            .clone()
            .expect("session id captured");

        // Second client, same data dir: the first connection is gone, so this
        // exercises rehydration from persisted state rather than in-memory state.
        run_acp_client(
            &mock_server,
            &[],
            data_dir.path(),
            BioRouterMode::Auto,
            None,
            {
                let cwd = work_dir.path().to_path_buf();
                move |cx, updates| async move {
                    cx.send_request(LoadSessionRequest::new(session_id, cwd))
                        .block_task()
                        .await
                        .unwrap();

                    // The user's original prompt is replayed back to the client...
                    wait_for(
                        &updates,
                        &SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(prompt),
                        ))),
                    )
                    .await;

                    // ...along with the assistant's answer from the earlier turn.
                    wait_for(
                        &updates,
                        &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("2"),
                        ))),
                    )
                    .await;
                }
            },
        )
        .await;
    });
}

/// R-30: an ACP `session/load` is opening a chat, not activity in it.
///
/// Loading binds this server's provider to the chat and, for a chat with no
/// message yet, applies the client's directory. Both writes used to stamp
/// `updated_at`, so every load moved the chat to the top of every list sorted
/// by activity (the desktop's sidebar and History, `biorouter session list`).
/// Both writes must still LAND, which is asserted too: otherwise "unchanged"
/// could mean the load never reached them.
#[test]
fn test_acp_load_session_is_not_activity() {
    use biorouter::conversation::message::Message;
    use biorouter::session::session_manager::SessionType;
    use biorouter::session::SessionManager;

    run_async_test(async {
        let data_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        let mock_server = setup_mock_openai(Vec::new()).await;

        let sm = SessionManager::new(data_dir.path().to_path_buf());
        let chat = sm
            .create_session(
                data_dir.path().to_path_buf(),
                "R-30 chat".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(&chat.id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        let empty = sm
            .create_session(
                data_dir.path().to_path_buf(),
                "R-30 empty chat".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let chat_before = sm.get_session(&chat.id, false).await.unwrap().updated_at;
        let empty_before = sm.get_session(&empty.id, false).await.unwrap().updated_at;
        sm.close().await;
        // `datetime('now')` has one-second resolution: load in a later second,
        // so a stamp cannot hide inside the second the rows were written in.
        tokio::time::sleep(Duration::from_millis(1_100)).await;

        run_acp_client(
            &mock_server,
            &[],
            data_dir.path(),
            BioRouterMode::Auto,
            None,
            {
                let cwd = work_dir.path().to_path_buf();
                let ids = [chat.id.clone(), empty.id.clone()];
                move |cx, _updates| async move {
                    for id in ids {
                        cx.send_request(LoadSessionRequest::new(id, cwd.clone()))
                            .block_task()
                            .await
                            .unwrap();
                    }
                }
            },
        )
        .await;

        let sm = SessionManager::new(data_dir.path().to_path_buf());
        let chat_after = sm.get_session(&chat.id, false).await.unwrap();
        let empty_after = sm.get_session(&empty.id, false).await.unwrap();
        sm.close().await;

        assert_eq!(
            chat_after.provider_name.as_deref(),
            Some("openai"),
            "premise: the load bound its provider"
        );
        assert_eq!(
            empty_after.working_dir,
            work_dir.path(),
            "premise: the load applied the client's directory to the empty chat"
        );
        assert_eq!(
            chat_after.updated_at, chat_before,
            "loading a chat is not activity in it (R-30)"
        );
        assert_eq!(
            empty_after.updated_at, empty_before,
            "applying the directory while loading is not activity either"
        );
    });
}

async fn wait_for(updates: &Arc<Mutex<Vec<SessionNotification>>>, expected: &SessionUpdate) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut context = String::new();

    loop {
        let matched = {
            let guard = updates.lock().unwrap();
            context.clear();

            match expected {
                SessionUpdate::AgentMessageChunk(chunk) => {
                    let expected_text = match &chunk.content {
                        ContentBlock::Text(t) => &t.text,
                        other => panic!("wait_for: unhandled content {:?}", other),
                    };
                    for n in guard.iter() {
                        if let SessionUpdate::AgentMessageChunk(c) = &n.update {
                            if let ContentBlock::Text(t) = &c.content {
                                if t.text.is_empty() {
                                    context.clear();
                                } else {
                                    context.push_str(&t.text);
                                }
                            }
                        }
                    }
                    context.contains(expected_text)
                }
                SessionUpdate::UserMessageChunk(chunk) => {
                    let expected_text = match &chunk.content {
                        ContentBlock::Text(t) => &t.text,
                        other => panic!("wait_for: unhandled content {:?}", other),
                    };
                    for n in guard.iter() {
                        if let SessionUpdate::UserMessageChunk(c) = &n.update {
                            if let ContentBlock::Text(t) = &c.content {
                                context.push_str(&t.text);
                            }
                        }
                    }
                    context.contains(expected_text)
                }
                SessionUpdate::ToolCallUpdate(expected_update) => {
                    for n in guard.iter() {
                        if let SessionUpdate::ToolCallUpdate(u) = &n.update {
                            context.push_str(&format!("{:?}\n", u));
                            if u.fields.status == expected_update.fields.status {
                                return;
                            }
                        }
                    }
                    false
                }
                other => panic!("wait_for: unhandled update {:?}", other),
            }
        };

        if matched {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timeout waiting for {:?}\n\n{}", expected, context);
        }
        tokio::task::yield_now().await;
    }
}

async fn spawn_server_in_process(
    mock_server: &MockServer,
    builtins: &[&str],
    data_root: &Path,
    biorouter_mode: BioRouterMode,
) -> (
    tokio::io::DuplexStream,
    tokio::io::DuplexStream,
    tokio::task::JoinHandle<()>,
) {
    let api_client = ApiClient::new(
        mock_server.uri(),
        AuthMethod::BearerToken("test-key".to_string()),
    )
    .unwrap();
    let model_config = ModelConfig::new("gpt-5-nano").unwrap();
    let provider = OpenAiProvider::new(api_client, model_config);

    let config = BioRouterAcpConfig {
        provider: Arc::new(provider),
        builtins: builtins.iter().map(|s| s.to_string()).collect(),
        work_dir: data_root.to_path_buf(),
        data_dir: data_root.to_path_buf(),
        config_dir: data_root.to_path_buf(),
        biorouter_mode,
        extensions: vec![],
    };

    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, client_write) = tokio::io::duplex(64 * 1024);

    let agent = Arc::new(BioRouterAcpAgent::with_config(config).await.unwrap());
    let handle = tokio::spawn(async move {
        if let Err(e) = serve(agent, server_read.compat(), server_write.compat_write()).await {
            tracing::error!("ACP server error: {e}");
        }
    });

    (client_read, client_write, handle)
}

/// Connect one ACP client to a freshly spawned in-process server and run `test_fn`
/// after the `initialize` handshake. The caller decides whether to create a new
/// session or load an existing one, so a test can simulate a client restart by
/// calling this twice against the same `data_root`.
async fn run_acp_client<F, Fut>(
    mock_server: &MockServer,
    builtins: &[&str],
    data_root: &Path,
    mode: BioRouterMode,
    select: Option<PermissionOptionKind>,
    test_fn: F,
) where
    F: FnOnce(JrConnectionCx<ClientToAgent>, Arc<Mutex<Vec<SessionNotification>>>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (client_read, client_write, _handle) =
        spawn_server_in_process(mock_server, builtins, data_root, mode).await;
    let updates = Arc::new(Mutex::new(Vec::new()));

    let transport = sacp::ByteStreams::new(client_write.compat_write(), client_read.compat());

    ClientToAgent::builder()
        .on_receive_notification(
            {
                let updates = updates.clone();
                async move |notification: SessionNotification, _cx| {
                    updates.lock().unwrap().push(notification);
                    Ok(())
                }
            },
            sacp::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, request_cx, _connection_cx| {
                let response = match select {
                    Some(kind) => {
                        let id = req
                            .options
                            .iter()
                            .find(|o| o.kind == kind)
                            .unwrap()
                            .option_id
                            .clone();
                        RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(id),
                        ))
                    }
                    None => RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                };
                request_cx.respond(response)
            },
            sacp::on_receive_request!(),
        )
        .connect_to(transport)
        .unwrap()
        .run_until({
            let updates = updates.clone();
            move |cx: JrConnectionCx<ClientToAgent>| async move {
                cx.send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                    .block_task()
                    .await
                    .unwrap();

                test_fn(cx.clone(), updates).await;
                Ok(())
            }
        })
        .await
        .unwrap();
}

async fn run_acp_session<F, Fut>(
    mock_server: &MockServer,
    mcp_servers: Vec<McpServer>,
    builtins: &[&str],
    data_root: &Path,
    mode: BioRouterMode,
    select: Option<PermissionOptionKind>,
    test_fn: F,
) where
    F: FnOnce(
        JrConnectionCx<ClientToAgent>,
        sacp::schema::SessionId,
        Arc<Mutex<Vec<SessionNotification>>>,
    ) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let work_dir = tempfile::tempdir().unwrap();

    run_acp_client(
        mock_server,
        builtins,
        data_root,
        mode,
        select,
        move |cx, updates| async move {
            let session = cx
                .send_request(NewSessionRequest::new(work_dir.path()).mcp_servers(mcp_servers))
                .block_task()
                .await
                .unwrap();

            test_fn(cx.clone(), session.session_id, updates).await;
        },
    )
    .await;
}

#[test_case(Some(PermissionOptionKind::AllowAlways), ToolCallStatus::Completed, "user:\n  always_allow:\n  - lookup__get_code\n  ask_before: []\n  never_allow: []\n"; "allow_always")]
#[test_case(Some(PermissionOptionKind::AllowOnce), ToolCallStatus::Completed, ""; "allow_once")]
#[test_case(Some(PermissionOptionKind::RejectAlways), ToolCallStatus::Failed, "user:\n  always_allow: []\n  ask_before: []\n  never_allow:\n  - lookup__get_code\n"; "reject_always")]
#[test_case(Some(PermissionOptionKind::RejectOnce), ToolCallStatus::Failed, ""; "reject_once")]
#[test_case(None, ToolCallStatus::Failed, ""; "cancelled")]
fn test_permission_persistence(
    kind: Option<PermissionOptionKind>,
    expected_status: ToolCallStatus,
    expected_yaml: &str,
) {
    run_async_test(async move {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompt = "Use the get_code tool and output only its result.";
        let (mcp_url, _handle) = spawn_mcp_http_server().await;

        let mock_server = setup_mock_openai(vec![
            (
                format!(r#"</info-msg>\n{prompt}""#),
                include_str!("./test_data/openai_tool_call_response.txt"),
            ),
            (
                format!(r#""content":"{FAKE_CODE}""#),
                include_str!("./test_data/openai_tool_result_response.txt"),
            ),
        ])
        .await;

        run_acp_session(
            &mock_server,
            vec![McpServer::Http(McpServerHttp::new("lookup", mcp_url))],
            &[],
            temp_dir.path(),
            BioRouterMode::Approve,
            kind,
            |cx, session_id, updates| async move {
                cx.send_request(PromptRequest::new(
                    session_id,
                    vec![ContentBlock::Text(TextContent::new(prompt))],
                ))
                .block_task()
                .await
                .unwrap();
                wait_for(
                    &updates,
                    &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        ToolCallId::new(""),
                        ToolCallUpdateFields::new().status(Some(expected_status)),
                    )),
                )
                .await;
            },
        )
        .await;

        assert_eq!(
            fs::read_to_string(temp_dir.path().join("permission.yaml")).unwrap_or_default(),
            expected_yaml
        );
    });
}
