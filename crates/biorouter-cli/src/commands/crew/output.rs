//! Output for `biorouter crew`.
//!
//! `--output-format json` and `stream-json` print the daemon's values as they are, made
//! terminal-safe and nothing else, so scripts keep every ID. Text output is for people:
//! type-specific formatters name people, teams and channels, and machine IDs appear only when
//! [`HumanOptions::show_ids`] asks for them ("Machine IDs stay internal" in
//! `docs/crew/design/naming-design.md`).
//!
//! A person renders as `"Display name" (@username)`. The display name is text a colleague
//! chose, so it is quoted and escaped, and a name that is not ASCII is wrapped in Unicode
//! isolates (U+2068 … U+2069): right-to-left text inside it cannot reorder the `@username` that
//! follows. ASCII holds no right-to-left text, so it is printed bare, and a copied name holds no
//! invisible characters. When the display name equals the username, `@username` alone is
//! printed, so the username is on every line either way.
//!
//! Names come from a [`Directory`]: the value being printed (a snapshot, or a message page's
//! `people` and `channel_names` maps) plus whatever the caller already knows, such as the
//! workspace snapshot fetched for a history page. An ID the directory cannot name prints as
//! "Unknown member", "this team" or "this channel", never as the ID.

use super::args::OutputFormat;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone, Utc};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::Path;

const MAX_INPUT: usize = 1_048_576;
const UNKNOWN_MEMBER: &str = "Unknown member";
const FORMER_MEMBER: &str = "Former member";

pub fn stream_format(format: OutputFormat) -> OutputFormat {
    match format {
        OutputFormat::Text => OutputFormat::Text,
        OutputFormat::Json | OutputFormat::StreamJson => OutputFormat::StreamJson,
    }
}

fn terminal_control(ch: char) -> bool {
    ch.is_control() || biorouter::utils::is_invisible_formatting(ch)
}

/// Whether a zero-width joiner or non-joiner between `before` and `after` is part of the text
/// a person sees (M14), so it is printed as itself rather than as `\u{200d}`.
///
/// Emoji sequences are joined with U+200D (👩🏽‍🔬, 👨‍👩‍👧‍👦, 🏳️‍🌈), and Persian, Arabic and
/// the Indic scripts use U+200C and U+200D between letters and marks (می‌خواهم, क्‍ष). Printed
/// as escape text, every one of them broke. A joiner beside ASCII, or beside whitespace, a
/// control or another invisible character, stays escaped: there it is invisible and joins
/// nothing, and `crew\u{200d}_alice` would read as `crew_alice`. A username is ASCII, so no
/// joiner the rule keeps can touch one.
///
/// Kept here rather than by narrowing `biorouter::utils::is_invisible_formatting`, which prompt
/// labels rely on.
fn joins_visible_text(before: Option<char>, after: Option<char>) -> bool {
    let visible = |ch: Option<char>| {
        ch.is_some_and(|ch| !ch.is_ascii() && !ch.is_whitespace() && !terminal_control(ch))
    };
    visible(before) && visible(after)
}

pub fn safe_text(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    for (index, &ch) in chars.iter().enumerate() {
        let joiner = matches!(ch, '\u{200c}' | '\u{200d}');
        let before = index.checked_sub(1).map(|at| chars[at]);
        let after = chars.get(index + 1).copied();
        if terminal_control(ch) && !(joiner && joins_visible_text(before, after)) {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
    }
    out
}

/// [`safe_text`] for text the CLI already made safe once and may have isolated: a balanced
/// U+2068 … U+2069 pair passes through, and everything else, any other isolate included, is
/// escaped as [`safe_text`] escapes it.
///
/// The CLI wraps a display name in that pair (`quoted_name`, `display_text`) so right-to-left
/// text in it cannot reorder the words around it. Such a name reaches an error sentence, and
/// escaping the sentence again printed the pair as the literal text `\u{2068}` (CLI-4). A
/// balanced pair can only keep what is inside it from reordering what is outside, so passing
/// it through gives a hostile name nothing: every raw isolate inside a name was escaped when
/// the name was made safe, and one that is not part of a pair is escaped here.
///
/// The text between pairs is made safe as a whole, not a character at a time, so a joiner in
/// it keeps its neighbours ([`safe_text`]).
pub fn safe_text_keeping_isolates(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut plain = String::new();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\u{2068}' {
            let rest = &chars[index + 1..];
            let next = rest
                .iter()
                .position(|c| matches!(c, '\u{2066}'..='\u{2069}'));
            if let Some(end) = next.filter(|&end| rest[end] == '\u{2069}') {
                out.push_str(&safe_text(&plain));
                plain.clear();
                out.push(ch);
                out.push_str(&safe_text(&rest[..end].iter().collect::<String>()));
                out.push('\u{2069}');
                index += end + 2;
                continue;
            }
        }
        plain.push(ch);
        index += 1;
    }
    out.push_str(&safe_text(&plain));
    out
}

/// Print `value` with the default text rendering: no IDs, names from the value alone.
pub fn emit(value: &Value, format: OutputFormat) -> Result<()> {
    emit_with(value, format, &HumanOptions::default())
}

/// Print `value`. JSON formats ignore `options` entirely.
pub fn emit_with(value: &Value, format: OutputFormat, options: &HumanOptions) -> Result<()> {
    let output = format_output(value, format, options)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{output}")?;
    stdout.flush()?;
    Ok(())
}

/// `value` as [`emit_with`] prints it, without the line break.
pub fn formatted(value: &Value, format: OutputFormat, options: &HumanOptions) -> Result<String> {
    format_output(value, format, options)
}

fn format_output(value: &Value, format: OutputFormat, options: &HumanOptions) -> Result<String> {
    Ok(match format {
        OutputFormat::Json => json_terminal_safe(serde_json::to_string_pretty(value)?),
        OutputFormat::StreamJson => json_terminal_safe(serde_json::to_string(value)?),
        OutputFormat::Text => render_text(value, options),
    })
}

fn json_terminal_safe(value: String) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if terminal_control(ch) && ch != '\n' && ch != '\r' && ch != '\t' {
            let mut units = [0; 2];
            for unit in ch.encode_utf16(&mut units) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// "Retry safely with --request-id …": the only place a request ID belongs in text output,
/// after a mutation whose outcome is uncertain.
#[allow(dead_code)] // Called by the command layer's error path.
pub fn retry_hint(request_id: &str) -> String {
    format!("Retry safely with --request-id {}", safe_text(request_id))
}

/// Broker refusal words shared with the desktop, byte for byte: `refusalCopy` and
/// `letInCopy.alreadyApproved` in `ui/desktop/src/components/crew/dialogs/copy.ts`. A test here
/// reads that file, so the two cannot drift apart.
const DEVICE_CONFLICT: &str =
    "This device key is already enrolled in this workspace. Join with a new device key.";
const IDENTITY_MISMATCH: &str = "This server account no longer matches the member it joined as. Remove the old member first, then invite them again.";
const STORAGE_FULL: &str = "This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace.";
const TOO_MANY_ATTEMPTS: &str = "Too many attempts at once. Wait a minute, then try again.";
/// How the `quota_exceeded` texts that [`STORAGE_FULL`] rewords begin, in lowercase.
const STORAGE_FULL_PREFIXES: [&str; 4] = [
    "retained audit journal exceeds",
    "journal exceeds",
    "workspace logical state exceeds",
    LEGACY_OPERATION_QUOTA,
];
/// The operation quota an older broker refused with once its table of remembered request IDs
/// was full. The broker no longer writes it (that table now evicts), but a workspace still
/// running an older broker does, so it is still said as [`STORAGE_FULL`].
const LEGACY_OPERATION_QUOTA: &str = "workspace operation quota requires maintenance";
/// The limits that stop ordinary changes a little short of full, so the host can still remove
/// members and change policy (`commit`'s admin headroom in `broker.rs`).
const FULL_BUT_HOST_CAN_ADMINISTER: &str = "This workspace is full. Reading still works, and the host can still remove members and change its privacy, but nothing else can change. Ask the host about starting a new workspace.";
/// How the `quota_exceeded` texts that [`FULL_BUT_HOST_CAN_ADMINISTER`] rewords begin, in
/// lowercase.
const FULL_BUT_HOST_CAN_ADMINISTER_PREFIXES: [&str; 2] = [
    "workspace logical state is full",
    "retained audit journal is nearly full",
];
/// [`STORAGE_FULL`] and [`FULL_BUT_HOST_CAN_ADMINISTER`] as the host reads them (MSG2-N6): the
/// host is the one to ask, so they say what the host can still do. The desktop's
/// `refusalCopy.storageFullHost` and `refusalCopy.fullButHostCanAdministerHost`, byte for byte.
const STORAGE_FULL_FOR_HOST: &str = "This workspace has grown past the size Crew supports and cannot take more changes. To keep working together, start a new workspace.";
const FULL_BUT_HOST_CAN_ADMINISTER_FOR_HOST: &str = "This workspace is full. Reading still works, and you can still remove members and change its privacy. To keep posting, start a new workspace.";

/// Which "this workspace is full" a `quota_exceeded` refusal is, if it is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Full {
    /// Nothing more can change ([`STORAGE_FULL`]).
    Storage,
    /// Only the host's removals and privacy changes can ([`FULL_BUT_HOST_CAN_ADMINISTER`]).
    HostCanAdminister,
}

/// `broker.rs`'s limits that mean "this workspace is full" (the audit journal and state-size
/// limits in `commit` and the operation quota in `apply_mutation`, which are what a request
/// meets, plus the journal limit in `open_inner`, which only stops the broker starting). The
/// join quota's own sentence means "too many people are waiting", and is not one. `message` is
/// the broker's `code: text` or the text alone.
fn workspace_full(code: &str, message: &str) -> Option<Full> {
    if code != "quota_exceeded" {
        return None;
    }
    let message = message.trim();
    let text = message
        .split_once(": ")
        .filter(|(prefix, _)| *prefix == code)
        .map_or(message, |(_, rest)| rest.trim())
        .to_ascii_lowercase();
    if STORAGE_FULL_PREFIXES.iter().any(|p| text.starts_with(p)) {
        Some(Full::Storage)
    } else if FULL_BUT_HOST_CAN_ADMINISTER_PREFIXES
        .iter()
        .any(|p| text.starts_with(p))
    {
        Some(Full::HostCanAdminister)
    } else {
        None
    }
}

const IDENTITY_CONFLICT_UNNAMED: &str =
    "Another active member already has this username. Remove the old member first.";

/// OW-5: every request to a deleted workspace but `hello` is refused `workspace_deleted`, and
/// the one thing left to do is on this computer.
const WORKSPACE_DELETED: &str = "The host deleted this workspace on the server. Remove it from this computer with biorouter crew connections remove.";
/// OW-8: a context change made against a revision someone else has since replaced.
const REVISION_CONFLICT: &str = "Someone changed the workspace context since you read it, so yours wasn't saved. Read it again with biorouter crew workspace context show, then make your change.";
/// OW-4: a deletion whose `expected_name` no longer matches, when the broker's text is not a
/// sentence of its own.
const NAME_CHANGED: &str =
    "Its name changed since you chose it, so nothing was deleted. Check the name, then try again.";
/// A joiner's claim the host has not approved (`code_mismatch`). A person's machine is a
/// "computer" in every sentence (2026-11), and a workspace keeps running the broker it started
/// with, so the words are this terminal's whatever an older broker wrote ("…this device in").
const CODE_MISMATCH: &str =
    "The host hasn't let this computer in. Send the host the code shown on your screen.";

fn identity_conflict(username: Option<&str>) -> String {
    match username {
        Some(name) => {
            format!("Another active member is already @{name}. Remove the old @{name} first.")
        }
        None => IDENTITY_CONFLICT_UNNAMED.to_owned(),
    }
}

/// Q2-75: a repeated approval means a code was already entered for them, not that any device
/// was let in; the broker says the same. The desktop words its own version for its dialog, so
/// this sentence is the terminal's, and the hint line says how to replace the code here.
fn already_approved(username: &str) -> String {
    format!("You already entered a code for @{username}.")
}

/// Codes whose broker text is written for a person, so the `code: ` prefix can go: the
/// desktop's `SENTENCE_CODES` in `dialogs/refusals.ts`.
const SENTENCE_CODES: &[&str] = &[
    "name_invalid",
    "name_taken",
    "device_code_invalid",
    "rate_limited",
    "target_mismatch",
    "not_invited",
    "identity_unavailable",
    "identity_ambiguous",
    "identity_conflict",
    "identity_mismatch",
    "unknown_account",
    "already_member",
    "already_approved",
    "quota_exceeded",
    "join_expired",
    "join_changed",
    "account_changed",
    "code_mismatch",
    "name_changed",
];

/// Whether the broker wrote `sentence` for a person: capitalised (or opening with a number, an
/// `@username` or a `#channel`), and ending in a full stop. The broker's technical texts are
/// neither.
fn reads_as_sentence(sentence: &str) -> bool {
    let first = sentence.chars().next();
    first.is_some_and(|ch| ch.is_uppercase() || ch.is_numeric() || ch == '@' || ch == '#')
        && sentence.ends_with(['.', '!', '?'])
}

/// The first `@username` in `text`. A username may contain dots but never ends in one, so a
/// sentence's own full stop is left out (`Invite @alice.` names `alice`, `@j.doe.` names
/// `j.doe`).
fn username_in(text: &str) -> Option<&str> {
    let (_, rest) = text.split_once('@')?;
    let name = rest
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    (!name.is_empty()).then_some(name)
}

/// The account spelling an `identity_ambiguous` refusal names (`Invite @alice.`), matched as the
/// desktop's `CANONICAL_HINT` matches it.
fn canonical_hint(sentence: &str) -> Option<&str> {
    // ASCII lowercasing keeps every byte offset, so an index into `lower` is one into `sentence`.
    let lower = sentence.to_ascii_lowercase();
    ["invite", "did you mean"].iter().find_map(|marker| {
        lower.match_indices(marker).find_map(|(at, _)| {
            let rest = sentence.get(at + marker.len()..)?;
            let spelled = rest.trim_start();
            (spelled.len() < rest.len() && spelled.starts_with('@'))
                .then(|| username_in(spelled))
                .flatten()
        })
    })
}

/// The broker's technical texts that name something a person can act on, said as a sentence
/// (Q2-76). Matched on the text after its code, in lowercase, without a closing full stop.
const TECHNICAL_TEXTS: &[(&str, &str)] = &[
    (
        "unknown device",
        "This computer isn't a member of this workspace.",
    ),
    (
        "signed device required",
        "This computer isn't signed in to this workspace.",
    ),
    // Only a channel the person is not in (or that does not exist) is unavailable: an archived
    // one stays readable to its members, so "it may be archived" was never the reason (M20).
    ("channel unavailable", "You're not in that channel."),
    (
        "attachment unavailable",
        "That file isn't available to you. It may have been removed, or you may not be in its channel.",
    ),
    (
        "principal unavailable",
        "That person isn't a member of this workspace.",
    ),
    (
        "invalid grant",
        "This task's access to the workspace has ended.",
    ),
    // `message.post`'s size limit (MSG2-N7).
    ("message too long", MESSAGE_TOO_LONG),
];

/// A message over the workspace's size limit, before it is sent or as the broker refuses it
/// (MSG2-N7). The desktop says "Attach long text as a file"; here that is `files upload`.
pub const MESSAGE_TOO_LONG: &str = "Messages can be up to 64 KB. Save the text to a file and share it with biorouter crew files upload.";

/// What a refusal can name when the command knows it (DW-11, M20, R-2, FILES-F9): the channel
/// it acted on, where a shared file came from, whether the person hosts the workspace, and the
/// server. Every text is already made safe by the caller. Without it the sentence names none of
/// them and stays true.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefusalPlace {
    /// The channel the request named, as a person reads it: `#methods`.
    pub channel: Option<String>,
    /// The channel an attachment or remote reference was shared in.
    pub shared_in: Option<String>,
    /// Whether the person hosts the workspace, when that could be read.
    pub host: Option<bool>,
    /// The host, as a person reads them: `"Alice Chen" (@alice)`.
    pub host_label: Option<String>,
    /// The server the workspace runs on.
    pub server: Option<String>,
}

/// What a refusal can name, and so what the command should read for it: the channel the
/// request named, where a shared file came from, or who hosts a server that can no longer save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalSubject {
    Channel,
    SharedIn(SharedKind),
    Storage,
}

/// What was shared in another channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedKind {
    Attachment,
    Reference,
}

/// What `code: message` could name that its text does not ([`placed_refusal`]), or `None`.
pub fn refusal_subject(code: &str, message: &str) -> Option<RefusalSubject> {
    if matches!(code, "storage_failed" | "storage_full") || workspace_full(code, message).is_some()
    {
        return Some(RefusalSubject::Storage);
    }
    let text = message
        .trim()
        .split_once(": ")
        .filter(|(prefix, _)| *prefix == code)
        .map_or(message.trim(), |(_, rest)| rest.trim())
        .trim_end_matches(['.', '!', '?'])
        .to_ascii_lowercase();
    match text.as_str() {
        "current owner required" | "channel is read-only" => Some(RefusalSubject::Channel),
        "attachment provenance cannot be dropped" => {
            Some(RefusalSubject::SharedIn(SharedKind::Attachment))
        }
        "reference provenance cannot be dropped" => {
            Some(RefusalSubject::SharedIn(SharedKind::Reference))
        }
        _ => None,
    }
}

/// The workspace's host as a person reads them, when the directory names them.
pub fn host_label(directory: &Directory) -> Option<String> {
    let host = directory.host_id()?;
    directory
        .people
        .get(host)
        .map(|person| directory.person_label(person))
}

/// A request whose required privacy (`--expected-mode`) is not the connection's (DW-12): both
/// modes named, and that nothing was sent.
pub fn mode_mismatch_text(actual: &str, expected: &str) -> String {
    format!(
        "Your connection is {}, but this request required {}. Nothing was sent.",
        mode_word(Some(actual)),
        mode_word(Some(expected))
    )
}

/// The broker's role, archive, provenance and storage refusals in words for a person, naming
/// what `place` knows. `key` is the refusal's text after its code, lowercase, without a closing
/// full stop. `None` for any other refusal.
fn placed_refusal(code: &str, sentence: &str, key: &str, place: &RefusalPlace) -> Option<String> {
    if matches!(code, "storage_failed" | "storage_full") {
        return Some(storage_refusal(code, sentence, place));
    }
    Some(match key {
        // `manager()`'s check is the host's account, not one of its devices.
        "workspace host device required"
        | "human host policy decision required"
        | "human host decision required"
        | "only host account can stop broker" => "Only the workspace host can do this.".to_owned(),
        "current owner required" => match &place.channel {
            Some(channel) => format!("Only {channel}'s owner can do this."),
            None => "Only the channel's owner can do this.".to_owned(),
        },
        "team owner required" => "Only the team's owner can do this.".to_owned(),
        "team creator required" => "Only the team's creator can do this.".to_owned(),
        "channel is read-only" => match &place.channel {
            Some(channel) => format!("{channel} is archived, so it's read-only."),
            None => "That channel is archived, so it's read-only.".to_owned(),
        },
        "attachment provenance cannot be dropped" => format!(
            "That file was shared in {}. Share it there, or upload it again here.",
            place.shared_in.as_deref().unwrap_or("another channel")
        ),
        "reference provenance cannot be dropped" => format!(
            "That remote reference was shared in {}. Share it there, or add it again here.",
            place.shared_in.as_deref().unwrap_or("another channel")
        ),
        _ => return None,
    })
}

/// A workspace server that cannot save a change (R-2, W2-BRK-3): `storage_full` for a full
/// disk or quota, `storage_failed` for any other write fault. Reading still works, and only the
/// host can put it right, on the server.
///
/// A current broker says what happened in a sentence of its own, and it matters which: the
/// change was not saved, it may not have been ([`storage_outcome_unknown`]), or nothing more
/// can be saved until Crew restarts. That sentence is kept, naming the host when this command
/// could read who they are, and a host is told, on a line of its own, what to run. An older
/// broker's "restart and recover before further mutations" was written for no one, and is said
/// plainly instead.
fn storage_refusal(code: &str, sentence: &str, place: &RefusalPlace) -> String {
    const LEAD: &str = "The workspace server can't save changes right now.";
    let host = place.host == Some(true);
    let said = if reads_as_sentence(sentence) {
        match place.host_label.as_deref().filter(|_| !host) {
            Some(label) => sentence.replacen("Ask the host ", &format!("Ask {label} "), 1),
            None => sentence.to_owned(),
        }
    } else if host {
        LEAD.to_owned()
    } else {
        format!(
            "{LEAD} Ask {} to restart Crew.",
            place.host_label.as_deref().unwrap_or("the host")
        )
    };
    if !host {
        return said;
    }
    let server = place.server.as_deref().unwrap_or("the server");
    let first = match code {
        "storage_full" => format!("Free space on {server}"),
        _ if reads_as_sentence(sentence) => format!("Check the storage on {server}"),
        // An older broker said storage_failed for a full disk too.
        _ => format!("Free space on {server} if it is full"),
    };
    format!("{said}\n{}", host_restart_steps(&first))
}

/// What the host of a server that stopped saving runs, after `first` ("Free space on hpc").
fn host_restart_steps(first: &str) -> String {
    format!(
        "You host this workspace. {first}, then restart Crew there: biorouter-crew stop, then biorouter-crew start, each with this workspace's --state-dir."
    )
}

/// Said under a connection whose workspace server has stopped saving changes, as the daemon
/// reads it from the server (`server_storage`, T3-BE-13, RES2-N2): before anyone tries to write,
/// not only after a change is refused. The host is told what to run on the server.
pub const SERVER_STOPPED_SAVING: &str =
    "The workspace server has stopped saving changes. Reading still works.";

/// Whether a storage refusal says the change may have been saved after all (the broker's
/// "…so it may not have been saved."): an outcome that is not known, so the one safe retry,
/// the same request ID, is offered. The broker has no code of its own for it.
pub fn storage_outcome_unknown(code: &str, message: &str) -> bool {
    matches!(code, "storage_failed" | "storage_full")
        && message
            .to_ascii_lowercase()
            .contains("may not have been saved")
}

/// What a refusal that carried only its code says (`stale_cursor`, with no text of its own).
fn code_only_sentence(code: &str) -> &'static str {
    match code {
        "stale_cursor" => "A message in this view is no longer available to you.",
        "rate_limited" => TOO_MANY_ATTEMPTS,
        "unauthorized" => "This computer isn't signed in to this workspace.",
        "forbidden" => "The workspace didn't allow this.",
        "principal_revoked" => "You're no longer a member of this workspace.",
        "name_taken" => "That name is already taken.",
        _ => "The workspace refused this request.",
    }
}

