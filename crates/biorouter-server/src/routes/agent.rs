use crate::routes::errors::ErrorResponse;
use crate::routes::workflow_utils::{
    apply_workflow_to_agent, build_workflow_with_parameter_values, load_workflow_by_id,
    validate_workflow,
};
use crate::state::AppState;
use axum::response::IntoResponse;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use biorouter::agents::ExtensionLoadResult;

use base64::Engine;
use biorouter::agents::ExtensionConfig;
use biorouter::config::resolve_extensions_for_new_session;
use biorouter::config::{BioRouterMode, Config};
use biorouter::model::ModelConfig;
use biorouter::permission::{grade_tool, SmartApproveConfig};
use biorouter::privacy::refusal::PrivacyRefusal;
use biorouter::privacy::{raise_needs_user_action, ProviderTier, SessionClassification};
use biorouter::prompt_template::render_global_file;
use biorouter::providers::create;
use biorouter::session::extension_data::ExtensionState;
use biorouter::session::session_manager::SessionType;
use biorouter::session::{EnabledExtensionsState, Session, SessionManager, WorkingDirUpdate};
use biorouter::workflow::Workflow;
#[cfg(test)]
use biorouter::workflow::WorkflowKnowledgeBases;
use biorouter::workflow_deeplink;
use biorouter::{
    agents::{
        extension::ToolInfo,
        extension_manager::{get_parameter_names, normalize},
    },
    config::permission::PermissionLevel,
};
use biorouter_mcp::knowledge::service::KnowledgeService;
// Issue #56 DR-16. Named through the LIB path, not `crate::auth`: `src/routes/`
// is compiled into the `biorouterd` binary as well as the lib (see
// `routes::secret_matches`), and the digest is a process-global that must have
// exactly ONE instance — the lib's. `crate::auth` does not exist in the binary
// compilation, and a copy under `routes` would give the binary a second, empty
// static that `commands::agent` never installs into.
// Task 49 takes the THREE-way form beside the boolean one: a daemon that holds no
// user-action key must be reported differently from a caller that presented no
// proof (Task 18A's open question 23), and only `user_action_proof` can tell them
// apart. Both are defined in terms of the same one header, one key, one
// comparison — a second mechanism is what DR-18 refused.
use biorouter_server::auth::{is_user_action, user_action_proof, UserActionProof};
use rmcp::model::{CallToolRequestParams, Content};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

const PERMISSION_SETTINGS_SESSION_ID: &str = "__permission_settings__";
const PERMISSION_SETTINGS_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateFromSessionRequest {
    session_id: String,
}

const SUBAGENT_USER_ACTION_REQUIRED: &str =
    "Changing or resuming a subagent from its tab requires proof that the request came from the person at the keyboard.";

/// …and when the daemon holds no user-action key at all.
///
/// A separate sentence, per Task 18A's open question 23 and SD-8: telling a
/// person at a `biorouter serve` page that their request "requires proof" sends
/// them hunting for a permission this daemon can never grant anyone. It names
/// the daemon as the reason, in the register `CROSS_AFFILIATION_GRANT_NO_KEY`
/// and `session_reach::SESSION_REACH_NO_KEY` already use. Since SD-11 this is
/// also what a keyless daemon answers a Stop aimed at a subagent's turn, because
/// that route gates through [`authorize_agent_control`] there. A *steer* at the
/// same turn is refused one step earlier, by `reply::steer_refusal`, which
/// never reads the row — so the two sentences differ, and both open by naming
/// this daemon rather than the caller.
const SUBAGENT_CONTROL_NO_KEY: &str =
    "This daemon was started without a user-action key, so it cannot verify that a request came \
     from the person at the keyboard, and changing, resuming, stopping or steering a subagent from \
     its tab requires that proof. Nothing was changed. This control is unavailable on this \
     daemon; use the desktop app.";

/// A 424 in the shape every other refusal in this file has, so that gating a
/// route which used to answer bare status codes does not change the body a client
/// sees for the failures it already handled.
fn agent_not_initialized(message: &str) -> ErrorResponse {
    ErrorResponse {
        message: message.to_string(),
        status: StatusCode::FAILED_DEPENDENCY,
    }
}

fn refuse_subagent_unless_user(
    session: &Session,
    headers: &HeaderMap,
) -> Result<(), ErrorResponse> {
    if session.session_type != SessionType::SubAgent {
        return Ok(());
    }
    let message = match user_action_proof(headers) {
        UserActionProof::Proven => return Ok(()),
        UserActionProof::Unproven => SUBAGENT_USER_ACTION_REQUIRED,
        UserActionProof::NoKeyInstalled => SUBAGENT_CONTROL_NO_KEY,
    };
    Err(ErrorResponse {
        message: message.to_string(),
        status: StatusCode::FORBIDDEN,
    })
}

#[async_trait::async_trait]
trait AgentSessionReader {
    async fn read_session(
        &self,
        session_id: &str,
        include_messages: bool,
    ) -> anyhow::Result<Session>;
}

#[async_trait::async_trait]
impl AgentSessionReader for SessionManager {
    async fn read_session(
        &self,
        session_id: &str,
        include_messages: bool,
    ) -> anyhow::Result<Session> {
        self.get_session(session_id, include_messages).await
    }
}

fn resume_session_read_error(session_id: &str, error: anyhow::Error) -> ErrorResponse {
    error!("Failed to resume session {}: {}", session_id, error);
    ErrorResponse {
        message: format!("Failed to resume session: {}", error),
        status: StatusCode::NOT_FOUND,
    }
}

