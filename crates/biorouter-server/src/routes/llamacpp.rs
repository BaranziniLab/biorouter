//! Routes for the managed llama-server sidecar backing the "Llama Server"
//! local provider. The desktop onboarding card and settings UI use these to
//! show sidecar state, kick off a model download/start, and stop the server.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::base::Provider;
use biorouter::providers::llamacpp::{
    default_model_name, resolve_chat_model_source, resolve_model_source, LlamaCppProvider,
    MODEL_CATALOG,
};
use biorouter::providers::llamacpp_sidecar::{self, ModelCacheStatus, SidecarStatus};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

const LLAMACPP_WARMUP_MAX_TOKENS: i32 = 256;

/// One curated local model, as shown in the GUI/TUI pickers.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LlamaCppModel {
    pub name: String,
    pub display_name: String,
    pub family: String,
    pub ollama_name: Option<String>,
    pub official_url: String,
    pub hf_spec: String,
    pub download_size: String,
    pub description: String,
    pub min_gpu_memory_gib: u64,
    pub recommended_gpu_memory_gib: u64,
    pub context_limit: usize,
    /// Approximate parameters active per generated token, in billions (MoE
    /// models activate a subset per token; this drives tokens/sec).
    pub active_params_b: u64,
    /// Short human-readable expected-speed hint shown next to the download
    /// size (e.g. "Fast per token — 3B active (MoE), but heavy to load").
    pub speed_hint: String,
    /// True for the model Biorouter preselects.
    pub is_default: bool,
    /// Whether the exact GGUF/quantization is already in Biorouter's llama.cpp cache.
    pub downloaded: bool,
    /// `downloaded`, `partial`, or `not_downloaded`.
    pub download_status: ModelCacheStatus,
    /// `ollama`, `huggingface_cache`, or `none`.
    pub download_source: String,
    /// Whether the llama.cpp-compatible Hugging Face fallback is already cached.
    pub fallback_downloaded: bool,
    /// Cache status for the llama.cpp-compatible Hugging Face fallback.
    pub fallback_download_status: ModelCacheStatus,
    /// The local GGUF blob path used when Ollama has already pulled this model.
    pub model_path: Option<String>,
    /// True when detected GPU-addressable memory meets the recommended tier.
    pub suitable: bool,
    /// `suitable`, `above_recommendation`, `below_minimum`, `cpu_only`, or
    /// `unknown_resources`.
    pub suitability_status: LlamaCppSuitability,
    pub suitability_message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlamaCppSuitability {
    /// Meets the model's recommended memory.
    Suitable,
    /// Meets the minimum but not the recommendation: it runs, with less room
    /// for the context window and the rest of the machine.
    AboveRecommendation,
    /// Below the model's minimum. Both catalog models are dense 27-31B models,
    /// so a machine under 32 GiB is still offered the default — there is no
    /// smaller model to fall back to — with this explicit warning, rather than
    /// a silent "not recommended".
    BelowMinimum,
    /// The bundled llama-server has no GPU backend here (an Intel Mac, the
    /// Linux CPU build): every layer runs on the CPU, at a few tokens per
    /// second, whatever the memory.
    CpuOnly,
    UnknownResources,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LlamaCppStatusResponse {
    pub sidecar: SidecarStatus,
    pub catalog: Vec<LlamaCppModel>,
    pub system: LlamaCppSystemInfo,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LlamaCppSystemInfo {
    pub os: String,
    pub total_memory_gib: u64,
    /// Apple Silicon unified memory, system RAM on a CPU-only machine, or
    /// detected discrete VRAM elsewhere.
    pub accelerator_memory_gib: Option<u64>,
    /// `apple_unified`, `cpu_only`, `vram`, or `unknown_vram`.
    pub accelerator_memory_kind: String,
    pub default_context_size: usize,
    /// Ollama-compatible model store root (`OLLAMA_MODELS` or `~/.ollama/models`).
    pub model_cache_dir: String,
    pub model_cache_layout: String,
}

#[derive(Deserialize, ToSchema)]
pub struct LlamaCppEnsureRequest {
    /// Catalog model name (e.g. `qwen3.8-27b`) or raw Hugging Face spec. A
    /// retired catalog name starts the default model.
    pub model: String,
}

#[derive(Deserialize, ToSchema)]
pub struct LlamaCppWarmupRequest {
    /// Catalog model name (e.g. `qwen3.8-27b`) or raw Hugging Face spec. A
    /// retired catalog name warms the default model.
    pub model: String,
}

#[derive(Deserialize, ToSchema)]
pub struct LlamaCppDeleteRequest {
    /// Catalog model name (e.g. `qwen3.8-27b`), retired catalog name (whose
    /// own cache is deleted), or raw Hugging Face spec.
    pub model: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LlamaCppWarmupResponse {
    pub sidecar: SidecarStatus,
    pub output: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LlamaCppDeleteResponse {
    /// Whether a Biorouter/llama.cpp Hugging Face fallback cache directory was removed.
    pub deleted_fallback_cache: bool,
    pub status: LlamaCppStatusResponse,
}

fn catalog() -> Vec<LlamaCppModel> {
    let default_model = default_model_name();
    let accelerator_memory_gib = llamacpp_sidecar::accelerator_memory_gib();
    let accelerator_memory_kind = llamacpp_sidecar::accelerator_memory_kind();
    MODEL_CATALOG
        .iter()
        .map(|e| {
            let source = resolve_model_source(e.name).expect("catalog entry resolves");
            let download_status = llamacpp_sidecar::model_source_cache_status(&source);
            let model_path = llamacpp_sidecar::model_source_path(&source);
            let fallback_download_status = llamacpp_sidecar::model_cache_status(e.hf_spec);
            // The order the sidecar launches from: the catalog's own Hugging
            // Face file first (it is the exact file the catalog pins), then a
            // GGUF in Ollama's store.
            let download_source = if fallback_download_status == ModelCacheStatus::Downloaded {
                "huggingface_cache"
            } else if model_path.is_some() {
                "ollama"
            } else {
                "none"
            };
            let (suitable, suitability_status, suitability_message) = suitability_for_model(
                e.display_name,
                e.min_gpu_memory_gib,
                e.recommended_gpu_memory_gib,
                accelerator_memory_gib,
                accelerator_memory_kind,
                std::env::consts::OS,
            );
            LlamaCppModel {
                name: e.name.to_string(),
                display_name: e.display_name.to_string(),
                family: e.family.to_string(),
                ollama_name: e.ollama_name.map(str::to_string),
                official_url: e.official_url.to_string(),
                hf_spec: e.hf_spec.to_string(),
                download_size: e.download_size.to_string(),
                description: e.description.to_string(),
                min_gpu_memory_gib: e.min_gpu_memory_gib,
                recommended_gpu_memory_gib: e.recommended_gpu_memory_gib,
                context_limit: e.context_limit,
                active_params_b: e.active_params_b,
                speed_hint: e.speed_hint.to_string(),
                is_default: e.name == default_model,
                downloaded: download_status == ModelCacheStatus::Downloaded,
                download_status,
                download_source: download_source.to_string(),
                fallback_downloaded: fallback_download_status == ModelCacheStatus::Downloaded,
                fallback_download_status,
                model_path: model_path.map(|path| path.display().to_string()),
                suitable,
                suitability_status,
                suitability_message,
            }
        })
        .collect()
}

fn suitability_for_model(
    display_name: &str,
    min_gpu_memory_gib: u64,
    recommended_gpu_memory_gib: u64,
    accelerator_memory_gib: Option<u64>,
    accelerator_memory_kind: &str,
    os: &str,
) -> (bool, LlamaCppSuitability, String) {
    // Checked before memory: an Intel Mac with 64 GiB of RAM meets every
    // memory figure and still runs a dense 27B at a token or two per second,
    // with the first turn's ~31-40k-token prompt taking many minutes. This
    // used to say "Recommended for this machine" there.
    if accelerator_memory_kind == "cpu_only" {
        let memory = accelerator_memory_gib
            .map(|gib| format!("{gib} GiB of system RAM"))
            .unwrap_or_else(|| "system RAM".to_string());
        // Where to send the user instead depends on the OS. On an Intel Mac,
        // Ollama's x86_64 build is CPU-only too, so recommending it there sent
        // the user to a second provider exactly as slow. On Linux the bundled
        // llama-server is the CPU build and Ollama ships CUDA, so a GPU Ollama
        // is the real alternative.
        let instead = if os == "macos" {
            "A cloud or institutional provider will be much faster here."
        } else {
            "An Ollama build with GPU support, or a cloud provider, will be much faster here."
        };
        return (
            false,
            LlamaCppSuitability::CpuOnly,
            format!(
                "Not recommended: Llama Server has no GPU acceleration on this machine, so \
                 {display_name} would run on the CPU out of {memory}. Expect a few tokens per \
                 second, and many minutes before the first reply of each chat. {instead}"
            ),
        );
    }
    match accelerator_memory_gib {
        Some(gib) if gib >= recommended_gpu_memory_gib => (
            true,
            LlamaCppSuitability::Suitable,
            format!(
                "Recommended for this machine: {gib} GiB {accelerator_memory_kind} meets the {recommended_gpu_memory_gib} GiB recommendation."
            ),
        ),
        Some(gib) if gib >= min_gpu_memory_gib => (
            false,
            LlamaCppSuitability::AboveRecommendation,
            format!(
                "This model recommends {recommended_gpu_memory_gib} GiB of GPU-addressable memory; this machine reports {gib} GiB {accelerator_memory_kind}. It will run, with less room for other apps."
            ),
        ),
        Some(gib) => (
            false,
            LlamaCppSuitability::BelowMinimum,
            format!(
                "Below the minimum: {display_name} needs at least {min_gpu_memory_gib} GiB of GPU-addressable memory and this machine reports {gib} GiB {accelerator_memory_kind}. It may fail to load or run very slowly; Biorouter has no smaller local model to offer instead."
            ),
        ),
        None => (
            false,
            LlamaCppSuitability::UnknownResources,
            format!(
                "Could not detect GPU VRAM. This model recommends {recommended_gpu_memory_gib} GiB of GPU-addressable memory; system RAM alone is not enough on Windows or Linux machines with discrete GPUs."
            ),
        ),
    }
}

fn system_info() -> LlamaCppSystemInfo {
    LlamaCppSystemInfo {
        os: std::env::consts::OS.to_string(),
        total_memory_gib: llamacpp_sidecar::total_memory_gib(),
        accelerator_memory_gib: llamacpp_sidecar::accelerator_memory_gib(),
        accelerator_memory_kind: llamacpp_sidecar::accelerator_memory_kind().to_string(),
        default_context_size: llamacpp_sidecar::default_context_size(),
        model_cache_dir: llamacpp_sidecar::model_cache_dir().display().to_string(),
        model_cache_layout:
            "Ollama manifests/blobs; Hugging Face fallback cache under the same root".to_string(),
    }
}

async fn status_response() -> LlamaCppStatusResponse {
    LlamaCppStatusResponse {
        sidecar: llamacpp_sidecar::global().status().await,
        catalog: catalog(),
        system: system_info(),
    }
}

#[utoipa::path(
    get,
    path = "/llamacpp/status",
    responses(
        (status = 200, description = "Sidecar status and curated model catalog", body = LlamaCppStatusResponse)
    ),
)]
async fn llamacpp_status() -> Json<LlamaCppStatusResponse> {
    Json(status_response().await)
}

#[utoipa::path(
    post,
    path = "/llamacpp/ensure",
    request_body = LlamaCppEnsureRequest,
    responses(
        (status = 200, description = "Sidecar start initiated (poll /llamacpp/status)", body = LlamaCppStatusResponse),
        (status = 400, description = "Unknown model name"),
    ),
)]
async fn llamacpp_ensure(
    Json(req): Json<LlamaCppEnsureRequest>,
) -> Result<Json<LlamaCppStatusResponse>, (StatusCode, String)> {
    // Starting a model is for chatting with it, so a retired catalog name
    // starts the default model, exactly as a chat that names it would.
    let (model, source) = resolve_chat_model_source(&req.model)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let sidecar = llamacpp_sidecar::global();
    sidecar
        .ensure(&model, &source)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Wait for readiness in the background (first run downloads the model);
    // clients poll /llamacpp/status for progress.
    tokio::spawn(async move {
        if let Err(e) = llamacpp_sidecar::global()
            .wait_ready(Duration::from_secs(3600))
            .await
        {
            tracing::warn!("llama-server for {model} did not become ready: {e}");
        }
    });

    Ok(Json(status_response().await))
}

