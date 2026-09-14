use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use biorouter::agents::{count_user_skills, reset_to_builtin_skills};
use biorouter::config::paths::Paths;
use biorouter::knowledge::soul::{
    MEDITATION_SCHEDULE_ID, MEDITATION_WORKFLOW_FILE, MEDITATION_WORKFLOW_YAML, SOUL_COLOR,
    SOUL_KB_ID, SOUL_KB_NAME,
};
use biorouter::workflow::local_workflows::get_workflow_library_dir;
use biorouter::workflow::WORKFLOW_FILE_EXTENSIONS;
use biorouter_mcp::agent_drafter::{default_root, store::ArtifactStore};
use biorouter_mcp::knowledge::service::{KnowledgeService, PrimaryUpdate};
use biorouter_mcp::knowledge::types::KbFormat;
// `src/routes/` is compiled into the `biorouterd` binary as well as the lib and
// cannot name `crate::auth`, so this is the shared direction — the same import
// `routes::session` and `routes::knowledge` use.
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResetCategory {
    Applications,
    Knowledge,
    Skills,
    Extensions,
    Schedules,
    Workflows,
    History,
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResetCounts {
    pub applications: u64,
    pub knowledge_bases: u64,
    pub skills: u64,
    pub extensions: u64,
    pub schedules: u64,
    pub workflows: u64,
    pub conversations: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResetPreviewResponse {
    pub counts: ResetCounts,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetRequest {
    pub categories: Vec<ResetCategory>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResetResponse {
    pub reset: Vec<ResetCategory>,
    pub removed: ResetCounts,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResetErrorResponse {
    pub message: String,
}

type ResetError = (StatusCode, Json<ResetErrorResponse>);
type ResetOperationResult<T> = std::result::Result<T, ResetError>;
type ResetResult<T> = ResetOperationResult<Json<T>>;

fn api_error(
    status: StatusCode,
    error: impl std::fmt::Display,
) -> (StatusCode, Json<ResetErrorResponse>) {
    tracing::error!("App data reset failed: {error}");
    (
        status,
        Json(ResetErrorResponse {
            message: error.to_string(),
        }),
    )
}

/// What `POST /reset` and `GET /reset/preview` say to a caller that carried no
/// proof it is the person at the keyboard, on a daemon that holds a key to check
/// one against (issue #56 DR-16).
///
/// ⚠ **Holding the daemon secret does not make a caller the user.** This route
/// used to take no headers at all, on the premise that only the user's own
/// Settings page reaches it. A public chat's shell recovers the secret with
/// `ps eww` (AR-11), and with it `{"categories":["history"]}` ran
/// `SessionManager::clear_all_sessions` — every chat, private ones included —
/// while `GET /sessions/{id}` for the same private chat answered 403 (measured
/// on a sandboxed daemon, 2026-09-14). `knowledge` did the same to every base,
/// private ones included, while each base's own routes refused the caller.
/// `DELETE /sessions/{id}` was closed for exactly this in QA's F0 sweep; this is
/// its machine-wide twin, and it had been left open.
///
/// ⚠ **The proof, not a stated capability.** The reach gate
/// (`routes::session_reach`) admits a caller whose stated provider is private,
/// because *reading* a chat is a question about what a model may see. A reset
/// asks a different question — whether to destroy the machine's data — and a
/// capability is a fact about a model, never a decision. So this is the
/// declassify rule, not the reach rule: `X-User-Action` and nothing else. It
/// does not depend on DR-15's master switch either, for the same reason: what is
/// being decided is not a tier.
///
/// ⚠ It carries NEITHER renderer marker (`USER_ACTION_REFUSAL_MARKER`,
/// `COPY_OF_PRIVATE_REFUSAL_MARKER`): each of those opens a toast that sends the
/// user to a control that cannot help here. Pinned by
/// `tests::the_refusals_carry_no_renderer_marker`.
///
/// ⚠ It is fixed text and names nothing on the machine, so it tells a refused
/// caller nothing about what a reset would have removed — which is why the
/// preview is refused with it too, rather than answering with counts.
pub const RESET_NEEDS_USER: &str =
    "Resetting Biorouter's data deletes it for good, and only the person at the keyboard can \
     decide to do that. This request carried no proof it came from them. Nothing was read and \
     nothing was deleted. Do not retry; the same call will be refused again. If this data \
     genuinely needs to be cleared, stop and ask the user to reset it from Settings in the \
     Biorouter app.";

/// …and when this daemon was handed no user-action key at all — `biorouter
/// serve` (SD-7), `just run-server`, a hand-run `biorouterd agent`.
///
/// A separate sentence for the reason `DECLASSIFY_NO_USER_KEY` is one: on such a
/// daemon every caller lands here, the person at the keyboard included, and
/// [`RESET_NEEDS_USER`]'s closing advice — ask the user to reset it from Settings
/// — is a loop when the reader IS that user, in Settings. So it names the daemon
/// as the reason and says where the control does work: on the machine the data
/// lives on.
///
/// ⚠ **It must not be softened into admitting the browser.** A `serve` page's
/// cookie earns its operator's reach on listings (SD-10) and is explicitly not a
/// proof of a person; admitting it here would admit anything that can read the
/// daemon's environment, which is where that token is.
pub const RESET_NO_USER_KEY: &str =
    "This daemon was started without a user-action key, so it cannot verify that a request came \
     from the person at the keyboard, and resetting Biorouter's data requires that proof. Nothing \
     was read and nothing was deleted. Do not retry; this control is unavailable on this daemon. \
     It is available on the machine running the daemon, in Settings in the Biorouter app there. \
     From a terminal on that machine, `biorouter session remove`, `biorouter schedule remove`, \
     `biorouter skill remove` and `biorouter extension remove` delete items one at a time.";

/// Which refusal these routes owe a caller, or `None` when the proof is good.
///
/// Pure, so all three verdicts are driven from `--lib` without an `AppState`.
/// The end-to-end measurement lives in two integration binaries, one per
/// credential state, because the installed digest is a process-global
/// `OnceLock`: `tests/reset_requires_user.rs` and `tests/reset_no_user_key.rs`.
///
/// ⚠ It reads [`user_action_proof`], not `is_user_action`: the boolean form
/// collapses `Unproven` and `NoKeyInstalled`, and that collapse is what once put
/// an agent's sentence in front of a person on `biorouter serve` (SD-8).
fn reset_refusal(proof: UserActionProof) -> Option<&'static str> {
    match proof {
        UserActionProof::Proven => None,
        UserActionProof::Unproven => Some(RESET_NEEDS_USER),
        UserActionProof::NoKeyInstalled => Some(RESET_NO_USER_KEY),
    }
}

/// A refusal in this route's own error envelope. Not [`api_error`], which logs
/// at `error`: a refused caller is the gate working, not the reset failing.
fn refused(message: &'static str) -> ResetError {
    tracing::warn!("App data reset refused: the request carried no proof of the user");
    (
        StatusCode::FORBIDDEN,
        Json(ResetErrorResponse {
            message: message.to_string(),
        }),
    )
}

fn is_managed_workflow(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| WORKFLOW_FILE_EXTENSIONS.contains(&extension))
}

fn count_user_workflows(directory: &Path) -> u64 {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            is_managed_workflow(&entry.path())
                && entry.file_name().to_string_lossy() != MEDITATION_WORKFLOW_FILE
        })
        .count() as u64
}

fn reset_applications(root: &Path) -> Result<u64> {
    let count = ArtifactStore::new(root.to_path_buf()).list().len() as u64;
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    Ok(count)
}

fn reset_knowledge(service: &KnowledgeService, memory_root: &Path) -> Result<u64> {
    let bases = service.list_bases()?;
    let count = bases.len() as u64;
    for base in bases {
        service.delete_base(&base.id)?;
    }
    service.create_base_in(SOUL_KB_ID, SOUL_KB_NAME, Some(SOUL_COLOR), KbFormat::Okf)?;
    service.set_selection(None, None, PrimaryUpdate::Inherit)?;
    if memory_root.exists() {
        fs::remove_dir_all(memory_root)?;
    }
    Ok(count)
}

fn reset_extensions(extensions_root: &Path) -> Result<u64> {
    if extensions_root.exists() {
        fs::remove_dir_all(extensions_root)?;
    }
    // ⚠ The install claims are a SIBLING of the extensions root — deliberately,
    // so a bundle cannot forge one through `extract_to` — which means
    // `remove_dir_all(extensions_root)` does not reach them. A factory reset
    // that left them would leave every claim pointing at a tree that no longer
    // exists. `read_claims` self-cleans such a claim on its next read, so this
    // is tidiness rather than correctness; doing it here is what stops a reset
    // reporting "done" while the state it was asked to clear is still on disk.
    let claims = biorouter::extension_install::claim::claims_dir();
    if claims.exists() {
        fs::remove_dir_all(&claims)?;
    }
    Ok(biorouter::config::extensions::reset_to_bundled_extensions()? as u64)
}

fn reset_workflows(directory: &Path) -> Result<u64> {
    let count = count_user_workflows(directory);
    if directory.exists() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if is_managed_workflow(&entry.path()) {
                fs::remove_file(entry.path())?;
            }
        }
    }
    fs::create_dir_all(directory)?;
    fs::write(
        directory.join(MEDITATION_WORKFLOW_FILE),
        MEDITATION_WORKFLOW_YAML,
    )?;
    Ok(count)
}