async fn read_resume_session(
    reader: &impl AgentSessionReader,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<Session, ErrorResponse> {
    let metadata = reader
        .read_session(session_id, false)
        .await
        .map_err(|error| resume_session_read_error(session_id, error))?;
    refuse_subagent_unless_user(&metadata, headers)?;
    reader
        .read_session(session_id, true)
        .await
        .map_err(|error| resume_session_read_error(session_id, error))
}

async fn read_update_session(
    reader: &impl AgentSessionReader,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<Session, ErrorResponse> {
    let session = reader
        .read_session(session_id, false)
        .await
        .map_err(|error| ErrorResponse {
            message: format!("Failed to get session: {}", error),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    refuse_subagent_unless_user(&session, headers)?;
    Ok(session)
}

/// Authorize an HTTP control-plane operation before it can touch an agent or a
/// queued child handle. The daemon bearer proves only that the caller reached
/// this process; it does not prove that a person chose to mutate a subagent.
///
/// ⚠ **Also the turn-control gate on a daemon with no user-action key** (SD-11):
/// `routes::reply`'s `authorize_turn_control` calls this for `/agent/cancel` and
/// the two continuation routes there, so that stopping a turn admits exactly the
/// callers `/agent/stop` admits. Tightening this therefore tightens those three
/// too, which is the point — but it is a change to who may press Stop in a
/// browser, and `tests/turn_control_no_user_key.rs` will say so. `/interrupt` is
/// NOT among them: `reply::steer_refusal` keeps the proof on every daemon,
/// because the dominance argument that admits a Stop does not reach a steer.
pub(crate) async fn authorize_agent_control(
    state: &AppState,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<Session, ErrorResponse> {
    crate::routes::session_reach::session_reach(state.session_manager(), session_id, headers)
        .await?;
    read_update_session(state.session_manager(), session_id, headers).await
}

async fn live_subagent_for_control(
    state: &AppState,
    session: &Session,
) -> Result<Arc<biorouter::agents::Agent>, ErrorResponse> {
    if biorouter::agents::subagent_handle::is_child_initializing(&session.id) {
        return Err(ErrorResponse {
            message: "Subagent runtime is not ready".into(),
            status: StatusCode::FAILED_DEPENDENCY,
        });
    }
    state
        .peek_agent(&session.id)
        .await
        .ok_or_else(|| ErrorResponse {
            message: "Subagent runtime is not ready".into(),
            status: StatusCode::FAILED_DEPENDENCY,
        })
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateProviderRequest {
    provider: String,
    model: Option<String>,
    session_id: String,
    context_limit: Option<usize>,
    request_params: Option<std::collections::HashMap<String, serde_json::Value>>,
}

/// The body of the 409 `/agent/update_provider` returns when a privacy boundary
/// refused the bind (issue #56, Gate A).
///
/// Typed rather than a bare string because the GUI does not merely report the
/// refusal — it renders §14.4's repair card from it, and the card names both
/// colliding tiers and offers the private models that would work. A plain-text
/// 409 would leave the renderer parsing prose.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PrivacyBarrierBody {
    /// Discriminator the client switches on. Always `privacy_barrier`.
    pub code: String,
    /// The classification of the chat that refused.
    pub session_classification: SessionClassification,
    /// The tier of the model that was offered.
    pub provider_tier: ProviderTier,
    /// The names of the providers a private chat *can* be switched to, so the
    /// card can offer the way forward rather than only the wall.
    pub available_private_providers: Vec<String>,
}

impl PrivacyBarrierBody {
    pub const CODE: &'static str = "privacy_barrier";
}

/// How `/agent/update_provider` failed, at the granularity the client needs.
///
/// Split out of the handler so the mapping can be tested without an
/// `AppState`: `AppState::new()` opens the REAL user session database (see the
/// note in `routes/session.rs`), so a route test must never go through it.
#[derive(Debug)]
pub(crate) enum ProviderBindFailure {
    /// A privacy boundary refused the bind — 409, with a body the GUI renders.
    Privacy(Box<PrivacyBarrierBody>),
    /// Anything else — 500, as before.
    Internal(String),
}

impl ProviderBindFailure {
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Privacy(_) => StatusCode::CONFLICT,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ProviderBindFailure {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        match self {
            Self::Privacy(body) => (status, Json(body)).into_response(),
            Self::Internal(message) => (status, message).into_response(),
        }
    }
}

/// Classify an `Agent::update_provider` failure.
///
/// The refusal is a typed error carried inside `anyhow::Error`, so this asks
/// with `downcast_ref` rather than matching on a message. Every other failure
/// keeps the pre-#56 behaviour — a 500 with the error's text.
///
/// The card needs the *pair* of colliding tiers, so only a refusal that carries
/// one is rendered as a barrier. `Agent::update_provider` produces exactly one
/// such refusal today; Task 18A's three HTTP-channel variants are about a
/// channel rather than a session collision, carry no pair, and are rendered at
/// their own handlers — they never travel through here.
pub(crate) fn classify_provider_bind_failure(
    error: &anyhow::Error,
    available_private_providers: Vec<String>,
) -> ProviderBindFailure {
    match error
        .downcast_ref::<PrivacyRefusal>()
        .and_then(|refusal| Some((refusal.session_classification()?, refusal.provider_tier()?)))
    {
        Some((session_classification, provider_tier)) => {
            ProviderBindFailure::Privacy(Box::new(PrivacyBarrierBody {
                code: PrivacyBarrierBody::CODE.to_string(),
                session_classification,
                provider_tier,
                available_private_providers,
            }))
        }
        None => ProviderBindFailure::Internal(format!("Failed to update provider: {error}")),
    }
}

/// Every registered provider whose shipped metadata claims Private — the models
/// a private chat may be switched to.
///
/// Read from the same registry the settings grid reads, so the card can never
/// offer a model the grid does not show.
async fn available_private_providers() -> Vec<String> {
    let mut names: Vec<String> = biorouter::providers::providers()
        .await
        .into_iter()
        .filter(|(metadata, _)| metadata.tier.is_private())
        .map(|(metadata, _)| metadata.name)
        .collect();
    names.sort();
    names
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct GetToolsQuery {
    extension_name: Option<String>,
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CallableToolCountQuery {
    session_id: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CallableToolCountResponse {
    count: usize,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct StartAgentRequest {
    working_dir: String,
    #[serde(default)]
    workflow: Option<Workflow>,
    #[serde(default)]
    workflow_id: Option<String>,
    #[serde(default)]
    workflow_deeplink: Option<String>,
    #[serde(default)]
    extension_overrides: Option<Vec<ExtensionConfig>>,
}

fn configured_new_session_provider() -> Result<Option<(String, ModelConfig)>, ErrorResponse> {
    let config = Config::global();
    match (
        config.get_biorouter_provider(),
        config.get_biorouter_model(),
    ) {
        (Ok(provider), Ok(model)) => {
            let model_config = ModelConfig::new(&model).map_err(|error| ErrorResponse {
                message: format!("The selected model cannot be used for a new chat: {error}"),
                status: StatusCode::BAD_REQUEST,
            })?;
            Ok(Some((provider, model_config)))
        }
        (Err(_), Err(_)) => Ok(None),
        (provider, model) => Err(ErrorResponse {
            message: format!(
                "The default provider selection is incomplete: provider={}, model={}",
                if provider.is_ok() { "set" } else { "missing" },
                if model.is_ok() { "set" } else { "missing" },
            ),
            status: StatusCode::BAD_REQUEST,
        }),
    }
}

/// Why a brand-new chat's bind to the operator's configured private provider may
/// not go ahead as it stands — SD-12's verdict, with the reason, because two of
/// the three refusals here are actionable by a *person* and a single boolean
/// would have answered all of them in a sentence written for a model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NewChatBind {
    /// Bind it.
    Allowed,
    /// This daemon can check a proof, and none arrived.
    NeedsUserProof,
    /// SD-12 would exempt this bind, but the configuration being read is no
    /// longer the one the daemon was launched with, so there is nothing left in
    /// it that is the operator's choice. Carries the key that moved.
    ConfigMovedSinceLaunch(String),
    /// The launcher declared it would hand over a user-action key and the key
    /// never arrived. A fault to repair, not a deployment where no proof can
    /// exist — so the exemption does not apply.
    KeyWasExpected,
}

/// SD-12 (`docs/deployment/serve-decisions.md`): does binding the operator's
/// configured default provider to a brand-new chat need a person's proof?
///
/// A new chat has no capability of its own yet, so a private default reads as a
/// raise from Public — the reading `update_agent_provider` gives any first bind.
/// What differs is who chose the model: `/agent/start` names no provider, it binds
/// `BIOROUTER_PROVIDER` + `BIOROUTER_MODEL`, and SD-1 says the tier the operator's
/// `biorouter configure` choice implies holds for every session in that daemon. So
/// the proof is asked for only where it can be given:
///
/// * `Proven` — the desktop renderer, which sends `X-User-Action` on every start
///   (`ui/desktop/src/sessions.ts`). Binds.
/// * `Unproven` — a daemon that holds a key, and a caller that did not present
///   it: a script, or the model holding the daemon secret AR-11 found
///   recoverable. Refused, as before. The person proves themselves here at no
///   cost, and without the refusal a model could mint a private-capability chat
///   with an extension set of its own choosing.
/// * `NoKeyInstalled` — `biorouter serve` (SD-7) or a hand-run `biorouterd`.
///   Nobody on this daemon can prove anything, so a refusal here refuses the
///   person too, on every new chat, always — the 2026-09-10 QA's F1. Binds, under
///   the two conditions below.
///
/// ⚠ **The original justification for that third arm was wrong, and both
/// conditions exist because of it** (2026-09-12 review, Findings 1 and 3):
///
/// 1. It said `BIOROUTER_PROVIDER` is *"a key only a proven person may write"*.
///    The HTTP doors are closed, but `config.yaml` is not an HTTP resource:
///    DR-14's filesystem deny is DEFERRED, the agent holds `developer__shell`, and
///    `Config` re-`stat`s and reloads the file — so the value read here is
///    whatever the file says at request time. A model with a shell on a keyless
///    daemon configured with a *public* default could write a private provider
///    there and take this exemption. So the exemption is pinned to the
///    configuration the daemon **launched** with (`launch::pinned_config_keys`);
///    drift refuses, and names the key.
/// 2. `NoKeyInstalled` is only *"this process read no valid digest"*, which a
///    desktop spawn also satisfies when its `userActionKey` is undefined or the
///    daemon's bounded 2s stdin read times out. That is a repairable fault, and on
///    `main` it degraded safely by refusing. It keeps refusing: the launcher
///    declares its intent (`launch::USER_ACTION_EXPECTED_ENV`), which is the only
///    signal that separates the two.
///
/// Only the configured default is ever exempt, and only at creation:
/// `/agent/start` cannot name another provider, and on a keyless daemon
/// `update_agent_provider` refuses every private bind ([`raise_baseline`]), so a
/// new chat there never reaches a private model the operator did not choose.
fn new_chat_bind_decision(
    enforced: bool,
    tier: ProviderTier,
    proof: UserActionProof,
    launcher_declared_a_key: bool,
    config_moved_since_launch: Option<String>,
) -> NewChatBind {
    // DR-15's master opt-out, and the plain reading that a public default raises
    // nothing for anybody. Neither is about who is asking.
    if !enforced || !raise_needs_user_action(ProviderTier::Public, tier) {
        return NewChatBind::Allowed;
    }
    match proof {
        UserActionProof::Proven => NewChatBind::Allowed,
        UserActionProof::Unproven => NewChatBind::NeedsUserProof,
        UserActionProof::NoKeyInstalled => {
            if launcher_declared_a_key {
                NewChatBind::KeyWasExpected
            } else if let Some(key) = config_moved_since_launch {
                NewChatBind::ConfigMovedSinceLaunch(key)
            } else {
                NewChatBind::Allowed
            }
        }
    }
}

/// The capability `update_agent_provider` measures a raise from — SD-12's other
/// half.
///
/// On a daemon that holds a user-action key it is the chat's live capability,
/// as it always was. On one that holds none, no chat's private capability came
/// from anything a person proved over HTTP: the only private binding such a
/// daemon hands out through its routes is [`new_chat_bind_decision`]'s
/// creation-time bind to the configured default. There is no private floor for
/// a request to build on, so a bind to ANY private provider is measured from
/// Public, and refused, since no proof can arrive. Without this, SD-12 would let
/// a new chat on a private default be moved sideways, `Private -> Private`, to a
/// private model nobody configured.
fn raise_baseline(current: ProviderTier, proof: UserActionProof) -> ProviderTier {
    match proof {
        UserActionProof::NoKeyInstalled => ProviderTier::Public,
        UserActionProof::Proven | UserActionProof::Unproven => current,
    }
}

async fn bind_new_session_provider(
    state: &AppState,
    session: &Session,
    headers: &HeaderMap,
) -> Result<(), ErrorResponse> {
    let Some((provider_name, model_config)) = configured_new_session_provider()? else {
        return Ok(());
    };
    let provider = create(&provider_name, model_config)
        .await
        .map_err(|error| ErrorResponse {
            message: format!("Failed to configure the selected provider for the new chat: {error}"),
            status: StatusCode::BAD_REQUEST,
        })?;
    // DR-15's master opt-out is read inside the gate, as every #56 surface does.
    // The launch state is read here rather than in the gate for the same reason:
    // one sample per request, threaded, so the decision cannot be made against two
    // different answers.
    match new_chat_bind_decision(
        biorouter::privacy::privacy_tiers_enabled(),
        provider.tier(),
        user_action_proof(headers),
        biorouter_server::launch::expected_a_user_action_key(),
        biorouter_server::launch::capability_config_moved_since_launch(),
    ) {
        NewChatBind::Allowed => {}
        NewChatBind::NeedsUserProof => {
            return Err(ErrorResponse {
                message: PrivacyRefusal::TierRaiseNeedsUser {
                    requested: provider_name,
                }
                .to_string(),
                status: StatusCode::CONFLICT,
            });
        }
        // Written for the operator, not for the model. SD-8's rule: a control the
        // caller cannot pass must say what would make it passable, and here that
        // is a restart — no proof exists on this daemon to offer instead.
        NewChatBind::ConfigMovedSinceLaunch(key) => {
            return Err(ErrorResponse {
                message: format!(
                    "This Biorouter daemon starts new chats on the model it was launched with, \
                     because nothing here can confirm a request came from you. '{key}' has \
                     changed in the configuration since it started, so '{provider_name}' is not \
                     the model it was launched on and starting a chat on it would be a switch \
                     nobody asked for. Restart the daemon to pick up the new configuration."
                ),
                status: StatusCode::CONFLICT,
            });
        }
        NewChatBind::KeyWasExpected => {
            return Err(ErrorResponse {
                message: format!(
                    "Biorouter cannot confirm that this request came from you: the application \
                     that started this daemon was meant to hand it a user-action key and none \
                     arrived, so a new chat on the private model '{provider_name}' is refused \
                     rather than started. Quit and reopen Biorouter. If it keeps happening, the \
                     daemon log records 'no user-action key on stdin'."
                ),
                status: StatusCode::CONFLICT,
            });
        }
    }
    let agent = state
        .get_agent(session.id.clone())
        .await
        .map_err(|error| ErrorResponse {
            message: format!("Failed to prepare the new chat agent: {error}"),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    agent
        .update_provider(provider, &session.id)
        .await
        .map_err(|error| ErrorResponse {
            message: format!("Failed to bind the selected provider to the new chat: {error}"),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })
}

async fn discard_failed_new_session(state: &AppState, session_id: &str) {
    if let Err(error) = state.agent_manager.remove_session(session_id).await {
        tracing::debug!(session_id, %error, "New chat had no cached agent to discard");
    }
    if let Err(error) = state.session_manager().delete_session(session_id).await {
        tracing::warn!(session_id, %error, "Failed to discard a new chat after provider binding failed");
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct StopAgentRequest {
    session_id: String,
}

/// Turn a workflow's `{ default, visible }` into "which bases the session
/// should hold" and "what its primary should be".
///
/// `WorkflowKnowledgeBases` already expresses a set plus one primary, which is
/// exactly the session model — so this is a translation, not a schema change.
/// It deliberately does **not** consult the installed bases: inverting the set
/// against an inventory is the service's job, inside the lock (see
/// [`apply_workflow_knowledge_selection`]).
///
/// **The primary comes only from `default`.** A workflow that lists bases
/// without naming one is a workflow with no primary, whatever the set size.
/// Promoting a sole visible member looked harmless — "there is only one
/// candidate, so it cannot be the wrong one" — but the merged model forbids
/// *inventing* the pointer at all: it is the target of KB-less writes, so a
/// promoted primary silently turns "I did not say where to write" into a
/// commit into someone's base. With no primary, the write fails and names the
/// candidates; the author who wanted the write target says so in `default`.
///
/// One rule still earns its keep: a `default` that was not listed in `visible`
/// is unioned into the set, because the invariant requires the primary to be a
/// member and the author plainly meant it.
#[cfg(test)]
pub(crate) fn plan_workflow_knowledge_selection(
    selection: &WorkflowKnowledgeBases,
) -> (Vec<String>, Option<String>) {
    biorouter::workflow::runtime::plan_knowledge_selection(selection)
}

/// Install a workflow's declared selection into the session it just created.
///
/// The whole gesture is one root-locked service call. It used to list the
/// installed bases here, invert them against `visible` to get a hidden list,
/// and only then take the lock to write it — so a base created by any other
/// surface in that window was absent from the hidden list it wrote, and joined
/// a workflow session the workflow had never declared it in. `set_visible_kbs`
/// takes the visible set and does the inversion *inside* the lock, against the
/// inventory as it is at the moment of the write.
fn apply_workflow_knowledge_selection(
    svc: &KnowledgeService,
    session_id: &str,
    workflow: &Workflow,
) -> Result<(), ErrorResponse> {
    biorouter::workflow::runtime::apply_knowledge_selection(svc, session_id, workflow).map_err(
        |err| {
            error!("Failed to apply workflow knowledge bases: {}", err);
            ErrorResponse {
                message: format!("Failed to apply workflow knowledge bases: {}", err),
                status: StatusCode::BAD_REQUEST,
            }
        },
    )
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RestartAgentRequest {
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateWorkingDirRequest {
    session_id: String,
    working_dir: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ResumeAgentRequest {
    session_id: String,
    load_model_and_extensions: bool,
    /// Stable per-window label used only to rehydrate that same window's
    /// pending Stop-and-Send lease. It is not an authentication credential.
    #[serde(default)]
    continuation_owner_id: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct AddExtensionRequest {
    session_id: String,
    config: ExtensionConfig,
}

/// Issue #56 Task 49 (DR-26): the user accepting ONE cross-institutional data
/// flow.
///
/// ⚠ **There is no `affiliation` field, and there must not be one.** The grant is
/// keyed on the affiliation of the model bound *right now*, read by the daemon
/// from the same sample that produced the warning. A client-supplied institution
/// would let a caller record an acceptance for a triple the user was never shown
/// — which is the one thing this control exists to prevent.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CrossAffiliationGrantRequest {
    session_id: String,
    /// The extension whose cross-institutional flow the user accepted. Any
    /// spelling the UI holds; it is `name_to_key`-normalised on both sides.
    extension: String,
}

/// What was accepted, echoed back so the caller can record the exact sentence the
/// user was shown rather than a paraphrase of it.
#[derive(Serialize, utoipa::ToSchema)]
pub struct CrossAffiliationGrantResponse {
    /// The full statement, including the scope of the approval
    /// (`privacy::grant::GRANT_SCOPE_COPY`).
    accepted: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RemoveExtensionRequest {
    name: String,
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ReadResourceRequest {
    session_id: String,
    extension_name: String,
    uri: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadResourceResponse {
    uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    text: String,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    meta: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CallToolRequest {
    session_id: String,
    name: String,
    arguments: Value,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CallToolResponse {
    content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    structured_content: Option<Value>,
    is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    _meta: Option<Value>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ResumeAgentResponse {
    pub session: Session,
    /// True only while a delegated child is queued before its exact runtime
    /// profile has been installed. Clients must wait instead of fetching tools
    /// or submitting a turn through a generic placeholder agent.
    pub initializing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_results: Option<Vec<ExtensionLoadResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initialization_error: Option<AgentInitializationError>,
    /// The turn in flight for this session right now, if any, so a window that
    /// has just reloaded can re-attach to it (`POST /reply` with this
    /// `turn_id`).
    ///
    /// The alternative is a pointer the client publishes through `localStorage`
    /// and carries across the reload itself — and a pointer the client keeps is
    /// a pointer that can go stale, which makes "attached to a turn that no
    /// longer exists" a category of bug. The server always knows; asking it
    /// removes the class. Absent means the session is idle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<ActiveTurnRef>,
    /// A Stop-and-Send admission whose exact retired generation still awaits a
    /// successor. The opaque token is present only when this resume request's
    /// stable owner id matches the recorded owner; foreign windows receive only
    /// enough metadata to offer explicit takeover or group abandonment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_continuation: Option<PendingContinuationRef>,
}

#[derive(Debug, Clone, Copy, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PendingContinuationOwnership {
    Owned,
    Foreign,
    Settling,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PendingContinuationRef {
    pub ownership: PendingContinuationOwnership,
    pub superseded_turn_id: String,
    /// Returned only to the same stable owner after the lease is fully live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_lease: Option<String>,
}

/// A pointer to a live turn, handed to a client that needs to attach to it.
#[derive(Serialize, utoipa::ToSchema)]
pub struct ActiveTurnRef {
    /// Post this as `ChatRequest.turn_id` to attach. `POST /reply` accepts
    /// either this server-assigned id or the idempotency key the turn's original
    /// caller chose, so a client can use whichever it happens to hold.
    pub turn_id: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct AgentInitializationError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RestartAgentResponse {
    pub extension_results: Vec<ExtensionLoadResult>,
}

#[utoipa::path(
    post,
    path = "/agent/start",
    request_body = StartAgentRequest,
    responses(
        (status = 200, description = "Agent started successfully", body = Session),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 409, description = "The configured provider is private and this daemon will not bind it to a new chat as things stand (SD-12). Either the daemon holds a user-action key and the request carried no proof it came from the user; or it holds none but its launcher declared it would send one, so the missing key is a fault rather than a deployment where no proof can exist; or it holds none and a capability-deciding configuration key has changed since it started, in which case the message names the key and asks for a restart. A daemon with no user-action key, launched without that declaration, binds the provider it was launched with and needs no proof.", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
#[allow(clippy::too_many_lines)]
async fn start_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<StartAgentRequest>,
) -> Result<Json<Session>, ErrorResponse> {
    let StartAgentRequest {
        working_dir,
        workflow,
        workflow_id,
        workflow_deeplink,
        extension_overrides,
    } = payload;

    let original_workflow = if let Some(deeplink) = workflow_deeplink {
        match workflow_deeplink::decode(&deeplink) {
            Ok(workflow) => Some(workflow),
            Err(err) => {
                error!("Failed to decode workflow deeplink: {}", err);
                return Err(ErrorResponse {
                    message: err.to_string(),
                    status: StatusCode::BAD_REQUEST,
                });
            }
        }
    } else if let Some(id) = workflow_id {
        match load_workflow_by_id(&id) {
            Ok(workflow) => Some(workflow),
            Err(err) => return Err(err),
        }
    } else {
        workflow
    };

    if let Some(ref workflow) = original_workflow {
        if let Err(err) = validate_workflow(workflow) {
            return Err(ErrorResponse {
                message: err.message,
                status: err.status,
            });
        }
    }

    // Always create new sessions with the same human-friendly placeholder.
    // The numbered variant ("New session 154") leaked process-internal counter
    // state into the UI and made it hard for the frontend to recognize a name
    // as still being the default. Whether this session has been named is now
    // tracked exclusively via `user_set_name` plus a name-vs-default check.
    let name = biorouter::session::DEFAULT_SESSION_NAME.to_string();

    let manager = state.session_manager();

    let mut session = manager
        .create_session(PathBuf::from(&working_dir), name, SessionType::User)
        .await
        .map_err(|err| {
            error!("Failed to create session: {}", err);
            ErrorResponse {
                message: format!("Failed to create session: {}", err),
                status: StatusCode::BAD_REQUEST,
            }
        })?;

    if let Err(error) = bind_new_session_provider(&state, &session, &headers).await {
        discard_failed_new_session(&state, &session.id).await;
        return Err(error);
    }

    let prepared_workflow_prompt = if let Some(workflow) = original_workflow.as_ref() {
        match biorouter::workflow::runtime::prepare_prompt(manager, &session.id, workflow).await {
            Ok(prompt) => prompt,
            Err(error) => {
                discard_failed_new_session(&state, &session.id).await;
                return Err(ErrorResponse {
                    message: error.to_string(),
                    status: StatusCode::BAD_REQUEST,
                });
            }
        }
    } else {
        None
    };

    if let Some(workflow) = original_workflow.as_ref() {
        // The siblings' shape, and for the same reason. A bare `?` here left the
        // chat this function had just created sitting in the session list — a
        // row the user never asked for and cannot explain. It is not a rare
        // race, either: a workflow whose `default` names a base that has since
        // been deleted fails here on EVERY start, so a stale workflow minted one
        // orphan per press.
        if let Err(error) =
            apply_workflow_knowledge_selection(&state.knowledge_service, &session.id, workflow)
        {
            discard_failed_new_session(&state, &session.id).await;
            return Err(error);
        }
    }

    let workflow_extensions = original_workflow
        .as_ref()
        .and_then(|r| r.extensions.as_deref());
    let mut extensions_to_use =
        resolve_extensions_for_new_session(workflow_extensions, extension_overrides);
    if let Some(workflow) = original_workflow.as_ref() {
        biorouter::workflow::runtime::ensure_required_extensions(workflow, &mut extensions_to_use);
    }
    let mut extension_data = session.extension_data.clone();
    let extensions_state = EnabledExtensionsState::new(extensions_to_use);
    if let Err(e) = extensions_state.to_extension_data(&mut extension_data) {
        tracing::warn!("Failed to initialize session with extensions: {}", e);
    } else {
        manager
            .update(&session.id)
            .extension_data(extension_data.clone())
            .apply()
            .await
            .map_err(|err| {
                error!("Failed to save initial extension state: {}", err);
                ErrorResponse {
                    message: format!("Failed to save initial extension state: {}", err),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                }
            })?;
    }

    if let Some(workflow) = original_workflow.clone() {
        manager
            .update(&session.id)
            .workflow(Some(workflow))
            .apply()
            .await
            .map_err(|err| {
                error!("Failed to update session with workflow: {}", err);
                ErrorResponse {
                    message: format!("Failed to update session with workflow: {}", err),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                }
            })?;
    }

    // Refetch session to get all updates
    session = manager
        .get_session(&session.id, false)
        .await
        .map_err(|err| {
            error!("Failed to get updated session: {}", err);
            ErrorResponse {
                message: format!("Failed to get updated session: {}", err),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;

    // Eagerly start loading extensions in the background
    let session_for_spawn = session.clone();
    let state_for_spawn = state.clone();
    let session_id_for_task = session.id.clone();
    let workflow_for_spawn = original_workflow;
    let task = tokio::spawn(async move {
        match state_for_spawn
            .get_agent(session_for_spawn.id.clone())
            .await
        {
            Ok(agent) => {
                let results = agent.load_extensions_from_session(&session_for_spawn).await;
                let context: HashMap<&str, Value> = HashMap::new();
                let desktop_prompt = render_global_file("desktop_prompt.md", &context)
                    .expect("Prompt should render");
                if let Some(workflow) = workflow_for_spawn.as_ref() {
                    biorouter::workflow::runtime::apply_prepared_to_agent(
                        agent.as_ref(),
                        workflow,
                        true,
                        prepared_workflow_prompt.clone(),
                    )
                    .await;
                    if prepared_workflow_prompt.is_none() {
                        agent.set_session_context_prompt(Some(desktop_prompt)).await;
                    }
                } else {
                    agent.set_session_context_prompt(Some(desktop_prompt)).await;
                }
                tracing::debug!(
                    "Background extension loading completed for session {}",
                    session_for_spawn.id
                );
                results
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create agent for background extension loading: {}",
                    e
                );
                vec![]
            }
        }
    });

    state
        .set_extension_loading_task(session_id_for_task, task)
        .await;

    Ok(Json(session))
}

fn validate_continuation_owner_id(owner_id: Option<&str>) -> Result<(), ErrorResponse> {
    let invalid = owner_id.is_some_and(|owner_id| {
        owner_id.is_empty()
            || owner_id.len() > 128
            || !owner_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
    });
    if invalid {
        return Err(ErrorResponse {
            message: "Invalid continuation owner id".to_string(),
            status: StatusCode::BAD_REQUEST,
        });
    }
    Ok(())
}

async fn restore_resumed_subagent_profile(
    state: &AppState,
    session_id: &str,
    agent: &Arc<biorouter::agents::Agent>,
    session: &Session,
) -> (
    Option<Vec<ExtensionLoadResult>>,
    Option<AgentInitializationError>,
) {
    match agent.restore_subagent_runtime_profile(session).await {
        Ok(true) => (Some(Vec::new()), None),
        Ok(false) => {
            let _ = state.agent_manager.remove_session(session_id).await;
            (
                None,
                Some(AgentInitializationError {
                    code: "subagent_runtime_profile_missing".into(),
                    message:
                        "Biorouter could not restore the delegated runtime profile for this subagent."
                            .into(),
                    retryable: false,
                }),
            )
        }
        Err(error) => {
            let _ = state.agent_manager.remove_session(session_id).await;
            tracing::error!(
                "Failed to restore delegated runtime profile for session {}: {}",
                session_id,
                error
            );
            (
                None,
                Some(AgentInitializationError {
                    code: "subagent_runtime_profile_restore_failed".into(),
                    message: error.to_string(),
                    retryable: false,
                }),
            )
        }
    }
}

async fn load_resumed_agent_extensions(
    state: &AppState,
    payload: &ResumeAgentRequest,
    session: &Session,
    child_initializing: bool,
) -> (
    Option<Vec<ExtensionLoadResult>>,
    Option<AgentInitializationError>,
) {
    if child_initializing || !payload.load_model_and_extensions {
        return (None, None);
    }

    let agent = match state.get_agent_for_route(payload.session_id.clone()).await {
        Ok(agent) => agent,
        Err(status) => {
            tracing::error!(
                "Failed to prepare agent for session {}: {}",
                payload.session_id,
                status
            );
            return (
                None,
                Some(AgentInitializationError {
                    code: "agent_unavailable".into(),
                    message: "Biorouter could not prepare the model agent for this session.".into(),
                    retryable: status.is_server_error(),
                }),
            );
        }
    };

    if let Err(error) = agent.restore_persisted_provider_if_missing(session).await {
        if session.session_type == SessionType::SubAgent {
            let _ = state
                .agent_manager
                .remove_session(&payload.session_id)
                .await;
        }
        tracing::error!(
            "Failed to restore provider for session {}: {}",
            payload.session_id,
            error
        );
        return (
            None,
            Some(AgentInitializationError {
                code: "provider_restore_failed".into(),
                message: error.to_string(),
                retryable: false,
            }),
        );
    }

    if session.session_type == SessionType::SubAgent {
        return restore_resumed_subagent_profile(state, &payload.session_id, &agent, session).await;
    }

    let results =
        if let Some(results) = state.take_extension_loading_task(&payload.session_id).await {
            tracing::debug!(
                "Using background extension loading results for session {}",
                payload.session_id
            );
            state
                .remove_extension_loading_task(&payload.session_id)
                .await;
            results
        } else {
            tracing::debug!(
                "No background task found, loading extensions for session {}",
                payload.session_id
            );
            agent.load_extensions_from_session(session).await
        };
    (Some(results), None)
}

#[utoipa::path(
    post,
    path = "/agent/resume",
    request_body = ResumeAgentRequest,
    responses(
        (status = 200, description = "Agent started successfully", body = ResumeAgentResponse),
        (status = 400, description = "Bad request - invalid working directory"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The session is out of reach, or the target is a subagent and the request lacks user-action proof"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn resume_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ResumeAgentRequest>,
) -> Result<Json<ResumeAgentResponse>, ErrorResponse> {
    validate_continuation_owner_id(payload.continuation_owner_id.as_deref())?;
    crate::routes::session_reach::session_reach(
        state.session_manager(),
        &payload.session_id,
        &headers,
    )
    .await?;

    let session =
        read_resume_session(state.session_manager(), &payload.session_id, &headers).await?;

    // A queued background child has a durable session row but not yet its
    // delegated runtime profile. Constructing a generic cached Agent here would
    // expose the wrong tools/provider and let a reply race for its initial turn.
    // Return the session metadata normally; the delegated runner registers the
    // exact live Agent once its permit and atomic profile are ready.
    let child_initializing = session.session_type == SessionType::SubAgent
        && biorouter::agents::subagent_handle::is_child_initializing(&payload.session_id);
    let (extension_results, initialization_error) =
        load_resumed_agent_extensions(&state, &payload, &session, child_initializing).await;

    let active_turn = state
        .active_turn_id(&payload.session_id)
        .map(|turn_id| ActiveTurnRef { turn_id });
    let pending_continuation = state
        .pending_continuation_for_owner(
            &payload.session_id,
            payload.continuation_owner_id.as_deref(),
        )
        .map(|pending| PendingContinuationRef {
            ownership: match pending.ownership {
                crate::state::PendingContinuationOwnership::Owned => {
                    PendingContinuationOwnership::Owned
                }
                crate::state::PendingContinuationOwnership::Foreign => {
                    PendingContinuationOwnership::Foreign
                }
                crate::state::PendingContinuationOwnership::Settling => {
                    PendingContinuationOwnership::Settling
                }
            },
            superseded_turn_id: pending.superseded_turn_id,
            continuation_lease: pending.continuation_lease,
        });

    Ok(Json(ResumeAgentResponse {
        session,
        initializing: child_initializing,
        extension_results,
        initialization_error,
        active_turn,
        pending_continuation,
    }))
}

#[utoipa::path(
    post,
    path = "/agent/update_from_session",
    request_body = UpdateFromSessionRequest,
    responses(
        (status = 200, description = "Update agent from session data successfully"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The session is out of reach, or the target is a subagent and the request lacks user-action proof"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error"),
    ),
)]
async fn update_from_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateFromSessionRequest>,
) -> Result<StatusCode, ErrorResponse> {
    crate::routes::session_reach::session_reach(
        state.session_manager(),
        &payload.session_id,
        &headers,
    )
    .await?;

    let session =
        read_update_session(state.session_manager(), &payload.session_id, &headers).await?;

    if session.session_type == SessionType::SubAgent {
        return Ok(StatusCode::OK);
    }

    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await
        .map_err(|status| ErrorResponse {
            message: format!("Failed to get agent: {}", status),
            status,
        })?;
    let context: HashMap<&str, Value> = HashMap::new();
    let desktop_prompt =
        render_global_file("desktop_prompt.md", &context).expect("Prompt should render");
    let mut update_prompt = desktop_prompt;
    if let Some(workflow) = session.workflow {
        match build_workflow_with_parameter_values(
            &workflow,
            session.user_workflow_values.unwrap_or_default(),
        )
        .await
        {
            Ok(Some(workflow)) => {
                if let Some(prompt) =
                    apply_workflow_to_agent(&agent, &payload.session_id, &workflow, true)
                        .await
                        .map_err(|e| ErrorResponse {
                            message: e.to_string(),
                            status: StatusCode::BAD_REQUEST,
                        })?
                {
                    update_prompt = prompt;
                }
            }
            Ok(None) => {
                // Workflow has missing parameters - use default prompt
            }
            Err(e) => {
                return Err(ErrorResponse {
                    message: e.to_string(),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                });
            }
        }
    }
    agent.set_session_context_prompt(Some(update_prompt)).await;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/agent/tools",
    params(
        ("extension_name" = Option<String>, Query, description = "Optional extension name to filter tools"),
        ("session_id" = String, Query, description = "Session ID used to inspect active tools; pass an empty string to inspect one globally enabled extension from settings")
    ),
    responses(
        (status = 200, description = "Tools retrieved successfully", body = Vec<ToolInfo>),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused by a privacy boundary: `session_id` names a chat \
                                      this caller may not reach, answered with the same refusal, \
                                      word for word, that `GET /sessions/{session_id}` gives \
                                      (body = plain text)"),
        (status = 408, description = "Extension timed out while loading for settings"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn get_tools(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GetToolsQuery>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    // Issue #56, QA 2026-09-10 M2. Naming a private chat here handed a caller
    // holding only the daemon secret that chat's private-extension tool names,
    // while `add_extension` on the same chat refused it — and, worse, `get_agent`
    // below MINTS an agent for the named chat, loading its extensions, on that
    // caller's say-so. So the read's own gate runs first. The comment further
    // down, about Gate E, is about which tools a MODEL is shown; this is about
    // whether the CALLER may address the chat at all, and the empty id — the
    // settings page's one global extension — names no chat and is not gated.
    if !query.session_id.is_empty() {
        if let Err(refusal) = crate::routes::session_reach::session_reach(
            state.session_manager(),
            &query.session_id,
            &headers,
        )
        .await
        {
            return refusal.into_response();
        }
    }
    permission_editor_tools(state, query).await.into_response()
}

/// The body of [`get_tools`], once the caller may address the named chat.
async fn permission_editor_tools(
    state: Arc<AppState>,
    query: GetToolsQuery,
) -> Result<Json<Vec<ToolInfo>>, StatusCode> {
    let config = Config::global();
    let biorouter_mode = config.get_biorouter_mode().unwrap_or(BioRouterMode::Auto);
    let session_id = query.session_id;
    let extension_name = query.extension_name.map(|name| normalize(&name));
    let agent_session_id = if session_id.is_empty() {
        PERMISSION_SETTINGS_SESSION_ID
    } else {
        &session_id
    };
    let child_initializing = !session_id.is_empty()
        && biorouter::agents::subagent_handle::is_child_initializing(&session_id);
    let agent = if child_initializing {
        // A queued child has a durable row before its delegated runtime is
        // installed. Creating here would cache an ordinary placeholder and let
        // it win the later child-tab resume race.
        state
            .peek_agent(&session_id)
            .await
            .ok_or(StatusCode::FAILED_DEPENDENCY)?
    } else {
        state
            .get_agent_for_route(agent_session_id.to_string())
            .await?
    };

    if session_id.is_empty() {
        let Some(extension_name) = extension_name.as_deref() else {
            return Ok(Json(Vec::new()));
        };
        let Some(extension) = biorouter::config::get_all_extensions()
            .into_iter()
            .find(|entry| entry.enabled && entry.config.key() == extension_name)
        else {
            return Ok(Json(Vec::new()));
        };

        agent
            .remove_extension(extension_name)
            .await
            .map_err(|error| {
                warn!(extension = extension_name, %error, "Failed to refresh permission settings extension");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        tokio::time::timeout(
            PERMISSION_SETTINGS_LOAD_TIMEOUT,
            agent.add_extension(extension.config),
        )
        .await
        .map_err(|_| {
            warn!(extension = extension_name, "Timed out loading extension for permission settings");
            StatusCode::REQUEST_TIMEOUT
        })?
        .map_err(|error| {
            warn!(extension = extension_name, %error, "Failed to load extension for permission settings");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    let permission_manager = agent.config.permission_manager.clone();
    // BR-18: SmartApprove now auto-approves read-only-annotated tools, so the
    // level shown here has to reflect the risk grade the inspector will actually
    // apply — otherwise the settings UI keeps claiming a plain file read will
    // prompt. Graded from the same annotations, through the same predicate the
    // inspector uses, so the two cannot disagree.
    let smart = SmartApproveConfig::from_config();

    // Issue #56 Task 16: the permission editor's list, NOT the model's. This
    // route is what Settings → Extensions → tool permissions renders, and a
    // private extension has to stay visible and badged there whatever model
    // happens to be bound — including on the empty-`session_id` branch above,
    // which loads one globally enabled extension purely so its tools can be
    // listed here. Gate E is about the model's context; this is the user's own
    // administration surface. Do not swap this for `list_tools`.
    let mut tools: Vec<ToolInfo> = agent
        .list_tools_for_permission_settings(&session_id, extension_name)
        .await
        .into_iter()
        .map(|tool| {
            let permission = permission_manager
                .get_user_permission(&tool.name)
                .or_else(|| {
                    if biorouter_mode == BioRouterMode::SmartApprove {
                        permission_manager
                            .get_smart_approve_permission(&tool.name)
                            .or_else(|| {
                                smart.enabled.then(|| {
                                    if smart.requires_confirmation(grade_tool(&tool)) {
                                        PermissionLevel::AskBefore
                                    } else {
                                        PermissionLevel::AlwaysAllow
                                    }
                                })
                            })
                    } else if biorouter_mode == BioRouterMode::Approve {
                        Some(PermissionLevel::AskBefore)
                    } else {
                        None
                    }
                });

            ToolInfo::new(
                &tool.name,
                tool.description
                    .as_ref()
                    .map(|d| d.as_ref())
                    .unwrap_or_default(),
                get_parameter_names(&tool),
                permission,
            )
        })
        .collect::<Vec<ToolInfo>>();
    tools.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(tools))
}

#[utoipa::path(
    get,
    path = "/agent/callable_tool_count",
    params(
        ("session_id" = String, Query, description = "Active session whose model-visible tools should be counted")
    ),
    responses(
        (status = 200, description = "Model-visible callable tool count", body = CallableToolCountResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 Task 58 / #47): \
                                      the named chat is private (or absent, and an unproven caller \
                                      is told the same thing for both) and the request carried \
                                      neither a capability that covers it nor proof it came from \
                                      the user. It is the same refusal, word for word, that \
                                      `GET /sessions/{session_id}` gives (body = plain text)"),
        (status = 424, description = "Agent not initialized")
    )
)]
async fn get_callable_tool_count(
    State(state): State<Arc<AppState>>,
    // Before `Query`, which is fine either way here, but keeps the extractor
    // order the rest of this file uses.
    headers: axum::http::HeaderMap,
    Query(query): Query<CallableToolCountQuery>,
) -> axum::response::Response {
    // Issue #56 Task 58 / #47, and QA 2026-09-10 M2's sibling. FIRST, before the
    // agent is fetched, for the reason `agent_add_extension` states at length:
    // `get_agent_for_route` CREATES an agent for a session that has none, so a
    // gate below it would let an unproven caller materialise one for a chat it may
    // not address — and this route's own 424 would then tell it what it had found.
    // `session_id` is a request parameter, not a credential; see
    // `routes::session_reach`.
    //
    // ⚠ This route had NO gate of any kind, and PR #260's renderer merely stopped
    // calling it for a subagent's chat, which left the route exactly as open as
    // it was. Routing a client around an ungated route does not gate it.
    //
    // ⚠ **The refusal is returned through `SessionOutOfReach`'s own
    // `IntoResponse` — PLAIN TEXT, the bytes `GET /sessions/{session_id}`
    // returns — and deliberately NOT with `?` through this route's
    // `ErrorResponse`, which would wrap the same words in a JSON envelope.** One
    // boundary has one body (see the module header of `routes::session_reach`),
    // and that is the only reason the gate lives in this wrapper and the work
    // lives in the function below rather than all in one body — `get_tools`
    // beside it has the same shape for the same reason. A caller must not be
    // able to tell the gated routes apart by their envelopes, which is what
    // `every_route_that_names_a_private_chat_refuses_it_exactly_as_the_read_does`
    // measures: it fails on the wrapping alone, with the words unchanged.
    if let Err(refusal) = crate::routes::session_reach::session_reach(
        state.session_manager(),
        &query.session_id,
        &headers,
    )
    .await
    {
        return refusal.into_response();
    }
    model_visible_tool_count(state, query).await.into_response()
}

/// The body of [`get_callable_tool_count`], once the caller may address the chat.
async fn model_visible_tool_count(
    state: Arc<AppState>,
    query: CallableToolCountQuery,
) -> Result<Json<CallableToolCountResponse>, ErrorResponse> {
    let session_id = query.session_id;
    let child_initializing = biorouter::agents::subagent_handle::is_child_initializing(&session_id);
    let agent = if child_initializing {
        state
            .peek_agent(&session_id)
            .await
            .ok_or_else(|| agent_not_initialized("that chat's runtime is not ready yet"))?
    } else {
        state
            .get_agent_for_route(session_id.clone())
            .await
            .map_err(|status| ErrorResponse {
                message: "could not load that chat".to_string(),
                status,
            })?
    };

    // This endpoint drives a model-context warning. Count the final model-facing
    // surface, after Gate E, frontend additions, Code Execution narrowing,
    // and coding-agent bridge replacement. `/agent/tools` deliberately remains
    // the unfiltered permission-editor surface so a human can administer private
    // tools a public model cannot see.
    let count = agent
        .callable_tool_count(&session_id)
        .await
        .map_err(|_| agent_not_initialized("that chat's tools could not be counted"))?;
    Ok(Json(CallableToolCountResponse { count }))
}

#[utoipa::path(
    post,
    path = "/agent/update_provider",
    request_body = UpdateProviderRequest,
    responses(
        (status = 200, description = "Provider updated. The body is DR-26's cross-institutional \
                                      warning for this chat on the model just bound: the \
                                      statement the user is shown before proceeding, warnings \
                                      separated by a blank line, and is EMPTY when the bind \
                                      crosses no institutional boundary, which is the normal \
                                      case.",
                       body = String),
        (status = 400, description = "Bad request - missing or invalid parameters"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The session is out of reach, or the target is a subagent and the request lacks user-action proof"),
        (status = 409, description = "Refused by a privacy boundary (issue #56). Gate A: \
                                      a public model cannot be bound to a private chat \
                                      (body = PrivacyBarrierBody). DR-16: the bind raises this \
                                      chat's capability to Private and the request carried no \
                                      proof it came from the user; on a daemon with no \
                                      user-action key, any bind to a private model (SD-12) \
                                      (body = plain text)",
                       body = PrivacyBarrierBody),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn update_agent_provider(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(payload): Json<UpdateProviderRequest>,
) -> Result<String, axum::response::Response> {
    let session = authorize_agent_control(&state, &payload.session_id, &headers)
        .await
        .map_err(IntoResponse::into_response)?;
    let agent = if session.session_type == SessionType::SubAgent {
        live_subagent_for_control(&state, &session)
            .await
            .map_err(IntoResponse::into_response)?
    } else {
        state
            .get_agent_for_route(payload.session_id.clone())
            .await
            .map_err(|e| (e, "No agent for session id".to_owned()).into_response())?
    };

    let config = Config::global();
    let model = match payload.model.or_else(|| config.get_biorouter_model().ok()) {
        Some(m) => m,
        None => {
            return Err((StatusCode::BAD_REQUEST, "No model specified".to_owned()).into_response());
        }
    };

    let model_config = ModelConfig::new(&model)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid model config: {}", e),
            )
                .into_response()
        })?
        .with_context_limit(payload.context_limit)
        .with_request_params(payload.request_params);

    let new_provider = create(&payload.provider, model_config).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to create {} provider: {}", &payload.provider, e),
        )
            .into_response()
    })?;

    // Issue #56 DR-26, taken from the provider this route CREATED rather than
    // re-read off the agent afterwards. It is the key half of the triple the
    // grant lookup below is done on, and `update_provider` reassigns the provider
    // mutex with no turn lock — so a second read could key the lookup on a model
    // some other caller bound in between and suppress a warning on an acceptance
    // that was never given for this model.
    let model_affiliation = new_provider.affiliation();

    // Issue #56 DR-16. Raising this chat's capability to Private is the user's
    // act alone, and this route has no principal — `check_token` compares one
    // machine-wide bearer and every authenticated request looks the same,
    // whoever sent it (AR-11/AR-15). `llamacpp` needs no credential at all, so
    // `{"provider":"llamacpp"}` would otherwise be a tier raise any caller can
    // perform with nothing but the daemon secret.
    //
    // A CONDITION, not a blanket refusal: refusing every raise would take the
    // user's own model picker away along with the model's, which is the posture
    // DR-16 rejected. Sideways and downward binds are untouched for every
    // caller, which is what keeps Gate A's path, the CLI,
    // `restore_provider_from_session` and every apps-runtime bind working.
    // The one exception is this route on a daemon with no user-action key,
    // where a move onto a private model is measured from Public however the
    // chat is bound today — `raise_baseline`, SD-12. The predicate itself is
    // unchanged, and none of the in-process binds above passes through here.
    //
    // An unbound session reads as Public — `Agent::provider` errors when nothing
    // is bound (and when Gate B' refuses what is), and the conservative reading
    // is that a session with no private capability has none to preserve, so a
    // first bind to a private provider is a raise.
    let current = agent
        .provider()
        .await
        .map(|p| p.tier())
        .unwrap_or(ProviderTier::Public);
    // SD-12's other half — see `raise_baseline`.
    let baseline = raise_baseline(current, user_action_proof(&headers));
    // DR-15's master opt-out, read INSIDE the gate. A direct read, not a
    // `CallCapability`: a provider raise over HTTP is not a tool call and has no
    // admitted capability to inherit.
    if biorouter::privacy::privacy_tiers_enabled()
        && raise_needs_user_action(baseline, new_provider.tier())
        && !is_user_action(&headers)
    {
        return Err((
            StatusCode::CONFLICT,
            PrivacyRefusal::TierRaiseNeedsUser {
                requested: payload.provider.clone(),
            }
            .to_string(),
        )
            .into_response());
    }

    // Issue #56 Gate A (P3). A privacy refusal is a 409 with a body the GUI
    // renders a repair card from, not a 500 with a sentence: collapsing it to
    // the generic error is what let the renderer report a refused switch as a
    // green success toast.
    if let Err(e) = agent
        .update_provider(new_provider, &payload.session_id)
        .await
    {
        return Err(
            classify_provider_bind_failure(&e, available_private_providers().await).into_response(),
        );
    }

    // Issue #56 DR-26 at the BIND surface, and the whole of what this route was
    // missing. `Agent::update_provider` has detected this mismatch since Task 48
    // and has only ever written it to `tracing::warn!`, where the person who just
    // switched models cannot see it, so the ruling's "warn the user, naming both
    // institutions, before proceeding" was, on this surface, unimplemented.
    //
    // ⚠ **It warns; it does not refuse, and the ordering says so.** This is read
    // AFTER the bind has succeeded and is returned with a 200, not raised as a
    // 409 — both endpoints are Private, legitimate cross-institutional work under
    // a real DUA exists, and refusing here would strand a chat whose model is
    // already bound. Gate C still refuses the first DISPATCH, which is where the
    // user meets an accept control; this is the earlier, quieter statement that
    // stops the refusal being the first they hear of it.
    //
    // ⚠ **Empty is the normal answer** — every public model, every local model,
    // every model bound to the same institution as the chat's connectors, and
    // every machine with DR-27's `open` policy. A caller must treat an empty body
    // as "nothing to say" and never as "the daemon did not answer".
    Ok(agent
        .cross_affiliation_notice(&payload.session_id, model_affiliation)
        .await)
}

#[utoipa::path(
    post,
    path = "/agent/add_extension",
    request_body = AddExtensionRequest,
    responses(
        (status = 200, description = "Extension added. The body is DR-26's cross-institutional \
                                      warning for this chat once the extension is attached: the \
                                      statement the user is shown before proceeding, warnings \
                                      separated by a blank line, and is EMPTY when nothing in \
                                      the chat crosses an institutional boundary, which is the \
                                      normal case.",
                       body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused by a privacy or subagent-control boundary: the named \
                                      chat is private/absent, or is a subagent, and the request \
                                      carried no proof it came from the user"),
        (status = 409, description = "Refused by a privacy boundary (issue #56, DR-16): a \
                                      private extension cannot be attached to a chat running on \
                                      a public model"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn agent_add_extension(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<AddExtensionRequest>,
) -> Result<String, ErrorResponse> {
    // Issue #56 Task 58 / #47. FIRST, before the agent is fetched — `get_agent`
    // CREATES one for a session that has none, so a gate below it would let an
    // unproven caller materialise an agent for a chat it may not address, and
    // the refusals downstream (424 "not initialized", the tier 409) would tell
    // it what it had found. `session_id` is a request parameter, not a
    // credential; see `routes::session_reach`. A public subagent adds an
    // independent ownership check: the daemon bearer is not proof that the
    // person at the keyboard chose to alter that child.
    //
    // ⚠ This is NOT a relaxation of the outright refusal below. That one is
    // about the EXTENSION's tier against the chat's model and stays
    // unconditional — attaching a private extension to a public chat is not a
    // raise the user can authorize either. This one is about whether the caller
    // may address the named chat at all.
    let session = authorize_agent_control(&state, &request.session_id, &headers).await?;
    if session.session_type == SessionType::SubAgent {
        return Err(ErrorResponse {
            status: StatusCode::CONFLICT,
            message: "A subagent's delegated extension grants are fixed for its lifetime. Steer the running child with a prompt, or start a new child with different tools.".into(),
        });
    }
    let agent = state.get_agent(request.session_id.clone()).await?;

    // Issue #56 DR-16. `/agent/add_extension` hands `request.config` straight to
    // the agent and persists it, which is how a private extension's TOOLS arrive
    // in a session Gate F1 already refuses to let the model enable through
    // `extensionmanager__manage_extensions`.
    //
    // Refused OUTRIGHT — no user-proof branch, deliberately. Attaching a private
    // extension to a public session is not a raise the user can authorize
    // either; their route is to switch the model first and then attach.
    // ONE read of the bound provider, from which BOTH privacy axes are taken.
    // Two `agent.provider().await` calls would be the read-then-read
    // `CallCapability` exists to collapse — the mutex can be reassigned between
    // them with no turn lock — so the `Arc` is taken once and asked twice.
    let bound = agent.provider().await.ok();
    let capability = bound
        .as_ref()
        .map(|p| p.tier())
        .unwrap_or(ProviderTier::Public);
    let extension_name = request.config.name();
    // Task 43 (DR-23): the config, not just its name — a renamed entry is
    // resolved through the install directory in its arguments. Task 48 (DR-26):
    // one resolution carries the tier AND the affiliation, so the two axes
    // cannot disagree about this entry.
    let classification =
        biorouter::privacy::resolve_extension(&extension_name, Some(&request.config));
    // DR-15's master opt-out, read INSIDE the gate. A direct read, not a
    // `CallCapability`: `/agent/add_extension` is not a tool dispatch and has no
    // admitted capability to inherit. Read ONCE and shared with the
    // cross-affiliation check below, so this route cannot refuse on the tier
    // axis with tiers on and then warn — or not warn — with tiers off.
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    // The DR-26 half of that same single read, bound once here rather than taken
    // twice: the gate below states the mismatch against it and the notice at the
    // bottom suppresses flows the user has already accepted for it, and those two
    // must be the same model or the route can warn about one institution while
    // checking an acceptance recorded for another.
    let model_affiliation = bound.as_ref().and_then(|p| p.affiliation());
    // ⚠ **The predicate is asked, not re-typed.** This is the third door to the
    // same capability — the agent's two are `extensionmanager__manage_extensions`
    // and the workspace's enable pair, which share
    // `privacy::refusal::extension_enable_refusal` — and the rule it applies is
    // the identical one. It cannot call that gate (its refusal is the typed
    // `PrivateExtensionOverHttp` body the GUI renders a repair card from, and its
    // posture on the other two arms is deliberately different: a user proceeds
    // past the affiliation warning, and the operator pin is theirs to override).
    // So it asks the boolean underneath instead. Writing
    // `classification.tier.is_private() && capability == Public` out here is what
    // made this rule four hand-written copies; copies agree until the edit nobody
    // cross-checks.
    if enforced && biorouter::privacy::refusal::tier_refuses(classification.tier, capability) {
        return Err(ErrorResponse {
            status: StatusCode::CONFLICT,
            message: PrivacyRefusal::PrivateExtensionOverHttp {
                name: extension_name,
            }
            .to_string(),
        });
    }

    // Issue #56 Task 48, DR-26 at the USER's enable path. This is the same
    // mismatch the bind surface finds from the other end, and — unlike the tier
    // refusal above — it WARNS rather than refusing.
    //
    // The asymmetry with `extensionmanager__manage_extensions`, which refuses,
    // is DR-26's user/agent rule: a user who insists proceeds past a warning; an
    // agent never clears one automatically. This route is the user acting
    // through the GUI's Settings > Extensions, so the extension is attached and
    // the risk is stated.
    //
    // ⚠ **The log is the support transcript's copy, not the product.** DR-26
    // requires the user be shown the warning before proceeding, and for a long
    // time this `tracing::warn!` was the whole of it: a researcher enabling
    // another institution's connector from Settings > Extensions was told
    // nothing. The user's copy is the 200 body at the bottom of this handler.
    //
    // ⚠ **Both remain, and neither is redundant.** This one is read here, BEFORE
    // the attach, off the entry the caller actually sent — so a mismatch is on
    // the record even if the attach then fails, and even for the callers that
    // discard the body. The notice below is read AFTER the attach, off the
    // agent's live extension set, so it states the chat as it now is.
    if let Some(warning) = biorouter::privacy::affiliation::gate_cross_affiliation_warning(
        enforced,
        capability,
        model_affiliation,
        &extension_name,
        &classification,
    ) {
        tracing::warn!(
            session_id = request.session_id,
            extension = extension_name,
            "{warning}"
        );
    }

    agent
        .add_extension(request.config)
        .await
        .map_err(|e| ErrorResponse::internal(format!("Failed to add extension: {}", e)))?;

    // Persist here rather than in add_extension to ensure we only save state
    // after the extension successfully loads. This prevents failed extensions
    // from being persisted as enabled in the session.
    agent
        .persist_extension_state(&request.session_id)
        .await
        .map_err(|e| {
            error!("Failed to persist extension state: {}", e);
            ErrorResponse::internal(format!("Failed to persist extension state: {}", e))
        })?;
    // After the write, never before. The clicking window self-repairs from its
    // own response, but a second window, the History list and `useToolCount` in
    // another pane had no signal at all and stayed stale.
    biorouter::catalog::CatalogEvents::global().publish_session_refresh(&request.session_id);

    // Issue #56 DR-26 at the USER's enable surface — the second half of the
    // ruling's "warn the user, naming both institutions, before proceeding", and
    // the half that had no implementation at all. Composed by
    // `Agent::cross_affiliation_notice`, the same method `/agent/update_provider`
    // returns, so the two surfaces cannot start describing one boundary in
    // different words.
    //
    // ⚠ **Read AFTER the attach, deliberately.** The user has already acted, and
    // DR-26 says a user who insists proceeds — so this states the chat as it now
    // is, including any OTHER connector the newly bound model does not reach.
    // Reading it before the attach would answer for a chat that no longer exists
    // by the time the caller sees it, and would silently omit the very extension
    // the request was about.
    //
    // ⚠ **This is not a refusal and must never become one.** The agent's own
    // enable path (`check_enable_allowed`) refuses; this one warns, because the
    // caller here is the person at the keyboard. Inverting that strands a
    // legitimate cross-institutional user with no way to attach their own
    // connector, which is the "researchers turn the feature off" outcome DR-26
    // exists to avoid.
    Ok(agent
        .cross_affiliation_notice(&request.session_id, model_affiliation)
        .await)
}

/// What the grant route says to a caller that presented no proof of a human.
///
/// It is the model-facing half of DR-26's ruling and it says the two things a
/// model needs: that this is not its decision, and what to do instead. It ends
/// on the user's own action rather than on
/// [`biorouter::privacy::refusal::ASK_THE_USER_TO_SWITCH`] — this chat is already
/// on a private model, so "switch to a private model" is advice it has taken.
///
/// ⚠ **It carries NEITHER of the two markers the renderer keys on**, which is the
/// same choice `DECLASSIFY_NEEDS_USER` makes and for the same reason.
/// `USER_ACTION_REFUSAL_MARKER`'s toast says *switch this chat's model* and
/// `COPY_OF_PRIVATE_REFUSAL_MARKER`'s says *branch it from the chat window*; both
/// would send the user somewhere that cannot help, because the way out of this
/// one is to approve the flow or to re-bind to the owning institution's model.
/// The wording steps around both.
const CROSS_AFFILIATION_GRANT_NEEDS_USER: &str =
    "Accepting a cross-institutional data flow is a decision only the person at the keyboard can \
     make, and this request carried no proof it came from them. Nothing was recorded. Do not \
     retry; the same call will be refused again, and no setting, hook or permission mode \
     changes it. Tell the user which extension you need and what for, and let them approve it.";

/// …and when the daemon holds no user-action key at all.
///
/// A separate sentence, per Task 18A's open question 23: reporting "this daemon
/// cannot verify a human" as "you are not a human" sends the person at the
/// keyboard hunting for a permission they can never obtain. `just run-server`, a
/// hand-run `biorouterd agent` and every headless deployment land here.
const CROSS_AFFILIATION_GRANT_NO_KEY: &str =
    "This daemon was started without a user-action key, so it cannot verify that a request came \
     from the person at the keyboard, and accepting a cross-institutional data flow requires \
     that proof. Nothing was recorded. This control is unavailable on this daemon; use the \
     desktop app, or bind this chat to a model covered by the same institution's agreements.";

/// …and when there is no live mismatch to accept.
///
/// ⚠ **Refusing here is the control, not a validation nicety.** DR-26's whole
/// premise is that a user accepts a risk **that was stated to them**. A grant
/// recorded with no mismatch behind it is a pre-authorisation for a flow nobody
/// has described — it would sit in the store waiting for a future bind to make it
/// meaningful, and the sentence the user agreed to would never have existed.
const CROSS_AFFILIATION_GRANT_NOTHING_TO_ACCEPT: &str =
    "There is no cross-institutional mismatch to accept for that extension in this chat: it is \
     not enabled here, or the model bound right now is already covered by agreements that reach \
     it. Nothing was recorded.";

/// …and when the chat is not loaded in this daemon at all.
///
/// ⚠ **A distinct sentence, and this route must not fold it into the one above.**
/// The two states are indistinguishable to the caller and completely different to
/// the user: "nothing to accept" says the risk you saw is gone, "this chat is not
/// loaded" says ask again once it is. Reporting the second as the first is open
/// question 23's mistake with a different subject — it sends the person at the
/// keyboard hunting for a mismatch to fix when what they need is to open the
/// chat.
///
/// It is a refusal rather than a load, because the affiliation a grant is keyed
/// on can only be sampled off the model this chat is actually bound to. Loading
/// the chat here to answer would read the process default instead — a grant
/// recorded against the wrong institution, which no later call would match.
const CROSS_AFFILIATION_GRANT_CHAT_NOT_LOADED: &str =
    "That chat is not loaded in this daemon right now, so the model it is bound to, and \
     therefore whose agreements cover it, cannot be read. Nothing was recorded. Open the chat \
     and approve the flow there.";

/// A verdict from [`user_action_proof`] to the grant route's answer: `Ok(())`
/// only for `Proven`.
///
/// ⚠ **Extracted so the claim "only the user may grant" is asserted rather than
/// grepped for.** The handler it belongs to cannot be driven from a test —
/// `AppState::new()` opens the developer's REAL session database — so every
/// other fact about this route is a source scan, and a scan for
/// `user_action_proof(` keeps passing against a match whose `Unproven` arm was
/// refactored into `=> {}`. This mapping is pure, so
/// `only_a_proven_user_action_gets_past_the_grant_guard` drives all three arms
/// for real. The one thing it cannot see — that the guard runs before the chat
/// is touched — stays a scan, and says so.
fn refuse_grant_unless_user(proof: UserActionProof) -> Result<(), ErrorResponse> {
    match proof {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(ErrorResponse {
            status: StatusCode::FORBIDDEN,
            message: CROSS_AFFILIATION_GRANT_NEEDS_USER.to_string(),
        }),
        // A separate sentence, per Task 18A's open question 23 — see the
        // constant.
        UserActionProof::NoKeyInstalled => Err(ErrorResponse {
            status: StatusCode::FORBIDDEN,
            message: CROSS_AFFILIATION_GRANT_NO_KEY.to_string(),
        }),
    }
}

/// …and when the machine is in `strict` and the operating system did not
/// confirm (issue #56 Task 52, DR-27).
///
/// It says which of the three modes is in force, because that is the fact the
/// user needs and the one they can change: a person who does not know their
/// machine is in `strict` reads a bare "authentication failed" as a bug. It does
/// **not** tell a model how to leave `strict` — that is a user-only setting
/// behind its own system prompt, and advice to change it is advice to attack the
/// control.
const CROSS_AFFILIATION_GRANT_STRICT_NEEDS_SYSTEM: &str =
    "This machine's cross-institution mixing policy is set to 'strict', so accepting a \
     cross-institutional data flow needs your operating system to confirm it is you as well as \
     the in-app approval. That did not happen. Nothing was recorded.";

/// What the `strict` prompt names where a declassification would name its chats.
///
/// ⚠ **A CONSTANT IN THIS SOURCE, and that is a security property rather than a
/// style.** [`biorouter::privacy::system_auth::AuthRequest::about`] is infallible
/// precisely because *"there is exactly one subject, and it is a constant in the
/// caller's source"*, and every prompter renders that slot verbatim. This route
/// is the first caller whose natural subject — an extension name — arrives on an
/// HTTP body and is stored in the agent-writable `config.yaml` (DR-17), so
/// putting it there would let a planted extension choose what the user's system
/// password dialog says. The name is still shown: it goes in the *reason*,
/// through [`biorouter::privacy::system_auth::dialog_safe`].
const CROSS_AFFILIATION_GRANT_AUTH_SUBJECT: &str = "one cross-institution connector";

/// The `strict` layer on the grant, as a function so it can be driven.
///
/// ⚠ **Extracted for [`refuse_grant_unless_user`]'s reason, which is the same
/// reason.** This handler cannot be driven from a test — `AppState::new()` opens
/// the developer's REAL session database — so every other fact about the route
/// is a source scan, and a scan cannot tell "prompts in strict" from "prompts in
/// every mode" or from "prompts in none". Taking the policy and the prompter as
/// arguments makes all three modes assertions about the real decision path, with
/// no password typed. Production passes [`biorouter::privacy::mixing::policy`]
/// and [`biorouter::privacy::system_auth::prompter`], which is the one resolver
/// in the tree that can reach the test seam — pinned by
/// `the_strict_prompt_sits_between_the_resolution_and_the_write`, because a
/// literal in either argument would disable `strict` in production and pass
/// every test in this module.
///
/// ⚠ **`open` and `standard` never touch the prompter.** In `standard` this must
/// be exactly today's behaviour — one in-app confirmation — and a prompt raised
/// there would be the DR-19 prompt fatigue this feature keeps arguing against. In
/// `open` a grant is recordable but pointless (the gate no longer refuses), and
/// charging a password for a no-op is worse than pointless.
async fn strict_mode_authorization(
    policy: biorouter::privacy::mixing::MixingPolicy,
    prompter: &dyn biorouter::privacy::system_auth::SystemAuthenticator,
    extension: &str,
) -> Result<(), ErrorResponse> {
    if policy != biorouter::privacy::mixing::MixingPolicy::Strict {
        return Ok(());
    }
    let named = biorouter::privacy::system_auth::dialog_safe(extension);
    let request = biorouter::privacy::system_auth::AuthRequest::about(
        format!(
            "Allow this Biorouter chat to send data to `{named}`, across an institutional \
             boundary."
        ),
        CROSS_AFFILIATION_GRANT_AUTH_SUBJECT,
    );
    match biorouter::privacy::system_auth::authenticate_or_refuse(prompter, &request).await {
        None => Ok(()),
        // Nothing has been written at this point — the grant row is the next
        // statement — so the flow stays refused, which is the safe direction.
        Some(refusal) => Err(ErrorResponse {
            status: StatusCode::FORBIDDEN,
            message: format!("{CROSS_AFFILIATION_GRANT_STRICT_NEEDS_SYSTEM} {refusal}"),
        }),
    }
}

/// Record the user's acceptance of one cross-institutional data flow (issue #56,
/// DR-26 / Task 49).
///
/// ⚠ **This route is a reversal of `/agent/add_extension`'s posture and the
/// difference is the ruling, not an inconsistency.** That route refuses a private
/// extension on a public session *outright*, with no user-proof branch, because
/// attaching one is a raise the user cannot authorize either. A Private↔Private
/// cross-institution flow is the opposite case: both endpoints are already
/// private, the tier boundary is not being crossed, and DR-26 states explicitly
/// that legitimate cross-institutional work under a real DUA exists and that the
/// user may accept the stated risk. Blocking it outright is the design
/// researchers route around by turning the feature off. So this route exists, and
/// the tier refusal beside it is untouched.
///
/// The grant is keyed on the **triple** (session, extension, model affiliation),
/// where the affiliation is read by the daemon from the same sample that produced
/// the warning — never from the request. Re-binding to a different institution's
/// model changes the triple, so the acceptance does not carry over.
///
/// ⚠ **The desktop surface is the accept control on the refusal itself** (Task
/// 57): `CrossAffiliationAcceptCard` renders inside the failed tool call that
/// carries `privacy::refusal::CROSS_AFFILIATION_ACCEPT_MARKER`, and posts here
/// with the `X-User-Action` header this route requires. The generated client
/// attaches no header of its own, so every caller must supply one — which is the
/// point: a call without it is refused, and only a surface the user gestured at
/// has one to send.
#[utoipa::path(
    post,
    path = "/agent/cross_affiliation_grant",
    request_body = CrossAffiliationGrantRequest,
    responses(
        (status = 200, description = "The user's acceptance was recorded", body = CrossAffiliationGrantResponse),
        (status = 400, description = "There is no cross-institutional mismatch to accept"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused (issue #56, DR-26): only the user may accept a \
                                      cross-institutional data flow, or (DR-27) this machine's \
                                      mixing policy is 'strict' and the operating system did not \
                                      confirm the user"),
        (status = 424, description = "That chat is not loaded in this daemon, so the model it is \
                                      bound to cannot be read"),
        (status = 500, description = "Internal server error")
    )
)]
async fn agent_cross_affiliation_grant(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(request): Json<CrossAffiliationGrantRequest>,
) -> Result<Json<CrossAffiliationGrantResponse>, ErrorResponse> {
    // FIRST, before the agent is fetched or the extension named in the request is
    // resolved. An unproven caller learns nothing about which extensions this chat
    // has, and cannot use the refusals to probe — pinned by
    // `the_guard_runs_before_the_chat_is_touched`.
    //
    // The three-way form rather than `is_user_action`, so a daemon that was handed
    // no key is told something different from a caller that presented no proof —
    // Task 18A's open question 23. The mapping is one function down so it can be
    // driven by a test; see `refuse_grant_unless_user`.
    refuse_grant_unless_user(user_action_proof(&headers))?;

    // PEEK, never `get_agent`. This route inspects a chat; creating one to
    // inspect it reads the process default provider rather than the chat's
    // binding, answers "nothing to accept" for a chat that has plenty, and
    // leaves a bare agent cached under a real session id. See
    // `AppState::peek_agent`.
    let Some(agent) = state.peek_agent(&request.session_id).await else {
        return Err(ErrorResponse {
            status: StatusCode::FAILED_DEPENDENCY,
            message: CROSS_AFFILIATION_GRANT_CHAT_NOT_LOADED.to_string(),
        });
    };

    // ONE sample: the warning the user is accepting and the affiliation the grant
    // is keyed on come from the same read of the bound provider. Two reads would
    // let `update_provider` slip between them and record an acceptance of a
    // sentence the user never saw.
    let Some((affiliation, warning)) = agent
        .cross_affiliation_grant_subject(&request.extension)
        .await
    else {
        return Err(ErrorResponse {
            status: StatusCode::BAD_REQUEST,
            message: CROSS_AFFILIATION_GRANT_NOTHING_TO_ACCEPT.to_string(),
        });
    };

    // Issue #56 Task 52, DR-27. In `strict` the in-app confirmation is not enough
    // on its own: the operating system has to say it is you as well. Raised HERE,
    // immediately before the write, so every other refusal this handler can make
    // is already past and the user is not asked for a password to be told
    // afterwards that there was nothing to accept.
    //
    // ⚠ **The mode is read once, at this one site.** The gate's own three modes
    // are decided in `privacy::affiliation::refusing_mismatch`; this is the
    // second half of DR-27 Step 3, and the only thing on the grant route that
    // knows the policy exists.
    //
    // ⚠ **It widens the window between the ONE sample above and the write below
    // from microseconds to human-scale, and that is accepted rather than
    // overlooked.** `cross_affiliation_grant_subject` exists to take the warning
    // and the affiliation from one `CallCapability`, because `update_provider`
    // reassigns the provider mutex with no turn lock; parking here for a password
    // means the chat can rebind while the user is looking at the dialog. It fails
    // SAFE: the grant is keyed to the affiliation that was sampled, so a grant
    // recorded after a rebind simply never matches and the flow stays refused.
    // The alternative — asking for the password first — takes one and then
    // reports there was nothing to accept, which is the failure DR-20's own
    // header rules out.
    strict_mode_authorization(
        biorouter::privacy::mixing::policy(),
        biorouter::privacy::system_auth::prompter(),
        &request.extension,
    )
    .await?;

    // The single construction of the cross-affiliation proof-of-user in the tree,
    // pinned by
    // `privacy::grant::tests::the_proof_of_user_is_constructed_in_exactly_one_place`.
    // It is minted here, inside the handler that checked the guard above, and
    // nowhere a tool call can reach.
    //
    // ⚠ **Written through THIS AGENT's session manager, not `AppState`'s.** A
    // grant that is not visible to the gate that reads it is a control that
    // silently does nothing, so the write and the read must be the same store —
    // and `Agent::with_config` hands `Arc::clone(&config.session_manager)` to
    // `ExtensionManager::new`, which is the very handle Gate C's
    // `cross_affiliation_denial` queries. Taking it from the agent makes that
    // identity structural. `state.session_manager()` resolves to the same `Arc`
    // today, through `AgentManager`, but only because nothing has ever built an
    // agent on a different one.
    biorouter::privacy::grant::record(
        &agent.config.session_manager,
        &request.session_id,
        &request.extension,
        affiliation,
        &biorouter::privacy::grant::UserCrossAffiliationGrant::from_user_action(),
    )
    .await
    .map_err(|e| {
        error!("Failed to record cross-affiliation grant: {}", e);
        ErrorResponse::internal(format!("Failed to record cross-affiliation grant: {}", e))
    })?;

    Ok(Json(CrossAffiliationGrantResponse {
        // Composed by the module that owns the copy, never here: the dialog that
        // asked and the response that confirms must not differ by a word.
        accepted: biorouter::privacy::grant::accepted_statement(&warning),
    }))
}

#[utoipa::path(
    post,
    path = "/agent/remove_extension",
    request_body = RemoveExtensionRequest,
    responses(
        (status = 200, description = "Extension removed", body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The session is out of reach, or the target is a subagent and the request lacks user-action proof"),
        (status = 409, description = "Subagent extension grants are immutable for the lifetime of the delegated child"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn agent_remove_extension(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RemoveExtensionRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let session = authorize_agent_control(&state, &request.session_id, &headers).await?;
    if session.session_type == SessionType::SubAgent {
        return Err(ErrorResponse {
            status: StatusCode::CONFLICT,
            message: "A subagent's delegated extension grants are fixed for its lifetime. Steer the running child with a prompt, or start a new child with different tools.".into(),
        });
    }
    let agent = state.get_agent(request.session_id.clone()).await?;
    agent.remove_extension(&request.name).await?;

    agent
        .persist_extension_state(&request.session_id)
        .await
        .map_err(|e| {
            error!("Failed to persist extension state: {}", e);
            ErrorResponse {
                message: format!("Failed to persist extension state: {}", e),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;
    // After the write, never before — see `agent_add_extension`.
    biorouter::catalog::CatalogEvents::global().publish_session_refresh(&request.session_id);

    Ok(StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/agent/stop",
    request_body = StopAgentRequest,
    responses(
        (status = 200, description = "Agent stopped successfully", body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The session is out of reach, or the target is a subagent and the request lacks user-action proof"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn stop_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<StopAgentRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let session_id = payload.session_id;
    let session = authorize_agent_control(&state, &session_id, &headers).await?;
    let stop_guard = state.begin_agent_stop(&session_id);
    let is_subagent = session.session_type == SessionType::SubAgent;
    if is_subagent {
        state.abandon_pending_continuations_for_session(&session_id);
    }
    let queued_child_cancelled =
        is_subagent && biorouter::agents::subagent_handle::cancel_initializing_child(&session_id);

    // The guard installed the admission barrier and cancelled the exact turn
    // under the same registry lock. Keep it across eviction so no successor can
    // claim the newly retired slot while this stop request is still in flight.
    if let Some(turn_id) = stop_guard.cancelled_turn_id() {
        tracing::info!(
            "Stop for session {} cancelled in-flight turn {}",
            session_id,
            turn_id
        );
    }

    if let Err(e) = state.agent_manager.remove_session(&session_id).await {
        // A semaphore-queued child intentionally has no generic/live Agent yet;
        // cancelling its background handle is the complete stop operation.
        if !queued_child_cancelled {
            return Err(ErrorResponse {
                message: format!("Failed to stop agent for session {}: {}", session_id, e),
                status: StatusCode::NOT_FOUND,
            });
        }
    }

    Ok(StatusCode::OK)
}

async fn restart_agent_internal(
    state: &Arc<AppState>,
    session_id: &str,
    session: &Session,
) -> Result<Vec<ExtensionLoadResult>, ErrorResponse> {
    // Remove existing agent (ignore error if not found)
    let _ = state.agent_manager.remove_session(session_id).await;

    let agent = state
        .get_agent_for_route(session_id.to_string())
        .await
        .map_err(|code| ErrorResponse {
            message: "Failed to create new agent during restart".into(),
            status: code,
        })?;

    let provider_future = agent.restore_provider_from_session(session);
    let extensions_future = agent.load_extensions_from_session(session);

    let (provider_result, extension_results) = tokio::join!(provider_future, extensions_future);
    provider_result.map_err(|e| ErrorResponse {
        message: e.to_string(),
        status: StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    let context: HashMap<&str, Value> = HashMap::new();
    let desktop_prompt =
        render_global_file("desktop_prompt.md", &context).expect("Prompt should render");
    let mut update_prompt = desktop_prompt;

    if let Some(ref workflow) = session.workflow {
        match build_workflow_with_parameter_values(
            workflow,
            session.user_workflow_values.clone().unwrap_or_default(),
        )
        .await
        {
            Ok(Some(workflow)) => {
                if let Some(prompt) = apply_workflow_to_agent(&agent, session_id, &workflow, true)
                    .await
                    .map_err(|e| ErrorResponse {
                        message: e.to_string(),
                        status: StatusCode::BAD_REQUEST,
                    })?
                {
                    update_prompt = prompt;
                }
            }
            Ok(None) => {
                // Workflow has missing parameters - use default prompt
            }
            Err(e) => {
                return Err(ErrorResponse {
                    message: e.to_string(),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                });
            }
        }
    }
    agent.set_session_context_prompt(Some(update_prompt)).await;

    Ok(extension_results)
}

#[utoipa::path(
    post,
    path = "/agent/restart",
    request_body = RestartAgentRequest,
    responses(
        (status = 200, description = "Agent restarted successfully", body = RestartAgentResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The session is out of reach, or the target is a subagent and the request lacks user-action proof"),
        (status = 404, description = "Session not found"),
        (status = 424, description = "The delegated child is still initializing"),
        (status = 500, description = "Internal server error")
    )
)]
async fn restart_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RestartAgentRequest>,
) -> Result<Json<RestartAgentResponse>, ErrorResponse> {
    let session_id = payload.session_id.clone();
    let session = authorize_agent_control(&state, &session_id, &headers).await?;

    let extension_results = if session.session_type == SessionType::SubAgent {
        let agent = live_subagent_for_control(&state, &session).await?;
        // A live child owns provider-local state and a narrower daemon-authored
        // grant profile. Ordinary restart hydration would replace the former and
        // load the session's general extension snapshot over the latter.
        agent
            .restore_persisted_provider_if_missing(&session)
            .await
            .map_err(|error| ErrorResponse {
                message: error.to_string(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            })?;
        agent
            .restore_subagent_runtime_profile(&session)
            .await
            .map_err(|error| ErrorResponse {
                message: error.to_string(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            })?;
        Vec::new()
    } else {
        restart_agent_internal(&state, &session_id, &session).await?
    };

    Ok(Json(RestartAgentResponse { extension_results }))
}

/// Validate and apply a working-directory change for `session_id`, enforcing
/// the empty-chat-only rule (#44): the working directory is choosable only
/// while the chat is completely empty and immutable once it has any messages.
/// A mid-chat switch breaks the session's own history — file references
/// printed earlier re-resolve against the new root — so a started session
/// rejects the change with 409 CONFLICT, as defense in depth against stale or
/// alternative clients. Returns the updated session so the caller can restart
/// the agent from it.
///
/// Concurrency: the emptiness check and the write are one atomic conditional
/// `UPDATE` ([`SessionManager::try_update_working_dir_if_empty`]), so a first
/// message landing concurrently can never slip between a check and a write.
/// That closes the persisted-state race only; the HTTP handler additionally
/// holds the per-session turn guard across the update + agent restart so a
/// restart can't race an in-flight `/reply` either.
pub(crate) async fn apply_working_dir_update(
    session_manager: &SessionManager,
    session_id: &str,
    working_dir: &str,
) -> Result<Session, ErrorResponse> {
    let working_dir = working_dir.trim();

    if working_dir.is_empty() {
        return Err(ErrorResponse {
            message: "Working directory cannot be empty".into(),
            status: StatusCode::BAD_REQUEST,
        });
    }

    let path = PathBuf::from(working_dir);
    if !path.exists() || !path.is_dir() {
        return Err(ErrorResponse {
            message: "Invalid directory path".into(),
            status: StatusCode::BAD_REQUEST,
        });
    }

    match session_manager
        .try_update_working_dir_if_empty(session_id, path)
        .await
    {
        Ok(WorkingDirUpdate::Updated) => {}
        Ok(WorkingDirUpdate::RefusedNotEmpty) => {
            return Err(ErrorResponse {
                message: "the working directory is fixed once a chat has messages".into(),
                status: StatusCode::CONFLICT,
            });
        }
        Ok(WorkingDirUpdate::SessionNotFound) => {
            return Err(ErrorResponse {
                message: format!("Session not found: {}", session_id),
                status: StatusCode::NOT_FOUND,
            });
        }
        Err(e) => {
            error!("Failed to update session working directory: {}", e);
            return Err(ErrorResponse {
                message: format!("Failed to update working directory: {}", e),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            });
        }
    }

    session_manager
        .get_session(session_id, false)
        .await
        .map_err(|err| {
            error!("Failed to get session after working dir update: {}", err);
            ErrorResponse {
                message: format!("Failed to get session: {}", err),
                status: StatusCode::NOT_FOUND,
            }
        })
}

#[utoipa::path(
    post,
    path = "/agent/update_working_dir",
    request_body = UpdateWorkingDirRequest,
    responses(
        (status = 200, description = "Working directory updated and agent restarted successfully"),
        (status = 400, description = "Bad request - invalid directory path"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 Task 58 / #47): \
                                      the named chat is private (or absent, and an unproven caller \
                                      is told the same thing for both) and the request carried no \
                                      proof it came from the user; or the named chat is a delegated \
                                      subagent's, whose working directory only the person at the \
                                      keyboard may repoint (SD-8)"),
        (status = 404, description = "Session not found"),
        (
            status = 409,
            description = "Conflict - the working directory is fixed once a chat has messages, or a turn is in flight"
        ),
        (status = 500, description = "Internal server error")
    )
)]
async fn update_working_dir(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(payload): Json<UpdateWorkingDirRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let session_id = payload.session_id.clone();

    // Issue #56 Task 58 / #47. Before the turn lock, whose 409 tells an
    // unproven caller whether the chat it named is busy — the disclosure the
    // refusal is worded to withhold. This route repoints a chat at a directory
    // of the caller's choosing and restarts its agent there; `session_id` is a
    // request parameter, not a credential. See `routes::session_reach`.
    crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
        .await?;

    // SD-8, and the one write to a subagent's chat this daemon did not refuse.
    // This route repoints the named chat at a directory of the caller's choosing
    // and restarts its agent there, which for a delegated child is a change to
    // the thing the parent is being told about. Every other write to a
    // subagent's chat already asks for the proof — `/reply` inline,
    // `/agent/resume` through `read_resume_session`, and provider, extension,
    // stop and restart through `authorize_agent_control` — and this one did not,
    // which made the shipped SD-8 claim that "the daemon refuses every write to
    // it" false.
    //
    // ⚠ **`session_reach` above does not cover it, and cannot.** That gate is the
    // PRIVACY slice and is deliberately inert for a public session; a delegated
    // subagent's chat is normally public. The two 409s below are not the boundary
    // either: `try_update_working_dir_if_empty` refuses a chat that has messages
    // and the turn lock refuses one that is busy, and a just-spawned or queued
    // child is neither — which is exactly the window in which a subagent's tab
    // is interesting.
    //
    // AFTER the reach gate and BEFORE the turn lock, so the three refusals stay
    // in the order the rest of this file uses: a chat this caller may not reach
    // is refused without disclosing that it is a subagent's, and a subagent's is
    // refused without disclosing whether it is busy.
    //
    // Not `authorize_agent_control`, which would call `session_reach` a second
    // time, and not `read_update_session`, whose read failure is a 500 — this
    // route documents (and the desktop handles) a 404 for a session that is not
    // there.
    let target = state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|error| {
            error!("Failed to get session before working dir update: {}", error);
            ErrorResponse {
                message: format!("Failed to get session: {}", error),
                status: StatusCode::NOT_FOUND,
            }
        })?;
    refuse_subagent_unless_user(&target, &headers)?;

    // Serialize with `/reply`'s per-session turn lock (BR-33) by claiming the
    // turn slot for the whole update + restart. Without it, a first message
    // accepted (but not yet persisted) by an in-flight reply could pass the
    // emptiness check here, and that reply would then keep streaming against
    // the old agent while the session already points at the new dir. Holding
    // the guard means a reply either finished (its message makes the update
    // 409) or has not begun (it will find the restarted agent); a turn in
    // flight refuses the switch outright.
    let _turn_guard = state
        .try_begin_turn_idempotent(&session_id, CancellationToken::new(), None)
        .map_err(|_conflict| ErrorResponse {
            message: "cannot change the working directory while a turn is in progress".into(),
            status: StatusCode::CONFLICT,
        })?;

    let session =
        apply_working_dir_update(state.session_manager(), &session_id, &payload.working_dir)
            .await?;

    restart_agent_internal(&state, &session_id, &session).await?;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/agent/read_resource",
    request_body = ReadResourceRequest,
    responses(
        (status = 200, description = "Resource read successfully", body = ReadResourceResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "Refused: this entry reaches extensions as a public caller, and the named extension is private"),
        (status = 424, description = "Agent not initialized"),
        (status = 404, description = "Resource or extension not found"),
        (status = 502, description = "The extension failed the read"),
        (status = 500, description = "Internal server error")
    )
)]
async fn read_resource(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ReadResourceRequest>,
) -> Result<Json<ReadResourceResponse>, ErrorResponse> {
    use rmcp::model::ResourceContents;

    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await
        .map_err(|status| ErrorResponse {
            message: "could not load that chat".to_string(),
            status,
        })?;

    let read_result = agent
        .extension_manager
        .read_resource(
            &payload.uri,
            &payload.extension_name,
            // Issue #56: this entry has NO caller identity, exactly like
            // `call_tool` below it. It arrives outside the agent loop, so there
            // is no admitted turn whose capability it could inherit, and the
            // session's currently-bound provider is not this caller's.
            //
            // ⚠ This passed `None` until 2026-09-09, which made the guard
            // sample the NAMED session's bound model — so an HTTP client holding
            // the daemon secret could name a private chat and read its private
            // extensions' resources on that chat's reach. That is the "borrow
            // another session's capability" hole the execution plan closed at
            // `call_tool`; it ruled this route the same way for the same reason
            // and only `call_tool` was migrated. Public + enforced is the most
            // restrictive pair, and it is a constant, so there is nothing here
            // to race with `update_provider`.
            Some(biorouter::privacy::CallCapability::public_enforced()),
            CancellationToken::default(),
        )
        .await
        .map_err(|error| read_resource_failure(error, &payload.extension_name))?;

    let content = read_result
        .contents
        .into_iter()
        .next()
        .ok_or_else(|| ErrorResponse {
            message: format!(
                "`{}` returned no contents for `{}`",
                payload.extension_name, payload.uri
            ),
            status: StatusCode::NOT_FOUND,
        })?;

    let (uri, mime_type, text, meta) = match content {
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            meta,
        } => (uri, mime_type, text, meta),
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            meta,
        } => {
            let decoded = match base64::engine::general_purpose::STANDARD.decode(&blob) {
                Ok(bytes) => String::from_utf8(bytes).map_err(|_| {
                    ErrorResponse::internal("that resource is binary, not UTF-8 text")
                })?,
                Err(_) => {
                    return Err(ErrorResponse::internal(
                        "that resource's blob is not valid base64",
                    ))
                }
            };
            (uri, mime_type, decoded, meta)
        }
    };

    let meta_map = meta.map(|m| m.0);

    Ok(Json(ReadResourceResponse {
        uri,
        mime_type,
        text,
        meta: meta_map,
    }))
}

/// Classify a resource-read failure: a refusal the caller can act on, an
/// extension that is not there, an extension that failed the read — or a fault
/// of this process.
///
/// Extracted so the mapping can be tested without `AppState`, which builds the
/// process-global `AgentManager` and opens the REAL user session database. It
/// is the resource-path sibling of [`dispatch_failure_response`], and it exists
/// for the same reason: `.map_err(|_e| INTERNAL_SERVER_ERROR)` threw Gate C's
/// sibling refusal away and told the caller Biorouter had crashed. A privacy
/// refusal and a panic were the same 500, which is the one distinction a caller
/// needs — a refusal is answered by switching the chat to a private model, a
/// crash is not answered at all.
///
/// The three codes are `ExtensionManager::read_resource`'s own, not a taxonomy
/// invented here, and `is_privacy_refusal` in that module names the same split:
/// `INVALID_REQUEST` is produced on this path by `privacy::refusal::privacy_refusal`
/// and by nothing else; `INVALID_PARAMS` is "no such extension"
/// (`extension_manager.rs`, the `get_server_client` miss); `INTERNAL_ERROR` is
/// the extension answering badly, which is a bad gateway rather than a fault of
/// this daemon.
///
/// ⚠ **The not-found message is REPLACED rather than forwarded, and that is the
/// whole reason this takes `requested`.** The manager's own text reads
/// *"Extension 'x' not found. Here are the available extensions: …"*. That list
/// is now Gate E's rather than the raw extension map's — the 2026-09-10 test
/// drive's finding M6 closed it at the source, in `ExtensionManager` — so what
/// arrives here can no longer name an extension a public caller was not already
/// shown. This route still replaces it, for two reasons that outlive that fix:
/// the caller named ONE extension and a roster it did not ask for is not an
/// answer to its question, and a message that is safe only because its producer
/// filters it correctly is one edit away from being unsafe again. Defence in
/// depth on a boundary this cheap to hold is worth keeping.
///
/// The other two messages ARE forwarded: a refusal names only the extension the
/// caller itself asked for and is written to be read, and the read failure names
/// only the caller's own URI.
fn read_resource_failure(error: rmcp::model::ErrorData, requested: &str) -> ErrorResponse {
    use rmcp::model::ErrorCode;

    // `if`/`else` rather than `match`: `ErrorCode` is a newtype over `i32` whose
    // variants are associated consts, which are not patterns.
    if error.code == ErrorCode::INVALID_REQUEST {
        return ErrorResponse {
            message: error.message.to_string(),
            status: StatusCode::FORBIDDEN,
        };
    }
    if error.code == ErrorCode::INVALID_PARAMS {
        return ErrorResponse {
            message: format!("no extension named `{requested}` is loaded in that chat"),
            status: StatusCode::NOT_FOUND,
        };
    }
    ErrorResponse {
        message: error.message.to_string(),
        status: StatusCode::BAD_GATEWAY,
    }
}

#[cfg(test)]
mod read_resource_route_tests {
    //! What `POST /agent/read_resource` may reach, and what it says when it is
    //! refused.
    //!
    //! This route lost its last in-repo caller when MCP apps was removed
    //! (`docs/history/mcp-apps-removal/`) and was kept as a generic MCP
    //! capability an external client may call. Kept means guarded: the two
    //! properties below are the ones nothing asserted while it had a caller.
    //!
    //! The behaviour is asserted through [`read_resource_failure`] rather than
    //! over HTTP because `AppState::new()` opens the developer's REAL session
    //! database — the same reason `cross_affiliation_grant_route_tests` states.
    //! The capability the route hands the extension manager cannot be reached
    //! that way at all, so it is pinned at the source, and the refusal it
    //! produces is proved end-to-end by building the REAL refusal rather than a
    //! hand-made error with the same code.

    use super::{read_resource_failure, StatusCode};
    use biorouter::privacy::{
        refusal::{privacy_refusal, private_or_absent_refusal},
        ProviderTier,
    };
    use rmcp::model::{ErrorCode, ErrorData};

    const SOURCE: &str = include_str!("agent.rs");

    /// The property the handoff asks for: a public caller reaching a private
    /// extension is refused, and the refusal is distinguishable from a crash.
    ///
    /// The refusal is the REAL one — `privacy_refusal` is the single producer
    /// of `INVALID_REQUEST` on this path — so this fails if that function ever
    /// changes the code it refuses with, which a hand-built `ErrorData` would
    /// not.
    #[test]
    fn a_privacy_refusal_is_a_403_that_carries_its_own_words() {
        let refusal = privacy_refusal("ucsfomopagent", ProviderTier::Private, ProviderTier::Public)
            .expect("a public caller reaching a private extension is refused");

        let response = read_resource_failure(refusal, "ucsfomopagent");

        assert_eq!(response.status, StatusCode::FORBIDDEN);
        assert!(
            response.message.contains("private extension"),
            "the refusal must reach the caller, not be swallowed: {}",
            response.message
        );
    }

    /// Both real neighbours, because a classifier that answered 403 for
    /// everything would pass the test above.
    ///
    /// The not-found case also pins what the answer may NOT say. The manager's
    /// text is copied verbatim from `extension_manager.rs`'s `get_server_client`
    /// miss, list and all, so this fails the day the route starts forwarding it.
    #[test]
    fn the_two_neighbours_are_told_apart_from_a_refusal() {
        // `ExtensionManager::read_resource`'s `get_server_client` miss.
        let unknown = ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Extension 'nope' not found. Here are the available extensions: developer, \
             ucsfomopagent"
                .to_string(),
            None,
        );
        let response = read_resource_failure(unknown, "nope");
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert!(
            response.message.contains("nope"),
            "the caller must learn which name failed: {}",
            response.message
        );
        assert!(
            !response.message.contains("ucsfomopagent"),
            "a caller evaluated as public was handed the chat's extension roster, which is \
             exactly what Gate E withholds: {}",
            response.message
        );

        // The extension itself failing the read.
        let unreadable = ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            "Could not read resource with uri: ui://cohort".to_string(),
            None,
        );
        assert_eq!(
            read_resource_failure(unreadable, "developer").status,
            StatusCode::BAD_GATEWAY
        );
    }

    /// Finding M18: the 403 must not tell the caller a thing this gate never
    /// established.
    ///
    /// `POST /agent/read_resource` with `extension_name: "nonexistent_ext"`
    /// answered *"`nonexistent_ext` is a private extension: it reaches data held
    /// inside the institution …"*. Failing closed there is correct and stays —
    /// `assert_extension_reachable` reads an unknown name as Private on purpose,
    /// and answering "no such extension" instead would let a public caller walk
    /// names until it found the private connectors Gate E hides. What was wrong
    /// was the *claim*: a name that names nothing was asserted to be a private
    /// extension, sending a model to the model picker to fix a typo.
    ///
    /// The refusal is the REAL one, built by the function
    /// `assert_extension_reachable` composes, for the same reason the test above
    /// builds a real `privacy_refusal`: a hand-made `ErrorData` would pass
    /// against a production gate that had never been rewired.
    #[test]
    fn a_refusal_for_a_name_this_gate_cannot_resolve_does_not_call_it_private() {
        let refusal = private_or_absent_refusal(
            "nonexistent_ext",
            ProviderTier::Private,
            ProviderTier::Public,
        )
        .expect("an unknown name reads Private, so a public caller is refused");

        let response = read_resource_failure(refusal, "nonexistent_ext");

        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "failing closed is the correct behaviour and this finding does not relax it"
        );
        assert!(
            !response
                .message
                .contains("`nonexistent_ext` is a private extension"),
            "the refusal still asserts that a name this gate could not resolve IS a private \
             extension: {}",
            response.message
        );
        assert!(
            response.message.contains("nonexistent_ext"),
            "the caller must still learn which name it was refused: {}",
            response.message
        );
        assert!(
            response.message.contains("not installed"),
            "the refusal has to state the OTHER case it covers, or it is the same assertion \
             in softer words: {}",
            response.message
        );
    }

    /// A refusal that never reaches this classifier is not refused at all.
    ///
    /// The handler is the only caller, so the mapping above is worth exactly as
    /// much as the line that routes through it. This is what regressed:
    /// `map_err(|_e| StatusCode::INTERNAL_SERVER_ERROR)` was the shipped line,
    /// and it is the shape a future edit reaches for.
    #[test]
    fn the_handler_routes_its_failure_through_the_classifier() {
        let handler = crate::routes::body_of(SOURCE, "async fn read_resource");

        assert!(
            handler.contains("read_resource_failure(error, &payload.extension_name)"),
            "the route no longer classifies the extension manager's answer, or no longer \
             hands the classifier the name the CALLER asked for — which is what keeps the \
             404 from echoing the chat's whole extension roster"
        );
        assert!(
            !handler.contains("map_err(|_e| StatusCode::INTERNAL_SERVER_ERROR)"),
            "the route collapses every failure to a 500 again, so a privacy \
             refusal is indistinguishable from a crash"
        );
    }

    /// The capability itself, which no test above can see.
    ///
    /// Assembled rather than written out, so this assertion is not itself a
    /// match for `crates/biorouter/tests/privacy_capability.rs`'s whole-tree
    /// census of the two production constructors.
    #[test]
    fn the_route_declares_public_and_never_borrows_the_named_chats_reach() {
        let public_enforced = concat!("CallCapability::", "public_enforced()");
        let sample = concat!("CallCapability::", "sample(");

        let handler = crate::routes::body_of(SOURCE, "async fn read_resource");

        assert!(
            handler.contains(public_enforced),
            "an entry with no caller identity must declare the most restrictive \
             pair; passing `None` here samples the NAMED session's bound model, \
             which lets an HTTP client borrow a private chat's reach"
        );
        assert!(
            !handler.contains(sample),
            "this route has no caller identity to sample from"
        );
    }
}

/// Classify a dispatch failure: a **tool** error the caller can act on, or a
/// server fault that keeps its 500.
///
/// Extracted so the mapping can be tested without `AppState`, which builds the
/// process-global `AgentManager` and opens the REAL user session database. It
/// mirrors the agent loop, which downcasts to `ErrorData` to avoid
/// double-wrapping and hands the model the message.
///
/// ⚠ **The downcast is the whole classification, and it is not a formality.**
/// Every refusal `dispatch_tool_call` itself produces — Gate C's, `Tool '…' not
/// found`, `not available for extension`, BR-23's secret denial — is an
/// `ErrorData`, so all of them reach the caller as a tool result, which is the
/// fix this task owes: `.map_err(|_| INTERNAL_SERVER_ERROR)` threw Gate C's
/// refusal away and told the caller Biorouter had crashed.
///
/// Anything that is *not* an `ErrorData` is a fault of this process rather than
/// an answer about the tool, and Gate C introduced the first one: the O5 ratchet
/// propagates a session-store failure with `?`. Rendering that as `200 +
/// is_error` would tell an HTTP client the tool ran and disagreed, retire its
/// retry, and hand it raw store text. It keeps the 500 it had before this task.
fn dispatch_failure_response(error: &anyhow::Error) -> Result<CallToolResponse, StatusCode> {
    let Some(data) = error.downcast_ref::<rmcp::model::ErrorData>() else {
        tracing::error!(%error, "call_tool dispatch failed for a non-tool reason");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    Ok(CallToolResponse {
        content: vec![Content::text(data.message.to_string())],
        structured_content: None,
        is_error: true,
        _meta: None,
    })
}

/// Every gate that has to fire at this door, which dispatches a tool call
/// straight to the extension manager — bypassing the agent loop and therefore
/// every [`ToolInspector`](biorouter::tool_inspection::ToolInspector).
///
/// The three share one shape: an inspector would have refused or asked, nothing
/// in an HTTP handler can put an operation to the user and wait, so the call is
/// refused here rather than performed unasked. Each returns a tool ERROR rather
/// than a status code, because the caller is a tool caller and the remedy is in
/// the text.
///
/// * **Machine-wide memory** (issue #63 review, finding 3) — the consent gate
///   this route walked around entirely.
/// * **The transcript store** (issue #56) — `SessionStoreInspector` also runs
///   only in the agent loop, so this route reached
///   `~/.config/biorouter/sessions/sessions.db`, every conversation on the
///   machine including private ones, through any tool that takes a path.
/// * **Deleting a knowledge base** — `kb_delete_base` is contracted to show the
///   user an approval naming the base and everything that goes with it. Its
///   refusal names `DELETE /knowledge/bases/{id}`, the route the Knowledge
///   view's own delete button calls, so a person who means to delete a base
///   still has a door.
///
/// Extracted from [`call_tool`] rather than inlined: the third gate pushed that
/// handler past the `clippy::too_many_lines` baseline, and a list of boundary
/// gates is exactly the thing that should be readable in one place — a fourth
/// door added to two of the three is the failure mode
/// [`biorouter::security::UninspectedBoundary`] exists to prevent.
fn call_tool_boundary_refusal(
    tool_name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
) -> Option<String> {
    const BOUNDARY: biorouter::security::UninspectedBoundary =
        biorouter::security::UninspectedBoundary::AgentCallToolRoute;

    biorouter::security::global_memory::uninspected_boundary_refusal(tool_name, arguments, BOUNDARY)
        .or_else(|| {
            biorouter::security::session_store::uninspected_boundary_refusal(
                tool_name, arguments, BOUNDARY,
            )
        })
        .or_else(|| {
            biorouter::security::knowledge_delete::uninspected_boundary_refusal(
                tool_name, arguments, BOUNDARY,
            )
        })
}

#[utoipa::path(
    post,
    path = "/agent/call_tool",
    request_body = CallToolRequest,
    responses(
        (status = 200, description = "Resource read successfully", body = CallToolResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 404, description = "Resource not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn call_tool(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CallToolRequest>,
) -> Result<Json<CallToolResponse>, StatusCode> {
    let arguments = match payload.arguments {
        Value::Object(map) => Some(map),
        _ => None,
    };

    // Decided ahead of resolving the agent on purpose: whether this call is
    // allowed does not depend on any session state, and a decision that cannot
    // be reached without one is a decision that can be skipped by arriving
    // without one.
    if let Some(refusal) = call_tool_boundary_refusal(&payload.name, arguments.as_ref()) {
        return Ok(Json(CallToolResponse {
            content: vec![Content::text(refusal)],
            structured_content: None,
            is_error: true,
            _meta: None,
        }));
    }

    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await?;

    // Captured before `payload.name` is moved: the classifier below needs it,
    // and this route is the one catalog-mutating door that never persisted.
    let tool_name = payload.name.clone();
    let tool_call = CallToolRequestParams {
        task: None,
        name: payload.name.into(),
        arguments,
        meta: None,
    };

    // Issue #56: this entry has NO caller identity. It arrives outside the agent
    // loop, so there is no admitted turn whose capability it could inherit, and
    // the session's currently-bound provider is not this caller's — reading it
    // would hand an HTTP client whatever reach the user's chat happens to have.
    // Public + enforced is the most restrictive pair, and it is a constant, so
    // there is nothing here to race with `update_provider`.
    // F-15: and there is nobody here to ask, either. This entry has no admitted
    // turn and no stream to draw a card on, so a decision raised beneath it
    // would be inserted into a queue only the agent loop drains — waiting out
    // its whole time-to-live unanswerable, and then surfacing in the NEXT chat
    // turn as a question about something that happened minutes ago. Refusing
    // to park is the same reasoning as the capability above, one layer down.
    //
    // ⚠ The scope has to cover RUNNING the tool, not just dispatching it, and
    // that is the same one-of-two-steps split the #152 note below describes.
    // `dispatch_tool_call` hands back a `ToolCallResult` whose `.result` is a
    // future; the tool body — and therefore every `park()` in it — executes when
    // that future is awaited. Scoping only the dispatch left the run outside,
    // where `no_human_surface()` is false, so the refusal above never fired.
    //
    // Measured cost: `skills__installMarketplaceSkill` and
    // `skills__importSkillPackage` — both `requires_user_proof: true` — parked a
    // card nobody could answer and never returned. Not a slow download: 180 s
    // with no reply, while their `dryRun` siblings answered in 0.5 s and 9.8 s,
    // and the daemon log showed `calling client.call_tool` with no completion.
    let dispatched = biorouter::user_surface::without_human_surface(async {
        let dispatch = agent
            .extension_manager
            .dispatch_tool_call(
                &payload.session_id,
                tool_call,
                biorouter::privacy::CallCapability::public_enforced(),
                CancellationToken::default(),
            )
            .await;
        match dispatch {
            // Awaited HERE, inside the scope, so the tool body sees the flag.
            Ok(tool_result) => Ok(tool_result.result.await),
            Err(error) => Err(error),
        }
    })
    .await;

    let awaited = match dispatched {
        Ok(result) => result,
        // Issue #56 Gate C: this used to be
        // `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)`, which threw the
        // refusal away and told the caller that Biorouter had crashed. A tool
        // that could not be dispatched is a tool error and the remedy is in the
        // text, exactly as the agent loop treats it — see
        // [`dispatch_failure_response`] for the one class this does NOT cover.
        Err(error) => return dispatch_failure_response(&error).map(Json),
    };

    // #152: dispatching a tool and RUNNING it are two separate fallible steps —
    // `dispatch_tool_call` hands back a `ToolCallResult` whose `.result` is a
    // future. The `Err` arm above repairs the first; this repairs the second,
    // which is where the overwhelming majority of tool errors are raised.
    //
    // ⚠ This line was `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)` — the
    // exact thing the comment three lines above says was wrong, left in place on
    // the sibling site when that one was fixed. Measured cost: 45 of 419 driven
    // tool calls answered `500` with a ZERO-LENGTH body while `rmcp` logged the
    // real message at WARN ("no such background job: …", "kb '…' already
    // exists", "`render_figure` arguments are invalid for kind \"volcano\"").
    // A 500 is retryable by convention and a tool error is not, so the caller
    // both lost the remedy and was invited to retry a call that cannot succeed.
    //
    // `extension_manager.rs:2837-2840` already maps `ServiceError::McpError`
    // back to its `ErrorData`, so the awaited error downcasts correctly here —
    // the classifier was simply never asked.
    let result = match awaited {
        Ok(result) => result,
        // No downcast needed, and that is the point: this future's error type is
        // ALREADY `ErrorData`, so the message was never ambiguous or wrapped —
        // it was simply discarded. `dispatch_failure_response` exists for the
        // sibling site above, whose error arrives as an `anyhow::Error` and has
        // to be classified; here the classification is in the type.
        Err(error) => {
            return Ok(Json(CallToolResponse {
                content: vec![Content::text(error.message.to_string())],
                structured_content: None,
                is_error: true,
                _meta: None,
            }))
        }
    };

    // A `manage_extensions` through this route mutated the LIVE manager and
    // left `enabled_extensions.v0` untouched — the exact divergence the agent
    // loop's post-batch block exists to close, on a path that never reaches it.
    // Persist first, then announce: a consumer woken before the row lands would
    // refetch the old state and render it authoritatively.
    if result.is_error != Some(true) {
        if let Some(mutation) =
            biorouter::agents::session_extensions::tool_catalog_mutation(&tool_name)
        {
            if mutation.persist_extension_state {
                if let Err(error) = agent.persist_extension_state(&payload.session_id).await {
                    // Reported as a tool error rather than a status code, for
                    // the same reason as every other refusal on this route: the
                    // caller is a tool caller and the remedy is in the text.
                    // A subagent session lands here deliberately — its grants
                    // are immutable, and the tool must not appear to succeed.
                    return Ok(Json(CallToolResponse {
                        content: vec![Content::text(format!(
                            "{tool_name} changed the live tool catalog, but this session could \
                             not record it, so the change will not survive a reload: {error}"
                        ))],
                        structured_content: None,
                        is_error: true,
                        _meta: None,
                    }));
                }
            }
            biorouter::catalog::CatalogEvents::global()
                .publish_session_refresh(&payload.session_id);
        }
    }

    Ok(Json(CallToolResponse {
        content: result.content,
        structured_content: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
        _meta: result.meta.and_then(|m| serde_json::to_value(m).ok()),
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/agent/start", post(start_agent))
        .route("/agent/resume", post(resume_agent))
        .route("/agent/restart", post(restart_agent))
        .route("/agent/update_working_dir", post(update_working_dir))
        .route("/agent/tools", get(get_tools))
        .route("/agent/callable_tool_count", get(get_callable_tool_count))
        .route("/agent/read_resource", post(read_resource))
        .route("/agent/call_tool", post(call_tool))
        .route("/agent/update_provider", post(update_agent_provider))
        .route("/agent/update_from_session", post(update_from_session))
        .route("/agent/add_extension", post(agent_add_extension))
        .route(
            "/agent/cross_affiliation_grant",
            post(agent_cross_affiliation_grant),
        )
        .route("/agent/remove_extension", post(agent_remove_extension))
        .route("/agent/stop", post(stop_agent))
        .with_state(state)
}

#[cfg(test)]
mod new_session_provider_binding_tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use biorouter::config::with_config_overrides;
    use serial_test::serial;

    fn provider_overrides(
        provider: &str,
        model: &str,
        command_key: Option<&str>,
    ) -> HashMap<String, String> {
        let mut overrides = HashMap::from([
            ("BIOROUTER_PROVIDER".to_string(), provider.to_string()),
            ("BIOROUTER_MODEL".to_string(), model.to_string()),
        ]);
        if let Some(command_key) = command_key {
            overrides.insert(
                command_key.to_string(),
                std::env::current_exe()
                    .expect("the test executable has an absolute path")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        overrides
    }

    async fn assert_first_turn_provider(provider: &str, model: &str, command_key: &str) {
        let state = AppState::new().await.unwrap();
        let working_dir = format!("/tmp/biorouter-new-chat-{provider}");
        let started = with_config_overrides(
            provider_overrides(provider, model, Some(command_key)),
            start_agent(
                State(Arc::clone(&state)),
                HeaderMap::new(),
                Json(StartAgentRequest {
                    working_dir,
                    workflow: None,
                    workflow_id: None,
                    workflow_deeplink: None,
                    extension_overrides: Some(Vec::new()),
                }),
            ),
        )
        .await
        .expect("a valid selected provider should start the new chat")
        .0;

        assert_eq!(started.provider_name.as_deref(), Some(provider));
        assert_eq!(
            started
                .model_config
                .as_ref()
                .map(|config| config.model_name.as_str()),
            Some(model)
        );

        let agent = state
            .get_agent_for_route(started.id.clone())
            .await
            .expect("the first turn should resolve the newly created agent");
        assert_eq!(
            agent
                .provider()
                .await
                .expect("the first turn must not fail with `Provider not set`")
                .get_name(),
            provider
        );

        let _ = state.agent_manager.remove_session(&started.id).await;
        state
            .session_manager()
            .delete_session(&started.id)
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_new_codex_chat_binds_the_selected_provider_before_its_first_turn() {
        assert_first_turn_provider("codex", "gpt-5.5", "CODEX_COMMAND").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_new_claude_code_chat_binds_the_selected_provider_before_its_first_turn() {
        assert_first_turn_provider("claude_code", "claude-sonnet-4-6", "CLAUDE_CODE_COMMAND").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_failed_default_provider_bind_leaves_no_visible_broken_chat() {
        let state = AppState::new().await.unwrap();
        let working_dir = "/tmp/biorouter-new-chat-invalid-provider";
        let result = with_config_overrides(
            provider_overrides("provider-does-not-exist", "test-model", None),
            start_agent(
                State(Arc::clone(&state)),
                HeaderMap::new(),
                Json(StartAgentRequest {
                    working_dir: working_dir.to_string(),
                    workflow: None,
                    workflow_id: None,
                    workflow_deeplink: None,
                    extension_overrides: Some(Vec::new()),
                }),
            ),
        )
        .await;

        let error = result.expect_err("an unknown selected provider was accepted");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("selected provider"));
        let sessions = state
            .session_manager()
            .list_sessions_by_types_including_empty(&[SessionType::User])
            .await
            .unwrap();
        assert!(
            sessions
                .iter()
                .all(|session| session.working_dir.as_path() != std::path::Path::new(working_dir)),
            "the failed start left a provider-less chat row visible"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_new_private_provider_chat_requires_user_action_before_first_bind() {
        install_test_user_action_key();
        assert!(biorouter::privacy::privacy_tiers_enabled());
        let state = AppState::new().await.unwrap();
        let working_dir = "/tmp/biorouter-new-chat-private-provider";
        let mut overrides = provider_overrides(
            "versa_azure",
            biorouter::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL,
            None,
        );
        overrides.insert("VERSA_AZURE_API_KEY".into(), "test-api-key".into());
        overrides.insert(
            "VERSA_AZURE_ENDPOINT".into(),
            biorouter::providers::versa_azure::VERSA_AZURE_ENDPOINT.into(),
        );
        overrides.insert(
            "VERSA_AZURE_DEPLOYMENT_NAME".into(),
            biorouter::providers::versa_azure::deployment_for_model(
                biorouter::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL,
            )
            .expect("the default model has a deployment")
            .into(),
        );
        overrides.insert(
            "VERSA_AZURE_API_VERSION".into(),
            biorouter::providers::versa_azure::VERSA_AZURE_API_VERSION.into(),
        );
        let request = || StartAgentRequest {
            working_dir: working_dir.to_string(),
            workflow: None,
            workflow_id: None,
            workflow_deeplink: None,
            extension_overrides: Some(Vec::new()),
        };

        let refused = with_config_overrides(
            overrides.clone(),
            start_agent(State(Arc::clone(&state)), HeaderMap::new(), Json(request())),
        )
        .await
        .expect_err("a private first bind without user-action proof was accepted");
        assert_eq!(refused.status, StatusCode::CONFLICT);
        assert!(
            state
                .session_manager()
                .list_sessions_by_types_including_empty(&[SessionType::User])
                .await
                .unwrap()
                .iter()
                .all(|session| session.working_dir.as_path() != std::path::Path::new(working_dir)),
            "the refused private bind left a broken chat visible"
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-User-Action",
            TEST_USER_ACTION_KEY
                .parse()
                .expect("a header-safe test key"),
        );
        let started = with_config_overrides(
            overrides,
            start_agent(State(Arc::clone(&state)), headers, Json(request())),
        )
        .await
        .expect("valid user-action proof should authorize the private first bind")
        .0;
        assert_eq!(started.provider_name.as_deref(), Some("versa_azure"));

        let _ = state.agent_manager.remove_session(&started.id).await;
        state
            .session_manager()
            .delete_session(&started.id)
            .await
            .unwrap();
    }

    /// SD-12, every proof verdict against both tiers. The keyless arm cannot be
    /// reached through a route in this binary — the installed digest is a
    /// process-global `OnceLock` and the test above installs one — so the route
    /// half lives in `tests/new_chat_no_user_key.rs`, a binary that never does.
    #[test]
    fn only_a_daemon_that_can_check_a_proof_asks_a_new_chat_for_one() {
        use UserActionProof::{NoKeyInstalled, Proven, Unproven};
        // A daemon launched on this configuration, by a launcher that promised no
        // key: the posture SD-12's exemption is for.
        let launched_here = |tier, proof| new_chat_bind_decision(true, tier, proof, false, None);
        assert_eq!(
            launched_here(ProviderTier::Private, Proven),
            NewChatBind::Allowed
        );
        assert_eq!(
            launched_here(ProviderTier::Private, Unproven),
            NewChatBind::NeedsUserProof
        );
        assert_eq!(
            launched_here(ProviderTier::Private, NoKeyInstalled),
            NewChatBind::Allowed,
            "a keyless daemon refusing its own configured default refuses every person, always"
        );
        for proof in [Proven, Unproven, NoKeyInstalled] {
            // A public default raises nothing, for anyone.
            assert_eq!(
                launched_here(ProviderTier::Public, proof),
                NewChatBind::Allowed
            );
            // DR-15's master opt-out turns the gate off, not the question.
            assert_eq!(
                new_chat_bind_decision(false, ProviderTier::Private, proof, false, None),
                NewChatBind::Allowed
            );
        }
    }

    /// Finding 1 (HIGH) of the 2026-09-12 review: the keyless exemption is for
    /// the configuration the daemon was **launched** with, not for whatever
    /// `config.yaml` — which the agent can write and `Config` reloads live — says
    /// at request time.
    ///
    /// Both conditions bite only on the keyless arm. A daemon that can check a
    /// proof already has one, and a person who edits the configuration and asks
    /// for a chat in the same breath is not the case this closes.
    #[test]
    fn drift_since_launch_costs_the_keyless_exemption_and_nothing_else() {
        use UserActionProof::{NoKeyInstalled, Proven, Unproven};
        let moved = || Some("BIOROUTER_PROVIDER".to_string());

        assert_eq!(
            new_chat_bind_decision(true, ProviderTier::Private, NoKeyInstalled, false, moved()),
            NewChatBind::ConfigMovedSinceLaunch("BIOROUTER_PROVIDER".to_string()),
            "a private provider written into config.yaml after launch took the exemption"
        );
        // The refusal names the key, because the only person who can act on it
        // needs to know which value to put back or which daemon to restart.
        assert_eq!(
            new_chat_bind_decision(
                true,
                ProviderTier::Private,
                NoKeyInstalled,
                false,
                Some("OLLAMA_HOST".to_string()),
            ),
            NewChatBind::ConfigMovedSinceLaunch("OLLAMA_HOST".to_string()),
        );
        // A public default is not a raise, so drift changes nothing about it —
        // there is no exemption being taken to withdraw.
        assert_eq!(
            new_chat_bind_decision(true, ProviderTier::Public, NoKeyInstalled, false, moved()),
            NewChatBind::Allowed
        );
        // And a daemon that can check a proof is unaffected in both directions.
        assert_eq!(
            new_chat_bind_decision(true, ProviderTier::Private, Proven, true, moved()),
            NewChatBind::Allowed
        );
        assert_eq!(
            new_chat_bind_decision(true, ProviderTier::Private, Unproven, true, moved()),
            NewChatBind::NeedsUserProof
        );
    }

    /// Finding 3 (LOW): `NoKeyInstalled` is two situations wearing one name, and
    /// only one of them is a deployment where no proof can exist. A desktop daemon
    /// whose key never arrived keeps `main`'s refusal.
    ///
    /// ⚠ The launcher's declaration is checked **before** the drift, so the
    /// message the person gets names the fault they can act on rather than a
    /// configuration key that may be perfectly fine.
    #[test]
    fn a_launcher_that_promised_a_key_does_not_inherit_the_keyless_exemption() {
        use UserActionProof::{NoKeyInstalled, Proven, Unproven};
        assert_eq!(
            new_chat_bind_decision(true, ProviderTier::Private, NoKeyInstalled, true, None),
            NewChatBind::KeyWasExpected
        );
        assert_eq!(
            new_chat_bind_decision(
                true,
                ProviderTier::Private,
                NoKeyInstalled,
                true,
                Some("BIOROUTER_PROVIDER".to_string()),
            ),
            NewChatBind::KeyWasExpected,
            "a missing key is the actionable fault; the drift message would send the person to the \
             wrong place"
        );
        // The declaration says nothing about a daemon that DID get its key: those
        // two arms are decided by the proof alone.
        for proof in [Proven, Unproven] {
            assert_eq!(
                new_chat_bind_decision(true, ProviderTier::Private, proof, true, None),
                new_chat_bind_decision(true, ProviderTier::Private, proof, false, None),
            );
        }
        // Nor about a public default, which raises nothing.
        assert_eq!(
            new_chat_bind_decision(true, ProviderTier::Public, NoKeyInstalled, true, None),
            NewChatBind::Allowed
        );
    }

    /// SD-12's other half: a keyless daemon measures every move onto a private
    /// model from Public, so its exemption for the configured default cannot be
    /// carried sideways to a private model nobody configured.
    #[test]
    fn a_keyless_daemon_has_no_private_floor_for_a_switch_to_build_on() {
        use UserActionProof::{NoKeyInstalled, Proven, Unproven};
        for current in [ProviderTier::Private, ProviderTier::Public] {
            assert_eq!(
                raise_baseline(current, NoKeyInstalled),
                ProviderTier::Public
            );
            // A daemon that can check a proof keeps measuring from the live binding.
            assert_eq!(raise_baseline(current, Proven), current);
            assert_eq!(raise_baseline(current, Unproven), current);
        }
        // The composition `update_agent_provider` asks. Sideways onto a private
        // model is a raise only where no proof can be checked.
        let sideways = |proof| {
            raise_needs_user_action(
                raise_baseline(ProviderTier::Private, proof),
                ProviderTier::Private,
            )
        };
        assert!(sideways(NoKeyInstalled));
        assert!(!sideways(Unproven));
        assert!(!sideways(Proven));
    }
}

#[cfg(test)]
mod resume_update_security_tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use axum::body::Body;
    use axum::http::Request;
    use biorouter::agents::Agent;
    use biorouter::agents::SessionConfig;
    use biorouter::config::with_config_overrides;
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use biorouter::providers::errors::ProviderError;
    use biorouter::workflow::Response as WorkflowResponse;
    use futures::StreamExt;
    use rmcp::model::Tool;
    use serial_test::serial;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tower::ServiceExt;

    #[test]
    fn foreign_pending_continuation_never_serializes_an_opaque_lease() {
        let pending = PendingContinuationRef {
            ownership: PendingContinuationOwnership::Foreign,
            superseded_turn_id: "turn-stopped".to_string(),
            continuation_lease: None,
        };

        let value = serde_json::to_value(pending).unwrap();
        assert_eq!(value["ownership"], "foreign");
        assert_eq!(value["superseded_turn_id"], "turn-stopped");
        assert!(value.get("continuation_lease").is_none());
    }

    #[test]
    fn openapi_describes_the_agent_route_failures_clients_must_handle() {
        let schema: serde_json::Value =
            serde_json::from_str(&crate::openapi::generate_schema()).unwrap();
        for (method, path, statuses) in [
            ("post", "/agent/start", &["409"][..]),
            ("post", "/agent/resume", &["403", "404"][..]),
            ("post", "/agent/update_from_session", &["403", "500"][..]),
            ("post", "/agent/restart", &["424"][..]),
            // The two the SD-8 review gated. A client that has only ever seen a
            // 200/424 from the tool count, or a 400/404/409 from the working-dir
            // switch, now has a 403 to handle, and the generated TS client is
            // where it has to be visible.
            ("post", "/agent/update_working_dir", &["403"][..]),
            ("get", "/agent/callable_tool_count", &["403", "424"][..]),
        ] {
            let responses = &schema["paths"][path][method]["responses"];
            for status in statuses {
                assert!(
                    responses.get(*status).is_some(),
                    "{} {path} is missing its {status} OpenAPI response",
                    method.to_uppercase()
                );
            }
        }
    }

    struct RecordingSessionReader {
        session: Session,
        reads: Mutex<Vec<bool>>,
    }

    impl RecordingSessionReader {
        fn subagent() -> Self {
            Self {
                session: Session {
                    session_type: SessionType::SubAgent,
                    ..Session::default()
                },
                reads: Mutex::new(Vec::new()),
            }
        }

        fn reads(&self) -> Vec<bool> {
            self.reads.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl AgentSessionReader for RecordingSessionReader {
        async fn read_session(
            &self,
            _session_id: &str,
            include_messages: bool,
        ) -> anyhow::Result<Session> {
            self.reads.lock().unwrap().push(include_messages);
            Ok(self.session.clone())
        }
    }

    struct SeededSession {
        state: Arc<AppState>,
        id: String,
    }

    struct LiveChildProvider;

    struct PromptRecordingProvider {
        prompts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Provider for LiveChildProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new(
                "live-child-provider",
                "Live child provider",
                "",
                "live-child-model",
                vec![],
                "",
                vec![],
            )
        }

        fn get_name(&self) -> &str {
            "live-child-provider-not-in-the-registry"
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("live-child-model")
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text("ok"),
                ProviderUsage::new("live-child-model".into(), Usage::default()),
            ))
        }
    }

    #[async_trait::async_trait]
    impl Provider for PromptRecordingProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new(
                "prompt-recording-provider",
                "Prompt recording provider",
                "",
                "prompt-recording-model",
                vec![],
                "",
                vec![],
            )
        }

        fn get_name(&self) -> &str {
            "prompt-recording-provider-not-in-the-registry"
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("prompt-recording-model")
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.prompts.lock().unwrap().push(system.to_string());
            Ok((
                Message::assistant().with_text("ok"),
                ProviderUsage::new("prompt-recording-model".into(), Usage::default()),
            ))
        }
    }

    impl SeededSession {
        fn id(&self) -> &str {
            &self.id
        }
    }

    impl Drop for SeededSession {
        fn drop(&mut self) {
            let state = Arc::clone(&self.state);
            let id = std::mem::take(&mut self.id);
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    let _ = state.agent_manager.remove_session(&id).await;
                    if let Err(error) = state.session_manager().delete_session(&id).await {
                        eprintln!("route security test could not delete session {id}: {error}");
                    }
                })
            });
        }
    }

    async fn seed(state: &Arc<AppState>, kind: SessionType, private: bool) -> SeededSession {
        let session = state
            .session_manager()
            .create_session(
                PathBuf::from("/tmp/biorouter-agent-route-security"),
                "Agent route security test fixture".into(),
                kind,
            )
            .await
            .unwrap();
        // Session ids are `YYYYMMDD_N` minted as MAX(N)+1 over the rows that
        // still exist, so `SeededSession::drop` deleting a row hands that id
        // straight back to the next test. `AgentManager` is a process-global
        // singleton that outlives all of them, so without this a fixture can
        // inherit an agent an earlier test registered — and the assertions here
        // are precisely `peek_agent(..).is_none()`, which would then fail for a
        // reason that has nothing to do with the route under test.
        let _ = state.agent_manager.remove_session(&session.id).await;
        if private {
            state
                .session_manager()
                .update(&session.id)
                .provider_name("versa_azure")
                .model_config(ModelConfig::new("gpt-4o").unwrap())
                .raise_privacy(SessionClassification::Private, "turn:versa_azure")
                .apply()
                .await
                .unwrap();
        }
        SeededSession {
            state: Arc::clone(state),
            id: session.id,
        }
    }

    async fn post_agent_route(
        state: Arc<AppState>,
        path: &str,
        session_id: &str,
        user_action: Option<&str>,
    ) -> StatusCode {
        let body = match path {
            "/agent/resume" => serde_json::json!({
                "session_id": session_id,
                "load_model_and_extensions": true,
            }),
            "/agent/update_from_session" => serde_json::json!({
                "session_id": session_id,
            }),
            "/agent/update_provider" => serde_json::json!({
                "session_id": session_id,
                "provider": "provider-must-not-be-created-before-authorization",
                "model": "test-model",
            }),
            "/agent/add_extension" => serde_json::json!({
                "session_id": session_id,
                "config": {
                    "type": "builtin",
                    "name": "authorization-test-extension",
                    "description": "must not be loaded before authorization",
                },
            }),
            "/agent/remove_extension" => serde_json::json!({
                "session_id": session_id,
                "name": "authorization-test-extension",
            }),
            "/agent/stop" | "/agent/restart" => serde_json::json!({
                "session_id": session_id,
            }),
            // An existing directory, deliberately: a path that does not exist is
            // rejected with a 400 before the boundary is ever consulted, so a
            // test written with one passes against an ungated route.
            "/agent/update_working_dir" => serde_json::json!({
                "session_id": session_id,
                "working_dir": std::env::temp_dir().to_string_lossy(),
            }),
            _ => panic!("unexpected route in test: {path}"),
        };
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(key) = user_action {
            request = request.header("X-User-Action", key);
        }
        routes(state)
            .oneshot(
                request
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    /// `GET /agent/callable_tool_count?session_id=`, with the status AND the body:
    /// the body is what separates a gate from a 424 that happens to look like one.
    async fn get_callable_tool_count_response(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let mut request = Request::builder().method("GET").uri(format!(
            "/agent/callable_tool_count?session_id={session_id}"
        ));
        if let Some(key) = user_action {
            request = request.header("X-User-Action", key);
        }
        let response = routes(state)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// One agent route, one session id, no proof — status and body, so a test can
    /// compare two refusals for sameness rather than merely for their code.
    async fn post_agent_route_response(
        state: Arc<AppState>,
        path: &str,
        session_id: &str,
    ) -> (StatusCode, String) {
        let response = routes(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "session_id": session_id,
                            "load_model_and_extensions": false,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    async fn get_agent_tools(state: Arc<AppState>, session_id: &str) -> StatusCode {
        routes(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/agent/tools?session_id={session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    async fn resume_response(
        state: Arc<AppState>,
        session_id: &str,
        load_model_and_extensions: bool,
    ) -> (StatusCode, serde_json::Value) {
        let response = routes(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agent/resume")
                    .header("content-type", "application/json")
                    .header("X-User-Action", TEST_USER_ACTION_KEY)
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "session_id": session_id,
                            "load_model_and_extensions": load_model_and_extensions,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[test]
    fn both_handlers_run_reach_before_they_read_an_authorized_session_or_agent() {
        let source = include_str!("agent.rs");
        for (handler_name, authorized_read) in [
            ("resume_agent", "read_resume_session("),
            ("update_from_session", "read_update_session("),
        ] {
            let body = crate::routes::body_of(source, &format!("async fn {handler_name}"));
            let reach = body
                .find("session_reach(")
                .unwrap_or_else(|| panic!("{handler_name} does not enforce session reach"));
            let session_read = body
                .find(authorized_read)
                .unwrap_or_else(|| panic!("{handler_name} no longer reads its session"));
            assert!(
                reach < session_read,
                "{handler_name} reads the named session before the reach gate"
            );
            if let Some(agent_read) = body.find(".get_agent_for_route(") {
                assert!(
                    reach < agent_read,
                    "{handler_name} creates or reads an agent before the reach gate"
                );
            }
        }
    }

    #[test]
    fn child_control_handlers_authorize_before_lookup_or_mutation() {
        let source = include_str!("agent.rs");
        for (handler_name, first_effect) in [
            ("update_agent_provider", "live_subagent_for_control("),
            (
                "agent_add_extension",
                "session.session_type == SessionType::SubAgent",
            ),
            (
                "agent_remove_extension",
                "session.session_type == SessionType::SubAgent",
            ),
            ("stop_agent", "cancel_initializing_child("),
            ("restart_agent(", "live_subagent_for_control("),
        ] {
            let body = crate::routes::body_of(source, &format!("async fn {handler_name}"));
            let authorization = body.find("authorize_agent_control(").unwrap_or_else(|| {
                panic!("{handler_name} no longer enforces the child-control boundary")
            });
            let effect = body
                .find(first_effect)
                .unwrap_or_else(|| panic!("{handler_name} no longer has its expected effect"));
            assert!(
                authorization < effect,
                "{handler_name} touches child state before authorization"
            );
        }
    }

    #[test]
    fn resume_store_sequence_authorizes_metadata_before_loading_messages() {
        let source = include_str!("agent.rs");
        let body = crate::routes::body_of(source, "async fn read_resume_session");
        let metadata = body
            .find("read_session(session_id, false)")
            .expect("resume no longer starts with a metadata-only session read");
        let proof = body
            .find("refuse_subagent_unless_user(&metadata, headers)")
            .expect("resume no longer authorizes the metadata row");
        let transcript = body
            .find("read_session(session_id, true)")
            .expect("resume no longer loads the authorized transcript");
        assert!(metadata < proof && proof < transcript);
    }

    #[tokio::test]
    #[serial]
    async fn forbidden_subagent_store_reads_stop_at_metadata() {
        for route in ["resume", "update"] {
            let reader = RecordingSessionReader::subagent();
            let result = match route {
                "resume" => read_resume_session(&reader, "child", &HeaderMap::new())
                    .await
                    .map(|_| ()),
                "update" => read_update_session(&reader, "child", &HeaderMap::new())
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            };
            let error = result.expect_err("an unproven subagent request was accepted");
            assert_eq!(error.status, StatusCode::FORBIDDEN);
            assert_eq!(
                reader.reads(),
                vec![false],
                "{route} performed a full transcript read before refusing the request"
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn proven_subagent_resume_loads_metadata_then_transcript() {
        install_test_user_action_key();
        let reader = RecordingSessionReader::subagent();
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-User-Action",
            TEST_USER_ACTION_KEY
                .parse()
                .expect("valid test header value"),
        );

        read_resume_session(&reader, "child", &headers)
            .await
            .expect("proven subagent resume was refused");
        assert_eq!(reader.reads(), vec![false, true]);
    }

    #[test]
    fn resume_only_restores_a_provider_when_the_live_agent_is_missing_one() {
        let source = include_str!("agent.rs");
        let resume = crate::routes::body_of(source, "async fn resume_agent");
        assert!(
            resume.contains("load_resumed_agent_extensions"),
            "resume no longer delegates provider and extension restoration to its guarded helper"
        );
        let body = crate::routes::body_of(source, "async fn load_resumed_agent_extensions");
        assert!(
            body.contains("restore_persisted_provider_if_missing(session)"),
            "resume can no longer rebuild a cold agent from its persisted provider"
        );
        assert!(
            !body.contains("restore_provider_from_session(session)"),
            "resume unconditionally rebinds a live Codex or Claude child and discards its provider-local session"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn unproven_public_subagent_access_is_forbidden_without_creating_an_agent() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed(&state, SessionType::SubAgent, false).await;

        for path in ["/agent/resume", "/agent/update_from_session"] {
            assert_eq!(
                post_agent_route(Arc::clone(&state), path, child.id(), None).await,
                StatusCode::FORBIDDEN,
                "{path} accepted a daemon-only request as though a user typed in the child tab"
            );
            assert!(
                state.peek_agent(child.id()).await.is_none(),
                "{path} mutated the agent registry before refusing the request"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn bearer_only_cannot_control_public_or_private_subagents() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();

        for private in [false, true] {
            let child = seed(&state, SessionType::SubAgent, private).await;
            for path in [
                "/agent/update_provider",
                "/agent/add_extension",
                "/agent/remove_extension",
                "/agent/stop",
                "/agent/restart",
            ] {
                assert_eq!(
                    post_agent_route(Arc::clone(&state), path, child.id(), None).await,
                    StatusCode::FORBIDDEN,
                    "{path} accepted bearer-only control of a {} child",
                    if private { "private" } else { "public" }
                );
                assert!(
                    state.peek_agent(child.id()).await.is_none(),
                    "{path} materialized or mutated an agent before subagent authorization"
                );
            }
        }
    }

    /// SD-8 review finding 1. `/agent/update_working_dir` is a WRITE to the named
    /// chat — it repoints the session at a directory of the caller's choosing and
    /// restarts its agent there — and it used to consult only `session_reach`,
    /// which is **deliberately inert for a public session**. A delegated
    /// subagent's chat is normally public, so a caller holding nothing but the
    /// daemon secret could repoint a child, while the shipped SD-8 record said
    /// the daemon "refuses every write" to a subagent's chat.
    ///
    /// ⚠ **The public arm is the one that fails without the gate.** The private
    /// arm was already refused, by `session_reach`, for a reason that has nothing
    /// to do with subagents — so a test written on a private child alone passes
    /// against the hole.
    ///
    /// The directory is read back rather than trusting the status code: the two
    /// 409s this route already had (a chat with messages, a turn in flight) are
    /// not the subagent boundary and must not be mistaken for it.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn bearer_only_cannot_repoint_a_subagents_working_directory() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();

        for private in [false, true] {
            let child = seed(&state, SessionType::SubAgent, private).await;
            let before = state
                .session_manager()
                .get_session(child.id(), false)
                .await
                .unwrap()
                .working_dir;

            assert_eq!(
                post_agent_route(
                    Arc::clone(&state),
                    "/agent/update_working_dir",
                    child.id(),
                    None,
                )
                .await,
                StatusCode::FORBIDDEN,
                "/agent/update_working_dir accepted bearer-only repointing of a {} child",
                if private { "private" } else { "public" }
            );
            assert_eq!(
                state
                    .session_manager()
                    .get_session(child.id(), false)
                    .await
                    .unwrap()
                    .working_dir,
                before,
                "the refused request still moved the {} child's working directory",
                if private { "private" } else { "public" }
            );
            assert!(
                state.peek_agent(child.id()).await.is_none(),
                "/agent/update_working_dir restarted an agent before refusing the request"
            );
        }
    }

    /// SD-8 review finding 2. `/agent/callable_tool_count` had no gate of any
    /// kind: not `session_reach`, not the subagent refusal. It is a read of the
    /// named session's model-facing tool surface, and it answers through
    /// `get_agent_for_route`, which **creates** an agent for a session that has
    /// none — the same hazard `agent_add_extension` gates against and says so.
    ///
    /// PR #260's renderer stopped calling it for a subagent's chat, which left
    /// the route exactly as open as it was. Gated the way its tier-bearing
    /// siblings are (`GET /sessions/{id}`, `POST /agent/resume`): the privacy
    /// reach gate, first, before the agent is fetched.
    ///
    /// The `peek_agent` assertion is not decoration — a gate placed below the
    /// fetch would pass the status assertion and still mint the agent.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn callable_tool_count_refuses_an_unproven_caller_naming_a_private_chat() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed(&state, SessionType::User, true).await;

        let (status, body) =
            get_callable_tool_count_response(Arc::clone(&state), private.id(), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "/agent/callable_tool_count answered an unproven caller about a private chat: {body}"
        );
        assert!(
            state.peek_agent(private.id()).await.is_none(),
            "/agent/callable_tool_count materialized an agent for a chat it may not address"
        );

        // And the proof is sufficient, so the desktop's own tool-count alert is
        // unaffected: whatever this answers, it is not the reach refusal.
        let (proven, _) = get_callable_tool_count_response(
            Arc::clone(&state),
            private.id(),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_ne!(
            proven,
            StatusCode::FORBIDDEN,
            "/agent/callable_tool_count refused a request carrying the user-action proof"
        );
    }

    /// SD-8 review finding 4, recorded as a MEASUREMENT rather than closed as a
    /// bug — see `docs/deployment/serve-decisions.md`.
    ///
    /// The refusal bodies do differ: a **public** subagent is told it is a
    /// subagent, an unknown id is told only that it is out of reach. That pair
    /// would be an existence oracle for subagent ids if it were the only way to
    /// learn the fact. It is not, and the fourth row here is why: the same
    /// unproven caller is answered **200** for a public chat that is not a
    /// subagent's, and `GET /sessions/{id}` — inert for public chats by the same
    /// rule — hands it the whole row, `session_type` included. The differing body
    /// discloses nothing that a 200 next door does not.
    ///
    /// What must hold, and is asserted here, is that the pair collapses wherever
    /// the 200 is *not* available: for a **private** subagent the two refusals are
    /// identical, because `session_reach` fires first and its one sentence answers
    /// "private" and "no such chat" alike.
    ///
    /// ⚠ With privacy tiers OFF `session_reach` returns `Ok` before its store
    /// read, so the private row joins the public one and the whole pair separates
    /// again. That is the master switch's pre-existing blast radius, not this
    /// route's, and it is not closed here.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_private_subagent_and_an_unknown_id_are_refused_in_the_same_words() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private_child = seed(&state, SessionType::SubAgent, true).await;
        let public_child = seed(&state, SessionType::SubAgent, false).await;
        let public_chat = seed(&state, SessionType::User, false).await;

        let (unknown_status, unknown_body) =
            post_agent_route_response(Arc::clone(&state), "/agent/resume", "19700101_404").await;
        let (private_status, private_body) =
            post_agent_route_response(Arc::clone(&state), "/agent/resume", private_child.id())
                .await;
        assert_eq!(unknown_status, StatusCode::FORBIDDEN);
        assert_eq!(private_status, StatusCode::FORBIDDEN);
        assert_eq!(
            private_body, unknown_body,
            "a private subagent is distinguishable from a nonexistent id by its refusal"
        );
        assert!(
            unknown_body.contains(crate::routes::session_reach::SESSION_OUT_OF_REACH),
            "the shared refusal is no longer the reach sentence: {unknown_body}"
        );

        // The dominating disclosure, measured in the same run so the argument
        // above cannot rot into an assumption.
        let (public_child_status, public_child_body) =
            post_agent_route_response(Arc::clone(&state), "/agent/resume", public_child.id()).await;
        assert_eq!(public_child_status, StatusCode::FORBIDDEN);
        assert!(
            public_child_body.contains(SUBAGENT_USER_ACTION_REQUIRED),
            "a public subagent is no longer told why it is refused: {public_child_body}"
        );
        let (public_chat_status, _) =
            post_agent_route_response(Arc::clone(&state), "/agent/resume", public_chat.id()).await;
        assert_eq!(
            public_chat_status,
            StatusCode::OK,
            "an unproven caller is refused an ordinary public chat, which would make the \
             subagent body the only existence signal and turn finding 4 into a real oracle"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn proven_user_cannot_drift_child_grants_and_can_stop_public_or_private_subagents() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();

        for private in [false, true] {
            let child = seed(&state, SessionType::SubAgent, private).await;
            let live = Arc::new(Agent::new());
            state
                .agent_manager
                .register_agent(child.id().to_string(), Arc::clone(&live))
                .await;

            assert_eq!(
                post_agent_route(
                    Arc::clone(&state),
                    "/agent/remove_extension",
                    child.id(),
                    Some(TEST_USER_ACTION_KEY),
                )
                .await,
                StatusCode::CONFLICT
            );
            assert!(Arc::ptr_eq(
                &state
                    .peek_agent(child.id())
                    .await
                    .expect("remove unexpectedly evicted the child"),
                &live
            ));

            assert_eq!(
                post_agent_route(
                    Arc::clone(&state),
                    "/agent/stop",
                    child.id(),
                    Some(TEST_USER_ACTION_KEY),
                )
                .await,
                StatusCode::OK
            );
            assert!(state.peek_agent(child.id()).await.is_none());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn queued_child_control_never_materializes_a_generic_agent() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed(&state, SessionType::SubAgent, false).await;
        let cancellation = CancellationToken::new();
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register_initializing(
            "route-security-parent",
            child.id().to_string(),
            "queued child route security",
            cancellation.clone(),
        );

        let (resume_status, resumed) = resume_response(Arc::clone(&state), child.id(), true).await;
        assert_eq!(resume_status, StatusCode::OK);
        assert_eq!(resumed["initializing"], true);
        assert!(state.peek_agent(child.id()).await.is_none());

        assert_eq!(
            post_agent_route(Arc::clone(&state), "/agent/stop", child.id(), None).await,
            StatusCode::FORBIDDEN
        );
        assert!(
            !cancellation.is_cancelled(),
            "bearer-only stop cancelled the queued child before user authorization"
        );

        assert_eq!(
            get_agent_tools(Arc::clone(&state), child.id()).await,
            StatusCode::FAILED_DEPENDENCY
        );
        assert!(state.peek_agent(child.id()).await.is_none());

        for (path, expected) in [
            ("/agent/update_provider", StatusCode::FAILED_DEPENDENCY),
            ("/agent/add_extension", StatusCode::CONFLICT),
            ("/agent/remove_extension", StatusCode::CONFLICT),
            ("/agent/restart", StatusCode::FAILED_DEPENDENCY),
        ] {
            assert_eq!(
                post_agent_route(
                    Arc::clone(&state),
                    path,
                    child.id(),
                    Some(TEST_USER_ACTION_KEY),
                )
                .await,
                expected,
                "{path} did not recognize the authorized-but-not-ready child"
            );
            assert!(
                state.peek_agent(child.id()).await.is_none(),
                "{path} cached a generic agent for the queued child"
            );
        }

        assert_eq!(
            post_agent_route(
                Arc::clone(&state),
                "/agent/stop",
                child.id(),
                Some(TEST_USER_ACTION_KEY),
            )
            .await,
            StatusCode::OK
        );
        assert!(cancellation.is_cancelled());
        assert!(state.peek_agent(child.id()).await.is_none());
        handle.complete(biorouter::agents::SubagentResult::from_error(
            "queued route-security fixture cleaned up",
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn ordinary_resume_reports_initializing_false() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let ordinary = seed(&state, SessionType::User, false).await;

        let (status, resumed) = resume_response(Arc::clone(&state), ordinary.id(), false).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resumed["initializing"], false);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn proven_resume_does_not_construct_a_generic_agent_for_a_queued_child() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed(&state, SessionType::SubAgent, false).await;
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register_initializing(
            "queued-resume-parent",
            child.id(),
            "delegated work",
            tokio_util::sync::CancellationToken::new(),
        );

        assert_eq!(
            post_agent_route(
                Arc::clone(&state),
                "/agent/resume",
                child.id(),
                Some(TEST_USER_ACTION_KEY),
            )
            .await,
            StatusCode::OK
        );
        assert!(
            state.peek_agent(child.id()).await.is_none(),
            "resume installed a generic agent before the delegated runtime was ready"
        );
        handle.complete(
            biorouter::agents::subagent_result::SubagentResult::from_error("test cleanup"),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn private_user_sessions_are_out_of_reach_before_either_handler_mutates() {
        install_test_user_action_key();
        assert!(
            biorouter::privacy::privacy_tiers_enabled(),
            "this test requires the normal privacy-tier gate"
        );
        let state = AppState::new().await.unwrap();
        let private = seed(&state, SessionType::User, true).await;

        for path in ["/agent/resume", "/agent/update_from_session"] {
            assert_eq!(
                post_agent_route(Arc::clone(&state), path, private.id(), None).await,
                StatusCode::FORBIDDEN,
                "{path} reached a private user session with no user proof or private caller"
            );
            assert!(
                state.peek_agent(private.id()).await.is_none(),
                "{path} created an agent before refusing private-session reach"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn proven_live_subagent_update_preserves_its_override_and_workflow_tools() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed(&state, SessionType::SubAgent, false).await;
        let mut workflow = Workflow::builder()
            .title("must not apply")
            .description("a discriminating update fixture")
            .instructions("THIS DESKTOP WORKFLOW PROMPT MUST NOT REACH THE CHILD")
            .build()
            .unwrap();
        workflow.response = Some(WorkflowResponse {
            json_schema: Some(serde_json::json!({
                "type": "object",
                "properties": { "unexpected": { "type": "boolean" } }
            })),
        });
        state
            .session_manager()
            .update(child.id())
            .workflow(Some(workflow))
            .apply()
            .await
            .unwrap();

        let agent = Arc::new(Agent::new());
        agent
            .override_system_prompt("CHILD-SPECIFIC-SUBAGENT-SYSTEM-PROMPT".into())
            .await;
        state
            .agent_manager
            .register_agent(child.id().to_string(), Arc::clone(&agent))
            .await;
        assert!(
            !agent.list_tools(child.id(), None).await.iter().any(|tool| {
                tool.name == biorouter::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME
            }),
            "the fixture unexpectedly began with workflow tools"
        );

        assert_eq!(
            post_agent_route(
                Arc::clone(&state),
                "/agent/update_from_session",
                child.id(),
                Some(TEST_USER_ACTION_KEY),
            )
            .await,
            StatusCode::OK
        );
        let still_live = state
            .peek_agent(child.id())
            .await
            .expect("the live child was removed by a prompt refresh");
        assert!(Arc::ptr_eq(&still_live, &agent));
        assert!(
            !agent
                .list_tools(child.id(), None)
                .await
                .iter()
                .any(|tool| {
                    tool.name == biorouter::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME
                }),
            "the update applied the desktop workflow to a live subagent; the same path also appends the desktop prompt behind its subagent override"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn cold_subagent_resume_restores_only_its_delegated_runtime_profile() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed(&state, SessionType::SubAgent, false).await;
        let command = std::env::current_exe()
            .expect("the test executable has an absolute path")
            .to_string_lossy()
            .into_owned();

        with_config_overrides(
            HashMap::from([("CODEX_COMMAND".to_string(), command)]),
            async {
                let live = state.get_agent(child.id().to_string()).await.unwrap();
                live.update_provider(
                    create("codex", ModelConfig::new_or_fail("gpt-5.5"))
                        .await
                        .unwrap(),
                    child.id(),
                )
                .await
                .unwrap();

                let mut extension_data = biorouter::session::ExtensionData::new();
                EnabledExtensionsState::new(vec![ExtensionConfig::Platform {
                    name: "workspace".into(),
                    description: "Legacy broad workspace snapshot".into(),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                }])
                .to_extension_data(&mut extension_data)
                .unwrap();
                extension_data.set_extension_state(
                    "subagent_runtime_profile",
                    "v2",
                    serde_json::json!({
                        "format_version": 2,
                        "system_prompt": "COLD_ROUTE_PROFILE_PROMPT",
                        "response": {
                            "json_schema": {
                                "type": "object",
                                "properties": { "answer": { "type": "string" } },
                                "required": ["answer"]
                            }
                        },
                        "sub_workflows": [],
                        "extension_grants": [{
                            "name": "todo",
                            "kind": "platform",
                            "tools": ["todo_write"]
                        }]
                    }),
                );
                state
                    .session_manager()
                    .update(child.id())
                    .extension_data(extension_data)
                    .apply()
                    .await
                    .unwrap();
                state
                    .agent_manager
                    .remove_session(child.id())
                    .await
                    .unwrap();

                let child_row = state
                    .session_manager()
                    .get_session(child.id(), false)
                    .await
                    .unwrap();
                let (extension_results, initialization_error) = load_resumed_agent_extensions(
                    &state,
                    &ResumeAgentRequest {
                        session_id: child.id().to_string(),
                        load_model_and_extensions: true,
                        continuation_owner_id: None,
                    },
                    &child_row,
                    false,
                )
                .await;
                assert!(initialization_error.is_none());
                assert!(extension_results.is_some_and(|results| results.is_empty()));

                let cold = state
                    .peek_agent(child.id())
                    .await
                    .expect("resume did not cache the restored child");
                let tool_names: Vec<String> = cold
                    .list_tools(child.id(), None)
                    .await
                    .into_iter()
                    .map(|tool| tool.name.to_string())
                    .collect();
                assert!(tool_names.iter().any(|name| name == "todo__todo_write"));
                assert!(tool_names.iter().any(|name| {
                    name == biorouter::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME
                }));
                assert!(
                    tool_names
                        .iter()
                        .all(|name| !name.starts_with("workspace__")),
                    "ordinary extension hydration widened the cold child's grants: {tool_names:?}"
                );

                let prompts = Arc::new(Mutex::new(Vec::new()));
                cold.update_provider(
                    Arc::new(PromptRecordingProvider {
                        prompts: Arc::clone(&prompts),
                    }),
                    child.id(),
                )
                .await
                .unwrap();
                let mut reply = cold
                    .reply(
                        Message::user().with_text("verify the restored prompt"),
                        SessionConfig {
                            id: child.id().to_string(),
                            schedule_id: None,
                            max_turns: Some(1),
                            max_tool_calls: Some(1),
                            budget: None,
                            retry_config: None,
                            reasoning_effort: None,
                        },
                        None,
                    )
                    .await
                    .unwrap();
                while reply.next().await.is_some() {}
                assert!(
                    prompts
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|prompt| prompt.contains("COLD_ROUTE_PROFILE_PROMPT")),
                    "the restored child did not receive its delegated system prompt"
                );
            },
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn live_subagent_restart_preserves_provider_and_runtime_profile() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed(&state, SessionType::SubAgent, false).await;
        let agent = state.get_agent(child.id().to_string()).await.unwrap();
        let provider: Arc<dyn Provider> = Arc::new(LiveChildProvider);
        let expected_provider = Arc::clone(&provider);
        agent.update_provider(provider, child.id()).await.unwrap();
        let mut extension_data = biorouter::session::ExtensionData::new();
        extension_data.set_extension_state(
            "subagent_runtime_profile",
            "v2",
            serde_json::json!({
                "format_version": 2,
                "system_prompt": "LIVE CHILD RUNTIME PROMPT",
                "response": {
                    "json_schema": {
                        "type": "object",
                        "properties": { "child_result": { "type": "string" } },
                        "required": ["child_result"]
                    }
                },
                "sub_workflows": [],
                "extension_grants": []
            }),
        );
        state
            .session_manager()
            .update(child.id())
            .extension_data(extension_data)
            .apply()
            .await
            .unwrap();
        let child_row = state
            .session_manager()
            .get_session(child.id(), false)
            .await
            .unwrap();
        assert!(
            agent
                .restore_subagent_runtime_profile(&child_row)
                .await
                .unwrap(),
            "the child runtime profile fixture did not install"
        );
        assert!(
            agent.list_tools(child.id(), None).await.iter().any(|tool| {
                tool.name == biorouter::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME
            }),
            "the child runtime fixture is missing its structured final-output tool"
        );

        assert_eq!(
            post_agent_route(
                Arc::clone(&state),
                "/agent/restart",
                child.id(),
                Some(TEST_USER_ACTION_KEY),
            )
            .await,
            StatusCode::OK
        );

        let still_live = state
            .peek_agent(child.id())
            .await
            .expect("restart removed the live child");
        assert!(Arc::ptr_eq(&still_live, &agent));
        let actual_provider = still_live.provider().await.unwrap();
        assert!(
            Arc::ptr_eq(&actual_provider, &expected_provider),
            "restart replaced the live child's provider-local session"
        );
        assert!(
            still_live
                .list_tools(child.id(), None)
                .await
                .iter()
                .any(|tool| {
                    tool.name == biorouter::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME
                }),
            "restart replaced the child runtime profile with ordinary session tools"
        );
    }
}

#[cfg(test)]
mod cross_affiliation_grant_route_tests {
    //! Issue #56 DR-26 / Task 49: what the grant route's four refusals may and
    //! may not say, and the one shape of the handler that cannot regress
    //! silently.
    //!
    //! The behaviour these guard is not testable at the HTTP layer here —
    //! `AppState::new()` opens the developer's REAL session database (see
    //! `working_dir_lock_tests`) — so the copy is asserted directly, the way
    //! `routes::session`'s `the_refusals_say_different_things` asserts the
    //! declassification pair.

    use super::{
        CROSS_AFFILIATION_GRANT_CHAT_NOT_LOADED, CROSS_AFFILIATION_GRANT_NEEDS_USER,
        CROSS_AFFILIATION_GRANT_NOTHING_TO_ACCEPT, CROSS_AFFILIATION_GRANT_NO_KEY,
    };
    use axum::http::StatusCode;

    const SOURCE: &str = include_str!("agent.rs");

    /// ⚠ **The grant route INSPECTS a chat, and an inspection must never mint
    /// one.** `AppState::get_agent` is `AgentManager::get_or_create_agent`,
    /// whose own sibling `peek_agent` documents the hazard verbatim: it reads
    /// the process-wide mode at creation time and leaves a bare, provider-less,
    /// extension-less agent cached under that session id.
    ///
    /// Three things go wrong on that miss path, and all three are silent. The
    /// minted agent enumerates no extensions, so a user who just watched a
    /// refusal in that very chat is told there is nothing to accept — the
    /// control is unusable in exactly the case it is needed, after a daemon
    /// restart or an LRU eviction. Its provider is the process default rather
    /// than the chat's, so the "one sample" the whole route is built around
    /// would be a sample of the wrong model's affiliation. And the bare agent
    /// stays cached under a real session id for whoever asks next.
    ///
    /// Both directions of the negative control, because `body_of` reads forward
    /// from a signature to the next column-0 `}`: `agent_add_extension` sits
    /// BEFORE this handler in the file and `agent_remove_extension` AFTER, and
    /// each legitimately creates. An over-reading extractor reports one of them
    /// as peeking, or reports this handler as creating.
    #[test]
    fn the_grant_route_looks_up_the_live_agent_and_never_creates_one() {
        // Assembled, so this assertion is not itself a match for the scan.
        let peek = concat!("peek", "_agent(");
        let create = concat!("get", "_agent(");

        let handler = crate::routes::body_of(SOURCE, "async fn agent_cross_affiliation_grant");
        assert!(
            handler.contains(peek),
            "the grant route no longer looks the chat up without creating it"
        );
        assert!(
            !handler.contains(create),
            "the grant route mints an agent to inspect a chat. On a miss that agent has the \
             process default provider and no extensions, so the affiliation the grant is keyed \
             on is the wrong model's and the user is told there is nothing to accept."
        );

        for (name, body) in [
            (
                "agent_add_extension",
                crate::routes::body_of(SOURCE, "async fn agent_add_extension"),
            ),
            (
                "agent_remove_extension",
                crate::routes::body_of(SOURCE, "async fn agent_remove_extension"),
            ),
        ] {
            assert!(
                body.contains(create),
                "{name} is the control for this scan and must still create on miss"
            );
            assert!(
                !body.contains(peek),
                "the body scan is over-reading: {name} reported the grant route's lookup"
            );
        }
    }

    /// ⚠ **The one piece of this route's behaviour that IS reachable from a
    /// test, so it is asserted rather than grepped for.**
    ///
    /// Everything else here is a source scan, for the reason the module header
    /// gives. That leaves the most important claim of all — *only the user may
    /// grant*, resting on `handler.contains("user_action_proof(")`, which
    /// survives a refactor that turns the `Unproven` arm into `=> {}`. The
    /// mapping from a proof verdict to a refusal is pure, so it lives in
    /// [`super::refuse_grant_unless_user`] and this drives all three arms.
    #[test]
    fn only_a_proven_user_action_gets_past_the_grant_guard() {
        // Through the LIB path, as the handler above imports it: this module is
        // compiled into the `biorouterd` binary too, where `crate::auth` does
        // not exist.
        use biorouter_server::auth::UserActionProof;

        super::refuse_grant_unless_user(UserActionProof::Proven)
            .expect("a proven user action is the one verdict that proceeds");

        let unproven = super::refuse_grant_unless_user(UserActionProof::Unproven)
            .expect_err("a caller with no proof of a human may not accept a compliance risk");
        assert_eq!(unproven.status, StatusCode::FORBIDDEN);
        assert_eq!(unproven.message, CROSS_AFFILIATION_GRANT_NEEDS_USER);

        let keyless = super::refuse_grant_unless_user(UserActionProof::NoKeyInstalled)
            .expect_err("a daemon that cannot verify a human refuses everyone, including them");
        assert_eq!(keyless.status, StatusCode::FORBIDDEN);
        assert_eq!(keyless.message, CROSS_AFFILIATION_GRANT_NO_KEY);
    }

    /// Task 52 gate (1), the half that lives on this route: **`strict` rejects a
    /// grant carrying only `X-User-Action`**, and the other two modes accept
    /// one.
    ///
    /// The prompter counts, because the two ways to get this wrong are opposite
    /// and a test that only checked the `strict` outcome would miss the other:
    /// prompting in `standard` is the DR-19 prompt fatigue this feature keeps
    /// arguing against, and prompting in none is `strict` not existing.
    #[tokio::test]
    async fn strict_demands_the_operating_system_on_top_of_the_header() {
        use biorouter::privacy::mixing::MixingPolicy;
        use biorouter::privacy::system_auth::{AuthOutcome, AuthRequest, SystemAuthenticator};
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Prompter {
            answer: AuthOutcome,
            prompts: AtomicUsize,
        }

        #[async_trait::async_trait]
        impl SystemAuthenticator for Prompter {
            async fn authenticate(&self, _req: &AuthRequest) -> AuthOutcome {
                self.prompts.fetch_add(1, Ordering::Relaxed);
                self.answer
            }

            fn platform(&self) -> &'static str {
                "counting test prompter"
            }
        }

        // `open` and `standard` are today's behaviour: the in-app confirmation
        // this handler already checked is the whole of it, and a prompter that
        // would refuse is never consulted.
        for mode in [MixingPolicy::Open, MixingPolicy::Standard] {
            let prompter = Prompter {
                answer: AuthOutcome::Denied,
                prompts: AtomicUsize::new(0),
            };
            super::strict_mode_authorization(mode, &prompter, "ucsfomopagent")
                .await
                .unwrap_or_else(|e| panic!("{mode} must not need a password: {}", e.message));
            assert_eq!(
                prompter.prompts.load(Ordering::Relaxed),
                0,
                "{mode} raised a system prompt. Only `strict` costs something real; making \
                 the common case expensive is how you teach people to stop using the control"
            );
        }

        // `strict`: the same request, with the same header, is refused.
        let denied = Prompter {
            answer: AuthOutcome::Denied,
            prompts: AtomicUsize::new(0),
        };
        let refusal =
            super::strict_mode_authorization(MixingPolicy::Strict, &denied, "ucsfomopagent")
                .await
                .expect_err("in `strict` the in-app confirmation is not enough on its own");
        assert_eq!(refusal.status, StatusCode::FORBIDDEN);
        assert_eq!(denied.prompts.load(Ordering::Relaxed), 1);
        assert!(
            refusal.message.contains("strict"),
            "the refusal must name the mode in force, or the user reads it as a bug: {}",
            refusal.message
        );
        assert!(
            refusal.message.contains("Nothing was recorded"),
            "{}",
            refusal.message
        );

        // …and clears once the operating system approves.
        let approved = Prompter {
            answer: AuthOutcome::Approved,
            prompts: AtomicUsize::new(0),
        };
        super::strict_mode_authorization(MixingPolicy::Strict, &approved, "ucsfomopagent")
            .await
            .expect("an approved system authentication is what `strict` costs");
        assert_eq!(approved.prompts.load(Ordering::Relaxed), 1);

        // A machine that cannot raise a prompt at all refuses rather than
        // approving — DR-24's asymmetry, unchanged.
        let unavailable = Prompter {
            answer: AuthOutcome::Unavailable,
            prompts: AtomicUsize::new(0),
        };
        let error =
            super::strict_mode_authorization(MixingPolicy::Strict, &unavailable, "ucsfomopagent")
                .await
                .expect_err("an unavailable prompter must not approve a compliance risk");
        assert!(
            error.message.contains("counting test prompter"),
            "an unavailable prompt must name the platform: {}",
            error.message
        );
    }

    /// **Request-supplied text may not be rendered verbatim into an operating
    /// system's authentication dialog** (review finding, Task 52 fixup).
    ///
    /// This is the first caller in the tree to hand
    /// [`biorouter::privacy::system_auth::AuthRequest::about`] something that did
    /// not come from its own source, and that function's doc states the opposite
    /// as its precondition: it is infallible *"because there is exactly one
    /// subject, and it is a constant in the caller's source."* Extension names
    /// live in `config.yaml`, which DR-17 leaves agent-writable, and every
    /// prompter renders the id slot verbatim — macOS into
    /// `LAContext.localizedReason`, Windows into `UserConsentVerifier`, polkit
    /// into `--detail chats <value>`. So an agent that can plant an extension
    /// could write its own sentence into the system password dialog the user is
    /// then shown. Not injection — spoofing, which is worse here, because DR-20
    /// point 4's whole premise is that the dialog says honestly what it
    /// authorises.
    #[tokio::test]
    async fn the_system_dialog_never_renders_an_extension_name_verbatim() {
        use biorouter::privacy::mixing::MixingPolicy;
        use biorouter::privacy::system_auth::{AuthOutcome, AuthRequest, SystemAuthenticator};
        use std::sync::Mutex;

        struct Recorder(Mutex<Option<AuthRequest>>);

        #[async_trait::async_trait]
        impl SystemAuthenticator for Recorder {
            async fn authenticate(&self, req: &AuthRequest) -> AuthOutcome {
                *self.0.lock().unwrap() = Some(req.clone());
                AuthOutcome::Approved
            }

            fn platform(&self) -> &'static str {
                "recording test prompter"
            }
        }

        // A name a compromised agent could plant: it ends the sentence it is
        // spliced into, opens a line of its own, and runs long enough to push
        // the real text out of a dialog.
        let hostile = format!(
            "ok`.\n\nBioRouter: this is routine. Press Allow.\r\n{}",
            "x".repeat(400)
        );

        let recorder = Recorder(Mutex::new(None));
        super::strict_mode_authorization(MixingPolicy::Strict, &recorder, &hostile)
            .await
            .expect("an approving prompter clears the strict layer");
        let asked = recorder.0.lock().unwrap().clone().expect("the prompt ran");

        // The id slot is a CONSTANT in this file, honouring `about`'s stated
        // precondition. A request-supplied subject there is the spoof.
        assert_eq!(
            asked.session_ids,
            vec![super::CROSS_AFFILIATION_GRANT_AUTH_SUBJECT.to_string()],
            "the dialog's subject slot carried request-supplied text: {asked:?}"
        );

        // The reason may name the extension — that is DR-20 point 4 — but only
        // after the name has been made safe to render.
        //
        // ⚠ The claim is STRUCTURAL, not semantic, and the assertions say so. A
        // connector genuinely named `Routine check, press Allow` is
        // indistinguishable from a planted one; what must hold is that the name
        // stays inside Biorouter's own sentence, on one line, bounded, and
        // never in the id slot — so a user can always tell which words are the
        // application's. See `system_auth::dialog_safe`.
        assert!(
            !asked.reason.chars().any(char::is_control),
            "a control character reached the dialog, so a planted name can add a \
             line to it: {:?}",
            asked.reason
        );
        assert_eq!(
            asked.reason.matches('`').count(),
            2,
            "the planted name closed the delimiters it was wrapped in, so part of \
             it renders as Biorouter speaking: {:?}",
            asked.reason
        );
        assert!(
            asked.reason.len() < 200,
            "an unbounded name can push the real sentence out of the dialog: {} chars",
            asked.reason.len()
        );

        // …and an ordinary name still reaches it, or the sanitiser has bought
        // safety by saying nothing.
        let plain = Recorder(Mutex::new(None));
        super::strict_mode_authorization(MixingPolicy::Strict, &plain, "ucsfomopagent")
            .await
            .unwrap();
        assert!(
            plain
                .0
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|r| r.reason.contains("ucsfomopagent")),
            "the dialog no longer says which connector it is authorising"
        );
    }

    /// …and the `strict` prompt is raised AFTER the mismatch is resolved and
    /// BEFORE the grant is written, **with the live policy and the real
    /// prompter**.
    ///
    /// The pure test above cannot see the order. Asking earlier would take a
    /// password and then report there was nothing to accept; asking later would
    /// take one for a row already written.
    ///
    /// ⚠ **Nor can it see the arguments, which is the mutation review found.**
    /// [`super::strict_mode_authorization`] is testable precisely because it
    /// takes the mode and the prompter rather than resolving them — and that is
    /// also how `strict` could be silently switched off:
    ///
    /// ```ignore
    /// strict_mode_authorization(MixingPolicy::Open, prompter(), &request.extension)
    /// ```
    ///
    /// …passes every behavioural test in this module, both gate commands and the
    /// ordering scan above, while no machine ever raises the prompt again. So the
    /// production call's own text is asserted: the mode comes from the resolver
    /// and the prompter from DR-24's, neither from a literal.
    #[test]
    fn the_strict_prompt_sits_between_the_resolution_and_the_write() {
        let handler = crate::routes::body_of(SOURCE, "async fn agent_cross_affiliation_grant");
        let resolved = handler
            .find("cross_affiliation_grant_subject(")
            .expect("the grant handler no longer resolves the mismatch it is accepting");
        let prompted = handler
            .find(concat!("strict_mode_", "authorization("))
            .expect("the grant handler no longer applies DR-27's strict layer");
        let written = handler
            .find(concat!("grant::", "record("))
            .expect("the grant handler no longer writes the grant");
        assert!(
            resolved < prompted && prompted < written,
            "the strict prompt is outside the window it has to sit in: resolved at \
             {resolved}, prompted at {prompted}, written at {written}"
        );

        // The call runs from `strict_mode_authorization(` to the `.await?` that
        // ends the statement. Line-wise rather than by byte slice, for the reason
        // `mixing.rs`'s writer audit records: `clippy::string_slice` is
        // warn-by-default here and `-D warnings` makes it an error.
        let mut call = String::new();
        let mut inside = false;
        for line in handler.lines() {
            inside |= line.contains(concat!("strict_mode_", "authorization("));
            if inside {
                call.push_str(line);
                call.push('\n');
                if line.contains(".await?") {
                    break;
                }
            }
        }
        assert!(
            call.contains(".await?"),
            "the strict layer's call no longer ends where this scan expects: {call}"
        );
        assert!(
            call.contains(concat!("mixing::", "policy()")),
            "the strict layer is no longer handed the LIVE mixing policy, so a \
             machine in `strict` may never be asked: {call}"
        );
        assert!(
            call.contains(concat!("system_auth::", "prompter()")),
            "the strict layer is no longer handed DR-24's real prompter: {call}"
        );
    }

    /// …and the guard runs BEFORE the chat is looked up, so an unproven caller
    /// cannot use the refusals to probe which extensions a chat has.
    ///
    /// The pure test above cannot see the order; only the source can.
    #[test]
    fn the_guard_runs_before_the_chat_is_touched() {
        let handler = crate::routes::body_of(SOURCE, "async fn agent_cross_affiliation_grant");
        let guarded = handler
            .find(concat!("refuse_grant_unless_user(", "user_action_proof("))
            .expect(
                "the grant handler no longer passes the real guard's verdict to the refusal \
                 mapping",
            );
        let looked_up = handler
            .find(concat!("peek", "_agent("))
            .expect("the grant handler no longer looks the chat up");
        assert!(
            guarded < looked_up,
            "the proof is checked AFTER the chat is inspected, so an unproven caller can \
             distinguish a chat with the extension from one without"
        );
    }

    /// …and the refusal that miss produces says what actually happened.
    ///
    /// Reporting "this chat is not loaded" as "there is nothing to accept" is
    /// the same class of error open question 23 rejected for the keyless daemon:
    /// it sends the person at the keyboard looking for a mismatch to fix when
    /// what they need is to open the chat.
    #[test]
    fn a_chat_that_is_not_loaded_is_told_so_and_not_that_there_is_nothing_to_accept() {
        assert_ne!(
            CROSS_AFFILIATION_GRANT_CHAT_NOT_LOADED,
            CROSS_AFFILIATION_GRANT_NOTHING_TO_ACCEPT
        );
        assert!(CROSS_AFFILIATION_GRANT_CHAT_NOT_LOADED.contains("Nothing was recorded"));
        // It names the act that clears it, which is neither a retry nor a
        // setting: open the chat again.
        assert!(CROSS_AFFILIATION_GRANT_CHAT_NOT_LOADED.contains("Open the chat"));
    }

    /// A refusal that carries a renderer marker is claiming to be a different
    /// refusal, and the toast it triggers sends the user somewhere that cannot
    /// help: `USER_ACTION_REFUSAL_MARKER`'s says *switch this chat's model* (this
    /// chat is already private), and `COPY_OF_PRIVATE_REFUSAL_MARKER`'s says
    /// *branch it from the chat window* (nothing is being branched).
    #[test]
    fn no_grant_refusal_claims_another_refusals_marker() {
        for message in [
            CROSS_AFFILIATION_GRANT_NEEDS_USER,
            CROSS_AFFILIATION_GRANT_NO_KEY,
            CROSS_AFFILIATION_GRANT_NOTHING_TO_ACCEPT,
            // Task 52's fifth refusal, enrolled here rather than left to be
            // remembered: it is the newest and therefore the likeliest to have
            // been written by copying one of the others.
            super::CROSS_AFFILIATION_GRANT_STRICT_NEEDS_SYSTEM,
        ] {
            assert!(
                !message.contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER),
                "this refusal is claiming to be the model picker's: {message}"
            );
            assert!(
                !message.contains(super::super::session::COPY_OF_PRIVATE_REFUSAL_MARKER),
                "this refusal is claiming to be a copy handler's: {message}"
            );
        }
    }

    /// The two proof refusals say different things, which is the whole reason
    /// Task 49 takes the three-way `user_action_proof` over the boolean:
    /// reporting "this daemon cannot verify a human" as "you are not a human"
    /// sends the person at the keyboard hunting for a permission they can never
    /// obtain (open question 23).
    #[test]
    fn a_keyless_daemon_is_told_something_a_caller_with_no_proof_is_not() {
        assert_ne!(
            CROSS_AFFILIATION_GRANT_NEEDS_USER,
            CROSS_AFFILIATION_GRANT_NO_KEY
        );
        assert!(CROSS_AFFILIATION_GRANT_NO_KEY.contains("without a user-action key"));
        // …and neither of them accuses the user of being the model.
        assert!(CROSS_AFFILIATION_GRANT_NEEDS_USER.contains("person at the keyboard"));
    }

    /// Every refusal in this feature forecloses the retry, because a model that
    /// reads one as transient loops on it — and this one also has to name the
    /// human act, since the human act is the entire product of DR-26.
    #[test]
    fn the_proof_refusal_forecloses_the_retry_and_names_the_human_act() {
        let m = CROSS_AFFILIATION_GRANT_NEEDS_USER;
        assert!(m.contains("Do not"), "{m}");
        assert!(m.contains("Nothing was recorded"), "{m}");
        assert!(m.contains("let them approve it"), "{m}");
    }
}

#[cfg(test)]
mod privacy_barrier_tests {
    //! Issue #56 Gate A, route half: an `Agent::update_provider` refusal must
    //! reach the client as a typed 409, and everything else must keep its 500.
    //!
    //! Exercised through [`classify_provider_bind_failure`] — the exact mapping
    //! the handler applies — rather than through `AppState`, which opens the
    //! REAL user session database.

    use super::{classify_provider_bind_failure, PrivacyBarrierBody, ProviderBindFailure};
    use axum::http::StatusCode;
    use biorouter::privacy::refusal::PrivacyRefusal;
    use biorouter::privacy::{ProviderTier, SessionClassification};

    fn refusal() -> anyhow::Error {
        PrivacyRefusal::PublicModelOnPrivateSession {
            session_id: "20260801_3".into(),
            provider: "anthropic".into(),
        }
        .into()
    }

    #[test]
    fn a_privacy_refusal_becomes_a_typed_409() {
        let failure = classify_provider_bind_failure(
            &refusal(),
            vec!["llamacpp".to_string(), "versa_azure".to_string()],
        );
        assert_eq!(failure.status(), StatusCode::CONFLICT);
        let ProviderBindFailure::Privacy(body) = failure else {
            panic!("a privacy refusal must not be classified as an internal error");
        };
        assert_eq!(body.code, PrivacyBarrierBody::CODE);
        assert_eq!(body.session_classification, SessionClassification::Private);
        assert_eq!(body.provider_tier, ProviderTier::Public);
        assert_eq!(
            body.available_private_providers,
            ["llamacpp", "versa_azure"]
        );
    }

    #[test]
    fn every_other_failure_keeps_its_500() {
        // The pre-#56 behaviour, pinned: a database or provider failure must not
        // start telling the user their chat is private.
        let failure = classify_provider_bind_failure(
            &anyhow::anyhow!("Failed to persist provider config to session"),
            vec![],
        );
        assert_eq!(failure.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            matches!(failure, ProviderBindFailure::Internal(_)),
            "a non-privacy failure must not be rendered as a privacy barrier"
        );
    }

    /// The refusal reaches the handler wrapped in the context
    /// `Agent::update_provider` adds, so the classifier must look through the
    /// whole chain rather than at the outermost error.
    #[test]
    fn a_refusal_is_still_found_under_an_added_context() {
        use anyhow::Context;
        let wrapped = Err::<(), _>(refusal())
            .context("while switching this chat's model")
            .unwrap_err();
        assert_eq!(
            classify_provider_bind_failure(&wrapped, vec![]).status(),
            StatusCode::CONFLICT
        );
    }

    /// §14.4: the body is what the user reads, and it must not carry
    /// conversation content — only the two tiers and the way forward.
    #[test]
    fn the_409_body_carries_no_session_identity() {
        let failure = classify_provider_bind_failure(&refusal(), vec!["ollama".to_string()]);
        let ProviderBindFailure::Privacy(body) = failure else {
            panic!("expected a privacy barrier");
        };
        let json = serde_json::to_string(&*body).unwrap();
        assert!(
            !json.contains("20260801_3"),
            "the 409 body leaked the session id: {json}"
        );
    }
}

#[cfg(test)]
mod add_extension_resolver_tests {
    //! Issue #56 Task 43 (DR-23), route half — and a **structural** test, said
    //! so plainly, because a behavioural one is not reachable from this crate.
    //!
    //! `/agent/add_extension` is one of the three callers that never had a
    //! stamped tier to read and re-classified from a bare name, so a private
    //! extension renamed in `config.yaml` could be attached to a public session
    //! over HTTP. Step 1 moved it to `classify_extension_entry`, passing the
    //! config as well as the name — the config is what carries the install
    //! directory, and the install directory is the only link a rename does not
    //! break.
    //!
    //! ⚠ **Why this is not a behavioural test.** Driving the real handler needs
    //! an `AppState`, which opens the real user session database (the module
    //! above says so for Gate A and takes the same way out), and the seam that
    //! states an extension's provenance — `provenance::insert_test_record` — is
    //! `#[cfg(test)] pub(crate)` in `biorouter`, so it does not exist for this
    //! crate at all. Making it exist would mean shipping a test-record injector
    //! in the release binary or adding a feature flag to a security module for
    //! the sake of one assertion; neither is worth it for a call that can only
    //! ever RAISE a tier.
    //!
    //! So this asserts the one thing that can silently regress: that the gate
    //! still hands the resolver the config. `classify_extension` compiles just
    //! as happily here and would take the refusal back to the name-only join
    //! without any test noticing — which is exactly how this bug lived in three
    //! places. The rename behaviour itself is covered where the seam exists, in
    //! `biorouter`'s `privacy::extensions` and `agents::extension_manager`
    //! tests.
    //!
    //! The `include_str!` pattern is `privacy::config_keys`'s, which scans
    //! provider sources the same way and for the same reason.

    const SOURCE: &str = include_str!("agent.rs");

    /// Whitespace-insensitive, so rustfmt reflowing the call does not fail it.
    fn squeezed() -> String {
        SOURCE.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// ⚠ The needle is assembled at run time, and finding out why is what the
    /// mutation check was for: written as one literal it appears in this very
    /// file, so the scan found its own assertion and passed against a handler
    /// mutated back to the name-only form. Split across the `format!`, neither
    /// half is the needle and only the real call site can satisfy it.
    ///
    /// ⚠ **Task 48 (DR-26) tightened this from `classify_extension_entry` to
    /// `resolve_extension`, which is a strengthening rather than a rename.**
    /// The entry form is this one's `tier` field; the route now needs the
    /// AFFILIATION too, and taking the two from separate calls is what lets
    /// them disagree about the same entry — a connector coming back Private
    /// *and* unconstrained, which is the whole failure DR-26 exists to catch.
    /// One resolution, one record, both axes.
    #[test]
    fn the_add_extension_gate_resolves_from_the_config_not_from_the_name() {
        let needle = format!(
            "{}(&extension_name,Some(&request.config))",
            "resolve_extension"
        );
        assert!(
            squeezed().contains(&needle),
            "`/agent/add_extension` no longer hands the resolver the config it was given. \
             A renamed private extension resolves Public from its name alone, so this route \
             would attach a clinical connector to a public session; see DR-23. It must also \
             be the resolution that carries the AFFILIATION, or the two axes can disagree \
             about the same entry; see DR-26."
        );
    }

    /// Issue #56 Task 48 (DR-26), the **user's** enable path — row 5 of Step
    /// 2's table, and the half nothing pinned.
    ///
    /// The agent's half (`check_enable_allowed`) has three behavioural tests in
    /// `biorouter`. This one had none: the gate above it only asserts that
    /// `resolve_extension` is called with the config, which the TIER arm
    /// already satisfies, so the entire DR-26 block could be deleted and every
    /// test in the tree would stay green.
    ///
    /// Structural for the reason the module header gives — `AppState` opens the
    /// real user session database and `provenance::insert_test_record` does not
    /// exist for this crate — so this pins the two things that can silently
    /// regress, in the order they would regress in:
    ///
    ///  1. the route stops asking the gate at all, and a cross-institutional
    ///     attach happens with nothing recorded anywhere;
    ///  2. someone "fixes the inconsistency" with the agent path by turning the
    ///     warning into a refusal. That is DR-26's user/agent asymmetry
    ///     inverted, not a tidy-up: a user who insists proceeds past a warning,
    ///     an agent never clears one automatically. Refusing here would also
    ///     leave a legitimate cross-institutional user under a real DUA with no
    ///     way to attach their own connector at all, which is the "researchers
    ///     turn the feature off" outcome the ruling exists to avoid.
    ///
    /// ⚠ Both needles are assembled at run time, for the reason the test above
    /// documents: written as literals they would appear in this very file and
    /// the scan would find its own assertion.
    #[test]
    fn the_add_extension_route_warns_on_a_mismatch_and_still_attaches() {
        let squeezed = squeezed();
        let gate = format!("gate_cross_affiliation{}(", "_warning");
        let attach = format!(".add_extension{}", "(request.config)");

        let asked = squeezed.find(&gate).expect(
            "`/agent/add_extension` no longer asks the cross-affiliation gate. A user attaching \
             another institution's connector to a chat bound to their own institution's model is \
             the mismatch DR-26 exists to state, and this route is the surface it is stated on.",
        );
        let attached = squeezed
            .find(&attach)
            .expect("`/agent/add_extension` no longer attaches the extension it was given");
        assert!(
            asked < attached,
            "the mismatch must be detected BEFORE the extension is attached, not after"
        );

        // Everything the route does between deciding and attaching. On a
        // mismatch it must state the risk and carry on.
        //
        // `get` rather than a slice: both indices come from `find`, so both are
        // already char boundaries, but the fallible form says that rather than
        // asserting it — and this file is not ASCII (the refusal copy carries
        // en-dashes), so a future needle landing mid-character would panic the
        // test with a UTF-8 error instead of the message it is here to give.
        let between = squeezed
            .get(asked..attached)
            .expect("both indices come from `find`, so both are UTF-8 boundaries");
        assert!(
            between.contains("tracing::warn!"),
            "the mismatch is detected and then dropped on the floor: {between}"
        );
        assert!(
            !between.contains("returnErr"),
            "DR-26 warns at the USER's enable path; it refuses only at the AGENT's \
             (`check_enable_allowed`). Turning this into a refusal inverts the asymmetry and \
             strands a legitimate cross-institutional user: {between}"
        );
    }

    /// The name-only form must not reappear anywhere in this file. It is not
    /// wrong in itself — it is the right call for a caller that genuinely holds
    /// only a name — but no route here is such a caller, and its presence is
    /// what the regression would look like.
    /// ⚠ The two needles are assembled at run time rather than written as
    /// literals, because a literal `classify_extension` + `(` in this file would
    /// be found by the scan and fail it against itself.
    #[test]
    fn no_route_in_this_file_classifies_from_a_bare_name() {
        let bare = format!("classify_extension{}", '(');
        let entry = format!("classify_extension_entry{}", '(');
        for line in SOURCE.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            assert!(
                !code.contains(&bare) || code.contains(&entry),
                "a route classified an extension from its name alone, which a rename defeats: \
                 {line}"
            );
        }
    }
}

#[cfg(test)]
mod cross_affiliation_notice_route_tests {
    //! Issue #56 DR-26, the two surfaces where "warn the user, naming both
    //! institutions, before proceeding" was **log-only**, and the shape of the
    //! repair that can silently come undone.
    //!
    //! The daemon has detected this mismatch at the bind (`Agent::update_provider`)
    //! and at the user's own enable (`POST /agent/add_extension`) since Task 48,
    //! and both wrote it to `tracing::warn!` and nowhere else. The dispatch
    //! surface's accept card worked; these two told the person at the keyboard
    //! nothing, so a researcher attaching another institution's connector from
    //! the extension picker learned about the boundary when a tool call was
    //! refused, if at all.
    //!
    //! ⚠ **Structural, and the module above says why at length**: `AppState::new()`
    //! opens the developer's real session database, so neither handler can be
    //! driven here. What CAN regress silently is the wiring, and it is one line
    //! per handler appended to a `try` block that already looked finished. The
    //! behaviour behind those lines — that the notice names both institutions,
    //! and goes quiet for a flow the user has already accepted — is driven end to
    //! end where the seams exist, in `biorouter`'s
    //! `agents::agent::gate_c_dispatch_tests::
    //! the_bind_notice_names_both_institutions_and_goes_quiet_once_accepted`.

    const SOURCE: &str = include_str!("agent.rs");

    /// Assembled at run time for the reason the two modules above document: a
    /// literal would appear in this very file and every scan below would find
    /// its own assertion instead of the call site.
    fn notice() -> String {
        format!("cross_affiliation{}(", "_notice")
    }

    /// The handler's return type, read from its own signature.
    ///
    /// ⚠ A whole-file `contains` would not do: several handlers in this file
    /// share a return type, so the assertion could be satisfied by a NEIGHBOUR's
    /// signature while the one under test had been reverted to `()`.
    fn returns(signature: &str) -> String {
        let after = SOURCE
            .split_once(signature)
            .unwrap_or_else(|| panic!("{signature} is no longer in this file"))
            .1;
        after
            .split_once(" {\n")
            .expect("every handler signature ends at its opening brace")
            .0
            .to_string()
    }

    /// The BIND surface. `POST /agent/update_provider` returned a bare `()` and
    /// the model picker showed a green success toast over a mismatch nobody had
    /// mentioned.
    ///
    /// Two claims, and the second is the one a refactor breaks: the handler asks
    /// for the notice, and it **returns** it. A version that computed it and
    /// dropped it on the floor is the exact defect being repaired, one layer up.
    #[test]
    fn the_bind_route_hands_back_the_warning_it_used_to_only_log() {
        let handler = crate::routes::body_of(SOURCE, "async fn update_agent_provider");
        assert!(
            handler.contains(&notice()),
            "`/agent/update_provider` no longer asks for DR-26's bind statement. Binding a \
             model covered by another institution's agreements into a chat holding this \
             institution's connectors is the mismatch the ruling exists to state, and this \
             route is the one the model picker calls."
        );
        // The signature, not the body: a handler that returns `()` cannot carry
        // the statement however carefully it composed it.
        let signature = returns("async fn update_agent_provider(");
        assert!(
            signature.contains("-> Result<String,"),
            "`/agent/update_provider` no longer returns a body, so the warning it composes \
             cannot reach the user, which is the state this fix found it in: {signature}"
        );
    }

    /// The USER's enable surface, and the one the ruling names most directly.
    ///
    /// ⚠ The notice is read **after** the attach, and the ordering is asserted
    /// rather than assumed. Read before it, the answer describes a chat that no
    /// longer exists by the time the caller sees it and — worse — omits the very
    /// extension the request was about, so the surface would go quiet in exactly
    /// the case it exists for.
    #[test]
    fn the_enable_route_hands_back_the_warning_it_used_to_only_log() {
        let handler = crate::routes::body_of(SOURCE, "async fn agent_add_extension");
        let attach = format!(".add_extension{}", "(request.config)");

        let asked = handler.find(&notice()).expect(
            "`/agent/add_extension` no longer hands the user DR-26's statement. It logged this \
             mismatch and nothing else for its whole life, which is how a user enabling a \
             foreign connector from the extension picker came to be told nothing at all.",
        );
        let attached = handler
            .find(&attach)
            .expect("`/agent/add_extension` no longer attaches the extension it was given");
        assert!(
            attached < asked,
            "the notice is composed BEFORE the extension is attached, so it answers for a chat \
             that no longer exists, and omits the connector the request was about"
        );
        let signature = returns("async fn agent_add_extension(");
        assert!(
            signature.contains("-> Result<String,"),
            "`/agent/add_extension` no longer returns a body, so the warning it composes cannot \
             reach the user: {signature}"
        );
    }

    /// The half that is a correctness bug rather than a wiring one.
    ///
    /// The notice suppresses flows the user has already accepted, and a grant is
    /// keyed on (session, extension, **model affiliation**). Both handlers must
    /// pass the affiliation of the provider they themselves hold — the one the
    /// bind just created, the one the enable read once for both privacy axes.
    /// `Agent::update_provider` reassigns the provider mutex with no turn lock,
    /// so a second read here could key the lookup on a model some other caller
    /// bound in between and suppress a warning on an acceptance that was never
    /// given for the model in question. That failure is silent and it fails
    /// OPEN — the user is not warned.
    ///
    /// So: exactly one binding of the affiliation per handler, and the notice
    /// call reads that binding.
    #[test]
    fn each_route_keys_the_notice_on_the_provider_it_holds_itself() {
        let bind = format!("let model_{} =", "affiliation");
        for name in [
            "async fn update_agent_provider",
            "async fn agent_add_extension",
        ] {
            let handler = crate::routes::body_of(SOURCE, name);
            let bindings = handler.matches(&bind).count();
            assert_eq!(
                bindings, 1,
                "{name} binds the model affiliation {bindings} times. Two reads of the provider \
                 are the read-then-read `CallCapability` exists to collapse: the route can warn \
                 about one institution while checking an acceptance recorded for another, and \
                 the failure is a warning that is never shown."
            );
            let at_bind = handler.find(&bind).expect("just counted one");
            let at_notice = handler.find(&notice()).expect("pinned by the tests above");
            assert!(
                at_bind < at_notice,
                "{name} composes the notice before it has the affiliation to key it on"
            );
        }
    }
}

#[cfg(test)]
mod gate_c_call_tool_tests {
    //! Issue #56 Gate C, route half. `POST /agent/call_tool` is one of the four
    //! production paths into `ExtensionManager::dispatch_tool_call`, and the
    //! only one whose caller is an HTTP client rather than a model — so the
    //! refusal has to arrive as the tool's own result. It used to be thrown
    //! away by `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)`, which told the
    //! caller nothing and told the model, when a client relayed it, that
    //! Biorouter had crashed.
    //!
    //! Exercised through [`dispatch_failure_response`] — the exact mapping the
    //! handler applies — rather than through `AppState`, which builds the
    //! process-global `AgentManager` and opens the REAL user session database.
    //! That is the same shape `privacy_barrier_tests` above uses for Gate A's
    //! route half, and the same reason.

    use super::dispatch_failure_response;
    use axum::http::StatusCode;
    use biorouter::privacy::refusal::privacy_refusal;
    use biorouter::privacy::ProviderTier;
    use rmcp::model::{ErrorCode, ErrorData};

    /// #152. The tests below prove the CLASSIFIER is right; none of them proved
    /// it was REACHED on the path that carries most tool errors, and for a long
    /// time it was not.
    ///
    /// `dispatch_tool_call` hands back a `ToolCallResult` whose `.result` is a
    /// future: dispatching a tool and RUNNING it are two separate fallible
    /// steps. The dispatch arm was repaired; the await three lines below it kept
    /// `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)`, so every error raised
    /// by the tool ITSELF — the common case — was answered `500` with a
    /// zero-length body while `rmcp` logged the real message at WARN. Measured
    /// on a 419-case drive: 45 of them.
    ///
    /// This asserts the shape of the repaired arm at the source, because the
    /// alternative is an `AppState` that opens the real user session database.
    /// A line-wise pin is weak evidence in general; it is the right strength
    /// here, because the defect was not a wrong CLASSIFICATION but a call site
    /// that never classified at all.
    #[test]
    fn the_tool_results_own_error_is_answered_as_a_tool_error_not_a_bare_500() {
        let source = include_str!("agent.rs");
        let awaited = source
            // ⚠ Anchor updated when the no-human-surface scope was widened to
            // cover RUNNING the tool: the await moved inside that scope and the
            // binding it produces is now `awaited`. The classification this
            // test pins is unchanged — re-read #152 before touching it again.
            .split("let result = match awaited {")
            .nth(1)
            .expect(
                "the tool-result await must be a `match` that inspects its error; if this \
                 shape changed, re-read #152 before updating the anchor",
            );
        // `split` rather than a byte-index slice: `clippy::string_slice` is denied
        // repo-wide, and indexing a string can land inside a UTF-8 character.
        let arm = awaited.split("};").next().unwrap_or(awaited);
        assert!(
            arm.contains("is_error: true") && arm.contains("error.message"),
            "the awaited tool error must reach the caller as its own message. #152: this arm \
             was `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)`, which discarded every \
             message a tool produced. Arm was:\n{arm}"
        );
        assert!(
            !arm.contains("INTERNAL_SERVER_ERROR"),
            "a tool that answered is not a server fault (#152). Arm was:\n{arm}"
        );
    }

    fn text_of(response: &super::CallToolResponse) -> String {
        response
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_gate_c_refusal_reaches_the_caller_rather_than_becoming_a_500() {
        let refusal = privacy_refusal("ucsfomopagent", ProviderTier::Private, ProviderTier::Public)
            .expect("a public caller on a private extension is refused");
        let response = dispatch_failure_response(&anyhow::Error::from(refusal))
            .expect("a refusal is an answer about the tool, not a server fault");
        assert!(response.is_error);
        let text = text_of(&response);
        assert!(text.contains("ucsfomopagent"), "{text}");
        assert!(text.contains("Settings"), "{text}");
    }

    /// Every OTHER refusal `dispatch_tool_call` raises is an `ErrorData` too, so
    /// the fix is not special-cased to Gate C: the pre-#56 500 was wrong for all
    /// of them, and a caller that asks for a tool that does not exist now learns
    /// which one.
    #[test]
    fn any_other_tool_level_failure_also_carries_its_reason() {
        let not_found = ErrorData::new(
            ErrorCode::RESOURCE_NOT_FOUND,
            "Tool 'nope__x' not found".to_string(),
            None,
        );
        let response = dispatch_failure_response(&anyhow::Error::from(not_found))
            .expect("a tool-level failure is a tool result");
        assert!(response.is_error);
        assert!(text_of(&response).contains("nope__x"));
    }

    /// …and the line the classification draws. Gate C's ratchet propagates a
    /// session-store failure with `?`, which is the first non-`ErrorData` error
    /// `dispatch_tool_call` can return. Rendering it as `200 + is_error` would
    /// tell an HTTP client the tool ran and disagreed — retiring its retry — and
    /// hand it raw store text. It keeps the 500 it had before this task.
    #[test]
    fn a_store_failure_is_not_laundered_into_a_tool_result() {
        // `CallToolResponse` is not `Debug`, so the outcome is matched rather
        // than `expect_err`'d.
        match dispatch_failure_response(&anyhow::anyhow!(
            "error returned from database: disk I/O error"
        )) {
            Ok(response) => panic!(
                "a session-store failure was rendered as a tool result: {}",
                text_of(&response)
            ),
            Err(status) => assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

#[cfg(test)]
mod working_dir_lock_tests {
    //! Route-level contract for `/agent/update_working_dir`'s empty-chat-only
    //! rule (#44), exercised through [`apply_working_dir_update`] — the exact
    //! validation + guard + update path the handler runs before restarting the
    //! agent. A hermetic tempdir-backed [`SessionManager`] stands in for the
    //! real one: `AppState::new()` opens the REAL user session database (see
    //! the note in `routes/session.rs`), so a mutating route test must never
    //! go through it.

    use super::apply_working_dir_update;
    use axum::http::StatusCode;
    use biorouter::conversation::message::Message;
    use biorouter::session::session_manager::SessionType;
    use biorouter::session::SessionManager;
    use tempfile::TempDir;

    /// A store + one session whose working dir is `start_dir`.
    async fn session_in(store: &TempDir, start_dir: &std::path::Path) -> (SessionManager, String) {
        let sm = SessionManager::new(store.path().to_path_buf());
        let session = sm
            .create_session(
                start_dir.to_path_buf(),
                "test session".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        (sm, session.id)
    }

    #[tokio::test]
    async fn an_empty_session_accepts_a_working_dir_change() {
        let store = TempDir::new().unwrap();
        let old_dir = TempDir::new().unwrap();
        let new_dir = TempDir::new().unwrap();
        let (sm, id) = session_in(&store, old_dir.path()).await;

        let updated = apply_working_dir_update(&sm, &id, new_dir.path().to_str().unwrap())
            .await
            .expect("an empty session's working dir is still choosable");
        assert_eq!(updated.working_dir, new_dir.path());

        // And the change is persisted, not just echoed.
        let reloaded = sm.get_session(&id, false).await.unwrap();
        assert_eq!(reloaded.working_dir, new_dir.path());
    }

    #[tokio::test]
    async fn a_session_with_a_message_locks_its_working_dir_with_409() {
        let store = TempDir::new().unwrap();
        let old_dir = TempDir::new().unwrap();
        let new_dir = TempDir::new().unwrap();
        let (sm, id) = session_in(&store, old_dir.path()).await;

        sm.add_message(&id, &Message::user().with_text("hello"))
            .await
            .unwrap();

        let err = apply_working_dir_update(&sm, &id, new_dir.path().to_str().unwrap())
            .await
            .expect_err("one message is enough to lock the working dir");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(
            err.message,
            "the working directory is fixed once a chat has messages"
        );

        // The rejected change must not have touched the session.
        let reloaded = sm.get_session(&id, false).await.unwrap();
        assert_eq!(reloaded.working_dir, old_dir.path());
    }

    #[tokio::test]
    async fn a_missing_session_is_a_404_not_a_silent_update() {
        let store = TempDir::new().unwrap();
        let new_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(store.path().to_path_buf());

        let err =
            apply_working_dir_update(&sm, "no_such_session", new_dir.path().to_str().unwrap())
                .await
                .expect_err("a missing session cannot accept a working dir");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_invalid_target_path_is_a_400() {
        let store = TempDir::new().unwrap();
        let old_dir = TempDir::new().unwrap();
        let (sm, id) = session_in(&store, old_dir.path()).await;

        for bad in ["", "   ", "/definitely/not/a/real/dir/for/br44"] {
            let err = apply_working_dir_update(&sm, &id, bad)
                .await
                .expect_err("invalid paths are rejected");
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "for {bad:?}");
        }
    }
}

#[cfg(test)]
mod knowledge_selection_tests {
    use super::{apply_workflow_knowledge_selection, plan_workflow_knowledge_selection};
    use biorouter::workflow::{Workflow, WorkflowKnowledgeBases};
    use biorouter_mcp::knowledge::service::KnowledgeService;
    use std::sync::Arc;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A workflow that lists five bases used to activate exactly one, silently
    /// (`selection.default.or(selection.visible.first())`). Under the merged
    /// model `{ default, visible }` already *is* "a set plus one primary", so
    /// the whole set applies and only `default` may set the pointer.
    #[test]
    fn workflow_applies_every_declared_base() {
        let selection = WorkflowKnowledgeBases {
            default: Some("c".to_string()),
            visible: ids(&["a", "b", "c", "d", "e"]),
        };
        let (visible, primary) = plan_workflow_knowledge_selection(&selection);
        assert_eq!(visible, ids(&["a", "b", "c", "d", "e"]));
        assert_eq!(primary.as_deref(), Some("c"));
    }

    /// No `default` means **no primary**, at every set size. A one-base
    /// workflow used to have its sole member promoted, which is the model's
    /// forbidden move — inventing a pointer the author never wrote. The
    /// author who wants that base to be the write target says so in `default`;
    /// the author who lists it without a `default` gets a session whose
    /// KB-less writes fail loudly naming `a` as the candidate.
    #[test]
    fn workflow_without_a_default_never_invents_a_primary() {
        let one = WorkflowKnowledgeBases {
            default: None,
            visible: ids(&["a"]),
        };
        assert_eq!(
            plan_workflow_knowledge_selection(&one).1,
            None,
            "a sole visible base is still not a primary the author chose"
        );

        let many = WorkflowKnowledgeBases {
            default: None,
            visible: ids(&["a", "b"]),
        };
        assert_eq!(plan_workflow_knowledge_selection(&many).1, None);
    }

    /// A `default` the author forgot to list is still the author's intent, and
    /// the invariant requires the primary to be a member — so union it in
    /// rather than dropping it.
    #[test]
    fn workflow_default_joins_the_set_when_it_was_not_listed() {
        let selection = WorkflowKnowledgeBases {
            default: Some("b".to_string()),
            visible: ids(&["a"]),
        };
        let (visible, primary) = plan_workflow_knowledge_selection(&selection);
        assert_eq!(
            visible,
            ids(&["a", "b"]),
            "b must be in the set: it is the primary"
        );
        assert_eq!(primary.as_deref(), Some("b"));
    }

    /// The workflow's set must be inverted against the installed bases *inside*
    /// the lock. Applying it used to list the bases first, unlocked, build a
    /// hidden list from that snapshot, and only then take the lock to write it —
    /// so a base created in that window was in nobody's hidden list and joined
    /// a workflow session the workflow had never mentioned.
    ///
    /// The race is staged deterministically: the creator takes the root lock
    /// first (`create_base` holds it for the whole git init), and the apply
    /// starts a few ms later. `list_bases` takes no lock, so the old code read
    /// its inventory straight through the creator's lock and missed `gamma`;
    /// the locked write blocks until the base is fully installed and then hides
    /// it like any other undeclared base.
    #[test]
    fn applying_a_workflow_hides_a_base_that_lands_mid_call() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(KnowledgeService::new(dir.path().to_path_buf()));
        for id in ["alpha", "beta"] {
            svc.create_base(id, id, None).unwrap();
        }

        let mut workflow = Workflow::builder()
            .title("t")
            .description("d")
            .instructions("i")
            .build()
            .unwrap();
        workflow.knowledge_bases = Some(WorkflowKnowledgeBases {
            default: Some("alpha".to_string()),
            visible: ids(&["alpha"]),
        });

        let creator = {
            let svc = Arc::clone(&svc);
            std::thread::spawn(move || svc.create_base("gamma", "gamma", None).unwrap())
        };
        // Wait for an observable write that happens only after `create_base`
        // has taken the root lock. A fixed sleep lost this race on slower
        // Windows runners: the apply could finish before the creator thread was
        // scheduled, making `gamma` legitimately visible when it landed later.
        let gamma_root = biorouter_mcp::knowledge::paths::kb_root(svc.root(), "gamma");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !gamma_root.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the creator must begin writing gamma while the test is waiting"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        apply_workflow_knowledge_selection(&svc, "s1", &workflow).unwrap();
        creator.join().unwrap();

        let selection = svc.selection(Some("s1")).unwrap();
        assert_eq!(
            selection.kb_ids,
            ids(&["alpha"]),
            "only the declared base belongs to a workflow session"
        );
        assert_eq!(selection.primary_kb.as_deref(), Some("alpha"));
    }

    /// Every step that can fail while the new chat already exists, but before it
    /// is returned, discards it. An orphan chat is a row the user never asked
    /// for and cannot explain, and the knowledge apply was the one step that
    /// left one: it used a bare `?` where its two siblings take the error, call
    /// `discard_failed_new_session`, and only then return.
    ///
    /// ⚠ It was not a rare race. A workflow whose `default` names a base that
    /// has since been deleted fails here on EVERY start, so a stale workflow
    /// minted one orphan per press.
    ///
    /// **A source read, deliberately.** Reaching the real handler needs an
    /// `AppState`, and `AppState::new` calls `AgentManager::instance()` and
    /// `KnowledgeService::new_default()` — both of which resolve the developer's
    /// own `~/.config/biorouter`. A test that creates and deletes chats there is
    /// worse than no test. The shape is what the defect was, so the shape is
    /// what is asserted.
    ///
    /// ⚠ **Scope.** The `?` sites *after* this window — the two
    /// `manager.update(...)` calls and the refetch — orphan a chat too and are
    /// deliberately not covered: each of those is the session store itself
    /// failing, where the discard's own `delete_session` would be failing for
    /// the same reason, and deciding what to do there is a separate question.
    /// Named here so the next reader knows they were seen, not missed.
    #[test]
    fn every_failure_before_a_new_chat_is_returned_discards_it() {
        let source = include_str!("agent.rs");
        // ⚠ The leading newline is load-bearing: `include_str!` reads this file
        // including this test, so an anchor without it matches the copy inside
        // this very string literal and slices the test instead of the handler.
        let body = source
            .split("\nasync fn start_agent(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("start_agent production body");

        // In source order. Each step's window runs to the next one, so the
        // assertion is "between one fallible step and the next, the error path
        // discards the chat" rather than a count that a fourth step could pass
        // without being looked at.
        const STEPS: [&str; 3] = [
            "bind_new_session_provider(",
            "runtime::prepare_prompt(",
            "apply_workflow_knowledge_selection(",
        ];

        let at = |needle: &str| {
            body.find(needle)
                .unwrap_or_else(|| panic!("`{needle}` is a step of start_agent"))
        };
        for (index, step) in STEPS.iter().enumerate() {
            let start = at(step);
            let end = STEPS.get(index + 1).map_or(body.len(), |next| at(next));
            // `get`, not `&body[start..end]`: the workspace denies
            // `clippy::string_slice`, and it is right to — a slice that is not on
            // a char boundary panics. Both bounds come from `find`, so they are
            // boundaries and this never fires.
            let window = body
                .get(start..end)
                .expect("both bounds come from `find`, so both are char boundaries");
            let discarded = window
                .find("discard_failed_new_session")
                .unwrap_or_else(|| {
                    panic!(
                        "`{step}` can return an error without discarding the chat it leaves behind"
                    )
                });
            if let Some(returned) = window.find("return Err") {
                assert!(
                    discarded < returned,
                    "`{step}` returns its error before discarding the chat"
                );
            }
        }
    }
}
