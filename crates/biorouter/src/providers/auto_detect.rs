//! Best-effort detection of which commercial provider an API key belongs to.
//!
//! Onboarding lets a user paste a single API key and figures out the provider
//! by probing each candidate's `/models` endpoint with the key. Three
//! properties matter and are enforced here:
//!
//! * **No environment mutation.** The candidate key is supplied to provider
//!   constructors via a task-local override (see
//!   [`crate::config::with_config_overrides`]), never `std::env::set_var`.
//!   Mutating the process environment from a multi-threaded program is unsound
//!   and would leak the key into any subprocess spawned during the probe.
//! * **Bounded latency.** The whole probe is wrapped in a hard timeout and each
//!   candidate is tried exactly once (no retry budget), so a throttled or
//!   unreachable vendor can't hang the onboarding spinner.
//! * **Prefix narrowing.** Keys with a distinctive prefix (`sk-ant-`, `AIza`,
//!   `gsk_`, `xai-`) are only tried against the matching provider, avoiding
//!   wasted calls and cross-provider false positives. Ambiguous keys (generic
//!   `sk-…`) fan out across the OpenAI-compatible pool.

use std::collections::HashMap;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::config::with_config_overrides;
use crate::model::ModelConfig;
use crate::providers::errors::ProviderError;

/// Upper bound on the whole detection probe. Provider retry budgets can reach
/// ~2 minutes on a rate limit; detection must never inherit that.
const DETECT_TIMEOUT: Duration = Duration::from_secs(12);

/// A provider that can be auto-detected from a single bearer API key.
struct Candidate {
    /// Registry name passed to [`crate::providers::create`].
    provider: &'static str,
    /// Secret key name the provider reads (and that we override during probing).
    env_key: &'static str,
    /// Distinctive key prefixes. If the pasted key starts with one of these,
    /// only this candidate is tried. Empty = part of the ambiguous fallback pool.
    exclusive_prefixes: &'static [&'static str],
    /// Extra non-secret config to persist alongside the key so the configured
    /// provider targets the same endpoint the probe validated against
    /// (e.g. MiMo's regional host).
    extra_config: &'static [(&'static str, &'static str)],
}

/// The detectable providers. Order is irrelevant — detection is first-success.
const CANDIDATES: &[Candidate] = &[
    Candidate {
        provider: "anthropic",
        env_key: "ANTHROPIC_API_KEY",
        exclusive_prefixes: &["sk-ant-"],
        extra_config: &[],
    },
    Candidate {
        provider: "google",
        env_key: "GOOGLE_API_KEY",
        exclusive_prefixes: &["AIza"],
        extra_config: &[],
    },
    Candidate {
        provider: "groq",
        env_key: "GROQ_API_KEY",
        exclusive_prefixes: &["gsk_"],
        extra_config: &[],
    },
    Candidate {
        provider: "xai",
        env_key: "XAI_API_KEY",
        exclusive_prefixes: &["xai-"],
        extra_config: &[],
    },
    // OpenAI-compatible providers share the generic `sk-…` shape, so they have
    // no exclusive prefix and form the ambiguous fallback pool.
    Candidate {
        provider: "openai",
        env_key: "OPENAI_API_KEY",
        exclusive_prefixes: &[],
        extra_config: &[],
    },
    Candidate {
        provider: "zai",
        env_key: "ZAI_API_KEY",
        exclusive_prefixes: &[],
        extra_config: &[],
    },
    Candidate {
        provider: "xiaomi_mimo",
        env_key: "XIAOMI_MIMO_API_KEY",
        exclusive_prefixes: &[],
        // Persist the host we validated against so the saved provider doesn't
        // silently fall back to a different default later.
        extra_config: &[(
            "XIAOMI_MIMO_HOST",
            crate::providers::xiaomi_mimo::XIAOMI_MIMO_API_HOST,
        )],
    },
];

