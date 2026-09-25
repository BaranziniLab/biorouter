use axum::extract::{DefaultBodyLimit, Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::crew::local_files::{CredentialRefusal, CREDENTIAL_REFUSAL_CODE};
use biorouter_server::crew::transfers::{service, FileRequest, PreviewRequest, StartRequest};
use serde::Deserialize;
use serde_json::{json, Value};

type TransferResult = Result<Json<Value>, TransferError>;
/// The code every refusal carries unless it has a more specific one.
const TRANSFER_REFUSED_CODE: &str = "crew_transfer_refused";
pub struct TransferError(StatusCode, String, &'static str);
impl From<anyhow::Error> for TransferError {
    /// A credential refusal (Q3-01) keeps its own code and its plain sentence, wherever in the
    /// chain it sits; every other refusal is `crew_transfer_refused` with the error's text.
    fn from(error: anyhow::Error) -> Self {
        if let Some(refusal) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<CredentialRefusal>())
        {
            return Self(
                StatusCode::BAD_REQUEST,
                refusal.to_string(),
                CREDENTIAL_REFUSAL_CODE,
            );
        }
        Self(
            StatusCode::BAD_REQUEST,
            error.to_string(),
            TRANSFER_REFUSED_CODE,
        )
    }
}
impl IntoResponse for TransferError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"code":self.2,"error":self.1}))).into_response()
    }
}
fn human(headers: &HeaderMap) -> Result<(), TransferError> {
    if !matches!(user_action_proof(headers), UserActionProof::Proven) {
        return Err(TransferError(StatusCode::FORBIDDEN,
            "A verified human action is required for local file and transfer access; agent grants and API keys do not authorize it".into(), TRANSFER_REFUSED_CODE));
    }
    Ok(())
}
#[utoipa::path(post, operation_id = "crew_transfer_register_file", path = "/crew/files", request_body = Value, responses((status = 200, body = Value)), tag = "Crew")]
pub async fn register_file(headers: HeaderMap, Json(body): Json<FileRequest>) -> TransferResult {
    human(&headers)?;
    Ok(Json(service().await?.register(body).await?))
}
#[utoipa::path(post, operation_id = "crew_transfer_confirm_file", path = "/crew/files/{capability_id}/confirm", params(("capability_id" = String, Path, description = "Pending file selection capability")), responses((status = 200, body = Value)), tag = "Crew")]
pub async fn confirm_file(headers: HeaderMap, Path(capability_id): Path<String>) -> TransferResult {
    human(&headers)?;
    Ok(Json(service().await?.confirm(&capability_id).await?))
}
#[utoipa::path(delete, operation_id = "crew_transfer_discard_file", path = "/crew/files/{capability_id}", params(("capability_id" = String, Path, description = "Unused file selection capability")), responses((status = 200, body = Value)), tag = "Crew")]
pub async fn discard_file(headers: HeaderMap, Path(capability_id): Path<String>) -> TransferResult {
    human(&headers)?;
    Ok(Json(service().await?.discard(&capability_id).await?))
}
#[utoipa::path(post, operation_id = "crew_transfer_start", path = "/crew/transfers", request_body = Value, responses((status = 200, body = Value)), tag = "Crew")]
pub async fn start(headers: HeaderMap, Json(body): Json<StartRequest>) -> TransferResult {
    human(&headers)?;
    Ok(Json(
        serde_json::to_value(service().await?.start(body).await?).map_err(anyhow::Error::from)?,
    ))
}
#[derive(Deserialize)]
pub struct Filter {
    connection_id: Option<String>,
    channel_id: Option<String>,
}
#[utoipa::path(get, operation_id = "crew_transfer_list", path = "/crew/transfers", responses((status = 200, body = Value)), tag = "Crew")]
pub async fn list(headers: HeaderMap, Query(filter): Query<Filter>) -> TransferResult {
    human(&headers)?;
    let transfers: Vec<_> = service()
        .await?
        .list()
        .await
        .into_iter()
        .filter(|receipt| {
            filter
                .connection_id
                .as_ref()
                .is_none_or(|id| id == &receipt.connection_id)
                && filter
                    .channel_id
                    .as_ref()
                    .is_none_or(|id| id == &receipt.channel_id)
        })
        .collect();
    Ok(Json(json!({"transfers":transfers})))
}
#[utoipa::path(get, operation_id = "crew_transfer_status", path = "/crew/transfers/{id}", params(("id" = String, Path, description = "Transfer ID")), responses((status = 200, body = Value)), tag = "Crew")]
pub async fn status(headers: HeaderMap, Path(id): Path<String>) -> TransferResult {
    human(&headers)?;
    Ok(Json(
        serde_json::to_value(service().await?.get(&id).await?).map_err(anyhow::Error::from)?,
    ))
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Resume {
    file_capability: String,
}
#[utoipa::path(post, operation_id = "crew_transfer_resume", path = "/crew/transfers/{id}/resume", params(("id" = String, Path, description = "Transfer ID")), request_body = Value, responses((status = 200, body = Value)), tag = "Crew")]
pub async fn resume(
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Resume>,
) -> TransferResult {
    human(&headers)?;
    Ok(Json(
        serde_json::to_value(service().await?.resume(&id, &body.file_capability).await?)
            .map_err(anyhow::Error::from)?,
    ))
}
#[utoipa::path(post, operation_id = "crew_transfer_pause", path = "/crew/transfers/{id}/pause", params(("id" = String, Path, description = "Transfer ID")), responses((status = 200, body = Value)), tag = "Crew")]
pub async fn pause(headers: HeaderMap, Path(id): Path<String>) -> TransferResult {
    human(&headers)?;
    Ok(Json(
        serde_json::to_value(service().await?.pause(&id).await?).map_err(anyhow::Error::from)?,
    ))
}
#[utoipa::path(delete, operation_id = "crew_transfer_forget", path = "/crew/transfers/{id}", params(("id" = String, Path, description = "Transfer ID")), responses((status = 200, body = Value)), tag = "Crew")]
pub async fn forget(
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<Resume>>,
) -> TransferResult {
    human(&headers)?;
    service()
        .await?
        .forget(&id, body.as_ref().map(|body| body.file_capability.as_str()))
        .await?;
    Ok(Json(
        json!({"forgotten":true,"message":"Receipt removed after authorized partial cleanup. Remote attachments and published downloads are not deleted."}),
    ))
}
#[utoipa::path(post, operation_id = "crew_transfer_preview", path = "/crew/transfers/preview", request_body = Value, responses((status = 200, description = "Digest-verified bounded image")), tag = "Crew")]
pub async fn preview(
    headers: HeaderMap,
    Json(body): Json<PreviewRequest>,
) -> Result<Response, TransferError> {
    human(&headers)?;
    let (media_type, bytes) = service().await?.preview(body).await?;
    Ok((
        [
            ("content-type", media_type),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            ("content-security-policy", "default-src 'none'; sandbox"),
        ],
        bytes,
    )
        .into_response())
}
pub fn routes() -> Router {
    Router::new()
        .route("/crew/files", post(register_file))
        .route("/crew/files/{capability_id}/confirm", post(confirm_file))
        .route("/crew/files/{capability_id}", delete(discard_file))
        .route("/crew/transfers", get(list).post(start))
        .route("/crew/transfers/preview", post(preview))
        .route("/crew/transfers/{id}", get(status).delete(forget))
        .route("/crew/transfers/{id}/resume", post(resume))
        .route("/crew/transfers/{id}/pause", post(pause))
        .layer(DefaultBodyLimit::max(16 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn transfer_operations_require_verified_human_action() {
        let error = human(&HeaderMap::new()).unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
        assert!(error.1.contains("verified human action"));
        assert_eq!(error.2, "crew_transfer_refused");
    }

    async fn body_of(error: TransferError) -> (StatusCode, Value) {
        let response = error.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// Q3-01: a credential refusal answers 400 with its own code and the plain sentence, even
    /// when something on the way wrapped it in context.
    #[tokio::test]
    async fn a_credential_refusal_keeps_its_code_and_sentence() {
        let source = anyhow::Error::from(CredentialRefusal::Source {
            name: "secrets.yaml".into(),
        });
        let (status, body) = body_of(source.into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({
                "code": "crew_file_is_credential",
                "error": "\u{201c}secrets.yaml\u{201d} looks like a credential file (a password, key or token store), so Crew won't share it."
            })
        );

        let wrapped = anyhow::Error::from(CredentialRefusal::Destination)
            .context("while selecting the destination");
        let (status, body) = body_of(wrapped.into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "crew_file_is_credential");
        assert_eq!(
            body["error"],
            "Crew won't save into a credential or settings location. Choose another folder."
        );
    }

    #[tokio::test]
    async fn every_other_refusal_keeps_the_general_code() {
        let (status, body) =
            body_of(anyhow::anyhow!("Symlink file selections are not supported").into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({"code":"crew_transfer_refused","error":"Symlink file selections are not supported"})
        );
    }

    #[tokio::test]
    async fn confirm_and_discard_refuse_missing_human_proof_before_service_lookup() {
        let confirm = confirm_file(HeaderMap::new(), Path("missing-capability".into()))
            .await
            .unwrap_err();
        assert_eq!(confirm.0, StatusCode::FORBIDDEN);

        let discard = discard_file(HeaderMap::new(), Path("missing-capability".into()))
            .await
            .unwrap_err();
        assert_eq!(discard.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn api_key_is_not_human_proof_for_confirm_or_discard() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-secret-key",
            HeaderValue::from_static("synthetic-api-key"),
        );

        let confirm = confirm_file(headers.clone(), Path("missing-capability".into()))
            .await
            .unwrap_err();
        assert_eq!(confirm.0, StatusCode::FORBIDDEN);

        let discard = discard_file(headers, Path("missing-capability".into()))
            .await
            .unwrap_err();
        assert_eq!(discard.0, StatusCode::FORBIDDEN);
    }
}
