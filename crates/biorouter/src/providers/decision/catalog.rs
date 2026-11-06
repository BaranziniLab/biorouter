//! Which decision models exist, and how to open one.
//!
//! - **Baseten**: the `decision-models/` entries of the live Baseten feed
//!   (`providers::live_catalog`), else the three slugs this release knows.
//!   They are read here and never reach the chat picker: the feed's
//!   `advertised()` drops them already (no `tools`, and a 16,384-token window
//!   under the agent's floor), and nothing here changes that.
//! - **Ollama**: every pulled model whose `/api/show` capabilities include
//!   `decision` (System One), on the server `OLLAMA_HOST` names, the shipped
//!   default first. A copy Ollama refuses to load (a format it no longer
//!   supports) is found by a probe that loads nothing, and is listed last with
//!   the reason. A pulled model without that capability can still be opened by
//!   name with the `logprob` adapter (a yes/no checker such as
//!   bespoke-minicheck).
//!
//! Tiers are declared, never inherited: Baseten is Public with no affiliation,
//! as its chat provider is; an Ollama model takes the self-hosted rule
//! (loopback is Private and `Local`), and a cloud model (a `-cloud` tag, or a
//! `remote_host` in `/api/show`) is Public whatever the host, the same rule
//! `OllamaProvider::tier` applies.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

use crate::config::Config;
use crate::decision::error::{DecisionError, DecisionResult};
use crate::decision::model::{DecisionModel, DecisionModelEntry, DecisionModelSource};
use crate::decision::types::{DecisionAdapter, DecisionKindTag, DecisionModelRef};
use crate::privacy::ProviderTier;
use crate::providers::api_client::{ApiClient, AuthMethod, AuthProvider};
use crate::providers::formats::ollama::ModelFacts;
use crate::providers::live_catalog::{self, LiveCatalog};
use crate::providers::ollama::{resolve_ollama_base_url, OLLAMA_HOST};

use super::chat::OLLAMA_DEFAULT_DECISION_MODEL;
use super::logprob::LogprobAdapter;
use super::system_one::{
    ollama_load_refusal_sentence, ollama_refusal, OllamaRefusal, SystemOneAdapter, SYSTEM_ONE_PATH,
};

pub const BASETEN_DECISION_PROVIDER: &str = "baseten_decision";
pub const OLLAMA_DECISION_PROVIDER: &str = "ollama_decision";
/// The id prefix Baseten's catalog gives its decision models.
pub const DECISION_MODEL_PREFIX: &str = "decision-models/";
/// Every Baseten decision model's window (Baseten's docs).
pub const BASETEN_DECISION_CONTEXT: usize = 16_384;
/// The window assumed for an Ollama decision model that states none.
pub const OLLAMA_DEFAULT_DECISION_CONTEXT: usize = 8_192;
/// The largest window assumed for a model read through logprobs, so a model
/// trained on 262k tokens is not loaded with a cache that size for a one-token
/// answer.
const LOGPROB_MAX_CONTEXT: usize = 32_768;

const TAGS_TIMEOUT: Duration = Duration::from_secs(3);
const SHOW_TIMEOUT: Duration = Duration::from_secs(5);
/// How long one discovery answers `list` while the server lists the same
/// copies. A pull or a removal changes the list `/api/tags` returns, which is
/// asked every time, so it shows up at once.
const DISCOVERY_TTL: Duration = Duration::from_secs(60);
const SHOW_CONCURRENCY: usize = 4;

/// A decision model this release knows by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownDecisionModel {
    pub name: &'static str,
    pub slug: &'static str,
    pub context_tokens: usize,
}

/// Baseten's three, smallest first: an automatic choice takes the first that
/// is allowed, and joey-1 is the fastest and cheapest. Matches the live feed
/// fetched 2026-11-04 and PR #469's catalog.
pub const BASETEN_DECISION_MODELS: &[KnownDecisionModel] = &[
    KnownDecisionModel {
        name: "joey-1",
        slug: "decision-models/baseten-joey-2b-latest",
        context_tokens: BASETEN_DECISION_CONTEXT,
    },
    KnownDecisionModel {
        name: "elephant-1",
        slug: "decision-models/baseten-elephant-9b-latest",
        context_tokens: BASETEN_DECISION_CONTEXT,
    },
    KnownDecisionModel {
        name: "mammoth-1",
        slug: "decision-models/baseten-mammoth-27b-latest",
        context_tokens: BASETEN_DECISION_CONTEXT,
    },
];

