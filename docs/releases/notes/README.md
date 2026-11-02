# Release notes

This folder holds user-facing release notes for Biorouter. Not every point release has a
note, and the selected index below is not a complete inventory. Headings and download
information vary by release; not every note includes a Downloads table.

Each note is a frozen record of a release as it shipped. Older entries describe the
product at that version, not its current behaviour. Read the newest published release
note for recent changes, and the user guides for current instructions.

Come here when you want to know **what changed in a particular version** — which
release introduced a feature, whether a bug you remember was fixed and when, or which
artifact filename to expect from a given download. This folder is not the place to
learn how to *use* a feature or how to *cut* a release. For installing or upgrading,
see [Installation and setup](../../getting-started/installation.md). For the release
engineering itself — the pre-flight QA and the cross-compilation recipes — stay one
level up in [`docs/releases/`](../README.md). For the engineering
paper trail behind a feature (design specs, implementation plans, review records),
`docs/history/` groups those by project rather than by version.

## Notes by version

| Document | What it covers |
| -------- | -------------- |
| [v1.94.1](v1.94.1.md) | Improves Ollama setup checks and recovery, clarifies Unlimited and Set a limit controls, and stops Biorouter Copilot status checks on Windows from launching hostname processes. |
| [v1.94.0](v1.94.0.md) | Adds explicit approval for folder access outside the project, removes the old default limits from ordinary chats, improves interrupted-reply recovery, and updates Versa API Bedrock models. |
| [v1.89.2](v1.89.2.md) | August 20, 2026. A patch by version number and a substantial one by content: more than 120 commits finishing four areas that were visibly incomplete in v1.89.1. Adds **Claude Code** and **Codex** as providers that run on the subscription you already pay for, with no API key, handing the child agent Biorouter's own tools over a private MCP bridge that Biorouter serves and polices; moves knowledge bases onto the **Open Knowledge Format** with **BioOKF** as a strict biomedical profile, typed edges and a merge; stops the desktop app freezing at launch; and lets you generate diagnostics from a private chat, with a warning about what the file holds. |
| [v1.89.1](v1.89.1.md) | August 19, 2026. A patch release. Fixes the AWS Bedrock failure that could burn half an hour of a session producing nothing, sizes short chat messages to their own content instead of stretching them across the window, and finishes the terminology pass so the app calls a chat a chat. |
| [v1.89.0](v1.89.0.md) | August 7, 2026. A turn now belongs to the session rather than the HTTP request, so a reload no longer kills work in flight and reopening a conversation attaches to the turn in progress; tabs can be torn off into their own window and merged back; the desktop interface is rebuilt on one design system with Figtree as the interface face; and `session attach` lets the terminal join and steer a running session. |
| [v1.88.6](v1.88.6.md) | July 27, 2026. A reliability and fit-and-finish release closing every open issue on the tracker — twenty-four reports spanning headless concurrency, the agent's tool surface, provider coverage, local models, and the desktop interface. The headline fixes are the ones that made parallel work unsafe: concurrent headless runs aborting with a database error, and unattended runs stalling on a permission prompt nobody could answer. |
| [v1.88.5](v1.88.5.md) | July 21, 2026. An agent-autonomy and desktop-polish release. Introduces **Fully-Automatic mode** — broad file autonomy, stopping only for genuinely sensitive system operations — hardens the permission engine against false-positive prompts that could stall an unattended run, and lands desktop chat, tab, terminal, and artifact-preview fixes on top of the v1.88.3 interface-cohesion work. |
| [v1.88.3](v1.88.3.md) | July 18, 2026. Completes the interface-cohesion redesign and relicenses the brand mark — the wordmark and BR icon move to Inter, an open-licensed typeface that may legally be used as a logo — and lands a cohesive desktop tab, terminal, and navigation model plus artifact-preview and permission hardening. |
| [v1.88.2](v1.88.2.md) | July 15, 2026. Introduces the new router identity across every shipped surface, standardizes the product name as **Biorouter**, adds recent-session sidebar navigation, corrects current-week usage reporting, and expands artifact previewing to documents, spreadsheets, notebooks, directories, and git-backed workspaces. |
| [v1.88.1](v1.88.1.md) | July 15, 2026. A focused reliability and control release: a selective factory-reset surface, a Home activity view served immediately from bounded caches, hardened bundled local-model startup and warm-up, a theme-pack persistence fix, and normalized tooltip layout. The production desktop dependency graph moves to patched releases and passes its security audit clean. |
| [v1.88.0](v1.88.0.md) | July 14, 2026. A broad reliability, safety, performance, and interface release spanning the desktop app, terminal UI, agent runtime, Agent Drafter, headless server, and cross-platform distribution pipeline; makes long-running agent work more controllable and reports usage more honestly. No required configuration migrations. |
| [v1.87.2](v1.87.2.md) | July 2026. Reworks the chat surface around three new capabilities — a side artifact viewer that auto-detects figures and HTML the agent produces, an in-app tabbed terminal rooted in the session directory, and a Local Model Inventory in Settings — alongside a streamlined composer with slash-command discovery and a compact context ring. |
| [v1.87.1](v1.87.1.md) | July 2026. A UI-polish release on top of v1.87.0: a warmer two-tone desktop theme (beige sidebar against an off-white chat canvas), flatter cards, hairline separators, and a fix for top-of-chat controls that became unclickable when the sidebar was collapsed. |
| [v1.87.0](v1.87.0.md) | July 2026. Gives the chat, home, dashboard, knowledge, settings, workflows, scheduler, extensions, skills, applications, and history panels a shared layout system — centered readable content, compact surfaces, softer shadows — while hardening the agent runtime, knowledge ingestion, and the release packaging flow. |
| [v1.86.1](v1.86.1.md) | June 2026. A polish-and-reliability release on v1.86.0: fixes session auto-naming so chats stop sticking on "New Session," restores rich artifact rendering in the standalone Expand window, redesigns the tool-permission confirmation card, and makes the CLI status line count extensions the way the GUI does. |
| [v1.86.0](v1.86.0.md) | June 2026. Adds one-click "Restart & Update" auto-update for macOS, multi-monitor screen capture with vision input, agent-readable UI-automation errors, built-in extension and skill authoring skills, and a Capabilities settings section for foundational built-ins. |
| [v1.85.4](v1.85.4.md) | June 2026. Adds the Agent Drafter built-in extension for authoring AI-agent-enabled artifacts, an ACP WebSocket transport, two LLM providers (z.ai / GLM and Xiaomi MiMo), 24 new Auto Visualiser charts, and tree-sitter support for C++, C, R, Julia, and MATLAB. |
| [v1.85.3](v1.85.3.md) | June 2026. A polish-and-fix release: chat defaults to autonomous tool-calling, the Llama Server context window is reported accurately across CLI and GUI, the interactive CLI gains clean streaming and copy/paste, and a more resilient installer ends the perpetual "Biorouter CLI Update" prompt. |
| [v1.85.2](v1.85.2.md) | June 2026. Introduces Llama Server — zero-setup local models bundled with the desktop app — plus an interactive knowledge graph view, background shell jobs for the developer tools, and a markdown-rendering CLI. Also removes analytics and telemetry instrumentation entirely. |
| [v1.85.0](v1.85.0.md) | June 2026. Brings the command-line interface to full parity with the desktop app, ships the `biorouter` CLI with every download, adds a shared system/setup layer (dependency checks, CLI install, self-update) and a headless CLI-only Linux package, and lands lifecycle hooks, agent goals and recurring automations, and PowerPoint/spreadsheet knowledge ingestion. |
| [v1.80.1](v1.80.1.md) | June 2026. A patch on v1.80.0 that makes Knowledge fully usable from chat — create, curate, visualize, and move knowledge bases by talking to the agent, with the graph view staying in sync with what the agent writes. |
| [v1.80.0](v1.80.0.md) | June 2026. The Knowledge release: personal, LLM-maintained knowledge bases backed by markdown trees and full git history, with credibility classification, `[[cross-links]]`, a force-directed graph, a roll-backable change log, and everything stored locally on disk. |
| [v1.76.1](v1.76.1.md) | June 2026. A feature release focused on multimodal image input — vision-capable models now actually receive the bytes of attached images. A patch on top of v1.76.0 with no breaking changes. |
| [v1.76.0](v1.76.0.md) | May 2026. Adds dashboard fold mode (windows folded into compact hover-preview cards), 7-day and 30-day windows in the session insights view, an onboarding flow rewritten as composable provider cards, and logging of failed provider requests for diagnosing endpoint connectivity. |
| [v1.75.2](v1.75.2.md) | May 2026. A small, focused patch. A fresh install of Versa API Bedrock now works on the first try with only an Access Key ID and Secret Access Key, because the UCSF MuleSoft proxy endpoint, region, and credentials are wired in explicitly. |

## Related documentation

- [Auto-update test checklist](../auto-update-test-checklist.md) — the pre-release QA script for the one-click "Restart & Update" flow these notes announce, plus a frozen evidence log of tests already run.
- [Cross-compiling locally with `cross`](../local-cross-compilation.md) — how to build and smoke-test release binaries for other architectures on your own machine, for ad-hoc packaging QA.
- [Installation and setup](../../getting-started/installation.md) — current installation and
  upgrade instructions, including provider setup and file locations.
- [Deployment](../../deployment/README.md) — running BioRouter as a shared server instead of a desktop app, the mode served by the headless Linux package introduced in v1.85.0.
