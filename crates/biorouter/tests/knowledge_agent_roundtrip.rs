//! A credential-free BioOKF round trip through both the curation macros and an
//! ordinary `Agent::reply` turn.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::knowledge::{
    convert::SourceInput,
    macros::{
        ingest::{ingest, IngestArgs},
        lint::{lint, LintArgs},
        query::{query, QueryArgs},
    },
    service::{KnowledgeService, PrimaryUpdate},
    store,
    subagent::loop_::{Completer, LlmMessage, LlmReply, LlmToolCall, SubAgentBounds},
    types::KbFormat,
};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::Mutex;

const KB_ID: &str = "bio-roundtrip";
const OTHER_KB_ID: &str = "bio-roundtrip-distractor";
const SOURCE_TITLE: &str = "BioRouter QZ-17 Roundtrip Study";
const MARKER: &str = "BIOOKF_ROUNDTRIP_QZ17_RESPONSE_731";
const OTHER_MARKER: &str = "BIOOKF_DISTRACTOR_RESPONSE_999";

struct CannedCompleter {
    replies: Mutex<Vec<LlmReply>>,
}

impl CannedCompleter {
    fn new(replies: Vec<LlmReply>) -> Self {
        Self {
            replies: Mutex::new(replies),
        }
    }
}

#[async_trait]
impl Completer for CannedCompleter {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[LlmMessage],
        _tools: &[Tool],
    ) -> Result<LlmReply> {
        let mut replies = self.replies.lock().await;
        Ok(replies.remove(0))
    }
}

struct GroundedMacroQuery {
    calls: AtomicUsize,
}

#[async_trait]
impl Completer for GroundedMacroQuery {
    async fn complete(
        &self,
        _system: &str,
        messages: &[LlmMessage],
        tools: &[Tool],
    ) -> Result<LlmReply> {
        assert!(tools.iter().any(|tool| tool.name == "kb_search"));
        assert!(!tools.iter().any(|tool| tool.name == "kb_write_page"));

        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(tool_reply(
                "kb_search",
                json!({"query": "QZ-17 response 731", "limit": 5}),
            ));
        }

        let results = macro_tool_results(messages);
        assert!(
            results.contains(MARKER),
            "the query completer did not receive the curated marker: {results}"
        );
        Ok(text_reply(&format!("Macro query grounded in {MARKER}.")))
    }
}

struct KnowledgeRoundtripProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for KnowledgeRoundtripProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> std::result::Result<(Message, ProviderUsage), ProviderError> {
        let tool_names = tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        assert!(
            tool_names.iter().any(|name| name == "knowledge__kb_search"),
            "the ordinary turn did not receive the knowledge search tool: {tool_names:?}"
        );
        assert!(
            tool_names
                .iter()
                .any(|name| name == "knowledge__kb_list_bases"),
            "the ordinary turn did not receive the knowledge listing tool: {tool_names:?}"
        );
        assert!(
            tool_names
                .iter()
                .any(|name| name == "knowledge__kb_get_active"),
            "the ordinary turn did not receive the knowledge selection tool: {tool_names:?}"
        );

        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let reply = match call {
            0 => {
                let prompt_context = messages
                    .iter()
                    .map(Message::as_concat_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    prompt_context.contains(MARKER),
                    "the explicit /kb attachment did not prefetch the curated marker: {prompt_context}"
                );
                provider_tool_reply("agent-list", "knowledge__kb_list_bases", json!({}))
            }
            1 => {
                let results = provider_tool_results(messages);
                assert!(
                    results.contains(KB_ID) && results.contains(OTHER_KB_ID),
                    "the two session-visible bases were absent from kb_list_bases: {results}"
                );
                provider_tool_reply("agent-selection", "knowledge__kb_get_active", json!({}))
            }
            2 => {
                let active = latest_provider_tool_result_json(messages);
                assert_eq!(active["primary_kb"].as_str(), Some(KB_ID));
                let visible = active["knowledge_bases"]
                    .as_array()
                    .expect("knowledge_bases is an array");
                assert!(
                    visible.iter().any(|id| id.as_str() == Some(KB_ID)),
                    "{active}"
                );
                assert!(
                    visible.iter().any(|id| id.as_str() == Some(OTHER_KB_ID)),
                    "{active}"
                );
                provider_tool_reply(
                    "agent-search",
                    "knowledge__kb_search",
                    json!({"kb_id": KB_ID, "query": "QZ-17 response 731", "limit": 5}),
                )
            }
            _ => {
                let results = latest_provider_tool_result(messages);
                assert!(
                    results.contains(MARKER),
                    "the explicitly targeted search did not return the curated marker: {results}"
                );
                assert!(
                    !results.contains(OTHER_MARKER),
                    "the explicitly targeted search returned the other visible base: {results}"
                );
                Message::assistant().with_text(format!("Grounded ordinary answer: {MARKER}"))
            }
        };