const ALL_KINDS: [DecisionKindTag; 3] = [
    DecisionKindTag::Binary,
    DecisionKindTag::Choice,
    DecisionKindTag::Score,
];

/// The display name of a Baseten decision slug: the known alias, else the
/// slug without its prefix.
pub fn baseten_display_name(slug: &str) -> String {
    BASETEN_DECISION_MODELS
        .iter()
        .find(|known| known.slug == slug)
        .map(|known| known.name.to_string())
        .unwrap_or_else(|| {
            slug.strip_prefix(DECISION_MODEL_PREFIX)
                .unwrap_or(slug)
                .to_string()
        })
}

/// A decision model's name for a person: a Baseten model's alias
/// ([`baseten_display_name`]: joey-1, or the slug without its prefix), else
/// its model id. Records, the decision log and the wire keep the slug; only
/// what a person reads says the alias, and the alias still names the model
/// wherever a model is asked for.
pub fn decision_model_short_name(model: &DecisionModelRef) -> String {
    if model.provider == BASETEN_DECISION_PROVIDER {
        baseten_display_name(&model.model)
    } else {
        model.model.clone()
    }
}

/// [`decision_model_short_name`] behind its provider, as
/// `baseten_decision/joey-1`.
pub fn decision_model_display_label(model: &DecisionModelRef) -> String {
    format!("{}/{}", model.provider, decision_model_short_name(model))
}

/// A Baseten decision model entry.
pub fn baseten_entry(slug: &str, context_tokens: usize) -> DecisionModelEntry {
    DecisionModelEntry {
        provider: BASETEN_DECISION_PROVIDER.to_string(),
        model: slug.to_string(),
        display_name: baseten_display_name(slug),
        context_tokens,
        kinds: ALL_KINDS.to_vec(),
        // Baseten's chat provider is Public (`providers/tier_tests.rs`), and its
        // decision models run on the same service.
        tier: ProviderTier::Public,
        affiliation: None,
        adapter: DecisionAdapter::SystemOne,
        local: false,
        load_refusal: None,
    }
}

/// The decision models a catalog lists, or the known three when it lists none
/// (no feed yet, or a feed from before the launch).
pub fn baseten_entries_from(catalog: Option<&LiveCatalog>) -> Vec<DecisionModelEntry> {
    let mut from_feed: Vec<DecisionModelEntry> = catalog
        .map(|catalog| {
            catalog
                .models
                .iter()
                .filter(|model| model.id.starts_with(DECISION_MODEL_PREFIX))
                .map(|model| baseten_entry(&model.id, model.context_length))
                .collect()
        })
        .unwrap_or_default();
    if from_feed.is_empty() {
        return BASETEN_DECISION_MODELS
            .iter()
            .map(|known| baseten_entry(known.slug, known.context_tokens))
            .collect();
    }
    // Known models first, in their order, then any newer ones by id.
    from_feed.sort_by_key(|entry| {
        let rank = BASETEN_DECISION_MODELS
            .iter()
            .position(|known| known.slug == entry.model)
            .unwrap_or(usize::MAX);
        (rank, entry.model.clone())
    });
    from_feed
}

/// Baseten's decision models as the live catalog in force lists them.
pub fn baseten_entries() -> Vec<DecisionModelEntry> {
    baseten_entries_from(live_catalog::current("baseten").as_deref())
}

/// Whether a Baseten key resolves. Read through the secret store's per-process
/// cache, so this prompts no more than the chat provider does.
pub fn baseten_configured() -> bool {
    Config::global()
        .get_secret::<String>("BASETEN_API_KEY")
        .is_ok()
}

/// The Ollama server `OLLAMA_HOST` names, resolved exactly as the Ollama chat
/// provider and Ollama's own client resolve it.
pub fn ollama_base_url() -> DecisionResult<Url> {
    let host: String = Config::global()
        .get_param("OLLAMA_HOST")
        .unwrap_or_else(|_| OLLAMA_HOST.to_string());
    resolve_ollama_base_url(&host).map_err(|e| DecisionError::NoModel(e.to_string()))
}

