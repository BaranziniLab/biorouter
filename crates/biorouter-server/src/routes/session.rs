use crate::routes::errors::ErrorResponse;
use crate::routes::workflow_utils::{
    apply_workflow_to_agent, build_workflow_with_parameter_values,
};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{
    extract::Path,
    http::StatusCode,
    routing::{delete, get, put},
    Json, Router,
};
use biorouter::agents::ExtensionConfig;
use biorouter::conversation::message::Message;
use biorouter::privacy::declassify::{
    authenticate_declassification, declassify, DeclassifyOutcome, UserConfirmation,
};
use biorouter::privacy::SessionClassification;
use biorouter::session::extension_data::ExtensionState;
use biorouter::session::session_manager::{
    ActivityWindow, ModelUsageRow, SessionInsights, SidebarCursor, TruncateOutcome,
};
use biorouter::session::{EnabledExtensionsState, Session, SessionSummary, SessionType};
use biorouter::workflow::Workflow;
use biorouter_server::auth::is_user_action;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResponse {
    /// List of available session information objects
    sessions: Vec<Session>,
}

const DEFAULT_SIDEBAR_SESSION_LIMIT: u32 = 10;
const MAX_SIDEBAR_SESSION_LIMIT: u32 = 50;

/// Issue #56 DR-19. What both copy handlers say when the SOURCE chat is private
/// and the request carried no proof it came from the person at the keyboard.
///
/// Deliberately **not** a [`biorouter::privacy::PrivacyRefusal`] variant: every
/// one of those ends on `ASK_THE_USER_TO_SWITCH` ("ask the user to switch this
/// chat to a private model first"), and that is the wrong way out of this one —
/// the chat is already private, and the refused act is a copy rather than a
/// model bind. §14.4's content rule still holds: it names the boundary and
/// nothing about the chat, no id, no title, no working directory.
///
/// It ends by foreclosing the retry for the same reason every refusal in this
/// feature does — a model that reads a refusal as transient loops on it.
const COPY_OF_PRIVATE_NEEDS_USER: &str =
    "This chat is private, and branching it creates a new chat that inherits its private model, \
     so only the person at the keyboard may do it, and this request carried no proof it came \
     from them. Nothing was branched and this chat is unchanged. Do not retry; the same call \
     will be refused again. If this task genuinely needs the branch, stop and ask the user to \
     branch the chat from the chat window.";

/// The substring the RENDERER keys on to tell [`COPY_OF_PRIVATE_NEEDS_USER`]
/// from any other plain-text failure of the same route.
///
/// A substring is all it has: under `throwOnError` the generated client throws
/// the parsed BODY rather than the response, so the 403 never reaches the catch
/// arm, and a 500 from this route carries plain text too. Without the match the
/// hook falls to `err instanceof Error ? err.message : …` — and a thrown string
/// is not an `Error`, so the whole refusal was replaced by "Could not branch
/// this conversation", which names neither the cause nor the way out.
///
/// Deliberately NOT `USER_ACTION_REFUSAL_MARKER`. That one marks the model
/// picker's refusal, whose remedy is "switch this chat's model"; this one's is
/// "branch it from the chat window". One toast answering for both would send the
/// user somewhere that cannot help.
///
/// ⚠ Mirrored verbatim in `ui/desktop/src/utils/userAction.ts`, and pinned by
/// `the_copy_refusal_carries_the_marker_the_renderer_keys_on`. The message above
/// is model-facing prose and gets reworded; the marker is the contract.
///
/// `#[cfg(test)]` because the only Rust readers are tests: production emits the
/// whole message and the substring match happens in the renderer. The contract is
/// no weaker for it — the test is what fails if a reword drops the marker, and it
/// is the reword this guards against, not the const.
///
/// `pub(super)` so a refusal in a SIBLING route can assert it does not carry this
/// marker (issue #56 Task 49's grant route does). A second copy of the string for
/// that purpose would be a second marker in every practical sense: the one that
/// gets reworded and the one that does not.
#[cfg(test)]
pub(super) const COPY_OF_PRIVATE_REFUSAL_MARKER: &str = "only the person at the keyboard may do it";

/// Issue #56 DR-19's **second** read: did the copy that just ran mint a
/// private-capability session for a request that proved no human?
///
/// Both copy handlers gate on the SOURCE's tier before calling into
/// `SessionManager`, which then reads the source again inside the copy. Gate
/// B's turn ratchet can raise the source between those two reads, so a
/// header-less request that passed the gate on a public row can copy a
/// now-private one — and `create_derived_session` carries `provider_name` and
/// `model_config`, so the child is not labelled private, it is *bound* to a
/// private provider with nothing downstream ever seeing a raise. That is
/// exactly the mint DR-19 blocks, arrived at through the back of the gate.
///
/// The window is two database reads wide and takes racing a turn in the source
/// chat, so it is not the likely path. It is closed anyway because the answer
/// costs nothing: the tier the child was born with is already in hand, so the
/// gate can be applied to the thing it is actually about (the capability that
/// now exists) rather than only to the evidence that predicted it.
///
/// The pre-check stays in front of the copy — it is what keeps the ordinary
/// refusal from doing the work of a full conversation copy first — and this is
/// the one that decides.
fn minted_capability_without_proof(child: SessionClassification, had_user_action: bool) -> bool {
    child == SessionClassification::Private && !had_user_action
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SidebarSessionsQuery {
    #[serde(default = "default_sidebar_session_limit")]
    limit: u32,
    /// The previous page's `next_cursor`, passed back unchanged. Absent for the
    /// first page.
    ///
    /// ⚠ **This replaced an `offset`** (adversarial security review 2026-09-12,
    /// HIGH). serde ignores unknown query fields, so a client still sending
    /// `offset=…` is served the first page rather than a 400 — it pages from the
    /// top instead of failing, which is the degradation to prefer for a listing.
    #[serde(default)]
    cursor: Option<String>,
    /// BR-71: include `sub_agent` sessions (grouped under `parent_session_id`).
    #[serde(default)]
    include_subagents: bool,
}

/// What `next_cursor` carries on the wire.
///
/// ⚠ **It is not signed, and it does not need to be.** A cursor names the sort
/// key of a row the caller was just handed, and the page it opens is assembled
/// by the same filter as every other page — so a caller that forges one, or
/// replays someone else's, still sees exactly the rows it may see. What the
/// encoding buys is that the value carries **no position**: there is nothing in
/// it to subtract from the next one, which is the whole defect it replaces.
///
/// Base64 rather than the two fields in the open so that no client starts
/// parsing it and pins a shape this route must then keep.
fn encode_sidebar_cursor(cursor: &SidebarCursor) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(format!("{}\u{1f}{}", cursor.updated_at, cursor.id))
}

/// The inverse of [`encode_sidebar_cursor`]. `None` for anything this route did
/// not mint.
fn decode_sidebar_cursor(encoded: &str) -> Option<SidebarCursor> {
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    let raw = String::from_utf8(raw).ok()?;
    let (updated_at, id) = raw.split_once('\u{1f}')?;
    (!updated_at.is_empty() && !id.is_empty()).then(|| SidebarCursor {
        updated_at: updated_at.to_string(),
        id: id.to_string(),
    })
}

/// Query parameters for `GET /sessions`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListSessionsQuery {
    /// BR-71: include `sub_agent` sessions (grouped under `parent_session_id`).
    #[serde(default)]
    pub include_subagents: bool,
}

fn default_sidebar_session_limit() -> u32 {
    DEFAULT_SIDEBAR_SESSION_LIMIT
}

