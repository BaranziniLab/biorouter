//! Cards in `session watch` and `session attach`: an approval, a question, a credential
//! request or a Crew step that a chat is waiting on.
//!
//! The observer stream carries each card as a `Message` frame whose content is one
//! `actionRequired` block. `watch` prints it as one line ([`card_line`]), so a turn that stops
//! on a card no longer looks stuck for no reason. `attach` prints the same line and, for an
//! approval, asks the person at this terminal and posts the answer with the user-action key it
//! already holds ([`Waiting`], [`answer_approval`]).
//!
//! ⚠ **An approval is shown whole before it is answered** (AO-12: "the payload is shown
//! whole"). The details `attach` prints are every line of the card's prompt and the call's
//! arguments in full, never a clipped copy ([`approval_details`]): a person who types
//! `allow_once` here posts, as themselves, exactly what they read.
//!
//! ⚠ **`attach` answers only the person typing at it** ([`Answering`]). Reading a pipe or
//! running in an agent's own shell (`BIOROUTER_TERMINAL`), it prints the card and points to the
//! desktop app, and its lines still go to the chat as messages. This is accident prevention, as
//! D5 is for `biorouter crew`: a script or an agent's shell must not approve a card by writing
//! `allow_once` to it.
//!
//! ⚠ **No argument whose name looks like a credential is ever printed.** A line names the tool
//! and the first line of the card's own prompt, never its arguments. The details `attach` shows
//! before it asks hide every value under a credential-like key and pass the rest through the
//! secret-output redactor ([`approval_details`]), and a shared-daemon chat shows a card whole
//! the same way ([`shown_card_json`]). A credential card prints key names only; a Crew step
//! prints the daemon's own fields and says where to take it, never here.
//!
//! The read consent (AO-12) lasts until the person revokes it, so every terminal offers it as
//! `allow` or `deny` and names the command that takes it back ([`read_consent_choices`]).

use std::collections::VecDeque;

use anyhow::Result;
use biorouter::conversation::message::{
    ActionRequiredData, CrewStepRequest, CrewVaultAction, SecretKeyRequest,
};
use biorouter::conversation::tool_preview::ToolPreview;
use biorouter::crew::tool_host::PersonTool;
use biorouter::permission::tool_risk::ToolRisk;
use serde_json::{Map, Value};

use super::DaemonAuth;
use crate::commands::shared_conversation::safe_text;
use crate::session::APPROVAL_EXPIRED;

/// The marker Crew's daemon host sets in a read consent card's arguments
/// (`Ctx::read_consent` in `biorouter-server/src/routes/crew_tools/mod.rs`, `"action": "read as
/// you"`). The desktop reads the same marker (`isCrewReadConsent` in `toolCallCopy.ts`).
const READ_CONSENT_ACTION: &str = "read as you";

/// The heading every read consent prompt starts with (`consent_prompt` on the daemon), which the
/// desktop also accepts when the marker is missing.
const READ_CONSENT_HEADING: &str = "Let this chat read ";

/// Where a card this terminal can't answer is answered, where the chat's ID is not at hand.
///
/// ⚠ It used to add "or in a biorouter session --shared-daemon chat", which promised more than
/// is true: a terminal chat answers only the cards of its own conversation, so a fresh one
/// answers nothing here. Where the ID is at hand, [`answer_elsewhere`] names the one terminal
/// chat that can.
const ANSWER_ELSEWHERE: &str = "Answer it in the desktop app";

/// [`ANSWER_ELSEWHERE`] for the chat `session_id`: the desktop app, or a terminal chat on this
/// same chat, which is the only terminal chat that answers its cards.
fn answer_elsewhere(session_id: &str) -> String {
    format!(
        "{ANSWER_ELSEWHERE}, or open this chat in a terminal with biorouter session --shared-daemon --resume --session-id {session_id}"
    )
}

/// The one-line form of a card (`watch`, and the line `attach` prints first) clips its heading
/// to this many characters. The details `attach` asks with are never clipped.
const LINE_CHARS: usize = 240;

/// What a value this terminal never prints shows instead: the word the daemon's secret-output
/// redactor writes for a value under a secret's name, which is what the desktop's tool card shows
/// for a hidden value. The redactor never rewrites its own marker, so a value hidden by key name
/// and one the redactor hid read the same, whatever the key's spelling.
const HIDDEN_VALUE: &str = "[REDACTED:secret-value]";

/// Argument names whose values are never printed, matched case-insensitively on a substring.
const CREDENTIAL_NAMES: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "passcode",
    "secret",
    "token",
    "credential",
    "api_key",
    "apikey",
    "private_key",
    "privatekey",
    "authorization",
    "cookie",
];

/// Whether `action` is AO-12's read consent: the one `crew__read` card. Unlike any other
/// approval it lasts, for this chat and workspace, until the person revokes it, so it is never
/// offered as "once".
pub(crate) fn is_read_consent(action: &ActionRequiredData) -> bool {
    let ActionRequiredData::ToolConfirmation {
        tool_name,
        arguments,
        prompt,
        ..
    } = action
    else {
        return false;
    };
    tool_name == PersonTool::Read.name()
        && (arguments.get("action").and_then(Value::as_str) == Some(READ_CONSENT_ACTION)
            || prompt
                .as_deref()
                .is_some_and(|prompt| prompt.starts_with(READ_CONSENT_HEADING)))
}

/// The read consent's two choices, in the words every terminal offers them with: allowing it
/// lasts until the person revokes it, and the sentence says how.
pub(crate) fn read_consent_choices(session_id: &str) -> String {
    format!(
        "allow (until you revoke it with biorouter crew grants revoke {session_id} --reads) or deny"
    )
}

/// Said once a read consent is allowed.
pub(crate) fn read_consent_allowed(session_id: &str) -> String {
    format!(
        "Allowed. This chat may read the workspace as you until you revoke it with biorouter crew grants revoke {session_id} --reads."
    )
}

/// The daemon's answer word for what the person typed, or `None` when it answers nothing. A
/// read consent is allowed with `allow`; any other approval with `allow_once`. Both send
/// `allow_once`, the one allow a card with a prompt takes (a read consent then lasts because
/// the daemon records it, not because of the word).
pub(crate) fn approval_answer(typed: &str, read_consent: bool) -> Option<&'static str> {
    let typed = typed.trim();
    let allow = if read_consent { "allow" } else { "allow_once" };
    if typed.eq_ignore_ascii_case(allow) {
        Some("allow_once")
    } else if typed.eq_ignore_ascii_case("deny") {
        Some("deny")
    } else {
        None
    }
}

/// The first line of `text`, clipped, or `None` when it is blank.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(clip(line, LINE_CHARS))
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push('…');
    clipped
}

/// A Crew step's heading, in the desktop card's words (`CrewSignInStep.copy.ts`,
/// `CrewVaultStep.copy.ts`, and `trustCopy` for `CrewHostKeyStep.copy.ts`), which the
/// shared-daemon chat's own step lines also use (`crew_step/host_key.rs`).
fn crew_step_heading(step: &CrewStepRequest) -> String {
    match step {
        CrewStepRequest::SignIn { login, .. } => format!("Sign in to {login}"),
        CrewStepRequest::Vault {
            action: CrewVaultAction::Unlock,
        } => "Unlock Crew's vault".to_string(),
        CrewStepRequest::Vault {
            action: CrewVaultAction::Init,
        } => "Set up Crew's vault".to_string(),
        CrewStepRequest::HostKey {
            host,
            changed: true,
            ..
        } => format!("{host}'s identity changed"),
        CrewStepRequest::HostKey { host, .. } => format!("Can't verify {host} yet"),
    }
}