/// What `/api/show` says that matters to a decision model.
#[derive(Debug, Clone, PartialEq)]
pub struct OllamaDecisionFacts {
    /// `capabilities` includes `decision`: served on `/v1/systemone`.
    pub decision: bool,
    /// `capabilities` includes `thinking`, so `think: false` may be sent.
    pub thinking: bool,
    /// Served from another host (an Ollama cloud model, or `remote_host`).
    pub remote: bool,
    /// The window: the Modelfile's `num_ctx`, else the trained window, else
    /// [`OLLAMA_DEFAULT_DECISION_CONTEXT`].
    pub context_tokens: usize,
}

/// `num_ctx` from `/api/show`'s `parameters` text (`num_ctx   16384`).
fn parameter_num_ctx(show: &Value) -> Option<usize> {
    show.get("parameters")?
        .as_str()?
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next()? == "num_ctx").then(|| parts.next()?.parse::<usize>().ok())?
        })
        .filter(|&n| n > 0)
}

/// Read `/api/show` for `name`. Uses the chat provider's own parser
/// ([`ModelFacts::from_show`]) for capabilities, the trained window and the
/// remote-host rule.
pub fn ollama_facts_from_show(name: &str, show: &Value) -> OllamaDecisionFacts {
    let facts = ModelFacts::from_show(show);
    OllamaDecisionFacts {
        decision: facts.has("decision"),
        thinking: facts.has("thinking"),
        remote: facts.remote || crate::providers::utils::is_ollama_cloud_model(name),
        context_tokens: parameter_num_ctx(show)
            .or(facts.trained_context)
            .unwrap_or(OLLAMA_DEFAULT_DECISION_CONTEXT),
    }
}

/// An Ollama entry for `name` on `base_url`.
pub fn ollama_entry(
    base_url: &Url,
    name: &str,
    facts: &OllamaDecisionFacts,
    adapter: DecisionAdapter,
) -> DecisionModelEntry {
    let (tier, affiliation) = if facts.remote {
        (ProviderTier::Public, None)
    } else {
        (
            crate::providers::self_hosted_tier(base_url.as_str()),
            crate::providers::self_hosted_affiliation(base_url.as_str()),
        )
    };
    let context_tokens = match adapter {
        DecisionAdapter::Logprob => facts.context_tokens.min(LOGPROB_MAX_CONTEXT),
        _ => facts.context_tokens,
    };
    DecisionModelEntry {
        provider: OLLAMA_DECISION_PROVIDER.to_string(),
        model: name.to_string(),
        display_name: name.to_string(),
        context_tokens,
        kinds: ALL_KINDS.to_vec(),
        tier,
        affiliation,
        adapter,
        local: tier == ProviderTier::Private,
        load_refusal: None,
    }
}

struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn get_auth_header(&self) -> anyhow::Result<(String, String)> {
        Ok(("X-No-Auth".to_string(), "true".to_string()))
    }
}

fn ollama_client(base_url: &Url, timeout: Duration) -> DecisionResult<ApiClient> {
    ApiClient::with_timeout(
        base_url.to_string(),
        AuthMethod::Custom(Box::new(NoAuth)),
        timeout,
    )
    .map_err(|e| DecisionError::NoModel(format!("Ollama client could not start: {e}")))
}

/// `/api/show` for one model, or `None` when the server does not describe it.
async fn show(client: &ApiClient, name: &str) -> Option<Value> {
    let probe = async {
        let response = client
            .response_post("api/show", &json!({ "model": name }))
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<Value>().await.ok()
    };
    tokio::time::timeout(SHOW_TIMEOUT, probe).await.ok()?
}

/// The facts for one model on `base_url`, asking the server.
pub async fn ollama_facts(
    base_url: &Url,
    name: &str,
) -> DecisionResult<Option<OllamaDecisionFacts>> {
    let client = ollama_client(base_url, SHOW_TIMEOUT)?;
    Ok(show(&client, name)
        .await
        .map(|show| ollama_facts_from_show(name, &show)))
}

/// Ollama decision models in the order an automatic choice tries them: the
/// ones Ollama loads before the ones it refused ([`DecisionModelEntry::load_refusal`]),
/// and within each, the shipped default ([`OLLAMA_DEFAULT_DECISION_MODEL`])
/// first, then by name. A name sort alone made the order an accident of
/// spelling, and put a copy Ollama refuses ahead of the default.
pub fn order_ollama_entries(entries: &mut [DecisionModelEntry]) {
    entries.sort_by(|a, b| {
        let rank = |entry: &DecisionModelEntry| {
            (
                entry.load_refusal.is_some(),
                entry.model != OLLAMA_DEFAULT_DECISION_MODEL,
            )
        };
        rank(a).cmp(&rank(b)).then_with(|| a.model.cmp(&b.model))
    });
}

