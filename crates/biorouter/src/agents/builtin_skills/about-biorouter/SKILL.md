---
name: about-biorouter
description: "Built-in self-knowledge about Biorouter. Load this skill whenever the user asks about Biorouter itself: what it is, how it works, who built it, or how to use or configure any of its features (extensions, skills, workflows, scheduler, knowledge bases, models and providers, secrets, CLI, or the desktop app)."
---

# About Biorouter

This is the authoritative self-knowledge reference for Biorouter. Use it to answer
questions about Biorouter accurately instead of guessing from general knowledge.
If a question goes deeper than this document, point the user to the documentation
at <https://biorouter.ucsf.edu/docs>.

## What Biorouter is

Biorouter is an open-source, AI-powered integrated research environment for
biomedical discovery, created by Wanjun Gu and the Baranzini Lab at UCSF. It
unifies commercial, institution-hosted, and local LLMs, AI agents, MCP-based
extensions, personal knowledge bases, and customizable workflows into one
extensible tool for exploratory analysis, prototyping, and automation.

- Website: <https://biorouter.ucsf.edu/>
- Extension & skill marketplace (BAAM): <https://biorouter.ucsf.edu/baam>
- Documentation: <https://biorouter.ucsf.edu/docs>
- Downloads: <https://biorouter.ucsf.edu/download>

## Architecture

Three layers:

1. **Interface**: the desktop app (Electron + React) or the `biorouter` CLI.
2. **Agent**: the reasoning loop holding session state, talking to the
   configured LLM provider, and dispatching tool calls.
3. **Extensions**: pluggable MCP (Model Context Protocol) servers that provide
   tools and context.

The desktop app spawns a local REST/WebSocket server (`biorouterd`) and talks to
it over a generated, type-safe API client. The CLI calls the agent library
directly. Sessions are persisted in the **data** directory, not the config one:
`~/.local/share/biorouter/sessions/` on macOS and Linux.

## The major pillars

### Capabilities and extensions

Capabilities are the tools compiled into Biorouter. **Developer**, **Computer
Controller**, **Auto Visualiser**, **Code Execution**, **Extension Manager**,
**Skills**, **Todo**, **Memory**, **Knowledge**, **Workspace Control**, and
**Agent Drafter** are enabled by default. **Chat Recall** is disabled by default.
Manage them in **Settings → Chat → Capabilities**.

User-installed extensions are MCP servers added through `stdio` (external
process), `streamable_http` (remote), or `inline_python`. Manage them from the
**Extensions** page in the desktop sidebar, via `biorouter configure`, ad-hoc with
`biorouter session --with-extension <cmd>`, or in
`~/.config/biorouter/config.yaml` under the `extensions:` key.
- Third-party extensions are browsable at <https://biorouter.ucsf.edu/baam>.

### Skills

Skills are reusable instruction sets: a folder containing a `SKILL.md` file
with YAML frontmatter (`name`, `description`) followed by markdown
instructions. The always-on system prompt carries only a **count** of the
enabled skills, not their names; the agent finds a skill with `searchSkills`,
pages the catalog with `searchSkills` (no query), and pulls in one body with `loadSkill`.

- Primary location: `~/.config/biorouter/skills/<slug>/SKILL.md`. Also
  discovered from `~/.claude/skills`, `~/.config/agents/skills`, extension
  bundles, and project-local `.biorouter/skills`, `.claude/skills`,
  `.agents/skills` directories.
- Manage from the **Skills** page in the sidebar (add, toggle, delete) or with
  `biorouter skill install|list|enable|disable|remove` in the CLI.
- Toggles persist in `~/.config/biorouter/skills-config.json`, written by both
  the GUI toggle and `biorouter skill enable|disable` (which accept a skill
  name, bundle name, or directory slug).
- A **bundle** is a directory of skills installed, listed and toggled as one
  unit: `<root>/<bundle>/<slug>/SKILL.md`. Disabling the bundle by name disables
  every member.
- Skills that ship with Biorouter are **Contexts**, switched in
  **Settings → Chat → Contexts** rather than offered per chat, and excluded from
  the "N skills enabled" counts because the user did not install them. There are
  five rows over nine shipped skills: `about-biorouter`, `develop-biorouter`,
  `develop-biorouter-extension`, `develop-biorouter-skill`, and **Knowledge** —
  one row over the `knowledge-bases` bundle, whose members are
  `knowledge-choose-a-format`, `knowledge-ingest-okf`, `knowledge-ingest-biookf`,
  `knowledge-lint` and `update-soul`.
- This `about-biorouter` skill is one of them: it ships with Biorouter, can be
  toggled off, cannot be deleted from any surface, and is restored automatically
  if its folder is removed. Switching a Context off stops it being *surfaced* in
  the catalog; it stays loadable by exact name.

### Workflows