fn key_names(keys: &[SecretKeyRequest]) -> String {
    keys.iter()
        .map(|key| key.key.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// One line for a card's `data`, or `None` for a record that is not a card (an answered
/// question's response, which carries what the person typed and is never printed).
pub(crate) fn card_line(data: &Value) -> Option<String> {
    let line = match serde_json::from_value::<ActionRequiredData>(data.clone()) {
        Ok(ActionRequiredData::ToolConfirmation {
            tool_name, prompt, ..
        }) => match prompt.as_deref().and_then(first_line) {
            Some(heading) => format!("[approval] {tool_name}: {heading}"),
            None => format!("[approval] {tool_name}"),
        },
        Ok(ActionRequiredData::Elicitation { message, .. }) => {
            format!("[question] {}", first_line(&message).unwrap_or_default())
        }
        Ok(ActionRequiredData::ElicitationResponse { .. }) => return None,
        Ok(ActionRequiredData::SecretRequest { prompt, keys, .. }) => format!(
            "[credentials] {} ({}). {ANSWER_ELSEWHERE}.",
            first_line(&prompt).unwrap_or_default(),
            key_names(&keys)
        ),
        // A changed host key's notice (WI-17) waits for nothing and is kept in the chat, so it
        // asks for no answer, live or read back.
        Ok(ActionRequiredData::CrewStep { step, .. })
            if biorouter::pending_user_action::crew_step_is_a_notice(&step) =>
        {
            format!(
                "[crew] {}. Nothing connects to it until the server's IT team confirms the change.",
                crew_step_heading(&step)
            )
        }
        Ok(ActionRequiredData::CrewStep { step, .. }) => format!(
            "[crew] {}. {ANSWER_ELSEWHERE}, never by typing here.",
            crew_step_heading(&step)
        ),
        Err(_) => format!("[waiting] This chat is waiting for your answer. {ANSWER_ELSEWHERE}."),
    };
    Some(safe_text(&redacted(&line)))
}

/// The card lines of an observer `message`: one per `actionRequired` block.
pub(crate) fn message_card_lines(message: &Value) -> Vec<String> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("actionRequired"))
        .filter_map(|block| block.get("data"))
        .filter_map(card_line)
        .collect()
}

fn looks_like_a_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_NAMES.iter().any(|needle| name.contains(needle))
}

/// `value` with every value under a credential-like key replaced, at any depth.
fn hide_credentials(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| {
                    let shown = if looks_like_a_credential(name) {
                        Value::String(HIDDEN_VALUE.into())
                    } else {
                        hide_credentials(value)
                    };
                    (name.clone(), shown)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(hide_credentials).collect()),
        other => other.clone(),
    }
}

/// `text` with any credential the secret-output guard recognises withheld.
fn redacted(text: &str) -> String {
    biorouter::guardrails::secret_output::redact_text(text)
        .map(|(text, _)| text)
        .unwrap_or_else(|| text.to_string())
}

/// An approval card as a shared-daemon chat shows it whole (`shared_conversation`): its JSON,
/// with every credential-like argument value hidden, the arguments preview (a second copy of the
/// same arguments) left out, and the rest passed through the secret-output redactor. A diff, a
/// new file or a command is still shown in full.
pub(crate) fn shown_card_json(action: &ActionRequiredData) -> String {
    let mut value = serde_json::to_value(action).unwrap_or_default();
    if let Some(card) = value.as_object_mut() {
        if let Some(arguments) = card.get("arguments").map(hide_credentials) {
            card.insert("arguments".into(), arguments);
        }
        let repeats_arguments = card
            .get("preview")
            .and_then(|preview| preview.get("kind"))
            .and_then(Value::as_str)
            == Some("arguments");
        if repeats_arguments {
            card.remove("preview");
        }
    }
    redacted(&serde_json::to_string_pretty(&value).unwrap_or_default())
}

/// `text` under `label`: its first line after the label, every other line indented below it.
fn labelled_block(label: &str, text: &str) -> Vec<String> {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let mut shown = vec![format!("{label} {first}").trim_end().to_string()];
    shown.extend(lines.map(|line| format!("  {line}")));
    shown
}

/// The call's arguments, whole and pretty-printed, with every credential-like value hidden and
/// the rest passed through the secret-output redactor. Nothing for a call with none.
fn argument_lines(arguments: &Map<String, Value>) -> Vec<String> {
    if arguments.is_empty() {
        return Vec::new();
    }
    let hidden = hide_credentials(&Value::Object(arguments.clone()));
    let text = serde_json::to_string_pretty(&hidden).unwrap_or_default();
    labelled_block("Arguments:", &redacted(&text))
}

fn risk_line(risk: ToolRisk) -> &'static str {
    match risk {
        ToolRisk::Low => "Risk: read-only",
        ToolRisk::Medium => "Risk: modifies data",
        ToolRisk::High => "Risk: destructive",
        ToolRisk::Unknown => "Risk: unverified",
    }
}

/// The command a shell card will run, when its preview holds it whole: then it is what the call
/// runs, read more easily than the arguments. `None` for a clipped command or any other preview.
fn whole_command(preview: &ToolPreview) -> Option<Vec<String>> {
    match preview {
        ToolPreview::Shell {
            command,
            truncated: false,
        } => Some(labelled_block("Command:", &redacted(command))),
        _ => None,
    }
}

/// A one-line summary of what a preview shows, for a preview whose body is not shown here: the
/// arguments below it are. The diff and the new file are clipped by the daemon to fit a card,
/// so they are never what a person answers from in this terminal.
fn preview_summary(preview: &ToolPreview) -> Option<String> {
    Some(match preview {
        ToolPreview::FileEdit {
            path,
            added,
            removed,
            ..
        } => format!("Edits {path}: {added} lines added, {removed} removed"),
        ToolPreview::FileWrite {
            path, line_count, ..
        } => format!("Writes {path}: {line_count} lines"),
        ToolPreview::FolderAccess { folder, access, .. } => {
            format!("Folder: {folder} ({access})")
        }
        ToolPreview::Shell { .. } | ToolPreview::Arguments { .. } => return None,
    })
}

/// What a person reads before answering an approval, whole (AO-12): every line of the card's
/// prompt (the daemon's or the tool's own words), how risky the call is, then what it will do.
/// A shell command is shown as the preview has it only when the preview holds it whole; any
/// other call shows its arguments, whole, because a preview is clipped to fit a card and the
/// arguments are what runs. Nothing is clipped. Every value under a credential-like key is
/// hidden, everything passes through the secret-output redactor, and each line is safe for a
/// terminal. Empty for anything but an approval.
pub(crate) fn approval_details(action: &ActionRequiredData) -> Vec<String> {
    let ActionRequiredData::ToolConfirmation {
        arguments,
        prompt,
        risk,
        preview,
        ..
    } = action
    else {
        return Vec::new();
    };
    let mut lines: Vec<String> = redacted(prompt.as_deref().unwrap_or_default())
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    lines.extend(risk.map(|risk| risk_line(risk).to_string()));
    match preview.as_ref().and_then(whole_command) {
        Some(command) => lines.extend(command),
        None => {
            lines.extend(preview.as_ref().and_then(preview_summary));
            lines.extend(argument_lines(arguments));
        }
    }
    lines.iter().map(|line| safe_text(line)).collect()
}

/// Whether the one-line form of a card shows its prompt's first line whole, so the details may
/// leave that line out rather than print it twice.
fn heading_is_whole(prompt: Option<&str>) -> bool {
    prompt
        .and_then(|prompt| prompt.lines().map(str::trim).find(|line| !line.is_empty()))
        .is_some_and(|line| line.chars().count() <= LINE_CHARS)
}

/// What the observer stream tells `attach` about cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CardEvent {
    /// An approval this terminal can answer.
    Approval(ApprovalCard),
    /// A question only another surface answers. Its line is already printed.
    Question,
    /// A tool call's response, carrying the call's ID. The agent's own approval card carries
    /// the same ID, so this means that card was answered, here or anywhere else.
    Responded(String),
    /// A card only another surface answers (a credential card, a Crew step). Its line is
    /// already printed; its heading names it again when it ends.
    Shown { id: String, heading: String },
    /// The daemon said a card ended (WI-17, `ActionRequiredEnded`): answered, dismissed, out of
    /// time or withdrawn, here or anywhere else. Its ID and the daemon's word for how, nothing
    /// more. The one way a card raised by a tool (a Crew change, a bridged call), whose ID is
    /// not its call's, is known to be over before the turn ends.
    Ended { id: String, ending: String },
    /// The daemon's notice that an approval's window closed before anyone answered it (its
    /// `expiredActionId`): the card's ID and the notice's own line. The same news as an end that
    /// timed out. It is the only word of the agent's own permission prompt expiring, which is
    /// never parked and so never gets an end frame; a parked card's end frame comes first and
    /// the notice follows it. `attach` prints one line for the two ([`Waiting`]).
    Expired { id: String, notice: String },
    /// The turn ended, so no card of it is still waiting.
    TurnEnded,
}

