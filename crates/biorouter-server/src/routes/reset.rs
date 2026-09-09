use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
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
        (status = 500, description = "Could not inspect reset data", body = ResetErrorResponse)
    ),
    tag = "App Reset"
)]
pub async fn preview_reset(
    State(state): State<Arc<AppState>>,
) -> ResetResult<ResetPreviewResponse> {
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
        (status = 409, description = "Reset is blocked by active work", body = ResetErrorResponse),
        (status = 500, description = "Reset failed", body = ResetErrorResponse)
    ),
    tag = "App Reset"
)]
pub async fn reset_app_data(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResetRequest>,
) -> ResetResult<ResetResponse> {
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
