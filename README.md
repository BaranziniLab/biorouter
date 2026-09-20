<div align="center">

<img src="ui/desktop/src/images/icon.png" alt="Biorouter" width="120"/>

# UCSF Biorouter

**An AI-powered integrated research environment for biomedical discovery**

<p>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="Apache 2.0 License"></a>
  <img src="https://img.shields.io/badge/version-1.91.0-tan.svg" alt="Version 1.91.0">
</p>

<a href="https://biorouter.ucsf.edu/">Website</a> ·
<a href="https://biorouter.ucsf.edu/download">Download</a> ·
<a href="https://biorouter.ucsf.edu/docs">Docs</a> ·
<a href="https://biorouter.ucsf.edu/baam">BAAM Marketplace</a>

</div>

## What is Biorouter?

[UCSF Biorouter](https://biorouter.ucsf.edu/) is an AI-powered integrated research environment for **biomedical discovery**, built by the [Baranzini Lab](https://baranzinilab.ucsf.edu/) at UCSF. It brings commercial, institution-hosted and fully local language models together with AI agents, biomedical databases and knowledge graphs, personal knowledge bases, and customizable workflows, in one extensible tool.

Biorouter can read and synthesize papers, query biomedical databases and knowledge graphs such as SPOKE, explore clinical, EHR and OMOP data, build cohorts, run genomics and bioinformatics pipelines, analyze drug and disease relationships, visualize results, and carry out multi-step research tasks.

It runs three ways on the same agent core: a desktop app, a full-screen terminal CLI, and a server you reach from a browser.

## Key features

### Bring your own model: commercial, institutional, or fully local

- **25+ built-in providers.** Anthropic Claude, OpenAI GPT, Google Gemini, Amazon Bedrock, Azure OpenAI, Databricks, Ollama and more, plus any OpenAI-compatible endpoint you add as a custom provider.
- **UCSF institution-hosted options.** **Versa API Azure** (UCSF ChatGPT) and **Versa API Bedrock** (UCSF Anthropic), listed under Institutional Models, for compliant access on sensitive research.
- **Zero-setup local models.** A bundled **Llama Server** (a llama.cpp sidecar) ships a curated Gemma and Qwen catalog with one-click download and runs entirely on your machine. Ollama is also supported. Nothing leaves your device.
- **Your existing coding-agent subscription.** If you already use the Claude Code or Codex command-line tools, Biorouter can run inference through them on your own vendor plan. Biorouter never sees the credential.

### Biomedical agents and the MCP extension ecosystem

- **Model Context Protocol.** Connect Biorouter to biomedical databases, web tools, file systems and APIs through pluggable extensions, and install third-party agents.
- **Biomedical agents from the BAAM marketplace** ([biorouter.ucsf.edu/baam](https://biorouter.ucsf.edu/baam)), including **SPOKEAgent** for the SPOKE biomedical knowledge graph, and the **UCSF OMOP Agent** and **CDWAgent** for clinical, EHR and cohort work, plus a library of bioinformatics and clinical skills.
- **Seven built-in extensions**, all on in a fresh install. See [Built-in MCP servers](#built-in-mcp-servers) below for what each one provides.

### Personal, LLM-maintained knowledge bases

- Knowledge bases backed by **markdown trees and git history** that a model curates as it ingests.
- **Ingest** papers and documents from PDF, HTML, DOCX, PowerPoint, CSV, XLSX and URLs.
- **Source credibility classification** via Crossref and OpenAlex, a knowledge-graph view of cross-linked pages, BM25 search, full change history, and `.brkb` export and import to share a base.

### Auto Visualiser: publication-ready figures

Turn structured data into self-contained, interactive HTML figures. The model calls **three tools** (`render_figure`, `describe_figure`, `render_dashboard`) covering **32 figure kinds**: scientific plots (volcano, Manhattan, Kaplan-Meier, forest), charts (histogram, box, bubble, area, radar, donut, gauge), relationships and hierarchies (network, Sankey, chord, heatmap, treemap, sunburst, dendrogram, word cloud, calendar heatmap), Mermaid diagrams (flowchart, gantt, sequence, mindmap, timeline, ER, state, class) and geographic maps.

`render_dashboard` combines any number of those into one scrollable report with a masthead, contents and numbered figure captions, instead of leaving you to open figures one at a time.

Figures open in the **artifact side panel**, from a click-to-open card in the transcript. Nothing renders inline in the conversation.

### Agent Drafter: apps the agent builds, then drives

Ask for a tool and the agent builds a small **Biorouter app**: a TypeScript front end wired to its own per-app agent. The agent does not just answer inside the app, it drives it, rendering panels, charts and graphs into the running page and asking you questions mid-task. A finished app exports as a directly runnable bundle. See the [Apps SDK reference](docs/apps-sdk/sdk-reference.md).

### Run several chats at once

- **Workspace control.** Lay work out across tabs, panes and windows: a second chat for the QC pass while the first writes the methods, each with its own working directory, extensions and history.
- **Delegate to subagents.** Hand a job to a child chat you can read, steer and stop, with the parent waiting on it properly instead of polling.
- **Reconfigure another chat from this one**, behind a confirmation card.

⚠ Workspace control is a **capability that ships on**, and its surface includes reading and steering your other conversations. Delegation to subagents is narrower: it is offered only in Completely Autonomous mode, and a subagent cannot spawn its own. A write from a private chat into a different chat raises an approval showing the payload. See [Workspace control](docs/agent-loop/workspace-control.md).

### Workflows, skills and automation

- **Workflows.** Package a multi-step task into a shareable file with Jinja-style templating, and compose sub-workflows.
- **Scheduling.** Run workflows and agent automations on a cron schedule, unattended.
- **Skills.** Teach Biorouter your lab's reusable instruction sets. Install them from a URL, a zip or the marketplace.
- **Lifecycle hooks.** Fire custom commands at `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop` and `SessionEnd` for logging, policy and automation.

### Three surfaces, one core

- **Desktop app** (Electron and React) for interactive work.
- **`biorouter` CLI**, a full-screen terminal interface at parity with the app: slash-command palette, model and provider setup, knowledge bases, and extension, skill and workflow installs. `biorouter doctor` checks prerequisites and can fix several of them with `--fix`.
- **`biorouter serve`**, the same interface in a browser. See [Running Biorouter from a browser](#running-biorouter-from-a-browser).

Underneath all three is `biorouterd`, a REST and WebSocket server with an OpenAPI-generated TypeScript client.

## Computer Use

Biorouter can observe and control the desktop of the machine running its backend, so an agent can drive applications that have no API: a licensed analysis package, an imaging viewer, an institutional client with no scripting interface.

**Ten tools.** Three only observe (list running applications, inspect one application's windows and accessibility tree, capture a display or window). Seven act: click, secondary click, scroll, drag, type text, press keys, and set a control's value. `screen_capture` handles multiple monitors, and can inventory displays and windows without taking an image.

**The agent cannot act blind.** An acting tool is refused unless the agent has just inspected that same application. Inspecting a different one does not count.

**Consent is per request, and it names what matters.** Before any observation or action, you are asked once for that request. The prompt names the provider and model, where that model's data goes, which computer is being controlled (by hostname and operating system), and what the model will be able to do. When the bound model is public, the prompt says plainly that screenshots, window information and application text may be sent to that provider, and warns you about anything another person or task left on screen. The grant covers that one request and expires when the reply finishes.

Four things follow from how that consent is built, and they are the reason it is worth trusting:

- **Turning the capability on is not consent.** It ships enabled and grants nothing by itself. Approval is refused unless a live request is actually waiting to use the computer.
- **Only the person at the chat can approve.** A model cannot, an API key cannot, and the daemon's own secret cannot, because desktop approval uses a different proof from ordinary API authentication.
- **One task holds the desktop at a time**, machine-wide, through an exclusive lock. A second Biorouter process is told to stop rather than share control.
- **The approval does not survive a change of model.** If the provider or model changes during a request, you are asked again.

**Chat Only mode switches it off outright.** The other three permission modes allow it, still subject to the per-request approval.

**Platforms.** macOS on Apple Silicon and Intel (macOS 14 or later), Windows x64, and Linux x64 and arm64. macOS needs Accessibility and Screen Recording permission, Windows needs an interactive signed-in desktop, and Linux needs a desktop session plus the AT-SPI and GTK packages the deb and rpm declare. Where a desktop or a permission is missing, Biorouter reports a named state such as "no desktop session" or "OS permission required" rather than hanging.

The native helper is **bundled with the app, not downloaded at runtime**. It is pinned to one upstream version and commit, every file is checksummed before it runs, and a payload that does not match is refused. The macOS helper is signed with UCSF's Developer ID and the release pipeline refuses an unsigned one.

The extension's configuration key is still `computercontroller`, its original name, so existing configs and command lines keep working.

## Built-in MCP servers

Seven servers ship inside Biorouter and are enabled in a fresh install.

| Extension | Tools |
|---|---|
| **Developer** | `text_editor`, `shell`, `shell_status`, `shell_kill`, `analyze`, `image_processor` |
| **Computer Use** | The ten desktop tools above |
| **Web & Documents** | `web_scrape`, `xlsx_tool`, `docx_tool`, `pdf_tool`, `cache` |
| **Auto Visualiser** | `render_figure`, `describe_figure`, `render_dashboard` |
| **Memory** | `remember_memory`, `retrieve_memories`, `remove_memory_category`, `remove_specific_memory` |
| **Knowledge** | Around two dozen `kb_*` tools: search, page read and write, base management, plus the ingest, query and lint macros |
| **Agent Drafter** | Build, configure, preview and export Biorouter apps |

Five of them also run as standalone MCP servers over stdio, so another MCP client can use them:

```bash
biorouter mcp developer        # also: computercontroller, webdocuments, autovisualiser, memory
```

Knowledge and Agent Drafter have no standalone name; they run only inside the daemon. A further set (`datasql`, `files`, `compute`, `appcontrol`, `evidence`) is injected per Biorouter app, only when that app declares the matching capability.

Third-party extensions install from the [BAAM marketplace](https://biorouter.ucsf.edu/baam), a URL or a local path, in any of four transports: Server-Sent Events, a stdio subprocess, streamable HTTP, or a frontend-hosted server.

## Running Biorouter from a browser

```bash
biorouter serve
```

This starts the daemon, points it at the built web interface and prints a URL. It is the full Biorouter interface, not a proxy or a cut-down chat page: the daemon serves the same application the desktop renderer uses, on its own origin. Useful on a lab server, an HPC login node or a container, where installing a desktop app is not an option. `biorouter headless` is the same command.

**Access.** It binds loopback on port 8765 by default, so nothing outside the machine can reach it. Serving on an address other machines can reach requires an access token, and `--no-token` is refused for a non-loopback bind before anything starts. The token in the printed URL is exchanged once for an `HttpOnly`, `SameSite=Strict` cookie that gates the page; from then on the page authenticates each API call with the daemon's secret key, the same way the desktop app does. The cookie is never accepted on an API route, so there is no cross-site request forgery surface. The token keeps working while the daemon runs, so a bookmark or a second browser still opens.

Stopping `serve` stops and reaps the daemon, which is the only way to revoke the URL it printed.

**Three things a browser session deliberately cannot do**, because the daemon it talks to has no way to tell a person from a model:

- Change its own model or provider. The model is whatever `biorouter configure` chose on the host.
- Grant an approval that has to be proved to come from a human, which includes credential entry. The refusal is a sentence written for a person, not a silent failure.
- Steer a turn that is already running. Stopping one works.

The interface knows these limits before you reach them and explains them in place.

For servers and HPC nodes, every release ships two CLI-only Linux packages (`biorouter-cli_<ver>_amd64.deb` and `biorouter-cli-<ver>-1.x86_64.rpm`) containing `biorouter`, `biorouterd` and the browser interface bundle, with no desktop app.

## What Biorouter enforces

Some of this used to be advice in a document. It is now enforced by the program, and the limits matter as much as the rules, because this is a guardrail against mistakes rather than a wall against a determined attacker.

**Four permission modes.** Autonomous approves every tool with no prompt. Manual prompts for everything. Smart auto-allows what it grades read-only and prompts otherwise. Chat only dispatches no tools at all. Underneath all four, a small denylist of catastrophic commands (`rm -rf /`, disk wipes, fork bombs) is a hard block that no configuration can turn off. In Autonomous mode a short list of extremely sensitive operations, such as writing to a system directory or recursively deleting an established directory, is still raised for approval.

**The secret guard is always on.** Any tool call whose arguments reach a credential file (`.env`, `*.pem`, `id_rsa`, `~/.aws/credentials`, `secrets.*`, a coding agent's stored token) is refused, in every chat, mode and privacy tier. It resolves a command the way a shell would, expanding `~`, variables, globs, `cd`, nested `sh -c` and here-documents, before deciding, and it refuses whether or not the file exists. Tool output is scanned on the way back too, so credential material never reaches the model. A `.biorouterignore` negation is the way to re-open one specific file.

**Your chat transcripts are not readable by tools.** The session database is refused by name at three separate doors, and the refusal is about the channel, not about which model you are on.

See [Permission modes](docs/security/permission-modes.md) and the [secret guard](docs/security/secret-guard.md).

## Working with sensitive data

Biorouter sends your inputs to a language model, so the privacy of a chat depends on which model that chat is using.

**What "private" means here is the endpoint, not the brand.** Exactly four providers can be private: the bundled Llama Server, Ollama, Versa API Azure and Versa API Bedrock. Everything else is public, including Anthropic, OpenAI, Google, the generic Azure OpenAI and Amazon Bedrock cards, and the Claude Code and Codex providers. That last pair matters because it is easy to misread: the command-line tool runs locally, but inference runs on the vendor's consumer subscription, which carries no BAA.

A local model is private **only while its address is actually this machine**. Pointing `OLLAMA_HOST` or `LLAMACPP_EXTERNAL_HOST` at another box makes it someone else's server, and Biorouter treats it as public from then on. An institutional model is private only while its endpoint is UCSF's gateway, for the same reason.

**A chat remembers where it has been.** Run a turn on a private model, or touch a private data source, and the chat is marked private from then on. The mark only goes up: the database update physically cannot lower it, whatever the caller passes, and a private chat cannot afterwards be bound to a public model. Starting a new chat on a public model is always available and is the intended way through, because the boundary is the transcript, not the model. Undoing the mark is deliberate and recorded, and for any chat other than one Biorouter watched run a private turn it takes three proofs, including your operating-system password.

**Which institution, not only how sensitive.** HIPAA compliance is established per data flow and does not transfer between institutions, so "both ends are private" is not sufficient. UCSF's Versa reaching UCSF's OMOP or CDW connector is the approved arrangement and passes quietly. The same model reaching another site's private connector is flagged, and you can accept it deliberately or have it refused, depending on your setting. A local model reaches everything, because nothing is disclosed to anyone.

**What a public model is mechanically stopped from doing.** It cannot reach another chat's private content, through chat recall, conversation ingest, the workspace tools or a private knowledge base; a refusal says so rather than quietly returning less. It cannot see, call or attach the two UCSF clinical connectors: their tools are filtered out before the model is offered them, a call is refused rather than prompted, and attaching one to a public chat is declined with no override, not even for you at the keyboard. And it cannot promote itself, by spawning a private subagent or by raising its own chat's tier.

⚠ **What is not stopped.** **There is no general filesystem barrier.** A public model you have given shell access can read ordinary files on this computer, including files an earlier private chat wrote outside Biorouter's own storage. This was descoped rather than forgotten, which is why the disclosure Biorouter shows you says so in as many words. Treat all of this as protection against forgetting which model you are on, not against an agent following instructions hidden in a document it was asked to read. [The full accounting](docs/security/data-privacy-and-phi.md) names the rest.

**You are told before, not after.** Before a chat first binds a model that is not private, Biorouter shows you what that model can reach, and keeps a short form of it on the model chip. The disclosure is shown once per installation, not once per chat, and it appears whether or not privacy enforcement is switched on, because turning the feature off removes the enforcement and not the exposure.

**Two things the app cannot do for you:**

- Do not use personal commercial API keys with patient data. Biorouter can refuse a chat the wrong model. It cannot make a commercial account an approved place for PHI.
- Verify with your institution's compliance office before processing sensitive data. Biorouter is a tool you run against approved services. It is not itself a HIPAA-compliant service, and none of the above makes it one.

See the [data privacy guide](docs/security/data-privacy-and-phi.md) and [privacy tiers](docs/security/privacy-tiers.md).

## Download

| Platform | Package |
|----------|---------|
| **macOS** (Apple Silicon) | `Biorouter-*-arm64.dmg`, open and drag to `/Applications` |
| **macOS** (Intel) | `Biorouter-*-x64.dmg`, open and drag to `/Applications` |
| **Windows** (x64) | `Biorouter-Setup-*.exe`, an installer that also upgrades an existing install |
| **Windows** (x64, portable) | `Biorouter-win32-x64-*.zip`, unzip and run `Biorouter.exe` |
| **Linux** Ubuntu and Pop!_OS (x64) | `biorouter_*_amd64.deb`, `sudo dpkg -i biorouter_*.deb` |
| **Linux** Fedora and RHEL (x64) | `Biorouter-*-1.x86_64.rpm`, `sudo rpm -i Biorouter-*.rpm` |
| **Linux, CLI only** Debian and Ubuntu | `biorouter-cli_*_amd64.deb`, `sudo apt install ./biorouter-cli_*.deb` |
| **Linux, CLI only** Fedora and RHEL | `biorouter-cli-*-1.x86_64.rpm`, `sudo dnf install ./biorouter-cli-*.rpm` |

**[Download Biorouter](https://biorouter.ucsf.edu/download)**, or take assets from the [releases page](https://github.com/BaranziniLab/biorouter/releases).

macOS updates in place from inside the app. On Windows the updater downloads the installer and hands it to you. On Linux it downloads the package for your package manager.

The `biorouter` command-line tool ships inside the desktop app. On macOS and Windows, install the app and then accept the in-app "Install Biorouter CLI" prompt, or run `biorouter setup-path`. On Linux you can install the CLI on its own with the `biorouter-cli` package, with no desktop app.

## Getting started

**1. Install** Biorouter for your platform from the table above.

**2. Connect a model.** On first launch Biorouter walks you through it:
- **UCSF users:** under Institutional Models, choose **Versa API Azure** (UCSF ChatGPT) or **Versa API Bedrock** (UCSF Anthropic). These are not the generic "Azure OpenAI" and "Amazon Bedrock" cards, which are the commercial bring-your-own-credentials providers.
- **Your own API key:** enter your Anthropic, OpenAI or Google key.
- **Fully local:** choose the bundled Llama Server, which needs no setup, or install [Ollama](https://ollama.com). No API key, and nothing leaves your device.

**3. Start.** Ask a research question, ingest papers into a knowledge base, query SPOKE, build a cohort, or load a workflow.

## Who Biorouter is for

- **Bench and computational researchers** analyzing data, reviewing literature and running genomics and bioinformatics pipelines.
- **Clinical researchers and data scientists** who need institution-compliant AI access for sensitive EHR, OMOP and cohort work.
- **Labs and teams** sharing reusable workflows, skills and knowledge bases.

## Documentation

Full documentation is at [biorouter.ucsf.edu/docs](https://biorouter.ucsf.edu/docs) and in [docs/](docs/).

| Guide | Description |
|---|---|
| [Architecture](docs/architecture/system-overview.md) | Backend, frontend and the agent loop |
| [Providers and models](docs/getting-started/choosing-a-model-provider.md) | The provider catalogue and how to switch |
| [Extensions, skills and MCP](docs/extensions/extensions-and-skills-guide.md) | Adding tools, agents and reusable skills |
| [Computer Use](docs/extensions/built-in/computer-controller.md) | Desktop control, consent and platform setup |
| [Browser access](docs/deployment/browser-access.md) | Running Biorouter from a browser with `biorouter serve` |
| [Workflows](docs/workflows/README.md) | Creating and sharing automated workflows |
| [Scheduled jobs](docs/workflows/scheduled-jobs.md) | Running workflows on a schedule |
| [Hooks](docs/agent-loop/hooks/hooks-reference.md) | Lifecycle hooks for logging and policy |
| [Workspace control](docs/agent-loop/workspace-control.md) | Several chats at once, and subagents |
| [Permission modes](docs/security/permission-modes.md) | The four modes and how to switch them |
| [Secret guard](docs/security/secret-guard.md) | The always-on credential floor |
| [Managed enterprise policy](docs/security/managed-policy.md) | Admin-owned policy that overrides user config |
| [Secret storage](docs/security/secret-storage.md) | How credentials are kept in your OS keychain |
| [Installation and setup](docs/getting-started/installation.md) | Step-by-step setup |
| [Data privacy](docs/security/data-privacy-and-phi.md) | Handling patient and sensitive data |

## Security, acceptable use and contributing

Report a suspected vulnerability privately per [SECURITY.md](SECURITY.md). Please do not open a public issue for one. Usage terms are in [ACCEPTABLE_USAGE.md](ACCEPTABLE_USAGE.md), and how to contribute is in [CONTRIBUTING.md](CONTRIBUTING.md) and [GOVERNANCE.md](GOVERNANCE.md).

## Built on the work of others

Biorouter is a fork of **[Goose](https://github.com/block/goose)**, Block, Inc.'s open-source agent, and that lineage is still visible in the code. Goose supplied the agent loop, the extension model and the desktop shell that Biorouter's biomedical work is built on. It is licensed under Apache 2.0, and the root [LICENSE](LICENSE) carries Block's copyright alongside ours.

Biorouter also redistributes these, with their licences and notices shipped alongside:

| Component | Licence | What it does here |
|---|---|---|
| [Open Computer Use](https://github.com/iFurySt/open-codex-computer-use) | MIT | The native desktop helper behind Computer Use. Vendored into this repository at a pinned commit, so a build needs no network and no upstream change can affect a release. |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) | MIT | `llama-server`, the bundled local model runtime. |
| [D3](https://d3js.org/), [d3-sankey](https://github.com/d3/d3-sankey), [Chart.js](https://www.chartjs.org/), [Leaflet](https://leafletjs.com/), [Leaflet.markercluster](https://github.com/Leaflet/Leaflet.markercluster), [Mermaid](https://mermaid.js.org/) | MIT, ISC and BSD | Compiled into the binary and inlined into every Auto Visualiser figure, so figures work offline. |

The Rust side builds on the official Rust MCP SDK (`rmcp`), Axum, `sqlx` with SQLite, `tiktoken-rs`, `minijinja` and the `keyring` crate. The desktop app is Electron with React 19. Attribution for everything redistributed is collected in [NOTICE](NOTICE).

Biorouter's design also drew on [Aider](https://aider.chat/), [Cline](https://github.com/cline/cline), [OpenCode](https://opencode.ai/) and [ForgeCode](https://forgecode.dev/). We are grateful to their authors and communities.

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

UCSF Biorouter is developed by **Wanjun Gu** ([wanjun.gu@ucsf.edu](mailto:wanjun.gu@ucsf.edu)) at the [Baranzini Lab](https://baranzinilab.ucsf.edu/), Department of Neurology, UCSF Bakar Computational Health Sciences Institute. Development is supported by UCSF IT and Information Commons.

Licensed under the [Apache License 2.0](LICENSE).

<div align="center">
  <p>
    <a href="https://github.com/BaranziniLab/biorouter/releases">Download</a> ·
    <a href="docs/getting-started/installation.md">Setup guide</a> ·
    <a href="https://github.com/BaranziniLab/biorouter/issues">Report an issue</a> ·
    <a href="mailto:wanjun.gu@ucsf.edu">Contact</a>
  </p>
</div>
