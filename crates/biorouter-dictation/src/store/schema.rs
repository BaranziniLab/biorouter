//! The store file's schema, v1 (DD-34), snake_case on disk and in the HTTP API.
//!
//! Every type refuses keys it does not know. A newer schema bumps `version`, which an
//! older binary refuses as a whole ([`super::StoreError::Unreadable`]); without the
//! refusal, an older binary that lags the app (a separately installed `biorouter` on
//! the PATH is the usual case) would drop the newer keys on its next write.

use std::fmt;

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

use super::fold::fold_key;

/// `format` of `dictionary.json`.
pub const STORE_FORMAT: &str = "biorouter.dictionary.store";
/// `version` of `dictionary.json`.
pub const STORE_VERSION: u32 = 1;
/// `format` of an export file (DD-43).
pub const EXPORT_FORMAT: &str = "biorouter.dictionary";
/// `version` of an export file.
pub const EXPORT_VERSION: u32 = 1;

/// The entries file inside the store folder.
pub const STORE_FILE: &str = "dictionary.json";
/// The usage counters, apart from the entries so a bump never changes a `rev` (DD-33).
pub const USAGE_FILE: &str = "usage.json";
/// The file every writer locks.
pub const LOCK_FILE: &str = ".lock";

/// `settings.learn_from_chats` in a new store. PR-24 sets it from the M-21 result; until
/// then the chat extractor does not exist and the setting starts off.
pub const LEARN_FROM_CHATS_DEFAULT: bool = false;

/// The current time as the store writes it: whole seconds, so a timestamp reads
/// `2026-11-04T17:02:11Z`.
pub fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(0)
}

/// `dct_` plus a random UUID v4, lowercase and hyphenated. Opaque: never derived from
/// the term, so an id names nothing (oracle rule 2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EntryId(String);

impl EntryId {
    /// A fresh random id.
    pub fn random() -> Self {
        Self(format!("dct_{}", uuid::Uuid::new_v4().hyphenated()))
    }

    /// `Some` only for the canonical shape the store writes.
    pub fn parse(text: &str) -> Option<Self> {
        let uuid = text.strip_prefix("dct_")?;
        let parsed = uuid::Uuid::parse_str(uuid).ok()?;
        (parsed.hyphenated().to_string() == uuid).then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EntryId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or_else(|| format!("{value:?} is not a dictionary entry id"))
    }
}

impl From<EntryId> for String {
    fn from(id: EntryId) -> Self {
        id.0
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the agent's dictionary writes do (O-3). Ordered: raising means a later variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWrites {
    /// Every agent term waits for the person.
    Suggest,
    /// Memory's own protocol: a term the person asked for, or agreed to, lands at once.
    OnRequest,
    /// Every agent term lands at once, with **Keep** and **Undo**.
    Add,
}

impl AgentWrites {
    pub const ALL: [Self; 3] = [Self::Suggest, Self::OnRequest, Self::Add];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suggest => "suggest",
            Self::OnRequest => "on_request",
            Self::Add => "add",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == text)
    }
}

/// Which dictations may use a row. `Private > Public`, so a stamp is a maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryTier {
    Public,
    Private,
}

impl EntryTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    pub fn is_private(self) -> bool {
        self == Self::Private
    }
}

/// Only `Active` rows reach recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    Active,
    Suggested,
}

impl EntryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suggested => "suggested",
        }
    }
}

/// Where a row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrySource {
    User,
    Import,
    Agent,
    Auto,
    Correction,
}

impl EntrySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Import => "import",
            Self::Agent => "agent",
            Self::Auto => "auto",
            Self::Correction => "correction",
        }
    }

    /// A person's own act: the Dictionary page, a terminal, an import. Every other
    /// source is a writer acting on its own (the agent, the extractor, the learner), which
    /// tombstones stop and caps turn into suggestions (DD-41).
    pub fn is_person(self) -> bool {
        matches!(self, Self::User | Self::Import)
    }
}

/// What a term is; it sets the default `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermKind {
    GeneSymbol,
    Acronym,
    Name,
    Term,
}

impl TermKind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "gene_symbol" => Some(Self::GeneSymbol),
            "acronym" => Some(Self::Acronym),
            "name" => Some(Self::Name),
            "term" => Some(Self::Term),
            _ => None,
        }
    }
}

/// How far correction may go for a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchPolicy {
    Fuzzy,
    Exact,
    PromptOnly,
}

