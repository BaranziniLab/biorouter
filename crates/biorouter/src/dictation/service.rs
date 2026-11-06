//! One dictation service per process (DD-6), in a `OnceLock` like
//! `llamacpp_sidecar::global`.
//!
//! The service is the parent half of the engine: it runs the CPU gate before any spawn,
//! builds the worker's command (DD-4: the command is a value the leaf crate's
//! supervisor runs), works out the thread count (DD-32), and owns the supervisor, which
//! starts lazily on the first request on the caller's runtime. [`DictationService::status`]
//! never spawns and never waits.
//!
//! For latency, [`DictationService::prepare_with`] starts the worker process before it
//! works out the thread count, so the two overlap, and the caller that arms a
//! microphone does not await it: the model loads while the person speaks.
//!
//! [`DictationService::resolve`] turns a catalog id (or the configured one) into the
//! files the worker loads, through the lookup order of [`super::paths`] and the
//! verification cache (PR-11). [`DictationService::warm_up`] runs one decode right after
//! an install, so the first dictation on Metal does not pay the 3 to 4 s shader compile.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use biorouter_dictation::audio::{PcmSlice, Span};
use biorouter_dictation::cpu::{self, CpuReport, HardwareClass};
use biorouter_dictation::protocol::{Hello, Loaded, TranscribeResult, VadOptions};
use biorouter_dictation::supervisor::{
    CommandSpawner, LoadSpec, Supervisor, SupervisorConfig, SupervisorError, SupervisorStatus,
    TranscribeParams, WorkerState,
};
use biorouter_dictation::worker::{ENGINE_ROLE, WORKER_MARKER};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use biorouter_dictation::catalog::{SttCatalogEntry, CATALOG};
use biorouter_dictation::hardware::{self, BenchRecord};

use super::file::RecordingTranscript;
use super::paths::{self, Installed, Roots};
use crate::config::paths::Paths;
use crate::config::Config;
use crate::managed::{Allowance, DictationPolicy};

/// `config.yaml`: seconds without a request before the worker is stopped (DD-91).
pub const IDLE_UNLOAD_KEY: &str = "BIOROUTER_DICTATION_IDLE_UNLOAD_SECS";
/// `config.yaml`: the language a multilingual model is told to expect.
pub const LANGUAGE_KEY: &str = "BIOROUTER_DICTATION_LANGUAGE";
/// Environment only: an absolute path to the binary the worker runs from.
pub const WORKER_ENV: &str = "BIOROUTER_DICTATION_WORKER";
/// `config.yaml`: the catalog id dictation uses (DD-17); `biorouter dictation use` writes it.
pub const MODEL_KEY: &str = "BIOROUTER_DICTATION_MODEL";
/// The speech the warm up decodes: long enough for a real decoder pass, short enough to
/// cost little on Metal.
pub const WARM_UP_CLIP_MS: u64 = 2_000;
/// The idle unload when `config.yaml` sets none (DD-7).
pub const DEFAULT_IDLE_UNLOAD_SECS: u64 = 300;
/// How long the thread count waits for the llama sidecar's status (DD-32).
const SIDECAR_PROBE: Duration = Duration::from_millis(250);
/// How long a worked out thread count is reused, so a stream's segments do not each
/// ask the sidecar again.
const THREADS_REUSE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    /// This binary carries no engine.
    NoEngine,
    /// The processor lacks instructions the engine needs.
    UnsupportedCpu,
    Stopped,
    /// Spawning or loading.
    Starting,
    /// Loaded, idle or busy.
    Ready,
    /// The last start failed; the next request tries again.
    Error,
}

/// What the engine was built from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineBuild {
    pub whisper: String,
    pub ggml: String,
    pub isa: String,
}

/// The `engine` object of `biorouter dictation status --format json` (schema 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineStatus {
    pub compiled: bool,
    pub state: EngineState,
    /// `"metal"` or `"cpu"`: the running worker's backend, else the one this build
    /// tries first; `None` for `no_engine` and `unsupported_cpu`.
    pub backend: Option<String>,
    pub device: Option<String>,
    pub threads: u32,
    /// Lower case ids: `avx2`, `fma`, `f16c`, `bmi2`, `dotprod`.
    pub missing_cpu_features: Vec<String>,
    pub rss_bytes: Option<u64>,
    pub build: Option<EngineBuild>,
    /// MSG-CPU or MSG-CPU-ROSETTA, then MSG-CPU-VM, for a refused processor. The text
    /// status prints them; schema 1 has no field for them.
    #[serde(skip)]
    pub cpu_sentences: Vec<String>,
}

/// Why a dictation request failed. `Display` is a sentence of the message catalog
/// (`docs/dictation/surfaces.md`) that carries no transcript or audio; only `Protocol`
/// names the worker binary. The pull requests that first refuse for a missing model, a
/// language or the server's memory add those variants with their catalog text (PR-11,
/// PR-12, PR-15).
#[derive(Debug)]
pub enum DictationError {
    /// MSG-NO-ENGINE.
    NotCompiled,
    /// MSG-POLICY and its narrower siblings: the allowance's own sentence.
    Policy(Allowance),
    /// MSG-CPU or MSG-CPU-ROSETTA.
    UnsupportedCpu {
        sentence: String,
    },
    WorkerExited {
        code: Option<i32>,
    },
    Deadline {
        phase: &'static str,
    },
    /// Names the worker binary: a `BIOROUTER_DICTATION_WORKER` that speaks another
    /// protocol, or is not an absolute path.
    Protocol(String),
    ModelLoadFailed {
        reason: String,
    },
    BadAudio,
    Cancelled,
    Internal(String),
    /// An id the catalog does not know.
    UnknownModel(String),
    /// A catalog speech model with no verified copy in any folder.
    NotInstalled {
        id: String,
    },
    /// The speech model is there and the voice detector it needs is not. The repair is
    /// installing the speech model again, which fetches only the detector; installing the
    /// detector by its own id is refused (MSG-DETECTOR-ONLY).
    DetectorNotInstalled {
        model: String,
        detector: String,
    },
    /// No speech model is installed at all: MSG-NO-MODEL-CLI's first sentence.
    NoModel {
        host: String,
        account: String,
    },
    /// MSG-LANGUAGE-EN.
    EnglishOnly {
        model: String,
    },
    /// A language code the engine does not know (PR-12): neither `auto` nor one of
    /// whisper.cpp's codes.
    LanguageUnsupported {
        language: String,
    },
}

impl std::fmt::Display for DictationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DictationError::NotCompiled => f.write_str(super::MSG_NO_ENGINE),
            DictationError::Policy(allowance) => f.write_str(
                allowance
                    .reason
                    .as_deref()
                    .unwrap_or(crate::managed::MANAGED_DICTATION_OFF),
            ),
            DictationError::UnsupportedCpu { sentence } => f.write_str(sentence),
            DictationError::WorkerExited { code: Some(code) } => {
                write!(f, "The dictation worker stopped with exit code {code}.")
            }
            DictationError::WorkerExited { code: None } => {
                f.write_str("The dictation worker was stopped.")
            }
            DictationError::Deadline { phase } => {
                write!(f, "The dictation worker did not answer in time ({phase}).")
            }
            DictationError::Protocol(message) => f.write_str(message),
            DictationError::ModelLoadFailed { reason } => {
                write!(f, "The speech model could not be loaded: {reason}.")
            }
            DictationError::BadAudio => f.write_str("The audio could not be used."),
            DictationError::Cancelled => f.write_str("Dictation was cancelled."),
            DictationError::Internal(message) => write!(f, "Dictation failed: {message}."),
            DictationError::UnknownModel(id) => write!(
                f,
                "There is no speech model named {id}. Run `biorouter dictation list` to see the models."
            ),
            DictationError::NotInstalled { id } => write!(
                f,
                "{id} is not installed on this computer. Run `biorouter dictation install {id}`."
            ),
            DictationError::DetectorNotInstalled { model, detector } => write!(
                f,
                "{model} needs the voice detector {detector}, which is not installed on this computer. Run `biorouter dictation install {model}`."
            ),
            DictationError::NoModel { host, account } => write!(
                f,
                "Dictation needs a speech model on this computer ({host}, account {account}). Run `biorouter dictation install`."
            ),
            DictationError::EnglishOnly { model } => write!(
                f,
                "{model} understands only English. Choose a multilingual model for other languages, or set the language to English."
            ),
            DictationError::LanguageUnsupported { language } => write!(
                f,
                "Biorouter does not know the language {language:?}. Use a two letter code such as en or de, or auto to let the model decide."
            ),
        }
    }
}