/// A refusal's text with its code gone, as a sentence (Q2-76): the broker's own sentence when
/// it wrote one, a known technical text in words, else the text itself with a capital and a
/// full stop. The code stays in JSON output (`broker_code`), for scripts and support.
fn plain_refusal(code: &str, sentence: &str) -> String {
    let key = sentence
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .to_ascii_lowercase();
    if key.is_empty() || key == code {
        return code_only_sentence(code).to_owned();
    }
    if let Some((_, words)) = TECHNICAL_TEXTS.iter().find(|(text, _)| *text == key) {
        return (*words).to_owned();
    }
    if reads_as_sentence(sentence) {
        return sentence.to_owned();
    }
    let trimmed = sentence.trim();
    let mut chars = trimmed.chars();
    let mut out: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// A broker refusal in words for a person: the sentence the desktop shows for the same code,
/// then, on its own line, the command that acts on it where there is one.
///
/// `message` is the broker's own `code: sentence`. The code never reaches text output (Q2-76):
/// a person-written sentence is printed without it; a technical text the desktop rewords gets
/// the same words here; anything else becomes a sentence ([`plain_refusal`]). JSON output keeps
/// the code as `broker_code`, which is what scripts and support match.
pub fn broker_refusal_text(code: &str, message: &str) -> String {
    broker_refusal_text_in(code, message, &RefusalPlace::default())
}

/// [`broker_refusal_text`], naming what `place` knows about the refusal.
pub fn broker_refusal_text_in(code: &str, message: &str, place: &RefusalPlace) -> String {
    let message = message.trim();
    let sentence = message
        .split_once(": ")
        .filter(|(prefix, _)| {
            !prefix.is_empty()
                && prefix
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        })
        .map_or(message, |(_, rest)| rest.trim());
    let lower = sentence.to_ascii_lowercase();
    let key = lower.trim().trim_end_matches(['.', '!', '?']);
    if let Some(placed) = placed_refusal(code, sentence, key, place) {
        return placed;
    }
    let shown = match (code, username_in(sentence)) {
        ("identity_conflict", named) if lower.starts_with("another active member is @") => {
            identity_conflict(named)
        }
        ("device_conflict", _) => DEVICE_CONFLICT.to_owned(),
        ("code_mismatch", _) => CODE_MISMATCH.to_owned(),
        ("identity_mismatch", _) if !reads_as_sentence(sentence) => IDENTITY_MISMATCH.to_owned(),
        // A full workspace ([`workspace_full`]). The desktop matches the same texts
        // (`STORAGE_FULL_TEXT` in `refusals.ts`). Its host reads what the host can do (MSG2-N6).
        ("quota_exceeded", _) if workspace_full(code, sentence).is_some() => {
            let host = place.host == Some(true);
            match (workspace_full(code, sentence), host) {
                (Some(Full::Storage), false) => STORAGE_FULL,
                (Some(Full::Storage), true) => STORAGE_FULL_FOR_HOST,
                (_, false) => FULL_BUT_HOST_CAN_ADMINISTER,
                (_, true) => FULL_BUT_HOST_CAN_ADMINISTER_FOR_HOST,
            }
            .to_owned()
        }
        ("rate_limited", _) if !reads_as_sentence(sentence) => TOO_MANY_ATTEMPTS.to_owned(),
        // OW-5 and OW-8: what to do next is a command here, whatever the broker's words.
        ("workspace_deleted", _) => WORKSPACE_DELETED.to_owned(),
        ("revision_conflict", _) => REVISION_CONFLICT.to_owned(),
        ("name_changed", _) if !reads_as_sentence(sentence) => NAME_CHANGED.to_owned(),
        ("already_approved", Some(name)) => already_approved(name),
        // CLIDOCS-F12: the broker's "Choose Add device…" names the desktop's control; the
        // hint below names this terminal's flag.
        ("already_member", Some(name)) => format!("@{name} is already a member."),
        // The same for an invitation that adds a computer for someone who isn't a member: the
        // broker's "Turn off Add another computer…" names the desktop's switch.
        ("invalid_params", Some(name)) if lower.contains("is not a member yet") => {
            format!("@{name} is not a member yet. Invite them without --add-device.")
        }
        _ if SENTENCE_CODES.contains(&code) && reads_as_sentence(sentence) => sentence.to_owned(),
        _ => plain_refusal(code, sentence),
    };
    match broker_refusal_hint(code, sentence, &shown) {
        Some(hint) => format!("{shown}\n{hint}"),
        None => shown,
    }
}

/// The command that acts on a broker refusal, when one does.
fn broker_refusal_hint(code: &str, sentence: &str, shown: &str) -> Option<String> {
    match code {
        "already_approved" => Some(format!(
            "If it didn't match their computer, run: biorouter crew enroll approve {} <code> --replace",
            username_in(sentence).map_or_else(|| "<user>".to_owned(), |name| format!("@{name}"))
        )),
        "identity_ambiguous" => canonical_hint(sentence)
            .map(|canonical| format!("Run: biorouter crew enroll invite @{canonical}")),
        "already_member" => username_in(sentence).map(|name| {
            format!(
                "To add another computer for them, run: biorouter crew enroll invite @{name} --add-device"
            )
        }),
        // The broker's own sentence already says so; a bare code does not.
        "name_taken" if !shown.to_ascii_lowercase().contains("choose a different name") => {
            Some("Choose another name.".to_owned())
        }
        // OW-4: a team's #general goes only with its team.
        "invalid_params" if sentence.contains("belongs to its team") => {
            Some("Run: biorouter crew teams delete TEAM".to_owned())
        }
        "invalid_params" if sentence.to_ascii_lowercase().contains("is not a member yet") => {
            username_in(sentence).map(|name| format!("Run: biorouter crew enroll invite @{name}"))
        }
        _ => None,
    }
}

/// `tasks start`'s institution refusal in the desktop's words (Q2-76, `institutionMismatch` in
/// `ui/desktop/src/components/crew/pane/copy.ts`), in place of the daemon's "…the model's
/// resolved affiliation…". `details` is the refusal's `institution_refusal` object; an older
/// daemon sends none, and the sentence then names only the model the person asked for.
pub fn institution_refusal_text(requested_model: &str, details: Option<&Value>) -> String {
    let field = |key: &str| {
        details
            .and_then(|details| details.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    let model = field("model").map_or_else(|| safe_text(requested_model), safe_text);
    let workspace = field("workspace").map_or_else(|| "This workspace".to_owned(), safe_text);
    let approved_for: Option<Vec<String>> = details
        .and_then(|details| details.get("approved_for"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(institution_label)
                .collect::<Vec<_>>()
        })
        .filter(|ids| !ids.is_empty());
    // SF-F4: named as the desktop's `institutionMismatch` names them, the workspace's
    // institution in the advice too, not "it".
    match (field("workspace_institution").map(institution_label), approved_for) {
        (Some(institution), Some(approved)) => format!(
            "{model} is approved for {}. {workspace} uses {institution}. Choose a model approved for {institution}, or a local model.",
            approved.join(" and ")
        ),
        (Some(institution), None) => format!(
            "{model} doesn't say which institution approved it. {workspace} uses {institution}. Choose a model approved for {institution}, or a local model."
        ),
        (None, _) => format!(
            "{model} isn't approved for this workspace's institution. Choose a model approved for it, or a local model."
        ),
    }
}

/// An institution as running text names it, as the desktop's `institutionLabel` does: the name
/// the model registry publishes for that exact ID (`ucsf` is UCSF), else the ID itself.
pub fn institution_label(id: &str) -> String {
    use biorouter::privacy::affiliation::{institution_display_name, InstitutionId};
    let id = id.trim();
    // Only a canonical ID can be one the registry names; nothing else is looked up.
    let canonical = (1..=64).contains(&id.len())
        && id.starts_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    canonical
        .then(|| InstitutionId::new(id))
        .filter(|key| key.as_str() == id)
        .and_then(institution_display_name)
        .map_or_else(|| safe_text(id), str::to_owned)
}

/// The daemon's typed connect failures (`connect_refusal` in `routes/crew.rs`) in words for a
/// person, each with what to do next, as the desktop's connection-problem screens say them.
/// `crew_handoff_failed` is not here: its own text is already written for a person.
pub fn connect_failure_text(code: &str) -> Option<&'static str> {
    Some(match code {
        "crew_ssh_auth_required" => {
            "The server wants your password or a verification code. Run biorouter crew auth to sign in."
        }
        // W2-DMN-5: a publickey-only refusal is a login matter, not a password one.
        "crew_ssh_key_refused" => {
            "The server refused this computer's SSH key. Check the server login and key file of the saved connection (biorouter crew connections show), then connect again."
        }
        // W2-DMN-5, R-7: the server answered, but the workspace server on it is not running.
        "crew_broker_not_running" => {
            "Crew isn't running on the server. If you host this workspace, start it there as the manual's After the server restarts shows, then connect again. Otherwise, ask your host to start Crew."
        }
        "crew_ssh_host_key_unknown" => {
            "Crew can't verify the server yet: its host key isn't in your known hosts file. Get the server's fingerprint from your IT team, compare it, add the full key to your known hosts file, then connect again."
        }
        "crew_ssh_host_key_changed" => {
            "The server's host key changed since Crew last connected. Don't connect until your IT team confirms the change; Crew won't connect while the old key is in your known hosts file."
        }
        "crew_ssh_unreachable" => {
            "Couldn't reach the server. Check your network or your VPN, then connect again."
        }
        // OW-9: the daemon installs it now, from the copy this computer carries.
        "crew_bridge_missing" => {
            "Crew isn't set up for your account on the server: ~/.local/bin/biorouter-crew is missing there. Run biorouter crew setup to install it, then connect again."
        }
        // OW-5: the host deleted the workspace on the server; the saved connection is all
        // that is left of it here.
        "crew_workspace_deleted" => {
            WORKSPACE_DELETED
        }
        "crew_ssh_failed" => {
            "SSH or Crew on the server failed. If the server restarted, ask your host to start the workspace again, then connect again."
        }
        "crew_workspace_identity_mismatch" => {
            "This isn't the workspace you joined: the server answered with a different workspace key than the one saved. Don't continue until your host confirms what changed."
        }
        _ => return None,
    })
}

/// The user and host of an SSH login that signs in to this machine over loopback
/// (`crew_iris@localhost`, `127.0.0.1`, `[::1]`): what the daemon's `SignInTarget::is_loopback`
/// in `crew/transport.rs` treats as the same host. `None` for any other login, an SSH alias
/// included, which only the person's SSH settings resolve.
pub fn loopback_login(ssh_target: &str) -> Option<(Option<&str>, &str)> {
    let ssh_target = ssh_target.trim();
    let (user, host) = match ssh_target.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() && !host.is_empty() => (Some(user), host),
        _ => (None, ssh_target),
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    loopback.then_some((user, host))
}

/// A refused connect to a login on this machine, in the daemon's same-host words (W2-DMN-2,
/// SETUPHPC2-F-A): a member on the workspace's own server needs this machine's host key in
/// their own known hosts file, and their own key in their own `authorized_keys`, so "get the
/// fingerprint from your IT team" and "check the saved connection" point the wrong way.
/// `None` for another code, or another server.
pub fn same_host_connect_failure_text(code: &str, ssh_target: &str) -> Option<String> {
    let (user, host) = loopback_login(ssh_target)?;
    match code {
        "crew_ssh_host_key_unknown" => Some(format!(
            "{}'s host key isn't in your ~/.ssh/known_hosts yet. Add this server's own key (from /etc/ssh/ssh_host_ed25519_key.pub) to it, then connect again.",
            safe_text(host)
        )),
        "crew_ssh_key_refused" => Some(format!(
            "Couldn't sign in as {} on this machine: add your public SSH key to your own ~/.ssh/authorized_keys, then connect again.",
            user.map_or_else(|| "yourself".to_owned(), safe_text)
        )),
        _ => None,
    }
}

/// The setup refusals and job failures (OW-9; `SetupError.code` and the setup routes' refusals
/// in `crew/setup.rs`), each with the sentence said when the daemon wrote none.
const SETUP_FAILURES: &[(&str, &str)] = &[
    (
        "crew_setup_busy",
        "Biorouter is already setting up as many servers as it can at once. Wait for one to finish, then try again.",
    ),
    (
        "crew_setup_not_found",
        "That setup isn't running any more. Run the same command again.",
    ),
    (
        "crew_setup_cancelled",
        "Setup was stopped before it finished. Run the same command again to finish.",
    ),
    (
        "crew_setup_timed_out",
        "Setting up took too long and was stopped. Check the connection to the server, then run the same command again.",
    ),
    (
        "crew_setup_sign_in_expired",
        "Nobody signed in to the server in time, so setup stopped. Run the same command again in a terminal.",
    ),
    (
        "crew_setup_not_waiting_for_sign_in",
        "This setup isn't waiting for anyone to sign in.",
    ),
    (
        "crew_setup_server_unreadable",
        "Biorouter couldn't read what the server answered, so nothing was changed there.",
    ),
    (
        "crew_setup_start_failed",
        "Crew didn't start the workspace on the server.",
    ),
    (
        "crew_host_storage_unsafe",
        "Other accounts on the server can change folders of yours that hold Crew's storage, so the workspace wasn't started.",
    ),
    (
        "crew_server_probe_unsupported",
        "The Crew in your account on the server can't check the server yet.",
    ),
    (
        "crew_install_artifact_unavailable",
        "This copy of Biorouter carries no Crew server program to install. Install the Biorouter app or a release package, then try again.",
    ),
    (
        "crew_install_unsupported_platform",
        "Crew's server program runs on Linux x86_64 servers only, and this server is something else.",
    ),
    (
        "crew_install_checksum_mismatch",
        "The copy that reached the server didn't match this computer's, so nothing was replaced. Run the same command again.",
    ),
    (
        "crew_install_checksum_tool_missing",
        "The server has no sha256sum, shasum or openssl to check the copy with, so Crew wasn't installed. Ask your IT team for one of them.",
    ),
    (
        "crew_install_permission_denied",
        "Crew couldn't write ~/.local/bin in your account on the server. Check that folder's permissions, then run the same command again.",
    ),
    (
        "crew_install_unexpected_target",
        "Where Crew goes in your account on the server, ~/.local/bin/biorouter-crew, there is a link, a folder or another account's file, so nothing was installed.",
    ),
    (
        "crew_install_dependency_missing",
        "Crew can't run on this server: a system library it needs is missing or too old.",
    ),
    (
        "crew_install_no_space",
        "Your account on the server doesn't have room for Crew. Free some space, then run the same command again.",
    ),
    (
        "crew_install_transfer_incomplete",
        "The copy to the server stopped before it finished, and nothing was replaced. Run the same command again.",
    ),
    (
        "crew_install_busy",
        "Crew is being installed in this account on the server right now. Wait a moment, then try again.",
    ),
    (
        "crew_install_outcome_unknown",
        "The connection dropped after Crew was copied, so Biorouter couldn't tell whether it was installed.",
    ),
    (
        "crew_install_incompatible",
        "The Crew in your account on the server doesn't run, or doesn't say which version it is.",
    ),
    (
        "crew_host_name_in_use",
        "You already host a workspace of that name on the server. Open it on this computer instead.",
    ),
];

/// Whether `code` is one of the setup routes' codes ([`SETUP_FAILURES`]).
pub fn is_setup_code(code: &str) -> bool {
    SETUP_FAILURES.iter().any(|(known, _)| *known == code)
}

/// A setup refusal or failure for a person: the daemon's own sentence (`own`), else the
/// command line's, then, on its own line, the flag or command that acts on it here.
pub fn setup_failure_sentence(code: &str, own: Option<&str>) -> String {
    let fallback = SETUP_FAILURES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or("Setting up Crew on the server failed.", |(_, text)| *text);
    let said = own
        .map(str::trim)
        .filter(|own| !own.is_empty())
        .unwrap_or(fallback);
    let hint = match code {
        "crew_install_unexpected_target" => Some(
            "Setup never replaces that. Move it out of the way on the server, then run the same command again.",
        ),
        "crew_install_incompatible" => Some(
            "To replace it, run the same command with --replace-existing; the old one is kept as biorouter-crew.previous.",
        ),
        "crew_install_outcome_unknown" => Some("Run biorouter crew doctor to check."),
        "crew_server_probe_unsupported" => {
            Some("Update it with biorouter crew setup, then try again.")
        }
        // L3: `crew host --fix-permissions` runs that chmod for the person, on exactly those
        // folders, and goes on.
        "crew_host_storage_unsafe" => Some(
            "Or run the same command with --fix-permissions, and setup removes that write access for you.",
        ),
        _ => None,
    };
    match hint {
        // The daemon's own sentence may already say it.
        Some(hint)
            if !said.contains("--replace-existing")
                && !said.contains("crew setup")
                && !said.contains("--fix-permissions") =>
        {
            format!("{said}\n{hint}")
        }
        _ => said.to_owned(),
    }
}

/// A setup failure's sentence, then the server's or the broker's own words beside it
/// (`SetupError.detail`, L3): what a failed start printed is the one thing that says why, so a
/// person reads it here, as a failed connect's OpenSSH words are read. The detail is already
/// terminal-safe line by line; at most [`SETUP_DETAIL_LINES`] lines are shown.
pub fn with_setup_detail(sentence: &str, detail: Option<&str>) -> String {
    let Some(detail) = detail.map(str::trim).filter(|detail| !detail.is_empty()) else {
        return sentence.to_owned();
    };
    let mut lines = vec![sentence.to_owned()];
    let all: Vec<&str> = detail
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    for (index, line) in all.iter().take(SETUP_DETAIL_LINES).enumerate() {
        lines.push(if index == 0 {
            format!("  Details: {line}")
        } else {
            format!("    {line}")
        });
    }
    if all.len() > SETUP_DETAIL_LINES {
        lines.push(format!(
            "    ({} more lines in --output-format json)",
            all.len() - SETUP_DETAIL_LINES
        ));
    }
    lines.join("\n")
}

/// How many lines of a setup failure's detail text output shows.
const SETUP_DETAIL_LINES: usize = 20;

/// The server a setup job is for, as a sentence names it: the login the daemon checked, else
/// the one the person gave.
pub fn setup_server(status: &Value, login: &str) -> String {
    str_field(&status["server"], "login")
        .or(Some(login.trim()).filter(|login| !login.is_empty()))
        .map_or_else(|| "the server".to_owned(), safe_text)
}

/// A setup stage for a person (OW-9), as the desktop's Host dialog names its steps. A stage
/// this version does not know is said from its own name.
pub fn setup_stage_words(name: &str) -> String {
    match name {
        "check" => "Checking the server".to_owned(),
        "sign_in" => "Signing in".to_owned(),
        "install" => "Installing Crew in your account".to_owned(),
        "workspaces" => "Looking for workspaces".to_owned(),
        "restart" => "Updating running workspaces".to_owned(),
        "start" => "Starting the workspace".to_owned(),
        "connect" => "Connecting".to_owned(),
        "work_folder" => "Making the work folder".to_owned(),
        "context" => "Reading what the server has, for the workspace context".to_owned(),
        "save" => "Saving the workspace on this computer".to_owned(),
        "join" => "Joining it as its host".to_owned(),
        "configure" => "Setting up the workspace".to_owned(),
        "done" => "Ready".to_owned(),
        other => sentence_case(&other.replace('_', " ")),
    }
}

/// The workspaces a setup request names by ID (AO-8): stopped ones to start
/// (`start_workspaces`) and running ones to restart (`restart_workspace_ids`). Its restart stage
/// is said by their names ([`setup_progress_lines`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupNamed {
    start: Vec<String>,
    restart: Vec<String>,
}

/// What `request` (a `POST /crew/setup` body) names to start and restart ([`SetupNamed`]).
pub fn named_in_setup(request: &Value) -> SetupNamed {
    let ids = |key: &str| -> Vec<String> {
        list_key(request, key)
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    SetupNamed {
        start: ids("start_workspaces"),
        restart: ids("restart_workspace_ids"),
    }
}

impl SetupNamed {
    /// The restart stage in `current`'s words: "Restarting old-lab", "Starting acct-lab", or
    /// "Restarting old-lab, then starting acct-lab", each workspace by the name the job's own
    /// listing gives its ID (the server restarts before it starts). `None` when nothing is
    /// named, or the listing names none of them: the stage then keeps its own words.
    fn restart_words(&self, current: &Value) -> Option<String> {
        let names = |ids: &[String]| -> Vec<String> {
            ids.iter()
                .filter_map(|id| {
                    list_key(current, "workspaces")
                        .iter()
                        .find(|workspace| str_field(workspace, "workspace_id") == Some(id.as_str()))
                        .and_then(|workspace| str_field(workspace, "name"))
                        .map(display_text)
                })
                .collect()
        };
        let (restart, start) = (names(&self.restart), names(&self.start));
        match (restart.is_empty(), start.is_empty()) {
            (true, true) => None,
            (false, true) => Some(format!("Restarting {}", super::and_list(&restart))),
            (true, false) => Some(format!("Starting {}", super::and_list(&start))),
            (false, false) => Some(format!(
                "Restarting {}, then starting {}",
                super::and_list(&restart),
                super::and_list(&start)
            )),
        }
    }
}

/// `previous` with the stages `earlier`, another job of the same command, said as it ran (the
/// listing `setup --start` and `open` run first), so a stage is said once in the command
/// ([`setup_progress_lines`]). Its Done stage is not among them: the listing was not the end.
pub fn with_said_stages(previous: &Value, earlier: &Value) -> Value {
    let said: Vec<Value> = list_key(earlier, "stages")
        .iter()
        .filter(|stage| str_field(stage, "name") != Some("done"))
        .cloned()
        .collect();
    if said.is_empty() {
        return previous.clone();
    }
    let mut stages = list_key(previous, "stages").to_vec();
    stages.extend(said);
    json!({"state": previous.get("state").cloned().unwrap_or(Value::Null), "stages": stages})
}

/// What a setup request asked to restart or start among the workspaces the account hosts
/// (AO-8, C3), and the route flags its suggested commands repeat. Only a workspace it asked for
/// can be said to have been left running ("wasn't restarted"): every one
/// (`restart_workspaces: true`), or one it named by ID (`restart_workspace_ids`,
/// `start_workspaces`). Any other was only looked at, whatever the job did for the ones beside
/// it, as the desktop's `askedToRestart` (`OpenWorkspaceDialog.tsx`) decides it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupAsked {
    all: bool,
    ids: Vec<String>,
    /// The route flags given, already escaped ([`super::route_flags`]).
    route: String,
}

impl SetupAsked {
    /// What `request` (a `POST /crew/setup` body, or `Value::Null` for one that asks nothing of
    /// the hosted workspaces) asked, with `route`, the route flags the person gave.
    pub fn of(request: &Value, route: String) -> Self {
        let ids = ["restart_workspace_ids", "start_workspaces"]
            .iter()
            .flat_map(|key| list_key(request, key))
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        Self {
            all: request.get("restart_workspaces").and_then(Value::as_bool) == Some(true),
            ids,
            route,
        }
    }

    /// Whether the request asked to restart `entry`, a workspace the account hosts.
    fn asked(&self, entry: &Value) -> bool {
        self.all
            || str_field(entry, "workspace_id")
                .is_some_and(|id| self.ids.iter().any(|asked| asked.eq_ignore_ascii_case(id)))
    }
}

/// The lines a listing ends with when the command goes on to another job (`setup --start`,
/// `open`): its own "Ready" left out, since the command is not done.
pub fn before_the_setup(lines: Vec<String>) -> Vec<String> {
    let ready = setup_stage_words("done");
    lines.into_iter().filter(|line| *line != ready).collect()
}

/// The account's place in the workspace `workspace_id` as a setup check job listed it (`host`,
/// `member` or `invited`), when every row the listing has for that ID agrees. `None` when the
/// listing does not name it, or names it twice with different places.
pub fn listed_role(listing: &Value, workspace_id: Option<&str>) -> Option<String> {
    let id = workspace_id?;
    let mut roles = list_key(listing, "workspaces")
        .iter()
        .filter(|workspace| str_field(workspace, "workspace_id") == Some(id))
        .map(|workspace| str_field(workspace, "role"));
    let first = roles.next()??;
    roles
        .all(|role| role == Some(first))
        .then(|| first.to_owned())
}

/// The lines a setup job's change from `previous` to `current` adds: each stage the first time
/// it is seen running or done (a quick stage can finish between two looks), and a sign-in the
/// server asks for. A skipped stage says nothing; a failure is the command's error. The restart
/// stage names the workspaces `named` asks for ([`SetupNamed::restart_words`]).
pub fn setup_progress_lines(
    previous: &Value,
    current: &Value,
    login: &str,
    named: &SetupNamed,
) -> Vec<String> {
    let seen = |status: &Value, name: &str| {
        list(status, "stages").iter().any(|stage| {
            str_field(stage, "name") == Some(name)
                && matches!(
                    str_field(stage, "state"),
                    Some("running" | "done" | "failed")
                )
        })
    };
    // A check that could not list the workspaces says why (`workspaces_note`, in the summary),
    // and that is all it found: no bare "Ready" before it.
    let unlisted_check = str_field(current, "intent") == Some("check")
        && str_field(current, "workspaces_note").is_some();
    let mut lines: Vec<String> = list(current, "stages")
        .iter()
        .filter(|stage| matches!(str_field(stage, "state"), Some("running" | "done")))
        // AO-8: a job that found workspaces this computer cannot reach says so in place of a
        // bare "Ready" (the Done stage's detail), and the summary says it.
        .filter(|stage| {
            !(str_field(stage, "name") == Some("done")
                && (str_field(stage, "detail").is_some() || unlisted_check))
        })
        .filter_map(|stage| str_field(stage, "name"))
        .filter(|name| !seen(previous, name))
        .map(|name| match name {
            "done" => setup_stage_words(name),
            "restart" => format!(
                "{}…",
                named
                    .restart_words(current)
                    .unwrap_or_else(|| setup_stage_words(name))
            ),
            _ => format!("{}…", setup_stage_words(name)),
        })
        .collect();
    if str_field(current, "state") == Some("needs_sign_in")
        && str_field(previous, "state") != Some("needs_sign_in")
    {
        lines.push(format!(
            "{} asks for your password or a verification code.",
            setup_server(current, login)
        ));
    }
    lines
}

/// What a finished setup job did, for a person: what a check found, or which Crew is now
/// installed and, for a member, that the connection is up and what agents may do there.
///
/// `connection` is the saved connection the job was for, read after it finished, when there is
/// one: the job's own server facts never say whether the kernel confines commands to a folder
/// (only the connection's `server_access`, from its bridge, does, C25), and what agents may do
/// is what the connection now says, not what the job could have set (C22).
///
/// `asked` is what the request asked to restart, with the route flags given ([`SetupAsked`]):
/// it decides whether a workspace still on an older Crew was left or only looked at, and the
/// commands suggested for it reach the server as the person's did.
pub fn setup_summary_lines(
    status: &Value,
    login: &str,
    connection: Option<&Value>,
    asked: &SetupAsked,
) -> Vec<String> {
    let server = setup_server(status, login);
    let server_access = connection.map_or(&Value::Null, |connection| &connection["server_access"]);
    let outcome = &status["outcome"];
    let installed = &status["installed"];
    let artifact = str_field(&status["artifact"], "version").map(safe_text);
    if str_field(status, "intent") == Some("check") {
        let facts = &status["server"];
        let mut lines = Vec::new();
        let system: Vec<String> = ["os", "arch"]
            .iter()
            .filter_map(|key| str_field(facts, key))
            .map(safe_text)
            .collect();
        if !system.is_empty() {
            let mut line = format!("{server}: {}", system.join(" "));
            if let Some(glibc) = str_field(facts, "glibc") {
                line.push_str(&format!(", glibc {}", safe_text(glibc)));
            }
            lines.push(format!("{line}."));
        }
        lines.push(match str_field(installed, "state") {
            Some("missing") | None => match &artifact {
                Some(version) => format!("Crew isn't installed in your account there. biorouter crew setup installs Crew {version}."),
                None => "Crew isn't installed in your account there.".to_owned(),
            },
            Some("symlink") => "~/.local/bin/biorouter-crew there is a symbolic link, which setup never replaces. Remove it, then run biorouter crew setup.".to_owned(),
            Some("file") => match (str_field(installed, "version").map(safe_text), &artifact) {
                (Some(version), Some(ours)) => match release_order(&version, ours) {
                    // AO-8: setup replaces an older Crew by default.
                    Some(std::cmp::Ordering::Less) => format!(
                        "Crew {version} is installed in your account there. This computer has Crew {ours}; biorouter crew setup updates it."
                    ),
                    Some(std::cmp::Ordering::Greater) => format!(
                        "Crew {version} is installed in your account there, newer than this computer's {ours}, so setup leaves it in place."
                    ),
                    _ => format!("Crew {version} is installed in your account there."),
                },
                (Some(version), None) => {
                    format!("Crew {version} is installed in your account there.")
                }
                (None, _) => "A Crew is installed in your account there, but it doesn't say which version it is. biorouter crew setup --replace-existing replaces it.".to_owned(),
            },
            Some(_) => "~/.local/bin/biorouter-crew there is a folder or something else that isn't a program, which setup never replaces. Move it out of the way, then run biorouter crew setup.".to_owned(),
        });
        if let Some(problem) = str_field(installed, "problem") {
            lines.push(safe_text(problem));
        }
        // The bridge's own report, from the connection's last connect: the SSH check never
        // learns the kernel's Landlock ABI.
        let unconfined = server_access
            .get("confined_execution_available")
            .and_then(Value::as_bool)
            == Some(false)
            || server_access
                .get("landlock_abi")
                .and_then(Value::as_i64)
                .is_some_and(|abi| abi < 3);
        if unconfined {
            lines.push("Agents can run commands there only with full access: its kernel can't confine them to a folder.".to_owned());
        }
        lines.extend(setup_workspace_lines(status, login, &server, asked));
        return lines;
    }
    let version = str_field(outcome, "installed_version")
        .or_else(|| str_field(installed, "version"))
        .map(safe_text);
    let crew = version
        .as_deref()
        .map_or_else(|| "Crew".to_owned(), |version| format!("Crew {version}"));
    let mut lines = vec![
        if outcome.get("reused").and_then(Value::as_bool) == Some(true) {
            format!("{crew} was already installed in your account on {server}.")
        } else if let Some(previous) = setup_replaced(outcome, installed) {
            match version.as_deref() {
                Some(now) if previous != now => {
                    format!("Updated Crew from {previous} to {now} in your account on {server}.")
                }
                // The same version with another SHA-256, or no version to compare.
                _ => format!(
                    "Replaced {crew} in your account on {server} with this computer's copy of it."
                ),
            }
        } else if outcome.get("previous_kept").and_then(Value::as_bool) == Some(true) {
            format!("Replaced the Crew program in your account on {server} with {crew}.")
        } else {
            format!("Installed {crew} in your account on {server}.")
        },
    ];
    if str_field(status, "intent") == Some("member") {
        if str_field(outcome, "connection_id").is_some() {
            lines.push("Connected.".to_owned());
        }
        lines.extend(member_access_line(outcome, connection, &server));
    }
    lines.extend(setup_workspace_lines(status, login, &server, asked));
    lines
}

/// What a setup job found and did about the workspaces on the server (AO-6, AO-8). A check or
/// an install lists the workspaces there for the account, then, in place of a bare "Ready", the
/// daemon's sentences for those this computer cannot reach (the Done stage's detail), or why
/// it could not list them. Every job then says which workspaces it restarted or started, which
/// still run an older Crew and why, and which restarts failed, with the command that starts
/// each by hand. Nothing when the job reported none of it. `asked` is what the request asked to
/// restart ([`SetupAsked`]).
fn setup_workspace_lines(
    status: &Value,
    login: &str,
    server: &str,
    asked: &SetupAsked,
) -> Vec<String> {
    let mut lines = Vec::new();
    if matches!(str_field(status, "intent"), Some("check" | "install")) {
        let workspaces = list_key(status, "workspaces");
        if !workspaces.is_empty() {
            lines.push(format!("Workspaces on {server} for your account:"));
            lines.extend(
                workspaces
                    .iter()
                    .map(|workspace| format!("  {}", setup_workspace_row(workspace))),
            );
        }
    }
    // Said in place of "Ready" by any job that found such a workspace.
    lines.extend(stage_detail(status, "done"));
    if matches!(str_field(status, "intent"), Some("check" | "install")) {
        if let Some(note) = str_field(status, "workspaces_note") {
            lines.push(safe_text(note));
        }
        let scan = &status["workspaces_scan"];
        match scan.get("unsupported").and_then(Value::as_u64).unwrap_or(0) {
            0 => {}
            1 => lines.push(format!(
                "Another workspace on {server} runs an older Crew, which can't say whether you belong to it."
            )),
            n => lines.push(format!(
                "{n} other workspaces on {server} run an older Crew, which can't say whether you belong to them."
            )),
        }
        if scan.get("truncated").and_then(Value::as_bool) == Some(true) {
            lines.push(format!(
                "There was more on {server} than setup looks through at once, so this list may not be complete."
            ));
        }
    }
    let outcome = &status["outcome"];
    lines.extend(list_key(outcome, "restarted").iter().map(restarted_line));
    // C3: a check only looks, and any other job only looked at a workspace it was not asked to
    // restart, so neither says that one "wasn't restarted".
    let check = str_field(status, "intent") == Some("check");
    lines.extend(list_key(outcome, "outdated").iter().map(|entry| {
        let look_only = check || !asked.asked(entry);
        outdated_line(entry, server, login, look_only, &asked.route)
    }));
    for failed in list_key(outcome, "restart_failed") {
        lines.extend(restart_failed_lines(failed, server, login));
    }
    lines
}

/// A hosted workspace a job could not restart or start (AO-7, AO-8), and what to do about it.
/// Two refusals the server makes before it stops anything are said in their own words, since
/// the job's sentence for a failed restart says the workspace stopped: the installed program is
/// missing or can't run (`program_missing`), or more than one folder holds the workspace
/// (`identity_conflict`). Either way it still runs as it did, and starting it by hand would be
/// refused the same way, so no command is offered for it.
fn restart_failed_lines(failed: &Value, server: &str, login: &str) -> Vec<String> {
    let name = listed_name(failed);
    let code = str_field(failed, "code").unwrap_or_default();
    let own = server_sentence(str_field(failed, "detail"), code);
    match code {
        "program_missing" => vec![format!(
            "{name}: {} Install it again with biorouter crew setup --server {}.",
            own.unwrap_or_else(|| format!(
                "The Crew program in your account on {server} is missing or can't run, so it was left as it was."
            )),
            safe_text(&super::shell_word(login.trim()))
        )],
        "identity_conflict" => vec![format!(
            "{name}: {}",
            own.unwrap_or_else(|| format!(
                "More than one folder of ~/.local/share/biorouter-crew on {server} holds it, so it was left as it was. Keep its own folder, move the other copies out of that folder, then try again."
            ))
        )],
        _ => {
            let mut lines = vec![format!(
                "{name}: {}",
                str_field(failed, "message")
                    .map_or_else(|| "it didn't restart.".to_owned(), safe_text)
            )];
            if let Some(command) = str_field(failed, "command") {
                lines.push(format!(
                    "  To start it by hand, run this on {server}: {}",
                    safe_text(command)
                ));
            }
            lines
        }
    }
}

/// The server's own sentence for its refusal `code` in what it printed (`detail`): a line
/// `Error: code: sentence` or `code: sentence`, or a JSON answer whose `error` says it.
fn server_sentence(detail: Option<&str>, code: &str) -> Option<String> {
    let after_code = |text: &str| -> Option<String> {
        let text = text.trim();
        let text = text.strip_prefix("Error:").map_or(text, str::trim);
        let sentence = text.strip_prefix(code)?.strip_prefix(':')?.trim();
        (!sentence.is_empty()).then(|| safe_text(sentence))
    };
    detail?.lines().find_map(|line| {
        after_code(line).or_else(|| {
            let answer: Value = serde_json::from_str(line.trim()).ok()?;
            after_code(str_field(&answer, "error")?)
        })
    })
}

/// A workspace a setup job listed, in a sentence: its name, or "A workspace".
fn listed_name(workspace: &Value) -> String {
    str_field(workspace, "name").map_or_else(|| "A workspace".to_owned(), display_text)
}

/// One workspace on the server, as a setup job listed it (AO-6): its name, who hosts it and the
/// account's place in it, whether it runs and on which Crew, and what this computer has for it.
fn setup_workspace_row(workspace: &Value) -> String {
    let host = str_field(workspace, "host_username").map(|host| format!("@{}", safe_text(host)));
    let role = match (str_field(workspace, "role"), host) {
        (Some("host"), _) => "you host it".to_owned(),
        (Some("member"), Some(host)) => format!("hosted by {host}, you're a member"),
        (Some("member"), None) => "you're a member".to_owned(),
        (Some("invited"), Some(host)) => format!("hosted by {host}, you're invited"),
        (Some("invited"), None) => "you're invited".to_owned(),
        (Some("removed"), _) => "you were removed".to_owned(),
        (_, Some(host)) => format!("hosted by {host}"),
        (_, None) => "your place in it is unknown".to_owned(),
    };
    let state = match (
        str_field(workspace, "state"),
        str_field(workspace, "version"),
    ) {
        (Some("running"), Some(version)) => format!("running Crew {}", safe_text(version)),
        (Some("running"), None) => "running".to_owned(),
        (Some("stopped"), _) => "stopped".to_owned(),
        (Some("deleted"), _) => "deleted".to_owned(),
        (Some("storage_failed"), _) => "its storage failed".to_owned(),
        _ => "state unknown".to_owned(),
    };
    let here = match str_field(workspace, "this_computer") {
        Some("connected") => "connected on this computer",
        Some("saved") => "saved on this computer",
        _ => "not on this computer",
    };
    format!("{} · {role} · {state} · {here}", listed_name(workspace))
}

/// A workspace a setup job restarted on the installed Crew, or started (AO-8).
fn restarted_line(entry: &Value) -> String {
    let name = listed_name(entry);
    let from = str_field(entry, "from_version").map(safe_text);
    let to = str_field(entry, "to_version").map(safe_text);
    match (from, to) {
        (Some(from), Some(to)) if from != to => {
            format!("Restarted {name}: it ran Crew {from} and now runs Crew {to}.")
        }
        (_, Some(to)) => format!("{name} now runs Crew {to}."),
        (_, None) => format!("{name} is running again."),
    }
}

/// A workspace the account hosts that still runs an older Crew than the one installed, and why
/// setup left it (AO-8), with the command that restarts it where one can, repeating the route
/// flags given (`route`). `look_only` is a workspace the job was not asked to restart (a check,
/// `--no-restart`, a setup that named another one, or one that named none, C3): it says what
/// updating does, never that the workspace "wasn't restarted", which only a run asked to
/// restart it and that left it running can say.
fn outdated_line(entry: &Value, server: &str, login: &str, look_only: bool, route: &str) -> String {
    let name = listed_name(entry);
    let running = str_field(entry, "running_version").map_or_else(
        || "an older Crew".to_owned(),
        |version| format!("Crew {}", safe_text(version)),
    );
    // The same version number with other bytes (a rebuilt broker) is said as the installed
    // one, never as "Crew 1.94.1, not Crew 1.94.1", as the desktop says it.
    let installed = match str_field(entry, "installed_version") {
        Some(version) if Some(version) != str_field(entry, "running_version") => {
            format!("Crew {}", safe_text(version))
        }
        Some(_) => "the Crew installed in your account".to_owned(),
        None => "the Crew installed there".to_owned(),
    };
    let setup = format!(
        "biorouter crew setup --server {}{route}",
        safe_text(&super::shell_word(login.trim()))
    );
    let restart = restart_one_command(entry, &setup);
    match str_field(entry, "reason") {
        Some("running_newer") => {
            format!("{name} runs {running}, newer than {installed}, so it was left running.")
        }
        Some("custom_state_dir") => format!(
            "{name} still runs {running}, not {installed}, and keeps its state outside ~/.local/share/biorouter-crew, so setup can't restart it. Restart it on {server} yourself."
        ),
        _ if look_only => looked_outdated_line(entry, &name, &restart),
        Some("others_connected") => {
            let who = others_connected(entry)
                .unwrap_or_else(|| "other people may be connected".to_owned());
            format!(
                "{name} still runs {running}, not {installed}, and {who} to it, so it wasn't restarted. To restart it anyway, run {restart}; their running agent commands stop."
            )
        }
        _ => format!("{name} still runs {running}, not {installed}. Run {setup} to restart it."),
    }
}

/// The command that restarts only `entry` (`setup`, then the workspace), never every outdated
/// one the account hosts as `--restart` would: by its name, else by its ID, which
/// `--restart-workspace` takes too. One with neither gets the plain update, which restarts it
/// once nobody is connected to it.
fn restart_one_command(entry: &Value, setup: &str) -> String {
    match str_field(entry, "name").or_else(|| str_field(entry, "workspace_id")) {
        Some(named) => format!(
            "{setup} --restart-workspace {}",
            safe_text(&super::shell_word(named))
        ),
        None => setup.to_owned(),
    }
}

/// Who else is connected to a hosted workspace, as the server counted them
/// (`others_connected`): one, how many, or "may be" when it did not say. `None` when it counted
/// nobody.
fn others_connected(entry: &Value) -> Option<String> {
    match entry.get("others_connected").and_then(Value::as_u64) {
        Some(0) => None,
        Some(1) => Some("1 other person is connected".to_owned()),
        Some(n) => Some(format!("{n} other people are connected")),
        None => Some("other people may be connected".to_owned()),
    }
}

/// [`outdated_line`] for a workspace the job only looked at: which Crew it runs, and the command
/// that restarts it on the installed one (`restart`), with who else is connected and what
/// stops for them (the desktop's "Update Crew restarts it." for the same row, C3).
fn looked_outdated_line(entry: &Value, name: &str, restart: &str) -> String {
    let running = str_field(entry, "running_version").map(safe_text);
    let installed = str_field(entry, "installed_version").map(safe_text);
    let same = running.is_some() && running == installed;
    let runs = match (&running, &installed) {
        (Some(running), Some(_)) if same => {
            format!("runs Crew {running}, not the Crew installed in your account")
        }
        (Some(running), Some(installed)) => format!("runs Crew {running}, not {installed}"),
        (Some(running), None) => format!("runs Crew {running}"),
        (None, _) => "runs an older Crew".to_owned(),
    };
    let on = match installed {
        Some(_) if same => "the Crew installed in your account".to_owned(),
        Some(installed) => format!("Crew {installed}"),
        None => "the Crew installed there".to_owned(),
    };
    match others_connected(entry) {
        Some(who) => format!(
            "{name} {runs}. To restart it on {on}, run {restart}; {who}, and their running agent commands stop."
        ),
        None => format!("{name} {runs}. To restart it on {on}, run {restart}."),
    }
}

/// A stage's detail, when the daemon wrote one: what it found or why it was skipped.
fn stage_detail(status: &Value, name: &str) -> Option<String> {
    list(status, "stages")
        .iter()
        .find(|stage| str_field(stage, "name") == Some(name))
        .and_then(|stage| str_field(stage, "detail"))
        .map(safe_text)
}

/// Why a job skipped the stage `name`, as the daemon said it (`host`'s Configure, AO-9).
pub fn skipped_stage_detail(status: &Value, name: &str) -> Option<String> {
    list(status, "stages")
        .iter()
        .find(|stage| {
            str_field(stage, "name") == Some(name) && str_field(stage, "state") == Some("skipped")
        })
        .and_then(|stage| str_field(stage, "detail"))
        .map(safe_text)
}

/// The version an install replaced, as the daemon reports it: `upgraded_from` (a version, or
/// `{version, sha256}` beside the new digest), else, when the replaced program was kept as
/// `biorouter-crew.previous`, the version the check found before the install (nothing checks
/// again after it).
fn setup_replaced(outcome: &Value, installed: &Value) -> Option<String> {
    let reported = match outcome.get("upgraded_from") {
        Some(Value::String(version)) => Some(version.as_str()),
        Some(previous) => str_field(previous, "version"),
        None => None,
    };
    let kept = || {
        (outcome.get("previous_kept").and_then(Value::as_bool) == Some(true))
            .then(|| str_field(installed, "version"))
            .flatten()
    };
    reported
        .or_else(kept)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(safe_text)
}

/// What a member's setup left agents able to do on `server` (OW-6, C22): what the job set
/// (`outcome.remote_access`), with the folder and whether commands run there read from the
/// connection when it was read again; a folder the job made but did not give the connection is
/// only named. Nothing when the job changed nothing about it.
fn member_access_line(outcome: &Value, connection: Option<&Value>, server: &str) -> Option<String> {
    let saved = |key: &str| connection.and_then(|connection| str_field(connection, key));
    let folder = str_field(outcome, "work_folder")
        .or_else(|| saved("remote_root"))
        .map(safe_text);
    match (str_field(outcome, "remote_access"), folder) {
        (Some("full"), Some(folder)) => Some(format!(
            "Your agent can work as you on {server}, starting in {folder}. Change that with biorouter crew connections set-access."
        )),
        (Some(_), Some(folder)) => {
            let runs = connection
                .and_then(|connection| connection.get("remote_execution"))
                .and_then(Value::as_bool)
                != Some(false);
            Some(if runs {
                format!("Agents can work in {folder} on {server}, confined to that folder. Change that with biorouter crew connections set-access.")
            } else {
                format!("Agents can read and write files in {folder} on {server}. Its kernel can't confine commands to a folder, so running them there needs full access: biorouter crew connections set-access full.")
            })
        }
        (None, Some(folder)) if str_field(outcome, "work_folder").is_some() => Some(format!(
            "Your work folder on {server} is {folder}. Choose what agents may do there with biorouter crew connections set-access."
        )),
        _ => None,
    }
}

/// How two release versions (`1.93.0`) compare, `None` when either is not one.
fn release_order(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let parts = |version: &str| {
        version
            .trim()
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    Some(parts(left)?.cmp(&parts(right)?))
}

// ---------------------------------------------------------------------------------------------
// Account ownership (design AO-5, AO-9, AO-10, AO-12)
// ---------------------------------------------------------------------------------------------

/// The desktop's button in the daemon's sentences for a lost or unrecognized device key
/// (`crew_device_key_missing`, `crew_device_not_recognized`), said as the command that does
/// the same here.
pub fn rejoin_sentence(sentence: &str) -> String {
    const DESKTOP: &str = "Choose Add this computer again";
    const TERMINAL: &str = "Run biorouter crew rejoin";
    if sentence.contains(DESKTOP) {
        sentence.replace(DESKTOP, TERMINAL)
    } else {
        format!("{} {TERMINAL}.", sentence.trim_end())
    }
}

/// Said before a connection is removed (AO-11): the chats granted through it stay bound to it,
/// and a computer that only lost its key keeps the connection with `rejoin`.
pub const REMOVAL_KEEPS_CHATS_HERE: &str = "Chats given Crew access through it can't get it through a new connection. If this computer only lost its key, run biorouter crew rejoin instead: it keeps the connection.";

/// The command that opens `name` from `login` (AO-9).
fn open_command(name: &str, login: &str) -> String {
    open_command_routed(name, login, "")
}

/// [`open_command`] with the route flags the person gave (`route`, already escaped), so it
/// reaches the server the same way.
fn open_command_routed(name: &str, login: &str, route: &str) -> String {
    format!(
        "biorouter crew open {} --server {}{route}",
        safe_text(&super::shell_word(name.trim())),
        safe_text(&super::shell_word(login.trim()))
    )
}

/// The line that points at `open` for a workspace the account hosts and runs (AO-9), with the
/// route flags the person gave (`route`, already escaped).
pub fn open_command_line(name: &str, login: &str, route: &str) -> String {
    format!("Run {}.", open_command_routed(name, login, route))
}

/// The daemon's sentence for a stopped workspace whose name is in use (`crew_host_name_in_use`),
/// with the route flags the person gave (`route`, already escaped) put after the login in its
/// `--start` command, which names the login alone. Left as it is when it holds no such command,
/// or when the daemon already wrote a route there.
pub fn with_route_in_start(sentence: &str, login: &str, route: &str) -> String {
    let server = format!("--server {}", safe_text(&super::shell_word(login.trim())));
    let plain = format!("{server} --start ");
    if route.is_empty() || !sentence.contains(&plain) {
        return sentence.to_owned();
    }
    sentence.replacen(&plain, &format!("{server}{route} --start "), 1)
}

/// What removing a connection to a server whose Crew knows the account means (AO-9): nothing
/// is lost for good, since any computer that signs in as the account opens it again, the host
/// controls included. `workspace` is already escaped for display; `name` is the workspace's own
/// name for the command.
pub fn account_removal_notice(
    host: bool,
    workspace: &str,
    name: &str,
    server: &str,
    login: &str,
) -> String {
    let open = open_command(name, login);
    if host {
        format!(
            "You host {workspace}. The host controls stay with your account on {server}: open it again from any computer that signs in as it, with {open}."
        )
    } else {
        format!(
            "You can open {workspace} again from any computer that signs in as your account on {server}, with {open}."
        )
    }
}

/// Added to the host-controls refusal on a server whose Crew predates the account path (AO-9).
pub fn update_crew_for_host_controls(server: &str, login: &str) -> String {
    format!(
        "Or first update Crew on {server} with biorouter crew setup --server {}: then the host controls follow your account there, from any computer that signs in as it.",
        safe_text(&super::shell_word(login.trim()))
    )
}

/// AO-2's sentence for an account the workspace removed (`crew_join_removed`).
pub fn removed_sentence(workspace: Option<&str>) -> String {
    let workspace = workspace
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| "this workspace".to_owned(), display_text);
    format!("You were removed from {workspace}. Ask a member to invite you again.")
}

/// "You're in lab as its host." (AO-9): the join `open` ends with, with the person's role.
pub fn joined_as_line(workspace: Option<&str>, role: &str) -> String {
    let workspace = workspace
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| "the workspace".to_owned(), display_text);
    match role {
        "host" => format!("You're in {workspace} as its host."),
        _ => format!("You're in {workspace} as a member."),
    }
}

/// What `auth --setup-job` says once the sign-in it opened ended.
pub fn setup_signed_in_line(server: &str) -> String {
    format!("Signed in to {server}. The setup goes on by itself.")
}

/// What tells one listed workspace from another: its ID, its host's UID and its key. Another
/// host or another key can present the same ID, and those are then two rows; entries that agree
/// on all three are one row, listed once.
fn row_key(workspace: &Value) -> (Option<&str>, Option<u64>, Option<&str>) {
    (
        str_field(workspace, "workspace_id"),
        workspace.get("host_uid").and_then(Value::as_u64),
        str_field(workspace, "workspace_key_fingerprint"),
    )
}

/// The workspaces a setup job listed whose name is `name`, case aside, one per row
/// ([`row_key`]).
fn named<'a>(status: &'a Value, name: &str) -> Vec<&'a Value> {
    let mut rows: Vec<&Value> = Vec::new();
    for workspace in list_key(status, "workspaces").iter().filter(|workspace| {
        str_field(workspace, "name").is_some_and(|listed| listed.eq_ignore_ascii_case(name))
    }) {
        if !rows.iter().any(|row| row_key(row) == row_key(workspace)) {
            rows.push(workspace);
        }
    }
    rows
}

/// The server's own sentence for a workspace whose ID more than one folder of the account holds
/// (`error` set to `identity_conflict: …` by the listing): nothing addressed by that ID can run
/// until the copies are moved, so it is said in place of anything else about it.
fn identity_conflict_in(workspace: &Value) -> Option<String> {
    let error = str_field(workspace, "error")?.trim();
    let sentence = error.strip_prefix("identity_conflict:")?.trim();
    (!sentence.is_empty()).then(|| safe_text(sentence))
}

/// Why a name could not be matched because nothing was listed: the job's own note.
fn unlisted(status: &Value, server: &str) -> Option<String> {
    let note = str_field(status, "workspaces_note")?;
    list_key(status, "workspaces").is_empty().then(|| {
        format!(
            "Biorouter couldn't list the workspaces on {server}. {}",
            safe_text(note)
        )
    })
}

/// The command that starts the stopped workspace `name` the account hosts on `server` (AO-8),
/// with the route flags the person gave (`route`, already escaped), so it reaches the server
/// the same way.
fn start_command(server: &str, route: &str, name: &str) -> String {
    format!(
        "biorouter crew setup --server {}{route} --start {}",
        safe_text(&super::shell_word(server)),
        safe_text(&super::shell_word(name))
    )
}

/// [`named`], else, when no workspace has that name, the ones whose ID it is: the ID is what a
/// setup summary names a workspace by when the server gave it no name (C3).
fn named_or_id<'a>(status: &'a Value, name: &str) -> Vec<&'a Value> {
    let by_name = named(status, name);
    if !by_name.is_empty() {
        return by_name;
    }
    let mut rows: Vec<&Value> = Vec::new();
    for workspace in list_key(status, "workspaces").iter().filter(|workspace| {
        str_field(workspace, "workspace_id").is_some_and(|id| id.eq_ignore_ascii_case(name))
    }) {
        if !rows.iter().any(|row| row_key(row) == row_key(workspace)) {
            rows.push(workspace);
        }
    }
    rows
}

/// The one workspace the account hosts under `name` (or with that ID, [`named_or_id`]) in a
/// check job's listing that is not deleted, for `setup --start` and `--restart-workspace`
/// (`verb`: `start` or `restart`), or the sentence that says why there is none.
fn hosted_one<'a>(
    status: &'a Value,
    name: &str,
    server: &str,
    verb: &str,
) -> Result<&'a Value, String> {
    let shown = display_text(name);
    if let Some(sentence) = unlisted(status, server) {
        return Err(sentence);
    }
    let hosted: Vec<&Value> = named_or_id(status, name)
        .into_iter()
        .filter(|workspace| str_field(workspace, "role") == Some("host"))
        .collect();
    let live: Vec<&Value> = hosted
        .iter()
        .copied()
        .filter(|workspace| str_field(workspace, "state") != Some("deleted"))
        .collect();
    match (live.as_slice(), hosted.is_empty()) {
        ([], true) => Err(format!("You don't host a workspace named {shown} on {server}.")),
        ([], false) => Err(format!(
            "{shown} on {server} was deleted, so it can't be {verb}ed."
        )),
        ([one], _) => identity_conflict_in(one).map_or(Ok(*one), Err),
        (many, _) => Err(format!(
            "You host {} workspaces named {shown} on {server}, so Biorouter can't tell which one to {verb}.",
            many.len()
        )),
    }
}

/// `setup --start NAME` (AO-8): the ID of the one stopped workspace the account hosts under
/// `name` in a check job's listing, or the sentence that says why there is none to start.
pub fn workspace_to_start(status: &Value, name: &str, server: &str) -> Result<String, String> {
    let typed = name.trim();
    let shown = display_text(typed);
    let one = hosted_one(status, typed, server, "start")?;
    match str_field(one, "state") {
        Some("stopped") => str_field(one, "workspace_id")
            .map(str::to_owned)
            .ok_or_else(|| format!("The server didn't say which workspace {shown} is.")),
        Some("running") => Err(format!(
            "{shown} is already running on {server}, so there is nothing to start."
        )),
        _ => Err(format!(
            "{shown} on {server} can't be started from here: Biorouter couldn't read whether it runs."
        )),
    }
}

/// `setup --restart-workspace NAME` (AO-8): the ID of the one running workspace the account
/// hosts under `name` in a check job's listing, or the sentence that says why there is none to
/// restart. A stopped one is pointed at `--start`, with the route flags given (`route`).
pub fn workspace_to_restart(
    status: &Value,
    name: &str,
    server: &str,
    route: &str,
) -> Result<String, String> {
    let typed = name.trim();
    let shown = display_text(typed);
    let one = hosted_one(status, typed, server, "restart")?;
    match str_field(one, "state") {
        Some("running") => str_field(one, "workspace_id")
            .map(str::to_owned)
            .ok_or_else(|| format!("The server didn't say which workspace {shown} is.")),
        Some("stopped") => Err(format!(
            "{shown} is stopped on {server}, so there is nothing to restart. Start it with {}.",
            start_command(server, route, typed)
        )),
        _ => Err(format!(
            "{shown} on {server} can't be restarted from here: Biorouter couldn't read whether it runs."
        )),
    }
}

/// The host's username a setup job listed for a workspace.
fn host_of(workspace: &Value) -> Option<String> {
    str_field(workspace, "host_username").map(str::to_owned)
}

