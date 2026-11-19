use crate::conversation::message::{Message, MessageContent, MessageMetadata};
use rmcp::model::{Role, TextContent};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::Arc;
use thiserror::Error;
use utoipa::ToSchema;

pub mod message;
pub mod normalize;
pub mod tool_preview;
pub mod tool_result_serde;

pub use normalize::{ConversationNormalizer, SharedNormalizer};

/// The message history of one session.
///
/// BR-56: the transcript is held behind an `Arc` and mutated copy-on-write
/// (`Arc::make_mut`), so `clone()` is a refcount bump rather than a deep copy of
/// every message. The agent loop clones the conversation several times per turn
/// (to hand it to `fix_conversation`, to MOIM injection, to the compaction
/// check, to the reply route's stream task); with a plain `Vec<Message>` each of
/// those copied every tool result in the session — the single largest per-turn
/// allocation in a long session. Only a writer (`push`/`extend`/`pop`/…) pays for
/// a copy, and only while another handle is alive.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation(Arc<Vec<Message>>);

pub(crate) fn without_bedrock_reasoning(conversation: &Conversation) -> Conversation {
    Conversation::new_unvalidated(
        conversation
            .iter()
            .cloned()
            .filter_map(|mut message| {
                if has_native_replay_content(&message) {
                    return Some(message);
                }
                let had_bedrock_reasoning = message.content.iter().any(|content| {
                    matches!(
                        content,
                        MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
                    )
                });
                message.content.retain(|content| {
                    let is_reasoning = matches!(
                        content,
                        MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
                    );
                    // A tool request the provider emitted but we could not parse was
                    // only ever coherent alongside the reasoning that authored it.
                    // Dropping the reasoning without it would leave an orphan the
                    // model cannot account for.
                    let is_orphaned_malformed_request = had_bedrock_reasoning
                        && matches!(content, MessageContent::ToolRequest(request) if request.tool_call.is_err());
                    !(is_reasoning || is_orphaned_malformed_request)
                });
                (!message.content.is_empty()).then_some(message)
            })
            .collect::<Vec<_>>(),
    )
}

impl<'schema> ToSchema<'schema> for Conversation {
    fn schema() -> (
        &'schema str,
        utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
    ) {
        (
            "Conversation",
            utoipa::openapi::schema::Array::new(utoipa::openapi::Ref::from_schema_name("Message"))
                .into(),
        )
    }
}

impl Serialize for Conversation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Conversation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<Message>::deserialize(deserializer).map(|messages| Self(Arc::new(messages)))
    }
}

#[derive(Error, Debug)]
#[error("invalid conversation: {reason}")]
pub struct InvalidConversation {
    reason: String,
    conversation: Conversation,
}

impl Conversation {
    pub fn new<I>(messages: I) -> Result<Self, InvalidConversation>
    where
        I: IntoIterator<Item = Message>,
    {
        Self::new_unvalidated(messages).validate()
    }

    pub fn new_unvalidated<I>(messages: I) -> Self
    where
        I: IntoIterator<Item = Message>,
    {
        Self(Arc::new(messages.into_iter().collect()))
    }

    pub fn empty() -> Self {
        Self::new_unvalidated([])
    }

    pub fn messages(&self) -> &Vec<Message> {
        &self.0
    }

    /// Take ownership of the messages, avoiding a copy when this is the only
    /// handle to the transcript (BR-56). A caller that still holds another clone
    /// pays for the copy-on-write here, exactly as `Arc::make_mut` would.
    pub fn into_messages(self) -> Vec<Message> {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| shared.as_ref().clone())
    }

    /// The copy-on-write handle used by every mutating method.
    fn messages_mut(&mut self) -> &mut Vec<Message> {
        Arc::make_mut(&mut self.0)
    }

    pub fn push(&mut self, message: Message) {
        let messages = self.messages_mut();
        if let Some(last) = messages
            .last_mut()
            .filter(|m| m.id.is_some() && m.id == message.id)
        {
            let merged = match (last.content.last_mut(), message.content.last()) {
                (Some(MessageContent::Text(ref mut last)), Some(MessageContent::Text(new)))
                    if message.content.len() == 1 =>
                {
                    merge_streamed_text(last, new)
                }
                _ => false,
            };
            if !merged {
                last.content.extend(message.content);
            }
        } else {
            messages.push(message);
        }
    }

    /// Append provider-authored blocks to an earlier message with the same id,
    /// even when tool-result messages have since been appended. Streaming
    /// Bedrock can expose one signed assistant response as reasoning followed
    /// by several tool blocks; the original assistant grouping is covered by
    /// the reasoning signature and must be reconstructed independently of the
    /// tool-execution batching setting.
    pub(crate) fn append_content_to_message_id(
        &mut self,
        id: &str,
        content: &[MessageContent],
    ) -> bool {
        let Some(message) = self
            .messages_mut()
            .iter_mut()
            .find(|message| message.id.as_deref() == Some(id))
        else {
            return false;
        };
        for next in content.iter().cloned() {
            let merged = match (message.content.last_mut(), &next) {
                (Some(MessageContent::Text(existing)), MessageContent::Text(next)) => {
                    merge_streamed_text(existing, next)
                }
                _ => false,
            };
            if !merged {
                message.content.push(next);
            }
        }
        true
    }

    pub fn last(&self) -> Option<&Message> {
        self.0.last()
    }

    pub fn first(&self) -> Option<&Message> {
        self.0.first()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Message>,
    {
        for message in iter {
            self.push(message);
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Message> {
        self.0.iter()
    }

    pub fn pop(&mut self) -> Option<Message> {
        self.messages_mut().pop()
    }

    pub fn truncate(&mut self, len: usize) {
        self.messages_mut().truncate(len);
    }

    pub fn clear(&mut self) {
        self.messages_mut().clear();
    }

    pub fn filtered_messages<F>(&self, filter: F) -> Vec<Message>
    where
        F: Fn(&MessageMetadata) -> bool,
    {
        self.0
            .iter()
            .filter(|msg| filter(&msg.metadata))
            .cloned()
            .collect()
    }

    pub fn agent_visible_messages(&self) -> Vec<Message> {
        self.filtered_messages(|meta| meta.agent_visible)
    }

    pub fn user_visible_messages(&self) -> Vec<Message> {
        self.filtered_messages(|meta| meta.user_visible)
    }

    fn validate(self) -> Result<Self, InvalidConversation> {
        let (_messages, issues) = fix_messages(self.0.as_ref().clone());
        if !issues.is_empty() {
            let reason = issues.join("\n");
            Err(InvalidConversation {
                reason,
                conversation: self,
            })
        } else {
            Ok(self)
        }
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::empty()
    }
}

impl IntoIterator for Conversation {
    type Item = Message;
    type IntoIter = std::vec::IntoIter<Message>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_messages().into_iter()
    }
}
impl<'a> IntoIterator for &'a Conversation {
    type Item = &'a Message;
    type IntoIter = std::slice::Iter<'a, Message>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// A message's place in the original conversation: either the *n*th agent-visible
/// message (normalization may rewrite it) or a non-visible message that passes
/// through untouched. Shared by [`fix_conversation`] and the incremental
/// normalizer (BR-56), which caches the shadow map of the frozen prefix.
#[derive(Debug, Clone)]
pub(crate) enum MessageSlot {
    /// Index into the *input* agent-visible message list.
    Visible(usize),
    /// Non-visible messages pass through unchanged.
    NonVisible(Message),
}

/// Split a message list into its shadow map and the agent-visible messages the
/// fix pipeline actually operates on. Moves rather than clones (BR-56).
pub(crate) fn split_slots(messages: Vec<Message>) -> (Vec<MessageSlot>, Vec<Message>) {
    let mut agent_visible = Vec::new();
    let mut slots = Vec::with_capacity(messages.len());
    for message in messages {
        if message.metadata.agent_visible {
            slots.push(MessageSlot::Visible(agent_visible.len()));
            agent_visible.push(message);
        } else {
            slots.push(MessageSlot::NonVisible(message));
        }
    }
    (slots, agent_visible)
}

/// Rebuild the full message list from the shadow map and the *fixed* visible
/// messages. A visible slot whose index no longer exists (the pipeline removed
/// or merged messages, so the fixed list is shorter) drops out.
pub(crate) fn reassemble(slots: Vec<MessageSlot>, fixed_visible: Vec<Message>) -> Vec<Message> {
    let mut fixed: Vec<Option<Message>> = fixed_visible.into_iter().map(Some).collect();
    slots
        .into_iter()
        .filter_map(|slot| match slot {
            // Slot indices are unique, so taking (rather than cloning) is
            // equivalent — and never copies a message body.
            MessageSlot::Visible(idx) => fixed.get_mut(idx).and_then(Option::take),
            MessageSlot::NonVisible(msg) => Some(msg),
        })
        .collect()
}

/// Fix a conversation that we're about to send to an LLM. So the last and first
/// messages should always be from the user.
pub fn fix_conversation(conversation: Conversation) -> (Conversation, Vec<String>) {
    let (slots, agent_visible_messages) = split_slots(conversation.into_messages());
    let (fixed_visible, issues) = fix_messages(agent_visible_messages);
    (
        Conversation::new_unvalidated(reassemble(slots, fixed_visible)),
        issues,
    )
}

fn fix_messages(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let (messages, mut issues) = fix_messages_core(messages);
    let (messages, edge_issues) = fix_messages_edges(messages);
    issues.extend(edge_issues);
    (messages, issues)
}

/// The body passes. Every one of them is *segment-decomposable* given a clean cut
/// (no tool request in the prefix answered in the suffix): running them over
/// `prefix ++ suffix` gives the same result as running them over each half and
/// re-running [`merge_consecutive_messages`] over the join. That is what lets the
/// incremental normalizer (BR-56) skip the prefix entirely.
fn fix_messages_core(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    run_passes(
        messages,
        [
            merge_text_content_items,
            trim_assistant_text_whitespace,
            remove_empty_messages,
            fix_tool_calling,
            merge_consecutive_messages,
        ],
    )
}

/// The end passes: they only look at the head and the tail of the whole list, so
/// they must run once, over the *complete* message list.
fn fix_messages_edges(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    run_passes(messages, [fix_lead_trail, populate_if_empty])
}

/// One normalization pass: message list in, fixed list + issues out.
type FixPass = fn(Vec<Message>) -> (Vec<Message>, Vec<String>);

fn run_passes<const N: usize>(
    messages: Vec<Message>,
    passes: [FixPass; N],
) -> (Vec<Message>, Vec<String>) {
    passes.into_iter().fold(
        (messages, Vec::new()),
        |(msgs, mut all_issues), processor| {
            let (new_msgs, issues) = processor(msgs);
            all_issues.extend(issues);
            (new_msgs, all_issues)
        },
    )
}

fn merge_text_content_in_message(mut msg: Message) -> Message {
    if msg.role != Role::Assistant || preserves_provider_parts(&msg) {
        return msg;
    }
    msg.content = msg
        .content
        .into_iter()
        .fold(Vec::new(), |mut content, item| {
            match item {
                MessageContent::Text(text) => {
                    if let Some(MessageContent::Text(ref mut last)) = content.last_mut() {
                        last.text.push_str(&text.text);
                    } else {
                        content.push(MessageContent::Text(text));
                    }
                }
                other => content.push(other),
            }
            content
        });
    msg
}

fn merge_text_content_items(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    messages.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut messages, mut issues), message| {
            let content_len = message.content.len();
            let message = merge_text_content_in_message(message);
            if content_len != message.content.len() {
                issues.push(String::from("Merged text content"))
            }
            messages.push(message);
            (messages, issues)
        },
    )
}

