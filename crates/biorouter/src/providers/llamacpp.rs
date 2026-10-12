//! Llama Server provider — local models served by a Biorouter-managed
//! llama.cpp `llama-server` sidecar (see [`super::llamacpp_sidecar`]).
//!
//! Unlike the Ollama provider, this requires no third-party server: the
//! desktop app bundles the pinned llama-server binary, prefers a GGUF already
//! on disk (the catalog's Hugging Face file, or one pulled into Ollama's local
//! model store), and falls back to llama.cpp's Hugging Face downloader on first
//! use. Setting `LLAMACPP_EXTERNAL_HOST` skips the sidecar entirely and talks
//! to an already-running llama-server (or any OpenAI-compatible llama.cpp
//! endpoint) instead.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use crate::conversation::message::MessageContent;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{Content, RawContent, Tool};
use serde_json::{json, Value};
use std::ops::Deref;
use url::Url;

use super::api_client::{ApiClient, AuthMethod, AuthProvider};
use super::base::{
    ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use super::errors::ProviderError;
use super::llamacpp_sidecar::{
    self, ExpectedGguf, ModelSource, SidecarLease, LLAMACPP_DEFAULT_PORT,
};
use super::retry::{ProviderRetry, TimeoutRetry};
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat,
    stream_openai_compat_showing_reasoning, RequestLog,
};
use crate::agents::effort::ReasoningEffort;
use crate::config::BioRouterMode;
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::formats::openai::{create_request, get_usage, response_to_message};
use crate::providers::formats::text_tool_calls::{
    recover_in_message, recover_leaked_tool_calls, RecoveryContext,
};
use crate::utils::safe_truncate;

/// How long the server may stay silent before a request is abandoned. An IDLE
/// bound, not a total one (see [`ApiClient::with_idle_timeout`]): a dense 27B
/// on a laptop needs minutes to prefill Biorouter's ~31-40k-token bootstrap and
/// can then stream for minutes more, and the old total deadline cut healthy
/// generations off mid-stream and retried the whole prompt against a server
/// still busy with the abandoned one. A timeout is therefore never retried
/// either (see [`TimeoutRetry::GiveUp`] at both request sites).
pub const LLAMACPP_TIMEOUT: u64 = 600;
/// The idle bound on a CPU-only machine (an Intel Mac, the bundled Linux
/// build), where a cold prefill of the bootstrap is tens of minutes of silence
/// on a non-streamed request.
pub const LLAMACPP_CPU_ONLY_TIMEOUT: u64 = 3600;

/// The idle bound for `LLAMACPP_TIMEOUT` as configured: a positive number of
/// seconds is the user's own; `0` (the declared default) or no value means the
/// tiered default for this machine.
///
/// The key used to declare `600` as its default, and the settings form saves a
/// key's declared default whenever it saves the provider — so opening Llama
/// Server's settings on an Intel Mac and pressing Save wrote 600 into
/// config.yaml, overriding the 3600 s CPU-only bound and timing out the cold
/// prefill it exists for.
fn request_timeout_secs(configured: Option<u64>, cpu_only: bool) -> u64 {
    match configured {
        Some(seconds) if seconds > 0 => seconds,
        _ if cpu_only => LLAMACPP_CPU_ONLY_TIMEOUT,
        _ => LLAMACPP_TIMEOUT,
    }
}

/// First run can include a multi-GB fallback download.
pub const LLAMACPP_STARTUP_TIMEOUT: u64 = 3600;
pub const LLAMACPP_DOC_URL: &str =
    "https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md";

/// The catalog model Biorouter selects by default, on every memory tier.
pub const DEFAULT_MODEL: &str = "qwen3.8-27b";

/// Default `max_tokens` for a request that sets none. Without one llama-server
/// generates until the window is full (`n_predict = -1`), so a model caught in
/// a repetition loop burned minutes of a 64-128k window before anything stopped
/// it — and the timeout that finally did was retried.
pub const LLAMACPP_DEFAULT_MAX_TOKENS: i32 = 16_384;
/// The same with thinking on, which spends its own budget before the answer
/// (Qwen evaluates Qwen3.8's coding benchmarks at `max_tokens=32,768`).
pub const LLAMACPP_THINKING_MAX_TOKENS: i32 = 32_768;

/// How a model's chat template takes per-request thinking control, through
/// `chat_template_kwargs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingControl {
    /// Qwen3.8: `enable_thinking`, plus `reasoning_effort` that the template
    /// accepts ONLY as `low`, `medium` or `xhigh` (its default). Anything else —
    /// OpenAI's `high` included — raises inside the template, which llama-server
    /// answers with HTTP 400/500 for the whole request.
    QwenReasoningEffort,
    /// Gemma 4: `enable_thinking` true or false; there is no level.
    Toggle,
}

/// Vendor-recommended sampling, sent unless the user pinned a value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampling {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub presence_penalty: f32,
}

/// One curated local model. `ollama_name` (then `ollama_gguf_names`) is
/// checked in Ollama's manifest/blob store; `hf_spec` is the llama.cpp `-hf`
/// fallback (`owner/repo:quant`). `name` is the friendly id Biorouter serves as
/// `--alias`.
///
/// Fields marked `serde(skip)` are launch and request policy, not catalog
/// facts a picker shows; the HTTP catalog (`routes/llamacpp.rs`) is built field
/// by field and does not carry them.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub display_name: &'static str,
    pub family: &'static str,
    pub ollama_name: Option<&'static str>,
    pub official_url: &'static str,
    pub hf_spec: &'static str,
    /// Approximate download size of the quantized GGUF.
    pub download_size: &'static str,
    pub description: &'static str,
    /// Minimum GPU-addressable memory for a plausible run at the default
    /// context. This means unified memory on Apple Silicon, VRAM on
    /// discrete-GPU systems, and system RAM on a CPU-only machine.
    pub min_gpu_memory_gib: u64,
    /// Recommended GPU-addressable memory for comfortable interactive use.
    pub recommended_gpu_memory_gib: u64,
    /// The model's native context window. What a running server allocates is
    /// the sidecar's `--ctx-size` (64k or 128k), read back from `/props`.
    pub context_limit: usize,
    /// Approximate parameters active per generated token, in billions (the
    /// main tokens/sec driver).
    pub active_params_b: u64,
    /// Short human-readable expected-speed hint, shown in pickers next to
    /// the download size so users opt into heavy models knowingly. It renders
    /// on the same row as the route's suitability message, so any memory figure
    /// in it must be the entry's own minimum and recommendation: it once said
    /// "needs Apple Silicon or a 24 GB+ GPU" beside "Below the minimum: needs
    /// at least 32 GiB" for the same 24 GB card.
    pub speed_hint: &'static str,
    /// Further Ollama tags holding the same model as a plain GGUF (see
    /// [`ModelSource::ollama_gguf_names`]).
    #[serde(skip)]
    pub ollama_gguf_names: &'static [&'static str],
    /// The file `hf_spec` resolves to, with the size and SHA-256 it had when
    /// the catalog was pinned (measured 2026-10-09 via the Hugging Face API).
    #[serde(skip)]
    pub gguf_file: &'static str,
    #[serde(skip)]
    pub gguf_size_bytes: u64,
    #[serde(skip)]
    pub gguf_sha256: &'static str,
    #[serde(skip)]
    pub thinking: ThinkingControl,
    /// Sampling with thinking on, and with it off.
    #[serde(skip)]
    pub sampling_thinking: Sampling,
    #[serde(skip)]
    pub sampling_instruct: Sampling,
    /// Default `--spec-type`, measured per model on b11429 (see the catalog
    /// comment). `None` passes no flag.
    #[serde(skip)]
    pub spec_type: Option<&'static str>,
}

/// The curated catalog: exactly two models, Qwen3.8 27B (the default on every
/// memory tier) and Gemma 4 31B. Both are dense, both have a 256K native
/// window, and both make structured tool calls reliably once they see the whole
/// prompt (18/18 on Ollama and 9/9 on llama.cpp in the 2026-10 investigation of
/// the "model writes JSON instead of calling tools" report, whose root cause was
/// a truncated prompt, not the model). The small Gemma 4 E2B/E4B/12B, the 26B
/// MoE and Qwen3.6 entries were retired with that change; their names still
/// resolve (see [`RETIRED_MODELS`]).
///
/// Sources, most credible first:
/// - **Qwen3.8 27B**: Qwen publishes no GGUF (its last official GGUF repo is
///   from 2026-02). `ggml-org/Qwen3.8-27B-GGUF` is the llama.cpp project's own
///   conversion of the official weights, with the official chat template byte
///   for byte and attention/linear-attention tensors kept at 8 bits.
/// - **Gemma 4 31B**: Google's own quantization-aware-trained Q4_0 GGUF, with
///   the 2026-07-15 template fix for tool-calling loops. Ungated, Apache-2.0.
///
/// The Ollama tags are the bare manifest-list tags (never `:latest`, which is
/// Gemma 4 E4B, and never `-nvfp4`/`-mlx`, which are Apple-Silicon-only). On
/// Apple Silicon a bare tag pulls MLX safetensors that llama-server cannot
/// load, so the GGUF-only tags are probed too.
///
/// Speculative decoding (`spec_type`) was re-measured for both models on
/// b11429, because Qwen3.8's linear-attention layers and Gemma 4's sliding
/// window both force checkpoint-based speculation and the Gemma E4B result in
/// `llamacpp_sidecar::build_args` need not carry over. M4 Max, thinking off,
/// the catalog's sampling, `--spec-type ngram-simple` against `none` on two
/// servers fed the SAME request at the same moment (other agents' 27-31B
/// models held the GPU throughout, so only the ratios mean anything and the
/// tok/s are lower bounds). Generated tok/s, none -> ngram-simple, per pair:
///
/// | 2026-10-10  | rewrite a 270-line file (900 tokens) | 450-word essay |
/// |-------------|--------------------------------------|----------------|
/// | Qwen3.8 27B | 10.0 -> 20.9, 3.4 -> 23.8            | 5.1 -> 4.9     |
/// | Gemma 4 31B | 3.8 -> 7.1, 3.3 -> 12.7              | 5.4 -> 4.8, 3.4 -> 3.2 |
///
/// On the rewrite ~77% of the output came from accepted drafts for both models
/// (696 and 698 of 900 tokens), and every rewritten file still compiled. That
/// is a 2-4x win on the repetition-heavy work an agent mostly does (quoting
/// tool output, rewriting files; the 7x pair is contention noise), against
/// 4-10% on free text, where every draft is rejected. So both keep
/// `ngram-simple`.
///
/// The same day with the GPU to itself (128k window, q8_0 KV, `ngram-simple`):
/// Qwen3.8 27B rewrote the file at 37.0 tok/s, wrote free text at 18.3 tok/s
/// and prefilled a cold ~31k-token prompt in 156 s (~197 tok/s); Gemma 4 31B
/// managed 37.8 and 16.9 tok/s and 238 s for ~32k tokens (~136 tok/s). That
/// prefill is the wait before the first reply of every new chat.
pub const MODEL_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "qwen3.8-27b",
        display_name: "Qwen3.8 27B",
        family: "Qwen3.8",
        // Manifest list: MLX nvfp4 on Apple Silicon, Q4_K_M GGUF on Intel Mac,
        // Linux and Windows. Needs Ollama >= 0.32.12.
        ollama_name: Some("qwen3.8:27b"),
        official_url: "https://ollama.com/library/qwen3.8:27b",
        hf_spec: "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M",
        download_size: "19 GB",
        description: "Default local model: Alibaba's Qwen3.8 27B (August 2026, Apache 2.0), a dense hybrid-attention model built for agentic coding and multi-step tool use, with a 256K native context window",
        // ~21.6 GiB at 64k with q8_0 KV; ~23.7 GiB at 128k.
        min_gpu_memory_gib: 32,
        recommended_gpu_memory_gib: 48,
        context_limit: 262_144,
        active_params_b: 27,
        speed_hint: "Moderate, dense 27B; needs 32 GiB of GPU memory, 48 GiB recommended",
        ollama_gguf_names: &["qwen3.8:27b-q4_K_M"],
        // Converted from Qwen/Qwen3.8-27B@1d4bf0f2; repo main 71bc7b62.
        gguf_file: "Qwen3.8-27B-Q4_K_M.gguf",
        gguf_size_bytes: 18_973_870_528,
        gguf_sha256: "c600de0300ae8a0eb3a6c0b8b5561b8b96f16bd2c863c2a66c42de29d391a747",
        thinking: ThinkingControl::QwenReasoningEffort,
        // Qwen's model card, "Thinking Mode" and "Instruct (or non-thinking)
        // mode"; repetition_penalty 1.0 is llama-server's default.
        sampling_thinking: Sampling {
            temperature: 1.0,
            top_p: 0.95,
            top_k: 20,
            min_p: 0.0,
            presence_penalty: 0.0,
        },
        sampling_instruct: Sampling {
            temperature: 0.7,
            top_p: 0.8,
            top_k: 20,
            min_p: 0.0,
            presence_penalty: 1.5,
        },
        spec_type: SPEC_QWEN38,
    },
    CatalogEntry {
        name: "gemma4-31b",
        display_name: "Gemma 4 31B",
        family: "Gemma 4",
        // Manifest list: MLX nvfp4 (+ draft) on Apple Silicon,
        // gemma4:31b-it-q4_K_M GGUF elsewhere. Needs Ollama >= 0.31.0 (MLX) /
        // 0.30.9 (GGUF).
        ollama_name: Some("gemma4:31b"),
        official_url: "https://ollama.com/library/gemma4:31b",
        // `:Q4_0` resolves through llama.cpp's own `TAG[.-]` file match
        // (Hugging Face's Ollama-compatible manifest API accepts only `:latest`
        // for this repo, but `-hf` does not use it).
        hf_spec: "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0",
        download_size: "18 GB",
        description: "Google DeepMind's Gemma 4 31B (Apache 2.0), a dense model with native function calling and a 256K native context window; the fallback is Google's own quantization-aware-trained Q4_0 GGUF, which keeps near-bf16 quality at 4 bits",
        // ~22.2 GiB at 64k with q8_0 KV; ~24.8 GiB at 128k.
        min_gpu_memory_gib: 32,
        recommended_gpu_memory_gib: 48,
        context_limit: 262_144,
        active_params_b: 31,
        speed_hint: "Moderate, dense 31B; needs 32 GiB of GPU memory, 48 GiB recommended",
        ollama_gguf_names: &["gemma4:31b-it-q4_K_M"],
        // Repo main 59dde245; re-uploaded 2026-07-17 with a corrected vocabulary.
        gguf_file: "gemma-4-31B_q4_0-it.gguf",
        gguf_size_bytes: 17_651_001_568,
        gguf_sha256: "179cfb99212709597eae5929112cfca677e1bbf566178b479ae1da0c4772874b",
        thinking: ThinkingControl::Toggle,
        // Google's model card: one standardized configuration for all uses.
        sampling_thinking: GEMMA4_SAMPLING,
        sampling_instruct: GEMMA4_SAMPLING,
        spec_type: SPEC_GEMMA4,
    },
];

