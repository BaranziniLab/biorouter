//! The Ollama provider: native `POST /api/chat`.
//!
//! Until 2026-10 this provider posted to Ollama's OpenAI-compatible
//! `/v1/chat/completions`, which cannot set the context window. On a machine
//! whose Ollama default window is 4096 tokens (every CPU-only or low-VRAM
//! machine, every Intel Mac) Ollama then silently kept the first 4 tokens plus
//! the last ~2046 of BioRouter's ~31-40k-token prompt, the model saw only the
//! tail of the alphabetically sorted tool list, and it wrote invented tool calls
//! as JSON text. BioRouter meanwhile believed it had a 128k window (a guess from
//! the model's name) and compaction trusted the truncated usage, so nothing
//! noticed. The wire format and the measurements behind it are in
//! [`super::formats::ollama`]; this module decides what to send:
//!
//! - **One window per model, sent on every request.** `options.num_ctx` is
//!   resolved once per (provider instance, model) — see [`choose_num_ctx`] —
//!   and the same number goes on a turn, a blocking completion and a session
//!   title alike, because on Ollama's llama.cpp runner a different `num_ctx`
//!   reloads the model (measured). The same number is what
//!   [`OllamaProvider::get_model_config`] reports, so compaction and the gauge
//!   work from the window the model really has. After loading, `/api/ps` may
//!   reduce an automatic window to 64k when the cache spills or exceeds the
//!   local memory budget; explicit windows are preserved.
//! - **`truncate: false`**, so an over-window prompt is a 400 mapped to
//!   [`ProviderError::ContextLengthExceeded`] instead of a silent cut, and a
//!   size check on every reply for servers that ignore it (0.4.0 to 0.12.5;
//!   before 0.4.0 the reported count leaves out the cache and cannot be judged).
//! - **Every request streams**, the blocking ones included, so `OLLAMA_TIMEOUT`
//!   bounds silence and never a whole generation; and a turn whose model config
//!   pins no output budget gets a ceiling, so a runaway generation ends.
//! - **No sampling defaults.** Only what the model config pins is sent, so the
//!   Modelfile's recommended sampling applies.
//! - **`think` from the reasoning-effort control**, only to models that declare
//!   the `thinking` capability, and `keep_alive` so a 27-31B model is not
//!   unloaded (and its ~30k-token prompt re-processed) after Ollama's 5-minute
//!   default.

use super::api_client::{ApiClient, AuthMethod};
use super::base::{
    ConfigKey, MessageStream, Provider, ProviderMetadata, ProviderStreamItem, ProviderUsage, Usage,
};
use super::errors::ProviderError;
use super::formats::ollama::{
    self as native, ChatRequest, DecodeOptions, ModelFacts, Think, UnknownToolCalled,
    OLLAMA_CHAT_PATH,
};
use super::formats::text_tool_calls::{
    recover_in_message, recover_leaked_tool_calls, RecoveryContext,
};
use super::retry::{ProviderRetry, TimeoutRetry};
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat, RequestLog,
};
use crate::agents::effort::ReasoningEffort;
use crate::config::declarative_providers::DeclarativeProviderConfig;
use crate::config::BioRouterMode;
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::formats::openai::{create_request, get_usage, response_to_message};
use crate::utils::safe_truncate;
use anyhow::Result;
use async_stream::try_stream;
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use reqwest::{Response, StatusCode};
use rmcp::model::Tool;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;
use url::Url;

pub const OLLAMA_HOST: &str = "localhost";
pub const OLLAMA_TIMEOUT: u64 = 600;
pub const OLLAMA_DEFAULT_PORT: u16 = 11434;
/// A bare tag, never `:latest` or a `-nvfp4`/`-mlx` variant: the bare tag is a
/// manifest list, so Ollama pulls the MLX build on Apple Silicon and the GGUF
/// build everywhere else.
pub const OLLAMA_DEFAULT_MODEL: &str = "qwen3.8:27b";
// Empty: with no curated list, the frontend falls back to the live `api/tags`
// endpoint via `fetch_supported_models` and lists the models the user has
// actually installed locally. Users can still write in any model name via the
// "Enter a model not listed..." option (allows_unlisted_models = true below).
// The models BioRouter RECOMMENDS are `OLLAMA_RECOMMENDED_MODELS`.
pub const OLLAMA_KNOWN_MODELS: &[&str] = &[];
pub const OLLAMA_DOC_URL: &str = "https://ollama.com/library";
/// Where Ollama's cloud models (`:cloud`, `-cloud` tags) run.
const OLLAMA_CLOUD_ORIGIN: &str = "https://ollama.com";

/// `keep_alive` when `OLLAMA_KEEP_ALIVE` is unset. Ollama's own default is 5
/// minutes, after which a 17-19 GB model unloads and the next turn pays the
/// load plus a full re-prefill of BioRouter's ~30k-token prompt.
pub const OLLAMA_DEFAULT_KEEP_ALIVE: &str = "30m";

/// The smallest window chosen automatically, unless the model was trained on
/// less. BioRouter's own instructions and tool schemas are ~31k tokens with the
/// default extensions and ~40k with more, so anything smaller fails (or, on an
/// Ollama that still truncates, silently loses the instructions) before the
/// conversation starts.
pub const OLLAMA_AGENT_CONTEXT_SIZE: usize = 65_536;
/// The automatic window on a machine with at least
/// [`OLLAMA_LARGE_CONTEXT_MEMORY_GIB`] of GPU-addressable memory.
pub const OLLAMA_LARGE_CONTEXT_SIZE: usize = 131_072;
pub const OLLAMA_LARGE_CONTEXT_MEMORY_GIB: u64 = 64;

/// The first Ollama that honours `truncate`. Older servers ignore the field and
/// keep cutting over-window prompts; [`TurnCheck`] is the defence there.
pub const OLLAMA_MIN_TRUNCATE_VERSION: (u64, u64, u64) = (0, 12, 6);
/// The first Ollama whose `prompt_eval_count` is the whole prompt. Before it
/// (the llama.cpp-server subprocess, through 0.3.x) the count was llama-server's
/// `timings.prompt_n`: only the tokens processed AFTER the cached prefix, so the
/// second request of every tool loop reported a few hundred tokens for a
/// ~30k-token prompt. From 0.4.0 the Go runner reports `len(inputs)`, taken
/// before the cache is consulted.
const OLLAMA_WHOLE_PROMPT_COUNT_VERSION: (u64, u64, u64) = (0, 4, 0);

/// The setting that changes the window the built-in provider asks for.
const BUILTIN_WINDOW_SETTING: &str = "OLLAMA_NUM_CTX";
/// The same for an Ollama-engine provider declared in a `custom_providers`
/// file: it does not read `OLLAMA_NUM_CTX` (that describes the built-in
/// provider's server), but it does honour a context limit set on its model,
/// which `BIOROUTER_CONTEXT_LIMIT` is the way to set.
const CUSTOM_WINDOW_SETTING: &str = "BIOROUTER_CONTEXT_LIMIT";

/// An output budget below this is spent on reasoning before an answer begins,
/// so a request that pins one gets thinking off unless Deep effort asked for it.
/// `biorouter configure` probes with 50 tokens: qwen3.8:27b at its default
/// (medium) thinking returned 243 characters of reasoning, no content and
/// `done_reason: length`, which the answered-or-error guard rightly fails; with
/// `think: false` the same probe made its `get_weather` call in 26 tokens
/// (Ollama 0.40.2). Real questions think for hundreds to thousands of tokens.
const SMALL_OUTPUT_BUDGET: i32 = 1_024;

/// `num_predict` for a turn when the model config pins none. Ollama's default
/// is -1, no limit, and since `OLLAMA_TIMEOUT` bounds only silence, a model
/// caught repeating itself would stream until someone pressed Stop — the Llama
/// Server provider caps its turns for the same reason, at the same numbers.
const OLLAMA_DEFAULT_NUM_PREDICT: i32 = 16_384;
/// The same when the model may think: thinking spends the budget before the
/// answer does (Qwen evaluates Qwen3.8 at 32,768).
const OLLAMA_THINKING_NUM_PREDICT: i32 = 32_768;

/// A model BioRouter recommends pulling into Ollama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RecommendedOllamaModel {
    /// The tag to `ollama pull`.
    pub tag: &'static str,
    pub display_name: &'static str,
    /// Approximate download, in GB, of the build a non-Apple machine gets (the
    /// MLX build on Apple Silicon is about the same).
    pub approx_download_gb: u32,
}

/// The two local models BioRouter supports on Ollama, default first. Machines
/// with less than [`OLLAMA_RECOMMENDED_MIN_MEMORY_GIB`] still get the default,
/// with a warning that it will be slow or may not load — not a different model.
pub const OLLAMA_RECOMMENDED_MODELS: &[RecommendedOllamaModel] = &[
    RecommendedOllamaModel {
        tag: OLLAMA_DEFAULT_MODEL,
        display_name: "Qwen3.8 27B",
        approx_download_gb: 18,
    },
    RecommendedOllamaModel {
        tag: "gemma4:31b",
        display_name: "Gemma 4 31B",
        approx_download_gb: 19,
    },
];

/// Below this much GPU-addressable memory a 27-31B model at Q4 (17-20 GB of
/// weights plus the KV cache) is below its practical minimum.
pub const OLLAMA_RECOMMENDED_MIN_MEMORY_GIB: u64 = 32;

/// How long a `/api/show` or `/api/version` probe may take. Both answer from
/// disk without loading the model; a server that does not answer this fast is
/// treated as not saying, rather than holding up the turn.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Output budget for a session title. Thinking is off for it, so a title is a
/// dozen tokens; the cap only matters for a model that ignores `think: false`.
const TITLE_NUM_PREDICT: i32 = 128;

/// A request whose text is smaller than this is never judged truncated: there
/// is too little of it for a bytes-per-token ratio to mean anything.
const TRUNCATION_CHECK_MIN_BYTES: usize = 20_000;
/// No real prompt averages more bytes per token than this (English prose and
/// JSON tool schemas run at ~3-4, code at ~3). The reporter's truncated turns
/// ran at ~78: 160 KB of request reported as 2050 prompt tokens.
const MAX_PLAUSIBLE_BYTES_PER_TOKEN: usize = 12;
/// The ratio that is enough when the prompt count ALSO has the shape of
/// Ollama's cut ([`looks_like_ollama_cut`]). A cut to half the window is not
/// always far below 12: lfm2:24b on `/v1` reported 16,387 prompt tokens for a
/// 38,570-token request, ~9.8 bytes per token, and answered with a tool call
/// as if nothing had happened.
const CUT_SHAPED_BYTES_PER_TOKEN: usize = 6;

/// Whether `prompt` is what Ollama keeps when it cuts an over-window prompt:
/// the first few tokens plus the last half of a power-of-two window, i.e.
/// half the window plus 1-8. Measured on Ollama 0.40.2: 2050 kept of a 4096
/// window, 4099 of 8192 (command-r7b), 16,387 of 32,768 (lfm2, whose window
/// Ollama had clamped there). The default windows are powers of two (2048,
/// 4096, 8192, and BioRouter's own 65,536 and 131,072).
fn looks_like_ollama_cut(prompt: usize) -> bool {
    (11..=20).any(|bits| {
        let half = 1_usize << (bits - 1);
        (half + 1..=half + 8).contains(&prompt)
    })
}

/// `/api/show` facts, process-wide, keyed by (base URL, model). Process-wide
/// rather than per instance because a reasoning-effort turn rebuilds the
/// provider (`Agent::provider_with_effort`), and each rebuild would otherwise
/// ask again. A failed probe is never cached.
static MODEL_FACTS: LazyLock<Mutex<HashMap<(String, String), ModelFacts>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `/api/version` per base URL, so the old-server warning is logged once per
/// server per process. Only an answer is cached: a server that was still
/// starting is asked again on the next request.
static SERVER_VERSIONS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

type LoadedModelMemoryKey = (String, String);
type LoadedModelMemoryMap = HashMap<LoadedModelMemoryKey, Arc<LoadedModelMemory>>;

/// Shared across provider rebuilds for reasoning effort and the desktop's
/// context gauge, so those cannot undo a post-load downgrade.
static LOADED_MODEL_MEMORY: LazyLock<Mutex<LoadedModelMemoryMap>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct LoadedModelMemory {
    context_cap: AtomicUsize,
    checked_windows: tokio::sync::Mutex<HashSet<usize>>,
}

struct LoadedModelObserver {
    memory: Arc<LoadedModelMemory>,
    system_memory_bytes: u64,
}

impl LoadedModelObserver {
    fn correction(&self, payload: &mut Value, check: &mut TurnCheck) {
        if let Some(window) = check.num_ctx {
            let window = self.memory.cap(window);
            payload["options"]["num_ctx"] = json!(window);
            check.num_ctx = Some(window);
        }
    }
}

#[derive(serde::Deserialize)]
struct LoadedModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    model: String,
    size: u64,
    size_vram: u64,
    context_length: usize,
}

#[derive(serde::Deserialize)]
struct LoadedModels {
    models: Vec<LoadedModel>,
}

fn normalized_ollama_model(name: &str) -> Cow<'_, str> {
    let name = name.strip_prefix("registry.ollama.ai/").unwrap_or(name);
    let name = name.strip_prefix("library/").unwrap_or(name);
    if name
        .rsplit('/')
        .next()
        .is_some_and(|part| part.contains(':'))
    {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{name}:latest"))
    }
}

fn same_ollama_model(left: &str, right: &str) -> bool {
    normalized_ollama_model(left) == normalized_ollama_model(right)
}

fn loaded_model_needs_smaller_window(
    model: &LoadedModel,
    system_memory_bytes: u64,
    available_memory_bytes: Option<u64>,
) -> bool {
    model.size > 0
        && (model.size_vram < model.size
            || (system_memory_bytes > 0
                && (model.size > system_memory_bytes.saturating_sub(system_memory_bytes / 4)
                    || (model.size >= system_memory_bytes / 4
                        && available_memory_bytes
                            .is_some_and(|available| available < system_memory_bytes / 5)))))
}

impl LoadedModelMemory {
    fn cap(&self, window: usize) -> usize {
        let cap = self.context_cap.load(Ordering::Relaxed);
        if cap > 0 {
            window.min(cap)
        } else {
            window
        }
    }

    async fn assess(
        &self,
        client: &ApiClient,
        model: &str,
        requested_context: usize,
        system_memory_bytes: u64,
    ) {
        let mut checked = self.checked_windows.lock().await;
        if checked.contains(&requested_context) {
            return;
        }
        let probe = async {
            let response = client.response_get("api/ps").await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let loaded: LoadedModels = response.json().await.ok()?;
            loaded.models.into_iter().find(|loaded| {
                same_ollama_model(&loaded.name, model) || same_ollama_model(&loaded.model, model)
            })
        };
        let Some(loaded) = tokio::time::timeout(PROBE_TIMEOUT, probe)
            .await
            .ok()
            .flatten()
        else {
            return;
        };
        // Some Ollama runners report the aggregate cache across parallel
        // slots, others the request window. Ignore an incompatible allocation
        // that another client may have loaded meanwhile.
        if requested_context == 0
            || loaded.context_length < requested_context
            || !loaded.context_length.is_multiple_of(requested_context)
        {
            return;
        }
        checked.insert(requested_context);
        let available_memory_bytes = (system_memory_bytes > 0)
            .then(|| {
                sys_info::mem_info()
                    .ok()
                    .map(|memory| memory.avail.saturating_mul(1024))
            })
            .flatten();
        if !loaded_model_needs_smaller_window(&loaded, system_memory_bytes, available_memory_bytes)
        {
            return;
        }
        if requested_context > OLLAMA_AGENT_CONTEXT_SIZE {
            self.context_cap
                .store(OLLAMA_AGENT_CONTEXT_SIZE, Ordering::Relaxed);
            tracing::warn!(
                model,
                size = loaded.size,
                size_vram = loaded.size_vram,
                context_length = loaded.context_length,
                system_memory_bytes,
                available_memory_bytes,
                "Ollama's loaded model spills outside GPU memory, exceeds 75% of system memory, \
                 or takes at least 25% while less than 20% of system memory is available; \
                 reducing the automatic context window to 65536 for later requests (one reload). \
                 OLLAMA_NUM_PARALLEL multiplies the context cache; reduce it on the Ollama server \
                 or set OLLAMA_NUM_CTX explicitly to choose your own window"
            );
        } else {
            tracing::warn!(
                model,
                size = loaded.size,
                size_vram = loaded.size_vram,
                context_length = loaded.context_length,
                system_memory_bytes,
                available_memory_bytes,
                "Ollama's loaded model still spills outside GPU memory or leaves too little system \
                 memory at the automatic context floor; keeping this window so the agent prompt \
                 can fit. Reduce OLLAMA_NUM_PARALLEL on the Ollama server, unload another model, \
                 or set OLLAMA_NUM_CTX explicitly"
            );
        }
    }
}

/// The memory Ollama's context window is sized by, and what kind it is:
/// `apple_unified`, `cpu_only` (an Intel Mac), `vram` or `unknown_vram`. `None`
/// when it could not be measured. May run `system_profiler` or `nvidia-smi` on
/// its first call (cached after), so call it off the async workers.
///
/// The Llama Server sidecar's figure is the right one everywhere but Linux.
/// There the sidecar answers about ITS OWN build — the bundled llama-server is
/// the CPU one, so every packaged install says `cpu_only` — and reports system
/// RAM. Ollama ships CUDA on Linux, so what it can hold is a GPU's VRAM: sized
/// by RAM, a 64 GiB box with a 12 GB GPU got the 128k window, and the KV cache
/// for it spilled onto the processor. So Linux reads the GPU (NVIDIA only;
/// anything else is unmeasured, which sizes at the 64k floor).
pub fn sizing_memory() -> (Option<u64>, &'static str) {
    use super::llamacpp_sidecar;
    if cfg!(target_os = "linux") {
        return match llamacpp_sidecar::discrete_vram_gib() {
            Some(gib) => (Some(gib), "vram"),
            None => (None, "unknown_vram"),
        };
    }
    (
        llamacpp_sidecar::accelerator_memory_gib(),
        llamacpp_sidecar::accelerator_memory_kind(),
    )
}

/// [`sizing_memory`]'s figure, read once per process (0 when unmeasured).
static MEMORY_GIB: tokio::sync::OnceCell<u64> = tokio::sync::OnceCell::const_new();

async fn memory_gib() -> u64 {
    *MEMORY_GIB
        .get_or_init(|| async {
            let gib = tokio::task::spawn_blocking(|| sizing_memory().0.unwrap_or(0))
                .await
                .unwrap_or(0);
            // Logged once per process. Such a machine still gets the default
            // model rather than a smaller one; it is told what to expect.
            if gib > 0 && gib < OLLAMA_RECOMMENDED_MIN_MEMORY_GIB {
                tracing::warn!(
                    memory_gib = gib,
                    "This machine has less GPU-addressable memory than the {OLLAMA_RECOMMENDED_MIN_MEMORY_GIB} GiB \
                     the supported local models need (Qwen3.8 27B and Gemma 4 31B take 17-20 GB at Q4 \
                     plus the context cache): expect very slow replies, or a load failure"
                );
            }
            gib
        })
        .await
}

/// Resolve an `OLLAMA_HOST` value to a base URL exactly the way Ollama's own
/// client does (`envconfig.Host()`), because the same variable is what
/// `ollama list` and `ollama run` read: one value must reach one server.
///
/// - No scheme (`localhost`, `192.168.1.10`, `::1`, `gpu-box:11500`): `http`,
///   and port 11434 unless one is given.
/// - `http://` with no port: 80. `https://` with no port: 443. A URL means what
///   a URL means — `OLLAMA_HOST=http://gpu-box` is Ollama behind a proxy on
///   port 80, and the Ollama CLI goes there.
///
/// The old `from_env` decided "explicit port" with `host.contains(':')`, which
/// the colon of `http://` satisfies: right for `http://gpu-box` (80, as Ollama
/// does) but wrong for a bare LAN address, which it also sent to 80 where
/// Ollama uses 11434. An `https` URL keeps 443 (a TLS proxy in front of
/// Ollama), and an explicit `:80` is kept — `Url` drops a scheme's default port
/// when parsing, which is why the port is looked for in the text.
pub fn resolve_ollama_base_url(host: &str) -> Result<Url> {
    resolve_host(host, 80)
}

/// Resolve a `custom_providers` file's `base_url` for an Ollama-engine
/// provider. As [`resolve_ollama_base_url`], except that `http://host` with no
/// port means 11434: that is what an Ollama-engine `base_url` has always meant
/// here, and it is BioRouter's own field, not a value shared with Ollama.
fn resolve_custom_base_url(base_url: &str) -> Result<Url> {
    resolve_host(base_url, OLLAMA_DEFAULT_PORT)
}

/// The resolver behind both: `portless_http` is the port an explicit `http://`
/// URL with no port gets.
fn resolve_host(host: &str, portless_http: u16) -> Result<Url> {
    let host = host.trim();
    let host = if host.is_empty() { OLLAMA_HOST } else { host };
    // A bare IPv6 literal (`::1`) needs brackets before it can be a URL host.
    let host = match host.parse::<std::net::Ipv6Addr>() {
        Ok(_) => format!("[{host}]"),
        Err(_) => host.to_string(),
    };
    let has_scheme = host.contains("://");
    let text = if has_scheme {
        host
    } else {
        format!("http://{host}")
    };
    let mut url =
        Url::parse(&text).map_err(|e| anyhow::anyhow!("Invalid Ollama host '{text}': {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!(
            "Invalid Ollama host '{text}': scheme '{}' is not http or https",
            url.scheme()
        );
    }
    if url.host_str().is_none_or(str::is_empty) {
        anyhow::bail!("Invalid Ollama host '{text}': no host name");
    }
    if url.scheme() == "http" && !has_explicit_port(&text) {
        let port = if has_scheme {
            portless_http
        } else {
            OLLAMA_DEFAULT_PORT
        };
        url.set_port(Some(port))
            .map_err(|_| anyhow::anyhow!("Invalid Ollama host '{text}': cannot set a port"))?;
    }
    Ok(url)
}

/// Whether the authority of `url_text` names a port, read from the text.
fn has_explicit_port(url_text: &str) -> bool {
    let rest = url_text
        .split_once("://")
        .map_or(url_text, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let port = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed
            .split_once(']')
            .and_then(|(_, after)| after.strip_prefix(':'))
    } else {
        host_port.split_once(':').map(|(_, port)| port)
    };
    port.is_some_and(|port| !port.is_empty())
}

/// The window to send as `num_ctx`, in order of precedence:
///
/// 1. `OLLAMA_NUM_CTX` (`config_override`);
/// 2. a context limit the user set on the model config (`model_override`);
/// 3. the memory tier — [`OLLAMA_LARGE_CONTEXT_SIZE`] with at least
///    [`OLLAMA_LARGE_CONTEXT_MEMORY_GIB`] of GPU-addressable memory, else
///    [`OLLAMA_AGENT_CONTEXT_SIZE`], never less, because BioRouter's own
///    instructions and tools fill ~31-40k tokens.
///
/// Whatever is chosen is capped at the window the model was trained for
/// (`trained`, from `/api/show`): Ollama clamps `num_ctx` to it anyway, and the
/// number reported to compaction must be the one the model really has.
pub fn choose_num_ctx(
    config_override: Option<usize>,
    model_override: Option<usize>,
    trained: Option<usize>,
    memory_gib: u64,
) -> usize {
    let tier = if memory_gib >= OLLAMA_LARGE_CONTEXT_MEMORY_GIB {
        OLLAMA_LARGE_CONTEXT_SIZE
    } else {
        OLLAMA_AGENT_CONTEXT_SIZE
    };
    let chosen = config_override.or(model_override).unwrap_or(tier);
    trained.map_or(chosen, |trained| chosen.min(trained))
}

