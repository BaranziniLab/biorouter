//! The supervisor: spawns, frames, times and restarts the worker (DD-6, DD-7).
//!
//! One actor task per supervisor owns the worker process and every queue. Callers send
//! it commands and await one-shot answers, so nothing a caller does ever blocks on the
//! pipe: frames are written by a writer task, read by a reader task, and the worker's
//! stderr is kept by a third. The command that runs the worker is a value the core
//! crate hands in ([`WorkerSpawner`], DD-4), and the tests hand in an in-memory worker
//! on tokio's paused clock instead.
//!
//! What the supervisor does for latency:
//!
//! * [`Supervisor::spawn_ahead`] starts the process before the model is chosen, and
//!   [`Supervisor::prepare`] loads the model while the person is still speaking (L-2);
//! * the worker stays loaded until [`SupervisorConfig::idle_unload`] passes with
//!   nothing to do (L-1), so a warm request pays for the decode alone;
//! * jobs share the caller's buffer ([`PcmSlice`]) and go to the pipe without a copy;
//! * segments of several streams are served round robin, one at a time, so a long
//!   dictation never starves a short one (DD-6), and `vad_segments` jobs go first
//!   because they gate the segmentation of a dictation still in progress.
//!
//! Lifecycle (DD-7): `hello` within 60 s of the spawn, `loaded` within 180 s of the
//! `load`, a job within max(30 s, 10 x audio seconds) on the GPU and max(120 s,
//! 20 x audio seconds) on the CPU. Only a transcription counts as a GPU job:
//! `vad_segments` runs on the separate detector, which the shim always opens on the CPU.
//! A GPU job whose worker dies or misses its deadline is retried once in a fresh worker
//! on the CPU, which then stays on the CPU until the next idle unload. A GPU `load` that
//! fails the same way, or answers `model_load_failed`, is sent once more on the CPU,
//! because the Metal warm up runs inside `load`; when that load fails too, the GPU was
//! not the cause, and the next load asks for it again. Exit code 20, or a `hello` whose
//! CPU lacks a feature the engine needs, marks the supervisor unsupported for the life
//! of the process.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::audio::{self, PcmSlice, Span};
use crate::cpu;
use crate::protocol::{
    self, BiasRef, DecodeOptions, ErrorCode, FrameError, FromWorker, Hello, Load, Loaded, ToWorker,
    Transcribe, TranscribeResult, VadOptions, VadSegments, PROMPT_BUDGET_TOKENS, PROTOCOL_VERSION,
};
use crate::worker::exit;

/// Starts worker processes. The real one runs a command the core crate builds; tests
/// run an in-memory worker.
pub trait WorkerSpawner: Send + Sync + 'static {
    fn spawn(&self) -> io::Result<WorkerProcess>;
    /// The program, named in a protocol version error.
    fn program(&self) -> String;
}

/// A started worker: its three pipes and the handle that kills and reaps it.
pub struct WorkerProcess {
    pub stdin: Pin<Box<dyn AsyncWrite + Send>>,
    pub stdout: Pin<Box<dyn AsyncRead + Send>>,
    pub stderr: Pin<Box<dyn AsyncRead + Send>>,
    pub child: Box<dyn WorkerChild>,
}

pub trait WorkerChild: Send {
    fn pid(&self) -> Option<u32>;
    fn start_kill(&mut self) -> io::Result<()>;
    /// Waits for the exit and reaps it: the exit code, or `None` after a signal.
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<Option<i32>>> + Send + '_>>;
}

/// Runs a [`tokio::process::Command`] built fresh for every spawn.
pub struct CommandSpawner {
    make: Arc<dyn Fn() -> tokio::process::Command + Send + Sync>,
    program: String,
}

impl CommandSpawner {
    pub fn new(
        program: impl Into<String>,
        make: impl Fn() -> tokio::process::Command + Send + Sync + 'static,
    ) -> CommandSpawner {
        CommandSpawner {
            make: Arc::new(make),
            program: program.into(),
        }
    }
}

impl WorkerSpawner for CommandSpawner {
    fn spawn(&self) -> io::Result<WorkerProcess> {
        let mut command = (self.make)();
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let missing = || io::Error::other("the worker's pipes were not opened");
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = child.stdout.take().ok_or_else(missing)?;
        let stderr = child.stderr.take().ok_or_else(missing)?;
        Ok(WorkerProcess {
            stdin: Box::pin(stdin),
            stdout: Box::pin(stdout),
            stderr: Box::pin(stderr),
            child: Box::new(ProcessChild(child)),
        })
    }

    fn program(&self) -> String {
        self.program.clone()
    }
}

struct ProcessChild(tokio::process::Child);

impl WorkerChild for ProcessChild {
    fn pid(&self) -> Option<u32> {
        self.0.id()
    }

    fn start_kill(&mut self) -> io::Result<()> {
        self.0.start_kill()
    }

    fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<Option<i32>>> + Send + '_>> {
        Box::pin(async move { self.0.wait().await.map(|status| status.code()) })
    }
}

/// DD-7's deadlines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deadlines {
    /// Spawn to `hello`.
    pub hello: Duration,
    /// `load` to `loaded`.
    pub load: Duration,
    pub gpu_floor: Duration,
    pub gpu_per_audio_s: u32,
    pub cpu_floor: Duration,
    pub cpu_per_audio_s: u32,
    /// `shutdown` to the exit, before the worker is killed.
    pub shutdown_grace: Duration,
}

impl Default for Deadlines {
    fn default() -> Deadlines {
        Deadlines {
            hello: Duration::from_secs(60),
            load: Duration::from_secs(180),
            gpu_floor: Duration::from_secs(30),
            gpu_per_audio_s: 10,
            cpu_floor: Duration::from_secs(120),
            cpu_per_audio_s: 20,
            shutdown_grace: Duration::from_secs(5),
        }
    }
}

impl Deadlines {
    /// The time a job of `audio_secs` may take, measured from its dispatch.
    pub fn job(&self, audio_secs: f64, gpu: bool) -> Duration {
        let (floor, per) = if gpu {
            (self.gpu_floor, self.gpu_per_audio_s)
        } else {
            (self.cpu_floor, self.cpu_per_audio_s)
        };
        floor.max(Duration::from_secs_f64(
            f64::from(per) * audio_secs.max(0.0),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupervisorConfig {
    /// `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS`, default 300 s.
    pub idle_unload: Duration,
    pub deadlines: Deadlines,
    /// How many stderr lines are kept for `status --verbose`.
    pub stderr_lines: usize,
}

impl Default for SupervisorConfig {
    fn default() -> SupervisorConfig {
        SupervisorConfig {
            idle_unload: Duration::from_secs(300),
            deadlines: Deadlines::default(),
            stderr_lines: STDERR_LINES,
        }
    }
}

/// Lines of worker stderr kept, each cut at [`STDERR_LINE_BYTES`] (DD-13, P-12).
pub const STDERR_LINES: usize = 64;
pub const STDERR_LINE_BYTES: usize = 200;
/// How long a worker that closed its stdout gets to exit before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// What to load. The parent verified the file's SHA-256 before building this (DD-17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadSpec {
    pub model_path: PathBuf,
    pub model_sha256: String,
    pub model_size: u64,
    /// The detector file; its name must be ASCII (the integrated detector opens it by
    /// name inside its folder).
    pub vad_path: PathBuf,
    pub use_gpu: bool,
    pub n_threads: u32,
}

impl LoadSpec {
    /// Whether a worker that loaded `self` can serve a job for `other` without a new
    /// `load`. The thread count is a per request value and does not count.
    pub fn same_model(&self, other: &LoadSpec) -> bool {
        self.model_sha256 == other.model_sha256
            && self.model_path == other.model_path
            && self.vad_path == other.vad_path
            && self.use_gpu == other.use_gpu
    }

    fn to_load(&self) -> Result<Load, SupervisorError> {
        let utf8 = |path: &std::path::Path| {
            path.to_str()
                .map(str::to_string)
                .ok_or_else(|| SupervisorError::Internal("a model path is not valid UTF-8".into()))
        };
        let vad_dir = self
            .vad_path
            .parent()
            .ok_or_else(|| SupervisorError::Internal("the voice detector has no folder".into()))?;
        let vad_file = self
            .vad_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SupervisorError::Internal("a model path is not valid UTF-8".into()))?;
        Ok(Load {
            model_path: utf8(&self.model_path)?,
            model_sha256: self.model_sha256.clone(),
            model_size: self.model_size,
            vad_dir: utf8(vad_dir)?,
            vad_file: vad_file.to_string(),
            use_gpu: self.use_gpu,
            n_threads: self.n_threads,
        })
    }
}

/// One dictation stream or file. Its segments are served first in, first out; streams
/// take turns, one segment each.
pub type StreamKey = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

/// A transcription's parameters other than its audio.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscribeParams {
    pub language: String,
    pub decode: DecodeOptions,
    pub vad: VadOptions,
    /// Ordered, best last. Empty until PR-14.
    pub prompt_terms: Vec<String>,
    pub prompt_budget_tokens: u32,
    /// `None` until PR-14.
    pub bias: Option<BiasRef>,
}

impl TranscribeParams {
    /// The DD-47 and DD-48 values, with no prompt and no bias.
    pub fn new(language: impl Into<String>, n_threads: u32) -> TranscribeParams {
        TranscribeParams {
            language: language.into(),
            decode: DecodeOptions::dd47(n_threads),
            vad: VadOptions::dd48(),
            prompt_terms: Vec::new(),
            prompt_budget_tokens: PROMPT_BUDGET_TOKENS,
            bias: None,
        }
    }
}

/// Why a request failed. `Display` is a fixed English phrase.
#[derive(Debug)]
pub enum SupervisorError {
    /// The worker's processor lacks what the engine needs; cached for the process.
    UnsupportedCpu {
        missing: Vec<String>,
    },
    WorkerExited {
        code: Option<i32>,
    },
    Deadline {
        phase: &'static str,
    },
    /// The worker speaks another protocol, or sent a frame that could not be read.
    Protocol(String),
    /// The worker's fixed phrase.
    ModelLoadFailed(String),
    BadAudio,
    Cancelled,
    Internal(String),
    Spawn(io::Error),
    /// The supervisor's task is gone: its runtime ended.
    Stopped,
}

impl SupervisorError {
    /// A copy, for failing every waiter with one error.
    fn duplicate(&self) -> SupervisorError {
        match self {
            SupervisorError::UnsupportedCpu { missing } => SupervisorError::UnsupportedCpu {
                missing: missing.clone(),
            },
            SupervisorError::WorkerExited { code } => SupervisorError::WorkerExited { code: *code },
            SupervisorError::Deadline { phase } => SupervisorError::Deadline { phase },
            SupervisorError::Protocol(message) => SupervisorError::Protocol(message.clone()),
            SupervisorError::ModelLoadFailed(message) => {
                SupervisorError::ModelLoadFailed(message.clone())
            }
            SupervisorError::BadAudio => SupervisorError::BadAudio,
            SupervisorError::Cancelled => SupervisorError::Cancelled,
            SupervisorError::Internal(message) => SupervisorError::Internal(message.clone()),
            SupervisorError::Spawn(e) => {
                SupervisorError::Spawn(io::Error::new(e.kind(), e.to_string()))
            }
            SupervisorError::Stopped => SupervisorError::Stopped,
        }
    }
}

impl std::fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupervisorError::UnsupportedCpu { missing } => write!(
                f,
                "this processor lacks instructions local dictation needs ({})",
                cpu::display_list(&missing.iter().map(String::as_str).collect::<Vec<_>>())
            ),
            SupervisorError::WorkerExited { code: Some(code) } => {
                write!(f, "the dictation worker stopped with exit code {code}")
            }
            SupervisorError::WorkerExited { code: None } => {
                f.write_str("the dictation worker was stopped")
            }
            SupervisorError::Deadline { phase } => {
                write!(f, "the dictation worker did not answer in time ({phase})")
            }
            SupervisorError::Protocol(message) => f.write_str(message),
            SupervisorError::ModelLoadFailed(message) => {
                write!(f, "the speech model could not be loaded: {message}")
            }
            SupervisorError::BadAudio => f.write_str("the audio could not be used"),
            SupervisorError::Cancelled => f.write_str("cancelled"),
            SupervisorError::Internal(message) => write!(f, "dictation failed: {message}"),
            SupervisorError::Spawn(e) => {
                write!(f, "the dictation worker could not start: {}", e.kind())
            }
            SupervisorError::Stopped => f.write_str("the dictation supervisor has stopped"),
        }
    }
}