#[utoipa::path(
    post,
    path = "/llamacpp/warmup",
    request_body = LlamaCppWarmupRequest,
    responses(
        (status = 200, description = "Model loaded and produced a test completion", body = LlamaCppWarmupResponse),
        (status = 400, description = "Unknown model name"),
        (status = 502, description = "Model failed to produce a warm-up completion"),
    ),
)]
async fn llamacpp_warmup(
    Json(req): Json<LlamaCppWarmupRequest>,
) -> Result<Json<LlamaCppWarmupResponse>, (StatusCode, String)> {
    let (model_name, _) = resolve_chat_model_source(&req.model)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let model = ModelConfig::new(&model_name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        .with_temperature(Some(0.0))
        // A ceiling, not a target: with thinking off the model answers `OK`
        // in a token or two.
        .with_max_tokens(Some(LLAMACPP_WARMUP_MAX_TOKENS));
    let provider = LlamaCppProvider::from_env(model)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Thinking off whatever `LLAMACPP_ENABLE_THINKING` says: a thinking model
    // can spend the whole budget reasoning and return no visible text, which
    // this check would report as a failed warm-up.
    let (message, _) = provider
        .complete_without_thinking(
            &provider.get_model_config(),
            "You are a local model warm-up check. Reply with OK.",
            &[Message::user().with_text("Reply with exactly OK.")],
            &[],
        )
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let output = message.as_concat_text().trim().to_string();
    if output.is_empty() {
        return Err((
            StatusCode::BAD_GATEWAY,
            "Llama Server returned an empty warm-up completion".to_string(),
        ));
    }

    let sidecar = llamacpp_sidecar::global();
    sidecar.mark_warmed(&model_name).await;
    Ok(Json(LlamaCppWarmupResponse {
        sidecar: sidecar.status().await,
        output,
    }))
}

#[utoipa::path(
    post,
    path = "/llamacpp/delete",
    request_body = LlamaCppDeleteRequest,
    responses(
        (status = 200, description = "Cached Llama Server fallback model removed", body = LlamaCppDeleteResponse),
        (status = 400, description = "Unknown model name"),
    ),
)]
async fn llamacpp_delete(
    Json(req): Json<LlamaCppDeleteRequest>,
) -> Result<Json<LlamaCppDeleteResponse>, (StatusCode, String)> {
    // EXACT resolution, never the chat alias: a retired name deletes its own
    // orphaned cache (3-24 GB), not the default model's that a chat on it runs.
    let source =
        resolve_model_source(&req.model).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let sidecar = llamacpp_sidecar::global();
    if sidecar.status().await.model.as_deref() == Some(req.model.as_str()) {
        sidecar.stop().await;
    }
    let deleted_fallback_cache = llamacpp_sidecar::delete_model_cache(&source.hf_spec)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(LlamaCppDeleteResponse {
        deleted_fallback_cache,
        status: status_response().await,
    }))
}

