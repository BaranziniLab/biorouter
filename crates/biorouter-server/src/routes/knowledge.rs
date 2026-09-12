use axum::{
    body::Body,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
    routing::{get, post, put},
    Json, Router,
};
use biorouter::knowledge::ProviderCompleter;
use biorouter::model::ModelConfig;
use biorouter_mcp::knowledge::{
    convert,
    macros::{ingest as ingest_macro, lint as lint_macro, query as query_macro},
    merge::{MergeAuthority, MergeReport, UserKbMerge},
    paths,
    service::{KnowledgeService, PrimaryUpdate, ReadPageError},
    source_paths, store,
    subagent::{events::SubAgentEvent, loop_::SubAgentBounds},
    tier,
    tier_user::UserKbTierChange,
    types::{Credibility, Graph, HistoryEntry, KbFormat, KbTier, Manifest, ModelRef},
};
// Issue #56 DR-16/DR-18. `src/routes/` is compiled into the `biorouterd` binary
// as well as the lib and cannot name `crate::auth`, so this is the shared
// direction — the same import `routes::session` uses for the declassify route.
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

/// Every route that names a knowledge base by `{id}`, and nothing else —
/// **ungated**, because the one place that consumes it applies
/// [`session_reach::gate_knowledge_base`](crate::routes::session_reach::gate_knowledge_base)
/// to the value it returns (issue #56, QA 2026-09-10 H2).
///
/// ⚠ **`Router::route_layer` is a SNAPSHOT, not a rule the router keeps.** It
/// consumes the routes present *at the moment it is called* and returns a map of
/// wrapped ones; a route registered afterwards is not wrapped, silently. The doc
/// on this pair used to claim that a route "added to it later" was gated, which
/// is not something axum offers — and because the `.route_layer(...)` sat last
/// in this chain, the natural way to add a route (append one more `.route(…)`)
/// produced an **ungated** `/bases/{id}` route that looked right.
///
/// So the layer is no longer part of the chain. Appending a `.route(…)` here —
/// anywhere, including after the last one — is gated, because the gate is
/// applied to whatever this function returns. Appending to the *call site*
/// instead is visibly outside the gate, which is the point: the mistake is now
/// one you can see.
///
/// Pinned from two directions by
/// `every_route_that_names_a_base_is_inside_the_gated_sub_router` — which reads
/// this file's own source rather than trusting the sentence above — and by
/// `base_addressing_routes`, whose probe list must cover every route registered
/// here and which the H2 tests drive against a real private base.
fn base_routes() -> Router<Arc<KnowledgeService>> {
    Router::new()
        .route(
            "/bases/{id}",
            get(get_base).put(update_base).delete(delete_base),
        )
        .route("/bases/{id}/tier", get(get_kb_tier).post(set_kb_tier))
        .route("/bases/{id}/default-model", put(set_default_model))
        .route("/bases/{id}/graph", get(get_graph))
        .route("/bases/{id}/location", get(get_location))
        .route("/bases/{id}/page", get(get_page_body))
        .route("/bases/{id}/pages", get(list_pages))
        .route(
            "/bases/{id}/pages/{*page_path}",
            get(read_page).put(write_page),
        )
        .route("/bases/{id}/history", get(list_history))
        .route("/bases/{id}/preview", post(preview_state))
        .route("/bases/{id}/restore", post(restore_state))
        .route("/bases/{id}/raw", post(add_raw_source))
        .route("/bases/{id}/ingest", post(ingest))
        .route("/bases/{id}/ingest-conversation", post(ingest_conversation))
        .route("/bases/{id}/query", post(query_kb))
        .route("/bases/{id}/lint", post(lint))
        .route("/bases/{id}/export", get(export_brkb))
        .route("/bases/{id}/merge", post(merge_bases))
        .route("/bases/{id}/sources/{sid}/reclassify", post(reclassify))
        .route(
            "/bases/{id}/sources/{sid}/credibility",
            put(override_credibility),
        )
}

/// Build the knowledge router.  The router owns an `Arc<KnowledgeService>` directly so
/// it can be tested without constructing a full `AppState`.
///
/// ⚠ **No route registered on THIS router may name a base by `{id}`** — those
/// live in [`base_routes`], which is gated on the line below. A `{id}` route
/// added here is ungated, and
/// `every_route_that_names_a_base_is_inside_the_gated_sub_router` fails when one
/// is.
pub fn router(svc: Arc<KnowledgeService>) -> Router {
    Router::new()
        .route("/bases", get(list_bases).post(create_base))
        .route(
            "/bases/import",
            post(import_brkb).layer(DefaultBodyLimit::max(
                biorouter_mcp::knowledge::brkb::MAX_ARCHIVE_HTTP_BODY_BYTES,
            )),
        )
        .route("/expand-path", post(expand_path))
        .route("/active", get(get_active).post(set_active))
        .route("/check-model", post(check_model))
        // The gate is applied HERE, to the whole of `base_routes()`, rather than
        // inside it — see that function for why the position is load-bearing.
        .merge(
            base_routes().route_layer(axum::middleware::from_fn_with_state(
                svc.clone(),
                crate::routes::session_reach::gate_knowledge_base,
            )),
        )
        .with_state(svc)
}

const MAX_INGEST_FILE_BYTES: usize = 25 * 1024 * 1024;
const MAX_INGEST_CSV_BYTES: usize = 8 * 1024 * 1024;

fn ingest_upload_limit(filename: &str) -> usize {
    let lower = filename.to_lowercase();
    // Tabular formats share the tighter cap: their markdown-table expansion is
    // much larger than the file itself.
    if [".csv", ".xlsx", ".xlsm", ".xls", ".ods"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        MAX_INGEST_CSV_BYTES
    } else {
        MAX_INGEST_FILE_BYTES
    }
}

fn validate_ingest_upload(filename: &str, size: usize) -> Result<(), (StatusCode, String)> {
    let lower = filename.to_lowercase();

    if lower.ends_with(".brkb") {
        return Err((
            StatusCode::BAD_REQUEST,
            "'.brkb' archives must be imported via the knowledge-base import action, not the digest dropzone"
                .to_string(),
        ));
    }

    if lower.ends_with(".ppt") {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{filename} is a legacy PowerPoint binary. Re-save it as .pptx and ingest that instead."
            ),
        ));
    }

    if [
        ".exe", ".app", ".pkg", ".dmg", ".msi", ".dll", ".dylib", ".so", ".bin", ".zip",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{filename} looks like an executable, binary, or packaged archive. Digest readable source material instead."
            ),
        ));
    }

    let limit = ingest_upload_limit(filename);
    if size > limit {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("{filename} is too large for ingestion ({size} bytes > {limit} byte limit)"),
        ));
    }

    Ok(())
}

#[derive(Debug)]
struct MultipartUpload {
    bytes: Vec<u8>,
    filename: String,
    mime: Option<String>,
}

fn sanitize_part_mime(mime: Option<&str>) -> Option<String> {
    mime.map(str::trim)
        .filter(|mime| !mime.is_empty())
        .map(ToOwned::to_owned)
}

async fn finish_macro_stream(
    sse_tx: mpsc::Sender<String>,
    mut result_rx: mpsc::Receiver<Result<serde_json::Value, String>>,
    macro_handle: JoinHandle<()>,
) {
    match result_rx.recv().await {
        Some(Ok(result_json)) => {
            let data = serde_json::to_string(&result_json).unwrap_or_default();
            let _ = sse_tx.send(format!("event: done\ndata: {data}\n\n")).await;
        }
        Some(Err(msg)) => {
            let _ = sse_tx.send(sse_error_frame(&msg)).await;
        }
        None => {
            let msg = match macro_handle.await {
                Ok(()) => "macro task ended without result".to_string(),
                Err(join_err) if join_err.is_cancelled() => {
                    "macro task was cancelled before producing a result".to_string()
                }
                Err(join_err) => format!("macro task failed: {join_err}"),
            };
            let _ = sse_tx.send(sse_error_frame(&msg)).await;
        }
    }
}

async fn forward_macro_stream(
    sse_tx: mpsc::Sender<String>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<SubAgentEvent>,
    result_rx: mpsc::Receiver<Result<serde_json::Value, String>>,
    macro_handle: JoinHandle<()>,
    cancel: CancellationToken,
) {
    let mut macro_handle = Some(macro_handle);
    loop {
        tokio::select! {
            biased;
            () = sse_tx.closed() => {
                cancel.cancel();
                let _ = macro_handle.take().expect("macro handle is present").await;
                return;
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                if let Ok(json) = serde_json::to_string(&event) {
                    if sse_tx.send(format!("data: {json}\n\n")).await.is_err() {
                        cancel.cancel();
                        let _ = macro_handle.take().expect("macro handle is present").await;
                        return;
                    }
                }
            }
        }
    }
    finish_macro_stream(
        sse_tx,
        result_rx,
        macro_handle.take().expect("macro handle is present"),
    )
    .await;
}

fn ingest_bounds() -> SubAgentBounds {
    SubAgentBounds {
        max_steps: 60,
        max_wall: Duration::from_secs(900),
        max_tokens: 200_000,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Request / response DTOs
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct CreateBaseBody {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    /// Which profile the new base is written in: `okf` (the default) or
    /// `biookf`. It decides the scaffolded tree and the `schema.md` the
    /// sub-agent is taught from, and it cannot be changed afterwards — DR-22
    /// defers format migration and DR-26 says so in as many words.
    ///
    /// ⚠ **A `String` parsed by hand, not a `KbFormat`, for the reason
    /// `kb_create_base`'s parameter is** (Stage 4). `KbFormat`'s own
    /// `Deserialize` is deliberately lenient because DR-12 traces what a
    /// `manifest.yaml` that fails to load costs the user — so an unknown
    /// profile *on disk* reads as plain OKF rather than destroying their
    /// pointers. That is the right reading of a file already written and the
    /// wrong reading of a request: a caller that asks for `bio-okf` and
    /// silently receives a plain-OKF base has been handed the opposite of what
    /// it asked for, and cannot convert. DR-7's rule — producers are held to a
    /// higher bar than consumers — applied to the HTTP surface.
    ///
    /// `schema(value_type)` keeps the published contract, and therefore the
    /// generated TypeScript, an enum of exactly the two words, so the strict
    /// parse below is the backstop and not the first line of defence.
    #[serde(default)]
    #[schema(value_type = Option<KbFormat>)]
    pub format: Option<String>,
}

impl CreateBaseBody {
    /// The requested profile, or a 400 naming the two that exist.
    fn format(&self) -> Result<KbFormat, (StatusCode, String)> {
        let raw = self.format.as_deref().map(str::trim).unwrap_or_default();
        if raw.is_empty() {
            return Ok(KbFormat::default());
        }
        KbFormat::parse(raw).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!(
                    "unknown knowledge base format {raw:?}: use \"{}\" for general-purpose \
                     knowledge or \"{}\" for biomedical knowledge under the BioOKF controlled \
                     vocabulary",
                    KbFormat::Okf.as_str(),
                    KbFormat::Biookf.as_str(),
                ),
            )
        })
    }
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateBaseBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetDefaultModelBody {
    #[serde(default)]
    pub model: Option<ModelRef>,
}

