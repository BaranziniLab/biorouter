//! Live model catalogs for bundled declarative providers, Baseten today.
//!
//! `providers/declarative/baseten.json` is a snapshot: its models and prices
//! change only when a release does. Baseten's own catalog (`GET /v1/models`)
//! needs an API key, so a scheduled job in `deploy-landing.yml` reads it with a
//! key held in the `baseten-feed` GitHub Environment and publishes the fields
//! Biorouter uses to [`BASETEN_CATALOG_URL`], next to `registry.json`. This
//! module reads that file with no key and no user data on the request.
//!
//! Resolution, from most to least current:
//! 1. the feed, which the daemon fetches only while Baseten is configured
//!    ([`spawn_refresher`]);
//! 2. the last good copy on disk (`<state>/provider-catalogs/baseten.json`),
//!    which is also what a CLI process with no daemon reads;
//! 3. the bundled JSON and `pricing::BASETEN_SYNC_PRICING`.
//!
//! A catalog fetched before the bundled one was read
//! ([`BUNDLED_CATALOG_AS_OF`]) counts as absent. A release therefore always
//! outranks a feed or cached copy that predates it, so a feed that stopped
//! updating (a revoked key, a response the builder refuses) cannot hide a model
//! a later release ships or bill its old prices over the new ones.
//!
//! What a current catalog may change, and what it may not:
//! - **Prices** (input, output, cached input) win over the bundled ones for
//!   `baseten`. They are keyed by provider, so they reach no other provider. A
//!   zero input or output price is read as unknown, not as free, so it cannot
//!   switch off a dollar budget.
//! - **The model list** shown for `baseten` is the feed's, minus models
//!   Biorouter cannot drive ([`LiveCatalog::advertised`]). An empty result
//!   keeps the bundled list rather than emptying the picker.
//! - **Context windows** reach only Baseten's own models, and only slugs the
//!   static `MODEL_CONTEXT_WINDOWS` lacks ([`stated_window`]). That table is
//!   keyed by model name alone and some slugs are shared (`openai/gpt-oss-120b`
//!   is also Groq's), so a static row always wins, and a feed window never
//!   reaches another provider's model of the same name.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};
use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::config::paths::Paths;
use crate::model::ModelConfig;
use crate::providers::base::ModelInfo;
use crate::providers::pricing::ProviderModelPricing;

pub const BASETEN_CATALOG_URL: &str = "https://biorouter.ucsf.edu/provider-catalogs/baseten.json";
const PROVIDER: &str = "baseten";
/// The day `providers/declarative/baseten.json` and
/// `pricing::BASETEN_SYNC_PRICING` were last read from Baseten. Move it in the
/// same change as either of them: a catalog fetched before this day is older
/// than what the release ships, and is ignored.
const BUNDLED_CATALOG_AS_OF: &str = "2026-10-18";
const CACHE_SUBPATH: &str = "provider-catalogs/baseten.json";
const SCHEMA_VERSION: u32 = 1;

// The bounds below are mirrored, under the same names, by
// `landing/scripts/build-provider-catalog.mjs`, so the builder never publishes
// a feed this parser refuses; `tests::the_feed_builder_mirrors_these_bounds`
// holds the two together.
const MAX_BYTES: usize = 256 * 1024;
const MAX_MODELS: usize = 512;
const MAX_ID_BYTES: usize = 128;
const MIN_CONTEXT: usize = 4096;
const MAX_CONTEXT: usize = 16 * 1024 * 1024;
/// Sanity ceiling on a per-token price: $1,000 per million tokens. A value
/// above it is a malformed feed, not a price.
const MAX_TOKEN_COST: f64 = 1e-3;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the daemon asks whether a fetch is due. Configuring Baseten in a
/// running app is noticed within this long.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// How old the catalog in force may get before the daemon fetches again. The
/// feed itself is rebuilt daily, and the shared daemon can run for days.
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

