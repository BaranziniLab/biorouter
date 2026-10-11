# Diagnostics and bug reports

> **What this is.** How to have biorouter report a bug for you, how to produce a diagnostics bundle from the desktop app or the CLI, what the bundle contains, and how to file a bug report or feature request on GitHub yourself.
> **Status:** Current.
> **Audience:** end users

biorouter provides several built-in features to help you get support, report issues, and request new functionality. This page covers the diagnostics system, bug reporting, and feature request tools. When something is going wrong, start with [Common problems and fixes](common-problems-and-fixes.md); come here once you need to hand a maintainer the details of your setup.

| Feature | Purpose | Location | Output |
|---------|---------|----------|---------|
| **Ask the agent** | Have biorouter work out what went wrong and file it for you | Say "report a bug" in any chat | A GitHub issue, after you approve the exact text |
| **Diagnostics** | Generate troubleshooting data | Chat summary (beside the chat, or its button in the chat header) → `Diagnostics` | ZIP file with system info, logs, and session data |
| **File Bug on GitHub** | Open a pre-filled issue template | Same `Diagnostics` dialog | Opens GitHub in your browser |
| **Report a Bug** | Open a blank issue template | Settings → Help & feedback | Opens GitHub issue template |
| **Request a Feature** | Suggest new features | Settings → Help & feedback | Opens GitHub issue template |

The first row is the shortest path and the one to reach for. The rest are
there for when you would rather do it yourself.

## Diagnostics bundle

The diagnostics feature creates a comprehensive troubleshooting bundle that includes system information, session data, configuration files, and recent logs. This is invaluable for debugging issues or getting technical support.

Generate one when you are:

- Experiencing crashes or unexpected behavior.
- Getting error messages you don't understand.
- Hitting performance issues or slow responses.
- About to report a bug and want to include technical details.

### Generating a bundle from the desktop app

1. Open the **Chat summary** of an active chat. In a wide window it is already docked to the right of the chat; otherwise click the summary button in the upper right of the chat header.
2. Click `Diagnostics` at the bottom of the summary.
3. Review the information in the dialog about what data will be collected.
4. Click `Generate diagnostics`. A native save dialog opens, defaulting to your `Downloads` folder.
5. The ZIP file is saved as `diagnostics_{session_id}.zip`. Nothing is uploaded anywhere.

> **Note.** Diagnostics is only available when you have an active session, as it needs a session ID to generate the bundle.

### Generating a bundle from the CLI