#[derive(Deserialize, ToSchema)]
pub struct ListPagesQuery {
    #[serde(default)]
    pub path_prefix: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct WritePageBody {
    pub content: String,
    pub commit_message: String,
}

#[derive(Serialize, ToSchema)]
pub struct CommitResponse {
    pub commit_sha: String,
}

#[derive(Deserialize, ToSchema)]
pub struct HistoryQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize {
    50
}

#[derive(Deserialize, ToSchema)]
pub struct PreviewBody {
    pub commit_sha: String,
    pub path: String,
}

#[derive(Serialize, ToSchema)]
pub struct PreviewResponse {
    pub content: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct RestoreBody {
    pub commit_sha: String,
}

#[derive(Serialize, ToSchema)]
pub struct RestoreResponse {
    pub new_commit_sha: String,
}

// Task 8 DTOs
#[derive(Serialize, ToSchema)]
pub struct RawSourceResponse {
    pub source_id: String,
    pub source_md_path: String,
}

// Task 11 DTOs
#[derive(Serialize, ToSchema)]
pub struct CredibilityResponse {
    pub credibility: Credibility,
}

// check-model DTOs
#[derive(Deserialize, ToSchema)]
pub struct CheckModelBody {
    pub model: ModelRef,
}

#[derive(Serialize, ToSchema)]
pub struct CheckModelResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ExpandPathBody {
    pub path: String,
}

#[derive(Serialize, ToSchema)]
pub struct ExpandPathFile {
    pub path: String,
    pub name: String,
    pub relative_path: String,
}

#[derive(Serialize, ToSchema)]
pub struct ExpandPathWarning {
    pub level: String,
    pub title: String,
    pub message: String,
}

#[derive(Serialize, ToSchema)]
pub struct ExpandPathResponse {
    pub files: Vec<ExpandPathFile>,
    pub warnings: Vec<ExpandPathWarning>,
}

// Task 9 DTOs
#[derive(Deserialize, ToSchema)]
pub struct IngestBody {
    pub source: serde_json::Value,
    pub model: ModelRef,
    #[serde(default)]
    pub focus: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct QueryBody {
    pub question: String,
    pub model: ModelRef,
    #[serde(default)]
    pub file_as_page: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct IngestConversationBody {
    /// Session ids to digest. At least one is required.
    pub session_ids: Vec<String>,
    pub model: ModelRef,
    #[serde(default)]
    pub focus: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct LintBody {
    pub model: ModelRef,
    #[serde(default)]
    pub autofix: Option<bool>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 5: read-only routes (list / create / get / delete / graph)
// ──────────────────────────────────────────────────────────────────────────────

/// One row of `GET /knowledge/bases`: the stored manifest plus the privacy tier
/// (issue #56).
///
/// The tier is **flattened alongside** the manifest rather than added to it,
/// because `manifest.yaml` is the on-disk record and the tier lives in
/// `.kb-tiers`. A `tier` field on [`Manifest`] would be persisted by the next
/// `manifest::save` and become a second, staler answer to a question the tier
/// store already answers — and it would also appear on `kb_list_bases`, a
/// model-facing tool whose payload Task 10D's metadata register governs.
///
/// ⚠ **"The renderer is the only caller" was this doc's premise, and QA
/// measured it false on 2026-09-10 (H2):** a public chat's shell recovered the
/// daemon secret and read this list, private bases included. So the rows are
/// now the bases the caller could open — the desktop app, which sends the
/// user's proof, still sees every one, with its tier — and a private base is
/// OMITTED for anyone else, as Task 10C already omits it from the model's own
/// listing: a base's id and name are user-authored content.
#[derive(Serialize, ToSchema)]
pub struct KbListEntry {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub tier: KbTier,
}

#[utoipa::path(
    get, path = "/knowledge/bases",
    responses((status = 200, description = "The knowledge bases this caller may open: every base \
                                            for the desktop app (the user-action proof) or a \
                                            caller stating a private provider, the public ones \
                                            for anyone else. A private base is omitted, never \
                                            redacted.", body = Vec<KbListEntry>))
)]
pub async fn list_bases(
    State(svc): State<Arc<KnowledgeService>>,
    headers: HeaderMap,
) -> Result<Json<Vec<KbListEntry>>, (StatusCode, String)> {
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    let bases = svc
        .list_bases()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        bases
            .into_iter()
            .filter(|manifest| {
                caller
                    .reach_knowledge_base(svc.root(), &manifest.id)
                    .is_ok()
            })
            .map(|manifest| KbListEntry {
                tier: tier::entry(svc.root(), &manifest.id).tier,
                manifest,
            })
            .collect(),
    ))
}

// `POST /knowledge/bases` — mint a base.
//
// ⚠ **Gated, and gated on the namespace rather than on `body.id`** (adversarial
// security review 2026-09-12, MEDIUM). Create refuses an id that is taken, so
// before this it was an existence oracle for exactly the ids
// `KNOWLEDGE_BASE_OUT_OF_REACH` exists to withhold: a secret-only caller POSTed
// a guessed id and read `400 kb '<id>' already exists at <the machine's absolute
// knowledge path>` when a **private** base had it, and `200` when nothing did.
// KB ids are user-authored names, so a short dictionary enumerated the private
// bases on the machine by name — with the path as a bonus.
//
// `HttpCaller::mints_knowledge_base` takes no id, which is what makes the answer
// the same for every id, including one that does not exist. See its doc for what
// the refusal costs.
//
// Deliberately `//` and not `///`: utoipa publishes a doc comment here as the
// operation's `description`, and this is a note to the next engineer rather than
// API reference for a client. The wire contract is in the `responses` below.
#[utoipa::path(
    post, path = "/knowledge/bases",
    request_body = CreateBaseBody,
    responses(
        (status = 200, description = "Created knowledge base", body = Manifest),
        (status = 400, description = "Duplicate id, invalid id, or unknown format"),
        (status = 403, description = "This caller may not mint a knowledge-base id: it is a \
                                      public model and the request carried no proof it came from \
                                      the person at the keyboard. The same answer for every id, \
                                      taken or free, so that creating is not a way to ask which \
                                      private bases exist (body = plain text)"),
    )
)]
pub async fn create_base(
    State(svc): State<Arc<KnowledgeService>>,
    headers: HeaderMap,
    Json(body): Json<CreateBaseBody>,
) -> Result<Json<Manifest>, (StatusCode, String)> {
    crate::routes::session_reach::http_caller(&headers)
        .await
        .mints_knowledge_base()
        .map_err(|refusal| (refusal.status, refusal.message.to_string()))?;
    // Refused before anything is created: `create_base_in` writes the manifest,
    // the scaffolded tree and `schema.md` in one transaction precisely because
    // those are three statements about one base, and a request this route
    // cannot read must not reach it half-answered.
    let format = body.format()?;
    let m = svc
        .create_base_in(&body.id, &body.name, body.color.as_deref(), format)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(m))
}

#[utoipa::path(
    get, path = "/knowledge/bases/{id}",
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Knowledge base manifest", body = Manifest),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_base(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
) -> Result<Json<Manifest>, (StatusCode, String)> {
    svc.get_base(&id)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

#[utoipa::path(
    put, path = "/knowledge/bases/{id}",
    request_body = UpdateBaseBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Updated knowledge base manifest", body = Manifest),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn update_base(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBaseBody>,
) -> Result<Json<Manifest>, (StatusCode, String)> {
    let manifest = svc
        .update_base_async(&id, body.name.as_deref(), body.color.as_deref(), None)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, msg)
            } else {
                (StatusCode::BAD_REQUEST, msg)
            }
        })?;
    Ok(Json(manifest))
}

#[utoipa::path(
    put, path = "/knowledge/bases/{id}/default-model",
    request_body = SetDefaultModelBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Updated knowledge base default model", body = Manifest),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn set_default_model(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Json(body): Json<SetDefaultModelBody>,
) -> Result<Json<Manifest>, (StatusCode, String)> {
    let manifest = svc
        .set_default_model_async(&id, body.model, None)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, msg)
            } else {
                (StatusCode::BAD_REQUEST, msg)
            }
        })?;
    Ok(Json(manifest))
}

// ──────────────────────────────────────────────────────────────────────────────
// Issue #56 DR-18 / Task 29A: the user's own publicize / privatize control.
// ──────────────────────────────────────────────────────────────────────────────

/// What `POST /knowledge/bases/{id}/tier` says to a caller holding nothing but
/// the daemon secret.
///
/// §9.3 A1: that secret is reachable from any developer-enabled agent shell, so
/// `X-Secret-Key` alone is not a human (AR-11/AR-15). Moving a base's tier is the
/// one operation that can *reverse* the knowledge-base ratchet, so it is the last
/// place an unproven caller may be given the benefit of the doubt.
///
/// §14.4's content rule: it names the boundary and nothing about the base, and it
/// forecloses the retry, because a model that reads a refusal as transient loops
/// on it.
///
/// ⚠ It deliberately carries NEITHER of the two markers the renderer keys on —
/// not `USER_ACTION_REFUSAL_MARKER` ("is the user's decision, not yours"), whose
/// toast says *switch this chat's model*, nor `COPY_OF_PRIVATE_REFUSAL_MARKER`
/// ("only the person at the keyboard may do it"), whose toast says *branch it
/// from the chat window*. Both would send the user somewhere that cannot help.
const TIER_NEEDS_USER: &str =
    "Changing a knowledge base's privacy is a choice only the person at the keyboard can make, and \
     this request carried no proof it came from them. Nothing was changed. Do not retry; the same \
     call will be refused again. If this base should be readable by public models, stop and ask \
     the user to change it from the Knowledge view.";

