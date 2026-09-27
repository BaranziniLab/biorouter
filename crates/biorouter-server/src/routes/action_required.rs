use crate::state::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use biorouter::agents::approval_relay::{self, ResolveOutcome};
use biorouter::agents::ConfirmationOutcome;
use biorouter::extension_install::{cancel_credentials, submit_credentials, SubmitOutcome};
use biorouter::permission::permission_confirmation::PrincipalType;
use biorouter::permission::{Permission, PermissionConfirmation};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmToolActionRequest {
    id: String,
    #[serde(default = "default_principal_type")]
    principal_type: PrincipalType,
    action: String,
    session_id: String,
}

fn default_principal_type() -> PrincipalType {
    PrincipalType::Tool
}

/// Deliver a tool-permission decision to the prompt that is waiting for it.
///
/// **Idempotent** (BR-62). The decision is routed by tool request id to that
/// prompt's own channel, so a decision for an id nobody is waiting on — a
/// double-clicked Allow, a card the user answered after the prompt expired or the
/// turn was cancelled, a stale client replaying an old confirmation — is dropped
/// rather than applied to whatever tool call happens to be pending now. (Before
/// BR-62 confirmations went to a single per-agent channel, so a late "allow"
/// really could approve an unrelated later tool call.)
///
/// Both outcomes are a 200: a duplicate click is a no-op, not a failure. The
/// `status` field reports which happened — `delivered` when a live prompt took
/// the decision, `unknown` when nothing was waiting on that id.
/// A bare status, in the shape this handler's error type now takes. The body is
/// empty on purpose: these are infrastructure failures with nothing to tell a
/// person, unlike the two proof refusals above.
fn status_only(status: StatusCode) -> (StatusCode, Json<Value>) {
    (status, Json(serde_json::json!({})))
}

#[utoipa::path(
    post,
    path = "/action-required/tool-confirmation",
    request_body = ConfirmToolActionRequest,
    responses(
        (status = 200, description = "Decision processed; `status` is `delivered` or `unknown`", body = Value),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused: `reason` is `unproven` (this request carried no proof it came from the user) or `noKeyInstalled` (this daemon can never obtain that proof)", body = Value),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn confirm_tool_action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ConfirmToolActionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // ⚠ The two refusals say different things, and returning a bare 403 for
    // both is what made this card unanswerable-and-unexplained in browser mode.
    // A caller who presented no proof is being told "the user decides this"; a
    // caller on a daemon that holds no key is being told "this control is
    // unavailable here" — and sending the second person hunting for a
    // permission they can never obtain is the failure `UserActionProof` draws
    // three variants for.
    // ⚠ Sampled ONCE, unconditionally, before the check below. Two reads of
    // the header would leave a window between `requires_user_proof_in_session`
    // (one lock acquisition) and `resolve_in_session` (a later one) — and a
    // gate whose two halves can observe different instants is the race the
    // sample-once rule exists to close.
    let authority = match user_action_proof(&headers) {
        UserActionProof::Proven => {
            biorouter::pending_user_action::DecisionAuthority::from_user_action_proof()
        }
        _ => biorouter::pending_user_action::DecisionAuthority::unproven(),
    };

    if biorouter::pending_user_action::PendingUserActions::global()
        .requires_user_proof_in_session(&request.session_id, &request.id)
    {
        match user_action_proof(&headers) {
            UserActionProof::Proven => {}
            UserActionProof::Unproven => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "status": "refused",
                        "reason": "unproven",
                        "error": UNPROVEN_APPROVAL_REFUSAL,
                    })),
                ))
            }
            UserActionProof::NoKeyInstalled => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "status": "refused",
                        "reason": "noKeyInstalled",
                        "error": NO_KEY_APPROVAL_REFUSAL,
                    })),
                ))
            }
        }
    }
    let permission = match request.action.as_str() {
        "always_allow" => Permission::AlwaysAllow,
        "allow_once" => Permission::AllowOnce,
        "deny" => Permission::DenyOnce,
        _ => Permission::DenyOnce,
    };

    // BR-71 Task 36b: two surfaces, ONE pending ask. The escalation card in the
    // parent's chat carries the origin's tool request id, so a decision posted
    // from either session resolves the same relay entry — and the OTHER surface
    // is dismissed rather than left pending.
    if let Some(ask) = approval_relay::lookup(&request.id, &request.session_id) {
        // ⚠ Resolve the ORIGIN's agent handle BEFORE `resolve` marks the ask
        // decided. `resolve` is a one-way door: it stamps the decision, clears
        // every surface and writes the tree memo. Doing that first and then
        // `?`-ing out of a failed lookup would consume the ask without ever
        // delivering it — the child's prompt stays parked until it times out,
        // both cards are unanswerable (every retry from either surface finds no
        // surface to route from, and one that did would get `already_resolved`),
        // and the memo now holds a grant for a decision that was never applied,
        // so the NEXT identical ask in that tree is auto-approved from it. The
        // fallible step therefore runs while the ask is still untouched.
        let origin = state
            .get_agent_for_route(ask.session_id.clone())
            .await
            .map_err(status_only)?;
        return match approval_relay::resolve(&ask, permission, &request.session_id) {
            ResolveOutcome::Resolved { decision, notify } => {
                let outcome = origin
                    .handle_confirmation_for_session(
                        &ask.session_id,
                        ask.request_id.clone(),
                        PermissionConfirmation {
                            principal_type: request.principal_type,
                            permission: decision,
                        },
                        authority,
                    )
                    .await;
                dismiss_on(&notify, &ask.request_id).await;
                Ok(Json(serde_json::json!({
                    "status": match outcome {
                        ConfirmationOutcome::Delivered => "delivered",
                        ConfirmationOutcome::Unknown => "unknown",
                        ConfirmationOutcome::Unproven => "refused",
                    },
                    "dismissed": notify,
                })))
            }
            // The other surface got there first. 200 and the truth: a
            // double-click is a no-op, and the client reconciles its card from
            // the decision rather than re-posting.
            ResolveOutcome::AlreadyResolved(decision) => Ok(Json(serde_json::json!({
                "status": "already_resolved",
                "decision": decision,
            }))),
            ResolveOutcome::Unknown => Ok(Json(serde_json::json!({ "status": "unknown" }))),
        };
    }

    // Not a delegated ask: the pre-Task-36b path, unchanged.
    let posted_session_id = request.session_id;
    let agent = state
        .get_agent_for_route(posted_session_id.clone())
        .await
        .map_err(status_only)?;
    let outcome = agent
        .handle_confirmation_for_session(
            &posted_session_id,
            request.id.clone(),
            PermissionConfirmation {
                principal_type: request.principal_type,
                permission,
            },
            authority,
        )
        .await;

    let status = match outcome {
        ConfirmationOutcome::Delivered => "delivered",
        ConfirmationOutcome::Unknown => "unknown",
        ConfirmationOutcome::Unproven => "refused",
    };

    Ok(Json(serde_json::json!({ "status": status })))
}

