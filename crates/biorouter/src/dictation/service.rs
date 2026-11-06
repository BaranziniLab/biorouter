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

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
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

use crate::config::paths::Paths;
use crate::config::Config;
use crate::managed::Allowance;

/// `config.yaml`: seconds without a request before the worker is stopped (DD-91).
pub const IDLE_UNLOAD_KEY: &str = "BIOROUTER_DICTATION_IDLE_UNLOAD_SECS";
/// `config.yaml`: the language a multilingual model is told to expect.
pub const LANGUAGE_KEY: &str = "BIOROUTER_DICTATION_LANGUAGE";
/// Environment only: an absolute path to the binary the worker runs from.
pub const WORKER_ENV: &str = "BIOROUTER_DICTATION_WORKER";
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
}

/// The process's one service.
pub fn service() -> &'static DictationService {
    static SERVICE: OnceLock<DictationService> = OnceLock::new();
    SERVICE.get_or_init(|| DictationService {
        supervisor: Mutex::new(None),
        threads: Mutex::new(None),
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
        let supervisor = Arc::new(Supervisor::start(Arc::new(spawner), supervisor_config()));
        *slot = Some(Arc::clone(&supervisor));
        Ok(supervisor)
    }
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

/// `BIOROUTER_DICTATION_LANGUAGE` from `config.yaml`, `en` when unset.
pub fn configured_language() -> String {
    Config::global()
        .get_param::<String>(LANGUAGE_KEY)
        .ok()
        .map(|language| language.trim().to_string())
        .filter(|language| !language.is_empty())
        .unwrap_or_else(|| "en".to_string())
}

/// Where this account's speech models go (DD-19).
pub fn models_dir() -> PathBuf {
    Paths::in_local_data_dir("dictation/models")
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
        ];
        for (error, sentence) in cases {
            assert_eq!(error.to_string(), sentence);
        }
    }
}