impl std::error::Error for DictationError {}

impl From<SupervisorError> for DictationError {
    fn from(error: SupervisorError) -> DictationError {
        match error {
            SupervisorError::UnsupportedCpu { missing } => {
                let ids: Vec<&str> = missing.iter().map(String::as_str).collect();
                DictationError::UnsupportedCpu {
                    sentence: cpu::MSG_CPU.replace("{missing}", &cpu::display_list(&ids)),
                }
            }
            SupervisorError::WorkerExited { code } => DictationError::WorkerExited { code },
            SupervisorError::Deadline { phase } => DictationError::Deadline { phase },
            SupervisorError::Protocol(message) => DictationError::Protocol(message),
            SupervisorError::ModelLoadFailed(reason) => DictationError::ModelLoadFailed { reason },
            SupervisorError::BadAudio => DictationError::BadAudio,
            SupervisorError::Cancelled => DictationError::Cancelled,
            SupervisorError::Internal(message) => DictationError::Internal(message),
            SupervisorError::Spawn(e) => {
                DictationError::Internal(format!("the worker could not start ({})", e.kind()))
            }
            SupervisorError::Stopped => {
                DictationError::Internal("the dictation supervisor has stopped".into())
            }
        }
    }
}

/// A transcription request other than its audio. PR-14 adds the bias set and the
/// context.
#[derive(Clone, Debug)]
pub struct TranscribeRequest {
    pub language: String,
    pub cancel: CancellationToken,
}

impl TranscribeRequest {
    pub fn new(language: impl Into<String>) -> TranscribeRequest {
        TranscribeRequest {
            language: language.into(),
            cancel: CancellationToken::new(),
        }
    }
}

/// What [`DictationService::warm_up`] measured.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WarmUp {
    pub model: String,
    pub backend: String,
    /// The lookup and the verification cache: id to load spec.
    pub resolve_ms: u64,
    /// Spawn to `loaded`, as the caller saw it.
    pub ready_ms: u64,
    /// The worker's own load and warm up figures.
    pub load_ms: u64,
    pub warmup_ms: u64,
    /// The one decode of [`WARM_UP_CLIP_MS`] of speech.
    pub decode_ms: u64,
}

/// `status --verbose`: a worker's `hello`, or why there was none.
#[derive(Debug)]
pub struct WorkerProbe {
    pub hello: Result<Hello, DictationError>,
    /// The worker's kept stderr lines, control characters escaped (DD-93).
    pub stderr: Vec<String>,
    /// The probe worker's exit code; `None` when the running worker answered.
    pub exit_code: Option<i32>,
}

pub struct DictationService {
    supervisor: Mutex<Option<Arc<Supervisor>>>,
    threads: Mutex<Option<(Instant, u32)>>,
    /// Set by [`DictationService::one_shot`].
    one_shot: AtomicBool,
}

/// The process's one service.
pub fn service() -> &'static DictationService {
    static SERVICE: OnceLock<DictationService> = OnceLock::new();
    SERVICE.get_or_init(|| DictationService {
        supervisor: Mutex::new(None),
        threads: Mutex::new(None),
        one_shot: AtomicBool::new(false),
    })
}

impl DictationService {
    /// The engine's state. Never spawns, never waits, never probes the sidecar.
    pub fn status(&self) -> EngineStatus {
        let supervisor = self
            .supervisor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .filter(|s| s.is_running())
            .map(|s| s.status());
        status_from(
            biorouter_dictation::ENGINE_COMPILED,
            cpu::host_report(),
            supervisor.as_ref(),
            base_threads(),
        )
    }

    /// Spawns the worker and loads `spec`, resolving when the model is ready. A
    /// `n_threads` of 0 asks the service for DD-32's count. The process starts first, so
    /// its start overlaps the sidecar probe the thread count needs.
    pub async fn prepare_with(&self, mut spec: LoadSpec) -> Result<Loaded, DictationError> {
        let supervisor = self.ready_supervisor()?;
        supervisor.spawn_ahead();
        if spec.n_threads == 0 {
            spec.n_threads = self.threads().await;
        }
        Ok(supervisor.prepare(Arc::new(spec)).await?)
    }

    /// Decodes at most 25 s of `pcm`. Segments of one `stream` are decoded in order;
    /// streams take turns. Cancelling `req.cancel` answers `Cancelled` at once and
    /// aborts the decode in the worker.
    pub async fn transcribe_with(
        &self,
        mut spec: LoadSpec,
        stream: u64,
        pcm: PcmSlice,
        req: TranscribeRequest,
    ) -> Result<TranscribeResult, DictationError> {
        let supervisor = self.ready_supervisor()?;
        if spec.n_threads == 0 {
            spec.n_threads = self.request_threads(&supervisor).await;
        }
        let params = TranscribeParams::new(req.language, spec.n_threads);
        let (job, answer) = supervisor.submit_transcribe(Arc::new(spec), stream, pcm, params);
        tokio::select! {
            answer = answer => answer.unwrap_or(Err(SupervisorError::Stopped)).map_err(Into::into),
            () = req.cancel.cancelled() => {
                supervisor.cancel(job);
                Err(DictationError::Cancelled)
            }
        }
    }

    /// The voice detector's spans in at most 600 s of `pcm`.
    pub async fn vad_segments_with(
        &self,
        mut spec: LoadSpec,
        pcm: PcmSlice,
    ) -> Result<Vec<Span>, DictationError> {
        let supervisor = self.ready_supervisor()?;
        if spec.n_threads == 0 {
            spec.n_threads = self.request_threads(&supervisor).await;
        }
        Ok(supervisor
            .vad_segments(Arc::new(spec), pcm, VadOptions::dd48())
            .await?)
    }

    /// Starts the worker process now, before the model is chosen or the audio is ready,
    /// after the gates every spawn passes. Returns at once: the process start (about 80 ms
    /// to `hello` [M]) overlaps whatever the caller does next, such as decoding a file
    /// and resolving the model.
    pub fn spawn_ahead(&self) -> Result<(), DictationError> {
        self.ready_supervisor()?.spawn_ahead();
        Ok(())
    }

    /// A whole recording of 16 kHz mono samples (PR-12, [`super::file`]): the detector in
    /// windows of at most 600 s, every closed stretch of speech queued for decoding as soon
    /// as it is planned, at most 25 s per decode, one model load for all of it.
    /// Cancelling `req.cancel` answers `Cancelled` at once and drops the recording's jobs.
    pub async fn transcribe_recording_with(
        &self,
        mut spec: LoadSpec,
        pcm: Arc<[i16]>,
        req: TranscribeRequest,
    ) -> Result<RecordingTranscript, DictationError> {
        let supervisor = self.ready_supervisor()?;
        if spec.n_threads == 0 {
            spec.n_threads = self.request_threads(&supervisor).await;
        }
        let params = TranscribeParams::new(req.language, spec.n_threads);
        super::file::transcribe_recording(&supervisor, Arc::new(spec), pcm, params, &req.cancel)
            .await
    }