/// The context limit the user set on `model`, if they set one.
///
/// `ModelConfig::context_limit` is also filled in from a guess on the model's
/// NAME (`model.rs`'s pattern table — "llama" or "qwen2" in a tag read as
/// 128k), and a guess must not be sent as `num_ctx`: it is the number that told
/// BioRouter it had 128k while Ollama served 4k. So a limit counts as the
/// user's when `BIOROUTER_CONTEXT_LIMIT` is set, or when it differs from what
/// the name alone resolves to (an explicit `context_limit` on `/agent/start`, a
/// value this provider reported earlier and a rebuilt provider carried over).
fn user_context_limit(model: &ModelConfig) -> Option<usize> {
    let limit = model.context_limit?;
    if std::env::var("BIOROUTER_CONTEXT_LIMIT").is_ok() {
        return Some(limit);
    }
    (limit != ModelConfig::context_window_for(&model.model_name)).then_some(limit)
}

/// `keep_alive` as Ollama reads it: a number is seconds (negative keeps the
/// model loaded forever), anything else a Go duration string such as `30m`.
fn keep_alive_value(raw: &str) -> Option<Value> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(match raw.parse::<i64>() {
        Ok(seconds) => json!(seconds),
        Err(_) => json!(raw),
    })
}

/// `OLLAMA_KEEP_ALIVE` as the config layer hands it over, in wire form.
///
/// Read as a JSON value, not a string: `Config::get_param` turns an
/// environment value of `-1` or `3600` into a NUMBER before deserializing (and
/// a YAML `-1` is one already), so reading it as a `String` failed for exactly
/// Ollama's documented "keep the model loaded" value and silently sent the
/// 30-minute default instead — which then overrode the server's own setting.
fn keep_alive_setting(value: Option<Value>) -> Option<Value> {
    match value {
        None => keep_alive_value(OLLAMA_DEFAULT_KEEP_ALIVE),
        Some(Value::Number(seconds)) => Some(Value::Number(seconds)),
        Some(Value::String(raw)) => keep_alive_value(&raw),
        Some(other) => {
            tracing::warn!(
                value = %other,
                "OLLAMA_KEEP_ALIVE is neither a number of seconds nor a duration such as \
                 \"30m\"; using {OLLAMA_DEFAULT_KEEP_ALIVE}"
            );
            keep_alive_value(OLLAMA_DEFAULT_KEEP_ALIVE)
        }
    }
}

/// How an Ollama of a given version cuts and counts a prompt, which decides
/// what [`TurnCheck`] can judge from a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServerGeneration {
    /// 0.12.6 or later: refuses an over-window prompt (`truncate: false`) and
    /// counts the whole prompt. Also a development build: Ollama built from
    /// source reports `0.0.0` (`version.Version`'s placeholder, overwritten
    /// only by release builds), and a current source build honours `truncate`
    /// — treating it as ancient held back every streamed reply and logged an
    /// "update Ollama" warning at a server that was already current. A version
    /// that does not parse is treated the same way.
    Current,
    /// 0.4.0 to 0.12.5: cuts an over-window prompt, but reports its whole size,
    /// so a cut is visible in the reply's prompt count.
    Cuts,
    /// Before 0.4.0: cuts, and reports only the tokens processed after the
    /// cached prefix ([`OLLAMA_WHOLE_PROMPT_COUNT_VERSION`]), so a cut and a
    /// cache hit look the same and nothing can be judged from the count.
    CutsAndCountsOnlyNewTokens,
}

fn server_generation(version: &str) -> ServerGeneration {
    match parse_version(version) {
        None | Some((0, 0, 0)) => ServerGeneration::Current,
        Some(version) if version >= OLLAMA_MIN_TRUNCATE_VERSION => ServerGeneration::Current,
        Some(version) if version >= OLLAMA_WHOLE_PROMPT_COUNT_VERSION => ServerGeneration::Cuts,
        Some(_) => ServerGeneration::CutsAndCountsOnlyNewTokens,
    }
}

/// `major.minor.patch` from an `/api/version` answer (`0.40.2`, `0.12.6-rc1`).
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// What a request is for. A title gets thinking off and a small output budget —
/// but the SAME window as a turn, so it never reloads the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Turn,
    Title,
}

/// The settings an instance reads once, at construction.
#[derive(Clone, Debug, Default)]
struct OllamaSettings {
    /// `OLLAMA_NUM_CTX`.
    num_ctx: Option<usize>,
    /// `OLLAMA_KEEP_ALIVE`, already in wire form.
    keep_alive: Option<Value>,
    /// `OLLAMA_THINK`: what `Normal` effort sends. Unset means the model's own
    /// default (no `think` at all).
    think: Option<Think>,
}

impl OllamaSettings {
    fn defaults() -> Self {
        Self {
            num_ctx: None,
            keep_alive: keep_alive_value(OLLAMA_DEFAULT_KEEP_ALIVE),
            think: None,
        }
    }
}

/// The built-in provider's `OLLAMA_*` settings, from the environment or
/// `config.yaml` (the environment wins, as for every setting).
fn read_settings(config: &crate::config::Config) -> OllamaSettings {
    let num_ctx = config
        .get_param::<usize>("OLLAMA_NUM_CTX")
        .ok()
        .filter(|&n| n > 0);
    let keep_alive = keep_alive_setting(config.get_param::<Value>("OLLAMA_KEEP_ALIVE").ok());
    let think = config
        .get_param::<Value>("OLLAMA_THINK")
        .ok()
        .and_then(|value| Think::from_json(&value));
    OllamaSettings {
        num_ctx,
        keep_alive,
        think,
    }
}

/// The check every reply passes before it is handed on: did the model answer,
/// and did it see the whole prompt?
#[derive(Clone, Debug)]
struct TurnCheck {
    /// Bytes of text in the request, images excluded (`native::text_bytes`).
    text_bytes: usize,
    /// The window BioRouter asked for; `None` on the `/v1` fallback, which
    /// cannot ask.
    num_ctx: Option<usize>,
    model: String,
    /// `/api/version`, when the server answered it.
    server_version: Option<String>,
    /// Whether the reply's prompt count can be judged at all. False for an
    /// Ollama before 0.4.0, whose count leaves out the cached prefix: there the
    /// second request of every tool loop reported ~300 tokens for ~130 KB, the
    /// ratio test failed it as truncated with a "596-token window", and the
    /// agent ended a turn that had done nothing wrong.
    judge_prompt_size: bool,
    /// Hold the reply's TEXT too, not only its tool calls, until the prompt
    /// size has been checked. Set where a cut is possible and can be seen — the
    /// `/v1` fallback, an Ollama from 0.4.0 to 0.12.5, and one that would not
    /// say its version — because there a reply streamed as it arrives is shown
    /// to the user before anyone knows the model never saw the question: behind
    /// a `/v1`-only server the user read the model's guess at a question cut to
    /// 2050 tokens, followed by the error. A current Ollama refuses instead of
    /// cutting, so its replies stream as before.
    hold_text: bool,
    /// The BioRouter setting that changes the window, named in the error;
    /// `None` on the `/v1` fallback, where only the server's own setting can.
    window_setting: Option<&'static str>,
}

impl TurnCheck {
    /// The check for a native `/api/chat` request, from what the server said
    /// its version is.
    fn native(
        text_bytes: usize,
        num_ctx: usize,
        model: &str,
        server_version: Option<String>,
        window_setting: &'static str,
    ) -> Self {
        let generation = server_version.as_deref().map(server_generation);
        Self {
            text_bytes,
            num_ctx: Some(num_ctx),
            model: model.to_string(),
            judge_prompt_size: generation != Some(ServerGeneration::CutsAndCountsOnlyNewTokens),
            hold_text: matches!(generation, None | Some(ServerGeneration::Cuts)),
            window_setting: Some(window_setting),
            server_version,
        }
    }

    /// The check for the `/v1/chat/completions` fallback: no window can be
    /// asked for, so a cut is possible on every request.
    fn openai_compat(text_bytes: usize, model: &str) -> Self {
        Self {
            text_bytes,
            num_ctx: None,
            model: model.to_string(),
            server_version: None,
            judge_prompt_size: true,
            hold_text: true,
            window_setting: None,
        }
    }

    /// The error for a reply to a prompt the server cut, or `None`.
    ///
    /// Ollama before 0.12.6 ignores `truncate: false`, and a server behind the
    /// `/v1` fallback may cut too. Either way the HTTP status is 200 and only
    /// the prompt size gives it away: the reporter's 160 KB requests came back
    /// as 2050 prompt tokens. A reply to a prompt the model never saw must not
    /// be returned as an answer — its tool calls in particular are guesses at
    /// tools whose definitions were cut — so it fails with a sentence that says
    /// what happened and what to change.
    fn truncation(&self, usage: &Usage) -> Option<ProviderError> {
        if !self.judge_prompt_size {
            return None;
        }
        let prompt = i64::from(usage.input_tokens.unwrap_or(0))
            + i64::from(usage.cache_read_input_tokens.unwrap_or(0));
        // No count reported: nothing to judge by.
        let prompt = usize::try_from(prompt).ok().filter(|&n| n > 0)?;
        let bound = if looks_like_ollama_cut(prompt) {
            CUT_SHAPED_BYTES_PER_TOKEN
        } else {
            MAX_PLAUSIBLE_BYTES_PER_TOKEN
        };
        if self.text_bytes <= TRUNCATION_CHECK_MIN_BYTES
            || prompt.saturating_mul(bound) >= self.text_bytes
        {
            return None;
        }
        // Both estimates feed the canonical overflow sentence, which the agent
        // reads back (`utils::context_overflow_sizes`) to decide whether
        // compacting can help at all. Without it, a server whose window cannot
        // hold BioRouter's instructions and tools sent the agent through four
        // compaction rounds, each answered from a cut prompt again (measured:
        // four rounds of 2050 prompt tokens behind a `/v1`-only server).
        //
        // The request at ~4 bytes per token (BioRouter's ~130 KB prompt
        // measured 4.3-4.6). The window from how Ollama cuts: it keeps
        // `n_ctx - (n_ctx - 4) / 2` tokens, so the 4096 window kept 2050.
        let expected = self.text_bytes / 4;
        let window = prompt.saturating_mul(2).saturating_sub(4);
        let asked = self
            .num_ctx
            .map(|n| format!(" BioRouter asked for a {n}-token window."))
            .unwrap_or_default();
        let server = match self.server_version.as_deref() {
            Some(version) if server_generation(version) == ServerGeneration::Cuts => format!(
                " This server is Ollama {version}, which ignores BioRouter's request to refuse \
                 an over-long prompt rather than cut it (that needs Ollama 0.12.6 or later): \
                 update Ollama."
            ),
            Some(version) => format!(" The server reports Ollama {version}."),
            None => String::new(),
        };
        // Name the setting that really moves this provider's window. On the
        // native path that is BioRouter's, because the `num_ctx` it sends
        // overrides the server's own default; on `/v1` BioRouter cannot send
        // one, so only the server's setting can.
        let raise = match self.window_setting {
            Some(setting) => format!("Raise {setting} (the window BioRouter asks Ollama for)"),
            None => "Raise the server's own context window (OLLAMA_CONTEXT_LENGTH for an Ollama \
                     server; BioRouter cannot set the window through /v1/chat/completions)"
                .to_string(),
        };
        Some(ProviderError::ContextLengthExceeded(format!(
            "{} answered without seeing the whole conversation, so its reply was discarded: \
             the server processed only {prompt} prompt tokens, so it cut the prompt to fit its \
             context window. BioRouter estimates the {}.{asked}{server} {raise}, or turn off \
             extensions you are not using to shrink BioRouter's instructions and tool list.",
            self.model,
            super::utils::context_overflow_message(expected as u64, window as u64),
        )))
    }
}

/// Whether `message` is an answer: text, or a tool call. Thinking is not.
fn is_answer(message: &Message) -> bool {
    message.is_tool_call()
        || message.content.iter().any(|content| match content {
            MessageContent::Text(text) => !text.text.trim().is_empty(),
            _ => false,
        })
}

fn no_answer_error(finish_reason: Option<&str>) -> ProviderError {
    let detail = if finish_reason == Some("length") {
        "The response reached its token limit before producing an answer. Review the model's output budget or choose another model."
    } else {
        "The model may have returned reasoning only. Retry or choose another model; no answer or tool result was produced."
    };
    ProviderError::RequestFailed(format!(
        "Ollama completed without answer text or tool calls. {detail}"
    ))
}

/// Output tokens beyond what an empty reply's reasoning accounts for that mark
/// it as a SWALLOWED tool call rather than reasoning only (R20). The smallest
/// call is about ten tokens (`{"name":"x","arguments":{}}`), and the smallest
/// swallowed reply measured on Ollama 0.40.2 was 13 (mistral-small3.2's
/// `follow_chain`); a threshold of 16 read that one as "reasoning only".
const SWALLOWED_CALL_MIN_TOKENS: i32 = 8;

/// Thinking characters per token for the reasoning estimate, rounded UP so
/// the estimate is a ceiling. Measured across every thinking capture
/// (qwen3:1.7b, gpt-oss:20b, deepseek-r1:8b, glm-4.7-flash on Ollama 0.40.2),
/// reasoning plus answer ran 2.4-4.7 characters per token: gpt-oss's
/// `no_tool` was 102 characters in 42 tokens. An estimate at 4.5 read
/// qwen3:1.7b's reasoning-only reply (389 characters, about 133 tokens) and
/// gpt-oss's (214 characters, about 85) as swallowed calls.
const THINKING_CHARS_PER_TOKEN: usize = 2;

/// Tokens a thinking reply spends outside its reasoning text: the think tags
/// or Harmony's channel markers, and the end-of-turn token.
const THINKING_OVERHEAD_TOKENS: i32 = 16;

/// What an empty reply — no answer text, no tool call — most likely was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptyReply {
    /// Cut at its output limit (`done_reason: length`).
    CutAtLimit,
    /// More output tokens than its reasoning can account for: a tool call
    /// Ollama's parser buffered and could not parse (R20).
    SwallowedCall(i32),
    /// Reasoning and nothing after it — or a call Ollama swallowed behind the
    /// reasoning: when a model thinks, its token count cannot tell the two
    /// apart. qwen3:1.7b's swallowed `single` (482 characters of thinking that
    /// end "I'll format the tool call accordingly", 135 tokens) and its
    /// answered `no_tool` (401 characters, 137 tokens) run at the same 3-3.6
    /// characters per token.
    ReasoningOnly(Option<i32>),
    /// Nothing at all.
    Nothing,
}

/// The two supported local models, minus the one in use: advising a
/// qwen3.8:27b user to switch to qwen3.8:27b reads as a bug.
fn other_supported_models(model: &str) -> Vec<&'static str> {
    OLLAMA_RECOMMENDED_MODELS
        .iter()
        .map(|recommended| recommended.tag)
        .filter(|tag| model != *tag && !model.starts_with(&format!("{tag}-")))
        .collect()
}

/// What to do about a model whose reply came back empty.
fn empty_reply_advice(model: &str) -> String {
    let others = other_supported_models(model);
    if others.len() < OLLAMA_RECOMMENDED_MODELS.len() {
        // The model in use is a supported one.
        return match others.as_slice() {
            [] => "Retry.".to_string(),
            others => format!("Retry; if it keeps happening, try {}.", others.join(" or ")),
        };
    }
    format!(
        "Retry, or choose a model that calls tools reliably through Ollama ({}).",
        others.join(" or ")
    )
}

/// The most tool names a corrective message lists. BioRouter offers a few
/// dozen; the cap only bounds a pathological roster.
const MAX_CORRECTION_TOOL_NAMES: usize = 64;

/// The check every reply passes before it is handed on: [`TurnCheck`] on its
/// usage, and a record of whether it answered at all.
///
/// Items that carry a tool call are held until the reply's usage has been
/// checked, so a call made from a truncated prompt never reaches the agent.
/// (The native decoder already yields calls together with the usage; the `/v1`
/// fallback's decoder may not.) Text streams through as it arrives, unless
/// [`TurnCheck::hold_text`] says the server might cut the prompt.
struct TurnGuard {
    check: TurnCheck,
    answered: bool,
    finish_reason: Option<String>,
    usage_checked: bool,
    held: Vec<ProviderStreamItem>,
    output_tokens: Option<i32>,
    thinking_chars: usize,
}

impl TurnGuard {
    fn new(check: TurnCheck) -> Self {
        Self {
            check,
            answered: false,
            finish_reason: None,
            usage_checked: false,
            held: Vec::new(),
            output_tokens: None,
            thinking_chars: 0,
        }
    }

    /// The items ready to hand on, in order, or the truncation error.
    fn push(&mut self, item: ProviderStreamItem) -> Result<Vec<ProviderStreamItem>, ProviderError> {
        if let Some(message) = &item.0 {
            self.answered |= is_answer(message);
            self.thinking_chars += message
                .content
                .iter()
                .map(|content| match content {
                    MessageContent::Thinking(thinking) => thinking.thinking.chars().count(),
                    _ => 0,
                })
                .sum::<usize>();
        }
        if let Some(usage) = &item.1 {
            if let Some(reason) = &usage.finish_reason {
                self.finish_reason = Some(reason.clone());
            }
            if usage.usage.output_tokens.is_some() {
                self.output_tokens = usage.usage.output_tokens;
            }
            if let Some(error) = self.check.truncation(&usage.usage) {
                return Err(error);
            }
            self.usage_checked = true;
        }
        let carries_call = item.0.as_ref().is_some_and(Message::is_tool_call);
        if !self.usage_checked && (self.check.hold_text || carries_call || !self.held.is_empty()) {
            self.held.push(item);
            return Ok(Vec::new());
        }
        let mut ready: Vec<ProviderStreamItem> = self.held.drain(..).collect();
        ready.push(item);
        Ok(ready)
    }

    /// Whatever is still held. No usage ever arrived: there is nothing to judge
    /// truncation by.
    fn finish(&mut self) -> Vec<ProviderStreamItem> {
        self.held.drain(..).collect()
    }

    /// What this reply was, if it had neither text nor a tool call.
    ///
    /// Ollama's generic tool parser buffers output it takes for the start of a
    /// call, and when the call does not parse (an argument named `name` for
    /// mistral-small3.2, an unknown tool for granite4, unescaped quotes for
    /// lfm2) it emits NOTHING: HTTP 200, `content: ""`, no call, and
    /// `eval_count` 13-68. The tokens are the evidence: more output than the
    /// reasoning can account for is a call that was swallowed.
    fn empty_reply(&self) -> EmptyReply {
        if self.finish_reason.as_deref() == Some("length") {
            return EmptyReply::CutAtLimit;
        }
        let reasoning = if self.thinking_chars == 0 {
            0
        } else {
            i32::try_from(self.thinking_chars.div_ceil(THINKING_CHARS_PER_TOKEN))
                .unwrap_or(i32::MAX)
                .saturating_add(THINKING_OVERHEAD_TOKENS)
        };
        match self.output_tokens {
            Some(output) if output > reasoning.saturating_add(SWALLOWED_CALL_MIN_TOKENS) => {
                EmptyReply::SwallowedCall(output)
            }
            output if self.thinking_chars > 0 => EmptyReply::ReasoningOnly(output),
            _ => EmptyReply::Nothing,
        }
    }

    /// Whether this empty reply earns the turn's one corrective retry (R20):
    /// a swallowed call, or reasoning that may hide one. Never a reply cut at
    /// its limit, which asking again would most likely repeat.
    fn empty_reply_may_be_a_lost_call(&self) -> bool {
        matches!(
            self.empty_reply(),
            EmptyReply::SwallowedCall(_) | EmptyReply::ReasoningOnly(_)
        )
    }

    /// The error for a reply with neither text nor a tool call, saying only
    /// what its numbers show. `tools_offered`: whether the request offered
    /// tools, without which there is no tool parser to swallow anything.
    fn empty_reply_error(&self, model: &str, retried: bool, tools_offered: bool) -> ProviderError {
        let again = if retried {
            ", also after Biorouter asked it once more"
        } else {
            ""
        };
        let detail = match self.empty_reply() {
            EmptyReply::CutAtLimit => return no_answer_error(Some("length")),
            EmptyReply::SwallowedCall(output) if tools_offered => {
                let beyond = if self.thinking_chars > 0 {
                    " beyond its reasoning"
                } else {
                    ""
                };
                format!(
                    "{model} produced {output} output tokens{beyond} that Ollama could not parse \
                     as a tool call, so the reply arrived empty{again}. {}",
                    empty_reply_advice(model)
                )
            }
            EmptyReply::SwallowedCall(output) => format!(
                "{model} produced {output} output tokens but no answer text{again}. Retry or \
                 choose another model."
            ),
            EmptyReply::ReasoningOnly(output) => {
                let tokens = output
                    .map(|output| format!(" ({output} output tokens)"))
                    .unwrap_or_default();
                if tools_offered {
                    format!(
                        "{model} returned reasoning only{tokens}{again}. It may also have written \
                         a tool call that Ollama could not parse: when a model thinks, the token \
                         count cannot tell the two apart. {}",
                        empty_reply_advice(model)
                    )
                } else {
                    format!(
                        "{model} returned reasoning only{tokens}{again}. Retry or choose another \
                         model."
                    )
                }
            }
            EmptyReply::Nothing => {
                format!("the model's reply was empty{again}. Retry or choose another model.")
            }
        };
        ProviderError::RequestFailed(format!(
            "Ollama completed without answer text or tool calls: {detail}"
        ))
    }
}

/// Wrap a `/v1` fallback turn's stream with [`TurnGuard`] and the
/// answered-or-error check.
fn guard_turn(
    mut stream: MessageStream,
    check: TurnCheck,
    model: String,
    tools_offered: bool,
) -> MessageStream {
    Box::pin(try_stream! {
        let mut guard = TurnGuard::new(check);
        while let Some(item) = stream.next().await {
            for ready in guard.push(item?)? {
                yield ready;
            }
        }
        for ready in guard.finish() {
            yield ready;
        }
        if !guard.answered {
            Err(guard.empty_reply_error(&model, false, tools_offered))?;
        }
    })
}

/// The tool names a corrective message lists.
fn correction_tool_names(tools: &[Tool]) -> String {
    let mut names: Vec<&str> = tools
        .iter()
        .take(MAX_CORRECTION_TOOL_NAMES)
        .map(|tool| tool.name.as_ref())
        .collect();
    if tools.len() > MAX_CORRECTION_TOOL_NAMES {
        names.push("...");
    }
    names.join(", ")
}

/// R20's nudge, from the protocol matrix: the one retry after an empty reply.
fn empty_reply_nudge(tools: &[Tool]) -> String {
    format!(
        "Your last reply was empty. If you meant to call a tool, call one of: {}, with valid JSON \
         arguments; otherwise answer in text.",
        correction_tool_names(tools)
    )
}

/// R18's correction: the synthetic tool result for a call to a tool that does
/// not exist, so the model can try again with a real one.
fn unknown_tool_nudge(name: &str, tools: &[Tool]) -> String {
    format!(
        "There is no tool named '{name}', so nothing was run. The tools you can call are: {}. \
         Call one of them with valid JSON arguments, or answer in text.",
        correction_tool_names(tools)
    )
}

