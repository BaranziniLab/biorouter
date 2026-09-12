//! `/skills/*` — the canonical skill catalog, and per-conversation skill state.
//!
//! # Why these routes exist (#113)
//!
//! The composer's skill menu used to answer both questions itself. It walked
//! the filesystem over its own list of three roots, and on a per-chat toggle it
//! wrote a `Map` in React state and raised a toast saying the skill was
//! "enabled for this chat". Nothing left the renderer: no request, no
//! `extension_data` write, no catalog refresh, no live agent. The switch moved,
//! the toast was green, and the next turn saw exactly the skills it saw before.
//!
//! A success toast that reports intent rather than confirmed state is worse
//! than no control at all, so the composer now goes through here, and the
//! handler's answer — not the click — is what moves the switch.
//!
//! # The three properties these handlers are built around
//!
//! **One catalog, served, not re-derived.** [`biorouter::agents::skill_catalog`]
//! enumerates the roots (all seven kinds, including
//! `~/.config/biorouter/extensions/<name>/skills`, which the renderer's own
//! scanner omitted), discovers the skills, and composes machine-wide with
//! per-session state. This route hands that over verbatim. There is no second
//! interpretation on the interface side; that separation was root cause 2.
//!
//! **A mutation is persisted before it is reported.**
//! [`biorouter::agents::session_skills::apply`] writes
//! `workspace_skills/v1` inside one transaction, and only then is a catalog
//! read back and returned. A failed write is a non-2xx with a message, and the
//! interface restores the switch from it — see `useSkillCatalog.ts`.
//!
//! **The write is never `skills-config.json`.** That file is the machine-wide
//! preference shared with `biorouter skill enable/disable` and every other
//! window; a per-chat toggle that touched it would change every other
//! conversation. The scoping rule is stated once, in `session_skills.rs`, and
//! this route is bound by it.
//!
//! # Why there is no `X-User-Action` gate
//!
//! Proof-of-user exists for raises the *model* must not perform on its own
//! (privacy Gate C's cross-affiliation grant). Enabling a skill in your own
//! chat is not one: it grants nothing the machine-wide catalog did not already
//! contain, it is scoped to a single conversation, and the model already has a
//! sanctioned route to the same state through `workspace_set_tools`. Requiring
//! the proof would also break browser access outright — `biorouter serve`
//! spawns the daemon with `Stdio::null()`, so no digest is ever installed
//! (`docs/deployment/serve-decisions.md`, SD-1). The secret-key middleware
//! guards these routes exactly as it guards `/config` and `/sessions`.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use biorouter::agents::session_skills::{self, SessionSkillOverride};
use biorouter::agents::skill_catalog::{self, CatalogView, PackageSummary};
use biorouter::agents::skill_package::{
    self, pending, ImportPlan, ImportPreview, ImportSource, InstalledPackage,
};
use biorouter::session::SessionManager;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::state::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SkillCatalogQuery {
    /// Compose the machine-wide catalog with this conversation's override.
    /// Omit for the machine-wide view — what a new chat would start with.
    pub session_id: Option<String>,
    /// Rescan the filesystem before answering, instead of reusing the cached
    /// snapshot. The interface sets this after an install it did not perform
    /// itself (a marketplace click, a `.brxt` drop) and after a `CatalogChanged`
    /// notice, since a change made by another process may land inside the
    /// snapshot's one-second mtime window.
    #[serde(default)]
    pub refresh: bool,
}

/// Add and remove are applied to **this** conversation only.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionSkillsRequest {
    pub session_id: String,
    /// Skill (or bundle) names to enable for this conversation, even where the
    /// machine-wide preference has them off.
    #[serde(default)]
    pub add: Vec<String>,
    /// Skill (or bundle) names to disable for this conversation, even where the
    /// machine-wide preference has them on.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// The catalog after the write, plus the override that produced it.
