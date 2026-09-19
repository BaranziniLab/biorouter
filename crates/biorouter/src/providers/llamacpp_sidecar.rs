//! Managed `llama-server` sidecar process for the Llama Server provider.
//!
//! Biorouter bundles a pinned llama.cpp `llama-server` binary (build
//! [`LLAMA_SERVER_BUILD`]) next to its own binaries. This module owns the
//! lifecycle of that process: locating the binary, spawning it with the
//! requested model (prefering an Ollama-style local model blob when present,
//! otherwise falling back to llama.cpp's Hugging Face `-hf` downloader),
//! waiting for readiness via `GET /health`, exposing a status snapshot for
//! the GUI/CLI, and restarting when the requested model changes or the
//! process dies.
//!
//! The process is a singleton per Biorouter process ([`global`]): llama-server
//! loads one model at a time, mirroring how Ollama schedules a model per
//! request stream. Switching models restarts the sidecar.

use std::collections::VecDeque;
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
/// with `ui/desktop/scripts/fetch-llama-server.js`.
pub const LLAMA_SERVER_BUILD: &str = "b9611";
/// Default port the managed sidecar listens on (loopback only).
pub const LLAMACPP_DEFAULT_PORT: u16 = 11543;
pub const LLAMACPP_AGENT_CONTEXT_SIZE: usize = 65_536;

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
    pub ollama_name: Option<String>,
}

impl ModelSource {
    pub fn huggingface(hf_spec: impl Into<String>) -> Self {
        Self {
            hf_spec: hf_spec.into(),
            ollama_name: None,
        }
    }

