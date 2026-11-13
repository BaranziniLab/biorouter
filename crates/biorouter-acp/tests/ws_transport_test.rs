//! End-to-end test for the ACP **WebSocket** transport: a real WS server
//! (`serve_ws`) backed by a mocked provider, driven by a real WS client speaking
//! ACP over sacp's message-based `Lines` transport. Proves the WS framing
//! adapter round-trips initialize -> new session -> prompt and streams the
//! agent's reply back over the socket.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod common;

use biorouter::config::BioRouterMode;
use biorouter::model::ModelConfig;
use biorouter::providers::api_client::{ApiClient, AuthMethod};
use biorouter::providers::openai::OpenAiProvider;
use biorouter_acp::server::{serve_ws, BioRouterAcpAgent, BioRouterAcpConfig};
use common::setup_mock_openai;
use futures::{SinkExt, StreamExt};
use sacp::schema::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, ProtocolVersion,
    SessionNotification, SessionUpdate, StopReason, TextContent,
};
use sacp::{ClientToAgent, JrConnectionCx};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

fn run_async_test(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[test]
fn ws_acp_basic_completion() {
    run_async_test(async {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompt = "what is 1+1";
        let mock_server = setup_mock_openai(vec![(
            format!(r#"</info-msg>\n{prompt}""#),
            include_str!("./test_data/openai_basic_response.txt"),
        )])
        .await;

        // ---- server: an ACP WebSocket endpoint backed by the mock provider ----
        let api_client = ApiClient::new(
            mock_server.uri(),
            AuthMethod::BearerToken("test-key".to_string()),
        )
        .unwrap();
        let provider = OpenAiProvider::new(api_client, ModelConfig::new("gpt-5-nano").unwrap());
        let config = BioRouterAcpConfig {
            provider: Arc::new(provider),
            builtins: vec![],
            work_dir: temp_dir.path().to_path_buf(),
            data_dir: temp_dir.path().to_path_buf(),
            config_dir: temp_dir.path().to_path_buf(),
            biorouter_mode: BioRouterMode::Auto,
            extensions: vec![],
        };
        let agent = Arc::new(BioRouterAcpAgent::with_config(config).await.unwrap());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = serve_ws(agent, ws).await;
        });

        // ---- client: connect over ws:// and speak ACP via sacp Lines ----
        let (ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        let (sink, stream) = ws.split();
        let outgoing = sink
            .sink_map_err(io_err)
            .with(|line: String| async move { Ok::<Message, std::io::Error>(Message::text(line)) });
        let incoming = stream.filter_map(|m| async move {
            match m {
                Ok(Message::Text(t)) => Some(Ok(t.to_string())),
                Ok(Message::Close(_)) => None,
                Ok(_) => None,
                Err(e) => Some(Err(io_err(e))),
            }
        });
        let transport = sacp::Lines::new(outgoing, incoming);

        let updates = Arc::new(Mutex::new(Vec::<SessionNotification>::new()));
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
            .connect_to(transport)
            .unwrap()
            .run_until({
                let updates = updates.clone();
                let work_dir = temp_dir.path().to_path_buf();
                move |cx: JrConnectionCx<ClientToAgent>| async move {
                    cx.send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                        .block_task()
                        .await
                        .unwrap();
                    let session = cx
                        .send_request(NewSessionRequest::new(work_dir))
                        .block_task()
                        .await
                        .unwrap();
                    let response = cx
                        .send_request(PromptRequest::new(
                            session.session_id,
                            vec![ContentBlock::Text(TextContent::new(prompt))],
                        ))
                        .block_task()
                        .await
                        .unwrap();
                    assert_eq!(response.stop_reason, StopReason::EndTurn);

                    // The streamed agent reply, received over the WebSocket,
                    // should contain "2".
                    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
                    loop {
                        let got: String = {
                            let g = updates.lock().unwrap();
                            g.iter()
                                .filter_map(|n| match &n.update {
                                    SessionUpdate::AgentMessageChunk(c) => match &c.content {
                                        ContentBlock::Text(t) => Some(t.text.clone()),
                                        _ => None,
                                    },
                                    _ => None,
                                })
                                .collect()
                        };
                        if got.contains('2') {
                            break;
                        }
                        if tokio::time::Instant::now() > deadline {
                            panic!("did not receive '2' over WebSocket; got: {got:?}");
                        }
                        tokio::task::yield_now().await;
                    }
                    Ok(())
                }
            })
            .await
            .unwrap();
    });
}
