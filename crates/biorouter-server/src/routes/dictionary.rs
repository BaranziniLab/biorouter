//! `/dictionary/*`: the personal dictionary over HTTP, for the desktop and `biorouter
//! serve` (PR-16; DD-37, DD-38, DD-40).
//!
//! `docs/dictation/dictionary.md` ("Routes", "Refusals", "Who stamps the tier", "Existence
//! oracle rules") is the contract. In short:
//!
//! * **Listing by standing** (DD-37). Public rows to every holder of the daemon secret,
//!   private rows only to a caller that may list them (`HttpCaller::lists_dictionary_entry`):
//!   omission, never redaction and never a count. The usage figures and `suggested_in` go
//!   only to a caller with private standing (`HttpCaller::dictation_standing`), because
//!   usage on a public row moves when private chats dictate. The listing's `ETag` hashes
//!   the caller's own filtered page, and no route reports a store wide revision.
//! * **The keyed and keyless write rules** (DD-38, O-4). On the desktop's daemon every
//!   write needs the user action proof; on a daemon with no key (`serve`) a write is
//!   admitted for a row the caller can list, stamped with the caller's standing, while
//!   tier changes, raising the settings and a private add without private standing are
//!   refused with the host sentences, judged from the request before any row is read.
//!   Both are `HttpCaller::authorizes_dictionary_write` and
//!   `HttpCaller::dictionary_write_stamp`, in `routes::session_reach`, because the
//!   caller's capability and its listing decision are private to that file.
//! * **Every `{id}` route sits behind `session_reach::gate_dictionary_entry`**, which
//!   answers 404 with MSG-NOT-FOUND for an id that names nothing and one the caller may
//!   not list alike (oracle rule 2).
//! * **A private row's institutions are the server's stamp** (O-32, O-35, O-36): an add
//!   that names the chat it started in (`origin_session_id`, read only after
//!   `session_reach` admits the caller) takes that chat's institutions, one the renderer
//!   could not resolve (`origin_unresolved`) the unknown `[]`, and one with no origin none.
//!   An observation takes its chat's, or `[]` when it names no chat.
//! * **Observations** (DD-58) are stamped through `chat_reach` (Private unless Public, and
//!   Private with no chat), are a silent no op when that stamp is a tier the caller cannot
//!   list or the chat is out of reach, drop secret and PII pairs, count once per
//!   (`record_id`, `from`, `to`), and always answer 204.
//!
//! Nothing here sits on the path from a stop to text: a dictation reads the store when its
//! stream is created (`routes::dictation`), and these routes are the person's acts. No
//! route logs a term.

use std::collections::BTreeSet;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use biorouter::dictation::{bias_set, BiasTarget};
use biorouter::managed::ManagedPolicy;
use biorouter::privacy::affiliation::{self, InstitutionId, RowInstitutions, WriterModel};
use biorouter::privacy::{ModelAffiliation, ProviderTier, SessionClassification};
use biorouter::security::dictionary_store::{self, store};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::transfer::{self, ImportFormat, RowOutcome};
use store::{
    AddOutcome, AgentWrites, BiasPolicy, DictionaryStore, Entry, EntryEdit, EntryId, EntrySource,
    EntryStatus, EntryTier, MatchPolicy, NewTerm, Observation, ObservationKind, ObservationStamp,
    Observed, Reach, RowAffiliation, StoreError, StoreFile, StoreSettings, StoreView, TermKind,
    UsageDelta, UsageScope, UserDictionaryTierChange,
};
use utoipa::{IntoParams, ToSchema};

use crate::routes::session_reach::{
    self, DictionaryRefusal, DictionaryWrite, HttpCaller, TargetTier,
};
use crate::state::AppState;

// ── limits ───────────────────────────────────────────────────────────────────────────

/// The largest JSON body an ordinary dictionary route takes.
pub const MAX_JSON_BYTES: usize = 64 * 1024;
/// The largest body of a route that names ids (bulk delete, keep, accept, dismiss).
pub const MAX_IDS_BODY_BYTES: usize = 512 * 1024;
/// The most ids one bulk request names.
pub const MAX_IDS: usize = 5_000;
/// The largest import, measured on its `content`.
pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
/// The largest import body: the content may grow when escaped in JSON.
pub const MAX_IMPORT_BODY_BYTES: usize = 3 * MAX_IMPORT_BYTES;
/// The most rows one import reads.
pub const MAX_IMPORT_ROWS: usize = 5_000;
/// The most rows one listing page holds.
pub const MAX_PAGE: usize = 500;
/// The most observations one request carries.
pub const MAX_OBSERVATIONS: usize = 16;
/// How long a stamp waits for a live chat's model, as the dictation stream does.
pub const LIVE_MODEL_BOUND: Duration = Duration::from_millis(250);

// ── sentences (the catalog in `docs/dictation/surfaces.md`) ──────────────────────────

/// MSG-DICTIONARY-AGENT: a write without the proof on a daemon that holds a key.
pub const MSG_DICTIONARY_AGENT: &str =
    "Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms.";
/// MSG-DICTIONARY-HOST-TIER as written, with `<verb>` and `<term>` for the page to fill.
pub const MSG_DICTIONARY_HOST_TIER: &str =
    "Do this on the server: `biorouter dictionary <verb> <term>` (as the service user).";
/// MSG-DICTIONARY-HOST-SETTINGS as written, with `<option>` for the page to fill.
pub const MSG_DICTIONARY_HOST_SETTINGS: &str =
    "Do this on the server: `biorouter dictionary settings <option>` (as the service user).";
/// MSG-DICTIONARY-HOST-PRIVATE: one constant sentence, the same whatever the store holds.
pub const MSG_DICTIONARY_HOST_PRIVATE: &str = "Private terms are added on the server: `biorouter dictionary add --private <term>` or `biorouter dictionary import <file>` (as the service user).";

/// MSG-DICTIONARY-HOST-TIER for a change to `to`: the verb filled, `<term>` left as
/// written, so the refusal names no term.
pub fn msg_dictionary_host_tier(to: EntryTier) -> String {
    let verb = if to.is_private() {
        "privatize"
    } else {
        "publicize"
    };
    MSG_DICTIONARY_HOST_TIER.replace("<verb>", verb)
}

/// MSG-DICTIONARY-HOST-SETTINGS naming the option the request asked for.
pub fn msg_dictionary_host_settings(option: &str) -> String {
    MSG_DICTIONARY_HOST_SETTINGS.replace("<option>", option)
}

/// MSG-SAME-LETTERS: an add whose letters fold like a row the caller can list.
pub fn msg_same_letters(written: &str) -> String {
    format!("A term with the same letters exists: {written}")
}

// ── the store ────────────────────────────────────────────────────────────────────────

/// The one store handle this daemon reads, so its parsed view is cached across requests
/// (two `stat` calls while nothing changed). Reopened if the configuration folder moved.
/// Every route here and every dictation stream (`routes::dictation`) reads it, and a write
/// through it leaves its cache holding what it wrote (PERF-1). Public for the latency
/// tests, which read its parse count; no other crate has a use for it.
#[doc(hidden)]
pub fn shared_store() -> Arc<DictionaryStore> {
    static STORE: Mutex<Option<Arc<DictionaryStore>>> = Mutex::new(None);
    let dir = dictionary_store::dictionary_dir();
    let mut held = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    match held.as_ref() {
        Some(store) if store.dir() == dir => Arc::clone(store),
        _ => {
            let store = Arc::new(DictionaryStore::open(dir));
            *held = Some(Arc::clone(&store));
            store
        }
    }
}

/// Whether any route may list a row: never an `auto` row seen once ("Routes").
pub(crate) fn ever_listed(entry: &Entry) -> bool {
    !(entry.source == EntrySource::Auto && entry.seen < 2)
}

/// Whether this caller may list this row: its tier, and never an `auto` row seen once.
/// Every route asks it of the row it acts on, and a write asks it again of the file it
/// holds under the store's lock, because the terminal can move a row to the other tier
/// (keeping its id) between a request's first read and its write.
fn listable(entry: &Entry, caller: &HttpCaller) -> bool {
    caller.lists_dictionary_entry(entry.tier) && ever_listed(entry)
}

/// MSG-DICTIONARY-FILE: 503, and nothing written.
pub(crate) fn unreadable(path: &FsPath) -> DictionaryRefusal {
    DictionaryRefusal {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "unreadable",
        message: store::msg_dictionary_file(path),
    }
}

/// MSG-NOT-FOUND: an id that names nothing and one the caller may not list are answered
/// alike (oracle rule 2).
pub(crate) fn not_found() -> DictionaryRefusal {
    DictionaryRefusal {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: store::MSG_NOT_FOUND.to_string(),
    }
}

/// MSG-DICTIONARY-WRITE for a store that could not be reached at all.
pub(crate) fn write_failed() -> DictionaryRefusal {
    DictionaryRefusal {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "write_failed",
        message: "The dictionary could not be written: the store could not be reached.".to_string(),
    }
}

// ── answers and refusals ─────────────────────────────────────────────────────────────

/// Every refusal of a dictionary route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryErrorBody {
    /// `agent`, `host_tier`, `host_settings`, `host_private`, `not_found`, `policy`,
    /// `unreadable`, `stale`, `cap`, `duplicate`, `too_large`, `bad_request`,
    /// `write_failed`, or a validation code (`too_long`, `url`, …).
    pub code: String,
    /// The sentence to show. It never names a term.
    pub message: String,
    /// `stale` only: the row's revision now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<u64>,
}