impl MatchPolicy {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "fuzzy" => Some(Self::Fuzzy),
            "exact" => Some(Self::Exact),
            "prompt_only" => Some(Self::PromptOnly),
            _ => None,
        }
    }

    /// The default for a written form of this kind ("Defaults by kind"): `exact` for gene
    /// symbols and acronyms, for anything of 5 characters or fewer and for any letter and
    /// digit shape; `fuzzy` for a name or term (or no kind) of 6 or more letters with no
    /// digit. The table's third exception, an entry whose letters spell an English word
    /// (`collides`), needs the word list PR-14 adds, and the matcher there never fuzzes
    /// such an entry whatever is stored.
    pub fn default_for(kind: Option<TermKind>, written: &str) -> Self {
        if matches!(kind, Some(TermKind::GeneSymbol | TermKind::Acronym)) {
            return Self::Exact;
        }
        let letters = written.chars().filter(|c| c.is_alphabetic()).count();
        let has_digit = written.chars().any(|c| c.is_numeric());
        if written.chars().count() <= 5 || has_digit || letters < 6 {
            Self::Exact
        } else {
            Self::Fuzzy
        }
    }
}

/// Whether a row reaches the prompt and the decoder bonus. `Always` is "pinned".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiasPolicy {
    #[default]
    Auto,
    Always,
    Never,
}

/// What the model wrote instead of a term; correction uses it once `count` reaches 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Misrecognition {
    pub text: String,
    pub count: u32,
    pub last_seen: DateTime<Utc>,
}

/// A deleted or dismissed term, per (`fold_key`, `tier`), which stops the agent, the
/// extractor and the learner from proposing it again in that tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tombstone {
    pub fold_key: String,
    pub tier: EntryTier,
    pub at: DateTime<Utc>,
}

/// The institutions whose data a private row may hold (O-32, O-35, O-36). On a row,
/// `affiliation: null` is "no institution claims it" (`None`), `{"institutions": []}` is
/// unknown, and 1 to 8 normalized ids name the owners. Public rows carry `null`.
///
/// The leaf crate stores the ids and compares nothing: the core maps them onto its
/// affiliation vocabulary (P-16), whose comparison is the only one in the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowAffiliation {
    pub institutions: Vec<String>,
}

impl RowAffiliation {
    /// `{"institutions": []}`: the value a writer stores when it could not read its chat.
    pub fn unknown() -> Self {
        Self {
            institutions: Vec::new(),
        }
    }
}

/// The most institutions a row may name; a union beyond it is stored as unknown.
pub const MAX_ROW_INSTITUTIONS: usize = 8;
/// The longest institution id a row may hold.
pub const MAX_INSTITUTION_ID_CHARS: usize = 64;

/// One institution id as the core normalizes it (`config::extensions::name_to_key`:
/// whitespace removed, lowercased), or `None` when it is empty, longer than 64
/// characters, or holds a control character. A test in the core pins the two together.
pub fn normalize_institution_id(raw: &str) -> Option<String> {
    let id: String = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let length = id.chars().count();
    (length > 0 && length <= MAX_INSTITUTION_ID_CHARS && !id.chars().any(char::is_control))
        .then_some(id)
}

/// Why a row changed tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceReason {
    PublicizedByUser,
    PrivatizedByUser,
}

/// The person's tier change, recorded on the row it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub reason: ProvenanceReason,
    pub at: DateTime<Utc>,
}

/// One dictionary row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: EntryId,
    /// Per entry revision, the `If-Match` token of an edit. Usage never changes it.
    pub rev: u64,
    /// The spelling dictation writes: NFC, trimmed, 1 to 64 characters, at most 6 words.
    pub written: String,
    pub tier: EntryTier,
    pub status: EntryStatus,
    pub source: EntrySource,
    /// `false` on an `agent` row the person has not kept.
    pub reviewed: bool,
    pub spoken_forms: Vec<String>,
    pub misrecognitions: Vec<Misrecognition>,
    pub kind: Option<TermKind>,
    #[serde(rename = "match")]
    pub match_policy: MatchPolicy,
    pub bias: BiasPolicy,
    pub context_cues: Vec<String>,
    pub note: Option<String>,
    pub reason: Option<String>,
    pub suggested_in: Option<String>,
    pub seen: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provenance: Option<Provenance>,
    /// Private rows only; `None` on every public row.
    pub affiliation: Option<RowAffiliation>,
}

