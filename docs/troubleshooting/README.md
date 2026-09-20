# Troubleshooting

> **What this is.** The index for the troubleshooting section: where to look up a specific error or symptom, how to produce a diagnostics bundle, and where to report a problem the documentation does not cover.
> **Status:** Current — this index replaces the two docs-site cards that the 2026-05-07 plain-markdown migration stripped.
> **Audience:** end users

This folder holds the end-user troubleshooting material: a catch-all reference of known problems and their fixes, and a guide to the built-in diagnostics bundle and the bug-report and feature-request flows. Support runs through the project's GitHub issue tracker, [github.com/BaranziniLab/biorouter/issues](https://github.com/BaranziniLab/biorouter/issues) — attaching a diagnostics bundle to an issue is the fastest way to get a useful answer.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Common problems and fixes](common-problems-and-fixes.md) | Roughly twenty independent problem/fix entries: doom-spiral loops, context-length errors, Ollama setup, rate limits, keyring failures, package runners, blocked malicious packages, macOS permissions, WSL networking, airgapped networks, and full uninstall and data-removal steps for macOS, Linux, and Windows. |
| [Diagnostics and bug reports](diagnostics-and-bug-reports.md) | What the diagnostics bundle collects, how to generate it from the desktop app or the CLI, how to file a bug report or feature request, and the "Ask biorouter" error-recovery button. |

## Where to start

1. Run `biorouter doctor`. It checks the system prerequisites and the CLI install in about a second, and names the missing one directly; `biorouter doctor --fix <dependency>` hands a failing prerequisite to Biorouter. See [`doctor`](../cli/command-reference.md#doctor-options) in the CLI command reference.
2. Search [Common problems and fixes](common-problems-and-fixes.md) for your error message or symptom. Most reported issues already have an entry there.
3. If nothing matches, generate a diagnostics bundle as described in [Diagnostics and bug reports](diagnostics-and-bug-reports.md). It captures your app version, operating system, session messages, configuration, and recent logs in one ZIP file.
4. Open an issue at [github.com/BaranziniLab/biorouter/issues](https://github.com/BaranziniLab/biorouter/issues) and attach the bundle.

> **Warning.** A diagnostics bundle contains your session messages and configuration. Review it before posting it publicly.

## Related documentation

- [Installation](../getting-started/installation.md) — install, update, and reinstall steps, which many fixes here end in.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — provider setup and default models, the source of most configuration errors.
- [Environment variables](../configuration/environment-variables.md) — the variables several fixes ask you to set, including `BIOROUTER_DISABLE_KEYRING` and the observability settings.
- [Secret storage](../security/secret-storage.md) — how biorouter stores API keys in the system keychain, and what to do when that fails.