Workflows are declarative YAML/JSON automation definitions executed by the
agent. Key fields: `version`, `title`, `description`, plus `instructions`
(system prompt for the session) and/or `prompt` (initial message, required for
headless/scheduled runs). Optional: `parameters` (typed inputs templated into
text with `{{ name }}` Jinja syntax), `extensions`, `skills`,
`knowledge_bases`, `settings` (per-workflow provider/model overrides),
`activities` (clickable starter buttons in the desktop UI), `response` (JSON
schema for structured output), `sub_workflows`, and `retry`.

- Manage from the **Workflows** page in the sidebar.
- CLI: `biorouter run --workflow my.yaml --params key=value` (repeat `--params`
  for each one), plus `biorouter workflow install|validate|deeplink|open|list`.

### Scheduler

The scheduler runs workflows on a cron cadence (standard 5-field cron, e.g.
`0 9 * * 1` = 9am Mondays). Jobs persist in `schedule.json` in the **data**
directory (`~/.local/share/biorouter/schedule.json`) and always run headless, so
the workflow must define a `prompt`.

- Manage from the **Scheduler** page in the sidebar (create, pause, resume,
  edit, delete, run-now) or with
  `biorouter schedule add|list|remove|sessions|run-now|cron-help` (aliased
  `biorouter sched`). The subcommand is `remove`, not `delete`.
- The agent itself can manage schedules via the platform schedule-management
  tool (list, create, run_now, pause, unpause, delete, inspect, sessions).

### Knowledge bases

Personal, LLM-maintained knowledge bases backed by markdown page trees with
full git history. Each KB lives at `~/.config/biorouter/knowledge/<kb-id>/`
with `raw/` (original sources), `knowledge/` (curated, cross-linked pages),
`index.md`, `log.md`, and `schema.md` (editable conventions that steer the
ingestion sub-agent). New bases are written in the **Open Knowledge Format**
(OKF v0.2) or its strict biomedical profile **BioOKF v0.5**, where a
cross-reference is an ordinary markdown link to the target page's path (BioOKF
additionally declares graph relations as typed `edges:` in the frontmatter).
Two different things are called "legacy" here, and only one of them still
works. A base in the retired **pre-OKF format** (`title:`/`kind:` frontmatter)
is purged on startup, and until it has been, every tool that WRITES or
validates refuses it — `kb_write_page`, `kb_add_raw_source`, `kb_validate_page`, `kb_lint`,
`kb_begin_txn` and `kb_append_log`. The read-only tools
(`kb_read_page`, `kb_list_pages`, `kb_search`, `kb_get_graph`, `kb_list_history`,
`kb_export`) still work, so the user can get their content out. Tell them to
restart Biorouter rather than trying to repair the base. Untyped
`[[double bracket]]` **links** inside a current OKF or BioOKF page are a
different matter: they are still read, permanently, so do not rewrite an old
page — just stop writing new ones.

- Ingest sources (URLs, pasted text, PDFs, HTML, DOCX, CSV) from the
  **Knowledge** page in the sidebar, with live streaming progress. Sources are
  credibility-classified (peer-reviewed > preprint > book > gray literature >
  web > personal) via Crossref/OpenAlex lookup.
- In chat, the Knowledge capability provides `kb_search` (BM25 over curated
  pages), `kb_read_page`, `kb_write_page`, graph view, history/restore, and
  `.brkb` export/import for sharing whole KBs.
- A graph view visualizes pages as nodes connected by those cross-references.
  An "active KB" can be selected per session from the chat
  composer.
- **Soul** is a built-in personal KB (`kb_id` "soul") installed on first run. It
  holds durable facts about the user (how they work, tools/commands they prefer,
  personal details) and is grown automatically by a "Meditation" workflow and a
  daily 3:00 AM "Daily Meditation" scheduled job, guided by the built-in
  `update-soul` skill. Consult it (`kb_search` with `kb_id="soul"`) to
  personalise answers; it may be hidden, so search it by explicit id.

### Models & providers

Biorouter registers 23 built-in providers, plus any declarative custom provider
the user adds: Anthropic, Azure OpenAI, AWS Bedrock, Versa Azure and Versa
Bedrock (institution-hosted, UCSF), Claude Code, Codex, Databricks, GCP Vertex
AI, GitHub Copilot, Google, LiteLLM, Llama Server, Ollama, OpenAI, OpenRouter,
SageMaker TGI, Snowflake, Tetrate, Venice, xAI, Xiaomi MiMo, and Z.ai. (Three of
them — Bedrock, Versa Bedrock and SageMaker TGI — sit behind the
`aws-providers` build feature, which is **on by default**, so a shipped build
has all 23.)

- **Local models need no setup: Llama Server** (`llamacpp`) runs a llama.cpp
  server bundled with the desktop app and downloads models on first use;
  **Ollama** drives a separate Ollama install. Local providers rank ahead of
  institutional and commercial ones everywhere in the UI.
