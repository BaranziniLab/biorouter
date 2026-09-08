# BioRouter documentation

BioRouter is an AI-powered integrated research environment for biomedical discovery, built by UCSF's Baranzini Lab. It unifies multiple LLM providers, AI agents, MCP-based extensions and customizable workflows behind one desktop app and one command-line tool, layered as interface → agent → extensions.

This tree holds everything written about BioRouter: end-user guides, subsystem design and reference documentation for developers, and the archived record of work that has already finished.

## How this tree is organized

Two kinds of document live here, and the difference matters more than any other fact on this page.

- **Living documentation is filed by subsystem at the top level.** One folder per area — `providers/`, `security/`, `agent-loop/`, and so on. These describe how BioRouter behaves now, and they are maintained.
- **`history/` holds records of completed or abandoned work.** Executed plans, closed review campaigns, finished stress tests, and a few designs that were removed or never built. It is kept so decisions stay auditable and so you can find out *why* a subsystem is shaped the way it is. It is explicitly **not** current guidance, and reading it as such will mislead you.

**Every document carries a status in its header** saying which it is. Trust that line over the filename, the folder, or your memory of the feature. A living document whose one section has rotted stays `Current` and carries a warning naming that section, rather than being restatused.

## Start here

| If you want to… | Read |
|---|---|
| Install BioRouter and run a first task | [biorouter in 5 minutes](getting-started/quickstart.md) — or [Installation and setup](getting-started/installation.md) for the thorough path with institutional providers |
| Pick an LLM provider and model | [Choosing a model provider](getting-started/choosing-a-model-provider.md) |
| Use a Claude or ChatGPT subscription you already pay for, instead of an API key | [Coding-agent providers](providers/coding-agents/README.md) — read [its compliance page](providers/coding-agents/compliance.md) before any research data goes near one |
| Look up a command or flag | [biorouter CLI command reference](cli/command-reference.md) |
| Change a setting | [Configuration file reference](configuration/config-file-reference.md) for `config.yaml`, [Environment variables](configuration/environment-variables.md) for per-invocation overrides |
| Decide how much autonomy the agent gets | [Permission modes](security/permission-modes.md) |
| Understand a cross-institution warning you just saw | [Institutional affiliation](security/institutional-affiliation.md) |
| Resume, export or prune your past work | [Managing sessions](getting-started/managing-sessions.md) |
| Run several conversations at once, or delegate to a subagent you can watch | [Workspace control](agent-loop/workspace-control.md) |
| Use Biorouter in a web browser, on your own machine or a shared host | [Reaching Biorouter from a browser](deployment/browser-access.md) — then [Headless Linux deployment](deployment/headless-linux.md) if it should run as a server |
| Read or follow a **private** chat from a script, a dashboard or CI | [Reaching a private chat from a script](deployment/programmatic-session-access.md) — the `X-Caller-Provider` header, and which routes honour it |
| Fix an error you are hitting right now | [Common problems and fixes](troubleshooting/common-problems-and-fixes.md) |
| Understand the codebase before changing it | [System overview](architecture/system-overview.md) |

## By topic

| Area | What it covers |
|---|---|
| [getting-started](getting-started/README.md) | The end-user on-ramp: installing the app and CLI, connecting a provider, running a first biomedical task, managing sessions, and day-to-day usage habits. |
| [architecture](architecture/README.md) | The orientation-level map of the three layers and the crate and process boundaries, plus the agentic system explorer's account of how one request becomes context, tool work and a verified answer. |
| [agent-loop](agent-loop/README.md) | The reasoning loop itself — durable context, subagents, lifecycle hooks, tool routing — plus the designs behind its guardrails: command policy, sandboxing, checkpoints, session branching. |
| [agent-drafter](agent-drafter/README.md) | The app-authoring MCP extension that builds BioRouter apps, and the frozen 100-spec corpus and runbook used to stress-test it. |
| [apps-sdk](apps-sdk/README.md) | The contract behind BioRouter apps in three layers: the shipped reference, the v2 design of record, and the phase roadmap. |
| [extensions](extensions/README.md) | Built-in capabilities, user-installed MCP extensions, and skills, with a reference page for each capability plus an open investigation into Slack posting. |
| [integrations](integrations/README.md) | The opposite direction from `extensions/`: adapters that let another application host a Biorouter agent, starting with the `@Biorouter` persona for JupyterLab's Jupyter AI chat. |
| [knowledge-base](knowledge-base/README.md) | The live working documents for the personal, LLM-maintained wiki: surveys of the ingestion pipeline and plans for extending it. |
| [providers](providers/README.md) | Maintainer-facing integration references for individual LLM providers: registry wiring, credential contracts, selection surfaces and verification commands. Includes [coding-agent providers](providers/coding-agents/README.md) — the two that run on the user's own Claude or ChatGPT subscription by driving a vendor CLI, with their tool bridge, child-isolation flags and the vendor-terms and PHI compliance position. |
| [security](security/README.md) | Agent autonomy, admin-imposed managed policy, credential storage, which providers are acceptable for patient and other sensitive data, and the institutional affiliation check behind cross-institution warnings. |
| [workflows](workflows/README.md) | Reusable workflow files that package instructions, extensions and model settings into one shareable session, plus the built-in cron scheduler. |
| [cli](cli/README.md) | The `biorouter` command-line surface: subcommands and flags, the interactive terminal UI, and the manual QA script that verifies both. |
| [configuration](configuration/README.md) | The complete reference for both configuration forms — persistent YAML files and the environment variables that override them. |
| [desktop-ui](desktop-ui/README.md) | Exercising the Electron desktop app as a running program: launching and driving the dev GUI, and the behavior to check once it is in front of you. |
| [deployment](deployment/README.md) | Reaching Biorouter through a browser instead of the desktop app — `biorouter serve` on your own machine or on a shared Linux host: the access token, what exposing the port means, and the decisions behind the serving path. |
| [releases](releases/README.md) | Shipping to users: the auto-update QA checklist, a local cross-compilation recipe, and the published per-version release notes. |
| [troubleshooting](troubleshooting/README.md) | Known problems and their fixes, the diagnostics bundle, and how to file a useful bug report. |
| [design](design/README.md) | Visual design specifications and their rendered companions: brand marks, theme families, chat-group design spikes, the desktop UI overhaul, the proposed Astryx interface revision, and the browser-only design-system and boot-splash studios. |
| [research](research/README.md) | External research — studies of other agentic coding tools, written to inform BioRouter's own design. |
| [contributing](contributing/README.md) | How this documentation tree itself is written and maintained: the house style every file follows, and the live register of unresolved problems found in it. |

