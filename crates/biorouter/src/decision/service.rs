//! [`DecisionService`]: the one path every decision takes, whoever asks (a
//! monitor hook, the `decide` tool, a phase gate, a decision chat, a route or
//! the CLI). In a fixed order it
//!
//! 1. validates the request,
//! 2. resolves a decision model: the one named, else
//!    `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL`, else the first
//!    available model the chat may use, local first;
//! 3. checks privacy (Gate H) and Crew **before anything is sent**;
//! 4. checks the request fits the model's window;
//! 5. makes the call, under a per-model concurrency limit and a deadline;
//! 6. hands the outcome to a [`DecisionSink`], the seam the decision log fills.
//!
//! # Privacy: which Gate H rule, and why
//!
//! A decision hands chat content (a monitor's window, a tool's chunks, a
//! gate's hand-off text) to a model the session row never records, so it is
//! an alternate-provider site. It uses the **downward** half,
//! [`assert_alt_tier_allowed`], which is `assert_alt_provider_allowed`'s rule
//! keyed on the adapter's declared tier: a private chat never reaches a public
//! decision model. That is the rule plan mode and prompt hooks follow, and it
//! fits for the same reason: a decision's answer lands back in the same chat
//! and ratchets nothing.
//!
//! **Owner ruling (2026-11-05): a public chat may use a private (local)
//! decision model, and no decision raises the chat's classification.** The
//! model reads only text the chat's own model already has (a monitor's
//! window, the `decide` tool's chunks, a gate's hand-off), so sending it to
//! this computer discloses nothing more. Nothing but probabilities over the
//! answers the chat offered reaches the chat or its model. The one free text
//! a decision can produce is a logprob model's verbalized fallback, which
//! keeps up to 2,000 characters of the model's reply in the log row's raw
//! JSON, never in the transcript. Refusing the cell (the symmetric rule)
//! would push the same text to a hosted model off this computer instead, and
//! ratcheting the chat would leave its public model unable to run the next
//! turn. An ad hoc decision about a chat is the exception to "the chat's own
//! text": its caller supplies the text, which is then kept at the chat's
//! reach, so `biorouter decide --session` says so before sending.
//!
//! The master switch is sampled once per call by whoever builds the
//! [`DecisionSession`] (`cap.enforced()` on a tool path) and threaded, never
//! re-read here. Affiliation is checked beside the tier with
//! [`owners_compatible`], as PR #469 did, so a UCSF model does not read another
//! institution's private data.
//!
//! ⚠ Opening an Ollama model asks its server's `/api/show` before the gate
//! runs, because the answer (a `remote_host`) is part of the model's tier. That
//! request carries the model's name and nothing of the chat.
//!
//! # Crew
//!
//! A Crew grant fixes the chat model a chat's content may reach
//! (`CrewManager::check_provider_binding` refuses any other provider as
//! "model fixed"). A decision model that runs on this computer
//! (`ModelAffiliation::Local`) moves nothing off the machine, so the default
//! [`CrewUseGate`] lets a Crew chat use one, and refuses any other decision
//! model before anything is sent ([`CREW_SCOPED_REFUSAL`]). A grant that no
//! longer stands (revoked, ended, its policy changed, its connection removed,
//! run out of time, or one this device cannot confirm) refuses every decision,
//! by the same local check that holds a rewrite of the chat's history
//! (`CrewManager::history_rewrite_refusal`), which never asks the workspace.
//!
//! # Late and abandoned answers
//!
//! Each request runs on a task of its own, holding a
//! [`super::late::InFlightGuard`] and never the concurrency permit. The caller
//! waits for it until the deadline. When the deadline passes, the timeout row
//! is written as before and the task is left to finish; when the caller goes
//! away (a stopped turn, a dropped HTTP request), the task finishes all the
//! same. Either way an answer that arrives later is handed to
//! [`DecisionSink::record_late`] under the request's id, so its tokens reach
//! the decision log and the usage ledger, and a priced one's cost reaches the
//! live turn's reply budget ([`super::turns::note_late_cost`]). It is never
//! counted in the turn's tally: the timeout row already was. Exactly one of the
//! waiter and the task hands a late answer over. A process that exits waits
//! for such tasks with [`super::late::settle_late_answers`].
//!
//! # Save-time checks
//!
//! [`DecisionService::check_model_use`] asks everything a decision would ask
//! before sending (the Crew grant, the model's resolution, Gate H and
//! affiliation, the question kinds, the server's refusal to load the model)
//! and sends nothing of the chat. [`DecisionService::check_settings`] and
//! [`DecisionService::check_chat_question`] run it for what a save turns on,
//! so a monitor that could never run is refused when it is saved rather than
//! failing at every turn (`super::save`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use indexmap::IndexMap;
use tokio::sync::{oneshot, Semaphore};

use super::error::{DecisionError, DecisionResult, INSTALL_ADVICE};
use super::model::{DecisionCaps, DecisionModel, DecisionModelEntry, DecisionModelSource};
use super::settings::{DecisionSettings, MonitorCriterion};
use super::types::{
    decision_from_verdict, validate_questions, ChunkOutcome, Decision, DecisionChunk,
    DecisionContext, DecisionKind, DecisionMeta, DecisionModelRef, DecisionOrigin, DecisionRequest,
    DecisionUsage, ModelVerdicts, Question,
};
use crate::config::Config;
use crate::privacy::affiliation::{owners_compatible, InstitutionId, ModelAffiliation};
use crate::privacy::alt_provider::{assert_alt_tier_allowed, DownwardAdvice};
use crate::privacy::{ProviderTier, SessionClassification};
use crate::providers::decision::catalog::{rank_local_first, ConfiguredDecisionModels};
use crate::session::SessionManager;

/// Remote models answer in tens of milliseconds (vendor figures); ten seconds
/// is generous and still bounds a stuck request.
pub const DEFAULT_REMOTE_TIMEOUT: Duration = Duration::from_secs(10);
/// A local model's first call includes loading it.
pub const DEFAULT_LOCAL_FIRST_TIMEOUT: Duration = Duration::from_secs(30);
/// Chunks in one batch.
pub const MAX_BATCH_CHUNKS: usize = 1_000;
/// o200k under-counts the Qwen-family tokenizers decision models use; the same
/// margin `token_counter` applies to every non-OpenAI estimate.
const TOKEN_ESTIMATE_MARGIN: f64 = 1.15;
/// Tokens each question adds beyond its own text (type, separators).
const PER_QUESTION_OVERHEAD: usize = 8;
/// The env key a refusal names as the way out when the configured default
/// chose the model.
const MODEL_KEY: &str = "BIOROUTER_DECISION_MODEL";
/// The way out a refusal names when the caller named the model (a `decide`
/// call, a monitor, a gate, the chat's own default), where setting the
/// configured default would not change the model asked.
const NAMED_MODEL_ADVICE: &str = "Name a private decision model (decision_models lists them)";
/// The way out a refusal names when a person's setting chose the model (a
/// monitor's model, the chat's default decision model), checked as the
/// setting is saved.
const SETTING_MODEL_ADVICE: &str = "Choose a decision model that runs on this computer";

/// Where the model a decision runs on came from, which decides what a privacy
/// refusal advises. A caller that resolves the model itself and pins it for a
/// run (`biorouter decide`) takes this from [`Self::of`] before it pins, and
/// hands it to [`DecisionService::decide_pinned`]: to the service a pinned
/// model is a named one, so without it a model the configured default chose
/// would be refused with the advice for a model the caller named.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelSource {
    /// The caller named it, or the chat's own default did.
    Named,
    /// The configured default, or the automatic choice.
    #[default]
    Configured,
    /// A person's setting names it or sets the default it comes from (a
    /// monitor's model, the chat's default decision model, a decision chat's
    /// bound model), checked as the setting is saved.
    Setting,
}

impl ModelSource {
    /// Where the model [`DecisionService::resolve`] opens for `session` and
    /// `wanted` comes from, by its own order.
    pub fn of(session: Option<&DecisionSession>, wanted: Option<&DecisionModelRef>) -> Self {
        if wanted.is_some() || matches!(chat_default_model(session), Ok(Some(_))) {
            Self::Named
        } else {
            Self::Configured
        }
    }

    fn advice(self) -> DownwardAdvice<'static> {
        match self {
            Self::Named => DownwardAdvice::Instead(NAMED_MODEL_ADVICE),
            Self::Configured => DownwardAdvice::SetKey(MODEL_KEY),
            Self::Setting => DownwardAdvice::Instead(SETTING_MODEL_ADVICE),
        }
    }

    /// [`Self::advice`] as the sentence that ends a refusal says it.
    fn advice_text(self) -> String {
        match self.advice() {
            DownwardAdvice::SetKey(key) => format!("Set {key} to a private model"),
            DownwardAdvice::Instead(instead) => instead.to_string(),
        }
    }
}

/// The chat a decision is about, as its caller sampled it.
#[derive(Clone)]
pub struct DecisionSession {
    pub session_id: String,
    /// The chat's classification. A phase gate uses its parent chat's, and
    /// the gate's state already withholds every child more private than the
    /// parent (`orchestration_plan::screened_state`), so nothing a gate sends
    /// is more private than this. Do not raise it to a child's tier instead:
    /// that would send a private child's text to a private model and report
    /// the verdict into the public parent.
    pub classification: SessionClassification,
    /// The institutions whose private data the chat holds.
    pub affiliations: BTreeSet<InstitutionId>,
    /// The privacy master switch, sampled once by the caller for this call
    /// (`cap.enforced()` on a tool path, `privacy_tiers_enabled()` elsewhere).
    pub enforced: bool,
    pub origin: DecisionOrigin,
    /// The criterion, tool call or phase that asked.
    pub origin_id: Option<String>,
    /// The agent turn this decision belongs to, so surfaces can aggregate per
    /// turn instead of listing every call.
    pub turn_id: Option<String>,
    /// A human label for the origin (a monitor's name, "decide", a phase title).
    pub origin_label: Option<String>,
    /// Re-read before every request, so a chat raised to Private in the middle
    /// of a batch stops reaching a public model at the next chunk.
    pub live: Option<Arc<dyn LiveClassification>>,
    /// The chat's own default decision model (`defaultModel` in
    /// `decision_settings.v1`): asked when a call names no model, before the
    /// configured default, so a person who chose a local model for this chat
    /// is not overruled by `BIOROUTER_DECISION_MODEL`. `Err` says why the
    /// chat's settings could not be read; a call that names no model is then
    /// refused rather than sent where that default may have ruled out.
    pub chat_default_model: Result<Option<DecisionModelRef>, String>,
    /// The capability tier of the chat model the chat is bound to now, when
    /// the caller knows it. Read ONLY by the save-time checks
    /// ([`DecisionService::check_model_use`]), never by a decision: a chat
    /// still Public but bound to a private model becomes private at its first
    /// turn, so a public decision model saved for it now would be refused at
    /// every turn. It compares capability with capability and never crosses
    /// into classification (no `privacy::floor`).
    pub bound_tier: Option<ProviderTier>,
}

impl std::fmt::Debug for DecisionSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionSession")
            .field("session_id", &self.session_id)
            .field("classification", &self.classification)
            .field("enforced", &self.enforced)
            .field("origin", &self.origin)
            .field("live", &self.live.is_some())
            .field("bound_tier", &self.bound_tier)
            .finish_non_exhaustive()
    }
}

impl DecisionSession {
    pub fn new(
        session_id: impl Into<String>,
        classification: SessionClassification,
        enforced: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            classification,
            affiliations: BTreeSet::new(),
            enforced,
            origin: DecisionOrigin::Adhoc,
            origin_id: None,
            turn_id: None,
            origin_label: None,
            live: None,
            chat_default_model: Ok(None),
            bound_tier: None,
        }
    }

    /// The tier of the chat model the chat is bound to, for the save-time
    /// checks only (see [`Self::bound_tier`]).
    pub fn with_bound_tier(mut self, tier: Option<ProviderTier>) -> Self {
        self.bound_tier = tier;
        self
    }

    pub fn with_chat_default_model(
        mut self,
        model: Result<Option<DecisionModelRef>, String>,
    ) -> Self {
        self.chat_default_model = model;
        self
    }

    pub fn with_affiliations(mut self, affiliations: BTreeSet<InstitutionId>) -> Self {
        self.affiliations = affiliations;
        self
    }

    pub fn with_origin(mut self, origin: DecisionOrigin, origin_id: Option<String>) -> Self {
        self.origin = origin;
        self.origin_id = origin_id;
        self
    }

    pub fn with_turn(mut self, turn_id: Option<String>) -> Self {
        self.turn_id = turn_id;
        self
    }

    pub fn with_origin_label(mut self, label: Option<String>) -> Self {
        self.origin_label = label;
        self
    }

    pub fn with_live(mut self, live: Arc<dyn LiveClassification>) -> Self {
        self.live = Some(live);
        self
    }

    /// The chat's stored classification and affiliations, read now, with the
    /// store kept for the per-request re-read.
    pub async fn load(
        manager: Arc<SessionManager>,
        session_id: &str,
        enforced: bool,
    ) -> anyhow::Result<Self> {
        let session = manager.get_session(session_id, false).await?;
        let affiliations = manager.session_affiliations(session_id).await?;
        let chat_default_model = load_chat_default_model(&manager, session_id).await;
        Ok(Self::new(session_id, session.privacy_tier, enforced)
            .with_affiliations(affiliations)
            .with_chat_default_model(chat_default_model)
            .with_live(manager))
    }
}

/// The chat's default decision model as [`DecisionSession::chat_default_model`]
/// holds it: `Err` (a sentence) when its decision settings cannot be read.
pub async fn load_chat_default_model(
    manager: &SessionManager,
    session_id: &str,
) -> Result<Option<DecisionModelRef>, String> {
    super::settings::load(manager, session_id)
        .await
        .map(|settings| settings.default_model)
        .map_err(|error| error.to_string())
}

/// The second rung of the model order: the chat's own default, when a call
/// names no model.
fn chat_default_model(
    session: Option<&DecisionSession>,
) -> DecisionResult<Option<DecisionModelRef>> {
    match session.map(|session| &session.chat_default_model) {
        None | Some(Ok(None)) => Ok(None),
        Some(Ok(Some(model))) => Ok(Some(model.clone())),
        Some(Err(why)) => Err(DecisionError::NoModel(format!(
            "{why}, so this chat's default decision model is unknown. Name a model to decide with."
        ))),
    }
}

/// A chat's classification as stored now.
#[async_trait]
pub trait LiveClassification: Send + Sync {
    async fn current(&self, session_id: &str) -> anyhow::Result<SessionClassification>;
}

#[async_trait]
impl LiveClassification for SessionManager {
    async fn current(&self, session_id: &str) -> anyhow::Result<SessionClassification> {
        Ok(self.get_session(session_id, false).await?.privacy_tier)
    }
}

/// A check the chat's workspace makes before its content reaches a model.
#[async_trait]
pub trait DecisionUseGate: Send + Sync {
    async fn check(
        &self,
        session: &DecisionSession,
        model: &dyn DecisionModel,
    ) -> DecisionResult<()>;

    /// Why `session` may reach no decision model at all, whichever is named,
    /// or `None`. A listing ([`DecisionService::models_for`]) gives it as
    /// every model's reason, so a picker says what [`Self::check`] would
    /// refuse before anything is tried. A gate whose rule depends on the
    /// model answers `None` here and refuses in `check`.
    async fn session_refusal(&self, _session: &DecisionSession) -> Option<String> {
        None
    }

    /// Why `session` may not use the listed model `entry`, by this gate's own
    /// rule, or `None`. A listing gives it as that row's reason, and the
    /// automatic choice passes over a model it names, so a chat that names no
    /// model is given one this gate admits. Advisory, as a listing is:
    /// [`Self::check`] is asked again before every call.
    async fn entry_refusal(
        &self,
        _session: &DecisionSession,
        _entry: &DecisionModelEntry,
    ) -> Option<String> {
        None
    }
}

/// The Crew rule for decisions. A Crew grant lets a chat's content reach only
/// the model it was granted with, which a decision model never is, and
/// decision models that run on this computer, which move nothing off it. So a
/// Crew-scoped chat may use a local decision model and no other, and only
/// while its grant stands.
#[derive(Debug, Default, Clone, Copy)]
pub struct CrewUseGate;

/// What a Crew registry that cannot be read refuses with.
fn crew_unreadable(error: impl std::fmt::Display) -> String {
    format!("The Crew registry could not be read, so no decision model was asked: {error}")
}

impl CrewUseGate {
    /// [`CREW_SCOPED_REFUSAL`] when `session` is Crew-scoped and the model does
    /// not run on this computer; `None` otherwise.
    async fn model_refusal(
        &self,
        session: &DecisionSession,
        affiliation: Option<ModelAffiliation>,
    ) -> Option<String> {
        if matches!(affiliation, Some(ModelAffiliation::Local)) {
            return None;
        }
        match crate::crew::manager() {
            Ok(manager) => manager
                .is_scoped_session(&session.session_id)
                .await
                .then(|| CREW_SCOPED_REFUSAL.to_string()),
            Err(error) => Some(crew_unreadable(error)),
        }
    }
}

#[async_trait]
impl DecisionUseGate for CrewUseGate {
    async fn check(
        &self,
        session: &DecisionSession,
        model: &dyn DecisionModel,
    ) -> DecisionResult<()> {
        if let Some(reason) = self.session_refusal(session).await {
            return Err(DecisionError::Refused(reason));
        }
        match self.model_refusal(session, model.affiliation()).await {
            Some(reason) => Err(DecisionError::Refused(reason)),
            None => Ok(()),
        }
    }

    /// Why the chat's grant reaches no decision model at all: it no longer
    /// stands (removed, ended, its policy changed, its connection removed,
    /// run out of time), or this device cannot confirm it is the chat's. The
    /// local standing check a rewrite of the chat's history asks
    /// (`CrewManager::history_rewrite_refusal`), which never asks the
    /// workspace. `None` for a chat no grant restricts and for a grant that
    /// stands. It never reads `session.enforced`: Crew is not the privacy
    /// master switch.
    async fn session_refusal(&self, session: &DecisionSession) -> Option<String> {
        match crate::crew::manager() {
            Ok(manager) => manager.history_rewrite_refusal(&session.session_id).await,
            Err(error) => Some(crew_unreadable(error)),
        }
    }

    async fn entry_refusal(
        &self,
        session: &DecisionSession,
        entry: &DecisionModelEntry,
    ) -> Option<String> {
        self.model_refusal(session, entry.affiliation).await
    }
}

/// Why a Crew-scoped chat reaches no decision model hosted elsewhere.
pub const CREW_SCOPED_REFUSAL: &str =
    "This chat works under a Crew grant, which lets its content reach only the model it was \
     granted with and decision models that run on this computer, so a decision model hosted \
     elsewhere cannot read it. Choose one that runs on this computer, such as tev1:0.8b.";

/// What happened to one decision call, for the decision log.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// Every question with a threshold passed it.
    Passed,
    /// A question with a threshold failed it.
    Failed,
    /// Decided, with no threshold to pass.
    Decided,
    Error,
    Refused,
}

/// One row's worth of what a decision call saw and did. The sink caps and
/// stores it; the service only hands it over.
#[derive(Debug, Clone)]
pub struct DecisionLogEntry {
    pub session_id: Option<String>,
    pub origin: DecisionOrigin,
    pub origin_id: Option<String>,
    pub origin_label: Option<String>,
    pub turn_id: Option<String>,
    pub batch_id: Option<String>,
    pub chunk_index: Option<usize>,
    pub chunk_id: Option<String>,
    pub model: Option<DecisionModelRef>,
    pub served_model: Option<String>,
    pub tier: Option<ProviderTier>,
    pub questions: IndexMap<String, Question>,
    /// Exactly what was (or would have been) sent, kept at the chat's tier.
    /// For a monitor, `decide`, a gate or a decision chat it is the chat's own
    /// content; for an ad hoc decision it is whatever the caller sent.
    pub context: DecisionContext,
    pub context_bytes: usize,
    pub outcome: DecisionOutcome,
    pub decisions: Vec<Decision>,
    pub error: Option<String>,
    pub latency_ms: Option<u64>,
    pub usage: DecisionUsage,
    pub privacy_enforced: bool,
}

/// Where decisions are recorded. A failed write is logged and never fails the
/// decision (PR #469's `.await?` on the ledger write ended the reply).
#[async_trait]
pub trait DecisionSink: Send + Sync {
    async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()>;

    /// [`Self::record`], storing the row under `id`, which the caller chose
    /// before the call ([`DecisionService::decide_as`]), so a failed request,
    /// whose row has no decision to carry an id, can still be named. A
    /// successful request's decisions already carry the same id. The default
    /// ignores `id`, for sinks that never name rows.
    async fn record_as(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let _ = id;
        self.record(entry).await
    }

    /// An answer that arrived after its caller stopped waiting (the deadline
    /// passed, or the caller went away), for the request whose rows are
    /// stored under `id`. `entry` is an error row carrying the tokens the
    /// request was billed for and the sentence that says what happened. A
    /// sink that bills usage stores it beside the row the request already
    /// wrote, or as that row when none was, billed once. The default does
    /// nothing, for sinks that keep no usage.
    async fn record_late(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let _ = (id, entry);
        Ok(())
    }
}

/// Which model to use when the caller names none.
#[derive(Debug, Clone)]
enum DefaultModel {
    /// `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL`, read per call.
    FromConfig,
    Fixed(Option<DecisionModelRef>),
    /// A decision provider and no model, as `BIOROUTER_DECISION_PROVIDER`
    /// alone configures.
    #[cfg(test)]
    Provider(String),
}

