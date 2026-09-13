//! What a turn the user STOPPED leaves in the transcript.
//!
//! A Stop trips the turn's cancellation token, and the detached runner
//! (`biorouter-server`'s `workspace::turn::drive_stream`) drops the reply stream
//! at once. The reply loop only writes an iteration's rows at the END of that
//! iteration (`persist_iteration_messages`), so everything the iteration had
//! streamed — the half-answer the user was reading when they pressed Stop — was
//! never written. And the only word that the answer had been interrupted was a
//! renderer-only "Stopped." line on a five-second timer. So a reload, a second
//! window and History all showed a chat ending on the user's message, with no
//! reply and nothing to say one had been cut off (item 7, measured 2026-09-13 on
//! `biorouter serve` and on the desktop app).
//!
//! The runner now settles a stopped turn the way it already settled the steers a
//! cancelled turn had accepted (`Agent::settle_carried_over_soft_interrupts`): it
//! hands the agent what the dropped stream can no longer write. Two things.
//!
//! 1. **The prose the reply had streamed, and only the prose.** It is what the
//!    user read, so a transcript that silently loses it disagrees with the
//!    screen it came from; and a Stop-and-Send correction ("no — the other
//!    telescope") only makes sense to the model if it can see what it is
//!    correcting. Everything else in the unfinished iteration is dropped on
//!    purpose, because a provider will refuse to replay it: a thinking block
//!    whose signature never arrived, and a tool call with no result. That is the
//!    same line `TurnAbortCode::SignedStreamTruncated` draws when it discards a
//!    partial response, and text is the part of a response that never needs a
//!    signature.
//! 2. **A durable notice** — [`TURN_STOPPED_NOTICE`], persisted exactly like the
//!    planning gate's verdicts (`Agent::durable_notice`): a user-visible, model-
//!    hidden inline system notification. A stop is a turn-level verdict, not
//!    narration of a moment that has passed, which is the distinction that
//!    decides which notices in the reply loop are made durable.

use std::collections::HashSet;

use rmcp::model::Role;

use crate::conversation::message::{Message, MessageContent};

/// The notice a stopped turn ends on.
///
/// ⚠ **Mirrored in the desktop** as `TURN_STOPPED_NOTICE` in
/// `ui/desktop/src/components/conversation/turnStoppedNotice.ts`, which draws a
/// stored notice with this exact text as the same quiet "Stopped." line a
/// confirmed Stop shows live. A test below reads that file; change both or
/// neither. A reader that does not know the constant — the CLI's export, an older
/// desktop — still shows it, as the plain inline notice it is.
pub const TURN_STOPPED_NOTICE: &str = "Stopped.";

/// The part of a stopped iteration's streamed rows that is kept.
///
/// `in_flight` is what the runner saw streamed since the iteration's last
/// `MessagesPersisted` (so, normally, nothing the store holds). `stored_ids` is
/// what the store actually holds: the runner can drop a stream in the instant
/// between a row's INSERT committing and the event naming it being yielded, and a
/// row the store already holds must never be written twice.
///
/// Kept: assistant rows the user could see, that carry an id, whose id the store
/// does not hold, reduced to their non-blank text. Rows streamed under one id are
/// folded into one, in order. Dropped: everything else — the user's own rows
/// (persisted when they were sent), inline notices (live-only narration such as
/// "Retrying (1/3)…"), thinking, tool calls and their results, and an id-less row,
/// which cannot be checked against the store and so is never guessed at.
pub fn salvage_stopped_reply(in_flight: &[Message], stored_ids: &HashSet<String>) -> Vec<Message> {
    let mut kept: Vec<Message> = Vec::new();
    for message in in_flight {
        if message.role != Role::Assistant || !message.is_user_visible() {
            continue;
        }
        let Some(id) = message.id.as_deref() else {
            continue;
        };
        if stored_ids.contains(id) {
            continue;
        }
        let text: Vec<MessageContent> = message
            .content
            .iter()
            .filter(
                |content| matches!(content, MessageContent::Text(t) if !t.text.trim().is_empty()),
            )
            .cloned()
            .collect();
        if text.is_empty() {
            continue;
        }
        match kept.iter_mut().find(|row| row.id.as_deref() == Some(id)) {
            Some(row) => append_text(row, text),
            None => {
                let mut row = message.clone();
                row.content = text;
                kept.push(row);
            }
        }
    }
    kept
}

/// Fold `text` into `row`, joining adjacent text blocks the way a streamed reply
/// is joined everywhere else (`Conversation::push`).
fn append_text(row: &mut Message, text: Vec<MessageContent>) {
    for next in text {
        match (row.content.last_mut(), next) {
            (Some(MessageContent::Text(existing)), MessageContent::Text(next)) => {
                existing.text.push_str(&next.text);
            }
            (_, next) => row.content.push(next),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::SystemNotificationType;

    fn ids(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn keeps_the_prose_of_a_reply_the_store_does_not_hold() {
        let in_flight = vec![
            Message::assistant()
                .with_id("r")
                .with_thinking("plan", "sig"),
            Message::assistant()
                .with_id("r")
                .with_text("The telescope "),
            Message::assistant().with_id("r").with_text("was invented"),
        ];
        let kept = salvage_stopped_reply(&in_flight, &ids(&[]));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id.as_deref(), Some("r"));
        assert_eq!(kept[0].as_concat_text(), "The telescope was invented");
        assert!(kept[0]
            .content
            .iter()
            .all(|c| matches!(c, MessageContent::Text(_))));
    }

    #[test]
    fn never_writes_a_row_the_store_already_holds() {
        let in_flight = vec![Message::assistant().with_id("done").with_text("persisted")];
        assert!(salvage_stopped_reply(&in_flight, &ids(&["done"])).is_empty());
    }

    #[test]
    fn drops_what_cannot_be_replayed_or_was_only_narration() {
        let in_flight = vec![
            Message::user().with_id("u").with_text("the prompt"),
            Message::assistant()
                .with_id("thinking-only")
                .with_thinking("unsigned half-thought", ""),
            Message::assistant().with_id("tool").with_tool_request(
                "call-1",
                Err(rmcp::model::ErrorData::internal_error("x", None)),
            ),
            Message::assistant()
                .with_system_notification(SystemNotificationType::InlineMessage, "Retrying (1/3)…"),
            Message::assistant().with_text("an id-less row cannot be checked"),
            Message::assistant().with_id("blank").with_text("   "),
            Message::assistant()
                .with_id("hidden")
                .with_text("model-only plumbing")
                .agent_only(),
        ];
        assert!(salvage_stopped_reply(&in_flight, &ids(&[])).is_empty());
    }

    #[test]
    fn keeps_the_text_beside_a_tool_call_but_not_the_call() {
        let in_flight = vec![Message::assistant()
            .with_id("mixed")
            .with_text("Let me check.")
            .with_tool_request(
                "call-1",
                Err(rmcp::model::ErrorData::internal_error("x", None)),
            )];
        let kept = salvage_stopped_reply(&in_flight, &ids(&[]));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].content.len(), 1);
        assert_eq!(kept[0].as_concat_text(), "Let me check.");
    }

    /// The desktop recognises a stored stop notice by this text; see the doc on
    /// [`TURN_STOPPED_NOTICE`].
    #[test]
    fn the_desktop_mirrors_the_notice_text() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/conversation/turnStoppedNotice.ts");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let declaration = format!("export const TURN_STOPPED_NOTICE = '{TURN_STOPPED_NOTICE}';");
        assert!(
            source.contains(&declaration),
            "{} must declare `{declaration}`",
            path.display()
        );
    }
}
