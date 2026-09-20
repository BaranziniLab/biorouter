# Choosing a model provider

> **What this is.** A reference of Biorouter's supported LLM providers: the credentials each one needs, its default model, a representative model list, and how to switch provider or override the choice per session.
> **Status:** Current, with one rotted section named here — the provider inventory and panel ordering below no longer match the shipping app. The live truth for panel ordering and grouping is [`ui/desktop/src/components/settings/providers/providerOrdering.ts`](../../ui/desktop/src/components/settings/providers/providerOrdering.ts); the live truth for which providers exist is the module list in `crates/biorouter/src/providers/`, and the Settings > Models panel in the app. The switching, orchestration, and custom-provider sections at the end remain accurate.
> **Audience:** end users

Biorouter connects to a wide range of LLM providers — commercial cloud APIs, institution-hosted services, and local models. You select and configure providers through the Provider Settings panel in the app (Settings > Models > Providers).

**UCSF users:** For institution-managed access, start with **Azure OpenAI** (UCSF ChatGPT) or **Amazon Bedrock** (UCSF-hosted Anthropic). For fully local, air-gapped inference, use **Ollama**.

> **Warning.** Four shipping providers have no entry on this page: **Llama Server** (`llamacpp`, the bundled llama.cpp sidecar and the first card a user sees), **Tetrate Agent Router** (`tetrate`, which [biorouter in 5 minutes](quickstart.md) recommends as the quickstart path), and the two UCSF institutional providers **`versa_azure`** and **`versa_bedrock`**. The generic "Azure OpenAI" and "Amazon Bedrock" sections below are the commercial providers, not the UCSF institutional ones. Configure any of these four from Settings > Models in the app.

> **Note.** The model lists on this page are hand-maintained snapshots with no recorded "as of" date, and they drift. Two are already internally inconsistent (flagged in place below). Treat the live model picker — which fetches from the provider — as authoritative.

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

### Azure OpenAI

**Environment variable:** `AZURE_OPENAI_API_KEY` (or Azure credential chain)

Start from this profile to access UCSF-hosted ChatGPT models. Uses Azure credential chain by default, making it compatible with institutional single sign-on.

Default model: `gpt-5.4-2026-03-05`

Available models include:

- gpt-4o, gpt-4o-mini, gpt-4

> **Warning.** The default model above does not appear in its own "available models" list. One of the two is out of date.

### Amazon Bedrock

**Environment variables:** `AWS_PROFILE`, `AWS_REGION` (or standard AWS credential chain)

Start from this profile to access UCSF-hosted Anthropic models. Supports AWS SSO profiles — run `aws sso login --profile <profile-name>` before using.

Default model: `us.anthropic.claude-sonnet-4-6`

Available models include:

- Claude Sonnet 4.5 (via Bedrock)
- Multiple Claude 3/4 variants

> **Warning.** The default model above is Sonnet 4.6, but the "available models" list names Sonnet 4.5. One of the two is out of date.

### Anthropic

**Environment variable:** `ANTHROPIC_API_KEY`

Direct API access to Anthropic's Claude models.

Default model: `claude-opus-4-8`

Available models include:

- claude-opus-4-8
- claude-sonnet-4-6
- claude-haiku-4-5

### OpenAI

**Environment variable:** `OPENAI_API_KEY`

Direct API access to OpenAI models.

Default model: `gpt-5.5`

Available models include:

- gpt-5.5, gpt-5.4-mini
- gpt-4.1
- o1, o3

Optional configuration: `OPENAI_ORG_ID`, `OPENAI_PROJECT_ID`

### Google Gemini

**Environment variable:** `GOOGLE_API_KEY`

Direct API access to Google's Gemini models.

Default model: `gemini-3.1-pro-preview`

Available models include:

- gemini-3.1-pro-preview
- gemini-2.5-pro
- gemini-2.5-flash variants

### GCP Vertex AI

**Authentication:** Service account or application default credentials

Runs Google and Anthropic models through Google Cloud's Vertex AI infrastructure.

Default model: `gemini-3.5-flash`

### Databricks

**Environment variables:** `DATABRICKS_HOST`, `DATABRICKS_TOKEN`

Access models through Databricks. Supports OAuth.

Default model: `databricks-claude-sonnet-4-6`

Available models include:

- Claude variants
- Llama models
- DBRX Instruct

### Snowflake Cortex

**Authentication:** Snowflake credentials

Access Claude and other models through Snowflake's Cortex integration.

Default model: `claude-sonnet-4-6`

### Ollama (local)

**No API key required** — runs fully on your machine.

Use Ollama for completely local, private inference. No data leaves your device.

Default model: `qwen3`

Available models include:

- qwen3, qwen3-coder variants
- Any model available in the Ollama library

To use: install [Ollama](https://ollama.com), pull a model (`ollama pull qwen3`), then configure Biorouter to use the Ollama provider. The endpoint defaults to `http://localhost:11434`.

### OpenRouter

**Environment variable:** `OPENROUTER_API_KEY`

A proxy service that provides access to many providers through a single API.

Default model: `anthropic/claude-sonnet-4.6`

Available models include access to Anthropic, Google, Deepseek, Qwen, and many others.

### LiteLLM

**Authentication:** depends on the configured backend

A proxy/gateway supporting many providers through a unified OpenAI-compatible interface.

Default model: `gpt-4o-mini`

### Venice AI

**Environment variable:** `VENICE_API_KEY`

Privacy-focused inference provider.

Default model: `llama-3.3-70b`

Available models include:

- Llama 3.2 / 3.3 variants
- Mistral variants

### GitHub Copilot

**Authentication:** Device code OAuth flow

Access GPT, Claude, Gemini, and Grok models through GitHub Copilot infrastructure.

Default model: `gpt-5.3-codex`

GitHub Copilot does not use an API key. During configuration an auth code is copied to your clipboard and a browser window opens for you to paste it. See the configuration walkthrough in [biorouter in 5 minutes](quickstart.md#cli).

### X.AI (Grok)

**Environment variable:** `XAI_API_KEY`

Access Grok models from xAI.

Default model: `grok-4.3`

### AWS SageMaker TGI

**Authentication:** AWS credential chain

Run models deployed on AWS SageMaker endpoints using TGI (Text Generation Inference).

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

- [Installation and setup](installation.md) — the install path that leads into provider configuration, including the UCSF institutional options.
- [biorouter in 5 minutes](quickstart.md) — the quickstart provider flow, including the Tetrate Agent Router path not covered on this page.
- [Configuration file reference](../configuration/config-file-reference.md) — the `config.yaml` keys that persist your provider and model choice.
- [Secret storage](../security/secret-storage.md) — where the API keys named on this page are actually stored.
- [Environment variables](../configuration/environment-variables.md) — the per-invocation form of the provider and model settings above.