impl Entry {
    /// Derived, never stored.
    pub fn fold_key(&self) -> String {
        fold_key(&self.written)
    }

    /// Bump `rev` and `updated_at`, which every change to a row does.
    pub(crate) fn touch(&mut self, at: DateTime<Utc>) {
        self.rev += 1;
        self.updated_at = at;
    }
}

/// The two stored settings. `suggest_from_chats` is not here: it is whether the
/// `dictionary` capability is on for new chats, kept in the configuration (PR-23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSettings {
    pub learn_from_chats: bool,
    pub agent_writes: AgentWrites,
}

impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            learn_from_chats: LEARN_FROM_CHATS_DEFAULT,
            agent_writes: AgentWrites::OnRequest,
        }
    }
}

/// `dictionary.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreFile {
    pub format: String,
    pub version: u32,
    pub settings: StoreSettings,
    pub entries: Vec<Entry>,
    pub tombstones: Vec<Tombstone>,
}

impl Default for StoreFile {
    /// A new store: no rows, `agent_writes: on_request` (O-3).
    fn default() -> Self {
        Self {
            format: STORE_FORMAT.to_string(),
            version: STORE_VERSION,
            settings: StoreSettings::default(),
            entries: Vec::new(),
            tombstones: Vec::new(),
        }
    }
}

