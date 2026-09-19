use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use biorouter::security::computer_use::ComputerUseStatus;
use biorouter_server::auth::{computer_use_action_proof, UserActionProof};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::{routes::errors::ErrorResponse, state::AppState};

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct ComputerUseSessionRequest {
    pub session_id: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ComputerUseConsentRequest {
    pub session_id: String,
    pub challenge_id: String,
}

fn conflict(error: anyhow::Error) -> ErrorResponse {
    ErrorResponse {
        status: StatusCode::CONFLICT,
        message: error.to_string(),
    }
}

#[utoipa::path(get, path = "/agent/computer_use/setup", operation_id = "computer_use_setup", responses((status = 200, body = serde_json::Value)))]
pub async fn setup() -> Json<serde_json::Value> {
    Json(biorouter_mcp::computer_use::probe_readiness(true).await)
}

async fn agent(
    state: &AppState,
    session: &str,
    headers: &HeaderMap,
) -> Result<Arc<biorouter::agents::Agent>, ErrorResponse> {
    super::agent::authorize_agent_control(state, session, headers).await?;
    state
        .peek_agent(session)
        .await
        .ok_or_else(|| ErrorResponse {
            status: StatusCode::FAILED_DEPENDENCY,
            message: "Load this chat and bind a model before setting up Computer Use".into(),
        })
}

#[utoipa::path(get, path = "/agent/computer_use/status", operation_id = "computer_use_status", params(ComputerUseSessionRequest), responses((status = 200, body = ComputerUseStatus), (status = 403, description = "Session out of reach"), (status = 424, description = "Chat not loaded")))]
pub async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(request): Query<ComputerUseSessionRequest>,
) -> Result<Json<ComputerUseStatus>, ErrorResponse> {
    let agent = agent(&state, &request.session_id, &headers).await?;
    let mut status = agent
        .extension_manager
        .computer_use_status(&request.session_id)
        .await
        .map_err(conflict)?;
    status.runtime = biorouter_mcp::computer_use::probe_readiness(false).await;
    Ok(Json(status))
}

#[utoipa::path(post, path = "/agent/computer_use/consent", operation_id = "computer_use_consent", request_body = ComputerUseConsentRequest, responses((status = 200, body = ComputerUseStatus), (status = 403, description = "Human proof required"), (status = 409, description = "Scope changed or desktop busy")))]
pub async fn consent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ComputerUseConsentRequest>,
) -> Result<Json<ComputerUseStatus>, ErrorResponse> {
    let proof = computer_use_action_proof(&headers);
    if proof != UserActionProof::Proven && biorouter_server::auth::computer_use_action_locked_out()
    {
        return Err(ErrorResponse {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "Too many Computer Use approval attempts. Wait one minute and try again."
                .into(),
        });
    }
    require_human(proof)?;
    let agent = agent(&state, &request.session_id, &headers).await?;
    agent
        .extension_manager
        .approve_computer_use(&request.session_id, &request.challenge_id)
        .await
        .map(Json)
        .map_err(conflict)
}

fn require_human(proof: UserActionProof) -> Result<(), ErrorResponse> {
    match proof {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(ErrorResponse { status: StatusCode::FORBIDDEN, message: "Only the person using this chat may approve Computer Use. A model or ordinary API credential cannot approve it.".into() }),
        UserActionProof::NoKeyInstalled => Err(ErrorResponse { status: StatusCode::FORBIDDEN, message: "This backend has no Computer Use approval key. Use the desktop app or restart serve with interactive Computer Use approval setup.".into() }),
    }
}

#[utoipa::path(post, path = "/agent/computer_use/revoke", operation_id = "computer_use_revoke", request_body = ComputerUseSessionRequest, responses((status = 200, body = ComputerUseStatus), (status = 403, description = "Session out of reach")))]
pub async fn revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ComputerUseSessionRequest>,
) -> Result<Json<ComputerUseStatus>, ErrorResponse> {
    let agent = agent(&state, &request.session_id, &headers).await?;
    agent.extension_manager.computer_use.revoke();
    agent
        .extension_manager
        .computer_use_status(&request.session_id)
        .await
        .map(Json)
        .map_err(conflict)
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/agent/computer_use/status", get(status))
        .route("/agent/computer_use/setup", get(setup))
        .route("/agent/computer_use/consent", post(consent))
        .route("/agent/computer_use/revoke", post(revoke))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_authentication_never_grants_computer_use() {
        assert!(require_human(UserActionProof::Unproven).is_err());
        assert!(require_human(UserActionProof::NoKeyInstalled).is_err());
        assert!(require_human(UserActionProof::Proven).is_ok());
    }
}