    /// A request's thread count: the loaded worker's, read from memory, so the path
    /// from stop to text never waits for the sidecar; DD-32's count when nothing is
    /// loaded yet, worked out while the worker process starts, as `prepare_with` does.
    async fn request_threads(&self, supervisor: &Supervisor) -> u32 {
        match supervisor.status().threads {
            Some(threads) => threads,
            None => {
                supervisor.spawn_ahead();
                self.threads().await
            }
        }
    }

    /// The files the worker loads for `id`, or for the configured model when `id` is
    /// `None` (DD-17, DD-19): the first verified copy of the model and of its voice
    /// detector in lookup order. Refused for an unknown or uninstalled id, a model the
    /// organization's policy leaves out, and a `.en` model while the language is not
    /// English. It sits on the arming path, so it looks up only what it will load, after
    /// the policy and language checks: no other model is opened, and with no `id` the
    /// candidates are tried in order and the first usable one ends the search. Hashing a
    /// copy that changed runs off the runtime's threads.
    pub async fn resolve(&self, id: Option<&str>) -> Result<LoadSpec, DictationError> {
        self.resolve_for(id, None).await
    }

    /// [`DictationService::resolve`] for a request in `language` (`None`: the configured
    /// one), which must be a code the engine knows, and English for a `.en` model. In a
    /// debug build, with no `id`, [`TEST_MODEL_ENV`] names a pinned fixture to load in
    /// place of a catalog model.
    pub async fn resolve_for(
        &self,
        id: Option<&str>,
        language: Option<&str>,
    ) -> Result<LoadSpec, DictationError> {
        let id = id.map(str::to_string);
        let language = language.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            let policy = crate::managed::ManagedPolicy::load().dictation().clone();
            let language = language.unwrap_or_else(configured_language);
            if id.is_none() {
                if let Some(fixture) = test_model() {
                    if !policy.enabled.is_allowed() {
                        return Err(DictationError::Policy(policy.enabled.allowance()));
                    }
                    return test_model_spec(&fixture, &language);
                }
            }
            let roots = Roots::current();
            spec_for(
                &mut paths::Lookup::new(&roots),
                &policy,
                id.as_deref(),
                configured_model().as_deref(),
                &language,
                cpu::host_report().class,
            )
        })
        .await
        .map_err(|e| DictationError::Internal(e.to_string()))?
    }

    /// Loads `id` (or the configured model), resolving when it is ready.
    pub async fn prepare(&self, id: Option<&str>) -> Result<Loaded, DictationError> {
        let spec = self.resolve(id).await?;
        self.prepare_with(spec).await
    }

    /// One decode right after an install (PR-11): loads `id` and decodes
    /// [`WARM_UP_CLIP_MS`] of the bench clip, so the Metal pipelines a model needs are
    /// compiled into the per user Metal cache before the first dictation (a cold cache
    /// costs 3.0 to 4.2 s [M]). On a CPU host there is nothing to compile, and a turbo
    /// decode would burn seconds of CPU, so only a GPU backend runs it.
    pub async fn warm_up(&self, id: &str) -> Result<WarmUp, DictationError> {
        let started = Instant::now();
        let spec = self.resolve(Some(id)).await?;
        let resolve_ms = started.elapsed().as_millis() as u64;
        let loaded = self.prepare_with(spec.clone()).await?;
        let ready_ms = started.elapsed().as_millis() as u64;
        let mut decode_ms = 0;
        if loaded.backend == "metal" {
            let clip = hardware::bench_clip(WARM_UP_CLIP_MS).ok_or(DictationError::NotCompiled)?;
            let decode = Instant::now();
            self.transcribe_with(
                spec,
                0,
                PcmSlice::whole(clip.into()),
                TranscribeRequest::new("en"),
            )
            .await?;
            decode_ms = decode.elapsed().as_millis() as u64;
        }
        Ok(WarmUp {
            model: id.to_string(),
            backend: loaded.backend,
            resolve_ms,
            ready_ms,
            load_ms: loaded.load_ms,
            warmup_ms: loaded.warmup_ms,
            decode_ms,
        })
    }

    /// `biorouter dictation bench` and **Test speed** (DD-22): loads the model, decodes
    /// the bench clip once to warm and once to time, and records the real time factor in
    /// `bench.json`. The clip is [`hardware::BENCH_CLIP_MS`] long, the length of a
    /// dictated sentence, because the encoder's cost does not shrink with the audio.
    pub async fn bench(&self, id: Option<&str>) -> Result<(String, BenchRecord), DictationError> {
        let mut spec = self.resolve(id).await?;
        let model = CATALOG
            .by_sha256(&spec.model_sha256)
            .map(|entry| entry.id.to_string())
            .unwrap_or_default();
        if spec.n_threads == 0 {
            spec.n_threads = self.threads().await;
        }
        let loaded = self.prepare_with(spec.clone()).await?;
        let clip: Arc<[i16]> = hardware::bench_clip(hardware::BENCH_CLIP_MS)
            .ok_or(DictationError::NotCompiled)?
            .into();
        let audio_ms = (clip.len() as u64) / biorouter_dictation::audio::SAMPLES_PER_MS;
        let mut elapsed = Duration::ZERO;
        for _ in 0..2 {
            let started = Instant::now();
            self.transcribe_with(
                spec.clone(),
                0,
                PcmSlice::whole(Arc::clone(&clip)),
                TranscribeRequest::new("en"),
            )
            .await?;
            elapsed = started.elapsed();
        }
        let record = BenchRecord {
            rtf: elapsed.as_secs_f64() / (audio_ms as f64 / 1000.0),
            elapsed_ms: elapsed.as_millis() as u64,
            audio_ms,
            threads: spec.n_threads,
            backend: loaded.backend,
            measured_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        hardware::write_bench(&bench_path(), &model, record.clone()).map_err(|e| {
            DictationError::Internal(format!("the bench result was not saved ({e})"))
        })?;
        Ok((model, record))
    }

    /// For `status --verbose`: the running worker's `hello`, or one from a worker
    /// started only to say it. Refused, before anything starts, while the organization's
    /// policy turns dictation off.
    pub async fn probe_worker(&self) -> Result<WorkerProbe, DictationError> {
        let enabled = crate::managed::ManagedPolicy::load()
            .dictation()
            .enabled
            .clone();
        if !enabled.is_allowed() {
            return Err(DictationError::Policy(enabled.allowance()));
        }
        let supervisor = self.ready_supervisor()?;
        let outcome = supervisor.probe().await;
        Ok(WorkerProbe {
            hello: outcome.hello.map_err(DictationError::from),
            stderr: outcome.stderr,
            exit_code: outcome.exit_code,
        })
    }

    /// Stops the worker after cancelling its work; returns once it has exited. Runs
    /// before a model is deleted or replaced.
    pub async fn unload(&self) {
        let supervisor = self
            .supervisor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(supervisor) = supervisor.filter(|s| s.is_running()) {
            supervisor.unload().await;
        }
    }

    /// DD-32: `min(8, available_parallelism)`, halved (at least 1) while the llama
    /// sidecar is serving. The sidecar gets 250 ms to answer; silence counts as not
    /// serving. The loopback Ollama half needs agent state and arrives with PR-15.
    pub async fn threads(&self) -> u32 {
        if let Some((at, n)) = *self.threads.lock().unwrap_or_else(PoisonError::into_inner) {
            if at.elapsed() < THREADS_REUSE {
                return n;
            }
        }
        // The sidecar's status does synchronous work before its first await (it looks
        // for the binary and builds an HTTP client, which reads the system's root
        // certificates), and a timeout cannot cut that short: inline, the probe took up to
        // 305 ms [M]. On a task of its own the 250 ms bound holds, and the caller's task
        // never runs that work.
        let probe =
            tokio::spawn(async { crate::providers::llamacpp_sidecar::global().status().await });
        let serving = tokio::time::timeout(SIDECAR_PROBE, probe)
            .await
            .ok()
            .and_then(Result::ok)
            .is_some_and(|status| {
                matches!(
                    status.state,
                    crate::providers::llamacpp_sidecar::SidecarState::Ready
                        | crate::providers::llamacpp_sidecar::SidecarState::Starting
                )
            });
        let n = threads_rule(base_threads(), serving);
        *self.threads.lock().unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), n));
        n
    }

    /// The supervisor, after the gates every spawn passes: an engine is linked, the
    /// processor can run it, and the worker binary is known. Started on the caller's
    /// runtime, and started again if that runtime has since ended.
    fn ready_supervisor(&self) -> Result<Arc<Supervisor>, DictationError> {
        gate(biorouter_dictation::ENGINE_COMPILED, cpu::host_report())?;
        let program = worker_program().map_err(DictationError::Protocol)?;
        let mut slot = self
            .supervisor
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(supervisor) = slot.as_ref().filter(|s| s.is_running()) {
            return Ok(Arc::clone(supervisor));
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(DictationError::Internal(
                "dictation needs an async runtime".into(),
            ));
        }
        let name = program.display().to_string();
        let spawner = CommandSpawner::new(name, move || worker_command(&program));
        let config = SupervisorConfig {
            warm_up: !self.one_shot.load(Ordering::Relaxed),
            ..supervisor_config()
        };
        let supervisor = Arc::new(Supervisor::start(Arc::new(spawner), config));
        *slot = Some(Arc::clone(&supervisor));
        Ok(supervisor)
    }

    /// Marks this process as one that loads one model for one recording, such as
    /// `biorouter transcribe`: its loads skip the encoder warm up (`Load::warm_up`), which
    /// on a warm Metal cache cost turbo 250 ms and `small.en` 56 ms of a one shot run and
    /// saved the first decode 6 to 14 ms [M]. The daemon and the interactive front ends
    /// never call it, so their first dictation still finds the encoder warm. Call it
    /// before the first spawn; a supervisor already running keeps its own setting.
    pub fn one_shot(&self) {
        self.one_shot.store(true, Ordering::Relaxed);
    }
}

