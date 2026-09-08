//! BioRouter's three effort tiers onto a coding-agent CLI's five-or-six rung
//! ladder.
//!
//! # Why these providers do not use `ReasoningEffort::provider_effort`
//!
//! The shared helper maps `Quick -> low`, `Deep -> high`, `Normal -> None`, which
//! is right for an API where `high` is the top of the scale. Both coding-agent
//! CLIs have a taller ladder — `low, medium, high, xhigh, max` — so `Deep` landing
//! on `high` would leave two rungs unused and make "deep" mean *less* here than the
//! word implies. These providers therefore climb their own ladder:
//!
//! | BioRouter | Claude Code | Codex |
//! |---|---|---|
//! | `Quick` | `low` | `low` |
//! | `Normal` (and unset) | `high` | `high` |
//! | `Deep` | `max` | the model's own top rung: `max` on Astra and 5.6, else `xhigh` |
//!
//! # Two consequences worth knowing before changing this
//!
//! **The default is no longer silent.** Elsewhere in Biorouter `Normal` means "say
//! nothing and let the model decide". Here it emits `high`, so *every* turn from a
//! user who never touched `/effort` asks for more reasoning than the vendor default
//! (`medium` on `gpt-5.5`). That is a deliberate product choice — a coding agent is
//! reached for when the work is hard — and it costs thinking tokens against the
//! user's own subscription on every turn.
//!
//! **`Normal` never actually arrives.** `Agent::effort_stamped_provider` returns
//! early when `effort.is_default()`, so the model config is not re-stamped and the
//! provider sees `None` rather than `Some(Normal)`. Matching only on
//! `Some(Normal)` would therefore be dead code and the middle rung would never
//! apply. Both spellings map to the same rung here, and a test pins it.

use crate::agents::effort::ReasoningEffort;

/// The rung `Deep` reaches on the Claude CLI.
///
/// Verified against `claude` 2.1.235: `--effort` accepts
/// `low, medium, high, xhigh, max`, one ladder for every model, and an unknown
/// value is **not** an error — the CLI warns and silently falls back to the
/// default, which would be a downgrade rather than a failure. So this must never
/// be a value the CLI does not know.
const CLAUDE_TOP: &str = "max";

/// The rung `Deep` reaches on a Codex model whose ladder we cannot confirm.
///
/// `xhigh` is the highest rung **every** model in the live catalogue advertises,
/// which is what makes it the safe floor: `max` and `ultra` exist only on part of
/// the 5.6 family and on Astra, and the two advertised models that stop here are
/// `gpt-5.5` and `gpt-5.3-codex-spark`.
const CODEX_SAFE_TOP: &str = "xhigh";

/// Codex models known to advertise `max`, from `model/list`'s
/// `supportedReasoningEfforts`.
///
/// A short static list rather than a live probe: `model/list` is a round-trip we
/// would otherwise pay on every turn, and being wrong in the safe direction costs
/// one rung. Re-derive it with
/// `codex app-server` → `model/list` → `supportedReasoningEfforts` when the
/// catalogue moves. Measured against codex-cli 0.153.4 on 2026-09-08: Astra,
/// Sol and Terra advertise `low, medium, high, xhigh, max, ultra`; Luna stops at
/// `max`; `gpt-5.5` and `gpt-5.3-codex-spark` stop at `xhigh`.
///
/// ⚠ Deliberately **not** reaching for `ultra`, which three of these also
/// advertise. `Deep` is the strongest ordinary tier; `ultra` is the delegating
/// mode above it and is not what `/effort deep` should silently buy.
///
/// ⚠ `codex-auto-review` used to sit here and has been removed. It is a hidden
/// review model: `codex exec -m codex-auto-review` is accepted on both 0.147.0
/// and 0.153.4, but it is absent from `model/list` on **both**, so its
/// `supportedReasoningEfforts` cannot be read and the `max` claim was never
/// evidence. Dropping it costs one rung on a model no picker offers — `Deep`
/// now sends `xhigh`, which every Codex model accepts.
const CODEX_MODELS_WITH_MAX: &[&str] = &[
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];

/// The `--effort` value for a Claude Code turn. Always emits: see the module
/// header on why the middle rung is not silence here.
pub fn claude_effort(effort: Option<ReasoningEffort>) -> &'static str {
    match effort {
        Some(ReasoningEffort::Quick) => "low",
        Some(ReasoningEffort::Deep) => CLAUDE_TOP,
        Some(ReasoningEffort::Normal) | None => "high",
    }
}