        Ok((
            reply,
            ProviderUsage::new("knowledge-roundtrip".to_string(), Usage::default()),
        ))
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> std::result::Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system_prompt, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("knowledge-roundtrip").unwrap()
    }

    fn metadata() -> ProviderMetadata
    where
        Self: Sized,
    {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "knowledge-roundtrip"
    }
}

fn tool_reply(name: &str, args: serde_json::Value) -> LlmReply {
    LlmReply {
        text: String::new(),
        tool_calls: vec![LlmToolCall {
            id: format!("macro-{name}"),
            name: name.to_string(),
            args,
        }],
    }
}

fn text_reply(text: &str) -> LlmReply {
    LlmReply {
        text: text.to_string(),
        tool_calls: vec![],
    }
}

fn macro_tool_results(messages: &[LlmMessage]) -> String {
    messages
        .iter()
        .flat_map(|message| match message {
            LlmMessage::ToolResult { content, .. } => vec![content.as_str()],
            LlmMessage::ToolResults(parts) => {
                parts.iter().map(|part| part.content.as_str()).collect()
            }
            _ => vec![],
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn provider_tool_reply(id: &str, name: &str, args: serde_json::Value) -> Message {
    Message::assistant().with_tool_request(
        id,
        Ok(CallToolRequestParams {
            task: None,
            meta: None,
            name: name.to_string().into(),
            arguments: Some(
                args.as_object()
                    .expect("tool arguments are an object")
                    .clone(),
            ),
        }),
    )
}

fn provider_tool_results(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|content| match content {
            MessageContent::ToolResponse(response) => Some(match &response.tool_result {
                Ok(result) => result
                    .content
                    .iter()
                    .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(error) => error.to_string(),
            }),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn latest_provider_tool_result(messages: &[Message]) -> String {
    for message in messages.iter().rev() {
        for content in message.content.iter().rev() {
            if let MessageContent::ToolResponse(response) = content {
                return match &response.tool_result {
                    Ok(result) => result
                        .content
                        .iter()
                        .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Err(error) => error.to_string(),
                };
            }
        }
    }
    String::new()
}

fn latest_provider_tool_result_json(messages: &[Message]) -> serde_json::Value {
    let guarded = latest_provider_tool_result(messages);
    let start = guarded
        .find('{')
        .expect("guarded tool result contains a JSON object");
    let end = guarded
        .rfind('}')
        .expect("guarded tool result contains a complete JSON object");
    let json = guarded
        .get(start..=end)
        .expect("tool-result JSON boundaries are UTF-8 character boundaries");
    serde_json::from_str(json).expect("tool-result JSON is valid")
}

async fn drain_reply(agent: &Agent, session_id: &str) -> Result<Vec<Message>> {
    let stream = agent
        .reply(
            Message::user().with_text(format!(
                "/kb:{KB_ID} What calibrated response was recorded for QZ-17?"
            )),
            SessionConfig {
                id: session_id.to_string(),
                schedule_id: None,
                max_turns: Some(8),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await?;
    tokio::pin!(stream);

    let mut messages = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event? {
            assert!(
                !matches!(
                    message.content.first(),
                    Some(MessageContent::ActionRequired(_))
                ),
                "public knowledge reads unexpectedly required confirmation: {message:?}"
            );
            messages.push(message);
        }
    }
    Ok(messages)
}

#[tokio::test]
#[serial_test::serial]
async fn biookf_curation_reaches_an_attached_ordinary_agent_turn() {
    let path_root = TempDir::new().unwrap();
    let _env = env_lock::lock_env([(
        "BIOROUTER_PATH_ROOT",
        Some(path_root.path().to_string_lossy().into_owned()),
    )]);

    let svc = KnowledgeService::new_default().unwrap();
    svc.create_base_in(KB_ID, "BioOKF roundtrip", None, KbFormat::Biookf)
        .unwrap();
    svc.create_base_in(
        OTHER_KB_ID,
        "BioOKF roundtrip distractor",
        None,
        KbFormat::Biookf,
    )
    .unwrap();
    store::write_page(
        &svc.root().join(OTHER_KB_ID),
        "knowledge/entities/distractor.md",
        &format!(
            "---\ntype: Molecule\nidentifier: DISTRACTOR\n---\n\nA different response was 999. {OTHER_MARKER}"
        ),
        "add roundtrip distractor",
        None,
    )
    .unwrap();

    let ingest_result = ingest(
        &svc,
        IngestArgs {
            kb_id: KB_ID.to_string(),
            caller_is_private: false,
            caller_affiliation: Default::default(),
            source: SourceInput::Text {
                text: format!(
                    "QZ-17 produced calibrated response 731. Distinctive marker: {MARKER}."
                ),
                title: Some(SOURCE_TITLE.to_string()),
            },
            completer: Box::new(CannedCompleter::new(vec![
                tool_reply(
                    "kb_write_concept",
                    json!({
                        "type": "Molecule",
                        "identifier": "QZ-17",
                        "description": format!("Calibrated response 731; marker {MARKER}"),
                        "body": format!("# QZ-17\n\nThe calibrated response was 731. {MARKER}"),
                        "edges": [{
                            "predicate": "reported_in",
                            "object": SOURCE_TITLE,
                            "knowledge_level": "observation",
                            "agent_type": "manual_agent",
                            "primary_source": SOURCE_TITLE
                        }]
                    }),
                ),
                text_reply("curation complete"),
            ])),
            focus: Some("QZ-17 calibrated response".to_string()),
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .unwrap();
    assert!(ingest_result.verification.ok, "{ingest_result:#?}");

    let lint_result = lint(
        &svc,
        LintArgs {
            kb_id: KB_ID.to_string(),
            caller_is_private: false,
            caller_affiliation: Default::default(),
            completer: None,
            autofix: false,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(lint_result.report.diagnostics.errors(), 0);
    assert!(lint_result.commit_sha.is_none());

    let query_result = query(
        &svc,
        QueryArgs {
            kb_id: KB_ID.to_string(),
            caller_is_private: false,
            caller_affiliation: Default::default(),
            question: "What calibrated response was recorded for QZ-17?".to_string(),
            completer: Box::new(GroundedMacroQuery {
                calls: AtomicUsize::new(0),
            }),
            file_as_page: false,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .unwrap();
    assert!(query_result.answer.contains(MARKER));
    assert!(query_result.commit_sha.is_none());

    let session_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(session_dir.path().to_path_buf()));
    let permission_dir = TempDir::new().unwrap();
    let agent = Agent::with_config(AgentConfig::new(
        Arc::clone(&session_manager),
        Arc::new(PermissionManager::new(permission_dir.path().to_path_buf())),
        None,
        BioRouterMode::Auto,
    ));
    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "knowledge".to_string(),
            description: "knowledge roundtrip".to_string(),
            display_name: Some("Knowledge".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .unwrap();

    let work_dir = TempDir::new().unwrap();
    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "knowledge-roundtrip".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    let selection = svc
        .set_selection(Some(&session.id), Some(&[]), PrimaryUpdate::Set(KB_ID))
        .unwrap();
    assert_eq!(
        selection.kb_ids,
        vec![KB_ID.to_string(), OTHER_KB_ID.to_string()]
    );
    assert_eq!(selection.primary_kb.as_deref(), Some(KB_ID));

    agent
        .update_provider(
            Arc::new(KnowledgeRoundtripProvider {
                calls: AtomicUsize::new(0),
            }),
            &session.id,
        )
        .await
        .unwrap();

    let messages = drain_reply(&agent, &session.id).await.unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message.as_concat_text().contains(MARKER)),
        "the ordinary turn never returned its grounded answer: {messages:#?}"
    );
    let tool_results = provider_tool_results(&messages);
    assert!(tool_results.contains(KB_ID), "{tool_results}");
    assert!(tool_results.contains(MARKER), "{tool_results}");
}