impl std::error::Error for SupervisorError {}

/// The worker's state as status reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    #[default]
    Stopped,
    /// Spawning or loading.
    Starting,
    /// Loaded, idle or busy.
    Ready,
    /// The last start failed; the next request tries again.
    Error,
    /// The processor cannot run the engine.
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct QueueStatus {
    /// Streams with segments waiting.
    pub streams: usize,
    /// Jobs waiting, not counting the one running.
    pub segments: usize,
}

/// A snapshot of the supervisor. Reading it never spawns.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SupervisorStatus {
    pub state: WorkerState,
    pub busy: bool,
    /// The loaded model's backend, `"metal"` or `"cpu"`.
    pub backend: Option<String>,
    pub device: Option<String>,
    pub threads: Option<u32>,
    pub rss_bytes: Option<u64>,
    pub pid: Option<u32>,
    /// A GPU fault moved this supervisor to the CPU until its next idle unload.
    pub force_cpu: bool,
    pub missing_cpu_features: Vec<String>,
    pub queue: QueueStatus,
    pub hello: Option<Hello>,
    pub last_error: Option<String>,
}

/// `status --verbose`: a worker's `hello`, or why there was none, and its stderr.
#[derive(Debug)]
pub struct ProbeOutcome {
    pub hello: Result<Hello, SupervisorError>,
    /// The kept stderr lines, control characters escaped.
    pub stderr: Vec<String>,
    /// The probe worker's exit code; `None` when the running worker answered.
    pub exit_code: Option<i32>,
}

/// The handle. Cloning is not needed: one per process, behind the core service.
pub struct Supervisor {
    commands: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<SupervisorStatus>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    next_id: AtomicU64,
    spawner: Arc<dyn WorkerSpawner>,
    config: SupervisorConfig,
}

/// Who waits for a model to be ready.
type Waiter = (
    Arc<LoadSpec>,
    oneshot::Sender<Result<Loaded, SupervisorError>>,
);

enum Command {
    Prepare {
        spec: Arc<LoadSpec>,
        reply: oneshot::Sender<Result<Loaded, SupervisorError>>,
    },
    SpawnAhead,
    Submit(Box<Job>),
    Cancel(u64),
    CancelStream(StreamKey),
    Unload(oneshot::Sender<()>),
    KillWorker,
}

struct Job {
    id: u64,
    spec: Arc<LoadSpec>,
    kind: JobKind,
    /// Already retried once on the CPU after a GPU fault.
    retried: bool,
}

enum JobKind {
    Transcribe {
        stream: StreamKey,
        pcm: PcmSlice,
        params: TranscribeParams,
        reply: oneshot::Sender<Result<TranscribeResult, SupervisorError>>,
    },
    Vad {
        pcm: PcmSlice,
        vad: VadOptions,
        reply: oneshot::Sender<Result<Vec<Span>, SupervisorError>>,
    },
}

impl Job {
    fn fail(self, error: SupervisorError) {
        match self.kind {
            JobKind::Transcribe { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            JobKind::Vad { reply, .. } => {
                let _ = reply.send(Err(error));
            }
        }
    }

    fn stream(&self) -> Option<StreamKey> {
        match &self.kind {
            JobKind::Transcribe { stream, .. } => Some(*stream),
            JobKind::Vad { .. } => None,
        }
    }

    fn pcm(&self) -> &PcmSlice {
        match &self.kind {
            JobKind::Transcribe { pcm, .. } | JobKind::Vad { pcm, .. } => pcm,
        }
    }

    fn phase(&self) -> &'static str {
        match &self.kind {
            JobKind::Transcribe { .. } => "transcribe",
            JobKind::Vad { .. } => "vad_segments",
        }
    }
}

impl Supervisor {
    /// Starts the actor on the current tokio runtime. Nothing is spawned until a request
    /// needs a worker.
    pub fn start(spawner: Arc<dyn WorkerSpawner>, config: SupervisorConfig) -> Supervisor {
        Supervisor::start_on(spawner, config, cpu::Platform::host())
    }

    /// [`Supervisor::start`] judging each `hello` for `platform`'s needs, so the tests
    /// reach the refusal of an x86 `hello` on any host.
    fn start_on(
        spawner: Arc<dyn WorkerSpawner>,
        config: SupervisorConfig,
        platform: cpu::Platform,
    ) -> Supervisor {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (status_tx, status) = watch::channel(SupervisorStatus::default());
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let actor = Actor {
            spawner: Arc::clone(&spawner),
            config: config.clone(),
            commands: receiver,
            status: status_tx,
            stderr: Arc::clone(&stderr),
            platform,
            worker: None,
            phase: Phase::Stopped,
            force_cpu: false,
            cpu_load_retry: false,
            unsupported: None,
            last_error: None,
            vad_queue: VecDeque::new(),
            rotation: VecDeque::new(),
            streams: HashMap::new(),
            running: None,
            waiters: Vec::new(),
            unload_waiters: Vec::new(),
            deadline: None,
            idle_since: Instant::now(),
        };
        tokio::spawn(actor.run());
        Supervisor {
            commands,
            status,
            stderr,
            next_id: AtomicU64::new(1),
            spawner,
            config,
        }
    }

    /// The current state. A watch read: never spawns, never waits.
    pub fn status(&self) -> SupervisorStatus {
        self.status.borrow().clone()
    }

    /// Every status change, for a caller that waits for one.
    pub fn subscribe(&self) -> watch::Receiver<SupervisorStatus> {
        self.status.clone()
    }

    /// False once the actor's runtime has ended; the handle is then useless.
    pub fn is_running(&self) -> bool {
        !self.commands.is_closed()
    }

    pub fn config(&self) -> &SupervisorConfig {
        &self.config
    }

    /// Starts the worker process now, before the model is known, so the process start
    /// overlaps whatever the caller does next. Returns at once.
    pub fn spawn_ahead(&self) {
        let _ = self.commands.send(Command::SpawnAhead);
    }

    /// Spawns and loads, and resolves once the model is ready. A caller arming a
    /// microphone does not await this: the load overlaps the speech (L-2).
    pub async fn prepare(&self, spec: Arc<LoadSpec>) -> Result<Loaded, SupervisorError> {
        let (reply, answer) = oneshot::channel();
        if self
            .commands
            .send(Command::Prepare { spec, reply })
            .is_err()
        {
            return Err(SupervisorError::Stopped);
        }
        answer.await.unwrap_or(Err(SupervisorError::Stopped))
    }

    /// Queues a transcription of at most 25 s and returns at once.
    pub fn submit_transcribe(
        &self,
        spec: Arc<LoadSpec>,
        stream: StreamKey,
        pcm: PcmSlice,
        params: TranscribeParams,
    ) -> (
        JobId,
        oneshot::Receiver<Result<TranscribeResult, SupervisorError>>,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        if pcm.len() > protocol::MAX_TRANSCRIBE_SAMPLES {
            let _ = reply.send(Err(SupervisorError::BadAudio));
            return (JobId(id), answer);
        }
        let job = Job {
            id,
            spec,
            kind: JobKind::Transcribe {
                stream,
                pcm,
                params,
                reply,
            },
            retried: false,
        };
        if let Err(mpsc::error::SendError(Command::Submit(job))) =
            self.commands.send(Command::Submit(Box::new(job)))
        {
            job.fail(SupervisorError::Stopped);
        }
        (JobId(id), answer)
    }

    pub async fn transcribe(
        &self,
        spec: Arc<LoadSpec>,
        stream: StreamKey,
        pcm: PcmSlice,
        params: TranscribeParams,
    ) -> Result<TranscribeResult, SupervisorError> {
        let (_, answer) = self.submit_transcribe(spec, stream, pcm, params);
        answer.await.unwrap_or(Err(SupervisorError::Stopped))
    }

    /// Queues a voice detection of at most 600 s and returns at once. These jobs go
    /// before any transcription.
    pub fn submit_vad_segments(
        &self,
        spec: Arc<LoadSpec>,
        pcm: PcmSlice,
        vad: VadOptions,
    ) -> (JobId, oneshot::Receiver<Result<Vec<Span>, SupervisorError>>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        if pcm.len() > protocol::MAX_VAD_SAMPLES {
            let _ = reply.send(Err(SupervisorError::BadAudio));
            return (JobId(id), answer);
        }
        let job = Job {
            id,
            spec,
            kind: JobKind::Vad { pcm, vad, reply },
            retried: false,
        };
        if let Err(mpsc::error::SendError(Command::Submit(job))) =
            self.commands.send(Command::Submit(Box::new(job)))
        {
            job.fail(SupervisorError::Stopped);
        }
        (JobId(id), answer)
    }

    pub async fn vad_segments(
        &self,
        spec: Arc<LoadSpec>,
        pcm: PcmSlice,
        vad: VadOptions,
    ) -> Result<Vec<Span>, SupervisorError> {
        let (_, answer) = self.submit_vad_segments(spec, pcm, vad);
        answer.await.unwrap_or(Err(SupervisorError::Stopped))
    }

    /// A queued job is dropped and answered `Cancelled`; a running one is aborted at the
    /// worker's next encoder pass or decoder step.
    pub fn cancel(&self, job: JobId) {
        let _ = self.commands.send(Command::Cancel(job.0));
    }

    /// Drops a stream's queued segments and aborts its running one.
    pub fn cancel_stream(&self, stream: StreamKey) {
        let _ = self.commands.send(Command::CancelStream(stream));
    }

    /// Cancels everything queued or running and stops the worker; resolves once it has
    /// exited. Runs before a model file is deleted or replaced.
    pub async fn unload(&self) {
        let (reply, done) = oneshot::channel();
        if self.commands.send(Command::Unload(reply)).is_ok() {
            let _ = done.await;
        }
    }

    /// The running worker's `hello`, or one from a worker started only to say it and
    /// stopped again, for `status --verbose`.
    pub async fn probe(&self) -> ProbeOutcome {
        let status = self.status();
        if let Some(hello) = status.hello {
            return ProbeOutcome {
                hello: Ok(hello),
                stderr: self.stderr_tail(),
                exit_code: None,
            };
        }
        if status.state == WorkerState::Unsupported {
            return ProbeOutcome {
                hello: Err(SupervisorError::UnsupportedCpu {
                    missing: status.missing_cpu_features,
                }),
                stderr: self.stderr_tail(),
                exit_code: None,
            };
        }
        probe_once(self.spawner.as_ref(), &self.config).await
    }

    /// The kept stderr lines, control characters escaped.
    pub fn stderr_tail(&self) -> Vec<String> {
        escaped_lines(&self.stderr)
    }

    /// The running worker's process id. For tests.
    #[doc(hidden)]
    pub fn worker_pid(&self) -> Option<u32> {
        self.status.borrow().pid
    }

    /// Kills the running worker as a crash would. For tests.
    #[doc(hidden)]
    pub fn kill_worker(&self) {
        let _ = self.commands.send(Command::KillWorker);
    }
}

