//! Crew authentication and workspace admission.
//!
//! **SSH authentication** (`/crew/connections/{id}/authentication`, `/crew/authentication/…`):
//! native clients only; terminal credentials never enter model or replay streams.
//!
//! **Workspace admission (S3a)**, as `docs/research/biorouter-crew/naming-design.md` ("Joining
//! a workspace (S3a)", "Daemon routes, OpenAPI and TypeScript client") specifies:
//!
//! - `POST /crew/connections/from-invitation`: preview a pasted invitation, or save the
//!   connection it pins.
//! - `GET /crew/connections/{id}/invitation`: the invitation message a host sends.
//! - `GET /crew/connections/{id}/join`: where this computer stands in joining, with the device
//!   code this daemon computed itself.
//! - `POST /crew/connections/{id}/join`: claim the join once the host approved the code.
//!
//! Every admission route asks for proof that a person acted, with the codes and words of
//! `routes::crew`'s person gate. They name a saved connection, never a chat, so no chat reach
//! decision applies. Nothing here authorizes anything on the workspace: the broker decides
//! every join, and the code a joiner shows never comes from the broker's answer.
use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use biorouter::crew::authentication::{
    self, HandoffFailed, InvitationAdvanced, InvitationOutcome, InvitationOverrides,
    InvitationPreview, InvitationRefusal, InvitationRefused, InvitationText, JoinPerson,
    JoinRefused, JoinState, JoinStatus, TerminalEvent,
};
use biorouter::crew::{
    manager, ClusterMode, Connection, CrewManager, SshFailure, WorkspaceIdentityError,
};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
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
                    Err(error) => {
                        let _ = socket.send(Message::Text(handoff_failure_frame(&error).to_string().into())).await;
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

/// The terminal's `error` frame for a failed handoff.
///
/// A [`HandoffFailed`] (sign-in worked, Crew did not start behind it) answers
/// `crew_handoff_failed` in words for a person, or `crew_workspace_identity_mismatch` when the
/// broker behind the signed-in master is not the pinned workspace, which is a trust problem
/// rather than a missing installation. The diagnostic text stays in the daemon's log, where
/// [`authentication::handoff`] already wrote it. Anything else (the session was replaced or
/// closed) keeps the older code and text.
fn handoff_failure_frame(error: &anyhow::Error) -> Value {
    match error.downcast_ref::<HandoffFailed>() {
        Some(failed) if failed.workspace_identity_mismatch() => json!({
            "type": "error",
            "code": WORKSPACE_IDENTITY_MISMATCH_CODE,
            "error": failed.cause().to_string(),
        }),
        Some(failed) => json!({
            "type": "error",
            "code": failed.api_code(),
            "error": failed.to_string(),
        }),
        None => json!({
            "type": "error",
            "code": authentication::HANDOFF_FAILURE_CODE,
            "error": authentication::HANDOFF_FAILURE_MESSAGE,
        }),
    }
}

// ---------------------------------------------------------------------------------------------
// Workspace admission (S3a)
// ---------------------------------------------------------------------------------------------

/// The code `routes::crew`'s person gate answers when the request carries no proof.
pub const USER_ACTION_REQUIRED_CODE: &str = "crew_user_action_required";
/// The code `routes::crew`'s person gate answers on a daemon that cannot check a proof.
pub const HUMAN_AUTHORITY_UNAVAILABLE_CODE: &str = "crew_human_authority_unavailable";
/// A request whose body or query is not in the shape the route takes.
pub const REQUEST_INVALID_CODE: &str = "crew_request_invalid";
/// The route names a connection this computer has not saved.
pub const CONNECTION_NOT_FOUND_CODE: &str = "crew_connection_not_found";
/// The route needs a live, verified connection and this one is not connected.
pub const NOT_CONNECTED_CODE: &str = "crew_not_connected";
/// A typed `@username` that is not an account name.
pub const INVALID_SELECTOR_CODE: &str = "crew_invalid_selector";
/// What `POST /crew/connections/{id}/connect` answers for the same cause.
pub const WORKSPACE_IDENTITY_MISMATCH_CODE: &str = "crew_workspace_identity_mismatch";
/// Any other refusal. On an admission route its words are a fixed sentence for the step that
/// failed, never the error's own text (see [`core_refusal`]).
pub const REQUEST_REFUSED_CODE: &str = "crew_request_refused";

/// What `POST /crew/connections/from-invitation` answers for a failure the core did not type.
pub const FROM_INVITATION_FAILED: &str = "Biorouter couldn't read or save this invitation. Try again. If it keeps failing, the Biorouter log has the details.";
/// What `GET /crew/connections/{id}/invitation` answers for a failure the core did not type.
pub const INVITATION_FAILED: &str = "Biorouter couldn't build an invitation for this workspace. Reconnect and try again. If it keeps failing, the Biorouter log has the details.";
/// What `GET /crew/connections/{id}/join` answers for a failure the core did not type.
pub const JOIN_STATUS_FAILED: &str = "Biorouter couldn't check this computer's join with the workspace. Try again. If it keeps failing, the Biorouter log has the details.";
/// What `POST /crew/connections/{id}/join` answers for a failure the core did not type.
pub const JOIN_FAILED: &str = "Biorouter couldn't finish joining the workspace. Try again. If it keeps failing, the Biorouter log has the details.";

/// A refusal from an admission route: `{code, error}` and any typed fields beside them. Every
/// refusal carries a code, so a client can tell a refusal from a daemon without the route.
#[derive(Debug)]
pub struct AdmissionRefusal {
    status: StatusCode,
    code: String,
    error: String,
    /// A list, empty and unallocated for most refusals, so the error stays small.
    fields: Vec<(String, Value)>,
}

impl AdmissionRefusal {
    fn new(status: StatusCode, code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            error: error.into(),
            fields: Vec::new(),
        }
    }

    /// Add a field beside `code` and `error`, which it can never replace.
    fn with(mut self, key: &str, value: impl Serialize) -> Self {
        if key != "code" && key != "error" {
            if let Ok(value) = serde_json::to_value(value) {
                self.fields.push((key.into(), value));
            }
        }
        self
    }
}