/// A successful detection result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// Registry provider name (e.g. `"openai"`).
    pub provider: String,
    /// Exact secret config key consumed by the detected provider.
    pub api_key_config_key: String,
    /// All model ids the provider reported for this key.
    pub models: Vec<String>,
    /// A sensible default/recommended chat model, when one can be determined.
    pub default_model: Option<String>,
    /// Non-secret config to persist alongside the key (may be empty).
    pub extra_config: HashMap<String, String>,
}

/// Why detection failed — surfaced to the UI so it can give actionable advice
/// instead of a single opaque "could not detect" message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectError {
    /// The probe exceeded [`DETECT_TIMEOUT`].
    Timeout,
    /// Every probe failed with a transient/connectivity error.
    Network,
    /// The key matched a provider by prefix, but that provider rejected it.
    InvalidKey,
    /// The key did not validate against any supported provider.
    NoMatch,
}

impl DetectError {
    /// Stable machine-readable code for the API/UI.
    pub fn code(&self) -> &'static str {
        match self {
            DetectError::Timeout => "timeout",
            DetectError::Network => "network",
            DetectError::InvalidKey => "invalid_key",
            DetectError::NoMatch => "no_match",
        }
    }
}

/// Per-candidate failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeError {
    /// The provider explicitly rejected the key (this isn't its key).
    Auth,
    /// Transient/connectivity failure — inconclusive.
    Network,
    /// Anything else (construction failed, unexpected error).
    Other,
}

/// Map a provider error encountered while probing to a coarse class.
fn classify(err: &ProviderError) -> ProbeError {
    match err {
        ProviderError::Authentication(_) => ProbeError::Auth,
        ProviderError::RateLimitExceeded { .. }
        | ProviderError::ServerError(_)
        | ProviderError::RequestFailed(_) => ProbeError::Network,
        _ => ProbeError::Other,
    }
}

/// Choose which candidates to probe for a given key, and whether the selection
/// was driven by an exclusive prefix match. Pure so it can be unit-tested.
fn candidates_for_key(key: &str) -> (Vec<&'static Candidate>, bool) {
    let matched: Vec<&'static Candidate> = CANDIDATES
        .iter()
        .filter(|c| {
            !c.exclusive_prefixes.is_empty()
                && c.exclusive_prefixes.iter().any(|p| key.starts_with(p))
        })
        .collect();
    if !matched.is_empty() {
        return (matched, true);
    }
    // No distinctive prefix: try the OpenAI-compatible ambiguous pool only.
    let pool: Vec<&'static Candidate> = CANDIDATES
        .iter()
        .filter(|c| c.exclusive_prefixes.is_empty())
        .collect();
    (pool, false)
}

/// Display-purpose list of provider registry names that auto-detection supports.
pub fn detectable_providers() -> Vec<&'static str> {
    CANDIDATES.iter().map(|c| c.provider).collect()
}

/// A registered provider's shipped default model and its curated model list, in
/// the order its metadata declares them. Empty when the name is not registered.
async fn shipped_catalog(provider: &str) -> (String, Vec<String>) {
    crate::providers::providers()
        .await
        .into_iter()
        .find(|(metadata, _)| metadata.name == provider)
        .map(|(metadata, _)| {
            let curated = metadata
                .known_models
                .into_iter()
                .map(|model| model.name)
                .collect();
            (metadata.default_model, curated)
        })
        .unwrap_or_default()
}

/// The model a detected key should start on, when the provider has a say.
///
/// The shipped default wins if the key serves it: it is what `biorouter
/// configure` and the desktop picker start the same provider on, and a
/// provider may hold it back from a newer model on purpose (Anthropic keeps
/// Opus 4.8 until Opus 5.5 passes a smoke test). Otherwise the first curated
/// model the key serves, because a curated list is ordered by preference.
/// `None` when the key serves none of them, and the caller falls back.
fn preferred_served_model(
    shipped_default: &str,
    curated: &[String],
    served: &[String],
) -> Option<String> {
    let is_served = |model: &str| served.iter().any(|id| id == model);
    if !shipped_default.is_empty() && is_served(shipped_default) {
        return Some(shipped_default.to_string());
    }
    curated.iter().find(|model| is_served(model)).cloned()
}

