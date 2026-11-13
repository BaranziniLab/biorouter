//! Bridges `biorouter::providers::Provider` → `biorouter_mcp::knowledge::subagent::loop_::Completer`.
//!
//! The `Completer` trait was introduced in Plan 2 to avoid a circular dep on `biorouter`
//! from within `biorouter-mcp`. This adapter lives in `biorouter` (which already depends on
//! `biorouter-mcp`) and lets HTTP handlers in `biorouter-server` pass a user-selected
//! `Provider` into the Plan-2 macros (ingest / query / lint / agentic credibility).

use crate::conversation::message::{Message, MessageContent};
use crate::providers::base::Provider;
use crate::providers::coding_agent::bridge::BridgeToolDispatch;
use crate::providers::tool_turn::ProviderToolTurnContext;
use anyhow::Result;
use async_trait::async_trait;
use biorouter_mcp::knowledge::subagent::loop_::{
    Completer, CompleterTurn, ExecutedCall, LlmMessage, LlmReply, LlmToolCall, ToolDispatch,
    ToolResultPart,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Tool};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// A fresh [`Completer`] per call.
///
/// `IngestArgs::completer` is a `Box<dyn Completer>` and every knowledge macro
/// **consumes** it, so ingesting a batch needs one per source. A factory rather
/// than a `Vec` because the count is not known until the caller's paths have
/// been expanded, and because every element must come off the *one* provider
/// whose tier and affiliation were reported — see
/// [`ProviderCompleter::paired_factory`].
pub type CompleterFactory = Box<dyn Fn() -> Box<dyn Completer> + Send + Sync>;

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

pub struct ProviderCompleter {
    /// Private, with [`Self::new`], so that [`Self::paired`] is the only way any
    /// other module can obtain a `ProviderCompleter` at all — see its doc.
    provider: Arc<dyn Provider>,
    /// Which session a human-decision card raised during a bridged turn belongs
    /// to (#107 / #109).
    ///
    /// `None` means the run has no chat behind it — the Knowledge view's ingest
    /// panel, for example. Safe workflow calls can still run, but an inspector
    /// that requires a person fails immediately with
    /// `approval_unavailable_without_session`; no unroutable card is published.
    session_id: Option<String>,
    /// The run's cancellation, so a bridged tool call is reachable by whatever
    /// stops the run.
    cancel: Option<CancellationToken>,
}

