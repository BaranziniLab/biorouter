//! "What is the agent running now" — a unified read of active long-running work
//! plus a per-item cancel affordance (BR-42).
//!
//! Background shell jobs and running subagents register themselves in the
//! process-wide `biorouter_mcp::active_work` registry as they start; the
//! scheduler tracks its own in-flight runs. This route aggregates all three into
//! one list so the user (via a GUI panel, deferred) can see and stop
//! runaway/forgotten work. `GET /active_work` lists; `POST
//! /active_work/{id}/cancel` cancels one item, dispatched by its id.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::state::AppState;
use biorouter::scheduler::ScheduledJob;
use biorouter_mcp::active_work::{active_work, ActiveWorkItem};

/// Cancel ids for scheduler runs are namespaced so the cancel route can tell a
/// scheduled run apart from a registry entry (`bg-*` / `sub-*` / `dturn-*`).
const SCHED_PREFIX: &str = "sched:";

/// One active unit of work, kind-tagged so the GUI can render/route uniformly.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWorkItemDto {
    /// Unique id; also the handle for `POST /active_work/{id}/cancel`.
    pub id: String,
    /// `background_job`, `subagent`, `detached_turn`, or `scheduled_run`.
    pub kind: String,
    /// Short human-readable label (command, task prompt, or schedule id).
    pub title: String,
    /// Extra context (full command, child session id, workflow source).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Owning/related session id where known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// RFC3339 start time where known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Wall-clock seconds this item has been running where known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_for_seconds: Option<i64>,
    /// Whether `POST /active_work/{id}/cancel` can stop it.
    pub cancellable: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ActiveWorkResponse {
    pub items: Vec<ActiveWorkItemDto>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CancelActiveWorkResponse {
    pub message: String,
}

/// Where a cancel id points.
enum CancelTarget {
    /// A scheduler run, addressed by schedule id (prefix stripped).
    Scheduler(String),
    /// A background job or subagent in the active-work registry.
    Registry(String),
}

/// Route a cancel id: `sched:<schedule-id>` hits the scheduler; anything else
/// (`bg-*` / `sub-*` / `dturn-*`) hits the registry.
fn classify_cancel_id(id: &str) -> CancelTarget {
    match id.strip_prefix(SCHED_PREFIX) {
        Some(sched_id) => CancelTarget::Scheduler(sched_id.to_string()),
        None => CancelTarget::Registry(id.to_string()),
    }
}

fn registry_item_to_dto(item: ActiveWorkItem, now: DateTime<Utc>) -> ActiveWorkItemDto {
    let started = DateTime::<Utc>::from_timestamp_millis(item.started_at_epoch_ms as i64);
    ActiveWorkItemDto {
        id: item.id,
        kind: item.kind.as_str().to_string(),
        title: item.title,
        detail: item.detail,
        session_id: item.session_id,
        started_at: started.map(|t| t.to_rfc3339()),
        running_for_seconds: started.map(|t| now.signed_duration_since(t).num_seconds()),
        cancellable: item.cancellable,
    }
}

fn scheduled_run_to_dto(job: &ScheduledJob, now: DateTime<Utc>) -> ActiveWorkItemDto {
    ActiveWorkItemDto {
        id: format!("{SCHED_PREFIX}{}", job.id),
        kind: "scheduled_run".to_string(),
        title: job.id.clone(),
        detail: Some(job.source.clone()),
        session_id: job.current_session_id.clone(),
        started_at: job.process_start_time.map(|t| t.to_rfc3339()),
        running_for_seconds: job
            .process_start_time
            .map(|t| now.signed_duration_since(t).num_seconds()),
        cancellable: true,
    }
}

/// Merge registry entries and in-flight scheduler runs into one list. Pure so it
/// can be unit-tested without an `AppState`.
fn build_items(
    registry: Vec<ActiveWorkItem>,
    jobs: Vec<ScheduledJob>,
    now: DateTime<Utc>,
) -> Vec<ActiveWorkItemDto> {
    let mut items: Vec<ActiveWorkItemDto> = registry
        .into_iter()
        .map(|i| registry_item_to_dto(i, now))
        .collect();
    items.extend(
        jobs.iter()
            .filter(|j| j.currently_running)
            .map(|j| scheduled_run_to_dto(j, now)),
    );
    items
}

#[utoipa::path(
    get,
    path = "/active_work",
    responses(
        (status = 200, description = "Current background jobs, subagents, and in-flight scheduled runs", body = ActiveWorkResponse),
    ),
    tag = "active_work"
)]
#[axum::debug_handler]
async fn list_active_work(State(state): State<Arc<AppState>>) -> Json<ActiveWorkResponse> {
    let registry = active_work().list();
    let jobs = state.scheduler().list_scheduled_jobs().await;
    let items = build_items(registry, jobs, Utc::now());
    Json(ActiveWorkResponse { items })
}

