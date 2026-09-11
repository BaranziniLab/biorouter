//! "What is the agent running now" — a unified read of active long-running work
//! plus a per-item cancel affordance (BR-42).
//!
//! Background shell jobs and running subagents register themselves in the
//! process-wide `biorouter_mcp::active_work` registry as they start; the
//! scheduler tracks its own in-flight runs. This route aggregates all three into
//! one list so the user (via a GUI panel, deferred) can see and stop
//! runaway/forgotten work. `GET /active_work` lists; `POST
//! /active_work/{id}/cancel` cancels one item, dispatched by its id.
//!
//! # Whose work a caller sees (issue #56)
//!
//! Every row carries the id of the chat it belongs to and a `title`/`detail`
//! holding that chat's SHELL COMMAND or TASK PROMPT — content, not metadata. So
//! both routes ask `routes::session_reach`'s one decision about that chat:
//!
//! * the list shows a row exactly when `GET /sessions` would show its chat
//!   ([`HttpCaller::lists_work`](crate::routes::session_reach::HttpCaller::lists_work)),
//!   omitted and never redacted;
//! * the cancel resolves its id to the owning chat and asks the chat READ's own
//!   gate ([`work_reach`](crate::routes::session_reach::work_reach)) before it
//!   stops anything, and refuses with the read's exact words.
//!
//! ⚠ **Work that names no chat is answered as a private chat's**, on both
//! routes, and so is work whose chat cannot be read, a handle that names
//! nothing and a schedule that is not running: the registry cannot say whose
//! command an unattributed row holds. The shell attributes its rows from the
//! chat id Biorouter's MCP client stamps on every call, so this arm is left to
//! work that genuinely has no chat.
//!
//! ⚠ **The scheduled half is not closed by this file.** `GET /schedule/list`
//! and `GET /schedule/{id}/inspect` still name a running schedule's chat, and
//! `POST /schedule/{id}/kill` still stops it, for any holder of the daemon
//! secret; see the residual table in
//! `docs/deployment/programmatic-session-access.md`.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
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

/// The rows this caller may be shown: those whose chat it could open. See the
/// module header.
///
/// One resolved caller for the whole list, so the rows cannot half-believe two
/// answers; each row's chat is looked up only when the caller is not already
/// shown every row.
async fn visible_items(
    caller: &crate::routes::session_reach::HttpCaller,
    manager: &biorouter::session::session_manager::SessionManager,
    items: Vec<ActiveWorkItemDto>,
) -> Vec<ActiveWorkItemDto> {
    let mut visible = Vec::with_capacity(items.len());
    for item in items {
        if caller.lists_work(manager, item.session_id.as_deref()).await {
            visible.push(item);
        }
    }
    visible
}

