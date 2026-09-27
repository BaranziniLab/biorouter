# Choosing a model provider

> **What this is.** A reference of Biorouter's supported LLM providers: the credentials each one needs, its default model, a representative model list, and how to switch provider or override the choice per session.
> **Status:** Current. The provider list, default models and model lists below were checked against `crates/biorouter/src/providers/` on 2026-09-25. One section is known to be out of date: the panel ordering table under "Provider configuration panel" no longer matches the shipping app, whose live ordering and grouping is [`ui/desktop/src/components/settings/providers/providerOrdering.ts`](../../ui/desktop/src/components/settings/providers/providerOrdering.ts). The switching, orchestration, and custom-provider sections at the end remain accurate.
> **Audience:** end users

Biorouter connects to a wide range of LLM providers — commercial cloud APIs, institution-hosted services, and local models. You select and configure providers through the Provider Settings panel in the app (Settings > Models > Providers).

**UCSF users:** For institution-managed access, start with **Versa API Azure** (`versa_azure`, the UCSF ChatGPT models) or **Versa API Bedrock** (`versa_bedrock`, the UCSF-hosted Anthropic models). The generic **Azure OpenAI** and **Amazon Bedrock** providers are the commercial ones, not the UCSF institutional ones. For fully local inference, use **Llama Server** or **Ollama**.

> **Note.** The model lists on this page are hand-maintained snapshots, last checked against the code on 2026-09-25, and they drift. The authoritative values are the `*_DEFAULT_MODEL` and `*_KNOWN_MODELS` constants in `crates/biorouter/src/providers/`, and the bundled JSON files in `crates/biorouter/src/providers/declarative/` for DeepSeek, Groq, Inception, Mistral AI and Moonshot AI. Treat the live model picker, which fetches from the provider, as authoritative.

The **default model** is the one Biorouter uses when you configure a provider without naming a model. The desktop model picker preselects the first model in a provider's list when you switch to that provider, and that is not always the default named here.

## Provider configuration panel

Providers are managed in Settings > Models. Each provider card shows:

- Provider name and status (configured / not configured)
- A "Configure" button to enter API keys or credentials
- A "Launch" button to switch to that provider and choose a model

Cards are grouped into three sections, in this order, with providers sorted by priority within each group and alphabetically thereafter:

| Order | Group | Providers, in order |
|---|---|---|
| 1 | Local Models | `llamacpp` (Llama Server), `ollama` |
| 2 | Institutional Models | `versa_azure`, `versa_bedrock` |
| 3 | Commercial Models | `azure_openai`, `aws_bedrock`, `anthropic`, `openai`, `google`, `zai`, `xiaomi_mimo`, then all others alphabetically |

The panel hides nothing: every provider Biorouter has is shown in one of the three sections.

You can also add fully custom providers (e.g. any OpenAI-compatible endpoint) via the "Add Custom Provider" card.

## Supported providers

The providers are grouped below: local models, UCSF institutional models, commercial providers, and the coding-agent providers that drive a CLI you have installed.

### Local models

#### Llama Server

**No API key required.** Provider id `llamacpp`.

The desktop app bundles a pinned llama.cpp `llama-server` binary and runs it for you. It downloads a model's weights once, on first use; after that, nothing leaves your machine.

Default model: `gemma4-12b` on machines with 64 GiB or more of GPU-addressable memory, `gemma4` (Gemma 4 E4B) below that.

Available models include:

- `gemma4`, `gemma4-e2b`, `gemma4-12b`, `gemma4-26b`, `gemma4-31b`
- `qwen3.6`, `qwen3.6-27b`
- Any other model, given as a raw Hugging Face `owner/repo:QUANT` spec

Optional configuration includes `LLAMACPP_CONTEXT_SIZE`, `LLAMACPP_ENABLE_THINKING` and `LLAMACPP_EXTERNAL_HOST`.

#### Ollama (local)

**No API key required** — runs fully on your machine.

Use Ollama for completely local, private inference. No data leaves your device.

Default model: `qwen3`

Available models include:

- qwen3, qwen3-coder variants
- Any model available in the Ollama library

