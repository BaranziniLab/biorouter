//! The Crew routes' answers, named for the OpenAPI spec and the generated client (CROSSCUT-6).
//!
//! Every `/crew` route answers a type declared here or in the core, so the spec says what
//! the wire carries and a client that renames a field fails to compile against the next
//! generated client. Only the workspace's own answers ([`CrewWorkspaceAnswer`]) stay open:
//! their shape is the Crew broker protocol's, which the daemon forwards unchanged.
//!
//! [`CrewError`] is every Crew refusal's shape. The refusals themselves are still built as
//! `{code, error}` plus fields (`CrewRouteError`, `AdmissionRefusal`, the profile and
//! transfer refusals), so `openapi_contract_tests` pins the two together: a refusal field
//! that [`CrewError`] does not name, or a `crew_*` code the spec does not mention, fails it.
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, OptionalFromRequest, Request};
use axum::response::{IntoResponse, Response};
use axum::Json;
use biorouter::crew::authentication::InvitationMissing;
use biorouter::crew::observation::RunView;
use biorouter::crew::{ClusterMode, Connection, GrantRow};
use biorouter_server::crew::transfers::Receipt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use super::names::SelectorKind;

/// The code of a request whose body or query is not in the shape the route takes.
pub const REQUEST_INVALID_CODE: &str = "crew_request_invalid";
/// What a request the route could not read is refused with. The reader's own diagnostic,
/// which can quote a value the client sent back to it, goes in `detail` for "Copy details".
pub const UNREADABLE_REQUEST: &str =
    "Biorouter couldn't read this request. Update the app or command that sent it, and try again.";

/// A Crew route's JSON body. A body it cannot read (not JSON, the wrong media type, a field it
/// does not take) is refused as every Crew refusal is, `crew_request_invalid` with the
/// reader's status, rather than with axum's plain-text answer, which carries no code.
pub struct CrewJson<T>(pub T);

/// A body [`CrewJson`] could not read.
pub struct CrewBodyRejection(JsonRejection);

impl IntoResponse for CrewBodyRejection {
    fn into_response(self) -> Response {
        let refusal = CrewError {
            code: REQUEST_INVALID_CODE.into(),
            error: UNREADABLE_REQUEST.into(),
            detail: Some(self.0.body_text()),
            ..CrewError::default()
        };
        (self.0.status(), Json(refusal)).into_response()
    }
}

impl<T, S> FromRequest<S> for CrewJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = CrewBodyRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        <Json<T> as FromRequest<S>>::from_request(request, state)
            .await
            .map(|Json(body)| Self(body))
            .map_err(CrewBodyRejection)
    }
}

/// A body the route takes when one is sent: none at all is `None`.
impl<T, S> OptionalFromRequest<S> for CrewJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = CrewBodyRejection;

    async fn from_request(request: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        <Json<T> as OptionalFromRequest<S>>::from_request(request, state)
            .await
            .map(|body| body.map(|Json(body)| Self(body)))
            .map_err(CrewBodyRejection)
    }
}