/// The `turn/start.effort` value for a Codex turn.
///
/// Takes the model because Codex's ladder is per-model, unlike Claude's.
pub fn codex_effort(effort: Option<ReasoningEffort>, model: &str) -> &'static str {
    match effort {
        Some(ReasoningEffort::Quick) => "low",
        Some(ReasoningEffort::Deep) => codex_top_rung(model),
        Some(ReasoningEffort::Normal) | None => "high",
    }
}

/// The strongest ordinary rung this Codex model has.
///
/// Sending an unadvertised value is *accepted* rather than rejected — measured on
/// `gpt-5.5`, which has no `max` — so the failure mode is not an error but a
/// silent clamp or a silent ignore, and we cannot tell which. An ignore would fall
/// back to the model's default and quietly deliver *less* than `Deep` asked for,
/// so we send only what the model advertises.
fn codex_top_rung(model: &str) -> &'static str {
    let model = model.trim().to_ascii_lowercase();
    if CODEX_MODELS_WITH_MAX.iter().any(|m| model == *m) {
        "max"
    } else {
        CODEX_SAFE_TOP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder, end to end. `Deep` reaching the top rung is the whole point of
    /// this module existing rather than reusing `provider_effort()`.
    #[test]
    fn the_ladder_climbs_all_the_way_on_claude() {
        assert_eq!(claude_effort(Some(ReasoningEffort::Quick)), "low");
        assert_eq!(claude_effort(Some(ReasoningEffort::Normal)), "high");
        assert_eq!(claude_effort(Some(ReasoningEffort::Deep)), "max");
    }

    /// `Normal` never reaches a provider — `effort_stamped_provider` returns early
    /// when the effort is default, so the config is not re-stamped and `None`
    /// arrives instead. If these two disagreed, the middle rung would silently
    /// never apply, which is exactly the bug this asserts against.
    #[test]
    fn unset_and_normal_are_the_same_rung() {
        assert_eq!(
            claude_effort(None),
            claude_effort(Some(ReasoningEffort::Normal))
        );
        assert_eq!(
            codex_effort(None, "gpt-5.5"),
            codex_effort(Some(ReasoningEffort::Normal), "gpt-5.5")
        );
        assert_eq!(claude_effort(None), "high");
    }

    /// Codex's ladder is per-model, and two of Biorouter's advertised models —
    /// `gpt-5.5` and `gpt-5.3-codex-spark` — stop at `xhigh`. Sending `max` to
    /// one of them is accepted but not observably applied, so `Deep` must not
    /// reach for it.
    #[test]
    fn codex_deep_never_exceeds_what_the_model_advertises() {
        for advertised in ["gpt-5.5", "gpt-5.3-codex-spark"] {
            assert_eq!(
                codex_effort(Some(ReasoningEffort::Deep), advertised),
                "xhigh",
                "{advertised} does not advertise `max`"
            );
        }
        for taller in [
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
        ] {
            assert_eq!(codex_effort(Some(ReasoningEffort::Deep), taller), "max");
        }
    }

    /// An unknown or user-typed model gets the safe floor rather than a guess —
    /// `with_unlisted_models` means anything can arrive here.
    #[test]
    fn an_unknown_model_falls_back_to_the_universally_supported_rung() {
        for unknown in ["gpt-6-does-not-exist", "", "   ", "GPT-5.5"] {
            let rung = codex_effort(Some(ReasoningEffort::Deep), unknown);
            assert!(
                rung == "xhigh" || rung == "max",
                "{unknown:?} produced {rung}"
            );
        }
        // …and casing must not change the answer for a model we do know.
        assert_eq!(
            codex_effort(Some(ReasoningEffort::Deep), "GPT-5.6-SOL"),
            "max"
        );
    }

    /// `ultra` is the delegating tier above the ordinary ladder. `/effort deep`
    /// must not silently buy it on the models that have it.
    #[test]
    fn deep_never_reaches_ultra() {
        for model in ["gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.5"] {
            assert_ne!(codex_effort(Some(ReasoningEffort::Deep), model), "ultra");
        }
        assert_ne!(claude_effort(Some(ReasoningEffort::Deep)), "ultra");
    }

    /// Quick is the bottom rung on both, so `/effort quick` means one thing.
    #[test]
    fn quick_is_the_bottom_rung_on_both() {
        assert_eq!(claude_effort(Some(ReasoningEffort::Quick)), "low");
        assert_eq!(codex_effort(Some(ReasoningEffort::Quick), "gpt-5.5"), "low");
    }
}