///
/// Returning the whole catalog rather than an acknowledgement is deliberate: it
/// is what lets the interface render confirmed state instead of the optimistic
/// state it just guessed, and it collapses the toggle-then-refetch race that
/// two concurrent toggles would otherwise lose.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionSkillsResponse {
    pub catalog: CatalogView,
    /// The persisted `workspace_skills/v1` value, echoed so a caller can tell a
    /// per-chat deviation from a machine-wide default without re-deriving it.
    pub session_add: Vec<String>,
    pub session_remove: Vec<String>,
}

async fn override_for(
    session_manager: &SessionManager,
    session_id: Option<&str>,
) -> Result<SessionSkillOverride, (StatusCode, String)> {
    let Some(session_id) = session_id else {
        return Ok(SessionSkillOverride::default());
    };
    session_skills::for_session(session_manager, session_id)
        .await
        .map_err(|e| {
            // Fail CLOSED, exactly as the extension's dispatch does: answering
            // from the machine-wide view would show every skill this
            // conversation had revoked as enabled.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not read this conversation's skill state: {e:#}"),
            )
        })
}

/// The canonical catalog: every root, every skill, every bundle, and the
/// composed enablement for one conversation.
#[utoipa::path(
    get,
    path = "/skills/catalog",
    params(SkillCatalogQuery),
    responses(
        (status = 200, description = "The catalog", body = CatalogView),
        (status = 401, description = "Unauthorized - invalid or missing secret key"),
        (status = 500, description = "This conversation's skill state is unreadable"),
    ),
    security(("api_key" = [])),
    tag = "skills",
)]
pub async fn skill_catalog_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SkillCatalogQuery>,
) -> Result<Json<CatalogView>, (StatusCode, String)> {
    let over = override_for(state.session_manager(), query.session_id.as_deref()).await?;
    let catalog = if query.refresh {
        skill_catalog::refresh()
    } else {
        skill_catalog::current()
    };
    Ok(Json(catalog.view(&over)))
}

/// Enable or disable skills for one conversation, and hand back what the model
/// will see on its next turn.
#[utoipa::path(
    post,
    path = "/skills/session",
    request_body = SessionSkillsRequest,
    responses(
        (status = 200, description = "Applied", body = SessionSkillsResponse),
        (status = 401, description = "Unauthorized - invalid or missing secret key"),
        (status = 403, description = "Refused by a privacy boundary: `sessionId` names a chat \
                                      this caller may not reach, answered with the same refusal, \
                                      word for word, that `GET /sessions/{session_id}` gives \
                                      (body = plain text)"),
        (status = 404, description = "No such conversation"),
        (status = 500, description = "The override could not be persisted"),
    ),
    security(("api_key" = [])),
    tag = "skills",
)]
pub async fn set_session_skills(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<SessionSkillsRequest>,
) -> Result<Json<SessionSkillsResponse>, (StatusCode, String)> {
    // Issue #56, QA 2026-09-10 (F0's sweep). Enabling a skill in a chat puts its
    // instructions into that chat's next turn — a write into the chat — so a
    // caller the read refuses may not do it. Asked first, before the request is
    // validated against anything the chat holds.
    crate::routes::session_reach::session_reach(
        state.session_manager(),
        &request.session_id,
        &headers,
    )
    .await
    .map_err(|refusal| (refusal.status, refusal.message.to_string()))?;
    if request.add.is_empty() && request.remove.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "name at least one skill to add or remove".to_string(),
        ));
    }

    let over = session_skills::apply(
        state.session_manager(),
        &request.session_id,
        &request.add,
        &request.remove,
    )
    .await
    .map_err(|e| {
        // `apply` reports a missing session as an error rather than a silent
        // grant, and that case is the caller's mistake, not the daemon's.
        let missing = format!("{e:#}").contains("not found");
        let status = if missing {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, format!("{e:#}"))
    })?;

    // Read the catalog only after the write committed, so what comes back is
    // confirmed state rather than the optimistic answer the caller already has.
    let catalog = skill_catalog::current();
    Ok(Json(SessionSkillsResponse {
        catalog: catalog.view(&over),
        session_add: over.add,
        session_remove: over.remove,
    }))
}