/// The decision service. One per process in production ([`global`]); tests
/// build their own over their own [`DecisionModelSource`].
pub struct DecisionService {
    source: Arc<dyn DecisionModelSource>,
    use_gate: Arc<dyn DecisionUseGate>,
    sink: RwLock<Option<Arc<dyn DecisionSink>>>,
    default_model: DefaultModel,
    remote_timeout: Option<Duration>,
    local_first_timeout: Option<Duration>,
    limits: Mutex<HashMap<String, Arc<Semaphore>>>,
    warm: Mutex<HashSet<String>>,
}

static GLOBAL: LazyLock<DecisionService> =
    LazyLock::new(|| DecisionService::new(Arc::new(ConfiguredDecisionModels)));

/// The process's decision service, over the configured models.
pub fn global() -> &'static DecisionService {
    &GLOBAL
}

/// What [`DecisionService::decide_one`] needs besides the request.
#[derive(Clone, Copy, Default)]
struct Attempt<'a> {
    session: Option<&'a DecisionSession>,
    batch_id: Option<&'a str>,
    chunk_index: Option<usize>,
    chunk_id: Option<&'a str>,
    /// The request id the caller chose ([`DecisionService::decide_as`]), else
    /// one is minted per request.
    request_id: Option<&'a str>,
    /// Set inside a batch: a refusal is not logged per chunk, because it is
    /// the chat's (privacy or Crew), and the batch logs one row for every
    /// chunk it stopped.
    defer_refusal: bool,
    /// Chunks a batch's one stopped row stands for (zero reads as one), so the
    /// turn's tally counts each of them as the tool's own result does.
    stopped_chunks: usize,
    /// Where the model came from, for what a privacy refusal advises.
    source: ModelSource,
}

impl<'a> Attempt<'a> {
    fn single(session: Option<&'a DecisionSession>, request_id: Option<&'a str>) -> Self {
        Self {
            session,
            request_id,
            ..Self::default()
        }
    }
}

/// One batch while its chunks run.
struct Batch<'a> {
    session: Option<&'a DecisionSession>,
    model: Arc<dyn DecisionModel>,
    source: ModelSource,
    questions: &'a IndexMap<String, Question>,
    batch_id: &'a str,
    stopped: Mutex<Stopped>,
}

/// What stopped a batch partway (the first chat-wide refusal) and the chunks
/// it held back, by position, so the batch logs them in one row.
#[derive(Default)]
struct Stopped {
    error: Option<DecisionError>,
    chunks: Vec<(usize, String)>,
}

impl Batch<'_> {
    fn lock(&self) -> std::sync::MutexGuard<'_, Stopped> {
        self.stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The refusal's sentence when the batch has stopped, with this chunk
    /// counted among those it held back; `None` while the batch runs.
    fn hold_if_stopped(&self, index: usize, chunk_id: &str) -> Option<String> {
        let mut stopped = self.lock();
        let reason = stopped.error.as_ref()?.to_string();
        stopped.chunks.push((index, chunk_id.to_string()));
        Some(reason)
    }

    /// Stop the batch at this chunk's refusal; the first refusal is the one
    /// the batch's row gives.
    fn stop(&self, index: usize, chunk_id: &str, error: &DecisionError) {
        let mut stopped = self.lock();
        if stopped.error.is_none() {
            stopped.error = Some(error.clone());
        }
        stopped.chunks.push((index, chunk_id.to_string()));
    }
}