fn count_user_extensions() -> u64 {
    biorouter::config::get_all_extensions()
        .into_iter()
        .filter(|entry| !entry.config.is_bundled())
        .count() as u64
}

#[utoipa::path(
    get,
    path = "/reset/preview",
    responses(
        (status = 200, description = "Counts of data affected by each reset category", body = ResetPreviewResponse),
        (status = 403, description = "Refused: a reset is the user's own decision, and the request \
                                      carried no proof it came from them, or this daemon holds no \
                                      user-action key at all. Nothing was counted", body = ResetErrorResponse),
        (status = 500, description = "Could not inspect reset data", body = ResetErrorResponse)
    ),
    tag = "App Reset"
)]
pub async fn preview_reset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ResetResult<ResetPreviewResponse> {
    // FIRST. The counts include every private chat and every private knowledge
    // base on the machine — rows `GET /sessions` and `GET /knowledge/bases` omit
    // for this same caller — and their only purpose is to show the person what a
    // reset would remove, which is a reset this caller could not perform.
    if let Some(refusal) = reset_refusal(user_action_proof(&headers)) {
        return Err(refused(refusal));
    }

    let knowledge_service = Arc::clone(&state.knowledge_service);
    let sync_counts = tokio::task::spawn_blocking(move || -> Result<(u64, u64, u64, u64, u64)> {
        Ok((
            ArtifactStore::new(default_root()).list().len() as u64,
            knowledge_service.list_bases()?.len() as u64,
            count_user_skills() as u64,
            count_user_extensions(),
            count_user_workflows(&get_workflow_library_dir(true)),
        ))
    })
    .await
    .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
    .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;

    let schedules = state
        .scheduler()
        .list_scheduled_jobs()
        .await
        .into_iter()
        .filter(|job| job.id != MEDITATION_SCHEDULE_ID)
        .count() as u64;
    let conversations = state
        .session_manager()
        .count_all_sessions()
        .await
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;

    Ok(Json(ResetPreviewResponse {
        counts: ResetCounts {
            applications: sync_counts.0,
            knowledge_bases: sync_counts.1,
            skills: sync_counts.2,
            extensions: sync_counts.3,
            schedules,
            workflows: sync_counts.4,
            conversations,
        },
    }))
}