/// `open NAME --server LOGIN` (AO-9): the one workspace a check job listed under `name` (and
/// `host`'s, when given) that can be opened, or the sentence that says why there is none. Two
/// hosts with a workspace of that name are named, and neither is chosen. Rows are told apart
/// by ID, host and key ([`row_key`]), so one workspace listed twice is still one. `route` is the
/// route flags the person gave, already escaped, which a suggested command repeats.
pub fn workspace_to_open(
    status: &Value,
    name: &str,
    host: Option<&str>,
    server: &str,
    route: &str,
) -> Result<Value, String> {
    let typed = name.trim();
    let shown = display_text(typed);
    if let Some(sentence) = unlisted(status, server) {
        return Err(sentence);
    }
    let host = host
        .map(|host| host.trim().trim_start_matches('@'))
        .filter(|host| !host.is_empty());
    let found: Vec<&Value> = named(status, typed)
        .into_iter()
        .filter(|workspace| host.is_none_or(|host| host_of(workspace).as_deref() == Some(host)))
        .collect();
    let openable: Vec<&Value> = found
        .iter()
        .copied()
        .filter(|workspace| {
            str_field(workspace, "state") != Some("deleted")
                && str_field(workspace, "role") != Some("removed")
        })
        .collect();
    let hosted_by = host
        .map(|host| format!(" hosted by @{}", safe_text(host)))
        .unwrap_or_default();
    let one = match openable.as_slice() {
        [one] => *one,
        [] => return Err(none_to_open(status, &found, typed, &hosted_by, server)),
        many => return Err(ambiguous_open(many, &shown, &hosted_by, server)),
    };
    if let Some(sentence) = identity_conflict_in(one) {
        return Err(sentence);
    }
    if str_field(one, "state") == Some("stopped") {
        return Err(if str_field(one, "role") == Some("host") {
            format!(
                "{shown} is stopped. Start it with {}, then open it.",
                start_command(server, route, typed)
            )
        } else {
            let host = host_of(one).map_or_else(
                || "its host".to_owned(),
                |host| format!("@{}", safe_text(&host)),
            );
            format!("{shown} isn't running on {server}. Ask {host} to start it.")
        });
    }
    if str_field(one, "invitation").is_none() {
        return Err(format!("The server didn't say how to open {shown}."));
    }
    Ok(one.clone())
}

/// Why [`workspace_to_open`] found nothing to open among the rows named `typed` (`found`): the
/// person was removed, the host deleted it, or nothing of that name is theirs.
fn none_to_open(
    status: &Value,
    found: &[&Value],
    typed: &str,
    hosted_by: &str,
    server: &str,
) -> String {
    let shown = display_text(typed);
    match found.first() {
        Some(workspace) if str_field(workspace, "role") == Some("removed") => {
            removed_sentence(Some(typed))
        }
        Some(_) => format!("{shown} on {server} was deleted by its host."),
        None => {
            let mut sentence = format!(
                "No workspace named {shown}{hosted_by} on {server} is yours or invites you."
            );
            if status["workspaces_scan"]["unsupported"]
                .as_u64()
                .is_some_and(|n| n > 0)
            {
                sentence.push_str(" Some workspaces there run an older Crew, which can't say whether you belong to them: open one of those with the invitation its host sends.");
            }
            sentence
        }
    }
}

