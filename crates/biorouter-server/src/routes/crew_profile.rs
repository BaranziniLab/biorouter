use axum::{
    extract::{DefaultBodyLimit, Path},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use biorouter::crew::manager;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

static SECRET_OPERATIONS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
pub struct Refusal(StatusCode, anyhow::Error);
impl From<anyhow::Error> for Refusal {
    fn from(error: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, error)
    }
}
impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"code":"crew_profile_refused","error":self.1.to_string()})),
        )
            .into_response()
    }
}
type Result = std::result::Result<Json<Value>, Refusal>;
fn person(headers: &HeaderMap) -> std::result::Result<(), Refusal> {
    if user_action_proof(headers) != UserActionProof::Proven {
        return Err(Refusal(
            StatusCode::FORBIDDEN,
            anyhow::anyhow!("Crew profile operations require the existing human approval secret"),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretBody {
    #[serde(deserialize_with = "secret")]
    passphrase: Zeroizing<String>,
}
fn secret<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}

#[utoipa::path(get, operation_id = "crew_profile_credentials", path="/crew/credentials", responses((status=200,body=Value)),tag="Crew")]
pub async fn credentials(headers: HeaderMap) -> Result {
    person(&headers)?;
    Ok(Json(
        serde_json::to_value(manager()?.credential_status().await?).map_err(anyhow::Error::from)?,
    ))
}
#[utoipa::path(post, operation_id = "crew_profile_init", path="/crew/credentials/init",request_body=Value,responses((status=200,body=Value)),tag="Crew")]
pub async fn init(headers: HeaderMap, Json(body): Json<SecretBody>) -> Result {
    change_secret(headers, body, true).await
}
#[utoipa::path(post, operation_id = "crew_profile_unlock", path="/crew/credentials/unlock",request_body=Value,responses((status=200,body=Value)),tag="Crew")]
pub async fn unlock(headers: HeaderMap, Json(body): Json<SecretBody>) -> Result {
    change_secret(headers, body, false).await
}
async fn change_secret(headers: HeaderMap, body: SecretBody, initialize: bool) -> Result {
    person(&headers)?;
    if headers.get("X-User-Action").and_then(|h| h.to_str().ok()) == Some(body.passphrase.as_str())
    {
        return Err(anyhow::anyhow!(
            "The vault passphrase must differ from the human approval secret"
        )
        .into());
    }
    let _permit = SECRET_OPERATIONS.clone().try_acquire_owned().map_err(|_| {
        anyhow::anyhow!("Another credential operation is active; retry after it finishes")
    })?;
    let crew = manager()?;
    if initialize {
        crew.init_vault(body.passphrase).await?;
    } else {
        crew.unlock_vault(body.passphrase).await?;
    }
    Ok(Json(
        serde_json::to_value(crew.credential_status().await?).map_err(anyhow::Error::from)?,
    ))
}
#[utoipa::path(post, operation_id = "crew_profile_lock",path="/crew/credentials/lock",responses((status=200,body=Value)),tag="Crew")]
pub async fn lock(headers: HeaderMap) -> Result {
    person(&headers)?;
    let crew = manager()?;
    crew.lock_vault().await?;
    Ok(Json(
        serde_json::to_value(crew.credential_status().await?).map_err(anyhow::Error::from)?,
    ))
}
#[utoipa::path(get, operation_id = "crew_profile_grants",path="/crew/connections/{id}/grants",params(("id"=String,Path,description="Crew connection ID")),responses((status=200,body=Value)),tag="Crew")]
pub async fn grants(headers: HeaderMap, Path(id): Path<String>) -> Result {
    person(&headers)?;
    Ok(Json(manager()?.session_grants(&id).await?))
}
#[utoipa::path(get, operation_id = "crew_profile_context",path="/crew/connections/{id}/sessions/{session}/context",params(("id"=String,Path,description="Crew connection ID"),("session"=String,Path,description="Session ID")),responses((status=200,body=Value)),tag="Crew")]
pub async fn context(headers: HeaderMap, Path((id, session)): Path<(String, String)>) -> Result {
    person(&headers)?;
    let crew = manager()?;
    let metadata = crew
        .run_metadata(&session)
        .await
        .ok_or_else(|| anyhow::anyhow!("No Crew grant for this session"))?;
    if metadata.connection_id != id {
        return Err(anyhow::anyhow!("Session belongs to a different Crew connection").into());
    }
    Ok(Json(
        crew.worker_request(&session, "context.manifest", json!({}))
            .await?,
    ))
}
#[utoipa::path(post, operation_id = "crew_profile_revoke",path="/crew/connections/{id}/sessions/{session}/revoke",params(("id"=String,Path,description="Crew connection ID"),("session"=String,Path,description="Session ID")),responses((status=200,body=Value)),tag="Crew")]
pub async fn revoke(headers: HeaderMap, Path((id, session)): Path<(String, String)>) -> Result {
    person(&headers)?;
    let crew = manager()?;
    let metadata = crew
        .run_metadata(&session)
        .await
        .ok_or_else(|| anyhow::anyhow!("No Crew grant for this session"))?;
    if metadata.connection_id != id {
        return Err(anyhow::anyhow!("Session belongs to a different Crew connection").into());
    }
    Ok(Json(
        crew.cancel_run_if_current(&session, &metadata.run_id)
            .await?,
    ))
}
pub fn routes() -> Router {
    Router::new()
        .route("/crew/credentials", get(credentials))
        .route("/crew/credentials/init", post(init))
        .route("/crew/credentials/unlock", post(unlock))
        .route("/crew/credentials/lock", post(lock))
        .route("/crew/connections/{id}/grants", get(grants))
        .route(
            "/crew/connections/{id}/sessions/{session}/context",
            get(context),
        )
        .route(
            "/crew/connections/{id}/sessions/{session}/revoke",
            post(revoke),
        )
        .layer(DefaultBodyLimit::max(8 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_operations_refuse_without_human_proof() {
        let error = person(&HeaderMap::new()).unwrap_err();
        assert!(error.1.to_string().contains("human approval secret"));
    }
}