async fn reset_schedules(state: &AppState) -> Result<u64> {
    let scheduler = state.scheduler();
    let jobs = scheduler.list_scheduled_jobs().await;
    let count = jobs
        .iter()
        .filter(|job| job.id != MEDITATION_SCHEDULE_ID)
        .count() as u64;
    for job in jobs {
        // `true` asks for the workflow copy; the scheduler decides whether there
        // is one. This used to compute `source.starts_with(scheduled_workflows)`
        // here — the right rule, in the wrong place: it was the ONLY caller that
        // knew it, so `delete_schedule` next door passed a literal `true` and
        // deleted the user's own workflow. The rule now lives once, beside the
        // ownership record, in `scheduler::scheduler_owns_source`.
        scheduler
            .remove_scheduled_job(&job.id, true)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
    }
    let config_dir = biorouter::config::paths::Paths::config_dir();
    let workflow_path = tokio::task::spawn_blocking(move || {
        biorouter::knowledge::soul::ensure_meditation_workflow(&config_dir)
    })
    .await?;
    let workflow_path = workflow_path?;
    biorouter::knowledge::soul::ensure_meditation_schedule(&scheduler, workflow_path).await?;
    Ok(count)
}

async fn prepare_for_reset(
    state: &AppState,
    categories: &HashSet<ResetCategory>,
) -> ResetOperationResult<()> {
    if categories.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Select at least one reset category",
        ));
    }
    if state.has_active_turns()
        || state
            .scheduler()
            .list_scheduled_jobs()
            .await
            .iter()
            .any(|job| job.currently_running)
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "Finish or stop active chats and scheduled runs before resetting",
        ));
    }
    if categories.iter().any(|category| {
        matches!(
            category,
            ResetCategory::Knowledge
                | ResetCategory::Skills
                | ResetCategory::Extensions
                | ResetCategory::Workflows
                | ResetCategory::History
        )
    }) {
        state.clear_cached_agents().await;
    }
    Ok(())
}