/// Open question 23's posture, applied here without inventing a second answer: a
/// daemon that was handed no user-action key cannot verify one, so the control is
/// **unavailable** rather than open — in both directions, for every caller,
/// including the human at the keyboard.
///
/// It names the cause, because a refusal that reads as a permission denial sends
/// the user hunting for a permission that does not exist.
const TIER_NEEDS_A_DAEMON_KEY: &str =
    "This Biorouter backend was started without a user-action key, so it cannot tell a request \
     made by you from one made by a model, and changing a knowledge base's privacy is yours to \
     decide. Nothing was changed. The desktop app supplies that key; a backend started by `just \
     run-server`, by running `biorouterd agent` by hand, or as a headless server deployment does \
     not, and cannot offer this control. Use the desktop app for this change.";

#[derive(Deserialize, ToSchema)]
pub struct SetKbTierBody {
    /// The tier the user chose. Both directions require the proof-of-user:
    /// privatizing discloses nothing and needs no confirmation dialog, but it is
    /// still not a thing a model may do, and admitting one direction unproven is
    /// how the tool channel gets the decision back.
    pub tier: KbTier,
}

#[derive(Serialize, ToSchema)]
pub struct KbTierResponse {
    pub id: String,
    pub tier: KbTier,
    /// `publicized_by_user` / `privatized_by_user`, or absent for a base whose
    /// tier only the ratchet has ever touched. A base the user released must
    /// never be indistinguishable from one that was always public.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// RFC 3339, and set exactly when `reason` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<String>,
    /// What a publicize would release, counted from the tree at read time — not
    /// from anything the renderer already had. The confirmation states the blast
    /// radius rather than asking "are you sure", so these are the numbers it
    /// says out loud.
    pub page_count: usize,
    pub raw_source_count: usize,
}

/// Count what a publicize would release: every page under `knowledge/` and every
/// raw source under `raw/`.
///
/// A missing directory counts zero rather than failing: a base can legitimately
/// have no raw sources, and a dialog that cannot open because a folder is absent
/// is worse than one that says "0 raw sources".
fn blast_radius(root: &std::path::Path, kb_id: &str) -> (usize, usize) {
    let kb_root = paths::kb_root(root, kb_id);
    let pages = store::list_pages(&kb_root, None)
        .map(|p| p.len())
        .unwrap_or(0);
    let raw = std::fs::read_dir(paths::kb_raw_dir(root, kb_id))
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0);
    (pages, raw)
}

fn tier_response(svc: &KnowledgeService, id: &str) -> KbTierResponse {
    let entry = tier::entry(svc.root(), id);
    let (page_count, raw_source_count) = blast_radius(svc.root(), id);
    KbTierResponse {
        id: id.to_string(),
        tier: entry.tier,
        reason: entry.reason,
        changed_at: entry.changed_at,
        page_count,
        raw_source_count,
    }
}

/// Read a base's tier, its provenance, and what publicizing it would release.
///
/// A plain read: it is the Knowledge view asking about the user's own base, and
/// the barrier Task 10C installs is for model callers. No proof-of-user, because
/// nothing is changed.
#[utoipa::path(
    get, path = "/knowledge/bases/{id}/tier",
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "The base's tier and what a publicize would release", body = KbTierResponse),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_kb_tier(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
) -> Result<Json<KbTierResponse>, (StatusCode, String)> {
    svc.get_base(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(tier_response(&svc, &id)))
}

/// Move a knowledge base's privacy tier, on the user's behalf (issue #56 DR-18).
///
/// The ONLY route in the tree that can LOWER one. It is user-only (DR-16's
/// `X-User-Action`, the same header and the same key Task 18A installs — not a
/// second one), it works in both directions, and the change carries its
/// provenance into `.kb-tiers` so a released base stays distinguishable from one
/// that was always public.
///
/// ⚠ **There is no `kb_set_tier` tool and there must never be one.** A model
/// raises a tier as a side effect of writing (Task 10B, raise-only) and can do
/// nothing else.
///
/// ⚠ **The typed confirmation is not enforced here, and that is deliberate**,
/// unlike `POST /sessions/{id}/declassify` where the daemon re-derives the grade.
/// A session's grade depends on server state (its stored provenance), so a client
/// could otherwise claim the weak control for a chat that no longer qualifies.
/// A base's grade depends only on the DIRECTION, which the request itself states:
/// a publicize with no phrase is exactly what the request says it wants, and the
/// phrase's job — making the user check *which* base — is a property of the
/// dialog, not a claim about server state. What the daemon enforces is the thing
/// a client cannot fake: the proof that a human asked at all.
#[utoipa::path(
    post, path = "/knowledge/bases/{id}/tier",
    request_body = SetKbTierBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "The base's tier after the change", body = KbTierResponse),
        (status = 403, description = "Refused by a privacy boundary: changing a knowledge base's \
                                      privacy is the user's decision and the request carried no \
                                      proof it came from them, or this daemon holds no \
                                      user-action key at all (body = plain text)"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn set_kb_tier(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: HeaderMap,
    Json(body): Json<SetKbTierBody>,
) -> Result<Json<KbTierResponse>, (StatusCode, String)> {
    // FIRST, before the base is even looked up. An unproven caller learns nothing
    // about which ids exist, and the refusal cannot be told apart from one for a
    // base that is not there.
    match user_action_proof(&headers) {
        UserActionProof::Proven => {}
        UserActionProof::Unproven => {
            return Err((StatusCode::FORBIDDEN, TIER_NEEDS_USER.to_string()))
        }
        UserActionProof::NoKeyInstalled => {
            return Err((StatusCode::FORBIDDEN, TIER_NEEDS_A_DAEMON_KEY.to_string()))
        }
    }

    svc.get_base(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // The single construction site of the proof-of-user, pinned by
    // `knowledge::tier_user::tests::the_proof_of_user_is_constructed_in_exactly_one_place`.
    svc.set_tier_by_user_async(&id, body.tier, UserKbTierChange::from_user_action(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(tier_response(&svc, &id)))
}

#[utoipa::path(
    delete, path = "/knowledge/bases/{id}",
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_base(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    svc.delete_base_async(&id, None).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") {
            (StatusCode::NOT_FOUND, msg)
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/graph",
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Knowledge graph", body = Graph),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_graph(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
) -> Result<Json<Graph>, (StatusCode, String)> {
    let g = svc
        .get_graph_async(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(g))
}

#[derive(Serialize, ToSchema)]
pub struct LocationResponse {
    /// Absolute on-disk path to the knowledge base directory (the folder that
    /// holds `knowledge/`, `raw/`, `index.md`, …). Clients use this to open the
    /// folder in the OS file explorer so users can inspect raw markdown.
    pub path: String,
}

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/location",
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Knowledge base on-disk location", body = LocationResponse),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_location(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
) -> Result<Json<LocationResponse>, (StatusCode, String)> {
    let kb_root = paths::kb_root(svc.root(), &id);
    if !kb_root.exists() {
        return Err((StatusCode::NOT_FOUND, format!("kb '{id}' not found")));
    }
    Ok(Json(LocationResponse {
        path: kb_root.to_string_lossy().into_owned(),
    }))
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 6: page CRUD routes
// ──────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/pages",
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("path_prefix" = Option<String>, Query, description = "Optional path prefix filter"),
    ),
    responses((status = 200, description = "Page list"))
)]
pub async fn list_pages(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Query(q): Query<ListPagesQuery>,
) -> Result<Json<Vec<store::PageRef>>, (StatusCode, String)> {
    let kb_root = paths::kb_root(svc.root(), &id);
    let pages = store::list_pages(&kb_root, q.path_prefix.as_deref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(pages))
}

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/pages/{page_path}",
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("page_path" = String, Path, description = "Page path within KB"),
    ),
    responses(
        (status = 200, description = "Page content"),
        (status = 404, description = "Page not found"),
    )
)]
pub async fn read_page(
    State(svc): State<Arc<KnowledgeService>>,
    Path((id, page_path)): Path<(String, String)>,
) -> Result<Json<store::PageContent>, (StatusCode, String)> {
    let kb_root = paths::kb_root(svc.root(), &id);
    let page = store::read_page(&kb_root, &page_path)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(page))
}

