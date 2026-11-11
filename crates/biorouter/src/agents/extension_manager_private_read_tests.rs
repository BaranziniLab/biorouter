// DR-33, design §2.6 and §3.9, at the dispatch choke point: a result that
// carries private content ratchets the chat that receives it, before it is
// returned, and Gate C's refusal names the decision model that made a chat
// public.

use super::*;
use crate::privacy::{CallCapability, ProviderTier, SessionClassification};
use rmcp::model::{
    CallToolResult, GetPromptResult, InitializeResult, JsonObject, ListPromptsResult,
    ListResourcesResult, ListToolsResult, ReadResourceResult, ServerNotification,
};
use rmcp::{object, ServiceError as Error};
use tokio::sync::mpsc;

/// A client that answers every call with one scripted result.
struct ScriptedClient {
    result: CallToolResult,
}

#[async_trait::async_trait]
impl McpClientTrait for ScriptedClient {
    fn get_info(&self) -> Option<&InitializeResult> {
        None
    }
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        Err(Error::TransportClosed)
    }
    async fn read_resource(
        &self,
        _uri: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        Err(Error::TransportClosed)
    }
    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: vec![],
            next_cursor: None,
            meta: None,
        })
    }
    async fn call_tool(
        &self,
        _name: &str,
        _arguments: Option<JsonObject>,
        _meta: McpMeta,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        Ok(self.result.clone())
    }
    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }
    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: Value,
        _cancellation_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }
    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        mpsc::channel(1).1
    }
}

fn marked(bases: &[&str]) -> CallToolResult {
    let mut result = CallToolResult::success(vec![rmcp::model::Content::text("page body")]);
    biorouter_mcp::knowledge::tier::mark_private_content(
        &mut result,
        &bases.iter().map(|b| b.to_string()).collect::<Vec<_>>(),
    );
    result
}

async fn manager() -> (
    tempfile::TempDir,
    ExtensionManager,
    Arc<crate::session::SessionManager>,
    String,
) {
    let dir = tempfile::tempdir().unwrap();
    let em = ExtensionManager::new_without_provider(dir.path().to_path_buf());
    let sm = em.get_context().session_manager.clone();
    let reader = sm
        .create_session(
            PathBuf::from("."),
            "reader".to_string(),
            crate::session::session_manager::SessionType::User,
        )
        .await
        .unwrap();
    (dir, em, sm, reader.id)
}

fn call_with(name: &str, arguments: JsonObject) -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        name: name.to_string().into(),
        arguments: Some(arguments),
        meta: None,
    }
}

async fn dispatch(
    em: &ExtensionManager,
    reader: &str,
    call: CallToolRequestParams,
    cap: CallCapability,
) -> Result<CallToolResult, ErrorData> {
    let dispatched = em
        .dispatch_tool_call(reader, call, cap, CancellationToken::default())
        .await
        .expect("admitted");
    dispatched.result.await
}

async fn tier_of(
    sm: &crate::session::SessionManager,
    id: &str,
) -> (SessionClassification, Option<String>) {
    let row = sm.get_session(id, false).await.unwrap();
    (row.privacy_tier, row.privacy_reason)
}

fn private_cap() -> CallCapability {
    CallCapability::for_test(ProviderTier::Private, true)
}

#[tokio::test]
async fn a_marked_built_in_result_ratchets_the_reader_with_the_base_named() {
    let (_dir, em, sm, reader) = manager().await;
    em.add_mock_extension(
        "knowledge".to_string(),
        Arc::new(ScriptedClient {
            result: marked(&["omop-notes"]),
        }),
    )
    .await;

    let result = dispatch(
        &em,
        &reader,
        call_with("knowledge__kb_read_page", object!({})),
        private_cap(),
    )
    .await
    .expect("the result is returned once the read is recorded");
    assert!(!result.content.is_empty());
    assert_eq!(
        tier_of(&sm, &reader).await,
        (
            SessionClassification::Private,
            Some("kb:omop-notes".to_string())
        )
    );
}

