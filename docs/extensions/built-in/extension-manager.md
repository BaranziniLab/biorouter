# Extension Manager capability

> **What this is.** User guide to the built-in Extension Manager: how BioRouter discovers, enables and disables other extensions mid-session so the active tool count stays small, how it searches the trusted BAAM marketplace to install a package you don't have, and how it permanently uninstalls one you no longer want — whether or not it came from the marketplace.
> **Status:** Current. The capability is enabled by default, so no manual setup is normally needed.
> **Audience:** end users.

You don't always need to manage extensions by hand. The Extension Manager lets BioRouter discover, enable and disable extensions during an active session. Based on the task you give it, BioRouter recognizes when it needs a specific extension, enables it, and suggests disabling unused ones when the tool bloat starts eating your context window. Describe your task and BioRouter handles the extension management.

It is not limited to what you already have. When nothing installed fits the task, BioRouter can search the trusted BAAM marketplace, install a package from it with your approval, and — with a separate approval — permanently uninstall an extension you no longer want.

> **Note.** This capability is **enabled by default**. Its internal registration still uses the legacy `PlatformExtensionDef` type; that storage name does not make it an installed extension. The configuration walkthrough below is only needed if you previously disabled it, or want to confirm its state.

## Configuration

1. Run the `configure` command:

   ```bash
   biorouter configure
   ```

2. Choose `Toggle Extensions`, then confirm `extensionmanager` is enabled:

   ```text
   ┌   biorouter-configure
   │
   ◇  What would you like to configure?
   │  Toggle Extensions
   │
   ◆  Enable capabilities and extensions: (use "space" to toggle and "enter" to submit)
   │  ● extensionmanager
   └  Extension settings updated successfully
   ```

## Why use the Extension Manager

BioRouter can work with many extensions, but having too many enabled at once can:

- Overwhelm the LLM with too many tool choices
- Reduce the quality of tool selection
- Slow down response times
- Exceed the recommended budget of 5 extensions or 50 tools

The Extension Manager addresses this by letting BioRouter:

- **Discover** what extensions are available
- **Enable** extensions only when needed for a specific task
- **Disable** extensions when they're no longer required

The same capability also reaches past what is already installed, letting BioRouter:

- **Search** the trusted BAAM marketplace for a package that fits the task
- **Install** that package with your approval, without you leaving the chat
- **Delete** an installed marketplace package permanently, again with your approval
- **Remove** any other installed extension — one you sideloaded, or an MCP server added by hand — by its installed name, again with your approval

The result is a more focused session where BioRouter has exactly the tools it needs, when it needs them.

> **Note.** The "5 extensions / 50 tools" figure is a rule of thumb for keeping tool selection sharp, not an enforced limit. Nothing fails when you exceed it; tool-choice quality and response time degrade gradually.

## Available tools

| Tool | Description | Use Case |
|------|-------------|----------|
| `search_available_extensions` | List installed third-party extensions and exact names | Finding what is already installed |
| `search_marketplace_extensions` | Browse or search trusted BAAM entries visible to this model — omit the query to list everything. A query is matched word by word, so a phrase returns every entry matching any of its words, best match first; a query that matches nothing says so and suggests shorter terms | Finding a package to install |
| `manage_extensions` | Enable or disable an extension by name | Loading/unloading extensions dynamically |
| `install_extension` | Install a BAAM marketplace extension end to end | The extension is not installed at all |
| `delete_extension_package` | Delete one or up to 50 validated marketplace packages after approval | Permanent removal of something installed from BAAM; shared credentials are retained |
| `remove_extension` | Remove one or up to 50 installed extensions by installed name after approval | Permanent removal of anything else — a sideloaded `.brxt`, a hand-configured MCP server |
| `list_resources` | List resources from extensions (if supported). When there are none, it says which extensions were asked — naming only extensions the model has already been shown | Discovering available data sources |
| `read_resource` | Read specific resource content (if supported) | Accessing extension-provided data |

> **Tip.** Not every tool in this table is offered in every session. The resource tools (`list_resources` and `read_resource`) appear only when at least one enabled extension supports resources. `install_extension`, `delete_extension_package` and `remove_extension` each wait on your approval, so they are withheld entirely where no one can be asked for it — on a daemon started by `biorouter serve` for browser access, for instance. Browsing and searching are read-only and always available.

## Uninstalling one you no longer want

There are two uninstall tools because there are two ways an extension gets onto your machine, and only one of them leaves a marketplace registry id behind.

