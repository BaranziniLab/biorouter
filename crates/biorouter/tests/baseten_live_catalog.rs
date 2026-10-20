//! Baseten's live catalog, loaded the way a user's machine loads it: from the
//! last good copy on disk, read the first time anything asks.
//!
//! The unit tests in `providers::live_catalog` never read the disk, so no
//! developer's cached feed can change what they assert. That leaves the paths a
//! loaded catalog actually travels untested there: pricing on both paths, the
//! provider registry the picker reads, and the window a Baseten provider is
//! built with. This binary seeds the committed feed fixture into its own
//! `BIOROUTER_PATH_ROOT` before the first lookup, in its own process, so the
//! process-wide catalog can be loaded without reaching any other test.
//!
//! What it holds the code to:
//! - the feed prices Baseten, and the same slug on Groq keeps Groq's price;
//! - the registry lists the feed's models for Baseten, default first, and no
//!   other provider gains one;
//! - a feed window reaches a Baseten provider for a slug the static registry
//!   lacks, never the shared registry, and never over a limit the user pinned.
//!
//! No network: the refresher is the daemon's, and nothing here starts it.

use std::sync::OnceLock;

use biorouter::model::ModelConfig;
use biorouter::providers::{live_catalog, pricing, providers};

/// A slug the feed lists and the bundled catalog and static registry do not.
const FEED_ONLY: &str = "zai-org/GLM-5.2";
/// A slug Baseten and Groq both serve, at different prices.
const SHARED: &str = "openai/gpt-oss-120b";

fn sandbox() {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::env::set_var("BIOROUTER_PATH_ROOT", dir.path());
        std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
        // Building a Baseten provider reads its key; nothing here sends it.
        std::env::set_var("BASETEN_API_KEY", "not-a-real-key");
        for sub in ["config", "data", "state/provider-catalogs"] {
            std::fs::create_dir_all(dir.path().join(sub)).expect("a sandbox tree");
        }
        std::fs::write(dir.path().join("config/config.yaml"), "{}\n").expect("a config file");
        std::fs::write(dir.path().join("config/secrets.yaml"), "{}\n").expect("a secrets file");
        std::fs::write(
            dir.path().join("state/provider-catalogs/baseten.json"),
            include_bytes!("../../../landing/scripts/fixtures/baseten-catalog.json"),
        )
        .expect("a cached feed");
        dir
    });
}

#[tokio::test]
async fn the_feed_prices_baseten_and_no_other_provider() {
    sandbox();

    let sync = pricing::provider_model_pricing("baseten", FEED_ONLY)
        .expect("a feed-only model is priced from the feed");
    let resolved = pricing::resolved_provider_model_pricing("baseten", FEED_ONLY)
        .await
        .expect("and on the async path too");
    for found in [&sync, &resolved] {
        assert_eq!(found.input_token_cost, 1.4e-6);
        assert_eq!(found.output_token_cost, 4.4e-6);
        assert_eq!(found.cache_read_cost, Some(1.4e-7));
    }

    // The fixture prices gpt-oss-120b at $0.10 / $0.50 for Baseten; Groq's
    // own table says $0.15 / $0.60, and a feed keyed by provider cannot
    // reach it.
    let groq = pricing::provider_model_pricing("groq", SHARED).expect("Groq prices its model");
    assert_eq!(groq.input_token_cost, 0.15 / 1_000_000.0);
    assert_eq!(groq.output_token_cost, 0.60 / 1_000_000.0);
    let baseten = pricing::provider_model_pricing("baseten", SHARED).unwrap();
    assert_eq!(baseten.input_token_cost, 1e-7);
}

#[tokio::test]
async fn the_registry_lists_the_feeds_models_for_baseten_only() {
    sandbox();

    let registry = providers().await;
    let known = |name: &str| -> Vec<String> {
        registry
            .iter()
            .find(|(meta, _)| meta.name == name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .0
            .known_models
            .iter()
            .map(|model| model.name.clone())
            .collect()
    };

    let baseten = known("baseten");
    assert_eq!(baseten.len(), 16, "18 in the feed, less Beam and SID-1");
    assert_eq!(
        baseten[0], "zai-org/GLM-5.3",
        "the bundled default stays first"
    );
    assert!(baseten.iter().any(|model| model == FEED_ONLY));
    assert!(!baseten.iter().any(|model| model == "reflection/Beam-1.0"));
    assert!(!baseten.iter().any(|model| model == "sid/sid-1"));

    assert!(!known("groq").iter().any(|model| model == FEED_ONLY));
}

#[tokio::test]
async fn a_feed_window_reaches_only_baseten_models_it_states() {
    sandbox();

    // The premise: the static registry has no row for the slug, so without
    // the feed every provider would fall back to a pattern or the default.
    assert!(ModelConfig::registry_context_window(FEED_ONLY).is_none());

    assert_eq!(
        live_catalog::stated_window("baseten", FEED_ONLY),
        Some(1_048_576)
    );
    assert_eq!(live_catalog::stated_window("groq", FEED_ONLY), None);
    // The shared, name-keyed resolution does not consult the feed.
    assert_ne!(ModelConfig::context_window_for(FEED_ONLY), 1_048_576);
    // A slug the registry has keeps its row: 131,072, not the feed's 128,072.
    assert_eq!(live_catalog::stated_window("baseten", SHARED), None);

    let built = biorouter::providers::create("baseten", ModelConfig::new_or_fail(FEED_ONLY))
        .await
        .expect("a Baseten provider builds");
    assert_eq!(built.get_model_config().context_limit(), 1_048_576);

    let pinned = ModelConfig::new_or_fail(FEED_ONLY).with_context_limit(Some(200_000));
    let built = biorouter::providers::create("baseten", pinned)
        .await
        .expect("a Baseten provider builds");
    assert_eq!(
        built.get_model_config().context_limit(),
        200_000,
        "a limit the user pinned stays"
    );
}