/// Rescan the filesystem and publish a new catalog to every live conversation.
///
/// The endpoint an install calls — a marketplace skill, a dropped `.zip`, a
/// `.brxt` extension whose `skills/` subdirectory becomes a new root, or
/// worktree 4's `CatalogChanged` notice. It rescans unconditionally rather than
/// consulting mtimes, because the caller already knows something changed.
#[utoipa::path(
    post,
    path = "/skills/refresh",
    params(SkillCatalogQuery),
    responses(
        (status = 200, description = "The freshly scanned catalog", body = CatalogView),
        (status = 401, description = "Unauthorized - invalid or missing secret key"),
        (status = 500, description = "This conversation's skill state is unreadable"),
    ),
    security(("api_key" = [])),
    tag = "skills",
)]
pub async fn refresh_skill_catalog(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SkillCatalogQuery>,
) -> Result<Json<CatalogView>, (StatusCode, String)> {
    let over = override_for(state.session_manager(), query.session_id.as_deref()).await?;
    Ok(Json(skill_catalog::refresh().view(&over)))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/skills/catalog", get(skill_catalog_handler))
        .route("/skills/session", post(set_session_skills))
        .route("/skills/refresh", post(refresh_skill_catalog))
        .route("/skills/packages/preview", post(preview_skill_package))
        .route("/skills/packages/install", post(install_skill_package))
        .route("/skills/packages/remove", post(remove_skill_package))
        .with_state(state)
}

#[cfg(test)]
mod tests {

    /// The per-chat write must never reach the machine-wide preference file.
    /// Stated as a source assertion because the handler's own body is where the
    /// mistake would be made, and a behavioural test would need the developer's
    /// real `~/.config/biorouter` to prove a negative about it.
    #[test]
    fn the_session_handler_writes_only_the_session_row() {
        let src = include_str!("skills.rs");
        let body = crate::routes::body_of(
            src,
            "async fn set_session_skills(\n    State(state): State<Arc<AppState>>,",
        );
        assert!(
            body.contains("session_skills::apply("),
            "the persisted write is `session_skills::apply`"
        );
        assert!(
            !body.contains("skills-config.json"),
            "a per-chat toggle must not touch the machine-wide preference file"
        );
        // Negative control: the extractor stopped inside this handler and did
        // not run on to the refresh handler next door.
        assert!(!body.contains("fn refresh_skill_catalog"));
    }

    /// Reading the catalog back *after* the write is what makes the response
    /// confirmed state rather than the caller's optimistic guess.
    #[test]
    fn the_catalog_is_read_after_the_write_not_before() {
        let src = include_str!("skills.rs");
        let body = crate::routes::body_of(
            src,
            "async fn set_session_skills(\n    State(state): State<Arc<AppState>>,",
        );
        let write = body.find("session_skills::apply(").expect("write present");
        let read = body
            .find("let catalog = skill_catalog::current();")
            .expect("read present");
        assert!(write < read, "the write must precede the read");
    }

    /// A delete handler must not take its directory from the caller. The root
    /// comes from the daemon's own enumeration, and a path outside it is
    /// refused rather than resolved.
    #[test]
    fn the_remove_handler_chooses_its_root_from_the_catalogs_own_list() {
        let src = include_str!("skills.rs");
        let body = crate::routes::body_of(
            src,
            "pub async fn remove_skill_package(\n    Json(request): Json<RemovePackageRequest>,",
        );
        assert!(
            body.contains("skill_catalog::roots()"),
            "the root set comes from the catalog"
        );
        assert!(
            body.contains("is not one of this machine's skill directories"),
            "an unknown root is refused, not resolved"
        );
        assert!(
            !body.contains("PathBuf::from(requested)"),
            "a caller-supplied path must never become the delete target"
        );
    }

    /// `refresh` rescans; `catalog` may reuse the cached snapshot. Swapping
    /// them would make the post-install refresh a no-op that looks like one.
    #[test]
    fn refresh_rescans_and_the_plain_read_does_not_have_to() {
        let src = include_str!("skills.rs");
        let refresh = crate::routes::body_of(
            src,
            "async fn refresh_skill_catalog(\n    State(state): State<Arc<AppState>>,",
        );
        assert!(refresh.contains("skill_catalog::refresh()"));
        assert!(!refresh.contains("skill_catalog::current()"));
    }
}