/// Whether the attempted call is something the conversation already holds,
/// so the reply is an echo rather than a request: a call an assistant turn
/// made (asking for it again could run it twice), or the call quoted in a
/// tool RESULT or an earlier assistant reply (a fetched page, a JSON file, a
/// summary). Re-asked, an echo of tool output would carry the output's
/// authority up to the user's. Generous on purpose: a false match only costs
/// a retry that is best-effort anyway.
///
/// A quote is the call's name and every string in its arguments, raw or
/// JSON-escaped, inside one message's text. User messages are not searched:
/// what a person wrote is theirs to ask for, and a request that merely
/// mentions a command must not cancel the model's attempt to run it.
fn echoes_conversation(
    payload: &Value,
    call: &super::formats::text_tool_calls::AttemptedCall,
) -> bool {
    fn strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
        match value {
            Value::String(text) if !text.trim().is_empty() => out.push(text),
            Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
            Value::Object(map) => map.values().for_each(|item| strings(item, out)),
            _ => {}
        }
    }
    fn message_text(message: &Value) -> String {
        match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }
    let mut leaves = Vec::new();
    strings(&call.arguments, &mut leaves);
    let quoted_in = |text: &str| {
        text.contains(call.name.as_str())
            && leaves.iter().all(|leaf| {
                text.contains(leaf)
                    || serde_json::to_string(leaf)
                        .ok()
                        .and_then(|escaped| {
                            escaped
                                .strip_prefix('"')
                                .and_then(|e| e.strip_suffix('"'))
                                .map(|escaped| text.contains(escaped))
                        })
                        .unwrap_or(false)
            })
    };
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    messages.iter().any(|message| {
        let role = message.get("role").and_then(Value::as_str);
        let made = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|made| {
                made.pointer("/function/name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        call.meant.iter().any(|meant| meant == name) || name == call.name
                    })
                    && made.pointer("/function/arguments") == Some(&call.arguments)
            });
        made || (matches!(role, Some("tool" | "assistant")) && quoted_in(&message_text(message)))
    })
}

/// The correction for a whole reply that is a tool call written as text
/// ([`super::formats::text_tool_calls::attempted_call`]). It asks; it never
/// tells the model to run anything, and it never claims what did or did not
/// run before — only that text is not a call.
fn attempted_call_nudge(
    call: &super::formats::text_tool_calls::AttemptedCall,
    tools: &[Tool],
) -> String {
    let meant = call
        .meant
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(" or ");
    if call.meant.len() == 1 && call.meant[0] == call.name {
        format!(
            "Your last reply was a call to '{}' written as text, and text in a reply is never \
             run as a tool call. If you intended to call {meant}, make it as a tool call with \
             arguments that match its parameters; otherwise answer in text.",
            call.name
        )
    } else {
        format!(
            "Your last reply was a call to '{}' written as text, and text in a reply is never \
             run as a tool call; there is also no tool with that name. If you intended to call \
             {meant}, make it as a tool call; otherwise answer in text. The tools you can call \
             are: {}.",
            call.name,
            correction_tool_names(tools)
        )
    }
}

/// The correction for a whole reply that imitates a tool RESULT
/// ([`super::formats::text_tool_calls::forged_tool_result`]). Like
/// [`attempted_call_nudge`] it asks and never tells: it says only that a
/// result written as text is not a call.
fn forged_result_nudge(
    forged: &super::formats::text_tool_calls::ForgedResult,
    tools: &[Tool],
) -> String {
    if forged.meant.is_empty() {
        format!(
            "Your last reply was written as the result of a call to '{}', but tool results come \
             only from Biorouter after a real tool call, and text in a reply is never run; there \
             is also no tool with that name. If you meant to use a tool, make a tool call to one \
             of: {}, with arguments that match its parameters; otherwise answer in text.",
            forged.tool,
            correction_tool_names(tools)
        )
    } else {
        format!(
            "Your last reply was written as the result of a call to '{}', but tool results come \
             only from Biorouter after a real tool call, and text in a reply is never run. If you \
             meant to call {}, make it as a tool call with arguments that match its parameters; \
             otherwise answer in text.",
            forged.tool,
            forged
                .meant
                .iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>()
                .join(" or ")
        )
    }
}

/// Whether an imitated result's body is something the conversation already
/// holds — a real tool result, or an earlier reply — so the reply is a relay
/// rather than an invention. Asked again, a relayed result could run its tool
/// a second time. An empty body is never an echo: it relays nothing.
fn echoes_tool_output(
    payload: &Value,
    forged: &super::formats::text_tool_calls::ForgedResult,
) -> bool {
    if forged.body.is_empty() {
        return false;
    }
    payload
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("tool" | "assistant")
            )
        })
        .any(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains(&forged.body))
        })
}

/// `payload` with the reply so far (if any) and a corrective user message
/// appended: the body of the one corrective retry a turn may make.
fn with_correction(payload: &Value, reply_so_far: &str, nudge: &str) -> Value {
    let mut payload = payload.clone();
    if let Some(messages) = payload.get_mut("messages").and_then(Value::as_array_mut) {
        if !reply_so_far.trim().is_empty() {
            messages.push(json!({"role": "assistant", "content": reply_so_far}));
        }
        messages.push(json!({"role": "user", "content": nudge}));
    }
    payload
}

/// Fold a guarded reply stream into one message, for the blocking calls
/// (compaction, knowledge macros, session titles). They stream too: a
/// non-streamed `/api/chat` sends nothing until the whole generation is done, so
/// under an idle-only timeout its silence was prefill PLUS generation, and a
/// long compaction summary could outlast `OLLAMA_TIMEOUT` and fail unretried.
/// Streamed, every token resets the silence clock, exactly as for a turn.
async fn collect_reply(
    mut stream: MessageStream,
    model: &str,
) -> Result<(Message, ProviderUsage), ProviderError> {
    let mut thinking = String::new();
    let mut text = String::new();
    let mut calls = Vec::new();
    let mut usage = None;
    while let Some(item) = stream.next().await {
        let (message, item_usage, _pending) = item?;
        for content in message.into_iter().flat_map(|message| message.content) {
            match content {
                MessageContent::Text(delta) => text.push_str(&delta.text),
                MessageContent::Thinking(delta) => thinking.push_str(&delta.thinking),
                other => calls.push(other),
            }
        }
        if item_usage.is_some() {
            usage = item_usage;
        }
    }
    let mut content = Vec::new();
    if !thinking.trim().is_empty() {
        content.push(MessageContent::thinking(thinking, ""));
    }
    if !text.is_empty() {
        content.push(MessageContent::text(text));
    }
    content.extend(calls);
    let message = Message::new(
        rmcp::model::Role::Assistant,
        chrono::Utc::now().timestamp(),
        content,
    );
    let usage = usage.unwrap_or_else(|| ProviderUsage::new(model.to_string(), Usage::default()));
    Ok((message, usage))
}

/// An error out of the format module: the [`ProviderError`] it carries, or a
/// decode failure.
fn provider_error(error: anyhow::Error) -> ProviderError {
    error.downcast::<ProviderError>().unwrap_or_else(|error| {
        ProviderError::RequestFailed(format!("Stream decode error: {error}"))
    })
}

/// The outcome of one `/api/chat` POST.
enum NativeOutcome {
    Ok(Response),
    /// HTTP 404 that is not Ollama's "model not found": the server has no
    /// `/api/chat` at all. A declarative Ollama-engine provider can point at any
    /// OpenAI-compatible server; that one gets the `/v1` path instead.
    RouteMissing,
    /// The model called a tool the request did not offer, and Ollama refused
    /// the whole reply with `{"error": "tool '<name>' not found"}` (an HTTP
    /// 500 from ministral-3:8b). The model's mistake (R18): it gets one
    /// corrective retry, and it is never retried as a server error.
    UnknownTool(String),
}

/// POST `/api/chat` once. Errors are mapped with the server's own words.
async fn post_native(
    client: &ApiClient,
    payload: &Value,
    model: &str,
    num_ctx: Option<usize>,
    window_setting: &str,
) -> Result<NativeOutcome, ProviderError> {
    let response = client.response_post(OLLAMA_CHAT_PATH, payload).await?;
    let status = response.status();
    if status.is_success() {
        return Ok(NativeOutcome::Ok(response));
    }
    let body = response.text().await.unwrap_or_default();
    if status == StatusCode::NOT_FOUND && !native::is_model_not_found(&body) {
        return Ok(NativeOutcome::RouteMissing);
    }
    let text = native::error_text_from_body(&body);
    if let Some(name) = native::unknown_tool_name(&text) {
        tracing::info!(
            status = status.as_u16(),
            "Ollama refused a reply that called a tool the request did not offer"
        );
        return Ok(NativeOutcome::UnknownTool(name));
    }
    let error = native::map_error(Some(status), &text, model, num_ctx, window_setting);
    tracing::warn!(
        status = status.as_u16(),
        error_type = error.telemetry_type(),
        "Ollama request failed"
    );
    Err(error)
}

/// Send a turn's one corrective retry (R18, R20), with its own request log.
/// Not retried itself: it is already the retry.
async fn send_correction(
    client: &ApiClient,
    payload: &Value,
    model_config: &ModelConfig,
    num_ctx: Option<usize>,
    window_setting: &'static str,
) -> Result<(Response, RequestLog), ProviderError> {
    let mut log = RequestLog::start(model_config, payload)?;
    let outcome = post_native(
        client,
        payload,
        &model_config.model_name,
        num_ctx,
        window_setting,
    )
    .await;
    let error = match outcome {
        Ok(NativeOutcome::Ok(response)) => return Ok((response, log)),
        Ok(NativeOutcome::UnknownTool(name)) => ProviderError::RequestFailed(
            UnknownToolCalled {
                name,
                after_text: false,
            }
            .to_string(),
        ),
        Ok(NativeOutcome::RouteMissing) => ProviderError::RequestFailed(
            "Ollama stopped answering /api/chat between two requests of one turn.".to_string(),
        ),
        Err(error) => error,
    };
    let _ = log.provider_error(&error);
    Err(error)
}

/// Decode a native NDJSON response body into stream items, logging each one.
/// A call to a tool the request did not offer is recorded in `unknown_tool`
/// (and still ends the stream as an error), so the turn can decide whether to
/// give the model its corrective retry.
fn decode_native(
    response: Response,
    mut log: RequestLog,
    options: DecodeOptions,
    unknown_tool: Arc<Mutex<Option<UnknownToolCalled>>>,
) -> MessageStream {
    let bytes = response.bytes_stream().map_err(io::Error::other);
    Box::pin(try_stream! {
        let lines = FramedRead::new(StreamReader::new(bytes), LinesCodec::new())
            .map_err(anyhow::Error::from);
        let decoded = native::response_to_streaming_message_with(lines, options);
        tokio::pin!(decoded);
        while let Some(item) = decoded.next().await {
            let item = item.map_err(|error| {
                let error = match error.downcast::<UnknownToolCalled>() {
                    Ok(called) => {
                        let mapped = ProviderError::RequestFailed(called.to_string());
                        if let Ok(mut slot) = unknown_tool.lock() {
                            *slot = Some(called);
                        }
                        mapped
                    }
                    Err(error) => provider_error(error),
                };
                let _ = log.provider_error(&error);
                error
            })?;
            log.write(&item.0, item.1.as_ref().map(|usage| &usage.usage))?;
            yield item;
        }
    })
}

/// Everything a native turn's stream owns once the first response is in.
struct NativeTurn {
    client: Arc<ApiClient>,
    /// The request as first sent; a corrective retry appends to it.
    payload: Value,
    model_config: ModelConfig,
    check: TurnCheck,
    window_setting: &'static str,
    /// Every item of the turn, retry included, carries this id, so the agent
    /// folds them into one assistant message.
    message_id: String,
    /// The tools advertised in this request (recovery, normalisation and the
    /// corrective messages).
    tools: Vec<Tool>,
    /// Whether the turn's one corrective retry is already spent.
    retried: bool,
    memory: Option<LoadedModelObserver>,
}

/// A native turn: decode, recover leaked calls (TIER1), guard, and — at most
/// once per turn — retry with a correction when the model called a tool that
/// does not exist (R18), the reply came back empty with tokens that may be a
/// lost call (R20), or the whole reply was a tool call written as text that
/// recovery did not run ([`super::formats::text_tool_calls::attempted_call`]).
///
/// The last retry follows a reply that already ended well enough to stand —
/// it answered, if only with JSON — so it is best-effort: if the corrective
/// request cannot be sent, fails, or comes back empty, the turn ends with what
/// was already shown, and that pass's usage, rather than turning a finished
/// reply into a failed turn.
// One `try_stream!` state machine, like the stream decoders in `formats/`: its
// state (the pass being read, whether the retry was spent, what was shown) is
// the generator's locals, and splitting it would move them into a struct that
// only this function reads.
#[allow(clippy::too_many_lines)]
fn native_turn(turn: NativeTurn, first: Response, first_log: RequestLog) -> MessageStream {
    Box::pin(try_stream! {
        let NativeTurn {
            client,
            payload,
            model_config,
            mut check,
            window_setting,
            message_id,
            tools,
            mut retried,
            memory,
        } = turn;
        let model = model_config.model_name.clone();
        let mut next_response = Some((first, first_log));
        // Set when a retry follows text the user has already read: the
        // retry's reply is a new assistant turn folded into the same message,
        // so its first text starts a new paragraph rather than running on
        // from the last sentence.
        let mut separate_next_text = false;
        // While a best-effort retry is in flight: the usage of the pass it
        // follows, which ends the turn if the retry does not.
        let mut fallback_usage: Option<ProviderStreamItem> = None;
        // Each pass ends in `break`, an error, or a corrective retry that sets
        // `next_response` again.
        while let Some((response, log)) = next_response.take() {
            if let Some(memory) = &memory {
                memory.memory.assess(
                    &client,
                    &model,
                    check.num_ctx.expect("a native request has a context window"),
                    memory.system_memory_bytes,
                ).await;
            }
            let unknown_tool: Arc<Mutex<Option<UnknownToolCalled>>> = Arc::default();
            let decoded = decode_native(
                response,
                log,
                DecodeOptions {
                    model_name: model.clone(),
                    window_setting,
                    message_id: Some(message_id.clone()),
                },
                unknown_tool.clone(),
            );
            let mut recovered = recover_leaked_tool_calls(
                decoded,
                RecoveryContext {
                    tools: tools.clone(),
                    provider: "ollama",
                    model: model.clone(),
                },
            );
            let mut guard = TurnGuard::new(check.clone());
            let mut shown_text = String::new();
            let mut called_a_tool = false;
            let mut failure: Option<ProviderError> = None;
            // The pass's usage is the last thing it yields, and it waits until
            // the pass is judged: a pass that is retried describes a reply the
            // turn replaced, and one turn reports one usage.
            let mut held_usage: Option<ProviderStreamItem> = None;
            let mut ready_items: Vec<ProviderStreamItem> = Vec::new();
            loop {
                let next = recovered.next().await;
                let finished = next.is_none();
                match next {
                    None => ready_items.extend(guard.finish()),
                    // Items the guard still holds stay held: without a usage
                    // nothing says the prompt arrived whole.
                    Some(Err(error)) => failure = Some(error),
                    Some(Ok(item)) => match guard.push(item) {
                        Ok(ready) => ready_items.extend(ready),
                        Err(error) => failure = Some(error),
                    },
                }
                for mut ready in ready_items.drain(..) {
                    if let Some(usage) = ready.1.take() {
                        held_usage = Some((None, Some(usage), None));
                    }
                    if ready.0.is_none() && ready.2.is_none() {
                        continue;
                    }
                    if separate_next_text && prefix_first_text(&mut ready, "\n\n") {
                        separate_next_text = false;
                    }
                    if let Some(message) = &ready.0 {
                        called_a_tool |= message.is_tool_call();
                        shown_text.push_str(&message.as_concat_text());
                    }
                    yield ready;
                }
                if finished || failure.is_some() {
                    break;
                }
            }
            if let Some(error) = failure {
                if let Some(usage) = fallback_usage.take() {
                    tracing::warn!(
                        model = model.as_str(),
                        error_type = error.telemetry_type(),
                        "the corrective retry after a call written as text failed; the turn ends \
                         with the reply already shown"
                    );
                    yield usage;
                    break;
                }
                let called = unknown_tool.lock().ok().and_then(|slot| slot.clone());
                match called {
                    Some(called) if !retried && !tools.is_empty() => {
                        // What was held is text the user would have read had
                        // the reply ended normally, and the retry carries it.
                        for mut ready in guard.finish() {
                            ready.1 = None;
                            if let Some(message) = &ready.0 {
                                shown_text.push_str(&message.as_concat_text());
                            }
                            yield ready;
                        }
                        retried = true;
                        separate_next_text = !shown_text.trim().is_empty();
                        tracing::info!(
                            model = model.as_str(),
                            after_text = called.after_text,
                            "the model called a tool this request did not offer; asking it once \
                             more, with the tool list"
                        );
                        let mut next = with_correction(
                            &payload,
                            &shown_text,
                            &unknown_tool_nudge(&called.name, &tools),
                        );
                        if let Some(memory) = &memory {
                            memory.correction(&mut next, &mut check);
                        }
                        next_response = Some(
                            send_correction(&client, &next, &model_config, check.num_ctx, window_setting)
                                .await?,
                        );
                        continue;
                    }
                    _ => Err(error)?,
                }
            }
            if guard.answered {
                // The WHOLE reply a tool call written as text that recovery
                // did not run (a name without its extension prefix, arguments
                // that did not fit, the result tag around it): the turn would
                // end on JSON — the bug report's symptom. The model is asked
                // once to make the call as a call. Never for a call quoted
                // inside prose, nor for an echo of what the conversation holds.
                // The same once for a whole reply that imitates a tool RESULT
                // in Biorouter's own frame (F04/F05): nothing ran, and the
                // turn would otherwise end on the imitation as its answer.
                let correction = (!retried
                    && !called_a_tool
                    && guard.finish_reason.as_deref() != Some("length"))
                .then(|| {
                    super::formats::text_tool_calls::attempted_call(&shown_text, &tools)
                        .filter(|call| !echoes_conversation(&payload, call))
                        .map(|call| {
                            (
                                attempted_call_nudge(&call, &tools),
                                "the model's whole reply was a tool call written as text; asking \
                                 it once more to make it as a call",
                            )
                        })
                        .or_else(|| {
                            super::formats::text_tool_calls::forged_tool_result(&shown_text, &tools)
                                .filter(|forged| !echoes_tool_output(&payload, forged))
                                .map(|forged| {
                                    (
                                        forged_result_nudge(&forged, &tools),
                                        "the model's whole reply imitated a tool result; asking \
                                         it once more to make the call",
                                    )
                                })
                        })
                })
                .flatten();
                if let Some((nudge, why)) = correction {
                    retried = true;
                    separate_next_text = true;
                    tracing::info!(model = model.as_str(), "{why}");
                    let mut next = with_correction(&payload, &shown_text, &nudge);
                    if let Some(memory) = &memory {
                        memory.correction(&mut next, &mut check);
                    }
                    match send_correction(&client, &next, &model_config, check.num_ctx, window_setting)
                        .await
                    {
                        Ok(response) => {
                            fallback_usage = held_usage;
                            next_response = Some(response);
                            continue;
                        }
                        Err(error) => {
                            tracing::warn!(
                                model = model.as_str(),
                                error_type = error.telemetry_type(),
                                "the corrective retry after a call written as text could not be \
                                 sent; the turn ends with the reply already shown"
                            );
                        }
                    }
                }
                if let Some(usage) = held_usage.or_else(|| fallback_usage.take()) {
                    yield usage;
                }
                break;
            }
            if let Some(usage) = fallback_usage.take() {
                // The best-effort retry came back empty: the reply before it
                // stands.
                tracing::warn!(
                    model = model.as_str(),
                    "the corrective retry after a call written as text came back empty; the turn \
                     ends with the reply already shown"
                );
                yield usage;
                break;
            }
            // R20: one retry for an empty reply whose tokens may be a call
            // Ollama swallowed. A reply cut at its limit fails at once:
            // asking again would most likely repeat it, at the cost of another
            // generation on a local model.
            if !retried && !tools.is_empty() && guard.empty_reply_may_be_a_lost_call() {
                retried = true;
                tracing::info!(
                    model = model.as_str(),
                    output_tokens = ?guard.output_tokens,
                    empty_reply = ?guard.empty_reply(),
                    "Ollama returned an empty reply to a request with tools, with tokens that \
                     may be a tool call it could not parse; asking once more"
                );
                let mut next = with_correction(&payload, "", &empty_reply_nudge(&tools));
                if let Some(memory) = &memory {
                    memory.correction(&mut next, &mut check);
                }
                next_response = Some(
                    send_correction(&client, &next, &model_config, check.num_ctx, window_setting)
                        .await?,
                );
                continue;
            }
            Err(guard.empty_reply_error(&model, retried, !tools.is_empty()))?;
        }
    })
}

/// Put `prefix` in front of the first non-empty text of `item`'s message.
/// Returns whether there was one.
fn prefix_first_text(item: &mut ProviderStreamItem, prefix: &str) -> bool {
    let Some(message) = item.0.as_mut() else {
        return false;
    };
    for content in &mut message.content {
        if let MessageContent::Text(text) = content {
            if !text.text.is_empty() {
                text.text.insert_str(0, prefix);
                return true;
            }
        }
    }
    false
}

#[derive(serde::Serialize)]
pub struct OllamaProvider {
    /// Shared with a turn's stream, which may send the turn's one corrective
    /// retry after the provider call that opened it has returned.
    #[serde(skip)]
    api_client: Arc<ApiClient>,
    model: ModelConfig,
    supports_streaming: bool,
    name: String,
    /// The base URL this instance resolved at construction — the same string
    /// the API client was handed. `tier()` reads it, never the provider's name:
    /// `from_custom_config` below builds an Ollama-engine provider from a
    /// user-writable JSON file whose `base_url` can point anywhere, and whose
    /// `name` can shadow a built-in.
    #[serde(skip)]
    resolved_base_url: String,
    #[serde(skip)]
    settings: OllamaSettings,
    /// The setting an overflow error tells the user to raise: the one this
    /// instance really reads ([`BUILTIN_WINDOW_SETTING`] or
    /// [`CUSTOM_WINDOW_SETTING`]). A custom provider's errors used to name
    /// `OLLAMA_NUM_CTX`, which it ignores, so following the advice changed
    /// nothing.
    #[serde(skip)]
    window_setting: &'static str,
    /// The `num_ctx` chosen per model, once. Only filled from a resolution that
    /// had `/api/show`'s answer, so a server that was briefly unreachable is
    /// asked again rather than pinned to a window chosen without the model's
    /// trained limit.
    #[serde(skip)]
    windows: Mutex<HashMap<String, usize>>,
    /// The window chosen for `self.model` (0 until known). Reported by
    /// [`Provider::get_model_config`] in place of the name-guessed limit — the
    /// same pattern as the Llama Server provider's `/props` window.
    #[serde(skip)]
    live_context_limit: AtomicUsize,
    /// Set once `/api/chat` answered 404: this server is OpenAI-compatible
    /// only, and every later request goes to `/v1/chat/completions`.
    #[serde(skip)]
    openai_compat: AtomicBool,
    /// What [`Provider::tier`] last answered was Private. A request for a model
    /// that turns out to be served remotely is refused while this is set: the
    /// privacy gates were told this chat's model runs here.
    #[serde(skip)]
    reported_private: AtomicBool,
}

