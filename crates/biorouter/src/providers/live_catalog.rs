//! Live model catalogs for bundled declarative providers — Baseten today.
//!
//! `providers/declarative/baseten.json` is a snapshot: its models and prices
//! change only when a release does. Baseten's own catalog (`GET /v1/models`)
//! needs an API key, so a scheduled job in `deploy-landing.yml` reads it with a
//! key held in a GitHub Environment secret and publishes the fields Biorouter
//! uses to [`BASETEN_CATALOG_URL`], next to `registry.json`. This module reads
//! that file with no key and no user data on the request.
//!
//! Resolution, from most to least current:
//! 1. the feed, fetched by the daemon at start and every [`REFRESH_EVERY`];
//! 2. the last good copy on disk (`<state>/provider-catalogs/baseten.json`),
//!    which is also what a CLI process with no daemon reads;
//! 3. the bundled JSON and `pricing::BASETEN_SYNC_PRICING`.
//!
//! What the feed may change, and what it may not:
//! - **Prices** (input, output, cached input) win over the bundled ones for
//!   `baseten`. They are keyed by provider, so they reach no other provider.
//! - **The model list** shown for `baseten` is the feed's, minus models
//!   Biorouter cannot drive ([`advertised`]). An empty result keeps the bundled
//!   list rather than emptying the picker.
//! - **Context windows** only fill gaps. `MODEL_CONTEXT_WINDOWS` is keyed by
//!   model name alone and some slugs are shared (`openai/gpt-oss-120b` is also
//!   Groq's), so a static row always wins and a feed window is used only for a
//!   slug the static table does not know.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::config::paths::Paths;
use crate::providers::base::ModelInfo;
use crate::providers::pricing::ProviderModelPricing;

pub const BASETEN_CATALOG_URL: &str = "https://biorouter.ucsf.edu/provider-catalogs/baseten.json";
const PROVIDER: &str = "baseten";
const CACHE_SUBPATH: &str = "provider-catalogs/baseten.json";
const SCHEMA_VERSION: u32 = 1;
const MAX_BYTES: usize = 256 * 1024;
const MAX_MODELS: usize = 512;
const MAX_ID_BYTES: usize = 128;
const MIN_CONTEXT: usize = 4_096;
const MAX_CONTEXT: usize = 16 * 1024 * 1024;
/// Sanity ceiling on a per-token price: $1,000 per million tokens. A value
/// above it is a malformed feed, not a price.
const MAX_TOKEN_COST: f64 = 1e-3;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The daemon outlives the app (the shared daemon can run for days), so one
/// fetch at start is not enough. The feed itself is rebuilt daily.
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

/// A model must hold Biorouter's instructions plus tool schemas (~31-40k
/// tokens) with room to work; SID-1's 32,768 cannot.
const MIN_AGENT_CONTEXT: usize = 65_536;
/// Models that return HTTP 400 on a multi-turn request whose assistant turns
/// lack `reasoning_content`, which only the `moonshot` provider replays
/// (`formats::openai::replays_reasoning_content`).
const NEEDS_REASONING_REPLAY: &[&str] = &["reflection/Beam-1.0"];