// ---------------------------------------------------------------------------
// Package import (#115).
// ---------------------------------------------------------------------------

/// What to import, in the two forms a caller has.
///
/// One request type for a pasted repository URL, an agent's tool call, a local
/// `.zip` and a marketplace asset, because giving each of those its own
/// resolution is how the four came to disagree in the first place.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    /// `https://github.com/owner/repo`, a `/tree/<ref>` URL, or a direct
    /// archive URL on an allowed host.
    pub url: Option<String>,
    /// A `.zip` on the machine the daemon runs on.
    pub file_path: Option<String>,
    /// Branch, tag or commit. Overrides a ref in the URL.
    pub reference: Option<String>,
    /// Answer to a previous preview's question, by its `planId`.
    pub plan_id: Option<String>,
    /// How to resolve an ambiguity: `bundle`, or `individual` with `components`.
    pub choice: Option<ImportChoice>,
    /// Which components to keep when `choice` is `individual`.
    #[serde(default)]
    pub components: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ImportChoice {
    Bundle,
    Individual,
}

/// The answer to an import request: either it happened, or somebody has to
/// choose first.
///
/// ⚠ **`NeedsChoice` is a 200, not an error.** It is a legitimate outcome the
/// caller is expected to act on — the issue's "real pending user-input state" —
/// and an agent that saw a 4xx would reasonably retry the same call rather than
/// asking the person the question it was handed.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ImportResult {
    #[serde(rename_all = "camelCase")]
    Installed {
        /// One entry per installed unit: a bundle is one, "install these
        /// separately" is one each.
        installed: Vec<InstalledPackage>,
        preview: ImportPreview,
    },
    #[serde(rename_all = "camelCase")]
    NeedsChoice {
        /// Pass this back with a `choice` to answer.
        plan_id: String,
        preview: ImportPreview,
    },
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemovePackageRequest {
    /// The install directory name — a `CatalogBundle.name`, or the last
    /// component of a `CatalogSkill.slug`.
    pub id: String,
    /// Which skills root it lives under. Omit for the Biorouter one.
    ///
    /// ⚠ **Validated against [`skill_catalog::roots`], not merely resolved.**
    /// This handler deletes a directory tree, so the root is chosen from the
    /// set the daemon itself enumerated rather than taken from the caller. A
    /// path the caller invents matches nothing and is refused — which is why
    /// this can safely cover `~/.claude/skills` and a project directory, the
    /// two the Skills pane has always offered a Delete for.
    pub source_root: Option<String>,
}

fn bad_request(message: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.to_string())
}

/// Resolve a request into a plan, either by fetching or by taking the parked
/// one it answers.
async fn resolve_plan(request: &ImportRequest) -> Result<ImportPlan, (StatusCode, String)> {
    if let Some(plan_id) = request.plan_id.as_deref() {
        return pending::take(plan_id).ok_or_else(|| {
            // Expired or already answered. Never fall through to a fresh fetch:
            // the answer was given about a specific archive, and a branch moves.
            (
                StatusCode::GONE,
                format!(
                    "that import preview ({plan_id}) has expired or was already answered. \
                     Preview the source again."
                ),
            )
        });
    }

    let source = match (request.url.as_deref(), request.file_path.as_deref()) {
        (Some(url), None) => ImportSource::Url {
            url: url.to_string(),
            reference: request.reference.clone(),
        },
        (None, Some(path)) => ImportSource::Archive {
            path: std::path::PathBuf::from(path),
        },
        (Some(_), Some(_)) => return Err(bad_request("name either a url or a filePath, not both")),
        (None, None) => {
            return Err(bad_request(
                "name a url, a filePath, or the planId of a preview to answer",
            ))
        }
    };

    let fetched = skill_package::fetch(&source)
        .await
        .map_err(|e| bad_request(format!("{e:#}")))?;
    skill_package::plan_from_entries(fetched.entries, &fetched.id_hints, fetched.source)
        .map_err(|e| bad_request(format!("{e:#}")))
}