- **Claude Code** and **Codex** run inference on the user's *own* vendor
  subscription by driving a coding-agent CLI they already installed and signed
  in to. Biorouter never sees a credential — there is no base URL and no API
  key. Both are Public-tier, so they must not be used with private data.
- Defaults are set with the `BIOROUTER_PROVIDER` and `BIOROUTER_MODEL` config
  keys; workflows can override per-run in their `settings` block.
- Configure in **Settings → Providers/Models** in the desktop app, or via CLI:
  `biorouter models current`, `biorouter models providers`,
  `biorouter models list <provider>`,
  `biorouter models set --provider X --model Y`, or `biorouter configure`.

### Secrets

API keys and other secrets are resolved in this order:

1. **Environment variable**, exact key match (e.g. `OPENAI_API_KEY`). This wins
   over everything else.
2. **The OS credential store** via the `keyring` crate — macOS Keychain,
   Windows Credential Manager, Linux Secret Service. This is the default store.
   It is read **at most once per process** and cached in memory, so macOS shows
   at most one Keychain authorization prompt per run; tell users to click
   **Always Allow**. Authorization is per binary, so the desktop backend
   (`biorouterd`) and the CLI (`biorouter`) each get their own grant.
3. **Plaintext `~/.config/biorouter/secrets.yaml`**, used when the keyring is
   switched off with `BIOROUTER_DISABLE_KEYRING`, or when the platform store is
   unavailable (headless Linux, SSH, WSL, no Secret Service daemon), where
   Biorouter falls back to it automatically. ⚠ **Any value switches it off** —
   the check is on the variable's presence, so `BIOROUTER_DISABLE_KEYRING=false`
   disables the keyring exactly as `=true` does. Unset it to re-enable.

There is no configurable "secrets backend" key and no encrypted-file store.
On Windows a large secret set is transparently chunked across several
credentials to stay under the 2560-byte per-credential limit.
See `docs/security/secret-storage.md`.

## Interfaces

### Desktop app

The left sidebar leads with **Home** and **New chat**. Everything else sits
behind a **Components** disclosure — collapsed by default, and remembered —
holding **Workflows**, **Scheduler**, **Extensions**, **Skills**, **Knowledge**
and **Built apps** (apps built with Agent Drafter). Below that is a **Recents**
list of recent chats with a **See all** link to the full session history, and
**Settings** (providers, models, permissions, theme) is pinned at the bottom.
Chat renders markdown, syntax-highlighted code and expandable tool-call
messages; figures, reports and other artifacts the agent creates open in the
**artifact side panel** on the right, never inline.

### CLI

Main commands: `biorouter session` (interactive chat, `--name`/`--resume`;
aliases `s` and `sessions`, with subcommands such as `session list`, `send`,
`watch`, `cancel` — there is no `session-list`), `biorouter run` (headless:
`--text "prompt"` or `--workflow file.yaml --params k=v`), `biorouter configure`
(interactive setup), `biorouter models …`, `biorouter schedule …` (alias
`sched`), `biorouter workflow …`, `biorouter skill …`, `biorouter extension …`
(alias `ext`), `biorouter knowledge …` (alias `kb`), `biorouter apps …`,
`biorouter serve` (alias `headless`: run Biorouter and reach it from a browser),
`biorouter usage` (token and cost report), `biorouter doctor` (check
prerequisites; `--fix <dep>` hands the failure to the agent), `biorouter info`,
`biorouter project`/`projects`, `biorouter term` (terminal-integrated session),
`biorouter completion`, `biorouter acp`, `biorouter mcp <server>`,
`biorouter bench …`, and `biorouter setup-path` (alias `install-cli`).

## Configuration file map

Two roots, and the split matters: **config** is `~/.config/biorouter/`, **data**
is `~/.local/share/biorouter/`.

| Path | Purpose |
|---|---|
| `~/.config/biorouter/config.yaml` | Providers, model defaults, extensions |
| `~/.config/biorouter/secrets.yaml` | Plaintext secrets, only when the OS keyring is off or unavailable |
| `~/.config/biorouter/skills/` | Installed skills |
| `~/.config/biorouter/workflows/` | Workflows |
| `~/.config/biorouter/knowledge/` | Knowledge bases |
| `~/.config/biorouter/extensions/<name>/` | Installed `.brxt` extensions |
| `~/.config/biorouter/skills-config.json` | Skill enable/disable state |
| `~/.local/share/biorouter/sessions/` | Session history (**data** dir) |
| `~/.local/share/biorouter/schedule.json` | Scheduled jobs (**data** dir) |
| `.biorouterhints` / `AGENTS.md` (project) | Project-specific agent guidance |
| `.biorouterignore` (project) | Files the agent must not read |

## How to use this skill

When answering questions about Biorouter, ground your answer in this document.
For hands-on guidance or anything not covered here, refer the user to
<https://biorouter.ucsf.edu/docs>.
