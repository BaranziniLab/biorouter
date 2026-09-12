use anyhow::{bail, Result};
use biorouter::config::Config;
use biorouter::providers::base::{ProviderMetadata, ProviderType};
use biorouter::providers::providers;
use console::style;
use serde::Serialize;

/// One entry in a provider's known-model list. Carries the per-model context
/// window and capability/pricing metadata the core crate already knows, so the
/// CLI surfaces the same facts the desktop model picker does (e.g. after every
/// model gained its own context window).
#[derive(Serialize)]
struct ModelSummary {
    name: String,
    context_limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_token_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_token_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
}

#[derive(Serialize)]
struct ProviderSummary {
    name: String,
    display_name: String,
    provider_type: ProviderType,
    default_model: String,
    models: Vec<ModelSummary>,
    allows_unlisted_models: bool,
}

fn provider_summary(metadata: ProviderMetadata, provider_type: ProviderType) -> ProviderSummary {
    ProviderSummary {
        name: metadata.name,
        display_name: metadata.display_name,
        provider_type,
        default_model: metadata.default_model,
        models: metadata
            .known_models
            .into_iter()
            .map(|model| ModelSummary {
                name: model.name,
                context_limit: model.context_limit,
                supports_vision: model.supports_vision,
                input_token_cost: model.input_token_cost,
                output_token_cost: model.output_token_cost,
                currency: model.currency,
            })
            .collect(),
        allows_unlisted_models: metadata.allows_unlisted_models,
    }
}

/// Format a raw per-token cost as a friendly `$/1M tokens` string, matching how
/// the desktop cost tracker presents pricing.
fn format_price_per_million(cost: f64, currency: &str) -> String {
    format!("{}{:.2}/1M", currency, cost * 1_000_000.0)
}

async fn provider_summaries() -> Vec<ProviderSummary> {
    providers()
        .await
        .into_iter()
        .map(|(metadata, provider_type)| provider_summary(metadata, provider_type))
        .collect()
}

pub async fn handle_models_current(format: &str) -> Result<()> {
    let config = Config::global();
    let provider = config.get_biorouter_provider().ok();
    let model = config.get_biorouter_model().ok();

    if format == "json" {
        println!(
            "{}",
            serde_json::json!({
                "provider": provider,
                "model": model,
            })
        );
        return Ok(());
    }

    match (provider, model) {
        (Some(provider), Some(model)) => {
            println!("Current model configuration");
            println!("  provider: {}", style(provider).cyan());
            println!("  model:    {}", style(model).cyan());
        }
        _ => {
            println!("No provider/model is configured.");
            println!(
                "Run `biorouter configure` or `biorouter models set --provider <name> --model <model>`."
            );
        }
    }

    Ok(())
}

pub async fn handle_models_providers(format: &str) -> Result<()> {
    let mut summaries = provider_summaries().await;
    summaries.sort_by(|a, b| a.name.cmp(&b.name));

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }

    if summaries.is_empty() {
        println!("No providers found.");
        return Ok(());
    }

    println!("Available providers");
    for provider in summaries {
        println!(
            "  {:<18} {:<12} default: {}",
            provider.name,
            format!("{:?}", provider.provider_type).to_lowercase(),
            provider.default_model
        );
    }

    Ok(())
}

pub async fn handle_models_list(provider_name: String, format: &str) -> Result<()> {
    let summaries = provider_summaries().await;
    let provider = summaries
        .into_iter()
        .find(|provider| provider.name == provider_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", provider_name))?;

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&provider)?);
        return Ok(());
    }

    println!(
        "{} ({})",
        style(&provider.display_name).cyan().bold(),
        provider.name
    );
    println!("  type:    {:?}", provider.provider_type);
    println!("  default: {}", provider.default_model);
    if provider.models.is_empty() {
        println!("  models:  no static model list available");
        if provider.allows_unlisted_models {
            println!("           custom model names are allowed");
        }
        return Ok(());
    }

    let name_width = provider
        .models
        .iter()
        .map(|m| m.name.len())
        .max()
        .unwrap_or(0);
    println!("  models:");
    for model in &provider.models {
        let mut detail = String::new();
        if model.context_limit > 0 {
            detail.push_str(&format!("{} ctx", human_context(model.context_limit)));
        }
        if model.supports_vision == Some(true) {
            if !detail.is_empty() {
                detail.push_str("  ");
            }
            detail.push_str("vision");
        }
        if let (Some(input), Some(output)) = (model.input_token_cost, model.output_token_cost) {
            let currency = model.currency.as_deref().unwrap_or("$");
            if !detail.is_empty() {
                detail.push_str("  ");
            }
            detail.push_str(&format!(
                "in {} · out {}",
                format_price_per_million(input, currency),
                format_price_per_million(output, currency)
            ));
        }
        if detail.is_empty() {
            println!("    {}", model.name);
        } else {
            println!(
                "    {:<width$}   {}",
                model.name,
                style(detail).dim(),
                width = name_width
            );
        }
    }
    if provider.allows_unlisted_models {
        println!("    (custom model names are also allowed)");
    }

    Ok(())
}