/// Why nothing can run here, before any download: no engine in this build, or a
/// processor the engine cannot use (MSG-NO-ENGINE, MSG-CPU). `None` when it can.
pub fn spawn_refusal() -> Option<DictationError> {
    gate(biorouter_dictation::ENGINE_COMPILED, cpu::host_report()).err()
}

/// This computer's hardware class (`AS`, `CPU` or `NO`).
pub fn host_class() -> cpu::HardwareClass {
    cpu::host_report().class
}

/// The checks every spawn passes, in order: an engine is linked, then the processor.
fn gate(compiled: bool, report: &CpuReport) -> Result<(), DictationError> {
    if !compiled {
        return Err(DictationError::NotCompiled);
    }
    if report.class == HardwareClass::No {
        return Err(DictationError::UnsupportedCpu {
            sentence: cpu_sentences(report).join(" "),
        });
    }
    Ok(())
}

fn cpu_sentences(report: &CpuReport) -> Vec<String> {
    let mut sentences: Vec<String> = cpu::sentence(report).into_iter().collect();
    sentences.extend(cpu::vm_sentence(report).map(str::to_string));
    sentences
}

/// The status a supervisor's snapshot gives, or the parent's own answer when there is
/// no engine or the processor is refused.
fn status_from(
    compiled: bool,
    report: &CpuReport,
    supervisor: Option<&SupervisorStatus>,
    threads: u32,
) -> EngineStatus {
    let build = compiled.then(|| EngineBuild {
        whisper: biorouter_dictation::build_info::WHISPER_COMMIT.to_string(),
        ggml: biorouter_dictation::build_info::GGML_VERSION.to_string(),
        isa: biorouter_dictation::build_info::ISA.to_string(),
    });
    let mut status = EngineStatus {
        compiled,
        state: EngineState::Stopped,
        backend: Some(default_backend().to_string()),
        device: None,
        threads,
        missing_cpu_features: Vec::new(),
        rss_bytes: None,
        build,
        cpu_sentences: Vec::new(),
    };
    if !compiled {
        status.state = EngineState::NoEngine;
        status.backend = None;
        return status;
    }
    if report.class == HardwareClass::No {
        status.state = EngineState::UnsupportedCpu;
        status.backend = None;
        status.missing_cpu_features = report.missing.iter().map(|s| s.to_string()).collect();
        status.cpu_sentences = cpu_sentences(report);
        return status;
    }
    let Some(supervisor) = supervisor else {
        return status;
    };
    status.state = match supervisor.state {
        WorkerState::Stopped => EngineState::Stopped,
        WorkerState::Starting => EngineState::Starting,
        WorkerState::Ready => EngineState::Ready,
        WorkerState::Error => EngineState::Error,
        WorkerState::Unsupported => EngineState::UnsupportedCpu,
    };
    if status.state == EngineState::UnsupportedCpu {
        status.backend = None;
        status.missing_cpu_features = supervisor.missing_cpu_features.clone();
        let ids: Vec<&str> = supervisor
            .missing_cpu_features
            .iter()
            .map(String::as_str)
            .collect();
        status.cpu_sentences = vec![cpu::MSG_CPU.replace("{missing}", &cpu::display_list(&ids))];
        return status;
    }
    if let Some(backend) = &supervisor.backend {
        status.backend = Some(backend.clone());
    }
    status.device = supervisor.device.clone();
    status.rss_bytes = supervisor.rss_bytes;
    if let Some(threads) = supervisor.threads {
        status.threads = threads;
    }
    status
}

/// The backend this build tries first.
pub fn default_backend() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "metal"
    } else {
        "cpu"
    }
}

/// Whether a load asks for the GPU by default: Metal on Apple Silicon only.
pub fn default_use_gpu() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// `min(8, available_parallelism)`.
pub fn base_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8) as u32
}

/// DD-32's halving beside a serving local model.
pub fn threads_rule(base: u32, llama_serving: bool) -> u32 {
    if llama_serving {
        (base / 2).max(1)
    } else {
        base.max(1)
    }
}

fn supervisor_config() -> SupervisorConfig {
    SupervisorConfig {
        idle_unload: Duration::from_secs(idle_unload_secs(
            Config::global()
                .get_param::<serde_json::Value>(IDLE_UNLOAD_KEY)
                .ok(),
        )),
        ..SupervisorConfig::default()
    }
}

