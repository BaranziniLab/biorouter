//! Every model gets its own context window.
//!
//! Two invariants, checked against the real provider registry:
//!
//! 1. Every model any provider advertises has its **own** explicit entry in
//!    `MODEL_CONTEXT_WINDOWS` — no model silently inherits a window from a
//!    substring pattern (`"gpt-5"` matching `gpt-5-pro`) or from
//!    `DEFAULT_CONTEXT_LIMIT`.
//! 2. The window a provider declares for a model equals the window
//!    `ModelConfig` resolves for it. The two sources can never drift.
//!
//! Together these mean two models that differ in context window are always
//! treated as two models, never reconciled into one number.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::model::ModelConfig;
use biorouter::providers::live_catalog;
use biorouter::providers::providers;

#[test]
fn november_2026_models_have_exact_windows_before_provider_overrides() {
    let cases = [
        ("gpt-6.2-astra", 1_050_000),
        ("gpt-6.2-sol", 1_050_000),
        ("gpt-6.1-sol", 1_050_000),
        ("gpt-6.1-terra", 1_050_000),
        ("gpt-6.2-astra-2026-11-17", 1_050_000),
        ("gpt-6.2-sol-2026-11-17", 1_050_000),
        ("gpt-6.1-sol-2026-09-29", 1_050_000),
        ("gpt-6.1-terra-2026-10-26", 1_050_000),
        ("openai/gpt-6.2-astra", 1_050_000),
        ("openai/gpt-6.2-sol", 1_050_000),
        ("claude-fable-5-2", 1_000_000),
        ("anthropic/claude-fable-5.2", 1_000_000),
        ("anthropic/claude-sonnet-5.5", 1_000_000),
        ("anthropic/claude-haiku-5.5", 1_000_000),
        ("us.anthropic.claude-fable-5-2", 1_000_000),
        ("claude-sonnet-5.5", 1_000_000),
        ("claude-haiku-5.5", 1_000_000),
        ("gemini-4-hydrogen", 1_048_576),
        ("gemini-4-argon", 1_048_576),
        ("gemini-4-carbon", 1_048_576),
        ("google/gemini-4-hydrogen", 1_048_576),
        ("google/gemini-4-argon", 1_048_576),
        ("google/gemini-4-carbon", 1_048_576),
        ("databricks-gpt-6-1-sol", 1_050_000),
        ("databricks-gpt-6-1-terra", 1_050_000),
        ("databricks-gemini-4-argon", 1_048_576),
        ("databricks-claude-opus-5-5", 1_000_000),
        ("databricks-claude-sonnet-5-5", 1_000_000),
        ("databricks-claude-haiku-5-5", 1_000_000),
        ("kimi-k3.1-pro", 1_000_000),
        ("kimi-k3.1-pro-highspeed", 1_000_000),
        ("moonshotai/Kimi-K3.1-Pro", 1_048_576),
        ("mercury-3", 1_000_000),
        ("mistral-large-4", 1_000_000),
        ("mistral-large-4-0", 1_000_000),
        ("deepseek-v4-1-flash-1017-uncensored", 1_000_000),
    ];
    for (id, expected) in cases {
        assert_eq!(
            ModelConfig::registry_context_window(id),
            Some(expected),
            "{id}"
        );
        assert_eq!(ModelConfig::context_window_for(id), expected, "{id}");
    }
    assert_eq!(
        ModelConfig::context_window_for("gpt-6.2-unannounced"),
        128_000
    );
    assert_eq!(ModelConfig::context_window_for("gpt-6-astra"), 1_050_000);
    const {
        assert!(biorouter::providers::codex::CODEX_CONTEXT_WINDOW < 1_050_000);
    }
}

