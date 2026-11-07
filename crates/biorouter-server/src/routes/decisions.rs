//! Decision models over HTTP: the catalog, installing a local decision model,
//! an ad hoc decision, one chat's decision log and decision settings, and the
//! person's controls over a chat's phase gates.
//!
//! | Route | Gate |
//! |---|---|
//! | `GET /decisions/models` | the daemon secret; with `sessionId`, [`session_reach`] first |
//! | `GET /decisions/models/install` | the daemon secret: it names no chat |
//! | `POST /decisions/models/install` | the daemon secret: it names no chat, and the installer itself refuses an Ollama that is not on this computer |
//! | `POST /decisions/decide` | with `sessionId`: [`session_reach`] first, then Gate H in the service |
//! | `GET /sessions/{id}/decisions` | [`session_reach`] |
//! | `GET /sessions/{id}/decisions/summary` | [`session_reach`] |
//! | `GET /sessions/{id}/decisions/{decision_id}` | [`session_reach`] |
//! | `GET /sessions/{id}/decision-settings` | [`session_reach`] |
//! | `PUT /sessions/{id}/decision-settings` | [`session_reach`], then the user-action proof on a keyed daemon, then the subagent rule on a keyless one, then the save check (422) |
//! | `GET /sessions/{id}/decision-chat` | [`session_reach`] |
//! | `PUT /sessions/{id}/decision-chat` | [`session_reach`], then the user-action proof on a keyed daemon, then the subagent rule on a keyless one, then the save check (422) |
//! | `PUT /sessions/{id}/phase-gates/{phase_id}` | [`session_reach`], then the user-action proof on a keyed daemon, then the subagent rule on a keyless one, then the gate's model check (422) |
//! | `DELETE /sessions/{id}/phase-gates/{phase_id}` | [`session_reach`], then the user-action proof on a keyed daemon, then the subagent rule on a keyless one |
//! | `POST /sessions/{id}/phase-gates/{phase_id}/decision` | [`session_reach`], then the user-action proof on a keyed daemon, then the subagent rule on a keyless one |
//!
//! Every handler that names a chat calls [`reach_chat`] before it reads or
//! parses anything about that chat, and a refusal is the reach gate's exact
//! text, so an unproven caller cannot tell a private chat from one that does
//! not exist by asking for its decisions. The subagent rule reads the row
//! `reach_chat` returns, so it too is asked only of an admitted caller.
//!
//! # Status codes of the writes
//!
//! 400 is a request that cannot be read or is out of bounds; 403 is the reach
//! gate, the proof or the subagent rule, and nothing else, because `biorouter
//! session monitors` reads any 403 as a missing user-action key; 404 is no
//! such chat, plan or phase; 409 is a conflict with what is stored (a closed
//! phase, no gate, an unreadable plan, another install running, an Ollama
//! elsewhere); 422 is a valid request whose model cannot run for this chat now
//! (not there, cannot take the question, will not load, or refused by the
//! chat's privacy tier, an institution or a Crew grant); 503 is an Ollama that
//! is not running.
//!
//! # The save check
//!
//! A settings write stores only what can run: after every refusal above it
//! asks the decision service whether each monitor the save turns on (and a
//! changed default model, or a decision chat's new question) could run for
//! this chat now ([`biorouter::decision::save_checked`]), sending nothing of
//! the chat, and answers 422 with the reason when one could not. It runs only
//! after the proof, so an unproven caller cannot use a write to probe which
//! models exist. A monitor that is switched off or left as it was is never
//! asked, so a person can always switch off or remove a monitor whose model
//! went away. A phase gate the person sets is checked the same way
//! ([`DecisionService::check_model_use`]). After a settings save, the chat's
//! monitors that also watch its subagents are copied onto the subagents it is
//! running now ([`decision_settings::sync_inherited`]).
//!
//! # Why these routes never draw one row per call
//!
//! A chat can ask a decision model thousands of times (a monitor every
//! iteration, a `decide` batch over hundreds of chunks). The decision log is the
//! only place individual calls are listed, and it is read here a page at a time
//! (`GET /sessions/{id}/decisions`, newest first, at most 200 rows). Everything
//! else a surface draws comes from `GET /sessions/{id}/decisions/summary`, which
//! aggregates per source and per turn.
//!
//! # Privacy
//!
//! The decision log holds what was sent, kept at the chat's own tier, so
//! reading it is reading the chat and takes the same gate. For a monitor, a
//! gate or a decision chat that text is the chat's own content. For an ad hoc
//! decision it is whatever the caller sent, so when the named chat is public
//! (and privacy tiers are on) the answer says the text is now kept where public
//! chats can read it. An ad hoc decision that names a chat is checked against
//! that chat's classification by the decision service itself (Gate H, downward
//! rule: `decision::service` and
//! `privacy::alt_provider::assert_alt_tier_allowed`), with the master switch
//! sampled once for the request. Without a chat there is no classification to
//! check, and the answer says so.
//!
//! Writing a chat's decision settings chooses which model reads the chat's
//! content and what a monitor may do to a running turn, so on a daemon that
//! holds a user-action key the write needs that proof, as the other
//! capability-moving writes do. On a daemon that holds none (`biorouter serve`,
//! or a hand-run `biorouterd`) nothing can produce the proof, so the reach gate
//! is the whole of the gate there, as it is for Stop (SD-11), steer monitors
//! included (owner ruling). One exception: a subagent's chat is refused there
//! with `SUBAGENT_CONTROL_NO_KEY`, as `/reply` and `/agent/stop` refuse it,
//! because a subagent's turns load that chat's own monitors. A desktop daemon
//! that was meant to receive a key and did not keeps the refusal, because that
//! is a fault a restart repairs. These routes are not the only door: `biorouter
//! session monitors` with no reachable daemon writes the same store directly,
//! under the tier rule alone, on any daemon, after the same save check. A
//! settings write applies at the chat's next monitor check, in a turn already
//! running too. Gate H is asked again before every call whatever the settings
//! say, so no setting can carry a private chat to a public model: the save
//! check is a usability check, not the privacy boundary.
//!
//! A phase gate decides when delegated work may go on, so setting, removing,
//! passing, skipping or retrying one takes the same proof
//! ([`PHASE_GATES_NEED_USER`] when it is missing), and the same subagent rule.
//! The agent's own tool has no such action.
//!
//! [`session_reach`]: crate::routes::session_reach::session_reach

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use biorouter::agents::orchestration_plan::{
    self as orchestration_plan, GateWriteError, OrchestrationSnapshot, PersonGateChoice,
    PersonGateOutcome, PhaseGateSpec, MAX_PHASE_ID_CHARS,
};
use biorouter::agents::subagent_roster::SessionSubagents;
use biorouter::decision::log::{
    self as decision_log, DecisionDetail, DecisionPage, DecisionQuery, InvalidDecisionQuery,
    SessionDecisionSummary, MAX_PAGE,
};
use biorouter::decision::settings::{
    self as decision_settings, DecisionChatSettings, DecisionSettings, InvalidDecisionSettings,
};
use biorouter::decision::types::{validate_questions, DecisionKindTag};
use biorouter::decision::{
    ChunkOutcome, Decision, DecisionAdapter, DecisionChunk, DecisionContext, DecisionError,
    DecisionModelRef, DecisionOrigin, DecisionRequest, DecisionService, DecisionSession,
    LiveClassification, ModelSource, UnusableDecisionSettings,
};
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::decision::install::{
    DecisionInstallSnapshot, DecisionInstaller, InstallState, OllamaPlace, DEFAULT_INSTALL_MODEL,
};
use biorouter::session::session_manager::SessionType;
use biorouter::session::Session;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::state::AppState;

/// The label an ad hoc decision is logged under.
const ADHOC_LABEL: &str = "Ad hoc";

/// What an ad hoc decision with no chat says about itself.
const NO_CHAT_CONTEXT: &str = "No chat was named, so this decision ran without a chat's \
     context: no chat's privacy tier was checked, and it is not in any chat's decision history.";

