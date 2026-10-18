//! Managed `llama-server` sidecar process for the Llama Server provider.
//!
//! Biorouter bundles a pinned llama.cpp `llama-server` binary (build
//! [`LLAMA_SERVER_BUILD`]) next to its own binaries. This module owns the
//! lifecycle of that process: locating the binary, spawning it with the
//! requested model from whatever is already on disk (see [`launch_source_for`]:
//! the catalog's own Hugging Face file in the cache first, then a GGUF in
//! Ollama's model store, and only then llama.cpp's `-hf` downloader), waiting
//! for readiness via `GET /health`, exposing a status snapshot for the
//! GUI/CLI, and restarting when the requested model changes or the process
//! dies.
//!
//! The process is a singleton per Biorouter process ([`global`]): llama-server
//! loads one model at a time, mirroring how Ollama schedules a model per
//! request stream. Switching models restarts the sidecar, and a request holds a
//! [`SidecarLease`] for as long as the server is generating for it, so a switch
//! waits for it instead of answering it with a different model. The lease also
//! holds one of the server's request slots, so a request the server has no
//! slot for waits here, in-process, rather than on a silent socket (see
//! [`SidecarLease`]).
//!
//! The server is loopback-only AND authenticated: every launch gets a fresh
//! random API key (written 0600 to the run dir and passed with
//! `--api-key-file`), because the loopback bind alone let any web page the user
//! opened drive the Private-tier model — llama-server reflects any `Origin` in
//! CORS with credentials allowed, and its `/props` leaks the user's home path.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use utoipa::ToSchema;

fn system_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    biorouter_mcp::developer::shell::strip_daemon_private_env_std(&mut command);
    biorouter_mcp::developer::shell::no_console_window_std(&mut command);
    command
}

/// The llama.cpp release this Biorouter version is pinned to. Keep in sync
/// with `ui/desktop/scripts/fetch-llama-server.js`. b11429 (2026-10-05) is the
/// first build with every fix the two catalog models need (Qwen3 tool parser,
/// `reasoning_effort` in templates, the Gated-DeltaNet normalization fix, the
/// Gemma 4 required-tool grammar fix, the n-gram draft fix at temperature > 0,
/// and `-hf` skipping `dflash-`/`eagle3-` draft files).
pub const LLAMA_SERVER_BUILD: &str = "b11429";
/// Default port the managed sidecar listens on (loopback only).
pub const LLAMACPP_DEFAULT_PORT: u16 = 11543;
/// The smallest context window the sidecar starts on its own, and the floor the
/// out-of-memory step-down stops at. Biorouter's instructions plus its tool
/// schemas alone measure ~31k tokens with the built-in extensions (Qwen3.8
/// tokenizer, 133 KB request) and ~37-40k with a few more, and compaction can
/// only summarise messages — it cannot shrink either. A 32k window (the old
/// small tier, and the old step-down floor) therefore failed the very first
/// request of every chat with `exceed_context_size_error`.
pub const LLAMACPP_AGENT_CONTEXT_SIZE: usize = 65_536;
/// The large automatic tier.
pub const LLAMACPP_LARGE_CONTEXT_SIZE: usize = 131_072;
/// GPU-addressable memory at which the large tier is chosen. Both catalog
/// models need ~21-22 GiB at 64k and ~24-25 GiB at 128k with q8_0 KV, so 48 GiB
/// leaves the OS, the prompt cache and the context checkpoints their room.
pub const LLAMACPP_LARGE_CONTEXT_MEMORY_GIB: u64 = 48;
/// Default `--parallel`. See `build_args`.
pub const LLAMACPP_DEFAULT_PARALLEL: usize = 1;

/// Set once llama-server has rejected an optional flag, so the rest of the
/// process stops emitting it. See `retry_without_optional_flags`.
static OPTIONAL_FLAGS_DISABLED: AtomicBool = AtomicBool::new(false);

/// Default `--spec-type`. See the rationale and the benchmark table in `build_args`.
pub const DEFAULT_SPEC_TYPE: &str = "ngram-simple";
const LOG_TAIL_LINES: usize = 60;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);
const STATUS_PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// Lifecycle state of the managed llama-server process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SidecarState {
    /// No llama-server binary could be located.
    NoBinary,
    /// Binary present but no process running.
    Stopped,
    /// Process running but not serving yet (downloading or loading a model).
    Starting,
    /// `GET /health` returns 200 — ready for completions.
    Ready,
    /// Process exited unexpectedly.
    Error,
}

/// Snapshot of the sidecar consumed by the GUI onboarding card, settings and
/// `biorouter doctor`.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SidecarStatus {
    pub state: SidecarState,
    /// Friendly model name (catalog name or raw HF spec) currently requested.
    pub model: Option<String>,
    /// Hugging Face `repo:quant` spec backing `model`.
    pub hf_spec: Option<String>,
    /// Ollama model name checked first, when this is a catalog model.
    #[serde(default)]
    pub ollama_name: Option<String>,
    /// Local model blob path used for the running process, if an Ollama model
    /// store hit was available.
    #[serde(default)]
    pub model_path: Option<String>,
    /// `ollama` when launched from an Ollama model blob, otherwise
    /// `huggingface`.
    #[serde(default)]
    pub model_source: Option<String>,
    pub port: Option<u16>,
    pub binary_path: Option<String>,
    /// Pinned llama.cpp build.
    pub build: String,
    /// Most recent log line (download progress, load stage) or error tail.
    pub detail: Option<String>,
    /// Context window the running model actually exposes (read live from the
    /// server), or `None` until it is ready. Tracks the model, not a preset.
    #[serde(default)]
    pub context_size: Option<usize>,
    /// True only after Biorouter has received a non-empty completion from this
    /// model in this process.
    #[serde(default)]
    pub warmed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelCacheStatus {
    Downloaded,
    Partial,
    NotDownloaded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSource {
    pub hf_spec: String,
    /// The Ollama tag a user would pull (shown in the UI, and probed first).
    pub ollama_name: Option<String>,
    /// Further Ollama tags that hold the same model as a plain GGUF. The bare
    /// tags (`qwen3.8:27b`, `gemma4:31b`) are manifest lists whose Apple Silicon
    /// child is MLX safetensors llama-server cannot load, so a Mac user who
    /// pulled the GGUF-only tag (`qwen3.8:27b-q4_K_M`) has a reusable file the
    /// bare tag never points at.
    pub ollama_gguf_names: Vec<String>,
    /// The exact file the catalog pins, when known.
    pub expected_file: Option<ExpectedGguf>,
    /// Default `--spec-type` for this model (`None` = pass no flag).
    /// `LLAMACPP_SPEC_TYPE` overrides it.
    pub spec_type: Option<String>,
}

/// The GGUF a catalog entry pins: `-hf` cannot pin a revision, so the size and
/// digest are what tell a re-uploaded file from the one the catalog was tested
/// with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedGguf {
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl ModelSource {
    pub fn huggingface(hf_spec: impl Into<String>) -> Self {
        Self {
            hf_spec: hf_spec.into(),
            ollama_name: None,
            ollama_gguf_names: Vec::new(),
            expected_file: None,
            spec_type: Some(DEFAULT_SPEC_TYPE.to_string()),
        }
    }

    pub fn ollama(ollama_name: impl Into<String>, hf_spec: impl Into<String>) -> Self {
        Self {
            ollama_name: Some(ollama_name.into()),
            ..Self::huggingface(hf_spec)
        }
    }

    pub fn with_ollama_gguf_names<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.ollama_gguf_names = names.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_expected_file(mut self, expected: ExpectedGguf) -> Self {
        self.expected_file = Some(expected);
        self
    }

    pub fn with_spec_type(mut self, spec_type: Option<&str>) -> Self {
        self.spec_type = spec_type.map(str::to_string);
        self
    }

    /// Every Ollama tag worth probing for a reusable GGUF, primary first.
    fn ollama_candidates(&self) -> impl Iterator<Item = &str> {
        self.ollama_name
            .iter()
            .chain(self.ollama_gguf_names.iter())
            .map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LaunchSource {
    /// A GGUF blob from Ollama's own store, loaded with `-m`.
    OllamaBlob(PathBuf),
    /// The Hugging Face fallback, already in the cache, loaded with `-m` so the
    /// launch needs no network and cannot silently re-download when the
    /// repository's `main` moves (ggml-org re-uploaded its Qwen3.8 GGUFs under
    /// the same names on 2026-09-19).
    CachedFile(PathBuf),
    /// Not cached anywhere yet: let llama.cpp's `-hf` downloader fetch it.
    HuggingFace(String),
}

impl LaunchSource {
    fn model_path(&self) -> Option<String> {
        match self {
            Self::OllamaBlob(path) | Self::CachedFile(path) => Some(path.display().to_string()),
            Self::HuggingFace(_) => None,
        }
    }

    fn source_name(&self) -> &'static str {
        match self {
            Self::OllamaBlob(_) => "ollama",
            Self::CachedFile(_) | Self::HuggingFace(_) => "huggingface",
        }
    }
}

struct Inner {
    child: Option<Child>,
    log_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Port the current/last process was started on.
    port: Option<u16>,
    model: Option<String>,
    model_source: Option<ModelSource>,
    launch_source: Option<LaunchSource>,
    log_tail: Arc<StdMutex<VecDeque<String>>>,
    last_error: Option<String>,
    /// Live context window of the running model, filled once ready.
    context_size: Option<usize>,
    /// Context window requested at process launch. This can differ from the
    /// memory-tiered default if automatic startup falls back after an OOM.
    requested_context_size: Option<usize>,
    automatic_context_size: bool,
    partial_download_restart_attempted: bool,
    warmed_model: Option<String>,
    /// True only while `wait_ready` has seen the child die and has not yet
    /// finished deciding whether one of its three self-heal paths applies.
    ///
    /// Without this, `status()` reports `SidecarState::Error` for the ~0.5-1.3 s
    /// window between the death and the respawn — and the GUI's 1500 ms status
    /// poller kills the whole warm-up on sight, so the recovery that was about
    /// to happen never runs. The user sees "failed to install model" on a
    /// startup that would have succeeded at a lower context or from the Hugging
    /// Face fallback. See `status()`.
    restarting: bool,
    /// One permit per request slot of the running server (its `--parallel`).
    /// Replaced on every spawn, which only happens under the switch lock, so
    /// no lease of the previous server can still be holding one of these.
    request_slots: Arc<tokio::sync::Semaphore>,
}

/// Singleton manager for the llama-server sidecar.
pub struct LlamaSidecar {
    inner: Mutex<Inner>,
    startup: Mutex<()>,
    /// Held for READ by every request for as long as it talks to the server
    /// ([`SidecarLease`]), and for WRITE by any (re)spawn in [`Self::ensure`].
    ///
    /// llama-server serves its one loaded model whatever a request's `model`
    /// field says. Without this, a second chat on the other catalog model (or the
    /// `/llamacpp/ensure` route) could kill and respawn the server between a
    /// turn's `ensure` and its POST, and the turn was silently answered by a
    /// different model than the session, the user or a Crew grant had chosen —
    /// or cut off mid-stream. tokio's `RwLock` is write-preferring, so a waiting
    /// switch is not starved by a stream of new requests.
    switch: Arc<tokio::sync::RwLock<()>>,
}

/// Proof that the sidecar is serving one model, held for as long as the server
/// is generating for a request (for a streamed reply, until generation ends —
/// see `GenerationLease` in the provider). A model switch waits for every lease
/// on the old model to drop before it restarts the server.
///
/// A lease also holds one of the server's request slots. With `--parallel 1` a
/// second request used to be queued INSIDE llama-server, which sends nothing —
/// not even response headers — until a slot starts the task. reqwest's read
/// timeout runs from the moment a request is sent and is not reset before the
/// headers arrive, so a request queued behind a generation longer than
/// `LLAMACPP_TIMEOUT` (600 s; a cold ~31-40k-token prefill alone takes that
/// long at the 32 GiB minimum) timed out on a perfectly healthy server and was
/// retried into the same queue. Waiting for the slot here instead costs no
/// socket and no timer: the request is only sent once the server will start it
/// at once, and `return_progress` then keeps the stream talking.
///
/// The slot is taken AFTER the read lock, never before: a waiting request then
/// holds only a read lock, and every slot holder already has its read lock and
/// waits on nothing but the server, so a queued model switch cannot close a
/// cycle with them.
pub struct SidecarLease {
    _guard: tokio::sync::OwnedRwLockReadGuard<()>,
    _slot: tokio::sync::OwnedSemaphorePermit,
    port: u16,
}

impl SidecarLease {
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Debug for SidecarLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarLease")
            .field("port", &self.port)
            .finish()
    }
}

/// How many times [`LlamaSidecar::lease`] re-ensures its model when another
/// caller switched the server between the readiness check and the lease.
const LEASE_ATTEMPTS: usize = 3;

enum StartupRecovery {
    Obsolete,
    Retried,
    Failed(String),
}

static SIDECAR: OnceLock<LlamaSidecar> = OnceLock::new();

/// The API key the running sidecar was launched with. A plain mutex rather than
/// a field behind `inner`, because the provider reads it inside every request's
/// auth-header hook and must never queue behind a status probe holding `inner`.
static SIDECAR_API_KEY: StdMutex<Option<String>> = StdMutex::new(None);

/// The bearer key the managed llama-server currently accepts, if one is running.
pub fn current_api_key() -> Option<String> {
    SIDECAR_API_KEY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn set_current_api_key(key: Option<String>) {
    *SIDECAR_API_KEY.lock().unwrap_or_else(|e| e.into_inner()) = key;
}

/// The process-wide sidecar manager.
pub fn global() -> &'static LlamaSidecar {
    SIDECAR.get_or_init(|| LlamaSidecar {
        inner: Mutex::new(Inner {
            child: None,
            log_tasks: Vec::new(),
            port: None,
            model: None,
            model_source: None,
            launch_source: None,
            log_tail: Arc::new(StdMutex::new(VecDeque::new())),
            last_error: None,
            context_size: None,
            requested_context_size: None,
            automatic_context_size: false,
            partial_download_restart_attempted: false,
            warmed_model: None,
            restarting: false,
            request_slots: Arc::new(tokio::sync::Semaphore::new(LLAMACPP_DEFAULT_PARALLEL)),
        }),
        startup: Mutex::new(()),
        switch: Arc::new(tokio::sync::RwLock::new(())),
    })
}

/// Locate a llama-server binary, in priority order:
/// 1. `BIOROUTER_LLAMACPP_BIN` (config param or environment variable)
/// 2. `<dir of current exe>/llamacpp/llama-server` (bundled by the desktop app)
/// 3. `<dir of current exe>/llama-server`
/// 4. `llama-server` on PATH
pub fn find_binary() -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    let override_path = std::env::var("BIOROUTER_LLAMACPP_BIN").ok().or_else(|| {
        crate::config::Config::global()
            .get_param::<String>("BIOROUTER_LLAMACPP_BIN")
            .ok()
    });
    if let Some(p) = override_path {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [
                dir.join("llamacpp").join(exe_name),
                dir.join(exe_name),
                // Dev builds run from target/{debug,release}/ inside the repo;
                // the fetched binary lives in the desktop app's bin dir.
                dir.join("../../ui/desktop/src/bin/llamacpp").join(exe_name),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    which_on_path(exe_name)
}

fn which_on_path(exe_name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let candidate = dir.join(exe_name);
        candidate.is_file().then_some(candidate)
    })
}

/// Directory used as `LLAMA_CACHE`. This follows Ollama's model-store
/// convention (`OLLAMA_MODELS`, then `~/.ollama/models`) so local model data
/// lives where Ollama users expect it.
pub fn model_cache_dir() -> PathBuf {
    std::env::var_os("OLLAMA_MODELS")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("biorouter requires a home dir")
                .join(".ollama")
                .join("models")
        })
}

pub fn model_cache_status(hf_spec: &str) -> ModelCacheStatus {
    model_cache_status_in_dir(&model_cache_dir(), hf_spec)
}

pub fn delete_model_cache(hf_spec: &str) -> Result<bool> {
    delete_model_cache_in_dir(&model_cache_dir(), hf_spec)
}

pub fn model_source_cache_status(source: &ModelSource) -> ModelCacheStatus {
    let dir = model_cache_dir();
    model_source_cache_status_in_dir(&dir, source)
}

/// Whether `source` can start without downloading anything.
///
/// An Ollama tag counts only when it resolves to a GGUF llama-server can load.
/// On Apple Silicon `qwen3.8:27b` and `gemma4:31b` pull as MLX safetensors (a
/// manifest with ~1,200 `image.tensor` layers and no `image.model` layer), and
/// this used to report that as `Partial` and return before the Hugging Face
/// cache was even consulted — so a user who had pulled the model in Ollama was
/// told the download was incomplete ("Resume download & run"), and even a fully
/// cached Hugging Face fallback was masked behind it. An MLX pull is simply not
/// reusable here; it is not a half-finished GGUF.
fn model_source_cache_status_in_dir(cache_dir: &Path, source: &ModelSource) -> ModelCacheStatus {
    let mut ollama_partial = false;
    for name in source.ollama_candidates() {
        match ollama_model_cache_status_in_dir(cache_dir, name) {
            ModelCacheStatus::Downloaded => return ModelCacheStatus::Downloaded,
            ModelCacheStatus::Partial => ollama_partial = true,
            ModelCacheStatus::NotDownloaded => {}
        }
    }
    match model_cache_status_in_dir(cache_dir, &source.hf_spec) {
        ModelCacheStatus::NotDownloaded if ollama_partial => ModelCacheStatus::Partial,
        status => status,
    }
}

/// The reusable GGUF in Ollama's store for `source`, if any of its tags has one.
pub fn model_source_path(source: &ModelSource) -> Option<PathBuf> {
    let dir = model_cache_dir();
    source
        .ollama_candidates()
        .find_map(|name| ollama_model_blob_path_in_dir(&dir, name))
}

fn model_cache_status_in_dir(cache_dir: &Path, hf_spec: &str) -> ModelCacheStatus {
    let Some(repo_dir) = model_cache_repo_dir(cache_dir, hf_spec) else {
        return ModelCacheStatus::NotDownloaded;
    };

    if !repo_dir.is_dir() {
        return ModelCacheStatus::NotDownloaded;
    }
    if hf_cached_model_file_in_dir(cache_dir, hf_spec).is_some() {
        return ModelCacheStatus::Downloaded;
    }

    let mut has_partial = false;
    visit_cache_files(&repo_dir, &mut |path| {
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.ends_with(".downloadInProgress"))
        {
            has_partial = true;
        }
    });
    if has_partial {
        ModelCacheStatus::Partial
    } else {
        ModelCacheStatus::NotDownloaded
    }
}

/// Files llama.cpp's `-hf` picker never treats as the model: projectors, an
/// importance matrix, and the speculative-decoding drafters ggml-org ships next
/// to the weights (`mtp-`, `dflash-`, `eagle3-`, `dspark-`). Mirrors b11429's
/// `gguf_filename_is_model`; b9611 lacked the drafter names and resolved
/// `ggml-org/Qwen3.8-27B-GGUF:Q4_0` to `dflash-Qwen3.8-27B-Q4_0.gguf`.
fn is_model_gguf_name(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(".gguf")
        && !["mmproj", "imatrix", "mtp-", "dflash-", "eagle3-", "dspark-"]
            .iter()
            .any(|marker| lower.contains(marker))
}