const GEMMA4_SAMPLING: Sampling = Sampling {
    temperature: 1.0,
    top_p: 0.95,
    top_k: 64,
    min_p: 0.0,
    presence_penalty: 0.0,
};

/// Per-model `--spec-type` defaults; see the measurements on [`MODEL_CATALOG`].
/// Kept as two constants so a later measurement can turn one model off alone.
const SPEC_QWEN38: Option<&str> = Some("ngram-simple");
const SPEC_GEMMA4: Option<&str> = Some("ngram-simple");

/// A model the catalog used to offer. Its name stays resolvable for two
/// different purposes, which must not be confused:
///
/// - **chat** ([`chat_model_name`]): a config, session, workflow, schedule or
///   app manifest that still names it runs the default model instead of
///   failing every request with "Unknown Llama Server model";
/// - **cache deletion** ([`resolve_model_source`]): it resolves to its OWN
///   Hugging Face spec, so a user can still delete the 3-24 GB it left on disk.
///   Aliasing here would make "delete gemma4" delete Qwen3.8's cache instead.
#[derive(Clone, Copy, Debug)]
pub struct RetiredModel {
    pub name: &'static str,
    pub hf_spec: &'static str,
    pub ollama_name: Option<&'static str>,
}

pub const RETIRED_MODELS: &[RetiredModel] = &[
    // The 2026-08 to 2026-10 catalog.
    RetiredModel {
        name: "gemma4",
        hf_spec: "google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0",
        ollama_name: Some("gemma4:latest"),
    },
    RetiredModel {
        name: "gemma4-e2b",
        hf_spec: "google/gemma-4-E2B-it-qat-q4_0-gguf:Q4_0",
        ollama_name: Some("gemma4:e2b"),
    },
    RetiredModel {
        name: "gemma4-12b",
        hf_spec: "google/gemma-4-12B-it-qat-q4_0-gguf:Q4_0",
        ollama_name: Some("gemma4:12b"),
    },
    RetiredModel {
        name: "gemma4-26b",
        hf_spec: "google/gemma-4-26B-A4B-it-qat-q4_0-gguf:Q4_0",
        ollama_name: Some("gemma4:26b"),
    },
    RetiredModel {
        name: "qwen3.6",
        hf_spec: "unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_M",
        ollama_name: Some("qwen3.6:latest"),
    },
    RetiredModel {
        name: "qwen3.6-27b",
        hf_spec: "unsloth/Qwen3.6-27B-GGUF:Q4_K_M",
        ollama_name: Some("qwen3.6:27b-q4_K_M"),
    },
    // The first (v1.85) catalog.
    RetiredModel {
        name: "qwen3.5-0.8b",
        hf_spec: "unsloth/Qwen3.5-0.8B-GGUF:Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "qwen3.5-4b",
        hf_spec: "unsloth/Qwen3.5-4B-GGUF:Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "qwen3.5-9b",
        hf_spec: "unsloth/Qwen3.5-9B-GGUF:Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "qwen3.6-35b-a3b",
        hf_spec: "unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "gemma-4-e2b",
        hf_spec: "unsloth/gemma-4-E2B-it-GGUF:Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "gemma-4-e4b",
        hf_spec: "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "gemma-4-12b",
        hf_spec: "unsloth/gemma-4-12b-it-GGUF:Q4_K_M",
        ollama_name: None,
    },
    RetiredModel {
        name: "gemma-4-26b-a4b",
        hf_spec: "unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M",
        ollama_name: None,
    },
];

/// Text substituted for an image the local server cannot see.
const IMAGE_UNSUPPORTED_NOTE: &str = "[An image was provided here, but the local Llama Server \
model is running without a multimodal projector and cannot see images. Describe the image in \
text, or switch to a vision-capable model, to work with it.]";

/// Replace every image in the conversation with a short text note.
///
/// The sidecar launches llama-server with `--no-mmproj`, so the running model
/// has no multimodal projector and cannot accept images at all. Nothing on the
/// send path enforced that: a screenshot (which arrives as an IMAGE inside a
/// tool response, and which the OpenAI formatter then re-emits as a separate
/// user message) was serialized into the request and the server rejected it.
/// The 5xx classified as retryable, so the turn burned three more retries
/// before surfacing an opaque server error — the user asked "what's on screen?"
/// and got a network-ish failure with no hint that the model is text-only.
///
/// Substituting a note keeps the turn coherent and tells the model — and
/// through it the user — exactly what happened and what to do instead. When
/// `--no-mmproj` is eventually dropped for projector-capable models, this
/// becomes conditional on the model's real capability rather than unconditional.
fn strip_images(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .map(|message| {
            let mut out = message.clone();
            out.content = message
                .content
                .iter()
                .map(|content| match content {
                    MessageContent::Image(_) => MessageContent::text(IMAGE_UNSUPPORTED_NOTE),
                    MessageContent::ToolResponse(response) => {
                        let mut response = response.clone();
                        if let Ok(result) = &mut response.tool_result {
                            result.content = result
                                .content
                                .iter()
                                .map(|item| match item.deref() {
                                    RawContent::Image(_) => Content::text(IMAGE_UNSUPPORTED_NOTE),
                                    _ => item.clone(),
                                })
                                .collect();
                        }
                        MessageContent::ToolResponse(response)
                    }
                    other => other.clone(),
                })
                .collect();
            out
        })
        .collect()
}

/// The catalog model for a machine with `gib` of GPU-addressable memory:
/// [`DEFAULT_MODEL`] on every tier. Below either model's
/// `min_gpu_memory_gib` it is still the answer — the route's suitability
/// status carries an explicit below-minimum warning instead of silently
/// offering a model the catalog no longer has.
pub fn recommended_model_for_memory_gib(_gib: u64) -> &'static str {
    DEFAULT_MODEL
}

pub fn default_model_name() -> &'static str {
    recommended_model_for_memory_gib(llamacpp_sidecar::recommendation_memory_gib())
}

pub fn catalog_entry(name: &str) -> Option<&'static CatalogEntry> {
    MODEL_CATALOG.iter().find(|entry| entry.name == name)
}

pub fn retired_model(name: &str) -> Option<&'static RetiredModel> {
    RETIRED_MODELS.iter().find(|model| model.name == name)
}

