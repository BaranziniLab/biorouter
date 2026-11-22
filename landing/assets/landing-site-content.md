# Landing site content and requirements tracker

> **What this is.** A page by page record of what the Biorouter website at biorouter.ucsf.edu shows, and the requirements its pages meet.
> **Status:** Current (checked against `landing/` on 2026-11-15).
> **Audience:** maintainers of the website under `landing/`.

The website is the static site in `landing/`, published by `.github/workflows/deploy-landing.yml`. This file says what each of its five pages carries, so a change to a page can be checked against it. Scripts read two parts of it: `scripts/release.sh landing` reads the version line below and the first release link in the News list under Tab 5, and `landing/scripts/check-consistency.mjs` and `assemble-docs.mjs` read the version line. How the site is built and checked is in [the website README](../README.md).

## Overview

Landing site for UCSF Biorouter. Design: the desktop app's own look on the web. White paper, warm ink, subtle fills instead of outlined boxes, hairlines only for section and table rules, one coral accent, mobile/tablet/desktop responsive, light only.

**Color palette:** white paper (#ffffff), warm ink (#2a2520), coral (#b85a32, rails #cf6d47), UCSF navy (#052049). The values mirror the app's Parchment light tokens in `site.css`.
**Font:** Arial first, the app's interface stack (`--font`); Inter 600 only for the wordmark and the BR mark; one monospace stack for code. The type scale is shared with the app: 12, 13, 14, 17 and 24 plus a 16px reading size and two clamps for titles (`--fs-*` in `site.css`).
**Version:** v1.94.5 (November 2026)

---

## File structure

```
landing/
├── index.html          # Introduction
├── download.html       # Download
├── docs.html           # Documentation (assembled, never edited by hand)
├── baam.html           # BAAM; its extension and skill cards generate registry.json
├── about.html          # About: team, collaborators, inspirations, updates, links
├── intro.html          # Redirects to index.html
├── skills.html         # Redirects to baam.html
├── CNAME               # biorouter.ucsf.edu
├── README.md           # How the site is built, published and checked
├── MIGRATION.md        # Record of the 2026-06-29 move of the site into this repository
├── site.css            # Tokens, type scale and parts shared by every page
├── home.css, download.css, docs.css, baam.css, about.css   # One stylesheet per page
├── site.js             # Header, menu, reveal on scroll, copy buttons, the release version
├── home.js             # The hero's download label and its "New" line
├── download.js         # Platform detection and the download table
├── baam.js, baam-details.js, marketplace-search.js   # BAAM shelves, card details, the search matcher
├── art/                # The small product animations (CSS and JS): the Introduction page's, and the Download page's five ways to use Biorouter (access-*)
├── registry.json       # The marketplace catalog, generated from baam.html
├── scripts/            # Consistency checks, the registry generator, the docs assembler
├── shared.css, theme.js, app-mockups.css, app-mockups.js   # The previous design's files (see below)
├── video/              # Sources for the tour, release and reel videos
├── docs/               # One design spec under docs/superpowers/specs/, not page content
├── icon.svg, icon-transparent.svg, icon.png, icon-transparent.png
└── assets/
    ├── landing-site-content.md   # This file
    ├── ehr-diabetes-recipe.yaml  # The workflow file BAAM's Workflows shelf offers
    └── videos/                   # Rendered mp4 and webm files and their posters
```

No page loads `shared.css`, `theme.js`, `app-mockups.css` or `app-mockups.js`. The reel in `video/reel/` loads `shared.css` and the two `app-mockups` files, `video/` keeps its own copies under `video/assets/`, and `check-consistency.mjs` reads `app-mockups.js`. The Introduction page and the Download page's "Ways to use Biorouter" draw their product animations with the CSS and JS in `art/`. No page embeds the files in `assets/videos/`.

Docs source: `docs/website/` (a shell and one fragment per page). `node landing/scripts/assemble-docs.mjs` writes `landing/docs.html` from it.
Icon: `landing/icon.svg`, `icon-transparent.svg`, `icon.png` (copied from `ui/desktop/src/images/`).
GitHub: https://github.com/BaranziniLab/biorouter

---

## Tab 1: Introduction (`index.html`)

### Header and footer (every page)
- Header: the Biorouter mark and wordmark, then Introduction, Download, Docs, BAAM, About, GitHub, and a Slack link
- Footer: the mark and "A research environment for biomedical AI, from the Baranzini Lab at UCSF."; three columns, Product (Download, Docs, BAAM, Release notes), Community (About, Updates, Slack, GitHub) and UCSF (Baranzini Lab, Bakar Institute, Academic Research Services); "© 2026 Baranzini Lab, UCSF" and the Apache 2.0 license

### Hero
- Eyebrow: "Baranzini Lab · UCSF"
- Headline: "The AI-native biomedical research environment."
- Lede: Biorouter combines local, institutional and commercial models in one place; private data stays private, inside your institution, and shared workflows carry the method, never the data
- Download button, which names the visitor's platform (`home.js`), and "Read the docs"
- Meta line: "Free and open source · macOS, Windows, Linux · v1.94.5", the version refreshed from GitHub releases
- "New" line: the headline of the newest row in the About page's Updates list, read by `home.js`, linking to that list. Its fallback text names no version ("Latest Biorouter updates"), because `scripts/release.sh landing` replaces every copy of the old version in `index.html`
- Hero animation (`art/hub`): Biorouter at the center, connected to local, institutional and public models. Caption: "One interface, every model." Local models, institution-hosted models, commercial APIs, and the Claude Code or Codex plan the visitor already pays for, switched inside a chat

### Principles (3)
1. Private stays private: a chat that uses a private model or private data becomes private, and stays private
2. Share the method, not the data: a workflow carries instructions, never records
3. Built for biomedical research: connectors from BAAM, research skills, and knowledge bases with a biomedical vocabulary

### Community band
"Join the Biorouter Slack." with a Join the Slack button. Every page carries the same band.

### Capabilities (9)
1. Workflows: instructions, extensions, skills and a model in one file, run again or shared
2. Scheduling: a workflow on a schedule, or a prompt looped at an interval
3. Skills: teach the agent a method once; install research skills from BAAM or write your own
4. Knowledge bases: a wiki the agent writes and reads, with typed links, sources and history; BioOKF adds a biomedical vocabulary
5. Applications: describe a tool and the agent builds it, then works inside it with you
6. Crew: shared lab channels, where each person's agent runs on their own computer with their own model
7. Subagents: hand parts of a task to subagents that run at the same time, watch each one in the side panel, and get the results back in the chat
8. Decision models: fast models answer a yes or no question with a probability; screen many notes, then send only the unsure ones to a larger model
9. Voice dictation: speak into any composer; speech is transcribed on your computer, and your dictionary spells gene names and lab terms

### Get started band
"Start with Biorouter." Download the app, connect a model, ask a first question; the `biorouter` command line tool comes with it. Download button and a Getting started link.

### Background (from the project presentation)
This is the project's framing, kept for reference. The page does not show it as written.

**Core message:** Biorouter is an agentic, AI-powered integrated biomedical research environment.

**Problem statement:**
- Biomedical researchers face: heavy local compute needs, unique local setups, proprietary codebases, on-prem datasets, sensitive patient/research data
- Commercial AI services are limited due to privacy/security
- UCSF enterprise AI lacks agentic support

**Solution: Biorouter is an all-in-one tool with:**
- Security offered by enterprise AI models
- Ability to orchestrate diverse workflows with AI agents
- Share workflows without sharing data across collaborations

**One platform that unifies:**
- Commercial, institution-hosted, and local LLMs
- AI agents
- Information Commons databases

**Supports:**
- Customizable workflows for exploratory analysis, prototyping, automation
- Federated cross-institution collaboration

**Key capability: Share Workflow and Skills:**
- Can be shared across institutions
- Standardized analysis pipelines with AI-driven adaptation

**Key capability: Diverse Research Database Connections:**
- Structured and unstructured electronic health records
- Biological knowledge graph (SPOKE)

**Conclusion: Biorouter:**
- Unifies the full research stack (commercial + institution-hosted + local AI models)
- Is secure by design: enterprise-grade AI security
- Enables agentic orchestration and automation
- Is federated-collaboration-ready
- Offers growing ecosystem (skills, agents, databases, etc.)
- Ongoing: UCSF Institution-managed AI Agent Marketplace (quality-controlled, community-driven)

---

## Tab 2: Download (`download.html`)

### Hero
- Title: "Biorouter for your computer." (the second line in the quiet ink)
- Buttons: the installer for this computer, then "All platforms"
- Meta line: "Free and open source · v1.94.5 · the release date from GitHub · Release notes"

### Smart OS detection
JavaScript (`download.js`) detects the visitor's OS and shows the recommended download button prominently.

**Detection logic:**
- macOS Apple Silicon (arm) → arm64.dmg, with the Intel file offered under it
- macOS Intel → x64.dmg, with the Apple silicon file offered under it
- macOS with an unknown chip → two buttons, Apple silicon first
- Windows → Biorouter-Setup-{ver}.exe (the installer, preferred), falling back to Biorouter-win32-x64-{ver}.zip; the zip is offered as "Or the archive:"
- Linux → amd64.deb, with the .rpm offered "For Fedora or RHEL:"
- Linux on ARM → a note that the Linux packages are built for x86_64
- Unknown/iOS/Android → show all options without a primary

### Download links (v1.94.5)
| Platform | File | URL |
|---|---|---|
| macOS Apple Silicon | Biorouter-1.94.5-arm64.dmg | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/Biorouter-1.94.5-arm64.dmg |
| macOS Intel | Biorouter-1.94.5-x64.dmg | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/Biorouter-1.94.5-x64.dmg |
| Windows installer | Biorouter-Setup-1.94.5.exe | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/Biorouter-Setup-1.94.5.exe |
| Windows archive | Biorouter-win32-x64-1.94.5.zip | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/Biorouter-win32-x64-1.94.5.zip |
| Linux Ubuntu/Pop!_OS | biorouter_1.94.5_amd64.deb | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/biorouter_1.94.5_amd64.deb |
| Linux Fedora/RHEL | Biorouter-1.94.5-1.x86_64.rpm | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/Biorouter-1.94.5-1.x86_64.rpm |
| Linux CLI Debian/Ubuntu | biorouter-cli_1.94.5_amd64.deb | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/biorouter-cli_1.94.5_amd64.deb |
| Linux CLI Fedora/RHEL | biorouter-cli-1.94.5-1.x86_64.rpm | https://github.com/BaranziniLab/biorouter/releases/download/v1.94.5/biorouter-cli-1.94.5-1.x86_64.rpm |

The Linux desktop packages (.deb and .rpm) need Debian 12, Ubuntu 22.04, Fedora or RHEL 9 or newer, and local models need Debian 12 or Ubuntu 22.04 or newer. The command line packages also run on Debian 11 (x86_64).

### Install commands
- macOS: Open DMG and drag Biorouter.app to /Applications
- Windows: run Biorouter-Setup-1.94.5.exe, which upgrades an existing install in place; or unzip the archive and run Biorouter.exe
- Linux Debian: sudo apt install ./biorouter_1.94.5_amd64.deb
- Linux RPM: sudo dnf install ./Biorouter-1.94.5-1.x86_64.rpm

### After you install
1. Open the app: from Applications, the Start menu or the app launcher
2. Connect a model: pick a provider on the Welcome screen, or later in Settings → Models. Llama Server, on the Local tab, needs no account or key. UCSF users pick Versa API Azure or Versa API Bedrock on the Institutional tab
3. Ask a question
4. Link to the Getting started page of the docs

### Ways to use Biorouter
Directly after "All platforms". Five looping animations (`art/access-*.css` and `.js`), 3 + 2 with the second row centred, then 2 + 2 + 1, then one column. Each caption is a title, one sentence and a link to where to get it:
1. Desktop app: "Chats, tools and a side panel for what the agent makes." Download the app (to All platforms)
2. Terminal chat: "A full screen chat in your terminal, with history and key hints." Chat in the terminal (docs)
3. Command line: "One task at a time, from a script, a cluster job or CI." Run one task (docs)
4. Browser: "On a server with no screen, open the same interface in a browser tab." Browser access (docs)
5. JupyterLab: "Ask @Biorouter in the Jupyter AI chat, beside your notebook." Jupyter AI persona (the reference on GitHub)

Every string a tile quotes from the product (the terminal banner and hints, the `serve` banner, the run flags, the placeholders, the tool rows) is read back from the source by `check-consistency.mjs`. The questions and answers are illustrations, with no patient data.

### Command line and servers
- The command line tool ships with the app; the app offers to put it on the PATH as it starts (the Biorouter CLI card), or `biorouter setup-path` does it by hand
- On a Linux server, HPC node or container, the command line only packages hold `biorouter`, `biorouterd`, the web interface and the `biorouter-crew` helper
- `biorouter configure`, `biorouter serve` and `biorouter doctor`, each with a copy button

### Requirements
macOS 12 or newer on Apple silicon and Intel (local models need macOS 13.3 or newer); Windows 10 or newer on x64; Linux x86_64 as above.

---

## Tab 3: Documentation (`docs.html`)

`docs.html` is assembled, never edited by hand: `node landing/scripts/assemble-docs.mjs`, run from the repository root, writes it from `docs/website/shell.html` and the fourteen page fragments in `docs/website/pages/`. A search box ("Search the docs") sits above the sidebar.

### Left sidebar navigation
| Group | Page | Source |
|---|---|---|
| Start | Getting started | `docs/website/pages/getstarted.html` |
| Start | The desktop app | `docs/website/pages/app.html` |
| Start | Command line | `docs/website/pages/cli.html` |
| Models and privacy | Models and providers | `docs/website/pages/providers.html` |
| Models and privacy | Private and public | `docs/website/pages/privacy.html` |
| Models and privacy | Security | `docs/website/pages/vaulting.html` |
| Build | Extensions and skills | `docs/website/pages/extensions.html` |
| Build | Workflows and scheduling | `docs/website/pages/workflows.html` |
| Build | Knowledge bases | `docs/website/pages/knowledge.html` |
| Build | Applications | `docs/website/pages/apps.html` |
| Build | Crew | `docs/website/pages/crew.html` |
| How it works | The agent loop | `docs/website/pages/agentloop.html` |
| How it works | Configuration | `docs/website/pages/config.html` |
| How it works | Architecture | `docs/website/pages/architecture.html` |

### Built-in extensions
The Extensions and skills page lists the seven bundled extensions, each on by default: Developer, Biorouter Copilot, Web & Documents, Auto Visualiser, Memory, Knowledge and Application Drafter (`ui/desktop/src/components/settings/extensions/bundled-extensions.json`; `check-consistency.mjs` checks the docs table against it). Chat Recall is an optional platform capability that is off by default, as is Dictionary. The platform capabilities on by default are Workspace Control, Code Execution, Todo, Skills, Extension Manager, Decisions and Crew.

---

## Tab 4: BAAM (`baam.html`)

**Full name:** Biorouter AI Agent Marketplace (BAAM)

### Hero
- Title: "Biorouter AI Agent Marketplace. Extensions, skills and workflows."
- Lede: install from the Extensions, Skills and Workflows pages in Biorouter, or download a file here and add it yourself
- Search box that filters the open shelf as you type ("Search extensions", "Search skills", "Search workflows")
- Three shelves as tabs, each with its count: Extensions, Skills and Workflows

### Catalog
BAAM has three shelves: Extensions, Skills and Workflows. The machine-readable catalog `landing/registry.json` (version 2; currently 28 extensions and 132 skills) is generated from `baam.html` by `landing/scripts/build-registry.mjs`, with privacy and institution badges. The same run writes the desktop app's bundled copy and the Rust list of private extensions. Workflows live only on `baam.html` (currently one, the EHR Diabetes Demographics & Reporting Dashboard) and are not in `registry.json`. The extensions include audited rebuilds such as Folklore Clinical Variant Interpretation MCP v2.0.0+biorouter.1. Read `registry.json` for the current list rather than a copy here. Every extension and skill, active or not, is tracked in `landing/catalog/` (see `docs/extensions/baam-catalog.md`), and items that left the shelves are listed under Moved and retired at the foot of the page.

### Cards
- Extension cards, grouped Featured, Extensions, Integrations and All extensions: an SVG icon tile, the name, the author and version, a GitHub link, the description, tags (a Private or Public privacy badge, an affiliation badge such as "UCSF data", then topic tags), and a `.brxt` download
- Skill cards, grouped Featured, Core skills, Developer and authoring skills, and Biomedical analysis skills: the name, its type or slash command, the description, tags and a `.zip` download
- The workflow card: the description, tags, a `.yaml` download (`assets/ehr-diabetes-recipe.yaml`), "Use it" and "Share it" steps, and the file itself

### Contribute
Where to contribute: the app and this website (BaranziniLab/biorouter), skills (BaranziniLab/biorouter-skills), workflows (BaranziniLab/biorouter-workflows), and extensions (each extension's own GitHub repository).

---

## Tab 5: About (`about.html`)

### Intro
- Headline: "Built at UCSF, open source."
- Lede: Biorouter is a research environment for biomedical AI, built by the Baranzini Lab (https://baranzinilab.ucsf.edu/) at UCSF; anyone may use, study and change it under the Apache 2.0 license

### Team
- **Wanjun Gu:** Lead developer, Baranzini Lab, UCSF; GitHub https://github.com/Broccolito; wanjun.gu@ucsf.edu
- **Gianmarco Bellucci:** Baranzini Lab, UCSF
- **Sergio Baranzini:** Baranzini Lab, UCSF; lab website https://baranzinilab.ucsf.edu/

### Collaborators
**Bakar Computational Health Sciences Institute (https://bakarinstitute.ucsf.edu/)**
- Sharat Israni
- Marina Sirota

**UCSF Academic Research Services (https://ars.ucsf.edu/)**
- William Santo
- Evan Philps
- Rick Larson
- Oksana Gologorskaya

### Inspirations
"Biorouter's design was inspired by the agents below." Claude Code, Codex CLI, Gemini CLI, OpenHands, Aider, Cline, OpenCode and ForgeCode, each with a one-line description and a link.

### News
The page's Updates list, newest first. The page shows its newest five rows and folds the rest under "Earlier updates". The first entry here is the newest release, which `scripts/release.sh landing` checks.

1. **Biorouter v1.94.5 Release** (20 November 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.94.5
   - What's new: Biorouter adds local voice dictation, clearer controls for longer tasks, browser provider setup and updates, and compact split panes. Monthly budgets that previously only drove the Usage gauge are now enforced.

2. **Biorouter v1.94.1 Release** (2 November 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.94.1
   - What's new: Ollama checks the configured server and model before work starts and gives clearer setup guidance; Meditation keeps its last successful discovery date after connection and server failures; Max turns has clear Unlimited and Set a limit controls; Windows no longer flashes console windows during Biorouter Copilot status checks

3. **Biorouter v1.94.0 Release** (25 October 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.94.0
   - What's new: file operations outside your project ask for read only or read and write access to that folder, once or for the session, and Folder access under the composer lists and revokes grants; session history and undo snapshots use storage only your account can read; ordinary chats no longer stop at 100 turns or 200 tool calls; stream recovery handles interrupted responses; tool cards tell a failed operation from a successful status lookup; bug reports need one approval of the exact report before they are filed; JSON CLI output stays parseable; Versa API Bedrock adds Claude Opus 5.5 and Sonnet 5.5

4. **Biorouter v1.93.4 Release** (21 October 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.93.4
   - What's new: Baseten joins the public providers with eleven open-weight models, including GLM 5.3, Kimi K3, DeepSeek V4.1 and GPT OSS 120B; chat costs use Baseten's prices, including cached input; Baseten's model list and prices stay current between releases from a public catalog Biorouter reads only while a Baseten key is saved

5. **Biorouter at Jupyter Day 2026** (19 October 2026, San Jose Convention Center)
   - Talk: "Notebooks Behind the Firewall: AI Agents and Real Patient Data in Jupyter"
   - Video: https://www.youtube.com/watch?v=DEq3HI-SCOI
   - Session details: https://sessionize.com/s/wanjun-gu/notebooks-behind-the-firewall-ai-agents-and-real-p/192251
   - The talk shows how Biorouter connects to Jupyter as an ACP agent, so researchers keep their notebooks, work with institutionally hosted models, and keep protected health data inside institutional boundaries

6. **Biorouter v1.93.2 Release** (18 October 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.93.2
   - What's new: replaces v1.93.1; an update download continues after a dropped connection instead of failing with net::ERR_CONNECTION_RESET; Restart & update waits until macOS has prepared the update and then reopens Biorouter, or says why it could not; the update dialog has Try again; plus everything in v1.93.1: Claude Code chats use about as much of the context window as Versa, Crew sets itself up on a shared Linux server with your server login, Llama Server retries a reply cut by a shared memory collision, and biorouter serve works behind HTTPS proxies and path prefixes, including JupyterHub

7. **Biorouter v1.93.0 Release** (13 October 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.93.0
   - What's new: simpler desktop controls and chat organization; clearer local inference failures and text tool-call handling; browser-free checks before built app build, launch, serving and export; exported launchers reuse the service on macOS and Linux; private /bug reports open for your review; Crew is in Settings > Chat > Capabilities

8. **Biorouter v1.92.1 Release** (6 October 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.92.1
   - What's new: Versa API Azure adds GPT-5.6 Sol, Terra and Luna, sent through the Responses API; Versa API Bedrock lists only the Claude models UCSF serves, adding Sonnet 5 and dropping Opus 5.5 and Opus 5; long GPT-5.6 chats compact instead of stopping

9. **Biorouter v1.92.0 Release** (30 September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.92.0
   - What's new: Crew brings shared lab channels, files and agent tasks; macOS and Linux connect to the background service silently; new models are available; Copy works again in chats and previews

10. **Biorouter v1.91.2 Release** (23 September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.2
   - What's new: Biorouter Copilot status appears only in conversations that request it and can be dismissed; prompts and slash commands use its current name; Windows child processes request hidden console windows; chat gains quoted selections, persistent drafts, active extensions in the slash picker, and adjustable text sizes

11. **Biorouter v1.91.1 Release** (22 September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.1
   - What's new: on Windows, Biorouter's network sockets no longer leak into the programs it starts, so ports are released and the tunnel closes cleanly; on macOS, inline-Python extensions no longer occasionally fail to start; and the test suite's flaky failures are fixed at their causes

12. **Biorouter v1.91.0 Release** (21 September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.0
   - What's new: Biorouter Copilot observes and controls the desktop with an approval for every request, web and documents become their own capability, office contexts load only when needed, Windows gets its first installer and a long list of fixes, and third-party attribution is now correct

13. **Biorouter v1.90.5 Release** (15 September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.90.5
   - What's new: chats keep their place when a tab closes mid-reply, streamed text keeps repeated tokens at chunk boundaries, mid-turn steering stays queued until the running turn consumes it, the artifact preview adapts between a side panel and a vertical layout, and scheduled work follows the caller's reach

14. **Biorouter v1.90.0 Release** (3 September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.90.0
   - What's new: every tool driven across providers, and a review run before the drive

15. **Biorouter at UCSF Research AI Day** (11 March 2026)
   - Link: https://www.youtube.com/live/_2N_w3DtO-Y?si=N1mWBmG-yRH6FVui&t=9983
   - The recorded talk, from 2:46:23, showed agentic workflows, BAAM and collaboration across institutions

The page's Earlier updates list goes back to v1.20.0 (21 January 2026) and also holds the BAAM expansion (2 July) and the office hours recording (16 March).

### Links
- GitHub: https://github.com/BaranziniLab/biorouter
- Baranzini Lab: https://baranzinilab.ucsf.edu/
- Bakar Institute: https://bakarinstitute.ucsf.edu/
- Academic Research Services: https://ars.ucsf.edu/
- UCSF Versa: https://ai.ucsf.edu/platforms-tools-and-resources/ucsf-versa
- UCSF Research AI Day: https://ai.ucsf.edu/researchday

---

## Design requirements

### General
- [x] The app's look on the web: white paper, warm ink, one coral accent, UCSF navy
- [x] Fills before lines: cards, fields, buttons, tags, notes and diagram boxes rest on the subtle fill (`--fill`) with no outline; the animations' app windows are lifted by a soft shadow; hairlines stay for section rules, table rules and the menus the animations draw
- [x] Light only (a test fails if a page follows the system dark mode)
- [x] One type scale shared with the app, weights 400 (700 only for strong in the docs)
- [x] Buttons 32px for small actions and 40px for the one call to action of a view
- [x] Mobile/tablet/desktop responsive, no horizontal overflow at 390px
- [x] Slack in the header of every page and in a Community band on every page
- [x] No emoji unless UI icons

### Navigation
- [x] Sticky header, one 1240px measure on every page so the brand never moves
- [x] The current page is underlined in coral
- [x] Menu button under 760px
- [x] Blurred header with a hairline once the page scrolls

### Introduction tab
- [x] Hero with the product animation inside the first 900px at 1440 wide
- [x] Three principles, each beside its animation
- [x] Capabilities (9)
- [x] Footer links to About, Updates, Release notes and the Baranzini Lab

### Download tab
- [x] Smart OS detection → primary download button
- [x] Complete platform table with all download links
- [x] Install instructions per platform
- [x] Getting started link
- [x] After you install: open the app, connect a model (Llama Server on the Local tab, Versa on the Institutional tab), ask a question
- [x] Ways to use Biorouter: desktop app, terminal chat, command line, browser and JupyterLab, each a looping animation

### Documentation tab
- [x] Sidebar navigation
- [x] Fourteen doc pages, assembled from `docs/website/`
- [x] UCSF-specific setup
- [x] Code blocks with proper monospace font
- [x] Data tables
- [x] Warning/info alert boxes

### BAAM tab
- [x] Search that filters the open shelf as you type
- [x] Three shelves with counts: Extensions, Skills and Workflows
- [x] Cards with an SVG icon tile, name, author and version, GitHub link, description, privacy and affiliation badges, tags and a download (`.brxt`, `.zip` or `.yaml`)
- [x] Extensions and skills, with privacy and institution badges, generated into `registry.json`; workflows stay on `baam.html`

### About tab
- [x] Updates list with dates, newest first, the newest five visible above a folded "Earlier updates" (`check-consistency.mjs` counts the five)
- [x] Links grid
- [x] Collaborators with group labels
- [x] Team cards, with contact links for the lead developer
- [x] Inspirations list

---

## Related documentation

- [The website README](../README.md): how the site is published, the shared files, and the rules its checks enforce
- [The website migration record](../MIGRATION.md): how the site moved into this repository on 2026-06-29
- [Website documentation sources](../../docs/website/README.md): the shell and page fragments `docs.html` is assembled from
- [Documentation style guide](../../docs/contributing/documentation-style.md): the context header, headings and closing section this file follows
