# Managing sessions

> **What this is.** The landing page for session management: what a session is, where biorouter keeps one on disk, and which document covers each part of starting, resuming, exporting, and pruning them.
> **Status:** Current — a working index naming the real source for each session topic. (The original docs-site body was stripped by the 2026-05-07 plain-markdown migration and its child pages were never recreated.)
> **Audience:** end users

Sessions are your continuous interactions with biorouter. Each session maintains context and conversation history, enabling biorouter to understand your ongoing work and provide relevant assistance.

This folder holds no guides of its own. Several pages elsewhere in the documentation link here for background on how sessions are stored, resumed, and pruned, so the sections below name the real source for each of those topics rather than restating them.

## Where sessions are stored

Session history lives under `~/.local/share/biorouter/sessions/sessions.db` on macOS and Linux. Run `biorouter info` for the authoritative path on your own machine.

> **Note.** biorouter stores sessions in a SQLite database (`sessions.db`) rather than individual `.jsonl` files, a change introduced in version 1.10.0. Sessions that predate the change are automatically imported into the database. Legacy `.jsonl` files remain on disk but are no longer managed by biorouter.

Because the history is a local database, features that search across past conversations read it directly from your own machine — see the [chat recall extension](../extensions/built-in/chat-recall.md). For where this sits among biorouter's other on-disk state, see the storage table in the [system overview](../architecture/system-overview.md).

## Where each topic is documented

| Topic | Where it is documented | What you get there |
|---|---|---|
| Starting, resuming, listing, removing, exporting, and diagnosing sessions | [CLI command reference — Session management](../cli/command-reference.md#session-management) | Every `biorouter session` subcommand and flag, including `--resume`, `--session-id`, `--name`, `session list`, `session remove`, `session export`, and `session diagnostics`. |
| Controls available once you are inside a session | [CLI command reference — Interactive session features](../cli/command-reference.md#interactive-session-features) | The slash commands (`/compact`, `/clear`, `/mode`, `/plan`, `/workflow`, `/t`, and the rest), themes, and keyboard shortcuts. |
| Turn limits and automatic compaction | [Environment variables — Session management](../configuration/environment-variables.md#session-management) | `BIOROUTER_MAX_TURNS`, `BIOROUTER_SUBAGENT_MAX_TURNS`, `BIOROUTER_AUTO_COMPACT_THRESHOLD`, and `BIOROUTER_CONTEXT_STRATEGY`, with their accepted values and defaults. |
| Persisting session defaults | [Configuration file reference](../configuration/config-file-reference.md) | The `config.yaml` settings that apply to every session, rather than one invocation. |
| Keeping a long session's context useful | [Context engineering](../agent-loop/context-engineering.md) | The memory, skills, workflow, hook, and subagent mechanisms for carrying knowledge between sessions. That page indexes the same set of guides from the context side; this page indexes them from the session-lifecycle side. |
| What a session is allowed to do | [Security](../security/README.md) | Permission modes, secret storage, and data-handling rules that govern a session's tool calls. This page covers session lifecycle; that one covers session authority. |

## Related documentation

- [CLI command reference](../cli/command-reference.md) — every `biorouter session` subcommand, and the slash commands available inside a session.
- [Environment variables](../configuration/environment-variables.md) — the turn-limit and auto-compaction settings that shape how long a session can run.
- [Context engineering](../agent-loop/context-engineering.md) — how to carry knowledge across sessions instead of re-explaining it each time.
- [Chat recall extension](../extensions/built-in/chat-recall.md) — searching the local SQLite history of past sessions.
- [Usage tips](usage-tips.md) — practical habits, including when to start a fresh session rather than continue one.
