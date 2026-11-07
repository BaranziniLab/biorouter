//! `/dictation/*`: local dictation over HTTP (PR-15; DD-27 to DD-32).
//!
//! `docs/dictation/engine-and-models.md` ("Daemon routes and transport") is the contract
//! and says why each rule is what it is. In short:
//!
//! * **One chunked HTTP stream** per dictation (DD-27): create, `PUT` raw 16 bit samples
//!   with a strictly increasing `seq`, `finish`, read, delete. A repeated `seq` is
//!   answered as before and its body ignored, so a client may resend after a timeout.
//! * **Segments are decoded while the person speaks** (DD-50, lever L-7). Once 8 s lie
//!   past the decoded mark, the voice detector reads the undecoded tail and every closed
//!   stretch of speech is queued for the warm worker, so at stop only the open tail is
//!   left. That tail, when it is under 25 s, goes straight to `transcribe`, whose own
//!   detector trims it, with no second detector pass and no second queue hop. Once the
//!   person has stopped, no new closed segment is cut. `finish` may carry the tail
//!   chunk itself (`?seq=N` with a body), so a stop costs one round trip, and a read
//!   (`GET ?wait_ms=`) is held like `finish` instead of being polled.
//! * **The worker is prepared when the stream is created**, not at stop, and before
//!   anything else the creation decides: the model loads while the person speaks, and
//!   the dictionary's prompt, corrector and bonus table are built off the path from stop
//!   to text. The 201 waits for neither the load nor the target decision.
//! * **The daemon decides the bias once per stream** (DD-30), on a task of its own after
//!   the 201: from the target chat's tier and live model, read only after
//!   `session_reach` admits the caller, with any refusal degrading silently to public
//!   terms and a byte identical answer; from the tier a new chat's bind would produce;
//!   or public for Crew and outside targets unless the person included private terms for
//!   this recording (O-8). Standing is `HttpCaller::dictation_standing`. A chat's live
//!   model is read within 250 ms, and never for a chat a Crew grant scopes, whose
//!   provider check can ask the Crew workspace over SSH; either way the stream then
//!   fails toward public terms.
//! * **Authority** (DD-29, O-17): every route takes the daemon secret through
//!   `check_token`; streams need nothing more. Installing, selecting and deleting a
//!   model need the user action proof on a daemon that holds a key (DD-20), and on a
//!   keyless daemon an install is refused while the operator lock is set.
//! * **Limits** (DD-28, DD-32): 8 open streams and 32 queued segments, then 429 with
//!   `Retry-After`; chunks of at most 524,288 B; JSON of at most 8 KiB; 600 s of audio
//!   per stream; 60 s idle expiry.
//!
//! Audio and transcript text stay in memory: the audio buffer is zeroed when the
//! transcript is ready, on `DELETE` and at the idle expiry, every job's copy of it when
//! the job is dropped on any path, and the transcript on `DELETE` and at the expiry
//! (P-1). Nothing here logs audio, text, partial text, changes, prompts, dictionary
//! entries or `context` (DD-93): only a stream's log id (a digest, never the id that
//! opens the stream), target kinds, scopes, durations, byte counts and model ids.
//!
//! A decode that fails while the person is still speaking ends the stream's intake: every
//! later chunk answers the same `500 engineFailed`, so the client says so at once and its
//! **Retry** replays the audio it kept.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use biorouter::dictation::audio::{
    plan_closed_segments, plan_final_segments, PcmSlice, Span, MAX_SEGMENT_MS, SAMPLES_PER_MS,
    STREAM_VAD_TRIGGER_MS,
};
use biorouter::dictation::bias::DictionaryCache;
use biorouter::dictation::catalog::{SttCatalogEntry, CATALOG};
use biorouter::dictation::models::{InstallOptions, Network, Phase, Progress, WriteMode};
use biorouter::dictation::paths::{self, Roots};
use biorouter::dictation::recognize::{text, BiasScope, RetryReason};
use biorouter::dictation::service::{self, DictationError};
use biorouter::dictation::{
    hardware, models, BiasChannels, BiasTarget, DictionaryRequest, LoadSpec, TranscribeRequest,
};
use biorouter::managed::{Allowance, DictationPolicy, ManagedPolicy};
use biorouter::privacy::{ModelAffiliation, ProviderTier, SessionClassification};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use utoipa::{IntoParams, ToSchema};
use zeroize::Zeroize;

use crate::routes::session_reach::{self, TargetTier};
use crate::state::AppState;

// ── limits (DD-28, DD-32) ────────────────────────────────────────────────────────────

/// The largest audio chunk a `PUT` (or `finish`) takes.
pub const MAX_CHUNK_BYTES: usize = 524_288;
/// The largest JSON body a dictation route takes.
pub const MAX_JSON_BYTES: usize = 8 * 1024;
/// The largest `context` a stream takes; it only ranks the dictionary's terms.
pub const MAX_CONTEXT_BYTES: usize = 2 * 1024;
/// Audio one stream holds at most.
pub const MAX_STREAM_SECONDS: u64 = 600;
/// A stream nobody has touched for this long is dropped, audio and transcript.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Open streams per daemon; the next one answers 429.
pub const MAX_STREAMS: usize = 8;
/// Segments queued for decoding across every stream; creation then answers 429.
pub const MAX_QUEUED_SEGMENTS: usize = 32;
/// How long `finish` waits for the transcript before it answers 202.
pub const FINISH_WAIT: Duration = Duration::from_secs(3);
/// The most audio one detector pass of the segmenter reads. Audio that arrives in real
/// time never fills it (a pass reads about 8 s, and every pass over 33 s moves the mark,
/// `plan_pass`); audio that arrives at once (a replay after a 410, the buffer a client
/// kept while creation answered 429) is read in passes of this size, because detector
/// passes go ahead of every stream's decodes, and one pass over minutes of audio held
/// another person's stop behind it (measured in
/// `a_short_stream_finishes_while_a_long_one_runs`).
pub const MAX_DETECTOR_PASS_MS: u64 = 60_000;
/// The longest wait a read (`GET ?wait_ms=`) may ask for: the same as `finish`.
pub const MAX_READ_WAIT_MS: u64 = 3_000;
/// What a 429 asks the client to wait.
pub const RETRY_AFTER_SECONDS: u64 = 5;
/// How long the target decision waits for a live chat's bound model before it counts as
/// none, which fails toward public terms. Reading it takes the agent's provider lock and
/// the privacy and Crew checks `Agent::provider` makes, all in memory for a chat no Crew
/// grant scopes.
pub const LIVE_MODEL_BOUND: Duration = Duration::from_millis(250);
/// The sample rates a stream may declare.
pub const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=96_000;
/// Debug builds only: a shorter idle expiry, for the route tests, as
/// `BIOROUTER_DICTATION_TEST_MODEL` is read only by a debug build.
pub const TEST_IDLE_ENV: &str = "BIOROUTER_DICTATION_TEST_IDLE_MS";
/// Set to `false` (or `0`, `no`, `off`) by `biorouter serve --no-dictation-install`: a
/// keyless daemon then refuses installs with MSG-INSTALL-LOCKED (O-2).
pub const BROWSER_INSTALL_ENV: &str = "BIOROUTER_DICTATION_BROWSER_INSTALL";

/// MSG-INSTALL-AGENT.
pub const MSG_INSTALL_AGENT: &str =
    "Speech models are installed by a person from Biorouter's Settings; an agent cannot install one.";
/// MSG-INSTALL-LOCKED.
pub const MSG_INSTALL_LOCKED: &str = "Dictation is not set up on this server. Ask the person who runs it to run `biorouter dictation install`.";

/// The limits one daemon applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamLimits {
    pub max_streams: usize,
    pub max_queued_segments: usize,
    pub idle: Duration,
}

impl Default for StreamLimits {
    fn default() -> StreamLimits {
        StreamLimits {
            max_streams: MAX_STREAMS,
            max_queued_segments: MAX_QUEUED_SEGMENTS,
            idle: IDLE_TIMEOUT,
        }
    }
}

impl StreamLimits {
    /// The production limits; a debug build honours [`TEST_IDLE_ENV`].
    pub fn from_env() -> StreamLimits {
        let mut limits = StreamLimits::default();
        if cfg!(debug_assertions) {
            if let Some(ms) = std::env::var(TEST_IDLE_ENV)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|ms| *ms > 0)
            {
                limits.idle = Duration::from_millis(ms);
            }
        }
        limits
    }
}

// ── request and answer shapes ────────────────────────────────────────────────────────

/// Where the dictated text goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationTargetKind {
    Chat,
    NewChat,
    Editor,
    Crew,
    External,
}

impl DictationTargetKind {
    fn as_str(self) -> &'static str {
        match self {
            DictationTargetKind::Chat => "chat",
            DictationTargetKind::NewChat => "new_chat",
            DictationTargetKind::Editor => "editor",
            DictationTargetKind::Crew => "crew",
            DictationTargetKind::External => "external",
        }
    }
}

/// `POST /dictation/streams`.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct DictationStreamRequest {
    /// 16000, or the declared native rate (8,000 to 96,000).
    pub sample_rate: u32,
    /// Always `s16le`.
    pub format: String,
    /// A language code, or `auto`; the configured language when absent.
    #[serde(default)]
    pub language: Option<String>,
    pub target_kind: DictationTargetKind,
    /// `chat` and `editor` only.
    #[serde(default)]
    pub session_id: Option<String>,
    /// `new_chat` only: the tab's provider pick, honoured only with the user action proof.
    #[serde(default)]
    pub new_chat_provider: Option<String>,
    /// `new_chat` only: the tab's model pick, beside `new_chat_provider` and under the same
    /// rule, so a provider whose tier reads the model (an Ollama cloud tag) is judged as
    /// the new chat will bind it.
    #[serde(default)]
    pub new_chat_model: Option<String>,
    /// `crew` and `external` only: sent only after the person confirmed MSG-PRIVATE-TERMS
    /// for this recording, honoured only with the proof and private standing (O-8).
    #[serde(default)]
    pub include_private_terms: bool,
    /// At most 2 KiB; ranks the dictionary's terms and is never logged.
    #[serde(default)]
    pub context: Option<String>,
}

/// 201 from `POST /dictation/streams`.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationStreamCreated {
    pub stream_id: String,
    pub max_chunk_bytes: u64,
    pub idle_timeout_s: u64,
    pub max_seconds: u64,
}

/// 200 from `PUT /dictation/streams/{id}/audio`.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationAudioAck {
    /// Seconds of audio the stream holds after this chunk.
    pub received_s: f64,
    /// The corrected text of the segments decoded so far, when it changed since the last
    /// answer: a muted preview only, never inserted (DD-31).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial: Option<String>,
    /// This stream's segments still waiting to be decoded.
    pub queue_position: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationTranscriptChange {
    /// Characters (Unicode scalar values) of `text`.
    pub start: u64,
    pub end: u64,
    pub from: String,
    pub to: String,
    /// `alias`, `exact` or `fuzzy`.
    pub stage: String,
    pub entry_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationTranscriptSegment {
    /// Seconds from the start of the stream.
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationTranscriptRetry {
    /// `echo` or `collapse`.
    pub reason: String,
}

/// Where the stop path's time went, as the daemon saw it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationTranscriptTimings {
    /// Segments decoded, the tail included.
    pub segments: u64,
    /// From `finish` to the tail's decode starting: a segment still decoding when the
    /// person stopped, or a model still loading.
    pub wait_ms: u64,
    /// The tail's decode.
    pub tail_ms: u64,
    /// Seconds of audio the tail held.
    pub tail_audio_s: f64,
}

/// The transcript, from `finish` or a read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DictationTranscript {
    /// The corrected text.
    pub text: String,
    /// As the engine wrote it, after the nonspeech filter.
    pub raw_text: String,
    pub changes: Vec<DictationTranscriptChange>,
    pub segments: Vec<DictationTranscriptSegment>,
    pub model: String,
    pub audio_s: f64,
    /// From `finish` to the transcript being ready.
    pub elapsed_ms: u64,
    /// `public` for any caller without private standing.
    pub bias_scope: DictationBiasScope,
    pub retried: Option<DictationTranscriptRetry>,
    pub no_speech: bool,
    pub timings: DictationTranscriptTimings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationBiasScope {
    Public,
    Private,
}

impl From<BiasScope> for DictationBiasScope {
    fn from(scope: BiasScope) -> DictationBiasScope {
        match scope {
            BiasScope::Public => DictationBiasScope::Public,
            BiasScope::Private => DictationBiasScope::Private,
        }
    }
}

/// 202 from `finish` or a read: the transcript is not ready yet.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationTranscriptPending {
    pub pending: bool,
}

