//! The rolling window a monitor hook asks its questions about: a bounded tail
//! of the conversation, built by the stall checker's own renderer
//! ([`crate::agents::stall::progress_tail`]) so a monitor and the loop checker
//! see the same lines. PR #469 made the same choice.
//!
//! What the tail holds, from `progress_tail`: user and assistant text,
//! `tool call: name(args)` and `tool result [ok|error]: ...` lines, with
//! system notifications skipped; at most 24 entries of 400 characters each and
//! 6,000 characters in all. The window narrows that further when asked (fewer
//! messages, fewer characters, no tool traffic) so it fits a small decision
//! model's context. It is built from stored messages, which have already
//! passed the secret-output guard.
//!
//! A window's message count is a count of messages that show something in
//! it. The conversation is read newest first, a message's tool traffic is
//! left out first when the window has no tool traffic, and a message left
//! with nothing to show (a run of tool steps, for a text-only window, or a
//! notification) takes no place. So a text-only window of two messages holds
//! the two newest messages with text, however many tool steps came after
//! them.

use sha2::{Digest, Sha256};

use super::types::{ContextMessage, DecisionContext};
use crate::agents::stall::{progress_tail, renders_in_tail};
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;

/// `progress_tail`'s own entry cap. A window asks for at most this many.
pub const MAX_WINDOW_MESSAGES: usize = 24;
/// `progress_tail`'s own character cap.
pub const MAX_WINDOW_CHARS: usize = 6_000;

/// How much of the conversation a monitor sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSpec {
    /// The most recent messages that show something in the window (text,
    /// or tool traffic when `tool_calls` is on), at most
    /// [`MAX_WINDOW_MESSAGES`].
    pub messages: usize,
    /// Characters of rendered tail kept, at most [`MAX_WINDOW_CHARS`]. The
    /// newest lines are kept.
    pub max_chars: usize,
    /// Include tool calls and results, not only text.
    pub tool_calls: bool,
}

impl Default for WindowSpec {
    fn default() -> Self {
        Self {
            messages: MAX_WINDOW_MESSAGES,
            max_chars: MAX_WINDOW_CHARS,
            tool_calls: true,
        }
    }
}

impl WindowSpec {
    /// Cap the character budget so the window fits a model of
    /// `context_tokens`, leaving `reserve_tokens` for the questions. Uses a
    /// conservative three characters per token.
    pub fn fitted_to(mut self, context_tokens: usize, reserve_tokens: usize) -> Self {
        let room = context_tokens
            .saturating_sub(reserve_tokens)
            .saturating_mul(3);
        self.max_chars = self.max_chars.min(room);
        self
    }
}

/// A built window: the state to send, plus what the decision log needs.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    pub context: DecisionContext,
    /// Hex SHA-256 of the rendered tail (the opening line, which names the
    /// loop iteration, is not part of it). Logged with the request; every
    /// due monitor is asked at every point, whether or not it changed.
    pub sha256: String,
    /// Characters of rendered tail.
    pub chars: usize,
    /// Messages of the conversation the tail was drawn from: those that show
    /// something in it.
    pub messages_considered: usize,
}

