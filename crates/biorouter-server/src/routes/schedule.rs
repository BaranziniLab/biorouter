use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use biorouter::scheduler::{ScheduledJob, RUN_CANCELLED_MARKER};

#[derive(Deserialize, Serialize, utoipa::ToSchema)]
pub struct CreateScheduleRequest {
    id: String,
    workflow_source: String,
    cron: String,
}

#[derive(Deserialize, Serialize, utoipa::ToSchema)]
pub struct UpdateScheduleRequest {
    cron: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ListSchedulesResponse {
    jobs: Vec<ScheduledJob>,
}

// Response for the kill endpoint
#[derive(Serialize, utoipa::ToSchema)]
pub struct KillJobResponse {
    message: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InspectJobResponse {
    session_id: Option<String>,
    process_start_time: Option<String>,
    running_duration_seconds: Option<i64>,
}

// Response for the run_now endpoint
#[derive(Serialize, utoipa::ToSchema)]
pub struct RunNowResponse {
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct SessionsQuery {
    limit: usize,
}

// Struct for the frontend session list
#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDisplayInfo {
    id: String,
    name: String,
    created_at: String,
    working_dir: String,
    schedule_id: Option<String>,
    message_count: usize,
    total_tokens: Option<i32>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    accumulated_total_tokens: Option<i64>,
    accumulated_input_tokens: Option<i64>,
    accumulated_output_tokens: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/schedule/create",
    request_body = CreateScheduleRequest,
    responses(
        (status = 200, description = "Scheduled job created successfully", body = ScheduledJob),
        (status = 400, description = "Invalid schedule name, cron expression or workflow file", body = ErrorResponse),
        (status = 409, description = "Job ID already exists", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<Json<ScheduledJob>, ErrorResponse> {
    let scheduler = state.scheduler();

    // ⚠ The id names a FILE. `Path::join` throws its base away when the argument
    // is absolute and `..` resolves in the kernel, so an unvalidated `req.id`
    // was an arbitrary-file-write primitive on this route:
    // `{"id": "/tmp/pwned"}` wrote `/tmp/pwned.yaml`, and did so *before* the
    // cron was parsed and before the duplicate guard, so the file landed even
    // when this handler answered 400 or 409.
    //
    // Refused here as well as inside `add_scheduled_job` — the same function in
    // both places, so the two cannot drift — because a request this malformed
    // should not reach the scheduler at all.
    //
    // ⚠ The refusal carries the REASON. It used to answer a bare
    // `StatusCode::BAD_REQUEST` — a 400 with no body — and log the sentence
    // where only a developer would find it, so the client's JSON parse failed
    // and the dialog reported "Unexpected response format": a transport-shaped
    // error for a validation problem the user could have fixed in one keystroke.
    if let Err(error) = biorouter::scheduler::validate_schedule_id(&req.id) {
        tracing::warn!("Refusing schedule create with an invalid id: {error}");
        return Err(create_schedule_error(StatusCode::BAD_REQUEST, error));
    }
    tracing::info!(
        "Server: Calling scheduler.add_scheduled_job() for job '{}'",
        req.id
    );
    let job = ScheduledJob {
        id: req.id,
        source: req.workflow_source,
        cron: req.cron,
        last_run: None,
        currently_running: false,
        paused: false,
        current_session_id: None,
        process_start_time: None,
        run_count: 0,
        max_runs: None,
        // `POST /schedule/create` schedules a workflow file, not a chat.
        creator_session_id: None,
        last_error: None,
        owns_source: None,
    };
    scheduler
        .add_scheduled_job(job.clone(), true)
        .await
        .map_err(|e| {
            eprintln!("Error creating schedule: {:?}", e); // Log error
            create_schedule_error(create_schedule_status(&e), e)
        })?;
    Ok(Json(job))
}

/// The status a failed create answers with.
///
/// Split out from the handler so the mapping can be asserted on its own — the
/// handler needs a live `AppState` and a real scheduler, which is exactly the
/// reason the old inline `match` had no test at all.
fn create_schedule_status(error: &biorouter::scheduler::SchedulerError) -> StatusCode {
    use biorouter::scheduler::SchedulerError;
    match error {
        SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
        SchedulerError::CronParseError(_) => StatusCode::BAD_REQUEST,
        SchedulerError::WorkflowLoadError(_) => StatusCode::BAD_REQUEST,
        SchedulerError::JobIdExists(_) => StatusCode::CONFLICT,
        // Reached only when a future caller skips the guard at the top of the
        // handler, and mapped anyway so such a caller still gets "you sent
        // something bad", not "the server broke".
        SchedulerError::InvalidJobId(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// A failed create, as the dialog will read it.
///
/// `ErrorResponse` is the shape the rest of `routes/` answers errors in
/// (`{"message": …}`, see `routes::errors`), so the client parses one body for
/// every route rather than one per handler. `SchedulerError`'s `Display` is
/// already written for a person — *"schedule id '…' may only contain letters,
/// digits, '-' and '_'"* — which is the whole sentence that used to be dropped.
fn create_schedule_error(
    status: StatusCode,
    error: biorouter::scheduler::SchedulerError,
) -> ErrorResponse {
    ErrorResponse {
        message: error.to_string(),
        status,
    }
}

#[utoipa::path(
    get,
    path = "/schedule/list",
    responses(
        (status = 200, description = "A list of scheduled jobs", body = ListSchedulesResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn list_schedules(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ListSchedulesResponse>, StatusCode> {
    let scheduler = state.scheduler();

    tracing::info!("Server: Calling scheduler.list_scheduled_jobs()");
    let jobs = scheduler.list_scheduled_jobs().await;
    Ok(Json(ListSchedulesResponse { jobs }))
}

#[utoipa::path(
    delete,
    path = "/schedule/delete/{id}",
    params(
        ("id" = String, Path, description = "ID of the schedule to delete")
    ),
    responses(
        (status = 204, description = "Scheduled job deleted successfully"),
        (status = 404, description = "Scheduled job not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let scheduler = state.scheduler();
    // ⚠ `true` asks for the workflow file; it no longer grants it. This route
    // never reads the job, so it cannot tell the private copy the scheduler made
    // for itself from a pointer at the user's own workflow — and while the flag
    // WAS the permission, deleting a schedule added from a workflow row deleted
    // that workflow. `scheduler::scheduler_owns_source` now has the last word.
    scheduler
        .remove_scheduled_job(&id, true)
        .await
        .map_err(|e| {
            eprintln!("Error deleting schedule '{}': {:?}", id, e);
            match e {
                biorouter::scheduler::SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/schedule/{id}/run_now",
    params(
        ("id" = String, Path, description = "ID of the schedule to run")
    ),
    responses(
        (status = 200, description = "Scheduled job triggered successfully, returns new session ID", body = RunNowResponse),
        (status = 404, description = "Scheduled job not found"),
        (status = 500, description = "Internal server error when trying to run the job")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn run_now_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<RunNowResponse>, (StatusCode, String)> {
    let scheduler = state.scheduler();

    let (workflow_display_name, workflow_version_opt) = if let Some(job) = scheduler
        .list_scheduled_jobs()
        .await
        .into_iter()
        .find(|job| job.id == id)
    {
        let workflow_display_name = std::path::Path::new(&job.source)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| id.clone());

        let workflow_version_opt =
            tokio::fs::read_to_string(&job.source)
                .await
                .ok()
                .and_then(|content: String| {
                    biorouter::workflow::template_workflow::parse_workflow_content(
                        &content,
                        Some(
                            std::path::Path::new(&job.source)
                                .parent()
                                .unwrap_or_else(|| std::path::Path::new(""))
                                .to_string_lossy()
                                .to_string(),
                        ),
                    )
                    .ok()
                    .map(|(r, _)| r.version)
                });

        (workflow_display_name, workflow_version_opt)
    } else {
        (id.clone(), None)
    };

    let workflow_version_tag = workflow_version_opt.as_deref().unwrap_or("");
    tracing::info!(
        counter.biorouter.workflow_runs = 1,
        workflow_name = %workflow_display_name,
        workflow_version = %workflow_version_tag,
        session_type = "schedule",
        interface = "server",
        "Workflow execution started"
    );

    tracing::info!("Server: Calling scheduler.run_now() for job '{}'", id);

    match scheduler.run_now(&id).await {
        Ok(session_id) => Ok(Json(RunNowResponse { session_id })),
        Err(e) => {
            eprintln!("Error running schedule '{}' now: {:?}", id, e);
            match classify_run_now_error(&id, &e) {
                // The sentinel `ScheduleDetailView.tsx` branches on.
                RunNowOutcome::Cancelled => Ok(Json(RunNowResponse {
                    session_id: "CANCELLED".to_string(),
                })),
                RunNowOutcome::Failed(status, message) => Err((status, message)),
            }
        }
    }
}

/// What a failed `scheduler.run_now` means to the client.
#[derive(Debug, PartialEq, Eq)]
enum RunNowOutcome {
    /// Not a failure at all: the user stopped the run.
    Cancelled,
    Failed(StatusCode, String),
}

/// Classify a failed `run_now` into the status and the **message** the caller
/// sees.
///
/// The message half is the fix. This handler used to answer every non-404 with a
/// bare `StatusCode`, so the response had no body and the client had nothing but
/// the number — which throws away the two errors here that were written to be
/// read by a person: the privacy barrier's refusal, which says exactly what the
/// user must change ("this chat is private and the model it is bound to is
/// public; switch it to a private model"), and the scheduler's own failure text,
/// the same string the schedules view shows as `last_error`. Both arrived at the
/// run-now toast as an anonymous 500.
///
/// ⚠ A refusal's **status** stays 500 rather than becoming a 403, and that is a
/// deliberate limit rather than an oversight: `#[utoipa::path]` above declares
/// 200/404/500, `ui/desktop/openapi.json` is generated from it, and CI fails on
/// drift between the two (`scripts/check-openapi-schema.sh`). A new status would
/// need that spec and the generated TypeScript client regenerated alongside.
/// Adding a *body* needs neither, because utoipa reads the attribute and not the
/// handler's return type.
fn classify_run_now_error(id: &str, error: &biorouter::scheduler::SchedulerError) -> RunNowOutcome {
    use biorouter::scheduler::SchedulerError;
    match error {
        SchedulerError::JobNotFound(_) => RunNowOutcome::Failed(
            StatusCode::NOT_FOUND,
            format!("Schedule '{id}' was not found."),
        ),
        // A stopped run is not a failed one. Issue #148B: until the scheduler
        // learned to tell cancellation from success this branch was unreachable
        // — nothing produced the marker, because a killed run reported `Ok` and
        // advanced the schedule's `last_run` cursor. The literal that used to be
        // typed here now comes from the scheduler that emits it, so a reword
        // cannot silently strand this arm again.
        SchedulerError::AnyhowError(err) if err.to_string().contains(RUN_CANCELLED_MARKER) => {
            RunNowOutcome::Cancelled
        }
        other => RunNowOutcome::Failed(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

#[utoipa::path(
    get,
    path = "/schedule/{id}/sessions",
    params(
        ("id" = String, Path, description = "ID of the schedule"),
        SessionsQuery // This will automatically pick up 'limit' as a query parameter
    ),
    responses(
        (status = 200, description = "A list of session display info", body = Vec<SessionDisplayInfo>),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn sessions_handler(
    State(state): State<Arc<AppState>>,
    Path(schedule_id_param): Path<String>, // Renamed to avoid confusion with session_id
    Query(query_params): Query<SessionsQuery>,
) -> Result<Json<Vec<SessionDisplayInfo>>, StatusCode> {
    let scheduler = state.scheduler();

    match scheduler
        .sessions(&schedule_id_param, query_params.limit)
        .await
    {
        Ok(session_tuples) => {
            let mut display_infos = Vec::new();
            for (session_name, session) in session_tuples {
                display_infos.push(SessionDisplayInfo {
                    id: session_name.clone(),
                    name: session.name,
                    created_at: session.created_at.to_rfc3339(),
                    working_dir: session.working_dir.to_string_lossy().into_owned(),
                    schedule_id: session.schedule_id,
                    message_count: session.message_count,
                    total_tokens: session.total_tokens,
                    input_tokens: session.input_tokens,
                    output_tokens: session.output_tokens,
                    accumulated_total_tokens: session.accumulated_total_tokens,
                    accumulated_input_tokens: session.accumulated_input_tokens,
                    accumulated_output_tokens: session.accumulated_output_tokens,
                });
            }
            Ok(Json(display_infos))
        }
        Err(e) => {
            eprintln!(
                "Error fetching sessions for schedule '{}': {:?}",
                schedule_id_param, e
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[utoipa::path(
    post,
    path = "/schedule/{id}/pause",
    params(
        ("id" = String, Path, description = "ID of the schedule to pause")
    ),
    responses(
        (status = 204, description = "Scheduled job paused successfully"),
        (status = 404, description = "Scheduled job not found"),
        (status = 400, description = "Cannot pause a currently running job"),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn pause_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let scheduler = state.scheduler();

    scheduler.pause_schedule(&id).await.map_err(|e| {
        eprintln!("Error pausing schedule '{}': {:?}", id, e);
        match e {
            biorouter::scheduler::SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
            biorouter::scheduler::SchedulerError::AnyhowError(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/schedule/{id}/unpause",
    params(
        ("id" = String, Path, description = "ID of the schedule to unpause")
    ),
    responses(
        (status = 204, description = "Scheduled job unpaused successfully"),
        (status = 404, description = "Scheduled job not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn unpause_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let scheduler = state.scheduler();

    scheduler.unpause_schedule(&id).await.map_err(|e| {
        eprintln!("Error unpausing schedule '{}': {:?}", id, e);
        match e {
            biorouter::scheduler::SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/schedule/{id}",
    params(
        ("id" = String, Path, description = "ID of the schedule to update")
    ),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Scheduled job updated successfully", body = ScheduledJob),
        (status = 404, description = "Scheduled job not found"),
        (status = 400, description = "Cannot update a currently running job or invalid request"),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Json<ScheduledJob>, StatusCode> {
    let scheduler = state.scheduler();

    scheduler
        .update_schedule(&id, req.cron)
        .await
        .map_err(|e| {
            eprintln!("Error updating schedule '{}': {:?}", id, e);
            match e {
                biorouter::scheduler::SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
                biorouter::scheduler::SchedulerError::AnyhowError(_) => StatusCode::BAD_REQUEST,
                biorouter::scheduler::SchedulerError::CronParseError(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;

    let jobs = scheduler.list_scheduled_jobs().await;
    let updated_job = jobs
        .into_iter()
        .find(|job| job.id == id)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(updated_job))
}

#[utoipa::path(
    post,
    path = "/schedule/{id}/kill",
    responses(
        (status = 200, description = "Running job killed successfully"),
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
pub async fn kill_running_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<KillJobResponse>, (StatusCode, String)> {
    let scheduler = state.scheduler();

    // ⚠ The success message below is only true because `kill_running_job` now
    // FAILS when there was nothing to cancel. It used to return `Ok(())` whenever
    // the cancel-token registry held no token for the schedule, so this route
    // reported "Successfully killed running job" for a Stop that stopped
    // nothing — the #148 cancel complaint.
    scheduler.kill_running_job(&id).await.map_err(|e| {
        eprintln!("Error killing running job '{}': {:?}", id, e);
        classify_kill_error(&e)
    })?;

    Ok(Json(KillJobResponse {
        message: format!("Successfully killed running job '{}'", id),
    }))
}

/// Turn a refused Stop into the status and the **message** the caller sees.
///
/// The message half matters more here than anywhere else on this route file.
/// `Scheduler::kill_running_job` now fails when there was no run to cancel —
/// before that it returned `Ok(())` and this route answered "Successfully killed
/// running job" for a Stop that stopped nothing, which is the #148 cancel
/// complaint. A bare `StatusCode` would replace that lie with a silent 400,
/// which is barely better: the whole content of the answer is the sentence
/// *"it is marked running but the run has already finished, or it was started by
/// another Biorouter process"*, and the user needs to read it.
fn classify_kill_error(error: &biorouter::scheduler::SchedulerError) -> (StatusCode, String) {
    use biorouter::scheduler::SchedulerError;
    let status = match error {
        SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
        SchedulerError::AnyhowError(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, error.to_string())
}

#[utoipa::path(
    get,
    path = "/schedule/{id}/inspect",
    params(
        ("id" = String, Path, description = "ID of the schedule to inspect")
    ),
    responses(
        (status = 200, description = "Running job information", body = InspectJobResponse),
        (status = 404, description = "Scheduled job not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "schedule"
)]
#[axum::debug_handler]
pub async fn inspect_running_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<InspectJobResponse>, StatusCode> {
    let scheduler = state.scheduler();

    match scheduler.get_running_job_info(&id).await {
        Ok(info) => {
            if let Some((session_id, start_time)) = info {
                let duration = chrono::Utc::now().signed_duration_since(start_time);
                Ok(Json(InspectJobResponse {
                    session_id: Some(session_id),
                    process_start_time: Some(start_time.to_rfc3339()),
                    running_duration_seconds: Some(duration.num_seconds()),
                }))
            } else {
                Ok(Json(InspectJobResponse {
                    session_id: None,
                    process_start_time: None,
                    running_duration_seconds: None,
                }))
            }
        }
        Err(e) => {
            eprintln!("Error inspecting running job '{}': {:?}", id, e);
            match e {
                biorouter::scheduler::SchedulerError::JobNotFound(_) => Err(StatusCode::NOT_FOUND),
                _ => Err(StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/schedule/create", post(create_schedule))
        .route("/schedule/list", get(list_schedules))
        .route("/schedule/delete/{id}", delete(delete_schedule)) // Corrected
        .route("/schedule/{id}", put(update_schedule))
        .route("/schedule/{id}/run_now", post(run_now_handler)) // Corrected
        .route("/schedule/{id}/pause", post(pause_schedule))
        .route("/schedule/{id}/unpause", post(unpause_schedule))
        .route("/schedule/{id}/kill", post(kill_running_job))
        .route("/schedule/{id}/inspect", get(inspect_running_job))
        .route("/schedule/{id}/sessions", get(sessions_handler)) // Corrected
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use biorouter::scheduler::SchedulerError;

    /// Issue #148B, the terminal half of it. A run the user stopped is not a
    /// failure: `ScheduleDetailView.tsx` branches on the `CANCELLED` session id,
    /// and it only ever sees it because this classification exists.
    ///
    /// Fails an implementation that treats every `AnyhowError` as a 500.
    #[test]
    fn a_stopped_run_is_reported_as_cancelled_not_as_a_failure() {
        let stopped = SchedulerError::AnyhowError(anyhow!(
            "the run was stopped, so it {RUN_CANCELLED_MARKER} rather than finishing; \
             the schedule's last-run cursor was not advanced"
        ));
        assert_eq!(
            classify_run_now_error("nightly", &stopped),
            RunNowOutcome::Cancelled
        );
    }

    /// The privacy barrier's refusal is written to be read by the person at the
    /// keyboard — it names what they have to change. It used to reach the
    /// run-now toast as an anonymous 500 with an empty body, so none of that
    /// sentence survived the route.
    ///
    /// Fails the `Err(StatusCode::INTERNAL_SERVER_ERROR)` implementation this
    /// replaced: there is no body for the assertion to look at.
    #[test]
    fn a_privacy_refusal_reaches_the_client_as_words_not_only_a_number() {
        let refused = SchedulerError::AnyhowError(anyhow!(
            "the privacy barrier refused this run's turn, so no work was done and the \
             schedule's last-run cursor was not advanced. This chat is private and the \
             model it is bound to is public; switch it to a private model."
        ));
        let RunNowOutcome::Failed(status, message) = classify_run_now_error("nightly", &refused)
        else {
            panic!("a refusal is a failure, not a cancellation");
        };
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            message.contains("switch it to a private model"),
            "the repair instruction must survive the route: {message}"
        );
        assert!(
            message.contains("privacy barrier refused"),
            "and so must the reason: {message}"
        );
    }

    /// A missing schedule is still a 404, and now says which one.
    #[test]
    fn a_missing_schedule_is_a_404_that_names_it() {
        let missing = SchedulerError::JobNotFound("nightly".to_string());
        let RunNowOutcome::Failed(status, message) = classify_run_now_error("nightly", &missing)
        else {
            panic!("a missing schedule is a failure");
        };
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(message.contains("nightly"), "{message}");
    }

    /// Every other scheduler failure keeps its own sentence too — this is the
    /// string the schedules view shows as `last_error`, and the run-now toast
    /// should not be the one surface that reduces it to "500".
    #[test]
    fn an_ordinary_failure_keeps_its_message() {
        let broken = SchedulerError::CronParseError("expected 5 or 6 fields, got 2".to_string());
        let RunNowOutcome::Failed(status, message) = classify_run_now_error("nightly", &broken)
        else {
            panic!("a broken cron is a failure");
        };
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(message.contains("expected 5 or 6 fields"), "{message}");
    }

    /// A refused Stop has to arrive as words. `Scheduler::kill_running_job` now
    /// fails when there was no run to cancel — it used to return `Ok(())`, and
    /// this route answered "Successfully killed running job" for a Stop that
    /// stopped nothing (#148). Replacing that lie with a bare 400 is barely
    /// better; the sentence is the answer.
    ///
    /// Fails the `Err(StatusCode::BAD_REQUEST)` implementation this replaced:
    /// there is no body for the assertion to look at.
    ///
    /// ⚠ This deliberately does NOT reach across into `biorouter`'s source with
    /// `include_str!`. An earlier version of this test did, and it was measured
    /// useless: mutating `scheduler.rs` recompiled `biorouter` but cargo reused
    /// the already-built `biorouter_server` test binary, so the pin read the
    /// *old* text and passed. A cross-crate source pin is a test that can go
    /// stale without anything saying so.
    #[test]
    fn a_refused_stop_reaches_the_client_as_words_not_only_a_number() {
        let nothing_to_cancel = SchedulerError::AnyhowError(anyhow!(
            "Schedule 'nightly' has no run this process can stop: it is marked running but the \
             run has already finished, or it was started by another Biorouter process. Nothing \
             was cancelled."
        ));
        let (status, message) = classify_kill_error(&nothing_to_cancel);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            message.contains("Nothing was cancelled"),
            "the user must be able to tell a Stop that stopped nothing from one that worked: \
             {message}"
        );

        let missing = SchedulerError::JobNotFound("nightly".to_string());
        let (status, message) = classify_kill_error(&missing);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(message.contains("nightly"), "{message}");
    }

    /// The reported defect: **New schedule** with the name
    /// `<img src=x onerror=alert(1)>` answered *"Failed to create schedule:
    /// Unexpected response format"* — a transport-shaped report for a
    /// validation problem the user could fix in one keystroke.
    ///
    /// The cause was a bare `StatusCode::BAD_REQUEST`: axum sends that with **no
    /// body**, so the client's JSON parse was the thing that actually failed and
    /// the real reason only ever reached a `tracing::warn!`.
    ///
    /// Asserted through `IntoResponse`, because the shape of the body is the
    /// finding — a test that only read `ErrorResponse.message` would pass on an
    /// implementation that never serialises it.
    #[tokio::test]
    async fn an_invalid_schedule_name_is_refused_with_the_reason_in_the_body() {
        use axum::body::to_bytes;
        use axum::response::IntoResponse;

        let hostile = "<img src=x onerror=alert(1)>";
        let error = biorouter::scheduler::validate_schedule_id(hostile)
            .expect_err("a name that names a file must still be refused");
        let response = create_schedule_error(create_schedule_status(&error), error).into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body)
            .expect("the client parses this as JSON; an empty body is the bug");
        let message = parsed["message"].as_str().expect("no message field");
        assert!(
            message.contains("may only contain letters, digits"),
            "the refusal must state the rule: {message}"
        );
        assert!(
            message.contains(hostile),
            "and name the value it refused: {message}"
        );
    }

    /// The id validation is a security boundary — it closes an arbitrary-file
    /// write — so reporting it better must not move it. The refusal still
    /// happens in the handler, *before* anything is handed to the scheduler.
    ///
    /// Structural, because the regression is a deletion: rewriting the error
    /// arm is exactly the edit that would drop the guard, and the route would
    /// then still answer a readable 400 from inside `add_scheduled_job` — after
    /// the request had already reached it.
    #[test]
    fn the_name_is_still_refused_in_the_handler_before_the_scheduler_sees_it() {
        let src = include_str!("schedule.rs");
        let (_, after) = src
            .split_once("async fn create_schedule(")
            .expect("create_schedule is gone");
        let (body, _) = after
            .split_once("\n/// The status a failed create")
            .expect("could not find the end of create_schedule");
        let guard = body
            .find("validate_schedule_id(&req.id)")
            .expect("the handler no longer refuses an invalid name itself");
        let handoff = body
            .find(".add_scheduled_job(")
            .expect("the handler no longer schedules anything");
        assert!(
            guard < handoff,
            "the name must be refused before the scheduler is handed the request"
        );

        // And the rule itself is unchanged: what was refused is still refused.
        for name in [
            "<img src=x onerror=alert(1)>",
            "{{ 7*7 }}",
            "/tmp/pwned",
            "",
        ] {
            assert!(
                biorouter::scheduler::validate_schedule_id(name).is_err(),
                "{name:?} must still be refused"
            );
        }
        assert!(biorouter::scheduler::validate_schedule_id("daily-summary_job").is_ok());
    }

    /// A create that fails for a reason other than the name keeps its own
    /// status, and now carries its own sentence rather than an anonymous number.
    #[test]
    fn each_create_failure_keeps_its_status_and_gains_its_words() {
        for (error, expected) in [
            (
                SchedulerError::JobIdExists("nightly".to_string()),
                StatusCode::CONFLICT,
            ),
            (
                SchedulerError::CronParseError("not a cron".to_string()),
                StatusCode::BAD_REQUEST,
            ),
            (
                SchedulerError::WorkflowLoadError("no such file".to_string()),
                StatusCode::BAD_REQUEST,
            ),
            (
                SchedulerError::InvalidJobId("not a slug".to_string()),
                StatusCode::BAD_REQUEST,
            ),
            (
                SchedulerError::JobNotFound("nightly".to_string()),
                StatusCode::NOT_FOUND,
            ),
            (
                SchedulerError::PersistError("disk full".to_string()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            let text = error.to_string();
            let status = create_schedule_status(&error);
            assert_eq!(status, expected, "wrong status for {text}");
            assert_eq!(
                create_schedule_error(status, error).message,
                text,
                "the scheduler's own sentence is what the dialog shows"
            );
        }
    }
}