/// Why [`workspace_to_open`] cannot choose among `many` rows of one name: the hosts, when there
/// are several, so `--host` chooses.
fn ambiguous_open(many: &[&Value], shown: &str, hosted_by: &str, server: &str) -> String {
    let mut hosts: Vec<String> = many
        .iter()
        .filter_map(|workspace| host_of(workspace))
        .collect();
    hosts.sort();
    hosts.dedup();
    if hosts.len() > 1 {
        format!(
            "More than one host on {server} has a workspace named {shown}: {}. Choose one with --host USERNAME.",
            hosts
                .iter()
                .map(|host| format!("@{}", safe_text(host)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(
            "There is more than one workspace named {shown}{hosted_by} on {server}, so Biorouter can't tell which one to open."
        )
    }
}

/// What `rejoin` says (AO-9), in the desktop's words (`rejoinCopy.done` and `evicted` in
/// `ui/desktop/src/components/crew/account/copy.ts`): this computer is back in the workspace and
/// the chats that had access through it need it again, the oldest computer the workspace
/// removed to make room, then the command that gives a chat access again.
pub fn rejoined_lines(answer: &Value) -> Vec<String> {
    let workspace = str_field(&answer["status"], "workspace_name")
        .map_or_else(|| "the workspace".to_owned(), display_text);
    let mut lines = vec![format!(
        "This computer is back in {workspace}. Chats that had Crew access through it need access again."
    )];
    if let Some(evicted) = answer
        .get("evicted_device")
        .filter(|device| device.is_object())
    {
        lines.push(match str_field(evicted, "fingerprint") {
            Some(fingerprint) => format!(
                "To make room, the workspace removed your oldest computer, {}.",
                safe_text(fingerprint)
            ),
            None => "To make room, the workspace removed your oldest computer.".to_owned(),
        });
    }
    lines.push(
        "Give a chat access again with biorouter crew grants grant SESSION CHANNEL.".to_owned(),
    );
    lines
}

/// The hex digits of a fingerprint as a person typed it, uppercased, or `None` when it holds
/// anything but hex digits, spaces and dashes.
fn fingerprint_hex(text: &str) -> Option<String> {
    let digits: String = text
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect();
    digits
        .chars()
        .all(|ch| ch.is_ascii_hexdigit())
        .then(|| digits.to_ascii_uppercase())
}

/// How a computer was added (`added_via`), in the desktop's words for it in its new-computer
/// notice and in Keys (`addedViaCopy` in `ui/desktop/src/components/crew/account/copy.ts`), the
/// words a chat reads too ([`biorouter::crew::added_via_words`]). A value this build does not
/// know has no words, never its raw code.
fn added_via(device: &Value) -> Option<&'static str> {
    str_field(device, "added_via").and_then(biorouter::crew::added_via_words)
}

/// Whether two device entries are the same computer, by fingerprint.
fn same_device(left: &Value, right: &Value) -> bool {
    let digits = |device: &Value| str_field(device, "fingerprint").and_then(fingerprint_hex);
    digits(left).is_some() && digits(left) == digits(right)
}

/// `profile devices` (AO-10): the computers enrolled as the person (`devices`), which one is
/// this computer, which are new since they last looked (`new_devices`), and what to do about a
/// new one that isn't theirs.
pub fn profile_device_lines(value: &Value, options: &HumanOptions) -> Vec<String> {
    let ctx = Ctx::new(options.directory.clone(), options.show_ids, options.clock);
    let workspace = str_field(value, "workspace_name")
        .map_or_else(|| "this workspace".to_owned(), display_text);
    let devices = list_key(value, "devices");
    let new = list_key(value, "new_devices");
    let mut lines = if devices.is_empty() {
        vec![format!("No computers are enrolled as you in {workspace}.")]
    } else {
        vec![format!("Computers enrolled as you in {workspace}:")]
    };
    for device in devices {
        let mut row = ctx.device_row(device);
        if new.iter().any(|added| same_device(added, device)) {
            row.push_str(" · new");
        }
        lines.push(format!("  {row}"));
    }
    if new.is_empty() {
        return lines;
    }
    if value.get("acknowledged").and_then(Value::as_bool) == Some(true) {
        lines.push(format!(
            "Marked {} as seen.",
            count(new.len(), "new computer", "new computers")
        ));
    } else {
        lines.push(format!(
            "{} added since you last looked. If one isn't yours, take its SSH access to the server away first, then remove it with biorouter crew profile revoke-device FINGERPRINT. Mark them seen with biorouter crew profile devices --acknowledge.",
            if new.len() == 1 {
                "1 computer was".to_owned()
            } else {
                format!("{} computers were", new.len())
            }
        ));
    }
    lines
}

/// `profile revoke-device` (AO-5): the device of the person's own the command names, from
/// their snapshot: this computer's (`this_device`), or the one with the fingerprint typed.
pub fn device_to_revoke(
    snapshot: &Value,
    fingerprint: &[String],
    this_device: bool,
) -> Result<Value, String> {
    let devices = list_key(&snapshot["actor"], "devices");
    if this_device {
        return devices
            .iter()
            .find(|device| device.get("this_device").and_then(Value::as_bool) == Some(true))
            .cloned()
            .ok_or_else(|| {
                "The workspace doesn't list this computer among yours, so there is nothing to remove here. biorouter crew profile devices lists them."
                    .to_owned()
            });
    }
    let typed = fingerprint.join(" ");
    let wanted = fingerprint_hex(&typed).filter(|digits| digits.len() == 16);
    let Some(wanted) = wanted else {
        return Err(
            "A fingerprint has 16 letters and digits, like 3F2A 9C1E 77B0 D4E1. biorouter crew profile devices lists yours."
                .to_owned(),
        );
    };
    devices
        .iter()
        .find(|device| {
            str_field(device, "fingerprint").and_then(fingerprint_hex).as_deref() == Some(&wanted)
        })
        .cloned()
        .ok_or_else(|| {
            format!(
                "None of your computers has the fingerprint {}. biorouter crew profile devices lists them.",
                safe_text(typed.trim())
            )
        })
}

/// The desktop's SSH-first warning before another computer is removed (`deviceCopy.sshWarning`
/// in `ui/desktop/src/components/crew/account/copy.ts`), word for word (AO-5): a computer that
/// can still sign in as the person can add itself again.
pub const REMOVE_DEVICE_SSH_FIRST: &str = "A lost computer can still sign in as you while it has an SSH key or password for your account. First remove its key from ~/.ssh/authorized_keys on the server, or change your password. Then remove it here.";

/// Said before a computer is removed (AO-5), in the desktop's words: for this computer, that it
/// stops reaching the workspace until it is added again (`deviceCopy.thisFollows`, its button
/// here the `rejoin` command); for another, that its SSH access goes first
/// ([`REMOVE_DEVICE_SSH_FIRST`]).
pub fn remove_device_warning(here: bool) -> String {
    if here {
        "This computer stops reaching the workspace until you add it again with biorouter crew rejoin."
            .to_owned()
    } else {
        REMOVE_DEVICE_SSH_FIRST.to_owned()
    }
}

/// The question `profile revoke-device` asks on a terminal, in the desktop's words
/// (`deviceCopy.confirm` and `confirmThis`).
pub fn remove_device_question(fingerprint: &str, here: bool) -> String {
    if here {
        "Remove this computer from your account?".to_owned()
    } else {
        format!("Remove {} from your account?", safe_text(fingerprint))
    }
}

/// What `profile revoke-device` says once the workspace removed the computer (AO-5).
pub fn device_revoked_lines(fingerprint: &str, here: bool, workspace: &str) -> Vec<String> {
    if here {
        vec![
            format!("Removed this computer. It can no longer use {workspace} as you."),
            "Add this computer again with biorouter crew rejoin.".to_owned(),
        ]
    } else {
        vec![format!(
            "Removed {}. That computer can no longer use {workspace} as you.",
            safe_text(fingerprint)
        )]
    }
}

/// `grants revoke SESSION --reads` once the chat's permission is gone (AO-12).
pub fn person_read_revoked_sentence(session: &str, workspace: &str) -> String {
    format!(
        "Chat {} can no longer read {workspace} as you.",
        safe_text(session)
    )
}

/// `grants revoke SESSION --reads` for a chat that had no such permission (AO-12).
pub fn no_person_read_sentence(session: &str, workspace: &str) -> String {
    format!(
        "Chat {} had no permission to read {workspace} as you.",
        safe_text(session)
    )
}

/// Markdown text as it is printed, line by line, each made terminal-safe.
pub fn markdown_lines(text: &str) -> Vec<String> {
    body_lines(text)
}

/// The workspace's context document (OW-8), `{text, revision, updated_by, updated_at}` or
/// `null`: the text, then who changed it last and when.
pub fn workspace_context_lines(context: &Value, options: &HumanOptions) -> Vec<String> {
    let Some(text) = str_field(context, "text") else {
        return vec![
            "This workspace has no context document yet.".to_owned(),
            "Write one with biorouter crew workspace context set FILE, or draft one from the server with biorouter crew workspace context draft --save.".to_owned(),
        ];
    };
    let mut directory = options.directory.clone();
    directory.absorb(context);
    let ctx = Ctx::new(directory, options.show_ids, options.clock);
    let mut lines = body_lines(text);
    if lines.is_empty() {
        lines.push("(The context document is empty.)".to_owned());
    }
    let mut about = Vec::new();
    if let Some(revision) = context.get("revision").and_then(Value::as_u64) {
        about.push(format!("Revision {revision}"));
    }
    if let Some(editor) = str_field(context, "updated_by") {
        about.push(format!("changed by {}", ctx.you_or_person(Some(editor))));
    }
    if let Some(at) = context.get("updated_at").and_then(Value::as_i64) {
        about.push(ctx.when(at));
    }
    if !about.is_empty() {
        lines.push(String::new());
        lines.push(about.join(" · "));
    }
    lines
}

/// `files list` (L8): `{channel_id, files: [{attachment_id, name, size, media_type, shared_by,
/// run_id, shared_at, message_id}], unavailable, complete}`, one line a file, newest first:
/// its name and size, who shared it and when, and always its attachment ID, which `files
/// download` takes. Then what the listing could not show.
pub fn channel_files_lines(listing: &Value, options: &HumanOptions) -> Vec<String> {
    let ctx = Ctx::new(options.directory.clone(), options.show_ids, options.clock);
    let files = list(listing, "files");
    let channel = ctx.channel(str_field(listing, "channel_id"));
    let mut lines: Vec<String> = files
        .iter()
        .map(|file| {
            let mut name = text_or(file, "name", "Unnamed file");
            if let Some(size) = file.get("size").and_then(Value::as_u64) {
                let _ = write!(name, " ({})", human_size(size));
            }
            let mut parts = vec![name];
            let by = str_field(file, "shared_by");
            parts.push(match str_field(file, "run_id") {
                None => format!("shared by {}", ctx.you_or_person(by)),
                Some(_) if ctx.is_actor(by) => "shared by your agent".to_owned(),
                Some(_) => format!("shared by the agent of {}", ctx.person(by)),
            });
            if let Some(at) = file.get("shared_at").and_then(Value::as_i64) {
                parts.push(ctx.when(at));
            }
            if let Some(id) = str_field(file, "attachment_id") {
                parts.push(format!("attachment ID {}", safe_text(id)));
            }
            parts.join(" · ")
        })
        .collect();
    if lines.is_empty() {
        lines.push(format!("No files were shared in {channel}."));
    }
    let unavailable = listing
        .get("unavailable")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if unavailable > 0 {
        let unavailable = usize::try_from(unavailable).unwrap_or(usize::MAX);
        lines.push(format!(
            "{} shared there can't be opened any more.",
            count(unavailable, "more file", "more files")
        ));
    }
    if listing.get("complete").and_then(Value::as_bool) == Some(false) {
        lines.push(
            "Only the channel's newest messages were read, so older files may not be listed."
                .to_owned(),
        );
    }
    if !files.is_empty() {
        lines.push(
            "Download one with biorouter crew files download ATTACHMENT_ID --output FILE."
                .to_owned(),
        );
    }
    lines
}

/// How text output is rendered.
#[derive(Clone, Debug, Default)]
pub struct HumanOptions {
    /// Append machine IDs (people, teams, channels, messages, tasks, transfers …) to text.
    pub show_ids: bool,
    /// Which formatter to use; [`View::Auto`] recognizes the value by its shape.
    pub view: View,
    /// Names known from elsewhere, such as the workspace snapshot behind a history page.
    pub directory: Directory,
    /// Lines printed after the value in text, such as that a list is not complete.
    pub notes: Vec<String>,
    clock: Clock,
}

#[allow(dead_code)] // The command layer builds these for `--show-ids` and snapshot-backed views.
impl HumanOptions {
    pub fn new(show_ids: bool) -> Self {
        Self {
            show_ids,
            ..Self::default()
        }
    }

    pub fn with_view(mut self, view: View) -> Self {
        self.view = view;
        self
    }

    pub fn with_directory(mut self, directory: Directory) -> Self {
        self.directory = directory;
        self
    }

    pub fn with_notes(mut self, notes: impl IntoIterator<Item = String>) -> Self {
        self.notes.extend(notes);
        self
    }

    /// Dates in UTC with a fixed "now", so a test reads the same in every time zone.
    #[cfg(test)]
    pub fn in_utc_at(now: i64) -> Self {
        Self {
            clock: Clock::Fixed {
                now,
                offset_seconds: 0,
            },
            ..Self::default()
        }
    }
}

/// The kind of value being printed, when the caller knows it better than its shape does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Auto,
    Snapshot,
    Members,
    Profile,
    Teams,
    Channels,
    Invitations,
    Connections,
    Connection,
    Messages,
    Message,
    Tasks,
    Task,
    Grants,
    Privacy,
    Transfers,
    Transfer,
    Reference,
    Attachment,
    Workspace,
    /// A mutation result or anything else: recognized results get a sentence, the rest a
    /// field list with ID-like fields left out.
    Result,
}

#[derive(Clone, Copy, Debug, Default)]
enum Clock {
    #[default]
    System,
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed { now: i64, offset_seconds: i32 },
}

impl Clock {
    fn now(self) -> i64 {
        match self {
            Self::System => Utc::now().timestamp(),
            Self::Fixed { now, .. } => now,
        }
    }

    fn local(self, epoch: i64) -> Option<DateTime<FixedOffset>> {
        match self {
            Self::System => Local
                .timestamp_opt(epoch, 0)
                .single()
                .map(|time| time.fixed_offset()),
            Self::Fixed { offset_seconds, .. } => FixedOffset::east_opt(offset_seconds)?
                .timestamp_opt(epoch, 0)
                .single(),
        }
    }
}

/// Names for the IDs a value refers to, learned from snapshots and message pages.
#[derive(Clone, Debug, Default)]
pub struct Directory {
    actor_id: Option<String>,
    host_principal_id: Option<String>,
    host_uid: Option<u64>,
    workspace_name: Option<String>,
    people: BTreeMap<String, Person>,
    teams: BTreeMap<String, String>,
    channels: BTreeMap<String, ChannelInfo>,
    references: BTreeMap<String, (String, String)>,
    unread: BTreeMap<String, u64>,
    /// The people the workspace reports online.
    online: std::collections::BTreeSet<String>,
    /// Why this directory could not read the workspace's names, and which workspace: an ID it
    /// cannot name is then said as that, with its ID, rather than as "this channel" (R-8,
    /// AG-F17).
    unavailable: Option<Unavailable>,
}

#[derive(Clone, Debug)]
struct Unavailable {
    workspace: String,
    disconnected: bool,
}

#[derive(Clone, Debug)]
struct Person {
    username: Option<String>,
    display_name: Option<String>,
    uid: Option<u64>,
    active: bool,
    stale: bool,
}

#[derive(Clone, Debug, Default)]
struct ChannelInfo {
    name: Option<String>,
    team_id: Option<String>,
    classification: Option<String>,
}

impl Directory {
    /// No names, because the workspace snapshot could not be read. `workspace` is what the
    /// person calls the workspace (the saved connection's name, unescaped); `disconnected` says
    /// the connection is down, which is the usual reason.
    pub fn names_unavailable(workspace: &str, disconnected: bool) -> Self {
        Self {
            unavailable: Some(Unavailable {
                workspace: workspace.to_owned(),
                disconnected,
            }),
            ..Self::default()
        }
    }

    /// "a channel in lab (names unavailable while disconnected)", for an ID this directory
    /// cannot name because the names could not be read.
    fn unnamed(&self, what: &str) -> Option<String> {
        let unavailable = self.unavailable.as_ref()?;
        let why = if unavailable.disconnected {
            "names unavailable while disconnected"
        } else {
            "names unavailable"
        };
        Some(format!(
            "{what} in {} ({why})",
            display_text(&unavailable.workspace)
        ))
    }

    #[allow(dead_code)] // The command layer passes a snapshot's names to history and lists.
    pub fn from_snapshot(snapshot: &Value) -> Self {
        let mut directory = Self::default();
        directory.absorb(snapshot);
        directory
    }

    /// Whether the workspace reports this person online (`online_principal_ids`, W2-BRK-6).
    /// Presence is shown only when the broker reports it: an older one names no one.
    fn online(&self, id: &str) -> bool {
        self.online.contains(id)
    }

    /// A channel's `#name` for a sentence, or "a channel" when this directory can't name it
    /// (never its ID).
    pub fn channel_label(&self, id: &str) -> String {
        self.channels
            .get(id)
            .and_then(|channel| channel.name.as_deref())
            .filter(|name| !name.is_empty())
            .map_or_else(|| "a channel".to_owned(), channel_name)
    }

    /// Learn names from any daemon or broker value: a workspace snapshot, a message page's
    /// `people` and `channel_names` maps, or a bare list of principals, teams or channels.
    pub fn absorb(&mut self, value: &Value) {
        match value {
            Value::Array(items) => items.iter().for_each(|item| self.absorb_item(item)),
            Value::Object(fields) => self.absorb_fields(fields, value),
            _ => {}
        }
    }

    fn absorb_fields(&mut self, fields: &Map<String, Value>, value: &Value) {
        if let Some(workspace) = fields.get("workspace").filter(|value| value.is_object()) {
            self.absorb_workspace(workspace);
        }
        if let Some(actor) = fields.get("actor").filter(|value| value.is_object()) {
            if let Some(id) = str_field(actor, "id") {
                self.actor_id = Some(id.to_owned());
            }
            self.absorb_item(actor);
        }
        for key in [
            "principals",
            "former_principals",
            "teams",
            "channels",
            "references",
            "invitations",
        ] {
            for item in list_key(value, key) {
                self.absorb_item(item);
            }
        }
        if let Some(Value::Object(people)) = fields.get("people") {
            for (id, person) in people {
                self.add_person(id, person_from(person), true);
            }
        }
        if let Some(Value::Object(names)) = fields.get("channel_names") {
            for (id, name) in names {
                let channel = self.channels.entry(id.clone()).or_default();
                if channel.name.is_none() {
                    channel.name = name.as_str().map(str::to_owned);
                }
            }
        }
        if let Some(Value::Array(online)) = fields.get("online_principal_ids") {
            self.online
                .extend(online.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        if let Some(Value::Object(unread)) = fields.get("unread") {
            for (id, count) in unread {
                if let Some(count) = count.as_u64() {
                    self.unread.insert(id.clone(), count);
                }
            }
        }
        self.absorb_item(value);
    }

    fn absorb_workspace(&mut self, workspace: &Value) {
        if let Some(name) = str_field(workspace, "name") {
            self.workspace_name = Some(name.to_owned());
        }
        if let Some(host) = str_field(workspace, "host_principal_id") {
            self.host_principal_id = Some(host.to_owned());
        }
        if let Some(uid) = workspace.get("host_uid").and_then(Value::as_u64) {
            self.host_uid = Some(uid);
        }
    }

    fn absorb_item(&mut self, item: &Value) {
        // An invitation's enrichment names its inviter; a snapshot's own principals, read
        // first, win over it.
        let inviter = item.get("inviter").filter(|value| value.is_object());
        if let (Some(id), Some(inviter)) = (str_field(item, "inviter_id"), inviter) {
            self.add_person(id, person_from(inviter), false);
        }
        let Some(id) = str_field(item, "id") else {
            return;
        };
        let has = |key: &str| item.get(key).is_some();
        if has("username") {
            self.add_person(id, person_from(item), true);
        } else if has("general_channel_id") {
            if let Some(name) = display_or_name(item) {
                self.teams.insert(id.to_owned(), name.to_owned());
            }
        } else if has("team_id") && has("classification") {
            let channel = ChannelInfo {
                name: display_or_name(item).map(str::to_owned),
                team_id: str_field(item, "team_id").map(str::to_owned),
                classification: str_field(item, "classification").map(str::to_owned),
            };
            self.channels.insert(id.to_owned(), channel);
        } else if let (Some(label), Some(path)) =
            (str_field(item, "label"), str_field(item, "path"))
        {
            self.references
                .insert(id.to_owned(), (label.to_owned(), path.to_owned()));
        }
    }

    fn add_person(&mut self, id: &str, person: Person, authoritative: bool) {
        if authoritative || !self.people.contains_key(id) {
            self.people.insert(id.to_owned(), person);
        }
    }

    /// The active member a former principal's account is now, when there is one: the same
    /// username, and the same UID where both are known. A member who was removed and joined
    /// again, or whose earlier entry the workspace replaced, has a new principal; what they
    /// wrote under the old one is theirs, so it is shown as that member, not as a former
    /// member (live finding). `None` for an active principal, or one nobody holds now.
    fn now_member(&self, person: &Person) -> Option<&Person> {
        if person.active {
            return None;
        }
        let username = person.username.as_deref()?;
        self.people.values().find(|other| {
            other.active
                && other.username.as_deref() == Some(username)
                && (other.uid.is_none() || person.uid.is_none() || other.uid == person.uid)
        })
    }

    /// How `person` reads in a line: as the member their account is now ([`Self::now_member`]),
    /// else as themselves.
    fn person_label(&self, person: &Person) -> String {
        self.now_member(person).unwrap_or(person).label()
    }

    /// The host: projected by newer brokers, else the active principal holding the host's UID
    /// (one active principal per UID, so the match is unique).
    fn host_id(&self) -> Option<&str> {
        if let Some(host) = self.host_principal_id.as_deref() {
            return Some(host);
        }
        let uid = self.host_uid?;
        self.people
            .iter()
            .find(|(_, person)| person.active && person.uid == Some(uid))
            .map(|(id, _)| id.as_str())
    }
}

fn person_from(value: &Value) -> Person {
    Person {
        username: str_field(value, "username").map(str::to_owned),
        display_name: str_field(value, "display_name")
            .or_else(|| str_field(value, "nickname"))
            .map(str::to_owned),
        uid: value.get("uid").and_then(Value::as_u64),
        active: value.get("active").and_then(Value::as_bool).unwrap_or(true),
        stale: value.get("account_stale").and_then(Value::as_bool) == Some(true),
    }
}

impl Person {
    /// `"Display name" (@username)`, or `@username` when the two are equal (or there is no
    /// display name). The username is on every label, so a decision about a person names them
    /// exactly without repeating it as a quoted "display name" (Q3-63: `Removed "crew_frank"
    /// (@crew_frank)`, with its isolates printed raw).
    fn label(&self) -> String {
        let Some(username) = self.username.as_deref() else {
            return if self.active {
                UNKNOWN_MEMBER
            } else {
                FORMER_MEMBER
            }
            .to_owned();
        };
        let display = self
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|display| !display.is_empty());
        let mut label = match display {
            Some(display) if display.to_lowercase() != username.to_lowercase() => {
                format!("{} (@{})", quoted_name(display), safe_text(username))
            }
            _ => format!("@{}", safe_text(username)),
        };
        if !self.active {
            label.push_str(" · former member");
        }
        label
    }
}

/// A display name quoted like a string literal, escaped for the terminal, and isolated when it
/// could hold right-to-left text, so its direction cannot leak into the text around it.
///
/// A name that is ASCII once escaped holds no right-to-left text, so it is left bare, as
/// [`display_text`] leaves a team or file name (F10): the isolates are invisible in a terminal
/// but travel into anything copied, so `"Alice Chen"` failed a search for that very text.
/// Escaping comes first, so a name's own bidi controls are printed as `\u{…}` text and cannot
/// make it look ASCII.
fn quoted_name(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '"' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    let text = safe_text(&escaped);
    if text.is_ascii() {
        format!("\"{text}\"")
    } else {
        format!("\"\u{2068}{text}\u{2069}\"")
    }
}

/// A team name or file name: escaped, and isolated when it could hold right-to-left text.
/// ASCII cannot, and leaving it bare keeps a copied name free of invisible characters.
fn display_text(name: &str) -> String {
    let text = safe_text(name.trim());
    if text.is_ascii() {
        text
    } else {
        format!("\u{2068}{text}\u{2069}")
    }
}

/// A channel as a sentence names it when another team of the person's may have one of the
/// same name (M11): `Chen Lab / #general` when another listed team has a channel of that name,
/// else `#general`. `None` when the directory cannot name the channel.
pub fn channel_across_teams(directory: &Directory, id: &str) -> Option<String> {
    let channel = directory.channels.get(id)?;
    let name = channel.name.as_deref().filter(|name| !name.is_empty())?;
    let shared = directory.channels.iter().any(|(other, info)| {
        other != id && info.name.as_deref() == Some(name) && info.team_id != channel.team_id
    });
    let team = channel
        .team_id
        .as_deref()
        .and_then(|team| directory.teams.get(team))
        .filter(|_| shared);
    Some(match team {
        Some(team) => format!("{} / {}", display_text(team), channel_name(name)),
        None => channel_name(name),
    })
}

/// `"Bob Lee" (@bob)`, or `@bob` when the display name is the username.
#[allow(dead_code)] // For the command layer's confirmations and refusals.
pub fn person_label(directory: &Directory, principal_id: &str, show_ids: bool) -> String {
    Ctx::new(directory.clone(), show_ids, Clock::System).person(Some(principal_id))
}

/// The person a decision is about (remove, offer, revoke): `"Display name" (@username)` for a
/// real display name, `@username` alone when the display name is the username (Q3-63), with
/// the ID when asked.
#[allow(dead_code)] // For the command layer's confirmations and refusals.
pub fn authority_label(directory: &Directory, principal_id: &str, show_ids: bool) -> String {
    let label = directory.people.get(principal_id).map_or_else(
        || UNKNOWN_MEMBER.to_owned(),
        |person| directory.person_label(person),
    );
    Ctx::new(directory.clone(), show_ids, Clock::System).with_id(label, "ID", Some(principal_id))
}

/// One computer enrolled as the person, as their profile lists it: its fingerprint, the date
/// it was added and how.
pub fn device_text(device: &Value, options: &HumanOptions) -> String {
    Ctx::new(options.directory.clone(), options.show_ids, options.clock).device_row(device)
}

/// A chat's Crew access for a person (AG-F7): `Access: #methods · also reads #general`, then
/// the messages it can read, oldest first (the manifest lists them newest first).
/// `destination` is the channel the chat posts in, when the grants list says; without it the
/// line names every channel the chat reads.
pub fn context_lines(
    manifest: &Value,
    destination: Option<&str>,
    options: &HumanOptions,
) -> Vec<String> {
    let mut directory = options.directory.clone();
    directory.absorb(manifest);
    let ctx = Ctx::new(directory, options.show_ids, options.clock);
    let sources = str_list(manifest, "source_channels");
    let others: Vec<String> = sources
        .iter()
        .filter(|source| Some(**source) != destination)
        .map(|source| ctx.channel_in_team(Some(source)))
        .collect();
    let mut out = vec![match destination {
        Some(destination) if others.is_empty() => {
            format!("Access: {}", ctx.channel_in_team(Some(destination)))
        }
        Some(destination) => format!(
            "Access: {} · also reads {}",
            ctx.channel_in_team(Some(destination)),
            others.join(", ")
        ),
        None if others.is_empty() => "Access: no channels".to_owned(),
        None => format!("Access: reads {}", others.join(", ")),
    }];
    let messages = list_key(manifest, "messages");
    if messages.is_empty() {
        out.push("No messages.".into());
    }
    for message in messages.iter().rev() {
        out.extend(ctx.message(message));
    }
    out
}

/// Render `value` as text for a person.
pub fn render_text(value: &Value, options: &HumanOptions) -> String {
    let mut directory = options.directory.clone();
    directory.absorb(value);
    let ctx = Ctx::new(directory, options.show_ids, options.clock);
    let view = match options.view {
        View::Auto => detect(value),
        view => view,
    };
    let mut lines = ctx.render(view, value);
    lines.extend(options.notes.iter().cloned());
    lines.join("\n")
}

/// "Showing 100 of your 140 channels." when a snapshot left some out (`totals`): the broker
/// lists as many teams and channels as fit in one answer, and the rest are still the person's.
pub fn partial_list_note(noun: &str, listed: usize, total: Option<u64>) -> Option<String> {
    let total = usize::try_from(total?).ok()?;
    (total > listed).then(|| format!("Showing {listed} of your {total} {noun}."))
}

/// Channels as a person reads them (M17, F6, SF-F10): grouped by team, teams by name, each
/// team's `#general` first and then its channels by name. A channel whose team is not listed
/// comes last. The broker sends them in the order of their random IDs.
pub fn channels_in_order(channels: &mut [Value], teams: &[Value]) {
    let team_rank: BTreeMap<&str, (String, &str)> = teams
        .iter()
        .filter_map(|team| {
            let id = str_field(team, "id")?;
            let name = display_or_name(team).unwrap_or_default().to_lowercase();
            Some((
                id,
                (
                    name,
                    str_field(team, "general_channel_id").unwrap_or_default(),
                ),
            ))
        })
        .collect();
    let key = |channel: &Value| {
        let id = str_field(channel, "id").unwrap_or_default().to_owned();
        let team = str_field(channel, "team_id").and_then(|team| team_rank.get(team));
        let name = display_or_name(channel)
            .unwrap_or_default()
            .trim_start_matches('#')
            .to_lowercase();
        (
            team.is_none(),
            team.map(|(name, _)| name.clone()).unwrap_or_default(),
            str_field(channel, "team_id").unwrap_or_default().to_owned(),
            team.is_none_or(|(_, general)| *general != id),
            name,
            id,
        )
    };
    channels.sort_by_cached_key(key);
}

/// Teams by name, case aside.
pub fn teams_in_order(teams: &mut [Value]) {
    teams.sort_by_cached_key(|team| {
        (
            display_or_name(team).unwrap_or_default().to_lowercase(),
            str_field(team, "id").unwrap_or_default().to_owned(),
        )
    });
}

/// People as a person reads them (SF-F10, M17): the host, then you, then everyone else by the
/// name their row leads with, a leading `@` and letter case aside, then by username. The
/// desktop's `peopleInOrder`, so the terminal lists a team the way the app does.
pub fn people_in_order(people: &mut [Value], host_id: Option<&str>, actor_id: Option<&str>) {
    people.sort_by_cached_key(|person| {
        let id = str_field(person, "id");
        let rank = if id.is_some() && id == host_id {
            0
        } else if id.is_some() && id == actor_id {
            1
        } else {
            2
        };
        let username = str_field(person, "username").unwrap_or_default();
        let display = str_field(person, "display_name")
            .or_else(|| str_field(person, "nickname"))
            .map(str::trim)
            .filter(|display| {
                !display.is_empty() && display.to_lowercase() != username.to_lowercase()
            });
        let shown = display
            .unwrap_or(username)
            .trim_start_matches('@')
            .to_lowercase();
        (rank, shown, username.to_lowercase(), username.to_owned())
    });
}

fn detect(value: &Value) -> View {
    match value {
        Value::Array(items) => items
            .first()
            .map_or(View::Result, |item| plural(detect_item(item))),
        Value::Object(fields) => {
            let array = |key: &str| matches!(fields.get(key), Some(Value::Array(_)));
            let object = |key: &str| matches!(fields.get(key), Some(Value::Object(_)));
            if array("messages") {
                View::Messages
            } else if object("workspace") && array("teams") && array("channels") {
                View::Snapshot
            } else if fields.contains_key("personal_mode") && object("workspace") {
                View::Privacy
            } else if array("connections") {
                View::Connections
            } else if array("runs") {
                View::Tasks
            } else if array("grants") {
                View::Grants
            } else if array("transfers") {
                View::Transfers
            } else {
                detect_item(value)
            }
        }
        _ => View::Result,
    }
}

fn detect_item(value: &Value) -> View {
    let has = |key: &str| value.get(key).is_some();
    if has("body") && has("actor_id") {
        View::Message
    } else if has("ssh_target") && has("workspace_id") {
        View::Connection
    } else if has("username") && has("id") {
        View::Profile
    } else if has("general_channel_id") {
        View::Teams
    } else if has("team_id") && has("classification") {
        View::Channels
    } else if has("kind") && has("target_id") && has("inviter_id") {
        View::Invitations
    } else if has("run_id") && has("status") && has("session_id") {
        View::Task
    } else if has("session_id") && has("channel_id") && has("expired") {
        View::Grants
    } else if has("direction") && has("state") {
        View::Transfer
    } else if has("path") && has("label") && has("channel_id") {
        View::Reference
    } else if has("media_type") && has("size") && has("channel_id") {
        View::Attachment
    } else if has("host_uid") && has("mode") && has("policy_epoch") {
        View::Workspace
    } else {
        View::Result
    }
}

fn plural(view: View) -> View {
    match view {
        View::Profile => View::Members,
        View::Message => View::Messages,
        View::Connection => View::Connections,
        View::Task => View::Tasks,
        View::Transfer => View::Transfers,
        other => other,
    }
}

type Row = fn(&Ctx, &Value) -> Vec<String>;

struct Ctx {
    dir: Directory,
    show_ids: bool,
    clock: Clock,
}

impl Ctx {
    fn new(dir: Directory, show_ids: bool, clock: Clock) -> Self {
        Self {
            dir,
            show_ids,
            clock,
        }
    }

    fn render(&self, view: View, value: &Value) -> Vec<String> {
        match view {
            View::Auto => self.render(detect(value), value),
            View::Snapshot => self.snapshot(value),
            View::Members => self.rows(value, "principals", "No people.", Self::member_row),
            View::Profile => self.profile(value),
            View::Teams => self.rows(value, "teams", "No teams.", Self::team_row),
            View::Channels => self.rows(value, "channels", "No channels.", Self::channel_row),
            View::Invitations => self.invitations(value),
            View::Connections => self.rows(
                value,
                "connections",
                "No saved connections.",
                Self::connection_row,
            ),
            View::Connection => self.connection(value),
            View::Messages => self.messages(value),
            View::Message => self.message(value),
            View::Tasks => self.tasks(value),
            View::Task => self.task(value),
            View::Grants => self.grants(value),
            View::Privacy => self.privacy(value),
            View::Transfers => self.transfers(value),
            View::Transfer => self.transfer(value, true),
            View::Reference => self.rows(
                value,
                "references",
                "No remote references.",
                Self::reference_row,
            ),
            View::Attachment => self.rows(value, "attachments", "No files.", Self::attachment_row),
            View::Workspace => vec![self.with_id(
                format!("Workspace privacy: {}", workspace_privacy(value)),
                "workspace ID",
                str_field(value, "id"),
            )],
            View::Result => self.result(value),
        }
    }

    fn rows(&self, value: &Value, key: &str, empty: &str, row: Row) -> Vec<String> {
        let items = list(value, key);
        if items.is_empty() {
            return vec![empty.to_owned()];
        }
        items.iter().flat_map(|item| row(self, item)).collect()
    }

    fn with_id(&self, text: String, what: &str, id: Option<&str>) -> String {
        match id.filter(|_| self.show_ids) {
            Some(id) => format!("{text} [{what} {}]", safe_text(id)),
            None => text,
        }
    }

    fn ids_hint(&self, out: &mut Vec<String>, commands: &str) {
        if !self.show_ids {
            out.push(format!("Add --show-ids for the IDs {commands} take."));
        }
    }

    fn is_actor(&self, id: Option<&str>) -> bool {
        id.is_some() && id == self.dir.actor_id.as_deref()
    }

    fn person(&self, id: Option<&str>) -> String {
        let label = id.and_then(|id| self.dir.people.get(id)).map_or_else(
            || UNKNOWN_MEMBER.to_owned(),
            |person| self.dir.person_label(person),
        );
        self.with_id(label, "ID", id)
    }

    fn you_or_person(&self, id: Option<&str>) -> String {
        if self.is_actor(id) {
            self.with_id("you".into(), "ID", id)
        } else {
            self.person(id)
        }
    }

    fn team(&self, id: Option<&str>) -> String {
        match id.and_then(|id| self.dir.teams.get(id)) {
            Some(name) => self.with_id(display_text(name), "team ID", id),
            None => self.unnamed_id("a team", "this team", "team ID", id),
        }
    }

    fn channel(&self, id: Option<&str>) -> String {
        let name = id
            .and_then(|id| self.dir.channels.get(id))
            .and_then(|channel| channel.name.as_deref())
            .filter(|name| !name.is_empty());
        match name {
            Some(name) => self.with_id(channel_name(name), "channel ID", id),
            None => self.unnamed_id("a channel", "this channel", "channel ID", id),
        }
    }

    /// An ID the directory cannot name: `fallback` ("this channel") when the value simply
    /// does not say, and, when the names could not be read, what it is, where, why, and its
    /// ID, which is then the only way to tell two rows apart (R-8, AG-F17).
    fn unnamed_id(&self, what: &str, fallback: &str, label: &str, id: Option<&str>) -> String {
        match (self.dir.unnamed(what), id) {
            (Some(unnamed), Some(id)) => format!("{unnamed} [{label} {}]", safe_text(id)),
            _ => self.with_id(fallback.to_owned(), label, id),
        }
    }

    fn channel_in_team(&self, id: Option<&str>) -> String {
        let team = id
            .and_then(|id| self.dir.channels.get(id))
            .and_then(|channel| channel.team_id.as_deref())
            .filter(|team| self.dir.teams.contains_key(*team));
        match team {
            Some(team) => format!("{} in {}", self.channel(id), self.team(Some(team))),
            None => self.channel(id),
        }
    }

    fn when(&self, epoch: i64) -> String {
        let Some(time) = self.clock.local(epoch) else {
            return "unknown time".into();
        };
        let pattern = match self.clock.local(self.clock.now()) {
            Some(now) if now.date_naive() == time.date_naive() => "%H:%M",
            Some(now) if now.year() == time.year() => "%b %-d %H:%M",
            _ => "%b %-d, %Y %H:%M",
        };
        time.format(pattern).to_string()
    }

    fn date(&self, epoch: i64) -> String {
        self.clock.local(epoch).map_or_else(
            || "unknown date".into(),
            |time| time.format("%b %-d, %Y").to_string(),
        )
    }

    fn expiry(&self, value: &Value) -> String {
        let expired = value.get("expired").and_then(Value::as_bool) == Some(true);
        match value.get("expires_at").and_then(Value::as_i64) {
            _ if expired => "expired".into(),
            Some(at) if at > self.clock.now() => {
                format!("expires {}", within(at - self.clock.now()))
            }
            Some(_) => "expired".into(),
            None => "no expiry shown".into(),
        }
    }

    fn snapshot(&self, snapshot: &Value) -> Vec<String> {
        let mut out = self.workspace_lines(snapshot.get("workspace").unwrap_or(&Value::Null));
        if let Some(actor) = self.dir.actor_id.as_deref() {
            out.push(format!("You: {}", self.person(Some(actor))));
        }
        let sections: [(&str, &str, Row); 7] = [
            ("People", "principals", Self::member_row),
            ("Former members", "former_principals", Self::member_row),
            ("Teams", "teams", Self::team_row),
            ("Channels", "channels", Self::channel_row),
            ("Invitations", "invitations", Self::invitation_row),
            ("Remote references", "references", Self::reference_row),
            ("Agent grants", "runs", Self::run_grant_row),
        ];
        for (title, key, row) in sections {
            let mut items = list_key(snapshot, key).to_vec();
            // What the workspace sent, which its totals count, before any entry is folded.
            let listed = items.len();
            let noun = match key {
                "principals" => {
                    people_in_order(&mut items, self.dir.host_id(), self.dir.actor_id.as_deref());
                    ""
                }
                "former_principals" => {
                    people_in_order(&mut items, self.dir.host_id(), self.dir.actor_id.as_deref());
                    self.one_line_per_account(&mut items);
                    ""
                }
                "teams" => {
                    teams_in_order(&mut items);
                    "teams"
                }
                "channels" => {
                    channels_in_order(&mut items, list_key(snapshot, "teams"));
                    "channels"
                }
                "invitations" => "invitations",
                "references" => "remote references",
                _ => "agent grants",
            };
            let note = partial_list_note(noun, listed, snapshot["totals"][key].as_u64());
            if items.is_empty() && note.is_none() {
                if matches!(title, "People" | "Teams" | "Channels") {
                    out.push(format!("{title}: none"));
                }
                continue;
            }
            out.push(format!("{title} ({}):", items.len()));
            for item in &items {
                out.extend(row(self, item).into_iter().map(|line| format!("  {line}")));
            }
            out.extend(note.map(|note| format!("  {note}")));
            if key == "former_principals" {
                out.extend(
                    self.alike_former_note(&items)
                        .map(|note| format!("  {note}")),
                );
            }
        }
        out
    }

    fn workspace_lines(&self, workspace: &Value) -> Vec<String> {
        let host = self.dir.host_id();
        let mut head = match (self.dir.workspace_name.as_deref(), host) {
            (Some(name), Some(host)) => format!(
                "Workspace: {} · hosted by {}",
                safe_text(name),
                self.person(Some(host))
            ),
            (Some(name), None) => format!("Workspace: {}", safe_text(name)),
            (None, Some(host)) => format!("Workspace: {}'s workspace", self.person(Some(host))),
            (None, None) => "Workspace: this workspace".into(),
        };
        head = self.with_id(head, "workspace ID", str_field(workspace, "id"));
        vec![head, format!("Privacy: {}", workspace_privacy(workspace))]
    }

    fn member_row(&self, principal: &Value) -> Vec<String> {
        let id = str_field(principal, "id");
        let person = id
            .and_then(|id| self.dir.people.get(id))
            .cloned()
            .unwrap_or_else(|| person_from(principal));
        // An earlier entry of someone who is a member again is listed as that member.
        let mut row = match self.dir.now_member(&person) {
            Some(member) => format!("{} · a member again", member.label()),
            None => person.label(),
        };
        if self.is_actor(id) {
            row.push_str(" · you");
        }
        if id.is_some() && id == self.dir.host_id() {
            row.push_str(" · host");
        }
        if person.stale {
            row.push_str(" · account no longer valid");
        }
        if id.is_some_and(|id| self.dir.online(id)) {
            row.push_str(" · online");
        }
        row = self.with_id(row, "ID", id);
        if let Some(uid) = person.uid.filter(|_| self.show_ids) {
            let _ = write!(row, " [UID {uid}]");
        }
        vec![row]
    }

    /// Former members as one line per account (live finding): an account removed and let in
    /// again twice left two earlier entries, and both read "@gbellucci · a member again". The
    /// earlier entries of a member again are one line, that member's
    /// ([`Directory::now_member`]). Every other entry keeps its own line: the broker sends no UID
    /// for a former member, so two entries of one username may be two accounts (one deleted on
    /// the server and a new one made with its name), and folding them would hide one.
    fn one_line_per_account(&self, former: &mut Vec<Value>) {
        let mut members = std::collections::BTreeSet::new();
        former.retain(
            |principal| match self.dir.now_member(&self.listed_person(principal)) {
                Some(member) => members.insert((member.username.clone(), member.uid)),
                None => true,
            },
        );
    }

    /// Said under the former members when two of their lines read alike: entries of one
    /// username that no member holds now, which only their IDs tell apart.
    fn alike_former_note(&self, former: &[Value]) -> Option<String> {
        if self.show_ids {
            return None;
        }
        let mut seen = std::collections::BTreeSet::new();
        former
            .iter()
            .map(|principal| self.listed_person(principal))
            .filter(|person| self.dir.now_member(person).is_none())
            .filter_map(|person| person.username)
            .any(|username| !seen.insert(username))
            .then(|| {
                "Some former members share a username. Add --show-ids to tell them apart."
                    .to_owned()
            })
    }

    /// The person a listed principal is: as the directory names its ID, else as it says itself.
    fn listed_person(&self, principal: &Value) -> Person {
        str_field(principal, "id")
            .and_then(|id| self.dir.people.get(id))
            .cloned()
            .unwrap_or_else(|| person_from(principal))
    }

    /// How many of a team's or channel's `members` (principal IDs) are members now. An ID the
    /// workspace lists as a former member is an earlier entry of someone, who is counted by the
    /// ID they hold now, if they are in it at all; an ID nothing names is counted.
    fn active_members(&self, members: &[Value]) -> usize {
        members
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| self.dir.people.get(*id).is_none_or(|person| person.active))
            .count()
    }

    fn profile(&self, principal: &Value) -> Vec<String> {
        let mut out = self.member_row(principal);
        let devices = list_key(principal, "devices");
        if !devices.is_empty() {
            // A person's machine is a computer (2026-11), headed as the desktop's Keys heads it.
            let heading = if self.is_actor(str_field(principal, "id")) {
                "Computers enrolled as you"
            } else {
                "Computers"
            };
            out.push(format!("  {heading} ({}):", devices.len()));
            for device in devices {
                out.push(format!("    {}", self.device_row(device)));
            }
        }
        out
    }

    fn device_row(&self, device: &Value) -> String {
        let mut parts =
            vec![str_field(device, "fingerprint")
                .map_or_else(|| "Unknown computer".into(), safe_text)];
        if let Some(at) = device.get("added_at").and_then(Value::as_i64) {
            parts.push(format!("added {}", self.date(at)));
        }
        parts.extend(added_via(device).map(str::to_owned));
        // AO-5: the key that signed the snapshot this row came from.
        if device.get("this_device").and_then(Value::as_bool) == Some(true) {
            parts.push("This computer".into());
        }
        parts.join(" · ")
    }

    /// A new computer in the desktop's notice's words (`newDevicesCopy.one`): its fingerprint,
    /// then when and how it was added, as known ("3F2A 9C1E 77B0 D4E1, added Sep 23, 2026 by
    /// signing in to the server").
    fn new_device_text(&self, device: &Value) -> String {
        let fingerprint =
            str_field(device, "fingerprint").map_or_else(|| "Unknown computer".into(), safe_text);
        let when = device
            .get("added_at")
            .and_then(Value::as_i64)
            .map(|at| self.date(at));
        let added: Vec<String> = when
            .into_iter()
            .chain(added_via(device).map(str::to_owned))
            .collect();
        if added.is_empty() {
            fingerprint
        } else {
            format!("{fingerprint}, added {}", added.join(" "))
        }
    }

    /// AO-10: a line under a connection for each computer added to the person's account there
    /// since they last looked, until they mark it seen.
    fn new_device_lines(&self, connection: &Value) -> Vec<String> {
        let devices = list_key(connection, "new_devices");
        if devices.is_empty() {
            return Vec::new();
        }
        let command = |verb: &str| match str_field(connection, "name") {
            Some(name) => format!(
                "biorouter crew --connection {} {verb}",
                safe_text(&super::shell_word(name))
            ),
            None => format!("biorouter crew {verb}"),
        };
        // `profile devices` reads the workspace, so on a connection that is down it is refused
        // until it connects, as every read is: the line names connect first.
        let down = str_field(connection, "status").is_some_and(|status| status != "connected");
        let check = if down {
            format!(
                "Connect with {}, then check it with {}.",
                command("connect"),
                command("profile devices")
            )
        } else {
            format!("Check it with {}.", command("profile devices"))
        };
        devices
            .iter()
            .map(|device| {
                format!(
                    "  New computer on your account: {}. {check}",
                    self.new_device_text(device)
                )
            })
            .collect()
    }

    fn team_row(&self, team: &Value) -> Vec<String> {
        let shown = display_or_name(team);
        let mut name = shown.map_or_else(|| "this team".into(), display_text);
        // The handle is what the command line accepts, so show it when it differs.
        if let Some(handle) = str_field(team, "handle")
            .filter(|handle| shown.is_none_or(|shown| !shown.eq_ignore_ascii_case(handle)))
        {
            let _ = write!(name, " ({})", safe_text(handle));
        }
        let mut parts = vec![name];
        if let Some(members) = team.get("members").and_then(Value::as_array) {
            parts.push(count(self.active_members(members), "member", "members"));
        }
        match str_field(team, "created_by") {
            creator if self.is_actor(creator) => parts.push("created by you".into()),
            Some(creator) => parts.push(format!("created by {}", self.person(Some(creator)))),
            None => {}
        }
        if team.get("name_conflict").and_then(Value::as_bool) == Some(true) {
            parts.push("another team has a similar name".into());
        }
        vec![self.with_id(parts.join(" · "), "team ID", str_field(team, "id"))]
    }

    fn channel_row(&self, channel: &Value) -> Vec<String> {
        let id = str_field(channel, "id");
        let name = display_or_name(channel).map_or_else(|| "this channel".into(), channel_name);
        let mut parts = vec![self.with_id(name, "channel ID", id)];
        if let Some(team) =
            str_field(channel, "team_id").filter(|t| self.dir.teams.contains_key(*t))
        {
            parts.push(self.team(Some(team)));
        }
        if let Some(classification) = str_field(channel, "classification") {
            parts.push(classification_word(classification));
        }
        if channel.get("archived").and_then(Value::as_bool) == Some(true) {
            parts.push("Archived".into());
        }
        if let Some(members) = channel.get("members").and_then(Value::as_array) {
            parts.push(count(self.active_members(members), "member", "members"));
        }
        match str_field(channel, "owner_id") {
            owner if self.is_actor(owner) => parts.push("you own it".into()),
            Some(owner) => parts.push(format!("owner {}", self.person(Some(owner)))),
            None => {}
        }
        if let Some(offered) = str_field(channel, "pending_owner") {
            parts.push(format!("offered to {}", self.you_or_person(Some(offered))));
        }
        if let Some(unread) = id
            .and_then(|id| self.dir.unread.get(id))
            .filter(|n| **n > 0)
        {
            parts.push(format!("{unread} unread"));
        }
        vec![parts.join(" · ")]
    }

    fn invitations(&self, value: &Value) -> Vec<String> {
        let mut out = self.rows(
            value,
            "invitations",
            "No pending invitations.",
            Self::invitation_row,
        );
        if !list(value, "invitations").is_empty() {
            self.ids_hint(&mut out, "invites accept and other invitation commands");
        }
        out
    }

    fn invitation_row(&self, invitation: &Value) -> Vec<String> {
        let invitee = str_field(invitation, "principal_id");
        let inviter = str_field(invitation, "inviter_id");
        // Only the invitee's view carries the broker's enrichment, so it identifies the reader
        // when the caller did not say who they are.
        let reader_is_invitee = self.is_actor(invitee)
            || (self.dir.actor_id.is_none() && invitation.get("inviter").is_some());
        let target = self.invitation_target(invitation);
        let row = if reader_is_invitee {
            format!("{} invited you to {target}", self.person(inviter))
        } else if self.is_actor(inviter) {
            format!("You invited {} to {target}", self.person(invitee))
        } else {
            format!(
                "{} invited {} to {target}",
                self.person(inviter),
                self.person(invitee)
            )
        };
        let row = format!("{row} · {}", self.expiry(invitation));
        vec![self.with_id(row, "invitation ID", str_field(invitation, "id"))]
    }

    fn invitation_target(&self, invitation: &Value) -> String {
        let target = str_field(invitation, "target_id");
        let named = str_field(invitation, "target_name");
        if str_field(invitation, "kind") != Some("channel") {
            return match named {
                Some(name) => self.with_id(display_text(name), "team ID", target),
                None => self.team(target),
            };
        }
        match (named, str_field(invitation, "team_name")) {
            (Some(name), Some(team)) => format!(
                "{} in {}",
                self.with_id(channel_name(name), "channel ID", target),
                display_text(team)
            ),
            (Some(name), None) => self.with_id(channel_name(name), "channel ID", target),
            (None, _) => self.channel_in_team(target),
        }
    }

    fn connection_row(&self, connection: &Value) -> Vec<String> {
        let mut parts = vec![text_or(connection, "name", "Unnamed connection")];
        if let Some(target) = str_field(connection, "ssh_target") {
            parts.push(safe_text(target));
        }
        if let Some(status) = connection_status(connection) {
            parts.push(status);
        }
        parts.push(match Effective::of(connection) {
            None | Some(Effective::Own) => connection_privacy(connection),
            Some(Effective::Workspace(workspace)) => {
                format!(
                    "Private because {workspace} is Private for everyone · your connection: Public"
                )
            }
            Some(Effective::Unknown) => format!(
                "Your connection: Public · workspace privacy unknown {}",
                unchecked_because(connection)
            ),
        });
        let mut out = vec![self.with_id(
            parts.join(" · "),
            "connection ID",
            str_field(connection, "id"),
        )];
        out.extend(server_storage_lines(connection));
        out.extend(workspace_deleted_lines(connection));
        out.extend(self.new_device_lines(connection));
        out.extend(last_error_lines(connection));
        out
    }

    fn connection(&self, connection: &Value) -> Vec<String> {
        let mut out = vec![text_or(connection, "name", "Unnamed connection")];
        if let Some(target) = str_field(connection, "ssh_target") {
            let mut server = format!("  Server: {}", safe_text(target));
            if let Some(port) = connection.get("port").and_then(Value::as_u64) {
                if port != 22 {
                    let _ = write!(server, " (port {port})");
                }
            }
            if let Some(jump) = str_field(connection, "proxy_jump") {
                let _ = write!(server, " via {}", safe_text(jump));
            }
            out.push(server);
        }
        if let Some(status) = connection_status(connection) {
            out.push(format!("  Status: {status}"));
        }
        out.extend(server_storage_lines(connection));
        out.extend(workspace_deleted_lines(connection));
        let mut privacy = vec![mode_word(str_field(connection, "mode"))];
        privacy.push(institution(connection));
        if let Some(epoch) = connection.get("policy_epoch").and_then(Value::as_u64) {
            privacy.push(format!("policy epoch {epoch}"));
        }
        // SF-F1: the effective privacy comes first, and the connection's own setting is said
        // as that whenever the two differ or the workspace's cannot be read.
        match Effective::of(connection) {
            None | Some(Effective::Own) => {
                out.push(format!("  Privacy: {}", privacy.join(" · ")));
            }
            Some(Effective::Workspace(workspace)) => {
                out.push(format!(
                    "  Privacy: Private ({workspace} is Private for everyone)"
                ));
                out.push(format!("  Your connection: {}", privacy.join(" · ")));
            }
            Some(Effective::Unknown) => {
                out.push(format!(
                    "  Privacy: can't be checked {}",
                    unchecked_because(connection)
                ));
                out.push(format!("  Your connection: {}", privacy.join(" · ")));
            }
        }
        if let Some(root) = str_field(connection, "remote_root") {
            let execution = connection.get("remote_execution").and_then(Value::as_bool);
            // OW-6: full access is the person's whole account, not the folder.
            let access = if str_field(connection, "remote_access") == Some("full") {
                "agents work as you on the server, starting here"
            } else if execution == Some(true) {
                "agents may run commands there"
            } else {
                "agents may read files there"
            };
            out.push(format!("  Remote folder: {} · {access}", safe_text(root)));
        }
        if let Some(file) = str_field(connection, "identity_file") {
            out.push(format!("  SSH key file: {}", safe_text(file)));
        }
        out.extend(self.new_device_lines(connection));
        out.extend(last_error_lines(connection));
        out.extend(self.detail_ids(connection, CONNECTION_IDS));
        out
    }

    fn detail_ids(&self, value: &Value, fields: &[(&str, &str)]) -> Vec<String> {
        if !self.show_ids {
            return Vec::new();
        }
        fields
            .iter()
            .filter_map(|(key, label)| {
                let text = match value.get(*key)? {
                    Value::String(text) if !text.is_empty() => safe_text(text),
                    Value::Number(number) => number.to_string(),
                    _ => return None,
                };
                Some(format!("  {label}: {text}"))
            })
            .collect()
    }

    fn messages(&self, value: &Value) -> Vec<String> {
        let messages = list(value, "messages");
        let mut out: Vec<String> = messages
            .iter()
            .flat_map(|message| self.message(message))
            .collect();
        if out.is_empty() {
            out.push("No messages.".into());
        }
        if let Some(cursor) = str_field(value, "cursor").filter(|_| self.show_ids) {
            out.push(format!("Cursor: {}", safe_text(cursor)));
        }
        out
    }

    fn message(&self, message: &Value) -> Vec<String> {
        let mut head = self.person(str_field(message, "actor_id"));
        if str_field(message, "run_id").is_some() {
            head.push_str(" · agent");
        }
        if let Some(status) = str_field(message, "status").filter(|status| *status != "progress") {
            let _ = write!(head, " · {}", run_status_word(status));
        }
        if let Some(created) = message.get("created_at").and_then(Value::as_i64) {
            let _ = write!(head, " · {}", self.when(created));
        }
        let restricted = message.get("restricted").and_then(Value::as_bool) == Some(true);
        let channel = str_field(message, "channel_id").and_then(|id| self.dir.channels.get(id));
        if restricted
            && channel.and_then(|channel| channel.classification.as_deref()) != Some("restricted")
        {
            head.push_str(" · Restricted");
        }
        let head = self.with_id(head, "message ID", str_field(message, "id"));
        let body = body_lines(message.get("body").and_then(Value::as_str).unwrap_or(""));
        let mut out = match body.first() {
            Some(first) => vec![format!("{head}  {first}")],
            None => vec![head],
        };
        out.extend(body.iter().skip(1).map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("    {line}")
            }
        }));
        out.extend(self.message_extras(message));
        out
    }

    /// A message's attachments, each by name and size where the command looked it up
    /// (`attachment_details`, DW-17), so `files download ID` can be given the right one; any
    /// it could not look up are counted, as before.
    fn message_extras(&self, message: &Value) -> Vec<String> {
        let mut out = Vec::new();
        let attachments = str_list(message, "attachments");
        let details = message.get("attachment_details");
        let (named, unnamed): (Vec<&str>, Vec<&str>) = attachments
            .iter()
            .partition(|id| details.and_then(|details| details.get(**id)).is_some());
        for id in &named {
            let blob = &details.expect("partitioned on it")[*id];
            let mut text = format!("Attachment: {}", text_or(blob, "name", "Unnamed file"));
            if let Some(size) = blob.get("size").and_then(Value::as_u64) {
                let _ = write!(text, " ({})", human_size(size));
            }
            out.push(format!(
                "    {}",
                self.with_id(text, "attachment ID", Some(id))
            ));
        }
        if !unnamed.is_empty() {
            let more = if named.is_empty() { "" } else { "more " };
            let mut line = format!(
                "    {}",
                count(
                    unnamed.len(),
                    &format!("{more}attachment"),
                    &format!("{more}attachments")
                )
            );
            if self.show_ids {
                let _ = write!(line, " [attachment IDs {}]", join_safe(&unnamed));
            }
            out.push(line);
        }
        for reference in str_list(message, "references") {
            let text = match self.dir.references.get(reference) {
                Some((label, path)) => format!(
                    "Remote reference: {} ({})",
                    display_text(label),
                    safe_text(path)
                ),
                None => "Remote reference".into(),
            };
            out.push(format!(
                "    {}",
                self.with_id(text, "reference ID", Some(reference))
            ));
        }
        if let Some(run) = str_field(message, "run_id").filter(|_| self.show_ids) {
            out.push(format!("    Task ID: {}", safe_text(run)));
        }
        out
    }

    fn tasks(&self, value: &Value) -> Vec<String> {
        let runs = list(value, "runs");
        if runs.is_empty() {
            return vec!["No tasks.".into()];
        }
        let mut out: Vec<String> = runs.iter().flat_map(|run| self.task(run)).collect();
        self.ids_hint(&mut out, "tasks show, watch and cancel");
        out
    }

    fn task(&self, run: &Value) -> Vec<String> {
        let channel = match str_field(run, "channel_name") {
            Some(name) => channel_name(name),
            None => self.channel_in_team(str_field(run, "channel_id")),
        };
        let status =
            str_field(run, "status").map_or_else(|| "Status unknown".into(), run_status_word);
        let title = str_field(run, "title").or_else(|| str_field(run, "prompt"));
        let head = match title {
            Some(title) => format!(
                "Task {} posting to {channel} · {status}",
                quoted_name(&excerpt(title))
            ),
            None => format!("Task posting to {channel} · {status}"),
        };
        let mut out = vec![self.with_id(head, "task ID", str_field(run, "run_id"))];
        // OW-11: the model a task started on without naming one, and where it came from.
        let chosen = &run["task_model"];
        if let (Some(model), Some(provider)) =
            (str_field(chosen, "model"), str_field(chosen, "provider"))
        {
            let source = match str_field(chosen, "source") {
                Some("crew_tasks") => ", your model for Crew tasks",
                Some("new_chats") => ", your default model for new chats",
                _ => "",
            };
            out.push(format!(
                "  Model: {} ({}){source}",
                safe_text(model),
                safe_text(provider)
            ));
        }
        if let Some(session) = str_field(run, "session_id") {
            out.push(format!("  Chat: {}", safe_text(session)));
        }
        if let Some(error) = str_field(run, "error") {
            out.push(format!("  Error: {}", safe_text(error)));
        }
        out.extend(self.detail_ids(run, &[("connection_id", "Connection ID")]));
        out
    }

    /// Every chat's grant, then the earlier grants no chat holds any more whose revocation the
    /// daemon keeps (`replaced_grants`, F3): replaced by a new grant to the same chat, or left
    /// by a deleted chat whose id a new chat now holds.
    fn grants(&self, value: &Value) -> Vec<String> {
        // AO-12: a chat the person let read a workspace as them is listed after the grants.
        let reads = list_key(value, "person_reads");
        let mut out = if reads.is_empty() || !list_key(value, "grants").is_empty() {
            self.rows(
                value,
                "grants",
                "No chats have Crew access.",
                Self::grant_row,
            )
        } else {
            Vec::new()
        };
        out.extend(reads.iter().flat_map(|read| self.person_read_row(read)));
        let replaced = value
            .get("replaced_grants")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if replaced.is_empty() {
            return out;
        }
        out.push("Earlier access no chat holds any more:".into());
        for grant in replaced {
            out.extend(
                self.grant_row(grant)
                    .into_iter()
                    .map(|row| format!("  {row}")),
            );
        }
        if replaced
            .iter()
            .any(|grant| str_field(grant, "revocation") == Some("unconfirmed"))
        {
            out.push(
                "Biorouter confirms these with the workspace by itself whenever the connection is up."
                    .into(),
            );
        }
        out
    }

    /// One chat's or task's grant (F4): the same facts the JSON carries, in the words the
    /// desktop's access list uses. A task is called a task (`kind`), the conversation by its
    /// title (`session_name`), and the state is read from `expired`, `expires_at` and
    /// `revocation` together, so a grant whose time ran out never reads Active.
    fn grant_row(&self, grant: &Value) -> Vec<String> {
        let task = str_field(grant, "kind") == Some("task");
        let (noun, lower) = if task {
            ("Task", "task")
        } else {
            ("Chat", "chat")
        };
        let session =
            str_field(grant, "session_id").map_or_else(|| format!("this {lower}"), safe_text);
        let title = str_field(grant, "session_name")
            .or_else(|| str_field(grant, "title"))
            .filter(|title| !title.trim().is_empty());
        let chat = match title {
            Some(title) => format!("{noun} {} ({lower} {session})", quoted_name(title)),
            None => format!("{noun} {session}"),
        };
        let destination = str_field(grant, "channel_id");
        let mut row = format!("{chat} → {}", self.channel_in_team(destination));
        let others: Vec<String> = str_list(grant, "source_channels")
            .into_iter()
            .filter(|source| Some(*source) != destination)
            .map(|source| self.channel(Some(source)))
            .collect();
        if !others.is_empty() {
            let _ = write!(row, " (also reads {})", others.join(", "));
        }
        let _ = write!(row, " · {}", self.grant_state(grant, task));
        if let Some(epoch) = grant.get("policy_epoch").and_then(Value::as_u64) {
            let _ = write!(row, " · policy epoch {epoch}");
        }
        vec![self.with_id(row, "task ID", str_field(grant, "run_id"))]
    }

    /// A chat the person let read a workspace as them (AO-12): the chat, the workspace (the
    /// saved connection's name the command added), and since when. `grants revoke SESSION
    /// --reads` withdraws it.
    fn person_read_row(&self, read: &Value) -> Vec<String> {
        let session =
            str_field(read, "session_id").map_or_else(|| "this chat".to_owned(), safe_text);
        let chat = match str_field(read, "session_name").filter(|title| !title.trim().is_empty()) {
            Some(title) => format!("Chat {} (chat {session})", quoted_name(title)),
            None => format!("Chat {session}"),
        };
        let workspace = str_field(read, "connection_name")
            .map_or_else(|| "its workspace".to_owned(), display_text);
        let mut row = format!("{chat} reads {workspace} as you");
        if let Some(at) = read.get("granted_at").and_then(Value::as_i64) {
            let _ = write!(row, " · since {}", self.date(at));
        }
        vec![row]
    }

    /// A grant's state as the desktop's access list says it (`accessRows.ts`): a stop the
    /// workspace has not confirmed yet, a stop the workspace made itself, a revocation, a
    /// grant whose time ran out, or an active one with its end. A task's grant ends with the
    /// task, so it reads Ended rather than Revoked or Expired.
    fn grant_state(&self, grant: &Value, task: bool) -> String {
        let stopped = grant.get("expired").and_then(Value::as_bool) == Some(true);
        let expires_at = grant.get("expires_at").and_then(Value::as_i64);
        let now = self.clock.now();
        // AGT2-N3: a task the person stopped reads Stopped, as its card in the channel does; one
        // that finished (or whose status the command could not read) reads Ended. The grant
        // cannot tell them apart, so the command adds the task's status (`task_status`).
        let task_over = || {
            if str_field(grant, "task_status") == Some("cancelled") {
                "Stopped".to_owned()
            } else {
                "Ended".to_owned()
            }
        };
        if stopped {
            return match str_field(grant, "revocation") {
                Some("unconfirmed") => {
                    "Stopped on this computer; the workspace hasn't confirmed yet".into()
                }
                Some("ended_by_workspace") => "Ended: Crew settings changed".into(),
                _ if task => task_over(),
                _ => "Revoked".into(),
            };
        }
        match expires_at {
            Some(at) if at <= now => {
                if task {
                    task_over()
                } else {
                    "Expired".into()
                }
            }
            Some(at) => format!("Active · ends {}", within(at - now)),
            None => "Active".into(),
        }
    }

    fn run_grant_row(&self, run: &Value) -> Vec<String> {
        let revoked = run.get("revoked").and_then(Value::as_bool) == Some(true);
        let state = if revoked {
            "ended".to_owned()
        } else {
            self.expiry(run)
        };
        let row = format!(
            "{} · {state}",
            self.channel_in_team(str_field(run, "channel_id"))
        );
        vec![self.with_id(row, "task ID", str_field(run, "id"))]
    }

    fn privacy(&self, value: &Value) -> Vec<String> {
        let mut connection = vec![mode_word(str_field(value, "personal_mode"))];
        connection.push(institution(value));
        if let Some(epoch) = value.get("connection_policy_epoch").and_then(Value::as_u64) {
            connection.push(format!("policy epoch {epoch}"));
        }
        let workspace = value.get("workspace").unwrap_or(&Value::Null);
        // Read only while connected (SF2-N3): a `privacy show` made offline has none.
        let unread = !workspace.is_object();
        // SF-F1: privacy is Public only when the connection is and the workspace allows it.
        let effective = match (
            str_field(value, "personal_mode"),
            str_field(workspace, "mode"),
        ) {
            (Some("public"), Some("private")) => format!(
                "Private ({} is Private for everyone)",
                str_field(workspace, "name")
                    .map_or_else(|| "the workspace".to_owned(), display_text)
            ),
            (Some("public"), Some("public")) => "Public".to_owned(),
            (Some("public"), _) if unread => "can't be checked while disconnected".to_owned(),
            (Some("public"), _) => "can't be checked".to_owned(),
            (own, _) => mode_word(own),
        };
        let workspace_line = if unread {
            "Workspace: can't be checked while disconnected".to_owned()
        } else {
            self.with_id(
                format!("Workspace: {}", workspace_privacy(workspace)),
                "workspace ID",
                str_field(workspace, "id"),
            )
        };
        let mut out = vec![
            format!("Privacy: {effective}"),
            self.with_id(
                format!("Your connection: {}", connection.join(" · ")),
                "connection ID",
                str_field(value, "connection_id"),
            ),
            workspace_line,
        ];
        let channels = list_key(value, "channels");
        if !channels.is_empty() {
            out.push(format!("Channels ({}):", channels.len()));
            for channel in channels {
                let mut parts =
                    vec![display_or_name(channel)
                        .map_or_else(|| "this channel".into(), channel_name)];
                if let Some(team) =
                    str_field(channel, "team_id").filter(|team| self.dir.teams.contains_key(*team))
                {
                    parts.push(self.team(Some(team)));
                }
                if let Some(classification) = str_field(channel, "classification") {
                    parts.push(classification_word(classification));
                }
                let row = self.with_id(parts.join(" · "), "channel ID", str_field(channel, "id"));
                out.push(format!("  {row}"));
            }
        }
        out.push("Pass the policy epochs to --expected-policy-epoch and --expected-workspace-policy-epoch to refuse a changed policy.".into());
        out
    }

    fn transfers(&self, value: &Value) -> Vec<String> {
        let transfers = list(value, "transfers");
        if transfers.is_empty() {
            return vec!["No file transfers.".into()];
        }
        let mut out: Vec<String> = transfers
            .iter()
            .flat_map(|transfer| self.transfer(transfer, false))
            .collect();
        self.ids_hint(&mut out, "files status, resume, pause and forget");
        out
    }

    /// One transfer. `alone` is a single receipt (`files status`), whose resume command names
    /// its ID: the person just typed it. In a list the ID is there only with `--show-ids`, as
    /// every other ID is.
    fn transfer(&self, transfer: &Value, alone: bool) -> Vec<String> {
        let name = text_or(transfer, "name", "Unnamed file");
        let direction = str_field(transfer, "direction").unwrap_or("upload");
        let channel = self.channel(str_field(transfer, "channel_id"));
        let route = if direction == "download" {
            format!("download from {channel}")
        } else {
            format!("upload to {channel}")
        };
        let size = transfer.get("size").and_then(Value::as_u64);
        let offset = transfer.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let state = str_field(transfer, "state").unwrap_or("unknown");
        let mut row = format!(
            "{name} · {route} · {}",
            transfer_state_word(state, direction, offset, size)
        );
        match size {
            Some(size) if matches!(state, "uploading" | "downloading") => {
                let _ = write!(row, " ({} of {})", human_size(offset), human_size(size));
            }
            Some(size) => {
                let _ = write!(row, " · {}", human_size(size));
            }
            None => {}
        }
        let mut out = vec![self.with_id(row, "transfer ID", str_field(transfer, "id"))];
        if matches!(state, "paused" | "needs_file_selection") {
            // CLIDOCS-F10: a pause is a state, not an error. Its reason in the desktop's few
            // words, then this terminal's way on, which the daemon's "Reselect the original
            // local file" is not.
            if let Some(reason) = str_field(transfer, "error").and_then(pause_reason) {
                out.push(format!("  {reason}"));
            }
            let what = if direction == "download" {
                "the same destination"
            } else {
                "the original file"
            };
            let id = str_field(transfer, "id")
                .filter(|_| alone || self.show_ids)
                .map_or_else(|| "ID".to_owned(), safe_text);
            out.push(format!(
                "  Resume with {what}: biorouter crew files resume {id} FILE"
            ));
        } else if let Some(error) = str_field(transfer, "error") {
            out.push(format!("  Error: {}", safe_text(error)));
        }
        out.extend(self.detail_ids(transfer, TRANSFER_IDS));
        out
    }

    fn reference_row(&self, reference: &Value) -> Vec<String> {
        let label = text_or(reference, "label", "Remote reference");
        let mut parts = vec![label];
        if let Some(path) = str_field(reference, "path") {
            parts.push(safe_text(path));
        }
        parts.push(self.channel(str_field(reference, "channel_id")));
        if let Some(owner) = str_field(reference, "owner_id") {
            parts.push(format!("shared by {}", self.you_or_person(Some(owner))));
        }
        parts.push("Not uploaded".into());
        vec![self.with_id(
            parts.join(" · "),
            "reference ID",
            str_field(reference, "id"),
        )]
    }

    fn attachment_row(&self, blob: &Value) -> Vec<String> {
        let mut parts = vec![text_or(blob, "name", "Unnamed file")];
        if let Some(size) = blob.get("size").and_then(Value::as_u64) {
            parts.push(human_size(size));
        }
        if let Some(media_type) = str_field(blob, "media_type") {
            parts.push(safe_text(media_type));
        }
        parts.push(format!(
            "in {}",
            self.channel(str_field(blob, "channel_id"))
        ));
        if let Some(owner) = str_field(blob, "owner_id") {
            parts.push(format!("shared by {}", self.you_or_person(Some(owner))));
        }
        if blob.get("complete").and_then(Value::as_bool) == Some(false) {
            parts.push("upload not finished".into());
        }
        let mut out = vec![self.with_id(parts.join(" · "), "attachment ID", str_field(blob, "id"))];
        out.extend(self.detail_ids(blob, &[("sha256", "SHA-256")]));
        out
    }

    fn result(&self, value: &Value) -> Vec<String> {
        let has = |key: &str| value.get(key).is_some_and(|value| !value.is_null());
        let flag = |key: &str| value.get(key).and_then(Value::as_bool);
        let team = value.get("team").filter(|value| value.is_object());
        if let (Some(team), Some(channel)) = (team, value.get("channel")) {
            let name = display_or_name(team).map_or_else(|| "a team".into(), display_text);
            let general =
                display_or_name(channel).map_or_else(|| "its first channel".into(), channel_name);
            let row = format!(
                "Created team {} with {}.",
                self.with_id(name, "team ID", str_field(team, "id")),
                self.with_id(general, "channel ID", str_field(channel, "id"))
            );
            return vec![row];
        }
        if has("principal") && has("workspace") {
            return self.joined(value);
        }
        if has("preparation_id") && has("public_key") {
            return self.prepared(value);
        }
        if has("invitation") && has("uid") && has("expires_at") {
            return self.enrollment_token(value);
        }
        if has("authentication_id") {
            let mut out = vec!["Authenticated. The connection is ready.".to_owned()];
            out.extend(self.detail_ids(value, &[("authentication_id", "Authentication ID")]));
            return out;
        }
        if has("instance_id") && (has("pid") || flag("stopped") == Some(true)) {
            return self.daemon(value);
        }
        if has("backend") && has("locked") {
            return vec![credentials(value)];
        }
        if flag("cancelled").is_some() {
            return vec![cancellation(value)];
        }
        self.small_result(value)
    }

    fn small_result(&self, value: &Value) -> Vec<String> {
        let flag = |key: &str| value.get(key).and_then(Value::as_bool);
        let fields = value.as_object().map_or(0, Map::len);
        if flag("revoked") == Some(true) && fields == 1 {
            return vec!["Revoked.".into()];
        }
        if flag("accepted") == Some(true) && fields == 1 {
            return vec!["Invitation accepted.".into()];
        }
        if let (Some(session), Some(run), 2) = (
            str_field(value, "session_id"),
            str_field(value, "run_id"),
            fields,
        ) {
            let row = format!("Crew access granted to chat {}.", safe_text(session));
            return vec![self.with_id(row, "task ID", Some(run))];
        }
        if str_field(value, "channel_id").is_some()
            && value.get("sequence").is_some()
            && fields == 2
        {
            return vec![format!(
                "Marked {} as read.",
                self.channel(str_field(value, "channel_id"))
            )];
        }
        if let Some(transfer) = str_field(value, "transfer_id") {
            let text = if flag("detached") == Some(true) {
                "Stopped watching. The transfer continues.".to_owned()
            } else {
                // The direction decides the word a finished transfer ends with: an upload is
                // Ready to attach, a download is Saved.
                let state = str_field(value, "state").unwrap_or("unknown");
                let direction = str_field(value, "direction").unwrap_or("");
                format!(
                    "Transfer {}.",
                    transfer_state_word(state, direction, 0, None)
                )
            };
            return vec![self.with_id(text, "transfer ID", Some(transfer))];
        }
        self.generic(value)
    }

    fn joined(&self, value: &Value) -> Vec<String> {
        let principal = value.get("principal").unwrap_or(&Value::Null);
        let workspace = match self.dir.workspace_name.as_deref() {
            Some(name) => safe_text(name),
            None => "the workspace".into(),
        };
        let person = self.with_id(
            person_from(principal).label(),
            "ID",
            str_field(principal, "id"),
        );
        let mut out = vec![format!("Signed in to {workspace} as {person}.")];
        out.extend(self.detail_ids(value, &[("device_id", "Computer ID")]));
        if let Some(workspace) = value.get("workspace") {
            out.extend(self.detail_ids(workspace, &[("id", "Workspace ID")]));
        }
        out
    }

    /// A prepared device key (SF-F3). `connections prepare` and `enroll prepare` are one
    /// command, and nothing in it says which use the key is for: a host passes it to
    /// `biorouter-crew start` and the preparation ID to `connections join-invitation`, while a
    /// joiner sends the key to the host and puts the ID in a `connections save` descriptor. So
    /// both uses are named, and neither reader is sent down the other's path.
    fn prepared(&self, value: &Value) -> Vec<String> {
        let key = str_field(value, "public_key").map_or_else(String::new, safe_text);
        let mut out = vec![
            "Device key prepared. Its public key:".to_owned(),
            format!("  {key}"),
            PREPARED_KEY_USES.to_owned(),
        ];
        if self.show_ids {
            out.extend(self.detail_ids(
                value,
                &[
                    ("preparation_id", "Preparation ID"),
                    ("device_id", "Computer ID"),
                ],
            ));
            out.push(format!("The preparation ID {PREPARATION_ID_USES}."));
        } else {
            out.push(format!(
                "Add --show-ids for the preparation ID, which {PREPARATION_ID_USES}."
            ));
        }
        out
    }

    fn enrollment_token(&self, value: &Value) -> Vec<String> {
        let token = str_field(value, "invitation").map_or_else(String::new, safe_text);
        let mut out = vec![
            format!(
                "Send this enrollment token to your colleague privately. It {}.",
                self.expiry(value)
            ),
            format!("  {token}"),
        ];
        out.extend(self.detail_ids(value, &[("uid", "UID"), ("device_id", "Computer ID")]));
        out
    }

    fn daemon(&self, value: &Value) -> Vec<String> {
        let mut out = Vec::new();
        if value.get("stopped").and_then(Value::as_bool) == Some(true) {
            out.push("Biorouter daemon stopped.".to_owned());
            if value.get("replacement_instance_id").is_some() {
                out.push("A new daemon has since started for this profile.".into());
            }
        } else {
            let pid = value.get("pid").and_then(Value::as_u64);
            out.push(match pid {
                Some(pid) => format!("Biorouter daemon running (pid {pid}) for this profile."),
                None => "Biorouter daemon running for this profile.".into(),
            });
            if value.get("user_action_installed").and_then(Value::as_bool) == Some(false) {
                out.push("It cannot verify human Crew actions. Stop it with biorouter crew daemon stop, and the next biorouter crew command starts a new one.".into());
            }
        }
        out.extend(self.detail_ids(value, DAEMON_IDS));
        out
    }

    /// Any other value: a field list in plain words, with ID-like fields left out unless
    /// `show_ids` asks for them.
    fn generic(&self, value: &Value) -> Vec<String> {
        match value {
            Value::Object(fields) => {
                let mut out = Vec::new();
                if let Some(message) = str_field(value, "message") {
                    out.push(safe_text(message));
                }
                for (key, item) in fields {
                    if key == "message" || item.is_null() || self.hidden(key, item) {
                        continue;
                    }
                    let label = sentence_case(key);
                    match item {
                        Value::Object(inner) if !inner.is_empty() => {
                            out.push(format!("{label}:"));
                            out.extend(self.generic(item).into_iter().map(|l| format!("  {l}")));
                        }
                        Value::Array(items) if items.iter().any(|item| item.is_object()) => {
                            out.push(format!("{label}:"));
                            for item in items {
                                let lines = self.render(plural(detect_item(item)), item);
                                out.extend(lines.into_iter().map(|line| format!("  {line}")));
                            }
                        }
                        _ => out.push(format!("{label}: {}", self.scalar(item))),
                    }
                }
                if out.is_empty() {
                    out.push("Done.".into());
                }
                out
            }
            Value::Array(items) if items.is_empty() => vec!["No items.".into()],
            Value::Array(items) => items
                .iter()
                .flat_map(|item| match item {
                    Value::Object(_) => self.render(plural(detect_item(item)), item),
                    other => vec![self.scalar(other)],
                })
                .collect(),
            other => vec![self.scalar(other)],
        }
    }

    fn hidden(&self, key: &str, value: &Value) -> bool {
        !self.show_ids && (id_key(key) || machine_value(value))
    }

    fn scalar(&self, value: &Value) -> String {
        match value {
            Value::String(text) => safe_text(text),
            Value::Bool(true) => "yes".into(),
            Value::Bool(false) => "no".into(),
            Value::Null => "none".into(),
            Value::Array(items) => {
                let shown: Vec<String> = items
                    .iter()
                    .filter(|item| self.show_ids || !machine_value(item))
                    .map(|item| self.scalar(item))
                    .collect();
                if shown.is_empty() {
                    "none".into()
                } else {
                    shown.join(", ")
                }
            }
            other => json_terminal_safe(other.to_string()),
        }
    }
}

