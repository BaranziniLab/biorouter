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
//! and ratchets nothing. A public chat may therefore use a private (local)
//! decision model, with the under-classification residual `alt_provider.rs`
//! records for those sites. Making that cell a refusal too (the symmetric
//! rule) is an owner decision the design leaves open; it would also keep
//! public chats off local Ollama decision models.
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
use super::model::{DecisionCaps, DecisionModel, DecisionModelSource};
use super::types::{
    decision_from_verdict, validate_questions, ChunkOutcome, Decision, DecisionChunk,
    DecisionContext, DecisionKind, DecisionMeta, DecisionModelRef, DecisionOrigin, DecisionRequest,
    DecisionUsage, Question,
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
    /// The most private classification whose content this decision carries:
    /// the chat's, or for a phase gate the most private of the parent and the
    /// children whose results it reads.
    pub classification: SessionClassification,
    /// The institutions whose private data the chat holds.
    pub affiliations: BTreeSet<InstitutionId>,
    /// The privacy master switch, sampled once by the caller for this call
    /// (`cap.enforced()` on a tool path, `privacy_tiers_enabled()` elsewhere).
    pub enforced: bool,
    pub origin: DecisionOrigin,
    /// The criterion, tool call or phase that asked.
    pub origin_id: Option<String>,
    /// Re-read before every request, so a chat raised to Private in the middle
    /// of a batch stops reaching a public model at the next chunk.
    pub live: Option<Arc<dyn LiveClassification>>,
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
            live: None,
        }
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
        Ok(Self::new(session_id, session.privacy_tier, enforced)
            .with_affiliations(affiliations)
            .with_live(manager))
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    pub batch_id: Option<String>,
    pub chunk_index: Option<usize>,
    pub chunk_id: Option<String>,
    pub model: Option<DecisionModelRef>,
    pub served_model: Option<String>,
    pub tier: Option<ProviderTier>,
    pub questions: IndexMap<String, Question>,
    /// Exactly what was (or would have been) sent. Same chat, same tier.
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
struct Attempt<'a> {
    session: Option<&'a DecisionSession>,
    batch_id: Option<&'a str>,
    chunk_index: Option<usize>,
    chunk_id: Option<&'a str>,
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
        request.validate().map_err(DecisionError::Invalid)?;
        let resolved = match self.resolve(session, model).await {
            Ok(resolved) => resolved,
            Err(error) => {
                self.record_failure(session, None, request, &error, None)
                    .await;
                return Err(error);
            }
        };
        self.decide_one(
            resolved.as_ref(),
            request,
            Attempt {
                session,
                batch_id: None,
                chunk_index: None,
                chunk_id: None,
            },
        )
        .await
    }

    /// Answer the same questions over many states, one request per chunk, with
    /// bounded parallelism. The model is resolved and the privacy gate asked
    /// once before anything is sent: a refusal there is the whole batch's, and
    /// no chunk is sent. After that, one chunk's failure never fails the batch;
    /// it is reported in that chunk's [`ChunkOutcome`]. Outcomes keep the
    /// chunks' order.
    pub async fn decide_many(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&DecisionModelRef>,
        questions: &IndexMap<String, Question>,
        chunks: Vec<DecisionChunk>,
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
        let resolved = self.resolve(session, model).await?;
        let caps = resolved.caps();
        self.check_kinds(resolved.as_ref(), &caps, questions)?;
        if let Some(session) = session {
            self.check_privacy(session, resolved.as_ref()).await?;
        }
        let batch_id = uuid::Uuid::now_v7().to_string();
        let parallel = self.concurrency(&caps).max(1);
        let model = resolved.as_ref();
        let outcomes = futures::stream::iter(chunks.into_iter().enumerate())
            .map(|(index, chunk)| {
                let batch_id = batch_id.as_str();
                async move {
                    let request = DecisionRequest {
                        questions: questions.clone(),
                        context: chunk.context,
                    };
                    let result = match request.validate() {
                        Err(e) => Err(DecisionError::Invalid(e)),
                        Ok(()) => {
                            self.decide_one(
                                model,
                                &request,
                                Attempt {
                                    session,
                                    batch_id: Some(batch_id),
                                    chunk_index: Some(index),
                                    chunk_id: Some(&chunk.id),
                                },
                            )
                            .await
                        }
                    };
                    match result {
                        Ok(decisions) => ChunkOutcome {
                            chunk_id: chunk.id,
                            decisions: Some(decisions),
                            error: None,
                        },
                        Err(error) => ChunkOutcome {
                            chunk_id: chunk.id,
                            decisions: None,
                            error: Some(error.to_string()),
                        },
                    }
                }
            })
            .buffered(parallel)
            .collect::<Vec<_>>()
            .await;
        Ok(outcomes)
    }

    /// The model named, else the configured default, else the first available
    /// model this chat may use, local first.
    async fn resolve(
        &self,
        session: Option<&DecisionSession>,
        wanted: Option<&DecisionModelRef>,
    ) -> DecisionResult<Arc<dyn DecisionModel>> {
        if let Some(wanted) = wanted {
            return self.source.open(wanted).await;
        }
        let configured = match &self.default_model {
            DefaultModel::FromConfig => configured_default(),
            DefaultModel::Fixed(model) => model.clone().map(ConfiguredDefault::Model),
        };
        let entries = match configured {
            Some(ConfiguredDefault::Model(model)) => return self.source.open(&model).await,
            Some(ConfiguredDefault::Provider(provider)) => self
                .models()
                .await
                .into_iter()
                .filter(|entry| entry.provider == provider)
                .collect::<Vec<_>>(),
            None => self.models().await,
        };
        let allowed = entries.into_iter().find(|entry| {
            session.is_none_or(|session| {
                privacy_allows(session, &entry.display_name, entry.tier, entry.affiliation).is_ok()
            })
        });
        match allowed {
            Some(entry) => self.source.open(&entry.model_ref()).await,
            None => Err(DecisionError::NoModel(match session {
                Some(session) if session.enforced && session.classification.is_private() => {
                    "No decision model this private chat may use is available. Pull a decision \
                     model into a local Ollama (for example `ollama pull tev1:0.8b`), or set \
                     BIOROUTER_DECISION_MODEL to a private one."
                        .to_string()
                }
                _ => "No decision model is available. Add a Baseten API key, pull a decision \
                      model into Ollama, or set BIOROUTER_DECISION_MODEL."
                    .to_string(),
            })),
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
        let model_ref = model.model_ref();
        let checks = async {
            self.check_kinds(model, &caps, &request.questions)?;
            if let Some(session) = attempt.session {
                self.check_privacy(session, model).await?;
            }
            check_size(model, &caps, request, attempt.chunk_id).await
        };
        if let Err(error) = checks.await {
            self.record_failure(
                attempt.session,
                Some(model),
                request,
                &error,
                Some(&attempt),
            )
            .await;
            return Err(error);
        }

        let permit = self
            .semaphore(model, &caps)
            .acquire_owned()
            .await
            .map_err(|_| {
                DecisionError::NoModel("The decision service is shutting down.".to_string())
            })?;
        let deadline = self.timeout_for(model, &caps);
        let started = Instant::now();
        let called = tokio::time::timeout(deadline, model.decide(request)).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        drop(permit);

        let verdicts = match called {
            Ok(Ok(verdicts)) => verdicts,
            Ok(Err(error)) => {
                self.record_failure(
                    attempt.session,
                    Some(model),
                    request,
                    &error,
                    Some(&attempt),
                )
                .await;
                return Err(error);
            }
            Err(_) => {
                let error = DecisionError::Timeout {
                    model: model.display_name(),
                    seconds: deadline.as_secs(),
                };
                self.record_failure(
                    attempt.session,
                    Some(model),
                    request,
                    &error,
                    Some(&attempt),
                )
                .await;
                return Err(error);
            }
        };
        if caps.local {
            if let Ok(mut warm) = self.warm.lock() {
                warm.insert(model_ref.label());
            }
        }

        let request_id = uuid::Uuid::now_v7().to_string();
        let meta = DecisionMeta {
            model: &model_ref,
            tier: model.tier(),
            request_id: &request_id,
            latency_ms,
            usage: verdicts.usage,
        };
        let mut by_name: HashMap<String, _> = verdicts
            .verdicts
            .into_iter()
            .map(|verdict| (verdict.question.clone(), verdict))
            .collect();
        let mut decisions = Vec::with_capacity(request.questions.len());
        for (name, question) in &request.questions {
            let built = by_name
                .remove(name)
                .ok_or_else(|| format!("no answer for question `{name}`"))
                .and_then(|verdict| decision_from_verdict(name, question, verdict, &meta));
            match built {
                Ok(decision) => decisions.push(decision),
                Err(detail) => {
                    let error = DecisionError::malformed(&model.display_name(), detail);
                    self.record_failure(
                        attempt.session,
                        Some(model),
                        request,
                        &error,
                        Some(&attempt),
                    )
                    .await;
                    return Err(error);
                }
            }
        }

        let outcome = if decisions.iter().any(|d| d.passed == Some(false)) {
            DecisionOutcome::Failed
        } else if decisions.iter().any(|d| d.passed == Some(true)) {
            DecisionOutcome::Passed
        } else {
            DecisionOutcome::Decided
        };
        self.record(DecisionLogEntry {
            session_id: attempt.session.map(|s| s.session_id.clone()),
            origin: attempt.session.map(|s| s.origin).unwrap_or_default(),
            origin_id: attempt.session.and_then(|s| s.origin_id.clone()),
            batch_id: attempt.batch_id.map(str::to_string),
            chunk_index: attempt.chunk_index,
            chunk_id: attempt.chunk_id.map(str::to_string),
            model: Some(model_ref),
            served_model: verdicts.served_model,
            tier: Some(model.tier()),
            questions: request.questions.clone(),
            context: request.context.clone(),
            context_bytes: request.context.byte_len(),
            outcome,
            decisions: decisions.clone(),
            error: None,
            latency_ms: Some(latency_ms),
            usage: verdicts.usage,
            privacy_enforced: attempt.session.is_some_and(|s| s.enforced),
        })
        .await;
        Ok(decisions)
    }

    async fn record_failure(
        &self,
        session: Option<&DecisionSession>,
        model: Option<&dyn DecisionModel>,
        request: &DecisionRequest,
        error: &DecisionError,
        attempt: Option<&Attempt<'_>>,
    ) {
        let outcome = if matches!(error, DecisionError::Refused(_)) {
            DecisionOutcome::Refused
        } else {
            DecisionOutcome::Error
        };
        self.record(DecisionLogEntry {
            session_id: session.map(|s| s.session_id.clone()),
            origin: session.map(|s| s.origin).unwrap_or_default(),
            origin_id: session.and_then(|s| s.origin_id.clone()),
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
            usage: DecisionUsage::default(),
            privacy_enforced: session.is_some_and(|s| s.enforced),
        })
        .await;
    }

    async fn record(&self, entry: DecisionLogEntry) {
        let sink = self.sink.read().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            if let Err(error) = sink.record(entry).await {
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

    #[derive(Default)]
    struct Log(Mutex<Vec<DecisionLogEntry>>);

    #[async_trait]
    impl DecisionSink for Log {
        async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(entry);
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

        let (only_public, _) = service(
            vec![entry("cloud", ProviderTier::Public, false)],
            Duration::ZERO,
        );
        let err = only_public
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
}