impl ProviderCompleter {
    fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            provider,
            session_id: None,
            cancel: None,
        }
    }

    /// Scope any human-decision card this completer's turns raise to
    /// `session_id`. See the field's doc for what `None` means.
    #[must_use]
    pub fn in_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Bind the run's cancellation, so a bridged tool call and a parked approval
    /// are both reachable by whatever stops the run.
    #[must_use]
    pub fn cancelled_by(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// The completer **and** the tier of the provider behind it, from one
    /// binding (issue #56).
    ///
    /// The **only** constructor visible outside this module: `new` and the
    /// `provider` field are both private, so no other module can pair a
    /// completer with a tier it looked up separately, and no grep or convention
    /// is load-bearing for that. It closes the defect a ban on hardcoded
    /// literals leaves open — the one the CLI and the probe are most exposed to,
    /// because they resolve a provider by NAME.
    ///
    /// [`Provider::tier`] is an instance method for exactly this reason:
    /// `providers::create("ollama", ..)` can return a lead/worker composite
    /// (the factory intercepts `BIOROUTER_LEAD_MODEL` *before* the registry
    /// lookup), and the composite's tier is the *least* of its two halves, not
    /// the name that was asked for.
    ///
    /// Returns `Self`, not `Box<dyn Completer>`: each caller boxes it where it
    /// already did, and the concrete type keeps `self.provider` readable *from
    /// inside this module*, which is what lets
    /// `the_completer_and_the_capability_come_from_the_same_provider` assert the
    /// completer and the tier came from the same `Arc` rather than merely from
    /// two calls that agreed.
    ///
    /// Issue #56 DR-26 / Task 50: the AFFILIATION comes off the same `Arc` in
    /// the same expression, for the reason the tier does. Two reads of one
    /// provider is how a chat ends up gated on one model's tier and another's
    /// institution.
    pub fn paired(
        provider: Arc<dyn Provider>,
    ) -> (
        Self,
        crate::privacy::ProviderTier,
        Option<crate::privacy::affiliation::ModelAffiliation>,
    ) {
        let tier = provider.tier();
        let affiliation = provider.affiliation();
        (Self::new(provider), tier, affiliation)
    }

    /// The batch form: a **factory** that mints one completer per call, beside
    /// the tier and affiliation of the single provider behind all of them.
    ///
    /// `IngestArgs::completer` is consumed by the macro, so ingesting N sources
    /// needs N completers — and every one of them must be the provider whose
    /// tier and affiliation were reported, or a batch could ratchet a base on
    /// one model's identity while running on another's. Minting them from one
    /// captured `Arc` in one expression is what makes that structural rather
    /// than conventional, exactly as [`Self::paired`] does for the single case.
    /// `session_id` is likewise captured once and cloned into every completer:
    /// a batch started from a chat must route an inspector escalation back to
    /// that chat, while `None` keeps chatless Knowledge-view runs fail-closed.
    /// The same owner token is cloned into every completer so cancelling a batch
    /// reaches whichever source is currently driving a bridged provider.
    pub fn paired_factory(
        provider: Arc<dyn Provider>,
        session_id: Option<String>,
        cancel: Option<CancellationToken>,
    ) -> (
        CompleterFactory,
        crate::privacy::ProviderTier,
        Option<crate::privacy::affiliation::ModelAffiliation>,
    ) {
        let tier = provider.tier();
        let affiliation = provider.affiliation();
        let session_id = session_id.filter(|id| !id.is_empty());
        let factory: CompleterFactory = Box::new(move || {
            let completer = Self::new(Arc::clone(&provider));
            let completer = match &session_id {
                Some(session_id) => completer.in_session(session_id.clone()),
                None => completer,
            };
            let completer = match &cancel {
                Some(cancel) => completer.cancelled_by(cancel.clone()),
                None => completer,
            };
            Box::new(completer) as Box<dyn Completer>
        });
        (factory, tier, affiliation)
    }
}

#[async_trait]
impl Completer for ProviderCompleter {
    async fn complete(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[Tool],
    ) -> Result<LlmReply> {
        // 1. Translate LlmMessage list to biorouter::conversation::message::Message list.
        let provider_messages: Vec<Message> =
            messages.iter().map(llm_to_provider_message).collect();

        // 2. The Tool type in loop_ is already rmcp::model::Tool — clone through unchanged.
        //    (The Completer trait re-uses rmcp::model::Tool directly.)
        let provider_tools: Vec<Tool> = tools.to_vec();

        // 3. Call the real provider.
        let (reply_msg, usage) = self
            .provider
            .complete(system_prompt, &provider_messages, &provider_tools)
            .await
            .map_err(|e| anyhow::anyhow!("provider.complete failed: {e}"))?;
        // Each macro iteration is billed. A run with no chat (the Knowledge
        // view's ingest panel) has nobody to charge and is not recorded.
        if let Some(session_id) = self.session_id.as_deref() {
            crate::session::session_manager::side_usage::record_side_call_usage(
                Some(session_id),
                crate::session::session_manager::side_usage::SideCall::KnowledgeMacro,
                self.provider.get_name(),
                &usage,
            )
            .await;
        }

        // 4. Extract assistant text and tool calls from the returned Message.
        let text = reply_msg.as_concat_text();
        let tool_calls = extract_tool_calls(&reply_msg)?;

        Ok(LlmReply { text, tool_calls })
    }