/// The model to actually run for `model_name`: a retired catalog name runs
/// [`DEFAULT_MODEL`] (logged once per name per process); anything else is
/// itself. Use this for chat — never for cache deletion (see
/// [`RetiredModel`]).
pub fn chat_model_name(model_name: &str) -> &str {
    if retired_model(model_name).is_none() {
        return model_name;
    }
    static LOGGED: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    let first = LOGGED
        .get_or_init(|| StdMutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(model_name.to_string());
    if first {
        tracing::warn!(
            "Llama Server model '{model_name}' was retired from the catalog; running \
             '{DEFAULT_MODEL}' instead. Choose qwen3.8-27b or gemma4-31b to silence this."
        );
    }
    DEFAULT_MODEL
}

/// The model config a provider runs: on the managed sidecar a retired catalog
/// name becomes [`DEFAULT_MODEL`], said in the config the session and its
/// gauge read (with the default's window), not only in the request.
///
/// On an `LLAMACPP_EXTERNAL_HOST` the name is kept verbatim. That server is
/// someone else's: `gemma4` and `qwen3.6` are real Ollama tags, and llama-swap
/// or a router-mode llama-server (`--models-preset`) routes on the name, so
/// rewriting it to `qwen3.8-27b` (as this once did for every host) asks such a
/// server for a model it may not have — "model not found" on every turn, or a
/// different model — and swaps in Qwen3.8's 262k window for the gauge. The
/// retired names only mean something to the sidecar's own catalog.
fn model_for_host(mut model: ModelConfig, external_host: bool) -> ModelConfig {
    if external_host {
        return model;
    }
    let chat_name = chat_model_name(&model.model_name).to_string();
    if chat_name != model.model_name {
        model.model_name = chat_name;
        model.context_limit = ModelConfig::new(&model.model_name)
            .ok()
            .and_then(|m| m.context_limit);
    }
    model
}

/// [`chat_model_name`] plus the source to launch it from.
pub fn resolve_chat_model_source(model_name: &str) -> Result<(String, ModelSource), ProviderError> {
    let name = chat_model_name(model_name);
    Ok((name.to_string(), resolve_model_source(name)?))
}

fn catalog_source(entry: &CatalogEntry) -> ModelSource {
    let source = match entry.ollama_name {
        Some(ollama_name) => ModelSource::ollama(ollama_name, entry.hf_spec),
        None => ModelSource::huggingface(entry.hf_spec),
    };
    source
        .with_ollama_gguf_names(entry.ollama_gguf_names.iter().copied())
        .with_expected_file(ExpectedGguf {
            file_name: entry.gguf_file.to_string(),
            size_bytes: entry.gguf_size_bytes,
            sha256: entry.gguf_sha256.to_string(),
        })
        .with_spec_type(entry.spec_type)
}

/// Resolve a Biorouter model name to the source llama-server loads, EXACTLY:
/// catalog names map to their pinned specs, retired catalog names to their OWN
/// former specs (so their caches stay deletable), and anything containing a
/// `/` is treated as a raw `owner/repo[:quant]` spec and passed through. Chat
/// goes through [`resolve_chat_model_source`], which maps retired names to the
/// default first.
pub fn resolve_model_source(model_name: &str) -> Result<ModelSource, ProviderError> {
    if let Some(entry) = catalog_entry(model_name) {
        return Ok(catalog_source(entry));
    }
    if let Some(retired) = retired_model(model_name) {
        return Ok(match retired.ollama_name {
            Some(ollama_name) => ModelSource::ollama(ollama_name, retired.hf_spec),
            None => ModelSource::huggingface(retired.hf_spec),
        });
    }
    if model_name.contains('/') {
        // A raw `owner/repo[:quant]` Hugging Face spec. This value is passed
        // verbatim to `llama-server -hf` (which builds cache file paths from it
        // under LLAMA_CACHE) and can reach the sidecar from recipes/config, so
        // validate it strictly: reject whitespace/flag-shaped/path-traversal
        // inputs before they hit argv or the on-disk cache layout.
        validate_raw_hf_spec(model_name)?;
        return Ok(ModelSource::huggingface(model_name));
    }
    Err(ProviderError::RequestFailed(format!(
        "Unknown Llama Server model '{model_name}'. Use one of the built-in models ({}) or a \
         Hugging Face GGUF spec like 'owner/repo:Q4_K_M'.",
        MODEL_CATALOG
            .iter()
            .map(|e| e.name)
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

pub fn resolve_hf_spec(model_name: &str) -> Result<String, ProviderError> {
    resolve_model_source(model_name).map(|source| source.hf_spec)
}

/// Validate a raw `owner/repo[:quant]` Hugging Face spec. Each path component
/// (`owner`, `repo`, optional `quant`) must be non-empty and contain only
/// `[A-Za-z0-9._-]`; no component may be `.` or `..`. This blocks path
/// traversal into the model cache, embedded whitespace/newlines, and
/// flag-shaped values like `repo --host`.
fn validate_raw_hf_spec(spec: &str) -> Result<(), ProviderError> {
    let reject = |why: &str| {
        Err(ProviderError::RequestFailed(format!(
            "Invalid Hugging Face model spec '{spec}': {why}. Expected 'owner/repo' or \
             'owner/repo:QUANT' using only letters, digits, '.', '_' and '-'."
        )))
    };

    // Split off an optional `:quant` suffix, then require exactly owner/repo.
    let (repo_part, quant) = match spec.split_once(':') {
        Some((r, q)) => (r, Some(q)),
        None => (spec, None),
    };
    let mut path = repo_part.split('/');
    let (Some(owner), Some(repo), None) = (path.next(), path.next(), path.next()) else {
        return reject("expected a single 'owner/repo'");
    };

    let valid_component = |c: &str| {
        !c.is_empty()
            && c != "."
            && c != ".."
            && c.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    };

    for (component, what) in [(owner, "owner"), (repo, "repo")] {
        if !valid_component(component) {
            return reject(&format!("invalid {what} segment"));
        }
    }
    if let Some(q) = quant {
        if !valid_component(q) {
            return reject("invalid quant suffix");
        }
    }
    Ok(())
}

/// The bearer key of the managed sidecar, read per request so a respawn's
/// fresh key is picked up without rebuilding the client.
struct SidecarAuth;

#[async_trait]
impl AuthProvider for SidecarAuth {
    async fn get_auth_header(&self) -> Result<(String, String)> {
        Ok(match llamacpp_sidecar::current_api_key() {
            Some(key) => ("Authorization".to_string(), format!("Bearer {key}")),
            None => ("X-No-Auth".to_string(), "true".to_string()),
        })
    }
}

struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn get_auth_header(&self) -> Result<(String, String)> {
        Ok(("X-No-Auth".to_string(), "true".to_string()))
    }
}

/// Whether a request should think. `deep` is the strongest setting the model
/// takes (Qwen3.8's `xhigh`); otherwise Qwen3.8 thinks at `medium`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Thinking {
    Off,
    On { deep: bool },
}

/// Map the per-turn reasoning effort onto thinking: Quick turns it off, Deep
/// turns it on at the model's strongest level, and Normal (or none) keeps
/// `LLAMACPP_ENABLE_THINKING`'s default.
fn thinking_for(effort: Option<ReasoningEffort>, default_on: bool) -> Thinking {
    match effort {
        Some(ReasoningEffort::Quick) => Thinking::Off,
        Some(ReasoningEffort::Deep) => Thinking::On { deep: true },
        Some(ReasoningEffort::Normal) | None if default_on => Thinking::On { deep: false },
        Some(ReasoningEffort::Normal) | None => Thinking::Off,
    }
}

/// Post-process the OpenAI-format payload for llama-server: the model alias,
/// per-request thinking control, the vendor's sampling and a `max_tokens`
/// ceiling. Every value the caller already set is left alone, so a pinned
/// temperature (`BIOROUTER_TEMPERATURE`, Quick's 0.0, the warm-up's 0.0) or
/// `BIOROUTER_MAX_TOKENS` still wins.
fn apply_request_policy(payload: &mut Value, alias: &str, thinking: Thinking) {
    let entry = catalog_entry(alias);
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    // llama-server serves its one loaded model whatever this says; send the
    // alias it was launched with, not a retired name the session still holds.
    object.insert("model".to_string(), json!(alias));

    let on = matches!(thinking, Thinking::On { .. });
    let mut kwargs = serde_json::Map::new();
    kwargs.insert("enable_thinking".to_string(), json!(on));
    if let (Some(entry), Thinking::On { deep }) = (entry, thinking) {
        if entry.thinking == ThinkingControl::QwenReasoningEffort {
            // Always stated, never left to the template: Qwen3.8's template
            // DEFAULTS to `xhigh` when thinking is on (it then adds "think
            // carefully, validate key assumptions, consider plausible
            // alternatives" to the system prompt; `medium` adds nothing). So a
            // Normal turn that named no level WAS a Deep one — measured on
            // b11429, same seed, byte-identical output — and with
            // `LLAMACPP_ENABLE_THINKING=true` every turn ran at the template's
            // highest level while the effort selector did nothing. `medium` is
            // also what Ollama runs this model at by default. Never "high": the
            // template raises on it (HTTP 500, see ThinkingControl).
            let level = if deep { "xhigh" } else { "medium" };
            kwargs.insert("reasoning_effort".to_string(), json!(level));
        }
    }
    match object.get_mut("chat_template_kwargs") {
        Some(Value::Object(existing)) => {
            for (key, value) in kwargs {
                existing.entry(key).or_insert(value);
            }
        }
        _ => {
            object.insert("chat_template_kwargs".to_string(), Value::Object(kwargs));
        }
    }
    // The OpenAI-family top-level field is honoured by b11429 and handed to the
    // template verbatim; a value Qwen3.8's template rejects fails the request.
    // Thinking is controlled through `chat_template_kwargs` above instead.
    if entry.is_some_and(|e| e.thinking == ThinkingControl::QwenReasoningEffort) {
        object.remove("reasoning_effort");
    }

    if let Some(entry) = entry {
        let sampling = if on {
            entry.sampling_thinking
        } else {
            entry.sampling_instruct
        };
        let mut fill = |key: &str, value: Value| {
            object.entry(key.to_string()).or_insert(value);
        };
        fill("temperature", json!(sampling.temperature));
        fill("top_p", json!(sampling.top_p));
        fill("top_k", json!(sampling.top_k));
        fill("min_p", json!(sampling.min_p));
        fill("presence_penalty", json!(sampling.presence_penalty));
    }

    if !object.contains_key("max_tokens") && !object.contains_key("max_completion_tokens") {
        let ceiling = if on {
            LLAMACPP_THINKING_MAX_TOKENS
        } else {
            LLAMACPP_DEFAULT_MAX_TOKENS
        };
        object.insert("max_tokens".to_string(), json!(ceiling));
    }
}

/// Ask the managed sidecar to stream prefill progress (LC-8).
///
/// The client's timeout is an IDLE bound ([`LLAMACPP_TIMEOUT`]), and a
/// streamed request is silent for the whole prefill: llama-server sends its
/// first chunk only once the prompt is processed. Measured on b11429 with
/// nothing else on the GPU of an M4 Max, a cold ~31k-token first turn
/// prefilled for 156 s (Qwen3.8 27B, ~197 tok/s) and 238 s (Gemma 4 31B,
/// ~136 tok/s). A machine at the 32 GiB minimum has a fraction of those GPU
/// cores, so the same prefill crosses the 600 s bound and a healthy request is
/// abandoned — and retried from the top. With `return_progress` the server
/// sends a `prompt_progress` frame per batch (every few seconds; on b11429 it
/// carries one empty `{"role":"assistant","content":null}` delta and no
/// usage), which the stream decoder drops and which keeps the idle timer fed.
///
/// Only for the managed sidecar, whose build is pinned: an
/// `LLAMACPP_EXTERNAL_HOST` may be some other OpenAI-compatible server that
/// rejects fields it does not know.
fn request_prefill_progress(payload: &mut Value) {
    if let Some(object) = payload.as_object_mut() {
        object
            .entry("return_progress".to_string())
            .or_insert(json!(true));
    }
}

/// The context window to report before the first request reaches the server.
///
/// `ModelConfig::new` fills the window from `MODEL_CONTEXT_WINDOWS`, which for
/// both catalog models is the native 262,144, while the sidecar allocates 64k
/// or 128k. Until the first request read the real window back from `/props`,
/// compaction and the gauge trusted 262k — so a long chat resumed after a
/// restart sent its first turn uncompacted into a 64k server and got
/// `exceed_context_size_error`. The smaller of the two is the one that is true.
fn clamp_to_server_window(registry_window: Option<usize>, server_window: usize) -> usize {
    registry_window.map_or(server_window, |window| window.min(server_window))
}

/// A streamed reply that holds the sidecar's model lease (any guard `G`) only
/// while the server is still generating it.
///
/// The lease cannot simply live as long as the stream: the agent loop runs a
/// reply's tool calls INSIDE its stream loop, before it polls the stream again,
/// so a lease dropped with the stream was held across every tool call and
/// every approval card a tool raised — for hours, if nobody answered. tokio's
/// `RwLock` is write-preferring, so one model switch queued behind that lease
/// then blocked every new request, including the subagent the waiting tool
/// itself had spawned: a deadlock. The decoder yields a tool-call message only
/// once the server has sent `finish_reason`, so generation is over by then and
/// the lease is released right there; only the trailing usage chunk can still
/// be lost to a switch. A text reply releases it at its usage chunk (which
/// b11429 sends once, after `finish_reason`) or at the end of the stream, and
/// an error releases it at once.
///
/// The lease also holds the server's only request slot, so every OTHER request
/// waits for it to be released: a consumer that stops polling a finished reply
/// short of its end would hold up every chat. Releasing at the usage chunk
/// rather than only at the end keeps that window to the `[DONE]` line.
struct GenerationLease<G> {
    inner: MessageStream,
    guard: Option<G>,
}

impl<G: Unpin> futures::Stream for GenerationLease<G> {
    type Item = <MessageStream as futures::Stream>::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        let polled = this.inner.as_mut().poll_next(cx);
        let generation_over = match &polled {
            std::task::Poll::Ready(None | Some(Err(_))) => true,
            std::task::Poll::Ready(Some(Ok((_, Some(_usage), _)))) => true,
            std::task::Poll::Ready(Some(Ok((Some(message), _, _)))) => message
                .content
                .iter()
                .any(|content| matches!(content, MessageContent::ToolRequest(_))),
            _ => false,
        };
        if generation_over {
            this.guard = None;
        }
        polled
    }
}

/// A request to the managed sidecar failed. A compute failure leaves
/// llama-server's GPU backend broken for every later request while `/health`
/// still answers, so the server is discarded and the next request starts a
/// fresh one (see [`llamacpp_sidecar::LlamaSidecar::discard_failed_backend`]).
async fn after_failure(error: ProviderError, port: Option<u16>) -> ProviderError {
    match (port, &error) {
        (Some(port), ProviderError::ServerError(message))
            if llamacpp_sidecar::is_compute_error(message) =>
        {
            match llamacpp_sidecar::global()
                .discard_failed_backend(port)
                .await
            {
                Some(explained) => ProviderError::ServerError(explained),
                None => error,
            }
        }
        _ => error,
    }
}

fn hold_lease_while_generating<G: Send + Unpin + 'static>(
    stream: MessageStream,
    guard: Option<G>,
) -> MessageStream {
    match guard {
        None => stream,
        Some(guard) => Box::pin(GenerationLease {
            inner: stream,
            guard: Some(guard),
        }),
    }
}

fn hold_lease_while_generating_with_recovery<G, F, Fut>(
    mut stream: MessageStream,
    guard: Option<G>,
    recover: F,
) -> MessageStream
where
    G: Send + Sync + Unpin + 'static,
    F: Fn(ProviderError, Arc<G>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ProviderError> + Send + 'static,
{
    use futures::StreamExt;

    let Some(guard) = guard else {
        return stream;
    };
    let guard = Arc::new(guard);
    // The error adapter must not keep the lease alive after the outer stream
    // releases it for a tool call or usage. Late errors then belong to a
    // completed generation and must not discard a replacement server.
    let weak_guard = Arc::downgrade(&guard);
    let recovered = Box::pin(async_stream::try_stream! {
        while let Some(item) = stream.next().await {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    let error = match weak_guard.upgrade() {
                        Some(guard) => recover(error, guard).await,
                        None => error,
                    };
                    Err(error)?;
                    break;
                }
            };
            yield item;
        }
    });
    hold_lease_while_generating(recovered, Some(guard))
}

#[derive(serde::Serialize)]
pub struct LlamaCppProvider {
    model: ModelConfig,
    /// Set when LLAMACPP_EXTERNAL_HOST points at an unmanaged server.
    external_base: Option<String>,
    #[serde(skip)]
    client: tokio::sync::Mutex<Option<(String, Arc<ApiClient>)>>,
    /// The running model's real context window, read from the server's `/props`
    /// once it is up (0 until known). The window is a live property of the
    /// loaded model — not a fixed catalog/config constant — so we cannot know it
    /// at construction (the sidecar starts lazily on first use). It is refreshed
    /// in `ensure_client` and overrides the construction-time fallback in
    /// `get_model_config`, keeping token accounting in sync with the model the
    /// server actually loaded.
    #[serde(skip)]
    live_context_limit: AtomicUsize,
    request_timeout: Duration,
    startup_timeout: Duration,
    name: String,
}