/// Every refusal of a dictation route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DictationErrorBody {
    pub code: String,
    /// The sentence to show, or empty where the client has its own.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_seq: Option<u64>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DictationSeqQuery {
    /// The chunk's sequence number, from 0.
    pub seq: u64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DictationFinishQuery {
    /// With a body: the sequence number of the tail chunk `finish` carries.
    #[serde(default)]
    pub seq: Option<u64>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DictationReadQuery {
    /// Hold the read up to this many milliseconds (at most 3,000) for the transcript.
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

// ── refusals ─────────────────────────────────────────────────────────────────────────

/// One refusal: its status, its body, and a `Retry-After` for a 429.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: StatusCode,
    pub body: DictationErrorBody,
    pub retry_after: Option<u64>,
}

impl Refusal {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Refusal {
        Refusal {
            status,
            body: DictationErrorBody {
                code: code.to_string(),
                message: message.into(),
                expected_seq: None,
            },
            retry_after: None,
        }
    }

    fn policy(allowance: Allowance) -> Refusal {
        Refusal::new(
            StatusCode::FORBIDDEN,
            "policy",
            allowance
                .reason
                .unwrap_or_else(|| biorouter::managed::MANAGED_DICTATION_OFF.to_string()),
        )
    }

    fn busy(code: &str) -> Refusal {
        Refusal {
            retry_after: Some(RETRY_AFTER_SECONDS),
            ..Refusal::new(StatusCode::TOO_MANY_REQUESTS, code, "")
        }
    }

    fn gone() -> Refusal {
        Refusal::new(StatusCode::GONE, "streamGone", "")
    }

    fn too_large() -> Refusal {
        Refusal::new(StatusCode::PAYLOAD_TOO_LARGE, "tooLarge", "")
    }

    fn bad_request() -> Refusal {
        Refusal::new(StatusCode::UNPROCESSABLE_ENTITY, "badRequest", "")
    }

    fn out_of_order(expected: u64) -> Refusal {
        let mut refusal = Refusal::new(StatusCode::CONFLICT, "outOfOrder", "");
        refusal.body.expected_seq = Some(expected);
        refusal
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        if let Some(seconds) = self.retry_after {
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}

/// The refusal a service error answers with (the error table of
/// `engine-and-models.md`). `install_reason` fills `noModel`'s message: this caller's
/// `install.reason`, or empty when installing is allowed, so the interface opens the setup
/// popover instead.
fn refusal_of(error: &DictationError, install_reason: Option<String>) -> Refusal {
    match error {
        DictationError::NotCompiled => {
            Refusal::new(StatusCode::CONFLICT, "notCompiled", error.to_string())
        }
        DictationError::UnsupportedCpu { .. } => {
            Refusal::new(StatusCode::CONFLICT, "unsupportedCpu", error.to_string())
        }
        DictationError::Policy(allowance) => Refusal::policy(allowance.clone()),
        DictationError::NoModel { .. }
        | DictationError::NotInstalled { .. }
        | DictationError::DetectorNotInstalled { .. } => Refusal::new(
            StatusCode::CONFLICT,
            "noModel",
            install_reason.unwrap_or_default(),
        ),
        DictationError::EnglishOnly { .. } => Refusal::new(
            StatusCode::CONFLICT,
            "languageUnsupported",
            error.to_string(),
        ),
        DictationError::LanguageUnsupported { .. } | DictationError::UnknownModel(_) => {
            Refusal::bad_request()
        }
        DictationError::Cancelled => Refusal::gone(),
        DictationError::WorkerExited { .. }
        | DictationError::Deadline { .. }
        | DictationError::Protocol(_)
        | DictationError::ModelLoadFailed { .. }
        | DictationError::BadAudio
        | DictationError::Internal(_) => Refusal::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "engineFailed",
            error.to_string(),
        ),
    }
}

/// A JSON body of at most [`MAX_JSON_BYTES`], answered with this module's refusals.
fn json_body<T: for<'de> Deserialize<'de>>(
    body: Result<Bytes, BytesRejection>,
) -> Result<T, Refusal> {
    let body = body.map_err(|_| Refusal::too_large())?;
    if body.len() > MAX_JSON_BYTES {
        return Err(Refusal::too_large());
    }
    serde_json::from_slice(&body).map_err(|_| Refusal::bad_request())
}

// ── the stream table (`AppState::dictation`) ─────────────────────────────────────────

/// The daemon's dictation state: its open streams, the queued segment count and the
/// install that may be running. One per [`AppState`].
pub struct DictationStreams {
    shared: Arc<Shared>,
}

struct Shared {
    limits: StreamLimits,
    table: Mutex<HashMap<String, Arc<LiveStream>>>,
    /// Segments queued for decoding and not decoded yet, every stream.
    queued: AtomicUsize,
    install: Mutex<Option<Arc<InstallJob>>>,
}

impl Default for DictationStreams {
    fn default() -> DictationStreams {
        DictationStreams::new(StreamLimits::from_env())
    }
}

impl DictationStreams {
    pub fn new(limits: StreamLimits) -> DictationStreams {
        DictationStreams {
            shared: Arc::new(Shared {
                limits,
                table: Mutex::new(HashMap::new()),
                queued: AtomicUsize::new(0),
                install: Mutex::new(None),
            }),
        }
    }

    pub fn limits(&self) -> StreamLimits {
        self.shared.limits
    }

    /// Open streams, finished ones not yet deleted included.
    pub fn open(&self) -> usize {
        lock(&self.shared.table).len()
    }

    /// Segments queued and not decoded yet, every stream.
    pub fn queued_segments(&self) -> usize {
        self.shared.queued.load(Ordering::SeqCst)
    }

    /// DD-32's limits on a new stream: 429 `tooManyStreams` with `Retry-After` while the
    /// daemon holds its most open streams, or its most queued segments.
    fn admission_refusal(&self) -> Option<Refusal> {
        let limits = self.limits();
        (self.open() >= limits.max_streams || self.queued_segments() >= limits.max_queued_segments)
            .then(|| Refusal::busy("tooManyStreams"))
    }

    fn get(&self, id: &str) -> Option<Arc<LiveStream>> {
        lock(&self.shared.table).get(id).cloned()
    }

    fn remove(&self, id: &str) -> Option<Arc<LiveStream>> {
        lock(&self.shared.table).remove(id)
    }

    /// The written forms of the dictionary entries stream `id` may use, once its target
    /// was decided and its dictionary built; `None` before then or for no such stream.
    /// For the route tests of which rows a target admits (P-16); no route answers it, and
    /// the daemon's own binary, which compiles this module privately, never calls it.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn eligible_terms(&self, id: &str) -> Option<Vec<String>> {
        let stream = self.get(id)?;
        let dictionary = stream.dictionary.get()?;
        Some(
            dictionary
                .eligible
                .entries()
                .iter()
                .map(|entry| entry.written.clone())
                .collect(),
        )
    }

    /// Where stream `id`'s segmenter and decoder stand; `None` for no such stream. For
    /// the route test of how streams take turns on the worker, which starts its short
    /// stream once the long one's detector has read everything and times the long one's
    /// own segments; no route answers it, and the daemon's own binary never calls it.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn progress(&self, id: &str) -> Option<StreamProgress> {
        let stream = self.get(id)?;
        let inner = stream.inner();
        Some(StreamProgress {
            caught_up: !inner.detecting && inner.due_window().is_none(),
            segments_decoded: inner.segments_decoded,
            detector_passes: inner.detector_passes,
        })
    }
}

/// What [`DictationStreams::progress`] answers.
#[doc(hidden)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamProgress {
    /// No detector pass runs, and none would start on the audio received so far: less
    /// than [`STREAM_VAD_TRIGGER_MS`] of it lies past the mark, or the last pass moved
    /// nothing and no audio came after what it read.
    pub caught_up: bool,
    /// The segments decoded so far, the tail among them once it is.
    pub segments_decoded: u64,
    /// The detector passes the segmenter has started.
    pub detector_passes: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 128 random bits in hex.
fn new_stream_id() -> String {
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}

/// What a log line names a stream by: the first 8 hex characters of its id's SHA-256.
/// The id itself is a bearer credential for the stream's audio and transcript (DD-29),
/// so no log line carries it, and a digest still ties one stream's lines together.
fn log_id_of(id: &str) -> String {
    hex::encode(&Sha256::digest(id.as_bytes())[..4])
}

/// Supervisor stream keys for live streams: a range of their own, two per stream (its
/// decodes, then their retries), apart from `transcribe`'s.
fn next_stream_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1 << 61);
    NEXT.fetch_add(2, Ordering::Relaxed)
}

/// One open stream.
struct LiveStream {
    id: String,
    /// [`log_id_of`] the id: what every log line names this stream by.
    log_id: String,
    key: u64,
    target_kind: DictationTargetKind,
    rate: u32,
    spec: LoadSpec,
    model: String,
    language: String,
    cancel: CancellationToken,
    closed: AtomicBool,
    /// True once the load started at creation has answered, either way.
    prepared: watch::Receiver<bool>,
    inner: Mutex<Inner>,
    /// The dictionary this stream decodes with, once built.
    dictionary: OnceLock<Arc<DictionaryRequest>>,
    outcome: watch::Sender<Option<Arc<Result<DictationTranscript, Refusal>>>>,
    activity: watch::Sender<Instant>,
    usage_recorded: AtomicBool,
    shared: Weak<Shared>,
}

/// A stream's mutable half. Held only for short, synchronous steps, never across an
/// await.
struct Inner {
    /// 16 kHz samples, or for another declared rate the native ones (resampled once, at
    /// finish).
    pcm: Vec<i16>,
    next_seq: u64,
    /// `received_s` after each acknowledged `seq`, for a repeat.
    acks: Vec<f64>,
    /// Milliseconds of the 16 kHz buffer already planned into segments.
    mark_ms: u64,
    /// The segmenter's detector pass is running.
    detecting: bool,
    /// The detector passes started, for [`DictationStreams::progress`].
    detector_passes: u64,
    /// Where the window of the last detector pass ended, when that pass moved nothing
    /// (open speech under 33 s past the mark): the same audio would give the same answer,
    /// so the next pass waits for audio past it, or for the stop.
    stalled_at_ms: Option<u64>,
    /// Speech the last detector pass found past the mark, still open then.
    open_speech_ms: u64,
    /// This stream's segments queued and not decoded yet.
    pending: usize,
    jobs: Option<mpsc::UnboundedSender<Job>>,
    finishing: bool,
    finish_at: Option<Instant>,
    pieces: Vec<Piece>,
    retried: Option<RetryReason>,
    segments_decoded: u64,
    partial: Option<String>,
    partial_reported: Option<String>,
}

#[derive(Clone, Debug)]
struct Piece {
    t0_ms: u64,
    t1_ms: u64,
    text: String,
}

/// What the decoder does next, in stream order.
enum Job {
    /// A closed segment the segmenter cut while the person spoke.
    Segment {
        pcm: AudioCopy,
        offset_ms: u64,
        speech_ms: u64,
    },
    /// What was left at stop: 16 kHz from `offset_ms`, or a whole recording at another
    /// rate to resample first.
    Tail {
        pcm: AudioCopy,
        offset_ms: u64,
        rate: u32,
        open_speech_ms: u64,
    },
}

/// One job's or one detector pass's copy of the audio (P-1). It zeroes itself when it is
/// dropped and nothing else holds it, so every path that ends a job wipes its copy: a
/// decode that answered, a stream deleted or expired with segments still queued, an
/// engine that failed, an early return on an error. The service's own clones are dropped
/// when its call returns, before this.
struct AudioCopy(Arc<[i16]>);

impl AudioCopy {
    fn new(samples: &[i16]) -> AudioCopy {
        AudioCopy(Arc::from(samples))
    }

    /// The samples, to hand the service for one call.
    fn shared(&self) -> Arc<[i16]> {
        Arc::clone(&self.0)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_silent(&self) -> bool {
        self.0.iter().all(|&s| s == 0)
    }
}

impl Drop for AudioCopy {
    fn drop(&mut self) {
        if let Some(samples) = Arc::get_mut(&mut self.0) {
            samples.zeroize();
            #[cfg(test)]
            WIPED_SAMPLES.with(|wiped| wiped.set(wiped.get() + samples.len()));
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Samples [`AudioCopy`] zeroed on this thread, for the P-1 tests.
    static WIPED_SAMPLES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The decoder's end of a stream's queue. However the decoder ends, the jobs still queued
/// are dropped there and then, so their copies of the audio are zeroed at once rather
/// than whenever the channel is freed (P-1).
struct JobQueue(mpsc::UnboundedReceiver<Job>);

impl JobQueue {
    async fn next(&mut self) -> Option<Job> {
        self.0.recv().await
    }
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        self.0.close();
        while let Ok(job) = self.0.try_recv() {
            drop(job);
        }
    }
}

/// What [`Inner::append`] did with one chunk.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Appended {
    Accepted { received_s: f64 },
    Repeat { received_s: f64 },
    OutOfOrder { expected: u64 },
    TooLong,
    OddLength,
    Finished,
}

impl Inner {
    fn new(jobs: mpsc::UnboundedSender<Job>) -> Inner {
        Inner {
            pcm: Vec::new(),
            next_seq: 0,
            acks: Vec::new(),
            mark_ms: 0,
            detecting: false,
            detector_passes: 0,
            stalled_at_ms: None,
            open_speech_ms: 0,
            pending: 0,
            jobs: Some(jobs),
            finishing: false,
            finish_at: None,
            pieces: Vec::new(),
            retried: None,
            segments_decoded: 0,
            partial: None,
            partial_reported: None,
        }
    }

    /// One chunk at `seq` (DD-28): the next `seq` is appended, a repeated one is answered
    /// as before with its body ignored, and a higher one is refused with the expected
    /// value. A chunk that would carry the stream past `max_samples` is refused whole.
    fn append(&mut self, seq: u64, body: &[u8], rate: u32, max_samples: usize) -> Appended {
        if seq < self.next_seq {
            let received_s = usize::try_from(seq)
                .ok()
                .and_then(|i| self.acks.get(i).copied())
                .unwrap_or(0.0);
            return Appended::Repeat { received_s };
        }
        if seq > self.next_seq {
            return Appended::OutOfOrder {
                expected: self.next_seq,
            };
        }
        if self.finishing {
            return Appended::Finished;
        }
        if !body.len().is_multiple_of(2) {
            return Appended::OddLength;
        }
        let samples = body.len() / 2;
        if self.pcm.len() + samples > max_samples {
            return Appended::TooLong;
        }
        reserve_zeroizing(&mut self.pcm, samples, max_samples);
        self.pcm.extend(
            body.chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]])),
        );
        self.next_seq += 1;
        let received_s = self.pcm.len() as f64 / f64::from(rate);
        self.acks.push(received_s);
        Appended::Accepted { received_s }
    }

    /// The buffer's end in milliseconds of 16 kHz audio.
    fn end_ms(&self) -> u64 {
        (self.pcm.len() as u64) / SAMPLES_PER_MS
    }

    /// The window the segmenter's next detector pass reads, if one is due: once
    /// [`STREAM_VAD_TRIGGER_MS`] lie past the mark, at most [`MAX_DETECTOR_PASS_MS`] of
    /// them, and after a pass that moved nothing only once audio arrived past what it
    /// read. Without that last rule a stalled pass started again at once, back to back
    /// for as long as no audio came (25 passes a second on the test fixture, each one
    /// queued ahead of every stream's decodes), and the idle expiry counted the passes as
    /// work, so a stream the client left open never expired.
    fn due_window(&self) -> Option<Range<u64>> {
        let end_ms = self.end_ms();
        if end_ms < self.mark_ms.saturating_add(STREAM_VAD_TRIGGER_MS) {
            return None;
        }
        let window = self.mark_ms..end_ms.min(self.mark_ms.saturating_add(MAX_DETECTOR_PASS_MS));
        if self.stalled_at_ms.is_some_and(|at| window.end <= at) {
            return None;
        }
        Some(window)
    }

    /// The partial text to report, when it changed since the last report.
    fn take_partial(&mut self) -> Option<String> {
        if self.partial.is_some() && self.partial != self.partial_reported {
            self.partial_reported.clone_from(&self.partial);
            return self.partial.clone();
        }
        None
    }

    fn wipe(&mut self) {
        self.pcm.zeroize();
        self.pcm = Vec::new();
    }

    /// The decoded pieces and the partial text, zeroed (P-1).
    fn wipe_text(&mut self) {
        for piece in &mut self.pieces {
            piece.text.zeroize();
        }
        self.pieces = Vec::new();
        self.partial.zeroize();
        self.partial_reported.zeroize();
    }
}

