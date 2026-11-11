# Command-line interface

This folder documents the `biorouter` command-line interface: the subcommands and flags you run from your shell, the slash commands, themes, and keyboard shortcuts available once you are inside an interactive session, and the manual QA script that verifies all of it. Two documents sit here in a define/verify pair — the reference states what the CLI surface is supposed to do, and the checklist is the pass that confirms a build actually does it.

Come here when you need to look up a command or flag, when you are learning the interactive terminal UI, or when you are running a QA pass before a release. Go elsewhere if you are still setting biorouter up — installation, provider configuration, and your first session live in [getting-started](../getting-started/quickstart.md), and the settings that change how commands behave live in [configuration](../configuration/environment-variables.md). The desktop GUI is a separate interface with its own folder, [desktop-ui](../desktop-ui/README.md); nothing here describes the Electron app.

## Documents in this folder

| Document | What it covers |
|---|---|
| [biorouter CLI command reference](command-reference.md) | The reference for every `biorouter` subcommand and its flags, plus the slash commands, themes, and keyboard shortcuts available inside an interactive session. It includes local dictation: `dictation` (install, list, use and remove speech models, `status`, `urls`, `devices`, `test`), `dictate`, `transcribe` and `dictionary`, and Ctrl+T and `/dictate` inside a session. Current. |
| [BioRouter CLI QA checklist](qa-checklist.md) | The end-to-end verification script for the CLI and its terminal UI: a per-surface checklist of things to exercise, the log of bugs found and fixed during the pass that produced it, and the GUI-parity gaps that pass identified. Current, but the pass records no date, version, or commit, so re-confirm its `[verified]` marks against the build you are testing. Dictation's terminal checks live in the [dictation test checklist](../releases/dictation-test-checklist.md) (sections S8 to S11 and S19), which this checklist points to. |

## Related documentation

- [biorouter in 5 minutes](../getting-started/quickstart.md) — install biorouter, configure a provider, and start a session before any command on this page will run.
- [Environment variables](../configuration/environment-variables.md) — the per-invocation overrides for model, permission mode, and data directories that change what a CLI command does.
- [Workflows](../workflows/README.md) — the reusable workflow files and scheduled jobs that the `workflow` and `schedule` subcommands load and run.
- [Troubleshooting](../troubleshooting/README.md) — where to go when a command fails, and how to produce a diagnostics bundle for a bug report.
- [Voice dictation](../dictation/README.md): how dictation works in the terminal, over SSH and on servers, which the `dictation`, `dictate`, `transcribe` and `dictionary` commands serve.