impl DecisionService {
    pub fn new(source: Arc<dyn DecisionModelSource>) -> Self {
        Self {
            source,
            use_gate: Arc::new(CrewUseGate),
            sink: RwLock::new(None),
            default_model: DefaultModel::FromConfig,
            remote_timeout: None,
            local_first_timeout: None,
            limits: Mutex::new(HashMap::new()),
            warm: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_use_gate(mut self, gate: Arc<dyn DecisionUseGate>) -> Self {
        self.use_gate = gate;
        self
    }

    /// Pin the default model instead of reading config (`None`: always choose
    /// automatically).
    pub fn with_default_model(mut self, model: Option<DecisionModelRef>) -> Self {
        self.default_model = DefaultModel::Fixed(model);
        self
    }

    /// Pin a configured decision provider with no model, instead of reading
    /// config.
    #[cfg(test)]
    fn with_default_provider(mut self, provider: &str) -> Self {
        self.default_model = DefaultModel::Provider(provider.to_string());
        self
    }

    pub fn with_timeouts(mut self, remote: Duration, local_first: Duration) -> Self {
        self.remote_timeout = Some(remote);
        self.local_first_timeout = Some(local_first);
        self
    }

    /// Install the decision log. Replaces any earlier sink.
    pub fn set_sink(&self, sink: Arc<dyn DecisionSink>) {
        if let Ok(mut slot) = self.sink.write() {
            *slot = Some(sink);
        }
    }

    pub fn with_sink(self, sink: Arc<dyn DecisionSink>) -> Self {
        self.set_sink(sink);
        self
    }

    /// The models available now, local first.
    pub async fn models(&self) -> Vec<super::model::DecisionModelEntry> {
        rank_local_first(self.source.list().await)
    }

    /// [`Self::models`], each with the reason `session` may not use it, or
    /// `None` when it may: the workspace's own rule for the chat (a Crew grant
    /// that no longer stands, [`DecisionUseGate::session_refusal`]), then its
    /// rule for this model ([`DecisionUseGate::entry_refusal`]: a Crew chat
    /// reaches only decision models on this computer), then Gate H's downward
    /// rule and affiliation, answered by the same checks [`Self::decide`] asks
    /// before it sends anything, then the server's own refusal to load the
    /// model ([`DecisionModelEntry::load_refusal`]), which holds with or
    /// without a session. With no session every model that loads is usable.
    /// Advisory only, for pickers and listings: the gates are asked again, on the
    /// classification as it stands then, before every call, and a model the
    /// caller names is still opened and asked. A row is a model one would
    /// name, so its privacy reason advises naming a private one.
    pub async fn models_for(
        &self,
        session: Option<&DecisionSession>,
    ) -> Vec<(super::model::DecisionModelEntry, Option<String>)> {
        let workspace = match session {
            Some(session) => self.session_refusal(session).await,
            None => None,
        };
        let entries = self.models().await;
        let mut gated = Vec::with_capacity(entries.len());
        for entry in &entries {
            gated.push(match (session, &workspace) {
                (Some(session), None) => self.use_gate.entry_refusal(session, entry).await,
                _ => None,
            });
        }
        entries
            .into_iter()
            .zip(gated)
            .map(|(entry, gated)| {
                let reason = workspace
                    .clone()
                    .or(gated)
                    .or_else(|| {
                        session.and_then(|session| {
                            privacy_allows(
                                session,
                                &entry.display_name,
                                entry.tier,
                                entry.affiliation,
                                ModelSource::Named,
                            )
                            .err()
                            .map(|error| error.to_string())
                        })
                    })
                    .or_else(|| entry.load_refusal.clone());
                (entry, reason)
            })
            .collect()
    }

    /// Why `session` may reach no decision model at all, whichever is named
    /// or chosen for it (a Crew grant, [`DecisionUseGate::session_refusal`]),
    /// or `None`. [`Self::models_for`] gives it as every row's reason; this is
    /// for a choice that names no model, such as a monitor left on the chat's
    /// decision model, which no row stands for. Advisory, as a listing is:
    /// every call asks the gate again.
    pub async fn session_refusal(&self, session: &DecisionSession) -> Option<String> {
        self.use_gate.session_refusal(session).await
    }

    /// Answer `request`'s questions over its state.
    ///
    /// `session` is the chat whose content the state is, or `None` for an ad
    /// hoc decision the person asked for directly (the CLI warns them when the
    /// model is Public). `model` is the model to use, or `None` to resolve one.
    pub async fn decide(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&DecisionModelRef>,
        request: &DecisionRequest,
    ) -> DecisionResult<Vec<Decision>> {
        let source = ModelSource::of(session, model);
        self.decide_single(model, request, Attempt::single(session, None), source)
            .await
    }

    /// [`Self::decide`] on `model`, which the caller resolved by the service's
    /// own order ([`Self::model_entry_for`], [`Self::resolve_for`]) and pinned
    /// for a run, with `source` saying where it came from
    /// ([`ModelSource::of`] for what the caller named), so a privacy refusal
    /// advises what would change the model rather than treating every pinned
    /// model as one the caller named.
    pub async fn decide_pinned(
        &self,
        session: Option<&DecisionSession>,
        model: &DecisionModelRef,
        source: ModelSource,
        request: &DecisionRequest,
    ) -> DecisionResult<Vec<Decision>> {
        self.decide_single(Some(model), request, Attempt::single(session, None), source)
            .await
    }

    /// [`Self::decide`] under the caller's own request id, so the caller can
    /// name the log row before the call ends, whether the request succeeds or
    /// fails (a decision chat's reply names its row either way). Every
    /// decision of the request carries `request_id`, and a failure's row is
    /// stored under it. A request that fails validation is not logged, as with
    /// [`Self::decide`]: nothing was resolved or sent.
    pub async fn decide_as(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&DecisionModelRef>,
        request: &DecisionRequest,
        request_id: &str,
    ) -> DecisionResult<Vec<Decision>> {
        let source = ModelSource::of(session, model);
        self.decide_single(
            model,
            request,
            Attempt::single(session, Some(request_id)),
            source,
        )
        .await
    }

    async fn decide_single(
        &self,
        model: Option<&DecisionModelRef>,
        request: &DecisionRequest,
        attempt: Attempt<'_>,
        source: ModelSource,
    ) -> DecisionResult<Vec<Decision>> {
        request.validate().map_err(DecisionError::Invalid)?;
        let resolved = match self.resolve(attempt.session, model).await {
            Ok(resolved) => resolved,
            Err(error) => {
                self.record_failure(
                    attempt.session,
                    None,
                    request,
                    &error,
                    Some(&attempt),
                    DecisionUsage::default(),
                )
                .await;
                return Err(error);
            }
        };
        self.decide_one(&resolved, request, Attempt { source, ..attempt })
            .await
    }

    /// Answer the same questions over many states, one request per chunk, with
    /// bounded parallelism. The model is resolved and the privacy gate asked
    /// once before anything is sent: a refusal there is the whole batch's, and
    /// no chunk is sent. After that, one chunk's failure never fails the batch;
    /// it is reported in that chunk's [`ChunkOutcome`]. A refusal partway (the
    /// chat became private, say) is the chat's, so the chunks not yet asked
    /// are not asked: each reports that refusal, and the log holds one row for
    /// all the chunks it stopped. Outcomes keep the chunks' order.
    pub async fn decide_many(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&DecisionModelRef>,
        questions: &IndexMap<String, Question>,
        chunks: Vec<DecisionChunk>,
    ) -> DecisionResult<Vec<ChunkOutcome>> {
        let batch_id = uuid::Uuid::now_v7().to_string();
        self.decide_batch(session, model, questions, chunks, &batch_id)
            .await
    }

    /// [`Self::decide_many`] on a model the caller pinned, with where it came
    /// from, as [`Self::decide_pinned`] is to [`Self::decide`].
    pub async fn decide_many_pinned(
        &self,
        session: Option<&DecisionSession>,
        model: &DecisionModelRef,
        source: ModelSource,
        questions: &IndexMap<String, Question>,
        chunks: Vec<DecisionChunk>,
    ) -> DecisionResult<Vec<ChunkOutcome>> {
        let batch_id = uuid::Uuid::now_v7().to_string();
        self.decide_batch_from(session, Some(model), source, questions, chunks, &batch_id)
            .await
    }

    /// [`Self::decide_many`] with the caller's own batch id, so the caller can
    /// name the batch (the `decide` tool's result does, and the decision
    /// history filters on it) before or after it runs.
    pub async fn decide_batch(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&DecisionModelRef>,
        questions: &IndexMap<String, Question>,
        chunks: Vec<DecisionChunk>,
        batch_id: &str,
    ) -> DecisionResult<Vec<ChunkOutcome>> {
        let source = ModelSource::of(session, model);
        self.decide_batch_from(session, model, source, questions, chunks, batch_id)
            .await
    }

    async fn decide_batch_from(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&DecisionModelRef>,
        source: ModelSource,
        questions: &IndexMap<String, Question>,
        chunks: Vec<DecisionChunk>,
        batch_id: &str,
    ) -> DecisionResult<Vec<ChunkOutcome>> {
        validate_questions(questions).map_err(DecisionError::Invalid)?;
        if chunks.is_empty() || chunks.len() > MAX_BATCH_CHUNKS {
            return Err(DecisionError::Invalid(format!(
                "Send between 1 and {MAX_BATCH_CHUNKS} chunks, not {}",
                chunks.len()
            )));
        }
        let mut seen = HashSet::new();
        for chunk in &chunks {
            if chunk.id.trim().is_empty() || !seen.insert(chunk.id.as_str()) {
                return Err(DecisionError::Invalid(format!(
                    "Every chunk needs its own non-empty id; `{}` is empty or repeated",
                    chunk.id
                )));
            }
        }
        let resolved = match self.resolve(session, model).await {
            Ok(resolved) => resolved,
            Err(error) => {
                self.record_batch_failure(session, None, questions, &chunks, batch_id, &error)
                    .await;
                return Err(error);
            }
        };
        let caps = resolved.caps();
        let checks = async {
            self.check_kinds(resolved.as_ref(), &caps, questions)?;
            if let Some(session) = session {
                self.check_privacy(session, resolved.as_ref(), source)
                    .await?;
            }
            Ok(())
        };
        if let Err(error) = checks.await {
            self.record_batch_failure(
                session,
                Some(resolved.as_ref()),
                questions,
                &chunks,
                batch_id,
                &error,
            )
            .await;
            return Err(error);
        }
        let parallel = self.concurrency(&caps).max(1);
        let total = chunks.len();
        let batch = Batch {
            session,
            model: resolved,
            source,
            questions,
            batch_id,
            stopped: Mutex::new(Stopped::default()),
        };
        let outcomes = futures::stream::iter(chunks.into_iter().enumerate())
            .map(|(index, chunk)| self.decide_chunk(&batch, index, chunk))
            .buffered(parallel)
            .collect::<Vec<_>>()
            .await;
        self.record_stopped(&batch, total).await;
        Ok(outcomes)
    }

    /// The one row for every chunk of `batch` a refusal partway stopped, when
    /// one did. It names them rather than holding them, because none was sent.
    async fn record_stopped(&self, batch: &Batch<'_>, total: usize) {
        let Stopped {
            error,
            chunks: mut held,
        } = std::mem::take(
            &mut *batch
                .stopped
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let Some(error) = error else {
            return;
        };
        held.sort_by_key(|(index, _)| *index);
        let ids: Vec<&str> = held.iter().map(|(_, id)| id.as_str()).collect();
        let request = DecisionRequest {
            questions: batch.questions.clone(),
            context: DecisionContext::Text(unsent_partway_text(total, &ids)),
        };
        let attempt = Attempt {
            session: batch.session,
            batch_id: Some(batch.batch_id),
            stopped_chunks: held.len(),
            source: batch.source,
            ..Attempt::default()
        };
        self.record_failure(
            batch.session,
            Some(batch.model.as_ref()),
            &request,
            &error,
            Some(&attempt),
            DecisionUsage::default(),
        )
        .await;
    }

    /// One chunk of a batch. A chunk the chat's privacy or Crew rule refuses
    /// stops the rest: the chunks after it are not asked (no live read, no
    /// send), and the batch logs them in one row when it ends.
    async fn decide_chunk(
        &self,
        batch: &Batch<'_>,
        index: usize,
        chunk: DecisionChunk,
    ) -> ChunkOutcome {
        if let Some(reason) = batch.hold_if_stopped(index, &chunk.id) {
            return ChunkOutcome {
                chunk_id: chunk.id,
                decisions: None,
                error: Some(reason),
                billed_usage: None,
            };
        }
        let request = DecisionRequest {
            questions: batch.questions.clone(),
            context: chunk.context,
        };
        let result = match request.validate() {
            Err(e) => Err(DecisionError::Invalid(e)),
            Ok(()) => {
                let attempt = Attempt {
                    session: batch.session,
                    batch_id: Some(batch.batch_id),
                    chunk_index: Some(index),
                    chunk_id: Some(&chunk.id),
                    request_id: None,
                    defer_refusal: true,
                    stopped_chunks: 0,
                    source: batch.source,
                };
                self.decide_one(&batch.model, &request, attempt).await
            }
        };
        match result {
            Ok(decisions) => ChunkOutcome {
                chunk_id: chunk.id,
                decisions: Some(decisions),
                error: None,
                billed_usage: None,
            },
            Err(error) => {
                if matches!(error, DecisionError::Refused(_)) {
                    batch.stop(index, &chunk.id, &error);
                }
                // A model that answered unreadably was billed all the same.
                let billed = error.billed_usage();
                ChunkOutcome {
                    chunk_id: chunk.id,
                    decisions: None,
                    error: Some(error.to_string()),
                    billed_usage: billed.billed_total().is_some().then_some(billed),
                }
            }
        }
    }

    /// The model named, else the chat's own default, else the configured
    /// default, else the first available model this chat may use, local first.
    /// The automatic choice is made among the models the workspace's gate
    /// admits ([`DecisionUseGate::entry_refusal`]), so a Crew chat that names
    /// no model is given one that runs on this computer.
    async fn resolve(
        &self,
        session: Option<&DecisionSession>,
        wanted: Option<&DecisionModelRef>,
    ) -> DecisionResult<Arc<dyn DecisionModel>> {
        if let Some(wanted) = wanted {
            return self.source.open(wanted).await;
        }
        if let Some(chat_default) = chat_default_model(session)? {
            return self.source.open(&chat_default).await;
        }
        let provider = match self.configured() {
            Some(ConfiguredDefault::Model(model)) => return self.source.open(&model).await,
            Some(ConfiguredDefault::Provider(provider)) => Some(provider),
            None => None,
        };
        let entries = self.models().await;
        let (chosen, admitted, gate_reason) = self
            .gated_choice(session, &entries, provider.as_deref())
            .await;
        if let Some(entry) = chosen {
            return self.source.open(&entry.model_ref()).await;
        }
        let candidates = || {
            entries
                .iter()
                .filter(|entry| provider.as_deref().is_none_or(|p| entry.provider == p))
        };
        // A model the server will not load is not one a privacy rule ruled
        // out: with only such models listed there is no model.
        let listed = candidates().any(|entry| entry.load_refusal.is_none());
        let unloadable = candidates().find_map(|entry| entry.load_refusal.as_deref());
        // Models were listed, and the workspace's gate refused every one that
        // loads (a Crew chat with only hosted decision models): its refusal,
        // and how to get a model it admits.
        let gate_refused_all = candidates()
            .filter(|entry| entry.load_refusal.is_none())
            .all(|entry| !admitted.iter().any(|a| a.model_ref() == entry.model_ref()));
        if let (true, true, Some(reason)) = (listed, gate_refused_all, gate_reason) {
            // The gate admits a model of another provider: the configured
            // decision provider, not a missing install, is what rules it out.
            let elsewhere = provider
                .as_deref()
                .zip(automatic_choice(session, &admitted, None));
            let mut text = match elsewhere {
                Some((provider, other)) => format!(
                    "{reason} A decision that names no model uses the configured decision \
                     provider, `{provider}`, which has none this chat may use. Name `{}`, or \
                     clear BIOROUTER_DECISION_PROVIDER.",
                    other.model_ref().label()
                ),
                None => format!(
                    "{reason} No decision model on this computer is installed. {INSTALL_ADVICE}"
                ),
            };
            if let Some(unloadable) = unloadable {
                text = format!("{text} {unloadable}");
            }
            return Err(DecisionError::Refused(text));
        }
        let hint = self.source.unavailable_hint().await;
        Err(no_model_error(session, listed, unloadable, hint.as_deref()))
    }

    /// [`automatic_choice`] among the `entries` the workspace's gate admits
    /// for `session`. The first choice is asked about alone, and kept when the
    /// gate admits it, which is every choice in a chat no gate restricts.
    /// Otherwise every entry is asked about, and the choice is made among
    /// those admitted, which come back with the gate's first refusal.
    async fn gated_choice(
        &self,
        session: Option<&DecisionSession>,
        entries: &[DecisionModelEntry],
        provider: Option<&str>,
    ) -> (
        Option<DecisionModelEntry>,
        Vec<DecisionModelEntry>,
        Option<String>,
    ) {
        if let Some(first) = automatic_choice(session, entries, provider) {
            let refused = match session {
                Some(session) => self.use_gate.entry_refusal(session, first).await,
                None => None,
            };
            if refused.is_none() {
                return (Some(first.clone()), Vec::new(), None);
            }
        }
        let (admitted, gate_reason) = self.gate_admitted(session, entries).await;
        let chosen = automatic_choice(session, &admitted, provider).cloned();
        (chosen, admitted, gate_reason)
    }

    /// The listed `entries` the workspace's gate admits for `session` (all of
    /// them with no session), in the listing's order, and the first refusal
    /// it gave, if any.
    async fn gate_admitted(
        &self,
        session: Option<&DecisionSession>,
        entries: &[DecisionModelEntry],
    ) -> (Vec<DecisionModelEntry>, Option<String>) {
        let Some(session) = session else {
            return (entries.to_vec(), None);
        };
        let mut admitted = Vec::with_capacity(entries.len());
        let mut first_refusal = None;
        for entry in entries {
            match self.use_gate.entry_refusal(session, entry).await {
                None => admitted.push(entry.clone()),
                Some(reason) => {
                    first_refusal.get_or_insert(reason);
                }
            }
        }
        (admitted, first_refusal)
    }

    /// The model a decision for `session` naming `wanted` would use, opened by
    /// the same order [`Self::decide`] follows (named, the chat's default, the
    /// configured default, the automatic choice). Opening an Ollama model asks
    /// its server's `/api/show` with the model's name only; nothing of the
    /// chat is sent. For a caller that must name the model before it sends,
    /// and then pin it (`biorouter decide`), so every request of a run reads
    /// the same model.
    pub async fn resolve_for(
        &self,
        session: Option<&DecisionSession>,
        wanted: Option<&DecisionModelRef>,
    ) -> DecisionResult<Arc<dyn DecisionModel>> {
        self.resolve(session, wanted).await
    }

    /// Log a decision about `session` that never started because no model
    /// could be chosen: the row [`Self::decide`] writes when its own
    /// resolution fails. For a caller that resolves first with
    /// [`Self::resolve_for`] and reads the text after (`biorouter decide`),
    /// so its refused attempt reaches the chat's decision history as the
    /// route's does. Nothing was sent.
    pub async fn record_unresolved(
        &self,
        session: &DecisionSession,
        request: &DecisionRequest,
        error: &DecisionError,
    ) {
        let attempt = Attempt::single(Some(session), None);
        self.record_failure(
            Some(session),
            None,
            request,
            error,
            Some(&attempt),
            DecisionUsage::default(),
        )
        .await;
    }

    /// [`Self::record_unresolved`] for a batch: one row naming its chunks, as
    /// [`Self::decide_many`] writes when it cannot choose a model.
    pub async fn record_unresolved_batch(
        &self,
        session: &DecisionSession,
        questions: &IndexMap<String, Question>,
        chunks: &[DecisionChunk],
        error: &DecisionError,
    ) {
        let batch_id = uuid::Uuid::now_v7().to_string();
        self.record_batch_failure(Some(session), None, questions, chunks, &batch_id, error)
            .await;
    }

    /// The listing's entry for the model a decision for `session` naming
    /// `wanted` would use: that model, else the chat's default, else the
    /// configured default, else the automatic choice [`Self::decide`] would
    /// make. Read from the listing, so nothing is opened and nothing is sent.
    /// `None` when the listing does not show that model, or when the chat's
    /// settings cannot be read and nothing is named (a decision is then
    /// refused, see [`DecisionSession::chat_default_model`]).
    pub async fn model_entry_for(
        &self,
        session: Option<&DecisionSession>,
        wanted: Option<&DecisionModelRef>,
    ) -> Option<DecisionModelEntry> {
        let named = match wanted {
            Some(wanted) => Some(wanted.clone()),
            None => chat_default_model(session).ok()?,
        };
        let (named, provider) = match named {
            Some(named) => (Some(named), None),
            None => match self.configured() {
                Some(ConfiguredDefault::Model(model)) => (Some(model), None),
                Some(ConfiguredDefault::Provider(provider)) => (None, Some(provider)),
                None => (None, None),
            },
        };
        let entries = self.models().await;
        let entry = match &named {
            Some(named) => entries
                .iter()
                .find(|entry| entry.answers_to(named))
                .cloned(),
            None => {
                self.gated_choice(session, &entries, provider.as_deref())
                    .await
                    .0
            }
        };
        entry
    }

    /// The window, in tokens, of the model a decision for `session` naming
    /// `wanted` would use ([`Self::model_entry_for`]). `None` when the listing
    /// does not show it; the size check before every call still applies then.
    pub async fn context_tokens_for(
        &self,
        session: Option<&DecisionSession>,
        wanted: Option<&DecisionModelRef>,
    ) -> Option<usize> {
        self.model_entry_for(session, wanted)
            .await
            .map(|entry| entry.context_tokens)
    }

    /// `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL`, or the
    /// default this service was built with.
    fn configured(&self) -> Option<ConfiguredDefault> {
        match &self.default_model {
            DefaultModel::FromConfig => configured_default(),
            DefaultModel::Fixed(model) => model.clone().map(ConfiguredDefault::Model),
            #[cfg(test)]
            DefaultModel::Provider(provider) => Some(ConfiguredDefault::Provider(provider.clone())),
        }
    }

    fn check_kinds(
        &self,
        model: &dyn DecisionModel,
        caps: &DecisionCaps,
        questions: &IndexMap<String, Question>,
    ) -> DecisionResult<()> {
        if questions.len() > caps.max_questions {
            return Err(DecisionError::Unsupported(format!(
                "`{}` takes at most {} questions per request, not {}",
                model.display_name(),
                caps.max_questions,
                questions.len()
            )));
        }
        for (name, question) in questions {
            if !caps.kinds.contains(&question.kind.tag()) {
                return Err(DecisionError::Unsupported(format!(
                    "`{}` cannot answer question `{name}`: it does not take {:?} questions",
                    model.display_name(),
                    question.kind.tag()
                )));
            }
        }
        Ok(())
    }

    /// Gate H and affiliation, on the classification as it stands now, then
    /// the workspace's own check.
    async fn check_privacy(
        &self,
        session: &DecisionSession,
        model: &dyn DecisionModel,
        source: ModelSource,
    ) -> DecisionResult<()> {
        let mut effective = session.clone();
        if let Some(live) = &session.live {
            // Fail closed: a classification that cannot be read is Private.
            let now = live
                .current(&session.session_id)
                .await
                .unwrap_or(SessionClassification::Private);
            effective.classification = effective.classification.max(now);
        }
        privacy_allows(
            &effective,
            &model.display_name(),
            model.tier(),
            model.affiliation(),
            source,
        )?;
        self.use_gate.check(&effective, model).await
    }

    fn timeout_for(&self, model: &dyn DecisionModel, caps: &DecisionCaps) -> Duration {
        let configured = Config::global()
            .get_param::<u64>("BIOROUTER_DECISION_TIMEOUT")
            .ok()
            .filter(|&secs| secs > 0)
            .map(Duration::from_secs);
        let remote = self
            .remote_timeout
            .or(configured)
            .unwrap_or(DEFAULT_REMOTE_TIMEOUT);
        if !caps.local {
            return remote;
        }
        let warm = self
            .warm
            .lock()
            .map(|warm| warm.contains(&model.model_ref().label()))
            .unwrap_or(false);
        if warm {
            remote
        } else {
            self.local_first_timeout
                .unwrap_or(DEFAULT_LOCAL_FIRST_TIMEOUT)
                .max(remote)
        }
    }

    fn concurrency(&self, caps: &DecisionCaps) -> usize {
        Config::global()
            .get_param::<usize>("BIOROUTER_DECISION_MAX_CONCURRENT")
            .ok()
            .filter(|&n| n > 0)
            .unwrap_or(caps.max_concurrency)
            .max(1)
    }

    fn semaphore(&self, model: &dyn DecisionModel, caps: &DecisionCaps) -> Arc<Semaphore> {
        let key = model.model_ref().label();
        let permits = self.concurrency(caps);
        let mut limits = self
            .limits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limits
            .entry(key)
            .or_insert_with(|| Arc::new(Semaphore::new(permits)))
            .clone()
    }

    async fn decide_one(
        &self,
        model: &Arc<dyn DecisionModel>,
        request: &DecisionRequest,
        attempt: Attempt<'_>,
    ) -> DecisionResult<Vec<Decision>> {
        let caps = model.caps();
        // Minted before anything is sent, so every row this request writes,
        // a failure's and a late answer's included, is stored under one id.
        let request_id = attempt
            .request_id
            .map_or_else(|| uuid::Uuid::now_v7().to_string(), str::to_string);
        let attempt = Attempt {
            request_id: Some(&request_id),
            ..attempt
        };
        if let Err(error) = self
            .preflight(model.as_ref(), &caps, request, &attempt)
            .await
        {
            return Err(self.failed(model.as_ref(), request, &attempt, error).await);
        }
        let (verdicts, latency_ms) = self
            .call(model, &caps, request, &attempt, &request_id)
            .await?;
        let usage = verdicts.usage;
        let served_model = verdicts.served_model.clone();
        let decisions =
            match build_decisions(model.as_ref(), request, verdicts, latency_ms, &request_id) {
                Ok(decisions) => decisions,
                // The model answered, so the request was billed even though its
                // answer cannot be used: the failure row keeps its usage, and the
                // usage ledger counts it.
                Err(error) => {
                    let error = error.with_billed_usage(usage);
                    return Err(self.failed(model.as_ref(), request, &attempt, error).await);
                }
            };
        let session = attempt.session;
        let entry = DecisionLogEntry {
            session_id: session.map(|s| s.session_id.clone()),
            origin: session.map(|s| s.origin).unwrap_or_default(),
            origin_id: session.and_then(|s| s.origin_id.clone()),
            origin_label: session.and_then(|s| s.origin_label.clone()),
            turn_id: session.and_then(|s| s.turn_id.clone()),
            batch_id: attempt.batch_id.map(str::to_string),
            chunk_index: attempt.chunk_index,
            chunk_id: attempt.chunk_id.map(str::to_string),
            model: Some(model.model_ref()),
            served_model,
            tier: Some(model.tier()),
            questions: request.questions.clone(),
            context: request.context.clone(),
            context_bytes: request.context.byte_len(),
            outcome: outcome_of(&decisions),
            decisions: decisions.clone(),
            error: None,
            latency_ms: Some(latency_ms),
            usage,
            privacy_enforced: session.is_some_and(|s| s.enforced),
        };
        self.record(entry, Some(&request_id), 1).await;
        Ok(decisions)
    }

    /// What is checked before one request is sent: the kinds, privacy as it
    /// stands now, and the size.
    async fn preflight(
        &self,
        model: &dyn DecisionModel,
        caps: &DecisionCaps,
        request: &DecisionRequest,
        attempt: &Attempt<'_>,
    ) -> DecisionResult<()> {
        self.check_kinds(model, caps, &request.questions)?;
        if let Some(session) = attempt.session {
            self.check_privacy(session, model, attempt.source).await?;
        }
        check_size(model, caps, request, attempt.chunk_id).await
    }

    /// Ask `model` under the concurrency limit and its deadline, on a task of
    /// its own ([`spawn_call`]), and wait for the answer. A failure is logged
    /// ([`Self::failed`]) before it is returned. A timeout leaves the call
    /// running: its timeout row is written, and its answer, when it comes, is
    /// stored and billed as a late one under the same id. A local model that
    /// answered counts as warm from then on.
    async fn call(
        &self,
        model: &Arc<dyn DecisionModel>,
        caps: &DecisionCaps,
        request: &DecisionRequest,
        attempt: &Attempt<'_>,
        request_id: &str,
    ) -> DecisionResult<(ModelVerdicts, u64)> {
        let permit = self
            .semaphore(model.as_ref(), caps)
            .acquire_owned()
            .await
            .map_err(|_| {
                DecisionError::NoModel("The decision service is shutting down.".to_string())
            })?;
        let deadline = self.timeout_for(model.as_ref(), caps);
        let late = self.late_answer(model.as_ref(), request, attempt, request_id, deadline);
        let mut waiter = spawn_call(Arc::clone(model), late);
        let waited = tokio::time::timeout(deadline, &mut waiter.receiver).await;
        // The permit bounds the requests being waited for, never a call left
        // running after its deadline.
        drop(permit);
        let error = match waited {
            Ok(Ok(Answered {
                answered: Ok(verdicts),
                latency_ms,
                ..
            })) => {
                if caps.local {
                    if let Ok(mut warm) = self.warm.lock() {
                        warm.insert(model.model_ref().label());
                    }
                }
                return Ok((verdicts, latency_ms));
            }
            Ok(Ok(Answered {
                answered: Err(error),
                ..
            })) => error,
            // The call's task ended without an answer: it panicked.
            Ok(Err(_)) => DecisionError::Backend {
                model: model.display_name(),
                status: None,
                error_type: None,
                message: "the decision request stopped unexpectedly".to_string(),
            },
            Err(_) => {
                let timeout = DecisionError::Timeout {
                    model: model.display_name(),
                    seconds: deadline.as_secs(),
                };
                let error = self.failed(model.as_ref(), request, attempt, timeout).await;
                waiter.give_up();
                return Err(error);
            }
        };
        Err(self.failed(model.as_ref(), request, attempt, error).await)
    }

    /// Everything a late or abandoned answer to this request needs to be
    /// stored and billed without the caller: who asked, what was asked, the
    /// deadline, and the sink as it is now.
    fn late_answer(
        &self,
        model: &dyn DecisionModel,
        request: &DecisionRequest,
        attempt: &Attempt<'_>,
        request_id: &str,
        deadline: Duration,
    ) -> LateAnswer {
        let session = attempt.session;
        LateAnswer {
            id: request_id.to_string(),
            session_id: session.map(|s| s.session_id.clone()),
            origin: session.map(|s| s.origin).unwrap_or_default(),
            origin_id: session.and_then(|s| s.origin_id.clone()),
            origin_label: session.and_then(|s| s.origin_label.clone()),
            turn_id: session.and_then(|s| s.turn_id.clone()),
            batch_id: attempt.batch_id.map(str::to_string),
            chunk_index: attempt.chunk_index,
            chunk_id: attempt.chunk_id.map(str::to_string),
            model_ref: model.model_ref(),
            tier: model.tier(),
            display_name: model.display_name(),
            request: Arc::new(request.clone()),
            privacy_enforced: session.is_some_and(|s| s.enforced),
            deadline,
            asked_at: Instant::now(),
            sink: self.sink.read().ok().and_then(|sink| sink.clone()),
        }
    }

    /// Log one request's failure and hand the error back. Inside a batch a
    /// refusal is left to the batch, which logs one row for every chunk it
    /// stopped. The row keeps what the model was billed for when it answered
    /// before the request failed ([`DecisionError::billed_usage`]).
    async fn failed(
        &self,
        model: &dyn DecisionModel,
        request: &DecisionRequest,
        attempt: &Attempt<'_>,
        error: DecisionError,
    ) -> DecisionError {
        if !(attempt.defer_refusal && matches!(error, DecisionError::Refused(_))) {
            self.record_failure(
                attempt.session,
                Some(model),
                request,
                &error,
                Some(attempt),
                error.billed_usage(),
            )
            .await;
        }
        error
    }

    /// A batch stopped before any chunk was sent (no model, a kind the model
    /// cannot take, privacy): ONE row for the whole batch, never one per
    /// chunk. Its context names the chunks rather than holding them, because
    /// none was sent.
    async fn record_batch_failure(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&dyn DecisionModel>,
        questions: &IndexMap<String, Question>,
        chunks: &[DecisionChunk],
        batch_id: &str,
        error: &DecisionError,
    ) {
        let request = DecisionRequest {
            questions: questions.clone(),
            context: DecisionContext::Text(unsent_batch_text(chunks)),
        };
        let attempt = Attempt {
            session,
            batch_id: Some(batch_id),
            stopped_chunks: chunks.len(),
            ..Attempt::default()
        };
        self.record_failure(
            session,
            model,
            &request,
            error,
            Some(&attempt),
            DecisionUsage::default(),
        )
        .await;
    }

    /// One failure row. `usage` is the default unless the model answered
    /// before the request failed; a billed answer keeps its usage, so the
    /// usage ledger counts it under the row's id.
    async fn record_failure(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&dyn DecisionModel>,
        request: &DecisionRequest,
        error: &DecisionError,
        attempt: Option<&Attempt<'_>>,
        usage: DecisionUsage,
    ) {
        let outcome = if matches!(error, DecisionError::Refused(_)) {
            DecisionOutcome::Refused
        } else {
            DecisionOutcome::Error
        };
        let entry = DecisionLogEntry {
            session_id: session.map(|s| s.session_id.clone()),
            origin: session.map(|s| s.origin).unwrap_or_default(),
            origin_id: session.and_then(|s| s.origin_id.clone()),
            origin_label: session.and_then(|s| s.origin_label.clone()),
            turn_id: session.and_then(|s| s.turn_id.clone()),
            batch_id: attempt.and_then(|a| a.batch_id.map(str::to_string)),
            chunk_index: attempt.and_then(|a| a.chunk_index),
            chunk_id: attempt.and_then(|a| a.chunk_id.map(str::to_string)),
            model: model.map(|m| m.model_ref()),
            served_model: None,
            tier: model.map(|m| m.tier()),
            questions: request.questions.clone(),
            context: request.context.clone(),
            context_bytes: request.context.byte_len(),
            outcome,
            decisions: Vec::new(),
            error: Some(error.to_string()),
            latency_ms: None,
            usage,
            privacy_enforced: session.is_some_and(|s| s.enforced),
        };
        let chunks = attempt.map_or(1, |a| a.stopped_chunks);
        self.record(entry, attempt.and_then(|a| a.request_id), chunks)
            .await;
    }

    /// Hand `entry` to the sink, under `id` when the caller chose one.
    /// `chunks` is how many chunks the row stands for: one, or every chunk of
    /// a batch the row records as stopped.
    async fn record(&self, entry: DecisionLogEntry, id: Option<&str>, chunks: usize) {
        // Counted per turn and origin in process, for the one transcript
        // marker a turn gets; never one row per call (owner rule A1).
        super::monitor::note_decision_chunks(&entry, chunks.max(1) as u64);
        let sink = self.sink.read().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            let written = match id {
                Some(id) => sink.record_as(id, entry).await,
                None => sink.record(entry).await,
            };
            if let Err(error) = written {
                tracing::warn!("decision log write failed: {error:#}");
            }
        }
    }
}

/// The save-time checks: whether what a setting turns on could run, asked
/// without sending anything of the chat.
impl DecisionService {
    /// Whether a decision about `session` asking `questions` of the model
    /// `wanted` names (or, with none, of the model a decision would choose)
    /// could run now, by every check a decision makes before it sends: the
    /// chat's Crew grant, the model's resolution (named, the chat's default,
    /// the configured default, the automatic choice), the chat's bound model
    /// ([`DecisionSession::bound_tier`]), Gate H and affiliation with the
    /// workspace's gate, the question kinds, and the server's own refusal to
    /// load the model. Opening an Ollama model asks its server's `/api/show`
    /// with the model's name only; nothing of the chat is sent.
    pub async fn check_model_use(
        &self,
        session: &DecisionSession,
        wanted: Option<&DecisionModelRef>,
        questions: &IndexMap<String, Question>,
        source: ModelSource,
    ) -> DecisionResult<()> {
        self.check_model_for(session, wanted, &[questions], source)
            .await?
            .pop()
            .unwrap_or(Ok(()))
    }

    /// [`Self::check_model_use`]'s model-wide checks once, then the kinds of
    /// each question set: the outer `Err` is the model's, each inner result
    /// one set's. The server's refusal to load the model is model-wide, so it
    /// fails a model asked with no question set at all (a changed default
    /// that no monitor runs on).
    async fn check_model_for(
        &self,
        session: &DecisionSession,
        wanted: Option<&DecisionModelRef>,
        question_sets: &[&IndexMap<String, Question>],
        source: ModelSource,
    ) -> DecisionResult<Vec<DecisionResult<()>>> {
        if let Some(reason) = self.session_refusal(session).await {
            return Err(DecisionError::Refused(reason));
        }
        let model = self.resolve(Some(session), wanted).await?;
        refuse_by_bound_tier(session, model.as_ref(), source)?;
        self.check_privacy(session, model.as_ref(), source).await?;
        let model_ref = model.model_ref();
        let load_refusal = self
            .models()
            .await
            .into_iter()
            .find(|entry| entry.answers_to(&model_ref))
            .and_then(|entry| entry.load_refusal);
        if let Some(refusal) = load_refusal {
            return Err(DecisionError::Unsupported(refusal));
        }
        let caps = model.caps();
        Ok(question_sets
            .iter()
            .map(|questions| self.check_kinds(model.as_ref(), &caps, questions))
            .collect())
    }

    /// Whether the chat `session` could run what saving `settings` over
    /// `stored` turns on. Only what the save changes is asked: the chat's
    /// default decision model when it changes to one, and every enabled
    /// monitor that is new, was off, asks another kind of question or would
    /// run on another model (its own, else the chat's default). A monitor left
    /// as it was, or switched off, is never asked, so a monitor whose model
    /// went away can always be switched off or removed. Each model is asked
    /// once, all at once. `Err` is the sentence the person reads, naming the
    /// first monitor in list order that could not run (or the default
    /// model, when that is what could not).
    pub async fn check_settings(
        &self,
        session: &DecisionSession,
        settings: &DecisionSettings,
        stored: Option<&DecisionSettings>,
    ) -> Result<(), String> {
        let session = session
            .clone()
            .with_chat_default_model(Ok(settings.default_model.clone()));
        let stored_default = stored.and_then(|stored| stored.default_model.as_ref());
        let default = settings
            .default_model
            .as_ref()
            .filter(|default| Some(*default) != stored_default);
        let monitors: Vec<&MonitorCriterion> = settings
            .monitors
            .iter()
            .filter(|monitor| monitor.enabled && turned_on(monitor, settings, stored))
            .collect();
        if default.is_none() && monitors.is_empty() {
            return Ok(());
        }
        if let Some(reason) = self.session_refusal(&session).await {
            return Err(format!("Nothing was saved. {reason}"));
        }
        // Each monitor's question as it would be asked, grouped by the model
        // it would run on.
        let mut groups: MonitorsByModel = IndexMap::new();
        if let Some(default) = default {
            groups.entry(Some(default.clone())).or_default();
        }
        for (index, monitor) in monitors.iter().enumerate() {
            let question = monitor.question_with_threshold().map_err(|why| {
                format!(
                    "Nothing was saved: monitor \"{}\" could not run. {}",
                    monitor.name,
                    as_sentence(&why)
                )
            })?;
            let model = monitor
                .model
                .clone()
                .or_else(|| settings.default_model.clone());
            groups
                .entry(model)
                .or_default()
                .push((index, IndexMap::from([(monitor.id.clone(), question)])));
        }
        let checks = groups.iter().map(|(model, sets)| {
            let sets: Vec<&IndexMap<String, Question>> =
                sets.iter().map(|(_, questions)| questions).collect();
            let session = &session;
            async move {
                self.check_model_for(session, model.as_ref(), &sets, ModelSource::Setting)
                    .await
            }
        });
        let results = futures::future::join_all(checks).await;
        let mut failures: Vec<Option<String>> = vec![None; monitors.len()];
        for ((model, sets), result) in groups.iter().zip(results) {
            match result {
                Err(error) => {
                    if let (Some(default), Some(model)) = (default, model) {
                        if default == model {
                            return Err(format!(
                                "Nothing was saved: the chat's default decision model {} could \
                                 not run. {}",
                                default.label(),
                                as_sentence(&error.to_string())
                            ));
                        }
                    }
                    for (index, _) in sets {
                        failures[*index] = Some(error.to_string());
                    }
                }
                Ok(each) => {
                    for ((index, _), result) in sets.iter().zip(each) {
                        if let Err(error) = result {
                            failures[*index] = Some(error.to_string());
                        }
                    }
                }
            }
        }
        match monitors
            .iter()
            .zip(failures)
            .find_map(|(m, f)| f.map(|f| (m, f)))
        {
            Some((monitor, cause)) => Err(format!(
                "Nothing was saved: monitor \"{}\" could not run. {}",
                monitor.name,
                as_sentence(&cause)
            )),
            None => Ok(()),
        }
    }

    /// Whether a decision chat bound to `model` could answer `question`, when
    /// saving it over `stored` changes what is asked: a chat bound to a
    /// decision model (`model` is `Some`) whose question is new or of another
    /// kind. Anything else is not asked. `Err` is the sentence the person
    /// reads.
    pub async fn check_chat_question(
        &self,
        session: &DecisionSession,
        model: Option<&DecisionModelRef>,
        question: &Question,
        stored: Option<&Question>,
    ) -> Result<(), String> {
        let Some(model) = model else {
            return Ok(());
        };
        if stored.is_some_and(|stored| stored.kind == question.kind) {
            return Ok(());
        }
        if let Some(reason) = self.session_refusal(session).await {
            return Err(format!("Nothing was saved. {reason}"));
        }
        let questions = IndexMap::from([(
            crate::providers::decision::chat::CHAT_QUESTION_NAME.to_string(),
            question.clone(),
        )]);
        self.check_model_use(session, Some(model), &questions, ModelSource::Setting)
            .await
            .map_err(|error| {
                format!(
                    "Nothing was saved: this decision chat's model {} could not answer this \
                     question. {}",
                    model.label(),
                    as_sentence(&error.to_string())
                )
            })
    }
}

/// The monitors a save asks about, by the model each would run on (`None`:
/// the automatic choice): each one's place among them and its question.
type MonitorsByModel = IndexMap<Option<DecisionModelRef>, Vec<(usize, IndexMap<String, Question>)>>;

/// Whether saving `settings` over `stored` turns `monitor` on in a way that
/// has not been asked before: it is new, it was off, its question is of
/// another kind, or it would run on another model (its own, else the chat's
/// default) than the stored monitor would.
fn turned_on(
    monitor: &MonitorCriterion,
    settings: &DecisionSettings,
    stored: Option<&DecisionSettings>,
) -> bool {
    let Some(stored) = stored else {
        return true;
    };
    let Some(before) = stored.monitors.iter().find(|m| m.id == monitor.id) else {
        return true;
    };
    let runs_on = monitor.model.as_ref().or(settings.default_model.as_ref());
    let ran_on = before.model.as_ref().or(stored.default_model.as_ref());
    !before.enabled || before.question.kind != monitor.question.kind || runs_on != ran_on
}

/// `text` as a sentence: with a full stop added when it ends without one.
fn as_sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(['.', '!', '?']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// The save-time rule for a chat bound to a private model while it is still
/// Public: its first turn makes it private, and then a public decision model
/// is refused at every call, so it is refused now. Capability against
/// capability, under the master switch as the caller sampled it.
fn refuse_by_bound_tier(
    session: &DecisionSession,
    model: &dyn DecisionModel,
    source: ModelSource,
) -> DecisionResult<()> {
    if session.enforced
        && session.bound_tier == Some(ProviderTier::Private)
        && !session.classification.is_private()
        && model.tier() == ProviderTier::Public
    {
        return Err(DecisionError::Refused(format!(
            "This chat runs on a private model, so it becomes private at its first turn, and \
             `{}` is a public model a private chat cannot use. {}.",
            model.display_name(),
            source.advice_text()
        )));
    }
    Ok(())
}

/// One call's answer, its latency in milliseconds, and the request's place in
/// the in-flight count. The guard travels with the answer, so the request is
/// counted in flight from the moment it is sent until its answer is read by
/// the caller or stored as a late one, with no gap between the two.
struct Answered {
    answered: DecisionResult<ModelVerdicts>,
    latency_ms: u64,
    in_flight: super::late::InFlightGuard,
}

impl Answered {
    /// Store and bill this answer as a late one ([`LateAnswer::settle`]),
    /// counted in flight until that is done.
    async fn settle(self, late: LateAnswer) {
        let Answered {
            answered,
            latency_ms,
            in_flight,
        } = self;
        late.settle(answered, latency_ms).await;
        drop(in_flight);
    }
}

/// Ask `model` on a task of its own, counted in flight, and hand its answer
/// and latency back to the returned waiter. When the waiter has stopped
/// waiting before the answer comes (it gave up at its deadline, or went
/// away), the send fails and the task hands the answer to `late` itself. It
/// never holds the concurrency permit.
fn spawn_call(model: Arc<dyn DecisionModel>, late: LateAnswer) -> AnswerWaiter {
    let (sender, receiver) = oneshot::channel::<Answered>();
    let in_flight = super::late::enter_in_flight();
    let task_late = late.clone();
    tokio::spawn(async move {
        let request = Arc::clone(&task_late.request);
        let started = Instant::now();
        let answered = model.decide(&request).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let answer = Answered {
            answered,
            latency_ms,
            in_flight,
        };
        if let Err(answer) = sender.send(answer) {
            answer.settle(task_late).await;
        }
    });
    AnswerWaiter { receiver, late }
}

/// The waiting end of one call ([`spawn_call`]). Exactly one of it and the
/// call's task hands a late answer over: when the waiter stops waiting, it
/// closes the channel, an answer already sent is handed over here, and one
/// that comes later fails its send and is handed over by the task.
struct AnswerWaiter {
    receiver: oneshot::Receiver<Answered>,
    late: LateAnswer,
}

impl AnswerWaiter {
    /// Stop waiting: close the channel, and hand an answer already sent over
    /// on a task of its own, still counted in flight. It never awaits, so a
    /// caller that is itself stopped right after (a steer, Stop, an HTTP
    /// client that gave up) cannot take the answer's storing down with it.
    /// Called at the deadline, after the timeout row is written, and again
    /// on drop, where it finds the channel empty if it already ran.
    fn give_up(&mut self) {
        self.receiver.close();
        let Ok(answer) = self.receiver.try_recv() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let late = self.late.clone();
        runtime.spawn(answer.settle(late));
    }
}

impl Drop for AnswerWaiter {
    /// A waiter that goes away (a stopped turn, a dropped request) at any
    /// point after the answer was sent, the timeout row's write included,
    /// leaves the answer in the channel, where no one else would read it: it
    /// is handed over as [`AnswerWaiter::give_up`] does. A waiter that read
    /// its answer, or gave up, finds the channel empty.
    fn drop(&mut self) {
        self.give_up();
    }
}

/// One request's answer when nobody is waiting for it any more: what it needs
/// to be stored and billed, owned, so the call's task can do it alone.
#[derive(Clone)]
struct LateAnswer {
    id: String,
    session_id: Option<String>,
    origin: DecisionOrigin,
    origin_id: Option<String>,
    origin_label: Option<String>,
    turn_id: Option<String>,
    batch_id: Option<String>,
    chunk_index: Option<usize>,
    chunk_id: Option<String>,
    model_ref: DecisionModelRef,
    tier: ProviderTier,
    display_name: String,
    request: Arc<DecisionRequest>,
    privacy_enforced: bool,
    deadline: Duration,
    asked_at: Instant,
    sink: Option<Arc<dyn DecisionSink>>,
}

impl LateAnswer {
    /// Store and bill `answered`, which came back `latency_ms` after it was
    /// sent: an error row carrying its tokens and a sentence saying why it was
    /// not used, handed to [`DecisionSink::record_late`], and a priced answer's
    /// cost against the live turn. Nothing when it reported no usage or
    /// belongs to no chat. Never counted in the turn's tally.
    async fn settle(self, answered: DecisionResult<ModelVerdicts>, latency_ms: u64) {
        let (usage, served_model) = match &answered {
            Ok(verdicts) => (verdicts.usage, verdicts.served_model.clone()),
            Err(error) => (error.billed_usage(), None),
        };
        if usage.billed_total().is_none() {
            return;
        }
        let Some(session_id) = self.session_id.clone() else {
            return;
        };
        let seconds = seconds_text(latency_ms);
        let error = if self.asked_at.elapsed() >= self.deadline {
            let timeout = DecisionError::Timeout {
                model: self.display_name.clone(),
                seconds: self.deadline.as_secs(),
            };
            format!(
                "{timeout} It answered after {seconds} s, too late to use; its tokens are \
                 counted in Usage."
            )
        } else {
            format!(
                "The request was abandoned before {} answered (the turn was stopped or the \
                 caller went away), so its answer was not used. It answered after {seconds} s; \
                 its tokens are counted in Usage.",
                self.display_name
            )
        };
        if let Some(turn_id) = &self.turn_id {
            let tokens = |count: Option<i64>| u64::try_from(count.unwrap_or(0)).unwrap_or(0);
            let cost = crate::providers::pricing::estimate_cost_usd(
                &self.model_ref.provider,
                &self.model_ref.model,
                tokens(usage.input_tokens),
                tokens(usage.output_tokens),
            )
            .filter(|cost| cost.is_finite() && *cost > 0.0);
            if let Some(cost) = cost {
                super::turns::note_late_cost(&session_id, turn_id, cost);
            }
        }
        let entry = DecisionLogEntry {
            session_id: Some(session_id),
            origin: self.origin,
            origin_id: self.origin_id,
            origin_label: self.origin_label,
            turn_id: self.turn_id,
            batch_id: self.batch_id,
            chunk_index: self.chunk_index,
            chunk_id: self.chunk_id,
            model: Some(self.model_ref),
            served_model,
            tier: Some(self.tier),
            questions: self.request.questions.clone(),
            context: self.request.context.clone(),
            context_bytes: self.request.context.byte_len(),
            outcome: DecisionOutcome::Error,
            decisions: Vec::new(),
            error: Some(error),
            latency_ms: Some(latency_ms),
            usage,
            privacy_enforced: self.privacy_enforced,
        };
        if let Some(sink) = &self.sink {
            if let Err(error) = sink.record_late(&self.id, entry).await {
                tracing::warn!("a late decision answer could not be logged: {error:#}");
            }
        }
    }
}

/// A latency as seconds: one decimal under ten seconds, whole ones above.
fn seconds_text(latency_ms: u64) -> String {
    if latency_ms >= 10_000 {
        (latency_ms / 1_000).to_string()
    } else {
        format!("{:.1}", latency_ms as f64 / 1_000.0)
    }
}

/// Gate H's downward rule on a declared tier, then affiliation.
fn privacy_allows(
    session: &DecisionSession,
    name: &str,
    tier: ProviderTier,
    affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
    source: ModelSource,
) -> DecisionResult<()> {
    assert_alt_tier_allowed(
        "this decision",
        name,
        tier,
        session.classification,
        session.enforced,
        source.advice(),
    )
    .map_err(|e| DecisionError::Refused(e.to_string()))?;
    if session.enforced && !owners_compatible(affiliation, &session.affiliations) {
        return Err(DecisionError::Refused(format!(
            "This chat holds private data from an institution `{name}` is not covered by, so \
             this decision cannot run on it. Choose a decision model that runs on this computer, \
             or one that institution covers."
        )));
    }
    Ok(())
}

/// The first listed model `session` may use, among `provider`'s when one is
/// configured, skipping one its server will not load
/// ([`DecisionModelEntry::load_refusal`]): choosing it would fail every call
/// that names no model. `entries` are ranked local first.
fn automatic_choice<'a>(
    session: Option<&DecisionSession>,
    entries: &'a [DecisionModelEntry],
    provider: Option<&str>,
) -> Option<&'a DecisionModelEntry> {
    entries
        .iter()
        .filter(|entry| provider.is_none_or(|provider| entry.provider == provider))
        .filter(|entry| entry.load_refusal.is_none())
        .find(|entry| {
            session.is_none_or(|session| {
                privacy_allows(
                    session,
                    &entry.display_name,
                    entry.tier,
                    entry.affiliation,
                    ModelSource::Configured,
                )
                .is_ok()
            })
        })
}

/// Why the automatic choice found nothing. `listed` says whether any model
/// its server loads (of the configured provider, when one is set) was listed:
/// then every one was ruled out for this chat by a privacy rule, which is a
/// refusal, as it is for a model the call names. With nothing listed it is no
/// model at all. `unloadable` is the sentence for a listed model the server
/// will not load, which names its fix, so it is said too.
///
/// `hint` is the source's reason models may be missing from the listing
/// ([`DecisionModelSource::unavailable_hint`]: a local server that is not
/// answering), said in place of the install advice, which would be wrong for
/// a model that is already installed.
fn no_model_error(
    session: Option<&DecisionSession>,
    listed: bool,
    unloadable: Option<&str>,
    hint: Option<&str>,
) -> DecisionError {
    let private = session.is_some_and(|s| s.enforced && s.classification.is_private());
    let fix = hint.unwrap_or(INSTALL_ADVICE);
    let text = if private {
        format!(
            "No decision model this private chat may use is available: only a decision model on \
             this computer may read a private chat. {fix} Or set BIOROUTER_DECISION_MODEL to a \
             private one."
        )
    } else if listed && session.is_some() {
        format!(
            "No listed decision model may read this chat: a privacy rule rules out every one. \
             Set BIOROUTER_DECISION_MODEL to one this chat may use. {fix}"
        )
    } else {
        format!(
            "No decision model is available. Add a Baseten API key or set \
             BIOROUTER_DECISION_MODEL. {fix}"
        )
    };
    let text = match unloadable {
        Some(unloadable) => format!("{text} {unloadable}"),
        None => text,
    };
    if listed && session.is_some() {
        DecisionError::Refused(text)
    } else {
        DecisionError::NoModel(text)
    }
}

/// One request's decisions, in the questions' order, from what the model
/// returned. Every decision of the request shares one request id, which is
/// also the decision log's id for the row that records the request.
fn build_decisions(
    model: &dyn DecisionModel,
    request: &DecisionRequest,
    verdicts: ModelVerdicts,
    latency_ms: u64,
    request_id: &str,
) -> DecisionResult<Vec<Decision>> {
    let model_ref = model.model_ref();
    let meta = DecisionMeta {
        model: &model_ref,
        tier: model.tier(),
        request_id,
        latency_ms,
        usage: verdicts.usage,
    };
    let mut by_name: HashMap<String, _> = verdicts
        .verdicts
        .into_iter()
        .map(|verdict| (verdict.question.clone(), verdict))
        .collect();
    request
        .questions
        .iter()
        .map(|(name, question)| {
            by_name
                .remove(name)
                .ok_or_else(|| format!("no answer for question `{name}`"))
                .and_then(|verdict| decision_from_verdict(name, question, verdict, &meta))
                .map_err(|detail| DecisionError::malformed(&model.display_name(), detail))
        })
        .collect()
}

/// A request's outcome: failed when any thresholded question failed, passed
/// when one passed and none failed, else decided.
fn outcome_of(decisions: &[Decision]) -> DecisionOutcome {
    if decisions.iter().any(|d| d.passed == Some(false)) {
        DecisionOutcome::Failed
    } else if decisions.iter().any(|d| d.passed == Some(true)) {
        DecisionOutcome::Passed
    } else {
        DecisionOutcome::Decided
    }
}

/// Chunk ids a batch-failure row names before it says how many more there were.
const UNSENT_IDS_SHOWN: usize = 20;

/// What a batch-failure row holds in place of a context: how many chunks, and
/// which, since none was sent.
fn unsent_batch_text(chunks: &[DecisionChunk]) -> String {
    let ids: Vec<&str> = chunks.iter().map(|chunk| chunk.id.as_str()).collect();
    format!(
        "Not sent: the batch of {} chunk{} was stopped before any chunk reached a decision \
         model. Chunks: {}.",
        chunks.len(),
        if chunks.len() == 1 { "" } else { "s" },
        shown_ids(&ids)
    )
}

/// What the row for a batch stopped partway holds: how many of its chunks
/// were held back, and which. None of them was sent.
fn unsent_partway_text(total: usize, ids: &[&str]) -> String {
    format!(
        "Not sent: {} of {total} chunk{} {} stopped partway through the batch, when a privacy \
         or Crew rule refused this chat's content to the decision model. Chunks: {}.",
        ids.len(),
        if total == 1 { "" } else { "s" },
        if ids.len() == 1 { "was" } else { "were" },
        shown_ids(ids)
    )
}

/// The first [`UNSENT_IDS_SHOWN`] ids, then how many more there were.
fn shown_ids(ids: &[&str]) -> String {
    let mut shown: Vec<&str> = ids.iter().take(UNSENT_IDS_SHOWN).copied().collect();
    let more = ids.len().saturating_sub(UNSENT_IDS_SHOWN);
    let more = (more > 0).then(|| format!("and {more} more"));
    shown.extend(more.as_deref());
    shown.join(", ")
}

/// The shared o200k counter once a size check (or [`load_token_counter`])
/// has loaded it, so a window can be fitted by the check's own count without
/// waiting ([`estimate_tokens_now`]).
static LOADED_COUNTER: std::sync::OnceLock<Arc<crate::token_counter::TokenCounter>> =
    std::sync::OnceLock::new();

/// The token estimate for a request: the state and every question's text,
/// through the shared o200k counter with the non-OpenAI margin.
pub async fn estimate_tokens(request: &DecisionRequest) -> usize {
    let text = request_text(request);
    let raw = match crate::token_counter::shared_token_counter().await {
        Ok(counter) => LOADED_COUNTER.get_or_init(|| counter).count_tokens(&text),
        // The tokenizer failed to load: the four-characters-a-token estimate
        // the context budget uses, which errs low, so the margin below matters.
        Err(_) => crate::context_budget::estimate_tokens(&text),
    } + extra_digit_tokens(&text);
    with_margin(raw, request.questions.len())
}

/// [`estimate_tokens`] without waiting, for fitting a state while a lock is
/// held: the same count once the shared tokenizer is loaded, and before that
/// three characters a token, which counts more than the four the check falls
/// back to, with the same per-digit charge, margin and overhead.
pub(crate) fn estimate_tokens_now(request: &DecisionRequest) -> usize {
    let text = request_text(request);
    let raw = match LOADED_COUNTER.get() {
        Some(counter) => counter.count_tokens(&text),
        None => text.chars().count().div_ceil(3),
    } + extra_digit_tokens(&text);
    with_margin(raw, request.questions.len())
}

/// Load the shared tokenizer for [`estimate_tokens_now`]. A tokenizer that
/// cannot load leaves the three-characters-a-token count in place.
pub(crate) async fn load_token_counter() {
    if let Ok(counter) = crate::token_counter::shared_token_counter().await {
        LOADED_COUNTER.get_or_init(|| counter);
    }
}

/// The text [`estimate_tokens`] counts: the state, then every question.
fn request_text(request: &DecisionRequest) -> String {
    let mut text = request.context.render_text();
    for (name, question) in &request.questions {
        push_question_text(&mut text, name, question);
    }
    text
}

/// A raw count with the non-OpenAI factor and the per-question overhead.
fn with_margin(raw: usize, questions: usize) -> usize {
    ((raw as f64) * TOKEN_ESTIMATE_MARGIN).ceil() as usize + PER_QUESTION_OVERHEAD * questions
}

/// A question's text as [`estimate_tokens`] counts it: its name, its
/// instructions and its options or levels, one to a line.
fn push_question_text(text: &mut String, name: &str, question: &Question) {
    text.push('\n');
    text.push_str(name);
    text.push('\n');
    text.push_str(&question.instructions);
    match &question.kind {
        DecisionKind::Binary {} => {}
        DecisionKind::Choice { options } => {
            for (option, description) in options {
                text.push('\n');
                text.push_str(option);
                if let Some(description) = description {
                    text.push_str(": ");
                    text.push_str(description);
                }
            }
        }
        DecisionKind::Score { levels } => {
            for level in levels {
                text.push('\n');
                text.push_str(level);
            }
        }
    }
}

/// The tokens a per-digit tokenizer spends on `text`'s numbers beyond what
/// o200k counts. o200k folds up to three digits into one token, while the
/// Qwen-family tokenizers decision models use give every digit its own, so
/// the flat margin alone under-counted lab values: live on 2026-11-05, a labs
/// note this estimate put at joey-1's window was refused by Baseten as over
/// it (HTTP 422), while prose at the same estimate was well inside. A window
/// fitted to a model ([`state_budget_tokens`], the monitor's fit) charges
/// digits by this same count, so a fitted window passes the check.
pub(crate) fn extra_digit_tokens(text: &str) -> usize {
    let mut extra = 0;
    let mut run = 0usize;
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() {
            run += 1;
        } else if run > 0 {
            extra += run - run.div_ceil(3);
            run = 0;
        }
    }
    extra
}