impl IntoResponse for AdmissionRefusal {
    fn into_response(self) -> Response {
        let mut body: serde_json::Map<String, Value> = self.fields.into_iter().collect();
        body.insert("code".into(), Value::String(self.code));
        body.insert("error".into(), Value::String(self.error));
        (self.status, Json(Value::Object(body))).into_response()
    }
}

/// Proof that a person asked, with the same three answers, codes and words as the person gate
/// in `routes::crew`, so a client handles every Crew route's refusal alike.
fn require_person(headers: &HeaderMap) -> Result<(), AdmissionRefusal> {
    person_refusal(user_action_proof(headers)).map_or(Ok(()), Err)
}

/// The refusal for each answer the proof check can give; `None` for a proven person.
fn person_refusal(proof: UserActionProof) -> Option<AdmissionRefusal> {
    match proof {
        UserActionProof::Proven => None,
        UserActionProof::Unproven => Some(AdmissionRefusal::new(
            StatusCode::FORBIDDEN,
            USER_ACTION_REQUIRED_CODE,
            "Authorize this action in the Crew panel or native Crew CLI with your human approval secret. Agent tools use their separate task grant.",
        )),
        UserActionProof::NoKeyInstalled => Some(AdmissionRefusal::new(
            StatusCode::FORBIDDEN,
            HUMAN_AUTHORITY_UNAVAILABLE_CODE,
            "This daemon cannot verify human Crew actions. Start the trusted desktop launcher or biorouter crew daemon start with your separately held approval secret.",
        )),
    }
}

fn crew() -> Result<Arc<CrewManager>, AdmissionRefusal> {
    manager().map_err(|error| {
        AdmissionRefusal::new(
            StatusCode::BAD_REQUEST,
            REQUEST_REFUSED_CODE,
            error.to_string(),
        )
    })
}