    /// #109: the seam that makes a coding-agent provider usable by a macro.
    ///
    /// `claude_code` and `codex` do not receive tools in the request; their
    /// child process reaches Biorouter's over an MCP bridge that the *provider
    /// call* establishes. So the run's dispatcher has to be reachable from
    /// inside the call, which is what this argument is for, and the child then
    /// chooses and executes the calls itself — behind the same inspectors,
    /// permission decision, approval round trip and cancellation a chat turn
    /// gets, because it is the same [`crate::providers::coding_agent::bridge`]
    /// gate stack.
    ///
    /// Everything else keeps the default: a provider that receives its tools in
    /// the request returns tool *requests* for the loop to dispatch, and
    /// [`ProviderToolTurnContext::run`] leaves that path untouched down to
    /// issuing no grant at all.
    async fn complete_with_dispatch(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[Tool],
        dispatch: Arc<dyn ToolDispatch>,
    ) -> Result<CompleterTurn> {
        if !self.provider.uses_tool_bridge() {
            return Ok(self.complete(system_prompt, messages, tools).await?.into());
        }

        let provider_messages: Vec<Message> =
            messages.iter().map(llm_to_provider_message).collect();

        // ONE read of the provider mutex and the master toggle, here, and
        // carried into the grant from there. A bridged call arrives from a child
        // process with no capability of its own to inherit, so fixing it before
        // the turn is the whole reason `CallCapability` exists — re-reading per
        // callback would be the two-reads race with a process boundary through
        // the middle.
        let shared: crate::agents::types::SharedProvider =
            Arc::new(tokio::sync::Mutex::new(Some(Arc::clone(&self.provider))));
        let capability = crate::privacy::CallCapability::sample(&shared).await;

        // An empty id means no chat owns the run. The bridge may still execute
        // calls its inspectors allow, but refuses an escalation before publishing
        // a card because no session can securely display and resolve it.
        let session = crate::session::session_manager::Session {
            id: self.session_id.clone().unwrap_or_default(),
            ..Default::default()
        };
        let context = ProviderToolTurnContext::for_workflow(
            session,
            Arc::new(SubAgentDispatchBridge { dispatch }),
            capability,
            self.cancel.clone(),
        );

        let turn = context
            .run(&self.provider, system_prompt, &provider_messages, tools)
            .await
            .map_err(|e| anyhow::anyhow!("provider.complete failed: {e}"))?;
        if let Some(session_id) = self.session_id.as_deref() {
            crate::session::session_manager::side_usage::record_side_call_usage(
                Some(session_id),
                crate::session::session_manager::side_usage::SideCall::KnowledgeMacro,
                self.provider.get_name(),
                &turn.usage,
            )
            .await;
        }

        // A mirrored request is a RECORD. Handing it back in `tool_calls` would
        // make the loop dispatch it a second time, and a `kb_write_page` run
        // twice is a second write, not a redraw.
        let executed = turn
            .executed
            .iter()
            .map(|call| ExecutedCall {
                id: call.id.clone(),
                name: call.name.clone(),
                args: call.arguments.clone(),
                output: call.output.clone(),
                is_error: call.is_error,
            })
            .collect();
        let tool_calls = turn
            .pending_tool_calls()
            .into_iter()
            .map(|request| match &request.tool_call {
                Ok(params) => Ok(LlmToolCall {
                    id: request.id.clone(),
                    name: params.name.to_string(),
                    args: params
                        .arguments
                        .clone()
                        .map(serde_json::Value::Object)
                        .unwrap_or(serde_json::Value::Null),
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "the model requested a tool call Biorouter could not decode: {}",
                    e.message
                )),
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(CompleterTurn {
            reply: LlmReply {
                text: turn.text(),
                tool_calls,
            },
            executed,
        })
    }
}

/// A sub-agent loop's dispatcher, as something the tool bridge can call.
///
/// The two traits are the same idea in two crates that cannot see each other:
/// `ToolDispatch` lives in `biorouter-mcp` (which must not depend on
/// `biorouter`), `BridgeToolDispatch` in `biorouter`. This adapter lives here,
/// the one place that already depends on both — the same reason
/// [`ProviderCompleter`] itself is here.
///
/// The session id and capability are dropped rather than forwarded: a
/// `ToolDispatch` is a closure over one run's state, so there is no second
/// session it could serve or capability it could be asked about. Cancellation
/// is enforced while the dispatch future is awaited so a bridged call cannot
/// outlive the request or turn that owns the macro.
struct SubAgentDispatchBridge {
    dispatch: Arc<dyn ToolDispatch>,
}

#[async_trait]
impl BridgeToolDispatch for SubAgentDispatchBridge {
    async fn dispatch(
        &self,
        _session_id: &str,
        call: CallToolRequestParams,
        _capability: crate::privacy::CallCapability,
        cancel: CancellationToken,
    ) -> std::result::Result<CallToolResult, String> {
        let name = call.name.to_string();
        let args = call
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("tool call cancelled".to_string()),
            result = self.dispatch.call(&name, args) => result,
        };
        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            // A tool that refused is a RESULT the child's model reads and acts
            // on, not a transport error it may retry — the same distinction the
            // loop's own dispatcher makes when it feeds `error: …` back as a
            // tool result. A JSON-RPC error here would read as a broken server.
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "error: {e}"
            ))])),
        }
    }
}