/// An approval waiting for this terminal's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalCard {
    pub(crate) id: String,
    pub(crate) read_consent: bool,
    pub(crate) details: Vec<String>,
}

/// Where the observer reader sends [`CardEvent`]s.
pub(crate) type CardSink = tokio::sync::mpsc::UnboundedSender<CardEvent>;

/// What one content block of a live message tells `attach`: a card, or a tool call's response.
fn block_event(block: &Value) -> Option<CardEvent> {
    match block.get("type").and_then(Value::as_str)? {
        "toolResponse" => block
            .get("id")
            .and_then(Value::as_str)
            .map(|id| CardEvent::Responded(id.to_string())),
        "actionRequired" => {
            let action =
                serde_json::from_value::<ActionRequiredData>(block.get("data")?.clone()).ok()?;
            match &action {
                ActionRequiredData::ToolConfirmation { id, prompt, .. } => {
                    // The card's line already shows the prompt's first line, unless it had to
                    // clip it: then the details show it whole.
                    let heading = heading_is_whole(prompt.as_deref());
                    Some(CardEvent::Approval(ApprovalCard {
                        id: id.clone(),
                        read_consent: is_read_consent(&action),
                        details: approval_details(&action)
                            .into_iter()
                            .skip(usize::from(heading))
                            .collect(),
                    }))
                }
                ActionRequiredData::Elicitation { .. } => Some(CardEvent::Question),
                ActionRequiredData::SecretRequest { id, prompt, .. } => Some(CardEvent::Shown {
                    id: id.clone(),
                    heading: first_line(prompt).unwrap_or_else(|| "A credential card".into()),
                }),
                // A changed host key's notice waits for nothing, so nothing will say it ended.
                ActionRequiredData::CrewStep { id, step }
                    if !biorouter::pending_user_action::crew_step_is_a_notice(step) =>
                {
                    Some(CardEvent::Shown {
                        id: id.clone(),
                        heading: crew_step_heading(step),
                    })
                }
                _ => None,
            }
        }
        "systemNotification" => {
            let id = block.get("expiredActionId").and_then(Value::as_str)?;
            Some(CardEvent::Expired {
                id: id.to_owned(),
                notice: format!(
                    "[notice] {}",
                    super::inline_notice(block).unwrap_or(APPROVAL_EXPIRED)
                ),
            })
        }
        _ => None,
    }
}

/// The approval card a message reports as expired: the daemon's expiry notice
/// (`pending_user_action::card_expiry_notice`), a message of inline notices one of which names
/// the card (`expiredActionId`). `None` for any message that says anything else.
pub(crate) fn expired_card_of(message: &Value) -> Option<&str> {
    let blocks = message.get("content")?.as_array()?;
    if !blocks
        .iter()
        .all(|block| block.get("type").and_then(Value::as_str) == Some("systemNotification"))
    {
        return None;
    }
    blocks
        .iter()
        .find_map(|block| block.get("expiredActionId").and_then(Value::as_str))
}

/// The approval whose expiry `frame` reports: a card's end frame that says `timed_out`, or the
/// notice that names an expired approval ([`expired_card_of`]). A parked card's expiry comes as
/// both, the end frame first; the agent's own permission prompt's comes as the notice alone.
pub(crate) fn expiry_reported_by(frame: &Value) -> Option<&str> {
    match frame.get("type").and_then(Value::as_str)? {
        "ActionRequiredEnded"
            if frame.get("ending").and_then(Value::as_str) == Some("timed_out") =>
        {
            frame.get("id").and_then(Value::as_str)
        }
        "Message" => expired_card_of(frame.get("message")?),
        _ => None,
    }
}

/// The cards whose end a terminal has already said, by ID, newest last and at most
/// [`SHOWN_LIMIT`]. The daemon says a parked approval's expiry twice, its end frame and then the
/// notice that names it, and a terminal prints one line for the two.
#[derive(Debug, Default)]
pub(crate) struct SaidEnds(VecDeque<String>);

impl SaidEnds {
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.0.iter().any(|said| said == id)
    }

    /// Note that the end of `id` was said: `false` when it already was.
    pub(crate) fn note(&mut self, id: &str) -> bool {
        if self.contains(id) {
            return false;
        }
        self.0.push_back(id.to_owned());
        while self.0.len() > SHOWN_LIMIT {
            self.0.pop_front();
        }
        true
    }
}

/// The line for an approval that stopped waiting before this terminal answered it, by the
/// daemon's word for how it ended. A window that closed is the one sentence every terminal says
/// for an expiry ([`APPROVAL_EXPIRED`]), never "answered somewhere else".
pub(crate) fn approval_closed_line(ending: &str) -> String {
    match ending {
        "answered" => "[approval closed] This request was answered somewhere else.".to_string(),
        "dismissed" => "[approval closed] This request was dismissed somewhere else.".to_string(),
        "timed_out" => format!("[approval closed] {APPROVAL_EXPIRED}"),
        _ => "[approval closed] This request is no longer waiting.".to_string(),
    }
}

/// How a card ended, in a phrase ([`CardEvent::Ended`]); the daemon's word is one of four, and
/// a word this build does not know still means the card stopped waiting.
fn ended_phrase(ending: &str) -> &'static str {
    match ending {
        "answered" => "answered",
        "dismissed" => "dismissed",
        "timed_out" => "timed out",
        _ => "no longer waiting",
    }
}

/// The line `watch` prints for a card's end (`ActionRequiredEnded`): it keeps no card, so it
/// names none. `attach` names the card instead ([`Waiting`]).
pub(crate) fn ended_frame_line(frame: &Value) -> String {
    let ending = frame
        .get("ending")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!(
        "[closed] A request in this chat is over: {}.",
        ended_phrase(ending)
    )
}

/// What one observer frame tells `attach` about cards: each live card and tool response in a
/// `Message` frame, each card's end, and the end of a turn. The join snapshot never carries a
/// card that waits, which is never stored; the cards still waiting follow it as live
/// `Message` frames.
pub(crate) fn card_events(frame: &Value) -> Vec<CardEvent> {
    match frame.get("type").and_then(Value::as_str) {
        Some("Finish" | "Error") => vec![CardEvent::TurnEnded],
        Some("ActionRequiredEnded") => frame
            .get("id")
            .and_then(Value::as_str)
            .map(|id| CardEvent::Ended {
                id: id.to_owned(),
                ending: frame
                    .get("ending")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
            .into_iter()
            .collect(),
        Some("Message") => frame
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(block_event)
            .collect(),
        _ => Vec::new(),
    }
}

/// Send `frame`'s card events to `sink`. A closed sink means `attach` has stopped reading.
pub(crate) fn forward(frame: &Value, sink: &CardSink) {
    for event in card_events(frame) {
        let _ = sink.send(event);
    }
}

/// What a line typed into `attach` is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LineUse {
    /// No approval waits: the line goes to the chat as it always did.
    Message,
    /// An approval waits and the line answers nothing: these lines are printed, and the line
    /// is not sent to the chat.
    Unanswered(Vec<String>),
    /// The line answers the first waiting approval. `next` is what to print for the one after.
    Answer {
        card: ApprovalCard,
        action: &'static str,
        next: Vec<String>,
    },
}

/// Whether an `attach` answers approvals, and, when it does not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answering {
    /// A person types at this terminal: while an approval waits, a line answers it.
    Here,
    /// `--read-only`: nothing is read from stdin.
    ReadOnly,
    /// stdin is a pipe or a file, not a person typing. Its lines still go to the chat.
    NotATerminal,
    /// This runs in an agent's own shell. Its lines still go to the chat.
    AgentShell,
}

impl Answering {
    /// How an `attach` answers, from its flag and what it runs in. An agent's shell is named
    /// first: its stdin is never the person's keyboard, whatever it is.
    pub(crate) fn for_attach(read_only: bool, stdin_is_terminal: bool, agent_shell: bool) -> Self {
        if read_only {
            Answering::ReadOnly
        } else if agent_shell {
            Answering::AgentShell
        } else if !stdin_is_terminal {
            Answering::NotATerminal
        } else {
            Answering::Here
        }
    }

