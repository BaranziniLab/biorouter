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
//! A Crew grant fixes the model a chat's content may reach
//! (`CrewManager::check_provider_binding` refuses any other provider as
//! "model fixed"), so the default [`CrewUseGate`] refuses a decision in a
//! Crew-scoped chat outright, which is what `check_provider_use` would answer
//! for a model that is not the grant's.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use indexmap::IndexMap;
use tokio::sync::Semaphore;

use super::error::{DecisionError, DecisionResult};
use super::model::{DecisionCaps, DecisionModel, DecisionModelEntry, DecisionModelSource};
use super::types::{
    decision_from_verdict, validate_questions, ChunkOutcome, Decision, DecisionChunk,
    DecisionContext, DecisionKind, DecisionMeta, DecisionModelRef, DecisionOrigin, DecisionRequest,
    DecisionUsage, ModelVerdicts, Question,
};
use crate::config::Config;
use crate::privacy::affiliation::{owners_compatible, InstitutionId};
use crate::privacy::alt_provider::assert_alt_tier_allowed;
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
/// The env key a refusal names as the way out.
const MODEL_KEY: &str = "BIOROUTER_DECISION_MODEL";

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
}

impl std::fmt::Debug for DecisionSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionSession")
            .field("session_id", &self.session_id)
            .field("classification", &self.classification)
            .field("enforced", &self.enforced)
            .field("origin", &self.origin)
            .field("live", &self.live.is_some())
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
        }
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
}

/// Refuses a decision in a Crew-scoped chat: a grant fixes the one model the
/// chat's content may reach.
#[derive(Debug, Default, Clone, Copy)]
pub struct CrewUseGate;

#[async_trait]
impl DecisionUseGate for CrewUseGate {
    async fn check(
        &self,
        session: &DecisionSession,
        _model: &dyn DecisionModel,
    ) -> DecisionResult<()> {
        let manager = crate::crew::manager().map_err(|e| {
            DecisionError::Refused(format!(
                "The Crew registry could not be read, so no decision model was asked: {e}"
            ))
        })?;
        if manager.is_scoped_session(&session.session_id).await {
            return Err(DecisionError::Refused(
                "This chat works under a Crew grant, which fixes the one model its content may \
                 reach, so a decision model cannot read it."
                    .to_string(),
            ));
        }
        Ok(())
    }
}

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
}

/// Which model to use when the caller names none.
#[derive(Debug, Clone)]
enum DefaultModel {
    /// `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL`, read per call.
    FromConfig,
    Fixed(Option<DecisionModelRef>),
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
    model: &'a dyn DecisionModel,
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
    /// `None` when it may: Gate H's downward rule and affiliation, answered by
    /// the same check [`Self::decide`] asks before it sends anything, then
    /// the server's own refusal to load the model
    /// ([`DecisionModelEntry::load_refusal`]), which holds with or without a
    /// session. With no session every model that loads is usable. Advisory
    /// only, for pickers and listings: the gate is asked again, on the
    /// classification as it stands then, before every call, and a model the
    /// caller names is still opened and asked.
    pub async fn models_for(
        &self,
        session: Option<&DecisionSession>,
    ) -> Vec<(super::model::DecisionModelEntry, Option<String>)> {
        self.models()
            .await
            .into_iter()
            .map(|entry| {
                let reason = session
                    .and_then(|session| {
                        privacy_allows(session, &entry.display_name, entry.tier, entry.affiliation)
                            .err()
                            .map(|error| error.to_string())
                    })
                    .or_else(|| entry.load_refusal.clone());
                (entry, reason)
            })
            .collect()
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
        self.decide_single(model, request, Attempt::single(session, None))
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
        self.decide_single(model, request, Attempt::single(session, Some(request_id)))
            .await
    }

