//! Routes for the Ollama provider's onboarding card and the chat's context
//! gauge.
//!
//! Both answer from the provider's own definitions rather than letting the
//! desktop keep a copy. The onboarding card used to hard-code `gpt-oss:20b` in
//! `utils/ollamaDetection.ts` while the provider defaulted to something else,
//! and the gauge guessed a window from the model's NAME — 128k for anything
//! with "qwen" in it — while Ollama served the 2k-4k it was started with. That
//! gap is the bug these routes exist to close: the "model writes JSON instead
//! of doing the task" report was Ollama silently cutting a ~31-40k-token
//! prompt to its default window.
//!
//! Neither route names a chat, a knowledge base or anything a chat produced, so
//! neither goes through `session_reach`; both sit behind the daemon's secret
//! key like `/llamacpp/*`.

use std::sync::Arc;

use axum::{extract::Query, http::StatusCode, routing::get, Json, Router};
use biorouter::config::Config;
use biorouter::model::ModelConfig;
use biorouter::privacy::ProviderTier;
use biorouter::providers::base::Provider;
use biorouter::providers::ollama::{
    resolve_ollama_base_url, sizing_memory, OllamaProvider, OLLAMA_DEFAULT_MODEL, OLLAMA_HOST,
    OLLAMA_RECOMMENDED_MIN_MEMORY_GIB, OLLAMA_RECOMMENDED_MODELS,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::state::AppState;

/// One model Biorouter recommends pulling into Ollama.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct OllamaRecommendedModel {
    /// The tag to `ollama pull` (e.g. `qwen3.8:27b`).
    pub tag: String,
    pub display_name: String,
    /// Approximate download size, in GB.
    pub approx_download_gb: u32,
    /// True for the model the Ollama provider uses when none is chosen.
    pub is_default: bool,
}

