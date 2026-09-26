use crate::config::paths::Paths;
use crate::config::Config;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::base::{ModelInfo, ProviderType};
use crate::providers::ollama::OllamaProvider;
use crate::providers::openai::OpenAiProvider;
use anyhow::Result;
use include_dir::{include_dir, Dir};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use utoipa::ToSchema;

static FIXED_PROVIDERS: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/providers/declarative");

pub fn custom_providers_dir() -> std::path::PathBuf {
    Paths::config_dir().join("custom_providers")
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProviderEngine {
    OpenAI,
    Ollama,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeclarativeProviderConfig {
    pub name: String,
    pub engine: ProviderEngine,
    pub display_name: String,
    pub description: Option<String>,
    pub api_key_env: String,
    pub base_url: String,
    pub models: Vec<ModelInfo>,
    pub headers: Option<HashMap<String, String>>,
    pub timeout_seconds: Option<u64>,
    pub supports_streaming: Option<bool>,
}

impl DeclarativeProviderConfig {
    pub fn id(&self) -> &str {
        &self.name
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoadedProvider {
    pub config: DeclarativeProviderConfig,
    pub is_editable: bool,
}

static ID_GENERATION_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
// Acquired before Config's writer mutex in both provider mutations and extension
// credential purging, so a new provider cannot acquire a key between its sharing
// check and deletion. Raw edits by other processes remain outside these locks.
pub(super) static PROVIDER_CREDENTIAL_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub fn generate_id(display_name: &str) -> String {
    let _guard = ID_GENERATION_LOCK.lock().unwrap();

    let normalized = display_name.to_lowercase().replace(' ', "_");
    let base_id = format!("custom_{}", normalized);

    let custom_dir = custom_providers_dir();
    let mut candidate_id = base_id.clone();
    let mut counter = 1;

    while custom_dir.join(format!("{}.json", candidate_id)).exists() {
        candidate_id = format!("{}_{}", base_id, counter);
        counter += 1;
    }

    candidate_id
}

pub fn generate_api_key_name(id: &str) -> String {
    format!("{}_API_KEY", id.to_uppercase())
}

pub fn create_custom_provider(
    engine: &str,
    display_name: String,
    api_url: String,
    api_key: String,
    models: Vec<String>,
    supports_streaming: Option<bool>,
    headers: Option<HashMap<String, String>>,
) -> Result<DeclarativeProviderConfig> {
    let _credential_guard = PROVIDER_CREDENTIAL_LOCK.lock().unwrap();
    let id = generate_id(&display_name);
    let api_key_name = generate_api_key_name(&id);

    let config = Config::global();
    config.set_secret(&api_key_name, &api_key)?;

    let model_infos: Vec<ModelInfo> = models
        .into_iter()
        .map(|name| ModelInfo::new(name, 128000))
        .collect();

    let provider_config = DeclarativeProviderConfig {
        name: id.clone(),
        engine: match engine {
            "openai_compatible" => ProviderEngine::OpenAI,
            "anthropic_compatible" => ProviderEngine::Anthropic,
            "ollama_compatible" => ProviderEngine::Ollama,
            _ => return Err(anyhow::anyhow!("Invalid provider type: {}", engine)),
        },
        display_name: display_name.clone(),
        description: Some(format!("Custom {} provider", display_name)),
        api_key_env: api_key_name,
        base_url: api_url,
        models: model_infos,
        headers,
        timeout_seconds: None,
        supports_streaming,
    };

    let custom_providers_dir = custom_providers_dir();
    std::fs::create_dir_all(&custom_providers_dir)?;

    let json_content = serde_json::to_string_pretty(&provider_config)?;
    let file_path = custom_providers_dir.join(format!("{}.json", id));
    std::fs::write(file_path, json_content)?;

    Ok(provider_config)
}

pub fn update_custom_provider(
    id: &str,
    provider_type: &str,
    display_name: String,
    api_url: String,
    api_key: String,
    models: Vec<String>,
    supports_streaming: Option<bool>,
) -> Result<()> {
    let _credential_guard = PROVIDER_CREDENTIAL_LOCK.lock().unwrap();
    let loaded_provider = load_provider(id)?;
    let existing_config = loaded_provider.config;
    let editable = loaded_provider.is_editable;

    let config = Config::global();
    if !api_key.is_empty() {
        config.set_secret(&existing_config.api_key_env, &api_key)?;
    }

    if editable {
        let model_infos: Vec<ModelInfo> = models
            .into_iter()
            .map(|name| ModelInfo::new(name, 128000))
            .collect();

        let updated_config = DeclarativeProviderConfig {
            name: id.to_string(),
            engine: match provider_type {
                "openai_compatible" => ProviderEngine::OpenAI,
                "anthropic_compatible" => ProviderEngine::Anthropic,
                "ollama_compatible" => ProviderEngine::Ollama,
                _ => return Err(anyhow::anyhow!("Invalid provider type: {}", provider_type)),
            },
            display_name,
            description: existing_config.description,
            api_key_env: existing_config.api_key_env,
            base_url: api_url,
            models: model_infos,
            headers: existing_config.headers,
            timeout_seconds: existing_config.timeout_seconds,
            supports_streaming,
        };

        let file_path = custom_providers_dir().join(format!("{}.json", id));
        let json_content = serde_json::to_string_pretty(&updated_config)?;
        std::fs::write(file_path, json_content)?;
    }
    Ok(())
}

pub fn remove_custom_provider(id: &str) -> Result<()> {
    let _credential_guard = PROVIDER_CREDENTIAL_LOCK.lock().unwrap();
    let config = Config::global();
    let api_key_name = generate_api_key_name(id);
    let _ = config.delete_secret(&api_key_name);

    let custom_providers_dir = custom_providers_dir();
    let file_path = custom_providers_dir.join(format!("{}.json", id));

    if file_path.exists() {
        std::fs::remove_file(file_path)?;
    }

    Ok(())
}

pub fn load_provider(id: &str) -> Result<LoadedProvider> {
    let custom_file_path = custom_providers_dir().join(format!("{}.json", id));

    if custom_file_path.exists() {
        let content = std::fs::read_to_string(&custom_file_path)?;
        let config: DeclarativeProviderConfig = serde_json::from_str(&content)?;
        return Ok(LoadedProvider {
            config,
            is_editable: true,
        });
    }

    for file in FIXED_PROVIDERS.files() {
        if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = file
            .contents_utf8()
            .ok_or_else(|| anyhow::anyhow!("Failed to read file as UTF-8: {:?}", file.path()))?;

        let config: DeclarativeProviderConfig = serde_json::from_str(content)?;
        if config.name == id {
            return Ok(LoadedProvider {
                config,
                is_editable: false,
            });
        }
    }

    Err(anyhow::anyhow!("Provider not found: {}", id))
}
pub fn load_custom_providers(dir: &Path) -> Result<Vec<DeclarativeProviderConfig>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    std::fs::read_dir(dir)?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "json").then_some(path)
        })
        .map(|path| {
            let content = std::fs::read_to_string(&path)?;
            serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))
        })
        .collect()
}