/// A body or query the route could not read. The words say what was wrong in general; the
/// extractor's own diagnostic, which can quote a value the client sent back to it, is kept in
/// `detail` for "Copy details" only.
fn unreadable_request(status: StatusCode, detail: String) -> AdmissionRefusal {
    AdmissionRefusal::new(
        status,
        REQUEST_INVALID_CODE,
        "Biorouter couldn't read this request. Update the app or command that sent it, and try again.",
    )
    .with("detail", detail)
}

/// The saved connection `id` names.
async fn saved_connection(crew: &CrewManager, id: &str) -> Result<Connection, AdmissionRefusal> {
    crew.connection(id).await.map_err(|_| {
        AdmissionRefusal::new(
            StatusCode::NOT_FOUND,
            CONNECTION_NOT_FOUND_CODE,
            "This computer has no saved Crew connection with that ID.",
        )
    })
}

fn not_connected() -> AdmissionRefusal {
    AdmissionRefusal::new(
        StatusCode::CONFLICT,
        NOT_CONNECTED_CODE,
        "Connect to this workspace first, then try again.",
    )
}

/// The routes that talk to the workspace need a live, verified connection.
fn require_connected(connection: &Connection) -> Result<(), AdmissionRefusal> {
    if connection.status == "connected" {
        Ok(())
    } else {
        Err(not_connected())
    }
}

/// A core error as a refusal, classified by its type, never by its words:
///
/// - [`InvitationRefused`]: its own code (`crew_invitation_invalid` with a `reason`,
///   `crew_invitation_conflict` or `crew_connection_exists` with the `connection_id` concerned).
/// - [`JoinRefused`]: its own `crew_join_*` code, `409`. The workspace's own words are
///   unauthenticated, so they go to the log and never into the answer.
/// - [`HandoffFailed`], [`SshFailure`], [`WorkspaceIdentityError`]: the codes the connect route
///   and the sign-in handoff answer for the same causes.
///
/// Anything else is `crew_not_connected` when the connection has dropped since the request
/// started (the transport retires a dead bridge), and otherwise `crew_request_refused` with
/// `fallback`, the route's fixed sentence, and nothing else. An untyped error's text cannot be
/// trusted to be the core's: it can be a workspace refusal passed through unchanged
/// (`enrollment.pending` or `profile.suggest` refused with a code the core does not know,
/// carrying the broker's whole envelope) or a value quoted from the workspace's answer. Those
/// words are unauthenticated, and the join screen shows a daemon's words verbatim beside the
/// device code, so a process in the joiner's bridge path could otherwise write "your code is
/// …" on it. The text goes to the log, escaped, and never into the answer, not even `detail`.
async fn core_refusal(
    crew: &CrewManager,
    connection_id: Option<&str>,
    error: anyhow::Error,
    fallback: &'static str,
) -> AdmissionRefusal {
    if let Some(refused) = find_cause::<InvitationRefused>(&error) {
        return invitation_refusal(refused);
    }
    if let Some(refused) = find_cause::<JoinRefused>(&error) {
        if let Some(message) = refused.broker_message() {
            // Debug-formatted, so the workspace's words cannot forge a log line.
            tracing::info!(
                code = refused.api_code(),
                broker_message = ?message,
                "Crew workspace refused a join"
            );
        }
        return AdmissionRefusal::new(
            StatusCode::CONFLICT,
            refused.api_code(),
            refused.to_string(),
        );
    }
    if let Some(failed) = find_cause::<HandoffFailed>(&error) {
        if failed.workspace_identity_mismatch() {
            return AdmissionRefusal::new(
                StatusCode::BAD_REQUEST,
                WORKSPACE_IDENTITY_MISMATCH_CODE,
                failed.cause().to_string(),
            );
        }
        return AdmissionRefusal::new(
            StatusCode::BAD_REQUEST,
            failed.api_code(),
            failed.to_string(),
        );
    }
    if let Some(failure) = find_cause::<SshFailure>(&error) {
        let refusal = AdmissionRefusal::new(
            StatusCode::BAD_REQUEST,
            failure.api_code(),
            failure.to_string(),
        );
        return match &failure.detail {
            Some(detail) => refusal.with("detail", detail),
            None => refusal,
        };
    }
    if find_cause::<WorkspaceIdentityError>(&error).is_some() {
        return AdmissionRefusal::new(
            StatusCode::BAD_REQUEST,
            WORKSPACE_IDENTITY_MISMATCH_CODE,
            error.to_string(),
        );
    }
    if let Some(id) = connection_id {
        if crew
            .connection(id)
            .await
            .is_ok_and(|connection| connection.status != "connected")
        {
            return not_connected();
        }
    }
    tracing::warn!(
        answer = fallback,
        cause = ?format!("{error:#}"),
        "Crew admission request failed"
    );
    AdmissionRefusal::new(StatusCode::BAD_REQUEST, REQUEST_REFUSED_CODE, fallback)
}