impl OllamaProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let host: String = config
            .get_param("OLLAMA_HOST")
            .unwrap_or_else(|_| OLLAMA_HOST.to_string());

        // OLLAMA_TIMEOUT bounds SILENCE, not the whole reply: a local server
        // sends nothing until it has processed the prompt (36 s measured for
        // ~30k tokens on an M4 Max, minutes on a CPU), and a healthy generation
        // on a slow machine can stream for longer than any fixed deadline.
        let idle: Duration =
            Duration::from_secs(config.get_param("OLLAMA_TIMEOUT").unwrap_or(OLLAMA_TIMEOUT));

        let base_url = resolve_ollama_base_url(&host)?;
        Self::build(
            model,
            base_url,
            idle,
            true,
            Self::metadata().name,
            read_settings(config),
            BUILTIN_WINDOW_SETTING,
        )
    }

    /// An Ollama-engine provider declared in a `custom_providers/*.json` file.
    /// It takes the defaults for window, `keep_alive` and `think` rather than the
    /// built-in provider's `OLLAMA_*` settings, which describe a different
    /// server; a context limit set on its model config still applies.
    pub fn from_custom_config(
        model: ModelConfig,
        config: DeclarativeProviderConfig,
    ) -> Result<Self> {
        let idle = Duration::from_secs(config.timeout_seconds.unwrap_or(OLLAMA_TIMEOUT));
        let base_url = resolve_custom_base_url(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;
        Self::build(
            model,
            base_url,
            idle,
            config.supports_streaming.unwrap_or(true),
            config.name.clone(),
            OllamaSettings::defaults(),
            CUSTOM_WINDOW_SETTING,
        )
    }

    fn build(
        model: ModelConfig,
        base_url: Url,
        idle: Duration,
        supports_streaming: bool,
        name: String,
        settings: OllamaSettings,
        window_setting: &'static str,
    ) -> Result<Self> {
        let auth = AuthMethod::Custom(Box::new(NoAuth));
        let resolved_base_url = base_url.to_string();
        let api_client = Arc::new(ApiClient::with_idle_timeout(
            resolved_base_url.clone(),
            auth,
            idle,
        )?);
        Ok(Self {
            api_client,
            model,
            supports_streaming,
            name,
            resolved_base_url,
            settings,
            window_setting,
            windows: Mutex::new(HashMap::new()),
            live_context_limit: AtomicUsize::new(0),
            openai_compat: AtomicBool::new(false),
            reported_private: AtomicBool::new(false),
        })
    }

    /// Whether a request for `model` leaves this machine even through a
    /// loopback `OLLAMA_HOST`: an Ollama cloud model (`gpt-oss:120b-cloud`,
    /// `glm-4.6:cloud`), which Ollama serves by forwarding to ollama.com, or a
    /// model this server's `/api/show` already said it serves from a remote
    /// host (a cloud model renamed with `ollama cp`).
    fn serves_remotely(&self, model: &str) -> bool {
        self.remote_origin(model).is_some()
    }

    /// Where a request for `model` really goes when it [serves
    /// remotely](Self::serves_remotely): the origin of `/api/show`'s
    /// `remote_host`, else ollama.com, where Ollama's cloud models run.
    fn remote_origin(&self, model: &str) -> Option<String> {
        let facts = MODEL_FACTS.lock().ok().and_then(|facts| {
            facts
                .get(&(self.resolved_base_url.clone(), model.to_string()))
                .cloned()
        });
        let remote =
            super::utils::is_ollama_cloud_model(model) || facts.as_ref().is_some_and(|f| f.remote);
        remote.then(|| {
            facts
                .and_then(|facts| facts.remote_origin)
                .unwrap_or_else(|| OLLAMA_CLOUD_ORIGIN.to_string())
        })
    }

    // A cloud alternate or newly discovered remote alias must not inherit
    // the main model's Private classification, whichever transport it uses.
    fn check_model_privacy(
        &self,
        model: &str,
        facts: Option<&ModelFacts>,
    ) -> Result<(), ProviderError> {
        if self.reported_private.load(Ordering::Relaxed)
            && (self.serves_remotely(model) || facts.is_some_and(|facts| facts.remote))
        {
            return Err(ProviderError::ExecutionError(format!(
                "{model} is served by Ollama from another host (an Ollama cloud model), not on \
                 this machine, so the request was not sent: this chat's model had been classified \
                 as running on this machine. Select the model again so the chat's privacy tier \
                 reflects where it runs."
            )));
        }
        Ok(())
    }

    /// The `num_ctx` BioRouter sends (and reports) for `model` on this
    /// instance, or `None` when the server could not be asked about the model
    /// (unreachable, not an Ollama server, or the model is not pulled).
    ///
    /// For the server and the desktop gauge: the window a chat on this model
    /// actually runs with, rather than a guess from the model's name.
    pub async fn context_window_for(&self, model: &str) -> Option<usize> {
        let mut config = self.model.clone();
        if config.model_name != model {
            config = ModelConfig::new(model).ok()?;
        }
        let (num_ctx, facts) = self.window_for(&config).await;
        facts.map(|_| num_ctx)
    }

    /// `/api/show` for `model`, from the process-wide cache when it is there.
    async fn model_facts(&self, model: &str) -> Option<ModelFacts> {
        if self.openai_compat.load(Ordering::Relaxed) {
            return None;
        }
        let key = (self.resolved_base_url.clone(), model.to_string());
        if let Some(facts) = MODEL_FACTS.lock().ok()?.get(&key) {
            return Some(facts.clone());
        }
        let probe = async {
            let response = self
                .api_client
                .response_post("api/show", &json!({ "model": model }))
                .await
                .ok()?;
            if !response.status().is_success() {
                tracing::debug!(
                    status = response.status().as_u16(),
                    "Ollama /api/show did not describe the model"
                );
                return None;
            }
            response.json::<Value>().await.ok()
        };
        let show = tokio::time::timeout(PROBE_TIMEOUT, probe).await.ok()??;
        let facts = ModelFacts::from_show(&show);
        if let Ok(mut cache) = MODEL_FACTS.lock() {
            cache.insert(key, facts.clone());
        }
        Some(facts)
    }

    /// The window for `model_config`'s model on this instance, plus the facts it
    /// was chosen from. See [`choose_num_ctx`] for the order of precedence.
    async fn window_for(&self, model_config: &ModelConfig) -> (usize, Option<ModelFacts>) {
        let name = model_config.model_name.as_str();
        let facts = self.model_facts(name).await;
        let user_limit = user_context_limit(model_config);
        let automatic = self.settings.num_ctx.is_none() && user_limit.is_none();
        let known = if automatic {
            self.windows.lock().ok().and_then(|w| w.get(name).copied())
        } else {
            None
        };
        let num_ctx = match known {
            Some(num_ctx) => num_ctx,
            None => {
                let num_ctx = choose_num_ctx(
                    self.settings.num_ctx,
                    user_limit,
                    facts.as_ref().and_then(|f| f.trained_context),
                    memory_gib().await,
                );
                if automatic && facts.is_some() {
                    if let Ok(mut windows) = self.windows.lock() {
                        // A racing first request may have stored one already;
                        // keep it, so two requests never send two windows.
                        let stored = *windows.entry(name.to_string()).or_insert(num_ctx);
                        if stored != num_ctx {
                            return (self.memory_adjusted_window(model_config, stored), facts);
                        }
                    }
                }
                num_ctx
            }
        };
        let num_ctx = self.memory_adjusted_window(model_config, num_ctx);
        if name == self.model.model_name {
            self.live_context_limit.store(num_ctx, Ordering::Relaxed);
        }
        (num_ctx, facts)
    }

    fn automatic_window(&self, model_config: &ModelConfig) -> bool {
        self.settings.num_ctx.is_none() && user_context_limit(model_config).is_none()
    }

    fn loaded_model_memory(&self, model: &str) -> Arc<LoadedModelMemory> {
        LOADED_MODEL_MEMORY
            .lock()
            .expect("Ollama memory cache lock poisoned")
            .entry((
                self.resolved_base_url.clone(),
                normalized_ollama_model(model).into_owned(),
            ))
            .or_default()
            .clone()
    }

    fn memory_adjusted_window(&self, model_config: &ModelConfig, window: usize) -> usize {
        if self.automatic_window(model_config) {
            self.loaded_model_memory(&model_config.model_name)
                .cap(window)
        } else {
            window
        }
    }

    fn memory_observer(&self, model_config: &ModelConfig) -> Option<LoadedModelObserver> {
        if !self.automatic_window(model_config) || self.serves_remotely(&model_config.model_name) {
            return None;
        }
        let system_memory_bytes = if crate::providers::self_hosted_tier(&self.resolved_base_url)
            == ProviderTier::Private
        {
            super::llamacpp_sidecar::total_memory_gib().saturating_mul(1024 * 1024 * 1024)
        } else {
            0
        };
        Some(LoadedModelObserver {
            memory: self.loaded_model_memory(&model_config.model_name),
            system_memory_bytes,
        })
    }

    /// `/api/version`, once per server per process; logs a warning when the
    /// server is too old to honour `truncate: false`.
    async fn server_version(&self) -> Option<String> {
        if let Some(known) = SERVER_VERSIONS
            .lock()
            .ok()
            .and_then(|v| v.get(&self.resolved_base_url).cloned())
        {
            return Some(known);
        }
        let probe = async {
            let response = self.api_client.response_get("api/version").await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let body: Value = response.json().await.ok()?;
            body.get("version")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let version = tokio::time::timeout(PROBE_TIMEOUT, probe)
            .await
            .ok()
            .flatten();
        if let Some(version) = &version {
            if let Ok(mut versions) = SERVER_VERSIONS.lock() {
                versions.insert(self.resolved_base_url.clone(), version.clone());
            }
            match server_generation(version) {
                ServerGeneration::Current => {}
                ServerGeneration::Cuts => tracing::warn!(
                    version = version.as_str(),
                    "This Ollama is older than 0.12.6 and ignores `truncate: false`: an over-long \
                     prompt is cut to the server's window instead of refused. BioRouter checks \
                     every reply's prompt size and fails a truncated turn; update Ollama."
                ),
                ServerGeneration::CutsAndCountsOnlyNewTokens => tracing::warn!(
                    version = version.as_str(),
                    "This Ollama is older than 0.4.0: it cuts an over-long prompt instead of \
                     refusing it, and reports only the part of each prompt it did not have \
                     cached, so BioRouter cannot tell a cut prompt from a cached one and cannot \
                     detect the cut. Update Ollama."
                ),
            }
        }
        version
    }

    /// What to send as `think` for this request, or `None` for nothing.
    ///
    /// - Never to a model without the `thinking` capability (an HTTP 400), nor
    ///   when the capabilities are unknown.
    /// - A title: off.
    /// - `Quick`: off. `Deep`: the deepest level the model declares (`xhigh`
    ///   for qwen3.8 — never `high`, which its template raises on — and `true`
    ///   for a model that takes only booleans, such as gemma4).
    /// - A pinned output budget under [`SMALL_OUTPUT_BUDGET`] (unless `Deep`):
    ///   off, or the reasoning eats the budget and the reply has no answer.
    /// - `Normal` (or no effort): `OLLAMA_THINK` when set, otherwise nothing,
    ///   which is the model's own default.
    ///
    /// `false` is sent to gemma4 after a tool result too. Ollama issue #18861
    /// reports gemma4 replying empty there with thinking off; on Ollama 0.40.2
    /// it did not reproduce — 6/6 follow-ups to a tool result answered with
    /// text, on both the MLX `gemma4:31b` and the GGUF `gemma4:31b-it-q4_K_M`.
    /// An empty reply would still fail loudly through [`guard_turn`].
    fn think_for(
        &self,
        facts: Option<&ModelFacts>,
        model_config: &ModelConfig,
        purpose: Purpose,
    ) -> Option<Think> {
        let facts = facts?;
        let requested = match purpose {
            Purpose::Title => Think::Off,
            Purpose::Turn => match model_config.reasoning_effort {
                Some(ReasoningEffort::Quick) => Think::Off,
                Some(ReasoningEffort::Deep) => facts.deepest_think(),
                _ if model_config
                    .max_tokens
                    .is_some_and(|budget| budget < SMALL_OUTPUT_BUDGET) =>
                {
                    Think::Off
                }
                _ => self.settings.think.clone()?,
            },
        };
        facts.accept(&requested)
    }

    /// `num_predict` for this request: what the model config pins, else a
    /// ceiling that stops a runaway generation (see
    /// [`OLLAMA_DEFAULT_NUM_PREDICT`]). A title always gets its small budget.
    fn num_predict_for(
        model_config: &ModelConfig,
        purpose: Purpose,
        think: Option<&Think>,
        facts: Option<&ModelFacts>,
    ) -> i32 {
        match purpose {
            Purpose::Title => model_config
                .max_tokens
                .map_or(TITLE_NUM_PREDICT, |pinned| pinned.min(TITLE_NUM_PREDICT)),
            Purpose::Turn => model_config.max_tokens.unwrap_or_else(|| {
                // With no `think` sent, a thinking model thinks by default
                // (qwen3.8 at medium, gemma4 on), and so may a model whose
                // capabilities could not be read.
                // And some think whatever `think` says (R6): deepseek-r1
                // ignores `false`, and gpt-oss has no off.
                let may_think = match think {
                    Some(think) => {
                        *think != Think::Off || facts.is_some_and(ModelFacts::thinks_regardless)
                    }
                    None => facts.is_none_or(ModelFacts::supports_thinking),
                };
                if may_think {
                    OLLAMA_THINKING_NUM_PREDICT
                } else {
                    OLLAMA_DEFAULT_NUM_PREDICT
                }
            }),
        }
    }

    /// Tools for this request: none in Chat mode.
    fn effective_tools(tools: &[Tool]) -> &[Tool] {
        let mode = crate::config::Config::global()
            .get_biorouter_mode()
            .unwrap_or(BioRouterMode::Auto);
        if mode == BioRouterMode::Chat {
            &[]
        } else {
            tools
        }
    }

    /// The `/api/chat` body for one request, and the check its reply must pass.
    /// Every request streams, the blocking ones included (see [`collect_reply`]).
    async fn native_request(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        purpose: Purpose,
    ) -> Result<(Value, TurnCheck), ProviderError> {
        let (num_ctx, facts) = self.window_for(model_config).await;
        let model = model_config.model_name.as_str();
        self.check_model_privacy(model, facts.as_ref())?;
        let mut system = Cow::Borrowed(system);
        if !tools.is_empty() {
            if let Some(facts) = &facts {
                // R19: a model without tool support answers every request
                // with tools with an HTTP 400. Said plainly, before the model
                // is loaded, rather than as "Ollama rejected the request". A
                // model whose template never shows the tools is refused the
                // same way: it answers with 200s, about tools it never saw.
                if let Some(error) = facts.no_tools_error(model) {
                    return Err(error);
                }
                if facts.tools_only_near_last_user {
                    // R8: the template drops the tool list after the first
                    // tool result, so the roster also rides in the system
                    // prompt, which every turn renders. On EVERY request for
                    // the model, not only after a tool result, so the prompt
                    // prefix stays the same and Ollama's cache keeps it.
                    warn_tools_only_near_last_user(model);
                    system = Cow::Owned(format!("{system}\n\n{}", tool_roster(tools)));
                }
            }
        }
        let system = system.as_ref();
        let think = self.think_for(facts.as_ref(), model_config, purpose);
        let num_predict =
            Self::num_predict_for(model_config, purpose, think.as_ref(), facts.as_ref());
        let request = ChatRequest {
            model: &model_config.model_name,
            system,
            messages,
            tools,
            stream: true,
            num_ctx,
            temperature: model_config.temperature,
            num_predict: Some(num_predict),
            think,
            keep_alive: self.settings.keep_alive.clone(),
            vision: facts.as_ref().is_some_and(ModelFacts::supports_vision),
            calls_without_text: facts
                .as_ref()
                .is_some_and(|facts| facts.drops_calls_after_text),
        };
        let payload = native::create_request(&request).map_err(|e| {
            ProviderError::RequestFailed(format!("Could not build the request: {e}"))
        })?;
        let check = TurnCheck::native(
            native::text_bytes(&payload),
            num_ctx,
            &model_config.model_name,
            self.server_version().await,
            self.window_setting,
        );
        Ok((payload, check))
    }

    /// Retry policy for every request to this server. A timeout is handed
    /// straight back rather than re-sent: Ollama serves one request per loaded
    /// model at a time, so a request that timed out is one the server is still
    /// working on, and a re-send queues a second copy of a ~40k-token prompt
    /// behind it. Connection failures (a server still starting) are retried.
    async fn with_ollama_retry<F, Fut, T>(&self, operation: F) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, ProviderError>> + Send,
        T: Send,
    {
        self.with_retry_policy(TimeoutRetry::GiveUp, operation)
            .await
    }

    fn switch_to_openai_compat(&self) {
        if !self.openai_compat.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "{} has no /api/chat (HTTP 404); using its OpenAI-compatible \
                 /v1/chat/completions instead. That endpoint cannot set the context \
                 window, so every reply's prompt size is checked for truncation.",
                self.name
            );
        }
    }

    /// One native request as a guarded stream, or `None` when the server has no
    /// `/api/chat` (it has then been switched to `/v1` for good, and the caller
    /// takes that path instead).
    async fn native_stream(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        purpose: Purpose,
    ) -> Result<Option<MessageStream>, ProviderError> {
        let (payload, check) = self
            .native_request(model_config, system, messages, tools, purpose)
            .await?;
        let mut log = RequestLog::start(model_config, &payload)?;
        let outcome = self
            .with_ollama_retry(|| async {
                post_native(
                    &self.api_client,
                    &payload,
                    &model_config.model_name,
                    check.num_ctx,
                    self.window_setting,
                )
                .await
            })
            .await
            .inspect_err(|e| {
                let _ = log.provider_error(e);
            })?;
        let mut turn = NativeTurn {
            client: self.api_client.clone(),
            payload,
            model_config: model_config.clone(),
            check,
            window_setting: self.window_setting,
            message_id: format!("ollama_{}", uuid::Uuid::new_v4().simple()),
            tools: tools.to_vec(),
            retried: false,
            memory: self.memory_observer(model_config),
        };
        match outcome {
            NativeOutcome::Ok(response) => Ok(Some(native_turn(turn, response, log))),
            NativeOutcome::RouteMissing => {
                drop(log);
                self.switch_to_openai_compat();
                Ok(None)
            }
            NativeOutcome::UnknownTool(name) => {
                // R18, before any text: the turn's one corrective retry, now.
                let called = UnknownToolCalled {
                    name,
                    after_text: false,
                };
                if turn.tools.is_empty() {
                    let error = ProviderError::RequestFailed(called.to_string());
                    let _ = log.provider_error(&error);
                    return Err(error);
                }
                drop(log);
                tracing::info!(
                    model = model_config.model_name.as_str(),
                    "the model called a tool this request did not offer; asking it once more, \
                     with the tool list"
                );
                let mut next = with_correction(
                    &turn.payload,
                    "",
                    &unknown_tool_nudge(&called.name, &turn.tools),
                );
                if let Some(memory) = &turn.memory {
                    memory
                        .memory
                        .assess(
                            &self.api_client,
                            &model_config.model_name,
                            turn.check
                                .num_ctx
                                .expect("a native request has a context window"),
                            memory.system_memory_bytes,
                        )
                        .await;
                    memory.correction(&mut next, &mut turn.check);
                }
                let (response, log) = send_correction(
                    &self.api_client,
                    &next,
                    model_config,
                    turn.check.num_ctx,
                    self.window_setting,
                )
                .await?;
                turn.retried = true;
                Ok(Some(native_turn(turn, response, log)))
            }
        }
    }

    /// One blocking request, native or (after a 404) `/v1`. The native one
    /// streams and is collected, so `OLLAMA_TIMEOUT` bounds its silence rather
    /// than its whole generation, and it passes the same [`TurnCheck`] and
    /// answered-or-error guard as a turn.
    async fn complete_for(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        purpose: Purpose,
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.check_model_privacy(&model_config.model_name, None)?;
        let tools = Self::effective_tools(tools);
        if !self.openai_compat.load(Ordering::Relaxed) {
            if let Some(stream) = self
                .native_stream(model_config, system, messages, tools, purpose)
                .await?
            {
                return collect_reply(stream, &model_config.model_name).await;
            }
        }
        self.complete_openai_compat(model_config, system, messages, tools)
            .await
    }

    /// The `/v1/chat/completions` path, for a server with no `/api/chat`.
    async fn complete_openai_compat(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &super::utils::ImageFormat::OpenAi,
            false,
        )?;
        let check =
            TurnCheck::openai_compat(native::text_bytes(&payload), &model_config.model_name);
        let mut log = RequestLog::start(model_config, &payload)?;
        let response = self
            .with_ollama_retry(|| async {
                let response = self
                    .api_client
                    .response_post("v1/chat/completions", &payload)
                    .await?;
                handle_response_openai_compat(response).await
            })
            .await
            .map_err(|e| v1_capability_error(e, &model_config.model_name))
            .inspect_err(|e| {
                let _ = log.provider_error(e);
            })?;
        let message = recover_in_message(
            response_to_message(&response)?,
            tools,
            response
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str),
            "ollama",
            &model_config.model_name,
        );
        let usage = response.get("usage").map(get_usage).unwrap_or_default();
        log.write(&response, Some(&usage))?;
        if let Some(error) = check.truncation(&usage) {
            return Err(error);
        }
        if !is_answer(&message) {
            let finish_reason = response
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str);
            return Err(no_answer_error(finish_reason));
        }
        Ok((message, ProviderUsage::new(get_model(&response), usage)))
    }

    /// The `/v1/chat/completions` stream, for a server with no `/api/chat`.
    async fn stream_openai_compat_turn(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let payload = create_request(
            &self.model,
            system,
            messages,
            tools,
            &super::utils::ImageFormat::OpenAi,
            true,
        )?;
        let check = TurnCheck::openai_compat(native::text_bytes(&payload), &self.model.model_name);
        let mut log = RequestLog::start(&self.model, &payload)?;
        let response = self
            .with_ollama_retry(|| async {
                let response = self
                    .api_client
                    .response_post("v1/chat/completions", &payload)
                    .await?;
                handle_status_openai_compat(response).await
            })
            .await
            .map_err(|e| v1_capability_error(e, &self.model.model_name))
            .inspect_err(|e| {
                let _ = log.provider_error(e);
            })?;
        let decoded = recover_leaked_tool_calls(
            stream_openai_compat_checked(response, log),
            RecoveryContext {
                tools: tools.to_vec(),
                provider: "ollama",
                model: self.model.model_name.clone(),
            },
        );
        Ok(guard_turn(
            decoded,
            check,
            self.model.model_name.clone(),
            !tools.is_empty(),
        ))
    }
}

/// R19 on the `/v1` fallback: OpenAI's error shape carries the same sentence
/// as the native 400 (`{"error":{"message":"... does not support tools",
/// "type":"invalid_request_error"}}` from gemma3:4b), and is answered the same
/// way, with what to do instead.
fn v1_capability_error(error: ProviderError, model: &str) -> ProviderError {
    if error
        .to_string()
        .to_ascii_lowercase()
        .contains("does not support tools")
    {
        native::no_tools_error(model, None)
    } else {
        error
    }
}

