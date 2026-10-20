# Baseten provider

> **What this is.** The integration reference for the Baseten provider (`baseten`): the bundled declarative catalog, the live catalog feed that keeps its models and prices current between releases without an API key in the app, the scheduled job that publishes that feed, where Baseten appears in the app, and the checks that verify both.
> **Status:** Current. Written 2026-10-19 alongside the commit that added the live catalog. Revised 2026-10-20 after review of PR #417: the daemon fetches the feed only while Baseten is configured, the newest data wins, and the feed builder holds to the parser's bounds.
> **Audience:** maintainers working on LLM providers, and the repo admin who holds the feed's key.

Baseten's **Model APIs** serve open-weight models (GLM, Kimi, DeepSeek, Nemotron, Muse Spark, GPT OSS) behind an OpenAI-compatible endpoint. Biorouter ships them as a bundled declarative provider, and refreshes the model list and prices from a feed it reads with no key.

## How the provider is wired

- **Declarative config:** `crates/biorouter/src/providers/declarative/baseten.json`. Provider id `baseten`, display name **Baseten**, engine `openai`, `BASETEN_API_KEY`, base URL `https://inference.baseten.co/v1`, streaming on. Public tier by default (`providers/tier_tests.rs`).
- **Default model:** the first entry, `zai-org/GLM-5.3`.
- **Context windows:** one exact `MODEL_CONTEXT_WINDOWS` row per slug in `crates/biorouter/src/model.rs`. Slugs are case-sensitive and Baseten keeps vendor casing (`moonshotai/Kimi-K3`).
- **Prices:** `pricing::BASETEN_SYNC_PRICING`, including each model's cached-input rate. The async path takes those cache rates through `with_cache_rates_from`, because declarative JSON has no cache field.
- **Left out on purpose:** Beam (`reflection/Beam-1.0`) returns HTTP 400 on a multi-turn request whose assistant turns lack `reasoning_content`, which only `moonshot` replays; SID-1 (`sid/sid-1`) has a 32,768-token window, smaller than Biorouter's instructions plus tool schemas.

## Where Baseten appears

No surface has Baseten-specific code. Each one reads the provider registry, so Baseten appears wherever a declarative provider does.

