//! Setting Crew up on a server (design OW-9), for the desktop and `biorouter crew setup`.
//!
//! - `GET /crew/setup/artifact`: the Crew server program this daemon would install.
//! - `POST /crew/setup`: start a setup job (check, install, host or member) for a saved
//!   connection or this computer's pending host setup.
//! - `GET` and `DELETE /crew/setup/{job_id}`: where a job stands, and stopping it.
//! - `POST /crew/setup/{job_id}/authentication`: the sign-in a job waits for, whose terminal is
//!   the ordinary `GET /crew/authentication/{id}/terminal`.
//! - `POST /crew/connections/{id}/context/draft`: the server's facts, and the workspace
//!   context document drafted from them.
//!
//! Every route asks for proof that a person acted, with the codes and words of
//! `routes::crew`'s person gate. Nothing a request carries becomes command text: the job runs
//! `biorouter::crew`'s fixed scripts only (`crew/setup.rs`).
use axum::{
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use biorouter::crew::authentication::AuthenticationSession;
use biorouter::crew::{
    cancel_setup, manager, prepare_setup_sign_in, setup_artifact, setup_status, ContextDraft,
    CrewRefusal, SetupArtifact, SetupRefused, SetupRequest, SetupStatus,
};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use serde_json::{json, Value};

use super::crew::wire::{CrewCancelled, CrewJson};
use super::crew_authentication::{
    no_human_authority, HUMAN_AUTHORITY_UNAVAILABLE_CODE, REQUEST_INVALID_CODE,
    REQUEST_REFUSED_CODE, USER_ACTION_REQUIRED_CODE,
};

/// A setup route's refusal: `{code, error}`, as every Crew refusal is.
pub struct SetupRouteError {
    status: StatusCode,
    code: String,
    error: String,
}

impl SetupRouteError {
    fn new(status: StatusCode, code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            error: error.into(),
        }
    }
}

impl IntoResponse for SetupRouteError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code": self.code, "error": self.error})),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for SetupRouteError {
    /// A refusal setup typed keeps its status, code and sentence, as does one the Crew core
    /// typed; anything else is `crew_request_refused` with the error's text.
    fn from(error: anyhow::Error) -> Self {
        if let Some(refused) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<SetupRefused>())
        {
            let status = StatusCode::from_u16(refused.status).unwrap_or(StatusCode::BAD_REQUEST);
            return Self::new(status, refused.code, refused.message.clone());
        }
        if let Some(refused) = CrewRefusal::find(&error) {
            let status =
                StatusCode::from_u16(refused.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
            return Self::new(status, refused.code(), refused.message());
        }
        Self::new(
            StatusCode::BAD_REQUEST,
            REQUEST_REFUSED_CODE,
            error.to_string(),
        )
    }
}

type SetupResult<T> = Result<Json<T>, SetupRouteError>;

/// Proof that a person asked, with the person gate's codes and words.
fn person(headers: &HeaderMap) -> Result<(), SetupRouteError> {
    match user_action_proof(headers) {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(SetupRouteError::new(
            StatusCode::FORBIDDEN,
            USER_ACTION_REQUIRED_CODE,
            "Only a person using the Biorouter desktop app or the biorouter crew command can do this. Agent tools use their separate task grant.",
        )),
        UserActionProof::NoKeyInstalled => Err(SetupRouteError::new(
            StatusCode::FORBIDDEN,
            HUMAN_AUTHORITY_UNAVAILABLE_CODE,
            no_human_authority("This daemon cannot verify human Crew actions. Start it from the Biorouter desktop app or with biorouter crew daemon start."),
        )),
    }
}

/// Read `body` as a `T`, refused as a request the route does not take.
fn read<T: serde::de::DeserializeOwned>(body: Value, what: &str) -> Result<T, SetupRouteError> {
    serde_json::from_value(body)
        .map_err(|_| SetupRouteError::new(StatusCode::BAD_REQUEST, REQUEST_INVALID_CODE, what))
}

fn job_gone() -> SetupRouteError {
    SetupRouteError::new(
        StatusCode::NOT_FOUND,
        "crew_setup_not_found",
        "That setup is not on this computer any more. Start again.",
    )
}

/// Open a setup job's sign-in, as a connection's is opened.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupSignIn {
    request_id: String,
    controller_id: String,
    cols: u16,
    rows: u16,
}