/// Every code a Crew answer carries: a refusal's `code`, a saved connection's
/// `last_error_code`, and a host-start run's `error.code`. Each route's responses say which it
/// answers and when. A client branches on these, never on a refusal's words, so the generated
/// client names every one; `openapi_contract_tests` fails on a code a Crew source answers that
/// is missing here, and on one listed here that nothing answers any more.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(
    dead_code,
    reason = "the spec's list of codes: the refusals write theirs as text, and the contract test holds the two together"
)]
pub enum CrewErrorCode {
    // Any route.
    CrewUserActionRequired,
    CrewHumanAuthorityUnavailable,
    CrewRequestInvalid,
    CrewRequestRefused,
    CrewCredentialStoreUnavailable,
    CrewCredentialStoreRefused,
    // A connection and its server.
    CrewConnectionNotFound,
    CrewConnectionRequired,
    CrewNotConnected,
    CrewMembershipEnded,
    CrewSshAuthRequired,
    CrewSshKeyRefused,
    CrewSshHostKeyUnknown,
    CrewSshHostKeyChanged,
    CrewSshUnreachable,
    CrewSshFailed,
    CrewBridgeMissing,
    CrewBrokerNotRunning,
    CrewHandoffFailed,
    CrewWorkspaceIdentityMismatch,
    // A request the workspace was sent.
    CrewNotSent,
    CrewOutcomeUnknown,
    CrewReconnecting,
    CrewModeMismatch,
    CrewInstitutionMismatch,
    CrewPublicModelRefused,
    CrewChannelNotInWorkspace,
    CrewModelFixed,
    CrewTypedRunRequired,
    // Names.
    CrewInvalidSelector,
    UnknownName,
    AmbiguousName,
    // Tasks and grants.
    CrewIdempotencyConflict,
    CrewStartOutcomeUnknown,
    CrewCancelPersistenceFailed,
    CrewSessionUnavailable,
    CrewRevocationUnconfirmed,
    CrewRevocationNotSaved,
    CrewGrantNotFound,
    CrewGrantOtherConnection,
    CrewGrantReplaced,
    // The credential vault.
    CrewProfileRefused,
    // Files and transfers.
    CrewTransferRefused,
    CrewFileIsCredential,
    CrewFileNameHidden,
    CrewFolderShared,
    CrewDestinationIsFolder,
    CrewDestinationExists,
    CrewFileIsProgram,
    // Observation.
    ObserverCapacityReached,
    // Joining and invitations.
    CrewInvitationInvalid,
    CrewInvitationConflict,
    CrewConnectionExists,
    CrewJoinUnsupported,
    CrewJoinNotApproved,
    CrewJoinCodeMismatch,
    CrewJoinNotInvited,
    CrewJoinExpired,
    CrewJoinReplaced,
    CrewJoinAccountChanged,
    CrewJoinDeviceConflict,
    CrewJoinIdentityConflict,
    CrewJoinRefused,
    // Starting Crew on a server for its host.
    CrewHostSetupUnknown,
    CrewHostSetupUsed,
    CrewHostStartBusy,
    CrewHostStartNotFound,
    CrewHostStartTimedOut,
    CrewHostStartCancelled,
}

/// A Crew route's refusal: `code` and `error` always, and beside them only the fields that
/// refusal names. Branch on `code`, never on `error`, which is written for a person.
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct CrewError {
    /// The refusal's stable code. Each route's responses say which it answers and when.
    #[schema(value_type = CrewErrorCode)]
    pub code: String,
    /// One plain sentence for a person.
    pub error: String,
    /// Diagnostic words for "Copy details", never shown by default: OpenSSH's own bounded
    /// words for an SSH failure, a request reader's diagnostic for `crew_request_invalid`, or
    /// why the workspace has not confirmed a revocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The SSH hop an SSH failure concerns, a jump host's included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The workspace's own refusal code, when `code` is `crew_request_refused` because the
    /// workspace refused (`name_taken`, `forbidden`, `response_too_large`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_code: Option<String>,
    /// The saved connection a `crew_invitation_conflict` or `crew_connection_exists` concerns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    /// Why `crew_invitation_invalid` refused a paste: the invitation codec's own code,
    /// `invalid_choice`, or `missing`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// With `reason: missing`: what the invitation lacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<InvitationMissing>,
    /// `crew_outcome_unknown`: the request's idempotency key. Retrying with it is applied at
    /// most once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// `crew_outcome_unknown` and `crew_not_sent`: the SSH failure's code when one caused it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_code: Option<String>,
    /// The workspace as a person calls it: `crew_reconnecting`, `crew_public_model_refused`,
    /// `crew_channel_not_in_workspace`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// `crew_mode_mismatch`: the connection's privacy mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub actual_mode: Option<ClusterMode>,
    /// `crew_mode_mismatch`: the privacy mode the request required.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub expected_mode: Option<ClusterMode>,
    /// `crew_institution_mismatch` for a model: who approved the model and whose the
    /// workspace is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub institution_refusal: Option<CrewInstitutionRefusal>,
    /// `crew_institution_mismatch` on a save: the other saved connection to the same
    /// workspace, by its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// `crew_institution_mismatch` on a save: that connection's institution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_institution: Option<String>,
    /// `crew_institution_mismatch` on a save: the institution the save gave.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub institution: Option<String>,
    /// `crew_institution_mismatch` when connections to one workspace disagree: every
    /// institution they name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub institutions: Option<Vec<String>>,
    /// `unknown_name` and `ambiguous_name`: what the name was looked up as.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub kind: Option<SelectorKind>,
    /// `unknown_name` and `ambiguous_name`: the name as sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `ambiguous_name`: every saved connection the name matches, as a person reads them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<String>>,
    /// A revocation refusal: the chat or task whose grant it concerns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// A revocation refusal: the grant's run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// `crew_revocation_unconfirmed`: `true`; the grant is stopped here and the stop is saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_on_this_device: Option<bool>,
    /// `crew_revocation_unconfirmed`: `false`; the daemon asks the workspace again by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_revocation_confirmed: Option<bool>,
    /// A task's revocation: the task's status after the stop (`cancelled`,
    /// `cancellation_unconfirmed`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_status: Option<String>,
    /// A task's revocation: why its status could not be saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_status_error: Option<String>,
}