/// A-5: tell every other surface the ask is answered, so its card stops showing
/// as pending. Best-effort — the relay is the truth, and a client that misses
/// the frame learns on its next POST (`already_resolved`).
///
/// ⚠ **The renderer does not handle `resolve_confirmation` yet.** The desktop
/// command union (`workspaceCommandRegistry.ts`) covers `open_tab` /
/// `activate_tab` / `close_tab` / `open_window` / `notify` / `annotate_tab`, so
/// today this frame is dropped and the second card keeps *rendering* as pending
/// until the user clicks it, at which point the `already_resolved` response
/// reconciles it. The wire contract below is the finished half; adding the
/// `resolve_confirmation` case to that registry is Task 25/37's, and until it
/// lands "approve once, both cards clear on their own" is only true for the
/// surface that was clicked. Nothing here needs to change when it does.
async fn dismiss_on(session_ids: &[String], request_id: &str) {
    let Some(services) = biorouter::workspace_services::get() else {
        return;
    };
    if !services.gui_attached() {
        return;
    }
    for session_id in session_ids {
        let _ = services
            .gui_command(
                serde_json::json!({
                    "type": "workspace",
                    "cmd": "resolve_confirmation",
                    "session_id": session_id,
                    "request_id": request_id,
                }),
                false,
            )
            .await;
    }
}

/// Answering a credential card (#117).
///
/// ⚠ **This request body carries secrets and nothing else in the codebase does.**
/// The values reach `submit_credentials`, which writes them to the OS credential
/// store and drops them. Do not add a field to the response that could carry one
/// back, do not add `#[derive(Debug)]` (see the hand-written impl below), and do
/// not log the body.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubmitSecretsRequest {
    /// The card id, as published on the `secretRequest` message.
    id: String,
    /// What the user typed, keyed by the card's key names.
    #[serde(default)]
    values: HashMap<String, String>,
    /// The user dismissed the dialog. `values` is ignored when this is set.
    #[serde(default)]
    cancelled: bool,
}

/// Redacting, deliberately.
///
/// `Debug` is derived on every other request type here, and a derive on this one
/// would put a passcode into any `tracing` line, panic message or test failure
/// that happened to format the request. Naming the *keys* keeps the type
/// debuggable without that ever being possible.
impl std::fmt::Debug for SubmitSecretsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<&str> = self.values.keys().map(String::as_str).collect();
        keys.sort_unstable();
        f.debug_struct("SubmitSecretsRequest")
            .field("id", &self.id)
            .field("keys", &keys)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

/// Store the credentials an extension install is parked on, and release it.
///
/// The response reports **names only**: `configuredKeys` on success, `missing`
/// when a required field came back empty. There is nowhere in it a value can
/// sit, which is what lets the parked install — and therefore the model — be
/// told the truth about what happened without being told what it was.
///
/// Requires the DR-16 proof-of-user header. The model reaches this daemon over
/// the same HTTP with the same secret key, so without the proof it could satisfy
/// its own credential card with a value it invented and drive the install past
/// the one step that exists to involve a person.
#[utoipa::path(
    post,
    path = "/action-required/secrets",
    request_body = SubmitSecretsRequest,
    responses(
        (status = 200, description = "Names of the keys configured, or which required ones are still missing", body = Value),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The request carried no proof it came from the user"),
    )
)]
pub async fn submit_secrets(
    State(_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<SubmitSecretsRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match user_action_proof(&headers) {
        UserActionProof::Proven => {}
        UserActionProof::Unproven => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "refused",
                    "error": UNPROVEN_REFUSAL,
                })),
            ))
        }
        UserActionProof::NoKeyInstalled => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "refused",
                    "error": NO_KEY_REFUSAL,
                })),
            ))
        }
    }

    if request.cancelled {
        let delivered = cancel_credentials(&request.id);
        return Ok(Json(serde_json::json!({
            "status": if delivered { "cancelled" } else { "unknown" },
        })));
    }

    Ok(Json(
        match submit_credentials(&request.id, request.values) {
            SubmitOutcome::Configured { configured_keys } => serde_json::json!({
                "status": "configured",
                "configuredKeys": configured_keys,
            }),
            SubmitOutcome::Incomplete { missing } => serde_json::json!({
                "status": "incomplete",
                "missing": missing,
            }),
            SubmitOutcome::Unknown => serde_json::json!({ "status": "unknown" }),
            SubmitOutcome::Failed { reason } => serde_json::json!({
                "status": "failed",
                "reason": reason,
            }),
        },
    ))
}