    /// What is printed under a card this `attach` of the chat `session_id` won't answer.
    fn elsewhere(self, session_id: &str) -> String {
        let elsewhere = answer_elsewhere(session_id);
        match self {
            Answering::Here => format!("{elsewhere}."),
            Answering::ReadOnly => {
                format!("{elsewhere}. Or attach without --read-only to answer it here.")
            }
            Answering::NotATerminal => format!(
                "{elsewhere}. This attach isn't reading your typing, so it answers no request. What it reads still goes to the chat as a message."
            ),
            Answering::AgentShell => format!(
                "{elsewhere}. This attach runs in an agent's shell, so it answers no request. What it reads still goes to the chat as a message."
            ),
        }
    }
}

/// How many cards that `attach` cannot answer it remembers at once, to name each when it ends.
const SHOWN_LIMIT: usize = 64;

/// The approvals an `attach` is waiting to answer, oldest first. While one waits, a typed line
/// answers it and is never sent to the chat.
pub(crate) struct Waiting {
    session_id: String,
    /// Whether a line read here answers an approval ([`Answering::Here`]) or none ever does.
    answering: Answering,
    cards: VecDeque<ApprovalCard>,
    /// The cards shown here that only another surface answers, by ID and heading, oldest first,
    /// so their end can name them ([`CardEvent::Shown`]).
    shown: VecDeque<(String, String)>,
    /// The cards whose end a line here already said, so the daemon's second word for the same
    /// end (a parked approval's expiry notice, after its end frame) prints nothing.
    said: SaidEnds,
}

impl Waiting {
    pub(crate) fn new(session_id: &str, answering: Answering) -> Self {
        Self {
            session_id: session_id.to_string(),
            answering,
            cards: VecDeque::new(),
            shown: VecDeque::new(),
            said: SaidEnds::default(),
        }
    }

    /// Stop holding the approval at `at`, saying `closed`: the next line typed here is a
    /// message again, or the answer to the request after it. Says nothing for one behind the
    /// first, which was never asked.
    fn close_held(&mut self, at: usize, closed: &str) -> Vec<String> {
        self.cards.remove(at);
        if at > 0 {
            return Vec::new();
        }
        let mut lines = vec![closed.to_string()];
        if let Some(next) = self.cards.front() {
            lines.extend(self.ask(next));
        }
        lines
    }

    /// What to print for the daemon's word that the card `id` ended as `ending`. An approval
    /// held here stops being held. A card shown here that another surface answers is named
    /// again. One this terminal answered, or never showed, needs nothing.
    fn on_ended(&mut self, id: &str, ending: &str) -> Vec<String> {
        if let Some(at) = self.cards.iter().position(|held| held.id == id) {
            // The first one's close is said here. One behind it goes quietly, so the expiry
            // notice that follows a parked card's end is then its one line ([`Self::on_expired`]).
            if at == 0 {
                self.said.note(id);
            }
            return self.close_held(at, &approval_closed_line(ending));
        }
        let Some(at) = self.shown.iter().position(|(shown, _)| shown == id) else {
            return Vec::new();
        };
        let Some((_, heading)) = self.shown.remove(at) else {
            return Vec::new();
        };
        vec![safe_text(&redacted(&format!(
            "[closed] {}: {}.",
            clip(&heading, LINE_CHARS),
            ended_phrase(ending)
        )))]
    }

    /// What to print for the daemon's notice that the approval `id` expired unanswered, whose
    /// own line is `notice` ([`CardEvent::Expired`]). One line for the expiry, whichever of its
    /// words came first: the approval waiting first closes as expired, and its call's response,
    /// which follows the notice, then finds nothing to close as "answered somewhere else"; one
    /// behind it, never asked, goes with the notice's line; any other card's notice is printed
    /// once, unless an end frame already said it.
    fn on_expired(&mut self, id: &str, notice: String) -> Vec<String> {
        match self.cards.iter().position(|held| held.id == id) {
            Some(0) => return self.on_ended(id, "timed_out"),
            Some(at) => {
                self.cards.remove(at);
            }
            None if self.said.contains(id) => return Vec::new(),
            None => {}
        }
        self.said.note(id);
        vec![notice]
    }

    /// The question for `card`: its details, then which words answer it.
    fn ask(&self, card: &ApprovalCard) -> Vec<String> {
        let choices = if card.read_consent {
            read_consent_choices(&self.session_id)
        } else {
            "allow_once or deny".to_string()
        };
        let mut lines = card.details.clone();
        lines.push(format!(
            "Type {choices}, then press Enter. Your next line answers this request and is not sent to the chat."
        ));
        lines
    }

    /// What to print for `event`.
    pub(crate) fn on_event(&mut self, event: CardEvent) -> Vec<String> {
        match event {
            // The same card shown again is still one request, and one whose end was said here
            // waits for nothing.
            CardEvent::Approval(card)
                if self.cards.iter().any(|held| held.id == card.id)
                    || self.said.contains(&card.id) =>
            {
                Vec::new()
            }
            // Shown whole as anywhere, but never held: a line read here stays a message.
            CardEvent::Approval(card) if self.answering != Answering::Here => {
                let mut lines = card.details;
                lines.push(self.answering.elsewhere(&self.session_id));
                lines
            }
            CardEvent::Approval(card) => {
                self.cards.push_back(card);
                if self.cards.len() == 1 {
                    self.ask(&self.cards[0])
                } else {
                    vec!["This request is asked once you answer the one before it.".to_string()]
                }
            }
            CardEvent::Question if self.answering == Answering::ReadOnly => {
                vec![format!("{}.", answer_elsewhere(&self.session_id))]
            }
            CardEvent::Question => vec![format!(
                "{}. What you type here goes to the chat as a message.",
                answer_elsewhere(&self.session_id)
            )],
            // Answered somewhere else: the next line typed here is a message again, or the
            // answer to the request after it. One this terminal answered is already gone.
            CardEvent::Responded(id) => {
                let Some(at) = self.cards.iter().position(|held| held.id == id) else {
                    return Vec::new();
                };
                self.close_held(at, &approval_closed_line("answered"))
            }
            // Its line is already printed; it is named again only when it ends.
            CardEvent::Shown { id, heading } => {
                if !self.shown.iter().any(|(shown, _)| *shown == id) {
                    self.shown.push_back((id, heading));
                    while self.shown.len() > SHOWN_LIMIT {
                        self.shown.pop_front();
                    }
                }
                Vec::new()
            }
            CardEvent::Ended { id, ending } => self.on_ended(&id, &ending),
            CardEvent::Expired { id, notice } => self.on_expired(&id, notice),
            CardEvent::TurnEnded if self.cards.is_empty() => {
                // Each card's own end came first; a turn's end leaves none waiting.
                self.shown.clear();
                Vec::new()
            }
            CardEvent::TurnEnded => {
                self.shown.clear();
                self.cards.clear();
                vec![
                    "[approval closed] The turn ended, so its request is no longer waiting."
                        .to_string(),
                ]
            }
        }
    }

    /// What `typed` is for. Only a line typed by a person at this terminal answers anything.
    pub(crate) fn on_line(&mut self, typed: &str) -> LineUse {
        if self.answering != Answering::Here {
            return LineUse::Message;
        }
        let Some(card) = self.cards.front() else {
            return LineUse::Message;
        };
        let Some(action) = approval_answer(typed, card.read_consent) else {
            let choices = if card.read_consent {
                read_consent_choices(&self.session_id)
            } else {
                "allow_once or deny".to_string()
            };
            return LineUse::Unanswered(vec![format!(
                "Not sent to the chat. A request is waiting: type {choices}."
            )]);
        };
        let Some(card) = self.cards.pop_front() else {
            return LineUse::Message;
        };
        let next = self
            .cards
            .front()
            .map(|next| self.ask(next))
            .unwrap_or_default();
        LineUse::Answer { card, action, next }
    }
}