async fn probe_once(spawner: &dyn WorkerSpawner, config: &SupervisorConfig) -> ProbeOutcome {
    let ring = Arc::new(Mutex::new(VecDeque::new()));
    let process = match spawner.spawn() {
        Ok(process) => process,
        Err(e) => {
            return ProbeOutcome {
                hello: Err(SupervisorError::Spawn(e)),
                stderr: Vec::new(),
                exit_code: None,
            }
        }
    };
    let WorkerProcess {
        stdin,
        mut stdout,
        stderr,
        mut child,
    } = process;
    let stderr_task = tokio::spawn(stderr_loop(stderr, Arc::clone(&ring), config.stderr_lines));
    let first =
        tokio::time::timeout(config.deadlines.hello, protocol::read_frame(&mut stdout)).await;
    // End of file: the worker exits on its own.
    drop(stdin);
    let code = reap(child.as_mut(), config.deadlines.shutdown_grace).await;
    let _ = tokio::time::timeout(Duration::from_millis(500), stderr_task).await;
    let program = spawner.program();
    let hello = match first {
        Err(_) => Err(SupervisorError::Deadline { phase: "hello" }),
        Ok(Err(FrameError::Closed)) => Err(SupervisorError::WorkerExited { code }),
        Ok(Err(e)) => Err(SupervisorError::Protocol(format!(
            "worker {program} sent a frame that could not be read: {e}"
        ))),
        Ok(Ok(frame)) => match decode(&frame.json, &program) {
            WorkerEvent::Message(FromWorker::Hello(hello)) => Ok(hello),
            WorkerEvent::Violation(message) => Err(SupervisorError::Protocol(message)),
            _ => Err(SupervisorError::Protocol(format!(
                "worker {program} did not start with hello"
            ))),
        },
    };
    ProbeOutcome {
        hello,
        stderr: escaped_lines(&ring),
        exit_code: code,
    }
}

/// Waits up to `grace` for the exit, then kills; reaps either way.
async fn reap(child: &mut dyn WorkerChild, grace: Duration) -> Option<i32> {
    match tokio::time::timeout(grace, child.wait()).await {
        Ok(Ok(code)) => code,
        _ => {
            let _ = child.start_kill();
            child.wait().await.ok().flatten()
        }
    }
}