#[utoipa::path(
    post,
    path = "/active_work/{id}/cancel",
    params(
        ("id" = String, Path, description = "Active-work item id from GET /active_work")
    ),
    responses(
        (status = 200, description = "Cancel requested", body = CancelActiveWorkResponse),
        (status = 404, description = "No such active-work item"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "active_work"
)]
#[axum::debug_handler]
async fn cancel_active_work(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CancelActiveWorkResponse>, StatusCode> {
    match classify_cancel_id(&id) {
        CancelTarget::Scheduler(sched_id) => {
            state
                .scheduler()
                .kill_running_job(&sched_id)
                .await
                .map_err(|e| match e {
                    biorouter::scheduler::SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
                    biorouter::scheduler::SchedulerError::AnyhowError(_) => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                })?;
            Ok(Json(CancelActiveWorkResponse {
                message: format!("Requested cancel of scheduled run '{sched_id}'"),
            }))
        }
        CancelTarget::Registry(reg_id) => {
            if active_work().cancel(&reg_id) {
                Ok(Json(CancelActiveWorkResponse {
                    message: format!("Requested cancel of '{reg_id}'"),
                }))
            } else {
                Err(StatusCode::NOT_FOUND)
            }
        }
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/active_work", get(list_active_work))
        .route("/active_work/{id}/cancel", post(cancel_active_work))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter_mcp::active_work::ActiveWorkKind;

    fn sched_job(id: &str, running: bool) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            source: format!("/workflows/{id}.yaml"),
            cron: "0 0 * * *".to_string(),
            last_run: None,
            currently_running: running,
            paused: false,
            current_session_id: running.then(|| format!("sess-{id}")),
            process_start_time: running.then(Utc::now),
            run_count: 1,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        }
    }

    fn reg_item(id: &str, kind: ActiveWorkKind, cancellable: bool) -> ActiveWorkItem {
        ActiveWorkItem {
            id: id.to_string(),
            kind,
            title: format!("title {id}"),
            detail: Some(format!("detail {id}")),
            session_id: Some("s1".to_string()),
            started_at_epoch_ms: 1_700_000_000_000,
            cancellable,
        }
    }

    #[test]
    fn build_items_merges_and_filters_idle_schedules() {
        let now = Utc::now();
        let registry = vec![
            reg_item("bg-1", ActiveWorkKind::BackgroundJob, true),
            reg_item("sub-2", ActiveWorkKind::Subagent, false),
        ];
        let jobs = vec![sched_job("nightly", true), sched_job("weekly", false)];

        let items = build_items(registry, jobs, now);

        // Two registry entries + only the running schedule.
        assert_eq!(items.len(), 3);
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["background_job", "subagent", "scheduled_run"]);

        let sched = items.iter().find(|i| i.kind == "scheduled_run").unwrap();
        assert_eq!(sched.id, "sched:nightly");
        assert_eq!(sched.title, "nightly");
        assert_eq!(sched.session_id.as_deref(), Some("sess-nightly"));
        assert!(sched.cancellable);
        assert!(sched.started_at.is_some());
    }

    #[test]
    fn registry_dto_preserves_cancellable_and_times() {
        let now = DateTime::<Utc>::from_timestamp_millis(1_700_000_010_000).unwrap();
        let dto = registry_item_to_dto(reg_item("bg-1", ActiveWorkKind::BackgroundJob, true), now);
        assert_eq!(dto.id, "bg-1");
        assert!(dto.cancellable);
        assert_eq!(dto.running_for_seconds, Some(10));
        assert!(dto.started_at.is_some());

        let dto2 = registry_item_to_dto(reg_item("sub-2", ActiveWorkKind::Subagent, false), now);
        assert!(!dto2.cancellable);
    }

    #[test]
    fn classify_cancel_id_routes_by_prefix() {
        assert!(matches!(
            classify_cancel_id("sched:nightly"),
            CancelTarget::Scheduler(s) if s == "nightly"
        ));
        assert!(matches!(
            classify_cancel_id("bg-3"),
            CancelTarget::Registry(s) if s == "bg-3"
        ));
        assert!(matches!(
            classify_cancel_id("sub-7"),
            CancelTarget::Registry(s) if s == "sub-7"
        ));
    }
}