/// What an ad hoc decision that names a public chat says, with privacy tiers
/// on: the caller's text is now in that chat's log, at that chat's reach.
pub const PUBLIC_CHAT_LOG_NOTE: &str = "This chat is public, so the text sent is kept in its \
     decision history, where public chats can read it. For private text, name a private chat \
     or no chat.";

/// The refusal of a decision-settings write aimed at a subagent's chat on a
/// daemon that holds no user-action key: the sentence `/reply` and
/// `/agent/stop` answer there, so a subagent's tab reads one refusal.
pub const DECISION_SETTINGS_SUBAGENT_NO_KEY: &str = crate::routes::agent::SUBAGENT_CONTROL_NO_KEY;

/// The refusal of a decision-settings write that carried no proof of a person,
/// on a daemon that holds a user-action key.
pub const DECISION_SETTINGS_NEED_USER: &str = "Changing a chat's decision settings (its decision \
     model, its monitors, or the question a decision chat asks) is the user's decision: these \
     settings choose which model reads the chat's content and what a monitor may do to a running \
     turn, and this request carried no proof that it came from the person at the keyboard. Nothing was changed. Do not retry as you are; the same \
     call will be refused again. If this task needs a monitor or a different decision model, \
     stop and ask the user to set it in the app.";

/// …and on a desktop daemon that was meant to receive a key and did not.
pub const DECISION_SETTINGS_KEY_MISSING: &str = "Biorouter cannot confirm that this change \
     came from you: the app that started it was meant to hand it a key for that, and none \
     arrived. Nothing was changed. Restarting Biorouter repairs this.";

/// The refusal of a phase-gate write that carried no proof of a person, on a
/// daemon that holds a user-action key.
pub const PHASE_GATES_NEED_USER: &str = "Changing a phase's gate, or passing, skipping or \
     retrying it, is the user's decision: a gate decides when delegated work may go on, and \
     this request carried no proof that it came from the person at the keyboard. Nothing was \
     changed. Do not retry as you are; the same call will be refused again. If this task needs \
     a gate changed, stop and ask the user to do it in the app.";

/// The refusal of a workflow start that declares monitors or phase gates and
/// carried no proof of a person, on a daemon that holds a user-action key
/// (`POST /agent/start`, through [`decision_write_refusal`]).
pub(crate) const WORKFLOW_SUPERVISION_NEED_USER: &str = "Starting a workflow that declares \
     monitors or phase gates is the user's decision: these choose which decision model reads \
     the chat and what may stop a running turn, and this request carried no proof that it came \
     from the person at the keyboard. No chat was created. Ask the user to start the workflow \
     in the app.";

/// The refusal of a phase id that is not one: what `PUT`, `DELETE` and
/// `POST …/decision` under `/phase-gates/` answer before reading anything else.
const NOT_A_PHASE_ID: &str =
    "That is not a phase id: it must be 1 to 40 lowercase letters, digits or hyphens.";

/// The routes' state: the app, the decision service they ask, and the
/// installer of local decision models. Production uses the process's one
/// service and installer; tests hand in their own (a service over a fake
/// source, an installer over a mock Ollama), so no route test reaches a model.
#[derive(Clone)]
pub struct DecisionRoutes {
    state: Arc<AppState>,
    service: &'static DecisionService,
    installer: &'static DecisionInstaller,
}

// ─── Wire types ───
//
// ⚠ Every `body =`, `params(...)` and `value_type =` below names its type BARE
// (`DecisionPage`, `DecisionOutcome`, `Question`). utoipa 4 names a `$ref`
// after the path as written, so `biorouter::decision::log::DecisionPage` there
// becomes a `biorouter.decision.log.DecisionPage` schema that nothing defines,
// and `npm run generate-api` fails on the dangling reference. The schemas
// themselves are registered in `openapi.rs`.

/// One decision model in the catalog, with whether the named chat may use it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionModelInfo {
    /// `baseten_decision` or `ollama_decision`.
    pub provider: String,
    pub model: String,
    pub display_name: String,
    pub context_tokens: usize,
    pub kinds: Vec<DecisionKindTag>,
    pub tier: ProviderTier,
    pub adapter: DecisionAdapter,
    /// Runs on this computer.
    pub local: bool,
    /// Why the chat named in the request may not use this model (its privacy
    /// tier, or an institution the model does not cover), or `null` when it
    /// may. Always `null` when no chat was named.
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionModelsResponse {
    /// Local models first.
    pub models: Vec<DecisionModelInfo>,
    /// The chat `unavailableReason` was worked out for, when one was named.
    pub session_id: Option<String>,
    /// Why the named chat may use no decision model at all, whichever is
    /// named or chosen for it (a Crew grant on it), or `null` when nothing
    /// rules out every model, and always when no chat was named. Every
    /// model's `unavailableReason` says the same; this one also covers a
    /// choice that names no model, such as a monitor left on the chat's
    /// decision model.
    pub session_unavailable_reason: Option<String>,
}

/// `GET /decisions/models`'s query.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionModelsQuery {
    pub session_id: Option<String>,
}

/// An ad hoc decision: questions over one state (`context`) or over a batch of
/// states (`chunks`), exactly one of the two.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecideRequest {
    /// The chat to check the decision against and log it in. When given, the
    /// chat is gated for this caller first, the decision is checked against
    /// the chat's privacy tier before anything is sent, and the call, with the
    /// text sent, is logged in the chat's decision history as an ad hoc
    /// decision, at the chat's tier.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The decision model to ask. Without one the service chooses the first
    /// model the chat may use, local first.
    #[serde(default)]
    pub model: Option<DecisionModelRef>,
    /// Question name -> question, in the order they are answered (1 to 64).
    #[schema(value_type = HashMap<String, Question>)]
    pub questions: serde_json::Map<String, Value>,
    /// One state to ask the questions about.
    #[serde(default)]
    pub context: Option<DecisionContext>,
    /// Many states, one request each, answered in order (1 to 1,000).
    #[serde(default)]
    pub chunks: Option<Vec<DecisionChunk>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecideResponse {
    /// The chat the decision was checked against and logged in, or `null`.
    pub session_id: Option<String>,
    /// The answers for `context`, in question order.
    pub decisions: Option<Vec<Decision>>,
    /// One outcome per chunk, in chunk order, for `chunks`. A chunk that
    /// failed carries its error and never fails the batch.
    pub chunks: Option<Vec<ChunkOutcome>>,
    /// Said when no chat was named, when the chat named is public (with
    /// privacy tiers on), so the text sent is kept where public chats can read
    /// it, and when the model that answered is public, so the text sent left
    /// this computer.
    pub note: Option<String>,
}

/// A chat's decision-chat settings, or `null` when it has none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionChatResponse {
    pub settings: Option<DecisionChatSettings>,
}

/// What a person's phase-gate write did, and the chat's plan after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhaseGateResponse {
    /// The sentence the person reads: what changed and what happens next.
    pub message: String,
    /// `false` when a pass or skip was recorded but cannot close the phase
    /// yet: it stands until the phase's working subagents finish and the
    /// agent completes the phase. Always `true` for a set, a removal or a
    /// retry.
    pub applied: bool,
    /// The chat's orchestration plan after the write, exactly as `GET
    /// /sessions/{session_id}/subagents` shows it to this caller: a subagent
    /// the caller could not open is neither listed nor counted. `null` when
    /// the plan could not be read back.
    pub plan: Option<OrchestrationSnapshot>,
}

/// A person's decision about a phase's gate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseGateDecisionRequest {
    /// `pass` or `skip` closes the phase without asking its gate, at once
    /// when none of the phase's subagents is still working, else once they
    /// finish and the agent completes it. `retry` gives the gate its attempts
    /// again.
    pub decision: PersonGateChoice,
}