To use: install [Ollama](https://ollama.com), pull a model (`ollama pull qwen3`), then configure Biorouter to use the Ollama provider. The endpoint defaults to `http://localhost:11434`.

### Institutional models (UCSF)

#### Versa API Azure

**Environment variable:** `VERSA_AZURE_API_KEY`

Provider id `versa_azure`. The UCSF ChatGPT models, served from UCSF's Azure tenant. Only the API key is asked for; the endpoint and the deployment for each model are preconfigured.

Default model: `gpt-5.5-2026-04-24`

Available models:

- `gpt-5.5-2026-04-24`
- `gpt-5.4-mini-2026-03-17`, `gpt-5.4-nano-2026-03-17`
- `gpt-5.2-2025-12-11`
- `gpt-5-2025-08-07`, `gpt-5-mini-2025-08-07`, `gpt-5-nano-2025-08-07`
- `gpt-4o-2024-11-20`

Each model maps to a UCSF deployment, so this provider does not accept a model outside the list. UCSF has not deployed GPT-6 or GPT-5.6 yet (checked 2026-09-25). A chat already bound to `o4-mini-2025-04-16`, `gpt-4.1-2025-04-14` or `gpt-4.1-mini-2025-04-14` keeps working until Azure retires the model (2026-11-19 for `o4-mini`, 2027-04-14 for the two GPT-4.1 models), but those models are no longer offered.

#### Versa API Bedrock

**Environment variables:** `VERSA_BEDROCK_ACCESS_KEY_ID`, `VERSA_BEDROCK_SECRET_ACCESS_KEY`

Provider id `versa_bedrock`. The UCSF-hosted Anthropic models, served through Amazon Bedrock. Only the access key pair is asked for; the UCSF endpoint and the region are preconfigured.

Default model: `us.anthropic.claude-opus-4-8`

Available models include:

- `us.anthropic.claude-opus-4-8`
- `us.anthropic.claude-opus-4-6-v1`, `us.anthropic.claude-sonnet-4-6`
- `us.anthropic.claude-opus-4-5-20251101-v1:0`
- `us.anthropic.claude-haiku-4-5-20251001-v1:0`
- `us.anthropic.claude-opus-5-5`, `us.anthropic.claude-opus-5`, `us.anthropic.claude-sonnet-5`

The last three are AWS's own ids, but none of them has completed a request through the UCSF gateway yet, so whether UCSF's account can use them is unconfirmed. They are listed last so the picker never preselects one. Claude Fable models are not offered, because Bedrock serves them only to accounts whose data retention mode is `aws_review`.

### Commercial providers

#### Anthropic

**Environment variable:** `ANTHROPIC_API_KEY`

Direct API access to Anthropic's Claude models.

Default model: `claude-opus-4-8`. Fast model, used for background calls such as chat titles and summaries: `claude-haiku-4-5`.

Available models include:

- `claude-opus-5-5`, `claude-fable-5-1`
- `claude-opus-5`, `claude-sonnet-5`
- `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`, `claude-sonnet-4-6`
- `claude-haiku-4-5`

Claude Fable 5.1 needs 30-day data retention: an organization on zero data retention gets an error for every request.

#### OpenAI

**Environment variable:** `OPENAI_API_KEY`

Direct API access to OpenAI models.

Default model: `gpt-6-sol`. Fast model, used for background calls such as chat titles and summaries: `gpt-6-luna`.

Available models include:

- GPT-6: `gpt-6-sol`, `gpt-6-astra`, `gpt-6-luna`
- GPT-5.6: `gpt-5.6-sol` (also reachable as `gpt-5.6`), `gpt-5.6-terra`, `gpt-5.6-luna`
- `gpt-5.5`, `gpt-5.5-pro`
- `gpt-5.4`, `gpt-5.4-pro`, `gpt-5.4-mini`, `gpt-5.4-nano`
- `gpt-5.3-codex`
- `gpt-5.2`, `gpt-5.1`
- `gpt-4.1`, `gpt-4.1-mini`, `gpt-4o-mini`

There is no `gpt-6-terra`; Terra exists only as `gpt-5.6-terra`. `gpt-5`, `gpt-5-mini`, `gpt-5-nano` and `o3` are no longer listed, because OpenAI shuts them down on 2026-12-11. OpenAI names `gpt-5.6-sol` as the replacement for `gpt-5` and `o3`, `gpt-5.6-terra` for `gpt-5-mini`, and `gpt-5.6-luna` for `gpt-5-nano`.

GPT-6, GPT-5.6, GPT-5.5, GPT-5.4 and `gpt-5.3-codex` go through OpenAI's Responses API (`/v1/responses`); the older models use Chat Completions. Biorouter picks the route from the model name. `OPENAI_BASE_PATH` changes only the Chat Completions path, so a proxy set through `OPENAI_HOST` must also serve `/v1/responses` for those newer models.

Optional configuration: `OPENAI_HOST`, `OPENAI_BASE_PATH`, `OPENAI_ORGANIZATION`, `OPENAI_PROJECT`, `OPENAI_CUSTOM_HEADERS`, `OPENAI_TIMEOUT`

#### Google Gemini

**Environment variable:** `GOOGLE_API_KEY`

Direct API access to Google's Gemini models.

Default model: `gemini-3.1-pro-preview`. Fast model, used for background calls such as chat titles and summaries: `gemini-3.5-flash-lite`.

Available models include:

- `gemini-3.8-flash`, `gemini-3.7-flash`, `gemini-3.6-flash`, `gemini-3.5-flash`
- `gemini-3.5-flash-lite`, `gemini-3.1-flash-lite`
- `gemini-3.1-pro-preview`, `gemini-3-flash-preview`
- `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.5-flash-lite`

Since 2026-09-18 Google serves the Gemini 2.5 models only to API keys that used them before.

#### GCP Vertex AI

**Environment variables:** `GCP_PROJECT_ID`, `GCP_LOCATION` (default `us-central1`). Authentication uses a service account or application default credentials.

Runs Google and Anthropic models through Google Cloud's Vertex AI infrastructure.

Default model: `gemini-3.8-flash`

Available models include:

- `gemini-3.8-flash`, `gemini-3.7-flash`, `gemini-3.6-flash`, `gemini-3.5-flash`, `gemini-3.5-flash-lite`
- `gemini-3.1-pro-preview`, `gemini-3.1-flash-lite`, `gemini-3-flash-preview`
- `claude-opus-5-5`, `claude-fable-5-1`, `claude-opus-5`, `claude-sonnet-5`
- `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`, `claude-sonnet-4-6`

A Claude model must first be enabled in your project's Model Garden.

`GCP_LOCATION` may be a single region such as `us-central1`, the `global` endpoint, or the `us` or `eu` multi-region. Vertex serves Gemini 3.x, Claude 5.x, Claude Opus 4.7 and Claude Opus 4.8 at `global` and the `us` and `eu` multi-regions rather than in single regions like `us-central1`. So when `GCP_LOCATION` names a single region, Biorouter sends those models to the `global` endpoint automatically. When it is `us` or `eu`, Biorouter keeps that multi-region, so requests stay in that geography; the two preview models always go to `global`. Older Claude models and Gemini 2.x use the configured region. The `us` and `eu` multi-regions do not serve them, so with either one they go to a single region in that geography instead (`europe-west1` for `eu`; `us-east5` for Claude or `us-central1` for Gemini with `us`), and a failed request is never retried outside that geography.

#### Azure OpenAI

**Environment variables:** `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_DEPLOYMENT_NAME`, and optionally `AZURE_OPENAI_API_KEY` (leave it empty to use the Azure credential chain) and `AZURE_OPENAI_API_VERSION`

The generic Azure OpenAI provider, for your own Azure resource. It is a commercial provider; UCSF users should use [Versa API Azure](#versa-api-azure) instead.

Default model: `gpt-6-sol-2026-09-22`

Available models include:

- GPT-6: `gpt-6-sol-2026-09-22`, `gpt-6-astra-2026-09-03`, `gpt-6-luna-2026-09-22`
- GPT-5.6: `gpt-5.6-sol-2026-07-09`, `gpt-5.6-terra-2026-07-09`, `gpt-5.6-luna-2026-07-09`
- `gpt-5.5-2026-04-24`
- `gpt-5.4-2026-03-05`, `gpt-5.4-mini-2026-03-17`, `gpt-5.4-nano-2026-03-17`
- `gpt-5.2-2025-12-11`, `gpt-5.1-2025-11-13`, `gpt-5-2025-08-07`
- `gpt-4o-2024-11-20`

Each id is the Azure model name plus its model version. The deployment a request goes to is `AZURE_OPENAI_DEPLOYMENT_NAME`. Azure notes that some quota tiers need a quota request for the GPT-6 family.

GPT-6, GPT-5.6, GPT-5.5 and GPT-5.4 models use `POST {endpoint}/openai/v1/responses`, with the deployment name as the `model` and no `api-version`. `AZURE_OPENAI_API_VERSION` (default `2025-01-01-preview`) applies only to the models that use Chat Completions: GPT-5.2, GPT-5.1, GPT-5 and GPT-4o.

Azure has deprecated `o1`, `o3-mini`, `o3` and `o4-mini` (retiring 2026-11-19) and GPT-4.1 and GPT-4.1 mini (retiring 2027-04-14), so they are no longer listed. On 2026-11-19 Azure upgrades Standard and Global Standard o-series deployments to GPT-5.6. From then on a chat configured as `o4-mini` keeps working, but one configured as `o1`, `o3` or `o3-mini` has to be switched to a GPT-5.6 model id.

#### Amazon Bedrock

**Environment variables:** `AWS_PROFILE`, `AWS_REGION` (or standard AWS credential chain)

The generic Amazon Bedrock provider, for your own AWS account. It is a commercial provider; UCSF users should use [Versa API Bedrock](#versa-api-bedrock) instead. Supports AWS SSO profiles: run `aws sso login --profile <profile-name>` before using.

Default model: `us.anthropic.claude-sonnet-4-6`

Available models include:

- `us.anthropic.claude-opus-5-5`, `us.anthropic.claude-fable-5-1`
- `us.anthropic.claude-opus-5`, `us.anthropic.claude-sonnet-5`
- `us.anthropic.claude-opus-4-8`, `us.anthropic.claude-sonnet-4-6`, `us.anthropic.claude-opus-4-6-v1`
- `us.anthropic.claude-opus-4-5-20251101-v1:0`, `us.anthropic.claude-sonnet-4-5-20250929-v1:0`, `us.anthropic.claude-haiku-4-5-20251001-v1:0`

Claude Fable 5.1 on Bedrock needs the AWS account's Bedrock data retention mode set to `aws_review`, or every request fails.

#### Databricks

**Environment variables:** `DATABRICKS_HOST`, `DATABRICKS_TOKEN`

Access models through Databricks. Supports OAuth.

Default model: `databricks-claude-sonnet-4-6`. Fast model, used for background calls such as chat titles and summaries: `databricks-gemini-3-5-flash`, when your workspace serves it.

Available models include:

- Claude: `databricks-claude-sonnet-5`, `databricks-claude-opus-5`, `databricks-claude-opus-4-8`, `databricks-claude-sonnet-4-6`
- OpenAI: `databricks-gpt-6-astra`, `databricks-gpt-6-sol`, `databricks-gpt-6-luna`, `databricks-gpt-5-6-sol`, `databricks-gpt-5-5`
- Google: `databricks-gemini-3-8-flash`, `databricks-gemini-3-5-flash`, `databricks-gemini-3-1-pro`
- Meta: `databricks-meta-llama-3-3-70b-instruct`, `databricks-llama-4-maverick`

Claude Opus 5.5 and Claude Fable 5.1 are not listed: Databricks documents them only on its Anthropic Messages API, which this provider does not call.

#### Snowflake Cortex

**Environment variables:** `SNOWFLAKE_HOST`, `SNOWFLAKE_TOKEN`

Access Claude and other models through Snowflake's Cortex integration.

Default model: `claude-sonnet-4-6`

Available models include:

- `claude-sonnet-4-6`, `claude-sonnet-5`, `claude-opus-5`
- `claude-opus-5-5` (a public preview on Snowflake)
- `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`
- `claude-opus-4-5`, `claude-sonnet-4-5`, `claude-haiku-4-5`

#### OpenRouter

**Environment variable:** `OPENROUTER_API_KEY`

A proxy service that provides access to many providers through a single API.

Default model: `anthropic/claude-sonnet-5`. Fast model, used for background calls such as chat titles and summaries: `google/gemini-3.5-flash`.

Available models include:

- `anthropic/claude-opus-5.5`, `anthropic/claude-fable-5.1`, `anthropic/claude-sonnet-5`
- `openai/gpt-6-astra`, `openai/gpt-6-sol`, `openai/gpt-6-luna`, `openai/gpt-5.6-terra`
- `google/gemini-3.1-pro-preview`, `google/gemini-3.8-flash`
- `x-ai/grok-4.7`, `deepseek/deepseek-v4.1-flash`, `moonshotai/kimi-k3`, `z-ai/glm-5.3`, `qwen/qwen3-coder-next`, `minimax/minimax-m3`

#### Tetrate Agent Router Service

**Environment variable:** `TETRATE_API_KEY` (optional `TETRATE_HOST`)

Provider id `tetrate`. A routing service across upstream models, and the provider [biorouter in 5 minutes](quickstart.md) uses. Its automatic setup signs you in through the browser.

Default model: `claude-haiku-4-5`

Available models include:

- `claude-opus-5-5`, `claude-fable-5-1`, `claude-sonnet-5`, `claude-sonnet-4-6`, `claude-haiku-4-5`
- `gemini-2.5-pro`, `gemini-2.5-flash`
- `gpt-4.1`

#### GitHub Copilot

**Authentication:** Device code OAuth flow

Access GPT, Claude, Gemini, and Grok models through GitHub Copilot infrastructure.

Default model: `gpt-5.3-codex`

Available models include:

- `gpt-5.3-codex`, `gpt-6-astra`, `gpt-6-sol`, `gpt-6-luna`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`
- `claude-opus-5.5`, `claude-fable-5.1`, `claude-opus-4.8`, `claude-sonnet-5`, `claude-haiku-4.5`
- `gemini-3.8-flash`, `grok-4.7`

Which of these your account can use depends on your Copilot plan, and the picker shows what GitHub reports for your account.

GitHub Copilot does not use an API key. During configuration an auth code is copied to your clipboard and a browser window opens for you to paste it. See the configuration walkthrough in [biorouter in 5 minutes](quickstart.md#cli).

#### X.AI (Grok)

**Environment variable:** `XAI_API_KEY`

Access Grok models from xAI.

Default model: `grok-4.7`

Available models include:

- `grok-4.7`, `grok-4.6`, `grok-4.5`
- `grok-4.3`, `grok-4.20-0309-reasoning`, `grok-4.20-0309-non-reasoning`
- `grok-build-0.1`, for fast agentic coding

#### z.ai (GLM)

**Environment variable:** `ZAI_API_KEY` (optional `ZAI_HOST`)

Provider id `zai`. The GLM model family on z.ai's OpenAI-compatible API.

Default model: `glm-5.3`

Available models include:

- `glm-5.3`, `glm-5.3-flash`, `glm-5.3-flashx`, `glm-5.2`, `glm-5.1`, `glm-5`, `glm-5-turbo`
- `glm-4.7`, `glm-4.6`, `glm-4.5`, `glm-4.5-air`

GLM-5.3, GLM-5.3 Flash and GLM-5.3 FlashX always reason. Of the listed models, only `glm-5.3-flash` and its faster variant `glm-5.3-flashx` accept images.

#### Xiaomi MiMo

**Environment variable:** `XIAOMI_MIMO_API_KEY` (optional `XIAOMI_MIMO_HOST`)

Provider id `xiaomi_mimo`. Xiaomi's MiMo models, with regional endpoints.

Default model: `mimo-v2.6-flash`

Available models: `mimo-v2.6-flash`, `mimo-v2.6-pro`

Xiaomi shuts down `mimo-v2.5` and `mimo-v2.5-pro` on 2026-10-21 with no replacement routing, so a chat configured with either has to be switched to a V2.6 model.

#### DeepSeek

**Environment variable:** `DEEPSEEK_API_KEY`

Provider id `custom_deepseek`. DeepSeek models on an OpenAI-compatible API.

Default model: `deepseek-flash` (DeepSeek V4.1 Flash)

Available models: `deepseek-flash`, `deepseek-v4-pro`

A saved configuration that names a retired id (`deepseek-chat`, `deepseek-reasoner` or `deepseek-v4-flash`) keeps working: requests to a DeepSeek host send `deepseek-flash` in its place.

#### Moonshot AI (Kimi)

**Environment variable:** `MOONSHOT_API_KEY`

Provider id `moonshot`. The Kimi model family.

Default model: `kimi-k3`

Available models: `kimi-k3`, `kimi-k2.7-code`, `kimi-k2.7-code-highspeed`, `kimi-k2.6`

#### Groq

**Environment variable:** `GROQ_API_KEY`

Provider id `groq`. Open-weight models served on Groq hardware.

Default model: `openai/gpt-oss-120b`

Available models: `openai/gpt-oss-120b`, `qwen/qwen3.8-27b`, `openai/gpt-oss-20b`, `openai/gpt-oss-safeguard-20b`

#### Inception

**Environment variable:** `INCEPTION_API_KEY`

Provider id `inception`. Inception's Mercury diffusion language models.

Default model: `mercury-2.5`

Available models: `mercury-2.5`, `mercury-2`

#### Mistral AI

**Environment variable:** `MISTRAL_API_KEY`

Provider id `mistral`. Direct access to Mistral models.

Default model: `mistral-medium-3-5`

Available models include:

- `mistral-medium-3-5`, `mistral-large-2512`, `mistral-small-2603`
- `ministral-14b-2512`, `ministral-8b-2512`, `ministral-3b-2512`
- `codestral-2508`

#### LiteLLM

**Authentication:** depends on the configured backend

A proxy/gateway supporting many providers through a unified OpenAI-compatible interface.

Default model: `gpt-4o-mini`

#### Venice AI

**Environment variable:** `VENICE_API_KEY`

Privacy-focused inference provider.

Default model: `llama-3.3-70b`

Available models include:

- Llama 3.2 / 3.3 variants
- Mistral variants

#### AWS SageMaker TGI

**Authentication:** AWS credential chain

Run models deployed on AWS SageMaker endpoints using TGI (Text Generation Inference).

### Coding-agent providers

These two providers take no API key. They drive a coding-agent CLI you have already installed and signed in to, so inference runs on your own vendor subscription. Both are public providers: a consumer subscription carries no business associate agreement, so never use them with PHI. See [coding-agent providers](../providers/coding-agents/README.md) for setup and limits.

#### Claude Code

**Configuration:** `CLAUDE_CODE_COMMAND` (optional; overrides where Biorouter finds the `claude` CLI)

Provider id `claude_code`.

Default model: `claude-opus-5-5`, which needs `claude` 2.1.280 or newer.

Available models: `claude-opus-5-5`, `claude-fable-5-1`, `claude-sonnet-5`, `claude-haiku-4-5`

#### Codex

**Configuration:** `CODEX_COMMAND` (optional; overrides where Biorouter finds the `codex` CLI)

Provider id `codex`.

Default model: `gpt-6-astra`, which needs `codex-cli` 0.153.4 or newer.

Available models: `gpt-6-astra`, `gpt-6-sol`, `gpt-6-luna`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`. `gpt-6-sol` and `gpt-6-luna` need `codex-cli` 0.156.1 or newer.

### Custom / declarative providers

Any OpenAI-compatible endpoint can be added as a custom provider through the "Add Custom Provider" card. You specify:

- Display name
- API base URL
- API key environment variable name
- Model list
- Streaming support

Custom providers are stored in `~/.config/biorouter/config.yaml` and available in all future sessions.

## Switch providers and models

**Desktop:** Settings > Models > select a provider card > Configure or Launch > choose a model.

**CLI:**

```sh
biorouter configure
# Select "Configure Providers"
```

You can also specify provider and model on a per-session or per-workflow basis without changing your default configuration.

## Route across multiple models

Biorouter supports routing tasks across multiple models:

- **Lead/worker pattern** — A lead model orchestrates tasks and delegates sub-tasks to worker models (potentially different providers).
- **Per-workflow model override** — A workflow can specify `settings.biorouter_provider` and `settings.biorouter_model` to use a different model for that workflow without changing your default.
- **Per-session override** — The CLI supports `--provider` and `--model` flags when starting a session.

## Related documentation

- [Installation and setup](installation.md): the install path that leads into provider configuration, including the UCSF institutional options.
- [biorouter in 5 minutes](quickstart.md): the quickstart provider flow, which uses the Tetrate Agent Router.
- [Coding-agent providers](../providers/coding-agents/README.md): how Claude Code and Codex run on your own subscription, and why they must never see PHI.
- [Llama Server provider](../providers/llama-server/README.md): the bundled local provider and how its model catalog is verified.
- [Configuration file reference](../configuration/config-file-reference.md): the `config.yaml` keys that persist your provider and model choice.
- [Secret storage](../security/secret-storage.md): where the API keys named on this page are actually stored.
- [Environment variables](../configuration/environment-variables.md): the per-invocation form of the provider and model settings above.
