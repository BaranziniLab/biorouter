# z.ai (GLM) provider

> **What this is.** The integration reference for the z.ai provider (`zai`), which serves the GLM model family: how it is wired into the provider registry, every surface where a user can select it, and the checks that verify it works.
> **Status:** Current. Written 2026-06-19 against v1.85.3 alongside the commit that added the provider. The default model and model list were re-derived from `zai.rs` on 2026-09-25.
> **Audience:** maintainers working on LLM providers.

z.ai — the international platform of **Zhipu AI** — is integrated as a first-class, OpenAI-compatible provider serving the **GLM** models. Because every model-selection surface in BioRouter is registry-driven, the provider appears automatically once it is registered and configured; only display polish needed explicit wiring. This document records that wiring so a maintainer can re-derive it, and gives the commands to re-verify the integration.

Run the verification section after changing `crates/biorouter/src/providers/zai.rs`, the provider factory, or the model context-limit table — not on every release.

> **Note.** The `xiaomi_mimo` provider is wired in the same shape; see [the Xiaomi MiMo provider reference](xiaomi-mimo.md) for its equivalent of every section below. The two documents deliberately share a structure so the surfaces and checks can be compared line for line.

## How the provider is wired

- **Native provider module:** `crates/biorouter/src/providers/zai.rs`
  (`ZaiProvider`), registered in `crates/biorouter/src/providers/factory.rs`.
  Provider id `zai`, display name **z.ai**, default model `glm-5.3`.
- **Auth / endpoint:** Bearer `ZAI_API_KEY`. Default host is the
  OpenAI-compatible base `https://api.z.ai/api/paas/v4`; override with
  `ZAI_HOST`. (z.ai also exposes an Anthropic-compatible surface at
  `https://api.z.ai/api/anthropic` used by Claude Code — not used here; we
  integrate the OpenAI surface, matching the other ~16 OpenAI-compatible
  providers.)
- **Models:** `glm-5.3`, `glm-5.3-flash`, `glm-5.2`, `glm-5.1`, `glm-5`,
  `glm-5-turbo`, `glm-4.7`, `glm-4.6`, `glm-4.5`, `glm-4.5-air`. Context limits
  are registered per id in `MODEL_CONTEXT_WINDOWS` in
  `crates/biorouter/src/model.rs`, with the `glm-*` patterns in
  `MODEL_SPECIFIC_LIMITS` as the fallback for other ids.

> **Why.** The model list, default model, and default host above are copied from
> code and will drift as the catalog changes. The authoritative values are the
> `ZAI_KNOWN_MODELS`, `ZAI_DEFAULT_MODEL`, and `ZAI_API_HOST` constants in
> `crates/biorouter/src/providers/zai.rs` — re-derive from there rather than
> trusting this page.

> **Note.** GLM-5.3 and GLM-5.3 Flash reason on every request: z.ai rejects a
> request that disables thinking. Of the listed models, only `glm-5.3-flash`
> accepts image input.

## Where GLM appears

Every place a user can choose a provider or model. Unless noted, the surface is
backend-driven and required no z.ai-specific code.

| Surface | What appears | Where it is wired |
| --- | --- | --- |
| Provider config dashboard (Settings → Providers) | Appears under *Commercial Models*; backend-driven via `GET /config/providers` | Ordering in `ui/desktop/src/components/settings/providers/providerOrdering.ts` (`zai`) |
| Provider configuration modal | `ZAI_API_KEY` (secret) + optional `ZAI_HOST` fields, rendered from backend `config_keys` | Backend-driven |
| Onboarding | Listed under "Auto-detect from API key" | Auto-detect in `crates/biorouter/src/providers/auto_detect.rs`; text in `ui/desktop/src/components/onboarding/CommercialSetupCard.tsx` |
| Main model selector (bottom menu / `SwitchModelModal`) | Once configured, `glm-*` models appear in the picker | Backend-driven |
| Leader/Worker mode | GLM models selectable for both lead and worker | `LeadWorkerSettings.tsx`, backend-driven |
| Knowledge base ingestion/digestion | GLM models selectable for ingest | `IngestModelPicker.tsx`, backend-driven |
| CLI (`biorouter configure`) | Appears in the provider list under Commercial; usable via `biorouter run --provider zai --model glm-5.3` | `configure_provider_dialog()` reads the registry |
| TUI | Stores the selected provider/model string; no separate list | — |
| Daemon/server | `GET /config/providers`, `GET /config/providers/zai/models`, and `/config/detect-provider` all surface it | Registry-driven; no allowlist |

## Verifying the integration

### Endpoint and auth reachable

`GET {host}/models` with the key returns HTTP 200.

> **Note.** On 2026-06-19 the test key authenticated, but chat returned HTTP 429
> code 1113 "insufficient balance" — auth was fine, the account simply needed
> credit. A bad key returns 401, which is how that 429 was confirmed to be a
> billing state rather than an auth failure. This is a record of one account at
> one moment, not a property of the provider.

### Registered in factory

```bash
cargo test -p biorouter --lib providers::zai
```

Covers `test_registered_in_factory` and `test_metadata_structure`.

### Config keys correct

```bash
cargo test -p biorouter --lib test_openai_compatible_providers_config_keys
```

Asserts the first key is `ZAI_API_KEY`, required and secret.

### Live completion through the provider stack

Needs a funded key.

```bash
ZAI_API_KEY=<key> cargo test -p biorouter --test providers test_zai_provider -- --ignored --exact --nocapture
```

Exercises factory → `ZaiProvider::from_env` → live HTTP.

### Context window

The default `glm-5.3` reports 1,048,576 tokens for token accounting, as do
`glm-5.3-flash` and `glm-5.2`; `glm-4.6` and `glm-4.7` report 200,000. The
values come from `MODEL_CONTEXT_WINDOWS` in `crates/biorouter/src/model.rs`.

## Gotchas

- z.ai keys have the form `<id>.<secret>` — pass the whole string (the dot is
  part of the key) as `ZAI_API_KEY`.
- The default endpoint is the OpenAI-compatible `/api/paas/v4` base. Don't point
  `ZAI_HOST` at the Anthropic surface (`/api/anthropic`) — the wire format
  differs and this provider speaks OpenAI chat/completions.

## Related documentation

- [Xiaomi MiMo provider](xiaomi-mimo.md) — the sibling provider, wired in the same registry-driven shape, with the same section structure.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the user-facing reference for every supported provider, its credentials, and its default model.
- [Environment variables](../configuration/environment-variables.md) — where `ZAI_API_KEY` and `ZAI_HOST` sit among all other configuration variables.
- [Secret storage](../security/secret-storage.md) — how the API key is stored in the OS credential store rather than plaintext.
- [biorouter CLI command reference](../cli/command-reference.md) — the `configure` and `run --provider` commands referenced in the surfaces table.
