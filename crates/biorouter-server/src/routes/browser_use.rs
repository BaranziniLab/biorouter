//! The Biorouter Browser's consent routes (BB-17), beside `/agent/computer_use/*`.
//!
//! * `GET /agent/browser/status` is the approval card's poll.
//! * `POST /agent/browser/consent` answers the card. It requires the user-action proof
//!   (`X-User-Action`), so neither a model nor the plain API secret can grant.
//! * `POST /agent/browser/revoke` is Stop and the card's Cancel. Stop is ONE control for both
//!   agent capabilities (BB-18): it revokes the browser's grants and Biorouter Copilot's, as
//!   Copilot's own revoke route now does too.
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use biorouter::security::browser_use::{ActLifetime, BrowserGrant, BrowserUseStatus};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::{routes::errors::ErrorResponse, state::AppState};

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct BrowserUseSessionRequest {
    pub session_id: String,
}

#[derive(Deserialize, ToSchema)]
pub struct BrowserUseConsentRequest {
    pub session_id: String,
    pub challenge_id: String,
    /// The grant the card asked for.
    pub grant: BrowserGrant,
    /// For an act grant: this request (default) or the whole chat.
    #[serde(default)]
    pub lifetime: Option<ActLifetime>,
}

fn conflict(error: anyhow::Error) -> ErrorResponse {
    ErrorResponse {
        status: StatusCode::CONFLICT,
        message: error.to_string(),
    }
}

/// Only the refusal that can never change (Chat mode) is a 409, as for Copilot: the card treats
/// a 409 from its poll as "this chat cannot browse" and stops polling.
fn status_refusal(error: anyhow::Error) -> ErrorResponse {
    if error
        .downcast_ref::<biorouter::security::browser_use::ModeForbidsBrowser>()
        .is_some()
    {
        return conflict(error);
    }
    ErrorResponse {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: error.to_string(),
    }
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
            message: "Load this chat and bind a model before using the Biorouter Browser".into(),
        })
}

#[utoipa::path(get, path = "/agent/browser/status", operation_id = "browser_use_status", params(BrowserUseSessionRequest), responses((status = 200, body = BrowserUseStatus), (status = 403, description = "Session out of reach"), (status = 409, description = "Chat mode does not run the browser"), (status = 424, description = "Chat not loaded")))]
pub async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(request): Query<BrowserUseSessionRequest>,
) -> Result<Json<BrowserUseStatus>, ErrorResponse> {
    let agent = agent(&state, &request.session_id, &headers).await?;
    agent
        .extension_manager
        .browser_use_status(&request.session_id)
        .await
        .map(Json)
        .map_err(status_refusal)
}

fn require_human(proof: UserActionProof) -> Result<(), ErrorResponse> {
    match proof {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(ErrorResponse {
            status: StatusCode::FORBIDDEN,
            message: "Only the person using this chat may allow the Biorouter Browser. A model or an ordinary API credential cannot allow it.".into(),
        }),
        UserActionProof::NoKeyInstalled => Err(ErrorResponse {
            status: StatusCode::FORBIDDEN,
            message: "The Biorouter Browser runs in the desktop app, which can ask you; this backend cannot.".into(),
        }),
    }
}

#[utoipa::path(post, path = "/agent/browser/consent", operation_id = "browser_use_consent", request_body = BrowserUseConsentRequest, responses((status = 200, body = BrowserUseStatus), (status = 403, description = "Human proof required"), (status = 409, description = "The request changed")))]
pub async fn consent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<BrowserUseConsentRequest>,
) -> Result<Json<BrowserUseStatus>, ErrorResponse> {
    require_human(user_action_proof(&headers))?;
    let agent = agent(&state, &request.session_id, &headers).await?;
    agent
        .extension_manager
        .approve_browser_use(
            &request.session_id,
            &request.challenge_id,
            request.grant,
            request.lifetime.unwrap_or(ActLifetime::Request),
        )
        .await
        .map(Json)
        .map_err(conflict)
}

#[utoipa::path(post, path = "/agent/browser/revoke", operation_id = "browser_use_revoke", request_body = BrowserUseSessionRequest, responses((status = 200, body = BrowserUseStatus), (status = 403, description = "Session out of reach")))]
pub async fn revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<BrowserUseSessionRequest>,
) -> Result<Json<BrowserUseStatus>, ErrorResponse> {
    let agent = agent(&state, &request.session_id, &headers).await?;
    stop_agent_capabilities(&agent);
    agent
        .extension_manager
        .browser_use_status(&request.session_id)
        .await
        .map(Json)
        .map_err(conflict)
}

/// The one Stop (BB-18): both agent capabilities' grants end, and any browser operation in
/// flight is cancelled through the grant's token.
pub fn stop_agent_capabilities(agent: &biorouter::agents::Agent) {
    agent.extension_manager.browser_use.revoke();
    agent.extension_manager.computer_use.revoke();
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/agent/browser/status", get(status))
        .route("/agent/browser/consent", post(consent))
        .route("/agent/browser/revoke", post(revoke))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_authentication_never_grants_the_browser() {
        assert!(require_human(UserActionProof::Unproven).is_err());
        assert!(require_human(UserActionProof::NoKeyInstalled).is_err());
        assert!(require_human(UserActionProof::Proven).is_ok());
    }

    #[test]
    fn only_the_mode_refusal_is_permanent() {
        use biorouter::security::browser_use::ModeForbidsBrowser;
        assert_eq!(
            status_refusal(ModeForbidsBrowser.into()).status,
            StatusCode::CONFLICT
        );
        let transient = status_refusal(anyhow::anyhow!(
            "Bind a model before using the Biorouter Browser"
        ));
        assert_eq!(transient.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn a_consent_request_names_its_grant() {
        let request: BrowserUseConsentRequest = serde_json::from_value(serde_json::json!({
            "session_id": "s", "challenge_id": "c", "grant": "act", "lifetime": "chat"
        }))
        .unwrap();
        assert_eq!(request.grant, BrowserGrant::Act);
        assert_eq!(request.lifetime, Some(ActLifetime::Chat));
        assert!(
            serde_json::from_value::<BrowserUseConsentRequest>(serde_json::json!({
                "session_id": "s", "challenge_id": "c", "grant": "everything"
            }))
            .is_err()
        );
    }
}