/// What a `crew_institution_mismatch` for a model names beside its sentence
/// (`biorouter::crew::institution_refusal_details`).
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewInstitutionRefusal {
    /// The model, as requested.
    pub model: String,
    /// The institutions that approved the model; `null` when it states none.
    pub approved_for: Option<Vec<String>>,
    /// The workspace's signed name, else the saved connection's name.
    pub workspace: Option<String>,
    /// The workspace's institution.
    pub workspace_institution: Option<String>,
}

/// A saved connection as the routes answer it: its saved fields, plus what to call its server
/// and, when the daemon has one, the code behind `last_error`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewConnectionView {
    #[serde(flatten)]
    pub connection: Connection,
    /// Types `last_error`, when the daemon has a code for it: `crew_membership_ended` (the
    /// workspace refused this computer or its person as no longer a member, so the daemon
    /// stops dialling it), an SSH failure's code (`crew_ssh_*`, `crew_bridge_missing`,
    /// `crew_broker_not_running`) or `crew_workspace_identity_mismatch`. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<CrewErrorCode>)]
    pub last_error_code: Option<String>,
    /// What to call the connection's server on screen (D-ALIAS): the person's own SSH alias
    /// for its address when one maps to it, else the host. Display only, and never saved, so
    /// it never enters the connection's binding or an invitation.
    pub server_label: String,
}

/// `GET /crew/connections`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewConnectionList {
    /// Every connection saved on this computer.
    pub connections: Vec<CrewConnectionView>,
}

/// `DELETE /crew/connections/{id}`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewConnectionRemoved {
    /// Always `true`.
    pub removed: bool,
}

/// `POST /crew/connections/{id}/disconnect`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewDisconnected {
    /// Always `true`.
    pub disconnected: bool,
}

/// A stop that has nothing more to say: `DELETE /crew/host/start/{job_id}` and
/// `DELETE /crew/authentication/{id}`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewCancelled {
    /// Always `true`. Stopping something that already ended changes nothing.
    pub cancelled: bool,
}

/// The workspace's own answer, forwarded unchanged: a protocol request's result
/// (`POST /crew/connections/{id}/request`) or a chat's context manifest. Its shape is the
/// Crew broker protocol's, not this API's.
#[derive(Debug, Serialize, ToSchema)]
#[serde(transparent)]
pub struct CrewWorkspaceAnswer(#[schema(value_type = Object)] pub Value);

/// `GET /crew/connections/{id}/runs`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewRunList {
    /// This computer's tasks on the connection, newest first.
    pub runs: Vec<RunView>,
}

