# System overview

> **What this is.** The orientation-level map of UCSF Biorouter: its three-layer architecture, its Rust and Electron tech stack, the agent interaction loop, where configuration and data live, and its security posture.
> **Status:** Current.
> **Audience:** anyone new to the codebase — developers, operators, and agents needing a starting point before descending into a subsystem.

UCSF Biorouter is an AI-powered integrated research environment that unifies commercial,
institution-hosted, and local large language models (LLMs), AI agents, Information Commons
databases, and customizable workflows into one extensible platform for explorative analysis,
prototyping, automation, and federated cross-institution collaboration.

Read this before any subsystem document. It names the layers and the crate boundaries that every
other page in `docs/` assumes you already know. It is a map, not a reference — where a topic has its
own page, this one links out rather than duplicating it.

## High-level overview

Biorouter is built as a modular, plugin-based system. It consists of three main layers:

1. **Interface** — The desktop GUI or CLI that accepts user input and displays responses.
2. **Agent** — The core reasoning loop that manages LLM interaction, tool execution, and session state.
3. **Extensions** — Pluggable MCP servers that give the agent access to tools (file operations, database queries, web access, code execution, etc.).

MCP is the Model Context Protocol, the open standard Biorouter uses to talk to extensions; an
extension is simply an MCP server, whether built in or installed by the user.

In a typical session, the interface starts an agent instance, which connects to one or more
extensions simultaneously and routes requests through the selected LLM provider.

## Tech stack

### Backend — Rust

The backend is a Rust workspace (`crates/`) organized into several crates:

| Crate | Purpose |
|---|---|
| `biorouter` | Core agent library — agent loop, provider integrations, session management, workflows, scheduling |
| `biorouter-server` | REST API server (`biorouterd`) that the desktop UI communicates with |
| `biorouter-cli` | Command-line interface (`biorouter` binary) |
| `biorouter-mcp` | Built-in MCP servers |
| `biorouter-sandbox` | Capability-scoped sandboxed execution for Biorouter agents (a leaf crate with no engine dependencies) |
| `biorouter-acp` | Agent Communication Protocol support |
| `biorouter-bench` | Benchmarking tools |
| `biorouter-test` | Integration tests |

The built-in MCP servers shipped in `biorouter-mcp` are `developer`, `computercontroller`, `webdocuments`, `memory`,
`autovisualiser`, `knowledge`, `agent_drafter`, `datasql`, `compute_server`, and
`files_server`.

Key Rust dependencies:

- **tokio** — Async runtime
- **axum** — HTTP web framework for the API server
- **rmcp** — Model Context Protocol implementation
- **reqwest** — HTTP client for provider API calls
- **serde / serde_json** — Serialization
- **tiktoken-rs** — Token counting for context management
- **minijinja** — Jinja-style template engine for workflows
- **tokio-cron-scheduler** — Cron-based job scheduling
- **sqlx (SQLite)** — Persistent session and schedule storage
- **etcetera** — Cross-platform config path resolution (`~/.config/biorouter/` on macOS/Linux)

### Frontend — Electron + React

The desktop application is an Electron app built with React and TypeScript.

| Component | Details |
|---|---|
| Framework | Electron 39 + React 19 |
| Build tool | Vite + Electron Forge |
| Language | TypeScript (strict mode) |
| Styling | TailwindCSS v4 with custom design tokens |
| UI components | Radix UI primitives |
| Routing | React Router DOM v7 |
| Testing | Vitest (unit), Playwright (E2E) |

The frontend communicates with the `biorouterd` REST server (started in the background by the
Electron main process) via a local HTTP API. The OpenAPI spec is generated from the Rust server and
used to type-safe frontend API calls.

### Local models — the bundled Llama Server