#[utoipa::path(
    put, path = "/knowledge/bases/{id}/pages/{page_path}",
    request_body = WritePageBody,
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("page_path" = String, Path, description = "Page path within KB"),
    ),
    responses(
        (status = 200, description = "Written", body = CommitResponse),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Write outcome uncertain or post-commit cache refresh failed"),
    )
)]
pub async fn write_page(
    State(svc): State<Arc<KnowledgeService>>,
    Path((id, page_path)): Path<(String, String)>,
    Json(body): Json<WritePageBody>,
) -> Result<Json<CommitResponse>, (StatusCode, String)> {
    let _lock = svc
        .lock_kb(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    svc.require_current_profile(&id)
        .map_err(knowledge_service_http_error)?;
    let kb_root = paths::kb_root(svc.root(), &id);
    let sha_opt = store::write_page(
        &kb_root,
        &page_path,
        &body.content,
        &body.commit_message,
        None,
    )
    .map_err(|e| {
        let status = if e
            .downcast_ref::<biorouter_mcp::knowledge::git::KnowledgeWriteFailure>()
            .is_some()
        {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, e.to_string())
    })?;
    let commit_sha = sha_opt.unwrap_or_default();
    // Re-derive the graph cache, exactly as the MCP `kb_write_page` tool does.
    //
    // `get_graph` serves `graph-cache.json` whenever it can read one, and a
    // base is created with an EMPTY cache. So a writer that does not refresh
    // it leaves the graph route answering "no nodes, no edges" for a base whose
    // pages are on disk and listed by `GET /pages` — measured on this route:
    // two pages written, one `[[wiki]]` link between them, `/graph` empty; the
    // same request after deleting the cache file returned both nodes and the
    // edge. Nothing in the UI can repair that, because "Refresh graph" re-reads
    // the same cache.
    //
    // The same fix already exists on the tool that writes a page from chat, and
    // this route is the other writer. If a third appears, it needs this too —
    // the cache is refreshed by the caller, not by `store::write_page`, because
    // a macro writes many pages under one lock and re-deriving per page would
    // do the whole derivation N times for one logical change.
    if let Err(error) = svc.rebuild_graph_cache(&id) {
        let failure = if commit_sha.is_empty() {
            anyhow::anyhow!(
                "page {page_path} already matched durable content, but its graph cache could not be refreshed: {error:#}. The cache will be re-derived on its next read"
            )
        } else {
            biorouter_mcp::knowledge::git::KnowledgeWriteFailure::committed(
                format!("page write to {page_path}"),
                commit_sha.clone(),
                error,
            )
            .into()
        };
        return Err((StatusCode::INTERNAL_SERVER_ERROR, failure.to_string()));
    }
    Ok(Json(CommitResponse { commit_sha }))
}

// ──────────────────────────────────────────────────────────────────────────────
// Plan 5 Task 1: GET /bases/:id/page?path=... — raw markdown body for the
// frontend NodePreview card. Distinct from `GET /bases/:id/pages/{*path}`
// (which returns parsed PageContent with frontmatter split out); this route
// returns the file verbatim so the renderer can show the original markdown.
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ReadPageQuery {
    pub path: String,
}

#[derive(Serialize, ToSchema)]
pub struct ReadPageResponse {
    pub content: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// GET + POST /knowledge/active — the session's knowledge-base set and its
// primary. One axis (the set, expressed as the hidden complement) plus one
// pointer. Both halves travel in one body so the primary is validated against
// the state the request produces, not the state it started from.
// ──────────────────────────────────────────────────────────────────────────────

/// Deserialize a nullable field while keeping "absent" and "explicitly null"
/// apart: absent → `None`, `null` → `Some(None)`, a value → `Some(Some(v))`.
///
/// A plain `Option<String>` collapses the first two, which costs a meaning the
/// wire needs. On this body the three primary-pointer states are *leave it*,
/// *forget it*, and *set it*, and the deprecated `kb_id` alias only ever spoke
/// the first and third — so the one gesture the alias was kept alive for, a
/// pre-`primary_kb` bundle sending `kb_id: null` to clear, silently did nothing.
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize, ToSchema)]
pub struct SetActiveBody {
    /// Make this base the session's primary — the KB-less write target. It
    /// must be a member of the **resulting** set, so `hidden_kbs` in the same
    /// body is applied first. Omit to leave the pointer alone; send `null` to
    /// forget it (the same as `clear_primary`).
    #[serde(default, deserialize_with = "present_or_null")]
    #[schema(value_type = Option<String>)]
    pub primary_kb: Option<Option<String>>,
    /// Deprecated alias for `primary_kb`, kept for one release so a stale
    /// renderer bundle talking to a fresh daemon keeps working. Follows the
    /// same rule: omitted leaves the pointer alone, `null` forgets it — which
    /// is exactly how such a bundle clears.
    #[serde(default, deserialize_with = "present_or_null")]
    #[schema(value_type = Option<String>)]
    pub kb_id: Option<Option<String>>,
    /// Forget the primary *at this scope*: the session then has no primary
    /// even while the machine-wide default names a base. Mutually exclusive
    /// with `primary_kb` and `inherit_primary`.
    #[serde(default)]
    pub clear_primary: bool,
    /// Drop this scope's own primary preference. A session then follows the
    /// machine-wide choice; at machine scope this restores Biorouter's shipped
    /// Soul default. This is the way back from `clear_primary`, and the only
    /// way out of the explicit "no primary" that deleting a session's pinned
    /// base leaves behind. Mutually exclusive with `primary_kb` and
    /// `clear_primary`.
    #[serde(default)]
    pub inherit_primary: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    /// Replace this scope's hidden list — i.e. redefine the session's set.
    /// Omit to leave the set alone. `[]` is an explicit "hide nothing here",
    /// not a request to inherit the machine-wide list.
    #[serde(default)]
    pub hidden_kbs: Option<Vec<String>>,
}

impl SetActiveBody {
    /// Fold the spellings of a primary change — `clear_primary`,
    /// `inherit_primary`, the current `primary_kb`, and the deprecated `kb_id`
    /// — into one decision, or reject the request.
    ///
    /// A field that is merely absent never votes. That is what lets a modern
    /// set-only edit (`{"hidden_kbs": [...]}`) leave the pointer where it is
    /// instead of clearing it as a side effect.
    ///
    /// Two fields asking for two *different* things is an error rather than a
    /// precedence rule. "Pin beta", "this chat has no primary" and "follow the
    /// machine default" are three incompatible outcomes; honouring one
    /// silently leaves the caller believing it got another, and the caller
    /// cannot tell which from a 200. Two fields asking for the *same* thing is
    /// not a conflict — `clear_primary` alongside `primary_kb: null` is how a
    /// bundle that predates `clear_primary` spells the identical gesture.
    fn primary_update(&self) -> Result<PrimaryUpdate<'_>, String> {
        let mut votes: Vec<(&'static str, PrimaryUpdate<'_>)> = Vec::new();
        if self.clear_primary {
            votes.push(("clear_primary", PrimaryUpdate::Clear));
        }
        if self.inherit_primary {
            votes.push(("inherit_primary", PrimaryUpdate::Inherit));
        }
        // `primary_kb` still shadows its deprecated alias: they are two names
        // for one field, not two opinions.
        let aliased = match self.primary_kb.as_ref() {
            Some(value) => Some(("primary_kb", value)),
            None => self.kb_id.as_ref().map(|value| ("kb_id", value)),
        };
        if let Some((field, value)) = aliased {
            votes.push((
                field,
                match value {
                    Some(id) => PrimaryUpdate::Set(id),
                    None => PrimaryUpdate::Clear,
                },
            ));
        }

        let mut distinct: Vec<(&'static str, PrimaryUpdate<'_>)> = Vec::new();
        for (field, update) in votes {
            if !distinct.iter().any(|(_, seen)| *seen == update) {
                distinct.push((field, update));
            }
        }