/// One decision model Biorouter installs into the Ollama on this computer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstallableDecisionModelInfo {
    /// The name Ollama and the model list know it by (`tev1:0.8b`).
    pub model: String,
    pub display_name: String,
    /// The download, roughly (`about 1.5 GB`).
    pub approx_size: String,
    /// One phrase on what it is for.
    pub note: String,
    /// The one installed when a request names none.
    pub is_default: bool,
    /// Listed already by the configured Ollama on this computer as a
    /// decision model it loads. Never asked of an Ollama elsewhere.
    pub installed: bool,
}

/// Where the configured Ollama (`OLLAMA_HOST`) is, as far as installing into
/// it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OllamaPlaceState {
    /// On this computer, and answering.
    Running,
    /// On this computer, and not answering.
    NotRunning,
    /// On another computer. Nothing was sent to it.
    NotThisComputer,
    /// `OLLAMA_HOST` does not resolve to a server address.
    InvalidHost,
}

impl From<&OllamaPlace> for OllamaPlaceState {
    fn from(place: &OllamaPlace) -> Self {
        match place {
            OllamaPlace::Running { .. } => Self::Running,
            OllamaPlace::NotRunning => Self::NotRunning,
            OllamaPlace::NotThisComputer => Self::NotThisComputer,
            OllamaPlace::InvalidHost(_) => Self::InvalidHost,
        }
    }
}

/// The configured Ollama, as the install status reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OllamaPlaceInfo {
    /// `OLLAMA_HOST` resolved, or `null` when it does not resolve.
    pub base_url: Option<String>,
    pub state: OllamaPlaceState,
    /// Ollama's version while it runs here.
    pub version: Option<String>,
    /// Why nothing can be installed there now, as a sentence to show, or
    /// `null` while Ollama runs on this computer.
    pub message: Option<String>,
}

/// Where an install stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionInstallState {
    /// Ollama is downloading the model.
    Downloading,
    /// The download finished; Biorouter is checking that the model answers
    /// decisions there.
    Verifying,
    /// The model is listed and answers decisions.
    Installed,
    /// The install stopped; `error` says why.
    Failed,
}

impl From<InstallState> for DecisionInstallState {
    fn from(state: InstallState) -> Self {
        match state {
            InstallState::Downloading => Self::Downloading,
            InstallState::Verifying => Self::Verifying,
            InstallState::Installed => Self::Installed,
            InstallState::Failed => Self::Failed,
        }
    }
}

/// The install that is running, or the last one, which is kept until the
/// next one starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInstallJob {
    pub model: String,
    pub state: DecisionInstallState,
    /// Bytes downloaded so far, over every layer.
    pub completed_bytes: u64,
    /// Bytes to download, over every layer named so far. It grows as Ollama
    /// names more layers.
    pub total_bytes: u64,
    /// Ollama's last step, in its words.
    pub status: Option<String>,
    /// Why the install failed, once it has.
    pub error: Option<String>,
}

/// `GET /decisions/models/install`: what can be installed, where, and how the
/// current or last install is going.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInstallStatus {
    /// The models Biorouter installs, the default first.
    pub offers: Vec<InstallableDecisionModelInfo>,
    pub ollama: OllamaPlaceInfo,
    /// The current or last install, or `null` when none ran since the daemon
    /// started.
    pub job: Option<DecisionInstallJob>,
}

impl From<DecisionInstallSnapshot> for DecisionInstallStatus {
    fn from(snapshot: DecisionInstallSnapshot) -> Self {
        let DecisionInstallSnapshot {
            offers,
            ollama,
            job,
        } = snapshot;
        Self {
            offers: offers
                .into_iter()
                .map(|offer| InstallableDecisionModelInfo {
                    model: offer.model.name.to_string(),
                    display_name: offer.model.display_name.to_string(),
                    approx_size: offer.model.approx_size.to_string(),
                    note: offer.model.note.to_string(),
                    is_default: offer.model.is_default(),
                    installed: offer.installed,
                })
                .collect(),
            ollama: OllamaPlaceInfo {
                base_url: ollama.base_url.as_ref().map(ToString::to_string),
                state: OllamaPlaceState::from(&ollama.place),
                version: ollama.place.version().map(str::to_string),
                message: ollama.message,
            },
            job: job.map(|job| DecisionInstallJob {
                model: job.model,
                state: job.state.into(),
                completed_bytes: job.completed_bytes,
                total_bytes: job.total_bytes,
                status: job.status,
                error: job.error,
            }),
        }
    }
}

/// `POST /decisions/models/install`'s body. An empty body installs the
/// default model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionInstallRequest {
    /// One of the offered models (`tev1:0.8b`, `clef-flash:latest`); left out,
    /// `tev1:0.8b`.
    #[serde(default)]
    pub model: Option<String>,
}

// ─── Shared steps ───

