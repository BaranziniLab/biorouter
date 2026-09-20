# Configuration file reference

> **What this is.** The reference for biorouter's YAML configuration files — where they live, every global setting they accept, how extensions and search paths are declared, and how config values interact with environment variables.
> **Status:** Current.
> **Audience:** end users

biorouter reads its persistent settings from YAML files in your user config directory. The primary file is:

* macOS/Linux: `~/.config/biorouter/config.yaml`
* Windows: `%APPDATA%\biorouter\config\config.yaml`

These files let you set default behaviours, configure language models, set tool permissions, and manage extensions. Many of the same settings can be supplied as [environment variables](environment-variables.md) instead; the config files are the persistent form of the same surface.

## Configuration files

- **`config.yaml`** — provider, model, extensions, and general settings
- **`permission.yaml`** — tool permission levels configured via `biorouter configure`
- **`secrets.yaml`** — API keys and secrets (only when the keyring is disabled)
- **`permissions/tool_permissions.json`** — runtime permission decisions (auto-managed)

## Global settings

Set these at the root level of `config.yaml`.

| Setting | Purpose | Values | Default | Required |
|---------|---------|---------|---------|-----------|
| `BIOROUTER_PROVIDER` | Primary [large language model (LLM) provider](../getting-started/choosing-a-model-provider.md) | `"anthropic"`, `"openai"`, etc. | None | Yes |
| `BIOROUTER_MODEL` | Default model to use | Model name (e.g. `"claude-4.5-sonnet"`, `"gpt-4"`) | None | Yes |
| `BIOROUTER_TEMPERATURE` | Model response randomness | Float between 0.0 and 1.0 | Model-specific | No |
| `BIOROUTER_MODE` | [Tool execution behaviour](../security/permission-modes.md) | `"auto"`, `"approve"`, `"chat"`, `"smart_approve"` | `"auto"` | No |
| `BIOROUTER_MAX_TURNS` | Maximum number of turns allowed without user input | Integer (e.g. 10, 50, 100) | 100 | No |
| `BIOROUTER_LEAD_PROVIDER` | Provider for the lead model in [lead/worker mode](environment-variables.md#leadworker-model-configuration) | Same as `BIOROUTER_PROVIDER` options | Falls back to `BIOROUTER_PROVIDER` | No |
| `BIOROUTER_LEAD_MODEL` | Lead model for lead/worker mode | Model name | None | No |
| `BIOROUTER_PLANNER_PROVIDER` | Provider for planning mode | Same as `BIOROUTER_PROVIDER` options | Falls back to `BIOROUTER_PROVIDER` | No |
| `BIOROUTER_PLANNER_MODEL` | Model for planning mode | Model name | Falls back to `BIOROUTER_MODEL` | No |
| `BIOROUTER_TOOLSHIM` | Enable tool interpretation | true/false | false | No |
| `BIOROUTER_TOOLSHIM_OLLAMA_MODEL` | Model for tool interpretation | Model name (e.g. `"llama3.2"`) | System default | No |
| `BIOROUTER_CLI_MIN_PRIORITY` | Tool output verbosity | Float between 0.0 and 1.0 | 0.0 | No |
| `BIOROUTER_CLI_THEME` | [Theme](../cli/command-reference.md#themes) for CLI response markdown | `"light"`, `"dark"`, `"ansi"` | `"dark"` | No |
| `BIOROUTER_CLI_SHOW_COST` | Show estimated cost for token use in the CLI | true/false | false | No |
| `BIOROUTER_ALLOWLIST` | URL for allowed extensions | Valid URL | None | No |
| `BIOROUTER_WORKFLOW_GITHUB_REPO` | GitHub repository for workflows | Format: `"org/repo"` | None | No |
| `BIOROUTER_AUTO_COMPACT_THRESHOLD` | Percentage threshold at which biorouter automatically summarizes your session | Float between 0.0 and 1.0 (disabled at 0.0) | 0.8 | No |
| `otel_exporter_otlp_endpoint` | OpenTelemetry protocol (OTLP) endpoint URL for [observability](environment-variables.md#opentelemetry-protocol-otlp) | URL (e.g. `http://localhost:4318`) | None | No |
| `otel_exporter_otlp_timeout` | Export timeout in milliseconds for [observability](environment-variables.md#opentelemetry-protocol-otlp) | Integer (ms) | 10000 | No |
| `SECURITY_COMMAND_POLICY` | Governs the auditable command policy engine. `off` disables it, leaving only the always-on catastrophic-command denylist | `"enforce"`, `"off"` | `"enforce"` | No |

## Experimental features

These settings enable features that are in active development. They may change or be removed in future releases.

| Setting | Purpose | Values | Default | Required |
|---------|---------|---------|---------|-----------|
| `ALPHA_FEATURES` | Enables access to experimental alpha features&mdash;check the feature docs to see if this flag is required | true/false | false | No |

Additional [environment variables](environment-variables.md) may also be supported in `config.yaml`.

## Example configuration

A basic `config.yaml`:

```yaml
# Model Configuration
BIOROUTER_PROVIDER: "anthropic"
BIOROUTER_MODEL: "claude-4.5-sonnet"
BIOROUTER_TEMPERATURE: 0.7

# Planning Configuration
BIOROUTER_PLANNER_PROVIDER: "openai"
BIOROUTER_PLANNER_MODEL: "gpt-4"

# Tool Configuration
BIOROUTER_MODE: "smart_approve"
BIOROUTER_TOOLSHIM: true
BIOROUTER_CLI_MIN_PRIORITY: 0.2

# Workflow Configuration
BIOROUTER_WORKFLOW_GITHUB_REPO: "BaranziniLab/biorouter-workflows"

# Search Path Configuration
BIOROUTER_SEARCH_PATHS:
  - "/usr/local/bin"
  - "~/custom/tools"
  - "/opt/homebrew/bin"

# Observability (OpenTelemetry)
otel_exporter_otlp_endpoint: "http://localhost:4318"
otel_exporter_otlp_timeout: 20000

# Security Configuration
SECURITY_COMMAND_POLICY: enforce

# Extensions Configuration
extensions:
  developer:
    bundled: true
    enabled: true
    name: developer
    timeout: 300
    type: builtin
  
  memory:
    bundled: true
    enabled: true
    name: memory
    timeout: 300
    type: builtin
```

## Extensions configuration

Extensions are configured under the `extensions` key. Each extension accepts the following settings:

```yaml
extensions:
  extension_name:
    bundled: true/false       # Whether it's included with biorouter
    display_name: "Name"      # Human-readable name (optional)
    enabled: true/false       # Whether the extension is active
    name: "extension_name"    # Internal name
    timeout: 300              # Operation timeout in seconds
    type: "builtin"/"stdio"   # Extension type
    
    # Additional settings for stdio extensions:
    cmd: "command"            # Command to execute
    args: ["arg1", "arg2"]    # Command arguments
    description: "text"       # Extension description
    env_keys: []              # Required environment variables
    envs: {}                  # Environment values
```

## Search path configuration

Extensions may need to execute external commands or tools. By default, biorouter uses your system's `PATH` environment variable. Add further search directories in your config file:

```yaml
BIOROUTER_SEARCH_PATHS:
  - "/usr/local/bin"
  - "~/custom/tools"
  - "/opt/homebrew/bin"
```

These paths are prepended to the system `PATH` when running extension commands, so your custom tools are found without modifying your global `PATH`.

## Workflow command configuration

You can optionally define custom [slash commands](../cli/command-reference.md#slash-commands) that run workflows you create. List the command (without the leading `/`) along with the path to the workflow:

```yaml
slash_commands:
  - command: "run-tests"
    workflow_path: "/path/to/workflow.yaml"
  - command: "daily-standup"
    workflow_path: "/Users/me/.local/share/biorouter/workflows/standup.yaml"
```

## Configuration priority

Settings are applied in the following order of precedence:

1. Environment variables (highest priority)
2. Config file settings
3. Default values (lowest priority)

> **Note.** Enterprise deployments add a fourth, admin-owned tier above these. [Managed policy](../security/managed-policy.md) documents its precedence chain for permissions and hooks as `Default (built-in) < User (global config) < Project (opt-in) < Managed (admin)`. The two orderings on this page and that one have not been reconciled; where an admin-installed managed policy file is present, treat the managed policy page as authoritative.

## Security considerations

- Avoid storing sensitive information (API keys, tokens) in the config file.
- Use the system keyring for storing secrets.
- If the keyring is disabled, secrets are stored in a separate `secrets.yaml` file.

## Updating configuration

Changes to config files require restarting biorouter to take effect. Verify your current configuration with:

```bash
biorouter info -v
```

This shows all active settings and their current values.

## Related documentation

- [Environment variables](environment-variables.md) — the per-invocation form of most settings on this page, and the only home for several variables that have no `config.yaml` equivalent.
- [Managed policy](../security/managed-policy.md) — the admin-owned tier that overrides user and project config for permissions and hooks.
- [Secret storage](../security/secret-storage.md) — how the keyring is used and what happens when it is disabled.
- [Permission modes](../security/permission-modes.md) — what the `BIOROUTER_MODE` values actually allow the agent to do.
- [Extensions, skills, and MCP agents](../extensions/extensions-and-skills-guide.md) — background on the extensions you declare under the `extensions` key.