#[tokio::test]
async fn an_unmarked_result_and_a_switched_off_sample_change_nothing() {
    let (_dir, em, sm, reader) = manager().await;
    em.add_mock_extension(
        "knowledge".to_string(),
        Arc::new(ScriptedClient {
            result: CallToolResult::success(vec![rmcp::model::Content::text("public page")]),
        }),
    )
    .await;
    dispatch(
        &em,
        &reader,
        call_with("knowledge__kb_read_page", object!({})),
        private_cap(),
    )
    .await
    .unwrap();
    assert_eq!(
        tier_of(&sm, &reader).await,
        (SessionClassification::Public, None)
    );

    let (_dir2, em2, sm2, reader2) = manager().await;
    em2.add_mock_extension(
        "knowledge".to_string(),
        Arc::new(ScriptedClient {
            result: marked(&["omop-notes"]),
        }),
    )
    .await;
    // AR-7: with the master switch off as sampled, nothing ratchets.
    dispatch(
        &em2,
        &reader2,
        call_with("knowledge__kb_read_page", object!({})),
        CallCapability::for_test(ProviderTier::Private, false),
    )
    .await
    .unwrap();
    assert_eq!(
        tier_of(&sm2, &reader2).await,
        (SessionClassification::Public, None)
    );
}

/// An installed server is not told the capability, and its mark is not
/// believed: otherwise any third party could reclassify the chats that call it.
#[tokio::test]
async fn a_third_party_servers_mark_is_not_believed() {
    let (_dir, em, sm, reader) = manager().await;
    em.add_mock_third_party_extension(
        "notabuiltin",
        Arc::new(ScriptedClient {
            result: marked(&["omop-notes"]),
        }),
    )
    .await;
    dispatch(
        &em,
        &reader,
        call_with("notabuiltin__tool", object!({})),
        private_cap(),
    )
    .await
    .unwrap();
    assert_eq!(
        tier_of(&sm, &reader).await,
        (SessionClassification::Public, None)
    );
}

#[tokio::test]
async fn reading_a_private_conversation_ratchets_the_reader() {
    let (_dir, em, sm, reader) = manager().await;
    let target = sm
        .create_session(
            PathBuf::from("."),
            "target".to_string(),
            crate::session::session_manager::SessionType::User,
        )
        .await
        .unwrap();
    em.add_mock_extension(
        "workspace".to_string(),
        Arc::new(ScriptedClient {
            result: CallToolResult::success(vec![rmcp::model::Content::text("transcript")]),
        }),
    )
    .await;
    let read = || {
        call_with(
            "workspace__workspace_read_conversation",
            object!({ "session_id": target.id.clone() }),
        )
    };

    // A public target: nothing to record.
    dispatch(&em, &reader, read(), private_cap()).await.unwrap();
    assert_eq!(
        tier_of(&sm, &reader).await,
        (SessionClassification::Public, None)
    );

    sm.update(&target.id)
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    dispatch(&em, &reader, read(), private_cap()).await.unwrap();
    assert_eq!(
        tier_of(&sm, &reader).await,
        (
            SessionClassification::Private,
            Some(format!("read:{}", target.id))
        )
    );
}

#[tokio::test]
async fn a_refused_conversation_read_ratchets_nobody() {
    let (_dir, em, sm, reader) = manager().await;
    let target = sm
        .create_session(
            PathBuf::from("."),
            "target".to_string(),
            crate::session::session_manager::SessionType::User,
        )
        .await
        .unwrap();
    sm.update(&target.id)
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    em.add_mock_extension(
        "workspace".to_string(),
        Arc::new(ScriptedClient {
            result: CallToolResult::error(vec![rmcp::model::Content::text(
                "That conversation is out of reach.",
            )]),
        }),
    )
    .await;
    dispatch(
        &em,
        &reader,
        call_with(
            "workspace__workspace_read_conversation",
            object!({ "session_id": target.id.clone() }),
        ),
        private_cap(),
    )
    .await
    .unwrap();
    assert_eq!(
        tier_of(&sm, &reader).await,
        (SessionClassification::Public, None)
    );
}

#[test]
fn only_an_enforced_private_call_is_probed() {
    let args = object!({ "session_id": "s-1" });
    let read = |cap| {
        PrivateContentRead::for_call("workspace", "workspace_read_conversation", Some(&args), cap)
    };
    assert_eq!(
        read(private_cap()),
        Some(PrivateContentRead::ConversationRead("s-1".to_string()))
    );
    assert_eq!(
        read(CallCapability::for_test(ProviderTier::Public, true)),
        None
    );
    assert_eq!(
        read(CallCapability::for_test(ProviderTier::Private, false)),
        None
    );
    assert_eq!(
        PrivateContentRead::for_call("knowledge", "kb_search", None, private_cap()),
        Some(PrivateContentRead::BuiltinMark)
    );
    assert_eq!(
        PrivateContentRead::for_call("someones_server", "tool", None, private_cap()),
        None
    );
    // Another workspace tool is not a conversation read.
    assert_eq!(
        PrivateContentRead::for_call("workspace", "workspace_list", Some(&args), private_cap()),
        None
    );
}

