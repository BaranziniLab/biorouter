//! The mirror marker: how a coding-agent provider says *"this tool call already
//! ran — show it, do not run it"*.
//!
//! # Why a marker exists at all
//!
//! Every other provider hands the agent loop a `ToolRequest` as a *request*: the
//! loop inspects it, gates it, dispatches it, and appends the `ToolResponse` it
//! gets back. The two coding-agent providers are the exception. Their child
//! executed the call already — over the tool bridge, where Biorouter ran it
//! behind the same inspectors, permission mode, `.biorouterignore`, vault and
//! privacy Gate C ([`super::bridge`]) — and by the time the frame describing it
//! reaches this crate the work is done and the result is back in the child.
//!
//! Surfacing that as a bare `ToolRequest` would be a correctness bug, not a
//! cosmetic one: a `ToolRequest` in the turn's response message is dispatched by
//! the loop (`categorize_tools`, `agents/agent.rs`), and
//! `categorize_tool_requests` (`agents/reply_parts.rs`) filters on **content
//! only** — it never looks at metadata. So an unmarked mirrored request is either
//! a `Tool '…' not found` error row (with the `mcp__biorouter__` prefix intact)
//! or a genuine **second execution** of a call that already ran. A shell command
//! run twice is not a display glitch.
//!
//! # Where the marker lives, and where it deliberately does not
//!
//! It is a reserved key in the existing per-tool [`ProviderMetadata`] on
//! [`ToolRequest`] / [`ToolResponse`] — a `serde_json::Map` those types already
//! carry and already serialize. Nothing new enters the message schema, and the
//! GUI keeps rendering the pair with the components it uses for every other
//! provider.
//!
//! It is **not** a [`crate::conversation::MessageProvenance`] variant. That type
//! is BR-71's security-purposed cross-session stamp, whose presence has a
//! specific meaning to `is_provenance_boundary` in merges and to the subagent
//! surfaces, and whose unknown `kind` deliberately degrades to `None`. Reusing
//! it would overload a security signal with a display one and would silently
//! lose the stamp on an older reader.
//!
//! # The fail-safe direction
//!
//! Losing the marker must never cause a double execution, so the loop's question
//! is [`contains_provider_executed`] — *does this message carry **any** mirrored
//! tool content?* — and not *are they all mirrored?* A message that somehow mixed
//! marked and unmarked content therefore dispatches **nothing**: the worst case
//! is a card whose tool did not run (visible, and a decoder bug), never a shell
//! command that ran twice (invisible, and unrecoverable). The decoders never mix
//! — [`super::claude_stream`] and [`super::codex_stream`] mint mirrored content
//! only — and the tests below pin the fail-safe direction anyway.
//!
//! The marker is honoured on the **live stream path only**. Persisted rows are
//! replayed as history and are never re-dispatched, so a reader that does not
//! understand the key loses attribution, never safety.

// ⚠ `ProviderMetadata` here is the per-tool `serde_json::Map` on a
// `ToolRequest`/`ToolResponse` (`conversation::message`), **not** the identically
// named provider-configuration type in `providers::base`. Importing the wrong one
// compiles nowhere near here and reads as if it should.
use crate::conversation::message::{
    Message, MessageContent, ProviderMetadata, ToolRequest, ToolResponse,
};
use crate::providers::formats::audience;

/// The reserved `ProviderMetadata` key. Namespaced, because the map is shared
/// with whatever a provider chooses to record there.
pub const PROVIDER_EXECUTED_KEY: &str = "biorouterProviderExecuted";

/// Who actually ran a mirrored tool call — and therefore which guarantees hold.
///
/// This is a real distinction, not a label: [`Self::Bridged`] passed every gate
/// Biorouter has, [`Self::Child`] passed none of them. The GUI is expected to
/// say so on the card, because a user reading a command card is entitled to know
/// whether Biorouter approved it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Execution {
    /// Ran on Biorouter's side of the tool bridge: inspectors, permission mode,
    /// `.biorouterignore`, vault and privacy Gate C all applied.
    Bridged,
    /// Ran inside the child agent's own sandbox and never reached Biorouter —
    /// Codex's `exec`/`apply_patch` under its read-only sandbox, and any MCP
    /// server the user configured in their own `~/.codex/config.toml`. Displayed
    /// for honesty; explicitly *not* something Biorouter vouched for.
    Child,
}