#[test]
fn november_canonical_limits_keep_foreground_output_and_vision_distinct() {
    use biorouter::providers::canonical::maybe_get_canonical_model;

    for (provider, id, context, output) in [
        ("openai", "gpt-6.2-astra", 1_050_000, 128_000),
        ("openai", "gpt-6.2-sol", 1_050_000, 128_000),
        ("openai", "gpt-6.1-sol", 1_050_000, 128_000),
        ("openai", "gpt-6.1-terra", 1_050_000, 128_000),
        ("azure_openai", "gpt-6.2-sol-2026-11-17", 1_050_000, 128_000),
        ("anthropic", "claude-fable-5-2", 1_000_000, 128_000),
        (
            "aws_bedrock",
            "us.anthropic.claude-fable-5-2",
            1_000_000,
            128_000,
        ),
        ("google", "gemini-4-hydrogen", 1_048_576, 65_536),
        ("google", "gemini-4-argon", 1_048_576, 65_536),
        ("google", "gemini-4-carbon", 1_048_576, 65_536),
        ("gcp_vertex_ai", "gemini-4-hydrogen", 1_048_576, 65_536),
    ] {
        let spec = maybe_get_canonical_model(provider, id)
            .unwrap_or_else(|| panic!("missing canonical mapping for {provider}/{id}"));
        assert_eq!(spec.context_length, context, "{provider}/{id}");
        assert_eq!(spec.max_completion_tokens, Some(output), "{provider}/{id}");
        assert!(spec.supports_tools, "{provider}/{id}");
        assert!(
            spec.input_modalities.iter().any(|m| m == "image"),
            "{provider}/{id}"
        );
    }

    let glm = maybe_get_canonical_model("zai", "glm-5.3").unwrap();
    assert_eq!(glm.input_modalities, ["text"]);

    for id in ["mistral-large-4", "mistral-large-4-0"] {
        let direct = maybe_get_canonical_model("mistral", id).unwrap();
        assert_eq!(direct.context_length, 1_000_000, "{id}");
        assert_eq!(direct.max_completion_tokens, None, "{id}");
        assert!(direct.supports_tools, "{id}");
    }
    let route = maybe_get_canonical_model("openrouter", "mistralai/mistral-large-4-0").unwrap();
    assert_eq!(route.context_length, 1_048_576);
    assert_eq!(route.max_completion_tokens, Some(262_144));
}

/// (provider, model, declared window) for every model in the registry.
async fn advertised_models() -> Vec<(String, String, usize)> {
    let mut rows = Vec::new();
    for (meta, _) in providers().await {
        for model in &meta.known_models {
            rows.push((meta.name.clone(), model.name.clone(), model.context_limit));
        }
    }
    rows.sort();
    assert!(!rows.is_empty(), "provider registry is empty");
    rows
}

#[tokio::test]
async fn every_advertised_model_has_its_own_declared_window() {
    let rows = advertised_models().await;
    let missing: Vec<_> = rows
        .iter()
        .filter(|(provider, model, _)| {
            !ModelConfig::has_declared_context_window(model)
                && live_catalog::stated_window(provider, model).is_none()
        })
        .map(|(provider, model, declared)| format!("  {provider}/{model} (declared {declared})"))
        .collect();

    assert!(
        missing.is_empty(),
        "these models have no explicit entry in MODEL_CONTEXT_WINDOWS, so their context \
         window is inherited from a substring pattern or the default. Give each one its \
         own entry:\n{}",
        missing.join("\n")
    );
}

/// The one provider whose window is not the model's: Codex drives the user's
/// ChatGPT sign-in, which the Codex backend gives a smaller working window than
/// the OpenAI API serves for the same model id (258,400 against 1,050,000,
/// measured 2026-09-25). The registry is keyed by bare id and shared with the
/// OpenAI API provider, so it keeps the API window and Codex states its own.
const CODEX: &str = "codex";

// A second way a provider states a window of its own: a model only its live
// catalog lists (Baseten's daily feed can add slugs between releases) has no
// registry row, so the provider budgets with the catalog's window for it
// (`live_catalog::stated_window`, applied in `OpenAiProvider::from_custom_config`).
// No catalog is in force in a fresh checkout or in CI, so these rows appear
// only where a feed has been cached, and only for that provider.

#[tokio::test]
async fn provider_declared_windows_match_the_registry() {
    let rows = advertised_models().await;
    let drifted: Vec<_> = rows
        .iter()
        .filter_map(|(provider, model, declared)| {
            let resolved = if provider == CODEX {
                biorouter::providers::codex::CODEX_CONTEXT_WINDOW
            } else if let Some(stated) = live_catalog::stated_window(provider, model) {
                stated
            } else {
                ModelConfig::context_window_for(model)
            };
            (resolved != *declared).then(|| {
                format!("  {provider}/{model}: declares {declared}, registry says {resolved}")
            })
        })
        .collect();

    assert!(
        drifted.is_empty(),
        "provider catalogs disagree with MODEL_CONTEXT_WINDOWS:\n{}",
        drifted.join("\n")
    );
}