/// Whether `file_name` is the `quant` file in the sense of llama.cpp's
/// `TAG[.-]` match (case-insensitive): `Qwen3.8-27B-Q4_K_M.gguf` and
/// `gemma-4-31B_q4_0-it.gguf` match `Q4_K_M` and `Q4_0` respectively, and
/// `...-Q4_K_M-00001-of-00002.gguf` matches as the first part of a split.
fn gguf_name_matches_quant(file_name: &str, quant: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let quant = quant.to_ascii_lowercase();
    lower
        .match_indices(&quant)
        .any(|(at, _)| matches!(lower.as_bytes().get(at + quant.len()), Some(b'.' | b'-')))
}

/// The cached model file for `hf_spec` (a `snapshots/<rev>/<file>` entry of the
/// Hugging Face cache layout llama.cpp writes under `LLAMA_CACHE`), chosen the
/// way llama.cpp's `-hf` would choose it: the quant's file, or with no quant
/// `Q4_K_M`, then `Q8_0`, then the first model file.
pub fn hf_cached_model_file(hf_spec: &str) -> Option<PathBuf> {
    hf_cached_model_file_in_dir(&model_cache_dir(), hf_spec)
}

fn hf_cached_model_file_in_dir(cache_dir: &Path, hf_spec: &str) -> Option<PathBuf> {
    let snapshots = model_cache_repo_dir(cache_dir, hf_spec)?.join("snapshots");
    let mut files = Vec::new();
    visit_cache_files(&snapshots, &mut |path| {
        let complete = path.is_file()
            && path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(is_model_gguf_name);
        if complete {
            files.push(path.to_path_buf());
        }
    });
    files.sort();
    let name_of = |p: &PathBuf| {
        p.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string()
    };
    let pick = |quant: &str| {
        files
            .iter()
            .find(|p| gguf_name_matches_quant(&name_of(p), quant))
            .cloned()
    };
    match hf_spec.split_once(':').map(|(_, quant)| quant) {
        Some(quant) => pick(quant),
        None => pick("Q4_K_M")
            .or_else(|| pick("Q8_0"))
            .or_else(|| files.first().cloned()),
    }
}

fn delete_model_cache_in_dir(cache_dir: &Path, hf_spec: &str) -> Result<bool> {
    let Some(repo_dir) = model_cache_repo_dir(cache_dir, hf_spec) else {
        return Ok(false);
    };
    if !repo_dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&repo_dir)?;
    Ok(true)
}

fn model_cache_repo_dir(cache_dir: &Path, hf_spec: &str) -> Option<PathBuf> {
    let repo = hf_spec.split_once(':').map_or(hf_spec, |(repo, _)| repo);
    let (owner, name) = repo.split_once('/')?;
    Some(cache_dir.join(format!("models--{owner}--{name}")))
}

#[derive(Debug, PartialEq, Eq)]
struct OllamaName {
    host: String,
    namespace: String,
    model: String,
    tag: String,
}

fn parse_ollama_name(name: &str) -> Option<OllamaName> {
    let (without_tag, tag) = name.rsplit_once(':').unwrap_or((name, "latest"));
    if without_tag.is_empty() || tag.is_empty() {
        return None;
    }
    let parts: Vec<_> = without_tag.split('/').collect();
    let (host, namespace, model) = match parts.as_slice() {
        [model] => ("registry.ollama.ai", "library", *model),
        [namespace, model] => ("registry.ollama.ai", *namespace, *model),
        [host, namespace, model] => (*host, *namespace, *model),
        _ => return None,
    };
    if [host, namespace, model, tag]
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains('\\'))
    {
        return None;
    }
    Some(OllamaName {
        host: host.to_string(),
        namespace: namespace.to_string(),
        model: model.to_string(),
        tag: tag.to_string(),
    })
}

/// Where Ollama may have written `name`'s manifest. Ollama 0.40 keeps the
/// legacy `manifests/registry.ollama.ai/...` file and adds
/// `manifests-v2/ollama.com/...`, which for a multi-runner tag is a symlink to
/// the manifest LIST blob.
fn ollama_manifest_paths(cache_dir: &Path, name: &str) -> Vec<PathBuf> {
    let Some(name) = parse_ollama_name(name) else {
        return Vec::new();
    };
    let mut hosts = vec![name.host.clone()];
    if name.host == "registry.ollama.ai" {
        hosts.push("ollama.com".to_string());
    }
    let mut paths = vec![cache_dir
        .join("manifests")
        .join(&name.host)
        .join(&name.namespace)
        .join(&name.model)
        .join(&name.tag)];
    for host in hosts {
        paths.push(
            cache_dir
                .join("manifests-v2")
                .join(host)
                .join(&name.namespace)
                .join(&name.model)
                .join(&name.tag),
        );
    }
    paths
}

const OLLAMA_MODEL_LAYER: &str = "application/vnd.ollama.image.model";
const OLLAMA_MANIFEST_LIST: &str = "application/vnd.ollama.manifest.list.v2+json";

fn ollama_blob_path(cache_dir: &Path, digest: &str) -> Option<PathBuf> {
    // Digests come from a file on disk; refuse anything that could leave the
    // blob directory before it becomes a path.
    let digest_path = digest.replace(':', "-");
    if digest_path.is_empty()
        || !digest_path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    Some(cache_dir.join("blobs").join(digest_path))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The digest of a manifest list's llama.cpp/GGUF child, if the list has one.
fn gguf_child_digest(list: &serde_json::Value) -> Option<String> {
    list.get("manifests")?
        .as_array()?
        .iter()
        .find(|child| {
            child.get("runner").and_then(|v| v.as_str()) == Some("llamacpp")
                || child.get("format").and_then(|v| v.as_str()) == Some("gguf")
        })?
        .get("digest")?
        .as_str()
        .map(str::to_string)
}

/// The image manifest `name` resolves to locally, following a manifest list to
/// its GGUF child when that child was pulled. `None` when there is no manifest.
fn ollama_image_manifests(cache_dir: &Path, name: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for path in ollama_manifest_paths(cache_dir, name) {
        let Some(manifest) = read_json(&path) else {
            continue;
        };
        let is_list = manifest.get("mediaType").and_then(|v| v.as_str())
            == Some(OLLAMA_MANIFEST_LIST)
            || manifest.get("manifests").is_some();
        if is_list {
            if let Some(child) = gguf_child_digest(&manifest)
                .and_then(|digest| ollama_blob_path(cache_dir, &digest))
                .and_then(|path| read_json(&path))
            {
                out.push(child);
            }
            continue;
        }
        // A legacy manifest of an MLX pull carries the manifest list as one of
        // its layers; its GGUF child is reusable if it was ever pulled.
        if let Some(list_digest) = manifest
            .get("layers")
            .and_then(|l| l.as_array())
            .and_then(|layers| {
                layers.iter().find(|layer| {
                    layer.get("mediaType").and_then(|v| v.as_str()) == Some(OLLAMA_MANIFEST_LIST)
                })
            })
            .and_then(|layer| layer.get("digest"))
            .and_then(|d| d.as_str())
        {
            if let Some(child) = ollama_blob_path(cache_dir, list_digest)
                .and_then(|path| read_json(&path))
                .and_then(|list| gguf_child_digest(&list))
                .and_then(|digest| ollama_blob_path(cache_dir, &digest))
                .and_then(|path| read_json(&path))
            {
                out.push(child);
            }
        }
        out.push(manifest);
    }
    out
}

/// The largest `image.model` layer of a manifest: the weights. Draft
/// (`image.draft`) and projector layers have their own media types and are
/// never candidates.
fn model_layer_digest(manifest: &serde_json::Value) -> Option<String> {
    manifest
        .get("layers")?
        .as_array()?
        .iter()
        .filter(|layer| layer.get("mediaType").and_then(|v| v.as_str()) == Some(OLLAMA_MODEL_LAYER))
        .max_by_key(|layer| layer.get("size").and_then(|v| v.as_u64()).unwrap_or(0))?
        .get("digest")?
        .as_str()
        .map(str::to_string)
}

fn ollama_model_cache_status_in_dir(cache_dir: &Path, name: &str) -> ModelCacheStatus {
    let manifests = ollama_image_manifests(cache_dir, name);
    if manifests.is_empty() {
        return ModelCacheStatus::NotDownloaded;
    }
    if ollama_model_blob_path_in_dir(cache_dir, name).is_some() {
        return ModelCacheStatus::Downloaded;
    }
    // A GGUF manifest whose weights blob is missing or unreadable is a pull
    // still in flight (or an interrupted one). A manifest with no GGUF model
    // layer at all — an MLX pull — is not reusable, which is NOT partial.
    if manifests
        .iter()
        .any(|manifest| model_layer_digest(manifest).is_some())
    {
        ModelCacheStatus::Partial
    } else {
        ModelCacheStatus::NotDownloaded
    }
}

fn ollama_model_blob_path_in_dir(cache_dir: &Path, name: &str) -> Option<PathBuf> {
    ollama_image_manifests(cache_dir, name)
        .iter()
        .filter_map(model_layer_digest)
        .filter_map(|digest| ollama_blob_path(cache_dir, &digest))
        .find(|path| path.is_file() && is_gguf_file(path))
}

fn is_gguf_file(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    std::io::Read::read_exact(&mut file, &mut magic).is_ok() && magic == *b"GGUF"
}

fn visit_cache_files(dir: &Path, f: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_cache_files(&path, f);
        } else {
            f(&path);
        }
    }
}

fn configured_port() -> u16 {
    crate::config::Config::global()
        .get_param::<u16>("LLAMACPP_PORT")
        .unwrap_or(LLAMACPP_DEFAULT_PORT)
}

/// Total physical memory in GiB (best-effort; 0 when it can't be read). On
/// Apple Silicon this is unified memory, which doubles as the GPU/Metal budget,
/// so it's a good proxy for how much KV cache we can afford.
pub fn total_memory_gib() -> u64 {
    // `sys_info::mem_info().total` is in KiB.
    sys_info::mem_info()
        .map(|m| m.total / (1024 * 1024))
        .unwrap_or(0)
}

fn is_apple_silicon() -> bool {
    cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")
}

/// True when the llama-server this machine runs has no GPU backend, so every
/// layer runs on the CPU out of system RAM.
///
/// - **Intel Macs**: upstream builds the `macos-x64` archive with
///   `-DGGML_METAL=OFF`, and Ollama's x86_64 slice does not link Metal either.
///   This used to budget an Intel Mac by its discrete GPU's VRAM (from
///   `system_profiler`) — a budget nothing ever used — and the bug reporter of
///   the 4096-token truncation was on exactly such a machine.
/// - **Linux**: the bundled archive is the CPU build. A GPU only counts when the
///   user supplied their own binary (`BIOROUTER_LLAMACPP_BIN`, or one on PATH)
///   AND an NVIDIA GPU is visible.
pub fn is_cpu_only() -> bool {
    if cfg!(target_os = "macos") {
        return !is_apple_silicon();
    }
    if cfg!(target_os = "linux") {
        return detect_discrete_vram_gib().is_none() || bundled_binary_in_use();
    }
    false
}

/// Whether the binary [`find_binary`] resolves is the one shipped next to the
/// Biorouter executable (as opposed to a user-supplied build).
fn bundled_binary_in_use() -> bool {
    let (Some(binary), Ok(exe)) = (find_binary(), std::env::current_exe()) else {
        return false;
    };
    exe.parent()
        .is_some_and(|dir| binary.starts_with(dir.join("llamacpp")))
}

/// The memory budget relevant for local inference. On Apple Silicon this is
/// unified system memory; on a CPU-only machine it is system RAM (see
/// [`is_cpu_only`]); elsewhere it is discrete VRAM when we can detect it.
/// Unknown VRAM is reported as `None` so the UI can warn instead of pretending
/// system RAM is enough for a GPU build.
pub fn accelerator_memory_gib() -> Option<u64> {
    if is_apple_silicon() || is_cpu_only() {
        let total = total_memory_gib();
        return (total > 0).then_some(total);
    }
    detect_discrete_vram_gib()
}

/// `apple_unified`, `cpu_only`, `vram`, or `unknown_vram`.
pub fn accelerator_memory_kind() -> &'static str {
    if is_apple_silicon() {
        "apple_unified"
    } else if is_cpu_only() {
        "cpu_only"
    } else if accelerator_memory_gib().is_some() {
        "vram"
    } else {
        "unknown_vram"
    }
}

pub fn recommendation_memory_gib() -> u64 {
    accelerator_memory_gib().unwrap_or(0)
}

/// A discrete GPU's memory, in GiB, whatever build this machine's llama-server
/// is: `system_profiler` on a Mac, `Win32_VideoController` on Windows, and
/// `nvidia-smi` on Linux (an AMD or Intel GPU there reads as `None`). Cached
/// after the first call; the first may run one of those commands, so call it
/// off the async workers.
///
/// Exposed for the Ollama provider, which ships CUDA on Linux and so must not
/// be sized by [`accelerator_memory_gib`]'s answer there: that is about THIS
/// sidecar's CPU build, and reports system RAM.
pub fn discrete_vram_gib() -> Option<u64> {
    detect_discrete_vram_gib()
}

fn detect_discrete_vram_gib() -> Option<u64> {
    static VRAM_GIB: OnceLock<Option<u64>> = OnceLock::new();
    *VRAM_GIB.get_or_init(detect_discrete_vram_gib_uncached)
}

fn detect_discrete_vram_gib_uncached() -> Option<u64> {
    if cfg!(target_os = "macos") {
        detect_macos_vram_gib()
    } else if cfg!(target_os = "windows") {
        detect_windows_vram_gib()
    } else {
        detect_linux_vram_gib()
    }
}

fn detect_macos_vram_gib() -> Option<u64> {
    let output = system_command("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let mut best = None;
    collect_vram_from_json(&json, &mut best);
    best
}

fn collect_vram_from_json(value: &serde_json::Value, best: &mut Option<u64>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key.to_ascii_lowercase().contains("vram") {
                    if let Some(text) = value.as_str() {
                        if let Some(gib) = parse_memory_gib(text) {
                            *best = Some(best.map_or(gib, |current| current.max(gib)));
                        }
                    }
                }
                collect_vram_from_json(value, best);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_vram_from_json(item, best);
            }
        }
        _ => {}
    }
}

fn detect_windows_vram_gib() -> Option<u64> {
    let output = system_command("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_VideoController | ForEach-Object { $_.AdapterRAM }",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
        .map(bytes_to_gib)
        .max()
}

fn detect_linux_vram_gib() -> Option<u64> {
    let output = system_command("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .filter(|mib| *mib > 0)
        .map(|mib| mib.div_ceil(1024))
        .max()
}

fn bytes_to_gib(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024 * 1024)
}

fn parse_memory_gib(text: &str) -> Option<u64> {
    let normalized = text.to_ascii_lowercase().replace(',', "");
    let number = normalized
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|s| !s.is_empty())?
        .parse::<f64>()
        .ok()?;
    if normalized.contains("tb") || normalized.contains("tib") {
        Some((number * 1024.0).ceil() as u64)
    } else if normalized.contains("gb") || normalized.contains("gib") {
        Some(number.ceil() as u64)
    } else if normalized.contains("mb") || normalized.contains("mib") {
        Some((number / 1024.0).ceil() as u64)
    } else {
        None
    }
}

/// The default context window when the user hasn't pinned `LLAMACPP_CONTEXT_SIZE`.
///
/// A model's *trained* window (read later from `/props`) is 262k for both
/// catalog models, but the KV cache for it scales with the window, and
/// allocating tens of GB at startup is slow-to-impossible on a laptop. Two
/// tiers: 128k with at least [`LLAMACPP_LARGE_CONTEXT_MEMORY_GIB`] of
/// GPU-addressable memory (unified memory on Apple Silicon, detected VRAM on a
/// discrete GPU, system RAM on a CPU-only machine), and otherwise 64k — the
/// floor ([`LLAMACPP_AGENT_CONTEXT_SIZE`]), because Biorouter's instructions and
/// tools alone take ~31-40k tokens. Measured KV cost at q8_0: Qwen3.8 27B about
/// 34 KiB/token (2.1 GiB at 64k), Gemma 4 31B about 42 KiB/token plus ~0.6 GiB
/// of sliding-window cache with one slot. Users can still pin any explicit
/// `LLAMACPP_CONTEXT_SIZE`, and the real allocated window is always read back
/// from `/props` for accounting.
pub fn default_context_size() -> usize {
    default_context_size_for(recommendation_memory_gib())
}

fn default_context_size_for(memory_gib: u64) -> usize {
    if memory_gib >= LLAMACPP_LARGE_CONTEXT_MEMORY_GIB {
        LLAMACPP_LARGE_CONTEXT_SIZE
    } else {
        LLAMACPP_AGENT_CONTEXT_SIZE
    }
}

/// Value for `--spec-type`, or `None` to omit the flag entirely, for a model
/// whose own default is `model_default` (see the catalog). An explicit
/// `LLAMACPP_SPEC_TYPE` wins either way; `none` or an empty value turns it off.
pub fn configured_spec_type_for(model_default: Option<&str>) -> Option<String> {
    if OPTIONAL_FLAGS_DISABLED.load(Ordering::Relaxed) {
        return None;
    }
    let configured = crate::config::Config::global()
        .get_param::<String>("LLAMACPP_SPEC_TYPE")
        .ok()
        .map(|v| v.trim().to_string())
        .or_else(|| model_default.map(str::to_string))?;
    if configured.is_empty() || configured.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(configured)
}

/// Value for `--spec-type` for a model with no catalog default of its own (a
/// raw Hugging Face spec), which keeps [`DEFAULT_SPEC_TYPE`].
pub fn configured_spec_type() -> Option<String> {
    configured_spec_type_for(Some(DEFAULT_SPEC_TYPE))
}

fn explicit_context_size() -> Option<usize> {
    crate::config::Config::global()
        .get_param::<usize>("LLAMACPP_CONTEXT_SIZE")
        .ok()
        .filter(|&n| n > 0)
}