Alongside the remote providers, the desktop app bundles a pinned llama.cpp `llama-server` binary and
manages it as a sidecar process, exposed as the `llamacpp` provider. This gives zero-setup local
models with no separate install. The provider and its curated model catalog live in
[`crates/biorouter/src/providers/llamacpp.rs`](../../crates/biorouter/src/providers/llamacpp.rs);
the process manager lives in
[`crates/biorouter/src/providers/llamacpp_sidecar.rs`](../../crates/biorouter/src/providers/llamacpp_sidecar.rs).
See the [Llama Server model catalog QA checklist](../providers/llama-server/model-catalog-qa-checklist.md)
for the catalog contents and the test harness.

## Agent interaction loop

The agent operates in a continuous loop:

1. **Human request** — The user sends a message or task through the interface.
2. **Provider chat** — The agent forwards the request plus a list of available tools to the configured LLM provider.
3. **Tool call** — If the LLM decides to invoke a tool, the agent extracts the tool call (JSON) and executes it via the appropriate extension.
4. **Result feedback** — The tool result is returned to the LLM as context.
5. **Context revision** — Old or irrelevant messages are summarized or pruned to manage token usage efficiently.
6. **Final response** — Once all tool calls are complete, the LLM sends a final response to the user.

If a tool call produces an error (invalid JSON, missing tool, etc.), Biorouter captures and returns
the error to the model as a tool response, allowing the LLM to self-correct without breaking the
session. The [agent error model](../history/legacy-architecture/agent-error-model.md) explains the two-tier policy behind that
behaviour and where it is implemented.

## Configuration and data paths

| Location | Purpose |
|---|---|
| `~/.config/biorouter/config.yaml` | Primary config — providers, API keys, extensions, settings |
| `~/.config/biorouter/sessions/` | Session history (SQLite) |
| `~/.config/biorouter/workflows/` | Saved workflows |
| `~/.config/biorouter/skills/` | Biorouter-specific global skills |
| `~/Library/Application Support/Biorouter/` | Electron app state (macOS) |

The config file is shared between the Desktop UI and the CLI — changes in either interface are
reflected in both.

> **Note.** The last row is the macOS location only. The Linux and Windows equivalents — including
> `%APPDATA%\Biorouter\` and `%LOCALAPPDATA%\Biorouter\` — are listed per platform in
> [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md).

## Multi-model and multi-agent support

Biorouter supports running multiple agents in parallel:

- **Sub-agents** — A workflow can spawn sub-agents to handle parallel tasks, each with its own LLM provider and extension set.
- **Lead/Worker orchestration** — A lead model delegates sub-tasks to worker models, enabling multi-model pipelines.
- **Subworkflows** — Workflows can call other workflows as sub-tasks, running them sequentially or in parallel.

## Security posture

- Extensions are scanned for known malware before activation.
- Biorouter enforces permission modes that control whether tool calls require user approval.
- `.biorouterignore` files can restrict which files and directories the agent is allowed to access.
- Allowlists can restrict which shell commands the agent may execute.

## Project and support

| Item | Detail |
|---|---|
| Developed by | Wanjun Gu (wanjun.gu@ucsf.edu), [Baranzini Lab](https://baranzinilab.ucsf.edu/), UCSF |
| Supported by | UCSF IT and Information Commons |
| Source | [BaranziniLab/biorouter on GitHub](https://github.com/BaranziniLab/biorouter) |
| Releases | [GitHub releases](https://github.com/BaranziniLab/biorouter/releases) |

## Related documentation

- [Installation](../getting-started/installation.md) — how to get the platform described here onto a machine; the natural next step after this page.
- [Agent error model](../history/legacy-architecture/agent-error-model.md) — the error policy governing step 4 of the agent loop.
- [Extensions and skills guide](../extensions/extensions-and-skills-guide.md) — how to install and configure the extension layer.
- [Config file reference](../configuration/config-file-reference.md) — the full schema of the `config.yaml` named above.
- [Security overview](../security/README.md) — expands each bullet in the security section into an enforced mechanism.