/// A positive whole number of seconds, written as a number or as text; 300 otherwise.
fn idle_unload_secs(value: Option<serde_json::Value>) -> u64 {
    let parsed = match value {
        Some(serde_json::Value::Number(n)) => n.as_u64(),
        Some(serde_json::Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    parsed
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_IDLE_UNLOAD_SECS)
}

/// `BIOROUTER_DICTATION_LANGUAGE` from `config.yaml` (or the environment), `en` when
/// unset.
pub fn configured_language() -> String {
    language_setting(Config::global().get_param::<String>(LANGUAGE_KEY).ok())
}

/// A language as written in the configuration, in the form the engine takes: trimmed and
/// lower case, as `--language` is, so `EN` works wherever `en` does; `en` when empty.
pub fn language_setting(written: Option<String>) -> String {
    written
        .map(|language| language.trim().to_ascii_lowercase())
        .filter(|language| !language.is_empty())
        .unwrap_or_else(|| "en".to_string())
}

/// Where this account's speech models go (DD-19).
pub fn models_dir() -> PathBuf {
    paths::user_models_dir()
}

/// `bench`'s records (DD-22).
pub fn bench_path() -> PathBuf {
    Paths::in_local_data_dir("dictation/bench.json")
}

/// `BIOROUTER_DICTATION_MODEL` from `config.yaml`.
pub fn configured_model() -> Option<String> {
    Config::global()
        .get_param::<String>(MODEL_KEY)
        .ok()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}

/// The bench record that still describes this host for `id`: same version, backend and
/// thread count (DD-22).
pub fn current_bench(id: &str) -> Option<BenchRecord> {
    let record = hardware::read_bench(&bench_path()).remove(id)?;
    record
        .applies(env!("CARGO_PKG_VERSION"), default_backend(), base_threads())
        .then_some(record)
}

/// The speech model dictation uses: the configured one when its files are verified and
/// policy allows it (a retired id stands for its replacement), else the class default,
/// else the first usable model in catalog order. Policy is asked before a model is looked
/// up, so a model the organization leaves out is never hashed, and the search stops at
/// the first usable model.
pub fn active_model(
    installed: &mut dyn Installed,
    policy: &DictationPolicy,
    configured: Option<&str>,
    class: cpu::HardwareClass,
) -> Option<&'static SttCatalogEntry> {
    let mut usable = |entry: &'static SttCatalogEntry| {
        policy.model(entry.id).allowed && installed.usable(&CATALOG, entry)
    };
    if let Some((entry, _)) = configured.and_then(|id| CATALOG.resolve(id)) {
        if usable(entry) {
            return Some(entry);
        }
    }
    if let Some(entry) = CATALOG.default_for(class) {
        if usable(entry) {
            return Some(entry);
        }
    }
    CATALOG.speech_models().find(|e| usable(e))
}

/// MSG-LANGUAGE-EN for a `.en` model asked for another language.
pub fn check_language(entry: &SttCatalogEntry, language: &str) -> Result<(), DictationError> {
    if entry.english_only() && !language.eq_ignore_ascii_case("en") {
        return Err(DictationError::EnglishOnly {
            model: entry.id.to_string(),
        });
    }
    Ok(())
}

/// The language must be `auto` or a code whisper.cpp knows, so a typing mistake is
/// refused with a sentence instead of failing inside the worker.
pub fn check_language_code(language: &str) -> Result<(), DictationError> {
    if biorouter_dictation::protocol::is_known_language(language) {
        Ok(())
    } else {
        Err(DictationError::LanguageUnsupported {
            language: language.to_string(),
        })
    }
}

/// Environment only, and read only by a debug build: a path to one of the vendored
/// `for-tests` models, which `resolve_for` loads in place of a catalog model when no id
/// is named, with the `for-tests` voice detector beside it. Both files must hash to their
/// pins. The fixture has no tensors and decodes to nothing [M], so it lets the CLI and
/// route tests run the whole path offline; a release build ignores the variable, so it
/// adds no way to load an arbitrary file (P-11).
pub const TEST_MODEL_ENV: &str = "BIOROUTER_DICTATION_TEST_MODEL";

fn test_model() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(TEST_MODEL_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The load spec for a `for-tests` fixture at `path`, on the CPU.
fn test_model_spec(path: &Path, language: &str) -> Result<LoadSpec, DictationError> {
    use biorouter_dictation::build_info::{fixtures, sha256_file};
    let refused = || {
        DictationError::Internal(format!(
            "{TEST_MODEL_ENV} must name a pinned for-tests model with the for-tests detector beside it"
        ))
    };
    let sha256 = sha256_file(path).map_err(|_| refused())?;
    if !fixtures::MODEL_PINS.contains(&sha256.as_str()) {
        return Err(refused());
    }
    let detector_name = Path::new(fixtures::SILERO)
        .file_name()
        .ok_or_else(refused)?;
    let detector = path.with_file_name(detector_name);
    if sha256_file(&detector).map_err(|_| refused())? != fixtures::SILERO_SHA256 {
        return Err(refused());
    }
    check_language_code(language)?;
    if sha256 == fixtures::BASE_EN_SHA256 && !language.eq_ignore_ascii_case("en") {
        return Err(DictationError::EnglishOnly {
            model: fixture_id(path),
        });
    }
    Ok(LoadSpec {
        model_size: std::fs::metadata(path).map_err(|_| refused())?.len(),
        model_path: path.to_path_buf(),
        model_sha256: sha256,
        vad_path: detector,
        use_gpu: false,
        n_threads: 0,
    })
}

fn fixture_id(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".bin").to_string())
        .unwrap_or_default()
}

/// The id a transcript names for the model `spec` loads: its catalog id, or for a
/// `for-tests` fixture its file name.
pub fn model_id(spec: &LoadSpec) -> String {
    CATALOG
        .by_sha256(&spec.model_sha256)
        .map(|entry| entry.id.to_string())
        .unwrap_or_else(|| fixture_id(&spec.model_path))
}

/// The load spec for `id` (or the active model) from what is installed: a
/// [`paths::Lookup`] for a load, a [`paths::Inventory`] in tests.
pub fn spec_for(
    installed: &mut dyn Installed,
    policy: &DictationPolicy,
    id: Option<&str>,
    configured: Option<&str>,
    language: &str,
    class: cpu::HardwareClass,
) -> Result<LoadSpec, DictationError> {
    if !policy.enabled.is_allowed() {
        return Err(DictationError::Policy(policy.enabled.allowance()));
    }
    let entry = match id {
        Some(id) => {
            let (entry, _) = CATALOG
                .resolve(id)
                .ok_or_else(|| DictationError::UnknownModel(id.to_string()))?;
            if !entry.is_speech() {
                return Err(DictationError::UnknownModel(id.to_string()));
            }
            entry
        }
        None => active_model(installed, policy, configured, class).ok_or_else(|| {
            DictationError::NoModel {
                host: host(),
                account: account(),
            }
        })?,
    };
    let allowance = policy.model(entry.id);
    if !allowance.allowed {
        return Err(DictationError::Policy(allowance));
    }
    check_language_code(language)?;
    check_language(entry, language)?;
    let model = installed
        .found(entry)
        .ok_or_else(|| DictationError::NotInstalled {
            id: entry.id.to_string(),
        })?;
    let vad_id = entry.requires.first().copied().unwrap_or_default();
    let vad = CATALOG
        .entry(vad_id)
        .and_then(|vad| installed.found(vad))
        .ok_or_else(|| DictationError::DetectorNotInstalled {
            model: entry.id.to_string(),
            detector: vad_id.to_string(),
        })?;
    Ok(LoadSpec {
        model_path: model.path.clone(),
        model_sha256: model.sha256.clone(),
        model_size: std::fs::metadata(&model.path).map_or(entry.size_bytes, |m| m.len()),
        vad_path: vad.path.clone(),
        use_gpu: default_use_gpu(),
        n_threads: 0,
    })
}

/// The account the models belong to.
pub fn account() -> String {
    whoami::username()
}

/// This computer's name, `unknown` when it cannot be read.
pub fn host() -> String {
    whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string())
}

/// The binary a worker runs from: `BIOROUTER_DICTATION_WORKER` when set (absolute
/// only), else on Linux `/proc/self/exe`, so a package upgrade during a session still
/// starts the build that is running, else this executable.
pub fn worker_program() -> Result<PathBuf, String> {
    worker_program_from(std::env::var_os(WORKER_ENV), std::env::current_exe)
}

fn worker_program_from(
    configured: Option<OsString>,
    current_exe: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = configured.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Err(format!("{WORKER_ENV} must be an absolute path"))
        };
    }
    if cfg!(target_os = "linux") {
        return Ok(PathBuf::from("/proc/self/exe"));
    }
    current_exe().map_err(|_| "the running executable could not be found".to_string())
}

