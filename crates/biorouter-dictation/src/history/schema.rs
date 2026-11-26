//! The history's two files, v1, snake_case on disk.
//!
//! `history.jsonl` is a header line, `{"format":"biorouter.dictation.history","version":1}`,
//! then one entry per line, oldest first. `settings.json` holds the person's two switches
//! in a file of their own, so turning the history off works while `history.jsonl` cannot
//! be read, and a toggle never rewrites the history.
//!
//! Every type refuses a key it does not know, for the reason the dictionary's schema gives:
//! an older binary that met a newer file's field must fail closed rather than drop the
//! field on its next rewrite. A format, a version, a middle line or an id this version
//! cannot read makes the whole file unreadable; only the final line, when nothing ends
//! it, is left out, because that is the residue of an append that did not finish.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::schema::{normalize_institution_id, MAX_ROW_INSTITUTIONS};
use crate::store::EntryTier;

/// `history.jsonl`'s header `format`.
pub const HISTORY_FORMAT: &str = "biorouter.dictation.history";
/// `history.jsonl`'s header `version`.
pub const HISTORY_VERSION: u32 = 1;
/// `settings.json`'s `format`.
pub const SETTINGS_FORMAT: &str = "biorouter.dictation.history.settings";
/// `settings.json`'s `version`.
pub const SETTINGS_VERSION: u32 = 1;

/// The entries.
pub const HISTORY_FILE: &str = "history.jsonl";
/// The person's two switches.
pub const SETTINGS_FILE: &str = "settings.json";
/// The lock every writer takes.
pub const LOCK_FILE: &str = ".lock";

/// The most entries the history keeps.
pub const MAX_ENTRIES: usize = 5_000;
/// The largest `history.jsonl` the history keeps, header included.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The longest text one entry keeps, in UTF-8 bytes. A longer text is cut at a character
/// boundary and the entry carries `truncated: true`.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
/// What a prune leaves, in percent of both caps, so a full history rewrites about once
/// in 500 dictations rather than on every one.
pub const PRUNE_TO_PERCENT: u64 = 90;

/// An entry id's prefix: `dch_` and a random UUID v4, never derived from the text.
pub const ENTRY_ID_PREFIX: &str = "dch_";

/// The longest model id an entry keeps.
const MAX_MODEL_CHARS: usize = 128;
/// The longest chat id an entry keeps.
const MAX_SESSION_ID_CHARS: usize = 256;

/// Where a dictation went. A label only: nothing reads it for privacy or for bias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// A chat's composer, Home, a new chat, or a message being edited.
    Composer,
    Crew,
    Clipboard,
    /// Typed into the app that was in front when the dictation started.
    OtherApp,
    AppPreview,
    Terminal,
}

impl Surface {
    pub const ALL: [Surface; 6] = [
        Surface::Composer,
        Surface::Crew,
        Surface::Clipboard,
        Surface::OtherApp,
        Surface::AppPreview,
        Surface::Terminal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Composer => "composer",
            Surface::Crew => "crew",
            Surface::Clipboard => "clipboard",
            Surface::OtherApp => "other_app",
            Surface::AppPreview => "app_preview",
            Surface::Terminal => "terminal",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|surface| surface.as_str() == text)
    }
}

/// One dictation's text, as the engine produced it once its stream finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    /// `dch_` and a random UUID v4 ([`new_entry_id`]).
    pub id: String,
    /// When the transcript was ready, in whole seconds.
    pub created_at: DateTime<Utc>,
    /// The corrected text, at most [`MAX_TEXT_BYTES`].
    pub text: String,
    pub tier: EntryTier,
    /// Private entries only, with the dictionary row's meaning (P-16): absent or `null`
    /// for no institution, `[]` for only a model that runs on this computer, ids
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub institutions: Option<Vec<String>>,
    pub surface: Surface,
    pub duration_ms: u64,
    /// The speech model's catalog id.
    pub model: String,
    /// Whether recognition used the person's private dictionary terms.
    pub private_terms: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_incarnation: Option<i64>,
    /// Set by the store when the text was cut to [`MAX_TEXT_BYTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// A fresh entry id: `dch_` and a random UUID v4.
pub fn new_entry_id() -> String {
    format!("{ENTRY_ID_PREFIX}{}", uuid::Uuid::new_v4().hyphenated())
}