/// `POST /crew/connections/{id}/runs/{run_id}/cancel`: what stopping one of this computer's
/// tasks did. A stop the workspace did not confirm is refused instead
/// (`crew_revocation_unconfirmed`), so a 200 is a stop that took effect.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewRunCancellation {
    /// The task's status is `cancelled` now.
    pub cancelled: bool,
    /// The task's status: `cancelled`, or the status a task that had already ended kept
    /// (`completed`, `failed`).
    pub status: String,
    /// `true`: the task had already ended, so nothing was stopped or revoked. Absent
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already_finished: Option<bool>,
    /// `true`: the workspace confirmed revoking the task's grant. Absent when the task had
    /// already ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_revocation_confirmed: Option<bool>,
    /// What the stop did, for a person. Absent when the task had already ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `POST /crew/connections/{id}/sessions/{session_id}/grant`: the chat's new grant.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewSessionGrant {
    /// The grant's run at the workspace.
    pub run_id: String,
    /// The chat the grant is for.
    pub session_id: String,
}

/// Whether a grant belongs to a chat connected with /crew or to an agent task started from
/// Crew.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrewGrantKind {
    Chat,
    Task,
}

/// One grant, as `GET /crew/connections/{id}/grants` lists it: the registry's row, and what
/// the route adds for display. Nothing here decides a revocation.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewGrantView {
    #[serde(flatten)]
    pub grant: GrantRow,
    /// `task` when the grant is one of this computer's tasks, else `chat`; `null` when the
    /// task ledger could not be read.
    #[schema(inline)]
    pub kind: Option<CrewGrantKind>,
    /// The chat's title; `null` when the chat is gone or untitled, and always for a replaced
    /// grant, whose ID may name a different chat now.
    pub session_name: Option<String>,
}

/// `GET /crew/connections/{id}/grants`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewGrantList {
    /// The chats and tasks holding a grant on this connection.
    pub grants: Vec<CrewGrantView>,
    /// Earlier grants whose revocation the workspace has not confirmed yet (F3). The daemon
    /// keeps asking by itself.
    pub replaced_grants: Vec<CrewGrantView>,
}

/// `POST /crew/connections/{id}/sessions/{session_id}/revoke`: a grant stopped here and
/// confirmed by the workspace. One the workspace did not confirm is refused instead
/// (`crew_revocation_unconfirmed`).
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewRevocation {
    /// Always `true`.
    pub revoked: bool,
    pub session_id: String,
    pub run_id: String,
    /// Always `true`.
    pub remote_revocation_confirmed: bool,
    /// The workspace's revoked run, as it answered `run.revoke`.
    #[schema(value_type = Object)]
    pub run: Value,
    /// A task's grant: the task's status after the stop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_status: Option<String>,
    /// A task's grant: why its status could not be saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_status_error: Option<String>,
}

/// `DELETE /crew/files/{capability_id}`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewFileDiscarded {
    /// Always `true`, whether or not the selection was still pending.
    pub discarded: bool,
}

/// `GET /crew/transfers`.
#[derive(Serialize, ToSchema)]
pub struct CrewTransferList {
    pub transfers: Vec<Receipt>,
}

/// `DELETE /crew/transfers/{id}`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CrewTransferForgotten {
    /// Always `true`.
    pub forgotten: bool,
    /// What forgetting did, for a person.
    pub message: String,
}

/// `POST /crew/transfers/preview`: the image's bytes, verified against the attachment's
/// digest, with its own media type.
#[derive(Debug, ToSchema)]
pub struct CrewPreviewImage(#[schema(value_type = String, format = Binary)] pub Vec<u8>);

impl CrewPreviewImage {
    /// The answer: the bytes under their own media type, never cached or sniffed, and
    /// sandboxed should a browser open it directly.
    pub fn answer(self, media_type: &'static str) -> Response {
        (
            [
                ("content-type", media_type),
                ("cache-control", "no-store"),
                ("x-content-type-options", "nosniff"),
                ("content-security-policy", "default-src 'none'; sandbox"),
            ],
            self.0,
        )
            .into_response()
    }
}
