//! Trusting a server's host key the person verified (the owner's D2 ruling on AO-12): the
//! "Trust this key" button of the Crew view's trust panes, of a chat's host-key card and of a
//! terminal chat on the shared daemon.
//!
//! - `POST /crew/host-keys/read`: the key this computer reads from a server, straight from
//!   here, for a pane whose refusal printed no fingerprint (OpenSSH prints none for an unknown
//!   key under strict checking).
//! - `POST /crew/host-keys/trust`: add the server's key to the known hosts file, only when no
//!   key is known for it and only when the key the daemon reads again has the fingerprint the
//!   person was shown. A changed key is always refused.
//!
//! Both ask for proof that a person acted, with the codes and words of `routes::crew`'s person
//! gate, and no Crew tool action reaches them: a model can never trust a key. Everything they
//! do is `biorouter::crew::host_keys`; nothing a request carries becomes command text beyond a
//! checked host name, port and login.
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use biorouter::crew::host_keys::{
    self, HostKeyRead, HostKeyRefused, HostKeyTarget, HostKeyTrusted,
};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use serde_json::{json, Value};

use super::crew::wire::CrewJson;
use super::crew_authentication::{
    no_human_authority, HUMAN_AUTHORITY_UNAVAILABLE_CODE, REQUEST_INVALID_CODE,
    REQUEST_REFUSED_CODE, USER_ACTION_REQUIRED_CODE,
};

/// A host-key route's refusal: `{code, error}`, as every Crew refusal is.
pub struct HostKeyRouteError {
    status: StatusCode,
    code: String,
    error: String,
}

impl HostKeyRouteError {
    fn new(status: StatusCode, code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            error: error.into(),
        }
    }
}

impl IntoResponse for HostKeyRouteError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code": self.code, "error": self.error})),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for HostKeyRouteError {
    /// A refusal the host-key module typed keeps its code and sentence; anything else is
    /// `crew_request_refused` with a fixed sentence.
    fn from(error: anyhow::Error) -> Self {
        let Some(refused) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<HostKeyRefused>())
        else {
            return Self::new(
                StatusCode::CONFLICT,
                REQUEST_REFUSED_CODE,
                "Crew couldn't trust that host key. Add it in a terminal of your own.",
            );
        };
        let status = match refused.code {
            host_keys::INVALID => StatusCode::BAD_REQUEST,
            host_keys::UNREACHABLE => StatusCode::BAD_GATEWAY,
            _ => StatusCode::CONFLICT,
        };
        Self::new(status, refused.code, refused.message.clone())
    }
}

/// Proof that a person asked, with the person gate's codes and words.
fn person(headers: &HeaderMap) -> Result<(), HostKeyRouteError> {
    match user_action_proof(headers) {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(HostKeyRouteError::new(
            StatusCode::FORBIDDEN,
            USER_ACTION_REQUIRED_CODE,
            "Only a person using the Biorouter desktop app or the biorouter crew command can do this. Agent tools use their separate task grant.",
        )),
        UserActionProof::NoKeyInstalled => Err(HostKeyRouteError::new(
            StatusCode::FORBIDDEN,
            HUMAN_AUTHORITY_UNAVAILABLE_CODE,
            no_human_authority("This daemon cannot verify human Crew actions. Start it from the Biorouter desktop app or with biorouter crew daemon start."),
        )),
    }
}

/// Read `body` as a `T`, refused as a request the route does not take.
fn read_body<T: serde::de::DeserializeOwned>(body: Value) -> Result<T, HostKeyRouteError> {
    serde_json::from_value(body).map_err(|_| {
        HostKeyRouteError::new(
            StatusCode::BAD_REQUEST,
            REQUEST_INVALID_CODE,
            "Send the server's host name and port, and nothing else.",
        )
    })
}

/// Which server's key to read.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HostKeyReadRequest {
    /// The host as OpenSSH looked the key up by, without brackets or port.
    host: String,
    port: u16,
    /// The SSH login the refused step used (`you@lab-server`), so its own SSH settings are
    /// checked for a jump host or another name.
    #[serde(default)]
    login: Option<String>,
}

/// Which server's key to trust, and the fingerprint the person was shown and verified.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HostKeyTrustRequest {
    /// The host as OpenSSH looked the key up by, without brackets or port.
    host: String,
    port: u16,
    /// The SSH login the refused step used (`you@lab-server`).
    #[serde(default)]
    login: Option<String>,
    /// `SHA256:…`, as the person was shown it.
    fingerprint: String,
}

