# Choosing a model provider

> **What this is.** A reference of Biorouter's supported LLM providers: the credentials each one needs, its default model, a representative model list, and how to switch provider or override the choice per session.
> **Status:** Current source snapshot, checked against Biorouter's bundled provider definitions on 2026-11-18 and the official model documentation reviewed on 2026-11-17, with Baseten's primary model list checked on 2026-11-18. The dated UCSF gateway checks below are historical observations; this refresh made no new UCSF inference request. Account, deployment, region and installed-model availability remain provider-specific.
> **Audience:** end users

Biorouter connects to a wide range of LLM providers: commercial cloud APIs, institution-hosted services, and local models. You select and configure providers in the provider panel of the app: open **Settings** > **Models**, then choose **Manage…** on the **Providers** row.

**UCSF users:** For institution-managed access, start with **Versa API Azure** (`versa_azure`, the UCSF ChatGPT models) or **Versa API Bedrock** (`versa_bedrock`, the UCSF-hosted Anthropic models). The generic **Azure OpenAI** and **Amazon Bedrock** providers are the commercial ones, not the UCSF institutional ones. For fully local inference, use **Llama Server** or **Ollama**.

> **Note.** These representative bundled lists change between releases. The authoritative definitions are the provider constants in `crates/biorouter/src/providers/` and the JSON files in `crates/biorouter/src/providers/declarative/`. The picker uses those definitions or the provider's own discovery path. A listed model still needs to be available to your account or endpoint.

For a bundled list, new provider setup starts with its declared default and the picker moves that default first when it is in the available list and retains the other entries' order. Live Codex and GitHub Copilot lists use the account's reported priority instead. A saved selection remains selected, including a model no longer suggested for new chats.

## Provider configuration panel

Open **Settings** > **Models** and choose **Manage…** on the **Providers** row. Each provider row shows:

- The provider's name, a one-line description, and **Configured** when it is set up.
- A **Configure** button to enter API keys or credentials. It appears when you point at the row or tab to it, and choosing the row itself opens it too.

The first run screen shows the same rows, and there a configured provider also has **Launch**, which switches to that provider and asks for a model. Everywhere else you choose a model in **Switch models**.

The providers sit on three tabs, always in this order:

| Tab | Opens with | Providers, in order |
|---|---|---|
| **Local** | "Private because inference runs on this machine." | `llamacpp` (Llama Server), `ollama`, then any other local provider by display name |
| **Institutional** | "Private because Biorouter recognizes this institutional gateway endpoint." | One section for each institution, named as its provider publishes it (such as UCSF, with `versa_azure` and `versa_bedrock`), sections in name order, then "Unaffiliated private gateways" |
| **Public** | "Public. Biorouter can't verify where this account's endpoint points…" | "AI agents · your subscription" (`claude_code`, then `codex`), then "API providers": `anthropic`, `openai` and `google`, then every other public provider by display name |

The panel hides nothing: every provider Biorouter has is on one of the three tabs. A tab opens selected when it holds your current provider. In `biorouter configure`, the providers are one list with no group headings: Llama Server, Ollama, the two Versa providers, then the rest by display name.

You can also add fully custom providers (e.g. any OpenAI-compatible endpoint) with **Add custom provider**, the last row of the API providers.

## Supported providers

The providers are grouped below: local models, UCSF institutional models, commercial providers, and the coding-agent providers that drive a CLI you have installed.

### Local models

#### Llama Server

**No API key required.** Provider id `llamacpp`.

The desktop app bundles a pinned llama.cpp `llama-server` binary and runs it for you. It downloads a model's weights once, on first use, or starts from a copy already on disk (including a GGUF build you pulled into Ollama); after that, nothing leaves your machine.

Default model: `qwen3.8-27b` (Qwen3.8 27B), on every machine.

Available models:

- `qwen3.8-27b`: Qwen3.8 27B, about 19 GB, the default
- `gemma4-31b`: Gemma 4 31B, about 18 GB
- Any other model, given as a raw Hugging Face `owner/repo:QUANT` spec