        match distinct.as_slice() {
            [] => Ok(PrimaryUpdate::Unchanged),
            [(_, update)] => Ok(*update),
            conflicting => Err(format!(
                "conflicting primary-KB fields ({}): send at most one of `primary_kb` \
                 (pin a base), `clear_primary` (this scope has no primary), or \
                 `inherit_primary` (follow the machine-wide default).",
                conflicting
                    .iter()
                    .map(|(field, _)| *field)
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ActiveKbResponse {
    /// The session's knowledge bases, sorted. Every one is searchable and
    /// readable; there is no narrower "active" list.
    pub kb_ids: Vec<String>,
    /// The KB-less write target. Always a member of `kb_ids`, or `null`.
    pub primary_kb: Option<String>,
    /// Deprecated mirror of `primary_kb`.
    pub active_kb: Option<String>,
    pub hidden_kbs: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct GetActiveQuery {
    #[serde(default)]
    pub session_id: Option<String>,
}

/// A selection as THIS caller may see it (issue #56, QA 2026-09-10 H2).
///
/// The second listing of base ids beside `GET /knowledge/bases`, and filtered
/// by the same gate: a base the caller cannot reach is dropped from the set and
/// from the hidden list, and a primary on one reads `null`. The result is the
/// selection the Knowledge view would hold if those bases did not exist — which
/// is exactly what the list it was given says — so nothing in it points at a
/// base the caller would then be refused. For the desktop app, which sends the
/// user's proof, nothing is dropped.
fn active_response(
    selection: biorouter_mcp::knowledge::service::KbSelection,
    root: &std::path::Path,
    caller: &crate::routes::session_reach::HttpCaller,
) -> ActiveKbResponse {
    let reachable = |id: &String| caller.reach_knowledge_base(root, id).is_ok();
    let primary_kb = selection.primary_kb.filter(reachable);
    ActiveKbResponse {
        kb_ids: selection.kb_ids.into_iter().filter(reachable).collect(),
        active_kb: primary_kb.clone(),
        primary_kb,
        hidden_kbs: selection.hidden_kbs.into_iter().filter(reachable).collect(),
    }
}

#[utoipa::path(
    get, path = "/knowledge/active",
    params(
        ("session_id" = Option<String>, Query, description = "Optional chat session id for the session-scoped selection"),
    ),
    responses(
        (status = 200, description = "The session's knowledge bases and its primary, showing only \
                                      the bases this caller may open: a private base is omitted \
                                      from both lists, and a private primary reads null, for a \
                                      caller without the user's proof or a private capability", body = ActiveKbResponse),
        (status = 403, description = "The named session is outside the caller's privacy reach")
    )
)]
pub async fn get_active(
    State(svc): State<Arc<KnowledgeService>>,
    Query(q): Query<GetActiveQuery>,
    headers: HeaderMap,
) -> Result<Json<ActiveKbResponse>, (StatusCode, String)> {
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    let selection = svc
        .selection(q.session_id.as_deref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(Json(active_response(selection, svc.root(), &caller)))
}

#[utoipa::path(
    post, path = "/knowledge/active",
    request_body = SetActiveBody,
    responses(
        (status = 200, description = "The resulting selection", body = ActiveKbResponse),
        (status = 400, description = "Unknown kb id, a primary outside the resulting set, \
                                      or conflicting primary-KB fields"),
        // Issue #56 Task 58 / #47. Produced by the layer on this router
        // (`routes::session_reach::gate_knowledge_active`), not by the handler
        // below — but it is what a client receives, so it belongs here.
        (status = 403, description = "Refused by a privacy boundary (issue #56 Task 58 / #47): \
                                      `session_id` names a private chat (or an absent one, and an \
                                      unproven caller is told the same thing for both) and the \
                                      request carried no proof it came from the user; or \
                                      `primary_kb` names a knowledge base this caller may not \
                                      reach, answered exactly as a base that does not exist \
                                      (body = plain text)"),
    )
)]
pub async fn set_active(
    State(svc): State<Arc<KnowledgeService>>,
    // Before `Json`, which consumes the body and must be last.
    headers: HeaderMap,
    Json(body): Json<SetActiveBody>,
) -> Result<Json<ActiveKbResponse>, (StatusCode, String)> {
    let primary = body
        .primary_update()
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    // Naming a base the caller may not reach is answered by the gate, in the
    // gate's words, before the service sees it — the same refusal a base that
    // does not exist gets, so pinning is not a way to ask which ids are private.
    if let PrimaryUpdate::Set(id) = primary {
        caller
            .reach_knowledge_base(svc.root(), id)
            .map_err(|refusal| (refusal.status, refusal.message.to_string()))?;
    }
    // A caller changes only what it can see: see `set_selection_within`. For a
    // caller that reaches every base this is exactly `set_selection`.
    let reachable = |id: &str| caller.reach_knowledge_base(svc.root(), id).is_ok();
    let selection = svc
        .set_selection_within(
            body.session_id.as_deref(),
            body.hidden_kbs.as_deref(),
            primary,
            &reachable,
        )
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    Ok(Json(active_response(selection, svc.root(), &caller)))
}

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/page",
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("path" = String, Query, description = "Page path under the KB root \
         (knowledge/*.md, raw/*/source.md, or index.md/schema.md/log.md)"),
    ),
    responses(
        (status = 200, description = "Page content", body = ReadPageResponse),
        (status = 400, description = "Invalid kb id or path"),
        (status = 404, description = "Page not found"),
    )
)]
pub async fn get_page_body(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Query(q): Query<ReadPageQuery>,
) -> Result<Json<ReadPageResponse>, (StatusCode, String)> {
    match svc.read_page(&id, &q.path) {
        Ok(content) => Ok(Json(ReadPageResponse { content })),
        Err(ReadPageError::InvalidKbId(m)) => Err((StatusCode::BAD_REQUEST, m)),
        Err(ReadPageError::InvalidPath(m)) => Err((StatusCode::BAD_REQUEST, m)),
        Err(ReadPageError::NotFound(m)) => Err((StatusCode::NOT_FOUND, m)),
        Err(e @ ReadPageError::Io(_)) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 7: history + preview + restore routes
// ──────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/history",
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("limit" = Option<usize>, Query, description = "Maximum entries (default 50)"),
    ),
    responses((status = 200, description = "Commit history"))
)]
pub async fn list_history(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryEntry>>, (StatusCode, String)> {
    svc.list_history(&id, q.limit)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/preview",
    request_body = PreviewBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "File content at commit", body = PreviewResponse),
        (status = 500, description = "Error"),
    )
)]
pub async fn preview_state(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Json(body): Json<PreviewBody>,
) -> Result<Json<PreviewResponse>, (StatusCode, String)> {
    let content = svc
        .preview_state(&id, &body.commit_sha, &body.path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(PreviewResponse { content }))
}

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/restore",
    request_body = RestoreBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Restored; returns new commit SHA", body = RestoreResponse),
        (status = 500, description = "Error"),
    )
)]
pub async fn restore_state(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<RestoreResponse>, (StatusCode, String)> {
    let new_commit_sha = svc
        .restore_state_async(&id, &body.commit_sha, None)
        .await
        .map_err(knowledge_service_http_error)?;
    Ok(Json(RestoreResponse { new_commit_sha }))
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 9: SSE-streamed macro routes (ingest / query / lint)
// ──────────────────────────────────────────────────────────────────────────────

/// Build a `Provider + ProviderCompleter` for the given `ModelRef`, **and** the
/// tier of the provider that was actually constructed (issue #56).
///
/// Returns a 400 error if the provider name is unknown or model config is invalid.
///
/// The tier comes back from here rather than being re-derived by each caller,
/// because `providers::create` intercepts `BIOROUTER_LEAD_MODEL` *before* the
/// registry lookup and can hand back a composite that is not the requested
/// name's provider at all. `ProviderCompleter::paired` reads it off the same
/// `Arc` the completer wraps, so the two cannot come from different providers.
async fn build_completer(
    model: &ModelRef,
    cancel: Option<CancellationToken>,
) -> Result<
    (
        Box<dyn biorouter_mcp::knowledge::subagent::loop_::Completer>,
        biorouter::privacy::ProviderTier,
        // Issue #56 DR-26 / Task 50: the third axis, off the same `Arc`.
        Option<biorouter::privacy::affiliation::ModelAffiliation>,
    ),
    (StatusCode, String),
> {
    if biorouter_mcp::knowledge::test_mode::env_enabled() {
        // ⚠ The FIRST of the two named literal exemptions (the CLI's
        // `build_completer` early return is the other). There is no provider
        // here to read a tier from, and the fail-safe direction for a *ratchet*
        // is not to privatise a base on a test path — a test-mode completer
        // reaches no network at all.
        return Ok((
            Box::new(biorouter_mcp::knowledge::test_mode::TestModeCompleter),
            biorouter::privacy::ProviderTier::Public,
            None,
        ));
    }

    let model_config =
        ModelConfig::new(&model.model).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let provider = biorouter::providers::create(&model.provider, model_config)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let (completer, tier, affiliation) = ProviderCompleter::paired(provider);
    let completer = match cancel {
        Some(cancel) => completer.cancelled_by(cancel),
        None => completer,
    };
    Ok((Box::new(completer), tier, affiliation))
}

/// The caller's identity for a lint that will **not** write (issue #56).
///
/// The tier comes from [`build_completer`] — the same one funnel the autofix
/// path and the two sibling macro routes use — so it is read off a *constructed
/// instance*, never re-derived from `model.provider`. That name-keyed lookup is
/// precisely what [`ProviderCompleter::paired`] exists to close: `ollama`'s
/// registry entry is Private unconditionally while its instance reads the
/// resolved base URL, and `providers::create` intercepts `BIOROUTER_LEAD_MODEL`
/// before the registry is consulted at all. The completer that comes back is
/// dropped: a scan has nothing to say to a model.
///
/// ⚠ **This read-only lint used to answer with a hardcoded `Public`**, on the
/// reasoning that a scan constructs no provider and so has no instance to read
/// a tier from. The premise was a choice, not a fact — `LintBody::model` is
/// required, so a lint always names a model and the tier was there to be had —
/// and the conclusion broke the feature: a Public capability can never reach a
/// private base, so [`assert_macro_target_reachable`] refused **every**
/// read-only lint of a private base, and refused it with a message telling the
/// user to switch this chat to a private model, the one remedy that could not
/// work while the model was not being read. That is the failure
/// `a_private_model_may_ingest_its_own_private_conversation_over_http` is
/// written to catch on the sibling gate: "refuse the public caller" is
/// satisfied by "refuse everyone".
///
/// ⚠ **The one thing this does differently is that it does not fail.** `autofix`
/// keeps `build_completer`'s 400, because a fix with no model cannot run at all.
/// A scan can, and always could — a read-only lint against an unconfigured or
/// unknown provider streams its report today, which is a real capability and
/// not an accident of the literal. So a provider that will not construct
/// resolves to Public with no affiliation: the restrictive reading on both axes,
/// so an identity that cannot be read can only ever refuse, never admit. Same
/// fail-safe direction, for the same reason, as `routes::apps::row_capability`.
async fn read_only_caller_identity(
    model: &ModelRef,
) -> (
    biorouter::privacy::ProviderTier,
    Option<biorouter::privacy::affiliation::ModelAffiliation>,
) {
    match build_completer(model, None).await {
        Ok((_completer, tier, affiliation)) => (tier, affiliation),
        Err(_) => (biorouter::privacy::ProviderTier::Public, None),
    }
}

/// Issue #56, Task 10C. Refuse a macro run whose model may not reach the target
/// base, **before** the SSE stream opens.
///
/// The barrier itself is CP2, inside each macro — that is what covers the CLI
/// and every non-HTTP caller, and it is the check a `grep` counts. This is the
/// same question asked one layer up so the GUI gets a real status code instead
/// of a stream that opens and immediately dies: a 200 with an `event: error`
/// frame is indistinguishable, to the fetch that started it, from a model that
/// failed to connect.
///
/// 409 CONFLICT and not 403: nothing about the *request* is unauthorised — the
/// user may read this base all day through `/bases/{id}/page`. What conflicts is
/// the base's tier with the model this chat is on, and the recovery is to change
/// the model.
///
/// ⚠ The message is `assert_reachable`'s own, never a second spelling of it.
fn assert_macro_target_reachable(
    svc: &Arc<KnowledgeService>,
    kb_id: &str,
    caller_capability: biorouter::privacy::ProviderTier,
    // Issue #56 DR-26 / Task 50: the third axis, so this pre-check asks the
    // caller's whole identity and not half of it. A pre-check that answered a
    // narrower question than the barrier would open the SSE stream on a flow
    // the barrier is about to refuse — which is the failure this function
    // exists to prevent, in the other direction.
    caller_affiliation: Option<biorouter::privacy::affiliation::ModelAffiliation>,
) -> Result<(), (StatusCode, String)> {
    biorouter_mcp::knowledge::tier::assert_reachable(
        svc.root(),
        kb_id,
        caller_capability.is_private(),
        &biorouter::privacy::affiliation::caller_affiliation(caller_affiliation),
    )
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}

/// Issue #56, Gate G. Refuse a conversation ingest whose model may not read the
/// requested chats, **before** the SSE stream opens.
///
/// The barrier itself is inside `conversation_ingest::ingest_conversation` —
/// that is the ONE guard, and it is what covers the CLI, the platform tool and
/// this route alike even if this pre-check were deleted. Exactly as
/// `assert_macro_target_reachable` does for Task 10C's barrier, this asks the
/// same question one layer up so the GUI gets a real status code instead of a
/// 200 whose stream opens and immediately dies — indistinguishable, to the
/// fetch that started it, from a model that failed to connect.
///
/// 409 CONFLICT and not 500: a barrier that surfaces as an internal error
/// teaches the caller to retry. Nothing about the *request* is malformed; what
/// conflicts is the chats' classification with the model this call is on, and
/// the recovery is to change the model.
///
/// ⚠ The message is `conversation_ingest`'s own, never a second spelling of it,
/// and it names no session (§11.4 classifies id, title and working directory as
/// content).
fn assert_conversations_readable(
    caller_capability: biorouter::privacy::ProviderTier,
    sessions: &[biorouter::session::session_manager::Session],
) -> Result<(), (StatusCode, String)> {
    if biorouter::knowledge::conversation_ingest::refuses_every_session(caller_capability, sessions)
    {
        return Err((
            StatusCode::CONFLICT,
            biorouter::knowledge::conversation_ingest::REFUSED_ALL_PRIVATE.to_string(),
        ));
    }
    Ok(())
}

/// Build a well-formed SSE error frame. Uses `serde_json` for proper escaping so
/// that backslashes in Windows paths and newlines in multi-line `anyhow` chains do
/// not break JSON or SSE line framing.
fn sse_error_frame(message: &str) -> String {
    let payload = serde_json::json!({ "message": message });
    let json = serde_json::to_string(&payload)
        .unwrap_or_else(|_| String::from("{\"message\":\"<unserializable error>\"}"));
    format!("event: error\ndata: {json}\n\n")
}

async fn parse_ingest_request(
    headers: &HeaderMap,
    req: Request,
) -> Result<(convert::SourceInput, ModelRef, Option<String>), (StatusCode, String)> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.starts_with("multipart/form-data") {
        return parse_ingest_multipart(req).await;
    }

    let body_bytes = axum::body::to_bytes(req.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let body: IngestBody = serde_json::from_slice(&body_bytes)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let source =
        parse_source_input(&body.source).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((source, body.model, body.focus))
}

async fn parse_ingest_multipart(
    req: Request,
) -> Result<(convert::SourceInput, ModelRef, Option<String>), (StatusCode, String)> {
    let mut mp = Multipart::from_request(req, &())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let mut upload: Option<MultipartUpload> = None;
    let mut provider: Option<String> = None;
    let mut model_name: Option<String> = None;
    let mut focus: Option<String> = None;

    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        match field.name() {
            Some("file") => {
                let filename = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "upload.bin".to_string());
                let mime = sanitize_part_mime(field.content_type());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
                    .to_vec();
                validate_ingest_upload(&filename, bytes.len())?;
                upload = Some(MultipartUpload {
                    bytes,
                    filename,
                    mime,
                });
            }
            Some("provider") => {
                provider = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
                );
            }
            Some("model") => {
                model_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
                );
            }
            Some("focus") => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                if !value.trim().is_empty() {
                    focus = Some(value);
                }
            }
            _ => {}
        }
    }

    let upload = upload.ok_or((StatusCode::BAD_REQUEST, "missing 'file' part".to_string()))?;
    let provider = provider.ok_or((
        StatusCode::BAD_REQUEST,
        "missing 'provider' field".to_string(),
    ))?;
    let model_name =
        model_name.ok_or((StatusCode::BAD_REQUEST, "missing 'model' field".to_string()))?;

    Ok((
        convert::SourceInput::File {
            bytes: upload.bytes,
            filename: upload.filename,
            mime: upload.mime,
        },
        ModelRef {
            provider,
            model: model_name,
        },
        focus,
    ))
}