/// The session types `GET /sessions` lists. BR-71: `sub_agent` is opt-in; the
/// default arm is exactly `list_sessions()`'s own filter, so the default path is
/// behaviour-identical to what the route did before the flag existed.
fn listed_session_types(include_subagents: bool) -> &'static [SessionType] {
    if include_subagents {
        &[
            SessionType::User,
            SessionType::Scheduled,
            SessionType::SubAgent,
        ]
    } else {
        &[SessionType::User, SessionType::Scheduled]
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SidebarSessionListResponse {
    sessions: Vec<SessionSummary>,
    has_more: bool,
    /// Where the next page resumes — pass it back as `cursor`, unchanged, and do
    /// not parse it. `null` when this was the last page.
    // Not a doc comment: utoipa publishes those as the schema's `description`,
    // and the reasoning belongs beside the codec. See `encode_sidebar_cursor`.
    next_cursor: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionNameRequest {
    /// Updated name for the session (max 200 characters)
    name: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionUserWorkflowValuesRequest {
    /// Workflow parameter values entered by the user
    user_workflow_values: HashMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateSessionUserWorkflowValuesResponse {
    workflow: Workflow,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSessionRequest {
    json: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EditType {
    #[serde(alias = "fork")]
    Diverge,
    Edit,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageRequest {
    timestamp: i64,
    #[serde(default = "default_edit_type")]
    edit_type: EditType,
    /// Your view of this session: the durable ids (`Message.id`) of every
    /// message it holds, in any order.
    ///
    /// OPTIONAL, and enforced when you send it. For `edit`, which truncates the
    /// live session, supplying it is how the server checks that you are deleting
    /// the history you actually saw: a stored message your list does not name
    /// landed after you rendered the conversation, and the cut is refused with
    /// 409 `conversation_out_of_date` rather than destroying it. Omit it and the
    /// cut still runs under the turn lock and is still bounded to the rows the
    /// server itself just read — safe, but blind to that wider race. Ignored for
    /// `diverge`, which writes only to a new session and cannot destroy anything.
    ///
    /// An EMPTY list is not the same as omitting the field: it asserts your view
    /// holds nothing, and is checked as such (so a non-empty session refuses it).
    //
    // The same precondition `/reply`'s `conversation_so_far` carries (#51 W5),
    // reduced to what a truncation needs: ids alone prove the client has seen
    // everything stored, and unlike that field this one supplies no content.
    //
    // Optional, and still optional now that #59 has landed. The reply stream
    // DOES publish the ids a turn persisted (`MessagesPersisted`), so a client
    // can now be complete — but only one that consumes that frame, and no
    // shipped client does yet: the desktop claims completeness solely for a
    // session it has just re-read from the store. A REQUIRED field would
    // therefore still 409 every in-place edit in every live chat. See
    // `edit_in_place`.
    #[serde(default)]
    expected_message_ids: Option<Vec<String>>,
}

fn default_edit_type() -> EditType {
    EditType::Diverge
}

/// Ids of messages `stored` holds that `client_view` does not name, in stored
/// order.
///
/// Message ids are durable and server-assigned (BR-45/#41), so a client naming
/// one is proof it has seen it. Anything stored that the view does not name is a
/// message that landed after the client rendered the conversation — and an
/// open-above cut would delete it, after its writer was already told the append
/// succeeded.
///
/// The twin of `reply::unacknowledged_stored_ids`, which answers the same
/// question for `conversation_so_far`; that one compares against whole messages
/// because the client is also supplying content. Keep the two in step.
fn unacknowledged_stored_ids(stored: &[Message], client_view: &[String]) -> Vec<String> {
    let acknowledged: HashSet<&str> = client_view.iter().map(String::as_str).collect();
    stored
        .iter()
        .filter_map(|message| message.id.as_deref())
        .filter(|id| !acknowledged.contains(id))
        .map(str::to_string)
        .collect()
}

/// The 409 a refused truncation answers with, in the shape `/reply` already uses
/// for a refused write-back: `code` names the condition and `missing_message_ids`
/// names exactly what the cut would have destroyed, so the client can re-read the
/// session and retry rather than guess.
fn edit_conflict_response(missing: Vec<String>, stored_message_count: usize) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "type": "Error",
            "error": "This session has messages your view does not contain; nothing was \
                      deleted. Re-read the session and retry.",
            "code": "conversation_out_of_date",
            "missing_message_ids": missing,
            "stored_message_count": stored_message_count,
        })),
    )
        .into_response()
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageResponse {
    session_id: String,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DivergeSessionRequest {
    /// Optional name for the diverged session. When omitted or blank, the
    /// diverged session inherits the original session's name so it stays
    /// recognizable in the session list.
    #[serde(default)]
    name: Option<String>,
    /// Optional anchor: the `created` timestamp (ms) of the assistant message a
    /// per-message Diverge button was clicked on. The branch is trimmed to end
    /// at that answer. When omitted, the branch ends at the most recent
    /// complete assistant answer — an in-flight turn captured mid-generation is
    /// never carried over.
    #[serde(default)]
    truncate_after: Option<i64>,
    /// Optional anchor by durable message id (`Message.id`), the BR-45 divergence
    /// point. Preferred over `truncate_after`: it is unambiguous when two
    /// messages share a whole second and it records the branch's divergence point. It
    /// takes precedence when both are supplied; `truncate_after` stays for
    /// back-compatibility with older clients.
    #[serde(default)]
    truncate_after_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DivergeSessionResponse {
    /// The id of the newly-created diverged session.
    session_id: String,
    /// The working directory of the diverged session (identical to the
    /// original). Surfaced so the client can spawn the new window/backend in
    /// the correct directory.
    working_dir: String,
    /// The diverged session's resolved name (e.g. "Foo (branch 2)").
    name: String,
    /// The id of the session this one was diverged from (its lineage parent).
    diverged_from: Option<String>,
}

const MAX_NAME_LENGTH: usize = 200;

fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

#[utoipa::path(
    get,
    path = "/sessions",
    params(
        ("include_subagents" = Option<bool>, Query, description = "Include sub_agent sessions (grouped under parent_session_id); default false")
    ),
    responses(
        (status = 200, description = "The sessions this caller could open. A private session is \
                                      omitted — never redacted — for a caller that carries neither \
                                      the user-action proof nor a private capability, exactly as \
                                      `GET /sessions/{session_id}` would refuse it", body = SessionListResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListSessionsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<SessionListResponse>, StatusCode> {
    // Issue #56, QA 2026-09-10 M1: this returned every row on the machine —
    // title, working directory, privacy reason — to a caller the singular read
    // refuses. It now returns the rows that read would admit, and nothing else.
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    let mut sessions = state
        .session_manager()
        .list_sessions_by_types(listed_session_types(query.include_subagents))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sessions.retain(|session| caller.lists_session(session.privacy_tier));

    Ok(Json(SessionListResponse { sessions }))
}

#[utoipa::path(
    get,
    path = "/sessions/sidebar",
    params(
        ("limit" = Option<u32>, Query, description = "Session summaries per page (default 10, clamped to 1..=50)"),
        ("cursor" = Option<String>, Query, description = "The previous page's `next_cursor`, passed back unchanged. Omit for the first page; an unrecognised value is answered 400"),
        ("include_subagents" = Option<bool>, Query, description = "Include sub_agent sessions (grouped under parent_session_id); default false")
    ),
    responses(
        (status = 200, description = "Paginated lightweight session summaries for the sidebar, \
                                      holding only the sessions this caller could open (see \
                                      `GET /sessions`). `next_cursor` is an OPAQUE continuation \
                                      token: pass it back as `cursor` and do not parse it. It is \
                                      not a position and not a count — it names the last row this \
                                      page returned, so it says nothing about rows that were \
                                      filtered out", body = SidebarSessionListResponse),
        (status = 400, description = "The `cursor` was not one this route issued"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn list_sidebar_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SidebarSessionsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<SidebarSessionListResponse>, StatusCode> {
    let limit = query.limit.clamp(1, MAX_SIDEBAR_SESSION_LIMIT);
    let caller = crate::routes::session_reach::http_caller(&headers).await;

    // Issue #56, QA 2026-09-10 M1: a caller that may not open a private chat is
    // not shown one here either — the listing is the union of what the singular
    // gate admits, and nothing more.
    //
    // ⚠ **The filter is a SQL predicate, and that is the security property, not
    // an optimisation** (adversarial security review 2026-09-12, HIGH). This
    // route first answered M1 by scanning the unfiltered ordering and dropping
    // private rows in Rust, then resuming from the *position it had reached*.
    // The positions it handed back were positions among the rows it had hidden,
    // so subtracting two of them gave their exact count — and because
    // `updated_at` is stamped on every token written, polling it watched private
    // chats start and finish. Rows the caller may not see now never leave the
    // database, and a page resumes from a keyset of the last row it RETURNED,
    // which is a fact the caller already holds.
    //
    // One path for both callers. The proven caller could still page by offset
    // cheaply, but two shapes of continuation token is how the first one came to
    // mean something different from the other.
    let after = match query.cursor.as_deref() {
        Some(encoded) => match decode_sidebar_cursor(encoded) {
            Some(cursor) => Some(cursor),
            None => return Err(StatusCode::BAD_REQUEST),
        },
        None => None,
    };
    let public_only = !caller.lists_session(SessionClassification::Private);

    let mut rows = state
        .session_manager()
        .list_session_summaries_page(
            limit.saturating_add(1),
            after.as_ref(),
            query.include_subagents,
            false,
            public_only,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last().map(|row| encode_sidebar_cursor(&row.cursor)))
        .flatten();

    Ok(Json(SidebarSessionListResponse {
        sessions: rows.into_iter().map(|row| row.summary).collect(),
        has_more,
        next_cursor,
    }))
}

#[derive(Default, Deserialize)]
struct SessionReadQuery {
    #[serde(default)]
    metadata_only: bool,
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session"),
        ("metadata_only" = Option<bool>, Query, description = "Omit conversation history when only session metadata is needed")
    ),
    responses(
        (status = 200, description = "Session history retrieved successfully", body = Session),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 Task 58 / #47): \
                                      the named chat is private (or absent, and an unproven caller \
                                      is told the same thing for both) and the request carried no \
                                      proof it came from the user (body = plain text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionReadQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Syntax only, and deliberately ahead of the gate: an id this rejects cannot
    // name a session on any machine, so answering it discloses nothing about
    // THIS one. Everything below here is about the store.
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Issue #56 Task 58 / #47. Both metadata and history disclose session content,
    // so authorization must precede either read. `session_id` is a request
    // parameter, not a credential; see `routes::session_reach`.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }
    let Ok(session) = state
        .session_manager()
        .get_session(&session_id, !query.metadata_only)
        .await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    Json(session).into_response()
}
#[utoipa::path(
    get,
    path = "/sessions/insights",
    responses(
        (status = 200, description = "Session insights retrieved successfully", body = SessionInsights),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session_insights(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionInsights>, StatusCode> {
    let insights = state
        .session_manager()
        .get_insights()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(insights))
}

/// Query for `GET /sessions/activity`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActivityQuery {
    /// How many calendar days back to report. Clamped to 1..=371 server-side.
    /// ~155 covers the five months the Home heatmap renders.
    #[serde(default = "default_activity_days")]
    pub days: i64,
}

fn default_activity_days() -> i64 {
    155
}

#[utoipa::path(
    get,
    path = "/sessions/activity",
    params(
        ("days" = Option<i64>, Query, description = "Calendar days to report (default 155, clamped to 1..=371)")
    ),
    responses(
        (status = 200, description = "Per-day usage for the Home heatmap", body = ActivityWindow),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session_activity(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<ActivityWindow>, StatusCode> {
    let activity = state
        .session_manager()
        .get_activity(query.days)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(activity))
}

/// Per-model token breakdown for one session.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionModelUsageResponse {
    /// One row per `(model, provider)` group; a `null` `modelId` is the
    /// "unknown" bucket (turns recorded before model attribution, or providers
    /// that reported no model).
    pub models: Vec<ModelUsageRow>,
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/usage",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Per-model usage for the session", body = SessionModelUsageResponse),
        (status = 400, description = "Invalid session id"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session_usage(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Issue #56, QA 2026-09-10: a named chat's metadata, and a 200/404 that told
    // an unproven caller whether the id existed. The read's own gate, first.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }
    match state
        .session_manager()
        .get_session_model_usage(&session_id)
        .await
    {
        Ok(models) => Json(SessionModelUsageResponse { models }).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/name",
    request_body = UpdateSessionNameRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session name updated successfully"),
        (status = 400, description = "Bad request - Name too long (max 200 characters)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn update_session_name(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<UpdateSessionNameRequest>,
) -> Response {
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Issue #56, QA 2026-09-10 (F0's sweep): renaming a chat is a write into it,
    // and a write may never be cheaper than the read.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }
    let name = request.name.trim();
    if name.is_empty() || name.len() > MAX_NAME_LENGTH {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match state
        .session_manager()
        .update(&session_id)
        .user_provided_name(name.to_string())
        .apply()
        .await
    {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/user_workflow_values",
    request_body = UpdateSessionUserWorkflowValuesRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session user workflow values updated successfully", body = UpdateSessionUserWorkflowValuesResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
// Update session user workflow parameter values
async fn update_session_user_workflow_values(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<UpdateSessionUserWorkflowValuesRequest>,
) -> Response {
    if !is_valid_session_id(&session_id) {
        return ErrorResponse {
            message: "Invalid session ID".to_string(),
            status: StatusCode::BAD_REQUEST,
        }
        .into_response();
    }
    // Issue #56, QA 2026-09-10 (F0's sweep): this rewrites the chat's workflow
    // values and re-applies the workflow to its live agent — a write into the
    // chat — so it asks the read's gate before it touches the row or the agent.
    // The refusal is the read's plain text, not this route's JSON envelope, so
    // a client recognises one boundary by one body.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }
    apply_user_workflow_values(&state, &session_id, request)
        .await
        .into_response()
}

/// The body of [`update_session_user_workflow_values`] once the caller may
/// address the chat.
async fn apply_user_workflow_values(
    state: &Arc<AppState>,
    session_id: &str,
    request: UpdateSessionUserWorkflowValuesRequest,
) -> Result<Json<UpdateSessionUserWorkflowValuesResponse>, ErrorResponse> {
    let session_id = session_id.to_string();
    state
        .session_manager()
        .update(&session_id)
        .user_workflow_values(Some(request.user_workflow_values))
        .apply()
        .await
        .map_err(|err| ErrorResponse {
            message: err.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    let session = state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|err| ErrorResponse {
            message: err.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    let workflow = session.workflow.ok_or_else(|| ErrorResponse {
        message: "Workflow not found".to_string(),
        status: StatusCode::NOT_FOUND,
    })?;

    let user_workflow_values = session.user_workflow_values.unwrap_or_default();
    match build_workflow_with_parameter_values(&workflow, user_workflow_values).await {
        Ok(Some(workflow)) => {
            let agent = state
                .get_agent_for_route(session_id.clone())
                .await
                .map_err(|status| ErrorResponse {
                    message: format!("Failed to get agent: {}", status),
                    status,
                })?;
            // ⚠ The returned prompt is deliberately DROPPED.
            // `apply_workflow_to_agent` already committed it, through
            // `workflow::runtime::apply_prepared_to_agent` ->
            // `Agent::set_session_context_prompt`, into the single named
            // `session_context` slot — re-applying replaces, it does not stack.
            // Feeding it a second time to `extend_system_prompt` (which is
            // `Vec::push` on `system_prompt_extras`, with no dedup) appended
            // another full copy of the workflow block on EVERY
            // `PUT /sessions/{id}/user_workflow_values`, and since
            // `prepare_prompt` now inlines each declared skill's whole
            // `SKILL.md`, a copy is kilobytes. The two call sites in
            // `routes/agent.rs` were converted to the named-slot form; this one
            // was missed. See `configure_agent` in `routes/apps.rs` for the
            // same mechanism on the app socket.
            let _committed_by_apply_workflow_to_agent =
                apply_workflow_to_agent(&agent, &session_id, &workflow, false)
                    .await
                    .map_err(|e| ErrorResponse {
                        message: e.to_string(),
                        status: StatusCode::BAD_REQUEST,
                    })?;
            Ok(Json(UpdateSessionUserWorkflowValuesResponse { workflow }))
        }
        Ok(None) => Err(ErrorResponse {
            message: "Missing required parameters".to_string(),
            status: StatusCode::BAD_REQUEST,
        }),
        Err(e) => Err(ErrorResponse {
            message: e.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }),
    }
}

#[utoipa::path(
    delete,
    path = "/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session deleted successfully"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56, QA 2026-09-10 \
                                      F0): the same refusal, word for word, that `GET \
                                      /sessions/{session_id}` gives — including for a chat that \
                                      does not exist (body = plain text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Issue #56, QA 2026-09-10 F0. A caller holding nothing but the daemon
    // secret was refused this chat's transcript and could delete it — four of
    // four, measured — so the delete now asks the read's own gate, FIRST: before
    // the turn is cancelled and before anything parked on a person is released,
    // because each of those is itself an effect on the chat. The refusal is the
    // read's, byte for byte, so it no more confirms the chat exists than the read
    // does — where the old 200/404 pair confirmed it and then destroyed it.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }

    // Deleting a chat stops its turn. This used to happen by accident and the
    // accident is gone: the UI unmounted the deleted chat, which dropped its SSE
    // socket, and a failed `tx.send` cancelled the turn. Now a turn survives its
    // listeners by design — which is the whole feature — so nothing was left
    // stopping a turn from running for up to five more minutes (the orphan
    // timeout), spending tokens and writing into a session that no longer
    // exists. Cancel BEFORE the delete, so the runner unwinds against a session
    // that is still readable rather than erroring its way out of a missing one.
    if let Some(turn_id) = state.cancel_turn(&session_id) {
        tracing::info!(
            session_id = %session_id,
            turn_id = %turn_id,
            "cancelled the in-flight turn of a session being deleted"
        );
    }

    // #107: and release anything parked on a human for it. A card belonging to a
    // deleted chat can never be answered — the surface it would be drawn on is
    // gone — so leaving it queued means the call holding a child CLI's HTTP
    // request open waits out its full TTL for a decision nobody can make. The
    // turn cancel above covers a call parked inside a running turn; this covers
    // the rest, and is idempotent when there is nothing to release.
    let released =
        biorouter::pending_user_action::PendingUserActions::global().cancel_session(&session_id);
    if released > 0 {
        tracing::info!(
            session_id = %session_id,
            released,
            "released requests parked on a person for a session being deleted"
        );
    }

    match state.session_manager().delete_session(&session_id).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) if e.to_string().contains("not found") => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/export",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session exported successfully", body = String),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Out of reach - a private or unreadable session named without the user-action proof"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn export_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Syntax only, and deliberately ahead of the gate — see `get_session`.
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // `SessionManager::export_session` is `get_session(id, true)` followed by
    // `to_string_pretty`: byte for byte the payload `GET /sessions/{id}` returns,
    // and that route has been gated since Task 58. An unguarded sibling of a
    // guarded read is the defect this campaign keeps shipping, so the gate goes
    // here too, before the transcript is loaded.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }
    let Ok(exported) = state.session_manager().export_session(&session_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    Json(exported).into_response()
}

#[utoipa::path(
    post,
    path = "/sessions/import",
    request_body = ImportSessionRequest,
    responses(
        (status = 200, description = "Session imported successfully", body = Session),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 400, description = "Bad request - Invalid JSON"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn import_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ImportSessionRequest>,
) -> Result<Json<Session>, StatusCode> {
    let session = state
        .session_manager()
        .import_session(&request.json)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    Ok(Json(session))
}

/// Prepare a session for an edited message.
///
/// `diverge` (the default) copies the history into a NEW session, trimmed at the
/// edited message, and returns that session's id. The original is untouched.
///
/// `edit` truncates THIS session in place, dropping every message from
/// `timestamp` onwards. Because that destroys history a concurrent writer may
/// already have been told was saved, the cut is refused — with nothing deleted —
/// while a turn is in flight, and it never reaches past the rows the server read
/// when it took the request. You may additionally send `expectedMessageIds`, the
/// ids of every message your view of the session holds: the cut is then also
/// refused if the session holds a message your view does not name, and the 409
/// body's `missing_message_ids` says which, so you can re-read and retry.
#[utoipa::path(
    post,
    path = "/sessions/{session_id}/edit_message",
    request_body = EditMessageRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session prepared for editing - frontend should submit the edited message", body = EditMessageResponse),
        (status = 400, description = "Bad request - invalid session id"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 DR-19): \
                                      `editType: diverge` on a private chat branches it into a \
                                      new chat that inherits its private model, and the request \
                                      carried no proof it came from the user (body = plain text)"),
        (status = 404, description = "Session or message not found"),
        (status = 409, description = "A turn is in flight for this session, or a supplied \
                                      `expectedMessageIds` is missing messages the server holds \
                                      (nothing was deleted; re-read the session and retry)"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn edit_message(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<EditMessageRequest>,
) -> Response {
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let manager = state.session_manager();
    match request.edit_type {
        EditType::Diverge => {
            // Issue #56 DR-19. The second of the two HTTP copy handlers, and the
            // one a gate written against `/diverge` alone would miss.
            // `diverge_session_for_edit` performs the same carry-over, so it
            // mints a session already bound to the source's private provider —
            // see `diverge_session` below for why nothing downstream ever sees
            // that as a raise.
            //
            // A CONDITION on the SOURCE's tier, not an unconditional refusal.
            // Fails closed on a source that cannot be read: a session we cannot
            // classify is not one we can prove is public, and the copy below
            // would fail on the same read anyway.
            let source_is_private = manager
                .get_session(&session_id, false)
                .await
                .map(|session| session.privacy_tier == SessionClassification::Private)
                .unwrap_or(true);
            let had_user_action = is_user_action(&headers);
            if source_is_private && !had_user_action {
                return (StatusCode::FORBIDDEN, COPY_OF_PRIVATE_NEEDS_USER).into_response();
            }
            match manager
                .diverge_session_for_edit(&session_id, request.timestamp)
                .await
            {
                Ok(new_session) => {
                    // The gate again, on the capability that now exists rather
                    // than on the read that predicted it — see
                    // `minted_capability_without_proof`. This arm is the second
                    // copy handler, so it takes the second read as well; a fix
                    // applied to `/diverge` alone would leave the same window
                    // open here, which is the omission shape that shipped last
                    // time.
                    if minted_capability_without_proof(new_session.privacy_tier, had_user_action) {
                        if let Err(e) = manager.delete_session(&new_session.id).await {
                            tracing::error!(
                                session_id = %new_session.id,
                                error = %e,
                                "dr19_unproven_branch_not_removed",
                            );
                        }
                        return (StatusCode::FORBIDDEN, COPY_OF_PRIVATE_NEEDS_USER).into_response();
                    }
                    Json(EditMessageResponse {
                        session_id: new_session.id,
                    })
                    .into_response()
                }
                Err(e) => {
                    tracing::error!("Failed to diverge session for message edit: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
        EditType::Edit => {
            // Issue #56, QA 2026-09-10 (F0's sweep). The in-place arm TRUNCATES
            // this chat's history, and it asked nothing of the caller — so a
            // caller the read refuses could cut a private transcript it could
            // not see. It asks the read's gate now, before the turn lock (whose
            // 409 would say the chat is busy) and before the snapshot.
            //
            // The `Diverge` arm is left on DR-19's gate above, which is strictly
            // stronger for a private source (the proof, not merely reach) and
            // already answers an unreadable one as private.
            if let Err(refusal) = crate::routes::session_reach::session_reach(
                state.session_manager(),
                &session_id,
                &headers,
            )
            .await
            {
                return refusal.into_response();
            }
            edit_in_place(&state, &session_id, &request).await
        }
    }
}

/// The destructive half of [`edit_message`]: truncate the LIVE session.
///
/// #51 NF-D. This used to delete an arbitrary tail from a client-supplied
/// timestamp with no freshness check, no expected state and no turn lock —
/// strictly more dangerous than the `/reply` write-back hardened alongside it,
/// which at least carried the client's own copy for the server to compare
/// against. It now takes the same three guards, in the same order, answering
/// with the same codes:
///
/// 1. the BR-33 per-session turn lock, so a cut cannot land in a session an
///    agent is generating into — `/reply` refuses a second writer with 409 and
///    so does this;
/// 2. the client's view of the conversation (`expected_message_ids`) **when it
///    is supplied**, refused with 409 + `missing_message_ids` when the store
///    holds a message that view never saw — the WIDE race, between the client
///    rendering the conversation and the user clicking edit;
/// 3. [`biorouter::session::session_manager::SessionManager::truncate_conversation_bounded`]
///    rather than the open-above `truncate_conversation`, so a message landing
///    between the check above and the DELETE — the NARROW race, which no
///    client-side check can close — is kept rather than destroyed, and a basis
///    from a previous incarnation of a recycled session id is refused outright.
///
/// Guard 2 is the only one that needs the client's cooperation, and it is
/// therefore the only optional one. It landed REQUIRED, which was wrong: at the
/// time the reply stream never published the ids messages are persisted under,
/// so a client that had watched a live turn could not name them — a required
/// field would 409 every in-place edit in every live chat, which is a regression
/// against the unguarded endpoint this replaced.
///
/// #59 has since made the stream publish them (`MessagesPersisted`), so a
/// complete client view is now ACHIEVABLE. It is not yet ACHIEVED: a client only
/// has one if it consumes that frame, and none does — the desktop supplies the
/// field solely for a session it has just re-read from the store. Making this
/// field required again is the last step of #59, not a precondition of it, and
/// the log below is what will show when that step is safe to take.
/// Sent, it is enforced
/// exactly as strictly as before; omitted, the cut falls back to guards 1 and 3,
/// which need nothing from the client and are still strictly more than the
/// nothing that shipped before this. The gap is logged at `warn`, per cut, so it
/// shows up in an operator's log and not only in a design document.
///
/// An EMPTY `expected_message_ids` is *not* omission: it is a client asserting
/// its view holds nothing, so it goes down the checked path and a non-empty
/// session refuses it. Folding the two together would hand every client a
/// one-token opt-out of guard 2 and quietly delete the check.
async fn edit_in_place(
    state: &Arc<AppState>,
    session_id: &str,
    request: &EditMessageRequest,
) -> Response {
    // (1) Hold the BR-33 turn lock for the whole check-and-cut, so a `/reply`
    // cannot start against the rows being deleted. Named binding: `let _ =`
    // would drop the guard immediately and hold nothing.
    let _turn_guard = match state.try_begin_turn_idempotent(
        session_id,
        CancellationToken::new(),
        // No idempotency key: an edit is not a turn a client retries under a
        // name, so every conflict here is a genuine "someone else has the
        // session", never a `duplicate: true`.
        None,
    ) {
        Ok(guard) => guard,
        Err(conflict) => {
            tracing::warn!(
                "Refused an in-place edit of session {}: turn {} is in flight",
                session_id,
                conflict.running_turn_id
            );
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "type": "Error",
                    "error": "A turn is in progress for this session; nothing was deleted. \
                              Stop it and retry.",
                    "code": "turn_in_flight",
                    "running_turn_id": conflict.running_turn_id,
                })),
            )
                .into_response();
        }
    };

    // Revision first, then conversation (see `snapshot_for_rewrite`): a message
    // landing between the two reads is then inside the snapshot rather than
    // looking foreign. Taken before the client-view check because `basis` bounds
    // the cut in BOTH cases — it is the server's own view, and the fallback when
    // there is no client one.
    let (session, basis) = match state
        .session_manager()
        .snapshot_for_rewrite(session_id)
        .await
    {
        Ok(snapshot) => snapshot,
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                tracing::error!("Failed to snapshot session {} for edit: {}", session_id, e);
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return status.into_response();
        }
    };
    let stored = session.conversation.unwrap_or_default();

    // (2) The wide race: anything stored the client never saw would be deleted
    // by a cut it planned without knowing about it. Checkable only against a
    // view the client supplied; `Some(&[])` is such a view (it claims to hold
    // nothing) and is checked, `None` is the absence of one.
    match request.expected_message_ids.as_deref() {
        Some(client_view) => {
            let missing = unacknowledged_stored_ids(stored.messages(), client_view);
            if !missing.is_empty() {
                tracing::warn!(
                    "Refused an in-place edit of session {}: {} stored message(s) the client's \
                     view does not contain",
                    session_id,
                    missing.len()
                );
                return edit_conflict_response(missing, stored.messages().len());
            }
        }
        None => {
            tracing::warn!(
                "In-place edit of session {} carries no `expectedMessageIds`, so the wide race \
                 (a message appended between the client rendering the conversation and this \
                 request) is unchecked; the cut is still under the turn lock and bounded to the \
                 {} message(s) the server just read. The reply stream now publishes the ids a \
                 turn persisted (issue #59, `MessagesPersisted`), so a client that consumes that \
                 frame can supply a view; this one did not.",
                session_id,
                stored.messages().len()
            );
        }
    }

    // (3) The narrow race: `basis` bounds the delete to the rows this handler
    // actually read, inside the truncation's own transaction.
    match state
        .session_manager()
        .truncate_conversation_bounded(session_id, request.timestamp, basis)
        .await
    {
        Ok(TruncateOutcome::Truncated { .. }) => Json(EditMessageResponse {
            session_id: session_id.to_string(),
        })
        .into_response(),
        // The session was wiped and recreated under this id between the snapshot
        // and the cut, so the basis describes a conversation that no longer
        // exists. Same answer as a stale view — refuse, delete nothing.
        Ok(TruncateOutcome::Stale) => {
            tracing::warn!(
                "Refused an in-place edit of session {}: the conversation moved under the check",
                session_id
            );
            edit_conflict_response(Vec::new(), stored.messages().len())
        }
        Ok(TruncateOutcome::SessionNotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to truncate conversation: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Diverge (branch) a session into a brand-new session that inherits the full
/// conversation history and metadata of the original, while leaving the
/// original completely untouched. Unlike `edit_message` with `Diverge`, no
/// truncation is performed — the entire history is carried over so the user can
/// continue a fresh conversation from exactly where they left off.
#[utoipa::path(
    post,
    path = "/sessions/{session_id}/diverge",
    request_body = DivergeSessionRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session to diverge from")
    ),
    responses(
        (status = 200, description = "Session diverged successfully", body = DivergeSessionResponse),
        (status = 400, description = "Bad request - Invalid session id or name too long"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 DR-19): the \
                                      source chat is private, so the branch would inherit its \
                                      private model, and the request carried no proof it came \
                                      from the user (body = plain text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn diverge_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<DivergeSessionRequest>,
) -> Result<Json<DivergeSessionResponse>, Response> {
    if !is_valid_session_id(&session_id) {
        return Err(StatusCode::BAD_REQUEST.into_response());
    }
    let manager = state.session_manager();

    // Validate the optional custom name up front (blank → use the auto branch
    // name; too long → reject).
    if let Some(ref n) = request.name {
        if !n.trim().is_empty() && n.trim().chars().count() > MAX_NAME_LENGTH {
            return Err(StatusCode::BAD_REQUEST.into_response());
        }
    }

    // A missing source session is a clean 404 (rather than a 500 from the copy).
    let source = manager
        .get_session(&session_id, false)
        .await
        .map_err(|_| StatusCode::NOT_FOUND.into_response())?;

    // Issue #56 DR-19. The branch inherits `provider_name` and `model_config`,
    // not just the tier (`create_derived_session`) — so branching a private chat
    // does not copy a label, it MINTS a new session already bound to a private
    // provider. DR-16 guards raises on sessions that already exist
    // (`raise_needs_user_action` compares a live agent's bound provider against
    // a requested one), so a session born private passes no gate at all: it
    // never calls `POST /agent/update_provider`, and every gate downstream then
    // reads it as legitimately private. This route sits behind `check_token`
    // alone, which AR-11 and AR-15 between them establish is not a proof of a
    // human.
    //
    // A CONDITION on the SOURCE's tier, never an unconditional refusal:
    // branching a public chat mints no capability, and a gate that fires on
    // every branch of every chat is one people route around. The proof is Task
    // 18A's `X-User-Action`, reused — one proof of user in this feature, not
    // two — and the renderer holds the key, so the person at the keyboard
    // clicks Diverge and it works.
    let had_user_action = is_user_action(&headers);
    if source.privacy_tier == SessionClassification::Private && !had_user_action {
        return Err((StatusCode::FORBIDDEN, COPY_OF_PRIVATE_NEEDS_USER).into_response());
    }

    // diverge_session does the placeholder-aware, sibling-numbered naming,
    // records the lineage pointer + divergence point, and trims the branch to end at
    // the last complete assistant answer. Anchored by the durable message id
    // (`truncate_after_id`) when supplied — unambiguous even for two same-second
    // messages — else by the legacy `truncate_after` timestamp.
    let new_session = manager
        .diverge_session_at(
            &session_id,
            request.name,
            request.truncate_after,
            request.truncate_after_id,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to diverge session {}: {}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;

    // The gate again, on the capability that now exists rather than on the read
    // that predicted it. Reached only when the source was raised between the two
    // reads — see `minted_capability_without_proof`.
    if minted_capability_without_proof(new_session.privacy_tier, had_user_action) {
        if let Err(e) = manager.delete_session(&new_session.id).await {
            // Loud, and still refused: the caller gets nothing back either way,
            // but a row that survives this is a private-capability session no
            // human authorised, and the only trace of it would be this line.
            tracing::error!(
                session_id = %new_session.id,
                error = %e,
                "dr19_unproven_branch_not_removed",
            );
        }
        return Err((StatusCode::FORBIDDEN, COPY_OF_PRIVATE_NEEDS_USER).into_response());
    }

    Ok(Json(DivergeSessionResponse {
        session_id: new_session.id,
        working_dir: new_session.working_dir.to_string_lossy().to_string(),
        name: new_session.name,
        diverged_from: new_session.diverged_from,
    }))
}

/// Issue #56 §12.4. What `POST /sessions/{id}/declassify` says to a caller
/// holding nothing but the daemon secret.
///
/// §9.3 A1: that secret is reachable from any developer-enabled agent shell, so
/// `X-Secret-Key` alone is not a human (AR-11/AR-15). Declassification is the one
/// operation in this feature that *reverses* the ratchet, so it is the last
/// place an unproven caller may be given the benefit of the doubt.
///
/// §14.4's content rule applies as it does to every other refusal: it names the
/// boundary and nothing about the chat, and it forecloses the retry, because a
/// model that reads a refusal as transient loops on it.
///
/// ⚠ It deliberately carries NEITHER of the two markers the renderer keys on —
/// not `USER_ACTION_REFUSAL_MARKER` ("is the user's decision, not yours"), whose
/// toast says *switch this chat's model*, and not
/// `COPY_OF_PRIVATE_REFUSAL_MARKER` ("only the person at the keyboard may do
/// it"), whose toast says *branch it from the chat window*. Both would send the
/// user somewhere that cannot help, so the wording steps around them; pinned by
/// [`declassify_tests::the_refusals_say_different_things`].
const DECLASSIFY_NEEDS_USER: &str =
    "Marking a private chat public is a decision only the person at the keyboard can make, and \
     this request carried no proof it came from them. Nothing was changed. Do not retry; the \
     same call will be refused again. If this chat no longer holds anything private, stop and \
     ask the user to mark it public from the chat history.";

/// What `POST /sessions/{id}/declassify` says when the typed confirmation does
/// not match. Human-facing: the only caller that reaches it is the renderer's
/// own dialog, and only if its phrase check and the daemon's disagree.
const DECLASSIFY_CONFIRMATION_MISMATCH: &str =
    "The confirmation did not match the last six characters of this chat's id. Nothing was \
     changed.";

/// Issue #56 DR-20, Task 55. What the route says when the system authentication
/// did not happen. The prompter's own sentence is appended, because "you pressed
/// Cancel" and "this machine has no way to raise the prompt" need different
/// advice and only the prompter knows which it was.
///
/// ⚠ It carries neither renderer marker, for the same reason
/// [`DECLASSIFY_NEEDS_USER`] does not: the model picker's toast says *switch
/// this chat's model* and the copy handler's says *branch it from the chat
/// window*, and neither marks a chat public.
///
/// ⚠ **It says what the RECORD says, not what the conversation did.** It used to
/// open "This chat reached a private data source", which is §12.4's rationale
/// for the strong control and is false for two of the provenances that reach it:
/// a `backfill:*` chat was marked by the one-time migration from the model it
/// was last bound to, and an `imported` chat arrived already marked. On day one
/// `backfill:*` is most of the private rows on a machine with history.
///
/// This route deliberately does not read the row — grading here would be a
/// check-then-act, and the comment in the handler says why — so it has no
/// provenance to name and uses the catch-all clause from
/// `biorouter::privacy::declassify::strong_confirmation_reason`, which is the
/// one statement true of every chat that reaches this arm. The CLI and the
/// desktop dialog, which do hold the provenance, say the specific one.
/// `the_system_auth_refusal_claims_only_what_the_record_says` pins the link.
const DECLASSIFY_SYSTEM_AUTH_REFUSED: &str =
    "This chat does not record an observed turn on a private model as the reason it is private, \
     so marking it public needs your operating system to confirm it is you. That did not happen, \
     and nothing was changed.";

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeclassifySessionRequest {
    /// The last six characters of the session id, as the user typed them.
    ///
    /// Required when the chat's provenance is anything other than `turn:*` —
    /// see `biorouter::privacy::declassify::requires_typed_confirmation`, which
    /// is where the grading lives. The daemon re-derives the grade from the
    /// STORED provenance rather than trusting the client to say which control it
    /// showed, so a caller cannot claim the single-click path for a chat that
    /// reached a private data source.
    #[serde(default)]
    pub confirmation: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeclassifySessionResponse {
    pub session_id: String,
    /// The tier after the call. Always `public` on a 200 — including the
    /// already-public case, which is a success rather than a conflict so a
    /// double-clicked confirm button does not surface an error. That holds on
    /// BOTH graded paths: an already-public row is answered before the
    /// confirmation is consulted, so the second single-click request is not
    /// refused over a phrase that path never showed the user (the first call
    /// rewrites the provenance to `declassified_by_user`, which grades onto the
    /// typed control).
    pub privacy_tier: SessionClassification,
}

/// Mark a private chat public (issue #56 §12.4/§12.5).
///
/// The ONLY route in the tree that lowers a session's classification. It is
/// user-only (DR-16's `X-User-Action`), it is graded on the chat's provenance,
/// and every use leaves a row in `classification_audit` — a declassified session
/// must never become indistinguishable from one that was always public.
#[utoipa::path(
    post,
    path = "/sessions/{session_id}/declassify",
    request_body = DeclassifySessionRequest,
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session to declassify")
    ),
    responses(
        (status = 200, description = "The chat is public", body = DeclassifySessionResponse),
        (status = 400, description = "Bad request - invalid session id, or the typed confirmation \
                                      did not match (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 §12.4): lowering a \
                                      chat's classification is the user's decision, and the \
                                      request carried no proof it came from them (body = plain \
                                      text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn declassify_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<DeclassifySessionRequest>,
) -> Result<Json<DeclassifySessionResponse>, Response> {
    if !is_valid_session_id(&session_id) {
        return Err(StatusCode::BAD_REQUEST.into_response());
    }

    // FIRST, before the row is even read. An unproven caller learns nothing
    // about which ids exist, and the refusal cannot be told apart from one for a
    // session that is already public.
    if !is_user_action(&headers) {
        return Err((StatusCode::FORBIDDEN, DECLASSIFY_NEEDS_USER).into_response());
    }

    // §12.4's grade is NOT decided here. The confirmation is handed to the
    // writer, which derives the grade from the provenance its own transaction
    // reads — so a chat that reaches an MCP data source between this request
    // arriving and the row being written cannot be declassified on the
    // single-click control it has stopped qualifying for. Reading the row here
    // to grade it would be a check-then-act, and would also cost a second
    // round-trip to say what the transaction is about to read anyway.
    //
    // The construction below is this file's only construction of the
    // proof-of-user, pinned by
    // `privacy::declassify::tests::the_proof_of_user_is_constructed_in_exactly_two_places`
    // — the other being `biorouter session declassify <id>` (issue #56 Task 31),
    // which is the only surface that can reach a private chat no listing shows.
    // ONE construction for a request that may call the writer twice: the proof
    // is borrowed, so the probe below and the write that follows it spend the
    // same human action rather than minting a second one.
    let ok = UserConfirmation::from_typed_confirmation();
    let manager = state.session_manager();
    let confirmation = request.confirmation.as_deref();

    // Issue #56 DR-20, Task 55. The FIRST call is a probe and writes nothing:
    // it finds the row, takes §12.4's grade from the provenance inside its own
    // transaction, and checks the typed phrase. Only if all of that passes does
    // it answer `SystemAuthenticationRequired`, and only then is the user asked
    // for their password — so a mistyped phrase is answered by a form field
    // rather than by an operating-system dialog the user had to satisfy first,
    // and a `turn:*` chat (which never reaches this arm) keeps its single click
    // with no prompt at all.
    let mut outcome = declassify(manager, &session_id, confirmation, None, &ok).await;
    if matches!(outcome, Ok(DeclassifyOutcome::SystemAuthenticationRequired)) {
        match authenticate_declassification(std::slice::from_ref(&session_id)).await {
            Ok(granted) => {
                outcome = declassify(manager, &session_id, confirmation, Some(&granted), &ok).await;
            }
            Err(refusal) => {
                return Err((
                    StatusCode::FORBIDDEN,
                    format!("{DECLASSIFY_SYSTEM_AUTH_REFUSED} {refusal}"),
                )
                    .into_response());
            }
        }
    }

    match outcome {
        Ok(DeclassifyOutcome::SessionNotFound) => Err(StatusCode::NOT_FOUND.into_response()),
        Ok(DeclassifyOutcome::ConfirmationRequired) => {
            Err((StatusCode::BAD_REQUEST, DECLASSIFY_CONFIRMATION_MISMATCH).into_response())
        }
        // Unreachable through the branch above, which either supplies an
        // authorisation naming exactly this id or returns. Answered rather than
        // collapsed into the 500 arm because the honest reading of it is "the
        // authentication did not cover this chat", which is a refusal and not a
        // daemon fault — and because an outcome that lowers nothing must never
        // be reported as a success.
        Ok(DeclassifyOutcome::SystemAuthenticationRequired) => Err((
            StatusCode::FORBIDDEN,
            DECLASSIFY_SYSTEM_AUTH_REFUSED.to_string(),
        )
            .into_response()),
        Ok(DeclassifyOutcome::Declassified) | Ok(DeclassifyOutcome::AlreadyPublic) => {
            Ok(Json(DeclassifySessionResponse {
                session_id,
                privacy_tier: SessionClassification::Public,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to declassify session {}: {}", session_id, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionExtensionsResponse {
    extensions: Vec<ExtensionConfig>,
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/extensions",
    params(
        ("session_id" = String, Path, description = "Unique identifier for the session")
    ),
    responses(
        (status = 200, description = "Session extensions retrieved successfully", body = SessionExtensionsResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("api_key" = [])
    ),
    tag = "Session Management"
)]
async fn get_session_extensions(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    if !is_valid_session_id(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Issue #56, QA 2026-09-10 — M2's sibling. A private chat's enabled
    // extensions name, by name, the private connectors Gate E hides from a
    // public model's own tool list (`cdwagent`, `ucsfomopagent`), so this asks
    // the read's gate before it reads the row.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }
    match session_extensions(&state, &session_id).await {
        Ok(extensions) => Json(SessionExtensionsResponse { extensions }).into_response(),
        Err(status) => status.into_response(),
    }
}

/// The enabled extension list of a chat the caller may address.
async fn session_extensions(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<Vec<ExtensionConfig>, StatusCode> {
    let session = state
        .session_manager()
        .get_session(session_id, false)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let extensions = if session.session_type == SessionType::SubAgent {
        match biorouter::agents::persisted_subagent_extension_projection(&session.extension_data) {
            Ok(Some(extensions)) => extensions,
            Ok(None) => EnabledExtensionsState::from_extension_data(&session.extension_data)
                .map(|state| state.extensions)
                .unwrap_or_default(),
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    } else {
        EnabledExtensionsState::from_extension_data(&session.extension_data)
            .map(|state| state.extensions)
            .unwrap_or_else(biorouter::config::get_enabled_extensions)
    };

    Ok(extensions)
}

/// BR-71: the sessions holding a turn right now.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RunningSessionsResponse {
    pub session_ids: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/sessions/running",
    // EXPLICIT tag, not utoipa's default module path. Task 42b's parity gate
    // selects the workspace-control route surface by this tag; a BR-71 route
    // that lands under "Session Management" with the other fifteen operations
    // is invisible to it.
    tag = "workspace",
    responses(
        (status = 200, description = "Sessions with a turn in flight", body = RunningSessionsResponse),
        (status = 401, description = "Unauthorized - invalid secret key")
    )
)]
async fn running_sessions(State(state): State<Arc<AppState>>) -> Json<RunningSessionsResponse> {
    Json(RunningSessionsResponse {
        session_ids: state.active_turn_session_ids(),
    })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions", get(list_sessions))
        .route("/sessions/sidebar", get(list_sidebar_sessions))
        .route("/sessions/running", get(running_sessions))
        .route("/sessions/{session_id}", get(get_session))
        .route("/sessions/{session_id}", delete(delete_session))
        .route("/sessions/{session_id}/export", get(export_session))
        .route("/sessions/import", post(import_session))
        .route("/sessions/insights", get(get_session_insights))
        .route("/sessions/activity", get(get_session_activity))
        // Static `/usage` suffix — distinct from the `/sessions/{session_id}`
        // wildcard, so axum routes it without a guard.
        .route("/sessions/{session_id}/usage", get(get_session_usage))
        .route("/sessions/{session_id}/name", put(update_session_name))
        .route(
            "/sessions/{session_id}/user_workflow_values",
            put(update_session_user_workflow_values),
        )
        .route("/sessions/{session_id}/edit_message", post(edit_message))
        .route("/sessions/{session_id}/diverge", post(diverge_session))
        .route(
            "/sessions/{session_id}/declassify",
            post(declassify_session),
        )
        .route(
            "/sessions/{session_id}/extensions",
            get(get_session_extensions),
        )
        .with_state(state)
}

// `pub(crate)` for [`install_test_user_action_key`] alone: the digest lives in a
// process-global `OnceLock` shared by every test in this binary, so a second
// module that installed its own key would turn whichever ran second into a wall
// of 403s reported as policy results. Task 30A's disclosure-ack test is that
// second module and takes this one instead.
#[cfg(test)]
pub(crate) mod diverge_tests {
    //! ⚠ **If a test in here fails with a 500 where it asserted a 200, suspect
    //! the session store's PATH before you suspect this file.**
    //!
    //! Several of these are read-only route tests over one process-global
    //! session store, and they were the rotating victims of a
    //! `test (windows-latest)` flake: `activity_clamps_an_absurd_window`
    //! answering 500 on one run, `sidebar_route_…` or a `/usage` route on the
    //! next, and — measured in one job of PR #240 — the same test `... ok` in the
    //! `biorouter_server` lib binary and `... FAILED` in the `biorouterd` bin
    //! binary 61 s later, off identical source (`main.rs` and `lib.rs` both
    //! declare `mod routes`).
    //!
    //! The 500 is honest: `get_activity(..).map_err(|_| INTERNAL_SERVER_ERROR)`
    //! reporting a real database error. The database had been moved out from
    //! under the process — a sibling test relocated `BIOROUTER_PATH_ROOT` under a
    //! `TempDir`, won the race to be the first to touch the store, and then
    //! unlinked it. The already-open connection kept answering, so it only bit
    //! when the pool had to open a *second* one: `(code: 14) unable to open
    //! database file`.
    //!
    //! `src/test_sandbox.rs` now freezes the store's directory before any test
    //! runs, and `tests/session_store_survives_a_relocated_path_root.rs` holds
    //! that closed. Read the pinning section of the first before changing
    //! anything that relocates the path root.

    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use biorouter::conversation::message::Message;
    use biorouter::session::SessionType;
    use serial_test::serial;
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// The user-action key this test binary's "daemon" was launched with.
    ///
    /// Issue #56 DR-16 keeps the digest in a process-global `OnceLock` that
    /// `commands::agent` fills from stdin, so a route test that wants the
    /// *authorised* arm has to install one. Nothing else in this crate installs
    /// a digest — `auth::tests` exercises the pure `user_action_matches` — and
    /// [`install_test_user_action_key`] asserts the install took rather than
    /// letting a second installer turn every positive arm below into a silent
    /// 403.
    ///
    /// `pub(super)` so [`super::declassify_tests`] installs the SAME key rather
    /// than a second one: the digest lives in a `OnceLock`, so whichever module
    /// ran first would win and every authorised arm in the other would report a
    /// 403 as though it were a policy result.
    pub(crate) const TEST_USER_ACTION_KEY: &str = "task-22-diverge-route-user-action-key";

    pub(crate) fn install_test_user_action_key() {
        let digest: [u8; 32] =
            <sha2::Sha256 as sha2::Digest>::digest(TEST_USER_ACTION_KEY.as_bytes()).into();
        biorouter_server::auth::install_user_action_digest(Some(digest));

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "X-User-Action",
            TEST_USER_ACTION_KEY.parse().expect("a header-safe key"),
        );
        assert!(
            biorouter_server::auth::is_user_action(&headers),
            "the user-action digest this test installs did not take: something else in this \
             test binary installed a different one first, and every authorised arm below would \
             otherwise report a 403 as a policy result"
        );
    }

    /// `POST /sessions/{id}/diverge`, optionally carrying DR-16's proof-of-user
    /// header. `user_action: None` is a request holding nothing but the daemon
    /// secret — the caller AR-11/AR-15 establish is indistinguishable from the
    /// model.
    async fn post_diverge_with(
        state: Arc<AppState>,
        session_id: &str,
        body: serde_json::Value,
        user_action: Option<&str>,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/sessions/{session_id}/diverge"))
            .header("content-type", "application/json");
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let req = builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// `POST /sessions/{id}/edit_message`, same header treatment. The edit route
    /// has its own test module below; this copy exists because the DR-19 gate is
    /// one requirement across BOTH copy handlers, and a test that reached only
    /// `/diverge` would miss exactly the half that shipped broken last time.
    async fn post_edit_message_with(
        state: Arc<AppState>,
        session_id: &str,
        body: serde_json::Value,
        user_action: Option<&str>,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/sessions/{session_id}/edit_message"))
            .header("content-type", "application/json");
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let req = builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn post_diverge(
        state: Arc<AppState>,
        session_id: &str,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        post_diverge_with(state, session_id, body, None).await
    }

    fn user_msg(text: &str) -> Message {
        Message::user().with_text(text)
    }

    #[test]
    fn edit_message_uses_diverge_as_the_canonical_action() {
        let request: EditMessageRequest =
            serde_json::from_value(serde_json::json!({ "timestamp": 1, "editType": "diverge" }))
                .unwrap();
        assert!(matches!(request.edit_type, EditType::Diverge));

        let legacy: EditMessageRequest =
            serde_json::from_value(serde_json::json!({ "timestamp": 1, "editType": "fork" }))
                .unwrap();
        assert!(matches!(legacy.edit_type, EditType::Diverge));
        assert!(matches!(default_edit_type(), EditType::Diverge));
    }

    async fn get_activity(
        state: Arc<AppState>,
        query: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions/activity{query}"))
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn get_sidebar_sessions(
        state: Arc<AppState>,
        query: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions/sidebar{query}"))
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// `/sessions/activity` must not be swallowed by the `/sessions/{session_id}`
    /// wildcard registered next to it, and the payload must be camelCase.
    ///
    /// NOTE: `AppState::new()` opens the ONE shared session database, so a route
    /// test here must be READ-ONLY. The behaviour of the activity aggregation is
    /// covered by `session_manager`'s unit tests, which use a `TempDir`.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn activity_route_is_not_shadowed_by_the_session_id_wildcard() {
        let state = AppState::new().await.unwrap();
        let (status, body) = get_activity(state, "?days=30").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(body.get("days").is_some(), "got {body}");
        assert!(body.get("currentStreak").is_some(), "camelCase payload");
        assert!(body.get("longestStreak").is_some());
        assert!(body.get("maxTokens").is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn sidebar_route_returns_paginated_lightweight_sessions() {
        let state = AppState::new().await.unwrap();
        let (status, body) = get_sidebar_sessions(state, "?limit=2").await;
        assert_eq!(status, axum::http::StatusCode::OK);

        let sessions = body
            .get("sessions")
            .and_then(|sessions| sessions.as_array())
            .expect("sessions array");
        assert!(sessions.len() <= 2);
        assert!(body.get("has_more").is_some());
        assert!(body.get("next_cursor").is_some());

        if let Some(session) = sessions.first().and_then(|session| session.as_object()) {
            for field in [
                "id",
                "working_dir",
                "name",
                "created_at",
                "updated_at",
                "message_count",
            ] {
                assert!(
                    session.contains_key(field),
                    "missing {field} in {session:?}"
                );
            }
            assert!(!session.contains_key("conversation"));
            assert!(!session.contains_key("extension_data"));
            assert!(!session.contains_key("workflow"));
        }
    }

    /// The cursor round-trips, and carries the two fields it claims to — no
    /// position among them (adversarial security review 2026-09-12, HIGH).
    #[test]
    fn a_sidebar_cursor_carries_one_rows_sort_key_and_nothing_else() {
        let cursor = SidebarCursor {
            updated_at: "2026-09-11 04:05:06".to_string(),
            id: "20260911_040506".to_string(),
        };
        let encoded = encode_sidebar_cursor(&cursor);
        assert_eq!(decode_sidebar_cursor(&encoded), Some(cursor.clone()));

        // Whatever a caller decodes out of it, it is the row it was just handed.
        use base64::Engine as _;
        let plain = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&encoded)
                .unwrap(),
        )
        .unwrap();
        assert!(plain.contains(&cursor.updated_at) && plain.contains(&cursor.id));

        for junk in ["", "not base64!!", "Zm9v", "AB8=", "\u{1f}"] {
            assert_eq!(
                decode_sidebar_cursor(junk),
                None,
                "{junk:?} was accepted as a cursor"
            );
        }
        // Neither half may be empty: an empty id would compare `> ''`, which is
        // every row, and an empty timestamp would resume before the beginning.
        let half =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("2026-09-11 04:05:06\u{1f}");
        assert_eq!(decode_sidebar_cursor(&half), None);
    }

    /// A cursor this route did not mint is a 400, and the `offset` the parameter
    /// replaced is IGNORED rather than rejected — a client built against the old
    /// shape pages from the top instead of breaking.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_sidebar_rejects_a_forged_cursor_and_ignores_a_stale_offset() {
        let state = AppState::new().await.unwrap();
        let (status, _) = get_sidebar_sessions(state.clone(), "?limit=1&cursor=not-a-cursor").await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);

        let (status, body) = get_sidebar_sessions(state.clone(), "?limit=1&offset=40").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let first = get_sidebar_sessions(state, "?limit=1").await.1;
        assert_eq!(
            body["sessions"], first["sessions"],
            "a stale `offset` moved the page it was ignored on"
        );
    }

    /// BR-71: the two type slices `GET /sessions` chooses between. A wrong slice
    /// — a dropped `Scheduled`, say, which would silently empty History of every
    /// scheduled run — compiles and passes every route test in this file, because
    /// those run read-only against whatever the real database happens to hold.
    /// This pins both arms exactly.
    #[test]
    fn listed_session_types_make_subagents_opt_in() {
        assert_eq!(
            listed_session_types(false),
            [SessionType::User, SessionType::Scheduled]
        );
        assert_eq!(
            listed_session_types(true),
            [
                SessionType::User,
                SessionType::Scheduled,
                SessionType::SubAgent
            ]
        );
    }

    async fn get_sessions(
        state: Arc<AppState>,
        query: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions{query}"))
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// BR-71 gave `GET /sessions` a `Query` extractor where it previously had
    /// none. serde ignores unknown fields, so no caller that was already sending
    /// a query string can start getting a 400 — only a malformed value of the
    /// brand-new `include_subagents` parameter can, and that parameter did not
    /// exist before. This pins that distinction.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn list_sessions_route_still_accepts_unknown_query_params() {
        let state = AppState::new().await.unwrap();
        for query in ["", "?foo=bar"] {
            let (status, _) = get_sessions(state.clone(), query).await;
            assert_eq!(status, axum::http::StatusCode::OK, "query {query:?}");
        }
    }

    /// BR-71: neither listing route may surface a `sub_agent` row unless asked,
    /// and neither may ever surface `hidden`/`terminal`. Read-only against the
    /// real database, so every assertion is a one-directional membership check
    /// that cannot flake on an empty or unusual database.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn listing_routes_hide_subagents_unless_asked() {
        let state = AppState::new().await.unwrap();
        let cases = [
            ("default", "", ["user", "scheduled"].as_slice()),
            (
                "opt-in",
                "include_subagents=true",
                ["user", "scheduled", "sub_agent"].as_slice(),
            ),
        ];

        for (label, param, allowed) in cases {
            let (status, body) = get_sessions(state.clone(), &format!("?{param}")).await;
            assert_eq!(status, axum::http::StatusCode::OK, "/sessions {label}");
            for session in body["sessions"].as_array().expect("sessions array") {
                let session_type = session["session_type"]
                    .as_str()
                    .unwrap_or_else(|| panic!("no session_type in {session:?}"));
                assert!(
                    allowed.contains(&session_type),
                    "/sessions {label} leaked a {session_type} session"
                );
            }

            let (status, body) =
                get_sidebar_sessions(state.clone(), &format!("?limit=50&{param}")).await;
            assert_eq!(
                status,
                axum::http::StatusCode::OK,
                "/sessions/sidebar {label}"
            );
            for session in body["sessions"].as_array().expect("sessions array") {
                let session_type = session["session_type"]
                    .as_str()
                    .unwrap_or_else(|| panic!("no session_type in {session:?}"));
                assert!(
                    allowed.contains(&session_type),
                    "/sessions/sidebar {label} leaked a {session_type} session"
                );
                // The sidebar must keep passing `include_empty: false`. Flipping
                // it would start showing message-less "Untitled chat" rows in
                // every user's sidebar — a visible regression no test would
                // otherwise catch, since `include_empty` has no other caller
                // until `workspace_list`.
                assert!(
                    session["message_count"].as_i64().unwrap_or_default() >= 1,
                    "/sessions/sidebar {label} listed an empty session: {session:?}"
                );
            }
        }
    }

    async fn get_usage(
        state: Arc<AppState>,
        session_id: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{session_id}/usage"))
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// The `/usage` response serializes to the exact camelCase JSON the TS client
    /// expects: `models` array of `{modelId, provider, inputTokens, outputTokens,
    /// totalTokens, turns}`, with the unknown bucket carrying `null`.
    ///
    /// This is a pure serialization assertion rather than a live round-trip:
    /// `AppState::new()` opens the ONE shared session DB, whose token-ledger
    /// contents (and, across parallel worktrees, whose schema) are not stable
    /// enough for a write-then-read route test. The aggregation SQL itself is
    /// covered by `session_manager`'s `TempDir` unit tests.
    #[test]
    fn usage_response_serializes_to_camelcase_shape() {
        let response = SessionModelUsageResponse {
            models: vec![
                ModelUsageRow {
                    model_id: Some("gpt-5".to_string()),
                    provider: Some("openai".to_string()),
                    input_tokens: 300,
                    output_tokens: 70,
                    total_tokens: Some(370),
                    cache_read_tokens: Some(800),
                    cache_creation_tokens: Some(0),
                    turns: 2,
                },
                ModelUsageRow {
                    model_id: None,
                    provider: None,
                    input_tokens: 1,
                    output_tokens: 2,
                    total_tokens: Some(3),
                    cache_read_tokens: Some(0),
                    cache_creation_tokens: Some(0),
                    turns: 1,
                },
            ],
        };

        let json = serde_json::to_value(&response).unwrap();
        let models = json.get("models").and_then(|m| m.as_array()).unwrap();
        assert_eq!(models.len(), 2);

        let gpt = &models[0];
        assert_eq!(gpt.get("modelId").and_then(|v| v.as_str()), Some("gpt-5"));
        assert_eq!(gpt.get("provider").and_then(|v| v.as_str()), Some("openai"));
        assert_eq!(gpt.get("inputTokens").and_then(|v| v.as_i64()), Some(300));
        assert_eq!(gpt.get("outputTokens").and_then(|v| v.as_i64()), Some(70));
        assert_eq!(gpt.get("totalTokens").and_then(|v| v.as_i64()), Some(370));
        assert_eq!(gpt.get("turns").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(
            gpt.get("cacheReadTokens").and_then(|v| v.as_i64()),
            Some(800)
        );

        // The unknown bucket serializes model/provider as JSON null.
        let unknown = &models[1];
        assert!(unknown.get("modelId").unwrap().is_null());
        assert!(unknown.get("provider").unwrap().is_null());
        assert_eq!(unknown.get("totalTokens").and_then(|v| v.as_i64()), Some(3));
    }

    /// The `/usage` route rejects an invalid session id before touching the DB,
    /// and the `/usage` suffix is not swallowed by the `/sessions/{session_id}`
    /// wildcard registered next to it.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn usage_route_rejects_invalid_session_id() {
        let state = AppState::new().await.unwrap();
        let (status, _) = get_usage(state, "bad.id").await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    }

    /// The person at the keyboard is told a missing chat is missing. A caller
    /// holding only the daemon secret is told what the read tells it — the same
    /// refusal it gets for a private chat — since QA measured this route's
    /// 200/404 pair to be an oracle for which ids exist (2026-09-10).
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn usage_route_returns_not_found_for_missing_session() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let res = routes(state.clone())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/sessions/29990101_99999/usage")
                    .header("X-User-Action", TEST_USER_ACTION_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::NOT_FOUND);

        let (status, _) = get_usage(state, "29990101_99999").await;
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
    }

    /// A genuine database failure on `/sessions/activity` is still a **500**.
    ///
    /// ⚠ This is here because of the flake in the module note, not in spite of
    /// it. That 500 was *correct* — the store really had been moved out from
    /// under the process — so the only legitimate fix was to make the store
    /// sound. Making this route quieter instead (an empty `ActivityWindow`, a
    /// 200 with zeroes, an `unwrap_or_default`) would have turned the same green
    /// suite into a daemon that answers the Home heatmap with silence when its
    /// database is unreadable. Nothing else asserts that mapping, because it
    /// cannot be driven from here: the handler reads the process-global store,
    /// and there is no seam to hand it a broken one.
    ///
    /// A source scan over the handler's own span, with the negative control
    /// [`crate::routes::body_of`] requires of every caller.
    #[test]
    fn a_database_failure_on_activity_is_reported_as_500() {
        const SOURCE: &str = include_str!("session.rs");
        let handler = crate::routes::body_of(SOURCE, "async fn get_session_activity(");

        // Two `contains`, not the whole line: the exact closure spelling is not
        // the claim (`|_|` vs `|_e|` is a rename, not a behaviour change), and a
        // test that fails on a rename gets relaxed rather than read. The claim is
        // that the error is still MAPPED and that what it maps to is 500.
        assert!(
            handler.contains(".map_err("),
            "the activity route stopped mapping its database error:\n{handler}"
        );
        assert!(
            handler.contains("StatusCode::INTERNAL_SERVER_ERROR"),
            "the activity route stopped reporting a database failure as 500:\n{handler}"
        );
        assert!(
            !handler.contains("SessionModelUsageResponse"),
            "body_of over-read past get_session_activity into the /usage route, so \
             the assertion above may have matched a different handler"
        );
    }

    /// `days` is attacker-controlled; the server clamps it rather than building a
    /// SQL modifier from an arbitrary integer.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn activity_clamps_an_absurd_window() {
        let state = AppState::new().await.unwrap();
        let (status, body) = get_activity(state, "?days=100000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(body.get("start").is_some());

        let state = AppState::new().await.unwrap();
        let (status, _) = get_activity(state, "?days=-5").await;
        assert_eq!(status, axum::http::StatusCode::OK);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverge_invalid_session_id_returns_400() {
        let state = AppState::new().await.unwrap();
        // Contains a '.', disallowed by is_valid_session_id but a valid URI path
        // segment — no DB touched.
        let (status, _) = post_diverge(state, "bad.id", serde_json::json!({})).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverge_nonexistent_session_returns_404() {
        let state = AppState::new().await.unwrap();
        let (status, _) = post_diverge(state, "29990101_99999", serde_json::json!({})).await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverge_copies_full_history_and_keeps_original() {
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();

        let original = manager
            .create_session(
                PathBuf::from("/tmp/diverge_route_test"),
                "placeholder".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        // Give the original a name unique to this run (its own id) so the
        // sibling count is deterministic against the shared session store —
        // otherwise leftover "Route Original (branch N)" rows from prior runs
        // would bump the expected index.
        let base_name = format!("Route Original {}", original.id);
        manager
            .update(&original.id)
            .user_provided_name(base_name.clone())
            .apply()
            .await
            .unwrap();
        // A complete exchange (question + answer) so the branch — which is
        // trimmed to the last complete assistant answer — carries it over.
        manager
            .add_message(&original.id, &user_msg("hello"))
            .await
            .unwrap();
        manager
            .add_message(&original.id, &Message::assistant().with_text("hi there"))
            .await
            .unwrap();

        let expected_branch = format!("{base_name} (branch 1)");

        let (status, json) = post_diverge(state.clone(), &original.id, serde_json::json!({})).await;
        assert_eq!(status, axum::http::StatusCode::OK);

        let new_id = json["sessionId"].as_str().unwrap().to_string();
        assert_ne!(new_id, original.id);
        // Working dir surfaced and matches the original.
        assert_eq!(
            json["workingDir"].as_str().unwrap(),
            "/tmp/diverge_route_test"
        );
        // Response carries the branch name and lineage pointer.
        assert_eq!(json["name"].as_str().unwrap(), expected_branch);
        assert_eq!(json["divergedFrom"].as_str().unwrap(), original.id);

        // Branch has the full history; sibling-numbered branch name; lineage set.
        let branch = manager.get_session(&new_id, true).await.unwrap();
        assert_eq!(branch.message_count, 2);
        assert_eq!(branch.name, expected_branch);
        assert_eq!(branch.diverged_from.as_deref(), Some(original.id.as_str()));

        // Original is untouched (no lineage, history intact).
        let orig_after = manager.get_session(&original.id, true).await.unwrap();
        assert_eq!(orig_after.message_count, 2);
        assert_eq!(orig_after.diverged_from, None);

        // Cleanup so we don't pollute the shared session store.
        manager.delete_session(&new_id).await.unwrap();
        manager.delete_session(&original.id).await.unwrap();
    }

    /// Issue #56 §9.3 B1, at the route that actually ships it. The storage-level
    /// loop in `session::session_manager::tests::derived_session_carry_over`
    /// covers all three copy paths; this one pins the specific claim that the
    /// GUI's Diverge button — `POST /sessions/{id}/diverge` — is one of them, so
    /// a branch of a private chat can never be resumed against the user's
    /// default public model through `restore_provider_from_session`'s
    /// `Config::global()` fallback.
    ///
    /// It carries DR-19's `X-User-Action` because the SOURCE is private and this
    /// route now asks who is calling before it mints a private-capability
    /// branch. That is the *authorised* arm; the refusal arm, and the public
    /// source that needs no proof at all, are
    /// [`diverging_a_private_chat_needs_the_user_and_diverging_a_public_one_does_not`].
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverging_a_private_session_through_the_route_keeps_it_private() {
        use biorouter::model::ModelConfig;
        use biorouter::privacy::SessionClassification;

        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();

        let original = manager
            .create_session(
                PathBuf::from("/tmp/diverge_route_privacy"),
                "Private Original".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        manager
            .add_message(&original.id, &user_msg("patient MRN 12345"))
            .await
            .unwrap();
        manager
            .add_message(&original.id, &Message::assistant().with_text("noted"))
            .await
            .unwrap();
        manager
            .update(&original.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();

        let (status, json) = post_diverge_with(
            state.clone(),
            &original.id,
            serde_json::json!({}),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let new_id = json["sessionId"].as_str().unwrap().to_string();

        let branch = manager.get_session(&new_id, false).await.unwrap();
        assert_eq!(
            branch.privacy_tier,
            SessionClassification::Private,
            "the GUI diverge route dropped the tier"
        );
        assert_eq!(
            branch.provider_name.as_deref(),
            Some("versa_azure"),
            "the GUI diverge route dropped the bound provider"
        );
        assert!(
            branch.model_config.is_some(),
            "the GUI diverge route dropped the model config"
        );
        assert_eq!(
            branch.privacy_reason.as_deref(),
            Some(format!("diverged:{}", original.id).as_str())
        );

        manager.delete_session(&new_id).await.unwrap();
        manager.delete_session(&original.id).await.unwrap();
    }

    /// One source session for the DR-19 gate test: a complete exchange, under a
    /// name derived from its own id so the sibling branch index is deterministic
    /// against the shared session store. Returns `(id, base_name)`.
    async fn seed_dr19_source(
        manager: &biorouter::session::session_manager::SessionManager,
        label: &str,
    ) -> (String, String) {
        let session = manager
            .create_session(
                PathBuf::from("/tmp/diverge_route_dr19"),
                "placeholder".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let base = format!("{label} {}", session.id);
        manager
            .update(&session.id)
            .user_provided_name(base.clone())
            .apply()
            .await
            .unwrap();
        manager
            .add_message(&session.id, &user_msg("patient MRN 12345"))
            .await
            .unwrap();
        manager
            .add_message(&session.id, &Message::assistant().with_text("noted"))
            .await
            .unwrap();
        (session.id, base)
    }

    /// Issue #56 DR-19, the other half of the carry-over above.
    ///
    /// The copy carries `provider_name` and `model_config`, not just the tier —
    /// so diverging a private chat does not copy a *label*, it **mints a new
    /// session already bound to a private provider**. DR-16 guards raises on
    /// sessions that ALREADY exist (`raise_needs_user_action` compares a live
    /// agent's bound provider against a requested one), so a session born
    /// private passes no gate at all: it never calls `/agent/update_provider`,
    /// and every gate downstream then reads it as legitimately private. The
    /// route sits behind `check_token`, which AR-11 and AR-15 between them
    /// establish is not a proof of a human — so on the unamended implementation
    /// a caller holding nothing but the daemon secret could hand itself
    /// private capability by branching a private chat.
    ///
    /// A CONDITION on the SOURCE's tier, never an unconditional refusal: a
    /// public source mints no capability, and a gate that fires on every branch
    /// of every chat is one people route around (DR-19's user half). And the
    /// proof is Task 18A's `X-User-Action`, reused — there is exactly one proof
    /// of user in this feature and this is not a second.
    ///
    /// Both handlers, not just the one the GUI's Diverge button uses:
    /// `/edit_message` reaches `diverge_session_for_edit`, which performs the
    /// same carry-over, and a test written against `/diverge` alone would miss
    /// it — the same omission shape as `copy_session`-only coverage.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverging_a_private_chat_needs_the_user_and_diverging_a_public_one_does_not() {
        use biorouter::model::ModelConfig;
        use biorouter::privacy::SessionClassification;

        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();

        let (private_id, private_base) = seed_dr19_source(manager, "DR19 Private").await;
        let (public_id, _public_base) = seed_dr19_source(manager, "DR19 Public").await;

        manager
            .update(&private_id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();

        // 1. A private source, with nothing but the daemon secret: refused.
        let (status, _) =
            post_diverge_with(state.clone(), &private_id, serde_json::json!({}), None).await;
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "a secret-key-only caller minted itself a private-capability session"
        );

        // 2. The same request, carrying the user's proof: allowed. The branch
        // index is what shows the refusal above branched NOTHING — had it
        // created a session, this one would be `(branch 2)`.
        let (status, json) = post_diverge_with(
            state.clone(),
            &private_id,
            serde_json::json!({}),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "got {json}");
        let diverge_branch = json["sessionId"].as_str().unwrap().to_string();
        assert_eq!(
            json["name"].as_str().unwrap(),
            format!("{private_base} (branch 1)"),
            "the refused diverge branched something anyway"
        );
        assert_eq!(
            manager
                .get_session(&diverge_branch, false)
                .await
                .unwrap()
                .privacy_tier,
            SessionClassification::Private
        );

        // 3. A PUBLIC source needs no proof: copying a public chat mints no
        // capability, and a gate that fires on every branch is one people route
        // around.
        let (status, json) =
            post_diverge_with(state.clone(), &public_id, serde_json::json!({}), None).await;
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "the gate is a wall in front of the user, not a condition: {json}"
        );
        let public_branch = json["sessionId"].as_str().unwrap().to_string();

        // 4. `/edit_message`'s diverge arm is the second copy handler and takes
        // the same gate. `i64::MAX / 2` is past every message, so the branch
        // carries the whole conversation and this stays a test about the gate.
        let edit_body = serde_json::json!({
            "timestamp": i64::MAX / 2,
            "editType": "diverge",
        });
        let (status, _) =
            post_edit_message_with(state.clone(), &private_id, edit_body.clone(), None).await;
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "the edit_message copy path is ungated"
        );

        let (status, json) = post_edit_message_with(
            state.clone(),
            &private_id,
            edit_body,
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "got {json}");
        let edit_branch = json["sessionId"].as_str().unwrap().to_string();
        assert_ne!(edit_branch, private_id);
        assert_eq!(
            manager.get_session(&edit_branch, false).await.unwrap().name,
            format!("{private_base} (branch 2)"),
            "the refused edit_message diverge branched something anyway"
        );

        for id in [
            diverge_branch,
            public_branch,
            edit_branch,
            private_id,
            public_id,
        ] {
            manager.delete_session(&id).await.unwrap();
        }
    }

    /// Issue #56 DR-19. The renderer cannot read the 403: under `throwOnError`
    /// the generated client throws the parsed BODY, so the status never reaches
    /// the catch arm and a substring is all there is to go on. A reword that
    /// dropped the marker would put every refused branch back on the generic
    /// "Could not branch this conversation" with nothing failing.
    #[test]
    fn the_copy_refusal_carries_the_marker_the_renderer_keys_on() {
        assert!(
            COPY_OF_PRIVATE_NEEDS_USER.contains(COPY_OF_PRIVATE_REFUSAL_MARKER),
            "the renderer cannot tell this refusal from a 500: {COPY_OF_PRIVATE_NEEDS_USER}"
        );
        // And deliberately NOT the model picker's marker. The two refusals are
        // different acts with different remedies, one is "switch this chat's
        // model", the other is "branch this chat" — and a toast that answered
        // for both would tell the user to do the wrong thing.
        assert!(
            !COPY_OF_PRIVATE_NEEDS_USER
                .contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER),
            "the copy refusal is claiming to be the model picker's refusal"
        );
    }

    /// Issue #56 DR-19, the window the gate above leaves open.
    ///
    /// The gate reads the SOURCE's tier; `diverge_session*` then reads the
    /// source again inside the copy. Between those two reads Gate B's turn
    /// ratchet can raise the source — so a request that passed the gate on a
    /// public row walks out holding a private-capability child that no human
    /// ever proved. Two database reads wide and it takes racing a turn in the
    /// source chat, so it is not the likely path; "unlikely" is simply not the
    /// standard a capability gate is held to, and the tier the CHILD was born
    /// with is already in hand — `create_derived_session` copies it.
    ///
    /// The decision is a pure function so it can be pinned at every corner
    /// rather than only at the corner a race happens to produce.
    #[test]
    fn a_child_born_private_without_proof_is_undone() {
        use biorouter::privacy::SessionClassification::{Private, Public};

        assert!(
            minted_capability_without_proof(Private, false),
            "this is the race: the source went private between the gate and the copy"
        );
        assert!(
            !minted_capability_without_proof(Private, true),
            "the user's own branch of their own private chat is the supported act"
        );
        assert!(
            !minted_capability_without_proof(Public, false),
            "branching a public chat mints no capability and needs no proof; a gate \
             that fires on every branch is one people route around"
        );
        assert!(!minted_capability_without_proof(Public, true));
    }

    /// The body of the FIRST `fn <name>(` in `src`. Both handlers are defined
    /// well above this test module, so the first hit is the real one and not
    /// the literal in the test below.
    fn fn_body(src: &str, signature: &str) -> String {
        let start = src
            .find(signature)
            .unwrap_or_else(|| panic!("no `{signature}` in the file"));
        let onwards = src.get(start..).expect("find returns a char boundary");
        let open = onwards
            .find('{')
            .unwrap_or_else(|| panic!("no body for {signature}"));
        let body = onwards.get(open..).expect("find returns a char boundary");
        let mut depth = 0usize;
        for (offset, ch) in body.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return body
                            .get(..offset + ch.len_utf8())
                            .expect("char_indices yields char boundaries")
                            .to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced braces in {signature}");
    }

    /// Both copy handlers, not one. `/edit_message`'s diverge arm performs the
    /// same carry-over and a gate written against `/diverge` alone would miss
    /// exactly the half that shipped broken last time — which is the omission
    /// shape this whole task exists to close. A race is not drivable from a
    /// route test, so the structural assertion is what stands in for it.
    #[test]
    fn both_copy_handlers_take_the_second_read() {
        // `include_str!`, not `read_to_string("src/routes/session.rs")`. A
        // relative read resolves against the process's working directory, which
        // is ambient state no test owns: `cargo test` happens to set it to the
        // package root, so the relative form works under cargo and fails with
        // `NotFound` the moment the same binary is run any other way — under a
        // debugger, from a load harness, or from a wrapper that runs the binary
        // directly. That is the same class of bug as the session-store path this
        // module's note describes, and `include_str!` closes it by resolving at
        // compile time relative to THIS file.
        let src = include_str!("session.rs");
        for signature in ["async fn diverge_session(", "async fn edit_message("] {
            assert!(
                fn_body(src, signature).contains("minted_capability_without_proof"),
                "{signature} gates on the source it read before the copy and never \
                 re-checks the child it minted"
            );
        }
        // The negative control, without which `fn_body` returning the whole file
        // — or the wrong function — would satisfy the loop above vacuously.
        // `edit_in_place` truncates the LIVE session and copies nothing, so it
        // mints no capability and must not be carrying this gate.
        assert!(
            !fn_body(src, "async fn edit_in_place(").contains("minted_capability_without_proof"),
            "fn_body is not returning one function's body"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverge_with_custom_name_and_too_long_name() {
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();

        let original = manager
            .create_session(
                PathBuf::from("/tmp/diverge_route_name"),
                "Orig".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        // Custom name applied.
        let (status, json) = post_diverge(
            state.clone(),
            &original.id,
            serde_json::json!({ "name": "My Branch" }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let new_id = json["sessionId"].as_str().unwrap().to_string();
        let branch = manager.get_session(&new_id, false).await.unwrap();
        assert_eq!(branch.name, "My Branch");

        // Name exceeding the 200-char cap is rejected.
        let long_name = "x".repeat(201);
        let (status, _) = post_diverge(
            state.clone(),
            &original.id,
            serde_json::json!({ "name": long_name }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);

        manager.delete_session(&new_id).await.unwrap();
        manager.delete_session(&original.id).await.unwrap();
    }

    async fn get_running(state: Arc<AppState>) -> Vec<String> {
        let app = routes(state);
        let req = Request::builder()
            .method("GET")
            .uri("/sessions/running")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        json["session_ids"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// BR-71 / CLI parity: the daemon is the ONLY authority on liveness
    /// (`AppState::active_turns` is an in-process map), so it has to publish it.
    ///
    /// ⚠ **No session is created and none is deleted.** `try_begin_turn_idempotent`
    /// inserts into the in-memory `active_turns` map and never consults the store,
    /// so fabricated ids exercise the whole path — which keeps this test inside
    /// this module's READ-ONLY rule (`AppState::new()` opens the real user DB).
    ///
    /// `#[serial]` matches the rest of this module, whose tests share that real
    /// database — NOT because `active_turns` is shared. It is not:
    /// `AppState::new` allocates a fresh `Arc<StdMutex<HashMap<…>>>` per call
    /// (`state.rs`), so this test's map is its own. The ids are still stamped
    /// unique and the assertions still speak only about this test's own ids,
    /// because both are free and neither depends on that reading of the code
    /// being right.
    ///
    /// The load-bearing assertion is the LAST one. Everything before it is also
    /// satisfied by a route that snapshots the running set once at construction;
    /// only the post-`drop(guard)` check separates a live read from a snapshot.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn running_sessions_reports_exactly_the_sessions_holding_a_turn() {
        let state = AppState::new().await.unwrap();
        // ⚠ NOT `uuid::Uuid::new_v4()`: `uuid` is not a dependency of
        // `biorouter-server`, so that would be an unresolved-crate error. A
        // nanosecond stamp is unique enough for two ids in one test.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let busy = format!("parity-busy-{stamp}");
        let idle = format!("parity-idle-{stamp}");

        // A cheap precondition, not a strong one — this map starts empty. Kept
        // so the failure message names the offender if that ever stops holding.
        let before = get_running(state.clone()).await;
        assert!(!before.contains(&busy), "precondition: {before:?}");

        let guard = state
            .try_begin_turn_idempotent(&busy, CancellationToken::new(), None)
            .expect("nothing holds this fabricated session");

        let during = get_running(state.clone()).await;
        assert!(during.contains(&busy), "a held turn must be reported");
        assert!(
            !during.contains(&idle),
            "a session with no turn must not be reported running"
        );

        drop(guard);
        assert!(
            !get_running(state.clone()).await.contains(&busy),
            "TurnGuard::drop clears the slot, so the route must read LIVE state: \
             a snapshot taken at construction passes every assertion above and \
             fails this one"
        );
    }
}

/// #51 NF-D: `edit_message` with `EditType::Edit` deletes an arbitrary tail of a
/// LIVE session. These pin the preconditions that make that safe — the same
/// discipline `/reply` applies to `conversation_so_far`.
///
/// NOTE: `AppState::new()` opens the ONE shared session database, so each test
/// creates its own session and deletes it again, exactly like `diverge_tests`.
#[cfg(test)]
mod edit_message_tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use biorouter::conversation::message::Message;
    use biorouter::session::SessionType;
    use serial_test::serial;
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    async fn post_edit(
        state: Arc<AppState>,
        session_id: &str,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let app = routes(state);
        let req = Request::builder()
            .method("POST")
            .uri(format!("/sessions/{session_id}/edit_message"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    fn user_at(ts: i64, text: &str) -> Message {
        let mut message = Message::user().with_text(text);
        message.created = ts;
        message
    }

    fn assistant_at(ts: i64, text: &str) -> Message {
        let mut message = Message::assistant().with_text(text);
        message.created = ts;
        message
    }

    /// A session holding `u1@1000, a1@1010, u2@1020`, returning its id plus the
    /// three server-assigned message uids in stored order.
    async fn seeded_session(state: &Arc<AppState>, dir: &str) -> (String, Vec<String>) {
        let manager = state.session_manager();
        let session = manager
            .create_session(
                PathBuf::from(dir),
                "placeholder".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let mut ids = Vec::new();
        for message in [
            user_at(1_000, "first question"),
            assistant_at(1_010, "first answer"),
            user_at(1_020, "a message the client never saw"),
        ] {
            ids.push(manager.add_message(&session.id, &message).await.unwrap());
        }
        (session.id, ids)
    }

    async fn message_count(state: &Arc<AppState>, session_id: &str) -> usize {
        state
            .session_manager()
            .get_session(session_id, true)
            .await
            .unwrap()
            .conversation
            .map(|conversation| conversation.messages().len())
            .unwrap_or(0)
    }

    /// THE NF-D CASE. The client asks to cut from `1010`, naming only the two
    /// messages it has seen. A third message landed since — an append that was
    /// already acknowledged to whoever wrote it — and the open-above cut would
    /// take it too. The request must be refused with 409 and NOTHING deleted,
    /// exactly as a stale `conversation_so_far` is.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_refuses_a_client_view_missing_a_stored_message() {
        let state = AppState::new().await.unwrap();
        let (session_id, ids) = seeded_session(&state, "/tmp/edit_route_stale").await;

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({
                "timestamp": 1_010,
                "editType": "edit",
                // Only the first two — the client never saw `ids[2]`.
                "expectedMessageIds": [ids[0], ids[1]],
            }),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::CONFLICT,
            "a stale client view must be refused; got {body}"
        );
        assert_eq!(body["code"], "conversation_out_of_date");
        assert_eq!(
            body["missing_message_ids"],
            serde_json::json!([ids[2]]),
            "the 409 must name what the cut would have destroyed"
        );
        assert_eq!(body["stored_message_count"], serde_json::json!(3));

        assert_eq!(
            message_count(&state, &session_id).await,
            3,
            "a refused edit must write nothing"
        );

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// The other half of the guard: a client that HAS seen everything still gets
    /// its edit. Without this, "return 409" degenerates into "reject everyone".
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_accepts_a_client_view_that_has_seen_every_stored_message() {
        let state = AppState::new().await.unwrap();
        let (session_id, ids) = seeded_session(&state, "/tmp/edit_route_fresh").await;

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({
                "timestamp": 1_010,
                "editType": "edit",
                "expectedMessageIds": ids,
            }),
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK, "got {body}");
        assert_eq!(body["sessionId"], serde_json::json!(session_id));
        assert_eq!(
            message_count(&state, &session_id).await,
            1,
            "the tail from 1010 onwards is cut"
        );
        // The edit takes the BR-33 turn lock; leaking it would soft-lock the
        // session — every later `/reply` would 409 forever.
        assert!(
            !state.is_turn_active(&session_id),
            "the edit must release the turn lock it took"
        );

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// An `Edit` that supplies no view is NOT refused. The field landed
    /// required, and at the time no client could satisfy it — the reply stream
    /// did not publish the ids messages are persisted under — so requiring it
    /// broke "Edit in Place" in every live chat, which is a regression against
    /// the unguarded endpoint it replaced. #59 has since made the stream publish
    /// them, but a client only benefits if it consumes that frame, so the
    /// no-view path this covers is still the one every live chat takes and this
    /// test still guards a reachable case, not a historical one.
    /// Omitting it now falls back to the two
    /// guards that need nothing from the client: the turn lock (covered by
    /// `edit_without_an_expected_view_still_refuses_while_a_turn_is_in_flight`)
    /// and the bounded cut against the server's own snapshot.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_without_an_expected_view_still_cuts() {
        let state = AppState::new().await.unwrap();
        let (session_id, _) = seeded_session(&state, "/tmp/edit_route_unchecked").await;

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({ "timestamp": 1_010, "editType": "edit" }),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "an edit with no client view must proceed under the remaining guards; got {body}"
        );
        assert_eq!(body["sessionId"], serde_json::json!(session_id));
        assert_eq!(
            message_count(&state, &session_id).await,
            1,
            "the tail from 1010 onwards is cut"
        );
        assert!(
            !state.is_turn_active(&session_id),
            "the edit must release the turn lock it took"
        );

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// `null` is how a JSON client spells "not sending this", so it must land on
    /// the same fallback as omitting the key entirely — not on the checked path
    /// with an empty view.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_with_a_null_expected_view_is_the_same_as_omitting_it() {
        let state = AppState::new().await.unwrap();
        let (session_id, _) = seeded_session(&state, "/tmp/edit_route_null_view").await;

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({
                "timestamp": 1_010,
                "editType": "edit",
                "expectedMessageIds": serde_json::Value::Null,
            }),
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK, "got {body}");
        assert_eq!(message_count(&state, &session_id).await, 1);

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// THE EMPTY-VS-ABSENT DECISION. `[]` is a client asserting its view holds
    /// NOTHING — a claim, not the absence of one — so it goes down the checked
    /// path and a session with messages refuses it, naming every one of them.
    /// Folding `[]` into "absent" would hand any caller a one-token opt-out of
    /// the client-view guard, which would delete the guard rather than make it
    /// optional.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_with_an_empty_expected_view_is_checked_not_waived() {
        let state = AppState::new().await.unwrap();
        let (session_id, ids) = seeded_session(&state, "/tmp/edit_route_empty_view").await;

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({
                "timestamp": 1_010,
                "editType": "edit",
                "expectedMessageIds": [],
            }),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::CONFLICT,
            "an empty view of a non-empty session is a stale view; got {body}"
        );
        assert_eq!(body["code"], "conversation_out_of_date");
        assert_eq!(
            body["missing_message_ids"],
            serde_json::json!(ids),
            "every stored message is one the view does not name"
        );
        assert_eq!(
            message_count(&state, &session_id).await,
            3,
            "a refused edit must write nothing"
        );

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// The fallback is a fallback, not a bypass: dropping the client view does
    /// not drop the turn lock, which is the guard that stops a cut landing in a
    /// session an agent is generating into.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_without_an_expected_view_still_refuses_while_a_turn_is_in_flight() {
        let state = AppState::new().await.unwrap();
        let (session_id, _) = seeded_session(&state, "/tmp/edit_route_unchecked_live").await;

        let guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), None)
            .expect("no turn should be running for a session we just created");

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({ "timestamp": 1_010, "editType": "edit" }),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::CONFLICT,
            "a live turn must block an unchecked cut too; got {body}"
        );
        assert_eq!(body["code"], "turn_in_flight");
        assert_eq!(
            message_count(&state, &session_id).await,
            3,
            "a refused edit must write nothing"
        );

        drop(guard);

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// Truncating the tail of a session an agent is actively generating into
    /// deletes rows out from under the running turn. `/reply` already refuses a
    /// second writer with 409; the destructive edit takes the SAME lock.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn edit_refuses_while_a_turn_is_in_flight() {
        let state = AppState::new().await.unwrap();
        let (session_id, ids) = seeded_session(&state, "/tmp/edit_route_live_turn").await;

        let guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), None)
            .expect("no turn should be running for a session we just created");

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({
                "timestamp": 1_010,
                "editType": "edit",
                "expectedMessageIds": ids,
            }),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::CONFLICT,
            "a live turn must block the cut; got {body}"
        );
        assert_eq!(body["code"], "turn_in_flight");
        assert!(
            body["running_turn_id"].is_string(),
            "name the turn that holds the session: {body}"
        );
        assert_eq!(
            message_count(&state, &session_id).await,
            3,
            "a refused edit must write nothing"
        );

        drop(guard);

        // Releasing the turn releases the edit: the 409 is about the live turn,
        // not a permanent refusal.
        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({
                "timestamp": 1_010,
                "editType": "edit",
                "expectedMessageIds": ids,
            }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "got {body}");
        assert_eq!(message_count(&state, &session_id).await, 1);

        state
            .session_manager()
            .delete_session(&session_id)
            .await
            .unwrap();
    }

    /// The hardening is scoped to the DESTRUCTIVE path. `Diverge` copies into a
    /// brand-new session and never writes to the live one, so it keeps working
    /// with no client view and with a turn in flight — which is what keeps the
    /// default, and the button the desktop app actually uses most, unbroken.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn diverge_is_not_subject_to_the_edit_preconditions() {
        let state = AppState::new().await.unwrap();
        let (session_id, _) = seeded_session(&state, "/tmp/edit_route_diverge").await;

        let _guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), None)
            .expect("no turn should be running for a session we just created");

        let (status, body) = post_edit(
            state.clone(),
            &session_id,
            serde_json::json!({ "timestamp": 1_010, "editType": "diverge" }),
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK, "got {body}");
        let branch_id = body["sessionId"].as_str().unwrap().to_string();
        assert_ne!(branch_id, session_id);
        assert_eq!(
            message_count(&state, &session_id).await,
            3,
            "diverge leaves the original untouched"
        );

        let manager = state.session_manager();
        manager.delete_session(&branch_id).await.unwrap();
        manager.delete_session(&session_id).await.unwrap();
    }

    /// The field is optional on the wire (so `Diverge` bodies stay valid, and so
    /// an `Edit` from a client that cannot name its view still parses) and
    /// camelCase, matching every other request type in this module.
    ///
    /// The three states must stay distinct at the type level, because the
    /// handler treats `Some([])` (a view that holds nothing — checked) and
    /// `None` (no view — unchecked) differently.
    #[test]
    fn expected_message_ids_deserializes_from_camel_case() {
        let request: EditMessageRequest = serde_json::from_value(serde_json::json!({
            "timestamp": 1,
            "editType": "edit",
            "expectedMessageIds": ["a", "b"],
        }))
        .unwrap();
        assert_eq!(
            request.expected_message_ids.as_deref(),
            Some(["a".to_string(), "b".to_string()].as_slice())
        );

        let absent: EditMessageRequest =
            serde_json::from_value(serde_json::json!({ "timestamp": 1 })).unwrap();
        assert!(absent.expected_message_ids.is_none());

        // An empty list is a view, not the absence of one: it must survive
        // deserialization as `Some([])` so the handler can check it.
        let empty: EditMessageRequest = serde_json::from_value(serde_json::json!({
            "timestamp": 1,
            "editType": "edit",
            "expectedMessageIds": [],
        }))
        .unwrap();
        assert_eq!(empty.expected_message_ids.as_deref(), Some([].as_slice()));

        // ... and an explicit `null` is the absence of one, like omitting it.
        let null: EditMessageRequest = serde_json::from_value(serde_json::json!({
            "timestamp": 1,
            "editType": "edit",
            "expectedMessageIds": serde_json::Value::Null,
        }))
        .unwrap();
        assert!(null.expected_message_ids.is_none());
    }
}

/// Issue #56 §12.4/§12.5 — `POST /sessions/{id}/declassify`.
#[cfg(test)]
mod declassify_tests {
    use super::diverge_tests::{install_test_user_action_key, TEST_USER_ACTION_KEY};
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    // The phrase lives beside the grading it belongs to, in the writer's module:
    // the daemon's check now happens inside the writing transaction, so the
    // route has nothing left to derive it for. Its own unit tests moved with it.
    use biorouter::privacy::declassify::confirmation_phrase;
    use biorouter::session::SessionType;
    use serial_test::serial;
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// The server secret this test binary's "daemon" was launched with. Unlike
    /// the user-action digest it is not a process global — `check_token` takes it
    /// as layer state — so it can simply be a constant.
    const TEST_SECRET: &str = "task-29-declassify-route-secret";

    /// `POST /sessions/{id}/declassify` through the SAME `check_token` layer
    /// `commands::agent::run` installs in front of the real router.
    ///
    /// Layering it here rather than calling `routes(state)` bare is what makes
    /// the 401 arm mean anything: `routes()` alone is unauthenticated, so a test
    /// against it would assert that a route nobody is guarding lets everyone
    /// through.
    async fn post_declassify(
        state: Arc<AppState>,
        session_id: &str,
        body: serde_json::Value,
        secret: Option<&str>,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let app = routes(state).layer(axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ));
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/sessions/{session_id}/declassify"))
            .header("content-type", "application/json");
        if let Some(key) = secret {
            builder = builder.header("X-Secret-Key", key);
        }
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let req = builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Arm DR-20's system-authentication seam for the next prompt.
    ///
    /// ⚠ **This compiles only because `biorouter` is a `[dev-dependency]` of
    /// this crate with `privacy-test-auth` on.** Dropping that dev-dependency
    /// stops this line compiling — a loud failure — rather than leaving a test
    /// suite that asks the developer for their password on every run. Moving it
    /// into `[dependencies]` would ship the bypass, which
    /// `privacy::system_auth::tests::the_test_seam_cannot_be_compiled_into_a_shipped_profile`
    /// turns red.
    fn arm_the_system_prompt(outcome: biorouter::privacy::system_auth::AuthOutcome) {
        biorouter::privacy::system_auth_seam::reset();
        biorouter::privacy::system_auth_seam::answer_next_prompt(outcome);
    }

    /// A private session with one message, `versa_azure` bound, and `reason` as
    /// its recorded provenance.
    async fn seed_private(
        manager: &biorouter::session::session_manager::SessionManager,
        reason: &str,
    ) -> String {
        let session = manager
            .create_session(
                PathBuf::from("/tmp/declassify_route"),
                "Declassify Source".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        manager
            .add_message(&session.id, &Message::user().with_text("patient MRN 12345"))
            .await
            .unwrap();
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, reason)
            .apply()
            .await
            .unwrap();
        session.id
    }

    /// §9.3 A1: the secret is reachable from any developer-enabled agent shell,
    /// so `X-Secret-Key` alone is not a human.
    ///
    /// Note that a test asserting the route is not in the public-GET exemption
    /// list would be VACUOUSLY true — `is_public_app_get` only matches GETs under
    /// `/apps/{id}` with an explicit tail allowlist, and can never match a POST
    /// under `/sessions`. The assertion that carries weight is this one: the same
    /// request, three credential sets, three different answers.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_route_needs_more_than_the_secret_key() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();
        // `turn:*`, so §12.4 grades this chat onto the single-click control and
        // the credential is the only variable under test.
        let id = seed_private(manager, "turn:versa_azure").await;

        let (status, _) =
            post_declassify(state.clone(), &id, serde_json::json!({}), None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = post_declassify(
            state.clone(),
            &id,
            serde_json::json!({}),
            Some(TEST_SECRET),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a secret-key-only caller lowered a chat's classification"
        );
        assert!(
            body.contains("Do not retry"),
            "the refusal must foreclose the retry: {body}"
        );
        assert_eq!(
            manager.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Private,
            "the refused call changed the row anyway"
        );

        let (status, body) = post_declassify(
            state.clone(),
            &id,
            serde_json::json!({}),
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "got {body}");
        let after = manager.get_session(&id, false).await.unwrap();
        assert_eq!(after.privacy_tier, SessionClassification::Public);
        assert_eq!(
            after.privacy_reason.as_deref(),
            Some("declassified_by_user")
        );

        manager.delete_session(&id).await.unwrap();
    }

    /// §12.4's graded confirmation, enforced by the DAEMON and not merely by the
    /// dialog. A client that renders the single-click control for a chat that
    /// reached a private data source — or a caller that skips the renderer
    /// entirely — gets the same answer the dialog would have insisted on.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn an_mcp_chat_needs_the_typed_phrase_and_a_turn_only_chat_does_not() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();
        let id = seed_private(manager, "mcp:ucsfomopagent").await;
        let phrase = confirmation_phrase(&id);

        // No phrase: refused, and the row is untouched.
        let (status, _) = post_declassify(
            state.clone(),
            &id,
            serde_json::json!({}),
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // The WRONG phrase — the same length, drawn from the same id — is refused
        // too, so this cannot pass merely because something was typed.
        let wrong: String = phrase.chars().rev().collect();
        if wrong != phrase {
            let (status, _) = post_declassify(
                state.clone(),
                &id,
                serde_json::json!({ "confirmation": wrong }),
                Some(TEST_SECRET),
                Some(TEST_USER_ACTION_KEY),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        assert_eq!(
            manager.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Private
        );

        // Since Task 55 the strong control also needs DR-20's system
        // authentication, so the phrase alone no longer writes.
        arm_the_system_prompt(biorouter::privacy::system_auth::AuthOutcome::Approved);
        let (status, body) = post_declassify(
            state.clone(),
            &id,
            serde_json::json!({ "confirmation": phrase }),
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "got {body}");
        assert_eq!(
            manager.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Public
        );

        manager.delete_session(&id).await.unwrap();
    }

    /// Issue #56 DR-20 / Task 55. The route asks the operating system as well as
    /// the two things it already asked for, and a refusal there leaves the chat
    /// private with nothing in the ledger.
    ///
    /// ⚠ **The failure worth testing is a route that reports success because the
    /// request was well-formed.** So every assertion below checks the *row*
    /// after the answer, not only the status code.
    ///
    /// The "and writes no audit row" half of Task 55 Step 4 is asserted at the
    /// writer — `privacy::declassify::tests::a_chat_that_reached_a_private_data_source_needs_the_password_as_well_as_the_phrase`
    /// — because `SessionStorage::pool` is `pub(crate)` and this crate has no
    /// SQL access at all. That is the right place for it: the writer is the only
    /// statement in the tree that inserts into `classification_audit`, pinned by
    /// `exactly_one_statement_in_the_tree_assigns_a_public_classification`, so a
    /// route cannot write a ledger row without going through the function that
    /// test covers.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_refused_system_authentication_leaves_the_chat_private_and_writes_no_audit_row() {
        use biorouter::privacy::system_auth::AuthOutcome;

        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();
        let id = seed_private(manager, "mcp:ucsfomopagent").await;
        let phrase = confirmation_phrase(&id);

        // Both the user's "no" and a platform with no prompter at all. The
        // second is DR-24's stated posture and is the state of every Linux
        // install until the packaging ships the polkit action, so it has to
        // refuse rather than fall through.
        for outcome in [AuthOutcome::Denied, AuthOutcome::Unavailable] {
            arm_the_system_prompt(outcome);
            let (status, body) = post_declassify(
                state.clone(),
                &id,
                serde_json::json!({ "confirmation": phrase }),
                Some(TEST_SECRET),
                Some(TEST_USER_ACTION_KEY),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{outcome:?}: {body}");
            let after = manager.get_session(&id, false).await.unwrap();
            assert_eq!(
                after.privacy_tier,
                SessionClassification::Private,
                "{outcome:?} declassified the chat anyway"
            );
            assert_eq!(
                after.privacy_reason.as_deref(),
                Some("mcp:ucsfomopagent"),
                "{outcome:?} rewrote the provenance of a chat it did not declassify"
            );
        }

        // The prompt names the chat it authorises (DR-20 point 4) — a dialog
        // that said "BioRouter wants to make changes" would satisfy the letter
        // of the ruling and defeat its purpose.
        assert_eq!(
            biorouter::privacy::system_auth_seam::last_request()
                .expect("the prompter was reached")
                .session_ids,
            vec![id.clone()]
        );

        arm_the_system_prompt(AuthOutcome::Approved);
        let (status, body) = post_declassify(
            state.clone(),
            &id,
            serde_json::json!({ "confirmation": phrase }),
            Some(TEST_SECRET),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "got {body}");
        assert_eq!(
            manager.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Public
        );

        manager.delete_session(&id).await.unwrap();
    }

    /// The idempotency `DeclassifySessionResponse` documents, on the path that
    /// actually needs it.
    ///
    /// That doc promises the already-public case is a 200 "so a double-clicked
    /// confirm button does not surface an error". It was only true on the typed
    /// path, which resends its phrase: a successful call rewrites the provenance
    /// to `declassified_by_user`, which `requires_typed_confirmation` grades onto
    /// the STRONG control, so the second of two single-click requests arrived
    /// carrying no confirmation, was refused at the phrase check over a field
    /// the user was never shown, and never reached the already-public arm at
    /// all. An already-public row is a no-op, so it is answered before the
    /// confirmation is consulted — there is nothing to confirm.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_second_single_click_confirm_is_a_success_not_a_refusal() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let manager = state.session_manager();
        let id = seed_private(manager, "turn:versa_azure").await;

        for attempt in 1..=2 {
            let (status, body) = post_declassify(
                state.clone(),
                &id,
                serde_json::json!({}),
                Some(TEST_SECRET),
                Some(TEST_USER_ACTION_KEY),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "attempt {attempt} answered {body}");
        }
        assert_eq!(
            manager.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Public
        );

        manager.delete_session(&id).await.unwrap();
    }

    /// The two refusals this route can emit are distinguishable from each other
    /// and from the two the renderer already keys on.
    ///
    /// Borrowing either marker would fire a toast whose remedy is wrong here:
    /// the model picker's says *switch this chat's model*, the copy handler's
    /// says *branch it from the chat window*, and neither marks a chat public.
    #[test]
    fn the_refusals_say_different_things() {
        let all = [
            DECLASSIFY_NEEDS_USER,
            DECLASSIFY_CONFIRMATION_MISMATCH,
            DECLASSIFY_SYSTEM_AUTH_REFUSED,
        ];
        for (i, one) in all.iter().enumerate() {
            for other in &all[i + 1..] {
                assert_ne!(one, other, "two of this route's refusals are the same text");
            }
        }
        for message in all {
            assert!(
                !message.contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER),
                "this refusal is claiming to be the model picker's: {message}"
            );
            assert!(
                !message.contains(COPY_OF_PRIVATE_REFUSAL_MARKER),
                "this refusal is claiming to be the copy handler's: {message}"
            );
        }
        // The model-facing one forecloses the retry; the human-facing one has no
        // model audience and does not need to.
        assert!(DECLASSIFY_NEEDS_USER.contains("Do not retry"));
    }

    /// This route's refusal may not tell a chat why it is private, because it
    /// deliberately never read the row.
    ///
    /// It shipped opening "This chat reached a private data source", which is
    /// §12.4's rationale for the strong control and not a fact about every chat
    /// that reaches this arm: `backfill:*` — most of the private rows on day one
    /// — was marked from the bound provider, and `imported` arrived marked. The
    /// only honest sentence here is the catch-all clause the grading module
    /// hands an unrecognised provenance, and this is what stops the two from
    /// drifting apart in different crates.
    #[test]
    fn the_system_auth_refusal_claims_only_what_the_record_says() {
        let catch_all = biorouter::privacy::declassify::strong_confirmation_reason(None)
            .expect("an absent provenance owes the strong control, so it has a clause");
        assert!(
            DECLASSIFY_SYSTEM_AUTH_REFUSED.contains(catch_all),
            "this route's refusal ({DECLASSIFY_SYSTEM_AUTH_REFUSED}) no longer says what the \
             grading module says about a chat whose provenance is unknown ({catch_all})"
        );

        // The specific claim it must not make, taken from the grading module
        // rather than typed out here, so a reworded clause is still caught.
        let only_true_of_mcp =
            biorouter::privacy::declassify::strong_confirmation_reason(Some("mcp:ucsfomopagent"))
                .expect("an `mcp:*` chat owes the strong control");
        assert_ne!(
            only_true_of_mcp, catch_all,
            "the grading module has collapsed its clauses into one, so this route cannot tell \
             the honest sentence from the false one"
        );
        assert!(
            !DECLASSIFY_SYSTEM_AUTH_REFUSED.contains(only_true_of_mcp),
            "this route never read the row, so it cannot say {only_true_of_mcp:?}: it is false \
             for `backfill:*` and `imported`, which dominate day one"
        );
    }
}

/// Deleting a chat stops its turn.
///
/// This used to hold by accident, and the accident is exactly what the live turn
/// stream removed: the UI unmounted the deleted chat, its SSE socket dropped,
/// the next `tx.send` failed, and the handler cancelled the turn. A turn now
/// outlives its listeners by design, so without an explicit cancel here one kept
/// running for up to the five-minute orphan timeout — spending tokens and
/// writing into a session that no longer exists.
#[cfg(test)]
mod delete_session_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use biorouter::session::SessionType;
    use serial_test::serial;
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn deleting_a_session_cancels_the_turn_it_was_running() {
        let state = AppState::new().await.unwrap();
        let session = state
            .session_manager()
            .create_session(
                PathBuf::from("/tmp/delete_cancels_turn"),
                "delete-cancels-turn".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        // A turn in flight, with the token `/agent/cancel` would trip.
        let cancel = CancellationToken::new();
        let _guard = state
            .try_begin_turn_idempotent(&session.id, cancel.clone(), Some("t-1".into()))
            .unwrap();
        assert!(!cancel.is_cancelled());

        let response = routes(state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/sessions/{}", session.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            cancel.is_cancelled(),
            "the deleted session's turn kept running, writing into a session that \
             no longer exists"
        );
    }
}