/// `LLAMACPP_ENABLE_THINKING`: whether a request that says nothing about
/// thinking thinks. Off unless set. The one reading of the key, for the
/// server's `--reasoning` default and the provider's Normal-effort turns alike,
/// which used to parse it in two places that could drift (a YAML `true`, or the
/// string `"true"` a settings form or environment variable stores).
pub fn thinking_enabled_by_default() -> bool {
    let config = crate::config::Config::global();
    config
        .get_param::<bool>("LLAMACPP_ENABLE_THINKING")
        .ok()
        .or_else(|| {
            config
                .get_param::<String>("LLAMACPP_ENABLE_THINKING")
                .ok()
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap_or(false)
}

/// `--parallel`: how many requests the server runs at once. Defaults to one.
///
/// llama-server's own default is 4 slots (`-np -1` = auto) sharing ONE unified
/// KV cache of `--ctx-size` tokens. Two concurrent agent requests — a subagent
/// next to its parent, the title request during the first turn, two open
/// chats — each carry Biorouter's ~31-40k-token bootstrap, so in a 64k window
/// they could not both fit and BOTH failed with HTTP 500 "Context size has been
/// exceeded" (measured on b9611; the 500 was then retried as transient, and the
/// retries collided again). With one slot the second request waits for the
/// first (in-process, on the lease's slot: see [`SidecarLease`]) and both
/// succeed; the window each request may use is then the whole `--ctx-size`.
/// `LLAMACPP_PARALLEL` overrides it for a user who sizes the window for more.
fn configured_parallel() -> usize {
    crate::config::Config::global()
        .get_param::<usize>("LLAMACPP_PARALLEL")
        .ok()
        .filter(|&n| n > 0)
        .unwrap_or(LLAMACPP_DEFAULT_PARALLEL)
}

/// The request slots a server launched with `args` really has: the LAST
/// `-np`/`--parallel` wins in llama.cpp, and an allowlisted
/// `LLAMACPP_EXTRA_ARGS` value comes after [`configured_parallel`]'s. Auto
/// (`-1`) or anything unreadable counts as one slot, which can only make
/// requests wait for each other here, never pile up silently in the server.
fn parallel_slots_in(args: &[String]) -> usize {
    args.windows(2)
        .rev()
        .find(|pair| matches!(pair[0].as_str(), "-np" | "--parallel"))
        .and_then(|pair| pair[1].parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

/// The `--ctx-size` passed to llama-server. An explicit positive
/// `LLAMACPP_CONTEXT_SIZE` pins the window (and caps memory); otherwise (unset
/// or `0` = "auto") we use a memory-tiered default ([`default_context_size`])
/// rather than the model's full native window, so large models don't hang
/// allocating a giant KV cache on startup. The size the server actually
/// allocates is read back from `/props` ([`live_context_size`]) and reported to
/// token accounting, so the gauge always matches reality. q8_0 KV-cache
/// quantization keeps the memory cost affordable.
pub fn configured_context_size() -> usize {
    explicit_context_size().unwrap_or_else(default_context_size)
}

/// The next automatic window to try after an out-of-memory start, or `None`
/// at the floor. There is deliberately no step below
/// [`LLAMACPP_AGENT_CONTEXT_SIZE`]: a smaller window cannot hold Biorouter's
/// instructions and tools, so every request would fail with
/// `exceed_context_size_error` — a server that started but cannot answer is
/// worse than a clear "not enough memory" (see [`startup_failure_message`]).
fn next_lower_auto_context_size(ctx_size: usize) -> Option<usize> {
    (ctx_size > LLAMACPP_AGENT_CONTEXT_SIZE).then_some(LLAMACPP_AGENT_CONTEXT_SIZE)
}

/// The error `wait_ready` reports once every self-heal path is exhausted.
/// Out of memory at the floor gets an actionable explanation instead of the
/// raw log tail, because the obvious user reaction ("lower the context") is
/// exactly what would make every request fail.
fn startup_failure_message(model: &str, ctx_size: Option<usize>, tail: &str) -> String {
    if looks_like_memory_failure(tail) {
        let window = ctx_size
            .map(|n| format!("a {n}-token context window"))
            .unwrap_or_else(|| "its context window".to_string());
        return format!(
            "There is not enough memory to load {model} with {window}. {LLAMACPP_AGENT_CONTEXT_SIZE} \
             tokens is the smallest window Biorouter starts a local model with, because its \
             instructions and tool definitions alone take about 31,000-40,000 tokens. Quit other \
             apps or local models that hold GPU memory and try again, or use an Ollama or cloud \
             provider on this machine.\n\nllama-server output:\n{tail}"
        );
    }
    if let Some(argument) = rejected_argument(tail).filter(|_| !rejects_an_optional_flag(tail)) {
        return format!(
            "llama-server refused the argument '{argument}' and did not start. If it comes from \
             LLAMACPP_EXTRA_ARGS, correct or remove it there.\n\nllama-server output:\n{tail}"
        );
    }
    format!("llama-server exited during startup:\n{tail}")
}

/// A cheap fingerprint of the log tail, used to tell "still working" from
/// "wedged". Combines the line count with the last line, so both a new line and
/// a rewritten progress line (llama.cpp's download percentage overwrites one
/// line) register as progress, while a genuinely stalled server produces the
/// same marker on every poll.
fn progress_marker(tail: &Arc<StdMutex<VecDeque<String>>>) -> String {
    let guard = tail.lock().unwrap_or_else(|e| e.into_inner());
    format!(
        "{}:{}",
        guard.len(),
        guard.back().map(String::as_str).unwrap_or("")
    )
}

/// True when llama-server rejected an argument rather than failing to run.
///
/// This is the safety net for every *accelerating* flag the sidecar adds (today
/// `--spec-type`): llama.cpp releases many times a day with no semver, and
/// CLAUDE.md's standing warning is that a pin bump can rename or drop a flag.
/// Without this, a dropped flag turns into "local models stopped working
/// entirely" — the process dies instantly at argv parse time, which looks
/// nothing like a flag problem. With it, the sidecar drops the optional flag and
/// respawns, so the worst case is losing the speedup, not losing inference.
fn looks_like_unsupported_argument(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    [
        "invalid argument",
        "unknown argument",
        "unrecognized argument",
        "error while handling argument",
        "invalid value for",
    ]
    .iter()
    .any(|needle| log.contains(needle))
}

/// The flags [`LlamaSidecar::retry_without_optional_flags`] can drop: the ones
/// `build_args` adds only to go faster. Everything else on the command line is
/// either required or the user's own `LLAMACPP_EXTRA_ARGS`.
const OPTIONAL_FLAGS: &[&str] = &["--spec-type"];

/// The argument llama-server says it rejected, in either of b11429's two
/// shapes: `error: invalid argument: --threads=8` for a flag it does not know,
/// and `error while handling argument "--spec-type": unknown speculative type:
/// x` for a value it does not take.
fn rejected_argument(log: &str) -> Option<String> {
    log.lines().rev().find_map(|line| {
        if let Some((_, rest)) = line.split_once("invalid argument: ") {
            return Some(rest.trim().to_string());
        }
        let (_, rest) = line.split_once("error while handling argument \"")?;
        rest.split_once('"').map(|(arg, _)| arg.to_string())
    })
}

/// True when an argument error is one dropping the optional flags can fix: the
/// rejected argument is one of [`OPTIONAL_FLAGS`], or the log does not say which
/// argument it was.
///
/// Any argument error used to switch the optional flags off, and that switch is
/// process-wide and never reset. So a typo in `LLAMACPP_EXTRA_ARGS` cost the
/// measured 2-4x speculative-decoding speedup until Biorouter was restarted,
/// even after the user fixed it — and the respawn without the optional flags
/// still carried the typo and died again anyway.
fn rejects_an_optional_flag(log: &str) -> bool {
    if !looks_like_unsupported_argument(log) {
        return false;
    }
    match rejected_argument(log) {
        None => true,
        Some(argument) => {
            let flag = argument
                .split_once('=')
                .map_or(argument.as_str(), |(f, _)| f);
            OPTIONAL_FLAGS.contains(&canonical_flag(flag).as_str())
        }
    }
}

fn looks_like_memory_failure(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    [
        "out of memory",
        "not enough memory",
        "failed to allocate",
        "cannot allocate",
        "unable to allocate",
        "memory allocation",
        // Metal, when the GPU working set is exhausted mid-request:
        // `error: Insufficient Memory (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)`.
        "insufficient memory",
        "outofmemory",
    ]
    .iter()
    .any(|needle| log.contains(needle))
}

/// The 500 body llama-server answers with once a decode failed on the GPU.
const COMPUTE_ERROR: &str = "Compute error";

/// True for an error message that carries llama-server's compute failure.
pub fn is_compute_error(message: &str) -> bool {
    message.contains(COMPUTE_ERROR)
}

/// What the user is told after [`LlamaSidecar::discard_failed_backend`], for a
/// server that was running with a `ctx_size`-token window.
///
/// A smaller window is only offered while there is one to offer. This used to
/// say "set a smaller LLAMACPP_CONTEXT_SIZE" with no floor, so a 32-47 GiB
/// machine already at the 64k floor that ran out of memory mid-request was
/// sent to 32k — where the server starts and then refuses every agent request
/// with `exceed_context_size_error`, the outcome [`startup_failure_message`]
/// exists to warn against.
fn backend_failure_message(model: &str, ctx_size: Option<usize>, tail: &str) -> String {
    if looks_like_memory_failure(tail) {
        let advice = match ctx_size {
            Some(window) if window > LLAMACPP_AGENT_CONTEXT_SIZE => format!(
                "Quit other apps or local models that hold GPU memory, or set \
                 LLAMACPP_CONTEXT_SIZE to {LLAMACPP_AGENT_CONTEXT_SIZE} (not lower: Biorouter's \
                 instructions and tool definitions alone take about 31,000-40,000 tokens)."
            ),
            _ => format!(
                "Quit other apps or local models that hold GPU memory. A smaller \
                 context window will not help: {LLAMACPP_AGENT_CONTEXT_SIZE} tokens is the \
                 smallest Biorouter runs a local model with, because its instructions and tool \
                 definitions alone take about 31,000-40,000 tokens. If this keeps happening, use \
                 an Ollama or cloud provider on this machine."
            ),
        };
        format!(
            "Llama Server ran out of GPU memory while running {model}. Biorouter stopped that \
             server, and the next request starts a fresh one. {advice}"
        )
    } else {
        // No "try again": the chat shows this right before "Trying again
        // (1/1)…", while Biorouter is already retrying, and the notice that
        // ends a turn adds its own advice.
        format!(
            "Llama Server's GPU backend failed while running {model}. Biorouter stopped that \
             server, and the next request starts a fresh one.\n\nllama-server output:\n{tail}"
        )
    }
}

fn looks_like_model_load_failure(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    [
        "wrong number of tensors",
        "error loading model",
        "failed to load model",
        "failed to load model from file",
    ]
    .iter()
    .any(|needle| log.contains(needle))
}

/// Best-effort live context window of the managed sidecar, for callers (e.g.
/// token accounting) that want the running model's real window rather than a
/// fixed default. Returns the value recorded when the server became ready, or
/// probes `/props` directly if a server is up but not yet recorded. `None` when
/// no server is reachable.
pub async fn current_context_size() -> Option<usize> {
    let (recorded, port) = {
        let inner = global().inner.lock().await;
        (inner.context_size, inner.port)
    };
    if recorded.is_some() {
        return recorded;
    }
    // Not recorded by this process. Probe the recorded port if we have one,
    // otherwise the configured port. Only a server this process launched
    // answers: `/props` needs the per-launch key, so another Biorouter
    // process's sidecar on the configured port reports nothing here rather
    // than its window.
    live_context_size(port.unwrap_or_else(configured_port)).await
}

/// A loopback probe client carrying the sidecar's bearer key when one is set.
/// `/health` is public on b11429; `/props`, `/slots`, `/v1/models` and every
/// completion endpoint answer 401 without the key.
fn probe_request(client: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
    let request = client.get(url);
    match current_api_key() {
        Some(key) => request.bearer_auth(key),
        None => request,
    }
}

/// Query a running llama-server's `/props` for the context window it actually
/// allocated for the current model. Returns `None` if the server isn't
/// reachable or doesn't report it.
async fn live_context_size(port: u16) -> Option<usize> {
    let url = format!("http://127.0.0.1:{port}/props");
    let client = reqwest::Client::builder()
        .timeout(STATUS_PROBE_TIMEOUT)
        .build()
        .ok()?;
    let response = probe_request(&client, &url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    // llama-server reports the live window under
    // default_generation_settings.n_ctx; fall back to a top-level n_ctx.
    body.get("default_generation_settings")
        .and_then(|g| g.get("n_ctx"))
        .or_else(|| body.get("n_ctx"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
}

/// How to start `source`, preferring whatever needs no network:
///
/// 1. the catalog's own Hugging Face file, already in the cache, by path (`-m`);
/// 2. a GGUF from Ollama's store under any of the model's Ollama tags (`-m`);
/// 3. otherwise `-hf`, which downloads it into the cache.
///
/// The first launch after a download therefore no longer contacts Hugging Face
/// at all: `-hf` re-resolves the repository's `main` on every launch, so an
/// air-gapped or VPN-restricted machine could not start a model it already had,
/// and a re-upload under the same file name (ggml-org's Qwen3.8 GGUFs changed on
/// 2026-09-19) silently cost a 19 GB re-download.
fn launch_source_for(source: &ModelSource) -> LaunchSource {
    launch_source_in_dir(&model_cache_dir(), source)
}

fn launch_source_in_dir(cache_dir: &Path, source: &ModelSource) -> LaunchSource {
    if let Some(path) = hf_cached_model_file_in_dir(cache_dir, &source.hf_spec) {
        warn_if_unexpected_file(&path, source.expected_file.as_ref());
        return LaunchSource::CachedFile(path);
    }
    source
        .ollama_candidates()
        .find_map(|name| ollama_model_blob_path_in_dir(cache_dir, name))
        .map(LaunchSource::OllamaBlob)
        .unwrap_or_else(|| LaunchSource::HuggingFace(source.hf_spec.clone()))
}

/// The size check that is cheap enough to run on every launch. A different
/// size means the repository's file changed since the catalog pinned it; it is
/// still a valid GGUF, so it is used, but the log says so.
fn cached_file_matches(path: &Path, expected: &ExpectedGguf) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == expected.size_bytes)
}

fn warn_if_unexpected_file(path: &Path, expected: Option<&ExpectedGguf>) {
    if let Some(expected) = expected {
        if !cached_file_matches(path, expected) {
            tracing::warn!(
                "{} is not the {} the catalog pins ({} bytes, sha256 {}); the repository's file \
                 has changed since. Using it anyway.",
                path.display(),
                expected.file_name,
                expected.size_bytes,
                expected.sha256
            );
        }
    }
}

/// Per-launch options that are not part of the model's identity.
struct LaunchOptions<'a> {
    /// The catalog's `--spec-type` default for this model.
    spec_type_default: Option<&'a str>,
    /// The 0600 file holding this launch's API key.
    api_key_file: Option<&'a Path>,
}

/// Build the llama-server argv (without the program itself). Factored out for
/// testing.
fn build_args(
    source: &LaunchSource,
    alias: &str,
    port: u16,
    ctx_size: usize,
    options: &LaunchOptions<'_>,
) -> Vec<String> {
    let config = crate::config::Config::global();
    let mut args = match source {
        LaunchSource::OllamaBlob(path) | LaunchSource::CachedFile(path) => {
            vec!["-m".to_string(), path.display().to_string()]
        }
        LaunchSource::HuggingFace(hf_spec) => vec!["-hf".to_string(), hf_spec.to_string()],
    };
    args.extend([
        "--alias".to_string(),
        alias.to_string(),
        "--ctx-size".to_string(),
        ctx_size.to_string(),
        "--parallel".to_string(),
        configured_parallel().to_string(),
        "--jinja".to_string(),
        // `--no-webui` is deprecated on b11429 ("use --ui/--no-ui"). A pin that
        // drops the alias would fail at argv parse on every start, and self-heal
        // only drops the optional acceleration flags, so it would not recover.
        "--no-ui".to_string(),
        // The slots monitor reports per-slot state of the cached conversation.
        // Nothing in Biorouter reads it.
        "--no-slots".to_string(),
        "--no-mmproj".to_string(),
        // Halve KV-cache memory so the (memory-tiered) default context stays
        // affordable. q8_0 is considered lossless in practice; it's q4 KV that
        // degrades tool calling. Overridable via LLAMACPP_EXTRA_ARGS.
        "--cache-type-k".to_string(),
        "q8_0".to_string(),
        "--cache-type-v".to_string(),
        "q8_0".to_string(),
    ]);
    // The server-wide thinking default, which a request without
    // `chat_template_kwargs.enable_thinking` gets. Biorouter's own requests
    // always state it (the provider maps Quick/Normal/Deep onto it, and turns it
    // off for warm-up and titles), so this flag decides only what an
    // unadorned request sees. Measured on b11429 with both catalog models: a
    // per-request `enable_thinking` wins over the flag in both directions
    // (`--reasoning off` + `true` thinks, `--reasoning on` + `false` does not),
    // so `off` costs Deep nothing, and an unadorned request still answers
    // without spending its budget on hidden reasoning — Qwen3.8's template
    // thinks by default, which `auto` would have let it do.
    let reasoning = if thinking_enabled_by_default() {
        "on"
    } else {
        "off"
    };
    args.extend(["--reasoning".to_string(), reasoning.to_string()]);
    // Self-speculative decoding. The `ngram-*` family drafts from patterns already
    // present in the context, so unlike draft-model speculation it needs no second
    // GGUF, no download, no extra VRAM and no tokenizer pairing — it is free.
    //
    // Measured on an M4 Max with gemma-4-E4B q4_0 (b9611), agentic task / free text:
    //   none          79.7 / 82.4 tok/s   (what we shipped)
    //   ngram-cache  113.8 / 82.4
    //   ngram-simple 354.4 / 82.8   <- chosen: 4.4x, and free text is unchanged
    //   ngram-mod    397.3 / 80.6        faster, but costs 2% on free text
    //   ngram-map-k4v 378.4 / 69.1       disqualified: -16% on free text
    // The agentic case — re-emitting text already in context (quoting a tool
    // result, rewriting a file, echoing a schema) — is most of what an agent does.
    //
    // The catalog models carry their own default, re-measured on b11429 (see
    // `MODEL_CATALOG` in llamacpp.rs): Qwen3.8's linear-attention layers and
    // Gemma 4's sliding window both force checkpoint-based speculation, so the
    // E4B result above did not simply carry over. A raw Hugging Face spec keeps
    // DEFAULT_SPEC_TYPE.
    //
    // Set LLAMACPP_SPEC_TYPE to another value to change it, or to "none"/"" to
    // pass nothing; `unsupported_argument` below recovers automatically if a
    // future llama.cpp drops the flag.
    if let Some(spec) = configured_spec_type_for(options.spec_type_default) {
        args.push("--spec-type".to_string());
        args.push(spec);
    }
    if let Ok(extra) = config.get_param::<String>("LLAMACPP_EXTRA_ARGS") {
        args.extend(sanitize_extra_args(&extra));
    }
    // After the user's extra arguments, so nothing they write can re-point the
    // key file (the allowlist already refuses it; this is the second layer).
    if let Some(key_file) = options.api_key_file {
        args.push("--api-key-file".to_string());
        args.push(key_file.display().to_string());
    }
    // Keep the managed bind after user-supplied arguments as a second layer of
    // protection in case the sanitizer misses a future argument form.
    args.push("--host".to_string());
    args.push("127.0.0.1".to_string());
    args.push("--port".to_string());
    args.push(port.to_string());
    args
}

/// How many values an allowed `LLAMACPP_EXTRA_ARGS` flag takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arity {
    /// A switch (`--no-warmup`).
    None,
    /// Exactly one value (`--threads 8`).
    One,
    /// An optional value from a fixed set (`--flash-attn [on|off|auto]`).
    Optional(&'static [&'static str]),
}

/// The flags `LLAMACPP_EXTRA_ARGS` may pass: performance and sampling knobs,
/// and nothing that changes what the server exposes, loads or executes.
///
/// This replaced a blocklist (`--host`, `--port`, `--api-key`, ...) that missed
/// `--tools`, which turns on llama-server's built-in `exec_shell_command`,
/// `write_file` and `edit_file` tools for any caller — and config.yaml is
/// agent-writable, so one prompt-injected write would have handed every web
/// page a shell (the server reflected any `Origin` with credentials before it
/// had an API key). A blocklist also silently admits whatever dangerous flag
/// the next pin adds; an allowlist cannot. Notable refusals: `--tools`,
/// `--agent`, `--mcp-*`, `--ui*`, `--path`, `--media-path`, `--slot-save-path`,
/// `--models-*`, `--props`, `--api-*`, `--ssl-*`, `--rpc`, `--reuse-port`,
/// `--context-shift` (it would truncate an over-long prompt silently instead of
/// rejecting it, which is the very failure the Ollama provider was fixed for),
/// `--spec-draft-model`/`--spec-draft-hf` (another model file or download), and
/// anything `--host`/`--port`.
const ALLOWED_EXTRA_FLAGS: &[(&[&str], Arity)] = &[
    (&["-t", "--threads"], Arity::One),
    (&["-tb", "--threads-batch"], Arity::One),
    (&["--threads-http"], Arity::One),
    (&["-ngl", "--gpu-layers", "--n-gpu-layers"], Arity::One),
    (
        &["-fa", "--flash-attn"],
        Arity::Optional(&["on", "off", "auto"]),
    ),
    (&["-b", "--batch-size"], Arity::One),
    (&["-ub", "--ubatch-size"], Arity::One),
    (&["-ctk", "--cache-type-k"], Arity::One),
    (&["-ctv", "--cache-type-v"], Arity::One),
    (&["-lm", "--load-mode"], Arity::One),
    (
        &["-ctxcp", "--ctx-checkpoints", "--swa-checkpoints"],
        Arity::One,
    ),
    (&["-cms", "--checkpoint-min-step"], Arity::One),
    (&["-cram", "--cache-ram"], Arity::One),
    (&["--cache-reuse"], Arity::One),
    (&["-np", "--parallel"], Arity::One),
    (
        &["-kvu", "--kv-unified", "-no-kvu", "--no-kv-unified"],
        Arity::None,
    ),
    (&["--kv-unified-per-slot"], Arity::One),
    (
        &["-cb", "--cont-batching", "-nocb", "--no-cont-batching"],
        Arity::None,
    ),
    (&["--no-warmup", "--warmup"], Arity::None),
    (&["--temp", "--temperature"], Arity::One),
    (&["--top-k"], Arity::One),
    (&["--top-p"], Arity::One),
    (&["--min-p"], Arity::One),
    (&["--presence-penalty"], Arity::One),
    (&["--repeat-penalty"], Arity::One),
    (&["--reasoning-budget"], Arity::One),
];

/// `--spec-*` flags are allowed by prefix (they tune speculative decoding), less
/// the ones that load another model file or download one. Matched against the
/// [`canonical_flag`] spelling, never the raw token.
const DENIED_SPEC_FLAGS: &[&str] = &[
    "--spec-draft-model",
    "--spec-draft-hf",
    "--spec-draft-override-tensor",
    "--spec-draft-device",
];
const SPEC_SWITCHES: &[&str] = &[
    "--spec-default",
    "--spec-draft-backend-sampling",
    "--no-spec-draft-backend-sampling",
    "--spec-draft-cpu-moe",
];

fn allowed_extra_flag(flag: &str) -> Option<Arity> {
    if let Some((_, arity)) = ALLOWED_EXTRA_FLAGS
        .iter()
        .find(|(names, _)| names.contains(&flag))
    {
        return Some(*arity);
    }
    let is_spec = flag.starts_with("--spec-") || flag.starts_with("--no-spec-");
    if is_spec && !DENIED_SPEC_FLAGS.contains(&flag) {
        return Some(if SPEC_SWITCHES.contains(&flag) {
            Arity::None
        } else {
            Arity::One
        });
    }
    None
}

/// A token that is a value rather than a flag. Negative numbers (`-ngl -1`)
/// are values.
fn is_value_token(token: &str) -> bool {
    match token.strip_prefix('-') {
        None => true,
        Some(rest) => rest.starts_with(|c: char| c.is_ascii_digit() || c == '.'),
    }
}

/// The spelling llama.cpp looks a flag up by. Its parser rewrites every `_` in
/// a `--` token to `-` before the lookup (`common/arg.cpp`), so
/// `--spec-draft_hf` IS `--spec-draft-hf` to the server. The allowlist used to
/// match the raw token, which let that spelling past [`DENIED_SPEC_FLAGS`]:
/// measured on b11429, `llama-server --spec-draft_model /nonexistent/x.gguf
/// --version` parses and exits 0, where an unknown flag is "invalid argument".
/// One agent-written `LLAMACPP_EXTRA_ARGS` line could then have every launch
/// download and load an attacker's GGUF as a draft model, whatever the user
/// picked. Single-dash flags are not rewritten by llama.cpp, so not here.
fn canonical_flag(flag: &str) -> String {
    if flag.starts_with("--") {
        flag.replace('_', "-")
    } else {
        flag.to_string()
    }
}

/// Whitespace-split `LLAMACPP_EXTRA_ARGS` and keep only allowlisted flags with
/// their values, in the [`canonical_flag`] spelling that was checked. Anything
/// else is dropped with a warning, together with a value token that follows it,
/// so a refused `--tools all` cannot leave a stray `all` that a later flag would
/// swallow.
///
/// `--flag=value` is passed on as two tokens: b11429 accepts no `=` form at
/// all (`--threads=8` is "error: invalid argument: --threads=8", exit 1), so
/// forwarding it verbatim failed every launch — and the argument error then
/// tripped the self-heal that switches speculative decoding off for the rest
/// of the process. A flag whose value is missing or not one it takes is
/// dropped rather than left to swallow whatever comes next.
fn sanitize_extra_args(extra: &str) -> Vec<String> {
    let tokens: Vec<&str> = extra.split_whitespace().collect();
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        let (raw_flag, inline_value) = match tok.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (tok, None),
        };
        let flag = canonical_flag(raw_flag);
        let next = tokens.get(i + 1).copied();
        let allowed = if tok.starts_with('-') {
            allowed_extra_flag(&flag)
        } else {
            None
        };
        let Some(arity) = allowed else {
            tracing::warn!(
                "Ignoring '{tok}' in LLAMACPP_EXTRA_ARGS: only performance and sampling flags \
                 may be passed to the managed llama-server"
            );
            i += 1;
            if tok.starts_with('-')
                && inline_value.is_none()
                && next.is_some_and(|v| !v.starts_with('-'))
            {
                i += 1;
            }
            continue;
        };
        i += 1;
        let takes = |value: &str| match arity {
            Arity::None => false,
            Arity::One => !value.is_empty() && is_value_token(value),
            Arity::Optional(values) => values.contains(&value),
        };
        match inline_value {
            Some(value) if takes(value) => out.extend([flag, value.to_string()]),
            Some(_) => tracing::warn!(
                "Ignoring '{tok}' in LLAMACPP_EXTRA_ARGS: {flag} does not take that value"
            ),
            None => match next {
                Some(value) if takes(value) => {
                    out.extend([flag, value.to_string()]);
                    i += 1;
                }
                _ if matches!(arity, Arity::One) => tracing::warn!(
                    "Ignoring '{tok}' in LLAMACPP_EXTRA_ARGS: it needs a value after it"
                ),
                _ => out.push(flag),
            },
        }
    }
    out
}

