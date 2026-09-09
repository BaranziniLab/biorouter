<div align="center">

<img src="ui/desktop/src/images/icon.png" alt="Biorouter" width="120"/>

# UCSF Biorouter

**An AI-powered integrated research environment for biomedical discovery**

<p>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="Apache 2.0 License"></a>
  <img src="https://img.shields.io/badge/version-1.90.3-tan.svg" alt="Version 1.90.3">
</p>

<a href="https://biorouter.ucsf.edu/">Website</a> ·
<a href="https://biorouter.ucsf.edu/download">Download</a> ·
<a href="https://biorouter.ucsf.edu/docs">Docs</a> ·
<a href="https://biorouter.ucsf.edu/baam">BAAM Marketplace</a>

</div>

## What is Biorouter?

[UCSF Biorouter](https://biorouter.ucsf.edu/) is an AI-powered integrated research environment for **biomedical discovery**, built by the [Baranzini Lab](https://baranzinilab.ucsf.edu/) at UCSF. It unifies commercial, institution-hosted, and fully local LLMs together with AI agents, biomedical databases and knowledge graphs, personal knowledge bases, and customizable workflows into a single extensible tool.

Think of Biorouter as your biomedical research co-pilot — one that can read and synthesize papers, query biomedical databases and knowledge graphs (like SPOKE), explore clinical/EHR/OMOP data, build cohorts, run genomics and bioinformatics pipelines, analyze drug–disease relationships, visualize results, and carry out complex multi-step research tasks — all from one unified interface.

Biorouter runs as a desktop app, a full-screen terminal CLI, or a headless REST/WebSocket server, sharing the same agent core across all three.

## Key Features

### Bring your own model — commercial, institutional, or fully local

- **25+ built-in LLM providers** — Anthropic Claude, OpenAI GPT, Google Gemini, Amazon Bedrock, Azure OpenAI, Databricks, Ollama, and more — plus any OpenAI-compatible endpoint you add as a custom provider.
- **UCSF institution-hosted options** — **Versa API Azure** (UCSF ChatGPT) and **Versa API Bedrock** (UCSF Anthropic), listed under **Institutional Models** in the provider grid, for compliant access on sensitive research.
- **Zero-setup local models** — a bundled **Llama Server** (llama.cpp sidecar) ships a curated Qwen/Gemma catalog with one-click download and runs entirely on your machine; Ollama is also supported. Ideal for **air-gapped, private inference** where no data leaves your device.

### Biomedical agents & the MCP extension ecosystem

- **Model Context Protocol (MCP)** — connect Biorouter to biomedical databases, web tools, file systems, and APIs through pluggable extensions, and install third-party agents.
- **Biomedical agents via the BAAM marketplace** ([biorouter.ucsf.edu/baam](https://biorouter.ucsf.edu/baam)) — including **SPOKEAgent** (the SPOKE biomedical knowledge graph), the **UCSF OMOP Agent** and **CDWAgent** (clinical/EHR/OMOP data and cohort building), plus a growing library of bioinformatics and clinical skills (ATAC-seq, ChIP-seq, alternative splicing, causal genomics, chemoinformatics, clinical biostatistics, and more).
- **Built-in extensions** — on by default: Developer (shell, files, code execution), Computer Controller (web/computer automation), Auto Visualiser, Memory, Agent Drafter, and Knowledge.

### Personal, LLM-maintained knowledge bases

- Build personal knowledge bases backed by **markdown trees + git history** that an LLM curates as it ingests.
- **Ingest** papers and documents from **PDF, HTML, DOCX, PowerPoint, CSV/XLSX, and URLs**.
- **Source credibility classification** via Crossref / OpenAlex, a **knowledge graph view** of cross-linked pages, **BM25 search**, full change history, and **`.brkb` export/import** to share a base.

### Auto Visualiser — publication-ready figures in chat

Turn structured data into self-contained, interactive HTML figures rendered inline in chat — **33 tools** spanning scientific plots (**volcano**, **Manhattan**, **Kaplan–Meier**, **forest**), charts (histogram, box, bubble, area, radar, donut, gauge), relationships and hierarchies (network, Sankey, chord, heatmap, treemap, sunburst, dendrogram, word cloud, calendar heatmap), diagrams (Mermaid flowchart/gantt/sequence/mindmap/timeline/ER/state/class), and geographic maps (Leaflet map, choropleth).

### Agent Drafter — apps the agent builds, then drives

Ask for a tool and the agent builds a small **Biorouter app**: a TypeScript front-end wired to its own per-app agent. It doesn't just answer inside the app — it drives it, rendering panels, charts and graphs into the running page and asking you questions mid-task. A finished app can be exported as a directly runnable bundle. Agent Drafter is a built-in capability, on by default. See the [Apps SDK reference](docs/apps-sdk/sdk-reference.md).

### Run several chats at once

- **Workspace control** — lay work out across tabs, panes and windows: a second chat for the QC pass while the first writes the methods, each with its own working directory, extensions and history.
- **Delegate to subagents** — hand a job to a child chat you can read, steer and stop, with the parent waiting on it properly instead of polling.
- **Reconfigure another chat from this one** — hand a chat a skill or an extension, behind a confirmation card.

Two gates: the full surface is an **opt-in extension** (Extensions → **Workspace Control**, or `biorouter configure` → Toggle Extensions → `workspace`), and delegation is offered only in the **Completely Autonomous** permission mode. See [Workspace control](docs/agent-loop/workspace-control.md).

### Computer Controller & vision

- Drive the web and your computer for research automation, with **multi-monitor screen capture** (enumerate displays and re-capture any screen by index) and **vision input** so the model can read screenshots and figures.

### Workflows, skills & automation

- **Workflows** — package any multi-step task into a shareable, reusable file with Jinja-style templating; compose **sub-workflows**.
- **Scheduling** — run workflows and agent automations on a **cron schedule**, unattended.
- **Skills** — teach Biorouter your lab's reusable instruction sets and best practices; built-in authoring skills (`develop-biorouter-extension`, `develop-biorouter-skill`) help you create your own.
- **Lifecycle hooks** — fire custom commands at `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, and `SessionEnd` for logging, policy, and automation.
- **Agent goals & ACP** — agents track goals across a chat, and the Agent Communication Protocol enables multi-agent orchestration.

### Three surfaces, one core

- **Desktop app** (Electron + React) for interactive work.
- **`biorouter` CLI** — a full-screen TUI at parity with the GUI: slash-command palette, model/provider setup, knowledge bases, and extension/skill/workflow install, all from the terminal. `biorouter doctor` checks prerequisites and flags when a newer release is available.
- **`biorouterd` server** — REST + WebSocket API with an OpenAPI-generated TypeScript client.

### Built for the desktop

- **One-click "Restart & Update"** on macOS (electron-updater) — downloads in the background and swaps the app in place, leaving your settings, sessions, and knowledge bases untouched.
- **Native installers** for macOS (Apple Silicon + Intel), Windows, and Linux (deb/rpm), plus **headless CLI-only Linux packages** for servers, HPC nodes, and containers.
- **Secret storage** via the OS keychain (macOS Keychain / Windows Credential Manager / Linux Secret Service), configurable **permission modes**, and a **`.biorouterignore`** to keep sensitive files out of the agent's reach.

## Download

Native installers for all major platforms are available in every release:

| Platform | Package |
|----------|---------|
| **macOS** (Apple Silicon) | `Biorouter-*-arm64.dmg` — open and drag to `/Applications` |
| **macOS** (Intel) | `Biorouter-*-x64.dmg` — open and drag to `/Applications` |
| **Windows** (x64) | `Biorouter-win32-x64-*.zip` — unzip and run `Biorouter.exe` |
| **Linux** Ubuntu / Pop!_OS (x64) | `biorouter_*_amd64.deb` — `sudo dpkg -i biorouter_*.deb` |
| **Linux** Fedora / RHEL (x64) | `Biorouter-*-1.x86_64.rpm` — `sudo rpm -i Biorouter-*.rpm` |
| **Linux — CLI only** Debian/Ubuntu (x64) | `biorouter-cli_*_amd64.deb` — `sudo apt install ./biorouter-cli_*.deb` |
| **Linux — CLI only** Fedora/RHEL (x64) | `biorouter-cli-*-1.x86_64.rpm` — `sudo dnf install ./biorouter-cli-*.rpm` |

**[Download Biorouter →](https://biorouter.ucsf.edu/download)** or grab assets from the [Releases page](https://github.com/BaranziniLab/biorouter/releases).

The `biorouter` command-line tool ships **inside** the desktop app. On macOS and Windows, install the app above, then accept the in-app "Install Biorouter CLI" prompt (or run `biorouter setup-path`) to add the bundled `biorouter` binary to your `PATH`. On Linux you can install the CLI on its own with the headless `biorouter-cli` package (`biorouter-cli_*_amd64.deb` or `biorouter-cli-*-1.x86_64.rpm`) — no desktop app required.

Always install the newest version for the latest features and fixes.

## Getting Started in 3 Steps

**1. Download and install** Biorouter for your platform from the table above.

**2. Connect a model** — on first launch, Biorouter walks you through choosing a provider:
- **UCSF users** — under **Institutional Models**, select **Versa API Azure** (UCSF ChatGPT) or **Versa API Bedrock** (UCSF Anthropic). These are *not* the generic "Azure OpenAI" and "Amazon Bedrock" cards, which are the commercial bring-your-own-credentials providers.
- **Your own API key** — enter your Anthropic, OpenAI, or Google key directly.
- **Fully local** — pick the bundled Llama Server (zero setup) or install [Ollama](https://ollama.com); no API key, no data leaves your device.

**3. Start exploring** — ask a research question, ingest papers into a knowledge base, query SPOKE, build a cohort, or load a workflow. Biorouter takes it from there.

## Who is Biorouter For?

- **Bench and computational researchers** analyzing data, reviewing literature, and running genomics/bioinformatics pipelines with AI assistance.
- **Clinical researchers and data scientists** who need secure, institution-compliant AI access for sensitive EHR/OMOP and cohort work.
- **Labs and teams** sharing reusable AI workflows, skills, and knowledge bases across their group.

## Working with Sensitive Data

Biorouter routes your inputs to an LLM provider, so the privacy of a chat depends on which model that chat is using. This used to be advice. Biorouter now **enforces** a version of it — and the limits below matter as much as the rules, because it is a guardrail against mistakes, not a wall against a determined attacker.

Throughout, a **private model** means one your institution hosts (**Versa API Azure**, **Versa API Bedrock**) or one that runs on your own machine (the bundled **Llama Server**, or **Ollama**). Everything else — Anthropic, OpenAI, Google, generic Azure OpenAI, generic Amazon Bedrock — is public.

**A chat remembers where it has been.** Run a turn on a private model, or touch a private data source (the UCSF OMOP or CDW connector, a knowledge base marked private), and the chat is marked **private** from then on. The mark is a one-way ratchet: it goes up on its own and never comes down on its own, and a private chat cannot afterwards be switched to a public model. Starting a *new* chat on the public model is always available and is the intended way through — the boundary is the transcript, not the model. Undoing the mark on an existing chat is a deliberate, recorded act (Settings, or `biorouter session declassify <id>`), and for anything but a chat Biorouter watched run a private turn it also asks for your operating-system password.

**Which institution, not just how sensitive.** HIPAA compliance is established per data flow and does **not** transfer between institutions, so "both ends are private" is not enough. UCSF's Versa reaching UCSF's OMOP or CDW connector is the approved arrangement and passes quietly. The same model reaching *another* site's private connector is a mismatch: Biorouter flags it, and depending on your setting you can accept it deliberately (recorded) or have it refused outright. A **local** model is the exception that reaches everything private — nothing is disclosed to anyone, so there is no agreement to be outside of.

**What a public model is mechanically stopped from doing.** It cannot obtain another chat's private content — not through chat recall, not by ingesting another chat, not through the Workspace Control tools, and not by reading a knowledge base marked private; a refusal says so instead of quietly returning less. It cannot **see, call or attach** the clinical connectors: they are filtered out of its tool list, a call is refused rather than prompted, and attaching one to a public chat is declined with no "approve anyway" — not even for you at the keyboard. And it cannot promote itself into private capability: it cannot spawn a subagent on a private model to fetch private material on its behalf, and raising a chat's tier is a user action rather than something the agent does for itself.

⚠ **What is not stopped.** **There is no general filesystem barrier in v1.** A public model that you have given shell access can read ordinary files on this computer — including files an earlier private chat wrote outside Biorouter's own storage, and the session database at `~/.config/biorouter/sessions/sessions.db`, which is not encrypted. And a connector's private mark is not tamper-proof: **two file edits** (renaming its entry in `config.yaml` and deleting `extension-provenance.json`) untag it, after which a public model can query it like any other tool. Nothing rebuilds, nothing reinstalls. That last one is a difference in kind, not degree — it is live access to clinical data from a public model, the one thing this system is otherwise good at refusing. Treat all of this as protection against forgetting which model you are on, not against an agent following instructions hidden in a document it was asked to read. These are the gaps you are most likely to meet, not the complete list; [the full accounting](docs/security/data-privacy-and-phi.md#the-provider-guidance-on-this-page-is-now-enforced) names the rest, including the Workspace Control write paths.

**You are told before, not after.** The first time a chat binds a model that is not private, Biorouter shows you what that model can reach, and keeps a short form of it on the model chip afterwards. **The disclosure appears whether or not privacy tiers are enabled** — turning the feature off in Settings removes the enforcement, not the exposure.

**Your existing chats.** Upgrading runs a one-time backfill that marks each old chat by **the model it was last using** — it reads no message bodies, so a chat that ran on Versa and later moved to a commercial model stays public, and a chat that ran one Ollama turn becomes private. Biorouter shows you the real counts from your own database once, before enforcement starts biting. See [what happens to your existing chats](docs/security/privacy-tiers-migration.md).

**And two things the app cannot do for you:**

- **Do not** use personal commercial API keys with patient data. In particular, the generic **Azure OpenAI** and **Amazon Bedrock** cards are the commercial providers, not the UCSF institutional ones — Biorouter can refuse a *chat* the wrong model, but it cannot make a commercial account an approved place for PHI.
- **Always verify** with your institution's compliance office before processing sensitive data. Biorouter is a tool you run against approved services; it is not itself a HIPAA-compliant service, and none of the above makes it one.

See the [Data Privacy Guide](docs/security/data-privacy-and-phi.md) for full details, and [Privacy tiers](docs/security/privacy-tiers.md) for how the enforcement is built.

## Documentation

Full documentation lives at [biorouter.ucsf.edu/docs](https://biorouter.ucsf.edu/docs) and in the [docs/](docs/) folder:

| Guide | Description |
|---|---|
| [Architecture](docs/architecture/system-overview.md) | How Biorouter is built — backend, frontend, agent loop |
| [Providers & Models](docs/getting-started/choosing-a-model-provider.md) | The commercial provider catalogue and how to switch — it does not yet cover the UCSF institutional or the local providers, which you configure in Settings > Models |
| [Extensions, Skills & MCP](docs/extensions/extensions-and-skills-guide.md) | Adding tools, agents, and reusable skills |
| [Workflows](docs/workflows/README.md) | Creating and sharing automated workflows |
| [Schedulers](docs/workflows/scheduled-jobs.md) | Running workflows on a schedule |
| [Hooks](docs/agent-loop/hooks/hooks-reference.md) | Lifecycle hooks for logging, policy, and automation |
| [Workspace Control](docs/agent-loop/workspace-control.md) | Running several chats at once and delegating to subagents |
| [Permission modes](docs/security/permission-modes.md) | The four autonomy modes and how to switch them |
| [Managed enterprise policy](docs/security/managed-policy.md) | Admin-owned policy that overrides user config for permissions and hooks |
| [Secret Storage](docs/security/secret-storage.md) | How credentials are kept in your OS keychain |
| [Installation & Setup](docs/getting-started/installation.md) | Step-by-step setup guide |
| [Data Privacy](docs/security/data-privacy-and-phi.md) | Guidelines for handling patient and sensitive data |

## Security, acceptable use & contributing

Report a suspected vulnerability privately per [SECURITY.md](SECURITY.md) — please do not open a public issue for one. Usage terms are in [ACCEPTABLE_USAGE.md](ACCEPTABLE_USAGE.md); how to contribute is in [CONTRIBUTING.md](CONTRIBUTING.md) and [GOVERNANCE.md](GOVERNANCE.md).

## Acknowledgments

Biorouter's agentic environment was built on the foundation of, and with reference to, the following open-source AI tools — we are grateful to their authors and communities:

- **[Goose](https://block.github.io/goose/)** — CLI/Desktop agent for full developer workflows (Block) — Biorouter's primary upstream foundation
- **[Aider](https://aider.chat/)** — open-source, Git-native CLI AI coding agent
- **[Cline](https://github.com/cline/cline)** — open-source interactive CLI coding agent
- **[OpenCode](https://opencode.ai/)** — open-source coding agent with multi-session and multi-provider support
- **[ForgeCode](https://forgecode.dev/)** — terminal AI assistant for task planning and code generation

## Citation

If you use Biorouter in your research, please cite:

```bibtex
@software{biorouter2025,
  title  = {UCSF Biorouter: An AI-Powered Integrated Research Environment},
  author = {Gu, Wanjun and Bellucci, Gianmarco and Baranzini, Sergio E.},
  year   = {2025},
  url    = {https://github.com/BaranziniLab/biorouter}
}
```

## About

UCSF Biorouter is developed by **Wanjun Gu** ([wanjun.gu@ucsf.edu](mailto:wanjun.gu@ucsf.edu)) at the [Baranzini Lab](https://baranzinilab.ucsf.edu/), Department of Neurology, UCSF Bakar Computational Health Sciences Institute. Development is supported by **UCSF IT** and **Information Commons**.

Licensed under the [Apache License 2.0](LICENSE).

<div align="center">
  <p>
    <a href="https://github.com/BaranziniLab/biorouter/releases">Download</a> ·
    <a href="docs/getting-started/installation.md">Setup Guide</a> ·
    <a href="https://github.com/BaranziniLab/biorouter/issues">Report an Issue</a> ·
    <a href="mailto:wanjun.gu@ucsf.edu">Contact</a>
  </p>
</div>