#[utoipa::path(
    post, path = "/knowledge/expand-path",
    request_body = ExpandPathBody,
    responses(
        (status = 200, description = "Expanded local path into stageable files", body = ExpandPathResponse),
        (status = 400, description = "Invalid local path"),
    )
)]
pub async fn expand_path(
    Json(body): Json<ExpandPathBody>,
) -> Result<Json<ExpandPathResponse>, (StatusCode, String)> {
    let expanded = source_paths::expand_ingest_path(std::path::Path::new(&body.path))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(ExpandPathResponse {
        files: expanded
            .files
            .into_iter()
            .map(|file| ExpandPathFile {
                path: file.path.to_string_lossy().into_owned(),
                name: file.name,
                relative_path: file.relative_path,
            })
            .collect(),
        warnings: expanded
            .warnings
            .into_iter()
            .map(|warning| ExpandPathWarning {
                level: match warning.level {
                    source_paths::WarningLevel::Warning => "warning".to_string(),
                    source_paths::WarningLevel::Error => "error".to_string(),
                },
                title: warning.title,
                message: warning.message,
            })
            .collect(),
    }))
}

#[utoipa::path(
    post, path = "/knowledge/check-model",
    request_body = CheckModelBody,
    responses(
        (status = 200, description = "Model responded OK", body = CheckModelResponse),
        (status = 502, description = "Model is unreachable / invalid", body = CheckModelResponse),
    )
)]
pub async fn check_model(
    State(svc): State<Arc<KnowledgeService>>,
    Json(body): Json<CheckModelBody>,
) -> Result<Json<CheckModelResponse>, (StatusCode, Json<CheckModelResponse>)> {
    let completer = match build_completer(&body.model, None).await {
        Ok((c, _tier, _affiliation)) => c,
        Err((_status, msg)) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(CheckModelResponse {
                    ok: false,
                    error: Some(format!("provider build failed: {msg}")),
                }),
            ));
        }
    };
    match svc.check_model(completer).await {
        Ok(()) => Ok(Json(CheckModelResponse {
            ok: true,
            error: None,
        })),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(CheckModelResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        )),
    }
}

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/ingest",
    request_body = IngestBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "SSE stream of sub-agent events (text/event-stream)"),
        (status = 400, description = "Invalid model or source"),
    )
)]
pub async fn ingest(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Result<crate::routes::reply::SseResponse, (StatusCode, String)> {
    let (source, model, focus) = parse_ingest_request(&headers, req).await?;
    svc.require_current_profile(&id)
        .map_err(knowledge_service_http_error)?;

    let cancel = CancellationToken::new();
    let (completer, caller_capability, caller_affiliation) =
        build_completer(&model, Some(cancel.clone())).await?;
    assert_macro_target_reachable(&svc, &id, caller_capability, caller_affiliation)?;

    let (sse_tx, sse_rx) = mpsc::channel::<String>(64);
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SubAgentEvent>();
    let (result_tx, result_rx) = mpsc::channel::<Result<serde_json::Value, String>>(1);

    // Macro task: run ingest, then report result through a dedicated channel.
    // Dropping `event_tx` signals the forwarder that no more events are coming.
    let cancel_for_macro = cancel.clone();
    let macro_handle = tokio::spawn(async move {
        let args = ingest_macro::IngestArgs {
            kb_id: id,
            // Issue #56. The tier of the provider `build_completer` actually
            // constructed — never `body.model.provider`, the string the caller
            // supplied, which `providers::create` is free to ignore.
            caller_is_private: caller_capability.is_private(),
            caller_affiliation: biorouter::privacy::affiliation::caller_affiliation(
                caller_affiliation,
            ),
            source,
            completer,
            focus,
            bounds: ingest_bounds(),
            event_sink: Some(event_tx),
            cancel: Some(cancel_for_macro),
        };
        let outcome = match ingest_macro::ingest(&svc, args).await {
            Ok(r) => Ok(serde_json::to_value(&r).unwrap_or_default()),
            Err(e) => Err(e.to_string()),
        };
        let _ = result_tx.send(outcome).await;
    });

    tokio::spawn(forward_macro_stream(
        sse_tx,
        event_rx,
        result_rx,
        macro_handle,
        cancel,
    ));

    Ok(crate::routes::reply::SseResponse::from_rx(sse_rx))
}

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/ingest-conversation",
    request_body = IngestConversationBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "SSE stream of sub-agent events (text/event-stream)"),
        (status = 400, description = "Invalid model or no sessions"),
    )
)]
pub async fn ingest_conversation(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: HeaderMap,
    Json(body): Json<IngestConversationBody>,
) -> Result<crate::routes::reply::SseResponse, (StatusCode, String)> {
    if body.session_ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "session_ids cannot be empty".into(),
        ));
    }

    // Load the requested sessions (with messages) from the global session store.
    //
    // Issue #56 DR-26 / Task 50 Step 3: ONE handle, shared with the macro below
    // rather than a second `instance()`. Both resolve to the same static storage
    // so the old pair was harmless — but the guard's whole claim is that it reads
    // each selected chat's institutions *from the store those chats came from*,
    // and one binding is what makes that visible instead of argued.
    let session_manager =
        std::sync::Arc::new(biorouter::session::session_manager::SessionManager::instance());

    // Issue #56, QA 2026-09-10 H2. This route NAMES chats, and streams what the
    // macro makes of them back to whoever asked — so the caller must be able to
    // reach each one, by the gate `GET /sessions/{id}` uses and in its words,
    // before a single transcript is read. Gate G below is a different question
    // (may the MODEL read them), and a caller holding only the daemon secret can
    // name a private model: without this it read a private chat through one.
    // Every id is checked before any is loaded, so the refusal cannot say which
    // of several named chats exist.
    for sid in &body.session_ids {
        crate::routes::session_reach::session_reach(&session_manager, sid, &headers)
            .await
            .map_err(|refusal| (refusal.status, refusal.message.to_string()))?;
    }

    let mut sessions = Vec::new();
    for sid in &body.session_ids {
        match session_manager.get_session(sid, true).await {
            Ok(s) => sessions.push(s),
            Err(e) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("session '{sid}' not found: {e}"),
                ));
            }
        }
    }

    let cancel = CancellationToken::new();
    let (completer, caller_capability, caller_affiliation) =
        build_completer(&body.model, Some(cancel.clone())).await?;
    // Issue #56, Gate G. Before the stream opens, and before a single transcript
    // is rendered: this route is the same private -> public laundering primitive
    // as the platform tool, reachable with nothing but the secret key.
    assert_conversations_readable(caller_capability, &sessions)?;

    let (sse_tx, sse_rx) = mpsc::channel::<String>(64);
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SubAgentEvent>();
    let (result_tx, result_rx) = mpsc::channel::<Result<serde_json::Value, String>>(1);

    let focus = body.focus.clone();
    let cancel_for_macro = cancel.clone();
    // Issue #56 DR-26 / Task 50 Step 3: the guard reads each selected chat's
    // institutions itself — see `ConversationIngestArgs::session_manager`. The
    // same handle the sessions above were loaded through.
    let session_manager_for_macro = session_manager.clone();
    let macro_handle = tokio::spawn(async move {
        let args = biorouter::knowledge::conversation_ingest::ConversationIngestArgs {
            kb_id: id,
            // Issue #56. Same rule as the other three macro routes: the tier of
            // the constructed provider, not of the requested name.
            caller_capability,
            caller_affiliation,
            session_manager: session_manager_for_macro,
            sessions,
            completer,
            focus,
            bounds: ingest_bounds(),
            event_sink: Some(event_tx),
            cancel: Some(cancel_for_macro),
        };
        let outcome = match biorouter::knowledge::conversation_ingest::ingest_conversation(
            &svc, args,
        )
        .await
        {
            Ok(r) => Ok(serde_json::to_value(&r).unwrap_or_default()),
            Err(e) => Err(e.to_string()),
        };
        let _ = result_tx.send(outcome).await;
    });

    tokio::spawn(forward_macro_stream(
        sse_tx,
        event_rx,
        result_rx,
        macro_handle,
        cancel,
    ));

    Ok(crate::routes::reply::SseResponse::from_rx(sse_rx))
}

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/query",
    request_body = QueryBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "SSE stream of sub-agent events (text/event-stream)"),
        (status = 400, description = "Invalid model"),
    )
)]
pub async fn query_kb(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Json(body): Json<QueryBody>,
) -> Result<crate::routes::reply::SseResponse, (StatusCode, String)> {
    svc.require_current_profile(&id)
        .map_err(knowledge_service_http_error)?;
    let cancel = CancellationToken::new();
    let (completer, caller_capability, caller_affiliation) =
        build_completer(&body.model, Some(cancel.clone())).await?;
    assert_macro_target_reachable(&svc, &id, caller_capability, caller_affiliation)?;

    let (sse_tx, sse_rx) = mpsc::channel::<String>(64);
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SubAgentEvent>();
    let (result_tx, result_rx) = mpsc::channel::<Result<serde_json::Value, String>>(1);

    let cancel_for_macro = cancel.clone();
    let macro_handle = tokio::spawn(async move {
        let args = query_macro::QueryArgs {
            kb_id: id,
            // Issue #56. A saved query writes model output into the base; an
            // ordinary query is read-only and must not reclassify it.
            caller_is_private: caller_capability.is_private(),
            caller_affiliation: biorouter::privacy::affiliation::caller_affiliation(
                caller_affiliation,
            ),
            question: body.question,
            completer,
            file_as_page: body.file_as_page.unwrap_or(false),
            bounds: SubAgentBounds::default(),
            event_sink: Some(event_tx),
            cancel: Some(cancel_for_macro),
        };
        let outcome = match query_macro::query(&svc, args).await {
            Ok(r) => Ok(serde_json::to_value(&r).unwrap_or_default()),
            Err(e) => Err(e.to_string()),
        };
        let _ = result_tx.send(outcome).await;
    });

    tokio::spawn(forward_macro_stream(
        sse_tx,
        event_rx,
        result_rx,
        macro_handle,
        cancel,
    ));

    Ok(crate::routes::reply::SseResponse::from_rx(sse_rx))
}

