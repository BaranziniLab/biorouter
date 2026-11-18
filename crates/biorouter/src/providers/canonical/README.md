# Canonical Model System

Provides a unified view of model metadata (pricing, capabilities, context limits) across different LLM providers. 
Normalizes provider-specific model names (e.g., `claude-3-5-sonnet-20241022`) 
to canonical IDs (e.g., `anthropic/claude-3.5-sonnet`).

## Build Canonical Models
Fetches latest model metadata from OpenRouter and validates provider mappings:
```bash
cargo run --bin build_canonical_models              # Build and check (default)
cargo run --bin build_canonical_models --no-check   # Build only, skip checker
```

This script performs two operations by default:
1. **Builds canonical models** - Fetches from OpenRouter API and updates the registry
   - Writes to: `src/providers/canonical/data/canonical_models.json`
2. **Checks model mappings** (unless `--no-check` is passed) - Tests provider mappings and tracks changes over time
   - Reports unmapped models
   - Compares with previous runs (like a lock file)
   - Shows changed/added/removed mappings
   - Writes to: `src/providers/canonical/data/canonical_mapping_report.json`

The script is located in this directory: `build_canonical_models.rs`

## Hand-curated records
Some records in `canonical_models.json` were written by hand on 2026-09-25 rather than generated.
They cover the GPT-6 and GPT-5.6 lineups, Claude Opus 5.5, Claude Fable 5.1, Gemini 3.5 Flash-Lite
and Gemini 3.6 to 3.8 Flash, Grok 4.5 to 4.7, Kimi K3, GLM-5.3 and DeepSeek V4.1 Flash.

Where OpenRouter's listing differs from the vendor's, these records use the vendor's list price
and published limits:
- `x-ai/grok-4.7` is $2/$6 per million tokens (OpenRouter lists $1.60/$4.80).
- `openai/gpt-5.6-sol` is $4/$20 (OpenRouter lists $2/$10).
- `deepseek/deepseek-v4.1-flash` is $0.15 input (OpenRouter lists $0.099).
- `z-ai/glm-5.3` has a context of 1,048,576 (OpenRouter lists 1,310,720).
- `moonshotai/kimi-k3` has a max completion of 1,048,576 (OpenRouter lists 943,718).

`build_canonical_models` regenerates the whole file from OpenRouter and does not know any of this,
so it will silently overwrite these values. After a regeneration, diff `canonical_models.json`
against the previous version and re-apply the values above before committing.

### November 2026 additions

The GPT 6.1/6.2, Fable 5.2, Sonnet/Haiku 5.5, Gemini 4 and Kimi K3.1 Pro records were
added from the [public OpenRouter model feed](https://openrouter.ai/api/v1/models) on
2026-11-18. Existing records were preserved. Keep these additional direct-provider
distinctions when regenerating:

- Gemini 4 Hydrogen, Argon and Carbon use a maximum foreground output of 65,536.
  OpenRouter's larger continuation budget does not describe one request through
  Biorouter's current Google/Vertex adapters. See the [Google model documentation](https://aistudio.google.com/docs/models/gemini-4-hydrogen).
- Direct Mistral Large 4 and its `mistral-large-4-0` alias map to
  `mistralai/mistral-large-4`: a conservative 1,000,000 context and no invented output
  maximum. OpenRouter's separate `mistralai/mistral-large-4.0` record retains its
  verified route limits. See the [direct model card](https://docs.mistral.ai/models/mistral-large-4).
- Direct Mercury 3 has a conservative 1,000,000 context and 131,072 maximum output.
  Its current launch price is $0.10/$0.35 per million tokens; the promotion has no
  published end date. See [Inception's model table](https://docs.inceptionlabs.ai/get-started/models.md).
- Direct Kimi K3.1 Pro Highspeed has its own record. The Pro canonical record keeps
  OpenRouter's route limits and rates, while direct Moonshot's exact registry,
  declarative metadata and pricing table supply a conservative 1,000,000 context
  and its own $2/$10 Pro or $4/$20 Highspeed rates. Canonical data does not set the
  direct provider's request budget. See the [Kimi guide](https://platform.kimi.ai/docs/guide/kimi-k3-1-pro-quickstart.md)
  and [direct prices](https://platform.kimi.ai/).

The pricing resolver handles provider-specific cache rates and long-context cards.
It also changes direct Gemini 4 Argon/Carbon from their launch card to the published
standard card on 2026-12-01; the canonical snapshot is not a historical price ledger.
Capability mapping for a hosted model does not establish its host's bill. New
unverified Azure, Vertex, Databricks and Tetrate prices remain unknown unless their
own rate metadata supplies them.

Baseten Kimi K3.1 Pro uses the exact hosted slug `moonshotai/Kimi-K3.1-Pro`, a
verified 1,048,576 context from its 2026-11-18 `/v1/models` response and Baseten's
own $2/$0.20 cached/$10 rate card in the explicit registry and pricing table. No
new output-cap field is introduced. Baseten remains outside canonical mapping: adding only this
one host record would make the generic recommendation filter hide its other
unmapped models. Its provider metadata supplies vision without changing Z.ai's
canonical GLM capabilities. See [Baseten's model table](https://docs.baseten.co/inference/model-apis/overview.md)
and [host pricing](https://www.baseten.co/pricing/).