/// Room kept below a model's window for what the estimate cannot see: the
/// server's own framing and the answer positions.
pub fn context_margin(context_tokens: usize) -> usize {
    (context_tokens / 32).max(128)
}

/// The tokens a state may take beside `questions` in a model of
/// `context_tokens`, by the arithmetic the size check before sending applies
/// (its margin, its per-question overhead and its non-OpenAI factor), with
/// the questions' own text counted at three characters a token plus the
/// check's per-digit charge ([`extra_digit_tokens`]). For building a state
/// that fits, such as a monitor's window; the check still judges.
pub fn state_budget_tokens(context_tokens: usize, questions: &IndexMap<String, Question>) -> usize {
    let overhead = context_margin(context_tokens) + PER_QUESTION_OVERHEAD * questions.len();
    let estimated_room = context_tokens.saturating_sub(overhead) as f64 / TOKEN_ESTIMATE_MARGIN;
    let question_tokens: usize = questions
        .iter()
        .map(|(name, question)| {
            let mut text = String::new();
            push_question_text(&mut text, name, question);
            text.len().div_ceil(3) + extra_digit_tokens(&text)
        })
        .sum();
    (estimated_room.floor() as usize).saturating_sub(question_tokens)
}

async fn check_size(
    model: &dyn DecisionModel,
    caps: &DecisionCaps,
    request: &DecisionRequest,
    chunk_id: Option<&str>,
) -> DecisionResult<()> {
    let chunk = match chunk_id {
        Some(id) => format!("Chunk `{id}`"),
        None => "The state".to_string(),
    };
    if let Some(limit) = caps.max_request_bytes {
        let bytes = request.context.byte_len()
            + request
                .questions
                .values()
                .map(|q| q.instructions.len() + 64)
                .sum::<usize>();
        if bytes > limit {
            return Err(DecisionError::Invalid(format!(
                "{chunk} is about {bytes} bytes, over the {limit}-byte request limit of `{}`. \
                 Split it into smaller pieces and ask again.",
                model.display_name()
            )));
        }
    }
    let tokens = estimate_tokens(request).await;
    let limit = caps.context_tokens;
    if tokens + context_margin(limit) > limit {
        return Err(DecisionError::TooLarge {
            chunk,
            tokens,
            limit,
            model: format!("`{}`", model.display_name()),
        });
    }
    Ok(())
}