/// The first cause of `error`, outermost first, that is an `E`.
fn find_cause<E: std::error::Error + Send + Sync + 'static>(error: &anyhow::Error) -> Option<&E> {
    error
        .downcast_ref::<E>()
        .or_else(|| error.chain().find_map(|cause| cause.downcast_ref::<E>()))
}

fn invitation_refusal(refused: &InvitationRefused) -> AdmissionRefusal {
    let status = match refused.reason() {
        InvitationRefusal::IdentityConflict | InvitationRefusal::AlreadySaved => {
            StatusCode::CONFLICT
        }
        InvitationRefusal::Unreadable(_)
        | InvitationRefusal::InvalidChoice
        | InvitationRefusal::Missing(_) => StatusCode::BAD_REQUEST,
    };
    let refusal = AdmissionRefusal::new(status, refused.api_code(), refused.to_string());
    let refusal = match refused.reason() {
        InvitationRefusal::Unreadable(code) => refusal.with("reason", code),
        InvitationRefusal::InvalidChoice => refusal.with("reason", "invalid_choice"),
        InvitationRefusal::Missing(missing) => {
            refusal.with("reason", "missing").with("missing", missing)
        }
        InvitationRefusal::IdentityConflict | InvitationRefusal::AlreadySaved => refusal,
    };
    match refused.connection_id() {
        Some(id) => refusal.with("connection_id", id),
        None => refusal,
    }
}

/// `POST /crew/connections/from-invitation`: the pasted invitation and the person's choices on
/// the Join screen. Every choice is optional; an absent one takes the invitation's.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FromInvitationRequest {
    /// What the person pasted: the host's whole message, the bare `brcrew1:` line, or the JSON
    /// `biorouter-crew status` prints.
    pub invitation: String,
    /// `true`: parse and describe it, and save nothing.
    #[serde(default)]
    pub preview: bool,
    /// The joiner's account name on the server. Default: the username the host invited.
    #[serde(default)]
    pub username: Option<String>,
    /// How this computer treats the workspace. Default: the workspace's own mode.
    #[serde(default)]
    pub mode: Option<ClusterMode>,
    /// Default: the invitation's institution. Required for a Private connection.
    #[serde(default)]
    pub institution_id: Option<String>,
    /// Settings under Advanced. None of them can change the pinned workspace.
    #[serde(default)]
    pub advanced: InvitationAdvanced,
}

impl FromInvitationRequest {
    fn overrides(&mut self) -> InvitationOverrides {
        InvitationOverrides {
            username: self.username.take(),
            mode: self.mode,
            institution_id: self.institution_id.take(),
            advanced: std::mem::take(&mut self.advanced),
        }
    }
}

/// A previewed invitation: the daemon's summary ([`InvitationPreview`]) and the SSH hints and
/// pinned key exactly as the invitation states them.
///
/// `ssh_host` and `ssh_port` are what the invitation *says*; `server` and `port` in the summary
/// are what saving would *use* once the person's choices are applied. Labels are not authority:
/// nothing here is trusted until `hello` verifies against the pinned workspace key.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InvitationSummary {
    #[serde(flatten)]
    pub preview: InvitationPreview,
    /// The Ed25519 workspace key the invitation pins, 64 lowercase hex characters.
    pub workspace_public_key: String,
    /// The SSH server the invitation names (never a host's local alias); `null` for the legacy
    /// status JSON, which names none.
    pub ssh_host: Option<String>,
    /// The SSH port the invitation names; `null` means 22.
    pub ssh_port: Option<u16>,
}