Use the session diagnostics command to generate a troubleshooting bundle. For complete details and all available options, see the [CLI command reference](../cli/command-reference.md#session-diagnostics-options).

```sh
# Generate diagnostics for a specific session
biorouter session diagnostics --session-id <session_id>

# Interactive selection (prompts you to choose a session)
biorouter session diagnostics

# Save to a custom location
biorouter session diagnostics --session-id <session_id> --output /path/to/diagnostics.zip
```

To find your session ID, first list available sessions:

```sh
biorouter session list
```

Example output:

```text
Available sessions:
abc123def - My coding session - 2024-01-15 14:30:22
xyz789ghi - Documentation work - 2024-01-15 10:15:45
```

### What the bundle contains

The diagnostics ZIP file contains several folders:

```text
diagnostics_abc123def.zip
├── logs/
│   ├── llm_request.0.jsonl          # This session's own request logs
│   ├── cli/<name>.log               # WARN and ERROR lines only
│   └── server/<name>.log            # WARN and ERROR lines only
├── logs-summary.txt      # What the log sweep found, and what it left out
├── session.json          # Your session messages, in full
├── config.yaml           # Your configuration, with credentials redacted
├── system.txt            # App version, OS, architecture, provider, model, extensions
├── usage.txt             # Token and cost accounting
├── schedule.json         # Scheduler state, if you have scheduled jobs
├── scheduled_workflows/  # Their workflow definitions
└── collection-notes.txt  # Present only if something could not be collected
```

Broken out by kind:

- **System information**: app version, operating system, architecture, provider, model, enabled extensions, timestamp.
- **Session data**: your conversation, including every tool call and every tool response.
- **Configuration**: your [configuration file](../configuration/config-file-reference.md), with any value whose key looks like a credential — and every value inside an extension's `envs` map — replaced.
- **Log files**: this session's own LLM request logs, plus the tail of the CLI and daemon logs filtered to `WARN` and `ERROR`. `logs-summary.txt` always says how many of each were included and why any were left out, so an empty `logs/` is never ambiguous.

> **Warning.** `session.json` is **not** redacted. It carries your whole conversation — every message, every tool call, every tool response — and your working directory path. Only `config.yaml` is scrubbed. Read the bundle before sharing it, and treat it as you would the conversation itself.

> **Note.** The agent-driven reporter below never attaches a bundle. It posts a short distilled report and scrubs it first; the bundle stays on your disk unless you attach it yourself.

## Asking biorouter to report the bug

The shortest path is to say so in the chat where it happened:

> report a bug

or, if you already know what is wrong:

> report a bug — the chart panel is blank when the dataset has one row

biorouter then:

1. **Reads the session's own record of failed tool calls**, grades each one, and works out whether there is a clear defect. It does this from the conversation, not from a bundle — the conversation is where a failed call is actually recorded.
2. **Pushes back if it cannot tell.** If nothing conclusive happened and you have not said what to report, it asks you rather than guessing. It will name what it can see and ask whether that is the problem. It does not file on a hunch.
3. **Writes the report**, adds the version, OS, provider, model, enabled extensions and the failure list, and removes home paths, usernames, e-mail addresses and anything credential-shaped. If identifying material survives that pass, it refuses to file rather than posting anyway.
4. **Asks you to approve the exact text.** The approval card shows the whole issue body, names the repository, and says whether pressing the button publishes immediately or opens a page you still have to submit. Nothing is posted until you approve, and a refusal files nothing.
5. **Files it** — with your own signed-in [GitHub CLI](https://cli.github.com) if you have one, otherwise by opening a pre-filled new-issue page for you to submit.

Two things it will not do:

- **It will not file from a chat classified private.** A GitHub issue is public and permanent, and a private chat has touched a private model or a private data source. It writes the report, hands it to you, and stops. File it yourself once you have read it — and for genuinely private material, prefer a private channel over the public tracker.
- **It will not treat a deliberate refusal as a bug.** If biorouter refused something on purpose — a privacy boundary, a permission decision — it says so instead of filing "the security boundary worked" as a defect. Tell it if you think the *wrong* thing was refused.

This works in a Claude Code or Codex chat too — the reporter is bridged to those, so you can ask for it there in the same words.

If the reporter is not offered in your chat, biorouter is running somewhere it cannot ask you to approve a publication — `biorouter serve` in a browser, for one. Use the manual flow below.

## Reporting bugs and requesting features yourself

Both flows open a structured GitHub issue template, so your report arrives with the information maintainers need. The desktop steps are the same for each; only the final button differs.

From the desktop app:

1. Open the sidebar using the button in the top-left.
2. Click `Settings` in the sidebar.
3. Scroll down to the `Help & feedback` section.
4. Click `Report a Bug` to file a bug, or `Request a Feature` to suggest new functionality.
5. This opens GitHub in your browser with the matching pre-filled template.

From the CLI, navigate directly to the GitHub repository:

| Report type | URL |
|---|---|
| Bug report | `https://github.com/BaranziniLab/biorouter/issues/new?template=bug_report.md` |
| Feature request | `https://github.com/BaranziniLab/biorouter/issues/new?template=feature_request.md` |

## Error recovery with "Ask biorouter"

When certain types of error occur in biorouter Desktop (such as failures to activate extensions), you'll see an `Ask biorouter` button in the error notification. This feature lets you quickly troubleshoot the issue with biorouter's help:

1. When the error occurs, an `Ask biorouter` button appears in the error notification.
2. Click the button to send the error details to biorouter in a chat prompt.
3. biorouter provides diagnostic suggestions and potential solutions.

## Further debugging

For issues not resolved by diagnostics:

- **Session and system logs**: `~/.local/state/biorouter/logs/` on macOS and Linux (`%APPDATA%\UCSF\biorouter\data\logs` on Windows) — the LLM request logs at its root, and the CLI and daemon's own logs under `cli/` and `server/`. The desktop app's main-process log is separate, under Electron's own application-support directory. The bundle above collects the relevant ones under `logs/` and says in `logs-summary.txt` what it took.
- **[Telemetry export](../configuration/environment-variables.md#observability)**: configure telemetry for performance analysis and production monitoring.

## Related documentation

- [Common problems and fixes](common-problems-and-fixes.md) — the symptom-by-symptom reference to check before filing an issue; several of its entries end by asking for the bundle described here.
- [Troubleshooting index](README.md) — the entry point for this folder and the recommended order of steps.
- [CLI command reference](../cli/command-reference.md#session-diagnostics-options) — every flag on `biorouter session diagnostics`, plus the rest of the session subcommands.
- [Configuration file reference](../configuration/config-file-reference.md) — what lives in the config files the bundle collects.
- [Environment variables](../configuration/environment-variables.md#observability) — the observability settings behind telemetry export.
