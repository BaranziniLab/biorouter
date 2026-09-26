use crate::providers::base::ProviderMetadata;
use crate::providers::canonical::maybe_get_canonical_model;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderModelPricing {
    pub input_token_cost: f64,
    pub output_token_cost: f64,
    /// Cost **per token** of an input token served from the prompt cache, or
    /// `None` when the model has no separate cache-read rate. Same units as
    /// `input_token_cost`.
    pub cache_read_cost: Option<f64>,
    /// Cost **per token** of an input token written to the prompt cache, or
    /// `None`. Same units as `input_token_cost`.
    pub cache_write_cost: Option<f64>,
    pub currency: String,
    pub context_length: Option<u32>,
}

impl ProviderModelPricing {
    fn usd_per_million(input: f64, output: f64, context_length: u32) -> Self {
        Self {
            input_token_cost: input / 1_000_000.0,
            output_token_cost: output / 1_000_000.0,
            cache_read_cost: None,
            cache_write_cost: None,
            currency: "$".to_string(),
            context_length: Some(context_length),
        }
    }

    /// Anthropic-style pricing with prompt caching. Cache read is billed at
    /// **0.1x** the input rate and cache creation (write) at **1.25x**, the
    /// standard Claude multipliers (applied identically to Claude-on-Bedrock).
    fn usd_per_million_cached(input: f64, output: f64, context_length: u32) -> Self {
        Self::usd_per_million_with_cache(input, output, context_length, 0.1, 1.25)
    }

    /// Prompt-cached pricing with explicit multipliers of the input rate, for
    /// models that break the 0.1x / 1.25x convention. Anthropic's price sheet
    /// (checked 2026-09-25) bills cache hits at **0.05x** on Opus 5.5 and
    /// **0.025x** on Fable 5.1 / Mythos 5.1; their 5-minute writes stay 1.25x.
    fn usd_per_million_with_cache(
        input: f64,
        output: f64,
        context_length: u32,
        cache_read_multiplier: f64,
        cache_write_multiplier: f64,
    ) -> Self {
        Self {
            input_token_cost: input / 1_000_000.0,
            output_token_cost: output / 1_000_000.0,
            cache_read_cost: Some(input * cache_read_multiplier / 1_000_000.0),
            cache_write_cost: Some(input * cache_write_multiplier / 1_000_000.0),
            currency: "$".to_string(),
            context_length: Some(context_length),
        }
    }
}

/// Dollar cost of one turn including its cache buckets, plus a flag when the
/// model is priced but its cache tokens could not be (no cache rate on file).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnCost {
    pub cost: f64,
    /// `true` when this turn had cache tokens but the model has no cache pricing,
    /// so those tokens are **excluded** from `cost` (a lower bound, not exact).
    pub cache_excluded: bool,
}

/// Dollar cost of a turn (or a rollup of turns) for one `(provider, model)`,
/// or `None` when the pair has no known pricing.
///
/// `input_token_cost` / `output_token_cost` are the currency's cost **per
/// token** (`usd_per_million` already divided the per-million rate by 1e6), so
/// the cost is simply `tokens * per-token-cost`. This is the single shared
/// definition of "what a turn costs" — the server usage report, the summary
/// gauge, and any future caller price identically through it.
pub fn model_cost(
    provider: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
) -> Option<f64> {
    let pricing = provider_model_pricing(provider, model)?;
    Some(
        input_tokens as f64 * pricing.input_token_cost
            + output_tokens as f64 * pricing.output_token_cost,
    )
}

/// Dollar cost of a turn (or rollup) including its cache buckets. `None` when the
/// `(provider, model)` pair is unpriced. When the pair IS priced but has no cache
/// rate and the turn carried cache tokens, those tokens are left out of the cost
/// and `cache_excluded` is set so callers can flag the figure as a lower bound.
///
/// The buckets are the disjoint counts from `Usage`: `input_tokens` is fresh
/// (non-cached) input, and `cache_read` / `cache_creation` are additive.
pub fn model_cost_with_cache(
    provider: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
) -> Option<TurnCost> {
    let pricing = provider_model_pricing(provider, model)?;
    Some(cost_with_pricing(
        &pricing,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    ))
}

pub fn cost_with_pricing(
    pricing: &ProviderModelPricing,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
) -> TurnCost {
    let mut cost = input_tokens as f64 * pricing.input_token_cost
        + output_tokens as f64 * pricing.output_token_cost;
    let mut cache_excluded = false;

    if cache_read_tokens > 0 {
        match pricing.cache_read_cost {
            Some(rate) => cost += cache_read_tokens as f64 * rate,
            None => cache_excluded = true,
        }
    }
    if cache_creation_tokens > 0 {
        match pricing.cache_write_cost {
            Some(rate) => cost += cache_creation_tokens as f64 * rate,
            None => cache_excluded = true,
        }
    }

    TurnCost {
        cost,
        cache_excluded,
    }
}

pub fn provider_model_pricing(provider: &str, model: &str) -> Option<ProviderModelPricing> {
    let provider = provider.to_ascii_lowercase();
    let model = model.to_ascii_lowercase();
    let explicit = explicit_provider_model_pricing(&provider, &model);

    if explicit.is_some() || blocks_fallback_pricing(&provider) {
        return explicit;
    }

    canonical_model_pricing(&provider, &model)
}

pub async fn resolved_provider_model_pricing(
    provider: &str,
    model: &str,
) -> Option<ProviderModelPricing> {
    let provider = provider.to_ascii_lowercase();
    let model = model.to_ascii_lowercase();
    let metadata = crate::providers::providers()
        .await
        .into_iter()
        .find(|(metadata, _)| metadata.name.eq_ignore_ascii_case(&provider))
        .map(|(metadata, _)| metadata);
    resolve_pricing_with_metadata(&provider, &model, metadata.as_ref())
}

/// Resolution order for the async path: hardcoded-only providers
/// ([`blocks_fallback_pricing`]) → provider metadata → the explicit override
/// table → the canonical catalog. Provider metadata outranks the explicit
/// sync table so a declarative provider's JSON (e.g.
/// `providers/declarative/moonshot.json`) stays authoritative here; the sync
/// table in this file serves sync-only callers (CLI cost line, BR-35 budget)
/// and models the metadata doesn't price. The
/// `moonshot_sync_pricing_matches_declarative_metadata` drift guard keeps
/// the two sources aligned. `provider` and `model` are already lowercased.
fn resolve_pricing_with_metadata(
    provider: &str,
    model: &str,
    metadata: Option<&ProviderMetadata>,
) -> Option<ProviderModelPricing> {
    let explicit = explicit_provider_model_pricing(provider, model);
    if blocks_fallback_pricing(provider) {
        return explicit;
    }

    if let Some(pricing) =
        metadata.and_then(|metadata| pricing_from_provider_metadata(metadata, model))
    {
        return Some(pricing);
    }

    if explicit.is_some() {
        return explicit;
    }

    canonical_model_pricing(provider, model)
}

pub fn pricing_from_provider_metadata(
    metadata: &ProviderMetadata,
    model: &str,
) -> Option<ProviderModelPricing> {
    let model = metadata
        .known_models
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(model))?;
    Some(ProviderModelPricing {
        input_token_cost: model.input_token_cost?,
        output_token_cost: model.output_token_cost?,
        cache_read_cost: None,
        cache_write_cost: None,
        currency: model.currency.clone().unwrap_or_else(|| "$".to_string()),
        context_length: u32::try_from(model.context_limit).ok(),
    })
}