| Surface | What appears |
| --- | --- |
| Provider catalog (Settings, and the first run screen) | **Baseten** on the *Public* tab, served by `GET /config/providers`. |
| Model picker (`SwitchModelModal`) | Once `BASETEN_API_KEY` is saved, the merged model list described in [What the feed changes](#what-the-feed-changes). |
| `biorouter configure` | **Baseten** in the one provider list. |
| Daemon | `GET /config/providers/baseten/models` returns the same merged list as the model picker. |

## The live catalog feed

Baseten's catalog (`GET https://inference.baseten.co/v1/models`) returns each model's context window, input types, supported features and prices, but it needs an API key. Biorouter must not need one, so the key lives in CI and the app reads a published copy.

```text
GitHub Actions (daily, key in the baseten-feed Environment)
  → GET inference.baseten.co/v1/models
  → landing/scripts/build-provider-catalog.mjs (no key)
  → https://biorouter.ucsf.edu/provider-catalogs/baseten.json
  → biorouterd fetches it while Baseten is configured (no key)
```

### The publishing job

`baseten-catalog` in `.github/workflows/deploy-landing.yml`:

- Runs once a day (09:17 UTC), on every push to `main` that touches `landing/`, `docs/website/` or `.github/workflows/deploy-landing.yml`, and on demand. The workflow has no `pull_request` trigger.
- Uses the `baseten-feed` Environment, restricted to `main`, so no other branch can read the secret. The environment exists: it was created on 2026-10-20 with deployments limited to `main` and the secret `BASETEN_API_KEY`.
- Runs `node --test landing/scripts/build-provider-catalog.test.mjs` first, before the key is used, so a builder that would publish a feed the app refuses fails the job instead.
- The key is in the environment of the one step that calls Baseten, sent only as an `Authorization` header and never echoed. The step that builds the file does not have it.
- `build-provider-catalog.mjs` copies only the fields the app reads, keeps prices as the strings Baseten sent, and holds every entry to the parser's bounds (below). A model whose window is under 4,096 tokens (an embedder or a reranker) is dropped with a `::warning::` line. Anything else out of bounds fails the build, as does a response with fewer than `MIN_MODELS` (8) models left after that drop, so a broken or filtered response never replaces a good feed. A failed build writes no file.
- If the job fails, the `deploy` job republishes the feed that is already live (a Pages deploy replaces the whole site). With no live feed either, it publishes none. The run still shows red.

The bounds are constants in `crates/biorouter/src/providers/live_catalog.rs`, mirrored under the same names at the top of the builder. `live_catalog::tests` reads the builder's source and fails when the two disagree.

| Field | Bound | The builder, on a value outside it |
| --- | --- | --- |
| `context_length` | 4,096 (`MIN_CONTEXT`) to 16,777,216 (`MAX_CONTEXT`) tokens | Below: drops the model and warns. Above: fails. |
| Each price, per token | A finite number from 0 to 0.001 (`MAX_TOKEN_COST`), written as a plain decimal or with a negative exponent (`1.4e-6`) | Fails. |
| `id` | 1 to 128 bytes (`MAX_ID_BYTES`) of `A-Z a-z 0-9 . _ / -`, no duplicates | Fails. |
| `models` | 1 to 512 (`MAX_MODELS`) | Fails. |
| The whole feed | At most 256 KiB (`MAX_BYTES`) | Fails. |
| `schema_version`, `provider` | `1`, `"baseten"` | Writes these itself. |
| `fetched_at` | An RFC 3339 timestamp | Writes the time of the run. |

### The app side

`crates/biorouter/src/providers/live_catalog.rs` refuses the whole document when any bound above fails, so one bad entry would cost every model in the feed. That is why the builder checks the same bounds before it publishes. Fields the parser does not know are ignored, so the builder can add one without breaking released apps; a change an older parser would misread must bump `schema_version`. The parser no longer reads `name` or `max_completion_tokens`, and the builder no longer writes them.

| Source | When it is used |
| --- | --- |
| The feed | Fetched by `biorouterd` only while Baseten is configured, meaning its `BASETEN_API_KEY` is saved. The daemon checks once an hour and fetches at most every 12 hours (`spawn_refresher`, started from `biorouter-server`'s `commands/agent.rs`). With Baseten not configured it makes no request at all. A change rebuilds the provider registry, so the picker updates without a restart. |
| The last good copy | `<state dir>/provider-catalogs/baseten.json`, written when a fetch brings a change. Read at first use, which is also how a CLI process with no daemon gets it. |
| The bundled catalog | When neither of the above exists, when both are older than the bundled catalog, or when the feed advertises no model Biorouter can drive. |

**Newest data wins.** The bundled catalog was read from Baseten on 2026-10-18 (`BUNDLED_CATALOG_AS_OF` in `live_catalog.rs`). A feed or cached copy whose `fetched_at` is older than that date is ignored, so a release always outranks an older feed or cache. A feed that stopped updating (a revoked key, or a response the builder refuses) cannot hide a model a later release ships, or bill its old prices over the new ones. Move the constant in the same change as `baseten.json` or `BASETEN_SYNC_PRICING`. A feed whose `fetched_at` is not RFC 3339 is refused.

#### What the feed changes

- **Prices** for `baseten`, including cached input, ahead of every bundled table on both pricing paths. A zero input or output price counts as unknown, not free, because a zero rate would switch off the dollar budget for the model. A bundled model then keeps its bundled price, and a model only the feed lists is shown without a price.
- **The model list:** the feed's models that offer tool calling, have at least a 65,536-token window and need no reasoning replay. Bundled models keep their order (so the default stays first) and are dropped if the feed no longer lists them; new models follow, by id. `GET /config/providers/baseten/models` returns this same list.
- **Context windows, for Baseten's own models only, and only to fill gaps.** `MODEL_CONTEXT_WINDOWS` is keyed by name alone and a slug can be shared (`openai/gpt-oss-120b` is also Groq's, and Baseten reports 128,072 for it), so a static row always wins. A feed window is used only for a Baseten model whose slug the static table does not have, and never reaches another provider's model of the same name.

### Setting up the key (repo admin, once)

Done on 2026-10-20. The steps stay here for a rotation or a rebuild of the environment.

1. In Baseten, create a **Team** key with the permission **Call Model APIs**, scoped to one cheap model (GPT OSS 120B). Listing models runs no inference, and the list is not filtered by the key's scope (measured 2026-10-19: a one-model key sees all 18). Turn on workspace **budget enforcement** so a leaked key can spend at most the cap.
2. In GitHub, **Settings → Environments → New environment** `baseten-feed`. Under **Deployment branches and tags**, choose **Selected branches and tags** and add `main`. Leave **Required reviewers** off, or every scheduled run waits for approval.
3. Under that environment's **Environment secrets**, add `BASETEN_API_KEY`. Do not add it as a repository secret, which every branch can read.
4. Run **Actions → Deploy landing site → Run workflow** on `main`, then check `https://biorouter.ucsf.edu/provider-catalogs/baseten.json`.

To rotate the key, revoke it in Baseten and replace the environment secret. Until the new one is in place, the feed stays at its last published version.

## Verifying the integration

```bash
cargo test -p biorouter --lib -- live_catalog declarative_providers pricing
cargo test -p biorouter --test context_windows
cargo test -p biorouter --test baseten_live_catalog
node --test landing/scripts/build-provider-catalog.test.mjs
```

`baseten_live_catalog` loads the committed feed fixture from disk in its own process, the way a user's machine does, and checks the paths the unit tests cannot: pricing, the provider registry, and the window a Baseten provider is built with.

Two fixtures hold the script and the parser to one format. `landing/scripts/fixtures/baseten-models.raw.json` is Baseten's real `/v1/models` response, read on 2026-10-20, and `baseten-catalog.json` beside it is what the script builds from it. The script's suite rebuilds the second from the first and fails if they differ, and `live_catalog::tests::the_published_feed_format_parses` parses the second. Regenerate both with the script, never by hand:

```bash
curl -sS -H "Authorization: Bearer $BASETEN_API_KEY" https://inference.baseten.co/v1/models \
  | node -e 'let s="";process.stdin.on("data",(d)=>(s+=d)).on("end",()=>process.stdout.write(JSON.stringify(JSON.parse(s),null,2)+"\n"))' \
  > landing/scripts/fixtures/baseten-models.raw.json
node landing/scripts/build-provider-catalog.mjs landing/scripts/fixtures/baseten-models.raw.json landing/scripts/fixtures/baseten-catalog.json
```

If Baseten's list changed, update the model counts `the_published_feed_format_parses` asserts.

## Known limits

- **A new Baseten model reaches the picker without review.** Once the feed lists it, every configured app offers it. Only a model without tool calling, one that needs reasoning replay (like Beam) or one with a window under 65,536 tokens (like SID-1) is kept out.
- **The key behind the feed can run only `openai/gpt-oss-120b`.** It lists every model but can call no other, so the job cannot try a new model before the picker offers it.
- **A feed-only model that later leaves the feed loses its price in past usage totals.** No bundled table holds a price for it, so once the feed drops it, nothing does.

## Gotchas

- **Testing the desktop app against a dev build:** on macOS the dev window attaches to an already-running shared daemon, which may be the installed app's and predate your change. Launch with `BIOROUTER_SHARED_DAEMON=0 just run-dev`.
- **Unit tests never read the cached feed.** `live_catalog`'s state is seeded from disk only outside `cfg(test)`, so a developer's cached copy cannot change what the pricing and catalog tests assert. Integration tests compile the library without `cfg(test)` and will see it.
- **A dev daemon fetches nothing until Baseten is configured.** Save a `BASETEN_API_KEY` to see the feed's list in the picker; without one, the bundled catalog or the last good copy on disk is what you get.

## Related documentation

- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the user-facing reference for every supported provider, its credentials, and its default model.
- [Xiaomi MiMo provider](xiaomi-mimo.md) — another provider reference in this folder.
- [Secret storage](../security/secret-storage.md) — how a user's `BASETEN_API_KEY` is stored in the OS credential store.