/// Whether this machine can run the recommended models at a usable speed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OllamaSuitability {
    /// At least the recommended minimum of GPU-addressable memory.
    Suitable,
    /// Less than the minimum. The default model is still offered — there is no
    /// smaller supported model to fall back to — with this warning.
    BelowMinimum,
    /// An Intel Mac: Ollama's x86_64 build does not use Metal, so every layer
    /// runs on the CPU, whatever the memory.
    CpuOnly,
    /// The GPU memory could not be measured: on Linux, no NVIDIA GPU answered
    /// `nvidia-smi` (system RAM is never rated instead; see
    /// [`suitability_for`]).
    UnknownResources,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct OllamaRecommendationsResponse {
    /// Default first.
    pub models: Vec<OllamaRecommendedModel>,
    /// The model the Ollama provider uses when none is chosen.
    pub default_model: String,
    /// GPU-addressable memory below which the recommended models are below
    /// their practical minimum.
    pub min_memory_gib: u64,
    /// The memory the Ollama provider sizes its context window by
    /// (`providers::ollama::sizing_memory`): unified memory on Apple Silicon,
    /// system RAM on an Intel Mac, detected VRAM elsewhere — on Linux an NVIDIA
    /// GPU's, never system RAM. Null when it could not be measured.
    pub memory_gib: Option<u64>,
    /// `apple_unified`, `cpu_only`, `vram`, or `unknown_vram`.
    pub memory_kind: String,
    pub suitability: OllamaSuitability,
    /// One or two sentences for the user, written for this machine.
    pub suitability_message: String,
    /// The Ollama server the provider talks to: `OLLAMA_HOST` resolved exactly
    /// as the provider (and the Ollama CLI) resolve it, so the desktop probes
    /// the server a chat will actually use.
    pub base_url: String,
    /// Whether `OLLAMA_HOST` is set, in the config file or the environment.
    /// Onboarding writes the default only when it is not: it used to write
    /// `localhost` unconditionally, replacing a remote host the user had chosen.
    pub host_configured: bool,
    /// The privacy tier a chat on the default model gets from this server:
    /// `private` exactly while `OLLAMA_HOST` resolves to a loopback address
    /// (`localhost`, `127.0.0.1`, `::1`). It is the provider's own `tier()`,
    /// not a second reading of the host: the desktop decides "this computer"
    /// with it, and used to decide by its own list, which called `0.0.0.0`
    /// this computer (the tier says Public) and `::1` another one (Private).
    pub tier: ProviderTier,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OllamaContextWindowQuery {
    /// The Ollama model tag, e.g. `qwen3.8:27b`.
    pub model: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct OllamaContextWindowResponse {
    pub model: String,
    /// The `num_ctx` Biorouter sends for this model to the configured server:
    /// `OLLAMA_NUM_CTX`, else a context limit set on the model
    /// (`BIOROUTER_CONTEXT_LIMIT`, or a predefined model's limit), else the
    /// memory tier (64k, or 128k with 64 GiB or more), capped at the window
    /// the model was trained for. Null when the server could not describe the
    /// model (unreachable, not Ollama, or the model is not pulled).
    pub context_window: Option<usize>,
}

/// What a memory figure means on this machine, for a sentence.
fn memory_label(kind: &str) -> &'static str {
    match kind {
        "apple_unified" => "unified memory",
        _ => "VRAM",
    }
}

fn suitability_for(
    memory_gib: Option<u64>,
    memory_kind: &str,
    os: &str,
) -> (OllamaSuitability, String) {
    let min = OLLAMA_RECOMMENDED_MIN_MEMORY_GIB;
    if memory_kind == "cpu_only" {
        // Checked before memory, like the Llama Server route: an Intel Mac with
        // 64 GiB of RAM clears every memory figure and still runs a dense 27B
        // at a token or two per second.
        if os == "macos" {
            return (
                OllamaSuitability::CpuOnly,
                "Ollama runs models on the processor on an Intel Mac, so Qwen3.8 27B and Gemma 4 \
                 31B will answer at a token or two per second, with several minutes before the \
                 first reply of each chat. A cloud or institutional provider will be much faster \
                 here."
                    .to_string(),
            );
        }
        // Elsewhere a `cpu_only` figure is the sidecar's answer about ITS OWN
        // build (on Linux the bundled llama-server is the CPU one) and is
        // system RAM, which says nothing about what Ollama, which has CUDA
        // there, can put on a GPU. `sizing_memory` no longer passes it here on
        // Linux; this arm keeps any caller that does from rating that RAM
        // against a GPU-memory minimum, which once answered "64 GiB of system
        // RAM, enough" for a 12 GB RTX 3060 and for a laptop with no GPU.
        return (OllamaSuitability::UnknownResources, no_gpu_message(min));
    }
    let label = memory_label(memory_kind);
    match memory_gib {
        Some(gib) if gib >= min => (
            OllamaSuitability::Suitable,
            format!(
                "This machine reports {gib} GiB of {label}, enough for the recommended local models."
            ),
        ),
        Some(gib) => (
            OllamaSuitability::BelowMinimum,
            format!(
                "This machine reports {gib} GiB of {label}; the recommended local models need at \
                 least {min} GiB. They may fail to load or run very slowly, and Biorouter has no \
                 smaller local model to recommend instead."
            ),
        ),
        // Linux reads an NVIDIA GPU only, so "unmeasured" there usually means
        // no GPU Ollama can use, and the processor is what runs the model.
        None if os == "linux" => (OllamaSuitability::UnknownResources, no_gpu_message(min)),
        None => (
            OllamaSuitability::UnknownResources,
            format!(
                "Biorouter could not measure this machine's GPU memory. The recommended local \
                 models need at least {min} GiB of GPU-addressable memory."
            ),
        ),
    }
}

/// The answer when no GPU memory was measured and system RAM must not stand in
/// for it.
fn no_gpu_message(min: u64) -> String {
    format!(
        "Biorouter could not measure this machine's GPU memory, and system RAM does not \
         count: the recommended local models need at least {min} GiB of GPU memory to \
         run at a usable speed. Without a supported GPU, Ollama runs them on the \
         processor at a token or two per second."
    )
}

/// `OLLAMA_HOST` as the provider reads it: environment first, then the config
/// file, with an empty value counting as unset.
fn configured_host() -> Option<String> {
    Config::global()
        .get_param::<String>("OLLAMA_HOST")
        .ok()
        .filter(|host| !host.trim().is_empty())
}

fn recommendations(
    tier: ProviderTier,
) -> Result<OllamaRecommendationsResponse, (StatusCode, String)> {
    let host = configured_host();
    let base_url = resolve_ollama_base_url(host.as_deref().unwrap_or(OLLAMA_HOST))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // The figure the provider sizes `num_ctx` by — the same function, so the
    // card and a chat can never disagree about this machine.
    let (memory_gib, memory_kind) = sizing_memory();
    let (suitability, suitability_message) =
        suitability_for(memory_gib, memory_kind, std::env::consts::OS);
    Ok(OllamaRecommendationsResponse {
        models: OLLAMA_RECOMMENDED_MODELS
            .iter()
            .map(|m| OllamaRecommendedModel {
                tag: m.tag.to_string(),
                display_name: m.display_name.to_string(),
                approx_download_gb: m.approx_download_gb,
                is_default: m.tag == OLLAMA_DEFAULT_MODEL,
            })
            .collect(),
        default_model: OLLAMA_DEFAULT_MODEL.to_string(),
        min_memory_gib: OLLAMA_RECOMMENDED_MIN_MEMORY_GIB,
        memory_gib,
        memory_kind: memory_kind.to_string(),
        suitability,
        suitability_message,
        base_url: base_url.to_string(),
        host_configured: host.is_some(),
        tier,
    })
}

/// The tier a chat on the default model gets from the configured server, from
/// the provider built exactly as a chat builds it. Building it reads config and
/// touches no network.
async fn default_model_tier() -> Result<ProviderTier, (StatusCode, String)> {
    let config = ModelConfig::new(OLLAMA_DEFAULT_MODEL)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let provider = OllamaProvider::from_env(config)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(provider.tier())
}

#[utoipa::path(
    get,
    path = "/ollama/recommended",
    responses(
        (status = 200, description = "The Ollama models Biorouter recommends, this machine's memory, and the configured Ollama server", body = OllamaRecommendationsResponse),
        (status = 400, description = "OLLAMA_HOST is not a usable address"),
    ),
)]
async fn ollama_recommended() -> Result<Json<OllamaRecommendationsResponse>, (StatusCode, String)> {
    let tier = default_model_tier().await?;
    // The memory probe can run `system_profiler` on a Mac with a discrete GPU
    // (cached after the first call), so it stays off the async workers.
    tokio::task::spawn_blocking(move || recommendations(tier))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/ollama/context_window",
    params(OllamaContextWindowQuery),
    responses(
        (status = 200, description = "The context window Biorouter uses for this Ollama model (null when the server could not describe it)", body = OllamaContextWindowResponse),
        (status = 400, description = "Empty model name, or OLLAMA_HOST is not a usable address"),
    ),
)]
async fn ollama_context_window(
    Query(query): Query<OllamaContextWindowQuery>,
) -> Result<Json<OllamaContextWindowResponse>, (StatusCode, String)> {
    let model = query.model.trim().to_string();
    if model.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "model is required".to_string()));
    }
    let config = ModelConfig::new(&model).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // The built-in provider, built exactly as a chat builds it, so the answer is
    // the window a chat on this model runs with — not a second computation that
    // could disagree with it.
    let provider = OllamaProvider::from_env(config)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let context_window = provider.context_window_for(&model).await;
    Ok(Json(OllamaContextWindowResponse {
        model,
        context_window,
    }))
}