/// One pulled model as `/api/tags` lists it.
struct PulledModel {
    name: String,
    /// What identifies this copy: the manifest digest, else the modified
    /// time. A re-pull changes it.
    version: Option<String>,
}

fn pulled_models(tags: &Value) -> Vec<PulledModel> {
    let Some(models) = tags.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|m| {
            let name = m.get("name").or_else(|| m.get("model"))?.as_str()?;
            let version = ["digest", "modified_at"]
                .iter()
                .find_map(|key| m.get(*key).and_then(Value::as_str))
                .filter(|v| !v.is_empty())
                .map(str::to_string);
            Some(PulledModel {
                name: name.to_string(),
                version,
            })
        })
        .collect()
}

async fn ollama_tags(client: &ApiClient) -> Option<Value> {
    let tags = async {
        let response = client.response_get("api/tags").await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<Value>().await.ok()
    };
    tokio::time::timeout(TAGS_TIMEOUT, tags)
        .await
        .ok()
        .flatten()
}

/// How long a load probe may take. Ollama answers one in a few milliseconds.
const LOAD_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// (base URL, model name, copy version) to the probe's answer: the refusal
/// sentence, or `None` for a copy Ollama loads.
type LoadProbeKey = (String, String, String);
static LOAD_PROBES: LazyLock<Mutex<HashMap<LoadProbeKey, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The sentence for a copy of `name` that the Ollama server at `base_url`
/// refuses to load ([`ollama_load_refusal_sentence`]), or `None`.
///
/// Asked with a System One request whose questions are empty, which Ollama
/// refuses either way: with the format refusal for a copy it cannot load,
/// which it checks first, or with its complaint that the questions are empty
/// for one it can. Neither loads the model (measured on Ollama 0.40.14:
/// `/api/ps` listed nothing before or after, and each answer took about 7 ms),
/// and the request carries no state.
///
/// Asked at most once per copy (`version`, the `/api/tags` digest) in this
/// process, and only a 400 is remembered: the format refusal, or any other
/// refusal of the empty questions, which comes only after the format check
/// passed. Any other outcome (a timeout, an unreachable server, a 404 or a
/// server fault) gives `None` and is asked again next time, so a model is
/// never marked on an ambiguous answer.
pub async fn ollama_load_refusal(
    base_url: &Url,
    name: &str,
    version: Option<&str>,
) -> Option<String> {
    let client = ollama_client(base_url, LOAD_PROBE_TIMEOUT).ok()?;
    probe_load_refusal(&client, base_url, name, version).await
}

async fn probe_load_refusal(
    client: &ApiClient,
    base_url: &Url,
    name: &str,
    version: Option<&str>,
) -> Option<String> {
    let key = version.map(|v| (base_url.to_string(), name.to_string(), v.to_string()));
    if let Some(key) = &key {
        if let Some(known) = LOAD_PROBES
            .lock()
            .ok()
            .and_then(|probes| probes.get(key).cloned())
        {
            return known;
        }
    }
    let ask = async {
        let probe = json!({ "model": name, "state": "", "questions": {} });
        let response = client.response_post(SYSTEM_ONE_PATH, &probe).await.ok()?;
        let status = response.status().as_u16();
        Some((
            status,
            response.json::<Value>().await.unwrap_or(Value::Null),
        ))
    };
    let (status, body) = tokio::time::timeout(LOAD_PROBE_TIMEOUT, ask)
        .await
        .ok()
        .flatten()?;
    let refusal = (ollama_refusal(status, &body) == Some(OllamaRefusal::FormatUnsupported))
        .then(|| ollama_load_refusal_sentence(name));
    if status == 400 {
        if let (Some(key), Ok(mut probes)) = (key, LOAD_PROBES.lock()) {
            // A new copy replaces what was known about the old one.
            probes.retain(|(base, model, _), _| !(*base == key.0 && *model == key.1));
            probes.insert(key, refusal.clone());
        }
    }
    refusal
}

