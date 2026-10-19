# Baseten provider

> **What this is.** The integration reference for the Baseten provider (`baseten`): the bundled declarative catalog, the live catalog feed that keeps its models and prices current between releases without an API key in the app, the scheduled job that publishes that feed, and the checks that verify both.
> **Status:** Current. Written 2026-10-19 alongside the commit that added the live catalog.
> **Audience:** maintainers working on LLM providers, and the repo admin who holds the feed's key.

Baseten's **Model APIs** serve open-weight models (GLM, Kimi, DeepSeek, Nemotron, Muse Spark, GPT OSS) behind an OpenAI-compatible endpoint. Biorouter ships them as a bundled declarative provider, and refreshes the model list and prices from a feed it reads with no key.

## How the provider is wired

- **Declarative config:** `crates/biorouter/src/providers/declarative/baseten.json`. Provider id `baseten`, display name **Baseten**, engine `openai`, `BASETEN_API_KEY`, base URL `https://inference.baseten.co/v1`, streaming on. Public tier by default (`providers/tier_tests.rs`).
- **Default model:** the first entry, `zai-org/GLM-5.3`.
- **Context windows:** one exact `MODEL_CONTEXT_WINDOWS` row per slug in `crates/biorouter/src/model.rs`. Slugs are case-sensitive and Baseten keeps vendor casing (`moonshotai/Kimi-K3`).
- **Prices:** `pricing::BASETEN_SYNC_PRICING`, including each model's cached-input rate. The async path takes those cache rates through `with_cache_rates_from`, because declarative JSON has no cache field.
- **Left out on purpose:** Beam (`reflection/Beam-1.0`) returns HTTP 400 on a multi-turn request whose assistant turns lack `reasoning_content`, which only `moonshot` replays; SID-1 (`sid/sid-1`) has a 32,768-token window, smaller than Biorouter's instructions plus tool schemas.

## The live catalog feed

Baseten's catalog (`GET https://inference.baseten.co/v1/models`) returns each model's context window, input types, supported features and prices, but it needs an API key. Biorouter must not need one, so the key lives in CI and the app reads a published copy.

```text
GitHub Actions (daily, key in the baseten-feed Environment)
  → GET inference.baseten.co/v1/models
  → landing/scripts/build-provider-catalog.mjs (no key)
  → https://biorouter.ucsf.edu/provider-catalogs/baseten.json
  → biorouterd fetches it at start and every 12 hours (no key)
```

### The publishing job

`baseten-catalog` in `.github/workflows/deploy-landing.yml`:

- Runs on the workflow's daily schedule, on pushes to `landing/`, and on demand. The workflow has no `pull_request` trigger.
- Uses the `baseten-feed` Environment, restricted to `main`, so no other branch can read the secret.
- The key is in the environment of the one step that calls Baseten, sent only as an `Authorization` header and never echoed. The step that builds the file does not have it.
- `build-provider-catalog.mjs` copies only the fields the app parses, and refuses a response with fewer than `MIN_MODELS` (8) models, so a broken or filtered response never replaces a good feed.
- If the job fails, the `deploy` job republishes the feed that is already live (a Pages deploy replaces the whole site). With no live feed either, it publishes none. The run still shows red.

### The app side

`crates/biorouter/src/providers/live_catalog.rs`, which refuses the whole document on any structural problem (unknown field, wrong provider or schema version, bad id, price out of range, duplicate):

| Source | When it is used |
| --- | --- |
| The feed | Fetched by `biorouterd` at start (`spawn_refresher`, in `biorouter-server`'s `commands/agent.rs`) and every 12 hours. A change rebuilds the provider registry, so the picker updates without a restart. |
| The last good copy | `<state dir>/provider-catalogs/baseten.json`, written on every successful fetch. Read at first use, which is also how a CLI process with no daemon gets it. |
| The bundled catalog | When neither of the above exists, or when the feed advertises no model Biorouter can drive. |

What the feed changes:

- **Prices** for `baseten`, including cached input, ahead of every bundled table on both pricing paths.
- **The model list:** the feed's models that offer tool calling, have at least a 65,536-token window and need no reasoning replay. Bundled models keep their order (so the default stays first) and are dropped if the feed no longer lists them; new models follow, by id.
- **Context windows only fill gaps.** `MODEL_CONTEXT_WINDOWS` is keyed by name alone and a slug can be shared (`openai/gpt-oss-120b` is also Groq's, and Baseten reports 128,072 for it), so a static row always wins. A feed window is used only for a slug the static table does not have.

### Setting up the key (repo admin, once)

1. In Baseten, create a **Team** key with the permission **Call Model APIs**, scoped to one cheap model (GPT OSS 120B). Listing models runs no inference, and the list is not filtered by the key's scope (measured 2026-10-19: a one-model key sees all 18). Turn on workspace **budget enforcement** so a leaked key can spend at most the cap.
2. In GitHub, **Settings → Environments → New environment** `baseten-feed`. Under **Deployment branches and tags**, choose **Selected branches and tags** and add `main`. Leave **Required reviewers** off, or every scheduled run waits for approval.
3. Under that environment's **Environment secrets**, add `BASETEN_API_KEY`. Do not add it as a repository secret, which every branch can read.
4. Run **Actions → Deploy landing site → Run workflow** on `main`, then check `https://biorouter.ucsf.edu/provider-catalogs/baseten.json`.

To rotate the key, revoke it in Baseten and replace the environment secret. Until the new one is in place, the feed stays at its last published version.

## Verifying the integration

```bash
cargo test -p biorouter --lib -- live_catalog declarative_providers pricing
cargo test -p biorouter --test context_windows
node --test landing/scripts/build-provider-catalog.test.mjs
```

`live_catalog::tests::the_published_feed_format_parses` reads `landing/scripts/fixtures/baseten-catalog.json`, which the script built from Baseten's real response. It is the only test where the script's output and the Rust parser meet; regenerate the fixture with the script, never by hand:

```bash
curl -sS -H "Authorization: Bearer $BASETEN_API_KEY" https://inference.baseten.co/v1/models -o /tmp/raw.json
node landing/scripts/build-provider-catalog.mjs /tmp/raw.json landing/scripts/fixtures/baseten-catalog.json
```

## Gotchas

- **Testing the desktop app against a dev build:** on macOS the dev window attaches to an already-running shared daemon, which may be the installed app's and predate your change. Launch with `BIOROUTER_SHARED_DAEMON=0 just run-dev`.
- **Unit tests never read the cached feed.** `live_catalog`'s state is seeded from disk only outside `cfg(test)`, so a developer's cached copy cannot change what the pricing and catalog tests assert. Integration tests compile the library without `cfg(test)` and will see it.

## Related documentation

- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the user-facing reference for every supported provider, its credentials, and its default model.
- [Xiaomi MiMo provider](xiaomi-mimo.md) — another provider reference in this folder.
- [Secret storage](../security/secret-storage.md) — how a user's `BASETEN_API_KEY` is stored in the OS credential store.
