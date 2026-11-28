//! `/dictation/history/*`: the dictation history over HTTP, for the desktop and `biorouter
//! serve` (DD-96, SD-20).
//!
//! The history is the text of every dictation a person started, kept on the computer that
//! transcribed it, so text that was dictated and then lost can be found again. The store
//! is the leaf crate's (`biorouter_dictation::history`), which decides nothing about
//! privacy; the stream route records into it (`routes::dictation`, once per finished
//! stream a person started); these routes are the person's acts on it. In short:
//!
//! * **Private entries are the person's alone** (P-17). Public entries are listed to every
//!   holder of the daemon secret, private entries only to the person:
//!   `HttpCaller::lists_dictation_history_entry`, which is the user action proof on a
//!   daemon that holds a key and the person in the served page on one that holds none
//!   (SD-20). A program that states a private provider in `X-Caller-Provider` is a model
//!   and gets public entries only, which is stricter than the dictionary on purpose: no
//!   model reads the person's history over HTTP. Omission, never redaction and never a
//!   count: `total` counts what this caller may list, the listing's `ETag` hashes this
//!   caller's own filtered page, and no route reports anything store wide.
//! * **Every `{id}` route sits behind `session_reach::gate_dictation_history_entry`**,
//!   which answers 404 with MSG-HISTORY-NOT-FOUND for an id that names nothing and one the
//!   caller may not list alike, and decides the tier route from the request before the id
//!   is read.
//! * **The keyed and keyless write rules**: on a daemon that holds a key every write needs
//!   the user action proof; on one that holds none a delete or a clear is admitted for the
//!   entries the caller can list, while a tier change and a raised setting are the served
//!   person's, refused to anyone else with the host sentences
//!   (`HttpCaller::authorizes_dictation_history_write`).
//! * **The managed `history: false` key** stops recording, refuses tier changes and raised
//!   settings with the policy's sentence, and leaves listing, delete and clear open, so a
//!   person can still remove what was kept.
//! * **A chat id is shown only for a chat the caller may list that still holds the
//!   incarnation the entry recorded**, so an entry dictated into a private chat and then
//!   made public never hands that chat's id to a public caller, and an id reissued to
//!   another chat never links to it.
//!
//! Nothing here logs a dictation's text or id. There is no route that adds an entry or
//! edits its text: the stream route is the only writer of new entries.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use biorouter::dictation::history::{
    self as core_history, EntryTier, HistoryCursor, HistoryEntry, HistoryError, HistoryStore,
    SettingsChange, Surface, TimeWindow,
};
use biorouter::managed::{Allowance, ManagedPolicy};
use biorouter::privacy::SessionClassification;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};

use crate::routes::dictionary::DictionaryTier;
use crate::routes::session_reach::{self, HistoryRefusal, HistoryWrite, HttpCaller, TargetTier};
use crate::state::AppState;

// ── limits ───────────────────────────────────────────────────────────────────────────

/// The largest JSON body an ordinary history route takes.
pub const MAX_JSON_BYTES: usize = 8 * 1024;
/// The largest body of the bulk delete, which names ids.
pub const MAX_IDS_BODY_BYTES: usize = 512 * 1024;
/// The most ids one bulk delete names: the history's own cap.
pub const MAX_IDS: usize = core_history::store::MAX_ENTRIES;
/// The most entries one listing page holds.
pub const MAX_PAGE: usize = 200;
/// The entries a listing page holds when the request names no `limit`.
pub const DEFAULT_PAGE: usize = 100;
/// The longest search, in characters.
pub const MAX_QUERY_CHARS: usize = 200;

// ── sentences (the catalog in `docs/dictation/surfaces.md`) ──────────────────────────

/// MSG-HISTORY-AGENT: a write without the proof on a daemon that holds a key.
pub const MSG_HISTORY_AGENT: &str = "Changing the dictation history needs the person.";
/// MSG-HISTORY-HOST-TIER as written, with `<verb>` and `<id>` for the page to fill.
pub const MSG_HISTORY_HOST_TIER: &str =
    "Do this on the server: `biorouter dictation history <verb> <id>` (as the service user).";