/// The entry for one pulled model, or `None` when it is not a decision model
/// or the server does not describe it.
async fn discover_one(
    client: &ApiClient,
    base_url: &Url,
    pulled: PulledModel,
) -> Option<DecisionModelEntry> {
    let show = show(client, &pulled.name).await?;
    let facts = ollama_facts_from_show(&pulled.name, &show);
    if !facts.decision {
        return None;
    }
    let mut entry = ollama_entry(base_url, &pulled.name, &facts, DecisionAdapter::SystemOne);
    // A cloud model is served by ollama.com, not loaded here; it is not probed.
    if !facts.remote {
        entry.load_refusal =
            probe_load_refusal(client, base_url, &pulled.name, pulled.version.as_deref()).await;
    }
    Some(entry)
}

/// When, for which server and which pulled copies, and what was found.
type Discovery = (Instant, DiscoveryKey, Vec<DecisionModelEntry>);
/// The server's address and every pulled copy (name and version), sorted.
type DiscoveryKey = (String, Vec<(String, Option<String>)>);
static DISCOVERED: LazyLock<Mutex<Option<Discovery>>> = LazyLock::new(|| Mutex::new(None));

/// What a discovery's answer depends on: the server and the copies it lists.
/// The address alone was not enough: a server started later on the same
/// address (a restarted Ollama, or in tests a mock server on a port a finished
/// one had) was handed the old server's models for a minute.
fn discovery_key(base_url: &Url, pulled: &[PulledModel]) -> DiscoveryKey {
    let mut copies: Vec<(String, Option<String>)> = pulled
        .iter()
        .map(|model| (model.name.clone(), model.version.clone()))
        .collect();
    copies.sort();
    (base_url.to_string(), copies)
}

/// Every decision model pulled on the Ollama server at `base_url`, from
/// `/api/tags` and each model's `/api/show`, with a copy Ollama will not load
/// marked ([`ollama_load_refusal`]) and listed last. An unreachable server
/// lists none.
pub async fn discover_ollama(base_url: &Url) -> Vec<DecisionModelEntry> {
    let Ok(client) = ollama_client(base_url, TAGS_TIMEOUT.max(SHOW_TIMEOUT)) else {
        return Vec::new();
    };
    let Some(tags) = ollama_tags(&client).await else {
        return Vec::new();
    };
    let pulled = pulled_models(&tags);
    let key = discovery_key(base_url, &pulled);
    if let Ok(cache) = DISCOVERED.lock() {
        if let Some((at, known, entries)) = cache.as_ref() {
            if *known == key && at.elapsed() < DISCOVERY_TTL {
                return entries.clone();
            }
        }
    }
    use futures::StreamExt;
    let client = &client;
    let mut entries: Vec<DecisionModelEntry> = futures::stream::iter(pulled)
        .map(|pulled| discover_one(client, base_url, pulled))
        .buffer_unordered(SHOW_CONCURRENCY)
        .filter_map(|entry| async move { entry })
        .collect()
        .await;
    order_ollama_entries(&mut entries);
    if let Ok(mut cache) = DISCOVERED.lock() {
        *cache = Some((Instant::now(), key, entries.clone()));
    }
    entries
}

/// The production source: config, the Baseten feed and the Ollama server.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfiguredDecisionModels;

#[async_trait]
impl DecisionModelSource for ConfiguredDecisionModels {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        let mut entries = Vec::new();
        if let Ok(base) = ollama_base_url() {
            entries.extend(discover_ollama(&base).await);
        }
        if baseten_configured() {
            entries.extend(baseten_entries());
        }
        entries
    }

    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        match wanted.provider.as_str() {
            BASETEN_DECISION_PROVIDER => {
                let entry = resolve_baseten(wanted)?;
                Ok(Arc::new(SystemOneAdapter::baseten_from_config(entry)?))
            }
            OLLAMA_DECISION_PROVIDER => {
                let base = ollama_base_url()?;
                let facts = ollama_facts(&base, &wanted.model).await?.ok_or_else(|| {
                    DecisionError::NoModel(format!(
                        "`{}` is not pulled on the Ollama server at {base}, or the server did \
                         not answer. Pull it with `ollama pull {}`.",
                        wanted.model, wanted.model
                    ))
                })?;
                open_ollama(&base, wanted, &facts)
            }
            other => Err(DecisionError::NoModel(format!(
                "`{other}` is not a decision model provider. Use `{BASETEN_DECISION_PROVIDER}` or \
                 `{OLLAMA_DECISION_PROVIDER}`."
            ))),
        }
    }
}