/// ⚠ Written for a MODEL to read, in the register `SESSION_OUT_OF_REACH` uses:
/// it forecloses a retry and never suggests that typing the value into the chat
/// could work. A refusal that invited one would be asking the user to do the
/// exact thing this whole feature exists to stop.
/// The two approval refusals. They are deliberately NOT the credential strings
/// below: an approval is a yes/no on an action the model already proposed, so
/// "do not ask for the value in chat" would be answering a question nobody
/// asked. The second one is the browser-mode case — `biorouter serve` starts
/// the daemon with `Stdio::null()`, so no proof-of-user digest is ever
/// installed and this approval can never be granted here, by anyone.
pub const UNPROVEN_APPROVAL_REFUSAL: &str =
    "This decision belongs to the person at the keyboard. Approve it in Biorouter's own \
     dialog rather than over the API.";

pub const NO_KEY_APPROVAL_REFUSAL: &str =
    "This Biorouter daemon was started without a way to tell a person from a model, so an \
     approval like this one cannot be granted here. Run the action in the desktop app, or \
     at a terminal with the Biorouter CLI.";

pub const UNPROVEN_REFUSAL: &str =
    "Credentials can only be submitted by the person at the keyboard, \
     through Biorouter's own dialog. Do not retry, and do not ask for the value in chat — \
     a value in a chat message cannot configure anything and would expose it. \
     Tell the user the dialog is waiting for them.";

pub const NO_KEY_REFUSAL: &str =
    "This Biorouter daemon was started without a way to tell a person from a model, \
     so it cannot accept credentials over HTTP. Configure them at a terminal with \
     `biorouter extension install <bundle>`, which prompts with echo off.";

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ElicitationResponseRequest {
    session_id: String,
    id: String,
    data: Option<Value>,
    #[serde(default)]
    cancelled: bool,
}

#[utoipa::path(
    post,
    path = "/action-required/elicitation",
    request_body = ElicitationResponseRequest,
    responses(
        (status = 200, description = "Delivered to the live ordinary elicitation", body = Value),
        (status = 400, description = "Invalid response shape", body = Value),
        (status = 403, description = "Verified human authority required", body = Value),
        (status = 409, description = "No matching live ordinary elicitation, or answer recorded after waiter ended", body = Value),
        (status = 413, description = "Response exceeds size bound"),
        (status = 500, description = "Persistence failed or completion outcome is unknown", body = Value)
    )
)]
pub async fn respond_to_elicitation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ElicitationResponseRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !matches!(user_action_proof(&headers), UserActionProof::Proven) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "status":"refused", "error":"Verified human authority is required to answer this request"
            })),
        ));
    }
    let valid_id =
        |id: &str| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control);
    let valid_answer = match (&request.data, request.cancelled) {
        (Some(data), false) => {
            data.is_object() && serde_json::to_vec(data).is_ok_and(|bytes| bytes.len() <= 16 * 1024)
        }
        (None, true) => true,
        _ => false,
    };
    if !valid_id(&request.session_id) || !valid_id(&request.id) || !valid_answer {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status":"refused", "error":"Provide bounded session_id and id, and either an object data response of at most 16 KiB or cancelled:true"
            })),
        ));
    }
    let claim = biorouter::action_required_manager::ActionRequiredManager::global()
        .claim_ordinary_elicitation(&request.session_id, &request.id)
        .filter(|claim| !claim.is_closed())
        .ok_or_else(elicitation_unknown)?;
    // The claimed decision owns its completion even if the HTTP client leaves.
    // Cancelling mid-persist could otherwise record an answer without delivery.
    tokio::spawn(complete_elicitation(state, claim, request.id, request.data))
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "status":"outcome_unknown", "error":"Elicitation response completion was interrupted; inspect session history before continuing"
        }))))?
}

fn elicitation_unknown() -> (StatusCode, Json<Value>) {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "status":"unknown", "error":"No matching live ordinary elicitation is waiting in this session; refresh the session before continuing"
        })),
    )
}

