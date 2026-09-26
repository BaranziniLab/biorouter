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