/// Probe a single candidate with the key, scoped via a task-local override so we
/// never touch the process environment.
async fn probe_candidate(c: &'static Candidate, key: String) -> Result<Detected, ProbeError> {
    let mut overrides = HashMap::new();
    overrides.insert(c.env_key.to_string(), key);
    for (k, v) in c.extra_config {
        overrides.insert((*k).to_string(), (*v).to_string());
    }

    with_config_overrides(overrides, async move {
        let provider =
            match crate::providers::create(c.provider, ModelConfig::new_or_fail("default")).await {
                Ok(p) => p,
                Err(_) => return Err(ProbeError::Other),
            };

        let models = match provider.fetch_supported_models().await {
            Ok(Some(models)) if !models.is_empty() => models,
            // No models returned means we can't confirm this is the right
            // provider for the key — treat as a non-match, not a hard error.
            Ok(_) => return Err(ProbeError::Auth),
            Err(e) => return Err(classify(&e)),
        };

        // Start the key on the provider's own choice of model whenever the key
        // serves it: the shipped default, else the first model of its curated
        // list. Only when the key serves none of them fall back to a recommended
        // (text-capable, canonical) model, which is the first of an arbitrary
        // ordering (Anthropic's `/v1/models` comes back sorted by name), and
        // last to whatever the key listed first. Best-effort.
        let (shipped_default, curated) = shipped_catalog(c.provider).await;
        let default_model = match preferred_served_model(&shipped_default, &curated, &models) {
            Some(model) => Some(model),
            None => provider
                .fetch_recommended_models()
                .await
                .ok()
                .flatten()
                .and_then(|recommended| recommended.into_iter().next())
                .or_else(|| models.first().cloned()),
        };

        let extra_config = c
            .extra_config
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();

        Ok(Detected {
            provider: c.provider.to_string(),
            api_key_config_key: c.env_key.to_string(),
            models,
            default_model,
            extra_config,
        })
    })
    .await
}