fn explicit_provider_model_pricing(provider: &str, model: &str) -> Option<ProviderModelPricing> {
    match provider {
        "openrouter" => openrouter_pricing(model),
        "groq" => groq_pricing(model),
        "zai" | "z.ai" | "z-ai" => zai_pricing(model),
        "xiaomi_mimo" | "xiaomi-mimo" | "xiaomi" => xiaomi_mimo_pricing(model),
        "custom_deepseek" | "deepseek" => deepseek_pricing(model),
        "moonshot" | "moonshotai" => moonshot_pricing(model),
        "inception" => inception_pricing(model),
        "mistral" | "mistralai" => mistral_pricing(model),
        "xai" | "x-ai" => xai_pricing(model),
        // Claude, natively and on Bedrock. Bedrock also serves non-Anthropic
        // models (Titan, Llama, …); `claude_family_pricing` returns None for
        // those, leaving them correctly unpriced.
        "anthropic" | "claude" | "bedrock" | "aws_bedrock" | "aws-bedrock" => {
            claude_family_pricing(model)
        }
        _ => None,
    }
}

fn blocks_fallback_pricing(provider: &str) -> bool {
    matches!(
        provider,
        "anthropic"
            | "claude"
            | "bedrock"
            | "aws_bedrock"
            | "aws-bedrock"
            | "ollama"
            | "llamacpp"
            | "github_copilot"
            // The last four are the CLI-agent providers, and they are here for
            // two different reasons. `codex` and `claude_code` are live
            // providers that are deliberately unpriced: they drive the user's
            // own installed CLI on the user's own subscription, so Biorouter
            // never sees a metered rate and any per-token figure would be
            // fabricated. `gemini_cli` and `cursor_agent` no longer exist, and
            // stay only because this function is keyed on a provider-name
            // *string*: `session_manager::resolve_grain_pricing` feeds it
            // `row.provider` straight out of stored usage rows, so a row
            // written while those two were wired up still says `gemini_cli`.
            // Either way, drop the entry and `canonical_model_pricing` invents
            // a price from the catalog for a run that billed a subscription.
            // Same reason as the guessed Claude tier below: a made-up rate
            // makes a partial report look exact and can misstate a budget.
            | "codex"
            | "claude_code"
            | "gemini_cli"
            | "cursor_agent"
    )
}

fn canonical_model_pricing(provider: &str, model: &str) -> Option<ProviderModelPricing> {
    let canonical = maybe_get_canonical_model(provider, model)?;
    Some(ProviderModelPricing {
        input_token_cost: canonical.pricing.prompt?,
        output_token_cost: canonical.pricing.completion?,
        cache_read_cost: None,
        cache_write_cost: None,
        currency: "$".to_string(),
        context_length: u32::try_from(canonical.context_length).ok(),
    })
}

/// First-party Claude list prices (USD per MTok), applied to the Claude API
/// and to Claude on Bedrock. Source: Anthropic's pricing page,
/// <https://platform.claude.com/docs/en/about-claude/pricing>, checked
/// 2026-09-25. `model` is already lowercased by the caller.
///
/// Matched on the parsed `(family, major, minor)` version rather than by
/// substring, because the substring match is what priced this table wrong
/// until 2026-09-25: `"opus-5"` is inside `claude-opus-5-5` (so Opus 5.5
/// billed at Opus 5's $5/$25), `"fable-5"` is inside `claude-fable-5-1`, and
/// the bare `"opus"` / `"haiku"` / `"sonnet"` catch-alls put Opus 4.5-4.8 on
/// Opus 4.1's legacy $15/$75 tier, Haiku 4.5 on Haiku 3.5's $0.80/$4, and
/// Sonnet 5 and 4.6 on $3/$15 with a 200K window. [`claude_version`]
/// reads native ids (`claude-opus-4-5-20251101`), Bedrock ids
/// (`us.anthropic.claude-opus-4-8`, `anthropic.claude-opus-4-20250514-v1:0`),
/// dotted spellings (`claude-opus-4.8`) and the Claude 3 word order
/// (`claude-3-5-haiku`) alike.
///
/// Never infer a price for an unknown Claude tier: a version this table does
/// not list (a future `claude-opus-5-6`, `claude-mythos-preview`) is `None`.
/// A guessed price makes a partial report look exact and silently misstates a
/// user's budget.
///
/// Bedrock's regional endpoints carry a 10% premium over global ones for
/// Claude Sonnet 4.5, Haiku 4.5, Opus 4.5 and every later model. The id does
/// say which one ran (`global.anthropic.…` is global; the `us.` / `eu.` /
/// `jp.` / `au.` / `apac.` geo profiles and a bare in-region `anthropic.…` id
/// are regional), but the premium is deliberately NOT modelled: every Bedrock
/// id bills the first-party rate here, so a turn on a regional or geo profile
/// reads about 9% low. An application inference-profile ARN names no model at
/// all and stays unpriced.
fn claude_family_pricing(model: &str) -> Option<ProviderModelPricing> {
    if !model.contains("claude") {
        return None;
    }
    let cached = ProviderModelPricing::usd_per_million_cached;
    let with_cache = ProviderModelPricing::usd_per_million_with_cache;
    let pricing = match claude_version(model)? {
        // Fable 5.1 / Mythos 5.1: cache hits at 0.025x ($0.25), not 0.1x.
        ("fable" | "mythos", 5, Some(1)) => with_cache(10.0, 50.0, 1_000_000, 0.025, 1.25),
        ("fable" | "mythos", 5, None) => cached(10.0, 50.0, 1_000_000),
        // Opus 5.5 (GA 2026-09-22) undercuts Opus 5, and its cache hits are
        // 0.05x ($0.20).
        ("opus", 5, Some(5)) => with_cache(4.0, 20.0, 1_000_000, 0.05, 1.25),
        ("opus", 5, None) => cached(5.0, 25.0, 1_000_000),
        ("opus", 4, Some(6..=8)) => cached(5.0, 25.0, 1_000_000),
        ("opus", 4, Some(5)) => cached(5.0, 25.0, 200_000),
        // Opus 4 / 4.1 and Claude 3 Opus: the legacy tier. All three are
        // retired on the Claude API; Opus 4.1 is still served on Bedrock and
        // Google Cloud and Opus 4 on Google Cloud, and stored usage names all
        // three.
        ("opus", 4, None | Some(0 | 1)) | ("opus", 3, None) => cached(15.0, 75.0, 200_000),
        // Sonnet 5's $2/$10 launch price is now its standard price: the
        // increase to $3/$15 scheduled for 2026-09-01 was cancelled.
        ("sonnet", 5, None) => cached(2.0, 10.0, 1_000_000),
        ("sonnet", 4, Some(6)) => cached(3.0, 15.0, 1_000_000),
        ("sonnet", 4, None | Some(0 | 5)) | ("sonnet", 3, None | Some(5 | 7)) => {
            cached(3.0, 15.0, 200_000)
        }
        ("haiku", 4, Some(5)) => cached(1.0, 5.0, 200_000),
        ("haiku", 3, Some(5)) => cached(0.80, 4.0, 200_000),
        ("haiku", 3, None) => cached(0.25, 1.25, 200_000),
        _ => return None,
    };
    Some(pricing)
}

/// The `(family, major, minor)` a Claude model id names, e.g.
/// `claude-opus-4-5-20251101` → `("opus", 4, Some(5))`,
/// `us.anthropic.claude-opus-5-5` → `("opus", 5, Some(5))`,
/// `claude-sonnet-4-20250514` → `("sonnet", 4, None)` and
/// `claude-3-5-haiku-20241022` → `("haiku", 3, Some(5))`.
///
/// The id is split on every non-alphanumeric character, so `.`, `-`, `@` and
/// `:` all separate tokens and the dotted and dashed spellings read the same.
/// A version number is a token of one or two digits: an eight-digit snapshot
/// date (`20250514`) is never mistaken for a minor version. `None` when the id
/// names no known family or carries no version at all.
fn claude_version(model: &str) -> Option<(&'static str, u32, Option<u32>)> {
    const FAMILIES: [&str; 5] = ["fable", "mythos", "opus", "sonnet", "haiku"];
    let version = |token: Option<&&str>| -> Option<u32> {
        token
            .filter(|t| (1..=2).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|t| t.parse().ok())
    };

    let tokens: Vec<&str> = model
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let at = tokens.iter().position(|t| FAMILIES.contains(t))?;
    let family = FAMILIES.into_iter().find(|f| *f == tokens[at])?;

    // Claude 4 and later put the version after the family (`opus-4-5`).
    if let Some(major) = version(tokens.get(at + 1)) {
        return Some((family, major, version(tokens.get(at + 2))));
    }
    // Claude 3 put it before (`claude-3-5-haiku`, `claude-3-opus`).
    let before = |back: usize| at.checked_sub(back).and_then(|i| version(tokens.get(i)));
    match (before(2), before(1)) {
        (Some(major), Some(minor)) => Some((family, major, Some(minor))),
        (None, Some(major)) => Some((family, major, None)),
        _ => None,
    }
}