/// The line for the daemon's answer to an approval this terminal posted.
pub(crate) fn answer_outcome(
    code: u16,
    body: &str,
    action: &str,
    read_consent: bool,
    session_id: &str,
) -> String {
    let answer = super::json_object(body);
    let field = |name: &str| {
        answer
            .as_ref()
            .and_then(|answer| answer.get(name))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let line = match (code, field("status").as_deref()) {
        (200, Some("delivered")) if action == "deny" => "[answered] Denied.".to_string(),
        (200, Some("delivered")) if read_consent => {
            format!("[answered] {}", read_consent_allowed(session_id))
        }
        (200, Some("delivered")) => "[answered] Allowed once.".to_string(),
        (200, Some("unknown" | "already_resolved")) => {
            "[not answered] This request is no longer waiting, so nothing was approved here."
                .to_string()
        }
        (403, _) => format!(
            "[not answered] {} {ANSWER_ELSEWHERE}.",
            field("error").unwrap_or_else(|| "The daemon refused this answer.".to_string())
        ),
        (401, _) => "[not answered] The daemon refused this terminal: BIOROUTER_SERVER__SECRET_KEY does not match the daemon's.".to_string(),
        (code, _) => {
            format!("[not answered] The daemon answered HTTP {code}. {ANSWER_ELSEWHERE}.")
        }
    };
    safe_text(&line)
}

/// Post the person's `action` on approval `card_id` to the daemon on `port`, with the
/// user-action key when `auth` holds one, and answer the line to print.
pub(crate) async fn answer_approval(
    port: u16,
    session_id: &str,
    card: &ApprovalCard,
    action: &str,
    auth: &DaemonAuth,
) -> Result<String> {
    let body = serde_json::json!({
        "sessionId": session_id,
        "id": card.id,
        "principalType": "Tool",
        "action": action,
    })
    .to_string();
    let (code, answer) =
        super::post_json_to(port, "/action-required/tool-confirmation", &body, auth).await?;
    Ok(answer_outcome(
        code,
        &answer,
        action,
        card.read_consent,
        session_id,
    ))
}

/// [`answer_approval`] on its own task, so the live rendering never waits on the daemon: the
/// outcome is printed when it comes, then `next`, the question for the approval after it.
pub(crate) fn spawn_answer(
    port: u16,
    session_id: String,
    card: ApprovalCard,
    action: &'static str,
    auth: DaemonAuth,
    next: Vec<String>,
) {
    tokio::spawn(async move {
        match answer_approval(port, &session_id, &card, action, &auth).await {
            Ok(line) => println!("{line}"),
            Err(error) => println!(
                "{}",
                safe_text(&format!("[not answered] {error} {ANSWER_ELSEWHERE}."))
            ),
        }
        for line in next {
            println!("{line}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::conversation::message::{
        ActionRequired, CrewSignInTarget, Message, MessageContent, SecretDestination,
    };

    const CANARY: &str = "CANARY-7f3a";

    fn frame_of(action: ActionRequiredData) -> Value {
        let message =
            Message::assistant().with_content(MessageContent::ActionRequired(ActionRequired {
                data: action,
            }));
        serde_json::json!({ "type": "Message", "message": message })
    }

    fn confirmation(tool: &str, arguments: Value, prompt: Option<&str>) -> ActionRequiredData {
        ActionRequiredData::ToolConfirmation {
            id: "card-1".into(),
            tool_name: tool.into(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
            prompt: prompt.map(str::to_string),
            risk: None,
            preview: None,
        }
    }

    fn read_consent() -> ActionRequiredData {
        confirmation(
            "crew__read",
            serde_json::json!({
                "action": "read as you",
                "workspace": "lab",
                "server": "hpc.example.edu",
                "connection": "lab",
            }),
            Some("Let this chat read lab as you\nIt can then read what you see in lab."),
        )
    }

    fn text_of(action: ActionRequiredData) -> String {
        super::super::render_frame(&frame_of(action)).expect("a card is printed")
    }

    #[test]
    fn every_card_is_one_line_and_no_credential_argument_is_printed() {
        let approval = text_of(confirmation(
            "developer__shell",
            serde_json::json!({
                "command": "ls",
                "password": CANARY,
                "nested": { "api_key": CANARY },
            }),
            None,
        ));
        assert_eq!(approval, "[approval] developer__shell");

        let change = text_of(confirmation(
            "crew__change",
            serde_json::json!({ "action": "channels_create", "token": CANARY }),
            Some("Create the channel #attach-test in lab\nIt is public to the workspace."),
        ));
        assert_eq!(
            change,
            "[approval] crew__change: Create the channel #attach-test in lab"
        );

        let question = text_of(ActionRequiredData::Elicitation {
            id: "q-1".into(),
            message: "Which dataset?\nPick one.".into(),
            requested_schema: serde_json::json!({
                "type": "object",
                "properties": { "password": { "type": "string", "default": CANARY } }
            }),
        });
        assert_eq!(question, "[question] Which dataset?");

        let step = text_of(ActionRequiredData::CrewStep {
            id: "step-1".into(),
            step: CrewStepRequest::SignIn {
                target: CrewSignInTarget::Setup {
                    job_id: "job-1".into(),
                },
                server: "hpc.example.edu".into(),
                login: "you@hpc.example.edu".into(),
            },
        });
        assert_eq!(
            step,
            "[crew] Sign in to you@hpc.example.edu. Answer it in the desktop app, never by typing here."
        );

        let credentials = text_of(ActionRequiredData::SecretRequest {
            id: "s-1".into(),
            prompt: "Connect the extension".into(),
            keys: vec![SecretKeyRequest {
                key: "EXAMPLE_API_KEY".into(),
                label: "API key".into(),
                description: None,
                required: true,
            }],
            destination: SecretDestination::Keyring,
        });
        assert!(
            credentials.starts_with("[credentials] Connect the extension (EXAMPLE_API_KEY)."),
            "{credentials}"
        );

        for line in [approval, change, question, step, credentials] {
            assert!(!line.contains('\n'), "one line each: {line}");
            assert!(!line.contains(CANARY), "{line}");
            assert!(!line.contains(['\u{2014}', '\u{2013}']), "{line}");
        }
    }

    #[test]
    fn an_answered_question_prints_nothing_of_what_the_person_typed() {
        let response = frame_of(ActionRequiredData::ElicitationResponse {
            id: "q-1".into(),
            user_data: serde_json::json!({ "answer": CANARY }),
        });
        assert_eq!(super::super::render_frame(&response), None);
    }

    #[test]
    fn a_card_this_build_does_not_know_still_says_something_is_waiting() {
        let line = card_line(&serde_json::json!({ "actionType": "somethingNew", "id": "x" }))
            .expect("a card");
        assert_eq!(
            line,
            "[waiting] This chat is waiting for your answer. Answer it in the desktop app."
        );
    }

    #[test]
    fn a_crew_step_names_the_daemons_values_for_each_kind() {
        let vault = card_line(
            &serde_json::to_value(ActionRequiredData::CrewStep {
                id: "v".into(),
                step: CrewStepRequest::Vault {
                    action: CrewVaultAction::Unlock,
                },
            })
            .unwrap(),
        )
        .unwrap();
        assert!(vault.starts_with("[crew] Unlock Crew's vault. "), "{vault}");
        let changed = card_line(
            &serde_json::to_value(ActionRequiredData::CrewStep {
                id: "h".into(),
                step: CrewStepRequest::HostKey {
                    host: "hpc.example.edu".into(),
                    port: 22,
                    login: "you".into(),
                    offered: Some("SHA256:abc".into()),
                    known: Some("SHA256:def".into()),
                    changed: true,
                },
            })
            .unwrap(),
        )
        .unwrap();
        assert!(
            changed.starts_with("[crew] hpc.example.edu's identity changed. "),
            "{changed}"
        );
        assert!(
            !changed.contains("SHA256:"),
            "a line names no key: {changed}"
        );
        // WI-17: a changed key's card is a notice kept in the chat, so its line asks for no
        // answer anywhere.
        assert!(changed.contains("IT team confirms the change"), "{changed}");
        assert!(!changed.contains("Answer it"), "{changed}");
        let unknown = card_line(
            &serde_json::to_value(ActionRequiredData::CrewStep {
                id: "u".into(),
                step: CrewStepRequest::HostKey {
                    host: "hpc.example.edu".into(),
                    port: 22,
                    login: "you".into(),
                    offered: Some("SHA256:abc".into()),
                    known: None,
                    changed: false,
                },
            })
            .unwrap(),
        )
        .unwrap();
        assert!(
            unknown.starts_with("[crew] Can't verify hpc.example.edu yet. "),
            "{unknown}"
        );
    }

    #[test]
    fn the_details_hide_credentials_and_redact_a_command() {
        let details = approval_details(&confirmation(
            "crew__change",
            serde_json::json!({
                "action": "channels_create",
                "name": "attach-test",
                "client_secret": CANARY,
                "headers": { "Authorization": CANARY },
            }),
            Some("Create the channel #attach-test in lab\nIt is public to the workspace."),
        ));
        assert_eq!(details[0], "Create the channel #attach-test in lab");
        assert_eq!(details[1], "It is public to the workspace.");
        assert_eq!(details[2], "Arguments: {", "{details:?}");
        let arguments = details[2..].join("\n");
        assert!(arguments.contains("attach-test"), "{arguments}");
        assert!(!arguments.contains(CANARY), "{arguments}");

        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n-----END OPENSSH PRIVATE KEY-----";
        let shell = ActionRequiredData::ToolConfirmation {
            id: "card-2".into(),
            tool_name: "developer__shell".into(),
            arguments: Map::new(),
            prompt: None,
            risk: Some(ToolRisk::High),
            preview: Some(ToolPreview::Shell {
                command: format!("printf '%s' '{pem}'"),
                truncated: false,
            }),
        };
        let details = approval_details(&shell);
        assert_eq!(details[0], "Risk: destructive");
        assert!(details[1].starts_with("Command: "), "{details:?}");
        assert!(
            !details.join("\n").contains("b3BlbnNzaC1rZXkt"),
            "{details:?}"
        );
    }

    /// AO-12: "the payload is shown whole". `attach` posts an allow with the person's proof, so
    /// what it shows first is every line of the prompt and every character of the arguments,
    /// however long; a preview the daemon clipped to fit a card is never what is shown.
    #[test]
    fn an_approval_is_shown_whole_before_it_is_answered() {
        let message = format!("{}END-OF-MESSAGE", "word ".repeat(1_200));
        let long_line = format!("Post as you in #general: {}", "x".repeat(400));
        let prompt = std::iter::once(long_line.clone())
            .chain((1..=60).map(|n| format!("Line {n} of the card")))
            .collect::<Vec<_>>()
            .join("\n");
        let mut card = confirmation(
            "crew__change",
            serde_json::json!({ "action": "send", "channel": "general", "text": message }),
            Some(&prompt),
        );
        if let ActionRequiredData::ToolConfirmation {
            arguments, preview, ..
        } = &mut card
        {
            *preview = Some(ToolPreview::arguments_within(arguments, 300 * 1024));
        }
        let shown = approval_details(&card).join("\n");
        assert!(shown.contains(&message), "the whole message is shown");
        assert!(
            shown.contains(&long_line),
            "a long prompt line is shown whole"
        );
        assert!(shown.contains("Line 60 of the card"), "every prompt line");
        assert!(!shown.contains('…'), "nothing is clipped: {shown}");

        // The card's one-line form clips that first line, so the details keep it rather than
        // leave it out as already shown.
        let events = card_events(&frame_of(card));
        let [CardEvent::Approval(approval)] = events.as_slice() else {
            panic!("one approval: {events:?}");
        };
        assert_eq!(approval.details[0], long_line);

        // A command the daemon clipped to fit the card is shown from the call's own arguments.
        let command = format!("echo {}", "y".repeat(5_000));
        let clipped = ActionRequiredData::ToolConfirmation {
            id: "card-4".into(),
            tool_name: "developer__shell".into(),
            arguments: serde_json::json!({ "command": command })
                .as_object()
                .cloned()
                .unwrap(),
            prompt: None,
            risk: None,
            preview: Some(ToolPreview::Shell {
                command: command.chars().take(4_000).collect(),
                truncated: true,
            }),
        };
        assert!(approval_details(&clipped).join("\n").contains(&command));

        // A new file is shown from the arguments too: the preview's copy of it is clipped.
        let body = "line\n".repeat(3_000);
        let write = ActionRequiredData::ToolConfirmation {
            id: "card-5".into(),
            tool_name: "developer__text_editor".into(),
            arguments:
                serde_json::json!({ "command": "write", "path": "notes.md", "file_text": body })
                    .as_object()
                    .cloned()
                    .unwrap(),
            prompt: None,
            risk: None,
            preview: Some(ToolPreview::FileWrite {
                path: "notes.md".into(),
                content: body.chars().take(8_000).collect(),
                line_count: 3_000,
                truncated: true,
            }),
        };
        let shown = approval_details(&write);
        assert_eq!(shown[0], "Writes notes.md: 3000 lines");
        let encoded = serde_json::to_string(&body).unwrap();
        assert!(
            shown.join("\n").contains(&encoded),
            "the whole file is in the arguments shown"
        );
    }

    #[test]
    fn a_shared_daemon_chat_shows_the_card_whole_but_never_a_credential() {
        let mut card = confirmation(
            "extensionmanager__install",
            serde_json::json!({ "name": "lab-tools", "env": { "LAB_API_TOKEN": CANARY } }),
            Some("Install lab-tools"),
        );
        if let ActionRequiredData::ToolConfirmation {
            arguments, preview, ..
        } = &mut card
        {
            *preview = Some(ToolPreview::arguments_within(arguments, 4096));
        }
        let shown = shown_card_json(&card);
        assert!(!shown.contains(CANARY), "{shown}");
        assert!(shown.contains("lab-tools"), "{shown}");
        // The key is still named and its value hidden.
        assert!(
            shown.contains("\"LAB_API_TOKEN\": \"[REDACTED:secret-value]\""),
            "{shown}"
        );
        assert!(!shown.contains("\"preview\""), "{shown}");
        assert_eq!(
            hide_credentials(&serde_json::json!({
                "folder": "data",
                "nested": [{ "Passphrase": CANARY, "label": "lab" }],
            })),
            serde_json::json!({
                "folder": "data",
                "nested": [{ "Passphrase": HIDDEN_VALUE, "label": "lab" }],
            })
        );

        let edit = ActionRequiredData::ToolConfirmation {
            id: "card-3".into(),
            tool_name: "developer__text_editor".into(),
            arguments: Map::new(),
            prompt: None,
            risk: None,
            preview: Some(ToolPreview::FileWrite {
                path: "notes.md".into(),
                content: "# Notes".into(),
                line_count: 1,
                truncated: false,
            }),
        };
        assert!(
            shown_card_json(&edit).contains("# Notes"),
            "a new file is still shown"
        );
    }

    /// A value hidden by its credential-like key reads the same whatever the key's spelling: an
    /// environment-style name (which the secret-output redactor also knows) and any other used
    /// to show two different words, because the redactor rewrote this terminal's own marker.
    /// Both now show the redactor's word, which is the desktop tool card's, in a shared-daemon
    /// chat and in `attach` alike.
    #[test]
    fn every_hidden_value_reads_the_same_whatever_its_key() {
        let card = confirmation(
            "extensionmanager__install",
            serde_json::json!({
                "name": "lab-tools",
                "env": { "LAB_API_TOKEN": CANARY },
                "passphrase": CANARY,
                "headers": { "Authorization": CANARY },
            }),
            Some("Install lab-tools"),
        );
        let hidden = |key: &str| format!("\"{key}\": \"{HIDDEN_VALUE}\"");
        for (surface, shown) in [
            ("shared-daemon chat", shown_card_json(&card)),
            ("attach", approval_details(&card).join("\n")),
        ] {
            assert!(!shown.contains(CANARY), "{surface}: {shown}");
            assert!(!shown.contains("(hidden)"), "{surface}: {shown}");
            for key in ["LAB_API_TOKEN", "passphrase", "Authorization"] {
                assert!(shown.contains(&hidden(key)), "{surface}, {key}: {shown}");
            }
        }
        assert_eq!(
            biorouter::guardrails::secret_output::redact_text(&hidden("LAB_API_TOKEN")),
            None,
            "the redactor leaves the marker as it is"
        );
    }

    #[test]
    fn the_read_consent_is_offered_as_lasting_until_revoked() {
        let consent = read_consent();
        assert!(is_read_consent(&consent));
        assert!(!is_read_consent(&confirmation(
            "crew__read",
            serde_json::json!({ "action": "setup_check" }),
            Some("Check hpc.example.edu"),
        )));
        assert!(!is_read_consent(&confirmation(
            "crew__change",
            serde_json::json!({ "action": "read as you" }),
            None,
        )));

        let choices = read_consent_choices("20261106_4");
        assert_eq!(
            choices,
            "allow (until you revoke it with biorouter crew grants revoke 20261106_4 --reads) or deny"
        );
        assert!(!choices.contains("once"), "{choices}");
        assert_eq!(approval_answer("allow", true), Some("allow_once"));
        assert_eq!(approval_answer(" Deny ", true), Some("deny"));
        assert_eq!(
            approval_answer("allow_once", true),
            None,
            "a lasting consent is never allowed as once"
        );
        assert_eq!(approval_answer("allow_once", false), Some("allow_once"));
        assert_eq!(approval_answer("allow", false), None);
        assert_eq!(approval_answer("yes", false), None);

        let mut waiting = Waiting::new("20261106_4", Answering::Here);
        let events = card_events(&frame_of(consent));
        let [CardEvent::Approval(card)] = events.as_slice() else {
            panic!("one approval: {events:?}");
        };
        assert!(card.read_consent);
        let asked = waiting.on_event(CardEvent::Approval(card.clone()));
        assert_eq!(
            asked.last().unwrap(),
            "Type allow (until you revoke it with biorouter crew grants revoke 20261106_4 --reads) or deny, then press Enter. Your next line answers this request and is not sent to the chat."
        );
        assert_eq!(
            answer_outcome(200, r#"{"status":"delivered"}"#, "allow_once", true, "20261106_4"),
            "[answered] Allowed. This chat may read the workspace as you until you revoke it with biorouter crew grants revoke 20261106_4 --reads."
        );
        for line in asked {
            assert!(!line.contains(['\u{2014}', '\u{2013}']), "{line}");
        }
    }

    #[test]
    fn while_an_approval_waits_a_typed_line_answers_it_and_never_reaches_the_chat() {
        let mut waiting = Waiting::new("s", Answering::Here);
        assert_eq!(waiting.on_line("hello"), LineUse::Message);
        let card = |id: &str| ApprovalCard {
            id: id.into(),
            read_consent: false,
            details: vec![format!("Do {id}")],
        };
        assert_eq!(
            waiting.on_event(CardEvent::Approval(card("a"))),
            vec![
                "Do a".to_string(),
                "Type allow_once or deny, then press Enter. Your next line answers this request and is not sent to the chat.".to_string()
            ]
        );
        assert_eq!(
            waiting.on_event(CardEvent::Approval(card("b"))),
            vec!["This request is asked once you answer the one before it.".to_string()]
        );
        assert_eq!(
            waiting.on_line("please do it"),
            LineUse::Unanswered(vec![
                "Not sent to the chat. A request is waiting: type allow_once or deny.".to_string()
            ])
        );
        let LineUse::Answer {
            card: a,
            action,
            next,
        } = waiting.on_line("allow_once")
        else {
            panic!("the line answers the first card");
        };
        assert_eq!((a.id.as_str(), action), ("a", "allow_once"));
        assert_eq!(next.first().map(String::as_str), Some("Do b"));
        assert!(matches!(waiting.on_line("deny"), LineUse::Answer { .. }));
        assert_eq!(waiting.on_line("back to chatting"), LineUse::Message);

        waiting.on_event(CardEvent::Approval(card("c")));
        assert_eq!(
            waiting.on_event(CardEvent::TurnEnded),
            vec![
                "[approval closed] The turn ended, so its request is no longer waiting."
                    .to_string()
            ]
        );
        assert_eq!(waiting.on_line("next message"), LineUse::Message);
    }

    /// The agent's own approval card carries its tool call's ID, so the call's response says it
    /// was answered (on the desktop, say): `attach` stops holding the next line for it, and the
    /// same card shown twice is still one request.
    #[test]
    fn a_request_answered_somewhere_else_stops_holding_the_next_line() {
        let mut waiting = Waiting::new("s", Answering::Here);
        let card = |id: &str| ApprovalCard {
            id: id.into(),
            read_consent: false,
            details: vec![format!("Do {id}")],
        };
        assert_eq!(waiting.on_event(CardEvent::Approval(card("a"))).len(), 2);
        assert_eq!(
            waiting.on_event(CardEvent::Approval(card("a"))),
            Vec::<String>::new(),
            "the same card again asks nothing new"
        );
        waiting.on_event(CardEvent::Approval(card("b")));

        let response = serde_json::json!({
            "type": "Message",
            "message": {
                "role": "user",
                "content": [{
                    "type": "toolResponse",
                    "id": "a",
                    "toolResult": { "status": "success", "value": { "content": [] } }
                }]
            }
        });
        let events = card_events(&response);
        assert_eq!(events, vec![CardEvent::Responded("a".into())]);
        let lines = waiting.on_event(events[0].clone());
        assert_eq!(
            lines.first().map(String::as_str),
            Some("[approval closed] This request was answered somewhere else.")
        );
        assert_eq!(lines.get(1).map(String::as_str), Some("Do b"), "{lines:?}");
        assert_eq!(
            waiting.on_event(CardEvent::Responded("unrelated".into())),
            Vec::<String>::new()
        );
        let LineUse::Answer { card: b, .. } = waiting.on_line("deny") else {
            panic!("the next line answers b, not a");
        };
        assert_eq!(b.id, "b");
        assert_eq!(waiting.on_line("back to chatting"), LineUse::Message);
    }

    #[test]
    fn a_read_only_attach_says_where_to_answer_and_holds_nothing() {
        let mut waiting = Waiting::new("s", Answering::ReadOnly);
        let lines = waiting.on_event(CardEvent::Approval(ApprovalCard {
            id: "a".into(),
            read_consent: false,
            details: Vec::new(),
        }));
        assert_eq!(
            lines,
            vec!["Answer it in the desktop app, or open this chat in a terminal with biorouter session --shared-daemon --resume --session-id s. Or attach without --read-only to answer it here.".to_string()]
        );
        assert_eq!(waiting.on_line("anything"), LineUse::Message);
    }

    /// WI-17 (4): where a card is answered names only what can answer it. A terminal chat
    /// answers only its own conversation's cards, so a line that has the chat's ID names the
    /// one terminal chat that can, and a line without it names the desktop app alone.
    #[test]
    fn where_to_answer_never_promises_a_terminal_that_cannot() {
        let line =
            card_line(&serde_json::json!({ "actionType": "somethingNew", "id": "x" })).unwrap();
        assert!(!line.contains("--shared-daemon"), "{line}");
        let mut waiting = Waiting::new("chat-7", Answering::Here);
        let lines = waiting.on_event(CardEvent::Question);
        assert!(
            lines[0].contains("biorouter session --shared-daemon --resume --session-id chat-7"),
            "{lines:?}"
        );
        assert!(!lines[0].contains("a biorouter session --shared-daemon chat"));
    }

    /// WI-17 (c) and (d) in `attach`: an approval the daemon says ended stops holding the next
    /// line at once, whatever its ID (a card a tool raised, whose ID is not its call's, used to
    /// hold every line until the turn ended); a Crew step or credential card shown here is named
    /// again when it ends; and an end for a card this terminal answered or never showed says
    /// nothing.
    #[test]
    fn an_end_stops_the_wait_and_names_what_ended() {
        let mut waiting = Waiting::new("s", Answering::Here);
        let card = |id: &str| ApprovalCard {
            id: id.into(),
            read_consent: false,
            details: vec![format!("Details of {id}")],
        };
        waiting.on_event(CardEvent::Approval(card("a")));
        waiting.on_event(CardEvent::Approval(card("b")));
        let lines = waiting.on_event(CardEvent::Ended {
            id: "a".into(),
            ending: "dismissed".into(),
        });
        assert_eq!(
            lines[0],
            "[approval closed] This request was dismissed somewhere else."
        );
        assert!(lines.contains(&"Details of b".to_string()), "{lines:?}");
        // The one behind the first, ended before it was asked, goes quietly.
        let _ = waiting.on_event(CardEvent::Approval(card("c")));
        assert!(waiting
            .on_event(CardEvent::Ended {
                id: "c".into(),
                ending: "timed_out".into(),
            })
            .is_empty());
        let lines = waiting.on_event(CardEvent::Ended {
            id: "b".into(),
            ending: "a word from a newer daemon".into(),
        });
        assert_eq!(
            lines,
            ["[approval closed] This request is no longer waiting."]
        );
        assert_eq!(waiting.on_line("my next message"), LineUse::Message);

        waiting.on_event(CardEvent::Shown {
            id: "step".into(),
            heading: "Sign in to you@hpc.example.edu".into(),
        });
        assert_eq!(
            waiting.on_event(CardEvent::Ended {
                id: "step".into(),
                ending: "withdrawn".into(),
            }),
            ["[closed] Sign in to you@hpc.example.edu: no longer waiting."]
        );
        assert!(waiting
            .on_event(CardEvent::Ended {
                id: "never-shown".into(),
                ending: "answered".into(),
            })
            .is_empty());
    }

    /// The daemon's expiry notice for approval `id`, as the observer stream carries it, built
    /// by the daemon's own constructor so this reader and that writer cannot drift apart.
    fn expiry_frame(id: &str) -> Value {
        let notice = biorouter::pending_user_action::card_expiry_notice(
            id,
            "The permission prompt for developer__shell",
        );
        serde_json::json!({ "type": "Message", "seq": 7, "message": notice })
    }

    fn held(id: &str) -> ApprovalCard {
        ApprovalCard {
            id: id.into(),
            read_consent: false,
            details: vec![format!("Do {id}")],
        }
    }

    /// The agent's own permission prompt is never parked, so its expiry reaches `attach` only
    /// as the daemon's notice, and the call's response follows it. The approval closes as
    /// expired in one line, never as "answered somewhere else", and the next line typed here
    /// answers the request after it.
    #[test]
    fn the_agents_own_prompt_expires_in_one_line_never_as_answered_elsewhere() {
        let mut waiting = Waiting::new("s", Answering::Here);
        waiting.on_event(CardEvent::Approval(held("call-1")));
        waiting.on_event(CardEvent::Approval(held("call-2")));

        let events = card_events(&expiry_frame("call-1"));
        assert!(
            matches!(events.as_slice(), [CardEvent::Expired { id, .. }] if id == "call-1"),
            "{events:?}"
        );
        let lines = waiting.on_event(events[0].clone());
        assert_eq!(lines[0], format!("[approval closed] {APPROVAL_EXPIRED}"));
        assert!(lines.contains(&"Do call-2".to_string()), "{lines:?}");
        assert!(
            !lines.iter().any(|line| line.contains("somewhere else")),
            "{lines:?}"
        );

        // The call's response, then a replay of the notice, add nothing.
        assert!(waiting
            .on_event(CardEvent::Responded("call-1".into()))
            .is_empty());
        assert!(waiting.on_event(events[0].clone()).is_empty());
        let LineUse::Answer { card, .. } = waiting.on_line("deny") else {
            panic!("the next line answers the request after the expired one");
        };
        assert_eq!(card.id, "call-2");
    }

    /// A parked card's expiry reaches `attach` twice, its `timed_out` end frame and then the
    /// notice that names it: one line. Held first, the end frame closes it as expired and the
    /// notice says nothing; held behind the first, never asked, it goes quietly and the notice
    /// is its line; not held at all (a read-only attach), the notice is its line, once.
    #[test]
    fn a_parked_cards_expiry_is_one_line_in_attach() {
        let ended = |id: &str| CardEvent::Ended {
            id: id.into(),
            ending: "timed_out".into(),
        };
        let expired = |id: &str| card_events(&expiry_frame(id)).remove(0);
        let notice = "[notice] The permission prompt for developer__shell expired before it was answered. The tool was not run. Ask again to retry.";

        let mut waiting = Waiting::new("s", Answering::Here);
        waiting.on_event(CardEvent::Approval(held("a")));
        waiting.on_event(CardEvent::Approval(held("b")));
        waiting.on_event(CardEvent::Approval(held("c")));
        let mut printed = waiting.on_event(ended("a"));
        printed.extend(waiting.on_event(expired("a")));
        assert_eq!(
            printed,
            [
                format!("[approval closed] {APPROVAL_EXPIRED}"),
                "Do b".to_string(),
                "Type allow_once or deny, then press Enter. Your next line answers this request and is not sent to the chat.".to_string(),
            ]
        );
        let mut printed = waiting.on_event(ended("c"));
        printed.extend(waiting.on_event(expired("c")));
        assert_eq!(printed, [notice]);

        let mut read_only = Waiting::new("s", Answering::ReadOnly);
        read_only.on_event(CardEvent::Approval(held("d")));
        let mut printed = read_only.on_event(ended("d"));
        printed.extend(read_only.on_event(expired("d")));
        printed.extend(read_only.on_event(expired("d")));
        assert_eq!(printed, [notice]);
    }

    /// The frames that carry a card's life reach `attach` as card events: a Crew step and a
    /// credential card as shown, a changed host key's notice as nothing that will end, and the
    /// daemon's end frame as its ID and ending. `watch` prints the end without naming the card.
    #[test]
    fn the_end_frame_and_the_cards_that_end_reach_attach() {
        let step = frame_of(ActionRequiredData::CrewStep {
            id: "step-1".into(),
            step: CrewStepRequest::Vault {
                action: CrewVaultAction::Unlock,
            },
        });
        assert_eq!(
            card_events(&step),
            [CardEvent::Shown {
                id: "step-1".into(),
                heading: "Unlock Crew's vault".into(),
            }]
        );
        let notice = frame_of(ActionRequiredData::CrewStep {
            id: "notice-1".into(),
            step: CrewStepRequest::HostKey {
                host: "hpc.example.edu".into(),
                port: 22,
                login: "you".into(),
                offered: None,
                known: None,
                changed: true,
            },
        });
        assert!(card_events(&notice).is_empty());
        let end = serde_json::json!({
            "type": "ActionRequiredEnded", "id": "step-1", "ending": "dismissed", "seq": 4
        });
        assert_eq!(
            card_events(&end),
            [CardEvent::Ended {
                id: "step-1".into(),
                ending: "dismissed".into(),
            }]
        );
        assert_eq!(
            ended_frame_line(&end),
            "[closed] A request in this chat is over: dismissed."
        );
    }

    /// Accident prevention, as D5 is for `biorouter crew`: `printf 'allow_once\n' | biorouter
    /// session attach <id>`, or the same from an agent's own shell, prints the card and the
    /// desktop's name and approves nothing. The line still reaches the chat as a message, as
    /// any steer from a pipe does.
    #[test]
    fn a_pipe_or_an_agents_shell_never_answers_an_approval() {
        assert_eq!(Answering::for_attach(false, true, false), Answering::Here);
        assert_eq!(
            Answering::for_attach(false, false, false),
            Answering::NotATerminal
        );
        assert_eq!(
            Answering::for_attach(false, true, true),
            Answering::AgentShell,
            "an agent's shell never answers, even with a terminal"
        );
        assert_eq!(Answering::for_attach(true, true, true), Answering::ReadOnly);

        for (answering, reason) in [
            (Answering::NotATerminal, "isn't reading your typing"),
            (Answering::AgentShell, "runs in an agent's shell"),
        ] {
            let mut waiting = Waiting::new("s", answering);
            let lines = waiting.on_event(CardEvent::Approval(ApprovalCard {
                id: "a".into(),
                read_consent: false,
                details: vec!["Do a".into()],
            }));
            assert_eq!(lines[0], "Do a", "the card is still shown whole");
            let pointer = lines.last().unwrap();
            assert!(
                pointer.starts_with("Answer it in the desktop app"),
                "{pointer}"
            );
            assert!(pointer.contains(reason), "{pointer}");
            assert!(!pointer.contains(['\u{2014}', '\u{2013}']), "{pointer}");
            assert_eq!(
                waiting.on_line("allow_once"),
                LineUse::Message,
                "nothing is answered from here"
            );
            assert_eq!(
                waiting.on_event(CardEvent::TurnEnded),
                Vec::<String>::new(),
                "nothing was held"
            );
        }
    }

    #[test]
    fn the_daemons_answer_is_said_in_plain_words() {
        assert_eq!(
            answer_outcome(200, r#"{"status":"delivered"}"#, "allow_once", false, "s"),
            "[answered] Allowed once."
        );
        assert_eq!(
            answer_outcome(200, r#"{"status":"delivered"}"#, "deny", true, "s"),
            "[answered] Denied."
        );
        assert_eq!(
            answer_outcome(
                200,
                r#"{"status":"already_resolved"}"#,
                "allow_once",
                false,
                "s"
            ),
            "[not answered] This request is no longer waiting, so nothing was approved here."
        );
        assert_eq!(
            answer_outcome(
                403,
                r#"{"status":"refused","reason":"unproven","error":"The person decides this."}"#,
                "allow_once",
                false,
                "s"
            ),
            "[not answered] The person decides this. Answer it in the desktop app."
        );
    }
}