/// Build the window over `conversation` at loop `iteration`, or `None` when
/// there is nothing to judge.
///
/// The messages are chosen before they are counted: newest first, each with
/// its tool traffic left out when `spec.tool_calls` is off, and only those
/// that still show something ([`renders_in_tail`]) take one of the
/// `spec.messages` places.
pub fn build_window(
    conversation: &Conversation,
    spec: WindowSpec,
    iteration: u32,
) -> Option<WindowState> {
    let take = spec.messages.clamp(1, MAX_WINDOW_MESSAGES);
    let mut selected: Vec<Message> = Vec::with_capacity(take);
    for message in conversation.messages().iter().rev() {
        if selected.len() >= take {
            break;
        }
        let is_tool_traffic = |content: &MessageContent| {
            matches!(
                content,
                MessageContent::ToolRequest(_) | MessageContent::ToolResponse(_)
            )
        };
        let message = if spec.tool_calls {
            message.clone()
        } else if message.content.iter().all(is_tool_traffic) {
            // A tool step shows nothing in a text-only window; not cloned.
            continue;
        } else {
            let mut message = message.clone();
            message.content.retain(|content| !is_tool_traffic(content));
            message
        };
        if !message.content.is_empty() && renders_in_tail(&message) {
            selected.push(message);
        }
    }
    selected.reverse();
    let messages_considered = selected.len();
    let tail = progress_tail(&Conversation::new_unvalidated(selected))?;
    let tail = keep_newest(&tail, spec.max_chars.min(MAX_WINDOW_CHARS));
    if tail.trim().is_empty() {
        return None;
    }
    let sha256 = hex(Sha256::digest(tail.as_bytes()).as_slice());
    let chars = tail.chars().count();
    Some(WindowState {
        context: DecisionContext::Messages(vec![ContextMessage {
            role: "user".to_string(),
            content: format!(
                "Recent work of an AI agent at loop iteration {iteration}, oldest first. \
                 Each line is the user, the assistant, a tool call or a tool result.\n\n{tail}"
            ),
        }]),
        sha256,
        chars,
        messages_considered,
    })
}