fn openrouter_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        "deepseek/deepseek-v4-flash" => {
            Some(ProviderModelPricing::usd_per_million(0.09, 0.18, 1_048_576))
        }
        "inception/mercury-2" => Some(ProviderModelPricing::usd_per_million(0.25, 0.75, 128_000)),
        "minimax/minimax-m3" => Some(ProviderModelPricing::usd_per_million(0.30, 1.20, 1_048_576)),
        // Both Kimi rows re-read from OpenRouter's /models on 2026-09-25: k2.6
        // now lists at Moonshot's own $0.95/$4.00, and k2.7-code dropped below it.
        "moonshotai/kimi-k2.6" => Some(ProviderModelPricing::usd_per_million(0.95, 4.00, 262_144)),
        "moonshotai/kimi-k2.7-code" => {
            Some(ProviderModelPricing::usd_per_million(0.6562, 3.30, 262_144))
        }
        "x-ai/grok-build-0.1" => Some(ProviderModelPricing::usd_per_million(1.00, 2.00, 256_000)),
        "xiaomi/mimo-v2.5" => Some(ProviderModelPricing::usd_per_million(
            0.105, 0.28, 1_048_576,
        )),
        "xiaomi/mimo-v2.5-pro" => Some(ProviderModelPricing::usd_per_million(
            0.435, 0.87, 1_048_576,
        )),
        "z-ai/glm-5.1" => Some(ProviderModelPricing::usd_per_million(0.966, 3.036, 202_752)),
        "z-ai/glm-5.2" => Some(ProviderModelPricing::usd_per_million(
            0.9086, 2.8556, 1_048_576,
        )),
        _ => None,
    }
}

fn groq_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        "openai/gpt-oss-120b" => Some(ProviderModelPricing::usd_per_million(0.15, 0.60, 131_072)),
        "openai/gpt-oss-20b" | "openai/gpt-oss-safeguard-20b" => {
            Some(ProviderModelPricing::usd_per_million(0.075, 0.30, 131_072))
        }
        // Qwen3.8 27B (Groq preview, the named replacement for qwen3.6-27b,
        // which Groq shut down on 2026-09-14): $0.80/$4.00, text + images.
        "qwen/qwen3.8-27b" => Some(ProviderModelPricing::usd_per_million(0.80, 4.00, 131_072)),
        // Both Llama rows left Groq's free and developer tiers on 2026-08-16
        // (enterprise contracts only now). Kept so stored usage stays priced.
        "llama-3.1-8b-instant" => Some(ProviderModelPricing::usd_per_million(0.05, 0.08, 131_072)),
        "llama-3.3-70b-versatile" => {
            Some(ProviderModelPricing::usd_per_million(0.59, 0.79, 131_072))
        }
        _ => None,
    }
}

fn zai_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        // GLM-5.3 (GA 2026-08-18) and its multimodal Flash sibling
        // (2026-08-26), per docs.z.ai's price sheet on 2026-09-25.
        "glm-5.3" => Some(ProviderModelPricing::usd_per_million(1.40, 4.40, 1_048_576)),
        "glm-5.3-flash" => Some(ProviderModelPricing::usd_per_million(0.15, 0.50, 1_048_576)),
        "glm-5.2" => Some(ProviderModelPricing::usd_per_million(1.40, 4.40, 1_048_576)),
        "glm-5.1" => Some(ProviderModelPricing::usd_per_million(1.40, 4.40, 202_752)),
        "glm-5" => Some(ProviderModelPricing::usd_per_million(1.00, 3.20, 200_000)),
        "glm-5-turbo" => Some(ProviderModelPricing::usd_per_million(1.20, 4.00, 200_000)),
        "glm-4.7" | "glm-4.6" => Some(ProviderModelPricing::usd_per_million(0.60, 2.20, 200_000)),
        // Same price as 4.6/4.7, but a 128K window (Z.ai's overview; the
        // registry's 131_072), not the 200K this row used to share with them.
        "glm-4.5" => Some(ProviderModelPricing::usd_per_million(0.60, 2.20, 131_072)),
        "glm-4.5-air" => Some(ProviderModelPricing::usd_per_million(0.20, 1.10, 131_072)),
        _ => None,
    }
}

fn xiaomi_mimo_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        // V2.6 (2026-09-22) is priced exactly like the V2.5 pair it replaces.
        // Xiaomi shuts V2.5 down on 2026-10-21 with no reroute; its rows stay
        // so stored usage is still priced.
        "mimo-v2.6-flash" => Some(ProviderModelPricing::usd_per_million(0.14, 0.28, 1_048_576)),
        "mimo-v2.6-pro" => Some(ProviderModelPricing::usd_per_million(
            0.435, 0.87, 1_048_576,
        )),
        "mimo-v2.5" => Some(ProviderModelPricing::usd_per_million(0.14, 0.28, 1_000_000)),
        "mimo-v2.5-pro" => Some(ProviderModelPricing::usd_per_million(
            0.435, 0.87, 1_000_000,
        )),
        _ => None,
    }
}

/// DeepSeek bills by time of day since 2026-08-16: the rates below are the
/// off-peak base rates, and the peak windows (weekdays 01:00-04:00 and
/// 06:00-10:00 UTC) cost exactly twice as much. Nothing here knows when a turn
/// ran, so a peak-hour turn reads at half its real cost. Source:
/// <https://api-docs.deepseek.com/quick_start/pricing>, checked 2026-09-25.
fn deepseek_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        // DeepSeek-V4.1-Flash (2026-09-10), called `deepseek-flash`: $0.15/$0.60
        // off-peak, $0.30/$1.20 peak.
        "deepseek-flash" => Some(ProviderModelPricing::usd_per_million(0.15, 0.60, 1_000_000)),
        // None of these three is a model of its own any more. deepseek-chat and
        // deepseek-reasoner were discontinued on 2026-07-24, and V4-Flash was
        // retired on 2026-09-10; that name is temporarily routed to V4.1-Flash
        // and billed at its rate. The rows stay so stored usage is priced.
        "deepseek-v4-flash" | "deepseek-chat" | "deepseek-reasoner" => {
            Some(ProviderModelPricing::usd_per_million(0.15, 0.60, 1_000_000))
        }
        // V4-Pro off-peak (peak $1.32/$3.96).
        "deepseek-v4-pro" => Some(ProviderModelPricing::usd_per_million(0.66, 1.98, 1_000_000)),
        _ => None,
    }
}

/// Direct Moonshot AI (Kimi) platform rates as `(model, $/MTok input,
/// $/MTok output, context)` for the models `providers/declarative/moonshot.json`
/// ships, verified against the Moonshot/Kimi platform price sheet
/// (2026-07, re-checked 2026-09-25). Without this table the sync path fell
/// through to the OpenRouter-derived canonical prices, which underpriced
/// k2.6/k2.7-code at the time.
///
/// Kept as data (not match arms) so the drift guard —
/// `config::declarative_providers::tests::moonshot_sync_pricing_matches_declarative_metadata`
/// — can enumerate it and fail when this table and moonshot.json diverge in
/// EITHER direction (a model added, removed, or repriced on one side only).
/// The async resolved path prefers the JSON metadata; this table serves
/// sync-only callers and is the async fallback.
pub(crate) const MOONSHOT_SYNC_PRICING: &[(&str, f64, f64, u32)] = &[
    ("kimi-k2.7-code", 0.95, 4.00, 262_144),
    ("kimi-k2.6", 0.95, 4.00, 262_144),
];