fn escaped_lines(ring: &Mutex<VecDeque<String>>) -> Vec<String> {
    let lines = ring.lock().map(|r| r.iter().cloned().collect::<Vec<_>>());
    lines
        .unwrap_or_default()
        .iter()
        .map(|line| {
            line.chars()
                .map(|c| {
                    if c.is_control() {
                        c.escape_default().to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect()
        })
        .collect()
}

/// Keeps the last `cap` stderr lines, each cut at [`STDERR_LINE_BYTES`]. Nothing is
/// logged (DD-13, P-12).
async fn stderr_loop(
    mut stderr: Pin<Box<dyn AsyncRead + Send>>,
    ring: Arc<Mutex<VecDeque<String>>>,
    cap: usize,
) {
    let push = |line: &[u8]| {
        let text = String::from_utf8_lossy(line)
            .trim_end_matches('\r')
            .to_string();
        if let Ok(mut ring) = ring.lock() {
            ring.push_back(text);
            while ring.len() > cap {
                ring.pop_front();
            }
        }
    };
    let mut buf = [0u8; 4096];
    let mut line = Vec::with_capacity(STDERR_LINE_BYTES);
    loop {
        let n = match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &byte in buf.get(..n).unwrap_or_default() {
            if byte == b'\n' {
                push(&line);
                line.clear();
            } else if line.len() < STDERR_LINE_BYTES {
                line.push(byte);
            }
        }
    }
    if !line.is_empty() {
        push(&line);
    }
}

enum WorkerEvent {
    Message(FromWorker),
    /// A frame or message that breaks the protocol; the worker is killed.
    Violation(String),
    /// Stdout ended.
    Closed,
}

/// One message from the worker. A `hello` of another protocol version is refused before
/// its fields are read, since they may differ.
fn decode(json: &[u8], program: &str) -> WorkerEvent {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) else {
        return WorkerEvent::Violation(format!(
            "worker {program} sent a message that could not be read"
        ));
    };
    if value.get("type").and_then(serde_json::Value::as_str) == Some("hello") {
        let version = value.get("protocol").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(PROTOCOL_VERSION)) {
            let spoken = version.map_or_else(
                || "an unknown protocol".to_string(),
                |v| format!("protocol {v}"),
            );
            return WorkerEvent::Violation(format!(
                "worker {program} speaks {spoken}; this build speaks {PROTOCOL_VERSION}"
            ));
        }
    }
    match serde_json::from_value::<FromWorker>(value) {
        Ok(message) => WorkerEvent::Message(message),
        Err(_) => WorkerEvent::Violation(format!(
            "worker {program} sent a message that could not be read"
        )),
    }
}

async fn read_loop(
    mut stdout: Pin<Box<dyn AsyncRead + Send>>,
    events: mpsc::UnboundedSender<WorkerEvent>,
    program: String,
) {
    loop {
        let event = match protocol::read_frame(&mut stdout).await {
            Ok(frame) => decode(&frame.json, &program),
            Err(FrameError::Closed) => WorkerEvent::Closed,
            Err(e) => WorkerEvent::Violation(format!(
                "worker {program} sent a frame that could not be read: {e}"
            )),
        };
        let last = !matches!(event, WorkerEvent::Message(_));
        if events.send(event).is_err() || last {
            return;
        }
    }
}

enum Outgoing {
    Message(Vec<u8>),
    /// A message and its audio, written straight from the shared buffer.
    WithAudio(Vec<u8>, PcmSlice),
}

async fn write_loop(
    mut stdin: Pin<Box<dyn AsyncWrite + Send>>,
    mut outgoing: mpsc::UnboundedReceiver<Outgoing>,
) {
    while let Some(next) = outgoing.recv().await {
        let written = match next {
            Outgoing::Message(json) => protocol::write_frame(&mut stdin, &json, &[]).await,
            Outgoing::WithAudio(json, pcm) => {
                protocol::write_frame(&mut stdin, &json, &audio::s16le_bytes(pcm.samples())).await
            }
        };
        if written.is_err() {
            break;
        }
    }
    // Dropping stdin is the worker's end of file.
}

/// A live worker process and its three tasks.
struct Worker {
    outgoing: mpsc::UnboundedSender<Outgoing>,
    events: mpsc::UnboundedReceiver<WorkerEvent>,
    child: Box<dyn WorkerChild>,
    pid: Option<u32>,
    hello: Option<Hello>,
    /// The spec being loaded, as sent (`use_gpu` already reflects `force_cpu`).
    loading: Option<Arc<LoadSpec>>,
    loaded: Option<(Arc<LoadSpec>, Loaded)>,
    tasks: Vec<JoinHandle<()>>,
}

impl Worker {
    fn send(&self, message: &ToWorker) {
        let _ = self
            .outgoing
            .send(Outgoing::Message(protocol::to_json(message)));
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Stopped,
    Spawning,
    /// `hello` received, no model asked for yet.
    Spawned,
    Loading,
    Ready,
    Busy,
    Draining,
    Failed,
    Unsupported,
}

#[derive(Clone, Copy, Debug)]
enum Timer {
    Hello,
    Load,
    Job(&'static str),
    Drain,
}

enum Loss {
    /// Stdout ended: the worker exited or crashed.
    Closed,
    Deadline(&'static str),
    Protocol(String),
}

struct Running {
    job: Job,
    gpu: bool,
}

struct Actor {
    spawner: Arc<dyn WorkerSpawner>,
    config: SupervisorConfig,
    commands: mpsc::UnboundedReceiver<Command>,
    status: watch::Sender<SupervisorStatus>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// The platform whose needs a `hello` is judged by: the host's, outside tests.
    platform: cpu::Platform,
    worker: Option<Worker>,
    phase: Phase,
    force_cpu: bool,
    /// The load under way is the one CPU retry of a GPU load that failed. Should it
    /// fail too, `force_cpu` is cleared again.
    cpu_load_retry: bool,
    unsupported: Option<Vec<String>>,
    last_error: Option<String>,
    vad_queue: VecDeque<Job>,
    rotation: VecDeque<StreamKey>,
    streams: HashMap<StreamKey, VecDeque<Job>>,
    running: Option<Running>,
    waiters: Vec<Waiter>,
    unload_waiters: Vec<oneshot::Sender<()>>,
    deadline: Option<(Instant, Timer)>,
    idle_since: Instant,
}

/// What the engine needs on `platform` that the worker's own CPUID view lacks.
fn missing_in(platform: cpu::Platform, hello: &Hello) -> Vec<String> {
    cpu::missing_features_for(platform, &hello.cpu.into())
        .into_iter()
        .map(str::to_string)
        .collect()
}

async fn next_event(worker: &mut Option<Worker>) -> WorkerEvent {
    match worker {
        Some(worker) => worker.events.recv().await.unwrap_or(WorkerEvent::Closed),
        None => std::future::pending().await,
    }
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

impl Actor {
    async fn run(mut self) {
        loop {
            self.publish();
            let wake = self.next_wake();
            tokio::select! {
                biased;
                event = next_event(&mut self.worker) => match event {
                    WorkerEvent::Message(message) => self.on_message(message).await,
                    WorkerEvent::Violation(message) => self.lose_worker(Loss::Protocol(message)).await,
                    WorkerEvent::Closed => self.lose_worker(Loss::Closed).await,
                },
                command = self.commands.recv() => match command {
                    Some(command) => self.on_command(command).await,
                    None => {
                        self.stop_everything().await;
                        return;
                    }
                },
                () = sleep_until(wake) => self.on_timer().await,
            }
        }
    }

    /// The handle was dropped: kill the worker and answer everyone.
    async fn stop_everything(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.child.start_kill();
            let _ = worker.child.wait().await;
        }
        self.fail_all(&SupervisorError::Stopped);
        for reply in self.unload_waiters.drain(..) {
            let _ = reply.send(());
        }
        self.phase = Phase::Stopped;
        self.publish();
    }

    fn publish(&self) {
        let next = self.snapshot();
        self.status.send_if_modified(|current| {
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        });
    }

    fn snapshot(&self) -> SupervisorStatus {
        let state = match self.phase {
            Phase::Stopped | Phase::Draining => WorkerState::Stopped,
            Phase::Spawning | Phase::Spawned | Phase::Loading => WorkerState::Starting,
            Phase::Ready | Phase::Busy => WorkerState::Ready,
            Phase::Failed => WorkerState::Error,
            Phase::Unsupported => WorkerState::Unsupported,
        };
        let loaded = self.worker.as_ref().and_then(|w| w.loaded.as_ref());
        SupervisorStatus {
            state,
            busy: self.running.is_some(),
            backend: loaded.map(|(_, l)| l.backend.clone()),
            device: loaded.and_then(|(_, l)| l.device.clone()),
            threads: loaded.map(|(spec, _)| spec.n_threads),
            rss_bytes: loaded.and_then(|(_, l)| l.rss_bytes),
            pid: self.worker.as_ref().and_then(|w| w.pid),
            force_cpu: self.force_cpu,
            missing_cpu_features: self.unsupported.clone().unwrap_or_default(),
            queue: QueueStatus {
                streams: self.rotation.len(),
                segments: self.vad_queue.len()
                    + self.streams.values().map(VecDeque::len).sum::<usize>(),
            },
            hello: self.worker.as_ref().and_then(|w| w.hello.clone()),
            last_error: self.last_error.clone(),
        }
    }

    fn has_work(&self) -> bool {
        !self.waiters.is_empty() || !self.vad_queue.is_empty() || !self.rotation.is_empty()
    }

    fn next_wake(&self) -> Option<Instant> {
        if let Some((at, _)) = self.deadline {
            return Some(at);
        }
        let idle = self.running.is_none() && !self.has_work();
        (idle && matches!(self.phase, Phase::Ready | Phase::Spawned))
            .then(|| self.idle_since + self.config.idle_unload)
    }

    /// The spec as it will be sent: on the CPU while a GPU fault holds.
    fn effective(&self, spec: &Arc<LoadSpec>) -> Arc<LoadSpec> {
        if self.force_cpu && spec.use_gpu {
            Arc::new(LoadSpec {
                use_gpu: false,
                ..(**spec).clone()
            })
        } else {
            Arc::clone(spec)
        }
    }

    fn loaded_spec(&self) -> Option<Arc<LoadSpec>> {
        self.worker
            .as_ref()
            .and_then(|w| w.loaded.as_ref())
            .map(|(spec, _)| Arc::clone(spec))
    }

    async fn on_command(&mut self, command: Command) {
        match command {
            Command::Prepare { spec, reply } => {
                self.idle_since = Instant::now();
                if let Some(missing) = &self.unsupported {
                    let _ = reply.send(Err(SupervisorError::UnsupportedCpu {
                        missing: missing.clone(),
                    }));
                    return;
                }
                let wanted = self.effective(&spec);
                if matches!(self.phase, Phase::Ready | Phase::Busy) {
                    if let Some(worker) = &self.worker {
                        if let Some((loaded_spec, loaded)) = &worker.loaded {
                            if loaded_spec.same_model(&wanted) {
                                let _ = reply.send(Ok(loaded.clone()));
                                return;
                            }
                        }
                    }
                }
                self.waiters.push((spec, reply));
                self.make_progress();
            }
            Command::SpawnAhead => {
                if self.unsupported.is_none()
                    && self.worker.is_none()
                    && matches!(self.phase, Phase::Stopped | Phase::Failed)
                {
                    self.start_worker();
                }
            }
            Command::Submit(job) => {
                let job = *job;
                self.idle_since = Instant::now();
                if let Some(missing) = &self.unsupported {
                    job.fail(SupervisorError::UnsupportedCpu {
                        missing: missing.clone(),
                    });
                    return;
                }
                match job.stream() {
                    None => self.vad_queue.push_back(job),
                    Some(stream) => {
                        let queue = self.streams.entry(stream).or_default();
                        if queue.is_empty() && !self.rotation.contains(&stream) {
                            self.rotation.push_back(stream);
                        }
                        queue.push_back(job);
                    }
                }
                self.make_progress();
            }
            Command::Cancel(id) => {
                if let Some(job) = self.remove_queued(|job| job.id == id).pop() {
                    job.fail(SupervisorError::Cancelled);
                }
                self.cancel_running(|job| job.id == id);
            }
            Command::CancelStream(stream) => {
                for job in self.remove_queued(|job| job.stream() == Some(stream)) {
                    job.fail(SupervisorError::Cancelled);
                }
                self.cancel_running(|job| job.stream() == Some(stream));
            }
            Command::Unload(reply) => {
                for job in self.remove_queued(|_| true) {
                    job.fail(SupervisorError::Cancelled);
                }
                for (_, waiter) in self.waiters.drain(..) {
                    let _ = waiter.send(Err(SupervisorError::Cancelled));
                }
                if let Some(running) = self.running.take() {
                    if let Some(worker) = &self.worker {
                        worker.send(&ToWorker::Cancel { id: running.job.id });
                    }
                    // Answered now; the worker's late answer is ignored.
                    running.job.fail(SupervisorError::Cancelled);
                }
                if self.worker.is_some() {
                    if self.phase != Phase::Draining {
                        self.begin_drain();
                    }
                    self.unload_waiters.push(reply);
                } else {
                    if self.phase != Phase::Unsupported {
                        self.phase = Phase::Stopped;
                    }
                    self.force_cpu = false;
                    self.cpu_load_retry = false;
                    let _ = reply.send(());
                }
            }
            Command::KillWorker => {
                if let Some(worker) = &mut self.worker {
                    let _ = worker.child.start_kill();
                }
            }
        }
    }

    /// Removes every queued job that matches, keeping the rest in order.
    fn remove_queued(&mut self, matches: impl Fn(&Job) -> bool) -> Vec<Job> {
        let mut removed = Vec::new();
        let mut kept = VecDeque::new();
        for job in self.vad_queue.drain(..) {
            if matches(&job) {
                removed.push(job);
            } else {
                kept.push_back(job);
            }
        }
        self.vad_queue = kept;
        for queue in self.streams.values_mut() {
            let mut kept = VecDeque::new();
            for job in queue.drain(..) {
                if matches(&job) {
                    removed.push(job);
                } else {
                    kept.push_back(job);
                }
            }
            *queue = kept;
        }
        self.streams.retain(|_, queue| !queue.is_empty());
        let streams = &self.streams;
        self.rotation.retain(|stream| streams.contains_key(stream));
        removed
    }

    /// Asks the worker to abort the running job when it matches; the worker answers
    /// `cancelled`, or the result when it finished first.
    fn cancel_running(&mut self, matches: impl Fn(&Job) -> bool) {
        if let (Some(running), Some(worker)) = (&self.running, &self.worker) {
            if matches(&running.job) {
                worker.send(&ToWorker::Cancel { id: running.job.id });
            }
        }
    }

    /// Moves toward serving what is waiting: spawn, load or dispatch.
    fn make_progress(&mut self) {
        match self.phase {
            Phase::Unsupported => {
                let missing = self.unsupported.clone().unwrap_or_default();
                self.fail_all(&SupervisorError::UnsupportedCpu { missing });
            }
            Phase::Stopped | Phase::Failed => {
                if self.has_work() {
                    self.start_worker();
                }
            }
            Phase::Spawned => {
                if let Some(spec) = self.next_spec() {
                    self.send_load(spec);
                }
            }
            Phase::Ready => self.dispatch(),
            Phase::Spawning | Phase::Loading | Phase::Busy | Phase::Draining => {}
        }
    }

    /// The model the next piece of work needs.
    fn next_spec(&self) -> Option<Arc<LoadSpec>> {
        let job = self.vad_queue.front().or_else(|| {
            self.rotation
                .front()
                .and_then(|stream| self.streams.get(stream))
                .and_then(VecDeque::front)
        });
        job.map(|job| Arc::clone(&job.spec))
            .or_else(|| self.waiters.first().map(|(spec, _)| Arc::clone(spec)))
            .map(|spec| self.effective(&spec))
    }

    fn take_next_job(&mut self) -> Option<Job> {
        if let Some(job) = self.vad_queue.pop_front() {
            return Some(job);
        }
        let stream = self.rotation.pop_front()?;
        let queue = self.streams.get_mut(&stream)?;
        let job = queue.pop_front();
        if queue.is_empty() {
            self.streams.remove(&stream);
        } else {
            self.rotation.push_back(stream);
        }
        job
    }

    /// Puts a job back at the head of its line.
    fn put_back(&mut self, job: Job) {
        match job.stream() {
            None => self.vad_queue.push_front(job),
            Some(stream) => {
                self.streams.entry(stream).or_default().push_front(job);
                self.rotation.retain(|s| *s != stream);
                self.rotation.push_front(stream);
            }
        }
    }

    fn start_worker(&mut self) {
        let process = match self.spawner.spawn() {
            Ok(process) => process,
            Err(e) => {
                let error = SupervisorError::Spawn(e);
                self.last_error = Some(error.to_string());
                self.fail_all(&error);
                self.cpu_load_retry_failed();
                self.phase = Phase::Failed;
                return;
            }
        };
        let WorkerProcess {
            stdin,
            stdout,
            stderr,
            child,
        } = process;
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::unbounded_channel();
        let tasks = vec![
            tokio::spawn(write_loop(stdin, outgoing_rx)),
            tokio::spawn(read_loop(stdout, events_tx, self.spawner.program())),
            tokio::spawn(stderr_loop(
                stderr,
                Arc::clone(&self.stderr),
                self.config.stderr_lines,
            )),
        ];
        self.worker = Some(Worker {
            outgoing,
            events,
            pid: child.pid(),
            child,
            hello: None,
            loading: None,
            loaded: None,
            tasks,
        });
        self.phase = Phase::Spawning;
        self.deadline = Some((Instant::now() + self.config.deadlines.hello, Timer::Hello));
    }

    fn send_load(&mut self, spec: Arc<LoadSpec>) {
        let load = match spec.to_load() {
            Ok(load) => load,
            Err(error) => {
                self.fail_spec(&spec, &error);
                self.make_progress();
                return;
            }
        };
        let Some(worker) = &mut self.worker else {
            return;
        };
        worker.send(&ToWorker::Load(load));
        worker.loading = Some(spec);
        self.phase = Phase::Loading;
        self.deadline = Some((Instant::now() + self.config.deadlines.load, Timer::Load));
    }

    fn dispatch(&mut self) {
        if self.phase != Phase::Ready || self.running.is_some() {
            return;
        }
        let Some(loaded) = self.loaded_spec() else {
            return;
        };
        let Some(job) = self.take_next_job() else {
            // A prepare for another model, with no job waiting.
            let other = self
                .waiters
                .iter()
                .map(|(spec, _)| self.effective(spec))
                .find(|spec| !spec.same_model(&loaded));
            if let Some(spec) = other {
                self.send_load(spec);
            }
            return;
        };
        let wanted = self.effective(&job.spec);
        if !wanted.same_model(&loaded) {
            self.put_back(job);
            self.send_load(wanted);
            return;
        }
        // Only a transcription touches the GPU. `vad_segments` runs on the separate
        // detector, which the shim opens with `use_gpu = false` (`br_vad_init`), so it
        // gets the CPU deadline, and a worker lost during one is not a GPU fault: the job
        // fails and the next one gets a fresh worker on the GPU (DD-7).
        let gpu = matches!(job.kind, JobKind::Transcribe { .. })
            && self
                .worker
                .as_ref()
                .and_then(|w| w.loaded.as_ref())
                .is_some_and(|(_, l)| l.backend == "metal");
        let message = match &job.kind {
            JobKind::Transcribe { params, .. } => ToWorker::Transcribe(Transcribe {
                id: job.id,
                bias: params.bias.clone(),
                prompt_terms: params.prompt_terms.clone(),
                prompt_budget_tokens: params.prompt_budget_tokens,
                language: params.language.clone(),
                decode: params.decode.clone(),
                vad: params.vad.clone(),
            }),
            JobKind::Vad { vad, .. } => ToWorker::VadSegments(VadSegments {
                id: job.id,
                vad: vad.clone(),
            }),
        };
        let limit = self.config.deadlines.job(job.pcm().duration_secs(), gpu);
        if let Some(worker) = &self.worker {
            let _ = worker.outgoing.send(Outgoing::WithAudio(
                protocol::to_json(&message),
                job.pcm().clone(),
            ));
        }
        self.deadline = Some((Instant::now() + limit, Timer::Job(job.phase())));
        self.phase = Phase::Busy;
        self.running = Some(Running { job, gpu });
    }

    async fn on_message(&mut self, message: FromWorker) {
        match (self.phase, message) {
            (Phase::Spawning, FromWorker::Hello(hello)) => {
                let missing = missing_in(self.platform, &hello);
                if let Some(worker) = &mut self.worker {
                    worker.hello = Some(hello);
                }
                if !missing.is_empty() {
                    self.kill().await;
                    self.become_unsupported(missing);
                    return;
                }
                self.deadline = None;
                self.phase = Phase::Spawned;
                self.idle_since = Instant::now();
                self.make_progress();
            }
            (Phase::Loading, FromWorker::Loaded(loaded)) => {
                let Some(worker) = &mut self.worker else {
                    return;
                };
                let Some(spec) = worker.loading.take() else {
                    return;
                };
                worker.loaded = Some((Arc::clone(&spec), loaded.clone()));
                self.deadline = None;
                self.phase = Phase::Ready;
                self.last_error = None;
                // A CPU retry that loads confirms the GPU fault: `force_cpu` holds.
                self.cpu_load_retry = false;
                self.idle_since = Instant::now();
                let waiters = std::mem::take(&mut self.waiters);
                for (wanted, reply) in waiters {
                    if self.effective(&wanted).same_model(&spec) {
                        let _ = reply.send(Ok(loaded.clone()));
                    } else {
                        self.waiters.push((wanted, reply));
                    }
                }
                self.dispatch();
            }
            (Phase::Loading, FromWorker::Error(e)) if e.id.is_none() => {
                let spec = self.worker.as_mut().and_then(|w| w.loading.take());
                if spec.as_ref().is_some_and(|s| s.use_gpu) && !self.force_cpu {
                    // The Metal warm up runs inside `load`, so a GPU that cannot serve
                    // answers here. One load on the CPU, in a fresh worker; the waiters
                    // and the queue wait for it (DD-7).
                    self.kill().await;
                    self.begin_cpu_load_retry();
                    return;
                }
                let error = SupervisorError::ModelLoadFailed(e.message);
                self.last_error = Some(error.to_string());
                if let Some(spec) = spec {
                    self.fail_spec(&spec, &error);
                }
                self.cpu_load_retry_failed();
                self.kill().await;
                self.phase = Phase::Failed;
                self.make_progress();
            }
            (Phase::Busy, FromWorker::Result(result)) if self.is_running(Some(result.id)) => {
                if let Some(Running { job, .. }) = self.finish_running() {
                    if let JobKind::Transcribe { reply, .. } = job.kind {
                        let _ = reply.send(Ok(result));
                    }
                }
                self.dispatch();
            }
            (Phase::Busy, FromWorker::Segments(segments)) if self.is_running(Some(segments.id)) => {
                if let Some(Running { job, .. }) = self.finish_running() {
                    if let JobKind::Vad { reply, .. } = job.kind {
                        let _ = reply.send(Ok(segments.spans));
                    }
                }
                self.dispatch();
            }
            (Phase::Busy, FromWorker::Error(e)) if e.id.is_none() || self.is_running(e.id) => {
                if let Some(Running { job, .. }) = self.finish_running() {
                    job.fail(match e.code {
                        ErrorCode::Cancelled => SupervisorError::Cancelled,
                        ErrorCode::BadAudio => SupervisorError::BadAudio,
                        ErrorCode::ModelLoadFailed => SupervisorError::ModelLoadFailed(e.message),
                        ErrorCode::Internal | ErrorCode::Unknown => {
                            SupervisorError::Internal(e.message)
                        }
                    });
                }
                self.dispatch();
            }
            // A late answer to a cancelled or abandoned job, a repeated hello, or a
            // message from a newer worker: nothing waits for it.
            _ => {}
        }
    }

    fn is_running(&self, id: Option<u64>) -> bool {
        self.running.as_ref().map(|r| r.job.id) == id && id.is_some()
    }

    fn finish_running(&mut self) -> Option<Running> {
        self.deadline = None;
        self.phase = Phase::Ready;
        self.idle_since = Instant::now();
        self.running.take()
    }

    async fn on_timer(&mut self) {
        match self.deadline.take() {
            Some((_, Timer::Hello)) => self.lose_worker(Loss::Deadline("hello")).await,
            Some((_, Timer::Load)) => self.lose_worker(Loss::Deadline("load")).await,
            Some((_, Timer::Job(phase))) => self.lose_worker(Loss::Deadline(phase)).await,
            Some((_, Timer::Drain)) => self.lose_worker(Loss::Deadline("shutdown")).await,
            // The idle unload (DD-7): the worker's exit returns all its memory.
            None => self.begin_drain(),
        }
    }

    fn begin_drain(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        worker.send(&ToWorker::Shutdown);
        self.phase = Phase::Draining;
        self.deadline = Some((
            Instant::now() + self.config.deadlines.shutdown_grace,
            Timer::Drain,
        ));
    }

    /// Kills and reaps the worker now.
    async fn kill(&mut self) -> Option<i32> {
        let mut worker = self.worker.take()?;
        self.deadline = None;
        let _ = worker.child.start_kill();
        worker.child.wait().await.ok().flatten()
    }

    /// The worker is gone or must go: reap it, then decide what its work does next.
    async fn lose_worker(&mut self, loss: Loss) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };
        self.deadline = None;
        let code = match loss {
            Loss::Closed => reap(worker.child.as_mut(), EXIT_GRACE).await,
            Loss::Deadline(_) | Loss::Protocol(_) => {
                let _ = worker.child.start_kill();
                worker.child.wait().await.ok().flatten()
            }
        };
        let missing = worker
            .hello
            .as_ref()
            .map(|hello| missing_in(self.platform, hello))
            .unwrap_or_default();
        let gpu_load = worker.loading.as_ref().is_some_and(|spec| spec.use_gpu);
        drop(worker);
        if code == Some(exit::UNSUPPORTED_CPU) {
            self.become_unsupported(missing);
            return;
        }
        let gpu_fault = matches!(loss, Loss::Closed | Loss::Deadline(_));
        let error = match loss {
            Loss::Closed => SupervisorError::WorkerExited { code },
            Loss::Deadline(phase) => SupervisorError::Deadline { phase },
            Loss::Protocol(message) => SupervisorError::Protocol(message),
        };
        let phase = std::mem::replace(&mut self.phase, Phase::Stopped);
        match phase {
            Phase::Draining => {
                self.force_cpu = false;
                self.cpu_load_retry = false;
                for reply in self.unload_waiters.drain(..) {
                    let _ = reply.send(());
                }
            }
            Phase::Loading if gpu_load && gpu_fault && !self.force_cpu => {
                // DD-7 for a load: the Metal warm up runs inside `load`, so the first GPU
                // work a worker does is here. One load on the CPU in a fresh worker; the
                // waiters and the queue stay for it.
                self.begin_cpu_load_retry();
                return;
            }
            Phase::Spawning | Phase::Loading => {
                self.last_error = Some(error.to_string());
                self.fail_all(&error);
                self.cpu_load_retry_failed();
                self.phase = Phase::Failed;
            }
            Phase::Busy => {
                if let Some(Running { mut job, gpu }) = self.running.take() {
                    if gpu && gpu_fault && !job.retried {
                        // DD-7: one retry of the same job, in a fresh worker, on the CPU.
                        self.force_cpu = true;
                        job.retried = true;
                        self.put_back(job);
                    } else {
                        self.last_error = Some(error.to_string());
                        job.fail(error);
                    }
                }
            }
            _ => {}
        }
        self.make_progress();
    }

    /// After a GPU load failed: the next load goes to the CPU, once, in a fresh worker.
    /// The worker is already gone; the waiters and the queue are kept.
    fn begin_cpu_load_retry(&mut self) {
        self.force_cpu = true;
        self.cpu_load_retry = true;
        self.phase = Phase::Stopped;
        self.make_progress();
    }

    /// The CPU retry of a load failed as well, so the GPU was not the cause (a damaged
    /// file fails both ways): the next load asks for the GPU again.
    fn cpu_load_retry_failed(&mut self) {
        if std::mem::take(&mut self.cpu_load_retry) {
            self.force_cpu = false;
        }
    }

    fn become_unsupported(&mut self, missing: Vec<String>) {
        self.phase = Phase::Unsupported;
        self.deadline = None;
        self.cpu_load_retry = false;
        self.unsupported = Some(missing.clone());
        self.fail_all(&SupervisorError::UnsupportedCpu { missing });
        for reply in self.unload_waiters.drain(..) {
            let _ = reply.send(());
        }
    }

    /// Fails the waiters and every queued or running job.
    fn fail_all(&mut self, error: &SupervisorError) {
        for (_, reply) in self.waiters.drain(..) {
            let _ = reply.send(Err(error.duplicate()));
        }
        for job in self.remove_queued(|_| true) {
            job.fail(error.duplicate());
        }
        if let Some(running) = self.running.take() {
            running.job.fail(error.duplicate());
        }
    }

    /// Fails the waiters and queued jobs that need `spec`.
    fn fail_spec(&mut self, spec: &LoadSpec, error: &SupervisorError) {
        let waiters = std::mem::take(&mut self.waiters);
        for (wanted, reply) in waiters {
            if self.effective(&wanted).same_model(spec) {
                let _ = reply.send(Err(error.duplicate()));
            } else {
                self.waiters.push((wanted, reply));
            }
        }
        let force_cpu = self.force_cpu;
        let needs = |job: &Job| {
            let wanted = LoadSpec {
                use_gpu: job.spec.use_gpu && !force_cpu,
                ..(*job.spec).clone()
            };
            wanted.same_model(spec)
        };
        for job in self.remove_queued(needs) {
            job.fail(error.duplicate());
        }
    }
}

/// Every deadline and rule above, on tokio's paused clock with an in-memory worker. A
/// real process cannot share a paused clock: tokio advances paused time whenever the
/// runtime is idle, including while it waits on a pipe.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{read_frame, write_frame, CpuFlags, ErrorMsg, Segments, Timings};
    use std::sync::atomic::AtomicU32;
    use tokio::io::{AsyncWriteExt, DuplexStream};

    /// How the in-memory worker behaves.
    #[derive(Clone, Default)]
    struct Script {
        /// The protocol its `hello` claims; 1 when unset.
        protocol: Option<u32>,
        no_hello: bool,
        /// The CPU its `hello` reports; this host's when unset.
        hello_cpu: Option<CpuFlags>,
        exit_after_hello: Option<i32>,
        load_delay: Duration,
        load_error: bool,
        gpu_load_error: bool,
        crash_on_gpu_load: bool,
        hang_on_gpu_load: bool,
        job_delay: Duration,
        hang_jobs: bool,
        hang_on_gpu: bool,
        crash_jobs: bool,
        crash_on_gpu: bool,
        crash_on_job: Option<u64>,
        crash_on_vad: bool,
        /// Answers a `transcribe` with a frame head over the 32 MiB cap.
        oversized_frame: bool,
        ignore_shutdown: bool,
        stderr: Vec<String>,
    }

    /// What the in-memory workers received, across every spawn.
    #[derive(Default)]
    struct Log {
        spawns: usize,
        events: Vec<String>,
    }

    struct FakeSpawner {
        script: Script,
        log: Arc<Mutex<Log>>,
        next_pid: AtomicU32,
    }

    struct AbortOnDrop(JoinHandle<()>);

    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    impl WorkerSpawner for FakeSpawner {
        fn spawn(&self) -> io::Result<WorkerProcess> {
            let (parent_stdin, worker_stdin) = tokio::io::duplex(1 << 16);
            let (worker_stdout, parent_stdout) = tokio::io::duplex(1 << 16);
            let (worker_stderr, parent_stderr) = tokio::io::duplex(1 << 16);
            let (kill, killed) = oneshot::channel::<()>();
            let (exit_tx, exit) = oneshot::channel::<Option<i32>>();
            self.log.lock().expect("log").spawns += 1;
            let script = self.script.clone();
            let log = Arc::clone(&self.log);
            tokio::spawn(async move {
                let code = tokio::select! {
                    code = fake_worker(script, Arc::clone(&log), worker_stdin, worker_stdout, worker_stderr) => code,
                    _ = killed => {
                        log.lock().expect("log").events.push("killed".into());
                        None
                    }
                };
                let _ = exit_tx.send(code);
            });
            Ok(WorkerProcess {
                stdin: Box::pin(parent_stdin),
                stdout: Box::pin(parent_stdout),
                stderr: Box::pin(parent_stderr),
                child: Box::new(FakeChild {
                    pid: self.next_pid.fetch_add(1, Ordering::Relaxed),
                    kill: Some(kill),
                    exit,
                    code: None,
                }),
            })
        }

        fn program(&self) -> String {
            "/opt/fake/biorouterd".into()
        }
    }

    struct FakeChild {
        pid: u32,
        kill: Option<oneshot::Sender<()>>,
        exit: oneshot::Receiver<Option<i32>>,
        code: Option<Option<i32>>,
    }

    impl WorkerChild for FakeChild {
        fn pid(&self) -> Option<u32> {
            Some(self.pid)
        }

        fn start_kill(&mut self) -> io::Result<()> {
            if let Some(kill) = self.kill.take() {
                let _ = kill.send(());
            }
            Ok(())
        }

        fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<Option<i32>>> + Send + '_>> {
            Box::pin(async move {
                if let Some(code) = self.code {
                    return Ok(code);
                }
                // Cancel safe: the receiver stays in place until it answers.
                let code = (&mut self.exit).await.unwrap_or(None);
                self.code = Some(code);
                Ok(code)
            })
        }
    }

    async fn send(stdout: &mut DuplexStream, message: &FromWorker) {
        let _ = write_frame(stdout, &protocol::to_json(message), &[]).await;
    }

    fn record(log: &Mutex<Log>, message: &ToWorker) {
        let event = match message {
            ToWorker::Load(load) => format!("load {} gpu={}", load.model_sha256, load.use_gpu),
            ToWorker::Transcribe(t) => format!("transcribe {}", t.id),
            ToWorker::VadSegments(v) => format!("vad {}", v.id),
            ToWorker::Cancel { id } => format!("cancel {id}"),
            ToWorker::Shutdown => "shutdown".into(),
            ToWorker::SetBias(_) | ToWorker::Unknown => "other".into(),
        };
        log.lock().expect("log").events.push(event);
    }

    async fn fake_worker(
        script: Script,
        log: Arc<Mutex<Log>>,
        stdin: DuplexStream,
        mut stdout: DuplexStream,
        mut stderr: DuplexStream,
    ) -> Option<i32> {
        for line in &script.stderr {
            let _ = stderr.write_all(line.as_bytes()).await;
        }
        if script.no_hello {
            std::future::pending::<()>().await;
        }
        let hello = Hello {
            protocol: script.protocol.unwrap_or(PROTOCOL_VERSION),
            whisper: "4da27c41".into(),
            ggml: "0.28.0".into(),
            backends: vec!["cpu".into()],
            cpu: script.hello_cpu.unwrap_or_else(|| cpu::detect().into()),
            isa: "test".into(),
            system_info: String::new(),
        };
        send(&mut stdout, &FromWorker::Hello(hello)).await;
        if let Some(code) = script.exit_after_hello {
            return Some(code);
        }
        // Frames are read by their own task, so a select below never cuts one in half.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _reader = AbortOnDrop(tokio::spawn(async move {
            let mut stdin = stdin;
            while let Ok(frame) = read_frame(&mut stdin).await {
                let message: ToWorker =
                    serde_json::from_slice(&frame.json).unwrap_or(ToWorker::Unknown);
                if tx.send((message, frame.payload.len())).is_err() {
                    break;
                }
            }
        }));
        let mut backend = "cpu";
        loop {
            let Some((message, payload_len)) = rx.recv().await else {
                return Some(0);
            };
            record(&log, &message);
            match message {
                ToWorker::Load(load) => {
                    if load.use_gpu && script.crash_on_gpu_load {
                        return Some(134);
                    }
                    let delay = if load.use_gpu && script.hang_on_gpu_load {
                        Duration::from_secs(1_000_000)
                    } else {
                        script.load_delay
                    };
                    tokio::time::sleep(delay).await;
                    if script.load_error || (load.use_gpu && script.gpu_load_error) {
                        send(
                            &mut stdout,
                            &FromWorker::Error(ErrorMsg {
                                id: None,
                                code: ErrorCode::ModelLoadFailed,
                                message: "model could not be loaded".into(),
                            }),
                        )
                        .await;
                        continue;
                    }
                    backend = if load.use_gpu { "metal" } else { "cpu" };
                    send(
                        &mut stdout,
                        &FromWorker::Loaded(Loaded {
                            load_ms: 100,
                            warmup_ms: 50,
                            backend: backend.into(),
                            device: load.use_gpu.then(|| "Fake GPU".to_string()),
                            rss_bytes: Some(1 << 30),
                            n_vocab: 51_864,
                            multilingual: false,
                        }),
                    )
                    .await;
                }
                ToWorker::Transcribe(t) => {
                    let gpu = backend == "metal";
                    if script.crash_jobs
                        || (gpu && script.crash_on_gpu)
                        || script.crash_on_job == Some(t.id)
                    {
                        return Some(134);
                    }
                    if script.oversized_frame {
                        let json = protocol::to_json(&FromWorker::Segments(Segments {
                            id: t.id,
                            spans: Vec::new(),
                        }));
                        let mut head = (json.len() as u32).to_le_bytes().to_vec();
                        head.extend_from_slice(&json);
                        head.extend_from_slice(&(protocol::MAX_FRAME_BYTES as u32).to_le_bytes());
                        let _ = stdout.write_all(&head).await;
                        // The supervisor must refuse the head; nothing more comes.
                        std::future::pending::<()>().await;
                    }
                    let answer = FromWorker::Result(TranscribeResult {
                        id: t.id,
                        text: format!("job {} on {backend}", t.id),
                        segments: Vec::new(),
                        prompt_terms_used: 0,
                        timings: Timings::default(),
                    });
                    let hang = script.hang_jobs || (gpu && script.hang_on_gpu);
                    if let Some(code) =
                        work(&script, hang, &mut rx, &log, &mut stdout, t.id, answer).await
                    {
                        return code;
                    }
                }
                ToWorker::VadSegments(v) => {
                    if script.crash_on_vad {
                        return Some(134);
                    }
                    let answer = FromWorker::Segments(Segments {
                        id: v.id,
                        spans: vec![Span {
                            t0_ms: 0,
                            t1_ms: payload_len as u64 / 32,
                        }],
                    });
                    if let Some(code) = work(
                        &script,
                        script.hang_jobs,
                        &mut rx,
                        &log,
                        &mut stdout,
                        v.id,
                        answer,
                    )
                    .await
                    {
                        return code;
                    }
                }
                ToWorker::Shutdown if script.ignore_shutdown => {}
                ToWorker::Shutdown => return Some(0),
                _ => {}
            }
        }
    }

    /// Works on one job, answering a `cancel` for it. `Some(code)` when the worker exits.
    async fn work(
        script: &Script,
        hang: bool,
        rx: &mut mpsc::UnboundedReceiver<(ToWorker, usize)>,
        log: &Mutex<Log>,
        stdout: &mut DuplexStream,
        id: u64,
        answer: FromWorker,
    ) -> Option<Option<i32>> {
        let delay = if hang {
            Duration::from_secs(1_000_000)
        } else {
            script.job_delay
        };
        let done = tokio::time::sleep(delay);
        tokio::pin!(done);
        loop {
            tokio::select! {
                () = &mut done => {
                    send(stdout, &answer).await;
                    return None;
                }
                next = rx.recv() => {
                    let Some((message, _)) = next else {
                        return Some(Some(0));
                    };
                    record(log, &message);
                    match message {
                        ToWorker::Cancel { id: cancelled } if cancelled == id => {
                            send(stdout, &FromWorker::Error(ErrorMsg {
                                id: Some(id),
                                code: ErrorCode::Cancelled,
                                message: "cancelled".into(),
                            })).await;
                            return None;
                        }
                        ToWorker::Shutdown => return Some(Some(0)),
                        _ => {}
                    }
                }
            }
        }
    }

    fn start_on(
        script: Script,
        config: SupervisorConfig,
        platform: cpu::Platform,
    ) -> (Supervisor, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        let spawner = FakeSpawner {
            script,
            log: Arc::clone(&log),
            next_pid: AtomicU32::new(1000),
        };
        (
            Supervisor::start_on(Arc::new(spawner), config, platform),
            log,
        )
    }

    fn start_with(script: Script, config: SupervisorConfig) -> (Supervisor, Arc<Mutex<Log>>) {
        start_on(script, config, cpu::Platform::host())
    }

    fn start(script: Script) -> (Supervisor, Arc<Mutex<Log>>) {
        start_with(script, SupervisorConfig::default())
    }

    fn model(sha: &str, gpu: bool) -> Arc<LoadSpec> {
        Arc::new(LoadSpec {
            model_path: PathBuf::from(format!("/models/{sha}.bin")),
            model_sha256: sha.into(),
            model_size: 10,
            vad_path: PathBuf::from("/models/vad/ggml-silero-v6.2.0.bin"),
            use_gpu: gpu,
            n_threads: 4,
        })
    }

    fn audio(seconds: f64) -> PcmSlice {
        PcmSlice::whole(vec![0i16; (seconds * 16_000.0) as usize].into())
    }

    fn params() -> TranscribeParams {
        TranscribeParams::new("en", 4)
    }

    fn events(log: &Mutex<Log>) -> Vec<String> {
        log.lock().expect("log").events.clone()
    }

    fn spawns(log: &Mutex<Log>) -> usize {
        log.lock().expect("log").spawns
    }

    fn secs(since: Instant) -> f64 {
        since.elapsed().as_secs_f64()
    }

    async fn wait_for_state(supervisor: &Supervisor, state: WorkerState) {
        let mut status = supervisor.subscribe();
        tokio::time::timeout(
            Duration::from_secs(3_600),
            status.wait_for(|s| s.state == state),
        )
        .await
        .expect("the state arrives")
        .expect("the supervisor runs");
    }

    #[test]
    fn job_deadlines_follow_dd_7() {
        let d = Deadlines::default();
        assert_eq!(d.job(1.0, true), Duration::from_secs(30));
        assert_eq!(d.job(10.0, true), Duration::from_secs(100));
        assert_eq!(d.job(1.0, false), Duration::from_secs(120));
        assert_eq!(d.job(25.0, false), Duration::from_secs(500));
    }

    #[tokio::test(start_paused = true)]
    async fn hello_is_awaited_for_60_s() {
        let (supervisor, log) = start(Script {
            no_hello: true,
            ..Script::default()
        });
        let started = Instant::now();
        let result = supervisor.prepare(model("aa", false)).await;
        assert!(
            matches!(result, Err(SupervisorError::Deadline { phase: "hello" })),
            "{result:?}"
        );
        assert!((60.0..61.0).contains(&secs(started)), "{}", secs(started));
        assert_eq!(supervisor.status().state, WorkerState::Error);
        assert_eq!(events(&log), ["killed"]);
        // The next request starts a fresh worker.
        let _ = supervisor.prepare(model("aa", false)).await;
        assert_eq!(spawns(&log), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn loaded_is_awaited_for_180_s() {
        let (supervisor, _log) = start(Script {
            load_delay: Duration::from_secs(1_000),
            ..Script::default()
        });
        let started = Instant::now();
        let result = supervisor.prepare(model("aa", false)).await;
        assert!(
            matches!(result, Err(SupervisorError::Deadline { phase: "load" })),
            "{result:?}"
        );
        assert!((180.0..181.0).contains(&secs(started)), "{}", secs(started));
    }

    #[tokio::test(start_paused = true)]
    async fn a_gpu_job_gets_max_30_s_or_10_x_audio_then_one_cpu_retry() {
        for (seconds, deadline) in [(1.0, 30.0), (10.0, 100.0)] {
            let (supervisor, log) = start(Script {
                hang_on_gpu: true,
                job_delay: Duration::from_secs(1),
                ..Script::default()
            });
            supervisor.prepare(model("aa", true)).await.expect("ready");
            let started = Instant::now();
            let result = supervisor
                .transcribe(model("aa", true), 1, audio(seconds), params())
                .await
                .expect("the CPU retry answers");
            assert_eq!(result.text, format!("job {} on cpu", result.id));
            let took = secs(started);
            assert!(
                (deadline + 1.0..deadline + 2.0).contains(&took),
                "{seconds} s: {took}"
            );
            assert!(supervisor.status().force_cpu);
            assert_eq!(
                events(&log),
                [
                    "load aa gpu=true",
                    "transcribe 1",
                    "killed",
                    "load aa gpu=false",
                    "transcribe 1"
                ]
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_cpu_job_gets_max_120_s_or_20_x_audio_and_no_retry() {
        for (seconds, deadline) in [(1.0, 120.0), (10.0, 200.0)] {
            let (supervisor, log) = start(Script {
                hang_jobs: true,
                ..Script::default()
            });
            supervisor.prepare(model("aa", false)).await.expect("ready");
            let started = Instant::now();
            let result = supervisor
                .transcribe(model("aa", false), 1, audio(seconds), params())
                .await;
            assert!(
                matches!(
                    result,
                    Err(SupervisorError::Deadline {
                        phase: "transcribe"
                    })
                ),
                "{result:?}"
            );
            let took = secs(started);
            assert!(
                (deadline..deadline + 1.0).contains(&took),
                "{seconds} s: {took}"
            );
            assert_eq!(spawns(&log), 1);
            assert!(!supervisor.status().force_cpu);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_worker_of_another_protocol_is_refused_and_named() {
        let (supervisor, log) = start(Script {
            protocol: Some(2),
            ..Script::default()
        });
        let result = supervisor.prepare(model("aa", false)).await;
        let Err(SupervisorError::Protocol(message)) = result else {
            panic!("{result:?}")
        };
        assert_eq!(
            message,
            "worker /opt/fake/biorouterd speaks protocol 2; this build speaks 1"
        );
        assert_eq!(events(&log), ["killed"]);
        assert_eq!(supervisor.status().state, WorkerState::Error);
    }

    #[tokio::test(start_paused = true)]
    async fn a_crash_is_a_typed_error_and_the_next_job_gets_a_fresh_worker() {
        let (supervisor, log) = start(Script {
            crash_on_job: Some(1),
            job_delay: Duration::from_millis(100),
            ..Script::default()
        });
        supervisor.prepare(model("aa", false)).await.expect("ready");
        let first_pid = supervisor.worker_pid();
        let result = supervisor
            .transcribe(model("aa", false), 1, audio(2.0), params())
            .await;
        assert!(
            matches!(
                result,
                Err(SupervisorError::WorkerExited { code: Some(134) })
            ),
            "{result:?}"
        );
        let result = supervisor
            .transcribe(model("aa", false), 1, audio(2.0), params())
            .await
            .expect("a fresh worker answers");
        assert_eq!(result.id, 2);
        assert_eq!(spawns(&log), 2);
        assert_ne!(supervisor.worker_pid(), first_pid);
    }

    #[tokio::test(start_paused = true)]
    async fn a_gpu_fault_moves_to_the_cpu_until_the_idle_unload() {
        let (supervisor, log) = start(Script {
            crash_on_gpu: true,
            job_delay: Duration::from_millis(100),
            ..Script::default()
        });
        let first = supervisor
            .transcribe(model("aa", true), 1, audio(2.0), params())
            .await
            .expect("the retry answers");
        assert_eq!(first.text, "job 1 on cpu");
        assert_eq!(spawns(&log), 2);
        assert!(supervisor.status().force_cpu);
        assert_eq!(supervisor.status().backend.as_deref(), Some("cpu"));
        // It stays on the CPU, in the same worker.
        let second = supervisor
            .transcribe(model("aa", true), 1, audio(2.0), params())
            .await
            .expect("the CPU answers");
        assert_eq!(second.text, "job 2 on cpu");
        assert_eq!(spawns(&log), 2);
        // The idle unload clears it, and the next load asks for the GPU again.
        tokio::time::sleep(Duration::from_secs(301)).await;
        wait_for_state(&supervisor, WorkerState::Stopped).await;
        assert!(!supervisor.status().force_cpu);
        let third = supervisor
            .transcribe(model("aa", true), 1, audio(2.0), params())
            .await
            .expect("the retry answers again");
        assert_eq!(third.text, "job 3 on cpu");
        let loads: Vec<String> = events(&log)
            .into_iter()
            .filter(|e| e.starts_with("load"))
            .collect();
        assert_eq!(
            loads,
            [
                "load aa gpu=true",
                "load aa gpu=false",
                "load aa gpu=true",
                "load aa gpu=false"
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_that_fails_on_the_cpu_retry_fails_once() {
        let (supervisor, log) = start(Script {
            crash_jobs: true,
            ..Script::default()
        });
        let result = supervisor
            .transcribe(model("aa", true), 1, audio(2.0), params())
            .await;
        assert!(
            matches!(
                result,
                Err(SupervisorError::WorkerExited { code: Some(134) })
            ),
            "{result:?}"
        );
        assert_eq!(spawns(&log), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_and_cancel_stream() {
        let (supervisor, log) = start(Script {
            job_delay: Duration::from_secs(10),
            ..Script::default()
        });
        supervisor.prepare(model("aa", false)).await.expect("ready");
        let spec = model("aa", false);
        let (a, a_answer) =
            supervisor.submit_transcribe(Arc::clone(&spec), 1, audio(2.0), params());
        let (b, b_answer) =
            supervisor.submit_transcribe(Arc::clone(&spec), 1, audio(2.0), params());
        let (_c, c_answer) =
            supervisor.submit_transcribe(Arc::clone(&spec), 2, audio(2.0), params());
        let started = Instant::now();
        supervisor.cancel(b);
        assert!(matches!(
            b_answer.await,
            Ok(Err(SupervisorError::Cancelled))
        ));
        supervisor.cancel_stream(2);
        assert!(matches!(
            c_answer.await,
            Ok(Err(SupervisorError::Cancelled))
        ));
        supervisor.cancel(a);
        assert!(matches!(
            a_answer.await,
            Ok(Err(SupervisorError::Cancelled))
        ));
        assert!(secs(started) < 1.0, "a cancel waits for nothing");
        assert_eq!(
            events(&log),
            [
                "load aa gpu=false".to_string(),
                format!("transcribe {}", a.0),
                format!("cancel {}", a.0)
            ]
        );
        let status = supervisor.status();
        assert_eq!(status.queue, QueueStatus::default());
        assert!(!status.busy);
    }

    #[tokio::test(start_paused = true)]
    async fn streams_take_turns_one_segment_each_and_detection_goes_first() {
        let (supervisor, log) = start(Script {
            job_delay: Duration::from_secs(1),
            ..Script::default()
        });
        let spec = model("aa", false);
        supervisor.prepare(Arc::clone(&spec)).await.expect("ready");
        let mut answers = Vec::new();
        for stream in [1, 1, 1, 2, 3, 3] {
            answers.push(
                supervisor
                    .submit_transcribe(Arc::clone(&spec), stream, audio(1.0), params())
                    .1,
            );
        }
        let (vad, vad_answer) =
            supervisor.submit_vad_segments(Arc::clone(&spec), audio(3.0), VadOptions::dd48());
        // Lets the actor take the commands; the first job runs for a virtual second.
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(
            supervisor.status().queue,
            QueueStatus {
                streams: 3,
                segments: 6
            }
        );
        for answer in answers {
            answer.await.expect("answered").expect("a result");
        }
        let spans = vad_answer.await.expect("answered").expect("spans");
        assert_eq!(
            spans,
            [Span {
                t0_ms: 0,
                t1_ms: 3_000
            }]
        );
        let order: Vec<String> = events(&log).into_iter().skip(1).collect();
        let expected: Vec<String> = [1, vad.0, 2, 4, 5, 3, 6]
            .into_iter()
            .map(|id| {
                if id == vad.0 {
                    format!("vad {id}")
                } else {
                    format!("transcribe {id}")
                }
            })
            .collect();
        assert_eq!(order, expected);
    }

    #[tokio::test(start_paused = true)]
    async fn the_worker_is_unloaded_after_300_s_idle() {
        let (supervisor, log) = start(Script::default());
        supervisor.prepare(model("aa", false)).await.expect("ready");
        tokio::time::sleep(Duration::from_secs(299)).await;
        assert_eq!(supervisor.status().state, WorkerState::Ready);
        tokio::time::sleep(Duration::from_secs(2)).await;
        wait_for_state(&supervisor, WorkerState::Stopped).await;
        assert_eq!(events(&log), ["load aa gpu=false", "shutdown"]);
        assert_eq!(supervisor.worker_pid(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_idle_unload_follows_its_setting_and_activity_restarts_it() {
        let config = SupervisorConfig {
            idle_unload: Duration::from_secs(30),
            ..SupervisorConfig::default()
        };
        let (supervisor, log) = start_with(
            Script {
                job_delay: Duration::from_secs(1),
                ..Script::default()
            },
            config,
        );
        supervisor.prepare(model("aa", false)).await.expect("ready");
        tokio::time::sleep(Duration::from_secs(20)).await;
        supervisor
            .transcribe(model("aa", false), 1, audio(1.0), params())
            .await
            .expect("a result");
        tokio::time::sleep(Duration::from_secs(29)).await;
        assert_eq!(supervisor.status().state, WorkerState::Ready);
        tokio::time::sleep(Duration::from_secs(2)).await;
        wait_for_state(&supervisor, WorkerState::Stopped).await;
        assert_eq!(events(&log).last().map(String::as_str), Some("shutdown"));
    }

    #[tokio::test(start_paused = true)]
    async fn exit_20_marks_the_processor_unsupported_for_good() {
        let (supervisor, log) = start(Script {
            exit_after_hello: Some(exit::UNSUPPORTED_CPU),
            ..Script::default()
        });
        let result = supervisor.prepare(model("aa", false)).await;
        assert!(
            matches!(result, Err(SupervisorError::UnsupportedCpu { .. })),
            "{result:?}"
        );
        assert_eq!(supervisor.status().state, WorkerState::Unsupported);
        let again = supervisor.prepare(model("aa", false)).await;
        assert!(matches!(again, Err(SupervisorError::UnsupportedCpu { .. })));
        let job = supervisor
            .transcribe(model("aa", false), 1, audio(1.0), params())
            .await;
        assert!(matches!(job, Err(SupervisorError::UnsupportedCpu { .. })));
        supervisor.spawn_ahead();
        assert!(matches!(
            supervisor.probe().await.hello,
            Err(SupervisorError::UnsupportedCpu { .. })
        ));
        assert_eq!(spawns(&log), 1, "nothing is spawned again");
    }

    #[tokio::test(start_paused = true)]
    async fn the_last_64_stderr_lines_are_kept_cut_and_escaped() {
        let mut stderr: Vec<String> = (0..100)
            .map(|i| format!("line {i} \x1b[1mbold\n"))
            .collect();
        stderr.push(format!("{}\n", "x".repeat(300)));
        let (supervisor, _log) = start(Script {
            stderr,
            ..Script::default()
        });
        supervisor.prepare(model("aa", false)).await.expect("ready");
        tokio::time::sleep(Duration::from_millis(10)).await;
        let tail = supervisor.stderr_tail();
        assert_eq!(tail.len(), 64);
        assert_eq!(
            tail.first().map(String::as_str),
            Some("line 37 \\u{1b}[1mbold")
        );
        assert_eq!(tail.last().map(String::len), Some(STDERR_LINE_BYTES));
        assert!(tail.iter().all(|line| !line.chars().any(char::is_control)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_for_another_model_loads_after_the_running_job() {
        let (supervisor, log) = start(Script {
            job_delay: Duration::from_secs(5),
            ..Script::default()
        });
        supervisor.prepare(model("aa", false)).await.expect("ready");
        let (_, first) = supervisor.submit_transcribe(model("aa", false), 1, audio(1.0), params());
        let (_, second) = supervisor.submit_transcribe(model("bb", false), 2, audio(1.0), params());
        first.await.expect("answered").expect("a result");
        second.await.expect("answered").expect("a result");
        assert_eq!(
            events(&log),
            [
                "load aa gpu=false",
                "transcribe 1",
                "load bb gpu=false",
                "transcribe 2"
            ]
        );
        assert_eq!(spawns(&log), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn spawn_ahead_starts_the_process_and_prepare_answers_from_memory() {
        let (supervisor, log) = start(Script::default());
        supervisor.spawn_ahead();
        wait_for_state(&supervisor, WorkerState::Starting).await;
        assert_eq!(spawns(&log), 1);
        let loaded = supervisor.prepare(model("aa", false)).await.expect("ready");
        assert_eq!(loaded.backend, "cpu");
        assert_eq!(
            supervisor.prepare(model("aa", false)).await.expect("again"),
            loaded
        );
        assert_eq!(events(&log), ["load aa gpu=false"]);
        let status = supervisor.status();
        assert_eq!(status.state, WorkerState::Ready);
        assert_eq!(status.threads, Some(4));
        assert_eq!(status.rss_bytes, Some(1 << 30));
        assert!(status.hello.is_some());
        let probe = supervisor.probe().await;
        assert!(probe.hello.is_ok());
        assert_eq!(probe.exit_code, None, "the running worker answered");
        assert_eq!(spawns(&log), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_load_error_fails_prepare_and_the_next_request_retries() {
        let (supervisor, log) = start(Script {
            load_error: true,
            ..Script::default()
        });
        let result = supervisor.prepare(model("aa", false)).await;
        let Err(SupervisorError::ModelLoadFailed(message)) = result else {
            panic!("{result:?}")
        };
        assert_eq!(message, "model could not be loaded");
        let status = supervisor.status();
        assert_eq!(status.state, WorkerState::Error);
        assert!(status.last_error.is_some());
        let _ = supervisor.prepare(model("aa", false)).await;
        assert_eq!(spawns(&log), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn unload_cancels_everything_and_stops_the_worker() {
        let (supervisor, log) = start(Script {
            job_delay: Duration::from_secs(10),
            ..Script::default()
        });
        let spec = model("aa", false);
        supervisor.prepare(Arc::clone(&spec)).await.expect("ready");
        let (_, running) = supervisor.submit_transcribe(Arc::clone(&spec), 1, audio(1.0), params());
        let (_, queued) = supervisor.submit_transcribe(Arc::clone(&spec), 2, audio(1.0), params());
        let started = Instant::now();
        supervisor.unload().await;
        assert!(secs(started) < 1.0);
        assert!(matches!(running.await, Ok(Err(SupervisorError::Cancelled))));
        assert!(matches!(queued.await, Ok(Err(SupervisorError::Cancelled))));
        assert_eq!(supervisor.status().state, WorkerState::Stopped);
        assert_eq!(
            events(&log),
            ["load aa gpu=false", "transcribe 1", "cancel 1", "shutdown"]
        );
        // Nothing loaded: unload answers at once.
        supervisor.unload().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_starts_a_worker_only_to_read_its_hello() {
        let (supervisor, log) = start(Script {
            stderr: vec!["probe warning\n".into()],
            ..Script::default()
        });
        let probe = supervisor.probe().await;
        let hello = probe.hello.expect("a hello");
        assert_eq!(hello.protocol, 1);
        assert_eq!(probe.exit_code, Some(0));
        assert_eq!(probe.stderr, ["probe warning"]);
        assert_eq!(spawns(&log), 1);
        assert_eq!(supervisor.status().state, WorkerState::Stopped);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_handle_stops_the_worker() {
        let (supervisor, log) = start(Script::default());
        supervisor.prepare(model("aa", false)).await.expect("ready");
        drop(supervisor);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(events(&log).last().map(String::as_str), Some("killed"));
    }

    #[tokio::test(start_paused = true)]
    async fn audio_over_the_caps_is_refused_before_it_is_queued() {
        let (supervisor, log) = start(Script::default());
        let result = supervisor
            .transcribe(model("aa", false), 1, audio(25.5), params())
            .await;
        assert!(matches!(result, Err(SupervisorError::BadAudio)));
        let result = supervisor
            .vad_segments(model("aa", false), audio(601.0), VadOptions::dd48())
            .await;
        assert!(matches!(result, Err(SupervisorError::BadAudio)));
        assert_eq!(spawns(&log), 0);
    }

    fn loads(log: &Mutex<Log>) -> Vec<String> {
        events(log)
            .into_iter()
            .filter(|e| e.starts_with("load"))
            .collect()
    }

    /// The detector never touches the GPU, so a worker lost during `vad_segments` on a
    /// Metal worker fails that job and leaves the GPU in use (DD-7's rule for a CPU job).
    #[tokio::test(start_paused = true)]
    async fn a_detector_job_is_a_cpu_job_even_on_a_gpu_worker() {
        let (supervisor, log) = start(Script {
            crash_on_vad: true,
            job_delay: Duration::from_millis(100),
            ..Script::default()
        });
        supervisor.prepare(model("aa", true)).await.expect("ready");
        let result = supervisor
            .vad_segments(model("aa", true), audio(3.0), VadOptions::dd48())
            .await;
        assert!(
            matches!(
                result,
                Err(SupervisorError::WorkerExited { code: Some(134) })
            ),
            "{result:?}"
        );
        assert!(!supervisor.status().force_cpu);
        let text = supervisor
            .transcribe(model("aa", true), 1, audio(2.0), params())
            .await
            .expect("a fresh worker answers")
            .text;
        assert!(text.ends_with("on metal"), "{text}");
        assert_eq!(loads(&log), ["load aa gpu=true", "load aa gpu=true"]);
        assert_eq!(spawns(&log), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_detector_job_on_a_gpu_worker_gets_the_cpu_deadline() {
        let (supervisor, log) = start(Script {
            hang_jobs: true,
            ..Script::default()
        });
        supervisor.prepare(model("aa", true)).await.expect("ready");
        let started = Instant::now();
        let result = supervisor
            .vad_segments(model("aa", true), audio(1.0), VadOptions::dd48())
            .await;
        assert!(
            matches!(
                result,
                Err(SupervisorError::Deadline {
                    phase: "vad_segments"
                })
            ),
            "{result:?}"
        );
        let took = secs(started);
        assert!((120.0..121.0).contains(&took), "{took}");
        assert!(!supervisor.status().force_cpu);
        assert_eq!(spawns(&log), 1, "no retry");
    }

    /// The Metal warm up runs inside `load`, so a GPU fault there gets DD-7's one CPU
    /// retry, which then holds until the idle unload.
    #[tokio::test(start_paused = true)]
    async fn a_crash_while_loading_on_the_gpu_loads_once_on_the_cpu() {
        let (supervisor, log) = start(Script {
            crash_on_gpu_load: true,
            ..Script::default()
        });
        let loaded = supervisor
            .prepare(model("aa", true))
            .await
            .expect("the CPU load answers");
        assert_eq!(loaded.backend, "cpu");
        assert_eq!(spawns(&log), 2);
        assert_eq!(loads(&log), ["load aa gpu=true", "load aa gpu=false"]);
        let status = supervisor.status();
        assert!(status.force_cpu);
        assert_eq!(status.state, WorkerState::Ready);
        assert_eq!(status.last_error, None);
        // It stays on the CPU, in the same worker.
        let text = supervisor
            .transcribe(model("aa", true), 1, audio(2.0), params())
            .await
            .expect("a result")
            .text;
        assert_eq!(text, "job 1 on cpu");
        assert_eq!(spawns(&log), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_gpu_load_that_misses_its_deadline_loads_once_on_the_cpu() {
        let (supervisor, log) = start(Script {
            hang_on_gpu_load: true,
            ..Script::default()
        });
        let started = Instant::now();
        let loaded = supervisor
            .prepare(model("aa", true))
            .await
            .expect("the CPU load answers");
        assert_eq!(loaded.backend, "cpu");
        assert!((180.0..181.0).contains(&secs(started)), "{}", secs(started));
        assert_eq!(
            events(&log),
            ["load aa gpu=true", "killed", "load aa gpu=false"]
        );
        assert!(supervisor.status().force_cpu);
    }

    #[tokio::test(start_paused = true)]
    async fn a_gpu_load_error_loads_once_on_the_cpu() {
        let (supervisor, log) = start(Script {
            gpu_load_error: true,
            ..Script::default()
        });
        let loaded = supervisor
            .prepare(model("aa", true))
            .await
            .expect("the CPU load answers");
        assert_eq!(loaded.backend, "cpu");
        assert_eq!(
            events(&log),
            ["load aa gpu=true", "killed", "load aa gpu=false"]
        );
        assert!(supervisor.status().force_cpu);
    }

    /// A file that fails on the CPU too was never the GPU's fault: the error stands, and
    /// the next request asks for the GPU again.
    #[tokio::test(start_paused = true)]
    async fn a_load_that_fails_on_both_backends_fails_and_keeps_the_gpu() {
        let (supervisor, log) = start(Script {
            load_error: true,
            ..Script::default()
        });
        let result = supervisor.prepare(model("aa", true)).await;
        assert!(
            matches!(result, Err(SupervisorError::ModelLoadFailed(_))),
            "{result:?}"
        );
        let status = supervisor.status();
        assert!(!status.force_cpu);
        assert_eq!(status.state, WorkerState::Error);
        assert_eq!(spawns(&log), 2);
        let _ = supervisor.prepare(model("aa", true)).await;
        assert_eq!(
            loads(&log),
            [
                "load aa gpu=true",
                "load aa gpu=false",
                "load aa gpu=true",
                "load aa gpu=false"
            ]
        );

        let (supervisor, log) = start(Script {
            crash_on_gpu_load: true,
            load_error: true,
            ..Script::default()
        });
        let result = supervisor.prepare(model("aa", true)).await;
        assert!(
            matches!(result, Err(SupervisorError::ModelLoadFailed(_))),
            "{result:?}"
        );
        assert!(!supervisor.status().force_cpu);
        assert_eq!(spawns(&log), 2);
    }

    /// The 32 MiB cap holds on the supervisor's side too: the head alone decides, the
    /// worker is killed and the job gets a protocol error.
    #[tokio::test(start_paused = true)]
    async fn an_oversized_frame_from_the_worker_kills_it() {
        let (supervisor, log) = start(Script {
            oversized_frame: true,
            ..Script::default()
        });
        supervisor.prepare(model("aa", false)).await.expect("ready");
        let result = supervisor
            .transcribe(model("aa", false), 1, audio(2.0), params())
            .await;
        let Err(SupervisorError::Protocol(message)) = result else {
            panic!("{result:?}")
        };
        assert!(message.contains("over the limit"), "{message}");
        assert_eq!(events(&log).last().map(String::as_str), Some("killed"));
        assert_eq!(supervisor.worker_pid(), None);
        assert_eq!(supervisor.status().state, WorkerState::Stopped);
    }

    /// A worker whose own CPUID view lacks a feature (an x86 build under Rosetta that
    /// the environment did not reach) is killed, and the supervisor spawns nothing more.
    #[tokio::test(start_paused = true)]
    async fn a_hello_without_the_needed_features_is_unsupported_for_good() {
        let (supervisor, log) = start_on(
            Script {
                hello_cpu: Some(CpuFlags::default()),
                ..Script::default()
            },
            SupervisorConfig::default(),
            cpu::Platform::X86,
        );
        let result = supervisor.prepare(model("aa", false)).await;
        let Err(SupervisorError::UnsupportedCpu { missing }) = result else {
            panic!("{result:?}")
        };
        assert_eq!(missing, ["avx2", "fma", "f16c", "bmi2"]);
        let status = supervisor.status();
        assert_eq!(status.state, WorkerState::Unsupported);
        assert_eq!(status.missing_cpu_features, missing);
        assert_eq!(events(&log), ["killed"]);
        let again = supervisor.prepare(model("aa", false)).await;
        assert!(matches!(again, Err(SupervisorError::UnsupportedCpu { .. })));
        assert_eq!(spawns(&log), 1);
    }

    /// A worker that ignores `shutdown` is killed 5 s after the idle unload sent it.
    #[tokio::test(start_paused = true)]
    async fn a_worker_that_ignores_shutdown_is_killed_after_5_s() {
        let (supervisor, log) = start(Script {
            ignore_shutdown: true,
            ..Script::default()
        });
        supervisor.prepare(model("aa", false)).await.expect("ready");
        tokio::time::sleep(Duration::from_secs(301)).await;
        assert_eq!(events(&log), ["load aa gpu=false", "shutdown"]);
        assert!(supervisor.worker_pid().is_some(), "still in its grace");
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!events(&log).contains(&"killed".to_string()));
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            events(&log),
            ["load aa gpu=false", "shutdown", "killed"],
            "killed at 305 s"
        );
        assert_eq!(supervisor.worker_pid(), None);
        assert!(!supervisor.status().force_cpu);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_stream_aborts_its_running_segment() {
        let (supervisor, log) = start(Script {
            job_delay: Duration::from_secs(10),
            ..Script::default()
        });
        let spec = model("aa", false);
        supervisor.prepare(Arc::clone(&spec)).await.expect("ready");
        let (running, answer) =
            supervisor.submit_transcribe(Arc::clone(&spec), 7, audio(2.0), params());
        let (_, other) = supervisor.submit_transcribe(Arc::clone(&spec), 8, audio(2.0), params());
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(supervisor.status().busy);
        let started = Instant::now();
        supervisor.cancel_stream(7);
        assert!(matches!(answer.await, Ok(Err(SupervisorError::Cancelled))));
        assert!(
            secs(started) < 1.0,
            "the worker answered the cancel at once"
        );
        assert!(events(&log).contains(&format!("cancel {}", running.0)));
        // The other stream's segment runs next.
        other.await.expect("answered").expect("a result");
    }
}
