//! Output for `biorouter crew`.
//!
//! `--output-format json` and `stream-json` print the daemon's values as they are, made
//! terminal-safe and nothing else, so scripts keep every ID. Text output is for people:
//! type-specific formatters name people, teams and channels, and machine IDs appear only when
//! [`HumanOptions::show_ids`] asks for them ("Machine IDs stay internal" in
//! `docs/research/biorouter-crew/naming-design.md`).
//!
//! A person renders as `"Display name" (@username)`. The display name is text a colleague
//! chose, so it is quoted, escaped and wrapped in Unicode isolates (U+2068 … U+2069): right-to-
//! left text inside it cannot reorder the `@username` that follows. When the display name equals
//! the username, `@username` alone is printed, so the username is on every line either way.
//!
//! Names come from a [`Directory`]: the value being printed (a snapshot, or a message page's
//! `people` and `channel_names` maps) plus whatever the caller already knows, such as the
//! workspace snapshot fetched for a history page. An ID the directory cannot name prints as
//! "Unknown member", "this team" or "this channel", never as the ID.

use super::args::OutputFormat;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone, Utc};
use serde_json::{Map, Value};
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

pub fn safe_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| {
            if terminal_control(ch) {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
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
    "workspace operation quota requires maintenance",
];
const IDENTITY_CONFLICT_UNNAMED: &str =
    "Another active member already has this username. Remove the old member first.";

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
];

