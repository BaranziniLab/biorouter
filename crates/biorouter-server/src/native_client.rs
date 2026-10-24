use axum::{
    extract::ws::{Message, WebSocketUpgrade},
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use biorouter::native_client::{NativeClient, NativeClientLease};
use futures::StreamExt;

pub const HEADER: &str = "x-biorouter-native-client";

pub fn from_headers(headers: &HeaderMap) -> Result<Option<NativeClient>, StatusCode> {
    let Some(value) = headers.get(HEADER) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
    if value == "unavailable" {
        return Ok(Some(NativeClient::unavailable()));
    }
    if crate::auth::user_action_proof(headers) != crate::auth::UserActionProof::Proven {
        return Err(StatusCode::FORBIDDEN);
    }
    NativeClient::lookup(value)
        .map(Some)
        .ok_or(StatusCode::FORBIDDEN)
}

pub async fn scope_request(request: Request, next: Next) -> Response {
    match from_headers(request.headers()) {
        Ok(client) => biorouter::native_client::scope(client, next.run(request)).await,
        Err(status) => status.into_response(),
    }
}

async fn connect(headers: HeaderMap, ws: WebSocketUpgrade) -> Result<Response, StatusCode> {
    if crate::auth::user_action_proof(&headers) != crate::auth::UserActionProof::Proven
        || !headers.contains_key("x-daemon-instance")
        || !crate::daemon_service::instance_matches(&headers)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let lease = NativeClientLease::register().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(ws
        .max_message_size(256)
        .max_frame_size(256)
        .on_upgrade(move |mut socket| async move {
            if socket
                .send(Message::Text(lease.client().id().to_string().into()))
                .await
                .is_err()
            {
                return;
            }
            while let Some(message) = socket.next().await {
                match message {
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(Message::Ping(bytes)) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            drop(lease);
        }))
}

pub fn routes() -> Router {
    Router::new().route("/daemon/native-client", get(connect))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };

    #[test]
    fn native_owner_needs_user_proof_and_an_exact_live_server_minted_id() {
        install_test_user_action_key();
        let lease = NativeClientLease::register().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-secret-key", "ordinary-api-secret".parse().unwrap());
        headers.insert(HEADER, lease.client().id().to_string().parse().unwrap());
        assert!(from_headers(&headers).is_err());
        headers.insert("x-user-action", TEST_USER_ACTION_KEY.parse().unwrap());
        assert_eq!(
            from_headers(&headers).unwrap().unwrap().id(),
            lease.client().id()
        );
        headers.insert(HEADER, uuid::Uuid::new_v4().to_string().parse().unwrap());
        assert!(from_headers(&headers).is_err());
        headers.insert(HEADER, lease.client().id().to_string().parse().unwrap());
        drop(lease);
        assert!(from_headers(&headers).is_err());
        headers.insert(HEADER, "unavailable".parse().unwrap());
        assert!(!from_headers(&headers).unwrap().unwrap().is_live());
        headers.remove(HEADER);
        assert!(from_headers(&headers).unwrap().is_none());
    }
}