/// The `/v1` fallback's stream, failing a body that ends with neither a
/// `finish_reason` nor `[DONE]`: that is a transport failure, not an answer.
/// Measured on Ollama 0.40.2: when ministral-3:8b called an undeclared tool
/// after streaming text, `/v1` silently dropped the error and just ended the
/// body, which the generic decoder handed on as a complete, successful reply.
fn stream_openai_compat_checked(response: Response, mut log: RequestLog) -> MessageStream {
    let bytes = response.bytes_stream().map_err(io::Error::other);
    Box::pin(try_stream! {
        let ended = Arc::new(AtomicBool::new(false));
        let seen = ended.clone();
        let lines = FramedRead::new(StreamReader::new(bytes), LinesCodec::new())
            .map_err(anyhow::Error::from)
            .inspect_ok(move |line| {
                let line = line.trim();
                if line == "data: [DONE]"
                    || line.strip_prefix("data:").is_some_and(|data| {
                        serde_json::from_str::<Value>(data.trim()).is_ok_and(|value| {
                            value.pointer("/choices/0/finish_reason").and_then(Value::as_str).is_some()
                        })
                    })
                {
                    seen.store(true, Ordering::Relaxed);
                }
            });
        let decoded = super::formats::openai::response_to_streaming_message(lines);
        tokio::pin!(decoded);
        while let Some(item) = decoded.next().await {
            let item = item.map_err(|error| {
                let error = provider_error(error);
                let _ = log.provider_error(&error);
                error
            })?;
            log.write(&item.0, item.1.as_ref().map(|usage| &usage.usage))?;
            yield item;
        }
        if !ended.load(Ordering::Relaxed) {
            let error = ProviderError::RequestFailed(
                "The server's response ended before the turn was complete (no finish reason and \
                 no end-of-stream marker); no tool call from it was run. Retry the message."
                    .to_string(),
            );
            let _ = log.provider_error(&error);
            Err(error)?;
        }
    })
}

/// R8: log once per model that its template hides the tool list after the
/// first tool result. Measured on Ollama 0.40.2 with llama3.1:8b and
/// mistral-small3.2:24b; see [`ModelFacts::tools_only_near_last_user`].
fn warn_tools_only_near_last_user(model: &str) {
    static WARNED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
    let first = WARNED
        .lock()
        .map(|mut warned| warned.insert(model.to_string()))
        .unwrap_or(false);
    if first {
        tracing::warn!(
            model,
            "{model}'s Ollama chat template shows the tool definitions only next to the latest \
             user message, so after the first tool result the model no longer sees them. \
             Biorouter also lists the tools in the system prompt for it, but multi-step tasks \
             may still stop early or be answered without the tools. Biorouter's supported \
             local models are qwen3.8:27b and gemma4:31b"
        );
    }
}

/// The most tools [`tool_roster`] lists, and the longest description line.
/// The roster costs prompt on every turn; BioRouter offers a few dozen tools,
/// so the caps only bound a pathological roster.
const MAX_ROSTER_TOOLS: usize = 64;
const MAX_ROSTER_DESCRIPTION_CHARS: usize = 120;

/// R8's mitigation: a compact list of the request's tools, one line each —
/// name, parameters (`?` marks an optional one), and the first sentence of
/// the description — for the system prompt of a model whose template shows
/// the tool definitions only next to the latest user message.
///
/// Measured on Ollama 0.40.2: once a tool result was the last message,
/// llama3.1:8b's rendered prompt held no tool definition at all (135-153
/// prompt tokens against 518 with the tools), and the model answered from
/// memory with a fabricated result in 3 of 3 multi-step runs; mistral-small3.2
/// invented tool names its parser then swallowed. The model still writes a
/// call in its own format (llama3.1's history shows it one), so names and
/// parameters are what it lacks.
fn tool_roster(tools: &[Tool]) -> String {
    let mut lines = vec![
        "# Tools available at every step".to_string(),
        String::new(),
        "These are the tools of this conversation. They stay available after every tool \
         result: when the next step needs one, call it as a tool call with JSON arguments; \
         never write the call as text."
            .to_string(),
    ];
    for tool in tools.iter().take(MAX_ROSTER_TOOLS) {
        let schema = tool.input_schema.as_ref();
        let required: HashSet<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // Required parameters first, in the schema's `required` order, then
        // the optional ones: the same line whatever order the schema's map
        // keeps its keys in.
        let mut params: Vec<String> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            params.extend(
                properties
                    .keys()
                    .filter(|name| !required.contains(name.as_str()))
                    .map(|name| format!("{name}?")),
            );
        }
        let description = tool.description.as_deref().unwrap_or_default();
        let first_sentence = description
            .split_terminator(['\n', '.'])
            .map(str::trim)
            .find(|sentence| !sentence.is_empty())
            .unwrap_or_default();
        let summary = safe_truncate(first_sentence, MAX_ROSTER_DESCRIPTION_CHARS);
        let line = if summary.is_empty() {
            format!("- {}({})", tool.name, params.join(", "))
        } else {
            format!("- {}({}): {summary}", tool.name, params.join(", "))
        };
        lines.push(line);
    }
    if tools.len() > MAX_ROSTER_TOOLS {
        lines.push(format!(
            "- ... and {} more, defined with the first message",
            tools.len() - MAX_ROSTER_TOOLS
        ));
    }
    lines.join("\n")
}

struct NoAuth;

#[async_trait]
impl super::api_client::AuthProvider for NoAuth {
    async fn get_auth_header(&self) -> Result<(String, String)> {
        Ok(("X-No-Auth".to_string(), "true".to_string()))
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "ollama",
            "Ollama",
            "Local open source models",
            OLLAMA_DEFAULT_MODEL,
            OLLAMA_KNOWN_MODELS.to_vec(),
            OLLAMA_DOC_URL,
            vec![
                ConfigKey::new("OLLAMA_HOST", true, false, Some(OLLAMA_HOST)),
                ConfigKey::new(
                    "OLLAMA_TIMEOUT",
                    false,
                    false,
                    Some(&(OLLAMA_TIMEOUT.to_string())),
                ),
                // Unset means automatic: the model's trained window, capped by
                // a memory tier (64k, or 128k with 64 GiB or more).
                ConfigKey::new("OLLAMA_NUM_CTX", false, false, None),
            ],
        )
        .with_unlisted_models()
        // Ollama ships pointed at this machine, so a default install is
        // Private. An instance pointed off it says so itself, below.
        .with_tier(ProviderTier::Private)
        .with_local_compute()
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    /// Private exactly while the resolved base URL is loopback AND the model
    /// runs on that server. An Ollama cloud model (`gpt-oss:120b-cloud`) is
    /// Public even through a loopback `OLLAMA_HOST`: Ollama serves it by
    /// forwarding the conversation to ollama.com. The model is this instance's
    /// own, so a switch to a cloud model — a new provider instance — is
    /// re-tiered by the bind gate, and every turn's gate reads it again.
    ///
    /// ⚠ This makes the model NAME a tier input for this provider, in BOTH
    /// directions, exactly as `OLLAMA_HOST` is one: a local tag to a cloud
    /// tag lowers the tier to Public, and a cloud tag back to a local one on
    /// a loopback host raises it to Private. That is why `BIOROUTER_MODEL` is
    /// a capability key (`privacy::config_keys`). What the name can never do
    /// is make a REMOTE host Private.
    fn tier(&self) -> ProviderTier {
        let tier = if self.serves_remotely(&self.model.model_name) {
            ProviderTier::Public
        } else {
            crate::providers::self_hosted_tier(&self.resolved_base_url)
        };
        self.reported_private
            .store(tier == ProviderTier::Private, Ordering::Relaxed);
        tier
    }