/// Detect which provider an API key belongs to by probing candidate `/models`
/// endpoints. Returns the first provider that validates the key, or a
/// classified [`DetectError`] on failure.
pub async fn detect_provider_from_api_key(api_key: &str) -> Result<Detected, DetectError> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err(DetectError::NoMatch);
    }

    let (candidates, used_exclusive) = candidates_for_key(&key);

    let probe = async move {
        let mut set: JoinSet<Result<Detected, ProbeError>> = JoinSet::new();
        for c in candidates {
            let key = key.clone();
            set.spawn(probe_candidate(c, key));
        }

        let mut saw_network = false;
        let mut saw_auth = false;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(detected)) => {
                    set.abort_all();
                    return Ok(detected);
                }
                Ok(Err(ProbeError::Network)) => saw_network = true,
                Ok(Err(ProbeError::Auth)) => saw_auth = true,
                Ok(Err(ProbeError::Other)) => {}
                // Task panicked or was aborted — ignore and keep going.
                Err(_) => {}
            }
        }

        // No candidate validated the key. Pick the most useful reason.
        if saw_network && !saw_auth {
            Err(DetectError::Network)
        } else if used_exclusive {
            // The key looked like a specific provider's key but was rejected.
            Err(DetectError::InvalidKey)
        } else {
            Err(DetectError::NoMatch)
        }
    };

    match tokio::time::timeout(DETECT_TIMEOUT, probe).await {
        Ok(result) => result,
        Err(_) => Err(DetectError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(cands: &[&'static Candidate]) -> Vec<&'static str> {
        cands.iter().map(|c| c.provider).collect()
    }

    #[test]
    fn exclusive_prefix_routes_to_single_provider() {
        let (cands, exclusive) = candidates_for_key("sk-ant-abc123");
        assert!(exclusive);
        assert_eq!(names(&cands), vec!["anthropic"]);

        let (cands, exclusive) = candidates_for_key("AIzaSyExample");
        assert!(exclusive);
        assert_eq!(names(&cands), vec!["google"]);

        let (cands, exclusive) = candidates_for_key("gsk_example");
        assert!(exclusive);
        assert_eq!(names(&cands), vec!["groq"]);

        let (cands, exclusive) = candidates_for_key("xai-example");
        assert!(exclusive);
        assert_eq!(names(&cands), vec!["xai"]);
    }

    #[test]
    fn generic_sk_key_uses_ambiguous_pool() {
        let (cands, exclusive) = candidates_for_key("sk-proj-generic-openai-key");
        assert!(!exclusive);
        let n = names(&cands);
        // Pool is exactly the no-prefix providers; anthropic/google/groq/xai
        // (which have distinctive prefixes) must be excluded.
        assert!(n.contains(&"openai"));
        assert!(n.contains(&"zai"));
        assert!(n.contains(&"xiaomi_mimo"));
        assert!(!n.contains(&"anthropic"));
        assert!(!n.contains(&"google"));
        assert!(!n.contains(&"groq"));
        assert!(!n.contains(&"xai"));
    }

    #[test]
    fn unknown_prefix_falls_back_to_pool() {
        // A key for an unsupported provider with an odd shape still gets probed
        // against the ambiguous pool (and will simply fail to validate).
        let (cands, exclusive) = candidates_for_key("weirdformatkey123");
        assert!(!exclusive);
        assert!(!cands.is_empty());
        assert!(cands.iter().all(|c| c.exclusive_prefixes.is_empty()));
    }

    #[test]
    fn classify_maps_errors_to_classes() {
        assert_eq!(
            classify(&ProviderError::Authentication("bad".into())),
            ProbeError::Auth
        );
        assert_eq!(
            classify(&ProviderError::ServerError("500".into())),
            ProbeError::Network
        );
        assert_eq!(
            classify(&ProviderError::RequestFailed("conn".into())),
            ProbeError::Network
        );
        assert_eq!(
            classify(&ProviderError::RateLimitExceeded {
                details: "429".into(),
                retry_delay: None
            }),
            ProbeError::Network
        );
        assert_eq!(
            classify(&ProviderError::UsageError("x".into())),
            ProbeError::Other
        );
    }

    #[test]
    fn detect_error_codes_are_stable() {
        assert_eq!(DetectError::Timeout.code(), "timeout");
        assert_eq!(DetectError::Network.code(), "network");
        assert_eq!(DetectError::InvalidKey.code(), "invalid_key");
        assert_eq!(DetectError::NoMatch.code(), "no_match");
    }

    #[test]
    fn detectable_providers_lists_all_candidates() {
        assert_eq!(
            detectable_providers(),
            vec![
                "anthropic",
                "google",
                "groq",
                "xai",
                "openai",
                "zai",
                "xiaomi_mimo",
            ]
        );
    }

    #[tokio::test]
    async fn every_candidate_uses_its_registered_secret_key() {
        let registered = crate::providers::providers().await;

        for candidate in CANDIDATES {
            let metadata = registered
                .iter()
                .find(|(metadata, _)| metadata.name == candidate.provider)
                .map(|(metadata, _)| metadata)
                .unwrap_or_else(|| panic!("missing registered provider {}", candidate.provider));

            let key = metadata
                .config_keys
                .iter()
                .find(|key| key.name == candidate.env_key)
                .unwrap_or_else(|| {
                    panic!(
                        "{} detection saves {}, but the provider does not declare that key",
                        candidate.provider, candidate.env_key
                    )
                });
            assert!(key.secret, "{} key must be secret", candidate.provider);
            assert!(key.required, "{} key must be required", candidate.provider);
        }
    }

    #[tokio::test]
    async fn empty_key_is_no_match_without_probing() {
        assert_eq!(
            detect_provider_from_api_key("   ").await,
            Err(DetectError::NoMatch)
        );
    }

    #[test]
    fn mimo_persists_host_in_extra_config() {
        let mimo = CANDIDATES
            .iter()
            .find(|c| c.provider == "xiaomi_mimo")
            .unwrap();
        assert_eq!(mimo.extra_config.len(), 1);
        assert_eq!(mimo.extra_config[0].0, "XIAOMI_MIMO_HOST");
    }

    fn ids(models: &[&str]) -> Vec<String> {
        models.iter().map(|model| model.to_string()).collect()
    }

    #[test]
    fn a_served_default_wins_over_every_newer_model() {
        let curated = ids(&["claude-opus-4-8", "claude-opus-5-5", "claude-fable-5-1"]);
        let served = ids(&["claude-fable-5-1", "claude-opus-4-8", "claude-opus-5-5"]);
        assert_eq!(
            preferred_served_model("claude-opus-4-8", &curated, &served).as_deref(),
            Some("claude-opus-4-8")
        );
    }

    #[test]
    fn an_unserved_default_falls_to_the_first_curated_model_the_key_serves() {
        let curated = ids(&["gpt-6-sol", "gpt-6-astra", "gpt-5.5"]);
        let served = ids(&["babbage-002", "gpt-5.5", "gpt-6-astra"]);
        assert_eq!(
            preferred_served_model("gpt-6-sol", &curated, &served).as_deref(),
            Some("gpt-6-astra")
        );
    }

    #[test]
    fn a_key_serving_nothing_curated_leaves_the_choice_to_the_caller() {
        let served = ids(&["some-model"]);
        assert_eq!(
            preferred_served_model("gpt-6-sol", &ids(&["gpt-6-sol"]), &served),
            None
        );
        // An unregistered provider has an empty catalog.
        assert_eq!(preferred_served_model("", &[], &served), None);
    }

    /// End to end against a stand-in for Anthropic's `/v1/models`, which
    /// answers every model the key can call, including newer ones than the
    /// default; the provider sorts them by name. Before 2026-09-27 onboarding
    /// started the key on the first recommended model of that sorted list,
    /// whatever it was, not on the default the provider holds Opus 5.5 back
    /// behind.
    #[tokio::test]
    async fn a_detected_anthropic_key_starts_on_the_shipped_default() {
        use crate::providers::anthropic::ANTHROPIC_DEFAULT_MODEL;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    { "id": "claude-opus-5-5" },
                    { "id": "claude-fable-5-1" },
                    { "id": "claude-sonnet-5" },
                    { "id": ANTHROPIC_DEFAULT_MODEL },
                    { "id": "claude-haiku-4-5" },
                ]
            })))
            .mount(&server)
            .await;

        // The real anthropic candidate, pointed at the stand-in. `extra_config`
        // is `'static`, and one leaked string per test run is the price.
        let host: &'static str = Box::leak(server.uri().into_boxed_str());
        let extra: &'static [(&'static str, &'static str)] =
            Box::leak(vec![("ANTHROPIC_HOST", host)].into_boxed_slice());
        let anthropic = CANDIDATES
            .iter()
            .find(|c| c.provider == "anthropic")
            .expect("anthropic is detectable");
        let candidate: &'static Candidate = Box::leak(Box::new(Candidate {
            extra_config: extra,
            ..*anthropic
        }));

        let detected = probe_candidate(candidate, "sk-ant-test-not-a-real-key".to_string())
            .await
            .expect("the stand-in accepts the key");
        assert_eq!(detected.provider, "anthropic");
        assert_eq!(
            detected.default_model.as_deref(),
            Some(ANTHROPIC_DEFAULT_MODEL),
            "onboarding would start this key on a model other than the shipped default \
             (served: {:?})",
            detected.models
        );
    }
}
