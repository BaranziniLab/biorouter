use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use biorouter::agents::baseten_decisions::BasetenDecisionModel;
use biorouter::agents::exit_criteria::{self, ExitCriteria};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

type RouteError = (StatusCode, String);

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopExitCriteriaSettings {
    pub criteria: Option<ExitCriteria>,
}

#[utoipa::path(
    get,
    path = "/agent/exit-criteria/{session_id}",
    params(("session_id" = String, Path, description = "Conversation ID")),
    responses(
        (status = 200, body = LoopExitCriteriaSettings),
        (status = 403, description = "Conversation is not reachable"),
        (status = 404, description = "Conversation not found"),
        (status = 500, description = "Stored criteria could not be read")
    ),
    security(("api_key" = [])), tag = "agent"
)]
pub async fn get_loop_exit_criteria(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<LoopExitCriteriaSettings>, RouteError> {
    crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
        .await
        .map_err(|r| (r.status, r.message.to_string()))?;
    state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let criteria = exit_criteria::load(state.session_manager(), &session_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(LoopExitCriteriaSettings { criteria }))
}

#[utoipa::path(
    post,
    path = "/agent/exit-criteria/{session_id}",
    params(("session_id" = String, Path, description = "Conversation ID")),
    request_body = LoopExitCriteriaSettings,
    responses(
        (status = 200, body = LoopExitCriteriaSettings),
        (status = 400, description = "Invalid criteria or unavailable decision model"),
        (status = 403, description = "Conversation or decision model is not permitted"),
        (status = 404, description = "Conversation not found"),
        (status = 500, description = "Criteria could not be saved")
    ),
    security(("api_key" = [])), tag = "agent"
)]
pub async fn set_loop_exit_criteria(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(settings): Json<LoopExitCriteriaSettings>,
) -> Result<Json<LoopExitCriteriaSettings>, RouteError> {
    crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
        .await
        .map_err(|r| (r.status, r.message.to_string()))?;
    state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if let Some(criteria) = &settings.criteria {
        criteria
            .validate()
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let provider = tokio::time::timeout(
            Duration::from_secs(30),
            biorouter::agents::baseten_decisions::create_provider(criteria),
        )
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Decision provider initialization timed out".into(),
            )
        })?
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        exit_criteria::assert_allowed(state.session_manager(), &session_id, provider.as_ref())
            .await
            .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?;
    }
    exit_criteria::save(
        state.session_manager(),
        &session_id,
        settings.criteria.as_ref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(settings))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/agent/exit-criteria/models/baseten",
            get(get_baseten_decision_models),
        )
        .route(
            "/agent/exit-criteria/{session_id}",
            get(get_loop_exit_criteria).post(set_loop_exit_criteria),
        )
        .with_state(state)
}

#[utoipa::path(
    get, path = "/agent/exit-criteria/models/baseten",
    responses((status = 200, body = Vec<BasetenDecisionModel>)),
    security(("api_key" = [])), tag = "agent"
)]
pub async fn get_baseten_decision_models() -> Json<Vec<BasetenDecisionModel>> {
    Json(biorouter::agents::baseten_decisions::models())
}