/// Moonshot rows priced whether or not moonshot.json lists them, so the drift
/// guard checks them in one direction only: a row the JSON does list must
/// still match it exactly, but the JSON may leave a row out.
///
/// Two kinds live here. `kimi-k2.5` was discontinued on 2026-08-31 (it now
/// answers "model not found") and stays priced because stored usage rows
/// still name it; requiring it in the JSON would force a dead model back into
/// the picker. `kimi-k3` (GA 2026-07-16, $3/$15, 1M context) and
/// `kimi-k2.7-code-highspeed` ($1.90/$8.00) are Moonshot's current models,
/// verified 2026-09-25 and priced here ahead of the catalog: once moonshot.json
/// lists one, move its row into [`MOONSHOT_SYNC_PRICING`] so the guard covers
/// it both ways.
pub(crate) const MOONSHOT_UNLISTED_PRICING: &[(&str, f64, f64, u32)] = &[
    ("kimi-k3", 3.00, 15.00, 1_048_576),
    ("kimi-k2.7-code-highspeed", 1.90, 8.00, 262_144),
    ("kimi-k2.5", 0.60, 3.00, 262_144),
];

fn moonshot_pricing(model: &str) -> Option<ProviderModelPricing> {
    MOONSHOT_SYNC_PRICING
        .iter()
        .chain(MOONSHOT_UNLISTED_PRICING)
        .find(|(name, _, _, _)| *name == model)
        .map(|&(_, input, output, context)| {
            ProviderModelPricing::usd_per_million(input, output, context)
        })
}

fn inception_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        // Mercury 2.5 (2026-09-08) at its list price. A launch promotion bills
        // it 80% off ($0.04/$0.15) with no stated end date; the list price is
        // kept so a report never understates once the promotion lapses.
        "mercury-2.5" => Some(ProviderModelPricing::usd_per_million(0.20, 0.75, 260_000)),
        "mercury-2" => Some(ProviderModelPricing::usd_per_million(0.25, 0.75, 128_000)),
        // Mercury Coder is legacy (existing customers only) with a 128K window,
        // matching the registry; Mercury Edit 2 is the 32K FIM/edit model.
        "mercury-coder" => Some(ProviderModelPricing::usd_per_million(0.25, 0.75, 128_000)),
        "mercury-edit-2" => Some(ProviderModelPricing::usd_per_million(0.25, 0.75, 32_000)),
        _ => None,
    }
}

fn mistral_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        "mistral-medium-3-5" | "mistral-medium-latest" => {
            Some(ProviderModelPricing::usd_per_million(1.50, 7.50, 262_144))
        }
        "mistral-large-2512" => Some(ProviderModelPricing::usd_per_million(0.50, 1.50, 262_144)),
        "mistral-small-2603" => Some(ProviderModelPricing::usd_per_million(0.15, 0.60, 262_144)),
        // The Ministral 3 family (GA 2025-12-02, vision): 14B, 8B and 3B.
        "ministral-14b-2512" => Some(ProviderModelPricing::usd_per_million(0.20, 0.20, 262_144)),
        "ministral-8b-2512" => Some(ProviderModelPricing::usd_per_million(0.15, 0.15, 262_144)),
        "ministral-3b-2512" => Some(ProviderModelPricing::usd_per_million(0.10, 0.10, 262_144)),
        "codestral-2508" => Some(ProviderModelPricing::usd_per_million(0.30, 0.90, 128_000)),
        // Retired by Mistral (Devstral 2 and Magistral Medium 1.2 on
        // 2026-07-31, Medium 3.1 on 2026-08-31). Kept so stored usage is priced.
        "devstral-2512" => Some(ProviderModelPricing::usd_per_million(0.40, 2.00, 262_144)),
        "mistral-medium-2508" => Some(ProviderModelPricing::usd_per_million(0.40, 2.00, 128_000)),
        "magistral-medium-2509" => Some(ProviderModelPricing::usd_per_million(2.00, 5.00, 128_000)),
        _ => None,
    }
}

/// xAI bills a long-context tier once a prompt reaches 200K tokens (Grok
/// 4.5-4.7: $4/$12 instead of $2/$6). This table holds the short-context rate,
/// which is what almost every turn pays; a turn past 200K reads low.
fn xai_pricing(model: &str) -> Option<ProviderModelPricing> {
    match model {
        // Grok 4.5 (2026-07-08), 4.6 (2026-08-12) and 4.7 (2026-09-21), the
        // latter xAI's current flagship: 500K context per docs.x.ai.
        "grok-4.7" | "grok-4.6" | "grok-4.5" => {
            Some(ProviderModelPricing::usd_per_million(2.00, 6.00, 500_000))
        }
        "grok-4.3" | "grok-4.3-latest" | "grok-latest" => {
            Some(ProviderModelPricing::usd_per_million(1.25, 2.50, 1_000_000))
        }
        "grok-4.20-0309-reasoning"
        | "grok-4.20-0309-non-reasoning"
        | "grok-4.20-multi-agent-0309" => {
            Some(ProviderModelPricing::usd_per_million(1.25, 2.50, 1_000_000))
        }
        "grok-build-0.1" => Some(ProviderModelPricing::usd_per_million(1.00, 2.00, 256_000)),
        _ => None,
    }
}

