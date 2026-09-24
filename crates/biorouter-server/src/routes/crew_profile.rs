use super::crew::{cancel_owned_session, owned_run_views, OwnedCancellation};
use crate::state::AppState;
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use biorouter::crew::{manager, CrewManager, RevocationUnconfirmed, RevokeOutcome};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

static SECRET_OPERATIONS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));

/// The code of every refusal that has no more specific one.
pub const REFUSED: &str = "crew_profile_refused";
/// A request that did not prove a person sent it.
pub const USER_ACTION_REQUIRED: &str = "crew_user_action_required";
/// A daemon holding no user-action key, so no request can prove a person sent it.
pub const HUMAN_AUTHORITY_UNAVAILABLE: &str = "crew_human_authority_unavailable";
/// The session holds no Crew grant on this device.
pub const GRANT_NOT_FOUND: &str = "crew_grant_not_found";
/// The session's grant belongs to a different saved connection than the one the path names.
pub const GRANT_OTHER_CONNECTION: &str = "crew_grant_other_connection";
/// The session was granted again while its previous grant was being revoked; the new grant
/// is live.
pub const GRANT_REPLACED: &str = "crew_grant_replaced";
/// The grant stopped on this device, but the stop could not be saved, so a restart would
/// honor it again.
pub const REVOCATION_NOT_SAVED: &str = "crew_revocation_not_saved";
/// The grant stopped on this device and the stop is saved; the workspace has not confirmed
/// `run.revoke`. The same code the task cancel route answers with.
pub const REVOCATION_UNCONFIRMED: &str = "crew_revocation_unconfirmed";
/// The text of a [`REVOCATION_UNCONFIRMED`] refusal (RV-D1).
pub const REVOCATION_UNCONFIRMED_MESSAGE: &str =
    "Stopped on this device. The workspace has not confirmed yet; reconnect and retry.";