impl InvitationSummary {
    fn new(preview: InvitationPreview, pasted: &str) -> Self {
        // The core parsed this text a moment ago; parse it again only for the hints its
        // summary folds into the save plan. A paste the codec now refuses cannot happen, and
        // would only leave the hints out.
        let invitation = biorouter_crew::invitation::parse(pasted)
            .ok()
            .map(|parsed| parsed.invitation);
        Self {
            workspace_public_key: invitation
                .as_ref()
                .map(|invitation| invitation.workspace_public_key.to_ascii_lowercase())
                .unwrap_or_default(),
            ssh_host: invitation.as_ref().map_or_else(
                || preview.server.clone(),
                |invitation| invitation.ssh_host.clone(),
            ),
            ssh_port: invitation.and_then(|invitation| invitation.ssh_port),
            preview,
        }
    }
}

/// What `POST /crew/connections/from-invitation` answers: exactly one of the two fields.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FromInvitationResponse {
    /// `preview: true`: what the invitation says and what saving would do. Nothing was saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<InvitationSummary>,
    /// The saved connection, pinned exactly as the invitation says (or the one this computer
    /// already had for the same workspace and settings), in the shape `GET /crew/connections`
    /// lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub connection: Option<Box<Connection>>,
}

#[utoipa::path(
    post,
    operation_id = "crew_connection_from_invitation",
    path = "/crew/connections/from-invitation",
    request_body = FromInvitationRequest,
    responses(
        (status = 200, description = "`preview` for a preview (nothing saved), else `connection`: the saved connection, pinned exactly as the invitation says", body = FromInvitationResponse),
        (status = 400, description = "`crew_invitation_invalid` (with `reason`: the invitation codec's code, `invalid_choice`, or `missing` with `missing`), `crew_request_invalid` for a body in the wrong shape, or `crew_request_refused` with a fixed sentence for any other failure", body = Value),
        (status = 403, description = "No proof that a person asked (`crew_user_action_required`, `crew_human_authority_unavailable`)", body = Value),
        (status = 409, description = "`crew_invitation_conflict`: this computer pins a different identity for the same workspace; `crew_connection_exists`: it already has the workspace with other settings. Both carry `connection_id`", body = Value)
    ),
    tag = "Crew"
)]
pub async fn from_invitation(
    headers: HeaderMap,
    body: Result<Json<FromInvitationRequest>, JsonRejection>,
) -> Result<Json<FromInvitationResponse>, AdmissionRefusal> {
    require_person(&headers)?;
    let Json(mut request) =
        body.map_err(|rejection| unreadable_request(rejection.status(), rejection.body_text()))?;
    let crew = crew()?;
    let overrides = request.overrides();
    match crew
        .connection_from_invitation(&request.invitation, request.preview, overrides)
        .await
    {
        Ok(InvitationOutcome::Preview(preview)) => Ok(Json(FromInvitationResponse {
            preview: Some(InvitationSummary::new(*preview, &request.invitation)),
            connection: None,
        })),
        Ok(InvitationOutcome::Saved(connection)) => Ok(Json(FromInvitationResponse {
            preview: None,
            connection: Some(connection),
        })),
        Err(error) => Err(core_refusal(&crew, None, error, FROM_INVITATION_FAILED).await),
    }
}

/// `GET /crew/connections/{id}/invitation`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct InvitationQuery {
    /// The invited person's username on the server, `@bob` or `bob`. The message then names
    /// them, and prefills their username when they paste it.
    #[serde(default)]
    pub invitee: Option<String>,
}