/// A model must hold Biorouter's instructions plus tool schemas (~31-40k
/// tokens) with room to work; SID-1's 32,768 cannot. The same floor as
/// `ollama::OLLAMA_AGENT_CONTEXT_SIZE` and the Llama Server sidecar's.
const MIN_AGENT_CONTEXT: usize = 65_536;
/// Models that return HTTP 400 on a multi-turn request whose assistant turns
/// lack `reasoning_content`, which only the `moonshot` provider replays
/// (`providers::openai::replays_reasoning_content`).
const NEEDS_REASONING_REPLAY: &[&str] = &["reflection/Beam-1.0"];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LiveCatalogError {
    #[error("catalog is {0} bytes; limit is {MAX_BYTES}")]
    TooLarge(usize),
    #[error("catalog is invalid: {0}")]
    Invalid(String),
    #[error("catalog fetched at {0} predates the bundled one ({BUNDLED_CATALOG_AS_OF})")]
    Stale(String),
    #[error("catalog fetch failed: {0}")]
    Fetch(String),
    #[error("provider registry not rebuilt with the new catalog: {0}")]
    Registry(String),
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

impl LiveModel {
    /// Input and output rates, unless either is zero. The feed has no way to
    /// say "free" that a broken upstream response could not also say, and a
    /// zero rate would switch off the dollar budget for the model, so zero
    /// means unknown.
    fn rates(&self) -> Option<(f64, f64)> {
        (self.input_token_cost > 0.0 && self.output_token_cost > 0.0)
            .then_some((self.input_token_cost, self.output_token_cost))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveCatalog {
    pub fetched_at: DateTime<Utc>,
    pub models: Vec<LiveModel>,
}

// Unknown fields are ignored rather than refused, so a field the builder adds
// later does not make every released daemon throw the whole feed away. A change
// an older parser would misread bumps `schema_version` instead.
#[derive(Deserialize)]
struct RawCatalog {
    schema_version: u32,
    provider: String,
    source: String,
    fetched_at: String,
    models: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
    context_length: u64,
    input_modalities: Vec<String>,
    supported_features: Vec<String>,
    pricing: RawPricing,
}

/// Prices stay strings, as Baseten writes them, so the feed never reformats a
/// float on the way through.
#[derive(Deserialize)]
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
        if raw.source.is_empty() {
            return invalid("source is required".to_string());
        }
        let Ok(fetched_at) = DateTime::parse_from_rfc3339(&raw.fetched_at) else {
            return invalid(format!("fetched_at `{}`", raw.fetched_at));
        };
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
            fetched_at: fetched_at.with_timezone(&Utc),
            models,
        })
    }

    /// Whether this catalog is older than the one the release ships, in which
    /// case the release's own data stands.
    fn predates_bundled(&self) -> bool {
        self.fetched_at.date_naive() < bundled_as_of()
    }

    /// Prices for `model`, matched case-blind because the pricing paths
    /// lowercase before lookup. `None` when the feed's rate is zero
    /// ([`LiveModel::rates`]), so the bundled price, if any, applies.
    pub fn pricing(&self, model: &str) -> Option<ProviderModelPricing> {
        let found = self
            .models
            .iter()
            .find(|candidate| candidate.id.eq_ignore_ascii_case(model))?;
        let (input, output) = found.rates()?;
        Some(ProviderModelPricing {
            input_token_cost: input,
            output_token_cost: output,
            cache_read_cost: found.cache_read_cost,
            cache_write_cost: None,
            currency: "$".to_string(),
            context_length: u32::try_from(found.context_length).ok(),
        })
    }

    /// The window this catalog states for exactly `model` (registry ids are
    /// case-sensitive), when the static registry has no row for the slug. A
    /// static row always wins; see the module comment.
    pub fn stated_window(&self, model: &str) -> Option<usize> {
        if ModelConfig::registry_context_window(model).is_some() {
            return None;
        }
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
            ModelConfig::registry_context_window(&live.id).unwrap_or(live.context_length)
        };
        let apply = |mut info: ModelInfo, live: &LiveModel| {
            if let Some((input, output)) = live.rates() {
                info.input_token_cost = Some(input);
                info.output_token_cost = Some(output);
                info.currency = Some("$".to_string());
            }
            if live.vision {
                info.with_vision()
            } else {
                info.supported_input_mime_types = None;
                info.without_vision()
            }
        };

        let mut merged: Vec<ModelInfo> = bundled
            .into_iter()
            .filter_map(|info| {
                let live = advertised.iter().find(|live| live.id == info.name)?;
                Some(apply(info, live))
            })
            .collect();
        let mut added: Vec<&LiveModel> = advertised
            .into_iter()
            .filter(|live| !merged.iter().any(|info| info.name == live.id))
            .collect();
        added.sort_by(|a, b| a.id.cmp(&b.id));
        for live in added {
            merged.push(apply(ModelInfo::new(live.id.clone(), window(live)), live));
        }
        merged
    }
}