async fn complete_elicitation(
    state: Arc<AppState>,
    claim: biorouter::action_required_manager::OrdinaryElicitationClaim,
    request_id: String,
    data: Option<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    use biorouter::agents::{AgentEvent, PersistedMessage};
    use biorouter::conversation::message::{Message, MessageContent};
    use biorouter::session_events::{publish, SessionBusEvent};
    if claim.is_closed() {
        return Err(elicitation_unknown());
    }
    let session_id = claim.session_id().to_owned();
    let mut message = match &data {
        Some(data) => Message::user()
            .with_content(MessageContent::action_required_elicitation_response(
                request_id,
                data.clone(),
            ))
            .agent_only(),
        None => Message::user()
            .with_text(format!("Cancelled input request {request_id}."))
            .user_only(),
    };
    state.session_manager().add_message_adopting_uid(&session_id, &mut message).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "status":"persistence_failed", "error":"The answer could not be recorded; the waiting request was interrupted without delivering it"
        }))))?;
    let message_id = message.id.clone().ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"status":"outcome_unknown","error":"The recorded answer has no authoritative message identity"}))))?;
    let persisted = PersistedMessage {
        id: message_id.clone(),
        user_visible: message.is_user_visible(),
    };
    publish(
        &session_id,
        SessionBusEvent::Agent(AgentEvent::Message(message)),
    );
    publish(
        &session_id,
        SessionBusEvent::Agent(AgentEvent::MessagesPersisted(vec![persisted])),
    );
    if claim.deliver(data) == biorouter::pending_user_action::ResolveOutcome::Delivered {
        Ok(Json(
            serde_json::json!({"status":"delivered","message_id":message_id}),
        ))
    } else {
        Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "status":"recorded_not_delivered", "message_id":message_id,
                "error":"The answer was recorded, but the waiting request ended before delivery; inspect session history before continuing"
            })),
        ))
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/action-required/elicitation",
            post(respond_to_elicitation).layer(axum::extract::DefaultBodyLimit::max(20 * 1024)),
        )
        .route(
            "/action-required/tool-confirmation",
            post(confirm_tool_action),
        )
        .route("/action-required/secrets", post(submit_secrets))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-written `Debug` impl on [`SubmitSecretsRequest`] is the only
    /// thing keeping a passcode out of a `tracing` line, a panic message, or a
    /// test failure that happens to format the request. Because it is written
    /// rather than derived, a later `#[derive(Debug)]` would restore the leak
    /// while every other test in this file kept passing -- the logs surface is
    /// the one credential surface with no other assertion on it.
    #[test]
    fn the_debug_impl_names_the_keys_and_never_the_values() {
        let req: SubmitSecretsRequest = serde_json::from_value(serde_json::json!({
            "id": "card-1",
            "values": {
                "SPOKEAGENT_PASSCODE": "hunter2-the-actual-secret",
                "UCSF_TOKEN": "second-secret-value",
            },
            "cancelled": false,
        }))
        .unwrap();

        for rendered in [format!("{req:?}"), format!("{req:#?}")] {
            assert!(
                !rendered.contains("hunter2-the-actual-secret"),
                "a credential value reached a Debug rendering: {rendered}"
            );
            assert!(
                !rendered.contains("second-secret-value"),
                "a credential value reached a Debug rendering: {rendered}"
            );
            assert!(
                rendered.contains("SPOKEAGENT_PASSCODE"),
                "the key names are what make this type debuggable: {rendered}"
            );
            assert!(
                rendered.contains("card-1"),
                "the card id should survive: {rendered}"
            );
        }
    }

    mod integration_tests {
        use super::*;
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        #[tokio::test(flavor = "multi_thread")]
        async fn test_tool_confirmation_endpoint() {
            let state = AppState::new().await.unwrap();

            let app = routes(state);

            let request = Request::builder()
                .uri("/action-required/tool-confirmation")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .body(Body::from(
                    serde_json::to_string(&ConfirmToolActionRequest {
                        id: "test-id".to_string(),
                        principal_type: PrincipalType::Tool,
                        action: "allow_once".to_string(),
                        session_id: "test-session".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }

        fn post(action: &str, id: &str, session_id: &str) -> Request<Body> {
            post_with_user_action(action, id, session_id, None)
        }

        fn post_with_user_action(
            action: &str,
            id: &str,
            session_id: &str,
            user_action: Option<&str>,
        ) -> Request<Body> {
            let mut builder = Request::builder()
                .uri("/action-required/tool-confirmation")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret");
            if let Some(proof) = user_action {
                builder = builder.header("X-User-Action", proof);
            }
            builder
                .body(Body::from(
                    serde_json::to_string(&ConfirmToolActionRequest {
                        id: id.to_string(),
                        principal_type: PrincipalType::Tool,
                        action: action.to_string(),
                        session_id: session_id.to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap()
        }

        async fn body_json(response: axum::response::Response) -> Value {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }

        fn parked_approval(session_id: &str) -> biorouter::pending_user_action::PendingUserAction {
            parked_approval_requiring(session_id, false)
        }

        fn parked_approval_requiring(
            session_id: &str,
            requires_user_proof: bool,
        ) -> biorouter::pending_user_action::PendingUserAction {
            use biorouter::pending_user_action::{
                PendingUserActions, ToolApprovalRequest, UserActionRequest,
            };

            PendingUserActions::global().park(
                Some(session_id),
                None,
                UserActionRequest::ToolApproval(ToolApprovalRequest {
                    tool_name: "developer__shell".to_string(),
                    arguments: serde_json::Map::new(),
                    prompt: None,
                    risk: None,
                    preview: None,
                    requires_user_proof,
                }),
            )
        }

        fn unique_session(prefix: &str) -> String {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("{prefix}-{}-{sequence}", std::process::id())
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_foreign_confirmation_is_unknown_and_the_owner_can_still_answer() {
            use biorouter::pending_user_action::{PendingUserActions, UserActionOutcome};

            let owner = unique_session("owner");
            let foreign = unique_session("foreign");
            let parked = parked_approval(&owner);
            let id = parked.id().to_string();
            let app = routes(AppState::new().await.unwrap());

            let response = app
                .clone()
                .oneshot(post("allow_once", &id, &foreign))
                .await
                .unwrap();
            assert_eq!(body_json(response).await["status"], "unknown");
            assert!(
                PendingUserActions::global().is_pending(&id),
                "a foreign post must leave the real waiter parked"
            );

            let response = app.oneshot(post("allow_once", &id, &owner)).await.unwrap();
            assert_eq!(body_json(response).await["status"], "delivered");
            assert!(matches!(
                parked.wait(std::time::Duration::from_secs(5), None).await,
                UserActionOutcome::Approved { .. }
            ));
        }

        #[tokio::test(flavor = "multi_thread")]
        #[serial_test::serial]
        async fn a_proof_required_authorization_cannot_be_answered_by_daemon_http_alone() {
            use crate::routes::session::diverge_tests::{
                install_test_user_action_key, TEST_USER_ACTION_KEY,
            };
            use biorouter::pending_user_action::{PendingUserActions, UserActionOutcome};

            install_test_user_action_key();
            let session = unique_session("proof-required");
            let parked = parked_approval_requiring(&session, true);
            let id = parked.id().to_string();
            let app = routes(AppState::new().await.unwrap());

            let no_proof = app
                .clone()
                .oneshot(post("allow_once", &id, &session))
                .await
                .unwrap();
            assert_eq!(no_proof.status(), StatusCode::FORBIDDEN);
            assert!(PendingUserActions::global().is_pending(&id));

            let wrong_proof = app
                .clone()
                .oneshot(post_with_user_action(
                    "allow_once",
                    &id,
                    &session,
                    Some("not-the-user-key"),
                ))
                .await
                .unwrap();
            assert_eq!(wrong_proof.status(), StatusCode::FORBIDDEN);
            assert!(PendingUserActions::global().is_pending(&id));

            let approved = app
                .oneshot(post_with_user_action(
                    "allow_once",
                    &id,
                    &session,
                    Some(TEST_USER_ACTION_KEY),
                ))
                .await
                .unwrap();
            assert_eq!(approved.status(), StatusCode::OK);
            assert!(matches!(
                parked.wait(std::time::Duration::from_secs(5), None).await,
                UserActionOutcome::Approved { .. }
            ));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn an_authorized_relay_resolves_using_the_origin_session() {
            use biorouter::pending_user_action::UserActionOutcome;

            let origin_session = unique_session("origin");
            let relay_session = unique_session("relay");
            let parked = parked_approval(&origin_session);
            let origin = approval_relay::AskId {
                session_id: origin_session.clone(),
                request_id: parked.id().to_string(),
            };
            let tool_request = biorouter::conversation::message::ToolRequest {
                id: origin.request_id.clone(),
                tool_call: Ok(rmcp::model::CallToolRequestParams {
                    meta: None,
                    name: "developer__shell".into(),
                    arguments: Some(serde_json::Map::new()),
                    task: None,
                }),
                metadata: None,
                tool_meta: None,
            };
            approval_relay::register(
                origin.clone(),
                approval_relay::AskKey::for_request(&tool_request).unwrap(),
                approval_relay::AskClass::Delegable,
                relay_session.clone(),
                "/relay-test".to_string(),
            );
            approval_relay::add_surface(&origin, &origin_session);
            approval_relay::add_surface(&origin, &relay_session);
            let app = routes(AppState::new().await.unwrap());

            let response = app
                .oneshot(post("allow_once", &origin.request_id, &relay_session))
                .await
                .unwrap();
            assert_eq!(body_json(response).await["status"], "delivered");
            assert!(matches!(
                parked.wait(std::time::Duration::from_secs(5), None).await,
                UserActionOutcome::Approved { .. }
            ));
            approval_relay::forget(&origin);
        }

        /// BR-71 Task 36b, A-4/A-5 **on the wire**. Everything else about the
        /// relay is unit-tested inside `biorouter`; this is the only test that
        /// proves the endpoint consults it at all. Without it the handler could
        /// ignore the relay entirely — a decision posted from the escalation
        /// surface would fall through to `get_agent_for_route(root)`, find no
        /// prompt parked there, report `unknown`, and leave the child's turn
        /// waiting out its timeout — and every unit test would still pass.
        #[tokio::test(flavor = "multi_thread")]
        async fn a_decision_from_the_escalation_surface_resolves_the_origins_ask_once() {
            let state = AppState::new().await.unwrap();
            let app = routes(state);

            let tool_request = biorouter::conversation::message::ToolRequest {
                id: "br71-call-1".to_string(),
                tool_call: Ok(rmcp::model::CallToolRequestParams {
                    meta: None,
                    name: "acme__widget".into(),
                    arguments: Some(serde_json::Map::new()),
                    task: None,
                }),
                metadata: None,
                tool_meta: None,
            };
            let origin = approval_relay::AskId {
                session_id: "br71-child".to_string(),
                request_id: "br71-call-1".to_string(),
            };
            approval_relay::register(
                origin.clone(),
                approval_relay::AskKey::for_request(&tool_request).unwrap(),
                approval_relay::AskClass::Delegable,
                "br71-root".to_string(),
                "/br71/work".to_string(),
            );
            approval_relay::add_surface(&origin, "br71-child");
            approval_relay::add_surface(&origin, "br71-root");

            // The user clicks Allow in the ROOT's chat — a session that is NOT
            // the one whose agent is parked on this request id.
            let response = app
                .clone()
                .oneshot(post("allow_once", "br71-call-1", "br71-root"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = body_json(response).await;
            assert_eq!(
                body["dismissed"],
                serde_json::json!(["br71-child"]),
                "the origin's own card must be named for dismissal, or it stays pending forever"
            );

            // Clicking again — in either place — is a no-op, not a second grant.
            let response = app
                .clone()
                .oneshot(post("deny", "br71-call-1", "br71-child"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(body_json(response).await["status"], "already_resolved");

            approval_relay::forget(&origin);
        }
    }

    mod elicitation_tests {
        use super::*;
        use crate::routes::session::diverge_tests::{
            install_test_user_action_key, TEST_USER_ACTION_KEY,
        };
        use axum::http::HeaderMap;
        use biorouter::action_required_manager::ActionRequiredManager;
        use biorouter::agents::AgentEvent;
        use biorouter::conversation::message::{
            ActionRequiredData, MessageContent, SecretDestination, SecretKeyRequest,
        };
        use biorouter::pending_user_action::{
            PendingUserActions, SecretsRequest, ToolApprovalRequest, UserActionRequest,
        };
        use biorouter::session_events::{self, SessionBusEvent};
        use serial_test::serial;

        async fn wait_for_ordinary_card(
            session_id: &str,
        ) -> (
            String,
            tokio::task::JoinHandle<anyhow::Result<Option<Value>>>,
        ) {
            let manager = ActionRequiredManager::global();
            let session_id = session_id.to_owned();
            let waiter_session_id = session_id.clone();
            let waiter = tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Which cohort?".into(),
                        serde_json::json!({"type":"object"}),
                        std::time::Duration::from_secs(5),
                        Some(&waiter_session_id),
                    )
                    .await
            });
            for _ in 0..100 {
                for message in manager.drain_requests(&session_id) {
                    for content in message.content {
                        if let MessageContent::ActionRequired(action) = content {
                            if let ActionRequiredData::Elicitation { id, .. } = action.data {
                                return (id, waiter);
                            }
                        }
                    }
                }
                tokio::task::yield_now().await;
            }
            waiter.abort();
            panic!("ordinary elicitation card was not published");
        }

        fn proven_headers() -> HeaderMap {
            install_test_user_action_key();
            let mut headers = HeaderMap::new();
            headers.insert("X-User-Action", TEST_USER_ACTION_KEY.parse().unwrap());
            headers
        }

        fn secret_request() -> SecretsRequest {
            SecretsRequest {
                prompt: "Synthetic secret".into(),
                keys: vec![SecretKeyRequest {
                    key: "SYNTHETIC_SECRET".into(),
                    label: "Synthetic secret".into(),
                    description: None,
                    required: true,
                }],
                destination: SecretDestination::Keyring,
            }
        }

        async fn respond(
            state: Arc<AppState>,
            headers: HeaderMap,
            request: ElicitationResponseRequest,
        ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
            respond_to_elicitation(State(state), headers, Json(request)).await
        }

        #[tokio::test]
        #[serial]
        async fn live_ordinary_elicitation_delivers_only_to_its_exact_session() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let root = tempfile::tempdir().unwrap();
            let _env = crate::test_sandbox::relocate_path_root(root.path());
            let state = AppState::new().await.unwrap();
            let session = state
                .session_manager()
                .create_session(
                    root.path().to_path_buf(),
                    "Synthetic ordinary elicitation".into(),
                    biorouter::session::session_manager::SessionType::User,
                )
                .await
                .unwrap()
                .id;
            let (id, waiter) = wait_for_ordinary_card(&session).await;
            let before_wrong = state
                .session_manager()
                .get_session(&session, true)
                .await
                .unwrap()
                .message_count;
            let wrong = respond(
                Arc::clone(&state),
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: "ordinary-other-session".into(),
                    id: id.clone(),
                    data: Some(serde_json::json!({"cohort":"wrong"})),
                    cancelled: false,
                },
            )
            .await
            .expect_err("a foreign session must not answer the card");
            assert_eq!(wrong.0, StatusCode::CONFLICT);
            assert_eq!(
                state
                    .session_manager()
                    .get_session(&session, true)
                    .await
                    .unwrap()
                    .message_count,
                before_wrong
            );
            let mut events = session_events::subscribe(&session);

            let delivered = respond(
                Arc::clone(&state),
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: session.clone(),
                    id: id.clone(),
                    data: Some(serde_json::json!({"cohort":"synthetic"})),
                    cancelled: false,
                },
            )
            .await
            .expect("live ordinary answer delivers");
            assert_eq!(delivered.0["status"], "delivered");
            assert!(delivered.0["message_id"].as_str().is_some());
            assert_eq!(
                waiter.await.unwrap().unwrap(),
                Some(serde_json::json!({"cohort":"synthetic"}))
            );
            let stored = state
                .session_manager()
                .get_session(&session, true)
                .await
                .unwrap();
            let conversation = stored.conversation.unwrap_or_default();
            let answer = conversation
                .iter()
                .find(|message| message.id.as_deref() == delivered.0["message_id"].as_str())
                .expect("delivered answer is persisted under the response message id");
            assert!(!answer.is_user_visible(), "answer remains agent-only");
            assert!(answer.is_agent_visible());
            let answer_json = serde_json::to_string(answer).unwrap();
            assert!(answer_json.contains("synthetic"));
            let first = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap();
            let second = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                first,
                SessionBusEvent::Agent(AgentEvent::Message(_))
            ));
            assert!(matches!(
                second,
                SessionBusEvent::Agent(AgentEvent::MessagesPersisted(_))
            ));

            let before_replay = state
                .session_manager()
                .get_session(&session, true)
                .await
                .unwrap()
                .message_count;
            let replay = respond(
                Arc::clone(&state),
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: session.clone(),
                    id,
                    data: Some(serde_json::json!({"cohort":"replay"})),
                    cancelled: false,
                },
            )
            .await
            .expect_err("a stale replay must be unknown");
            assert_eq!(replay.0, StatusCode::CONFLICT);
            assert_eq!(
                state
                    .session_manager()
                    .get_session(&session, true)
                    .await
                    .unwrap()
                    .message_count,
                before_replay
            );

            let (cancel_id, cancel_waiter) = wait_for_ordinary_card(&session).await;
            let cancelled = respond(
                state.clone(),
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: session.clone(),
                    id: cancel_id,
                    data: None,
                    cancelled: true,
                },
            )
            .await
            .expect("ordinary cancellation is durably recorded");
            assert_eq!(cancelled.0["status"], "delivered");
            assert!(cancel_waiter.await.unwrap().unwrap().is_none());
            let cancelled_session = state
                .session_manager()
                .get_session(&session, true)
                .await
                .unwrap()
                .conversation
                .unwrap_or_default();
            let cancelled_row = cancelled_session
                .iter()
                .find(|message| {
                    serde_json::to_string(message)
                        .map(|value| value.contains("Cancelled input request"))
                        .unwrap_or(false)
                })
                .expect("cancellation receipt is persisted");
            assert!(cancelled_row.is_user_visible());
            assert!(!cancelled_row.is_agent_visible());
        }

        #[tokio::test]
        #[serial]
        async fn cancellation_cannot_consume_tool_or_secret_asks() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let root = tempfile::tempdir().unwrap();
            let _env = crate::test_sandbox::relocate_path_root(root.path());
            let state = AppState::new().await.unwrap();
            let session = format!("ordinary-cancel-{}", uuid::Uuid::new_v4());
            let approval = PendingUserActions::global().park(
                Some(&session),
                None,
                UserActionRequest::ToolApproval(ToolApprovalRequest {
                    tool_name: "synthetic_tool".into(),
                    arguments: serde_json::Map::new(),
                    prompt: None,
                    risk: None,
                    preview: None,
                    requires_user_proof: false,
                }),
            );
            let id = approval.id().to_string();
            let response = respond(
                Arc::clone(&state),
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: session.clone(),
                    id: id.clone(),
                    data: None,
                    cancelled: true,
                },
            )
            .await
            .expect_err("ordinary cancellation cannot answer a tool approval");
            assert_eq!(response.0, StatusCode::CONFLICT);
            assert!(PendingUserActions::global().is_pending(&id));
            drop(approval);

            let secret = PendingUserActions::global().park(
                Some(&session),
                None,
                UserActionRequest::Secrets(secret_request()),
            );
            let secret_id = secret.id().to_string();
            let response = respond(
                state,
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: session,
                    id: secret_id.clone(),
                    data: None,
                    cancelled: true,
                },
            )
            .await
            .expect_err("ordinary cancellation cannot answer a secret request");
            assert_eq!(response.0, StatusCode::CONFLICT);
            assert!(PendingUserActions::global().is_pending(&secret_id));
            drop(secret);
        }

        #[tokio::test]
        #[serial]
        async fn persistence_failure_does_not_deliver_or_publish_an_answer() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let root = tempfile::tempdir().unwrap();
            let _env = crate::test_sandbox::relocate_path_root(root.path());
            let state = AppState::new().await.unwrap();
            let session = state
                .session_manager()
                .create_session(
                    root.path().to_path_buf(),
                    "Synthetic persistence failure".into(),
                    biorouter::session::session_manager::SessionType::User,
                )
                .await
                .unwrap()
                .id;
            let (id, waiter) = wait_for_ordinary_card(&session).await;
            let mut events = biorouter::session_events::subscribe(&session);
            state.session_manager().close().await;

            let failure = respond(
                state,
                proven_headers(),
                ElicitationResponseRequest {
                    session_id: session,
                    id,
                    data: Some(serde_json::json!({"cohort":"must-not-deliver"})),
                    cancelled: false,
                },
            )
            .await
            .expect_err("closed persistence must refuse delivery");
            assert_eq!(failure.0, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(failure.1["status"], "persistence_failed");
            let waiter_result = tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
                .await
                .expect("persistence failure must settle the live waiter without an answer")
                .unwrap();
            assert!(
                waiter_result.is_err() || waiter_result.as_ref().is_ok_and(Option::is_none),
                "persistence failure must not deliver an answer"
            );
            assert!(
                events.try_recv().is_err(),
                "failed persistence publishes no events"
            );
        }

        #[tokio::test]
        #[serial]
        async fn ordinary_elicitation_rejects_unproven_wrong_shapes_and_oversized_data() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let root = tempfile::tempdir().unwrap();
            let _env = crate::test_sandbox::relocate_path_root(root.path());
            let state = AppState::new().await.unwrap();
            let session = format!("ordinary-shape-{}", uuid::Uuid::new_v4());
            let id = "synthetic-request";
            let mut headers = HeaderMap::new();
            let unproven = respond(
                Arc::clone(&state),
                headers.clone(),
                ElicitationResponseRequest {
                    session_id: session.clone(),
                    id: id.into(),
                    data: Some(serde_json::json!({})),
                    cancelled: false,
                },
            )
            .await
            .expect_err("ordinary response requires proof");
            assert_eq!(unproven.0, StatusCode::FORBIDDEN);

            headers = proven_headers();
            let wrong_shape = respond(
                Arc::clone(&state),
                headers.clone(),
                ElicitationResponseRequest {
                    session_id: session.clone(),
                    id: id.into(),
                    data: Some(serde_json::json!("scalar")),
                    cancelled: false,
                },
            )
            .await
            .expect_err("ordinary response data must be an object");
            assert_eq!(wrong_shape.0, StatusCode::BAD_REQUEST);

            let oversized = respond(
                state,
                headers,
                ElicitationResponseRequest {
                    session_id: session,
                    id: id.into(),
                    data: Some(serde_json::json!({"answer":"x".repeat(16 * 1024 + 1)})),
                    cancelled: false,
                },
            )
            .await
            .expect_err("ordinary response data has a 16 KiB bound");
            assert_eq!(oversized.0, StatusCode::BAD_REQUEST);
        }
    }

    /// Issue #117. The credential card, over the wire.
    ///
    /// These are the assertions the feature's security argument rests on, and
    /// none of them can be made from inside `biorouter`: only here do a request
    /// body, an auth header and a response body exist at once.
    mod secrets_tests {
        use super::*;
        use crate::routes::session::diverge_tests::{
            install_test_user_action_key, TEST_USER_ACTION_KEY,
        };
        use axum::{body::Body, http::Request};
        use biorouter::conversation::message::SecretDestination;
        use biorouter::extension_install::{park_credentials, BrxtEnvVar, CredentialSpec};
        use biorouter::pending_user_action::PendingUserActions;
        use serial_test::serial;
        use tower::ServiceExt;

        fn var(key: &str, required: bool, secret: bool) -> BrxtEnvVar {
            BrxtEnvVar {
                key: key.to_string(),
                required,
                auto_propagate: false,
                default: None,
                description: String::new(),
                secret,
            }
        }

        fn spec(vars: Vec<BrxtEnvVar>) -> CredentialSpec {
            CredentialSpec {
                destination: SecretDestination::ExtensionEnv {
                    extension_name: "spokeagent".to_string(),
                },
                vars,
            }
        }

        fn post(body: Value, user_action: Option<&str>) -> Request<Body> {
            let mut builder = Request::builder()
                .uri("/action-required/secrets")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret");
            if let Some(key) = user_action {
                builder = builder.header("X-User-Action", key);
            }
            builder.body(Body::from(body.to_string())).unwrap()
        }

        async fn body_of(response: axum::response::Response) -> Value {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }

        /// The whole point, stated as an assertion: what the user typed goes to
        /// the credential store, and the answer carries the key's NAME.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn a_submitted_value_is_never_echoed_back() {
            install_test_user_action_key();
            let app = routes(AppState::new().await.unwrap());

            let parked = park_credentials(
                Some("s-echo"),
                None,
                "Configure".to_string(),
                // Non-secret so the test writes nothing to the machine's
                // credential store; the response shape is identical either way.
                spec(vec![var("OMOP_HOST", true, false)]),
            );
            let id = parked.id().to_string();

            let response = app
                .clone()
                .oneshot(post(
                    serde_json::json!({
                        "id": id,
                        "values": { "OMOP_HOST": "https://omop.internal.example" },
                    }),
                    Some(TEST_USER_ACTION_KEY),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);

            let body = body_of(response).await;
            assert_eq!(body["status"], "configured");
            assert_eq!(body["configuredKeys"], serde_json::json!(["OMOP_HOST"]));
            let rendered = body.to_string();
            assert!(
                !rendered.contains("omop.internal.example"),
                "the response echoed the submitted value back: {rendered}"
            );

            let (outcome, settings) = parked.wait(std::time::Duration::from_secs(5), None).await;
            assert!(outcome.is_allowed());
            // The non-secret setting reaches the install out of band — never
            // through the conversation transport, and never through the model.
            assert_eq!(
                settings.get("OMOP_HOST").map(String::as_str),
                Some("https://omop.internal.example")
            );
        }

        /// DR-16. The model reaches this daemon over the same HTTP with the same
        /// secret key, so without the proof-of-user header it could satisfy its
        /// own credential card and drive the install past the one step that
        /// exists to involve a person.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn a_request_without_the_proof_of_user_header_is_refused() {
            install_test_user_action_key();
            let app = routes(AppState::new().await.unwrap());

            let parked = park_credentials(
                Some("s-unproven"),
                None,
                "Configure".to_string(),
                spec(vec![var("OMOP_HOST", true, false)]),
            );
            let id = parked.id().to_string();

            let response = app
                .clone()
                .oneshot(post(
                    serde_json::json!({ "id": id, "values": { "OMOP_HOST": "x" } }),
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);

            let error = body_of(response).await["error"]
                .as_str()
                .unwrap()
                .to_string();
            // ⚠ The refusal is read by a MODEL. It must not send it back to ask
            // the user for the value in chat, which is the exact failure #117
            // exists to end.
            let lowered = error.to_lowercase();
            assert!(
                !lowered.contains("ask the user for the value") && !lowered.contains("paste"),
                "the refusal invited a chat answer: {error}"
            );
            assert!(lowered.contains("do not retry"));

            assert!(
                PendingUserActions::global().is_pending(&id),
                "a refused submission must leave the install parked"
            );
            drop(parked);
        }

        /// BR-62's property on the credential path: two installs in flight
        /// cannot answer each other, and a replayed answer lands nowhere.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn one_dialog_cannot_satisfy_another_installs_request() {
            install_test_user_action_key();
            let app = routes(AppState::new().await.unwrap());

            let first = park_credentials(
                Some("s-one"),
                None,
                "One".to_string(),
                spec(vec![var("ONE_HOST", true, false)]),
            );
            let second = park_credentials(
                Some("s-two"),
                None,
                "Two".to_string(),
                spec(vec![var("TWO_HOST", true, false)]),
            );
            let (id_one, id_two) = (first.id().to_string(), second.id().to_string());

            let response = app
                .clone()
                .oneshot(post(
                    serde_json::json!({ "id": id_one, "values": { "ONE_HOST": "a" } }),
                    Some(TEST_USER_ACTION_KEY),
                ))
                .await
                .unwrap();
            assert_eq!(body_of(response).await["status"], "configured");
            assert!(
                PendingUserActions::global().is_pending(&id_two),
                "answering one install released another"
            );

            // A replay of the same id is a no-op, not a second grant, and it
            // does not fall through onto whatever is parked now.
            let response = app
                .clone()
                .oneshot(post(
                    serde_json::json!({ "id": id_one, "values": { "ONE_HOST": "b" } }),
                    Some(TEST_USER_ACTION_KEY),
                ))
                .await
                .unwrap();
            assert_eq!(body_of(response).await["status"], "unknown");
            assert!(PendingUserActions::global().is_pending(&id_two));

            let (outcome, _) = first.wait(std::time::Duration::from_secs(5), None).await;
            assert!(outcome.is_allowed());
            drop(second);
        }

        /// A typo is not a rollback: an empty required field leaves the dialog
        /// open and the install parked, and says which field.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn an_empty_required_field_leaves_the_dialog_open() {
            install_test_user_action_key();
            let app = routes(AppState::new().await.unwrap());

            let parked = park_credentials(
                Some("s-empty"),
                None,
                "Configure".to_string(),
                spec(vec![var("OMOP_HOST", true, false)]),
            );
            let id = parked.id().to_string();

            let response = app
                .clone()
                .oneshot(post(
                    serde_json::json!({ "id": id, "values": { "OMOP_HOST": "   " } }),
                    Some(TEST_USER_ACTION_KEY),
                ))
                .await
                .unwrap();
            let body = body_of(response).await;
            assert_eq!(body["status"], "incomplete");
            assert_eq!(body["missing"], serde_json::json!(["OMOP_HOST"]));
            assert!(PendingUserActions::global().is_pending(&id));
            drop(parked);
        }

        /// Cancel is a first-class result, and it releases the install so it can
        /// roll back rather than leaving it parked until the TTL.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn cancelling_releases_the_install() {
            install_test_user_action_key();
            let app = routes(AppState::new().await.unwrap());

            let parked = park_credentials(
                Some("s-cancel"),
                None,
                "Configure".to_string(),
                spec(vec![var("OMOP_HOST", true, false)]),
            );
            let id = parked.id().to_string();

            let response = app
                .clone()
                .oneshot(post(
                    serde_json::json!({ "id": id, "cancelled": true }),
                    Some(TEST_USER_ACTION_KEY),
                ))
                .await
                .unwrap();
            assert_eq!(body_of(response).await["status"], "cancelled");

            let (outcome, _) = parked.wait(std::time::Duration::from_secs(5), None).await;
            assert!(!outcome.is_allowed());
        }

        /// The redacting `Debug` impl, asserted rather than assumed: a derive
        /// here would put a passcode into any log line that formatted the body.
        #[test]
        fn the_request_debug_impl_names_keys_and_never_values() {
            let request: SubmitSecretsRequest = serde_json::from_value(serde_json::json!({
                "id": "card-1",
                "values": { "SPOKEAGENT_PASSCODE": "hunter2" },
            }))
            .unwrap();
            let rendered = format!("{request:?}");
            assert!(rendered.contains("SPOKEAGENT_PASSCODE"));
            assert!(
                !rendered.contains("hunter2"),
                "the request Debug impl leaked a value: {rendered}"
            );
        }
    }
}