fn plain(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

/// The status a failed decision is answered with. The body is the error's own
/// sentence, which never carries a backend's error body (`decision::error`).
fn decision_error_status(error: &DecisionError) -> StatusCode {
    match error {
        DecisionError::Invalid(_) => StatusCode::BAD_REQUEST,
        DecisionError::Refused(_) => StatusCode::FORBIDDEN,
        DecisionError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        DecisionError::NoModel(_) => StatusCode::NOT_FOUND,
        DecisionError::Unsupported(_) => StatusCode::UNPROCESSABLE_ENTITY,
        DecisionError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
        DecisionError::Backend { .. } | DecisionError::Malformed { .. } => StatusCode::BAD_GATEWAY,
    }
}

fn decision_error_response(error: DecisionError) -> Response {
    plain(decision_error_status(&error), error.to_string())
}

/// The reach gate for every route that names a chat, and the one place this
/// module calls it: the id's shape (which says nothing about any chat), then
/// [`session_reach`](crate::routes::session_reach::session_reach), whose
/// refusal is returned word for word, then whether the chat exists. Only an
/// admitted caller is told a chat is absent, and only an admitted caller gets
/// the row back (the writes read its `session_type`).
async fn reach_chat(
    state: &AppState,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<Session, Response> {
    if !crate::routes::session::is_valid_session_id(session_id) {
        return Err(plain(
            StatusCode::BAD_REQUEST,
            "That is not a chat id: it must be 1 to 128 letters, digits, `-` or `_`.",
        ));
    }
    let manager = state.session_manager();
    // FIRST, before the row is read: a handler that looked the chat up before
    // this would answer an unproven caller 404 for an absent id and something
    // else for a private one.
    crate::routes::session_reach::session_reach(manager, session_id, headers)
        .await
        .map_err(IntoResponse::into_response)?;
    match manager.get_session(session_id, false).await {
        Ok(session) => Ok(session),
        Err(error) if error.to_string().contains("not found") => Err(plain(
            StatusCode::NOT_FOUND,
            "There is no chat with that id.",
        )),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

/// The proof rule every write here shares, as the sentence it refuses with:
/// `need_user` when a daemon that holds a user-action key was sent no proof,
/// [`DECISION_SETTINGS_KEY_MISSING`] on a desktop daemon that was meant to get
/// a key and did not, and nothing on a daemon that can never check a proof
/// (`biorouter serve`), where the reach gate is the whole gate. Pure, so every
/// corner is tested; `expected_key` is whether the launcher declared it would
/// hand this daemon a user-action key.
fn proof_refusal(
    proof: UserActionProof,
    expected_key: bool,
    need_user: &'static str,
) -> Option<&'static str> {
    match proof {
        UserActionProof::Proven => None,
        UserActionProof::Unproven => Some(need_user),
        UserActionProof::NoKeyInstalled if expected_key => Some(DECISION_SETTINGS_KEY_MISSING),
        UserActionProof::NoKeyInstalled => None,
    }
}

/// The proof rule of a decision-settings write without its subagent arm, for
/// a door that writes supervision into a chat it has not created yet:
/// `POST /agent/start` with a workflow that declares monitors or phase gates.
/// Refused with [`WORKFLOW_SUPERVISION_NEED_USER`] when a daemon that holds a
/// user-action key was sent no proof, and with
/// [`DECISION_SETTINGS_KEY_MISSING`] on a desktop daemon whose key never
/// arrived; admitted on a daemon that holds none.
// `/agent/start` is the caller; until it is wired the binary sees no use.
#[allow(dead_code)]
pub(crate) fn decision_write_refusal(headers: &HeaderMap) -> Option<Response> {
    decision_write_refusal_sentence(headers).map(|sentence| plain(StatusCode::FORBIDDEN, sentence))
}

/// [`decision_write_refusal`]'s sentence, for a door that answers in its own
/// shape (`/agent/start` answers JSON).
#[allow(dead_code)]
pub(crate) fn decision_write_refusal_sentence(headers: &HeaderMap) -> Option<&'static str> {
    proof_refusal(
        user_action_proof(headers),
        biorouter_server::launch::expected_a_user_action_key(),
        WORKFLOW_SUPERVISION_NEED_USER,
    )
}

/// The subagent rule SD-11 keeps for Stop and steering, asked of a write the
/// proof check admitted. A subagent's turns load its own chat's monitors, so a
/// monitor written there stops or steers the subagent. On a daemon that holds
/// no user-action key nothing shows that a person asked for that, so the write
/// is refused; on a keyed daemon only a proven write gets this far, and the
/// proof is what admits it. Pure, so every corner is tested.
fn subagent_write_refusal(proof: UserActionProof, session_type: SessionType) -> Option<Response> {
    (session_type == SessionType::SubAgent && proof == UserActionProof::NoKeyInstalled)
        .then(|| plain(StatusCode::FORBIDDEN, DECISION_SETTINGS_SUBAGENT_NO_KEY))
}

/// Every refusal a decision-settings write meets after the reach gate, in
/// order: the proof, then the subagent rule.
fn write_refusal(session: &Session, headers: &HeaderMap) -> Option<Response> {
    write_refusal_with(session, headers, DECISION_SETTINGS_NEED_USER)
}

/// [`write_refusal`] with the sentence an unproven caller reads (a phase-gate
/// write answers [`PHASE_GATES_NEED_USER`]).
fn write_refusal_with(
    session: &Session,
    headers: &HeaderMap,
    need_user: &'static str,
) -> Option<Response> {
    let proof = user_action_proof(headers);
    proof_refusal(
        proof,
        biorouter_server::launch::expected_a_user_action_key(),
        need_user,
    )
    .map(|sentence| plain(StatusCode::FORBIDDEN, sentence))
    .or_else(|| subagent_write_refusal(proof, session.session_type))
}

/// Read a settings body as `T`, the `what` it is ("decision settings") naming
/// it in the refusal. The settings records refuse a key they do not have
/// (`decision::settings`), and that refusal is answered with
/// [`unknown_key_sentence`]; any other problem keeps serde's own text, as
/// before.
fn read_settings_body<T: serde::de::DeserializeOwned>(
    body: &[u8],
    what: &str,
) -> Result<T, String> {
    let could_not_read =
        |error: &dyn std::fmt::Display| format!("The {what} could not be read: {error}");
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        unknown_key_sentence(&error, body, what).unwrap_or_else(|| could_not_read(error.inner()))
    })?;
    deserializer.end().map_err(|error| could_not_read(&error))?;
    Ok(value)
}