/// MSG-HISTORY-HOST-SETTINGS as written, with `<option>` for the page to fill.
pub const MSG_HISTORY_HOST_SETTINGS: &str =
    "Do this on the server: `biorouter dictation history settings <option>` (as the service user).";
/// MSG-HISTORY-PRIVATE-ELSEWHERE as written, with `<host>` for this daemon's host.
pub const MSG_HISTORY_PRIVATE_ELSEWHERE: &str =
    "Private dictations are shown only to you in Biorouter on <host>.";
/// MSG-HISTORY-NOT-FOUND, the store's own sentence.
pub use biorouter::dictation::history::store::MSG_HISTORY_NOT_FOUND;

/// MSG-HISTORY-HOST-TIER for a change to `to`: the verb filled, `<id>` left as written, so
/// the refusal names no id.
pub fn msg_history_host_tier(to: EntryTier) -> String {
    let verb = if to.is_private() {
        "privatize"
    } else {
        "publicize"
    };
    MSG_HISTORY_HOST_TIER.replace("<verb>", verb)
}

/// MSG-HISTORY-HOST-SETTINGS naming the option the request asked for.
pub fn msg_history_host_settings(option: &str) -> String {
    MSG_HISTORY_HOST_SETTINGS.replace("<option>", option)
}

/// MSG-HISTORY-PRIVATE-ELSEWHERE naming this daemon's host.
pub fn msg_history_private_elsewhere(host: &str) -> String {
    MSG_HISTORY_PRIVATE_ELSEWHERE.replace("<host>", host)
}

// ── the store ────────────────────────────────────────────────────────────────────────

/// The one history handle this daemon reads and writes, so its parse is cached across
/// requests and streams. The stream route records through it, and every route here and
/// the entry gate read it. Reopened if the local data folder moved (a test's sandbox).
pub fn shared_history() -> Arc<HistoryStore> {
    core_history::open_history()
}

/// Whether `id` could name an entry: `dch_` and a UUID.
pub(crate) fn is_entry_id(id: &str) -> bool {
    core_history::store::is_entry_id(id)
}

fn refusal(status: StatusCode, code: &'static str, message: impl Into<String>) -> HistoryRefusal {
    HistoryRefusal {
        status,
        code,
        message: message.into(),
    }
}

/// MSG-HISTORY-NOT-FOUND: an id that names nothing and one the caller may not list are
/// answered alike (rule 1).
pub(crate) fn not_found() -> HistoryRefusal {
    refusal(StatusCode::NOT_FOUND, "not_found", MSG_HISTORY_NOT_FOUND)
}

/// MSG-HISTORY-FILE: 503, and nothing written.
pub(crate) fn unreadable(path: &FsPath) -> HistoryRefusal {
    refusal(
        StatusCode::SERVICE_UNAVAILABLE,
        "unreadable",
        core_history::store::msg_history_file(path),
    )
}

/// The history could not be reached at all, or its write failed.
pub(crate) fn write_failed() -> HistoryRefusal {
    refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        "write_failed",
        "The dictation history could not be written: the store could not be reached.",
    )
}

pub(crate) fn too_large() -> HistoryRefusal {
    refusal(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "")
}

pub(crate) fn bad_request() -> HistoryRefusal {
    refusal(StatusCode::UNPROCESSABLE_ENTITY, "bad_request", "")
}

/// The managed `history: false` key's refusal of a tier change or a raised setting, with
/// the policy's own sentence (MSG-POLICY); `None` while the key allows the history.
pub(crate) fn policy_refusal() -> Option<HistoryRefusal> {
    let policy = ManagedPolicy::load();
    policy
        .dictation()
        .history
        .reason()
        .map(|reason| refusal(StatusCode::FORBIDDEN, "policy", reason))
}

/// A store error as this module answers it. None of them names a dictation's text.
pub(crate) fn store_refusal(error: HistoryError) -> HistoryRefusal {
    match error {
        HistoryError::Unreadable { path } => unreadable(&path),
        HistoryError::Invalid(problem) => refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid",
            problem.message(),
        ),
        HistoryError::Busy => refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            HistoryError::Busy.to_string(),
        ),
        HistoryError::Io(_) => write_failed(),
    }
}