/// Look at a source without installing anything.
#[utoipa::path(
    post,
    path = "/skills/packages/preview",
    request_body = ImportRequest,
    responses(
        (status = 200, description = "What an install would do", body = ImportResult),
        (status = 400, description = "The source could not be read"),
        (status = 401, description = "Unauthorized - invalid or missing secret key"),
        (status = 410, description = "That preview has expired"),
    ),
    security(("api_key" = [])),
    tag = "skills",
)]
pub async fn preview_skill_package(
    Json(request): Json<ImportRequest>,
) -> Result<Json<ImportResult>, (StatusCode, String)> {
    let plan = resolve_plan(&request).await?;
    let preview = plan.preview();
    Ok(Json(ImportResult::NeedsChoice {
        plan_id: pending::park(plan),
        preview,
    }))
}

/// Import a skill package, atomically, and refresh the catalog.
#[utoipa::path(
    post,
    path = "/skills/packages/install",
    request_body = ImportRequest,
    responses(
        (status = 200, description = "Installed, or a question to answer", body = ImportResult),
        (status = 400, description = "The source could not be read or installed"),
        (status = 401, description = "Unauthorized - invalid or missing secret key"),
        (status = 410, description = "That preview has expired"),
    ),
    security(("api_key" = [])),
    tag = "skills",
)]
pub async fn install_skill_package(
    Json(request): Json<ImportRequest>,
) -> Result<Json<ImportResult>, (StatusCode, String)> {
    let plan = resolve_plan(&request).await?;

    let plans = match (request.choice, plan.ambiguity.is_some()) {
        (Some(ImportChoice::Individual), _) => {
            let keep = if request.components.is_empty() {
                plan.components.iter().map(|c| c.name.clone()).collect()
            } else {
                request.components.clone()
            };
            let picked = plan.clone().into_individual(&keep);
            if picked.is_empty() {
                return Err(bad_request(
                    "none of the named components are in this package",
                ));
            }
            picked
        }
        (Some(ImportChoice::Bundle), _) => vec![plan.clone().as_bundle()],
        // Unambiguous and no choice given: install it as detected. This is what
        // makes an explicit manifest a one-call install rather than a dialog
        // per child.
        (None, false) => vec![plan.clone()],
        // Ambiguous and no choice given: ask. Deliberately a 200 — see
        // `ImportResult`.
        (None, true) => {
            let preview = plan.preview();
            return Ok(Json(ImportResult::NeedsChoice {
                plan_id: pending::park(plan),
                preview,
            }));
        }
    };

    let root = skill_package::install::install_root();
    let mut installed = Vec::new();
    for plan in &plans {
        installed
            .push(skill_package::install(plan, &root).map_err(|e| bad_request(format!("{e:#}")))?);
    }
    Ok(Json(ImportResult::Installed {
        preview: plan.preview(),
        installed,
    }))
}

/// Remove an installed package or single skill, and every component with it.
#[utoipa::path(
    post,
    path = "/skills/packages/remove",
    request_body = RemovePackageRequest,
    responses(
        (status = 200, description = "Removed", body = PackageSummary),
        (status = 401, description = "Unauthorized - invalid or missing secret key"),
        (status = 404, description = "No such package"),
    ),
    security(("api_key" = [])),
    tag = "skills",
)]
pub async fn remove_skill_package(
    Json(request): Json<RemovePackageRequest>,
) -> Result<Json<PackageSummary>, (StatusCode, String)> {
    let root = match request.source_root.as_deref() {
        None => skill_package::install::install_root(),
        Some(requested) => skill_catalog::roots()
            .into_iter()
            .map(|root| root.path)
            .find(|path| path.as_os_str() == requested)
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("`{requested}` is not one of this machine's skill directories"),
                )
            })?,
    };
    skill_package::remove(&request.id, &root)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e:#}")))
}