/// Every string of a transcript, zeroed (P-1).
fn wipe_transcript(transcript: &mut DictationTranscript) {
    transcript.text.zeroize();
    transcript.raw_text.zeroize();
    for change in &mut transcript.changes {
        change.from.zeroize();
        change.to.zeroize();
    }
    for segment in &mut transcript.segments {
        segment.text.zeroize();
    }
}

/// Grows `buf` for `more` samples without leaving a copy of the audio behind: the old
/// allocation is zeroed before it is freed (P-1).
fn reserve_zeroizing(buf: &mut Vec<i16>, more: usize, max: usize) {
    if buf.capacity() - buf.len() >= more {
        return;
    }
    let wanted = (buf.len() + more)
        .max(buf.capacity().saturating_mul(2))
        .max(30 * 16_000)
        .min(max.max(buf.len() + more));
    let mut grown = Vec::with_capacity(wanted);
    grown.extend_from_slice(buf);
    buf.zeroize();
    *buf = grown;
}

/// A copy of `range` (milliseconds) of the 16 kHz buffer, for one job.
fn copy_ms(pcm: &[i16], range: &Range<u64>) -> AudioCopy {
    let at = |ms: u64| usize::try_from(ms.saturating_mul(SAMPLES_PER_MS)).unwrap_or(usize::MAX);
    let end = at(range.end).min(pcm.len());
    let start = at(range.start).min(end);
    AudioCopy::new(&pcm[start..end])
}

/// How much audio before a window's end a pass that found no speech keeps unread, so a
/// word starting there, shorter than the detector's 250 ms minimum, is read again.
const SILENCE_KEPT_MS: u64 = 1_000;

/// One segmenter pass over `[mark, window_end)` (DD-50): the closed segments to queue and
/// the new mark. `plan_closed_segments`, plus two rules so every full pass moves the mark
/// and the audio left at stop stays one decode long:
///
/// * a window with no speech at all moves the mark to [`SILENCE_KEPT_MS`] before its end,
///   so a long silence is read once, not again on every `PUT`;
/// * speech that ran longer than one decode with no pause the detector closes (measured:
///   synthetic voices end to end, a minute with no 100 ms pause, left all 60 s to decode
///   at stop, 1.14 s on `small.en` with Metal) is cut every 25 s, as the plan at stop
///   cuts it, once [`MAX_SEGMENT_MS`] plus [`STREAM_VAD_TRIGGER_MS`] lie past the mark.
fn plan_pass(spans: &[Span], mark: u64, window_end: u64) -> (Vec<Range<u64>>, u64) {
    let (closed, next) = plan_closed_segments(spans, mark, window_end);
    if !closed.is_empty() || window_end < mark.saturating_add(STREAM_VAD_TRIGGER_MS) {
        return (closed, next.max(mark));
    }
    if !spans.iter().any(|s| s.t1_ms > mark && s.t0_ms < window_end) {
        return (
            Vec::new(),
            window_end.saturating_sub(SILENCE_KEPT_MS).max(mark),
        );
    }
    if window_end >= mark + MAX_SEGMENT_MS + STREAM_VAD_TRIGGER_MS {
        let cut = mark + MAX_SEGMENT_MS;
        return (plan_final_segments(spans, mark, cut), cut);
    }
    (Vec::new(), mark)
}

/// The detector's speech inside `range`, in milliseconds.
fn speech_within(spans: &[Span], range: &Range<u64>) -> u64 {
    spans
        .iter()
        .map(|s| {
            s.t1_ms
                .min(range.end)
                .saturating_sub(s.t0_ms.max(range.start))
        })
        .sum()
}

impl LiveStream {
    fn touch(&self) {
        self.activity.send_replace(Instant::now());
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }

    /// Work that keeps the stream alive past its idle timer: a queued or running decode, a
    /// detector pass, or a finish waiting for its transcript.
    fn busy(&self) -> bool {
        let inner = self.inner();
        inner.pending > 0 || inner.detecting || (inner.finishing && self.outcome.borrow().is_none())
    }

    fn max_samples(&self) -> usize {
        usize::try_from(MAX_STREAM_SECONDS * u64::from(self.rate)).unwrap_or(usize::MAX)
    }

