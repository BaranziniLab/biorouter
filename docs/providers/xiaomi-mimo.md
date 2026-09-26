# Xiaomi MiMo provider

> **What this is.** The integration reference for the Xiaomi MiMo provider (`xiaomi_mimo`): how it is wired into the provider registry, every surface where a user can select it, and the checks that verify it works.
> **Status:** Current. Written 2026-06-19 against v1.85.3 alongside the commit that added the provider. The default model and model list were re-derived from `xiaomi_mimo.rs` on 2026-09-25.
> **Audience:** maintainers working on LLM providers.

Xiaomi's **MiMo** LLM family is integrated as a first-class, OpenAI-compatible provider. Because every model-selection surface in BioRouter is registry-driven, the provider appears automatically once it is registered and configured — only display polish needed explicit wiring. This document records that wiring so a maintainer can re-derive it, and gives the commands to re-verify the integration.

Run the verification section after changing `crates/biorouter/src/providers/xiaomi_mimo.rs`, the provider factory, or the model context-limit table — not on every release.

> **Note.** The `zai` provider is wired in the same shape; see [the z.ai (GLM) provider reference](zai-glm.md) for its equivalent of every section below.

## How the provider is wired

- **Native provider module:** `crates/biorouter/src/providers/xiaomi_mimo.rs`
  (`XiaomiMimoProvider`), registered in `crates/biorouter/src/providers/factory.rs`.
  Provider id `xiaomi_mimo`, display name **Xiaomi MiMo**, default model
  `mimo-v2.6-flash`.
- **Auth / endpoint:** Bearer `XIAOMI_MIMO_API_KEY`. Default host is the
  live-verified Singapore Token-Plan endpoint
  `https://token-plan-sgp.xiaomimimo.com/v1`; override with `XIAOMI_MIMO_HOST`
  for another region/tier:
  - Pay-as-you-go (`sk-` keys): `https://api.xiaomimimo.com/v1`
  - Token Plan (`tp-` keys): `https://token-plan-{cn,sgp,ams}.xiaomimimo.com/v1`
- **Models:** `mimo-v2.6-flash`, `mimo-v2.6-pro` (1,048,576 tokens each; both
  take text, image, video and audio input). Context limits are registered per id
  in `MODEL_CONTEXT_WINDOWS` in `crates/biorouter/src/model.rs`. Xiaomi shuts
  down `mimo-v2.5` and `mimo-v2.5-pro` on 2026-10-21 with no replacement
  routing, and the `mimo-v2-pro` and `mimo-v2-omni` names expired on
  2026-06-30, so none of them is listed.

> **Why.** The model list, default model, and default host above are copied from
> code and will drift as the catalog changes. The authoritative values are the
> `XIAOMI_MIMO_KNOWN_MODELS`, `XIAOMI_MIMO_DEFAULT_MODEL`, and
> `XIAOMI_MIMO_API_HOST` constants in
> `crates/biorouter/src/providers/xiaomi_mimo.rs` — re-derive from there rather
> than trusting this page.

## Where MiMo appears

Every place a user can choose a provider or model. Unless noted, the surface is
backend-driven and required no MiMo-specific code.

| Surface | What appears | Where it is wired |
| --- | --- | --- |
| Provider config dashboard (Settings → Providers) | Appears under *Commercial Models*; backend-driven via `GET /config/providers` | Ordering in `ui/desktop/src/components/settings/providers/providerOrdering.ts` (`xiaomi_mimo`) |
| Provider configuration modal | `XIAOMI_MIMO_API_KEY` (secret) + optional `XIAOMI_MIMO_HOST` fields, rendered from backend `config_keys` | Labels in `ui/desktop/src/utils/configUtils.ts` |
| Onboarding | Listed under "Auto-detect from API key" and "View all commercial providers" | Auto-detect in `crates/biorouter/src/providers/auto_detect.rs`; text in `ui/desktop/src/components/onboarding/CommercialSetupCard.tsx` |
| Main model selector (bottom menu / `SwitchModelModal`) | Once configured, `mimo-*` models appear in the picker | Backend-driven |
| Leader/Worker mode | MiMo models selectable for both lead and worker | `LeadWorkerSettings.tsx`, backend-driven |
| Knowledge base ingestion/digestion | MiMo models selectable for ingest | `IngestModelPicker.tsx`, backend-driven |
| CLI (`biorouter configure`) | Appears in the provider list under Commercial; usable via `biorouter run --provider xiaomi_mimo --model mimo-v2.6-flash` | Registry-driven |
| TUI | Stores the selected provider/model string; no separate list | — |
| Daemon/server | `GET /config/providers`, `GET /config/providers/xiaomi_mimo/models`, and `/config/detect-provider` all surface it | Registry-driven; no allowlist |

## Verifying the integration

### Live endpoint reachable

`POST {host}/v1/chat/completions` with the key returns HTTP 200 and a
completion from the default model.

> **Note.** On 2026-06-19 this returned `BIOROUTER-OK` with `reasoning_tokens: 0`
> and thinking disabled. That is a record of one run against one key and region,
> not a permanent property — re-run it rather than citing it.

### Registered in factory

```bash
cargo test -p biorouter --lib providers::xiaomi_mimo
```

Covers `test_registered_in_factory` and `test_metadata_structure`.

### Config keys correct

```bash
cargo test -p biorouter --lib test_openai_compatible_providers_config_keys
```

Asserts the first key is `XIAOMI_MIMO_API_KEY`, required and secret. Passed on
2026-06-19.

### Live completion through the provider stack

```bash
XIAOMI_MIMO_API_KEY=<key> cargo test -p biorouter --test providers test_xiaomi_mimo_provider -- --ignored --exact --nocapture
```

Exercises factory → `XiaomiMimoProvider::from_env` → live HTTP.

### Context window

`mimo-v2.6-flash` and `mimo-v2.6-pro` report 1,048,576 tokens for token
accounting, from `MODEL_CONTEXT_WINDOWS` in `crates/biorouter/src/model.rs`.

## Gotchas

- MiMo enables **thinking** by default (adds reasoning tokens/latency). The
  OpenAI surface disables it via `chat_template_kwargs.enable_thinking=false`.
- A `tp-` key is bound to a single region — set `XIAOMI_MIMO_HOST` to the
  matching regional endpoint if not on Singapore.
- Token accounting: the API reports cached prompt tokens separately; counts are
  not directly comparable across regions/tiers.

## Related documentation

- [z.ai (GLM) provider](zai-glm.md) — the sibling provider, wired in the same registry-driven shape, with the same section structure.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the user-facing reference for every supported provider, its credentials, and its default model.
- [Environment variables](../configuration/environment-variables.md) — where `XIAOMI_MIMO_API_KEY` and `XIAOMI_MIMO_HOST` sit among all other configuration variables.
- [Secret storage](../security/secret-storage.md) — how the API key is stored in the OS credential store rather than plaintext.
- [Llama Server local model testing checklist](llama-server/model-catalog-qa-checklist.md) — the equivalent per-model verification pass for the bundled local provider.