/// Lint a knowledge base and stream the result.
///
/// The stream's terminal `event: done` frame carries a `LintResult`: the
/// autofix commit, if any, wrapped around a `LintReport` — the four hygiene
/// lists this route has always returned, plus `diagnostics`, the typed findings.
/// Each of those is a stable rule id (`kb.` for hygiene, `okf.` and `biookf.`
/// for the format layers), a severity, the subject it is about and a message;
/// a consumer matches on `rule` and never on `message`, which is prose.
///
/// Both schemas are published under `components.schemas` rather than as this
/// response's body, because the body is an event stream and typing it as JSON
/// would be a false statement the generated client believes.
///
/// Retired pre-OKF bases are refused before the macro starts. Startup removes
/// them, and every remaining base receives its current OKF or BioOKF format
/// layer in addition to the shared hygiene rules.
#[utoipa::path(
    post, path = "/knowledge/bases/{id}/lint",
    request_body = LintBody,
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "SSE stream of sub-agent events (text/event-stream); \
                                      the terminal `event: done` frame's data is a LintResult"),
        (status = 400, description = "Invalid model"),
    )
)]
pub async fn lint(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    Json(body): Json<LintBody>,
) -> Result<crate::routes::reply::SseResponse, (StatusCode, String)> {
    svc.require_current_profile(&id)
        .map_err(knowledge_service_http_error)?;
    let autofix = body.autofix.unwrap_or(false);
    let cancel = CancellationToken::new();
    // Only build a *completer* when autofix is requested (it requires an LLM).
    // The caller's CAPABILITY is read on both paths, off the provider
    // `body.model` names — see `read_only_caller_identity` for why a scan asks
    // the same question a fix does, and why it may not answer with a literal.
    let (completer, caller_capability, caller_affiliation): (
        Option<Box<dyn biorouter_mcp::knowledge::subagent::loop_::Completer>>,
        biorouter::privacy::ProviderTier,
        Option<biorouter::privacy::affiliation::ModelAffiliation>,
    ) = if autofix {
        let (c, tier, affiliation) = build_completer(&body.model, Some(cancel.clone())).await?;
        (Some(c), tier, affiliation)
    } else {
        let (tier, affiliation) = read_only_caller_identity(&body.model).await;
        (None, tier, affiliation)
    };
    assert_macro_target_reachable(&svc, &id, caller_capability, caller_affiliation)?;

    let (sse_tx, sse_rx) = mpsc::channel::<String>(64);
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SubAgentEvent>();
    let (result_tx, result_rx) = mpsc::channel::<Result<serde_json::Value, String>>(1);

    let cancel_for_macro = cancel.clone();
    let macro_handle = tokio::spawn(async move {
        let args = lint_macro::LintArgs {
            kb_id: id,
            // Issue #56. The tier of the provider the autofix will run on.
            caller_is_private: caller_capability.is_private(),
            caller_affiliation: biorouter::privacy::affiliation::caller_affiliation(
                caller_affiliation,
            ),
            completer,
            autofix,
            bounds: SubAgentBounds::default(),
            event_sink: Some(event_tx),
            cancel: Some(cancel_for_macro),
        };
        let outcome = match lint_macro::lint(&svc, args).await {
            Ok(r) => Ok(serde_json::to_value(&r).unwrap_or_default()),
            Err(e) => Err(e.to_string()),
        };
        let _ = result_tx.send(outcome).await;
    });

    tokio::spawn(forward_macro_stream(
        sse_tx,
        event_rx,
        result_rx,
        macro_handle,
        cancel,
    ));

    Ok(crate::routes::reply::SseResponse::from_rx(sse_rx))
}

/// Parse the JSON `source` field into a typed `SourceInput`.
fn parse_source_input(v: &serde_json::Value) -> anyhow::Result<convert::SourceInput> {
    if let Some(url) = v.get("url").and_then(|x| x.as_str()) {
        Ok(convert::SourceInput::Url(url.to_string()))
    } else if let Some(path) = v.get("path").and_then(|x| x.as_str()) {
        Ok(convert::SourceInput::Path(std::path::PathBuf::from(path)))
    } else if let Some(text) = v.get("text").and_then(|x| x.as_str()) {
        Ok(convert::SourceInput::Text {
            text: text.to_string(),
            title: v
                .get("title")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        })
    } else {
        anyhow::bail!("source must have 'url', 'path', or 'text'")
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 8: POST /bases/:id/raw  (multipart file | JSON {url} | JSON {text,title})
// ──────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/raw",
    params(("id" = String, Path, description = "Knowledge base ID")),
    request_body(
        content = inline(serde_json::Value),
        description = "One of: multipart/form-data with 'file' field, \
                       JSON {url}, or JSON {text, title?}",
    ),
    responses(
        (status = 200, description = "Source ingested", body = RawSourceResponse),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Internal error"),
    )
)]
pub async fn add_raw_source(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Result<Json<RawSourceResponse>, (StatusCode, String)> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let input = if content_type.starts_with("multipart/form-data") {
        // Parse multipart — consume the whole request.
        let mut mp = Multipart::from_request(req, &())
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let mut upload: Option<MultipartUpload> = None;
        while let Some(field) = mp
            .next_field()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        {
            if field.name() == Some("file") {
                let filename = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "upload.bin".to_string());
                let mime = sanitize_part_mime(field.content_type());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
                    .to_vec();
                validate_ingest_upload(&filename, bytes.len())?;
                upload = Some(MultipartUpload {
                    bytes,
                    filename,
                    mime,
                });
            }
        }
        let upload = upload.ok_or((StatusCode::BAD_REQUEST, "missing 'file' part".to_string()))?;
        convert::SourceInput::File {
            bytes: upload.bytes,
            filename: upload.filename,
            mime: upload.mime,
        }
    } else {
        // JSON body — read raw bytes then parse.
        let body_bytes = axum::body::to_bytes(req.into_body(), 16 * 1024 * 1024)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let json: serde_json::Value = serde_json::from_slice(&body_bytes)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
            convert::SourceInput::Url(url.to_string())
        } else if let Some(path) = json.get("path").and_then(|v| v.as_str()) {
            convert::SourceInput::Path(std::path::PathBuf::from(path))
        } else if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
            let title = json
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            convert::SourceInput::Text {
                text: text.to_string(),
                title,
            }
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                "expected file (multipart), {url}, or {text}".to_string(),
            ));
        }
    };

    let _lock = svc
        .lock_kb(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    svc.require_current_profile(&id)
        .map_err(knowledge_service_http_error)?;
    let res = svc
        .add_raw_source(&id, input, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(RawSourceResponse {
        source_id: res.source_id,
        source_md_path: res.source_md_path,
    }))
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 10: GET /bases/:id/export + POST /bases/import
// ──────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/knowledge/bases/{id}/export",
    params(("id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Binary .brkb archive", content_type = "application/octet-stream"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal error"),
    )
)]
pub async fn export_brkb(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let kb_root = paths::kb_root(svc.root(), &id);
    if !kb_root.exists() {
        return Err((StatusCode::NOT_FOUND, format!("kb '{id}' not found")));
    }
    let bytes = svc
        .export_brkb(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let disposition = format!("attachment; filename=\"{id}.brkb\"");
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(response)
}

#[utoipa::path(
    post, path = "/knowledge/bases/import",
    request_body(
        content = inline(serde_json::Value),
        description = "multipart/form-data with a 'file' field containing the .brkb archive",
    ),
    responses(
        (status = 200, description = "Imported knowledge base ID"),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Internal error"),
    )
)]
pub async fn import_brkb(
    State(svc): State<Arc<KnowledgeService>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut file_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        if field.name() == Some("file") {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
            if bytes.len() as u64 > biorouter_mcp::knowledge::brkb::MAX_ARCHIVE_FILE_BYTES {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "compressed archive exceeds the {} MiB limit",
                        biorouter_mcp::knowledge::brkb::MAX_ARCHIVE_FILE_BYTES / (1024 * 1024)
                    ),
                ));
            }
            file_bytes = Some(bytes.to_vec());
        }
    }

    let bytes = file_bytes.ok_or((StatusCode::BAD_REQUEST, "missing 'file' part".to_string()))?;

    // Issue #56: the USER importing from the Knowledge view, not a model. The
    // archive's own provenance marker still applies as a floor — on both axes:
    // no model is bound here, so the importer contributes no institution
    // (`Unstated`), but the owners the archive carries are still recorded, or
    // routing an archive through this route would strip them (DR-26 / Task 50).
    let new_id = svc
        .import_brkb(
            &bytes,
            /* importer_is_private */ false,
            &biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated,
        )
        .map_err(knowledge_service_http_error)?;

    Ok(Json(serde_json::json!({ "id": new_id })))
}

fn knowledge_service_http_error(error: anyhow::Error) -> (StatusCode, String) {
    let bad_request = error
        .downcast_ref::<biorouter_mcp::knowledge::service::LegacyKnowledgeArchiveUnsupported>()
        .is_some()
        || error
            .downcast_ref::<biorouter_mcp::knowledge::service::LegacyKnowledgeBaseUnsupported>()
            .is_some()
        || error
            .downcast_ref::<biorouter_mcp::knowledge::service::LegacyKnowledgeRestoreUnsupported>()
            .is_some()
        || error
            .downcast_ref::<biorouter_mcp::knowledge::brkb::InvalidKnowledgeArchive>()
            .is_some();
    let status = if bad_request {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, error.to_string())
}

// ──────────────────────────────────────────────────────────────────────────────
// POST /bases/:id/merge — the user's own KB-to-KB merge
// ──────────────────────────────────────────────────────────────────────────────