type Answer<T> = Result<T, HistoryRefusal>;

/// Run a store operation off the async runtime.
async fn on_history<T: Send + 'static>(
    work: impl FnOnce(&HistoryStore) -> Result<T, HistoryError> + Send + 'static,
) -> Answer<T> {
    let store = shared_history();
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|_| write_failed())?
        .map_err(store_refusal)
}

/// A JSON body of at most `limit` bytes, refused with this module's answers.
fn json_body<T: for<'de> Deserialize<'de>>(
    body: Result<Bytes, BytesRejection>,
    limit: usize,
) -> Answer<T> {
    let body = body.map_err(|_| too_large())?;
    if body.len() > limit {
        return Err(too_large());
    }
    serde_json::from_slice(&body).map_err(|_| bad_request())
}

/// A query string, refused with this module's `bad_request` rather than axum's plain text
/// 400, so every refusal of these routes is `{code, message}`.
fn query_of<T>(query: Result<Query<T>, QueryRejection>) -> Answer<T> {
    query.map(|Query(query)| query).map_err(|_| bad_request())
}

// ── answers ──────────────────────────────────────────────────────────────────────────

/// Every refusal of a history route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistoryErrorBody {
    /// `agent`, `host_tier`, `host_settings`, `not_found`, `policy`, `unreadable`, `busy`,
    /// `invalid`, `too_large`, `bad_request` or `write_failed`.
    pub code: String,
    /// The sentence to show. It never names a dictation's text or id.
    pub message: String,
}

/// Where a dictation went: a label only, never read for privacy or bias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationHistorySurface {
    Composer,
    Crew,
    Clipboard,
    OtherApp,
    AppPreview,
    Terminal,
}

impl From<Surface> for DictationHistorySurface {
    fn from(surface: Surface) -> Self {
        match surface {
            Surface::Composer => Self::Composer,
            Surface::Crew => Self::Crew,
            Surface::Clipboard => Self::Clipboard,
            Surface::OtherApp => Self::OtherApp,
            Surface::AppPreview => Self::AppPreview,
            Surface::Terminal => Self::Terminal,
        }
    }
}