/// The census: every permit of private content into a chat goes through one
/// writer, and these are its callers. A new read that hands a chat private
/// content belongs here, with its reason.
#[test]
fn every_read_ratchet_goes_through_raise_reader_privacy() {
    fn production(src: &str) -> &str {
        src.split("\n#[cfg(test)]\nmod tests").next().unwrap()
    }
    let count = |src: &str| production(src).matches("raise_reader_privacy(").count();
    // The definition, Gate C's private dispatch (`raise_session_privacy`), and
    // the post-call ratchet of a marked or conversation-read result.
    assert_eq!(count(include_str!("extension_manager.rs")), 3);
    // Chat recall's LOAD, search and recent modes share one helper.
    assert_eq!(count(include_str!("chatrecall_extension.rs")), 1);
}

/// A private provider whose slot a decision model lowered, for Gate C's words.
struct PrivateModel;

#[async_trait::async_trait]
impl crate::providers::base::Provider for PrivateModel {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new("versa-mock", "", "", "m", vec![], "", vec![])
    }
    fn get_name(&self) -> &str {
        "versa-mock"
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }
    fn affiliation(&self) -> Option<crate::privacy::ModelAffiliation> {
        Some(crate::privacy::ModelAffiliation::Local)
    }
    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("m")
    }
    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        _system: &str,
        _messages: &[crate::conversation::message::Message],
        _tools: &[rmcp::model::Tool],
    ) -> std::result::Result<
        (
            crate::conversation::message::Message,
            crate::providers::base::ProviderUsage,
        ),
        crate::providers::errors::ProviderError,
    > {
        unreachable!("no turn runs here")
    }
}

/// DR-33 §3.9: a lowered chat is refused a private extension like any public
/// chat, and the refusal says why in terms it can act on, not "switch to a
/// private model", which it already runs on.
#[tokio::test]
async fn gate_c_names_the_decision_model_that_made_the_chat_public() {
    use crate::privacy::decision_axes::{DecisionAxes, HostLabel, LoweredBy, NameSource};
    let (_dir, em, _sm, reader) = manager().await;
    em.add_mock_extension(
        "ucsfomopagent".to_string(),
        Arc::new(ScriptedClient {
            result: CallToolResult::success(vec![]),
        }),
    )
    .await;
    let cause = LoweredBy {
        source: NameSource::Monitor,
        label: "Done".to_string(),
        model: "joey-1".to_string(),
        host: HostLabel::Baseten,
        others: 0,
    };
    *em.provider.lock().await = Some(
        crate::providers::decision_bound::DecisionBoundProvider::bind(
            Arc::new(PrivateModel),
            DecisionAxes::public_for_test(cause),
        ),
    );

    let refused = match em
        .dispatch_tool_call(
            &reader,
            call_with("ucsfomopagent__tool", object!({})),
            CallCapability::for_test_lowered_by_decision(true),
            CancellationToken::default(),
        )
        .await
    {
        Ok(_) => panic!("a lowered chat reached a private extension"),
        Err(e) => e.to_string(),
    };
    assert!(refused.contains("public decision model"), "{refused}");
    assert!(refused.contains("joey-1"), "{refused}");
    assert!(
        !refused.contains("switch this chat to a private model"),
        "circular advice for a chat that runs on a private model: {refused}"
    );

    // The same refusal on a chat public by its own model keeps today's words.
    let plain = match em
        .dispatch_tool_call(
            &reader,
            call_with("ucsfomopagent__tool", object!({})),
            CallCapability::for_test(ProviderTier::Public, true),
            CancellationToken::default(),
        )
        .await
    {
        Ok(_) => panic!("a public chat reached a private extension"),
        Err(e) => e.to_string(),
    };
    assert!(
        plain.contains("switch this chat to a private model"),
        "{plain}"
    );
}