const VISION_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/webp",
    "image/gif",
];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LiveCatalogError {
    #[error("catalog is {0} bytes; limit is {MAX_BYTES}")]
    TooLarge(usize),
    #[error("catalog is invalid: {0}")]
    Invalid(String),
    #[error("catalog fetch failed: {0}")]
    Fetch(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveModel {
    pub id: String,
    pub context_length: usize,
    pub vision: bool,
    pub tools: bool,
    pub input_token_cost: f64,
    pub output_token_cost: f64,
    pub cache_read_cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveCatalog {
    pub fetched_at: String,
    pub models: Vec<LiveModel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    schema_version: u32,
    provider: String,
    source: String,
    fetched_at: String,
    models: Vec<RawModel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModel {
    id: String,
    #[allow(dead_code)]
    name: Option<String>,
    context_length: u64,
    #[allow(dead_code)]
    max_completion_tokens: Option<u64>,
    input_modalities: Vec<String>,
    supported_features: Vec<String>,
    pricing: RawPricing,
}

/// Prices stay strings, as Baseten writes them, so the feed never reformats a
/// float on the way through.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPricing {
    prompt: String,
    completion: String,
    input_cache_read: Option<String>,
}

impl LiveCatalog {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LiveCatalogError> {
        if bytes.len() > MAX_BYTES {
            return Err(LiveCatalogError::TooLarge(bytes.len()));
        }
        let raw: RawCatalog = serde_json::from_slice(bytes)
            .map_err(|error| LiveCatalogError::Invalid(error.to_string()))?;
        let invalid = |message: String| Err(LiveCatalogError::Invalid(message));

        if raw.schema_version != SCHEMA_VERSION {
            return invalid(format!("schema_version {}", raw.schema_version));
        }
        if raw.provider != PROVIDER {
            return invalid(format!("provider `{}`", raw.provider));
        }
        if raw.source.is_empty() || raw.fetched_at.is_empty() {
            return invalid("source and fetched_at are required".to_string());
        }
        if raw.models.is_empty() || raw.models.len() > MAX_MODELS {
            return invalid(format!("{} models", raw.models.len()));
        }

        let mut models = Vec::with_capacity(raw.models.len());
        for model in raw.models {
            if !valid_id(&model.id) {
                return invalid(format!("model id `{}`", model.id));
            }
            if models.iter().any(|seen: &LiveModel| seen.id == model.id) {
                return invalid(format!("duplicate model id `{}`", model.id));
            }
            let context_length = usize::try_from(model.context_length).unwrap_or(usize::MAX);
            if !(MIN_CONTEXT..=MAX_CONTEXT).contains(&context_length) {
                return invalid(format!("{}: context_length {context_length}", model.id));
            }
            let price = |value: &str| {
                parse_token_cost(value).ok_or_else(|| {
                    LiveCatalogError::Invalid(format!("{}: price `{value}`", model.id))
                })
            };
            models.push(LiveModel {
                input_token_cost: price(&model.pricing.prompt)?,
                output_token_cost: price(&model.pricing.completion)?,
                cache_read_cost: model
                    .pricing
                    .input_cache_read
                    .as_deref()
                    .map(price)
                    .transpose()?,
                vision: model.input_modalities.iter().any(|m| m == "image"),
                tools: model.supported_features.iter().any(|f| f == "tools"),
                context_length,
                id: model.id,
            });
        }
        Ok(Self {
            fetched_at: raw.fetched_at,
            models,
        })
    }

    /// Prices for `model`, matched case-blind because the pricing paths
    /// lowercase before lookup.
    pub fn pricing(&self, model: &str) -> Option<ProviderModelPricing> {
        let found = self
            .models
            .iter()
            .find(|candidate| candidate.id.eq_ignore_ascii_case(model))?;
        Some(ProviderModelPricing {
            input_token_cost: found.input_token_cost,
            output_token_cost: found.output_token_cost,
            cache_read_cost: found.cache_read_cost,
            cache_write_cost: None,
            currency: "$".to_string(),
            context_length: u32::try_from(found.context_length).ok(),
        })
    }

    /// The feed's window for exactly `model` (registry ids are case-sensitive).
    pub fn context_window(&self, model: &str) -> Option<usize> {
        self.models
            .iter()
            .find(|candidate| candidate.id == model)
            .map(|found| found.context_length)
    }

    /// The models Biorouter can drive: tool calling, a window that holds the
    /// agent's own prompt, and no reasoning-replay requirement.
    pub fn advertised(&self) -> impl Iterator<Item = &LiveModel> {
        self.models.iter().filter(|model| {
            model.tools
                && model.context_length >= MIN_AGENT_CONTEXT
                && !NEEDS_REASONING_REPLAY.contains(&model.id.as_str())
        })
    }

    /// The model list to register for the provider. Bundled models the feed
    /// still advertises keep their place (so the default stays first) and
    /// take the feed's prices and image support; bundled models the feed no
    /// longer lists are dropped; new advertised models follow, by id. A
    /// window comes from the static registry when it has the slug, else from
    /// the feed (see the module comment). Nothing advertised keeps `bundled`.
    pub fn merge_models(&self, bundled: Vec<ModelInfo>) -> Vec<ModelInfo> {
        let advertised: Vec<&LiveModel> = self.advertised().collect();
        if advertised.is_empty() {
            return bundled;
        }
        let window = |live: &LiveModel| {
            crate::model::ModelConfig::registry_context_window(&live.id)
                .unwrap_or(live.context_length)
        };
        let apply = |info: &mut ModelInfo, live: &LiveModel| {
            info.input_token_cost = Some(live.input_token_cost);
            info.output_token_cost = Some(live.output_token_cost);
            info.currency = Some("$".to_string());
            info.supports_vision = Some(live.vision);
            info.supported_input_mime_types = live
                .vision
                .then(|| VISION_MIME_TYPES.iter().map(|m| m.to_string()).collect());
        };

        let mut merged: Vec<ModelInfo> = bundled
            .into_iter()
            .filter_map(|mut info| {
                let live = advertised.iter().find(|live| live.id == info.name)?;
                apply(&mut info, live);
                Some(info)
            })
            .collect();
        let mut added: Vec<&LiveModel> = advertised
            .into_iter()
            .filter(|live| !merged.iter().any(|info| info.name == live.id))
            .collect();
        added.sort_by(|a, b| a.id.cmp(&b.id));
        for live in added {
            let mut info = ModelInfo::new(live.id.clone(), window(live));
            apply(&mut info, live);
            merged.push(info);
        }
        merged
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

fn parse_token_cost(value: &str) -> Option<f64> {
    let cost: f64 = value.trim().parse().ok()?;
    (cost.is_finite() && (0.0..=MAX_TOKEN_COST).contains(&cost)).then_some(cost)
}

// ── Process-wide state ─────────────────────────────────────────────────────

/// The current Baseten catalog. Seeded from the last good copy on disk the
/// first time anything asks; unit tests never read the disk, so a developer's
/// cached feed cannot change what they assert.
static BASETEN: Lazy<RwLock<Option<Arc<LiveCatalog>>>> = Lazy::new(|| {
    #[cfg(not(test))]
    let seeded = read_cached(&cache_path());
    #[cfg(test)]
    let seeded = None;
    RwLock::new(seeded)
});

fn cache_path() -> PathBuf {
    Paths::in_state_dir(CACHE_SUBPATH)
}

#[cfg_attr(test, allow(dead_code))]
fn read_cached(path: &Path) -> Option<Arc<LiveCatalog>> {
    let bytes = std::fs::read(path).ok()?;
    match LiveCatalog::from_bytes(&bytes) {
        Ok(catalog) => Some(Arc::new(catalog)),
        Err(error) => {
            tracing::warn!(
                "ignoring cached Baseten catalog at {}: {error}",
                path.display()
            );
            None
        }
    }
}

/// The live catalog for `provider`, if one is loaded. Only `baseten` has one.
pub fn current(provider: &str) -> Option<Arc<LiveCatalog>> {
    if !provider.eq_ignore_ascii_case(PROVIDER) {
        return None;
    }
    BASETEN.read().ok()?.clone()
}

/// Live prices for `(provider, model)`, ahead of every bundled table.
pub(crate) fn pricing(provider: &str, model: &str) -> Option<ProviderModelPricing> {
    current(provider)?.pricing(model)
}

/// A feed window for a slug, for [`crate::model::ModelConfig`]'s gap filling.
pub(crate) fn context_window(model: &str) -> Option<usize> {
    current(PROVIDER)?.context_window(model)
}

/// Every feed window, for the list the desktop resolves windows from.
pub(crate) fn context_windows() -> Vec<(String, usize)> {
    current(PROVIDER)
        .map(|catalog| {
            catalog
                .models
                .iter()
                .map(|model| (model.id.clone(), model.context_length))
                .collect()
        })
        .unwrap_or_default()
}

/// `bundled`, or the live list for `provider` when one is loaded.
pub(crate) fn merged_models(provider: &str, bundled: Vec<ModelInfo>) -> Vec<ModelInfo> {
    match current(provider) {
        Some(catalog) => catalog.merge_models(bundled),
        None => bundled,
    }
}

/// Fetch the feed now and every [`REFRESH_EVERY`] for the life of the daemon.
/// Failures are logged and leave the current catalog in place.
pub fn spawn_refresher() {
    tokio::spawn(async {
        loop {
            match refresh_once().await {
                Ok(true) => tracing::info!("Baseten model catalog updated"),
                Ok(false) => tracing::debug!("Baseten model catalog unchanged"),
                Err(error) => tracing::info!("Baseten model catalog not refreshed: {error}"),
            }
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

/// One fetch. `Ok(true)` when the catalog changed and the provider registry
/// was rebuilt with it.
async fn refresh_once() -> Result<bool, LiveCatalogError> {
    let bytes = fetch().await?;
    let catalog = LiveCatalog::from_bytes(&bytes)?;
    if current(PROVIDER).as_deref() == Some(&catalog) {
        return Ok(false);
    }
    if let Err(error) = atomic_write(&cache_path(), &bytes) {
        tracing::warn!("could not cache the Baseten catalog: {error}");
    }
    if let Ok(mut slot) = BASETEN.write() {
        *slot = Some(Arc::new(catalog));
    }
    // Rebuilds every declarative entry, so the picker sees the new list.
    if let Err(error) = crate::providers::refresh_custom_providers().await {
        tracing::warn!("provider registry not rebuilt after a catalog update: {error}");
    }
    Ok(true)
}

async fn fetch() -> Result<Vec<u8>, LiveCatalogError> {
    use futures::StreamExt;

    let fetch_error = |error: reqwest::Error| LiveCatalogError::Fetch(error.to_string());
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(fetch_error)?;
    let response = client
        .get(BASETEN_CATALOG_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::USER_AGENT,
            concat!("Biorouter/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .map_err(fetch_error)?
        .error_for_status()
        .map_err(fetch_error)?;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(fetch_error)?;
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(LiveCatalogError::TooLarge(bytes.len() + chunk.len()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("cache path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file_mut().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, context: u64, image: bool, tools: bool, prices: [&str; 3]) -> String {
        let modalities = if image {
            r#"["text","image"]"#
        } else {
            r#"["text"]"#
        };
        let features = if tools {
            r#"["tools","reasoning"]"#
        } else {
            r#"["reasoning"]"#
        };
        format!(
            r#"{{"id":"{id}","name":"{id}","context_length":{context},"max_completion_tokens":{context},
                "input_modalities":{modalities},"supported_features":{features},
                "pricing":{{"prompt":"{}","completion":"{}","input_cache_read":"{}"}}}}"#,
            prices[0], prices[1], prices[2]
        )
    }

    fn feed(models: &[String]) -> Vec<u8> {
        format!(
            r#"{{"schema_version":1,"provider":"baseten","source":"https://inference.baseten.co/v1/models",
                "fetched_at":"2026-10-19T00:00:00Z","models":[{}]}}"#,
            models.join(",")
        )
        .into_bytes()
    }

    /// Shaped like Baseten's `/v1/models` on 2026-10-18: one bundled model
    /// with a new price, one model new to Biorouter, Beam, SID-1 and a model
    /// without tool calling.
    fn sample() -> LiveCatalog {
        LiveCatalog::from_bytes(&feed(&[
            model(
                "zai-org/GLM-5.3",
                1_048_576,
                true,
                true,
                ["0.0000012", "0.000004", "0.00000012"],
            ),
            model(
                "zai-org/GLM-5.2",
                1_048_576,
                true,
                true,
                ["0.0000014", "0.0000044", "0.00000014"],
            ),
            model(
                "openai/gpt-oss-120b",
                128_072,
                false,
                true,
                ["0.0000001", "0.0000005", "0.0000001"],
            ),
            model(
                "reflection/Beam-1.0",
                1_048_576,
                false,
                true,
                ["0.0000013", "0.0000043", "0.00000013"],
            ),
            model(
                "sid/sid-1",
                32_768,
                false,
                true,
                ["0.00000075", "0.0000045", "0.000000075"],
            ),
            model(
                "example/no-tools",
                1_048_576,
                false,
                false,
                ["0.000001", "0.000001", "0.0000001"],
            ),
        ]))
        .expect("sample feed parses")
    }

    fn bundled() -> Vec<ModelInfo> {
        vec![
            ModelInfo::with_cost("zai-org/GLM-5.3", 1_048_576, 1.4e-6, 4.4e-6),
            ModelInfo::with_cost("moonshotai/Kimi-K3", 1_048_576, 3e-6, 15e-6),
            ModelInfo::with_cost("openai/gpt-oss-120b", 131_072, 1e-7, 5e-7),
        ]
    }

    #[test]
    fn a_feed_shaped_like_baseten_parses() {
        let catalog = sample();
        assert_eq!(catalog.models.len(), 6);
        let glm = &catalog.models[0];
        assert!(glm.vision && glm.tools);
        assert_eq!(glm.cache_read_cost, Some(1.2e-7));
    }

    #[test]
    fn a_malformed_feed_is_refused_whole() {
        let good = || {
            model(
                "a/b",
                131_072,
                false,
                true,
                ["0.000001", "0.000002", "0.0000001"],
            )
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("wrong provider", {
                let text = String::from_utf8(feed(&[good()])).unwrap();
                text.replace(r#""provider":"baseten""#, r#""provider":"groq""#)
                    .into_bytes()
            }),
            ("future schema", {
                let text = String::from_utf8(feed(&[good()])).unwrap();
                text.replace(r#""schema_version":1"#, r#""schema_version":2"#)
                    .into_bytes()
            }),
            ("unknown field", {
                let text = String::from_utf8(feed(&[good()])).unwrap();
                text.replace(r#""models":"#, r#""extra":true,"models":"#)
                    .into_bytes()
            }),
            ("no models", feed(&[])),
            ("duplicate id", feed(&[good(), good()])),
            (
                "id with a space",
                feed(&[model(
                    "a b",
                    131_072,
                    false,
                    true,
                    ["0.1e-6", "0.1e-6", "0.1e-6"],
                )]),
            ),
            (
                "negative price",
                feed(&[model(
                    "a/b",
                    131_072,
                    false,
                    true,
                    ["-0.000001", "0.000002", "0"],
                )]),
            ),
            (
                "absurd price",
                feed(&[model("a/b", 131_072, false, true, ["5", "0.000002", "0"])]),
            ),
            (
                "not a number",
                feed(&[model("a/b", 131_072, false, true, ["NaN", "0.000002", "0"])]),
            ),
            (
                "tiny window",
                feed(&[model(
                    "a/b",
                    1_000,
                    false,
                    true,
                    ["0.000001", "0.000002", "0"],
                )]),
            ),
        ];
        for (label, bytes) in cases {
            assert!(
                LiveCatalog::from_bytes(&bytes).is_err(),
                "{label} must be refused"
            );
        }
        let huge = vec![b' '; MAX_BYTES + 1];
        assert_eq!(
            LiveCatalog::from_bytes(&huge),
            Err(LiveCatalogError::TooLarge(MAX_BYTES + 1))
        );
    }

    #[test]
    fn only_models_biorouter_can_drive_are_advertised() {
        let catalog = sample();
        let ids: Vec<&str> = catalog.advertised().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            ["zai-org/GLM-5.3", "zai-org/GLM-5.2", "openai/gpt-oss-120b"]
        );
    }

    #[test]
    fn the_merged_list_keeps_bundled_order_drops_retired_and_appends_new() {
        let merged = sample().merge_models(bundled());
        let names: Vec<&str> = merged.iter().map(|m| m.name.as_str()).collect();
        // Kimi K3 is bundled but absent from this feed, so it is dropped.
        assert_eq!(
            names,
            ["zai-org/GLM-5.3", "openai/gpt-oss-120b", "zai-org/GLM-5.2"]
        );
        // The feed's price wins over the bundled one.
        assert_eq!(merged[0].input_token_cost, Some(1.2e-6));
        assert_eq!(merged[0].supports_vision, Some(true));
        assert!(merged[0].supported_input_mime_types.is_some());
        assert_eq!(merged[1].supports_vision, Some(false));
        assert_eq!(merged[1].supported_input_mime_types, None);
    }

    #[test]
    fn a_shared_slug_keeps_the_registry_window_and_a_new_slug_takes_the_feeds() {
        let merged = sample().merge_models(bundled());
        let window = |name: &str| {
            merged
                .iter()
                .find(|m| m.name == name)
                .unwrap()
                .context_limit
        };
        // Groq's row; the feed's 128,072 must not reach it.
        assert_eq!(window("openai/gpt-oss-120b"), 131_072);
        // Not in MODEL_CONTEXT_WINDOWS, so the feed's window is the model's.
        assert!(crate::model::ModelConfig::registry_context_window("zai-org/GLM-5.2").is_none());
        assert_eq!(window("zai-org/GLM-5.2"), 1_048_576);
    }

    #[test]
    fn a_feed_with_nothing_drivable_keeps_the_bundled_list() {
        let catalog = LiveCatalog::from_bytes(&feed(&[model(
            "sid/sid-1",
            32_768,
            false,
            true,
            ["0.00000075", "0.0000045", "0.000000075"],
        )]))
        .unwrap();
        assert_eq!(catalog.merge_models(bundled()), bundled());
    }

    #[test]
    fn prices_match_case_blind_and_carry_the_cache_rate() {
        let pricing = sample().pricing("zai-org/glm-5.3").unwrap();
        assert_eq!(pricing.input_token_cost, 1.2e-6);
        assert_eq!(pricing.output_token_cost, 4e-6);
        assert_eq!(pricing.cache_read_cost, Some(1.2e-7));
        assert_eq!(pricing.cache_write_cost, None);
        assert!(sample().pricing("zai-org/glm-9").is_none());
    }

    #[test]
    fn windows_match_case_sensitively() {
        let catalog = sample();
        assert_eq!(catalog.context_window("zai-org/GLM-5.2"), Some(1_048_576));
        assert_eq!(catalog.context_window("zai-org/glm-5.2"), None);
    }

    /// The feed `landing/scripts/build-provider-catalog.mjs` built from
    /// Baseten's real `/v1/models` on 2026-10-19. This parser and that script
    /// are the two halves of one format, in two languages; this is the only
    /// place both meet. Regenerate the fixture with the script, never by hand.
    #[test]
    fn the_published_feed_format_parses() {
        let bytes = include_bytes!("../../../../landing/scripts/fixtures/baseten-catalog.json");
        let catalog = LiveCatalog::from_bytes(bytes).expect("the script's output parses");
        assert_eq!(catalog.models.len(), 18);

        let advertised: Vec<&str> = catalog.advertised().map(|m| m.id.as_str()).collect();
        assert_eq!(
            advertised.len(),
            16,
            "all but Beam and SID-1: {advertised:?}"
        );
        assert!(!advertised.contains(&"reflection/Beam-1.0"));
        assert!(!advertised.contains(&"sid/sid-1"));

        // Every model the bundled baseten.json ships is in the feed, so
        // merging today drops none of them.
        let bundled = crate::config::declarative_providers::bundled_models("baseten");
        assert!(!bundled.is_empty());
        for info in &bundled {
            assert!(advertised.contains(&info.name.as_str()), "{}", info.name);
        }
        let merged = catalog.merge_models(bundled.clone());
        assert_eq!(merged[0].name, bundled[0].name, "the default stays first");
        assert_eq!(merged.len(), 16);
    }

    #[test]
    fn unit_tests_never_see_a_cached_feed() {
        assert!(current("baseten").is_none());
        assert!(current("groq").is_none());
    }

    #[test]
    fn a_corrupt_cache_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baseten.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(read_cached(&path).is_none());
        atomic_write(
            &path,
            &feed(&[model(
                "a/b",
                131_072,
                false,
                true,
                ["0.000001", "0.000002", "0.0000001"],
            )]),
        )
        .unwrap();
        assert_eq!(read_cached(&path).unwrap().models.len(), 1);
    }
}