    async fn decide_single(
        &self,
        model: Option<&DecisionModelRef>,
        request: &DecisionRequest,
        attempt: Attempt<'_>,
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
        self.decide_one(resolved.as_ref(), request, attempt).await
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
                self.check_privacy(session, resolved.as_ref()).await?;
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
            model: resolved.as_ref(),
            questions,
            batch_id,
            stopped: Mutex::new(Stopped::default()),
        };
        let outcomes = futures::stream::iter(chunks.into_iter().enumerate())
            .map(|(index, chunk)| self.decide_chunk(&batch, index, chunk))
            .buffered(parallel)
            .collect::<Vec<_>>()
            .await;
        let Stopped {
            error,
            chunks: mut held,
        } = batch
            .stopped
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(error) = &error {
            held.sort_by_key(|(index, _)| *index);
            let ids: Vec<&str> = held.iter().map(|(_, id)| id.as_str()).collect();
            let request = DecisionRequest {
                questions: questions.clone(),
                context: DecisionContext::Text(unsent_partway_text(total, &ids)),
            };
            let attempt = Attempt {
                session,
                batch_id: Some(batch_id),
                ..Attempt::default()
            };
            self.record_failure(
                session,
                Some(resolved.as_ref()),
                &request,
                error,
                Some(&attempt),
                DecisionUsage::default(),
            )
            .await;
        }
        Ok(outcomes)
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
                };
                self.decide_one(batch.model, &request, attempt).await
            }
        };
        match result {
            Ok(decisions) => ChunkOutcome {
                chunk_id: chunk.id,
                decisions: Some(decisions),
                error: None,
            },
            Err(error) => {
                if matches!(error, DecisionError::Refused(_)) {
                    batch.stop(index, &chunk.id, &error);
                }
                ChunkOutcome {
                    chunk_id: chunk.id,
                    decisions: None,
                    error: Some(error.to_string()),
                }
            }
        }
    }

    /// The model named, else the chat's own default, else the configured
    /// default, else the first available model this chat may use, local first.
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
        match automatic_choice(session, &entries, provider.as_deref()) {
            Some(entry) => self.source.open(&entry.model_ref()).await,
            None => {
                let candidates = || {
                    entries
                        .iter()
                        .filter(|entry| provider.as_deref().is_none_or(|p| entry.provider == p))
                };
                // A model the server will not load is not one a privacy rule
                // ruled out: with only such models listed there is no model.
                let listed = candidates().any(|entry| entry.load_refusal.is_none());
                let unloadable = candidates().find_map(|entry| entry.load_refusal.as_deref());
                Err(no_model_error(session, listed, unloadable))
            }
        }
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
            Some(named) => entries.iter().find(|entry| entry.answers_to(named)),
            None => automatic_choice(session, &entries, provider.as_deref()),
        };
        entry.cloned()
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
        model: &dyn DecisionModel,
        request: &DecisionRequest,
        attempt: Attempt<'_>,
    ) -> DecisionResult<Vec<Decision>> {
        let caps = model.caps();
        if let Err(error) = self.preflight(model, &caps, request, &attempt).await {
            let unsent = DecisionUsage::default();
            return Err(self.failed(model, request, &attempt, error, unsent).await);
        }
        let permit = self
            .semaphore(model, &caps)
            .acquire_owned()
            .await
            .map_err(|_| {
                DecisionError::NoModel("The decision service is shutting down.".to_string())
            })?;
        let called = self.call_model(model, &caps, request).await;
        drop(permit);
        let (verdicts, latency_ms) = match called {
            Ok(called) => called,
            // No answer, so no usage to report. A request that timed out may
            // still have been billed by the provider; that is not counted.
            Err(error) => {
                let unknown = DecisionUsage::default();
                return Err(self.failed(model, request, &attempt, error, unknown).await);
            }
        };
        let usage = verdicts.usage;
        let served_model = verdicts.served_model.clone();
        let request_id = attempt
            .request_id
            .map_or_else(|| uuid::Uuid::now_v7().to_string(), str::to_string);
        let decisions = match build_decisions(model, request, verdicts, latency_ms, &request_id) {
            Ok(decisions) => decisions,
            // The model answered, so the request was billed even though its
            // answer cannot be used: the failure row keeps its usage, and the
            // usage ledger counts it.
            Err(error) => return Err(self.failed(model, request, &attempt, error, usage).await),
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
        self.record(entry, Some(&request_id)).await;
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
            self.check_privacy(session, model).await?;
        }
        check_size(model, caps, request, attempt.chunk_id).await
    }

    /// One request, under its deadline: the verdicts and the latency in
    /// milliseconds. A local model that answered counts as warm from then on.
    async fn call_model(
        &self,
        model: &dyn DecisionModel,
        caps: &DecisionCaps,
        request: &DecisionRequest,
    ) -> DecisionResult<(ModelVerdicts, u64)> {
        let deadline = self.timeout_for(model, caps);
        let started = Instant::now();
        let called = tokio::time::timeout(deadline, model.decide(request)).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let verdicts = match called {
            Ok(result) => result?,
            Err(_) => {
                return Err(DecisionError::Timeout {
                    model: model.display_name(),
                    seconds: deadline.as_secs(),
                })
            }
        };
        if caps.local {
            if let Ok(mut warm) = self.warm.lock() {
                warm.insert(model.model_ref().label());
            }
        }
        Ok((verdicts, latency_ms))
    }

    /// Log one request's failure and hand the error back. Inside a batch a
    /// refusal is left to the batch, which logs one row for every chunk it
    /// stopped. `usage` is what the model reported when it answered before
    /// the request failed (an answer that cannot be used), else the default.
    async fn failed(
        &self,
        model: &dyn DecisionModel,
        request: &DecisionRequest,
        attempt: &Attempt<'_>,
        error: DecisionError,
        usage: DecisionUsage,
    ) -> DecisionError {
        if !(attempt.defer_refusal && matches!(error, DecisionError::Refused(_))) {
            self.record_failure(
                attempt.session,
                Some(model),
                request,
                &error,
                Some(attempt),
                usage,
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
        self.record(entry, attempt.and_then(|a| a.request_id)).await;
    }

    /// Hand `entry` to the sink, under `id` when the caller chose one.
    async fn record(&self, entry: DecisionLogEntry, id: Option<&str>) {
        // Counted per turn and origin in process, for the one transcript
        // marker a turn gets; never one row per call (owner rule A1).
        super::monitor::note_decision(&entry);
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

/// Gate H's downward rule on a declared tier, then affiliation.
fn privacy_allows(
    session: &DecisionSession,
    name: &str,
    tier: ProviderTier,
    affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
) -> DecisionResult<()> {
    assert_alt_tier_allowed(
        "this decision",
        name,
        tier,
        session.classification,
        session.enforced,
        MODEL_KEY,
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
                privacy_allows(session, &entry.display_name, entry.tier, entry.affiliation).is_ok()
            })
        })
}