/// A refusal: `{code, error}` plus any fields a route adds beside them (`detail`, …).
pub struct Refusal {
    status: StatusCode,
    code: &'static str,
    error: String,
    fields: Map<String, Value>,
}
impl Refusal {
    fn new(status: StatusCode, code: &'static str, error: impl Into<String>) -> Self {
        Self {
            status,
            code,
            error: error.into(),
            fields: Map::new(),
        }
    }
    /// Add a field beside `code` and `error`, which it can never replace.
    fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        if key != "code" && key != "error" {
            self.fields.insert(key.into(), value.into());
        }
        self
    }
}
impl From<anyhow::Error> for Refusal {
    fn from(error: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_REQUEST, REFUSED, error.to_string())
    }
}
impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let mut body = self.fields;
        body.insert("code".into(), Value::String(self.code.into()));
        body.insert("error".into(), Value::String(self.error));
        (self.status, Json(Value::Object(body))).into_response()
    }
}
type Result = std::result::Result<Json<Value>, Refusal>;
/// Every route here acts for the person, so every one needs their proof. The daemon secret,
/// a stated capability and the chat's own tier are none of them.
fn person(headers: &HeaderMap) -> std::result::Result<(), Refusal> {
    match user_action_proof(headers) {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(Refusal::new(
            StatusCode::FORBIDDEN,
            USER_ACTION_REQUIRED,
            "Crew profile operations require the existing human approval secret. Authorize this action in the Crew panel or the native Crew CLI.",
        )),
        UserActionProof::NoKeyInstalled => Err(Refusal::new(
            StatusCode::FORBIDDEN,
            HUMAN_AUTHORITY_UNAVAILABLE,
            "This daemon cannot verify the human approval secret that Crew profile operations require. Start the trusted desktop launcher or biorouter crew daemon start with your separately held approval secret.",
        )),
    }
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

/// The chats and tasks holding a grant on this connection (RV-D2). Each row adds, to what
/// the Crew registry records:
///
/// - `kind`: `task` when the grant is one of this device's ledger tasks (the ledger holds its
///   session and its current run), else `chat`; `null` when the ledger could not be read, so
///   the list still answers while the task ledger is unavailable.
/// - `session_name`: the conversation's title, or `null` when the conversation is gone or
///   untitled.
/// - `expires_at`: when the workspace ends the grant on its own, from grant time; `null` for
///   a grant recorded before that was kept.
///
/// All three are display only: revoking still decides on the registry and the ledger.
#[utoipa::path(get, operation_id = "crew_profile_grants",path="/crew/connections/{id}/grants",params(("id"=String,Path,description="Crew connection ID")),responses((status=200,body=Value)),tag="Crew")]
pub async fn grants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result {
    person(&headers)?;
    let mut listed = manager()?.session_grants(&id).await?;
    let tasks: Option<HashSet<(String, String)>> = match owned_run_views(&id).await {
        Ok(views) => Some(
            views
                .into_iter()
                .map(|view| (view.session_id, view.run_id))
                .collect(),
        ),
        Err(error) => {
            tracing::warn!("Crew grants listed without task kinds: {error}");
            None
        }
    };
    if let Some(rows) = listed.get_mut("grants").and_then(Value::as_array_mut) {
        for row in rows.iter_mut() {
            let Some(fields) = row.as_object_mut() else {
                continue;
            };
            let session = text_field(fields, "session_id");
            let run = text_field(fields, "run_id");
            let kind = tasks.as_ref().map(|tasks| {
                if tasks.contains(&(session.clone(), run)) {
                    "task"
                } else {
                    "chat"
                }
            });
            fields.insert("kind".into(), json!(kind));
            fields.insert(
                "session_name".into(),
                json!(session_name(&state, &session).await),
            );
            fields.entry("expires_at").or_insert(Value::Null);
        }
    }
    Ok(Json(listed))
}
fn text_field(fields: &Map<String, Value>, key: &str) -> String {
    fields
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
/// The conversation's title, or `None` when it cannot be read or has none.
async fn session_name(state: &AppState, session: &str) -> Option<String> {
    if session.is_empty() {
        return None;
    }
    state
        .session_manager()
        .get_session(session, false)
        .await
        .ok()
        .map(|found| found.name)
        .filter(|name| !name.trim().is_empty())
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

/// Revoke a chat's or task's grant (RV-D1, RV-D3). The grant stops on this device first,
/// and is saved, before the workspace is asked; so:
///
/// - 200 `{revoked: true, session_id, run_id, remote_revocation_confirmed: true, run}`: stopped
///   here and confirmed by the workspace. `run` is the workspace's revoked run.
/// - 503 [`REVOCATION_UNCONFIRMED`]: stopped here and saved, not yet confirmed. Retrying asks
///   the workspace again.
/// - 404 [`GRANT_NOT_FOUND`], 409 [`GRANT_OTHER_CONNECTION`] or [`GRANT_REPLACED`], 500
///   [`REVOCATION_NOT_SAVED`]: not revoked, or not durably.
///
/// A ledger task's session is stopped through the task cancel path, so its ledger entry reads
/// `cancelled` or `cancellation_unconfirmed` rather than a stale `running`; its answer adds
/// `task_status`. The request body, if any, is ignored.
#[utoipa::path(post, operation_id = "crew_profile_revoke",path="/crew/connections/{id}/sessions/{session}/revoke",params(("id"=String,Path,description="Crew connection ID"),("session"=String,Path,description="Session ID")),responses((status=200,body=Value)),tag="Crew")]
pub async fn revoke(headers: HeaderMap, Path((id, session)): Path<(String, String)>) -> Result {
    person(&headers)?;
    let crew = manager()?;
    let Some(metadata) = crew.run_metadata(&session).await else {
        return Err(grant_not_found());
    };
    if metadata.connection_id != id {
        return Err(Refusal::new(
            StatusCode::CONFLICT,
            GRANT_OTHER_CONNECTION,
            "This session's Crew grant belongs to a different Crew connection.",
        ));
    }
    let finished_task = match cancel_owned_session(&id, &session).await {
        Ok(Some(OwnedCancellation::Requested {
            run_id,
            session_id,
            status,
            revocation,
            persistence_error,
        })) => {
            return task_revoked(
                &crew,
                session_id,
                run_id,
                status,
                revocation,
                persistence_error,
            )
            .await;
        }
        // A finished task's grant stays live until it expires, so it is revoked below like
        // a chat's.
        Ok(Some(OwnedCancellation::AlreadyFinished(view))) => Some(view.status),
        Ok(None) => None,
        // Nothing was done: the ledger could not be read, or no longer lists the task. The
        // grant still has to stop, so it is revoked below.
        Err(error) => {
            tracing::warn!("Crew revoke is not recording a task outcome: {error}");
            None
        }
    };
    let outcome = match crew
        .revoke_session_if_current(&session, &metadata.run_id)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => return Err(not_revoked(&crew, &session, &metadata.run_id, error).await),
    };
    let answer = chat_revoked(session, metadata.run_id, outcome);
    match finished_task {
        Some(status) => answer.map(|Json(mut body)| {
            body["task_status"] = json!(status);
            Json(body)
        }),
        None => answer,
    }
}
fn grant_not_found() -> Refusal {
    Refusal::new(
        StatusCode::NOT_FOUND,
        GRANT_NOT_FOUND,
        "No Crew grant for this session.",
    )
}
fn revoked(session_id: String, run_id: String, run: Value) -> Value {
    json!({"revoked":true,"session_id":session_id,"run_id":run_id,"remote_revocation_confirmed":true,"run":run})
}
fn unconfirmed(session_id: String, run_id: String, remote_error: &anyhow::Error) -> Refusal {
    Refusal::new(
        StatusCode::SERVICE_UNAVAILABLE,
        REVOCATION_UNCONFIRMED,
        REVOCATION_UNCONFIRMED_MESSAGE,
    )
    .with("detail", remote_error.to_string())
    .with("session_id", session_id)
    .with("run_id", run_id)
    .with("stopped_on_this_device", true)
    .with("remote_revocation_confirmed", false)
}
fn chat_revoked(session_id: String, run_id: String, outcome: RevokeOutcome) -> Result {
    match outcome {
        RevokeOutcome {
            remote_confirmed: true,
            run: Some(run),
            ..
        } => Ok(Json(revoked(session_id, run_id, run))),
        RevokeOutcome { remote_error, .. } => {
            let error = remote_error
                .unwrap_or_else(|| anyhow::anyhow!("The workspace did not confirm the revocation"));
            Err(unconfirmed(session_id, run_id, &error))
        }
    }
}
/// The task cancel path's outcome as a revoke answer. `revocation` is the workspace's
/// revoked run, a [`RevocationUnconfirmed`] when the grant stopped here only, or any other
/// error when it did not stop here.
async fn task_revoked(
    crew: &CrewManager,
    session_id: String,
    run_id: String,
    status: String,
    revocation: anyhow::Result<Value>,
    persistence_error: Option<String>,
) -> Result {
    let task_fields = |refusal: Refusal| {
        let refusal = refusal.with("task_status", status.clone());
        match &persistence_error {
            Some(error) => refusal.with("task_status_error", error.clone()),
            None => refusal,
        }
    };
    match revocation {
        Ok(run) => {
            let mut body = revoked(session_id, run_id, run);
            body["task_status"] = json!(status);
            if let Some(error) = &persistence_error {
                body["task_status_error"] = json!(error);
            }
            Ok(Json(body))
        }
        Err(error) => match error.downcast_ref::<RevocationUnconfirmed>() {
            Some(unconfirmed_here) => Err(task_fields(unconfirmed(
                session_id,
                run_id,
                unconfirmed_here.remote_error(),
            ))),
            None => Err(task_fields(
                not_revoked(crew, &session_id, &run_id, error).await,
            )),
        },
    }
}
/// Why a revocation did not stop the grant durably on this device. The registry is read again
/// to tell a grant that vanished or was replaced meanwhile from a stop that could not be saved.
async fn not_revoked(
    crew: &CrewManager,
    session: &str,
    expected_run_id: &str,
    error: anyhow::Error,
) -> Refusal {
    match crew.run_metadata(session).await {
        None => grant_not_found(),
        Some(current) if current.run_id != expected_run_id => Refusal::new(
            StatusCode::CONFLICT,
            GRANT_REPLACED,
            "This session was granted Crew access again while its previous grant was being revoked. The new grant is active; revoke again to stop it.",
        ),
        Some(_) => Refusal::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            REVOCATION_NOT_SAVED,
            error.to_string(),
        ),
    }
}
pub fn routes(state: Arc<AppState>) -> Router {
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
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whichever way a request fails to prove a person sent it, the refusal is a 403 with its
    /// own code, never the generic one. The lib's tests share one process-global digest, so
    /// this binary may or may not hold a key; both arms are checked by what they answer.
    #[test]
    fn credential_operations_refuse_without_human_proof() {
        for headers in [HeaderMap::new(), {
            let mut wrong = HeaderMap::new();
            wrong.insert("X-User-Action", "not-the-approval-secret".parse().unwrap());
            wrong
        }] {
            let refusal = person(&headers).unwrap_err();
            assert_eq!(refusal.status, StatusCode::FORBIDDEN);
            assert!(
                [USER_ACTION_REQUIRED, HUMAN_AUTHORITY_UNAVAILABLE].contains(&refusal.code),
                "an unproven request was refused with {}",
                refusal.code
            );
            assert!(refusal.error.contains("human approval secret"));
        }
    }

    #[tokio::test]
    async fn a_refusal_keeps_its_code_and_error_over_added_fields() {
        let response = Refusal::new(
            StatusCode::SERVICE_UNAVAILABLE,
            REVOCATION_UNCONFIRMED,
            REVOCATION_UNCONFIRMED_MESSAGE,
        )
        .with("code", "forged")
        .with("error", "forged")
        .with("detail", "Crew connection is disconnected")
        .into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            body,
            json!({"code":REVOCATION_UNCONFIRMED,"error":REVOCATION_UNCONFIRMED_MESSAGE,"detail":"Crew connection is disconnected"})
        );
    }

    #[test]
    fn a_confirmed_revoke_answers_revoked_with_the_run() {
        let Json(body) = chat_revoked(
            "session-1".into(),
            "run-1".into(),
            RevokeOutcome {
                remote_confirmed: true,
                run: Some(json!({"id":"run-1","revoked":true})),
                remote_error: None,
            },
        )
        .unwrap_or_else(|_| panic!("a confirmed revoke was refused"));
        assert_eq!(
            body,
            json!({"revoked":true,"session_id":"session-1","run_id":"run-1","remote_revocation_confirmed":true,"run":{"id":"run-1","revoked":true}})
        );
    }

    /// Anything short of the workspace's confirmation is a 503, never a success: a missing
    /// run with no error included.
    #[test]
    fn an_unconfirmed_revoke_is_a_503_with_the_workspace_error() {
        for (outcome, detail) in [
            (
                RevokeOutcome {
                    remote_confirmed: false,
                    run: None,
                    remote_error: Some(anyhow::anyhow!(
                        "Crew connection is disconnected; authenticate and connect in Crew"
                    )),
                },
                "Crew connection is disconnected; authenticate and connect in Crew",
            ),
            (
                RevokeOutcome {
                    remote_confirmed: true,
                    run: None,
                    remote_error: None,
                },
                "The workspace did not confirm the revocation",
            ),
        ] {
            let Err(refusal) = chat_revoked("session-1".into(), "run-1".into(), outcome) else {
                panic!("an unconfirmed revoke answered success");
            };
            assert_eq!(refusal.status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(refusal.code, REVOCATION_UNCONFIRMED);
            assert_eq!(refusal.error, REVOCATION_UNCONFIRMED_MESSAGE);
            assert_eq!(refusal.fields["detail"], json!(detail));
            assert_eq!(refusal.fields["stopped_on_this_device"], json!(true));
            assert_eq!(refusal.fields["remote_revocation_confirmed"], json!(false));
        }
    }

    /// A Crew registry of its own, holding one grant: `session-1` on `run-current`. Nothing
    /// here reads a credential or opens a connection.
    fn registry_with_one_grant() -> (tempfile::TempDir, CrewManager) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("connections.json"),
            serde_json::to_vec(&json!({
                "connections": [],
                "scopes": {"session-1": {
                    "connection_id": "conn-1", "run_id": "run-current",
                    "channel_id": "chan-1", "source_channels": [], "epoch": 1,
                    "provider_binding": "versa_azure", "public_provider": false,
                    "expired": true,
                }},
            }))
            .unwrap(),
        )
        .unwrap();
        let crew = CrewManager::new(root.path().to_path_buf()).unwrap();
        (root, crew)
    }

    /// A revoke that did not stop the grant durably names why, from the registry as it is
    /// now: the grant vanished, was granted again meanwhile, or the stop could not be saved.
    /// None of them reads as revoked.
    #[tokio::test]
    async fn a_revoke_that_did_not_land_says_why() {
        let (_root, crew) = registry_with_one_grant();
        let error =
            || anyhow::anyhow!("Couldn't save the revocation on this device; retry to finish it");

        let gone = not_revoked(&crew, "session-gone", "run-current", error()).await;
        assert_eq!(
            (gone.status, gone.code),
            (StatusCode::NOT_FOUND, GRANT_NOT_FOUND)
        );

        let replaced = not_revoked(&crew, "session-1", "run-previous", error()).await;
        assert_eq!(
            (replaced.status, replaced.code),
            (StatusCode::CONFLICT, GRANT_REPLACED)
        );

        let unsaved = not_revoked(&crew, "session-1", "run-current", error()).await;
        assert_eq!(
            (unsaved.status, unsaved.code),
            (StatusCode::INTERNAL_SERVER_ERROR, REVOCATION_NOT_SAVED)
        );
        assert_eq!(
            unsaved.error,
            "Couldn't save the revocation on this device; retry to finish it"
        );
    }

    /// RV-D3's answers: the task cancel path's outcome keeps the revoke contract, and adds
    /// the ledger's `task_status` (and its write error, when the ledger could not be saved).
    #[tokio::test]
    async fn a_task_revoke_keeps_the_revoke_contract_and_adds_the_task_status() {
        let (_root, crew) = registry_with_one_grant();

        let Json(confirmed) = task_revoked(
            &crew,
            "session-1".into(),
            "run-current".into(),
            "cancelled".into(),
            Ok(json!({"id":"run-current","revoked":true})),
            None,
        )
        .await
        .unwrap_or_else(|_| panic!("a confirmed task revoke was refused"));
        assert_eq!(confirmed["revoked"], json!(true));
        assert_eq!(confirmed["remote_revocation_confirmed"], json!(true));
        assert_eq!(confirmed["task_status"], "cancelled");
        assert_eq!(confirmed["run"]["revoked"], json!(true));
        assert!(confirmed.get("task_status_error").is_none());

        let unconfirmed_here = RevokeOutcome {
            remote_confirmed: false,
            run: None,
            remote_error: Some(anyhow::anyhow!("Crew connection is disconnected")),
        }
        .into_confirmed();
        let Err(refusal) = task_revoked(
            &crew,
            "session-1".into(),
            "run-current".into(),
            "cancellation_unconfirmed".into(),
            unconfirmed_here,
            Some("ledger write failed".into()),
        )
        .await
        else {
            panic!("an unconfirmed task revoke answered success");
        };
        assert_eq!(
            (refusal.status, refusal.code),
            (StatusCode::SERVICE_UNAVAILABLE, REVOCATION_UNCONFIRMED)
        );
        assert_eq!(
            refusal.fields["detail"],
            json!("Crew connection is disconnected")
        );
        assert_eq!(
            refusal.fields["task_status"],
            json!("cancellation_unconfirmed")
        );
        assert_eq!(
            refusal.fields["task_status_error"],
            json!("ledger write failed")
        );

        let Err(refusal) = task_revoked(
            &crew,
            "session-1".into(),
            "run-previous".into(),
            "cancellation_unconfirmed".into(),
            Err(anyhow::anyhow!("replaced")),
            None,
        )
        .await
        else {
            panic!("a task revoke that stopped nothing answered success");
        };
        assert_eq!(
            (refusal.status, refusal.code),
            (StatusCode::CONFLICT, GRANT_REPLACED)
        );
        assert_eq!(
            refusal.fields["task_status"],
            json!("cancellation_unconfirmed")
        );
    }
}