    /// DR-26: `Local` exactly while the resolved base URL is loopback and the
    /// model runs on it. A remote `OLLAMA_HOST`, or a cloud model, is someone
    /// else's server, so it gets no affiliation at all rather than inheriting
    /// `Local`'s blanket permission over every private extension.
    fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
        if self.serves_remotely(&self.model.model_name) {
            return None;
        }
        crate::providers::self_hosted_affiliation(&self.resolved_base_url)
    }

    /// The origin Biorouter Copilot's consent card names as where screenshots
    /// go. For a cloud model that is ollama.com, not the loopback server that
    /// relays to it: the card used to say `http://localhost:11434` — "this
    /// machine" — while the tier, rightly, said Public.
    fn computer_use_destination(&self) -> Option<String> {
        let server = self.api_client.computer_use_destination(OLLAMA_CHAT_PATH);
        match self.remote_origin(&self.model.model_name) {
            Some(remote) => Some(match server {
                Some(server) => format!("{remote} (Ollama cloud model, relayed by {server})"),
                None => format!("{remote} (Ollama cloud model)"),
            }),
            None => server,
        }
    }

    /// The consent binding: the server endpoint, plus where the model runs
    /// when that is elsewhere, so a switch between a local and a cloud model
    /// on the same server is a new scope.
    fn computer_use_destination_identity(&self) -> Option<String> {
        let server = self
            .api_client
            .computer_use_destination_identity(OLLAMA_CHAT_PATH);
        Some(match self.remote_origin(&self.model.model_name) {
            Some(remote) => super::base::computer_use_destination_digest(&format!(
                "{server}\0ollama-remote\0{remote}"
            )),
            None => server,
        })
    }

    /// The configured model, with the window this provider actually sends as
    /// `num_ctx` once it is known — not the guess from the model's name, which
    /// is what let compaction believe in 128k while Ollama served 4k.
    fn get_model_config(&self) -> ModelConfig {
        let live = self.live_context_limit.load(Ordering::Relaxed);
        let live = self.memory_adjusted_window(
            &self.model,
            if live > 0 {
                live
            } else {
                self.model.context_limit.unwrap_or(0)
            },
        );
        if live > 0 {
            self.model.clone().with_context_limit(Some(live))
        } else {
            self.model.clone()
        }
    }

    fn restore_binding(&self) -> crate::providers::provider_binding::ProviderRestoreBinding {
        crate::providers::provider_binding::ProviderRestoreBinding::registry(
            self.name.clone(),
            self.model.clone(),
        )
    }

    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete_for(&self.model, system, messages, tools, Purpose::Turn)
            .await
    }

    async fn complete_fast(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let fast = self.model.use_fast_model();
        match self
            .complete_for(&fast, system, messages, tools, Purpose::Turn)
            .await
        {
            Err(_) if fast.model_name != self.model.model_name => {
                self.complete_for(&self.model, system, messages, tools, Purpose::Turn)
                    .await
            }
            result => result,
        }
    }

    #[tracing::instrument(
        skip(self, model_config, system, messages, tools),
        fields(model_config, input, output, input_tokens, output_tokens, total_tokens)
    )]
    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete_for(model_config, system, messages, tools, Purpose::Turn)
            .await
    }

    /// A title with thinking off and a small output budget, at the same window
    /// as every other request for this model — a different `num_ctx` would
    /// reload the model between the turn and its title.
    async fn generate_session_name(
        &self,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        let context = self.get_initial_user_messages(messages);
        let message = Message::user().with_text(self.create_session_name_prompt(&context));
        let (reply, _) = self
            .complete_for(
                &self.model,
                "You are a title generator. Output only the requested title of 4 words or less, with no additional text, reasoning, or explanations.",
                &[message],
                &[],
                Purpose::Title,
            )
            .await?;
        // Thinking arrives in its own content (and a leading inline `<think>`
        // block is split off by the decoder), so the text is the title.
        let title = reply
            .as_concat_text()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Ok(safe_truncate(title.trim_matches(['"', '\'', '*']), 100))
    }

    fn supports_streaming(&self) -> bool {
        self.supports_streaming
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.check_model_privacy(&self.model.model_name, None)?;
        let tools = Self::effective_tools(tools);
        if !self.openai_compat.load(Ordering::Relaxed) {
            if let Some(stream) = self
                .native_stream(&self.model, system, messages, tools, Purpose::Turn)
                .await?
            {
                return Ok(stream);
            }
        }
        self.stream_openai_compat_turn(system, messages, tools)
            .await
    }

    /// The tags `/api/tags` lists, minus the ones `/api/show` says cannot call
    /// tools (embedding models, old chat-only models): BioRouter's agent sends
    /// tools on every turn, so those fail with an Ollama 400 on the first one.
    /// A model whose capabilities could not be read stays listed, and so does
    /// the configured model, so a selection never silently disappears.
    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let response = self
            .api_client
            .response_get("api/tags")
            .await
            .map_err(|e| ProviderError::RequestFailed(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError::RequestFailed(format!(
                "Failed to fetch models: HTTP {}",
                response.status()
            )));
        }

        let json_response = response.json::<Value>().await.map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to parse response: {}", e))
        })?;

        let models = json_response
            .get("models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| {
                ProviderError::RequestFailed("No models array in response".to_string())
            })?;

        let names: Vec<String> = models
            .iter()
            .filter_map(|model| model.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        // Bounded: a machine with dozens of tags must not open dozens of
        // connections, and each probe is capped by PROBE_TIMEOUT.
        let without_tools: HashSet<String> = futures::stream::iter(names.iter().cloned())
            .map(|name| async move {
                let lacks_tools = self
                    .model_facts(&name)
                    .await
                    .and_then(|facts| facts.supports_tools())
                    == Some(false);
                (name, lacks_tools)
            })
            .buffer_unordered(4)
            .filter_map(|(name, lacks_tools)| async move { lacks_tools.then_some(name) })
            .collect()
            .await;

        let mut model_names: Vec<String> = names
            .into_iter()
            .filter(|name| !without_tools.contains(name) || *name == self.model.model_name)
            .collect();
        model_names.sort();
        Ok(Some(model_names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::declarative_providers::ProviderEngine;
    use crate::conversation::message::Message;
    use crate::providers::base::ProviderStreamItem;
    use futures::TryStreamExt;
    use rmcp::model::CallToolResult;
    use serde_json::Value;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Every provider here points at a fresh mock server, and a fresh mock
    /// server can be handed the port of one an earlier test dropped. The
    /// version and model caches are per base URL and per process, so without
    /// this a test could read the previous server's answer:
    /// `a_pre_0_4_server_is_not_judged_by_its_prompt_count` failed in a full
    /// run judging its 0.3.14 server as 0.40.2, and passed alone.
    fn config(base_url: &str) -> DeclarativeProviderConfig {
        let base = base_url.trim_end_matches('/');
        if let Ok(mut versions) = SERVER_VERSIONS.lock() {
            versions.retain(|url, _| url.trim_end_matches('/') != base);
        }
        if let Ok(mut facts) = MODEL_FACTS.lock() {
            facts.retain(|(url, _), _| url.trim_end_matches('/') != base);
        }
        DeclarativeProviderConfig {
            name: "ollama-test".to_string(),
            engine: ProviderEngine::Ollama,
            display_name: "Ollama test server".to_string(),
            description: None,
            api_key_env: "NOT_USED".to_string(),
            base_url: base_url.to_string(),
            models: vec![],
            headers: None,
            timeout_seconds: Some(5),
            supports_streaming: Some(true),
        }
    }

    /// `/api/show` for a qwen3.8-shaped model trained on 40960 tokens, so the
    /// chosen window is 40960 whatever the test machine's memory.
    fn show_body(capabilities: &[&str], think_values: Value) -> Value {
        json!({
            "capabilities": capabilities,
            "model_info": {"general.architecture": "qwen35", "qwen35.context_length": 40960},
            "thinking": {"values": think_values},
        })
    }

    async fn mount_show(server: &MockServer, show: Value) {
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(show))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "0.40.2"})))
            .mount(server)
            .await;
    }

    async fn mount_chat(server: &MockServer, body: &str) {
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-ndjson")
                    .set_body_string(body),
            )
            .mount(server)
            .await;
    }

    async fn mount_v1_only_chat(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(404).set_body_string("404 page not found"))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "t", "model": "mock", "choices": [{
                    "index": 0, "message": {"role": "assistant", "content": "ready"},
                    "finish_reason": "stop",
                }], "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6},
            })))
            .mount(server)
            .await;
    }

    /// A model name no other test uses. `/api/show` answers are cached
    /// process-wide by (base URL, model), and wiremock hands out pooled servers,
    /// so two tests can see the same base URL; a shared name would let one
    /// test's capabilities leak into another's request.
    fn unique(name: &str) -> String {
        format!("{name}-{}", uuid::Uuid::new_v4().simple())
    }

    fn provider_at(server: &MockServer, model: &str) -> OllamaProvider {
        OllamaProvider::from_custom_config(ModelConfig::new_or_fail(model), config(&server.uri()))
            .expect("test Ollama provider should construct")
    }

    /// A provider whose `/api/show` says qwen3.8 (tools, thinking with levels),
    /// answering every `/api/chat` with `body`.
    async fn provider_for(body: &str) -> (MockServer, OllamaProvider) {
        let server = MockServer::start().await;
        mount_show(
            &server,
            show_body(
                &["completion", "tools", "thinking"],
                json!([false, "low", "medium", "xhigh"]),
            ),
        )
        .await;
        mount_chat(&server, body).await;
        let provider = provider_at(&server, &unique("qwen3.8:27b"));
        (server, provider)
    }

    async fn chat_requests(server: &MockServer) -> Vec<Value> {
        server
            .received_requests()
            .await
            .expect("wiremock should return received requests")
            .iter()
            .filter(|request| request.url.path() == "/api/chat")
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect()
    }

    async fn collect_stream(
        provider: &OllamaProvider,
    ) -> Result<Vec<ProviderStreamItem>, ProviderError> {
        provider
            .stream(
                "system",
                &[Message::user().with_text("test the mocked stream")],
                &[],
            )
            .await?
            .try_collect()
            .await
    }

    async fn stalled_then_successful_probe_server(
        probe_path: &'static str,
        body: Value,
    ) -> (String, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn read_request(socket: &mut tokio::net::TcpStream, probe_path: &str) {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0, "probe must send request headers");
                request.extend_from_slice(&buffer[..count]);
            }
            let request = std::str::from_utf8(&request).unwrap();
            assert!(
                request.lines().next().unwrap().contains(probe_path),
                "probe used the wrong route"
            );
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let task = tokio::spawn(async move {
            let body = body.to_string();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let (mut first, _) = listener.accept().await.unwrap();
            read_request(&mut first, probe_path).await;
            seen.fetch_add(1, Ordering::Relaxed);
            first.write_all(headers.as_bytes()).await.unwrap();
            first.flush().await.unwrap();
            // Keep the first body stalled until a later uncached probe arrives.
            let (mut second, _) = listener.accept().await.unwrap();
            read_request(&mut second, probe_path).await;
            seen.fetch_add(1, Ordering::Relaxed);
            second.write_all(headers.as_bytes()).await.unwrap();
            second.write_all(body.as_bytes()).await.unwrap();
            second.flush().await.unwrap();
            drop(first);
        });
        (base_url, task, requests)
    }

    #[tokio::test]
    async fn show_probe_bounds_a_stalled_body_and_retries_without_caching_the_failure() {
        let (base_url, server, requests) = stalled_then_successful_probe_server(
            "/api/show",
            show_body(&["completion", "tools"], json!([])),
        )
        .await;
        let mut settings = config(&base_url);
        settings.timeout_seconds = Some(60);
        let model = unique("bounded-show");
        let provider =
            OllamaProvider::from_custom_config(ModelConfig::new_or_fail(&model), settings).unwrap();
        let started = std::time::Instant::now();
        let failed = tokio::time::timeout(
            PROBE_TIMEOUT + Duration::from_secs(2),
            provider.model_facts(&model),
        )
        .await
        .expect("the whole show probe must respect PROBE_TIMEOUT, not the 60s read timeout");
        assert!(failed.is_none());
        assert!(started.elapsed() >= PROBE_TIMEOUT);
        assert_eq!(requests.load(Ordering::Relaxed), 1);

        let facts = provider.model_facts(&model).await.unwrap();
        assert_eq!(facts.trained_context, Some(40_960));
        assert_eq!(facts.supports_tools(), Some(true));
        server.await.unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        assert_eq!(
            provider.model_facts(&model).await.unwrap().trained_context,
            Some(40_960)
        );
        assert_eq!(requests.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn version_probe_bounds_a_stalled_body_and_retries_without_caching_the_failure() {
        let (base_url, server, requests) =
            stalled_then_successful_probe_server("/api/version", json!({"version": "0.40.2"}))
                .await;
        let mut settings = config(&base_url);
        settings.timeout_seconds = Some(60);
        let provider = OllamaProvider::from_custom_config(
            ModelConfig::new_or_fail(&unique("bounded-version")),
            settings,
        )
        .unwrap();
        let started = std::time::Instant::now();
        let failed = tokio::time::timeout(
            PROBE_TIMEOUT + Duration::from_secs(2),
            provider.server_version(),
        )
        .await
        .expect("the whole version probe must respect PROBE_TIMEOUT, not the 60s read timeout");
        assert!(failed.is_none());
        assert!(started.elapsed() >= PROBE_TIMEOUT);
        assert_eq!(requests.load(Ordering::Relaxed), 1);

        assert_eq!(provider.server_version().await.as_deref(), Some("0.40.2"));
        server.await.unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        assert_eq!(provider.server_version().await.as_deref(), Some("0.40.2"));
        assert_eq!(requests.load(Ordering::Relaxed), 2);
    }

    const DONE: &str = r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":30,"prompt_eval_cached_count":0,"eval_count":3}"#;

    async fn memory_test_provider(
        spilled: bool,
        starting_context: usize,
    ) -> (MockServer, OllamaProvider) {
        let server = MockServer::start().await;
        mount_show(
            &server,
            json!({"capabilities": ["completion", "tools"], "model_info": {
                "general.architecture": "llama", "llama.context_length": 262144
            }}),
        )
        .await;
        let model = unique("qwen3.8:27b");
        let current_context = Arc::new(AtomicUsize::new(0));
        let chat_context = current_context.clone();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(move |request: &wiremock::Request| {
                let payload: Value = serde_json::from_slice(&request.body).unwrap();
                chat_context.store(payload["options"]["num_ctx"].as_u64().unwrap() as usize, Ordering::Relaxed);
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-ndjson")
                    .set_body_string(format!(
                        "{}\n{DONE}\n",
                        r#"{"model":"m","message":{"role":"assistant","content":"Mocked Session Title"},"done":false}"#,
                    ))
            })
            .mount(&server)
            .await;
        let ps_model = model.clone();
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(move |_request: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(json!({"models": [{
                    "name": ps_model, "model": ps_model,
                    "size": 1024, "size_vram": if spilled { 512 } else { 1024 },
                    "context_length": current_context.load(Ordering::Relaxed) * 5,
                }]}))
            })
            .mount(&server)
            .await;
        let provider = provider_at(&server, &model);
        provider
            .windows
            .lock()
            .unwrap()
            .insert(model, starting_context);
        (server, provider)
    }

    async fn ps_requests(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/api/ps")
            .count()
    }

    #[tokio::test]
    async fn automatic_window_downgrade_is_shared_by_turns_titles_and_fresh_providers() {
        let (server, provider) = memory_test_provider(true, OLLAMA_LARGE_CONTEXT_SIZE).await;
        collect_stream(&provider).await.unwrap();
        assert_eq!(
            provider.get_model_config().context_limit,
            Some(OLLAMA_AGENT_CONTEXT_SIZE)
        );
        provider
            .complete("system", &[Message::user().with_text("hi")], &[])
            .await
            .unwrap();
        provider
            .generate_session_name(&Conversation::new_unvalidated(vec![
                Message::user().with_text("hi")
            ]))
            .await
            .unwrap();
        let rebuilt = provider_at(&server, &provider.model.model_name);
        assert_eq!(
            rebuilt.get_model_config().context_limit,
            Some(OLLAMA_AGENT_CONTEXT_SIZE)
        );
        rebuilt
            .windows
            .lock()
            .unwrap()
            .insert(provider.model.model_name.clone(), OLLAMA_LARGE_CONTEXT_SIZE);
        assert_eq!(
            rebuilt.context_window_for(&provider.model.model_name).await,
            Some(OLLAMA_AGENT_CONTEXT_SIZE)
        );
        collect_stream(&rebuilt).await.unwrap();
        let requests = chat_requests(&server).await;
        assert_eq!(
            requests
                .iter()
                .map(|payload| payload["options"]["num_ctx"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [131072, 65536, 65536, 65536]
        );
        assert_eq!(
            ps_requests(&server).await,
            2,
            "one check at 128k and one at the 64k floor"
        );
    }

    #[tokio::test]
    async fn explicit_windows_are_preserved_even_when_the_loaded_model_spills() {
        let (server, mut provider) = memory_test_provider(true, OLLAMA_LARGE_CONTEXT_SIZE).await;
        provider.settings.num_ctx = Some(OLLAMA_LARGE_CONTEXT_SIZE);
        for _ in 0..2 {
            collect_stream(&provider).await.unwrap();
        }
        assert_eq!(
            provider.get_model_config().context_limit,
            Some(OLLAMA_LARGE_CONTEXT_SIZE)
        );
        assert_eq!(ps_requests(&server).await, 0);

        let (server, mut provider) = memory_test_provider(true, 98304).await;
        provider.model = provider.model.clone().with_context_limit(Some(98304));
        collect_stream(&provider).await.unwrap();
        assert_eq!(provider.get_model_config().context_limit, Some(98304));
        assert_eq!(chat_requests(&server).await[0]["options"]["num_ctx"], 98304);
        assert_eq!(ps_requests(&server).await, 0);
    }

    #[tokio::test]
    async fn a_reported_automatic_window_stays_automatic_when_the_provider_is_rebuilt() {
        let (server, provider) = memory_test_provider(true, OLLAMA_LARGE_CONTEXT_SIZE).await;
        assert_eq!(
            provider
                .context_window_for(&provider.model.model_name)
                .await,
            Some(131072)
        );
        let derived = provider.get_model_config();
        assert_ne!(
            derived.context_limit,
            Some(ModelConfig::context_window_for(&derived.model_name))
        );
        let original = provider.restore_binding().model().clone();
        assert_eq!(original.context_limit, provider.model.context_limit);
        let rebuilt = OllamaProvider::from_custom_config(original, config(&server.uri())).unwrap();
        rebuilt
            .windows
            .lock()
            .unwrap()
            .insert(provider.model.model_name.clone(), 131072);
        assert!(rebuilt.automatic_window(&rebuilt.model));
        collect_stream(&rebuilt).await.unwrap();
        assert_eq!(rebuilt.get_model_config().context_limit, Some(65536));
        collect_stream(&rebuilt).await.unwrap();
        let requests = chat_requests(&server).await;
        assert_eq!(requests[0]["options"]["num_ctx"], 131072);
        assert_eq!(requests[1]["options"]["num_ctx"], 65536);
    }

    #[tokio::test]
    async fn an_explicit_model_window_after_an_automatic_downgrade_is_preserved() {
        let (server, provider) = memory_test_provider(true, OLLAMA_LARGE_CONTEXT_SIZE).await;
        collect_stream(&provider).await.unwrap();
        assert_eq!(provider.get_model_config().context_limit, Some(65536));
        let explicit = provider.model.clone().with_context_limit(Some(131072));
        let rebuilt = OllamaProvider::from_custom_config(explicit, config(&server.uri())).unwrap();
        assert!(!rebuilt.automatic_window(&rebuilt.model));
        collect_stream(&rebuilt).await.unwrap();
        assert_eq!(rebuilt.get_model_config().context_limit, Some(131072));
        assert_eq!(
            chat_requests(&server).await[1]["options"]["num_ctx"],
            131072
        );
        assert_eq!(ps_requests(&server).await, 1);
    }

    #[tokio::test]
    async fn per_call_context_overrides_do_not_use_or_replace_the_automatic_window() {
        let (server, provider) = memory_test_provider(false, OLLAMA_LARGE_CONTEXT_SIZE).await;
        assert!(provider.automatic_window(&provider.model));
        let messages = [Message::user().with_text("test context overrides")];

        provider.complete("system", &messages, &[]).await.unwrap();
        for context in [65_536, 98_304] {
            let explicit = provider.model.clone().with_context_limit(Some(context));
            assert_eq!(user_context_limit(&explicit), Some(context));
            provider
                .complete_with_model(&explicit, "system", &messages, &[])
                .await
                .unwrap();
            assert_eq!(
                provider
                    .windows
                    .lock()
                    .unwrap()
                    .get(&provider.model.model_name)
                    .copied(),
                Some(OLLAMA_LARGE_CONTEXT_SIZE)
            );
        }
        provider.complete("system", &messages, &[]).await.unwrap();

        let contexts: Vec<u64> = chat_requests(&server)
            .await
            .iter()
            .map(|request| request["options"]["num_ctx"].as_u64().unwrap())
            .collect();
        assert_eq!(contexts, vec![131_072, 65_536, 98_304, 131_072]);
    }

    #[tokio::test]
    async fn an_explicit_first_call_does_not_populate_the_automatic_window_cache() {
        let body = format!(
            "{}\n{DONE}\n",
            r#"{"model":"m","message":{"role":"assistant","content":"Mock response"},"done":false}"#,
        );
        let (server, provider) = provider_for(&body).await;
        assert!(provider.automatic_window(&provider.model));
        let messages = [Message::user().with_text("test the first explicit context")];
        let explicit = provider.model.clone().with_context_limit(Some(16_384));
        assert_eq!(user_context_limit(&explicit), Some(16_384));

        provider
            .complete_with_model(&explicit, "system", &messages, &[])
            .await
            .unwrap();
        assert!(provider.windows.lock().unwrap().is_empty());
        provider.complete("system", &messages, &[]).await.unwrap();

        let requests = chat_requests(&server).await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["options"]["num_ctx"], 16_384);
        assert_eq!(requests[1]["options"]["num_ctx"], 40_960);
        assert_eq!(
            provider
                .windows
                .lock()
                .unwrap()
                .get(&provider.model.model_name)
                .copied(),
            Some(40_960)
        );
    }

    #[tokio::test]
    async fn per_call_context_overrides_keep_setting_precedence_and_the_trained_limit() {
        let body = format!(
            "{}\n{DONE}\n",
            r#"{"model":"m","message":{"role":"assistant","content":"Mock response"},"done":false}"#,
        );
        let (server, mut provider) = provider_for(&body).await;
        assert!(provider.automatic_window(&provider.model));
        provider.settings.num_ctx = Some(65_536);
        let messages = [Message::user().with_text("test setting precedence")];

        for context in [32_768, 98_304] {
            let explicit = provider.model.clone().with_context_limit(Some(context));
            assert_eq!(user_context_limit(&explicit), Some(context));
            provider
                .complete_with_model(&explicit, "system", &messages, &[])
                .await
                .unwrap();
        }

        let requests = chat_requests(&server).await;
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|request| request["options"]["num_ctx"] == 40_960));
        assert!(provider.windows.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn healthy_load_and_spilled_floor_are_checked_once_without_reloads() {
        for (spilled, context) in [
            (false, OLLAMA_LARGE_CONTEXT_SIZE),
            (true, OLLAMA_AGENT_CONTEXT_SIZE),
        ] {
            let (server, provider) = memory_test_provider(spilled, context).await;
            for _ in 0..2 {
                collect_stream(&provider).await.unwrap();
            }
            assert_eq!(provider.get_model_config().context_limit, Some(context));
            assert_eq!(ps_requests(&server).await, 1);
            assert!(chat_requests(&server)
                .await
                .iter()
                .all(|payload| payload["options"]["num_ctx"] == context));
        }
    }

    #[tokio::test]
    async fn fully_gpu_resident_load_over_three_quarters_of_ram_downgrades() {
        let (server, provider) = memory_test_provider(false, OLLAMA_LARGE_CONTEXT_SIZE).await;
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{
                "model": provider.model.model_name, "size": 97, "size_vram": 97,
                "context_length": 131072,
            }]})))
            .expect(1)
            .mount(&server)
            .await;
        let memory = provider.loaded_model_memory(&provider.model.model_name);
        memory
            .assess(
                &provider.api_client,
                &provider.model.model_name,
                131072,
                128,
            )
            .await;
        assert_eq!(memory.cap(131072), 65536);
        let boundary = LoadedModel {
            name: String::new(),
            model: String::new(),
            size: 96,
            size_vram: 96,
            context_length: 131072,
        };
        assert!(!loaded_model_needs_smaller_window(&boundary, 128, Some(32)));
        assert!(loaded_model_needs_smaller_window(&boundary, 128, Some(24)));
        assert!(
            !loaded_model_needs_smaller_window(&boundary, 0, Some(0)),
            "a remote server is not compared with client RAM"
        );
    }

    #[tokio::test]
    async fn failed_or_unrelated_memory_probes_are_retried_without_changing_the_window() {
        let (server, provider) = memory_test_provider(false, OLLAMA_LARGE_CONTEXT_SIZE).await;
        let memory = provider.loaded_model_memory(&provider.model.model_name);
        for body in [
            json!({"models": []}),
            json!({"models": [{"model": "another-model", "size": 100, "size_vram": 0, "context_length": 131072}]}),
            json!({"models": [{"model": provider.model.model_name, "size": 100, "size_vram": 0, "context_length": 65536}]}),
            json!({"models": [{"model": provider.model.model_name, "size": 100}]}),
        ] {
            server.reset().await;
            Mock::given(method("GET"))
                .and(path("/api/ps"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            memory
                .assess(
                    &provider.api_client,
                    &provider.model.model_name,
                    131072,
                    128,
                )
                .await;
            assert_eq!(memory.cap(131072), 131072);
            assert!(memory.checked_windows.lock().await.is_empty());
        }
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        memory
            .assess(
                &provider.api_client,
                &provider.model.model_name,
                131072,
                128,
            )
            .await;
        assert_eq!(memory.cap(131072), 131072);
        assert!(memory.checked_windows.lock().await.is_empty());
    }

    #[test]
    fn loaded_model_names_accept_the_implicit_latest_tag_only() {
        assert!(same_ollama_model("llama3.1", "llama3.1:latest"));
        assert!(same_ollama_model("org/llama3.1", "org/llama3.1:latest"));
        assert!(!same_ollama_model("llama3.1:8b", "llama3.1:latest"));
        assert!(!same_ollama_model("", "llama3.1:8b"));
        assert!(same_ollama_model(
            "registry.ollama.ai/library/llama3.1:8b",
            "llama3.1:8b"
        ));
        assert!(same_ollama_model("library/llama3.1", "llama3.1:latest"));
        assert!(!same_ollama_model(
            "example.com/library/llama3.1:8b",
            "llama3.1:8b"
        ));
        assert!(!same_ollama_model("team/llama3.1:8b", "llama3.1:8b"));
    }

    #[tokio::test]
    async fn reasoning_only_stream_is_an_explicit_provider_error() {
        let body = format!(
            "{}\n{}\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"","thinking":"thinking only"},"done":false}"#,
            DONE
        );
        let (server, provider) = provider_for(&body).await;

        let error = collect_stream(&provider)
            .await
            .expect_err("reasoning without an answer must fail the provider stream");
        assert!(matches!(error, ProviderError::RequestFailed(_)));
        let detail = error.to_string();
        assert!(detail.contains("completed without answer text or tool calls"));
        assert!(detail.contains("reasoning only"));
        let requests = chat_requests(&server).await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["stream"], true);
    }

    #[tokio::test]
    async fn a_reply_cut_at_its_token_limit_says_so() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"","thinking":"long"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"length","prompt_eval_count":30,"eval_count":64}"#,
            "\n",
        );
        let (_server, provider) = provider_for(body).await;
        let error = collect_stream(&provider).await.unwrap_err();
        assert!(error.to_string().contains("token limit"), "{error}");
    }

    #[tokio::test]
    async fn an_empty_stream_is_an_explicit_provider_error() {
        let (_server, provider) = provider_for(&format!("{DONE}\n")).await;

        let error = collect_stream(&provider)
            .await
            .expect_err("an empty provider stream must not be reported as success");
        assert!(matches!(error, ProviderError::RequestFailed(_)));
        assert!(error
            .to_string()
            .contains("completed without answer text or tool calls"));
    }

    #[tokio::test]
    async fn normal_text_stream_remains_successful() {
        let body = format!(
            "{}\n{}\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"ready"},"done":false}"#,
            DONE
        );
        let (server, provider) = provider_for(&body).await;

        let items = collect_stream(&provider)
            .await
            .expect("text response should remain successful");
        let text = items
            .iter()
            .filter_map(|(message, _, _)| message.as_ref())
            .map(Message::as_concat_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(text, ["ready"]);
        assert_eq!(chat_requests(&server).await.len(), 1);
    }

    #[tokio::test]
    async fn completed_tool_call_stream_remains_successful() {
        let body = concat!(
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"","tool_calls":[{"id":"call_1","function":{"index":0,"name":"shell","arguments":{"command":"pwd"}}}]},"done":false}"#,
            "\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":30,"eval_count":9}"#,
            "\n",
        );
        let (_server, provider) = provider_for(body).await;

        let items = collect_stream(&provider)
            .await
            .expect("completed tool call should remain successful");
        let tool_request = items
            .iter()
            .filter_map(|(message, _, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .find_map(|content| content.as_tool_request())
            .expect("stream should emit the completed tool request");
        assert_eq!(tool_request.id, "call_1");
        let call = tool_request
            .tool_call
            .as_ref()
            .expect("tool call should be valid");
        assert_eq!(call.name, "shell");
        assert_eq!(call.arguments.as_ref().unwrap()["command"], "pwd");
    }

    /// The core fix: the same `num_ctx` on a turn, a blocking completion and a
    /// session title (a different one reloads the model), `truncate: false`
    /// everywhere, no default sampling, and the window reported to compaction.
    #[tokio::test]
    async fn every_request_kind_sends_the_same_window_and_refuses_truncation() {
        let server = MockServer::start().await;
        mount_show(
            &server,
            show_body(
                &["completion", "tools", "thinking"],
                json!([false, "low", "medium", "xhigh"]),
            ),
        )
        .await;
        mount_chat(
            &server,
            &format!(
                "{}\n{}\n",
                r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"Mocked Session Title"},"done":false}"#,
                DONE
            ),
        )
        .await;
        let model = unique("qwen3.8:27b");
        let provider = provider_at(&server, &model);
        assert_eq!(
            provider.get_model_config().context_limit,
            ModelConfig::new_or_fail(&model).context_limit,
            "before the first request the configured limit stands"
        );

        collect_stream(&provider).await.unwrap();
        provider
            .complete("system", &[Message::user().with_text("hi")], &[])
            .await
            .unwrap();
        let title = provider
            .generate_session_name(&Conversation::new_unvalidated(vec![
                Message::user().with_text("what is a urine count")
            ]))
            .await
            .unwrap();
        assert_eq!(title, "Mocked Session Title");

        let requests = chat_requests(&server).await;
        assert_eq!(requests.len(), 3);
        for payload in &requests {
            assert_eq!(payload["options"]["num_ctx"], 40_960, "{payload}");
            assert_eq!(payload["truncate"], false);
            assert_eq!(payload["keep_alive"], OLLAMA_DEFAULT_KEEP_ALIVE);
            assert!(payload["options"].get("temperature").is_none(), "{payload}");
            assert!(payload["options"].get("top_p").is_none(), "{payload}");
            assert_eq!(payload["messages"][0]["role"], "system");
        }
        // Every request streams, the blocking ones included, so a long
        // generation is never one long silence under the idle timeout.
        for payload in &requests {
            assert_eq!(payload["stream"], true, "{payload}");
        }
        // Normal effort: the model's own thinking default, and the ceiling for
        // a model that may think.
        assert!(requests[0].get("think").is_none());
        assert!(requests[1].get("think").is_none());
        assert_eq!(
            requests[0]["options"]["num_predict"],
            OLLAMA_THINKING_NUM_PREDICT
        );
        assert_eq!(
            requests[1]["options"]["num_predict"],
            OLLAMA_THINKING_NUM_PREDICT
        );
        // The title: thinking off and a small budget, at the same window.
        assert_eq!(requests[2]["think"], false);
        assert_eq!(requests[2]["options"]["num_predict"], TITLE_NUM_PREDICT);

        assert_eq!(provider.get_model_config().context_limit, Some(40_960));
        assert_eq!(provider.context_window_for(&model).await, Some(40_960));
    }

    async fn think_sent(
        capabilities: &[&str],
        think_values: Value,
        effort: Option<ReasoningEffort>,
    ) -> Option<Value> {
        let server = MockServer::start().await;
        mount_show(&server, show_body(capabilities, think_values)).await;
        mount_chat(
            &server,
            &format!(
                "{}\n{}\n",
                r#"{"model":"m","message":{"role":"assistant","content":"ok"},"done":false}"#, DONE
            ),
        )
        .await;
        let provider = OllamaProvider::from_custom_config(
            ModelConfig::new_or_fail(&unique("m")).with_reasoning_effort(effort),
            config(&server.uri()),
        )
        .unwrap();
        collect_stream(&provider).await.unwrap();
        chat_requests(&server).await[0].get("think").cloned()
    }

    #[tokio::test]
    async fn effort_maps_to_think_only_for_models_that_can_think() {
        let qwen = || json!([false, "low", "medium", "xhigh"]);
        let caps = ["completion", "tools", "thinking"];
        assert_eq!(
            think_sent(&caps, qwen(), Some(ReasoningEffort::Quick)).await,
            Some(json!(false))
        );
        assert_eq!(
            think_sent(&caps, qwen(), Some(ReasoningEffort::Deep)).await,
            Some(json!("xhigh")),
            "the deepest level qwen3.8 declares — never `high`"
        );
        assert_eq!(
            think_sent(&caps, qwen(), Some(ReasoningEffort::Normal)).await,
            None
        );
        assert_eq!(
            think_sent(&caps, json!([false, true]), Some(ReasoningEffort::Deep)).await,
            Some(json!(true)),
            "a boolean thinker gets `true`"
        );
        // A model without the capability: `think` is a 400 there.
        assert_eq!(
            think_sent(
                &["completion", "tools"],
                json!([]),
                Some(ReasoningEffort::Quick)
            )
            .await,
            None
        );
    }

    /// Ollama 0.40.2's verbatim body for a 12,025-token prompt at a 2048
    /// window with `truncate: false`.
    const OVERFLOW_BODY: &str = r#"{"error":"{\"error\":{\"code\":400,\"message\":\"request (12025 tokens) exceeds the available context size (2048 tokens), try increasing it\",\"type\":\"exceed_context_size_error\",\"n_prompt_tokens\":12025,\"n_ctx\":2048}}"}"#;

    #[tokio::test]
    async fn the_overflow_400_is_context_length_exceeded_and_is_sent_once() {
        let server = MockServer::start().await;
        mount_show(&server, show_body(&["completion", "tools"], json!([]))).await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(400).set_body_string(OVERFLOW_BODY))
            .mount(&server)
            .await;
        let provider = provider_at(&server, &unique("qwen3:8b"));
        for streaming in [true, false] {
            let error = if streaming {
                collect_stream(&provider).await.unwrap_err()
            } else {
                provider
                    .complete("system", &[Message::user().with_text("hi")], &[])
                    .await
                    .unwrap_err()
            };
            let ProviderError::ContextLengthExceeded(text) = &error else {
                panic!("expected ContextLengthExceeded, got {error:?}");
            };
            assert!(text.contains("12025") && text.contains("2048"), "{text}");
            // A custom provider: the setting it honours, not the built-in one.
            assert!(text.contains("BIOROUTER_CONTEXT_LIMIT"), "{text}");
            assert!(!text.contains("OLLAMA_NUM_CTX"), "{text}");
            assert!(text.contains("extensions"), "{text}");
        }
        assert_eq!(
            chat_requests(&server).await.len(),
            2,
            "an overflow is never retried"
        );
    }

    #[tokio::test]
    async fn a_missing_model_says_how_to_pull_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_string(r#"{"error":"model 'qwen3.8:27b' not found"}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_string(r#"{"error":"model 'qwen3.8:27b' not found"}"#),
            )
            .mount(&server)
            .await;
        let provider = provider_at(&server, "qwen3.8:27b");
        let error = collect_stream(&provider).await.unwrap_err();
        assert!(
            error.to_string().contains("ollama pull qwen3.8:27b"),
            "{error}"
        );
        assert!(
            !provider.openai_compat.load(Ordering::Relaxed),
            "a missing model is not a missing route"
        );
    }

    /// A declarative Ollama-engine provider pointed at a server with no
    /// `/api/chat` (any OpenAI-compatible server) falls back to `/v1` for good.
    #[tokio::test]
    async fn a_server_without_api_chat_falls_back_to_v1() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(404).set_body_string("404 page not found"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(concat!(
                        "data: {\"id\":\"t\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ready\"},\"finish_reason\":null}]}\n\n",
                        "data: {\"id\":\"t\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1,\"total_tokens\":6}}\n\n",
                        "data: [DONE]\n\n",
                    )),
            )
            .mount(&server)
            .await;
        let provider = provider_at(&server, &unique("m"));
        for _ in 0..2 {
            let items = collect_stream(&provider).await.unwrap();
            let text: String = items
                .iter()
                .filter_map(|(m, _, _)| m.as_ref())
                .map(Message::as_concat_text)
                .collect();
            assert_eq!(text, "ready");
        }
        let paths: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.url.path().to_string())
            .filter(|p| p.ends_with("completions") || p.ends_with("chat"))
            .collect();
        assert_eq!(
            paths,
            ["/api/chat", "/v1/chat/completions", "/v1/chat/completions"],
            "the 404 is remembered"
        );
    }

    #[tokio::test]
    async fn v1_null_finish_reasons_and_quoted_markers_do_not_complete_a_stream() {
        for content in ["partial", r#"quoted \"finish_reason\": \"stop\""#] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/chat"))
                .respond_with(ResponseTemplate::new(404).set_body_string("404 page not found"))
                .mount(&server)
                .await;
            let frame = json!({"id": "t", "model": "m", "choices": [{
                "index": 0, "delta": {"content": content}, "finish_reason": null,
            }]})
            .to_string()
            .replace("\"finish_reason\":null", "\"finish_reason\": null");
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(format!("data: {frame}\n\n")),
                )
                .mount(&server)
                .await;
            let provider = provider_at(&server, &unique("m"));
            let error = collect_stream(&provider).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("ended before the turn was complete"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn blocking_v1_fallback_recovers_only_complete_advertised_tool_calls() {
        for (finish_reason, recovered) in [("stop", true), ("length", false)] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/chat"))
                .respond_with(ResponseTemplate::new(404).set_body_string("404 page not found"))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "t", "model": "m", "choices": [{
                        "index": 0, "message": {"role": "assistant", "content":
                            r#"{"name":"developer__shell","arguments":{"command":"pwd"}}"#},
                        "finish_reason": finish_reason,
                    }], "usage": {"prompt_tokens": 30, "completion_tokens": 5, "total_tokens": 35},
                })))
                .mount(&server)
                .await;
            let provider = provider_at(&server, &unique("m"));
            let (message, _) = provider
                .complete(
                    "system",
                    &[Message::user().with_text("run pwd")],
                    &[shell_tool()],
                )
                .await
                .unwrap();
            assert_eq!(message.is_tool_call(), recovered);
            if recovered {
                let call = message
                    .content
                    .iter()
                    .find_map(MessageContent::as_tool_request)
                    .unwrap();
                assert_eq!(call.tool_call.as_ref().unwrap().name, "developer__shell");
                assert_eq!(
                    call.tool_call.as_ref().unwrap().arguments.as_ref().unwrap()["command"],
                    "pwd"
                );
            }
        }
    }

    /// The defence for servers that ignore `truncate: false`: a reply whose
    /// prompt count cannot be the whole request is an error, and a tool call in
    /// it never reaches the agent.
    #[tokio::test]
    async fn a_reply_to_a_truncated_prompt_fails_and_releases_no_tool_call() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"","tool_calls":[{"id":"c","function":{"name":"shell","arguments":{"command":"rm -rf x"}}}]},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":2050,"prompt_eval_cached_count":4,"eval_count":20}"#,
            "\n",
        );
        let (_server, provider) = provider_for(body).await;
        let big = "The quick brown fox jumps over the lazy dog. ".repeat(2_000);
        let items: Vec<Result<ProviderStreamItem, ProviderError>> = provider
            .stream("system", &[Message::user().with_text(big)], &[])
            .await
            .unwrap()
            .collect()
            .await;
        assert!(
            items
                .iter()
                .filter_map(|item| item.as_ref().ok())
                .all(|(m, _, _)| !m.as_ref().is_some_and(Message::is_tool_call)),
            "no tool call from a truncated prompt"
        );
        let error = items.into_iter().find_map(Result::err).unwrap();
        let ProviderError::ContextLengthExceeded(text) = &error else {
            panic!("expected ContextLengthExceeded, got {error:?}");
        };
        assert!(text.contains("2050 prompt tokens"), "{text}");
        assert!(text.contains("Raise BIOROUTER_CONTEXT_LIMIT"), "{text}");
        assert!(!text.contains("OLLAMA_NUM_CTX"), "{text}");
    }

    /// An Ollama older than 0.12.6 ignores `truncate: false`, so there even the
    /// reply's text waits for the prompt-size check: a reply to a cut prompt
    /// shows the user nothing but the error, and a whole one arrives intact.
    #[tokio::test]
    async fn an_old_server_holds_the_text_until_the_prompt_size_is_checked() {
        let big = "The quick brown fox jumps over the lazy dog. ".repeat(2_000);
        for (prompt_eval_count, truncated) in [(2_050, true), (40_000, false)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/version"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"version": "0.11.4"})),
                )
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/show"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(show_body(&["completion", "tools"], json!([]))),
                )
                .mount(&server)
                .await;
            mount_chat(
                &server,
                &format!(
                    "{}\n{}\n",
                    r#"{"model":"m","message":{"role":"assistant","content":"a guess"},"done":false}"#,
                    json!({"model": "m", "message": {"role": "assistant", "content": ""},
                           "done": true, "done_reason": "stop",
                           "prompt_eval_count": prompt_eval_count, "eval_count": 3}),
                ),
            )
            .await;
            let provider = provider_at(&server, &unique("old"));
            let items: Vec<Result<ProviderStreamItem, ProviderError>> = provider
                .stream("system", &[Message::user().with_text(big.clone())], &[])
                .await
                .unwrap()
                .collect()
                .await;
            let text: String = items
                .iter()
                .filter_map(|item| item.as_ref().ok())
                .filter_map(|(m, _, _)| m.as_ref())
                .map(Message::as_concat_text)
                .collect();
            if truncated {
                assert_eq!(text, "", "nothing from a cut prompt reaches the user");
                assert!(matches!(
                    items.last(),
                    Some(Err(ProviderError::ContextLengthExceeded(_)))
                ));
            } else {
                assert_eq!(text, "a guess");
                assert!(items.iter().all(Result::is_ok));
            }
        }
    }

    #[test]
    fn the_truncation_check_reads_bytes_per_token() {
        let check = TurnCheck::native(
            160_000,
            65_536,
            "llama3.1:8b",
            Some("0.11.4".into()),
            BUILTIN_WINDOW_SETTING,
        );
        assert!(check.hold_text && check.judge_prompt_size);
        // The reporter's numbers: 2046 fresh + 4 cached of a 160 KB request.
        let truncated = Usage::new(Some(2_046), Some(10), Some(2_060)).with_cache(Some(4), None);
        let error = check.truncation(&truncated).unwrap();
        assert!(error.to_string().contains("0.12.6"), "{error}");
        assert!(
            error.to_string().contains("Raise OLLAMA_NUM_CTX"),
            "{error}"
        );
        let ProviderError::ContextLengthExceeded(detail) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(
            super::super::utils::context_overflow_sizes(detail),
            Some((40_000, 4_096)),
            "the agent reads the estimated request and the window Ollama's cut implies: {detail}"
        );
        // A real 40k-token prompt of the same size passes, cached or not.
        let whole = Usage::new(Some(40_000), Some(10), Some(40_010));
        assert!(check.truncation(&whole).is_none());
        let cached = Usage::new(Some(100), Some(10), None).with_cache(Some(39_900), None);
        assert!(check.truncation(&cached).is_none());
        // No count: nothing to judge. A small request: never judged.
        assert!(check.truncation(&Usage::default()).is_none());
        let small = TurnCheck {
            text_bytes: 15_000,
            ..check
        };
        assert!(small.truncation(&truncated).is_none());
    }

    /// A cut to HALF the window is not always beyond 12 bytes per token, so a
    /// count with the exact shape of Ollama's cut is judged at 6: lfm2:24b on
    /// `/v1` kept 16,387 of a 38,570-token request (~160 KB, ~9.8 bytes per
    /// token) and answered with a tool call. The shape alone is not enough.
    #[test]
    fn a_count_shaped_like_ollamas_cut_is_judged_more_strictly() {
        for kept in [2_050, 4_099, 16_387] {
            assert!(looks_like_ollama_cut(kept), "{kept}");
        }
        for real in [2_040, 3_022, 16_000, 31_000, 40_000] {
            assert!(!looks_like_ollama_cut(real), "{real}");
        }
        let check = TurnCheck::openai_compat(160_000, "lfm2:24b");
        let cut = Usage::new(Some(16_387), Some(17), Some(16_404));
        let error = check
            .truncation(&cut)
            .expect("a cut-shaped count at 9.8 bytes per token");
        assert!(
            error.to_string().contains("OLLAMA_CONTEXT_LENGTH"),
            "{error}"
        );
        // The same ratio without the cut's shape is a plausible prompt.
        let plausible = Usage::new(Some(16_000), Some(17), Some(16_017));
        assert!(check.truncation(&plausible).is_none());
        // And the shape at an ordinary ratio is a prompt that happens to be
        // that long.
        let ordinary = TurnCheck::openai_compat(60_000, "lfm2:24b");
        assert!(ordinary.truncation(&cut).is_none());
    }

    /// On Linux the window is sized by a GPU, never by the sidecar's system-RAM
    /// figure: that is about the bundled CPU llama-server, while Ollama has
    /// CUDA there. Without the Linux arm a GPU-less runner reads `cpu_only`
    /// here (the bundled binary is absent, and no `nvidia-smi` answers).
    #[test]
    fn linux_sizes_ollama_by_its_gpu_never_by_system_ram() {
        use super::super::llamacpp_sidecar as sidecar;
        let (gib, kind) = sizing_memory();
        if cfg!(target_os = "linux") {
            assert!(matches!(kind, "vram" | "unknown_vram"), "{kind}");
            assert_eq!(gib, sidecar::discrete_vram_gib());
        } else {
            // Elsewhere the sidecar's own figure is the right one.
            assert_eq!(
                (gib, kind),
                (
                    sidecar::accelerator_memory_gib(),
                    sidecar::accelerator_memory_kind()
                )
            );
        }
    }

    #[test]
    fn the_window_prefers_the_users_setting_then_the_memory_tier_capped_at_training() {
        // Automatic: the memory tier, never below 64k...
        assert_eq!(choose_num_ctx(None, None, Some(262_144), 128), 131_072);
        assert_eq!(choose_num_ctx(None, None, Some(262_144), 64), 131_072);
        assert_eq!(choose_num_ctx(None, None, Some(262_144), 16), 65_536);
        assert_eq!(choose_num_ctx(None, None, None, 0), 65_536);
        // ...unless the model was trained on less.
        assert_eq!(choose_num_ctx(None, None, Some(40_960), 128), 40_960);
        // OLLAMA_NUM_CTX beats the model config, which beats the tier.
        assert_eq!(
            choose_num_ctx(Some(8_192), Some(32_768), Some(262_144), 128),
            8_192
        );
        assert_eq!(
            choose_num_ctx(None, Some(32_768), Some(262_144), 128),
            32_768
        );
        // Ollama clamps to the trained window, so the reported number does too.
        assert_eq!(
            choose_num_ctx(Some(500_000), None, Some(262_144), 128),
            262_144
        );
    }

    #[test]
    fn a_name_guessed_limit_is_not_the_users() {
        let _env = env_lock::lock_env([("BIOROUTER_CONTEXT_LIMIT", None::<&str>)]);
        // "llama" in the tag reads as 128k in model.rs's pattern table: that is
        // the guess that hid the 4k window, and it must not become num_ctx.
        let guessed = ModelConfig::new_or_fail("llama3.1:8b");
        assert!(guessed.context_limit.is_some());
        assert_eq!(user_context_limit(&guessed), None);
        assert_eq!(
            user_context_limit(&ModelConfig::new_or_fail("qwen3.8:27b")),
            None
        );
        let pinned = ModelConfig::new_or_fail("llama3.1:8b").with_context_limit(Some(32_768));
        assert_eq!(user_context_limit(&pinned), Some(32_768));
    }

    /// `OLLAMA_HOST` is read the way Ollama's own client reads it
    /// (`envconfig.Host()`), so the Ollama CLI and BioRouter reach one server.
    #[test]
    fn host_settings_resolve_the_way_ollama_reads_them() {
        let cases = [
            // No scheme: Ollama's port.
            ("localhost", "http://localhost:11434/"),
            ("localhost:11434", "http://localhost:11434/"),
            ("127.0.0.1:11500", "http://127.0.0.1:11500/"),
            ("192.168.1.10", "http://192.168.1.10:11434/"),
            ("[::1]:11434", "http://[::1]:11434/"),
            ("::1", "http://[::1]:11434/"),
            ("  ", "http://localhost:11434/"),
            // A URL means what a URL means: `http://` without a port is 80,
            // where `ollama list` with the same OLLAMA_HOST goes too.
            ("http://localhost", "http://localhost/"),
            ("http://127.0.0.1", "http://127.0.0.1/"),
            ("http://gpu-box", "http://gpu-box/"),
            ("http://[::1]", "http://[::1]/"),
            (
                "http://gpu.lab.example:11434",
                "http://gpu.lab.example:11434/",
            ),
            ("https://ollama.example.org", "https://ollama.example.org/"),
            (
                "https://ollama.example.org:8443",
                "https://ollama.example.org:8443/",
            ),
            ("http://proxy.example:80", "http://proxy.example/"),
            (
                "http://localhost:11434/ollama/",
                "http://localhost:11434/ollama/",
            ),
        ];
        for (host, expected) in cases {
            let resolved = resolve_ollama_base_url(host)
                .unwrap_or_else(|e| panic!("{host:?} should resolve: {e}"));
            assert_eq!(resolved.as_str(), expected, "{host:?}");
        }
        assert!(resolve_ollama_base_url("ftp://localhost").is_err());
        assert!(resolve_ollama_base_url("http://").is_err());
    }

    /// A custom provider's `base_url` is BioRouter's own field, where an
    /// `http://` URL without a port has always meant Ollama's port.
    #[test]
    fn a_custom_base_url_keeps_its_ollama_port_default() {
        for (base_url, expected) in [
            ("http://localhost", "http://localhost:11434/"),
            ("http://gpu-box", "http://gpu-box:11434/"),
            ("gpu-box", "http://gpu-box:11434/"),
            ("http://proxy.example:80", "http://proxy.example/"),
            ("https://ollama.example.org", "https://ollama.example.org/"),
        ] {
            assert_eq!(
                resolve_custom_base_url(base_url).unwrap().as_str(),
                expected,
                "{base_url:?}"
            );
        }
        let provider = OllamaProvider::from_custom_config(
            ModelConfig::new_or_fail("qwen3.8:27b"),
            config("http://localhost"),
        )
        .unwrap();
        assert_eq!(provider.resolved_base_url, "http://localhost:11434/");
    }

    #[test]
    fn keep_alive_and_versions_parse() {
        assert_eq!(keep_alive_value("30m"), Some(json!("30m")));
        assert_eq!(keep_alive_value("-1"), Some(json!(-1)));
        assert_eq!(keep_alive_value("3600"), Some(json!(3600)));
        assert_eq!(keep_alive_value(" "), None);
        use ServerGeneration::*;
        for (version, generation) in [
            ("0.40.2", Current),
            ("0.12.6", Current),
            ("0.12.6-rc1", Current),
            ("0.12.5", Cuts),
            ("v0.9.0", Cuts),
            ("0.4.0", Cuts),
            // Before the Go runner the prompt count left out the cache.
            ("0.3.14", CutsAndCountsOnlyNewTokens),
            ("0.1.32", CutsAndCountsOnlyNewTokens),
            // A source build's placeholder, and anything that does not parse:
            // a development build, not an ancient one.
            ("0.0.0", Current),
            ("dev", Current),
        ] {
            assert_eq!(server_generation(version), generation, "{version}");
        }
    }

    #[tokio::test]
    async fn the_model_list_hides_tags_that_cannot_call_tools() {
        let server = MockServer::start().await;
        let (tools_model, embedder, mystery) =
            (unique("qwen"), unique("nomic-embed"), unique("mystery"));
        // An embedding model that is ALSO the configured one stays listed.
        let configured_embedder = unique("nomic-configured");
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"name": tools_model},
                {"name": embedder},
                {"name": mystery},
                {"name": configured_embedder},
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(|request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let model = body["model"].as_str().unwrap();
                if model.starts_with("qwen") {
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"capabilities": ["completion", "tools"]}))
                } else if model.starts_with("nomic") {
                    ResponseTemplate::new(200).set_body_json(json!({"capabilities": ["embedding"]}))
                } else {
                    ResponseTemplate::new(500)
                }
            })
            .mount(&server)
            .await;
        let provider = provider_at(&server, &configured_embedder);
        let models = provider.fetch_supported_models().await.unwrap().unwrap();
        let mut expected = vec![mystery, configured_embedder, tools_model];
        expected.sort();
        assert_eq!(models, expected, "{embedder} cannot call tools");
    }

    #[test]
    fn the_recommended_models_are_the_two_supported_tags() {
        let tags: Vec<&str> = OLLAMA_RECOMMENDED_MODELS.iter().map(|m| m.tag).collect();
        assert_eq!(tags, ["qwen3.8:27b", "gemma4:31b"]);
        assert_eq!(OLLAMA_RECOMMENDED_MODELS[0].tag, OLLAMA_DEFAULT_MODEL);
        for model in OLLAMA_RECOMMENDED_MODELS {
            assert!(!model.tag.ends_with(":latest") && !model.tag.contains("nvfp4"));
        }
    }

    /// A tool result reaches the server as the native `tool` message, after a
    /// call whose arguments are an object — the history the model is trained
    /// on, end to end through the provider.
    #[tokio::test]
    async fn tool_history_is_sent_in_the_native_shape() {
        let (server, provider) = provider_for(&format!(
            "{}\n{}\n",
            r#"{"model":"m","message":{"role":"assistant","content":"4 lines"},"done":false}"#,
            DONE
        ))
        .await;
        let call = rmcp::model::CallToolRequestParams {
            task: None,
            name: "developer__shell".into(),
            arguments: Some(rmcp::object!({"command": "wc -l notes.txt"})),
            meta: None,
        };
        let messages = [
            Message::user().with_text("count the lines"),
            Message::assistant().with_tool_request("call_9", Ok(call)),
            Message::user().with_tool_response(
                "call_9",
                Ok(CallToolResult {
                    content: vec![rmcp::model::Content::text("4 notes.txt")],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];
        provider
            .stream("system", &messages, &[])
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        let sent = &chat_requests(&server).await[0]["messages"];
        assert_eq!(
            sent[2]["tool_calls"][0]["function"]["arguments"]["command"],
            "wc -l notes.txt"
        );
        assert_eq!(sent[3]["role"], "tool");
        assert_eq!(sent[3]["tool_name"], "developer__shell");
        assert_eq!(sent[3]["tool_call_id"], "call_9");
    }

    /// `biorouter configure` probes a model with a 50-token budget
    /// (`provider_test.rs`). Ollama thinks when `think` is omitted (qwen3.8 at
    /// medium), so the probe used to return reasoning only and fail the
    /// answered-or-error guard, and the supported default could not be saved.
    /// The mock answers the way Ollama 0.40.2 did, measured: reasoning and
    /// `done_reason: length` when thinking is on, a tool call when it is off.
    #[tokio::test]
    async fn a_small_output_budget_turns_thinking_off_so_the_configure_probe_passes() {
        let server = MockServer::start().await;
        mount_show(
            &server,
            show_body(
                &["completion", "tools", "thinking"],
                json!([false, "low", "medium", "xhigh"]),
            ),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(|request: &wiremock::Request| {
                let payload: Value = serde_json::from_slice(&request.body).unwrap();
                let body = if payload["think"] == false {
                    concat!(
                        r#"{"model":"m","message":{"role":"assistant","content":"","tool_calls":[{"id":"call_w","function":{"index":0,"name":"get_weather","arguments":{"location":"San Francisco"}}}]},"done":false}"#,
                        "\n",
                        r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":180,"eval_count":26}"#,
                        "\n",
                    )
                } else {
                    concat!(
                        r#"{"model":"m","message":{"role":"assistant","content":"","thinking":"The user wants the weather in San Francisco, so I should"},"done":false}"#,
                        "\n",
                        r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"length","prompt_eval_count":180,"eval_count":50}"#,
                        "\n",
                    )
                };
                ResponseTemplate::new(200).set_body_string(body)
            })
            .mount(&server)
            .await;
        let weather = Tool::new(
            "get_weather".to_string(),
            "Get current temperature for a given location.".to_string(),
            rmcp::object!({
                "type": "object",
                "required": ["location"],
                "properties": {"location": {"type": "string"}}
            }),
        );
        let probe = |effort| {
            OllamaProvider::from_custom_config(
                ModelConfig::new_or_fail(&unique("qwen3.8:27b"))
                    .with_max_tokens(Some(50))
                    .with_reasoning_effort(effort),
                config(&server.uri()),
            )
            .unwrap()
        };
        let ask = "What is the weather like in San Francisco today?";

        let (reply, _) = probe(None)
            .complete(
                "system",
                &[Message::user().with_text(ask)],
                std::slice::from_ref(&weather),
            )
            .await
            .expect("the configure probe must pass on a thinking model");
        assert!(reply.is_tool_call());
        let requests = chat_requests(&server).await;
        assert_eq!(requests[0]["think"], false);
        assert_eq!(requests[0]["options"]["num_predict"], 50);

        // Deep effort is an explicit request to think, and is honoured.
        let _ = probe(Some(ReasoningEffort::Deep))
            .complete("system", &[Message::user().with_text(ask)], &[weather])
            .await;
        assert_eq!(chat_requests(&server).await[1]["think"], "xhigh");
    }

    /// A turn whose model config pins no budget gets a ceiling, so a model
    /// caught repeating itself stops: OLLAMA_TIMEOUT bounds only silence, and
    /// Ollama's own default (-1) is no limit at all.
    #[tokio::test]
    async fn a_turn_without_a_pinned_budget_gets_a_runaway_ceiling() {
        async fn num_predict_sent(
            capabilities: &[&str],
            effort: Option<ReasoningEffort>,
            pinned: Option<i32>,
        ) -> Value {
            let server = MockServer::start().await;
            mount_show(
                &server,
                show_body(capabilities, json!([false, "low", "medium", "xhigh"])),
            )
            .await;
            mount_chat(
                &server,
                &format!(
                    "{}\n{}\n",
                    r#"{"model":"m","message":{"role":"assistant","content":"ok"},"done":false}"#,
                    DONE
                ),
            )
            .await;
            let provider = OllamaProvider::from_custom_config(
                ModelConfig::new_or_fail(&unique("m"))
                    .with_reasoning_effort(effort)
                    .with_max_tokens(pinned),
                config(&server.uri()),
            )
            .unwrap();
            collect_stream(&provider).await.unwrap();
            chat_requests(&server).await[0]["options"]["num_predict"].clone()
        }
        let thinker = ["completion", "tools", "thinking"];
        // Thinking left at the model's default (on for qwen3.8): the larger
        // ceiling, because the reasoning spends the budget first.
        assert_eq!(
            num_predict_sent(&thinker, None, None).await,
            OLLAMA_THINKING_NUM_PREDICT
        );
        assert_eq!(
            num_predict_sent(&thinker, Some(ReasoningEffort::Deep), None).await,
            OLLAMA_THINKING_NUM_PREDICT
        );
        // Thinking off, or a model that cannot think: the smaller one.
        assert_eq!(
            num_predict_sent(&thinker, Some(ReasoningEffort::Quick), None).await,
            OLLAMA_DEFAULT_NUM_PREDICT
        );
        assert_eq!(
            num_predict_sent(&["completion", "tools"], None, None).await,
            OLLAMA_DEFAULT_NUM_PREDICT
        );
        // A budget the model config pins always wins.
        assert_eq!(num_predict_sent(&thinker, None, Some(4_000)).await, 4_000);
    }

    /// A blocking call (compaction, a knowledge macro, a title) streams and is
    /// collected into one message, so its silence is bounded per token, and it
    /// passes the same checks as a turn.
    #[tokio::test]
    async fn a_blocking_call_streams_and_is_collected_into_one_message() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"","thinking":"Let me "},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"","thinking":"think."},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"Hello"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":" world"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"","tool_calls":[{"id":"call_1","function":{"index":0,"name":"shell","arguments":{"command":"pwd"}}}]},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":30,"eval_count":9}"#,
            "\n",
        );
        let (server, provider) = provider_for(body).await;
        let (message, usage) = provider
            .complete("system", &[Message::user().with_text("hi")], &[])
            .await
            .unwrap();
        assert_eq!(message.as_concat_text(), "Hello world");
        assert!(message.is_tool_call());
        let thinking: Vec<&str> = message
            .content
            .iter()
            .filter_map(|content| match content {
                MessageContent::Thinking(thinking) => Some(thinking.thinking.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thinking, ["Let me think."], "one thinking block, merged");
        assert_eq!(usage.usage.input_tokens, Some(30));
        assert_eq!(usage.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(chat_requests(&server).await[0]["stream"], true);

        // A truncated reply fails a blocking call exactly as it fails a turn.
        let truncated = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"a guess"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":2050,"eval_count":3}"#,
            "\n",
        );
        let (_server, provider) = provider_for(truncated).await;
        let big = "The quick brown fox jumps over the lazy dog. ".repeat(2_000);
        let error = provider
            .complete("system", &[Message::user().with_text(big)], &[])
            .await
            .unwrap_err();
        assert!(
            matches!(error, ProviderError::ContextLengthExceeded(_)),
            "{error:?}"
        );
    }

    /// Ollama before 0.4.0 counts only the prompt tokens after the cached
    /// prefix, so a cache hit and a cut look the same. Judging them failed the
    /// second request of every tool loop as truncated.
    #[tokio::test]
    async fn a_pre_0_4_server_is_not_judged_by_its_prompt_count() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "0.3.14"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(show_body(&["completion", "tools"], json!([]))),
            )
            .mount(&server)
            .await;
        // ~90 KB of request, of which only the last 300 tokens were not cached.
        mount_chat(
            &server,
            concat!(
                r#"{"model":"m","message":{"role":"assistant","content":"done"},"done":false}"#,
                "\n",
                r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":300,"eval_count":3}"#,
                "\n",
            ),
        )
        .await;
        let provider = provider_at(&server, &unique("ancient"));
        let big = "The quick brown fox jumps over the lazy dog. ".repeat(2_000);
        let items: Vec<ProviderStreamItem> = provider
            .stream("system", &[Message::user().with_text(big)], &[])
            .await
            .unwrap()
            .try_collect()
            .await
            .expect("a cache hit on an old server is not a truncation");
        let text: String = items
            .iter()
            .filter_map(|(m, _, _)| m.as_ref())
            .map(Message::as_concat_text)
            .collect();
        assert_eq!(text, "done");
    }

    /// Whether the reply's text waits for the prompt-size check, by what the
    /// server says its version is. A source build reports `0.0.0` and is
    /// current; holding its text made every reply arrive at once, minutes in.
    #[test]
    fn only_a_server_that_might_cut_unseen_holds_back_the_text() {
        let text_item: ProviderStreamItem =
            (Some(Message::assistant().with_text("live")), None, None);
        let first_item_is_live = |version: Option<&str>| {
            let inner: MessageStream = Box::pin(
                futures::stream::iter(vec![Ok(text_item.clone())])
                    .chain(futures::stream::pending()),
            );
            let check = TurnCheck::native(
                100,
                65_536,
                "m",
                version.map(str::to_string),
                BUILTIN_WINDOW_SETTING,
            );
            let mut guarded = guard_turn(inner, check, "m".to_string(), false);
            futures::FutureExt::now_or_never(guarded.next()).is_some()
        };
        assert!(first_item_is_live(Some("0.40.2")));
        assert!(first_item_is_live(Some("0.0.0")), "a source build");
        assert!(first_item_is_live(Some("dev")));
        assert!(
            first_item_is_live(Some("0.3.14")),
            "nothing can be judged there, so nothing is gained by holding"
        );
        assert!(!first_item_is_live(Some("0.11.4")), "cuts, and it shows");
        assert!(!first_item_is_live(None), "would not say");
    }

    /// The truncation error names the setting that moves THIS window: the
    /// provider's own on the native path, the server's on `/v1`, which cannot
    /// send one.
    #[test]
    fn the_truncation_error_names_the_setting_that_moves_this_window() {
        let cut = Usage::new(Some(2_046), Some(10), Some(2_060)).with_cache(Some(4), None);
        let native = |setting| {
            TurnCheck::native(160_000, 65_536, "m", Some("0.11.4".into()), setting)
                .truncation(&cut)
                .unwrap()
                .to_string()
        };
        assert!(native(BUILTIN_WINDOW_SETTING).contains("Raise OLLAMA_NUM_CTX"));
        let custom = native(CUSTOM_WINDOW_SETTING);
        assert!(custom.contains("Raise BIOROUTER_CONTEXT_LIMIT"), "{custom}");
        assert!(!custom.contains("OLLAMA_NUM_CTX"), "{custom}");
        let v1 = TurnCheck::openai_compat(160_000, "m")
            .truncation(&cut)
            .unwrap()
            .to_string();
        assert!(v1.contains("OLLAMA_CONTEXT_LENGTH"), "{v1}");
        assert!(!v1.contains("OLLAMA_NUM_CTX"), "{v1}");
    }

    /// `OLLAMA_KEEP_ALIVE=-1` is Ollama's documented "keep the model loaded".
    /// The config layer hands a numeric value over as a NUMBER, and reading it
    /// as a string silently replaced it with 30 minutes.
    #[test]
    fn a_numeric_keep_alive_reaches_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .unwrap();
        let keep_alive = |value: Option<&str>| {
            let _env = env_lock::lock_env([
                ("OLLAMA_KEEP_ALIVE", value),
                ("OLLAMA_NUM_CTX", None),
                ("OLLAMA_THINK", None),
            ]);
            read_settings(&config).keep_alive
        };
        assert_eq!(keep_alive(Some("-1")), Some(json!(-1)));
        assert_eq!(keep_alive(Some("0")), Some(json!(0)));
        assert_eq!(keep_alive(Some("3600")), Some(json!(3600)));
        assert_eq!(keep_alive(Some("10m")), Some(json!("10m")));
        assert_eq!(keep_alive(None), Some(json!(OLLAMA_DEFAULT_KEEP_ALIVE)));
        // A YAML number in config.yaml, and the quoted string the settings UI
        // writes.
        config.set_param("OLLAMA_KEEP_ALIVE", json!(-1)).unwrap();
        assert_eq!(keep_alive(None), Some(json!(-1)));
        config.set_param("OLLAMA_KEEP_ALIVE", json!("-1")).unwrap();
        assert_eq!(keep_alive(None), Some(json!(-1)));
        config.set_param("OLLAMA_KEEP_ALIVE", json!("2h")).unwrap();
        assert_eq!(keep_alive(None), Some(json!("2h")));
    }

    fn shell_tool() -> Tool {
        Tool::new(
            "developer__shell".to_string(),
            "Execute a command in the shell. Returns its output.".to_string(),
            rmcp::model::object(json!({
                "type": "object",
                "properties": {"command": {"type": "string"}, "timeout": {"type": "integer"}},
                "required": ["command"]
            })),
        )
    }

    /// An Ollama cloud model runs on ollama.com even through a loopback
    /// `OLLAMA_HOST`: Ollama forwards the conversation there. The tier and the
    /// affiliation follow where the MODEL runs, not where the server is — by
    /// its tag, or by `/api/show`'s `remote_host` for a cloud model renamed
    /// with `ollama cp`, which carries no tag.
    #[tokio::test]
    async fn the_tier_follows_where_the_model_runs_not_only_the_host() {
        use crate::privacy::affiliation::ModelAffiliation;

        let server = MockServer::start().await;
        let local = provider_at(&server, &unique("qwen3.8:27b"));
        assert_eq!(local.tier(), ProviderTier::Private);
        assert_eq!(local.affiliation(), Some(ModelAffiliation::Local));

        for cloud in ["gpt-oss:120b-cloud", "glm-4.6:cloud"] {
            let provider = provider_at(&server, cloud);
            assert_eq!(provider.tier(), ProviderTier::Public, "{cloud}");
            assert_eq!(provider.affiliation(), None, "{cloud}");
        }

        // A renamed cloud model: Private until the server has said, Public
        // from the moment `/api/show` reports a remote host.
        mount_show(
            &server,
            json!({
                "capabilities": ["completion", "tools"],
                "remote_host": "https://ollama.com:443",
                "remote_model": "gpt-oss:120b",
            }),
        )
        .await;
        let renamed = provider_at(&server, &unique("team-model:latest"));
        assert!(renamed
            .context_window_for(&renamed.model.model_name)
            .await
            .is_some());
        assert_eq!(renamed.tier(), ProviderTier::Public);
        assert_eq!(renamed.affiliation(), None);
    }

    /// Biorouter Copilot's consent card names where screenshots go. For a
    /// cloud model behind a loopback server that is ollama.com, not
    /// `http://127.0.0.1:<port>` ("this machine"), and switching between a
    /// local and a cloud model on the same server is a new consent scope.
    #[tokio::test]
    async fn copilot_names_ollama_com_as_a_cloud_models_destination() {
        let server = MockServer::start().await;
        let local = provider_at(&server, &unique("qwen3-vl:8b"));
        let server_origin = local.computer_use_destination().unwrap();
        assert!(
            server_origin.starts_with("http://127.0.0.1:"),
            "{server_origin}"
        );

        let cloud = provider_at(&server, "qwen3-vl:235b-cloud");
        let destination = cloud.computer_use_destination().unwrap();
        assert!(
            destination.starts_with("https://ollama.com (Ollama cloud model"),
            "{destination}"
        );
        assert!(destination.contains(&server_origin), "{destination}");
        assert_ne!(
            cloud.computer_use_destination_identity(),
            local.computer_use_destination_identity()
        );

        // A renamed cloud model: the origin `/api/show` reports.
        mount_show(
            &server,
            json!({
                "capabilities": ["completion", "tools", "vision"],
                "remote_host": "https://cloud.example:443",
                "remote_model": "qwen3-vl:235b",
            }),
        )
        .await;
        let renamed = provider_at(&server, &unique("team-vision:latest"));
        assert!(renamed
            .context_window_for(&renamed.model.model_name)
            .await
            .is_some());
        assert!(
            renamed
                .computer_use_destination()
                .unwrap()
                .starts_with("https://cloud.example (Ollama cloud model"),
            "{:?}",
            renamed.computer_use_destination()
        );
    }

    /// A request for a model served remotely, on an instance that told the
    /// privacy gates its model runs here, is refused before anything is sent:
    /// a cloud `fast_model` behind a local main model.
    #[tokio::test]
    async fn a_cloud_model_on_an_instance_reported_private_is_never_sent() {
        let (server, provider) = provider_for(&format!("{DONE}\n")).await;
        assert_eq!(provider.tier(), ProviderTier::Private);
        let error = provider
            .complete_with_model(
                &ModelConfig::new_or_fail("gpt-oss:120b-cloud"),
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )
            .await
            .expect_err("a cloud model must not be reached from a chat classified local");
        assert!(
            matches!(error, ProviderError::ExecutionError(_)),
            "{error:?}"
        );
        assert!(error.to_string().contains("another host"), "{error}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remembered_v1_fallback_refuses_private_cloud_alternates_but_allows_public_cloud() {
        let server = MockServer::start().await;
        mount_show(&server, show_body(&["completion", "tools"], json!([]))).await;
        mount_v1_only_chat(&server).await;
        let model = unique("qwen3.8:27b");
        let cloud = "gpt-oss:120b-cloud";
        let provider = OllamaProvider::from_custom_config(
            ModelConfig::new_or_fail(&model).with_fast(cloud.to_string()),
            config(&server.uri()),
        )
        .unwrap();
        let messages = [Message::user().with_text("private request")];
        assert_eq!(provider.tier(), ProviderTier::Private);
        let (reply, _) = provider.complete("system", &messages, &[]).await.unwrap();
        assert_eq!(reply.as_concat_text(), "ready");
        assert!(provider.openai_compat.load(Ordering::Relaxed));
        assert_eq!(chat_requests(&server).await.len(), 1);
        let before = server.received_requests().await.unwrap().len();

        let error = provider
            .complete_with_model(&ModelConfig::new_or_fail(cloud), "system", &messages, &[])
            .await
            .unwrap_err();
        assert!(
            matches!(error, ProviderError::ExecutionError(_)),
            "{error:?}"
        );
        assert!(error.to_string().contains("another host"), "{error}");
        assert_eq!(server.received_requests().await.unwrap().len(), before);

        let (reply, _) = provider
            .complete_fast("system", &messages, &[])
            .await
            .unwrap();
        assert_eq!(reply.as_concat_text(), "ready");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), before + 1);
        assert_eq!(requests.last().unwrap().url.path(), "/v1/chat/completions");
        let sent: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
        assert_eq!(sent["model"], model);
        assert_eq!(chat_requests(&server).await.len(), 1);

        let public = provider_at(&server, cloud);
        assert_eq!(public.tier(), ProviderTier::Public);
        let (reply, _) = public.complete("system", &messages, &[]).await.unwrap();
        assert_eq!(reply.as_concat_text(), "ready");
        assert!(public.openai_compat.load(Ordering::Relaxed));
        let requests = server.received_requests().await.unwrap();
        let models: Vec<Value> = requests
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .map(|request| serde_json::from_slice::<Value>(&request.body).unwrap()["model"].clone())
            .collect();
        assert_eq!(models, [json!(model), json!(model), json!(cloud)]);
    }

    #[tokio::test]
    async fn remembered_v1_fallback_refuses_a_discovered_remote_main_on_every_request_kind() {
        let server = MockServer::start().await;
        mount_v1_only_chat(&server).await;
        let model = unique("team-model:latest");
        let provider = provider_at(&server, &model);
        let messages = [Message::user().with_text("private request")];
        assert_eq!(provider.tier(), ProviderTier::Private);
        provider.complete("system", &messages, &[]).await.unwrap();
        assert!(provider.openai_compat.load(Ordering::Relaxed));
        assert_eq!(chat_requests(&server).await.len(), 1);

        mount_show(
            &server,
            json!({
                "capabilities": ["completion", "tools"],
                "remote_host": "https://cloud.example:443",
                "remote_model": "gpt-oss:120b",
            }),
        )
        .await;
        let discovery = provider_at(&server, &model);
        assert!(discovery.model_facts(&model).await.unwrap().remote);
        assert!(provider.reported_private.load(Ordering::Relaxed));
        let before = server.received_requests().await.unwrap().len();

        let error = provider
            .complete("system", &messages, &[])
            .await
            .unwrap_err();
        assert!(
            matches!(error, ProviderError::ExecutionError(_)),
            "{error:?}"
        );
        let error = provider
            .stream("system", &messages, &[])
            .await
            .err()
            .expect("stream must refuse the cached remote model");
        assert!(
            matches!(error, ProviderError::ExecutionError(_)),
            "{error:?}"
        );
        let error = provider
            .generate_session_name(&Conversation::new_unvalidated(messages.to_vec()))
            .await
            .unwrap_err();
        assert!(
            matches!(error, ProviderError::ExecutionError(_)),
            "{error:?}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), before);
    }

    #[tokio::test]
    async fn native_discovery_refuses_a_remote_alias_before_inference() {
        let server = MockServer::start().await;
        let mut show = show_body(&["completion", "tools"], json!([]));
        show["remote_host"] = json!("https://cloud.example:443");
        show["remote_model"] = json!("gpt-oss:120b");
        mount_show(&server, show).await;
        mount_chat(&server, &format!("{DONE}\n")).await;
        let model = unique("team-model:latest");
        let provider = provider_at(&server, &model);
        assert_eq!(provider.tier(), ProviderTier::Private);
        let error = provider
            .complete(
                "system",
                &[Message::user().with_text("private request")],
                &[],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, ProviderError::ExecutionError(_)),
            "{error:?}"
        );
        assert_eq!(provider.get_model_config().context_limit, Some(40_960));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/api/show");
        assert!(provider.serves_remotely(&model));
    }

    /// R19: a model whose `/api/show` capabilities lack `tools` is refused
    /// before the model loads, with the actionable sentence, and is not a
    /// transient error (so nothing retries it).
    #[tokio::test]
    async fn a_model_without_tool_support_is_refused_before_it_loads() {
        let server = MockServer::start().await;
        mount_show(&server, json!({"capabilities": ["completion", "vision"]})).await;
        mount_chat(&server, &format!("{DONE}\n")).await;
        let provider = provider_at(&server, &unique("gemma3:4b"));
        let error = provider
            .stream(
                "system",
                &[Message::user().with_text("list /tmp")],
                &[shell_tool()],
            )
            .await
            .err()
            .expect("a tools request to a model without tools must fail");
        let text = error.to_string();
        assert!(text.contains("cannot call tools"), "{text}");
        assert!(text.contains("BIOROUTER_TOOLSHIM"), "{text}");
        assert!(!error.kind().is_transient(), "{error:?}");
        assert!(chat_requests(&server).await.is_empty());

        // Without tools (Chat mode, a title) the same model is fine.
        let items = provider
            .stream("system", &[Message::user().with_text("hi")], &[])
            .await;
        assert!(items.is_ok());
    }

    /// F11 (live 2026-10-12): deepseek-r1:8b reports `tools`, but its template
    /// never renders them, so a tools request reached it with every tool
    /// definition dropped and it answered for 166 s about tools it never saw.
    /// It is refused before it loads, like a model without tools, and hidden
    /// from the model list the same way.
    #[tokio::test]
    async fn a_model_whose_template_never_shows_tools_is_refused_before_it_loads() {
        let show = json!({
            "capabilities": ["tools", "thinking", "completion"],
            "template": include_str!("../../tests/fixtures/ollama/templates/deepseek-r1_8b.jinja"),
            "modelfile": format!(
                "FROM /blobs/sha256-x\nTEMPLATE \"\"\"{}\"\"\"\n",
                include_str!("../../tests/fixtures/ollama/templates/deepseek-r1_8b.tmpl")
            ),
        });
        let server = MockServer::start().await;
        mount_show(&server, show).await;
        mount_chat(&server, &format!("{DONE}\n")).await;
        let provider = provider_at(&server, &unique("deepseek-r1:8b"));
        let error = provider
            .stream(
                "system",
                &[Message::user().with_text("write factorial.py")],
                &[shell_tool()],
            )
            .await
            .err()
            .expect("a tools request to a tool-blind template must fail");
        let text = error.to_string();
        assert!(text.contains("never shows the tool definitions"), "{text}");
        assert!(text.contains("BIOROUTER_TOOLSHIM"), "{text}");
        assert!(!error.kind().is_transient(), "{error:?}");
        assert!(chat_requests(&server).await.is_empty());
    }

    /// R8: for a template that shows the tools only next to the latest user
    /// message (llama3.1's `if and $.Tools $last`), the system prompt carries
    /// the tool roster on every request; for any other template it does not.
    #[tokio::test]
    async fn the_tool_roster_rides_in_the_system_prompt_only_where_the_template_hides_tools() {
        const LLAMA_TOOLS_BRANCH: &str = "{{- if eq .Role \"user\" }}<|start_header_id|>user<|end_header_id|>\n{{- if and $.Tools $last }}\nGiven the following functions";
        let body = format!(
            "{}\n{}\n",
            r#"{"model":"m","message":{"role":"assistant","content":"ok"},"done":false}"#, DONE
        );
        let system_sent = |requests: Vec<Value>| {
            requests[0]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .to_string()
        };

        let server = MockServer::start().await;
        mount_show(
            &server,
            json!({"capabilities": ["completion", "tools"], "template": LLAMA_TOOLS_BRANCH}),
        )
        .await;
        mount_chat(&server, &body).await;
        let provider = provider_at(&server, &unique("llama3.1:8b"));
        let _ = collect_with_tools(&provider, &[shell_tool()])
            .await
            .unwrap();
        let system = system_sent(chat_requests(&server).await);
        assert!(
            system.starts_with("system\n\n# Tools available at every step"),
            "{system}"
        );
        assert!(
            system
                .contains("- developer__shell(command, timeout?): Execute a command in the shell"),
            "{system}"
        );

        let (server, provider) = provider_for(&body).await;
        let _ = collect_with_tools(&provider, &[shell_tool()])
            .await
            .unwrap();
        assert_eq!(system_sent(chat_requests(&server).await), "system");
    }

    async fn collect_with_tools(
        provider: &OllamaProvider,
        tools: &[Tool],
    ) -> Result<Vec<ProviderStreamItem>, ProviderError> {
        provider
            .stream("system", &[Message::user().with_text("go")], tools)
            .await?
            .try_collect()
            .await
    }

    /// R20's evidence: an empty reply is a swallowed call only when its
    /// output tokens exceed what its reasoning could account for. The numbers
    /// are the measured ones (Ollama 0.40.2).
    #[test]
    fn an_empty_reply_is_a_swallowed_call_only_when_its_tokens_say_so() {
        let guard = |output: i32, thinking_chars: usize, finish: &str| {
            let mut guard = TurnGuard::new(TurnCheck::native(10, 4096, "m", None, "X"));
            guard.output_tokens = Some(output);
            guard.thinking_chars = thinking_chars;
            guard.finish_reason = Some(finish.to_string());
            guard
        };
        // mistral-small3.2 (13, 24, 68), granite4 (28), lfm2 (31): no thinking.
        for tokens in [13, 24, 28, 31, 68] {
            assert_eq!(
                guard(tokens, 0, "stop").empty_reply(),
                EmptyReply::SwallowedCall(tokens)
            );
        }
        // Real reasoning-only replies — qwen3:1.7b's `no_tool` (389
        // characters of thinking in 137 tokens with a 12-character answer)
        // and gpt-oss:20b's `json_answer` (214 in 89 with a 23-character
        // one), their answers removed — are not swallowed calls. At 4.5
        // characters per token both were.
        for (output, chars) in [(137, 389), (89, 214), (42, 100), (356, 1134)] {
            assert_eq!(
                guard(output, chars, "stop").empty_reply(),
                EmptyReply::ReasoningOnly(Some(output)),
                "{output} tokens behind {chars} characters"
            );
        }
        // qwen3:1.7b's swallowed `single` runs at the same ratio: it cannot be
        // told apart, and is reported as what it may be — but still retried.
        let single = guard(135, 482, "stop");
        assert_eq!(single.empty_reply(), EmptyReply::ReasoningOnly(Some(135)));
        assert!(single.empty_reply_may_be_a_lost_call());
        // Far more output than any reasoning ratio explains.
        assert_eq!(
            guard(400, 100, "stop").empty_reply(),
            EmptyReply::SwallowedCall(400)
        );
        // A bare end-of-turn, and a reply cut at its limit.
        assert_eq!(guard(2, 0, "stop").empty_reply(), EmptyReply::Nothing);
        assert!(!guard(2, 0, "stop").empty_reply_may_be_a_lost_call());
        assert_eq!(
            guard(500, 0, "length").empty_reply(),
            EmptyReply::CutAtLimit
        );
        assert!(!guard(500, 0, "length").empty_reply_may_be_a_lost_call());

        let text = guard(13, 0, "stop")
            .empty_reply_error("mistral-small3.2:24b", true, true)
            .to_string();
        assert!(text.contains("13 output tokens"), "{text}");
        assert!(text.contains("could not parse as a tool call"), "{text}");
        assert!(
            text.contains("also after Biorouter asked it once more"),
            "{text}"
        );
        assert!(text.contains("qwen3.8:27b or gemma4:31b"), "{text}");

        let text = guard(137, 389, "stop")
            .empty_reply_error("qwen3:1.7b", true, true)
            .to_string();
        assert!(
            text.contains("reasoning only (137 output tokens)"),
            "{text}"
        );
        assert!(text.contains("may also have written a tool call"), "{text}");
        assert!(!text.contains("could not parse as a tool call"), "{text}");
    }

    /// The advice never recommends the model already in use.
    #[test]
    fn empty_reply_advice_names_only_the_other_supported_model() {
        for (model, names, not) in [
            ("qwen3.8:27b", "gemma4:31b", "qwen3.8:27b"),
            ("qwen3.8:27b-q4_K_M", "gemma4:31b", "qwen3.8:27b"),
            ("gemma4:31b", "qwen3.8:27b", "gemma4:31b"),
        ] {
            let mut guard = TurnGuard::new(TurnCheck::native(10, 4096, "m", None, "X"));
            guard.output_tokens = Some(40);
            guard.finish_reason = Some("stop".to_string());
            let text = guard.empty_reply_error(model, true, true).to_string();
            assert!(text.contains(names), "{model}: {text}");
            assert!(!text.contains(&format!("try {not}")), "{model}: {text}");
            assert!(!text.contains("not reliable"), "{model}: {text}");
        }
        // Without tools there is no tool parser to blame.
        let mut guard = TurnGuard::new(TurnCheck::native(10, 4096, "m", None, "X"));
        guard.output_tokens = Some(40);
        guard.thinking_chars = 300;
        guard.finish_reason = Some("stop".to_string());
        let text = guard
            .empty_reply_error("qwen3:8b", false, false)
            .to_string();
        assert!(text.contains("reasoning only"), "{text}");
        assert!(!text.contains("could not parse"), "{text}");
    }
}