## Historical records

[`history/`](history/README.md) is the archive: 28 topic folders covering May–August 2026, almost all of which shipped. Read it to trace a decision, reconstruct what landed in a release, or decode an identifier like `BR-43` in a commit message — never to learn what the code does today.

The largest campaigns in there:

| Campaign | What it records |
|---|---|
| [agent-loop-review](history/agent-loop-review/README.md) | The agentic-loop review: a loop walkthrough, a comparison against nine open-source coding agents, 28 sub-reports, and the 67-item `BR-NN` proposal register the rest of the tree cites. |
| [agent-loop-campaign](history/agent-loop-campaign/README.md) | The `BR-1`…`BR-70` fix campaign that implemented those findings — wave conventions, regression gates and a dated merge log, across 86 commits. |
| [knowledge-base-buildout](history/knowledge-base-buildout/founding-design.md) | The founding design for the Knowledge feature and the six implementation plans that built it out in order. |
| [agent-drafter-stress-test](history/agent-drafter-stress-test/README.md) | A campaign driving a real model to build 100 agentic apps with Agent Drafter; the eight resulting drafter fixes shipped. |
| [agent-drafter-testdrive-100](history/agent-drafter-testdrive-100/README.md) | A separate test drive against a 100-app spec corpus — per-app rubrics, three cross-cutting audits, and a six-wave remediation plan. |
| [performance-2026-06](history/performance-2026-06/review-findings.md) | A whole-app latency review against v1.86.0 plus an independent comparison against the jcode harness; nine fixes merged. |
| [streaming-tool-call-ui-2026-07](history/streaming-tool-call-ui-2026-07/README.md) | The July 2026 streaming tool-call campaign: why a tool card appeared late and already finished, the streaming implemented across fourteen providers that never streamed, and three QA rounds over the result. |
| [dashboard-mode](history/dashboard-mode/README.md) | Four generations of design for a free-floating multi-chat canvas, and the record of its removal on 2026-07-18. **The feature no longer exists.** |
| [mcp-apps-removal](history/mcp-apps-removal/README.md) | The sandboxed iframe in which a third-party MCP server ran its own UI inside BioRouter — inherited from the upstream fork, never requested, removed on 2026-09-08. **The feature no longer exists**; Agent Drafter's Built apps is a different one and stays. |
| [legacy-architecture](history/legacy-architecture/README.md) | Two superseded internals designs: a hand-written `Extension` trait framework that was **never shipped**, and the agent error model, whose two-tier policy still holds but whose every type name is gone. |

## Conventions

- **Every file opens with a context header** — a blockquote giving *What this is*, *Status*, and *Audience* — before any other content. `README.md` index files are the exception: they open with a paragraph describing the folder's scope instead.
- **Status is one of three values:** `Current`, `Historical record — <what completed, and when>`, or `Superseded — <what changed, and where the current truth lives>`. Anything dated, planned, or reporting completed work must carry one.
- **Folders are topic-specific and each carries a `README.md`** listing its documents with a one-line description each.
- **Filenames are kebab-case and name the document's purpose,** not the process that produced it — `defects-found-and-fixed.md`, not `findings.md`. `README.md` is the only `ALL_CAPS` name. Dated reports carry an ISO date suffix when the date is load-bearing.
- **Where a document goes** is governed by [how this documentation is organized](organization.md) — the sorting rules, when a new folder is justified, and what to do when the rules do not fit. **Read it before adding a document or creating a folder.**
- What a document looks like inside is governed by [documentation style](contributing/documentation-style.md); gaps and known problems in this tree are tracked in [open documentation issues](contributing/open-documentation-issues.md).

## Adding a document

> **Start at [how this documentation is organized](organization.md).** It answers where a document goes, when a new folder is warranted, and how to extend the system for a feature area that does not exist yet.

Decide first whether you are writing living documentation or a record of finished work — that choice picks the folder and it is the one thing readers rely on. Put living documentation in the subsystem folder it belongs to; put anything describing completed or abandoned work under `history/`. Then write the context header before the body, so the status is never left implicit. Finally, add a row for it in the owning folder's `README.md`, and check that every link you wrote resolves. If your document is the obvious answer to one of the arrivals in **Start here** above, add it there too.