/// The sentence for a key a settings record does not have: the key, where it
/// is (a monitor by its id, else by its place in the list), and the spelling
/// that is accepted when the key is another spelling of a real field
/// (`on_error` or `ON_ERROR` for `onError`); otherwise the fields that are
/// accepted there. `None` for any other error.
fn unknown_key_sentence(
    error: &serde_path_to_error::Error<serde_json::Error>,
    body: &[u8],
    what: &str,
) -> Option<String> {
    use serde_path_to_error::Segment;

    let segments: Vec<&Segment> = error.path().iter().collect();
    let (last, parent) = segments.split_last()?;
    let Segment::Map { key } = last else {
        return None;
    };
    // serde's own wording for this error (`de::Error::unknown_field`), which
    // lists the fields the record has as `a`, `b` or `c`.
    let message = error.inner().to_string();
    let rest = message.strip_prefix(&format!("unknown field `{key}`"))?;
    let fields: Vec<&str> = rest
        .split_once("expected ")
        .map(|(_, list)| list.split('`').skip(1).step_by(2).collect())
        .unwrap_or_default();

    // A monitor is named by its id, which is read from the body when the body
    // is whole JSON, else by its place in the list.
    let (monitor, inner) = match parent {
        [Segment::Map { key }, Segment::Seq { index }, inner @ ..] if key == "monitors" => {
            let id = serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|value| {
                    value["monitors"][*index]["id"]
                        .as_str()
                        .filter(|id| !id.trim().is_empty())
                        .map(str::to_string)
                });
            let monitor = match id {
                Some(id) => format!("monitor `{id}`"),
                None => format!("monitor {} in the list", index + 1),
            };
            (Some(monitor), inner)
        }
        _ => (None, parent),
    };
    let mut place = String::new();
    for segment in inner {
        if !place.is_empty() && !matches!(segment, Segment::Seq { .. }) {
            place.push('.');
        }
        place.push_str(&segment.to_string());
    }
    let owner = match (monitor, place.is_empty()) {
        (Some(monitor), true) => monitor,
        (Some(monitor), false) => format!("`{place}` in {monitor}"),
        (None, true) => format!("the {what}"),
        (None, false) => format!("`{place}`"),
    };

    let folded = |name: &str| -> String {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let spelled = fields
        .iter()
        .find(|field| folded(field) == folded(key))
        .map(|field| format!("; the field is spelled `{field}`."));
    let accepted = || match fields.as_slice() {
        [] => ".".to_string(),
        fields => format!(
            ". The fields accepted there are {}.",
            fields
                .iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    Some(format!(
        "The {what} could not be read: `{key}` is not a field of {owner}{}",
        spelled.unwrap_or_else(accepted)
    ))
}

/// A chat's classification as stored now, re-read before every request of a
/// batch, from the same store this daemon's routes read.
struct StoredClassification(Arc<AppState>);

#[async_trait]
impl LiveClassification for StoredClassification {
    async fn current(&self, session_id: &str) -> anyhow::Result<SessionClassification> {
        Ok(self
            .0
            .session_manager()
            .get_session(session_id, false)
            .await?
            .privacy_tier)
    }
}

/// The chat a decision is about, read now, with the master switch as this
/// request sampled it.
async fn chat_session(
    state: &Arc<AppState>,
    session_id: &str,
    enforced: bool,
) -> anyhow::Result<DecisionSession> {
    let manager = state.session_manager();
    let session = manager.get_session(session_id, false).await?;
    let affiliations = manager.session_affiliations(session_id).await?;
    // The chat's own default decision model, the rung after a named model.
    let chat_default_model =
        biorouter::decision::service::load_chat_default_model(manager, session_id).await;
    Ok(
        DecisionSession::new(session_id, session.privacy_tier, enforced)
            .with_affiliations(affiliations)
            .with_chat_default_model(chat_default_model)
            .with_live(Arc::new(StoredClassification(state.clone()))),
    )
}

/// The capability tier of the chat model `session` is bound to now, when this
/// daemon holds the chat's agent already, for the save-time checks only
/// ([`DecisionSession::bound_tier`]). It matters only for a chat still
/// classified Public with privacy tiers on (a chat that will become private
/// at its first turn), so nothing is read otherwise, and an agent is never
/// made here to ask: a chat this daemon has not opened is checked on its
/// classification alone.
async fn bound_tier(state: &AppState, session: &Session, enforced: bool) -> Option<ProviderTier> {
    if !enforced || session.privacy_tier.is_private() {
        return None;
    }
    let agent = state.peek_agent(&session.id).await?;
    agent.provider().await.ok().map(|provider| provider.tier())
}

/// The chat as the save-time checks see it: [`chat_session`] with the tier
/// of the chat model it is bound to.
async fn checked_session(
    state: &Arc<AppState>,
    session: &Session,
    enforced: bool,
) -> anyhow::Result<DecisionSession> {
    let bound = bound_tier(state, session, enforced).await;
    Ok(chat_session(state, &session.id, enforced)
        .await?
        .with_bound_tier(bound))
}

/// What a checked save answers when it stores nothing: 422 with the sentence
/// for settings that are valid but cannot run for this chat now, 400 with the
/// sentence for settings that are not valid, 500 for anything else.
fn save_error_response(error: anyhow::Error) -> Response {
    if let Some(unusable) = error.downcast_ref::<UnusableDecisionSettings>() {
        return plain(StatusCode::UNPROCESSABLE_ENTITY, unusable.0.clone());
    }
    if let Some(invalid) = error.downcast_ref::<InvalidDecisionSettings>() {
        return plain(StatusCode::BAD_REQUEST, invalid.0.clone());
    }
    tracing::warn!("a decision settings write failed: {error:#}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// The subagents `parent_id` has running or queued in this daemon now, each
/// once: the chats a save of its monitors copies the ones that also watch its
/// subagents onto.
fn live_children(parent_id: &str) -> Vec<String> {
    let mut children: Vec<String> = Vec::new();
    for handle in biorouter::agents::subagent_handle::list_for_session(parent_id) {
        if handle.is_running() && !children.contains(&handle.child_session_id) {
            children.push(handle.child_session_id.clone());
        }
    }
    children
}

/// A phase id is a plan's slug: 1 to [`MAX_PHASE_ID_CHARS`] lowercase ASCII
/// letters, digits or hyphens. Checked before anything is read, so a malformed
/// id never reaches the plan.
fn valid_phase_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_PHASE_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The status a person's gate write that changed nothing is answered with.
/// The body is the refusal's own sentence.
fn gate_error_status(error: &GateWriteError) -> StatusCode {
    match error {
        GateWriteError::NoPlan | GateWriteError::NoPhase(_) => StatusCode::NOT_FOUND,
        GateWriteError::Closed(_) | GateWriteError::NoGate(_) | GateWriteError::Unreadable(_) => {
            StatusCode::CONFLICT
        }
        GateWriteError::Invalid(_) => StatusCode::BAD_REQUEST,
    }
}

/// The chat's orchestration plan exactly as `GET
/// /sessions/{session_id}/subagents` answers this caller, read by asking that
/// route: the parent's reach gate, one resolved caller, every child this
/// caller could not open omitted (and the Crew exclusion), and the plan
/// recomputed over the rows that survive. Asking the route keeps one
/// implementation of that filter rather than a second one here. `None` when
/// the route does not answer 200, or its answer cannot be read.
async fn visible_plan(
    state: &Arc<AppState>,
    session_id: &str,
    headers: &HeaderMap,
) -> Option<OrchestrationSnapshot> {
    let response = crate::routes::session_subagents::get_session_subagents(
        State(state.clone()),
        Path(session_id.to_string()),
        headers.clone(),
    )
    .await;
    if response.status() != StatusCode::OK {
        tracing::warn!(
            session_id,
            status = %response.status(),
            "the plan after a phase-gate write could not be read back"
        );
        return None;
    }
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .ok()?;
    match serde_json::from_slice::<SessionSubagents>(&bytes) {
        Ok(roster) => roster.plan,
        Err(error) => {
            tracing::warn!(
                session_id,
                "the plan after a phase-gate write could not be read: {error}"
            );
            None
        }
    }
}

/// A person's gate write as the routes answer it: 200 with the outcome and
/// the plan as this caller may see it, or the refusal's status and sentence.
async fn gate_response(
    state: &Arc<AppState>,
    session_id: &str,
    headers: &HeaderMap,
    written: Result<PersonGateOutcome, GateWriteError>,
) -> Response {
    match written {
        Ok(outcome) => Json(PhaseGateResponse {
            message: outcome.message,
            applied: outcome.applied,
            plan: visible_plan(state, session_id, headers).await,
        })
        .into_response(),
        Err(error) => plain(gate_error_status(&error), error.message()),
    }
}

/// `text` without the full stop it ends with, so it can sit inside a
/// sentence.
fn without_full_stop(text: &str) -> &str {
    let text = text.trim_end();
    text.strip_suffix('.').unwrap_or(text)
}

/// What the log itself does not refuse: a `limit` outside 1 to [`MAX_PAGE`]
/// (the log clamps it, which would answer a page the caller did not ask for)
/// and a `minP` above `maxP` (an empty answer that looks like no decisions).
/// A probability outside 0 to 1 and a cursor this server did not issue are the
/// log's own [`InvalidDecisionQuery`], answered 400 below.
fn validate_query(query: &DecisionQuery) -> Result<(), String> {
    if let (Some(min), Some(max)) = (query.min_p, query.max_p) {
        if min > max {
            return Err("`minP` must not be above `maxP`.".to_string());
        }
    }
    if query
        .limit
        .is_some_and(|limit| limit == 0 || limit > MAX_PAGE)
    {
        return Err(format!("`limit` must be between 1 and {MAX_PAGE}."));
    }
    Ok(())
}

/// A decision id is a log row's own id: the same shape rule as a chat id.
fn valid_decision_id(id: &str) -> bool {
    crate::routes::session::is_valid_session_id(id)
}

/// The note an ad hoc answer carries: no chat, a public chat whose log now
/// holds the caller's text (only while privacy tiers are on, since with them
/// off every chat reaches every log), and a public model.
fn decide_note(session: Option<&DecisionSession>, decisions: &[&Decision]) -> Option<String> {
    let public_chat =
        session.is_some_and(|session| session.enforced && !session.classification.is_private());
    let public = decisions
        .iter()
        .find(|decision| !decision.tier.is_private())
        .map(|decision| {
            format!(
                "`{}` is a public model, so the text sent left this computer.",
                decision.model.label()
            )
        });
    let parts: Vec<String> = [
        session.is_none().then(|| NO_CHAT_CONTEXT.to_string()),
        public_chat.then(|| PUBLIC_CHAT_LOG_NOTE.to_string()),
        public,
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

// ─── Handlers ───

#[utoipa::path(
    get,
    path = "/decisions/models",
    tag = "decisions",
    params(
        ("sessionId" = Option<String>, Query, description = "A chat to work out \
          `unavailableReason` for. Gated like every route that names a chat")
    ),
    responses(
        (status = 200, description = "The decision models available now, local first, each \
                                      with the reason the named chat may not use it", body = DecisionModelsResponse),
        (status = 400, description = "Invalid session id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn list_decision_models(
    State(routes): State<DecisionRoutes>,
    Query(query): Query<DecisionModelsQuery>,
    headers: HeaderMap,
) -> Response {
    let session = match &query.session_id {
        Some(id) => {
            if let Err(refusal) = reach_chat(&routes.state, id, &headers).await {
                return refusal;
            }
            let enforced = biorouter::privacy::privacy_tiers_enabled();
            match chat_session(&routes.state, id, enforced).await {
                Ok(session) => Some(session),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        None => None,
    };
    let session_unavailable_reason = match &session {
        Some(session) => routes.service.session_refusal(session).await,
        None => None,
    };
    let models = routes
        .service
        .models_for(session.as_ref())
        .await
        .into_iter()
        .map(|(entry, unavailable_reason)| DecisionModelInfo {
            provider: entry.provider,
            model: entry.model,
            display_name: entry.display_name,
            context_tokens: entry.context_tokens,
            kinds: entry.kinds,
            tier: entry.tier,
            adapter: entry.adapter,
            local: entry.local,
            unavailable_reason,
        })
        .collect();
    Json(DecisionModelsResponse {
        models,
        session_id: query.session_id,
        session_unavailable_reason,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/decisions/decide",
    tag = "decisions",
    request_body = DecideRequest,
    responses(
        (status = 200, description = "The decisions, with probabilities", body = DecideResponse),
        (status = 400, description = "The request could not be read, or its questions or \
                                      states are invalid (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "The named chat is out of this caller's reach (the same \
                                      refusal, word for word, that `GET /sessions/{session_id}` \
                                      gives), or its privacy tier or a Crew grant refuses the \
                                      decision model; nothing was sent (body = plain text)"),
        (status = 404, description = "The named chat, or a decision model, was not found \
                                      (body = plain text)"),
        (status = 413, description = "The state does not fit the model's window; split it \
                                      (body = plain text)"),
        (status = 422, description = "The model cannot answer this kind of question (body = \
                                      plain text)"),
        (status = 502, description = "The decision model failed or answered in a shape that \
                                      cannot be trusted (body = plain text)"),
        (status = 504, description = "The decision model did not answer in time (body = plain \
                                      text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn decide(
    State(routes): State<DecisionRoutes>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // The body names the chat, so it is read first; reading it says nothing
    // about any chat. Everything after this is behind the reach gate.
    let body: DecideRequest = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(error) => {
            return plain(
                StatusCode::BAD_REQUEST,
                format!("The decision request could not be read: {error}"),
            )
        }
    };
    let session = match &body.session_id {
        Some(id) => {
            if let Err(refusal) = reach_chat(&routes.state, id, &headers).await {
                return refusal;
            }
            // ONE sample of the master switch for this request, threaded into
            // the service rather than re-read there.
            let enforced = biorouter::privacy::privacy_tiers_enabled();
            match chat_session(&routes.state, id, enforced).await {
                Ok(session) => Some(
                    session
                        .with_origin(DecisionOrigin::Adhoc, None)
                        .with_origin_label(Some(ADHOC_LABEL.to_string())),
                ),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        None => None,
    };

    let questions = match serde_json::from_value(Value::Object(body.questions)) {
        Ok(questions) => questions,
        Err(error) => {
            return plain(
                StatusCode::BAD_REQUEST,
                format!("The questions could not be read: {error}"),
            )
        }
    };
    let (context, chunks) = match (body.context, body.chunks) {
        (Some(context), None) => (context, None),
        (None, Some(chunks)) => (DecisionContext::Text(String::new()), Some(chunks)),
        _ => {
            return plain(
                StatusCode::BAD_REQUEST,
                "Send either `context` (one state) or `chunks` (a batch of states), not both \
                 and not neither.",
            )
        }
    };
    let request = DecisionRequest { questions, context };
    if let Err(error) = validate_questions(&request.questions) {
        return plain(StatusCode::BAD_REQUEST, error);
    }

    let model = body.model.as_ref();
    let response = match chunks {
        None => match routes
            .service
            .decide(session.as_ref(), model, &request)
            .await
        {
            Ok(decisions) => DecideResponse {
                session_id: body.session_id.clone(),
                note: decide_note(session.as_ref(), &decisions.iter().collect::<Vec<_>>()),
                decisions: Some(decisions),
                chunks: None,
            },
            Err(error) => return decision_error_response(error),
        },
        Some(chunks) => match routes
            .service
            .decide_many(session.as_ref(), model, &request.questions, chunks)
            .await
        {
            Ok(outcomes) => DecideResponse {
                session_id: body.session_id.clone(),
                note: decide_note(
                    session.as_ref(),
                    &outcomes
                        .iter()
                        .flat_map(|outcome| outcome.decisions.iter().flatten())
                        .collect::<Vec<_>>(),
                ),
                decisions: None,
                chunks: Some(outcomes),
            },
            Err(error) => return decision_error_response(error),
        },
    };
    Json(response).into_response()
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/decisions",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat whose decisions to list"),
        ("decisionId" = Option<String>, Query, description = "Only this decision (its row id, which is the request id its decisions carry): a page of at most one row"),
        ("origin" = Option<DecisionOrigin>, Query, description = "Only this source"),
        ("originId" = Option<String>, Query, description = "Only this monitor, tool call or phase"),
        ("turnId" = Option<String>, Query, description = "Only this agent turn"),
        ("batchId" = Option<String>, Query, description = "Only this `decide` batch"),
        ("outcome" = Option<DecisionOutcome>, Query, description = "Only this outcome"),
        ("minP" = Option<f64>, Query, description = "Lowest pass probability at least this (0 to 1)"),
        ("maxP" = Option<f64>, Query, description = "Lowest pass probability at most this (0 to 1)"),
        ("errorsOnly" = Option<bool>, Query, description = "Only failed and refused calls"),
        ("before" = Option<String>, Query, description = "The previous page's `nextCursor`"),
        ("limit" = Option<u32>, Query, description = "Rows per page, 1 to 200")
    ),
    responses(
        (status = 200, description = "One page of the chat's decision calls, newest first, \
                                      without their full context", body = DecisionPage),
        (status = 400, description = "Invalid session id or filter (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn list_session_decisions(
    State(routes): State<DecisionRoutes>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if let Err(refusal) = reach_chat(&routes.state, &session_id, &headers).await {
        return refusal;
    }
    // Parsed after the gate, so a malformed filter is answered only to a
    // caller that may read the chat.
    let query = match Query::<DecisionQuery>::try_from_uri(&uri) {
        Ok(Query(query)) => query,
        Err(rejection) => {
            return plain(
                StatusCode::BAD_REQUEST,
                format!("The filter could not be read: {}", rejection.body_text()),
            )
        }
    };
    if let Err(error) = validate_query(&query) {
        return plain(StatusCode::BAD_REQUEST, error);
    }
    match decision_log::list(routes.state.session_manager(), &session_id, &query).await {
        Ok(page) => Json::<DecisionPage>(page).into_response(),
        Err(error) => match error.downcast_ref::<InvalidDecisionQuery>() {
            Some(invalid) => plain(StatusCode::BAD_REQUEST, invalid.to_string()),
            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/decisions/summary",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat whose decisions to aggregate")
    ),
    responses(
        (status = 200, description = "The chat's decisions aggregated per source and criterion \
                                      and per recent turn, never one row per call", body = SessionDecisionSummary),
        (status = 400, description = "Invalid session id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn get_session_decision_summary(
    State(routes): State<DecisionRoutes>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(refusal) = reach_chat(&routes.state, &session_id, &headers).await {
        return refusal;
    }
    match decision_log::summary(routes.state.session_manager(), &session_id).await {
        Ok(summary) => Json::<SessionDecisionSummary>(summary).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/decisions/{decision_id}",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat the decision belongs to"),
        ("decision_id" = String, Path, description = "The decision's id")
    ),
    responses(
        (status = 200, description = "The decision call with the exact context it was sent \
                                      (capped at 64 KiB)", body = DecisionDetail),
        (status = 400, description = "Invalid session or decision id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "No such chat, or the chat has no decision with that id \
                                      (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn get_session_decision(
    State(routes): State<DecisionRoutes>,
    Path((session_id, decision_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(refusal) = reach_chat(&routes.state, &session_id, &headers).await {
        return refusal;
    }
    if !valid_decision_id(&decision_id) {
        return plain(StatusCode::BAD_REQUEST, "That is not a decision id.");
    }
    match decision_log::get(routes.state.session_manager(), &session_id, &decision_id).await {
        Ok(Some(detail)) => Json::<DecisionDetail>(detail).into_response(),
        Ok(None) => plain(
            StatusCode::NOT_FOUND,
            "This chat has no decision with that id.",
        ),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/decision-settings",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat whose decision settings to read")
    ),
    responses(
        (status = 200, description = "The chat's default decision model and its monitors", body = DecisionSettings),
        (status = 400, description = "Invalid session id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn get_decision_settings(
    State(routes): State<DecisionRoutes>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(refusal) = reach_chat(&routes.state, &session_id, &headers).await {
        return refusal;
    }
    match decision_settings::load(routes.state.session_manager(), &session_id).await {
        Ok(settings) => Json(settings).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/decision-settings",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat whose decision settings to replace")
    ),
    request_body = DecisionSettings,
    responses(
        (status = 200, description = "The settings as stored", body = DecisionSettings),
        (status = 400, description = "Invalid session id, or settings that could not be read or \
                                      are invalid; the body is the sentence that says why (plain \
                                      text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused: the named chat is out of this caller's reach (the \
                                      same refusal, word for word, that `GET \
                                      /sessions/{session_id}` gives), or, on a daemon that holds \
                                      a user-action key, the request carried no proof it came \
                                      from the person at the keyboard, or, on a daemon that \
                                      holds none, the chat is a subagent's (body = plain text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 422, description = "What this save turns on could not run for this chat now: \
                                      the model is not available, cannot take the question, or \
                                      the chat's privacy tier, an institution or a Crew grant \
                                      refuses it. Nothing was saved (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn put_decision_settings(
    State(routes): State<DecisionRoutes>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match reach_chat(&routes.state, &session_id, &headers).await {
        Ok(session) => session,
        Err(refusal) => return refusal,
    };
    if let Some(refusal) = write_refusal(&session, &headers) {
        return refusal;
    }
    let settings: DecisionSettings = match read_settings_body(&body, "decision settings") {
        Ok(settings) => settings,
        Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
    };
    if let Err(sentence) = decision_settings::validate(&settings) {
        return plain(StatusCode::BAD_REQUEST, sentence);
    }
    // ONE sample of the master switch for this request (not a tool path).
    // The check comes only now, after the proof: an unproven caller cannot
    // use this write to learn which models exist.
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let checked = match checked_session(&routes.state, &session, enforced).await {
        Ok(checked) => checked,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let manager = routes.state.session_manager();
    let stored = match biorouter::decision::save_checked(
        routes.service,
        manager,
        &checked,
        settings,
    )
    .await
    {
        Ok(stored) => stored,
        Err(error) => return save_error_response(error),
    };
    // A monitor that also watches the subagents this chat starts reaches the
    // ones running now as well as the next one. A child that cannot take its
    // copies is warned about by `sync_inherited` and does not fail the save.
    let live = live_children(&session_id);
    if !live.is_empty() {
        decision_settings::sync_inherited(manager, &session_id, &stored, &live).await;
    }
    Json(stored).into_response()
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/decision-chat",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat whose decision-chat question to read")
    ),
    responses(
        (status = 200, description = "The question a decision chat asks of each message, or \
                                      `null` settings when it has none", body = DecisionChatResponse),
        (status = 400, description = "Invalid session id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn get_decision_chat(
    State(routes): State<DecisionRoutes>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(refusal) = reach_chat(&routes.state, &session_id, &headers).await {
        return refusal;
    }
    match decision_settings::load_chat(routes.state.session_manager(), &session_id).await {
        Ok(settings) => Json(DecisionChatResponse { settings }).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/decision-chat",
    tag = "decisions",
    params(
        ("session_id" = String, Path, description = "The chat whose decision-chat question to set")
    ),
    request_body = DecisionChatSettings,
    responses(
        (status = 200, description = "The settings as stored", body = DecisionChatResponse),
        (status = 400, description = "Invalid session id, or a question that could not be read \
                                      or is invalid; the body is the sentence that says why \
                                      (plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused: the named chat is out of this caller's reach (the \
                                      same refusal, word for word, that `GET \
                                      /sessions/{session_id}` gives), or, on a daemon that holds \
                                      a user-action key, the request carried no proof it came \
                                      from the person at the keyboard, or, on a daemon that \
                                      holds none, the chat is a subagent's (body = plain text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 422, description = "The decision model this decision chat is bound to cannot \
                                      answer this question for this chat now: it is not \
                                      available, cannot take this kind of question, or the \
                                      chat's privacy tier, an institution or a Crew grant \
                                      refuses it. Nothing was saved (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn put_decision_chat(
    State(routes): State<DecisionRoutes>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match reach_chat(&routes.state, &session_id, &headers).await {
        Ok(session) => session,
        Err(refusal) => return refusal,
    };
    if let Some(refusal) = write_refusal(&session, &headers) {
        return refusal;
    }
    let settings: DecisionChatSettings = match read_settings_body(&body, "decision-chat settings") {
        Ok(settings) => settings,
        Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
    };
    if let Err(sentence) = decision_settings::validate_chat(&settings) {
        return plain(StatusCode::BAD_REQUEST, sentence);
    }
    // One sample of the master switch; the check only after the proof.
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let checked = match checked_session(&routes.state, &session, enforced).await {
        Ok(checked) => checked,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match biorouter::decision::save_chat_checked(
        routes.service,
        routes.state.session_manager(),
        &checked,
        settings,
    )
    .await
    {
        Ok(stored) => Json(DecisionChatResponse {
            settings: Some(stored),
        })
        .into_response(),
        Err(error) => save_error_response(error),
    }
}

// ─── The person's phase-gate controls ───

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/phase-gates/{phase_id}",
    // Workspace control: what may stop delegated work. The CLI parity gate
    // pairs it with `biorouter session gates set`.
    tag = "workspace",
    params(
        ("session_id" = String, Path, description = "The chat whose plan holds the phase"),
        ("phase_id" = String, Path, description = "The phase to gate")
    ),
    request_body = PhaseGateSpec,
    responses(
        (status = 200, description = "The gate is the person's now and its attempts start over; \
                                      the plan as this caller may see it", body = PhaseGateResponse),
        (status = 400, description = "Invalid session or phase id, or a gate that could not be \
                                      read or is invalid; the body is the sentence that says why \
                                      (plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused: the named chat is out of this caller's reach (the \
                                      same refusal, word for word, that `GET \
                                      /sessions/{session_id}` gives), or, on a daemon that holds \
                                      a user-action key, the request carried no proof it came \
                                      from the person at the keyboard, or, on a daemon that \
                                      holds none, the chat is a subagent's (body = plain text)"),
        (status = 404, description = "No such chat, the chat has no orchestration plan, or the \
                                      plan has no such phase (body = plain text)"),
        (status = 409, description = "The phase is completed or skipped, or the plan could not be \
                                      read or written. Nothing was changed (body = plain text)"),
        (status = 422, description = "The gate's decision model cannot answer it in this chat \
                                      now: not available, cannot take yes/no questions, will not \
                                      load, or refused by the chat's privacy tier, an \
                                      institution or a Crew grant. Nothing was changed (body = \
                                      plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn put_phase_gate(
    State(routes): State<DecisionRoutes>,
    Path((session_id, phase_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match reach_chat(&routes.state, &session_id, &headers).await {
        Ok(session) => session,
        Err(refusal) => return refusal,
    };
    if let Some(refusal) = write_refusal_with(&session, &headers, PHASE_GATES_NEED_USER) {
        return refusal;
    }
    if !valid_phase_id(&phase_id) {
        return plain(StatusCode::BAD_REQUEST, NOT_A_PHASE_ID);
    }
    let spec: PhaseGateSpec = match read_settings_body(&body, "phase gate") {
        Ok(spec) => spec,
        Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
    };
    let gate = match orchestration_plan::validate_person_gate(&phase_id, &spec) {
        Ok(gate) => gate,
        Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
    };
    // One sample of the master switch; the model check only after the proof,
    // sending nothing of the chat. A phase gate reads the chat it belongs
    // to, at that chat's tier.
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let checked = match checked_session(&routes.state, &session, enforced).await {
        Ok(checked) => checked,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if let Err(error) = routes
        .service
        .check_model_use(
            &checked,
            gate.model.as_ref(),
            &gate.questions,
            ModelSource::Setting,
        )
        .await
    {
        return plain(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "Phase \"{phase_id}\"'s gate cannot be answered in this chat: {}. Nothing was \
                 changed.",
                without_full_stop(&error.to_string())
            ),
        );
    }
    let written = orchestration_plan::person_set_gate(
        routes.state.session_manager(),
        &session_id,
        &phase_id,
        gate,
    )
    .await;
    gate_response(&routes.state, &session_id, &headers, written).await
}

#[utoipa::path(
    delete,
    path = "/sessions/{session_id}/phase-gates/{phase_id}",
    tag = "workspace",
    params(
        ("session_id" = String, Path, description = "The chat whose plan holds the phase"),
        ("phase_id" = String, Path, description = "The phase whose gate to remove")
    ),
    responses(
        (status = 200, description = "The gate is removed and the phases after it are released; \
                                      the plan as this caller may see it", body = PhaseGateResponse),
        (status = 400, description = "Invalid session or phase id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused: the named chat is out of this caller's reach (the \
                                      same refusal, word for word, that `GET \
                                      /sessions/{session_id}` gives), or, on a daemon that holds \
                                      a user-action key, the request carried no proof it came \
                                      from the person at the keyboard, or, on a daemon that \
                                      holds none, the chat is a subagent's (body = plain text)"),
        (status = 404, description = "No such chat, the chat has no orchestration plan, or the \
                                      plan has no such phase (body = plain text)"),
        (status = 409, description = "The phase is completed or skipped, it has no gate, or the \
                                      plan could not be read or written. Nothing was changed \
                                      (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn delete_phase_gate(
    State(routes): State<DecisionRoutes>,
    Path((session_id, phase_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let session = match reach_chat(&routes.state, &session_id, &headers).await {
        Ok(session) => session,
        Err(refusal) => return refusal,
    };
    if let Some(refusal) = write_refusal_with(&session, &headers, PHASE_GATES_NEED_USER) {
        return refusal;
    }
    if !valid_phase_id(&phase_id) {
        return plain(StatusCode::BAD_REQUEST, NOT_A_PHASE_ID);
    }
    let written = orchestration_plan::person_remove_gate(
        routes.state.session_manager(),
        &session_id,
        &phase_id,
    )
    .await;
    gate_response(&routes.state, &session_id, &headers, written).await
}

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/phase-gates/{phase_id}/decision",
    tag = "workspace",
    params(
        ("session_id" = String, Path, description = "The chat whose plan holds the phase"),
        ("phase_id" = String, Path, description = "The gated phase to decide")
    ),
    request_body = PhaseGateDecisionRequest,
    responses(
        (status = 200, description = "What the decision did: `applied` is false when a pass or \
                                      skip stands until the phase's working subagents finish; \
                                      the plan as this caller may see it", body = PhaseGateResponse),
        (status = 400, description = "Invalid session or phase id, or a decision that could not \
                                      be read (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused: the named chat is out of this caller's reach (the \
                                      same refusal, word for word, that `GET \
                                      /sessions/{session_id}` gives), or, on a daemon that holds \
                                      a user-action key, the request carried no proof it came \
                                      from the person at the keyboard, or, on a daemon that \
                                      holds none, the chat is a subagent's (body = plain text)"),
        (status = 404, description = "No such chat, the chat has no orchestration plan, or the \
                                      plan has no such phase (body = plain text)"),
        (status = 409, description = "The phase is completed or skipped, it has no gate, or the \
                                      plan could not be read or written. Nothing was changed \
                                      (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn decide_phase_gate(
    State(routes): State<DecisionRoutes>,
    Path((session_id, phase_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match reach_chat(&routes.state, &session_id, &headers).await {
        Ok(session) => session,
        Err(refusal) => return refusal,
    };
    if let Some(refusal) = write_refusal_with(&session, &headers, PHASE_GATES_NEED_USER) {
        return refusal;
    }
    if !valid_phase_id(&phase_id) {
        return plain(StatusCode::BAD_REQUEST, NOT_A_PHASE_ID);
    }
    let request: PhaseGateDecisionRequest = match read_settings_body(&body, "gate decision") {
        Ok(request) => request,
        Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
    };
    let written = orchestration_plan::person_gate_decision(
        routes.state.session_manager(),
        &session_id,
        &phase_id,
        request.decision,
    )
    .await;
    gate_response(&routes.state, &session_id, &headers, written).await
}

// ─── Installing a local decision model ───

#[utoipa::path(
    get,
    path = "/decisions/models/install",
    tag = "decisions",
    responses(
        (status = 200, description = "The decision models Biorouter installs, whether each is \
                                      installed, where the configured Ollama is, and the current \
                                      or last install", body = DecisionInstallStatus),
        (status = 401, description = "Unauthorized - Invalid or missing API key")
    ),
    security(("api_key" = []))
)]
pub async fn get_decision_install(State(routes): State<DecisionRoutes>) -> Response {
    Json(DecisionInstallStatus::from(
        routes.installer.snapshot().await,
    ))
    .into_response()
}

#[utoipa::path(
    post,
    path = "/decisions/models/install",
    tag = "decisions",
    request_body = DecisionInstallRequest,
    responses(
        (status = 202, description = "The install started, or the same model was installing \
                                      already; poll `GET /decisions/models/install` for its \
                                      progress", body = DecisionInstallStatus),
        (status = 400, description = "The model is not one Biorouter installs, or the request \
                                      could not be read (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 409, description = "Another model is still installing, or OLLAMA_HOST names an \
                                      Ollama on another computer or cannot be read. Nothing was \
                                      sent to another computer (body = plain text)"),
        (status = 503, description = "Ollama is not running on this computer (body = plain text)")
    ),
    security(("api_key" = []))
)]
pub async fn post_decision_install(State(routes): State<DecisionRoutes>, body: Bytes) -> Response {
    // An empty body installs the default model.
    let request: DecisionInstallRequest = if body.iter().all(u8::is_ascii_whitespace) {
        DecisionInstallRequest::default()
    } else {
        match read_settings_body(&body, "install request") {
            Ok(request) => request,
            Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
        }
    };
    let model = request.model.as_deref().unwrap_or(DEFAULT_INSTALL_MODEL);
    if let Err(refusal) = routes.installer.start(model).await {
        let status = StatusCode::from_u16(refusal.kind.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return plain(status, refusal.message);
    }
    (
        StatusCode::ACCEPTED,
        Json(DecisionInstallStatus::from(
            routes.installer.snapshot().await,
        )),
    )
        .into_response()
}

/// The routes over the process's decision service and installer.
pub fn routes(state: Arc<AppState>) -> Router {
    routes_with_service(
        state,
        biorouter::decision::global(),
        biorouter::providers::decision::install::installer(),
    )
}

/// The routes over a given decision service and installer (tests hand in a
/// service over a fake source and an installer over a mock Ollama).
pub fn routes_with_service(
    state: Arc<AppState>,
    service: &'static DecisionService,
    installer: &'static DecisionInstaller,
) -> Router {
    Router::new()
        .route("/decisions/models", get(list_decision_models))
        .route(
            "/decisions/models/install",
            get(get_decision_install).post(post_decision_install),
        )
        .route("/decisions/decide", post(decide))
        .route(
            "/sessions/{session_id}/decisions",
            get(list_session_decisions),
        )
        .route(
            "/sessions/{session_id}/decisions/summary",
            get(get_session_decision_summary),
        )
        .route(
            "/sessions/{session_id}/decisions/{decision_id}",
            get(get_session_decision),
        )
        .route(
            "/sessions/{session_id}/decision-settings",
            get(get_decision_settings).put(put_decision_settings),
        )
        .route(
            "/sessions/{session_id}/decision-chat",
            get(get_decision_chat).put(put_decision_chat),
        )
        .route(
            "/sessions/{session_id}/phase-gates/{phase_id}",
            put(put_phase_gate).delete(delete_phase_gate),
        )
        .route(
            "/sessions/{session_id}/phase-gates/{phase_id}/decision",
            post(decide_phase_gate),
        )
        .with_state(DecisionRoutes {
            state,
            service,
            installer,
        })
}

#[cfg(test)]
#[path = "decisions_tests.rs"]
mod tests;
