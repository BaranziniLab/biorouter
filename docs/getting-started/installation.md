# Installation and setup

> **What this is.** The step-by-step guide to installing Biorouter, connecting an LLM provider (including UCSF institutional options), verifying the setup, adding extensions and remote MCP agents, and finding config and log locations.
> **Status:** Current.
> **Audience:** end users

Biorouter is distributed as a desktop app with a bundled `biorouter` command-line tool. This page is the long form of the install path; if you only want to be running in five minutes, follow [biorouter in 5 minutes](quickstart.md) instead — it covers the same install and provider steps in condensed form, and this page adds the institutional provider options, MCP agent configuration, and file locations that the quickstart omits.

MCP (Model Context Protocol) is the open standard Biorouter uses to connect to pluggable tool servers, which it calls *extensions*.

- **GitHub:** [BaranziniLab/biorouter](https://github.com/BaranziniLab/biorouter)
- **Latest releases:** [Biorouter releases](https://github.com/BaranziniLab/biorouter/releases)

Always install the newest release for the latest features and bug fixes.

## Install Biorouter

### macOS desktop app (recommended)

1. Go to the [releases page](https://github.com/BaranziniLab/biorouter/releases) and download the latest macOS release (`.dmg` or `.zip`).
2. Open the downloaded file and move Biorouter to your Applications folder.
3. Launch Biorouter from Applications.

> **Note.** If you see a security warning on an Apple Silicon Mac, ensure `~/.config` has read and write permissions. Biorouter uses this directory for its configuration and log files.

### macOS command-line tool

On macOS the `biorouter` command-line tool is bundled inside the desktop app — there is no separate CLI download. After installing the app (above), add it to your `PATH`:

1. Accept the in-app **Install Biorouter CLI** prompt, or run:

   ```sh
   biorouter setup-path
   ```

   This symlinks the bundled `biorouter` binary onto your `PATH`.

2. Configure a provider and extensions from the terminal:

   ```sh
   biorouter configure
   ```

To upgrade, re-download the latest app from the [releases page](https://github.com/BaranziniLab/biorouter/releases) and run `biorouter setup-path` again to point at the new binary.

### Linux

Linux builds are published with each release. Install the desktop app with the `biorouter_*_amd64.deb` (Debian/Ubuntu) or `Biorouter-*-1.x86_64.rpm` (Fedora/RHEL) package, or install the standalone headless CLI with the `biorouter-cli_*_amd64.deb` / `biorouter-cli-*-1.x86_64.rpm` package, which provides `biorouter` and `biorouterd` without the desktop app. Download them from the [releases page](https://github.com/BaranziniLab/biorouter/releases) or the [download page](http://biorouter.ucsf.edu/download).

### Windows

Windows builds are published with each release. Download `Biorouter-win32-x64-*.zip` from the [releases page](https://github.com/BaranziniLab/biorouter/releases) or the [download page](http://biorouter.ucsf.edu/download), unzip it, and run `Biorouter.exe`. The `biorouter` CLI is bundled inside the app — accept the in-app **Install Biorouter CLI** prompt, or run `biorouter setup-path`, to add it to your `PATH`.

Windows has no symlinks, so this *copies* the CLI into `%LOCALAPPDATA%\Biorouter\bin` and records the folder it came from beside the copy. To upgrade, unzip the new release and run `biorouter setup-path` again from inside it (`<application folder>\resources\bin\biorouter.exe setup-path`) — that refreshes both the copy and the record, so `biorouter serve` keeps finding the daemon and the interface.

## Configure an LLM provider

Biorouter needs an LLM provider to function. On first launch, you will be prompted to configure one.

### Recommended options for UCSF users

**Option A: Azure OpenAI (UCSF ChatGPT)**

- Access is managed through your UCSF institutional account.
- Select "Azure OpenAI" in the provider setup and follow the credential prompts.
- This provider routes through UCSF's Azure tenant — no separate API key required for most users.

**Option B: Amazon Bedrock (UCSF-hosted Anthropic)**

- Uses AWS SSO with your UCSF AWS account.
- Run `aws sso login --profile <your-profile>` before launching Biorouter.
- Select "Amazon Bedrock" and configure `AWS_PROFILE` and `AWS_REGION`.

**Option C: local / air-gapped (Ollama)**

- Install [Ollama](https://ollama.com).
- Pull a model: `ollama pull qwen3`.
- Select "Ollama" in Biorouter — no API key needed.
- Data stays entirely on your device.

> **Note.** The desktop app also ships a bundled **Llama Server** (`llamacpp`) local provider, which the app's own provider grid ranks first under Local Models — ahead of Ollama. It is not documented in the options above or in [Choosing a model provider](choosing-a-model-provider.md); use the Settings > Models panel in the app for its current setup steps.

### Commercial cloud providers

For direct access using your own API key:

| Provider | Where to get a key |
|---|---|
| Anthropic | [console.anthropic.com](https://console.anthropic.com) |
| OpenAI | [platform.openai.com/api-keys](https://platform.openai.com/api-keys) |
| Google Gemini | [aistudio.google.com/app/apikey](https://aistudio.google.com/app/apikey) |
| X.AI (Grok) | [console.x.ai](https://console.x.ai) |
| Venice AI | [venice.ai](https://venice.ai) |
| OpenRouter | [openrouter.ai/keys](https://openrouter.ai/keys) |

**Desktop setup:**

1. On the welcome screen, enter your API key in the Quick Setup panel.
2. Biorouter will test the key and prompt you to choose a model.
3. You're ready to start a chat.

**CLI setup:**

```sh
biorouter configure
# Select "Configure Providers"
# Choose your provider and enter the API key when prompted
```

### Switch or update your provider

**Desktop:** Settings > Models > select a provider card > Configure or Launch.

**CLI:**

```sh
biorouter configure
# Select "Configure Providers"
```

## Verify the setup

After configuration, start a chat and send a test message.

**Desktop:** Type a message in the chat input and press Enter.

**CLI:**

```sh
biorouter session
> Hello, can you confirm you're working?
```

If Biorouter responds, the setup is complete.

## Enable or add extensions (optional)

Extensions give Biorouter access to tools like file operations, web search, databases, and more.

The **Developer** capability is enabled by default and provides file, shell, editing, and code-analysis tools.

### Enable built-in capabilities

**Desktop:** Sidebar > Extensions > toggle the extension on.

**CLI:**

```sh
biorouter configure
# Select "Add Extension" > "Built-in Extension"
```

Available built-in capabilities: Developer, Computer Controller, Memory, Auto Visualiser, Knowledge, Agent Drafter, Code Execution, Extension Manager, Skills, Todo, and Workspace Control are enabled by default. Chat Recall ships disabled by default. You can change any of these choices in Settings → Chat → Capabilities.

### Add external MCP servers

Any MCP-compatible server can be added as an extension.

**Desktop:** Sidebar > Extensions > Add custom extension > enter the command and configuration.

**CLI example (GitHub MCP server):**

```sh
biorouter configure
# Select "Add Extension" > "Command-line Extension"
# Name: GitHub
# Command: npx -y @modelcontextprotocol/server-github
# Environment variable: GITHUB_PERSONAL_ACCESS_TOKEN = <your token>
```

**Manual config** (`~/.config/biorouter/config.yaml`):

```yaml
extensions:
  github:
    name: GitHub
    cmd: npx
    args: ["-y", "@modelcontextprotocol/server-github"]
    enabled: true
    envs:
      GITHUB_PERSONAL_ACCESS_TOKEN: "<your_token>"
    type: stdio
    timeout: 300
```

## Set up MCP agents for specialized tasks (optional)

Biorouter can connect to remote MCP agents that provide specialized research capabilities (database access, literature retrieval, bioinformatics tools, etc.).

To connect a remote MCP agent over HTTP:

```yaml
# In ~/.config/biorouter/config.yaml
extensions:
  my-research-agent:
    name: My Research Agent
    url: https://my-agent.example.com/mcp
    type: streamable_http
    enabled: true
    timeout: 300
```

Or via CLI:

```sh
biorouter session --with-streamable-http-extension "https://my-agent.example.com/mcp"
```

## Where configuration is stored

The main configuration file is at:

```text
~/.config/biorouter/config.yaml                  (macOS / Linux)
%APPDATA%\biorouter\config\config.yaml           (Windows)
```

This file stores provider settings, extension configurations, and model preferences. It is shared between the Desktop app and the CLI.

> **Warning.** API keys are **not** stored in `config.yaml`. Secrets live in your operating system's native credential store — the macOS Keychain, the Windows Credential Manager, or the Linux Secret Service. See [Secret storage](../security/secret-storage.md) for how that works and how to opt out.

**Electron app state** (Desktop only) is stored at:

```text
~/Library/Application Support/Biorouter/     (macOS)
```

For the full set of keys accepted in `config.yaml`, see the [configuration file reference](../configuration/config-file-reference.md).

## Update Biorouter

**Desktop:** Biorouter checks for updates automatically. You can also manually download the latest release from the [releases page](https://github.com/BaranziniLab/biorouter/releases).

**CLI:** The `biorouter` CLI is bundled with the desktop app. To upgrade it on macOS or Windows, re-download the latest app from the [releases page](https://github.com/BaranziniLab/biorouter/releases) and run `biorouter setup-path` to point your `PATH` at the new binary. On Linux, upgrade the headless `biorouter-cli` package instead (`sudo apt install ./biorouter-cli_*.deb` or `sudo dnf install ./biorouter-cli-*.rpm`).

## Troubleshoot a failed install

These are the install-time problems only. For the full catalogue, see [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md).

**App shows no window on launch (macOS M3):**

Ensure `~/.config` has read and write permissions.

```sh
chmod 755 ~/.config
```

**Extension fails to activate:**

- Check that the required runtime is installed (Node.js for `npx`, Python/`uvx` for Python extensions).
- In corporate or air-gapped networks, extensions may need pre-installation rather than on-demand downloads.

**API key not working:**

- Verify the key is correct and has not expired.
- Check that the provider account has sufficient credits or quota.
- Try setting the key as an environment variable instead of via the keyring:

  ```sh
  export ANTHROPIC_API_KEY=your_key_here
  biorouter configure
  ```

**Log files:**

Logs are stored in `~/.config/biorouter/logs/`. Review them for detailed error messages.

## Related documentation

- [biorouter in 5 minutes](quickstart.md) — the condensed version of this install and provider path, ending in a first research task.
- [Choosing a model provider](choosing-a-model-provider.md) — per-provider credentials, default models, and switching providers.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md) — the full troubleshooting catalogue this page's last section samples from.
- [Secret storage](../security/secret-storage.md) — where API keys actually live and how the OS credential store is used.
- [Extensions, skills, and MCP agents](../extensions/extensions-and-skills-guide.md) — how the extension mechanisms you configured above relate to one another.