/// Models in the same family that differ in window stay distinct — the whole
/// point of the exact registry. These are the pairs a `contains()` pattern
/// would have collapsed.
#[tokio::test]
async fn same_family_models_with_different_windows_stay_distinct() {
    let cases = [
        // GPT-5.4: flagship and -pro are 1.05M, but mini/nano are 400k.
        ("gpt-5.4", 1_050_000),
        ("gpt-5.4-pro", 1_050_000),
        ("gpt-5.4-mini", 400_000),
        ("gpt-5.4-nano", 400_000),
        // GPT-6: Astra, Sol and Luna are 1.05M, and "gpt-5"/"gpt-5.6" must not
        // claim them. The dated Azure id below has its own exact entry; dated
        // ids the registry does not list, which reach the pattern rows, are
        // covered by `exact_registry_beats_the_substring_fallback` in model.rs.
        ("gpt-6-astra", 1_050_000),
        ("gpt-6-sol", 1_050_000),
        ("gpt-6-luna", 1_050_000),
        ("gpt-6-sol-2026-09-22", 1_050_000),
        // Databricks serves GPT-5.5 at its full window, not the 400k of mini.
        ("databricks-gpt-5-5", 1_050_000),
        ("databricks-gpt-5-4-mini", 400_000),
        // Claude: 4.6+ and the 5 series are 1M (Haiku 5.5 included); the 4.5
        // tier and Haiku 4.5 are 200k.
        // Fable 5.1's 1M is the same number the `claude-fable-5` pattern would
        // hand it, so this case cannot tell an exact entry from an inherited
        // one — `every_advertised_model_has_its_own_declared_window` above is
        // what pins the exact entry.
        ("claude-fable-5-1", 1_000_000),
        ("claude-opus-5", 1_000_000),
        ("claude-opus-5-5", 1_000_000),
        ("claude-opus-4-6", 1_000_000),
        ("claude-sonnet-5", 1_000_000),
        ("claude-haiku-5-5", 1_000_000),
        ("us.anthropic.claude-haiku-5-5", 1_000_000),
        ("claude-opus-4-5", 200_000),
        ("claude-haiku-4-5", 200_000),
        // Inception: Mercury Coder is 128k, Mercury Edit 2 is 32k.
        ("mercury-coder", 128_000),
        ("mercury-edit-2", 32_000),
    ];
    for (model, expected) in cases {
        assert_eq!(
            ModelConfig::context_window_for(model),
            expected,
            "{model} should have its own {expected}-token window"
        );
    }
}

/// The desktop chat input resolves a model's window client-side by scanning the
/// list `/config/read model-limits` serves and taking the first entry whose
/// pattern is a substring of the model name (`ChatInput.tsx::findModelLimit`).
///
/// Replicate that algorithm exactly: it must land on each model's true window,
/// or the UI shows the user a different budget than the agent actually uses.
#[tokio::test]
async fn the_uis_pattern_scan_lands_on_each_models_true_window() {
    let served = ModelConfig::get_all_model_limits();

    // Verbatim port of findModelLimit().
    let ui_would_show = |model: &str| -> Option<usize> {
        served
            .iter()
            .find(|l| model.to_lowercase().contains(&l.pattern.to_lowercase()))
            .map(|l| l.context_limit)
    };

    let wrong: Vec<_> = advertised_models()
        .await
        .iter()
        .filter_map(|(provider, model, _)| {
            // A window the provider states for its own model reaches the UI
            // through that provider's `known_models`, which `ChatInput.tsx`
            // reads before it falls back to this scan.
            if live_catalog::stated_window(provider, model).is_some() {
                return None;
            }
            let truth = ModelConfig::context_window_for(model);
            match ui_would_show(model) {
                Some(shown) if shown == truth => None,
                shown => Some(format!(
                    "  {provider}/{model}: agent uses {truth}, UI would show {shown:?}"
                )),
            }
        })
        .collect();

    assert!(
        wrong.is_empty(),
        "the UI's pattern scan disagrees with the real context window:\n{}",
        wrong.join("\n")
    );
}

/// Decision models: each registry row is the window its server enforces,
/// measured 2026-11-10 (Baseten's runtime takes 16,000, not the 16,384 its
/// feed states; Ollama's limit is the Modelfile's `num_ctx`), and the Ollama
/// decision provider declares no model list, so no listing reports one window
/// for every model it has pulled (it reported tev1:0.8b at 8,192).
#[tokio::test]
async fn decision_model_windows_are_the_ones_their_servers_enforce() {
    use biorouter::providers::decision::catalog::{
        BASETEN_DECISION_CONTEXT, BASETEN_DECISION_MODELS,
    };
    assert_eq!(BASETEN_DECISION_CONTEXT, 16_000);
    for known in BASETEN_DECISION_MODELS {
        assert_eq!(
            known.context_tokens, BASETEN_DECISION_CONTEXT,
            "{}",
            known.name
        );
        assert_eq!(
            ModelConfig::context_window_for(known.name),
            BASETEN_DECISION_CONTEXT,
            "{}",
            known.name
        );
    }
    assert_eq!(ModelConfig::context_window_for("tev1:0.8b"), 2_048);
    assert_eq!(ModelConfig::context_window_for("clef-flash:latest"), 16_384);
    let (ollama_decision, _) = providers()
        .await
        .into_iter()
        .find(|(meta, _)| meta.name == "ollama_decision")
        .expect("the Ollama decision provider is registered");
    assert!(
        ollama_decision.known_models.is_empty(),
        "{:?}",
        ollama_decision.known_models
    );
}