/// Why the automatic choice found nothing. `listed` says whether any model
/// its server loads (of the configured provider, when one is set) was listed:
/// then every one was ruled out for this chat by a privacy rule, which is a
/// refusal, as it is for a model the call names. With nothing listed it is no
/// model at all. `unloadable` is the sentence for a listed model the server
/// will not load, which names its fix, so it is said too.
fn no_model_error(
    session: Option<&DecisionSession>,
    listed: bool,
    unloadable: Option<&str>,
) -> DecisionError {
    let private = session.is_some_and(|s| s.enforced && s.classification.is_private());
    let text = if private {
        "No decision model this private chat may use is available. Pull a decision model into a \
         local Ollama (for example `ollama pull tev1:0.8b`), or set BIOROUTER_DECISION_MODEL to \
         a private one."
    } else if listed && session.is_some() {
        "No listed decision model may read this chat: a privacy rule rules out every one. Pull \
         a decision model into a local Ollama, or set BIOROUTER_DECISION_MODEL to one this chat \
         may use."
    } else {
        "No decision model is available. Add a Baseten API key, pull a decision model into \
         Ollama, or set BIOROUTER_DECISION_MODEL."
    };
    let text = match unloadable {
        Some(unloadable) => format!("{text} {unloadable}"),
        None => text.to_string(),
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

/// The token estimate for a request: the state and every question's text,
/// through the shared o200k counter with the non-OpenAI margin.
pub async fn estimate_tokens(request: &DecisionRequest) -> usize {
    let mut text = request.context.render_text();
    for (name, question) in &request.questions {
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
    let raw = match crate::token_counter::shared_token_counter().await {
        Ok(counter) => counter.count_tokens(&text),
        // The tokenizer failed to load: the four-characters-a-token estimate
        // the context budget uses, which errs low, so the margin below matters.
        Err(_) => crate::context_budget::estimate_tokens(&text),
    };
    ((raw as f64) * TOKEN_ESTIMATE_MARGIN).ceil() as usize
        + PER_QUESTION_OVERHEAD * request.questions.len()
}

/// Room kept below a model's window for what the estimate cannot see: the
/// server's own framing and the answer positions.
pub fn context_margin(context_tokens: usize) -> usize {
    (context_tokens / 32).max(128)
}

/// The tokens a state may take beside `questions` in a model of
/// `context_tokens`, by the arithmetic the size check before sending applies
/// (its margin, its per-question overhead and its non-OpenAI factor), with
/// the questions' own text counted at three characters a token. For building
/// a state that fits, such as a monitor's window; the check still judges.
pub fn state_budget_tokens(context_tokens: usize, questions: &IndexMap<String, Question>) -> usize {
    let overhead = context_margin(context_tokens) + PER_QUESTION_OVERHEAD * questions.len();
    let estimated_room = context_tokens.saturating_sub(overhead) as f64 / TOKEN_ESTIMATE_MARGIN;
    let question_tokens: usize = questions
        .iter()
        .map(|(name, question)| (name.len() + question_text_len(question)).div_ceil(3))
        .sum();
    (estimated_room.floor() as usize).saturating_sub(question_tokens)
}

/// Bytes of a question's text as [`estimate_tokens`] renders it.
fn question_text_len(question: &Question) -> usize {
    let kind = match &question.kind {
        DecisionKind::Binary {} => 0,
        DecisionKind::Choice { options } => options
            .iter()
            .map(|(option, description)| {
                option.len() + 3 + description.as_ref().map_or(0, String::len)
            })
            .sum(),
        DecisionKind::Score { levels } => levels.iter().map(|level| level.len() + 1).sum(),
    };
    question.instructions.len() + 2 + kind
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
        let err = svc
            .decide_batch(
                Some(&private_chat()),
                Some(&named),
                &request("", None).questions,
                chunks(50),
                "batch-1",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");
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
        let session =
            DecisionSession::new("s", SessionClassification::Public, true).with_live(live.clone());
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
    }
}