impl StoreFile {
    /// Parse the bytes of `dictionary.json`; `None` for anything this version cannot
    /// read: bad JSON, an unknown key or value, another `format`, a `version` other than
    /// 1, or two rows with one id.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let file: Self = serde_json::from_slice(bytes).ok()?;
        if file.format != STORE_FORMAT || file.version != STORE_VERSION {
            return None;
        }
        let mut ids = std::collections::HashSet::with_capacity(file.entries.len());
        if !file.entries.iter().all(|entry| ids.insert(&entry.id)) {
            return None;
        }
        Some(file)
    }

    /// The bytes written to disk: one row per line, so the file stays readable and a
    /// person can repair it by hand.
    pub fn to_bytes(&self) -> Vec<u8> {
        fn list<T: Serialize>(out: &mut String, items: &[T]) {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str("    ");
                out.push_str(&serde_json::to_string(item).expect("a store value serializes"));
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("  ]");
        }
        let mut out = String::with_capacity(256 + self.entries.len() * 420);
        out.push_str("{\n  \"format\": ");
        out.push_str(&serde_json::to_string(&self.format).expect("a string serializes"));
        out.push_str(",\n  \"version\": ");
        out.push_str(&self.version.to_string());
        out.push_str(",\n  \"settings\": ");
        out.push_str(&serde_json::to_string(&self.settings).expect("settings serialize"));
        out.push_str(",\n  \"entries\": ");
        list(&mut out, &self.entries);
        out.push_str(",\n  \"tombstones\": ");
        list(&mut out, &self.tombstones);
        out.push_str("\n}\n");
        out.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(written: &str) -> Entry {
        let at = "2026-11-01T09:00:00Z".parse().unwrap();
        Entry {
            id: EntryId::parse("dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60").unwrap(),
            rev: 3,
            written: written.into(),
            tier: EntryTier::Public,
            status: EntryStatus::Active,
            source: EntrySource::User,
            reviewed: true,
            spoken_forms: vec!["epas one".into()],
            misrecognitions: vec![Misrecognition {
                text: "a pass one".into(),
                count: 2,
                last_seen: "2026-11-04T17:02:11Z".parse().unwrap(),
            }],
            kind: Some(TermKind::GeneSymbol),
            match_policy: MatchPolicy::Exact,
            bias: BiasPolicy::Auto,
            context_cues: vec![],
            note: None,
            reason: None,
            suggested_in: None,
            seen: 1,
            created_at: at,
            updated_at: "2026-11-04T17:02:11Z".parse().unwrap(),
            provenance: None,
            affiliation: None,
        }
    }

    /// The design document's example file parses, and writing it back parses to the same
    /// value, field for field.
    #[test]
    fn the_documented_store_round_trips() {
        let documented = r#"{
          "format": "biorouter.dictionary.store",
          "version": 1,
          "settings": { "learn_from_chats": false, "agent_writes": "on_request" },
          "entries": [
            {
              "id": "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60",
              "rev": 3, "written": "EPAS1", "tier": "public", "status": "active",
              "source": "user", "reviewed": true, "spoken_forms": ["epas one"],
              "misrecognitions": [{ "text": "a pass one", "count": 2, "last_seen": "2026-11-04T17:02:11Z" }],
              "kind": "gene_symbol", "match": "exact", "bias": "auto", "context_cues": [],
              "note": null, "reason": null, "suggested_in": null, "seen": 1,
              "created_at": "2026-11-01T09:00:00Z", "updated_at": "2026-11-04T17:02:11Z",
              "provenance": null, "affiliation": null
            }
          ],
          "tombstones": [{ "fold_key": "phd2", "tier": "private", "at": "2026-11-03T12:00:00Z" }]
        }"#;
        let file = StoreFile::parse(documented.as_bytes()).expect("the example parses");
        assert_eq!(file.entries, vec![entry("EPAS1")]);
        assert_eq!(file.settings.agent_writes, AgentWrites::OnRequest);
        let written = file.to_bytes();
        assert_eq!(StoreFile::parse(&written), Some(file));
        let text = String::from_utf8(written).unwrap();
        assert!(
            text.contains("\"created_at\":\"2026-11-01T09:00:00Z\""),
            "{text}"
        );
        assert_eq!(
            text.lines().filter(|l| l.contains("\"written\"")).count(),
            1,
            "one row per line: {text}"
        );
    }

    #[test]
    fn what_this_version_cannot_read_is_refused() {
        let good = StoreFile::default().to_bytes();
        assert!(StoreFile::parse(&good).is_some());
        let text = String::from_utf8(good).unwrap();
        for bad in [
            text.replace("\"version\": 1", "\"version\": 2"),
            text.replace(STORE_FORMAT, "something.else"),
            text.replace("\"tombstones\"", "\"tombstones_v2\""),
            text.replace("on_request", "sometimes"),
            text.replace("{\n", "{\n  \"extra\": 1,\n"),
            "{".to_string(),
            String::new(),
        ] {
            assert_eq!(StoreFile::parse(bad.as_bytes()), None, "{bad}");
        }
        let twice = StoreFile {
            entries: vec![entry("EPAS1"), entry("PHD2")],
            ..StoreFile::default()
        };
        assert_eq!(
            StoreFile::parse(&twice.to_bytes()),
            None,
            "two rows, one id"
        );
    }

    #[test]
    fn ids_are_random_and_canonical() {
        let a = EntryId::random();
        let b = EntryId::random();
        assert_ne!(a, b);
        assert_eq!(EntryId::parse(a.as_str()), Some(a.clone()));
        for bad in [
            "dct_6F0B8A52-6A1E-4F7E-9D0B-2A3C1D4E5F60",
            "dct_6f0b8a526a1e4f7e9d0b2a3c1d4e5f60",
            "6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60",
            "dct_",
            "EPAS1",
        ] {
            assert_eq!(EntryId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn agent_writes_are_ordered_from_lowest_to_highest() {
        assert!(AgentWrites::Suggest < AgentWrites::OnRequest);
        assert!(AgentWrites::OnRequest < AgentWrites::Add);
        assert_eq!(
            StoreSettings::default().agent_writes,
            AgentWrites::OnRequest
        );
        for mode in AgentWrites::ALL {
            assert_eq!(AgentWrites::parse(mode.as_str()), Some(mode));
        }
        assert!(EntryTier::Private > EntryTier::Public);
    }

    #[test]
    fn match_defaults_by_kind() {
        use MatchPolicy::*;
        for (kind, written, want) in [
            (Some(TermKind::GeneSymbol), "Biorouter", Exact),
            (Some(TermKind::Acronym), "Biorouter", Exact),
            (None, "Biorouter", Fuzzy),
            (Some(TermKind::Name), "ventilin", Fuzzy),
            (Some(TermKind::Term), "klenl", Exact),
            (None, "UCSF", Exact),
            (None, "EPAS1", Exact),
            (None, "HIF2alpha", Exact),
            (None, "FLET3", Exact),
        ] {
            assert_eq!(MatchPolicy::default_for(kind, written), want, "{written}");
        }
    }

    #[test]
    fn institution_ids_normalize_like_the_core() {
        assert_eq!(normalize_institution_id(" UCSF "), Some("ucsf".into()));
        assert_eq!(normalize_institution_id("U C S F"), Some("ucsf".into()));
        assert_eq!(normalize_institution_id(""), None);
        assert_eq!(normalize_institution_id("   "), None);
        assert_eq!(normalize_institution_id(&"a".repeat(65)), None);
        assert_eq!(normalize_institution_id("uc\u{7}sf"), None);
    }
}