/// `@bob` or `bob` as the account name `bob`; blank is no invitee.
fn invitee(typed: Option<&str>) -> Result<Option<String>, AdmissionRefusal> {
    let Some(typed) = typed.map(str::trim).filter(|typed| !typed.is_empty()) else {
        return Ok(None);
    };
    let name = typed.strip_prefix('@').unwrap_or(typed);
    if biorouter_crew::valid_username(name) {
        Ok(Some(name.to_owned()))
    } else {
        Err(AdmissionRefusal::new(
            StatusCode::BAD_REQUEST,
            INVALID_SELECTOR_CODE,
            "Type the person's username on the server, like @bob.",
        ))
    }
}

#[utoipa::path(
    get,
    operation_id = "crew_connection_invitation",
    path = "/crew/connections/{id}/invitation",
    params(("id" = String, Path, description = "The host's saved Crew connection"), InvitationQuery),
    responses(
        (status = 200, description = "The message to send, and the `brcrew1:` line inside it. Built from this computer's verified connection, the workspace's own word about its name and privacy, and `ssh -G` (never a local alias or the connection's local name)", body = InvitationText),
        (status = 400, description = "`crew_invalid_selector` for an invitee that is not an account name, `crew_request_invalid` for an unknown query parameter, or `crew_request_refused` with a fixed sentence when the invitation can't be built for any other reason (the cause goes to the log)", body = Value),
        (status = 403, description = "No proof that a person asked", body = Value),
        (status = 404, description = "`crew_connection_not_found`", body = Value),
        (status = 409, description = "`crew_not_connected`: connect first", body = Value)
    ),
    tag = "Crew"
)]
pub async fn invitation(
    headers: HeaderMap,
    Path(id): Path<String>,
    query: Result<Query<InvitationQuery>, QueryRejection>,
) -> Result<Json<InvitationText>, AdmissionRefusal> {
    require_person(&headers)?;
    let Query(query) =
        query.map_err(|rejection| unreadable_request(rejection.status(), rejection.body_text()))?;
    let invitee = invitee(query.invitee.as_deref())?;
    let crew = crew()?;
    require_connected(&saved_connection(&crew, &id).await?)?;
    match crew.invitation_for(&id, invitee.as_deref()).await {
        Ok(text) => Ok(Json(text)),
        Err(error) => Err(core_refusal(&crew, Some(&id), error, INVITATION_FAILED).await),
    }
}

#[utoipa::path(
    get,
    operation_id = "crew_connection_join_status",
    path = "/crew/connections/{id}/join",
    params(("id" = String, Path, description = "The joiner's saved Crew connection")),
    responses(
        (status = 200, description = "Where this computer stands in joining. `code` is computed here from the saved device key and the pinned workspace key, never read from the workspace's answer. `unsupported` when the workspace's server can't join by invitation", body = JoinStatus),
        (status = 400, description = "A typed connection failure (`crew_ssh_*`, `crew_bridge_missing`, `crew_workspace_identity_mismatch`), or `crew_request_refused` with a fixed sentence for any other failure. The workspace's own words, which are unauthenticated, go to the log and never into the answer", body = Value),
        (status = 403, description = "No proof that a person asked", body = Value),
        (status = 404, description = "`crew_connection_not_found`", body = Value),
        (status = 409, description = "`crew_not_connected`: connect first", body = Value)
    ),
    tag = "Crew"
)]
pub async fn join_status(
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<JoinStatus>, AdmissionRefusal> {
    require_person(&headers)?;
    let crew = crew()?;
    require_connected(&saved_connection(&crew, &id).await?)?;
    match crew.join_status(&id).await {
        Ok(status) => Ok(Json(status)),
        Err(error) => Err(core_refusal(&crew, Some(&id), error, JOIN_STATUS_FAILED).await),
    }
}

/// What `POST /crew/connections/{id}/join` answers once this computer is a member.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct JoinClaimed {
    /// Always `true`: a join that did not happen is a refusal, never this answer.
    pub joined: bool,
    /// Always `joined`.
    pub status: JoinState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inviter: Option<JoinPerson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    /// This computer was added to an existing member.
    pub add_device: bool,
}

impl JoinClaimed {
    fn new(status: JoinStatus) -> Self {
        Self {
            joined: true,
            status: JoinState::Joined,
            inviter: status.inviter,
            workspace_name: status.workspace_name,
            add_device: status.add_device,
        }
    }
}