/// A refusal with its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryError {
    pub status: StatusCode,
    pub body: DictionaryErrorBody,
}

impl DictionaryError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: DictionaryErrorBody {
                code: code.to_string(),
                message: message.into(),
                rev: None,
            },
        }
    }

    fn bad_request() -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "bad_request", "")
    }

    fn too_large() -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "")
    }

    fn policy(reason: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, "policy", reason)
    }
}

impl From<DictionaryRefusal> for DictionaryError {
    fn from(refusal: DictionaryRefusal) -> Self {
        Self::new(refusal.status, refusal.code, refusal.message)
    }
}

impl From<StoreError> for DictionaryError {
    fn from(error: StoreError) -> Self {
        let message = error.to_string();
        match error {
            StoreError::Unreadable { path } => unreadable(&path).into(),
            StoreError::Invalid(code) => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, code.as_str(), message)
            }
            StoreError::Stale { current_rev } => {
                let mut stale = Self::new(StatusCode::PRECONDITION_FAILED, "stale", message);
                stale.body.rev = Some(current_rev);
                stale
            }
            StoreError::NotFound => not_found().into(),
            StoreError::Cap(_) => Self::new(StatusCode::CONFLICT, "cap", message),
            StoreError::Duplicate => Self::new(StatusCode::CONFLICT, "duplicate", message),
            StoreError::Io(_) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "write_failed", message)
            }
        }
    }
}

impl IntoResponse for DictionaryError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

type Answer<T> = Result<T, DictionaryError>;

/// A JSON body of at most `limit` bytes, refused with this module's answers.
fn json_body<T: for<'de> Deserialize<'de>>(
    body: Result<Bytes, BytesRejection>,
    limit: usize,
) -> Answer<T> {
    let body = body.map_err(|_| DictionaryError::too_large())?;
    if body.len() > limit {
        return Err(DictionaryError::too_large());
    }
    serde_json::from_slice(&body).map_err(|_| DictionaryError::bad_request())
}

/// A query string, refused with this module's `bad_request` rather than axum's plain text
/// 400, so every refusal of these routes is `{code, message}`.
fn query_of<T>(query: Result<Query<T>, QueryRejection>) -> Answer<T> {
    query
        .map(|Query(query)| query)
        .map_err(|_| DictionaryError::bad_request())
}

/// The managed `dictionary: false` key (or `enabled: false`) refuses writes and
/// observations with MSG-POLICY; deletes and the export stay allowed.
fn policy_allows_writes() -> Answer<()> {
    let policy = ManagedPolicy::load();
    match policy.dictation().dictionary.reason() {
        Some(reason) => Err(DictionaryError::policy(reason)),
        None => Ok(()),
    }
}

/// After every write, read the shared store once more off the request, so the next
/// dictation's stream finds the parse warm and never pays for one (PERF-1). The write
/// already left the cache holding what it wrote, so this is two `stat` calls; it parses
/// only when the file changed again since (the terminal wrote it), and a stream that starts
/// during that parse waits for it instead of parsing a second time. Then the dictation
/// cache builds again the sets the last few dictations used, from the new file, so the next
/// dictation finds its set ready too, and one that starts while its set is being built
/// waits for that build rather than building the set a second time beside it. The answer
/// never waits for either.
fn warm_after_write() {
    drop(tokio::task::spawn_blocking(|| {
        if let Ok(view) = shared_store().view() {
            super::dictation::dictionary_cache().prewarm(&view);
        }
    }));
}

/// Run a store operation off the async runtime.
async fn on_store<T: Send + 'static>(
    work: impl FnOnce(&DictionaryStore) -> Result<T, StoreError> + Send + 'static,
) -> Answer<T> {
    let store = shared_store();
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|_| DictionaryError::from(write_failed()))?
        .map_err(DictionaryError::from)
}

/// The core's secret scan, which the store cannot call.
fn is_secret(text: &str) -> bool {
    biorouter::guardrails::secret_output::redact_text(text).is_some()
}

/// A term the secret scan or the PII detector flags: a correction pair that holds one is
/// dropped ("Extraction filters").
fn holds_secret_or_pii(text: &str) -> bool {
    is_secret(text) || biorouter::guardrails::pii::PiiDetector::new().has_pii(text)
}

/// The tiers this caller may list, as the store's warning and import plan consult them.
fn reach_of(caller: &HttpCaller) -> Reach {
    if caller.lists_dictionary_entry(EntryTier::Private) {
        Reach::All
    } else {
        Reach::Public
    }
}

// ── the wire vocabulary ──────────────────────────────────────────────────────────────

/// Which dictations may use a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryTier {
    Public,
    Private,
}

impl From<EntryTier> for DictionaryTier {
    fn from(tier: EntryTier) -> Self {
        match tier {
            EntryTier::Public => Self::Public,
            EntryTier::Private => Self::Private,
        }
    }
}

