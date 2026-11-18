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
//! 2. **A durable note.** What it says depends on who ended the turn
//!    ([`TurnEnd`]).
//!
//!    - **The person cancelled it** (Stop on the desktop, a `biorouter serve`
//!      page, `biorouter session cancel`, the terminal, an ACP editor): the
//!      turn ends on the cancel note, [`TURN_STOPPED_NOTICE`], built by
//!      [`turn_stopped_note`]. It is a RECORD, not a guard (owner ruling
//!      2026-11-17): the model on its next turn and anyone reading the chat
//!      later both see that a cancel happened here, and the next message
//!      decides what follows. So it is stored as an ordinary assistant text
//!      row that both the person and the model read. It takes the place of
//!      the reply the turn did not finish, which keeps the stored chat in its
//!      user and assistant shape: the salvaged prose and the note are two
//!      assistant rows in a row, which `fix_conversation` sends as one reply,
//!      so the next request carries the note once. A system notification
//!      would not do, because every wire format drops those, and neither
//!      would a user-only notice.
//!    - **Something else cancelled it** (another agent's `workspace_close`, the
//!      chat that started a subagent, a monitor, the orphan reaper, a Crew
//!      task's time limit): the turn ends on [`TURN_ENDED_NOTICE`], the short
//!      "Stopped." line every stopped turn ended on before the cancel note. It
//!      is for people only, as it always was, because the cancel note would
//!      say a person did something nobody did.

use std::collections::HashSet;

use rmcp::model::Role;

use crate::conversation::message::{Message, MessageContent};

/// The note a turn the person cancelled ends on: a plain record of what
/// happened, which tells the model nothing about what to do next.
///
/// ⚠ **Mirrored in the desktop** as `TURN_STOPPED_NOTICE` in
/// `ui/desktop/src/components/conversation/turnStoppedNotice.ts`, which draws a
/// stored note with this exact text as the same quiet line a confirmed Stop
/// shows live. A test below reads that file; change both or neither. A reader
/// that does not know the constant (the CLI's export, an older desktop) shows it
/// as the line of the agent's it is.
pub const TURN_STOPPED_NOTICE: &str = "The person canceled this turn before it finished.";

/// The line a turn ends on when something other than the person cancelled it,
/// and the one every stopped turn ended on before [`TURN_STOPPED_NOTICE`]: an
/// inline system notification the person sees and the model never reads.
///
/// ⚠ Mirrored in the desktop as `TURN_ENDED_NOTICE`, beside the other.
pub const TURN_ENDED_NOTICE: &str = "Stopped.";

/// Who ended an unfinished turn, which decides the note it ends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnEnd {
    /// The person cancelled it: it ends on [`TURN_STOPPED_NOTICE`], which the
    /// model reads on its next turn.
    CancelledByThePerson,
    /// Something other than the person cancelled it: it ends on
    /// [`TURN_ENDED_NOTICE`], for people only.
    CancelledOtherwise,
    /// The reply stream failed. Not a cancel, so no note: the turn's error says
    /// what happened.
    Failed,
}

impl TurnEnd {
    /// The ending of a turn whose cancellation token tripped.
    pub fn cancelled(by_the_person: bool) -> Self {
        if by_the_person {
            Self::CancelledByThePerson
        } else {
            Self::CancelledOtherwise
        }
    }
}

/// The cancel note, as it is stored: an assistant text row with default
/// visibility, so the person and the model both read it.
///
/// `after_the_agents_text` says the row before it is the agent's own text (the
/// prose a stopped reply had streamed). The two rows are then sent as one reply
/// (`fix_conversation` merges consecutive assistant rows, and a later pass joins
/// their texts as written), so the note opens a new paragraph there; without
/// the break the model read "The telescope was invented inThe person
/// cancelled…". Every reader compares the note's words trimmed.
pub fn turn_stopped_note(after_the_agents_text: bool) -> Message {
    let text = if after_the_agents_text {
        format!("\n\n{TURN_STOPPED_NOTICE}")
    } else {
        TURN_STOPPED_NOTICE.to_string()
    };
    Message::assistant().with_text(text)
}