// ---------------------------------------------------------------------------
// Translation helpers
// ---------------------------------------------------------------------------

/// Convert one `LlmMessage` into the `biorouter::conversation::message::Message`
/// shape that `Provider::complete` expects.
///
/// Mapping:
/// - `LlmMessage::User(text)`  → `Message::user()` with a single text content item.
/// - `LlmMessage::Assistant(reply)` → `Message::assistant()` with one Text block for the
///   assistant text (if non-empty) plus one `ToolRequest` block per tool call.
/// - `LlmMessage::ToolResult { … }` → `Message::user()` with a single `ToolResponse` block.
///   Providers expect tool results on the *user* role (this matches the Anthropic / OpenAI
///   convention that biorouter's Anthropic/OpenAI provider adapters implement).
fn llm_to_provider_message(m: &LlmMessage) -> Message {
    match m {
        LlmMessage::User(text) => Message::user().with_text(text.clone()),

        LlmMessage::Assistant(reply) => {
            let mut msg = Message::assistant();
            if !reply.text.is_empty() {
                msg = msg.with_text(reply.text.clone());
            }
            for call in &reply.tool_calls {
                // Build a CallToolRequestParams from the LlmToolCall.
                let args_obj = match &call.args {
                    serde_json::Value::Object(obj) => Some(obj.clone()),
                    _ => None,
                };
                let params = CallToolRequestParams {
                    name: call.name.clone().into(),
                    arguments: args_obj,
                    task: None,
                    meta: None,
                };
                msg = msg.with_tool_request(call.id.clone(), Ok(params));
            }
            msg
        }

        LlmMessage::ToolResult {
            request_id,
            name: _,
            content,
        } => {
            // Wrap the result string as a text Content inside a CallToolResult.
            let call_result = CallToolResult::success(vec![Content::text(content.clone())]);
            Message::user().with_content(MessageContent::tool_response(
                request_id.clone(),
                Ok(call_result),
            ))
        }

        LlmMessage::ToolResults(parts) => {
            // Bundle ALL tool-result blocks into ONE user-role message.
            //
            // Bedrock (and the Anthropic spec) require that when an assistant turn
            // emits N `tool_use` blocks, ALL N `tool_result` blocks appear in a
            // single subsequent user message.  Emitting one message per result
            // causes a ValidationException ("Expected toolResult blocks at
            // messages.N.content for the following tool_use_id").
            let mut msg = Message::user();
            for ToolResultPart {
                request_id,
                name: _,
                content,
            } in parts
            {
                let call_result = CallToolResult::success(vec![Content::text(content.clone())]);
                msg = msg.with_tool_response(request_id.clone(), Ok(call_result));
            }
            msg
        }
    }
}