impl From<DictionaryTier> for EntryTier {
    fn from(tier: DictionaryTier) -> Self {
        match tier {
            DictionaryTier::Public => Self::Public,
            DictionaryTier::Private => Self::Private,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryEntryStatus {
    Active,
    Suggested,
}

impl From<EntryStatus> for DictionaryEntryStatus {
    fn from(status: EntryStatus) -> Self {
        match status {
            EntryStatus::Active => Self::Active,
            EntryStatus::Suggested => Self::Suggested,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryEntrySource {
    User,
    Import,
    Agent,
    Auto,
    Correction,
}

impl From<EntrySource> for DictionaryEntrySource {
    fn from(source: EntrySource) -> Self {
        match source {
            EntrySource::User => Self::User,
            EntrySource::Import => Self::Import,
            EntrySource::Agent => Self::Agent,
            EntrySource::Auto => Self::Auto,
            EntrySource::Correction => Self::Correction,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryTermKind {
    GeneSymbol,
    Acronym,
    Name,
    Term,
}

impl From<TermKind> for DictionaryTermKind {
    fn from(kind: TermKind) -> Self {
        match kind {
            TermKind::GeneSymbol => Self::GeneSymbol,
            TermKind::Acronym => Self::Acronym,
            TermKind::Name => Self::Name,
            TermKind::Term => Self::Term,
        }
    }
}

impl From<DictionaryTermKind> for TermKind {
    fn from(kind: DictionaryTermKind) -> Self {
        match kind {
            DictionaryTermKind::GeneSymbol => Self::GeneSymbol,
            DictionaryTermKind::Acronym => Self::Acronym,
            DictionaryTermKind::Name => Self::Name,
            DictionaryTermKind::Term => Self::Term,
        }
    }
}

/// How far correction may go for a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryMatch {
    Fuzzy,
    Exact,
    PromptOnly,
}

impl From<MatchPolicy> for DictionaryMatch {
    fn from(policy: MatchPolicy) -> Self {
        match policy {
            MatchPolicy::Fuzzy => Self::Fuzzy,
            MatchPolicy::Exact => Self::Exact,
            MatchPolicy::PromptOnly => Self::PromptOnly,
        }
    }
}

impl From<DictionaryMatch> for MatchPolicy {
    fn from(policy: DictionaryMatch) -> Self {
        match policy {
            DictionaryMatch::Fuzzy => Self::Fuzzy,
            DictionaryMatch::Exact => Self::Exact,
            DictionaryMatch::PromptOnly => Self::PromptOnly,
        }
    }
}

/// Whether a row reaches the prompt; `always` is "pinned".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryBias {
    Auto,
    Always,
    Never,
}

impl From<BiasPolicy> for DictionaryBias {
    fn from(bias: BiasPolicy) -> Self {
        match bias {
            BiasPolicy::Auto => Self::Auto,
            BiasPolicy::Always => Self::Always,
            BiasPolicy::Never => Self::Never,
        }
    }
}

impl From<DictionaryBias> for BiasPolicy {
    fn from(bias: DictionaryBias) -> Self {
        match bias {
            DictionaryBias::Auto => Self::Auto,
            DictionaryBias::Always => Self::Always,
            DictionaryBias::Never => Self::Never,
        }
    }
}

/// What the agent's dictionary writes do (O-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryAgentWrites {
    Suggest,
    OnRequest,
    Add,
}

impl From<AgentWrites> for DictionaryAgentWrites {
    fn from(mode: AgentWrites) -> Self {
        match mode {
            AgentWrites::Suggest => Self::Suggest,
            AgentWrites::OnRequest => Self::OnRequest,
            AgentWrites::Add => Self::Add,
        }
    }
}

impl From<DictionaryAgentWrites> for AgentWrites {
    fn from(mode: DictionaryAgentWrites) -> Self {
        match mode {
            DictionaryAgentWrites::Suggest => Self::Suggest,
            DictionaryAgentWrites::OnRequest => Self::OnRequest,
            DictionaryAgentWrites::Add => Self::Add,
        }
    }
}

/// Why a row changed tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryProvenanceReason {
    PublicizedByUser,
    PrivatizedByUser,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryProvenance {
    pub reason: DictionaryProvenanceReason,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryMisrecognition {
    pub text: String,
    pub count: u32,
}

/// One institution a private row names, with its published name (`null` when the
/// registry publishes none).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryInstitution {
    pub id: String,
    pub name: Option<String>,
}

/// A private row's institutions: empty when they are unknown, which only local models
/// clear (O-32, O-35).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryAffiliation {
    pub institutions: Vec<DictionaryInstitution>,
}

/// `EntryView` ("Routes"): the stored row minus `seen`. `affiliation` is non null on
/// private rows only. The usage figures, `suggested_in` and `came_from_a_chat` are present
/// only for a caller with private standing (oracle rule 5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryEntryView {
    pub id: String,
    /// The `If-Match` token of an edit.
    pub rev: u64,
    pub written: String,
    pub tier: DictionaryTier,
    pub status: DictionaryEntryStatus,
    pub source: DictionaryEntrySource,
    pub reviewed: bool,
    pub spoken_forms: Vec<String>,
    pub misrecognitions: Vec<DictionaryMisrecognition>,
    pub kind: Option<DictionaryTermKind>,
    #[serde(rename = "match")]
    pub match_policy: DictionaryMatch,
    pub bias: DictionaryBias,
    pub context_cues: Vec<String>,
    pub note: Option<String>,
    pub reason: Option<String>,
    pub provenance: Option<DictionaryProvenance>,
    pub affiliation: Option<DictionaryAffiliation>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uses: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrected_to: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_in: Option<String>,
    /// Private rows, for a caller with private standing: whether the row shows that it
    /// came from a chat (`Entry::came_from_a_chat`), which picks MSG-PUBLICIZE-WARN over
    /// MSG-PUBLICIZE-WARN-NO-CHAT before **Make public…**.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub came_from_a_chat: Option<bool>,
}

/// The row as this caller may see it. `standing` is private standing: the usage map
/// read is `all`, which moves with private dictation.
fn entry_view(entry: &Entry, view: &StoreView, standing: bool) -> DictionaryEntryView {
    let affiliation = match (&entry.affiliation, entry.tier) {
        (Some(stored), EntryTier::Private) => Some(DictionaryAffiliation {
            institutions: stored
                .institutions
                .iter()
                .map(|id| DictionaryInstitution {
                    id: id.clone(),
                    name: affiliation::institution_display_name(InstitutionId::new(id))
                        .map(str::to_string),
                })
                .collect(),
        }),
        _ => None,
    };
    let usage = standing
        .then(|| view.usage(&entry.id, UsageScope::All))
        .flatten();
    DictionaryEntryView {
        id: entry.id.to_string(),
        rev: entry.rev,
        written: entry.written.clone(),
        tier: entry.tier.into(),
        status: entry.status.into(),
        source: entry.source.into(),
        reviewed: entry.reviewed,
        spoken_forms: entry.spoken_forms.clone(),
        misrecognitions: entry
            .misrecognitions
            .iter()
            .map(|m| DictionaryMisrecognition {
                text: m.text.clone(),
                count: m.count,
            })
            .collect(),
        kind: entry.kind.map(Into::into),
        match_policy: entry.match_policy.into(),
        bias: entry.bias.into(),
        context_cues: entry.context_cues.clone(),
        note: entry.note.clone(),
        reason: entry.reason.clone(),
        provenance: entry.provenance.as_ref().map(|p| DictionaryProvenance {
            reason: match p.reason {
                store::ProvenanceReason::PublicizedByUser => {
                    DictionaryProvenanceReason::PublicizedByUser
                }
                store::ProvenanceReason::PrivatizedByUser => {
                    DictionaryProvenanceReason::PrivatizedByUser
                }
            },
            at: p.at,
        }),
        affiliation,
        created_at: entry.created_at,
        updated_at: entry.updated_at,
        uses: standing.then(|| usage.map_or(0, |u| u.uses)),
        last_used_at: usage.and_then(|u| u.last_used_at),
        corrected_to: standing.then(|| usage.map_or(0, |u| u.corrected_to)),
        suggested_in: standing.then(|| entry.suggested_in.clone()).flatten(),
        came_from_a_chat: (standing && entry.tier.is_private()).then(|| entry.came_from_a_chat()),
    }
}

/// Deserialize a nullable field keeping "absent" and "null" apart, as `routes::knowledge`
/// does: absent `None`, `null` `Some(None)`, a value `Some(Some(v))`.
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

// ── listing ──────────────────────────────────────────────────────────────────────────

/// Which rows a listing holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryStatusFilter {
    Active,
    Suggested,
    #[default]
    All,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DictionaryListQuery {
    /// `active`, `suggested` or `all` (the default).
    #[serde(default)]
    pub status: Option<DictionaryStatusFilter>,
    /// A case folded substring of the written form, the spoken forms and the note.
    #[serde(default)]
    pub q: Option<String>,
    /// The `next_cursor` of the previous page; opaque.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// 200 from `GET /dictionary/entries`: at most 500 rows, newest change first. It carries no
/// count of rows, listed or withheld (oracle rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryEntryList {
    pub entries: Vec<DictionaryEntryView>,
    pub next_cursor: Option<String>,
}

/// A page's place in the order `updated_at` descending, then `id`: the last row's time and
/// id, which the caller already holds.
fn cursor_of(entry: &Entry) -> String {
    format!("{}.{}", entry.updated_at.timestamp(), entry.id)
}

fn parse_cursor(cursor: &str) -> Option<(i64, EntryId)> {
    let (at, id) = cursor.split_once('.')?;
    Some((at.parse().ok()?, EntryId::parse(id)?))
}

/// The quoted hex digest a listing's `ETag` is: of the caller's own page, so a write it
/// may not list never changes what it polls (oracle rule 4).
fn etag_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("\"{}\"", hex::encode(&digest[..16]))
}

#[utoipa::path(
    get,
    path = "/dictionary/entries",
    params(DictionaryListQuery),
    responses(
        (status = 200, description = "The rows this caller may list, with an ETag over this page", body = DictionaryEntryList),
        (status = 304, description = "Unchanged since the If-None-Match tag"),
        (status = 422, description = "bad_request: a query this daemon cannot read, or a cursor it did not give", body = DictionaryErrorBody),
        (status = 503, description = "unreadable (MSG-DICTIONARY-FILE)", body = DictionaryErrorBody),
    )
)]
pub async fn list_dictionary_entries(
    query: Result<Query<DictionaryListQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Answer<Response> {
    let query = query_of(query)?;
    let caller = session_reach::http_caller(&headers).await;
    let standing = caller.dictation_standing().is_private();
    let after = match query.cursor.as_deref() {
        None | Some("") => None,
        Some(cursor) => Some(parse_cursor(cursor).ok_or_else(DictionaryError::bad_request)?),
    };
    let needle = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);
    let status = query.status.unwrap_or_default();
    let view = on_store(|store| store.view()).await?;
    let mut rows: Vec<&Entry> = view
        .entries()
        .iter()
        .filter(|e| listable(e, &caller))
        .filter(|e| match status {
            DictionaryStatusFilter::All => true,
            DictionaryStatusFilter::Active => e.status == EntryStatus::Active,
            DictionaryStatusFilter::Suggested => e.status == EntryStatus::Suggested,
        })
        .filter(|e| {
            needle.as_deref().is_none_or(|needle| {
                std::iter::once(&e.written)
                    .chain(&e.spoken_forms)
                    .chain(e.note.iter())
                    .any(|text| text.to_lowercase().contains(needle))
            })
        })
        .collect();
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
    if let Some((at, id)) = &after {
        rows.retain(|e| {
            let t = e.updated_at.timestamp();
            t < *at || (t == *at && &e.id > id)
        });
    }
    let more = rows.len() > MAX_PAGE;
    rows.truncate(MAX_PAGE);
    let page = DictionaryEntryList {
        next_cursor: more.then(|| rows.last().map(|e| cursor_of(e))).flatten(),
        entries: rows
            .iter()
            .map(|e| entry_view(e, &view, standing))
            .collect(),
    };
    let bytes = serde_json::to_vec(&page).map_err(|_| DictionaryError::from(write_failed()))?;
    let tag = etag_of(&bytes);
    let tag_value =
        HeaderValue::from_str(&tag).map_err(|_| DictionaryError::from(write_failed()))?;
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|presented| {
            presented
                .split(',')
                .any(|t| t.trim().trim_start_matches("W/") == tag)
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            bytes,
        )
            .into_response()
    };
    response.headers_mut().insert(header::ETAG, tag_value);
    Ok(response)
}

// ── adding ───────────────────────────────────────────────────────────────────────────

/// `POST /dictionary/entries`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct DictionaryAddRequest {
    pub written: String,
    #[serde(default)]
    pub spoken_forms: Vec<String>,
    #[serde(default)]
    pub kind: Option<DictionaryTermKind>,
    #[serde(default)]
    pub note: Option<String>,
    /// **Only in private chats.** On a daemon with no key, refused without private
    /// standing (MSG-DICTIONARY-HOST-PRIVATE); there every term is stamped with the
    /// caller's standing.
    #[serde(default)]
    pub private: bool,
    #[serde(default, rename = "match")]
    pub match_policy: Option<DictionaryMatch>,
    #[serde(default)]
    pub bias: Option<DictionaryBias>,
    #[serde(default)]
    pub context_cues: Vec<String>,
    /// The chat the add started in (the context menu's **Add to Biorouter dictionary…**):
    /// a private row takes its institutions (O-36), read after `session_reach` admits
    /// the caller.
    #[serde(default)]
    pub origin_session_id: Option<String>,
    /// The add started in a chat the renderer could not resolve (a shared transcript): a
    /// private row's institutions are unknown, `[]`.
    #[serde(default)]
    pub origin_unresolved: bool,
}

