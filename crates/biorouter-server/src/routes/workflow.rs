use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::routing::get;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use biorouter::workflow::service;
use biorouter::workflow::validate_workflow::validate_workflow_template_from_content;
use biorouter::workflow::Workflow;
use biorouter::{slash_commands, workflow_deeplink};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_path_to_error::deserialize as deserialize_with_path;
use utoipa::ToSchema;

fn format_json_rejection_message(rejection: &JsonRejection) -> String {
    match rejection {
        JsonRejection::JsonDataError(err) => {
            format!("Request body validation failed: {}", clean_data_error(err))
        }
        JsonRejection::JsonSyntaxError(err) => format!("Invalid JSON payload: {}", err.body_text()),
        JsonRejection::MissingJsonContentType(err) => err.body_text(),
        JsonRejection::BytesRejection(err) => err.body_text(),
        _ => rejection.body_text(),
    }
}

fn clean_data_error(err: &axum::extract::rejection::JsonDataError) -> String {
    let message = err.body_text();
    message
        .strip_prefix("Failed to deserialize the JSON body into the target type: ")
        .map(|s| s.to_string())
        .unwrap_or_else(|| message.to_string())
}

use crate::routes::errors::ErrorResponse;
use crate::routes::workflow_utils::{
    get_all_workflows_manifests, get_workflow_file_path_by_id, short_id_from_path,
    validate_workflow, WorkflowManifest, WorkflowValidationError,
};
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkflowRequest {
    session_id: String,
    #[serde(default)]
    author: Option<AuthorRequest>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorRequest {
    #[serde(default)]
    contact: Option<String>,
    #[serde(default)]
    metadata: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateWorkflowResponse {
    workflow: Option<Workflow>,
    error: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EncodeWorkflowRequest {
    workflow: Workflow,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EncodeWorkflowResponse {
    deeplink: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DecodeWorkflowRequest {
    deeplink: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecodeWorkflowResponse {
    workflow: Workflow,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScanWorkflowRequest {
    workflow: Workflow,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScanWorkflowResponse {
    has_security_warnings: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveWorkflowRequest {
    workflow: Workflow,
    id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SaveWorkflowResponse {
    id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct ParseWorkflowRequest {
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParseWorkflowResponse {
    pub workflow: Workflow,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteWorkflowRequest {
    id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListWorkflowResponse {
    manifests: Vec<WorkflowManifest>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScheduleWorkflowRequest {
    id: String,
    cron_schedule: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSlashCommandRequest {
    id: String,
    slash_command: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkflowToYamlRequest {
    workflow: Workflow,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowToYamlResponse {
    yaml: String,
}

#[utoipa::path(
    post,
    path = "/workflows/create",
    request_body = CreateWorkflowRequest,
    responses(
        (status = 200, description = "Workflow created successfully. Its `knowledge_bases` names \
                                      only the bases this caller may open", body = CreateWorkflowResponse),
        (status = 400, description = "Bad request"),
        (status = 403, description = "Refused by a privacy boundary: `session_id` names a chat \
                                      this caller may not reach, answered with the same refusal, \
                                      word for word, that `GET /sessions/{session_id}` gives \
                                      (body = plain text)"),
        (status = 412, description = "Precondition failed - Agent not available"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Workflow Management"
)]
async fn create_workflow(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<CreateWorkflowRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Issue #56, QA 2026-09-10 (F0's sweep). This loads the named chat's WHOLE
    // transcript and hands back a workflow a model wrote from it — the
    // transcript again, summarised — so it asks the read's gate first, before
    // the chat is loaded or an agent is built for it.
    if let Err(refusal) = crate::routes::session_reach::session_reach(
        state.session_manager(),
        &request.session_id,
        &headers,
    )
    .await
    {
        return refusal.into_response();
    }
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    match workflow_from_session(&state, request).await {
        Ok(Json(mut response)) => {
            // The enrichment records the chat's visible knowledge bases, which
            // can include a private base even for a public chat. Named only as
            // far as this caller may open them — the rule `GET
            // /knowledge/active` applies to the same list.
            if let Some(bases) = response
                .workflow
                .as_mut()
                .and_then(|workflow| workflow.knowledge_bases.as_mut())
            {
                let root = state.knowledge_service.root();
                bases
                    .visible
                    .retain(|id| caller.reach_knowledge_base(root, id).is_ok());
                if bases
                    .default
                    .as_deref()
                    .is_some_and(|id| caller.reach_knowledge_base(root, id).is_err())
                {
                    bases.default = None;
                }
            }
            Json(response).into_response()
        }
        Err(status) => status.into_response(),
    }
}

/// The body of [`create_workflow`], once the caller may address the chat.
async fn workflow_from_session(
    state: &Arc<AppState>,
    request: CreateWorkflowRequest,
) -> Result<Json<CreateWorkflowResponse>, StatusCode> {
    tracing::info!(
        "Workflow creation request received for session_id: {}",
        request.session_id
    );

    let session = match state
        .session_manager()
        .get_session(&request.session_id, true)
        .await
    {
        Ok(session) => session,
        Err(e) => {
            tracing::error!("Failed to get session: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let conversation = match session.conversation {
        Some(conversation) => conversation,
        None => {
            let error_message = "Session has no conversation".to_string();
            let error_response = CreateWorkflowResponse {
                workflow: None,
                error: Some(error_message),
            };
            return Ok(Json(error_response));
        }
    };

    let agent = state
        .get_agent_for_route(request.session_id.clone())
        .await?;

    let workflow_result = agent.create_workflow(conversation).await;

    match workflow_result {
        Ok(mut workflow) => {
            // Issue: the ONE enrichment. This block used to be inline here, and
            // the CLI's copy of the same flow had no equivalent — same
            // conversation, same generator, two different documents. Both
            // surfaces now call `service::apply_session_enrichment`.
            let enrichment = match service::session_enrichment(
                agent.as_ref(),
                &state.knowledge_service,
                &request.session_id,
                request.author.map(|author| biorouter::workflow::Author {
                    contact: author.contact,
                    metadata: author.metadata,
                }),
            )
            .await
            {
                Ok(enrichment) => enrichment,
                Err(err) => {
                    tracing::error!("Failed to enrich the generated workflow: {}", err);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            };
            service::apply_session_enrichment(&mut workflow, enrichment);

            Ok(Json(CreateWorkflowResponse {
                workflow: Some(workflow),
                error: None,
            }))
        }
        Err(e) => {
            tracing::error!("Error details: {:?}", e);
            let error_response = CreateWorkflowResponse {
                workflow: None,
                error: Some(format!("Failed to create workflow: {}", e)),
            };
            Ok(Json(error_response))
        }
    }
}

#[utoipa::path(
    post,
    path = "/workflows/encode",
    request_body = EncodeWorkflowRequest,
    responses(
        (status = 200, description = "Workflow encoded successfully", body = EncodeWorkflowResponse),
        (status = 400, description = "Bad request")
    ),
    tag = "Workflow Management"
)]
async fn encode_workflow(
    Json(request): Json<EncodeWorkflowRequest>,
) -> Result<Json<EncodeWorkflowResponse>, StatusCode> {
    match workflow_deeplink::encode(&request.workflow) {
        Ok(encoded) => Ok(Json(EncodeWorkflowResponse { deeplink: encoded })),
        Err(err) => {
            tracing::error!("Failed to encode workflow: {}", err);
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

#[utoipa::path(
    post,
    path = "/workflows/decode",
    request_body = DecodeWorkflowRequest,
    responses(
        (status = 200, description = "Workflow decoded successfully", body = DecodeWorkflowResponse),
        (status = 400, description = "Bad request")
    ),
    tag = "Workflow Management"
)]
async fn decode_workflow(
    Json(request): Json<DecodeWorkflowRequest>,
) -> Result<Json<DecodeWorkflowResponse>, StatusCode> {
    match workflow_deeplink::decode(&request.deeplink) {
        Ok(workflow) => match validate_workflow(&workflow) {
            Ok(_) => Ok(Json(DecodeWorkflowResponse { workflow })),
            Err(WorkflowValidationError { status, .. }) => Err(status),
        },
        Err(err) => {
            tracing::error!("Failed to decode deeplink: {}", err);
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

#[utoipa::path(
    post,
    path = "/workflows/scan",
    request_body = ScanWorkflowRequest,
    responses(
        (status = 200, description = "Workflow scanned successfully", body = ScanWorkflowResponse),
    ),
    tag = "Workflow Management"
)]
async fn scan_workflow(
    Json(request): Json<ScanWorkflowRequest>,
) -> Result<Json<ScanWorkflowResponse>, StatusCode> {
    let has_security_warnings = request.workflow.check_for_security_warnings();

    Ok(Json(ScanWorkflowResponse {
        has_security_warnings,
    }))
}

#[utoipa::path(
    get,
    path = "/workflows/list",
    responses(
        (status = 200, description = "Get workflow list successfully", body = ListWorkflowResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Workflow Management"
)]
async fn list_workflows(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ListWorkflowResponse>, StatusCode> {
    let mut manifests = get_all_workflows_manifests().unwrap_or_default();

    let scheduler = state.scheduler();
    let scheduled_jobs = scheduler.list_scheduled_jobs().await;
    let schedule_map: HashMap<_, _> = scheduled_jobs
        .into_iter()
        .map(|j| (PathBuf::from(j.source), j.cron))
        .collect();

    let all_commands = slash_commands::list_commands();
    let slash_map: HashMap<_, _> = all_commands
        .into_iter()
        .map(|sc| (PathBuf::from(sc.workflow_path), sc.command))
        .collect();

    service::attach_bindings(&mut manifests, &schedule_map, &slash_map);

    Ok(Json(ListWorkflowResponse { manifests }))
}

#[utoipa::path(
    post,
    path = "/workflows/delete",
    request_body = DeleteWorkflowRequest,
    responses(
        (status = 204, description = "Workflow deleted successfully"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 404, description = "Workflow not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Workflow Management"
)]
async fn delete_workflow(Json(request): Json<DeleteWorkflowRequest>) -> StatusCode {
    // Through the core, so the id->path cache is invalidated by the same call
    // that removes the file. A bare `fs::remove_file` here left the deleted
    // workflow resolvable until the next `GET /workflows/list`.
    match service::delete(&request.id) {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(err) => {
            tracing::error!("Failed to delete workflow {}: {}", request.id, err);
            if err.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

#[utoipa::path(
    post,
    path = "/workflows/schedule",
    request_body = ScheduleWorkflowRequest,
    responses(
        (status = 200, description = "Workflow scheduled successfully"),
        (status = 404, description = "Workflow not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Workflow Management"
)]
async fn schedule_workflow(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScheduleWorkflowRequest>,
) -> Result<StatusCode, StatusCode> {
    let file_path = match get_workflow_file_path_by_id(&request.id) {
        Ok(path) => path,
        Err(err) => return Err(err.status),
    };

    let scheduler = state.scheduler();
    match scheduler
        .schedule_workflow(file_path, request.cron_schedule)
        .await
    {
        Ok(_) => Ok(StatusCode::OK),
        Err(e) => {
            tracing::error!("Failed to schedule workflow: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[utoipa::path(
    post,
    path = "/workflows/slash-command",
    request_body = SetSlashCommandRequest,
    responses(
        (status = 200, description = "Slash command set successfully"),
        (status = 404, description = "Workflow not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Workflow Management"
)]
async fn set_workflow_slash_command(
    Json(request): Json<SetSlashCommandRequest>,
) -> Result<StatusCode, StatusCode> {
    let file_path = match get_workflow_file_path_by_id(&request.id) {
        Ok(path) => path,
        Err(err) => return Err(err.status),
    };

    match slash_commands::set_workflow_slash_command(file_path, request.slash_command) {
        Ok(_) => Ok(StatusCode::OK),
        Err(e) => {
            tracing::error!("Failed to set slash command: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[utoipa::path(
    post,
    path = "/workflows/save",
    request_body = SaveWorkflowRequest,
    responses(
        (status = 204, description = "Workflow saved to file successfully", body = SaveWorkflowResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "Workflow Management"
)]
async fn save_workflow(
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Json<SaveWorkflowResponse>, ErrorResponse> {
    let Json(raw_json) = payload.map_err(json_rejection_to_error_response)?;
    let request = deserialize_save_workflow_request(raw_json)?;
    let has_security_warnings = request.workflow.check_for_security_warnings();
    if has_security_warnings {
        return Err(ErrorResponse {
            message: "This workflow contains hidden characters that could be malicious. Please remove them before trying to save.".to_string(),
            status: StatusCode::BAD_REQUEST,
        });
    }
    ensure_workflow_valid(&request.workflow)?;

    let target = match request.id.as_ref() {
        Some(id) => service::SaveTarget::ExistingId(id.clone()),
        None => service::SaveTarget::Library,
    };

    match service::save(&request.workflow, target) {
        Ok(save_file_path) => Ok(Json(SaveWorkflowResponse {
            id: short_id_from_path(&save_file_path.display().to_string()),
        })),
        Err(e) => Err(ErrorResponse {
            message: e.to_string(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }),
    }
}

fn json_rejection_to_error_response(rejection: JsonRejection) -> ErrorResponse {
    ErrorResponse {
        message: format_json_rejection_message(&rejection),
        status: StatusCode::BAD_REQUEST,
    }
}

fn ensure_workflow_valid(workflow: &Workflow) -> Result<(), ErrorResponse> {
    if let Err(err) = validate_workflow(workflow) {
        return Err(ErrorResponse {
            message: err.message,
            status: err.status,
        });
    }
    Ok(())
}

fn deserialize_save_workflow_request(value: Value) -> Result<SaveWorkflowRequest, ErrorResponse> {
    let payload = value.to_string();
    let mut deserializer = serde_json::Deserializer::from_str(&payload);
    let result: Result<SaveWorkflowRequest, _> = deserialize_with_path(&mut deserializer);
    result.map_err(|err| {
        let path = err.path().to_string();
        let inner = err.into_inner();
        let message = if path.is_empty() {
            format!("Save workflow validation failed: {}", inner)
        } else {
            format!(
                "save workflow validation failed at {}: {}",
                path.trim_start_matches('.'),
                inner
            )
        };
        ErrorResponse {
            message,
            status: StatusCode::BAD_REQUEST,
        }
    })
}

#[utoipa::path(
    post,
    path = "/workflows/parse",
    request_body = ParseWorkflowRequest,
    responses(
        (status = 200, description = "Workflow parsed successfully", body = ParseWorkflowResponse),
        (status = 400, description = "Bad request - Invalid workflow format", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "Workflow Management"
)]
async fn parse_workflow(
    Json(request): Json<ParseWorkflowRequest>,
) -> Result<Json<ParseWorkflowResponse>, ErrorResponse> {
    let workflow =
        validate_workflow_template_from_content(&request.content, None).map_err(|e| {
            ErrorResponse {
                message: format!("Invalid workflow format: {}", e),
                status: StatusCode::BAD_REQUEST,
            }
        })?;

    Ok(Json(ParseWorkflowResponse { workflow }))
}

#[utoipa::path(
    post,
    path = "/workflows/to-yaml",
    request_body = WorkflowToYamlRequest,
    responses(
        (status = 200, description = "Workflow converted to YAML successfully", body = WorkflowToYamlResponse),
        (status = 400, description = "Bad request - Failed to convert workflow to YAML", body = ErrorResponse),
    ),
    tag = "Workflow Management"
)]
async fn workflow_to_yaml(
    Json(request): Json<WorkflowToYamlRequest>,
) -> Result<Json<WorkflowToYamlResponse>, ErrorResponse> {
    let yaml = request.workflow.to_yaml().map_err(|e| ErrorResponse {
        message: format!("Failed to convert workflow to YAML: {}", e),
        status: StatusCode::BAD_REQUEST,
    })?;

    Ok(Json(WorkflowToYamlResponse { yaml }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/workflows/create", post(create_workflow))
        .route("/workflows/encode", post(encode_workflow))
        .route("/workflows/decode", post(decode_workflow))
        .route("/workflows/scan", post(scan_workflow))
        .route("/workflows/list", get(list_workflows))
        .route("/workflows/delete", post(delete_workflow))
        .route("/workflows/schedule", post(schedule_workflow))
        .route("/workflows/slash-command", post(set_workflow_slash_command))
        .route("/workflows/save", post(save_workflow))
        .route("/workflows/parse", post(parse_workflow))
        .route("/workflows/to-yaml", post(workflow_to_yaml))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::workflow::Workflow;

    #[tokio::test]
    async fn test_decode_and_encode_workflow() {
        let original_workflow = Workflow::builder()
            .title("Test Workflow")
            .description("A test workflow")
            .instructions("Test instructions")
            .build()
            .unwrap();
        let encoded = workflow_deeplink::encode(&original_workflow).unwrap();

        let request = DecodeWorkflowRequest {
            deeplink: encoded.clone(),
        };
        let response = decode_workflow(Json(request)).await;

        assert!(response.is_ok());
        let decoded = response.unwrap().0.workflow;
        assert_eq!(decoded.title, original_workflow.title);
        assert_eq!(decoded.description, original_workflow.description);
        assert_eq!(decoded.instructions, original_workflow.instructions);

        let encode_request = EncodeWorkflowRequest { workflow: decoded };
        let encode_response = encode_workflow(Json(encode_request)).await;

        assert!(encode_response.is_ok());
        let encoded_again = encode_response.unwrap().0.deeplink;
        assert!(!encoded_again.is_empty());
        assert_eq!(encoded, encoded_again);
    }
}