/// Whether the broker wrote `sentence` for a person: capitalised (or opening with a number or
/// an `@username`), and ending in a full stop. The broker's technical texts are neither.
fn reads_as_sentence(sentence: &str) -> bool {
    let first = sentence.chars().next();
    first.is_some_and(|ch| ch.is_uppercase() || ch.is_numeric() || ch == '@')
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
    (
        "channel unavailable",
        "That channel isn't available to you. It may be archived, or you may not be in it.",
    ),
    (
        "principal unavailable",
        "That person isn't a member of this workspace.",
    ),
    (
        "invalid grant",
        "This task's access to the workspace has ended.",
    ),
];

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
    let shown = match (code, username_in(sentence)) {
        ("identity_conflict", named) if lower.starts_with("another active member is @") => {
            identity_conflict(named)
        }
        ("device_conflict", _) => DEVICE_CONFLICT.to_owned(),
        ("identity_mismatch", _) if !reads_as_sentence(sentence) => IDENTITY_MISMATCH.to_owned(),
        // `broker.rs`'s limits that mean "this workspace is full": the audit journal and
        // state-size limits in `commit` and the operation quota in `apply_mutation`, which are
        // what a request meets, plus the journal limit in `open_inner`, which only stops the
        // broker starting. The join quota's own sentence means "too many people are waiting" and
        // is kept. The desktop matches the same texts (`STORAGE_FULL_TEXT` in `refusals.ts`).
        ("quota_exceeded", _) if STORAGE_FULL_PREFIXES.iter().any(|p| lower.starts_with(p)) => {
            STORAGE_FULL.to_owned()
        }
        ("rate_limited", _) if !reads_as_sentence(sentence) => TOO_MANY_ATTEMPTS.to_owned(),
        ("already_approved", Some(name)) => already_approved(name),
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
            .map(safe_text)
    };
    let model = field("model").unwrap_or_else(|| safe_text(requested_model));
    let workspace = field("workspace").unwrap_or_else(|| "This workspace".to_owned());
    let approved_for: Option<Vec<String>> = details
        .and_then(|details| details.get("approved_for"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(safe_text)
                .collect::<Vec<_>>()
        })
        .filter(|ids| !ids.is_empty());
    const CHOOSE: &str = "Choose a model approved for it, or a local model.";
    match (field("workspace_institution"), approved_for) {
        (Some(institution), Some(approved)) => format!(
            "{model} is approved for {}. {workspace} uses {institution}. {CHOOSE}",
            approved.join(" and ")
        ),
        (Some(institution), None) => format!(
            "{model} doesn't say which institution approved it. {workspace} uses {institution}. {CHOOSE}"
        ),
        (None, _) => {
            format!("{model} isn't approved for this workspace's institution. {CHOOSE}")
        }
    }
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
    #[allow(dead_code)] // The command layer passes a snapshot's names to history and lists.
    pub fn from_snapshot(snapshot: &Value) -> Self {
        let mut directory = Self::default();
        directory.absorb(snapshot);
        directory
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

/// A display name quoted like a string literal, escaped for the terminal and isolated so its
/// direction cannot leak into the text around it.
fn quoted_name(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '"' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    format!("\"\u{2068}{}\u{2069}\"", safe_text(&escaped))
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
    let label = directory
        .people
        .get(principal_id)
        .map_or_else(|| UNKNOWN_MEMBER.to_owned(), Person::label);
    Ctx::new(directory.clone(), show_ids, Clock::System).with_id(label, "ID", Some(principal_id))
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
    ctx.render(view, value).join("\n")
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
            View::Grants => self.rows(
                value,
                "grants",
                "No chats have Crew access.",
                Self::grant_row,
            ),
            View::Privacy => self.privacy(value),
            View::Transfers => self.transfers(value),
            View::Transfer => self.transfer(value),
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
        let label = id
            .and_then(|id| self.dir.people.get(id))
            .map_or_else(|| UNKNOWN_MEMBER.to_owned(), |person| person.label());
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
        let name = id
            .and_then(|id| self.dir.teams.get(id))
            .map_or_else(|| "this team".to_owned(), |name| display_text(name));
        self.with_id(name, "team ID", id)
    }

    fn channel(&self, id: Option<&str>) -> String {
        let name = id
            .and_then(|id| self.dir.channels.get(id))
            .and_then(|channel| channel.name.as_deref())
            .filter(|name| !name.is_empty())
            .map_or_else(|| "this channel".to_owned(), channel_name);
        self.with_id(name, "channel ID", id)
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
            let items = list_key(snapshot, key);
            if items.is_empty() {
                if matches!(title, "People" | "Teams" | "Channels") {
                    out.push(format!("{title}: none"));
                }
                continue;
            }
            out.push(format!("{title} ({}):", items.len()));
            for item in items {
                out.extend(row(self, item).into_iter().map(|line| format!("  {line}")));
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
        let mut row = person.label();
        if self.is_actor(id) {
            row.push_str(" · you");
        }
        if id.is_some() && id == self.dir.host_id() {
            row.push_str(" · host");
        }
        if person.stale {
            row.push_str(" · account no longer valid");
        }
        row = self.with_id(row, "ID", id);
        if let Some(uid) = person.uid.filter(|_| self.show_ids) {
            let _ = write!(row, " [UID {uid}]");
        }
        vec![row]
    }

    fn profile(&self, principal: &Value) -> Vec<String> {
        let mut out = self.member_row(principal);
        let devices = list_key(principal, "devices");
        if !devices.is_empty() {
            out.push(format!("  Devices ({}):", devices.len()));
            for device in devices {
                out.push(format!("    {}", self.device_row(device)));
            }
        }
        out
    }

    fn device_row(&self, device: &Value) -> String {
        let mut parts = vec![
            str_field(device, "fingerprint").map_or_else(|| "Unknown device".into(), safe_text)
        ];
        if let Some(at) = device.get("added_at").and_then(Value::as_i64) {
            parts.push(format!("added {}", self.date(at)));
        }
        if let Some(via) = str_field(device, "added_via") {
            let via = match via {
                "bootstrap" => "workspace setup".into(),
                "token" => "enrollment token".into(),
                "invitation_code" => "invitation code".into(),
                other => sentence_case(other).to_lowercase(),
            };
            parts.push(format!("via {via}"));
        }
        parts.join(" · ")
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
            parts.push(count(members.len(), "member", "members"));
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
            parts.push(count(members.len(), "member", "members"));
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
            "No invitations.",
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
        if let Some(status) = str_field(connection, "status") {
            parts.push(sentence_case(status));
        }
        parts.push(connection_privacy(connection));
        let mut out = vec![self.with_id(
            parts.join(" · "),
            "connection ID",
            str_field(connection, "id"),
        )];
        if let Some(error) = str_field(connection, "last_error") {
            out.push(format!("  Last error: {}", safe_text(error)));
        }
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
        if let Some(status) = str_field(connection, "status") {
            out.push(format!("  Status: {}", sentence_case(status)));
        }
        let mut privacy = vec![mode_word(str_field(connection, "mode"))];
        privacy.push(institution(connection));
        if let Some(epoch) = connection.get("policy_epoch").and_then(Value::as_u64) {
            privacy.push(format!("policy epoch {epoch}"));
        }
        out.push(format!("  Privacy: {}", privacy.join(" · ")));
        if let Some(root) = str_field(connection, "remote_root") {
            let execution = connection.get("remote_execution").and_then(Value::as_bool);
            let access = if execution == Some(true) {
                "agents may run commands there"
            } else {
                "agents may read files there"
            };
            out.push(format!("  Remote folder: {} · {access}", safe_text(root)));
        }
        if let Some(file) = str_field(connection, "identity_file") {
            out.push(format!("  SSH key file: {}", safe_text(file)));
        }
        if let Some(error) = str_field(connection, "last_error") {
            out.push(format!("  Last error: {}", safe_text(error)));
        }
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

    fn message_extras(&self, message: &Value) -> Vec<String> {
        let mut out = Vec::new();
        let attachments = str_list(message, "attachments");
        if !attachments.is_empty() {
            let mut line = format!(
                "    {}",
                count(attachments.len(), "attachment", "attachments")
            );
            if self.show_ids {
                let _ = write!(line, " [attachment IDs {}]", join_safe(&attachments));
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
        if let Some(session) = str_field(run, "session_id") {
            out.push(format!("  Chat: {}", safe_text(session)));
        }
        if let Some(error) = str_field(run, "error") {
            out.push(format!("  Error: {}", safe_text(error)));
        }
        out.extend(self.detail_ids(run, &[("connection_id", "Connection ID")]));
        out
    }

    fn grant_row(&self, grant: &Value) -> Vec<String> {
        let session = str_field(grant, "session_id").map_or_else(|| "this chat".into(), safe_text);
        let chat = match str_field(grant, "title") {
            Some(title) => format!("{} (chat {session})", quoted_name(title)),
            None => format!("Chat {session}"),
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
        let expired = grant.get("expired").and_then(Value::as_bool) == Some(true);
        row.push_str(if expired { " · Expired" } else { " · Active" });
        if let Some(epoch) = grant.get("policy_epoch").and_then(Value::as_u64) {
            let _ = write!(row, " · policy epoch {epoch}");
        }
        vec![self.with_id(row, "task ID", str_field(grant, "run_id"))]
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
        let mut out = vec![
            self.with_id(
                format!("Your connection: {}", connection.join(" · ")),
                "connection ID",
                str_field(value, "connection_id"),
            ),
            self.with_id(
                format!("Workspace: {}", workspace_privacy(workspace)),
                "workspace ID",
                str_field(workspace, "id"),
            ),
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
            .flat_map(|transfer| self.transfer(transfer))
            .collect();
        self.ids_hint(&mut out, "files status, resume, pause and forget");
        out
    }

    fn transfer(&self, transfer: &Value) -> Vec<String> {
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
        if let Some(error) = str_field(transfer, "error") {
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
                let state = str_field(value, "state").unwrap_or("unknown");
                format!("Transfer {}.", transfer_state_word(state, "", 0, None))
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
        out.extend(self.detail_ids(value, &[("device_id", "Device ID")]));
        if let Some(workspace) = value.get("workspace") {
            out.extend(self.detail_ids(workspace, &[("id", "Workspace ID")]));
        }
        out
    }

    fn prepared(&self, value: &Value) -> Vec<String> {
        let key = str_field(value, "public_key").map_or_else(String::new, safe_text);
        let mut out = vec![
            "Device key prepared. Give this public key to the workspace host:".to_owned(),
            format!("  {key}"),
        ];
        if self.show_ids {
            out.extend(self.detail_ids(
                value,
                &[
                    ("preparation_id", "Preparation ID"),
                    ("device_id", "Device ID"),
                ],
            ));
        } else {
            out.push("Add --show-ids for the preparation ID that connections save takes.".into());
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
        out.extend(self.detail_ids(value, &[("uid", "UID"), ("device_id", "Device ID")]));
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
                out.push("It has no human approval authority; restart it with the trusted Crew terminal launcher.".into());
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

const CONNECTION_IDS: &[(&str, &str)] = &[
    ("id", "Connection ID"),
    ("workspace_id", "Workspace ID"),
    ("workspace_public_key", "Workspace key"),
    ("node_id", "Node ID"),
    ("device_id", "Device ID"),
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
fn run_status_word(status: &str) -> String {
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

    /// The display-name half of a person label: quoted and isolated.
    fn q(name: &str) -> String {
        format!("\"\u{2068}{name}\u{2069}\"")
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
            "  @crew_carol".into(),
            format!("  {} · you · host", alice()),
            format!("  {}", bob()),
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
            format!(
                "  Crew QA Lab (crew-qa-lab) · 3 members · created by {}",
                alice()
            ),
            "Channels (1):".into(),
            format!(
                "  #general · Crew QA Lab · Restricted · 3 members · owner {}",
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
            format!("{} · you\n  Devices (1):\n    70DC-FFAF-1751-A59B · added Sep 24, 2026 · via enrollment token", bob())
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
        assert_eq!(
            text,
            "\"\u{2068}Bob \\\"Lee\\\" (@bob)\\\\\\u{202e}gnp.exe\\u{2069}\u{2069}\" (@mallory)"
        );
        // One isolate pair, opened and closed by the formatter: the name's own U+2069 was
        // escaped, so it cannot close the isolate early.
        assert_eq!(text.matches('\u{2068}').count(), 1);
        assert_eq!(text.matches('\u{2069}').count(), 1);
        assert!(!text.contains('\u{202e}'));
        assert!(text.ends_with("\" (@mallory)"));
        let rtl = plain(&json!([{"id":"p2","username":"dana","nickname":"דנה"}]));
        assert_eq!(rtl, "\"\u{2068}דנה\u{2069}\" (@dana)");
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
        assert_eq!(
            text,
            format!("Device key prepared. Give this public key to the workspace host:\n  {KEY}\nAdd --show-ids for the preparation ID that connections save takes.")
        );
        assert!(!text.contains(CONNECTION) && !text.contains(DEVICE));
        assert!(with_ids(&prepared, &json!({})).contains(CONNECTION));
        let token = "4f".repeat(32);
        let invite =
            json!({"invitation":token,"expires_at":NOW + 3600,"uid":10002,"device_id":DEVICE});
        let text = plain(&invite);
        assert_eq!(
            text,
            format!("Send this enrollment token to your colleague privately. It expires in 1 hour.\n  {token}")
        );
        assert!(!text.contains(DEVICE) && !text.contains("10002"));
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
        (
            "quota_exceeded",
            "quota_exceeded: workspace operation quota requires maintenance",
            STORAGE_FULL,
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
            "@bob is already a member. Choose Add device to add another computer for them.\nTo add another computer for them, run: biorouter crew enroll invite @bob --add-device",
        ),
        (
            "code_mismatch",
            "code_mismatch: The host hasn't let this device in. Send the host the code shown on your screen.",
            "The host hasn't let this device in. Send the host the code shown on your screen.",
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
            "That channel isn't available to you. It may be archived, or you may not be in it.",
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
    #[test]
    fn each_storage_full_fixture_is_the_brokers_literal_text() {
        const BROKER: &str = include_str!("../../../../biorouter-crew/src/broker.rs");
        let storage_full: Vec<&str> = BROKER_REFUSALS
            .iter()
            .filter(|(_, _, shown)| *shown == STORAGE_FULL)
            .map(|(_, broker, _)| *broker)
            .collect();
        // The journal limit at startup and in `commit`, the state-size limit and the operation
        // quota.
        assert_eq!(storage_full.len(), 4, "{storage_full:#?}");
        for text in storage_full {
            assert!(
                BROKER.contains(&format!("\"{text}\"")),
                "{text} is not a literal in broker.rs"
            );
        }
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
            TOO_MANY_ATTEMPTS,
            IDENTITY_CONFLICT_UNNAMED,
        ] {
            assert!(
                copy.contains(&format!("'{sentence}'")),
                "{sentence} is not in {}",
                path.display()
            );
        }
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
        assert_eq!(
            institution_refusal_text("gpt-5.5-2026-04-24", Some(&details)),
            "gpt-5.5-2026-04-24 is approved for ucsf. foreign-lab uses stanford. Choose a model approved for it, or a local model."
        );
        let unstated = json!({"model": "private-model", "approved_for": null, "workspace": "lab", "workspace_institution": "ucsf"});
        assert_eq!(
            institution_refusal_text("private-model", Some(&unstated)),
            "private-model doesn't say which institution approved it. lab uses ucsf. Choose a model approved for it, or a local model."
        );
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
}
