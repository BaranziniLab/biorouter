use crate::state::AppState;
use crate::tunnel::TunnelInfo;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
// `src/routes/` is compiled into the `biorouterd` binary as well as the lib and cannot name
// `crate::auth`, so this is the shared direction, the same import `routes::reset` uses.
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

/// The refusal for a caller that did not prove it is the person, on a daemon that holds a
/// user-action key.
///
/// Opening the tunnel puts the daemon on a public relay, and the daemon secret alone does not
/// make a caller the person: a public chat's shell recovers it with `ps eww` (AR-11). Measured
/// before this gate (GAP-SV-01): a secret-only `POST /tunnel/start` answered 200, opened the
/// relay and saved `tunnel_auto_start: true`.
pub const TUNNEL_NEEDS_USER: &str =
    "Only the person using Biorouter can open or close the remote tunnel, and this request \
     carried no proof that they sent it. Nothing was changed. Ask them to turn remote access on \
     or off in the Biorouter app.";

/// The refusal on a daemon that was handed no user-action key at all (`biorouter serve`,
/// SD-7), where every caller lands, the person included. A separate sentence, because
/// [`TUNNEL_NEEDS_USER`]'s advice to ask the person is a loop when the reader is that person.
pub const TUNNEL_NO_USER_KEY: &str =
    "This daemon was started without a user-action key, so it cannot tell a person from a \
     program, and the remote tunnel opens or closes only for a person. Nothing was changed. To \
     reach this daemon from another computer, use an SSH tunnel or a reverse proxy instead.";

/// Which refusal start and stop owe a caller, or `None` when the proof is good.
///
/// It reads [`user_action_proof`], not `is_user_action`: the boolean form collapses
/// `Unproven` and `NoKeyInstalled`, and that collapse is what once put an agent's sentence in
/// front of a person on `biorouter serve` (SD-8).
fn tunnel_refusal(proof: UserActionProof) -> Option<&'static str> {
    match proof {
        UserActionProof::Proven => None,
        UserActionProof::Unproven => Some(TUNNEL_NEEDS_USER),
        UserActionProof::NoKeyInstalled => Some(TUNNEL_NO_USER_KEY),
    }
}

/// A refusal in this route's own error envelope, logged at `warn`: a refused caller is the
/// gate working, not the tunnel failing.
fn refused(message: &'static str) -> Response {
    tracing::warn!("Tunnel change refused: the request carried no proof of the user");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
        .into_response()
}

/// What status shows a caller: the state to anyone holding the secret, and the relay URL,
/// hostname and tunnel secret only to the person. Those three are what let someone use the
/// tunnel, so they go no further than start would.
fn shown_to(mut info: TunnelInfo, proof: UserActionProof) -> TunnelInfo {
    if tunnel_refusal(proof).is_some() {
        info.url.clear();
        info.hostname.clear();
        info.secret.clear();
    }
    info
}

/// Start the tunnel
#[utoipa::path(
    post,
    path = "/tunnel/start",
    responses(
        (status = 200, description = "Tunnel starting. The state reads `starting` until the \
                                      relay has taken the connection, then `running`", body = TunnelInfo),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 403, description = "Refused: the request carried no proof it came from the \
                                      person, or this daemon holds no user-action key at all. \
                                      Nothing was changed", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
#[axum::debug_handler]
pub async fn start_tunnel(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = tunnel_refusal(user_action_proof(&headers)) {
        return refused(refusal);
    }
    match state.tunnel_manager.start().await {
        Ok(info) => (StatusCode::OK, Json(info)).into_response(),
        Err(e) => {
            tracing::error!("Failed to start tunnel: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

/// Stop the tunnel
#[utoipa::path(
    post,
    path = "/tunnel/stop",
    responses(
        (status = 200, description = "Tunnel stopped successfully"),
        (status = 403, description = "Refused: the request carried no proof it came from the \
                                      person, or this daemon holds no user-action key at all. \
                                      Nothing was changed", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
pub async fn stop_tunnel(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(refusal) = tunnel_refusal(user_action_proof(&headers)) {
        return refused(refusal);
    }
    state.tunnel_manager.stop(true).await;
    StatusCode::OK.into_response()
}

/// Get tunnel info
#[utoipa::path(
    get,
    path = "/tunnel/status",
    responses(
        (status = 200, description = "Tunnel info. Without the person's proof, `url`, \
                                      `hostname` and `secret` are empty", body = TunnelInfo)
    )
)]
pub async fn get_tunnel_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let info = shown_to(
        state.tunnel_manager.get_info().await,
        user_action_proof(&headers),
    );
    (StatusCode::OK, Json(info)).into_response()
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/tunnel/start", post(start_tunnel))
        .route("/tunnel/stop", post(stop_tunnel))
        .route("/tunnel/status", get(get_tunnel_status))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::TunnelState;

    /// Only a request that proves the person sent it gets past, and the two refusals are told
    /// apart so a person on a keyless daemon is never handed the sentence for a desktop daemon.
    /// The end-to-end measurement lives in `tests/tunnel_requires_user.rs` and
    /// `tests/tunnel_no_user_key.rs`, one per credential state, because the installed digest is
    /// a process-global `OnceLock`.
    #[test]
    fn only_a_proven_request_passes_the_tunnel_gate() {
        assert_eq!(tunnel_refusal(UserActionProof::Proven), None);
        assert_eq!(
            tunnel_refusal(UserActionProof::Unproven),
            Some(TUNNEL_NEEDS_USER)
        );
        assert_eq!(
            tunnel_refusal(UserActionProof::NoKeyInstalled),
            Some(TUNNEL_NO_USER_KEY)
        );
        assert_ne!(TUNNEL_NEEDS_USER, TUNNEL_NO_USER_KEY);
        assert!(!TUNNEL_NO_USER_KEY.contains("Ask them"));
    }

    /// Each renderer marker opens a toast with its own advice, and neither helps here.
    #[test]
    fn the_refusals_carry_no_renderer_marker() {
        for refusal in [TUNNEL_NEEDS_USER, TUNNEL_NO_USER_KEY] {
            assert!(!refusal.contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER));
            assert!(!refusal.contains(crate::routes::session::COPY_OF_PRIVATE_REFUSAL_MARKER));
        }
    }

    #[test]
    fn status_shows_the_url_and_secret_only_to_the_person() {
        let info = || TunnelInfo {
            state: TunnelState::Running,
            url: "https://relay.example/tunnel/agent".to_string(),
            hostname: "relay.example".to_string(),
            secret: "tunnel-secret".to_string(),
        };
        let shown = shown_to(info(), UserActionProof::Proven);
        assert_eq!(shown.url, info().url);
        assert_eq!(shown.secret, info().secret);
        for proof in [UserActionProof::Unproven, UserActionProof::NoKeyInstalled] {
            let shown = shown_to(info(), proof);
            assert_eq!(
                shown.state,
                TunnelState::Running,
                "the state is still shown"
            );
            assert!(shown.url.is_empty() && shown.hostname.is_empty() && shown.secret.is_empty());
        }
    }
}