/// The worker's command (DD-4, engine-and-models.md "Environment"): the engine role of
/// `program`, with every `GGML_*` and `WHISPER_*` variable removed (`GGML_VK_*`
/// included), no backtrace fork on abort, Rosetta told to advertise AVX, and the
/// daemon's own credentials stripped last.
///
/// On Unix the worker leads a process group of its own, so the Ctrl-C a terminal sends
/// to its foreground group (an interactive `biorouter` catches it to cancel a turn) does
/// not kill a warm worker. A parent that dies still ends it: its stdin reaches end of
/// file, `kill_on_drop` holds, and every kill the supervisor sends names the pid. On
/// Windows the worker has no console (`CREATE_NO_WINDOW`), so Ctrl-C cannot reach it.
pub fn worker_command(program: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command
        .arg(WORKER_MARKER)
        .arg(ENGINE_ROLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if key.starts_with("GGML_") || key.starts_with("WHISPER_") {
            command.env_remove(key.as_ref());
        }
    }
    command.env("GGML_NO_BACKTRACE", "1");
    if cpu::rosetta_translated() {
        command.env("ROSETTA_ADVERTISE_AVX", "1");
    }
    // Last: the strip and `env` write the same map.
    crate::subprocess::prepare_agent_child_command(&mut command);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::paths::Inventory;
    use biorouter_dictation::cpu::{classify_for, CpuFeatures, Platform};
    use biorouter_dictation::supervisor::QueueStatus;

    fn envs(command: &tokio::process::Command) -> Vec<(String, Option<String>)> {
        command
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect()
    }

    /// The worker's environment: no library tuning variable reaches it, ggml never
    /// forks a debugger, and the daemon's secret is gone.
    #[test]
    fn the_worker_command_cleans_its_environment() {
        let _env = env_lock::lock_env([
            ("GGML_VK_VISIBLE_DEVICES", Some("0")),
            ("GGML_METAL_PATH_RESOURCES", Some("/tmp/evil")),
            ("WHISPER_COREML", Some("1")),
            ("BIOROUTER_SERVER__SECRET_KEY", Some("daemon-private")),
            ("BIOROUTER_PORT", Some("4931")),
        ]);
        let command = worker_command(Path::new("/opt/biorouter/biorouterd"));
        let envs = envs(&command);
        for key in [
            "GGML_VK_VISIBLE_DEVICES",
            "GGML_METAL_PATH_RESOURCES",
            "WHISPER_COREML",
            "BIOROUTER_SERVER__SECRET_KEY",
        ] {
            assert!(
                envs.contains(&(key.to_string(), None)),
                "{key} must not reach the worker: {envs:?}"
            );
        }
        assert!(envs.contains(&("GGML_NO_BACKTRACE".to_string(), Some("1".to_string()))));
        assert!(
            !envs.iter().any(|(k, _)| k == "BIOROUTER_PORT"),
            "nothing else is touched"
        );
        assert_eq!(
            envs.iter()
                .any(|(k, v)| k == "ROSETTA_ADVERTISE_AVX" && v.is_some()),
            cpu::rosetta_translated()
        );
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, [WORKER_MARKER, "engine"]);
        assert_eq!(command.as_std().get_program(), "/opt/biorouter/biorouterd");
    }

    #[test]
    fn the_worker_program_rules() {
        let absolute = if cfg!(windows) {
            "C:\\Biorouter\\biorouterd.exe"
        } else {
            "/opt/biorouter/biorouterd"
        };
        let no_exe = || Err(std::io::Error::other("unreachable"));
        assert_eq!(
            worker_program_from(Some(absolute.into()), no_exe),
            Ok(PathBuf::from(absolute))
        );
        assert_eq!(
            worker_program_from(Some("bin/biorouterd".into()), no_exe),
            Err("BIOROUTER_DICTATION_WORKER must be an absolute path".to_string())
        );
        let current = || Ok(PathBuf::from("/here/biorouter"));
        let expected = if cfg!(target_os = "linux") {
            PathBuf::from("/proc/self/exe")
        } else {
            PathBuf::from("/here/biorouter")
        };
        assert_eq!(worker_program_from(None, current), Ok(expected.clone()));
        assert_eq!(worker_program_from(Some("".into()), current), Ok(expected));
    }

    #[test]
    fn the_threads_rule() {
        assert_eq!(threads_rule(8, false), 8);
        assert_eq!(threads_rule(8, true), 4);
        assert_eq!(threads_rule(3, true), 1);
        assert_eq!(threads_rule(1, true), 1);
        assert_eq!(threads_rule(0, false), 1);
        assert!((1..=8).contains(&base_threads()));
    }

    #[test]
    fn the_idle_unload_setting() {
        use serde_json::json;
        assert_eq!(idle_unload_secs(None), 300);
        assert_eq!(idle_unload_secs(Some(json!(45))), 45);
        assert_eq!(idle_unload_secs(Some(json!("90"))), 90);
        assert_eq!(idle_unload_secs(Some(json!(0))), 300);
        assert_eq!(idle_unload_secs(Some(json!(-5))), 300);
        assert_eq!(idle_unload_secs(Some(json!("soon"))), 300);
        assert_eq!(idle_unload_secs(Some(json!(1.5))), 300);
    }

    fn supported() -> CpuReport {
        classify_for(
            Platform::X86,
            &CpuFeatures {
                avx2: true,
                fma: true,
                f16c: true,
                bmi2: true,
                dotprod: false,
            },
            false,
            None,
        )
    }

    #[test]
    fn a_build_without_an_engine_says_so_and_refuses() {
        let status = status_from(false, &supported(), None, 8);
        assert_eq!(status.state, EngineState::NoEngine);
        assert!(!status.compiled);
        assert_eq!(status.backend, None);
        assert_eq!(status.build, None);
        assert!(matches!(
            gate(false, &supported()),
            Err(DictationError::NotCompiled)
        ));
        assert_eq!(
            DictationError::NotCompiled.to_string(),
            "This build of Biorouter has no local dictation engine."
        );
        let json = serde_json::to_value(&status).expect("JSON");
        assert_eq!(json["state"], "no_engine");
        assert!(json.get("cpu_sentences").is_none());
    }

    #[test]
    fn a_refused_processor_spawns_nothing_and_gets_its_sentence() {
        let mut refused = classify_for(Platform::X86, &CpuFeatures::default(), false, None);
        refused.hypervisor = true;
        let status = status_from(true, &refused, None, 8);
        assert_eq!(status.state, EngineState::UnsupportedCpu);
        assert_eq!(status.backend, None);
        assert_eq!(status.missing_cpu_features, ["avx2", "fma", "f16c", "bmi2"]);
        assert_eq!(status.cpu_sentences.len(), 2);
        assert!(status.cpu_sentences[0].starts_with(
            "This processor does not support the instructions local dictation needs (AVX2, FMA, F16C and BMI2)."
        ));
        assert_eq!(status.cpu_sentences[1], cpu::MSG_CPU_VM);
        assert!(status.build.is_some());
        let Err(DictationError::UnsupportedCpu { sentence }) = gate(true, &refused) else {
            panic!("the gate must refuse")
        };
        assert!(sentence.ends_with(cpu::MSG_CPU_VM));
    }

    #[test]
    fn the_supervisor_state_maps_to_the_status() {
        let stopped = status_from(true, &supported(), None, 6);
        assert_eq!(stopped.state, EngineState::Stopped);
        assert_eq!(stopped.backend.as_deref(), Some(default_backend()));
        assert_eq!(stopped.threads, 6);

        let ready = SupervisorStatus {
            state: WorkerState::Ready,
            backend: Some("cpu".into()),
            threads: Some(4),
            rss_bytes: Some(700_000_000),
            ..SupervisorStatus::default()
        };
        let status = status_from(true, &supported(), Some(&ready), 8);
        assert_eq!(status.state, EngineState::Ready);
        assert_eq!(status.backend.as_deref(), Some("cpu"));
        assert_eq!(status.threads, 4);
        assert_eq!(status.rss_bytes, Some(700_000_000));

        for (state, expected) in [
            (WorkerState::Starting, EngineState::Starting),
            (WorkerState::Error, EngineState::Error),
            (WorkerState::Stopped, EngineState::Stopped),
        ] {
            let snapshot = SupervisorStatus {
                state,
                queue: QueueStatus::default(),
                ..SupervisorStatus::default()
            };
            assert_eq!(
                status_from(true, &supported(), Some(&snapshot), 8).state,
                expected
            );
        }
        let unsupported = SupervisorStatus {
            state: WorkerState::Unsupported,
            missing_cpu_features: vec!["avx2".into()],
            ..SupervisorStatus::default()
        };
        let status = status_from(true, &supported(), Some(&unsupported), 8);
        assert_eq!(status.state, EngineState::UnsupportedCpu);
        assert_eq!(status.missing_cpu_features, ["avx2"]);
        assert_eq!(status.backend, None);
    }

    /// The process's own status, on whatever this test binary was built with.
    #[test]
    fn the_service_status_never_spawns() {
        let status = service().status();
        assert_eq!(status.compiled, biorouter_dictation::ENGINE_COMPILED);
        assert_ne!(status.state, EngineState::Starting);
        assert_ne!(status.state, EngineState::Ready);
    }

    #[test]
    fn supervisor_errors_keep_their_meaning() {
        let error: DictationError = SupervisorError::UnsupportedCpu {
            missing: vec!["avx2".into(), "fma".into()],
        }
        .into();
        assert_eq!(
            error.to_string(),
            "This processor does not support the instructions local dictation needs (AVX2 and FMA). Your system's own dictation still works, without your Biorouter dictionary."
        );
        let error: DictationError = SupervisorError::Protocol(
            "worker /x/biorouterd speaks protocol 2; this build speaks 1".into(),
        )
        .into();
        assert!(error.to_string().contains("/x/biorouterd"));
        assert!(matches!(
            DictationError::from(SupervisorError::Cancelled),
            DictationError::Cancelled
        ));
    }

    /// MSG-LANGUAGE-EN, and the model refusals of PR-11.
    #[test]
    fn the_model_sentences() {
        let cases = [
            (
                DictationError::EnglishOnly {
                    model: "whisper-small.en-q8_0".into(),
                },
                "whisper-small.en-q8_0 understands only English. Choose a multilingual model for other languages, or set the language to English.",
            ),
            (
                DictationError::UnknownModel("whisper-tiny".into()),
                "There is no speech model named whisper-tiny. Run `biorouter dictation list` to see the models.",
            ),
            (
                DictationError::NotInstalled {
                    id: "whisper-large-v3-turbo-q5_0".into(),
                },
                "whisper-large-v3-turbo-q5_0 is not installed on this computer. Run `biorouter dictation install whisper-large-v3-turbo-q5_0`.",
            ),
            // The repair names the speech model, because `install silero-vad-v6.2.0` is
            // refused (MSG-DETECTOR-ONLY) and `install <model>` fetches only the detector.
            (
                DictationError::DetectorNotInstalled {
                    model: "whisper-small.en-q8_0".into(),
                    detector: "silero-vad-v6.2.0".into(),
                },
                "whisper-small.en-q8_0 needs the voice detector silero-vad-v6.2.0, which is not installed on this computer. Run `biorouter dictation install whisper-small.en-q8_0`.",
            ),
            (
                DictationError::NoModel {
                    host: "lab-mac".into(),
                    account: "ana".into(),
                },
                "Dictation needs a speech model on this computer (lab-mac, account ana). Run `biorouter dictation install`.",
            ),
        ];
        for (error, sentence) in cases {
            assert_eq!(error.to_string(), sentence);
        }
    }

    fn found(id: &str) -> super::paths::Found {
        let entry = CATALOG.entry(id).unwrap();
        super::paths::Found {
            entry,
            location: super::paths::Location::User,
            path: PathBuf::from("/m").join(id).join(entry.file_name),
            sha256: entry.sha256.to_string(),
            superseded: false,
        }
    }

    fn inventory(ids: &[&str]) -> Inventory {
        Inventory {
            found: ids.iter().map(|id| found(id)).collect(),
            strays: Vec::new(),
        }
    }

    /// Which model a load uses, and every refusal before it starts (DD-15, DD-17, DD-90).
    #[test]
    fn the_load_spec_follows_the_installed_files_policy_and_language() {
        use biorouter_dictation::catalog::{SMALL_EN_ID, TURBO_ID, VAD_ID};
        use cpu::HardwareClass::{As, Cpu};
        let open = DictationPolicy::unmanaged();
        let spec = |inv: &Inventory, policy: &DictationPolicy, id, configured, language| {
            spec_for(&mut inv.clone(), policy, id, configured, language, As)
        };

        let none = inventory(&[]);
        assert!(matches!(
            spec(&none, &open, None, None, "en"),
            Err(DictationError::NoModel { .. })
        ));
        // A model without its detector cannot load.
        let no_vad = inventory(&[SMALL_EN_ID]);
        assert!(matches!(
            spec(&no_vad, &open, Some(SMALL_EN_ID), None, "en"),
            Err(DictationError::DetectorNotInstalled { model, detector })
                if model == SMALL_EN_ID && detector == VAD_ID
        ));
        assert!(matches!(
            spec(&no_vad, &open, None, None, "en"),
            Err(DictationError::NoModel { .. })
        ));

        let small = inventory(&[VAD_ID, SMALL_EN_ID]);
        // On Apple Silicon the class default (turbo) is missing, so small.en is used,
        // even when turbo is the configured model.
        let chosen = spec(&small, &open, None, Some(TURBO_ID), "en").unwrap();
        assert!(chosen.model_path.ends_with("ggml-small.en-q8_0.bin"));
        assert!(chosen.vad_path.ends_with("ggml-silero-v6.2.0.bin"));
        assert_eq!(
            chosen.model_sha256,
            CATALOG.entry(SMALL_EN_ID).unwrap().sha256
        );
        assert_eq!(chosen.n_threads, 0);
        assert_eq!(chosen.use_gpu, default_use_gpu());
        assert!(matches!(
            spec(&small, &open, Some(TURBO_ID), None, "en"),
            Err(DictationError::NotInstalled { id }) if id == TURBO_ID
        ));
        assert!(matches!(
            spec(&small, &open, Some("whisper-tiny"), None, "en"),
            Err(DictationError::UnknownModel(_))
        ));
        assert!(matches!(
            spec(&small, &open, Some(VAD_ID), None, "en"),
            Err(DictationError::UnknownModel(_))
        ));
        assert!(matches!(
            spec(&small, &open, None, None, "fr"),
            Err(DictationError::EnglishOnly { .. })
        ));

        let both = inventory(&[VAD_ID, SMALL_EN_ID, TURBO_ID]);
        let class_default =
            |class| active_model(&mut both.clone(), &open, None, class).map(|e| e.id);
        assert_eq!(class_default(As), Some(TURBO_ID));
        assert_eq!(class_default(Cpu), Some(SMALL_EN_ID));
        assert_eq!(
            active_model(&mut both.clone(), &open, Some(SMALL_EN_ID), As).map(|e| e.id),
            Some(SMALL_EN_ID)
        );
        // Turbo is multilingual, so another language is fine, and so is `auto`.
        assert!(spec(&both, &open, Some(TURBO_ID), None, "fr").is_ok());
        assert!(spec(&both, &open, Some(TURBO_ID), None, "auto").is_ok());
        // A code the engine does not know is refused before the worker sees it (PR-12).
        for language in ["xx", "EN", "english", "en-US", ""] {
            assert!(
                matches!(
                    spec(&both, &open, Some(TURBO_ID), None, language),
                    Err(DictationError::LanguageUnsupported { language: l }) if l == language
                ),
                "{language:?}"
            );
        }
        // `config.yaml` is read as `--language` is: `EN` there is `en`, which small.en
        // takes; before, it was refused for every model.
        let written = language_setting(Some(" EN ".to_string()));
        assert_eq!(written, "en");
        assert!(spec_for(&mut small.clone(), &open, None, None, &written, As).is_ok());
        assert_eq!(language_setting(Some("Auto".into())), "auto");
        assert_eq!(language_setting(Some("  ".into())), "en");
        assert_eq!(language_setting(None), "en");

        let only_turbo = DictationPolicy {
            managed: true,
            allowed_models: Some(vec![TURBO_ID.to_string()]),
            ..DictationPolicy::unmanaged()
        };
        assert!(matches!(
            spec(&both, &only_turbo, Some(SMALL_EN_ID), None, "en"),
            Err(DictationError::Policy(_))
        ));
        assert!(matches!(
            spec(&small, &only_turbo, None, None, "en"),
            Err(DictationError::NoModel { .. })
        ));
        assert!(matches!(
            spec(&both, &DictationPolicy::off(), None, None, "en"),
            Err(DictationError::Policy(_))
        ));
    }

    /// The entries a lookup was asked for, in order: the copies a load would hash. A
    /// repeat is not recorded, because `paths::Lookup` answers it from what it found.
    struct Recorder {
        inventory: Inventory,
        asked: Vec<&'static str>,
    }

    impl Installed for Recorder {
        fn found(&mut self, entry: &'static SttCatalogEntry) -> Option<paths::Found> {
            if !self.asked.contains(&entry.id) {
                self.asked.push(entry.id);
            }
            self.inventory.get(entry.id).cloned()
        }
    }

    /// The arming path looks up only what it loads (DD-17): an explicit id reads that
    /// model and its detector, never another model; with no id the search stops at the
    /// first usable model; a model the policy leaves out, or one a refusal stops, is never
    /// looked up.
    #[test]
    fn a_load_looks_up_only_what_it_loads() {
        use biorouter_dictation::catalog::{SMALL_EN_ID, TURBO_ID, VAD_ID};
        use cpu::HardwareClass::As;
        let open = DictationPolicy::unmanaged();
        let asked = |policy: &DictationPolicy, id, configured, language| {
            let mut recorder = Recorder {
                inventory: inventory(&[VAD_ID, SMALL_EN_ID, TURBO_ID]),
                asked: Vec::new(),
            };
            let _ = spec_for(&mut recorder, policy, id, configured, language, As);
            recorder.asked
        };
        assert_eq!(
            asked(&open, Some(SMALL_EN_ID), None, "en"),
            [SMALL_EN_ID, VAD_ID]
        );
        assert_eq!(
            asked(&open, None, Some(SMALL_EN_ID), "en"),
            [SMALL_EN_ID, VAD_ID]
        );
        // The class default on Apple Silicon, then its detector; small.en is not opened.
        assert_eq!(asked(&open, None, None, "en"), [TURBO_ID, VAD_ID]);
        let only_turbo = DictationPolicy {
            managed: true,
            allowed_models: Some(vec![TURBO_ID.to_string()]),
            ..DictationPolicy::unmanaged()
        };
        assert_eq!(
            asked(&only_turbo, None, Some(SMALL_EN_ID), "en"),
            [TURBO_ID, VAD_ID]
        );
        assert!(asked(&only_turbo, Some(SMALL_EN_ID), None, "en").is_empty());
        assert!(asked(&open, Some(SMALL_EN_ID), None, "fr").is_empty());
        assert!(asked(&DictationPolicy::off(), None, None, "en").is_empty());
    }

    /// The sentences of the message catalog's worker diagnostics (MSG-DIAG-*, shown by
    /// `status --verbose`), word for word.
    #[test]
    fn the_error_sentences_are_the_catalogs() {
        use crate::managed::{Allowance, MANAGED_DICTATION_OFF};
        let cases: Vec<(DictationError, &str)> = vec![
            (
                DictationError::WorkerExited { code: Some(134) },
                "The dictation worker stopped with exit code 134.",
            ),
            (
                DictationError::WorkerExited { code: None },
                "The dictation worker was stopped.",
            ),
            (
                DictationError::Deadline { phase: "load" },
                "The dictation worker did not answer in time (load).",
            ),
            (
                DictationError::ModelLoadFailed {
                    reason: "model file size does not match".into(),
                },
                "The speech model could not be loaded: model file size does not match.",
            ),
            (DictationError::BadAudio, "The audio could not be used."),
            (DictationError::Cancelled, "Dictation was cancelled."),
            (
                SupervisorError::Spawn(std::io::Error::from(std::io::ErrorKind::NotFound)).into(),
                "Dictation failed: the worker could not start (entity not found).",
            ),
            (
                DictationError::Policy(Allowance::refused(MANAGED_DICTATION_OFF)),
                MANAGED_DICTATION_OFF,
            ),
            (
                DictationError::Policy(Allowance::granted()),
                MANAGED_DICTATION_OFF,
            ),
            (
                DictationError::LanguageUnsupported {
                    language: "xx".into(),
                },
                "Biorouter does not know the language \"xx\". Use a two letter code such as en or de, or auto to let the model decide.",
            ),
        ];
        for (error, sentence) in cases {
            assert_eq!(error.to_string(), sentence);
        }
    }

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/whisper.cpp/source")
            .join(relative)
    }

    /// `BIOROUTER_DICTATION_TEST_MODEL` loads only a pinned fixture with the pinned
    /// detector beside it, on the CPU, and still refuses a language the model cannot take.
    #[test]
    fn the_test_model_is_a_pinned_fixture() {
        use biorouter_dictation::build_info::fixtures;
        let base = fixture(fixtures::BASE_EN);
        let spec = test_model_spec(&base, "en").expect("the base.en fixture");
        assert_eq!(spec.model_sha256, fixtures::BASE_EN_SHA256);
        assert!(spec.vad_path.ends_with("for-tests-silero-v6.2.0-ggml.bin"));
        assert!(!spec.use_gpu);
        assert_eq!(spec.n_threads, 0);
        assert_eq!(model_id(&spec), "for-tests-ggml-base.en");
        assert!(matches!(
            test_model_spec(&base, "de"),
            Err(DictationError::EnglishOnly { model }) if model == "for-tests-ggml-base.en"
        ));
        assert!(matches!(
            test_model_spec(&base, "klingon"),
            Err(DictationError::LanguageUnsupported { .. })
        ));
        assert!(test_model_spec(&fixture(fixtures::TINY), "de").is_ok());
        // Not a model fixture; a fixture without its detector beside it.
        assert!(matches!(
            test_model_spec(&fixture(fixtures::JFK), "en"),
            Err(DictationError::Internal(_))
        ));
        let dir = tempfile::tempdir().unwrap();
        let alone = dir.path().join("for-tests-ggml-base.en.bin");
        std::fs::copy(&base, &alone).unwrap();
        assert!(matches!(
            test_model_spec(&alone, "en"),
            Err(DictationError::Internal(_))
        ));
        assert!(matches!(
            test_model_spec(&dir.path().join("missing.bin"), "en"),
            Err(DictationError::Internal(_))
        ));
    }
}