/// Refused for the same reason `TIER_NEEDS_USER` is, worded for this control.
///
/// A merge can raise a base's tier and can add an owning institution to it, both
/// permanently — and it writes another base's content into this one. That is not
/// a decision the tool channel gets to make on the user's behalf through an
/// unproven HTTP call.
const MERGE_NEEDS_USER: &str =
    "Merging one knowledge base into another is a choice only the person at the keyboard can \
     make, and this request carried no proof it came from them. Nothing was changed. Do not \
     retry; the same call will be refused again. A model that wants this must use the kb_merge \
     tool, which is gated on the privacy of both bases.";

const MERGE_NEEDS_A_DAEMON_KEY: &str =
    "This Biorouter backend was started without a user-action key, so it cannot tell a request \
     made by you from one made by a model, and merging two knowledge bases is yours to decide. \
     Nothing was changed. The desktop app supplies that key; a backend started by `just \
     run-server`, by running `biorouterd agent` by hand, or as a headless server deployment does \
     not, and cannot offer this control.";

#[derive(Deserialize, ToSchema)]
pub struct MergeBody {
    /// The knowledge base to merge FROM. It is only read and is left unchanged.
    pub source_kb_id: String,
    /// Report what would happen and write nothing. Defaults to **true**, so a
    /// client that forgets the field gets the preview rather than the merge.
    /// This is the least reversible operation in the subsystem and
    /// `POST /restore` restores a whole tree, not one page.
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

fn default_true() -> bool {
    true
}

/// ⚠ **No caller barrier here, and it is the same position `GET /export` takes.**
/// This route is the USER's own path: they can already read both bases through
/// `GET /bases/{id}/page` and download either as a `.brkb`, and DR-14 governs
/// what a MODEL can reach. Passing a public-model identity instead would refuse
/// the user every merge involving a private base of their own — the feature's
/// main case — and passing a private one would be inventing a model that is not
/// there.
///
/// What still runs, and is the part that matters, is the classification fold:
/// the destination takes `max` over the tier axis and the union over owning
/// institutions. That is what keeps the model side honest after a merge the user
/// performed.
///
/// The proof-of-user is therefore load-bearing rather than ceremonial — it is
/// the whole of what separates this branch from the tool channel.
#[utoipa::path(
    post, path = "/knowledge/bases/{id}/merge",
    request_body = MergeBody,
    params(("id" = String, Path, description = "Destination knowledge base ID — canonical; never modified")),
    responses(
        (status = 200, description = "What the merge did, or (dry run) would do", body = MergeReport),
        (status = 400, description = "Bad request"),
        (status = 403, description = "Refused: merging is the user's decision and the request \
                                      carried no proof it came from them, or this daemon holds \
                                      no user-action key at all (body = plain text)"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal error"),
    )
)]
pub async fn merge_bases(
    State(svc): State<Arc<KnowledgeService>>,
    Path(id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: HeaderMap,
    Json(body): Json<MergeBody>,
) -> Result<Json<MergeReport>, (StatusCode, String)> {
    // FIRST, before either base is looked up. An unproven caller learns nothing
    // about which ids exist.
    match user_action_proof(&headers) {
        UserActionProof::Proven => {}
        UserActionProof::Unproven => {
            return Err((StatusCode::FORBIDDEN, MERGE_NEEDS_USER.to_string()))
        }
        UserActionProof::NoKeyInstalled => {
            return Err((StatusCode::FORBIDDEN, MERGE_NEEDS_A_DAEMON_KEY.to_string()))
        }
    }

    // The single construction site of the merge proof-of-user, pinned by
    // `knowledge::merge::tests::the_merge_proof_of_user_is_constructed_in_exactly_one_place`.
    let proof = UserKbMerge::from_user_action();
    let report = svc
        .merge_bases(
            &id,
            &body.source_kb_id,
            &MergeAuthority::User(&proof),
            body.dry_run,
        )
        .await
        .map_err(knowledge_service_http_error)?;
    Ok(Json(report))
}

// ──────────────────────────────────────────────────────────────────────────────
// Task 11: POST /bases/:id/sources/:sid/reclassify
//          PUT  /bases/:id/sources/:sid/credibility
// ──────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/knowledge/bases/{id}/sources/{sid}/reclassify",
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("sid" = String, Path, description = "Source ID"),
    ),
    responses(
        (status = 200, description = "Reclassified credibility", body = CredibilityResponse),
        (status = 404, description = "Source not found"),
        (status = 500, description = "Internal error"),
    )
)]
pub async fn reclassify(
    State(svc): State<Arc<KnowledgeService>>,
    Path((id, sid)): Path<(String, String)>,
) -> Result<Json<CredibilityResponse>, (StatusCode, String)> {
    let source_path = paths::kb_root(svc.root(), &id).join("raw").join(&sid);
    if !source_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("source '{sid}' not found in kb '{id}'"),
        ));
    }
    let credibility = svc
        .reclassify_source(&id, &sid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(CredibilityResponse { credibility }))
}

#[utoipa::path(
    put, path = "/knowledge/bases/{id}/sources/{sid}/credibility",
    request_body = Credibility,
    params(
        ("id" = String, Path, description = "Knowledge base ID"),
        ("sid" = String, Path, description = "Source ID"),
    ),
    responses(
        (status = 200, description = "Credibility overridden", body = CredibilityResponse),
        (status = 404, description = "Source not found"),
        (status = 500, description = "Internal error"),
    )
)]
pub async fn override_credibility(
    State(svc): State<Arc<KnowledgeService>>,
    Path((id, sid)): Path<(String, String)>,
    Json(cred): Json<Credibility>,
) -> Result<Json<CredibilityResponse>, (StatusCode, String)> {
    let source_path = paths::kb_root(svc.root(), &id).join("raw").join(&sid);
    if !source_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("source '{sid}' not found in kb '{id}'"),
        ));
    }
    let credibility = svc
        .override_credibility_async(&id, &sid, cred, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(CredibilityResponse { credibility }))
}

#[cfg(test)]
mod tests {
    use super::{assert_conversations_readable, forward_macro_stream};
    use axum::http::StatusCode;
    use biorouter::privacy::{ProviderTier, SessionClassification};
    use biorouter::session::session_manager::{Session, SessionType};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn dropping_a_macro_stream_cancels_and_joins_the_macro() {
        let (sse_tx, sse_rx) = mpsc::channel(1);
        drop(sse_rx);
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let (_result_tx, result_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let macro_cancel = cancel.clone();
        let cancellation_seen = std::sync::Arc::new(tokio::sync::Notify::new());
        let macro_saw_cancellation = cancellation_seen.clone();
        let allow_macro_exit = std::sync::Arc::new(tokio::sync::Notify::new());
        let macro_exit = allow_macro_exit.clone();
        let macro_handle = tokio::spawn(async move {
            macro_cancel.cancelled().await;
            macro_saw_cancellation.notify_one();
            macro_exit.notified().await;
        });

        let forwarder = tokio::spawn(forward_macro_stream(
            sse_tx,
            event_rx,
            result_rx,
            macro_handle,
            cancel.clone(),
        ));
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cancellation_seen.notified(),
        )
        .await
        .expect("a disconnected client did not cancel the macro");

        assert!(cancel.is_cancelled());
        assert!(
            !forwarder.is_finished(),
            "the stream supervisor detached the cancelled macro before it settled"
        );

        allow_macro_exit.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), forwarder)
            .await
            .expect("the stream supervisor did not finish after the macro settled")
            .expect("the stream supervisor panicked");
    }

    /// ⚠ DEVIATION, recorded rather than hidden. Task 11 writes this row as a
    /// `POST /bases/{id}/ingest-conversation` against the router in
    /// `tests/knowledge_routes.rs`. It cannot live there: that route loads its
    /// sessions from the process-global `SessionManager`, i.e. **the
    /// developer's real session database** — which is exactly why Task 10B's
    /// own ratchet matrix in that file excludes `ingest-conversation` by name.
    /// So the row is spelled here instead, against the production function the
    /// route calls, with sessions built in memory.
    fn session(id: &str, tier: SessionClassification) -> Session {
        Session {
            id: id.into(),
            working_dir: std::path::PathBuf::from("/tmp/x"),
            name: format!("chat {id}"),
            user_set_name: false,
            session_type: SessionType::User,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            extension_data: Default::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: None,
            workflow: None,
            user_workflow_values: None,
            conversation: None,
            message_count: 0,
            provider_name: None,
            model_config: None,
            diverged_from: None,
            branch_point_msg_uid: None,
            parent_session_id: None,
            privacy_tier: tier,
            privacy_reason: None,
        }
    }

    /// D8: this route is the same one-call private -> public laundering
    /// primitive as the platform tool, behind nothing but the secret key.
    #[test]
    fn a_public_model_is_refused_another_sessions_private_conversation_with_409() {
        let err = assert_conversations_readable(
            ProviderTier::Public,
            &[session("phi", SessionClassification::Private)],
        )
        .expect_err("a public model was handed a private transcript over HTTP");

        assert_eq!(err.0, StatusCode::CONFLICT, "409, never 500: {}", err.1);
        assert!(err.1.contains("private"), "{}", err.1);
        assert!(
            !err.1.contains("phi") && !err.1.contains("chat phi"),
            "the refusal named the session: {}",
            err.1
        );
    }

    /// BOTH directions. Without this row, "refuse the public caller" is
    /// satisfied by "refuse everyone" — a hardcoded `ProviderTier::Public` at
    /// the call site passes every refusal assertion above and quietly breaks
    /// the feature for exactly the sessions it was built for.
    #[test]
    fn a_private_model_may_ingest_its_own_private_conversation_over_http() {
        assert_conversations_readable(
            ProviderTier::Private,
            &[session("phi", SessionClassification::Private)],
        )
        .expect("a private model was refused its own private chat");
    }

    /// The ratchet is `max`, not `set`: a public chat ingesting itself is the
    /// overwhelmingly common call and must not regress.
    #[test]
    fn a_public_model_may_still_ingest_a_public_conversation_over_http() {
        assert_conversations_readable(
            ProviderTier::Public,
            &[session("mine", SessionClassification::Public)],
        )
        .expect("a public chat may always ingest itself");
    }

    /// Per session, not once. A mixed list keeps the public chats and lets the
    /// shared barrier drop the rest, so the route must NOT 409 here.
    #[test]
    fn a_mixed_list_is_not_refused_wholesale_at_the_route() {
        assert_conversations_readable(
            ProviderTier::Public,
            &[
                session("mine", SessionClassification::Public),
                session("phi", SessionClassification::Private),
            ],
        )
        .expect("one private chat in the list must not refuse the public ones");
    }
}