    pub fn ollama(ollama_name: impl Into<String>, hf_spec: impl Into<String>) -> Self {
        Self {
            hf_spec: hf_spec.into(),
            ollama_name: Some(ollama_name.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LaunchSource {
    OllamaBlob(PathBuf),
    HuggingFace(String),
}

impl LaunchSource {
    fn model_path(&self) -> Option<String> {
        match self {
            Self::OllamaBlob(path) => Some(path.display().to_string()),
            Self::HuggingFace(_) => None,
        }
    }

    fn source_name(&self) -> &'static str {
        match self {
            Self::OllamaBlob(_) => "ollama",
            Self::HuggingFace(_) => "huggingface",
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
}

/// Singleton manager for the llama-server sidecar.
pub struct LlamaSidecar {
    inner: Mutex<Inner>,
    startup: Mutex<()>,
}

static SIDECAR: OnceLock<LlamaSidecar> = OnceLock::new();

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
        }),
        startup: Mutex::new(()),
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

fn model_source_cache_status_in_dir(cache_dir: &Path, source: &ModelSource) -> ModelCacheStatus {
    let ollama_status = source
        .ollama_name
        .as_deref()
        .map(|name| ollama_model_cache_status_in_dir(cache_dir, name));
    match ollama_status {
        Some(ModelCacheStatus::Downloaded) => ModelCacheStatus::Downloaded,
        Some(ModelCacheStatus::Partial) => ModelCacheStatus::Partial,
        _ => model_cache_status_in_dir(cache_dir, &source.hf_spec),
    }
}

pub fn model_source_path(source: &ModelSource) -> Option<PathBuf> {
    source
        .ollama_name
        .as_deref()
        .and_then(|name| ollama_model_blob_path_in_dir(&model_cache_dir(), name))
}

fn model_cache_status_in_dir(cache_dir: &Path, hf_spec: &str) -> ModelCacheStatus {
    let Some(repo_dir) = model_cache_repo_dir(cache_dir, hf_spec) else {
        return ModelCacheStatus::NotDownloaded;
    };

    if !repo_dir.is_dir() {
        return ModelCacheStatus::NotDownloaded;
    }

    let quant = hf_spec
        .split_once(':')
        .map(|(_, quant)| quant.to_ascii_lowercase());
    let mut has_partial = false;
    let mut has_any_gguf = false;
    let mut has_matching_gguf = false;
    visit_cache_files(&repo_dir, &mut |path| {
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            return;
        };
        if file_name.ends_with(".downloadInProgress") {
            has_partial = true;
        }
        if file_name.to_ascii_lowercase().ends_with(".gguf") {
            has_any_gguf = true;
            if quant
                .as_ref()
                .is_none_or(|q| file_name.to_ascii_lowercase().contains(q))
            {
                has_matching_gguf = true;
            }
        }
    });

    if has_matching_gguf || (quant.is_none() && has_any_gguf) {
        ModelCacheStatus::Downloaded
    } else if has_partial {
        ModelCacheStatus::Partial
    } else {
        ModelCacheStatus::NotDownloaded
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
        .any(|part| part.is_empty())
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

fn ollama_manifest_path(cache_dir: &Path, name: &str) -> Option<PathBuf> {
    let name = parse_ollama_name(name)?;
    Some(
        cache_dir
            .join("manifests")
            .join(name.host)
            .join(name.namespace)
            .join(name.model)
            .join(name.tag),
    )
}

fn ollama_model_cache_status_in_dir(cache_dir: &Path, name: &str) -> ModelCacheStatus {
    let Some(manifest_path) = ollama_manifest_path(cache_dir, name) else {
        return ModelCacheStatus::NotDownloaded;
    };
    if !manifest_path.is_file() {
        return ModelCacheStatus::NotDownloaded;
    }
    let Some(blob_path) = ollama_model_blob_path_in_dir(cache_dir, name) else {
        return ModelCacheStatus::Partial;
    };
    if is_gguf_file(&blob_path) {
        ModelCacheStatus::Downloaded
    } else {
        ModelCacheStatus::Partial
    }
}

fn ollama_model_blob_path_in_dir(cache_dir: &Path, name: &str) -> Option<PathBuf> {
    let manifest_path = ollama_manifest_path(cache_dir, name)?;
    let bytes = std::fs::read(manifest_path).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let digest = manifest
        .get("layers")?
        .as_array()?
        .iter()
        .find(|layer| {
            layer.get("mediaType").and_then(|value| value.as_str())
                == Some("application/vnd.ollama.image.model")
        })?
        .get("digest")?
        .as_str()?;
    let digest_path = digest.replace(':', "-");
    let path = cache_dir.join("blobs").join(digest_path);
    path.is_file().then_some(path)
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

/// The memory budget relevant for local GPU inference. On Apple Silicon this
/// is unified system memory; elsewhere it is discrete VRAM when we can detect
/// it. Unknown VRAM is reported as `None` so the UI can warn instead of
/// pretending system RAM is enough.
pub fn accelerator_memory_gib() -> Option<u64> {
    if is_apple_silicon() {
        let total = total_memory_gib();
        return (total > 0).then_some(total);
    }
    detect_discrete_vram_gib()
}

pub fn accelerator_memory_kind() -> &'static str {
    if is_apple_silicon() {
        "apple_unified"
    } else if accelerator_memory_gib().is_some() {
        "vram"
    } else {
        "unknown_vram"
    }
}

pub fn recommendation_memory_gib() -> u64 {
    accelerator_memory_gib().unwrap_or(0)
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
/// A model's *trained* window (read later from `/props`) can be huge — Qwen3.6
/// is 262k — but the KV cache for it scales with both the window and the model
/// size, and allocating tens of GB at startup is slow-to-impossible on a
/// laptop. Biorouter's agent bootstrap can also consume around 20k tokens
/// before the first user turn, so machines with at least 16 GiB of
/// GPU-addressable memory get a 64k context. On Apple Silicon, unified memory
/// counts; on Intel Macs, Windows, and Linux this uses detected VRAM. 128k is
/// the ceiling. Users can still pin any explicit `LLAMACPP_CONTEXT_SIZE`, and
/// the real allocated window is always read back from `/props` for accounting.
pub fn default_context_size() -> usize {
    match recommendation_memory_gib() {
        gib if gib >= 64 => 131_072, // 128k — workstations / Apple Silicon Max/Ultra
        gib if gib >= 16 => LLAMACPP_AGENT_CONTEXT_SIZE, // 64k — agent-ready accelerator default
        _ => 32_768,                 // 32k — small or unknown VRAM, with a UI warning
    }
}

/// Value for `--spec-type`, or `None` to omit the flag entirely.
pub fn configured_spec_type() -> Option<String> {
    if OPTIONAL_FLAGS_DISABLED.load(Ordering::Relaxed) {
        return None;
    }
    let configured = crate::config::Config::global()
        .get_param::<String>("LLAMACPP_SPEC_TYPE")
        .ok()
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| DEFAULT_SPEC_TYPE.to_string());
    if configured.is_empty() || configured.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(configured)
}

fn explicit_context_size() -> Option<usize> {
    crate::config::Config::global()
        .get_param::<usize>("LLAMACPP_CONTEXT_SIZE")
        .ok()
        .filter(|&n| n > 0)
}

/// The `--ctx-size` passed to llama-server. An explicit positive
/// `LLAMACPP_CONTEXT_SIZE` pins the window (and caps memory); otherwise (unset
/// or `0` = "auto") we use a memory-tiered default ([`default_context_size`])
/// rather than the model's full native window, so large models don't hang
/// allocating a giant KV cache on startup. The size the server actually
/// allocates is read back from `/props` ([`live_context_size`]) and reported to
/// token accounting, so the gauge always matches reality. q8_0 KV-cache
/// quantization keeps the memory cost affordable.
fn configured_context_size() -> usize {
    explicit_context_size().unwrap_or_else(default_context_size)
}

fn next_lower_auto_context_size(ctx_size: usize) -> Option<usize> {
    match ctx_size {
        n if n > LLAMACPP_AGENT_CONTEXT_SIZE => Some(LLAMACPP_AGENT_CONTEXT_SIZE),
        n if n > 32_768 => Some(32_768),
        _ => None,
    }
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

fn looks_like_memory_failure(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    [
        "out of memory",
        "not enough memory",
        "failed to allocate",
        "cannot allocate",
        "unable to allocate",
        "memory allocation",
    ]
    .iter()
    .any(|needle| log.contains(needle))
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

/// Query a running llama-server's `/props` for the context window it actually
/// allocated for the current model. Returns `None` if the server isn't
/// reachable or doesn't report it.
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
    // otherwise the configured port — so an already-running server (started by
    // another Biorouter process, or before this one recorded it) still yields
    // the real window. Keeps the CLI gauge and GUI status consistent instead of
    // falling back to a fixed default.
    live_context_size(port.unwrap_or_else(configured_port)).await
}

async fn live_context_size(port: u16) -> Option<usize> {
    let url = format!("http://127.0.0.1:{port}/props");
    let client = reqwest::Client::builder()
        .timeout(STATUS_PROBE_TIMEOUT)
        .build()
        .ok()?;
    let body: serde_json::Value = client.get(&url).send().await.ok()?.json().await.ok()?;
    // llama-server reports the live window under
    // default_generation_settings.n_ctx; fall back to a top-level n_ctx.
    body.get("default_generation_settings")
        .and_then(|g| g.get("n_ctx"))
        .or_else(|| body.get("n_ctx"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
}

fn launch_source_for(source: &ModelSource) -> LaunchSource {
    if model_cache_status(&source.hf_spec) == ModelCacheStatus::Downloaded {
        return LaunchSource::HuggingFace(source.hf_spec.clone());
    }
    model_source_path(source)
        .map(LaunchSource::OllamaBlob)
        .unwrap_or_else(|| LaunchSource::HuggingFace(source.hf_spec.clone()))
}

/// Build the llama-server argv (without the program itself). Factored out for
/// testing.
fn build_args(source: &LaunchSource, alias: &str, port: u16, ctx_size: usize) -> Vec<String> {
    let config = crate::config::Config::global();
    let mut args = match source {
        LaunchSource::OllamaBlob(path) => {
            vec!["-m".to_string(), path.display().to_string()]
        }
        LaunchSource::HuggingFace(hf_spec) => vec!["-hf".to_string(), hf_spec.to_string()],
    };
    args.extend([
        "--alias".to_string(),
        alias.to_string(),
        "--ctx-size".to_string(),
        ctx_size.to_string(),
        "--jinja".to_string(),
        "--no-webui".to_string(),
        "--no-mmproj".to_string(),
        // Halve KV-cache memory so the (memory-tiered) default context stays
        // affordable. q8_0 is considered lossless in practice; it's q4 KV that
        // degrades tool calling. Overridable via LLAMACPP_EXTRA_ARGS.
        "--cache-type-k".to_string(),
        "q8_0".to_string(),
        "--cache-type-v".to_string(),
        "q8_0".to_string(),
    ]);
    // Thinking is off by default so small warm-up/test completions produce
    // visible content instead of spending the budget on hidden reasoning.
    // Set LLAMACPP_ENABLE_THINKING=true to enable it for capable models.
    // Uses the current `--reasoning on|off` flag, not the deprecated
    // `--chat-template-kwargs {"enable_thinking":...}` form.
    let thinking = config
        .get_param::<bool>("LLAMACPP_ENABLE_THINKING")
        .ok()
        .or_else(|| {
            config
                .get_param::<String>("LLAMACPP_ENABLE_THINKING")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(false);
    args.push("--reasoning".to_string());
    args.push(if thinking { "on" } else { "off" }.to_string());
    // Self-speculative decoding. The `ngram-*` family drafts from patterns already
    // present in the context, so unlike draft-model speculation it needs no second
    // GGUF, no download, no extra VRAM and no tokenizer pairing — it is free.
    //
    // Measured on an M4 Max with gemma-4-E4B q4_0, agentic task / free text:
    //   none          79.7 / 82.4 tok/s   (what we shipped)
    //   ngram-cache  113.8 / 82.4
    //   ngram-simple 354.4 / 82.8   <- chosen: 4.4x, and free text is unchanged
    //   ngram-mod    397.3 / 80.6        faster, but costs 2% on free text
    //   ngram-map-k4v 378.4 / 69.1       disqualified: -16% on free text
    // The agentic case — re-emitting text already in context (quoting a tool
    // result, rewriting a file, echoing a schema) — is most of what an agent does.
    //
    // Set LLAMACPP_SPEC_TYPE to another value to change it, or to "none"/"" to
    // pass nothing. Only Metal/Gemma has been measured; if a platform or family
    // regresses, that key is the escape hatch, and `unsupported_argument` below
    // recovers automatically if a future llama.cpp drops the flag.
    if let Some(spec) = configured_spec_type() {
        args.push("--spec-type".to_string());
        args.push(spec);
    }
    if let Ok(extra) = config.get_param::<String>("LLAMACPP_EXTRA_ARGS") {
        args.extend(sanitize_extra_args(&extra));
    }
    // Keep the managed bind after user-supplied arguments as a second layer of
    // protection in case the sanitizer misses a future argument form.
    args.push("--host".to_string());
    args.push("127.0.0.1".to_string());
    args.push("--port".to_string());
    args.push(port.to_string());
    args
}

/// Flags a user must not be able to inject via `LLAMACPP_EXTRA_ARGS`: anything
/// that changes the network bind or the server's auth/file-serving posture. The
/// managed sidecar is a loopback-only, no-auth server *by design* (the only
/// access control is the 127.0.0.1 bind), so letting config re-bind it to
/// `0.0.0.0` would silently expose an unauthenticated inference server on the
/// LAN. `--port` is owned by `LLAMACPP_PORT`; the rest change file serving.
const FORBIDDEN_EXTRA_FLAGS: &[&str] = &[
    "--host",
    "--port",
    "--api-key",
    "--api-key-file",
    "--path",
    "--rpc",
];

/// Whitespace-split `LLAMACPP_EXTRA_ARGS` and drop any forbidden flag together
/// with the token that follows it (its value). Emits a warning so the dropped
/// flag is visible in logs rather than silently ignored.
fn sanitize_extra_args(extra: &str) -> Vec<String> {
    let tokens: Vec<&str> = extra.split_whitespace().collect();
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        // Match `--flag` and `--flag=value` forms.
        let flag_name = tok.split('=').next().unwrap_or(tok);
        if FORBIDDEN_EXTRA_FLAGS.contains(&flag_name) {
            tracing::warn!(
                "Ignoring forbidden flag '{tok}' in LLAMACPP_EXTRA_ARGS: the managed \
                 llama-server is loopback-only and cannot be re-bound or have its auth/path changed"
            );
            // Skip a following value token if this was the separate-arg form
            // (`--host 0.0.0.0`) rather than the `--host=0.0.0.0` form.
            if !tok.contains('=') && i + 1 < tokens.len() && !tokens[i + 1].starts_with('-') {
                i += 1;
            }
            i += 1;
            continue;
        }
        out.push(tok.to_string());
        i += 1;
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

/// Kill llama-server children recorded by Biorouter processes that no longer
/// exist. Returns true when anything was killed (callers may want to give
/// the OS a moment to release the port).
fn reap_orphans() -> bool {
    let mut killed = false;
    let Ok(entries) = std::fs::read_dir(run_dir()) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let parent: Option<u32> = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse().ok());
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
    let args = build_args(&launch_source, model, port, ctx_size);
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

    let mut command = Command::new(binary);
    command
        .args(&args)
        .env("LLAMA_CACHE", cache_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
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
    /// Ensure a llama-server is running and serving `model` (friendly alias)
    /// backed by `source`. Returns the port it is (or will be) serving on.
    /// Does NOT wait for readiness — pair with [`Self::wait_ready`].
    pub async fn ensure(&self, model: &str, source: &ModelSource) -> Result<u16> {
        let mut inner = self.inner.lock().await;
        let explicit_ctx = explicit_context_size();
        let ctx_size = configured_context_size();
        let automatic_context_size = explicit_ctx.is_none();

        // Already running the requested model?
        if inner.model.as_deref() == Some(model) {
            if let Some(child) = inner.child.as_mut() {
                if child.try_wait()?.is_none() {
                    let same_context = if inner.automatic_context_size && automatic_context_size {
                        true
                    } else {
                        inner.requested_context_size == Some(ctx_size)
                            && inner.automatic_context_size == automatic_context_size
                    };
                    if same_context {
                        return Ok(inner.port.expect("running child always has a port"));
                    }
                }
                // Process died — fall through to respawn.
                finish_log_tasks(&mut inner.log_tasks).await;
                inner.last_error = Some(log_tail_joined(&inner.log_tail, 8));
                inner.child = None;
                inner.warmed_model = None;
            }
        }

        // A different model (or nothing) is running: stop what we own.
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

        let child = spawn_child(
            &mut inner, &binary, &cache_dir, model, source, port, ctx_size,
        )?;
        inner.child = Some(child);
        inner.port = Some(port);
        inner.model = Some(model.to_string());
        inner.model_source = Some(source.clone());
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
            let (port, dead, tail) = {
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
                (port, dead, inner.log_tail.clone())
            };

            if dead {
                let tail = log_tail_joined(&tail, 10);
                // Publish "restarting" BEFORE evaluating the retry paths. Each of
                // them re-takes `inner`, so the lock is released in between, and
                // that gap is exactly when the GUI's poller used to observe a
                // terminal Error and abort a recoverable startup.
                self.inner.lock().await.restarting = true;
                let retried = self.try_self_heal(&tail).await;
                self.inner.lock().await.restarting = false;
                if retried? {
                    deadline = tokio::time::Instant::now() + timeout;
                    continue;
                }
                self.inner.lock().await.last_error = Some(tail.clone());
                return Err(anyhow!("llama-server exited during startup:\n{tail}"));
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
    /// rejected an argument. Process-wide and sticky: binary support does not
    /// change while we are running, so re-learning it on every spawn would mean
    /// one guaranteed crash per model switch.
    async fn retry_without_optional_flags(&self, startup_log: &str) -> Result<bool> {
        if !looks_like_unsupported_argument(startup_log)
            || OPTIONAL_FLAGS_DISABLED.load(Ordering::Relaxed)
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
        if !matches!(inner.launch_source, Some(LaunchSource::OllamaBlob(_))) {
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
            "llama-server could not load Ollama blob for {model}; retrying Hugging Face fallback"
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
    fn model_source_cache_status_detects_ollama_manifest_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_dir = tmp
            .path()
            .join("manifests")
            .join("registry.ollama.ai")
            .join("library")
            .join("gemma4");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        let digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        std::fs::write(
            manifest_dir.join("latest"),
            serde_json::json!({
                "schemaVersion": 2,
                "layers": [
                    {
                        "mediaType": "application/vnd.ollama.image.model",
                        "digest": digest,
                        "size": 4
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let source = ModelSource::ollama("gemma4:latest", "ggml-org/gemma-4-E4B-it-GGUF:Q4_K_M");

        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::Partial
        );

        let blob_dir = tmp.path().join("blobs");
        std::fs::create_dir_all(&blob_dir).unwrap();
        std::fs::write(
            blob_dir
                .join("sha256-1111111111111111111111111111111111111111111111111111111111111111"),
            b"GGUF",
        )
        .unwrap();

        assert_eq!(
            model_source_cache_status_in_dir(tmp.path(), &source),
            ModelCacheStatus::Downloaded
        );
        assert_eq!(
            ollama_model_blob_path_in_dir(tmp.path(), "gemma4:latest")
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("sha256-1111111111111111111111111111111111111111111111111111111111111111")
        );
    }

    #[test]
    fn launch_source_prefers_downloaded_huggingface_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_string_lossy().into_owned();
        let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);
        let digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let manifest = tmp
            .path()
            .join("manifests/registry.ollama.ai/library/gemma4/latest");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(
            manifest,
            serde_json::json!({
                "layers": [{
                    "mediaType": "application/vnd.ollama.image.model",
                    "digest": digest,
                }]
            })
            .to_string(),
        )
        .unwrap();
        let blob = tmp
            .path()
            .join(format!("blobs/{}", digest.replace(':', "-")));
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, b"GGUF").unwrap();
        let source =
            ModelSource::ollama("gemma4:latest", "google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0");

        assert_eq!(launch_source_for(&source), LaunchSource::OllamaBlob(blob));

        let fallback = tmp
            .path()
            .join("models--google--gemma-4-E4B-it-qat-q4_0-gguf/snapshots/rev");
        std::fs::create_dir_all(&fallback).unwrap();
        std::fs::write(fallback.join("gemma-4-E4B-Q4_0.gguf"), b"GGUF").unwrap();

        assert_eq!(
            launch_source_for(&source),
            LaunchSource::HuggingFace("google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0".to_string())
        );
    }

    #[test]
    fn parse_ollama_name_applies_registry_defaults() {
        assert_eq!(
            parse_ollama_name("qwen3.6:latest"),
            Some(OllamaName {
                host: "registry.ollama.ai".to_string(),
                namespace: "library".to_string(),
                model: "qwen3.6".to_string(),
                tag: "latest".to_string(),
            })
        );
        assert_eq!(
            parse_ollama_name("team/model:tag"),
            Some(OllamaName {
                host: "registry.ollama.ai".to_string(),
                namespace: "team".to_string(),
                model: "model".to_string(),
                tag: "tag".to_string(),
            })
        );
    }

    #[test]
    fn model_cache_dir_follows_ollama_models() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_string_lossy().into_owned();
        let _guard = env_lock::lock_env([("OLLAMA_MODELS", Some(tmp_path.as_str()))]);

        assert_eq!(model_cache_dir(), PathBuf::from(&tmp_path));
    }

    #[test]
    fn default_context_size_is_memory_tiered_and_capped() {
        // Whatever this machine reports, the default must be one of the tiers,
        // never exceed the 128k cap, and never be absurdly small.
        let d = default_context_size();
        assert!(
            [32_768usize, LLAMACPP_AGENT_CONTEXT_SIZE, 131_072].contains(&d),
            "unexpected tier: {d}"
        );
        assert!(d <= 131_072, "must not exceed the 128k cap: {d}");
        assert!(d >= 32_768, "must stay usable: {d}");
        // The tier must agree with the measured memory (guards the thresholds).
        let gib = recommendation_memory_gib();
        let expected = if gib >= 64 {
            131_072
        } else if gib >= 16 {
            LLAMACPP_AGENT_CONTEXT_SIZE
        } else {
            32_768
        };
        assert_eq!(
            d, expected,
            "tier disagrees with {gib} GiB of GPU-addressable memory"
        );
    }

    #[test]
    fn parses_vram_size_strings() {
        assert_eq!(parse_memory_gib("1536 MB"), Some(2));
        assert_eq!(parse_memory_gib("5.07 GB"), Some(6));
        assert_eq!(parse_memory_gib("1 TB"), Some(1024));
        assert_eq!(parse_memory_gib("Built-In"), None);
    }

    #[test]
    fn build_args_includes_model_port_and_jinja() {
        let _guard = env_lock::lock_env([("LLAMACPP_ENABLE_THINKING", Some("false"))]);
        let source =
            LaunchSource::HuggingFace("google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0".to_string());
        let args = build_args(&source, "gemma-4-e4b", 12345, 32_768);
        let joined = args.join(" ");
        assert!(joined.contains("-hf google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0"));
        assert!(joined.contains("--alias gemma-4-e4b"));
        assert!(joined.contains("--port 12345"));
        assert!(joined.contains("--ctx-size 32768"));
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
        assert!(joined.contains("--no-webui"));
        assert!(joined.contains("--no-mmproj"));
        // Thinking is disabled by default via the current --reasoning flag.
        assert!(joined.contains("--reasoning off"));
        assert!(!joined.contains("enable_thinking"));
    }

    #[test]
    fn build_args_explicit_ctx_size_is_passed_through() {
        let args = build_args(
            &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
            "m",
            11543,
            16384,
        );
        assert!(args.join(" ").contains("--ctx-size 16384"));
    }

    #[test]
    fn auto_context_steps_down_like_ollama() {
        assert_eq!(
            next_lower_auto_context_size(131_072),
            Some(LLAMACPP_AGENT_CONTEXT_SIZE)
        );
        assert_eq!(
            next_lower_auto_context_size(65_537),
            Some(LLAMACPP_AGENT_CONTEXT_SIZE)
        );
        assert_eq!(
            next_lower_auto_context_size(LLAMACPP_AGENT_CONTEXT_SIZE),
            Some(32_768)
        );
        assert_eq!(next_lower_auto_context_size(32_768), None);
    }

    #[test]
    fn detects_memory_failure_log_fragments() {
        assert!(looks_like_memory_failure("failed to allocate Metal buffer"));
        assert!(looks_like_memory_failure("CUDA out of memory"));
        assert!(looks_like_memory_failure("not enough memory for KV cache"));
        assert!(!looks_like_memory_failure("download failed with 404"));
    }

    #[test]
    fn detects_model_load_failure_log_fragments() {
        assert!(looks_like_model_load_failure(
            "wrong number of tensors; expected 2131, got 720"
        ));
        assert!(looks_like_model_load_failure("error loading model"));
        assert!(!looks_like_model_load_failure("download failed with 404"));
    }

    #[test]
    fn build_args_reasoning_off_when_disabled() {
        let _guard = env_lock::lock_env([("LLAMACPP_ENABLE_THINKING", Some("false"))]);
        let args = build_args(
            &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
            "m",
            11543,
            32_768,
        );
        assert!(args.join(" ").contains("--reasoning off"));
    }

    #[test]
    fn build_args_reasoning_on_when_enabled() {
        let _guard = env_lock::lock_env([("LLAMACPP_ENABLE_THINKING", Some("true"))]);
        let args = build_args(
            &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
            "m",
            11543,
            32_768,
        );
        assert!(args.join(" ").contains("--reasoning on"));
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

        // A new line.
        push_log(&tail, "loading model".to_string());
        let second = progress_marker(&tail);
        assert_ne!(first, second, "a new line must count as progress");

        // Same line count, different content — llama.cpp's download percentage
        // overwrites one line, and a count-only marker would read that as a
        // stall and time out a healthy multi-GB download.
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

        // A genuinely quiet server yields a stable marker, which is what lets
        // the deadline still fire on a wedged process.
        assert_eq!(third, progress_marker(&tail), "no change must be stable");
    }

    #[test]
    fn build_args_enables_self_speculative_decoding_by_default() {
        let _guard = env_lock::lock_env([("LLAMACPP_SPEC_TYPE", None::<&str>)]);
        OPTIONAL_FLAGS_DISABLED.store(false, Ordering::Relaxed);
        let args = build_args(
            &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
            "m",
            11543,
            32_768,
        );
        // Measured 4.4x on agentic (repetition-heavy) generation with no
        // free-text cost; see the table in `build_args`. It must be ON by
        // default — shipping it behind an opt-in nobody sets is the same as
        // not shipping it.
        assert!(
            args.join(" ")
                .contains(&format!("--spec-type {DEFAULT_SPEC_TYPE}")),
            "expected default --spec-type in {args:?}"
        );
    }

    #[test]
    fn build_args_omits_spec_type_when_disabled() {
        // "none" is the documented escape hatch for a platform or model family
        // that regresses. It must emit NO flag at all, not `--spec-type none`.
        for value in ["none", "NONE", ""] {
            let _guard = env_lock::lock_env([("LLAMACPP_SPEC_TYPE", Some(value))]);
            OPTIONAL_FLAGS_DISABLED.store(false, Ordering::Relaxed);
            let args = build_args(
                &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
                "m",
                11543,
                32_768,
            );
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
        let args = build_args(
            &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
            "m",
            11543,
            32_768,
        );
        OPTIONAL_FLAGS_DISABLED.store(false, Ordering::Relaxed);
        assert!(
            !args.iter().any(|a| a == "--spec-type"),
            "a rejected flag must not be re-emitted: {args:?}"
        );
    }

    #[test]
    fn unsupported_argument_is_told_apart_from_a_memory_failure() {
        // The distinction decides WHICH self-heal runs: dropping the optional
        // flag, or stepping the context down. Confusing them means a pin bump
        // that renames a flag silently halves the user's context window
        // instead of restoring inference.
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
    fn sanitize_extra_args_drops_forbidden_flags() {
        // Separate-arg form: flag and its value are both dropped.
        assert_eq!(
            sanitize_extra_args("--host 0.0.0.0 --threads 8"),
            vec!["--threads", "8"]
        );
        // `--flag=value` form is dropped (no following value to skip).
        assert_eq!(
            sanitize_extra_args("--host=0.0.0.0 --threads 8"),
            vec!["--threads", "8"]
        );
        // api-key / path / rpc are also forbidden.
        assert_eq!(
            sanitize_extra_args("--api-key secret"),
            Vec::<String>::new()
        );
        assert_eq!(
            sanitize_extra_args("--rpc 1.2.3.4:50052"),
            Vec::<String>::new()
        );
        // Benign flags pass through untouched.
        assert_eq!(
            sanitize_extra_args("--flash-attn --parallel 4"),
            vec!["--flash-attn", "--parallel", "4"]
        );
    }

    #[test]
    fn build_args_reasserts_loopback_last_even_with_injected_host() {
        let _guard =
            env_lock::lock_env([("LLAMACPP_EXTRA_ARGS", Some("--host 0.0.0.0 --port 9999"))]);
        let args = build_args(
            &LaunchSource::HuggingFace("owner/repo:Q4_K_M".to_string()),
            "m",
            11543,
            32768,
        );
        // The injected 0.0.0.0 must not survive, and the final --host/--port
        // (last-wins for llama-server) must be the loopback bind on our port.
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
    fn build_args_can_launch_from_ollama_blob_path() {
        let path = PathBuf::from("/tmp/ollama/models/blobs/sha256-abc");
        let args = build_args(
            &LaunchSource::OllamaBlob(path.clone()),
            "gemma4",
            11543,
            32768,
        );
        let joined = args.join(" ");
        assert!(joined.contains(&format!("-m {}", path.display())));
        assert!(!joined.contains("-hf"));
        assert!(joined.contains("--alias gemma4"));
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