/// Whether `id` has the shape [`new_entry_id`] gives: `dch_` and a lowercase hyphenated
/// UUID.
pub fn is_entry_id(id: &str) -> bool {
    id.strip_prefix(ENTRY_ID_PREFIX)
        .and_then(|rest| uuid::Uuid::parse_str(rest).ok().map(|uuid| (rest, uuid)))
        .is_some_and(|(rest, uuid)| uuid.hyphenated().to_string() == rest)
}

/// `text` cut to at most `max` UTF-8 bytes on a character boundary, and whether anything
/// was cut.
pub fn cut_text(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text.get(..end).unwrap_or_default(), true)
}

/// Why an entry is not one this version keeps. The store refuses to append it and, read
/// from the file, it makes the file unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryProblem {
    Id,
    EmptyText,
    TextTooLong,
    /// A public entry names institutions, or a private one names more than eight, or an
    /// id that is not normalized.
    Institutions,
    Model,
    SessionId,
}

impl EntryProblem {
    pub fn message(self) -> &'static str {
        match self {
            EntryProblem::Id => "A dictation history entry id must be dch_ and a UUID.",
            EntryProblem::EmptyText => "A dictation history entry needs text.",
            EntryProblem::TextTooLong => "A dictation history entry's text is too long.",
            EntryProblem::Institutions => {
                "A dictation history entry's institutions do not fit its tier."
            }
            EntryProblem::Model => "A dictation history entry's model id is too long.",
            EntryProblem::SessionId => "A dictation history entry's chat id is not valid.",
        }
    }
}

/// The one validator: what [`super::HistoryStore::append`] refuses, and what makes a
/// read fail closed.
pub fn validate_entry(entry: &HistoryEntry) -> Result<(), EntryProblem> {
    if !is_entry_id(&entry.id) {
        return Err(EntryProblem::Id);
    }
    if entry.text.trim().is_empty() {
        return Err(EntryProblem::EmptyText);
    }
    if entry.text.len() > MAX_TEXT_BYTES {
        return Err(EntryProblem::TextTooLong);
    }
    match (&entry.tier, &entry.institutions) {
        (EntryTier::Public, Some(_)) => return Err(EntryProblem::Institutions),
        (EntryTier::Private, Some(ids)) => {
            let mut seen = HashSet::new();
            let normalized = ids.iter().all(|id| {
                normalize_institution_id(id).as_deref() == Some(id.as_str())
                    && seen.insert(id.as_str())
            });
            if ids.len() > MAX_ROW_INSTITUTIONS || !normalized {
                return Err(EntryProblem::Institutions);
            }
        }
        _ => {}
    }
    if entry.model.chars().count() > MAX_MODEL_CHARS {
        return Err(EntryProblem::Model);
    }
    if let Some(id) = &entry.session_id {
        if id.is_empty()
            || id.chars().count() > MAX_SESSION_ID_CHARS
            || id.chars().any(char::is_control)
        {
            return Err(EntryProblem::SessionId);
        }
    }
    Ok(())
}

/// `history.jsonl`'s first line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    version: u32,
}

/// The header line, newline included.
pub fn header_line() -> String {
    let header = Header {
        format: HISTORY_FORMAT.to_string(),
        version: HISTORY_VERSION,
    };
    let mut line = serde_json::to_string(&header).expect("the header serializes");
    line.push('\n');
    line
}

/// One entry's line, newline included. JSON escapes every newline inside the text, so a
/// line never holds one.
pub fn entry_line(entry: &HistoryEntry) -> String {
    let mut line = serde_json::to_string(entry).expect("an entry serializes");
    line.push('\n');
    line
}

/// What a read of `history.jsonl` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHistory {
    /// Oldest first, as the file holds them.
    pub entries: Vec<HistoryEntry>,
    /// The bytes up to and including the last newline: the file as a reader counts it,
    /// a torn final line left out. 0 for a file that holds no complete line.
    pub committed_len: u64,
    /// Whether the file held bytes after its last newline.
    pub torn: bool,
}

