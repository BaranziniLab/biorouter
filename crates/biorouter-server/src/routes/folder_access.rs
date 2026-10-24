use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use biorouter::folder_access::{FolderAccessManager, FolderGrantView};
use biorouter_server::auth::{folder_access_action_proof, UserActionProof};
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::{IntoParams, ToSchema};

use crate::state::AppState;

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct FolderAccessSessionQuery {
    pub session_id: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevokeFolderAccessRequest {
    pub session_id: String,
    pub grant_id: String,
}

#[utoipa::path(get, path = "/action-required/folder-access", params(FolderAccessSessionQuery), responses((status = 200, body = Vec<FolderGrantView>), (status = 403, description = "Session is not reachable")))]
pub async fn list_folder_access(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FolderAccessSessionQuery>,
) -> Result<Json<Vec<FolderGrantView>>, StatusCode> {
    super::session_reach::session_reach(state.session_manager(), &query.session_id, &headers)
        .await
        .map_err(|error| error.status)?;
    let client = biorouter_server::native_client::from_headers(&headers)?;
    Ok(Json(
        biorouter::native_client::scope(client, async {
            FolderAccessManager::global().list_session(&query.session_id)
        })
        .await,
    ))
}

#[utoipa::path(post, path = "/action-required/folder-access/revoke", request_body = RevokeFolderAccessRequest, responses((status = 200, body = Value), (status = 403, description = "Human approval proof required")))]
pub async fn revoke_folder_access(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RevokeFolderAccessRequest>,
) -> Result<Json<Value>, StatusCode> {
    let proof = folder_access_action_proof(&headers);
    if proof != UserActionProof::Proven {
        return Err(
            if biorouter_server::auth::folder_access_action_locked_out() {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::FORBIDDEN
            },
        );
    }
    super::session_reach::session_reach(state.session_manager(), &request.session_id, &headers)
        .await
        .map_err(|error| error.status)?;
    let client = biorouter_server::native_client::from_headers(&headers)?;
    Ok(Json(biorouter::native_client::scope(client, async {
        json!({"revoked": FolderAccessManager::global().revoke(&request.session_id, &request.grant_id)})
    }).await))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/action-required/folder-access", get(list_folder_access))
        .route(
            "/action-required/folder-access/revoke",
            post(revoke_folder_access),
        )
        .with_state(state)
}