/// Estimated dollar cost of one completion, or `None` when the model's price is
/// unknown (local models, subscription providers, an unrecognised model id).
///
/// Priced by [`provider_model_pricing`] and nothing else: provider-specific
/// overrides first, then the canonical model catalog, except for the providers
/// [`blocks_fallback_pricing`] keeps off the catalog. Lives here so the
/// per-reply dollar budget (BR-35) and the display paths can never disagree
/// about what a turn cost.
///
/// This used to fall back to the catalog on its own whenever
/// `provider_model_pricing` said `None`. For an unblocked provider that lookup
/// had already been made, so the fallback only ever fired for the blocked
/// ones, and there it priced exactly what the block exists to leave unpriced:
/// once the catalog gained `openai/gpt-6-sol`, a non-Claude Bedrock id
/// (`us.openai.gpt-6-sol`) got OpenAI's list price in the CLI cost line and
/// the budget while the usage report showed no cost for the same turn.
pub fn estimate_cost_usd(
    provider: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> Option<f64> {
    let pricing = provider_model_pricing(provider, model)?;
    Some(
        pricing.input_token_cost * input_tokens as f64
            + pricing.output_token_cost * output_tokens as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::ModelInfo;

    #[test]
    fn estimate_cost_prices_a_provider_override() {
        // groq/llama-3.1-8b-instant: $0.05/M in, $0.08/M out.
        let cost = estimate_cost_usd("groq", "llama-3.1-8b-instant", 1_000_000, 1_000_000).unwrap();
        assert!((cost - 0.13).abs() < 1e-9, "expected $0.13, got {cost}");
    }

    #[test]
    fn estimate_cost_is_none_for_an_unknown_model() {
        assert!(estimate_cost_usd("ollama", "llama3", 1_000, 1_000).is_none());
        assert!(estimate_cost_usd("nope", "not-a-model", 1_000, 1_000).is_none());
    }

    #[test]
    fn estimate_cost_never_prices_what_the_shared_resolver_leaves_unpriced() {
        // A non-Claude model on Bedrock is unpriced (blocks_fallback_pricing),
        // but the catalog does hold its first-party record: `us.openai.gpt-6-sol`
        // maps to openai/gpt-6-sol. The CLI cost line and the BR-35 budget must
        // agree with the usage report and leave it unpriced too.
        for (provider, model) in [
            ("bedrock", "us.openai.gpt-6-sol"),
            ("aws_bedrock", "us.openai.gpt-5.6-terra"),
        ] {
            assert!(
                crate::providers::canonical::maybe_get_canonical_model(provider, model).is_some(),
                "{provider}/{model}: the catalog must hold a record, or this proves nothing"
            );
            assert!(provider_model_pricing(provider, model).is_none());
            assert!(
                estimate_cost_usd(provider, model, 1_000_000, 1_000_000).is_none(),
                "{provider}/{model} must not be given an estimated cost"
            );
        }
        // And where the resolver does price, the estimate is its figure: Opus
        // 5.5 at $4 in + $20 out per MTok.
        let cost = estimate_cost_usd("anthropic", "claude-opus-5-5", 1_000_000, 1_000_000).unwrap();
        assert!((cost - 24.0).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn openrouter_uses_openrouter_specific_price() {
        let pricing = provider_model_pricing("openrouter", "deepseek/deepseek-v4-flash").unwrap();
        assert_eq!(pricing.input_token_cost, 0.09 / 1_000_000.0);
        assert_eq!(pricing.output_token_cost, 0.18 / 1_000_000.0);
        assert_eq!(pricing.context_length, Some(1_048_576));
    }

    #[test]
    fn direct_zai_uses_zai_price() {
        let pricing = provider_model_pricing("zai", "glm-5.2").unwrap();
        assert_eq!(pricing.input_token_cost, 1.40 / 1_000_000.0);
        assert_eq!(pricing.output_token_cost, 4.40 / 1_000_000.0);
    }

    #[test]
    fn canonical_models_are_resolved_through_the_shared_pricing_function() {
        let pricing = provider_model_pricing("openai", "gpt-4o").unwrap();
        assert_eq!(pricing.input_token_cost, 2.50 / 1_000_000.0);
        assert_eq!(pricing.output_token_cost, 10.00 / 1_000_000.0);
        assert_eq!(pricing.cache_read_cost, None);
        assert_eq!(pricing.cache_write_cost, None);
    }

    /// `(provider, model)` → `($/MTok in, $/MTok out, context)`.
    fn assert_priced(provider: &str, model: &str, (input, output, context): (f64, f64, u32)) {
        let p = provider_model_pricing(provider, model)
            .unwrap_or_else(|| panic!("{provider}/{model} must be priced"));
        assert!(
            (p.input_token_cost - input / 1_000_000.0).abs() < 1e-15,
            "{provider}/{model} input {}",
            p.input_token_cost
        );
        assert!(
            (p.output_token_cost - output / 1_000_000.0).abs() < 1e-15,
            "{provider}/{model} output {}",
            p.output_token_cost
        );
        assert_eq!(
            p.context_length,
            Some(context),
            "{provider}/{model} context"
        );
    }

    #[test]
    fn openais_current_lineup_is_priced_through_the_canonical_catalog() {
        // OpenAI and Azure have no explicit table here; they price through
        // canonical_models.json. Until 2026-09-25 the catalog had no GPT-5.6
        // or GPT-6 record, so OpenAI's own default model showed no cost. The
        // Azure ids are deployment-style (`<model>-<version date>`) and must
        // resolve to the same records.
        for (provider, model, want) in [
            ("openai", "gpt-6-astra", (10.0, 50.0, 1_050_000)),
            ("openai", "gpt-6-sol", (2.0, 10.0, 1_050_000)),
            ("openai", "gpt-6-luna", (0.10, 0.50, 1_050_000)),
            ("openai", "gpt-5.6", (4.0, 20.0, 1_050_000)),
            ("openai", "gpt-5.6-sol", (4.0, 20.0, 1_050_000)),
            ("openai", "gpt-5.6-terra", (2.0, 12.0, 1_050_000)),
            ("openai", "gpt-5.6-luna", (0.20, 1.20, 1_050_000)),
            (
                "azure_openai",
                "gpt-6-sol-2026-09-22",
                (2.0, 10.0, 1_050_000),
            ),
            (
                "azure_openai",
                "gpt-6-astra-2026-09-03",
                (10.0, 50.0, 1_050_000),
            ),
            (
                "azure_openai",
                "gpt-6-luna-2026-09-22",
                (0.10, 0.50, 1_050_000),
            ),
            (
                "azure_openai",
                "gpt-5.6-terra-2026-07-09",
                (2.0, 12.0, 1_050_000),
            ),
            (
                "versa_azure",
                "gpt-6-sol-2026-09-22",
                (2.0, 10.0, 1_050_000),
            ),
        ] {
            assert_priced(provider, model, want);
        }
    }

    #[test]
    fn new_hosted_models_are_priced_through_the_canonical_catalog() {
        // Gemini and the OpenRouter slugs with no explicit row fall through to
        // the catalog, which carries the vendor's list price.
        for (provider, model, want) in [
            ("google", "gemini-3.8-flash", (0.75, 3.75, 1_048_576)),
            ("google", "gemini-3.7-flash", (0.75, 3.75, 1_048_576)),
            ("google", "gemini-3.6-flash", (0.75, 3.75, 1_048_576)),
            ("google", "gemini-3.5-flash-lite", (0.30, 2.50, 1_048_576)),
            ("gcp_vertex_ai", "gemini-3.8-flash", (0.75, 3.75, 1_048_576)),
            ("openrouter", "openai/gpt-6-sol", (2.0, 10.0, 1_050_000)),
            (
                "openrouter",
                "anthropic/claude-opus-5.5",
                (4.0, 20.0, 1_000_000),
            ),
            (
                "openrouter",
                "anthropic/claude-fable-5.1",
                (10.0, 50.0, 1_000_000),
            ),
            (
                "openrouter",
                "google/gemini-3.8-flash",
                (0.75, 3.75, 1_048_576),
            ),
            ("openrouter", "x-ai/grok-4.7", (2.0, 6.0, 500_000)),
            ("openrouter", "moonshotai/kimi-k3", (3.0, 15.0, 1_048_576)),
            ("openrouter", "z-ai/glm-5.3", (1.40, 4.40, 1_048_576)),
            (
                "openrouter",
                "deepseek/deepseek-v4.1-flash",
                (0.15, 0.60, 1_048_576),
            ),
        ] {
            assert_priced(provider, model, want);
        }
    }

    #[test]
    fn direct_provider_tables_price_the_september_2026_models() {
        for (provider, model, want) in [
            ("xai", "grok-4.7", (2.00, 6.00, 500_000)),
            ("xai", "grok-4.6", (2.00, 6.00, 500_000)),
            ("xai", "grok-4.5", (2.00, 6.00, 500_000)),
            ("custom_deepseek", "deepseek-flash", (0.15, 0.60, 1_000_000)),
            (
                "custom_deepseek",
                "deepseek-v4-pro",
                (0.66, 1.98, 1_000_000),
            ),
            ("groq", "qwen/qwen3.8-27b", (0.80, 4.00, 131_072)),
            ("mistral", "ministral-14b-2512", (0.20, 0.20, 262_144)),
            ("mistral", "ministral-3b-2512", (0.10, 0.10, 262_144)),
            ("inception", "mercury-2.5", (0.20, 0.75, 260_000)),
            ("zai", "glm-5.3", (1.40, 4.40, 1_048_576)),
            ("zai", "glm-5.3-flash", (0.15, 0.50, 1_048_576)),
            ("xiaomi_mimo", "mimo-v2.6-flash", (0.14, 0.28, 1_048_576)),
            ("xiaomi_mimo", "mimo-v2.6-pro", (0.435, 0.87, 1_048_576)),
        ] {
            assert_priced(provider, model, want);
        }
    }

    #[test]
    fn deepseeks_retired_names_bill_at_the_flash_rate() {
        // DeepSeek routes the retired V4-Flash name to V4.1-Flash and bills it
        // at Flash's rate; the discontinued chat/reasoner aliases keep a price
        // only so stored usage rows are not dropped from a report.
        let flash = provider_model_pricing("deepseek", "deepseek-flash").unwrap();
        for retired in ["deepseek-v4-flash", "deepseek-chat", "deepseek-reasoner"] {
            assert_eq!(
                provider_model_pricing("deepseek", retired).as_ref(),
                Some(&flash),
                "{retired}"
            );
        }
    }

    #[test]
    fn retired_models_stay_priced_for_stored_usage() {
        // Usage rows record the model they ran on, and a report prices them
        // when it is read. A model the vendor retired must keep its row, or
        // every past turn on it silently falls out of the total.
        for (provider, model) in [
            ("moonshot", "kimi-k2.5"),
            ("mistral", "devstral-2512"),
            ("mistral", "magistral-medium-2509"),
            ("mistral", "mistral-medium-2508"),
            ("xiaomi_mimo", "mimo-v2.5"),
            ("xiaomi_mimo", "mimo-v2.5-pro"),
            ("inception", "mercury-coder"),
            ("groq", "llama-3.3-70b-versatile"),
        ] {
            assert!(
                provider_model_pricing(provider, model).is_some(),
                "{provider}/{model} must stay priced"
            );
        }
    }

    #[test]
    fn direct_moonshot_uses_direct_platform_prices_for_all_shipped_models() {
        // The sync path (CLI cost line, /config/pricing, BR-35 budget) must
        // serve the direct-platform rates for every Moonshot model — the
        // listed ones, the current ones priced ahead of the catalog, and the
        // discontinued k2.5 that stored usage rows still name — never fall
        // through to the OpenRouter-derived canonical record.
        for (model, want_in, want_out, want_ctx) in [
            ("kimi-k3", 3.00, 15.00, 1_048_576),
            ("kimi-k2.7-code-highspeed", 1.90, 8.00, 262_144),
            ("kimi-k2.7-code", 0.95, 4.00, 262_144),
            ("kimi-k2.6", 0.95, 4.00, 262_144),
            ("kimi-k2.5", 0.60, 3.00, 262_144),
        ] {
            let p = provider_model_pricing("moonshot", model).unwrap();
            assert_eq!(p.input_token_cost, want_in / 1_000_000.0, "{model}");
            assert_eq!(p.output_token_cost, want_out / 1_000_000.0, "{model}");
            assert_eq!(p.context_length, Some(want_ctx), "{model}");
        }
        // OpenRouter keeps its own (hosted) rates for the same models. The
        // two are separate price sheets that happen to agree on k2.6 today
        // and disagree on k2.7-code.
        let hosted = provider_model_pricing("openrouter", "moonshotai/kimi-k2.6").unwrap();
        assert_eq!(hosted.input_token_cost, 0.95 / 1_000_000.0);
        assert_eq!(hosted.output_token_cost, 4.00 / 1_000_000.0);
        let hosted = provider_model_pricing("openrouter", "moonshotai/kimi-k2.7-code").unwrap();
        assert_eq!(hosted.input_token_cost, 0.6562 / 1_000_000.0);
        assert_eq!(hosted.output_token_cost, 3.30 / 1_000_000.0);
    }

    #[test]
    fn a_moonshot_model_is_priced_by_exactly_one_table() {
        // moonshot_pricing chains the two tables and takes the first hit, so a
        // name in both would silently let the guarded row shadow the other.
        for &(name, _, _, _) in MOONSHOT_UNLISTED_PRICING {
            assert!(
                !MOONSHOT_SYNC_PRICING.iter().any(|(n, _, _, _)| *n == name),
                "{name} is in both Moonshot pricing tables"
            );
        }
    }

    #[test]
    fn kimi_k2_5_canonical_record_prices_hosted_access() {
        // k2.5 has no OpenRouter-specific override, so this exercises the
        // canonical registry record end-to-end via the fallback path — the
        // record any hosting provider's usage attribution resolves through.
        let p = provider_model_pricing("openrouter", "moonshotai/kimi-k2.5").unwrap();
        assert!((p.input_token_cost - 0.60 / 1_000_000.0).abs() < 1e-15);
        assert!((p.output_token_cost - 3.00 / 1_000_000.0).abs() < 1e-15);
        assert_eq!(p.context_length, Some(262_144));
    }

    #[test]
    fn async_resolution_prefers_declarative_metadata_over_sync_override() {
        // The async path treats provider metadata (declarative JSON, e.g.
        // moonshot.json) as authoritative: when the metadata prices a model,
        // that pricing wins even though the sync table also carries an
        // explicit entry for the same (provider, model).
        let metadata = ProviderMetadata::with_models(
            "moonshot",
            "Moonshot AI (Kimi)",
            "test",
            "kimi-k2.6",
            vec![ModelInfo::with_cost(
                "kimi-k2.6",
                262_144,
                0.000_001,
                0.000_005,
            )],
            "",
            vec![],
        );
        let p = resolve_pricing_with_metadata("moonshot", "kimi-k2.6", Some(&metadata)).unwrap();
        assert_eq!(
            p.input_token_cost, 0.000_001,
            "metadata outranks sync table"
        );
        assert_eq!(p.output_token_cost, 0.000_005);

        // Without metadata the explicit sync table is the fallback…
        let p = resolve_pricing_with_metadata("moonshot", "kimi-k2.6", None).unwrap();
        assert_eq!(p.input_token_cost, 0.95 / 1_000_000.0);
        assert_eq!(p.output_token_cost, 4.00 / 1_000_000.0);
        // …including for a model the metadata does not price.
        let p = resolve_pricing_with_metadata("moonshot", "kimi-k2.5", Some(&metadata)).unwrap();
        assert_eq!(p.input_token_cost, 0.60 / 1_000_000.0);
    }

    #[test]
    fn blocked_provider_ignores_metadata_pricing_on_the_async_path() {
        // Providers in blocks_fallback_pricing (Anthropic & co) stay
        // hardcoded-only: metadata must not price an unknown tier, and the
        // explicit table keeps serving the known ones. `claude-opus-9` is a
        // made-up version of a real family, the shape a guess would latch on to.
        let metadata = ProviderMetadata::with_models(
            "anthropic",
            "Anthropic",
            "test",
            "claude-opus-9",
            vec![ModelInfo::with_cost(
                "claude-opus-9",
                200_000,
                0.000_001,
                0.000_005,
            )],
            "",
            vec![],
        );
        assert!(
            resolve_pricing_with_metadata("anthropic", "claude-opus-9", Some(&metadata)).is_none()
        );
        let p =
            resolve_pricing_with_metadata("anthropic", "claude-opus-5", Some(&metadata)).unwrap();
        assert_eq!(p.input_token_cost, 5.0 / 1_000_000.0);
    }

    #[test]
    fn declarative_provider_metadata_resolves_configured_prices() {
        let metadata = ProviderMetadata::with_models(
            "custom_acme",
            "Acme",
            "test",
            "acme-model",
            vec![ModelInfo::with_cost(
                "acme-model",
                32_000,
                0.000_002,
                0.000_008,
            )],
            "",
            vec![],
        );
        let pricing = pricing_from_provider_metadata(&metadata, "acme-model").unwrap();
        assert_eq!(pricing.input_token_cost, 0.000_002);
        assert_eq!(pricing.output_token_cost, 0.000_008);
        assert_eq!(pricing.context_length, Some(32_000));
    }

    #[test]
    fn local_or_subscription_models_are_unpriced() {
        assert!(provider_model_pricing("ollama", "llama3").is_none());
        assert!(provider_model_pricing("github_copilot", "gpt-5.5").is_none());
    }

    #[test]
    fn model_cost_multiplies_tokens_by_per_token_rate() {
        // glm-5.2 on zai: $1.40 / 1M input, $4.40 / 1M output.
        // Hand-computed for 2,000,000 input + 500,000 output tokens:
        //   input:  2_000_000 * 1.40 / 1_000_000 = 2.80
        //   output:   500_000 * 4.40 / 1_000_000 = 2.20
        //   total = 5.00
        let cost = model_cost("zai", "glm-5.2", 2_000_000, 500_000).unwrap();
        assert!((cost - 5.00).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn model_cost_is_none_for_unpriced_pair() {
        assert!(model_cost("ollama", "llama3", 1_000, 1_000).is_none());
    }

    #[test]
    fn model_cost_of_zero_tokens_is_zero_not_none_for_priced_model() {
        assert_eq!(model_cost("zai", "glm-5.2", 0, 0), Some(0.0));
    }

    #[test]
    fn claude_sonnet_priced_natively_and_on_bedrock() {
        // Sonnet: $3/M input, $15/M output; cache read 0.1x = $0.30/M,
        // cache write 1.25x = $3.75/M.
        for (provider, model) in [
            ("anthropic", "claude-sonnet-4-20250514"),
            ("bedrock", "us.anthropic.claude-sonnet-4-20250514-v1:0"),
        ] {
            let p = provider_model_pricing(provider, model).unwrap();
            assert_eq!(p.input_token_cost, 3.0 / 1_000_000.0);
            assert_eq!(p.output_token_cost, 15.0 / 1_000_000.0);
            // Cache read = 0.1x input, write = 1.25x input (float-tolerant).
            assert!((p.cache_read_cost.unwrap() - 0.30 / 1_000_000.0).abs() < 1e-15);
            assert!((p.cache_write_cost.unwrap() - 3.75 / 1_000_000.0).abs() < 1e-15);
        }
    }

    /// Asserts one Claude id's full price record: $/MTok in and out, context,
    /// and the cache read / write rates in $/MTok.
    fn assert_claude_price(
        provider: &str,
        model: &str,
        (input, output, context, cache_read, cache_write): (f64, f64, u32, f64, f64),
    ) {
        let p = provider_model_pricing(provider, model)
            .unwrap_or_else(|| panic!("{provider}/{model} must be priced"));
        let per_token = |usd_per_mtok: f64| usd_per_mtok / 1_000_000.0;
        let close = |got: f64, want: f64| (got - per_token(want)).abs() < 1e-15;
        assert!(close(p.input_token_cost, input), "{provider}/{model} input");
        assert!(
            close(p.output_token_cost, output),
            "{provider}/{model} output"
        );
        assert_eq!(
            p.context_length,
            Some(context),
            "{provider}/{model} context"
        );
        assert!(
            close(p.cache_read_cost.unwrap(), cache_read),
            "{provider}/{model} cache read {:?}",
            p.cache_read_cost
        );
        assert!(
            close(p.cache_write_cost.unwrap(), cache_write),
            "{provider}/{model} cache write {:?}",
            p.cache_write_cost
        );
    }

    #[test]
    fn every_claude_tier_bills_anthropics_published_rate() {
        // Anthropic's pricing page, 2026-09-25: (in, out, context, cache hit,
        // 5-minute cache write), all $/MTok. One row per tier, native id.
        for (model, want) in [
            ("claude-fable-5-1", (10.0, 50.0, 1_000_000, 0.25, 12.50)),
            ("claude-mythos-5-1", (10.0, 50.0, 1_000_000, 0.25, 12.50)),
            ("claude-fable-5", (10.0, 50.0, 1_000_000, 1.00, 12.50)),
            ("claude-mythos-5", (10.0, 50.0, 1_000_000, 1.00, 12.50)),
            ("claude-opus-5-5", (4.0, 20.0, 1_000_000, 0.20, 5.00)),
            ("claude-opus-5", (5.0, 25.0, 1_000_000, 0.50, 6.25)),
            ("claude-opus-4-8", (5.0, 25.0, 1_000_000, 0.50, 6.25)),
            ("claude-opus-4-7", (5.0, 25.0, 1_000_000, 0.50, 6.25)),
            ("claude-opus-4-6", (5.0, 25.0, 1_000_000, 0.50, 6.25)),
            ("claude-opus-4-5-20251101", (5.0, 25.0, 200_000, 0.50, 6.25)),
            (
                "claude-opus-4-1-20250805",
                (15.0, 75.0, 200_000, 1.50, 18.75),
            ),
            ("claude-opus-4-20250514", (15.0, 75.0, 200_000, 1.50, 18.75)),
            ("claude-opus-4-0", (15.0, 75.0, 200_000, 1.50, 18.75)),
            ("claude-3-opus-20240229", (15.0, 75.0, 200_000, 1.50, 18.75)),
            ("claude-sonnet-5", (2.0, 10.0, 1_000_000, 0.20, 2.50)),
            ("claude-sonnet-4-6", (3.0, 15.0, 1_000_000, 0.30, 3.75)),
            (
                "claude-sonnet-4-5-20250929",
                (3.0, 15.0, 200_000, 0.30, 3.75),
            ),
            ("claude-sonnet-4-20250514", (3.0, 15.0, 200_000, 0.30, 3.75)),
            (
                "claude-3-7-sonnet-20250219",
                (3.0, 15.0, 200_000, 0.30, 3.75),
            ),
            ("claude-haiku-4-5-20251001", (1.0, 5.0, 200_000, 0.10, 1.25)),
            (
                "claude-3-5-haiku-20241022",
                (0.80, 4.0, 200_000, 0.08, 1.00),
            ),
            (
                "claude-3-haiku-20240307",
                (0.25, 1.25, 200_000, 0.025, 0.3125),
            ),
        ] {
            assert_claude_price("anthropic", model, want);
        }
    }

    #[test]
    fn bedrock_claude_ids_bill_the_first_party_rate() {
        // Bedrock ids carry a region/`anthropic.` prefix and, before Opus 4.7,
        // a `-v1:0` / `-v1` suffix. The 10% regional-endpoint premium is not
        // modelled, so the `us.` profiles price exactly like the native ids.
        for (model, want) in [
            (
                "us.anthropic.claude-opus-5-5",
                (4.0, 20.0, 1_000_000, 0.20, 5.00),
            ),
            (
                "anthropic.claude-opus-5-5",
                (4.0, 20.0, 1_000_000, 0.20, 5.00),
            ),
            (
                "us.anthropic.claude-fable-5-1",
                (10.0, 50.0, 1_000_000, 0.25, 12.50),
            ),
            (
                "us.anthropic.claude-opus-5",
                (5.0, 25.0, 1_000_000, 0.50, 6.25),
            ),
            (
                "us.anthropic.claude-opus-5-v1:0",
                (5.0, 25.0, 1_000_000, 0.50, 6.25),
            ),
            (
                "us.anthropic.claude-opus-4-8",
                (5.0, 25.0, 1_000_000, 0.50, 6.25),
            ),
            (
                "us.anthropic.claude-opus-4-6-v1",
                (5.0, 25.0, 1_000_000, 0.50, 6.25),
            ),
            (
                "us.anthropic.claude-opus-4-5-20251101-v1:0",
                (5.0, 25.0, 200_000, 0.50, 6.25),
            ),
            (
                "anthropic.claude-opus-4-20250514-v1:0",
                (15.0, 75.0, 200_000, 1.50, 18.75),
            ),
            (
                "us.anthropic.claude-sonnet-5",
                (2.0, 10.0, 1_000_000, 0.20, 2.50),
            ),
            (
                "us.anthropic.claude-sonnet-4-6",
                (3.0, 15.0, 1_000_000, 0.30, 3.75),
            ),
            (
                "us.anthropic.claude-haiku-4-5-20251001-v1:0",
                (1.0, 5.0, 200_000, 0.10, 1.25),
            ),
            (
                "anthropic.claude-3-5-haiku-20241022-v1:0",
                (0.80, 4.0, 200_000, 0.08, 1.00),
            ),
        ] {
            assert_claude_price("bedrock", model, want);
        }
    }

    #[test]
    fn a_claude_version_inside_a_longer_one_does_not_match_it() {
        // The substring traps that priced this table wrong until 2026-09-25:
        // "opus-5" is inside "opus-5-5", "fable-5" inside "fable-5-1" and
        // "mythos-5" inside "mythos-5-1". Each longer id must get its own
        // rate, never its prefix's.
        let opus_5_5 = provider_model_pricing("anthropic", "claude-opus-5-5").unwrap();
        let opus_5 = provider_model_pricing("anthropic", "claude-opus-5").unwrap();
        assert_ne!(opus_5_5.input_token_cost, opus_5.input_token_cost);
        for (longer, shorter) in [
            ("claude-fable-5-1", "claude-fable-5"),
            ("claude-mythos-5-1", "claude-mythos-5"),
        ] {
            let longer = provider_model_pricing("anthropic", longer).unwrap();
            let shorter = provider_model_pricing("anthropic", shorter).unwrap();
            assert_ne!(longer.cache_read_cost, shorter.cache_read_cost);
        }
        // And the reverse: 4.5-generation ids do not contain the 5.x names
        // ("opus-4-5" is not "opus-5", "sonnet-4-5" is not "sonnet-5").
        let opus_4_5 = provider_model_pricing("anthropic", "claude-opus-4-5").unwrap();
        assert_eq!(opus_4_5.context_length, Some(200_000));
        let sonnet_4_5 = provider_model_pricing("anthropic", "claude-sonnet-4-5").unwrap();
        assert_eq!(sonnet_4_5.input_token_cost, 3.0 / 1_000_000.0);
        // A snapshot date is not a minor version: 20250514 must not read as
        // Opus 4.2 or Sonnet 4.2.
        let sonnet_4 = provider_model_pricing("anthropic", "claude-sonnet-4-20250514").unwrap();
        assert_eq!(sonnet_4.context_length, Some(200_000));
    }

    #[test]
    fn dotted_claude_spellings_price_like_the_dashed_ids() {
        // GitHub Copilot and OpenRouter spell versions with a dot. Same model,
        // same price.
        for (dotted, dashed) in [
            ("claude-opus-5.5", "claude-opus-5-5"),
            ("claude-fable-5.1", "claude-fable-5-1"),
            ("claude-opus-4.8", "claude-opus-4-8"),
            ("claude-opus-4.5", "claude-opus-4-5"),
            ("claude-sonnet-4.6", "claude-sonnet-4-6"),
            ("claude-haiku-4.5", "claude-haiku-4-5"),
            ("claude-3.5-haiku", "claude-3-5-haiku"),
            ("claude-3.7-sonnet", "claude-3-7-sonnet"),
        ] {
            assert_eq!(
                provider_model_pricing("anthropic", dotted),
                provider_model_pricing("anthropic", dashed),
                "{dotted} vs {dashed}"
            );
            assert!(
                provider_model_pricing("anthropic", dotted).is_some(),
                "{dotted}"
            );
        }
    }

    #[test]
    fn unknown_claude_tier_is_unpriced_instead_of_assumed_sonnet() {
        // An unknown family, and unknown versions of real families: each must
        // stay unpriced rather than borrow the nearest known tier's rate.
        for model in [
            "claude-foo-9",
            "claude-opus-9",
            "claude-opus-5-6",
            "claude-sonnet-5-1",
            "claude-haiku-5",
            "claude-fable-5-2",
            "claude-mythos-preview",
            "claude-instant-1.2",
        ] {
            assert!(
                provider_model_pricing("anthropic", model).is_none(),
                "{model} must stay unpriced"
            );
        }
        assert!(provider_model_pricing("bedrock", "us.anthropic.claude-opus-5-6").is_none());
        assert!(provider_model_pricing("bedrock", "us.anthropic.claude-foo-9-v1:0").is_none());
    }

    #[test]
    fn non_claude_bedrock_model_is_unpriced() {
        assert!(provider_model_pricing("bedrock", "amazon.titan-text-express-v1").is_none());
        assert!(provider_model_pricing("bedrock", "meta.llama3-70b-instruct-v1:0").is_none());
    }

    #[test]
    fn model_cost_with_cache_prices_every_bucket() {
        // Sonnet rates: input $3/M, output $15/M, cache_read $0.30/M,
        // cache_write $3.75/M. Hand-computed for:
        //   input 1,000,000  -> 1,000,000 * 3.00 / 1e6 = 3.00
        //   output 200,000   ->   200,000 * 15.0 / 1e6 = 3.00
        //   cache_read 500,000 -> 500,000 * 0.30 / 1e6 = 0.15
        //   cache_creation 100,000 -> 100,000 * 3.75 / 1e6 = 0.375
        //   total = 6.525
        let tc = model_cost_with_cache(
            "anthropic",
            "claude-sonnet-4-20250514",
            1_000_000,
            200_000,
            500_000,
            100_000,
        )
        .unwrap();
        assert!((tc.cost - 6.525).abs() < 1e-9, "got {}", tc.cost);
        assert!(!tc.cache_excluded);
    }

    #[test]
    fn model_cost_with_cache_flags_excluded_when_model_has_no_cache_rate() {
        // zai has input/output rates but no cache pricing. A turn with cache
        // tokens prices only input+output and flags the exclusion.
        // glm-5.2: input $1.40/M, output $4.40/M.
        //   input 1,000,000 -> 1.40 ; output 0 -> 0 ; cache omitted.
        let tc = model_cost_with_cache("zai", "glm-5.2", 1_000_000, 0, 900_000, 0).unwrap();
        assert!((tc.cost - 1.40).abs() < 1e-9, "got {}", tc.cost);
        assert!(tc.cache_excluded);
    }

    #[test]
    fn model_cost_with_cache_no_flag_when_no_cache_tokens() {
        let tc = model_cost_with_cache("zai", "glm-5.2", 1_000_000, 0, 0, 0).unwrap();
        assert!(!tc.cache_excluded);
    }

    #[test]
    fn model_cost_with_cache_is_none_for_unpriced_pair() {
        assert!(model_cost_with_cache("ollama", "llama3", 10, 10, 5, 5).is_none());
    }

    /// Two of these are live providers (`claude_code`, `codex`) and two are
    /// historical (`gemini_cli`, `cursor_agent`, whose names survive only in
    /// stored usage rows — `session_manager::resolve_grain_pricing` reads
    /// `row.provider` straight back out of the `provider` column). In both
    /// cases a per-token price would be fabricated, because the run billed the
    /// user's own CLI subscription rather than a metered API. So all four must
    /// stay unpriced on every entry point — including `estimate_cost_usd`,
    /// which once reached the canonical catalog on its own rather than through
    /// `blocks_fallback_pricing`.
    #[test]
    fn a_cli_agent_provider_is_never_priced_on_any_entry_point() {
        for (provider, model) in [
            ("claude_code", "claude-sonnet-4-20250514"),
            ("codex", "gpt-5.5"),
            ("gemini_cli", "gemini-2.5-flash"),
            ("cursor_agent", "gpt-5"),
        ] {
            assert!(
                provider_model_pricing(provider, model).is_none(),
                "{provider}/{model} must not be priced"
            );
            assert!(
                resolve_pricing_with_metadata(provider, model, None).is_none(),
                "{provider}/{model} must not be priced on the async path"
            );
            assert!(
                estimate_cost_usd(provider, model, 1_000_000, 1_000_000).is_none(),
                "{provider}/{model} must not be given an estimated cost"
            );
        }

        // The two live providers are registered, so the async path really does
        // hand `resolve_pricing_with_metadata` their `ProviderMetadata` — the
        // `None` case above does not cover the path production takes. The
        // metadata is built inline rather than pulled from the provider modules
        // so this test stays a test of *pricing*: it must fail if the block is
        // dropped, whatever those modules happen to declare, and priced models
        // here are the hostile input (a real provider listing a cost is exactly
        // the regression to catch).
        for (provider, model) in [
            ("claude_code", "claude-sonnet-4-20250514"),
            ("codex", "gpt-5.5"),
        ] {
            let metadata = ProviderMetadata::with_models(
                provider,
                provider,
                "test",
                model,
                vec![ModelInfo::with_cost(model, 200_000, 0.000_001, 0.000_005)],
                "",
                vec![],
            );
            assert!(
                resolve_pricing_with_metadata(provider, model, Some(&metadata)).is_none(),
                "{provider}/{model} must not be priced from provider metadata"
            );
        }
    }
}