/// The Baseten entry `wanted` names, by alias or slug. A `decision-models/`
/// slug this release does not know is accepted with Baseten's stated window.
pub fn resolve_baseten(wanted: &DecisionModelRef) -> DecisionResult<DecisionModelEntry> {
    if wanted
        .adapter
        .is_some_and(|adapter| adapter != DecisionAdapter::SystemOne)
    {
        return Err(DecisionError::Unsupported(
            "Baseten decision models answer only through System One.".to_string(),
        ));
    }
    let entries = baseten_entries();
    if let Some(entry) = entries.into_iter().find(|entry| entry.answers_to(wanted)) {
        return Ok(entry);
    }
    if let Some(known) = BASETEN_DECISION_MODELS
        .iter()
        .find(|known| known.name == wanted.model || known.slug == wanted.model)
    {
        return Ok(baseten_entry(known.slug, known.context_tokens));
    }
    if wanted.model.starts_with(DECISION_MODEL_PREFIX) {
        return Ok(baseten_entry(&wanted.model, BASETEN_DECISION_CONTEXT));
    }
    Err(DecisionError::NoModel(format!(
        "`{}` is not a Baseten decision model. The known ones are {}.",
        wanted.model,
        BASETEN_DECISION_MODELS
            .iter()
            .map(|known| known.name)
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Open an Ollama model from its facts: System One for a decision model,
/// logprobs for any other, or whichever adapter `wanted` asks for.
pub fn open_ollama(
    base: &Url,
    wanted: &DecisionModelRef,
    facts: &OllamaDecisionFacts,
) -> DecisionResult<Arc<dyn DecisionModel>> {
    let adapter = wanted.adapter.unwrap_or(if facts.decision {
        DecisionAdapter::SystemOne
    } else {
        DecisionAdapter::Logprob
    });
    match adapter {
        DecisionAdapter::SystemOne if !facts.decision => Err(DecisionError::Unsupported(format!(
            "`{}` is not a decision model (its capabilities lack `decision`). Ask with the \
             `logprob` adapter instead.",
            wanted.model
        ))),
        DecisionAdapter::SystemOne => Ok(Arc::new(SystemOneAdapter::ollama(
            base,
            ollama_entry(base, &wanted.model, facts, adapter),
        )?)),
        DecisionAdapter::Logprob => Ok(Arc::new(LogprobAdapter::ollama(
            base,
            ollama_entry(base, &wanted.model, facts, adapter),
            facts.thinking,
        )?)),
        DecisionAdapter::Verbalized => Err(DecisionError::Unsupported(
            "A verbalized decision runs on a chat model; name the chat's model instead."
                .to_string(),
        )),
    }
}

/// The models a session may use, in the order an automatic choice tries them:
/// local first (as Llama Server and Ollama rank first in the provider
/// catalog), otherwise in the order they were listed. The sort is stable.
pub fn rank_local_first(mut entries: Vec<DecisionModelEntry>) -> Vec<DecisionModelEntry> {
    entries.sort_by_key(|entry| !entry.local);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feed_supplies_decision_models_and_chat_advertising_is_unchanged() {
        let feed = br#"{"schema_version":1,"provider":"baseten","source":"test",
            "fetched_at":"2026-11-04T17:20:51Z","models":[
            {"id":"zai-org/GLM-5.2","context_length":202752,"input_modalities":["text"],
             "supported_features":["tools"],"pricing":{"prompt":"0.0000006","completion":"0.0000022"}},
            {"id":"decision-models/baseten-elephant-9b-latest","context_length":16384,
             "input_modalities":["text","image"],"supported_features":[],
             "pricing":{"prompt":"0.000000035","completion":"0.00","input_cache_read":"0.00"}},
            {"id":"decision-models/baseten-joey-2b-latest","context_length":16384,
             "input_modalities":["text","image"],"supported_features":[],
             "pricing":{"prompt":"0.00000001","completion":"0.00"}},
            {"id":"decision-models/baseten-newt-1b-latest","context_length":8192,
             "input_modalities":["text"],"supported_features":[],
             "pricing":{"prompt":"0.00000001","completion":"0.00"}}]}"#;
        let catalog = LiveCatalog::from_bytes(feed).unwrap();
        let entries = baseten_entries_from(Some(&catalog));
        assert_eq!(
            entries
                .iter()
                .map(|e| e.display_name.as_str())
                .collect::<Vec<_>>(),
            ["joey-1", "elephant-1", "baseten-newt-1b-latest"]
        );
        assert_eq!(entries[2].context_tokens, 8192);
        assert!(entries.iter().all(|e| e.tier == ProviderTier::Public
            && e.affiliation.is_none()
            && !e.local
            && e.provider == BASETEN_DECISION_PROVIDER));
        // The chat picker still sees only the chat model.
        assert_eq!(
            catalog
                .advertised()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["zai-org/GLM-5.2"]
        );
    }

    #[test]
    fn without_a_feed_the_three_known_slugs_stand() {
        let entries = baseten_entries_from(None);
        assert_eq!(
            entries.iter().map(|e| e.model.as_str()).collect::<Vec<_>>(),
            [
                "decision-models/baseten-joey-2b-latest",
                "decision-models/baseten-elephant-9b-latest",
                "decision-models/baseten-mammoth-27b-latest",
            ]
        );
        assert!(entries.iter().all(|e| e.context_tokens == 16_384));
    }

    #[test]
    fn baseten_models_resolve_by_alias_or_slug() {
        let by_alias =
            resolve_baseten(&DecisionModelRef::new(BASETEN_DECISION_PROVIDER, "joey-1")).unwrap();
        assert_eq!(by_alias.model, "decision-models/baseten-joey-2b-latest");
        let by_slug = resolve_baseten(&DecisionModelRef::new(
            BASETEN_DECISION_PROVIDER,
            "decision-models/baseten-mammoth-27b-latest",
        ))
        .unwrap();
        assert_eq!(by_slug.display_name, "mammoth-1");
        assert!(resolve_baseten(&DecisionModelRef::new(
            BASETEN_DECISION_PROVIDER,
            "zai-org/GLM-5.3"
        ))
        .is_err());
        assert!(resolve_baseten(
            &DecisionModelRef::new(BASETEN_DECISION_PROVIDER, "joey-1")
                .with_adapter(DecisionAdapter::Logprob)
        )
        .is_err());
    }

    fn show(capabilities: &[&str], extra: Value) -> Value {
        let mut show = json!({
            "capabilities": capabilities,
            "model_info": {"general.architecture": "qwen35", "qwen35.context_length": 262144},
        });
        if let (Some(target), Value::Object(extra)) = (show.as_object_mut(), extra) {
            target.extend(extra);
        }
        show
    }

    #[test]
    fn ollama_facts_follow_capabilities_window_and_the_cloud_rule() {
        let clef = ollama_facts_from_show(
            "clef-flash:latest",
            &show(
                &["decision", "vision"],
                json!({"parameters": "num_ctx                        16384"}),
            ),
        );
        assert!(clef.decision && !clef.thinking && !clef.remote);
        assert_eq!(clef.context_tokens, 16384);

        let trained = ollama_facts_from_show("nimble:9b", &show(&["decision"], json!({})));
        assert_eq!(trained.context_tokens, 262144);

        let bare = ollama_facts_from_show("tev1:0.8b", &json!({"capabilities": ["decision"]}));
        assert_eq!(bare.context_tokens, OLLAMA_DEFAULT_DECISION_CONTEXT);

        let checker =
            ollama_facts_from_show("bespoke-minicheck:7b", &show(&["completion"], json!({})));
        assert!(!checker.decision);

        let cloud = ollama_facts_from_show("gpt-oss:120b-cloud", &show(&["completion"], json!({})));
        assert!(cloud.remote);
        let renamed = ollama_facts_from_show(
            "mine:latest",
            &show(
                &["decision"],
                json!({"remote_host": "https://ollama.com:443"}),
            ),
        );
        assert!(renamed.remote);
    }

    #[test]
    fn an_ollama_tier_is_the_self_hosted_rule_with_the_cloud_exception() {
        let loopback = Url::parse("http://localhost:11434/").unwrap();
        let lan = Url::parse("http://gpu-box:11434/").unwrap();
        let local = OllamaDecisionFacts {
            decision: true,
            thinking: false,
            remote: false,
            context_tokens: 8192,
        };
        let entry = ollama_entry(&loopback, "tev1:0.8b", &local, DecisionAdapter::SystemOne);
        assert_eq!(entry.tier, ProviderTier::Private);
        assert!(entry.local);
        assert!(entry.affiliation.is_some());
        let remote_host = ollama_entry(&lan, "tev1:0.8b", &local, DecisionAdapter::SystemOne);
        assert_eq!(remote_host.tier, ProviderTier::Public);
        assert!(!remote_host.local);
        let cloud = OllamaDecisionFacts {
            remote: true,
            ..local.clone()
        };
        let cloud = ollama_entry(&loopback, "x:cloud", &cloud, DecisionAdapter::SystemOne);
        assert_eq!(cloud.tier, ProviderTier::Public);
        assert!(cloud.affiliation.is_none());

        // A logprob model is not given a 262k cache for a one-token answer.
        let big = OllamaDecisionFacts {
            decision: false,
            context_tokens: 262_144,
            ..local
        };
        assert_eq!(
            ollama_entry(&loopback, "m", &big, DecisionAdapter::Logprob).context_tokens,
            32_768
        );
    }

    #[test]
    fn the_adapter_follows_the_capability_unless_asked() {
        let base = Url::parse("http://localhost:11434/").unwrap();
        let checker = OllamaDecisionFacts {
            decision: false,
            thinking: false,
            remote: false,
            context_tokens: 8192,
        };
        let wanted = DecisionModelRef::new(OLLAMA_DECISION_PROVIDER, "bespoke-minicheck:7b");
        let model = open_ollama(&base, &wanted, &checker).unwrap();
        assert_eq!(model.model_ref().adapter, Some(DecisionAdapter::Logprob));
        assert!(open_ollama(
            &base,
            &wanted.clone().with_adapter(DecisionAdapter::SystemOne),
            &checker
        )
        .is_err());
        let decision = OllamaDecisionFacts {
            decision: true,
            ..checker
        };
        let model = open_ollama(&base, &wanted, &decision).unwrap();
        assert_eq!(model.model_ref().adapter, Some(DecisionAdapter::SystemOne));
    }

    #[test]
    fn the_shipped_default_leads_the_local_models_then_names_follow() {
        let base = Url::parse("http://localhost:11434/").unwrap();
        let facts = OllamaDecisionFacts {
            decision: true,
            thinking: false,
            remote: false,
            context_tokens: 8192,
        };
        let entry = |name: &str| ollama_entry(&base, name, &facts, DecisionAdapter::SystemOne);
        let mut entries = vec![
            entry("tev1:4b"),
            entry("clef-flash:latest"),
            entry(OLLAMA_DEFAULT_DECISION_MODEL),
            entry("nimble:9b"),
        ];
        order_ollama_entries(&mut entries);
        let order: Vec<_> = entries.iter().map(|e| e.model.as_str()).collect();
        assert_eq!(
            order,
            ["tev1:0.8b", "clef-flash:latest", "nimble:9b", "tev1:4b"]
        );
        // The local-first rank keeps that order.
        let ranked = rank_local_first(entries);
        assert_eq!(ranked[0].model, OLLAMA_DEFAULT_DECISION_MODEL);
        assert_eq!(ranked[1].model, "clef-flash:latest");

        // A copy Ollama refuses goes after every one it loads, the default too.
        let refused = |name: &str| DecisionModelEntry {
            load_refusal: Some(ollama_load_refusal_sentence(name)),
            ..entry(name)
        };
        let mut entries = vec![
            refused("clef-flash:latest"),
            entry("tev1:4b"),
            refused(OLLAMA_DEFAULT_DECISION_MODEL),
            entry("nimble:9b"),
        ];
        order_ollama_entries(&mut entries);
        let order: Vec<_> = entries.iter().map(|e| e.model.as_str()).collect();
        assert_eq!(
            order,
            ["nimble:9b", "tev1:4b", "tev1:0.8b", "clef-flash:latest"]
        );
    }

    #[test]
    fn local_models_rank_first_and_order_is_otherwise_kept() {
        let base = Url::parse("http://localhost:11434/").unwrap();
        let facts = OllamaDecisionFacts {
            decision: true,
            thinking: false,
            remote: false,
            context_tokens: 8192,
        };
        let ranked = rank_local_first(vec![
            baseten_entry("decision-models/baseten-joey-2b-latest", 16384),
            ollama_entry(&base, "tev1:0.8b", &facts, DecisionAdapter::SystemOne),
            baseten_entry("decision-models/baseten-elephant-9b-latest", 16384),
        ]);
        assert_eq!(
            ranked
                .iter()
                .map(|e| e.display_name.as_str())
                .collect::<Vec<_>>(),
            ["tev1:0.8b", "joey-1", "elephant-1"]
        );
    }
}