/// 201 for a new row, with MSG-SAME-LETTERS when a row this caller can list folds to the
/// same letters; 200 with `created: false` when the row already exists in the stamp tier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryEntryAnswer {
    pub entry: DictionaryEntryView,
    pub created: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// What a stamp learns of a chat's live agent.
enum LiveModel {
    /// No agent is live: the model is unknown, and the stored tier decides.
    NotLive,
    /// An agent is live and its model was not read: the read did not answer within
    /// [`LIVE_MODEL_BOUND`], a Crew grant scopes the chat, the Crew manager or the provider
    /// read failed. The chat may be bound to a private model before its first turn, so the
    /// stamp fails toward Private ("Who stamps the tier"), and the model is unknown.
    Unanswered,
    /// The bound model's tier and affiliation.
    Read(ProviderTier, Option<ModelAffiliation>),
}

/// A live chat's bound model, read within [`LIVE_MODEL_BOUND`] and never for a chat a Crew
/// grant scopes, as the dictation stream reads it: a stamp must neither wait for a Crew
/// broker nor cost the chat its admission. Unlike the stream, whose safe direction is a
/// public target, a stamp's is a private row, so a live agent whose model was not read is
/// [`LiveModel::Unanswered`], never the same as no agent.
async fn live_model(state: &AppState, session_id: &str) -> LiveModel {
    let Some(agent) = state.peek_agent(session_id).await else {
        return LiveModel::NotLive;
    };
    let read = async {
        let crew = biorouter::crew::manager().ok()?;
        if crew.is_scoped_session(session_id).await {
            return None;
        }
        let provider = agent.provider().await.ok()?;
        Some((provider.tier(), provider.affiliation()))
    };
    match tokio::time::timeout(LIVE_MODEL_BOUND, read).await {
        Ok(Some((tier, affiliation))) => LiveModel::Read(tier, affiliation),
        Ok(None) | Err(_) => LiveModel::Unanswered,
    }
}

/// What a stamp reads of a chat the caller may reach.
struct ChatRead {
    /// `chat_reach` over the stored tier and the live model, as a stamp reads it: Private
    /// unless it answers Public.
    private: bool,
    /// The live model's affiliation; `None` when the model is unknown (no agent is live,
    /// or its model was not read).
    model: Option<Option<ModelAffiliation>>,
    /// `sessions.session_affiliations`: the institutions whose connectors the chat touched.
    touched: anyhow::Result<BTreeSet<InstitutionId>>,
}

impl ChatRead {
    /// The institutions a private row written from this chat records (O-35).
    fn institutions(self) -> Option<RowAffiliation> {
        let model = match self.model {
            Some(affiliation) => WriterModel::Known(affiliation),
            None => WriterModel::Unknown,
        };
        stored_institutions(affiliation::dictionary_row_institutions(
            model,
            self.touched,
        ))
    }
}

fn stored_institutions(row: RowInstitutions) -> Option<RowAffiliation> {
    row.stored_ids()
        .map(|institutions| RowAffiliation { institutions })
}

/// Read a chat for a stamp: `None` when `session_reach` refuses the caller, which reads
/// nothing of the chat first.
async fn read_chat(state: &AppState, session_id: &str, headers: &HeaderMap) -> Option<ChatRead> {
    let manager = state.session_manager();
    session_reach::session_reach(manager, session_id, headers)
        .await
        .ok()?;
    let stored = match session_reach::target_tier(manager, session_id).await {
        TargetTier::Private => Some(SessionClassification::Private),
        TargetTier::Public => Some(SessionClassification::Public),
        TargetTier::Unreadable => None,
    };
    let (live_tier, model) = match live_model(state, session_id).await {
        LiveModel::NotLive => (None, None),
        LiveModel::Unanswered => (Some(ProviderTier::Private), None),
        LiveModel::Read(tier, affiliation) => (Some(tier), Some(affiliation)),
    };
    let reach = biorouter::dictation::chat_reach(stored, live_tier);
    Some(ChatRead {
        private: reach.stamp_private(),
        model,
        touched: manager.session_affiliations(session_id).await,
    })
}

#[utoipa::path(
    post,
    path = "/dictionary/entries",
    request_body = DictionaryAddRequest,
    responses(
        (status = 201, description = "A new row", body = DictionaryEntryAnswer),
        (status = 200, description = "The row already exists in the stamp tier; created is false", body = DictionaryEntryAnswer),
        (status = 403, description = "agent (MSG-DICTIONARY-AGENT), host_private (MSG-DICTIONARY-HOST-PRIVATE) or policy (MSG-POLICY)", body = DictionaryErrorBody),
        (status = 409, description = "cap (MSG-DICTIONARY-CAP)", body = DictionaryErrorBody),
        (status = 422, description = "A validation code", body = DictionaryErrorBody),
        (status = 503, description = "unreadable (MSG-DICTIONARY-FILE)", body = DictionaryErrorBody),
    )
)]
pub async fn add_dictionary_entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Response> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictionaryAddRequest = json_body(body, MAX_JSON_BYTES)?;
    // Judged from the request alone, before any row is read (oracle rule 1).
    caller.authorizes_dictionary_write(
        &DictionaryWrite::Entries {
            private: request.private,
        },
        None,
    )?;
    policy_allows_writes()?;
    if std::iter::once(&request.written)
        .chain(&request.spoken_forms)
        .chain(&request.context_cues)
        .any(|text| is_secret(text))
    {
        return Err(DictionaryError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            store::ValidationCode::SecretShaped.as_str(),
            store::ValidationCode::SecretShaped.message(),
        ));
    }
    let chosen = if request.private {
        EntryTier::Private
    } else {
        EntryTier::Public
    };
    let tier = caller.dictionary_write_stamp(chosen, None);
    // The person's add with no chat carries no institution; one inside a chat takes the
    // chat's; one the renderer could not place carries the unknown value (O-36).
    let affiliation = if !tier.is_private() {
        None
    } else if let Some(origin) = request.origin_session_id.as_deref() {
        match read_chat(&state, origin, &headers).await {
            Some(chat) => chat.institutions(),
            None => Some(RowAffiliation::unknown()),
        }
    } else if request.origin_unresolved {
        Some(RowAffiliation::unknown())
    } else {
        None
    };
    let term = NewTerm {
        spoken_forms: request.spoken_forms,
        kind: request.kind.map(Into::into),
        match_policy: request.match_policy.map(Into::into),
        bias: request.bias.map_or(BiasPolicy::Auto, Into::into),
        context_cues: request.context_cues,
        note: request.note,
        affiliation,
        ..NewTerm::person(request.written, tier)
    };
    let reach = reach_of(&caller);
    let standing = caller.dictation_standing().is_private();
    // The answer's row is the one this write left, read under the store's lock, so a
    // row the terminal moves to the other tier right after is never shown as it is then.
    let (outcome, entry, view) = on_store(move |store| {
        let (outcome, entry) = store.write(|file| {
            let outcome = file.add(term, reach, store::schema::now())?;
            let entry = match &outcome {
                AddOutcome::Created { id, .. } | AddOutcome::Exists { id } => {
                    file.find(id).cloned()
                }
                AddOutcome::Skipped => None,
            };
            Ok((outcome, entry))
        })?;
        Ok((outcome, entry, store.view()?))
    })
    .await?;
    warm_after_write();
    let (created, warning) = match outcome {
        AddOutcome::Created { same_letters, .. } => {
            (true, same_letters.as_deref().map(msg_same_letters))
        }
        AddOutcome::Exists { .. } => (false, None),
        // A person's add passes tombstones and caps refuse; nothing else skips it.
        AddOutcome::Skipped => return Err(DictionaryError::from(write_failed())),
    };
    let entry = entry.ok_or_else(|| DictionaryError::from(write_failed()))?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(DictionaryEntryAnswer {
            entry: entry_view(&entry, &view, standing),
            created,
            warning,
        }),
    )
        .into_response())
}

// ── editing and deleting one row ─────────────────────────────────────────────────────

/// `PATCH /dictionary/entries/{id}`: any of these fields; an absent field stays as it is,
/// and `kind` or `note` sent as `null` clears it.
#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct DictionaryEditRequest {
    #[serde(default)]
    pub written: Option<String>,
    #[serde(default)]
    pub spoken_forms: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present_or_null")]
    #[schema(value_type = Option<DictionaryTermKind>)]
    pub kind: Option<Option<DictionaryTermKind>>,
    #[serde(default, deserialize_with = "present_or_null")]
    #[schema(value_type = Option<String>)]
    pub note: Option<Option<String>>,
    #[serde(default, rename = "match")]
    pub match_policy: Option<DictionaryMatch>,
    #[serde(default)]
    pub bias: Option<DictionaryBias>,
    #[serde(default)]
    pub context_cues: Option<Vec<String>>,
}

/// One row, after a change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryEntryChanged {
    pub entry: DictionaryEntryView,
}

/// The revision an `If-Match` names: `3`, `"3"` or `W/"3"`.
fn if_match(headers: &HeaderMap) -> Answer<Option<u64>> {
    let Some(value) = headers.get(header::IF_MATCH) else {
        return Ok(None);
    };
    let text = value.to_str().map_err(|_| DictionaryError::bad_request())?;
    let text = text.trim().trim_start_matches("W/").trim_matches('"');
    text.parse()
        .map(Some)
        .map_err(|_| DictionaryError::bad_request())
}