/// A saved connection's status word, as the desktop's status row says it: "Not joined yet" for a
/// connection the host has not let in (`joined: false`, SC2-N5), which is connected but not a
/// member, before the transport's own status.
fn connection_status(connection: &Value) -> Option<String> {
    if connection.get("joined").and_then(Value::as_bool) == Some(false) {
        return Some("Not joined yet".to_owned());
    }
    str_field(connection, "status").map(sentence_case)
}

/// [`SERVER_STOPPED_SAVING`] for a connection whose `server_storage` says so, and, for its host
/// (`you_host`), what to run on the server. Nothing for an absent or `null` `server_storage`:
/// the server is saving, or it is an older one that does not say.
fn server_storage_lines(connection: &Value) -> Vec<String> {
    let Some(storage) = connection.get("server_storage").filter(|s| s.is_object()) else {
        return Vec::new();
    };
    let mut out = vec![format!("  {SERVER_STOPPED_SAVING}")];
    if connection.get("you_host").and_then(Value::as_bool) == Some(true) {
        let server = str_field(connection, "server_label")
            .or_else(|| {
                str_field(connection, "ssh_target").and_then(|target| target.rsplit('@').next())
            })
            .map_or_else(|| "the server".to_owned(), safe_text);
        let first = match str_field(storage, "code") {
            Some("storage_full") => format!("Free space on {server}"),
            _ => format!("Check the storage on {server}"),
        };
        out.push(format!("  {}", host_restart_steps(&first)));
    }
    out
}

/// Said under a member's connection to a workspace its host deleted (OW-5): the daemon marks
/// it (`workspace_deleted`), and the one thing left is to remove it here. A last error that
/// already says so is not said twice.
fn workspace_deleted_lines(connection: &Value) -> Vec<String> {
    let marked = connection.get("workspace_deleted").and_then(Value::as_bool) == Some(true);
    if !marked || str_field(connection, "last_error_code") == Some("crew_workspace_deleted") {
        return Vec::new();
    }
    connect_failure_text("crew_workspace_deleted")
        .map(|sentence| vec![format!("  {sentence}")])
        .unwrap_or_default()
}

/// A saved connection's last error. When the daemon types it (`last_error_code`) with a code
/// the connect failures share, the sentence that says what to do comes first and the daemon's
/// own text follows as its details (a wave-1 follow-up to CLI-7); otherwise the text is shown
/// as it is.
fn last_error_lines(connection: &Value) -> Vec<String> {
    let Some(error) = str_field(connection, "last_error") else {
        return Vec::new();
    };
    let code = str_field(connection, "last_error_code");
    // A login on this machine is told what a member on the server itself must fix.
    let same_host = code
        .zip(str_field(connection, "ssh_target"))
        .and_then(|(code, ssh_target)| same_host_connect_failure_text(code, ssh_target));
    match same_host.or_else(|| code.and_then(connect_failure_text).map(str::to_owned)) {
        Some(sentence) => vec![
            format!("  Last error: {sentence}"),
            format!("    Details: {}", safe_text(error)),
        ],
        None => vec![format!("  Last error: {}", safe_text(error))],
    }
}

/// What a prepared device key is for, both ways (`command-line.md` "Host a workspace", and
/// `cli-guide.md` "Save a connection from a descriptor").
const PREPARED_KEY_USES: &str = "To host a workspace, pass this key to biorouter-crew start --bootstrap-key on the server. To join one, send it to the workspace's host.";
/// Where each reader puts the preparation ID, completing "The preparation ID …".
const PREPARATION_ID_USES: &str = "goes to connections join-invitation --preparation-id when you host, or into the descriptor for connections save when you join";

const CONNECTION_IDS: &[(&str, &str)] = &[
    ("id", "Connection ID"),
    ("workspace_id", "Workspace ID"),
    ("workspace_public_key", "Workspace key"),
    ("node_id", "Node ID"),
    ("device_id", "Computer ID"),
    ("public_key", "Device key"),
    ("cluster_connection_id", "Cluster connection ID"),
    ("socket_path", "Socket"),
    ("owner_uid", "Host UID"),
];

const TRANSFER_IDS: &[(&str, &str)] = &[
    ("request_id", "Request ID"),
    ("connection_id", "Connection ID"),
    ("blob_id", "Attachment ID"),
    ("sha256", "SHA-256"),
];

const DAEMON_IDS: &[(&str, &str)] = &[
    ("profile_id", "Profile ID"),
    ("instance_id", "Instance ID"),
    ("replacement_instance_id", "Replacement instance ID"),
];

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn str_list<'a>(value: &'a Value, key: &str) -> Vec<&'a str> {
    list_key(value, key)
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

fn list_key<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match value.get(key) {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

/// The items of a list view: the value itself when it is an array, its `key` array when it has
/// one, and otherwise the single object as a one-item list.
fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match value {
        Value::Array(items) => items,
        Value::Object(fields) => match fields.get(key) {
            Some(Value::Array(items)) => items,
            _ => std::slice::from_ref(value),
        },
        _ => &[],
    }
}

fn display_or_name(value: &Value) -> Option<&str> {
    str_field(value, "display_name").or_else(|| str_field(value, "name"))
}

fn text_or(value: &Value, key: &str, fallback: &str) -> String {
    str_field(value, key).map_or_else(|| fallback.to_owned(), display_text)
}

fn channel_name(name: &str) -> String {
    format!("#{}", safe_text(name.trim_start_matches('#')))
}

fn join_safe(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| safe_text(item))
        .collect::<Vec<_>>()
        .join(", ")
}