async fn health_ok(port: u16, timeout: Duration) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(client.get(&url).send().await, Ok(r) if r.status().is_success())
}

fn port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

// ── orphan reaping ──────────────────────────────────────────────────────────
// `kill_on_drop` only covers drops inside a live tokio runtime; the manager
// lives in a static, so a normal process exit (or SIGTERM/SIGKILL) leaks the
// llama-server child — with a multi-GB model held in memory. Each spawn is
// therefore recorded as `<parent-pid>.pid -> <child-pid>` under a run dir,
// and the next `ensure()` in any Biorouter process kills children whose
// parent is gone. Pidfiles of still-living parents (e.g. the CLI and the
// desktop running side by side) are left alone.

fn run_dir() -> PathBuf {
    crate::config::paths::Paths::in_data_dir("llamacpp/run")
}

fn pid_alive(pid: u32) -> bool {
    let pid = pid.to_string();
    if cfg!(target_os = "windows") {
        system_command("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
            .unwrap_or(false)
    } else {
        system_command("kill")
            .args(["-0", &pid])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Guards against PID reuse: only kill a recorded pid if it still looks like
/// a llama-server process.
fn pid_is_llama_server(pid: u32) -> bool {
    let pid = pid.to_string();
    if cfg!(target_os = "windows") {
        system_command("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("llama-server"))
            .unwrap_or(false)
    } else {
        system_command("ps")
            .args(["-p", &pid, "-o", "comm="])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("llama-server"))
            .unwrap_or(false)
    }
}

fn kill_pid(pid: u32) {
    let pid = pid.to_string();
    if cfg!(target_os = "windows") {
        let _ = system_command("taskkill")
            .args(["/PID", &pid, "/F"])
            .output();
    } else {
        let _ = system_command("kill").args(["-9", &pid]).output();
    }
}

fn pidfile_path() -> PathBuf {
    run_dir().join(format!("{}.pid", std::process::id()))
}

fn write_pidfile(child_pid: u32) {
    let dir = run_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(pidfile_path(), child_pid.to_string());
    }
}

fn clear_pidfile() {
    let _ = std::fs::remove_file(pidfile_path());
}

fn key_file_path_in(dir: &Path) -> PathBuf {
    dir.join(format!("{}.key", std::process::id()))
}

/// 32 random bytes from the OS, hex-encoded.
fn generate_api_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `key` to `<dir>/<our pid>.key`, readable by this user only. The file is
/// removed and created fresh, because `mode` applies only when a file is
/// created: rewriting one a looser umask left behind would keep its looser
/// permissions.
fn write_api_key_file_in(dir: &Path, key: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = key_file_path_in(dir);
    let _ = std::fs::remove_file(&path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    std::io::Write::write_all(&mut file, format!("{key}\n").as_bytes())?;
    Ok(path)
}

fn clear_api_key() {
    set_current_api_key(None);
    let _ = std::fs::remove_file(key_file_path_in(&run_dir()));
}

/// Kill llama-server children recorded by Biorouter processes that no longer
/// exist. Returns true when anything was killed (callers may want to give
/// the OS a moment to release the port).
fn reap_orphans() -> bool {
    reap_orphans_in(&run_dir())
}

fn reap_orphans_in(dir: &Path) -> bool {
    let mut killed = false;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let parent: Option<u32> = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse().ok());
        // `<pid>.key` files share the directory. A dead parent's key goes with
        // it; a live one's is never touched (it is not a pidfile, and reading
        // it as one used to mean deleting it as unparseable).
        if path.extension().and_then(|e| e.to_str()) == Some("key") {
            if parent.is_some_and(|p| p != std::process::id() && !pid_alive(p)) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        let child: Option<u32> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse().ok());
        let (Some(parent), Some(child)) = (parent, child) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if parent == std::process::id() || pid_alive(parent) {
            continue;
        }
        if pid_alive(child) && pid_is_llama_server(child) {
            tracing::info!(
                "Reaping orphaned llama-server (pid {child}) left by exited Biorouter process {parent}"
            );
            kill_pid(child);
            killed = true;
        }
        let _ = std::fs::remove_file(&path);
    }
    killed
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn push_log(tail: &StdMutex<VecDeque<String>>, line: String) {
    if let Ok(mut t) = tail.lock() {
        if t.len() >= LOG_TAIL_LINES {
            t.pop_front();
        }
        t.push_back(line);
    }
}

fn last_log(tail: &StdMutex<VecDeque<String>>) -> Option<String> {
    tail.lock().ok().and_then(|t| t.back().cloned())
}

fn log_tail_joined(tail: &StdMutex<VecDeque<String>>, n: usize) -> String {
    tail.lock()
        .map(|t| {
            t.iter()
                .rev()
                .take(n)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

async fn finish_log_tasks(tasks: &mut Vec<tokio::task::JoinHandle<()>>) {
    for task in std::mem::take(tasks) {
        let _ = task.await;
    }
}

/// Environment variables the sidecar must not inherit.
///
/// llama-server reads nearly every flag from a `LLAMA_ARG_*` variable as well
/// (`LLAMA_ARG_TOOLS`, `LLAMA_ARG_HOST`, `LLAMA_ARG_UI_MCP_PROXY`, ...), plus
/// `LLAMA_API_KEY`, so a stray or injected variable in the user's environment
/// reached the child unfiltered and could undo every guarantee `build_args`
/// makes. A Hugging Face token is the user's credential for every gated repo
/// they can read; the catalog's repos are ungated, so the sidecar gets one only
/// when `LLAMACPP_HF_TOKEN` is configured for it ([`configured_hf_token`]).
fn should_strip_child_env(key: &OsStr) -> bool {
    let Some(key) = key.to_str() else {
        return false;
    };
    // Windows environment names are case-insensitive.
    let key = key.to_ascii_uppercase();
    key.starts_with("LLAMA_")
        || matches!(
            key.as_str(),
            "HF_TOKEN" | "HUGGING_FACE_HUB_TOKEN" | "HF_TOKEN_PATH"
        )
}

fn scrub_child_env(command: &mut Command, hf_token: Option<&str>) {
    for (key, _) in std::env::vars_os() {
        if should_strip_child_env(&key) {
            command.env_remove(&key);
        }
    }
    if let Some(token) = hf_token.map(str::trim).filter(|t| !t.is_empty()) {
        command.env("HF_TOKEN", token);
    }
}

/// `LLAMACPP_HF_TOKEN`: a credential, declared secret in the provider's
/// metadata, so it is read like one — the environment first, then the secret
/// store, where the settings form saves a secret. A value an older build left
/// in config.yaml as a plain setting is still honoured. The secret store is
/// read once per process and cached, so a spawn does not prompt again.
fn configured_hf_token() -> Option<String> {
    let config = crate::config::Config::global();
    config
        .get_secret::<String>("LLAMACPP_HF_TOKEN")
        .or_else(|_| config.get_param::<String>("LLAMACPP_HF_TOKEN"))
        .ok()
        .filter(|token| !token.trim().is_empty())
}

fn spawn_child(
    inner: &mut Inner,
    binary: &Path,
    cache_dir: &Path,
    model: &str,
    source: &ModelSource,
    port: u16,
    ctx_size: usize,
) -> Result<Child> {
    let launch_source = launch_source_for(source);
    spawn_child_with_launch_source(
        inner,
        binary,
        cache_dir,
        model,
        launch_source,
        port,
        ctx_size,
    )
}

fn spawn_child_with_launch_source(
    inner: &mut Inner,
    binary: &Path,
    cache_dir: &Path,
    model: &str,
    launch_source: LaunchSource,
    port: u16,
    ctx_size: usize,
) -> Result<Child> {
    debug_assert!(inner.log_tasks.is_empty());
    // A fresh key for every launch: whatever the previous process accepted is
    // dead with it.
    let api_key = generate_api_key();
    let key_file = write_api_key_file_in(&run_dir(), &api_key)
        .map_err(|e| anyhow!("Failed to write the llama-server API key file: {e}"))?;
    let spec_type_default = inner
        .model_source
        .as_ref()
        .and_then(|source| source.spec_type.clone());
    let args = build_args(
        &launch_source,
        model,
        port,
        ctx_size,
        &LaunchOptions {
            spec_type_default: spec_type_default.as_deref(),
            api_key_file: Some(&key_file),
        },
    );
    if ctx_size < LLAMACPP_AGENT_CONTEXT_SIZE {
        tracing::warn!(
            "LLAMACPP_CONTEXT_SIZE={ctx_size} is below the {LLAMACPP_AGENT_CONTEXT_SIZE} tokens \
             Biorouter's instructions and tools need; most requests will be refused as too long"
        );
    }
    tracing::info!(
        "Starting llama-server ({}) on port {port}: {} {}",
        LLAMA_SERVER_BUILD,
        binary.display(),
        args.join(" ")
    );

    if let Ok(mut t) = inner.log_tail.lock() {
        t.clear();
    }
    inner.last_error = None;
    inner.launch_source = Some(launch_source);
    inner.request_slots = Arc::new(tokio::sync::Semaphore::new(parallel_slots_in(&args)));
    set_current_api_key(Some(api_key));

    let mut command = Command::new(binary);
    command
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    scrub_child_env(&mut command, configured_hf_token().as_deref());
    command.env("LLAMA_CACHE", cache_dir);
    crate::subprocess::prepare_agent_child_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("Failed to start llama-server at {}: {e}", binary.display()))?;

    if let Some(stdout) = child.stdout.take() {
        let tail = inner.log_tail.clone();
        inner.log_tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    push_log(&tail, line);
                }
            }
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let tail = inner.log_tail.clone();
        inner.log_tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    push_log(&tail, line);
                }
            }
        }));
    }

    if let Some(child_pid) = child.id() {
        write_pidfile(child_pid);
    }

    Ok(child)
}

impl LlamaSidecar {
    /// True when the running child serves `model` at the window this call
    /// would ask for, so `ensure` has nothing to do.
    fn serving_requested(
        inner: &mut Inner,
        model: &str,
        ctx_size: usize,
        automatic_context_size: bool,
    ) -> Result<bool> {
        if inner.model.as_deref() != Some(model) {
            return Ok(false);
        }
        let Some(child) = inner.child.as_mut() else {
            return Ok(false);
        };
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        Ok(if inner.automatic_context_size && automatic_context_size {
            true
        } else {
            inner.requested_context_size == Some(ctx_size)
                && inner.automatic_context_size == automatic_context_size
        })
    }

    /// Ensure a llama-server is running and serving `model` (friendly alias)
    /// backed by `source`. Returns the port it is (or will be) serving on.
    /// Does NOT wait for readiness — pair with [`Self::wait_ready`], or use
    /// [`Self::lease`], which does both and then holds the model.
    ///
    /// Any (re)spawn first takes the switch lock for writing, so it waits for
    /// every request still holding a [`SidecarLease`] on the running model.
    /// Never call this while holding a lease yourself: that is a deadlock.
    pub async fn ensure(&self, model: &str, source: &ModelSource) -> Result<u16> {
        let explicit_ctx = explicit_context_size();
        let ctx_size = configured_context_size();
        let automatic_context_size = explicit_ctx.is_none();

        // Fast path: already serving it. Checked without the switch lock so a
        // turn on the running model never queues behind anything.
        {
            let mut inner = self.inner.lock().await;
            if Self::serving_requested(&mut inner, model, ctx_size, automatic_context_size)? {
                return Ok(inner.port.expect("running child always has a port"));
            }
        }

        // The switch lock is taken BEFORE `inner`, and `inner` is never held
        // while waiting for it: a lease holder takes `inner` briefly after its
        // read lock, so the opposite order deadlocks.
        let _switch = self.switch.write().await;
        let mut inner = self.inner.lock().await;
        // Re-check: another caller may have started this model while we waited.
        if Self::serving_requested(&mut inner, model, ctx_size, automatic_context_size)? {
            return Ok(inner.port.expect("running child always has a port"));
        }

        if inner.model.as_deref() == Some(model) {
            if let Some(child) = inner.child.as_mut() {
                if child.try_wait()?.is_some() {
                    // Process died — record why before respawning.
                    finish_log_tasks(&mut inner.log_tasks).await;
                    inner.last_error = Some(log_tail_joined(&inner.log_tail, 8));
                    inner.child = None;
                    inner.warmed_model = None;
                }
            }
        }

        // A different model (or window, or nothing) is running: stop what we own.
        if let Some(mut child) = inner.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
            finish_log_tasks(&mut inner.log_tasks).await;
        }