fn trim_assistant_text_whitespace(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let mut issues = Vec::new();

    let fixed_messages = messages
        .into_iter()
        .map(|mut message| {
            if message.role == Role::Assistant && !preserves_provider_parts(&message) {
                for content in &mut message.content {
                    if let MessageContent::Text(text) = content {
                        let trimmed = text.text.trim_end();
                        if trimmed.len() != text.text.len() {
                            issues.push(
                                "Trimmed trailing whitespace from assistant message".to_string(),
                            );
                            text.text = trimmed.to_string();
                        }
                    }
                }
            }
            message
        })
        .collect();

    (fixed_messages, issues)
}

fn remove_empty_messages(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let mut issues = Vec::new();
    let filtered_messages = messages
        .into_iter()
        .filter(|msg| {
            if !has_native_replay_content(msg)
                && msg
                    .content
                    .iter()
                    .all(|c| c.as_text().is_some_and(str::is_empty))
            {
                issues.push("Removed empty message".to_string());
                false
            } else {
                true
            }
        })
        .collect();
    (filtered_messages, issues)
}

fn fix_tool_calling(mut messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let mut issues = Vec::new();
    let mut pending_tool_requests: HashSet<String> = HashSet::new();

    for message in &mut messages {
        let preserve_parts = preserves_provider_parts(message);
        let mut content_to_remove = Vec::new();

        match message.role {
            Role::User => {
                for (idx, content) in message.content.iter().enumerate() {
                    match content {
                        MessageContent::ToolRequest(req) => {
                            content_to_remove.push(idx);
                            issues.push(format!(
                                "Removed tool request '{}' from user message",
                                req.id
                            ));
                        }
                        MessageContent::ToolConfirmationRequest(req) => {
                            content_to_remove.push(idx);
                            issues.push(format!(
                                "Removed tool confirmation request '{}' from user message",
                                req.id
                            ));
                        }
                        MessageContent::Thinking(_) | MessageContent::RedactedThinking(_) => {
                            content_to_remove.push(idx);
                            issues.push("Removed thinking content from user message".to_string());
                        }
                        MessageContent::ToolResponse(resp) => {
                            if pending_tool_requests.contains(&resp.id) {
                                pending_tool_requests.remove(&resp.id);
                            } else {
                                content_to_remove.push(idx);
                                issues
                                    .push(format!("Removed orphaned tool response '{}'", resp.id));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Role::Assistant => {
                for (idx, content) in message.content.iter().enumerate() {
                    match content {
                        MessageContent::ToolResponse(resp) => {
                            if !preserve_parts {
                                content_to_remove.push(idx);
                                issues.push(format!(
                                    "Removed tool response '{}' from assistant message",
                                    resp.id
                                ));
                            }
                        }
                        MessageContent::FrontendToolRequest(req) => {
                            if !preserve_parts {
                                content_to_remove.push(idx);
                                issues.push(format!(
                                    "Removed frontend tool request '{}' from assistant message",
                                    req.id
                                ));
                            }
                        }
                        MessageContent::ToolRequest(req) => {
                            pending_tool_requests.insert(req.id.clone());
                        }
                        _ => {}
                    }
                }
            }
        }

        for &idx in content_to_remove.iter().rev() {
            message.content.remove(idx);
        }
    }

    for message in &mut messages {
        if message.role == Role::Assistant && !preserves_provider_parts(message) {
            let mut content_to_remove = Vec::new();
            for (idx, content) in message.content.iter().enumerate() {
                if let MessageContent::ToolRequest(req) = content {
                    if pending_tool_requests.contains(&req.id) {
                        content_to_remove.push(idx);
                        issues.push(format!("Removed orphaned tool request '{}'", req.id));
                    }
                }
            }
            for &idx in content_to_remove.iter().rev() {
                message.content.remove(idx);
            }
        }
    }
    let (messages, empty_removed) = remove_empty_messages(messages);
    issues.extend(empty_removed);
    (messages, issues)
}

/// #51: whether this message's preservation marker makes it a hard boundary for
/// [`merge_consecutive_messages`].
///
/// A merge extends the FIRST message and keeps only its metadata and its durable
/// id, so merging across a marker either destroys it (an unpinned neighbour
/// swallows the pinned note — the note is then summarized away like anything
/// else) or broadens it (a pinned carrier absorbs what follows, so unrelated
/// content silently inherits the exemption and spends the pinned-set token
/// budget). Both are invisible to `context_mgmt`, which honours pins correctly
/// but only ever sees the already-merged transcript — and that same transcript is
/// what the overflow path writes back to the store. The boundary has to survive
/// here or it never reaches the code that respects it.
///
/// A marker is a boundary only where it could actually be honoured, which is
/// [`MessageContent::is_pin_eligible`]'s exhaustive ruling — not a second
/// exclusion list here. Duplicating it would default every future content
/// variant to "boundary" and let the two drift, which is exactly the failure that
/// put `FrontendToolRequest` on the preservable side to begin with. Delegating
/// also keeps the provider-shape passes untouched wherever no pin is at stake:
/// a marker on thinking content, on either half of a tool pair, or on a UI
/// handshake merges exactly as it did before.
fn is_pin_boundary(message: &Message) -> bool {
    message.metadata.pinned && message.content.iter().all(MessageContent::is_pin_eligible)
}

/// BR-71: whether two adjacent messages disagree about where they came from,
/// which makes the join between them a hard boundary for
/// [`merge_consecutive_messages`].
///
/// Same reasoning as [`is_pin_boundary`], same mechanism (a merge keeps only the
/// FIRST message's metadata) and the same path back to storage — but the rule is
/// a *change* of origin rather than the presence of a marker, because two
/// injections from the same session carry byte-identical metadata and lose
/// nothing by merging. Across a change, one of two things happens and both are
/// wrong: an unstamped neighbour swallows an injection and the stamp is gone, or
/// a stamped one absorbs what follows and the human's own words are recorded as
/// agent-injected. The second is the worse failure — a lost stamp
/// under-attributes, a broadened stamp MIS-attributes — which is why this is a
/// boundary in both directions rather than a carry-forward.
fn is_provenance_boundary(last: &Message, next: &Message) -> bool {
    last.metadata.provenance != next.metadata.provenance
}

/// Signed/redacted reasoning authenticates the provider-authored assistant
/// message in its original block shape. Any formatter-independent cleanup that
/// trims text, merges blocks/messages, or rewrites tool requests invalidates
/// that signature when the message is replayed to Bedrock.
pub(crate) fn has_signed_reasoning(message: &Message) -> bool {
    message.content.iter().any(|content| match content {
        MessageContent::Thinking(thinking) => !thinking.signature.is_empty(),
        MessageContent::RedactedThinking(_) => true,
        _ => false,
    })
}

fn has_google_native_part(metadata: Option<&Map<String, Value>>) -> bool {
    metadata.is_some_and(|metadata| {
        metadata
            .get("googleNativePart")
            .is_some_and(Value::is_object)
    })
}

fn has_mistral_content_capture(metadata: Option<&Map<String, Value>>) -> bool {
    metadata.is_some_and(|metadata| {
        metadata
            .get("mistral_content_provider")
            .and_then(Value::as_str)
            == Some("mistral")
            && metadata
                .get("mistral_content")
                .and_then(Value::as_array)
                .is_some_and(|chunks| !chunks.is_empty())
    })
}

pub(crate) fn has_google_native_replay_content(message: &Message) -> bool {
    message.role == Role::Assistant
        && message.content.iter().any(|content| match content {
            MessageContent::Text(text) => {
                has_google_native_part(text.meta.as_ref().map(|meta| &meta.0))
            }
            MessageContent::ToolRequest(request) => {
                has_google_native_part(request.metadata.as_ref())
            }
            _ => false,
        })
}

pub(crate) fn has_native_replay_content(message: &Message) -> bool {
    message.role == Role::Assistant
        && message.content.iter().any(|content| {
            let metadata = match content {
                MessageContent::Text(text) => text.meta.as_ref().map(|meta| &meta.0),
                MessageContent::ToolRequest(request) => request.metadata.as_ref(),
                _ => None,
            };
            has_google_native_part(metadata) || has_mistral_content_capture(metadata)
        })
}

fn preserves_provider_parts(message: &Message) -> bool {
    has_signed_reasoning(message) || has_native_replay_content(message)
}

fn merge_streamed_text(existing: &mut TextContent, next: &TextContent) -> bool {
    let existing_metadata = existing.meta.as_ref().map(|meta| &meta.0);
    let next_metadata = next.meta.as_ref().map(|meta| &meta.0);
    if has_google_native_part(existing_metadata) || has_google_native_part(next_metadata) {
        return false;
    }
    let existing_mistral = has_mistral_content_capture(existing_metadata);
    let next_mistral = has_mistral_content_capture(next_metadata);
    if existing_mistral != next_mistral {
        return false;
    }
    existing.text.push_str(&next.text);
    if next_mistral {
        // Mistral captures are cumulative; retaining each streamed copy would
        // make the persisted history grow quadratically with reply length.
        existing.meta = next.meta.clone();
    }
    true
}

pub fn merge_consecutive_messages(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let mut issues = Vec::new();
    let mut merged_messages: Vec<Message> = Vec::new();

    for message in messages {
        if let Some(last) = merged_messages.last_mut() {
            let effective = effective_role(&message);
            if effective_role(last) == effective
                && !is_pin_boundary(last)
                && !is_pin_boundary(&message)
                && !is_provenance_boundary(last, &message)
                && !preserves_provider_parts(last)
                && !preserves_provider_parts(&message)
            {
                last.content.extend(message.content);
                issues.push(format!("Merged consecutive {} messages", effective));
                continue;
            }
        }
        merged_messages.push(message);
    }

    (merged_messages, issues)
}

fn has_tool_response(message: &Message) -> bool {
    message
        .content
        .iter()
        .any(|content| matches!(content, MessageContent::ToolResponse(_)))
}

pub fn effective_role(message: &Message) -> String {
    if message.role == Role::User && has_tool_response(message) {
        "tool".to_string()
    } else {
        match message.role {
            Role::User => "user".to_string(),
            Role::Assistant => "assistant".to_string(),
        }
    }
}

fn fix_lead_trail(mut messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let mut issues = Vec::new();

    if let Some(first) = messages.first() {
        if first.role == Role::Assistant && !preserves_provider_parts(first) {
            messages.remove(0);
            issues.push("Removed leading assistant message".to_string());
        }
    }

    if let Some(last) = messages.last() {
        if last.role == Role::Assistant && !preserves_provider_parts(last) {
            messages.pop();
            issues.push("Removed trailing assistant message".to_string());
        }
    }

    (messages, issues)
}

const PLACEHOLDER_USER_MESSAGE: &str = "Hello";

fn populate_if_empty(mut messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
    let mut issues = Vec::new();

    if messages.is_empty() {
        issues.push("Added placeholder user message to empty conversation".to_string());
        messages.push(Message::user().with_text(PLACEHOLDER_USER_MESSAGE));
    }
    (messages, issues)
}

pub fn debug_conversation_fix(
    messages: &[Message],
    fixed: &[Message],
    issues: &[String],
) -> String {
    let mut output = String::new();

    output.push_str("=== CONVERSATION FIX DEBUG ===\n\n");

    output.push_str("BEFORE:\n");
    for (i, msg) in messages.iter().enumerate() {
        output.push_str(&format!("  [{}] {}\n", i, msg.debug()));
    }

    output.push_str("\nISSUES FOUND:\n");
    if issues.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for issue in issues {
            output.push_str(&format!("  - {}\n", issue));
        }
    }

    output.push_str("\nAFTER:\n");
    for (i, msg) in fixed.iter().enumerate() {
        output.push_str(&format!("  [{}] {}\n", i, msg.debug()));
    }

    output.push_str("\n==============================\n");
    output
}

#[cfg(test)]
mod tests {
    use crate::conversation::message::{
        Message, MessageContent, MessageProvenance, ProvenanceKind,
    };
    use crate::conversation::{debug_conversation_fix, fix_conversation, Conversation};
    use rmcp::model::{CallToolRequestParams, CallToolResult, Role};
    use rmcp::object;

    #[test]
    fn conversation_openapi_schema_is_a_message_array() {
        let (_, schema) = <Conversation as utoipa::ToSchema>::schema();
        let schema = serde_json::to_value(schema).unwrap();

        assert_eq!(schema["type"], "array");
        assert_eq!(schema["items"]["$ref"], "#/components/schemas/Message");
    }

    #[test]
    fn native_parts_append_to_earlier_stream_preserves_text_signatures() {
        use crate::model::ModelConfig;
        use crate::providers::formats::google::{create_request, response_to_message_for_model};
        use serde_json::json;

        let parts = [
            json!({"text": "First ", "thoughtSignature": "FIRST"}),
            json!({"text": "second ", "thoughtSignature": "SECOND"}),
        ];
        let messages: Vec<_> = parts
            .iter()
            .map(|part| {
                response_to_message_for_model(
                    json!({"candidates": [{"content": {"parts": [part]}}]}),
                    "gemini-4-hydrogen",
                )
                .unwrap()
                .with_id("native-stream".to_string())
            })
            .collect();
        let mut conversation = Conversation::new_unvalidated([
            messages[0].clone(),
            Message::user().with_text("intervening result"),
        ]);
        assert!(conversation.append_content_to_message_id("native-stream", &messages[1].content));
        let payload = create_request(
            &ModelConfig::new_or_fail("gemini-4-hydrogen"),
            "system",
            conversation.messages(),
            &[],
        )
        .unwrap();
        assert_eq!(payload["contents"][0]["parts"], json!(parts));
    }

    #[tokio::test]
    async fn native_parts_mistral_stream_keeps_the_latest_cumulative_capture() {
        use crate::model::ModelConfig;
        use crate::providers::formats::openai::{
            create_request, prepare_mistral_request, response_to_streaming_message,
            stamp_reasoning_provenance,
        };
        use crate::providers::utils::ImageFormat;
        use futures::StreamExt;
        use serde_json::json;

        let chunks = [
            vec![
                json!({"type": "thinking", "thinking": [{"type": "text", "text": "Plan"}]}),
                json!({"type": "text", "text": "First "}),
            ],
            vec![json!({"type": "text", "text": "second "})],
        ];
        let expected: Vec<_> = chunks.iter().flatten().cloned().collect();
        let lines: Vec<_> = chunks
            .into_iter()
            .map(|content| {
                let chunk = json!({
                    "id": "mistral-native-stream",
                    "choices": [{"index": 0, "delta": {"content": content}}]
                });
                Ok(format!("data: {chunk}"))
            })
            .collect();
        let stream = response_to_streaming_message(futures::stream::iter(lines));
        futures::pin_mut!(stream);
        let mut conversation = Conversation::empty();
        conversation.push(Message::user().with_text("inspect"));
        while let Some(item) = stream.next().await {
            if let Some(mut message) = item.unwrap().0 {
                stamp_reasoning_provenance(&mut message, "mistral");
                conversation.push(message);
            }
        }
        assert_eq!(conversation.messages()[1].content.len(), 1);
        conversation.push(Message::user().with_text("continue"));
        let model = ModelConfig::new_or_fail("mistral-large-4");
        let mut payload = create_request(
            &model,
            "system",
            conversation.messages(),
            &[],
            &ImageFormat::OpenAi,
            false,
        )
        .unwrap();
        prepare_mistral_request(&mut payload, &model, conversation.messages(), "mistral");
        let assistant = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["content"], json!(expected));
    }

    #[test]
    fn native_parts_mixed_provenance_keeps_boundaries_and_ordinary_text_coalesces() {
        use crate::providers::formats::openai::{response_to_message, stamp_reasoning_provenance};
        use serde_json::json;

        let captured = |provider: Option<&str>, text: &str| {
            let mut message = response_to_message(&json!({
                "choices": [{"message": {"content": [{"type": "text", "text": text}]}}]
            }))
            .unwrap()
            .with_id("same-stream".to_string());
            if let Some(provider) = provider {
                stamp_reasoning_provenance(&mut message, provider);
            }
            message
        };
        for (first, second, expected_parts) in [
            (None, None, 1),
            (Some("other"), Some("other"), 1),
            (Some("mistral"), Some("mistral"), 1),
            (Some("mistral"), Some("other"), 2),
            (Some("other"), Some("mistral"), 2),
            (Some("mistral"), None, 2),
            (None, Some("mistral"), 2),
        ] {
            let mut conversation = Conversation::empty();
            conversation.push(captured(first, "First "));
            conversation.push(captured(second, "second "));
            assert_eq!(
                conversation.messages()[0].content.len(),
                expected_parts,
                "capture provenance {first:?} then {second:?}"
            );
        }

        let mut ordinary = Conversation::empty();
        ordinary.push(Message::user().with_text("inspect"));
        ordinary.push(
            Message::assistant()
                .with_text("First ")
                .with_id("ordinary".to_string()),
        );
        ordinary.push(
            Message::assistant()
                .with_text("second ")
                .with_id("ordinary".to_string()),
        );
        ordinary.push(Message::user().with_text("continue"));
        ordinary.push(Message::assistant().with_text(""));
        ordinary.push(Message::user().with_text("last"));
        let (normalized, _) = fix_conversation(ordinary);
        assert_eq!(normalized.messages()[1].content.len(), 1);
        assert_eq!(normalized.messages()[1].as_concat_text(), "First second");
        assert_eq!(normalized.len(), 3);
    }

    macro_rules! assert_has_issues_unordered {
        ($fixed:expr, $issues:expr, $($expected:expr),+ $(,)?) => {
            {
                let mut expected: Vec<&str> = vec![$($expected),+];
                let mut actual: Vec<&str> = $issues.iter().map(|s| s.as_str()).collect();
                expected.sort();
                actual.sort();

                if actual != expected {
                    panic!(
                        "assertion failed: issues don't match\nexpected: {:?}\n  actual: {:?}. Fixed conversation is:\n{:#?}",
                        expected, $issues, $fixed,
                    );
                }
            }
        };
    }

    fn run_verify(messages: Vec<Message>) -> (Vec<Message>, Vec<String>) {
        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages.clone()));

        // Uncomment the following line to print the debug report
        // let report = debug_conversation_fix(&messages, &fixed, &issues);
        // print!("\n{}", report);

        let (_fixed, issues_with_fixed) = fix_conversation(fixed.clone());
        assert_eq!(
            issues_with_fixed.len(),
            0,
            "Fixed conversation should have no issues, but found: {:?}\n\n{}",
            issues_with_fixed,
            debug_conversation_fix(&messages, fixed.messages(), &issues)
        );
        (fixed.messages().clone(), issues)
    }

    #[test]
    fn test_valid_conversation() {
        let all_messages = [
            Message::user().with_text("Can you help me search for something?"),
            Message::assistant()
                .with_text("I'll help you search.")
                .with_tool_request(
                    "search_1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "web_search".into(),
                        arguments: Some(object!({"query": "rust programming"})),
                        meta: None,
                    }),
                ),
            Message::user().with_tool_response(
                "search_1",
                Ok(rmcp::model::CallToolResult {
                    content: vec![],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            Message::assistant().with_text("Based on the search results, here's what I found..."),
        ];

        for i in 1..=all_messages.len() {
            let messages = Conversation::new_unvalidated(all_messages[..i].to_vec());
            if messages.last().unwrap().role == Role::User {
                let (fixed, issues) = fix_conversation(messages.clone());
                assert_eq!(
                    fixed.len(),
                    messages.len(),
                    "Step {}: Length should match",
                    i
                );
                assert!(
                    issues.is_empty(),
                    "Step {}: Should have no issues, but found: {:?}",
                    i,
                    issues
                );
                assert_eq!(
                    fixed.messages(),
                    messages.messages(),
                    "Step {}: Messages should be unchanged",
                    i
                );
            }
        }
    }

    #[test]
    fn test_role_alternation_and_content_placement_issues() {
        let messages = vec![
            Message::user().with_text("Hello"),
            Message::user().with_text("Another user message"),
            Message::assistant()
                .with_text("Response")
                .with_tool_response(
                    "orphan_1",
                    Ok(rmcp::model::CallToolResult {
                        content: vec![],
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    }),
                ), // Wrong role
            Message::assistant().with_thinking("Let me think", ""),
            Message::user()
                .with_tool_request(
                    "bad_req",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "search".into(),
                        arguments: Some(object!({})),
                        meta: None,
                    }),
                )
                .with_text("User with bad tool request"),
        ];

        let (fixed, issues) = run_verify(messages);

        assert_eq!(fixed.len(), 3);

        assert_has_issues_unordered!(
            fixed,
            issues,
            "Merged consecutive assistant messages",
            "Merged consecutive user messages",
            "Removed tool response 'orphan_1' from assistant message",
            "Removed tool request 'bad_req' from user message",
        );

        assert_eq!(fixed[0].role, Role::User);
        assert_eq!(fixed[1].role, Role::Assistant);
        assert_eq!(fixed[2].role, Role::User);

        assert_eq!(fixed[0].content.len(), 2);
    }

    #[test]
    fn test_orphaned_tools_and_empty_messages() {
        // This conversation completely collapses. the first user message is invalid
        // then we remove the empty user message and the wrong tool response
        // then we collapse the assistant messages
        // which we then remove because you can't end a conversation with an assistant message
        let messages = vec![
            Message::assistant()
                .with_text("I'll search for you")
                .with_tool_request(
                    "search_1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "search".into(),
                        arguments: Some(object!({})),
                        meta: None,
                    }),
                ),
            Message::user(),
            Message::user().with_tool_response(
                "wrong_id",
                Ok(rmcp::model::CallToolResult {
                    content: vec![],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            Message::assistant().with_tool_request(
                "search_2",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "search".into(),
                    arguments: Some(object!({})),
                    meta: None,
                }),
            ),
        ];

        let (fixed, issues) = run_verify(messages);

        assert_eq!(fixed.len(), 1);

        assert_has_issues_unordered!(
            fixed,
            issues,
            "Removed empty message",
            "Removed orphaned tool response 'wrong_id'",
            "Removed orphaned tool request 'search_1'",
            "Removed orphaned tool request 'search_2'",
            "Removed empty message",
            "Removed empty message",
            "Removed leading assistant message",
            "Added placeholder user message to empty conversation",
        );

        assert_eq!(fixed[0].role, Role::User);
        assert_eq!(fixed[0].as_concat_text(), "Hello");
    }

    #[test]
    fn test_real_world_consecutive_assistant_messages() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("run ls in the current directory and then run a word count on the smallest file"),

            Message::assistant()
                .with_text("I'll help you run `ls` in the current directory and then perform a word count on the smallest file. Let me start by listing the directory contents.")
                .with_tool_request("toolu_bdrk_018adWbP4X26CfoJU5hkhu3i", Ok(CallToolRequestParams { task: None, name: "developer__shell".into(), arguments: Some(object!({"command": "ls -la"})), meta: None })),

            Message::assistant()
                .with_text("Now I'll identify the smallest file by size. Looking at the output, I can see that both `slack.yaml` and `subworkflows.yaml` have a size of 0 bytes, making them the smallest files. I'll run a word count on one of them:")
                .with_tool_request("toolu_bdrk_01KgDYHs4fAodi22NqxRzmwx", Ok(CallToolRequestParams { task: None, name: "developer__shell".into(), arguments: Some(object!({"command": "wc slack.yaml"})), meta: None })),

            Message::user()
                .with_tool_response("toolu_bdrk_01KgDYHs4fAodi22NqxRzmwx", Ok(rmcp::model::CallToolResult {
                    content: vec![],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                })),

            Message::assistant()
                .with_text("I ran `ls -la` in the current directory and found several files. Looking at the file sizes, I can see that both `slack.yaml` and `subworkflows.yaml` are 0 bytes (the smallest files). I ran a word count on `slack.yaml` which shows: **0 lines**, **0 words**, **0 characters**"),
            Message::user().with_text("thanks!"),
        ]);

        let (fixed, issues) = fix_conversation(conversation);

        assert_eq!(fixed.len(), 5);
        assert_has_issues_unordered!(
            fixed,
            issues,
            "Removed orphaned tool request 'toolu_bdrk_018adWbP4X26CfoJU5hkhu3i'",
            "Merged consecutive assistant messages"
        )
    }

    #[test]
    fn test_tool_response_effective_role() {
        let messages = vec![
            Message::user().with_text("Search for something"),
            Message::assistant()
                .with_text("I'll search for you")
                .with_tool_request(
                    "search_1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "search".into(),
                        arguments: Some(object!({})),
                        meta: None,
                    }),
                ),
            Message::user().with_tool_response(
                "search_1",
                Ok(rmcp::model::CallToolResult {
                    content: vec![],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            Message::user().with_text("Thanks!"),
        ];

        let (_fixed, issues) = run_verify(messages);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_merge_text_content_items() {
        use crate::conversation::message::MessageContent;
        use rmcp::model::{AnnotateAble, RawTextContent};

        let mut message = Message::assistant().with_text("Hello");

        message.content.push(MessageContent::Text(
            RawTextContent {
                text: " world".to_string(),
                meta: None,
            }
            .no_annotation(),
        ));
        message.content.push(MessageContent::Text(
            RawTextContent {
                text: "!".to_string(),
                meta: None,
            }
            .no_annotation(),
        ));

        let messages = vec![
            Message::user().with_text("hello"),
            message,
            Message::user().with_text("thanks"),
        ];

        let (fixed, issues) = run_verify(messages);

        assert_eq!(fixed.len(), 3);
        assert_has_issues_unordered!(fixed, issues, "Merged text content");

        let fixed_msg = &fixed[1];
        assert_eq!(fixed_msg.content.len(), 1);

        if let MessageContent::Text(text_content) = &fixed_msg.content[0] {
            assert_eq!(text_content.text, "Hello world!");
        } else {
            panic!("Expected text content");
        }
    }

    #[test]
    fn test_merge_text_content_items_with_mixed_content() {
        use crate::conversation::message::MessageContent;
        use rmcp::model::{AnnotateAble, RawTextContent};

        let mut image_message = Message::assistant().with_text("Look at");

        image_message.content.push(MessageContent::Text(
            RawTextContent {
                text: " this image:".to_string(),
                meta: None,
            }
            .no_annotation(),
        ));

        image_message = image_message.with_image("", "");

        let messages = vec![
            Message::user().with_text("hello"),
            image_message,
            Message::user().with_text("thanks"),
        ];

        let (fixed, issues) = run_verify(messages);

        assert_eq!(fixed.len(), 3);
        assert_has_issues_unordered!(fixed, issues, "Merged text content");
        let fixed_msg = &fixed[1];

        assert_eq!(fixed_msg.content.len(), 2);
        if let MessageContent::Text(text_content) = &fixed_msg.content[0] {
            assert_eq!(text_content.text, "Look at this image:");
        } else {
            panic!("Expected first item to be text content");
        }

        if let MessageContent::Image(_) = &fixed_msg.content[1] {
            // Good
        } else {
            panic!("Expected second item to be an image");
        }
    }

    #[test]
    fn test_agent_visible_non_visible_message_ordering_with_fixes() {
        // Test that non-visible messages maintain their position relative to visible messages
        // even when visible messages are fixed (merged, removed, etc.)

        // Create messages with mixed visibility where visible ones need fixing
        let mut msg1_user = Message::user().with_text("First user message");
        msg1_user.metadata.agent_visible = true;

        let mut msg2_non_visible = Message::user().with_text("Non-visible note 1");
        msg2_non_visible.metadata.agent_visible = false;

        // These two consecutive user messages should be merged (triggering a fix)
        let mut msg3_user = Message::user().with_text("Second user message");
        msg3_user.metadata.agent_visible = true;

        let mut msg4_user = Message::user().with_text("Third user message");
        msg4_user.metadata.agent_visible = true;

        let mut msg5_non_visible = Message::user().with_text("Non-visible note 2");
        msg5_non_visible.metadata.agent_visible = false;

        let mut msg6_assistant = Message::assistant().with_text("Assistant response");
        msg6_assistant.metadata.agent_visible = true;

        let mut msg7_non_visible = Message::user().with_text("Non-visible note 3");
        msg7_non_visible.metadata.agent_visible = false;

        let mut msg8_user = Message::user().with_text("Final user message");
        msg8_user.metadata.agent_visible = true;

        let messages = vec![
            msg1_user.clone(),
            msg2_non_visible.clone(),
            msg3_user.clone(),
            msg4_user.clone(),
            msg5_non_visible.clone(),
            msg6_assistant.clone(),
            msg7_non_visible.clone(),
            msg8_user.clone(),
        ];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages.clone()));

        // Should have merged consecutive user messages
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|i| i.contains("Merged consecutive")));

        let fixed_messages = fixed.messages();

        // Verify non-visible messages are still present
        let non_visible_texts: Vec<String> = fixed_messages
            .iter()
            .filter(|m| !m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(non_visible_texts.len(), 3);
        assert_eq!(non_visible_texts[0], "Non-visible note 1");
        assert_eq!(non_visible_texts[1], "Non-visible note 2");
        assert_eq!(non_visible_texts[2], "Non-visible note 3");

        // Verify visible messages were processed
        let visible_texts: Vec<String> = fixed_messages
            .iter()
            .filter(|m| m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        // Should have 3 visible messages: first user, merged user messages, assistant, final user
        // But after merging consecutive users and fixing lead/trail, we get fewer
        assert!(!visible_texts.is_empty());

        // The key assertion: non-visible messages should be preserved and not reordered
        // relative to each other
        let mut found_note1 = false;
        let mut found_note2 = false;

        for msg in fixed_messages {
            let text = msg.as_concat_text();
            if text == "Non-visible note 1" {
                assert!(!found_note2 && !found_note1);
                found_note1 = true;
            } else if text == "Non-visible note 2" {
                assert!(found_note1 && !found_note2);
                found_note2 = true;
            } else if text == "Non-visible note 3" {
                assert!(found_note1 && found_note2);
            }
        }
    }

    #[test]
    fn test_shadow_map_with_multiple_consecutive_merges() {
        // Test the shadow map handles multiple consecutive visible messages that all merge
        let mut msg1 = Message::user().with_text("User 1");
        msg1.metadata.agent_visible = true;

        let mut msg2_non_vis = Message::user().with_text("Non-visible A");
        msg2_non_vis.metadata.agent_visible = false;

        let mut msg3 = Message::user().with_text("User 2");
        msg3.metadata.agent_visible = true;

        let mut msg4 = Message::user().with_text("User 3");
        msg4.metadata.agent_visible = true;

        let mut msg5 = Message::user().with_text("User 4");
        msg5.metadata.agent_visible = true;

        let mut msg6_non_vis = Message::user().with_text("Non-visible B");
        msg6_non_vis.metadata.agent_visible = false;

        let messages = vec![
            msg1,
            msg2_non_vis.clone(),
            msg3,
            msg4,
            msg5,
            msg6_non_vis.clone(),
        ];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        // Should have merged the consecutive user messages
        assert!(issues.iter().any(|i| i.contains("Merged consecutive")));

        let fixed_messages = fixed.messages();

        // Non-visible messages should still be present and in order
        let non_visible: Vec<String> = fixed_messages
            .iter()
            .filter(|m| !m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(non_visible.len(), 2);
        assert_eq!(non_visible[0], "Non-visible A");
        assert_eq!(non_visible[1], "Non-visible B");

        // The merged message should contain all the user texts
        let visible: Vec<String> = fixed_messages
            .iter()
            .filter(|m| m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(visible.len(), 1);
        assert!(visible[0].contains("User 1"));
        assert!(visible[0].contains("User 2"));
        assert!(visible[0].contains("User 3"));
        assert!(visible[0].contains("User 4"));
    }

    #[test]
    fn test_shadow_map_with_leading_trailing_removal() {
        // Test that shadow map handles removal of leading/trailing assistant messages
        let mut msg1_assistant = Message::assistant().with_text("Leading assistant");
        msg1_assistant.metadata.agent_visible = true;

        let mut msg2_non_vis = Message::user().with_text("Non-visible note");
        msg2_non_vis.metadata.agent_visible = false;

        let mut msg3_user = Message::user().with_text("User message");
        msg3_user.metadata.agent_visible = true;

        let mut msg4_assistant = Message::assistant().with_text("Assistant response");
        msg4_assistant.metadata.agent_visible = true;

        let mut msg5_assistant = Message::assistant().with_text("Trailing assistant");
        msg5_assistant.metadata.agent_visible = true;

        let messages = vec![
            msg1_assistant,
            msg2_non_vis.clone(),
            msg3_user,
            msg4_assistant,
            msg5_assistant,
        ];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        // Should have merged consecutive assistants, removed leading, and removed trailing
        assert!(issues
            .iter()
            .any(|i| i.contains("Merged consecutive assistant")));
        assert!(issues
            .iter()
            .any(|i| i.contains("Removed leading assistant")));
        assert!(issues
            .iter()
            .any(|i| i.contains("Removed trailing assistant")));

        let fixed_messages = fixed.messages();

        // Non-visible message should still be present
        let non_visible: Vec<String> = fixed_messages
            .iter()
            .filter(|m| !m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(non_visible.len(), 1);
        assert_eq!(non_visible[0], "Non-visible note");

        // The two consecutive assistant messages get merged, then the merged message
        // is removed as trailing, leaving only the user message
        let visible: Vec<String> = fixed_messages
            .iter()
            .filter(|m| m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0], "User message");
    }

    #[test]
    fn test_shadow_map_all_visible_messages_removed() {
        // Edge case: all visible messages are removed, only non-visible remain
        let mut msg1_assistant = Message::assistant().with_text("Only assistant");
        msg1_assistant.metadata.agent_visible = true;

        let mut msg2_non_vis = Message::user().with_text("Non-visible note 1");
        msg2_non_vis.metadata.agent_visible = false;

        let mut msg3_non_vis = Message::user().with_text("Non-visible note 2");
        msg3_non_vis.metadata.agent_visible = false;

        let messages = vec![msg1_assistant, msg2_non_vis, msg3_non_vis];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        // Should have removed the assistant and added placeholder
        assert!(issues
            .iter()
            .any(|i| i.contains("Removed leading assistant")));
        assert!(issues.iter().any(|i| i.contains("Added placeholder")));

        let fixed_messages = fixed.messages();

        // Non-visible messages should still be present
        let non_visible: Vec<String> = fixed_messages
            .iter()
            .filter(|m| !m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(non_visible.len(), 2);
        assert_eq!(non_visible[0], "Non-visible note 1");
        assert_eq!(non_visible[1], "Non-visible note 2");

        // Should have placeholder user message
        let visible: Vec<String> = fixed_messages
            .iter()
            .filter(|m| m.metadata.agent_visible)
            .map(|m| m.as_concat_text())
            .collect();

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0], "Hello");
    }

    /// #51 part (b) depends on the preservation marker reaching compaction. The
    /// transcript that reaches it — and that the overflow path writes back to the
    /// store — is the *normalized* one, and `merge_consecutive_messages` keeps
    /// only the FIRST message's metadata and durable id. So an unpinned neighbour
    /// swallowing a pinned note destroys the marker (and the note's id) before any
    /// compaction function gets the chance to honour it.
    #[test]
    fn merging_does_not_swallow_a_pinned_message() {
        let messages = vec![
            Message::user()
                .with_id("m-plain")
                .with_text("summarise the logs"),
            Message::user()
                .with_id("m-note")
                .with_text("standing instruction: always cite sources")
                .pinned(),
        ];

        let (fixed, _issues) = fix_conversation(Conversation::new_unvalidated(messages));

        let note = fixed
            .messages()
            .iter()
            .find(|m| m.as_concat_text().contains("standing instruction"))
            .expect("the pinned note must survive normalization");
        assert!(
            note.is_pinned(),
            "normalization dropped the #51 preservation marker: {:#?}",
            fixed.messages()
        );
        assert_eq!(
            note.id.as_deref(),
            Some("m-note"),
            "the pinned note must keep its own durable id: {:#?}",
            fixed.messages()
        );
    }

    /// The mirror image: a pinned message that absorbs what follows lends its
    /// exemption to unrelated content, which then silently escapes summarization
    /// and spends the pinned-set token budget it never claimed.
    #[test]
    fn merging_does_not_broaden_a_pin_onto_later_content() {
        let messages = vec![
            Message::user()
                .with_id("m-note")
                .with_text("standing instruction: always cite sources")
                .pinned(),
            Message::user()
                .with_id("m-plain")
                .with_text("summarise the logs"),
        ];

        let (fixed, _issues) = fix_conversation(Conversation::new_unvalidated(messages));

        assert!(
            !fixed
                .messages()
                .iter()
                .any(|m| m.is_pinned() && m.as_concat_text().contains("summarise the logs")),
            "unpinned content inherited the pin's exemption: {:#?}",
            fixed.messages()
        );
        assert_eq!(
            fixed.len(),
            2,
            "the pin boundary must survive as a message boundary: {:#?}",
            fixed.messages()
        );
    }

    /// BR-71: `MessageProvenance` documents itself as "stamped in storage, not
    /// just in the UI, and never suppressible". `merge_consecutive_messages`
    /// keeps only the FIRST message's metadata, and the transcript it produces is
    /// what the overflow path writes back to the store — so an unstamped
    /// neighbour swallowing a stamped injection erases the stamp on the durable
    /// copy. Exactly the failure `is_pin_boundary` exists to prevent, one field
    /// over.
    #[test]
    fn merging_does_not_swallow_a_provenance_stamp() {
        let stamp = MessageProvenance {
            kind: ProvenanceKind::AgentInjection,
            from_session_id: Some("s-parent".into()),
            from_session_name: Some("Planning chat".into()),
        };
        let messages = vec![
            Message::user()
                .with_id("m-typed")
                .with_text("summarise the logs"),
            Message::user()
                .with_id("m-injected")
                .with_text("also open a PR")
                .with_provenance(stamp.clone()),
        ];

        let (fixed, _issues) = fix_conversation(Conversation::new_unvalidated(messages));

        let injected = fixed
            .messages()
            .iter()
            .find(|m| m.as_concat_text().contains("also open a PR"))
            .expect("the injected text must survive normalization");
        assert_eq!(
            injected.metadata.provenance.as_ref(),
            Some(&stamp),
            "normalization erased the BR-71 origin stamp: {:#?}",
            fixed.messages()
        );
    }

    /// The mirror image, and the worse of the two: a stamped message that absorbs
    /// what follows makes the human's own words read as agent-injected. A lost
    /// stamp under-attributes; this one MIS-attributes, which is what a reader of
    /// the transcript (and any future policy keyed on provenance) would act on.
    #[test]
    fn merging_does_not_broaden_a_provenance_stamp_onto_later_content() {
        let messages = vec![
            Message::user()
                .with_id("m-injected")
                .with_text("also open a PR")
                .with_provenance(MessageProvenance {
                    kind: ProvenanceKind::AgentInjection,
                    from_session_id: Some("s-parent".into()),
                    from_session_name: None,
                }),
            Message::user()
                .with_id("m-typed")
                .with_text("actually, stop and explain first"),
        ];

        let (fixed, _issues) = fix_conversation(Conversation::new_unvalidated(messages));

        assert!(
            !fixed
                .messages()
                .iter()
                .any(|m| m.metadata.provenance.is_some()
                    && m.as_concat_text().contains("actually, stop and explain")),
            "the user's own words were mis-attributed as an agent injection: {:#?}",
            fixed.messages()
        );
        assert_eq!(
            fixed.len(),
            2,
            "a provenance change must survive as a message boundary: {:#?}",
            fixed.messages()
        );
    }

    /// The boundary is a *change* of origin, not the mere presence of a stamp:
    /// two consecutive messages injected by the same session carry identical
    /// metadata, so merging them loses nothing and the ordinary provider-shape
    /// merge must still happen.
    #[test]
    fn merging_still_joins_two_injections_from_the_same_source() {
        let stamp = MessageProvenance {
            kind: ProvenanceKind::AgentInjection,
            from_session_id: Some("s-parent".into()),
            from_session_name: Some("Planning chat".into()),
        };
        let messages = vec![
            Message::user()
                .with_text("first half")
                .with_provenance(stamp.clone()),
            Message::user()
                .with_text("second half")
                .with_provenance(stamp.clone()),
        ];

        let (fixed, _issues) = fix_conversation(Conversation::new_unvalidated(messages));

        assert_eq!(
            fixed.len(),
            1,
            "identical provenance must not become a merge boundary: {:#?}",
            fixed.messages()
        );
        assert_eq!(
            fixed.messages()[0].metadata.provenance.as_ref(),
            Some(&stamp)
        );
    }

    /// Two adjacent pins stay two messages. The pinned-set budget evicts
    /// oldest-first and reports each evicted pin by its own id
    /// (`context_mgmt::pins::EvictedPin`); merging them would collapse that into
    /// one all-or-nothing unit under a single id.
    #[test]
    fn merging_keeps_adjacent_pins_separate() {
        let messages = vec![
            Message::user()
                .with_id("m-note-1")
                .with_text("first note")
                .pinned(),
            Message::user()
                .with_id("m-note-2")
                .with_text("second note")
                .pinned(),
        ];

        let (fixed, _issues) = fix_conversation(Conversation::new_unvalidated(messages));

        let ids: Vec<Option<&str>> = fixed.messages().iter().map(|m| m.id.as_deref()).collect();
        assert_eq!(
            ids,
            vec![Some("m-note-1"), Some("m-note-2")],
            "each pin keeps its own identity: {:#?}",
            fixed.messages()
        );
        assert!(fixed.messages().iter().all(|m| m.is_pinned()));
    }

    /// A pin is only a merge boundary where it can actually be honoured. Tool
    /// content is never eligible for preservation (exempting half a tool pair
    /// from a summarization that hides the other half is a rejected request), so
    /// a marked tool message must keep merging exactly as it did before — the
    /// provider-shape invariants win where no pin is at stake.
    #[test]
    fn a_marked_tool_message_still_merges() {
        use rmcp::model::CallToolResult;

        let messages = vec![
            Message::user().with_text("go"),
            Message::assistant()
                .with_tool_request(
                    "call-1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "developer__shell".into(),
                        arguments: Some(object!({})),
                        meta: None,
                    }),
                )
                .with_tool_request(
                    "call-2",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "developer__shell".into(),
                        arguments: Some(object!({})),
                        meta: None,
                    }),
                ),
            Message::user()
                .with_tool_response(
                    "call-1",
                    Ok(CallToolResult {
                        content: vec![rmcp::model::Content::text("out")],
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    }),
                )
                .pinned(),
            Message::user()
                .with_tool_response(
                    "call-2",
                    Ok(CallToolResult {
                        content: vec![rmcp::model::Content::text("more")],
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    }),
                )
                .pinned(),
        ];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        assert!(
            issues.iter().any(|i| i.contains("Merged consecutive tool")),
            "a pin that can never be honoured must not block the merge: {issues:?}"
        );
        assert_eq!(fixed.len(), 3, "{:#?}", fixed.messages());
    }

    /// The boundary rule delegates to `MessageContent::is_pin_eligible`, the one
    /// exhaustive ruling on what a pin can preserve — it must not re-derive its
    /// own exclusion list, which would default every future content variant to
    /// "boundary" and drift from what compaction actually honours.
    ///
    /// A signed thinking block is never pin-eligible, but its provider-authored
    /// message shape is still immutable while it is in an active Bedrock chain.
    #[test]
    fn a_marker_on_signed_non_preservable_content_does_not_merge_the_message() {
        let messages = vec![
            Message::user().with_text("go"),
            Message::assistant()
                .with_thinking("reasoning", "sig")
                .with_text("part one")
                .pinned(),
            Message::assistant().with_text("part two"),
            Message::user().with_text("thanks"),
        ];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        assert!(
            !issues
                .iter()
                .any(|i| i.contains("Merged consecutive assistant")),
            "signed assistant content must remain in its provider-authored message: {issues:?}"
        );
        assert_eq!(fixed.len(), 4, "{:#?}", fixed.messages());
    }

    #[test]
    fn test_shadow_map_preserves_interleaving_pattern() {
        // Test that complex interleaving patterns are preserved
        let mut msg1_user = Message::user().with_text("User 1");
        msg1_user.metadata.agent_visible = true;

        let mut msg2_non_vis = Message::user().with_text("Non-vis A");
        msg2_non_vis.metadata.agent_visible = false;

        let mut msg3_assistant = Message::assistant().with_text("Assistant 1");
        msg3_assistant.metadata.agent_visible = true;

        let mut msg4_non_vis = Message::user().with_text("Non-vis B");
        msg4_non_vis.metadata.agent_visible = false;

        let mut msg5_user = Message::user().with_text("User 2");
        msg5_user.metadata.agent_visible = true;

        let mut msg6_non_vis = Message::user().with_text("Non-vis C");
        msg6_non_vis.metadata.agent_visible = false;

        let messages = vec![
            msg1_user,
            msg2_non_vis,
            msg3_assistant,
            msg4_non_vis,
            msg5_user,
            msg6_non_vis,
        ];

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        // Should have no issues for this valid conversation
        assert!(issues.is_empty());

        let fixed_messages = fixed.messages();

        // Verify the interleaving pattern is preserved
        assert_eq!(fixed_messages.len(), 6);

        assert_eq!(fixed_messages[0].as_concat_text(), "User 1");
        assert!(fixed_messages[0].metadata.agent_visible);

        assert_eq!(fixed_messages[1].as_concat_text(), "Non-vis A");
        assert!(!fixed_messages[1].metadata.agent_visible);

        assert_eq!(fixed_messages[2].as_concat_text(), "Assistant 1");
        assert!(fixed_messages[2].metadata.agent_visible);

        assert_eq!(fixed_messages[3].as_concat_text(), "Non-vis B");
        assert!(!fixed_messages[3].metadata.agent_visible);

        assert_eq!(fixed_messages[4].as_concat_text(), "User 2");
        assert!(fixed_messages[4].metadata.agent_visible);

        assert_eq!(fixed_messages[5].as_concat_text(), "Non-vis C");
        assert!(!fixed_messages[5].metadata.agent_visible);
    }

    #[test]
    fn signed_assistant_edges_are_never_deleted_or_trimmed() {
        let leading = Message::assistant()
            .with_thinking("private reasoning", "signature-a")
            .with_text("answer with trailing space  ");
        let trailing = Message::assistant()
            .with_redacted_thinking("opaque-bytes")
            .with_text("final trailing tab\t");
        let leading_bytes = serde_json::to_vec(&leading).unwrap();
        let trailing_bytes = serde_json::to_vec(&trailing).unwrap();

        let (fixed, _) = fix_conversation(Conversation::new_unvalidated(vec![
            leading,
            Message::user().with_text("next"),
            trailing,
        ]));

        assert_eq!(fixed.len(), 3);
        assert_eq!(
            serde_json::to_vec(&fixed.messages()[0]).unwrap(),
            leading_bytes
        );
        assert_eq!(
            serde_json::to_vec(&fixed.messages()[2]).unwrap(),
            trailing_bytes
        );
    }

    #[test]
    fn signed_multi_tool_assistant_is_byte_exact_through_normalization() {
        let signed = Message::assistant()
            .with_thinking("reasoning", "signature")
            .with_text("keep whitespace  ")
            .with_tool_request(
                "tool-a",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "shell".into(),
                    arguments: Some(object!({"command": 7})),
                    meta: None,
                }),
            )
            .with_tool_request(
                "tool-b",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "shell".into(),
                    arguments: Some(object!({"command": "pwd"})),
                    meta: None,
                }),
            );
        let signed_bytes = serde_json::to_vec(&signed).unwrap();
        let result = || CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: Some(false),
            meta: None,
        };
        let (fixed, _) = fix_conversation(Conversation::new_unvalidated(vec![
            signed,
            Message::user().with_tool_response("tool-a", Ok(result())),
            Message::user().with_tool_response("tool-b", Ok(result())),
            Message::user().with_text("continue"),
        ]));

        assert_eq!(
            serde_json::to_vec(&fixed.messages()[0]).unwrap(),
            signed_bytes
        );
        assert_eq!(
            fixed.messages()[1]
                .content
                .iter()
                .filter(|content| matches!(content, MessageContent::ToolResponse(_)))
                .count(),
            2
        );
    }
}