async fn reset_selected_categories(
    state: &AppState,
    categories: &HashSet<ResetCategory>,
) -> ResetOperationResult<ResetCounts> {
    let mut removed = ResetCounts::default();

    if categories.contains(&ResetCategory::Applications) {
        removed.applications = tokio::task::spawn_blocking(|| reset_applications(&default_root()))
            .await
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    if categories.contains(&ResetCategory::Knowledge) {
        let knowledge_service = Arc::clone(&state.knowledge_service);
        removed.knowledge_bases = tokio::task::spawn_blocking(move || {
            reset_knowledge(&knowledge_service, &Paths::config_dir().join("memory"))
        })
        .await
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    if categories.contains(&ResetCategory::Skills) {
        removed.skills = tokio::task::spawn_blocking(reset_to_builtin_skills)
            .await
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
            as u64;
    }
    if categories.contains(&ResetCategory::Extensions) {
        removed.extensions = tokio::task::spawn_blocking(|| {
            reset_extensions(&Paths::config_dir().join("extensions"))
        })
        .await
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    if categories.contains(&ResetCategory::Workflows) {
        biorouter::slash_commands::remove_commands_for_directory(&get_workflow_library_dir(true))
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        removed.workflows =
            tokio::task::spawn_blocking(|| reset_workflows(&get_workflow_library_dir(true)))
                .await
                .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
                .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    if categories.contains(&ResetCategory::Schedules) {
        removed.schedules = reset_schedules(state)
            .await
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    if categories.contains(&ResetCategory::History) {
        removed.conversations = state
            .session_manager()
            .clear_all_sessions()
            .await
            .context("clearing conversation and usage history")
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let checkpoints = Paths::data_dir().join("checkpoints");
        tokio::task::spawn_blocking(move || {
            if checkpoints.exists() {
                fs::remove_dir_all(checkpoints)?;
            }
            Ok::<_, std::io::Error>(())
        })
        .await
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }

    Ok(removed)
}

#[utoipa::path(
    post,
    path = "/reset",
    request_body = ResetRequest,
    responses(
        (status = 200, description = "Selected app data was reset", body = ResetResponse),
        (status = 400, description = "No reset category was selected", body = ResetErrorResponse),
        (status = 403, description = "Refused: a reset is the user's own decision, and the request \
                                      carried no proof it came from them, or this daemon holds no \
                                      user-action key at all. Nothing was deleted", body = ResetErrorResponse),
        (status = 409, description = "Reset is blocked by active work", body = ResetErrorResponse),
        (status = 500, description = "Reset failed", body = ResetErrorResponse)
    ),
    tag = "App Reset"
)]
pub async fn reset_app_data(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: HeaderMap,
    Json(request): Json<ResetRequest>,
) -> ResetResult<ResetResponse> {
    // FIRST, before the category check, the active-work check and the agent
    // cache flush in `prepare_for_reset`. Each of those answers something — a
    // 400 for an empty selection, a 409 that says a chat or a scheduled run is
    // in flight right now — and an unproven caller is owed none of it. See
    // `RESET_NEEDS_USER` for what this closed.
    if let Some(refusal) = reset_refusal(user_action_proof(&headers)) {
        return Err(refused(refusal));
    }

    let categories = request.categories.into_iter().collect::<HashSet<_>>();
    prepare_for_reset(&state, &categories).await?;
    let removed = reset_selected_categories(&state, &categories).await?;

    let mut reset = categories.into_iter().collect::<Vec<_>>();
    reset.sort_by_key(|category| match category {
        ResetCategory::Applications => 0,
        ResetCategory::Knowledge => 1,
        ResetCategory::Skills => 2,
        ResetCategory::Extensions => 3,
        ResetCategory::Schedules => 4,
        ResetCategory::Workflows => 5,
        ResetCategory::History => 6,
    });
    Ok(Json(ResetResponse { reset, removed }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/reset/preview", get(preview_reset))
        .route("/reset", post(reset_app_data))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::body_of;

    /// The rule, at every verdict: only a request that proves the person at the
    /// keyboard sent it gets past, and the two refusals are told apart so a
    /// person on a keyless daemon is never handed the sentence written for a
    /// model.
    #[test]
    fn only_a_proven_request_passes_the_reset_gate() {
        assert_eq!(reset_refusal(UserActionProof::Proven), None);
        assert_eq!(
            reset_refusal(UserActionProof::Unproven),
            Some(RESET_NEEDS_USER)
        );
        assert_eq!(
            reset_refusal(UserActionProof::NoKeyInstalled),
            Some(RESET_NO_USER_KEY)
        );
        assert_ne!(RESET_NEEDS_USER, RESET_NO_USER_KEY);

        // The keyless sentence must not send its reader to go and do what they
        // are already doing. Quoted out of the constant that owns the clause, so
        // a rewording cannot leave this passing against words no longer sent.
        let hand_it_to_the_user = RESET_NEEDS_USER
            .split_once("stop and ")
            .expect("the model-facing refusal has stopped delegating to the user")
            .1;
        assert!(!RESET_NO_USER_KEY.contains(hand_it_to_the_user));
        // …and it must say where the control does work.
        assert!(RESET_NO_USER_KEY.contains("machine running the daemon"));
    }

    /// Each renderer marker opens a toast with its own advice — switch this
    /// chat's model, branch it from the chat window — and neither helps a person
    /// whose reset was refused.
    #[test]
    fn the_refusals_carry_no_renderer_marker() {
        for refusal in [RESET_NEEDS_USER, RESET_NO_USER_KEY] {
            assert!(!refusal.contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER));
            assert!(!refusal.contains(crate::routes::session::COPY_OF_PRIVATE_REFUSAL_MARKER));
        }
    }

    /// The gate runs before either handler does anything else.
    ///
    /// The HTTP binaries prove the refusal happens and deletes nothing; what they
    /// cannot see cheaply is the ORDER, which is what keeps a refused caller from
    /// learning "a chat is running" (the 409) or "you selected nothing" (the 400)
    /// ahead of the 403, or from flushing every cached agent on the way to it. So
    /// that is a source scan, and it asserts the early return as well as the
    /// read — a verdict consulted and then ignored would pass a scan for the call
    /// alone.
    #[test]
    fn both_reset_routes_refuse_before_they_touch_anything() {
        let source = include_str!("reset.rs");
        let gate = "if let Some(refusal) = reset_refusal(user_action_proof(&headers)) {\n        \
                    return Err(refused(refusal));";
        for (handler, first_act) in [
            ("pub async fn reset_app_data(", "prepare_for_reset("),
            ("pub async fn preview_reset(", "spawn_blocking("),
        ] {
            let body = body_of(source, handler);
            let (before_the_gate, after_the_gate) = body
                .split_once(gate)
                .unwrap_or_else(|| panic!("`{handler}` no longer refuses on the proof verdict"));
            assert!(
                after_the_gate.contains(first_act),
                "`{handler}` reaches `{first_act}` BEFORE its proof gate, or not at all"
            );
            assert!(
                !before_the_gate.contains(first_act) && !before_the_gate.contains("state."),
                "`{handler}` touches app state before its proof gate"
            );
        }
        // The negative control, so the scan is provably not vacuous: a function
        // in the same file with no gate must come back without one, or `body_of`
        // is over-reading past a function end.
        assert!(
            !body_of(source, "async fn reset_schedules(").contains("user_action_proof("),
            "the body scan is over-reading: a function with no gate reported one"
        );
    }

    #[test]
    fn workflow_reset_keeps_only_a_fresh_factory_workflow() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("custom.yaml"), "title: custom").unwrap();
        fs::write(temp.path().join(MEDITATION_WORKFLOW_FILE), "modified").unwrap();
        fs::write(temp.path().join("notes.txt"), "keep").unwrap();

        assert_eq!(reset_workflows(temp.path()).unwrap(), 1);
        assert_eq!(
            fs::read_to_string(temp.path().join(MEDITATION_WORKFLOW_FILE)).unwrap(),
            MEDITATION_WORKFLOW_YAML
        );
        assert!(!temp.path().join("custom.yaml").exists());
        assert!(temp.path().join("notes.txt").exists());
    }

    #[test]
    fn knowledge_reset_recreates_one_empty_soul_base() {
        let temp = tempfile::tempdir().unwrap();
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("profile.json"), "{}").unwrap();
        let service = KnowledgeService::new(temp.path().join("knowledge"));
        service.create_base(SOUL_KB_ID, SOUL_KB_NAME, None).unwrap();
        service.create_base("custom", "Custom", None).unwrap();

        assert_eq!(reset_knowledge(&service, &memory).unwrap(), 2);
        let bases = service.list_bases().unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].id, SOUL_KB_ID);
        assert_eq!(
            service.primary_for_session(None).unwrap().as_deref(),
            Some(SOUL_KB_ID)
        );
        assert!(!memory.exists());
    }
}