/// Stateless router, exposed separately so tests can drive the routes
/// without constructing an `AppState`.
pub fn router() -> Router {
    Router::new()
        .route("/ollama/recommended", get(ollama_recommended))
        .route("/ollama/context_window", get(ollama_context_window))
}

pub fn routes(_state: Arc<AppState>) -> Router {
    router()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suitability_follows_the_minimum() {
        let (s, message) = suitability_for(Some(64), "apple_unified", "macos");
        assert_eq!(s, OllamaSuitability::Suitable);
        assert!(message.contains("64 GiB of unified memory"), "{message}");

        let (s, message) = suitability_for(Some(32), "vram", "windows");
        assert_eq!(s, OllamaSuitability::Suitable, "{message}");

        // The product decision: a 16 GiB machine is warned, not offered a
        // different model.
        let (s, message) = suitability_for(Some(16), "apple_unified", "macos");
        assert_eq!(s, OllamaSuitability::BelowMinimum);
        assert!(message.contains("at least 32 GiB"), "{message}");
        assert!(message.contains("16 GiB of unified memory"), "{message}");

        let (s, _) = suitability_for(None, "unknown_vram", "windows");
        assert_eq!(s, OllamaSuitability::UnknownResources);
    }

    /// An Intel Mac clears every memory figure and is still a CPU.
    #[test]
    fn an_intel_mac_is_cpu_only_whatever_its_memory() {
        let (s, message) = suitability_for(Some(64), "cpu_only", "macos");
        assert_eq!(s, OllamaSuitability::CpuOnly);
        assert!(message.contains("Intel Mac"), "{message}");
        // One sentence flow: a lost `\` continuation would leave indentation
        // runs inside it.
        assert!(!message.contains("  "), "{message}");
    }

    /// The sidecar says `cpu_only` for its own CPU build on Linux, which says
    /// nothing about Ollama's CUDA build there, and its figure is system RAM.
    /// This answered Suitable — "64 GiB of system RAM, enough" — for a 12 GB
    /// GPU and for a machine with no GPU at all.
    #[test]
    fn linux_system_ram_is_never_rated_as_gpu_memory() {
        for gib in [Some(64), Some(16), None] {
            let (s, message) = suitability_for(gib, "cpu_only", "linux");
            assert_eq!(s, OllamaSuitability::UnknownResources, "{message}");
            assert!(!message.contains("enough"), "{message}");
            assert!(
                message.contains("at least 32 GiB of GPU memory"),
                "{message}"
            );
            assert!(message.contains("processor"), "{message}");
            assert!(!message.contains("  "), "{message}");
        }
    }

    /// Linux is judged by the NVIDIA GPU's VRAM (`sizing_memory`), so a GPU
    /// that clears the minimum is Suitable, one that does not is warned, and
    /// only a machine with no measurable GPU is unknown — with the processor
    /// named, because that is what runs the model there.
    #[test]
    fn linux_is_judged_by_its_gpu() {
        let (s, message) = suitability_for(Some(48), "vram", "linux");
        assert_eq!(s, OllamaSuitability::Suitable, "{message}");
        assert!(message.contains("48 GiB of VRAM"), "{message}");

        let (s, message) = suitability_for(Some(12), "vram", "linux");
        assert_eq!(s, OllamaSuitability::BelowMinimum, "{message}");
        assert!(message.contains("12 GiB of VRAM"), "{message}");

        let (s, message) = suitability_for(None, "unknown_vram", "linux");
        assert_eq!(s, OllamaSuitability::UnknownResources, "{message}");
        assert!(message.contains("processor"), "{message}");
        assert!(message.contains("system RAM does not count"), "{message}");
        assert!(!message.contains("  "), "{message}");

        // Off Linux an unmeasured GPU is only that.
        let (_, message) = suitability_for(None, "unknown_vram", "windows");
        assert!(!message.contains("processor"), "{message}");
    }

    /// The model names the Llama Server catalog and the docs use.
    #[test]
    fn the_intel_mac_message_spells_the_models_as_the_catalog_does() {
        let (_, message) = suitability_for(Some(64), "cpu_only", "macos");
        assert!(message.contains("Qwen3.8 27B"), "{message}");
        assert!(!message.contains("Qwen 3.8"), "{message}");
    }

    #[test]
    fn every_message_is_one_flowing_sentence() {
        for (gib, kind, os) in [
            (Some(8), "vram", "linux"),
            (None, "unknown_vram", "windows"),
            (Some(96), "apple_unified", "macos"),
        ] {
            let (_, message) = suitability_for(gib, kind, os);
            assert!(!message.contains("  "), "{message}");
        }
    }

    #[test]
    fn serialized_statuses_are_snake_case() {
        for (status, wire) in [
            (OllamaSuitability::Suitable, "suitable"),
            (OllamaSuitability::BelowMinimum, "below_minimum"),
            (OllamaSuitability::CpuOnly, "cpu_only"),
            (OllamaSuitability::UnknownResources, "unknown_resources"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), wire);
        }
    }
}