        // Clean up llama-servers leaked by Biorouter processes that died
        // without dropping their child (statics are never dropped on exit).
        if reap_orphans() {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        // Pick a port. If something else (an unrelated service, or another
        // live Biorouter process's sidecar) already listens on the configured
        // port, leave it alone and use an ephemeral one — never adopt a
        // process we did not spawn, since its launch flags may not match
        // this version's.
        let preferred = configured_port();
        let port = if port_in_use(preferred) {
            tracing::warn!(
                "Port {preferred} is already in use; starting llama-server on an ephemeral port"
            );
            free_port()?
        } else {
            preferred
        };

        let binary = find_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found. It ships with the Biorouter desktop app; for \
                 CLI-only installs, install llama.cpp (e.g. `brew install llama.cpp`) or set \
                 BIOROUTER_LLAMACPP_BIN to a llama-server path."
            )
        })?;

        let cache_dir = model_cache_dir();
        std::fs::create_dir_all(&cache_dir)?;

        // Set before the spawn: `spawn_child` reads the model's launch defaults
        // (its `--spec-type`) from here, as every self-heal respawn does.
        inner.model_source = Some(source.clone());
        let child = spawn_child(
            &mut inner, &binary, &cache_dir, model, source, port, ctx_size,
        )?;
        inner.child = Some(child);
        inner.port = Some(port);
        inner.model = Some(model.to_string());
        // A fresh spawn ends any restart-in-progress. `wait_ready` clears this on
        // every path it owns, but it does not own cancellation: if the task
        // awaiting readiness is dropped between the flag being set and cleared,
        // the flag would otherwise stay true and `status()` would report
        // `Starting` forever for a process that is actually dead.
        inner.restarting = false;
        // New model: the previous model's window no longer applies; it is
        // re-read from /props once this one is ready.
        inner.context_size = None;
        inner.requested_context_size = Some(ctx_size);
        inner.automatic_context_size = automatic_context_size;
        inner.partial_download_restart_attempted = false;
        inner.warmed_model = None;
        Ok(port)
    }

    /// Start `model` if needed, wait for it, and hold it: the returned lease
    /// keeps any model switch waiting until it is dropped, and holds one of the
    /// server's request slots. A request holds one for as long as the server is
    /// generating for it. When every slot is taken this waits, without a
    /// deadline, for one to free up: the request it guards has not been sent
    /// yet, so no timer is running against it.
    ///
    /// Between `wait_ready` and taking the read lock another caller can switch
    /// the server; the lease is then retried, and after [`LEASE_ATTEMPTS`]
    /// switches the request fails clearly rather than being answered by
    /// whichever model happened to be loaded.
    pub async fn lease(
        &self,
        model: &str,
        source: &ModelSource,
        startup_timeout: Duration,
    ) -> Result<SidecarLease> {
        for _ in 0..LEASE_ATTEMPTS {
            self.ensure(model, source).await?;
            let port = self.wait_ready(startup_timeout).await?;
            if let Some(lease) = self.try_lease(model, port).await {
                return Ok(lease);
            }
        }
        Err(anyhow!(
            "Llama Server kept being switched to another model while {model} was starting \
             (another chat or window is using a different local model). Try again once that \
             reply has finished."
        ))
    }

    /// Take the read lock, confirm the server is still the one serving `model`
    /// on `port`, then wait for one of its request slots. `None` when a switch
    /// got in first.
    async fn try_lease(&self, model: &str, port: u16) -> Option<SidecarLease> {
        let guard = self.switch.clone().read_owned().await;
        let slots = {
            let mut inner = self.inner.lock().await;
            if !Self::still_serving(&mut inner, model, port) {
                return None;
            }
            inner.request_slots.clone()
        };
        // Waited for without `inner`: a status poll must never queue behind a
        // request that is waiting for another one to finish generating.
        if slots.available_permits() == 0 {
            tracing::info!(
                "Llama Server is busy with another request for {model}; this one waits for it"
            );
        }
        // The semaphore is never closed, so this cannot fail.
        let slot = slots.acquire_owned().await.ok()?;
        // The read lock kept every model switch out, but not
        // `discard_failed_backend`, which kills a server whose GPU backend
        // failed while the request ahead of this one was on it. `None` sends
        // `lease` round again to start a fresh server, instead of sending this
        // request to a dead port.
        if !Self::still_serving(&mut *self.inner.lock().await, model, port) {
            return None;
        }
        Some(SidecarLease {
            _guard: guard,
            _slot: slot,
            port,
        })
    }

    fn still_serving(inner: &mut Inner, model: &str, port: u16) -> bool {
        inner.model.as_deref() == Some(model)
            && inner.port == Some(port)
            && matches!(inner.child.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    /// Wait until `GET /health` succeeds or `timeout` elapses. The first run
    /// can include a large fallback download, so generous timeouts are expected.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<u16> {
        let _startup = self.startup.lock().await;
        let mut deadline = tokio::time::Instant::now() + timeout;
        // The timeout is a NO-PROGRESS budget, not a total one. A first run
        // downloads the weights — 3 to 24 GB from Hugging Face depending on the
        // catalog entry — and on an ordinary connection that alone can outlast
        // the 3600 s ceiling every caller passes. Failing then reports "timed
        // out waiting for the local model" at a moment when the download is
        // healthily progressing, and throws away everything fetched so far.
        //
        // llama-server narrates the download and the load on stderr, which
        // `push_log` feeds into `log_tail`, so a changing tail is a reliable
        // liveness signal. Every time it moves we extend the budget; the
        // deadline therefore only bites on a server that has genuinely gone
        // quiet.
        let mut last_progress: Option<String> = None;
        loop {
            let (port, dead, tail, request_slots) = {
                let mut inner = self.inner.lock().await;
                let port = inner
                    .port
                    .ok_or_else(|| anyhow!("llama-server is not running"))?;
                let dead = match inner.child.as_mut() {
                    Some(child) => child.try_wait()?.is_some(),
                    None => true,
                };
                if dead {
                    finish_log_tasks(&mut inner.log_tasks).await;
                }
                (
                    port,
                    dead,
                    inner.log_tail.clone(),
                    inner.request_slots.clone(),
                )
            };

            if dead {
                let tail = log_tail_joined(&tail, 10);
                match self.recover_exited_server(&tail, &request_slots).await? {
                    StartupRecovery::Obsolete => continue,
                    StartupRecovery::Retried => {
                        deadline = tokio::time::Instant::now() + timeout;
                        continue;
                    }
                    StartupRecovery::Failed(message) => return Err(anyhow!(message)),
                }
            }
            if health_ok(port, STATUS_PROBE_TIMEOUT).await {
                // Record the live context window now that the model is loaded,
                // so callers report the model's real window, not a preset.
                if let Some(ctx) = live_context_size(port).await {
                    self.inner.lock().await.context_size = Some(ctx);
                }
                return Ok(port);
            }
            // Any movement in the log means the server is still working.
            let marker = progress_marker(&tail);
            if last_progress.as_deref() != Some(marker.as_str()) {
                last_progress = Some(marker);
                deadline = tokio::time::Instant::now() + timeout;
            }

            if tokio::time::Instant::now() >= deadline {
                let last = last_log(&tail).unwrap_or_default();
                self.inner.lock().await.last_error = Some(last.clone());
                return Err(anyhow!(
                    "llama-server made no progress for {}s (last output: {last})",
                    timeout.as_secs()
                ));
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    async fn recover_exited_server(
        &self,
        tail: &str,
        request_slots: &Arc<tokio::sync::Semaphore>,
    ) -> Result<StartupRecovery> {
        // A model switch can win after wait_ready observes an exited child.
        // Hold the switch lock across validation and every retry so its old
        // log cannot restart the replacement or change its launch policy.
        let _switch = self.switch.write().await;
        {
            let mut inner = self.inner.lock().await;
            if !Arc::ptr_eq(&inner.request_slots, request_slots) {
                return Ok(StartupRecovery::Obsolete);
            }
            if let Some(child) = inner.child.as_mut() {
                if child.try_wait()?.is_none() {
                    return Ok(StartupRecovery::Obsolete);
                }
            }
            inner.restarting = true;
        }

        let retried = self.try_self_heal(tail).await;
        let mut inner = self.inner.lock().await;
        inner.restarting = false;
        if retried? {
            return Ok(StartupRecovery::Retried);
        }
        let message = startup_failure_message(
            inner.model.as_deref().unwrap_or("the local model"),
            inner.requested_context_size,
            tail,
        );
        inner.last_error = Some(message.clone());
        Ok(StartupRecovery::Failed(message))
    }

    /// The three self-heal paths, in order, as one unit so `wait_ready` can hold
    /// the `restarting` flag across all of them. Returns `Ok(true)` when one of
    /// them respawned the server and the caller should keep waiting.
    async fn try_self_heal(&self, tail: &str) -> Result<bool> {
        if self.retry_without_optional_flags(tail).await? {
            return Ok(true);
        }
        if self.retry_with_lower_auto_context(tail).await? {
            return Ok(true);
        }
        if self.retry_with_huggingface_fallback(tail).await? {
            return Ok(true);
        }
        if self.retry_partial_huggingface_download_once().await? {
            return Ok(true);
        }
        Ok(false)
    }

    /// Drop optional accelerating flags and respawn, once, when llama-server
    /// rejected one of them (see [`rejects_an_optional_flag`]). Process-wide and
    /// sticky: binary support does not change while we are running, so
    /// re-learning it on every spawn would mean one guaranteed crash per model
    /// switch.
    async fn retry_without_optional_flags(&self, startup_log: &str) -> Result<bool> {
        if !rejects_an_optional_flag(startup_log) || OPTIONAL_FLAGS_DISABLED.load(Ordering::Relaxed)
        {
            return Ok(false);
        }
        let mut inner = self.inner.lock().await;
        let Some(model) = inner.model.clone() else {
            return Ok(false);
        };
        let Some(launch_source) = inner.launch_source.clone() else {
            return Ok(false);
        };
        let Some(port) = inner.port else {
            return Ok(false);
        };
        let ctx_size = inner
            .requested_context_size
            .unwrap_or_else(default_context_size);

        // Flip the switch BEFORE respawning: `build_args` reads it.
        OPTIONAL_FLAGS_DISABLED.store(true, Ordering::Relaxed);

        inner.child = None;
        clear_pidfile();
        let binary = find_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found. It ships with the Biorouter desktop app; for \
                 CLI-only installs, install llama.cpp (e.g. `brew install llama.cpp`) or set \
                 BIOROUTER_LLAMACPP_BIN to a llama-server path."
            )
        })?;
        let cache_dir = model_cache_dir();
        std::fs::create_dir_all(&cache_dir)?;
        tracing::warn!(
            "llama-server rejected an optional argument while loading {model}; retrying without \
             optional acceleration flags"
        );
        let child = spawn_child_with_launch_source(
            &mut inner,
            &binary,
            &cache_dir,
            &model,
            launch_source,
            port,
            ctx_size,
        )?;
        inner.child = Some(child);
        inner.context_size = None;
        inner.warmed_model = None;
        push_log(
            &inner.log_tail,
            "Retrying without optional acceleration flags after an argument error".to_string(),
        );
        Ok(true)
    }

    async fn retry_with_lower_auto_context(&self, startup_log: &str) -> Result<bool> {
        let mut inner = self.inner.lock().await;
        if !inner.automatic_context_size || !looks_like_memory_failure(startup_log) {
            return Ok(false);
        }

        let Some(current_ctx) = inner.requested_context_size else {
            return Ok(false);
        };
        let Some(next_ctx) = next_lower_auto_context_size(current_ctx) else {
            return Ok(false);
        };
        let Some(model) = inner.model.clone() else {
            return Ok(false);
        };
        let Some(launch_source) = inner.launch_source.clone() else {
            return Ok(false);
        };
        let Some(port) = inner.port else {
            return Ok(false);
        };

        inner.child = None;
        clear_pidfile();
        let binary = find_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found. It ships with the Biorouter desktop app; for \
                 CLI-only installs, install llama.cpp (e.g. `brew install llama.cpp`) or set \
                 BIOROUTER_LLAMACPP_BIN to a llama-server path."
            )
        })?;
        let cache_dir = model_cache_dir();
        std::fs::create_dir_all(&cache_dir)?;

        tracing::warn!(
            "llama-server exited while loading {model} with {current_ctx} context; retrying \
             automatic context at {next_ctx}"
        );
        let child = spawn_child_with_launch_source(
            &mut inner,
            &binary,
            &cache_dir,
            &model,
            launch_source,
            port,
            next_ctx,
        )?;
        inner.child = Some(child);
        inner.context_size = None;
        inner.requested_context_size = Some(next_ctx);
        inner.automatic_context_size = true;
        inner.warmed_model = None;
        push_log(
            &inner.log_tail,
            format!("Retrying with lower automatic context ({next_ctx}) after memory failure"),
        );
        Ok(true)
    }

    async fn retry_with_huggingface_fallback(&self, startup_log: &str) -> Result<bool> {
        if !looks_like_model_load_failure(startup_log) {
            return Ok(false);
        }

        let mut inner = self.inner.lock().await;
        // An Ollama blob llama.cpp cannot load (Ollama's own GGUF variants are
        // not always loadable upstream), or a cached file that turned out
        // corrupt: `-hf` re-validates against the repository and re-fetches.
        if !matches!(
            inner.launch_source,
            Some(LaunchSource::OllamaBlob(_) | LaunchSource::CachedFile(_))
        ) {
            return Ok(false);
        }
        let Some(model) = inner.model.clone() else {
            return Ok(false);
        };
        let Some(source) = inner.model_source.clone() else {
            return Ok(false);
        };
        let Some(port) = inner.port else {
            return Ok(false);
        };
        let ctx_size = inner
            .requested_context_size
            .unwrap_or_else(configured_context_size);

        inner.child = None;
        clear_pidfile();
        let binary = find_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found. It ships with the Biorouter desktop app; for \
                 CLI-only installs, install llama.cpp (e.g. `brew install llama.cpp`) or set \
                 BIOROUTER_LLAMACPP_BIN to a llama-server path."
            )
        })?;
        let cache_dir = model_cache_dir();
        std::fs::create_dir_all(&cache_dir)?;

        tracing::warn!(
            "llama-server could not load the local file for {model}; retrying the Hugging Face \
             download"
        );
        let child = spawn_child_with_launch_source(
            &mut inner,
            &binary,
            &cache_dir,
            &model,
            LaunchSource::HuggingFace(source.hf_spec),
            port,
            ctx_size,
        )?;
        inner.child = Some(child);
        inner.context_size = None;
        inner.warmed_model = None;
        push_log(
            &inner.log_tail,
            "Ollama model blob could not be loaded by llama-server; retrying Hugging Face GGUF fallback"
                .to_string(),
        );
        Ok(true)
    }

    async fn retry_partial_huggingface_download_once(&self) -> Result<bool> {
        let mut inner = self.inner.lock().await;
        if inner.partial_download_restart_attempted
            || !matches!(inner.launch_source, Some(LaunchSource::HuggingFace(_)))
        {
            return Ok(false);
        }
        let Some(model) = inner.model.clone() else {
            return Ok(false);
        };
        let Some(source) = inner.model_source.clone() else {
            return Ok(false);
        };
        if model_cache_status(&source.hf_spec) != ModelCacheStatus::Partial {
            return Ok(false);
        }
        let Some(port) = inner.port else {
            return Ok(false);
        };
        let ctx_size = inner
            .requested_context_size
            .unwrap_or_else(configured_context_size);

        inner.child = None;
        inner.partial_download_restart_attempted = true;
        clear_pidfile();
        let binary = find_binary().ok_or_else(|| {
            anyhow!(
                "llama-server binary not found. It ships with the Biorouter desktop app; for \
                 CLI-only installs, install llama.cpp (e.g. `brew install llama.cpp`) or set \
                 BIOROUTER_LLAMACPP_BIN to a llama-server path."
            )
        })?;
        let cache_dir = model_cache_dir();
        std::fs::create_dir_all(&cache_dir)?;

        tracing::warn!(
            "llama-server exited with a partial Hugging Face download for {model}; resuming once"
        );
        let child = spawn_child_with_launch_source(
            &mut inner,
            &binary,
            &cache_dir,
            &model,
            LaunchSource::HuggingFace(source.hf_spec),
            port,
            ctx_size,
        )?;
        inner.child = Some(child);
        inner.context_size = None;
        inner.warmed_model = None;
        push_log(
            &inner.log_tail,
            "Resuming the interrupted Hugging Face download once".to_string(),
        );
        Ok(true)
    }

    /// Non-blocking-ish status snapshot for UIs.
    pub async fn status(&self) -> SidecarStatus {
        let binary = find_binary();
        let mut inner = self.inner.lock().await;

        let mut status = SidecarStatus {
            state: SidecarState::Stopped,
            model: inner.model.clone(),
            hf_spec: inner
                .model_source
                .as_ref()
                .map(|source| source.hf_spec.clone()),
            ollama_name: inner
                .model_source
                .as_ref()
                .and_then(|source| source.ollama_name.clone()),
            model_path: inner
                .launch_source
                .as_ref()
                .and_then(LaunchSource::model_path),
            model_source: inner
                .launch_source
                .as_ref()
                .map(|source| source.source_name().to_string()),
            port: inner.port,
            binary_path: binary.as_ref().map(|p| p.display().to_string()),
            build: LLAMA_SERVER_BUILD.to_string(),
            detail: None,
            context_size: inner.context_size,
            warmed: inner.warmed_model.as_deref() == inner.model.as_deref()
                && inner.model.is_some(),
        };

        if binary.is_none() {
            status.state = SidecarState::NoBinary;
            status.warmed = false;
            return status;
        }

        let had_child = inner.child.is_some();
        let running = matches!(inner.child.as_mut().map(Child::try_wait), Some(Ok(None)));

        if had_child && !running {
            finish_log_tasks(&mut inner.log_tasks).await;
            // A dead child is only an ERROR once `wait_ready` has ruled out all
            // three self-heal paths. While it is still deciding, this is a
            // restart in progress — report it as `Starting`, which every caller
            // already treats as "keep waiting", so an automatic step-down to a
            // lower context (or a Hugging Face fallback) is allowed to complete
            // instead of being aborted by a status poll.
            status.state = if inner.restarting {
                SidecarState::Starting
            } else {
                SidecarState::Error
            };
            status.warmed = false;
            status.detail = Some(
                inner
                    .last_error
                    .clone()
                    .unwrap_or_else(|| log_tail_joined(&inner.log_tail, 8)),
            );
            return status;
        }

        if !running {
            // No process managed by this instance. A server may still be
            // running on the configured port — started by another Biorouter
            // process and not yet adopted here. Probe it so callers (the GUI's
            // context gauge, token accounting) see a real, ready window instead
            // of a "stopped"/default fallback. We don't take ownership of the
            // child; we just report what's reachable.
            let probe_port = inner.port.unwrap_or_else(configured_port);
            if health_ok(probe_port, STATUS_PROBE_TIMEOUT).await {
                status.state = SidecarState::Ready;
                status.port = Some(probe_port);
                status.warmed = false;
                if let Some(ctx) = live_context_size(probe_port).await {
                    inner.context_size = Some(ctx);
                    status.context_size = Some(ctx);
                }
            }
            return status;
        }

        let port = inner.port.expect("running sidecar always has a port");
        if health_ok(port, STATUS_PROBE_TIMEOUT).await {
            status.state = SidecarState::Ready;
            // Fill the live window if we haven't yet (e.g. an adopted server).
            if status.context_size.is_none() {
                if let Some(ctx) = live_context_size(port).await {
                    inner.context_size = Some(ctx);
                    status.context_size = Some(ctx);
                }
            }
        } else {
            status.state = SidecarState::Starting;
            status.warmed = false;
            status.detail = last_log(&inner.log_tail);
        }
        status
    }

    /// Kill the server on `port` after its compute backend failed, so the next
    /// request starts a fresh one, and return what to tell the user.
    ///
    /// When Metal runs out of GPU memory mid-request (measured on b11429 with
    /// other local models holding the GPU), llama-server answers 500 "Compute
    /// error." and logs "backend is in error state from a previous command
    /// buffer failure - recreate the backend to recover" — and then fails EVERY
    /// later request the same way, while `/health` still says it is fine. The
    /// process never dies, so none of the startup self-heal paths ran; the user
    /// was left with a local model that errored until they restarted the app.
    ///
    /// Takes no switch lock: the caller still holds its own lease, and every
    /// other request on this server fails anyway.
    pub async fn discard_failed_backend(&self, port: u16) -> Option<String> {
        let mut inner = self.inner.lock().await;
        if inner.port != Some(port) {
            return None;
        }
        let mut child = inner.child.take()?;
        let _ = child.start_kill();
        let _ = child.wait().await;
        finish_log_tasks(&mut inner.log_tasks).await;
        clear_pidfile();
        clear_api_key();
        let model = inner
            .model
            .clone()
            .unwrap_or_else(|| "the local model".to_string());
        let tail = log_tail_joined(&inner.log_tail, 12);
        let window = inner.context_size.or(inner.requested_context_size);
        let message = backend_failure_message(&model, window, &tail);
        tracing::warn!("llama-server's compute backend failed; discarded it. Log tail:\n{tail}");
        inner.last_error = Some(message.clone());
        inner.context_size = None;
        inner.warmed_model = None;
        Some(message)
    }

    /// Mark the currently served model as warmed after a real generation.
    pub async fn mark_warmed(&self, model: &str) {
        let mut inner = self.inner.lock().await;
        if inner.model.as_deref() == Some(model) {
            inner.warmed_model = Some(model.to_string());
        }
    }

    /// Stop the managed process (no-op when nothing is running or adopted).
    pub async fn stop(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(mut child) = inner.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
            finish_log_tasks(&mut inner.log_tasks).await;
        }
        clear_pidfile();
        clear_api_key();
        inner.model = None;
        inner.model_source = None;
        inner.launch_source = None;
        inner.port = None;
        inner.context_size = None;
        inner.requested_context_size = None;
        inner.automatic_context_size = false;
        inner.partial_download_restart_attempted = false;
        inner.warmed_model = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL_DIGEST: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const DRAFT_DIGEST: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const PROJECTOR_DIGEST: &str =
        "sha256:3333333333333333333333333333333333333333333333333333333333333333";
    const GGUF_CHILD_DIGEST: &str =
        "sha256:4444444444444444444444444444444444444444444444444444444444444444";
    const MLX_CHILD_DIGEST: &str =
        "sha256:5555555555555555555555555555555555555555555555555555555555555555";
    const LIST_DIGEST: &str =
        "sha256:6666666666666666666666666666666666666666666666666666666666666666";

    fn options() -> LaunchOptions<'static> {
        LaunchOptions {
            spec_type_default: Some(DEFAULT_SPEC_TYPE),
            api_key_file: None,
        }
    }

    fn hf(spec: &str) -> LaunchSource {
        LaunchSource::HuggingFace(spec.to_string())
    }

    fn blob_path(dir: &Path, digest: &str) -> PathBuf {
        dir.join("blobs").join(digest.replace(':', "-"))
    }

    fn write_blob(dir: &Path, digest: &str, bytes: &[u8]) -> PathBuf {
        let path = blob_path(dir, digest);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn write_legacy_manifest(dir: &Path, model: &str, tag: &str, manifest: serde_json::Value) {
        let path = dir
            .join("manifests/registry.ollama.ai/library")
            .join(model)
            .join(tag);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, manifest.to_string()).unwrap();
    }

    fn layer(media_type: &str, digest: &str, size: u64) -> serde_json::Value {
        serde_json::json!({ "mediaType": media_type, "digest": digest, "size": size })
    }

    /// The shape of a GGUF tag as Ollama 0.40 writes it (`qwen3.8:27b-q4_K_M`):
    /// a projector, ONE `image.model` layer, and — on Gemma — a separately typed
    /// MTP drafter.
    fn gguf_manifest() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "layers": [
                layer("application/vnd.ollama.image.projector", PROJECTOR_DIGEST, 900),
                layer("application/vnd.ollama.image.model", MODEL_DIGEST, 16),
                layer("application/vnd.ollama.image.draft", DRAFT_DIGEST, 500),
            ]
        })
    }

    /// The shape of `qwen3.8:27b` pulled on Apple Silicon: ~1,200 safetensors
    /// `image.tensor` layers, the manifest list it was chosen from, and no
    /// `image.model` layer at all.
    fn mlx_manifest() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "runner": "mlx",
            "format": "safetensors",
            "layers": [
                layer("application/vnd.ollama.image.tensor", MODEL_DIGEST, 4096),
                layer("application/vnd.ollama.image.tensor", DRAFT_DIGEST, 4096),
                layer("application/vnd.ollama.image.params", PROJECTOR_DIGEST, 92),
                layer("application/vnd.ollama.manifest.list.v2+json", LIST_DIGEST, 470),
            ]
        })
    }

    fn manifest_list() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OLLAMA_MANIFEST_LIST,
            "manifests": [
                { "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                  "digest": MLX_CHILD_DIGEST, "runner": "mlx", "format": "safetensors" },
                { "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                  "digest": GGUF_CHILD_DIGEST, "runner": "llamacpp", "format": "gguf" },
            ]
        })
    }

    #[test]
    fn model_cache_status_detects_missing_partial_and_downloaded() {
        let tmp = tempfile::tempdir().unwrap();
        let hf_spec = "unsloth/Qwen3.5-0.8B-GGUF:Q4_K_M";

        assert_eq!(
            model_cache_status_in_dir(tmp.path(), hf_spec),
            ModelCacheStatus::NotDownloaded
        );

        let repo_dir = tmp.path().join("models--unsloth--Qwen3.5-0.8B-GGUF");
        std::fs::create_dir_all(repo_dir.join("blobs")).unwrap();
        std::fs::write(
            repo_dir.join("blobs").join("abc.downloadInProgress"),
            b"partial",
        )
        .unwrap();
        assert_eq!(
            model_cache_status_in_dir(tmp.path(), hf_spec),
            ModelCacheStatus::Partial
        );

        let snapshot_dir = repo_dir.join("snapshots").join("rev");
        std::fs::create_dir_all(&snapshot_dir).unwrap();
        std::fs::write(snapshot_dir.join("Qwen3.5-0.8B-Q4_K_M.gguf"), b"model").unwrap();
        assert_eq!(
            model_cache_status_in_dir(tmp.path(), hf_spec),
            ModelCacheStatus::Downloaded
        );
    }

    #[test]
    fn model_cache_status_requires_matching_quant_when_specified() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("models--unsloth--model-GGUF");
        let snapshot_dir = repo_dir.join("snapshots").join("rev");
        std::fs::create_dir_all(&snapshot_dir).unwrap();
        std::fs::write(snapshot_dir.join("model-Q8_0.gguf"), b"model").unwrap();

        assert_eq!(
            model_cache_status_in_dir(tmp.path(), "unsloth/model-GGUF:Q4_K_M"),
            ModelCacheStatus::NotDownloaded
        );
        assert_eq!(
            model_cache_status_in_dir(tmp.path(), "unsloth/model-GGUF:Q8_0"),
            ModelCacheStatus::Downloaded
        );
    }

    /// The catalog's two files, and the drafters that sit next to them. b9611
    /// resolved `ggml-org/Qwen3.8-27B-GGUF:Q4_0` to `dflash-Qwen3.8-27B-Q4_0.gguf`
    /// — a draft model, not the model — so a cached drafter must never read as
    /// the cached model either.
    #[test]
    fn cached_model_file_is_chosen_the_way_llama_cpp_chooses_it() {
        let tmp = tempfile::tempdir().unwrap();
        let qwen = tmp
            .path()
            .join("models--ggml-org--Qwen3.8-27B-GGUF/snapshots/71bc7b62");
        std::fs::create_dir_all(&qwen).unwrap();
        for drafter in [
            "dflash-Qwen3.8-27B-Q4_0.gguf",
            "mtp-Qwen3.8-27B-Q4_0.gguf",
            "eagle3-Qwen3.8-27B-Q4_0.gguf",
            "mmproj-Qwen3.8-27B-Q8_0.gguf",
        ] {
            std::fs::write(qwen.join(drafter), b"GGUF").unwrap();
        }
        assert_eq!(
            hf_cached_model_file_in_dir(tmp.path(), "ggml-org/Qwen3.8-27B-GGUF:Q4_0"),
            None,
            "a drafter or projector is not the model"
        );
        std::fs::write(qwen.join("Qwen3.8-27B-Q4_K_M.gguf"), b"GGUF").unwrap();
        assert_eq!(
            hf_cached_model_file_in_dir(tmp.path(), "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M"),
            Some(qwen.join("Qwen3.8-27B-Q4_K_M.gguf"))
        );

        // Google names its file `<size>_q4_0-it.gguf`: lower case, underscore
        // before the quant, `-` after — llama.cpp's `TAG[.-]` match.
        let gemma = tmp
            .path()
            .join("models--google--gemma-4-31B-it-qat-q4_0-gguf/snapshots/59dde245");
        std::fs::create_dir_all(&gemma).unwrap();
        std::fs::write(gemma.join("gemma-4-31B-it-mmproj.gguf"), b"GGUF").unwrap();
        std::fs::write(gemma.join("gemma-4-31B_q4_0-it.gguf"), b"GGUF").unwrap();
        assert_eq!(
            hf_cached_model_file_in_dir(tmp.path(), "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0"),
            Some(gemma.join("gemma-4-31B_q4_0-it.gguf"))
        );

        assert!(gguf_name_matches_quant(
            "model-Q4_K_M-00001-of-00002.gguf",
            "Q4_K_M"
        ));
        assert!(!gguf_name_matches_quant("model-Q4_K_M_L.gguf", "Q4_K_M"));
        assert!(!is_model_gguf_name("README.md"));
    }

    #[test]
    fn cached_file_size_is_checked_against_the_pin() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("model.gguf");
        std::fs::write(&path, b"GGUF1234").unwrap();
        let expected = |size_bytes| ExpectedGguf {
            file_name: "model.gguf".to_string(),
            size_bytes,
            sha256: "00".to_string(),
        };
        assert!(cached_file_matches(&path, &expected(8)));
        assert!(!cached_file_matches(&path, &expected(9)));
    }

    #[test]
    fn delete_model_cache_removes_huggingface_repo_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("models--unsloth--model-GGUF");
        let snapshot_dir = repo_dir.join("snapshots").join("rev");
        std::fs::create_dir_all(&snapshot_dir).unwrap();
        std::fs::write(snapshot_dir.join("model-Q4_K_M.gguf"), b"model").unwrap();

        assert_eq!(
            model_cache_status_in_dir(tmp.path(), "unsloth/model-GGUF:Q4_K_M"),
            ModelCacheStatus::Downloaded
        );
        assert!(delete_model_cache_in_dir(tmp.path(), "unsloth/model-GGUF:Q4_K_M").unwrap());
        assert!(!repo_dir.exists());
        assert!(!delete_model_cache_in_dir(tmp.path(), "unsloth/model-GGUF:Q4_K_M").unwrap());
    }

    #[test]
    fn model_source_cache_status_detects_ollama_gguf_blob() {
        let tmp = tempfile::tempdir().unwrap();
        write_legacy_manifest(tmp.path(), "qwen3.8", "27b-q4_K_M", gguf_manifest());
        let source = ModelSource::ollama("qwen3.8:27b-q4_K_M", "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M");

        // Manifest written, weights still arriving: a genuine partial pull.
        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::Partial
        );

        let blob = write_blob(tmp.path(), MODEL_DIGEST, b"GGUF\x03\x00\x00\x00");
        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::Downloaded
        );
        assert_eq!(
            ollama_model_blob_path_in_dir(tmp.path(), "qwen3.8:27b-q4_K_M"),
            Some(blob)
        );
    }

    /// Only the `image.model` layer is the weights. Gemma 4's GGUF tag carries a
    /// separately typed MTP drafter and both carry a projector; picking "the
    /// largest layer" or "the first layer" would hand llama-server one of those.
    #[test]
    fn only_the_model_layer_is_ever_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        write_legacy_manifest(tmp.path(), "gemma4", "31b-it-q4_K_M", gguf_manifest());
        // The drafter and the projector are complete GGUF files on disk, and
        // larger than the model in this fixture.
        write_blob(tmp.path(), DRAFT_DIGEST, b"GGUF-draft-weights");
        write_blob(tmp.path(), PROJECTOR_DIGEST, b"GGUF-projector-weights");
        assert_eq!(
            ollama_model_blob_path_in_dir(tmp.path(), "gemma4:31b-it-q4_K_M"),
            None,
            "the drafter/projector must not stand in for missing weights"
        );

        let model = write_blob(tmp.path(), MODEL_DIGEST, b"GGUF");
        assert_eq!(
            ollama_model_blob_path_in_dir(tmp.path(), "gemma4:31b-it-q4_K_M"),
            Some(model)
        );
    }

    /// A blob that is not a GGUF (a safetensors shard, a truncated write) is
    /// never handed to `-m`.
    #[test]
    fn a_blob_without_the_gguf_magic_is_not_reusable() {
        let tmp = tempfile::tempdir().unwrap();
        write_legacy_manifest(tmp.path(), "qwen3.8", "27b-q4_K_M", gguf_manifest());
        write_blob(tmp.path(), MODEL_DIGEST, b"{\"safetensors\":1}");
        assert_eq!(
            ollama_model_blob_path_in_dir(tmp.path(), "qwen3.8:27b-q4_K_M"),
            None
        );
        assert!(!is_gguf_file(&tmp.path().join("missing")));
    }

    /// LC-7. On Apple Silicon `qwen3.8:27b` pulls as MLX safetensors. That was
    /// reported as "Partial download" — and returned before the Hugging Face
    /// cache was consulted, so a fully cached fallback was masked too.
    #[test]
    fn an_mlx_pull_is_not_partial_and_does_not_mask_the_hf_cache() {
        let tmp = tempfile::tempdir().unwrap();
        write_legacy_manifest(tmp.path(), "qwen3.8", "27b", mlx_manifest());
        // The tensor blobs are all present: the MLX pull is complete.
        write_blob(tmp.path(), MODEL_DIGEST, b"safetensors");
        write_blob(
            tmp.path(),
            LIST_DIGEST,
            manifest_list().to_string().as_bytes(),
        );
        let source = ModelSource::ollama("qwen3.8:27b", "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M");

        assert_eq!(
            ollama_model_cache_status_in_dir(tmp.path(), "qwen3.8:27b"),
            ModelCacheStatus::NotDownloaded
        );
        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::NotDownloaded
        );

        let snapshot = tmp
            .path()
            .join("models--ggml-org--Qwen3.8-27B-GGUF/snapshots/71bc7b62");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::write(snapshot.join("Qwen3.8-27B-Q4_K_M.gguf"), b"GGUF").unwrap();
        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::Downloaded,
            "the Hugging Face cache must be consulted behind an MLX pull"
        );
    }

    /// Ollama 0.40's `manifests-v2/ollama.com/...` entry for a multi-runner tag
    /// is the manifest LIST. Its `runner: llamacpp` child is reusable once it has
    /// been pulled — which is what `ollama pull` does on an Intel Mac or Linux.
    #[test]
    fn a_manifest_list_resolves_to_its_gguf_child() {
        let tmp = tempfile::tempdir().unwrap();
        let list_path = tmp
            .path()
            .join("manifests-v2/ollama.com/library/qwen3.8/27b");
        std::fs::create_dir_all(list_path.parent().unwrap()).unwrap();
        std::fs::write(&list_path, manifest_list().to_string()).unwrap();

        // The child manifest was never pulled (an Apple Silicon machine).
        assert_eq!(
            ollama_model_cache_status_in_dir(tmp.path(), "qwen3.8:27b"),
            ModelCacheStatus::NotDownloaded
        );

        write_blob(
            tmp.path(),
            GGUF_CHILD_DIGEST,
            gguf_manifest().to_string().as_bytes(),
        );
        assert_eq!(
            ollama_model_cache_status_in_dir(tmp.path(), "qwen3.8:27b"),
            ModelCacheStatus::Partial
        );
        let model = write_blob(tmp.path(), MODEL_DIGEST, b"GGUF");
        assert_eq!(
            ollama_model_blob_path_in_dir(tmp.path(), "qwen3.8:27b"),
            Some(model)
        );
    }

    /// The bare tag is MLX on a Mac, so the GGUF-only tag (`ollama pull
    /// qwen3.8:27b-q4_K_M`) is probed too, and it is what `-m` loads.
    #[test]
    fn the_gguf_only_tag_is_probed_behind_the_bare_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_string_lossy().into_owned();
        let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);
        write_legacy_manifest(tmp.path(), "qwen3.8", "27b", mlx_manifest());
        write_legacy_manifest(tmp.path(), "qwen3.8", "27b-q4_K_M", gguf_manifest());
        let model = write_blob(tmp.path(), MODEL_DIGEST, b"GGUF");
        let source = ModelSource::ollama("qwen3.8:27b", "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M")
            .with_ollama_gguf_names(["qwen3.8:27b-q4_K_M"]);

        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::Downloaded
        );
        assert_eq!(model_source_path(&source), Some(model.clone()));
        assert_eq!(launch_source_for(&source), LaunchSource::OllamaBlob(model));
    }

    /// LC-15. Nothing cached: `-hf` downloads. Cached anywhere: launch from the
    /// file with `-m`, so no network is needed and a re-upload under the same
    /// name cannot trigger a surprise download. The catalog's own Hugging Face
    /// file wins over an Ollama blob (it is the exact file the catalog pins).
    #[test]
    fn launch_source_prefers_files_already_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_string_lossy().into_owned();
        let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);
        let source = ModelSource::ollama("gemma4:31b", "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0")
            .with_ollama_gguf_names(["gemma4:31b-it-q4_K_M"]);

        assert_eq!(
            launch_source_for(&source),
            hf("google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0")
        );

        write_legacy_manifest(tmp.path(), "gemma4", "31b-it-q4_K_M", gguf_manifest());
        let blob = write_blob(tmp.path(), MODEL_DIGEST, b"GGUF");
        assert_eq!(launch_source_for(&source), LaunchSource::OllamaBlob(blob));

        let snapshot = tmp
            .path()
            .join("models--google--gemma-4-31B-it-qat-q4_0-gguf/snapshots/59dde245");
        std::fs::create_dir_all(&snapshot).unwrap();
        let cached = snapshot.join("gemma-4-31B_q4_0-it.gguf");
        std::fs::write(&cached, b"GGUF").unwrap();
        assert_eq!(launch_source_for(&source), LaunchSource::CachedFile(cached));
    }

    #[test]
    fn parse_ollama_name_applies_registry_defaults() {
        assert_eq!(
            parse_ollama_name("qwen3.8:27b"),
            Some(OllamaName {
                host: "registry.ollama.ai".to_string(),
                namespace: "library".to_string(),
                model: "qwen3.8".to_string(),
                tag: "27b".to_string(),
            })
        );
        assert_eq!(
            parse_ollama_name("team/model"),
            Some(OllamaName {
                host: "registry.ollama.ai".to_string(),
                namespace: "team".to_string(),
                model: "model".to_string(),
                tag: "latest".to_string(),
            })
        );
        assert_eq!(parse_ollama_name("../../etc:passwd"), None);
    }

    #[test]
    fn ollama_manifest_paths_cover_the_v2_store() {
        let paths = ollama_manifest_paths(Path::new("/m"), "qwen3.8:27b");
        assert!(paths.contains(&PathBuf::from(
            "/m/manifests/registry.ollama.ai/library/qwen3.8/27b"
        )));
        assert!(paths.contains(&PathBuf::from(
            "/m/manifests-v2/ollama.com/library/qwen3.8/27b"
        )));
    }

    #[test]
    fn blob_paths_cannot_leave_the_blob_directory() {
        assert_eq!(ollama_blob_path(Path::new("/m"), "sha256:../../x"), None);
        assert_eq!(ollama_blob_path(Path::new("/m"), ""), None);
        assert_eq!(
            ollama_blob_path(Path::new("/m"), "sha256:abc"),
            Some(PathBuf::from("/m/blobs/sha256-abc"))
        );
    }

    #[test]
    fn model_cache_dir_follows_ollama_models() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_string_lossy().into_owned();
        let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);

        assert_eq!(model_cache_dir(), PathBuf::from(&tmp_path));
    }

    /// LC-3. Two tiers, and no 32k one: Biorouter's instructions and tools
    /// alone are ~31-40k tokens, so a 32k window failed the first request of
    /// every chat.
    #[test]
    fn default_context_size_has_a_64k_floor_and_a_128k_tier() {
        for (gib, expected) in [
            (0, LLAMACPP_AGENT_CONTEXT_SIZE),
            (8, LLAMACPP_AGENT_CONTEXT_SIZE),
            (16, LLAMACPP_AGENT_CONTEXT_SIZE),
            (32, LLAMACPP_AGENT_CONTEXT_SIZE),
            (47, LLAMACPP_AGENT_CONTEXT_SIZE),
            (48, LLAMACPP_LARGE_CONTEXT_SIZE),
            (64, LLAMACPP_LARGE_CONTEXT_SIZE),
            (128, LLAMACPP_LARGE_CONTEXT_SIZE),
        ] {
            assert_eq!(default_context_size_for(gib), expected, "{gib} GiB");
        }
        assert_eq!(LLAMACPP_AGENT_CONTEXT_SIZE, 65_536);
        assert_eq!(LLAMACPP_LARGE_CONTEXT_SIZE, 131_072);
        assert_eq!(
            default_context_size(),
            default_context_size_for(recommendation_memory_gib())
        );
    }

    #[test]
    fn auto_context_steps_down_to_the_floor_and_no_further() {
        assert_eq!(
            next_lower_auto_context_size(LLAMACPP_LARGE_CONTEXT_SIZE),
            Some(LLAMACPP_AGENT_CONTEXT_SIZE)
        );
        assert_eq!(
            next_lower_auto_context_size(65_537),
            Some(LLAMACPP_AGENT_CONTEXT_SIZE)
        );
        assert_eq!(
            next_lower_auto_context_size(LLAMACPP_AGENT_CONTEXT_SIZE),
            None,
            "a window below the agent bootstrap starts a server that refuses every request"
        );
        assert_eq!(next_lower_auto_context_size(32_768), None);
    }

    #[test]
    fn out_of_memory_at_the_floor_explains_itself() {
        let tail = "ggml_backend_metal_buffer: failed to allocate buffer of size 2147483648";
        let message = startup_failure_message("qwen3.8-27b", Some(65_536), tail);
        assert!(message.contains("not enough memory"), "{message}");
        assert!(message.contains("qwen3.8-27b"), "{message}");
        assert!(message.contains("65536"), "{message}");
        assert!(
            message.contains(tail),
            "the raw log must survive: {message}"
        );

        let other = startup_failure_message("qwen3.8-27b", Some(65_536), "error: bad magic");
        assert_eq!(
            other,
            "llama-server exited during startup:\nerror: bad magic"
        );
    }

    #[test]
    fn parses_vram_size_strings() {
        assert_eq!(parse_memory_gib("1536 MB"), Some(2));
        assert_eq!(parse_memory_gib("5.07 GB"), Some(6));
        assert_eq!(parse_memory_gib("1 TB"), Some(1024));
        assert_eq!(parse_memory_gib("Built-In"), None);
    }

    /// LC-9. Upstream builds the macOS x64 archive without Metal, so an Intel
    /// Mac is budgeted by system RAM and labelled CPU-only.
    #[test]
    fn this_machine_reports_its_accelerator_honestly() {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert!(!is_cpu_only());
            assert_eq!(accelerator_memory_kind(), "apple_unified");
        }
        if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            assert!(is_cpu_only());
            assert_eq!(accelerator_memory_kind(), "cpu_only");
            assert_eq!(accelerator_memory_gib(), Some(total_memory_gib()));
        }
    }

    #[test]
    fn build_args_includes_model_port_and_jinja() {
        let _guard = env_lock::lock_env([
            ("LLAMACPP_ENABLE_THINKING", None::<&str>),
            ("LLAMACPP_PARALLEL", None),
            ("LLAMACPP_EXTRA_ARGS", None),
        ]);
        let source = hf("google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0");
        let args = build_args(&source, "gemma4-31b", 12345, 65_536, &options());
        let joined = args.join(" ");
        assert!(joined.contains("-hf google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0"));
        assert!(joined.contains("--alias gemma4-31b"));
        assert!(joined.contains("--port 12345"));
        assert!(joined.contains("--ctx-size 65536"));
        assert!(joined.contains("--cache-type-k q8_0"));
        assert!(joined.contains("--cache-type-v q8_0"));
        assert!(joined.contains("--jinja"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--host").count(),
            1
        );
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--port").count(),
            1
        );
        // b11429 deprecates `--no-webui` in favour of `--no-ui`.
        assert!(joined.contains("--no-ui"));
        assert!(!joined.contains("--no-webui"));
        assert!(joined.contains("--no-slots"));
        assert!(joined.contains("--no-mmproj"));
        // Off for an unadorned request; each request's `enable_thinking` wins.
        assert!(joined.contains("--reasoning off"));
        assert!(!joined.contains("enable_thinking"));
        // No key file was given, so none is passed.
        assert!(!joined.contains("--api-key"));
    }

    /// LC-2. llama-server's default is 4 slots sharing one KV cache; two
    /// concurrent agent requests then both failed with HTTP 500.
    #[test]
    fn build_args_runs_one_slot_unless_configured() {
        let _guard = env_lock::lock_env([("LLAMACPP_PARALLEL", None::<&str>)]);
        let args = build_args(&hf("o/r:Q4_K_M"), "m", 11543, 65_536, &options());
        assert!(args.join(" ").contains("--parallel 1"), "{args:?}");
        drop(_guard);

        let _guard = env_lock::lock_env([("LLAMACPP_PARALLEL", Some("2"))]);
        let args = build_args(&hf("o/r:Q4_K_M"), "m", 11543, 131_072, &options());
        assert!(args.join(" ").contains("--parallel 2"), "{args:?}");
    }

    #[test]
    fn build_args_explicit_ctx_size_is_passed_through() {
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 16384, &options());
        assert!(args.join(" ").contains("--ctx-size 16384"));
    }

    #[test]
    fn build_args_pins_reasoning_on_when_enabled() {
        let _guard = env_lock::lock_env([("LLAMACPP_ENABLE_THINKING", Some("true"))]);
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &options());
        assert!(args.join(" ").contains("--reasoning on"));
        drop(_guard);

        let _guard = env_lock::lock_env([("LLAMACPP_ENABLE_THINKING", Some("false"))]);
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &options());
        assert!(args.join(" ").contains("--reasoning off"));
    }

    /// LC-5. The key file goes after the user's extra arguments, and before
    /// the loopback re-assertion.
    #[test]
    fn build_args_passes_the_key_file_after_extra_args() {
        let _guard = env_lock::lock_env([(
            "LLAMACPP_EXTRA_ARGS",
            Some("--api-key-file /tmp/attacker.key --threads 4"),
        )]);
        let key_file = PathBuf::from("/run/biorouter/42.key");
        let args = build_args(
            &hf("owner/repo:Q4_K_M"),
            "m",
            11543,
            65_536,
            &LaunchOptions {
                spec_type_default: None,
                api_key_file: Some(&key_file),
            },
        );
        assert_eq!(
            args.iter()
                .filter(|a| a.as_str() == "--api-key-file")
                .count(),
            1,
            "{args:?}"
        );
        let at = args.iter().position(|a| a == "--api-key-file").unwrap();
        assert_eq!(args[at + 1], "/run/biorouter/42.key");
        assert!(!args.iter().any(|a| a.contains("attacker")));
        assert!(at > args.iter().position(|a| a == "--threads").unwrap());
        assert!(at < args.iter().rposition(|a| a == "--host").unwrap());
    }

    /// The readiness budget is a NO-PROGRESS budget. A first run downloads
    /// several GB, which outlasts the wall-clock ceiling every caller passes,
    /// so the marker has to move whenever llama-server narrates anything —
    /// including a rewritten progress line, which does not change the line
    /// count.
    #[test]
    fn progress_marker_moves_on_new_and_on_rewritten_lines() {
        let tail: Arc<StdMutex<VecDeque<String>>> = Arc::new(StdMutex::new(VecDeque::new()));
        let empty = progress_marker(&tail);

        push_log(&tail, "downloading 10%".to_string());
        let first = progress_marker(&tail);
        assert_ne!(empty, first, "a first line must count as progress");

        push_log(&tail, "loading model".to_string());
        let second = progress_marker(&tail);
        assert_ne!(first, second, "a new line must count as progress");

        {
            let mut guard = tail.lock().unwrap();
            let last = guard.back_mut().unwrap();
            *last = "loading model (42%)".to_string();
        }
        let third = progress_marker(&tail);
        assert_ne!(
            second, third,
            "a rewritten last line must count as progress"
        );
        assert_eq!(third, progress_marker(&tail), "no change must be stable");
    }

    #[test]
    fn build_args_uses_the_models_own_spec_type_default() {
        let _guard = env_lock::lock_env([("LLAMACPP_SPEC_TYPE", None::<&str>)]);
        OPTIONAL_FLAGS_DISABLED.store(false, Ordering::Relaxed);
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &options());
        assert!(
            args.join(" ")
                .contains(&format!("--spec-type {DEFAULT_SPEC_TYPE}")),
            "a raw spec keeps the default: {args:?}"
        );

        let no_default = LaunchOptions {
            spec_type_default: None,
            api_key_file: None,
        };
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &no_default);
        assert!(
            !args.iter().any(|a| a == "--spec-type"),
            "a catalog entry measured without speculation passes no flag: {args:?}"
        );
        drop(_guard);

        // An explicit setting wins over the model's default, in both directions.
        let _guard = env_lock::lock_env([("LLAMACPP_SPEC_TYPE", Some("ngram-mod"))]);
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &no_default);
        assert!(args.join(" ").contains("--spec-type ngram-mod"), "{args:?}");
    }

    #[test]
    fn build_args_omits_spec_type_when_disabled() {
        for value in ["none", "NONE", ""] {
            let _guard = env_lock::lock_env([("LLAMACPP_SPEC_TYPE", Some(value))]);
            OPTIONAL_FLAGS_DISABLED.store(false, Ordering::Relaxed);
            let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &options());
            assert!(
                !args.iter().any(|a| a == "--spec-type"),
                "expected no --spec-type for {value:?}, got {args:?}"
            );
        }
    }

    #[test]
    fn build_args_drops_spec_type_once_the_binary_has_rejected_it() {
        let _guard = env_lock::lock_env([("LLAMACPP_SPEC_TYPE", None::<&str>)]);
        OPTIONAL_FLAGS_DISABLED.store(true, Ordering::Relaxed);
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &options());
        OPTIONAL_FLAGS_DISABLED.store(false, Ordering::Relaxed);
        assert!(
            !args.iter().any(|a| a == "--spec-type"),
            "a rejected flag must not be re-emitted: {args:?}"
        );
    }

    #[test]
    fn unsupported_argument_is_told_apart_from_a_memory_failure() {
        for log in [
            "error: invalid argument: --spec-type",
            "error: unknown argument: --spec-type",
            "error while handling argument \"--spec-type\"",
            "invalid value for --spec-type",
        ] {
            assert!(looks_like_unsupported_argument(log), "should match: {log}");
            assert!(
                !looks_like_memory_failure(log),
                "argv error must not read as a memory failure: {log}"
            );
        }
        for log in [
            "ggml_backend_metal_buffer: failed to allocate buffer",
            "llama_model_load: error loading model: out of memory",
        ] {
            assert!(
                !looks_like_unsupported_argument(log),
                "memory failure must not read as an argv error: {log}"
            );
            assert!(looks_like_memory_failure(log), "should match: {log}");
        }
    }

    #[test]
    fn a_metal_out_of_memory_reads_as_a_memory_failure() {
        let tail =
            "E error: Insufficient Memory (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)\n\
                    E ggml_metal_graph_compute: backend is in error state from a previous command \
                    buffer failure - recreate the backend to recover";
        assert!(looks_like_memory_failure(tail));
        let message = backend_failure_message("gemma4-31b", Some(131_072), tail);
        assert!(message.contains("ran out of GPU memory"), "{message}");
        assert!(
            message.contains("Biorouter stopped that server"),
            "{message}"
        );
        let other = backend_failure_message("gemma4-31b", Some(131_072), "E decode: failed");
        assert!(other.contains("E decode: failed"), "{other}");

        assert!(is_compute_error(
            "Server error (500 Internal Server Error): Compute error."
        ));
        assert!(!is_compute_error(
            "Server error (500 Internal Server Error): Context size has been exceeded."
        ));
    }

    /// The chat shows a compute failure right before "Trying again (1/1)…", so
    /// it must not also tell the person to try again (live, against a scripted
    /// llama-server: "…has been restarted; try again. … Trying again (1/1)…").
    #[test]
    fn a_compute_failure_does_not_ask_the_person_to_try_again() {
        let message = backend_failure_message("qwen3.8-27b", Some(131_072), "E decode: failed");
        assert!(
            message.starts_with(
                "Llama Server's GPU backend failed while running qwen3.8-27b. Biorouter stopped \
                 that server, and the next request starts a fresh one."
            ),
            "{message}"
        );
        assert!(!message.to_lowercase().contains("try again"), "{message}");
        assert!(
            message.ends_with("llama-server output:\nE decode: failed"),
            "{message}"
        );
    }

    /// The out-of-memory notice is a compute-failure notice too, shown right
    /// before "Trying again (1/1)…": it keeps its advice but must not ask the
    /// person to try again, nor say a server was restarted that was stopped.
    #[test]
    fn an_out_of_memory_failure_does_not_ask_the_person_to_try_again() {
        let tail =
            "E error: Insufficient Memory (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)";
        for window in [Some(131_072), Some(65_536), None] {
            let message = backend_failure_message("qwen3.8-27b", window, tail);
            assert!(
                message.starts_with(
                    "Llama Server ran out of GPU memory while running qwen3.8-27b. Biorouter \
                     stopped that server, and the next request starts a fresh one. Quit other \
                     apps or local models that hold GPU memory"
                ),
                "{window:?}: {message}"
            );
            assert!(
                !message.to_lowercase().contains("try again"),
                "{window:?}: {message}"
            );
            assert!(!message.contains("restarted"), "{window:?}: {message}");
        }
    }

    /// A GPU out-of-memory mid-request offers a smaller window only while there
    /// is one above the floor, and names the floor when it does. The old advice
    /// ("set a smaller LLAMACPP_CONTEXT_SIZE") sent a machine already at 64k to
    /// 32k, where every agent request is refused as too long.
    #[test]
    fn out_of_memory_advice_never_goes_below_the_floor() {
        let tail =
            "E error: Insufficient Memory (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)";

        let above = backend_failure_message("qwen3.8-27b", Some(131_072), tail);
        assert!(
            above.contains("LLAMACPP_CONTEXT_SIZE to 65536 (not lower"),
            "{above}"
        );

        for at_or_below in [Some(65_536), Some(32_768), None] {
            let message = backend_failure_message("qwen3.8-27b", at_or_below, tail);
            assert!(
                !message.contains("set LLAMACPP_CONTEXT_SIZE"),
                "{at_or_below:?}: {message}"
            );
            assert!(
                message.contains("A smaller context window will not help"),
                "{message}"
            );
            assert!(message.contains("65536"), "{message}");
        }
        for window in [Some(131_072), Some(65_536)] {
            let message = backend_failure_message("qwen3.8-27b", window, tail);
            assert!(
                !message.contains("smaller LLAMACPP_CONTEXT_SIZE"),
                "{message}"
            );
            assert!(!message.contains("  "), "{message}");
        }
    }

    /// Only an error in one of the optional flags switches them off. Any argv
    /// error used to, process-wide and for good, so a typo in
    /// `LLAMACPP_EXTRA_ARGS` cost speculative decoding until a restart even
    /// after it was fixed. The two error shapes are b11429's own, captured from
    /// the pinned binary.
    #[test]
    fn only_an_optional_flag_error_switches_the_optional_flags_off() {
        assert_eq!(
            rejected_argument("error: invalid argument: --threads=8").as_deref(),
            Some("--threads=8")
        );
        assert_eq!(
            rejected_argument(
                "error while handling argument \"--flash-attn\": error: unknown value for \
                 --flash-attn: 'maybe'\n\nusage:\n-fa,   --flash-attn [on|off|auto]"
            )
            .as_deref(),
            Some("--flash-attn")
        );

        for ours in [
            "error: invalid argument: --spec-type",
            "error while handling argument \"--spec-type\": unknown speculative type: ngram-simple",
            // A message that does not say which argument keeps the old safety net.
            "error: unknown argument",
        ] {
            assert!(rejects_an_optional_flag(ours), "{ours}");
        }
        for theirs in [
            "error: invalid argument: --threads=8",
            "error: invalid argument: --bogus-flag",
            "error while handling argument \"--flash-attn\": error: unknown value for \
             --flash-attn: 'maybe'",
            "ggml_backend_metal_buffer: failed to allocate buffer",
        ] {
            assert!(!rejects_an_optional_flag(theirs), "{theirs}");
        }

        // The user is told which argument, and where it probably came from.
        let message = startup_failure_message(
            "qwen3.8-27b",
            Some(65_536),
            "error: invalid argument: --threads=8",
        );
        assert!(message.contains("'--threads=8'"), "{message}");
        assert!(message.contains("LLAMACPP_EXTRA_ARGS"), "{message}");
    }

    #[test]
    fn detects_model_load_failure_log_fragments() {
        assert!(looks_like_model_load_failure(
            "wrong number of tensors; expected 2131, got 720"
        ));
        assert!(looks_like_model_load_failure("error loading model"));
        assert!(!looks_like_model_load_failure("download failed with 404"));
    }

    /// LC-6. An allowlist, not a blocklist: `--tools` (llama-server's built-in
    /// shell, file-write and file-edit tools) was missing from the old
    /// blocklist, and config.yaml is agent-writable.
    #[test]
    fn extra_args_keep_only_performance_and_sampling_flags() {
        for refused in [
            "--tools all",
            "--tools=exec_shell_command",
            "--agent",
            "--ui-mcp-proxy",
            "--ui",
            "--host 0.0.0.0",
            "--port 9999",
            "--api-key secret",
            "--api-key-file /tmp/k",
            "--path /",
            "--media-path /Users",
            "--slot-save-path /tmp",
            "--models-dir /tmp",
            "--props",
            "--rpc 1.2.3.4:50052",
            "--ssl-key-file k.pem",
            "--context-shift",
            "--spec-draft-hf evil/model:Q4_0",
            "--spec-draft-model /tmp/x.gguf",
            // llama.cpp reads `_` as `-` in a long flag, so these ARE the two
            // above; b11429 accepts `--spec-draft_model` (measured).
            "--spec-draft_hf evil/model:Q4_0",
            "--spec-draft_model /tmp/x.gguf",
            "--spec_draft_hf evil/model:Q4_0",
            "--spec-draft_hf=evil/model:Q4_0",
            "--spec-draft_override_tensor x=CPU",
            "--api_key_file /tmp/k",
            "--no_ui",
            // A value the flag does not take, or none at all.
            "--flash-attn=maybe",
            "--no-warmup=1",
            "--threads=",
            "--threads",
            "-m /tmp/other.gguf",
            "-hf evil/model",
            "--chat-template-kwargs {}",
            "--reuse-port",
        ] {
            assert_eq!(
                sanitize_extra_args(refused),
                Vec::<String>::new(),
                "{refused} must be refused"
            );
        }

        // A refused flag takes its value with it, so the value cannot become an
        // argument of whatever comes next.
        assert_eq!(
            sanitize_extra_args("--tools all --threads 8"),
            vec!["--threads", "8"]
        );
        assert_eq!(
            sanitize_extra_args("--host=0.0.0.0 --threads 8"),
            vec!["--threads", "8"]
        );

        for (allowed, expected) in [
            ("--threads 8", vec!["--threads", "8"]),
            ("-ngl -1", vec!["-ngl", "-1"]),
            ("--n-gpu-layers 99", vec!["--n-gpu-layers", "99"]),
            ("--flash-attn on", vec!["--flash-attn", "on"]),
            (
                "--flash-attn --parallel 4",
                vec!["--flash-attn", "--parallel", "4"],
            ),
            ("-b 1024 -ub 256", vec!["-b", "1024", "-ub", "256"]),
            ("--cache-type-k f16", vec!["--cache-type-k", "f16"]),
            ("--load-mode mlock", vec!["--load-mode", "mlock"]),
            ("--ctx-checkpoints 8", vec!["--ctx-checkpoints", "8"]),
            // b11429 rejects every `--flag=value` ("invalid argument:
            // --threads=8"), so the `=` form is passed as two tokens.
            ("--cache-ram=4096", vec!["--cache-ram", "4096"]),
            ("--threads=8", vec!["--threads", "8"]),
            ("--flash-attn=on", vec!["--flash-attn", "on"]),
            ("-ngl=-1", vec!["-ngl", "-1"]),
            // An underscore spelling of an allowed flag is passed in the
            // spelling that was checked.
            ("--cache_ram 4096", vec!["--cache-ram", "4096"]),
            (
                "--spec-ngram-simple_size-n 8",
                vec!["--spec-ngram-simple-size-n", "8"],
            ),
            ("--no-warmup", vec!["--no-warmup"]),
            ("--temp 0.6", vec!["--temp", "0.6"]),
            (
                "--spec-ngram-simple-size-n 8",
                vec!["--spec-ngram-simple-size-n", "8"],
            ),
            (
                "--spec-default --threads 4",
                vec!["--spec-default", "--threads", "4"],
            ),
        ] {
            assert_eq!(sanitize_extra_args(allowed), expected, "{allowed}");
        }
    }

    #[test]
    fn build_args_reasserts_loopback_last_even_with_injected_host() {
        let _guard =
            env_lock::lock_env([("LLAMACPP_EXTRA_ARGS", Some("--host 0.0.0.0 --port 9999"))]);
        let args = build_args(&hf("owner/repo:Q4_K_M"), "m", 11543, 65_536, &options());
        assert!(!args.iter().any(|a| a == "0.0.0.0"));
        let last_host = args.iter().rposition(|a| a == "--host").unwrap();
        assert_eq!(args[last_host + 1], "127.0.0.1");
        let last_port = args.iter().rposition(|a| a == "--port").unwrap();
        assert_eq!(args[last_port + 1], "11543");
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--host").count(),
            1
        );
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--port").count(),
            1
        );
    }

    /// LC-6. llama-server reads nearly every flag from a `LLAMA_ARG_*`
    /// variable too, so the environment is a second way in.
    #[test]
    fn the_child_environment_is_scrubbed() {
        for stripped in [
            "LLAMA_ARG_TOOLS",
            "LLAMA_ARG_HOST",
            "LLAMA_ARG_UI_MCP_PROXY",
            "LLAMA_API_KEY",
            "llama_arg_tools",
            "HF_TOKEN",
            "HUGGING_FACE_HUB_TOKEN",
        ] {
            assert!(should_strip_child_env(OsStr::new(stripped)), "{stripped}");
        }
        for kept in ["PATH", "HOME", "OLLAMA_MODELS", "LLAMACPP_PORT", "TMPDIR"] {
            assert!(!should_strip_child_env(OsStr::new(kept)), "{kept}");
        }

        let _guard = env_lock::lock_env([
            ("LLAMA_ARG_TOOLS", Some("all")),
            ("HF_TOKEN", Some("hf_users_own_token")),
            ("LLAMACPP_HF_TOKEN", None),
        ]);
        let mut command = Command::new("llama-server");
        // No configured token. (`configured_hf_token` is not asked here: with
        // the variable unset it would read the real secret store.)
        scrub_child_env(&mut command, None);
        let envs: Vec<_> = command.as_std().get_envs().collect();
        assert!(
            envs.contains(&(OsStr::new("LLAMA_ARG_TOOLS"), None)),
            "{envs:?}"
        );
        assert!(envs.contains(&(OsStr::new("HF_TOKEN"), None)), "{envs:?}");
    }

    #[test]
    fn a_configured_sidecar_token_is_the_only_one_passed() {
        let _guard = env_lock::lock_env([
            ("HF_TOKEN", Some("hf_users_own_token")),
            ("LLAMACPP_HF_TOKEN", Some("hf_sidecar_token")),
        ]);
        // The environment answers before the secret store is consulted.
        let token = configured_hf_token();
        assert_eq!(token.as_deref(), Some("hf_sidecar_token"));
        let mut command = Command::new("llama-server");
        scrub_child_env(&mut command, token.as_deref());
        let envs: Vec<_> = command.as_std().get_envs().collect();
        assert!(
            envs.contains(&(OsStr::new("HF_TOKEN"), Some(OsStr::new("hf_sidecar_token")))),
            "{envs:?}"
        );
    }

    /// LC-5. A fresh 256-bit key per launch, in a file only this user can read.
    #[test]
    fn api_key_file_is_private_and_fresh() {
        let a = generate_api_key();
        let b = generate_api_key();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);

        let tmp = tempfile::tempdir().unwrap();
        // A file left with looser permissions is replaced, not rewritten.
        let path = key_file_path_in(tmp.path());
        std::fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let written = write_api_key_file_in(tmp.path(), &a).unwrap();
        assert_eq!(written, path);
        assert_eq!(std::fs::read_to_string(&written).unwrap().trim(), a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&written).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{mode:o}");
        }
    }

    #[test]
    fn reaping_removes_a_dead_parents_files_and_keeps_a_live_ones() {
        let tmp = tempfile::tempdir().unwrap();
        // A pid that existed and is gone.
        let mut exited = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" });
        if cfg!(windows) {
            exited.args(["/C", "exit"]);
        }
        let mut exited = exited.spawn().unwrap();
        let dead = exited.id();
        exited.wait().unwrap();
        let live = std::process::id();

        let dead_key = tmp.path().join(format!("{dead}.key"));
        let dead_pid = tmp.path().join(format!("{dead}.pid"));
        let live_key = tmp.path().join(format!("{live}.key"));
        let live_pid = tmp.path().join(format!("{live}.pid"));
        std::fs::write(&dead_key, "k").unwrap();
        // Its child is long gone too, so nothing is killed.
        std::fs::write(&dead_pid, dead.to_string()).unwrap();
        std::fs::write(&live_key, "k").unwrap();
        std::fs::write(&live_pid, "1").unwrap();

        assert!(!reap_orphans_in(tmp.path()));
        assert!(!dead_key.exists());
        assert!(!dead_pid.exists());
        assert!(live_key.exists(), "a live process's key is never touched");
        assert!(live_pid.exists());
    }

    #[tokio::test]
    async fn finish_log_tasks_waits_for_pending_output() {
        let tail = Arc::new(StdMutex::new(VecDeque::new()));
        let task_tail = tail.clone();
        let mut tasks = vec![tokio::spawn(async move {
            tokio::task::yield_now().await;
            push_log(&task_tail, "final stderr line".to_string());
        })];

        finish_log_tasks(&mut tasks).await;

        assert!(tasks.is_empty());
        assert_eq!(last_log(&tail).as_deref(), Some("final stderr line"));
    }

    #[test]
    fn build_args_can_launch_from_a_file_on_disk() {
        for source in [
            LaunchSource::OllamaBlob(PathBuf::from("/tmp/ollama/models/blobs/sha256-abc")),
            LaunchSource::CachedFile(PathBuf::from(
                "/tmp/ollama/models/models--o--r/snapshots/x/r-Q4_K_M.gguf",
            )),
        ] {
            let path = source.model_path().unwrap();
            let args = build_args(&source, "qwen3.8-27b", 11543, 65_536, &options());
            let joined = args.join(" ");
            assert!(joined.contains(&format!("-m {path}")), "{joined}");
            assert!(!joined.contains("-hf"), "{joined}");
            assert!(joined.contains("--alias qwen3.8-27b"));
        }
    }

    fn sidecar_for_test() -> LlamaSidecar {
        LlamaSidecar {
            inner: Mutex::new(Inner {
                child: None,
                log_tasks: Vec::new(),
                port: None,
                model: None,
                model_source: None,
                launch_source: None,
                log_tail: Arc::new(StdMutex::new(VecDeque::new())),
                last_error: None,
                context_size: None,
                requested_context_size: None,
                automatic_context_size: false,
                partial_download_restart_attempted: false,
                warmed_model: None,
                restarting: false,
                request_slots: Arc::new(tokio::sync::Semaphore::new(LLAMACPP_DEFAULT_PARALLEL)),
            }),
            startup: Mutex::new(()),
            switch: Arc::new(tokio::sync::RwLock::new(())),
        }
    }

    /// A sidecar "serving" `model` on `port` with `slots` request slots, its
    /// child a `sleep` that stands in for llama-server.
    #[cfg(unix)]
    async fn serving_sidecar(model: &str, port: u16, slots: usize) -> LlamaSidecar {
        let sidecar = sidecar_for_test();
        {
            let mut inner = sidecar.inner.lock().await;
            inner.child = Some(
                Command::new("sleep")
                    .arg("30")
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap(),
            );
            inner.model = Some(model.to_string());
            inner.port = Some(port);
            inner.request_slots = Arc::new(tokio::sync::Semaphore::new(slots));
        }
        sidecar
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_startup_error_cannot_restart_a_healthy_child() {
        let sidecar = serving_sidecar("gemma4-31b", 4242, 1).await;
        let (slots, pid) = {
            let mut inner = sidecar.inner.lock().await;
            inner.automatic_context_size = true;
            inner.requested_context_size = Some(LLAMACPP_LARGE_CONTEXT_SIZE);
            (
                inner.request_slots.clone(),
                inner.child.as_ref().unwrap().id(),
            )
        };

        for tail in [
            "ggml_backend_metal_buffer_type_alloc_buffer: error: failed to allocate buffer",
            "error: invalid argument: --spec-type",
        ] {
            assert!(matches!(
                sidecar.recover_exited_server(tail, &slots).await.unwrap(),
                StartupRecovery::Obsolete
            ));
        }
        let mut inner = sidecar.inner.lock().await;
        assert_eq!(inner.child.as_ref().unwrap().id(), pid);
        assert!(inner.child.as_mut().unwrap().try_wait().unwrap().is_none());
        assert_eq!(
            inner.requested_context_size,
            Some(LLAMACPP_LARGE_CONTEXT_SIZE)
        );
        assert!(!inner.restarting);
        assert!(inner.last_error.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_startup_snapshot_cannot_change_the_replacement_server() {
        let sidecar = sidecar_for_test();
        let old_slots = sidecar.inner.lock().await.request_slots.clone();
        {
            let mut inner = sidecar.inner.lock().await;
            let mut child = Command::new("true").spawn().unwrap();
            child.wait().await.unwrap();
            inner.child = Some(child);
            inner.port = Some(4242);
            inner.model = Some("gemma4-31b".to_string());
            inner.request_slots = Arc::new(tokio::sync::Semaphore::new(1));
        }
        assert!(matches!(
            sidecar
                .recover_exited_server("the previous model failed", &old_slots)
                .await
                .unwrap(),
            StartupRecovery::Obsolete
        ));
        let inner = sidecar.inner.lock().await;
        assert!(inner.last_error.is_none());
        assert!(!inner.restarting);
    }

    #[tokio::test]
    async fn an_absent_child_finishes_startup_instead_of_rechecking_forever() {
        let sidecar = sidecar_for_test();
        {
            let mut inner = sidecar.inner.lock().await;
            inner.port = Some(4242);
            inner.model = Some("gemma4-31b".to_string());
        }
        let error =
            tokio::time::timeout(Duration::from_secs(1), sidecar.wait_ready(Duration::ZERO))
                .await
                .expect("a missing child is terminal")
                .unwrap_err();
        assert!(
            error.to_string().contains("exited during startup"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_exited_child_with_the_same_snapshot_can_step_down_and_restart() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("fake-llama-server");
        std::fs::write(&binary, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _root = crate::test_sandbox::relocate_path_root_and(
            tmp.path().to_str().unwrap(),
            [
                ("BIOROUTER_LLAMACPP_BIN", binary.to_str()),
                ("OLLAMA_MODELS", Some(tmp.path().to_str().unwrap())),
                ("LLAMACPP_HF_TOKEN", Some("")),
                ("LLAMACPP_EXTRA_ARGS", None),
                ("LLAMACPP_CONTEXT_SIZE", None),
            ],
        );
        let sidecar = sidecar_for_test();
        let slots = {
            let mut inner = sidecar.inner.lock().await;
            let mut child = Command::new("true").spawn().unwrap();
            child.wait().await.unwrap();
            inner.child = Some(child);
            inner.port = Some(4242);
            inner.model = Some("gemma4-31b".to_string());
            inner.model_source = Some(ModelSource::huggingface("example/model:Q4_K_M"));
            inner.launch_source = Some(hf("example/model:Q4_K_M"));
            inner.automatic_context_size = true;
            inner.requested_context_size = Some(LLAMACPP_LARGE_CONTEXT_SIZE);
            inner.request_slots.clone()
        };

        assert!(matches!(
            sidecar
                .recover_exited_server(
                    "ggml_backend_metal_buffer_type_alloc_buffer: error: failed to allocate buffer",
                    &slots,
                )
                .await
                .unwrap(),
            StartupRecovery::Retried
        ));
        {
            let mut inner = sidecar.inner.lock().await;
            assert_eq!(
                inner.requested_context_size,
                Some(LLAMACPP_AGENT_CONTEXT_SIZE)
            );
            assert!(!Arc::ptr_eq(&inner.request_slots, &slots));
            assert!(inner.child.as_mut().unwrap().try_wait().unwrap().is_none());
            assert!(!inner.restarting);
        }
        sidecar.stop().await;
    }

    /// LC-10. A model switch waits for in-flight requests on the old model, and
    /// a lease is only granted for the model actually being served.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_lease_holds_off_a_model_switch() {
        // A server with two slots, so two leases can be held at once.
        let sidecar = serving_sidecar("qwen3.8-27b", 4242, 2).await;

        assert!(
            sidecar.try_lease("gemma4-31b", 4242).await.is_none(),
            "no lease for a model the server is not serving"
        );
        assert!(sidecar.try_lease("qwen3.8-27b", 1).await.is_none());

        let lease = sidecar.try_lease("qwen3.8-27b", 4242).await.unwrap();
        assert_eq!(lease.port(), 4242);
        // Two requests on the same model share it.
        let second = sidecar.try_lease("qwen3.8-27b", 4242).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sidecar.switch.write())
                .await
                .is_err(),
            "a switch must wait while a request holds the model"
        );
        drop(lease);
        drop(second);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), sidecar.switch.write())
                .await
                .is_ok(),
            "and proceed once every request has finished"
        );

        // A dead child is never leased.
        let mut inner = sidecar.inner.lock().await;
        let mut child = inner.child.take().unwrap();
        child.kill().await.unwrap();
        inner.child = Some(child);
        drop(inner);
        assert!(sidecar.try_lease("qwen3.8-27b", 4242).await.is_none());
    }

    /// The finding behind the slots: with `--parallel 1`, a second request was
    /// queued INSIDE llama-server, which sends no headers until a slot takes
    /// the task, and reqwest's read timeout is not reset before the headers —
    /// so a request queued behind a generation longer than `LLAMACPP_TIMEOUT`
    /// timed out on a healthy server and was retried into the same queue. Now
    /// the second lease is not granted (and the request not sent) until the
    /// first is gone, and a model switch queued meanwhile still gets its turn.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_request_without_a_free_slot_waits_for_it_in_process() {
        let sidecar = serving_sidecar("qwen3.8-27b", 4242, 1).await;
        let first = sidecar.try_lease("qwen3.8-27b", 4242).await.unwrap();

        let second = sidecar.try_lease("qwen3.8-27b", 4242);
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut second)
                .await
                .is_err(),
            "one slot, and it is taken: the second request must wait here"
        );

        // A switch arriving now waits for both requests, and closes no cycle
        // with the one waiting for a slot.
        let switch = sidecar.switch.write();
        tokio::pin!(switch);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut switch)
                .await
                .is_err()
        );

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(5), &mut second)
            .await
            .expect("the slot is handed on once the first request ends")
            .expect("the server is still the one serving the model");
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut switch)
                .await
                .is_err(),
            "the switch still waits for the request now holding the slot"
        );
        drop(second);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), &mut switch)
                .await
                .is_ok(),
            "and proceeds once it ends"
        );
    }

    /// A server discarded after a GPU failure (which takes no switch lock)
    /// while a request waited for its slot is not handed that request: `None`
    /// sends `lease` round again to start a fresh server.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_slot_on_a_discarded_server_is_not_a_lease() {
        let sidecar = serving_sidecar("gemma4-31b", 4242, 1).await;
        let first = sidecar.try_lease("gemma4-31b", 4242).await.unwrap();
        let second = sidecar.try_lease("gemma4-31b", 4242);
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut second)
                .await
                .is_err()
        );

        assert!(sidecar.discard_failed_backend(4242).await.is_some());
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(5), &mut second)
            .await
            .expect("the wait ends");
        assert!(second.is_none(), "a dead server is never leased");
    }

    #[test]
    fn the_slot_count_is_the_servers_own() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(parallel_slots_in(&args(&["--parallel", "1"])), 1);
        assert_eq!(parallel_slots_in(&args(&["--parallel", "2"])), 2);
        // The last one wins, as in llama.cpp.
        assert_eq!(
            parallel_slots_in(&args(&["--parallel", "1", "--jinja", "-np", "3"])),
            3
        );
        // Auto, or nothing readable: one, which can only make requests wait.
        assert_eq!(parallel_slots_in(&args(&["--parallel", "-1"])), 1);
        assert_eq!(parallel_slots_in(&args(&["--jinja"])), 1);

        let _guard = env_lock::lock_env([
            ("LLAMACPP_PARALLEL", None::<&str>),
            ("LLAMACPP_EXTRA_ARGS", Some("-np 3")),
        ]);
        let built = build_args(&hf("o/r:Q4_K_M"), "m", 11543, 65_536, &options());
        assert_eq!(parallel_slots_in(&built), 3, "{built:?}");
    }

    /// A server whose GPU backend failed is killed, so the next request starts
    /// a fresh one instead of erroring until the app restarts.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_backend_is_discarded() {
        let sidecar = sidecar_for_test();
        {
            let mut inner = sidecar.inner.lock().await;
            inner.child = Some(
                Command::new("sleep")
                    .arg("30")
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap(),
            );
            inner.model = Some("gemma4-31b".to_string());
            inner.port = Some(4242);
            push_log(
                &inner.log_tail,
                "E error: Insufficient Memory (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)"
                    .to_string(),
            );
        }
        assert_eq!(
            sidecar.discard_failed_backend(1).await,
            None,
            "another port"
        );
        let message = sidecar.discard_failed_backend(4242).await.unwrap();
        assert!(message.contains("ran out of GPU memory"), "{message}");
        let inner = sidecar.inner.lock().await;
        assert!(inner.child.is_none());
        assert_eq!(inner.last_error.as_deref(), Some(message.as_str()));
        drop(inner);
        assert_eq!(
            sidecar.discard_failed_backend(4242).await,
            None,
            "nothing left to kill"
        );
    }

    #[test]
    fn find_binary_env_override() {
        let _guard =
            env_lock::lock_env([("BIOROUTER_LLAMACPP_BIN", Some("/nonexistent/llama-server"))]);
        // Nonexistent override is skipped, not an error.
        let _ = find_binary();
    }

    #[test]
    fn free_port_is_nonzero() {
        assert!(free_port().unwrap() > 0);
    }
}