/// One entry as a caller is shown it: the stored entry without its chat's incarnation and
/// its institutions, with the model's display name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistoryEntryView {
    pub id: String,
    /// When the transcript was ready, in whole seconds.
    pub created_at: DateTime<Utc>,
    /// The corrected text, at most 32 KiB.
    pub text: String,
    pub tier: DictionaryTier,
    pub surface: DictationHistorySurface,
    pub duration_ms: u64,
    /// The speech model's catalog id.
    pub model: String,
    /// The model's display name, when the catalog knows it.
    pub model_name: Option<String>,
    /// Whether private dictionary terms were eligible for this dictation: only for a
    /// caller that may list private entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_terms: Option<bool>,
    /// The chat the text was dictated into: only while the chat holding that id is the one
    /// the entry recorded and this caller may list it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Set when the text was cut to 32 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// 200 from `GET /dictation/history`: newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistoryList {
    pub entries: Vec<DictationHistoryEntryView>,
    /// The next page's `cursor`; opaque.
    pub next_cursor: Option<String>,
    /// How many entries this caller may list match, over every page. Never a count of
    /// anything withheld.
    pub total: u64,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DictationHistoryQuery {
    /// A case folded substring of the text, at most 200 characters.
    #[serde(default)]
    pub q: Option<String>,
    /// The `next_cursor` of the previous page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// At most 200; 100 when absent.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// The chats a page's entries name, checked once per chat: whether the chat now holding
/// the id is the one the entry recorded, and whether this caller may list it.
async fn linked_chats(
    state: &AppState,
    caller: &HttpCaller,
    entries: &[HistoryEntry],
) -> HashMap<(String, i64), bool> {
    let manager = state.session_manager();
    let mut linked = HashMap::new();
    for entry in entries {
        let (Some(id), Some(incarnation)) = (&entry.session_id, entry.session_incarnation) else {
            continue;
        };
        let key = (id.clone(), incarnation);
        if linked.contains_key(&key) {
            continue;
        }
        let same_chat = manager
            .session_incarnation(id)
            .await
            .ok()
            .flatten()
            .is_some_and(|now| now == incarnation);
        let listed = same_chat
            && match session_reach::target_tier(manager, id).await {
                TargetTier::Public => caller.lists_session(SessionClassification::Public),
                TargetTier::Private => caller.lists_session(SessionClassification::Private),
                TargetTier::Unreadable => false,
            };
        linked.insert(key, listed);
    }
    linked
}

fn entry_view(
    entry: &HistoryEntry,
    caller: &HttpCaller,
    linked: &HashMap<(String, i64), bool>,
) -> DictationHistoryEntryView {
    let session_id = match (&entry.session_id, entry.session_incarnation) {
        (Some(id), Some(incarnation)) => linked
            .get(&(id.clone(), incarnation))
            .copied()
            .unwrap_or(false)
            .then(|| id.clone()),
        _ => None,
    };
    DictationHistoryEntryView {
        id: entry.id.clone(),
        created_at: entry.created_at,
        text: entry.text.clone(),
        tier: entry.tier.into(),
        surface: entry.surface.into(),
        duration_ms: entry.duration_ms,
        model: entry.model.clone(),
        model_name: biorouter::dictation::catalog::CATALOG
            .entry(&entry.model)
            .map(|model| model.display_name.to_string()),
        private_terms: caller
            .lists_dictation_history_entry(EntryTier::Private)
            .then_some(entry.private_terms),
        session_id,
        truncated: entry.truncated,
    }
}

async fn views_of(
    state: &AppState,
    caller: &HttpCaller,
    entries: &[HistoryEntry],
) -> Vec<DictationHistoryEntryView> {
    let linked = linked_chats(state, caller, entries).await;
    entries
        .iter()
        .map(|entry| entry_view(entry, caller, &linked))
        .collect()
}

/// The quoted hex digest a listing's `ETag` is: of the caller's own page, so an entry it
/// may not list never changes what it polls (rule 3).
fn etag_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("\"{}\"", hex::encode(&digest[..16]))
}