/// Read a server's host key straight from this computer.
#[utoipa::path(
    post,
    operation_id = "crew_host_keys_read",
    path = "/crew/host-keys/read",
    request_body = HostKeyReadRequest,
    responses(
        (status = 200, description = "The key this computer read from the server, straight from here with `ssh-keyscan`, the first of the types ssh prefers. Shown to the person to compare with the fingerprint their IT team gave them; nothing is written", body = HostKeyRead),
        (status = 400, description = "`crew_request_invalid`: a body that is not this route's, or a host, port or login that is not a plain name", body = CrewError),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 409, description = "`crew_request_refused`: the person's SSH settings reach the server through a jump host, a ProxyCommand or under a HostKeyAlias, or a saved Crew connection to it has a jump host (a key read from here could be another machine's), or the SSH settings or the saved connections could not be read", body = CrewError),
        (status = 502, description = "`crew_ssh_unreachable`: the server gave this computer no host key", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn read_key(
    headers: HeaderMap,
    CrewJson(body): CrewJson<Value>,
) -> Result<Json<HostKeyRead>, HostKeyRouteError> {
    person(&headers)?;
    let request: HostKeyReadRequest = read_body(body)?;
    let target = HostKeyTarget {
        host: request.host,
        port: request.port,
        login: request.login,
    };
    Ok(Json(host_keys::read(target).await?))
}

/// Trust a server's unknown host key the person verified.
#[utoipa::path(
    post,
    operation_id = "crew_host_keys_trust",
    path = "/crew/host-keys/trust",
    request_body = HostKeyTrustRequest,
    responses(
        (status = 200, description = "Trusted: the daemon read the server's key again, found the one with this fingerprint, and added it to the first `UserKnownHostsFile` `ssh -G` names (hashed when `HashKnownHosts` is set), under the name OpenSSH looks it up by. `already_trusted` when that file already held exactly this key and nothing else, so nothing was written. A chat waiting on the host-key card sees the key and goes on", body = HostKeyTrusted),
        (status = 400, description = "`crew_request_invalid`: a body that is not this route's, a host, port or login that is not a plain name, or a fingerprint that is not `SHA256:` and its digest", body = CrewError),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 409, description = "`crew_ssh_host_key_changed`: the known hosts files already hold a key for this server (another key, a revoked one or a certificate authority: Crew never replaces one), or the key the server gives now does not have this fingerprint. Nothing was written. `crew_request_refused`: the SSH settings reach the server another way (a saved Crew connection's jump host included), could not be read, or name no known hosts file Crew can write; or a known hosts file could not be read, so a key already on file could not be ruled out", body = CrewError),
        (status = 502, description = "`crew_ssh_unreachable`: the server gave this computer no host key", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn trust_key(
    headers: HeaderMap,
    CrewJson(body): CrewJson<Value>,
) -> Result<Json<HostKeyTrusted>, HostKeyRouteError> {
    person(&headers)?;
    let request: HostKeyTrustRequest = read_body(body)?;
    let target = HostKeyTarget {
        host: request.host,
        port: request.port,
        login: request.login,
    };
    Ok(Json(host_keys::trust(target, &request.fingerprint).await?))
}

pub fn routes() -> Router {
    Router::new()
        .route("/crew/host-keys/read", post(read_key))
        .route("/crew/host-keys/trust", post(trust_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn answer(refusal: HostKeyRouteError) -> (StatusCode, Value) {
        let response = refusal.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("refusal body");
        (status, serde_json::from_slice(&bytes).expect("JSON"))
    }

    /// A typed refusal keeps its code and sentence, with the status its code takes; anything
    /// else says a fixed sentence, never the error's own words.
    #[tokio::test]
    async fn a_refusal_keeps_its_code_and_says_nothing_else() {
        for (code, status) in [
            (host_keys::CHANGED, StatusCode::CONFLICT),
            (host_keys::UNREACHABLE, StatusCode::BAD_GATEWAY),
            (host_keys::REFUSED, StatusCode::CONFLICT),
            (host_keys::INVALID, StatusCode::BAD_REQUEST),
        ] {
            let error = anyhow::Error::new(HostKeyRefused {
                code,
                message: "Said.".into(),
            });
            let (got, body) = answer(HostKeyRouteError::from(error)).await;
            assert_eq!(got, status, "{code}");
            assert_eq!(body, json!({"code": code, "error": "Said."}));
        }
        let (got, body) = answer(HostKeyRouteError::from(anyhow::anyhow!(
            "/home/you/.ssh/known_hosts: os error 13"
        )))
        .await;
        assert_eq!(got, StatusCode::CONFLICT);
        assert_eq!(body["code"], REQUEST_REFUSED_CODE);
        assert!(!body["error"].as_str().unwrap().contains("os error"));
    }

    #[test]
    fn a_body_the_route_does_not_take_is_refused() {
        for body in [
            json!({"host": "h", "port": 22, "fingerprint": "SHA256:x", "key": "ssh-ed25519 AAAA"}),
            json!({"host": "h", "fingerprint": "SHA256:x"}),
            json!({"host": "h", "port": 70000, "fingerprint": "SHA256:x"}),
        ] {
            assert!(read_body::<HostKeyTrustRequest>(body).is_err());
        }
        assert!(read_body::<HostKeyReadRequest>(json!({"host": "h", "port": 22})).is_ok());
    }
}