#[utoipa::path(
    post,
    operation_id = "crew_connection_join",
    path = "/crew/connections/{id}/join",
    params(("id" = String, Path, description = "The joiner's saved Crew connection")),
    responses(
        (status = 200, description = "This computer is a member. Idempotent: a member answers this without asking the workspace again", body = JoinClaimed),
        (status = 400, description = "A typed connection failure (`crew_ssh_*`, `crew_bridge_missing`, `crew_workspace_identity_mismatch`), or `crew_request_refused` with a fixed sentence for any other failure. The workspace's own words, which are unauthenticated, go to the log and never into the answer", body = Value),
        (status = 403, description = "No proof that a person asked", body = Value),
        (status = 404, description = "`crew_connection_not_found`", body = Value),
        (status = 409, description = "`crew_not_connected`, or a typed join refusal: `crew_join_unsupported`, `crew_join_not_approved`, `crew_join_code_mismatch`, `crew_join_not_invited`, `crew_join_expired`, `crew_join_replaced`, `crew_join_account_changed`, `crew_join_device_conflict`, `crew_join_identity_conflict` or `crew_join_refused`", body = Value)
    ),
    tag = "Crew"
)]
pub async fn join(
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<JoinClaimed>, AdmissionRefusal> {
    require_person(&headers)?;
    let crew = crew()?;
    require_connected(&saved_connection(&crew, &id).await?)?;
    match crew.join(&id).await {
        Ok(status) => Ok(Json(JoinClaimed::new(status))),
        Err(error) => Err(core_refusal(&crew, Some(&id), error, JOIN_FAILED).await),
    }
}

pub fn routes() -> Router {
    Router::new()
        .route("/crew/connections/{id}/authentication", post(prepare))
        .route("/crew/authentication/{id}", axum::routing::delete(cancel))
        .route("/crew/authentication/{id}/terminal", get(terminal))
        .route("/crew/connections/from-invitation", post(from_invitation))
        .route("/crew/connections/{id}/invitation", get(invitation))
        .route("/crew/connections/{id}/join", get(join_status).post(join))
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

    async fn body_of(refusal: AdmissionRefusal) -> (StatusCode, Value) {
        let response = refusal.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// The admission routes answer the proof check exactly as `routes::crew`'s person gate
    /// does: a missing proof and a daemon that cannot check one are told apart, and both refuse.
    /// Pinned here because the digest is a process-global `OnceLock`, so an HTTP test binary
    /// can only ever see one of the two.
    #[tokio::test]
    async fn the_proof_check_has_the_person_gates_three_answers() {
        assert!(person_refusal(UserActionProof::Proven).is_none());
        for (proof, code) in [
            (UserActionProof::Unproven, USER_ACTION_REQUIRED_CODE),
            (
                UserActionProof::NoKeyInstalled,
                HUMAN_AUTHORITY_UNAVAILABLE_CODE,
            ),
        ] {
            let (status, body) = body_of(person_refusal(proof).unwrap()).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body["code"], code);
            assert!(body["error"].as_str().is_some_and(|text| !text.is_empty()));
        }
    }

    #[tokio::test]
    async fn a_refusal_field_never_replaces_its_code_or_words() {
        let refusal = AdmissionRefusal::new(StatusCode::CONFLICT, "crew_example", "Why.")
            .with("code", "forged")
            .with("error", "forged")
            .with("connection_id", "c-1");
        let (status, body) = body_of(refusal).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body,
            json!({"code": "crew_example", "error": "Why.", "connection_id": "c-1"})
        );
    }

    #[test]
    fn an_invitee_is_an_account_name_with_or_without_its_at_sign() {
        assert_eq!(invitee(None).unwrap(), None);
        assert_eq!(invitee(Some("  ")).unwrap(), None);
        assert_eq!(invitee(Some("@bob")).unwrap().as_deref(), Some("bob"));
        assert_eq!(invitee(Some(" bob ")).unwrap().as_deref(), Some("bob"));
        for typed in ["@", "bob lee", "bob/lee", "a:b"] {
            let refusal = invitee(Some(typed)).unwrap_err();
            assert_eq!(
                (refusal.status, refusal.code.as_str()),
                (StatusCode::BAD_REQUEST, INVALID_SELECTOR_CODE),
                "{typed}"
            );
        }
    }

    /// A handoff that failed for a reason the core did not type (the session was replaced or
    /// closed first) keeps the older code and text, which every client already maps.
    #[test]
    fn an_untyped_handoff_failure_keeps_the_older_code() {
        let frame = handoff_failure_frame(&anyhow::anyhow!("Authentication session not found"));
        assert_eq!(
            frame,
            json!({
                "type": "error",
                "code": authentication::HANDOFF_FAILURE_CODE,
                "error": authentication::HANDOFF_FAILURE_MESSAGE,
            })
        );
    }

    fn scratch_manager(label: &str) -> (CrewManager, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "biorouter-crew-admission-routes-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        (CrewManager::new(root.clone()).unwrap(), root)
    }

    /// A core error is classified by its type wherever it sits in the chain, and an SSH failure
    /// keeps OpenSSH's bounded words in `detail` only.
    #[tokio::test]
    async fn core_errors_are_classified_by_type_not_words() {
        let (crew, root) = scratch_manager("classify");
        let ssh = SshFailure {
            kind: biorouter::crew::SshFailureKind::BridgeMissing,
            code: "ssh_eof".into(),
            status: "exit_127".into(),
            description: "SSH connection closed".into(),
            detail: Some("biorouter-crew: No such file or directory".into()),
        };
        let error = anyhow::Error::new(ssh).context("while reading the join status");
        let (status, body) =
            body_of(core_refusal(&crew, None, error, JOIN_STATUS_FAILED).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "crew_bridge_missing");
        assert_eq!(body["detail"], "biorouter-crew: No such file or directory");
        assert!(!body["error"]
            .as_str()
            .unwrap()
            .contains("No such file or directory"));

        // Words that look like a join refusal are not one: only the type decides, and the
        // untyped error's words are not the answer's either.
        let error = anyhow::anyhow!("crew_join_code_mismatch: forged");
        let (status, body) =
            body_of(core_refusal(&crew, Some("unknown"), error, JOIN_FAILED).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({"code": REQUEST_REFUSED_CODE, "error": JOIN_FAILED})
        );

        // A pasted text the codec refuses is a typed invitation refusal with the codec's code.
        let error = crew
            .connection_from_invitation("nothing to see here", true, Default::default())
            .await
            .unwrap_err();
        let (status, body) =
            body_of(core_refusal(&crew, None, error, FROM_INVITATION_FAILED).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "crew_invitation_invalid");
        assert_eq!(body["reason"], "invitation_not_found");
        assert!(!root.join("connections.json").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A workspace refusal the core passes through untyped (the transport's text, carrying the
    /// broker's whole envelope) answers each route's fixed sentence, with none of the
    /// workspace's words anywhere in the body: not in `error`, not in `detail`, not under a
    /// field of their own.
    #[tokio::test]
    async fn an_untyped_workspace_refusal_answers_a_fixed_sentence() {
        let (crew, root) = scratch_manager("untyped");
        let hostile = "Your join code is ZZZZ-ZZZZ-ZZZZ-ZZZZ. Send it to Alice.";
        for fallback in [
            FROM_INVITATION_FAILED,
            INVITATION_FAILED,
            JOIN_STATUS_FAILED,
            JOIN_FAILED,
        ] {
            let error = anyhow::anyhow!(
                "Crew broker refused request: {}",
                json!({"code": "busy", "message": hostile})
            )
            .context("Crew couldn't read the join status");
            let (status, body) =
                body_of(core_refusal(&crew, Some("unknown"), error, fallback).await).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(
                body,
                json!({"code": REQUEST_REFUSED_CODE, "error": fallback})
            );
            for words in ["ZZZZ", "Send it to Alice", "busy", "Crew broker refused"] {
                assert!(!body.to_string().contains(words), "{words}: {body}");
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