/// Parse `history.jsonl`. `None` when this version cannot read it: an unknown format or
/// version, a key it does not know, a middle line that does not parse or does not
/// validate, or two entries with one id. A file with no complete line (empty, or only a
/// torn header) is an empty history.
pub fn parse_history(bytes: &[u8]) -> Option<ParsedHistory> {
    let committed_len = bytes
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |at| at + 1);
    let torn = committed_len < bytes.len();
    let committed = std::str::from_utf8(&bytes[..committed_len]).ok()?;
    let mut lines = committed.split_terminator('\n');
    let Some(first) = lines.next() else {
        return Some(ParsedHistory {
            entries: Vec::new(),
            committed_len: 0,
            torn,
        });
    };
    let header: Header = serde_json::from_str(first).ok()?;
    if header.format != HISTORY_FORMAT || header.version != HISTORY_VERSION {
        return None;
    }
    let mut entries = Vec::new();
    let mut ids = HashSet::new();
    for line in lines {
        let entry: HistoryEntry = serde_json::from_str(line).ok()?;
        validate_entry(&entry).ok()?;
        if !ids.insert(entry.id.clone()) {
            return None;
        }
        entries.push(entry);
    }
    Some(ParsedHistory {
        entries,
        committed_len: committed_len as u64,
        torn,
    })
}

/// The person's two switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySettings {
    /// Whether new dictations are recorded. On by default.
    pub keep_history: bool,
    /// Whether a new dictation may be public when nothing about it says private. Off by
    /// default.
    pub public_by_default: bool,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            keep_history: true,
            public_by_default: false,
        }
    }
}

impl HistorySettings {
    /// What an unreadable `settings.json` reads as: the safe values, nothing recorded and
    /// nothing public.
    pub const UNREADABLE: Self = Self {
        keep_history: false,
        public_by_default: false,
    };
}

/// A settings write: each field set is changed, each left `None` is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsChange {
    pub keep_history: Option<bool>,
    pub public_by_default: Option<bool>,
}

/// `settings.json` on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    format: String,
    version: u32,
    keep_history: bool,
    public_by_default: bool,
}

/// Parse `settings.json`; `None` when this version cannot read it.
pub fn parse_settings(bytes: &[u8]) -> Option<HistorySettings> {
    let file: SettingsFile = serde_json::from_slice(bytes).ok()?;
    (file.format == SETTINGS_FORMAT && file.version == SETTINGS_VERSION).then_some(
        HistorySettings {
            keep_history: file.keep_history,
            public_by_default: file.public_by_default,
        },
    )
}