/// Walk a provider-returned `Message` and collect any `ToolRequest` content
/// blocks into `LlmToolCall` values.
///
/// A `ToolRequest` whose `tool_call` is `Err` is a call the model asked for and
/// the provider adapter could not decode — Google mints one for every
/// `functionCall` whose name breaks its character rule, and the streaming
/// decoders do the same for unparseable arguments. Skipping those quietly
/// emptied `tool_calls`, and an empty `tool_calls` is precisely the sub-agent
/// loop's signal that the agent has finished: the digest stopped early and was
/// reported as a success (issue #71). Refusing the whole completion is the only
/// answer that stays honest — the run failed, and the user is told why.
fn extract_tool_calls(msg: &Message) -> Result<Vec<LlmToolCall>> {
    let mut out = Vec::new();
    for c in &msg.content {
        let MessageContent::ToolRequest(req) = c else {
            continue;
        };
        match &req.tool_call {
            Ok(params) => {
                let args = params
                    .arguments
                    .as_ref()
                    .map(|obj| serde_json::Value::Object(obj.clone()))
                    .unwrap_or(serde_json::Value::Null);
                out.push(LlmToolCall {
                    id: req.id.clone(),
                    name: params.name.to_string(),
                    args,
                });
            }
            Err(e) => {
                anyhow::bail!(
                    "the model requested a tool call Biorouter could not decode: {}",
                    e.message
                );
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelConfig;
    use crate::providers::base::{ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use rmcp::model::{ErrorCode, ErrorData, Tool};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// A minimal mock Provider that returns a single canned Message.
    struct MockProvider {
        response: Message,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            unimplemented!("MockProvider::metadata not needed in this test")
        }

        fn get_name(&self) -> &str {
            "mock"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            let usage = ProviderUsage::new("mock".into(), Usage::new(None, None, None));
            Ok((self.response.clone(), usage))
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("claude-3-5-sonnet-20241022")
        }
    }

    /// A mock Provider that records every `messages` slice it receives so tests
    /// can inspect what was actually sent to the provider.
    struct RecordingMockProvider {
        response: Message,
        received: Mutex<Vec<Vec<Message>>>,
    }

    impl RecordingMockProvider {
        fn new(response: Message) -> Self {
            Self {
                response,
                received: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Provider for RecordingMockProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            unimplemented!()
        }

        fn get_name(&self) -> &str {
            "recording-mock"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.received.lock().await.push(messages.to_vec());
            let usage = ProviderUsage::new("recording-mock".into(), Usage::new(None, None, None));
            Ok((self.response.clone(), usage))
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("claude-3-5-sonnet-20241022")
        }
    }

    #[tokio::test]
    async fn roundtrips_text_and_tool_calls() {
        // Build a provider response: assistant text "ok" + one tool call kb_search(query="x")
        let params = CallToolRequestParams {
            name: "kb_search".into(),
            arguments: Some({
                let mut m = serde_json::Map::new();
                m.insert(
                    "query".to_string(),
                    serde_json::Value::String("x".to_string()),
                );
                m
            }),
            task: None,
            meta: None,
        };
        let response = Message::assistant()
            .with_text("ok")
            .with_tool_request("req-1", Ok(params));

        let provider = Arc::new(MockProvider { response });
        let completer = ProviderCompleter::new(provider);

        let reply = completer
            .complete("sys", &[LlmMessage::User("hi".into())], &[])
            .await
            .unwrap();

        assert_eq!(reply.text, "ok");
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].name, "kb_search");
        assert_eq!(reply.tool_calls[0].args["query"], "x");
    }

    /// Issue #71. A provider can hand back a `ToolRequest` it could not decode —
    /// Google emits one for every `functionCall` whose name fails its character
    /// rule, and the streaming decoders do the same for unparseable arguments.
    /// Dropping it left `tool_calls` empty, which the sub-agent loop reads as
    /// "the agent is finished": the digest ended early and reported success. A
    /// tool call the model asked for and Biorouter could not run is a failed
    /// completion, and must be reported as one.
    #[tokio::test]
    async fn an_undecodable_tool_request_fails_the_completion() {
        let broken = ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: std::borrow::Cow::from(
                "The provided function name 'kb.write page' had invalid characters",
            ),
            data: None,
        };
        let response = Message::assistant()
            .with_text("writing the page now")
            .with_tool_request("req-1", Err(broken));

        let provider = Arc::new(MockProvider { response });
        let completer = ProviderCompleter::new(provider);

        let err = completer
            .complete("sys", &[LlmMessage::User("hi".into())], &[])
            .await
            .expect_err("an undecodable tool request must not look like a finished turn")
            .to_string();

        assert!(
            err.contains("invalid characters"),
            "the provider's own explanation must reach the user, got: {err}"
        );
    }

    /// The mirror image: a well-formed tool request alongside a broken one must
    /// still fail rather than silently running only half of what the model asked
    /// for.
    #[tokio::test]
    async fn one_broken_tool_request_fails_even_beside_a_good_one() {
        let good = CallToolRequestParams {
            name: "kb_search".into(),
            arguments: None,
            task: None,
            meta: None,
        };
        let broken = ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: std::borrow::Cow::from("arguments were not valid JSON"),
            data: None,
        };
        let response = Message::assistant()
            .with_tool_request("ok-1", Ok(good))
            .with_tool_request("bad-1", Err(broken));

        let provider = Arc::new(MockProvider { response });
        let completer = ProviderCompleter::new(provider);

        let err = completer
            .complete("sys", &[LlmMessage::User("hi".into())], &[])
            .await
            .expect_err("a partially undecodable turn must not be executed as if intact")
            .to_string();

        assert!(
            err.contains("not valid JSON"),
            "the provider's own explanation must reach the user, got: {err}"
        );
    }

    #[tokio::test]
    async fn tool_result_message_is_translated() {
        // Feed a ToolResult LlmMessage; the provider receives it as a user Message
        // with a ToolResponse content block. The provider echoes back a text reply.
        let response = Message::assistant().with_text("done");
        let provider = Arc::new(MockProvider { response });
        let completer = ProviderCompleter::new(provider);

        let msgs = vec![
            LlmMessage::User("start".into()),
            LlmMessage::Assistant(LlmReply {
                text: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "r1".into(),
                    name: "kb_search".into(),
                    args: serde_json::json!({}),
                }],
            }),
            LlmMessage::ToolResult {
                request_id: "r1".into(),
                name: "kb_search".into(),
                content: "search results here".into(),
            },
        ];

        let reply = completer.complete("sys", &msgs, &[]).await.unwrap();
        assert_eq!(reply.text, "done");
        assert!(reply.tool_calls.is_empty());
    }

    /// Verify that the `tool_use_id` from the assistant's `tool_use` block is
    /// preserved on the `tool_result` user message that the provider receives.
    ///
    /// Bedrock (and Anthropic) require strict pairing: every `tool_use` in an
    /// assistant message must be matched by a `tool_result` with the same id in
    /// the immediately following user message.  If the id is lost or replaced the
    /// provider will reject the request with a `ValidationException`.
    #[tokio::test]
    async fn tool_result_round_trip_preserves_tool_use_id() {
        let tool_use_id = "abc123";

        // Recording mock: captures the messages it receives, replies with plain text.
        let response = Message::assistant().with_text("finished");
        let recording = Arc::new(RecordingMockProvider::new(response));
        let completer = ProviderCompleter::new(recording.clone());

        // Build a three-message conversation:
        //   [0] user prompt
        //   [1] assistant reply that issued one tool call with id=tool_use_id
        //   [2] tool result that references the same id
        let msgs = vec![
            LlmMessage::User("hello".into()),
            LlmMessage::Assistant(LlmReply {
                text: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: tool_use_id.into(),
                    name: "kb_read_page".into(),
                    args: serde_json::json!({ "path": "raw/src/source.md" }),
                }],
            }),
            LlmMessage::ToolResult {
                request_id: tool_use_id.into(),
                name: "kb_read_page".into(),
                content: "page content here".into(),
            },
        ];

        let _ = completer.complete("sys", &msgs, &[]).await.unwrap();

        // Inspect what the provider saw.
        let calls = recording.received.lock().await;
        assert_eq!(
            calls.len(),
            1,
            "provider should have been called exactly once"
        );
        let provider_msgs = &calls[0];

        // message[1] must be an assistant message containing a ToolRequest with the
        // expected id.
        let assistant_msg = &provider_msgs[1];
        let tool_req_id = assistant_msg
            .content
            .iter()
            .find_map(|c| {
                if let MessageContent::ToolRequest(req) = c {
                    Some(req.id.clone())
                } else {
                    None
                }
            })
            .expect("assistant message must contain a ToolRequest block");
        assert_eq!(
            tool_req_id, tool_use_id,
            "ToolRequest id must match the original tool_use_id"
        );

        // message[2] must be a user message containing a ToolResponse whose id
        // matches the tool_use_id — that's what Bedrock checks.
        let tool_result_msg = &provider_msgs[2];
        let tool_resp_id = tool_result_msg
            .content
            .iter()
            .find_map(|c| {
                if let MessageContent::ToolResponse(resp) = c {
                    Some(resp.id.clone())
                } else {
                    None
                }
            })
            .expect("tool-result user message must contain a ToolResponse block");
        assert_eq!(
            tool_resp_id, tool_use_id,
            "ToolResponse id must equal the tool_use_id so Bedrock can pair them"
        );
    }

    /// Bedrock requires that N tool_use blocks from one assistant turn are answered
    /// by exactly ONE user message containing N tool_result blocks.  Verify that
    /// `LlmMessage::ToolResults` (the compound variant) is collapsed into a single
    /// user-role `Message` carrying all ToolResponse content blocks.
    #[tokio::test]
    async fn multiple_tool_results_collapse_into_single_user_message() {
        let response = Message::assistant().with_text("all done");
        let recording = Arc::new(RecordingMockProvider::new(response));
        let completer = ProviderCompleter::new(recording.clone());

        // Build a conversation: user → assistant (2 tool calls) → ToolResults (2 parts).
        let msgs = vec![
            LlmMessage::User("hello".into()),
            LlmMessage::Assistant(LlmReply {
                text: String::new(),
                tool_calls: vec![
                    LlmToolCall {
                        id: "tc-1".into(),
                        name: "kb_search".into(),
                        args: serde_json::json!({}),
                    },
                    LlmToolCall {
                        id: "tc-2".into(),
                        name: "kb_read_page".into(),
                        args: serde_json::json!({}),
                    },
                ],
            }),
            LlmMessage::ToolResults(vec![
                ToolResultPart {
                    request_id: "tc-1".into(),
                    name: "kb_search".into(),
                    content: "result one".into(),
                },
                ToolResultPart {
                    request_id: "tc-2".into(),
                    name: "kb_read_page".into(),
                    content: "result two".into(),
                },
            ]),
        ];

        let _ = completer.complete("sys", &msgs, &[]).await.unwrap();

        let calls = recording.received.lock().await;
        assert_eq!(calls.len(), 1);
        let provider_msgs = &calls[0];

        // The provider must see exactly 3 messages (user, assistant, tool-results).
        assert_eq!(
            provider_msgs.len(),
            3,
            "expected 3 provider messages, got {}",
            provider_msgs.len()
        );

        // The third message must be user-role and contain exactly 2 ToolResponse blocks.
        let tool_result_msg = &provider_msgs[2];
        let tool_resp_ids: Vec<String> = tool_result_msg
            .content
            .iter()
            .filter_map(|c| {
                if let MessageContent::ToolResponse(resp) = c {
                    Some(resp.id.clone())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            tool_resp_ids.len(),
            2,
            "both tool results must be in a single user message; got {} ToolResponse block(s)",
            tool_resp_ids.len()
        );
        assert!(
            tool_resp_ids.contains(&"tc-1".to_string()),
            "ToolResponse for tc-1 missing"
        );
        assert!(
            tool_resp_ids.contains(&"tc-2".to_string()),
            "ToolResponse for tc-2 missing"
        );
    }

    // ── Issue #56, Task 10B ─────────────────────────────────────────────────

    /// A provider whose only interesting property is its tier.
    struct TieredProvider(crate::privacy::ProviderTier);

    #[async_trait]
    impl Provider for TieredProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            unimplemented!()
        }

        fn get_name(&self) -> &str {
            "tiered"
        }

        fn tier(&self) -> crate::privacy::ProviderTier {
            self.0
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            let usage = ProviderUsage::new("tiered".into(), Usage::new(None, None, None));
            Ok((Message::assistant().with_text("ok"), usage))
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("claude-3-5-sonnet-20241022")
        }
    }

    fn stub_provider_with_tier(tier: crate::privacy::ProviderTier) -> Arc<dyn Provider> {
        Arc::new(TieredProvider(tier))
    }

    #[tokio::test]
    async fn the_completer_and_the_capability_come_from_the_same_provider() {
        use crate::privacy::ProviderTier;

        // The unit under the CLI's, the routes' and the probe's rows. Both
        // directions, and the third assertion is the one that matters: `paired`
        // cannot be implemented as "wrap A, look up B" because there is only one
        // argument.
        let (_c, tier, _a) =
            ProviderCompleter::paired(stub_provider_with_tier(ProviderTier::Private));
        assert_eq!(tier, ProviderTier::Private);
        let (_c, tier, _a) =
            ProviderCompleter::paired(stub_provider_with_tier(ProviderTier::Public));
        assert_eq!(tier, ProviderTier::Public);

        // The completer really wraps the provider whose tier was reported —
        // `ProviderCompleter.provider` is a pub field, so this is a plain
        // pointer comparison and needs no downcast.
        let p = stub_provider_with_tier(ProviderTier::Private);
        let (c, tier, _a) = ProviderCompleter::paired(Arc::clone(&p));
        assert_eq!(tier, ProviderTier::Private);
        assert!(
            Arc::ptr_eq(&c.provider, &p),
            "the tier was read from a different Arc than the completer wraps"
        );
    }
}
