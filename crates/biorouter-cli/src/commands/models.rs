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

use biorouter::providers::llamacpp::{
    default_model_name, resolve_chat_model_source, resolve_model_source, MODEL_CATALOG,
};
use biorouter::providers::llamacpp_sidecar::{self, ModelCacheStatus, ModelSource};
use std::path::PathBuf;

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

/// Which on-disk copy the sidecar would launch a catalog model from, and its
/// path, in the sidecar's own launch order (`launch_source_in_dir`): the
/// catalog's Hugging Face file first, then a GGUF in Ollama's store.
///
/// This used to ask Ollama first, so a machine holding both copies was told
/// "downloaded (ollama)" and shown the Ollama blob while the server it would
/// start loaded the Hugging Face file. The `/llamacpp/status` route's
/// `download_source` already follows the launch order; the terminal now says
/// the same thing the desktop does.
fn launch_source_label(
    hf_cached_file: Option<PathBuf>,
    ollama_blob: Option<PathBuf>,
) -> (&'static str, Option<PathBuf>) {
    match (hf_cached_file, ollama_blob) {
        (Some(path), _) => ("huggingface_cache", Some(path)),
        (None, Some(path)) => ("ollama", Some(path)),
        (None, None) => ("none", None),
    }
}

fn local_model_rows() -> Vec<LocalModelRow> {
    let default_model = default_model_name();
    MODEL_CATALOG
        .iter()
        .map(|e| {
            let (downloaded, source, model_path) = match resolve_model_source(e.name) {
                Ok(source) => {
                    let status = llamacpp_sidecar::model_source_cache_status(&source);
                    let (source_label, path) = launch_source_label(
                        llamacpp_sidecar::hf_cached_model_file(&source.hf_spec),
                        llamacpp_sidecar::model_source_path(&source),
                    );
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

/// The files `models local rm <name>` deletes: the model's OWN source, exactly.
/// A retired catalog name resolves to its former Hugging Face spec here, so
/// `rm gemma4` frees the Gemma 4 E4B download it left behind. Aliasing it the
/// way a chat does (see [`pull_target`]) would delete the default model's cache
/// instead.
fn rm_source(model: &str) -> Result<ModelSource> {
    resolve_model_source(model)
        .map_err(|e| anyhow::anyhow!("Unknown local model '{}': {}", model, e))
}

/// The model `models local pull <name>` downloads: the one a chat naming it
/// would run, resolved exactly as the `/llamacpp/ensure` route resolves it. A
/// retired catalog name therefore pulls the default model.
///
/// This used `resolve_model_source`, so `pull qwen3.5-4b` (the example in this
/// command's own help) downloaded and started the retired Qwen3.5 4B under that
/// name: a model no chat would ever run, since every chat naming it is served by
/// the default.
fn pull_target(model: &str) -> Result<(String, ModelSource)> {
    resolve_chat_model_source(model)
        .map_err(|e| anyhow::anyhow!("Unknown local model '{}': {}", model, e))
}

/// Whether `models local rm <requested>` would free anything once `pull` has
/// downloaded `target` in its place: the retired model's own Hugging Face
/// cache, asked of `hf_cache_status` (`llamacpp_sidecar::model_cache_status`).
///
/// ⚠ The Hugging Face cache only, never the whole source. A retired name such
/// as `gemma4` also names an Ollama tag (`gemma4:latest`), and this used to
/// count a GGUF in Ollama's store as "files still on disk". But `rm` deletes
/// only the Hugging Face cache, so the advice it printed freed nothing
/// ("nothing to remove for gemma4"), and the files it pointed at were the
/// user's own Ollama pull, which they may still be using through the Ollama
/// provider.
fn retired_cache_to_free(
    requested: &str,
    target: &str,
    hf_cache_status: impl Fn(&str) -> ModelCacheStatus,
) -> bool {
    target != requested
        && rm_source(requested)
            .is_ok_and(|own| hf_cache_status(&own.hf_spec) != ModelCacheStatus::NotDownloaded)
}

/// What `pull` says before downloading a model other than the one named: which
/// model a chat that names it runs, and, when the retired model's own Hugging
/// Face cache is still on disk ([`retired_cache_to_free`]), how to free it.
fn retired_pull_notice(
    requested: &str,
    target: &str,
    retired_files_on_disk: bool,
) -> Option<String> {
    if requested == target {
        return None;
    }
    let mut notice = format!(
        "{requested} was retired from the local model catalog. A chat that names it runs \
         {target}, so {target} is what this downloads."
    );
    if retired_files_on_disk {
        notice.push_str(&format!(
            " The retired model's own files are still on disk; run `biorouter models local rm \
             {requested}` to free them."
        ));
    }
    Some(notice)
}

pub async fn handle_models_local_rm(model: String) -> Result<()> {
    let source = rm_source(&model)?;

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

/// How `models local pull` reports (PROV-F11): a spinner on a terminal, and plain lines
/// anywhere else. The spinner draws on stderr and draws nothing at all when stderr is not a
/// terminal, so a script, CI job or `pull 2>log` heard nothing even on success. Plain, the
/// start goes to stderr and the result to stdout, where a script reads results; a failure is
/// the error `main` prints, once.
struct PullReport<Out: std::io::Write, Err: std::io::Write> {
    spinner: Option<cliclack::ProgressBar>,
    out: Out,
    err: Err,
}

impl PullReport<std::io::Stdout, std::io::Stderr> {
    fn start(model: &str) -> Self {
        use std::io::IsTerminal;
        let spinner = std::io::stderr().is_terminal().then(cliclack::spinner);
        Self::started(spinner, std::io::stdout(), std::io::stderr(), model)
    }
}

impl<Out: std::io::Write, Err: std::io::Write> PullReport<Out, Err> {
    fn started(
        spinner: Option<cliclack::ProgressBar>,
        out: Out,
        mut err: Err,
        model: &str,
    ) -> Self {
        let line = format!("Downloading {model} (first run can take a while)…");
        match &spinner {
            Some(spinner) => spinner.start(line),
            None => {
                let _ = writeln!(err, "{line}");
            }
        }
        Self { spinner, out, err }
    }

    fn done(mut self, model: &str) {
        let line = format!("{model} is downloaded and cached.");
        match self.spinner.take() {
            Some(spinner) => spinner.stop(style(line).green()),
            None => {
                let _ = writeln!(self.out, "{line}");
            }
        }
    }

    fn failed(mut self, line: String) {
        if let Some(spinner) = self.spinner.take() {
            spinner.stop(style(line).red());
        }
        let _ = self.err.flush();
    }
}

pub async fn handle_models_local_pull(requested: String) -> Result<()> {
    use std::time::Duration;

    let (model, source) = pull_target(&requested)?;
    let retired_files_on_disk =
        retired_cache_to_free(&requested, &model, llamacpp_sidecar::model_cache_status);
    if let Some(notice) = retired_pull_notice(&requested, &model, retired_files_on_disk) {
        eprintln!("{notice}");
    }

    let report = PullReport::start(&model);

    let sidecar = llamacpp_sidecar::global();
    if let Err(e) = sidecar.ensure(&model, &source).await {
        report.failed(format!("Could not start Llama Server: {e}"));
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
            report.done(&model);
            Ok(())
        }
        Err(e) => {
            report.failed(format!("{model} did not become ready: {e}"));
            Err(anyhow::anyhow!(e))
        }
    }
}

#[cfg(test)]
mod pull_report_tests {
    use super::PullReport;

    /// PROV-F11: off a terminal, `pull` says it started on stderr and that it finished on
    /// stdout, where a spinner drew nothing at all.
    #[test]
    fn a_pull_off_a_terminal_reports_in_plain_lines() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        PullReport::started(None, &mut out, &mut err, "qwen3.8-27b").done("qwen3.8-27b");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "Downloading qwen3.8-27b (first run can take a while)…\n"
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "qwen3.8-27b is downloaded and cached.\n"
        );

        // A failure prints nothing more here: the error `main` prints says it, once.
        let (mut out, mut err) = (Vec::new(), Vec::new());
        PullReport::started(None, &mut out, &mut err, "qwen3.8-27b")
            .failed("qwen3.8-27b did not become ready: timed out".into());
        assert!(out.is_empty());
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "Downloading qwen3.8-27b (first run can take a while)…\n"
        );
    }
}

#[cfg(test)]
mod local_model_tests {
    use super::{
        launch_source_label, pull_target, retired_cache_to_free, retired_pull_notice, rm_source,
    };
    use biorouter::providers::llamacpp::{retired_model, DEFAULT_MODEL, MODEL_CATALOG};
    use biorouter::providers::llamacpp_sidecar::ModelCacheStatus;
    use std::cell::RefCell;
    use std::path::PathBuf;

    /// `pull` with a retired catalog name downloads the model a chat naming it
    /// runs, never the retired model. `qwen3.5-4b` is the name this command's
    /// help used as its example until 2026-10.
    #[test]
    fn pulling_a_retired_name_downloads_the_model_a_chat_would_run() {
        let default = MODEL_CATALOG
            .iter()
            .find(|e| e.name == DEFAULT_MODEL)
            .expect("the default is in the catalog");
        for retired in [
            "qwen3.5-4b",
            "gemma4",
            "gemma4-e2b",
            "gemma4-12b",
            "qwen3.6",
        ] {
            let (model, source) = pull_target(retired).unwrap();
            assert_eq!(model, DEFAULT_MODEL, "{retired}");
            assert_eq!(source.hf_spec, default.hf_spec, "{retired}");
            assert_ne!(
                source.hf_spec,
                retired_model(retired).unwrap().hf_spec,
                "{retired} must not download its retired spec"
            );
        }
        // A current catalog name and a raw spec are pulled as named.
        let (model, source) = pull_target("gemma4-31b").unwrap();
        assert_eq!(model, "gemma4-31b");
        assert_eq!(source.hf_spec, "google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0");
        let (model, source) = pull_target("ggml-org/Qwen3.8-27B-GGUF:Q8_0").unwrap();
        assert_eq!(model, "ggml-org/Qwen3.8-27B-GGUF:Q8_0");
        assert_eq!(source.hf_spec, "ggml-org/Qwen3.8-27B-GGUF:Q8_0");
        assert!(pull_target("no-such-model").is_err());
    }

    /// `rm` keeps a retired name pointed at its OWN files: aliasing it as `pull`
    /// does would make `rm gemma4` delete the default model's cache.
    #[test]
    fn removing_a_retired_name_targets_its_own_files() {
        let source = rm_source("gemma4").unwrap();
        assert_eq!(source.hf_spec, "google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0");
        let source = rm_source("qwen3.5-4b").unwrap();
        assert_eq!(source.hf_spec, "unsloth/Qwen3.5-4B-GGUF:Q4_K_M");
    }

    #[test]
    fn a_retired_pull_says_what_it_downloads_instead() {
        assert_eq!(
            retired_pull_notice("qwen3.8-27b", "qwen3.8-27b", false),
            None
        );
        let notice = retired_pull_notice("gemma4", "qwen3.8-27b", false).unwrap();
        assert!(notice.contains("gemma4 was retired"), "{notice}");
        assert!(
            notice.contains("qwen3.8-27b is what this downloads"),
            "{notice}"
        );
        assert!(!notice.contains("models local rm"), "{notice}");
        let notice = retired_pull_notice("gemma4", "qwen3.8-27b", true).unwrap();
        assert!(
            notice.contains("`biorouter models local rm gemma4`"),
            "{notice}"
        );
    }

    /// `pull gemma4` offers `rm gemma4` only when `rm` would delete something:
    /// the retired model's Hugging Face cache. Its Ollama copy (`gemma4:latest`)
    /// is the user's own pull, which `rm` never touches, so it never triggers the
    /// advice. The old check asked the whole source and counted that copy.
    #[test]
    fn the_rm_advice_follows_what_rm_deletes() {
        let asked = RefCell::new(Vec::new());
        let status = |answer: ModelCacheStatus| {
            let asked = &asked;
            move |spec: &str| {
                asked.borrow_mut().push(spec.to_string());
                answer
            }
        };
        let own = rm_source("gemma4").unwrap();
        assert!(
            own.ollama_name.is_some(),
            "gemma4 must also name an Ollama tag"
        );

        // Only an Ollama copy (or nothing) on disk: no Hugging Face cache, no advice.
        assert!(!retired_cache_to_free(
            "gemma4",
            DEFAULT_MODEL,
            status(ModelCacheStatus::NotDownloaded)
        ));
        assert_eq!(*asked.borrow(), std::slice::from_ref(&own.hf_spec));

        // A complete or partial Hugging Face download is what `rm` frees.
        for answer in [ModelCacheStatus::Downloaded, ModelCacheStatus::Partial] {
            assert!(retired_cache_to_free(
                "gemma4",
                DEFAULT_MODEL,
                status(answer)
            ));
        }

        // A model pulled as named is not retired: nothing is asked.
        asked.borrow_mut().clear();
        assert!(!retired_cache_to_free(
            DEFAULT_MODEL,
            DEFAULT_MODEL,
            status(ModelCacheStatus::Downloaded)
        ));
        assert!(asked.borrow().is_empty());
    }

    /// The label follows the sidecar's launch order: with both copies on disk
    /// the server starts from the Hugging Face file, so that is the source and
    /// the path the list shows.
    #[test]
    fn the_listed_source_is_the_one_the_sidecar_launches() {
        let hf = PathBuf::from("/cache/hub/models--ggml-org--Qwen3.8-27B-GGUF/x.gguf");
        let blob = PathBuf::from("/cache/blobs/sha256-abc");
        assert_eq!(
            launch_source_label(Some(hf.clone()), Some(blob.clone())),
            ("huggingface_cache", Some(hf.clone()))
        );
        assert_eq!(
            launch_source_label(Some(hf.clone()), None),
            ("huggingface_cache", Some(hf))
        );
        assert_eq!(
            launch_source_label(None, Some(blob.clone())),
            ("ollama", Some(blob))
        );
        assert_eq!(launch_source_label(None, None), ("none", None));
    }
}