fn load_fixed_providers() -> Result<Vec<DeclarativeProviderConfig>> {
    let mut res = Vec::new();
    for file in FIXED_PROVIDERS.files() {
        if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = file
            .contents_utf8()
            .ok_or_else(|| anyhow::anyhow!("Failed to read file as UTF-8: {:?}", file.path()))?;

        let config: DeclarativeProviderConfig = serde_json::from_str(content)?;
        res.push(config)
    }

    Ok(res)
}

/// Read every declarative credential reference without relying on the cached
/// registry, which may predate a provider configuration edit.
pub(super) fn credential_references() -> Result<Vec<(String, String)>> {
    let mut providers: Vec<DeclarativeProviderConfig> = Vec::new();
    let directory = custom_providers_dir();
    match std::fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                    providers.push(serde_json::from_str(&std::fs::read_to_string(path)?)?);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    providers.extend(load_fixed_providers()?);
    Ok(providers
        .into_iter()
        .map(|provider| (provider.api_key_env, provider.name))
        .collect())
}

pub fn register_declarative_providers(
    registry: &mut crate::providers::provider_registry::ProviderRegistry,
) -> Result<()> {
    let dir = custom_providers_dir();
    let custom_providers = load_custom_providers(&dir)?;
    let fixed_providers = load_fixed_providers()?;
    for config in fixed_providers {
        register_declarative_provider(registry, config, ProviderType::Declarative);
    }

    for config in custom_providers {
        register_declarative_provider(registry, config, ProviderType::Custom);
    }

    Ok(())
}

