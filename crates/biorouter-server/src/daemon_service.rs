use axum::{
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use biorouter::daemon_runtime::Identity;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{LazyLock, OnceLock};
use tokio_util::sync::CancellationToken;

static IDENTITY: OnceLock<Identity> = OnceLock::new();
static STOP: LazyLock<CancellationToken> = LazyLock::new(CancellationToken::new);

pub fn install(identity: Identity) -> anyhow::Result<()> {
    IDENTITY
        .set(identity)
        .map_err(|_| anyhow::anyhow!("Daemon identity already installed"))
}
pub fn shutdown_token() -> CancellationToken {
    STOP.clone()
}
pub fn instance_matches(headers: &HeaderMap) -> bool {
    match headers.get("X-Daemon-Instance") {
        None => true,
        Some(value) => IDENTITY
            .get()
            .is_some_and(|identity| value.to_str().ok() == Some(identity.instance_id.as_str())),
    }
}

async fn identity() -> Result<Json<Identity>, StatusCode> {
    IDENTITY
        .get()
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopRequest {
    instance_id: String,
}

async fn stop(
    headers: HeaderMap,
    Json(request): Json<StopRequest>,
) -> Result<Json<Value>, StatusCode> {
    if crate::auth::user_action_proof(&headers) != crate::auth::UserActionProof::Proven {
        return Err(StatusCode::FORBIDDEN);
    }
    let identity = IDENTITY.get().ok_or(StatusCode::NOT_FOUND)?;
    if identity.instance_id != request.instance_id {
        return Err(StatusCode::CONFLICT);
    }
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        STOP.cancel();
    });
    Ok(Json(
        json!({"stopping": true, "instance_id": identity.instance_id}),
    ))
}

pub fn routes() -> Router {
    Router::new()
        .route("/daemon/identity", get(identity))
        .route("/daemon/stop", post(stop))
}