#[utoipa::path(
    get,
    path = "/active_work",
    responses(
        (status = 200, description = "Current background jobs, subagents, and in-flight scheduled \
                                      runs, holding only the work of the chats this caller could \
                                      open: a row whose chat is private, cannot be read, or that \
                                      names no chat at all is omitted — never redacted — for a \
                                      caller with neither the user-action proof nor a private \
                                      capability, as its chat is from `GET /sessions`", body = ActiveWorkResponse),
    ),
    tag = "active_work"
)]
#[axum::debug_handler]
async fn list_active_work(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<ActiveWorkResponse> {
    // Issue #56: every row is some chat's command or prompt, and this handed
    // all of them to a caller holding nothing but the daemon secret.
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    let registry = active_work().list();
    let jobs = state.scheduler().list_scheduled_jobs().await;
    let items = build_items(registry, jobs, Utc::now());
    let items = visible_items(&caller, state.session_manager(), items).await;
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
        (status = 403, description = "The work belongs to a chat this caller could not open — a \
                                      private chat, one that cannot be read, or none at all — and \
                                      the request carried neither the user-action proof nor a \
                                      private capability. Plain text, byte-for-byte what `GET \
                                      /sessions/{session_id}` answers, and the same for an id \
                                      that names nothing, so a refusal says nothing about the \
                                      work. Nothing was stopped"),
        (status = 404, description = "No such active-work item"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "active_work"
)]
#[axum::debug_handler]
async fn cancel_active_work(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<CancelActiveWorkResponse>, Response> {
    let target = classify_cancel_id(&id);

    // Issue #56: the id names WORK, not a chat. Resolve it to the chat that
    // owns the work and ask the chat read's own gate BEFORE anything is
    // stopped — this route stopped any chat's work for a caller holding only
    // the daemon secret. Both lookups are reads; a handle that names nothing,
    // and a schedule with no run in a chat, resolve to no chat at all.
    let owner = match &target {
        CancelTarget::Scheduler(sched_id) => state
            .scheduler()
            .get_running_job_info(sched_id)
            .await
            .ok()
            .flatten()
            .map(|(session_id, _)| session_id),
        CancelTarget::Registry(reg_id) => {
            active_work().get(reg_id).and_then(|item| item.session_id)
        }
    };
    crate::routes::session_reach::work_reach(state.session_manager(), owner.as_deref(), &headers)
        .await
        .map_err(IntoResponse::into_response)?;

    match target {
        CancelTarget::Scheduler(sched_id) => {
            state
                .scheduler()
                .kill_running_job(&sched_id)
                .await
                .map_err(|e| match e {
                    biorouter::scheduler::SchedulerError::JobNotFound(_) => StatusCode::NOT_FOUND,
                    biorouter::scheduler::SchedulerError::AnyhowError(_) => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                })
                .map_err(IntoResponse::into_response)?;
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
                Err(StatusCode::NOT_FOUND.into_response())
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

    // ─── Issue #56: whose work a caller sees ───

    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use crate::routes::session_reach::{http_caller, CALLER_PROVIDER_HEADER};
    use biorouter::privacy::SessionClassification;
    use biorouter::session::session_manager::SessionManager;

    /// A session store of this test's own, holding one public and one private
    /// chat, so no `AppState` has to be built and no other test's rows are in
    /// it. The private one gets there the way a real one does, by binding a
    /// private provider.
    async fn store_with_a_public_and_a_private_chat(
    ) -> (tempfile::TempDir, SessionManager, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let mut ids = Vec::new();
        for label in ["public", "private"] {
            let session = manager
                .create_session(
                    std::path::PathBuf::from("/tmp/active_work_reach"),
                    format!("Active work {label} (test fixture)"),
                    biorouter::session::SessionType::User,
                )
                .await
                .unwrap();
            ids.push(session.id);
        }
        manager
            .update(&ids[1])
            .provider_name("versa_azure")
            .model_config(biorouter::model::ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        let private = ids.pop().unwrap();
        let public = ids.pop().unwrap();
        (dir, manager, public, private)
    }

    fn owned_by(id: &str, kind: ActiveWorkKind, owner: Option<&str>) -> ActiveWorkItem {
        ActiveWorkItem {
            session_id: owner.map(str::to_string),
            ..reg_item(id, kind, true)
        }
    }

    fn running_in(id: &str, owner: Option<&str>) -> ScheduledJob {
        ScheduledJob {
            current_session_id: owner.map(str::to_string),
            ..sched_job(id, true)
        }
    }

    fn ids(items: Vec<ActiveWorkItemDto>) -> Vec<String> {
        items.into_iter().map(|item| item.id).collect()
    }

    /// The list's own filter, row by row and kind by kind — including a
    /// scheduled run, whose `currently_running` only the scheduler can set, so
    /// the HTTP tests in `session_reach` cannot fabricate one.
    ///
    /// A secret-only caller keeps exactly the rows of the public chat. The
    /// private chat's rows go, and so do the rows that name no chat or a chat
    /// that is not there — a schedule between starting its run and naming its
    /// chat among them. The person at the keyboard and a program on a private
    /// model keep everything.
    #[tokio::test]
    async fn the_list_shows_each_row_exactly_when_its_chat_would_be_shown() {
        install_test_user_action_key();
        let (_dir, manager, public, private) = store_with_a_public_and_a_private_chat().await;
        let items = build_items(
            vec![
                owned_by("bg-1", ActiveWorkKind::BackgroundJob, Some(public.as_str())),
                owned_by("sub-2", ActiveWorkKind::Subagent, Some(private.as_str())),
                owned_by("fg-3", ActiveWorkKind::ForegroundCommand, None),
                owned_by(
                    "dturn-4",
                    ActiveWorkKind::DetachedTurn,
                    Some("29990101_99999"),
                ),
            ],
            vec![
                running_in("hourly", Some(public.as_str())),
                running_in("nightly", Some(private.as_str())),
                running_in("starting", None),
            ],
            Utc::now(),
        );
        let every_id = ids(items.clone());

        let secret_only = http_caller(&HeaderMap::new()).await;
        assert_eq!(
            ids(visible_items(&secret_only, &manager, items.clone()).await),
            ["bg-1", "sched:hourly"],
            "a caller holding only the daemon secret must be shown the public chat's work and \
             nothing else"
        );

        let mut proof = HeaderMap::new();
        proof.insert("X-User-Action", TEST_USER_ACTION_KEY.parse().unwrap());
        let mut private_model = HeaderMap::new();
        private_model.insert(CALLER_PROVIDER_HEADER, "versa_azure".parse().unwrap());
        for headers in [proof, private_model] {
            let caller = http_caller(&headers).await;
            assert_eq!(
                ids(visible_items(&caller, &manager, items.clone()).await),
                every_id,
                "{headers:?} lost a row it could open"
            );
        }
    }
}