// ── listing ──────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/dictation/history",
    params(DictationHistoryQuery),
    responses(
        (status = 200, description = "The entries this caller may list, newest first, with an ETag over this page", body = DictationHistoryList),
        (status = 304, description = "Unchanged since the If-None-Match tag"),
        (status = 422, description = "bad_request: a query this daemon cannot read, a search over 200 characters, or a cursor it did not give", body = DictationHistoryErrorBody),
        (status = 503, description = "unreadable (MSG-HISTORY-FILE)", body = DictationHistoryErrorBody),
    )
)]
pub async fn list_dictation_history(
    State(state): State<Arc<AppState>>,
    query: Result<Query<DictationHistoryQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Answer<Response> {
    let query = query_of(query)?;
    let caller = session_reach::http_caller(&headers).await;
    let search = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string);
    if search
        .as_deref()
        .is_some_and(|q| q.chars().count() > MAX_QUERY_CHARS)
    {
        return Err(bad_request());
    }
    let cursor = match query.cursor.as_deref() {
        None | Some("") => None,
        Some(token) => Some(HistoryCursor::from_token(token).ok_or_else(bad_request)?),
    };
    let limit = query
        .limit
        .map_or(DEFAULT_PAGE, |limit| (limit as usize).clamp(1, MAX_PAGE));
    let page = on_history(move |store| {
        store.search(
            search.as_deref(),
            TimeWindow::all(),
            cursor.as_ref(),
            limit,
            &|entry| caller.lists_dictation_history_entry(entry.tier),
        )
    })
    .await?;
    let list = DictationHistoryList {
        entries: views_of(&state, &caller, &page.entries).await,
        next_cursor: page.next_cursor.as_ref().map(HistoryCursor::to_token),
        total: page.total as u64,
    };
    let bytes = serde_json::to_vec(&list).map_err(|_| write_failed())?;
    let tag = etag_of(&bytes);
    let tag_value = HeaderValue::from_str(&tag).map_err(|_| write_failed())?;
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

// ── deleting ─────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    delete,
    path = "/dictation/history/{id}",
    params(("id" = String, Path, description = "The entry's id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "agent (MSG-HISTORY-AGENT): a daemon that holds a key and no proof", body = DictationHistoryErrorBody),
        (status = 404, description = "not_found (MSG-HISTORY-NOT-FOUND), for a missing id and one this caller may not list alike", body = DictationHistoryErrorBody),
        (status = 503, description = "unreadable (MSG-HISTORY-FILE)", body = DictationHistoryErrorBody),
    )
)]
pub async fn delete_dictation_history_entry(
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Answer<StatusCode> {
    let caller = session_reach::http_caller(&headers).await;
    caller.authorizes_dictation_history_write(&HistoryWrite::Entries, None)?;
    let deleted = on_history(move |store| {
        store.delete(&[id], &|entry| {
            caller.lists_dictation_history_entry(entry.tier)
        })
    })
    .await?;
    if deleted == 0 {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /dictation/history/delete`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct DictationHistoryIdsRequest {
    /// At most 5,000.
    pub ids: Vec<String>,
}

/// 200 from the bulk delete and the clear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistoryDeleted {
    /// How many entries went. An id this caller may not list is skipped as missing.
    pub deleted: u64,
}

#[utoipa::path(
    post,
    path = "/dictation/history/delete",
    request_body = DictationHistoryIdsRequest,
    responses(
        (status = 200, description = "How many of the named entries this caller may list were deleted", body = DictationHistoryDeleted),
        (status = 403, description = "agent (MSG-HISTORY-AGENT)", body = DictationHistoryErrorBody),
        (status = 413, description = "too_large: a body over 512 KiB or more than 5,000 ids", body = DictationHistoryErrorBody),
        (status = 503, description = "unreadable (MSG-HISTORY-FILE)", body = DictationHistoryErrorBody),
    )
)]
pub async fn delete_dictation_history_entries(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictationHistoryDeleted>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictationHistoryIdsRequest = json_body(body, MAX_IDS_BODY_BYTES)?;
    if request.ids.len() > MAX_IDS {
        return Err(too_large());
    }
    caller.authorizes_dictation_history_write(&HistoryWrite::Entries, None)?;
    let ids: Vec<String> = request
        .ids
        .into_iter()
        .filter(|id| is_entry_id(id))
        .collect();
    let deleted = on_history(move |store| {
        store.delete(&ids, &|entry| {
            caller.lists_dictation_history_entry(entry.tier)
        })
    })
    .await?;
    Ok(Json(DictationHistoryDeleted {
        deleted: deleted as u64,
    }))
}

/// `POST /dictation/history/clear`.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DictationHistoryClearRequest {
    /// The newest entry's time the page showed: every listable entry created at or before
    /// it goes, so a dictation recorded while the confirmation was open survives.
    pub through: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/dictation/history/clear",
    request_body = DictationHistoryClearRequest,
    responses(
        (status = 200, description = "How many entries this caller may list, created at or before `through`, were deleted", body = DictationHistoryDeleted),
        (status = 403, description = "agent (MSG-HISTORY-AGENT)", body = DictationHistoryErrorBody),
        (status = 503, description = "unreadable (MSG-HISTORY-FILE)", body = DictationHistoryErrorBody),
    )
)]
pub async fn clear_dictation_history(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictationHistoryDeleted>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictationHistoryClearRequest = json_body(body, MAX_JSON_BYTES)?;
    caller.authorizes_dictation_history_write(&HistoryWrite::Entries, None)?;
    let through = request.through;
    let deleted = on_history(move |store| {
        store.clear_through(through, &|entry| {
            caller.lists_dictation_history_entry(entry.tier)
        })
    })
    .await?;
    Ok(Json(DictationHistoryDeleted {
        deleted: deleted as u64,
    }))
}

// ── the tier ─────────────────────────────────────────────────────────────────────────

/// `POST /dictation/history/{id}/tier`.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DictationHistoryTierRequest {
    pub tier: DictionaryTier,
}

/// 200 from the tier route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistoryEntryAnswer {
    pub entry: DictationHistoryEntryView,
}

#[utoipa::path(
    post,
    path = "/dictation/history/{id}/tier",
    params(("id" = String, Path, description = "The entry's id")),
    request_body = DictationHistoryTierRequest,
    responses(
        (status = 200, description = "The entry with its new tier", body = DictationHistoryEntryAnswer),
        (status = 403, description = "agent, host_tier (MSG-HISTORY-HOST-TIER, decided before the id is read) or policy", body = DictationHistoryErrorBody),
        (status = 404, description = "not_found (MSG-HISTORY-NOT-FOUND)", body = DictationHistoryErrorBody),
        (status = 503, description = "unreadable (MSG-HISTORY-FILE)", body = DictationHistoryErrorBody),
    )
)]
pub async fn change_dictation_history_tier(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictationHistoryEntryAnswer>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictationHistoryTierRequest = json_body(body, MAX_JSON_BYTES)?;
    let to = EntryTier::from(request.tier);
    // The gate asked both already, before the id was read; asked again here so the
    // handler refuses on its own if it is ever mounted without the gate.
    caller.authorizes_dictation_history_write(&HistoryWrite::TierChange { to }, None)?;
    if let Some(refusal) = policy_refusal() {
        return Err(refusal);
    }
    let changed = on_history(move |store| {
        store.set_tier(&id, to, &|entry| {
            caller.lists_dictation_history_entry(entry.tier)
        })
    })
    .await?;
    let Some(entry) = changed else {
        return Err(not_found());
    };
    let entry = views_of(&state, &caller, std::slice::from_ref(&entry))
        .await
        .pop()
        .ok_or_else(write_failed)?;
    Ok(Json(DictationHistoryEntryAnswer { entry }))
}

// ── settings ─────────────────────────────────────────────────────────────────────────

/// What this caller may do, before any click, so no surface offers a doomed control.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistoryAuthority {
    /// Whether private entries are listed to this caller; MSG-HISTORY-PRIVATE-ELSEWHERE
    /// when not, whether or not any exist.
    pub list_private: Allowance,
    /// Delete and clear. The managed `history` key never closes them.
    pub write: Allowance,
    /// MSG-HISTORY-HOST-TIER in its publicize form; the page fills the verb and the id.
    pub tier_change: Allowance,
    /// Turning **Dictation history** or **Save new dictations as public** on.
    pub raise_settings: Allowance,
}

/// 200 from `GET` and `POST /dictation/history/settings`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictationHistorySettings {
    /// **Dictation history**: whether new dictations are kept. On by default; an
    /// unreadable settings file reads as off.
    pub keep_history: bool,
    /// **Save new dictations as public**. Off by default.
    pub public_by_default: bool,
    pub authority: DictationHistoryAuthority,
}

/// `POST /dictation/history/settings`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct DictationHistorySettingsRequest {
    #[serde(default)]
    pub keep_history: Option<bool>,
    #[serde(default)]
    pub public_by_default: Option<bool>,
}

fn allowance_of(answer: Result<(), String>) -> Allowance {
    match answer {
        Ok(()) => Allowance::granted(),
        Err(reason) => Allowance::refused(reason),
    }
}

fn authority_of(caller: &HttpCaller) -> DictationHistoryAuthority {
    let policy = policy_refusal().map(|refusal| refusal.message);
    let ask = |write: HistoryWrite| -> Result<(), String> {
        caller
            .authorizes_dictation_history_write(&write, None)
            .map_err(|refusal| refusal.message)
    };
    let narrowed_by_policy = |write: HistoryWrite| -> Result<(), String> {
        ask(write)?;
        match &policy {
            Some(reason) => Err(reason.clone()),
            None => Ok(()),
        }
    };
    DictationHistoryAuthority {
        list_private: allowance_of(
            if caller.lists_dictation_history_entry(EntryTier::Private) {
                Ok(())
            } else {
                Err(msg_history_private_elsewhere(
                    &biorouter::dictation::service::host(),
                ))
            },
        ),
        write: allowance_of(ask(HistoryWrite::Entries)),
        tier_change: allowance_of(narrowed_by_policy(HistoryWrite::TierChange {
            to: EntryTier::Public,
        })),
        raise_settings: allowance_of(narrowed_by_policy(HistoryWrite::Settings {
            raising: Some("<option>".to_string()),
        })),
    }
}

async fn settings_answer(caller: &HttpCaller) -> Answer<DictationHistorySettings> {
    let store = shared_history();
    let settings = tokio::task::spawn_blocking(move || store.settings())
        .await
        .map_err(|_| write_failed())?;
    Ok(DictationHistorySettings {
        keep_history: settings.keep_history,
        public_by_default: settings.public_by_default,
        authority: authority_of(caller),
    })
}

#[utoipa::path(
    get,
    path = "/dictation/history/settings",
    responses(
        (status = 200, description = "The person's switches and what this caller may do", body = DictationHistorySettings),
    )
)]
pub async fn get_dictation_history_settings(
    headers: HeaderMap,
) -> Answer<Json<DictationHistorySettings>> {
    let caller = session_reach::http_caller(&headers).await;
    Ok(Json(settings_answer(&caller).await?))
}

/// The option a settings request raises over `current`, as the terminal spells it
/// (`--keep on`, `--public-by-default on`), or `None` when it only lowers or names the
/// values already set.
fn raised_option(
    current: core_history::HistorySettings,
    request: &DictationHistorySettingsRequest,
) -> Option<String> {
    if request.keep_history == Some(true) && !current.keep_history {
        return Some("--keep on".to_string());
    }
    if request.public_by_default == Some(true) && !current.public_by_default {
        return Some("--public-by-default on".to_string());
    }
    None
}

#[utoipa::path(
    post,
    path = "/dictation/history/settings",
    request_body = DictationHistorySettingsRequest,
    responses(
        (status = 200, description = "The switches after the change", body = DictationHistorySettings),
        (status = 403, description = "agent, host_settings (MSG-HISTORY-HOST-SETTINGS, a raise on a daemon with no key from anyone but the served person) or policy", body = DictationHistoryErrorBody),
        (status = 503, description = "unreadable: the settings file could not be read, so nothing was changed", body = DictationHistoryErrorBody),
    )
)]
pub async fn change_dictation_history_settings(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Answer<Json<DictationHistorySettings>> {
    let caller = session_reach::http_caller(&headers).await;
    let request: DictationHistorySettingsRequest = json_body(body, MAX_JSON_BYTES)?;
    let current = on_history(|store| store.read_settings()).await?;
    let raising = raised_option(current, &request);
    let raises = raising.is_some();
    caller.authorizes_dictation_history_write(&HistoryWrite::Settings { raising }, None)?;
    if raises {
        if let Some(refusal) = policy_refusal() {
            return Err(refusal);
        }
    }
    // A request admitted only to lower writes only what lowers: a switch the terminal
    // turned off between the read above and the write below is never turned back on by
    // a caller that may not turn it on.
    let lower_only = |value: Option<bool>| value.filter(|on| !*on || raises);
    let change = SettingsChange {
        keep_history: lower_only(request.keep_history),
        public_by_default: lower_only(request.public_by_default),
    };
    on_history(move |store| store.set_settings(change)).await?;
    Ok(Json(settings_answer(&caller).await?))
}

// ── the router ───────────────────────────────────────────────────────────────────────

/// Every route that names an entry. ⚠ Ungated by construction: the gate is applied to
/// what this returns at its one call site in [`routes`], so a route appended here is
/// gated (`route_layer` wraps only the routes present when it is called).
fn entry_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/dictation/history/{id}",
            delete(delete_dictation_history_entry),
        )
        .route(
            "/dictation/history/{id}/tier",
            post(change_dictation_history_tier),
        )
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/dictation/history", get(list_dictation_history))
        .route(
            "/dictation/history/delete",
            post(delete_dictation_history_entries).layer(DefaultBodyLimit::max(MAX_IDS_BODY_BYTES)),
        )
        .route(
            "/dictation/history/clear",
            post(clear_dictation_history).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictation/history/settings",
            get(get_dictation_history_settings)
                .post(change_dictation_history_settings)
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .merge(
            entry_routes()
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
                .route_layer(axum::middleware::from_fn(
                    session_reach::gate_dictation_history_entry,
                )),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The daemon's sentences are the catalog's, word for word
    /// (`docs/dictation/surfaces.md`, the MSG-HISTORY rows).
    #[test]
    fn the_route_sentences_are_the_catalogs() {
        let catalog = std::fs::read_to_string(
            FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/dictation/surfaces.md"),
        )
        .unwrap()
        .replace("\r\n", "\n");
        for (id, text) in [
            ("MSG-HISTORY-HOST-TIER", MSG_HISTORY_HOST_TIER),
            ("MSG-HISTORY-HOST-SETTINGS", MSG_HISTORY_HOST_SETTINGS),
            (
                "MSG-HISTORY-PRIVATE-ELSEWHERE",
                MSG_HISTORY_PRIVATE_ELSEWHERE,
            ),
            ("MSG-HISTORY-NOT-FOUND", MSG_HISTORY_NOT_FOUND),
        ] {
            let prefix = format!("| {id} | ");
            let rest = catalog
                .lines()
                .find_map(|line| line.strip_prefix(prefix.as_str()))
                .unwrap_or_else(|| panic!("{id} has no catalog row"));
            let cell = rest.split(" | ").next().unwrap().trim().replace("\\<", "<");
            assert_eq!(cell, text, "{id} is not the catalog's");
        }
        assert!(
            core_history::store::msg_history_file(FsPath::new("<path>")).starts_with(
                "The dictation history file at <path> could not be read, so nothing was changed."
            )
        );
    }

    #[test]
    fn the_host_sentences_fill_their_verb_and_option_and_name_no_id() {
        assert_eq!(
            msg_history_host_tier(EntryTier::Public),
            "Do this on the server: `biorouter dictation history publicize <id>` (as the service user)."
        );
        assert_eq!(
            msg_history_host_tier(EntryTier::Private),
            "Do this on the server: `biorouter dictation history privatize <id>` (as the service user)."
        );
        assert_eq!(
            msg_history_host_settings("--keep on"),
            "Do this on the server: `biorouter dictation history settings --keep on` (as the service user)."
        );
        assert_eq!(
            msg_history_private_elsewhere("lab-server"),
            "Private dictations are shown only to you in Biorouter on lab-server."
        );
    }

    #[test]
    fn a_raise_is_named_as_the_terminal_spells_it() {
        let off = core_history::HistorySettings {
            keep_history: false,
            public_by_default: false,
        };
        let on = core_history::HistorySettings {
            keep_history: true,
            public_by_default: false,
        };
        let ask = |keep: Option<bool>, public: Option<bool>| DictationHistorySettingsRequest {
            keep_history: keep,
            public_by_default: public,
        };
        assert_eq!(
            raised_option(off, &ask(Some(true), None)).as_deref(),
            Some("--keep on")
        );
        assert_eq!(raised_option(on, &ask(Some(true), None)), None);
        assert_eq!(
            raised_option(on, &ask(None, Some(true))).as_deref(),
            Some("--public-by-default on")
        );
        assert_eq!(raised_option(on, &ask(Some(false), Some(false))), None);
        assert_eq!(raised_option(off, &ask(None, None)), None);
    }

    /// Every route that names an entry sits inside the gated sub router, and no other
    /// route does.
    #[test]
    fn every_route_that_names_an_entry_is_inside_the_gated_router() {
        let source = include_str!("dictation_history.rs");
        let entry = crate::routes::body_of(source, "fn entry_routes()");
        let outer = crate::routes::body_of(source, "pub fn routes(state: Arc<AppState>)");
        for path in [
            "\"/dictation/history/{id}\"",
            "\"/dictation/history/{id}/tier\"",
        ] {
            assert!(entry.contains(path), "{path} is not in entry_routes");
            assert!(!outer.contains(path), "{path} is routed outside the gate");
        }
        assert!(outer.contains("session_reach::gate_dictation_history_entry"));
        assert!(!entry.contains("\"/dictation/history\""));
    }
}