impl LlamaCppProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        let external_base = config
            .get_param::<String>("LLAMACPP_EXTERNAL_HOST")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .map(|host| {
                let base = if host.starts_with("http://") || host.starts_with("https://") {
                    host
                } else {
                    format!("http://{host}")
                };
                let url = Url::parse(&base)
                    .map_err(|e| anyhow::anyhow!("Invalid LLAMACPP_EXTERNAL_HOST: {e}"))?;
                // Only http(s) is meaningful here; reject file://, ftp://, etc.
                // so a malformed value fails loudly instead of being handed to
                // the HTTP client.
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(anyhow::anyhow!(
                        "Invalid LLAMACPP_EXTERNAL_HOST: scheme '{}' is not http/https",
                        url.scheme()
                    ));
                }
                // Setting this bypasses the managed, loopback-only sidecar and
                // sends full prompts (system + messages + tool schemas) to an
                // unmanaged endpoint with no auth. Warn so a config-injected or
                // non-loopback host is visible rather than a silent exfil path.
                let is_loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
                if !is_loopback {
                    tracing::warn!(
                        "LLAMACPP_EXTERNAL_HOST points at a non-loopback host ({}); \
                         conversation contents will be sent there unauthenticated",
                        url.host_str().unwrap_or("?")
                    );
                }
                Ok::<String, anyhow::Error>(url.to_string())
            })
            .transpose()?;

        let mut model = model_for_host(model, external_base.is_some());

        // Construction-time context window for token accounting, used until the
        // server reports the loaded model's real window (`live_context_limit`,
        // refreshed in `ensure_client`). For the managed sidecar the window the
        // server will hold is known now — the one it already recorded, else the
        // `--ctx-size` it will be launched with — so the registry's native
        // window is clamped to it (see `clamp_to_server_window`). An explicit
        // BIOROUTER_CONTEXT_LIMIT is the user's own number and is kept.
        if external_base.is_none() {
            if std::env::var("BIOROUTER_CONTEXT_LIMIT").is_err() {
                let server_window = match llamacpp_sidecar::current_context_size().await {
                    Some(live) => live,
                    None => llamacpp_sidecar::configured_context_size(),
                };
                let window = clamp_to_server_window(model.context_limit, server_window);
                model = model.with_context_limit(Some(window));
            }
        } else if model.context_limit.is_none() {
            let ctx = config
                .get_param::<usize>("LLAMACPP_CONTEXT_SIZE")
                .ok()
                .filter(|&n| n > 0)
                .unwrap_or_else(llamacpp_sidecar::default_context_size);
            model = model.with_context_limit(Some(ctx));
        }

        let request_timeout = Duration::from_secs(request_timeout_secs(
            config.get_param::<u64>("LLAMACPP_TIMEOUT").ok(),
            llamacpp_sidecar::is_cpu_only(),
        ));
        let startup_timeout = Duration::from_secs(
            config
                .get_param("LLAMACPP_STARTUP_TIMEOUT")
                .unwrap_or(LLAMACPP_STARTUP_TIMEOUT),
        );

        Ok(Self {
            model,
            external_base,
            client: tokio::sync::Mutex::new(None),
            live_context_limit: AtomicUsize::new(0),
            request_timeout,
            startup_timeout,
            name: Self::metadata().name,
        })
    }

    /// Make sure a server is reachable for `model_name` and return a client
    /// pointed at it, plus — for the managed sidecar — the lease that keeps a
    /// model switch from replacing the server under this request. Hold the
    /// lease until the response is fully read. Managed mode starts (and waits
    /// on) the sidecar; catalog models use a local GGUF first and fall back to
    /// `-hf`.
    async fn ensure_client(
        &self,
        model_name: &str,
    ) -> Result<(Arc<ApiClient>, Option<SidecarLease>), ProviderError> {
        let (base_url, lease) = if let Some(base) = &self.external_base {
            (base.clone(), None)
        } else {
            let (name, source) = resolve_chat_model_source(model_name)?;
            let lease = llamacpp_sidecar::global()
                .lease(&name, &source, self.startup_timeout)
                .await
                .map_err(|e| {
                    ProviderError::ExecutionError(format!(
                        "Llama Server could not start {name}: {e}"
                    ))
                })?;
            // The server is up: capture the model's real context window so token
            // accounting reflects the loaded model rather than the
            // construction-time fallback. Best-effort — leave the fallback if
            // the window can't be read.
            if let Some(n) = llamacpp_sidecar::current_context_size().await {
                self.live_context_limit.store(n, Ordering::Relaxed);
            }
            (format!("http://127.0.0.1:{}/", lease.port()), Some(lease))
        };

        let mut cached = self.client.lock().await;
        if let Some((url, client)) = cached.as_ref() {
            if url == &base_url {
                return Ok((client.clone(), lease));
            }
        }
        let auth = if self.external_base.is_some() {
            AuthMethod::Custom(Box::new(NoAuth))
        } else {
            AuthMethod::Custom(Box::new(SidecarAuth))
        };
        let client = Arc::new(
            ApiClient::with_idle_timeout(base_url.clone(), auth, self.request_timeout)
                .map_err(|e| ProviderError::ExecutionError(e.to_string()))?,
        );
        *cached = Some((base_url, client.clone()));
        Ok((client, lease))
    }

    /// The alias a request for `model_name` is served under.
    fn served_alias(&self, model_name: &str) -> String {
        if self.external_base.is_some() {
            model_name.to_string()
        } else {
            chat_model_name(model_name).to_string()
        }
    }

    fn filtered_tools(tools: &[Tool]) -> &[Tool] {
        let config = crate::config::Config::global();
        if config.get_biorouter_mode().unwrap_or(BioRouterMode::Auto) == BioRouterMode::Chat {
            &[]
        } else {
            tools
        }
    }

    /// One non-streamed completion. `thinking` overrides the effort mapping
    /// (warm-up and title requests turn it off).
    async fn complete_inner(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        thinking: Option<Thinking>,
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let filtered_tools = Self::filtered_tools(tools);
        let (client, lease) = self.ensure_client(&model_config.model_name).await?;

        // The sidecar runs with `--no-mmproj`, so images cannot reach the model.
        let messages = &strip_images(messages);
        let mut payload = create_request(
            model_config,
            system,
            messages,
            filtered_tools,
            &super::utils::ImageFormat::OpenAi,
            false,
        )?;
        let thinking = thinking.unwrap_or_else(|| {
            thinking_for(
                model_config.reasoning_effort,
                llamacpp_sidecar::thinking_enabled_by_default(),
            )
        });
        apply_request_policy(
            &mut payload,
            &self.served_alias(&model_config.model_name),
            thinking,
        );

        let mut log = RequestLog::start(model_config, &payload)?;
        // A timeout is handed straight back, never re-sent: this request was
        // the one the server was busy with, and a non-streamed reply is silent
        // until it is complete, so a resend queued a second copy of the prompt
        // behind the abandoned first — four tries, 40 minutes (four hours
        // CPU-only) before the user saw an error. Connection failures, as
        // while a server starts, are still retried.
        let response = match self
            .with_retry_policy(TimeoutRetry::GiveUp, || async {
                let response = client
                    .response_post("v1/chat/completions", &payload)
                    .await?;
                handle_response_openai_compat(response).await
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let _ = log.error(&error);
                return Err(after_failure(error, lease.as_ref().map(SidecarLease::port)).await);
            }
        };

        let message = recover_in_message(
            response_to_message(&response)?,
            filtered_tools,
            response
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str),
            "llamacpp",
            &model_config.model_name,
        );
        let usage = response.get("usage").map(get_usage).unwrap_or_else(|| {
            tracing::debug!("Failed to get usage data");
            Usage::default()
        });
        let response_model = get_model(&response);
        log.write(&response, Some(&usage))?;
        Ok((message, ProviderUsage::new(response_model, usage)))
    }

    /// A completion with thinking off, whatever the effort or
    /// `LLAMACPP_ENABLE_THINKING` say: the warm-up check and other short
    /// utility requests, which must spend their small budget on visible text.
    pub async fn complete_without_thinking(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete_inner(model_config, system, messages, tools, Some(Thinking::Off))
            .await
    }

    fn filter_reasoning_tokens(text: &str) -> String {
        let mut filtered = text.to_string();
        for tag in ["<think>", "</think>", "<thinking>", "</thinking>"] {
            filtered = filtered.replace(tag, "");
        }
        filtered
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[async_trait]
impl Provider for LlamaCppProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::with_models(
            "llamacpp",
            "Llama Server",
            "Built-in local models (llama.cpp): private, free, no setup required",
            default_model_name(),
            MODEL_CATALOG
                .iter()
                .map(|e| ModelInfo::new(e.name, e.context_limit))
                .collect(),
            LLAMACPP_DOC_URL,
            vec![
                ConfigKey::new(
                    "LLAMACPP_PORT",
                    true,
                    false,
                    Some(&LLAMACPP_DEFAULT_PORT.to_string()),
                ),
                // Default "0" = Biorouter chooses a memory-tiered context
                // window. Set a positive value only to cap the window for
                // memory reasons.
                ConfigKey::new("LLAMACPP_CONTEXT_SIZE", false, false, Some("0")),
                // Default "0" = the tiered idle bound (600 s, or 3600 s on a
                // CPU-only machine); see `request_timeout_secs`. A numeric
                // default, not none, so the settings form saves a number.
                ConfigKey::new("LLAMACPP_TIMEOUT", false, false, Some("0")),
                ConfigKey::new("LLAMACPP_ENABLE_THINKING", false, false, Some("false")),
                ConfigKey::new("LLAMACPP_EXTERNAL_HOST", false, false, None),
                // A Hugging Face token, only for gated repos (the catalog's
                // are not). A credential, so it is a SECRET: the settings
                // form stores it in the keyring, never in config.yaml.
                ConfigKey::new("LLAMACPP_HF_TOKEN", false, true, None),
            ],
        )
        .with_unlisted_models()
        // The managed sidecar runs here, so a default install is Private. An
        // instance pointed at an external host says so itself, below.
        .with_tier(ProviderTier::Private)
        .with_local_compute()
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn tier(&self) -> ProviderTier {
        match &self.external_base {
            // No external host: this is the managed sidecar, which
            // `llamacpp_sidecar` binds to loopback and reasserts loopback-last
            // on the command line even when a host is injected.
            None => ProviderTier::Private,
            // LLAMACPP_EXTERNAL_HOST is user-writable and needs no auth, so the
            // same rule as ollama applies: loopback or nothing.
            Some(base) => crate::providers::self_hosted_tier(base),
        }
    }

    /// DR-26: `Local`, following the **same branch** as the tier above rather
    /// than a new rule — the managed sidecar runs here by construction, and an
    /// injected `LLAMACPP_EXTERNAL_HOST` is Local only while it is loopback. A
    /// remote host gets no affiliation rather than inheriting `Local`'s blanket
    /// permission over every private extension.
    fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
        match &self.external_base {
            None => Some(crate::privacy::affiliation::ModelAffiliation::Local),
            Some(base) => crate::providers::self_hosted_affiliation(base),
        }
    }

    fn get_model_config(&self) -> ModelConfig {
        // Prefer the running model's real context window (read from `/props`)
        // over the construction-time fallback, so callers (token accounting,
        // context gauges) see the actual window. Falls back to the stored
        // (already server-clamped) value until the server has been contacted
        // at least once.
        let live = self.live_context_limit.load(Ordering::Relaxed);
        if live > 0 {
            self.model.clone().with_context_limit(Some(live))
        } else {
            self.model.clone()
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
        self.complete_inner(model_config, system, messages, tools, None)
            .await
    }

    async fn generate_session_name(
        &self,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        let context = self.get_initial_user_messages(messages);
        let message = Message::user().with_text(self.create_session_name_prompt(&context));
        // A title is four words: no thinking, and a small ceiling, so it neither
        // spends minutes reasoning nor holds the single server slot long while
        // the first agent turn queues behind it.
        let mut model_config = self.get_model_config();
        model_config.max_tokens = Some(64);
        let result = self
            .complete_without_thinking(
                &model_config,
                "You are a title generator. Output only the requested title of 4 words or less, with no additional text, reasoning, or explanations.",
                &[message],
                &[],
            )
            .await?;

        let description = Self::filter_reasoning_tokens(&result.0.as_concat_text());
        Ok(safe_truncate(&description, 100))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let filtered_tools = Self::filtered_tools(tools);
        let (client, lease) = self.ensure_client(&self.model.model_name).await?;

        let messages = &strip_images(messages);
        let mut payload = create_request(
            &self.model,
            system,
            messages,
            filtered_tools,
            &super::utils::ImageFormat::OpenAi,
            true,
        )?;
        apply_request_policy(
            &mut payload,
            &self.served_alias(&self.model.model_name),
            thinking_for(
                self.model.reasoning_effort,
                llamacpp_sidecar::thinking_enabled_by_default(),
            ),
        );
        if self.external_base.is_none() {
            request_prefill_progress(&mut payload);
        }
        let mut log = RequestLog::start(&self.model, &payload)?;

        // Never re-send on a timeout; see `complete_inner`.
        let response = match self
            .with_retry_policy(TimeoutRetry::GiveUp, || async {
                let resp = client
                    .response_post("v1/chat/completions", &payload)
                    .await?;
                handle_status_openai_compat(resp).await
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let _ = log.error(&error);
                return Err(after_failure(error, lease.as_ref().map(SidecarLease::port)).await);
            }
        };
        // The thinking is SHOWN, not only captured: a Deep turn on Qwen3.8
        // thinks at `xhigh`, and the capture-only decoder kept every word of
        // it out of the chat (the "invisible thinking" OLL-07 describes). It
        // is not live yet: the decoder coalesces a reasoning phase into ONE
        // block and yields it when the phase ends, just before the answer or
        // tool call (`formats::openai::take_thinking`; measured live, the
        // block arrived 4.3 s into a 4.4 s Normal turn). Safe because
        // `create_request` formats with `formats::openai::format_messages`,
        // which drops `Thinking` content, so the unsigned blocks this yields
        // are never sent back.
        //
        // A call the model wrote as text instead of in llama-server's tool
        // format is recovered under the same guards as on Ollama (TIER1:
        // the whole reply is the call, every name advertised in this request,
        // every argument object valid for its tool), with hold-back so the
        // user never watches the JSON being typed. The lease sees the
        // recovered call arrive with the usage, as it would a parsed one.
        let recovered = recover_leaked_tool_calls(
            stream_openai_compat_showing_reasoning(response, log)?,
            RecoveryContext {
                tools: filtered_tools.to_vec(),
                provider: "llamacpp",
                model: self.model.model_name.clone(),
            },
        );
        Ok(hold_lease_while_generating_with_recovery(
            recovered,
            lease,
            |error, lease| async move { after_failure(error, Some(lease.port())).await },
        ))
    }

    /// The curated catalog. Built-ins are tied to Ollama library model names
    /// when available; unlisted Hugging Face specs are still accepted.
    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        Ok(Some(
            MODEL_CATALOG.iter().map(|e| e.name.to_string()).collect(),
        ))
    }
}