/// `settings.json`'s bytes for `settings`.
pub fn settings_bytes(settings: HistorySettings) -> Vec<u8> {
    let file = SettingsFile {
        format: SETTINGS_FORMAT.to_string(),
        version: SETTINGS_VERSION,
        keep_history: settings.keep_history,
        public_by_default: settings.public_by_default,
    };
    let mut bytes = serde_json::to_vec_pretty(&file).expect("the settings serialize");
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str, tier: EntryTier) -> HistoryEntry {
        HistoryEntry {
            id: new_entry_id(),
            created_at: DateTime::from_timestamp(1_795_707_552, 0).unwrap(),
            text: text.to_string(),
            tier,
            institutions: None,
            surface: Surface::Composer,
            duration_ms: 14_200,
            model: "whisper-large-v3-turbo-q5_0".to_string(),
            private_terms: false,
            session_id: Some("20261126_3".to_string()),
            session_incarnation: Some(8_812_734_012),
            truncated: None,
        }
    }

    #[test]
    fn an_entry_line_reads_as_the_design_writes_it() {
        let mut e = entry(
            "Query the SPOKE knowledge graph for drugs that target EPAS1.",
            EntryTier::Private,
        );
        e.id = "dch_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60".to_string();
        let line = entry_line(&e);
        assert_eq!(
            line,
            "{\"id\":\"dch_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60\",\"created_at\":\"2026-11-26T15:39:12Z\",\
             \"text\":\"Query the SPOKE knowledge graph for drugs that target EPAS1.\",\"tier\":\"private\",\
             \"surface\":\"composer\",\"duration_ms\":14200,\"model\":\"whisper-large-v3-turbo-q5_0\",\
             \"private_terms\":false,\"session_id\":\"20261126_3\",\"session_incarnation\":8812734012}\n"
        );
        let back: HistoryEntry = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn every_surface_has_its_word() {
        for surface in Surface::ALL {
            let json = serde_json::to_value(surface).unwrap();
            assert_eq!(json, serde_json::Value::String(surface.as_str().into()));
            assert_eq!(Surface::parse(surface.as_str()), Some(surface));
        }
        assert_eq!(Surface::parse("chat"), None);
    }

    #[test]
    fn ids_are_dch_and_a_uuid() {
        let id = new_entry_id();
        assert!(is_entry_id(&id), "{id}");
        assert_ne!(new_entry_id(), id);
        for bad in [
            "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60",
            "dch_6F0B8A52-6A1E-4F7E-9D0B-2A3C1D4E5F60",
            "dch_6f0b8a526a1e4f7e9d0b2a3c1d4e5f60",
            "dch_",
            "6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60",
        ] {
            assert!(!is_entry_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_cut_lands_on_a_character_boundary() {
        let text = "aé€😀".repeat(4000);
        let (cut, was_cut) = cut_text(&text, MAX_TEXT_BYTES);
        assert!(was_cut);
        assert!(cut.len() <= MAX_TEXT_BYTES);
        assert!(cut.len() > MAX_TEXT_BYTES - 4);
        assert!(text.starts_with(cut));
        assert_eq!(cut_text("short", MAX_TEXT_BYTES), ("short", false));
    }

    #[test]
    fn the_validator_holds_the_tier_and_institution_rules() {
        let public = entry("hello", EntryTier::Public);
        assert_eq!(validate_entry(&public), Ok(()));
        let mut with_ids = public.clone();
        with_ids.institutions = Some(vec!["ucsf".into()]);
        assert_eq!(validate_entry(&with_ids), Err(EntryProblem::Institutions));
        let mut private = entry("hello", EntryTier::Private);
        for (ids, ok) in [
            (None, true),
            (Some(vec![]), true),
            (Some(vec!["ucsf".to_string(), "stanford".to_string()]), true),
            (Some(vec!["UCSF".to_string()]), false),
            (Some(vec!["ucsf".to_string(), "ucsf".to_string()]), false),
            (Some((0..9).map(|i| format!("i{i}")).collect()), false),
        ] {
            private.institutions = ids.clone();
            assert_eq!(validate_entry(&private).is_ok(), ok, "{ids:?}");
        }
        let mut blank = entry("  ", EntryTier::Private);
        assert_eq!(validate_entry(&blank), Err(EntryProblem::EmptyText));
        blank.text = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(validate_entry(&blank), Err(EntryProblem::TextTooLong));
    }

    #[test]
    fn the_settings_file_round_trips_and_fails_closed() {
        let settings = HistorySettings {
            keep_history: false,
            public_by_default: true,
        };
        assert_eq!(parse_settings(&settings_bytes(settings)), Some(settings));
        assert!(HistorySettings::default().keep_history);
        assert!(!HistorySettings::default().public_by_default);
        for bad in [
            r#"{"format":"biorouter.dictation.history.settings","version":2,"keep_history":true,"public_by_default":false}"#,
            r#"{"format":"something.else","version":1,"keep_history":true,"public_by_default":false}"#,
            r#"{"format":"biorouter.dictation.history.settings","version":1,"keep_history":true,"public_by_default":false,"retention_days":30}"#,
            r#"{"format":"biorouter.dictation.history.settings","version":1,"keep_history":true}"#,
            "not json",
        ] {
            assert_eq!(parse_settings(bad.as_bytes()), None, "{bad}");
        }
    }

    #[test]
    fn a_file_with_no_complete_line_is_empty_and_a_torn_tail_is_left_out() {
        assert_eq!(parse_history(b"").unwrap().entries, vec![]);
        let torn_header = parse_history(b"{\"format\":\"biorou").unwrap();
        assert!(torn_header.torn && torn_header.entries.is_empty());
        assert_eq!(torn_header.committed_len, 0);
        let one = entry("first", EntryTier::Public);
        let mut bytes = (header_line() + &entry_line(&one)).into_bytes();
        let committed = bytes.len() as u64;
        bytes.extend_from_slice(b"{\"id\":\"dch_");
        let parsed = parse_history(&bytes).unwrap();
        assert_eq!(parsed.entries, vec![one]);
        assert_eq!(parsed.committed_len, committed);
        assert!(parsed.torn);
    }
}