/// The Crew server program this daemon installs.
#[utoipa::path(
    get,
    operation_id = "crew_setup_artifact",
    path = "/crew/setup/artifact",
    responses(
        (status = 200, description = "The program, checked against its manifest and this daemon's version: `source` is `bundled` (part of this app) or `dev` (from `BIOROUTER_CREW_BROKER_DIR`)", body = SetupArtifact),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 409, description = "`crew_install_artifact_unavailable`: this build has no Crew server program, or the one it has does not match its manifest or this daemon's version", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn artifact(headers: HeaderMap) -> SetupResult<SetupArtifact> {
    person(&headers)?;
    Ok(Json(setup_artifact()?))
}

/// Set Crew up on a server (OW-9).
///
/// Check the server, sign in when it asks for a password or a code, install this app's Crew
/// program in the account, then start the workspace (`host`) or connect (`member`). The body
/// names no command: every step is a fixed script.
#[utoipa::path(
    post,
    operation_id = "crew_setup_start",
    path = "/crew/setup",
    request_body = SetupRequest,
    responses(
        (status = 200, description = "The job, started (or the one already under way for the same server and intent): poll `GET /crew/setup/{job_id}`", body = SetupStatus),
        (status = 400, description = "`crew_request_invalid`: a body that is not JSON, a field the route does not take, a workspace name, login or route outside what a connection allows, a `host` job without a pending host setup and a name, or a `member` job without a saved connection; `crew_request_refused` for an SSH configuration the preflight refuses", body = CrewError),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 404, description = "`crew_connection_not_found`: no saved connection has that ID", body = CrewError),
        (status = 409, description = "`crew_host_setup_unknown`: no pending host setup with that ID on this computer; `crew_host_setup_used`: it already has a saved connection; `crew_setup_busy`: another kind of job is under way for this server, or too many jobs at once", body = CrewError),
        (status = 413, description = "`crew_request_invalid`: the body is larger than the route takes", body = CrewError),
        (status = 415, description = "`crew_request_invalid`: the body is not sent as `application/json`", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn start(
    headers: HeaderMap,
    CrewJson(body): CrewJson<Value>,
) -> SetupResult<SetupStatus> {
    person(&headers)?;
    let request: SetupRequest = read(
        body,
        "Send the server to set up, what to do there, and nothing else.",
    )?;
    Ok(Json(manager()?.start_setup(request).await?))
}

/// Where a setup job stands.
#[utoipa::path(
    get,
    operation_id = "crew_setup_status",
    path = "/crew/setup/{job_id}",
    params(("job_id" = String, Path, description = "The job `POST /crew/setup` answered")),
    responses(
        (status = 200, description = "`state` is `running`, `needs_sign_in` (open `POST /crew/setup/{job_id}/authentication`), `finished` (`outcome`), `failed` or `cancelled` (`error`: a typed code and a sentence, such as `crew_setup_cancelled`, `crew_setup_timed_out`, `crew_setup_sign_in_expired`, `crew_setup_start_failed`, `crew_install_checksum_mismatch` or an SSH code)", body = SetupStatus),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 404, description = "`crew_setup_not_found`: no such job on this computer", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn status(headers: HeaderMap, Path(job_id): Path<String>) -> SetupResult<SetupStatus> {
    person(&headers)?;
    setup_status(&job_id).map(Json).ok_or_else(job_gone)
}

/// Stop a setup job.
#[utoipa::path(
    delete,
    operation_id = "crew_setup_cancel",
    path = "/crew/setup/{job_id}",
    params(("job_id" = String, Path, description = "The job to stop")),
    responses(
        (status = 200, description = "Stopped; stopping a finished job changes nothing", body = CrewCancelled),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 404, description = "`crew_setup_not_found`: no such job on this computer", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn cancel(headers: HeaderMap, Path(job_id): Path<String>) -> SetupResult<CrewCancelled> {
    person(&headers)?;
    if cancel_setup(&job_id) {
        Ok(Json(CrewCancelled { cancelled: true }))
    } else {
        Err(job_gone())
    }
}

/// Open the sign-in a setup job waits for. Its terminal is `GET /crew/authentication/{id}/terminal`.
#[utoipa::path(
    post,
    operation_id = "crew_setup_authentication_prepare",
    path = "/crew/setup/{job_id}/authentication",
    params(("job_id" = String, Path, description = "The job waiting for a sign-in")),
    request_body = SetupSignIn,
    responses(
        (status = 200, description = "The sign-in, ready for its terminal. Once its master is up the job carries on over it, and so do the start, Create and the connect", body = AuthenticationSession),
        (status = 400, description = "`crew_request_invalid` for a body that is not JSON or not this route's; `crew_request_refused` for a sign-in already open for this server under another controller, or one that could not start", body = CrewError),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 404, description = "`crew_setup_not_found`: no such job on this computer", body = CrewError),
        (status = 409, description = "`crew_setup_not_waiting_for_sign_in`: the job is not at `needs_sign_in`", body = CrewError),
        (status = 413, description = "`crew_request_invalid`: the body is larger than the route takes", body = CrewError),
        (status = 415, description = "`crew_request_invalid`: the body is not sent as `application/json`", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn authentication(
    headers: HeaderMap,
    Path(job_id): Path<String>,
    CrewJson(body): CrewJson<Value>,
) -> SetupResult<AuthenticationSession> {
    person(&headers)?;
    let body: SetupSignIn = read(
        body,
        "Send the sign-in's request ID, controller ID and terminal size, and nothing else.",
    )?;
    Ok(Json(
        prepare_setup_sign_in(
            &job_id,
            &body.request_id,
            &body.controller_id,
            body.cols,
            body.rows,
        )
        .await?,
    ))
}

/// The server's facts, and the workspace context document drafted from them (OW-8).
///
/// Runs `biorouter-crew probe` in the connection's account over its own control path, never
/// prompting; the draft is for a person to read and save, and nothing is saved here.
#[utoipa::path(
    post,
    operation_id = "crew_context_draft",
    path = "/crew/connections/{id}/context/draft",
    params(("id" = String, Path, description = "The saved connection")),
    responses(
        (status = 200, description = "`draft` (Markdown) and the `facts` it was drafted from", body = ContextDraft),
        (status = 400, description = "An SSH failure's code (`crew_ssh_auth_required`: sign in first; `crew_ssh_key_refused`, `crew_ssh_host_key_unknown`, `crew_ssh_host_key_changed`, `crew_ssh_unreachable`, `crew_ssh_failed`), or `crew_request_refused` for an SSH configuration the preflight refuses", body = CrewError),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError),
        (status = 404, description = "`crew_connection_not_found`: no saved connection has that ID", body = CrewError),
        (status = 409, description = "`crew_server_probe_unsupported`: the Crew program in the account is too old to describe the server (set Crew up again); `crew_setup_server_unreadable`: its answer could not be read; `crew_setup_timed_out`: it did not answer in time", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn context_draft(
    headers: HeaderMap,
    Path(id): Path<String>,
) -> SetupResult<ContextDraft> {
    person(&headers)?;
    Ok(Json(manager()?.context_draft(&id).await?))
}

pub fn routes() -> Router {
    Router::new()
        .route("/crew/setup", post(start))
        .route("/crew/setup/artifact", get(artifact))
        .route("/crew/setup/{job_id}", get(status).delete(cancel))
        .route("/crew/setup/{job_id}/authentication", post(authentication))
        .route("/crew/connections/{id}/context/draft", post(context_draft))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn answer(refusal: SetupRouteError) -> (StatusCode, Value) {
        let response = refusal.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("refusal body");
        (status, serde_json::from_slice(&bytes).expect("JSON"))
    }

    async fn proofless(refusal: SetupRouteError) {
        let (status, body) = answer(refusal).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            matches!(
                body["code"].as_str(),
                Some("crew_user_action_required" | "crew_human_authority_unavailable")
            ),
            "{body}"
        );
    }

    /// Every setup door needs proof that a person asked, before it reads the body, looks up a
    /// job, reads the program or reaches a server.
    #[tokio::test]
    async fn setup_needs_a_person_at_every_door() {
        let none = HeaderMap::new;
        let job = || Path("job".to_owned());
        match artifact(none()).await {
            Err(refusal) => proofless(refusal).await,
            Ok(_) => panic!("read the program without proof of a person"),
        }
        let request =
            json!({"target": {"kind": "connection", "connection_id": "c"}, "intent": "member"});
        match start(none(), CrewJson(request)).await {
            Err(refusal) => proofless(refusal).await,
            Ok(_) => panic!("started setup without proof of a person"),
        }
        match status(none(), job()).await {
            Err(refusal) => proofless(refusal).await,
            Ok(_) => panic!("read a job without proof of a person"),
        }
        match cancel(none(), job()).await {
            Err(refusal) => proofless(refusal).await,
            Ok(_) => panic!("stopped a job without proof of a person"),
        }
        let sign_in = json!({"request_id": "r", "controller_id": "c", "cols": 80, "rows": 24});
        match authentication(none(), job(), CrewJson(sign_in)).await {
            Err(refusal) => proofless(refusal).await,
            Ok(_) => panic!("opened a sign-in without proof of a person"),
        }
        match context_draft(none(), Path("c".to_owned())).await {
            Err(refusal) => proofless(refusal).await,
            Ok(_) => panic!("reached a server without proof of a person"),
        }
    }

    #[tokio::test]
    async fn a_setup_refusal_keeps_its_status_and_code() {
        let refused = anyhow::Error::new(SetupRefused {
            status: 409,
            code: "crew_setup_busy",
            message: "Crew is already being set up on this server from this computer. Wait for it to finish.".into(),
        })
        .context("while starting");
        let (status, body) = answer(refused.into()).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "crew_setup_busy");
        assert_eq!(body.as_object().unwrap().len(), 2, "{body}");
        let (status, body) = answer(anyhow::anyhow!("preflight").into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "crew_request_refused");
        let (status, body) = answer(job_gone()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "crew_setup_not_found");
        // A body the route does not take is refused before anything else is read.
        let refused = read::<SetupRequest>(json!({"intent": "member", "command": "id"}), "x")
            .err()
            .unwrap();
        let (status, body) = answer(refused).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "crew_request_invalid");
    }
}