    /// Ends the stream: its decodes are cancelled, its queue closed, and its audio, its
    /// decoded text and its transcript zeroed (P-1). The queued jobs' copies are zeroed
    /// when the decoder drops them, which the cancel makes it do at once. A read that
    /// crossed the close answers 410.
    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.cancel.cancel();
        {
            let mut inner = self.inner();
            inner.jobs = None;
            if let Some(shared) = self.shared.upgrade() {
                shared.queued.fetch_sub(inner.pending, Ordering::SeqCst);
            }
            inner.pending = 0;
            inner.wipe();
            inner.wipe_text();
        }
        self.retire_outcome();
    }

    /// Replaces the outcome with 410 and zeroes the transcript it held, unless a read
    /// still holds it, which drops it when it answers.
    fn retire_outcome(&self) {
        let gone = Some(Arc::new(Err(Refusal::gone())));
        if let Some(mut done) = self.outcome.send_replace(gone) {
            if let Some(Ok(transcript)) = Arc::get_mut(&mut done) {
                wipe_transcript(transcript);
            }
        }
    }

    /// The failure that ended this stream while the person spoke, if one did.
    fn failure(&self) -> Option<Refusal> {
        match self.outcome.borrow().as_deref() {
            Some(Err(refusal)) => Some(refusal.clone()),
            _ => None,
        }
    }

    fn queue(&self, inner: &mut Inner, job: Job) {
        if let Some(jobs) = &inner.jobs {
            if jobs.send(job).is_ok() {
                inner.pending += 1;
                if let Some(shared) = self.shared.upgrade() {
                    shared.queued.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    fn job_done(&self, inner: &mut Inner) {
        if inner.pending > 0 {
            inner.pending -= 1;
            if let Some(shared) = self.shared.upgrade() {
                shared.queued.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// Cuts the closed segments of the audio past the mark once
    /// [`STREAM_VAD_TRIGGER_MS`] lie past it (DD-50), on a task of its own, so a `PUT` is
    /// answered at once. Nothing is cut once the person has stopped, while a detector
    /// pass runs, for a stream at another rate (decoded whole at stop), once a decode
    /// failed (nothing would decode what it cut, and each pass would restart a worker that
    /// crashed), while 32 segments are already queued (the audio waits in its stream,
    /// so a `PUT` never answers 429), or after a pass that moved nothing until more audio
    /// arrives ([`Inner::due_window`]).
    fn segment(self: &Arc<Self>) {
        let (window, pcm) = {
            let mut inner = self.inner();
            let Some(shared) = self.shared.upgrade() else {
                return;
            };
            if inner.finishing
                || inner.detecting
                || self.rate != biorouter::dictation::audio::SAMPLE_RATE
                || self.closed.load(Ordering::SeqCst)
                || self.outcome.borrow().is_some()
                || shared.queued.load(Ordering::SeqCst) >= shared.limits.max_queued_segments
            {
                return;
            }
            let Some(window) = inner.due_window() else {
                return;
            };
            inner.detecting = true;
            inner.detector_passes += 1;
            let pcm = copy_ms(&inner.pcm, &window);
            (window, pcm)
        };
        let stream = Arc::clone(self);
        tokio::spawn(async move {
            let found = service::service()
                .vad_segments_with(stream.spec.clone(), PcmSlice::whole(pcm.shared()))
                .await;
            drop(pcm);
            let more = {
                let mut inner = stream.inner();
                inner.detecting = false;
                if inner.finishing
                    || stream.closed.load(Ordering::SeqCst)
                    || stream.outcome.borrow().is_some()
                {
                    false
                } else {
                    match found {
                        Ok(spans) => {
                            let spans: Vec<Span> = spans
                                .into_iter()
                                .map(|s| Span {
                                    t0_ms: s.t0_ms + window.start,
                                    t1_ms: (s.t1_ms + window.start).min(window.end),
                                })
                                .filter(|s| s.t0_ms < s.t1_ms)
                                .collect();
                            let before = inner.mark_ms;
                            let (closed, mark) = plan_pass(&spans, before, window.end);
                            for range in biorouter::dictation::file::pack(closed) {
                                let job = Job::Segment {
                                    pcm: copy_ms(&inner.pcm, &range),
                                    offset_ms: range.start,
                                    speech_ms: speech_within(&spans, &range),
                                };
                                stream.queue(&mut inner, job);
                            }
                            inner.mark_ms = before.max(mark);
                            // A pass that moved nothing waits for audio past its window.
                            inner.stalled_at_ms = (inner.mark_ms == before).then_some(window.end);
                            let open = inner.mark_ms..window.end;
                            inner.open_speech_ms = speech_within(&spans, &open);
                            true
                        }
                        Err(_) => {
                            // A detector that failed here fails the decode at stop too,
                            // which reports it; the audio stays for the tail meanwhile.
                            tracing::debug!(stream = %stream.log_id, "dictation detector pass failed");
                            false
                        }
                    }
                }
            };
            if more {
                // The mark moved, or audio arrived while the detector read; a pass that
                // moved nothing over all the audio there is starts no other.
                stream.segment();
            }
        });
    }

    /// The stop (DD-50): the open tail is queued as one job and the queue closed, so the
    /// decoder finishes the transcript after it. A detector pass still running is left
    /// to end on its own and its spans unused. Idempotent.
    fn finish(&self) {
        let mut inner = self.inner();
        if inner.finishing || self.closed.load(Ordering::SeqCst) {
            return;
        }
        inner.finishing = true;
        inner.finish_at = Some(Instant::now());
        let job = if self.rate == biorouter::dictation::audio::SAMPLE_RATE {
            let tail = inner.mark_ms..inner.end_ms();
            (!tail.is_empty()).then(|| Job::Tail {
                pcm: copy_ms(&inner.pcm, &tail),
                offset_ms: tail.start,
                rate: self.rate,
                open_speech_ms: inner.open_speech_ms,
            })
        } else {
            (!inner.pcm.is_empty()).then(|| Job::Tail {
                pcm: AudioCopy::new(&inner.pcm),
                offset_ms: 0,
                rate: self.rate,
                open_speech_ms: 0,
            })
        };
        if let Some(job) = job {
            self.queue(&mut inner, job);
        }
        inner.jobs = None;
    }

    /// Waits up to `wait` for the transcript.
    async fn answer(self: &Arc<Self>, wait: Duration) -> Response {
        let mut outcome = self.outcome.subscribe();
        if !wait.is_zero() {
            let _ = tokio::time::timeout(wait, outcome.wait_for(Option::is_some)).await;
        }
        let ready = outcome.borrow().clone();
        self.touch();
        match ready.as_deref() {
            Some(Ok(transcript)) => {
                self.record_usage(&transcript.text);
                (StatusCode::OK, Json(transcript.clone())).into_response()
            }
            Some(Err(refusal)) => refusal.clone().into_response(),
            None => (
                StatusCode::ACCEPTED,
                Json(DictationTranscriptPending { pending: true }),
            )
                .into_response(),
        }
    }

    /// The dictionary learns which of its terms the text used once the text is out
    /// ("Usage"), once per stream, off the request's task.
    fn record_usage(&self, text: &str) {
        if self.usage_recorded.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(dictionary) = self.dictionary.get().cloned() else {
            return;
        };
        let mut text = text.to_string();
        tokio::task::spawn_blocking(move || {
            let bumps = dictionary.usage_bumps(&text, chrono::Utc::now());
            text.zeroize();
            // Through the daemon's one handle (PERF-1): the flush reads the entries from its
            // warm cache under the lock, where a fresh handle parsed the whole file, and
            // leaves the counters it wrote there for the next stream.
            let store = super::dictionary::shared_store();
            let flushed = biorouter::dictation::record_usage(
                &store,
                &bumps,
                biorouter::security::dictionary_store::store::USAGE_FLUSH_BUDGET,
            );
            // The counters moved, so the dictionary cache copies the rows of the recent
            // sets again for them here, after the text is out, instead of in the next
            // stream's creation (2.8 ms at 5,000 rows in a release build, the PERF-1
            // review). A stream that starts meanwhile waits for this copy.
            if flushed {
                if let Ok(view) = store.view() {
                    dictionary_cache().prewarm(&view);
                }
            }
        });
    }
}

/// Drops a stream nobody touched for the idle timeout, unless it is still working.
async fn reap(
    shared: Arc<Shared>,
    stream: Weak<LiveStream>,
    mut activity: watch::Receiver<Instant>,
) {
    let idle = shared.limits.idle;
    loop {
        let last = *activity.borrow_and_update();
        tokio::select! {
            () = tokio::time::sleep_until((last + idle).into()) => {
                let Some(live) = stream.upgrade() else { return };
                if live.closed.load(Ordering::SeqCst) {
                    return;
                }
                if live.busy() {
                    live.touch();
                    continue;
                }
                {
                    let mut table = lock(&shared.table);
                    if table.get(&live.id).is_some_and(|held| Arc::ptr_eq(held, &live)) {
                        table.remove(&live.id);
                    }
                }
                live.close();
                tracing::info!(stream = %live.log_id, "dictation stream expired");
                return;
            }
            changed = activity.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// Places a piece decoded from a segment that starts at `offset_ms` and lasts
/// `length_ms`, its times `t0_ms` and `t1_ms` inside that segment: inside the segment, its
/// start never after its end. Whisper can time a piece past the audio it was given.
fn place(offset_ms: u64, length_ms: u64, t0_ms: u64, t1_ms: u64, text: &str) -> Piece {
    let t1_ms = (offset_ms + t1_ms).min(offset_ms + length_ms);
    let t0_ms = (offset_ms + t0_ms).min(t1_ms);
    Piece {
        t0_ms,
        t1_ms,
        text: text.to_string(),
    }
}

/// The decoder: one per stream, decoding its jobs in order on the shared worker, then the
/// transcript once the queue closes. Every return drops the job in hand and the queue,
/// which zeroes their copies of the audio.
async fn decode(
    stream: Arc<LiveStream>,
    dictionary: tokio::task::JoinHandle<DictionaryRequest>,
    jobs: mpsc::UnboundedReceiver<Job>,
) {
    let mut jobs = JobQueue(jobs);
    let dictionary = Arc::new(
        dictionary
            .await
            .unwrap_or_else(|_| DictionaryRequest::none()),
    );
    let _ = stream.dictionary.set(Arc::clone(&dictionary));
    let _ = service::service().install_bias(&dictionary);
    let request = TranscribeRequest {
        language: stream.language.clone(),
        cancel: stream.cancel.clone(),
        dictionary: Some(Arc::clone(&dictionary)),
        decode: None,
    };
    let prompt = dictionary.prompt_terms();
    let mut wait_ms = 0;
    let mut tail_ms = 0;
    let mut tail_audio_s = 0.0;
    while let Some(job) = jobs.next().await {
        if stream.closed.load(Ordering::SeqCst) {
            return;
        }
        if matches!(job, Job::Tail { .. }) {
            // A load still running at stop is waiting, not decoding.
            let mut prepared = stream.prepared.clone();
            let _ = prepared.wait_for(|ready| *ready).await;
        }
        let started = Instant::now();
        let result = match job {
            Job::Segment {
                pcm,
                offset_ms,
                speech_ms,
            } => {
                let length_ms = (pcm.len() as u64) / SAMPLES_PER_MS;
                let decoded = service::service()
                    .decode_segment(
                        stream.spec.clone(),
                        stream.key,
                        PcmSlice::whole(pcm.shared()),
                        Some(speech_ms),
                        &request,
                    )
                    .await;
                drop(pcm);
                decoded.map(|d| vec![(offset_ms, length_ms, d)])
            }
            Job::Tail {
                pcm,
                offset_ms,
                rate,
                open_speech_ms,
            } => {
                if let Some(finish_at) = stream.inner().finish_at {
                    wait_ms = started.saturating_duration_since(finish_at).as_millis() as u64;
                }
                let tail =
                    decode_tail(&stream, &request, pcm, offset_ms, rate, open_speech_ms).await;
                tail_ms = started.elapsed().as_millis() as u64;
                if let Ok((_, audio_ms)) = &tail {
                    tail_audio_s = *audio_ms as f64 / 1000.0;
                }
                tail.map(|(decoded, _)| decoded)
            }
        };
        let mut inner = stream.inner();
        stream.job_done(&mut inner);
        match result {
            Ok(decoded) => {
                for (offset_ms, length_ms, segment) in decoded {
                    inner.segments_decoded += 1;
                    if inner.retried.is_none() {
                        inner.retried = segment.retried;
                    }
                    inner.pieces.extend(segment.pieces.iter().map(|piece| {
                        place(offset_ms, length_ms, piece.t0_ms, piece.t1_ms, &piece.text)
                    }));
                }
                let raw = text::filter_whole_output(&text::join(
                    inner.pieces.iter().map(|p| p.text.as_str()),
                ));
                inner.partial = (!raw.is_empty()).then(|| dictionary.correct(&raw, &prompt).text);
            }
            Err(DictationError::Cancelled) => return,
            Err(error) => {
                tracing::info!(stream = %stream.log_id, "dictation stream failed");
                let refusal = refusal_of(&error, None);
                // The jobs still queued will never run: they stop counting now, so the
                // stream can expire and the daemon's queue is not held by it, and they
                // are dropped with the queue when this returns, their audio zeroed. The
                // stream takes no more audio: every later chunk answers this refusal.
                if let Some(shared) = stream.shared.upgrade() {
                    shared.queued.fetch_sub(inner.pending, Ordering::SeqCst);
                }
                inner.pending = 0;
                inner.jobs = None;
                inner.wipe();
                inner.wipe_text();
                drop(inner);
                stream.cancel.cancel();
                stream.outcome.send_replace(Some(Arc::new(Err(refusal))));
                return;
            }
        }
    }
    if stream.closed.load(Ordering::SeqCst) {
        return;
    }
    let mut inner = stream.inner();
    if !inner.finishing {
        return;
    }
    let raw_text =
        text::filter_whole_output(&text::join(inner.pieces.iter().map(|p| p.text.as_str())));
    let audio_s = inner.pcm.len() as f64 / f64::from(stream.rate);
    let segment = |p: &Piece, text: String| DictationTranscriptSegment {
        t0: p.t0_ms as f64 / 1000.0,
        t1: p.t1_ms as f64 / 1000.0,
        text,
    };
    let (text, changes, segments) = if raw_text.is_empty() {
        (String::new(), Vec::new(), Vec::new())
    } else {
        let corrected = dictionary.correct(&raw_text, &prompt);
        let segments = inner
            .pieces
            .iter()
            .map(|p| segment(p, dictionary.correct_piece(&p.text, &prompt)))
            .collect();
        let changes = corrected
            .changes
            .into_iter()
            .map(|c| DictationTranscriptChange {
                start: c.start as u64,
                end: c.end as u64,
                from: c.from,
                to: c.to,
                stage: c.stage.as_str().to_string(),
                entry_id: c.entry_id,
            })
            .collect();
        (corrected.text, changes, segments)
    };
    let elapsed_ms = inner
        .finish_at
        .map_or(0, |at| at.elapsed().as_millis() as u64);
    let transcript = DictationTranscript {
        no_speech: raw_text.is_empty(),
        text,
        raw_text,
        changes,
        segments,
        model: stream.model.clone(),
        audio_s,
        elapsed_ms,
        bias_scope: dictionary.scope().into(),
        retried: inner.retried.map(|r| DictationTranscriptRetry {
            reason: r.as_str().to_string(),
        }),
        timings: DictationTranscriptTimings {
            segments: inner.segments_decoded,
            wait_ms,
            tail_ms,
            tail_audio_s,
        },
    };
    // P-1: the audio goes as soon as the transcript is ready.
    inner.wipe();
    inner.wipe_text();
    drop(inner);
    tracing::info!(
        stream = %stream.log_id,
        target_kind = stream.target_kind.as_str(),
        bias_scope = dictionary.scope().as_str(),
        model = %stream.model,
        audio_ms = (audio_s * 1000.0) as u64,
        segments = transcript.timings.segments,
        stop_to_text_ms = elapsed_ms,
        wait_ms,
        tail_ms,
        "dictation stream transcribed"
    );
    stream.outcome.send_replace(Some(Arc::new(Ok(transcript))));
    if stream.closed.load(Ordering::SeqCst) {
        // A `DELETE` or the expiry crossed the answer: the transcript goes as the close
        // would have taken it.
        stream.retire_outcome();
    }
}

/// The open tail at stop. At 16 kHz and under 25 s it goes straight to `transcribe`,
/// whose integrated detector trims it; a longer one (speech that never paused) and a
/// whole recording at another rate are read by the detector first and planned like a
/// recording. A tail of exact zeros costs no request. Answers the decoded segments with
/// their offsets and lengths, and the tail's length. Every copy of the audio here is an
/// [`AudioCopy`] or is zeroed in place, so an error that returns early wipes it too.
async fn decode_tail(
    stream: &LiveStream,
    request: &TranscribeRequest,
    pcm: AudioCopy,
    offset_ms: u64,
    rate: u32,
    open_speech_ms: u64,
) -> Result<(Vec<(u64, u64, service::SegmentDecode)>, u64), DictationError> {
    let pcm = if rate == biorouter::dictation::audio::SAMPLE_RATE {
        pcm
    } else {
        let native = pcm.shared();
        let resampled = tokio::task::spawn_blocking(move || {
            let mut floats = Vec::with_capacity(native.len());
            biorouter::dictation::audio::s16_to_f32(&native, &mut floats);
            let out = biorouter::dictation::audio::resample_to_16k(&floats, rate);
            floats.zeroize();
            let mut out_floats = out;
            let samples = biorouter::dictation::audio::f32_to_s16(&out_floats);
            out_floats.zeroize();
            samples
        })
        .await
        .map_err(|e| DictationError::Internal(e.to_string()))?;
        drop(pcm);
        let mut resampled = resampled;
        let copy = AudioCopy::new(&resampled);
        resampled.zeroize();
        copy
    };
    let audio_ms = (pcm.len() as u64) / SAMPLES_PER_MS;
    if pcm.is_silent() {
        return Ok((Vec::new(), audio_ms));
    }
    // Declared after `pcm`, so it is dropped first on every return and `pcm` is the last
    // holder when it wipes.
    let whole = PcmSlice::whole(pcm.shared());
    let service = service::service();
    if audio_ms <= MAX_SEGMENT_MS && rate == biorouter::dictation::audio::SAMPLE_RATE {
        // The speech the last detector pass saw still open in the tail is a lower bound
        // on the tail's speech, for the collapse check; with none, the decode's own
        // pieces stand in.
        let speech = (open_speech_ms > 0).then_some(open_speech_ms);
        let decoded = service
            .decode_segment(stream.spec.clone(), stream.key, whole, speech, request)
            .await;
        return decoded.map(|d| (vec![(offset_ms, audio_ms, d)], audio_ms));
    }
    let spans = service
        .vad_segments_with(stream.spec.clone(), whole.clone())
        .await?;
    let mut decoded = Vec::new();
    for range in biorouter::dictation::file::pack(plan_final_segments(&spans, 0, audio_ms)) {
        let segment = service
            .decode_segment(
                stream.spec.clone(),
                stream.key,
                whole.slice_ms(&range),
                Some(speech_within(&spans, &range)),
                request,
            )
            .await?;
        decoded.push((offset_ms + range.start, range.end - range.start, segment));
    }
    Ok((decoded, audio_ms))
}

// ── status ───────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationEngineState {
    NoEngine,
    UnsupportedCpu,
    Stopped,
    Starting,
    Ready,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationEngine {
    pub compiled: bool,
    pub state: DictationEngineState,
    /// `metal` or `cpu`; `None` without an engine or on a refused processor.
    pub backend: Option<String>,
    pub device: Option<String>,
    pub threads: u32,
    pub missing_cpu_features: Vec<String>,
    pub rss_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationModelLocation {
    User,
    System,
    Managed,
    Env,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationSuitability {
    Recommended,
    Suitable,
    TooSlow,
    BelowMinimum,
    UnsupportedCpu,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationModel {
    pub id: String,
    pub display_name: String,
    /// A verified copy is in some folder.
    pub installed: bool,
    pub bytes: u64,
    pub location: Option<DictationModelLocation>,
    pub superseded: bool,
    pub suitability: DictationSuitability,
    /// The class default.
    pub recommended: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictationInstallPhase {
    Downloading,
    Verifying,
    Installed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationInstallJob {
    pub model: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub bytes_per_second: u64,
    pub phase: DictationInstallPhase,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationPolicyView {
    pub managed: bool,
    pub enabled: Allowance,
    pub capture: Allowance,
    pub install: Allowance,
    pub external_engine: Allowance,
    pub dictionary: Allowance,
    pub agent_writes: Allowance,
    pub allowed_models: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationLimits {
    pub max_seconds: u64,
    pub max_chunk_bytes: u64,
    pub max_streams: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationQueue {
    pub streams: u64,
    pub segments: u64,
}

/// `GET /dictation/status`. Reading it never starts a worker.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationStatus {
    pub engine: DictationEngine,
    /// Every catalog speech model, installed or not.
    pub models: Vec<DictationModel>,
    pub active_model: Option<String>,
    pub language: String,
    /// Whether this caller may install the recommended model now.
    pub install: Allowance,
    pub install_job: Option<DictationInstallJob>,
    pub policy: DictationPolicyView,
    pub limits: DictationLimits,
    pub queue: DictationQueue,
    pub account: String,
    pub host: String,
    pub models_dir: String,
}

fn engine_view(status: biorouter::dictation::EngineStatus) -> DictationEngine {
    use biorouter::dictation::EngineState as S;
    DictationEngine {
        compiled: status.compiled,
        state: match status.state {
            S::NoEngine => DictationEngineState::NoEngine,
            S::UnsupportedCpu => DictationEngineState::UnsupportedCpu,
            S::Stopped => DictationEngineState::Stopped,
            S::Starting => DictationEngineState::Starting,
            S::Ready => DictationEngineState::Ready,
            S::Error => DictationEngineState::Error,
        },
        backend: status.backend,
        device: status.device,
        threads: status.threads,
        missing_cpu_features: status.missing_cpu_features,
        rss_bytes: status.rss_bytes,
    }
}

fn location_view(location: paths::Location) -> DictationModelLocation {
    match location {
        paths::Location::User => DictationModelLocation::User,
        paths::Location::System => DictationModelLocation::System,
        paths::Location::Managed => DictationModelLocation::Managed,
        paths::Location::Env => DictationModelLocation::Env,
    }
}

fn suitability_view(suitability: hardware::Suitability) -> DictationSuitability {
    match suitability {
        hardware::Suitability::Recommended => DictationSuitability::Recommended,
        hardware::Suitability::Suitable => DictationSuitability::Suitable,
        hardware::Suitability::TooSlow => DictationSuitability::TooSlow,
        hardware::Suitability::BelowMinimum => DictationSuitability::BelowMinimum,
        hardware::Suitability::UnsupportedCpu => DictationSuitability::UnsupportedCpu,
        hardware::Suitability::Unknown => DictationSuitability::Unknown,
    }
}

/// Who may change the installed models (DD-20, O-2, O-17): the person on a daemon that
/// holds a user action key; anyone holding the secret on a keyless one. A daemon whose
/// launcher promised a key and got none keeps the keyed rule.
fn model_change_refusal(headers: &HeaderMap) -> Option<Refusal> {
    match user_action_proof(headers) {
        UserActionProof::Proven => None,
        UserActionProof::NoKeyInstalled
            if !biorouter_server::launch::expected_a_user_action_key() =>
        {
            None
        }
        _ => Some(Refusal::new(
            StatusCode::FORBIDDEN,
            "proofRequired",
            MSG_INSTALL_AGENT,
        )),
    }
}

/// Whether the operator locked browser installs on a keyless daemon.
fn browser_install_locked() -> bool {
    std::env::var(BROWSER_INSTALL_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        )
    })
}

/// `install` for this caller and `id` (the class default when `None`), in the precedence
/// the status documents: MSG-NO-ENGINE, MSG-POLICY, the CPU sentence, MSG-POLICY-DOWNLOADS
/// (and MSG-POLICY-MODELS), MSG-INSTALL-AGENT, MSG-INSTALL-LOCKED. So no interface offers
/// a button that would fail.
fn install_allowance(policy: &DictationPolicy, id: Option<&str>, headers: &HeaderMap) -> Allowance {
    let engine = service::spawn_refusal();
    if let Some(error @ DictationError::NotCompiled) = &engine {
        return Allowance::refused(error.to_string());
    }
    if !policy.enabled.is_allowed() {
        return policy.enabled.allowance();
    }
    if let Some(error) = engine {
        return Allowance::refused(error.to_string());
    }
    let managed = match id {
        Some(id) => policy.install(id),
        None => policy.model_downloads.allowance(),
    };
    if !managed.allowed {
        return managed;
    }
    if let Some(refusal) = model_change_refusal(headers) {
        return Allowance::refused(refusal.body.message);
    }
    if user_action_proof(headers) == UserActionProof::NoKeyInstalled && browser_install_locked() {
        return Allowance::refused(MSG_INSTALL_LOCKED);
    }
    Allowance::granted()
}

#[utoipa::path(
    get,
    path = "/dictation/status",
    responses(
        (status = 200, description = "The engine, the models, the policy and this caller's install allowance; never starts a worker", body = DictationStatus),
    ),
)]
pub async fn dictation_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<DictationStatus> {
    let streams = &state.dictation;
    let (policy, inventory_models, active) = tokio::task::spawn_blocking(|| {
        // One read of the policy for the whole answer, so a file with a key this
        // version does not know is warned about once per request.
        let policy = ManagedPolicy::load().dictation().clone();
        let roots = Roots::from_parts(
            &policy,
            std::env::var_os(paths::MODELS_DIR_ENV),
            paths::user_models_dir(),
            paths::system_models_dir(),
        );
        let mut inventory = paths::inventory(&roots, &CATALOG);
        let host = hardware::HostFacts::current();
        let active = service::active_model(
            &mut inventory,
            &policy,
            service::configured_model().as_deref(),
            host.class,
        )
        .map(|e| e.id.to_string());
        let default = CATALOG.default_for(host.class).map(|e| e.id);
        let models: Vec<DictationModel> = CATALOG
            .speech_models()
            .map(|entry| {
                let found = inventory.get(entry.id);
                let bench = service::current_bench(entry.id);
                DictationModel {
                    id: entry.id.to_string(),
                    display_name: entry.display_name.to_string(),
                    installed: found.is_some(),
                    bytes: entry.size_bytes,
                    location: found.map(|f| location_view(f.location)),
                    superseded: found.is_some_and(|f| f.superseded),
                    suitability: suitability_view(hardware::rate(
                        &CATALOG,
                        entry,
                        &host,
                        bench.as_ref(),
                    )),
                    recommended: default == Some(entry.id),
                }
            })
            .collect();
        (policy, models, active)
    })
    .await
    .unwrap_or_else(|_| (DictationPolicy::off(), Vec::new(), None));
    let class = service::host_class();
    let install = install_allowance(&policy, CATALOG.default_for(class).map(|e| e.id), &headers);
    let install_job = lock(&streams.shared.install).as_ref().map(|job| job.view());
    Json(DictationStatus {
        engine: engine_view(service::service().status()),
        models: inventory_models,
        active_model: active,
        language: service::configured_language(),
        install,
        install_job,
        policy: DictationPolicyView {
            managed: policy.managed,
            enabled: policy.enabled.allowance(),
            capture: policy.capture.allowance(),
            install: policy.model_downloads.allowance(),
            external_engine: policy.external_engine.allowance(),
            dictionary: policy.dictionary.allowance(),
            agent_writes: policy.dictionary_agent_writes.allowance(),
            allowed_models: policy.allowed_models.clone(),
        },
        limits: DictationLimits {
            max_seconds: MAX_STREAM_SECONDS,
            max_chunk_bytes: MAX_CHUNK_BYTES as u64,
            max_streams: streams.limits().max_streams as u64,
        },
        queue: DictationQueue {
            streams: streams.open() as u64,
            segments: streams.queued_segments() as u64,
        },
        account: service::account(),
        host: service::host(),
        models_dir: service::models_dir().display().to_string(),
    })
}

// ── streams ──────────────────────────────────────────────────────────────────────────

/// The bias target of DD-30, decided once per stream from what the daemon reads, never
/// from a tier the client states.
async fn stream_target(
    state: &AppState,
    request: &DictationStreamRequest,
    headers: &HeaderMap,
    standing: biorouter::dictation::Standing,
    proven: bool,
) -> BiasTarget {
    match (request.target_kind, request.session_id.as_deref()) {
        (DictationTargetKind::Chat | DictationTargetKind::Editor, Some(id)) => {
            // Resolve the tier before touching the session; a refusal degrades to public
            // terms silently, so the answer says nothing about another chat.
            let manager = state.session_manager();
            match session_reach::session_reach(manager, id, headers).await {
                Ok(()) => {
                    let stored = match session_reach::target_tier(manager, id).await {
                        TargetTier::Private => Some(SessionClassification::Private),
                        TargetTier::Public => Some(SessionClassification::Public),
                        TargetTier::Unreadable => None,
                    };
                    // One read of the live agent's bound provider: its tier and its
                    // model's affiliation. No live agent: the affiliation is unstated.
                    let live = match state.peek_agent(id).await {
                        Some(agent) => live_model(&agent, id).await,
                        None => None,
                    };
                    let reach =
                        biorouter::dictation::chat_reach(stored, live.map(|(tier, _)| tier));
                    BiasTarget {
                        private: reach.target_private(),
                        affiliation: live.and_then(|(_, affiliation)| affiliation),
                        person_included: false,
                    }
                }
                Err(_) => BiasTarget::public(),
            }
        }
        (DictationTargetKind::NewChat, _) => {
            // The per tab pick counts only with the proof, so an unproven caller cannot
            // choose the tier its own stream is judged by.
            let pick = request.new_chat_provider.as_deref().filter(|_| proven);
            let model = request.new_chat_model.as_deref().filter(|_| proven);
            let (tier, affiliation) = biorouter::dictation::new_chat_bind_tier(pick, model).await;
            BiasTarget {
                private: tier.is_private(),
                affiliation,
                person_included: false,
            }
        }
        (DictationTargetKind::Crew | DictationTargetKind::External, _) => {
            // O-8: for one recording the person confirmed after MSG-PRIVATE-TERMS; every
            // private row then passes, institution tagged rows included (O-37).
            let included = request.include_private_terms && proven && standing.is_private();
            BiasTarget {
                private: included,
                affiliation: None,
                person_included: included,
            }
        }
        _ => BiasTarget::public(),
    }
}

/// A live chat's bound model, its tier and its affiliation, for the stream target: read
/// within [`LIVE_MODEL_BOUND`], and never for a chat a Crew grant scopes, so the target
/// fails toward public terms instead of waiting. `Agent::provider` asks the Crew
/// workspace over SSH for such a chat when the grant's last admission is older than 15 s,
/// and forgets that admission first; a dictation must neither wait for a broker nor cost
/// the chat its admission. Such a chat is judged by its stored tier alone, with an
/// unstated affiliation. A provider the privacy check refuses (a public model on a
/// private row) reads as none, as before.
async fn live_model(
    agent: &biorouter::agents::Agent,
    session_id: &str,
) -> Option<(ProviderTier, Option<ModelAffiliation>)> {
    let read = async {
        let crew = biorouter::crew::manager().ok()?;
        if crew.is_scoped_session(session_id).await {
            return None;
        }
        let provider = agent.provider().await.ok()?;
        Some((provider.tier(), provider.affiliation()))
    };
    tokio::time::timeout(LIVE_MODEL_BOUND, read)
        .await
        .ok()
        .flatten()
}

/// DD-30's decision and the dictionary it admits, on a task of its own once the stream
/// exists: nothing in the 201 depends on it, and the decoder waits for it before its
/// first decode, while the model loads. Answers no dictionary if the store cannot be
/// read.
async fn stream_dictionary(
    state: Arc<AppState>,
    headers: HeaderMap,
    request: DictationStreamRequest,
    log_id: String,
) -> DictionaryRequest {
    let started = Instant::now();
    // DD-30: the target and the caller's standing, decided here, once.
    let caller = session_reach::http_caller(&headers).await;
    let standing = caller.dictation_standing();
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let proven = user_action_proof(&headers) == UserActionProof::Proven;
    let target = stream_target(&state, &request, &headers, standing, proven).await;
    let target_ms = started.elapsed().as_millis() as u64;
    let context = request.context.unwrap_or_default();
    let channels = BiasChannels::for_dictation();
    let (dictionary, store_us, request_us) = tokio::task::spawn_blocking(move || {
        // The daemon's one handle (PERF-1): its parse is cached across streams and left
        // warm by every write, so an unchanged store costs two `stat` calls here, not a
        // parse of the whole file, and its content stamp, which the bonus table's revision
        // follows, stays the same from one dictation to the next.
        let read = Instant::now();
        let view = super::dictionary::shared_store().view();
        let store_us = read.elapsed().as_micros() as u64;
        // The bias set's rules run in full; what they admitted is then found in the
        // daemon's cache, so an unchanged store ranks and builds the request's own part
        // alone (PERF-1).
        let built = Instant::now();
        let dictionary = match view {
            Ok(view) => dictionary_cache()
                .bias_request(&view, &target, standing, enforced, &context, channels),
            Err(_) => DictionaryRequest::new(
                biorouter::dictation::recognize::EligibleEntries::empty(),
                &context,
                channels,
            ),
        };
        (dictionary, store_us, built.elapsed().as_micros() as u64)
    })
    .await
    .unwrap_or_else(|_| (DictionaryRequest::none(), 0, 0));
    let dictionary_us = started.elapsed().as_micros() as u64;
    tracing::debug!(
        stream = %log_id,
        target_ms,
        store_us,
        request_us,
        dictionary_us,
        dictionary_ms = dictionary_us / 1000,
        "dictation stream target decided"
    );
    dictionary
}

/// The daemon's one [`DictionaryCache`]: what its dictations built from an unchanged
/// dictionary, kept across streams beside the one store handle they read
/// (`routes::dictionary::shared_store`). Public for the latency tests, which read its
/// counters; no other crate has a use for it.
#[doc(hidden)]
pub fn dictionary_cache() -> &'static DictionaryCache {
    static CACHE: DictionaryCache = DictionaryCache::new();
    &CACHE
}

#[utoipa::path(
    post,
    path = "/dictation/streams",
    request_body = DictationStreamRequest,
    responses(
        (status = 201, description = "The stream is open; the worker is being prepared", body = DictationStreamCreated),
        (status = 403, description = "Managed policy turns dictation or live capture off", body = DictationErrorBody),
        (status = 409, description = "No engine, an unsupported processor, no model, or an English only model asked for another language", body = DictationErrorBody),
        (status = 413, description = "A body over 8 KiB or a context over 2 KiB", body = DictationErrorBody),
        (status = 422, description = "A malformed body", body = DictationErrorBody),
        (status = 429, description = "Eight open streams or 32 queued segments; Retry-After set", body = DictationErrorBody),
        (status = 503, description = "Not enough free memory to load the model", body = DictationErrorBody),
    ),
)]
pub async fn create_dictation_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    match open_stream(&state, &headers, body).await {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

async fn open_stream(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<DictationStreamCreated, Refusal> {
    let request: DictationStreamRequest = json_body(body)?;
    let policy = ManagedPolicy::load().dictation().clone();
    if !policy.enabled.is_allowed() {
        return Err(Refusal::policy(policy.enabled.allowance()));
    }
    if !policy.capture.is_allowed() {
        return Err(Refusal::policy(policy.capture.allowance()));
    }
    if let Some(error) = service::spawn_refusal() {
        return Err(refusal_of(&error, None));
    }
    if request.format != "s16le" || !SAMPLE_RATES.contains(&request.sample_rate) {
        return Err(Refusal::bad_request());
    }
    if request.context.as_ref().map_or(0, String::len) > MAX_CONTEXT_BYTES {
        return Err(Refusal::too_large());
    }
    let streams = &state.dictation;
    if let Some(refusal) = streams.admission_refusal() {
        return Err(refusal);
    }
    let language = request
        .language
        .as_deref()
        .map(|l| service::language_setting(Some(l.to_string())))
        .unwrap_or_else(service::configured_language);
    let spec = match service::service().resolve_for(None, Some(&language)).await {
        Ok(spec) => spec,
        Err(error) => {
            let reason = install_allowance(
                &policy,
                CATALOG.default_for(service::host_class()).map(|e| e.id),
                headers,
            )
            .reason;
            return Err(refusal_of(&error, reason));
        }
    };
    let model = service::model_id(&spec);
    if let Some(short) = service::memory_refusal(&spec, &model) {
        return Err(Refusal::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "serverMemory",
            short.to_string(),
        ));
    }

    // The model loads while the person speaks, started before anything else here so no
    // decision below delays it. The tail waits for it before its clock starts, so a load
    // still running at stop is reported as `wait_ms`, not as decoding. A stream refused
    // below leaves only an idle load, which the idle unload frees.
    let prepare = spec.clone();
    let (prepared_tx, prepared) = watch::channel(false);
    tokio::spawn(async move {
        let _ = service::service().prepare_with(prepare).await;
        prepared_tx.send_replace(true);
    });

    let id = new_stream_id();
    let log_id = log_id_of(&id);
    // DD-30's target and the dictionary it admits, beside the model load and after the
    // 201: the answer does not depend on them, so a slow decision (a new chat's first
    // provider build) never holds the stream's creation.
    let target_kind = request.target_kind;
    let rate = request.sample_rate;
    let dictionary = tokio::spawn(stream_dictionary(
        Arc::clone(state),
        headers.clone(),
        request,
        log_id.clone(),
    ));

    let (jobs, queue) = mpsc::unbounded_channel();
    let (outcome, _) = watch::channel(None);
    let (activity, activity_rx) = watch::channel(Instant::now());
    let stream = Arc::new(LiveStream {
        id,
        log_id,
        key: next_stream_key(),
        target_kind,
        rate,
        spec,
        model,
        language,
        cancel: CancellationToken::new(),
        closed: AtomicBool::new(false),
        prepared,
        inner: Mutex::new(Inner::new(jobs)),
        dictionary: OnceLock::new(),
        outcome,
        activity,
        usage_recorded: AtomicBool::new(false),
        shared: Arc::downgrade(&streams.shared),
    });
    {
        let mut table = lock(&streams.shared.table);
        if table.len() >= streams.limits().max_streams {
            dictionary.abort();
            return Err(Refusal::busy("tooManyStreams"));
        }
        table.insert(stream.id.clone(), Arc::clone(&stream));
    }
    tokio::spawn(decode(Arc::clone(&stream), dictionary, queue));
    tokio::spawn(reap(
        Arc::clone(&streams.shared),
        Arc::downgrade(&stream),
        activity_rx,
    ));
    tracing::info!(
        stream = %stream.log_id,
        target_kind = target_kind.as_str(),
        model = %stream.model,
        sample_rate = rate,
        "dictation stream opened"
    );
    Ok(DictationStreamCreated {
        stream_id: stream.id.clone(),
        max_chunk_bytes: MAX_CHUNK_BYTES as u64,
        idle_timeout_s: streams.limits().idle.as_secs(),
        max_seconds: MAX_STREAM_SECONDS,
    })
}

/// One chunk into `stream`, as `PUT` and `finish` take it. `segment` is false for the tail
/// chunk `finish` carries: a detector pass started on it would only queue ahead of the
/// tail's own decode, which already trims the tail. A stream whose decode failed takes no
/// more audio and answers its failure, so the client says so while the person still
/// speaks rather than at the stop.
fn take_chunk(
    stream: &Arc<LiveStream>,
    seq: u64,
    body: &[u8],
    segment: bool,
) -> Result<DictationAudioAck, Refusal> {
    if let Some(failure) = stream.failure() {
        return Err(failure);
    }
    if body.len() > MAX_CHUNK_BYTES {
        return Err(Refusal::too_large());
    }
    let appended = stream
        .inner()
        .append(seq, body, stream.rate, stream.max_samples());
    let received_s = match appended {
        Appended::Accepted { received_s } | Appended::Repeat { received_s } => received_s,
        Appended::OutOfOrder { expected } => return Err(Refusal::out_of_order(expected)),
        Appended::TooLong => return Err(Refusal::too_large()),
        Appended::OddLength => return Err(Refusal::bad_request()),
        Appended::Finished => return Err(Refusal::new(StatusCode::CONFLICT, "finished", "")),
    };
    if segment && matches!(appended, Appended::Accepted { .. }) {
        stream.segment();
    }
    let mut inner = stream.inner();
    Ok(DictationAudioAck {
        received_s,
        partial: inner.take_partial(),
        queue_position: inner.pending as u64,
    })
}

#[utoipa::path(
    put,
    path = "/dictation/streams/{id}/audio",
    params(
        ("id" = String, Path, description = "The stream id"),
        DictationSeqQuery,
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "Raw 16 bit little endian samples, at most 524,288 B"),
    responses(
        (status = 200, description = "Appended, or a repeat answered as before", body = DictationAudioAck),
        (status = 409, description = "A seq above the next expected one", body = DictationErrorBody),
        (status = 410, description = "Expired, cancelled or delivered and dropped", body = DictationErrorBody),
        (status = 413, description = "A chunk over 524,288 B, or past 600 s of audio", body = DictationErrorBody),
        (status = 422, description = "An odd number of bytes", body = DictationErrorBody),
        (status = 500, description = "engineFailed: a decode failed while the person spoke; the stream takes no more audio", body = DictationErrorBody),
    ),
)]
pub async fn append_dictation_audio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<DictationSeqQuery>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(stream) = state.dictation.get(&id) else {
        return Refusal::gone().into_response();
    };
    stream.touch();
    let Ok(body) = body else {
        return Refusal::too_large().into_response();
    };
    match take_chunk(&stream, query.seq, &body, true) {
        Ok(ack) => (StatusCode::OK, Json(ack)).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/dictation/streams/{id}/finish",
    params(
        ("id" = String, Path, description = "The stream id"),
        DictationFinishQuery,
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "Optional: the tail chunk, with its seq in the query, so a stop costs one round trip"),
    responses(
        (status = 200, description = "The transcript", body = DictationTranscript),
        (status = 202, description = "Not ready after 3 s; read it with GET", body = DictationTranscriptPending),
        (status = 409, description = "The tail chunk's seq is above the next expected one", body = DictationErrorBody),
        (status = 410, description = "Expired or cancelled", body = DictationErrorBody),
        (status = 413, description = "A tail chunk over 524,288 B", body = DictationErrorBody),
        (status = 500, description = "engineFailed: the worker exited or missed its deadline, at the stop or while the person spoke", body = DictationErrorBody),
    ),
)]
pub async fn finish_dictation_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<DictationFinishQuery>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(stream) = state.dictation.get(&id) else {
        return Refusal::gone().into_response();
    };
    stream.touch();
    let Ok(body) = body else {
        return Refusal::too_large().into_response();
    };
    if !body.is_empty() {
        let Some(seq) = query.seq else {
            return Refusal::bad_request().into_response();
        };
        if let Err(refusal) = take_chunk(&stream, seq, &body, false) {
            if refusal.body.code != "finished" {
                return refusal.into_response();
            }
        }
    }
    stream.finish();
    stream.answer(FINISH_WAIT).await
}

#[utoipa::path(
    get,
    path = "/dictation/streams/{id}",
    params(
        ("id" = String, Path, description = "The stream id"),
        DictationReadQuery,
    ),
    responses(
        (status = 200, description = "The transcript", body = DictationTranscript),
        (status = 202, description = "Not ready yet", body = DictationTranscriptPending),
        (status = 410, description = "Expired, cancelled, or delivered and dropped", body = DictationErrorBody),
        (status = 500, description = "engineFailed", body = DictationErrorBody),
    ),
)]
pub async fn get_dictation_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<DictationReadQuery>,
) -> Response {
    let Some(stream) = state.dictation.get(&id) else {
        return Refusal::gone().into_response();
    };
    let wait = Duration::from_millis(query.wait_ms.unwrap_or(0).min(MAX_READ_WAIT_MS));
    stream.answer(wait).await
}

#[utoipa::path(
    delete,
    path = "/dictation/streams/{id}",
    params(("id" = String, Path, description = "The stream id")),
    responses((status = 204, description = "Dropped: queued segments cancelled, audio and transcript zeroed")),
)]
pub async fn delete_dictation_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> StatusCode {
    if let Some(stream) = state.dictation.remove(&id) {
        stream.close();
    }
    StatusCode::NO_CONTENT
}

// ── models ───────────────────────────────────────────────────────────────────────────

/// An install the daemon runs in the background; its progress is in the status.
struct InstallJob {
    model: &'static str,
    /// Every file it fetches, in order, with its size.
    files: Vec<(&'static str, u64)>,
    started: Instant,
    cancel: Arc<AtomicBool>,
    progress: watch::Receiver<Progress>,
    outcome: Mutex<Option<Result<(), String>>>,
}

impl InstallJob {
    fn running(&self) -> bool {
        lock(&self.outcome).is_none()
    }

    fn view(&self) -> DictationInstallJob {
        let total: u64 = self.files.iter().map(|(_, size)| size).sum();
        let progress = self.progress.borrow().clone();
        let before: u64 = self
            .files
            .iter()
            .take_while(|(id, _)| *id != progress.id)
            .map(|(_, size)| size)
            .sum();
        let current = if self.files.iter().any(|(id, _)| *id == progress.id) {
            progress.downloaded
        } else {
            0
        };
        let (phase, done, error) = match lock(&self.outcome).as_ref() {
            Some(Ok(())) => (DictationInstallPhase::Installed, total, None),
            Some(Err(error)) => (
                DictationInstallPhase::Failed,
                (before + current).min(total),
                Some(error.clone()),
            ),
            None => (
                match progress.phase {
                    Phase::Verifying => DictationInstallPhase::Verifying,
                    _ => DictationInstallPhase::Downloading,
                },
                (before + current).min(total),
                None,
            ),
        };
        let seconds = self.started.elapsed().as_secs_f64().max(0.001);
        DictationInstallJob {
            model: self.model.to_string(),
            bytes_done: done,
            bytes_total: total,
            bytes_per_second: (done as f64 / seconds) as u64,
            phase,
            error,
        }
    }
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct DictationModelRequest {
    /// A catalog id.
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationInstallStarted {
    pub model: String,
    pub phase: DictationInstallPhase,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationCancelled {
    pub cancelled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationModelsRemoved {
    /// The ids removed from this account's folder: the model, and its voice detector
    /// when no other speech model there needs it.
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct DictationSelectRequest {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationSelection {
    pub active_model: Option<String>,
    pub language: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationRescan {
    /// Catalog ids placed from files found by their SHA-256.
    pub added: Vec<String>,
    /// File names no catalog pin matched.
    pub unrecognized: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DictationBench {
    pub model: String,
    pub rtf: f64,
    pub elapsed_ms: u64,
}

/// A catalog speech model by id (a retired id stands for its replacement).
fn speech_entry(id: &str) -> Result<&'static SttCatalogEntry, Refusal> {
    match CATALOG.resolve(id.trim()) {
        Some((entry, _)) if entry.is_speech() => Ok(entry),
        _ => Err(Refusal::bad_request()),
    }
}

#[utoipa::path(
    post,
    path = "/dictation/models/install",
    request_body = DictationModelRequest,
    responses(
        (status = 200, description = "Already installed", body = DictationInstallStarted),
        (status = 202, description = "Installing; progress is in the status", body = DictationInstallStarted),
        (status = 403, description = "policy, proofRequired (MSG-INSTALL-AGENT) or installLocked (MSG-INSTALL-LOCKED)", body = DictationErrorBody),
        (status = 409, description = "notCompiled, unsupportedCpu or installBusy", body = DictationErrorBody),
        (status = 422, description = "Not a catalog speech model", body = DictationErrorBody),
    ),
)]
pub async fn install_dictation_model(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    match start_install(&state, &headers, body).await {
        Ok((status, started)) => (status, Json(started)).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

async fn start_install(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, DictationInstallStarted), Refusal> {
    let request: DictationModelRequest = json_body(body)?;
    let policy = ManagedPolicy::load().dictation().clone();
    if !policy.enabled.is_allowed() {
        return Err(Refusal::policy(policy.enabled.allowance()));
    }
    if let Some(error) = service::spawn_refusal() {
        return Err(refusal_of(&error, None));
    }
    let entry = speech_entry(&request.model)?;
    let allowance = policy.install(entry.id);
    if !allowance.allowed {
        return Err(Refusal::policy(allowance));
    }
    if let Some(refusal) = model_change_refusal(headers) {
        return Err(refusal);
    }
    if user_action_proof(headers) == UserActionProof::NoKeyInstalled && browser_install_locked() {
        return Err(Refusal::new(
            StatusCode::FORBIDDEN,
            "installLocked",
            MSG_INSTALL_LOCKED,
        ));
    }
    let roots = Roots::current();
    let target = roots
        .writable(false)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(Refusal::bad_request)?;
    if roots.managed_only && roots.managed.as_deref() != Some(target.as_path()) {
        return Err(Refusal::policy(policy.model_downloads.allowance()));
    }
    // A file verified in any folder serves this account; an earlier revision counts as
    // missing, because an install fetches the current one (DD-23).
    let files = {
        let roots = roots.clone();
        tokio::task::spawn_blocking(move || {
            CATALOG
                .with_requirements(entry)
                .into_iter()
                .filter(|file| paths::find(&roots, file).is_none_or(|f| f.superseded))
                .collect::<Vec<&'static SttCatalogEntry>>()
        })
        .await
        .map_err(|_| Refusal::bad_request())?
    };
    if files.is_empty() {
        return Ok((
            StatusCode::OK,
            DictationInstallStarted {
                model: entry.id.to_string(),
                phase: DictationInstallPhase::Installed,
            },
        ));
    }
    let (progress_tx, progress) = watch::channel(Progress {
        id: String::new(),
        phase: Phase::Connecting,
        downloaded: 0,
        total: 0,
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let job = Arc::new(InstallJob {
        model: entry.id,
        files: files.iter().map(|f| (f.id, f.size_bytes)).collect(),
        started: Instant::now(),
        cancel: Arc::clone(&cancel),
        progress,
        outcome: Mutex::new(None),
    });
    {
        let mut slot = lock(&state.dictation.shared.install);
        if slot.as_ref().is_some_and(|running| running.running()) {
            return Err(Refusal::new(StatusCode::CONFLICT, "installBusy", ""));
        }
        *slot = Some(Arc::clone(&job));
    }
    let mut options = InstallOptions::new(Network::production(), WriteMode::User);
    options.progress = Some(progress_tx);
    options.cancel = cancel;
    let streams = Arc::clone(&state.dictation);
    tokio::spawn(async move {
        let result = models::install_files(&target, &files, &options).await;
        drop(options);
        match result {
            Ok(outcomes) => {
                tracing::info!(
                    model = job.model,
                    elapsed_ms = job.started.elapsed().as_millis() as u64,
                    "dictation model installed"
                );
                *lock(&job.outcome) = Some(Ok(()));
                // The first load after an install compiles the model's Metal pipelines,
                // 3.0 to 4.2 s [M]; the warm up pays that now, not at the first stop.
                if outcomes.iter().any(|o| o.id == job.model && o.downloaded) {
                    warm_up_in_background(vec![job.model], "install", &streams);
                }
            }
            Err(error) => {
                let message = match error {
                    models::InstallError::Offline => biorouter::dictation::msg_offline(&target),
                    other => other.to_string(),
                };
                tracing::info!(model = job.model, "dictation model install stopped");
                *lock(&job.outcome) = Some(Err(message));
            }
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        DictationInstallStarted {
            model: entry.id.to_string(),
            phase: DictationInstallPhase::Downloading,
        },
    ))
}

#[utoipa::path(
    post,
    path = "/dictation/models/cancel",
    request_body = DictationModelRequest,
    responses((status = 200, description = "Whether a running install of that model was cancelled; its partial stays for a resume", body = DictationCancelled)),
)]
pub async fn cancel_dictation_install(
    State(state): State<Arc<AppState>>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let request: DictationModelRequest = match json_body(body) {
        Ok(request) => request,
        Err(refusal) => return refusal.into_response(),
    };
    let wanted = CATALOG
        .resolve(request.model.trim())
        .map(|(entry, _)| entry.id);
    let cancelled = lock(&state.dictation.shared.install)
        .as_ref()
        .filter(|job| job.running() && Some(job.model) == wanted)
        .map(|job| job.cancel.swap(true, Ordering::SeqCst))
        .is_some();
    (StatusCode::OK, Json(DictationCancelled { cancelled })).into_response()
}

#[utoipa::path(
    post,
    path = "/dictation/models/delete",
    request_body = DictationModelRequest,
    responses(
        (status = 200, description = "Removed from this account's folder; never refused by policy", body = DictationModelsRemoved),
        (status = 403, description = "proofRequired on a daemon that holds a user action key", body = DictationErrorBody),
        (status = 409, description = "notDeletable (a system, managed or env copy) or installBusy", body = DictationErrorBody),
        (status = 422, description = "Not a catalog model", body = DictationErrorBody),
    ),
)]
pub async fn delete_dictation_model(
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    match remove_model(&headers, body).await {
        Ok(removed) => (StatusCode::OK, Json(removed)).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

async fn remove_model(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<DictationModelsRemoved, Refusal> {
    let request: DictationModelRequest = json_body(body)?;
    if let Some(refusal) = model_change_refusal(headers) {
        return Err(refusal);
    }
    let entry = CATALOG
        .resolve(request.model.trim())
        .map(|(entry, _)| entry)
        .ok_or_else(Refusal::bad_request)?;
    let roots = Roots::current();
    let target = roots.user.clone();
    let path = models::model_path(&target, entry);
    if !path.exists() {
        let elsewhere = tokio::task::spawn_blocking({
            let roots = roots.clone();
            move || paths::find(&roots, entry).is_some()
        })
        .await
        .unwrap_or(false);
        if elsewhere {
            return Err(Refusal::new(StatusCode::CONFLICT, "notDeletable", ""));
        }
        return Ok(DictationModelsRemoved {
            removed: Vec::new(),
        });
    }
    // The worker may hold the file open: it stops first.
    service::service().unload().await;
    tokio::task::spawn_blocking(move || {
        let needs_detector = |vad: &SttCatalogEntry| {
            CATALOG.requiring(vad.id).any(|other| {
                other.id != entry.id
                    && (models::model_path(&target, other).exists()
                        || models::install_running(&target, other))
            })
        };
        if !entry.is_speech() && needs_detector(entry) {
            return Err(Refusal::new(StatusCode::CONFLICT, "notDeletable", ""));
        }
        let busy = |e: models::RemoveError| match e {
            models::RemoveError::Busy { .. } => {
                Refusal::new(StatusCode::CONFLICT, "installBusy", "")
            }
            other => Refusal::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "removeFailed",
                other.to_string(),
            ),
        };
        let mut removed = Vec::new();
        if models::remove(&target, entry).map_err(busy)? {
            removed.push(entry.id.to_string());
        }
        if entry.is_speech() {
            for vad in entry.requires.iter().filter_map(|id| CATALOG.entry(id)) {
                if !needs_detector(vad) && models::remove(&target, vad).map_err(busy)? {
                    removed.push(vad.id.to_string());
                }
            }
        }
        Ok(DictationModelsRemoved { removed })
    })
    .await
    .map_err(|_| Refusal::bad_request())?
}

#[utoipa::path(
    post,
    path = "/dictation/models/select",
    request_body = DictationSelectRequest,
    responses(
        (status = 200, description = "Saved as BIOROUTER_DICTATION_MODEL and BIOROUTER_DICTATION_LANGUAGE", body = DictationSelection),
        (status = 403, description = "policy, or proofRequired on a daemon that holds a user action key", body = DictationErrorBody),
        (status = 409, description = "noModel, or languageUnsupported for an English only model", body = DictationErrorBody),
        (status = 422, description = "An unknown model or language", body = DictationErrorBody),
    ),
)]
pub async fn select_dictation_model(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    match select(&state, &headers, body).await {
        Ok(selection) => (StatusCode::OK, Json(selection)).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

async fn select(
    state: &AppState,
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<DictationSelection, Refusal> {
    let request: DictationSelectRequest = json_body(body)?;
    // MSG-POLICY first, as for an install: a choice that can never load is refused
    // whoever asks.
    let policy = ManagedPolicy::load().dictation().clone();
    if !policy.enabled.is_allowed() {
        return Err(Refusal::policy(policy.enabled.allowance()));
    }
    if let Some(refusal) = model_change_refusal(headers) {
        return Err(refusal);
    }
    let language = match request.language.as_deref() {
        Some(language) => {
            let language = service::language_setting(Some(language.to_string()));
            service::check_language_code(&language).map_err(|_| Refusal::bad_request())?;
            language
        }
        None => service::configured_language(),
    };
    let config = biorouter::config::Config::global();
    if let Some(id) = request.model.as_deref() {
        let entry = speech_entry(id)?;
        // The checks a load makes: policy, the language, a verified copy.
        let spec = service::service()
            .resolve_for(Some(entry.id), Some(&language))
            .await
            .map_err(|e| refusal_of(&e, None))?;
        drop(spec);
        let before = service::configured_model()
            .and_then(|id| CATALOG.resolve(&id).map(|(entry, _)| entry.id))
            .or_else(|| CATALOG.default_for(service::host_class()).map(|e| e.id));
        config
            .set_param(service::MODEL_KEY, entry.id)
            .map_err(|e| {
                Refusal::new(StatusCode::INTERNAL_SERVER_ERROR, "notSaved", e.to_string())
            })?;
        if before != Some(entry.id) {
            warm_up_in_background(vec![entry.id], "select", &state.dictation);
        }
    } else if let Some(active) = service::configured_model() {
        if let Some((entry, _)) = CATALOG.resolve(&active) {
            service::check_language(entry, &language).map_err(|e| refusal_of(&e, None))?;
        }
    }
    if request.language.is_some() {
        config
            .set_param(service::LANGUAGE_KEY, language.as_str())
            .map_err(|e| {
                Refusal::new(StatusCode::INTERNAL_SERVER_ERROR, "notSaved", e.to_string())
            })?;
    }
    Ok(DictationSelection {
        active_model: service::configured_model(),
        language,
    })
}

#[utoipa::path(
    post,
    path = "/dictation/models/rescan",
    responses(
        (status = 200, description = "Files in the models folders identified by SHA-256 against the catalog, and placed in this account's folder unless managed policy forbids downloads, when nothing is placed", body = DictationRescan),
        (status = 403, description = "policy: managed policy turns dictation off", body = DictationErrorBody),
    ),
)]
pub async fn rescan_dictation_models(State(state): State<Arc<AppState>>) -> Response {
    match tokio::task::spawn_blocking(rescan).await {
        Ok(Ok(rescan)) => {
            warm_up_placed(&rescan.added, &state.dictation);
            (StatusCode::OK, Json(rescan)).into_response()
        }
        Ok(Err(refusal)) => refusal.into_response(),
        Err(_) => (
            StatusCode::OK,
            Json(DictationRescan {
                added: Vec::new(),
                unrecognized: Vec::new(),
            }),
        )
            .into_response(),
    }
}

/// The speech models among `added` that a rescan placed, warmed up in the background.
fn warm_up_placed(added: &[String], streams: &DictationStreams) {
    let placed: Vec<&'static str> = added
        .iter()
        .filter_map(|id| CATALOG.entry(id))
        .filter(|entry| entry.is_speech())
        .map(|entry| entry.id)
        .collect();
    warm_up_in_background(placed, "rescan", streams);
}

/// The first load of a model by an executable whose Metal cache lacks that model's
/// pipelines compiles them, 3.0 to 4.2 s [M] (measured live for turbo: 3,459 ms to a
/// ready model, against 805 ms once compiled), and the startup warm up covers only the
/// model that was active then. So a model an install or a rescan placed, or the person
/// just chose, is loaded and decoded once now, in the background, with the active model
/// last so it is the one left loaded. Metal only, and never in an administrator's
/// account (the cache belongs to the account that compiles). Each warm up is skipped
/// while a stream is open: the worker holds one model, so loading another would make a
/// dictation in progress load its own again at the stop.
fn warm_up_in_background(
    mut ids: Vec<&'static str>,
    why: &'static str,
    streams: &DictationStreams,
) {
    if ids.is_empty() || service::default_backend() != "metal" || models::running_as_administrator()
    {
        return;
    }
    let shared = Arc::clone(&streams.shared);
    tokio::spawn(async move {
        if let Ok(spec) = service::service().resolve(None).await {
            let active = service::model_id(&spec);
            ids.sort_by_key(|id| *id == active);
        }
        for id in ids {
            if !lock(&shared.table).is_empty() {
                tracing::info!(
                    model = id,
                    why,
                    "dictation model warm up skipped beside a stream"
                );
                continue;
            }
            if let Ok(warm) = service::service().warm_up(id).await {
                tracing::info!(
                    model = %warm.model,
                    why,
                    ready_ms = warm.ready_ms,
                    decode_ms = warm.decode_ms,
                    "dictation model warm up finished"
                );
            }
        }
    });
}

/// The rescan, under the policy's choke points (`engine-and-models.md`, "Choke points"):
/// refused with MSG-POLICY under `enabled: false`; under `model_downloads: false`, or
/// where only the managed folder may hold models, it identifies what it finds and places
/// nothing; and a speech model `allowed_models` leaves out is never placed. A file that
/// is a catalog file but is not placed is not `unrecognized`. A file it placed from the
/// top of this account's own folder is moved, not copied: the original is removed once
/// its copy is verified in place, so a model is not kept twice (measured live: the
/// three catalog files stayed beside their placed copies, 840 MB held twice).
fn rescan() -> Result<DictationRescan, Refusal> {
    let policy = ManagedPolicy::load().dictation().clone();
    if !policy.enabled.is_allowed() {
        return Err(Refusal::policy(policy.enabled.allowance()));
    }
    let roots = Roots::from_parts(
        &policy,
        std::env::var_os(paths::MODELS_DIR_ENV),
        paths::user_models_dir(),
        paths::system_models_dir(),
    );
    let place = policy.model_downloads.is_allowed() && !roots.managed_only;
    let inventory = paths::inventory(&roots, &CATALOG);
    let space: models::SpaceProbe =
        Arc::new(|path: &std::path::Path| models::available_space(path));
    let allow =
        |entry: &SttCatalogEntry| place && (!entry.is_speech() || policy.model(entry.id).allowed);
    let mut rescan = DictationRescan {
        added: Vec::new(),
        unrecognized: Vec::new(),
    };
    for stray in inventory.strays.iter().filter(|s| s.path.is_file()) {
        let name = stray
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match models::import_from(
            &CATALOG,
            &roots.user,
            &stray.path,
            WriteMode::User,
            &allow,
            &space,
        ) {
            Ok(report) if !report.installed.is_empty() => {
                rescan
                    .added
                    .extend(report.installed.iter().map(|(id, _)| id.to_string()));
                if stray.path.parent() == Some(roots.user.as_path()) {
                    let _ = std::fs::remove_file(&stray.path);
                }
            }
            Ok(report) if !report.already.is_empty() || !report.refused.is_empty() => {}
            _ => rescan.unrecognized.push(name),
        }
    }
    Ok(rescan)
}

#[utoipa::path(
    post,
    path = "/dictation/bench",
    responses(
        (status = 200, description = "The active model's real time factor on this host, recorded for suitability", body = DictationBench),
        (status = 403, description = "policy", body = DictationErrorBody),
        (status = 409, description = "notCompiled, unsupportedCpu or noModel", body = DictationErrorBody),
        (status = 500, description = "engineFailed", body = DictationErrorBody),
    ),
)]
pub async fn bench_dictation_model() -> Response {
    match service::service().bench(None).await {
        Ok((model, record)) => (
            StatusCode::OK,
            Json(DictationBench {
                model,
                rtf: record.rtf,
                elapsed_ms: record.elapsed_ms,
            }),
        )
            .into_response(),
        Err(error) => refusal_of(&error, None).into_response(),
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/dictation/status", get(dictation_status))
        .route(
            "/dictation/streams",
            post(create_dictation_stream).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictation/streams/{id}/audio",
            put(append_dictation_audio).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
        .route(
            "/dictation/streams/{id}/finish",
            post(finish_dictation_stream).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
        .route(
            "/dictation/streams/{id}",
            get(get_dictation_stream).delete(delete_dictation_stream),
        )
        .route(
            "/dictation/models/install",
            post(install_dictation_model).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictation/models/cancel",
            post(cancel_dictation_install).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictation/models/delete",
            post(delete_dictation_model).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/dictation/models/select",
            post(select_dictation_model).layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route("/dictation/models/rescan", post(rescan_dictation_models))
        .route("/dictation/bench", post(bench_dictation_model))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn inner() -> Inner {
        let (jobs, _queue) = mpsc::unbounded_channel();
        Inner::new(jobs)
    }

    /// DD-28: strictly increasing `seq`; a repeat is answered as before and its body
    /// ignored; a higher one names the expected value; nothing past the cap, nothing odd,
    /// nothing after the stop.
    #[test]
    fn append_follows_the_seq_rules() {
        let mut inner = inner();
        let a = chunk(&[1; 16_000]);
        assert_eq!(
            inner.append(0, &a, 16_000, 64_000),
            Appended::Accepted { received_s: 1.0 }
        );
        assert_eq!(
            inner.append(2, &a, 16_000, 64_000),
            Appended::OutOfOrder { expected: 1 }
        );
        assert_eq!(
            inner.append(1, &chunk(&[2; 8_000]), 16_000, 64_000),
            Appended::Accepted { received_s: 1.5 }
        );
        // The first, larger body arrived; its halved repeat is answered with the full
        // chunk's total and changes nothing.
        assert_eq!(
            inner.append(1, &chunk(&[9; 4_000]), 16_000, 64_000),
            Appended::Repeat { received_s: 1.5 }
        );
        assert_eq!(inner.pcm.len(), 24_000);
        assert_eq!(inner.pcm[23_999], 2);
        assert_eq!(
            inner.append(2, &[0, 0, 0], 16_000, 64_000),
            Appended::OddLength
        );
        assert_eq!(
            inner.append(2, &chunk(&[0; 40_001]), 16_000, 64_000),
            Appended::TooLong
        );
        assert_eq!(inner.next_seq, 2, "a refused chunk takes no seq");
        inner.finishing = true;
        assert_eq!(inner.append(2, &a, 16_000, 64_000), Appended::Finished);
        assert_eq!(
            inner.append(0, &a, 16_000, 64_000),
            Appended::Repeat { received_s: 1.0 },
            "a repeat after the stop is still a repeat"
        );
        // Another rate counts its own seconds.
        let mut native = self::inner();
        assert_eq!(
            native.append(0, &chunk(&[1; 48_000]), 48_000, 48_000 * 600),
            Appended::Accepted { received_s: 1.0 }
        );
    }

    /// The partial is reported once per change (DD-31).
    #[test]
    fn the_partial_is_reported_when_it_changes() {
        let mut inner = inner();
        assert_eq!(inner.take_partial(), None);
        inner.partial = Some("one".into());
        assert_eq!(inner.take_partial().as_deref(), Some("one"));
        assert_eq!(inner.take_partial(), None);
        inner.partial = Some("one two".into());
        assert_eq!(inner.take_partial().as_deref(), Some("one two"));
    }

    /// P-1: growing the buffer leaves no copy behind, and a wipe zeroes it.
    #[test]
    fn the_buffer_grows_and_wipes() {
        let mut buf: Vec<i16> = Vec::with_capacity(4);
        buf.extend_from_slice(&[7, 7, 7, 7]);
        reserve_zeroizing(&mut buf, 10, 1_000_000);
        assert_eq!(buf, [7, 7, 7, 7]);
        assert!(buf.capacity() >= 14);
        let mut inner = inner();
        inner.pcm = vec![5; 100];
        inner.pieces.push(Piece {
            t0_ms: 0,
            t1_ms: 1,
            text: "spoken".into(),
        });
        inner.partial = Some("spoken".into());
        inner.wipe();
        inner.wipe_text();
        assert!(inner.pcm.is_empty());
        assert!(inner.pieces.is_empty() && inner.partial.is_none());
        let copy = copy_ms(
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
            &(0..1),
        );
        assert_eq!(copy.len(), 16);
        let shared = copy.shared();
        let wiped = WIPED_SAMPLES.with(|w| w.get());
        drop(copy);
        assert_eq!(shared[0], 1, "a copy still shared is left alone");
        assert_eq!(WIPED_SAMPLES.with(|w| w.get()), wiped);
        drop(shared);
        drop(copy_ms(&[3; 32], &(0..2)));
        assert_eq!(
            WIPED_SAMPLES.with(|w| w.get()),
            wiped + 32,
            "a copy alone is zeroed"
        );
    }

    /// P-1 on the paths that do not end in a transcript: jobs still queued when the
    /// decoder ends (a `DELETE`, the expiry, an engine that failed) are zeroed when it
    /// drops its queue, and the job in hand when it is dropped.
    #[test]
    fn every_queued_copy_is_zeroed_when_the_decoder_ends() {
        let (jobs, queue) = mpsc::unbounded_channel();
        let before = WIPED_SAMPLES.with(|w| w.get());
        for n in 0..5 {
            let pcm = AudioCopy::new(&vec![7; 16_000 + n]);
            let job = if n == 4 {
                Job::Tail {
                    pcm,
                    offset_ms: 0,
                    rate: 16_000,
                    open_speech_ms: 0,
                }
            } else {
                Job::Segment {
                    pcm,
                    offset_ms: 0,
                    speech_ms: 0,
                }
            };
            assert!(jobs.send(job).is_ok(), "queued");
        }
        let mut queue = JobQueue(queue);
        let in_hand = queue.0.try_recv().expect("a job");
        // The sender is still held, as a stream that was not finished holds it.
        drop(queue);
        let queued: usize = (1..5).map(|n| 16_000 + n).sum();
        assert_eq!(WIPED_SAMPLES.with(|w| w.get()), before + queued);
        drop(in_hand);
        assert_eq!(WIPED_SAMPLES.with(|w| w.get()), before + queued + 16_000);
        assert!(jobs.is_closed(), "the decoder's end closed the queue");
    }

    /// A transcript retired by a close keeps no text (P-1).
    #[test]
    fn a_retired_transcript_is_zeroed() {
        let mut transcript = DictationTranscript {
            text: "EPAS1".into(),
            raw_text: "a pass one".into(),
            changes: vec![DictationTranscriptChange {
                start: 0,
                end: 5,
                from: "a pass one".into(),
                to: "EPAS1".into(),
                stage: "alias".into(),
                entry_id: "e".into(),
            }],
            segments: vec![DictationTranscriptSegment {
                t0: 0.0,
                t1: 1.0,
                text: "EPAS1".into(),
            }],
            model: "m".into(),
            audio_s: 1.0,
            elapsed_ms: 0,
            bias_scope: DictationBiasScope::Private,
            retried: None,
            no_speech: false,
            timings: DictationTranscriptTimings {
                segments: 1,
                wait_ms: 0,
                tail_ms: 0,
                tail_audio_s: 1.0,
            },
        };
        wipe_transcript(&mut transcript);
        assert!(transcript.text.is_empty() && transcript.raw_text.is_empty());
        assert!(transcript.changes[0].from.is_empty() && transcript.changes[0].to.is_empty());
        assert!(transcript.segments[0].text.is_empty());
    }

    /// DD-93: a log line names a stream by a digest of its id, never the id that opens it.
    #[test]
    fn a_stream_is_logged_by_a_digest_of_its_id() {
        let id = new_stream_id();
        let log_id = log_id_of(&id);
        assert_eq!(log_id.len(), 8);
        assert!(!id.contains(&log_id), "the digest is not a piece of the id");
        assert_eq!(log_id, log_id_of(&id));
        assert_ne!(log_id, log_id_of(&new_stream_id()));
    }

    #[test]
    fn refusals_carry_their_code_and_retry_after() {
        let busy = Refusal::busy("tooManyStreams").into_response();
        assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            busy.headers().get(header::RETRY_AFTER).unwrap(),
            &RETRY_AFTER_SECONDS.to_string()
        );
        assert_eq!(Refusal::out_of_order(3).body.expected_seq, Some(3));
        assert_eq!(
            refusal_of(&DictationError::NotCompiled, None).body.code,
            "notCompiled"
        );
        let no_model = refusal_of(
            &DictationError::NoModel {
                host: "h".into(),
                account: "a".into(),
            },
            Some(MSG_INSTALL_LOCKED.into()),
        );
        assert_eq!(
            (
                no_model.status,
                no_model.body.code.as_str(),
                no_model.body.message.as_str()
            ),
            (StatusCode::CONFLICT, "noModel", MSG_INSTALL_LOCKED)
        );
        assert_eq!(
            refusal_of(&DictationError::WorkerExited { code: Some(9) }, None)
                .body
                .code,
            "engineFailed"
        );
    }

    #[test]
    fn the_install_lock_reads_its_switch() {
        for (value, locked) in [
            (Some("false"), true),
            (Some("0"), true),
            (Some(" OFF "), true),
            (Some("true"), false),
            (None, false),
        ] {
            let _env = env_lock::lock_env([(BROWSER_INSTALL_ENV, value)]);
            assert_eq!(browser_install_locked(), locked, "{value:?}");
        }
    }

    /// Every full segmenter pass moves the mark: closed speech is cut as DD-50 says, a
    /// window with no speech is read once, and speech with no pause for more than one
    /// decode is cut every 25 s, so the stop is left at most one decode.
    #[test]
    fn every_full_pass_moves_the_mark() {
        let span = |t0_ms, t1_ms| Span { t0_ms, t1_ms };
        // Closed speech: plan_closed_segments decides, as before.
        let (cut, mark) = plan_pass(&[span(1_000, 4_000), span(6_000, 9_000)], 0, 9_000);
        assert_eq!(cut.len(), 1);
        assert!(mark > 4_000 && mark < 6_000, "{mark}");
        // Too little audio: nothing.
        assert_eq!(plan_pass(&[span(0, 3_000)], 0, 7_000), (Vec::new(), 0));
        // No speech at all: read once, a second kept for a word just starting.
        assert_eq!(plan_pass(&[], 10_000, 30_000), (Vec::new(), 29_000));
        // Open speech under 33 s: wait for a pause.
        assert_eq!(plan_pass(&[span(0, 32_000)], 0, 32_000), (Vec::new(), 0));
        // Open speech past 33 s: cut at 25 s, and again from there.
        let (cut, mark) = plan_pass(&[span(0, 40_000)], 0, 40_000);
        assert_eq!(
            (cut.len(), cut.first().cloned(), mark),
            (1, Some(0..25_000), 25_000)
        );
        let (cut, mark) = plan_pass(&[span(0, 60_000)], 25_000, 60_000);
        assert_eq!(
            (cut.len(), cut.first().cloned(), mark),
            (1, Some(25_000..50_000), 50_000)
        );
        // A minute of open speech in one pass of at most 60 s never stalls.
        let mut mark = 0;
        let spans = [span(0, 600_000)];
        while mark + 33_000 <= 600_000 {
            let end = (mark + MAX_DETECTOR_PASS_MS).min(600_000);
            let (_, next) = plan_pass(&spans, mark, end);
            assert!(next > mark, "stalled at {mark}");
            mark = next;
        }
    }

    /// A pass is due once 8 s lie past the mark, reads at most 60 s, and after a pass
    /// that moved nothing waits for audio past what that pass read.
    #[test]
    fn a_pass_that_moved_nothing_waits_for_new_audio() {
        let mut inner = inner();
        let max = 600 * 16_000;
        let second = chunk(&vec![1; 16_000]);
        for seq in 0..7 {
            inner.append(seq, &second, 16_000, max);
        }
        assert_eq!(inner.due_window(), None, "under 8 s past the mark");
        for seq in 7..20 {
            inner.append(seq, &second, 16_000, max);
        }
        assert_eq!(inner.due_window(), Some(0..20_000));
        inner.stalled_at_ms = Some(20_000);
        assert_eq!(inner.due_window(), None, "the same audio again");
        inner.append(20, &second, 16_000, max);
        assert_eq!(inner.due_window(), Some(0..21_000), "audio past it");
        for seq in 21..70 {
            inner.append(seq, &second, 16_000, max);
        }
        assert_eq!(inner.due_window(), Some(0..60_000), "at most 60 s a pass");
        inner.mark_ms = 25_000;
        inner.stalled_at_ms = None;
        assert_eq!(inner.due_window(), Some(25_000..70_000));
    }

    /// DD-32: a new stream answers 429 once 32 segments are queued across every stream,
    /// as it does at eight open streams.
    #[test]
    fn a_full_queue_refuses_a_new_stream() {
        let streams = DictationStreams::new(StreamLimits::default());
        assert_eq!(streams.admission_refusal(), None);
        streams
            .shared
            .queued
            .store(MAX_QUEUED_SEGMENTS - 1, Ordering::SeqCst);
        assert_eq!(streams.admission_refusal(), None);
        streams.shared.queued.fetch_add(1, Ordering::SeqCst);
        let refusal = streams.admission_refusal().expect("refused");
        assert_eq!(refusal.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(refusal.body.code, "tooManyStreams");
        assert_eq!(refusal.retry_after, Some(RETRY_AFTER_SECONDS));
    }

    /// Live streams take keys of their own, two at a time.
    #[test]
    fn stream_keys_and_ids() {
        let a = next_stream_key();
        let b = next_stream_key();
        assert_eq!(b - a, 2);
        assert!(a >= 1 << 61);
        let id = new_stream_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id, new_stream_id());
    }
}