/// The last `max_chars` characters of `tail`, cut at a line start where one
/// falls inside the budget, so the window opens on a whole line.
fn keep_newest(tail: &str, max_chars: usize) -> String {
    let count = tail.chars().count();
    if count <= max_chars {
        return tail.to_string();
    }
    let skip = count - max_chars;
    let kept: String = tail.chars().skip(skip).collect();
    match kept.split_once('\n') {
        Some((_, rest)) if !rest.is_empty() => rest.to_string(),
        _ => kept,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content};

    fn conversation(messages: Vec<Message>) -> Conversation {
        Conversation::new_unvalidated(messages)
    }

    fn with_tools() -> Conversation {
        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: "browser__navigate".into(),
            arguments: Some(rmcp::object!({"url": "https://example.test/consent"})),
        };
        conversation(vec![
            Message::user().with_text("Open the consent form."),
            Message::assistant()
                .with_text("Opening it.")
                .with_tool_request("t1", Ok(call)),
            Message::user().with_tool_response(
                "t1",
                Ok(CallToolResult::success(vec![Content::text(
                    "Consent form loaded",
                )])),
            ),
            Message::assistant().with_text("The consent form is open."),
        ])
    }

    fn text_of(state: &WindowState) -> String {
        match &state.context {
            DecisionContext::Messages(messages) => messages[0].content.clone(),
            other => panic!("a messages state, not {other:?}"),
        }
    }

    #[test]
    fn the_window_is_the_progress_tail_as_one_message() {
        let state = build_window(&with_tools(), WindowSpec::default(), 3).unwrap();
        let text = text_of(&state);
        assert!(text.contains("loop iteration 3"));
        assert!(text.contains("user: Open the consent form."));
        assert!(text.contains("tool call: browser__navigate("));
        assert!(text.contains("tool result [ok]: Consent form loaded"));
        assert!(text.ends_with("assistant: The consent form is open."));
        assert_eq!(state.sha256.len(), 64);
        assert_eq!(state.messages_considered, 4);
    }

    #[test]
    fn tool_traffic_can_be_left_out() {
        let spec = WindowSpec {
            tool_calls: false,
            ..WindowSpec::default()
        };
        let text = text_of(&build_window(&with_tools(), spec, 1).unwrap());
        assert!(!text.contains("tool call: "));
        assert!(!text.contains("tool result ["));
        assert!(text.contains("assistant: Opening it."));
    }

    /// A text-only window counts messages with text: a run of tool steps
    /// after the newest text takes none of its places, so the window still
    /// holds that text instead of coming out empty.
    #[test]
    fn a_text_only_window_keeps_the_newest_text_past_a_run_of_tool_steps() {
        let mut messages = vec![
            Message::user().with_text("Open the consent form."),
            Message::assistant().with_text("Opening it now."),
        ];
        for i in 0..5 {
            let call = CallToolRequestParams {
                task: None,
                meta: None,
                name: "browser__click".into(),
                arguments: Some(rmcp::object!({"step": i})),
            };
            let id = format!("t{i}");
            messages.push(Message::assistant().with_tool_request(&id, Ok(call)));
            messages.push(Message::user().with_tool_response(
                &id,
                Ok(CallToolResult::success(vec![Content::text("clicked")])),
            ));
        }
        messages.push(Message::assistant().with_system_notification(
            crate::conversation::message::SystemNotificationType::InlineMessage,
            "Monitor 'x' fired.",
        ));
        assert_eq!(
            messages.len(),
            13,
            "two texts, ten tool-only messages, a notice"
        );
        let spec = WindowSpec {
            messages: 2,
            max_chars: MAX_WINDOW_CHARS,
            tool_calls: false,
        };
        let state = build_window(&conversation(messages.clone()), spec, 4)
            .expect("the two texts make a window");
        let text = text_of(&state);
        assert!(text.contains("user: Open the consent form."), "{text}");
        assert!(text.contains("assistant: Opening it now."), "{text}");
        assert!(!text.contains("tool call:"), "{text}");
        assert!(!text.contains("tool result ["), "{text}");
        assert!(!text.contains("fired"), "{text}");
        assert_eq!(state.messages_considered, 2);

        // With tool traffic, the newest two messages that show something are
        // the last tool step: the notification takes no place.
        let with_tools = WindowSpec {
            tool_calls: true,
            ..spec
        };
        let text = text_of(&build_window(&conversation(messages), with_tools, 4).unwrap());
        assert!(
            text.contains("tool call: browser__click({\"step\":4})"),
            "{text}"
        );
        assert!(text.contains("tool result [ok]: clicked"), "{text}");
        assert!(!text.contains("Opening it now."), "{text}");
    }

    #[test]
    fn the_window_is_bounded_and_keeps_the_newest_lines() {
        let messages: Vec<Message> = (0..40)
            .map(|i| Message::user().with_text(format!("message number {i} {}", "x".repeat(300))))
            .collect();
        let conv = conversation(messages);
        let spec = WindowSpec {
            messages: 5,
            max_chars: 700,
            tool_calls: true,
        };
        let state = build_window(&conv, spec, 9).unwrap();
        let text = text_of(&state);
        assert!(state.chars <= 700, "{}", state.chars);
        assert!(text.contains("message number 39"));
        assert!(!text.contains("message number 34"));
        assert_eq!(state.messages_considered, 5);
        // Never more than the tail's own caps, whatever is asked.
        let wide = WindowSpec {
            messages: 1000,
            max_chars: 1_000_000,
            tool_calls: true,
        };
        assert!(build_window(&conv, wide, 1).unwrap().chars <= MAX_WINDOW_CHARS);
    }

    #[test]
    fn the_hash_changes_only_with_the_tail() {
        let a = build_window(&with_tools(), WindowSpec::default(), 1).unwrap();
        let b = build_window(&with_tools(), WindowSpec::default(), 2).unwrap();
        assert_eq!(
            a.sha256, b.sha256,
            "the iteration number is not part of the hash"
        );
        let mut more = with_tools().messages().clone();
        more.push(Message::user().with_text("Now sign it."));
        let c = build_window(&conversation(more), WindowSpec::default(), 2).unwrap();
        assert_ne!(a.sha256, c.sha256);
    }

    #[test]
    fn an_empty_conversation_has_no_window_and_fitting_shrinks_the_budget() {
        assert!(build_window(&conversation(vec![]), WindowSpec::default(), 1).is_none());
        let fitted = WindowSpec::default().fitted_to(2_000, 1_000);
        assert_eq!(fitted.max_chars, 3_000);
        assert_eq!(
            WindowSpec::default().fitted_to(16_384, 512).max_chars,
            MAX_WINDOW_CHARS
        );
    }
}