#[cfg(test)]
impl LlamaCppProvider {
    /// A provider on the managed sidecar for `model`, built as `from_env` builds one but
    /// without reading any configuration, for tests outside this module.
    pub(crate) fn managed_for_test(model: ModelConfig) -> Self {
        Self {
            model,
            external_base: None,
            client: tokio::sync::Mutex::new(None),
            live_context_limit: AtomicUsize::new(0),
            request_timeout: Duration::from_secs(LLAMACPP_TIMEOUT),
            startup_timeout: Duration::from_secs(LLAMACPP_STARTUP_TIMEOUT),
            name: "llamacpp".to_string(),
        }
    }

    /// Record `window` as the loaded model's real context window, as `ensure_client` does
    /// once the server is up on the first request.
    pub(crate) fn observe_live_context_for_test(&self, window: usize) {
        self.live_context_limit.store(window, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    /// A provider wired the way `from_env` builds one, minus the global config
    /// lookup — `from_env` reads `LLAMACPP_EXTERNAL_HOST` from the developer's
    /// real config, so a test that went through it would assert Private on one
    /// machine and fail on a colleague's that legitimately points at a lab box.
    fn provider_with_external_base(external_base: Option<&str>) -> LlamaCppProvider {
        LlamaCppProvider {
            model: ModelConfig::new_or_fail(default_model_name()).with_context_limit(Some(4096)),
            external_base: external_base.map(str::to_string),
            client: tokio::sync::Mutex::new(None),
            live_context_limit: AtomicUsize::new(0),
            request_timeout: Duration::from_secs(LLAMACPP_TIMEOUT),
            startup_timeout: Duration::from_secs(LLAMACPP_STARTUP_TIMEOUT),
            name: "llamacpp".to_string(),
        }
    }

    /// Task 5 rule 1, **wired** — all three arms, none of them environmental.
    ///
    /// `external_base: None` is the managed sidecar, which is spawned by this
    /// process and bound to loopback, so it is the one case that is Private
    /// without a URL to inspect. Every other case is a host `LLAMACPP_EXTERNAL_HOST`
    /// supplied: setting it bypasses the sidecar and sends the full prompt to an
    /// unmanaged endpoint with **no auth**, so anything but loopback is Public.
    #[test]
    fn tier_is_private_only_while_inference_stays_on_this_machine() {
        assert_eq!(
            provider_with_external_base(None).tier(),
            ProviderTier::Private,
            "the bundled sidecar is the default install and runs here"
        );
        assert_eq!(
            LlamaCppProvider::metadata().tier,
            ProviderTier::Private,
            "so the type-level claim agrees with it"
        );

        for loopback in [
            "http://localhost:11543/",
            "http://127.0.0.1:11543/",
            "http://[::1]:11543/",
        ] {
            assert_eq!(
                provider_with_external_base(Some(loopback)).tier(),
                ProviderTier::Private,
                "{loopback}"
            );
        }
        for remote in [
            "http://gpu.lab.ucsf.edu:11543/",
            "https://api.example-saas.com/",
        ] {
            assert_eq!(
                provider_with_external_base(Some(remote)).tier(),
                ProviderTier::Public,
                "{remote}"
            );
        }
    }

    /// DR-26 (Task 46), **wired** — all three arms, following the same branch
    /// as the tier above rather than a new rule.
    ///
    /// The remote arm is the one that matters: `Local` is the *most* permissive
    /// affiliation in DR-26's model, reaching every private extension because no
    /// transfer occurs at all. An implementation that answered `Local` for any
    /// `llamacpp` — the obvious one, since the provider is "the local models
    /// one" — would hand that blanket permission to an unmanaged lab box the
    /// user pointed `LLAMACPP_EXTERNAL_HOST` at, with no auth in between.
    #[test]
    fn affiliation_is_local_only_while_inference_stays_on_this_machine() {
        use crate::privacy::affiliation::ModelAffiliation;

        assert_eq!(
            provider_with_external_base(None).affiliation(),
            Some(ModelAffiliation::Local),
            "the bundled sidecar is the default install and runs here"
        );

        for loopback in [
            "http://localhost:11543/",
            "http://127.0.0.1:11543/",
            "http://[::1]:11543/",
        ] {
            assert_eq!(
                provider_with_external_base(Some(loopback)).affiliation(),
                Some(ModelAffiliation::Local),
                "{loopback}"
            );
        }
        for remote in [
            "http://gpu.lab.ucsf.edu:11543/",
            "https://api.example-saas.com/",
        ] {
            assert_eq!(
                provider_with_external_base(Some(remote)).affiliation(),
                None,
                "{remote} must not inherit Local's blanket permission"
            );
        }
    }

    /// The product decision: the default is Qwen3.8 27B on every memory tier.
    /// A machine below the catalog minimum gets the same answer plus a
    /// below-minimum warning (in the route), never a different model.
    #[test]
    fn the_default_is_qwen38_on_every_memory_tier() {
        for gib in [0, 8, 16, 24, 31, 32, 47, 48, 64, 128, 512] {
            assert_eq!(
                recommended_model_for_memory_gib(gib),
                "qwen3.8-27b",
                "{gib}"
            );
        }
        assert_eq!(default_model_name(), DEFAULT_MODEL);
        let entry = catalog_entry(DEFAULT_MODEL).expect("the default is in the catalog");
        assert!(entry.context_limit >= llamacpp_sidecar::LLAMACPP_AGENT_CONTEXT_SIZE);
        assert_eq!(
            MODEL_CATALOG[0].name, DEFAULT_MODEL,
            "the default is listed first"
        );
    }

    #[test]
    fn catalog_descriptions_do_not_describe_a_tier() {
        // Ruling 26 (FR-R3-6): the Llama Server panel's Memory row states the fit in GB, so a
        // catalog description must not describe the machine as a tier ("above the laptop
        // tier" half contradicted that row on a large machine). The gemma4-12b entry it was
        // first written for is retired; the rule holds for every entry still listed.
        for entry in MODEL_CATALOG {
            assert!(
                !entry.description.to_lowercase().contains("tier"),
                "{}'s description should state memory, not a tier: {}",
                entry.name,
                entry.description
            );
        }
    }

    #[test]
    fn the_catalog_is_exactly_the_two_supported_models() {
        let names: Vec<_> = MODEL_CATALOG.iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["qwen3.8-27b", "gemma4-31b"]);
        let display: Vec<_> = MODEL_CATALOG.iter().map(|e| e.display_name).collect();
        assert_eq!(display, vec!["Qwen3.8 27B", "Gemma 4 31B"]);

        let qwen = catalog_entry("qwen3.8-27b").unwrap();
        assert_eq!(qwen.family, "Qwen3.8");
        assert_eq!(qwen.ollama_name, Some("qwen3.8:27b"));
        assert_eq!(qwen.ollama_gguf_names, &["qwen3.8:27b-q4_K_M"]);
        // The llama.cpp project's own conversion of the official weights.
        assert_eq!(qwen.hf_spec, "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M");
        assert_eq!(qwen.gguf_file, "Qwen3.8-27B-Q4_K_M.gguf");
        assert_eq!(qwen.gguf_size_bytes, 18_973_870_528);
        assert_eq!(qwen.thinking, ThinkingControl::QwenReasoningEffort);

        let gemma = catalog_entry("gemma4-31b").unwrap();
        assert_eq!(gemma.family, "Gemma 4");
        assert_eq!(gemma.ollama_name, Some("gemma4:31b"));
        assert_eq!(gemma.ollama_gguf_names, &["gemma4:31b-it-q4_K_M"]);
        // Google's own quantization-aware-trained GGUF.
        assert_eq!(gemma.hf_spec, "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0");
        assert_eq!(gemma.gguf_file, "gemma-4-31B_q4_0-it.gguf");
        assert_eq!(gemma.gguf_size_bytes, 17_651_001_568);
        assert_eq!(gemma.thinking, ThinkingControl::Toggle);

        for entry in MODEL_CATALOG {
            // Bare manifest-list tags only: `:latest` is a different model
            // (gemma4:latest is E4B) and `-nvfp4`/`-mlx` never run outside
            // Apple Silicon.
            let tag = entry.ollama_name.unwrap();
            for wrong in [":latest", "nvfp4", "mlx"] {
                assert!(!tag.contains(wrong), "{tag}");
            }
            assert_eq!(entry.min_gpu_memory_gib, 32, "{}", entry.name);
            assert_eq!(entry.recommended_gpu_memory_gib, 48, "{}", entry.name);
            assert_eq!(entry.context_limit, 262_144, "{}", entry.name);
            assert!(
                entry
                    .official_url
                    .starts_with("https://ollama.com/library/"),
                "{}",
                entry.official_url
            );
            assert_eq!(entry.gguf_sha256.len(), 64, "{}", entry.name);
            assert!(entry.gguf_sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(gguf_name_is_the_specs_quant(entry), "{}", entry.name);
            assert!(!entry.speed_hint.trim().is_empty(), "{}", entry.name);
            assert!(entry.active_params_b > 0, "{}", entry.name);
        }
    }

    /// The pinned file name is what llama.cpp's `TAG[.-]` match picks for the
    /// spec's quant, so the expected size describes the file `-hf` fetches.
    fn gguf_name_is_the_specs_quant(entry: &CatalogEntry) -> bool {
        let quant = entry
            .hf_spec
            .split_once(':')
            .unwrap()
            .1
            .to_ascii_lowercase();
        let file = entry.gguf_file.to_ascii_lowercase();
        file.match_indices(&quant)
            .any(|(at, _)| matches!(file.as_bytes().get(at + quant.len()), Some(b'.' | b'-')))
    }

    #[test]
    fn metadata_uses_catalog_context_limits() {
        let metadata = LlamaCppProvider::metadata();
        let names: Vec<_> = metadata
            .known_models
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["qwen3.8-27b", "gemma4-31b"]);
        for info in &metadata.known_models {
            assert_eq!(info.context_limit, 262_144, "{}", info.name);
        }
    }

    /// LC-11. A config, session, workflow or schedule that still names a
    /// retired catalog model runs the default instead of failing every request
    /// with "Unknown Llama Server model".
    #[test]
    fn a_retired_name_runs_the_default_for_chat() {
        for retired in RETIRED_MODELS {
            assert!(catalog_entry(retired.name).is_none(), "{}", retired.name);
            assert_eq!(
                chat_model_name(retired.name),
                DEFAULT_MODEL,
                "{}",
                retired.name
            );
            let (name, source) = resolve_chat_model_source(retired.name).unwrap();
            assert_eq!(name, DEFAULT_MODEL);
            assert_eq!(source.hf_spec, "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M");
        }
        for kept in [
            "qwen3.8-27b",
            "gemma4-31b",
            "bartowski/some-model-GGUF:Q8_0",
        ] {
            assert_eq!(chat_model_name(kept), kept);
        }
        for listed in [
            "gemma4",
            "gemma4-e2b",
            "gemma4-12b",
            "gemma4-26b",
            "qwen3.6",
            "qwen3.6-27b",
        ] {
            assert!(
                retired_model(listed).is_some(),
                "{listed} must stay resolvable"
            );
        }
    }

    /// LC-11, the other half. Deleting a retired model's cache must delete THAT
    /// cache: aliasing here would make "delete gemma4" remove the 19 GB Qwen3.8
    /// file instead of the orphaned E4B one.
    #[test]
    fn a_retired_name_keeps_its_own_spec_for_deletion() {
        let gemma4 = resolve_model_source("gemma4").unwrap();
        assert_eq!(gemma4.hf_spec, "google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0");
        assert_eq!(gemma4.ollama_name.as_deref(), Some("gemma4:latest"));
        assert_eq!(
            resolve_hf_spec("qwen3.6-27b").unwrap(),
            "unsloth/Qwen3.6-27B-GGUF:Q4_K_M"
        );
        let catalog_specs: Vec<_> = MODEL_CATALOG.iter().map(|e| e.hf_spec).collect();
        for retired in RETIRED_MODELS {
            assert!(
                !catalog_specs.contains(&retired.hf_spec),
                "{} would delete a current model's cache",
                retired.name
            );
            assert_eq!(resolve_hf_spec(retired.name).unwrap(), retired.hf_spec);
        }
    }

    #[test]
    fn resolve_catalog_name() {
        let qwen = resolve_model_source("qwen3.8-27b").unwrap();
        assert_eq!(qwen.hf_spec, "ggml-org/Qwen3.8-27B-GGUF:Q4_K_M");
        assert_eq!(qwen.ollama_name.as_deref(), Some("qwen3.8:27b"));
        assert_eq!(qwen.ollama_gguf_names, vec!["qwen3.8:27b-q4_K_M"]);
        let expected = qwen.expected_file.unwrap();
        assert_eq!(expected.file_name, "Qwen3.8-27B-Q4_K_M.gguf");
        assert_eq!(expected.size_bytes, 18_973_870_528);
        assert_eq!(qwen.spec_type.as_deref(), SPEC_QWEN38);

        let gemma = resolve_model_source("gemma4-31b").unwrap();
        assert_eq!(gemma.hf_spec, "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0");
        assert_eq!(gemma.spec_type.as_deref(), SPEC_GEMMA4);
    }

    #[test]
    fn resolve_raw_hf_spec_passthrough() {
        assert_eq!(
            resolve_hf_spec("bartowski/some-model-GGUF:Q8_0").unwrap(),
            "bartowski/some-model-GGUF:Q8_0"
        );
    }

    #[test]
    fn resolve_unknown_name_errors_with_catalog_hint() {
        let err = resolve_hf_spec("gpt-4o").unwrap_err().to_string();
        assert!(err.contains("qwen3.8-27b"), "{err}");
        assert!(err.contains("gemma4-31b"), "{err}");
    }

    #[test]
    fn reasoning_effort_maps_onto_thinking() {
        for default_on in [false, true] {
            assert_eq!(
                thinking_for(Some(ReasoningEffort::Quick), default_on),
                Thinking::Off
            );
            assert_eq!(
                thinking_for(Some(ReasoningEffort::Deep), default_on),
                Thinking::On { deep: true }
            );
            let normal = if default_on {
                Thinking::On { deep: false }
            } else {
                Thinking::Off
            };
            assert_eq!(
                thinking_for(Some(ReasoningEffort::Normal), default_on),
                normal
            );
            assert_eq!(thinking_for(None, default_on), normal);
        }
    }

    fn payload() -> Value {
        json!({
            "model": "qwen3.6",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
        })
    }

    #[test]
    fn request_policy_without_thinking_uses_the_instruct_sampling() {
        let mut p = payload();
        apply_request_policy(&mut p, "qwen3.8-27b", Thinking::Off);
        assert_eq!(
            p["model"], "qwen3.8-27b",
            "the served alias, not a retired name"
        );
        assert_eq!(p["chat_template_kwargs"], json!({"enable_thinking": false}));
        assert_eq!(p["temperature"].as_f64().unwrap() as f32, 0.7);
        assert_eq!(p["top_p"].as_f64().unwrap() as f32, 0.8);
        assert_eq!(p["top_k"], 20);
        assert_eq!(p["min_p"].as_f64().unwrap(), 0.0);
        assert_eq!(p["presence_penalty"].as_f64().unwrap() as f32, 1.5);
        assert_eq!(p["max_tokens"], LLAMACPP_DEFAULT_MAX_TOKENS);
    }

    /// Qwen3.8's template raises on any `reasoning_effort` other than low,
    /// medium or xhigh — OpenAI's `high` included — so Deep asks for `xhigh`
    /// and a top-level `reasoning_effort` never reaches the template.
    #[test]
    fn deep_thinking_on_qwen38_asks_for_xhigh_and_never_high() {
        let mut p = payload();
        p["reasoning_effort"] = json!("high");
        apply_request_policy(&mut p, "qwen3.8-27b", Thinking::On { deep: true });
        assert_eq!(
            p["chat_template_kwargs"],
            json!({"enable_thinking": true, "reasoning_effort": "xhigh"})
        );
        assert!(p.get("reasoning_effort").is_none(), "{p}");
        assert_eq!(p["temperature"].as_f64().unwrap() as f32, 1.0);
        assert_eq!(p["top_p"].as_f64().unwrap() as f32, 0.95);
        assert_eq!(p["presence_penalty"].as_f64().unwrap(), 0.0);
        assert_eq!(p["max_tokens"], LLAMACPP_THINKING_MAX_TOKENS);

        // Normal states its level: the template's own default is `xhigh`, so a
        // Normal turn that sent none reasoned exactly as long as a Deep one.
        let mut normal = payload();
        apply_request_policy(&mut normal, "qwen3.8-27b", Thinking::On { deep: false });
        assert_eq!(
            normal["chat_template_kwargs"],
            json!({"enable_thinking": true, "reasoning_effort": "medium"}),
        );
        assert_ne!(
            normal["chat_template_kwargs"], p["chat_template_kwargs"],
            "Normal and Deep must reach the template differently"
        );

        // Off says nothing about a level.
        let mut off = payload();
        apply_request_policy(&mut off, "qwen3.8-27b", Thinking::Off);
        assert_eq!(
            off["chat_template_kwargs"],
            json!({"enable_thinking": false})
        );
    }

    #[test]
    fn gemma4_takes_a_plain_toggle_and_its_own_sampling() {
        let mut p = payload();
        apply_request_policy(&mut p, "gemma4-31b", Thinking::On { deep: true });
        assert_eq!(p["chat_template_kwargs"], json!({"enable_thinking": true}));
        assert_eq!(p["top_k"], 64);
        assert_eq!(p["temperature"].as_f64().unwrap() as f32, 1.0);
        assert_eq!(p["presence_penalty"].as_f64().unwrap(), 0.0);

        let mut off = payload();
        apply_request_policy(&mut off, "gemma4-31b", Thinking::Off);
        assert_eq!(
            off["chat_template_kwargs"],
            json!({"enable_thinking": false})
        );
        assert_eq!(off["top_k"], 64);
    }

    /// A value the caller set — `BIOROUTER_TEMPERATURE`, Quick's 0.0, the
    /// warm-up's 0.0 and its small `max_tokens`, a title's 64 — always wins.
    #[test]
    fn request_policy_never_overrides_a_pinned_value() {
        let mut p = payload();
        p["temperature"] = json!(0.0);
        p["max_tokens"] = json!(64);
        p["chat_template_kwargs"] = json!({"enable_thinking": true, "preserve_thinking": false});
        apply_request_policy(&mut p, "qwen3.8-27b", Thinking::Off);
        assert_eq!(p["temperature"], 0.0);
        assert_eq!(p["max_tokens"], 64);
        assert_eq!(
            p["chat_template_kwargs"],
            json!({"enable_thinking": true, "preserve_thinking": false})
        );
        // Unpinned fields are still filled.
        assert_eq!(p["top_k"], 20);
    }

    /// LC-8: a streamed request asks for prefill progress so a long cold
    /// prefill keeps the idle timeout fed; a value already set is kept.
    #[test]
    fn a_stream_asks_for_prefill_progress() {
        let mut p = payload();
        request_prefill_progress(&mut p);
        assert_eq!(p["return_progress"], true);

        let mut pinned = payload();
        pinned["return_progress"] = json!(false);
        request_prefill_progress(&mut pinned);
        assert_eq!(pinned["return_progress"], false);
    }

    /// The frames `return_progress` adds, exactly as b11429 streamed them for a
    /// 1,939-token prompt (ids and timestamps dropped), decode to nothing: the
    /// only items are the answer and the usage. On b11429 a progress frame is
    /// NOT `choices: []` — it carries the same empty role delta as an ordinary
    /// first chunk — so this pins the shape the provider opts into.
    #[tokio::test]
    async fn b11429_progress_frames_decode_to_nothing() {
        let frames = [
            r#"data: {"choices":[{"finish_reason":null,"index":0,"delta":{"role":"assistant","content":null}}],"model":"qwen3.8-27b","object":"chat.completion.chunk","prompt_progress":{"total":1939,"cache":0,"processed":0,"time_ms":0}}"#,
            r#"data: {"choices":[{"finish_reason":null,"index":0,"delta":{"role":"assistant","content":null}}],"model":"qwen3.8-27b","object":"chat.completion.chunk","prompt_progress":{"total":1939,"cache":0,"processed":1423,"time_ms":4291}}"#,
            r#"data: {"choices":[{"finish_reason":null,"index":0,"delta":{"role":"assistant","content":null}}],"model":"qwen3.8-27b","object":"chat.completion.chunk","prompt_progress":{"total":1939,"cache":0,"processed":1939,"time_ms":8550}}"#,
            r#"data: {"choices":[{"finish_reason":null,"index":0,"delta":{"role":"assistant","content":null}}],"model":"qwen3.8-27b","object":"chat.completion.chunk"}"#,
            r#"data: {"choices":[{"finish_reason":null,"index":0,"delta":{"content":"Batch"}}],"model":"qwen3.8-27b","object":"chat.completion.chunk"}"#,
            r#"data: {"choices":[{"finish_reason":"stop","index":0,"delta":{}}],"model":"qwen3.8-27b","object":"chat.completion.chunk"}"#,
            r#"data: {"choices":[],"model":"qwen3.8-27b","object":"chat.completion.chunk","usage":{"completion_tokens":2,"prompt_tokens":1939,"total_tokens":1941,"prompt_tokens_details":{"cached_tokens":0}}}"#,
            "data: [DONE]",
        ];
        let lines: Vec<anyhow::Result<String>> =
            frames.iter().map(|line| Ok(line.to_string())).collect();
        let decoded: Vec<_> = crate::providers::formats::openai::response_to_streaming_message(
            futures::stream::iter(lines),
        )
        .collect()
        .await;
        let texts: Vec<String> = decoded
            .iter()
            .filter_map(|item| item.as_ref().unwrap().0.as_ref())
            .map(Message::as_concat_text)
            .collect();
        assert_eq!(texts, vec!["Batch"], "{decoded:?}");
        let usage = decoded
            .iter()
            .find_map(|item| item.as_ref().unwrap().1.as_ref())
            .expect("the usage chunk still arrives");
        assert_eq!(usage.usage.input_tokens, Some(1939));
    }

    #[test]
    fn a_raw_spec_gets_thinking_control_and_a_ceiling_but_no_vendor_sampling() {
        let mut p = payload();
        apply_request_policy(&mut p, "bartowski/some-model-GGUF:Q8_0", Thinking::Off);
        assert_eq!(p["model"], "bartowski/some-model-GGUF:Q8_0");
        assert_eq!(p["chat_template_kwargs"], json!({"enable_thinking": false}));
        assert_eq!(p["max_tokens"], LLAMACPP_DEFAULT_MAX_TOKENS);
        for key in ["temperature", "top_p", "top_k", "min_p", "presence_penalty"] {
            assert!(p.get(key).is_none(), "{key} in {p}");
        }
    }

    /// LC-4. Before the first request, compaction must not trust the native
    /// 262k window against a 64k server.
    #[test]
    fn the_reported_window_is_clamped_to_the_servers() {
        assert_eq!(clamp_to_server_window(Some(262_144), 65_536), 65_536);
        assert_eq!(clamp_to_server_window(Some(262_144), 131_072), 131_072);
        assert_eq!(clamp_to_server_window(Some(32_768), 65_536), 32_768);
        assert_eq!(clamp_to_server_window(None, 131_072), 131_072);
    }

    fn text_item(text: &str) -> Result<crate::providers::base::ProviderStreamItem, ProviderError> {
        Ok((Some(Message::assistant().with_text(text)), None, None))
    }

    fn tool_item() -> Result<crate::providers::base::ProviderStreamItem, ProviderError> {
        Ok((
            Some(Message::assistant().with_tool_request(
                "call_1",
                Ok(rmcp::model::CallToolRequestParams {
                    meta: None,
                    task: None,
                    name: "developer__shell".into(),
                    arguments: None,
                }),
            )),
            None,
            None,
        ))
    }

    async fn guarded(
        items: Vec<Result<crate::providers::base::ProviderStreamItem, ProviderError>>,
    ) -> (MessageStream, Arc<tokio::sync::RwLock<()>>) {
        let lock = Arc::new(tokio::sync::RwLock::new(()));
        let guard = lock.clone().read_owned().await;
        let stream: MessageStream = Box::pin(futures::stream::iter(items));
        (hold_lease_while_generating(stream, Some(guard)), lock)
    }

    /// LC-10, and the deadlock the lease would otherwise cause: the agent runs
    /// tool calls before polling the stream again, so the lease must be gone
    /// the moment the tool-call message arrives.
    #[tokio::test]
    async fn the_lease_is_released_when_generation_ends() {
        let (mut stream, lock) = guarded(vec![
            text_item("Let me check."),
            tool_item(),
            text_item("tail"),
        ])
        .await;
        stream.next().await.unwrap().unwrap();
        assert!(lock.try_write().is_err(), "held while text is streaming");
        stream.next().await.unwrap().unwrap();
        assert!(
            lock.try_write().is_ok(),
            "released once the tool call arrived, with the stream still alive"
        );

        let (mut stream, lock) = guarded(vec![text_item("done")]).await;
        stream.next().await.unwrap().unwrap();
        assert!(lock.try_write().is_err());
        assert!(stream.next().await.is_none());
        assert!(
            lock.try_write().is_ok(),
            "released at the end of a text reply"
        );

        let (mut stream, lock) = guarded(vec![
            text_item("partial"),
            Err(ProviderError::RequestFailed("connection reset".into())),
        ])
        .await;
        stream.next().await.unwrap().unwrap();
        assert!(stream.next().await.unwrap().is_err());
        assert!(lock.try_write().is_ok(), "released on an error");

        // The usage chunk comes after `finish_reason`: generation is over, so
        // the slot is free for the next request before `[DONE]` is read.
        let usage = Ok((
            None,
            Some(ProviderUsage::new(
                "qwen3.8-27b".to_string(),
                Usage::new(Some(10), Some(2), Some(12)),
            )),
            None,
        ));
        let (mut stream, lock) = guarded(vec![text_item("done"), usage, text_item("late")]).await;
        stream.next().await.unwrap().unwrap();
        assert!(lock.try_write().is_err());
        stream.next().await.unwrap().unwrap();
        assert!(
            lock.try_write().is_ok(),
            "released at the usage chunk, with the stream still alive"
        );
    }

    #[tokio::test]
    async fn recovery_does_not_hold_a_finished_generation_or_discard_on_late_errors() {
        for terminal in [
            tool_item(),
            Ok((
                None,
                Some(ProviderUsage::new(
                    "qwen3.8-27b".to_string(),
                    Usage::new(Some(10), Some(2), Some(12)),
                )),
                None,
            )),
        ] {
            let lock = Arc::new(tokio::sync::RwLock::new(()));
            let guard = lock.clone().read_owned().await;
            let recovered = Arc::new(AtomicUsize::new(0));
            let count = recovered.clone();
            let items = vec![
                text_item("partial"),
                terminal,
                Err(ProviderError::ServerError("Compute error.".into())),
            ];
            let mut stream = hold_lease_while_generating_with_recovery(
                Box::pin(futures::stream::iter(items)),
                Some(guard),
                move |error, _guard| {
                    count.fetch_add(1, Ordering::Relaxed);
                    async move { error }
                },
            );
            stream.next().await.unwrap().unwrap();
            assert!(lock.try_write().is_err());
            stream.next().await.unwrap().unwrap();
            assert!(lock.try_write().is_ok());
            assert!(stream.next().await.unwrap().is_err());
            assert_eq!(recovered.load(Ordering::Relaxed), 0);
        }
    }

    async fn http_error_stream(error: Value) -> (wiremock::MockServer, MessageStream) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let body = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({
                "choices": [{"finish_reason": null, "index": 0, "delta": {"role": "assistant", "content": null}}],
                "model": "qwen3.8-27b",
                "prompt_progress": {"total": 31000, "processed": 4096}
            }),
            json!({"error": error})
        );
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/events"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let response = reqwest::get(format!("{}/events", server.uri()))
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let log = RequestLog::start(
            &ModelConfig::new_or_fail("qwen3.8-27b"),
            &json!({"model": "qwen3.8-27b"}),
        )
        .unwrap();
        let stream = stream_openai_compat_showing_reasoning(response, log).unwrap();
        (server, stream)
    }

    #[cfg(unix)]
    fn fake_sidecar_binary(root: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let binary = root.join("llama-server");
        std::fs::write(&binary, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        binary
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn streamed_compute_failure_discards_the_backend_before_releasing_its_lease() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let binary = fake_sidecar_binary(tmp.path());
        let _env = crate::test_sandbox::relocate_path_root_and(
            tmp.path().to_str().unwrap(),
            [
                ("BIOROUTER_LLAMACPP_BIN", binary.to_str()),
                ("OLLAMA_MODELS", Some(tmp.path().to_str().unwrap())),
                ("LLAMACPP_HF_TOKEN", Some("")),
                ("LLAMACPP_EXTRA_ARGS", None),
                ("LLAMACPP_CONTEXT_SIZE", None),
            ],
        );
        let sidecar = llamacpp_sidecar::global();
        let source = ModelSource::huggingface("example/model:Q4_K_M");
        let port = sidecar.ensure("test-model", &source).await.unwrap();
        let old_key = llamacpp_sidecar::current_api_key().unwrap();
        let (_server, decoded) = http_error_stream(json!({
            "code": 500,
            "type": "server_error",
            "message": "Compute error.",
            "details": "SECRET_PROVIDER_PAYLOAD"
        }))
        .await;
        let lock = Arc::new(tokio::sync::RwLock::new(()));
        let guard = (port, lock.clone().read_owned().await);
        let held = lock.clone();
        let mut stream =
            hold_lease_while_generating_with_recovery(decoded, Some(guard), move |error, lease| {
                assert!(
                    held.try_write().is_err(),
                    "cleanup runs while the lease is held"
                );
                async move { after_failure(error, Some(lease.0)).await }
            });
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(
            &error,
            ProviderError::ServerError(message) if message.contains("GPU backend failed")
        ));
        assert!(!error.to_string().contains("SECRET_PROVIDER_PAYLOAD"));
        assert!(
            lock.try_write().is_ok(),
            "released before reporting the failure"
        );
        assert!(llamacpp_sidecar::current_api_key().is_none());
        assert!(sidecar.discard_failed_backend(port).await.is_none());

        sidecar.ensure("test-model", &source).await.unwrap();
        assert_ne!(llamacpp_sidecar::current_api_key().unwrap(), old_key);
        sidecar.stop().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unrelated_and_external_stream_errors_preserve_the_managed_backend() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let binary = fake_sidecar_binary(tmp.path());
        let _env = crate::test_sandbox::relocate_path_root_and(
            tmp.path().to_str().unwrap(),
            [
                ("BIOROUTER_LLAMACPP_BIN", binary.to_str()),
                ("OLLAMA_MODELS", Some(tmp.path().to_str().unwrap())),
                ("LLAMACPP_HF_TOKEN", Some("")),
                ("LLAMACPP_EXTRA_ARGS", None),
                ("LLAMACPP_CONTEXT_SIZE", None),
            ],
        );
        let sidecar = llamacpp_sidecar::global();
        let source = ModelSource::huggingface("example/model:Q4_K_M");
        let port = sidecar.ensure("test-model", &source).await.unwrap();
        let key = llamacpp_sidecar::current_api_key().unwrap();
        let (_server, decoded) = http_error_stream(json!({
            "code": 500,
            "type": "server_error",
            "message": "SECRET_PROVIDER_PAYLOAD"
        }))
        .await;
        let mut stream = hold_lease_while_generating_with_recovery(
            decoded,
            Some(port),
            |error, port| async move { after_failure(error, Some(*port)).await },
        );
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(&error, ProviderError::RequestFailed(_)));
        assert!(!error.to_string().contains("SECRET_PROVIDER_PAYLOAD"));
        assert_eq!(
            llamacpp_sidecar::current_api_key().as_deref(),
            Some(key.as_str())
        );

        let (_server, decoded) = http_error_stream(json!({
            "code": 500,
            "type": "server_error",
            "message": "Compute error."
        }))
        .await;
        let mut stream = hold_lease_while_generating_with_recovery(
            decoded,
            None::<u16>,
            |error, port| async move { after_failure(error, Some(*port)).await },
        );
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(ProviderError::ServerError(message)) if message == "Compute error."
        ));
        assert_eq!(
            llamacpp_sidecar::current_api_key().as_deref(),
            Some(key.as_str())
        );
        sidecar.stop().await;
    }

    /// A loopback listener whose port number contains no HTTP status code.
    /// `errors::classify_provider_details` sniffs status codes out of the WHOLE
    /// message, URL included, so a timeout from a mock on port 50451 read as a
    /// 504 server error — retried, not handed back — and failed this test
    /// intermittently (twice in about fifteen runs).
    fn listener_without_a_status_code_in_its_port() -> std::net::TcpListener {
        const CODES: [&str; 10] = [
            "400", "401", "403", "404", "422", "429", "500", "502", "503", "504",
        ];
        loop {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port().to_string();
            if !CODES.iter().any(|code| port.contains(code)) {
                return listener;
            }
        }
    }

    /// A timeout is handed straight back on both request paths, not re-sent
    /// three times to a server still busy with the first copy. Under
    /// `TimeoutRetry::Retry` this saw eight requests for the two calls.
    #[tokio::test]
    async fn a_timeout_is_never_resent() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::builder()
            .listener(listener_without_a_status_code_in_its_port())
            .start()
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(30))
                    .set_body_json(json!({})),
            )
            .mount(&server)
            .await;
        // `request_timeout` (1 s, below) IS the idle bound: an IdleOnly client
        // has no 300 s floor. The lock only pins it against an operator's
        // BIOROUTER_HTTP_READ_TIMEOUT_SECS, which is read when the client is
        // built (on the first call, then cached).
        let _env = env_lock::lock_env([("BIOROUTER_HTTP_READ_TIMEOUT_SECS", Some("1"))]);
        let mut provider = provider_with_external_base(Some(&format!("{}/", server.uri())));
        provider.request_timeout = Duration::from_secs(1);
        let messages = [Message::user().with_text("hi")];

        let started = std::time::Instant::now();
        let error = provider
            .complete_with_model(&provider.get_model_config(), "system", &messages, &[])
            .await
            .expect_err("the server never answers in time");
        assert!(super::super::retry::is_timeout(&error), "{error:?}");
        let error = provider
            .stream("system", &messages, &[])
            .await
            .err()
            .expect("the stream never starts in time");
        assert!(super::super::retry::is_timeout(&error), "{error:?}");

        let sent = server.received_requests().await.unwrap().len();
        assert_eq!(sent, 2, "one request per call, never a resend");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no backoff either: {:?}",
            started.elapsed()
        );
    }

    /// Thinking is SHOWN, as one block before the answer, through the
    /// provider's own stream path. The body is b11429 serving Qwen3.8 27B with
    /// `enable_thinking` on (recorded 2026-10-10; ids, timestamps and the
    /// fingerprint dropped, and 37 of the 45 reasoning deltas elided). The
    /// capture-only decoder the provider used yields only "No." from it — a
    /// Deep turn's `xhigh` thinking never reached the chat at all.
    #[tokio::test]
    async fn a_thinking_stream_shows_its_reasoning() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let delta = |delta: &str| {
            format!(
                r#"data: {{"choices":[{{"finish_reason":null,"index":0,"delta":{delta}}}],"model":"qwen3.8-27b","object":"chat.completion.chunk"}}"#
            )
        };
        let mut frames = vec![delta(r#"{"role":"assistant","content":null}"#)];
        for token in [
            "The", " user", " is", " asking", " if", " ", "9", "1", " not", " prime", ".", "\\n",
        ] {
            frames.push(delta(&format!(r#"{{"reasoning_content":"{token}"}}"#)));
        }
        for token in ["No", "."] {
            frames.push(delta(&format!(r#"{{"content":"{token}"}}"#)));
        }
        frames.push(r#"data: {"choices":[{"finish_reason":"stop","index":0,"delta":{}}],"model":"qwen3.8-27b","object":"chat.completion.chunk"}"#.to_string());
        frames.push(r#"data: {"choices":[],"model":"qwen3.8-27b","object":"chat.completion.chunk","usage":{"completion_tokens":50,"prompt_tokens":51,"total_tokens":101,"prompt_tokens_details":{"cached_tokens":0}}}"#.to_string());
        frames.push("data: [DONE]".to_string());
        let body = frames.join("\n\n") + "\n\n";

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let provider = provider_with_external_base(Some(&format!("{}/", server.uri())));
        let mut stream = provider
            .stream("system", &[Message::user().with_text("Is 91 prime?")], &[])
            .await
            .unwrap();
        let mut messages = Vec::new();
        while let Some(item) = stream.next().await {
            if let (Some(message), _, _) = item.unwrap() {
                messages.push(message);
            }
        }
        let thinking: Vec<_> = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::Thinking(t) => Some(t.thinking.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            thinking,
            vec!["The user is asking if 91 not prime.\n"],
            "{messages:?}"
        );
        let answer: String = messages.iter().map(Message::as_concat_text).collect();
        assert_eq!(answer, "No.");
    }

    /// A call the model wrote as the whole reply's TEXT — the shape
    /// qwen2.5-coder:7b gave on every call through Ollama (protocol matrix L1)
    /// — is recovered on this provider too, under the same guards, on both
    /// paths: no JSON reaches the chat, and the call carries the recovered
    /// marker. A reply that merely is JSON stays text.
    #[tokio::test]
    async fn a_call_written_as_the_whole_reply_is_recovered_on_both_paths() {
        use crate::providers::formats::text_tool_calls::is_recovered;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let shell = Tool::new(
            "developer__shell".to_string(),
            "run a command".to_string(),
            rmcp::model::object(json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            })),
        );
        let leak = r#"{"name": "developer__shell", "arguments": {"command": "ls /tmp"}}"#;
        let delta = |content: &str| {
            format!(
                r#"data: {{"choices":[{{"finish_reason":null,"index":0,"delta":{{"content":{}}}}}],"model":"m","object":"chat.completion.chunk"}}"#,
                serde_json::to_string(content).unwrap()
            )
        };
        let mut frames: Vec<String> = leak
            .as_bytes()
            .chunks(6)
            .map(|chunk| delta(std::str::from_utf8(chunk).unwrap()))
            .collect();
        frames.push(r#"data: {"choices":[{"finish_reason":"stop","index":0,"delta":{}}],"model":"m","object":"chat.completion.chunk"}"#.to_string());
        frames.push(r#"data: {"choices":[],"model":"m","object":"chat.completion.chunk","usage":{"completion_tokens":20,"prompt_tokens":400,"total_tokens":420}}"#.to_string());
        frames.push("data: [DONE]".to_string());
        let sse = frames.join("\n\n") + "\n\n";

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "m",
                "choices": [{"index": 0, "finish_reason": "stop",
                             "message": {"role": "assistant", "content": leak}}],
                "usage": {"prompt_tokens": 400, "completion_tokens": 20, "total_tokens": 420}
            })))
            .mount(&server)
            .await;
        let provider = provider_with_external_base(Some(&format!("{}/", server.uri())));
        let messages = [Message::user().with_text("list /tmp")];

        let mut stream = provider
            .stream("system", &messages, std::slice::from_ref(&shell))
            .await
            .unwrap();
        let mut streamed = Vec::new();
        while let Some(item) = stream.next().await {
            if let (Some(message), _, _) = item.unwrap() {
                streamed.push(message);
            }
        }
        let text: String = streamed.iter().map(Message::as_concat_text).collect();
        assert!(text.is_empty(), "the JSON never reaches the chat: {text}");
        let calls: Vec<_> = streamed
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(MessageContent::as_tool_request)
            .collect();
        assert_eq!(calls.len(), 1, "{streamed:?}");
        assert!(is_recovered(calls[0]));
        assert_eq!(
            calls[0]
                .tool_call
                .as_ref()
                .unwrap()
                .arguments
                .as_ref()
                .unwrap()["command"],
            "ls /tmp"
        );

        let (message, _) = provider
            .complete_with_model(
                &provider.get_model_config(),
                "system",
                &messages,
                std::slice::from_ref(&shell),
            )
            .await
            .unwrap();
        assert!(message.as_concat_text().is_empty());
        let request = message
            .content
            .iter()
            .find_map(MessageContent::as_tool_request)
            .expect("the blocking path recovers the call too");
        assert!(is_recovered(request));
    }

    /// `LLAMACPP_TIMEOUT` declares `0` (= this machine's tier), never `600`.
    /// The settings form saves a key's declared default on every save, which
    /// wrote 600 over the CPU-only 3600 s bound (LC-9).
    #[test]
    fn the_timeout_default_is_the_machines_tier() {
        let meta = LlamaCppProvider::metadata();
        let key = meta
            .config_keys
            .iter()
            .find(|k| k.name == "LLAMACPP_TIMEOUT")
            .unwrap();
        assert_eq!(key.default.as_deref(), Some("0"));

        assert_eq!(request_timeout_secs(None, false), LLAMACPP_TIMEOUT);
        assert_eq!(request_timeout_secs(None, true), LLAMACPP_CPU_ONLY_TIMEOUT);
        assert_eq!(request_timeout_secs(Some(0), false), LLAMACPP_TIMEOUT);
        assert_eq!(
            request_timeout_secs(Some(0), true),
            LLAMACPP_CPU_ONLY_TIMEOUT
        );
        // The user's own number wins, on either kind of machine.
        assert_eq!(request_timeout_secs(Some(1200), true), 1200);
        assert_eq!(request_timeout_secs(Some(30), false), 30);
    }

    /// A retired catalog name is the sidecar's alias for the default model,
    /// and nobody else's: an external host's `gemma4` is that server's own
    /// model (a real Ollama tag; llama-swap and router-mode llama-server route
    /// on it) and is sent as written.
    #[test]
    fn a_retired_name_is_only_remapped_on_the_managed_sidecar() {
        let config = ModelConfig::new_or_fail("gemma4").with_context_limit(Some(131_072));
        let external = model_for_host(config.clone(), true);
        assert_eq!(external.model_name, "gemma4");
        assert_eq!(external.context_limit, Some(131_072));

        let managed = model_for_host(config, false);
        assert_eq!(managed.model_name, DEFAULT_MODEL);
        assert_eq!(
            managed.context_limit,
            ModelConfig::new(DEFAULT_MODEL).unwrap().context_limit
        );

        // And the request an external host receives names its own model.
        let provider = LlamaCppProvider {
            model: external,
            ..provider_with_external_base(Some("http://127.0.0.1:8080/"))
        };
        let mut p = payload();
        apply_request_policy(&mut p, &provider.served_alias("gemma4"), Thinking::Off);
        assert_eq!(p["model"], "gemma4");
    }

    /// The constructor itself: the remap used to run before
    /// `LLAMACPP_EXTERNAL_HOST` was read, so `served_alias`'s pass-through for
    /// external hosts never saw the user's name.
    #[tokio::test]
    async fn from_env_keeps_an_external_hosts_model_name() {
        let _guard = env_lock::lock_env([
            ("LLAMACPP_EXTERNAL_HOST", Some("http://127.0.0.1:8080")),
            ("BIOROUTER_CONTEXT_LIMIT", None),
        ]);
        let provider = LlamaCppProvider::from_env(ModelConfig::new_or_fail("qwen3.6"))
            .await
            .unwrap();
        assert_eq!(provider.get_model_config().model_name, "qwen3.6");
        assert_eq!(provider.served_alias("qwen3.6"), "qwen3.6");
    }

    /// The speed hint renders beside the suitability message, so its memory
    /// figures are the entry's own.
    #[test]
    fn the_speed_hint_states_the_entrys_own_memory_figures() {
        for entry in MODEL_CATALOG {
            let hint = entry.speed_hint;
            assert!(
                hint.contains(&format!("{} GiB", entry.min_gpu_memory_gib)),
                "{hint}"
            );
            assert!(
                hint.contains(&format!(
                    "{} GiB recommended",
                    entry.recommended_gpu_memory_gib
                )),
                "{hint}"
            );
            assert!(!hint.contains("24 GB"), "{hint}");
        }
    }

    #[test]
    fn raw_hf_spec_accepts_valid_forms() {
        for ok in [
            "owner/repo",
            "owner/repo:Q4_K_M",
            "bartowski/some-model-GGUF:UD-Q4_K_M",
            "a.b_c-d/e.f_g-h:Q8_0",
        ] {
            assert!(validate_raw_hf_spec(ok).is_ok(), "should accept {ok}");
        }
    }

    #[test]
    fn raw_hf_spec_rejects_traversal_flags_and_whitespace() {
        for bad in [
            "../../etc/passwd",    // path traversal
            "owner/../repo",       // dot-dot component
            "owner/repo extra",    // embedded whitespace / extra token
            "owner/repo --host",   // flag-shaped value
            "owner/repo/extra:Q4", // too many path components
            "owner/repo:Q4 K_M",   // whitespace in quant
            "owner/",              // empty repo
            "/repo",               // empty owner
            "owner/repo:",         // empty quant
        ] {
            assert!(validate_raw_hf_spec(bad).is_err(), "should reject {bad:?}");
            // And the public entrypoint rejects it too (raw specs contain '/').
            if bad.contains('/') {
                assert!(
                    resolve_hf_spec(bad).is_err(),
                    "resolve should reject {bad:?}"
                );
            }
        }
    }

    /// A screenshot reaches the provider as an IMAGE inside a tool response.
    /// The sidecar runs `--no-mmproj`, so that image cannot be seen; sending it
    /// produced a 5xx that classified as retryable, burning three more retries
    /// before surfacing an opaque error. Both shapes must become text.
    #[test]
    fn images_are_replaced_with_an_explanatory_note() {
        use crate::conversation::message::ToolResponse;
        use rmcp::model::CallToolResult;

        let image = Content::image("aGVsbG8=".to_string(), "image/png".to_string());
        let with_image = Message::user()
            .with_text("what is on screen?")
            .with_image("aGVsbG8=", "image/png");
        let tool_msg = Message::user().with_content(MessageContent::ToolResponse(ToolResponse {
            id: "call_1".to_string(),
            tool_result: Ok(CallToolResult::success(vec![
                Content::text("here is the screen"),
                image,
            ])),
            metadata: None,
        }));

        let stripped = strip_images(&[with_image, tool_msg]);

        // No bare image content survives.
        assert!(
            !stripped
                .iter()
                .flat_map(|m| m.content.iter())
                .any(|c| matches!(c, MessageContent::Image(_))),
            "a bare image survived stripping"
        );

        // No image survives INSIDE a tool response either — this is the one the
        // screenshot case actually takes, and the one a naive fix misses.
        let tool_images = stripped
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::ToolResponse(r) => r.tool_result.as_ref().ok(),
                _ => None,
            })
            .flat_map(|r| r.content.iter())
            .filter(|item| matches!(&item.raw, RawContent::Image(_)))
            .count();
        assert_eq!(tool_images, 0, "an image survived inside a tool response");

        // The surrounding text is preserved, and the note explains the failure.
        // Collect from BOTH levels: `MessageContent`'s Display does not render a
        // tool response's inner items, so a Display-only sweep would silently
        // miss whether the tool's own text survived — and would still pass.
        let mut all = Vec::new();
        for message in &stripped {
            for content in &message.content {
                match content {
                    MessageContent::ToolResponse(r) => {
                        if let Ok(result) = &r.tool_result {
                            for item in &result.content {
                                if let RawContent::Text(t) = &item.raw {
                                    all.push(t.text.clone());
                                }
                            }
                        }
                    }
                    other => all.push(other.to_string()),
                }
            }
        }
        let all = all.join(" ");
        assert!(all.contains("what is on screen?"), "user text was dropped");
        assert!(all.contains("here is the screen"), "tool text was dropped");
        assert_eq!(
            all.matches("cannot see images").count(),
            2,
            "both the bare image and the tool-response image need a note: {all}"
        );
    }

    #[test]
    fn metadata_has_zero_config_defaults() {
        let meta = LlamaCppProvider::metadata();
        assert_eq!(meta.name, "llamacpp");
        assert_eq!(meta.display_name, "Llama Server");
        assert_eq!(meta.default_model, default_model_name());
        assert!(meta.allows_unlisted_models);
        let port_key = meta
            .config_keys
            .iter()
            .find(|k| k.name == "LLAMACPP_PORT")
            .unwrap();
        assert!(port_key.required);
        assert_eq!(port_key.default.as_deref(), Some("11543"));
        // A download credential: declared secret, so the settings form keeps
        // it in the keyring rather than writing it to config.yaml.
        let token_key = meta
            .config_keys
            .iter()
            .find(|k| k.name == "LLAMACPP_HF_TOKEN")
            .expect("LLAMACPP_HF_TOKEN is a declared setting");
        assert!(token_key.secret && !token_key.required);
    }
}
