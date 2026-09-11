# Environment variables

> **What this is.** The grouped reference of every environment variable that customizes biorouter's behaviour, from provider selection through observability, workflow discovery, and test isolation.
> **Status:** Current.
> **Audience:** end users

Environment variables are the per-invocation form of biorouter's settings: they override anything set in the [configuration files](config-file-reference.md), so you can change a model, a permission mode, or a data directory for one command without editing YAML. This page lists them by the subsystem they affect.

> **Note.** The `**Examples**` command blocks throughout this page were lost during the 2026-05 docs-site-to-markdown migration; several sections retain only the captions of the examples that used to follow. The tables are complete and authoritative — treat a missing example block as missing, not as an indication that the variable is unused.

## Contents

- [Model configuration](#model-configuration)
- [Session management](#session-management)
- [Tool configuration](#tool-configuration)
- [Security configuration](#security-configuration)
- [Browser access](#browser-access)
- [Observability](#observability)
- [Workflow configuration](#workflow-configuration)
- [Experimental features](#experimental-features)
- [Development and testing](#development-and-testing)
- [Variables controlled by biorouter](#variables-controlled-by-biorouter)
- [Variables documented elsewhere](#variables-documented-elsewhere)
- [Precedence and caveats](#precedence-and-caveats)

## Model configuration

These variables control the [language models](../getting-started/choosing-a-model-provider.md) and their behaviour.

### Basic provider configuration

These are the minimum variables required to get started with biorouter.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_PROVIDER` | Specifies the large language model (LLM) provider to use | [See available providers](../getting-started/choosing-a-model-provider.md) | None (must be configured) |
| `BIOROUTER_MODEL` | Specifies which model to use from the provider | Model name (e.g. `"gpt-4"`, `"claude-sonnet-4-20250514"`) | None (must be configured) |
| `BIOROUTER_TEMPERATURE` | Sets the [temperature](https://medium.com/@kelseyywang/a-comprehensive-guide-to-llm-temperature-%EF%B8%8F-363a40bbc91f) for model responses | Float between 0.0 and 1.0 | Model-specific default |

### Advanced provider configuration

These variables are needed when using custom endpoints, enterprise deployments, or specific provider implementations.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_PROVIDER__TYPE` | The specific type/implementation of the provider | [See available providers](../getting-started/choosing-a-model-provider.md) | Derived from `BIOROUTER_PROVIDER` |
| `BIOROUTER_PROVIDER__HOST` | Custom API endpoint for the provider | URL (e.g. `"https://api.openai.com"`) | Provider-specific default |
| `BIOROUTER_PROVIDER__API_KEY` | Authentication key for the provider | API key string | None |

### Lead/worker model configuration

These variables configure a lead/worker model pattern where a powerful lead model handles initial planning and complex reasoning, then switches to a faster or cheaper worker model for execution. The switch happens automatically based on your settings.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_LEAD_MODEL` | **Required to enable lead mode.** Name of the lead model | Model name (e.g. `"gpt-4o"`, `"claude-sonnet-4-20250514"`) | None |
| `BIOROUTER_LEAD_PROVIDER` | Provider for the lead model | [See available providers](../getting-started/choosing-a-model-provider.md) | Falls back to `BIOROUTER_PROVIDER` |
| `BIOROUTER_LEAD_TURNS` | Number of initial turns using the lead model before switching to the worker model | Integer | 3 |
| `BIOROUTER_LEAD_FAILURE_THRESHOLD` | Consecutive failures before fallback to the lead model | Integer | 2 |
| `BIOROUTER_LEAD_FALLBACK_TURNS` | Number of turns to use the lead model in fallback mode | Integer | 2 |

A _turn_ is one complete prompt-response interaction. With the default settings:

- Use the lead model for the first 3 turns.
- Use the worker model starting on the 4th turn.
- Fall back to the lead model if the worker model struggles for 2 consecutive turns.
- Use the lead model for 2 turns, then switch back to the worker model.

The lead model and worker model names are displayed at the start of the biorouter CLI session. If you do not export a `BIOROUTER_MODEL` for your session, the worker model defaults to the `BIOROUTER_MODEL` in your [configuration file](config-file-reference.md).

### Planning mode configuration

These variables control biorouter's planning functionality.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_PLANNER_PROVIDER` | Specifies which provider to use for planning mode | [See available providers](../getting-started/choosing-a-model-provider.md) | Falls back to `BIOROUTER_PROVIDER` |
| `BIOROUTER_PLANNER_MODEL` | Specifies which model to use for planning mode | Model name (e.g. `"gpt-4"`, `"claude-sonnet-4-20250514"`) | Falls back to `BIOROUTER_MODEL` |

### Provider retries

Configurable retry parameters for LLM providers.

#### AWS Bedrock

| Variable | Purpose | Default |
|---------------------|-------------|---------|
| `BEDROCK_MAX_RETRIES` | The max number of retry attempts before giving up | 6 |
| `BEDROCK_INITIAL_RETRY_INTERVAL_MS` | How long to wait (in milliseconds) before the first retry | 2000 |
| `BEDROCK_BACKOFF_MULTIPLIER` | The factor by which the retry interval increases after each attempt | 2 (doubles every time) |
| `BEDROCK_MAX_RETRY_INTERVAL_MS` | The cap on the retry interval in milliseconds | 120000 |

#### Databricks

| Variable | Purpose | Default |
|---------------------|-------------|---------|
| `DATABRICKS_MAX_RETRIES` | The max number of retry attempts before giving up | 3 |
| `DATABRICKS_INITIAL_RETRY_INTERVAL_MS` | How long to wait (in milliseconds) before the first retry | 1000 |
| `DATABRICKS_BACKOFF_MULTIPLIER` | The factor by which the retry interval increases after each attempt | 2 (doubles every time) |
| `DATABRICKS_MAX_RETRY_INTERVAL_MS` | The cap on the retry interval in milliseconds | 30000 |

## Session management

These variables control how biorouter manages conversation sessions and context.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_CONTEXT_STRATEGY` | Controls how biorouter handles context-limit-exceeded situations | `"summarize"`, `"truncate"`, `"clear"`, `"prompt"` | `"prompt"` (interactive), `"summarize"` (headless) |
| `BIOROUTER_MAX_TURNS` | Maximum number of turns allowed without user input | Integer (e.g. 10, 50, 100) | 1000 |
| `BIOROUTER_SUBAGENT_MAX_TURNS` | Sets the maximum turns allowed for a [subagent](../agent-loop/subagents.md) to complete before timeout | Integer (e.g. 25) | 25 |
| `CONTEXT_FILE_NAMES` | Specifies custom filenames for hint/context files | JSON array of strings (e.g. `["CLAUDE.md", ".biorouterhints"]`) | `[".biorouterhints"]` |
| `BIOROUTER_CLI_THEME` | [Theme](../cli/command-reference.md#themes) for CLI response markdown | `"light"`, `"dark"`, `"ansi"` | `"dark"` |
| `BIOROUTER_RANDOM_THINKING_MESSAGES` | Controls whether to show amusing random messages during processing | `"true"`, `"false"` | `"true"` |
| `BIOROUTER_CLI_SHOW_COST` | Toggles display of model cost estimates in CLI output | `"true"`, `"1"` (case insensitive) to enable | false |
| `BIOROUTER_AUTO_COMPACT_THRESHOLD` | Percentage threshold at which biorouter automatically summarizes your session | Float between 0.0 and 1.0 (disabled at 0.0) | 0.8 |

The lost example block for this section covered these scenarios:

- Automatically summarize when the context limit is reached.
- Always prompt the user to choose (the default for interactive mode).
- Set a low turn limit for step-by-step control.
- Set a moderate turn limit for controlled automation.
- Set a reasonable turn limit for production.
- Customize the subagent turn limit.
- Use multiple context files.
- Set the ANSI theme for the session.
- Disable random thinking messages for less distraction.
- Enable model cost display in the CLI.
- Automatically compact sessions when 60% of available tokens are used.

### Model context limit overrides

These variables override the default context window size (token limit) for your models. This is particularly useful when using [LiteLLM proxies](https://docs.litellm.ai/docs/providers/litellm_proxy) or custom models that do not match biorouter's predefined model patterns.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_CONTEXT_LIMIT` | Override context limit for the main model | Integer (number of tokens) | Model-specific default or 128,000 |
| `BIOROUTER_LEAD_CONTEXT_LIMIT` | Override context limit for the lead model in lead/worker mode | Integer (number of tokens) | Falls back to `BIOROUTER_CONTEXT_LIMIT` or model default |
| `BIOROUTER_WORKER_CONTEXT_LIMIT` | Override context limit for the worker model in lead/worker mode | Integer (number of tokens) | Falls back to `BIOROUTER_CONTEXT_LIMIT` or model default |
| `BIOROUTER_PLANNER_CONTEXT_LIMIT` | Override context limit for the planner model | Integer (number of tokens) | Falls back to `BIOROUTER_CONTEXT_LIMIT` or model default |

## Tool configuration

These variables control how biorouter handles [tool execution](../security/permission-modes.md) and tool management.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_MODE` | Controls how biorouter handles tool execution | `"auto"`, `"approve"`, `"chat"`, `"smart_approve"` | `"auto"` |
| `BIOROUTER_TOOLSHIM` | Enables/disables tool call interpretation | `"1"`, `"true"` (case insensitive) to enable | false |
| `BIOROUTER_TOOLSHIM_OLLAMA_MODEL` | Specifies the model for tool call interpretation | Model name (e.g. `llama3.2`, `qwen2.5`) | System default |
| `BIOROUTER_CLI_MIN_PRIORITY` | Controls verbosity of tool output | Float between 0.0 and 1.0 | 0.0 |
| `BIOROUTER_CLI_TOOL_PARAMS_TRUNCATION_MAX_LENGTH` | Maximum length for tool parameter values before truncation in CLI output (not in debug mode) | Integer | 40 |
| `BIOROUTER_DEBUG` | Enables debug mode to show full tool parameters without truncation | `"1"`, `"true"` (case insensitive) to enable | false |
| `BIOROUTER_SEARCH_PATHS` | Additional directories to search for executables when running extensions | JSON array of paths (e.g. `["/usr/local/bin", "~/custom/bin"]`) | System `PATH` only |
| `BIOROUTER_AUTOVIS_CDN` | Makes [Auto Visualiser](../extensions/built-in/README.md) figures reference pinned CDN libraries instead of inlining them, so a stored figure is a few KB rather than megabytes. The desktop app sets it to `1`; a figure still never fetches anything at display time, because the Electron main process inlines the source before the artifact CSP applies — see [how a figure's libraries reach the renderer](../desktop-ui/artifact-cdn-assets.md). | `"1"`, `"true"`, `"yes"` to enable | off outside the desktop app; `1` inside it |
| `BIOROUTER_AUTOVIS_DEBUG` | Dumps each generated figure's HTML to `<cache>/autovisualiser/<name>-<pid>.html` | `"1"`, `"true"` to enable | on in debug builds, otherwise off |

These paths are prepended to the system `PATH` when extensions execute commands, so your custom tools are found without modifying your global `PATH`.

### Foreground shell budget

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_SHELL_FOREGROUND_TIMEOUT_SECS` | Wall-clock budget for a **foreground** Developer `shell` command. When it expires the command's whole process group is killed and the call fails with an error naming the command, its elapsed time, and the `background=true` alternative. | Whole seconds; `0` disables the watchdog | `240` |

This is deliberately shorter than the generic 300-second per-extension MCP timeout, so the shell's own error — which the model can act on — is what fires rather than an opaque transport timeout. It only applies to foreground commands: work started with `background=true` is unbounded and managed with `shell_wait` / `shell_output` / `shell_kill`. Raise it if your agents legitimately run long foreground builds; see [the Developer capability guide](../extensions/built-in/developer.md#foreground-and-background-commands).

### Enhanced code editing

These variables configure AI-powered code editing for the Developer capability's `str_replace` tool. All three variables must be set and non-empty for the feature to activate.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_EDITOR_API_KEY` | API key for the code editing model | API key string | None |
| `BIOROUTER_EDITOR_HOST` | API endpoint for the code editing model | URL (e.g. `"https://api.openai.com/v1"`) | None |
| `BIOROUTER_EDITOR_MODEL` | Model to use for code editing | Model name (e.g. `"gpt-4o"`, `"claude-sonnet-4"`) | None |

This feature works with any OpenAI-compatible API endpoint. The lost example block covered three configurations: OpenAI, Anthropic via an OpenAI-compatible proxy, and a local model.

## Security configuration

These variables control security-related features.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_ALLOWLIST` | Controls which extensions can be loaded | URL for the allowed-extensions list | Unset |
| `BIOROUTER_DISABLE_KEYRING` | Disables the system keyring for secret storage | Set to any value (e.g. `"1"`, `"true"`, `"yes"`) to disable. The actual value does not matter, only whether the variable is set. | Unset (keyring enabled) |

> **Note.** When the keyring is disabled, secrets are stored here:
>
> * macOS/Linux: `~/.config/biorouter/secrets.yaml`
> * Windows: `%APPDATA%\biorouter\config\secrets.yaml`

## Browser access

These variables control what `biorouterd` serves to a web browser. Both are set for you by
[`biorouter serve`](../deployment/browser-access.md) on the daemon it starts — set them yourself
only when you run `biorouterd` directly, or from a service unit where the token has to survive a
restart.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_SERVE_UI` | Directory holding the built browser interface for the daemon to serve. When it is unset the daemon serves no interface and answers only its API, which is what the desktop app wants — Electron loads the interface itself. `biorouter serve` sets it on the daemon it starts, and reads it as well: without `--web-dir`, which takes precedence, a non-blank value is the directory `serve` uses, and one with no `index.html` stops `serve` with an error rather than being skipped. | Path to a directory containing an `index.html` | Unset (no interface served) |
| `BIOROUTER_BROWSER_TOKEN` | The access token a browser presents to be handed the interface. It is exchanged on the first request for a session cookie and authenticates nothing else. When it is unset no token is required, which is only correct for a loopback bind — `biorouter serve` refuses to expose an untokened port. `biorouter serve --token <t>` sets it; otherwise the command generates a new one per launch. | Token string (`biorouter serve` uses 64 hexadecimal characters) | Unset (no token required) |

> **Warning.** `BIOROUTER_BROWSER_TOKEN` is a credential. Put it in a file readable only by the
> service user rather than on a command line, where `ps` shows it to every user on the host.

## Observability

Beyond biorouter's built-in logging system, you can export telemetry to external observability platforms for advanced monitoring, performance analysis, and production insights.

### OpenTelemetry protocol (OTLP)

Configure biorouter to export traces and metrics to any OTLP-compatible observability platform. OTLP is the standard protocol for sending telemetry collected by [OpenTelemetry](https://opentelemetry.io/docs/). When configured, biorouter exports telemetry asynchronously and flushes on exit.

| Variable | Purpose | Values | Default |
|----------|---------|--------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP endpoint URL | URL (e.g. `http://localhost:4318`) | None |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | Export timeout in milliseconds | Integer (ms) | `10000` |

Reach for OTLP when you are:

- Diagnosing slow tool execution or LLM response times.
- Understanding intermittent failures across multiple sessions.
- Monitoring biorouter performance in production or CI/CD environments.
- Tracking usage patterns, costs, and resource consumption over time.
- Setting up alerts for performance degradation or high error rates.

### Langfuse integration

These variables configure the Langfuse integration for observability.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `LANGFUSE_PUBLIC_KEY` | Public key for Langfuse integration | String | None |
| `LANGFUSE_SECRET_KEY` | Secret key for Langfuse integration | String | None |
| `LANGFUSE_URL` | Custom URL for Langfuse service | URL string | Default Langfuse URL |
| `LANGFUSE_INIT_PROJECT_PUBLIC_KEY` | Alternative public key for Langfuse | String | None |
| `LANGFUSE_INIT_PROJECT_SECRET_KEY` | Alternative secret key for Langfuse | String | None |

## Workflow configuration

These variables control workflow discovery and management.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_WORKFLOW_PATH` | Additional directories to search for workflows | Colon-separated paths on Unix, semicolon-separated on Windows | None |
| `BIOROUTER_WORKFLOW_SCAN_WORKING_DIR` | Also list workflows found loose in the working directory root. Off by default: listing would otherwise open every top-level `.yaml`/`.json` in whatever directory a session points at. Running a workflow *by name* searches the working directory either way | `"1"`, `"true"`, `"yes"`, `"on"` to enable | false |
| `BIOROUTER_WORKFLOW_GITHUB_REPO` | GitHub repository to search for workflows | Format: `"owner/repo"` (e.g. `"BaranziniLab/biorouter-workflows"`) | None |
| `BIOROUTER_WORKFLOW_RETRY_TIMEOUT_SECONDS` | Global timeout for workflow success check commands | Integer (seconds) | Workflow-specific default |
| `BIOROUTER_WORKFLOW_ON_FAILURE_TIMEOUT_SECONDS` | Global timeout for workflow `on_failure` commands | Integer (seconds) | Workflow-specific default |

## Experimental features

These variables enable features that are in active development. They may change or be removed in future releases — use them with caution in production environments.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `ALPHA_FEATURES` | Enables experimental alpha features&mdash;check the feature docs to see if this flag is required | `"true"`, `"1"` (case insensitive) to enable | false |

```bash
# Enable for a single session
ALPHA_FEATURES=true biorouter session
```

## Development and testing

These variables are primarily used for developing, testing, and debugging biorouter itself.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_PATH_ROOT` | Override the root directory for all biorouter data, config, and state files | Absolute path to a directory | Platform-specific defaults |

Default locations:

- macOS: `~/Library/Application Support/Block/biorouter/`
- Linux: `~/.local/share/biorouter/`
- Windows: `%APPDATA%\Block\biorouter\`

When set, biorouter creates `config/`, `data/`, and `state/` subdirectories under the specified path. This is useful for isolating test environments, running multiple configurations, or CI/CD pipelines.

```bash
# Isolated environment for a single command
BIOROUTER_PATH_ROOT="/tmp/biorouter-isolated" biorouter run --workflow my-workflow.yaml

# CI/CD usage
BIOROUTER_PATH_ROOT="$(mktemp -d)" biorouter run --workflow integration-test.yaml

# Use with developer tools
BIOROUTER_PATH_ROOT="/tmp/biorouter-test" ./scripts/biorouter-db-helper.sh status
```

## Variables controlled by biorouter

biorouter sets these automatically during command execution.

| Variable | Purpose | Values | Default |
|----------|---------|---------|---------|
| `BIOROUTER_TERMINAL` | Indicates that a command is being executed by biorouter, enabling customized shell behaviour | `"1"` when set | Unset |

### Customizing shell behaviour

Sometimes you want biorouter to use different commands or have different shell behaviour than your normal terminal usage. For example, you might want biorouter to use a different tool, prevent biorouter from running `git commit`, or block long-running development servers that could hang the AI agent. This is most useful with the biorouter CLI, where shell commands are executed directly in your terminal environment.

How it works:

1. When biorouter runs commands, `BIOROUTER_TERMINAL` is automatically set to `"1"`.
2. Your shell configuration detects this and changes behaviour, while your normal terminal usage stays unchanged.

```bash
# In ~/.zshenv (for zsh users) or ~/.bashrc (for bash users)

# Block git commit when run by biorouter
if [[ -n "$BIOROUTER_TERMINAL" ]]; then
  git() {
    if [[ "$1" == "commit" ]]; then
      echo "❌ BLOCKED: git commit is not allowed when run by biorouter"
      return 1
    fi
    command git "$@"
  }
fi
```

```bash
# Guide biorouter toward better tool choices
if [[ -n "$BIOROUTER_TERMINAL" ]]; then
  alias find="echo 'Use rg instead: rg --files | rg <pattern> for filenames, or rg <pattern> for content search'"
fi
```

## Variables documented elsewhere

Some user-facing variables are not listed above and are documented on their own feature pages:

- llama.cpp sidecar variables such as `BIOROUTER_LLAMACPP_BIN`, `LLAMACPP_EXTRA_ARGS`, and `LLAMACPP_EXTERNAL_HOST` — see the [Llama Server model catalog QA checklist](../providers/llama-server/model-catalog-qa-checklist.md).
- The auto-update feed override `BIOROUTER_UPDATE_FEED_URL` — see the [auto-update test checklist](../releases/auto-update-test-checklist.md).

## Precedence and caveats

- Environment variables take precedence over configuration files.
- For security-sensitive variables such as API keys, prefer the system keyring over environment variables.
- Some variables require restarting biorouter to take effect.
- In planning mode, if planner-specific variables are not set, biorouter falls back to the main model configuration.

## Related documentation

- [Configuration file reference](config-file-reference.md) — the persistent YAML form of most settings on this page, and the file these variables override.
- [Secret storage](../security/secret-storage.md) — why `BIOROUTER_DISABLE_KEYRING` exists and what changes when you set it.
- [Permission modes](../security/permission-modes.md) — what each `BIOROUTER_MODE` value permits.
- [biorouter CLI command reference](../cli/command-reference.md) — the commands these variables modify, including themes and slash commands.
- [Reaching Biorouter from a browser](../deployment/browser-access.md) — what `BIOROUTER_SERVE_UI` and `BIOROUTER_BROWSER_TOKEN` do in practice, and the command that sets them.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the provider names and model IDs the provider variables expect.