/// Compact human-readable context window, e.g. `1,050,000` → `1.05M`, `200000`
/// → `200K`.
fn human_context(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

pub async fn handle_models_set(provider_name: String, model: String) -> Result<()> {
    let summaries = provider_summaries().await;
    let provider = summaries
        .iter()
        .find(|provider| provider.name == provider_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", provider_name))?;

    let known_model = provider.models.iter().any(|known| known.name == model);
    if !known_model && !provider.allows_unlisted_models {
        bail!(
            "Model '{}' is not listed for provider '{}'. Run `biorouter models list {}` to see known models.",
            model,
            provider_name,
            provider_name
        );
    }

    let config = Config::global();
    config.set_biorouter_provider_and_model(provider_name.clone(), &model)?;

    println!("Model configuration updated");
    println!("  provider: {}", style(provider_name).cyan());
    println!("  model:    {}", style(model).cyan());

    Ok(())
}

// ── Local model inventory (Llama Server) ─────────────────────────────────────
//
// Terminal parity with the desktop "Local model inventory" controls. The CLI
// calls the core `biorouter` crate directly, so these read/manage the same
// on-disk model cache (Ollama / Hugging Face store) the desktop daemon uses —
// no running server required.

use biorouter::providers::llamacpp::{default_model_name, resolve_model_source, MODEL_CATALOG};
use biorouter::providers::llamacpp_sidecar::{self, ModelCacheStatus};

#[derive(Serialize)]
struct LocalModelRow {
    name: String,
    display_name: String,
    family: String,
    download_size: String,
    is_default: bool,
    downloaded: bool,
    source: String,
    model_path: Option<String>,
    context_limit: usize,
}

fn local_model_rows() -> Vec<LocalModelRow> {
    let default_model = default_model_name();
    MODEL_CATALOG
        .iter()
        .map(|e| {
            let (downloaded, source, model_path) = match resolve_model_source(e.name) {
                Ok(source) => {
                    let status = llamacpp_sidecar::model_source_cache_status(&source);
                    let path = llamacpp_sidecar::model_source_path(&source);
                    let fallback = llamacpp_sidecar::model_cache_status(e.hf_spec);
                    let source_label = if path.is_some() {
                        "ollama"
                    } else if fallback == ModelCacheStatus::Downloaded {
                        "huggingface_cache"
                    } else {
                        "none"
                    };
                    (
                        status == ModelCacheStatus::Downloaded,
                        source_label.to_string(),
                        path.map(|p| p.display().to_string()),
                    )
                }
                Err(_) => (false, "none".to_string(), None),
            };
            LocalModelRow {
                name: e.name.to_string(),
                display_name: e.display_name.to_string(),
                family: e.family.to_string(),
                download_size: e.download_size.to_string(),
                is_default: e.name == default_model,
                downloaded,
                source,
                model_path,
                context_limit: e.context_limit,
            }
        })
        .collect()
}

pub async fn handle_models_local_list(format: &str) -> Result<()> {
    let rows = local_model_rows();

    if format == "json" {
        let payload = serde_json::json!({
            "os": std::env::consts::OS,
            "total_memory_gib": llamacpp_sidecar::total_memory_gib(),
            "accelerator_memory_gib": llamacpp_sidecar::accelerator_memory_gib(),
            "accelerator_memory_kind": llamacpp_sidecar::accelerator_memory_kind(),
            "default_context_size": llamacpp_sidecar::default_context_size(),
            "model_cache_dir": llamacpp_sidecar::model_cache_dir().display().to_string(),
            "models": rows,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("{}", style("Local models (Llama Server)").cyan().bold());
    println!(
        "  cache dir: {}",
        style(llamacpp_sidecar::model_cache_dir().display()).dim()
    );
    let accel = match llamacpp_sidecar::accelerator_memory_gib() {
        Some(gib) => format!(
            "{} GiB {}",
            gib,
            llamacpp_sidecar::accelerator_memory_kind()
        ),
        None => "undetected".to_string(),
    };
    println!(
        "  memory:    {} GiB system · {} GPU",
        llamacpp_sidecar::total_memory_gib(),
        accel
    );
    println!(
        "  {}",
        style("(the approximate download size is shown; delete with `biorouter models local rm <name>`)").dim()
    );
    println!();

    let name_width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    for r in &rows {
        let marker = if r.downloaded {
            style("●").green().to_string()
        } else {
            style("○").dim().to_string()
        };
        let state = if r.downloaded {
            format!("downloaded ({})", r.source)
        } else {
            "not downloaded".to_string()
        };
        let default_tag = if r.is_default {
            format!(" {}", style("[default]").cyan())
        } else {
            String::new()
        };
        println!(
            "  {} {:<width$}  {:<8}  {:<22}{}",
            marker,
            style(&r.name).bold(),
            style(&r.download_size).dim(),
            style(state).dim(),
            default_tag,
            width = name_width
        );
        if let Some(path) = &r.model_path {
            println!("      {}", style(path).dim());
        }
    }
    Ok(())
}

pub async fn handle_models_local_rm(model: String) -> Result<()> {
    let source = resolve_model_source(&model)
        .map_err(|e| anyhow::anyhow!("Unknown local model '{}': {}", model, e))?;

    // If this exact model is the one the current process's sidecar is running,
    // stop it first so the files aren't in use. This only affects a server this
    // CLI spawned; a server owned by the desktop app or biorouterd is untouched.
    let sidecar = llamacpp_sidecar::global();
    if sidecar.status().await.model.as_deref() == Some(model.as_str()) {
        sidecar.stop().await;
    }

    let deleted = llamacpp_sidecar::delete_model_cache(&source.hf_spec)?;
    if deleted {
        println!(
            "  {} removed the Hugging Face cache for {}",
            style("✓").green(),
            style(&model).cyan()
        );
    } else {
        println!(
            "  {} nothing to remove for {}: it was not in the Hugging Face fallback cache",
            style("·").dim(),
            style(&model).cyan()
        );
        if source.ollama_name.is_some() {
            println!(
                "      {}",
                style("if it was pulled via Ollama, remove it with `ollama rm <name>`").dim()
            );
        }
    }
    Ok(())
}

pub async fn handle_models_local_pull(model: String) -> Result<()> {
    use std::time::Duration;

    let source = resolve_model_source(&model)
        .map_err(|e| anyhow::anyhow!("Unknown local model '{}': {}", model, e))?;

    let spin = cliclack::spinner();
    spin.start(format!(
        "Downloading {} (first run can take a while)…",
        model
    ));

    let sidecar = llamacpp_sidecar::global();
    if let Err(e) = sidecar.ensure(&model, &source).await {
        spin.stop(style(format!("Could not start Llama Server: {e}")).red());
        return Err(anyhow::anyhow!(e));
    }

    // Foreground-wait for readiness so `pull` is scriptable: it returns only
    // once the model is fully downloaded and the server answered /health.
    let ready = sidecar.wait_ready(Duration::from_secs(3600)).await;

    // Leave the machine clean: `pull` pre-populates the shared on-disk model
    // cache (which the desktop daemon also reads), it is not meant to leave a
    // multi-GB server running after a one-shot CLI command.
    sidecar.stop().await;

    match ready {
        Ok(_) => {
            spin.stop(style(format!("{} is downloaded and cached", model)).green());
            Ok(())
        }
        Err(e) => {
            spin.stop(style(format!("{model} did not become ready: {e}")).red());
            Err(anyhow::anyhow!(e))
        }
    }
}