Both models need about 32 GiB of GPU-addressable memory (unified memory on Apple Silicon), and 48 GiB is recommended. They still run below that, slowly. On an Intel Mac the bundled server has no GPU acceleration, so expect a few tokens per second. A model name from an earlier version of Biorouter, such as `gemma4` or `gemma4-12b`, now runs `qwen3.8-27b`. See the [Llama Server model catalog](../providers/llama-server/model-catalog.md) for the details.

Optional configuration includes `LLAMACPP_CONTEXT_SIZE`, `LLAMACPP_ENABLE_THINKING` and `LLAMACPP_EXTERNAL_HOST`.

#### Ollama (local)

**No API key required**: runs fully on your machine.

Use Ollama for completely local, private inference with an [Ollama](https://ollama.com) server you install and run yourself. No data leaves your device.

Default model: `qwen3.8:27b`

Supported models:

- `qwen3.8:27b` (Qwen3.8 27B, about 18 GB), the default
- `gemma4:31b` (Gemma 4 31B, about 19 GB)

To use: install Ollama 0.32.12 or later (the oldest that runs `qwen3.8:27b`; `gemma4:31b` needs 0.31.0 on Apple Silicon and 0.30.9 elsewhere), pull a model (`ollama pull qwen3.8:27b`), then configure Biorouter to use the Ollama provider. The endpoint defaults to `http://localhost:11434`. The model list shows the models you have pulled that can call tools. Other models may work, but many write their tool calls as text or return empty replies; see [which local models to use](../providers/ollama/tool-call-protocols.md#which-models-to-use). Biorouter sets the context window itself (`OLLAMA_NUM_CTX` overrides it); see [context window and settings](../providers/ollama/context-window-and-settings.md).

Biorouter checks the configured server and selected model before inference; if the check
fails, follow its guidance and choose **Check again** after correcting the service,
address or model.

### Institutional models (UCSF)

#### Versa API Azure

**Environment variable:** `VERSA_AZURE_API_KEY`

Provider id `versa_azure`. The UCSF ChatGPT models, served from UCSF's Azure tenant. Only the API key is asked for; the endpoint and the deployment for each model are preconfigured.

Default model: `gpt-5.6-sol-2026-07-09`

Available models, in the order the model picker lists them:

- GPT-5.6: `gpt-5.6-sol-2026-07-09`, `gpt-5.6-terra-2026-07-09`, `gpt-5.6-luna-2026-07-09`
- `gpt-5.5-2026-04-24`
- `gpt-5.4-mini-2026-03-17`, `gpt-5.4-nano-2026-03-17`
- `gpt-5.2-2025-12-11`
- `gpt-5-2025-08-07`, `gpt-5-mini-2025-08-07`, `gpt-5-nano-2025-08-07`
- `gpt-4o-2024-11-20`

Each model maps to a UCSF deployment, so this provider does not accept a model outside the list. The three GPT-5.6 models became available at UCSF on 2026-10-01, with a 1,050,000-token context window. Biorouter sends them to the Responses API, because on Chat Completions they cannot use tools and reasoning in the same request. The other models stay on Chat Completions. The 2026-10-05 check did not establish a UCSF GPT-6 deployment. This refresh retains the institution-specific list; check UCSF's current deployment documentation for later availability.

`gpt-5.6-sol-2026-07-09` is the default after Biorouter 1.92.1. Up to 1.92.1 the default was `gpt-5.5-2026-04-24`, which is still offered. An existing configuration keeps the model it names, and a chat keeps the model it is bound to. The default is what a new setup of this provider starts on, and what the model picker selects when you switch to this provider.

A chat already bound to `o4-mini-2025-04-16`, `gpt-4.1-2025-04-14` or `gpt-4.1-mini-2025-04-14` keeps working until Azure retires the model (2026-11-19 for `o4-mini`, 2027-04-14 for the two GPT-4.1 models), but those models are no longer offered. UCSF names `gpt-5.6-terra-2026-07-09` as the replacement for `o4-mini`.

#### Versa API Bedrock

**Environment variables:** `VERSA_BEDROCK_ACCESS_KEY_ID`, `VERSA_BEDROCK_SECRET_ACCESS_KEY`

Provider id `versa_bedrock`. The UCSF-hosted Anthropic models, served through Amazon Bedrock. Only the access key pair is asked for; the UCSF endpoint and the region are preconfigured.

Default model: `us.anthropic.claude-opus-5-5`

Available models:

- `us.anthropic.claude-opus-5-5` (Claude Opus 5.5), `us.anthropic.claude-sonnet-5-5` (Claude Sonnet 5.5), `us.anthropic.claude-haiku-5-5` (Claude Haiku 5.5)
- `us.anthropic.claude-opus-4-8` (Claude Opus 4.8)
- `us.anthropic.claude-sonnet-5` (Claude Sonnet 5)
- `us.anthropic.claude-opus-4-6-v1` (Claude Opus 4.6), `us.anthropic.claude-sonnet-4-6` (Claude Sonnet 4.6)
- `us.anthropic.claude-opus-4-5-20251101-v1:0` (Claude Opus 4.5)
- `us.anthropic.claude-haiku-4-5-20251001-v1:0` (Claude Haiku 4.5)

`us.anthropic.claude-opus-5-5` is the default from 2026-11-12. Up to then it was `us.anthropic.claude-opus-4-8`, which is still offered. An existing configuration keeps the model it names, and a chat keeps the model it is bound to.

Claude Opus 5.5, Sonnet 5.5 and Haiku 5.5 each answered a request and made a tool call through the UCSF gateway on 2026-11-12. For each one the gateway named a 1,000,000-token prompt limit and a 128,000-token output limit when a request went over them, which matches [Anthropic's model specifications](https://platform.claude.com/docs/en/models/overview). Opus 5.5 and Sonnet 5.5 were first reported working on 2026-10-22 in [PR #434](https://github.com/BaranziniLab/biorouter/pull/434). The other listed models answered on 2026-10-05. These are checks of what UCSF's account may use, and UCSF can change that.

Biorouter's cost estimates apply its bundled provider/model rate cards, including prompt-size tiers where present. The Anthropic-based rates used for Versa are estimates, not a fresh verification of UCSF's bill; Haiku 5.5's gateway rate remains unconfirmed. See [usage limits](../agent-loop/usage-limits.md#when-a-dollar-limit-cannot-fire) for the effect of unknown hosted rates. None of these three models accepts a temperature setting, so Biorouter does not send one.

At the recorded gateway checks, UCSF did not serve Claude Opus 5, Opus 4.7, Sonnet 4.5, Sonnet 4, Fable 5.1 or Fable 5 through Versa. Bedrock refused those requests from the UCSF account, and these models remain outside the bundled list. A chat still set to one of them stops with a message that names the models above; your access key pair is not the problem. Claude Fable models would stay off the list even if UCSF enabled them, because Bedrock serves Fable only to accounts whose data retention mode is `aws_review`. Claude Opus 4.1 still answers, but Amazon has marked it legacy and ends it on 2027-01-08, so it is not offered either. Claude Opus 4 and the Claude 3 models are retired on Bedrock.

### Commercial providers

#### Anthropic

**Environment variable:** `ANTHROPIC_API_KEY`

Direct API access to Anthropic's Claude models.

Default model: `claude-opus-5-5`. Fast model, used for background calls such as chat titles and summaries: `claude-haiku-5-5`.

Available models include:

- `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`, `claude-fable-5-2`
- `claude-fable-5-1`
- `claude-opus-5`, `claude-sonnet-5`
- `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`, `claude-sonnet-4-6`
- `claude-haiku-4-5`

Claude Fable remains an explicit optional tier with account and retention requirements. See [Anthropic and Claude Code models](../providers/anthropic-models.md) for current identifiers, access and client-version boundaries.

#### OpenAI

**Environment variable:** `OPENAI_API_KEY`

Direct API access to OpenAI models.

Default model: `gpt-6.2-astra`. Fast model, used for background calls such as chat titles and summaries: `gpt-6-luna`.

Available models include:

- GPT-6.2: `gpt-6.2-astra`, `gpt-6.2-sol`
- GPT-6.1: `gpt-6.1-terra`, `gpt-6.1-sol`
- GPT-6: `gpt-6-astra`, `gpt-6-sol`, `gpt-6-luna`
- GPT-5.6: `gpt-5.6-sol` (also reachable as `gpt-5.6`), `gpt-5.6-terra`, `gpt-5.6-luna`
- `gpt-5.5`, `gpt-5.5-pro`
- `gpt-5.4`, `gpt-5.4-pro`, `gpt-5.4-mini`, `gpt-5.4-nano`
- `gpt-5.3-codex`
- `gpt-5.2`, `gpt-5.1`
- `gpt-4.1`, `gpt-4.1-mini`, `gpt-4o-mini`

Terra is available as `gpt-6.1-terra` and `gpt-5.6-terra`. `gpt-5`, `gpt-5-mini`, `gpt-5-nano` and `o3` are no longer listed, because OpenAI shuts them down on 2026-12-11. OpenAI names `gpt-5.6-sol` as the replacement for `gpt-5` and `o3`, `gpt-5.6-terra` for `gpt-5-mini`, and `gpt-5.6-luna` for `gpt-5-nano`.

GPT-6, GPT-5.6, GPT-5.5, GPT-5.4 and `gpt-5.3-codex` go through OpenAI's Responses API (`/v1/responses`); the older models use Chat Completions. Biorouter picks the route from the model name. `OPENAI_BASE_PATH` changes only the Chat Completions path, so a proxy set through `OPENAI_HOST` must also serve `/v1/responses` for those newer models.

Optional configuration: `OPENAI_HOST`, `OPENAI_BASE_PATH`, `OPENAI_ORGANIZATION`, `OPENAI_PROJECT`, `OPENAI_CUSTOM_HEADERS`, `OPENAI_TIMEOUT`. See [OpenAI models](../providers/openai.md) for current API limits and the separate Codex account contract.

#### Google Gemini

**Environment variable:** `GOOGLE_API_KEY`

Direct API access to Google's Gemini models.

Default model: `gemini-4-hydrogen`. Fast model, used for background calls such as chat titles and summaries: `gemini-3.5-flash-lite`.

Available models include:

- `gemini-4-hydrogen`, `gemini-4-argon`, `gemini-4-carbon`
- `gemini-3.8-flash`, `gemini-3.7-flash`, `gemini-3.6-flash`, `gemini-3.5-flash`
- `gemini-3.5-flash-lite`, `gemini-3.1-flash-lite`
- `gemini-3.1-pro-preview`, `gemini-3-flash-preview`
- `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.5-flash-lite`

Since 2026-09-18 Google serves the Gemini 2.5 models only to API keys that used them before. See [Google Gemini](../providers/google.md) for Gemini 4 thinking, output and history requirements.

#### GCP Vertex AI

**Environment variables:** `GCP_PROJECT_ID`, `GCP_LOCATION` (default `us-central1`). Authentication uses a service account or application default credentials.

Runs Google and Anthropic models through Google Cloud's Vertex AI infrastructure.

Default model: `gemini-4-hydrogen`

Available models include:

- `gemini-4-hydrogen`, `gemini-4-argon`
- `gemini-3.8-flash`, `gemini-3.7-flash`, `gemini-3.6-flash`, `gemini-3.5-flash`, `gemini-3.5-flash-lite`
- `gemini-3.1-pro-preview`, `gemini-3.1-flash-lite`, `gemini-3-flash-preview`
- `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`, `claude-fable-5-1`
- `claude-opus-5`, `claude-sonnet-5`
- `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`, `claude-sonnet-4-6`

A Claude model must first be enabled in your project's Model Garden. Gemini 4 Carbon's restricted deployment and Claude Fable 5.2 are not bundled for this Vertex adapter. Direct Gemini availability does not establish Vertex project access.

`GCP_LOCATION` may be a single region such as `us-central1`, the `global` endpoint, or the `us` or `eu` multi-region. Vertex serves Gemini 3.x and 4.x, Claude 5.x, Claude Opus 4.7 and Claude Opus 4.8 at `global` and the `us` and `eu` multi-regions rather than in single regions like `us-central1`. So when `GCP_LOCATION` names a single region, Biorouter sends those models to the `global` endpoint automatically. When it is `us` or `eu`, Biorouter keeps that multi-region, so requests stay in that geography; the two preview models always go to `global`. Gemini 4 Argon also honors its documented single regions `europe-west2` and `asia-southeast1`. Older Claude models and Gemini 2.x use the configured region. The `us` and `eu` multi-regions do not serve them, so with either one they go to a single region in that geography instead (`europe-west1` for `eu`; `us-east5` for Claude or `us-central1` for Gemini with `us`), and a failed request is never retried outside that geography.

#### Azure OpenAI

**Environment variables:** `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_DEPLOYMENT_NAME`, and optionally `AZURE_OPENAI_API_KEY` (leave it empty to use the Azure credential chain) and `AZURE_OPENAI_API_VERSION`

The generic Azure OpenAI provider, for your own Azure resource. It is a commercial provider; UCSF users should use [Versa API Azure](#versa-api-azure) instead.

Default model: `gpt-6.2-astra-2026-11-17`

Available models include:

- GPT-6.2: `gpt-6.2-astra-2026-11-17`, `gpt-6.2-sol-2026-11-17`
- GPT-6.1: `gpt-6.1-sol-2026-09-29`, `gpt-6.1-terra-2026-10-26`
- GPT-6: `gpt-6-sol-2026-09-22`, `gpt-6-astra-2026-09-03`, `gpt-6-luna-2026-09-22`
- GPT-5.6: `gpt-5.6-sol-2026-07-09`, `gpt-5.6-terra-2026-07-09`, `gpt-5.6-luna-2026-07-09`
- `gpt-5.5-2026-04-24`
- `gpt-5.4-2026-03-05`, `gpt-5.4-mini-2026-03-17`, `gpt-5.4-nano-2026-03-17`
- `gpt-5.2-2025-12-11`, `gpt-5.1-2025-11-13`, `gpt-5-2025-08-07`
- `gpt-4o-2024-11-20`

Each id is the Azure model name plus its model version. The deployment a request goes to is `AZURE_OPENAI_DEPLOYMENT_NAME`. Azure notes that some quota tiers need a quota request for the GPT-6 family. Your deployment, region and quota determine access. Newly added hosted models do not inherit an OpenAI retail price when Azure's rate is unverified.

GPT-6, GPT-5.6, GPT-5.5 and GPT-5.4 models use `POST {endpoint}/openai/v1/responses`, with the deployment name as the `model` and no `api-version`. `AZURE_OPENAI_API_VERSION` (default `2025-01-01-preview`) applies only to the models that use Chat Completions: GPT-5.2, GPT-5.1, GPT-5 and GPT-4o.

Azure has deprecated `o1`, `o3-mini`, `o3` and `o4-mini` (retiring 2026-11-19) and GPT-4.1 and GPT-4.1 mini (retiring 2027-04-14), so they are no longer listed. On 2026-11-19 Azure upgrades Standard and Global Standard o-series deployments to GPT-5.6. From then on a chat configured as `o4-mini` keeps working, but one configured as `o1`, `o3` or `o3-mini` has to be switched to a GPT-5.6 model id.

#### Amazon Bedrock

**Environment variables:** `AWS_PROFILE`, `AWS_REGION` (or standard AWS credential chain)

The generic Amazon Bedrock provider, for your own AWS account. It is a commercial provider; UCSF users should use [Versa API Bedrock](#versa-api-bedrock) instead. Supports AWS SSO profiles: run `aws sso login --profile <profile-name>` before using.

Default model: `us.anthropic.claude-opus-5-5`

Available models include:

- `us.anthropic.claude-opus-5-5`, `us.anthropic.claude-sonnet-5-5`, `us.anthropic.claude-haiku-5-5`, `us.anthropic.claude-fable-5-1`
- `us.anthropic.claude-opus-5`, `us.anthropic.claude-sonnet-5`
- `us.anthropic.claude-opus-4-8`, `us.anthropic.claude-sonnet-4-6`, `us.anthropic.claude-opus-4-6-v1`
- `us.anthropic.claude-opus-4-5-20251101-v1:0`, `us.anthropic.claude-sonnet-4-5-20250929-v1:0`, `us.anthropic.claude-haiku-4-5-20251001-v1:0`

Claude Fable 5.1 on Bedrock needs the AWS account's Bedrock data retention mode set to `aws_review`, or every request fails.

#### Databricks

**Environment variables:** `DATABRICKS_HOST`, `DATABRICKS_TOKEN`

Access models through Databricks. Supports OAuth.

Default model: `databricks-claude-opus-5-5`. Fast model, used for background calls such as chat titles and summaries: `databricks-gemini-3-5-flash`, when your workspace serves it.

Available models include:

- Claude: `databricks-claude-opus-5-5`, `databricks-claude-sonnet-5-5`, `databricks-claude-haiku-5-5`, `databricks-claude-sonnet-5`, `databricks-claude-sonnet-4-6`
- OpenAI: `databricks-gpt-6-1-sol`, `databricks-gpt-6-1-terra`, `databricks-gpt-6-astra`, `databricks-gpt-6-sol`, `databricks-gpt-6-luna`
- Google: `databricks-gemini-4-hydrogen`, `databricks-gemini-4-argon`, `databricks-gemini-3-8-flash`, `databricks-gemini-3-5-flash`
- Meta: `databricks-meta-llama-3-3-70b-instruct`, `databricks-llama-4-maverick`

The bundled Claude 5.5 models use this provider's invocations adapter. Claude Fable 5.1 and 5.2 remain excluded because their documented Anthropic Messages path needs another adapter. Workspace, region and endpoint access still apply; newly added hosted rates stay unknown without host-specific pricing.

#### Snowflake Cortex

**Environment variables:** `SNOWFLAKE_HOST`, `SNOWFLAKE_TOKEN`

Access Claude and other models through Snowflake's Cortex integration.

Default model: `claude-opus-5-5`

Available models include:

- `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-sonnet-4-6`, `claude-sonnet-5`, `claude-opus-5`
- `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6`
- `claude-opus-4-5`, `claude-sonnet-4-5`, `claude-haiku-4-5`

#### OpenRouter

**Environment variable:** `OPENROUTER_API_KEY`

A proxy service that provides access to many providers through a single API.

Default model: `anthropic/claude-opus-5.5`. Fast model, used for background calls such as chat titles and summaries: `google/gemini-3.5-flash`.

Available models include:

- `anthropic/claude-opus-5.5`, `anthropic/claude-sonnet-5.5`, `anthropic/claude-haiku-5.5`, `anthropic/claude-fable-5.1`
- `openai/gpt-6.2-sol`, `openai/gpt-6.2-astra`, `openai/gpt-6-astra`, `openai/gpt-6-sol`, `openai/gpt-6-luna`
- `google/gemini-4-hydrogen`, `google/gemini-4-argon`, `google/gemini-3.1-pro-preview`, `google/gemini-3.8-flash`
- `x-ai/grok-4.7`, `deepseek/deepseek-v4.1-flash`, `moonshotai/kimi-k3`, `z-ai/glm-5.3`, `qwen/qwen3-coder-next`, `minimax/minimax-m3`

#### Tetrate Agent Router Service

**Environment variable:** `TETRATE_API_KEY` (optional `TETRATE_HOST`)

Provider id `tetrate`. A routing service across upstream models, and the provider [biorouter in 5 minutes](quickstart.md) uses. Its automatic setup signs you in through the browser.

Default model: `claude-opus-5-5`

Available models include:

- `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`, `claude-fable-5-1`, `claude-sonnet-5`
- `gemini-2.5-pro`, `gemini-2.5-flash`
- `gpt-4.1`

#### GitHub Copilot

**Authentication:** Device code OAuth flow

Access GPT, Claude, Gemini, and Grok models through GitHub Copilot infrastructure.

Fresh choices come from the signed-in account's eligible chat-completions catalogue. An eligible API-marked default comes first. Otherwise, an eligible account-reported `claude-sonnet-5.5` is preferred under GitHub's daily-use guidance; other eligible entries retain service order. Hidden, disabled, Auto-only and incompatible-endpoint rows do not become named choices.

A successful empty catalogue stays empty. If discovery is unavailable, the named fallback is `gpt-5.3-codex`, GitHub's documented Business/Enterprise LTS choice; that fallback does not prove account entitlement. Saved explicit models, including older identifiers absent from current suggestions, remain selected. Static capability metadata is retained for attachments, not merged into fresh account suggestions. No upstream API token price is assigned to a Copilot subscription.

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

Default model: `kimi-k3.1-pro`

Available models: `kimi-k3.1-pro`, `kimi-k3.1-pro-highspeed`, `kimi-k3`, `kimi-k2.7-code`, `kimi-k2.7-code-highspeed`, `kimi-k2.6`

#### Groq

**Environment variable:** `GROQ_API_KEY`

Provider id `groq`. Open-weight models served on Groq hardware.

Default model: `openai/gpt-oss-120b`

Available models: `openai/gpt-oss-120b`, `qwen/qwen3.8-27b`, `openai/gpt-oss-20b`, `openai/gpt-oss-safeguard-20b`

#### Baseten

**Environment variable:** `BASETEN_API_KEY`

Provider id `baseten`. Open-weight models hosted on Baseten Model APIs (`https://inference.baseten.co/v1`). Create a key at https://app.baseten.co/settings/api_keys.

Default model: `zai-org/GLM-5.3`

Available models: `zai-org/GLM-5.3`, `zai-org/GLM-5.3-Fast`, `zai-org/GLM-5.3-Flash`, `moonshotai/Kimi-K3.1-Pro`, `moonshotai/Kimi-K3`, `deepseek-ai/DeepSeek-V4.1-Flash`, `deepseek-ai/DeepSeek-V4.1-Flash-1017`, `deepseek-ai/DeepSeek-V4-Pro-0813`, `meta-models/muse-spark-open`, `nvidia/NVIDIA-Nemotron-3.5-Super-VL-120B-A12B`, `nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B`, `openai/gpt-oss-120b`

Once your Baseten key is saved, this list and the prices refresh from Baseten's catalog at most twice a day, so the picker can show models released after your version of Biorouter. Without a key Biorouter asks for nothing. Offline, the last list Biorouter saw or the list above is used, unless it is older than the list above. See [the Baseten provider reference](../providers/baseten.md).

Slugs are case-sensitive, as Baseten spells them. Costs use Baseten's own per-token rates, including cached input. The bundle's freshness cutoff is 2026-11-18. Its host-specific Kimi K3.1 entry is separate from native Moonshot and OpenRouter routes; their context and price records do not substitute for one another.

#### Inception

**Environment variable:** `INCEPTION_API_KEY`

Provider id `inception`. Inception's Mercury diffusion language models.

Default model: `mercury-3`

Available models: `mercury-3`, `mercury-2.5`, `mercury-2`. Inception schedules these Model APIs to end on 2026-12-28; an existing choice is preserved, but continued service depends on that lifecycle.

#### Mistral AI

**Environment variable:** `MISTRAL_API_KEY`

Provider id `mistral`. Direct access to Mistral models.

Default model: `mistral-large-4`

Available models include:

- `mistral-large-4`, `mistral-medium-3-5`, `mistral-large-2512`, `mistral-small-2603`
- `ministral-14b-2512`, `ministral-8b-2512`, `ministral-3b-2512`
- `codestral-2508`

#### LiteLLM

**Authentication:** depends on the configured backend

A proxy/gateway supporting many providers through a unified OpenAI-compatible interface.

Default model: `gpt-4o-mini`

#### Venice AI

**Environment variable:** `VENICE_API_KEY`

Privacy-focused inference provider.

Default model: `deepseek-v4-1-flash-1017-uncensored`

Available bundled choices include that model, `llama-3.2-3b` and `llama-3.3-70b`. The endpoint may list additional account-available models.

#### AWS SageMaker TGI

**Authentication:** AWS credential chain

Run models deployed on AWS SageMaker endpoints using TGI (Text Generation Inference).

Default model: `sagemaker-tgi-endpoint`, a placeholder. The model is whatever your endpoint serves, and you name the endpoint in `SAGEMAKER_ENDPOINT_NAME`.

### Coding-agent providers

These two providers take no API key. They drive a coding-agent CLI you have already installed and signed in to, so inference runs on your own vendor subscription. Both are public providers: a consumer subscription carries no business associate agreement, so never use them with PHI. See [coding-agent providers](../providers/coding-agents/README.md) for setup and limits.

#### Claude Code

**Configuration:** `CLAUDE_CODE_COMMAND` (optional; overrides where Biorouter finds the `claude` CLI)

Provider id `claude_code`.

Default model: `claude-opus-5-5`, which needs `claude` 2.1.280 or newer.

The bundled list starts with `claude-opus-5-5`, `claude-sonnet-5-5` and `claude-haiku-5-5`, then the optional `claude-fable-5-2` tier. Sonnet 5.5 needs Claude Code 2.1.284, Haiku 5.5 needs 2.1.293 and Fable 5.2 needs 2.1.321. See [Anthropic and Claude Code models](../providers/anthropic-models.md) for access and usage-credit boundaries.

#### Codex

**Configuration:** `CODEX_COMMAND` (optional; overrides where Biorouter finds the `codex` CLI)

Provider id `codex`.

The account's visible text-capable `model/list` catalogue supplies its preferred default and available models. Saved explicit choices stay selected. An unavailable or empty discovery result uses the verified bundled fallback: `gpt-6-astra` first, then `gpt-6-sol`, `gpt-6-luna`, `gpt-5.6-sol`, `gpt-5.6-terra` and `gpt-5.6-luna`. Astra needs `codex-cli` 0.153.4 or newer; bundled Sol and Luna need 0.156.1.

API model availability and context do not establish Codex subscription availability or working context. See [coding-agent performance and limits](../providers/coding-agents/performance-and-limits.md#codex-model-and-reasoning-selection) for runtime default and effort selection.

### Custom / declarative providers

Any OpenAI-compatible endpoint can be added as a custom provider through the "Add Custom Provider" card. You specify:

- Display name
- API base URL
- API key environment variable name
- Model list
- Streaming support

Custom providers are stored in `~/.config/biorouter/config.yaml` and available in all future sessions.

## Switch providers and models

**Desktop:** in a chat, choose the model chip under the message box, then pick a provider and model in **Switch models**. To set up a provider first, open **Settings** > **Models** > **Providers** > **Manage…**, and choose **Configure** on its row.

**CLI:**

```sh
biorouter configure
# Select "Configure Providers"
```

You can also specify provider and model on a per-session or per-workflow basis without changing your default configuration.

## Route across multiple models

Biorouter supports routing tasks across multiple models:

- **Default worker model**: the chat's model leads, and the subagents it starts run on a default worker model you choose (potentially a different provider), unless the agent names another. See [Model control](../agent-loop/subagents.md#model-control).
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
