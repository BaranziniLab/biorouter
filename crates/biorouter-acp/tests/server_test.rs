biorouter::keep_this_test_binary_off_the_os_credential_store!();

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
    AudioContent, ContentBlock, ContentChunk, InitializeRequest, LoadSessionRequest, McpServer,
    McpServerHttp, NewSessionRequest, PermissionOptionKind, PromptRequest, ProtocolVersion,
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

/// How long one step of a test may take before it fails. A step here takes well
/// under a second (a debug build on a loaded CI runner takes a few), so a step
/// that runs this long is stuck, and failing it names the step instead of
/// leaving the job to its 90 minute timeout with no output.
const STEP_LIMIT: Duration = Duration::from_secs(60);

/// How long a whole ACP client run may take, from spawning the server to the
/// client's last request. The slowest test here runs a few tool calls.
const CLIENT_RUN_LIMIT: Duration = Duration::from_secs(120);

/// How long the runtime may take to shut down once a test is done. Dropping a
/// runtime waits for every blocking task without a limit, so a stuck one would
/// hang the test after its last assertion.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(30);

fn run_async_test(future: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.block_on(future)));
    let shutdown_started = std::time::Instant::now();
    runtime.shutdown_timeout(SHUTDOWN_LIMIT);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    assert!(
        shutdown_started.elapsed() < SHUTDOWN_LIMIT,
        "the test's runtime did not shut down within {SHUTDOWN_LIMIT:?}: a blocking task it started never returned"
    );
}

/// Await `future`, failing the test with `step` in the message if it takes
/// longer than [`STEP_LIMIT`].
async fn bounded<T>(step: &str, future: impl Future<Output = T>) -> T {
    match tokio::time::timeout(STEP_LIMIT, future).await {
        Ok(value) => value,
        Err(_) => panic!("{step} did not finish within {STEP_LIMIT:?}"),
    }
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
fn test_dictate_command_is_advertised_and_answers_without_a_provider() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock_server = setup_mock_openai(vec![]).await;
        run_acp_session(
            &mock_server,
            vec![],
            &[],
            temp_dir.path(),
            BioRouterMode::Auto,
            None,
            |cx, session_id, updates| async move {
                wait_for_command(&updates, "dictate").await;
                let response = cx
                    .send_request(PromptRequest::new(
                        session_id,
                        vec![ContentBlock::Text(TextContent::new("/dictate"))],
                    ))
                    .block_task()
                    .await
                    .unwrap();
                assert_eq!(response.stop_reason, StopReason::EndTurn);
                wait_for(
                    &updates,
                    &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                        TextContent::new("To dictate, use Dictate to clipboard in the Biorouter app or run `biorouter dictate --copy`, then paste. Your system's own dictation also works, without your Biorouter dictionary."),
                    ))),
                )
                .await;
            },
        )
        .await;
        assert!(mock_server.received_requests().await.unwrap().is_empty());
    });
}

#[test]
fn test_audio_block_gets_the_note_without_processing_the_audio() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock_server = setup_mock_openai(vec![]).await;
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
                        vec![ContentBlock::Audio(AudioContent::new("ZmFrZQ==", "audio/wav"))],
                    ))
                    .block_task()
                    .await
                    .unwrap();
                assert_eq!(response.stop_reason, StopReason::EndTurn);
                wait_for(
                    &updates,
                    &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                        TextContent::new("(An audio clip was attached here. Biorouter does not take audio from this editor, so the clip was left out. To dictate, use Dictate to clipboard in the Biorouter app or run `biorouter dictate --copy`, then paste. Your system's own dictation also works, without your Biorouter dictionary.)"),
                    ))),
                )
                .await;
            },
        )
        .await;
        assert!(mock_server.received_requests().await.unwrap().is_empty());
    });
}

/// DD-82 and S12: in a prompt that also carries text, the audio arm becomes a
/// text part with MSG-ACP-NOTE, so the provider gets the text and the note and
/// never the clip.
#[test]
fn test_mixed_prompt_sends_the_text_and_the_note_but_not_the_audio() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompt = "what is 1+1";
        let mock_server = setup_mock_openai(vec![(
            "the clip was left out".to_string(),
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
                        vec![
                            ContentBlock::Text(TextContent::new(prompt)),
                            ContentBlock::Audio(AudioContent::new("ZmFrZQ==", "audio/wav")),
                        ],
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
        let bodies: Vec<String> = mock_server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let reached = bodies
            .iter()
            .any(|body| body.contains(prompt) && body.contains("the clip was left out"));
        assert!(reached, "the provider got the text and the note");
        assert!(!bodies.iter().any(|body| body.contains("ZmFrZQ==")));
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

                    wait_for_command(&updates, "dictate").await;

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
        let (chat, empty, chat_before, empty_before) =
            bounded("writing the two chats before the load", async {
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
                (chat, empty, chat_before, empty_before)
            })
            .await;
        bounded("closing the store that wrote the chats", sm.close()).await;
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
                        let step = format!("session/load of {id}");
                        bounded(
                            &step,
                            cx.send_request(LoadSessionRequest::new(id.clone(), cwd.clone()))
                                .block_task(),
                        )
                        .await
                        .unwrap_or_else(|e| panic!("{step} failed: {e}"));
                    }
                }
            },
        )
        .await;

        let sm = SessionManager::new(data_dir.path().to_path_buf());
        let (chat_after, empty_after) = bounded("reading the two chats after the load", async {
            (
                sm.get_session(&chat.id, false).await.unwrap(),
                sm.get_session(&empty.id, false).await.unwrap(),
            )
        })
        .await;
        bounded("closing the store that read the chats", sm.close()).await;

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

