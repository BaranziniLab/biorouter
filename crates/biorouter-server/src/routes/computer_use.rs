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

/// Separate "this chat will never run Biorouter Copilot" from "not yet".
///
/// ⚠ Only [`ModeForbidsComputerUse`] may be a 409 here. The interface treats a
/// 409 from the status poll as a permanent verdict -- it hides the panel and
/// stops polling for the life of the chat, taking the **Stop** button with it --
/// so a chat that merely had no model bound yet, or whose runtime was briefly
/// held by another chat, was written off forever and could not be recovered by
/// binding a model, because the component never remounts. Everything else is
/// retryable and says so, which is what keeps the poll alive.
fn status_refusal(error: anyhow::Error) -> ErrorResponse {
    if error
        .downcast_ref::<biorouter::security::computer_use::ModeForbidsComputerUse>()
        .is_some()
    {
        return conflict(error);
    }
    ErrorResponse {
        status: StatusCode::SERVICE_UNAVAILABLE,
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
            message: "Load this chat and bind a model before setting up Biorouter Copilot".into(),
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
        .map_err(status_refusal)?;
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
            message: "Too many Biorouter Copilot approval attempts. Wait one minute and try again."
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
        UserActionProof::Unproven => Err(ErrorResponse { status: StatusCode::FORBIDDEN, message: "Only the person using this chat may approve Biorouter Copilot. A model or ordinary API credential cannot approve it.".into() }),
        UserActionProof::NoKeyInstalled => Err(ErrorResponse { status: StatusCode::FORBIDDEN, message: "This backend has no Biorouter Copilot approval key. Use the desktop app or restart serve with interactive Biorouter Copilot approval setup.".into() }),
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

    /// ⚠ The interface acts on the STATUS CODE, not on the sentence: a 409 from
    /// the status poll hides the Biorouter Copilot panel and stops polling for the
    /// rest of the chat, which removes the **Stop** button. So only the refusal
    /// that can never change may be a 409.
    ///
    /// `status` also fails while a chat simply has no model bound yet -- the
    /// state a chat is in whenever its restore did not bind one, which the
    /// renderer treats as ready -- and while another chat holds the runtime.
    /// Both of those become true a moment later, and while they shared 409 with
    /// the mode refusal, binding a model could not bring the panel back: the
    /// component is keyed on the chat id, so it never remounts.
    #[test]
    fn only_the_mode_refusal_is_permanent() {
        use biorouter::security::computer_use::ModeForbidsComputerUse;

        assert_eq!(
            status_refusal(ModeForbidsComputerUse.into()).status,
            StatusCode::CONFLICT
        );
        for transient in [
            "Bind a model before starting Biorouter Copilot",
            "Biorouter Copilot runtime belongs to a different chat",
            "Biorouter Copilot is busy in another BioRouter process. Stop its task before switching control.",
        ] {
            let answer = status_refusal(anyhow::anyhow!(transient));
            assert_eq!(
                answer.status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{transient:?} is not permanent and must not read as one"
            );
            assert_eq!(answer.message, transient, "the reason must survive");
        }
    }

    /// A sentence that merely READS like the mode refusal is not one. This is
    /// what keeps the classification on the type rather than drifting back into
    /// a string comparison two crates apart.
    #[test]
    fn the_mode_refusal_is_a_type_not_a_sentence() {
        let impostor = anyhow::anyhow!("Chat mode does not run Biorouter Copilot tools");
        assert_eq!(
            status_refusal(impostor).status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