fn parse_id(id: &str) -> Answer<EntryId> {
    EntryId::parse(id).ok_or_else(|| not_found().into())
}

/// The tier of a row this caller may list, read for the write's authority and stamp. The
/// write asks [`still_listed`] again under the store's lock.
fn listed_tier(view: &StoreView, id: &EntryId, caller: &HttpCaller) -> Answer<EntryTier> {
    view.find(id)
        .filter(|e| listable(e, caller))
        .map(|e| e.tier)
        .ok_or_else(|| not_found().into())
}

/// The row a write acts on, asked of the file the write holds under the store's lock: a
/// row the terminal moved since the request's first read to a tier this caller may not
/// list answers as a missing one (DD-35, DD-37); one it moved to another tier the caller
/// may list, when the write was judged for `tier`, answers stale, so the page reloads it.
fn still_listed(
    file: &StoreFile,
    id: &EntryId,
    caller: &HttpCaller,
    tier: Option<EntryTier>,
) -> Result<(), StoreError> {
    match file.find(id) {
        Some(e) if !listable(e, caller) => Err(StoreError::NotFound),
        Some(e) if tier.is_some_and(|tier| tier != e.tier) => {
            Err(StoreError::Stale { current_rev: e.rev })
        }
        Some(_) => Ok(()),
        None => Err(StoreError::NotFound),
    }
}

/// An edit whose stamp is private, of a public row: written into the row's private twin,
/// created from the public row when absent with no institution (the person's act with no
/// chat), so the public row stays as it was (DD-35). The id of the row changed.
fn edit_into_private_twin(
    file: &mut StoreFile,
    public: &EntryId,
    expected_rev: Option<u64>,
    change: EntryEdit,
    at: DateTime<Utc>,
) -> Result<EntryId, StoreError> {
    let row = file.find(public).cloned().ok_or(StoreError::NotFound)?;
    if expected_rev.is_some_and(|rev| rev != row.rev) {
        return Err(StoreError::Stale {
            current_rev: row.rev,
        });
    }
    let existing = file
        .rows_written(&row.written, Reach::All)
        .into_iter()
        .find(|e| e.tier.is_private())
        .map(|twin| twin.id.clone());
    let twin = match existing {
        Some(twin) => twin,
        None => {
            let term = NewTerm {
                status: row.status,
                spoken_forms: row.spoken_forms.clone(),
                misrecognitions: row
                    .misrecognitions
                    .iter()
                    .filter(|m| m.count >= 2)
                    .map(|m| m.text.clone())
                    .collect(),
                kind: row.kind,
                match_policy: Some(row.match_policy),
                bias: row.bias,
                context_cues: row.context_cues.clone(),
                note: row.note.clone(),
                ..NewTerm::person(row.written.clone(), EntryTier::Private)
            };
            match file.add(term, Reach::All, at)? {
                AddOutcome::Created { id, .. } | AddOutcome::Exists { id } => id,
                AddOutcome::Skipped => return Err(StoreError::NotFound),
            }
        }
    };
    file.edit(&twin, None, change, at)?;
    Ok(twin)
}

#[utoipa::path(
    patch,
    path = "/dictionary/entries/{id}",
    params(("id" = String, Path, description = "The entry's id")),
    request_body = DictionaryEditRequest,
    responses(
        (status = 200, description = "The row after the edit (on a daemon with no key, a private standing caller's edit of a public row lands in its private twin)", body = DictionaryEntryChanged),
        (status = 403, description = "agent or policy", body = DictionaryErrorBody),
        (status = 404, description = "not_found (MSG-NOT-FOUND)", body = DictionaryErrorBody),
        (status = 409, description = "duplicate (MSG-DICTIONARY-DUPLICATE)", body = DictionaryErrorBody),
        (status = 412, description = "stale (MSG-DICTIONARY-STALE), with the row's rev", body = DictionaryErrorBody),
        (status = 422, description = "A validation code", body = DictionaryErrorBody),
    )
)]
pub async fn edit_dictionary_entry(
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryEntryChanged>> {
    let caller = session_reach::http_caller(&headers).await;
    let id = parse_id(&id)?;
    let request: DictionaryEditRequest = json_body(body, MAX_JSON_BYTES)?;
    let expected_rev = if_match(&headers)?;
    let view = on_store(|store| store.view()).await?;
    let row_tier = listed_tier(&view, &id, &caller)?;
    caller.authorizes_dictionary_write(
        &DictionaryWrite::Entries { private: false },
        Some(row_tier),
    )?;
    policy_allows_writes()?;
    let texts = request
        .written
        .iter()
        .chain(request.spoken_forms.iter().flatten())
        .chain(request.context_cues.iter().flatten());
    if texts.into_iter().any(|text| is_secret(text)) {
        return Err(DictionaryError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            store::ValidationCode::SecretShaped.as_str(),
            store::ValidationCode::SecretShaped.message(),
        ));
    }
    let change = EntryEdit {
        written: request.written,
        spoken_forms: request.spoken_forms,
        kind: request.kind.map(|kind| kind.map(Into::into)),
        note: request.note,
        match_policy: request.match_policy.map(Into::into),
        bias: request.bias.map(Into::into),
        context_cues: request.context_cues,
    };
    let stamp = caller.dictionary_write_stamp(row_tier, Some(row_tier));
    let standing = caller.dictation_standing().is_private();
    let (entry, view) = on_store(move |store| {
        let entry = store.write(|file| {
            still_listed(file, &id, &caller, Some(row_tier))?;
            let at = store::schema::now();
            let changed = if stamp == row_tier {
                file.edit(&id, expected_rev, change, at)?;
                id
            } else {
                edit_into_private_twin(file, &id, expected_rev, change, at)?
            };
            // The answer's row as this write left it, read under the lock.
            file.find(&changed).cloned().ok_or(StoreError::NotFound)
        })?;
        Ok((entry, store.view()?))
    })
    .await?;
    warm_after_write();
    Ok(Json(DictionaryEntryChanged {
        entry: entry_view(&entry, &view, standing),
    }))
}