/// A `session/load` the server cannot complete is answered with the error, and
/// the connection stays open for the client's next request.
///
/// The handler used to hand the error back to sacp, which treats an error from
/// a handler as a reason to close the connection and sends no response. The
/// client then waited for an answer that never came: measured with this test
/// and no watch on the server, the load was still waiting when the 60 second
/// step limit ended it, with nothing in the log saying which request or why.
/// A `session/new` on the same connection afterwards shows it is still open.
#[test]
fn test_acp_failed_load_is_answered_and_the_connection_stays_open() {
    run_async_test(async {
        let data_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        let mock_server = setup_mock_openai(Vec::new()).await;

        run_acp_client(
            &mock_server,
            &[],
            data_dir.path(),
            BioRouterMode::Auto,
            None,
            {
                let cwd = work_dir.path().to_path_buf();
                move |cx, updates| async move {
                    let error = bounded(
                        "session/load of a chat that does not exist",
                        cx.send_request(LoadSessionRequest::new("no-such-chat", cwd.clone()))
                            .block_task(),
                    )
                    .await
                    .expect_err("loading a chat that does not exist must fail");
                    assert_eq!(error.code, sacp::Error::invalid_params().code);
                    assert!(
                        error
                            .to_string()
                            .contains("Failed to load session no-such-chat"),
                        "the error names the chat it could not load: {error}"
                    );

                    let session = bounded(
                        "session/new after the failed load",
                        cx.send_request(NewSessionRequest::new(cwd)).block_task(),
                    )
                    .await
                    .expect("the connection still serves after a failed load");
                    wait_for_command(&updates, "dictate").await;
                    assert!(
                        !updates
                            .lock()
                            .unwrap()
                            .iter()
                            .any(|notification| &*notification.session_id.0 == "no-such-chat"),
                        "a load that failed announced nothing for its chat"
                    );
                    assert!(!session.session_id.0.is_empty());
                }
            },
        )
        .await;
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

async fn wait_for_command(updates: &Arc<Mutex<Vec<SessionNotification>>>, name: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    loop {
        let found = updates.lock().unwrap().iter().any(|notification| {
            matches!(
                &notification.update,
                SessionUpdate::AvailableCommandsUpdate(update)
                    if update.available_commands.iter().any(|command| command.name == name)
            )
        });
        if found {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "ACP session did not advertise {name}"
        );
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
    tokio::task::JoinHandle<Result<(), String>>,
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
        serve(agent, server_read.compat(), server_write.compat_write())
            .await
            .map_err(|e| format!("{e:#}"))
    });

    (client_read, client_write, handle)
}

/// Connect one ACP client to a freshly spawned in-process server and run `test_fn`
/// after the `initialize` handshake. The caller decides whether to create a new
/// session or load an existing one, so a test can simulate a client restart by
/// calling this twice against the same `data_root`.
///
/// The server only stops when the client hangs up, so a server that stops first
/// has dropped the connection, and a request it never answered would leave the
/// client waiting forever: the run fails at once with the server's own error.
/// The whole run is also bounded by [`CLIENT_RUN_LIMIT`].
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
    let run = async {
        let (client_read, client_write, server) =
            spawn_server_in_process(mock_server, builtins, data_root, mode).await;
        let client = run_acp_client_on(client_read, client_write, select, test_fn);
        tokio::select! {
            () = client => {}
            ended = server => panic!(
                "the ACP server stopped before the client finished, so a request may never be answered: {ended:?}"
            ),
        }
    };
    if tokio::time::timeout(CLIENT_RUN_LIMIT, run).await.is_err() {
        panic!("the ACP client run did not finish within {CLIENT_RUN_LIMIT:?}");
    }
}

async fn run_acp_client_on<F, Fut>(
    client_read: tokio::io::DuplexStream,
    client_write: tokio::io::DuplexStream,
    select: Option<PermissionOptionKind>,
    test_fn: F,
) where
    F: FnOnce(JrConnectionCx<ClientToAgent>, Arc<Mutex<Vec<SessionNotification>>>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
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
                let initialized = cx
                    .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                    .block_task()
                    .await
                    .unwrap();
                assert!(!initialized.agent_capabilities.prompt_capabilities.audio);

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