impl Execution {
    /// The wire value. Stable — it is persisted in session rows.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Execution::Bridged => "bridged",
            Execution::Child => "child",
        }
    }

    /// Parse the wire value. An unrecognised value is `None`, which the loop
    /// treats as "not mirrored" — the fail-safe direction is to dispatch nothing
    /// only when we positively recognise the marker.
    ///
    /// Named `from_wire` rather than `from_str` so it is not mistaken for
    /// `std::str::FromStr`, whose contract this deliberately does not follow: an
    /// unknown value here is a `None` the caller treats as "not mirrored", not a
    /// parse error.
    #[must_use]
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "bridged" => Some(Execution::Bridged),
            "child" => Some(Execution::Child),
            _ => None,
        }
    }
}

fn stamp(metadata: &mut Option<ProviderMetadata>, exec: Execution) {
    metadata.get_or_insert_with(serde_json::Map::new).insert(
        PROVIDER_EXECUTED_KEY.to_string(),
        serde_json::Value::String(exec.as_str().to_string()),
    );
}

fn read(metadata: Option<&ProviderMetadata>) -> Option<Execution> {
    metadata
        .and_then(|m| m.get(PROVIDER_EXECUTED_KEY))
        .and_then(serde_json::Value::as_str)
        .and_then(Execution::from_wire)
}

/// Stamp a request the child already made.
pub fn mark_request(request: &mut ToolRequest, exec: Execution) {
    stamp(&mut request.metadata, exec);
}

/// Stamp the response that closes a mirrored request.
///
/// The response is stamped too, so a row read on its own — by the transcript
/// flattener, by a session export, by the GUI — carries its own provenance
/// rather than depending on finding its partner.
pub fn mark_response(response: &mut ToolResponse, exec: Execution) {
    stamp(&mut response.metadata, exec);
}

/// The execution kind recorded on a request, if it is mirrored.
#[must_use]
pub fn request_execution(request: &ToolRequest) -> Option<Execution> {
    read(request.metadata.as_ref())
}

/// The execution kind recorded on a response, if it is mirrored.
#[must_use]
pub fn response_execution(response: &ToolResponse) -> Option<Execution> {
    read(response.metadata.as_ref())
}

/// Does this message carry **any** mirrored tool content?
///
/// This is the question the agent loop asks, and the `any` rather than `all` is
/// the fail-safe choice explained in the module header: a mixed message
/// dispatches nothing.
#[must_use]
pub fn contains_provider_executed(message: &Message) -> bool {
    message.content.iter().any(|content| match content {
        MessageContent::ToolRequest(r) => request_execution(r).is_some(),
        MessageContent::ToolResponse(r) => response_execution(r).is_some(),
        _ => false,
    })
}