/// Does the next row after `previous` (the last row the model reads) join it
/// as one reply? See [`turn_stopped_note`].
pub fn ends_on_the_agents_text(previous: Option<&Message>) -> bool {
    previous.is_some_and(|message| {
        message.role == Role::Assistant
            && matches!(message.content.last(), Some(MessageContent::Text(_)))
    })
}

/// Is this row the cancel note? Exact words (trimmed) and shape, nothing looser.
pub fn is_turn_stopped_note(message: &Message) -> bool {
    message.role == Role::Assistant
        && matches!(
            message.content.as_slice(),
            [MessageContent::Text(text)] if text.text.trim() == TURN_STOPPED_NOTICE
        )
}

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

    /// The desktop recognises a stored stop row by these texts; see the doc on
    /// [`TURN_STOPPED_NOTICE`].
    #[test]
    fn the_desktop_mirrors_the_notice_text() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/conversation/turnStoppedNotice.ts");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        for declaration in [
            format!("export const TURN_STOPPED_NOTICE = '{TURN_STOPPED_NOTICE}';"),
            format!("export const TURN_ENDED_NOTICE = '{TURN_ENDED_NOTICE}';"),
        ] {
            assert!(
                source.contains(&declaration),
                "{} must declare `{declaration}`",
                path.display()
            );
        }
    }

    /// The cancel note is the agent's text with default visibility, the one
    /// shape every wire format carries to the model and every surface shows.
    #[test]
    fn the_cancel_note_is_text_the_person_and_the_model_read() {
        let note = turn_stopped_note(false);
        assert!(is_turn_stopped_note(&note));
        assert!(note.is_user_visible() && note.is_agent_visible());
        assert_eq!(note.as_concat_text(), TURN_STOPPED_NOTICE);
        let after_prose = turn_stopped_note(true);
        assert!(is_turn_stopped_note(&after_prose));
        assert_eq!(
            after_prose.as_concat_text(),
            format!("\n\n{TURN_STOPPED_NOTICE}"),
            "after the agent's text the note opens a new paragraph"
        );
        assert!(!note
            .content
            .iter()
            .any(|content| matches!(content, MessageContent::SystemNotification(_))));
    }

    /// Exact words and shape: a reply that says more, the person's own words,
    /// or a notice carrying the same text is not the note.
    #[test]
    fn only_the_note_itself_is_the_note() {
        let longer = Message::assistant().with_text(format!("{TURN_STOPPED_NOTICE} More."));
        let typed = Message::user().with_text(TURN_STOPPED_NOTICE);
        let notice = Message::assistant()
            .with_system_notification(SystemNotificationType::InlineMessage, TURN_STOPPED_NOTICE);
        let two = turn_stopped_note(false).with_text(" Again.");
        for lookalike in [longer, typed, notice, two] {
            assert!(!is_turn_stopped_note(&lookalike), "{lookalike:?}");
        }
    }

    #[test]
    fn the_note_opens_a_paragraph_only_after_the_agents_text() {
        let prose = Message::assistant().with_text("The telescope was invented in");
        let asked = Message::user().with_text("When?");
        let call = Message::assistant().with_tool_request(
            "call-1",
            Err(rmcp::model::ErrorData::internal_error("x", None)),
        );
        assert!(ends_on_the_agents_text(Some(&prose)));
        assert!(!ends_on_the_agents_text(Some(&asked)));
        assert!(!ends_on_the_agents_text(Some(&call)));
        assert!(!ends_on_the_agents_text(None));
    }

    #[test]
    fn only_the_persons_cancel_ends_on_the_note() {
        assert_eq!(TurnEnd::cancelled(true), TurnEnd::CancelledByThePerson);
        assert_eq!(TurnEnd::cancelled(false), TurnEnd::CancelledOtherwise);
    }
}