#[utoipa::path(
    delete,
    path = "/dictionary/entries/{id}",
    params(("id" = String, Path, description = "The entry's id")),
    responses(
        (status = 204, description = "Deleted, with a tombstone for its (fold key, tier); allowed under managed policy"),
        (status = 403, description = "agent", body = DictionaryErrorBody),
        (status = 404, description = "not_found (MSG-NOT-FOUND)", body = DictionaryErrorBody),
    )
)]
pub async fn delete_dictionary_entry(
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Answer<StatusCode> {
    let caller = session_reach::http_caller(&headers).await;
    let id = parse_id(&id)?;
    let view = on_store(|store| store.view()).await?;
    let tier = listed_tier(&view, &id, &caller)?;
    caller.authorizes_dictionary_write(&DictionaryWrite::Entries { private: false }, Some(tier))?;
    on_store(move |store| {
        store.write(|file| {
            still_listed(file, &id, &caller, None)?;
            file.remove(&id, store::schema::now()).map(|_| ())
        })
    })
    .await?;
    warm_after_write();
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /dictionary/entries/{id}/tier`.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DictionaryTierRequest {
    pub tier: DictionaryTier,
}

#[utoipa::path(
    post,
    path = "/dictionary/entries/{id}/tier",
    params(("id" = String, Path, description = "The entry's id")),
    request_body = DictionaryTierRequest,
    responses(
        (status = 200, description = "The row that holds the term afterwards: moved, or merged into its twin", body = DictionaryEntryChanged),
        (status = 403, description = "agent, host_tier (MSG-DICTIONARY-HOST-TIER, decided before the id is read) or policy", body = DictionaryErrorBody),
        (status = 404, description = "not_found (MSG-NOT-FOUND)", body = DictionaryErrorBody),
        (status = 409, description = "cap", body = DictionaryErrorBody),
    )
)]
pub async fn change_dictionary_entry_tier(
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryEntryChanged>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictionaryTierRequest = json_body(body, MAX_JSON_BYTES)?;
    let to: EntryTier = request.tier.into();
    // `gate_dictionary_entry` asked this before it read the id; asked again here, so the
    // proof below is built only after the person's proof is in hand (P-13).
    caller.authorizes_dictionary_write(&DictionaryWrite::TierChange { to }, None)?;
    let proof = UserDictionaryTierChange::from_user_action();
    policy_allows_writes()?;
    let id = parse_id(&id)?;
    let standing = caller.dictation_standing().is_private();
    let (kept, view) = on_store(move |store| {
        let kept = store.write(|file| file.set_tier(&id, to, &proof, store::schema::now()))?;
        Ok((kept, store.view()?))
    })
    .await?;
    warm_after_write();
    let entry = view
        .find(&kept)
        .ok_or_else(|| DictionaryError::from(write_failed()))?;
    Ok(Json(DictionaryEntryChanged {
        entry: entry_view(entry, &view, standing),
    }))
}

// ── the bulk routes ──────────────────────────────────────────────────────────────────

/// A list of entry ids, at most 5,000. An id that names nothing or a row this caller may
/// not list is skipped as missing.
#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct DictionaryIdsRequest {
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryDeleted {
    pub deleted: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryKept {
    pub kept: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryAccepted {
    pub accepted: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryDismissed {
    pub dismissed: u64,
}

/// A bulk request's ids, read and authorized: who asked, and each id it names that parses,
/// once, in the order sent. Which of them name a row the caller may list is asked by
/// [`listed_now`] under the store's lock, never of an earlier read.
struct BulkIds {
    caller: HttpCaller,
    ids: Vec<EntryId>,
}

impl BulkIds {
    /// The ids that name a row this caller may list in `file`, the copy the write holds
    /// under the store's lock: a row the terminal moved out of the caller's reach since the
    /// request arrived is skipped as missing, like an id that names nothing.
    fn listed_now(&self, file: &StoreFile) -> Vec<EntryId> {
        self.ids
            .iter()
            .filter(|id| file.find(id).is_some_and(|e| listable(e, &self.caller)))
            .cloned()
            .collect()
    }
}

/// Read a bulk body and authorize it: the caller's write, and the ids it names.
async fn bulk(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
    writes: bool,
) -> Answer<BulkIds> {
    let caller = session_reach::http_caller(headers).await;
    let request: DictionaryIdsRequest = json_body(body, MAX_IDS_BODY_BYTES)?;
    if request.ids.len() > MAX_IDS {
        return Err(DictionaryError::too_large());
    }
    caller.authorizes_dictionary_write(&DictionaryWrite::Entries { private: false }, None)?;
    if writes {
        policy_allows_writes()?;
    }
    let mut seen = BTreeSet::new();
    let ids = request
        .ids
        .iter()
        .filter_map(|id| EntryId::parse(id))
        .filter(|id| seen.insert(id.clone()))
        .collect();
    Ok(BulkIds { caller, ids })
}

#[utoipa::path(
    post,
    path = "/dictionary/entries/delete",
    request_body = DictionaryIdsRequest,
    responses(
        (status = 200, description = "How many of the listed rows were deleted, each with its tombstone; allowed under managed policy", body = DictionaryDeleted),
        (status = 403, description = "agent", body = DictionaryErrorBody),
    )
)]
pub async fn delete_dictionary_entries(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryDeleted>> {
    let request = bulk(&headers, body, false).await?;
    let deleted = on_store(move |store| {
        store.write(|file| {
            let ids = request.listed_now(file);
            Ok(file
                .remove_confirmed(&ids, |_| true, store::schema::now())
                .len())
        })
    })
    .await?;
    warm_after_write();
    Ok(Json(DictionaryDeleted {
        deleted: deleted as u64,
    }))
}

#[utoipa::path(
    post,
    path = "/dictionary/entries/keep",
    request_body = DictionaryIdsRequest,
    responses(
        (status = 200, description = "How many agent rows the person had not reviewed are now reviewed", body = DictionaryKept),
        (status = 403, description = "agent or policy", body = DictionaryErrorBody),
    )
)]
pub async fn keep_dictionary_entries(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryKept>> {
    let request = bulk(&headers, body, true).await?;
    let kept = on_store(move |store| {
        store.write(|file| {
            let ids = request.listed_now(file);
            let at = store::schema::now();
            let mut kept = 0u64;
            for id in &ids {
                let unreviewed = file.find(id).is_some_and(|e| {
                    e.source == EntrySource::Agent && e.status == EntryStatus::Active && !e.reviewed
                });
                if unreviewed {
                    file.accept(id, at)?;
                    kept += 1;
                }
            }
            Ok(kept)
        })
    })
    .await?;
    warm_after_write();
    Ok(Json(DictionaryKept { kept }))
}

#[utoipa::path(
    post,
    path = "/dictionary/suggestions/accept",
    request_body = DictionaryIdsRequest,
    responses(
        (status = 200, description = "How many suggestions are now active and reviewed", body = DictionaryAccepted),
        (status = 403, description = "agent or policy", body = DictionaryErrorBody),
        (status = 409, description = "cap: nothing was accepted", body = DictionaryErrorBody),
    )
)]
pub async fn accept_dictionary_suggestions(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryAccepted>> {
    let request = bulk(&headers, body, true).await?;
    let accepted = on_store(move |store| {
        store.write(|file| {
            let ids = request.listed_now(file);
            let at = store::schema::now();
            let mut accepted = 0u64;
            for id in &ids {
                if file
                    .find(id)
                    .is_some_and(|e| e.status == EntryStatus::Suggested)
                {
                    // At the cap the whole request fails and nothing is written.
                    file.accept(id, at)?;
                    accepted += 1;
                }
            }
            Ok(accepted)
        })
    })
    .await?;
    warm_after_write();
    Ok(Json(DictionaryAccepted { accepted }))
}

#[utoipa::path(
    post,
    path = "/dictionary/suggestions/dismiss",
    request_body = DictionaryIdsRequest,
    responses(
        (status = 200, description = "How many suggestions were deleted, each with its tombstone; allowed under managed policy", body = DictionaryDismissed),
        (status = 403, description = "agent", body = DictionaryErrorBody),
    )
)]
pub async fn dismiss_dictionary_suggestions(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryDismissed>> {
    let request = bulk(&headers, body, false).await?;
    let dismissed = on_store(move |store| {
        store.write(|file| {
            let ids = request.listed_now(file);
            Ok(file
                .remove_confirmed(
                    &ids,
                    |e| e.status == EntryStatus::Suggested,
                    store::schema::now(),
                )
                .len())
        })
    })
    .await?;
    warm_after_write();
    Ok(Json(DictionaryDismissed {
        dismissed: dismissed as u64,
    }))
}

// ── import and export ────────────────────────────────────────────────────────────────

/// The import formats: one term per line, CSV or TSV with `written,spoken_forms,note`, or
/// the export JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryImportFormat {
    Lines,
    Csv,
    Tsv,
    Json,
}

/// `POST /dictionary/import`: at most 1 MiB of content and 5,000 rows.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DictionaryImportRequest {
    pub format: DictionaryImportFormat,
    pub content: String,
    /// Plan only: the counts the import would give, writing nothing.
    #[serde(default)]
    pub dry_run: bool,
    /// Import the rows not marked private as private. On a daemon with no key, refused
    /// without private standing, as is any row marked `"tier": "private"`.
    #[serde(default)]
    pub private: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryImportInvalid {
    pub line: u64,
    /// A validation code, `cap` or `unreadable`.
    pub code: String,
}

/// What the import did, or would do, over the rows this caller can list (oracle rule 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryImportAnswer {
    pub new: u64,
    pub merged: u64,
    pub duplicate: u64,
    pub invalid: Vec<DictionaryImportInvalid>,
}

#[utoipa::path(
    post,
    path = "/dictionary/import",
    request_body = DictionaryImportRequest,
    responses(
        (status = 200, description = "The import's counts; it merges and never deletes", body = DictionaryImportAnswer),
        (status = 403, description = "agent, host_private (MSG-DICTIONARY-HOST-PRIVATE, before any row is read) or policy", body = DictionaryErrorBody),
        (status = 413, description = "too_large: over 1 MiB or 5,000 rows", body = DictionaryErrorBody),
        (status = 422, description = "not_an_export or unparsable: the file as a whole could not be read", body = DictionaryErrorBody),
    )
)]
pub async fn import_dictionary(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionaryImportAnswer>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictionaryImportRequest = json_body(body, MAX_IMPORT_BODY_BYTES)?;
    if request.content.len() > MAX_IMPORT_BYTES {
        return Err(DictionaryError::too_large());
    }
    let format = match request.format {
        DictionaryImportFormat::Lines => ImportFormat::Lines,
        DictionaryImportFormat::Csv => ImportFormat::Csv,
        DictionaryImportFormat::Tsv => ImportFormat::Tsv,
        DictionaryImportFormat::Json => ImportFormat::Json,
    };
    let rows = transfer::parse_import(format, &request.content).map_err(|error| {
        let code = match error {
            transfer::ImportError::NotAnExport => "not_an_export",
            transfer::ImportError::Unparsable(_) => "unparsable",
        };
        DictionaryError::new(StatusCode::UNPROCESSABLE_ENTITY, code, error.to_string())
    })?;
    if rows.len() > MAX_IMPORT_ROWS {
        return Err(DictionaryError::too_large());
    }
    // A private import is judged from the request and the file alone, before any row of
    // the store is read (oracle rule 1).
    let private = request.private || rows.iter().any(|row| row.private);
    caller.authorizes_dictionary_write(&DictionaryWrite::Entries { private }, None)?;
    policy_allows_writes()?;
    let chosen = if request.private {
        EntryTier::Private
    } else {
        EntryTier::Public
    };
    let default_tier = caller.dictionary_write_stamp(chosen, None);
    let reach = reach_of(&caller);
    let dry_run = request.dry_run;
    let plan = on_store(move |store| {
        if dry_run {
            let mut file = store.view()?.file().clone();
            Ok(file.import(rows, default_tier, &is_secret, store::schema::now()))
        } else {
            store
                .write(|file| Ok(file.import(rows, default_tier, &is_secret, store::schema::now())))
        }
    })
    .await?;
    if !dry_run {
        warm_after_write();
    }
    let count = |which: fn(&RowOutcome) -> bool| plan.count(reach, which) as u64;
    Ok(Json(DictionaryImportAnswer {
        new: count(|o| matches!(o, RowOutcome::New)),
        merged: count(|o| matches!(o, RowOutcome::Merged)),
        duplicate: count(|o| matches!(o, RowOutcome::Duplicate)),
        invalid: plan
            .rows
            .iter()
            .filter(|row| reach.lists(row.tier))
            .filter_map(|row| match &row.outcome {
                RowOutcome::Invalid { code } => Some(DictionaryImportInvalid {
                    line: row.line as u64,
                    code: code.to_string(),
                }),
                _ => None,
            })
            .collect(),
    }))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DictionaryExportQuery {
    /// Include private rows: honoured only with private standing, after the person
    /// confirmed MSG-EXPORT-PRIVATE.
    #[serde(default)]
    pub include_private: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/dictionary/export",
    params(DictionaryExportQuery),
    responses(
        (status = 200, description = "The export JSON (biorouter.dictionary v1) as an attachment: no ids, session ids or usage; allowed under managed policy", body = serde_json::Value),
        (status = 403, description = "agent", body = DictionaryErrorBody),
        (status = 422, description = "bad_request: a query this daemon cannot read", body = DictionaryErrorBody),
    )
)]
pub async fn export_dictionary(
    query: Result<Query<DictionaryExportQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Answer<Response> {
    let query = query_of(query)?;
    let caller = session_reach::http_caller(&headers).await;
    caller.authorizes_dictionary_write(&DictionaryWrite::Export, None)?;
    let reach = if query.include_private == Some(true) && caller.dictation_standing().is_private() {
        Reach::All
    } else {
        Reach::Public
    };
    let view = on_store(|store| store.view()).await?;
    let export = view.file().export(reach, store::schema::now());
    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"biorouter-dictionary.json\""),
        )],
        Json(export),
    )
        .into_response())
}

// ── observations ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryObservationKind {
    Correction,
    Rejected,
}

/// One correction pair the composer's ledger found: never the transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryObservation {
    /// The dictation's stream id.
    pub record_id: String,
    pub kind: DictionaryObservationKind,
    pub from: String,
    pub to: String,
    /// `rejected`: the row that made the change the person undid.
    #[serde(default)]
    pub entry_id: Option<String>,
}

/// `POST /dictionary/observations`: at most 16 pairs, from the chat the send went to.
#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct DictionaryObservationsRequest {
    #[serde(default)]
    pub session_id: Option<String>,
    pub observations: Vec<DictionaryObservation>,
}

/// The stamp an observation takes (`chat_reach`, Private unless Public; Private with no
/// chat) and the model a private row's institutions must admit: `None` when the caller
/// may not reach the named chat, a silent no op.
async fn observation_stamp(
    state: &AppState,
    session_id: Option<&str>,
    headers: &HeaderMap,
) -> Option<(ObservationStamp, Option<ModelAffiliation>)> {
    let Some(session_id) = session_id else {
        // No chat: nothing to read a model or a record from, so a private stamp with the
        // unknown institutions (O-35), unlike a person's add with no chat (O-36).
        return Some((
            ObservationStamp {
                tier: EntryTier::Private,
                affiliation: Some(RowAffiliation::unknown()),
                session_id: None,
            },
            None,
        ));
    };
    let chat = read_chat(state, session_id, headers).await?;
    let model = chat.model.flatten();
    let tier = if chat.private {
        EntryTier::Private
    } else {
        EntryTier::Public
    };
    let affiliation = if chat.private {
        chat.institutions()
    } else {
        None
    };
    Some((
        ObservationStamp {
            tier,
            affiliation,
            session_id: Some(session_id.to_string()),
        },
        model,
    ))
}

#[utoipa::path(
    post,
    path = "/dictionary/observations",
    request_body = DictionaryObservationsRequest,
    responses(
        (status = 204, description = "Always, including a silent no op: a chat out of reach, a stamp this caller cannot list, a pair holding a secret or PII, or one seen before"),
        (status = 403, description = "agent or policy", body = DictionaryErrorBody),
        (status = 413, description = "too_large: more than 16 pairs", body = DictionaryErrorBody),
    )
)]
pub async fn post_dictionary_observations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<StatusCode> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictionaryObservationsRequest = json_body(body, MAX_JSON_BYTES)?;
    if request.observations.len() > MAX_OBSERVATIONS {
        return Err(DictionaryError::too_large());
    }
    caller.authorizes_dictionary_write(&DictionaryWrite::Entries { private: false }, None)?;
    policy_allows_writes()?;
    let Some((stamp, model)) =
        observation_stamp(&state, request.session_id.as_deref(), &headers).await
    else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if !caller.lists_dictionary_entry(stamp.tier) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let observations: Vec<Observation> = request
        .observations
        .into_iter()
        .filter(|o| !holds_secret_or_pii(&o.from) && !holds_secret_or_pii(&o.to))
        .map(|o| Observation {
            record_id: o.record_id,
            kind: match o.kind {
                DictionaryObservationKind::Correction => ObservationKind::Correction,
                DictionaryObservationKind::Rejected => ObservationKind::Rejected,
            },
            from: o.from,
            to: o.to,
            entry_id: o.entry_id.as_deref().and_then(EntryId::parse),
        })
        .collect();
    if observations.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    let standing = caller.dictation_standing();
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let usage_scope = if stamp.tier.is_private() {
        UsageScope::All
    } else {
        UsageScope::Public
    };
    on_store(move |store| {
        // Whether a private row's institutions all admit the chat's model: the bias set's
        // own rule, asked of that one row, so there is no second spelling of it (P-16).
        let target = BiasTarget {
            private: true,
            affiliation: model,
            person_included: false,
        };
        let admits = |entry: &Entry| {
            let row = Entry {
                status: EntryStatus::Active,
                ..entry.clone()
            };
            let alone = StoreFile {
                entries: vec![row],
                ..StoreFile::default()
            };
            !bias_set(&StoreView::detached(alone), &target, standing, enforced).is_empty()
        };
        let corrected = store.write(|file| {
            let at = store::schema::now();
            Ok(observations
                .iter()
                .filter_map(|o| match file.observe(o, &stamp, &admits, at) {
                    Observed::CorrectedTo(id) => Some(id),
                    Observed::Nothing | Observed::Rejected => None,
                })
                .collect::<Vec<_>>())
        })?;
        let deltas: Vec<(EntryId, UsageScope, UsageDelta)> = corrected
            .into_iter()
            .map(|id| {
                (
                    id,
                    usage_scope,
                    UsageDelta {
                        uses: 0,
                        corrected_to: 1,
                        last_used_at: None,
                    },
                )
            })
            .collect();
        // Loss tolerant, as every usage figure is.
        let _ = store.flush_usage(&deltas, store::USAGE_FLUSH_BUDGET);
        Ok(())
    })
    .await?;
    warm_after_write();
    Ok(StatusCode::NO_CONTENT)
}

// ── settings ─────────────────────────────────────────────────────────────────────────

/// Whether one control may be used, and if not, the sentence to show before the click.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryAllowance {
    pub allowed: bool,
    pub reason: Option<String>,
}

impl DictionaryAllowance {
    fn of(answer: Result<(), String>) -> Self {
        match answer {
            Ok(()) => Self {
                allowed: true,
                reason: None,
            },
            Err(reason) => Self {
                allowed: false,
                reason: Some(reason),
            },
        }
    }
}

/// What this caller may do, before any click, so no surface offers a doomed control.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionaryAuthority {
    pub write: DictionaryAllowance,
    pub add_private: DictionaryAllowance,
    /// MSG-DICTIONARY-HOST-TIER in its publicize form; the page fills the verb and the term.
    pub tier_change: DictionaryAllowance,
    /// Raising `agent_writes`, or turning chat learning or suggestions on.
    pub raise_settings: DictionaryAllowance,
}

/// 200 from `GET` and `POST /dictionary/settings`. `suggest_from_chats` joins it with the
/// `dictionary` capability (PR-23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictionarySettings {
    pub learn_from_chats: bool,
    pub agent_writes: DictionaryAgentWrites,
    pub authority: DictionaryAuthority,
}

/// `POST /dictionary/settings`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct DictionarySettingsRequest {
    #[serde(default)]
    pub learn_from_chats: Option<bool>,
    #[serde(default)]
    pub agent_writes: Option<DictionaryAgentWrites>,
}

fn authority_of(caller: &HttpCaller) -> DictionaryAuthority {
    let policy = ManagedPolicy::load();
    let dictation = policy.dictation();
    let blocked_by = |rule: &biorouter::managed::Rule| rule.reason().map(str::to_string);
    let ask = |write: DictionaryWrite| -> Result<(), String> {
        if let Some(reason) = blocked_by(&dictation.dictionary) {
            return Err(reason);
        }
        caller
            .authorizes_dictionary_write(&write, None)
            .map_err(|refusal| refusal.message)
    };
    DictionaryAuthority {
        write: DictionaryAllowance::of(ask(DictionaryWrite::Entries { private: false })),
        add_private: DictionaryAllowance::of(ask(DictionaryWrite::Entries { private: true })),
        tier_change: DictionaryAllowance::of(ask(DictionaryWrite::TierChange {
            to: EntryTier::Public,
        })),
        raise_settings: DictionaryAllowance::of(
            ask(DictionaryWrite::Settings {
                raising: Some("<option>".to_string()),
            })
            .and_then(|()| match blocked_by(&dictation.dictionary_agent_writes) {
                Some(reason) => Err(reason),
                None => Ok(()),
            }),
        ),
    }
}

async fn settings_answer(caller: &HttpCaller) -> Answer<DictionarySettings> {
    let view = on_store(|store| store.view()).await?;
    let settings = view.settings();
    Ok(DictionarySettings {
        learn_from_chats: settings.learn_from_chats,
        agent_writes: settings.agent_writes.into(),
        authority: authority_of(caller),
    })
}

#[utoipa::path(
    get,
    path = "/dictionary/settings",
    responses(
        (status = 200, description = "The stored settings and what this caller may do", body = DictionarySettings),
        (status = 503, description = "unreadable (MSG-DICTIONARY-FILE)", body = DictionaryErrorBody),
    )
)]
pub async fn get_dictionary_settings(headers: HeaderMap) -> Answer<Json<DictionarySettings>> {
    let caller = session_reach::http_caller(&headers).await;
    Ok(Json(settings_answer(&caller).await?))
}

/// The option a settings request raises over `current`, as the terminal spells it
/// (`--agent-writes add`, `--learn-from-chats on`), or `None` when it only lowers or names
/// the values already set.
fn raised_option(
    current: &StoreSettings,
    agent_writes: Option<AgentWrites>,
    learn_from_chats: Option<bool>,
) -> Option<String> {
    match (agent_writes, learn_from_chats) {
        (Some(mode), _) if mode > current.agent_writes => {
            Some(format!("--agent-writes {}", mode.as_str()))
        }
        (_, Some(true)) if !current.learn_from_chats => Some("--learn-from-chats on".to_string()),
        _ => None,
    }
}

/// Whether this caller may make a settings change that raises `raising`: the write rules
/// (MSG-DICTIONARY-HOST-SETTINGS for a raise on a daemon with no key), managed
/// `dictionary`, and for a raise managed `dictionary_agent_writes`.
fn admit_settings(caller: &HttpCaller, raising: Option<String>) -> Answer<()> {
    let raises = raising.is_some();
    caller.authorizes_dictionary_write(&DictionaryWrite::Settings { raising }, None)?;
    policy_allows_writes()?;
    if raises {
        let policy = ManagedPolicy::load();
        if let Some(reason) = policy.dictation().dictionary_agent_writes.reason() {
            return Err(DictionaryError::policy(reason));
        }
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/dictionary/settings",
    request_body = DictionarySettingsRequest,
    responses(
        (status = 200, description = "The settings after the change", body = DictionarySettings),
        (status = 403, description = "agent, host_settings (MSG-DICTIONARY-HOST-SETTINGS, a raise on a daemon with no key) or policy", body = DictionaryErrorBody),
    )
)]
pub async fn change_dictionary_settings(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictionarySettings>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictionarySettingsRequest = json_body(body, MAX_JSON_BYTES)?;
    let agent_writes = request.agent_writes.map(AgentWrites::from);
    let learn_from_chats = request.learn_from_chats;
    // Asked of the cached view to refuse early, and again of the copy the write reads
    // under the store's lock: a setting the person lowered at the terminal in between is
    // judged as it now is, so a request naming the old value is a raise and is refused
    // where a raise is, never written back over the person's change (P-7).
    let current = on_store(|store| store.view()).await?.settings().clone();
    admit_settings(
        &caller,
        raised_option(&current, agent_writes, learn_from_chats),
    )?;
    on_store(move |store| {
        store.write(|file| {
            let raising = raised_option(&file.settings, agent_writes, learn_from_chats);
            if let Err(refusal) = admit_settings(&caller, raising) {
                return Ok(Err(refusal));
            }
            if let Some(mode) = agent_writes {
                file.settings.agent_writes = mode;
            }
            if let Some(learn) = learn_from_chats {
                file.settings.learn_from_chats = learn;
            }
            Ok(Ok(()))
        })
    })
    .await??;
    warm_after_write();
    Ok(Json(settings_answer(&caller).await?))
}

// ── the router ───────────────────────────────────────────────────────────────────────

/// Every route that names an entry. ⚠ Ungated by construction: the gate is applied to
/// what this returns at its one call site in [`routes`], so a route appended here is
/// gated (`route_layer` wraps only the routes present when it is called).
fn entry_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/dictionary/entries/{id}",
            patch(edit_dictionary_entry).delete(delete_dictionary_entry),
        )
        .route(
            "/dictionary/entries/{id}/tier",
            post(change_dictionary_entry_tier),
        )
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/dictionary/entries",
            get(list_dictionary_entries)
                .post(add_dictionary_entry)
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictionary/entries/delete",
            post(delete_dictionary_entries).layer(DefaultBodyLimit::max(MAX_IDS_BODY_BYTES)),
        )
        .route(
            "/dictionary/entries/keep",
            post(keep_dictionary_entries).layer(DefaultBodyLimit::max(MAX_IDS_BODY_BYTES)),
        )
        .route(
            "/dictionary/suggestions/accept",
            post(accept_dictionary_suggestions).layer(DefaultBodyLimit::max(MAX_IDS_BODY_BYTES)),
        )
        .route(
            "/dictionary/suggestions/dismiss",
            post(dismiss_dictionary_suggestions).layer(DefaultBodyLimit::max(MAX_IDS_BODY_BYTES)),
        )
        .route(
            "/dictionary/import",
            post(import_dictionary).layer(DefaultBodyLimit::max(MAX_IMPORT_BODY_BYTES)),
        )
        .route("/dictionary/export", get(export_dictionary))
        .route(
            "/dictionary/observations",
            post(post_dictionary_observations).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictionary/settings",
            get(get_dictionary_settings)
                .post(change_dictionary_settings)
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .merge(
            entry_routes()
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
                .route_layer(axum::middleware::from_fn(
                    session_reach::gate_dictionary_entry,
                )),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentences these routes speak, as `docs/dictation/surfaces.md` writes them.
    #[test]
    fn the_route_sentences_are_the_catalogs() {
        let catalog = std::fs::read_to_string(
            FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/dictation/surfaces.md"),
        )
        .unwrap()
        .replace("\r\n", "\n");
        let row = |id: &str| -> String {
            let prefix = format!("| {id} | ");
            let rest = catalog
                .lines()
                .find_map(|l| l.strip_prefix(prefix.as_str()))
                .unwrap_or_else(|| panic!("{id} has no row"));
            rest.split(" | ").next().unwrap().trim().replace("\\<", "<")
        };
        assert_eq!(MSG_DICTIONARY_AGENT, row("MSG-DICTIONARY-AGENT"));
        assert_eq!(MSG_DICTIONARY_HOST_TIER, row("MSG-DICTIONARY-HOST-TIER"));
        assert_eq!(
            MSG_DICTIONARY_HOST_SETTINGS,
            row("MSG-DICTIONARY-HOST-SETTINGS")
        );
        assert_eq!(
            MSG_DICTIONARY_HOST_PRIVATE,
            row("MSG-DICTIONARY-HOST-PRIVATE")
        );
        assert_eq!(
            msg_same_letters("EPAS1"),
            row("MSG-SAME-LETTERS").replace("<written>", "EPAS1")
        );
        assert_eq!(
            msg_dictionary_host_tier(EntryTier::Private),
            "Do this on the server: `biorouter dictionary privatize <term>` (as the service user)."
        );
        assert_eq!(
            msg_dictionary_host_settings("--agent-writes add"),
            "Do this on the server: `biorouter dictionary settings --agent-writes add` (as the service user)."
        );
        assert_eq!(store::MSG_NOT_FOUND, row("MSG-NOT-FOUND"));
    }

    #[test]
    fn cursors_round_trip_and_refuse_what_this_daemon_did_not_give() {
        let id = EntryId::random();
        let cursor = format!("1790000000.{id}");
        assert_eq!(parse_cursor(&cursor), Some((1_790_000_000, id)));
        for bad in [
            "",
            "x",
            "1.EPAS1",
            "a.dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60",
        ] {
            assert_eq!(parse_cursor(bad), None, "{bad}");
        }
    }

    /// A raise is judged against the settings as stored: the same request raises over a
    /// lowered value and raises nothing over the value it names.
    #[test]
    fn a_raise_is_judged_against_the_stored_settings() {
        let low = StoreSettings {
            agent_writes: AgentWrites::Suggest,
            learn_from_chats: false,
        };
        let high = StoreSettings {
            agent_writes: AgentWrites::Add,
            learn_from_chats: true,
        };
        let add = Some(AgentWrites::Add);
        assert_eq!(
            raised_option(&low, add, None).as_deref(),
            Some("--agent-writes add")
        );
        assert_eq!(raised_option(&high, add, None), None);
        assert_eq!(
            raised_option(&low, None, Some(true)).as_deref(),
            Some("--learn-from-chats on")
        );
        assert_eq!(raised_option(&high, None, Some(true)), None);
        assert_eq!(
            raised_option(&low, Some(AgentWrites::Suggest), Some(true)).as_deref(),
            Some("--learn-from-chats on")
        );
        assert_eq!(
            raised_option(&high, Some(AgentWrites::Suggest), Some(false)),
            None
        );
    }

    #[test]
    fn if_match_reads_a_bare_a_quoted_and_a_weak_revision() {
        for (value, want) in [("3", 3), ("\"3\"", 3), ("W/\"3\"", 3)] {
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_MATCH, HeaderValue::from_static(value));
            assert_eq!(if_match(&headers).unwrap(), Some(want), "{value}");
        }
        assert_eq!(if_match(&HeaderMap::new()).unwrap(), None);
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("soon"));
        assert!(if_match(&headers).is_err());
    }
}
