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

use biorouter::model::ModelConfig;
use biorouter::providers::providers;

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
        .filter(|(_, model, _)| !ModelConfig::has_declared_context_window(model))
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

#[tokio::test]
async fn provider_declared_windows_match_the_registry() {
    let rows = advertised_models().await;
    let drifted: Vec<_> = rows
        .iter()
        .filter_map(|(provider, model, declared)| {
            let resolved = ModelConfig::context_window_for(model);
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
        // GPT-6: Astra is 1.05M, and "gpt-5"/"gpt-5.6" must not claim it.
        ("gpt-6-astra", 1_050_000),
        // Claude: 4.6+ and the 5 series are 1M; the 4.5 tier and Haiku are 200k.
        // Fable 5.1's 1M is the same number the `claude-fable-5` pattern would
        // hand it, so this case cannot tell an exact entry from an inherited
        // one — `every_advertised_model_has_its_own_declared_window` above is
        // what pins the exact entry.
        ("claude-fable-5-1", 1_000_000),
        ("claude-opus-5", 1_000_000),
        ("claude-opus-4-6", 1_000_000),
        ("claude-sonnet-5", 1_000_000),
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