#[utoipa::path(
    post,
    path = "/llamacpp/stop",
    responses(
        (status = 200, description = "Sidecar stopped", body = LlamaCppStatusResponse)
    ),
)]
async fn llamacpp_stop() -> Json<LlamaCppStatusResponse> {
    let sidecar = llamacpp_sidecar::global();
    sidecar.stop().await;
    Json(status_response().await)
}

/// Stateless router, exposed separately so tests can drive the routes
/// without constructing an `AppState`.
pub fn router() -> Router {
    Router::new()
        .route("/llamacpp/status", get(llamacpp_status))
        .route("/llamacpp/ensure", post(llamacpp_ensure))
        .route("/llamacpp/warmup", post(llamacpp_warmup))
        .route("/llamacpp/delete", post(llamacpp_delete))
        .route("/llamacpp/stop", post(llamacpp_stop))
}

pub fn routes(_state: Arc<AppState>) -> Router {
    router()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(gib: Option<u64>, kind: &str) -> (bool, LlamaCppSuitability, String) {
        suitability_for_model("Qwen3.8 27B", 32, 48, gib, kind, "linux")
    }

    #[test]
    fn suitability_follows_the_minimum_and_the_recommendation() {
        let (ok, s, _) = status(Some(128), "apple_unified");
        assert!(ok);
        assert_eq!(s, LlamaCppSuitability::Suitable);

        let (ok, s, message) = status(Some(36), "apple_unified");
        assert!(!ok);
        assert_eq!(s, LlamaCppSuitability::AboveRecommendation);
        assert!(message.contains("48 GiB"), "{message}");

        // The product decision: a 16 GiB machine still gets the default model,
        // with an explicit below-minimum warning rather than a different model.
        let (ok, s, message) = status(Some(16), "apple_unified");
        assert!(!ok);
        assert_eq!(s, LlamaCppSuitability::BelowMinimum);
        assert!(message.contains("at least 32 GiB"), "{message}");
        assert!(message.contains("Qwen3.8 27B"), "{message}");

        let (_, s, _) = status(None, "unknown_vram");
        assert_eq!(s, LlamaCppSuitability::UnknownResources);
    }

    /// LC-9. An Intel Mac with plenty of RAM meets every memory figure and is
    /// still not a machine to run a dense 27B on: it has no GPU backend.
    #[test]
    fn a_cpu_only_machine_is_never_recommended() {
        let (ok, s, message) = status(Some(64), "cpu_only");
        assert!(!ok);
        assert_eq!(s, LlamaCppSuitability::CpuOnly);
        assert!(message.contains("64 GiB of system RAM"), "{message}");
        assert!(message.contains("CPU"), "{message}");
        // The message is one sentence flow: a lost `\` line continuation once
        // left runs of indentation spaces inside it.
        assert!(!message.contains("  "), "{message}");
        assert_eq!(
            serde_json::to_value(LlamaCppSuitability::CpuOnly).unwrap(),
            "cpu_only"
        );
        assert_eq!(
            serde_json::to_value(LlamaCppSuitability::BelowMinimum).unwrap(),
            "below_minimum"
        );
    }

    /// The alternative named depends on the OS: an Intel Mac's Ollama is
    /// CPU-only as well, so it is not one to suggest there; on Linux a GPU
    /// Ollama is the real way out of the bundled CPU build.
    #[test]
    fn a_cpu_only_machine_is_pointed_at_an_alternative_that_is_faster_there() {
        let (_, _, mac) =
            suitability_for_model("Qwen3.8 27B", 32, 48, Some(64), "cpu_only", "macos");
        assert!(!mac.contains("Ollama"), "{mac}");
        assert!(mac.contains("cloud or institutional provider"), "{mac}");
        assert!(!mac.contains("  "), "{mac}");

        let (_, _, linux) = status(Some(64), "cpu_only");
        assert!(linux.contains("Ollama build with GPU support"), "{linux}");
        assert!(!linux.contains("  "), "{linux}");
    }
}
