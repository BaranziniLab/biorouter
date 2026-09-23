//! Native clients only: terminal credentials never enter model or replay streams.
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use biorouter::crew::authentication::{self, TerminalEvent};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

type ApiError = (StatusCode, Json<serde_json::Value>);
fn refused(message: &str) -> ApiError {
    (StatusCode::FORBIDDEN, Json(json!({"error":message})))
}
fn person(headers: &HeaderMap) -> Result<(), ApiError> {
    if matches!(user_action_proof(headers), UserActionProof::Proven) {
        Ok(())
    } else {
        Err(refused("Verified human Crew authority is required"))
    }
}
fn controller(headers: &HeaderMap) -> Result<String, ApiError> {
    person(headers)?;
    let value = headers
        .get("X-Crew-Controller")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| refused("Authentication controller required"))?;
    uuid::Uuid::parse_str(value).map_err(|_| refused("Invalid authentication controller"))?;
    Ok(value.into())
}
fn failure(error: anyhow::Error) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":error.to_string()})),
    )
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    request_id: String,
    controller_id: String,
    cols: u16,
    rows: u16,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

#[utoipa::path(post, operation_id = "crew_authentication_prepare", path = "/crew/connections/{id}/authentication", params(("id" = String, Path, description = "Saved Crew connection")), request_body = Prepare, responses((status = 200, body = serde_json::Value)), tag = "Crew")]
pub async fn prepare(
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Prepare>,
) -> Result<Json<authentication::AuthenticationSession>, ApiError> {
    person(&headers)?;
    authentication::prepare(
        &id,
        &body.request_id,
        &body.controller_id,
        body.cols,
        body.rows,
    )
    .await
    .map(Json)
    .map_err(failure)
}
#[utoipa::path(delete, operation_id = "crew_authentication_cancel", path = "/crew/authentication/{id}", params(("id" = String, Path, description = "Authentication session")), responses((status = 200, body = serde_json::Value)), tag = "Crew")]
pub async fn cancel(
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let controller = controller(&headers)?;
    authentication::cancel_and_disconnect(&id, &controller)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"cancelled":true})))
}
#[utoipa::path(get, operation_id = "crew_authentication_terminal", path = "/crew/authentication/{id}/terminal", params(("id" = String, Path, description = "Authentication session")), responses((status = 101, description = "Human-authorized terminal websocket")), tag = "Crew")]
pub async fn terminal(
    headers: HeaderMap,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let controller = controller(&headers)?;
    if headers.contains_key("origin") {
        return Err(refused(
            "Use the native authenticated Crew terminal adapter",
        ));
    }
    authentication::validate(&id, &controller)
        .await
        .map_err(failure)?;
    Ok(ws
        .max_message_size(8192)
        .max_frame_size(8192)
        .on_upgrade(move |socket| serve(socket, headers, id, controller))
        .into_response())
}
struct AttachedTerminal {
    id: String,
    controller: String,
}
impl Drop for AttachedTerminal {
    fn drop(&mut self) {
        let id = self.id.clone();
        let controller = self.controller.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = authentication::detach(&id, &controller).await;
            });
        }
    }
}

async fn serve(mut socket: WebSocket, headers: HeaderMap, id: String, controller: String) {
    let Ok(mut output) = authentication::attach(&id, &controller).await else {
        let _ = socket.send(Message::Text(json!({"type":"error","code":authentication::ATTACH_FAILURE_CODE,"error":authentication::ATTACH_FAILURE_MESSAGE}).to_string().into())).await;
        return;
    };
    let _owned_terminal = AttachedTerminal {
        id: id.clone(),
        controller: controller.clone(),
    };
    let mut policy_check = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = policy_check.tick() => {
                if person(&headers).is_err() || authentication::validate(&id,&controller).await.is_err() { break; }
                match authentication::handoff(&id,&controller).await {
                    Ok(true) => {
                        let _ = tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(json!({"type":"exit","exit_code":0,"authenticated":true}).to_string().into()))).await;
                        break;
                    }
                    Ok(false) => (),
                    Err(_) => {
                        let _ = socket.send(Message::Text(json!({"type":"error","code":authentication::HANDOFF_FAILURE_CODE,"error":authentication::HANDOFF_FAILURE_MESSAGE}).to_string().into())).await;
                        break;
                    }
                }
            }
            event = output.recv() => {
                if person(&headers).is_err() || authentication::validate(&id,&controller).await.is_err() { break; }
                let message = match event {
                    Some(TerminalEvent::Data(bytes)) => Message::Binary(bytes.into()),
                    Some(TerminalEvent::Exit(code)) => {
                        let _ = socket.send(Message::Text(json!({"type":"exit","exit_code":code}).to_string().into())).await;
                        break;
                    }
                    None => break,
                };
                if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(message)).await,Ok(Ok(()))) { break; }
            }
            incoming = socket.recv() => {
                if person(&headers).is_err() || authentication::validate(&id,&controller).await.is_err() { break; }
                let result = match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<Input>(&text) {
                        Ok(Input::Input {data}) => {
                            let result = authentication::input(&id,&controller,&data);
                            // No retention after delivery to the bounded writer queue.
                            data.into_bytes().fill(0);
                            result
                        }
                        Ok(Input::Resize {cols,rows}) => authentication::resize(&id,&controller,cols,rows),
                        Err(_) => break,
                    },
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    _ => break,
                };
                if result.is_err() { break; }
            }
        }
    }
    let _ = authentication::detach(&id, &controller).await;
}
pub fn routes() -> Router {
    Router::new()
        .route("/crew/connections/{id}/authentication", post(prepare))
        .route("/crew/authentication/{id}", axum::routing::delete(cancel))
        .route("/crew/authentication/{id}/terminal", get(terminal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_requires_human_proof_before_controller_header() {
        let error = controller(&HeaderMap::new()).unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
        assert!(error.1["error"]
            .as_str()
            .unwrap()
            .contains("Verified human Crew authority"));
    }
}