fn body_lines(body: &str) -> Vec<String> {
    let mut lines: Vec<String> = body
        .split('\n')
        .map(|line| safe_text(line.strip_suffix('\r').unwrap_or(line)))
        .collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

fn excerpt(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() > 60 {
        format!("{}…", first.chars().take(59).collect::<String>())
    } else {
        first.to_owned()
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn within(seconds: i64) -> String {
    let minutes = (seconds + 59) / 60;
    if minutes < 60 {
        return format!("in {}", count(minutes as usize, "minute", "minutes"));
    }
    let hours = (seconds + 1800) / 3600;
    if hours < 48 {
        return format!("in {}", count(hours as usize, "hour", "hours"));
    }
    format!(
        "in {}",
        count(((seconds + 43_200) / 86_400) as usize, "day", "days")
    )
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn sentence_case(value: &str) -> String {
    let text = safe_text(&value.replace('_', " "));
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The run status words the desktop shows (`crewStatus.ts`).
pub(super) fn run_status_word(status: &str) -> String {
    match status {
        "starting" => "Starting…".into(),
        "running" => "Working…".into(),
        "waiting_for_approval" => "Waiting for your approval".into(),
        "cancellation_pending" => "Stopping…".into(),
        "cancellation_unconfirmed" => "Stop not confirmed".into(),
        "interrupted" => "Interrupted".into(),
        "outcome_not_durable" => "Outcome unknown".into(),
        "completed" => "Done".into(),
        "failed" => "Couldn't finish".into(),
        "cancelled" => "Stopped".into(),
        other => sentence_case(other),
    }
}

/// Why a paused transfer stopped, from the daemon's recovery sentence
/// (`transfer_recovery_message` in `crates/biorouter-server/src/crew/transfers.rs`), in the
/// desktop's few words (`PAUSE_REASONS` in `ui/desktop/src/components/crew/state/crewStatus.ts`),
/// with a full stop. A sentence neither knows is said as it came.
fn pause_reason(error: &str) -> Option<String> {
    const REASONS: [(&str, &str); 6] = [
        ("Transfer paused", "You paused it."),
        (
            "Authenticate and reconnect the saved connection",
            "The connection dropped.",
        ),
        (
            "Unlock the Crew credential vault",
            "The credential vault is locked.",
        ),
        (
            "Two transfers are active",
            "Two other transfers were running.",
        ),
        (
            "The Crew connection or privacy policy changed",
            "The connection's privacy changed.",
        ),
        ("Transfer stopped", "It stopped."),
    ];
    let text = error.trim();
    if text.is_empty() {
        return None;
    }
    Some(
        REASONS
            .iter()
            .find(|(start, _)| {
                text.strip_prefix(start)
                    .is_some_and(|rest| !rest.starts_with(|ch: char| ch.is_alphanumeric()))
            })
            .map_or_else(|| safe_text(text), |(_, words)| (*words).to_owned()),
    )
}

fn transfer_state_word(state: &str, direction: &str, offset: u64, size: Option<u64>) -> String {
    let percent = match size {
        Some(0) | None => 0,
        Some(size) => (u128::from(offset.min(size)) * 100 / u128::from(size)) as u64,
    };
    match state {
        "starting" => "Starting…".into(),
        "uploading" => format!("Uploading {percent}%"),
        "downloading" => format!("Downloading {percent}%"),
        "publishing" => "Finishing…".into(),
        "pause_requested" => "Pausing…".into(),
        "paused" | "needs_file_selection" => "Paused".into(),
        "completed" if direction == "download" => "Saved".into(),
        "completed" => "Ready".into(),
        "failed" => "Failed".into(),
        "publication_unconfirmed" => "Not confirmed".into(),
        other => sentence_case(other),
    }
}

fn classification_word(classification: &str) -> String {
    match classification {
        "restricted" => "Restricted".into(),
        "public_safe" => "Public-safe".into(),
        other => sentence_case(other),
    }
}

fn mode_word(mode: Option<&str>) -> String {
    match mode {
        Some("private") => "Private".into(),
        Some("public") => "Public".into(),
        Some(other) => sentence_case(other),
        None => "Not set".into(),
    }
}

fn institution(value: &Value) -> String {
    str_field(value, "institution_id").map_or_else(
        || "no institution".into(),
        |id| format!("institution {}", safe_text(id)),
    )
}

fn connection_privacy(connection: &Value) -> String {
    let mode = mode_word(str_field(connection, "mode"));
    match str_field(connection, "institution_id") {
        Some(institution) => format!("{mode} ({})", safe_text(institution)),
        None => mode,
    }
}

/// A saved connection's effective privacy, as the CLI annotated it (`effective_mode`,
/// `workspace_mode`, `workspace_name`; SF-F1). `None` for a value it did not annotate.
enum Effective {
    /// The connection's own setting is the effective one.
    Own,
    /// The connection is Public, but the workspace, named here, is Private for everyone.
    Workspace(String),
    /// The connection is Public, and the workspace's setting could not be read.
    Unknown,
}

impl Effective {
    fn of(connection: &Value) -> Option<Self> {
        let effective = connection.get("effective_mode")?;
        let own = str_field(connection, "mode");
        Some(match effective.as_str() {
            None => Self::Unknown,
            Some(mode) if Some(mode) == own => Self::Own,
            Some(_) => Self::Workspace(
                str_field(connection, "workspace_name")
                    .map_or_else(|| "the workspace".to_owned(), display_text),
            ),
        })
    }
}

/// Why a Public connection's workspace privacy was not read: it is disconnected, or the
/// workspace did not answer in time.
fn unchecked_because(connection: &Value) -> &'static str {
    if str_field(connection, "status") == Some("connected") {
        "right now"
    } else {
        "while disconnected"
    }
}

/// The privacy a connection in `own` mode has in a workspace in `workspace` mode (SF-F1, the
/// manual's definition): Private when either is, Public only when both allow it, and unknown
/// when the connection is Public and the workspace's mode could not be read.
pub fn effective_mode(own: Option<&str>, workspace: Option<&str>) -> Option<&'static str> {
    match (own, workspace) {
        (Some("public"), Some("public")) => Some("public"),
        (Some("public"), Some(_)) => Some("private"),
        (Some("public"), None) => None,
        _ => Some("private"),
    }
}

fn workspace_privacy(workspace: &Value) -> String {
    let mode = match str_field(workspace, "mode") {
        Some("private") => "Private for everyone".into(),
        Some("public") => "Allows Public".into(),
        other => mode_word(other),
    };
    let mut parts = vec![mode, institution(workspace)];
    if let Some(epoch) = workspace.get("policy_epoch").and_then(Value::as_u64) {
        parts.push(format!("policy epoch {epoch}"));
    }
    parts.join(" · ")
}

fn credentials(value: &Value) -> String {
    let state = match (
        value.get("initialized").and_then(Value::as_bool),
        value.get("locked").and_then(Value::as_bool),
    ) {
        (Some(false), _) => "Not set up",
        (_, Some(true)) => "Locked",
        _ => "Unlocked",
    };
    // SETUPHPC2-F-C: a keyring that cannot keep a key (a headless Linux node with no Secret
    // Service) is not the normal "Not set up · keyring": say so, and what to run instead.
    if value.get("available").and_then(Value::as_bool) == Some(false) {
        return format!(
            "Credential vault: {state} · no keyring on this computer; run biorouter crew credentials init"
        );
    }
    match str_field(value, "backend") {
        Some(backend) => format!("Credential vault: {state} · {}", safe_text(backend)),
        None => format!("Credential vault: {state}"),
    }
}

fn cancellation(value: &Value) -> String {
    if let Some(message) = str_field(value, "message") {
        return safe_text(message);
    }
    let status =
        str_field(value, "status").map_or_else(|| "Status unknown".into(), run_status_word);
    if value.get("already_finished").and_then(Value::as_bool) == Some(true) {
        format!("The task had already finished: {status}.")
    } else if value.get("cancelled").and_then(Value::as_bool) == Some(true) {
        "Task stopped.".into()
    } else {
        format!("Task status: {status}.")
    }
}

fn id_key(key: &str) -> bool {
    key == "id"
        || key.ends_with("_id")
        || key.ends_with("_ids")
        || matches!(
            key,
            "members"
                | "sequence"
                | "cursor"
                | "public_key"
                | "workspace_public_key"
                | "sha256"
                | "binding"
                | "uid"
                | "host_uid"
                | "owner_uid"
                | "socket_path"
                | "source_channels"
                | "attachments"
                | "references"
                | "created_by"
                | "pending_owner"
        )
}

/// A UUID or a 64-hex digest, key or device ID, or a list made only of them.
fn machine_value(value: &Value) -> bool {
    match value {
        Value::String(text) => is_uuid(text) || is_hex64(text),
        Value::Array(items) => !items.is_empty() && items.iter().all(machine_value),
        _ => false,
    }
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(index, ch)| match index {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn read_input(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .lock()
            .take((MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)?;
    } else {
        std::fs::File::open(path)
            .context("Could not open Crew input file")?
            .take((MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)?;
    }
    ensure!(bytes.len() <= MAX_INPUT, "Crew input exceeds one MiB");
    String::from_utf8(bytes).context("Crew input must be UTF-8")
}

pub fn component(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "Crew IDs must contain 1–128 letters, digits, underscores or hyphens"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn terminal_controls_are_escaped_without_changing_json_shape() {
        let value = json!({"request_id":"req-1\u{202e}tail", "ok":true});
        let encoded = serde_json::to_string(&value).unwrap();
        let safe = json_terminal_safe(encoded);
        assert!(safe.contains("\\u202e"));
        assert!(!safe.contains('\u{202e}'));
    }

    /// CLI-4: the CLI's own isolates around a name survive a second pass; any other isolate,
    /// and every other control, is still escaped.
    #[test]
    fn a_balanced_isolate_pair_passes_through_and_nothing_else_does() {
        let named = "Ask \"\u{2068}Alice Chen\u{2069}\" (@alice) again.";
        assert_eq!(safe_text_keeping_isolates(named), named);
        assert_eq!(
            safe_text_keeping_isolates("\u{2068}#données\u{2069}: gone"),
            "\u{2068}#données\u{2069}: gone"
        );
        assert_eq!(
            safe_text_keeping_isolates("\u{2068}a\u{202e}b\u{2069}"),
            "\u{2068}a\\u{202e}b\u{2069}"
        );
        for (hostile, escaped) in [
            ("open \u{2068}forever", "open \\u{2068}forever"),
            ("\u{2069}close", "\\u{2069}close"),
            ("\u{2066}ltr\u{2069}", "\\u{2066}ltr\\u{2069}"),
            (
                "\u{2068}a\u{2068}b\u{2069}c\u{2069}",
                "\\u{2068}a\u{2068}b\u{2069}c\\u{2069}",
            ),
            ("bell\u{7}", "bell\\u{7}"),
        ] {
            assert_eq!(safe_text_keeping_isolates(hostile), escaped, "{hostile:?}");
        }
        // M14: a joiner outside the pairs keeps its neighbours too.
        let joined = "Posted 👩🏽\u{200d}🔬 to \u{2068}#données\u{2069}.";
        assert_eq!(safe_text_keeping_isolates(joined), joined);
    }

    #[test]
    fn terminal_controls_escape_without_losing_emoji_or_non_ascii_text() {
        let input = "safe🙂 café\u{202e}bidi\u{200b}zero\u{feff}bom\u{e0041}tag\n\x1b";
        let escaped = safe_text(input);
        assert!(escaped.contains("safe🙂 café"));
        assert!(escaped.contains("\\u{202e}"));
        assert!(escaped.contains("\\u{200b}"));
        assert!(escaped.contains("\\u{feff}"));
        assert!(escaped.contains("\\u{e0041}"));
        assert!(escaped.contains("\\n"));
        assert!(escaped.contains("\\u{1b}"));
        assert!(escaped.chars().all(|ch| !terminal_control(ch)));
    }

    /// M14: joiners inside emoji sequences and between letters of scripts that use them print
    /// as themselves; a joiner anywhere else is escaped, the spoof `crew\u{200d}_alice` first.
    #[test]
    fn joiners_print_where_they_join_text_and_are_escaped_everywhere_else() {
        for kept in [
            "👩🏽\u{200d}🔬",
            "👨\u{200d}👩\u{200d}👧\u{200d}👦",
            "🏳\u{fe0f}\u{200d}🌈",
            "❤\u{fe0f}\u{200d}🔥",
            "می\u{200c}خواهم",
            "क्\u{200d}ष",
            "क्\u{200c}ष",
        ] {
            assert_eq!(safe_text(kept), kept, "{kept:?}");
        }
        for (hostile, escaped) in [
            ("crew\u{200d}_alice", "crew\\u{200d}_alice"),
            ("ali\u{200c}ce", "ali\\u{200c}ce"),
            ("\u{200d}🔬", "\\u{200d}🔬"),
            ("🔬\u{200d}", "🔬\\u{200d}"),
            ("🔬\u{200d} 🔬", "🔬\\u{200d} 🔬"),
            ("🔬\u{200d}\u{200d}🔬", "🔬\\u{200d}\\u{200d}🔬"),
            ("🔬\u{200d}\u{202e}🔬", "🔬\\u{200d}\\u{202e}🔬"),
            ("é\u{200b}é", "é\\u{200b}é"),
        ] {
            assert_eq!(safe_text(hostile), escaped, "{hostile:?}");
        }
        // A message body keeps its emoji on every line.
        let body = plain(&message(
            "m",
            ALICE,
            NOW,
            "Emoji check: 👩🏽\u{200d}🔬 and می\u{200c}خواهم",
        ));
        assert!(
            body.ends_with("Emoji check: 👩🏽\u{200d}🔬 and می\u{200c}خواهم"),
            "{body}"
        );
    }

    #[test]
    fn json_terminal_safe_round_trips_invisible_non_bmp_tags() {
        let value = json!({
            "text": "emoji🙂 café\u{202e}bidi\u{200b}zero\u{feff}bom\u{e0041}tag\n",
            "plain": "東京"
        });
        let encoded = serde_json::to_string(&value).unwrap();
        let safe = json_terminal_safe(encoded);
        let reparsed: serde_json::Value = serde_json::from_str(&safe).unwrap();
        assert_eq!(reparsed, value);
        assert!(safe.contains("emoji🙂 café"));
        assert!(safe.contains("東京"));
        assert!(safe.chars().all(|ch| !terminal_control(ch)));
        for ch in ['\u{202e}', '\u{200b}', '\u{feff}', '\u{e0041}'] {
            assert!(
                !safe.contains(ch),
                "raw invisible character {ch:?} survived"
            );
        }
    }

    #[test]
    fn component_accepts_stable_ids_and_refuses_terminal_or_empty_values() {
        assert_eq!(component("request_01-abc").unwrap(), "request_01-abc");
        let oversized = "x".repeat(129);
        for invalid in ["", "bad/id", "bad\nline", oversized.as_str()] {
            assert!(component(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn read_input_is_bounded_and_requires_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid");
        std::fs::write(&valid, b"hello").unwrap();
        assert_eq!(read_input(&valid).unwrap(), "hello");
        let invalid = dir.path().join("invalid");
        std::fs::write(&invalid, [0xff]).unwrap();
        assert!(read_input(&invalid).is_err());
        let oversized = dir.path().join("oversized");
        let mut file = std::fs::File::create(&oversized).unwrap();
        file.write_all(&vec![b'x'; MAX_INPUT + 1]).unwrap();
        assert!(read_input(&oversized).is_err());
    }

    // Golden tests. The IDs and shapes below are the ones the three-account AWS fixture's
    // native CLI printed (`--output-format json`) on 2026-09-24; the S1a fields (`people`,
    // `former_principals`, invitation enrichment, `display_name`, `devices`, `handle`) follow
    // naming-design.md, since no broker emits them yet.
    const ALICE: &str = "a4fddc30-dc61-4832-9934-d47f89ef109d";
    const BOB: &str = "e03904b0-f8c6-4f33-8ea3-ac5ee2838682";
    const CAROL: &str = "9baa46a2-412a-4c92-b129-2fdbb440d08b";
    const DAVE: &str = "5f0c1e2d-3b4a-4c5d-8e6f-7a8b9c0d1e2f";
    const TEAM: &str = "d07f00df-8f40-4183-9923-7850c88815bc";
    const GENERAL: &str = "1c1f0647-41d2-444b-85fa-d5df5882a7f1";
    const METHODS: &str = "d0a509d4-df66-4575-988f-144afcf3a12a";
    const WORKSPACE: &str = "6fdafdf2-6c62-4878-81a0-65bb3fbe62f1";
    const CONNECTION: &str = "81788c18-b466-4e8e-b36c-49df694a0d9b";
    const RUN: &str = "d8d962a8-e434-4bc1-8398-59b60e6e49ea";
    const INVITATION: &str = "974f2aed-31a3-431c-8ada-7ded6edc5789";
    const REFERENCE: &str = "0b7e6f1a-2c3d-4e5f-9a8b-7c6d5e4f3a2b";
    const BLOB: &str = "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f";
    const TRANSFER: &str = "6a7b8c9d-0e1f-4a2b-9c3d-4e5f6a7b8c9d";
    const NODE: &str = "c1439346b5f638c4ecdc2153365fa6b54782d8bbf991d35cb4039b1ce129312a";
    const DEVICE: &str = "70dcffaf1751a59be3d734850d0d77edf2acfcea99b7da0c48695770b5f40d26";
    const KEY: &str = "b5481e87bf90b340be9acdbc5527b87ec69bdd418542286516bc8c9db27ff2cb";
    /// 2026-09-24 01:50:55 UTC, when the fixture's last history page was read.
    const NOW: i64 = 1_790_214_655;

    fn options(show_ids: bool, directory: Directory) -> HumanOptions {
        HumanOptions {
            show_ids,
            directory,
            clock: Clock::Fixed {
                now: NOW,
                offset_seconds: 0,
            },
            ..HumanOptions::default()
        }
    }

    fn plain(value: &Value) -> String {
        render_text(value, &options(false, Directory::default()))
    }

    fn named(value: &Value, snapshot: &Value) -> String {
        render_text(value, &options(false, Directory::from_snapshot(snapshot)))
    }

    fn with_ids(value: &Value, snapshot: &Value) -> String {
        render_text(value, &options(true, Directory::from_snapshot(snapshot)))
    }

    /// The display-name half of a person label: quoted, and isolated only when it is not ASCII.
    fn q(name: &str) -> String {
        if name.is_ascii() {
            format!("\"{name}\"")
        } else {
            format!("\"\u{2068}{name}\u{2069}\"")
        }
    }

    fn alice() -> String {
        format!("{} (@crew_alice)", q("Alice Chen"))
    }

    fn bob() -> String {
        format!("{} (@crew_bob)", q("Bob Lee"))
    }

    fn assert_no_machine_ids(text: &str) {
        let uuid = regex::Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-").unwrap();
        let hex = regex::Regex::new(r"(?i)[0-9a-f]{64}").unwrap();
        assert!(!uuid.is_match(text), "UUID in text output:\n{text}");
        assert!(!hex.is_match(text), "64-hex value in text output:\n{text}");
    }

    fn principal(id: &str, uid: u32, username: &str, nickname: &str) -> Value {
        json!({"id":id,"uid":uid,"username":username,"nickname":nickname,"avatar":null,"active":true})
    }

    fn channel(id: &str, name: &str, classification: &str, owner: &str, members: &[&str]) -> Value {
        json!({"id":id,"team_id":TEAM,"name":name,"created_by":owner,"owner_id":owner,
            "members":members,"archived":false,"classification":classification,"pending_owner":null})
    }

    /// Alice's snapshot from today's broker: no S1a fields.
    fn alice_snapshot() -> Value {
        let mut methods = channel(METHODS, "methods", "public_safe", BOB, &[ALICE, BOB]);
        methods["pending_owner"] = json!(ALICE);
        json!({
            "workspace": {"id":WORKSPACE,"host_uid":10001,"institution_id":"ucsf","mode":"private","policy_epoch":4},
            "protected_channel_ids": [],
            "actor": principal(ALICE, 10001, "crew_alice", "Alice Chen"),
            "principals": [
                principal(CAROL, 10003, "crew_carol", "crew_carol"),
                principal(ALICE, 10001, "crew_alice", "Alice Chen"),
                principal(BOB, 10002, "crew_bob", "Bob Lee"),
            ],
            "teams": [{"id":TEAM,"name":"Crew QA Lab","created_by":ALICE,
                "members":[CAROL,ALICE,BOB],"general_channel_id":GENERAL}],
            "channels": [channel(GENERAL, "general", "restricted", ALICE, &[CAROL, ALICE, BOB]), methods],
            "invitations": [{"id":INVITATION,"kind":"channel","target_id":METHODS,
                "principal_id":CAROL,"inviter_id":ALICE,"expires_at":1_790_300_395}],
            "runs": [{"id":RUN,"owner_id":ALICE,"channel_id":GENERAL,"source_channels":[GENERAL],
                "policy_epoch":4,"expires_at":NOW + 2700,"revoked":false}],
            "read_positions": {GENERAL: "f9613140-dc3d-4ed9-a634-f36b108888d2"},
            "unread": {GENERAL: 2, METHODS: 0},
            "references": [{"id":REFERENCE,"channel_id":GENERAL,"owner_id":BOB,"path":"/project/results",
                "label":"Remote results","restricted":true,"source_channels":[GENERAL],"verified":false}],
        })
    }

    /// Bob's snapshot from an S1a/S2a broker: projected names, a former member and an enriched
    /// invitation.
    fn bob_snapshot() -> Value {
        let mut actor = principal(BOB, 10002, "crew_bob", "Bob Lee");
        actor["display_name"] = json!("Bob Lee");
        actor["devices"] = json!([{"fingerprint":"70DC-FFAF-1751-A59B","added_at":1_790_213_983,"added_via":"token"}]);
        let mut alice = principal(ALICE, 10001, "crew_alice", "Alice Chen");
        alice["display_name"] = json!("Alice Chen");
        json!({
            "workspace": {"id":WORKSPACE,"host_uid":10001,"institution_id":"ucsf","mode":"private",
                "policy_epoch":4,"name":"lab","host_principal_id":ALICE},
            "actor": actor.clone(),
            "principals": [alice, actor],
            "former_principals": [{"id":DAVE,"username":"dave","display_name":"Dave Old","avatar":null,"active":false}],
            "teams": [{"id":TEAM,"name":"Crew QA Lab","handle":"crew-qa-lab","created_by":ALICE,
                "members":[ALICE,BOB,DAVE],"general_channel_id":GENERAL}],
            "channels": [channel(GENERAL, "general", "restricted", ALICE, &[ALICE, BOB, DAVE])],
            "invitations": [{"id":INVITATION,"kind":"channel","target_id":METHODS,"principal_id":BOB,
                "inviter_id":ALICE,"expires_at":1_790_300_395,"target_name":"methods",
                "team_name":"Crew QA Lab","inviter":{"username":"crew_alice","display_name":"Alice Chen"},
                "expired":false}],
            "runs": [], "read_positions": {}, "unread": {}, "references": [],
        })
    }

    fn message(id: &str, actor: &str, created_at: i64, body: &str) -> Value {
        json!({"id":id,"sequence":id,"channel_id":GENERAL,"actor_id":actor,"run_id":null,"body":body,
            "created_at":created_at,"restricted":true,"source_channels":[GENERAL],
            "attachments":[],"references":[],"status":null})
    }

    fn history() -> Value {
        let mut agent = message(
            "2d79401b-e3fa-4f05-9f3d-cb6b61662a1d",
            ALICE,
            1_790_214_503,
            "Summary ready.\r\nSecond line of the summary.\n\n",
        );
        agent["run_id"] = json!(RUN);
        agent["status"] = json!("completed");
        agent["attachments"] = json!([BLOB]);
        agent["references"] = json!([REFERENCE]);
        json!({"messages": [
            message("f9613140-dc3d-4ed9-a634-f36b108888d2", ALICE, 1_790_214_015,
                "CREWQA_SETUP_ALICE_001 Hello team, Alice here (synthetic fixture message)."),
            message("794a4785-49af-4874-8dfe-1405eec3fea3", BOB, 1_790_214_015,
                "CREWQA_SETUP_BOB_002 Hi all, Bob checking in (synthetic fixture message)."),
            message("f0e7172c-c5b7-425d-9c9e-ebb7bd372b4e", CAROL, 1_790_214_015,
                "CREWQA_SETUP_CAROL_003 Carol here, ready to go (synthetic fixture message)."),
            agent,
        ], "cursor": "2d79401b-e3fa-4f05-9f3d-cb6b61662a1d"})
    }

    fn connection() -> Value {
        json!({"id":CONNECTION,"node_id":NODE,"name":"Bob UCSF","ssh_target":"crew_bob@34.217.178.174",
            "port":22,"identity_file":"/private/tmp/crew-ui-redesign/fixture/aws/crew_bob","proxy_jump":null,
            "socket_path":"/tmp/crew-10001-8d7fa7ec81f84fa7ac3cac1b27433378/broker.sock","owner_uid":10001,
            "workspace_id":WORKSPACE,"workspace_public_key":KEY,"remote_root":"/home/crew_bob/crew-work",
            "remote_execution":true,"cluster_connection_id":"be2be26b-9322-4568-be69-4303f087f564",
            "mode":"private","institution_id":"ucsf","policy_epoch":2,"status":"connected",
            "last_error":null,"device_id":DEVICE,"public_key":KEY})
    }

    fn task() -> Value {
        json!({"run_id":RUN,"connection_id":CONNECTION,"channel_id":GENERAL,"session_id":"20260924_2",
            "status":"failed","error":"Authentication error: Authentication failed. Status: 401 Unauthorized. Response: {\"error\":\"Invalid client id or secret\"}"})
    }

    fn grants() -> Value {
        json!({"grants":[{"session_id":"20260924_3","run_id":RUN,"connection_id":CONNECTION,
            "channel_id":METHODS,"source_channels":[METHODS, GENERAL],"policy_epoch":4,"expired":false}]})
    }

    fn privacy() -> Value {
        json!({"connection_id":CONNECTION,"personal_mode":"private","institution_id":"ucsf",
            "connection_policy_epoch":2,
            "workspace":{"id":WORKSPACE,"host_uid":10001,"institution_id":"ucsf","mode":"private","policy_epoch":4},
            "channels":[channel(GENERAL, "general", "restricted", ALICE, &[CAROL, ALICE, BOB])]})
    }

    fn transfers() -> Value {
        json!({"transfers":[
            {"id":TRANSFER,"request_id":"fixture-upload-001","connection_id":CONNECTION,"channel_id":METHODS,
                "direction":"upload","name":"counts.csv","size":55,"sha256":NODE,"offset":22,"blob_id":null,
                "state":"uploading","error":null,"binding":KEY,"destination_identity":null},
            {"id":"7b8c9d0e-1f2a-4b3c-8d4e-5f6a7b8c9d0e","request_id":"fixture-download-002",
                "connection_id":CONNECTION,"channel_id":GENERAL,"direction":"download","name":"plot.png",
                "size":1_572_864,"sha256":NODE,"offset":1_572_864,"blob_id":BLOB,"state":"completed",
                "error":null,"binding":KEY,"destination_identity":DEVICE},
        ]})
    }

    #[test]
    fn history_names_people_and_keeps_ids_out() {
        let text = named(&history(), &alice_snapshot());
        let expected = [
            format!("{} · 01:40  CREWQA_SETUP_ALICE_001 Hello team, Alice here (synthetic fixture message).", alice()),
            format!("{} · 01:40  CREWQA_SETUP_BOB_002 Hi all, Bob checking in (synthetic fixture message).", bob()),
            "@crew_carol · 01:40  CREWQA_SETUP_CAROL_003 Carol here, ready to go (synthetic fixture message).".into(),
            format!("{} · agent · Done · 01:48  Summary ready.", alice()),
            "    Second line of the summary.".into(),
            "    1 attachment".into(),
            "    Remote reference: Remote results (/project/results)".into(),
        ]
        .join("\n");
        assert_eq!(text, expected);
        assert_no_machine_ids(&text);
    }

    /// DW-17: an attachment the command looked up is named with its size, beside its ID with
    /// `--show-ids`; one it could not look up is counted.
    #[test]
    fn history_names_each_attachment_it_looked_up() {
        let mut page = history();
        page["messages"][3]["attachments"] = json!([BLOB, TRANSFER]);
        page["messages"][3]["attachment_details"] =
            json!({BLOB: {"name": "counts.csv", "size": 56_320, "media_type": "text/csv"}});
        let text = named(&page, &alice_snapshot());
        assert!(
            text.contains("    Attachment: counts.csv (55 KB)\n    1 more attachment\n"),
            "{text}"
        );
        assert_no_machine_ids(&text);
        let ids = with_ids(&page, &alice_snapshot());
        assert!(
            ids.contains(&format!("    Attachment: counts.csv (55 KB) [attachment ID {BLOB}]\n    1 more attachment [attachment IDs {TRANSFER}]")),
            "{ids}"
        );
    }

    /// AG-F7: `crew context` shows the grant's scope, then the messages oldest first.
    #[test]
    fn a_chats_context_shows_its_access_then_its_messages_oldest_first() {
        let mut manifest = json!({"run_id": RUN, "policy_epoch": 4,
            "source_channels": [METHODS, GENERAL], "restricted": true,
            "people": {BOB: {"username": "crew_bob", "display_name": "Bob Lee"}},
            "channel_names": {METHODS: "methods", GENERAL: "general"}});
        manifest["messages"] = json!([
            message("m-2", BOB, NOW, "Second."),
            message("m-1", BOB, NOW - 60, "First."),
        ]);
        let lines = context_lines(
            &manifest,
            Some(METHODS),
            &options(false, Directory::default()),
        );
        assert_eq!(lines[0], "Access: #methods · also reads #general");
        let first = lines
            .iter()
            .position(|line| line.ends_with("First."))
            .unwrap();
        let second = lines
            .iter()
            .position(|line| line.ends_with("Second."))
            .unwrap();
        assert!(first < second, "{lines:?}");
        let unknown = context_lines(&manifest, None, &options(false, Directory::default()));
        assert_eq!(unknown[0], "Access: reads #methods, #general");
    }

    #[test]
    fn history_without_names_degrades_to_unknown_member_and_marks_restriction() {
        let text = plain(&history());
        assert!(text
            .starts_with("Unknown member · 01:40 · Restricted  CREWQA_SETUP_ALICE_001 Hello team"));
        assert!(text.contains("    Remote reference\n") || text.ends_with("    Remote reference"));
        assert_no_machine_ids(&text);
        assert_eq!(plain(&json!({"messages":[],"cursor":null})), "No messages.");
    }

    #[test]
    fn history_uses_the_brokers_people_map_and_names_former_members() {
        let page = json!({
            "messages": [message("0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d", DAVE, NOW - 86_400 * 3, "Old notes.")],
            "people": {DAVE: {"username":"dave","display_name":"Dave Old","active":false}},
            "channel_names": {GENERAL: "general"},
            "cursor": "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
        });
        let text = plain(&page);
        assert_eq!(
            text,
            format!(
                "{} (@dave) · former member · Sep 21 01:50 · Restricted  Old notes.",
                q("Dave Old")
            )
        );
        assert_no_machine_ids(&text);
    }

    /// Live finding: a member who was removed and joined again has a new principal, and what
    /// they wrote under the old one is shown as that member, not as a former member. Another
    /// account that once had the name (another UID) stays a former member.
    #[test]
    fn an_earlier_entry_of_a_member_again_is_shown_as_that_member() {
        let snapshot = json!({
            "workspace": {"id": WORKSPACE, "name": "lab", "host_uid": 1000},
            "actor": {"id": ALICE, "username": "alice", "uid": 1000},
            "principals": [
                {"id": ALICE, "username": "alice", "uid": 1000},
                {"id": BOB, "username": "dave", "display_name": "Dave Old", "uid": 1022}
            ],
            "former_principals": [
                {"id": DAVE, "username": "dave", "display_name": "Dave Old", "uid": 1022, "active": false},
                {"id": CAROL, "username": "carol", "uid": 1030, "active": false}
            ]
        });
        let page = json!({
            "messages": [message("0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d", DAVE, NOW - 86_400 * 3, "Old notes.")],
            "people": {DAVE: {"username":"dave","display_name":"Dave Old","active":false}},
            "channel_names": {GENERAL: "general"},
            "cursor": "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
        });
        let text = named(&page, &snapshot);
        assert_eq!(
            text,
            format!(
                "{} (@dave) · Sep 21 01:50 · Restricted  Old notes.",
                q("Dave Old")
            )
        );
        let directory = Directory::from_snapshot(&snapshot);
        assert_eq!(
            authority_label(&directory, DAVE, false),
            format!("{} (@dave)", q("Dave Old"))
        );
        let shown = named(&snapshot, &snapshot);
        assert!(
            shown.contains(&format!("  {} (@dave) · a member again", q("Dave Old"))),
            "{shown}"
        );
        assert!(shown.contains("  @carol · former member"), "{shown}");
        // The same name held by another account now is not the same person.
        let mut other = snapshot.clone();
        other["principals"][1]["uid"] = json!(1099);
        assert_eq!(
            authority_label(&Directory::from_snapshot(&other), DAVE, false),
            format!("{} (@dave) · former member", q("Dave Old"))
        );
    }

    /// The restart refusals made before anything is stopped are said in the server's own
    /// words, with no command to start the workspace by hand: it still runs, and that command
    /// would be refused the same way.
    #[test]
    fn a_restart_refused_before_any_stop_says_why_in_its_own_words() {
        let status = json!({
            "intent": "install",
            "outcome": {"installed_version": "1.94.2", "restart_failed": [
                {"workspace_id": "a", "name": "lab", "code": "identity_conflict",
                 "message": "lab on leo stopped but didn't start again. Start it on leo with x.",
                 "command": "biorouter-crew start --workspace-id a",
                 "detail": "Error: identity_conflict: Workspace a is kept in more than one folder of ~/.local/share/biorouter-crew (lab, lab-2). Move every copy out of that folder, keeping the workspace's own, then try again."},
                {"workspace_id": "b", "name": "old-lab", "code": "program_missing",
                 "message": "old-lab on leo stopped but didn't start again. Start it on leo with x.",
                 "command": "biorouter-crew start --workspace-id b",
                 "detail": "{\"error\": \"program_missing: /home/w/.local/bin/biorouter-crew is missing or cannot run, so the running broker was left as it is.\"}"},
                {"workspace_id": "c", "name": "bare-lab", "code": "program_missing",
                 "message": "bare-lab stopped.", "command": "biorouter-crew start --workspace-id c"}
            ]}
        });
        let lines = setup_summary_lines(&status, "wanjun@leo", None, &SetupAsked::default());
        assert_eq!(
            lines[1..],
            [
                "lab: Workspace a is kept in more than one folder of ~/.local/share/biorouter-crew (lab, lab-2). Move every copy out of that folder, keeping the workspace's own, then try again.",
                "old-lab: /home/w/.local/bin/biorouter-crew is missing or cannot run, so the running broker was left as it is. Install it again with biorouter crew setup --server wanjun@leo.",
                "bare-lab: The Crew program in your account on wanjun@leo is missing or can't run, so it was left as it was. Install it again with biorouter crew setup --server wanjun@leo.",
            ]
        );
    }

    /// L3: the listing a job reports when it ends is the one printed, so a workspace it just
    /// started or restarted reads as running on the new Crew, never as it was before.
    #[test]
    fn the_listing_after_a_start_shows_what_runs_now() {
        let status = json!({
            "intent": "install",
            "server": {"login": "wanjun@leo"},
            "workspaces": [{"workspace_id": "a", "name": "acct-lab", "role": "host", "state": "running",
                            "version": "1.94.2", "binary_current": true, "this_computer": "connected"}],
            "outcome": {"installed_version": "1.94.2", "reused": true,
                        "restarted": [{"workspace_id": "a", "name": "acct-lab", "to_version": "1.94.2"}]}
        });
        assert_eq!(
            setup_summary_lines(&status, "wanjun@leo", None, &SetupAsked::default()),
            [
                "Crew 1.94.2 was already installed in your account on wanjun@leo.",
                "Workspaces on wanjun@leo for your account:",
                "  acct-lab · you host it · running Crew 1.94.2 · connected on this computer",
                "acct-lab now runs Crew 1.94.2.",
            ]
        );
    }

    #[test]
    fn show_ids_appends_the_ids_to_history() {
        let text = with_ids(&history(), &alice_snapshot());
        for id in [
            ALICE,
            BOB,
            CAROL,
            RUN,
            BLOB,
            REFERENCE,
            "f9613140-dc3d-4ed9-a634-f36b108888d2",
        ] {
            assert!(text.contains(id), "{id} missing from:\n{text}");
        }
        assert!(text.ends_with("Cursor: 2d79401b-e3fa-4f05-9f3d-cb6b61662a1d"));
        assert!(text.contains(&format!("{} [ID {ALICE}] · 01:40 [message ID f9613140-dc3d-4ed9-a634-f36b108888d2]  CREWQA_SETUP_ALICE_001", alice())));
    }

    #[test]
    fn legacy_snapshot_renders_names_roles_and_counts() {
        let text = plain(&alice_snapshot());
        let expected = [
            format!("Workspace: {}'s workspace", alice()),
            "Privacy: Private for everyone · institution ucsf · policy epoch 4".into(),
            format!("You: {}", alice()),
            "People (3):".into(),
            format!("  {} · you · host", alice()),
            format!("  {}", bob()),
            "  @crew_carol".into(),
            "Teams (1):".into(),
            "  Crew QA Lab · 3 members · created by you".into(),
            "Channels (2):".into(),
            "  #general · Crew QA Lab · Restricted · 3 members · you own it · 2 unread".into(),
            format!(
                "  #methods · Crew QA Lab · Public-safe · 2 members · owner {} · offered to you",
                bob()
            ),
            "Invitations (1):".into(),
            "  You invited @crew_carol to #methods in Crew QA Lab · expires in 24 hours".into(),
            "Remote references (1):".into(),
            format!(
                "  Remote results · /project/results · #general · shared by {} · Not uploaded",
                bob()
            ),
            "Agent grants (1):".into(),
            "  #general in Crew QA Lab · expires in 45 minutes".into(),
        ]
        .join("\n");
        assert_eq!(text, expected);
        assert_no_machine_ids(&text);
    }

    /// M17, F6, SF-F10: lists read in one order everywhere, not in the order of random IDs:
    /// channels by team with each `#general` first, teams by name, and people with the host
    /// first, then you, then by the name their row leads with.
    #[test]
    fn channels_teams_and_people_are_listed_in_a_readers_order() {
        let team = |id: &str, name: &str, general: &str| json!({"id": id, "name": name, "general_channel_id": general, "members": []});
        let place = |id: &str, team: &str, name: &str| json!({"id": id, "team_id": team, "name": name, "classification": "restricted"});
        let teams = vec![
            team("t-b", "bench crew", "c-4"),
            team("t-a", "Chen Lab", "c-6"),
        ];
        let mut channels = vec![
            place("c-1", "t-a", "msg-qa"),
            place("c-2", "t-a", "random"),
            place("c-3", "t-a", "methods"),
            place("c-4", "t-b", "general"),
            place("c-5", "t-b", "scratch"),
            place("c-6", "t-a", "general"),
            place("c-7", "t-gone", "orphan"),
        ];
        channels_in_order(&mut channels, &teams);
        let order: Vec<&str> = channels
            .iter()
            .map(|channel| channel["id"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["c-4", "c-5", "c-6", "c-3", "c-1", "c-2", "c-7"]);

        let mut sorted_teams = teams.clone();
        teams_in_order(&mut sorted_teams);
        assert_eq!(sorted_teams[0]["id"], "t-b");

        let mut people = vec![
            json!({"id": "p-jack", "username": "crew_jack", "display_name": "Jack Moreno"}),
            json!({"id": "p-mal", "username": "crew_mallory"}),
            json!({"id": "p-alice", "username": "crew_alice", "display_name": "Alice Chen"}),
            json!({"id": "p-carol", "username": "crew_carol", "display_name": "carol nguyen"}),
            json!({"id": "p-bob", "username": "crew_bob", "display_name": "Bob Lee"}),
        ];
        people_in_order(&mut people, Some("p-alice"), Some("p-mal"));
        let order: Vec<&str> = people
            .iter()
            .map(|person| person["id"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["p-alice", "p-mal", "p-bob", "p-carol", "p-jack"]);
    }

    /// The broker lists as many teams and channels as fit in one answer (`totals` counts them
    /// all), so a list that leaves some out says so, and one that does not says nothing more.
    #[test]
    fn a_partial_list_says_how_many_it_shows() {
        assert_eq!(
            partial_list_note("channels", 100, Some(140)).as_deref(),
            Some("Showing 100 of your 140 channels.")
        );
        assert_eq!(partial_list_note("channels", 3, Some(3)), None);
        assert_eq!(partial_list_note("channels", 3, None), None);
        let mut snapshot = alice_snapshot();
        snapshot["totals"] =
            json!({"teams": 1, "channels": 5, "invitations": 1, "runs": 1, "references": 1});
        let text = plain(&snapshot);
        assert!(
            text.contains("Channels (2):\n  #general · Crew QA Lab · Restricted · 3 members · you own it · 2 unread\n  #methods"),
            "{text}"
        );
        assert!(text.contains("\n  Showing 2 of your 5 channels."), "{text}");
        assert!(!text.contains("of your 1 teams"), "{text}");
    }

    /// M18 with W2-BRK-6: a person the broker reports online says so; a broker that reports no
    /// presence shows none.
    #[test]
    fn members_the_broker_reports_online_say_so() {
        let mut snapshot = alice_snapshot();
        snapshot["online_principal_ids"] = json!([BOB]);
        let people = render_text(
            &snapshot["principals"],
            &options(false, Directory::from_snapshot(&snapshot)).with_view(View::Members),
        );
        assert_eq!(
            people,
            format!("@crew_carol\n{} · you · host\n{} · online", alice(), bob())
        );
        assert!(!plain(&alice_snapshot()["principals"]).contains("online"));
    }

    #[test]
    fn enriched_snapshot_names_the_workspace_host_former_members_and_invitations() {
        let text = plain(&bob_snapshot());
        let expected = [
            format!("Workspace: lab · hosted by {}", alice()),
            "Privacy: Private for everyone · institution ucsf · policy epoch 4".into(),
            format!("You: {}", bob()),
            "People (2):".into(),
            format!("  {} · host", alice()),
            format!("  {} · you", bob()),
            "Former members (1):".into(),
            format!("  {} (@dave) · former member", q("Dave Old")),
            "Teams (1):".into(),
            // Dave, a former member, is still on the team's and channel's lists; he is not
            // counted (live re-test).
            format!(
                "  Crew QA Lab (crew-qa-lab) · 2 members · created by {}",
                alice()
            ),
            "Channels (1):".into(),
            format!(
                "  #general · Crew QA Lab · Restricted · 2 members · owner {}",
                alice()
            ),
            "Invitations (1):".into(),
            format!(
                "  {} invited you to #methods in Crew QA Lab · expires in 24 hours",
                alice()
            ),
        ]
        .join("\n");
        assert_eq!(text, expected);
        assert_no_machine_ids(&text);
    }

    #[test]
    fn list_views_take_names_from_the_snapshot_directory() {
        let snapshot = alice_snapshot();
        assert_eq!(
            render_text(
                &snapshot["principals"],
                &options(false, Directory::from_snapshot(&snapshot)).with_view(View::Members)
            ),
            format!("@crew_carol\n{} · you · host\n{}", alice(), bob())
        );
        assert_eq!(
            named(&snapshot["teams"], &snapshot),
            "Crew QA Lab · 3 members · created by you"
        );
        let channels = named(&snapshot["channels"], &snapshot);
        assert!(channels.starts_with("#general · Crew QA Lab · Restricted"));
        let invites = named(&snapshot["invitations"], &snapshot);
        assert_eq!(
            invites,
            "You invited @crew_carol to #methods in Crew QA Lab · expires in 24 hours\nAdd --show-ids for the IDs invites accept and other invitation commands take."
        );
        let profile = named(&bob_snapshot()["actor"], &bob_snapshot());
        assert_eq!(
            profile,
            format!("{} · you\n  Computers enrolled as you (1):\n    70DC-FFAF-1751-A59B · added Sep 24, 2026 · with an invitation token", bob())
        );
        for text in [channels, invites, profile] {
            assert_no_machine_ids(&text);
        }
    }

    #[test]
    fn list_views_without_a_directory_still_print_no_ids() {
        let snapshot = alice_snapshot();
        assert_eq!(
            plain(&snapshot["principals"]),
            format!("@crew_carol\n{}\n{}", alice(), bob())
        );
        assert_eq!(
            plain(&snapshot["teams"]),
            "Crew QA Lab · 3 members · created by Unknown member"
        );
        let invites = plain(&bob_snapshot()["invitations"]);
        assert!(invites.starts_with(&format!(
            "{} invited you to #methods in Crew QA Lab · expires in 24 hours",
            alice()
        )));
        let legacy = plain(&snapshot["invitations"]);
        assert!(legacy.starts_with(
            "Unknown member invited Unknown member to this channel · expires in 24 hours"
        ));
        for value in [
            &snapshot["channels"],
            &snapshot["invitations"],
            &snapshot["runs"],
        ] {
            assert_no_machine_ids(&plain(value));
        }
        assert_eq!(plain(&json!([])), "No items.");
        assert_eq!(
            render_text(
                &json!([]),
                &options(false, Directory::default()).with_view(View::Teams)
            ),
            "No teams."
        );
    }

    #[test]
    fn connections_render_one_row_each_and_details_on_show() {
        let list = json!({"connections":[connection()]});
        assert_eq!(
            plain(&list),
            "Bob UCSF · crew_bob@34.217.178.174 · Connected · Private (ucsf)"
        );
        let show = plain(&connection());
        assert_eq!(
            show,
            [
                "Bob UCSF",
                "  Server: crew_bob@34.217.178.174",
                "  Status: Connected",
                "  Privacy: Private · institution ucsf · policy epoch 2",
                "  Remote folder: /home/crew_bob/crew-work · agents may run commands there",
                "  SSH key file: /private/tmp/crew-ui-redesign/fixture/aws/crew_bob",
            ]
            .join("\n")
        );
        assert_no_machine_ids(&show);
        let details = with_ids(&connection(), &json!({}));
        for id in [CONNECTION, WORKSPACE, NODE, DEVICE, KEY] {
            assert!(details.contains(id), "{id} missing from:\n{details}");
        }
        assert!(details.contains("  Host UID: 10001"));
        assert_eq!(plain(&json!({"connections":[]})), "No saved connections.");
    }

    /// SF-F1: every surface gives one answer to effective privacy, Private when the connection
    /// or the workspace is, and says the connection's own setting as that.
    #[test]
    fn a_public_connection_in_a_private_workspace_reads_private_everywhere() {
        let mut public = connection();
        public["name"] = json!("okafor-lab");
        public["mode"] = json!("public");
        public["institution_id"] = json!("stanford");
        let mut annotated = public.clone();
        annotated["effective_mode"] = json!("private");
        annotated["workspace_mode"] = json!("private");
        annotated["workspace_name"] = json!("okafor-lab");
        assert_eq!(
            plain(&json!({"connections": [annotated.clone()]})),
            "okafor-lab · crew_bob@34.217.178.174 · Connected · Private because okafor-lab is Private for everyone · your connection: Public"
        );
        let show = plain(&annotated);
        assert!(
            show.contains("  Privacy: Private (okafor-lab is Private for everyone)\n  Your connection: Public · institution stanford · policy epoch 2"),
            "{show}"
        );
        // Connected but unanswered, it says it can't be checked right now.
        let mut unanswered = public.clone();
        unanswered["effective_mode"] = Value::Null;
        assert!(plain(&unanswered).contains("  Privacy: can't be checked right now\n"));
        // Offline, the workspace's setting can't be read, and it says so.
        let mut offline = public.clone();
        offline["status"] = json!("disconnected");
        offline["effective_mode"] = Value::Null;
        assert!(plain(&json!({"connections": [offline.clone()]})).ends_with(
            "Disconnected · Your connection: Public · workspace privacy unknown while disconnected"
        ));
        assert!(plain(&offline)
            .contains("  Privacy: can't be checked while disconnected\n  Your connection: Public"));
        // A Private connection is Private whatever the workspace allows, and reads as before.
        let mut private = connection();
        private["effective_mode"] = json!("private");
        assert_eq!(
            plain(&json!({"connections": [private]})),
            "Bob UCSF · crew_bob@34.217.178.174 · Connected · Private (ucsf)"
        );
        // privacy show leads with the effective privacy.
        let mut shown = privacy();
        shown["personal_mode"] = json!("public");
        shown["workspace"]["name"] = json!("okafor-lab");
        assert!(plain(&shown).starts_with(
            "Privacy: Private (okafor-lab is Private for everyone)\nYour connection: Public"
        ));
        assert_eq!(
            effective_mode(Some("public"), Some("public")),
            Some("public")
        );
        assert_eq!(effective_mode(Some("public"), None), None);
        assert_eq!(effective_mode(Some("private"), None), Some("private"));
    }

    /// A last error the daemon typed with a connect code leads with what to do; an untyped
    /// one, or one typed with a code of its own (`crew_membership_ended`), reads as it is.
    #[test]
    fn a_typed_last_error_says_what_to_do_first() {
        let mut failed = connection();
        failed["status"] = json!("disconnected");
        failed["last_error"] = json!("Crew SSH failure [ssh_eof]: ssh exited");
        failed["last_error_code"] = json!("crew_ssh_unreachable");
        let rows = plain(&json!({"connections": [failed.clone()]}));
        assert!(
            rows.ends_with("  Last error: Couldn't reach the server. Check your network or your VPN, then connect again.\n    Details: Crew SSH failure [ssh_eof]: ssh exited"),
            "{rows}"
        );
        failed["last_error"] = json!("This computer is no longer a member of lab.");
        failed["last_error_code"] = json!("crew_membership_ended");
        assert!(
            plain(&failed).contains("  Last error: This computer is no longer a member of lab.")
        );
    }

    #[test]
    fn tasks_grants_and_privacy_name_their_channels() {
        let snapshot = alice_snapshot();
        assert_eq!(
            named(&json!({"runs":[task()]}), &snapshot),
            [
                "Task posting to #general in Crew QA Lab · Couldn't finish",
                "  Chat: 20260924_2",
                "  Error: Authentication error: Authentication failed. Status: 401 Unauthorized. Response: {\"error\":\"Invalid client id or secret\"}",
                "Add --show-ids for the IDs tasks show, watch and cancel take.",
            ]
            .join("\n")
        );
        assert_eq!(
            plain(&task()).lines().next(),
            Some("Task posting to this channel · Couldn't finish")
        );
        let mut titled = task();
        titled["title"] = json!("Summarize counts.csv for the methods section and post a table");
        assert!(plain(&titled).starts_with(&format!(
            "Task {} posting to this channel",
            q("Summarize counts.csv for the methods section and post a tab…")
        )));
        assert_eq!(
            named(&grants(), &snapshot),
            "Chat 20260924_3 → #methods in Crew QA Lab (also reads #general) · Active · policy epoch 4"
        );
        assert_eq!(
            named(&privacy(), &snapshot),
            [
                "Privacy: Private",
                "Your connection: Private · institution ucsf · policy epoch 2",
                "Workspace: Private for everyone · institution ucsf · policy epoch 4",
                "Channels (1):",
                "  #general · Crew QA Lab · Restricted",
                "Pass the policy epochs to --expected-policy-epoch and --expected-workspace-policy-epoch to refuse a changed policy.",
            ]
            .join("\n")
        );
        assert_eq!(plain(&json!({"runs":[]})), "No tasks.");
        assert_eq!(plain(&json!({"grants":[]})), "No chats have Crew access.");
        let ids = with_ids(&json!({"runs":[task()]}), &snapshot);
        assert!(ids.contains(&format!("[task ID {RUN}]")) && ids.contains(CONNECTION));
        assert!(with_ids(&grants(), &snapshot).contains(RUN));
        assert!(with_ids(&privacy(), &snapshot).contains(WORKSPACE));
    }

    /// F4: the text list says what the JSON says. A grant whose `expires_at` has passed never
    /// reads Active; a task is a task; the chat is named by its title; a stop reads as the
    /// revocation it was, or as not yet confirmed while the workspace has not confirmed it.
    #[test]
    fn grants_text_matches_the_json() {
        let snapshot = alice_snapshot();
        let row = |fields: Value| {
            let mut grant = grants()["grants"][0].clone();
            for (key, value) in fields.as_object().unwrap() {
                grant[key] = value.clone();
            }
            named(&json!({ "grants": [grant] }), &snapshot)
        };
        let base = "→ #methods in Crew QA Lab (also reads #general)";
        for (fields, expected) in [
            (
                json!({"kind": "chat", "session_name": "Assay results summary", "expires_at": NOW - 11 * 3600}),
                format!("Chat {} (chat 20260924_3) {base} · Expired · policy epoch 4", q("Assay results summary")),
            ),
            (
                json!({"kind": "chat", "expires_at": NOW + 45 * 60}),
                format!("Chat 20260924_3 {base} · Active · ends in 45 minutes · policy epoch 4"),
            ),
            (
                json!({"kind": "task", "session_name": "Summarize counts", "expires_at": NOW + 45 * 60}),
                format!("Task {} (task 20260924_3) {base} · Active · ends in 45 minutes · policy epoch 4", q("Summarize counts")),
            ),
            (
                json!({"kind": "task", "expired": true, "revocation": "confirmed", "remote_revocation_confirmed": true}),
                format!("Task 20260924_3 {base} · Ended · policy epoch 4"),
            ),
            // AGT2-N3: a task the person stopped reads Stopped, as its card in the channel does.
            (
                json!({"kind": "task", "expired": true, "revocation": "confirmed", "remote_revocation_confirmed": true, "task_status": "cancelled"}),
                format!("Task 20260924_3 {base} · Stopped · policy epoch 4"),
            ),
            (
                json!({"kind": "task", "expired": true, "revocation": "confirmed", "remote_revocation_confirmed": true, "task_status": "completed"}),
                format!("Task 20260924_3 {base} · Ended · policy epoch 4"),
            ),
            (
                json!({"kind": "task", "expires_at": NOW - 60, "task_status": "cancelled"}),
                format!("Task 20260924_3 {base} · Stopped · policy epoch 4"),
            ),
            (
                json!({"kind": "task", "expires_at": NOW - 60}),
                format!("Task 20260924_3 {base} · Ended · policy epoch 4"),
            ),
            (
                json!({"kind": "chat", "expired": true, "revocation": "confirmed", "remote_revocation_confirmed": true}),
                format!("Chat 20260924_3 {base} · Revoked · policy epoch 4"),
            ),
            (
                json!({"kind": "chat", "expired": true}),
                format!("Chat 20260924_3 {base} · Revoked · policy epoch 4"),
            ),
            (
                json!({"kind": "chat", "expired": true, "revocation": "unconfirmed", "remote_revocation_confirmed": false}),
                format!("Chat 20260924_3 {base} · Stopped on this computer; the workspace hasn't confirmed yet · policy epoch 4"),
            ),
            (
                json!({"kind": "chat", "expired": true, "revocation": "ended_by_workspace", "remote_revocation_confirmed": null}),
                format!("Chat 20260924_3 {base} · Ended: Crew settings changed · policy epoch 4"),
            ),
        ] {
            assert_eq!(row(fields.clone()), expected, "{fields}");
        }
    }

    /// F3: an earlier grant the daemon keeps for its revocation (`replaced_grants`) is listed
    /// after every chat's grant, under its own heading, never as the chat's access; while it
    /// is unconfirmed the list says the daemon confirms it by itself. The list without any
    /// reads as it always did.
    #[test]
    fn replaced_grants_are_listed_apart_from_every_chats_grant() {
        let snapshot = alice_snapshot();
        let base = "→ #methods in Crew QA Lab (also reads #general)";
        let current = grants()["grants"][0].clone();
        let mut earlier = current.clone();
        earlier["run_id"] = json!("run-earlier");
        earlier["expired"] = json!(true);
        earlier["kind"] = json!("chat");
        earlier["revocation"] = json!("unconfirmed");
        earlier["remote_revocation_confirmed"] = json!(false);
        assert_eq!(
            named(
                &json!({"grants": [current.clone()], "replaced_grants": [earlier.clone()]}),
                &snapshot
            ),
            [
                format!("Chat 20260924_3 {base} · Active · policy epoch 4"),
                "Earlier access no chat holds any more:".into(),
                format!("  Chat 20260924_3 {base} · Stopped on this computer; the workspace hasn't confirmed yet · policy epoch 4"),
                "Biorouter confirms these with the workspace by itself whenever the connection is up.".into(),
            ]
            .join("\n")
        );
        earlier["revocation"] = json!("confirmed");
        earlier["remote_revocation_confirmed"] = json!(true);
        assert_eq!(
            named(
                &json!({"grants": [], "replaced_grants": [earlier]}),
                &snapshot
            ),
            [
                "No chats have Crew access.".to_owned(),
                "Earlier access no chat holds any more:".into(),
                format!("  Chat 20260924_3 {base} · Revoked · policy epoch 4"),
            ]
            .join("\n")
        );
        assert_eq!(
            named(
                &json!({"grants": [current.clone()], "replaced_grants": []}),
                &snapshot
            ),
            named(&json!({"grants": [current]}), &snapshot)
        );
        assert_eq!(
            plain(&json!({"grants": [], "replaced_grants": []})),
            "No chats have Crew access."
        );
    }

    #[test]
    fn transfers_show_names_progress_and_sizes() {
        let snapshot = alice_snapshot();
        let text = named(&transfers(), &snapshot);
        assert_eq!(
            text,
            [
                "counts.csv · upload to #methods · Uploading 40% (22 B of 55 B)",
                "plot.png · download from #general · Saved · 1.5 MB",
                "Add --show-ids for the IDs files status, resume, pause and forget take.",
            ]
            .join("\n")
        );
        assert_no_machine_ids(&text);
        let ids = with_ids(&transfers(), &snapshot);
        for id in [TRANSFER, BLOB, NODE, CONNECTION, "fixture-upload-001"] {
            assert!(ids.contains(id), "{id} missing from:\n{ids}");
        }
        assert_eq!(plain(&json!({"transfers":[]})), "No file transfers.");
    }

    /// CLIDOCS-F10: a paused transfer is a state with its reason in the desktop's words and
    /// this terminal's resume command, never an "Error:" with desktop wording. A failed one
    /// keeps its error.
    #[test]
    fn a_paused_transfer_says_why_and_how_to_resume() {
        let snapshot = alice_snapshot();
        let mut paused = transfers()["transfers"][0].clone();
        paused["state"] = json!("needs_file_selection");
        paused["error"] =
            json!("Transfer paused. Reselect the original local file or destination to resume.");
        let text = named(&paused, &snapshot);
        assert_eq!(
            text,
            [
                "counts.csv · upload to #methods · Paused · 55 B",
                "  You paused it.",
                &format!(
                    "  Resume with the original file: biorouter crew files resume {TRANSFER} FILE"
                ),
            ]
            .join("\n")
        );
        assert!(!text.contains("Error"), "{text}");
        assert!(!text.contains("Reselect"), "{text}");
        // In a list, the ID is there only with --show-ids, as every other ID is.
        let listed = named(&json!({"transfers": [paused.clone()]}), &snapshot);
        assert!(
            listed.contains("  Resume with the original file: biorouter crew files resume ID FILE"),
            "{listed}"
        );
        assert_no_machine_ids(&listed);
        // A download resumes with its destination; a reason Crew doesn't know is said as is.
        let mut download = transfers()["transfers"][1].clone();
        download["state"] = json!("needs_file_selection");
        download["error"] = json!("The workspace server ran out of disk space while saving this change, so it may not have been saved.");
        let text = named(&download, &snapshot);
        assert!(text.contains("\n  The workspace server ran out of disk space while saving this change, so it may not have been saved.\n"), "{text}");
        assert!(
            text.contains("  Resume with the same destination: biorouter crew files resume"),
            "{text}"
        );
        for (error, reason) in [
            ("Authenticate and reconnect the saved connection in Crew, then reselect the original local file or destination and resume.", "The connection dropped."),
            ("Unlock the Crew credential vault for this daemon session, then reselect the original local file or destination and resume.", "The credential vault is locked."),
            ("Two transfers are active; reselect and resume when one finishes", "Two other transfers were running."),
            ("Transfer stopped. Reselect the original local file or destination to resume. Inspect any unconfirmed publication before retrying.", "It stopped."),
        ] {
            assert_eq!(pause_reason(error).as_deref(), Some(reason));
        }
        assert_eq!(pause_reason("  "), None);
        let mut failed = transfers()["transfers"][0].clone();
        failed["state"] = json!("failed");
        failed["error"] = json!("You're not in that channel.");
        let text = named(&failed, &snapshot);
        assert!(
            text.ends_with("\n  Error: You're not in that channel."),
            "{text}"
        );
        assert!(!text.contains("Resume"), "{text}");
    }

    #[test]
    fn every_fixture_prints_no_machine_ids_by_default_and_ids_with_show_ids() {
        let snapshot = alice_snapshot();
        let fixtures = [
            (alice_snapshot(), WORKSPACE),
            (bob_snapshot(), DAVE),
            (history(), RUN),
            (json!({"connections":[connection()]}), CONNECTION),
            (connection(), NODE),
            (json!({"runs":[task()]}), RUN),
            (task(), RUN),
            (grants(), RUN),
            (privacy(), CONNECTION),
            (transfers(), TRANSFER),
            (snapshot["principals"].clone(), CAROL),
            (snapshot["teams"].clone(), TEAM),
            (snapshot["channels"].clone(), METHODS),
            (snapshot["invitations"].clone(), INVITATION),
            (snapshot["references"].clone(), REFERENCE),
            (history()["messages"][3].clone(), BLOB),
        ];
        for (value, id) in &fixtures {
            assert_no_machine_ids(&plain(value));
            assert_no_machine_ids(&named(value, &snapshot));
            let shown = with_ids(value, &snapshot);
            assert!(shown.contains(id), "{id} missing with show_ids:\n{shown}");
        }
    }

    #[test]
    fn display_names_are_quoted_isolated_and_escaped() {
        let hostile = json!([{"id":"p1","username":"mallory",
            "nickname":"Bob \"Lee\" (@bob)\\\u{202e}gnp.exe\u{2069}\n"}]);
        let text = plain(&hostile);
        // Every control in the name is printed as escape text, so what is left is ASCII: it
        // holds no right-to-left text to isolate, and no raw control survives.
        assert_eq!(
            text,
            "\"Bob \\\"Lee\\\" (@bob)\\\\\\u{202e}gnp.exe\\u{2069}\" (@mallory)"
        );
        assert!(!text.contains(['\u{202e}', '\u{2068}', '\u{2069}']));
        assert!(text.ends_with("\" (@mallory)"));
        // A name with right-to-left text keeps one isolate pair, opened and closed by the
        // formatter: the name's own U+2069 is escaped, so it cannot close the isolate early.
        let mixed =
            plain(&json!([{"id":"p3","username":"eve","nickname":"דנה\u{2069}\u{202e} (@bob)"}]));
        assert_eq!(
            mixed,
            "\"\u{2068}דנה\\u{2069}\\u{202e} (@bob)\u{2069}\" (@eve)"
        );
        assert_eq!(mixed.matches('\u{2068}').count(), 1);
        assert_eq!(mixed.matches('\u{2069}').count(), 1);
        let rtl = plain(&json!([{"id":"p2","username":"dana","nickname":"דנה"}]));
        assert_eq!(rtl, "\"\u{2068}דנה\u{2069}\" (@dana)");
        // F10: a plain Latin name is printed bare, so a copy of it matches "Alice Chen".
        let latin = plain(&json!([{"id":"p4","username":"alice","nickname":"Alice Chen"}]));
        assert_eq!(latin, "\"Alice Chen\" (@alice)");
        let team = plain(
            &json!([{"id":"t","name":"מעבדה","created_by":"p","members":[],"general_channel_id":"g"}]),
        );
        assert!(team.starts_with("\u{2068}מעבדה\u{2069} · 0 members"));
    }

    #[test]
    fn a_display_name_equal_to_the_username_prints_the_username_alone() {
        let equal = plain(&json!([{"id":"p","username":"bob","nickname":"BOB"}]));
        assert_eq!(equal, "@bob");
        let blank = plain(&json!([{"id":"p","username":"bob","nickname":"  "}]));
        assert_eq!(blank, "@bob");
        let mut directory = Directory::default();
        directory.absorb(&json!([{"id":"p","username":"bob","nickname":"bob"}]));
        // Q3-63: a decision names them the same way, without the username quoted as a name.
        assert_eq!(authority_label(&directory, "p", false), "@bob");
        let mut named = Directory::default();
        named.absorb(&json!([{"id":"f","username":"crew_frank","nickname":"Frank Okafor"}]));
        assert_eq!(
            authority_label(&named, "f", false),
            format!("{} (@crew_frank)", q("Frank Okafor"))
        );
        let mut unnamed = Directory::default();
        unnamed.absorb(&json!([{"id":"f","username":"crew_frank"}]));
        assert_eq!(authority_label(&unnamed, "f", false), "@crew_frank");
        assert_eq!(person_label(&directory, "p", false), "@bob");
        assert_eq!(person_label(&directory, "p", true), "@bob [ID p]");
        assert_eq!(person_label(&directory, "missing", false), "Unknown member");
        assert_eq!(
            authority_label(&directory, "missing", false),
            "Unknown member"
        );
    }

    #[test]
    fn fallbacks_name_nothing_they_cannot_see() {
        let channel = plain(
            &json!({"id":METHODS,"team_id":TEAM,"name":"","created_by":BOB,
            "owner_id":BOB,"members":[],"archived":true,"classification":"restricted"}),
        );
        assert_eq!(
            channel,
            "this channel · Restricted · Archived · 0 members · owner Unknown member"
        );
        let invite = plain(&json!({"id":INVITATION,"kind":"team","target_id":TEAM,
            "principal_id":BOB,"inviter_id":ALICE,"expires_at":NOW - 1}));
        assert_eq!(
            invite,
            "Unknown member invited Unknown member to this team · expired\nAdd --show-ids for the IDs invites accept and other invitation commands take."
        );
        let workspace = plain(
            &json!({"workspace":{"id":WORKSPACE,"host_uid":1,"mode":"public",
            "policy_epoch":1},"teams":[],"channels":[]}),
        );
        assert_eq!(
            workspace,
            "Workspace: this workspace\nPrivacy: Allows Public · no institution · policy epoch 1\nPeople: none\nTeams: none\nChannels: none"
        );
        let nameless = plain(&json!({"messages":[message("m", DAVE, NOW, "hi")],
            "people":{DAVE:{"active":false}}}));
        assert!(nameless.starts_with("Former member · 01:50"), "{nameless}");
    }

    /// R-8, AG-F17: offline, a row says the names could not be read and shows the ID, the one
    /// thing that tells two rows apart, instead of pretending with "this channel".
    #[test]
    fn rows_say_when_names_are_unavailable_and_show_the_id() {
        let offline = options(false, Directory::names_unavailable("okafor-lab", true));
        let grant_rows = render_text(&grants(), &offline);
        assert_eq!(
            grant_rows,
            format!("Chat 20260924_3 → a channel in okafor-lab (names unavailable while disconnected) [channel ID {METHODS}] (also reads a channel in okafor-lab (names unavailable while disconnected) [channel ID {GENERAL}]) · Active · policy epoch 4")
        );
        let transfer_rows = render_text(&transfers(), &offline);
        assert!(
            transfer_rows.starts_with(&format!("counts.csv · upload to a channel in okafor-lab (names unavailable while disconnected) [channel ID {METHODS}] · Uploading 40%")),
            "{transfer_rows}"
        );
        assert!(!transfer_rows.contains("this channel"), "{transfer_rows}");
        // For another reason, it says only that the names are unavailable.
        let unreadable = options(false, Directory::names_unavailable("lab", false));
        assert!(render_text(&grants(), &unreadable)
            .contains("a channel in lab (names unavailable) [channel ID"));
        // A value that simply does not name a channel reads as it did.
        assert!(plain(&task()).starts_with("Task posting to this channel"));
    }

    #[test]
    fn message_bodies_keep_their_lines_and_escape_terminal_controls() {
        let body = "first\n  indented\u{1b}[31m red\n\u{202e}flipped";
        let text = plain(&message("m", ALICE, NOW, body));
        assert_eq!(
            text,
            "Unknown member · 01:50 · Restricted  first\n      indented\\u{1b}[31m red\n    \\u{202e}flipped"
        );
        let empty = plain(&json!({"id":"m","actor_id":ALICE,"body":"","attachments":[BLOB, "b2"]}));
        assert_eq!(empty, "Unknown member\n    2 attachments");
    }

    #[test]
    fn json_output_is_untouched_by_text_options() {
        let rich =
            options(true, Directory::from_snapshot(&alice_snapshot())).with_view(View::Members);
        for value in [
            alice_snapshot(),
            history(),
            transfers(),
            json!({"error":"the model's \"x\""}),
        ] {
            assert_eq!(
                format_output(&value, OutputFormat::Json, &rich).unwrap(),
                serde_json::to_string_pretty(&value).unwrap()
            );
            assert_eq!(
                format_output(&value, OutputFormat::StreamJson, &rich).unwrap(),
                serde_json::to_string(&value).unwrap()
            );
        }
        assert_eq!(safe_text("the model's \"x\""), "the model's \"x\"");
    }

    #[test]
    fn results_read_as_sentences() {
        let created = json!({"team":{"id":TEAM,"name":"Foreign CLI QA","created_by":ALICE,
            "members":[ALICE],"general_channel_id":GENERAL},
            "channel":channel(GENERAL, "general", "restricted", ALICE, &[ALICE])});
        assert_eq!(
            plain(&created),
            "Created team Foreign CLI QA with #general."
        );
        let daemon = json!({"version":1,"profile_id":"b8209215-f3d5-4b84-bbe8-3c20c3a91807",
            "instance_id":"73a1bdf3-fe40-45e3-8f6c-091ad9640444","pid":60227,"user_action_installed":true});
        assert_eq!(
            plain(&daemon),
            "Biorouter daemon running (pid 60227) for this profile."
        );
        assert!(with_ids(&daemon, &json!({})).contains("Instance ID: 73a1bdf3"));
        let stopped = json!({"stopped":true,"instance_id":"73a1bdf3-fe40-45e3-8f6c-091ad9640444"});
        assert_eq!(plain(&stopped), "Biorouter daemon stopped.");
        let auth = json!({"authentication_id":RUN,"exit_code":0,"authenticated":true});
        assert_eq!(plain(&auth), "Authenticated. The connection is ready.");
        let joined = json!({"principal":principal(BOB, 10002, "crew_bob", "crew_bob"),
            "device_id":DEVICE,"workspace":{"id":WORKSPACE,"host_uid":10001,"institution_id":"ucsf",
            "mode":"private","policy_epoch":2}});
        assert_eq!(plain(&joined), "Signed in to the workspace as @crew_bob.");
        let cases = [
            (json!({"accepted":true}), "Invitation accepted."),
            (json!({"revoked":true}), "Revoked."),
            (
                json!({"run_id":RUN,"session_id":"20260924_1"}),
                "Crew access granted to chat 20260924_1.",
            ),
            (
                json!({"channel_id":GENERAL,"sequence":RUN}),
                "Marked this channel as read.",
            ),
            (
                json!({"cancelled":true,"status":"cancelled","remote_revocation_confirmed":true,
                "message":"Local cancellation requested and remote grant revoked."}),
                "Local cancellation requested and remote grant revoked.",
            ),
            (
                json!({"cancelled":false,"already_finished":true,"status":"completed"}),
                "The task had already finished: Done.",
            ),
            (
                json!({"backend":"file","initialized":true,"locked":false}),
                "Credential vault: Unlocked · file",
            ),
            // Normal on a desktop, where the OS keyring keeps the keys.
            (
                json!({"backend":"keyring","initialized":false,"locked":false,"available":true}),
                "Credential vault: Not set up · keyring",
            ),
            // SETUPHPC2-F-C: a headless node's keyring cannot keep a key.
            (
                json!({"backend":"keyring","initialized":false,"locked":false,"available":false}),
                "Credential vault: Not set up · no keyring on this computer; run biorouter crew credentials init",
            ),
            (
                json!({"detached":true,"transfer_id":TRANSFER}),
                "Stopped watching. The transfer continues.",
            ),
            (
                json!({"transfer_id":TRANSFER,"state":"completed"}),
                "Transfer Ready.",
            ),
            (json!({}), "Done."),
        ];
        for (value, expected) in cases {
            assert_eq!(plain(&value), expected, "for {value}");
        }
    }

    #[test]
    fn deliverable_keys_and_tokens_are_printed_but_their_ids_are_not() {
        let prepared = json!({"preparation_id":CONNECTION,"public_key":KEY,"device_id":DEVICE});
        let text = plain(&prepared);
        // SF-F3: one command serves a host and a joiner, and nothing in it says which, so the
        // key and the preparation ID are described for both, each with its own next command.
        assert_eq!(
            text,
            format!(
                "Device key prepared. Its public key:\n  {KEY}\n\
                 To host a workspace, pass this key to biorouter-crew start --bootstrap-key on the server. \
                 To join one, send it to the workspace's host.\n\
                 Add --show-ids for the preparation ID, which goes to connections join-invitation \
                 --preparation-id when you host, or into the descriptor for connections save when you join."
            )
        );
        assert!(!text.contains(CONNECTION) && !text.contains(DEVICE));
        let shown = with_ids(&prepared, &json!({}));
        assert_eq!(
            shown,
            format!(
                "Device key prepared. Its public key:\n  {KEY}\n\
                 To host a workspace, pass this key to biorouter-crew start --bootstrap-key on the server. \
                 To join one, send it to the workspace's host.\n  \
                 Preparation ID: {CONNECTION}\n  Computer ID: {DEVICE}\n\
                 The preparation ID goes to connections join-invitation --preparation-id when you \
                 host, or into the descriptor for connections save when you join."
            )
        );
        let token = "4f".repeat(32);
        let invite =
            json!({"invitation":token,"expires_at":NOW + 3600,"uid":10002,"device_id":DEVICE});
        let text = plain(&invite);
        assert_eq!(
            text,
            format!("Send this enrollment token to your colleague privately. It expires in 1 hour.\n  {token}")
        );
        assert!(!text.contains(DEVICE) && !text.contains("10002"));
        // A person's machine is a computer wherever --show-ids names it: the token, a saved
        // connection and a prepared key alike.
        let token_ids = with_ids(&invite, &json!({}));
        assert!(
            token_ids.contains(&format!("\n  Computer ID: {DEVICE}")),
            "{token_ids}"
        );
        let connection = json!({
            "id": CONNECTION, "name": "lab", "ssh_target": "bob@hpc", "workspace_id": "w",
            "status": "connected", "device_id": DEVICE
        });
        let connection_ids = with_ids(&connection, &json!({}));
        assert!(
            connection_ids.contains(&format!("Computer ID: {DEVICE}")),
            "{connection_ids}"
        );
        for shown in [&token_ids, &connection_ids, &shown] {
            assert!(!shown.contains("Device ID"), "{shown}");
        }
    }

    #[test]
    fn unknown_values_list_their_fields_without_ids() {
        // Keys in alphabetical order, so the expectation holds whether or not serde_json's
        // `preserve_order` is unified into this build.
        let value = json!({"channel_ids":[GENERAL, METHODS],"count":3,"hash":NODE,
            "message":"Queued.","nested":{"label":"x","owner_id":ALICE},"note":"ok",
            "queue_id":RUN,"ready":true});
        let text = plain(&value);
        assert_eq!(
            text,
            "Queued.\nCount: 3\nNested:\n  Label: x\nNote: ok\nReady: yes"
        );
        let shown = with_ids(&value, &json!({}));
        for id in [RUN, NODE, GENERAL, ALICE] {
            assert!(shown.contains(id), "{id} missing from:\n{shown}");
        }
    }

    #[test]
    fn a_request_id_appears_only_in_the_retry_hint() {
        assert_eq!(
            retry_hint("cli-retry-01"),
            "Retry safely with --request-id cli-retry-01"
        );
        assert_eq!(
            retry_hint("x\u{1b}y"),
            "Retry safely with --request-id x\\u{1b}y"
        );
        assert!(!plain(&transfers()).contains("fixture-upload-001"));
    }

    #[test]
    fn times_and_sizes_use_the_readers_calendar() {
        let ctx = Ctx::new(
            Directory::default(),
            false,
            Clock::Fixed {
                now: NOW,
                offset_seconds: -7 * 3600,
            },
        );
        assert_eq!(ctx.when(NOW), "18:50");
        assert_eq!(ctx.when(NOW - 86_400), "Sep 22 18:50");
        assert_eq!(ctx.when(NOW - 400 * 86_400), "Aug 19, 2025 18:50");
        assert_eq!(within(30), "in 1 minute");
        assert_eq!(within(5400), "in 2 hours");
        assert_eq!(within(3 * 86_400), "in 3 days");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(56_320), "55 KB");
        assert_eq!(
            transfer_state_word("needs_file_selection", "upload", 0, None),
            "Paused"
        );
        assert_eq!(
            run_status_word("waiting_for_approval"),
            "Waiting for your approval"
        );
        assert_eq!(run_status_word("new_state"), "New state");
    }

    /// Every fixture is the broker's literal text (`crates/biorouter-crew/src/broker.rs`,
    /// `broker/join.rs`, `DeviceCodeError`), paired with what the CLI prints for it. The words
    /// match the desktop's table in `ui/desktop/src/components/crew/dialogs/refusals.test.ts`,
    /// except where the desktop still shows a code and the terminal no longer does (Q2-76): a
    /// technical text or a bare code becomes a sentence here, and the code stays in JSON.
    const BROKER_REFUSALS: &[(&str, &str, &str)] = &[
        (
            "name_taken",
            "name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.",
            "A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.",
        ),
        (
            "name_taken",
            "name_taken",
            "That name is already taken.\nChoose another name.",
        ),
        (
            "rate_limited",
            "rate_limited: Too many name attempts. Try again later.",
            "Too many name attempts. Try again later.",
        ),
        (
            "rate_limited",
            "rate_limited: too many live challenges",
            TOO_MANY_ATTEMPTS,
        ),
        (
            "target_mismatch",
            "target_mismatch: The person you chose no longer has that username. Refresh and choose again.",
            "The person you chose no longer has that username. Refresh and choose again.",
        ),
        (
            "not_invited",
            "not_invited: @eve has no pending invitation. Invite them first.",
            "@eve has no pending invitation. Invite them first.",
        ),
        (
            "identity_unavailable",
            "identity_unavailable: @Bob can't be matched to one account on this server.",
            "@Bob can't be matched to one account on this server.",
        ),
        (
            "unknown_account",
            "unknown_account: There is no account @zed on this server. Check the spelling.",
            "There is no account @zed on this server. Check the spelling.",
        ),
        (
            "quota_exceeded",
            "quota_exceeded: 100 people are already waiting to join. Cancel an invitation or wait for one to expire.",
            "100 people are already waiting to join. Cancel an invitation or wait for one to expire.",
        ),
        // `open_inner` only: the broker says this when it starts, never in answer to a request.
        (
            "quota_exceeded",
            "quota_exceeded: journal exceeds supported replay size of 1 GiB",
            STORAGE_FULL,
        ),
        (
            "quota_exceeded",
            "quota_exceeded: retained audit journal exceeds 1 GiB; preserve the complete store and use a new workspace; in-place audit deletion is not supported",
            STORAGE_FULL,
        ),
        (
            "quota_exceeded",
            "quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported",
            STORAGE_FULL,
        ),
        // An older broker's operation quota; the current one no longer writes it.
        (
            "quota_exceeded",
            "quota_exceeded: workspace operation quota requires maintenance",
            STORAGE_FULL,
        ),
        (
            "quota_exceeded",
            "quota_exceeded: workspace logical state is full; reads remain available and the host can still remove members and change policy, but further changes require a new workspace; in-place pruning of history is not supported",
            FULL_BUT_HOST_CAN_ADMINISTER,
        ),
        (
            "quota_exceeded",
            "quota_exceeded: retained audit journal is nearly full; reads remain available and the host can still remove members and change policy; preserve the complete store and use a new workspace",
            FULL_BUT_HOST_CAN_ADMINISTER,
        ),
        (
            "quota_exceeded",
            "quota_exceeded: You have used your share of this workspace's storage. Reading still works; ask the workspace host about starting a new workspace.",
            "You have used your share of this workspace's storage. Reading still works; ask the workspace host about starting a new workspace.",
        ),
        (
            "identity_conflict",
            "identity_conflict: another active member is @j.doe; remove the old @j.doe first",
            "Another active member is already @j.doe. Remove the old @j.doe first.",
        ),
        (
            "identity_conflict",
            "identity_conflict: Another account on this server is already invited as @bob. Cancel that invitation first.",
            "Another account on this server is already invited as @bob. Cancel that invitation first.",
        ),
        (
            "device_conflict",
            "device_conflict: this device key is already enrolled in this workspace; use a new device key",
            DEVICE_CONFLICT,
        ),
        (
            "identity_mismatch",
            "identity_mismatch: enrollment principal changed; request a new invitation explicitly identifying the existing principal or offboard the old account",
            IDENTITY_MISMATCH,
        ),
        (
            "identity_mismatch",
            "identity_mismatch: UID account name changed; offboard the old principal before enrollment",
            IDENTITY_MISMATCH,
        ),
        (
            "identity_mismatch",
            "identity_mismatch: This account joined as @bob and is now @robert on the server. Remove @bob first.",
            "This account joined as @bob and is now @robert on the server. Remove @bob first.",
        ),
        (
            "already_approved",
            "already_approved: You already entered a code for @eve. If it didn't match their computer, enter the code they sent and choose Replace.",
            "You already entered a code for @eve.\nIf it didn't match their computer, run: biorouter crew enroll approve @eve <code> --replace",
        ),
        (
            "identity_ambiguous",
            "identity_ambiguous: This server spells the account @alice. Invite @alice.",
            "This server spells the account @alice. Invite @alice.\nRun: biorouter crew enroll invite @alice",
        ),
        (
            "identity_ambiguous",
            "identity_ambiguous: @Al is an alias on this server. Invite @alice.",
            "@Al is an alias on this server. Invite @alice.\nRun: biorouter crew enroll invite @alice",
        ),
        (
            "already_member",
            "already_member: @bob is already a member. Choose Add device to add another computer for them.",
            "@bob is already a member.\nTo add another computer for them, run: biorouter crew enroll invite @bob --add-device",
        ),
        // The broker's sentence names the desktop's switch; here it is the flag left off.
        (
            "invalid_params",
            "invalid_params: @carol is not a member yet. Turn off Add another computer, then invite them.",
            "@carol is not a member yet. Invite them without --add-device.\nRun: biorouter crew enroll invite @carol",
        ),
        (
            "code_mismatch",
            "code_mismatch: The host hasn't let this device in. Send the host the code shown on your screen.",
            CODE_MISMATCH,
        ),
        (
            "code_mismatch",
            "code_mismatch: The host hasn't let this computer in. Send the host the code shown on your screen.",
            CODE_MISMATCH,
        ),
        (
            "forbidden",
            "forbidden: only a person can invite or admit people",
            "Only a person can invite or admit people.",
        ),
        // Q2-76: live, each of these reached the terminal with its code.
        (
            "forbidden",
            "forbidden: Only the team's owner or the workspace host can add people to it.",
            "Only the team's owner or the workspace host can add people to it.",
        ),
        (
            "forbidden",
            "forbidden: channel unavailable",
            "You're not in that channel.",
        ),
        // DW-11, M20, FILES-F9: the role, archive and provenance texts in words. With nothing
        // known about the request, each names what it can and stays true.
        (
            "forbidden",
            "forbidden: workspace host device required",
            "Only the workspace host can do this.",
        ),
        (
            "forbidden",
            "forbidden: current owner required",
            "Only the channel's owner can do this.",
        ),
        (
            "forbidden",
            "forbidden: team creator required",
            "Only the team's creator can do this.",
        ),
        (
            "forbidden",
            "forbidden: team owner required",
            "Only the team's owner can do this.",
        ),
        (
            "channel_archived",
            "channel_archived: channel is read-only",
            "That channel is archived, so it's read-only.",
        ),
        (
            "forbidden",
            "forbidden: attachment provenance cannot be dropped",
            "That file was shared in another channel. Share it there, or upload it again here.",
        ),
        (
            "forbidden",
            "forbidden: reference provenance cannot be dropped",
            "That remote reference was shared in another channel. Share it there, or add it again here.",
        ),
        // R-2: never "restart and recover" to someone who cannot restart. An older broker's
        // technical text is said plainly; a current broker's own sentence is kept.
        (
            "storage_failed",
            "storage_failed: restart and recover before further mutations",
            "The workspace server can't save changes right now. Ask the host to restart Crew.",
        ),
        (
            "storage_full",
            "storage_full: The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.",
            "The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.",
        ),
        (
            "storage_failed",
            "storage_failed: The workspace server could not confirm this change was saved to disk, so it may not have been saved. Reading still works. Ask the host to check the server's storage and restart Crew.",
            "The workspace server could not confirm this change was saved to disk, so it may not have been saved. Reading still works. Ask the host to check the server's storage and restart Crew.",
        ),
        (
            "unauthorized",
            "unauthorized: unknown device",
            "This computer isn't a member of this workspace.",
        ),
        (
            "forbidden",
            "forbidden: That person isn't a member of this workspace. Refresh and choose again.",
            "That person isn't a member of this workspace. Refresh and choose again.",
        ),
        ("stale_cursor", "stale_cursor", "A message in this view is no longer available to you."),
        ("brand_new_code", "brand_new_code", "The workspace refused this request."),
        // OW-4, OW-5, OW-8 (design contract texts; the broker's own literals are its own
        // engineer's): what to do next is a command on this computer.
        (
            "workspace_deleted",
            "workspace_deleted: workspace was deleted by its host",
            WORKSPACE_DELETED,
        ),
        (
            "revision_conflict",
            "revision_conflict: context revision changed",
            REVISION_CONFLICT,
        ),
        (
            "name_changed",
            "name_changed: expected name does not match",
            NAME_CHANGED,
        ),
        (
            "name_changed",
            "name_changed: #methods is now called #methods-old. Check it before deleting it.",
            "#methods is now called #methods-old. Check it before deleting it.",
        ),
        (
            "invalid_params",
            "invalid_params: #general belongs to its team. Delete the team instead.",
            "#general belongs to its team. Delete the team instead.\nRun: biorouter crew teams delete TEAM",
        ),
    ];

    #[test]
    fn each_broker_code_prints_the_desktops_sentence_and_its_flag() {
        for (code, broker, shown) in BROKER_REFUSALS {
            assert_eq!(broker_refusal_text(code, broker), *shown, "{broker}");
        }
    }

    /// A reworded text that the broker never writes makes a check that can never fire, and a
    /// fixture row for it passes all the same. The storage-full rows are therefore read back
    /// against the broker's source, where each must appear exactly as written.
    ///
    /// The one exception is [`LEGACY_OPERATION_QUOTA`], which only an older broker writes and
    /// which must therefore be absent from the current one.
    #[test]
    fn each_storage_full_fixture_is_the_brokers_literal_text() {
        const BROKER: &str = include_str!("../../../../biorouter-crew/src/broker.rs");
        let legacy = format!("quota_exceeded: {LEGACY_OPERATION_QUOTA}");
        let full: Vec<&str> = BROKER_REFUSALS
            .iter()
            .filter(|(_, _, shown)| {
                *shown == STORAGE_FULL || *shown == FULL_BUT_HOST_CAN_ADMINISTER
            })
            .map(|(_, broker, _)| *broker)
            .filter(|broker| *broker != legacy)
            .collect();
        // The journal limit at startup and in `commit`, the state-size limit, and the two
        // limits that leave the host room to administer.
        assert_eq!(full.len(), 5, "{full:#?}");
        for text in full {
            assert!(
                BROKER.contains(&format!("\"{text}\"")),
                "{text} is not a literal in broker.rs"
            );
        }
        assert!(
            !BROKER.contains(&format!("\"{legacy}\"")),
            "the broker writes {legacy} again: it is no longer legacy"
        );
        assert_eq!(broker_refusal_text("quota_exceeded", &legacy), STORAGE_FULL);
    }

    /// The broker's storage sentences the CLI keeps, and the words it reads an unknown outcome
    /// from, are the broker's own: if they drift, this says so.
    #[test]
    fn each_storage_sentence_fixture_is_the_brokers_literal_text() {
        const BROKER: &str = include_str!("../../../../biorouter-crew/src/broker.rs");
        let current: Vec<&str> = BROKER_REFUSALS
            .iter()
            .filter(|(code, broker, _)| {
                matches!(*code, "storage_full" | "storage_failed") && !broker.contains("recover")
            })
            .map(|(_, broker, _)| *broker)
            .collect();
        assert_eq!(current.len(), 2, "{current:#?}");
        for text in current {
            assert!(
                BROKER.contains(&format!("\"{text}\"")),
                "{text} is not a literal in broker.rs"
            );
        }
        assert!(
            BROKER.contains("so it may not have been saved."),
            "the broker no longer says an outcome is unknown this way"
        );
    }

    /// The broker's "not a member yet" refusal, which the CLI words for this terminal's flag, is
    /// the broker's own text: if it drifts, the rewording can never fire, and this says so.
    #[test]
    fn the_not_a_member_yet_fixture_is_the_brokers_text() {
        const JOIN: &str = include_str!("../../../../biorouter-crew/src/broker/join.rs");
        assert!(
            JOIN.contains("\"invalid_params: @{} is not a member yet."),
            "join.rs no longer refuses an added computer for a non-member this way"
        );
    }

    /// A reworded text the broker never writes is a check that can never fire, so the role,
    /// archive and provenance fixtures are read back against the broker's source.
    #[test]
    fn each_role_archive_and_provenance_fixture_is_the_brokers_literal_text() {
        const BROKER: &str = include_str!("../../../../biorouter-crew/src/broker.rs");
        for text in [
            "forbidden: workspace host device required",
            "forbidden: current owner required",
            "forbidden: team creator required",
            "forbidden: team owner required",
            "channel_archived: channel is read-only",
            "forbidden: attachment provenance cannot be dropped",
            "forbidden: reference provenance cannot be dropped",
        ] {
            assert!(
                BROKER.contains(&format!("\"{text}\"")),
                "{text} is not a literal in broker.rs"
            );
            assert!(
                BROKER_REFUSALS.iter().any(|(_, broker, _)| *broker == text),
                "{text} has no fixture row"
            );
        }
    }

    /// DW-11, M20, R-2, FILES-F9: what the command knows about the refusal is named in it.
    #[test]
    fn a_refusal_names_the_channel_host_and_server_the_command_knows() {
        let place = RefusalPlace {
            channel: Some("#methods".into()),
            shared_in: Some("#raw-data".into()),
            host: Some(false),
            host_label: Some("\"Alice Chen\" (@alice)".into()),
            server: Some("hpc.ucsf.edu".into()),
        };
        let said = |code: &str, text: &str, place: &RefusalPlace| {
            broker_refusal_text_in(code, text, place)
        };
        assert_eq!(
            said("forbidden", "forbidden: current owner required", &place),
            "Only #methods's owner can do this."
        );
        assert_eq!(
            said(
                "channel_archived",
                "channel_archived: channel is read-only",
                &place
            ),
            "#methods is archived, so it's read-only."
        );
        assert_eq!(
            said(
                "forbidden",
                "forbidden: attachment provenance cannot be dropped",
                &place
            ),
            "That file was shared in #raw-data. Share it there, or upload it again here."
        );
        assert_eq!(
            said("storage_failed", "storage_failed: restart and recover before further mutations", &place),
            "The workspace server can't save changes right now. Ask \"Alice Chen\" (@alice) to restart Crew."
        );
        const FULL: &str = "storage_full: The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.";
        assert_eq!(
            said("storage_full", FULL, &place),
            "The workspace server is out of disk space, so this change was not saved. Reading still works. Ask \"Alice Chen\" (@alice) to free space on the server and restart Crew."
        );
        let host = RefusalPlace {
            host: Some(true),
            ..place.clone()
        };
        let text = said("storage_full", FULL, &host);
        assert_eq!(
            text,
            "The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.\nYou host this workspace. Free space on hpc.ucsf.edu, then restart Crew there: biorouter-crew stop, then biorouter-crew start, each with this workspace's --state-dir."
        );
        let old = said(
            "storage_failed",
            "storage_failed: restart and recover before further mutations",
            &host,
        );
        assert_eq!(
            old,
            "The workspace server can't save changes right now.\nYou host this workspace. Free space on hpc.ucsf.edu if it is full, then restart Crew there: biorouter-crew stop, then biorouter-crew start, each with this workspace's --state-dir."
        );
        assert!(!old.to_ascii_lowercase().contains("recover"), "{old}");
        assert!(storage_outcome_unknown(
            "storage_full",
            "storage_full: The workspace server ran out of disk space while saving this change, so it may not have been saved."
        ));
        assert!(!storage_outcome_unknown("storage_full", FULL));
        assert!(!storage_outcome_unknown(
            "forbidden",
            "may not have been saved"
        ));
        // Only the texts it rewords take the place: another refusal is unchanged.
        assert_eq!(
            said("forbidden", "forbidden: channel unavailable", &place),
            "You're not in that channel."
        );
    }

    #[test]
    fn the_canonical_spelling_never_takes_the_sentences_full_stop() {
        for (sentence, canonical) in [
            (
                "This server spells the account @alice. Invite @alice.",
                "alice",
            ),
            ("@Al is an alias on this server. Invite @alice.", "alice"),
            ("@jd is an alias on this server. Invite @j.doe.", "j.doe"),
            (
                "@jd is an alias on this server. invite   @j.doe_2-x.",
                "j.doe_2-x",
            ),
            ("Did you mean @bob?", "bob"),
        ] {
            assert_eq!(canonical_hint(sentence), Some(canonical), "{sentence}");
        }
        assert_eq!(canonical_hint("Invite them first."), None);
        assert_eq!(canonical_hint("Invite@alice."), None);
    }

    /// The CLI's sentences are the desktop's, byte for byte. Reading the desktop's copy deck is
    /// the only place both halves are visible at once.
    #[test]
    fn broker_refusal_sentences_match_the_desktops_copy_deck() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/crew/dialogs/copy.ts");
        let copy = std::fs::read_to_string(&path).expect("the desktop copy deck");
        for sentence in [
            DEVICE_CONFLICT,
            IDENTITY_MISMATCH,
            STORAGE_FULL,
            STORAGE_FULL_FOR_HOST,
            FULL_BUT_HOST_CAN_ADMINISTER,
            FULL_BUT_HOST_CAN_ADMINISTER_FOR_HOST,
            TOO_MANY_ATTEMPTS,
            IDENTITY_CONFLICT_UNNAMED,
        ] {
            assert!(
                copy.contains(&format!("'{sentence}'")),
                "{sentence} is not in {}",
                path.display()
            );
        }
        // A sentence with an ASCII apostrophe sits in double quotes in the deck.
        assert!(
            copy.contains(&format!("codeMismatch:\n    \"{CODE_MISMATCH}\",")),
            "{CODE_MISMATCH} is not refusalCopy.codeMismatch in {}",
            path.display()
        );
        assert_eq!(
            broker_refusal_text(
                "code_mismatch",
                "code_mismatch: The host hasn't let this device in. Send the host the code shown on your screen."
            ),
            CODE_MISMATCH
        );
        // The host-only checks read as the desktop's dialogs word them (`refusals.ts`).
        let refusals = std::fs::read_to_string(path.with_file_name("refusals.ts"))
            .expect("the desktop's refusal wording");
        let host_only =
            broker_refusal_text("forbidden", "forbidden: workspace host device required");
        assert!(
            refusals.contains(&format!("HOST_ONLY_REFUSAL = '{host_only}';")),
            "{host_only}"
        );
        assert!(copy.contains(
            "`Another active member is already @${username}. Remove the old @${username} first.`"
        ));
        assert_eq!(
            identity_conflict(Some("bob")),
            "Another active member is already @bob. Remove the old @bob first."
        );
        // `already_approved` is deliberately the terminal's own sentence (Q2-75): the desktop
        // words it for its dialog's Replace button, which a terminal does not have.
        assert_eq!(
            already_approved("eve"),
            "You already entered a code for @eve."
        );
    }

    /// Q2-76: no broker code reaches text output, whatever the code, and whether or not the
    /// broker wrote a sentence.
    #[test]
    fn no_broker_code_reaches_text_output() {
        for (code, broker, _) in BROKER_REFUSALS {
            let shown = broker_refusal_text(code, broker);
            let first = shown.lines().next().unwrap();
            assert!(!first.starts_with(&format!("{code}:")), "{shown}");
            assert!(!first.contains(&format!("{code}:")), "{shown}");
            assert!(reads_as_sentence(first), "{shown}");
        }
    }

    #[test]
    fn a_technical_text_becomes_a_sentence() {
        assert_eq!(
            plain_refusal("invalid_params", "missing or invalid expected_username"),
            "Missing or invalid expected_username."
        );
        assert_eq!(
            plain_refusal("unauthorized", "Unknown device."),
            "This computer isn't a member of this workspace."
        );
        assert_eq!(
            plain_refusal("forbidden", ""),
            "The workspace didn't allow this."
        );
    }

    #[test]
    fn the_institution_refusal_is_the_desktops_sentence() {
        let details = json!({
            "model": "gpt-5.5-2026-04-24",
            "approved_for": ["ucsf"],
            "workspace": "foreign-lab",
            "workspace_institution": "stanford",
        });
        // SF-F4: institutions are named as the desktop names them, by the registry's name for
        // an ID it publishes (ucsf is UCSF) and by the ID otherwise, and the advice names the
        // workspace's institution rather than "it".
        assert_eq!(
            institution_refusal_text("gpt-5.5-2026-04-24", Some(&details)),
            "gpt-5.5-2026-04-24 is approved for UCSF. foreign-lab uses stanford. Choose a model approved for stanford, or a local model."
        );
        let unstated = json!({"model": "private-model", "approved_for": null, "workspace": "lab", "workspace_institution": "ucsf"});
        assert_eq!(
            institution_refusal_text("private-model", Some(&unstated)),
            "private-model doesn't say which institution approved it. lab uses UCSF. Choose a model approved for UCSF, or a local model."
        );
        assert_eq!(
            institution_label("stanford-synthetic"),
            "stanford-synthetic"
        );
        assert_eq!(
            institution_label("UCSF"),
            "UCSF",
            "not an ID: said as it came"
        );
        assert_eq!(institution_label("ucsf-west"), "ucsf-west");
        // The desktop's own sentences (`agentCopy` in pane/copy.ts), read where both are visible.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/crew/pane/copy.ts");
        let copy = std::fs::read_to_string(&path).expect("the desktop copy deck");
        for template in [
            "`${model} is approved for ${affiliation}. ${workspace} uses ${institution}. Choose a model approved for ${institution}, or a local model.`",
            "`${model} doesn\u{2019}t say which institution approved it. ${workspace} uses ${institution}. Choose a model approved for ${institution}, or a local model.`",
        ] {
            assert!(copy.contains(template), "{template} is not in {}", path.display());
        }
        // An older daemon sends no details: the model the person asked for, and nothing made up.
        assert_eq!(
            institution_refusal_text("gpt-5.5", None),
            "gpt-5.5 isn't approved for this workspace's institution. Choose a model approved for it, or a local model."
        );
        for text in [
            institution_refusal_text("m", Some(&details)),
            institution_refusal_text("m", None),
        ] {
            assert!(!text.contains("resolved affiliation"), "{text}");
        }
    }

    /// OW-5, OW-9: a connection whose workspace was deleted, and a server where Crew is
    /// missing, each say the command that acts on it.
    #[test]
    fn a_deleted_workspace_and_a_missing_crew_name_their_command() {
        assert_eq!(
            connect_failure_text("crew_workspace_deleted"),
            Some(WORKSPACE_DELETED)
        );
        assert!(connect_failure_text("crew_bridge_missing")
            .expect("missing bridge")
            .contains("Run biorouter crew setup"));
        let mut deleted = connection();
        deleted["workspace_deleted"] = json!(true);
        let text = plain(&deleted);
        assert!(text.contains(WORKSPACE_DELETED), "{text}");
        assert!(plain(&json!({"connections": [deleted.clone()]})).contains(WORKSPACE_DELETED));
        // A last error that already says so is not said twice.
        deleted["last_error"] = json!("workspace_deleted: workspace was deleted");
        deleted["last_error_code"] = json!("crew_workspace_deleted");
        assert_eq!(plain(&deleted).matches("The host deleted").count(), 1);

        let mut full = connection();
        full["remote_access"] = json!("full");
        assert!(
            plain(&full).contains("Remote folder: /home/crew_bob/crew-work · agents work as you on the server, starting here"),
            "{}",
            plain(&full)
        );
    }

    /// OW-11: a task that started without naming its model says which one runs, and why.
    #[test]
    fn a_task_says_the_model_it_started_on() {
        let mut run = task();
        run["task_model"] =
            json!({"provider": "versa_azure", "model": "gpt-5.5", "source": "crew_tasks"});
        assert!(
            plain(&run).contains("\n  Model: gpt-5.5 (versa_azure), your model for Crew tasks\n"),
            "{}",
            plain(&run)
        );
        run["task_model"]["source"] = json!("flags");
        assert!(plain(&run).contains("\n  Model: gpt-5.5 (versa_azure)\n"));
        assert!(!plain(&task()).contains("Model:"));
    }

    fn stage(name: &str, state: &str) -> Value {
        json!({"name": name, "state": state})
    }

    /// OW-9: each stage is said once, when first seen running or done, a quick one included; a
    /// skipped stage says nothing; a sign-in the server asks for is said.
    #[test]
    fn setup_progress_says_each_stage_once() {
        let first = json!({"state": "running", "stages": [stage("check", "running"), stage("install", "pending")]});
        assert_eq!(
            setup_progress_lines(&Value::Null, &first, "bob@hpc", &SetupNamed::default()),
            ["Checking the server…"]
        );
        let later = json!({"state": "running", "stages": [
            stage("check", "done"), stage("work_folder", "done"),
            stage("start", "skipped"), stage("install", "running")
        ]});
        assert_eq!(
            setup_progress_lines(&first, &later, "bob@hpc", &SetupNamed::default()),
            [
                "Making the work folder…",
                "Installing Crew in your account…"
            ]
        );
        assert!(setup_progress_lines(&later, &later, "bob@hpc", &SetupNamed::default()).is_empty());
        let sign_in = json!({"state": "needs_sign_in", "server": {"login": "bob@leo"},
            "stages": [stage("check", "running"), stage("sign_in", "running")]});
        assert_eq!(
            setup_progress_lines(&first, &sign_in, "ignored", &SetupNamed::default()),
            [
                "Signing in…",
                "bob@leo asks for your password or a verification code."
            ]
        );
        let done = json!({"state": "finished", "stages": [stage("check", "done"), stage("brand_new", "done"), stage("done", "done")]});
        assert_eq!(
            setup_progress_lines(&first, &done, "bob@hpc", &SetupNamed::default()),
            ["Brand new…", "Ready"]
        );
    }

    /// Live re-test: `setup --start` and `--restart-workspace` list the server first, and the
    /// setup that follows said "Checking the server…" and "Looking for workspaces…" again, with
    /// a "Ready" in between and never the workspace's name. Each stage is said once in the
    /// command, the listing leaves its "Ready" to the setup, and the restart stage names what
    /// it starts or restarts.
    #[test]
    fn a_setup_after_its_listing_says_each_stage_once_and_names_what_it_starts() {
        let listing = json!({"intent": "check", "state": "finished", "stages": [
            stage("check", "done"), stage("sign_in", "skipped"),
            stage("workspaces", "done"), stage("done", "done")
        ]});
        let none = SetupNamed::default();
        assert_eq!(
            before_the_setup(setup_progress_lines(&Value::Null, &listing, "w@leo", &none)),
            ["Checking the server…", "Looking for workspaces…"]
        );
        let install = json!({"intent": "install", "state": "finished",
        "stages": [
            stage("check", "done"), stage("install", "skipped"), stage("workspaces", "done"),
            stage("restart", "done"), stage("done", "done")
        ],
        "workspaces": [
            {"workspace_id": "s", "name": "acct-lab", "role": "host"},
            {"workspace_id": "r", "name": "old-lab", "role": "host"},
            {"workspace_id": "x", "name": "chen-lab", "role": "member"}
        ]});
        let said = with_said_stages(&Value::Null, &listing);
        let progress = |request: Value| {
            setup_progress_lines(&said, &install, "w@leo", &named_in_setup(&request))
        };
        assert_eq!(
            progress(json!({"start_workspaces": ["s"]})),
            ["Starting acct-lab…", "Ready"]
        );
        assert_eq!(
            progress(json!({"restart_workspace_ids": ["r"]})),
            ["Restarting old-lab…", "Ready"]
        );
        assert_eq!(
            progress(json!({"start_workspaces": ["s"], "restart_workspace_ids": ["r", "x"]})),
            [
                "Restarting old-lab and chen-lab, then starting acct-lab…",
                "Ready"
            ]
        );
        // Nothing named, or an ID the job's listing doesn't name: the stage's own words.
        assert_eq!(
            progress(json!({})),
            ["Updating running workspaces…", "Ready"]
        );
        assert_eq!(
            progress(json!({"start_workspaces": ["gone"]})),
            ["Updating running workspaces…", "Ready"]
        );
        // A job with no listing before it says its stages as it always did.
        assert_eq!(
            setup_progress_lines(&Value::Null, &install, "w@leo", &none)[..2],
            ["Checking the server…", "Looking for workspaces…"]
        );
        // The listing's stages join what this job already said, never replace it.
        let running = json!({"state": "running", "stages": [stage("restart", "running")]});
        assert_eq!(
            setup_progress_lines(
                &with_said_stages(&running, &listing),
                &install,
                "w@leo",
                &none
            ),
            ["Ready"]
        );
        assert_eq!(with_said_stages(&running, &Value::Null), running);

        // A check that could not list the workspaces says why in its summary, with no bare
        // "Ready" before it; an install that could not list them still says Ready.
        let unlisted = json!({"intent": "check", "state": "finished",
            "workspaces_note": "Crew isn't installed in your account on leo, so Biorouter can't look for workspaces there.",
            "stages": [stage("check", "done"), stage("workspaces", "skipped"), stage("done", "done")]});
        assert_eq!(
            setup_progress_lines(&Value::Null, &unlisted, "w@leo", &none),
            ["Checking the server…"]
        );
        let mut installed = unlisted.clone();
        installed["intent"] = json!("install");
        assert_eq!(
            setup_progress_lines(&Value::Null, &installed, "w@leo", &none),
            ["Checking the server…", "Ready"]
        );

        // Whether the account hosts a workspace, as the listing says: one place for its ID.
        assert_eq!(listed_role(&install, Some("s")).as_deref(), Some("host"));
        assert_eq!(listed_role(&install, Some("x")).as_deref(), Some("member"));
        assert_eq!(listed_role(&install, Some("unknown")), None);
        assert_eq!(listed_role(&Value::Null, Some("s")), None);
        assert_eq!(listed_role(&install, None), None);
        let mut twice = install.clone();
        twice["workspaces"]
            .as_array_mut()
            .expect("workspaces")
            .push(json!({"workspace_id": "s", "name": "acct-lab", "role": "member"}));
        assert_eq!(listed_role(&twice, Some("s")), None);
    }

    /// Live re-test: an account removed and let in again twice left two earlier entries, which
    /// teams and channels still list. The workspace's two people were "4 members", and former
    /// members listed "@gbellucci · a member again" twice. Counts are of members now, and former
    /// members are one line per account.
    #[test]
    fn a_member_let_in_again_counts_once_and_is_one_former_member() {
        let snapshot = json!({
            "workspace": {"id": WORKSPACE, "name": "acct-lab", "host_uid": 1020},
            "actor": {"id": ALICE, "username": "wanjun", "uid": 1020},
            "principals": [
                {"id": ALICE, "username": "wanjun", "uid": 1020},
                {"id": BOB, "username": "gbellucci", "uid": 1022}
            ],
            "former_principals": [
                {"id": CAROL, "username": "gbellucci", "active": false},
                {"id": DAVE, "username": "gbellucci", "active": false}
            ],
            "teams": [{"id": TEAM, "name": "Account Test", "general_channel_id": GENERAL,
                       "members": [CAROL, BOB, DAVE, ALICE]}],
            "channels": [{"id": GENERAL, "team_id": TEAM, "name": "general",
                          "classification": "restricted", "members": [CAROL, BOB, DAVE, ALICE]},
                         {"id": METHODS, "team_id": TEAM, "name": "methods",
                          "classification": "restricted", "members": [BOB, DAVE, ALICE]}]
        });
        let shown = named(&snapshot, &snapshot);
        assert!(
            shown.contains("Former members (1):\n  @gbellucci · a member again\nTeams"),
            "{shown}"
        );
        assert!(
            !shown.contains("4 members") && !shown.contains("3 members"),
            "{shown}"
        );
        assert_eq!(shown.matches(" · 2 members").count(), 3, "{shown}");
        // channels list counts the same way.
        let listed = named(&snapshot["channels"], &snapshot);
        assert_eq!(listed.matches(" · 2 members").count(), 2, "{listed}");
        // An ID nothing names is still counted: it may be someone the names don't reach.
        let unnamed = named(
            &json!([{"id": GENERAL, "team_id": TEAM, "name": "general",
            "classification": "restricted", "members": [BOB, "someone-else"]}]),
            &snapshot,
        );
        assert!(unnamed.contains(" · 2 members"), "{unnamed}");
    }

    /// The broker sends no UID for a former member, so two former entries of one username that
    /// no member holds now may be two accounts (one deleted on the server, another made with its
    /// name). Each keeps its line, and the IDs tell them apart. Only the earlier entries of a
    /// member again are folded, into that member's one line.
    #[test]
    fn former_entries_of_one_username_stay_apart_without_a_member_now() {
        let snapshot = json!({
            "workspace": {"id": WORKSPACE, "name": "acct-lab", "host_uid": 1020},
            "actor": {"id": ALICE, "username": "wanjun", "uid": 1020},
            "principals": [{"id": ALICE, "username": "wanjun", "uid": 1020}],
            "former_principals": [
                {"id": CAROL, "username": "gbellucci", "active": false},
                {"id": DAVE, "username": "gbellucci", "active": false}
            ],
            "teams": [],
            "channels": []
        });
        let shown = named(&snapshot, &snapshot);
        assert!(
            shown.contains("Former members (2):\n  @gbellucci · former member\n  @gbellucci · former member\n  Some former members share a username. Add --show-ids to tell them apart.\n"),
            "{shown}"
        );
        let ids = with_ids(&snapshot, &snapshot);
        assert!(
            ids.contains(&format!("Former members (2):\n  @gbellucci · former member [ID {CAROL}]\n  @gbellucci · former member [ID {DAVE}]\n")),
            "{ids}"
        );
        assert!(!ids.contains("share a username"), "{ids}");
        // One former entry of a username is one line, and nothing needs telling apart.
        let mut one = snapshot.clone();
        one["former_principals"] = json!([{"id": CAROL, "username": "gbellucci", "active": false}]);
        let shown = named(&one, &one);
        assert!(
            shown.contains("Former members (1):\n  @gbellucci · former member\n"),
            "{shown}"
        );
        assert!(!shown.contains("share a username"), "{shown}");
        // Let in again, the account's earlier entries are its member's one line.
        let mut again = snapshot.clone();
        again["principals"]
            .as_array_mut()
            .expect("principals")
            .push(json!({"id": BOB, "username": "gbellucci", "uid": 1022}));
        let shown = named(&again, &again);
        assert!(
            shown.contains("Former members (1):\n  @gbellucci · a member again\n"),
            "{shown}"
        );
        assert!(!shown.contains("share a username"), "{shown}");
    }

    /// OW-9: a check says what the server is and what is installed; an install says what it did;
    /// a member's setup says it connected and what agents may do.
    #[test]
    fn setup_summaries_say_what_was_found_and_done() {
        // The daemon's `SetupServer` has no Landlock ABI: the check's own facts never say
        // whether the kernel confines commands (C25).
        let check = json!({
            "intent": "check", "state": "finished",
            "server": {"login": "bob@leo", "os": "Linux", "arch": "x86_64", "glibc": "2.35"},
            "installed": {"path": "~/.local/bin/biorouter-crew", "state": "file", "version": "1.92.1"},
            "artifact": {"version": "1.93.0"}
        });
        // AO-8: setup replaces an older Crew by default, so the plain command updates it.
        let found = [
            "bob@leo: Linux x86_64, glibc 2.35.",
            "Crew 1.92.1 is installed in your account there. This computer has Crew 1.93.0; biorouter crew setup updates it.",
        ];
        assert_eq!(
            setup_summary_lines(&check, "bob@leo", None, &SetupAsked::default()),
            found
        );
        // A saved connection's bridge does say, from its last connect (leo: Landlock ABI 1).
        let leo =
            json!({"server_access": {"landlock_abi": 1, "confined_execution_available": false}});
        assert_eq!(
            setup_summary_lines(&check, "bob@leo", Some(&leo), &SetupAsked::default()),
            [
                found[0],
                found[1],
                "Agents can run commands there only with full access: its kernel can't confine them to a folder.",
            ]
        );
        let confined =
            json!({"server_access": {"landlock_abi": 4, "confined_execution_available": true}});
        assert_eq!(
            setup_summary_lines(&check, "bob@leo", Some(&confined), &SetupAsked::default()),
            found
        );
        let mut newer = check.clone();
        newer["installed"]["version"] = json!("1.94.0");
        assert_eq!(
            setup_summary_lines(&newer, "bob@leo", None, &SetupAsked::default())[1],
            "Crew 1.94.0 is installed in your account there, newer than this computer's 1.93.0, so setup leaves it in place."
        );
        let mut missing = check.clone();
        missing["installed"] = json!({"state": "missing"});
        assert_eq!(
            setup_summary_lines(&missing, "bob@leo", None, &SetupAsked::default())[1],
            "Crew isn't installed in your account there. biorouter crew setup installs Crew 1.93.0."
        );

        let install = json!({"intent": "install", "state": "finished", "server": {"login": "bob@leo"},
            "outcome": {"installed_version": "1.93.0", "reused": false}});
        assert_eq!(
            setup_summary_lines(&install, "x", None, &SetupAsked::default()),
            ["Installed Crew 1.93.0 in your account on bob@leo."]
        );
        let mut reused = install.clone();
        reused["outcome"]["reused"] = json!(true);
        assert_eq!(
            setup_summary_lines(&reused, "x", None, &SetupAsked::default()),
            ["Crew 1.93.0 was already installed in your account on bob@leo."]
        );
        // C25: an update replaces the older program and keeps it (`previous_kept`); the version
        // the check found before the install is the one it replaced.
        let mut updated = install.clone();
        updated["outcome"]["previous_kept"] = json!(true);
        updated["installed"] = json!({"state": "file", "version": "1.92.1"});
        assert_eq!(
            setup_summary_lines(&updated, "x", None, &SetupAsked::default()),
            ["Updated Crew from 1.92.1 to 1.93.0 in your account on bob@leo."]
        );
        // The daemon's own `upgraded_from`, as a version or with its digest beside the new one.
        let mut reported = install.clone();
        reported["outcome"]["previous_kept"] = json!(true);
        reported["outcome"]["upgraded_from"] =
            json!({"version": "1.91.0", "sha256": "ab".repeat(32)});
        assert_eq!(
            setup_summary_lines(&reported, "x", None, &SetupAsked::default()),
            ["Updated Crew from 1.91.0 to 1.93.0 in your account on bob@leo."]
        );
        reported["outcome"]["upgraded_from"] = json!("1.90.2");
        assert_eq!(
            setup_summary_lines(&reported, "x", None, &SetupAsked::default()),
            ["Updated Crew from 1.90.2 to 1.93.0 in your account on bob@leo."]
        );
        // The same version with another SHA-256 is replaced, not "updated".
        reported["outcome"]["upgraded_from"] =
            json!({"version": "1.93.0", "sha256": "cd".repeat(32)});
        assert_eq!(
            setup_summary_lines(&reported, "x", None, &SetupAsked::default()),
            ["Replaced Crew 1.93.0 in your account on bob@leo with this computer's copy of it."]
        );

        // C22: what agents may do is what the job set, never read from the folder alone.
        let member = json!({"intent": "member", "state": "finished", "server": {"login": "bob@leo"},
            "outcome": {"installed_version": "1.93.0", "reused": false, "connection_id": "c",
                        "work_folder": "/pool1/home/bob/crew-work/lab", "remote_access": "full"}});
        assert_eq!(
            setup_summary_lines(&member, "x", None, &SetupAsked::default()),
            [
                "Installed Crew 1.93.0 in your account on bob@leo.",
                "Connected.",
                "Your agent can work as you on bob@leo, starting in /pool1/home/bob/crew-work/lab. Change that with biorouter crew connections set-access.",
            ]
        );
        let mut made_only = member.clone();
        made_only["outcome"]
            .as_object_mut()
            .expect("outcome")
            .remove("remote_access");
        assert_eq!(
            setup_summary_lines(&made_only, "x", None, &SetupAsked::default())[2],
            "Your work folder on bob@leo is /pool1/home/bob/crew-work/lab. Choose what agents may do there with biorouter crew connections set-access."
        );
        // The job set folder access; the connection says whether commands run there.
        let mut folder = member.clone();
        folder["outcome"]["remote_access"] = json!("folder");
        let files_only =
            json!({"remote_root": "/pool1/home/bob/crew-work/lab", "remote_execution": false});
        assert_eq!(
            setup_summary_lines(&folder, "x", Some(&files_only), &SetupAsked::default())[2],
            "Agents can read and write files in /pool1/home/bob/crew-work/lab on bob@leo. Its kernel can't confine commands to a folder, so running them there needs full access: biorouter crew connections set-access full."
        );
        let runs =
            json!({"remote_root": "/pool1/home/bob/crew-work/lab", "remote_execution": true});
        assert_eq!(
            setup_summary_lines(&folder, "x", Some(&runs), &SetupAsked::default())[2],
            "Agents can work in /pool1/home/bob/crew-work/lab on bob@leo, confined to that folder. Change that with biorouter crew connections set-access."
        );
        // A re-run on a connection that keeps its own folder changes nothing, and says nothing.
        let mut kept = member.clone();
        let outcome = kept["outcome"].as_object_mut().expect("outcome");
        outcome.remove("remote_access");
        outcome.remove("work_folder");
        let own = json!({"remote_root": "/scratch/bob/proj", "remote_execution": true, "remote_access": "folder"});
        assert_eq!(
            setup_summary_lines(&kept, "x", Some(&own), &SetupAsked::default()),
            [
                "Installed Crew 1.93.0 in your account on bob@leo.",
                "Connected."
            ]
        );
    }

    /// AO-6, AO-8: a check lists the workspaces on the server for the account and, in place of
    /// a bare "Ready", the daemon's sentences for those this computer cannot reach; any job
    /// says what it restarted or started, what still runs an older Crew and why, and which
    /// restarts failed with the command that starts each by hand.
    #[test]
    fn setup_summaries_list_the_servers_workspaces_and_what_was_restarted() {
        let reach = "You host baranzini-lab here (running). This computer has no connection to it. Open it with biorouter crew open baranzini-lab --server wanjun@leo.";
        let check = json!({
            "intent": "check", "state": "finished",
            "server": {"login": "wanjun@leo"},
            "installed": {"state": "file", "version": "1.94.2"},
            "artifact": {"version": "1.94.2"},
            "stages": [stage("workspaces", "done"), {"name": "done", "state": "done", "detail": reach}],
            "workspaces": [
                {"workspace_id": "a", "name": "baranzini-lab", "role": "host", "host_username": "wanjun",
                 "state": "running", "version": "1.94.1", "this_computer": "not_saved"},
                {"workspace_id": "b", "name": "chen-lab", "role": "member", "host_username": "alice",
                 "state": "running", "this_computer": "connected"},
                {"workspace_id": "c", "name": "old-lab", "role": "invited", "host_username": "bob",
                 "state": "stopped", "this_computer": "saved"},
                {"workspace_id": "d", "role": "removed", "state": "unknown", "this_computer": "not_saved"}
            ],
            "workspaces_scan": {"skipped": 0, "unsupported": 2, "truncated": true},
            "outcome": {"outdated": [
                {"workspace_id": "a", "name": "baranzini-lab", "running_version": "1.94.1",
                 "installed_version": "1.94.2", "others_connected": 1, "reason": "not_running_installed"}
            ]}
        });
        assert_eq!(
            setup_summary_lines(&check, "wanjun@leo", None, &SetupAsked::default()),
            [
                "Crew 1.94.2 is installed in your account there.",
                "Workspaces on wanjun@leo for your account:",
                "  baranzini-lab · you host it · running Crew 1.94.1 · not on this computer",
                "  chen-lab · hosted by @alice, you're a member · running · connected on this computer",
                "  old-lab · hosted by @bob, you're invited · stopped · saved on this computer",
                "  A workspace · you were removed · state unknown · not on this computer",
                reach,
                "2 other workspaces on wanjun@leo run an older Crew, which can't say whether you belong to them.",
                "There was more on wanjun@leo than setup looks through at once, so this list may not be complete.",
                "baranzini-lab runs Crew 1.94.1, not 1.94.2. To restart it on Crew 1.94.2, run biorouter crew setup --server wanjun@leo --restart-workspace baranzini-lab; 1 other person is connected, and their running agent commands stop.",
            ]
        );
        // C3: a check asked nothing to restart, so a workspace others are connected to is said
        // as a look, never as one that "wasn't restarted", with the count the server gave. One
        // with no name is restarted by its ID, never with --restart, which restarts them all.
        let mut busy = check.clone();
        busy["outcome"]["outdated"] = json!([
            {"workspace_id": "b", "name": "busy lab", "running_version": "1.94.1",
             "installed_version": "1.94.2", "others_connected": 3, "reason": "others_connected"},
            {"workspace_id": "e", "reason": "others_connected"}
        ]);
        let looked = setup_summary_lines(&busy, "wanjun@leo", None, &SetupAsked::default());
        assert_eq!(
            looked[looked.len() - 2..],
            [
                "busy lab runs Crew 1.94.1, not 1.94.2. To restart it on Crew 1.94.2, run biorouter crew setup --server wanjun@leo --restart-workspace 'busy lab'; 3 other people are connected, and their running agent commands stop.",
                "A workspace runs an older Crew. To restart it on the Crew installed there, run biorouter crew setup --server wanjun@leo --restart-workspace e; other people may be connected, and their running agent commands stop.",
            ]
        );
        assert!(!looked.iter().any(|line| line.contains("wasn't restarted")));
        assert!(!looked.iter().any(|line| line.contains("--restart;")));
        // The same entry from a job asked to restart it, which left it running, says so.
        let mut ran = busy.clone();
        ran["intent"] = json!("install");
        let asked_b = SetupAsked::of(&json!({"restart_workspace_ids": ["b"]}), String::new());
        assert!(setup_summary_lines(&ran, "wanjun@leo", None, &asked_b).contains(
            &"busy lab still runs Crew 1.94.1, not Crew 1.94.2, and 3 other people are connected to it, so it wasn't restarted. To restart it anyway, run biorouter crew setup --server wanjun@leo --restart-workspace 'busy lab'; their running agent commands stop.".to_owned()
        ));
        // The same version number with other bytes (a rebuilt broker, Phase C T6) is said as
        // the Crew installed in the account, never as "Crew 1.94.1, not 1.94.1".
        let mut rebuilt = check.clone();
        rebuilt["outcome"]["outdated"] = json!([
            {"workspace_id": "a", "name": "acct-lab", "running_version": "1.94.1",
             "installed_version": "1.94.1", "others_connected": 0,
             "reason": "not_running_installed"}
        ]);
        let looked = setup_summary_lines(&rebuilt, "wanjun@leo", None, &SetupAsked::default());
        assert_eq!(
            looked.last().map(String::as_str),
            Some("acct-lab runs Crew 1.94.1, not the Crew installed in your account. To restart it on the Crew installed in your account, run biorouter crew setup --server wanjun@leo --restart-workspace acct-lab.")
        );
        let mut asked = rebuilt.clone();
        asked["intent"] = json!("install");
        let asked_a = SetupAsked::of(&json!({"restart_workspace_ids": ["a"]}), String::new());
        let ran = setup_summary_lines(&asked, "wanjun@leo", None, &asked_a);
        assert!(
            ran.iter().any(|line| line.starts_with(
                "acct-lab still runs Crew 1.94.1, not the Crew installed in your account."
            )),
            "{ran:?}"
        );
        assert!(
            !ran.iter()
                .chain(looked.iter())
                .any(|line| line.contains("1.94.1, not 1.94.1")
                    || line.contains("1.94.1, not Crew 1.94.1")),
            "{ran:?}"
        );
        // The reach sentences replace "Ready", so the progress lines leave it out.
        assert!(
            setup_progress_lines(&Value::Null, &check, "wanjun@leo", &SetupNamed::default())
                .iter()
                .all(|line| line != "Ready")
        );
        assert_eq!(
            setup_progress_lines(&Value::Null, &check, "wanjun@leo", &SetupNamed::default()),
            ["Looking for workspaces…"]
        );

        let install = json!({
            "intent": "install", "state": "finished", "server": {"login": "wanjun@leo"},
            "workspaces_note": "Crew 1.94.1 in your account on leo is too old to list workspaces.",
            "outcome": {
                "installed_version": "1.94.2", "reused": false,
                "restarted": [
                    {"workspace_id": "a", "name": "baranzini-lab", "from_version": "1.94.1", "to_version": "1.94.2"},
                    {"workspace_id": "e", "name": "spare-lab", "from_version": null, "to_version": "1.94.2"}
                ],
                "outdated": [
                    {"workspace_id": "b", "name": "busy-lab", "running_version": "1.94.1", "installed_version": "1.94.2", "others_connected": 3, "reason": "others_connected"},
                    {"workspace_id": "c", "name": "new-lab", "running_version": "1.95.0", "installed_version": "1.94.2", "reason": "running_newer"},
                    {"workspace_id": "d", "name": "tmp-lab", "running_version": "1.94.1", "installed_version": "1.94.2", "reason": "custom_state_dir"}
                ],
                "restart_failed": [
                    {"workspace_id": "f", "name": "stuck-lab", "code": "stop_timeout",
                     "message": "The broker did not stop in 15 seconds.",
                     "command": "biorouter-crew start --workspace-id f"}
                ]
            }
        });
        let every = SetupAsked::of(&json!({"restart_workspaces": true}), String::new());
        assert_eq!(
            setup_summary_lines(&install, "wanjun@leo", None, &every),
            [
                "Installed Crew 1.94.2 in your account on wanjun@leo.",
                "Crew 1.94.1 in your account on leo is too old to list workspaces.",
                "Restarted baranzini-lab: it ran Crew 1.94.1 and now runs Crew 1.94.2.",
                "spare-lab now runs Crew 1.94.2.",
                "busy-lab still runs Crew 1.94.1, not Crew 1.94.2, and 3 other people are connected to it, so it wasn't restarted. To restart it anyway, run biorouter crew setup --server wanjun@leo --restart-workspace busy-lab; their running agent commands stop.",
                "new-lab runs Crew 1.95.0, newer than Crew 1.94.2, so it was left running.",
                "tmp-lab still runs Crew 1.94.1, not Crew 1.94.2, and keeps its state outside ~/.local/share/biorouter-crew, so setup can't restart it. Restart it on wanjun@leo yourself.",
                "stuck-lab: The broker did not stop in 15 seconds.",
                "  To start it by hand, run this on wanjun@leo: biorouter-crew start --workspace-id f",
            ]
        );
        assert_eq!(setup_stage_words("workspaces"), "Looking for workspaces");
        assert_eq!(setup_stage_words("restart"), "Updating running workspaces");
        assert_eq!(setup_stage_words("configure"), "Setting up the workspace");
    }

    /// C3: a workspace still on an older Crew reads as left running ("wasn't restarted") only
    /// when the request asked to restart it: every one, or it by ID among those to restart or
    /// start, as the desktop's `askedToRestart` decides. `--no-restart`, a setup that named
    /// another one, a plain setup and a check only looked. The suggested command restarts that
    /// one alone, by name, else ID, else the plain update, with the route flags given, and says
    /// who else is connected as the server counted them.
    #[test]
    fn an_outdated_workspace_is_said_left_only_when_the_setup_was_asked_to_restart_it() {
        let job = |intent: &str, outdated: Value| {
            json!({"intent": intent, "state": "finished", "server": {"login": "wanjun@leo"},
                   "outcome": {"installed_version": "1.94.2", "reused": true, "outdated": [outdated]}})
        };
        let busy = json!({"workspace_id": "B-1", "name": "busy-lab", "running_version": "1.94.1",
                          "installed_version": "1.94.2", "others_connected": 2,
                          "reason": "others_connected"});
        let route = " --port 2222 --identity-file /home/w/.ssh/leo --proxy-jump bastion".to_owned();
        let left = |lines: &[String]| lines.iter().any(|line| line.contains("wasn't restarted"));
        let summary = |status: &Value, request: Value| {
            setup_summary_lines(
                status,
                "wanjun@leo",
                None,
                &SetupAsked::of(&request, route.clone()),
            )
        };
        let install = job("install", busy.clone());
        // Asked: every one, or this one by ID (in either list, any case).
        for request in [
            json!({"restart_workspaces": true}),
            json!({"restart_workspace_ids": ["b-1"]}),
            json!({"start_workspaces": ["B-1"]}),
        ] {
            let lines = summary(&install, request.clone());
            assert!(left(&lines), "{request}: {lines:?}");
            assert_eq!(
                lines.last().map(String::as_str),
                Some("busy-lab still runs Crew 1.94.1, not Crew 1.94.2, and 2 other people are connected to it, so it wasn't restarted. To restart it anyway, run biorouter crew setup --server wanjun@leo --port 2222 --identity-file /home/w/.ssh/leo --proxy-jump bastion --restart-workspace busy-lab; their running agent commands stop.")
            );
        }
        // Not asked: --no-restart, another one named, a plain setup, and a check even with
        // every one asked.
        let looked = "busy-lab runs Crew 1.94.1, not 1.94.2. To restart it on Crew 1.94.2, run biorouter crew setup --server wanjun@leo --port 2222 --identity-file /home/w/.ssh/leo --proxy-jump bastion --restart-workspace busy-lab; 2 other people are connected, and their running agent commands stop.";
        for (status, request) in [
            (&install, json!({"restart_workspaces": false})),
            (&install, json!({"restart_workspace_ids": ["other"]})),
            (&install, json!({})),
            (
                &job("check", busy.clone()),
                json!({"restart_workspaces": true}),
            ),
        ] {
            let lines = summary(status, request.clone());
            assert!(!left(&lines), "{request}: {lines:?}");
            assert_eq!(lines.last().map(String::as_str), Some(looked), "{request}");
        }
        // The count the server gave: one, nobody (no caution), or unknown.
        let counted = |others: Value| {
            let mut entry = busy.clone();
            entry["others_connected"] = others;
            entry["reason"] = json!("not_running_installed");
            summary(&job("install", entry), json!({"restart_workspaces": false}))
                .last()
                .cloned()
                .unwrap_or_default()
        };
        assert!(counted(json!(1)).ends_with("--restart-workspace busy-lab; 1 other person is connected, and their running agent commands stop."));
        assert!(counted(json!(0)).ends_with("--restart-workspace busy-lab."));
        assert!(counted(Value::Null).ends_with("--restart-workspace busy-lab; other people may be connected, and their running agent commands stop."));
        // No name: by ID; neither: the plain update. Never --restart, which restarts them all.
        let mut by_id = busy.clone();
        by_id.as_object_mut().expect("entry").remove("name");
        assert!(summary(&job("install", by_id.clone()), json!({}))
            .last()
            .is_some_and(|line| line.contains("--proxy-jump bastion --restart-workspace B-1;")));
        by_id.as_object_mut().expect("entry").remove("workspace_id");
        let plain = summary(&job("install", by_id), json!({}));
        assert!(
            plain.last().is_some_and(|line| line.contains(
                "run biorouter crew setup --server wanjun@leo --port 2222 --identity-file /home/w/.ssh/leo --proxy-jump bastion; 2 other people are connected"
            )),
            "{plain:?}"
        );
        assert!(!plain.iter().any(|line| line.contains("--restart")));
    }

    /// C3: `--restart-workspace` and `--start` take a workspace's ID too, which is what a setup
    /// summary names one by when the server gave it no name. A name still wins.
    #[test]
    fn a_hosted_workspace_is_matched_by_its_id_when_no_name_matches() {
        let listing = json!({"workspaces": [
            {"workspace_id": "B-1", "role": "host", "state": "running"},
            {"workspace_id": "c-2", "name": "d-3", "role": "host", "state": "stopped"},
            {"workspace_id": "d-3", "name": "other", "role": "host", "state": "running"}
        ]});
        assert_eq!(
            workspace_to_restart(&listing, "b-1", "leo", ""),
            Ok("B-1".to_owned())
        );
        // "d-3" is one workspace's name and another's ID: the name is matched first.
        assert_eq!(
            workspace_to_start(&listing, "d-3", "leo"),
            Ok("c-2".to_owned())
        );
        assert!(workspace_to_restart(&listing, "nope", "leo", "").is_err());
    }

    /// AO-6, AO-9: setup --start and open match a name to one workspace in a check job's
    /// listing, and say in a sentence why when they can't.
    #[test]
    fn a_workspace_is_matched_by_name_in_a_listing() {
        let listing = json!({"workspaces": [
            {"workspace_id": "a", "name": "lab", "role": "host", "state": "stopped", "invitation": "brcrew1:a"},
            {"workspace_id": "b", "name": "lab", "role": "host", "state": "deleted"},
            {"workspace_id": "c", "name": "shared", "role": "member", "host_username": "alice", "state": "stopped", "invitation": "brcrew1:c"}
        ]});
        assert_eq!(
            workspace_to_start(&listing, "LAB", "leo"),
            Ok("a".to_owned())
        );
        assert_eq!(
            workspace_to_start(&listing, "shared", "leo"),
            Err("You don't host a workspace named shared on leo.".to_owned())
        );
        assert_eq!(
            workspace_to_open(&listing, "shared", None, "leo", ""),
            Err("shared isn't running on leo. Ask @alice to start it.".to_owned())
        );
        assert_eq!(
            workspace_to_open(&listing, "shared", Some("bob"), "leo", ""),
            Err(
                "No workspace named shared hosted by @bob on leo is yours or invites you."
                    .to_owned()
            )
        );
        let unlisted = json!({"workspaces": [], "workspaces_note": "Crew 1.94.1 in your account on leo is too old to list workspaces."});
        assert_eq!(
            workspace_to_start(&unlisted, "lab", "leo"),
            Err("Biorouter couldn't list the workspaces on leo. Crew 1.94.1 in your account on leo is too old to list workspaces.".to_owned())
        );
        let deleted = json!({"workspaces": [{"workspace_id": "b", "name": "lab", "role": "host", "state": "deleted"}]});
        assert_eq!(
            workspace_to_start(&deleted, "lab", "leo"),
            Err("lab on leo was deleted, so it can't be started.".to_owned())
        );
        assert_eq!(
            workspace_to_open(&deleted, "lab", None, "leo", ""),
            Err("lab on leo was deleted by its host.".to_owned())
        );
    }

    /// AO-5, AO-9: removing a computer and adding this one again are said in the desktop's
    /// words (`deviceCopy` and `rejoinCopy`), read from its copy deck so the two cannot drift.
    #[test]
    fn removing_and_adding_a_computer_again_say_the_desktops_words() {
        let copy = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ui/desktop/src/components/crew/account/copy.ts"),
        )
        .expect("the desktop copy deck");
        assert!(copy.contains(&format!("'{REMOVE_DEVICE_SSH_FIRST}'")));
        assert_eq!(remove_device_warning(false), REMOVE_DEVICE_SSH_FIRST);
        assert!(copy.contains("`Remove ${fingerprint} from your account?`"));
        assert_eq!(
            remove_device_question("AAAA BBBB CCCC DDDD", false),
            "Remove AAAA BBBB CCCC DDDD from your account?"
        );
        assert!(copy.contains("'Remove this computer from your account?'"));
        assert_eq!(
            remove_device_question("AAAA BBBB CCCC DDDD", true),
            "Remove this computer from your account?"
        );
        // This computer's: the desktop's sentence, its button here the rejoin command.
        assert!(copy.contains(
            "'This computer stops reaching the workspace until you choose Add this computer again.'"
        ));
        assert_eq!(
            remove_device_warning(true),
            "This computer stops reaching the workspace until you add it again with biorouter crew rejoin."
        );
        assert!(device_revoked_lines("AAAA BBBB CCCC DDDD", false, "lab")[0]
            .starts_with("Removed AAAA BBBB CCCC DDDD."));
        assert!(device_revoked_lines("AAAA", true, "lab")[0].starts_with("Removed this computer."));

        assert!(copy.contains(
            "`This computer is back in ${workspace}. Chats that had Crew access through it need access again.`"
        ));
        assert!(copy.contains(
            "`To make room, the workspace removed your oldest computer, ${fingerprint}.`"
        ));
        let rejoined = json!({
            "status": {"workspace_name": "lab"},
            "principal": {"username": "bob", "display_name": "Bob Lee"},
            "evicted_device": {"fingerprint": "AAAA BBBB CCCC DDDD", "added_at": 1_790_200_000}
        });
        assert_eq!(
            rejoined_lines(&rejoined),
            [
                "This computer is back in lab. Chats that had Crew access through it need access again.",
                "To make room, the workspace removed your oldest computer, AAAA BBBB CCCC DDDD.",
                "Give a chat access again with biorouter crew grants grant SESSION CHANNEL.",
            ]
        );
        // The desktop names no person, so neither does this.
        assert!(!rejoined_lines(&rejoined)
            .iter()
            .any(|line| line.contains("@bob")));
        assert_eq!(
            rejoined_lines(&json!({"status": {}})),
            [
                "This computer is back in the workspace. Chats that had Crew access through it need access again.",
                "Give a chat access again with biorouter crew grants grant SESSION CHANNEL.",
            ]
        );
    }

    /// AO-5, AO-12, C2: a computer's row says how it was added, in the desktop's words, and
    /// which is this computer; the grants list names a chat that reads a workspace as the person.
    #[test]
    fn devices_and_reads_are_said_in_words() {
        let options = HumanOptions::in_utc_at(1_790_214_655);
        let device = json!({"fingerprint": "3F2A 9C1E 77B0 D4E1", "added_at": 1_790_200_000, "added_via": "account", "this_device": true});
        assert_eq!(
            device_text(&device, &options),
            "3F2A 9C1E 77B0 D4E1 · added Sep 23, 2026 · by signing in to the server · This computer"
        );
        // C2: how a computer was added reads as the desktop's notice and Keys say it, byte for
        // byte (`addedViaCopy`), and one this build does not know has no words.
        let copy = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ui/desktop/src/components/crew/account/copy.ts"),
        )
        .expect("the desktop copy deck");
        for (via, said) in [
            ("account", "by signing in to the server"),
            ("bootstrap", "when the workspace was created"),
            ("token", "with an invitation token"),
            ("invitation_code", "with an invitation"),
        ] {
            let row = device_text(&json!({"fingerprint": "AAAA", "added_via": via}), &options);
            assert_eq!(row, format!("AAAA · {said}"));
            assert!(copy.contains(&format!("{via}: '{said}',")), "{via}: {said}");
            // A chat reads the same words, from the one table both use.
            assert_eq!(biorouter::crew::added_via_words(via), Some(said));
        }
        assert_eq!(biorouter::crew::added_via_words("migration"), None);
        assert_eq!(
            device_text(&json!({"added_via": "migration"}), &options),
            "Unknown computer"
        );
        // The status line's new computer, as the desktop's notice says it.
        let ctx = Ctx::new(Directory::default(), false, options.clock);
        assert_eq!(
            ctx.new_device_text(&device),
            "3F2A 9C1E 77B0 D4E1, added Sep 23, 2026 by signing in to the server"
        );
        assert_eq!(
            ctx.new_device_text(&json!({"fingerprint": "AAAA", "added_via": "account"})),
            "AAAA, added by signing in to the server"
        );
        assert_eq!(
            ctx.new_device_text(&json!({"fingerprint": "AAAA", "added_via": "migration"})),
            "AAAA"
        );
        let grants = json!({"grants": [], "person_reads": [
            {"session_id": "20260924_2", "connection_id": "c", "granted_at": 1_790_200_000, "connection_name": "lab"}
        ]});
        assert_eq!(
            render_text(&grants, &options),
            "Chat 20260924_2 reads lab as you · since Sep 23, 2026"
        );
        assert_eq!(
            render_text(&json!({"grants": []}), &options),
            "No chats have Crew access."
        );
        assert_eq!(
            removed_sentence(None),
            "You were removed from this workspace. Ask a member to invite you again."
        );
    }

    /// L3: a failed start's own words are said under its sentence, a bounded number of lines.
    #[test]
    fn a_setup_failure_shows_what_the_server_printed() {
        assert_eq!(with_setup_detail("It failed.", None), "It failed.");
        assert_eq!(with_setup_detail("It failed.", Some("  ")), "It failed.");
        assert_eq!(
            with_setup_detail(
                "Crew didn't start the workspace on the server.",
                Some("unsafe_storage: writable non-sticky ancestor\n/pool1/home/wanjun/.local")
            ),
            "Crew didn't start the workspace on the server.\n  Details: unsafe_storage: writable non-sticky ancestor\n    /pool1/home/wanjun/.local"
        );
        let long: Vec<String> = (0..25).map(|line| format!("line {line}")).collect();
        let shown = with_setup_detail("It failed.", Some(&long.join("\n")));
        assert_eq!(shown.lines().count(), 1 + 20 + 1, "{shown}");
        assert!(
            shown.ends_with("(5 more lines in --output-format json)"),
            "{shown}"
        );
        // The storage refusal names the flag that fixes it, unless the daemon already did.
        assert!(is_setup_code("crew_host_storage_unsafe"));
        assert_eq!(
            setup_failure_sentence(
                "crew_host_storage_unsafe",
                Some("Other accounts can write to /pool1/home/wanjun/.local. Run chmod go-w /pool1/home/wanjun/.local on the server.")
            ),
            "Other accounts can write to /pool1/home/wanjun/.local. Run chmod go-w /pool1/home/wanjun/.local on the server.\nOr run the same command with --fix-permissions, and setup removes that write access for you."
        );
        assert_eq!(
            setup_failure_sentence(
                "crew_host_storage_unsafe",
                Some("Run it again with --fix-permissions.")
            ),
            "Run it again with --fix-permissions."
        );
    }

    /// L8: a channel's files, newest first, each with who shared it, when, and the attachment ID
    /// files download takes, whatever --show-ids says.
    #[test]
    fn a_channel_files_are_listed_with_their_attachment_ids() {
        let snapshot = alice_snapshot();
        let options = options(false, Directory::from_snapshot(&snapshot));
        let listing = json!({"channel_id": "nope", "files": [
            {"attachment_id": "b-2", "name": "fig.pdf", "size": 2048, "shared_by": BOB, "run_id": "r", "shared_at": NOW - 60},
            {"attachment_id": "b-1", "name": "plate_counts.csv", "size": 1200, "shared_by": BOB, "shared_at": NOW - 120}
        ], "unavailable": 1, "complete": false});
        let lines = channel_files_lines(&listing, &options);
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert!(
            lines[0].starts_with("fig.pdf (2.0 KB) · shared by the agent of "),
            "{lines:?}"
        );
        assert!(lines[0].ends_with(" · attachment ID b-2"), "{lines:?}");
        assert!(
            lines[1].starts_with("plate_counts.csv (1.2 KB) · shared by "),
            "{lines:?}"
        );
        assert!(lines[1].contains("(@crew_bob)"), "{lines:?}");
        assert!(
            lines[1].ends_with(" · 01:48 · attachment ID b-1"),
            "{lines:?}"
        );
        assert_eq!(
            lines[2],
            "1 more file shared there can't be opened any more."
        );
        assert!(lines[3].starts_with("Only the channel's newest messages were read"));
        assert_eq!(
            lines[4],
            "Download one with biorouter crew files download ATTACHMENT_ID --output FILE."
        );
        let empty = channel_files_lines(&json!({"files": [], "complete": true}), &options);
        assert_eq!(empty.len(), 1, "{empty:?}");
        assert!(
            empty[0].starts_with("No files were shared in "),
            "{empty:?}"
        );
    }

    /// OW-9: a setup refusal is the daemon's sentence when it wrote one, else the command
    /// line's, with the flag that acts on it here.
    #[test]
    fn setup_failures_say_the_flag_that_acts_on_them() {
        assert!(is_setup_code("crew_install_incompatible"));
        assert!(is_setup_code("crew_setup_busy"));
        assert!(!is_setup_code("crew_ssh_unreachable"));
        assert_eq!(
            setup_failure_sentence("crew_install_incompatible", Some("Crew 1.80.0 doesn't run.")),
            "Crew 1.80.0 doesn't run.\nTo replace it, run the same command with --replace-existing; the old one is kept as biorouter-crew.previous."
        );
        assert_eq!(
            setup_failure_sentence("crew_install_no_space", None),
            "Your account on the server doesn't have room for Crew. Free some space, then run the same command again."
        );
        // A sentence that already names the flag is not told it again.
        assert_eq!(
            setup_failure_sentence(
                "crew_install_incompatible",
                Some("Set it up again with --replace-existing.")
            ),
            "Set it up again with --replace-existing."
        );
        for (code, text) in SETUP_FAILURES {
            assert!(code.starts_with("crew_"), "{code}");
            assert!(reads_as_sentence(text) || text.starts_with('~'), "{text}");
        }
    }

    /// OW-8: the context document is printed as written, then who changed it and when.
    #[test]
    fn the_workspace_context_names_who_changed_it() {
        let snapshot = alice_snapshot();
        let options = options(false, Directory::from_snapshot(&snapshot));
        let context = json!({"text": "# Leo\n\nNo job scheduler.\n", "revision": 2,
            "updated_by": BOB, "updated_at": NOW - 120});
        let lines = workspace_context_lines(&context, &options);
        assert_eq!(lines[..3], ["# Leo", "", "No job scheduler."]);
        assert_eq!(lines[3], "");
        assert!(
            lines[4].starts_with("Revision 2 · changed by "),
            "{lines:?}"
        );
        assert!(lines[4].contains("(@crew_bob)"), "{lines:?}");
        assert!(lines[4].ends_with(" · 01:48"), "{lines:?}");
        assert_eq!(
            workspace_context_lines(&Value::Null, &options)[0],
            "This workspace has no context document yet."
        );
    }
}