fn bundled_as_of() -> NaiveDate {
    NaiveDate::parse_from_str(BUNDLED_CATALOG_AS_OF, "%Y-%m-%d")
        .expect("BUNDLED_CATALOG_AS_OF is a YYYY-MM-DD date")
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

/// The Baseten catalog last loaded, whatever its age; [`current`] decides
/// whether it is in force. Seeded from the last good copy on disk the first
/// time anything asks. Unit tests never read the disk, so a developer's cached
/// feed cannot change what they assert; `tests/baseten_live_catalog.rs` seeds
/// one on purpose, in its own process.
static BASETEN: Lazy<RwLock<Option<Arc<LiveCatalog>>>> = Lazy::new(|| {
    #[cfg(not(test))]
    let seeded = read_cached(&cache_path());
    #[cfg(test)]
    let seeded = None;
    RwLock::new(seeded)
});

/// Set when a rebuild of the provider registry failed after the catalog
/// changed, so the next fetch retries it even when the feed has not moved.
static REGISTRY_STALE: AtomicBool = AtomicBool::new(false);

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

/// The live catalog in force for `provider`, if any. Only `baseten` has one,
/// and one older than the bundled catalog is not in force.
pub fn current(provider: &str) -> Option<Arc<LiveCatalog>> {
    if !provider.eq_ignore_ascii_case(PROVIDER) {
        return None;
    }
    let catalog = BASETEN.read().ok()?.clone()?;
    if catalog.predates_bundled() {
        static SAID: Once = Once::new();
        SAID.call_once(|| {
            tracing::info!(
                "ignoring the Baseten catalog fetched at {}: it predates the one this release ships",
                catalog.fetched_at
            );
        });
        return None;
    }
    Some(catalog)
}

/// Live prices for `(provider, model)`, ahead of every bundled table.
pub(crate) fn pricing(provider: &str, model: &str) -> Option<ProviderModelPricing> {
    current(provider)?.pricing(model)
}

/// The window `provider`'s live catalog states for `model`, for the one case
/// a feed window is used: a Baseten model the static registry has no row for.
/// `OpenAiProvider::from_custom_config` applies it unless the user pinned a
/// limit.
pub fn stated_window(provider: &str, model: &str) -> Option<usize> {
    current(provider)?.stated_window(model)
}

/// `bundled`, or the live list for `provider` when one is in force.
pub fn merged_models(provider: &str, bundled: Vec<ModelInfo>) -> Vec<ModelInfo> {
    match current(provider) {
        Some(catalog) => catalog.merge_models(bundled),
        None => bundled,
    }
}

/// While Baseten is configured, fetch the feed whenever the last attempt is
/// [`REFRESH_EVERY`] old, checking every [`CHECK_EVERY`]. With Baseten not
/// configured the daemon makes no request at all. Failures are logged and
/// leave the current catalog in place.
pub fn spawn_refresher() {
    tokio::spawn(async {
        let mut attempted: Option<Instant> = None;
        loop {
            let due = attempted.is_none_or(|at| at.elapsed() >= REFRESH_EVERY);
            if due && baseten_configured() {
                match refresh_once().await {
                    Ok(true) => tracing::info!("Baseten model catalog updated"),
                    Ok(false) => tracing::debug!("Baseten model catalog unchanged"),
                    Err(error) => tracing::info!("Baseten model catalog not refreshed: {error}"),
                }
                attempted = Some(Instant::now());
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Baseten counts as configured once its key is saved, the same test the
/// provider catalog uses (`routes::utils::keys_are_saved`). The key gates the
/// fetch and is never sent with it: the feed needs none. Named as a literal so
/// `destination_keys`' scan can see and classify it.
fn baseten_configured() -> bool {
    crate::config::Config::global()
        .get_secret::<String>("BASETEN_API_KEY")
        .is_ok()
}

/// One fetch. `Ok(true)` when the catalog changed and the provider registry
/// was rebuilt with it.
async fn refresh_once() -> Result<bool, LiveCatalogError> {
    let bytes = fetch().await?;
    let loaded = BASETEN.read().ok().and_then(|slot| slot.clone());
    match apply_fetched(&bytes, &cache_path(), loaded.as_deref())? {
        Refreshed::Unchanged => {
            if REGISTRY_STALE.load(Ordering::Relaxed) {
                rebuild_registry().await?;
            }
            Ok(false)
        }
        Refreshed::Updated(catalog) => {
            if let Ok(mut slot) = BASETEN.write() {
                *slot = Some(Arc::new(catalog));
            }
            rebuild_registry().await?;
            Ok(true)
        }
    }
}

/// What one fetched feed does to the catalog in force.
#[derive(Debug, PartialEq)]
enum Refreshed {
    Unchanged,
    Updated(LiveCatalog),
}

/// Validate `bytes`, refuse a feed older than the bundled catalog, compare it
/// with `loaded` and write the cache when it changed. The comparison is by
/// models alone: `fetched_at` moves every day whether or not anything else
/// did, and rebuilding the registry for a timestamp would make "updated" mean
/// nothing. A loaded catalog that is itself older than the bundled one is
/// replaced even when its models match, so the cache carries a date that is in
/// force again.
fn apply_fetched(
    bytes: &[u8],
    cache: &Path,
    loaded: Option<&LiveCatalog>,
) -> Result<Refreshed, LiveCatalogError> {
    let catalog = LiveCatalog::from_bytes(bytes)?;
    if catalog.predates_bundled() {
        return Err(LiveCatalogError::Stale(catalog.fetched_at.to_rfc3339()));
    }
    if loaded.is_some_and(|loaded| !loaded.predates_bundled() && loaded.models == catalog.models) {
        return Ok(Refreshed::Unchanged);
    }
    if let Err(error) = atomic_write(cache, bytes) {
        tracing::warn!("could not cache the Baseten catalog: {error}");
    }
    Ok(Refreshed::Updated(catalog))
}

/// Rebuild every declarative registry entry, so the picker sees the new list.
async fn rebuild_registry() -> Result<(), LiveCatalogError> {
    let result = crate::providers::refresh_custom_providers().await;
    REGISTRY_STALE.store(result.is_err(), Ordering::Relaxed);
    result.map_err(|error| LiveCatalogError::Registry(error.to_string()))
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
            r#"{{"id":"{id}","context_length":{context},
                "input_modalities":{modalities},"supported_features":{features},
                "pricing":{{"prompt":"{}","completion":"{}","input_cache_read":"{}"}}}}"#,
            prices[0], prices[1], prices[2]
        )
    }

    fn feed_at(fetched_at: &str, models: &[String]) -> Vec<u8> {
        format!(
            r#"{{"schema_version":1,"provider":"baseten","source":"https://inference.baseten.co/v1/models",
                "fetched_at":"{fetched_at}","models":[{}]}}"#,
            models.join(",")
        )
        .into_bytes()
    }

    fn feed(models: &[String]) -> Vec<u8> {
        feed_at("2026-10-19T00:00:00Z", models)
    }

    fn one_model() -> String {
        model(
            "a/b",
            131_072,
            false,
            true,
            ["0.000001", "0.000002", "0.0000001"],
        )
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
        assert_eq!(
            catalog.fetched_at,
            DateTime::parse_from_rfc3339("2026-10-19T00:00:00Z").unwrap()
        );
    }

    #[test]
    fn a_malformed_feed_is_refused_whole() {
        let replace = |from: &str, to: &str| {
            String::from_utf8(feed(&[one_model()]))
                .unwrap()
                .replace(from, to)
                .into_bytes()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "wrong provider",
                replace(r#""provider":"baseten""#, r#""provider":"groq""#),
            ),
            (
                "future schema",
                replace(r#""schema_version":1"#, r#""schema_version":2"#),
            ),
            (
                "fetched_at that is not RFC 3339",
                replace("2026-10-19T00:00:00Z", "yesterday"),
            ),
            ("no models", feed(&[])),
            ("duplicate id", feed(&[one_model(), one_model()])),
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

    /// A field the builder adds later must not make a released daemon refuse
    /// the whole feed; `schema_version` is for changes an old parser would
    /// misread.
    #[test]
    fn unknown_fields_are_ignored() {
        let text = String::from_utf8(feed(&[one_model()]))
            .unwrap()
            .replace(r#""models":"#, r#""extra":true,"models":"#)
            .replace(r#""context_length":"#, r#""name":"A B","context_length":"#);
        let catalog = LiveCatalog::from_bytes(text.as_bytes()).expect("unknown fields parse");
        assert_eq!(catalog.models[0].id, "a/b");
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
        // The premise: Groq's static row for gpt-oss-120b exists and differs
        // from the feed's 128,072, and GLM 5.2 has no row at all. Without both
        // halves this test would pass whichever window `merge_models` chose.
        assert_eq!(
            ModelConfig::registry_context_window("openai/gpt-oss-120b"),
            Some(131_072)
        );
        assert!(ModelConfig::registry_context_window("zai-org/GLM-5.2").is_none());

        let merged = sample().merge_models(bundled());
        let window = |name: &str| {
            merged
                .iter()
                .find(|m| m.name == name)
                .unwrap()
                .context_limit
        };
        assert_eq!(window("openai/gpt-oss-120b"), 131_072);
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

    /// A feed that says a paid model costs nothing is not believed: the
    /// bundled price stands for a bundled model, and a feed-only model has no
    /// price rather than a price of zero, which would switch off the budget.
    #[test]
    fn a_zero_price_is_unknown_not_free() {
        let catalog = LiveCatalog::from_bytes(&feed(&[
            model("zai-org/GLM-5.3", 1_048_576, true, true, ["0", "0", "0"]),
            model("new/model", 1_048_576, false, true, ["0.000001", "0", "0"]),
        ]))
        .unwrap();
        assert!(catalog.pricing("zai-org/GLM-5.3").is_none());
        assert!(catalog.pricing("new/model").is_none());

        let merged = catalog.merge_models(bundled());
        assert_eq!(merged[0].name, "zai-org/GLM-5.3");
        assert_eq!(
            merged[0].input_token_cost,
            Some(1.4e-6),
            "bundled price kept"
        );
        let added = merged.iter().find(|m| m.name == "new/model").unwrap();
        assert_eq!(added.input_token_cost, None);
        assert_eq!(added.output_token_cost, None);
    }

    #[test]
    fn a_window_is_stated_only_for_a_slug_the_registry_lacks() {
        let catalog = sample();
        assert_eq!(catalog.stated_window("zai-org/GLM-5.2"), Some(1_048_576));
        // Case-sensitive, like the registry.
        assert_eq!(catalog.stated_window("zai-org/glm-5.2"), None);
        // A shared slug keeps its static row, not the feed's 128,072.
        assert_eq!(catalog.stated_window("openai/gpt-oss-120b"), None);
        // No live catalog is in force in unit tests, so no provider gets one.
        assert_eq!(stated_window("baseten", "zai-org/GLM-5.2"), None);
    }

    #[test]
    fn a_catalog_older_than_the_bundled_one_is_not_in_force() {
        let day_before =
            LiveCatalog::from_bytes(&feed_at("2026-10-17T23:59:59Z", &[one_model()])).unwrap();
        assert!(day_before.predates_bundled());
        let same_day =
            LiveCatalog::from_bytes(&feed_at("2026-10-18T00:00:00Z", &[one_model()])).unwrap();
        assert!(!same_day.predates_bundled());
    }

    #[test]
    fn a_fetch_older_than_the_bundled_catalog_is_refused_and_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("baseten.json");
        let old = feed_at("2026-09-01T00:00:00Z", &[one_model()]);
        assert!(matches!(
            apply_fetched(&old, &cache, None),
            Err(LiveCatalogError::Stale(_))
        ));
        assert!(!cache.exists());
    }

    #[test]
    fn a_malformed_fetch_leaves_the_cache_alone() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("baseten.json");
        assert!(apply_fetched(b"{not json", &cache, None).is_err());
        assert!(!cache.exists());
    }

    #[test]
    fn a_new_feed_is_cached_and_a_redated_one_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("baseten.json");

        let first = feed(&[one_model()]);
        let Refreshed::Updated(loaded) = apply_fetched(&first, &cache, None).unwrap() else {
            panic!("a first feed is an update");
        };
        assert_eq!(std::fs::read(&cache).unwrap(), first);

        // The next day's feed: same models, new `fetched_at`.
        let next_day = feed_at("2026-10-20T00:00:00Z", &[one_model()]);
        assert_eq!(
            apply_fetched(&next_day, &cache, Some(&loaded)).unwrap(),
            Refreshed::Unchanged
        );
        assert_eq!(std::fs::read(&cache).unwrap(), first, "not rewritten");

        // A price change is an update, and is cached.
        let repriced = feed_at(
            "2026-10-21T00:00:00Z",
            &[model(
                "a/b",
                131_072,
                false,
                true,
                ["0.000003", "0.000002", "0.0000001"],
            )],
        );
        assert!(matches!(
            apply_fetched(&repriced, &cache, Some(&loaded)).unwrap(),
            Refreshed::Updated(_)
        ));
        assert_eq!(std::fs::read(&cache).unwrap(), repriced);
    }

    /// A cached copy older than this release's bundled catalog is replaced by
    /// a current fetch even when the models match, so the cache is in force
    /// again for a CLI process that reads only the disk.
    #[test]
    fn a_stale_loaded_catalog_is_replaced_by_a_matching_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("baseten.json");
        let stale =
            LiveCatalog::from_bytes(&feed_at("2026-09-01T00:00:00Z", &[one_model()])).unwrap();
        let current_feed = feed(&[one_model()]);
        assert!(matches!(
            apply_fetched(&current_feed, &cache, Some(&stale)).unwrap(),
            Refreshed::Updated(_)
        ));
        assert_eq!(std::fs::read(&cache).unwrap(), current_feed);
    }

    /// The feed `landing/scripts/build-provider-catalog.mjs` built from
    /// Baseten's real `/v1/models` response (`fixtures/baseten-models.raw.json`,
    /// read 2026-10-20). The script's own suite checks that rebuilding the raw
    /// response gives this file back, and this test that the parser accepts
    /// it, so the two halves of the format meet here. Regenerate the fixture
    /// with the script, never by hand.
    #[test]
    fn the_published_feed_format_parses() {
        let bytes = include_bytes!("../../../../landing/scripts/fixtures/baseten-catalog.json");
        let catalog = LiveCatalog::from_bytes(bytes).expect("the script's output parses");
        assert_eq!(catalog.models.len(), 18);
        assert!(
            !catalog.predates_bundled(),
            "the fixture must postdate {BUNDLED_CATALOG_AS_OF}"
        );

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

    /// The bundled snapshot must itself pass the rules the feed is filtered
    /// by, or a feed listing exactly the bundled models would drop one.
    #[test]
    fn every_bundled_model_is_one_the_feed_would_advertise() {
        for info in crate::config::declarative_providers::bundled_models("baseten") {
            assert!(
                info.context_limit >= MIN_AGENT_CONTEXT,
                "{}: window {} is below the agent floor",
                info.name,
                info.context_limit
            );
            assert!(
                !NEEDS_REASONING_REPLAY.contains(&info.name.as_str()),
                "{} needs reasoning replay",
                info.name
            );
        }
    }

    /// The builder publishes only what this parser accepts, because it checks
    /// the same bounds under the same names. Read them from the script itself,
    /// the way `app_folder` pins its constants against the desktop's copy.
    #[test]
    fn the_feed_builder_mirrors_these_bounds() {
        let script = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../landing/scripts/build-provider-catalog.mjs"
        ))
        .expect("the feed builder is in the tree");
        for (line, value) in [
            ("export const MIN_CONTEXT = 4096;", MIN_CONTEXT),
            ("export const MAX_CONTEXT = 16 * 1024 * 1024;", MAX_CONTEXT),
            ("export const MAX_BYTES = 256 * 1024;", MAX_BYTES),
            ("export const MAX_ID_BYTES = 128;", MAX_ID_BYTES),
            ("export const MAX_MODELS = 512;", MAX_MODELS),
        ] {
            assert!(
                script.contains(line),
                "build-provider-catalog.mjs must declare `{line}`"
            );
            let declared = line
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .trim_end_matches(';')
                .split(" * ")
                .map(|factor| factor.parse::<usize>().expect("a numeric factor"))
                .product::<usize>();
            assert_eq!(declared, value, "{line}");
        }
        assert!(script.contains("export const MAX_TOKEN_COST = 1e-3;"));
        assert_eq!(MAX_TOKEN_COST, 1e-3);
    }

    #[test]
    fn the_bundled_date_is_a_date() {
        assert_eq!(bundled_as_of().to_string(), BUNDLED_CATALOG_AS_OF);
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
        atomic_write(&path, &feed(&[one_model()])).unwrap();
        assert_eq!(read_cached(&path).unwrap().models.len(), 1);
    }
}