/// The execution kind for a message, when every mirrored item agrees.
///
/// Used for display and for tests; the loop's gate is
/// [`contains_provider_executed`]. Returns `None` for a message with no mirrored
/// content, and for one whose items disagree (which a decoder must never
/// produce).
#[must_use]
pub fn message_execution(message: &Message) -> Option<Execution> {
    let mut seen: Option<Execution> = None;
    for content in &message.content {
        let found = match content {
            MessageContent::ToolRequest(r) => request_execution(r),
            MessageContent::ToolResponse(r) => response_execution(r),
            _ => None,
        };
        match (seen, found) {
            (_, None) => {}
            (None, Some(e)) => seen = Some(e),
            (Some(a), Some(b)) if a == b => {}
            (Some(_), Some(_)) => return None,
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
    use rmcp::object;

    fn request(id: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                name: "developer__shell".into(),
                arguments: Some(object!({ "command": "ls" })),
                meta: None,
                task: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    fn response(id: &str) -> ToolResponse {
        ToolResponse {
            id: id.to_string(),
            tool_result: Ok(CallToolResult::success(vec![Content::text("ok")])),
            metadata: None,
        }
    }

    fn message_with(content: Vec<MessageContent>) -> Message {
        Message::new(
            rmcp::model::Role::Assistant,
            chrono::Utc::now().timestamp(),
            content,
        )
    }

    #[test]
    fn a_marked_request_round_trips_through_metadata() {
        let mut r = request("toolu_1");
        assert_eq!(request_execution(&r), None, "unmarked to begin with");
        mark_request(&mut r, Execution::Bridged);
        assert_eq!(request_execution(&r), Some(Execution::Bridged));

        let mut r2 = request("toolu_2");
        mark_request(&mut r2, Execution::Child);
        assert_eq!(request_execution(&r2), Some(Execution::Child));
    }

    #[test]
    fn a_marked_response_round_trips_through_metadata() {
        let mut r = response("toolu_1");
        assert_eq!(response_execution(&r), None);
        mark_response(&mut r, Execution::Bridged);
        assert_eq!(response_execution(&r), Some(Execution::Bridged));
    }

    /// The marker must survive the session store, which round-trips messages
    /// through serde. A marker that only exists in memory would be honoured on
    /// the live stream and lost on reload, which is exactly the drift the
    /// metadata home was chosen to avoid.
    #[test]
    fn the_marker_survives_a_serde_round_trip() {
        let mut r = request("toolu_1");
        mark_request(&mut r, Execution::Child);
        let msg = message_with(vec![MessageContent::ToolRequest(r)]);

        let json = serde_json::to_string(&msg).expect("serialize");
        let back: Message = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(message_execution(&back), Some(Execution::Child));
        assert!(contains_provider_executed(&back));
    }

    /// Stamping must not discard whatever the provider already recorded there.
    #[test]
    fn stamping_preserves_other_provider_metadata() {
        let mut r = request("toolu_1");
        let mut existing = serde_json::Map::new();
        existing.insert("vendorId".to_string(), serde_json::json!("abc"));
        r.metadata = Some(existing);

        mark_request(&mut r, Execution::Bridged);

        let meta = r.metadata.as_ref().expect("metadata");
        assert_eq!(meta.get("vendorId").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(request_execution(&r), Some(Execution::Bridged));
    }

    /// An ordinary API-provider message must never look mirrored — this is the
    /// assertion that keeps the loop's new branch off every other provider's
    /// path.
    #[test]
    fn an_unmarked_message_is_not_provider_executed() {
        let msg = message_with(vec![MessageContent::ToolRequest(request("toolu_1"))]);
        assert!(!contains_provider_executed(&msg));
        assert_eq!(message_execution(&msg), None);
    }

    /// A value nobody wrote (a newer marker kind, a corrupted row) is not
    /// recognised, so the message dispatches normally rather than being silently
    /// swallowed.
    #[test]
    fn an_unrecognised_marker_value_is_not_mirrored() {
        let mut r = request("toolu_1");
        let mut meta = serde_json::Map::new();
        meta.insert(
            PROVIDER_EXECUTED_KEY.to_string(),
            serde_json::json!("teleported"),
        );
        r.metadata = Some(meta);

        assert_eq!(request_execution(&r), None);
        assert!(!contains_provider_executed(&message_with(vec![
            MessageContent::ToolRequest(r)
        ])));
    }

    /// The fail-safe direction, stated as a test: a message mixing a mirrored
    /// call with an unmirrored one still answers "yes" to the loop's question,
    /// so **nothing** in it is dispatched. Not running a tool is recoverable;
    /// running one twice is not.
    #[test]
    fn a_mixed_message_still_suppresses_dispatch() {
        let mut marked = request("toolu_1");
        mark_request(&mut marked, Execution::Bridged);
        let msg = message_with(vec![
            MessageContent::ToolRequest(marked),
            MessageContent::ToolRequest(request("toolu_2")),
        ]);

        assert!(
            contains_provider_executed(&msg),
            "any mirrored content suppresses dispatch for the whole message"
        );
        assert_eq!(
            message_execution(&msg),
            Some(Execution::Bridged),
            "one marked item, so the kinds do not disagree"
        );
    }

    /// Disagreeing kinds have no single answer for display, and saying so is
    /// better than picking one.
    #[test]
    fn disagreeing_kinds_have_no_message_execution() {
        let mut a = request("toolu_1");
        mark_request(&mut a, Execution::Bridged);
        let mut b = request("toolu_2");
        mark_request(&mut b, Execution::Child);
        let msg = message_with(vec![
            MessageContent::ToolRequest(a),
            MessageContent::ToolRequest(b),
        ]);

        assert!(contains_provider_executed(&msg));
        assert_eq!(message_execution(&msg), None);
    }

    #[test]
    fn text_only_messages_are_never_mirrored() {
        let msg = message_with(vec![MessageContent::text("hello")]);
        assert!(!contains_provider_executed(&msg));
        assert_eq!(message_execution(&msg), None);
    }

    #[test]
    fn the_wire_values_are_stable() {
        assert_eq!(Execution::Bridged.as_str(), "bridged");
        assert_eq!(Execution::Child.as_str(), "child");
        assert_eq!(Execution::from_wire("bridged"), Some(Execution::Bridged));
        assert_eq!(Execution::from_wire("child"), Some(Execution::Child));
        assert_eq!(Execution::from_wire("Bridged"), None);
    }
}

/// The name a bridged tool has inside the child.
///
/// The MCP server Biorouter serves over the bridge is called `biorouter`
/// (`claude_code::bridge_mcp_config`, `codex::thread_params`), and both vendors
/// namespace an MCP tool as `mcp__<server>__<tool>`. The card must show the name
/// the user knows — `developer__shell`, not `mcp__biorouter__developer__shell` —
/// and it is also the name every other provider's card shows for the same tool.
const BRIDGE_TOOL_PREFIX: &str = "mcp__biorouter__";

/// Strip the child-side MCP namespacing from a bridged tool name.
///
/// A name without the prefix is returned unchanged: that is a tool the child ran
/// itself (Codex's `exec`), and inventing a prefix for it would misattribute it.
#[must_use]
pub fn display_tool_name(name: &str) -> &str {
    name.strip_prefix(BRIDGE_TOOL_PREFIX).unwrap_or(name)
}

/// Build the assistant message that records a call the child already made.
///
/// The message id is left unset: the loop names it, and the provider's own
/// streaming message id belongs to the *text* rows, where re-using it is what
/// merges chunks into one row. Sharing it here would fold the tool card into the
/// prose row.
#[must_use]
pub fn request_message(
    call_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    exec: Execution,
) -> Message {
    let arguments = match arguments {
        serde_json::Value::Object(map) => Some(map),
        // A tool called with no arguments, or with a non-object body the vendor
        // let through. Neither is worth dropping the card for.
        _ => None,
    };
    let mut request = ToolRequest {
        id: call_id.to_string(),
        tool_call: Ok(rmcp::model::CallToolRequestParams {
            name: display_tool_name(tool_name).to_string().into(),
            arguments,
            meta: None,
            task: None,
        }),
        metadata: None,
        tool_meta: None,
    };
    mark_request(&mut request, exec);

    Message::new(
        rmcp::model::Role::Assistant,
        chrono::Utc::now().timestamp(),
        vec![MessageContent::ToolRequest(request)],
    )
}

/// Build the user message carrying the result the child got back.
///
/// A failed call is recorded as a **successful transport carrying
/// `is_error: true`**, not as a transport error. That is the shape the GUI reads
/// (`ToolCallWithResponse::getToolResultError` checks `isError` on the value as
/// well as a transport-level `status: "error"`), and it is also the shape the
/// vendor used — Claude puts `is_error` on the `tool_result` block itself. It
/// keeps the failure text visible in the card body instead of collapsing it to
/// an error string.
#[must_use]
pub fn response_message(
    call_id: &str,
    content: Vec<rmcp::model::Content>,
    is_error: bool,
    exec: Execution,
) -> Message {
    let result = if is_error {
        rmcp::model::CallToolResult::error(content)
    } else {
        rmcp::model::CallToolResult::success(content)
    };
    response_message_with_result(call_id, result, exec)
}

#[must_use]
pub fn response_message_with_result(
    call_id: &str,
    result: rmcp::model::CallToolResult,
    exec: Execution,
) -> Message {
    let mut response = ToolResponse {
        id: call_id.to_string(),
        tool_result: Ok(result),
        metadata: None,
    };
    mark_response(&mut response, exec);

    Message::new(
        rmcp::model::Role::User,
        chrono::Utc::now().timestamp(),
        vec![MessageContent::ToolResponse(response)],
    )
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    #[test]
    fn a_bridged_tool_shows_the_name_the_user_knows() {
        assert_eq!(
            display_tool_name("mcp__biorouter__developer__shell"),
            "developer__shell"
        );
        // A child-executed tool has no bridge prefix and keeps its own name.
        assert_eq!(display_tool_name("exec"), "exec");
    }

    #[test]
    fn a_request_message_is_marked_and_carries_its_arguments() {
        let message = request_message(
            "toolu_1",
            "mcp__biorouter__developer__shell",
            serde_json::json!({ "command": "ls" }),
            Execution::Bridged,
        );

        assert!(contains_provider_executed(&message));
        let MessageContent::ToolRequest(request) = &message.content[0] else {
            panic!("expected a tool request");
        };
        let call = request.tool_call.as_ref().expect("a well-formed call");
        assert_eq!(call.name.as_ref(), "developer__shell");
        assert_eq!(
            call.arguments
                .as_ref()
                .and_then(|a| a.get("command"))
                .and_then(|v| v.as_str()),
            Some("ls")
        );
    }

    /// The pairing the GUI needs: same id on both halves, request on the
    /// assistant row, response on the user row — exactly as an API provider's
    /// dispatched pair looks.
    #[test]
    fn a_pair_shares_its_id_across_the_two_roles() {
        let request = request_message(
            "toolu_7",
            "mcp__biorouter__developer__shell",
            serde_json::json!({}),
            Execution::Bridged,
        );
        let response = response_message(
            "toolu_7",
            vec![rmcp::model::Content::text("ok")],
            false,
            Execution::Bridged,
        );

        assert_eq!(request.role, rmcp::model::Role::Assistant);
        assert_eq!(response.role, rmcp::model::Role::User);

        let MessageContent::ToolRequest(req) = &request.content[0] else {
            panic!("expected a tool request");
        };
        let MessageContent::ToolResponse(resp) = &response.content[0] else {
            panic!("expected a tool response");
        };
        assert_eq!(req.id, resp.id, "the card pairs on this id");
    }

    /// A failed call must reach the GUI as `isError` on the value, because that
    /// is what turns the card red while keeping the failure text readable.
    #[test]
    fn a_failed_call_is_recorded_as_is_error_not_as_a_transport_error() {
        let response = response_message(
            "toolu_2",
            vec![rmcp::model::Content::text("No such file or directory")],
            true,
            Execution::Bridged,
        );

        let MessageContent::ToolResponse(resp) = &response.content[0] else {
            panic!("expected a tool response");
        };
        let result = resp
            .tool_result
            .as_ref()
            .expect("the transport succeeded; the tool did not");
        assert_eq!(
            result.is_error,
            Some(true),
            "the GUI reads isError on the value to colour the card"
        );
        assert!(contains_provider_executed(&response));
    }
}

/// Turn a vendor's tool-result body into MCP content blocks.
///
/// Both vendors are loose about this field: Claude's `tool_result.content` is
/// either a plain string or an array of content blocks, and Codex's item results
/// vary by item type. Anything that is not recognisably a text block is
/// preserved as its JSON rather than dropped — a card showing an unexpected
/// shape is debuggable, a card showing nothing is not.
#[must_use]
pub fn content_from_value(value: &serde_json::Value) -> Vec<rmcp::model::Content> {
    match value {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::String(text) => vec![rmcp::model::Content::text(text.clone())],
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .map(
                |block| match block.get("text").and_then(serde_json::Value::as_str) {
                    Some(text) => rmcp::model::Content::text(text.to_string()),
                    None => rmcp::model::Content::text(block.to_string()),
                },
            )
            .collect(),
        other => vec![rmcp::model::Content::text(other.to_string())],
    }
}

/// The texts in a vendor's echo of a tool result, whatever its shape: a bare
/// string, an array of blocks, or a result object carrying `content`.
///
/// Text blocks contribute their text and embedded text resources theirs;
/// images and anything else contribute nothing, because neither side of the
/// comparison in [`recorded_if_received`] can be matched on them.
#[must_use]
pub fn echoed_texts(value: &serde_json::Value) -> Vec<String> {
    use serde_json::Value;
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block.get("text").and_then(Value::as_str),
                Some("resource") => block.pointer("/resource/text").and_then(Value::as_str),
                _ => None,
            })
            .map(str::to_string)
            .collect(),
        Value::Object(result) => result.get("content").map(echoed_texts).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Biorouter's own result for a bridged call, when the child demonstrably got it.
///
/// The bridge hands the child only [`super::bridge::child_view`] — the blocks a
/// model reads, with their annotations removed — and keeps the full result. The
/// vendor's echo of that view is lossy on top: Claude Code drops every
/// annotation and rewrites an embedded resource as
/// `[Resource from biorouter at <uri>] <text>`. So the transcript stores the full
/// result instead, and the card counts its user-facing block and the next
/// turn's prompt its model-facing one, exactly as for any other provider's call
/// (QA-E F4).
///
/// Only when the echo shows the child received it, though. A child whose call
/// timed out (#110), or whose CLI truncated a large result, worked from
/// something else, and the transcript records what the child actually saw: the
/// error flag must agree, and every text Biorouter sent must appear in what the
/// child echoed.
#[must_use]
pub fn recorded_if_received(
    recorded: rmcp::model::CallToolResult,
    echoed_texts: &[String],
    echoed_is_error: bool,
) -> Option<rmcp::model::CallToolResult> {
    if recorded.is_error.unwrap_or(false) != echoed_is_error {
        return None;
    }
    let echoed = echoed_texts.join("\n");
    let received = super::bridge::child_view(&recorded)
        .content
        .iter()
        .filter_map(audience::flattened_text)
        .all(|sent| echoed.contains(sent.as_str()));
    received.then_some(recorded)
}

/// The result to store for a bridged call: Biorouter's own when the bridge
/// kept it and the child's echo shows the child got it, else `None` — the
/// caller then stores the echo, as it always did.
#[must_use]
pub fn stored_bridged_result(
    bridge_url: Option<&str>,
    child_call_id: &str,
    echoed: &serde_json::Value,
    echoed_is_error: bool,
) -> Option<rmcp::model::CallToolResult> {
    let recorded = super::bridge::take_recorded_result(bridge_url?, child_call_id)?;
    recorded_if_received(recorded, &echoed_texts(echoed), echoed_is_error)
}

#[cfg(test)]
mod recorded_result_tests {
    use super::*;
    use rmcp::model::{CallToolResult, Content, Role};
    use serde_json::json;

    const DATE: &str = "Thu Sep 11 01:00:00 PDT 2026";

    /// The developer shell's result shape (`rmcp_developer.rs`): the output for
    /// the assistant, and a copy for the user marked low priority.
    fn shell_shaped(output: &str) -> CallToolResult {
        CallToolResult::success(vec![
            Content::text(output).with_audience(vec![Role::Assistant]),
            Content::text(output)
                .with_audience(vec![Role::User])
                .with_priority(0.0),
        ])
    }

    /// Blocks the card shows: no audience, or one naming the user.
    fn user_visible(content: &[Content]) -> usize {
        content
            .iter()
            .filter(|c| c.audience().is_none_or(|a| a.contains(&Role::User)))
            .count()
    }

    fn stored_result(message: &Message) -> &CallToolResult {
        let MessageContent::ToolResponse(response) = &message.content[0] else {
            panic!("expected a tool response");
        };
        response
            .tool_result
            .as_ref()
            .expect("a successful transport")
    }

    /// QA-E F4, the case the finding names: `developer__shell {"command":"date"}`
    /// over Claude Code stored two identical, unlabelled blocks — "2 results" on
    /// the card, and the output twice in the child's next prompt. The same
    /// two-block annotated result must mirror to ONE model-facing block (and one
    /// user-facing block, so the card reads "1 result"), audiences preserved.
    #[test]
    fn a_two_block_annotated_result_mirrors_to_one_model_facing_block_with_audience_preserved() {
        let recorded = shell_shaped(DATE);
        // Claude Code's echo in the QA run: both blocks, annotations gone.
        let echo = json!([{"type": "text", "text": DATE}, {"type": "text", "text": DATE}]);

        let stored = recorded_if_received(recorded.clone(), &echoed_texts(&echo), false)
            .expect("the child demonstrably received the result");
        let message = response_message_with_result("toolu_1", stored, Execution::Bridged);
        let content = &stored_result(&message).content;

        let model_facing: Vec<&Content> = content
            .iter()
            .filter(|c| audience::is_for_model(c))
            .collect();
        assert_eq!(
            model_facing.len(),
            1,
            "one block for the model: {content:?}"
        );
        assert_eq!(model_facing[0].audience(), Some(&vec![Role::Assistant]));
        assert_eq!(user_visible(content), 1, "the card must read \"1 result\"");
        assert_eq!(
            stored_result(&message),
            &recorded,
            "stored exactly as the tool returned it"
        );
    }

    /// After the fix the child is handed only the model's block, and echoes one.
    #[test]
    fn the_echo_of_the_model_view_adopts_the_full_result() {
        let echo = json!([{"type": "text", "text": DATE}]);
        let recorded = shell_shaped(DATE);
        assert_eq!(
            recorded_if_received(recorded.clone(), &echoed_texts(&echo), false),
            Some(recorded)
        );
    }

    /// A child whose call timed out (#110) never saw Biorouter's result, and
    /// the transcript records what it did see.
    #[test]
    fn a_child_that_timed_out_keeps_its_own_echo() {
        let echo = json!("The operation timed out");
        assert!(recorded_if_received(shell_shaped(DATE), &echoed_texts(&echo), true).is_none());
    }

    /// Same when the flags disagree the other way round, or the CLI truncated
    /// a large result before the child read it.
    #[test]
    fn an_echo_that_is_not_what_biorouter_sent_keeps_the_echo() {
        let echo = json!([{"type": "text", "text": DATE}]);
        assert!(
            recorded_if_received(shell_shaped(DATE), &echoed_texts(&echo), true).is_none(),
            "the child says its call failed"
        );

        let long = "x".repeat(1_000);
        let truncated = json!([{"type": "text", "text": "x".repeat(200)}]);
        assert!(
            recorded_if_received(shell_shaped(&long), &echoed_texts(&truncated), false).is_none(),
            "the child saw only part of the output"
        );
    }

    /// Claude Code rewrites an embedded text resource as prose with a prefix
    /// (measured, 2.1.266); the file's text is still what the child received.
    #[test]
    fn a_resource_echoed_as_prefixed_text_still_counts_as_received() {
        let recorded = CallToolResult::success(audience::text_editor_view_result());
        let echo = json!([{
            "type": "text",
            "text": format!("[Resource from biorouter at str:///notes.rs] {}", audience::VIEW_FOR_MODEL)
        }]);
        assert_eq!(
            recorded_if_received(recorded.clone(), &echoed_texts(&echo), false),
            Some(recorded)
        );
    }

    #[test]
    fn echoed_texts_reads_every_vendor_shape() {
        assert_eq!(echoed_texts(&json!("plain")), vec!["plain"]);
        assert_eq!(
            echoed_texts(&json!([
                {"type": "text", "text": "a"},
                {"type": "image", "source": {"type": "base64", "data": "AA=="}},
                {"type": "resource", "resource": {"uri": "str:///f", "text": "r"}}
            ])),
            vec!["a", "r"]
        );
        assert_eq!(
            echoed_texts(&json!({"content": [{"type": "text", "text": "c"}], "isError": false})),
            vec!["c"]
        );
        assert!(echoed_texts(&serde_json::Value::Null).is_empty());
    }

    /// No bridge, or nothing recorded for the call: the caller keeps the echo.
    #[test]
    fn with_nothing_recorded_the_echo_is_kept() {
        assert!(stored_bridged_result(None, "toolu_1", &json!("x"), false).is_none());
        assert!(stored_bridged_result(
            Some("http://127.0.0.1:1/tool_bridge/00000000000000000000000000000000"),
            "toolu_1",
            &json!("x"),
            false
        )
        .is_none());
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    fn texts(content: &[rmcp::model::Content]) -> Vec<String> {
        content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect()
    }

    #[test]
    fn a_plain_string_body_becomes_one_text_block() {
        let content = content_from_value(&serde_json::json!("a.txt\nb.txt"));
        assert_eq!(texts(&content), vec!["a.txt\nb.txt".to_string()]);
    }

    #[test]
    fn an_array_of_text_blocks_keeps_each_block() {
        let content = content_from_value(&serde_json::json!([
            { "type": "text", "text": "first" },
            { "type": "text", "text": "second" },
        ]));
        assert_eq!(
            texts(&content),
            vec!["first".to_string(), "second".to_string()]
        );
    }

    /// An unexpected block is preserved as JSON rather than silently dropped:
    /// the user can see what the tool actually returned.
    #[test]
    fn an_unrecognised_block_is_preserved_as_json() {
        let content = content_from_value(&serde_json::json!([{ "type": "image", "id": 7 }]));
        assert_eq!(content.len(), 1);
        assert!(texts(&content)[0].contains("image"));
    }

    #[test]
    fn a_null_body_produces_no_content_rather_than_the_word_null() {
        assert!(content_from_value(&serde_json::Value::Null).is_empty());
    }
}
