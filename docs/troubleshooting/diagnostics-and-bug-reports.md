# Diagnostics and bug reports

> **What this is.** How to have biorouter report a bug for you, how to produce a diagnostics bundle from the desktop app or the CLI, what the bundle contains, and how to file a bug report or feature request on GitHub yourself.
> **Status:** Current.
> **Audience:** end users

biorouter provides several built-in features to help you get support, report issues, and request new functionality. This page covers the diagnostics system, bug reporting, and feature request tools. When something is going wrong, start with [Common problems and fixes](common-problems-and-fixes.md); come here once you need to hand a maintainer the details of your setup.

| Feature | Purpose | Location | Output |
|---------|---------|----------|---------|
| **Ask the agent** | Have biorouter work out what went wrong and file it for you | Type `/bug` in any chat, or say "report a bug" | A GitHub issue, after you approve the exact text |
| **Diagnostics** | Generate troubleshooting data | Chat summary (upper right) → `Diagnostics` | ZIP file with system info, logs, and session data |
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

1. In an active chat session, click the **chat summary** icon in the upper right of the chat header.
2. Click `Diagnostics` in the popover.
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

The shortest path is to type `/bug` in the chat where it happened:

> /bug

or, if you already know what is wrong, add it after the command:

> /bug the chart panel is blank when the dataset has one row

Saying it in your own words works too: "report a bug", or "report a bug: the chart panel is blank when the dataset has one row".

In the desktop app, `/bug` becomes a **Report a bug** chip at the start of the composer once you type the space after it, or when you pick it from the `/` menu. Type what went wrong after the chip, or send the chip on its own. The chip's **×**, or Backspace at the start of the text, removes it and keeps your text. Your sent message shows the same chip. In the terminal `/bug` stays plain text: the full-screen chat lists it when you type `/`, and the classic prompt completes it with Tab.

biorouter then:

1. **Reads the session's own record of failed tool calls**, grades each one, and works out whether there is a clear defect. It does this from the conversation, not from a bundle — the conversation is where a failed call is actually recorded.
2. **Pushes back if it cannot tell.** If nothing conclusive happened and you have not said what to report, it asks you rather than guessing. It will name what it can see and ask whether that is the problem. It does not file on a hunch.
3. **Checks the documentation and the source code.** It compares what you expected with what happened, looks up whether the behaviour is documented at [biorouter.ucsf.edu/docs](https://biorouter.ucsf.edu/docs), and reads the code involved, at the version you run, in the [project repository](https://github.com/BaranziniLab/biorouter). If the documentation and the code show biorouter worked as designed, it explains how the feature is meant to be used instead of filing, and files only if you still want it to. If the chat has no tool that can read web pages or files, the report's **Suspected cause** says the documentation and the source were not checked.
4. **Writes the report.** What was observed goes in the description. Its diagnosis goes in a separate **Suspected cause** section: the files and functions it suspects, its reasoning, the evidence, and how confident it is, so a maintainer or a debugging agent can take over from there. When nothing supports a diagnosis, that section says so, along with what it checked and what it could not check, instead of guessing. It adds the version, OS, provider, model, enabled extensions and the failure list, and removes home paths, usernames, e-mail addresses and anything credential-shaped. If identifying material survives that pass, it refuses to file rather than posting anyway.
5. **Asks you to approve the exact text.** The approval card shows the whole issue body, names the repository, and says whether pressing the button publishes immediately or opens a page you still have to submit. Nothing is posted until you approve, and a refusal files nothing. In the terminal, the report's title and body are printed with the approval question; in the full-screen chat, scroll the approval box with PageUp and PageDown to read them.
6. **Files it**, with your own signed-in [GitHub CLI](https://cli.github.com) if you have one. Otherwise it opens a prefilled new-issue page in your browser for you to submit. If the browser cannot be opened, the reply gives you the link to open yourself.

It will not treat a deliberate refusal as a bug. If biorouter refused something on purpose (a privacy boundary, a permission decision), it says so instead of filing "the security boundary worked" as a defect. Tell it if you think the *wrong* thing was refused.

This works in a Claude Code or Codex chat too. The reporter is bridged to those, so `/bug` and the same words work there.

### Reporting from a private chat

A chat classified private can report a bug. Three things are different:

- **The approval card warns you first.** It says the chat is private and why (for example, it reached a private data source), and that the report becomes public if you submit it. Read the whole text for patient or participant data, credentials, unpublished results or institutional information before you go on. The list of failed tool calls quotes raw tool output.
- **Nothing is posted automatically.** Even with a signed-in GitHub CLI, approving only opens a prefilled GitHub page, or hands you the text when it is too long for a link. You decide on GitHub whether to press **Submit**.
- **The agent is told to stay on public pages.** In a private chat its instructions say to fetch only fixed pages on GitHub and the documentation site, never to put text from your chat into a search or a web address, and to write about biorouter's behaviour rather than about your data. Biorouter does not enforce this, and the fetches happen before the approval card appears. A model that ignores the instruction could send chat text to those sites. If that matters, switch to the **Manual Approval** [permission mode](../security/permission-modes.md), which asks before every tool call you have not already allowed, or watch the tool calls the agent makes and stop the turn if it searches or fetches anything built from your conversation.

The prefilled page carries the report in its web address. Opening it sends the text to github.com and leaves it in your browser history, even before you press Submit.

If biorouter cannot confirm the chat's classification, it treats the chat as private. If you turned private and public protection off in **Settings → App → Privacy**, every chat files the same way. From a private chat the report leaves out its usual line inviting you to attach a diagnostics bundle, and after you approve, biorouter tells you not to attach one: a private chat's bundle holds the conversation unredacted and does not belong on a public issue.

If a private chat has no private model to run on, biorouter refuses the whole turn before any command runs, `/bug` included. Use **File Bug on GitHub** in the Diagnostics dialog instead; it needs no model.

### Where it cannot file

If biorouter is running somewhere it cannot ask you to approve a publication (`biorouter serve` in a browser, for one), the reporter is not offered, and `/bug` answers that it cannot file there and points you to the issue tracker. A chat connected to Crew allows only Crew and checklist tools, so `/bug` asks you to report from a regular chat. In both cases, use the manual flow below.

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

- **Session and system logs**: `~/.local/state/biorouter/logs/` on macOS and Linux (`%LOCALAPPDATA%\biorouter\logs` on Windows) — the LLM request logs at its root, and the CLI and daemon's own logs under `cli/` and `server/`. The desktop app's main-process log is separate, under Electron's own application-support directory. The bundle above collects the relevant ones under `logs/` and says in `logs-summary.txt` what it took.
- **[Telemetry export](../configuration/environment-variables.md#observability)**: configure telemetry for performance analysis and production monitoring.

## Related documentation

- [Common problems and fixes](common-problems-and-fixes.md) — the symptom-by-symptom reference to check before filing an issue; several of its entries end by asking for the bundle described here.
- [Troubleshooting index](README.md) — the entry point for this folder and the recommended order of steps.
- [CLI command reference](../cli/command-reference.md#session-diagnostics-options) — every flag on `biorouter session diagnostics`, plus the rest of the session subcommands.
- [Configuration file reference](../configuration/config-file-reference.md) — what lives in the config files the bundle collects.
- [Environment variables](../configuration/environment-variables.md#observability) — the observability settings behind telemetry export.
