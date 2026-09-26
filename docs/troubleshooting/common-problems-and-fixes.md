# Common problems and fixes

> **What this is.** A reference of the problems users hit most often with biorouter, each with the symptom, the cause where it is known, and the fix. Roughly twenty independent entries, grouped by the part of the system they affect.
> **Status:** Current, with one exception noted in place: the stack trace under [API errors](#api-errors) comes from an older Python agent and cannot be produced by Biorouter.
> **Audience:** end users

biorouter, like any system, may run into occasional issues. Use the contents below to jump to the entry that matches your symptom; entries are independent, so you do not need to read the page in order. If nothing here matches, generate a diagnostics bundle and open an issue — see [Getting further help](#getting-further-help).

> **Note.** The [GitHub issues page][github-issues] is here to help. For the fastest support, generate a [diagnostic report](diagnostics-and-bug-reports.md) first — it helps maintainers understand your setup quickly.

## Contents

- [Expected behaviour that can look like a problem](#expected-behaviour-that-can-look-like-a-problem)
- [Sessions and long-running work](#sessions-and-long-running-work)
- [Providers and models](#providers-and-models)
- [Secrets and the system keyring](#secrets-and-the-system-keyring)
- [Extensions and package runners](#extensions-and-package-runners)
- [System permissions](#system-permissions)
- [Uninstalling biorouter or removing cached data](#uninstalling-biorouter-or-removing-cached-data)
- [Getting further help](#getting-further-help)

## Expected behaviour that can look like a problem

Two behaviours are reported as bugs often enough to belong here, but both are intended.

### biorouter edits files

biorouter can and will edit files as part of its workflow. To avoid losing personal changes, use version control to stage your personal edits. Leave biorouter edits unstaged until reviewed. Consider separate commits for biorouter's edits so you can easily revert them if needed.

### New workflow warning

The first time you run a given workflow in biorouter Desktop, you'll see a `New Workflow Warning` dialog that allows you to review the workflow's title, description, and instructions. If you trust the workflow, click `Trust and Execute` to continue. You won't be prompted again for the same workflow unless it changes.

This warning helps protect against inadvertently executing potentially harmful workflow code. For how workflows are defined and shared, see the [workflows guide](../workflows/README.md).

## Sessions and long-running work

### Interrupting biorouter

If biorouter is heading in the wrong direction or gets stuck, you can interrupt it to correct its actions or provide additional information. The interrupt controls available inside a running session are listed under [interactive session features](../cli/command-reference.md#interactive-session-features).

### Stuck in a loop or unresponsive

In rare cases, biorouter may enter a "doom spiral" or become unresponsive during a long chat. This is often resolved by ending the current chat and starting a new chat.

1. Hold down `Ctrl+C` to cancel.
2. Start a new chat:

   ```sh
   biorouter session
   ```

> **Note.** For particularly large or complex tasks, break them into smaller chats.

### Preventing long-running commands

If you use biorouter CLI and work with web development projects, you may encounter commands that cause biorouter to hang indefinitely. Commands like `npm run dev`, `python -m http.server`, or `webpack serve` start development servers that never exit on their own.

You can prevent these issues by customizing your shell to handle these commands differently when biorouter runs them. See [customizing shell behaviour](../configuration/environment-variables.md#customizing-shell-behaviour) for details on using the `BIOROUTER_TERMINAL` environment variable.

### Context length exceeded error

This error occurs when the input provided to biorouter exceeds the maximum token limit of the LLM being used. To resolve this, try breaking down your input into smaller parts. You can also use [`.biorouterhints`][biorouterhints] as a way to provide biorouter with detailed context, and use message queues in biorouter Desktop.

## Providers and models

### Using the Ollama provider

Ollama provides local LLMs, which means you must first [download Ollama and run a model](../getting-started/choosing-a-model-provider.md#ollama-local) before attempting to use this provider with biorouter. If you do not have the model downloaded, you'll run into the following error:

```text
ExecutionError("error sending request for url (http://localhost:11434/v1/chat/completions)")
```

Another thing to note is that the DeepSeek models do not support tool calling, so all biorouter [extensions must be disabled](../extensions/extensions-and-skills-guide.md#managing-extensions) to use one of these models. Unfortunately, without the use of tools, there is not much biorouter will be able to do autonomously if using DeepSeek. However, Ollama's other models such as `qwen2.5` do support tool calling and can be used with biorouter extensions.

> **Note.** The provider guide gives `qwen3` as biorouter's current default Ollama model. See [Ollama (local)](../getting-started/choosing-a-model-provider.md#ollama-local) for the models it lists today.

### Connection error with the Ollama provider on WSL

If you encounter an error like this when setting up Ollama as the provider in biorouter:

```text
Execution error: error sending request for url (http://localhost:11434/v1/chat/completions)
```

This likely means that the local host address is not accessible from WSL.

1. Check if the service is running:

   ```bash
   curl http://localhost:11434/api/tags
   ```

   If you receive a `failed to connect` error, it’s possible that WSL is using a different IP for localhost. In that case, run the following command to find the correct IP address for WSL:

   ```bash
   ip route show | grep -i default | awk '{ print $3 }'
   ```

2. Once you get the IP address, use it in your biorouter configuration instead of localhost. For example:

   ```text
   http://172.24.80.1:11434
   ```

If you still encounter a `failed to connect` error, you can try using WSL's [mirrored networking mode](https://learn.microsoft.com/en-us/windows/wsl/networking#mirrored-mode-networking) setting if you using Windows 11 22H2 or higher.

### Handling rate limit errors

biorouter may encounter a `429 error` (rate limit exceeded) when interacting with LLM providers. The recommended solution is to use a provider that provides built-in rate limiting. For the retry and backoff settings biorouter exposes per provider, see [provider retries](../configuration/environment-variables.md#provider-retries).

### API errors

You may run into an error like the one below when there are issues with your LLM API tokens, such as running out of credits or incorrect configuration:

```text
Traceback (most recent call last):
  File "/Users/admin/.local/pipx/venvs/example-agent/lib/python3.13/site-packages/exchange/providers/utils.py",
line 30, in raise_for_status
    response.raise_for_status()
    ~~~~~~~~~~~~~~~~~~~~~~~~~^^
  File "/Users/admin/.local/pipx/venvs/example-agent/lib/python3.13/site-packages/httpx/_models.py",
line 829, in raise_for_status
    raise HTTPStatusError(message, request=request, response=self)
httpx.HTTPStatusError: Client error '404 Not Found' for url
'https://api.openai.com/v1/chat/completions'

...
```

> **Warning.** This Python traceback is an old example from a Python agent installed with pipx, not output from Biorouter. Biorouter is a Rust program and cannot emit a `pipx`/`httpx` trace, so do not try to match your error against it. The surrounding advice still applies to the equivalent credential and credit errors in Biorouter.

This error typically occurs when LLM API credits are exhausted or your API key is invalid. To resolve this issue:

1. Check your API credits:
    - Log into your LLM provider's dashboard.
    - Verify that you have enough credits. If not, refill them.
2. Verify your API key:
    - Run the following command to reconfigure your API key:

    ```sh
    biorouter configure
    ```

For detailed steps on updating your LLM provider, refer to the [installation guide][installation].

### GitHub Copilot provider configuration

If you encounter errors when configuring GitHub Copilot as your provider, try these workarounds for common scenarios.

#### OAuth error with lead/worker models

If the [lead/worker model](../configuration/environment-variables.md#leadworker-model-configuration) feature is configured in your environment, you might see the following error during GitHub Copilot setup. This feature conflicts with the OAuth flow to connect to the provider.

```text
Failed to authenticate: Execution error: OAuth configuration not supported by this provider
```

To resolve:

1. Temporarily comment out or remove lead/worker model variables from the main config file (`~/.config/biorouter/config.yaml`):

   ```yaml
   # BIOROUTER_LEAD_MODEL: your-model
   # BIOROUTER_WORKER_MODEL: your-model
   ```

2. Run `biorouter configure` again to set up GitHub Copilot.
3. Complete the OAuth authentication flow.
4. Re-enable your lead/worker model settings as needed.

#### Container and keyring issues

If you're running biorouter in Docker containers or Linux environments without keyring support, authentication may fail with keyring errors like:

```text
Failed to save token: Failed to access keyring: Platform secure storage failure: DBus error: Using X11 for dbus-daemon autolaunch was disabled at compile time
```

biorouter tries to use the system keyring (which requires DBus and X11) to securely store your GitHub token, but these aren't available in containerized or headless environments.

To resolve, use the `BIOROUTER_DISABLE_KEYRING` environment variable to tell biorouter to store secrets in files instead. This example sets the variable only while executing the `biorouter configure` command:

```bash
BIOROUTER_DISABLE_KEYRING=1 biorouter configure
```

See [Keychain and keyring errors](#keychain-and-keyring-errors) for more details on keyring alternatives.

## Secrets and the system keyring

### Keychain and keyring errors

biorouter tries to use the system keyring to store secrets. In environments where there is no keyring support, you may see an error like:

```text
Error Failed to access secure storage (keyring): Platform secure storage failure: DBus error: The name org.freedesktop.secrets was not provided by any .service files
Please check your system keychain and run 'biorouter configure' again.
If your system is unable to use the keyring, please try setting secret key(s) via environment variables.
```

In this case, you will need to set your provider specific environment variable(s), which can be found at [Supported LLM providers][configure-llm-provider].

You can set them either by doing:

- `export GOOGLE_API_KEY=$YOUR_KEY_HERE` - for the duration of your session
- in your `~/.bashrc` or `~/.zshrc` - (or equivalents) so it persists on new shell each new session

Then select the `No` option when prompted to save the value to your keyring.

```text
$ biorouter configure

Welcome to biorouter! Let's get you set up with a provider.
  you can rerun this command later to update your configuration

┌   biorouter-configure
│
◇  Which model provider should we use?
│  Google Gemini
│
◇  GOOGLE_API_KEY is set via environment variable
│
◇  Would you like to save this value to your keyring?
│  No
│
◇  Enter a model from that provider:
│  gemini-2.0-flash-exp
```

You may also use the `BIOROUTER_DISABLE_KEYRING` environment variable, which disables the system keyring for secret storage. Set to any value (e.g., "1", "true", "yes"), to disable. The actual value doesn't matter, only whether the variable is set.

When the keyring is disabled, secrets are stored here:

- macOS/Linux: `~/.config/biorouter/secrets.yaml`
- Windows: `%APPDATA%\Biorouter\config\secrets.yaml`

## Extensions and package runners

### Start with `biorouter doctor`

Before working through the fixes below by hand, run:

```bash
biorouter doctor
```

It prints one status line per prerequisite (Git, uv, Python 3, Node.js, AWS CLI, `llama-server`, the Rust toolchain) and says whether `biorouter` is on your PATH, so it names the missing piece directly. To hand a failing prerequisite to Biorouter itself, run `biorouter doctor --fix <dependency>`, for example `biorouter doctor --fix uv`, or leave the name off to take the first missing required one.

### Hermit errors

If you see an issue installing an extension in the app that says "hermit:fatal", you may need to reset your hermit cache. biorouter uses a copy of hermit to ensure npx and uvx are consistently available. If you have already used an older version of hermit, you may need to clean up the cache — on Mac this cache is at

```bash
sudo rm -rf ~/Library/Caches/hermit
```

### Package runners

Many of the external extensions require a package runner. For example, if you run into an error like this one:

```text
Failed to start extension `{extension name}`: Could not run extension command (`{extension command}`): No such file or directory (os error 2)
Please check extension configuration for {extension name}.
```

... it signals that the extension may not have been installed and you need the package runner in order to do so.

An example is the GitHub extension whose command is `npx -y @modelcontextprotocol/server-github`. You'd need [Node.js](https://nodejs.org/) installed on your system to run this command, as it uses `npx`.

### Node.js extensions not activating on Windows

If you encounter the error `Node.js installer script not found` when trying to activate Node.js-based extensions on Windows, this is likely due to biorouter not finding Node.js in the expected system path.

Symptoms:

- Node.js is installed and working (verified with `node -v` and `npm -v`).
- Other extensions (like Python-based ones) work fine.
- The error occurs specifically when activating Node.js extensions.

This issue typically occurs when Node.js is installed in a non-standard location. biorouter expects to find Node.js in `C:\Program Files\nodejs\`, but it may be installed elsewhere (e.g., `D:\Program Files\nodejs\`). To fix it:

1. **Check your Node.js installation path:**

   ```powershell
   where.exe node
   ```

2. **If Node.js is not in `C:\Program Files\nodejs\`, create a symbolic link:**
   - Open PowerShell as Administrator.
   - Create a symbolic link to redirect biorouter to your actual Node.js installation:

   ```powershell
   mklink /D "C:\Program Files\nodejs" "D:\Program Files\nodejs"
   ```

   (Replace `D:\Program Files\nodejs` with your actual Node.js installation path.)

3. **Restart biorouter** and try activating the extension again.

This creates a symbolic link that allows biorouter to find Node.js in the expected location while keeping your actual installation intact.

### Malicious package detected

If you see an error about a "blocked malicious package" when trying to use an extension, it means the extension was blocked because malware was detected in a package used by the extension. The error message will contain details about the package, for example:

```text
Blocked malicious package: package-name@1.0.0 (npm). OSV MAL advisories: MAL-2024-1234
```

Steps to resolve:

1. **Find an alternative**: Look for similar extensions in the [extensions directory][extensions-directory] or [PulseMCP](https://www.pulsemcp.com/servers).
2. **Optional verification**: Verify the source of the blocked extension or the package name/publisher.
3. **Report false positives**: If you believe this is an error, please [open an issue][github-issues].

This security check only applies to locally-executed external extensions that use PyPI (`uvx`) or NPM (`npx`). The check uses real-time data from the OSV database; if the security service is unavailable, extensions will still install normally.

As a best practice, only install extensions from trusted, official sources.

### Airgapped and offline environments

If you're working in an airgapped, offline, or corporate-restricted environment, you may encounter issues where MCP server extensions fail to activate or download their runtime dependencies.

Symptoms:

- Extensions fail to activate with error messages about missing runtime environments.
- Errors containing "hermit:fatal" or failed internet downloads.
- Extensions work on personal machines but fail in corporate/restricted networks.
- Error messages like: `Failed to start extension: Could not run extension command`.

biorouter Desktop uses **"shims"** (packaged versions of `npx` and `uvx`) that automatically download runtime environments via Hermit. In restricted networks, these downloads fail. The workaround is to use custom command names:

1. **Create alternatively named versions of package runners on your system:**

   ```bash
   # For uvx (Python packages)
   ln -s /usr/local/bin/uvx /usr/local/bin/runuv
   
   # For npx (Node.js packages)  
   ln -s /usr/local/bin/npx /usr/local/bin/runnpx
   ```

2. **Update your MCP server configurations to use the custom names:**

   Instead of:

   ```yaml
   extensions:
     example:
       cmd: uvx
       args: [mcp-server-example]
   ```

   Use:

   ```yaml
   extensions:
     example:
       cmd: runuv  # This bypasses biorouter's shims
       args: [mcp-server-example]
   ```

3. **Why this works:** biorouter only replaces known command names (`npx`, `uvx`, `jbang`, etc.) with its packaged shims. Custom names are passed through unchanged to your system's actual executables.

4. **Require more changes**: In a corporate proxy environment or airgapped environment where the above doesn't work, it is recommended that you customize and package up biorouter desktop with shims/config that will work given the network constraints you have (for example, TLS certificate limitations, proxies, inability to download required content etc).

## System permissions

### macOS permission issues

If you encounter an issue where the biorouter Desktop app shows no window on launch, it may be due to file and folder permissions. This typically happens because biorouter needs read and write access to the `~/.config` directory to create its log directory and file. Similarly, if tools fail to create files or directories during use, it could be caused by the same permission issue.

#### Checking permissions

1. Open Terminal.
2. Run the following command to check the current permissions for `~/.config`:

   ```sh
   ls -ld ~/.config
   ```

   Example output:

   ```text
   drwx------  7 yourusername  staff  224 Jan 15 12:00 /Users/yourusername/.config
   ```

`rwx` indicates you have read (r), write (w), and execute (x) permissions for your user. If you do not see `rwx` for your user, follow the steps below.

#### Granting read and write permissions

1. To add the correct permissions, run the following commands:

    ```sh
    chmod u+rw ~/.config
    ```

    If the `~/.config` directory does not exist, create it and then assign permissions:

      ```sh
      mkdir -p ~/.config
      chmod u+rw ~/.config
      ```

2. Verify the change:

    ```sh
    ls -ld ~/.config
    ```

If you still experience issues after fixing permissions, try launching biorouter with superuser (admin) privileges:

```sh
sudo /Applications/Biorouter.app/Contents/MacOS/Biorouter
```

> **Note.** Running biorouter with sudo may create files owned by root, which could lead to further permission issues. Use this as a troubleshooting step rather than a permanent fix.

#### Updating permissions in System Settings

1. Go to `System Settings` -> `Privacy & Security` -> `Files & Folders`.
2. Grant biorouter access.

## Uninstalling biorouter or removing cached data

You may need to uninstall biorouter or clear existing data before re-installing. biorouter stores data in different locations depending on your operating system. Secrets, such as API keys, are stored exclusively in the system keychain/keyring.

### macOS

#### Data locations

- **Config**: `~/.config/biorouter`
- **Data and sessions**: `~/.local/share/biorouter`
- **Logs**: `~/.local/state/biorouter`
- **Application Data**: `~/Library/Application Support/Biorouter`
- **Secrets**: macOS Keychain (credential named "biorouter").

`biorouter info` prints the first three for your own machine.

#### Removal steps

1. Stop any copies of biorouter running (CLI or GUI).

   - Consider confirming you've stopped them all via Activity Monitor.

2. Open Keychain Access and delete the credential called "biorouter", which contains all secrets stored by biorouter.
3. Remove data directories:

   ```bash
   rm -rf ~/.config/biorouter
   rm -rf ~/.local/share/biorouter
   rm -rf ~/.local/state/biorouter
   rm -rf ~/Library/Application\ Support/Biorouter
   ```

4. Delete the "biorouter" app from your Applications folder (if using biorouter Desktop).

### Linux

#### Data locations

- **Data/Sessions**: `~/.local/share/biorouter/`
- **Logs**: `~/.local/state/biorouter/`
- **Config**: `~/.config/biorouter/`
- **Secrets**: System keyring (if available)

#### Removal steps

- Stop any copies of biorouter running (CLI or GUI).
- Clear secrets from your system keyring (if applicable).
- Remove data directories:

  ```bash
  rm -rf ~/.local/share/biorouter/
  rm -rf ~/.local/state/biorouter/
  rm -rf ~/.config/biorouter/
  ```

### Windows

#### Data locations

- **Configuration and Data**: `%APPDATA%\Biorouter\`
- **Local Application Data**: `%LOCALAPPDATA%\Biorouter\`
- **Secrets**: Windows Credential Manager

#### Removal steps

1. Stop any copies of biorouter running (CLI or GUI).

   - Check Task Manager to confirm all instances are closed.

2. Open Windows Credential Manager and delete credentials related to "biorouter".
3. Remove data directories:

   ```text
   rmdir /s /q "%APPDATA%\Biorouter"
   rmdir /s /q "%LOCALAPPDATA%\Biorouter"
   ```

4. Uninstall the biorouter Desktop app from Settings > Apps (if applicable).

> **Note.** After this cleanup, if you are looking to try out a fresh install of biorouter, you can now start from the usual [install instructions](../getting-started/installation.md).

## Getting further help

Still running into issues? [Open an issue on GitHub][github-issues] where the biorouter team and community members are happy to assist.

> **Note.** If you can share a [diagnostic report](diagnostics-and-bug-reports.md#diagnostics-bundle) along with your question, it helps maintainers understand your setup and provide more targeted solutions.

## Related documentation

- [Diagnostics and bug reports](diagnostics-and-bug-reports.md) — how to generate the bundle every issue report should carry, and where to file the report.
- [Installation](../getting-started/installation.md) — the install and reinstall path several fixes above end in.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — provider setup, including Ollama and its current default models.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_DISABLE_KEYRING`, `BIOROUTER_TERMINAL`, lead/worker settings, and provider retry tuning.
- [Secret storage](../security/secret-storage.md) — what biorouter puts in the system keychain and how the plaintext fallback works.

[github-issues]: https://github.com/BaranziniLab/biorouter/issues
[installation]: ../getting-started/installation.md
[biorouterhints]: ../agent-loop/context-engineering.md
[configure-llm-provider]: ../getting-started/choosing-a-model-provider.md
[extensions-directory]: ../extensions/extensions-and-skills-guide.md#installing-from-the-baam-marketplace