`delete_extension_package` is for a package installed from BAAM. It is named by that registry id, and BioRouter re-checks the marketplace entry and the recorded install before it asks you anything.

`remove_extension` is for everything else — a `.brxt` you downloaded and installed yourself, an MCP server you added to `config.yaml` by hand, anything that never had a registry id. It is named by the **installed name** you see in Settings → Extensions. Before this tool existed the agent had no way to uninstall one of these in a single step, and would instead edit your configuration file a line at a time.

Both work the same way once you approve them. In one transaction BioRouter detaches the extension from the chat, removes its entry from your configuration, deletes its package directory and any skills that directory contributed, drops the record of where it came from, and clears it from every saved chat that still listed it — so reopening an old conversation does not try to launch a server that is gone. Up to 50 extensions can be named in one batch; the whole batch is validated and shown to you before anything is touched, and if the installed extension changes while the approval is on screen the removal is abandoned rather than applied to something you did not see.

**Your credentials are deliberately left alone.** An API key can be shared between extensions — one UCSF credential can unlock more than one connector — so removing an extension never revokes, deletes or overwrites a stored secret. Remove a key you no longer want in Settings, where you can see what else uses it.

Built-in capabilities are not removable by either tool. They are not extensions you installed, they own no package directory, and they are managed in Settings → Chat → Capabilities.

## Installing one the user does not have

`manage_extensions` only enables what is already installed. `install_extension` handles the rest: it downloads and validates the bundle, builds its Python environment, collects any credentials the extension needs, registers it, and attaches it to the current chat — so the agent never shells out to `curl` and the CLI.

**The agent never sees a credential.** If the extension needs an API key, passcode or token, the install pauses and BioRouter opens its own dialog; the agent learns only which key *names* were configured. It must never ask for a value in chat — a credential in a chat message cannot configure anything and exposes it to every model that reads the transcript. The full design is in [Installing an extension, and where its credentials go](../installing-an-extension.md).

An agent using a public model cannot install or attach a private extension through
this manager, even when the diagnostic privacy toggle is off. Marketplace
eligibility is checked before approval or download. A private model can manage
public or private extensions subject to the normal approval and reach checks;
users can configure extensions directly in the app.

If an installed public extension was deliberately disabled, the manager can ask
for explicit approval to attach its unchanged configuration to this chat. That
approval does not authorize a private extension or override a later configuration
change. Built-in capabilities are managed separately in Settings → Chat → Capabilities.

## Malware scanning when an extension is enabled

Enabling an extension that launches a package runner triggers a security check before the extension starts. BioRouter infers the ecosystem from the command — a command ending in `npx` is treated as npm, one ending in `uvx` as PyPI — parses the first package argument, and queries the OSV vulnerability database for `MAL-*` advisories. If the package is flagged as malicious, enabling it is denied.

Two consequences are worth knowing:

- Commands from other ecosystems are not recognized, and the check **fails open** — it is skipped rather than blocking the extension.
- The endpoint can be overridden with the `OSV_ENDPOINT` environment variable.

## Example usage

In this example BioRouter enables the GitHub extension — an extension you install separately, rather than one of the built-ins documented in this folder — because the task needs it.

### BioRouter prompt

```text
List all my GitHub repositories
```

### BioRouter output

> **Note.** The transcript below is stylized. The uppercase `MANAGE_EXTENSIONS` and `LIST_REPOSITORIES` markers stand in for tool calls; they are not real tool names or a real tool-call format. The actual tool is `manage_extensions`, listed in [Available tools](#available-tools) above.

```text
I'll enable the GitHub extension for you so we can work with repositories.

MANAGE_EXTENSIONS
action: enable
extension_name: github

✅ The extension 'github' has been installed successfully

The GitHub extension is now active.

I'll list your GitHub repositories using the GitHub extension.

LIST_REPOSITORIES

Here are your repositories:

...

Would you like to work with any of these repositories?
```

## Related documentation

- [Extensions and skills guide](../extensions-and-skills-guide.md) — installing, configuring and removing extensions yourself.
- [Code Execution capability](code-execution.md) — the other lever for reducing context pressure, by batching tool calls instead of trimming the tool list.
- [Context engineering](../../agent-loop/context-engineering.md) — why the active tool count matters to the context window.
- [Extension trait design](../../history/legacy-architecture/extension-trait-design.md) — historical design record for how extensions are modelled internally.