pub fn register_declarative_provider(
    registry: &mut crate::providers::provider_registry::ProviderRegistry,
    config: DeclarativeProviderConfig,
    provider_type: ProviderType,
) {
    let config_clone = config.clone();

    match config.engine {
        ProviderEngine::OpenAI => {
            registry.register_with_name::<OpenAiProvider, _>(
                &config,
                provider_type,
                move |model| OpenAiProvider::from_custom_config(model, config_clone.clone()),
            );
        }
        ProviderEngine::Ollama => {
            registry.register_with_name::<OllamaProvider, _>(
                &config,
                provider_type,
                move |model| OllamaProvider::from_custom_config(model, config_clone.clone()),
            );
        }
        ProviderEngine::Anthropic => {
            registry.register_with_name::<AnthropicProvider, _>(
                &config,
                provider_type,
                move |model| AnthropicProvider::from_custom_config(model, config_clone.clone()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A malformed JSON in `providers/declarative/` fails
    /// `load_fixed_providers` wholesale, silently dropping every fixed
    /// provider — this test is the guard that keeps the bundled set parsing.
    #[test]
    fn fixed_providers_parse_and_include_moonshot() {
        let providers = load_fixed_providers().expect("bundled declarative providers must parse");

        let moonshot = providers
            .iter()
            .find(|p| p.name == "moonshot")
            .expect("moonshot.json is bundled");
        assert!(matches!(moonshot.engine, ProviderEngine::OpenAI));
        assert_eq!(moonshot.display_name, "Moonshot AI (Kimi)");
        assert_eq!(moonshot.api_key_env, "MOONSHOT_API_KEY");
        assert_eq!(moonshot.base_url, "https://api.moonshot.ai/v1");
        assert_eq!(moonshot.supports_streaming, Some(true));

        // Newest-first: the first entry becomes the UI's default model. Kimi
        // K3 (GA 2026-07-16) is Moonshot's flagship and its named replacement
        // for every discontinued model; kimi-k2.5 was discontinued on
        // 2026-08-31 and now answers "model not found" (platform.kimi.ai,
        // read 2026-09-25).
        let names: Vec<&str> = moonshot.models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "kimi-k3",
                "kimi-k2.7-code",
                "kimi-k2.7-code-highspeed",
                "kimi-k2.6"
            ]
        );

        // Real token costs so pricing flows from provider metadata
        // (pricing_from_provider_metadata) with no pricing.rs entry.
        // Direct-platform rates per MTok (platform.kimi.ai, 2026-09-25):
        // k3 $3.00/$15.00, k2.7-code-highspeed $1.90/$8.00, k2.7-code and
        // k2.6 $0.95/$4.00 — distinct from OpenRouter's rates for the same
        // models.
        for model in &moonshot.models {
            // K3 is the one 1M-window Kimi (1,048,576); the K2 line is 256k.
            let want_context = if model.name == "kimi-k3" {
                1_048_576
            } else {
                262_144
            };
            assert_eq!(model.context_limit, want_context, "{}", model.name);
            let input = model.input_token_cost.expect("input cost set");
            let output = model.output_token_cost.expect("output cost set");
            let (want_in, want_out) = match model.name.as_str() {
                "kimi-k3" => (3.00 / 1_000_000.0, 15.00 / 1_000_000.0),
                "kimi-k2.7-code-highspeed" => (1.90 / 1_000_000.0, 8.00 / 1_000_000.0),
                _ => (0.95 / 1_000_000.0, 4.00 / 1_000_000.0),
            };
            assert!((input - want_in).abs() < 1e-15, "{}", model.name);
            assert!((output - want_out).abs() < 1e-15, "{}", model.name);

            // Vision: per Moonshot's platform docs ("Use the Kimi Vision
            // Model", platform.kimi.ai, verified 2026-07 and again
            // 2026-09-25), kimi-k2.6 and kimi-k2.7-code accept image input in
            // png/jpeg/webp/gif, K3 takes image and video, and
            // kimi-k2.7-code-highspeed is listed under "Multi-modal Model".
            // Without this declaration the metadata reports None and the
            // desktop rejects image attachments.
            assert_eq!(
                model.supports_vision,
                Some(true),
                "{}: must declare vision",
                model.name
            );
            let mimes = model
                .supported_input_mime_types
                .as_ref()
                .unwrap_or_else(|| panic!("{}: image MIME types declared", model.name));
            for mime in ["image/png", "image/jpeg", "image/webp", "image/gif"] {
                assert!(mimes.contains(&mime.to_string()), "{}: {mime}", model.name);
            }
        }
    }

    /// The sync pricing table (`providers::pricing`, used by the CLI cost
    /// line and `/config/pricing`) and this declarative metadata (preferred
    /// by the async resolved path) must agree for every model EITHER side
    /// ships — a drift silently forks cost reports between the two paths.
    ///
    /// Bidirectional and full-field: every JSON model must be priced
    /// identically on the sync path (rates, currency, cache rates, context),
    /// and every sync-table entry must still exist in the JSON — so removing
    /// or repricing a model on one side only, or forking the currency,
    /// fails here instead of shipping divergent prices.
    #[test]
    fn moonshot_sync_pricing_matches_declarative_metadata() {
        let providers = load_fixed_providers().expect("bundled declarative providers must parse");
        let moonshot = providers
            .iter()
            .find(|p| p.name == "moonshot")
            .expect("moonshot.json is bundled");

        assert!(!moonshot.models.is_empty());
        // Direction 1: every JSON model is priced on the sync path with the
        // exact same full pricing record.
        for model in &moonshot.models {
            let sync = crate::providers::pricing::provider_model_pricing("moonshot", &model.name)
                .unwrap_or_else(|| panic!("{} must be priced on the sync path too", model.name));
            let meta_in = model.input_token_cost.expect("input cost set");
            let meta_out = model.output_token_cost.expect("output cost set");
            assert!(
                (sync.input_token_cost - meta_in).abs() < 1e-15,
                "{}: sync input {} != metadata {}",
                model.name,
                sync.input_token_cost,
                meta_in
            );
            assert!(
                (sync.output_token_cost - meta_out).abs() < 1e-15,
                "{}: sync output {} != metadata {}",
                model.name,
                sync.output_token_cost,
                meta_out
            );
            assert_eq!(
                sync.currency,
                model.currency.clone().unwrap_or_else(|| "$".to_string()),
                "{}: the two paths must bill in the same currency",
                model.name
            );
            // Neither source carries Moonshot cache rates today. If either
            // side ever grows one, the other must learn it in the same
            // change — a one-sided cache rate forks cached-turn costs.
            assert_eq!(
                sync.cache_read_cost, None,
                "{}: sync cache-read rate has no metadata counterpart",
                model.name
            );
            assert_eq!(
                sync.cache_write_cost, None,
                "{}: sync cache-write rate has no metadata counterpart",
                model.name
            );
            assert_eq!(
                sync.context_length,
                u32::try_from(model.context_limit).ok(),
                "{}",
                model.name
            );
        }

        // Direction 2: every sync-table entry still exists in moonshot.json.
        // Removing a model from the JSON while the hardcoded table keeps
        // pricing it would leave the sync path serving a phantom model.
        for &(name, _, _, _) in crate::providers::pricing::MOONSHOT_SYNC_PRICING {
            assert!(
                moonshot.models.iter().any(|m| m.name == name),
                "{name}: priced in pricing::MOONSHOT_SYNC_PRICING but missing from \
                 moonshot.json; remove it from the sync table or restore it in the JSON"
            );
        }
    }

    /// `display_name` is what the provider catalog PRINTS, so a note-to-self
    /// left in one ships to every user. `groq.json` carried
    /// `"display_name": "Groq (d)"` — a leftover marker (the other four
    /// bundled providers are clean), and the Public tab of the catalog listed
    /// **"Groq (d)"** through at least v1.90.2.
    ///
    /// The shape is what is banned, not the one string: a trailing
    /// parenthesised **single letter** is a marker, never a name. Real
    /// parentheses in a display name are words and stay legal — `Moonshot AI
    /// (Kimi)` is the bundled proof, and it is asserted here so a future
    /// tightening of this rule has to notice it.
    #[test]
    fn no_bundled_display_name_carries_a_single_letter_marker() {
        let providers = load_fixed_providers().expect("bundled declarative providers must parse");
        assert!(
            providers.len() >= 5,
            "expected the bundled set to load; got {}",
            providers.len()
        );

        for provider in &providers {
            let name = provider.display_name.trim();
            assert!(
                !name.is_empty(),
                "{}: display_name must not be empty",
                provider.name
            );
            // `rsplit_once`, not an index: slicing a `&str` by a byte offset can
            // split a multi-byte character, which is what `clippy::string_slice`
            // is there to stop. A display name is user-visible text and can hold
            // any character at all.
            if let Some(inner) = name
                .strip_suffix(')')
                .and_then(|rest| rest.rsplit_once('(').map(|(_, inner)| inner))
            {
                assert!(
                    inner.chars().count() != 1,
                    "{}: display_name {name:?} ends in a parenthesised single letter — that is a \
                     leftover marker, not a name (groq.json shipped \"Groq (d)\")",
                    provider.name
                );
            }
        }

        // The two ends of the rule, pinned by name so neither can drift.
        let groq = providers
            .iter()
            .find(|p| p.name == "groq")
            .expect("groq.json is bundled");
        assert_eq!(groq.display_name, "Groq");
        let moonshot = providers
            .iter()
            .find(|p| p.name == "moonshot")
            .expect("moonshot.json is bundled");
        assert_eq!(
            moonshot.display_name, "Moonshot AI (Kimi)",
            "a multi-letter parenthetical is a name, not a marker, and stays legal"
        );
    }

    fn bundled(providers: &[DeclarativeProviderConfig], name: &str) -> Vec<String> {
        providers
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("{name}.json is bundled"))
            .models
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    /// The bundled catalogs as of the 2026-09-25 refresh. The first entry is
    /// the provider's default (`register_with_name` takes `models.first()`),
    /// and the desktop's model switcher preselects it too, so the order is
    /// load-bearing, not cosmetic.
    ///
    /// Every id retired upstream is named with its date, because a retired id
    /// that stays listed is worse than a missing one: the picker offers it and
    /// the first request fails.
    #[test]
    fn bundled_catalogs_list_current_models_default_first() {
        let providers = load_fixed_providers().expect("bundled declarative providers must parse");

        let cases: &[(&str, &[&str], &[&str])] = &[
            (
                // deepseek-flash is DeepSeek-V4.1-Flash (GA 2026-09-10), the id
                // DeepSeek tells users to adopt. deepseek-chat and
                // deepseek-reasoner were discontinued 2026-07-24; V4-Flash was
                // retired 2026-09-10 and its name is only temporarily routed to
                // V4.1-Flash (api-docs.deepseek.com, read 2026-09-24).
                "custom_deepseek",
                &["deepseek-flash", "deepseek-v4-pro"],
                &["deepseek-chat", "deepseek-reasoner", "deepseek-v4-flash"],
            ),
            (
                // groq/compound and compound-mini were decommissioned
                // 2026-09-21; both Llama models shut down for free and
                // developer tiers on 2026-08-16 and are enterprise-only now.
                // qwen/qwen3.8-27b (preview) replaces qwen/qwen3.6-27b.
                "groq",
                &[
                    "openai/gpt-oss-120b",
                    "qwen/qwen3.8-27b",
                    "openai/gpt-oss-20b",
                    "openai/gpt-oss-safeguard-20b",
                ],
                &[
                    "groq/compound",
                    "groq/compound-mini",
                    "llama-3.1-8b-instant",
                    "llama-3.3-70b-versatile",
                ],
            ),
            (
                // Mercury 2.5 (GA 2026-09-08) is Inception's most capable
                // production model; Mercury Coder is no longer offered to new
                // users.
                "inception",
                &["mercury-2.5", "mercury-2"],
                &["mercury-coder"],
            ),
            (
                // Mistral Medium 3.1 retired 2026-08-31; Devstral 2 and
                // Magistral Medium 1.2 retired 2026-07-31. Mistral names
                // Medium 3.5 as the alternative for all three.
                "mistral",
                &[
                    "mistral-medium-3-5",
                    "mistral-large-2512",
                    "mistral-small-2603",
                    "ministral-14b-2512",
                    "ministral-8b-2512",
                    "ministral-3b-2512",
                    "mistral-medium-latest",
                    "codestral-2508",
                ],
                &[
                    "mistral-medium-2508",
                    "devstral-2512",
                    "magistral-medium-2509",
                ],
            ),
            (
                "moonshot",
                &[
                    "kimi-k3",
                    "kimi-k2.7-code",
                    "kimi-k2.7-code-highspeed",
                    "kimi-k2.6",
                ],
                &["kimi-k2.5"],
            ),
        ];

        for (provider, want, retired) in cases {
            let names = bundled(&providers, provider);
            assert_eq!(names, *want, "{provider}: catalog or default drifted");
            for id in *retired {
                assert!(
                    !names.iter().any(|n| n == id),
                    "{provider}: {id} is retired upstream and must not be offered"
                );
            }
        }

        // Each bundled window must be the registry's own entry for that id —
        // not an approximation, and not one inherited from a substring
        // pattern. `tests/context_windows.rs` checks the same thing across
        // every provider; this keeps the failure next to the JSON.
        for provider in &providers {
            for model in &provider.models {
                assert!(
                    crate::model::ModelConfig::has_declared_context_window(&model.name),
                    "{}/{}: no MODEL_CONTEXT_WINDOWS entry",
                    provider.name,
                    model.name
                );
                assert_eq!(
                    model.context_limit,
                    crate::model::ModelConfig::context_window_for(&model.name),
                    "{}/{}",
                    provider.name,
                    model.name
                );
            }
        }
    }

    /// Which bundled models take images, per each vendor's own model page
    /// (read 2026-09-24/25). A model left out of this table must not claim
    /// vision, and one in it must declare the MIME types the desktop uses to
    /// accept an attachment — `supports_vision` alone leaves the picker to
    /// guess.
    #[test]
    fn bundled_vision_declarations_follow_vendor_docs() {
        let providers = load_fixed_providers().expect("bundled declarative providers must parse");

        let vision: &[(&str, &str)] = &[
            // DeepSeek's pricing table lists vision for deepseek-flash and
            // "Not supported" for deepseek-v4-pro.
            ("custom_deepseek", "deepseek-flash"),
            // Groq: text plus up to 3 images per request.
            ("groq", "qwen/qwen3.8-27b"),
            // Mistral documents these as multimodal; codestral-2508 is
            // text-only. mistral-medium-latest is an alias of Medium 3.5.
            ("mistral", "mistral-medium-3-5"),
            ("mistral", "mistral-large-2512"),
            ("mistral", "mistral-small-2603"),
            ("mistral", "ministral-14b-2512"),
            ("mistral", "ministral-8b-2512"),
            ("mistral", "ministral-3b-2512"),
            ("mistral", "mistral-medium-latest"),
            ("moonshot", "kimi-k3"),
            ("moonshot", "kimi-k2.7-code"),
            ("moonshot", "kimi-k2.7-code-highspeed"),
            ("moonshot", "kimi-k2.6"),
        ];

        for provider in &providers {
            for model in &provider.models {
                let expected = vision
                    .iter()
                    .any(|(p, m)| *p == provider.name && *m == model.name);
                if expected {
                    assert_eq!(
                        model.supports_vision,
                        Some(true),
                        "{}/{} takes images",
                        provider.name,
                        model.name
                    );
                    let mimes = model
                        .supported_input_mime_types
                        .as_ref()
                        .unwrap_or_else(|| {
                            panic!(
                                "{}/{}: image MIME types declared",
                                provider.name, model.name
                            )
                        });
                    for mime in ["image/png", "image/jpeg"] {
                        assert!(
                            mimes.iter().any(|m| m == mime),
                            "{}/{}: {mime}",
                            provider.name,
                            model.name
                        );
                    }
                } else {
                    assert_ne!(
                        model.supports_vision,
                        Some(true),
                        "{}/{} is text-only",
                        provider.name,
                        model.name
                    );
                }
            }
        }
    }

    /// Kimi K3 fixes `temperature` at 1.0 and `top_p` at 0.95 and rejects any
    /// other value; thinking is always on and `reasoning_effort` takes only
    /// low/high/max (platform.kimi.ai, read 2026-09-25). The OpenAI engine
    /// this provider runs on sends `temperature` only when the operator set
    /// `BIOROUTER_TEMPERATURE`, never sends `top_p`, and never sends a
    /// `reasoning_effort` to a Kimi model — so the default request is one K3
    /// accepts. Pinned here so a default sampling value added to the engine
    /// has to meet this model.
    #[test]
    fn kimi_k3_default_request_sends_no_sampling_overrides() {
        let model = crate::model::ModelConfig::new_or_fail("kimi-k3").with_temperature(None);
        let payload = crate::providers::formats::openai::create_request(
            &model,
            "system",
            &[],
            &[],
            &crate::providers::utils::ImageFormat::OpenAi,
            true,
        )
        .expect("request builds");

        assert_eq!(payload["model"], "kimi-k3");
        for key in ["temperature", "top_p", "top_k", "reasoning_effort"] {
            assert!(
                payload.get(key).is_none(),
                "kimi-k3 rejects a non-default {key}; got {payload}"
            );
        }
    }
}