enum ConfiguredDefault {
    Model(DecisionModelRef),
    Provider(String),
}

/// `BIOROUTER_DECISION_PROVIDER` and `BIOROUTER_DECISION_MODEL`. The model may
/// be written `provider/model` with no provider key.
fn configured_default() -> Option<ConfiguredDefault> {
    let config = Config::global();
    let read = |key: &str| {
        config
            .get_param::<String>(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let provider = read("BIOROUTER_DECISION_PROVIDER");
    let model = read("BIOROUTER_DECISION_MODEL");
    match (provider, model) {
        (Some(provider), Some(model)) => Some(ConfiguredDefault::Model(DecisionModelRef::new(
            provider, model,
        ))),
        (None, Some(model)) => DecisionModelRef::parse(&model).map(ConfiguredDefault::Model),
        (Some(provider), None) => Some(ConfiguredDefault::Provider(provider)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::model::{DecisionModelEntry, LOCAL_CONCURRENCY};
    use crate::decision::types::{
        Answer, DecisionAdapter, DecisionKindTag, ModelVerdicts, ProbabilitySource,
        QuestionVerdict, Threshold, NO, YES,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A model that answers P(yes) = `p` to every binary question and counts
    /// its calls.
    struct Fixed {
        entry: DecisionModelEntry,
        p: f64,
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    #[async_trait]
    impl DecisionModel for Fixed {
        fn model_ref(&self) -> DecisionModelRef {
            self.entry.model_ref()
        }
        fn display_name(&self) -> String {
            self.entry.display_name.clone()
        }
        fn tier(&self) -> ProviderTier {
            self.entry.tier
        }
        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            self.entry.affiliation
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: self.entry.kinds.clone(),
                max_questions: 64,
                context_tokens: self.entry.context_tokens,
                max_concurrency: LOCAL_CONCURRENCY,
                max_request_bytes: None,
                local: self.entry.local,
            }
        }
        async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if request.context.render_text().contains("FAIL") {
                return Err(DecisionError::Backend {
                    model: self.entry.model.clone(),
                    status: Some(500),
                    error_type: None,
                    message: "boom".into(),
                });
            }
            Ok(ModelVerdicts {
                verdicts: request
                    .questions
                    .keys()
                    .map(|name| QuestionVerdict {
                        question: name.clone(),
                        probabilities: IndexMap::from([
                            (YES.to_string(), self.p),
                            (NO.to_string(), 1.0 - self.p),
                        ]),
                        source: ProbabilitySource::Native,
                        calibrated: true,
                        reported_choice: None,
                        raw: serde_json::json!({"noul": self.p}),
                    })
                    .collect(),
                usage: DecisionUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(1),
                },
                served_model: None,
            })
        }
    }

    fn entry(name: &str, tier: ProviderTier, local: bool) -> DecisionModelEntry {
        DecisionModelEntry {
            provider: "test_decision".into(),
            model: name.into(),
            display_name: name.into(),
            context_tokens: 4_096,
            kinds: vec![DecisionKindTag::Binary],
            tier,
            affiliation: local.then_some(crate::privacy::affiliation::ModelAffiliation::Local),
            adapter: DecisionAdapter::SystemOne,
            local,
            load_refusal: None,
        }
    }

    struct Source {
        entries: Vec<DecisionModelEntry>,
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    #[async_trait]
    impl DecisionModelSource for Source {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            self.entries.clone()
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            let entry = self
                .entries
                .iter()
                .find(|e| e.answers_to(wanted))
                .cloned()
                .ok_or_else(|| DecisionError::NoModel("unknown".into()))?;
            Ok(Arc::new(Fixed {
                entry,
                p: 0.9,
                calls: self.calls.clone(),
                delay: self.delay,
            }))
        }
    }

    struct Allow;

    #[async_trait]
    impl DecisionUseGate for Allow {
        async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
            Ok(())
        }
    }

    /// Every entry the service hands over, and the id it named for each
    /// (`None`: the sink chooses, as [`crate::decision::log::row_id`] does).
    #[derive(Default)]
    struct Log(Mutex<Vec<DecisionLogEntry>>, Mutex<Vec<Option<String>>>);

    #[async_trait]
    impl DecisionSink for Log {
        async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(entry);
            self.1.lock().unwrap().push(None);
            Ok(())
        }
        async fn record_as(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(entry);
            self.1.lock().unwrap().push(Some(id.to_string()));
            Ok(())
        }
    }

    struct Broken;

    #[async_trait]
    impl DecisionSink for Broken {
        async fn record(&self, _: DecisionLogEntry) -> anyhow::Result<()> {
            anyhow::bail!("disk full")
        }
    }

    fn service(
        entries: Vec<DecisionModelEntry>,
        delay: Duration,
    ) -> (DecisionService, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let source = Source {
            entries,
            calls: calls.clone(),
            delay,
        };
        (
            DecisionService::new(Arc::new(source))
                .with_use_gate(Arc::new(Allow))
                .with_default_model(None),
            calls,
        )
    }

    fn request(text: &str, threshold: Option<Threshold>) -> DecisionRequest {
        DecisionRequest {
            questions: IndexMap::from([(
                "done".to_string(),
                Question {
                    instructions: "Is the task done?".into(),
                    kind: DecisionKind::Binary {},
                    threshold,
                },
            )]),
            context: DecisionContext::Text(text.into()),
        }
    }

    fn private_chat() -> DecisionSession {
        DecisionSession::new("s1", SessionClassification::Private, true)
    }

    #[test]
    fn digits_are_counted_one_token_each_beyond_what_o200k_folds() {
        assert_eq!(extra_digit_tokens(""), 0);
        assert_eq!(extra_digit_tokens("no numbers here"), 0);
        // 138 is one o200k token and three per-digit ones; 4.1 is two runs of one.
        assert_eq!(extra_digit_tokens("Na 138 K 4.1"), 2);
        // A run is closed at the end of the text too: 2026 is 2 + 2 in o200k terms.
        assert_eq!(extra_digit_tokens("EDSS since 2026"), 2);
        assert_eq!(extra_digit_tokens("1234567"), 7 - 3);
    }

    #[tokio::test]
    async fn a_private_chat_never_reaches_a_public_model() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let named = DecisionModelRef::new("test_decision", "cloud");
        let err = svc
            .decide(Some(&private_chat()), Some(&named), &request("PHI", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert!(err.to_string().contains("private") && err.to_string().contains("cloud"));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");
        // The refusal is logged, as refused.
        let rows = log.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, DecisionOutcome::Refused);
        assert!(rows[0].privacy_enforced);
    }

    /// The refusal's way out follows where the model came from: a model the
    /// caller named (a monitor, a gate, a `decide` call, the chat's own
    /// default, a listing row) is refused with what to name instead, since
    /// the configured default would not change it; only a model the
    /// configured default chose names that setting.
    #[tokio::test]
    async fn a_refusal_advises_by_where_the_public_model_came_from() {
        let named_advice =
            "Name a private decision model (decision_models lists them), or start this work in \
             a public chat.";
        let key_advice =
            "Set BIOROUTER_DECISION_MODEL to a private model, or start this work in a public chat.";
        let cloud = DecisionModelRef::new("test_decision", "cloud");
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let refusal = |result: DecisionResult<Vec<Decision>>| match result {
            Err(DecisionError::Refused(why)) => why,
            other => panic!("not refused: {other:?}"),
        };

        let why = refusal(
            svc.decide(Some(&private_chat()), Some(&cloud), &request("x", None))
                .await,
        );
        assert!(why.ends_with(named_advice), "{why}");
        let batch = svc
            .decide_batch(
                Some(&private_chat()),
                Some(&cloud),
                &request("", None).questions,
                chunks(2),
                "batch-named",
            )
            .await;
        assert!(
            matches!(&batch, Err(DecisionError::Refused(why)) if why.ends_with(named_advice)),
            "{batch:?}"
        );
        let mut chat_default = private_chat();
        chat_default.chat_default_model = Ok(Some(cloud.clone()));
        let why = refusal(
            svc.decide(Some(&chat_default), None, &request("x", None))
                .await,
        );
        assert!(why.ends_with(named_advice), "{why}");
        let listed = svc.models_for(Some(&private_chat())).await;
        let why = listed[0].1.as_deref().expect("a public model is refused");
        assert!(why.ends_with(named_advice), "{why}");

        let (configured, _) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let configured = configured.with_default_model(Some(cloud.clone()));
        let why = refusal(
            configured
                .decide(Some(&private_chat()), None, &request("x", None))
                .await,
        );
        assert!(why.ends_with(key_advice), "{why}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");
    }

    /// A workspace rule that holds for the whole chat (a Crew grant that no
    /// longer stands, which refuses every model) is every model's reason in a
    /// listing, ahead of the gate's per-model rule, privacy and a server's
    /// refusal to load, so a picker says what every call would be refused
    /// with.
    #[tokio::test]
    async fn a_listing_gives_the_workspace_gates_reason_for_every_model() {
        struct Scoped;
        #[async_trait]
        impl DecisionUseGate for Scoped {
            async fn check(
                &self,
                s: &DecisionSession,
                _: &dyn DecisionModel,
            ) -> DecisionResult<()> {
                match self.session_refusal(s).await {
                    Some(why) => Err(DecisionError::Refused(why)),
                    None => Ok(()),
                }
            }
            async fn session_refusal(&self, session: &DecisionSession) -> Option<String> {
                (session.session_id == "scoped").then(|| CREW_SCOPED_REFUSAL.to_string())
            }
        }
        let mut stale = entry("stale", ProviderTier::Private, true);
        stale.load_refusal = Some("Ollama cannot load this copy.".into());
        let (svc, calls) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
                stale,
            ],
            Duration::ZERO,
        );
        let svc = svc.with_use_gate(Arc::new(Scoped));
        let scoped = DecisionSession::new("scoped", SessionClassification::Private, true);
        let listed = svc.models_for(Some(&scoped)).await;
        assert_eq!(listed.len(), 3);
        assert!(
            listed
                .iter()
                .all(|(_, why)| why.as_deref() == Some(CREW_SCOPED_REFUSAL)),
            "{listed:?}"
        );
        let other = DecisionSession::new("other", SessionClassification::Private, true);
        let listed = svc.models_for(Some(&other)).await;
        let local = listed.iter().find(|(e, _)| e.model == "local").unwrap();
        assert_eq!(local.1, None, "another chat keeps its own reasons");
        let named = DecisionModelRef::new("test_decision", "local");
        let err = svc
            .decide(Some(&scoped), Some(&named), &request("x", None))
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            CREW_SCOPED_REFUSAL,
            "the call says the same"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_master_switch_as_sampled_decides() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let off = DecisionSession::new("s1", SessionClassification::Private, false);
        let named = DecisionModelRef::new("test_decision", "cloud");
        assert!(svc
            .decide(Some(&off), Some(&named), &request("x", None))
            .await
            .is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn automatic_choice_skips_what_the_chat_may_not_use_and_prefers_local() {
        let (svc, _) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            Duration::ZERO,
        );
        let private = svc
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap();
        assert_eq!(private[0].model.model, "local");
        let public = DecisionSession::new("s2", SessionClassification::Public, true);
        let chosen = svc
            .decide(Some(&public), None, &request("x", None))
            .await
            .unwrap();
        assert_eq!(
            chosen[0].model.model, "local",
            "local first, and the downward rule allows it"
        );

        // A model is listed, but privacy rules every one out: a refusal, as
        // naming that model would be, and nothing is sent.
        let (only_public, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let err = only_public
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert!(err.to_string().contains("private chat"), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // Nothing listed at all is no model, not a refusal.
        let (none, _) = service(vec![], Duration::ZERO);
        let err = none
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::NoModel(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_chat_raised_mid_batch_stops_reaching_a_public_model() {
        struct RaisedAfter(AtomicUsize);
        #[async_trait]
        impl LiveClassification for RaisedAfter {
            async fn current(&self, _: &str) -> anyhow::Result<SessionClassification> {
                Ok(if self.0.fetch_add(1, Ordering::SeqCst) < 2 {
                    SessionClassification::Public
                } else {
                    SessionClassification::Private
                })
            }
        }
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let session = DecisionSession::new("s", SessionClassification::Public, true)
            .with_live(Arc::new(RaisedAfter(AtomicUsize::new(0))));
        let chunks: Vec<_> = (0..4)
            .map(|i| DecisionChunk {
                id: format!("c{i}"),
                context: DecisionContext::Text(format!("chunk {i}")),
            })
            .collect();
        // One read for the batch gate, then one per chunk: the first two pass
        // and every later read says Private.
        let outcomes = svc
            .decide_many(
                Some(&session),
                Some(&DecisionModelRef::new("test_decision", "cloud")),
                &request("", None).questions,
                chunks,
            )
            .await
            .unwrap();
        let refused = outcomes.iter().filter(|o| o.error.is_some()).count();
        assert!(
            refused >= 2,
            "chunks after the raise are refused: {outcomes:?}"
        );
        assert!(calls.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn thresholds_confidence_and_usage_reach_the_decision() {
        let (svc, _) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let decisions = svc
            .decide(None, None, &request("x", Some(Threshold::Yes(0.85))))
            .await
            .unwrap();
        let d = &decisions[0];
        assert_eq!(d.answer, Answer::Binary { value: true });
        assert_eq!(d.passed, Some(true));
        assert!((d.confidence - 0.8).abs() < 1e-9);
        assert_eq!(d.usage.input_tokens, Some(10));
        assert_eq!(d.tier, ProviderTier::Private);
        let failing = svc
            .decide(None, None, &request("x", Some(Threshold::Yes(0.95))))
            .await
            .unwrap();
        assert_eq!(failing[0].passed, Some(false));
    }

    #[tokio::test]
    async fn an_oversized_state_is_refused_before_sending_and_names_the_chunk() {
        let (svc, calls) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let big = "word ".repeat(10_000);
        let err = svc
            .decide(None, None, &request(&big, None))
            .await
            .unwrap_err();
        let DecisionError::TooLarge { chunk, limit, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(chunk, "The state");
        assert_eq!(*limit, 4_096);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let outcomes = svc
            .decide_many(
                None,
                None,
                &request("", None).questions,
                vec![
                    DecisionChunk {
                        id: "note-1".into(),
                        context: DecisionContext::Text("short".into()),
                    },
                    DecisionChunk {
                        id: "note-2".into(),
                        context: DecisionContext::Text(big),
                    },
                ],
            )
            .await
            .unwrap();
        assert!(outcomes[0].decisions.is_some());
        let error = outcomes[1].error.as_deref().unwrap();
        assert!(error.contains("`note-2`"), "{error}");
    }

    #[tokio::test]
    async fn one_failed_chunk_never_fails_the_batch_and_order_is_kept() {
        let (svc, calls) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::from_millis(5),
        );
        let chunks: Vec<_> = ["a", "FAIL", "c", "d"]
            .iter()
            .enumerate()
            .map(|(i, text)| DecisionChunk {
                id: format!("c{i}"),
                context: DecisionContext::Text(text.to_string()),
            })
            .collect();
        let outcomes = svc
            .decide_many(None, None, &request("", None).questions, chunks)
            .await
            .unwrap();
        assert_eq!(
            outcomes
                .iter()
                .map(|o| o.chunk_id.as_str())
                .collect::<Vec<_>>(),
            ["c0", "c1", "c2", "c3"]
        );
        assert!(outcomes[1].error.is_some() && outcomes[1].decisions.is_none());
        assert!(outcomes.iter().filter(|o| o.decisions.is_some()).count() == 3);
        assert_eq!(calls.load(Ordering::SeqCst), 4);

        // Repeated ids and an empty batch are the caller's mistake, not a chunk's.
        assert!(svc
            .decide_many(
                None,
                None,
                &request("", None).questions,
                vec![
                    DecisionChunk {
                        id: "x".into(),
                        context: DecisionContext::Text("a".into())
                    },
                    DecisionChunk {
                        id: "x".into(),
                        context: DecisionContext::Text("b".into())
                    },
                ],
            )
            .await
            .is_err());
        assert!(svc
            .decide_many(None, None, &request("", None).questions, vec![])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_slow_model_times_out_and_a_failed_log_write_is_not_an_error() {
        let (svc, _) = service(
            vec![entry("remote", ProviderTier::Public, false)],
            Duration::from_millis(200),
        );
        let svc = svc
            .with_timeouts(Duration::from_millis(20), Duration::from_millis(20))
            .with_sink(Arc::new(Broken));
        let err = svc
            .decide(None, None, &request("x", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Timeout { .. }), "{err:?}");
        assert!(err.is_transient());

        let (fast, _) = service(
            vec![entry("remote", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let fast = fast.with_sink(Arc::new(Broken));
        assert!(fast.decide(None, None, &request("x", None)).await.is_ok());
    }

    #[tokio::test]
    async fn kinds_the_model_cannot_take_are_refused_before_sending() {
        let (svc, calls) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let mut req = request("x", None);
        req.questions.insert(
            "level".into(),
            Question {
                instructions: "Rate".into(),
                kind: DecisionKind::Score {
                    levels: vec!["a".into(), "b".into()],
                },
                threshold: None,
            },
        );
        let err = svc.decide(None, None, &req).await.unwrap_err();
        assert!(matches!(err, DecisionError::Unsupported(_)), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_institution_the_model_does_not_cover_is_refused() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Private, false)],
            Duration::ZERO,
        );
        let session =
            private_chat().with_affiliations(BTreeSet::from([InstitutionId::new("ucsf")]));
        let err = svc
            .decide(
                Some(&session),
                Some(&DecisionModelRef::new("test_decision", "cloud")),
                &request("x", None),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_margin_scales_with_the_window() {
        assert_eq!(context_margin(16_384), 512);
        assert_eq!(context_margin(2_000), 128);
    }

    #[tokio::test]
    async fn a_listing_names_why_a_chat_may_not_use_a_model_and_sends_nothing() {
        let (svc, calls) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            Duration::ZERO,
        );
        let reasons = |listed: Vec<(DecisionModelEntry, Option<String>)>| {
            listed
                .into_iter()
                .map(|(entry, reason)| (entry.model, reason))
                .collect::<Vec<_>>()
        };

        let private = reasons(svc.models_for(Some(&private_chat())).await);
        assert_eq!(private[0], ("local".to_string(), None), "local first");
        assert_eq!(private[1].0, "cloud");
        let why = private[1].1.as_deref().expect("a public model is refused");
        assert!(why.contains("private") && why.contains("cloud"), "{why}");

        let public = DecisionSession::new("s2", SessionClassification::Public, true);
        assert!(reasons(svc.models_for(Some(&public)).await)
            .iter()
            .all(|(_, reason)| reason.is_none()));
        let off = DecisionSession::new("s3", SessionClassification::Private, false);
        assert!(reasons(svc.models_for(Some(&off)).await)
            .iter()
            .all(|(_, reason)| reason.is_none()));
        assert!(reasons(svc.models_for(None).await)
            .iter()
            .all(|(_, reason)| reason.is_none()));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a listing asks no model");
    }

    fn chunks(n: usize) -> Vec<DecisionChunk> {
        (0..n)
            .map(|i| DecisionChunk {
                id: format!("note-{i:03}"),
                context: DecisionContext::Text(format!("SYNTHETIC NOTE {i}")),
            })
            .collect()
    }

    /// A batch the privacy gate refuses before any chunk is sent is ONE
    /// refused row in the log, under the batch's id, never one per chunk and
    /// never none: the history is where the person sees why it did not run.
    #[tokio::test]
    async fn a_batch_refused_before_sending_is_one_refused_row() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let named = DecisionModelRef::new("test_decision", "cloud");
        let turn = "turn-refused-before-sending";
        let err = svc
            .decide_batch(
                Some(&private_chat().with_turn(Some(turn.into()))),
                Some(&named),
                &request("", None).questions,
                chunks(50),
                "batch-1",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");
        // The turn's marker counts every chunk the one row stands for.
        let tally = super::super::monitor::take_turn_tally(turn);
        assert_eq!(tally.len(), 1, "{tally:?}");
        assert_eq!((tally[0].count, tally[0].errors), (50, 50));
        let rows = log.0.lock().unwrap();
        assert_eq!(rows.len(), 1, "one row for the batch");
        let row = &rows[0];
        assert_eq!(row.outcome, DecisionOutcome::Refused);
        assert_eq!(row.batch_id.as_deref(), Some("batch-1"));
        assert_eq!((row.chunk_index, row.chunk_id.as_deref()), (None, None));
        assert_eq!(row.model.as_ref().map(|m| m.model.as_str()), Some("cloud"));
        assert!(row.decisions.is_empty());
        let text = row.context.render_text();
        assert!(
            text.starts_with("Not sent") && text.contains("50 chunks") && text.contains("note-000"),
            "{text}"
        );
        assert!(text.contains("and 30 more"), "{text}");
        assert!(
            !text.contains("SYNTHETIC NOTE"),
            "the row names chunks, it does not hold them"
        );
    }

    /// A batch no listed model may serve (privacy rules out every one) is
    /// one refused row, the outcome naming such a model would get.
    #[tokio::test]
    async fn a_batch_no_listed_model_may_serve_is_one_refused_row() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let err = svc
            .decide_batch(
                Some(&private_chat()),
                None,
                &request("", None).questions,
                chunks(3),
                "batch-2",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let rows = log.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, DecisionOutcome::Refused);
        assert_eq!(rows[0].batch_id.as_deref(), Some("batch-2"));
        assert_eq!(rows[0].model, None);
        assert!(rows[0].context.render_text().contains("3 chunks"));
    }

    /// A batch with nothing listed at all is one error row: no model, not a
    /// refusal.
    #[tokio::test]
    async fn a_batch_with_nothing_listed_is_one_error_row() {
        let (svc, _) = service(vec![], Duration::ZERO);
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let err = svc
            .decide_batch(
                Some(&private_chat()),
                None,
                &request("", None).questions,
                chunks(3),
                "batch-3",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::NoModel(_)), "{err:?}");
        let rows = log.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, DecisionOutcome::Error);
    }

    /// A chat raised to Private partway through a batch stops the rest: the
    /// chunks after the raise are not sent, and they are ONE refused row that
    /// names them, never one row per chunk holding text that was never sent.
    #[tokio::test]
    async fn a_chat_raised_mid_batch_logs_one_row_for_the_unsent_rest() {
        struct RaisedAfter(AtomicUsize);
        #[async_trait]
        impl LiveClassification for RaisedAfter {
            async fn current(&self, _: &str) -> anyhow::Result<SessionClassification> {
                Ok(if self.0.fetch_add(1, Ordering::SeqCst) < 3 {
                    SessionClassification::Public
                } else {
                    SessionClassification::Private
                })
            }
        }
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let live = Arc::new(RaisedAfter(AtomicUsize::new(0)));
        let turn = "turn-raised-mid-batch";
        let session = DecisionSession::new("s", SessionClassification::Public, true)
            .with_live(live.clone())
            .with_turn(Some(turn.into()));
        let outcomes = svc
            .decide_batch(
                Some(&session),
                Some(&DecisionModelRef::new("test_decision", "cloud")),
                &request("", None).questions,
                chunks(8),
                "batch-raised",
            )
            .await
            .unwrap();
        let stopped: Vec<&str> = outcomes
            .iter()
            .filter(|o| o.error.is_some())
            .map(|o| o.chunk_id.as_str())
            .collect();
        assert!(!stopped.is_empty(), "{outcomes:?}");
        let rows = log.0.lock().unwrap();
        let refused: Vec<_> = rows
            .iter()
            .filter(|r| r.outcome == DecisionOutcome::Refused)
            .collect();
        assert_eq!(refused.len(), 1, "one row for the rest of the batch");
        let row = refused[0];
        assert_eq!(row.batch_id.as_deref(), Some("batch-raised"));
        assert_eq!((row.chunk_index, row.chunk_id.as_deref()), (None, None));
        let text = row.context.render_text();
        assert!(
            text.starts_with("Not sent") && text.contains("of 8 chunks"),
            "{text}"
        );
        for id in &stopped {
            assert!(text.contains(id), "{id} is named: {text}");
        }
        assert!(!text.contains("SYNTHETIC NOTE"), "{text}");
        let decided = rows
            .iter()
            .filter(|r| r.outcome == DecisionOutcome::Decided)
            .count();
        assert!(rows
            .iter()
            .filter(|r| r.outcome == DecisionOutcome::Decided)
            .all(|r| r.chunk_index.is_some()));
        assert_eq!(rows.len(), decided + 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            decided,
            "nothing after the raise"
        );
        assert_eq!(decided + stopped.len(), 8);
        assert!(
            live.0.load(Ordering::SeqCst) < 9,
            "the stopped rest is not read again chunk by chunk"
        );
        // The marker names the batch's 8 chunks, as the tool's result does,
        // with every stopped one an error, not one error for the row.
        let tally = super::super::monitor::take_turn_tally(turn);
        assert_eq!(tally.len(), 1, "{tally:?}");
        assert_eq!((tally[0].count, tally[0].errors), (8, stopped.len() as u64));
    }

    /// Every decision of one request carries that request's id, and the log
    /// stores the request's row under it, so a holder of any decision names
    /// its row without a lookup.
    #[tokio::test]
    async fn a_requests_decisions_share_the_id_its_row_is_stored_under() {
        let (svc, _) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let mut two = request("the note", Some(Threshold::Yes(0.5)));
        two.questions.insert(
            "signed".to_string(),
            Question {
                instructions: "Is the note signed?".into(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
        );
        let decisions = svc.decide(None, None, &two).await.unwrap();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].request_id, decisions[1].request_id);
        let other = svc.decide(None, None, &two).await.unwrap();
        assert_ne!(other[0].request_id, decisions[0].request_id);
        let rows = log.0.lock().unwrap();
        assert_eq!(
            crate::decision::log::row_id(&rows[0]),
            decisions[0].request_id
        );
        assert_eq!(crate::decision::log::row_id(&rows[1]), other[0].request_id);
    }

    /// `decide_as` stores its row under the caller's id, whether the request
    /// succeeds (every decision carries the id) or fails (the failure row has
    /// no decision to carry one). A request that fails validation is not
    /// logged at all.
    #[tokio::test]
    async fn decide_as_uses_the_callers_id() {
        let (svc, _) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let chat = DecisionSession::new("s", SessionClassification::Public, true);
        let decisions = svc
            .decide_as(Some(&chat), None, &request("x", None), "req-ok")
            .await
            .unwrap();
        assert!(decisions.iter().all(|d| d.request_id == "req-ok"));
        let err = svc
            .decide_as(Some(&chat), None, &request("FAIL", None), "req-fail")
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Backend { .. }), "{err:?}");
        let mut bad = request("x", None);
        bad.questions.clear();
        assert!(svc
            .decide_as(Some(&chat), None, &bad, "req-invalid")
            .await
            .is_err());
        let rows = log.0.lock().unwrap();
        let ids = log.1.lock().unwrap();
        assert_eq!(rows.len(), 2, "the invalid request is not logged");
        assert_eq!(ids[0].as_deref(), Some("req-ok"));
        assert_eq!(rows[0].outcome, DecisionOutcome::Decided);
        assert_eq!(ids[1].as_deref(), Some("req-fail"));
        assert_eq!(rows[1].outcome, DecisionOutcome::Error);
        assert!(rows[1].decisions.is_empty());
    }

    /// A decision chat whose request fails names the failure's row, so the
    /// reply draws as a failed card with a link to what would have been sent.
    #[tokio::test]
    async fn a_failed_decision_chat_reply_names_its_failure_row() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let chat = DecisionSession::new("s", SessionClassification::Public, true);
        let model = DecisionModelRef::new("test_decision", "cloud");
        let reply = crate::providers::decision::chat::run_decision_chat(
            &svc,
            chat,
            &model,
            None,
            "Q: Is it done?\nFAIL this note",
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(reply.decision.is_none());
        let id = reply.decision_id.clone().expect("the reply names its row");
        assert!(reply
            .text
            .starts_with("The decision model could not answer"));
        let rows = log.0.lock().unwrap();
        let ids = log.1.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(ids[0].as_deref(), Some(id.as_str()));
        assert_eq!(rows[0].outcome, DecisionOutcome::Error);
    }

    /// A refused decision-chat request names its refused row, and nothing is
    /// sent.
    #[tokio::test]
    async fn a_refused_decision_chat_reply_names_its_refused_row() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let model = DecisionModelRef::new("test_decision", "cloud");
        let reply = crate::providers::decision::chat::run_decision_chat(
            &svc,
            private_chat(),
            &model,
            None,
            "Q: Is it done?\nSYNTHETIC NOTE",
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");
        let id = reply.decision_id.clone().expect("the reply names its row");
        let rows = log.0.lock().unwrap();
        let ids = log.1.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(ids[0].as_deref(), Some(id.as_str()));
        assert_eq!(rows[0].outcome, DecisionOutcome::Refused);
    }

    /// The window a decision would use is read from the listing, by the same
    /// choice `decide` makes, and nothing is opened or sent to learn it.
    /// The model order's second rung: a call that names no model uses the
    /// chat's own default ahead of the configured one, and a chat whose
    /// settings cannot be read refuses such a call rather than guess.
    #[tokio::test]
    async fn a_chats_default_decision_model_comes_before_the_configured_one() {
        let mut local = entry("local", ProviderTier::Private, true);
        local.context_tokens = 2_048;
        let (svc, _) = service(
            vec![entry("cloud", ProviderTier::Public, false), local],
            Duration::ZERO,
        );
        // `BIOROUTER_DECISION_MODEL` names the hosted model.
        let svc = svc.with_default_model(Some(DecisionModelRef::new("test_decision", "cloud")));
        let public = DecisionSession::new("s", SessionClassification::Public, true);
        let configured = svc
            .decide(Some(&public), None, &request("x", None))
            .await
            .unwrap();
        assert_eq!(configured[0].model.model, "cloud");

        let local_default = public
            .clone()
            .with_chat_default_model(Ok(Some(DecisionModelRef::new("test_decision", "local"))));
        let chosen = svc
            .decide(Some(&local_default), None, &request("x", None))
            .await
            .unwrap();
        assert_eq!(chosen[0].model.model, "local", "the chat's default wins");
        assert_eq!(
            svc.context_tokens_for(Some(&local_default), None).await,
            Some(2_048),
            "the window looked up is the chat default's"
        );
        let named = DecisionModelRef::new("test_decision", "cloud");
        let overridden = svc
            .decide(Some(&local_default), Some(&named), &request("x", None))
            .await
            .unwrap();
        assert_eq!(overridden[0].model.model, "cloud", "a named model wins");

        let unreadable = public
            .with_chat_default_model(Err("This chat's decision settings could not be read".into()));
        let err = svc
            .decide(Some(&unreadable), None, &request("x", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::NoModel(_)), "{err:?}");
        assert!(err.to_string().contains("Name a model"), "{err}");
        assert!(svc
            .decide(Some(&unreadable), Some(&named), &request("x", None))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn the_window_named_is_the_one_decide_would_use() {
        let mut small = entry("local", ProviderTier::Private, true);
        small.context_tokens = 2_048;
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false), small],
            Duration::ZERO,
        );
        assert_eq!(
            svc.context_tokens_for(Some(&private_chat()), None).await,
            Some(2_048),
            "a private chat's automatic choice is the local model"
        );
        let cloud = DecisionModelRef::new("test_decision", "cloud");
        assert_eq!(
            svc.context_tokens_for(None, Some(&cloud)).await,
            Some(4_096)
        );
        let unknown = DecisionModelRef::new("test_decision", "unlisted");
        assert_eq!(svc.context_tokens_for(None, Some(&unknown)).await, None);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// The entry the listing names for a call is the model that call then
    /// uses: the automatic choice (local first, and what the chat may use),
    /// the chat's own default, a named model. A chat whose settings cannot be
    /// read names none, because such a call is refused.
    #[tokio::test]
    async fn the_entry_named_is_the_model_decide_uses() {
        let (svc, _) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            Duration::ZERO,
        );
        let public = DecisionSession::new("s", SessionClassification::Public, true);
        let chats_default = public
            .clone()
            .with_chat_default_model(Ok(Some(DecisionModelRef::new("test_decision", "cloud"))));
        for session in [&public, &private_chat(), &chats_default] {
            let named = svc.model_entry_for(Some(session), None).await.unwrap();
            let used = svc
                .decide(Some(session), None, &request("x", None))
                .await
                .unwrap();
            assert_eq!(named.model_ref(), used[0].model, "{session:?}");
        }
        assert_eq!(
            svc.model_entry_for(Some(&chats_default), None)
                .await
                .unwrap()
                .model,
            "cloud"
        );
        let cloud = DecisionModelRef::new("test_decision", "cloud");
        assert_eq!(
            svc.model_entry_for(Some(&public), Some(&cloud))
                .await
                .unwrap()
                .model,
            "cloud"
        );
        let unreadable = public.with_chat_default_model(Err("unreadable".into()));
        assert!(svc.model_entry_for(Some(&unreadable), None).await.is_none());
        assert!(svc.resolve_for(Some(&unreadable), None).await.is_err());
    }

    /// A model pinned for a run (as `biorouter decide` pins it) is the only
    /// model the run reaches: once it leaves the listing, a request naming it
    /// fails rather than moving to the next model, which here is hosted.
    #[tokio::test]
    async fn a_pinned_model_is_never_swapped_for_another() {
        struct Vanishing {
            entries: Vec<DecisionModelEntry>,
            gone: Mutex<HashSet<String>>,
            calls: Mutex<HashMap<String, Arc<AtomicUsize>>>,
        }
        impl Vanishing {
            fn counter(&self, model: &str) -> Arc<AtomicUsize> {
                self.calls
                    .lock()
                    .unwrap()
                    .entry(model.to_string())
                    .or_default()
                    .clone()
            }
        }
        #[async_trait]
        impl DecisionModelSource for Vanishing {
            async fn list(&self) -> Vec<DecisionModelEntry> {
                let gone = self.gone.lock().unwrap().clone();
                self.entries
                    .iter()
                    .filter(|e| !gone.contains(&e.model))
                    .cloned()
                    .collect()
            }
            async fn open(
                &self,
                wanted: &DecisionModelRef,
            ) -> DecisionResult<Arc<dyn DecisionModel>> {
                let entry = self
                    .list()
                    .await
                    .into_iter()
                    .find(|e| e.answers_to(wanted))
                    .ok_or_else(|| DecisionError::NoModel("not listed".into()))?;
                let calls = self.counter(&entry.model);
                Ok(Arc::new(Fixed {
                    entry,
                    p: 0.9,
                    calls,
                    delay: Duration::ZERO,
                }))
            }
        }
        let source = Arc::new(Vanishing {
            entries: vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            gone: Mutex::new(HashSet::new()),
            calls: Mutex::new(HashMap::new()),
        });
        let svc = DecisionService::new(source.clone())
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let public = DecisionSession::new("s", SessionClassification::Public, true);
        let pinned = svc
            .resolve_for(Some(&public), None)
            .await
            .unwrap()
            .model_ref();
        assert_eq!(pinned.model, "local");
        let questions = request("", None).questions;
        let first = svc
            .decide_many(Some(&public), Some(&pinned), &questions, chunks(2))
            .await
            .unwrap();
        assert!(first.iter().all(|o| o.decisions.is_some()));

        source.gone.lock().unwrap().insert("local".into());
        assert!(svc
            .decide_many(Some(&public), Some(&pinned), &questions, chunks(2))
            .await
            .is_err());
        assert_eq!(source.counter("cloud").load(Ordering::SeqCst), 0);
        // Unpinned, the same call would have moved to the hosted model.
        svc.decide_many(Some(&public), None, &questions, chunks(1))
            .await
            .unwrap();
        assert_eq!(source.counter("cloud").load(Ordering::SeqCst), 1);
    }

    /// A model that answers but leaves a question out has still been billed,
    /// so its failure row keeps the usage the model reported, and the usage
    /// ledger counts it.
    #[tokio::test]
    async fn a_malformed_answer_still_logs_its_usage() {
        struct HalfAnswer(DecisionModelEntry);
        #[async_trait]
        impl DecisionModel for HalfAnswer {
            fn model_ref(&self) -> DecisionModelRef {
                self.0.model_ref()
            }
            fn display_name(&self) -> String {
                self.0.display_name.clone()
            }
            fn tier(&self) -> ProviderTier {
                self.0.tier
            }
            fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
                self.0.affiliation
            }
            fn caps(&self) -> DecisionCaps {
                DecisionCaps {
                    kinds: self.0.kinds.clone(),
                    max_questions: 64,
                    context_tokens: self.0.context_tokens,
                    max_concurrency: 1,
                    max_request_bytes: None,
                    local: false,
                }
            }
            async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
                let first = request.questions.keys().next().unwrap().clone();
                Ok(ModelVerdicts {
                    verdicts: vec![QuestionVerdict {
                        question: first,
                        probabilities: IndexMap::from([
                            (YES.to_string(), 0.7),
                            (NO.to_string(), 0.3),
                        ]),
                        source: ProbabilitySource::Native,
                        calibrated: true,
                        reported_choice: None,
                        raw: serde_json::Value::Null,
                    }],
                    usage: DecisionUsage {
                        input_tokens: Some(50),
                        output_tokens: Some(2),
                    },
                    served_model: None,
                })
            }
        }
        struct OneModel(DecisionModelEntry);
        #[async_trait]
        impl DecisionModelSource for OneModel {
            async fn list(&self) -> Vec<DecisionModelEntry> {
                vec![self.0.clone()]
            }
            async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
                Ok(Arc::new(HalfAnswer(self.0.clone())))
            }
        }
        let svc = DecisionService::new(Arc::new(OneModel(entry(
            "half",
            ProviderTier::Public,
            false,
        ))))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None);
        let log = Arc::new(Log::default());
        svc.set_sink(log.clone());
        let mut asked = request("a note", None);
        asked.questions.insert(
            "signed".into(),
            Question {
                instructions: "Is it signed?".into(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
        );
        let chat = DecisionSession::new("s1", SessionClassification::Public, true);
        let err = svc.decide(Some(&chat), None, &asked).await.unwrap_err();
        assert!(matches!(err, DecisionError::Malformed { .. }), "{err:?}");
        let rows = log.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, DecisionOutcome::Error);
        assert_eq!(
            rows[0].usage,
            DecisionUsage {
                input_tokens: Some(50),
                output_tokens: Some(2),
            },
            "the billed request's usage stays on its failure row"
        );
        assert!(
            rows[0].model.is_some(),
            "the ledger needs the model it billed"
        );
    }

    /// A model its server will not load stays listed, with the server's
    /// reason as the reason it is unavailable, and the automatic choice
    /// passes over it. A call that names it is still sent to it.
    #[tokio::test]
    async fn the_automatic_choice_passes_over_a_model_its_server_will_not_load() {
        let refusal = "Ollama decision model `stale` cannot be loaded: re-download it.";
        let mut stale = entry("stale", ProviderTier::Private, true);
        stale.load_refusal = Some(refusal.to_string());
        let (svc, calls) = service(
            vec![stale.clone(), entry("good", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let listed = svc.models_for(None).await;
        assert_eq!(
            listed[0].0.model, "stale",
            "still listed, in the order given"
        );
        assert_eq!(listed[0].1.as_deref(), Some(refusal));
        assert_eq!(listed[1].1, None);
        let public = DecisionSession::new("s2", SessionClassification::Public, true);
        let for_chat = svc.models_for(Some(&public)).await;
        assert_eq!(for_chat[0].1.as_deref(), Some(refusal));

        for session in [None, Some(&public), Some(&private_chat())] {
            let chosen = svc
                .decide(session, None, &request("x", None))
                .await
                .unwrap();
            assert_eq!(chosen[0].model.model, "good");
            assert_eq!(
                svc.model_entry_for(session, None).await.map(|e| e.model),
                Some("good".to_string()),
                "the CLI pins the same model"
            );
        }
        let named = DecisionModelRef::new("test_decision", "stale");
        let before = calls.load(Ordering::SeqCst);
        svc.decide(None, Some(&named), &request("x", None))
            .await
            .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            before + 1,
            "a named model is asked"
        );

        // Only that model listed: there is no model, not a privacy refusal,
        // and the reason names the fix.
        let (only_stale, calls) = service(vec![stale.clone()], Duration::ZERO);
        for session in [None, Some(&public), Some(&private_chat())] {
            let err = only_stale
                .decide(session, None, &request("x", None))
                .await
                .unwrap_err();
            assert!(matches!(err, DecisionError::NoModel(_)), "{err:?}");
            assert!(err.to_string().contains(refusal), "{err}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // A private chat whose local model will not load, beside a hosted
        // model it may not use: refused by privacy, and the stale copy is
        // named too.
        let (beside_public, _) = service(
            vec![stale, entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let err = beside_public
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert!(err.to_string().contains(refusal), "{err}");
    }

    #[test]
    fn a_state_budget_leaves_room_for_the_margin_and_the_questions() {
        let questions = request("", None).questions;
        let budget = state_budget_tokens(4_096, &questions);
        assert!(budget < 4_096 - context_margin(4_096));
        assert!(budget > 3_000, "{budget}");
        assert_eq!(state_budget_tokens(100, &questions), 0, "never negative");

        // A question's digits are charged as the size check charges them.
        let asked = |instructions: &str| {
            let mut questions = request("", None).questions;
            questions[0].instructions = instructions.to_string();
            state_budget_tokens(4_096, &questions)
        };
        let letters = asked("Is any lot number above abcdefghijkl?");
        let digits = asked("Is any lot number above 123456789012?");
        assert_eq!(letters - digits, extra_digit_tokens("123456789012"));
    }

    // ── usage on unreadable answers, late and abandoned answers ────────────

    /// A model whose answer, usage, delay and failure the test sets, which
    /// says when it has answered.
    struct Scripted {
        entry: DecisionModelEntry,
        delay: Duration,
        usage: DecisionUsage,
        /// Answer with this error instead of verdicts.
        error: Option<DecisionError>,
        /// Notified when the model is asked.
        asked: Arc<tokio::sync::Notify>,
        /// Notified when it has its answer.
        answered: Arc<tokio::sync::Notify>,
        calls: Arc<AtomicUsize>,
    }

    impl Scripted {
        fn new(entry: DecisionModelEntry, delay: Duration, usage: DecisionUsage) -> Self {
            Self {
                entry,
                delay,
                usage,
                error: None,
                asked: Arc::new(tokio::sync::Notify::new()),
                answered: Arc::new(tokio::sync::Notify::new()),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl DecisionModel for Scripted {
        fn model_ref(&self) -> DecisionModelRef {
            self.entry.model_ref()
        }
        fn display_name(&self) -> String {
            self.entry.display_name.clone()
        }
        fn tier(&self) -> ProviderTier {
            self.entry.tier
        }
        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            self.entry.affiliation
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: self.entry.kinds.clone(),
                max_questions: 64,
                context_tokens: self.entry.context_tokens,
                max_concurrency: 4,
                max_request_bytes: None,
                local: self.entry.local,
            }
        }
        async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.asked.notify_one();
            tokio::time::sleep(self.delay).await;
            self.answered.notify_one();
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(ModelVerdicts {
                verdicts: request
                    .questions
                    .keys()
                    .map(|name| QuestionVerdict {
                        question: name.clone(),
                        probabilities: IndexMap::from([
                            (YES.to_string(), 0.9),
                            (NO.to_string(), 0.1),
                        ]),
                        source: ProbabilitySource::Native,
                        calibrated: true,
                        reported_choice: None,
                        raw: serde_json::Value::Null,
                    })
                    .collect(),
                usage: self.usage,
                served_model: Some(format!("{}-served", self.entry.model)),
            })
        }
    }

    /// Lists one model and opens it.
    struct OneScripted(Arc<Scripted>);

    #[async_trait]
    impl DecisionModelSource for OneScripted {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            vec![self.0.entry.clone()]
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            if self.0.entry.answers_to(wanted) {
                Ok(self.0.clone())
            } else {
                Err(DecisionError::NoModel("unknown".into()))
            }
        }
    }

    /// Every row the service hands its sink, by kind: `record` (no id),
    /// `record_as` (the id) and `record_late`. `record_as` can be held until
    /// a notification.
    #[derive(Default)]
    struct LateLog {
        rows: Mutex<Vec<(Option<String>, DecisionLogEntry)>>,
        late: Mutex<Vec<(String, DecisionLogEntry)>>,
        late_seen: tokio::sync::Notify,
        hold_rows_until: Option<Arc<tokio::sync::Notify>>,
        /// After the hold, never finish writing a row (notifying
        /// `row_stalled`), so the caller can go away mid-write.
        stall_rows: bool,
        row_stalled: tokio::sync::Notify,
        /// Notified when a late row's write starts, which then waits for
        /// `hold_late_until` before it lands, so a caller can be stopped
        /// while a late answer is being stored.
        late_started: tokio::sync::Notify,
        hold_late_until: Option<Arc<tokio::sync::Notify>>,
    }

    impl LateLog {
        fn rows(&self) -> Vec<(Option<String>, DecisionLogEntry)> {
            self.rows.lock().unwrap().clone()
        }
        fn late(&self) -> Vec<(String, DecisionLogEntry)> {
            self.late.lock().unwrap().clone()
        }
        /// Wait (generously) for the first late row.
        async fn first_late(&self) -> (String, DecisionLogEntry) {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let seen = self.late_seen.notified();
                    if let Some(first) = self.late().into_iter().next() {
                        return first;
                    }
                    seen.await;
                }
            })
            .await
            .expect("a late answer is recorded")
        }
    }

    #[async_trait]
    impl DecisionSink for LateLog {
        async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.rows.lock().unwrap().push((None, entry));
            Ok(())
        }
        async fn record_as(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
            if let Some(hold) = &self.hold_rows_until {
                hold.notified().await;
                // The answer's task sends on the channel right after the
                // model returns; let it run before the row is done.
                for _ in 0..8 {
                    tokio::task::yield_now().await;
                }
            }
            if self.stall_rows {
                self.row_stalled.notify_one();
                std::future::pending::<()>().await;
            }
            self.rows
                .lock()
                .unwrap()
                .push((Some(id.to_string()), entry));
            Ok(())
        }
        async fn record_late(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.late_started.notify_one();
            if let Some(hold) = &self.hold_late_until {
                hold.notified().await;
            }
            self.late.lock().unwrap().push((id.to_string(), entry));
            self.late_seen.notify_waiters();
            Ok(())
        }
    }

    fn usage(input: i64, output: i64) -> DecisionUsage {
        DecisionUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
        }
    }

    fn scripted_service(model: Arc<Scripted>) -> DecisionService {
        DecisionService::new(Arc::new(OneScripted(model)))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None)
    }

    fn public_chat(id: &str) -> DecisionSession {
        DecisionSession::new(id, SessionClassification::Public, true)
    }

    /// An answer that could not be read was still billed: its failure row,
    /// and in a batch the chunk's outcome, keep what the adapter said it was
    /// billed for.
    #[tokio::test]
    async fn an_adapter_error_that_was_billed_keeps_its_usage() {
        let mut model = Scripted::new(
            entry("odd", ProviderTier::Public, false),
            Duration::ZERO,
            DecisionUsage::default(),
        );
        model.error = Some(
            DecisionError::malformed("odd", "the answer was not JSON")
                .with_billed_usage(usage(40, 3)),
        );
        let svc = scripted_service(Arc::new(model));
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());
        let chat = public_chat("billed-error");
        let err = svc
            .decide(Some(&chat), None, &request("a note", None))
            .await
            .unwrap_err();
        assert_eq!(err.billed_usage(), usage(40, 3));
        let rows = log.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.outcome, DecisionOutcome::Error);
        assert_eq!(
            rows[0].1.usage,
            usage(40, 3),
            "the failure row keeps its usage"
        );

        let outcomes = svc
            .decide_many(Some(&chat), None, &request("", None).questions, chunks(2))
            .await
            .unwrap();
        assert!(outcomes
            .iter()
            .all(|o| o.error.is_some() && o.billed_usage == Some(usage(40, 3))));
        let wire = serde_json::to_value(&outcomes[0]).unwrap();
        assert_eq!(wire["billedUsage"]["inputTokens"], 40);

        // An error the model never answered with bills nothing.
        let (plain, _) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let outcomes = plain
            .decide_many(
                None,
                None,
                &request("", None).questions,
                vec![DecisionChunk {
                    id: "x".into(),
                    context: DecisionContext::Text("FAIL".into()),
                }],
            )
            .await
            .unwrap();
        assert_eq!(outcomes[0].billed_usage, None);
        assert!(serde_json::to_value(&outcomes[0])
            .unwrap()
            .get("billedUsage")
            .is_none());
    }

    /// A model that answers after the deadline: the timeout row is written as
    /// before, and the answer that comes later fills its tokens in, under the
    /// same id, saying it came too late.
    #[tokio::test]
    async fn a_late_answer_is_billed_to_its_timeout_row() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(150),
            usage(30, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(20), Duration::from_millis(20));
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());
        let chat = public_chat("late-timeout");
        let err = svc
            .decide(Some(&chat), None, &request("a note", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Timeout { .. }), "{err:?}");
        let rows = log.rows();
        assert_eq!(rows.len(), 1, "one timeout row");
        let (id, row) = &rows[0];
        let id = id
            .clone()
            .expect("the timeout row is stored under the request id");
        assert_eq!(row.outcome, DecisionOutcome::Error);
        assert_eq!(row.usage, DecisionUsage::default());

        let (late_id, late) = log.first_late().await;
        assert_eq!(late_id, id, "the late answer names the timeout row");
        assert_eq!(late.usage, usage(30, 1));
        assert_eq!(late.outcome, DecisionOutcome::Error);
        assert!(late.decisions.is_empty());
        assert_eq!(late.served_model.as_deref(), Some("slow-served"));
        assert!(late.latency_ms.is_some_and(|ms| ms >= 150), "{late:?}");
        let text = late.error.as_deref().unwrap();
        assert!(
            text.starts_with(
                "slow did not answer within 0 s. Nothing was decided. It answered \
                              after "
            ) && text.contains("too late to use; its tokens are counted in Usage."),
            "{text}"
        );
        assert_eq!(late.session_id.as_deref(), Some("late-timeout"));
        assert_eq!(log.rows().len(), 1, "a late answer writes no second row");
    }

    /// An answer that arrives while the timeout row is still being written is
    /// read by the waiter, which then hands it over: exactly once.
    #[tokio::test]
    async fn an_answer_that_lands_while_the_timeout_row_is_written_is_still_billed() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(60),
            usage(30, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(20), Duration::from_millis(20));
        let log = Arc::new(LateLog {
            hold_rows_until: Some(model.answered.clone()),
            ..LateLog::default()
        });
        svc.set_sink(log.clone());
        let err = svc
            .decide(
                Some(&public_chat("late-race")),
                None,
                &request("a note", None),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Timeout { .. }), "{err:?}");
        // The waiter handed it over, on a task of its own, before returning.
        log.first_late().await;
        assert_eq!(
            super::super::late::settle_late_answers(Duration::from_secs(10)).await,
            0
        );
        let late = log.late();
        assert_eq!(
            late.len(),
            1,
            "exactly one of the waiter and the task settles"
        );
        assert_eq!(Some(late[0].0.clone()), log.rows()[0].0);
        assert_eq!(late[0].1.usage, usage(30, 1));
    }

    /// A caller stopped while the answer it gave up on is being stored (a
    /// steer or Stop right after the timeout, an HTTP client that gave up)
    /// does not take the storing with it: the late row still lands, once,
    /// and the request counts in flight until it has.
    #[tokio::test]
    async fn a_caller_stopped_while_its_late_answer_is_stored_still_bills_it() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(60),
            usage(30, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(20), Duration::from_millis(20));
        let release = Arc::new(tokio::sync::Notify::new());
        let log = Arc::new(LateLog {
            // The answer is in the channel by the time the timeout row is
            // written, so the waiter is the one that hands it over.
            hold_rows_until: Some(model.answered.clone()),
            hold_late_until: Some(release.clone()),
            ..LateLog::default()
        });
        svc.set_sink(log.clone());
        let note = request("a note", None);
        let chat = public_chat("late-stopped-mid-settle");
        let returned = tokio::select! {
            decided = svc.decide(Some(&chat), None, &note) => Some(decided),
            // The late row's write has started while the caller still waits
            // on it: the caller is stopped here.
            () = log.late_started.notified() => None,
        };
        if let Some(decided) = returned {
            assert!(
                matches!(decided, Err(DecisionError::Timeout { .. })),
                "{decided:?}"
            );
            log.late_started.notified().await;
        }
        assert!(
            super::super::late::in_flight() >= 1,
            "the late answer counts in flight until it is stored"
        );
        release.notify_one();
        let (late_id, late) = log.first_late().await;
        assert_eq!(late.usage, usage(30, 1));
        assert_eq!(Some(late_id), log.rows()[0].0, "under the timeout row's id");
        assert_eq!(
            super::super::late::settle_late_answers(Duration::from_secs(10)).await,
            0
        );
        assert_eq!(log.late().len(), 1, "handed over once");
    }

    /// A caller that goes away while its timeout row is being written, after
    /// the model answered, leaves the answer in the channel: it is still
    /// stored and billed, once.
    #[tokio::test]
    async fn a_caller_that_goes_away_while_the_timeout_row_is_written_still_bills_the_answer() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(60),
            usage(30, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(20), Duration::from_millis(20));
        let log = Arc::new(LateLog {
            hold_rows_until: Some(model.answered.clone()),
            stall_rows: true,
            ..LateLog::default()
        });
        svc.set_sink(log.clone());
        let stalled = log.row_stalled.notified();
        let note = request("a note", None);
        let chat = public_chat("late-gone-mid-row");
        tokio::select! {
            decided = svc.decide(Some(&chat), None, &note) => {
                panic!("the timeout row never finishes: {decided:?}")
            }
            () = stalled => {}
        }
        let (_, late) = log.first_late().await;
        assert_eq!(late.usage, usage(30, 1));
        assert!(
            late.error
                .as_deref()
                .is_some_and(|text| text.contains("too late to use")),
            "{late:?}"
        );
        assert_eq!(
            super::super::late::settle_late_answers(Duration::from_secs(10)).await,
            0
        );
        assert_eq!(log.late().len(), 1, "handed over once");
        assert!(log.rows().is_empty(), "the row's write was abandoned");
    }

    /// A caller that goes away (a stopped turn, a dropped HTTP request)
    /// leaves no row, but the model's answer is still stored and billed,
    /// saying the request was abandoned.
    #[tokio::test]
    async fn a_cancelled_decision_still_bills_what_the_model_answered() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(100),
            usage(30, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_secs(5), Duration::from_secs(5));
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());
        let chat = public_chat("late-cancelled");
        // The caller goes away once the model has been asked, well inside the
        // deadline (a fixed delay could fire before the request is sent, on a
        // loaded machine).
        let asked = model.asked.notified();
        let note = request("a note", None);
        tokio::select! {
            decided = svc.decide(Some(&chat), None, &note) => {
                panic!("the caller was meant to go away first: {decided:?}")
            }
            () = asked => {}
        }
        let (_, late) = log.first_late().await;
        assert_eq!(late.usage, usage(30, 1));
        let text = late.error.as_deref().unwrap();
        assert!(
            text.starts_with(
                "The request was abandoned before slow answered (the turn was stopped or the \
                 caller went away), so its answer was not used. It answered after "
            ) && text.ends_with("s; its tokens are counted in Usage."),
            "{text}"
        );
        assert!(log.rows().is_empty(), "no row was written by the caller");
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    }

    /// A call whose task panics fails as a backend error, logged under the
    /// request's id, and is no longer counted in flight.
    #[tokio::test]
    async fn a_decision_whose_call_panics_fails_as_a_backend_error() {
        struct Panics(DecisionModelEntry);
        #[async_trait]
        impl DecisionModel for Panics {
            fn model_ref(&self) -> DecisionModelRef {
                self.0.model_ref()
            }
            fn display_name(&self) -> String {
                self.0.display_name.clone()
            }
            fn tier(&self) -> ProviderTier {
                self.0.tier
            }
            fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
                self.0.affiliation
            }
            fn caps(&self) -> DecisionCaps {
                DecisionCaps {
                    kinds: self.0.kinds.clone(),
                    max_questions: 64,
                    context_tokens: self.0.context_tokens,
                    max_concurrency: 1,
                    max_request_bytes: None,
                    local: false,
                }
            }
            async fn decide(&self, _: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
                panic!("the adapter crashed");
            }
        }
        struct OnePanics(DecisionModelEntry);
        #[async_trait]
        impl DecisionModelSource for OnePanics {
            async fn list(&self) -> Vec<DecisionModelEntry> {
                vec![self.0.clone()]
            }
            async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
                Ok(Arc::new(Panics(self.0.clone())))
            }
        }
        let svc = DecisionService::new(Arc::new(OnePanics(entry(
            "crashy",
            ProviderTier::Public,
            false,
        ))))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None);
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());
        let err = svc
            .decide(Some(&public_chat("panics")), None, &request("x", None))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            DecisionError::Backend {
                model: "crashy".into(),
                status: None,
                error_type: None,
                message: "the decision request stopped unexpectedly".into(),
            }
        );
        let rows = log.rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].0.is_some(), "stored under the request's id");
        assert_eq!(rows[0].1.outcome, DecisionOutcome::Error);
        assert!(log.late().is_empty());
    }

    /// A late answer with no usage, or about no chat, writes nothing.
    #[tokio::test]
    async fn a_late_answer_with_no_usage_writes_nothing() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(80),
            DecisionUsage::default(),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(10), Duration::from_millis(10));
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());
        assert!(svc
            .decide(
                Some(&public_chat("late-unbilled")),
                None,
                &request("x", None)
            )
            .await
            .is_err());
        let billed = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(80),
            usage(30, 1),
        ));
        let no_chat = scripted_service(billed.clone())
            .with_timeouts(Duration::from_millis(10), Duration::from_millis(10));
        no_chat.set_sink(log.clone());
        assert!(no_chat
            .decide(None, None, &request("x", None))
            .await
            .is_err());
        assert_eq!(
            super::super::late::settle_late_answers(Duration::from_secs(10)).await,
            0
        );
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(billed.calls.load(Ordering::SeqCst), 1);
        assert!(log.late().is_empty(), "{:?}", log.late());
    }

    /// A priced late answer's cost reaches the turn it belongs to while that
    /// turn is live, and never the tally; once the turn has ended it reaches
    /// no turn.
    #[tokio::test]
    async fn a_late_priced_answer_counts_against_the_live_turn_only() {
        let mut joey = entry("joey-1", ProviderTier::Public, false);
        joey.provider = "baseten_decision".into();
        let model = Arc::new(Scripted::new(
            joey,
            Duration::from_millis(80),
            usage(30_000, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(10), Duration::from_millis(10));
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());

        let live_turn = "turn-late-priced-live";
        crate::decision::turns::begin_turn("late-priced", live_turn);
        let chat = public_chat("late-priced").with_turn(Some(live_turn.into()));
        assert!(svc
            .decide(Some(&chat), None, &request("x", None))
            .await
            .is_err());
        log.first_late().await;
        let cost = crate::decision::turns::take_late_cost(live_turn);
        assert!(cost > 0.0, "{cost}");
        let tally = super::super::monitor::take_turn_tally(live_turn);
        assert_eq!(tally.len(), 1, "{tally:?}");
        assert_eq!(
            (tally[0].count, tally[0].errors),
            (1, 1),
            "the timeout row is tallied once; the late answer is not"
        );
        crate::decision::turns::end_turn("late-priced", live_turn);

        // The turn ends before the answer comes: no turn is charged.
        let ended_turn = "turn-late-priced-ended";
        crate::decision::turns::begin_turn("late-priced-2", ended_turn);
        let chat = public_chat("late-priced-2").with_turn(Some(ended_turn.into()));
        assert!(svc
            .decide(Some(&chat), None, &request("x", None))
            .await
            .is_err());
        crate::decision::turns::end_turn("late-priced-2", ended_turn);
        assert_eq!(
            super::super::late::settle_late_answers(Duration::from_secs(10)).await,
            0
        );
        assert_eq!(log.late().len(), 2, "still stored and billed");
        assert_eq!(crate::decision::turns::take_late_cost(ended_turn), 0.0);
        let _ = super::super::monitor::take_turn_tally(ended_turn);
    }

    /// An exiting process waits for requests still in flight, up to its cap,
    /// and is told how many it could not wait for.
    #[tokio::test]
    async fn settle_late_answers_waits_for_calls_in_flight() {
        let model = Arc::new(Scripted::new(
            entry("slow", ProviderTier::Public, false),
            Duration::from_millis(150),
            usage(30, 1),
        ));
        let svc = scripted_service(model.clone())
            .with_timeouts(Duration::from_millis(10), Duration::from_millis(10));
        let log = Arc::new(LateLog::default());
        svc.set_sink(log.clone());
        assert!(svc
            .decide(Some(&public_chat("late-settle")), None, &request("x", None))
            .await
            .is_err());
        assert!(
            super::super::late::in_flight() >= 1,
            "the call is still running"
        );
        assert!(
            super::super::late::settle_late_answers(Duration::ZERO).await >= 1,
            "no time to wait: it is still in flight"
        );
        assert_eq!(
            super::super::late::settle_late_answers(Duration::from_secs(10)).await,
            0
        );
        assert_eq!(log.late().len(), 1, "it settled before the wait returned");
    }

    // ── save-time checks ───────────────────────────────────────────────────

    fn monitor(value: serde_json::Value) -> MonitorCriterion {
        let mut base = serde_json::json!({
            "id": "done",
            "name": "Done",
            "question": {"instructions": "Is the task done?", "kind": {"binary": {}}},
            "model": null,
            "threshold": 0.8,
            "option": null,
            "rearmBelow": null
        });
        for (key, field) in value.as_object().unwrap() {
            base[key] = field.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn named(model: &str) -> serde_json::Value {
        serde_json::json!({"provider": "test_decision", "model": model})
    }

    fn settings_of(default: Option<&str>, monitors: Vec<MonitorCriterion>) -> DecisionSettings {
        DecisionSettings {
            default_model: default.map(|model| DecisionModelRef::new("test_decision", model)),
            monitors,
        }
    }

    fn score_question() -> serde_json::Value {
        serde_json::json!({
            "instructions": "How far along is it?",
            "kind": {"score": {"levels": ["none", "some", "all"]}}
        })
    }

    #[tokio::test]
    async fn a_save_check_refuses_a_public_model_for_a_private_chat_and_sends_nothing() {
        let (svc, calls) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            Duration::ZERO,
        );
        let settings = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("cloud")}))],
        );
        let why = svc
            .check_settings(&private_chat(), &settings, None)
            .await
            .unwrap_err();
        assert_eq!(
            why,
            "Nothing was saved: monitor \"Done\" could not run. This chat is private, so this \
             decision cannot run on `cloud`, which is a public model. Choose a decision model \
             that runs on this computer, or start this work in a public chat."
        );
        let local = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("local")}))],
        );
        assert_eq!(
            svc.check_settings(&private_chat(), &local, None).await,
            Ok(())
        );
        // The same check through the one-model door.
        let err = svc
            .check_model_use(
                &private_chat(),
                Some(&DecisionModelRef::new("test_decision", "cloud")),
                &request("", None).questions,
                ModelSource::Setting,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing is sent");
    }

    #[tokio::test]
    async fn a_save_check_refuses_a_model_that_is_not_there_cannot_take_the_kind_or_will_not_load()
    {
        let mut stale = entry("stale", ProviderTier::Private, true);
        stale.load_refusal = Some("Ollama cannot load this copy of `stale`; pull it again.".into());
        let (svc, calls) = service(
            vec![entry("local", ProviderTier::Private, true), stale],
            Duration::ZERO,
        );
        let chat = public_chat("save-check-models");
        let gone = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("gone")}))],
        );
        assert_eq!(
            svc.check_settings(&chat, &gone, None).await.unwrap_err(),
            "Nothing was saved: monitor \"Done\" could not run. unknown."
        );
        let scored = settings_of(
            None,
            vec![monitor(serde_json::json!({
                "model": named("local"),
                "question": score_question(),
                "option": "some"
            }))],
        );
        let why = svc.check_settings(&chat, &scored, None).await.unwrap_err();
        assert!(
            why.starts_with(
                "Nothing was saved: monitor \"Done\" could not run. `local` cannot \
                             answer question `done`: it does not take Score questions"
            ),
            "{why}"
        );
        let unloadable = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("stale")}))],
        );
        assert_eq!(
            svc.check_settings(&chat, &unloadable, None)
                .await
                .unwrap_err(),
            "Nothing was saved: monitor \"Done\" could not run. Ollama cannot load this copy of \
             `stale`; pull it again."
        );
        // The chat's default decision model is named when it is what fails.
        let default_gone = settings_of(Some("gone"), vec![]);
        assert_eq!(
            svc.check_settings(&chat, &default_gone, None)
                .await
                .unwrap_err(),
            "Nothing was saved: the chat's default decision model test_decision/gone could not \
             run. unknown."
        );
        // A listed default its server will not load is refused too, with no
        // monitor running on it, and so is one an unnamed monitor would use.
        let default_stale = settings_of(Some("stale"), vec![]);
        let stale_default = "Nothing was saved: the chat's default decision model \
                             test_decision/stale could not run. Ollama cannot load this copy \
                             of `stale`; pull it again.";
        assert_eq!(
            svc.check_settings(&chat, &default_stale, None)
                .await
                .unwrap_err(),
            stale_default
        );
        let unnamed_on_stale = settings_of(Some("stale"), vec![monitor(serde_json::json!({}))]);
        assert_eq!(
            svc.check_settings(&chat, &unnamed_on_stale, None)
                .await
                .unwrap_err(),
            stale_default
        );
        assert!(matches!(
            svc.check_model_use(
                &chat,
                Some(&DecisionModelRef::new("test_decision", "stale")),
                &IndexMap::new(),
                ModelSource::Setting,
            )
            .await,
            Err(DecisionError::Unsupported(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_save_check_refuses_an_unnamed_monitor_no_model_may_serve() {
        let (svc, calls) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let unnamed = settings_of(None, vec![monitor(serde_json::json!({}))]);
        let why = svc
            .check_settings(&private_chat(), &unnamed, None)
            .await
            .unwrap_err();
        assert!(
            why.starts_with("Nothing was saved: monitor \"Done\" could not run. No decision model this private chat may use is available"),
            "{why}"
        );
        assert!(why.contains(INSTALL_ADVICE), "{why}");
        // Off, it is not asked.
        let off = settings_of(None, vec![monitor(serde_json::json!({"enabled": false}))]);
        assert_eq!(
            svc.check_settings(&private_chat(), &off, None).await,
            Ok(())
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// Only what the save turns on is asked, so a monitor whose model went
    /// away can always be switched off, renamed or removed.
    #[tokio::test]
    async fn a_save_check_asks_only_what_the_save_turns_on() {
        let (svc, calls) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            Duration::ZERO,
        );
        let chat = private_chat();
        let broken =
            monitor(serde_json::json!({"id": "broken", "name": "Broken", "model": named("gone")}));
        let stored = settings_of(None, vec![broken.clone()]);

        let good =
            monitor(serde_json::json!({"id": "good", "name": "Good", "model": named("local")}));
        let added = settings_of(None, vec![broken.clone(), good.clone()]);
        assert_eq!(
            svc.check_settings(&chat, &added, Some(&stored)).await,
            Ok(())
        );

        let mut off = broken.clone();
        off.enabled = false;
        assert_eq!(
            svc.check_settings(&chat, &settings_of(None, vec![off.clone()]), Some(&stored))
                .await,
            Ok(())
        );
        assert_eq!(
            svc.check_settings(&chat, &settings_of(None, vec![]), Some(&stored))
                .await,
            Ok(()),
            "removing it"
        );
        let mut renamed = broken.clone();
        renamed.name = "Still broken".into();
        assert_eq!(
            svc.check_settings(&chat, &settings_of(None, vec![renamed]), Some(&stored))
                .await,
            Ok(())
        );

        let mut moved = broken.clone();
        moved.model = Some(DecisionModelRef::new("test_decision", "cloud"));
        let why = svc
            .check_settings(&chat, &settings_of(None, vec![moved]), Some(&stored))
            .await
            .unwrap_err();
        assert!(
            why.starts_with(
                "Nothing was saved: monitor \"Broken\" could not run. This chat is private"
            ),
            "{why}"
        );

        let stored_off = settings_of(None, vec![off.clone()]);
        let why = svc
            .check_settings(
                &chat,
                &settings_of(None, vec![broken.clone()]),
                Some(&stored_off),
            )
            .await
            .unwrap_err();
        assert_eq!(
            why,
            "Nothing was saved: monitor \"Broken\" could not run. unknown."
        );

        // A monitor that names no model runs on the chat's default, so
        // changing the default asks it again.
        let unnamed = monitor(serde_json::json!({"id": "watch", "name": "Watch"}));
        let on_local = settings_of(Some("local"), vec![unnamed.clone()]);
        assert_eq!(svc.check_settings(&chat, &on_local, None).await, Ok(()));
        let to_cloud = settings_of(Some("cloud"), vec![unnamed.clone()]);
        let why = svc
            .check_settings(&chat, &to_cloud, Some(&on_local))
            .await
            .unwrap_err();
        assert!(
            why.starts_with(
                "Nothing was saved: the chat's default decision model \
                             test_decision/cloud could not run. This chat is private"
            ),
            "{why}"
        );
        // A default left as it was is not asked again.
        assert_eq!(
            svc.check_settings(&chat, &on_local, Some(&on_local)).await,
            Ok(())
        );
        // A kind change asks it again.
        let mut rescored = unnamed.clone();
        rescored.question = serde_json::from_value(score_question()).unwrap();
        rescored.option = Some("some".into());
        let why = svc
            .check_settings(
                &chat,
                &settings_of(Some("local"), vec![rescored]),
                Some(&on_local),
            )
            .await
            .unwrap_err();
        assert!(why.contains("does not take Score questions"), "{why}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A gate that refuses the whole chat (a Crew grant that no longer
    /// stands) refuses any monitor a save turns on, and nothing else.
    #[tokio::test]
    async fn a_crew_chat_refuses_any_monitor_turned_on_but_lets_one_be_turned_off() {
        struct Stopped;
        #[async_trait]
        impl DecisionUseGate for Stopped {
            async fn check(
                &self,
                _: &DecisionSession,
                _: &dyn DecisionModel,
            ) -> DecisionResult<()> {
                Err(DecisionError::Refused(
                    "This chat's Crew access was removed.".into(),
                ))
            }
            async fn session_refusal(&self, _: &DecisionSession) -> Option<String> {
                Some("This chat's Crew access was removed.".into())
            }
        }
        let (svc, calls) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let svc = svc.with_use_gate(Arc::new(Stopped));
        let chat = public_chat("crew-save");
        let on = monitor(serde_json::json!({"model": named("local")}));
        assert_eq!(
            svc.check_settings(&chat, &settings_of(None, vec![on.clone()]), None)
                .await
                .unwrap_err(),
            "Nothing was saved. This chat's Crew access was removed."
        );
        let mut off = on.clone();
        off.enabled = false;
        let stored = settings_of(None, vec![on]);
        assert_eq!(
            svc.check_settings(&chat, &settings_of(None, vec![off]), Some(&stored))
                .await,
            Ok(())
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_institution_the_model_does_not_cover_is_refused_at_save() {
        let (svc, calls) = service(
            vec![entry("hosted", ProviderTier::Private, false)],
            Duration::ZERO,
        );
        let chat = private_chat().with_affiliations(BTreeSet::from([InstitutionId::new("ucsf")]));
        let settings = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("hosted")}))],
        );
        let why = svc
            .check_settings(&chat, &settings, None)
            .await
            .unwrap_err();
        assert!(
            why.starts_with(
                "Nothing was saved: monitor \"Done\" could not run. This chat holds \
                             private data from an institution `hosted` is not covered by"
            ),
            "{why}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A chat still Public but bound to a private model becomes private at
    /// its first turn, so a public decision model is refused at save. A
    /// decision itself never reads the bound tier.
    #[tokio::test]
    async fn a_fresh_chat_bound_to_a_private_model_refuses_a_public_decision_model_at_save() {
        let (svc, calls) = service(
            vec![
                entry("cloud", ProviderTier::Public, false),
                entry("local", ProviderTier::Private, true),
            ],
            Duration::ZERO,
        );
        let fresh = public_chat("bound-private").with_bound_tier(Some(ProviderTier::Private));
        let cloud = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("cloud")}))],
        );
        assert_eq!(
            svc.check_settings(&fresh, &cloud, None).await.unwrap_err(),
            "Nothing was saved: monitor \"Done\" could not run. This chat runs on a private \
             model, so it becomes private at its first turn, and `cloud` is a public model a \
             private chat cannot use. Choose a decision model that runs on this computer."
        );
        let local = settings_of(
            None,
            vec![monitor(serde_json::json!({"model": named("local")}))],
        );
        assert_eq!(svc.check_settings(&fresh, &local, None).await, Ok(()));
        let bound_public = public_chat("bound-public").with_bound_tier(Some(ProviderTier::Public));
        assert_eq!(
            svc.check_settings(&bound_public, &cloud, None).await,
            Ok(())
        );
        let unknown = public_chat("bound-unknown");
        assert_eq!(svc.check_settings(&unknown, &cloud, None).await, Ok(()));
        let off = DecisionSession::new("bound-off", SessionClassification::Public, false)
            .with_bound_tier(Some(ProviderTier::Private));
        assert_eq!(
            svc.check_settings(&off, &cloud, None).await,
            Ok(()),
            "the master switch as sampled"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // A decision does not read it.
        svc.decide(
            Some(&fresh),
            Some(&DecisionModelRef::new("test_decision", "cloud")),
            &request("x", None),
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_decision_chat_question_is_checked_against_its_bound_model_only_when_its_kind_changes(
    ) {
        let (svc, calls) = service(
            vec![entry("local", ProviderTier::Private, true)],
            Duration::ZERO,
        );
        let chat = public_chat("decision-chat-save");
        let local = DecisionModelRef::new("test_decision", "local");
        let binary = request("", None).questions["done"].clone();
        let score: Question = serde_json::from_value(score_question()).unwrap();
        assert_eq!(
            svc.check_chat_question(&chat, None, &score, None).await,
            Ok(()),
            "a chat bound to no decision model is not asked"
        );
        assert_eq!(
            svc.check_chat_question(&chat, Some(&local), &binary, None)
                .await,
            Ok(())
        );
        let why = svc
            .check_chat_question(&chat, Some(&local), &score, None)
            .await
            .unwrap_err();
        assert!(
            why.starts_with(
                "Nothing was saved: this decision chat's model test_decision/local \
                             could not answer this question. `local` cannot answer question"
            ),
            "{why}"
        );
        assert!(svc
            .check_chat_question(&chat, Some(&local), &score, Some(&binary))
            .await
            .is_err());
        let mut reworded = score.clone();
        reworded.instructions = "How much is done?".into();
        assert_eq!(
            svc.check_chat_question(&chat, Some(&local), &reworded, Some(&score))
                .await,
            Ok(()),
            "the same kind is not asked again"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // ── Crew ───────────────────────────────────────────────────────────────

    /// The production gate over a listing the test supplies.
    fn crew_service(entries: Vec<DecisionModelEntry>) -> (DecisionService, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let source = Source {
            entries,
            calls: calls.clone(),
            delay: Duration::ZERO,
        };
        (
            DecisionService::new(Arc::new(source))
                .with_use_gate(Arc::new(CrewUseGate))
                .with_default_model(None),
            calls,
        )
    }

    fn local_ollama() -> DecisionModelEntry {
        DecisionModelEntry {
            provider: "ollama_decision".into(),
            model: "tev1:0.8b".into(),
            display_name: "tev1:0.8b".into(),
            context_tokens: 4_096,
            kinds: vec![DecisionKindTag::Binary],
            tier: ProviderTier::Private,
            affiliation: Some(crate::privacy::affiliation::ModelAffiliation::Local),
            adapter: DecisionAdapter::SystemOne,
            local: true,
            load_refusal: None,
        }
    }

    fn baseten() -> DecisionModelEntry {
        DecisionModelEntry {
            provider: "baseten_decision".into(),
            model: "joey-1".into(),
            display_name: "joey-1".into(),
            context_tokens: 4_096,
            kinds: vec![DecisionKindTag::Binary],
            tier: ProviderTier::Public,
            affiliation: None,
            adapter: DecisionAdapter::SystemOne,
            local: false,
            load_refusal: None,
        }
    }

    fn crew_session_id() -> String {
        format!("crew-decision-{}", uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn crew_chats_reach_decision_models_on_this_computer_only() {
        let id = crew_session_id();
        let manager = crate::crew::install_test_scope(&id, None).await;
        crate::crew::stand_test_scope(&manager, &id).await;
        let (svc, calls) = crew_service(vec![baseten(), local_ollama()]);
        let chat = public_chat(&id);

        assert_eq!(svc.session_refusal(&chat).await, None, "the grant stands");
        let listed = svc.models_for(Some(&chat)).await;
        let reason = |model: &str| {
            listed
                .iter()
                .find(|(entry, _)| entry.model == model)
                .map(|(_, why)| why.clone())
                .unwrap()
        };
        assert_eq!(reason("tev1:0.8b"), None);
        assert_eq!(reason("joey-1").as_deref(), Some(CREW_SCOPED_REFUSAL));

        let unnamed = svc
            .decide(Some(&chat), None, &request("plate_counts.csv", None))
            .await
            .unwrap();
        assert_eq!(unnamed[0].model.model, "tev1:0.8b");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the local model was asked once"
        );
        let err = svc
            .decide(
                Some(&chat),
                Some(&DecisionModelRef::new("baseten_decision", "joey-1")),
                &request("plate_counts.csv", None),
            )
            .await
            .unwrap_err();
        assert_eq!(err, DecisionError::Refused(CREW_SCOPED_REFUSAL.to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "nothing reached Baseten");
        crate::crew::remove_test_scope(&manager, &id).await;
    }

    #[tokio::test]
    async fn crew_counts_an_ollama_cloud_model_as_elsewhere() {
        let id = crew_session_id();
        let manager = crate::crew::install_test_scope(&id, None).await;
        crate::crew::stand_test_scope(&manager, &id).await;
        let mut cloud = local_ollama();
        cloud.model = "gpt-oss:20b-cloud".into();
        cloud.display_name = "gpt-oss:20b-cloud".into();
        cloud.tier = ProviderTier::Public;
        cloud.affiliation = None;
        cloud.local = false;
        let (svc, calls) = crew_service(vec![cloud]);
        let chat = public_chat(&id);
        let listed = svc.models_for(Some(&chat)).await;
        assert_eq!(listed[0].1.as_deref(), Some(CREW_SCOPED_REFUSAL));
        let err = svc
            .decide(
                Some(&chat),
                Some(&DecisionModelRef::new(
                    "ollama_decision",
                    "gpt-oss:20b-cloud",
                )),
                &request("x", None),
            )
            .await
            .unwrap_err();
        assert_eq!(err, DecisionError::Refused(CREW_SCOPED_REFUSAL.to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        crate::crew::remove_test_scope(&manager, &id).await;
    }

    #[tokio::test]
    async fn a_crew_grant_that_no_longer_stands_refuses_every_model() {
        let id = crew_session_id();
        let manager = crate::crew::install_test_scope(&id, None).await;
        crate::crew::stand_test_scope(&manager, &id).await;
        crate::crew::expire_test_scope(&manager, &id).await;
        let (svc, calls) = crew_service(vec![baseten(), local_ollama()]);
        let chat = public_chat(&id);
        let stopped = svc
            .session_refusal(&chat)
            .await
            .expect("a grant that no longer stands refuses the chat");
        assert_ne!(stopped, CREW_SCOPED_REFUSAL);
        assert!(svc
            .models_for(Some(&chat))
            .await
            .iter()
            .all(|(_, why)| why.as_deref() == Some(stopped.as_str())));
        let err = svc
            .decide(
                Some(&chat),
                Some(&DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
                &request("x", None),
            )
            .await
            .unwrap_err();
        assert_eq!(err, DecisionError::Refused(stopped));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        crate::crew::remove_test_scope(&manager, &id).await;
    }

    #[tokio::test]
    async fn a_crew_chat_with_only_hosted_models_is_told_to_install_one_here() {
        let id = crew_session_id();
        let manager = crate::crew::install_test_scope(&id, None).await;
        crate::crew::stand_test_scope(&manager, &id).await;
        let (svc, calls) = crew_service(vec![baseten()]);
        let chat = public_chat(&id);
        let err = svc
            .decide(Some(&chat), None, &request("x", None))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            DecisionError::Refused(format!(
                "{CREW_SCOPED_REFUSAL} No decision model on this computer is installed. \
                 {INSTALL_ADVICE}"
            ))
        );
        assert_eq!(svc.model_entry_for(Some(&chat), None).await, None);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        crate::crew::remove_test_scope(&manager, &id).await;
    }

    /// A Crew chat whose decision provider is configured as a hosted one,
    /// with a decision model installed on this computer, is not told to
    /// install one: it is told the configured provider is what rules the
    /// local model out, and how to reach it.
    #[tokio::test]
    async fn a_crew_chat_whose_configured_provider_is_hosted_is_pointed_at_the_local_model() {
        let id = crew_session_id();
        let manager = crate::crew::install_test_scope(&id, None).await;
        crate::crew::stand_test_scope(&manager, &id).await;
        let (svc, calls) = crew_service(vec![baseten(), local_ollama()]);
        let svc = svc.with_default_provider("baseten_decision");
        let chat = public_chat(&id);
        let err = svc
            .decide(Some(&chat), None, &request("x", None))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            DecisionError::Refused(format!(
                "{CREW_SCOPED_REFUSAL} A decision that names no model uses the configured \
                 decision provider, `baseten_decision`, which has none this chat may use. Name \
                 `ollama_decision/tev1:0.8b`, or clear BIOROUTER_DECISION_PROVIDER."
            ))
        );
        assert!(
            !err.to_string().contains("is installed"),
            "the local model is installed: {err}"
        );
        // Named, the local model answers.
        let named = svc
            .decide(
                Some(&chat),
                Some(&DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
                &request("x", None),
            )
            .await
            .unwrap();
        assert_eq!(named[0].model.model, "tev1:0.8b");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "nothing reached Baseten");
        crate::crew::remove_test_scope(&manager, &id).await;
    }

    #[tokio::test]
    async fn a_chat_without_a_crew_grant_is_unchanged() {
        let (svc, calls) = crew_service(vec![baseten()]);
        let chat = public_chat(&crew_session_id());
        assert_eq!(svc.session_refusal(&chat).await, None);
        assert!(svc
            .models_for(Some(&chat))
            .await
            .iter()
            .all(|(_, why)| why.is_none()));
        let chosen = svc
            .decide(Some(&chat), None, &request("x", None))
            .await
            .unwrap();
        assert_eq!(chosen[0].model.model, "joey-1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_private_no_model_sentence_names_the_install() {
        let (svc, _) = service(vec![], Duration::ZERO);
        let err = svc
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            DecisionError::NoModel(format!(
                "No decision model this private chat may use is available: only a decision \
                 model on this computer may read a private chat. {INSTALL_ADVICE} Or set \
                 BIOROUTER_DECISION_MODEL to a private one."
            ))
        );
        let err = svc
            .decide(None, None, &request("x", None))
            .await
            .unwrap_err();
        assert!(err.to_string().ends_with(INSTALL_ADVICE), "{err}");
        let (only_public, _) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let err = only_public
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap_err();
        assert!(err.to_string().contains(INSTALL_ADVICE), "{err}");
    }

    #[tokio::test]
    async fn an_unreachable_ollama_is_named_instead_of_install_advice() {
        struct Down;
        #[async_trait]
        impl DecisionModelSource for Down {
            async fn list(&self) -> Vec<DecisionModelEntry> {
                Vec::new()
            }
            async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
                Err(DecisionError::NoModel("unknown".into()))
            }
            async fn unavailable_hint(&self) -> Option<String> {
                Some(
                    "Ollama is not answering at http://127.0.0.1:11434/, so no local decision \
                     model could be listed. Start Ollama, then try again."
                        .into(),
                )
            }
        }
        let svc = DecisionService::new(Arc::new(Down))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let err = svc
            .decide(Some(&private_chat()), None, &request("x", None))
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("Ollama is not answering at http://127.0.0.1:11434/"),
            "{text}"
        );
        assert!(!text.contains(INSTALL_ADVICE), "{text}");
    }
}
